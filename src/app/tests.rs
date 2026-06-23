use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use hypesafe_signing_intent::{Network, TaskContext, TemplateSpec};
use rust_decimal::Decimal;
use serde_json::{json, Value};

use super::mode::RunMode;
use super::runner::{mode_for_signer, Runner};
use crate::config::{Config, SignerConfig};
use crate::gateway::{
    GatewayClient, I18nText, SubAccountRegistry, TaskView, TemplateField, TemplateFieldType,
    TemplateRegistry, TemplateView,
};
use crate::hyperliquid::HlExchangeClient;
use crate::observe::debug_http::{DebugSnapshot, DebugStatus};
use crate::signing::{typed_data_digest_hex, NodeSigner};
use crate::state::StateStore;

const MULTISIG: &str = "0x0000000000000000000000000000000000000002";
const OTHER: &str = "0x0000000000000000000000000000000000000003";

#[tokio::test]
async fn pending_auto_cosigns() {
    let signer = NodeSigner::random_for_test();
    let gateway = TestGateway::spawn(gateway_state(
        &signer,
        vec![task_for_leader("pending-1", "pending", OTHER)],
        vec![],
    ))
    .await;
    let hl = TestHl::spawn(json!({"status": "ok"})).await;
    let mut runner = test_runner(signer, gateway.url(), hl.url(), RunMode::CoSigner, false).await;

    runner.process_cycle().await.unwrap();

    assert_eq!(gateway.signing_payload_count(), 1);
    assert_eq!(gateway.signature_count(), 1);
    assert_eq!(gateway.reject_count(), 0);
    assert_eq!(hl.exchange_count(), 0);
}

#[tokio::test]
async fn digest_mismatch_pending_does_not_sign() {
    let signer = NodeSigner::random_for_test();
    let mut gateway_state = gateway_state(
        &signer,
        vec![task_for_leader("pending-1", "pending", OTHER)],
        vec![],
    );
    gateway_state.signing_digest = bad_digest();
    let gateway = TestGateway::spawn(gateway_state).await;
    let hl = TestHl::spawn(json!({"status": "ok"})).await;
    let mut runner = test_runner(signer, gateway.url(), hl.url(), RunMode::CoSigner, false).await;

    let err = runner.process_cycle().await.unwrap_err();
    assert!(err.to_string().contains("digest mismatch"));

    assert_eq!(gateway.signing_payload_count(), 1);
    assert_eq!(gateway.signature_count(), 0);
    assert_eq!(hl.exchange_count(), 0);
}

#[tokio::test]
async fn does_not_sign_rejected_task() {
    let signer = NodeSigner::random_for_test();
    let rejected = task_with_inputs("pending-1", "pending", OTHER, json!({ "amount": "1001" }));
    let gateway = TestGateway::spawn(gateway_state(&signer, vec![rejected], vec![])).await;
    let hl = TestHl::spawn(json!({"status": "ok"})).await;
    let mut runner = test_runner(signer, gateway.url(), hl.url(), RunMode::CoSigner, false).await;

    runner.process_cycle().await.unwrap();

    assert_eq!(gateway.signing_payload_count(), 0);
    assert_eq!(gateway.signature_count(), 0);
    assert_eq!(gateway.reject_count(), 1);
    assert_eq!(hl.exchange_count(), 0);
}

#[tokio::test]
async fn leader_submits_executable_to_hl() {
    let signer = NodeSigner::random_for_test();
    let gateway = TestGateway::spawn(gateway_state(
        &signer,
        vec![],
        vec![task_for_leader("exec-1", "executable", signer.address_lc())],
    ))
    .await;
    let hl = TestHl::spawn_for_signer(
        &signer,
        json!({
            "status": "ok",
            "response": { "data": { "statuses": [{ "resting": { "oid": 1 } }] } }
        }),
    )
    .await;
    let mut runner = test_runner(
        signer,
        gateway.url(),
        hl.url(),
        RunMode::LeaderExecutor,
        false,
    )
    .await;

    runner.process_cycle().await.unwrap();

    assert_eq!(gateway.outer_payload_count(), 1);
    assert_eq!(hl.info_count(), 1);
    assert_eq!(hl.exchange_count(), 1);
    assert_eq!(gateway.result_count(), 1);
    assert_eq!(gateway.last_result_success(), Some(true));
}

#[tokio::test]
async fn digest_mismatch_leader_does_not_submit_hl() {
    let signer = NodeSigner::random_for_test();
    let mut gateway_state = gateway_state(
        &signer,
        vec![],
        vec![task_for_leader("exec-1", "executable", signer.address_lc())],
    );
    gateway_state.outer_signing_digest = bad_digest();
    let gateway = TestGateway::spawn(gateway_state).await;
    let hl = TestHl::spawn_for_signer(&signer, json!({"status": "ok"})).await;
    let mut runner = test_runner(
        signer,
        gateway.url(),
        hl.url(),
        RunMode::LeaderExecutor,
        false,
    )
    .await;

    let err = runner.process_cycle().await.unwrap_err();
    assert!(err.to_string().contains("digest mismatch"));

    assert_eq!(gateway.outer_payload_count(), 1);
    assert_eq!(hl.exchange_count(), 0);
    assert_eq!(gateway.result_count(), 0);
}

