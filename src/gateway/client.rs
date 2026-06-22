use serde::de::DeserializeOwned;
use serde_json::{json, Value};

use super::response::parse_response;
use super::types::{
    AccountView, OuterSigningPayload, SignInMessage, SigningPayload, SigningPayloadResponse,
    SubAccountRegistry, SubAccountView, TaskResultRequest, TaskView, TemplateView, TodoType,
    TokenResponse,
};
use crate::retry::sleep_backoff;
use crate::signing::NodeSigner;
use crate::state::now_secs;
use crate::{HttpErrorContext, NodeError, Result};
use tracing::warn;

const TOKEN_REFRESH_WINDOW_SECS: i64 = 10 * 60;
const GATEWAY_MAX_ATTEMPTS: usize = 4;
const ERROR_BODY_LOG_LIMIT: usize = 512;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RetryPolicy {
    RetrySafe,
    NoRetry,
}

impl RetryPolicy {
    const fn max_attempts(self) -> usize {
        match self {
            Self::RetrySafe => GATEWAY_MAX_ATTEMPTS,
            Self::NoRetry => 1,
        }
    }
}

pub(crate) struct GatewayClient {
    base_url: String,
    client: reqwest::Client,
    token: Option<AuthToken>,
}

struct AuthToken {
    token: String,
    next_end_time: i64,
}

