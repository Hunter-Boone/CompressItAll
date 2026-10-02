//! Output naming (DESIGN.md 3.10): `<stem> (<Label>).<ext>`, collisions get
//! ` 2`, ` 3`..., Windows reserved names and length limits are handled here.

/// Split "photo.final.jpg" into ("photo.final", Some("jpg")). Handles ".tar.gz"/".tar.zst"/".tar.xz".
pub fn split_stem(file_name: &str) -> (String, Option<String>) {
    let lower = file_name.to_ascii_lowercase();
    for multi in [".tar.gz", ".tar.zst", ".tar.xz", ".tar.bz2"] {
        if lower.ends_with(multi) && file_name.len() > multi.len() {
            let stem = &file_name[..file_name.len() - multi.len()];
            return (stem.to_string(), Some(multi[1..].to_string()));
        }
    }
    match file_name.rfind('.') {
        Some(i) if i > 0 && i + 1 < file_name.len() => (
            file_name[..i].to_string(),
            Some(file_name[i + 1..].to_string()),
        ),
        _ => (file_name.to_string(), None),
    }
}

const WINDOWS_RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Max name length in UTF-16 units (Windows limit; also safe on macOS/Linux at 255 bytes for ASCII).
pub const MAX_NAME_UTF16: usize = 255;

/// Build the n-th candidate name. n = 1 gives `<stem> (<Label>).<ext>`, n = 2 gives `<stem> (<Label> 2).<ext>`.
pub fn output_name(stem: &str, label: &str, ext: Option<&str>, n: u32) -> String {
    let suffix = if n <= 1 {
        format!(" ({label})")
    } else {
        format!(" ({label} {n})")
    };
    let ext_part = ext.map(|e| format!(".{e}")).unwrap_or_default();
    let mut stem = sanitize_stem(stem);
    // Length: trim the stem and add an ellipsis so the suffix and extension always fit.
    let fixed = suffix.encode_utf16().count() + ext_part.encode_utf16().count();
    let max_stem = MAX_NAME_UTF16.saturating_sub(fixed);
    if stem.encode_utf16().count() > max_stem {
        let keep = max_stem.saturating_sub(1);
        let mut acc = String::new();
        let mut units = 0;
        for ch in stem.chars() {
            let u = ch.len_utf16();
            if units + u > keep {
                break;
            }
            acc.push(ch);
            units += u;
        }
        stem = format!("{}…", acc.trim_end());
    }
    format!("{stem}{suffix}{ext_part}")
}

/// Strip characters no filesystem accepts and defuse Windows reserved names.
pub fn sanitize_stem(stem: &str) -> String {
    let mut s: String = stem
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if (c as u32) < 32 => '_',
            c => c,
        })
        .collect();
    let trimmed = s.trim().trim_end_matches('.').to_string();
    s = if trimmed.is_empty() {
        "file".to_string()
    } else {
        trimmed
    };
    if WINDOWS_RESERVED.contains(&s.to_ascii_uppercase().as_str()) {
        s.push('_');
    }
    s
}

/// Pick the first name for which `exists(name)` is false. Never overwrites.
pub fn first_free_name(
    stem: &str,
    label: &str,
    ext: Option<&str>,
    mut exists: impl FnMut(&str) -> bool,
) -> String {
    let mut n = 1;
    loop {
        let name = output_name(stem, label, ext, n);
        if !exists(&name) {
            return name;
        }
        n += 1;
        if n > 10_000 {
            return output_name(stem, label, ext, n);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stems() {
        assert_eq!(
            split_stem("photo.final.jpg"),
            ("photo.final".into(), Some("jpg".into()))
        );
        assert_eq!(
            split_stem("archive.tar.gz"),
            ("archive".into(), Some("tar.gz".into()))
        );
        assert_eq!(split_stem("README"), ("README".into(), None));
        assert_eq!(split_stem(".bashrc"), (".bashrc".into(), None));
    }
    #[test]
    fn names_and_collisions() {
        assert_eq!(
            output_name("photo", "Discord", Some("jpg"), 1),
            "photo (Discord).jpg"
        );
        assert_eq!(
            output_name("photo", "Discord", Some("jpg"), 2),
            "photo (Discord 2).jpg"
        );
        assert_eq!(
            output_name("Grandkids", "Email", None, 1),
            "Grandkids (Email)"
        );
        let mut taken = vec![
            "photo (Discord).jpg".to_string(),
            "photo (Discord 2).jpg".to_string(),
        ];
        let n = first_free_name("photo", "Discord", Some("jpg"), |n| {
            taken.contains(&n.to_string())
        });
        assert_eq!(n, "photo (Discord 3).jpg");
        taken.push(n);
    }
    #[test]
    fn keeps_emoji_and_scripts() {
        assert_eq!(
            output_name("Grandkids 🎂 誕生日 (final)", "Custom 8 MB", Some("mov"), 1),
            "Grandkids 🎂 誕生日 (final) (Custom 8 MB).mov"
        );
    }
    #[test]
    fn reserved_and_illegal() {
        assert_eq!(
            output_name("CON", "Discord", Some("png"), 1),
            "CON_ (Discord).png"
        );
        assert_eq!(output_name("a:b?c", "Discord", None, 1), "a_b_c (Discord)");
        assert_eq!(output_name("   ", "Discord", None, 1), "file (Discord)");
        assert_eq!(
            output_name("trailing.", "Discord", None, 1),
            "trailing (Discord)"
        );
    }
    #[test]
    fn long_names_are_trimmed() {
        let stem = "x".repeat(300);
        let name = output_name(&stem, "Discord", Some("jpg"), 1);
        assert!(
            name.encode_utf16().count() <= MAX_NAME_UTF16,
            "{}",
            name.len()
        );
        assert!(name.ends_with("… (Discord).jpg"));
        let stem = "誕".repeat(300);
        let name = output_name(&stem, "Email", Some("jpg"), 12);
        assert!(name.encode_utf16().count() <= MAX_NAME_UTF16);
    }
}
