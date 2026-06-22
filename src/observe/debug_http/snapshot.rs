use std::sync::Arc;

use tokio::sync::RwLock;

use super::types::DebugStatus;
use crate::state::now_secs;

#[derive(Clone)]
pub(crate) struct DebugSnapshot {
    inner: Arc<RwLock<DebugStatus>>,
}

impl DebugSnapshot {
    pub(crate) fn new(status: DebugStatus) -> Self {
        Self {
            inner: Arc::new(RwLock::new(status)),
        }
    }

    pub(crate) async fn mark_poll(&self) {
        let mut status = self.inner.write().await;
        status.last_poll_at = Some(now_secs());
    }

    pub(crate) async fn mark_success(&self) {
        let mut status = self.inner.write().await;
        status.last_success_at = Some(now_secs());
        status.last_error = None;
        status.consecutive_gateway_failures = 0;
    }

    pub(crate) async fn mark_error(&self, error: impl ToString, consecutive_gateway_failures: u64) {
        let mut status = self.inner.write().await;
        status.last_error_at = Some(now_secs());
        status.last_error = Some(error.to_string());
        status.consecutive_gateway_failures = consecutive_gateway_failures;
    }

    pub(super) async fn read(&self) -> DebugStatus {
        self.inner.read().await.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::DebugSnapshot;
    use crate::observe::debug_http::DebugStatus;

    fn status() -> DebugStatus {
        DebugStatus {
            signer: "0x0000000000000000000000000000000000000001".to_string(),
            mode: "co-signer".to_string(),
            multisig: "0x0000000000000000000000000000000000000002".to_string(),
            leader: "0x0000000000000000000000000000000000000003".to_string(),
            last_poll_at: None,
            last_success_at: None,
            last_error: None,
            last_error_at: None,
            consecutive_gateway_failures: 0,
        }
    }

    #[tokio::test]
    async fn poll_does_not_clear_last_error() {
        let snapshot = DebugSnapshot::new(status());

        snapshot.mark_error("http status 502", 1).await;
        snapshot.mark_poll().await;
        let status = snapshot.read().await;

        assert!(status.last_poll_at.is_some());
        assert_eq!(status.last_error.as_deref(), Some("http status 502"));
        assert!(status.last_error_at.is_some());
        assert_eq!(status.consecutive_gateway_failures, 1);
    }

    #[tokio::test]
    async fn success_clears_error_and_gateway_failure_count() {
        let snapshot = DebugSnapshot::new(status());

        snapshot.mark_error("http status 502", 2).await;
        snapshot.mark_success().await;
        let status = snapshot.read().await;

        assert!(status.last_success_at.is_some());
        assert!(status.last_error.is_none());
        assert_eq!(status.consecutive_gateway_failures, 0);
    }
}
