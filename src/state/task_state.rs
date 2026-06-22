use std::convert::TryInto;

use sea_orm::entity::prelude::*;
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize)]
#[sea_orm(table_name = "task_states")]
pub(crate) struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) task_id: String,
    pub(crate) multisig: String,
    pub(crate) template_id: String,
    pub(crate) template_version: i64,
    pub(crate) leader: String,
    pub(crate) nonce: i64,
    pub(crate) local_status: String,
    pub(crate) reject_reason: Option<String>,
    pub(crate) signature_result: Option<String>,
    pub(crate) hl_response_json: Option<String>,
    pub(crate) gateway_result_json: Option<String>,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub(crate) enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
