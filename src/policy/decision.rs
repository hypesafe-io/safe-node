#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PolicyOutcome {
    Allow,
    Reject,
}

#[derive(Debug, Clone)]
pub(crate) struct PolicyDecision {
    pub(crate) outcome: PolicyOutcome,
    pub(crate) reason: Option<String>,
    pub(crate) rule: String,
}

impl PolicyDecision {
    pub(crate) fn allow(rule: impl Into<String>) -> Self {
        Self {
            outcome: PolicyOutcome::Allow,
            reason: None,
            rule: rule.into(),
        }
    }

    pub(crate) fn reject(rule: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            outcome: PolicyOutcome::Reject,
            reason: Some(reason.into()),
            rule: rule.into(),
        }
    }

    pub(crate) fn is_reject(&self) -> bool {
        self.outcome == PolicyOutcome::Reject
    }
}
