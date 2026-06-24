use std::collections::BTreeMap;
use std::{net::SocketAddr, str::FromStr};

use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::Value;

use super::address::{normalize_address, trim_url};
use super::types::{Config, InputPolicyRule, SignerConfig, TemplateInputPolicies};
use crate::{NodeError, Result};

const DEFAULT_ALLOWED_TEMPLATES: &[&str] = &["withdraw3", "sub_account_withdraw3"];

#[derive(Debug, Deserialize)]
pub(super) struct RawConfig {
    gateway_url: String,
    hl_api_url: String,
    #[serde(default = "default_poll_interval_secs")]
    poll_interval_secs: u64,
    #[serde(default)]
    dry_run: bool,
    #[serde(default = "default_allowed_templates")]
    allowed_templates: Vec<String>,
    #[serde(default)]
    allowed_creators: Vec<String>,
    #[serde(default)]
    allowed_leaders: Vec<String>,
    #[serde(default)]
    template_input_policies: BTreeMap<String, BTreeMap<String, Value>>,
    state_db: String,
    #[serde(default = "default_rpc_http_addr")]
    rpc_http_addr: String,
    #[serde(default)]
    rpc_auth_token: Option<String>,
    signer: RawSignerConfig,
    leader: String,
    multisig: String,
    withdraw_limit: String,
}

#[derive(Debug, Deserialize)]
struct RawSignerConfig {
    keystore_path: String,
    #[serde(default)]
    password_env: Option<String>,
}

fn default_poll_interval_secs() -> u64 {
    15
}

fn default_rpc_http_addr() -> String {
    "127.0.0.1:9909".to_string()
}

fn default_allowed_templates() -> Vec<String> {
    DEFAULT_ALLOWED_TEMPLATES
        .iter()
        .map(|template| (*template).to_string())
        .collect()
}

impl RawConfig {
    pub(super) fn validate(self) -> Result<Config> {
        if self.gateway_url.trim().is_empty() {
            return Err(NodeError::Config("gateway_url is required".to_string()));
        }
        if self.hl_api_url.trim().is_empty() {
            return Err(NodeError::Config("hl_api_url is required".to_string()));
        }
        if self.state_db.trim().is_empty() {
            return Err(NodeError::Config("state_db is required".to_string()));
        }
        let allowed_templates = normalize_allowed_templates(self.allowed_templates)?;
        if self.signer.keystore_path.trim().is_empty() {
            return Err(NodeError::Config(
                "signer.keystore_path is required".to_string(),
            ));
        }
        let leader = normalize_address(&self.leader)?;
        let multisig = normalize_address(&self.multisig)?;
        let allowed_creators =
            normalize_allowed_addresses(self.allowed_creators, &leader, "allowed_creators")
                .map_err(NodeError::Config)?;
        let allowed_leaders =
            normalize_allowed_addresses(self.allowed_leaders, &leader, "allowed_leaders")
                .map_err(NodeError::Config)?;
        let template_input_policies =
            normalize_template_input_policies(self.template_input_policies)?;
        let withdraw_limit = Decimal::from_str(self.withdraw_limit.trim()).map_err(|err| {
            NodeError::Config(format!("withdraw_limit must be a decimal string: {err}"))
        })?;
        if withdraw_limit.is_sign_negative() {
            return Err(NodeError::Config(
                "withdraw_limit must be non-negative".to_string(),
            ));
        }
        let rpc_http_addr = self
            .rpc_http_addr
            .parse::<SocketAddr>()
            .map_err(|err| NodeError::Config(format!("rpc_http_addr must be host:port: {err}")))?;
        let rpc_auth_token = normalize_optional_secret(self.rpc_auth_token);

        Ok(Config {
            gateway_url: trim_url(&self.gateway_url),
            hl_api_url: trim_url(&self.hl_api_url),
            poll_interval_secs: self.poll_interval_secs,
            dry_run: self.dry_run,
            allowed_templates,
            allowed_creators,
            allowed_leaders,
            template_input_policies,
            state_db: self.state_db,
            rpc_http_addr,
            rpc_auth_token,
            signer: SignerConfig {
                keystore_path: self.signer.keystore_path,
                password_env: self
                    .signer
                    .password_env
                    .filter(|value| !value.trim().is_empty()),
            },
            leader,
            multisig,
            withdraw_limit,
        })
    }
}

