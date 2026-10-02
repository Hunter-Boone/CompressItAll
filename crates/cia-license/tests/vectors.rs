//! Shared test vectors (`packages/api-types/test-vectors/*.json`), read by this
//! suite and by the Vitest suite in `packages/api-types`. Regenerate with
//!
//! ```text
//! cargo test -p cia-license --all-features --test vectors -- --ignored write_vectors
//! ```

use cia_license::key;
use cia_license::token::{self, TokenError};
use cia_license::validate::{validate_with_keys, LicenseState};
use serde_json::Value;

const PRODUCT_KEYS: &str =
    include_str!("../../../packages/api-types/test-vectors/product-keys.json");
const LICENSE_TOKENS: &str =
    include_str!("../../../packages/api-types/test-vectors/license-tokens.json");

fn product_keys() -> Value {
    serde_json::from_str(PRODUCT_KEYS).unwrap()
}

fn license_tokens() -> Value {
    serde_json::from_str(LICENSE_TOKENS).unwrap()
}

fn hex_to_bytes(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn valid_product_keys_pass() {
    let v = product_keys();
    let valid = v["valid"].as_array().unwrap();
    assert_eq!(valid.len(), 50);
    for k in valid {
        let display = k["display"].as_str().unwrap();
        let normalised = k["normalised"].as_str().unwrap();
        assert_eq!(key::normalise(display).unwrap(), normalised, "{display}");
        assert!(key::is_valid(normalised));
        assert_eq!(key::format_display(normalised), display);
        assert_eq!(key::masked(normalised), k["masked"].as_str().unwrap());
        assert_eq!(key::key4(normalised), k["key4"].as_str().unwrap());
        assert_eq!(hex(&key::hash(normalised)), k["sha256"].as_str().unwrap());
    }
}

#[test]
fn invalid_product_keys_fail() {
    let v = product_keys();
    let invalid = v["invalid"].as_array().unwrap();
    assert_eq!(invalid.len(), 50);
    for k in invalid {
        let display = k["display"].as_str().unwrap();
        assert_eq!(
            key::normalise(display),
            Err(key::KeyError::Typo),
            "{display}"
        );
        assert!(!key::is_valid(&display.replace('-', "")));
    }
}

#[test]
fn raw_inputs_normalise_as_expected() {
    let v = product_keys();
    let cases = v["normalise"].as_array().unwrap();
    assert_eq!(cases.len(), 20);
    for c in cases {
        let input = c["input"].as_str().unwrap();
        match c["normalised"].as_str() {
            Some(expected) => {
                assert_eq!(key::normalise(input).as_deref(), Ok(expected), "{input:?}")
            }
            None => {
                assert_eq!(c["error"], "typo");
                assert_eq!(key::normalise(input), Err(key::KeyError::Typo), "{input:?}");
            }
        }
    }
}

fn test_keys(v: &Value) -> Vec<(String, [u8; 32])> {
    let pk: [u8; 32] = hex_to_bytes(v["public_key_hex"].as_str().unwrap())
        .try_into()
        .unwrap();
    vec![(v["kid"].as_str().unwrap().to_string(), pk)]
}

#[test]
fn signed_tokens_verify_and_roundtrip_their_payload() {
    let v = license_tokens();
    let keys = test_keys(&v);
    let keys: Vec<(&str, [u8; 32])> = keys.iter().map(|(k, p)| (k.as_str(), *p)).collect();
    let tokens = v["tokens"].as_array().unwrap();
    assert_eq!(tokens.len(), 5);
    for t in tokens {
        let token = t["token"].as_str().unwrap();
        let expected: Value = t["payload"].clone();
        match token::verify(token, &keys) {
            Ok(p) => assert_eq!(serde_json::to_value(&p).unwrap(), expected, "{}", t["name"]),
            Err(TokenError::UnknownKid(k)) => assert_eq!(expected["kid"], k, "{}", t["name"]),
            Err(e) => panic!("{}: {e}", t["name"]),
        }
    }
}

#[test]
fn every_case_verifies_and_validates_as_the_file_says() {
    let v = license_tokens();
    let keys = test_keys(&v);
    let keys: Vec<(&str, [u8; 32])> = keys.iter().map(|(k, p)| (k.as_str(), *p)).collect();
    for c in v["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let token = c["token"].as_str().unwrap();
        let got = match token::verify(token, &keys) {
            Ok(_) => "ok".to_string(),
            Err(e) => e.code().to_string(),
        };
        assert_eq!(got, c["verify"].as_str().unwrap(), "verify {name}");

        let mut state = LicenseState {
            last_seen_utc: c["last_seen_utc"].as_i64().unwrap_or(0),
            last_refresh_utc: None,
        };
        let status = validate_with_keys(
            token,
            &keys,
            c["device_hash"].as_str().unwrap(),
            c["now"].as_i64().unwrap(),
            &mut state,
        );
        assert_eq!(
            serde_json::to_value(&status).unwrap(),
            c["status"],
            "status {name}"
        );
    }
}

/// Deterministic generator so regenerated vectors only change when the code does.
#[cfg(all(feature = "sign", feature = "native"))]
struct SplitMix(u64);
#[cfg(all(feature = "sign", feature = "native"))]
impl SplitMix {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn fill(&mut self, buf: &mut [u8]) {
        for chunk in buf.chunks_mut(8) {
            let bytes = self.next().to_le_bytes();
            chunk.copy_from_slice(&bytes[..chunk.len()]);
        }
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

#[cfg(all(feature = "sign", feature = "native"))]
#[test]
#[ignore = "writes packages/api-types/test-vectors/*.json; run on purpose"]
fn write_vectors() {
    use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
    use base64::Engine;
    use cia_license::device::{device_hash_from, web_install_id};
    use cia_license::token::{sign, Kind, Payload, Plan};
    use cia_license::validate::validate_with_keys;
    use serde_json::json;

    let dir = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../packages/api-types/test-vectors"
    );
    let mut rng = SplitMix(0x5EED_2026_1002);

    // ---- product keys ----
    let mut valid = Vec::new();
    let mut normalised_keys = Vec::new();
    for _ in 0..50 {
        let display = key::generate(|b| rng.fill(b));
        let n = key::normalise(&display).unwrap();
        valid.push(json!({
            "display": display,
            "normalised": n,
            "masked": key::masked(&n),
            "key4": key::key4(&n),
            "sha256": hex(&key::hash(&n)),
        }));
        normalised_keys.push(n);
    }
    let mut invalid = Vec::new();
    for i in 0..50 {
        let n = key::normalise(&key::generate(|b| rng.fill(b))).unwrap();
        let pos = if i < 20 { i } else { rng.below(key::KEY_LEN) };
        let mut bytes = n.clone().into_bytes();
        let alt = loop {
            let c = key::CHARSET[rng.below(key::CHARSET.len())];
            if c != bytes[pos] {
                break c;
            }
        };
        bytes[pos] = alt;
        let mutated = String::from_utf8(bytes).unwrap();
        assert!(!key::is_valid(&mutated));
        invalid.push(json!({ "display": key::format_display(&mutated), "from": key::format_display(&n), "position": pos }));
    }
    let k = |i: usize| normalised_keys[i].clone();
    let d = |i: usize| key::format_display(&normalised_keys[i]);
    let raw: Vec<(String, Option<String>)> = vec![
        (d(0).to_lowercase(), Some(k(0))),
        (d(1).replace('-', " "), Some(k(1))),
        (k(2), Some(k(2))),
        (k(3).to_lowercase(), Some(k(3))),
        (format!("  {}  ", d(4)), Some(k(4))),
        (d(5).replace('-', "  -  "), Some(k(5))),
        (
            format!("{}\n", d(6).to_lowercase().replace('-', " ")),
            Some(k(6)),
        ),
        (format!("\t{}\t", k(7).to_lowercase()), Some(k(7))),
        (d(8).replace('-', "--"), Some(k(8))),
        (format!("{}-{}", &k(9)[..10], &k(9)[10..]), Some(k(9))),
        (
            k(10)
                .chars()
                .enumerate()
                .map(|(i, c)| {
                    if i % 2 == 0 {
                        c.to_ascii_lowercase()
                    } else {
                        c
                    }
                })
                .collect(),
            Some(k(10)),
        ),
        (d(11).replace('-', " - "), Some(k(11))),
        // typos
        (format!("0{}", &k(12)[1..]), None),
        (format!("O{}", &k(13)[1..]), None),
        (format!("{}1{}", &k(14)[..5], &k(14)[6..]), None),
        (format!("{}I{}", &k(15)[..5], &k(15)[6..]), None),
        (format!("{}l", &k(16)[..19]), None),
        (k(17)[..19].to_string(), None),
        (format!("{}A", k(18)), None),
        (String::new(), None),
    ];
    let normalise: Vec<Value> = raw
        .into_iter()
        .map(|(input, expected)| match expected {
            Some(n) => json!({ "input": input, "normalised": n }),
            None => json!({ "input": input, "error": "typo" }),
        })
        .collect();
    let product_keys = json!({
        "_comment": "Generated by `cargo test -p cia-license --all-features --test vectors -- --ignored write_vectors`. Read by cia-license tests and packages/api-types tests.",
        "charset": std::str::from_utf8(key::CHARSET).unwrap(),
        "valid": valid,
        "invalid": invalid,
        "normalise": normalise,
    });
    std::fs::write(
        format!("{dir}/product-keys.json"),
        serde_json::to_string_pretty(&product_keys).unwrap() + "\n",
    )
    .unwrap();

    // ---- license tokens ----
    let keys_doc: Value = serde_json::from_str(include_str!("../test-keys.json")).unwrap();
    let test_entry = keys_doc["keys"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["kid"] == "test")
        .unwrap();
    let seed: [u8; 32] = STANDARD
        .decode(test_entry["seed_b64"].as_str().unwrap())
        .unwrap()
        .try_into()
        .unwrap();
    let public = token::public_key(&seed);
    assert_eq!(hex(&public), test_entry["public_key_hex"]);
    let keys: Vec<(&str, [u8; 32])> = vec![("test", public)];

    const DAY: i64 = 86_400;
    let t0: i64 = 1_759_400_000;
    let dev_desktop = device_hash_from("windows", "9F6A8B1C-2D3E-4F50-A1B2-C3D4E5F60718");
    let dev_other = device_hash_from("macos", "1D3B5F7A-9C2E-4B6D-8F0A-2C4E6A8B0D1F");
    let dev_web = web_install_id([0x5a; 16]);

    let base = Payload {
        v: 1,
        kid: String::new(),
        ent: "ent_01K6QW3Z8B4N7P2R9S5T6V7W8X".into(),
        plan: Plan::Lifetime,
        kind: Kind::Desktop,
        dev: dev_desktop.clone(),
        iat: t0,
        exp: t0 + 45 * DAY,
        acc: None,
        key4: key::key4(&k(0)),
    };
    let yearly_web = Payload {
        ent: "ent_01K6QW4A9C5P8Q3S0T6U7V8W9Y".into(),
        plan: Plan::Yearly,
        kind: Kind::Web,
        dev: dev_web.clone(),
        exp: t0 + 14 * DAY,
        acc: Some(t0 + 365 * DAY),
        key4: key::key4(&k(1)),
        ..base.clone()
    };
    let yearly_ended = Payload {
        ent: "ent_01K6QW5B0D6Q9R4T1U7V8W9X0Z".into(),
        plan: Plan::Yearly,
        iat: t0 - 20 * DAY,
        exp: t0 + 25 * DAY,
        acc: Some(t0 - 10 * DAY),
        key4: key::key4(&k(2)),
        ..base.clone()
    };
    let expired = Payload {
        ent: "ent_01K6QW6C1E7R0S5U2V8W9X0Y1A".into(),
        iat: t0 - 60 * DAY,
        exp: t0 - 15 * DAY,
        key4: key::key4(&k(3)),
        ..base.clone()
    };

    let tok_lifetime = sign(&base, &seed, "test");
    let tok_yearly_web = sign(&yearly_web, &seed, "test");
    let tok_ended = sign(&yearly_ended, &seed, "test");
    let tok_expired = sign(&expired, &seed, "test");
    let tok_wrong_kid = sign(&base, &seed, "2099-01");

    let with_kid = |p: &Payload, kid: &str| {
        let mut p = p.clone();
        p.kid = kid.into();
        serde_json::to_value(p).unwrap()
    };
    let tokens = json!([
        { "name": "lifetime desktop", "payload": with_kid(&base, "test"), "token": tok_lifetime },
        { "name": "yearly web", "payload": with_kid(&yearly_web, "test"), "token": tok_yearly_web },
        { "name": "yearly desktop, paid-through in the past", "payload": with_kid(&yearly_ended, "test"), "token": tok_ended },
        { "name": "lifetime desktop, exp in the past", "payload": with_kid(&expired, "test"), "token": tok_expired },
        { "name": "lifetime desktop signed under unknown kid 2099-01", "payload": with_kid(&base, "2099-01"), "token": tok_wrong_kid },
    ]);

    // Tampered payload: swap in the attacker's device hash, keep the signature.
    let tampered_payload = {
        let parts: Vec<&str> = tok_lifetime.split('.').collect();
        let mut p: Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[1]).unwrap()).unwrap();
        p["dev"] = json!(dev_other);
        let b64 = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&p).unwrap());
        format!("SMG1.{b64}.{}", parts[2])
    };
    // Tampered signature: change one character in the middle of the signature.
    let tampered_signature = {
        let parts: Vec<&str> = tok_lifetime.split('.').collect();
        let mut sig: Vec<char> = parts[2].chars().collect();
        sig[10] = if sig[10] == 'A' { 'B' } else { 'A' };
        format!("SMG1.{}.{}", parts[1], sig.into_iter().collect::<String>())
    };

    struct Case<'a> {
        name: &'a str,
        token: &'a str,
        device_hash: &'a str,
        now: i64,
        last_seen_utc: Option<i64>,
        extra: Value,
    }
    let seen = t0 + 10 * DAY;
    let acc_ended = yearly_ended.acc.unwrap();
    let cases = [
        Case {
            name: "valid lifetime desktop",
            token: &tok_lifetime,
            device_hash: &dev_desktop,
            now: t0 + DAY,
            last_seen_utc: None,
            extra: json!({}),
        },
        Case {
            name: "valid yearly web",
            token: &tok_yearly_web,
            device_hash: &dev_web,
            now: t0 + DAY,
            last_seen_utc: None,
            extra: json!({}),
        },
        Case {
            name: "tampered payload",
            token: &tampered_payload,
            device_hash: &dev_other,
            now: t0 + DAY,
            last_seen_utc: None,
            extra: json!({}),
        },
        Case {
            name: "tampered signature",
            token: &tampered_signature,
            device_hash: &dev_desktop,
            now: t0 + DAY,
            last_seen_utc: None,
            extra: json!({}),
        },
        Case {
            name: "malformed",
            token: "SMG1.not-a-token",
            device_hash: &dev_desktop,
            now: t0 + DAY,
            last_seen_utc: None,
            extra: json!({}),
        },
        Case {
            name: "wrong kid",
            token: &tok_wrong_kid,
            device_hash: &dev_desktop,
            now: t0 + DAY,
            last_seen_utc: None,
            extra: json!({}),
        },
        Case {
            name: "wrong device",
            token: &tok_lifetime,
            device_hash: &dev_other,
            now: t0 + DAY,
            last_seen_utc: None,
            extra: json!({}),
        },
        Case {
            name: "rolled-back clock",
            token: &tok_lifetime,
            device_hash: &dev_desktop,
            now: seen - 48 * 3600 - 1,
            last_seen_utc: Some(seen),
            extra: json!({}),
        },
        Case {
            name: "expired",
            token: &tok_expired,
            device_hash: &dev_desktop,
            now: t0,
            last_seen_utc: None,
            extra: json!({}),
        },
        Case {
            name: "ended yearly",
            token: &tok_ended,
            device_hash: &dev_desktop,
            now: t0,
            last_seen_utc: None,
            extra: json!({ "acc": acc_ended }),
        },
        Case {
            name: "yearly inside the 3-day grace",
            token: &tok_ended,
            device_hash: &dev_desktop,
            now: acc_ended + 3 * DAY,
            last_seen_utc: None,
            extra: json!({ "acc": acc_ended }),
        },
    ];
    let cases: Vec<Value> = cases
        .iter()
        .map(|c| {
            let verify = match token::verify(c.token, &keys) {
                Ok(_) => "ok".to_string(),
                Err(e) => e.code().to_string(),
            };
            let mut state = LicenseState {
                last_seen_utc: c.last_seen_utc.unwrap_or(0),
                last_refresh_utc: None,
            };
            let status = validate_with_keys(c.token, &keys, c.device_hash, c.now, &mut state);
            let mut v = json!({
                "name": c.name,
                "token": c.token,
                "device_hash": c.device_hash,
                "now": c.now,
                "verify": verify,
                "status": status,
            });
            if let Some(s) = c.last_seen_utc {
                v["last_seen_utc"] = json!(s);
            }
            for (k, val) in c.extra.as_object().unwrap() {
                v[k] = val.clone();
            }
            v
        })
        .collect();

    let doc = json!({
        "_comment": "Generated by `cargo test -p cia-license --all-features --test vectors -- --ignored write_vectors`. Tokens are signed with the 'test' key from crates/cia-license/test-keys.json. `verify` is the token::verify outcome (ok | malformed | unsupported_version | unknown_kid | bad_signature); `status` is validate() with the given device_hash, now and last_seen_utc (default 0).",
        "kid": "test",
        "public_key_hex": hex(&public),
        "tokens": tokens,
        "cases": cases,
    });
    std::fs::write(
        format!("{dir}/license-tokens.json"),
        serde_json::to_string_pretty(&doc).unwrap() + "\n",
    )
    .unwrap();
}
