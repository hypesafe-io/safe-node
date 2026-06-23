use serde_json::Value;
use tracing::info;

use super::mode::RunMode;
use super::runner::Runner;
use super::signing_intent::{build_and_validate_outer_submission, build_and_validate_task_intent};
use crate::config::normalize_address;
use crate::gateway::{TaskResultRequest, TaskView, TodoType};
use crate::hyperliquid::classify_exchange_response;
use crate::policy::evaluate;
use crate::{NodeError, Result};

impl Runner {
    pub(super) async fn process_executable(&mut self) -> Result<()> {
        if self.mode != RunMode::LeaderExecutor {
            return Ok(());
        }
        let tasks = self
            .gateway
            .task_inbox(TodoType::Execute, &self.config.multisig)
            .await?;
        for task in tasks {
            if task.status != "executable" {
                continue;
            }
            if let Some(pending) = self.state.pending_result(&task.id).await? {
                self.write_gateway_result(&task, pending.hl_response)
                    .await?;
                continue;
            }
            if self.already_submitted_exchange(&task).await? {
                continue;
            }
            if self.state.is_ignored(&task.id).await? {
                continue;
            }

            if let Err(err) = build_and_validate_task_intent(&task, self.templates.by_task(&task)) {
                self.state.record_failed(&task, &err).await?;
                return Err(NodeError::Signer(err));
            }
            let decision = evaluate(&self.config, &self.templates, &self.sub_accounts, &task);
            if decision.is_reject() {
                self.submit_policy_reject(&task, decision.reason.as_deref())
                    .await?;
                continue;
            }
            let policy_rule = decision.rule;
            if self.config.dry_run {
                self.state.record_allowed(&task).await?;
                info!(
                    task_id = task.id,
                    policy_rule, "dry-run executable allow; not submitting HL"
                );
                continue;
            }

            if !self.task_requires_hyperliquid_submit(&task) {
                self.write_local_only_result(&task, &policy_rule).await?;
                continue;
            }
            if let Err(err) = self.ensure_current_signer_is_task_leader(&task) {
                self.state.record_failed(&task, &err).await?;
                return Err(NodeError::Signer(err));
            }

            let outer = match self.gateway.outer_signing_payload(&task.id).await {
                Ok(outer) => outer,
                Err(err) => {
                    self.state.record_failed(&task, &err.to_string()).await?;
                    return Err(err);
                }
            };
            let multisig_info = match self.hl.multisig_info(&task.multisig_address).await {
                Ok(info) => info,
                Err(err) if err.retryable() => return Err(err),
                Err(err) => {
                    self.state.record_failed(&task, &err.to_string()).await?;
                    return Err(err);
                }
            };
            let verified = match build_and_validate_outer_submission(
                &task,
                self.templates.by_task(&task),
                &outer,
                &multisig_info.authorized_users,
                multisig_info.threshold,
            ) {
                Ok(verified) => verified,
                Err(err) => {
                    self.state.record_failed(&task, &err).await?;
                    return Err(NodeError::Signer(err));
                }
            };
            let signature = match self.signer.sign_and_verify_typed_data(&verified.typed_data) {
                Ok(signature) => signature,
                Err(err) => {
                    self.state.record_failed(&task, &err.to_string()).await?;
                    return Err(err);
                }
            };
            self.state.record_exchange_submitted(&task).await?;
            info!(
                task_id = task.id,
                policy_rule, "submitting executable task to HL"
            );
            let hl_response = match self
                .hl
                .submit(
                    &verified.multi_sig_action,
                    task.nonce,
                    &signature,
                    verified.vault_address.as_deref(),
                )
                .await
            {
                Ok(response) => response,
                Err(err) => {
                    self.state
                        .record_execute_unknown(&task, &err.to_string())
                        .await?;
                    return Err(err);
                }
            };
            self.state.record_hl_response(&task, &hl_response).await?;
            self.write_gateway_result(&task, hl_response).await?;
        }
        Ok(())
    }

    fn task_requires_hyperliquid_submit(&self, task: &TaskView) -> bool {
        self.templates
            .by_task(task)
            .map(|template| template.requires_hyperliquid_submit())
            .unwrap_or(true)
    }

    async fn write_local_only_result(&mut self, task: &TaskView, policy_rule: &str) -> Result<()> {
        let response = serde_json::json!({
            "status": "ok",
            "localOnly": true,
            "templateId": task.template_id.clone(),
        });
        self.state.record_hl_response(task, &response).await?;
        info!(
            task_id = task.id,
            policy_rule, "writing local-only task result without HL submit"
        );
        self.write_gateway_result(task, response).await
    }

    async fn write_gateway_result(&mut self, task: &TaskView, hl_response: Value) -> Result<()> {
        let outcome = classify_exchange_response(&hl_response);
        let result = TaskResultRequest {
            success: outcome.success,
            tx_hash: None,
            error: outcome.error,
            response: Some(hl_response.clone()),
        };
        let updated = match self.gateway.submit_result(&task.id, &result).await {
            Ok(updated) => updated,
            Err(err) => return Err(err),
        };
        let gateway_result = serde_json::to_value(&updated)?;
        self.state
            .record_result_written(&updated, &hl_response, &gateway_result)
            .await?;
        info!(task_id = updated.id, "wrote gateway execution result");
        Ok(())
    }

    async fn already_submitted_exchange(&self, task: &TaskView) -> Result<bool> {
        self.state.has_submitted_exchange(&task.id).await
    }

    fn ensure_current_signer_is_task_leader(
        &self,
        task: &TaskView,
    ) -> std::result::Result<(), String> {
        let task_leader = normalize_address(&task.leader).map_err(|err| err.to_string())?;
        if task_leader != self.signer.address_lc() {
            return Err(format!(
                "task leader {} does not match local signer {}",
                task.leader,
                self.signer.address_lc()
            ));
        }
        Ok(())
    }
}
