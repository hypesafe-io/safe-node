use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;

use super::types::{Config, TemplateInputPolicies};

#[derive(Debug, Clone, Serialize)]
pub(crate) struct RedactedConfig {
    pub(crate) gateway_url: String,
    pub(crate) hl_api_url: String,
    pub(crate) poll_interval_secs: u64,
    pub(crate) dry_run: bool,
    pub(crate) allowed_templates: Vec<String>,
    pub(crate) allowed_creators: Vec<String>,
    pub(crate) allowed_leaders: Vec<String>,
    pub(crate) template_input_policies: BTreeMap<String, BTreeMap<String, Value>>,
    pub(crate) state_db: String,
    pub(crate) debug_http_addr: String,
    pub(crate) signer: RedactedSignerConfig,
    pub(crate) leader: String,
    pub(crate) multisig: String,
    pub(crate) withdraw_limit: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct RedactedSignerConfig {
    pub(crate) keystore_path: String,
    pub(crate) password_env: Option<String>,
}

impl Config {
    pub(crate) fn redacted(&self) -> RedactedConfig {
        RedactedConfig {
            gateway_url: self.gateway_url.clone(),
            hl_api_url: self.hl_api_url.clone(),
            poll_interval_secs: self.poll_interval_secs,
            dry_run: self.dry_run,
            allowed_templates: self.allowed_templates.clone(),
            allowed_creators: self.allowed_creators.clone(),
            allowed_leaders: self.allowed_leaders.clone(),
            template_input_policies: template_input_policies_json(&self.template_input_policies),
            state_db: self.state_db.clone(),
            debug_http_addr: self.debug_http_addr.to_string(),
            signer: RedactedSignerConfig {
                keystore_path: self.signer.keystore_path.clone(),
                password_env: self.signer.password_env.clone(),
            },
            leader: self.leader.clone(),
            multisig: self.multisig.clone(),
            withdraw_limit: self.withdraw_limit.to_string(),
        }
    }
}

fn template_input_policies_json(
    policies: &TemplateInputPolicies,
) -> BTreeMap<String, BTreeMap<String, Value>> {
    policies
        .iter()
        .map(|(template, rules)| {
            let rules = rules
                .iter()
                .map(|(path, rule)| (path.clone(), rule.to_json_value()))
                .collect();
            (template.clone(), rules)
        })
        .collect()
}
