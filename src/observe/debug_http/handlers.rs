use axum::extract::{Query, State};
use axum::response::Html;
use axum::routing::get;
use axum::{Json, Router};

use super::types::{DebugAppState, DebugPolicy, DebugStatus, TransactionsQuery};
use super::web::INDEX_HTML;
use crate::config::RedactedConfig;
use crate::state::RecentTask;
use crate::Result;

pub(super) fn router(state: DebugAppState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/debug", get(index))
        .route("/debug/status", get(status))
        .route("/debug/transactions", get(transactions))
        .route("/debug/config", get(config))
        .route("/debug/policy", get(policy))
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
        withdraw_limit: state.config.withdraw_limit,
    })
}

#[cfg(test)]
mod tests {
    use rust_decimal::Decimal;

    use super::{index, router};
    use crate::config::{Config, SignerConfig};
    use crate::observe::debug_http::{DebugSnapshot, DebugStatus};
    use crate::state::StateStore;

    fn config() -> Config {
        Config {
            gateway_url: "http://gateway".to_string(),
            hl_api_url: "http://hl".to_string(),
            poll_interval_secs: 15,
            dry_run: true,
            allowed_templates: vec!["withdraw3".to_string(), "sub_account_withdraw3".to_string()],
            allowed_creators: vec!["0x0000000000000000000000000000000000000001".to_string()],
            state_db: "sqlite::memory:".to_string(),
            debug_http_addr: "127.0.0.1:9909".parse().unwrap(),
            signer: SignerConfig {
                keystore_path: "config/signer.json".to_string(),
                password_env: None,
            },
            leader: "0x0000000000000000000000000000000000000001".to_string(),
            multisig: "0x0000000000000000000000000000000000000002".to_string(),
            withdraw_limit: Decimal::new(10, 0),
        }
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
}
