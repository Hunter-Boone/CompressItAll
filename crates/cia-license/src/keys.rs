//! Public keys the app trusts, by kid (DESIGN.md 5.5). The server signs with
//! `LICENSE_SIGNING_KID`, which must appear here. Rotation: add the new key,
//! ship, wait for installs to update, then switch the server; the old key
//! stays for one more major version.
//!
//! Generate a pair with `cargo xtask keygen` (prints the seed for
//! `LICENSE_SIGNING_KEY` and the entry to paste below).

/// Kid of the key that signs test vectors (`crates/cia-license/test-keys.json`).
#[cfg(any(test, feature = "e2e"))]
pub const TEST_KID: &str = "test";

/// Public half of the test key. Its seed is public; never sign anything real with it.
#[cfg(any(test, feature = "e2e"))]
pub const TEST_PUBLIC_KEY: [u8; 32] = [
    0x71, 0x8d, 0xd1, 0xfb, 0x15, 0xc4, 0xf5, 0xbd, 0x97, 0xe1, 0x1f, 0x4d, 0x21, 0xc2, 0xa9, 0x7f,
    0x15, 0x48, 0xe6, 0x25, 0xa4, 0xaf, 0x24, 0xd6, 0xe3, 0x7c, 0x80, 0xdb, 0x31, 0x5c, 0xba, 0x97,
];

/// `(kid, Ed25519 public key)`; first match wins.
pub const PUBLIC_KEYS: &[(&str, [u8; 32])] = &[
    // PLACEHOLDER: generated for development; replace with the output of
    // `cargo xtask keygen` before launch (its seed sits in test-keys.json).
    (
        "2026-10",
        [
            0x43, 0x98, 0x9d, 0xdd, 0x3c, 0x48, 0xfb, 0x73, 0x99, 0xf1, 0x93, 0x86, 0x17, 0x56,
            0x5f, 0x09, 0xf1, 0xdd, 0x62, 0x98, 0x00, 0xf4, 0xe6, 0xd7, 0x5d, 0xff, 0x3a, 0x87,
            0xba, 0xa6, 0xa6, 0x64,
        ],
    ),
    #[cfg(any(test, feature = "e2e"))]
    (TEST_KID, TEST_PUBLIC_KEY),
];

/// Look up a public key by kid.
pub fn find(kid: &str) -> Option<&'static [u8; 32]> {
    PUBLIC_KEYS
        .iter()
        .find(|(k, _)| *k == kid)
        .map(|(_, key)| key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;

    #[test]
    fn public_keys_match_test_keys_json() {
        let doc: serde_json::Value =
            serde_json::from_str(include_str!("../test-keys.json")).unwrap();
        let mut checked = 0;
        for entry in doc["keys"].as_array().unwrap() {
            let kid = entry["kid"].as_str().unwrap();
            let seed: [u8; 32] = STANDARD
                .decode(entry["seed_b64"].as_str().unwrap())
                .unwrap()
                .try_into()
                .unwrap();
            let public = crate::token::public_key(&seed);
            let hex: String = public.iter().map(|b| format!("{b:02x}")).collect();
            assert_eq!(hex, entry["public_key_hex"], "public_key_hex for {kid}");
            if let Some(listed) = find(kid) {
                assert_eq!(
                    *listed, public,
                    "keys.rs entry for {kid} does not match its seed"
                );
                checked += 1;
            }
        }
        assert!(
            checked >= 1,
            "test-keys.json must describe at least one listed key"
        );
    }

    #[test]
    fn kids_are_unique() {
        for (i, (a, _)) in PUBLIC_KEYS.iter().enumerate() {
            assert!(
                !PUBLIC_KEYS[i + 1..].iter().any(|(b, _)| a == b),
                "duplicate kid {a}"
            );
        }
    }
}
