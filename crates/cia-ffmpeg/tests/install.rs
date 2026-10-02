//! Install path (DESIGN.md 4.8, 6.4) against a signed fake release: a
//! `file://` fetcher for the integrity checks, a tiny `TcpListener` HTTP
//! server for the real client behind the `download` feature.

mod support;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use cia_ffmpeg::install::{
    install_with, locate_with_override, platform_key, remove, sha256_hex, use_own_copy,
    verify_manifest, FileFetcher, InstallError, InstallProgress,
};
use cia_ffmpeg::pinned::{Pin, PUBLIC_KEY, VERSION};
use cia_ffmpeg::Cancel;
use ed25519_dalek::{Signer, SigningKey};
use std::io::Write;
use std::path::Path;

fn seed() -> [u8; 32] {
    let doc: serde_json::Value = serde_json::from_str(include_str!("../test-keys.json")).unwrap();
    let entry = &doc["keys"][0];
    assert_eq!(entry["kid"], "libraries-dev");
    STANDARD
        .decode(entry["seed_b64"].as_str().unwrap())
        .unwrap()
        .try_into()
        .unwrap()
}

fn sign(bytes: &[u8], seed: &[u8; 32]) -> Vec<u8> {
    SigningKey::from_bytes(seed).sign(bytes).to_bytes().to_vec()
}

/// A release zip: one top-level folder, fake `ffmpeg` / `ffprobe` that answer
/// `-version`, LICENSE.md and BUILD_CONFIG.txt. `working` false makes the
/// binaries unrunnable (the security-software case).
fn fake_zip(working: bool) -> Vec<u8> {
    use zip::write::SimpleFileOptions;
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut w = zip::ZipWriter::new(&mut buf);
        let top = format!("ffmpeg-{VERSION}-{}", platform_key().unwrap_or("test"));
        let exe = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .unix_permissions(0o755);
        let plain =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for name in ["ffmpeg", "ffprobe"] {
            let file = format!("{top}/{name}{}", if cfg!(windows) { ".exe" } else { "" });
            // An unrunnable binary has no exec bit (macOS would otherwise run a text file through sh).
            let opts = if working {
                exe
            } else {
                SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored)
                    .unix_permissions(0o644)
            };
            w.start_file(file, opts).unwrap();
            if working {
                w.write_all(
                    format!(
                        "#!/bin/sh\necho \"{name} version {VERSION}-fake Copyright (c) test\"\n"
                    )
                    .as_bytes(),
                )
                .unwrap();
            } else {
                // A shebang pointing at a missing interpreter fails to spawn on every Unix (ENOENT),
                // and a text .exe fails to spawn on Windows; both are the "blocked" case.
                w.write_all(b"#!/nonexistent/interpreter\nthis is not a program\n")
                    .unwrap();
            }
        }
        w.start_file(format!("{top}/LICENSE.md"), plain).unwrap();
        w.write_all(b"LGPL-3.0-or-later\n").unwrap();
        w.start_file(format!("{top}/BUILD_CONFIG.txt"), plain)
            .unwrap();
        w.write_all(b"--enable-version3\n").unwrap();
        w.finish().unwrap();
    }
    buf.into_inner()
}

fn manifest_json(version: &str, zip_url: &str, sha256: &str, bytes: u64) -> Vec<u8> {
    let key = platform_key().unwrap_or("test");
    serde_json::to_vec_pretty(&serde_json::json!({
        "schema": 1,
        "name": "ffmpeg",
        "version": version,
        "revision": 1,
        "license": "LGPL-3.0-or-later",
        "source": { "url": "https://example.invalid/source.tar.xz", "sha256": "00" },
        "assets": {
            key: { "url": zip_url, "sha256": sha256, "bytes": bytes, "unpacked_bytes": bytes * 2 }
        }
    }))
    .unwrap()
}

struct Release {
    dir: tempfile::TempDir,
}

