//! License tokens: `SMG1.<payload_b64url>.<signature_b64url>` (DESIGN.md 5.5).
//!
//! The signature is Ed25519 over the ASCII bytes of `"SMG1." + payload_b64url`,
//! so verifiers never re-serialise the payload. Base64 is URL-safe without
//! padding. The TypeScript twin is `packages/api-types/src/license-token.ts`.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};

/// Token prefix and version marker.
pub const PREFIX: &str = "SMG1";
/// `payload.v` this crate understands.
pub const VERSION: u8 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Plan {
    Lifetime,
    Yearly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Desktop,
    Web,
}

/// Token payload. Field order is the wire order (DESIGN.md 5.5); serde keeps
/// declaration order, so `serde_json::to_vec` produces the canonical bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Payload {
    pub v: u8,
    /// Which public key verifies the token.
    pub kid: String,
    /// Entitlement id (`ent_` + ULID).
    pub ent: String,
    pub plan: Plan,
    pub kind: Kind,
    /// Desktop device hash (`d_…`) or web install id (`w_…`).
    pub dev: String,
    /// Issued at, unix seconds.
    pub iat: i64,
    /// Offline validity end, unix seconds: iat + 45 days (desktop), + 14 days (web).
    pub exp: i64,
    /// Paid-through time for yearly plans; `None` (JSON `null`) for lifetime.
    pub acc: Option<i64>,
    /// Last five characters of the product key.
    pub key4: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TokenError {
    /// Wrong shape: not three parts, wrong prefix, bad base64 or JSON, or a
    /// signature that is not 64 bytes.
    #[error("malformed license token")]
    Malformed,
    /// `payload.v` is not [`VERSION`].
    #[error("unsupported license token version {0}")]
    UnsupportedVersion(u8),
    /// No public key for `payload.kid` in the list given to [`verify`].
    #[error("unknown signing key {0:?}")]
    UnknownKid(String),
    /// The signature does not verify under the key for `payload.kid`.
    #[error("license token signature does not verify")]
    BadSignature,
}

impl TokenError {
    /// Stable snake_case code, shared with the TypeScript tests and useful in logs.
    pub fn code(&self) -> &'static str {
        match self {
            TokenError::Malformed => "malformed",
            TokenError::UnsupportedVersion(_) => "unsupported_version",
            TokenError::UnknownKid(_) => "unknown_kid",
            TokenError::BadSignature => "bad_signature",
        }
    }
}

/// Decode and check a token against `keys` (`(kid, public_key)` pairs).
/// Returns the payload; the caller still applies the device, clock and
/// expiry rules (see [`crate::validate`]).
pub fn verify(token: &str, keys: &[(&str, [u8; 32])]) -> Result<Payload, TokenError> {
    let mut parts = token.split('.');
    let (Some(prefix), Some(payload_b64), Some(sig_b64), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(TokenError::Malformed);
    };
    if prefix != PREFIX {
        return Err(TokenError::Malformed);
    }
    let payload_bytes = URL_SAFE_NO_PAD
        .decode(payload_b64)
        .map_err(|_| TokenError::Malformed)?;
    let sig_bytes = URL_SAFE_NO_PAD
        .decode(sig_b64)
        .map_err(|_| TokenError::Malformed)?;
    let signature = Signature::from_slice(&sig_bytes).map_err(|_| TokenError::Malformed)?;

    let payload: Payload =
        serde_json::from_slice(&payload_bytes).map_err(|_| TokenError::Malformed)?;
    if payload.v != VERSION {
        return Err(TokenError::UnsupportedVersion(payload.v));
    }
    let key = keys
        .iter()
        .find(|(kid, _)| *kid == payload.kid)
        .map(|(_, key)| key)
        .ok_or_else(|| TokenError::UnknownKid(payload.kid.clone()))?;
    let verifying_key = VerifyingKey::from_bytes(key).map_err(|_| TokenError::BadSignature)?;

    let message = signing_input(payload_b64);
    verifying_key
        .verify_strict(message.as_bytes(), &signature)
        .map_err(|_| TokenError::BadSignature)?;
    Ok(payload)
}

/// The bytes that are signed: `"SMG1." + payload_b64url`.
pub fn signing_input(payload_b64: &str) -> String {
    format!("{PREFIX}.{payload_b64}")
}

/// Ed25519 public key for a 32-byte seed. Used by `cia-keygen` and by the
/// test that checks `keys.rs` against `test-keys.json`.
pub fn public_key(seed: &[u8; 32]) -> [u8; 32] {
    ed25519_dalek::SigningKey::from_bytes(seed)
        .verifying_key()
        .to_bytes()
}

/// Sign `payload` with `seed`, stamping `kid` into the payload first.
/// Server-side and test-vector use only (feature `sign`).
#[cfg(feature = "sign")]
pub fn sign(payload: &Payload, seed: &[u8; 32], kid: &str) -> String {
    use ed25519_dalek::Signer;
    let mut payload = payload.clone();
    payload.kid = kid.to_string();
    let json = serde_json::to_vec(&payload).expect("payload serialises");
    let payload_b64 = URL_SAFE_NO_PAD.encode(json);
    let message = signing_input(&payload_b64);
    let signature = ed25519_dalek::SigningKey::from_bytes(seed).sign(message.as_bytes());
    format!("{message}.{}", URL_SAFE_NO_PAD.encode(signature.to_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_serialises_in_wire_order_with_null_acc() {
        let p = Payload {
            v: 1,
            kid: "2026-10".into(),
            ent: "ent_01K6QW".into(),
            plan: Plan::Lifetime,
            kind: Kind::Desktop,
            dev: "d_k3j".into(),
            iat: 1_759_400_000,
            exp: 1_763_288_000,
            acc: None,
            key4: "7KQ2P".into(),
        };
        assert_eq!(
            serde_json::to_string(&p).unwrap(),
            r#"{"v":1,"kid":"2026-10","ent":"ent_01K6QW","plan":"lifetime","kind":"desktop","dev":"d_k3j","iat":1759400000,"exp":1763288000,"acc":null,"key4":"7KQ2P"}"#
        );
        let yearly = Payload {
            plan: Plan::Yearly,
            kind: Kind::Web,
            acc: Some(1_790_936_000),
            ..p
        };
        let s = serde_json::to_string(&yearly).unwrap();
        assert!(s.contains(r#""plan":"yearly","kind":"web""#));
        assert!(s.contains(r#""acc":1790936000"#));
        assert_eq!(serde_json::from_str::<Payload>(&s).unwrap(), yearly);
    }

    #[test]
    fn shape_errors_are_malformed() {
        let keys: &[(&str, [u8; 32])] = &[("k", [0; 32])];
        for t in [
            "",
            "SMG1",
            "SMG1.abc",
            "SMG1.abc.def.ghi",
            "SMG2.e30.AAAA",
            "SMG1.!!.AAAA",
            "SMG1.e30.AAAA", // signature not 64 bytes
        ] {
            assert_eq!(verify(t, keys), Err(TokenError::Malformed), "{t:?}");
        }
    }
}
