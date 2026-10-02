//! Inspect inputs into `InputItem`s: detect the kind from content and fill the
//! `KindDetail` each planner needs.

use crate::detect::detect;
use crate::{Engine, EngineError};
use cia_core::*;

/// A host hands these to `Engine::inspect`.
#[derive(Debug, Clone)]
pub struct InputSpec {
    pub source: SourceRef,
    pub rel_path: String,
    pub folder: Option<String>,
}

const HEAD: usize = 64 * 1024;

impl Engine {
    pub fn inspect(&self, specs: &[InputSpec]) -> Vec<InputItem> {
        specs.iter().map(|s| self.inspect_one(s)).collect()
    }

    pub fn inspect_one(&self, spec: &InputSpec) -> InputItem {
        let id = cia_core::new_id();
        let bytes = self.reader.len(&spec.source).unwrap_or(0);
        let mut item = InputItem {
            id,
            source: spec.source.clone(),
            rel_path: spec.rel_path.clone(),
            bytes,
            kind: Kind::Other,
            detail: KindDetail::default(),
            folder: spec.folder.clone(),
        };
        let head = match self.reader.read_head(&spec.source, HEAD) {
            Ok(h) => h,
            Err(_) => {
                item.detail.format = "unreadable".into();
                return item;
            }
        };
        if head.is_empty() {
            item.detail.format = "corrupt".into();
            return item;
        }
        let det = detect(
            &head,
            spec.rel_path
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or(&spec.rel_path),
        );
        item.kind = det.kind;
        item.detail.format = det.format;
        match item.kind {
            Kind::Image | Kind::AnimatedImage => {
                match cia_image::inspect(&head) {
                    Ok(info) => {
                        item.detail.width = Some(info.width);
                        item.detail.height = Some(info.height);
                        item.detail.has_alpha = Some(info.has_alpha);
                        item.detail.frame_count = Some(info.frames);
                    }
                    Err(cia_image::ImageError::Unsupported(_)) => {
                        // AVIF/HEIC need a decoder we do not compile in.
                        if !self.caps.heic_input && item.detail.format == "heic"
                            || !self.caps.avif_input && item.detail.format == "avif"
                        {
                            item.detail.encrypted = None;
                        }
                    }
                    Err(_) => {
                        // Header only may be too short; try the whole file once.
                        if let Ok(all) = self.reader.read(&spec.source) {
                            if let Ok(info) = cia_image::inspect(&all) {
                                item.detail.width = Some(info.width);
                                item.detail.height = Some(info.height);
                                item.detail.has_alpha = Some(info.has_alpha);
                                item.detail.frame_count = Some(info.frames);
                            } else {
                                item.detail.format = "corrupt".into();
                            }
                        }
                    }
                }
            }
            Kind::Pdf => {
                if let Ok(all) = self.reader.read(&spec.source) {
                    match cia_pdf::inspect(&all) {
                        Ok(info) => {
                            item.detail.page_count = Some(info.page_count);
                            item.detail.encrypted = Some(info.encrypted);
                        }
                        Err(_) => item.detail.format = "corrupt".into(),
                    }
                }
            }
            Kind::Audio => {
                if let Ok(all) = self.reader.read(&spec.source) {
                    if let Ok(info) = cia_audio::probe(&all) {
                        item.detail.duration_ms = Some(info.duration_ms);
                        item.detail.audio_streams = vec![AudioStreamInfo {
                            index: 0,
                            codec: info.codec.clone(),
                            channels: info.channels as u32,
                            sample_rate: info.sample_rate,
                            bitrate_bps: info.bitrate_bps,
                            title: None,
                        }];
                        if info.needs_ffmpeg {
                            if let Some(v) = &self.video {
                                if let Ok((dur, ch, sr)) = v.probe_audio(&spec.source) {
                                    item.detail.duration_ms = Some(dur);
                                    item.detail.audio_streams = vec![AudioStreamInfo {
                                        index: 0,
                                        codec: info.codec.clone(),
                                        channels: ch as u32,
                                        sample_rate: sr,
                                        bitrate_bps: (bytes * 8 * 1000).checked_div(dur),
                                        title: None,
                                    }];
                                }
                            }
                        }
                    }
                }
            }
            Kind::Video => {
                if let Some(v) = &self.video {
                    match v.probe(&spec.source) {
                        Ok(p) => {
                            item.detail.duration_ms = Some(p.duration_ms);
                            item.detail.width = Some(p.display_w);
                            item.detail.height = Some(p.display_h);
                            item.detail.fps = Some(p.avg_fps);
                            item.detail.video_codec = Some(p.video_codec.clone());
                            item.detail.is_hdr = Some(p.is_hdr);
                            item.detail.rotation_degrees = Some(p.rotation_degrees);
                            item.detail.audio_streams = p
                                .audio
                                .iter()
                                .map(|a| AudioStreamInfo {
                                    index: a.index,
                                    codec: a.codec.clone(),
                                    channels: a.channels,
                                    sample_rate: a.sample_rate,
                                    bitrate_bps: a.bitrate_bps,
                                    title: a.title.clone(),
                                })
                                .collect();
                        }
                        Err(_) => item.detail.format = "corrupt".into(),
                    }
                }
            }
            Kind::Archive => {
                if item.detail.format != "rar" {
                    if let Ok(all) = self.reader.read(&spec.source) {
                        if let Ok(a) = cia_archive::open(&all) {
                            item.detail.entry_count =
                                Some(a.entries().iter().filter(|e| !e.is_dir).count() as u32);
                            item.detail.encrypted = Some(a.entries().iter().any(|e| e.encrypted));
                        } else {
                            item.detail.format = "corrupt".into();
                        }
                    }
                }
            }
            Kind::OfficeDoc => {
                item.detail.encrypted = Some(item.detail.format == "ole");
            }
            Kind::Text | Kind::Other => {}
        }
        let _ = det;
        item
    }
}

impl Engine {
    /// Does this item need FFmpeg on this host (desktop without video support)?
    pub fn needs_ffmpeg(&self, item: &InputItem) -> bool {
        if self.video.is_some() {
            return false;
        }
        match item.kind {
            Kind::Video => true,
            Kind::Audio => matches!(item.detail.format.as_str(), "m4a" | "aac" | "wma" | "alac"),
            Kind::Image => {
                matches!(item.detail.format.as_str(), "heic" | "avif") && !self.caps.heic_input
            }
            _ => false,
        }
    }
}

pub fn err_code(e: &EngineError) -> &'static str {
    match e {
        EngineError::Io(m) if m.starts_with("disk_full") => "disk_full",
        EngineError::Io(m) if m.starts_with("not_writable") => "not_writable",
        EngineError::Io(m) if m.starts_with("source_vanished") => "source_vanished",
        EngineError::Io(_) => "io",
        EngineError::Damaged(_) => "damaged_input",
        EngineError::Unsupported(_) => "unsupported",
        EngineError::Cancelled => "cancelled",
        EngineError::Other(_) => "other",
    }
}