impl Release {
    /// Write zip, manifest and signature into a folder, with `file://` URLs.
    fn new(zip: &[u8], version: &str, sha_override: Option<&str>, seed: &[u8; 32]) -> Release {
        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("ffmpeg.zip");
        std::fs::write(&zip_path, zip).unwrap();
        let sha = sha_override
            .map(str::to_string)
            .unwrap_or_else(|| sha256_hex(zip));
        let manifest = manifest_json(
            version,
            &format!("file://{}", zip_path.display()),
            &sha,
            zip.len() as u64,
        );
        std::fs::write(dir.path().join("manifest.json"), &manifest).unwrap();
        std::fs::write(dir.path().join("manifest.json.sig"), sign(&manifest, seed)).unwrap();
        Release { dir }
    }

    fn pin(&self) -> Pin {
        let base = format!("file://{}", self.dir.path().display());
        Pin {
            manifest_url: Box::leak(format!("{base}/manifest.json").into_boxed_str()),
            signature_url: Box::leak(format!("{base}/manifest.json.sig").into_boxed_str()),
            version: VERSION,
            public_key: PUBLIC_KEY,
        }
    }
}

fn run_install(
    pin: &Pin,
    app: &Path,
) -> (
    Result<cia_ffmpeg::InstalledFfmpeg, InstallError>,
    Vec<InstallProgress>,
) {
    let mut states = Vec::new();
    let result = install_with(
        &FileFetcher,
        pin,
        app,
        &mut |p| states.push(p),
        &Cancel::new(),
    );
    (result, states)
}

#[test]
fn manifest_signature_verifies_with_the_test_key() {
    let seed = seed();
    let manifest = manifest_json(VERSION, "https://example.invalid/x.zip", "ab", 1);
    let sig = sign(&manifest, &seed);
    let m = verify_manifest(&manifest, &sig, &PUBLIC_KEY).unwrap();
    assert_eq!(m.version, VERSION);
    assert_eq!(m.name, "ffmpeg");
    assert_eq!(m.assets.len(), 1);
    assert!(m.asset_for_this_platform().is_some() || platform_key().is_none());

    // hex and base64 signature files are accepted too
    let hex: String = sig.iter().map(|b| format!("{b:02x}")).collect();
    verify_manifest(&manifest, hex.as_bytes(), &PUBLIC_KEY).unwrap();
    verify_manifest(&manifest, STANDARD.encode(&sig).as_bytes(), &PUBLIC_KEY).unwrap();

    // tampered manifest
    let mut tampered = manifest.clone();
    let last = tampered.len() - 2;
    tampered[last] = b' ';
    assert!(matches!(
        verify_manifest(&tampered, &sig, &PUBLIC_KEY),
        Err(InstallError::BadSignature)
    ));
    // wrong key
    let other = SigningKey::from_bytes(&[9u8; 32]);
    let bad = other.sign(&manifest).to_bytes();
    assert!(matches!(
        verify_manifest(&manifest, &bad, &PUBLIC_KEY),
        Err(InstallError::BadSignature)
    ));
    // right signature, wrong schema
    let wrong_schema = manifest_json(VERSION, "u", "ab", 1).replace_schema();
    let sig2 = sign(&wrong_schema, &seed);
    assert!(matches!(
        verify_manifest(&wrong_schema, &sig2, &PUBLIC_KEY),
        Err(InstallError::BadManifest(_))
    ));
}

trait ReplaceSchema {
    fn replace_schema(self) -> Vec<u8>;
}
impl ReplaceSchema for Vec<u8> {
    fn replace_schema(self) -> Vec<u8> {
        String::from_utf8(self)
            .unwrap()
            .replace("\"schema\": 1", "\"schema\": 2")
            .into_bytes()
    }
}

