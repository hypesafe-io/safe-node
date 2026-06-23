use std::collections::BTreeSet;
use std::convert::TryFrom;

use serde_json::Value;

use crate::config::normalize_address;
use crate::gateway::{OuterSigningPayload, TaskView, TemplateView};
use crate::signing::typed_data_digest_hex;

type IntentResult<T> = std::result::Result<T, String>;

pub(super) struct VerifiedOuterSubmission {
    pub(super) typed_data: Value,
    pub(super) multi_sig_action: Value,
    pub(super) vault_address: Option<String>,
}

pub(super) fn validate_task_signing_payload_digest(
    typed_data: &Value,
    task_digest: Option<&str>,
    response_digest: Option<&str>,
) -> IntentResult<()> {
    let task_digest = required_digest("TaskView.signingDigest", task_digest)?;
    let response_digest = required_digest("signingPayload.signingDigest", response_digest)?;
    if !digest_eq(task_digest, response_digest) {
        return Err(format!(
            "signing digest mismatch between task and signing payload: task={}, response={}",
            task_digest, response_digest
        ));
    }
    validate_typed_data_digest("signing payload", typed_data, task_digest)
}

pub(super) fn build_and_validate_task_intent(
    task: &TaskView,
    template: Option<&TemplateView>,
) -> IntentResult<Option<Value>> {
    let Some(template) = template else {
        return Ok(None);
    };
    let Some(template_spec) = template
        .signing_intent_spec()
        .map_err(|err| err.to_string())?
    else {
        return Ok(None);
    };
    let creator_signature = task
        .creator_signature
        .as_deref()
        .map(str::trim)
        .filter(|signature| !signature.is_empty())
        .ok_or_else(|| {
            "TaskView.creatorSignature is required for local intent validation".to_string()
        })?;
    let nonce = u64::try_from(task.nonce)
        .map_err(|_| format!("task nonce must be non-negative: {}", task.nonce))?;
    let network =
        hypesafe_signing_intent::Network::parse(&task.network).map_err(|err| err.to_string())?;
    let ctx = hypesafe_signing_intent::TaskContext {
        multisig_address: &task.multisig_address,
        leader: &task.leader,
        nonce,
        network,
        template: &template_spec,
        params: &task.inputs,
    };
    let typed_data =
        hypesafe_signing_intent::build_signing_payload(&ctx).map_err(|err| err.to_string())?;
    let recovered = hypesafe_signing_intent::recover_signer(&typed_data, creator_signature)
        .map_err(|err| err.to_string())?;
    let recovered = format!("0x{recovered:x}");
    if !recovered.eq_ignore_ascii_case(task.creator.trim()) {
        return Err(format!(
            "creator signature mismatch: task.creator={}, recovered={}",
            task.creator, recovered
        ));
    }
    if let Some(task_digest) = task.signing_digest.as_deref() {
        validate_typed_data_digest("local signing intent", &typed_data, task_digest)?;
    }
    Ok(Some(typed_data))
}

pub(super) fn validate_outer_signing_payload_digest(
    typed_data: &Value,
    outer_digest: Option<&str>,
) -> IntentResult<()> {
    let outer_digest = required_digest("outerSigningPayload.outerSigningDigest", outer_digest)?;
    validate_typed_data_digest("outer signing payload", typed_data, outer_digest)
}

