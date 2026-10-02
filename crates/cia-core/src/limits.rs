//! Raw budgets from preset limits (DESIGN.md 3.11).

/// How the destination counts bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Counts {
    Raw,
    MimeBase64,
}

/// Bytes of attachment data that fit in `limit_bytes` of message, minus safety.
///
/// `MimeBase64`: base64 is 4/3 larger, plus CRLF every 76 characters; 100 kB is
/// reserved for headers and body. Gmail's 25 MB gives exactly 18,196,153.
pub fn raw_budget(limit_bytes: u64, counts: Counts, safety_bytes: u64) -> u64 {
    let raw = match counts {
        Counts::Raw => limit_bytes,
        Counts::MimeBase64 => {
            let usable = limit_bytes.saturating_sub(100_000) as f64;
            (usable * 3.0 / 4.0 * 76.0 / 78.0).floor() as u64
        }
    };
    raw.saturating_sub(safety_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gmail_is_exact() {
        assert_eq!(raw_budget(25_000_000, Counts::MimeBase64, 0), 18_196_153);
    }
    #[test]
    fn raw_minus_safety() {
        assert_eq!(raw_budget(20_971_520, Counts::Raw, 65_536), 20_905_984);
        assert_eq!(raw_budget(10, Counts::Raw, 100), 0);
    }
}