impl GatewayClient {
    pub(crate) fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            client: reqwest::Client::new(),
            token: None,
        }
    }

    pub(crate) async fn login(&mut self, signer: &NodeSigner) -> Result<()> {
        let path = format!("/auth/sign-in-message?address={}", signer.address_lc());
        let message: SignInMessage = self
            .send(
                "auth.sign_in_message",
                reqwest::Method::GET,
                &path,
                None,
                false,
                RetryPolicy::RetrySafe,
            )
            .await?;
        let signature = signer.sign_message(&message.message)?;
        let body = json!({
            "signer": message.signer,
            "timestamp": message.timestamp,
            "signature": signature,
        });
        let response: TokenResponse = self
            .send(
                "auth.login",
                reqwest::Method::POST,
                "/auth/login",
                Some(body),
                false,
                RetryPolicy::RetrySafe,
            )
            .await?;
        self.store_token(response);
        Ok(())
    }

    async fn refresh_token(&mut self) -> Result<()> {
        let old_next_end_time = self
            .token
            .as_ref()
            .map(|token| token.next_end_time)
            .ok_or(NodeError::Unauthorized)?;
        let response: TokenResponse = self
            .send(
                "auth.refresh",
                reqwest::Method::POST,
                "/auth/refresh",
                None,
                true,
                RetryPolicy::RetrySafe,
            )
            .await?;
        let new_next_end_time = response.claims.next_end_time;
        self.store_token(response);
        if new_next_end_time <= old_next_end_time {
            return Err(NodeError::SessionRenewalRequired);
        }
        Ok(())
    }

    pub(crate) async fn track_account(&mut self, multisig: &str) -> Result<AccountView> {
        let body = json!({ "address": multisig, "alias": "safe-node" });
        self.send_auth(
            "accounts.track",
            reqwest::Method::POST,
            "/accounts",
            Some(body),
            RetryPolicy::NoRetry,
        )
        .await
    }

    pub(crate) async fn sub_accounts(&mut self, multisig: &str) -> Result<SubAccountRegistry> {
        let path = format!("/accounts/{multisig}/sub-accounts");
        let sub_accounts: Vec<SubAccountView> = self
            .send_auth(
                "accounts.sub_accounts.list",
                reqwest::Method::GET,
                &path,
                None,
                RetryPolicy::RetrySafe,
            )
            .await?;
        SubAccountRegistry::new(sub_accounts)
    }

    pub(crate) async fn templates(&self) -> Result<Vec<TemplateView>> {
        self.send(
            "templates.list",
            reqwest::Method::GET,
            "/templates",
            None,
            false,
            RetryPolicy::RetrySafe,
        )
        .await
    }

    pub(crate) async fn task_inbox(
        &mut self,
        todo_type: TodoType,
        multisig: &str,
    ) -> Result<Vec<TaskView>> {
        let path = format!(
            "/tasks/inbox?type={}&multisig={multisig}",
            todo_type.as_str()
        );
        let operation = format!("tasks.inbox.{}", todo_type.as_str());
        self.send_auth(
            &operation,
            reqwest::Method::GET,
            &path,
            None,
            RetryPolicy::RetrySafe,
        )
        .await
    }

    pub(crate) async fn signing_payload(&mut self, task_id: &str) -> Result<SigningPayload> {
        let path = format!("/tasks/{task_id}/signing-payload");
        let response: SigningPayloadResponse = self
            .send_auth(
                "tasks.signing_payload",
                reqwest::Method::GET,
                &path,
                None,
                RetryPolicy::RetrySafe,
            )
            .await?;
        Ok(SigningPayload {
            typed_data: response.signing_payload,
            signing_digest: response.signing_digest,
        })
    }

    pub(crate) async fn submit_signature(
        &mut self,
        task_id: &str,
        signature: &str,
    ) -> Result<TaskView> {
        let path = format!("/tasks/{task_id}/signatures");
        // NoRetry: a duplicate POST after response loss can cross the task
        // threshold and then fail as "not accepting signatures".
        self.send_auth(
            "tasks.submit_signature",
            reqwest::Method::POST,
            &path,
            Some(json!({ "signature": signature })),
            RetryPolicy::NoRetry,
        )
        .await
    }

    pub(crate) async fn reject_task(
        &mut self,
        task_id: &str,
        signer_lc: &str,
        reason: &str,
    ) -> Result<TaskView> {
        let path = format!("/tasks/{task_id}/reject");
        let task: TaskView = self
            // NoRetry: a duplicate rejection can terminalize the task or become
            // a business error after the first request is processed.
            .send_auth(
                "tasks.reject",
                reqwest::Method::POST,
                &path,
                Some(json!({ "reason": reason })),
                RetryPolicy::NoRetry,
            )
            .await?;
        if !task.has_rejection_from(signer_lc, reason) {
            return Err(NodeError::Gateway(format!(
                "gateway reject response did not include signer rejection; task_id={}, signer={}, \
                 status={}, rejects={}, rejections={}",
                task.id,
                signer_lc,
                task.status,
                task.rejects,
                task.rejections.len()
            )));
        }
        Ok(task)
    }

    pub(crate) async fn outer_signing_payload(
        &mut self,
        task_id: &str,
    ) -> Result<OuterSigningPayload> {
        let path = format!("/tasks/{task_id}/outer-signing-payload");
        self.send_auth(
            "tasks.outer_signing_payload",
            reqwest::Method::GET,
            &path,
            None,
            RetryPolicy::RetrySafe,
        )
        .await
    }

    pub(crate) async fn submit_result(
        &mut self,
        task_id: &str,
        result: &TaskResultRequest,
    ) -> Result<TaskView> {
        let path = format!("/tasks/{task_id}/result");
        let body = serde_json::to_value(result)?;
        // NoRetry: a successful first POST moves the gateway task to terminal;
        // retrying after response loss can turn success into a business error.
        self.send_auth(
            "tasks.submit_result",
            reqwest::Method::POST,
            &path,
            Some(body),
            RetryPolicy::NoRetry,
        )
        .await
    }

    async fn send_auth<T: DeserializeOwned>(
        &mut self,
        operation: &str,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
        retry_policy: RetryPolicy,
    ) -> Result<T> {
        self.ensure_valid_token().await?;
        self.send(operation, method, path, body, true, retry_policy)
            .await
    }

    async fn ensure_valid_token(&mut self) -> Result<()> {
        if self.token.is_none() {
            return Err(NodeError::Unauthorized);
        }
        if !self.token_refresh_due() {
            return Ok(());
        }

        self.refresh_token().await
    }

    fn store_token(&mut self, response: TokenResponse) {
        self.token = Some(AuthToken {
            token: response.token,
            next_end_time: response.claims.next_end_time,
        });
    }

    fn token_refresh_due(&self) -> bool {
        self.token
            .as_ref()
            .map(|token| should_refresh_token(token.next_end_time, now_secs()))
            .unwrap_or(false)
    }

    async fn send<T: DeserializeOwned>(
        &self,
        operation: &str,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
        auth: bool,
        retry_policy: RetryPolicy,
    ) -> Result<T> {
        let url = format!("{}{}", self.base_url, path);
        let mut last_err = None;
        let max_attempts = retry_policy.max_attempts();
        for attempt in 0..max_attempts {
            let mut request = self.client.request(method.clone(), &url);
            if auth {
                let token = self
                    .token
                    .as_ref()
                    .map(|token| token.token.as_str())
                    .ok_or(NodeError::Unauthorized)?;
                request = request.bearer_auth(token);
            }
            if let Some(body) = body.clone() {
                request = request.json(&body);
            }

            match request.send().await {
                Ok(response) => match parse_response(
                    response,
                    HttpErrorContext::for_request(operation, method.as_str(), path),
                )
                .await
                {
                    Ok(value) => return Ok(value),
                    Err(NodeError::Unauthorized) => return Err(NodeError::Unauthorized),
                    Err(err) if err.retryable() && attempt + 1 < max_attempts => {
                        log_retryable_error(
                            operation,
                            &method,
                            path,
                            attempt + 1,
                            max_attempts,
                            &err,
                        );
                        last_err = Some(err);
                        sleep_backoff(attempt).await;
                    }
                    Err(err) => {
                        log_final_error(operation, &method, path, attempt + 1, max_attempts, &err);
                        return Err(err);
                    }
                },
                Err(err) => {
                    let err = NodeError::Reqwest(err);
                    if err.retryable() && attempt + 1 < max_attempts {
                        log_retryable_error(
                            operation,
                            &method,
                            path,
                            attempt + 1,
                            max_attempts,
                            &err,
                        );
                        last_err = Some(err);
                        sleep_backoff(attempt).await;
                    } else {
                        log_final_error(operation, &method, path, attempt + 1, max_attempts, &err);
                        return Err(err);
                    }
                }
            }
        }
        Err(last_err.unwrap_or_else(|| NodeError::Gateway("request failed".to_string())))
    }
}

