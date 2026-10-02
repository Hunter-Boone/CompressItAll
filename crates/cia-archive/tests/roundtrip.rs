//! Native integration tests for cia-archive: round trips, the stored/deflate
//! and zopfli decisions, exact size prediction, format detection, failure
//! reporting.

use std::io::Write;

use cia_archive::{
    open, verify, zip_predicted_size, ArchiveError, ArchiveFormat, ArchiveKind, ArchiveOptions,
    ArchiveWriter,
};
use cia_core::zip_overhead::zip_overhead_bytes;

const UNICODE_NAME: &str = "誕生日 🎂.txt";

fn fixture(name: &str) -> Vec<u8> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/");
    std::fs::read(format!("{path}{name}")).unwrap_or_else(|e| panic!("fixture {name}: {e}"))
}

/// Deterministic pseudo-random bytes (xorshift64*), incompressible.
fn noise(len: usize, seed: u64) -> Vec<u8> {
    let mut x = seed | 1;
    (0..len)
        .map(|_| {
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 56) as u8
        })
        .collect()
}

fn text(len: usize) -> Vec<u8> {
    let line = b"The quick brown fox jumps over the lazy dog. 0123456789\n";
    line.iter().copied().cycle().take(len).collect()
}

/// Small dictionary keeps the 7z tests fast; the default is exercised once.
fn test_options() -> ArchiveOptions {
    ArchiveOptions {
        sevenz_dict_bytes: 1 << 20,
        ..ArchiveOptions::default()
    }
}

fn three_entries() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("hello.txt", text(5000)),
        (
            UNICODE_NAME,
            "Happy birthday! 誕生日おめでとう 🎂\n"
                .repeat(40)
                .into_bytes(),
        ),
        ("empty.bin", Vec::new()),
    ]
}

fn build(kind: ArchiveKind, entries: &[(&str, Vec<u8>)], opts: ArchiveOptions) -> Vec<u8> {
    let mut w = ArchiveWriter::in_memory(kind, opts).unwrap();
    for (name, data) in entries {
        w.add_entry(name, data).unwrap();
    }
    w.finish().unwrap()
}

fn round_trip(kind: ArchiveKind, expected_format: ArchiveFormat) {
    let entries = three_entries();
    let bytes = build(kind, &entries, test_options());

    let mut ar = open(&bytes).unwrap();
    assert_eq!(ar.format(), expected_format, "{kind}");
    assert_eq!(ar.len(), 3);
    for (i, (name, data)) in entries.iter().enumerate() {
        let e = &ar.entries()[i];
        assert_eq!(e.name, *name);
        assert_eq!(e.size, data.len() as u64);
        assert!(!e.is_dir && !e.encrypted && e.supported, "{e:?}");
    }
    let all = ar.extract_all().unwrap();
    for (i, (_, data)) in entries.iter().enumerate() {
        assert_eq!(&all[i], data, "{kind} entry {i}");
        assert_eq!(ar.extract(i).unwrap(), *data);
    }

    let names: Vec<String> = entries.iter().map(|e| e.0.to_owned()).collect();
    let report = verify(&bytes, &names).unwrap();
    assert_eq!(report.entry_count, 3);
    assert_eq!(report.format, expected_format);
    assert_eq!(
        report.unpacked_bytes,
        entries.iter().map(|e| e.1.len() as u64).sum::<u64>()
    );
}

#[test]
fn round_trip_zip() {
    round_trip(ArchiveKind::Zip, ArchiveFormat::Zip);
}

#[test]
fn round_trip_7z() {
    round_trip(ArchiveKind::SevenZip, ArchiveFormat::SevenZip);
}

#[test]
fn round_trip_tar_zst() {
    round_trip(ArchiveKind::TarZst, ArchiveFormat::TarZst);
}

#[test]
fn round_trip_tar_xz() {
    round_trip(ArchiveKind::TarXz, ArchiveFormat::TarXz);
}

#[test]
fn seven_zip_default_dictionary_works() {
    // 64 MiB dictionary as the desktop uses it (DESIGN.md 3.9.2).
    let entries = vec![("a.txt", text(100_000))];
    let bytes = build(ArchiveKind::SevenZip, &entries, ArchiveOptions::default());
    let mut ar = open(&bytes).unwrap();
    assert_eq!(ar.entries()[0].method, "lzma2");
    assert_eq!(ar.extract(0).unwrap(), entries[0].1);
}

