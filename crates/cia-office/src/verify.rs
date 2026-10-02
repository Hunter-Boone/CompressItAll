//! Step 5 of the Office planner (DESIGN.md 3.8) and the Office row of the
//! honesty rule (3.11): the bytes handed back must re-open, keep every entry
//! name in order, carry XML that parses, point relationships at parts that
//! exist, hold images that decode, and fit the limit.

use std::collections::{HashMap, HashSet};
use std::fmt;

use crate::package::{self, Entry, DETAILS_PARTS};
use crate::xml;
use crate::{ImageFormat, MediaKind, OfficeKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyCheck {
    pub name: &'static str,
    pub passed: bool,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyReport {
    pub ok: bool,
    pub checks: Vec<VerifyCheck>,
    pub entry_count: usize,
    pub output_bytes: u64,
}

impl VerifyReport {
    fn new(output_bytes: u64) -> Self {
        Self {
            ok: true,
            checks: Vec::new(),
            entry_count: 0,
            output_bytes,
        }
    }

    fn push(&mut self, name: &'static str, passed: bool, detail: impl Into<String>) {
        self.checks.push(VerifyCheck {
            name,
            passed,
            detail: detail.into(),
        });
        self.ok &= passed;
    }

    pub fn failures(&self) -> impl Iterator<Item = &VerifyCheck> {
        self.checks.iter().filter(|c| !c.passed)
    }
}

impl fmt::Display for VerifyReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for c in &self.checks {
            writeln!(
                f,
                "[{}] {}: {}",
                if c.passed { "ok" } else { "FAIL" },
                c.name,
                c.detail
            )?;
        }
        Ok(())
    }
}

/// Extensions whose entries must parse as XML.
fn is_xml_name(name: &str) -> bool {
    matches!(
        package::extension(name).as_str(),
        "xml" | "rels" | "opf" | "ncx"
    )
}

