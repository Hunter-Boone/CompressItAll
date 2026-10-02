//! Every user-facing sentence the engine produces (DESIGN.md 3.11, 4.3, 4.10).
//! One function per outcome variant so the UI cannot invent its own success
//! message. Tone: plain words, short sentences, real numbers, no exclamation marks.

use crate::format::{duration, mb, mb_whole};
use crate::model::*;

/// The brand name as it appears in sentences. Read from packages/brand at build time.
pub const BRAND: &str = {
    // brand.json is small; a const parse is overkill, so the name is mirrored here
    // and a test checks it against packages/brand/brand.json.
    "Smidge"
};

pub fn fitted(a: &Artifact, label: &str) -> String {
    format!("Fits in {label}. {}", a.summary)
}

pub fn kept_original(smaller_mode: bool) -> String {
    if smaller_mode {
        "Already as small as it gets.".to_string()
    } else {
        "Already fits. Nothing to change.".to_string()
    }
}

pub fn refusal_message(
    code: &RefusalCode,
    hard_bytes: Option<u64>,
    smallest: Option<u64>,
    kind: Kind,
) -> String {
    let limit = hard_bytes.map(mb_whole).unwrap_or_default();
    match code {
        RefusalCode::TooLongForLimit { max_duration_ms } => {
            format!("Too long to fit in {limit} at watchable quality. Trim it to under {}.", duration(*max_duration_ms))
        }
        RefusalCode::BelowQualityFloor => match smallest {
            Some(s) => format!("The smallest {BRAND} could make this was {}. The limit is {limit}.", mb(s)),
            None => format!("This can't be made small enough for {limit} without ruining it."),
        },
        RefusalCode::CannotShrinkType => match smallest {
            Some(s) => format!("{BRAND} can't make this kind of file smaller. It's {} and the limit is {limit}.", mb(s)),
            None => format!("{BRAND} can't make this kind of file smaller."),
        },
        RefusalCode::TooManyFilesForMessage { max } => format!("That's more than {max} files for one message."),
        RefusalCode::TotalTooBig => match smallest {
            Some(s) => format!("Together these come to at least {}. The limit is {limit}.", mb(s)),
            None => format!("Together these are too big for {limit}."),
        },
        RefusalCode::NeedsFfmpeg => {
            let what = match kind {
                Kind::Audio => "This kind of music file needs",
                _ => "Videos need",
            };
            format!("{what} a one-time setup.")
        }
        RefusalCode::BrowserLacksCodec { .. } => match kind {
            Kind::Video => "This browser can't read this video. Try Chrome, Edge or Safari, or the desktop app.".to_string(),
            _ => "This browser can't open this kind of file. Try Chrome, Edge or Safari, or the desktop app.".to_string(),
        },
        RefusalCode::UnsupportedInput { what } => {
            if what == "heic" {
                "This browser can't open iPhone HEIC photos. Use Safari, or the desktop app.".to_string()
            } else {
                format!("{BRAND} can't open this kind of file yet.")
            }
        }
        RefusalCode::Encrypted => format!("This file is password-protected, so {BRAND} can't change it."),
    }
}

pub fn failure_message(code: &str, closest: Option<u64>, hard_bytes: Option<u64>) -> String {
    match code {
        "damaged_input" => format!("This file is damaged or incomplete, so {BRAND} can't read it."),
        "damaged_pdf" => "This PDF is damaged.".to_string(),
        "damaged_video" => {
            format!("This video file is damaged or incomplete, so {BRAND} can't read it.")
        }
        "disk_full" => {
            "There isn't enough free space to save the result. Free up some space and try again."
                .to_string()
        }
        "not_writable" => {
            format!("{BRAND} can't save in that folder. Choose another folder in Advanced.")
        }
        "source_vanished" => {
            format!("The original file was moved or deleted while {BRAND} was working.")
        }
        "encoder_stalled" => "The video encoder stopped responding.".to_string(),
        "out_of_memory" => format!("{BRAND} ran out of memory on this file. Try the desktop app."),
        "over_after_retries" => match (closest, hard_bytes) {
            (Some(c), Some(h)) => format!(
                "{BRAND} couldn't get this under {}. The closest was {}, so nothing was saved.",
                mb_whole(h),
                mb(c)
            ),
            _ => format!("{BRAND} couldn't get this under the limit, so nothing was saved."),
        },
        _ => format!("Something went wrong and {BRAND} couldn't finish this file."),
    }
}

