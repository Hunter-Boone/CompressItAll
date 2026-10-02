//! Offline validation (DESIGN.md 5.5), identical on desktop and web.
//!
//! ```text
//! 1. Split on ".", require prefix "SMG1", decode both parts.
//! 2. Find the public key for payload.kid; unknown kid -> Invalid.
//! 3. Verify the signature; failure -> Invalid.
//! 4. payload.dev != device_hash -> Invalid
//! 5. now < state.last_seen_utc - 48 h -> NeedsOnline (clock rolled back)
//! 6. now > payload.exp -> NeedsOnline
//! 7. payload.acc != null and now > payload.acc + 3 days -> Ended
//! 8. otherwise Valid { plan, key4, acc }.
//! state.last_seen_utc = max(state.last_seen_utc, now) after every check.
//! ```
//!
//! Every non-Valid outcome falls back to the free tier with a banner; nothing
//! is ever locked beyond that.

use serde::{Deserialize, Serialize};

use crate::keys;
use crate::token::{self, Plan, TokenError};

/// Clock tolerance before a backwards jump counts as tampering.
pub const CLOCK_ROLLBACK_TOLERANCE_SECS: i64 = 48 * 60 * 60;
/// Grace after a yearly plan's paid-through time.
pub const ACCESS_GRACE_SECS: i64 = 3 * 24 * 60 * 60;

/// User-facing reasons. Wording is DESIGN.md 5.5; the UI shows them verbatim.
pub mod reason {
    pub const OTHER_COMPUTER: &str = "This license belongs to another computer.";
    pub const CLOCK: &str = "Your computer's clock looks wrong.";
    pub const NEEDS_CHECK: &str =
        "Smidge needs to check your license. Connect to the internet once.";
    pub const NOT_VALID: &str = "This license isn't valid.";
    pub const NEWER_KEY: &str =
        "This license was issued for a newer version of Smidge. Update Smidge to use it.";
}

/// Persisted next to the token (`<app_data>/state.json` on desktop, IndexedDB
/// on web). `last_seen_utc` is the largest clock value ever observed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LicenseState {
    pub last_seen_utc: i64,
    /// Last successful `POST /license/refresh`, unix seconds.
    #[serde(default)]
    pub last_refresh_utc: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum LicenseStatus {
    /// Token unusable on this computer. The app behaves as unlicensed.
    Invalid { reason: String },
    /// Token looks fine but must be refreshed before it counts.
    NeedsOnline { reason: String },
    /// Yearly plan's paid-through time (`at`, unix seconds) plus grace has passed.
    Ended { at: i64 },
    Valid {
        plan: Plan,
        key4: String,
        acc: Option<i64>,
    },
}

/// Validate against the built-in [`keys::PUBLIC_KEYS`].
pub fn validate(
    token: &str,
    device_hash: &str,
    now_unix: i64,
    state: &mut LicenseState,
) -> LicenseStatus {
    validate_with_keys(token, keys::PUBLIC_KEYS, device_hash, now_unix, state)
}

/// Same as [`validate`] with an explicit key list (tests, e2e, tooling).
pub fn validate_with_keys(
    token: &str,
    public_keys: &[(&str, [u8; 32])],
    device_hash: &str,
    now_unix: i64,
    state: &mut LicenseState,
) -> LicenseStatus {
    let status = check(token, public_keys, device_hash, now_unix, state);
    state.last_seen_utc = state.last_seen_utc.max(now_unix);
    status
}

