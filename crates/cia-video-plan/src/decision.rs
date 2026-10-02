//! Keep, remux or encode (DESIGN.md 3.5.4, last paragraph).

use crate::plan::Container;
use crate::probe::VideoProbe;
use cia_core::presets::VideoFormat;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum Decision {
    /// The file fits and its container and codecs are allowed: copy it.
    KeepOriginal,
    /// The streams are allowed but the container is not: `-c copy`.
    Remux {
        container: Container,
    },
    Encode,
}

/// If the source already fits `hard_bytes` and its codecs are in `allowed`,
/// keep it; if only the container is wrong (H.264+AAC in MKV for WhatsApp),
/// remux to the first allowed container for those codecs; otherwise encode.
///
/// A source with more than one audio track is always encoded: a copied or
/// remuxed file would keep both tracks, and chat players play only the first
/// (the mix the user expects needs an encode). Sources with no audio match
/// any allowed format by video codec alone.
pub fn keep_original_or_remux(
    probe: &VideoProbe,
    hard_bytes: u64,
    source_bytes: u64,
    allowed: &[VideoFormat],
) -> Decision {
    if source_bytes > hard_bytes || source_bytes == 0 || probe.audio.len() > 1 {
        return Decision::Encode;
    }
    let video = probe.video_codec.to_ascii_lowercase();
    let audio = probe.audio.first().map(|t| t.codec.to_ascii_lowercase());
    let container = probe.container.to_ascii_lowercase();
    let streams_match = |f: &VideoFormat| {
        f.video.eq_ignore_ascii_case(&video)
            && audio
                .as_ref()
                .is_none_or(|a| f.audio.eq_ignore_ascii_case(a))
    };
    if allowed
        .iter()
        .any(|f| streams_match(f) && f.container.eq_ignore_ascii_case(&container))
    {
        return Decision::KeepOriginal;
    }
    allowed
        .iter()
        .find(|f| streams_match(f))
        .and_then(|f| Container::parse(&f.container))
        .map_or(Decision::Encode, |container| Decision::Remux { container })
}