fn normalize_optional_secret(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn normalize_allowed_addresses(
    addresses: Vec<String>,
    default_address: &str,
    field: &str,
) -> std::result::Result<Vec<String>, String> {
    if addresses.is_empty() {
        return Ok(vec![default_address.to_string()]);
    }
    let mut deduped = Vec::with_capacity(addresses.len());
    for address in addresses {
        let address = normalize_address(&address).map_err(|err| err.to_string())?;
        if !deduped.iter().any(|existing| existing == &address) {
            deduped.push(address);
        }
    }
    if deduped.is_empty() {
        return Err(format!("{field} must contain at least one address"));
    }
    Ok(deduped)
}

fn normalize_template_input_policies(
    raw: BTreeMap<String, BTreeMap<String, Value>>,
) -> Result<TemplateInputPolicies> {
    let mut policies = TemplateInputPolicies::new();
    for (template_id, rules) in raw {
        let template_id = template_id.trim();
        if template_id.is_empty() {
            return Err(NodeError::Config(
                "template_input_policies must not contain empty template ids".to_string(),
            ));
        }
        if rules.is_empty() {
            return Err(NodeError::Config(format!(
                "template_input_policies.{template_id} must contain at least one rule"
            )));
        }

        let mut normalized_rules = BTreeMap::new();
        for (path, value) in rules {
            let path = normalize_input_path(&path)?;
            if normalized_rules.contains_key(&path) {
                return Err(NodeError::Config(format!(
                    "duplicate template input policy path `{path}` for template `{template_id}`"
                )));
            }
            normalized_rules.insert(path, normalize_input_policy_value(&value)?);
        }

        if policies
            .insert(template_id.to_string(), normalized_rules)
            .is_some()
        {
            return Err(NodeError::Config(format!(
                "duplicate template input policy for template `{template_id}`"
            )));
        }
    }
    Ok(policies)
}

fn normalize_input_path(path: &str) -> Result<String> {
    let path = path.trim();
    let Some(field) = path.strip_prefix("inputs.") else {
        return Err(NodeError::Config(format!(
            "template input policy path `{path}` must start with `inputs.`"
        )));
    };
    if field.is_empty() || field.contains('.') {
        return Err(NodeError::Config(format!(
            "template input policy path `{path}` must use v1 form `inputs.<field>`"
        )));
    }
    Ok(format!("inputs.{field}"))
}

fn normalize_input_policy_value(value: &Value) -> Result<InputPolicyRule> {
    match value {
        Value::String(raw) => {
            let value = Decimal::from_str(raw.trim()).map_err(|err| {
                NodeError::Config(format!(
                    "template input amount policy must be a decimal string: {err}"
                ))
            })?;
            if value.is_sign_negative() {
                return Err(NodeError::Config(
                    "template input amount policy must be non-negative".to_string(),
                ));
            }
            Ok(InputPolicyRule::DecimalMax(value))
        }
        Value::Array(items) => {
            let mut addresses = Vec::with_capacity(items.len());
            for item in items {
                let Some(raw) = item.as_str() else {
                    return Err(NodeError::Config(
                        "template input address policy entries must be strings".to_string(),
                    ));
                };
                let address = normalize_address(raw)?;
                if !addresses.iter().any(|existing| existing == &address) {
                    addresses.push(address);
                }
            }
            if addresses.is_empty() {
                return Err(NodeError::Config(
                    "template input address policy must contain at least one address".to_string(),
                ));
            }
            Ok(InputPolicyRule::AddressAllowList(addresses))
        }
        _ => Err(NodeError::Config(
            "template input policy values must be decimal strings or address arrays".to_string(),
        )),
    }
}

fn normalize_allowed_templates(templates: Vec<String>) -> Result<Vec<String>> {
    if templates.is_empty() {
        return Err(NodeError::Config(
            "allowed_templates must contain at least one template id".to_string(),
        ));
    }
    let mut deduped = Vec::with_capacity(templates.len());
    for template in templates {
        let template = template.trim();
        if template.is_empty() {
            return Err(NodeError::Config(
                "allowed_templates must not contain empty template ids".to_string(),
            ));
        }
        if !deduped.iter().any(|existing| existing == template) {
            deduped.push(template.to_string());
        }
    }
    Ok(deduped)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::RawConfig;

    fn base_config() -> serde_json::Value {
        json!({
            "gateway_url": "http://gateway",
            "hl_api_url": "http://hl",
            "state_db": "sqlite::memory:",
            "signer": {
                "keystore_path": "config/signer.json"
            },
            "leader": "0x0000000000000000000000000000000000000001",
            "multisig": "0x0000000000000000000000000000000000000002",
            "withdraw_limit": "1000"
        })
    }

    #[test]
    fn defaults_allowed_templates_when_missing() {
        let raw: RawConfig = serde_json::from_value(base_config()).unwrap();
        let config = raw.validate().unwrap();

        assert_eq!(
            config.allowed_templates,
            ["withdraw3".to_string(), "sub_account_withdraw3".to_string()]
        );
        assert_eq!(
            config.allowed_leaders,
            ["0x0000000000000000000000000000000000000001".to_string()]
        );
        assert!(config.rpc_http_addr.ip().is_loopback());
        assert!(config.rpc_auth_token.is_none());
    }

    #[test]
    fn normalizes_rpc_auth_token() {
        let mut value = base_config();
        value["rpc_auth_token"] = json!("  token-1  ");
        let raw: RawConfig = serde_json::from_value(value).unwrap();
        let config = raw.validate().unwrap();

        assert_eq!(config.rpc_auth_token.as_deref(), Some("token-1"));
    }

    #[test]
    fn normalizes_allowed_leaders() {
        let mut value = base_config();
        value["allowed_leaders"] = json!([
            "0x000000000000000000000000000000000000dEaD",
            "0x000000000000000000000000000000000000dead"
        ]);
        let raw: RawConfig = serde_json::from_value(value).unwrap();
        let config = raw.validate().unwrap();

        assert_eq!(
            config.allowed_leaders,
            ["0x000000000000000000000000000000000000dead".to_string()]
        );
    }

    #[test]
    fn parses_template_input_policies() {
        let mut value = base_config();
        value["template_input_policies"] = json!({
            "withdraw3": {
                "inputs.destination": [
                    "0x000000000000000000000000000000000000dEaD",
                    "0x000000000000000000000000000000000000dead"
                ],
                "inputs.amount": "10"
            }
        });
        let raw: RawConfig = serde_json::from_value(value).unwrap();
        let config = raw.validate().unwrap();
        let rules = config.template_input_policies.get("withdraw3").unwrap();

        assert_eq!(rules.len(), 2);
    }

    #[test]
    fn rejects_bad_template_input_policy_path() {
        let mut value = base_config();
        value["template_input_policies"] = json!({
            "withdraw3": {
                "inputs.destination.extra": [
                    "0x000000000000000000000000000000000000dead"
                ]
            }
        });
        let raw: RawConfig = serde_json::from_value(value).unwrap();

        let err = raw.validate().unwrap_err();

        assert!(err.to_string().contains("inputs.<field>"));
    }

    #[test]
    fn rejects_bad_template_input_policy_amount() {
        let mut value = base_config();
        value["template_input_policies"] = json!({
            "withdraw3": {
                "inputs.amount": "-1"
            }
        });
        let raw: RawConfig = serde_json::from_value(value).unwrap();

        let err = raw.validate().unwrap_err();

        assert!(err.to_string().contains("non-negative"));
    }

    #[test]
    fn deduplicates_allowed_templates() {
        let mut value = base_config();
        value["allowed_templates"] = json!(["withdraw3", " withdraw3 ", "send_asset"]);
        let raw: RawConfig = serde_json::from_value(value).unwrap();
        let config = raw.validate().unwrap();

        assert_eq!(
            config.allowed_templates,
            ["withdraw3".to_string(), "send_asset".to_string()]
        );
    }

    #[test]
    fn rejects_empty_allowed_templates() {
        let mut value = base_config();
        value["allowed_templates"] = json!([]);
        let raw: RawConfig = serde_json::from_value(value).unwrap();

        let err = raw.validate().unwrap_err();

        assert!(err.to_string().contains("allowed_templates"));
    }

    #[test]
    fn rejects_empty_allowed_template_item() {
        let mut value = base_config();
        value["allowed_templates"] = json!(["withdraw3", " "]);
        let raw: RawConfig = serde_json::from_value(value).unwrap();

        let err = raw.validate().unwrap_err();

        assert!(err.to_string().contains("empty template"));
    }
}