fn check(
    token: &str,
    public_keys: &[(&str, [u8; 32])],
    device_hash: &str,
    now: i64,
    state: &LicenseState,
) -> LicenseStatus {
    // Steps 1-3.
    let payload = match token::verify(token, public_keys) {
        Ok(p) => p,
        Err(e) => {
            return LicenseStatus::Invalid {
                reason: invalid_reason(&e).to_string(),
            }
        }
    };
    // Step 4.
    if payload.dev != device_hash {
        return LicenseStatus::Invalid {
            reason: reason::OTHER_COMPUTER.to_string(),
        };
    }
    // Step 5.
    if now < state.last_seen_utc - CLOCK_ROLLBACK_TOLERANCE_SECS {
        return LicenseStatus::NeedsOnline {
            reason: reason::CLOCK.to_string(),
        };
    }
    // Step 6.
    if now > payload.exp {
        return LicenseStatus::NeedsOnline {
            reason: reason::NEEDS_CHECK.to_string(),
        };
    }
    // Step 7.
    if let Some(acc) = payload.acc {
        if now > acc + ACCESS_GRACE_SECS {
            return LicenseStatus::Ended { at: acc };
        }
    }
    // Step 8.
    LicenseStatus::Valid {
        plan: payload.plan,
        key4: payload.key4,
        acc: payload.acc,
    }
}