#[cfg(unix)]
#[test]
fn install_from_file_urls_succeeds() {
    let seed = seed();
    let release = Release::new(&fake_zip(true), VERSION, None, &seed);
    let app = tempfile::tempdir().unwrap();
    let (result, states) = run_install(&release.pin(), app.path());
    let installed = result.expect("install");
    let dir = app.path().join("tools").join("ffmpeg").join(VERSION);
    assert_eq!(installed.install_dir.as_deref(), Some(dir.as_path()));
    assert_eq!(installed.ffmpeg, dir.join("ffmpeg"));
    assert_eq!(installed.ffprobe, dir.join("ffprobe"));
    assert_eq!(installed.version, format!("{VERSION}-fake"));
    assert!(!installed.user_supplied);
    for f in [
        "ffmpeg",
        "ffprobe",
        "LICENSE.md",
        "BUILD_CONFIG.txt",
        "manifest.json",
    ] {
        assert!(dir.join(f).is_file(), "{f} missing");
    }
    assert!(!app
        .path()
        .join("tools")
        .join("ffmpeg")
        .join(format!(".staging-{VERSION}"))
        .exists());
    assert!(states.contains(&InstallProgress::Checking));
    assert!(states.contains(&InstallProgress::Unpacking));
    assert!(states.contains(&InstallProgress::Testing));
    assert_eq!(states.last(), Some(&InstallProgress::Ready));
    assert!(states
        .iter()
        .any(|s| matches!(s, InstallProgress::Downloading { done, total: Some(t) } if done == t && *t > 0)));

    // locate finds it (without the env override), and its cache path sits inside
    let found = locate_with_override(app.path(), None).expect("locate");
    assert_eq!(found, installed);
    assert_eq!(
        found.encoders_cache_path(app.path()),
        dir.join("encoders.json")
    );

    // remove deletes the folder
    remove(&installed).unwrap();
    assert!(!dir.exists());
    assert!(locate_with_override(app.path(), None).is_none());
}

#[test]
fn hash_mismatch_installs_nothing() {
    let seed = seed();
    let release = Release::new(&fake_zip(true), VERSION, Some(&"0".repeat(64)), &seed);
    let app = tempfile::tempdir().unwrap();
    let (result, states) = run_install(&release.pin(), app.path());
    assert!(
        matches!(result, Err(InstallError::HashMismatch)),
        "{result:?}"
    );
    assert!(states.contains(&InstallProgress::Checking));
    assert!(!states.contains(&InstallProgress::Unpacking));
    assert!(!app
        .path()
        .join("tools")
        .join("ffmpeg")
        .join(VERSION)
        .exists());
    let err = result.unwrap_err();
    assert!(err.is_integrity_failure());
    assert_eq!(
        err.user_message(false),
        "The download was damaged, so Smidge deleted it. Please try again."
    );
}

#[test]
fn bad_signature_installs_nothing() {
    let release = Release::new(&fake_zip(true), VERSION, None, &[3u8; 32]);
    let app = tempfile::tempdir().unwrap();
    let (result, _) = run_install(&release.pin(), app.path());
    assert!(
        matches!(result, Err(InstallError::BadSignature)),
        "{result:?}"
    );
    assert!(
        !app.path().join("tools").exists()
            || std::fs::read_dir(app.path().join("tools").join("ffmpeg"))
                .map(|d| d.count() == 0)
                .unwrap_or(true)
    );
}

#[test]
fn version_pin_mismatch_is_rejected() {
    let seed = seed();
    let release = Release::new(&fake_zip(true), "9.9.9", None, &seed);
    let app = tempfile::tempdir().unwrap();
    let (result, _) = run_install(&release.pin(), app.path());
    assert!(
        matches!(result, Err(InstallError::BadManifest(_))),
        "{result:?}"
    );
}

