use std::convert::TryFrom;

use serde_json::Value;

use crate::gateway::{TaskView, TemplateView};
use crate::signing::typed_data_digest_hex;

type IntentResult<T> = std::result::Result<T, String>;

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
