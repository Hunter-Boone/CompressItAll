//! The audio verification predicate (DESIGN.md 3.11): full decode, duration
//! within 100 ms, channel count as planned, size at most `hard_bytes`.

use crate::{decode, AudioError};

pub const DURATION_TOLERANCE_MS: u64 = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyReport {
    /// All checks passed. Only then may the item be reported `Fitted`.
    pub ok: bool,
    pub size_bytes: u64,
    pub size_ok: bool,
    /// The whole file decoded without a fatal error.
    pub decodes: bool,
    pub duration_ms: Option<u64>,
    pub duration_ok: bool,
    pub channels: Option<u16>,
    pub channels_ok: bool,
    /// Human-readable reasons for each failed check.
    pub problems: Vec<String>,
}

/// Decode `output` completely and compare against the plan.
pub fn verify(
    output: &[u8],
    expected_duration_ms: u64,
    expected_channels: u16,
    hard_bytes: Option<u64>,
) -> VerifyReport {
    let size_bytes = output.len() as u64;
    let mut problems = Vec::new();
    let size_ok = hard_bytes.is_none_or(|h| size_bytes <= h);
    if !size_ok {
        problems.push(format!(
            "output is {size_bytes} bytes, over the limit of {} bytes",
            hard_bytes.unwrap_or(0)
        ));
    }
    let (decodes, duration_ms, channels) = match decode(output, None) {
        Ok(d) => (true, Some(d.duration_ms), Some(d.channels)),
        Err(AudioError::Unsupported(msg)) => {
            problems.push(format!("cannot verify: {msg}"));
            (false, None, None)
        }
        Err(e) => {
            problems.push(format!("output does not decode: {e}"));
            (false, None, None)
        }
    };
    let duration_ok = match duration_ms {
        Some(d) => d.abs_diff(expected_duration_ms) <= DURATION_TOLERANCE_MS,
        None => false,
    };
    if decodes && !duration_ok {
        problems.push(format!(
            "duration {} ms differs from expected {} ms by more than {} ms",
            duration_ms.unwrap_or(0),
            expected_duration_ms,
            DURATION_TOLERANCE_MS
        ));
    }
    let channels_ok = channels == Some(expected_channels);
    if decodes && !channels_ok {
        problems.push(format!(
            "{} channels, expected {}",
            channels.unwrap_or(0),
            expected_channels
        ));
    }
    VerifyReport {
        ok: size_ok && decodes && duration_ok && channels_ok,
        size_bytes,
        size_ok,
        decodes,
        duration_ms,
        duration_ok,
        channels,
        channels_ok,
        problems,
    }
}
