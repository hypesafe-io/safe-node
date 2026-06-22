use std::fs;
use std::path::{Path, PathBuf};

use dialoguer::{theme::ColorfulTheme, Password};

use crate::{NodeError, Result};

pub(super) struct KeystoreOutput {
    pub(super) parent: PathBuf,
    pub(super) file_name: String,
}

pub(super) fn read_new_keystore_password() -> Result<String> {
    let theme = ColorfulTheme::default();
    let password = Password::with_theme(&theme)
        .with_prompt("Keystore password")
        .interact()
        .map_err(|err| NodeError::Signer(format!("reading password failed: {err}")))?;
    if password.is_empty() {
        return Err(NodeError::Signer(
            "keystore password cannot be empty".to_string(),
        ));
    }
    let confirm = Password::with_theme(&theme)
        .with_prompt("Confirm password")
        .interact()
        .map_err(|err| NodeError::Signer(format!("reading password confirmation failed: {err}")))?;
    if password != confirm {
        return Err(NodeError::Signer(
            "keystore passwords do not match".to_string(),
        ));
    }

    Ok(password)
}

pub(super) fn validate_keystore_output(out: &Path, force: bool) -> Result<()> {
    if out.exists() && !force {
        return Err(NodeError::Signer(format!(
            "keystore {} already exists; pass --force to overwrite",
            out.display()
        )));
    }
    keystore_file_name(out).map(|_| ())
}

pub(super) fn prepare_keystore_output(out: &Path, force: bool) -> Result<KeystoreOutput> {
    validate_keystore_output(out, force)?;

    let parent = out
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let file_name = keystore_file_name(out)?;

    if out.exists() {
        fs::remove_file(out)?;
    }

    Ok(KeystoreOutput {
        parent: parent.to_path_buf(),
        file_name: file_name.to_string(),
    })
}

fn keystore_file_name(out: &Path) -> Result<&str> {
    out.file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| NodeError::Signer(format!("invalid output path: {}", out.display())))
}

pub(super) fn finish_keystore_output(out: &Path) -> Result<()> {
    set_owner_only_permissions(out)
}

#[cfg(unix)]
fn set_owner_only_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mut perms = fs::metadata(path)?.permissions();
    perms.set_mode(0o600);
    fs::set_permissions(path, perms)?;
    Ok(())
}

#[cfg(not(unix))]
fn set_owner_only_permissions(_path: &Path) -> Result<()> {
    Ok(())
}