pub(super) fn build_and_validate_outer_submission(
    task: &TaskView,
    template: Option<&TemplateView>,
    outer: &OuterSigningPayload,
    authorized_users: &[String],
    threshold: i64,
) -> IntentResult<VerifiedOuterSubmission> {
    let Some(template) = template else {
        return Err("template metadata is required before leader execution".to_string());
    };
    let template_spec = template
        .signing_intent_spec()
        .map_err(|err| err.to_string())?
        .ok_or_else(|| {
            format!(
                "template `{}` must include signing and exchange specs before leader execution",
                task.template_id
            )
        })?;
    let creator_signature = task
        .creator_signature
        .as_deref()
        .map(str::trim)
        .filter(|signature| !signature.is_empty())
        .ok_or_else(|| {
            "TaskView.creatorSignature is required for local intent validation".to_string()
        })?;
    let nonce = u64::try_from(task.nonce)
        .map_err(|_| format!("task nonce must be non-negative: {}", task.nonce))?;
    let network =
        hypesafe_signing_intent::Network::parse(&task.network).map_err(|err| err.to_string())?;
    let ctx = hypesafe_signing_intent::TaskContext {
        multisig_address: &task.multisig_address,
        leader: &task.leader,
        nonce,
        network,
        template: &template_spec,
        params: &task.inputs,
    };

    let inner_typed_data =
        hypesafe_signing_intent::build_signing_payload(&ctx).map_err(|err| err.to_string())?;
    validate_creator_signature(task, &inner_typed_data, creator_signature)?;
    if let Some(task_digest) = task.signing_digest.as_deref() {
        validate_typed_data_digest("local signing intent", &inner_typed_data, task_digest)?;
    }

    let signatures = raw_signatures_from_multi_sig_action(&outer.multi_sig_action)?;
    validate_inner_signature_threshold(
        &inner_typed_data,
        &signatures,
        authorized_users,
        threshold,
    )?;

    let multi_sig_action = hypesafe_signing_intent::build_multi_sig_action(&ctx, &signatures)
        .map_err(|err| err.to_string())?;
    if multi_sig_action != outer.multi_sig_action {
        return Err("gateway multiSigAction does not match local reconstruction".to_string());
    }

    let typed_data = hypesafe_signing_intent::build_outer_signing_payload(&ctx, &multi_sig_action)
        .map_err(|err| err.to_string())?;
    if typed_data != outer.typed_data {
        return Err("gateway outer typedData does not match local reconstruction".to_string());
    }
    validate_outer_signing_payload_digest(&typed_data, outer.outer_signing_digest.as_deref())?;

    let vault_address =
        hypesafe_signing_intent::exchange_vault_address(&ctx).map_err(|err| err.to_string())?;
    if !vault_address_eq(vault_address.as_deref(), outer.vault_address.as_deref())? {
        return Err("gateway vaultAddress does not match local reconstruction".to_string());
    }

    Ok(VerifiedOuterSubmission {
        typed_data,
        multi_sig_action,
        vault_address,
    })
}

fn validate_creator_signature(
    task: &TaskView,
    typed_data: &Value,
    signature: &str,
) -> IntentResult<()> {
    let recovered = hypesafe_signing_intent::recover_signer(typed_data, signature)
        .map_err(|err| err.to_string())?;
    let recovered = format!("0x{recovered:x}");
    if !recovered.eq_ignore_ascii_case(task.creator.trim()) {
        return Err(format!(
            "creator signature mismatch: task.creator={}, recovered={}",
            task.creator, recovered
        ));
    }
    Ok(())
}

fn validate_inner_signature_threshold(
    typed_data: &Value,
    signatures: &[String],
    authorized_users: &[String],
    threshold: i64,
) -> IntentResult<()> {
    let authorized = normalize_authorized_users(authorized_users, threshold)?;
    let threshold = usize::try_from(threshold)
        .map_err(|_| "multisig threshold must be positive".to_string())?;
    let mut recovered_signers = BTreeSet::new();
    for signature in signatures {
        let recovered = hypesafe_signing_intent::recover_signer(typed_data, signature)
            .map_err(|err| err.to_string())?;
        let recovered =
            normalize_address(&format!("0x{recovered:x}")).map_err(|err| err.to_string())?;
        if !authorized.contains(&recovered) {
            return Err(format!(
                "inner signature recovered unauthorized signer {recovered}"
            ));
        }
        if !recovered_signers.insert(recovered.clone()) {
            return Err(format!("duplicate inner signature signer {recovered}"));
        }
    }
    if recovered_signers.len() < threshold {
        return Err(format!(
            "inner signatures below threshold: recovered={}, threshold={threshold}",
            recovered_signers.len()
        ));
    }
    Ok(())
}

fn normalize_authorized_users(
    authorized_users: &[String],
    threshold: i64,
) -> IntentResult<BTreeSet<String>> {
    let mut authorized = BTreeSet::new();
    for signer in authorized_users {
        authorized.insert(normalize_address(signer).map_err(|err| err.to_string())?);
    }
    let threshold = usize::try_from(threshold)
        .map_err(|_| "multisig threshold must be positive".to_string())?;
    if threshold == 0 || threshold > authorized.len() {
        return Err(format!(
            "multisig threshold must be between 1 and authorized signer count ({})",
            authorized.len()
        ));
    }
    Ok(authorized)
}

