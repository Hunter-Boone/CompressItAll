//! Candidate encoders in order (DESIGN.md 3.4.3) and their fixed settings.

use crate::classify::Class;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputImageFormat {
    Jpeg,
    Png,
    Webp,
    Avif,
    Gif,
}

impl OutputImageFormat {
    pub fn token(self) -> &'static str {
        match self {
            OutputImageFormat::Jpeg => "jpeg",
            OutputImageFormat::Png => "png",
            OutputImageFormat::Webp => "webp",
            OutputImageFormat::Avif => "avif",
            OutputImageFormat::Gif => "gif",
        }
    }
    pub fn extension(self) -> &'static str {
        match self {
            OutputImageFormat::Jpeg => "jpg",
            OutputImageFormat::Png => "png",
            OutputImageFormat::Webp => "webp",
            OutputImageFormat::Avif => "avif",
            OutputImageFormat::Gif => "gif",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "jpeg" | "jpg" => Some(OutputImageFormat::Jpeg),
            "png" => Some(OutputImageFormat::Png),
            "webp" => Some(OutputImageFormat::Webp),
            "avif" => Some(OutputImageFormat::Avif),
            "gif" => Some(OutputImageFormat::Gif),
            _ => None,
        }
    }
    /// Compatibility order for ties (3.4.5): JPEG, PNG, WebP, AVIF.
    pub fn compat_rank(self) -> u8 {
        match self {
            OutputImageFormat::Jpeg => 0,
            OutputImageFormat::Png => 1,
            OutputImageFormat::Gif => 2,
            OutputImageFormat::Webp => 3,
            OutputImageFormat::Avif => 4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Candidate {
    Jpeg,
    WebpLossy,
    Avif,
    PngLossless,
    WebpLossless,
    /// Palette-quantised PNG with this many colours (256, 128, 64).
    PngPalette(u16),
}

impl Candidate {
    pub fn format(self) -> OutputImageFormat {
        match self {
            Candidate::Jpeg => OutputImageFormat::Jpeg,
            Candidate::WebpLossy | Candidate::WebpLossless => OutputImageFormat::Webp,
            Candidate::Avif => OutputImageFormat::Avif,
            Candidate::PngLossless | Candidate::PngPalette(_) => OutputImageFormat::Png,
        }
    }
    pub fn is_lossy(self) -> bool {
        matches!(self, Candidate::Jpeg | Candidate::WebpLossy | Candidate::Avif)
    }
    /// Quality search bounds (3.4.4); None for lossless/palette candidates.
    pub fn quality_range(self) -> Option<(u8, u8)> {
        match self {
            Candidate::Jpeg | Candidate::WebpLossy => Some((45, 92)),
            Candidate::Avif => Some((40, 85)),
            _ => None,
        }
    }
    pub fn name(self) -> String {
        match self {
            Candidate::Jpeg => "mozjpeg".into(),
            Candidate::WebpLossy => "libwebp-lossy".into(),
            Candidate::Avif => "ravif".into(),
            Candidate::PngLossless => "oxipng".into(),
            Candidate::WebpLossless => "libwebp-lossless".into(),
            Candidate::PngPalette(n) => format!("png-palette-{n}"),
        }
    }
}

/// The ordered candidate list for a class/alpha pair, filtered by allowed formats.
/// `modern` enables AVIF (Smaller mode with "Modern formats").
pub fn candidates_for(class: Class, has_alpha: bool, allowed: &[OutputImageFormat], modern: bool, source_format_only: Option<OutputImageFormat>) -> Vec<Candidate> {
    let all: Vec<Candidate> = match (class, has_alpha) {
        (Class::Photo, false) => vec![Candidate::Jpeg, Candidate::WebpLossy, Candidate::Avif],
        (Class::Photo, true) => vec![Candidate::WebpLossy, Candidate::PngPalette(256), Candidate::PngLossless],
        (Class::Graphic, false) => vec![Candidate::PngLossless, Candidate::WebpLossless, Candidate::PngPalette(256), Candidate::PngPalette(128), Candidate::PngPalette(64), Candidate::Jpeg],
        (Class::Graphic, true) => vec![Candidate::PngLossless, Candidate::WebpLossless, Candidate::PngPalette(256), Candidate::WebpLossy],
    };
    all.into_iter()
        .filter(|c| allowed.contains(&c.format()))
        .filter(|c| *c != Candidate::Avif || modern)
        .filter(|c| source_format_only.is_none_or(|f| c.format() == f))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn alpha_never_routes_to_jpeg() {
        let all = [OutputImageFormat::Jpeg, OutputImageFormat::Png, OutputImageFormat::Webp, OutputImageFormat::Avif];
        for class in [Class::Photo, Class::Graphic] {
            let c = candidates_for(class, true, &all, true, None);
            assert!(!c.contains(&Candidate::Jpeg), "{class:?}: {c:?}");
            assert!(!c.is_empty());
        }
    }
    #[test]
    fn filtered_by_allowed_and_source() {
        let c = candidates_for(Class::Photo, false, &[OutputImageFormat::Png], false, None);
        assert!(c.is_empty());
        let c = candidates_for(Class::Graphic, false, &[OutputImageFormat::Png, OutputImageFormat::Jpeg], false, Some(OutputImageFormat::Png));
        assert_eq!(c, vec![Candidate::PngLossless, Candidate::PngPalette(256), Candidate::PngPalette(128), Candidate::PngPalette(64)]);
    }
}