#[tokio::test]
async fn tampered_multi_sig_action_does_not_submit_hl() {
    let signer = NodeSigner::random_for_test();
    let mut gateway_state = gateway_state(
        &signer,
        vec![],
        vec![task_for_leader("exec-1", "executable", signer.address_lc())],
    );
    gateway_state.multi_sig_action["payload"]["action"]["amount"] = json!("2");
    let gateway = TestGateway::spawn(gateway_state).await;
    let hl = TestHl::spawn_for_signer(&signer, json!({"status": "ok"})).await;
    let mut runner = test_runner(
        signer,
        gateway.url(),
        hl.url(),
        RunMode::LeaderExecutor,
        false,
    )
    .await;

    let err = runner.process_cycle().await.unwrap_err();

    assert!(err.to_string().contains("multiSigAction does not match"));
    assert_eq!(gateway.outer_payload_count(), 1);
    assert_eq!(hl.exchange_count(), 0);
    assert_eq!(gateway.result_count(), 0);
}

#[tokio::test]
async fn insufficient_inner_signatures_do_not_submit_hl() {
    let signer = NodeSigner::random_for_test();
    let gateway = TestGateway::spawn(gateway_state(
        &signer,
        vec![],
        vec![task_for_leader("exec-1", "executable", signer.address_lc())],
    ))
    .await;
    let hl = TestHl::spawn_status_with_authorized(
        vec![
            signer.address_lc().to_string(),
            "0x00000000000000000000000000000000000000aa".to_string(),
        ],
        2,
        StatusCode::OK,
        json!({"status": "ok"}),
    )
    .await;
    let mut runner = test_runner(
        signer,
        gateway.url(),
        hl.url(),
        RunMode::LeaderExecutor,
        false,
    )
    .await;

    let err = runner.process_cycle().await.unwrap_err();

    assert!(err.to_string().contains("below threshold"));
    assert_eq!(hl.exchange_count(), 0);
}

#[tokio::test]
async fn unauthorized_inner_signature_does_not_submit_hl() {
    let signer = NodeSigner::random_for_test();
    let gateway = TestGateway::spawn(gateway_state(
        &signer,
        vec![],
        vec![task_for_leader("exec-1", "executable", signer.address_lc())],
    ))
    .await;
    let hl = TestHl::spawn_status_with_authorized(
        vec!["0x00000000000000000000000000000000000000aa".to_string()],
        1,
        StatusCode::OK,
        json!({"status": "ok"}),
    )
    .await;
    let mut runner = test_runner(
        signer,
        gateway.url(),
        hl.url(),
        RunMode::LeaderExecutor,
        false,
    )
    .await;

    let err = runner.process_cycle().await.unwrap_err();

    assert!(err.to_string().contains("unauthorized signer"));
    assert_eq!(hl.exchange_count(), 0);
}

#[tokio::test]
async fn duplicate_inner_signature_does_not_submit_hl() {
    let signer = NodeSigner::random_for_test();
    let mut gateway_state = gateway_state(
        &signer,
        vec![],
        vec![task_for_leader("exec-1", "executable", signer.address_lc())],
    );
    let signature = gateway_state.multi_sig_action["signatures"][0].clone();
    gateway_state.multi_sig_action["signatures"] = json!([signature.clone(), signature]);
    let gateway = TestGateway::spawn(gateway_state).await;
    let hl = TestHl::spawn_for_signer(&signer, json!({"status": "ok"})).await;
    let mut runner = test_runner(
        signer,
        gateway.url(),
        hl.url(),
        RunMode::LeaderExecutor,
        false,
    )
    .await;

    let err = runner.process_cycle().await.unwrap_err();

    assert!(err.to_string().contains("duplicate inner signature signer"));
    assert_eq!(hl.exchange_count(), 0);
}

#[tokio::test]
async fn whitelisted_nonlocal_leader_does_not_execute() {
    let signer = NodeSigner::random_for_test();
    let gateway = TestGateway::spawn(gateway_state(
        &signer,
        vec![],
        vec![task_for_leader("exec-1", "executable", OTHER)],
    ))
    .await;
    let hl = TestHl::spawn_for_signer(&signer, json!({"status": "ok"})).await;
    let mut runner = test_runner(
        signer,
        gateway.url(),
        hl.url(),
        RunMode::LeaderExecutor,
        false,
    )
    .await;
    runner.config.allowed_leaders.push(OTHER.to_string());
    runner.templates = TemplateRegistry::new(vec![template("withdraw3")]);

    let err = runner.process_cycle().await.unwrap_err();

    assert!(err.to_string().contains("does not match local signer"));
    assert_eq!(gateway.outer_payload_count(), 0);
    assert_eq!(hl.exchange_count(), 0);
}