#[test]
fn missing_release_is_a_network_error() {
    let pin = Pin {
        manifest_url: "file:///definitely/not/here/manifest.json",
        signature_url: "file:///definitely/not/here/manifest.json.sig",
        version: VERSION,
        public_key: PUBLIC_KEY,
    };
    let app = tempfile::tempdir().unwrap();
    let (result, _) = run_install(&pin, app.path());
    assert!(
        matches!(result, Err(InstallError::Network(_))),
        "{result:?}"
    );
    assert!(result
        .unwrap_err()
        .user_message(false)
        .starts_with("The download didn't finish."));
}

#[cfg(unix)]
#[test]
fn unrunnable_binary_is_the_blocked_case() {
    let seed = seed();
    let release = Release::new(&fake_zip(false), VERSION, None, &seed);
    let app = tempfile::tempdir().unwrap();
    let (result, states) = run_install(&release.pin(), app.path());
    assert!(
        matches!(result, Err(InstallError::Blocked(_))),
        "{result:?}"
    );
    assert!(states.contains(&InstallProgress::Testing));
    let tools = app.path().join("tools").join("ffmpeg");
    assert!(!tools.join(VERSION).exists());
    assert!(!tools.join(format!(".staging-{VERSION}")).exists());
    assert!(result
        .unwrap_err()
        .user_message(false)
        .starts_with("Your security software blocked FFmpeg."));
}

#[test]
fn cancel_before_download_stops_cleanly() {
    let seed = seed();
    let release = Release::new(&fake_zip(true), VERSION, None, &seed);
    let app = tempfile::tempdir().unwrap();
    let cancel = Cancel::new();
    cancel.cancel();
    let result = install_with(
        &FileFetcher,
        &release.pin(),
        app.path(),
        &mut |_| {},
        &cancel,
    );
    assert!(matches!(result, Err(InstallError::Cancelled)), "{result:?}");
    assert!(!app
        .path()
        .join("tools")
        .join("ffmpeg")
        .join(VERSION)
        .exists());
}

#[cfg(unix)]
#[test]
fn use_own_copy_wants_ffprobe_next_to_ffmpeg() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let ffmpeg = dir.path().join("ffmpeg");
    std::fs::write(
        &ffmpeg,
        "#!/bin/sh\necho \"ffmpeg version 6.0-own Copyright\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&ffmpeg, std::fs::Permissions::from_mode(0o755)).unwrap();
    let err = use_own_copy(&ffmpeg).unwrap_err();
    assert!(matches!(err, InstallError::NotWorking(_)), "{err:?}");

    let ffprobe = dir.path().join("ffprobe");
    std::fs::write(
        &ffprobe,
        "#!/bin/sh\necho \"ffprobe version 6.0-own Copyright\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&ffprobe, std::fs::Permissions::from_mode(0o755)).unwrap();
    let own = use_own_copy(&ffmpeg).unwrap();
    assert!(own.user_supplied);
    assert_eq!(own.version, "6.0-own");
    assert_eq!(own.install_dir, None);
    // pointing at the folder works too
    assert_eq!(use_own_copy(dir.path()).unwrap(), own);
    // a user copy's cache lives under tools/ffmpeg/external
    let app = tempfile::tempdir().unwrap();
    let cache = own.encoders_cache_path(app.path());
    assert!(cache.starts_with(app.path().join("tools").join("ffmpeg").join("external")));
    assert_eq!(cache.file_name().unwrap(), "encoders.json");
    // the override finds it and never touches it on remove
    assert_eq!(
        locate_with_override(app.path(), Some(ffmpeg.as_os_str())),
        Some(own.clone())
    );
    remove(&own).unwrap();
    assert!(ffmpeg.is_file());
}

#[test]
fn use_own_copy_with_the_real_ffmpeg_if_present() {
    let ff = need!(support::ffmpeg(), "no ffmpeg");
    assert!(ff.user_supplied);
    assert!(!ff.version.is_empty());
    assert!(ff.ffprobe.is_file());
}

/// Minimal HTTP/1.1 file server on 127.0.0.1 for the real client. Files can
/// be added after binding, so URLs that include the port can be signed.
#[cfg(all(unix, feature = "download"))]
mod http {
    use std::collections::HashMap;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    pub type Files = Arc<Mutex<HashMap<String, Vec<u8>>>>;

    pub fn serve() -> (String, Files) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let files: Files = Arc::new(Mutex::new(HashMap::new()));
        let shared = files.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let files = shared.clone();
                std::thread::spawn(move || {
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut request = String::new();
                    if reader.read_line(&mut request).is_err() {
                        return;
                    }
                    let mut line = String::new();
                    while reader.read_line(&mut line).is_ok() && line != "\r\n" && !line.is_empty()
                    {
                        line.clear();
                    }
                    let path = request.split_whitespace().nth(1).unwrap_or("/").to_string();
                    let body = files.lock().unwrap().get(&path).cloned();
                    match body {
                        Some(body) => {
                            let _ = write!(
                                stream,
                                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n",
                                body.len()
                            );
                            let _ = stream.write_all(&body);
                        }
                        None => {
                            let _ = write!(stream, "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                        }
                    }
                    let _ = stream.flush();
                    let mut sink = [0u8; 1];
                    let _ = stream.read(&mut sink);
                });
            }
        });
        (format!("http://{addr}"), files)
    }
}