#[test]
fn streaming_writer_matches_in_memory() {
    let entries = three_entries();
    let in_memory = build(ArchiveKind::Zip, &entries, test_options());
    let mut sink = Vec::new();
    {
        let mut w = ArchiveWriter::new(ArchiveKind::Zip, test_options(), &mut sink).unwrap();
        for (name, data) in &entries {
            w.add_entry(name, data).unwrap();
        }
        w.finish().unwrap();
    }
    assert_eq!(sink, in_memory);
}

#[test]
fn directories_round_trip_everywhere() {
    for kind in [
        ArchiveKind::Zip,
        ArchiveKind::SevenZip,
        ArchiveKind::TarZst,
        ArchiveKind::TarXz,
    ] {
        let mut w = ArchiveWriter::in_memory(kind, test_options()).unwrap();
        w.add_dir("photos").unwrap();
        w.add_entry("photos/a.txt", b"aaa").unwrap();
        let bytes = w.finish().unwrap();
        let ar = open(&bytes).unwrap();
        let names: Vec<&str> = ar.entries().iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["photos/", "photos/a.txt"], "{kind}");
        assert!(ar.entries()[0].is_dir, "{kind}");
        verify(&bytes, &["photos/".to_owned(), "photos/a.txt".to_owned()]).unwrap();
    }
}

#[test]
fn zip_stored_when_deflate_saves_under_one_percent() {
    let entries = vec![("noise.bin", noise(20_000, 7)), ("text.txt", text(20_000))];
    let bytes = build(ArchiveKind::Zip, &entries, test_options());
    let ar = open(&bytes).unwrap();
    assert_eq!(ar.entries()[0].method, "stored");
    assert_eq!(ar.entries()[1].method, "deflate");

    // Cross-check with the zip crate directly: stored entry's compressed size equals its size.
    let mut za = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
    let f = za.by_index_raw(0).unwrap();
    assert_eq!(f.compressed_size(), f.size());
    assert_eq!(f.compression(), zip::CompressionMethod::Stored);
    drop(f);
    let f = za.by_index_raw(1).unwrap();
    assert!(f.compressed_size() < f.size() / 10);
    assert_eq!(f.compression(), zip::CompressionMethod::Deflated);
}

#[test]
fn zip_uses_zopfli_below_one_mib_and_zlib_above() {
    let small = text(300_000); // < 1 MiB -> zopfli
    let large = text(1_100_000); // >= 1 MiB -> zlib-rs level 9

    let zopfli_on = test_options();
    let zopfli_off = ArchiveOptions {
        zopfli_max_bytes: 0,
        ..test_options()
    };

    // Small entry: zopfli output differs from (and is no larger than) zlib level 9.
    let a = build(
        ArchiveKind::Zip,
        &[("s.txt", small.clone())],
        zopfli_on.clone(),
    );
    let b = build(
        ArchiveKind::Zip,
        &[("s.txt", small.clone())],
        zopfli_off.clone(),
    );
    assert_ne!(a, b, "small entry must take the zopfli path");
    assert!(a.len() <= b.len(), "zopfli {} vs zlib {}", a.len(), b.len());

    // Reference: zopfli with the default options produces exactly our deflate stream.
    let mut reference = Vec::new();
    zopfli::compress(
        zopfli::Options::default(),
        zopfli::Format::Deflate,
        &small[..],
        &mut reference,
    )
    .unwrap();
    let name_len = "s.txt".len();
    assert_eq!(
        &a[30 + name_len..30 + name_len + reference.len()],
        &reference[..]
    );

    // Large entry: identical bytes whether zopfli is enabled or not.
    let c = build(ArchiveKind::Zip, &[("l.txt", large.clone())], zopfli_on);
    let d = build(ArchiveKind::Zip, &[("l.txt", large)], zopfli_off);
    assert_eq!(c, d, "large entry must skip zopfli");
}

