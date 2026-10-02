//! Product keys: `XXXXX-XXXXX-XXXXX-XXXXX` over a 31-character alphabet
//! (DESIGN.md 5.4). The TypeScript twin is `packages/api-types/src/product-key.ts`;
//! `packages/api-types/test-vectors/product-keys.json` keeps them identical.
//!
//! Checksum: the 20th character is a Luhn mod N check character with N = 31,
//! weights 2,1,2,1,... from the right, and the sum reduced modulo 31. The
//! textbook Luhn mod N "digit fold" (`a / N + a % N`) is not used because it is
//! only a bijection for even N: with N = 31 it maps both 1 and 16 to 2, so a
//! B/T swap in a doubled position would slip through. Reducing mod 31 (a prime)
//! keeps both weights invertible, so every single-character substitution and
//! every adjacent transposition is caught.

use sha2::{Digest, Sha256};

/// The 31 allowed characters. No 0, O, 1, I, L.
pub const CHARSET: &[u8; 31] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";
/// Characters in a normalised key: 19 random + 1 check.
pub const KEY_LEN: usize = 20;
/// Random characters before the check character.
pub const BODY_LEN: usize = 19;
/// Bytes at or above this are rejected when sampling (248 = 8 × 31), so
/// `byte % 31` is uniform.
pub const REJECT_FROM: u8 = 248;
/// Characters [`normalise`] drops before checking: ASCII whitespace, no-break
/// space (pasted from emails) and dashes. Listed explicitly because Rust's
/// `is_whitespace` and JavaScript's `\s` disagree at the edges.
pub const IGNORED: &[char] = &[' ', '\t', '\n', '\r', '\u{0B}', '\u{0C}', '\u{A0}', '-'];

const N: u32 = CHARSET.len() as u32;

/// The only way a key can be wrong offline. The server decides whether a
/// well-formed key exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum KeyError {
    #[error("That key has a typo.")]
    Typo,
}

fn code_of(c: u8) -> Option<u32> {
    CHARSET.iter().position(|&x| x == c).map(|p| p as u32)
}

/// Weighted sum from the right. `first_factor` is the weight of the rightmost
/// character: 2 when summing the 19-character body, 1 when summing a full key
/// whose last character is the check character.
fn weighted_sum(codes: &[u32], first_factor: u32) -> u32 {
    let mut factor = first_factor;
    let mut sum = 0u32;
    for &c in codes.iter().rev() {
        sum = (sum + factor * c) % N;
        factor = if factor == 2 { 1 } else { 2 };
    }
    sum
}

fn codes(s: &[u8]) -> Option<Vec<u32>> {
    s.iter().map(|&c| code_of(c)).collect()
}

/// Check character for a 19-character body. Panics if the body has the wrong
/// length or a character outside [`CHARSET`]; callers build the body themselves.
pub fn check_char(body: &[u8]) -> u8 {
    assert_eq!(
        body.len(),
        BODY_LEN,
        "key body must be {BODY_LEN} characters"
    );
    let codes = codes(body).expect("key body uses the key charset");
    let sum = weighted_sum(&codes, 2);
    CHARSET[((N - sum) % N) as usize]
}

/// Generate a new key in display form (`XXXXX-XXXXX-XXXXX-XXXXX`).
///
/// `rng` fills a buffer with random bytes (`getrandom::fill`, a CSPRNG on the
/// server, a seeded generator in tests). Bytes >= 248 are rejected so each
/// character is uniform over the 31-character alphabet.
pub fn generate(mut rng: impl FnMut(&mut [u8])) -> String {
    let mut body: Vec<u8> = Vec::with_capacity(KEY_LEN);
    let mut buf = [0u8; 32];
    while body.len() < BODY_LEN {
        rng(&mut buf);
        for &b in buf.iter() {
            if body.len() == BODY_LEN {
                break;
            }
            if b < REJECT_FROM {
                body.push(CHARSET[(b % N as u8) as usize]);
            }
        }
    }
    let check = check_char(&body);
    body.push(check);
    let normalised = String::from_utf8(body).expect("charset is ASCII");
    format_display(&normalised)
}