fn raw_signatures_from_multi_sig_action(action: &Value) -> IntentResult<Vec<String>> {
    let signatures = action
        .get("signatures")
        .and_then(Value::as_array)
        .ok_or_else(|| "multiSigAction.signatures must be an array".to_string())?;
    signatures.iter().map(hl_signature_to_raw).collect()
}

fn hl_signature_to_raw(value: &Value) -> IntentResult<String> {
    let object = value
        .as_object()
        .ok_or_else(|| "multiSigAction.signatures entries must be objects".to_string())?;
    let r = signature_component(object.get("r"), "r")?;
    let s = signature_component(object.get("s"), "s")?;
    let v = signature_recovery_id(object.get("v"))?;
    Ok(format!("0x{r}{s}{v:02x}"))
}

fn signature_component(value: Option<&Value>, field: &str) -> IntentResult<String> {
    let raw = value
        .and_then(Value::as_str)
        .ok_or_else(|| format!("signature field `{field}` must be a string"))?
        .trim();
    let raw = raw
        .strip_prefix("0x")
        .or_else(|| raw.strip_prefix("0X"))
        .unwrap_or(raw);
    if raw.is_empty() || raw.len() > 64 || !raw.as_bytes().iter().all(u8::is_ascii_hexdigit) {
        return Err(format!("signature field `{field}` is malformed"));
    }
    Ok(format!("{raw:0>64}").to_ascii_lowercase())
}

fn signature_recovery_id(value: Option<&Value>) -> IntentResult<u8> {
    let recovery = match value {
        Some(Value::Number(number)) => number
            .as_u64()
            .ok_or_else(|| "signature field `v` must be an integer".to_string())?,
        Some(Value::String(raw)) => {
            let raw = raw.trim();
            raw.strip_prefix("0x")
                .or_else(|| raw.strip_prefix("0X"))
                .map_or_else(|| raw.parse::<u64>(), |hex| u64::from_str_radix(hex, 16))
                .map_err(|_| "signature field `v` must be an integer".to_string())?
        }
        _ => return Err("signature field `v` must be an integer".to_string()),
    };
    match recovery {
        0 | 1 | 27 | 28 => {
            u8::try_from(recovery).map_err(|_| "signature field `v` is out of range".to_string())
        }
        _ => Err("signature field `v` must be 0, 1, 27, or 28".to_string()),
    }
}

fn vault_address_eq(left: Option<&str>, right: Option<&str>) -> IntentResult<bool> {
    match (left, right) {
        (None, None) => Ok(true),
        (Some(left), Some(right)) => {
            let left = normalize_address(left).map_err(|err| err.to_string())?;
            let right = normalize_address(right).map_err(|err| err.to_string())?;
            Ok(left == right)
        }
        _ => Ok(false),
    }
}

fn validate_typed_data_digest(
    context: &str,
    typed_data: &Value,
    expected: &str,
) -> IntentResult<()> {
    let actual = typed_data_digest_hex(typed_data)?;
    if !digest_eq(&actual, expected) {
        return Err(format!(
            "{context} digest mismatch: expected {expected}, actual {actual}"
        ));
    }
    Ok(())
}

fn required_digest<'a>(field: &str, value: Option<&'a str>) -> IntentResult<&'a str> {
    let value = value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{field} is required before signing"))?;
    if !looks_like_digest(value) {
        return Err(format!("{field} must be a 0x-prefixed 32-byte hex digest"));
    }
    Ok(value)
}

fn looks_like_digest(value: &str) -> bool {
    value.len() == 66
        && value.starts_with("0x")
        && value.as_bytes()[2..].iter().all(u8::is_ascii_hexdigit)
}

fn digest_eq(left: &str, right: &str) -> bool {
    left.trim().eq_ignore_ascii_case(right.trim())
}

#[cfg(test)]
mod tests {
    use hypesafe_signing_intent::{build_signing_payload, Network, TaskContext, TemplateSpec};
    use serde_json::json;

    use super::{build_and_validate_task_intent, validate_task_signing_payload_digest};
    use crate::gateway::{I18nText, TaskView, TemplateField, TemplateFieldType, TemplateView};
    use crate::signing::{typed_data_digest_hex, NodeSigner};

    fn payload() -> serde_json::Value {
        json!({
            "domain": {
                "name": "HypeSafe",
                "version": "1",
                "chainId": 42161,
                "verifyingContract": "0x0000000000000000000000000000000000000001"
            },
            "primaryType": "Ping",
            "types": {
                "Ping": [
                    { "name": "sender", "type": "address" },
                    { "name": "nonce", "type": "uint64" }
                ]
            },
            "message": {
                "sender": "0x0000000000000000000000000000000000000002",
                "nonce": 1
            }
        })
    }

