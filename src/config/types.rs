use std::collections::BTreeMap;
use std::net::SocketAddr;

use rust_decimal::Decimal;
use serde::Serialize;
use serde_json::{json, Value};

#[derive(Debug, Clone)]
pub(crate) struct Config {
    pub(crate) gateway_url: String,
    pub(crate) hl_api_url: String,
    pub(crate) poll_interval_secs: u64,
    pub(crate) dry_run: bool,
    pub(crate) allowed_templates: Vec<String>,
    pub(crate) allowed_creators: Vec<String>,
    pub(crate) allowed_leaders: Vec<String>,
    pub(crate) template_input_policies: TemplateInputPolicies,
    pub(crate) state_db: String,
    pub(crate) debug_http_addr: SocketAddr,
    pub(crate) signer: SignerConfig,
    pub(crate) leader: String,
    pub(crate) multisig: String,
    pub(crate) withdraw_limit: Decimal,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct SignerConfig {
    pub(crate) keystore_path: String,
    pub(crate) password_env: Option<String>,
}

pub(crate) type TemplateInputPolicies = BTreeMap<String, BTreeMap<String, InputPolicyRule>>;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum InputPolicyRule {
    DecimalMax(Decimal),
    AddressAllowList(Vec<String>),
}

impl InputPolicyRule {
    pub(crate) fn to_json_value(&self) -> Value {
        match self {
            Self::DecimalMax(value) => json!(value.to_string()),
            Self::AddressAllowList(addresses) => json!(addresses),
        }
    }
}
