use std::{fs, path::Path};

use super::raw::RawConfig;
use super::types::Config;
use crate::{NodeError, Result};

impl Config {
    pub(crate) fn load(path: &Path) -> Result<Self> {
        let contents = fs::read_to_string(path).map_err(|err| {
            NodeError::Config(format!("reading config {} failed: {err}", path.display()))
        })?;
        let raw: RawConfig = serde_json::from_str(&contents).map_err(|err| {
            NodeError::Config(format!("parsing config {} failed: {err}", path.display()))
        })?;
        raw.validate()
    }
}
