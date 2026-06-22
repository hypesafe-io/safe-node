use std::convert::TryFrom;

use serde::Serialize;
use serde_json::Value;

use super::task_state;
use crate::{NodeError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LocalStatus {
    Allow,
    Reject,
    Ignore,
    Signed,
    ExecuteSubmitted,
    ExecuteUnknown,
    ResultWritten,
    Failed,
}

impl LocalStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Reject => "reject",
            Self::Ignore => "ignore",
            Self::Signed => "signed",
            Self::ExecuteSubmitted => "execute_submitted",
            Self::ExecuteUnknown => "execute_unknown",
            Self::ResultWritten => "result_written",
            Self::Failed => "failed",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::LocalStatus;

    #[test]
    fn failed_status_has_boundary_string() {
        assert_eq!(LocalStatus::Failed.as_str(), "failed");
    }

    #[test]
    fn execute_unknown_status_has_boundary_string() {
        assert_eq!(LocalStatus::ExecuteUnknown.as_str(), "execute_unknown");
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PendingResult {
    pub(crate) hl_response: Value,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct RecentTask {
    pub(crate) task_id: String,
    pub(crate) multisig: String,
    pub(crate) template_id: String,
    pub(crate) template_version: i64,
    pub(crate) leader: String,
    pub(crate) nonce: i64,
    pub(crate) local_status: String,
    pub(crate) reject_reason: Option<String>,
    pub(crate) signature_result: Option<String>,
    pub(crate) hl_response: Option<Value>,
    pub(crate) gateway_result: Option<Value>,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
}

impl TryFrom<task_state::Model> for RecentTask {
    type Error = NodeError;

    fn try_from(model: task_state::Model) -> Result<Self> {
        Ok(Self {
            task_id: model.task_id,
            multisig: model.multisig,
            template_id: model.template_id,
            template_version: model.template_version,
            leader: model.leader,
            nonce: model.nonce,
            local_status: model.local_status,
            reject_reason: model.reject_reason,
            signature_result: model.signature_result,
            hl_response: model
                .hl_response_json
                .as_deref()
                .map(serde_json::from_str)
                .transpose()?,
            gateway_result: model
                .gateway_result_json
                .as_deref()
                .map(serde_json::from_str)
                .transpose()?,
            created_at: model.created_at,
            updated_at: model.updated_at,
        })
    }
}