fn invalid_reason(e: &TokenError) -> &'static str {
    match e {
        TokenError::UnknownKid(_) | TokenError::UnsupportedVersion(_) => reason::NEWER_KEY,
        TokenError::Malformed | TokenError::BadSignature => reason::NOT_VALID,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::{TEST_KID, TEST_PUBLIC_KEY};
    use serde_json::Value;

    const VECTORS: &str =
        include_str!("../../../packages/api-types/test-vectors/license-tokens.json");

    fn vectors() -> Value {
        serde_json::from_str(VECTORS).expect("license-tokens.json parses")
    }

    fn case(name: &str) -> Value {
        vectors()["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == name)
            .unwrap_or_else(|| panic!("no vector case {name:?}"))
            .clone()
    }

    fn run(c: &Value) -> (LicenseStatus, LicenseState) {
        let mut state = LicenseState {
            last_seen_utc: c["last_seen_utc"].as_i64().unwrap_or(0),
            last_refresh_utc: None,
        };
        let status = validate(
            c["token"].as_str().unwrap(),
            c["device_hash"].as_str().unwrap(),
            c["now"].as_i64().unwrap(),
            &mut state,
        );
        (status, state)
    }

    #[test]
    fn test_key_is_in_the_builtin_list_under_test() {
        assert_eq!(keys::find(TEST_KID), Some(&TEST_PUBLIC_KEY));
        let v = vectors();
        assert_eq!(v["kid"], TEST_KID);
        assert_eq!(v["public_key_hex"].as_str().unwrap(), hex(&TEST_PUBLIC_KEY));
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Every case in the shared vector file, compared as JSON so the TS suite
    /// and this one read the same expectation.
    #[test]
    fn every_shared_case_matches() {
        let v = vectors();
        let cases = v["cases"].as_array().unwrap();
        assert!(cases.len() >= 9);
        for c in cases {
            let (status, state) = run(c);
            let got = serde_json::to_value(&status).unwrap();
            assert_eq!(got, c["status"], "case {}", c["name"]);
            let now = c["now"].as_i64().unwrap();
            let before = c["last_seen_utc"].as_i64().unwrap_or(0);
            assert_eq!(
                state.last_seen_utc,
                before.max(now),
                "last_seen after {}",
                c["name"]
            );
        }
    }

    #[test]
    fn valid_lifetime_has_null_acc() {
        let (status, _) = run(&case("valid lifetime desktop"));
        match status {
            LicenseStatus::Valid { plan, acc, key4 } => {
                assert_eq!(plan, Plan::Lifetime);
                assert_eq!(acc, None);
                assert_eq!(key4.len(), 5);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn tampered_payload_and_signature_are_invalid() {
        for name in ["tampered payload", "tampered signature"] {
            let (status, _) = run(&case(name));
            assert_eq!(
                status,
                LicenseStatus::Invalid {
                    reason: reason::NOT_VALID.into()
                },
                "{name}"
            );
        }
    }

    #[test]
    fn wrong_kid_is_invalid() {
        let (status, _) = run(&case("wrong kid"));
        assert_eq!(
            status,
            LicenseStatus::Invalid {
                reason: reason::NEWER_KEY.into()
            }
        );
    }

    #[test]
    fn wrong_device_is_invalid() {
        let (status, _) = run(&case("wrong device"));
        assert_eq!(
            status,
            LicenseStatus::Invalid {
                reason: reason::OTHER_COMPUTER.into()
            }
        );
    }

    #[test]
    fn expired_needs_online() {
        let (status, _) = run(&case("expired"));
        assert_eq!(
            status,
            LicenseStatus::NeedsOnline {
                reason: reason::NEEDS_CHECK.into()
            }
        );
    }

    #[test]
    fn rolled_back_clock_needs_online_and_keeps_last_seen() {
        let c = case("rolled-back clock");
        let (status, state) = run(&c);
        assert_eq!(
            status,
            LicenseStatus::NeedsOnline {
                reason: reason::CLOCK.into()
            }
        );
        assert_eq!(state.last_seen_utc, c["last_seen_utc"].as_i64().unwrap());

        // Exactly 48 h back is still fine; one second more is not.
        let token = c["token"].as_str().unwrap();
        let dev = c["device_hash"].as_str().unwrap();
        let seen = c["last_seen_utc"].as_i64().unwrap();
        let mut st = LicenseState {
            last_seen_utc: seen,
            ..Default::default()
        };
        assert!(matches!(
            validate(token, dev, seen - CLOCK_ROLLBACK_TOLERANCE_SECS, &mut st),
            LicenseStatus::Valid { .. }
        ));
        assert!(matches!(
            validate(
                token,
                dev,
                seen - CLOCK_ROLLBACK_TOLERANCE_SECS - 1,
                &mut st
            ),
            LicenseStatus::NeedsOnline { .. }
        ));
    }

    #[test]
    fn ended_yearly_reports_paid_through_time() {
        let c = case("ended yearly");
        let (status, _) = run(&c);
        let acc = c["acc"].as_i64().unwrap();
        assert_eq!(status, LicenseStatus::Ended { at: acc });

        // Inside the 3-day grace the plan is still Valid.
        let token = c["token"].as_str().unwrap();
        let dev = c["device_hash"].as_str().unwrap();
        let mut st = LicenseState::default();
        assert!(matches!(
            validate(token, dev, acc + ACCESS_GRACE_SECS, &mut st),
            LicenseStatus::Valid { plan: Plan::Yearly, acc: Some(a), .. } if a == acc
        ));
        assert!(matches!(
            validate(token, dev, acc + ACCESS_GRACE_SECS + 1, &mut st),
            LicenseStatus::Ended { at } if at == acc
        ));
    }

    #[test]
    fn device_check_comes_before_clock_and_expiry() {
        let c = case("wrong device");
        let mut st = LicenseState {
            last_seen_utc: i64::MAX / 2,
            ..Default::default()
        };
        let status = validate(
            c["token"].as_str().unwrap(),
            c["device_hash"].as_str().unwrap(),
            0,
            &mut st,
        );
        assert!(matches!(status, LicenseStatus::Invalid { .. }));
        assert_eq!(st.last_seen_utc, i64::MAX / 2);
    }

    #[test]
    fn status_serialises_with_a_status_tag() {
        let s = LicenseStatus::Valid {
            plan: Plan::Yearly,
            key4: "7KQ2P".into(),
            acc: Some(5),
        };
        assert_eq!(
            serde_json::to_string(&s).unwrap(),
            r#"{"status":"valid","plan":"yearly","key4":"7KQ2P","acc":5}"#
        );
        assert_eq!(
            serde_json::to_string(&LicenseStatus::Ended { at: 9 }).unwrap(),
            r#"{"status":"ended","at":9}"#
        );
    }
}
