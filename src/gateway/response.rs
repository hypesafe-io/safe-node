use serde::de::DeserializeOwned;

use super::types::Envelope;
use crate::{HttpErrorContext, NodeError, Result};

pub(super) async fn parse_response<T: DeserializeOwned>(
    response: reqwest::Response,
    context: HttpErrorContext,
) -> Result<T> {
    let status = response.status();
    let context = context.with_response_headers(response.headers());
    let body = response.text().await?;
    if status.as_u16() == 401 {
        return Err(NodeError::Unauthorized);
    }
    if !status.is_success() {
        return Err(NodeError::HttpStatus {
            status: status.as_u16(),
            body,
            context,
        });
    }
    let envelope: Envelope = serde_json::from_str(&body)?;
    if envelope.code != 0 {
        return Err(NodeError::GatewayBusiness {
            code: envelope.code,
            message: envelope.message,
            data: envelope.data,
        });
    }
    serde_json::from_value(envelope.data)
        .map_err(|err| NodeError::Gateway(format!("invalid response data: {err}")))
}