/// Suggestion button labels.
pub fn suggestion_label(s: &Suggestion, preset_tile: impl Fn(&str) -> String) -> String {
    match s {
        Suggestion::Trim { max_duration_ms } => {
            format!("Trim to under {}", duration(*max_duration_ms))
        }
        Suggestion::PickPreset {
            preset_id,
            predicted_bytes,
        } => {
            format!(
                "Use {} (about {})",
                preset_tile(preset_id),
                mb(*predicted_bytes)
            )
        }
        Suggestion::SplitIntoMessages { groups } => format!("Split into {} messages", groups.len()),
        Suggestion::RemoveFiles { item_ids } => {
            if item_ids.len() == 1 {
                "Leave out the biggest file".to_string()
            } else {
                format!("Leave out the {} biggest files", item_ids.len())
            }
        }
        Suggestion::InstallFfmpeg => "Set up video support".to_string(),
        Suggestion::UseDesktopApp => "Use the desktop app".to_string(),
        Suggestion::UseOtherBrowser { browser } => format!("Try {browser}"),
    }
}

/// Step 3 headline for a plan. `items` is (count by kind, total input bytes).
pub fn plan_headline(
    plan_items: &[ItemPlan],
    inputs: &[InputItem],
    goal: &Goal,
    verdict: &PlanVerdict,
) -> String {
    if let PlanVerdict::CannotFit { refusal } = verdict {
        return refusal.message.clone();
    }
    let total: u64 = plan_items
        .iter()
        .map(|p| p.prediction.predicted_bytes)
        .sum();
    let exact = plan_items.iter().all(|p| p.prediction.exact);
    let about = if exact { "" } else { "about " };
    let quality = match verdict {
        PlanVerdict::WillFit { quality } | PlanVerdict::Uncertain { quality } => Some(*quality),
        _ => None,
    };
    let q = quality
        .map(|q| format!(" Quality: {}.", q.word()))
        .unwrap_or_default();
    if plan_items.len() == 1 {
        let item = &inputs[0];
        let p = &plan_items[0];
        let what = describe_input(item);
        if matches!(p.strategy, Strategy::Copy) {
            return format!("{what} already fits. Nothing to change.");
        }
        return format!(
            "{what} → {}, {about}{}.{q}",
            p.prediction.summary,
            mb(total)
        );
    }
    let what = describe_group(inputs);
    match goal {
        Goal::Fit { limit, .. } if limit.scope == LimitScope::PerMessage => {
            format!("{what} → {about}{}, fits in one message.{q}", mb(total))
        }
        Goal::Fit { .. } => format!("{what} → {about}{} in total.{q}", mb(total)),
        Goal::Smaller { .. } => format!("{what} → {about}{}.", mb(total)),
    }
}

pub fn describe_input(item: &InputItem) -> String {
    let d = &item.detail;
    match item.kind {
        Kind::Image => match (d.width, d.height) {
            (Some(w), Some(h)) => format!("{w} × {h} photo"),
            _ => "Photo".to_string(),
        },
        Kind::AnimatedImage => "Animated image".to_string(),
        Kind::Video => match d.duration_ms {
            Some(ms) => format!("{} video", duration(ms)),
            None => "Video".to_string(),
        },
        Kind::Audio => match d.duration_ms {
            Some(ms) => format!("{} audio", duration(ms)),
            None => "Audio".to_string(),
        },
        Kind::Pdf => match d.page_count {
            Some(1) => "1-page PDF".to_string(),
            Some(n) => format!("{n}-page PDF"),
            None => "PDF".to_string(),
        },
        Kind::OfficeDoc => match d.format.as_str() {
            "pptx" | "pptm" | "odp" => "Presentation".to_string(),
            "xlsx" | "xlsm" | "ods" => "Spreadsheet".to_string(),
            "epub" => "Ebook".to_string(),
            _ => "Document".to_string(),
        },
        Kind::Archive => "Zip file".to_string(),
        Kind::Text => "Text file".to_string(),
        Kind::Other => "File".to_string(),
    }
}

pub fn describe_group(items: &[InputItem]) -> String {
    let n = items.len();
    let photos = items.iter().filter(|i| i.kind == Kind::Image).count();
    let videos = items.iter().filter(|i| i.kind == Kind::Video).count();
    if photos == n {
        format!("{n} photos")
    } else if videos == n {
        format!("{n} videos")
    } else {
        format!("{n} files")
    }
}

pub fn result_headline(
    verdict: JobVerdict,
    label: &str,
    fitted: usize,
    total: usize,
    smallest: Option<u64>,
    hard: Option<u64>,
    saved_before_stop: usize,
) -> String {
    match verdict {
        JobVerdict::AllFit => format!("Fits in {label}"),
        JobVerdict::SomeFit => format!(
            "{fitted} of {total} files fit. {} couldn't.",
            total - fitted
        ),
        JobVerdict::NoneFit => match (smallest, hard) {
            (Some(s), Some(h)) => format!(
                "The smallest {BRAND} could make this was {}. {label} allows {}.",
                mb(s),
                mb_whole(h)
            ),
            _ => "Nothing could be made small enough.".to_string(),
        },
        JobVerdict::Cancelled => {
            if saved_before_stop == 0 {
                "Stopped. Nothing was saved.".to_string()
            } else {
                format!("Stopped. {saved_before_stop} files were saved before you stopped.")
            }
        }
    }
}

