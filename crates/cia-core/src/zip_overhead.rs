//! Exact ZIP container overhead, computed before encoding (DESIGN.md 3.9.2).
//! Per entry: 30 + n bytes local header and 46 + n bytes central directory
//! entry (n = UTF-8 name length), plus 20 bytes Zip64 extra per entry when any
//! size exceeds 4 GiB, plus the 22-byte end record (98 with Zip64).

const FOUR_GIB: u64 = 4 * 1024 * 1024 * 1024;

pub fn zip_overhead_bytes<'a>(
    names: impl IntoIterator<Item = &'a str>,
    largest_entry_bytes: u64,
) -> u64 {
    let zip64 = largest_entry_bytes >= FOUR_GIB;
    let mut total = 0u64;
    let mut count = 0u64;
    for n in names {
        let len = n.len() as u64;
        total += 30 + len + 46 + len;
        if zip64 {
            total += 20;
        }
        count += 1;
    }
    let _ = count;
    total + if zip64 { 98 } else { 22 }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn small_archive() {
        assert_eq!(zip_overhead_bytes(["a.jpg"], 1000), 30 + 5 + 46 + 5 + 22);
    }
    #[test]
    fn zip64() {
        assert_eq!(
            zip_overhead_bytes(["a", "bb"], FOUR_GIB),
            (30 + 1 + 46 + 1 + 20) + (30 + 2 + 46 + 2 + 20) + 98
        );
    }
    #[test]
    fn utf8_names_count_bytes() {
        assert_eq!(zip_overhead_bytes(["é.png"], 1), 30 + 6 + 46 + 6 + 22);
    }
}
