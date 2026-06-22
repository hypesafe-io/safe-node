use alloy_primitives::Address;

use crate::{NodeError, Result};

pub(crate) fn normalize_address(raw: &str) -> Result<String> {
    let address: Address = raw
        .trim()
        .parse()
        .map_err(|_| NodeError::Config(format!("invalid address: {raw}")))?;
    Ok(format!("0x{address:x}"))
}

pub(crate) fn trim_url(raw: &str) -> String {
    raw.trim().trim_end_matches('/').to_string()
}

#[cfg(test)]
mod tests {
    use super::normalize_address;

    #[test]
    fn normalizes_address_to_lowercase() {
        let address = normalize_address("0x000000000000000000000000000000000000dEaD").unwrap();
        assert_eq!(address, "0x000000000000000000000000000000000000dead");
    }
}