#[test]
fn zip_size_prediction_is_exact_for_stored_entries() {
    // Incompressible data forces Stored on every entry; the empty file is Stored by definition.
    let entries = vec![
        ("photo one.jpg", noise(40_000, 1)),
        (UNICODE_NAME, noise(12_345, 2)),
        ("sub/dir/empty.bin", Vec::new()),
        ("z.bin", noise(1, 3)),
    ];
    let bytes = build(ArchiveKind::Zip, &entries, test_options());

    let sized: Vec<(&str, u64)> = entries.iter().map(|e| (e.0, e.1.len() as u64)).collect();
    let predicted = zip_predicted_size(&sized);
    let data: u64 = sized.iter().map(|e| e.1).sum();
    let largest = sized.iter().map(|e| e.1).max().unwrap();
    let overhead = zip_overhead_bytes(sized.iter().map(|e| e.0), largest);

    assert_eq!(predicted, data + overhead, "predictor agrees with cia-core");
    assert_eq!(
        bytes.len() as u64,
        predicted,
        "real archive equals prediction"
    );

    let ar = open(&bytes).unwrap();
    assert!(
        ar.entries().iter().all(|e| e.method == "stored"),
        "{:?}",
        ar.entries()
    );
}

#[test]
fn zip_deflated_archive_never_exceeds_prediction() {
    let entries = three_entries();
    let bytes = build(ArchiveKind::Zip, &entries, test_options());
    let sized: Vec<(&str, u64)> = entries.iter().map(|e| (e.0, e.1.len() as u64)).collect();
    assert!(bytes.len() as u64 <= zip_predicted_size(&sized));
}

#[test]
fn zip_sets_utf8_flag_and_no_extra_fields() {
    let bytes = build(
        ArchiveKind::Zip,
        &[("a.txt", b"abc".to_vec())],
        test_options(),
    );
    // Local header: flags at 6, extra length at 28.
    assert_eq!(&bytes[0..4], b"PK\x03\x04");
    let flags = u16::from_le_bytes([bytes[6], bytes[7]]);
    assert_ne!(flags & (1 << 11), 0, "language encoding flag (bit 11)");
    assert_eq!(flags & (1 << 3), 0, "no data descriptor");
    assert_eq!(
        u16::from_le_bytes([bytes[28], bytes[29]]),
        0,
        "no local extra field"
    );
    // Central directory entry follows the 3 data bytes.
    let cd = 30 + 5 + 3;
    assert_eq!(&bytes[cd..cd + 4], b"PK\x01\x02");
    let cd_flags = u16::from_le_bytes([bytes[cd + 8], bytes[cd + 9]]);
    assert_ne!(cd_flags & (1 << 11), 0);
    assert_eq!(
        u16::from_le_bytes([bytes[cd + 30], bytes[cd + 31]]),
        0,
        "no central extra field"
    );
    assert_eq!(
        u16::from_le_bytes([bytes[cd + 32], bytes[cd + 33]]),
        0,
        "no comment"
    );
}

#[test]
fn zip_from_7zip_opens() {
    let bytes = fixture("plain_7zip.zip");
    let mut ar = open(&bytes).unwrap();
    assert_eq!(ar.format(), ArchiveFormat::Zip);
    // 7-Zip orders entries its own way; look them up by name.
    let mut names: Vec<&str> = ar.entries().iter().map(|e| e.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, ["empty.bin", "hello.txt"]);
    let hello = ar
        .entries()
        .iter()
        .position(|e| e.name == "hello.txt")
        .unwrap();
    let empty = ar
        .entries()
        .iter()
        .position(|e| e.name == "empty.bin")
        .unwrap();
    assert_eq!(ar.extract(hello).unwrap(), b"hello hello hello hello\n");
    assert!(ar.extract(empty).unwrap().is_empty());
}

fn tar_bytes() -> Vec<u8> {
    let mut b = tar::Builder::new(Vec::new());
    for (name, data) in three_entries() {
        let mut h = tar::Header::new_ustar();
        h.set_size(data.len() as u64);
        h.set_mode(0o644);
        h.set_mtime(0);
        h.set_entry_type(tar::EntryType::Regular);
        b.append_data(&mut h, name, &data[..]).unwrap();
    }
    b.into_inner().unwrap()
}

