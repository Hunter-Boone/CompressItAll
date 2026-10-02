//! Native H.264 encoder order (DESIGN.md 3.5.6). The web pipeline has its
//! own capability check and does not use this.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Windows,
    MacOs,
    Linux,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gpu {
    Nvidia,
    Amd,
    Intel,
}

/// Encoders to try, first available and working wins:
///
/// 1. `libx264` when the user's own FFmpeg has it (two-pass).
/// 2. macOS: `h264_videotoolbox`.
/// 3. With `faster`: `h264_nvenc` / `h264_amf` / `h264_qsv` by GPU vendor;
///    Linux adds `h264_vaapi`.
/// 4. Windows: `h264_mf` (Media Foundation, on every Windows 10/11).
/// 5. `libvpx-vp9` when the preset allows WebM (two-pass), so the caller
///    can switch the target. An empty list means `NoEncoder`.
///
/// "Faster" off skips the GPU encoders but keeps VideoToolbox and Media
/// Foundation, since those are the only H.264 encoders on many machines.
pub fn encoder_chain(
    platform: Platform,
    gpu: Option<Gpu>,
    has_libx264: bool,
    faster: bool,
    allow_webm: bool,
) -> Vec<&'static str> {
    let mut chain = Vec::new();
    if has_libx264 {
        chain.push("libx264");
    }
    match platform {
        Platform::MacOs => chain.push("h264_videotoolbox"),
        Platform::Windows | Platform::Linux => {
            if faster {
                match gpu {
                    Some(Gpu::Nvidia) => chain.push("h264_nvenc"),
                    Some(Gpu::Amd) => chain.push("h264_amf"),
                    Some(Gpu::Intel) => chain.push("h264_qsv"),
                    None => {}
                }
                if platform == Platform::Linux {
                    chain.push("h264_vaapi");
                }
            }
            if platform == Platform::Windows {
                chain.push("h264_mf");
            }
        }
    }
    if allow_webm {
        chain.push("libvpx-vp9");
    }
    chain
}