    fn text(value: &str) -> I18nText {
        I18nText {
            en: value.to_string(),
            zh: value.to_string(),
        }
    }

    fn template() -> TemplateView {
        TemplateView {
            id: "withdraw3".to_string(),
            version: 1,
            type_name: "withdraw".to_string(),
            hl_action_type: Some("withdraw3".to_string()),
            display_name: text("Withdraw"),
            description: text("Withdraw"),
            summary: text("Withdraw"),
            fields: vec![TemplateField {
                name: "amount".to_string(),
                field_type: TemplateFieldType::Amount,
                required: true,
                label: text("Amount"),
                description: text("Amount"),
            }],
            signing: Some(json!({
                "domain": {
                    "name": "HypeSafe",
                    "version": "1",
                    "chainId": 42161,
                    "verifyingContract": "ctx:multiSigAddress"
                },
                "primaryType": "Ping",
                "types": [
                    { "name": "leader", "type": "address" },
                    { "name": "amount", "type": "string" }
                ],
                "message": [
                    { "name": "leader", "from": "ctx:leader" },
                    { "name": "amount", "from": "param:amount" }
                ]
            })),
            exchange: Some(json!({
                "action": [
                    { "key": "type", "from": "const:withdraw3" },
                    { "key": "amount", "from": "param:amount" }
                ]
            })),
        }
    }

    fn task(creator: &str, signature: String, digest: String) -> TaskView {
        TaskView {
            id: "task-1".to_string(),
            multisig_address: "0x0000000000000000000000000000000000000002".to_string(),
            creator: creator.to_string(),
            leader: "0x0000000000000000000000000000000000000001".to_string(),
            nonce: 1,
            network: "mainnet".to_string(),
            template_id: "withdraw3".to_string(),
            template_version: 1,
            inputs: json!({ "amount": "1" }),
            signing_digest: Some(digest),
            creator_signature: Some(signature),
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

    fn sign_task_intent(creator: &NodeSigner, template: &TemplateView) -> (String, String) {
        let spec = TemplateSpec::new(
            template.id.clone(),
            template.version,
            template.signing.clone().unwrap(),
            template.exchange.clone().unwrap(),
        );
        let inputs = json!({ "amount": "1" });
        let ctx = TaskContext {
            multisig_address: "0x0000000000000000000000000000000000000002",
            leader: "0x0000000000000000000000000000000000000001",
            nonce: 1,
            network: Network::Mainnet,
            template: &spec,
            params: &inputs,
        };
        let typed_data = build_signing_payload(&ctx).unwrap();
        let digest = typed_data_digest_hex(&typed_data).unwrap();
        let signature = creator.sign_and_verify_typed_data(&typed_data).unwrap();
        (signature, digest)
    }

    #[test]
    fn accepts_matching_task_and_payload_digest() {
        let payload = payload();
        let digest = typed_data_digest_hex(&payload).unwrap();

        validate_task_signing_payload_digest(&payload, Some(&digest), Some(&digest)).unwrap();
    }

    #[test]
    fn rejects_missing_task_digest() {
        let payload = payload();
        let digest = typed_data_digest_hex(&payload).unwrap();
        let err = validate_task_signing_payload_digest(&payload, None, Some(&digest)).unwrap_err();

        assert!(err.contains("TaskView.signingDigest"));
    }

    #[test]
    fn validates_local_task_intent_creator_signature() {
        let creator = NodeSigner::random_for_test();
        let template = template();
        let (signature, digest) = sign_task_intent(&creator, &template);
        let task = task(creator.address_lc(), signature, digest);

        let typed_data = build_and_validate_task_intent(&task, Some(&template))
            .unwrap()
            .unwrap();

        assert_eq!(typed_data["primaryType"], "Ping");
    }

    #[test]
    fn rejects_local_task_intent_creator_mismatch() {
        let creator = NodeSigner::random_for_test();
        let other = NodeSigner::random_for_test();
        let template = template();
        let (signature, digest) = sign_task_intent(&creator, &template);
        let task = task(other.address_lc(), signature, digest);

        let err = build_and_validate_task_intent(&task, Some(&template)).unwrap_err();

        assert!(err.contains("creator signature mismatch"));
    }
}
