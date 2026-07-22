use std::net::SocketAddr;

use tokio::net::TcpListener;
use tracing::warn;

use super::handlers::router;
use super::snapshot::DebugSnapshot;
use super::types::DebugAppState;
use crate::config::{Config, RedactedConfig};
use crate::gateway::{GatewayClient, SharedSubAccountRegistry, TemplateRegistry};
use crate::signing::NodeSigner;
use crate::state::StateStore;
use crate::{NodeError, Result};

pub(crate) fn spawn(
    addr: SocketAddr,
    snapshot: DebugSnapshot,
    state: StateStore,
    config: RedactedConfig,
    task_config: Config,
    signer: NodeSigner,
    templates: TemplateRegistry,
    sub_accounts: SharedSubAccountRegistry,
    gateway: GatewayClient,
) -> tokio::task::JoinHandle<Result<()>> {
    if requires_remote_rpc_auth_warning(addr, config.rpc_auth_token_configured) {
        warn!(
            rpc_http_addr = %addr,
            "RPC HTTP is bound to a non-localhost address without rpc_auth_token"
        );
    }
    tokio::spawn(async move {
        let listener = TcpListener::bind(addr).await?;
        let app = router(DebugAppState {
            snapshot,
            state,
            config,
            task: Some(super::types::RpcTaskState::new(
                task_config,
                signer,
                templates,
                sub_accounts,
                gateway,
            )),
        });
        axum::serve(listener, app)
            .await
            .map_err(|err| NodeError::Runtime(format!("RPC HTTP server failed: {err}")))
    })
}

fn requires_remote_rpc_auth_warning(addr: SocketAddr, auth_token_configured: bool) -> bool {
    !auth_token_configured && !addr.ip().is_loopback()
}

#[cfg(test)]
mod tests {
    use super::requires_remote_rpc_auth_warning;

    #[test]
    fn loopback_rpc_addr_does_not_require_remote_warning() {
        assert!(!requires_remote_rpc_auth_warning(
            "127.0.0.1:9909".parse().unwrap(),
            false
        ));
    }

    #[test]
    fn non_loopback_rpc_addr_without_auth_requires_remote_warning() {
        assert!(requires_remote_rpc_auth_warning(
            "0.0.0.0:9909".parse().unwrap(),
            false
        ));
    }

    #[test]
    fn non_loopback_rpc_addr_with_auth_does_not_require_remote_warning() {
        assert!(!requires_remote_rpc_auth_warning(
            "0.0.0.0:9909".parse().unwrap(),
            true
        ));
    }
}
