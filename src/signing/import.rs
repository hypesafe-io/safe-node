use std::path::Path;

use alloy_signer_local::PrivateKeySigner;
use dialoguer::{theme::ColorfulTheme, Password};
use rand::thread_rng;

use super::keystore::{
    finish_keystore_output, prepare_keystore_output, read_new_keystore_password,
    validate_keystore_output,
};
use super::private_key::parse_private_key;
use crate::{NodeError, Result};

/// Imports a raw private key into an encrypted Ethereum keystore file.
///
/// # Errors
///
/// Returns an error when the output file exists without `force`, interactive
/// input fails, the private key is invalid, passwords do not match, or writing
/// the keystore fails.
pub fn import_keystore(out: &Path, force: bool) -> Result<()> {
    validate_keystore_output(out, force)?;

    let theme = ColorfulTheme::default();
    let private_key = Password::with_theme(&theme)
        .with_prompt("Private key")
        .interact()
        .map_err(|err| NodeError::Signer(format!("reading private key failed: {err}")))?;
    let password = read_new_keystore_password()?;

    let key = parse_private_key(&private_key)?;
    let output = prepare_keystore_output(out, force)?;
    let mut rng = thread_rng();
    let (signer, _) = PrivateKeySigner::encrypt_keystore(
        &output.parent,
        &mut rng,
        key,
        password.as_bytes(),
        Some(output.file_name.as_str()),
    )
    .map_err(|err| NodeError::Signer(format!("writing keystore failed: {err}")))?;

    finish_keystore_output(out)?;
    println!("Imported signer: 0x{:x}", signer.address());
    println!("Wrote keystore: {}", out.display());
    Ok(())
}
