use std::fmt;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use reqwest::header::HeaderMap;
use serde_json::Value;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, NodeError>;

#[derive(Debug, Error)]
pub enum NodeError {
    #[error("config error: {0}")]
    Config(String),
    #[error("signer error: {0}")]
    Signer(String),
    #[error("gateway error: {0}")]
    Gateway(String),
    #[error("gateway business error {code}: {message}; data: {data}")]
    GatewayBusiness {
        code: i32,
        message: String,
        data: Value,
    },
    #[error("gateway authentication failed")]
    Unauthorized,
    #[error("gateway session refresh reached lifetime ceiling")]
    SessionRenewalRequired,
    #[error("hyperliquid error: {0}")]
    Hyperliquid(String),
    #[error("runtime error: {0}")]
    Runtime(String),
    #[error("http status {status}: {body}{context}")]
    HttpStatus {
        status: u16,
        body: String,
        context: HttpErrorContext,
    },
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Reqwest(#[from] reqwest::Error),
    #[error(transparent)]
    SeaOrm(#[from] sea_orm::DbErr),
}

impl From<hypesafe_signing_intent::IntentError> for NodeError {
    fn from(err: hypesafe_signing_intent::IntentError) -> Self {
        Self::Signer(err.to_string())
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HttpErrorContext {
    pub operation: Option<String>,
    pub method: Option<String>,
    pub path: Option<String>,
    pub cf_ray: Option<String>,
    pub request_id: Option<String>,
}

impl HttpErrorContext {
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    #[must_use]
    pub(crate) fn for_request(
        operation: impl Into<String>,
        method: impl Into<String>,
        path: impl Into<String>,
    ) -> Self {
        Self {
            operation: Some(operation.into()),
            method: Some(method.into()),
            path: Some(path.into()),
            cf_ray: None,
            request_id: None,
        }
    }

    #[must_use]
    pub(crate) fn with_response_headers(mut self, headers: &HeaderMap) -> Self {
        self.cf_ray = header_value(headers, "cf-ray");
        self.request_id = header_value(headers, "x-request-id");
        self
    }
}

impl fmt::Display for HttpErrorContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = Vec::new();
        if let Some(value) = self.operation.as_deref() {
            parts.push(format!("operation={value}"));
        }
        if let Some(value) = self.method.as_deref() {
            parts.push(format!("method={value}"));
        }
        if let Some(value) = self.path.as_deref() {
            parts.push(format!("path={value}"));
        }
        if let Some(value) = self.cf_ray.as_deref() {
            parts.push(format!("cf_ray={value}"));
        }
        if let Some(value) = self.request_id.as_deref() {
            parts.push(format!("request_id={value}"));
        }
        if parts.is_empty() {
            Ok(())
        } else {
            write!(f, " ({})", parts.join(", "))
        }
    }
}

fn header_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

impl NodeError {
    #[must_use]
    pub fn retryable(&self) -> bool {
        match self {
            Self::Reqwest(err) => err.is_timeout() || err.is_connect() || err.is_request(),
            Self::HttpStatus { status, .. } => *status >= 500 || *status == 429,
            _ => false,
        }
    }
}

impl IntoResponse for NodeError {
    fn into_response(self) -> Response {
        let status = match &self {
            Self::Config(_) => StatusCode::BAD_REQUEST,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::HttpStatus { status, .. } => {
                StatusCode::from_u16(*status).unwrap_or(StatusCode::BAD_GATEWAY)
            }
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let body = serde_json::json!({
            "error": self.to_string(),
        });
        (status, axum::Json(body)).into_response()
    }
}