#[tokio::test]
async fn hl_status_error_writes_gateway_failure() {
    let signer = NodeSigner::random_for_test();
    let gateway = TestGateway::spawn(gateway_state(
        &signer,
        vec![],
        vec![task_for_leader("exec-1", "executable", signer.address_lc())],
    ))
    .await;
    let hl = TestHl::spawn_for_signer(
        &signer,
        json!({
            "status": "ok",
            "response": { "data": { "statuses": [{ "error": "Insufficient margin" }] } }
        }),
    )
    .await;
    let mut runner = test_runner(
        signer,
        gateway.url(),
        hl.url(),
        RunMode::LeaderExecutor,
        false,
    )
    .await;

    runner.process_cycle().await.unwrap();

    assert_eq!(hl.exchange_count(), 1);
    assert_eq!(gateway.result_count(), 1);
    assert_eq!(gateway.last_result_success(), Some(false));
    assert_eq!(
        gateway.last_result_error().as_deref(),
        Some("Insufficient margin")
    );
}

#[tokio::test]
async fn hl_top_level_error_writes_gateway_failure() {
    let signer = NodeSigner::random_for_test();
    let gateway = TestGateway::spawn(gateway_state(
        &signer,
        vec![],
        vec![task_for_leader("exec-1", "executable", signer.address_lc())],
    ))
    .await;
    let hl = TestHl::spawn_for_signer(
        &signer,
        json!({
            "status": "err",
            "response": { "error": "bad signature" }
        }),
    )
    .await;
    let mut runner = test_runner(
        signer,
        gateway.url(),
        hl.url(),
        RunMode::LeaderExecutor,
        false,
    )
    .await;

    runner.process_cycle().await.unwrap();

    assert_eq!(gateway.result_count(), 1);
    assert_eq!(gateway.last_result_success(), Some(false));
    assert_eq!(
        gateway.last_result_error().as_deref(),
        Some("bad signature")
    );
}

#[tokio::test]
async fn local_only_response_writes_gateway_success() {
    let signer = NodeSigner::random_for_test();
    let gateway = TestGateway::spawn(gateway_state(
        &signer,
        vec![],
        vec![task_with_template(
            "local-1",
            "executable",
            signer.address_lc(),
            "local_task",
        )],
    ))
    .await;
    let hl = TestHl::spawn(json!({"status": "ok"})).await;
    let mut runner = test_runner(
        signer,
        gateway.url(),
        hl.url(),
        RunMode::LeaderExecutor,
        false,
    )
    .await;
    runner.config.allowed_templates = vec!["local_task".to_string()];
    runner.templates = TemplateRegistry::new(vec![local_template("local_task")]);

    runner.process_cycle().await.unwrap();

    assert_eq!(hl.exchange_count(), 0);
    assert_eq!(gateway.result_count(), 1);
    assert_eq!(gateway.last_result_success(), Some(true));
}

#[tokio::test]
async fn does_not_resubmit_hl_after_submit_error() {
    let signer = NodeSigner::random_for_test();
    let gateway = TestGateway::spawn(gateway_state(
        &signer,
        vec![],
        vec![task_for_leader("exec-1", "executable", signer.address_lc())],
    ))
    .await;
    let hl = TestHl::spawn_status_for_signer(
        &signer,
        StatusCode::BAD_GATEWAY,
        json!({"error": "upstream"}),
    )
    .await;
    let mut runner = test_runner(
        signer,
        gateway.url(),
        hl.url(),
        RunMode::LeaderExecutor,
        false,
    )
    .await;

    let err = runner.process_cycle().await.unwrap_err();
    assert!(err.to_string().contains("502"));
    let exchange_count = hl.exchange_count();
    let recent = runner.state.recent(1).await.unwrap();
    assert_eq!(recent[0].local_status, "execute_unknown");

    runner.process_cycle().await.unwrap();

    assert_eq!(hl.exchange_count(), exchange_count);
    assert_eq!(gateway.result_count(), 0);
}

#[tokio::test]
async fn submit_before_hl_failure_records_failed_without_hl_call() {
    let signer = NodeSigner::random_for_test();
    let mut gateway_state = gateway_state(
        &signer,
        vec![],
        vec![task_for_leader("exec-1", "executable", signer.address_lc())],
    );
    gateway_state.outer_typed_data = json!({});
    let gateway = TestGateway::spawn(gateway_state).await;
    let hl = TestHl::spawn_for_signer(&signer, json!({"status": "ok"})).await;
    let mut runner = test_runner(
        signer,
        gateway.url(),
        hl.url(),
        RunMode::LeaderExecutor,
        false,
    )
    .await;

    let err = runner.process_cycle().await.unwrap_err();
    assert!(err.to_string().contains("outer typedData does not match"));
    let recent = runner.state.recent(1).await.unwrap();

    assert_eq!(hl.exchange_count(), 0);
    assert_eq!(recent[0].local_status, "failed");
}

