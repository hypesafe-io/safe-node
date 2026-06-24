use serde_json::Value;

use super::types::TuiData;
use crate::{HttpErrorContext, NodeError, Result};

pub(super) async fn fetch_data(client: &reqwest::Client, url: &str, limit: u32) -> TuiData {
    let base = url.trim_end_matches('/');
    let status = fetch_json(client, &format!("{base}/debug/status")).await;
    let config = fetch_json(client, &format!("{base}/debug/config")).await;
    let policy = fetch_json(client, &format!("{base}/debug/policy")).await;
    let transactions =
        fetch_json(client, &format!("{base}/debug/transactions?limit={limit}")).await;

    let mut data = TuiData::default();
    for result in [&status, &config, &policy, &transactions] {
        if let Err(err) = result {
            data.error = Some(err.to_string());
            break;
        }
    }
    data.status = status.ok();
    data.config = config.ok();
    data.policy = policy.ok();
    data.transactions = transactions
        .ok()
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default();
    data
}

async fn fetch_json(client: &reqwest::Client, url: &str) -> Result<Value> {
    let response = client.get(url).send().await?;
    let status = response.status();
    let context = HttpErrorContext::for_request("rpc_http.fetch", "GET", url)
        .with_response_headers(response.headers());
    let body = response.text().await?;
    if !status.is_success() {
        return Err(NodeError::HttpStatus {
            status: status.as_u16(),
            body,
            context,
        });
    }
    Ok(serde_json::from_str(&body)?)
}