pub fn allowance_blocked(next_free_clock: &str) -> String {
    format!("You've used your 3 free files. Your next free file is ready at {next_free_clock}, or get {BRAND} Pro for unlimited files.")
}

pub fn allowance_too_many(files: usize, remaining: usize) -> String {
    format!("This has {files} files. The free version does {remaining} more today.")
}

pub fn zip_because_of_count(label: &str, max: u32, files: usize) -> String {
    format!("{label} takes {max} files per message, so {BRAND} will put these {files} files in one zip.")
}

pub fn size_arrow(before: u64, after: u64) -> String {
    format!("{} → {}", mb(before), mb(after))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brand_matches_brand_json() {
        let json: serde_json::Value =
            serde_json::from_str(include_str!("../../../packages/brand/brand.json")).unwrap();
        assert_eq!(json["name"].as_str().unwrap(), BRAND);
    }

    #[test]
    fn refusal_sentences() {
        let s = refusal_message(
            &RefusalCode::TooLongForLimit {
                max_duration_ms: 150_000,
            },
            Some(20_971_520),
            None,
            Kind::Video,
        );
        assert_eq!(
            s,
            "Too long to fit in 20 MB at watchable quality. Trim it to under 2 min 30 s."
        );
        let s = refusal_message(
            &RefusalCode::CannotShrinkType,
            Some(20_971_520),
            Some(48_200_000),
            Kind::Other,
        );
        assert_eq!(
            s,
            "Smidge can't make this kind of file smaller. It's 48.2 MB and the limit is 20 MB."
        );
        let s = refusal_message(&RefusalCode::Encrypted, None, None, Kind::Pdf);
        assert_eq!(
            s,
            "This file is password-protected, so Smidge can't change it."
        );
        assert_eq!(
            refusal_message(
                &RefusalCode::BelowQualityFloor,
                Some(20_971_520),
                Some(23_800_000),
                Kind::Image
            ),
            "The smallest Smidge could make this was 23.8 MB. The limit is 20 MB."
        );
    }

    #[test]
    fn failure_sentences() {
        assert_eq!(
            failure_message("over_after_retries", Some(21_300_000), Some(20_971_520)),
            "Smidge couldn't get this under 20 MB. The closest was 21.3 MB, so nothing was saved."
        );
        assert_eq!(
            failure_message("source_vanished", None, None),
            "The original file was moved or deleted while Smidge was working."
        );
    }

    #[test]
    fn no_exclamation_marks_anywhere() {
        let all = [
            refusal_message(&RefusalCode::NeedsFfmpeg, None, None, Kind::Video),
            failure_message("damaged_input", None, None),
            failure_message("whatever", None, None),
            kept_original(true),
            kept_original(false),
            allowance_blocked("10:42"),
            result_headline(JobVerdict::Cancelled, "Discord", 0, 3, None, None, 0),
        ];
        for s in all {
            assert!(!s.contains('!'), "{s}");
            assert!(!s.to_lowercase().contains("oops"), "{s}");
        }
    }

    #[test]
    fn headlines() {
        let item = InputItem {
            id: "i".into(),
            source: SourceRef::Path {
                path: "/x/clip.mkv".into(),
            },
            rel_path: "clip.mkv".into(),
            bytes: 380_000_000,
            kind: Kind::Video,
            detail: KindDetail {
                duration_ms: Some(124_000),
                ..Default::default()
            },
            folder: None,
        };
        let plan = ItemPlan {
            item_id: "i".into(),
            budget_bytes: Some(20_000_000),
            strategy: Strategy::None,
            prediction: Prediction {
                predicted_bytes: 19_600_000,
                exact: false,
                summary: "720p, 30 fps".into(),
                quality: Some(QualityLabel::Good),
                notes: vec![],
            },
        };
        let goal = Goal::Fit {
            preset_id: "discord-free".into(),
            limit: crate::presets::find("discord-free")
                .unwrap()
                .resolve()
                .unwrap(),
        };
        let h = plan_headline(
            &[plan],
            &[item],
            &goal,
            &PlanVerdict::WillFit {
                quality: QualityLabel::Good,
            },
        );
        assert_eq!(
            h,
            "2 min 4 s video → 720p, 30 fps, about 19.6 MB. Quality: Good."
        );
        assert_eq!(
            result_headline(JobVerdict::SomeFit, "Discord", 18, 23, None, None, 0),
            "18 of 23 files fit. 5 couldn't."
        );
    }
}
