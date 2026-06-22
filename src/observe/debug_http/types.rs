use serde::{Deserialize, Serialize};

use crate::config::RedactedConfig;
use crate::state::StateStore;

#[derive(Debug, Clone, Serialize)]
pub(crate) struct DebugStatus {
    pub(crate) signer: String,
    pub(crate) mode: String,
    pub(crate) multisig: String,
    pub(crate) leader: String,
    pub(crate) last_poll_at: Option<i64>,
    pub(crate) last_success_at: Option<i64>,
    pub(crate) last_error: Option<String>,
    pub(crate) last_error_at: Option<i64>,
    pub(crate) consecutive_gateway_failures: u64,
}

#[derive(Clone)]
pub(super) struct DebugAppState {
    pub(super) snapshot: super::snapshot::DebugSnapshot,
    pub(super) state: StateStore,
    pub(super) config: RedactedConfig,
}

#[derive(Debug, Deserialize)]
pub(super) struct TransactionsQuery {
    pub(super) limit: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct DebugPolicy {
    pub(super) multisig: String,
    pub(super) leader: String,
    pub(super) allowed_templates: Vec<String>,
    pub(super) allowed_creators: Vec<String>,
    pub(super) withdraw_limit: String,
}