fn should_refresh_token(next_end_time: i64, now: i64) -> bool {
    now.saturating_add(TOKEN_REFRESH_WINDOW_SECS) >= next_end_time
}

fn log_retryable_error(
    operation: &str,
    method: &reqwest::Method,
    path: &str,
    attempt: usize,
    max_attempts: usize,
    err: &NodeError,
) {
    log_gateway_error(
        "gateway request failed; retrying",
        operation,
        method,
        path,
        attempt,
        max_attempts,
        err,
    );
}

fn log_final_error(
    operation: &str,
    method: &reqwest::Method,
    path: &str,
    attempt: usize,
    max_attempts: usize,
    err: &NodeError,
) {
    match err {
        NodeError::HttpStatus { .. } | NodeError::Reqwest(_) => {
            log_gateway_error(
                "gateway request failed",
                operation,
                method,
                path,
                attempt,
                max_attempts,
                err,
            );
        }
        _ => {}
    }
}

fn log_gateway_error(
    message: &'static str,
    operation: &str,
    method: &reqwest::Method,
    path: &str,
    attempt: usize,
    max_attempts: usize,
    err: &NodeError,
) {
    match err {
        NodeError::HttpStatus {
            status,
            body,
            context,
        } => {
            warn!(
                operation,
                method = %method,
                path,
                attempt,
                max_attempts,
                status,
                body = %truncate_body(body),
                cf_ray = context.cf_ray.as_deref().unwrap_or(""),
                request_id = context.request_id.as_deref().unwrap_or(""),
                error = %err,
                "{message}"
            );
        }
        NodeError::Reqwest(err) => {
            warn!(
                operation,
                method = %method,
                path,
                attempt,
                max_attempts,
                error = %err,
                "{message}"
            );
        }
        _ => {}
    }
}

