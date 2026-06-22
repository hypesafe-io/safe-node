use std::path::Path;

use alloy_dyn_abi::TypedData;
use alloy_primitives::{Signature, B256};
use alloy_signer::SignerSync;
use alloy_signer_local::PrivateKeySigner;
use serde_json::Value;

use super::password::read_keystore_password;
use crate::{NodeError, Result};

#[derive(Clone)]
pub(crate) struct NodeSigner {
    signer: PrivateKeySigner,
    address_lc: String,
}

impl NodeSigner {
    pub(crate) fn decrypt(keystore_path: &Path, password_env: Option<&str>) -> Result<Self> {
        let password = read_keystore_password(password_env)?;
        let signer = PrivateKeySigner::decrypt_keystore(keystore_path, password.as_bytes())
            .map_err(|err| NodeError::Signer(format!("decrypting keystore failed: {err}")))?;
        let address_lc = format!("0x{:x}", signer.address());
        Ok(Self { signer, address_lc })
    }

    #[cfg(test)]
    pub(crate) fn random_for_test() -> Self {
        let signer = PrivateKeySigner::random();
        let address_lc = format!("0x{:x}", signer.address());
        Self { signer, address_lc }
    }

    pub(crate) fn address_lc(&self) -> &str {
        &self.address_lc
    }

    pub(crate) fn sign_message(&self, message: &str) -> Result<String> {
        self.signer
            .sign_message_sync(message.as_bytes())
            .map(|sig| sig.to_string())
            .map_err(|err| NodeError::Signer(format!("signing message failed: {err}")))
    }

    pub(crate) fn sign_and_verify_typed_data(&self, typed_data: &Value) -> Result<String> {
        let typed = parse_typed_data(typed_data)?;
        let signature = self
            .signer
            .sign_dynamic_typed_data_sync(&typed)
            .map_err(|err| NodeError::Signer(format!("signing typed-data failed: {err}")))?;
        let recovered = recover_typed_data_signer(&typed, &signature)?;
        if recovered != self.address_lc {
            return Err(NodeError::Signer(format!(
                "typed-data signer mismatch: expected {}, got {}",
                self.address_lc, recovered
            )));
        }
        Ok(signature.to_string())
    }
}

pub(crate) fn typed_data_digest_hex(typed_data: &Value) -> std::result::Result<String, String> {
    hypesafe_signing_intent::signing_digest(typed_data).map_err(|err| err.to_string())
}

fn parse_typed_data(typed_data: &Value) -> Result<TypedData> {
    serde_json::from_value(typed_data.clone())
        .map_err(|err| NodeError::Signer(format!("invalid typed-data: {err}")))
}

fn typed_data_hash(typed_data: &TypedData) -> std::result::Result<B256, String> {
    typed_data
        .eip712_signing_hash()
        .map_err(|err| format!("hashing typed-data failed: {err}"))
}

fn recover_typed_data_signer(typed_data: &TypedData, signature: &Signature) -> Result<String> {
    let hash = typed_data_hash(typed_data).map_err(NodeError::Signer)?;
    signature
        .recover_address_from_prehash(&hash)
        .map(|address| format!("0x{address:x}"))
        .map_err(|err| NodeError::Signer(format!("recovering typed-data signer failed: {err}")))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{typed_data_digest_hex, NodeSigner};

    fn signer() -> NodeSigner {
        NodeSigner::random_for_test()
    }

    #[test]
    fn sign_and_verify_typed_data_recovers_local_signer() {
        let signer = signer();
        let payload = json!({
            "domain": {
                "name": "HypeSafe",
                "version": "1",
                "chainId": 42161,
                "verifyingContract": "0x0000000000000000000000000000000000000001"
            },
            "primaryType": "Ping",
            "types": {
                "Ping": [
                    { "name": "sender", "type": "address" },
                    { "name": "nonce", "type": "uint64" }
                ]
            },
            "message": {
                "sender": signer.address_lc(),
                "nonce": 1
            }
        });

        let signature = signer.sign_and_verify_typed_data(&payload).unwrap();

        assert!(signature.starts_with("0x"));
        assert_eq!(signature.len(), 132);
        assert!(typed_data_digest_hex(&payload).unwrap().starts_with("0x"));
    }
}