/// Uppercase, strip [`IGNORED`] characters, then check charset, length and
/// checksum. Anything wrong is a [`KeyError::Typo`]; the returned string is
/// the 20-character normalised key.
pub fn normalise(input: &str) -> Result<String, KeyError> {
    let mut out = String::with_capacity(KEY_LEN);
    for ch in input.chars() {
        if IGNORED.contains(&ch) {
            continue;
        }
        if !ch.is_ascii() {
            return Err(KeyError::Typo);
        }
        let up = ch.to_ascii_uppercase() as u8;
        if code_of(up).is_none() {
            return Err(KeyError::Typo);
        }
        out.push(up as char);
    }
    if is_valid(&out) {
        Ok(out)
    } else {
        Err(KeyError::Typo)
    }
}

/// True when `normalised` is exactly 20 charset characters with a correct
/// check character. Does not normalise; use [`normalise`] for user input.
pub fn is_valid(normalised: &str) -> bool {
    let bytes = normalised.as_bytes();
    if bytes.len() != KEY_LEN {
        return false;
    }
    match codes(bytes) {
        Some(codes) => weighted_sum(&codes, 1) == 0,
        None => false,
    }
}

/// `ABCDE-FGHJK-MNPQR-STUVW` from a normalised key.
pub fn format_display(normalised: &str) -> String {
    normalised
        .as_bytes()
        .chunks(5)
        .map(|c| std::str::from_utf8(c).expect("ASCII"))
        .collect::<Vec<_>>()
        .join("-")
}

/// `ABCDE-…-STUVW`: first and last group visible, for emails and the dashboard.
pub fn masked(normalised: &str) -> String {
    let n = normalised.len();
    if n < 10 {
        return "…".to_string();
    }
    format!("{}-…-{}", &normalised[..5], &normalised[n - 5..])
}

/// The last five characters, stored as `key4` for display and support.
pub fn key4(normalised: &str) -> String {
    let n = normalised.len();
    normalised[n.saturating_sub(5)..].to_string()
}