#[tokio::test]
async fn non_leader_does_not_submit_hl() {
    let signer = NodeSigner::random_for_test();
    let gateway = TestGateway::spawn(gateway_state(
        &signer,
        vec![],
        vec![task_for_leader("exec-1", "executable", OTHER)],
    ))
    .await;
    let hl = TestHl::spawn(json!({"status": "ok"})).await;
    let mut runner = test_runner(signer, gateway.url(), hl.url(), RunMode::CoSigner, false).await;

    runner.process_cycle().await.unwrap();

    assert_eq!(gateway.execute_inbox_count(), 0);
    assert_eq!(gateway.outer_payload_count(), 0);
    assert_eq!(gateway.result_count(), 0);
    assert_eq!(hl.exchange_count(), 0);
}

#[test]
fn starts_as_cosigner_when_signer_is_not_leader() {
    assert_eq!(
        mode_for_signer(OTHER, &[MULTISIG.to_string()]),
        RunMode::CoSigner
    );
}

#[tokio::test]
async fn does_not_resubmit_exchange_after_hl_response() {
    let signer = NodeSigner::random_for_test();
    let executable = task_for_leader("exec-1", "executable", signer.address_lc());
    let gateway =
        TestGateway::spawn(gateway_state(&signer, vec![], vec![executable.clone()])).await;
    let hl = TestHl::spawn(json!({"status": "ok"})).await;
    let mut runner = test_runner(
        signer,
        gateway.url(),
        hl.url(),
        RunMode::LeaderExecutor,
        false,
    )
    .await;
    runner
        .state
        .record_hl_response(&executable, &json!({"status": "ok"}))
        .await
        .unwrap();

    runner.process_cycle().await.unwrap();

    assert_eq!(gateway.outer_payload_count(), 0);
    assert_eq!(hl.exchange_count(), 0);
    assert_eq!(gateway.result_count(), 1);
    assert_eq!(gateway.last_result_success(), Some(true));
}

#[tokio::test]
async fn dry_run_does_not_sign_or_submit() {
    let signer = NodeSigner::random_for_test();
    let gateway = TestGateway::spawn(gateway_state(
        &signer,
        vec![task_for_leader("pending-1", "pending", signer.address_lc())],
        vec![task_for_leader("exec-1", "executable", signer.address_lc())],
    ))
    .await;
    let hl = TestHl::spawn(json!({"status": "ok"})).await;
    let mut runner = test_runner(
        signer,
        gateway.url(),
        hl.url(),
        RunMode::LeaderExecutor,
        true,
    )
    .await;

    runner.process_cycle().await.unwrap();

    assert_eq!(gateway.signing_payload_count(), 0);
    assert_eq!(gateway.signature_count(), 0);
    assert_eq!(gateway.reject_count(), 0);
    assert_eq!(gateway.outer_payload_count(), 0);
    assert_eq!(gateway.result_count(), 0);
    assert_eq!(hl.exchange_count(), 0);
}

#[tokio::test]
async fn local_signed_task_skips_signing_payload_request() {
    let signer = NodeSigner::random_for_test();
    let pending = task_for_leader("pending-1", "pending", OTHER);
    let gateway = TestGateway::spawn(gateway_state(&signer, vec![pending.clone()], vec![])).await;
    let hl = TestHl::spawn(json!({"status": "ok"})).await;
    let mut runner = test_runner(signer, gateway.url(), hl.url(), RunMode::CoSigner, false).await;
    runner
        .state
        .record_signed(&pending, "submitted")
        .await
        .unwrap();

    runner.process_cycle().await.unwrap();

    assert_eq!(gateway.signing_payload_count(), 0);
    assert_eq!(gateway.signature_count(), 0);
}

#[tokio::test]
async fn template_refresh_success_uses_new_metadata() {
    let signer = NodeSigner::random_for_test();
    let gateway_state = gateway_state(
        &signer,
        vec![],
        vec![task_with_template(
            "local-1",
            "executable",
            signer.address_lc(),
            "local_task",
        )],
    );
    *gateway_state.templates.lock().unwrap() = vec![local_template("local_task")];
    let gateway = TestGateway::spawn(gateway_state).await;
    let hl = TestHl::spawn(json!({"status": "ok"})).await;
    let mut runner = test_runner(
        signer,
        gateway.url(),
        hl.url(),
        RunMode::LeaderExecutor,
        false,
    )
    .await;
    runner.config.allowed_templates = vec!["local_task".to_string()];
    runner.last_template_refresh_at = None;

    runner.process_cycle().await.unwrap();

    assert_eq!(gateway.template_count(), 1);
    assert_eq!(hl.exchange_count(), 0);
    assert_eq!(gateway.result_count(), 1);
    assert_eq!(gateway.last_result_success(), Some(true));
}