#[cfg(all(unix, feature = "download"))]
#[test]
fn install_over_http_with_the_real_client() {
    use cia_ffmpeg::install::HttpFetcher;
    let seed = seed();
    let zip = fake_zip(true);
    let (base, files) = http::serve();
    let manifest = manifest_json(
        VERSION,
        &format!("{base}/ffmpeg.zip"),
        &sha256_hex(&zip),
        zip.len() as u64,
    );
    let sig = sign(&manifest, &seed);
    {
        let mut f = files.lock().unwrap();
        f.insert("/ffmpeg.zip".to_string(), zip);
        f.insert("/manifest.json".to_string(), manifest);
        f.insert("/manifest.json.sig".to_string(), sig);
    }
    let pin = Pin {
        manifest_url: Box::leak(format!("{base}/manifest.json").into_boxed_str()),
        signature_url: Box::leak(format!("{base}/manifest.json.sig").into_boxed_str()),
        version: VERSION,
        public_key: PUBLIC_KEY,
    };
    let app = tempfile::tempdir().unwrap();
    let mut states = Vec::new();
    let installed = install_with(
        &HttpFetcher::new().unwrap(),
        &pin,
        app.path(),
        &mut |p| states.push(p),
        &Cancel::new(),
    )
    .expect("http install");
    assert!(installed.ffmpeg.is_file());
    assert_eq!(installed.version, format!("{VERSION}-fake"));
    assert_eq!(states.last(), Some(&InstallProgress::Ready));
    assert!(states
        .iter()
        .any(|s| matches!(s, InstallProgress::Downloading { done, total: Some(_) } if *done > 0)));

    // 404 is a network error
    let missing = Pin {
        manifest_url: Box::leak(format!("{base}/nope.json").into_boxed_str()),
        signature_url: Box::leak(format!("{base}/nope.json.sig").into_boxed_str()),
        version: VERSION,
        public_key: PUBLIC_KEY,
    };
    let err = install_with(
        &HttpFetcher::new().unwrap(),
        &missing,
        app.path(),
        &mut |_| {},
        &Cancel::new(),
    )
    .unwrap_err();
    assert!(matches!(err, InstallError::Network(_)), "{err:?}");

    // A zip that was swapped on the server after signing fails the hash check.
    files
        .lock()
        .unwrap()
        .insert("/ffmpeg.zip".to_string(), fake_zip(false));
    let app2 = tempfile::tempdir().unwrap();
    let err = install_with(
        &HttpFetcher::new().unwrap(),
        &pin,
        app2.path(),
        &mut |_| {},
        &Cancel::new(),
    )
    .unwrap_err();
    assert!(matches!(err, InstallError::HashMismatch), "{err:?}");
}