fn assert_three(bytes: &[u8], format: ArchiveFormat) {
    let mut ar = open(bytes).unwrap();
    assert_eq!(ar.format(), format);
    let entries = three_entries();
    assert_eq!(ar.len(), 3);
    for (i, (name, data)) in entries.iter().enumerate() {
        assert_eq!(ar.entries()[i].name, *name);
        assert_eq!(ar.extract(i).unwrap(), *data, "{format:?} entry {i}");
    }
    let names: Vec<String> = entries.iter().map(|e| e.0.to_owned()).collect();
    verify(bytes, &names).unwrap();
}

#[test]
fn detects_plain_tar() {
    assert_three(&tar_bytes(), ArchiveFormat::Tar);
}

#[test]
fn detects_tar_gz() {
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    enc.write_all(&tar_bytes()).unwrap();
    assert_three(&enc.finish().unwrap(), ArchiveFormat::TarGz);
}

#[test]
fn detects_tar_bz2() {
    let mut enc = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::best());
    enc.write_all(&tar_bytes()).unwrap();
    assert_three(&enc.finish().unwrap(), ArchiveFormat::TarBz2);
}

#[test]
fn detects_tar_xz_from_the_xz_tool() {
    let bytes = fixture("ustar.tar.xz");
    let mut ar = open(&bytes).unwrap();
    assert_eq!(ar.format(), ArchiveFormat::TarXz);
    let names: Vec<&str> = ar.entries().iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["hello.txt", "empty.bin"]);
    assert_eq!(ar.extract(0).unwrap(), b"hello hello hello hello\n");
}

#[test]
fn detects_tar_zst_and_tar_xz_from_our_writers() {
    let entries = three_entries();
    let zst = build(ArchiveKind::TarZst, &entries, test_options());
    assert_eq!(open(&zst).unwrap().format(), ArchiveFormat::TarZst);
    let xz = build(ArchiveKind::TarXz, &entries, test_options());
    assert_eq!(open(&xz).unwrap().format(), ArchiveFormat::TarXz);
}

#[test]
fn plain_gz_is_one_entry_named_from_the_header() {
    let payload = text(3000);
    let mut enc = flate2::GzBuilder::new()
        .filename("server.log")
        .write(Vec::new(), flate2::Compression::default());
    enc.write_all(&payload).unwrap();
    let bytes = enc.finish().unwrap();

    let mut ar = open(&bytes).unwrap();
    assert_eq!(ar.format(), ArchiveFormat::Gz);
    assert_eq!(ar.len(), 1);
    assert_eq!(ar.entries()[0].name, "server.log");
    assert_eq!(ar.entries()[0].method, "gzip");
    assert_eq!(ar.entries()[0].size, payload.len() as u64);
    assert_eq!(ar.extract(0).unwrap(), payload);
    verify(&bytes, &["server.log".to_owned()]).unwrap();

    // Without a name in the header the single entry is called "data".
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(b"x").unwrap();
    let unnamed = enc.finish().unwrap();
    let ar = open(&unnamed).unwrap();
    assert_eq!(ar.entries()[0].name, "data");
}

#[test]
fn plain_zst_and_bz2_are_single_entries() {
    let payload = text(2000);
    let zst = zstd::encode_all(&payload[..], 3).unwrap();
    let mut ar = open(&zst).unwrap();
    assert_eq!(ar.format(), ArchiveFormat::Zst);
    assert_eq!(ar.extract(0).unwrap(), payload);

    let mut enc = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::default());
    enc.write_all(&payload).unwrap();
    let bz = enc.finish().unwrap();
    let mut ar = open(&bz).unwrap();
    assert_eq!(ar.format(), ArchiveFormat::Bz2);
    assert_eq!(ar.extract(0).unwrap(), payload);
}

#[test]
fn corrupted_zip_byte_fails_verify() {
    let entries = vec![("text.txt", text(10_000)), ("b.txt", text(100))];
    let mut bytes = build(ArchiveKind::Zip, &entries, test_options());
    let names: Vec<String> = entries.iter().map(|e| e.0.to_owned()).collect();
    verify(&bytes, &names).unwrap();

    // Flip a byte well inside the first entry's deflate stream.
    let pos = 30 + "text.txt".len() + 40;
    bytes[pos] ^= 0xFF;
    let err = verify(&bytes, &names).unwrap_err();
    assert!(matches!(err, ArchiveError::Corrupt(_)), "{err:?}");
}