#[tokio::test]
async fn template_refresh_failure_keeps_previous_registry() {
    let signer = NodeSigner::random_for_test();
    let gateway_state = gateway_state(
        &signer,
        vec![task_for_leader("pending-1", "pending", OTHER)],
        vec![],
    );
    gateway_state
        .template_bad_gateway_failures
        .store(4, Ordering::SeqCst);
    let gateway = TestGateway::spawn(gateway_state).await;
    let hl = TestHl::spawn(json!({"status": "ok"})).await;
    let mut runner = test_runner(signer, gateway.url(), hl.url(), RunMode::CoSigner, false).await;
    runner.last_template_refresh_at = None;

    runner.process_cycle().await.unwrap();

    assert_eq!(gateway.template_count(), 4);
    assert_eq!(gateway.signing_payload_count(), 1);
    assert_eq!(gateway.signature_count(), 1);
}

async fn test_runner(
    signer: NodeSigner,
    gateway_url: String,
    hl_url: String,
    mode: RunMode,
    dry_run: bool,
) -> Runner {
    let leader = if mode == RunMode::LeaderExecutor {
        signer.address_lc().to_string()
    } else {
        OTHER.to_string()
    };
    let config = Config {
        gateway_url: gateway_url.clone(),
        hl_api_url: hl_url.clone(),
        poll_interval_secs: 15,
        dry_run,
        allowed_templates: vec!["withdraw3".to_string()],
        allowed_creators: vec![OTHER.to_string(), signer.address_lc().to_string()],
        allowed_leaders: vec![leader.clone()],
        template_input_policies: Default::default(),
        state_db: "sqlite::memory:".to_string(),
        debug_http_addr: "127.0.0.1:9909".parse().unwrap(),
        signer: SignerConfig {
            keystore_path: "test.json".to_string(),
            password_env: None,
        },
        leader,
        multisig: MULTISIG.to_string(),
        withdraw_limit: Decimal::new(1000, 0),
    };
    let mut gateway = GatewayClient::new(gateway_url);
    gateway.login(&signer).await.unwrap();
    let debug = DebugSnapshot::new(DebugStatus {
        signer: signer.address_lc().to_string(),
        mode: mode.as_str().to_string(),
        multisig: MULTISIG.to_string(),
        leader: config.leader.clone(),
        last_poll_at: None,
        last_success_at: None,
        last_error: None,
        last_error_at: None,
        consecutive_gateway_failures: 0,
    });

    let templates = if mode == RunMode::LeaderExecutor {
        TemplateRegistry::new(vec![template_with_intent("withdraw3")])
    } else {
        TemplateRegistry::new(vec![template("withdraw3")])
    };

    Runner {
        config,
        signer,
        gateway,
        hl: HlExchangeClient::new(hl_url),
        state: StateStore::connect("sqlite::memory:").await.unwrap(),
        mode,
        debug,
        templates,
        sub_accounts: SubAccountRegistry::default(),
        consecutive_gateway_failures: 0,
        last_template_refresh_at: Some(crate::state::now_secs()),
    }
}

fn task_for_leader(id: &str, status: &str, leader: &str) -> TaskView {
    task_with_inputs(id, status, leader, json!({ "amount": "1" }))
}

fn task_with_inputs(id: &str, status: &str, leader: &str, inputs: Value) -> TaskView {
    task_with_template_and_inputs(id, status, leader, "withdraw3", inputs)
}

fn task_with_template(id: &str, status: &str, leader: &str, template_id: &str) -> TaskView {
    task_with_template_and_inputs(id, status, leader, template_id, json!({ "amount": "1" }))
}

fn task_with_template_and_inputs(
    id: &str,
    status: &str,
    leader: &str,
    template_id: &str,
    inputs: Value,
) -> TaskView {
    TaskView {
        id: id.to_string(),
        multisig_address: MULTISIG.to_string(),
        creator: leader.to_string(),
        leader: leader.to_string(),
        nonce: 1,
        network: "mainnet".to_string(),
        template_id: template_id.to_string(),
        template_version: 1,
        inputs,
        signing_digest: None,
        creator_signature: None,
        action: None,
        threshold: 1,
        status: status.to_string(),
        signatures: vec![],
        approvals: 0,
        rejects: 0,
        rejections: vec![],
        created_at: 0,
        expires_at: 999,
        result: None,
    }
}

fn template(id: &str) -> TemplateView {
    TemplateView {
        id: id.to_string(),
        version: 1,
        type_name: "withdraw".to_string(),
        hl_action_type: Some("withdraw3".to_string()),
        display_name: text("Withdraw"),
        description: text("Withdraw"),
        summary: text("Withdraw"),
        fields: vec![TemplateField {
            name: "amount".to_string(),
            field_type: TemplateFieldType::Amount,
            required: true,
            label: text("Amount"),
            description: text("Amount"),
        }],
        signing: None,
        exchange: None,
    }
}

