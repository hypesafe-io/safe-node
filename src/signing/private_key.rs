use crate::{NodeError, Result};

pub(super) fn parse_private_key(raw: &str) -> Result<[u8; 32]> {
    let raw = raw.trim().trim_start_matches("0x");
    if raw.len() != 64 || !raw.as_bytes().iter().all(u8::is_ascii_hexdigit) {
        return Err(NodeError::Signer(
            "private key must be 32 bytes hex, with optional 0x prefix".to_string(),
        ));
    }
    let bytes = hex::decode(raw)
        .map_err(|err| NodeError::Signer(format!("private key hex decode failed: {err}")))?;
    let mut key = [0_u8; 32];
    key.copy_from_slice(&bytes);
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::parse_private_key;

    #[test]
    fn rejects_bad_private_key() {
        assert!(parse_private_key("1234").is_err());
    }

    #[test]
    fn accepts_32_byte_private_key() {
        assert!(parse_private_key(&"11".repeat(32)).is_ok());
    }
}
