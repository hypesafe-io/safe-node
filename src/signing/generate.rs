use std::path::Path;

use alloy_signer_local::PrivateKeySigner;
use rand::thread_rng;

use super::keystore::{
    finish_keystore_output, prepare_keystore_output, read_new_keystore_password,
    validate_keystore_output,
};
use crate::{NodeError, Result};

/// Generates a new signer key and writes it to an encrypted Ethereum keystore file.
///
/// # Errors
///
/// Returns an error when the output file exists without `force`, interactive
/// input fails, passwords do not match, or writing the keystore fails.
pub fn generate_keystore(out: &Path, force: bool) -> Result<()> {
    validate_keystore_output(out, force)?;
    let password = read_new_keystore_password()?;

    let signer = write_generated_keystore(out, force, password.as_bytes())?;
    println!("Generated signer: 0x{:x}", signer.address());
    println!("Wrote keystore: {}", out.display());
    Ok(())
}

fn write_generated_keystore(out: &Path, force: bool, password: &[u8]) -> Result<PrivateKeySigner> {
    if password.is_empty() {
        return Err(NodeError::Signer(
            "keystore password cannot be empty".to_string(),
        ));
    }

    let output = prepare_keystore_output(out, force)?;
    let mut rng = thread_rng();
    let (signer, _) = PrivateKeySigner::new_keystore(
        &output.parent,
        &mut rng,
        password,
        Some(output.file_name.as_str()),
    )
    .map_err(|err| NodeError::Signer(format!("writing keystore failed: {err}")))?;

    finish_keystore_output(out)?;
    Ok(signer)
}

#[cfg(test)]
mod tests {
    use super::write_generated_keystore;
    use alloy_signer_local::PrivateKeySigner;

    #[test]
    fn writes_decryptable_generated_keystore() {
        let dir = tempfile::tempdir().expect("tempdir should be created");
        let out = dir.path().join("signer.json");
        let signer = write_generated_keystore(&out, false, b"test-password")
            .expect("keystore should be written");

        let decrypted = PrivateKeySigner::decrypt_keystore(&out, b"test-password")
            .expect("keystore should decrypt");
        assert_eq!(signer.address(), decrypted.address());
    }

    #[test]
    #[cfg(unix)]
    fn generated_keystore_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("tempdir should be created");
        let out = dir.path().join("signer.json");
        write_generated_keystore(&out, false, b"test-password")
            .expect("keystore should be written");

        let mode = std::fs::metadata(out)
            .expect("metadata should be readable")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
}