fn template_with_intent(id: &str) -> TemplateView {
    TemplateView {
        id: id.to_string(),
        version: 1,
        type_name: "withdraw".to_string(),
        hl_action_type: Some("withdraw3".to_string()),
        display_name: text("Withdraw"),
        description: text("Withdraw"),
        summary: text("Withdraw"),
        fields: vec![TemplateField {
            name: "amount".to_string(),
            field_type: TemplateFieldType::Amount,
            required: true,
            label: text("Amount"),
            description: text("Amount"),
        }],
        signing: Some(json!({
            "domain": {
                "name": "HypeSafe",
                "version": "1",
                "chainId": 42161,
                "verifyingContract": "ctx:multiSigAddress"
            },
            "primaryType": "Ping",
            "types": [
                { "name": "leader", "type": "address" },
                { "name": "amount", "type": "string" }
            ],
            "message": [
                { "name": "leader", "from": "ctx:leader" },
                { "name": "amount", "from": "param:amount" }
            ]
        })),
        exchange: Some(json!({
            "action": [
                { "key": "type", "from": "const:withdraw3" },
                { "key": "amount", "from": "param:amount" }
            ]
        })),
    }
}

fn local_template(id: &str) -> TemplateView {
    TemplateView {
        id: id.to_string(),
        version: 1,
        type_name: "local".to_string(),
        hl_action_type: None,
        display_name: text("Local"),
        description: text("Local"),
        summary: text("Local"),
        fields: vec![],
        signing: None,
        exchange: None,
    }
}

fn text(value: &str) -> I18nText {
    I18nText {
        en: value.to_string(),
        zh: value.to_string(),
    }
}

fn executable_payloads(signer: &NodeSigner) -> (Value, String, String, Value, Value) {
    let template = template_with_intent("withdraw3");
    let template_spec = TemplateSpec::new(
        template.id,
        template.version,
        template.signing.unwrap(),
        template.exchange.unwrap(),
    );
    let inputs = json!({ "amount": "1" });
    let ctx = TaskContext {
        multisig_address: MULTISIG,
        leader: signer.address_lc(),
        nonce: 1,
        network: Network::Mainnet,
        template: &template_spec,
        params: &inputs,
    };
    let signing_payload = hypesafe_signing_intent::build_signing_payload(&ctx).unwrap();
    let signing_digest = typed_data_digest_hex(&signing_payload).unwrap();
    let inner_signature = signer.sign_and_verify_typed_data(&signing_payload).unwrap();
    let multi_sig_action =
        hypesafe_signing_intent::build_multi_sig_action(&ctx, &[inner_signature.clone()]).unwrap();
    let outer_typed_data =
        hypesafe_signing_intent::build_outer_signing_payload(&ctx, &multi_sig_action).unwrap();
    (
        signing_payload,
        signing_digest,
        inner_signature,
        multi_sig_action,
        outer_typed_data,
    )
}

fn bad_digest() -> String {
    format!("0x{}", "00".repeat(32))
}

#[derive(Clone)]
struct GatewayState {
    signer: String,
    inner_signature: String,
    signing_payload: Value,
    signing_digest: String,
    outer_typed_data: Value,
    outer_signing_digest: String,
    multi_sig_action: Value,
    vault_address: Option<String>,
    templates: Arc<Mutex<Vec<TemplateView>>>,
    sign_tasks: Arc<Mutex<Vec<TaskView>>>,
    execute_tasks: Arc<Mutex<Vec<TaskView>>>,
    template_bad_gateway_failures: Arc<AtomicUsize>,
    sign_inbox_count: Arc<AtomicUsize>,
    execute_inbox_count: Arc<AtomicUsize>,
    template_count: Arc<AtomicUsize>,
    signing_payload_count: Arc<AtomicUsize>,
    signature_count: Arc<AtomicUsize>,
    reject_count: Arc<AtomicUsize>,
    outer_payload_count: Arc<AtomicUsize>,
    result_count: Arc<AtomicUsize>,
    last_result: Arc<Mutex<Option<Value>>>,
}

fn gateway_state(
    signer: &NodeSigner,
    sign_tasks: Vec<TaskView>,
    execute_tasks: Vec<TaskView>,
) -> GatewayState {
    let (signing_payload, signing_digest, inner_signature, multi_sig_action, outer_typed_data) =
        executable_payloads(signer);
    let outer_signing_digest = typed_data_digest_hex(&outer_typed_data).expect("outer digest");
    GatewayState {
        signer: signer.address_lc().to_string(),
        inner_signature,
        signing_payload,
        signing_digest,
        outer_typed_data,
        outer_signing_digest,
        multi_sig_action,
        vault_address: None,
        templates: Arc::new(Mutex::new(vec![template_with_intent("withdraw3")])),
        sign_tasks: Arc::new(Mutex::new(sign_tasks)),
        execute_tasks: Arc::new(Mutex::new(execute_tasks)),
        template_bad_gateway_failures: Arc::new(AtomicUsize::new(0)),
        sign_inbox_count: Arc::new(AtomicUsize::new(0)),
        execute_inbox_count: Arc::new(AtomicUsize::new(0)),
        template_count: Arc::new(AtomicUsize::new(0)),
        signing_payload_count: Arc::new(AtomicUsize::new(0)),
        signature_count: Arc::new(AtomicUsize::new(0)),
        reject_count: Arc::new(AtomicUsize::new(0)),
        outer_payload_count: Arc::new(AtomicUsize::new(0)),
        result_count: Arc::new(AtomicUsize::new(0)),
        last_result: Arc::new(Mutex::new(None)),
    }
}

