//! Device identity (DESIGN.md 5.5), native only.
//!
//! ```text
//! machine_id = Windows: HKLM\SOFTWARE\Microsoft\Cryptography\MachineGuid
//!              macOS:   IOPlatformUUID from `ioreg -rd1 -c IOPlatformExpertDevice`
//!              Linux:   /etc/machine-id, else /var/lib/dbus/machine-id
//! device_hash = "d_" + base32(sha256("smidge-device-v1|" + os + "|" + upper(machine_id))[0..16]).lower()
//! ```
//!
//! The raw machine id never leaves the computer; only the hash is sent.

use data_encoding::BASE32_NOPAD;
use sha2::{Digest, Sha256};

/// Domain separator in the hash preimage. Bump to `v2` only with a migration.
pub const HASH_DOMAIN: &str = "smidge-device-v1";

#[derive(Debug, thiserror::Error)]
pub enum DeviceError {
    #[error("no machine id available: {0}")]
    Unavailable(String),
}

/// `"d_" + base32_lower(sha256(domain|os|UPPER(machine_id))[..16])`. Pure, so
/// the same inputs give the same hash on every platform (tested).
pub fn device_hash_from(os: &str, machine_id: &str) -> String {
    let preimage = format!(
        "{HASH_DOMAIN}|{os}|{}",
        machine_id.trim().to_ascii_uppercase()
    );
    let digest = Sha256::digest(preimage.as_bytes());
    format!("d_{}", base32_lower(&digest[..16]))
}

/// Web install id: `"w_" + base32_lower(16 random bytes)`. The web app creates
/// it on first load and keeps it in IndexedDB; this is the reference encoding.
pub fn web_install_id(random16: [u8; 16]) -> String {
    format!("w_{}", base32_lower(&random16))
}

fn base32_lower(bytes: &[u8]) -> String {
    BASE32_NOPAD.encode(bytes).to_ascii_lowercase()
}

/// Hash for this computer. Falls back to the hostname when no machine id can
/// be read (a stripped-down container, a broken registry), so activation still
/// works; that device simply re-activates if the hostname changes.
pub fn device_hash() -> String {
    let id = machine_id().unwrap_or_else(|_| format!("hostname:{}", device_name()));
    device_hash_from(std::env::consts::OS, &id)
}

/// Human-readable name shown in the account dashboard: the hostname.
pub fn device_name() -> String {
    let name = gethostname::gethostname()
        .to_string_lossy()
        .trim()
        .to_string();
    if name.is_empty() {
        "Unknown computer".to_string()
    } else {
        name
    }
}

/// The platform's stable machine identifier, raw. Never send this anywhere.
pub fn machine_id() -> Result<String, DeviceError> {
    platform::machine_id()
}

