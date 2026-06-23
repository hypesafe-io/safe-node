use std::{fs, path::Path};

use super::raw::RawConfig;
use super::types::Config;
use crate::{NodeError, Result};

impl Config {
    pub(crate) fn load(path: &Path) -> Result<Self> {
        let contents = fs::read_to_string(path).map_err(|err| {
            NodeError::Config(format!("reading config {} failed: {err}", path.display()))
        })?;
        let raw: RawConfig = json5::from_str(&contents).map_err(|err| {
            NodeError::Config(format!(
                "parsing JSON5 config {} failed: {err}",
                path.display()
            ))
        })?;
        raw.validate()
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::Config;

    #[test]
    fn loads_json5_config_with_comments_and_trailing_commas() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("node.json");
        fs::write(
            &path,
            r#"
            // safe-node accepts JSON5 so operators can document local policy.
            {
              gateway_url: "http://gateway",
              hl_api_url: "http://hl",
              poll_interval_secs: 3,
              dry_run: true,
              state_db: "sqlite::memory:",
              signer: {
                keystore_path: "config/signer.json",
              },
              leader: "0x0000000000000000000000000000000000000001",
              multisig: "0x0000000000000000000000000000000000000002",
              withdraw_limit: "1000",
            }
            "#,
        )
        .unwrap();

        let config = Config::load(&path).unwrap();

        assert!(config.dry_run);
        assert_eq!(config.poll_interval_secs, 3);
        assert_eq!(
            config.allowed_leaders,
            ["0x0000000000000000000000000000000000000001".to_string()]
        );
    }
}