struct TestGateway {
    base_url: String,
    state: GatewayState,
}

impl TestGateway {
    async fn spawn(state: GatewayState) -> Self {
        let app = Router::new()
            .route("/auth/sign-in-message", get(sign_in_message))
            .route("/auth/login", post(login))
            .route("/templates", get(templates))
            .route("/tasks/inbox", get(task_inbox))
            .route("/tasks/{task_id}/signing-payload", get(signing_payload))
            .route("/tasks/{task_id}/signatures", post(submit_signature))
            .route("/tasks/{task_id}/reject", post(reject_task))
            .route("/tasks/{task_id}/outer-signing-payload", get(outer_payload))
            .route("/tasks/{task_id}/result", post(submit_result))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            base_url: format!("http://{addr}"),
            state,
        }
    }

    fn url(&self) -> String {
        self.base_url.clone()
    }

    fn signing_payload_count(&self) -> usize {
        self.state.signing_payload_count.load(Ordering::SeqCst)
    }

    fn signature_count(&self) -> usize {
        self.state.signature_count.load(Ordering::SeqCst)
    }

    fn reject_count(&self) -> usize {
        self.state.reject_count.load(Ordering::SeqCst)
    }

    fn execute_inbox_count(&self) -> usize {
        self.state.execute_inbox_count.load(Ordering::SeqCst)
    }

    fn template_count(&self) -> usize {
        self.state.template_count.load(Ordering::SeqCst)
    }

    fn outer_payload_count(&self) -> usize {
        self.state.outer_payload_count.load(Ordering::SeqCst)
    }

    fn result_count(&self) -> usize {
        self.state.result_count.load(Ordering::SeqCst)
    }

    fn last_result_success(&self) -> Option<bool> {
        self.state
            .last_result
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|body| body.get("success"))
            .and_then(Value::as_bool)
    }

    fn last_result_error(&self) -> Option<String> {
        self.state
            .last_result
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|body| body.get("error"))
            .and_then(Value::as_str)
            .map(str::to_string)
    }
}

async fn sign_in_message(Query(query): Query<HashMap<String, String>>) -> Response {
    let signer = query.get("address").cloned().unwrap_or_default();
    envelope(json!({
        "signer": signer,
        "timestamp": "2026-06-22 00:00:00",
        "message": "Sign in to HypeSafe"
    }))
}

async fn login() -> Response {
    envelope(json!({
        "token": "test-token",
        "claims": { "nextEndTime": i64::MAX }
    }))
}

async fn templates(State(state): State<GatewayState>) -> Response {
    state.template_count.fetch_add(1, Ordering::SeqCst);
    if consume_failure(&state.template_bad_gateway_failures) {
        return (StatusCode::BAD_GATEWAY, "error code: 502").into_response();
    }
    envelope(json!(state.templates.lock().unwrap().clone()))
}

async fn task_inbox(
    State(state): State<GatewayState>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    match query.get("type").map(String::as_str) {
        Some("sign") => {
            state.sign_inbox_count.fetch_add(1, Ordering::SeqCst);
            envelope(json!(tasks_with_digest(
                &state.sign_tasks.lock().unwrap(),
                &state,
            )))
        }
        Some("execute") => {
            state.execute_inbox_count.fetch_add(1, Ordering::SeqCst);
            envelope(json!(tasks_with_digest(
                &state.execute_tasks.lock().unwrap(),
                &state,
            )))
        }
        _ => envelope(json!([])),
    }
}

async fn signing_payload(
    State(state): State<GatewayState>,
    Path(_task_id): Path<String>,
) -> Response {
    state.signing_payload_count.fetch_add(1, Ordering::SeqCst);
    envelope(json!({
        "signingPayload": state.signing_payload,
        "signingDigest": state.signing_digest,
    }))
}

async fn submit_signature(
    State(state): State<GatewayState>,
    Path(task_id): Path<String>,
    Json(_body): Json<Value>,
) -> Response {
    state.signature_count.fetch_add(1, Ordering::SeqCst);
    envelope(json!(task_view_response(&state, &task_id, "pending")))
}

