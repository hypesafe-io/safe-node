use std::convert::TryFrom;

pub(crate) async fn sleep_backoff(attempt: usize) {
    let exponent = u32::try_from(attempt).unwrap_or(u32::MAX);
    let millis = 250_u64.saturating_mul(2_u64.saturating_pow(exponent));
    tokio::time::sleep(std::time::Duration::from_millis(millis.min(5_000))).await;
}