/// Check `output` against `original`. Every problem is a failed check in
/// the report; the only way to get no checks at all is an original that
/// does not open, which is reported as the single failing `original_opens`.
pub fn verify(original: &[u8], output: &[u8], hard_bytes: Option<u64>) -> VerifyReport {
    let mut report = VerifyReport::new(output.len() as u64);

    let orig = match package::read(original) {
        Ok(p) => p,
        Err(e) => {
            report.push(
                "original_opens",
                false,
                format!("original does not open: {e}"),
            );
            return report;
        }
    };

    // 1. Re-open and read every entry fully; the zip reader checks each CRC
    //    and we check the declared size.
    let out_entries = match read_all_checked(output) {
        Ok(v) => v,
        Err(e) => {
            report.push("reads", false, e);
            return report;
        }
    };
    report.entry_count = out_entries.len();
    report.push(
        "reads",
        true,
        format!("{} entries read back, CRCs match", out_entries.len()),
    );

    // 2. Same names in the same order (mimetype may have moved to the front).
    let expected: Vec<&str> = orig
        .output_order()
        .into_iter()
        .map(|i| orig.entries[i].name.as_str())
        .collect();
    let found: Vec<&str> = out_entries.iter().map(|e| e.entry.name.as_str()).collect();
    let order_ok = expected == found;
    report.push(
        "entry_order",
        order_ok,
        if order_ok {
            format!("{} entries in the original order", found.len())
        } else {
            first_order_difference(&expected, &found)
        },
    );

    // ODF and EPUB: mimetype first, stored, at the fixed offset readers use.
    if orig.kind.has_mimetype() {
        let first_ok = out_entries
            .first()
            .is_some_and(|e| e.entry.name == "mimetype" && e.method == "stored");
        let raw_ok =
            output.len() >= 38 && &output[30..38] == b"mimetype" && output[8..10] == [0, 0];
        report.push(
            "mimetype_first_stored",
            first_ok && raw_ok,
            if first_ok && raw_ok {
                "mimetype is entry 0, method 0, name at offset 30"
            } else {
                "mimetype is not the first stored entry"
            },
        );
    }

    // 3. Every XML part parses.
    let mut bad_xml = Vec::new();
    for e in &out_entries {
        if e.entry.is_dir || !is_xml_name(&e.entry.name) {
            continue;
        }
        if let Err(msg) = xml::parses(&e.data) {
            bad_xml.push(format!("{}: {msg}", e.entry.name));
        }
    }
    report.push(
        "xml_parses",
        bad_xml.is_empty(),
        if bad_xml.is_empty() {
            format!(
                "{} XML parts parse",
                out_entries
                    .iter()
                    .filter(|e| !e.entry.is_dir && is_xml_name(&e.entry.name))
                    .count()
            )
        } else {
            bad_xml.join("; ")
        },
    );

    // 4. Every internal reference points at an entry that exists.
    let names_lower: HashSet<String> = out_entries
        .iter()
        .map(|e| e.entry.name.trim_end_matches('/').to_ascii_lowercase())
        .collect();
    let out_plain: Vec<Entry> = out_entries
        .iter()
        .map(|e| Entry {
            name: e.entry.name.clone(),
            bytes: e.data.clone(),
            is_dir: e.entry.is_dir,
        })
        .collect();
    let (refs, missing) = check_references(orig.kind, &out_plain, &names_lower);
    report.push(
        "references_resolve",
        missing.is_empty(),
        if missing.is_empty() {
            format!("{refs} internal references resolve")
        } else {
            format!("missing targets: {}", missing.join(", "))
        },
    );

    // 5. Replaced images decode as their declared format; nothing but media
    //    and the details parts changed.
    let orig_by_name: HashMap<&str, usize> = orig
        .entries
        .iter()
        .enumerate()
        .map(|(i, e)| (e.name.as_str(), i))
        .collect();
    let mut replaced = 0usize;
    let mut undecodable = Vec::new();
    let mut unexpected_changes = Vec::new();
    for e in &out_entries {
        let Some(&oi) = orig_by_name.get(e.entry.name.as_str()) else {
            continue; // already reported by entry_order
        };
        if orig.entries[oi].bytes == e.data {
            continue;
        }
        match orig.media_kind(oi) {
            Some(MediaKind::Image(fmt)) => {
                replaced += 1;
                if let Err(msg) = decodes_as(&e.data, fmt) {
                    undecodable.push(format!("{}: {msg}", e.entry.name));
                }
            }
            Some(_) => {
                replaced += 1;
            }
            None if DETAILS_PARTS.contains(&e.entry.name.as_str()) => {}
            None => unexpected_changes.push(e.entry.name.clone()),
        }
    }
    report.push(
        "replaced_images_decode",
        undecodable.is_empty(),
        if undecodable.is_empty() {
            format!("{replaced} media entries replaced, every image decodes")
        } else {
            undecodable.join("; ")
        },
    );
    report.push(
        "only_media_changed",
        unexpected_changes.is_empty(),
        if unexpected_changes.is_empty() {
            "document parts are byte-identical".to_owned()
        } else {
            format!("changed non-media parts: {}", unexpected_changes.join(", "))
        },
    );

    // 6. Size.
    match hard_bytes {
        Some(limit) => report.push(
            "size",
            output.len() as u64 <= limit,
            format!("{} bytes, limit {limit}", output.len()),
        ),
        None => report.push("size", true, format!("{} bytes, no limit", output.len())),
    }

    report
}

struct ReadEntry {
    entry: cia_archive::ArchiveEntry,
    method: String,
    data: Vec<u8>,
}

fn read_all_checked(output: &[u8]) -> Result<Vec<ReadEntry>, String> {
    let mut archive = cia_archive::open(output).map_err(|e| format!("does not open: {e}"))?;
    if archive.format() != cia_archive::ArchiveFormat::Zip {
        return Err(format!(
            "output is a {} archive, not a zip",
            archive.format().extension()
        ));
    }
    let mut out = Vec::with_capacity(archive.len());
    archive
        .for_each(|_, entry, data| {
            if !entry.is_dir && data.len() as u64 != entry.size {
                return Err(cia_archive::ArchiveError::Corrupt(format!(
                    "entry {} is {} bytes, header says {}",
                    entry.name,
                    data.len(),
                    entry.size
                )));
            }
            out.push(ReadEntry {
                entry: entry.clone(),
                method: entry.method.clone(),
                data: data.to_vec(),
            });
            Ok(())
        })
        .map_err(|e| e.to_string())?;
    Ok(out)
}