fn truncate_body(body: &str) -> String {
    let mut chars = body.chars();
    let snippet = chars
        .by_ref()
        .take(ERROR_BODY_LOG_LIMIT)
        .collect::<String>();
    if chars.next().is_some() {
        format!("{snippet}...<truncated>")
    } else {
        snippet
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use axum::extract::{Path, Query, State};
    use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
    use axum::response::{IntoResponse, Response};
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use serde_json::{json, Value};

    use super::{should_refresh_token, AuthToken, GatewayClient};
    use crate::gateway::{TaskResultRequest, TodoType};
    use crate::signing::NodeSigner;
    use crate::NodeError;

    #[test]
    fn refresh_is_due_inside_ten_minute_window() {
        assert!(should_refresh_token(1_600, 1_000));
    }

    #[test]
    fn refresh_is_not_due_before_ten_minute_window() {
        assert!(!should_refresh_token(1_601, 1_000));
    }

    #[tokio::test]
    async fn auth_request_without_token_fails_before_network_request() {
        let server = TestGateway::spawn(false).await;
        let mut client = GatewayClient::new(server.url());

        let err = client
            .task_inbox(TodoType::Sign, "0x0000000000000000000000000000000000000001")
            .await
            .unwrap_err();

        assert!(matches!(err, NodeError::Unauthorized));
        assert_eq!(server.task_count(), 0);
    }

    #[tokio::test]
    async fn templates_request_is_public() {
        let server = TestGateway::spawn(false).await;
        let client = GatewayClient::new(server.url());

        let templates = client.templates().await.unwrap();

        assert_eq!(templates.len(), 1);
        assert_eq!(templates[0].id, "withdraw3");
        assert_eq!(server.template_count(), 1);
    }

    #[tokio::test]
    async fn auth_request_refreshes_due_token_before_request() {
        let server = TestGateway::spawn(false).await;
        let mut client = GatewayClient::new(server.url());
        client.token = Some(AuthToken {
            token: "old-token".to_string(),
            next_end_time: 0,
        });

        let tasks = client
            .task_inbox(TodoType::Sign, "0x0000000000000000000000000000000000000001")
            .await
            .unwrap();

        assert!(tasks.is_empty());
        assert_eq!(server.refresh_count(), 1);
        assert_eq!(server.task_count(), 1);
        assert_eq!(
            server.last_task_auth(),
            Some("Bearer refreshed-token".to_string())
        );
    }

    #[tokio::test]
    async fn auth_request_requests_session_renewal_when_refresh_stops_extending() {
        let server = TestGateway::spawn_with_options(TestGatewayOptions {
            refresh_next_end_time: 1_000,
            ..Default::default()
        })
        .await;
        let mut client = GatewayClient::new(server.url());
        client.token = Some(AuthToken {
            token: "old-token".to_string(),
            next_end_time: 1_000,
        });

        let err = client
            .task_inbox(TodoType::Sign, "0x0000000000000000000000000000000000000001")
            .await
            .unwrap_err();

        assert!(matches!(err, NodeError::SessionRenewalRequired));
        assert_eq!(server.refresh_count(), 1);
        assert_eq!(server.task_count(), 0);
    }

    #[tokio::test]
    async fn auth_request_returns_refresh_error_without_business_request() {
        let server = TestGateway::spawn(true).await;
        let mut client = GatewayClient::new(server.url());
        client.token = Some(AuthToken {
            token: "old-token".to_string(),
            next_end_time: 0,
        });

        let err = client
            .task_inbox(TodoType::Sign, "0x0000000000000000000000000000000000000001")
            .await
            .unwrap_err();

        assert!(matches!(err, NodeError::Unauthorized));
        assert_eq!(server.refresh_count(), 1);
        assert_eq!(server.task_count(), 0);
    }

    #[tokio::test]
    async fn client_can_relogin_after_refresh_unauthorized() {
        let server = TestGateway::spawn(true).await;
        let signer = NodeSigner::random_for_test();
        let mut client = GatewayClient::new(server.url());
        client.token = Some(AuthToken {
            token: "old-token".to_string(),
            next_end_time: 0,
        });

        let err = client
            .task_inbox(TodoType::Sign, "0x0000000000000000000000000000000000000001")
            .await
            .unwrap_err();

        assert!(matches!(err, NodeError::Unauthorized));
        assert_eq!(server.refresh_count(), 1);
        assert_eq!(server.task_count(), 0);

        client.login(&signer).await.unwrap();
        let tasks = client
            .task_inbox(TodoType::Sign, "0x0000000000000000000000000000000000000001")
            .await
            .unwrap();

        assert!(tasks.is_empty());
        assert_eq!(server.login_count(), 1);
        assert_eq!(server.task_count(), 1);
        assert_eq!(
            server.last_task_auth(),
            Some("Bearer login-token".to_string())
        );
    }

    #[tokio::test]
    async fn task_inbox_retries_retryable_502_then_succeeds() {
        let server = TestGateway::spawn_with_options(TestGatewayOptions {
            task_bad_gateway_failures: 2,
            ..Default::default()
        })
        .await;
        let mut client = GatewayClient::new(server.url());
        client.token = Some(AuthToken {
            token: "token".to_string(),
            next_end_time: i64::MAX,
        });

        let tasks = client
            .task_inbox(TodoType::Sign, "0x0000000000000000000000000000000000000001")
            .await
            .unwrap();

        assert!(tasks.is_empty());
        assert_eq!(server.task_count(), 3);
    }

    #[tokio::test]
    async fn submit_result_502_is_not_retried() {
        let server = TestGateway::spawn_with_options(TestGatewayOptions {
            result_bad_gateway_failures: 1,
            ..Default::default()
        })
        .await;
        let mut client = GatewayClient::new(server.url());
        client.token = Some(AuthToken {
            token: "token".to_string(),
            next_end_time: i64::MAX,
        });
        let result = TaskResultRequest {
            success: true,
            tx_hash: None,
            error: None,
            response: Some(json!({ "status": "ok" })),
        };

        let err = client.submit_result("task-1", &result).await.unwrap_err();

        match err {
            NodeError::HttpStatus {
                status, context, ..
            } => {
                assert_eq!(status, 502);
                assert_eq!(context.operation.as_deref(), Some("tasks.submit_result"));
                assert_eq!(context.method.as_deref(), Some("POST"));
                assert_eq!(context.path.as_deref(), Some("/tasks/task-1/result"));
            }
            other => panic!("unexpected error: {:?}", other),
        }
        assert_eq!(server.result_count(), 1);
    }

    #[tokio::test]
    async fn task_inbox_502_error_preserves_request_context() {
        let server = TestGateway::spawn_with_options(TestGatewayOptions {
            task_bad_gateway_failures: 4,
            ..Default::default()
        })
        .await;
        let mut client = GatewayClient::new(server.url());
        client.token = Some(AuthToken {
            token: "token".to_string(),
            next_end_time: i64::MAX,
        });

        let err = client
            .task_inbox(TodoType::Sign, "0x0000000000000000000000000000000000000001")
            .await
            .unwrap_err();
        let error_text = err.to_string();

        match err {
            NodeError::HttpStatus {
                status,
                body,
                context,
            } => {
                assert_eq!(status, 502);
                assert_eq!(body, "error code: 502");
                assert_eq!(context.operation.as_deref(), Some("tasks.inbox.sign"));
                assert_eq!(context.method.as_deref(), Some("GET"));
                assert_eq!(
                    context.path.as_deref(),
                    Some("/tasks/inbox?type=sign&multisig=0x0000000000000000000000000000000000000001")
                );
                assert_eq!(context.cf_ray.as_deref(), Some("task-ray"));
                assert_eq!(context.request_id.as_deref(), Some("task-request"));
            }
            other => panic!("unexpected error: {:?}", other),
        }
        assert!(error_text.contains("operation=tasks.inbox.sign"));
        assert!(error_text.contains("method=GET"));
        assert!(error_text.contains("cf_ray=task-ray"));
        assert!(!error_text.contains("Bearer"));
    }

    #[tokio::test]
    async fn refresh_502_error_does_not_send_business_request() {
        let server = TestGateway::spawn_with_options(TestGatewayOptions {
            refresh_bad_gateway_failures: 4,
            ..Default::default()
        })
        .await;
        let mut client = GatewayClient::new(server.url());
        client.token = Some(AuthToken {
            token: "old-token".to_string(),
            next_end_time: 0,
        });

        let err = client
            .task_inbox(TodoType::Sign, "0x0000000000000000000000000000000000000001")
            .await
            .unwrap_err();

        match err {
            NodeError::HttpStatus {
                status, context, ..
            } => {
                assert_eq!(status, 502);
                assert_eq!(context.operation.as_deref(), Some("auth.refresh"));
                assert_eq!(context.method.as_deref(), Some("POST"));
                assert_eq!(context.path.as_deref(), Some("/auth/refresh"));
                assert_eq!(context.cf_ray.as_deref(), Some("refresh-ray"));
            }
            other => panic!("unexpected error: {:?}", other),
        }
        assert_eq!(server.refresh_count(), 4);
        assert_eq!(server.task_count(), 0);
    }

    #[tokio::test]
    async fn sub_accounts_returns_normalized_registry() {
        let server = TestGateway::spawn(false).await;
        let mut client = GatewayClient::new(server.url());
        client.token = Some(AuthToken {
            token: "token".to_string(),
            next_end_time: i64::MAX,
        });

        let registry = client
            .sub_accounts("0x0000000000000000000000000000000000000001")
            .await
            .unwrap();

        assert_eq!(registry.len(), 2);
        assert!(registry.contains_normalized("0x00000000000000000000000000000000000000aa"));
        assert!(registry.contains_normalized("0x00000000000000000000000000000000000000bb"));
        assert_eq!(server.sub_account_count(), 1);
    }

    #[tokio::test]
    async fn reject_task_posts_reason() {
        let server = TestGateway::spawn(false).await;
        let mut client = GatewayClient::new(server.url());
        client.token = Some(AuthToken {
            token: "token".to_string(),
            next_end_time: i64::MAX,
        });

        let task = client
            .reject_task(
                "task-1",
                "0x0000000000000000000000000000000000000002",
                "withdraw amount exceeds withdraw_limit",
            )
            .await
            .unwrap();

        assert_eq!(task.id, "task-1");
        assert_eq!(server.reject_count(), 1);
        assert_eq!(
            server.last_reject_reason(),
            Some("withdraw amount exceeds withdraw_limit".to_string())
        );
    }

    #[tokio::test]
    async fn reject_task_requires_returned_rejection() {
        let server = TestGateway::spawn(false).await;
        let mut client = GatewayClient::new(server.url());
        client.token = Some(AuthToken {
            token: "token".to_string(),
            next_end_time: i64::MAX,
        });

        let err = client
            .reject_task(
                "task-1",
                "0x0000000000000000000000000000000000000002",
                "missing-rejection",
            )
            .await
            .unwrap_err();

        assert!(err.to_string().contains("did not include signer rejection"));
    }

    #[tokio::test]
    async fn business_error_preserves_envelope_data() {
        let server = TestGateway::spawn(false).await;
        let mut client = GatewayClient::new(server.url());
        client.token = Some(AuthToken {
            token: "token".to_string(),
            next_end_time: i64::MAX,
        });

        let err = client
            .reject_task(
                "task-1",
                "0x0000000000000000000000000000000000000002",
                "business-error",
            )
            .await
            .unwrap_err();

        match err {
            NodeError::GatewayBusiness { code, data, .. } => {
                assert_eq!(code, 4301);
                assert_eq!(data.get("scope").and_then(Value::as_str), Some("template"));
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    struct TestGateway {
        base_url: String,
        state: TestState,
    }

    impl TestGateway {
        async fn spawn(refresh_unauthorized: bool) -> Self {
            Self::spawn_with_options(TestGatewayOptions {
                refresh_unauthorized,
                ..Default::default()
            })
            .await
        }

        async fn spawn_with_options(options: TestGatewayOptions) -> Self {
            let state = TestState::new(options);
            let app = Router::new()
                .route("/auth/sign-in-message", get(sign_in_message))
                .route("/auth/login", post(login))
                .route("/auth/refresh", post(refresh))
                .route("/templates", get(templates))
                .route("/accounts/{multisig}/sub-accounts", get(sub_accounts))
                .route("/tasks/inbox", get(task_inbox))
                .route("/tasks/{task_id}/reject", post(reject_task))
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

        fn refresh_count(&self) -> usize {
            self.state.refresh_count.load(Ordering::SeqCst)
        }

        fn login_count(&self) -> usize {
            self.state.login_count.load(Ordering::SeqCst)
        }

        fn task_count(&self) -> usize {
            self.state.task_count.load(Ordering::SeqCst)
        }

        fn template_count(&self) -> usize {
            self.state.template_count.load(Ordering::SeqCst)
        }

        fn sub_account_count(&self) -> usize {
            self.state.sub_account_count.load(Ordering::SeqCst)
        }

        fn last_task_auth(&self) -> Option<String> {
            self.state.last_task_auth.lock().unwrap().clone()
        }

        fn reject_count(&self) -> usize {
            self.state.reject_count.load(Ordering::SeqCst)
        }

        fn result_count(&self) -> usize {
            self.state.result_count.load(Ordering::SeqCst)
        }

        fn last_reject_reason(&self) -> Option<String> {
            self.state.last_reject_reason.lock().unwrap().clone()
        }
    }

    struct TestGatewayOptions {
        refresh_unauthorized: bool,
        refresh_next_end_time: i64,
        task_bad_gateway_failures: usize,
        refresh_bad_gateway_failures: usize,
        result_bad_gateway_failures: usize,
    }

    impl Default for TestGatewayOptions {
        fn default() -> Self {
            Self {
                refresh_unauthorized: false,
                refresh_next_end_time: i64::MAX,
                task_bad_gateway_failures: 0,
                refresh_bad_gateway_failures: 0,
                result_bad_gateway_failures: 0,
            }
        }
    }

    #[derive(Clone)]
    struct TestState {
        refresh_unauthorized: Arc<AtomicBool>,
        refresh_next_end_time: i64,
        task_bad_gateway_failures: Arc<AtomicUsize>,
        refresh_bad_gateway_failures: Arc<AtomicUsize>,
        result_bad_gateway_failures: Arc<AtomicUsize>,
        login_count: Arc<AtomicUsize>,
        refresh_count: Arc<AtomicUsize>,
        task_count: Arc<AtomicUsize>,
        template_count: Arc<AtomicUsize>,
        sub_account_count: Arc<AtomicUsize>,
        reject_count: Arc<AtomicUsize>,
        result_count: Arc<AtomicUsize>,
        last_task_auth: Arc<Mutex<Option<String>>>,
        last_reject_reason: Arc<Mutex<Option<String>>>,
    }

    impl TestState {
        fn new(options: TestGatewayOptions) -> Self {
            Self {
                refresh_unauthorized: Arc::new(AtomicBool::new(options.refresh_unauthorized)),
                refresh_next_end_time: options.refresh_next_end_time,
                task_bad_gateway_failures: Arc::new(AtomicUsize::new(
                    options.task_bad_gateway_failures,
                )),
                refresh_bad_gateway_failures: Arc::new(AtomicUsize::new(
                    options.refresh_bad_gateway_failures,
                )),
                result_bad_gateway_failures: Arc::new(AtomicUsize::new(
                    options.result_bad_gateway_failures,
                )),
                login_count: Arc::new(AtomicUsize::new(0)),
                refresh_count: Arc::new(AtomicUsize::new(0)),
                task_count: Arc::new(AtomicUsize::new(0)),
                template_count: Arc::new(AtomicUsize::new(0)),
                sub_account_count: Arc::new(AtomicUsize::new(0)),
                reject_count: Arc::new(AtomicUsize::new(0)),
                result_count: Arc::new(AtomicUsize::new(0)),
                last_task_auth: Arc::new(Mutex::new(None)),
                last_reject_reason: Arc::new(Mutex::new(None)),
            }
        }
    }

    async fn sign_in_message(Query(query): Query<HashMap<String, String>>) -> Response {
        let signer = query.get("address").cloned().unwrap_or_default();
        Json(json!({
            "code": 0,
            "message": "ok",
            "data": {
                "signer": signer,
                "timestamp": "2026-06-17 00:00:00",
                "message": "Sign in to HypeSafe"
            }
        }))
        .into_response()
    }

    async fn login(State(state): State<TestState>) -> Response {
        state.login_count.fetch_add(1, Ordering::SeqCst);
        Json(json!({
            "code": 0,
            "message": "ok",
            "data": {
                "token": "login-token",
                "claims": {
                    "next_end_time": i64::MAX
                }
            }
        }))
        .into_response()
    }

    async fn refresh(State(state): State<TestState>) -> Response {
        state.refresh_count.fetch_add(1, Ordering::SeqCst);
        if consume_failure(&state.refresh_bad_gateway_failures) {
            let mut response = (StatusCode::BAD_GATEWAY, "error code: 502").into_response();
            response
                .headers_mut()
                .insert("cf-ray", HeaderValue::from_static("refresh-ray"));
            response
                .headers_mut()
                .insert("x-request-id", HeaderValue::from_static("refresh-request"));
            return response;
        }
        if state.refresh_unauthorized.load(Ordering::SeqCst) {
            return StatusCode::UNAUTHORIZED.into_response();
        }

        Json(json!({
            "code": 0,
            "message": "ok",
            "data": {
                "token": "refreshed-token",
                "claims": {
                    "next_end_time": state.refresh_next_end_time
                }
            }
        }))
        .into_response()
    }

    async fn templates(State(state): State<TestState>) -> Response {
        state.template_count.fetch_add(1, Ordering::SeqCst);
        Json(json!({
            "code": 0,
            "message": "ok",
            "data": [
                {
                    "id": "withdraw3",
                    "version": 1,
                    "typeName": "withdraw",
                    "hlActionType": "withdraw3",
                    "displayName": { "en": "Withdraw", "zh": "提现" },
                    "description": { "en": "Withdraw", "zh": "提现" },
                    "summary": { "en": "Withdraw {{amount}}", "zh": "提现 {{amount}}" },
                    "fields": [
                        {
                            "name": "amount",
                            "type": "amount",
                            "required": true,
                            "label": { "en": "Amount", "zh": "数量" },
                            "description": { "en": "Amount", "zh": "数量" }
                        }
                    ]
                }
            ]
        }))
        .into_response()
    }

    async fn sub_accounts(State(state): State<TestState>) -> Response {
        state.sub_account_count.fetch_add(1, Ordering::SeqCst);
        Json(json!({
            "code": 0,
            "message": "ok",
            "data": [
                {
                    "subAccountAddress": "0x00000000000000000000000000000000000000AA",
                    "name": "ops",
                    "createdAt": 0,
                    "updatedAt": 0,
                    "syncedAt": 0
                },
                {
                    "subAccountAddress": "0x00000000000000000000000000000000000000bb",
                    "name": "risk",
                    "createdAt": 0,
                    "updatedAt": 0,
                    "syncedAt": 0
                }
            ]
        }))
        .into_response()
    }

    async fn task_inbox(State(state): State<TestState>, headers: HeaderMap) -> Response {
        state.task_count.fetch_add(1, Ordering::SeqCst);
        let auth = headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        *state.last_task_auth.lock().unwrap() = auth;

        if consume_failure(&state.task_bad_gateway_failures) {
            let mut response = (StatusCode::BAD_GATEWAY, "error code: 502").into_response();
            response
                .headers_mut()
                .insert("cf-ray", HeaderValue::from_static("task-ray"));
            response
                .headers_mut()
                .insert("x-request-id", HeaderValue::from_static("task-request"));
            return response;
        }

        Json(json!({
            "code": 0,
            "message": "ok",
            "data": []
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

    async fn reject_task(State(state): State<TestState>, Json(body): Json<Value>) -> Response {
        state.reject_count.fetch_add(1, Ordering::SeqCst);
        let reason = body
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        *state.last_reject_reason.lock().unwrap() = Some(reason.clone());
        if reason == "business-error" {
            return Json(json!({
                "code": 4301,
                "message": "task creation blocked by a risk rule",
                "data": { "scope": "template" }
            }))
            .into_response();
        }
        if reason == "missing-rejection" {
            return Json(json!({
                "code": 0,
                "message": "ok",
                "data": task_view_without_rejection("task-1")
            }))
            .into_response();
        }

        Json(json!({
            "code": 0,
            "message": "ok",
            "data": task_view("task-1", &reason)
        }))
        .into_response()
    }

    async fn submit_result(
        State(state): State<TestState>,
        Path(task_id): Path<String>,
        Json(_body): Json<Value>,
    ) -> Response {
        state.result_count.fetch_add(1, Ordering::SeqCst);
        if consume_failure(&state.result_bad_gateway_failures) {
            let mut response = (StatusCode::BAD_GATEWAY, "error code: 502").into_response();
            response
                .headers_mut()
                .insert("cf-ray", HeaderValue::from_static("result-ray"));
            response
                .headers_mut()
                .insert("x-request-id", HeaderValue::from_static("result-request"));
            return response;
        }

        Json(json!({
            "code": 0,
            "message": "ok",
            "data": task_view(&task_id, "result written")
        }))
        .into_response()
    }

    fn task_view(task_id: &str, reject_reason: &str) -> Value {
        json!({
            "id": task_id,
            "multisigAddress": "0x0000000000000000000000000000000000000001",
            "creator": "0x0000000000000000000000000000000000000002",
            "leader": "0x0000000000000000000000000000000000000002",
            "nonce": 1,
            "network": "mainnet",
            "templateId": "withdraw3",
            "templateVersion": 1,
            "inputs": { "amount": "1" },
            "action": null,
            "threshold": 2,
            "status": "pending",
            "signatures": [],
            "approvals": 0,
            "rejects": 1,
            "rejections": [
                {
                    "signer": "0x0000000000000000000000000000000000000002",
                    "reason": reject_reason,
                    "rejectedAt": 0
                }
            ],
            "createdAt": 0,
            "expiresAt": 999,
            "result": null
        })
    }

    fn task_view_without_rejection(task_id: &str) -> Value {
        json!({
            "id": task_id,
            "multisigAddress": "0x0000000000000000000000000000000000000001",
            "creator": "0x0000000000000000000000000000000000000002",
            "leader": "0x0000000000000000000000000000000000000002",
            "nonce": 1,
            "network": "mainnet",
            "templateId": "withdraw3",
            "templateVersion": 1,
            "inputs": { "amount": "1" },
            "action": null,
            "threshold": 2,
            "status": "pending",
            "signatures": [],
            "approvals": 0,
            "rejects": 0,
            "rejections": [],
            "createdAt": 0,
            "expiresAt": 999,
            "result": null
        })
    }
}
