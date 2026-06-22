use std::convert::TryFrom;

use sea_orm::{
    ActiveModelTrait, ConnectOptions, ConnectionTrait, Database, DatabaseConnection, EntityTrait,
    QueryOrder, QuerySelect, Set,
};
use serde_json::Value;
use tracing::info;

use super::clock::now_secs;
use super::sqlite::{ensure_sqlite_parent, sea_orm_sqlite_url};
use super::task_state;
use super::types::{LocalStatus, PendingResult, RecentTask};
use crate::gateway::TaskView;
use crate::Result;

#[derive(Clone)]
pub(crate) struct StateStore {
    db: DatabaseConnection,
}

impl StateStore {
    pub(crate) async fn connect(raw_url: &str) -> Result<Self> {
        ensure_sqlite_parent(raw_url)?;
        let url = sea_orm_sqlite_url(raw_url);
        let mut options = ConnectOptions::new(url);
        options.sqlx_logging(false);
        let db = Database::connect(options).await?;
        let store = Self { db };
        store.create_schema().await?;
        Ok(store)
    }

    pub(crate) async fn record_allowed(&self, task: &TaskView) -> Result<()> {
        self.record_status(task, LocalStatus::Allow, None, None, None, None)
            .await
    }

    pub(crate) async fn record_rejected(&self, task: &TaskView, reason: &str) -> Result<()> {
        self.record_status(task, LocalStatus::Reject, Some(reason), None, None, None)
            .await
    }

    pub(crate) async fn record_ignored(&self, task: &TaskView, reason: &str) -> Result<()> {
        self.record_status(task, LocalStatus::Ignore, Some(reason), None, None, None)
            .await
    }

    pub(crate) async fn record_signed(
        &self,
        task: &TaskView,
        signature_result: &str,
    ) -> Result<()> {
        self.record_status(
            task,
            LocalStatus::Signed,
            None,
            Some(signature_result),
            None,
            None,
        )
        .await
    }

    pub(crate) async fn has_signed(&self, task_id: &str) -> Result<bool> {
        let Some(model) = self.execution_state(task_id).await? else {
            return Ok(false);
        };
        Ok(model.local_status == LocalStatus::Signed.as_str())
    }

    pub(crate) async fn record_exchange_submitted(&self, task: &TaskView) -> Result<()> {
        self.record_status(task, LocalStatus::ExecuteSubmitted, None, None, None, None)
            .await
    }

    pub(crate) async fn record_execute_unknown(&self, task: &TaskView, reason: &str) -> Result<()> {
        self.record_status(
            task,
            LocalStatus::ExecuteUnknown,
            Some(reason),
            None,
            None,
            None,
        )
        .await
    }

    pub(crate) async fn record_hl_response(
        &self,
        task: &TaskView,
        hl_response: &Value,
    ) -> Result<()> {
        self.record_status(
            task,
            LocalStatus::ExecuteSubmitted,
            None,
            None,
            Some(hl_response),
            None,
        )
        .await
    }

    pub(crate) async fn record_result_written(
        &self,
        task: &TaskView,
        hl_response: &Value,
        gateway_result: &Value,
    ) -> Result<()> {
        self.record_status(
            task,
            LocalStatus::ResultWritten,
            None,
            None,
            Some(hl_response),
            Some(gateway_result),
        )
        .await
    }

    pub(crate) async fn record_failed(&self, task: &TaskView, reason: &str) -> Result<()> {
        self.record_status(task, LocalStatus::Failed, Some(reason), None, None, None)
            .await
    }

    async fn record_status(
        &self,
        task: &TaskView,
        status: LocalStatus,
        reject_reason: Option<&str>,
        signature_result: Option<&str>,
        hl_response: Option<&Value>,
        gateway_result: Option<&Value>,
    ) -> Result<()> {
        // For append-only details, None means "preserve the existing value" on
        // updates. The semantic record_* wrappers own the transition rules.
        let now = now_secs();
        let hl_response_json = hl_response.map(serde_json::to_string).transpose()?;
        let gateway_result_json = gateway_result.map(serde_json::to_string).transpose()?;

        if let Some(model) = task_state::Entity::find_by_id(task.id.clone())
            .one(&self.db)
            .await?
        {
            let mut active: task_state::ActiveModel = model.into();
            active.multisig = Set(task.multisig_address.clone());
            active.template_id = Set(task.template_id.clone());
            active.template_version = Set(task.template_version);
            active.leader = Set(task.leader.clone());
            active.nonce = Set(task.nonce);
            active.local_status = Set(status.as_str().to_string());
            active.reject_reason = Set(reject_reason.map(str::to_string));
            if signature_result.is_some() {
                active.signature_result = Set(signature_result.map(str::to_string));
            }
            if hl_response_json.is_some() {
                active.hl_response_json = Set(hl_response_json);
            }
            if gateway_result_json.is_some() {
                active.gateway_result_json = Set(gateway_result_json);
            }
            active.updated_at = Set(now);
            active.update(&self.db).await?;
        } else {
            task_state::ActiveModel {
                task_id: Set(task.id.clone()),
                multisig: Set(task.multisig_address.clone()),
                template_id: Set(task.template_id.clone()),
                template_version: Set(task.template_version),
                leader: Set(task.leader.clone()),
                nonce: Set(task.nonce),
                local_status: Set(status.as_str().to_string()),
                reject_reason: Set(reject_reason.map(str::to_string)),
                signature_result: Set(signature_result.map(str::to_string)),
                hl_response_json: Set(hl_response_json),
                gateway_result_json: Set(gateway_result_json),
                created_at: Set(now),
                updated_at: Set(now),
            }
            .insert(&self.db)
            .await?;
        }
        Ok(())
    }

