//! Per-kind planners. Each one answers two questions: "what will this become
//! and how big" (for the preview) and "do it" (for the run). The run functions
//! return the encoded bytes plus everything the engine needs to verify, name
//! and report the result.

pub mod archive;
pub mod audio;
pub mod image;
pub mod pdf;
pub mod plain;

use cia_core::*;

/// A finished encode, before the engine writes and re-verifies it.
#[derive(Debug, Clone)]
pub struct Encoded {
    pub bytes: Vec<u8>,
    /// Output format token ("jpeg", "pdf", "zip", "flac"...).
    pub format: String,
    pub extension: String,
    pub summary: String,
    pub quality: Option<QualityLabel>,
    pub attempts: Vec<Attempt>,
    /// Extra checks already run by the planner's own verifier (recorded, then re-run by the engine where it can).
    pub verification: VerificationReport,
}

#[derive(Debug, Clone)]
pub enum PlannerOutcome {
    Encoded(Encoded),
    KeptOriginal { attempts: Vec<Attempt> },
    Refused { code: RefusalCode, smallest_bytes: Option<u64>, attempts: Vec<Attempt> },
    Failed { code: &'static str, message: Option<String>, closest_bytes: Option<u64> },
}

/// Sizes used by the per-message allocator (DESIGN.md 3.9.1).
#[derive(Debug, Clone, Copy)]
pub struct Sizes {
    pub lossless: u64,
    pub floor: u64,
}

pub struct Ctx<'a> {
    pub options: &'a JobOptions,
    pub allowed_image: Vec<String>,
    pub allowed_audio: Vec<String>,
    pub hard_bytes: Option<u64>,
    pub smaller: Option<SmallerLevel>,
    pub cancel: &'a (dyn Fn() -> bool + Sync),
    pub progress: &'a (dyn Fn(f32, &str) + Sync),
}

impl Ctx<'_> {
    pub fn smaller_mode(&self) -> bool {
        self.smaller.is_some()
    }
}

pub fn attempt(n: u32, item_id: &str, encoder: &str, params: serde_json::Value, bytes: Option<u64>, score: Option<f32>, verdict: AttemptVerdict) -> Attempt {
    Attempt { n, item_id: item_id.to_string(), encoder: encoder.to_string(), params, output_bytes: bytes, score, elapsed_ms: 0, verdict }
}