async fn reject_task(
    State(state): State<GatewayState>,
    Path(task_id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    state.reject_count.fetch_add(1, Ordering::SeqCst);
    let reason = body
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or("policy reject");
    let mut value = task_view_response(&state, &task_id, "pending");
    value["rejects"] = json!(1);
    value["rejections"] = json!([{
        "signer": state.signer,
        "reason": reason,
        "rejectedAt": 0
    }]);
    envelope(json!(value))
}

async fn outer_payload(
    State(state): State<GatewayState>,
    Path(_task_id): Path<String>,
) -> Response {
    state.outer_payload_count.fetch_add(1, Ordering::SeqCst);
    envelope(json!({
        "typedData": state.outer_typed_data,
        "outerSigningDigest": state.outer_signing_digest,
        "multiSigAction": state.multi_sig_action,
        "vaultAddress": state.vault_address
    }))
}

async fn submit_result(
    State(state): State<GatewayState>,
    Path(task_id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    state.result_count.fetch_add(1, Ordering::SeqCst);
    *state.last_result.lock().unwrap() = Some(body.clone());
    let mut value = task_view_response(&state, &task_id, "history");
    value["result"] = body;
    envelope(json!(value))
}

fn task_view_response(state: &GatewayState, task_id: &str, status: &str) -> Value {
    json!({
        "id": task_id,
        "multisigAddress": MULTISIG,
        "creator": OTHER,
        "leader": state.signer,
        "nonce": 1,
        "network": "mainnet",
        "templateId": "withdraw3",
        "templateVersion": 1,
        "inputs": { "amount": "1" },
        "signingDigest": state.signing_digest,
        "action": null,
        "threshold": 1,
        "status": status,
        "signatures": [],
        "approvals": 0,
        "rejects": 0,
        "rejections": [],
        "createdAt": 0,
        "expiresAt": 999,
        "result": null
    })
}

fn tasks_with_digest(tasks: &[TaskView], state: &GatewayState) -> Vec<TaskView> {
    tasks
        .iter()
        .cloned()
        .map(|mut task| {
            if task.signing_digest.is_none() {
                task.signing_digest = Some(state.signing_digest.clone());
            }
            if task.creator.eq_ignore_ascii_case(&state.signer) && task.creator_signature.is_none()
            {
                task.creator_signature = Some(state.inner_signature.clone());
            }
            task
        })
        .collect()
}

fn envelope(data: Value) -> Response {
    Json(json!({
        "code": 0,
        "message": "ok",
        "data": data
    }))
    .into_response()
}

fn consume_failure(counter: &AtomicUsize) -> bool {
    counter
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
            value.checked_sub(1)
        })
        .is_ok()
}

#[derive(Clone)]
struct HlState {
    response: Value,
    status: StatusCode,
    authorized_users: Vec<String>,
    threshold: i64,
    exchange_count: Arc<AtomicUsize>,
    info_count: Arc<AtomicUsize>,
}

struct TestHl {
    base_url: String,
    state: HlState,
}

impl TestHl {
    async fn spawn(response: Value) -> Self {
        Self::spawn_status(StatusCode::OK, response).await
    }

    async fn spawn_status(status: StatusCode, response: Value) -> Self {
        Self::spawn_status_with_authorized(Vec::new(), 1, status, response).await
    }

    async fn spawn_for_signer(signer: &NodeSigner, response: Value) -> Self {
        Self::spawn_status_for_signer(signer, StatusCode::OK, response).await
    }

    async fn spawn_status_for_signer(
        signer: &NodeSigner,
        status: StatusCode,
        response: Value,
    ) -> Self {
        Self::spawn_status_with_authorized(
            vec![signer.address_lc().to_string()],
            1,
            status,
            response,
        )
        .await
    }

    async fn spawn_status_with_authorized(
        authorized_users: Vec<String>,
        threshold: i64,
        status: StatusCode,
        response: Value,
    ) -> Self {
        let state = HlState {
            response,
            status,
            authorized_users,
            threshold,
            exchange_count: Arc::new(AtomicUsize::new(0)),
            info_count: Arc::new(AtomicUsize::new(0)),
        };
        let app = Router::new()
            .route("/info", post(info))
            .route("/exchange", post(exchange))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            base_url: format!("http://{addr}"),
            state,
        }
    }

    fn url(&self) -> String {
        self.base_url.clone()
    }

    fn exchange_count(&self) -> usize {
        self.state.exchange_count.load(Ordering::SeqCst)
    }

    fn info_count(&self) -> usize {
        self.state.info_count.load(Ordering::SeqCst)
    }
}

async fn info(State(state): State<HlState>, Json(_body): Json<Value>) -> Response {
    state.info_count.fetch_add(1, Ordering::SeqCst);
    Json(json!({
        "authorizedUsers": state.authorized_users,
        "threshold": state.threshold
    }))
    .into_response()
}

async fn exchange(State(state): State<HlState>, Json(_body): Json<Value>) -> Response {
    state.exchange_count.fetch_add(1, Ordering::SeqCst);
    (state.status, Json(state.response)).into_response()
}
