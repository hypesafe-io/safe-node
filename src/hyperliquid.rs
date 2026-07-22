use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::retry::sleep_backoff;
use crate::{HttpErrorContext, NodeError, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExchangeOutcome {
    pub(crate) success: bool,
    pub(crate) error: Option<String>,
}

impl ExchangeOutcome {
    fn success() -> Self {
        Self {
            success: true,
            error: None,
        }
    }

    fn failed(error: String) -> Self {
        Self {
            success: false,
            error: Some(error),
        }
    }
}

#[derive(Debug, Serialize)]
struct HlSignature {
    r: String,
    s: String,
    v: u8,
}

#[derive(Debug, Serialize)]
struct ExchangeRequest<'a> {
    action: &'a Value,
    nonce: i64,
    signature: HlSignature,
    #[serde(rename = "vaultAddress", skip_serializing_if = "Option::is_none")]
    vault_address: Option<&'a str>,
    #[serde(rename = "expiresAfter")]
    expires_after: Option<u64>,
}

#[derive(Debug, Clone)]
pub(crate) struct MultiSigInfo {
    pub(crate) authorized_users: Vec<String>,
    pub(crate) threshold: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UserToMultiSigSigners {
    authorized_users: Vec<String>,
    threshold: i64,
}

pub struct HlExchangeClient {
    base_url: String,
    client: reqwest::Client,
}

impl HlExchangeClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            client: reqwest::Client::new(),
        }
    }

    pub async fn multisig_info(&self, address: &str) -> Result<MultiSigInfo> {
        let body = serde_json::json!({ "type": "userToMultiSigSigners", "user": address });
        let parsed: Option<UserToMultiSigSigners> = self
            .post_info("hyperliquid.info.user_to_multi_sig_signers", body)
            .await?;
        let Some(parsed) = parsed else {
            return Err(NodeError::Hyperliquid(format!(
                "{address} is not a multisig account on Hyperliquid"
            )));
        };
        Ok(MultiSigInfo {
            authorized_users: parsed.authorized_users,
            threshold: parsed.threshold,
        })
    }

    pub async fn submit(
        &self,
        action: &Value,
        nonce: i64,
        signature: &str,
        vault_address: Option<&str>,
        expires_after: Option<u64>,
    ) -> Result<Value> {
        let signature = signature_to_hl(signature)?;
        let request = ExchangeRequest {
            action,
            nonce,
            signature,
            vault_address,
            expires_after,
        };
        let url = format!("{}/exchange", self.base_url);
        let mut last_err = None;
        for attempt in 0..=3 {
            match self.client.post(&url).json(&request).send().await {
                Ok(response) => {
                    let status = response.status();
                    let context =
                        HttpErrorContext::for_request("hyperliquid.exchange", "POST", "/exchange")
                            .with_response_headers(response.headers());
                    let body = response.text().await?;
                    if !status.is_success() {
                        let err = NodeError::HttpStatus {
                            status: status.as_u16(),
                            body,
                            context,
                        };
                        if err.retryable() && attempt < 3 {
                            last_err = Some(err);
                            sleep_backoff(attempt).await;
                            continue;
                        }
                        return Err(err);
                    }
                    return serde_json::from_str(&body)
                        .map_err(|err| NodeError::Hyperliquid(format!("invalid response: {err}")));
                }
                Err(err) if attempt < 3 => {
                    last_err = Some(NodeError::Reqwest(err));
                    sleep_backoff(attempt).await;
                }
                Err(err) => return Err(NodeError::Reqwest(err)),
            }
        }
        Err(last_err.unwrap_or_else(|| NodeError::Hyperliquid("exchange failed".to_string())))
    }

    async fn post_info<T: DeserializeOwned>(&self, operation: &str, body: Value) -> Result<T> {
        let url = format!("{}/info", self.base_url);
        let mut last_err = None;
        for attempt in 0..=3 {
            match self.client.post(&url).json(&body).send().await {
                Ok(response) => {
                    let status = response.status();
                    let context = HttpErrorContext::for_request(operation, "POST", "/info")
                        .with_response_headers(response.headers());
                    let body = response.text().await?;
                    if !status.is_success() {
                        let err = NodeError::HttpStatus {
                            status: status.as_u16(),
                            body,
                            context,
                        };
                        if err.retryable() && attempt < 3 {
                            last_err = Some(err);
                            sleep_backoff(attempt).await;
                            continue;
                        }
                        return Err(err);
                    }
                    return serde_json::from_str(&body).map_err(|err| {
                        NodeError::Hyperliquid(format!("invalid info response: {err}"))
                    });
                }
                Err(err) if attempt < 3 => {
                    last_err = Some(NodeError::Reqwest(err));
                    sleep_backoff(attempt).await;
                }
                Err(err) => return Err(NodeError::Reqwest(err)),
            }
        }
        Err(last_err.unwrap_or_else(|| NodeError::Hyperliquid("info request failed".to_string())))
    }
}

