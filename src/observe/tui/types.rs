use serde_json::Value;

#[derive(Default)]
pub(super) struct TuiData {
    pub(super) status: Option<Value>,
    pub(super) config: Option<Value>,
    pub(super) policy: Option<Value>,
    pub(super) transactions: Vec<Value>,
    pub(super) error: Option<String>,
}
