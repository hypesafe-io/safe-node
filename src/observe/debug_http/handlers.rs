use axum::extract::{Query, State};
use axum::http::{header, HeaderMap};
use axum::response::Html;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::Value;
use tracing::warn;

use super::types::{DebugAppState, DebugPolicy, DebugStatus, RpcTaskState, TransactionsQuery};
use super::web::INDEX_HTML;
use crate::config::normalize_address;
use crate::config::RedactedConfig;
use crate::gateway::{CreateTaskPayloadRequest, TaskView};
use crate::policy::evaluate;
use crate::state::{now_secs, RecentTask};
use crate::{NodeError, Result};

const RPC_NETWORK: &str = "mainnet";

pub(super) fn router(state: DebugAppState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/debug", get(index))
        .route("/debug/status", get(status))
        .route("/debug/transactions", get(transactions))
        .route("/debug/config", get(config))
        .route("/debug/policy", get(policy))
        .route("/rpc/task/create", post(create_task))
        .with_state(state)
}

async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

async fn status(State(state): State<DebugAppState>) -> Json<DebugStatus> {
    Json(state.snapshot.read().await)
}

async fn transactions(
    State(state): State<DebugAppState>,
    Query(query): Query<TransactionsQuery>,
) -> Result<Json<Vec<RecentTask>>> {
    let limit = query.limit.unwrap_or(20);
    Ok(Json(state.state.recent(limit).await?))
}

async fn config(State(state): State<DebugAppState>) -> Json<RedactedConfig> {
    Json(state.config)
}

