use std::str::FromStr;

use rust_decimal::Decimal;
use serde_json::Value;

use super::decision::PolicyDecision;
#[cfg(test)]
use super::decision::PolicyOutcome;
use crate::config::{normalize_address, Config, InputPolicyRule, TemplateInputPolicies};
use crate::gateway::{SubAccountRegistry, TaskView, TemplateRegistry};

const RULE_MULTISIG: &str = "multisig";
const RULE_CREATOR: &str = "creator";
const RULE_LEADER: &str = "leader";
const RULE_TEMPLATE: &str = "template";
const RULE_TEMPLATE_INPUT_POLICY: &str = "template_input_policy";
const RULE_AMOUNT: &str = "amount";
const RULE_WITHDRAW_LIMIT: &str = "withdraw_limit";
const RULE_TEMPLATE_ALLOW: &str = "template_allow";
const RULE_SEND_ASSET_SUB_ACCOUNTS: &str = "send_asset_sub_accounts";
const SEND_ASSET_TEMPLATE_ID: &str = "send_asset";

pub(crate) fn evaluate(
    config: &Config,
    templates: &TemplateRegistry,
    sub_accounts: &SubAccountRegistry,
    task: &TaskView,
) -> PolicyDecision {
    let task_multisig = match normalize_address(&task.multisig_address) {
        Ok(value) => value,
        Err(err) => return PolicyDecision::reject(RULE_MULTISIG, err.to_string()),
    };
    if task_multisig != config.multisig {
        return PolicyDecision::reject(RULE_MULTISIG, "task multisig does not match config");
    }

    let task_creator = match normalize_address(&task.creator) {
        Ok(value) => value,
        Err(err) => return PolicyDecision::reject(RULE_CREATOR, err.to_string()),
    };
    if !config
        .allowed_creators
        .iter()
        .any(|creator| creator == &task_creator)
    {
        return PolicyDecision::reject(RULE_CREATOR, "task creator is not in allowed_creators");
    }

    let task_leader = match normalize_address(&task.leader) {
        Ok(value) => value,
        Err(err) => return PolicyDecision::reject(RULE_LEADER, err.to_string()),
    };
    if !config
        .allowed_leaders
        .iter()
        .any(|leader| leader == &task_leader)
    {
        return PolicyDecision::reject(RULE_LEADER, "task leader is not in allowed_leaders");
    }

    if !config
        .allowed_templates
        .iter()
        .any(|template| template == &task.template_id)
    {
        return PolicyDecision::reject(RULE_TEMPLATE, "template is not in allowed_templates");
    }

    let Some(template) = templates.by_task(task) else {
        return PolicyDecision::reject(RULE_TEMPLATE, "template metadata is unavailable");
    };
    if let Some(rules) = config.template_input_policies.get(&task.template_id) {
        if let Err(decision) = validate_template_input_policies(rules, task) {
            return decision;
        }
    }
    if task.template_id == SEND_ASSET_TEMPLATE_ID {
        let decision = validate_send_asset_sub_accounts(&config.multisig, sub_accounts, task);
        if decision.is_reject() {
            return decision;
        }
    }
    if !template.has_amount_field()
        || has_template_amount_policy(&config.template_input_policies, task)
    {
        return PolicyDecision::allow(RULE_TEMPLATE_ALLOW);
    }

    let Some(amount_raw) = task.inputs.get("amount").and_then(|value| value.as_str()) else {
        return PolicyDecision::reject(RULE_AMOUNT, "inputs.amount is missing or not a string");
    };
    let amount = match Decimal::from_str(amount_raw.trim()) {
        Ok(value) => value,
        Err(err) => {
            return PolicyDecision::reject(RULE_AMOUNT, format!("inputs.amount is invalid: {err}"));
        }
    };
    if amount > config.withdraw_limit {
        return PolicyDecision::reject(RULE_AMOUNT, "withdraw amount exceeds withdraw_limit");
    }

    PolicyDecision::allow(RULE_WITHDRAW_LIMIT)
}

fn validate_template_input_policies(
    rules: &std::collections::BTreeMap<String, InputPolicyRule>,
    task: &TaskView,
) -> std::result::Result<(), PolicyDecision> {
    for (path, rule) in rules {
        let field = path.trim_start_matches("inputs.");
        match rule {
            InputPolicyRule::DecimalMax(limit) => {
                validate_decimal_max(field, limit, task)?;
            }
            InputPolicyRule::AddressAllowList(allowed) => {
                validate_address_allowlist(field, allowed, task)?;
            }
        }
    }
    Ok(())
}