fn first_order_difference(expected: &[&str], found: &[&str]) -> String {
    if expected.len() != found.len() {
        return format!("{} entries expected, {} found", expected.len(), found.len());
    }
    for (i, (e, f)) in expected.iter().zip(found).enumerate() {
        if e != f {
            return format!("entry {i} is {f:?}, expected {e:?}");
        }
    }
    "order differs".to_owned()
}

/// Walk OOXML `.rels`, the ODF manifest and EPUB manifests; return the
/// number of internal references seen and the ones with no entry.
fn check_references(
    kind: OfficeKind,
    entries: &[Entry],
    names_lower: &HashSet<String>,
) -> (usize, Vec<String>) {
    let mut seen = 0usize;
    let mut missing = Vec::new();
    let mut check = |target: String, from: &str| {
        seen += 1;
        if !names_lower.contains(&target.to_ascii_lowercase()) {
            missing.push(format!("{target} (from {from})"));
        }
    };

    if kind.is_ooxml() {
        for e in entries {
            if e.is_dir || package::extension(&e.name) != "rels" {
                continue;
            }
            let base = rels_source_dir(&e.name);
            let _ = xml::for_each_element(&e.bytes, |el| {
                if el.local_name != "Relationship" {
                    return;
                }
                if el
                    .attr("TargetMode")
                    .is_some_and(|m| m.eq_ignore_ascii_case("External"))
                {
                    return;
                }
                if let Some(t) = el.attr("Target") {
                    if t.contains("://") || t.starts_with("mailto:") {
                        return;
                    }
                    check(xml::resolve(base, t), &e.name);
                }
            });
        }
    } else if kind.is_odf() {
        if let Some(m) = entries.iter().find(|e| e.name == "META-INF/manifest.xml") {
            let _ = xml::for_each_element(&m.bytes, |el| {
                if el.local_name != "file-entry" {
                    return;
                }
                if let Some(p) = el.attr("full-path") {
                    if p == "/" || p.ends_with('/') {
                        return;
                    }
                    check(xml::resolve("", p), "META-INF/manifest.xml");
                }
            });
        }
    } else {
        for opf_path in package::epub_rootfiles(entries) {
            check(opf_path.clone(), "META-INF/container.xml");
            let Some(opf) = entries.iter().find(|e| e.name == opf_path) else {
                continue;
            };
            let base = xml::dir_of(&opf_path);
            let _ = xml::for_each_element(&opf.bytes, |el| {
                if el.local_name != "item" {
                    return;
                }
                if let Some(href) = el.attr("href") {
                    if href.contains("://") {
                        return;
                    }
                    check(xml::resolve(base, href), &opf_path);
                }
            });
        }
    }
    (seen, missing)
}

/// `word/_rels/document.xml.rels` describes `word/document.xml`, so its
/// targets resolve against `word/`; `_rels/.rels` resolves against the root.
fn rels_source_dir(rels_name: &str) -> &str {
    match rels_name.find("_rels/") {
        Some(i) => &rels_name[..i],
        None => xml::dir_of(rels_name),
    }
}

fn decodes_as(bytes: &[u8], format: ImageFormat) -> Result<(), String> {
    let guessed = image::guess_format(bytes).map_err(|e| e.to_string())?;
    let expected = match format {
        ImageFormat::Jpeg => image::ImageFormat::Jpeg,
        ImageFormat::Png => image::ImageFormat::Png,
    };
    if guessed != expected {
        return Err(format!("is {guessed:?}, expected {expected:?}"));
    }
    image::load_from_memory_with_format(bytes, expected)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rels_source_dirs() {
        assert_eq!(rels_source_dir("_rels/.rels"), "");
        assert_eq!(rels_source_dir("word/_rels/document.xml.rels"), "word/");
        assert_eq!(
            rels_source_dir("ppt/slides/_rels/slide1.xml.rels"),
            "ppt/slides/"
        );
    }
}
