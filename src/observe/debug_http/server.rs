use std::net::SocketAddr;

use tokio::net::TcpListener;
use tracing::warn;

use super::handlers::router;
use super::snapshot::DebugSnapshot;
use super::types::DebugAppState;
use crate::config::RedactedConfig;
use crate::state::StateStore;
use crate::{NodeError, Result};

pub(crate) fn spawn(
    addr: SocketAddr,
    snapshot: DebugSnapshot,
    state: StateStore,
    config: RedactedConfig,
) -> tokio::task::JoinHandle<Result<()>> {
    if requires_remote_debug_warning(addr) {
        warn!(
            debug_http_addr = %addr,
            "debug HTTP is bound to a non-localhost address; v1 has no remote access authentication"
        );
    }
    tokio::spawn(async move {
        let listener = TcpListener::bind(addr).await?;
        let app = router(DebugAppState {
            snapshot,
            state,
            config,
        });
        axum::serve(listener, app)
            .await
            .map_err(|err| NodeError::Runtime(format!("debug HTTP server failed: {err}")))
    })
}

fn requires_remote_debug_warning(addr: SocketAddr) -> bool {
    !addr.ip().is_loopback()
}

#[cfg(test)]
mod tests {
    use super::requires_remote_debug_warning;

    #[test]
    fn loopback_debug_addr_does_not_require_remote_warning() {
        assert!(!requires_remote_debug_warning(
            "127.0.0.1:9909".parse().unwrap()
        ));
    }

    #[test]
    fn non_loopback_debug_addr_requires_remote_warning() {
        assert!(requires_remote_debug_warning(
            "0.0.0.0:9909".parse().unwrap()
        ));
    }
}