fn validate_decimal_max(
    field: &str,
    limit: &Decimal,
    task: &TaskView,
) -> std::result::Result<(), PolicyDecision> {
    let raw = task
        .inputs
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| {
            PolicyDecision::reject(
                RULE_TEMPLATE_INPUT_POLICY,
                format!("inputs.{field} is missing or not a string"),
            )
        })?;
    let amount = Decimal::from_str(raw.trim()).map_err(|err| {
        PolicyDecision::reject(
            RULE_TEMPLATE_INPUT_POLICY,
            format!("inputs.{field} is invalid: {err}"),
        )
    })?;
    if amount > *limit {
        return Err(PolicyDecision::reject(
            RULE_TEMPLATE_INPUT_POLICY,
            format!("inputs.{field} exceeds configured maximum"),
        ));
    }
    Ok(())
}

fn validate_address_allowlist(
    field: &str,
    allowed: &[String],
    task: &TaskView,
) -> std::result::Result<(), PolicyDecision> {
    let raw = task
        .inputs
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            PolicyDecision::reject(
                RULE_TEMPLATE_INPUT_POLICY,
                format!("inputs.{field} is missing or not an address"),
            )
        })?;
    let address = normalize_address(raw).map_err(|err| {
        PolicyDecision::reject(
            RULE_TEMPLATE_INPUT_POLICY,
            format!("inputs.{field} is invalid: {err}"),
        )
    })?;
    if !allowed.iter().any(|allowed| allowed == &address) {
        return Err(PolicyDecision::reject(
            RULE_TEMPLATE_INPUT_POLICY,
            format!("inputs.{field} is not in configured address allowlist"),
        ));
    }
    Ok(())
}

fn has_template_amount_policy(policies: &TemplateInputPolicies, task: &TaskView) -> bool {
    policies
        .get(&task.template_id)
        .map(|rules| {
            rules
                .values()
                .any(|rule| matches!(rule, InputPolicyRule::DecimalMax(_)))
        })
        .unwrap_or(false)
}

fn validate_send_asset_sub_accounts(
    multisig: &str,
    sub_accounts: &SubAccountRegistry,
    task: &TaskView,
) -> PolicyDecision {
    if sub_accounts.is_empty() {
        return PolicyDecision::reject(
            RULE_SEND_ASSET_SUB_ACCOUNTS,
            "no sub-accounts are cached for configured multisig",
        );
    }

    let source = match optional_address_input(&task.inputs, "fromSubAccount") {
        None => multisig.to_string(),
        Some(address) => match normalize_address(address) {
            Ok(address) => address,
            Err(err) => {
                return PolicyDecision::reject(
                    RULE_SEND_ASSET_SUB_ACCOUNTS,
                    format!("inputs.fromSubAccount is invalid: {err}"),
                );
            }
        },
    };
    let Some(destination) = optional_address_input(&task.inputs, "destination") else {
        return PolicyDecision::reject(
            RULE_SEND_ASSET_SUB_ACCOUNTS,
            "inputs.destination is missing or not an address",
        );
    };
    let destination = match normalize_address(destination) {
        Ok(address) => address,
        Err(err) => {
            return PolicyDecision::reject(
                RULE_SEND_ASSET_SUB_ACCOUNTS,
                format!("inputs.destination is invalid: {err}"),
            );
        }
    };

    if source == multisig {
        if !sub_accounts.contains_normalized(&destination) {
            return PolicyDecision::reject(
                RULE_SEND_ASSET_SUB_ACCOUNTS,
                "inputs.destination is not a sub-account of configured multisig",
            );
        }
        return PolicyDecision::allow(RULE_SEND_ASSET_SUB_ACCOUNTS);
    }

    if destination == multisig {
        if !sub_accounts.contains_normalized(&source) {
            return PolicyDecision::reject(
                RULE_SEND_ASSET_SUB_ACCOUNTS,
                "inputs.fromSubAccount is not a sub-account of configured multisig",
            );
        }
        return PolicyDecision::allow(RULE_SEND_ASSET_SUB_ACCOUNTS);
    }

    PolicyDecision::reject(
        RULE_SEND_ASSET_SUB_ACCOUNTS,
        "send_asset must move between configured multisig and one of its sub-accounts",
    )
}