async fn policy(State(state): State<DebugAppState>) -> Json<DebugPolicy> {
    Json(DebugPolicy {
        multisig: state.config.multisig,
        leader: state.config.leader,
        allowed_templates: state.config.allowed_templates,
        allowed_creators: state.config.allowed_creators,
        allowed_leaders: state.config.allowed_leaders,
        template_input_policies: state.config.template_input_policies,
        withdraw_limit: state.config.withdraw_limit,
    })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateTaskRequest {
    template_id: String,
    template_version: i64,
    inputs: Value,
    expires_in_secs: Option<i64>,
}

async fn create_task(
    State(state): State<DebugAppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<TaskView>> {
    let task_state = state
        .task
        .as_ref()
        .ok_or_else(|| NodeError::Runtime("RPC task creation is unavailable".to_string()))?;
    authorize_rpc(&headers, task_state.config.rpc_auth_token.as_deref())?;
    let request = serde_json::from_value(body)
        .map_err(|err| NodeError::Config(format!("invalid RPC task create request: {err}")))?;
    Ok(Json(
        create_task_with_node_signer(task_state, request).await?,
    ))
}

fn authorize_rpc(headers: &HeaderMap, token: Option<&str>) -> Result<()> {
    let Some(expected) = token else {
        return Ok(());
    };
    let authorized = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .is_some_and(|value| value == expected);
    if authorized {
        Ok(())
    } else {
        Err(NodeError::Unauthorized)
    }
}

async fn create_task_with_node_signer(
    state: &RpcTaskState,
    request: CreateTaskRequest,
) -> Result<TaskView> {
    validate_create_task_policy(state, &request)?;
    let create_payload = CreateTaskPayloadRequest {
        template_id: request.template_id,
        template_version: request.template_version,
        inputs: request.inputs,
        leader: state.config.leader.clone(),
        network: Some(RPC_NETWORK.to_string()),
        expires_in_secs: request.expires_in_secs,
    };
    let payload = {
        let mut gateway = state.gateway.lock().await;
        match gateway
            .create_task_payload(&state.config.multisig, &create_payload)
            .await
        {
            Ok(payload) => payload,
            Err(err) if gateway_reauthentication_required(&err) => {
                reauthenticate_rpc_gateway(state, &mut gateway).await?;
                gateway
                    .create_task_payload(&state.config.multisig, &create_payload)
                    .await?
            }
            Err(err) => return Err(err),
        }
    };
    let signature = state
        .signer
        .sign_and_verify_typed_data(&payload.signing_payload)?;
    let created = {
        let mut gateway = state.gateway.lock().await;
        match gateway
            .create_task(&state.config.multisig, &payload.challenge_id, &signature)
            .await
        {
            Ok(task) => task,
            Err(err) if gateway_reauthentication_required(&err) => {
                reauthenticate_rpc_gateway(state, &mut gateway).await?;
                gateway
                    .create_task(&state.config.multisig, &payload.challenge_id, &signature)
                    .await?
            }
            Err(err) => return Err(err),
        }
    };
    validate_gateway_creator(state, &created)?;
    Ok(created)
}

fn validate_create_task_policy(state: &RpcTaskState, request: &CreateTaskRequest) -> Result<()> {
    if state.config.dry_run {
        return Err(NodeError::Config(
            "RPC task creation is disabled in dry_run mode".to_string(),
        ));
    }
    if !state
        .config
        .allowed_creators
        .iter()
        .any(|creator| creator == state.signer.address_lc())
    {
        return Err(NodeError::Config(format!(
            "RPC task creator {} is not in allowed_creators",
            state.signer.address_lc()
        )));
    }

    let draft = TaskView {
        id: "rpc-preview".to_string(),
        multisig_address: state.config.multisig.clone(),
        creator: state.signer.address_lc().to_string(),
        leader: state.config.leader.clone(),
        nonce: 0,
        network: RPC_NETWORK.to_string(),
        template_id: request.template_id.clone(),
        template_version: request.template_version,
        inputs: request.inputs.clone(),
        signing_digest: None,
        creator_signature: None,
        action: None,
        threshold: 0,
        status: "pending".to_string(),
        signatures: Vec::new(),
        approvals: 0,
        rejects: 0,
        rejections: Vec::new(),
        created_at: now_secs(),
        expires_at: now_secs(),
        result: None,
    };
    let decision = evaluate(&state.config, &state.templates, &state.sub_accounts, &draft);
    if decision.is_reject() {
        return Err(NodeError::Config(format!(
            "RPC task rejected by local policy `{}`: {}",
            decision.rule,
            decision.reason.unwrap_or_else(|| "rejected".to_string())
        )));
    }
    Ok(())
}

fn validate_gateway_creator(state: &RpcTaskState, task: &TaskView) -> Result<()> {
    let creator = normalize_address(&task.creator)?;
    if creator != state.signer.address_lc() {
        warn!(
            task_id = task.id,
            task_creator = task.creator,
            signer = state.signer.address_lc(),
            "gateway returned task creator that does not match local signer"
        );
        return Err(NodeError::Gateway(format!(
            "gateway task creator {} does not match local signer {}",
            task.creator,
            state.signer.address_lc()
        )));
    }
    Ok(())
}

async fn reauthenticate_rpc_gateway(
    state: &RpcTaskState,
    gateway: &mut crate::gateway::GatewayClient,
) -> Result<()> {
    gateway.login(&state.signer).await?;
    let account = gateway.track_account(&state.config.multisig).await?;
    if !account.has_signer(state.signer.address_lc()) {
        return Err(NodeError::Runtime(format!(
            "signer {} is not an authorized signer of multisig {}",
            state.signer.address_lc(),
            state.config.multisig
        )));
    }
    Ok(())
}

fn gateway_reauthentication_required(err: &NodeError) -> bool {
    matches!(
        err,
        NodeError::Unauthorized | NodeError::SessionRenewalRequired
    )
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use axum::extract::{Path, Query, State};
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use rust_decimal::Decimal;
    use serde_json::{json, Value};

    use super::{index, router};
    use crate::config::{Config, SignerConfig};
    use crate::gateway::{
        GatewayClient, I18nText, SubAccountRegistry, TemplateField, TemplateFieldType,
        TemplateRegistry, TemplateView,
    };
    use crate::observe::debug_http::types::RpcTaskState;
    use crate::observe::debug_http::{DebugSnapshot, DebugStatus};
    use crate::signing::NodeSigner;
    use crate::state::StateStore;

    fn config_for(signer: &str, gateway_url: String) -> Config {
        Config {
            gateway_url,
            hl_api_url: "http://hl".to_string(),
            poll_interval_secs: 15,
            dry_run: false,
            allowed_templates: vec!["withdraw3".to_string(), "sub_account_withdraw3".to_string()],
            allowed_creators: vec![signer.to_string()],
            allowed_leaders: vec![signer.to_string()],
            template_input_policies: Default::default(),
            state_db: "sqlite::memory:".to_string(),
            rpc_http_addr: "127.0.0.1:9909".parse().unwrap(),
            rpc_auth_token: None,
            signer: SignerConfig {
                keystore_path: "config/signer.json".to_string(),
                password_env: None,
            },
            leader: signer.to_string(),
            multisig: "0x0000000000000000000000000000000000000002".to_string(),
            withdraw_limit: Decimal::new(10, 0),
        }
    }

    fn config() -> Config {
        config_for(
            "0x0000000000000000000000000000000000000001",
            "http://gateway".to_string(),
        )
    }

    fn debug_status() -> DebugStatus {
        DebugStatus {
            signer: "0x0000000000000000000000000000000000000003".to_string(),
            mode: "co-signer".to_string(),
            multisig: "0x0000000000000000000000000000000000000002".to_string(),
            leader: "0x0000000000000000000000000000000000000001".to_string(),
            last_poll_at: None,
            last_success_at: None,
            last_error: None,
            last_error_at: None,
            consecutive_gateway_failures: 0,
        }
    }

    #[tokio::test]
    async fn index_serves_web_view() {
        let html = index().await.0;

        assert!(html.contains("<title>safe-node</title>"));
        assert!(html.contains("Authorization Scope"));
        assert!(html.contains("Risk Controls"));
        assert!(html.contains("Default deny"));
        assert!(html.contains("Reject when"));
        assert!(html.contains("/debug/status"));
        assert!(html.contains("/debug/transactions"));
    }

    #[tokio::test]
    async fn root_route_serves_web_view() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let state = super::DebugAppState {
            snapshot: DebugSnapshot::new(debug_status()),
            state: StateStore::connect("sqlite::memory:").await.unwrap(),
            config: config().redacted(),
            task: None,
        };
        let app = router(state);
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let response = reqwest::get(format!("http://{addr}/")).await.unwrap();
        let status = response.status();
        let body = response.text().await.unwrap();
        server.abort();

        assert_eq!(status, reqwest::StatusCode::OK);
        assert!(body.contains("<title>safe-node</title>"));
    }

    #[tokio::test]
    async fn rpc_task_create_without_token_uses_node_signer() {
        let signer = NodeSigner::random_for_test();
        let gateway = TestGateway::spawn(signer.address_lc(), None).await;
        let config = config_for(signer.address_lc(), gateway.url());
        let addr = spawn_rpc_app(config, signer, None).await;

        let response = reqwest::Client::new()
            .post(format!("http://{addr}/rpc/task/create"))
            .json(&create_request())
            .send()
            .await
            .unwrap();
        let status = response.status();
        let body: Value = response.json().await.unwrap();

        assert_eq!(status, reqwest::StatusCode::OK);
        assert_eq!(
            body["creator"].as_str().unwrap().to_ascii_lowercase(),
            gateway.signer
        );
        assert_eq!(gateway.create_payload_count(), 1);
    }

    #[tokio::test]
    async fn rpc_task_create_requires_configured_token() {
        let signer = NodeSigner::random_for_test();
        let mut config = config_for(signer.address_lc(), "http://127.0.0.1:9".to_string());
        config.rpc_auth_token = Some("secret".to_string());
        let addr = spawn_rpc_app(config, signer, None).await;

        let response = reqwest::Client::new()
            .post(format!("http://{addr}/rpc/task/create"))
            .json(&create_request())
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn rpc_task_create_rejects_forbidden_identity_fields() {
        let signer = NodeSigner::random_for_test();
        let config = config_for(signer.address_lc(), "http://127.0.0.1:9".to_string());
        let addr = spawn_rpc_app(config, signer, None).await;
        let mut request = create_request();
        request["leader"] = json!("0x0000000000000000000000000000000000000003");

        let response = reqwest::Client::new()
            .post(format!("http://{addr}/rpc/task/create"))
            .json(&request)
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn rpc_task_create_rejects_signer_outside_allowed_creators() {
        let signer = NodeSigner::random_for_test();
        let mut config = config_for(signer.address_lc(), "http://127.0.0.1:9".to_string());
        config.allowed_creators = vec!["0x0000000000000000000000000000000000000001".to_string()];
        let addr = spawn_rpc_app(config, signer, None).await;

        let response = reqwest::Client::new()
            .post(format!("http://{addr}/rpc/task/create"))
            .json(&create_request())
            .send()
            .await
            .unwrap();
        let status = response.status();
        let body = response.text().await.unwrap();

        assert_eq!(status, reqwest::StatusCode::BAD_REQUEST);
        assert!(body.contains("allowed_creators"));
    }

    #[tokio::test]
    async fn rpc_task_create_rejects_gateway_creator_mismatch() {
        let signer = NodeSigner::random_for_test();
        let other = "0x0000000000000000000000000000000000000003";
        let gateway = TestGateway::spawn(signer.address_lc(), Some(other.to_string())).await;
        let config = config_for(signer.address_lc(), gateway.url());
        let addr = spawn_rpc_app(config, signer, None).await;

        let response = reqwest::Client::new()
            .post(format!("http://{addr}/rpc/task/create"))
            .json(&create_request())
            .send()
            .await
            .unwrap();
        let status = response.status();
        let body = response.text().await.unwrap();

        assert_eq!(status, reqwest::StatusCode::INTERNAL_SERVER_ERROR);
        assert!(body.contains("does not match local signer"));
    }

    async fn spawn_rpc_app(
        config: Config,
        signer: NodeSigner,
        gateway: Option<GatewayClient>,
    ) -> std::net::SocketAddr {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = RpcTaskState::new(
            config.clone(),
            signer,
            TemplateRegistry::new(vec![template()]),
            SubAccountRegistry::default(),
            gateway.unwrap_or_else(|| GatewayClient::new(config.gateway_url.clone())),
        );
        let state = super::DebugAppState {
            snapshot: DebugSnapshot::new(debug_status()),
            state: StateStore::connect("sqlite::memory:").await.unwrap(),
            config: config.redacted(),
            task: Some(task),
        };
        let app = router(state);
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        addr
    }

    fn create_request() -> Value {
        json!({
            "templateId": "withdraw3",
            "templateVersion": 1,
            "inputs": {
                "amount": "1"
            },
            "expiresInSecs": 3600
        })
    }

    fn template() -> TemplateView {
        TemplateView {
            id: "withdraw3".to_string(),
            version: 1,
            type_name: "withdraw".to_string(),
            hl_action_type: Some("withdraw3".to_string()),
            display_name: text("Withdraw"),
            description: text("Withdraw"),
            summary: text("Withdraw {{amount}}"),
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

    fn text(value: &str) -> I18nText {
        I18nText {
            en: value.to_string(),
            zh: value.to_string(),
        }
    }

    #[derive(Clone)]
    struct TestGateway {
        addr: std::net::SocketAddr,
        signer: String,
        create_payload_count: Arc<AtomicUsize>,
    }

    #[derive(Clone)]
    struct TestGatewayState {
        signer: String,
        creator_override: Option<String>,
        create_payload_count: Arc<AtomicUsize>,
        last_create_payload_body: Arc<Mutex<Option<Value>>>,
    }

    impl TestGateway {
        async fn spawn(signer: &str, creator_override: Option<String>) -> Self {
            let state = TestGatewayState {
                signer: signer.to_string(),
                creator_override,
                create_payload_count: Arc::new(AtomicUsize::new(0)),
                last_create_payload_body: Arc::new(Mutex::new(None)),
            };
            let app = Router::new()
                .route("/auth/sign-in-message", get(sign_in_message))
                .route("/auth/login", post(login))
                .route("/accounts", post(track_account))
                .route(
                    "/accounts/{multisig}/tasks/create-payload",
                    post(create_payload),
                )
                .route("/accounts/{multisig}/tasks", post(create_gateway_task))
                .with_state(state.clone());
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });
            Self {
                addr,
                signer: state.signer,
                create_payload_count: state.create_payload_count,
            }
        }

        fn url(&self) -> String {
            format!("http://{}", self.addr)
        }

        fn create_payload_count(&self) -> usize {
            self.create_payload_count.load(Ordering::SeqCst)
        }
    }

    async fn sign_in_message(
        State(state): State<TestGatewayState>,
        Query(_query): Query<HashMap<String, String>>,
    ) -> Json<Value> {
        Json(envelope(json!({
            "signer": state.signer,
            "timestamp": "1",
            "message": "safe-node test login"
        })))
    }

    async fn login() -> Json<Value> {
        Json(envelope(json!({
            "token": "token",
            "claims": {
                "nextEndTime": i64::MAX
            }
        })))
    }

    async fn track_account(State(state): State<TestGatewayState>) -> Json<Value> {
        Json(envelope(json!({
            "signers": [state.signer]
        })))
    }

    async fn create_payload(
        State(state): State<TestGatewayState>,
        Path(_multisig): Path<String>,
        Json(body): Json<Value>,
    ) -> Json<Value> {
        state.create_payload_count.fetch_add(1, Ordering::SeqCst);
        *state.last_create_payload_body.lock().unwrap() = Some(body);
        Json(envelope(json!({
            "challengeId": "challenge-1",
            "signingPayload": signing_payload(&state.signer)
        })))
    }

    async fn create_gateway_task(
        State(state): State<TestGatewayState>,
        Path(multisig): Path<String>,
        Json(_body): Json<Value>,
    ) -> Json<Value> {
        let creator = state.creator_override.unwrap_or(state.signer);
        Json(envelope(json!({
            "id": "task-1",
            "multisigAddress": multisig,
            "creator": creator,
            "leader": creator,
            "nonce": 1,
            "network": "mainnet",
            "templateId": "withdraw3",
            "templateVersion": 1,
            "inputs": { "amount": "1" },
            "signingDigest": "0x0000000000000000000000000000000000000000000000000000000000000000",
            "creatorSignature": "0x00",
            "threshold": 1,
            "status": "pending",
            "signatures": [],
            "approvals": 1,
            "rejects": 0,
            "rejections": [],
            "createdAt": 1,
            "expiresAt": 2
        })))
    }

    fn signing_payload(signer: &str) -> Value {
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
                "sender": signer,
                "nonce": 1
            }
        })
    }

    fn envelope(data: Value) -> Value {
        json!({
            "code": 0,
            "message": "ok",
            "data": data
        })
    }
}