    pub(crate) async fn pending_result(&self, task_id: &str) -> Result<Option<PendingResult>> {
        let Some(model) = self.execution_state(task_id).await? else {
            return Ok(None);
        };
        // execute_submitted + hl_response_json + no gateway_result_json means
        // HL has replied and only the gateway result write-back is still pending.
        if model.local_status != LocalStatus::ExecuteSubmitted.as_str()
            || model.gateway_result_json.is_some()
        {
            return Ok(None);
        }
        let Some(raw) = model.hl_response_json else {
            return Ok(None);
        };
        let hl_response = serde_json::from_str(&raw)?;
        Ok(Some(PendingResult { hl_response }))
    }

    pub(crate) async fn has_submitted_exchange(&self, task_id: &str) -> Result<bool> {
        let Some(model) = self.execution_state(task_id).await? else {
            return Ok(false);
        };
        Ok(model.local_status == LocalStatus::ExecuteSubmitted.as_str()
            || model.local_status == LocalStatus::ExecuteUnknown.as_str()
            || model.local_status == LocalStatus::ResultWritten.as_str())
    }

    pub(crate) async fn is_ignored(&self, task_id: &str) -> Result<bool> {
        let Some(model) = self.execution_state(task_id).await? else {
            return Ok(false);
        };
        Ok(model.local_status == LocalStatus::Ignore.as_str())
    }

    pub(crate) async fn recent(&self, limit: u64) -> Result<Vec<RecentTask>> {
        let limit = limit.clamp(1, 200);
        let rows = task_state::Entity::find()
            .order_by_desc(task_state::Column::UpdatedAt)
            .limit(limit)
            .all(&self.db)
            .await?;
        rows.into_iter().map(RecentTask::try_from).collect()
    }

    async fn execution_state(&self, task_id: &str) -> Result<Option<task_state::Model>> {
        Ok(task_state::Entity::find_by_id(task_id.to_string())
            .one(&self.db)
            .await?)
    }

    async fn create_schema(&self) -> Result<()> {
        info!(
            table = "task_states",
            "ensuring local state database schema"
        );
        self.db
            .execute_unprepared(
                r"
                CREATE TABLE IF NOT EXISTS task_states (
                    task_id TEXT PRIMARY KEY NOT NULL,
                    multisig TEXT NOT NULL,
                    template_id TEXT NOT NULL,
                    template_version INTEGER NOT NULL,
                    leader TEXT NOT NULL,
                    nonce INTEGER NOT NULL,
                    local_status TEXT NOT NULL,
                    reject_reason TEXT,
                    signature_result TEXT,
                    hl_response_json TEXT,
                    gateway_result_json TEXT,
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL
                )
                ",
            )
            .await?;
        info!(table = "task_states", "local state database schema ready");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::StateStore;
    use crate::gateway::TaskView;

    fn task() -> TaskView {
        TaskView {
            id: "task-1".to_string(),
            multisig_address: "0x0000000000000000000000000000000000000002".to_string(),
            creator: "0x0000000000000000000000000000000000000001".to_string(),
            leader: "0x0000000000000000000000000000000000000001".to_string(),
            nonce: 1,
            network: "mainnet".to_string(),
            template_id: "withdraw3".to_string(),
            template_version: 1,
            inputs: json!({ "amount": "1" }),
            signing_digest: None,
            creator_signature: None,
            action: None,
            threshold: 1,
            status: "pending".to_string(),
            signatures: vec![],
            approvals: 0,
            rejects: 0,
            rejections: vec![],
            created_at: 0,
            expires_at: 999,
            result: None,
        }
    }

    #[tokio::test]
    async fn semantic_record_methods_write_expected_statuses() {
        let store = StateStore::connect("sqlite::memory:").await.unwrap();
        let task = task();

        store.record_allowed(&task).await.unwrap();
        store.record_signed(&task, "submitted").await.unwrap();
        store
            .record_ignored(&task, "gateway reject unavailable")
            .await
            .unwrap();
        assert!(store.is_ignored(&task.id).await.unwrap());
        store.record_failed(&task, "gateway timeout").await.unwrap();

        let recent = store.recent(1).await.unwrap();
        assert_eq!(recent[0].local_status, "failed");
        assert_eq!(recent[0].reject_reason.as_deref(), Some("gateway timeout"));
        assert_eq!(recent[0].signature_result.as_deref(), Some("submitted"));
    }

    #[tokio::test]
    async fn has_signed_tracks_local_signed_status() {
        let store = StateStore::connect("sqlite::memory:").await.unwrap();
        let task = task();

        assert!(!store.has_signed(&task.id).await.unwrap());
        store.record_signed(&task, "submitted").await.unwrap();

        assert!(store.has_signed(&task.id).await.unwrap());
    }

    #[tokio::test]
    async fn execute_unknown_blocks_exchange_resubmission() {
        let store = StateStore::connect("sqlite::memory:").await.unwrap();
        let task = task();

        store
            .record_execute_unknown(&task, "exchange timeout")
            .await
            .unwrap();

        assert!(store.has_submitted_exchange(&task.id).await.unwrap());
        let recent = store.recent(1).await.unwrap();
        assert_eq!(recent[0].local_status, "execute_unknown");
        assert_eq!(recent[0].reject_reason.as_deref(), Some("exchange timeout"));
    }
}