fn optional_address_input<'a>(inputs: &'a Value, field: &str) -> Option<&'a str> {
    inputs
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use rust_decimal::Decimal;
    use serde_json::json;

    use super::{evaluate, PolicyOutcome};
    use crate::config::{Config, InputPolicyRule, SignerConfig};
    use crate::gateway::{
        I18nText, SubAccountRegistry, TaskView, TemplateField, TemplateFieldType, TemplateRegistry,
        TemplateView,
    };

    fn config() -> Config {
        Config {
            gateway_url: "http://gateway".to_string(),
            hl_api_url: "http://hl".to_string(),
            poll_interval_secs: 15,
            dry_run: false,
            allowed_templates: vec![
                "withdraw3".to_string(),
                "sub_account_withdraw3".to_string(),
                "create_sub_account".to_string(),
                "send_asset".to_string(),
            ],
            allowed_creators: vec!["0x0000000000000000000000000000000000000001".to_string()],
            allowed_leaders: vec!["0x0000000000000000000000000000000000000001".to_string()],
            template_input_policies: Default::default(),
            state_db: "sqlite::memory:".to_string(),
            rpc_http_addr: "127.0.0.1:9909".parse().unwrap(),
            rpc_auth_token: None,
            signer: SignerConfig {
                keystore_path: "signer.json".to_string(),
                password_env: None,
            },
            leader: "0x0000000000000000000000000000000000000001".to_string(),
            multisig: "0x0000000000000000000000000000000000000002".to_string(),
            withdraw_limit: Decimal::new(1000, 0),
        }
    }

    fn templates() -> TemplateRegistry {
        TemplateRegistry::new(vec![
            template(
                "withdraw3",
                vec![field("amount", TemplateFieldType::Amount)],
            ),
            template(
                "sub_account_withdraw3",
                vec![field("amount", TemplateFieldType::Amount)],
            ),
            template(
                "create_sub_account",
                vec![field("name", TemplateFieldType::String)],
            ),
            template(
                "send_asset",
                vec![
                    field("destination", TemplateFieldType::Address),
                    field("fromSubAccount", TemplateFieldType::String),
                    field("amount", TemplateFieldType::Amount),
                ],
            ),
        ])
    }

    fn sub_accounts() -> SubAccountRegistry {
        SubAccountRegistry::from_addresses(&[
            "0x00000000000000000000000000000000000000aa",
            "0x00000000000000000000000000000000000000bb",
        ])
    }

    fn template(id: &str, fields: Vec<TemplateField>) -> TemplateView {
        TemplateView {
            id: id.to_string(),
            version: 1,
            type_name: "test".to_string(),
            hl_action_type: Some(id.to_string()),
            display_name: text(id),
            description: text(id),
            summary: text(id),
            fields,
            signing: None,
            exchange: None,
        }
    }

    fn field(name: &str, field_type: TemplateFieldType) -> TemplateField {
        TemplateField {
            name: name.to_string(),
            field_type,
            required: true,
            label: text(name),
            description: text(name),
        }
    }

    fn text(value: &str) -> I18nText {
        I18nText {
            en: value.to_string(),
            zh: value.to_string(),
        }
    }

    fn task(template_id: &str, inputs: serde_json::Value) -> TaskView {
        TaskView {
            id: "task".to_string(),
            multisig_address: "0x0000000000000000000000000000000000000002".to_string(),
            creator: "0x0000000000000000000000000000000000000001".to_string(),
            leader: "0x0000000000000000000000000000000000000001".to_string(),
            nonce: 1,
            network: "mainnet".to_string(),
            template_id: template_id.to_string(),
            template_version: 1,
            inputs,
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

    #[test]
    fn rejects_non_withdraw_template() {
        let mut config = config();
        config.allowed_templates = vec![
            "withdraw3".to_string(),
            "sub_account_withdraw3".to_string(),
            "create_sub_account".to_string(),
        ];

        let decision = evaluate(
            &config,
            &templates(),
            &sub_accounts(),
            &task("send_asset", json!({ "amount": "1" })),
        );
        assert_eq!(decision.outcome, PolicyOutcome::Reject);
    }

    #[test]
    fn rejects_template_missing_from_allow_list() {
        let mut config = config();
        config.allowed_templates = vec!["withdraw3".to_string()];

        let decision = evaluate(
            &config,
            &templates(),
            &sub_accounts(),
            &task("sub_account_withdraw3", json!({ "amount": "1" })),
        );

        assert_eq!(decision.outcome, PolicyOutcome::Reject);
    }

    #[test]
    fn rejects_non_allowed_creator() {
        let mut task = task("withdraw3", json!({ "amount": "1" }));
        task.creator = "0x00000000000000000000000000000000000000ff".to_string();

        let decision = evaluate(&config(), &templates(), &sub_accounts(), &task);

        assert_eq!(decision.outcome, PolicyOutcome::Reject);
        assert_eq!(decision.rule, "creator");
    }

    #[test]
    fn rejects_non_allowed_leader() {
        let mut task = task("withdraw3", json!({ "amount": "1" }));
        task.leader = "0x00000000000000000000000000000000000000ff".to_string();

        let decision = evaluate(&config(), &templates(), &sub_accounts(), &task);

        assert_eq!(decision.outcome, PolicyOutcome::Reject);
        assert_eq!(decision.rule, "leader");
    }

    #[test]
    fn applies_template_destination_allowlist() {
        let mut config = config();
        config.template_input_policies = template_policy(
            "withdraw3",
            "inputs.destination",
            InputPolicyRule::AddressAllowList(vec![
                "0x00000000000000000000000000000000000000aa".to_string()
            ]),
        );

        let rejected = evaluate(
            &config,
            &templates(),
            &sub_accounts(),
            &task(
                "withdraw3",
                json!({
                    "destination": "0x00000000000000000000000000000000000000bb",
                    "amount": "1"
                }),
            ),
        );
        let allowed = evaluate(
            &config,
            &templates(),
            &sub_accounts(),
            &task(
                "withdraw3",
                json!({
                    "destination": "0x00000000000000000000000000000000000000aa",
                    "amount": "1"
                }),
            ),
        );

        assert_eq!(rejected.outcome, PolicyOutcome::Reject);
        assert_eq!(rejected.rule, "template_input_policy");
        assert_eq!(allowed.outcome, PolicyOutcome::Allow);
    }

    #[test]
    fn applies_template_amount_max_before_withdraw_limit_fallback() {
        let mut config = config();
        config.withdraw_limit = Decimal::new(1000, 0);
        config.template_input_policies = template_policy(
            "withdraw3",
            "inputs.amount",
            InputPolicyRule::DecimalMax(Decimal::new(10, 0)),
        );

        let rejected = evaluate(
            &config,
            &templates(),
            &sub_accounts(),
            &task("withdraw3", json!({ "amount": "10.01" })),
        );
        let allowed = evaluate(
            &config,
            &templates(),
            &sub_accounts(),
            &task("withdraw3", json!({ "amount": "10" })),
        );

        assert_eq!(rejected.outcome, PolicyOutcome::Reject);
        assert_eq!(rejected.rule, "template_input_policy");
        assert_eq!(allowed.outcome, PolicyOutcome::Allow);
    }

    #[test]
    fn rejects_withdraw_over_limit() {
        let decision = evaluate(
            &config(),
            &templates(),
            &sub_accounts(),
            &task("withdraw3", json!({ "amount": "1001" })),
        );
        assert_eq!(decision.outcome, PolicyOutcome::Reject);
    }

    #[test]
    fn allows_withdraw_under_limit() {
        let decision = evaluate(
            &config(),
            &templates(),
            &sub_accounts(),
            &task("withdraw3", json!({ "amount": "1000" })),
        );
        assert_eq!(decision.outcome, PolicyOutcome::Allow);
    }

    #[test]
    fn allows_non_amount_template_when_configured() {
        let decision = evaluate(
            &config(),
            &templates(),
            &sub_accounts(),
            &task("create_sub_account", json!({ "name": "desk" })),
        );

        assert_eq!(decision.outcome, PolicyOutcome::Allow);
    }

    #[test]
    fn rejects_amount_template_missing_amount() {
        let decision = evaluate(
            &config(),
            &templates(),
            &sub_accounts(),
            &task("withdraw3", json!({})),
        );

        assert_eq!(decision.outcome, PolicyOutcome::Reject);
    }

    #[test]
    fn rejects_unknown_template_metadata() {
        let mut config = config();
        config.allowed_templates = vec!["unknown".to_string()];

        let decision = evaluate(
            &config,
            &templates(),
            &sub_accounts(),
            &task("unknown", json!({})),
        );

        assert_eq!(decision.outcome, PolicyOutcome::Reject);
    }

    #[test]
    fn allows_send_asset_from_multisig_to_configured_sub_account() {
        let decision = evaluate(
            &config(),
            &templates(),
            &sub_accounts(),
            &task(
                "send_asset",
                json!({
                    "destination": "0x00000000000000000000000000000000000000bb",
                    "fromSubAccount": "",
                    "amount": "10"
                }),
            ),
        );

        assert_eq!(decision.outcome, PolicyOutcome::Allow);
    }

    #[test]
    fn allows_send_asset_from_configured_sub_account_to_multisig() {
        let decision = evaluate(
            &config(),
            &templates(),
            &sub_accounts(),
            &task(
                "send_asset",
                json!({
                    "destination": "0x0000000000000000000000000000000000000002",
                    "fromSubAccount": "0x00000000000000000000000000000000000000aa",
                    "amount": "10"
                }),
            ),
        );

        assert_eq!(decision.outcome, PolicyOutcome::Allow);
    }

    #[test]
    fn rejects_send_asset_to_external_destination() {
        let decision = evaluate(
            &config(),
            &templates(),
            &sub_accounts(),
            &task(
                "send_asset",
                json!({
                    "destination": "0x00000000000000000000000000000000000000cc",
                    "fromSubAccount": "0x00000000000000000000000000000000000000aa",
                    "amount": "10"
                }),
            ),
        );

        assert_eq!(decision.outcome, PolicyOutcome::Reject);
        assert_eq!(decision.rule, "send_asset_sub_accounts");
    }

    #[test]
    fn rejects_send_asset_between_two_sub_accounts() {
        let decision = evaluate(
            &config(),
            &templates(),
            &sub_accounts(),
            &task(
                "send_asset",
                json!({
                    "destination": "0x00000000000000000000000000000000000000bb",
                    "fromSubAccount": "0x00000000000000000000000000000000000000aa",
                    "amount": "10"
                }),
            ),
        );

        assert_eq!(decision.outcome, PolicyOutcome::Reject);
        assert_eq!(decision.rule, "send_asset_sub_accounts");
    }

    #[test]
    fn rejects_send_asset_from_external_source_to_multisig() {
        let decision = evaluate(
            &config(),
            &templates(),
            &sub_accounts(),
            &task(
                "send_asset",
                json!({
                    "destination": "0x0000000000000000000000000000000000000002",
                    "fromSubAccount": "0x00000000000000000000000000000000000000cc",
                    "amount": "10"
                }),
            ),
        );

        assert_eq!(decision.outcome, PolicyOutcome::Reject);
        assert_eq!(decision.rule, "send_asset_sub_accounts");
    }

    #[test]
    fn rejects_send_asset_when_sub_accounts_are_not_cached() {
        let decision = evaluate(
            &config(),
            &templates(),
            &SubAccountRegistry::default(),
            &task(
                "send_asset",
                json!({
                    "destination": "0x00000000000000000000000000000000000000bb",
                    "fromSubAccount": "0x00000000000000000000000000000000000000aa",
                    "amount": "10"
                }),
            ),
        );

        assert_eq!(decision.outcome, PolicyOutcome::Reject);
        assert_eq!(
            decision.reason.as_deref(),
            Some("no sub-accounts are cached for configured multisig")
        );
    }

    #[test]
    fn exposes_allowed_templates_for_debug_surfaces() {
        assert_eq!(
            config().allowed_templates,
            [
                "withdraw3".to_string(),
                "sub_account_withdraw3".to_string(),
                "create_sub_account".to_string(),
                "send_asset".to_string()
            ]
        );
    }

    fn template_policy(
        template_id: &str,
        path: &str,
        rule: InputPolicyRule,
    ) -> crate::config::TemplateInputPolicies {
        let mut rules = std::collections::BTreeMap::new();
        rules.insert(path.to_string(), rule);
        let mut policies = crate::config::TemplateInputPolicies::new();
        policies.insert(template_id.to_string(), rules);
        policies
    }
}
