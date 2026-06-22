use std::io::IsTerminal;

use dialoguer::{theme::ColorfulTheme, Password};

use crate::{NodeError, Result};

pub(super) fn read_keystore_password(password_env: Option<&str>) -> Result<String> {
    if let Some(value) = password_env
        .and_then(|env_key| std::env::var(env_key).ok())
        .filter(|value| !value.is_empty())
    {
        return Ok(value);
    }

    if !std::io::stdin().is_terminal() {
        return Err(NodeError::Signer(
            "keystore password env is empty and stdin is not an interactive TTY".to_string(),
        ));
    }
    let password = Password::with_theme(&ColorfulTheme::default())
        .with_prompt("Keystore password")
        .interact()
        .map_err(|err| NodeError::Signer(format!("reading keystore password failed: {err}")))?;
    if password.is_empty() {
        return Err(NodeError::Signer(
            "keystore password cannot be empty".to_string(),
        ));
    }
    Ok(password)
}
