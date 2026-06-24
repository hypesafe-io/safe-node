use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::{normalize_address, InputPolicyRule, TemplateInputPolicies};
use crate::{NodeError, Result};

#[derive(Debug, Deserialize)]
pub(super) struct Envelope {
    pub(super) code: i32,
    pub(super) message: String,
    pub(super) data: Value,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TodoType {
    Sign,
    Execute,
}

impl TodoType {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Sign => "sign",
            Self::Execute => "execute",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SignInMessage {
    pub(super) signer: String,
    pub(super) timestamp: String,
    pub(super) message: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct TokenResponse {
    pub(super) token: String,
    pub(super) claims: TokenClaims,
}

#[derive(Debug, Deserialize)]
pub(super) struct TokenClaims {
    #[serde(alias = "nextEndTime")]
    pub(super) next_end_time: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AccountView {
    pub(crate) signers: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SubAccountView {
    pub(crate) sub_account_address: String,
    pub(crate) name: String,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
    #[serde(default)]
    pub(crate) synced_at: Option<i64>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct SubAccountRegistry {
    addresses: BTreeSet<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct I18nText {
    pub(crate) en: String,
    pub(crate) zh: String,
}

#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum TemplateFieldType {
    Address,
    #[serde(rename = "addressList")]
    AddressList,
    Amount,
    String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TemplateField {
    pub(crate) name: String,
    #[serde(rename = "type")]
    pub(crate) field_type: TemplateFieldType,
    pub(crate) required: bool,
    pub(crate) label: I18nText,
    pub(crate) description: I18nText,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TemplateView {
    pub(crate) id: String,
    pub(crate) version: i64,
    pub(crate) type_name: String,
    pub(crate) hl_action_type: Option<String>,
    pub(crate) display_name: I18nText,
    pub(crate) description: I18nText,
    pub(crate) summary: I18nText,
    pub(crate) fields: Vec<TemplateField>,
    #[serde(default)]
    pub(crate) signing: Option<Value>,
    #[serde(default)]
    pub(crate) exchange: Option<Value>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct TemplateRegistry {
    templates: Vec<TemplateView>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TaskSignatureView {
    pub(crate) signer: String,
    pub(crate) signed_at: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TaskRejectionView {
    pub(crate) signer: String,
    pub(crate) reason: String,
    pub(crate) rejected_at: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TaskView {
    pub(crate) id: String,
    pub(crate) multisig_address: String,
    pub(crate) creator: String,
    pub(crate) leader: String,
    pub(crate) nonce: i64,
    pub(crate) network: String,
    pub(crate) template_id: String,
    pub(crate) template_version: i64,
    pub(crate) inputs: Value,
    #[serde(default)]
    pub(crate) signing_digest: Option<String>,
    #[serde(default)]
    pub(crate) creator_signature: Option<String>,
    #[serde(default)]
    pub(crate) action: Option<Value>,
    pub(crate) threshold: i64,
    pub(crate) status: String,
    #[serde(default)]
    pub(crate) signatures: Vec<TaskSignatureView>,
    pub(crate) approvals: i64,
    #[serde(default)]
    pub(crate) rejects: i64,
    #[serde(default)]
    pub(crate) rejections: Vec<TaskRejectionView>,
    pub(crate) created_at: i64,
    pub(crate) expires_at: i64,
    #[serde(default)]
    pub(crate) result: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SigningPayloadResponse {
    pub(super) signing_payload: Value,
    #[serde(default)]
    pub(super) signing_digest: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateTaskPayloadRequest {
    pub(crate) template_id: String,
    pub(crate) template_version: i64,
    pub(crate) inputs: Value,
    pub(crate) leader: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) network: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) expires_in_secs: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateTaskPayloadResponse {
    pub(crate) challenge_id: String,
    pub(crate) signing_payload: Value,
}

#[derive(Debug)]
pub(crate) struct SigningPayload {
    pub(crate) typed_data: Value,
    pub(crate) signing_digest: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OuterSigningPayload {
    pub(crate) typed_data: Value,
    #[serde(default)]
    pub(crate) outer_signing_digest: Option<String>,
    pub(crate) multi_sig_action: Value,
    #[serde(default)]
    pub(crate) vault_address: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TaskResultRequest {
    pub(crate) success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tx_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) response: Option<Value>,
}

impl AccountView {
    pub(crate) fn has_signer(&self, signer_lc: &str) -> bool {
        self.signers
            .iter()
            .filter_map(|signer| normalize_address(signer).ok())
            .any(|signer| signer == signer_lc)
    }
}

impl SubAccountRegistry {
    pub(crate) fn new(sub_accounts: Vec<SubAccountView>) -> Result<Self> {
        let mut addresses = BTreeSet::new();
        for sub_account in sub_accounts {
            let address = normalize_address(&sub_account.sub_account_address).map_err(|err| {
                NodeError::Gateway(format!("invalid sub-account address from gateway: {err}"))
            })?;
            addresses.insert(address);
        }
        Ok(Self { addresses })
    }

    #[cfg(test)]
    pub(crate) fn from_addresses(addresses: &[&str]) -> Self {
        let addresses = addresses
            .iter()
            .map(|address| normalize_address(address).expect("valid sub-account address"))
            .collect();
        Self { addresses }
    }

    pub(crate) fn len(&self) -> usize {
        self.addresses.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.addresses.is_empty()
    }

    pub(crate) fn contains_normalized(&self, address: &str) -> bool {
        self.addresses.contains(address)
    }
}

impl TemplateRegistry {
    pub(crate) fn new(templates: Vec<TemplateView>) -> Self {
        Self { templates }
    }

    pub(crate) fn validate_allowed_templates(&self, allowed_templates: &[String]) -> Result<()> {
        for template in allowed_templates {
            if self.by_id(template).is_none() {
                return Err(NodeError::Config(format!(
                    "allowed template `{template}` is not exposed by the gateway"
                )));
            }
        }
        Ok(())
    }

    pub(crate) fn validate_template_input_policies(
        &self,
        policies: &TemplateInputPolicies,
    ) -> Result<()> {
        for (template_id, rules) in policies {
            let template = self.by_id(template_id).ok_or_else(|| {
                NodeError::Config(format!(
                    "template input policy references unknown template `{template_id}`"
                ))
            })?;
            for (path, rule) in rules {
                template.validate_input_policy(path, rule)?;
            }
        }
        Ok(())
    }

    pub(crate) fn by_task(&self, task: &TaskView) -> Option<&TemplateView> {
        self.templates
            .iter()
            .find(|template| {
                template.id == task.template_id && template.version == task.template_version
            })
            .or_else(|| self.by_id(&task.template_id))
    }

    fn by_id(&self, id: &str) -> Option<&TemplateView> {
        self.templates.iter().find(|template| template.id == id)
    }
}

impl TemplateView {
    pub(crate) fn has_amount_field(&self) -> bool {
        self.fields
            .iter()
            .any(|field| field.field_type == TemplateFieldType::Amount)
    }

    pub(crate) fn requires_hyperliquid_submit(&self) -> bool {
        self.hl_action_type.is_some()
    }

    fn validate_input_policy(&self, path: &str, rule: &InputPolicyRule) -> Result<()> {
        let Some(field_name) = path.strip_prefix("inputs.") else {
            return Err(NodeError::Config(format!(
                "template input policy path `{path}` must start with `inputs.`"
            )));
        };
        let field = self
            .fields
            .iter()
            .find(|field| field.name == field_name)
            .ok_or_else(|| {
                NodeError::Config(format!(
                    "template input policy references unknown field `{}` on template `{}`",
                    field_name, self.id
                ))
            })?;
        match (rule, field.field_type) {
            (InputPolicyRule::DecimalMax(_), TemplateFieldType::Amount)
            | (InputPolicyRule::AddressAllowList(_), TemplateFieldType::Address) => Ok(()),
            (InputPolicyRule::DecimalMax(_), _) => Err(NodeError::Config(format!(
                "template input policy `{}` on template `{}` must reference an amount field",
                path, self.id
            ))),
            (InputPolicyRule::AddressAllowList(_), _) => Err(NodeError::Config(format!(
                "template input policy `{}` on template `{}` must reference an address field",
                path, self.id
            ))),
        }
    }

    pub(crate) fn signing_intent_spec(
        &self,
    ) -> std::result::Result<Option<hypesafe_signing_intent::TemplateSpec>, String> {
        match (&self.signing, &self.exchange) {
            (Some(signing), Some(exchange)) => {
                Ok(Some(hypesafe_signing_intent::TemplateSpec::new(
                    self.id.clone(),
                    self.version,
                    signing.clone(),
                    exchange.clone(),
                )))
            }
            (None, None) => Ok(None),
            _ => Err(format!(
                "template `{}` must include both signing and exchange specs",
                self.id
            )),
        }
    }
}

impl TaskView {
    pub(crate) fn has_rejection_from(&self, signer_lc: &str, reason: &str) -> bool {
        let reason = reason.trim();
        self.rejections.iter().any(|rejection| {
            normalize_address(&rejection.signer)
                .map(|signer| signer == signer_lc)
                .unwrap_or(false)
                && rejection.reason == reason
        })
    }
}
