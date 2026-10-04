use sha2::{Digest, Sha256};

pub const ACCOUNT_NAMESPACE_DOMAIN: &[u8] = b"gridtimer-account-namespace-v1\0";

/// Stable account workspace namespace shared by the server and every client.
/// Lengths are little-endian byte lengths of the UTF-8 fields.
pub fn account_namespace_identifier(server_instance_id: &str, user_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(ACCOUNT_NAMESPACE_DOMAIN);
    hasher.update((server_instance_id.len() as u64).to_le_bytes());
    hasher.update(server_instance_id.as_bytes());
    hasher.update((user_id.len() as u64).to_le_bytes());
    hasher.update(user_id.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespace_vector_is_stable() {
        assert_eq!(
            "2092f1c2444c15b6efb42d27e9c1add2b9a2ef4cb2c691bb1f28d22e1b9d0df6",
            account_namespace_identifier(
                "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                "user-1",
            )
        );
    }
}