#[test]
fn corrupted_7z_and_tar_zst_fail_verify() {
    let entries = vec![("text.txt", text(10_000))];
    let names = vec!["text.txt".to_owned()];
    for kind in [
        ArchiveKind::SevenZip,
        ArchiveKind::TarZst,
        ArchiveKind::TarXz,
    ] {
        let mut bytes = build(kind, &entries, test_options());
        verify(&bytes, &names).unwrap();
        let pos = bytes.len() / 2;
        bytes[pos] ^= 0x55;
        let res = verify(&bytes, &names);
        assert!(res.is_err(), "{kind}: corrupted archive verified");
    }
}

#[test]
fn verify_checks_names_and_count() {
    let bytes = build(ArchiveKind::Zip, &three_entries(), test_options());
    let err = verify(&bytes, &["hello.txt".to_owned()]).unwrap_err();
    assert!(matches!(
        err,
        ArchiveError::CountMismatch {
            expected: 1,
            found: 3
        }
    ));
    let err = verify(
        &bytes,
        &[
            "hello.txt".to_owned(),
            "wrong.txt".to_owned(),
            "empty.bin".to_owned(),
        ],
    )
    .unwrap_err();
    assert!(
        matches!(err, ArchiveError::NameMismatch { index: 1, .. }),
        "{err:?}"
    );
}

#[test]
fn hand_crafted_encrypted_zip_entry_is_listed_not_fatal() {
    let entries = vec![("open.txt", text(500)), ("locked.txt", text(500))];
    let mut bytes = build(ArchiveKind::Zip, &entries, test_options());

    // Set general-purpose flag bit 0 on the second entry, in its local header and
    // its central directory record.
    let locals: Vec<usize> = bytes
        .windows(4)
        .enumerate()
        .filter(|(_, w)| *w == b"PK\x03\x04")
        .map(|(i, _)| i)
        .collect();
    let centrals: Vec<usize> = bytes
        .windows(4)
        .enumerate()
        .filter(|(_, w)| *w == b"PK\x01\x02")
        .map(|(i, _)| i)
        .collect();
    assert_eq!(locals.len(), 2);
    assert_eq!(centrals.len(), 2);
    bytes[locals[1] + 6] |= 1;
    bytes[centrals[1] + 8] |= 1;

    let mut ar = open(&bytes).expect("archive still opens");
    assert_eq!(ar.len(), 2);
    assert!(!ar.entries()[0].encrypted);
    assert!(ar.entries()[1].encrypted);
    assert!(!ar.entries()[1].supported);
    assert_eq!(ar.entries()[1].name, "locked.txt");
    assert_eq!(ar.extract(0).unwrap(), entries[0].1);
    let err = ar.extract(1).unwrap_err();
    assert!(
        matches!(err, ArchiveError::Encrypted(ref n) if n == "locked.txt"),
        "{err:?}"
    );
    let names: Vec<String> = entries.iter().map(|e| e.0.to_owned()).collect();
    assert!(matches!(
        verify(&bytes, &names),
        Err(ArchiveError::Encrypted(_))
    ));
}

#[test]
fn zipcrypto_zip_from_7zip_lists_encrypted_entries() {
    let bytes = fixture("zipcrypto.zip");
    let mut ar = open(&bytes).unwrap();
    assert_eq!(ar.len(), 2);
    for e in ar.entries() {
        assert!(e.encrypted, "{e:?}");
        assert!(!e.supported);
    }
    assert!(matches!(ar.extract(0), Err(ArchiveError::Encrypted(_))));
}

#[test]
fn aes_7z_lists_encrypted_entries() {
    let bytes = fixture("aes_header_plain.7z");
    let mut ar = open(&bytes).unwrap();
    assert_eq!(ar.format(), ArchiveFormat::SevenZip);
    assert_eq!(ar.len(), 2);
    for e in ar.entries() {
        assert!(e.encrypted, "{e:?}");
        assert!(e.method.contains("aes"), "{e:?}");
        assert!(!e.supported);
    }
    assert!(matches!(ar.extract(0), Err(ArchiveError::Encrypted(_))));
}

