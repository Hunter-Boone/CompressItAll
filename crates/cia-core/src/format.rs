//! Number and duration formatting shared by copy and the UI (DESIGN.md 4.1):
//! decimal megabytes, one decimal place, "about" is the caller's job.

/// "19.6 MB", "812 KB", "1.2 GB". Decimal units.
pub fn mb(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        let v = bytes as f64 / 1_000_000_000.0;
        format!("{} GB", one_decimal(v))
    } else if bytes >= 1_000_000 {
        let v = bytes as f64 / 1_000_000.0;
        format!("{} MB", one_decimal(v))
    } else if bytes >= 1_000 {
        format!("{} KB", (bytes as f64 / 1_000.0).round() as u64)
    } else {
        format!("{} bytes", bytes)
    }
}

/// Whole megabytes for limits: "20 MB", "18 MB", "1 GB".
pub fn mb_whole(bytes: u64) -> String {
    if bytes >= 1_000_000_000 && bytes.is_multiple_of(100_000_000) {
        let v = bytes as f64 / 1_000_000_000.0;
        return format!("{} GB", trim_zero(v));
    }
    if bytes >= 1_000_000 {
        // A typed custom value such as 1.5 MB or 1.25 MB keeps its decimals; a preset limit
        // like 18,196,153 bytes (Email) still reads "18 MB".
        if !bytes.is_multiple_of(1_000_000) && bytes.is_multiple_of(10_000) {
            let s = format!("{:.2}", bytes as f64 / 1_000_000.0);
            return format!("{} MB", s.trim_end_matches('0').trim_end_matches('.'));
        }
        return format!("{} MB", (bytes as f64 / 1_000_000.0).floor() as u64);
    }
    format!("{} KB", (bytes as f64 / 1_000.0).floor() as u64)
}

fn one_decimal(v: f64) -> String {
    let s = format!("{:.1}", v);
    s
}

fn trim_zero(v: f64) -> String {
    let s = format!("{:.1}", v);
    s.trim_end_matches(".0").to_string()
}

/// "4 min 12 s", "48 s", "1 h 12 min".
pub fn duration(ms: u64) -> String {
    let total_s = (ms + 500) / 1000;
    let h = total_s / 3600;
    let m = (total_s % 3600) / 60;
    let s = total_s % 60;
    if h > 0 {
        if m > 0 {
            format!("{} h {} min", h, m)
        } else {
            format!("{} h", h)
        }
    } else if m > 0 {
        if s > 0 {
            format!("{} min {} s", m, s)
        } else {
            format!("{} min", m)
        }
    } else {
        format!("{} s", s)
    }
}

/// "10:42" from unix seconds in the local-time offset the host provides (minutes east of UTC).
pub fn clock_time(unix_s: u64, tz_offset_min: i32) -> String {
    let local = unix_s as i64 + tz_offset_min as i64 * 60;
    let day_s = local.rem_euclid(86_400);
    format!("{:02}:{:02}", day_s / 3600, (day_s % 3600) / 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sizes() {
        assert_eq!(mb(19_600_000), "19.6 MB");
        assert_eq!(mb(212_400_000), "212.4 MB");
        assert_eq!(mb(1_200_000_000), "1.2 GB");
        assert_eq!(mb(812_000), "812 KB");
        assert_eq!(mb(512), "512 bytes");
        assert_eq!(mb_whole(20_971_520), "20 MB");
        assert_eq!(mb_whole(1_000_000_000), "1 GB");
        assert_eq!(mb_whole(18_196_153), "18 MB");
    }
    #[test]
    fn durations() {
        assert_eq!(duration(252_000), "4 min 12 s");
        assert_eq!(duration(48_400), "48 s");
        assert_eq!(duration(4_320_000), "1 h 12 min");
        assert_eq!(duration(120_000), "2 min");
    }
    #[test]
    fn clock() {
        assert_eq!(clock_time(0, 0), "00:00");
        assert_eq!(clock_time(38_520, -240), "06:42");
    }

    #[test]
    fn whole_limits_keep_typed_decimals_only() {
        assert_eq!(mb_whole(20 * 1024 * 1024), "20 MB");
        assert_eq!(mb_whole(18_196_153), "18 MB");
        assert_eq!(mb_whole(1_500_000), "1.5 MB");
        assert_eq!(mb_whole(1_250_000), "1.25 MB");
        assert_eq!(mb_whole(8_000_000), "8 MB");
        assert_eq!(mb_whole(2_000_000_000), "2 GB");
        assert_eq!(mb_whole(512_000), "512 KB");
    }
}