/// `sha256(normalised)`, the database lookup key. The full key is never stored
/// in clear and never logged.
pub fn hash(normalised: &str) -> [u8; 32] {
    Sha256::digest(normalised.as_bytes()).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// splitmix64: deterministic, good enough to measure our own sampler.
    struct SplitMix(u64);
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
    }

    fn gen_norm(rng: &mut SplitMix) -> String {
        normalise(&generate(|b| rng.fill(b))).unwrap()
    }

    #[test]
    fn generated_keys_have_the_right_shape() {
        let mut rng = SplitMix(1);
        for _ in 0..100 {
            let display = generate(|b| rng.fill(b));
            assert_eq!(display.len(), 23);
            assert_eq!(
                display.split('-').map(str::len).collect::<Vec<_>>(),
                [5, 5, 5, 5]
            );
            let norm = normalise(&display).unwrap();
            assert!(is_valid(&norm));
            assert_eq!(format_display(&norm), display);
        }
    }

    /// DESIGN 7.1: character distribution over 100,000 keys. With 1.9 million
    /// samples the standard deviation per cell is about 0.4 percent of the
    /// expectation; 5 percent is a 12-sigma bound, so this never flakes.
    #[test]
    fn random_characters_are_uniform() {
        let mut rng = SplitMix(0xC0FFEE);
        let mut counts: HashMap<u8, u64> = HashMap::new();
        const KEYS: usize = 100_000;
        for _ in 0..KEYS {
            let k = gen_norm(&mut rng);
            for &c in &k.as_bytes()[..BODY_LEN] {
                *counts.entry(c).or_default() += 1;
            }
        }
        let expected = (KEYS * BODY_LEN) as f64 / CHARSET.len() as f64;
        let mut chi_square = 0.0;
        for &c in CHARSET.iter() {
            let observed = *counts.get(&c).unwrap_or(&0) as f64;
            let deviation = (observed - expected).abs() / expected;
            assert!(
                deviation <= 0.05,
                "{} appears {observed} times, expected {expected:.0} (±{:.1}%)",
                c as char,
                deviation * 100.0
            );
            chi_square += (observed - expected).powi(2) / expected;
        }
        // 30 degrees of freedom; the 99.9th percentile is 59.7.
        assert!(
            chi_square < 59.7,
            "chi-square {chi_square:.1} too high for 30 dof"
        );
        assert_eq!(counts.len(), CHARSET.len(), "every character must occur");
    }

    /// DESIGN 5.4: the check character catches every single-character typo.
    /// 200 keys × 19 positions × 30 alternatives, all must fail.
    #[test]
    fn checksum_catches_every_single_substitution() {
        let mut rng = SplitMix(42);
        let mut checked = 0usize;
        for _ in 0..200 {
            let key = gen_norm(&mut rng);
            let bytes = key.as_bytes();
            for pos in 0..BODY_LEN {
                for &alt in CHARSET.iter() {
                    if alt == bytes[pos] {
                        continue;
                    }
                    let mut mutated = bytes.to_vec();
                    mutated[pos] = alt;
                    let mutated = String::from_utf8(mutated).unwrap();
                    assert!(
                        !is_valid(&mutated),
                        "{key} -> {mutated} passed the checksum"
                    );
                    checked += 1;
                }
            }
            // The check character itself, too.
            for &alt in CHARSET.iter() {
                if alt != bytes[BODY_LEN] {
                    let mut mutated = bytes.to_vec();
                    mutated[BODY_LEN] = alt;
                    assert!(!is_valid(std::str::from_utf8(&mutated).unwrap()));
                }
            }
        }
        assert_eq!(checked, 200 * BODY_LEN * 30);
    }

    #[test]
    fn checksum_catches_adjacent_transpositions() {
        let mut rng = SplitMix(7);
        for _ in 0..200 {
            let key = gen_norm(&mut rng);
            let bytes = key.as_bytes();
            for pos in 0..KEY_LEN - 1 {
                if bytes[pos] == bytes[pos + 1] {
                    continue;
                }
                let mut swapped = bytes.to_vec();
                swapped.swap(pos, pos + 1);
                assert!(
                    !is_valid(std::str::from_utf8(&swapped).unwrap()),
                    "{key} swap at {pos}"
                );
            }
        }
    }

    #[test]
    fn normalise_accepts_messy_input_and_rejects_confusables() {
        let mut rng = SplitMix(3);
        let key = gen_norm(&mut rng);
        let display = format_display(&key);
        assert_eq!(normalise(&display).unwrap(), key);
        assert_eq!(normalise(&display.to_lowercase()).unwrap(), key);
        assert_eq!(normalise(&display.replace('-', " ")).unwrap(), key);
        assert_eq!(
            normalise(&format!("  {}\t\n", display.replace('-', ""))).unwrap(),
            key
        );

        for bad in ["0", "O", "1", "I", "L", "o", "i", "l"] {
            let mut s = key.clone();
            s.replace_range(0..1, bad);
            assert_eq!(normalise(&s), Err(KeyError::Typo), "{bad} must be a typo");
        }
        assert_eq!(normalise(&key[..19]), Err(KeyError::Typo));
        assert_eq!(normalise(&format!("{key}A")), Err(KeyError::Typo));
        assert_eq!(normalise(""), Err(KeyError::Typo));
        assert_eq!(normalise("ABCDE-FGHJK-MNPQR-STUVÜ"), Err(KeyError::Typo));
        assert_eq!(normalise(&format!("{key}.")), Err(KeyError::Typo));
    }

    #[test]
    fn display_helpers() {
        let key = "ABCDEFGHJKMNPQRSTUVW";
        assert_eq!(format_display(key), "ABCDE-FGHJK-MNPQR-STUVW");
        assert_eq!(masked(key), "ABCDE-…-STUVW");
        assert_eq!(key4(key), "STUVW");
        assert_eq!(
            hash(key),
            <[u8; 32]>::from(Sha256::digest(b"ABCDEFGHJKMNPQRSTUVW"))
        );
    }

    #[test]
    fn rejection_sampling_skips_high_bytes() {
        // A generator that only ever yields 255 would spin forever without
        // rejection; one that alternates 255 and 0 must give all-A bodies.
        let mut toggle = false;
        let display = generate(|buf| {
            for b in buf.iter_mut() {
                *b = if toggle { 255 } else { 0 };
                toggle = !toggle;
            }
        });
        let norm = normalise(&display).unwrap();
        assert_eq!(&norm[..BODY_LEN], "AAAAAAAAAAAAAAAAAAA");
    }
}
