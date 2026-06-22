#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RunMode {
    LeaderExecutor,
    CoSigner,
}

impl RunMode {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::LeaderExecutor => "leader-executor",
            Self::CoSigner => "co-signer",
        }
    }
}