pub(crate) fn classify_exchange_response(response: &Value) -> ExchangeOutcome {
    if response.get("status").and_then(Value::as_str) != Some("ok") {
        return ExchangeOutcome::failed(
            readable_error(response).unwrap_or_else(|| response.to_string()),
        );
    }

    let errors = response
        .pointer("/response/data/statuses")
        .and_then(Value::as_array)
        .map(|statuses| statuses.iter().filter_map(status_error).collect::<Vec<_>>())
        .unwrap_or_default();
    if errors.is_empty() {
        ExchangeOutcome::success()
    } else {
        ExchangeOutcome::failed(errors.join("; "))
    }
}

fn status_error(status: &Value) -> Option<String> {
    status
        .get("error")
        .map(readable_value)
        .or_else(|| status.get("err").map(readable_value))
}

fn readable_error(value: &Value) -> Option<String> {
    value
        .get("error")
        .map(readable_value)
        .or_else(|| value.get("message").map(readable_value))
        .or_else(|| value.pointer("/response/error").map(readable_value))
        .or_else(|| value.pointer("/response/data/error").map(readable_value))
}

fn readable_value(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| value.to_string())
}

fn signature_to_hl(signature: &str) -> Result<HlSignature> {
    let raw = signature.trim().trim_start_matches("0x");
    if raw.len() != 130 || !raw.as_bytes().iter().all(u8::is_ascii_hexdigit) {
        return Err(NodeError::Hyperliquid("malformed signature".to_string()));
    }
    let recovery = u8::from_str_radix(&raw[128..130], 16)
        .map_err(|err| NodeError::Hyperliquid(format!("bad recovery id: {err}")))?;
    let v = match recovery {
        0 | 1 => recovery + 27,
        27 | 28 => recovery,
        _ => {
            return Err(NodeError::Hyperliquid(
                "unsupported signature recovery id".to_string(),
            ));
        }
    };
    Ok(HlSignature {
        r: format!("0x{}", &raw[0..64]),
        s: format!("0x{}", &raw[64..128]),
        v,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::classify_exchange_response;
    use super::signature_to_hl;

    #[test]
    fn converts_signature_v() {
        let sig = format!("0x{}{}00", "11".repeat(32), "22".repeat(32));
        let hl = signature_to_hl(&sig).unwrap();
        assert_eq!(hl.v, 27);
    }

    #[test]
    fn classifies_action_status_error_as_failure() {
        let response = json!({
            "status": "ok",
            "response": {
                "data": {
                    "statuses": [
                        { "resting": { "oid": 1 } },
                        { "error": "Insufficient margin" }
                    ]
                }
            }
        });

        let outcome = classify_exchange_response(&response);

        assert!(!outcome.success);
        assert_eq!(outcome.error.as_deref(), Some("Insufficient margin"));
    }

    #[test]
    fn classifies_top_level_non_ok_as_failure() {
        let response = json!({
            "status": "err",
            "response": { "error": "bad signature" }
        });

        let outcome = classify_exchange_response(&response);

        assert!(!outcome.success);
        assert_eq!(outcome.error.as_deref(), Some("bad signature"));
    }

    #[test]
    fn classifies_local_only_ok_as_success() {
        let response = json!({
            "status": "ok",
            "localOnly": true
        });

        let outcome = classify_exchange_response(&response);

        assert!(outcome.success);
        assert_eq!(outcome.error, None);
    }
}
