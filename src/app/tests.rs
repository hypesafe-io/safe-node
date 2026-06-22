use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
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
    let hl = TestHl::spawn(json!({
        "status": "ok",
        "response": { "data": { "statuses": [{ "resting": { "oid": 1 } }] } }
    }))
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
    let hl = TestHl::spawn(json!({"status": "ok"})).await;
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
async fn hl_status_error_writes_gateway_failure() {
    let signer = NodeSigner::random_for_test();
    let gateway = TestGateway::spawn(gateway_state(
        &signer,
        vec![],
        vec![task_for_leader("exec-1", "executable", signer.address_lc())],
    ))
    .await;
    let hl = TestHl::spawn(json!({
        "status": "ok",
        "response": { "data": { "statuses": [{ "error": "Insufficient margin" }] } }
    }))
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
    let hl = TestHl::spawn(json!({
        "status": "err",
        "response": { "error": "bad signature" }
    }))
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
    let hl = TestHl::spawn_status(StatusCode::BAD_GATEWAY, json!({"error": "upstream"})).await;
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
    gateway_state.signing_payload = json!({});
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

    let err = runner.process_cycle().await.unwrap_err();
    assert!(err.to_string().contains("invalid typed-data"));
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
    assert_eq!(mode_for_signer(OTHER, MULTISIG), RunMode::CoSigner);
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
        allowed_creators: vec![OTHER.to_string()],
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

    Runner {
        config,
        signer,
        gateway,
        hl: HlExchangeClient::new(hl_url),
        state: StateStore::connect("sqlite::memory:").await.unwrap(),
        mode,
        debug,
        templates: TemplateRegistry::new(vec![template("withdraw3")]),
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
        creator: OTHER.to_string(),
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

fn typed_data(signer: &NodeSigner) -> Value {
    json!({
        "domain": {
            "name": "HypeSafe",
            "version": "1",
            "chainId": 42161,
            "verifyingContract": "0x0000000000000000000000000000000000000001"
        },
        "primaryType": "Ping",
        "types": {
            "Ping": [
                { "name": "sender", "type": "address" },
                { "name": "nonce", "type": "uint64" }
            ]
        },
        "message": {
            "sender": signer.address_lc(),
            "nonce": 1
        }
    })
}

fn bad_digest() -> String {
    format!("0x{}", "00".repeat(32))
}

#[derive(Clone)]
struct GatewayState {
    signer: String,
    signing_payload: Value,
    signing_digest: String,
    outer_signing_digest: String,
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
    let signing_payload = typed_data(signer);
    let signing_digest = typed_data_digest_hex(&signing_payload).expect("typed-data digest");
    GatewayState {
        signer: signer.address_lc().to_string(),
        signing_payload,
        signing_digest: signing_digest.clone(),
        outer_signing_digest: signing_digest,
        templates: Arc::new(Mutex::new(vec![template("withdraw3")])),
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
                &state.signing_digest,
            )))
        }
        Some("execute") => {
            state.execute_inbox_count.fetch_add(1, Ordering::SeqCst);
            envelope(json!(tasks_with_digest(
                &state.execute_tasks.lock().unwrap(),
                &state.signing_digest,
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
        "typedData": state.signing_payload,
        "outerSigningDigest": state.outer_signing_digest,
        "multiSigAction": { "type": "withdraw3" },
        "vaultAddress": null
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

fn tasks_with_digest(tasks: &[TaskView], digest: &str) -> Vec<TaskView> {
    tasks
        .iter()
        .cloned()
        .map(|mut task| {
            if task.signing_digest.is_none() {
                task.signing_digest = Some(digest.to_string());
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
    exchange_count: Arc<AtomicUsize>,
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
        let state = HlState {
            response,
            status,
            exchange_count: Arc::new(AtomicUsize::new(0)),
        };
        let app = Router::new()
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
}

async fn exchange(State(state): State<HlState>, Json(_body): Json<Value>) -> Response {
    state.exchange_count.fetch_add(1, Ordering::SeqCst);
    (state.status, Json(state.response)).into_response()
}