#[test]
fn bzip2_7z_reports_unsupported_method_per_entry() {
    let bytes = fixture("bzip2_method.7z");
    let mut ar = open(&bytes).unwrap();
    assert_eq!(ar.len(), 1);
    let e = &ar.entries()[0];
    assert_eq!(e.method, "bzip2");
    assert!(!e.encrypted);
    assert!(!e.supported);
    assert!(matches!(
        ar.extract(0),
        Err(ArchiveError::UnsupportedMethod { .. })
    ));
}

#[test]
fn rar_magic_is_unsupported() {
    let mut bytes = b"Rar!\x1A\x07\x01\x00".to_vec();
    bytes.extend_from_slice(&[0u8; 600]);
    let err = open(&bytes).unwrap_err();
    assert!(
        matches!(err, ArchiveError::Unsupported(ref s) if s == "rar"),
        "{err:?}"
    );

    let mut v4 = b"Rar!\x1A\x07\x00".to_vec();
    v4.extend_from_slice(&[0u8; 600]);
    assert!(matches!(open(&v4), Err(ArchiveError::Unsupported(ref s)) if s == "rar"));
}

#[test]
fn garbage_is_unsupported_not_a_panic() {
    assert!(matches!(open(b"hello"), Err(ArchiveError::Unsupported(_))));
    assert!(matches!(
        open(&[0u8; 2048]),
        Err(ArchiveError::Unsupported(_))
    ));
    assert!(matches!(open(b""), Err(ArchiveError::Unsupported(_))));
}

#[test]
fn bad_entry_names_are_rejected() {
    let mut w = ArchiveWriter::in_memory(ArchiveKind::Zip, test_options()).unwrap();
    for bad in [
        "",
        "/abs",
        "../up",
        "a/../b",
        "c:\\win",
        "nul\0byte",
        "back\\slash",
        "trail/",
    ] {
        assert!(
            matches!(w.add_entry(bad, b"x"), Err(ArchiveError::InvalidName(_))),
            "{bad:?} accepted"
        );
    }
    w.add_entry("ok/fine.txt", b"x").unwrap();
    assert_eq!(w.len(), 1);
}

#[test]
fn unpack_limit_is_enforced() {
    let bytes = build(
        ArchiveKind::Zip,
        &[("big.txt", text(50_000))],
        test_options(),
    );
    let mut ar = cia_archive::open_with_limit(&bytes, 1000).unwrap();
    assert!(matches!(
        ar.extract(0),
        Err(ArchiveError::TooLarge { limit: 1000 })
    ));

    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(&text(50_000)).unwrap();
    let gz = enc.finish().unwrap();
    assert!(matches!(
        cia_archive::open_with_limit(&gz, 1000),
        Err(ArchiveError::TooLarge { limit: 1000 })
    ));
}

#[test]
fn seven_zip_keeps_order_with_empty_entries_interleaved() {
    let entries = vec![
        ("0-empty.bin", Vec::new()),
        ("1-a.txt", text(1000)),
        ("2-b.txt", text(2000)),
        ("3-empty.bin", Vec::new()),
        ("4-c.txt", text(3000)),
        ("5-empty.bin", Vec::new()),
    ];
    let bytes = build(ArchiveKind::SevenZip, &entries, test_options());
    let mut ar = open(&bytes).unwrap();
    let names: Vec<&str> = ar.entries().iter().map(|e| e.name.as_str()).collect();
    let expected: Vec<&str> = entries.iter().map(|e| e.0).collect();
    assert_eq!(names, expected);
    let all = ar.extract_all().unwrap();
    for (i, (_, data)) in entries.iter().enumerate() {
        assert_eq!(&all[i], data, "entry {i}");
        assert_eq!(ar.extract(i).unwrap(), *data, "entry {i}");
    }
    verify(
        &bytes,
        &entries.iter().map(|e| e.0.to_owned()).collect::<Vec<_>>(),
    )
    .unwrap();
}
