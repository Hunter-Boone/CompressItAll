//! The one FFmpeg release this build of Smidge installs (DESIGN.md 6.4).
//!
//! A new FFmpeg reaches users only through an app release that changes these
//! constants. The manifest is signed with `LIBRARIES_MANIFEST_KEY`; its public
//! half is pinned here.
//!
//! PLACEHOLDER: the URL points at a release tag that does not exist yet and
//! the key pair below was generated for development. Its seed sits in
//! `crates/cia-ffmpeg/test-keys.json` so tests can sign manifests. Before the
//! first real Smidge-Libraries release: generate the real pair, keep the seed
//! only in the Smidge-Libraries CI secret, replace `PUBLIC_KEY`, and delete
//! the placeholder entry from test-keys.json.

/// What the installer pins: where the manifest is and who may sign it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pin {
    pub manifest_url: &'static str,
    pub signature_url: &'static str,
    /// Expected `version` field; the manifest is rejected when it differs.
    pub version: &'static str,
    pub public_key: [u8; 32],
}

pub const VERSION: &str = "7.1.2";
pub const REVISION: u32 = 1;

pub const MANIFEST_URL: &str =
    "https://github.com/Hunter-Boone/Smidge-Libraries/releases/download/ffmpeg-7.1.2-r1/manifest.json";
pub const SIGNATURE_URL: &str =
    "https://github.com/Hunter-Boone/Smidge-Libraries/releases/download/ffmpeg-7.1.2-r1/manifest.json.sig";

/// PLACEHOLDER public key (seed in test-keys.json, kid "libraries-dev").
pub const PUBLIC_KEY: [u8; 32] = [
    0xd9, 0xc5, 0x05, 0xed, 0x7e, 0xf1, 0xbb, 0x33, 0x4f, 0xdc, 0x34, 0x26, 0x6a, 0xd7, 0x44, 0x4f,
    0x84, 0x67, 0xca, 0x57, 0xe2, 0xc1, 0xb4, 0x49, 0xd8, 0xae, 0x44, 0x93, 0xcb, 0x3d, 0x19, 0x22,
];

pub const PIN: Pin = Pin {
    manifest_url: MANIFEST_URL,
    signature_url: SIGNATURE_URL,
    version: VERSION,
    public_key: PUBLIC_KEY,
};

/// Approximate unpacked size quoted in the disk-full message (DESIGN 4.8: "about 90 MB").
pub const APPROX_UNPACKED_BYTES: u64 = 90 * 1024 * 1024;

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;

    #[test]
    fn public_key_matches_test_keys_json() {
        let doc: serde_json::Value =
            serde_json::from_str(include_str!("../test-keys.json")).unwrap();
        let entry = doc["keys"]
            .as_array()
            .unwrap()
            .iter()
            .find(|k| k["kid"] == "libraries-dev")
            .expect("libraries-dev entry");
        let seed: [u8; 32] = STANDARD
            .decode(entry["seed_b64"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        let public = ed25519_dalek::SigningKey::from_bytes(&seed)
            .verifying_key()
            .to_bytes();
        assert_eq!(public, PUBLIC_KEY);
        let hex: String = public.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, entry["public_key_hex"]);
    }

    #[test]
    fn urls_follow_the_release_tag_convention() {
        let tag = format!("ffmpeg-{VERSION}-r{REVISION}");
        assert!(MANIFEST_URL.contains(&tag));
        assert!(MANIFEST_URL.ends_with("/manifest.json"));
        assert_eq!(SIGNATURE_URL, format!("{MANIFEST_URL}.sig"));
    }
}
