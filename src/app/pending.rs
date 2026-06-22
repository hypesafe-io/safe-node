use tracing::info;

use super::runner::Runner;
use super::signing_intent::{build_and_validate_task_intent, validate_task_signing_payload_digest};
use crate::config::normalize_address;
use crate::gateway::{TaskView, TodoType};
use crate::policy::evaluate;
use crate::{NodeError, Result};

impl Runner {
    pub(super) async fn process_pending(&mut self) -> Result<()> {
        let tasks = self
            .gateway
            .task_inbox(TodoType::Sign, &self.config.multisig)
            .await?;
        for task in tasks {
            if self.state.is_ignored(&task.id).await? {
                continue;
            }
            if task.status != "pending" || self.already_signed(&task) {
                continue;
            }
            if self.state.has_signed(&task.id).await? {
                continue;
            }
            let local_typed_data =
                match build_and_validate_task_intent(&task, self.templates.by_task(&task)) {
                    Ok(local_typed_data) => local_typed_data,
                    Err(err) => {
                        self.state.record_failed(&task, &err).await?;
                        return Err(NodeError::Signer(err));
                    }
                };
            let decision = evaluate(&self.config, &self.templates, &self.sub_accounts, &task);
            if decision.is_reject() {
                self.submit_policy_reject(&task, decision.reason.as_deref())
                    .await?;
                continue;
            }
            let policy_rule = decision.rule;
            self.state.record_allowed(&task).await?;
            if self.config.dry_run {
                info!(task_id = task.id, policy_rule, "dry-run allow; not signing");
                continue;
            }

            let payload = match self.gateway.signing_payload(&task.id).await {
                Ok(payload) => payload,
                Err(err) => {
                    self.state.record_failed(&task, &err.to_string()).await?;
                    return Err(err);
                }
            };
            if let Err(err) = validate_task_signing_payload_digest(
                &payload.typed_data,
                task.signing_digest.as_deref(),
                payload.signing_digest.as_deref(),
            ) {
                self.state.record_failed(&task, &err).await?;
                return Err(NodeError::Signer(err));
            }
            let typed_data_to_sign = local_typed_data.as_ref().unwrap_or(&payload.typed_data);
            let signature = match self.signer.sign_and_verify_typed_data(typed_data_to_sign) {
                Ok(signature) => signature,
                Err(err) => {
                    self.state.record_failed(&task, &err.to_string()).await?;
                    return Err(err);
                }
            };
            let updated = match self.gateway.submit_signature(&task.id, &signature).await {
                Ok(updated) => updated,
                Err(err) => {
                    self.state.record_failed(&task, &err.to_string()).await?;
                    return Err(err);
                }
            };
            self.state.record_signed(&updated, "submitted").await?;
            info!(task_id = updated.id, "submitted task signature");
        }
        Ok(())
    }

    fn already_signed(&self, task: &TaskView) -> bool {
        task.signatures
            .iter()
            .filter_map(|sig| normalize_address(&sig.signer).ok())
            .any(|signer| signer == self.signer.address_lc())
    }
}