#[cfg(target_os = "windows")]
mod platform {
    use super::DeviceError;
    use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_64KEY};
    use winreg::RegKey;

    pub fn machine_id() -> Result<String, DeviceError> {
        let key = RegKey::predef(HKEY_LOCAL_MACHINE)
            .open_subkey_with_flags(
                "SOFTWARE\\Microsoft\\Cryptography",
                KEY_READ | KEY_WOW64_64KEY,
            )
            .map_err(|e| DeviceError::Unavailable(format!("open Cryptography key: {e}")))?;
        let guid: String = key
            .get_value("MachineGuid")
            .map_err(|e| DeviceError::Unavailable(format!("read MachineGuid: {e}")))?;
        non_empty(guid)
    }

    fn non_empty(s: String) -> Result<String, DeviceError> {
        let t = s.trim().to_string();
        if t.is_empty() {
            Err(DeviceError::Unavailable("MachineGuid is empty".into()))
        } else {
            Ok(t)
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::DeviceError;
    use std::process::Command;

    pub fn machine_id() -> Result<String, DeviceError> {
        let out = Command::new("ioreg")
            .args(["-rd1", "-c", "IOPlatformExpertDevice"])
            .output()
            .map_err(|e| DeviceError::Unavailable(format!("run ioreg: {e}")))?;
        let text = String::from_utf8_lossy(&out.stdout);
        parse_ioreg(&text)
            .ok_or_else(|| DeviceError::Unavailable("IOPlatformUUID not in ioreg output".into()))
    }

    /// Finds `"IOPlatformUUID" = "XXXXXXXX-..."` in `ioreg` output.
    pub fn parse_ioreg(text: &str) -> Option<String> {
        super::parse_quoted_value(text, "IOPlatformUUID")
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
mod platform {
    use super::DeviceError;

    pub fn machine_id() -> Result<String, DeviceError> {
        let mut last = String::new();
        for path in ["/etc/machine-id", "/var/lib/dbus/machine-id"] {
            match std::fs::read_to_string(path) {
                Ok(s) if !s.trim().is_empty() => return Ok(s.trim().to_string()),
                Ok(_) => last = format!("{path} is empty"),
                Err(e) => last = format!("{path}: {e}"),
            }
        }
        Err(DeviceError::Unavailable(last))
    }
}

/// `"<key>" = "<value>"` lookup used for `ioreg` output; kept out of the cfg
/// block so it is unit-tested on every platform.
#[allow(dead_code)]
fn parse_quoted_value(text: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    for line in text.lines() {
        let Some(rest) = line.split_once(&needle).map(|(_, r)| r) else {
            continue;
        };
        let rest = rest.trim_start().strip_prefix('=')?.trim_start();
        let rest = rest.strip_prefix('"')?;
        let value = rest.split('"').next()?.trim();
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_deterministic_and_case_insensitive() {
        let a = device_hash_from("linux", "4c2e8f3a9b1d4e5f8a7b6c5d4e3f2a1b");
        let b = device_hash_from("linux", "4C2E8F3A9B1D4E5F8A7B6C5D4E3F2A1B\n");
        assert_eq!(a, b);
        assert!(a.starts_with("d_"));
        // 16 bytes -> 26 base32 characters, no padding, lower case.
        assert_eq!(a.len(), 2 + 26);
        assert!(a[2..]
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()));
        assert_ne!(
            a,
            device_hash_from("windows", "4c2e8f3a9b1d4e5f8a7b6c5d4e3f2a1b")
        );
    }

    #[test]
    fn hash_matches_reference_vector() {
        // sha256("smidge-device-v1|windows|9F6A8B1C-2D3E-4F50-A1B2-C3D4E5F60718")[..16], base32 lower.
        let preimage = "smidge-device-v1|windows|9F6A8B1C-2D3E-4F50-A1B2-C3D4E5F60718";
        let digest = Sha256::digest(preimage.as_bytes());
        let expected = format!(
            "d_{}",
            BASE32_NOPAD.encode(&digest[..16]).to_ascii_lowercase()
        );
        assert_eq!(
            device_hash_from("windows", "9f6a8b1c-2d3e-4f50-a1b2-c3d4e5f60718"),
            expected
        );
    }

    #[test]
    fn web_install_id_shape() {
        let id = web_install_id([0xAB; 16]);
        assert!(id.starts_with("w_"));
        assert_eq!(id.len(), 28);
        assert_eq!(web_install_id([0; 16]), format!("w_{}", "a".repeat(26)));
    }

    #[test]
    fn parses_ioreg_output() {
        let text = r#"+-o J316sAP  <class IOPlatformExpertDevice, id 0x100000110, registered, matched, active, busy 0 (10 ms), retain 42>
  {
    "IOPlatformSerialNumber" = "ABC123"
    "IOPlatformUUID" = "9F6A8B1C-2D3E-4F50-A1B2-C3D4E5F60718"
    "board-id" = <"Mac-xx">
  }"#;
        assert_eq!(
            parse_quoted_value(text, "IOPlatformUUID").as_deref(),
            Some("9F6A8B1C-2D3E-4F50-A1B2-C3D4E5F60718")
        );
        assert_eq!(parse_quoted_value("nothing here", "IOPlatformUUID"), None);
    }

    #[test]
    fn this_machine_has_a_hash_and_a_name() {
        let h = device_hash();
        assert!(h.starts_with("d_") && h.len() == 28, "{h}");
        assert_eq!(h, device_hash(), "must be stable");
        assert!(!device_name().is_empty());
    }
}
