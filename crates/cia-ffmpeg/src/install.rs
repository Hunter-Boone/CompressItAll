//! Download, verify and unpack the pinned FFmpeg build (DESIGN.md 4.8, 6.4),
//! or adopt the user's own copy.
//!
//! Order of checks: Ed25519 signature over the exact manifest bytes, then
//! the zip's SHA-256 from the manifest, then unpack into a staging folder
//! that is renamed to `<app_data>/tools/ffmpeg/<version>/` only once
//! `ffmpeg -version` has run from it. The HTTP client lives behind the
//! `download` feature; [`install_with`] takes any [`Fetcher`] so the whole
//! path is testable with a local server or `file://` URLs.

use crate::cancel::Cancel;
use crate::command::version_args;
use crate::pinned::Pin;
use crate::process::run_capture;
use crate::{exe_name, is_disk_full, parse_version, FfmpegError, InstalledFfmpeg};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The 4.8 error cases, plus the ones that only matter for logs.
#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("download failed: {0}")]
    Network(String),
    #[error("cancelled")]
    Cancelled,
    #[error("manifest signature does not verify")]
    BadSignature,
    #[error("manifest is not usable: {0}")]
    BadManifest(String),
    #[error("downloaded archive does not match the manifest hash")]
    HashMismatch,
    #[error("no FFmpeg build for this platform: {0}")]
    UnsupportedPlatform(String),
    #[error("not enough free space")]
    DiskFull,
    #[error("ffmpeg vanished or cannot run after unpacking: {0}")]
    Blocked(String),
    #[error("ffmpeg does not work: {0}")]
    NotWorking(String),
    #[error("archive is not usable: {0}")]
    BadArchive(String),
    #[error("{0}")]
    Io(std::io::Error),
}

impl From<std::io::Error> for InstallError {
    fn from(e: std::io::Error) -> Self {
        if is_disk_full(&e) {
            InstallError::DiskFull
        } else {
            InstallError::Io(e)
        }
    }
}

impl InstallError {
    /// Exact copy from DESIGN.md 4.8. `repeat_mismatch` is the second hash
    /// or signature failure in a row.
    pub fn user_message(&self, repeat_mismatch: bool) -> String {
        match self {
            InstallError::Network(_) | InstallError::Cancelled => {
                "The download didn't finish. Check your internet connection and try again."
                    .to_string()
            }
            InstallError::BadSignature
            | InstallError::HashMismatch
            | InstallError::BadManifest(_)
            | InstallError::BadArchive(_) => {
                if repeat_mismatch {
                    "Smidge couldn't get a good copy of FFmpeg. Contact support and we'll help."
                        .to_string()
                } else {
                    "The download was damaged, so Smidge deleted it. Please try again.".to_string()
                }
            }
            InstallError::DiskFull => {
                "There isn't enough free space. FFmpeg needs about 90 MB.".to_string()
            }
            InstallError::Blocked(_) => {
                "Your security software blocked FFmpeg. You can allow it, or use your own copy."
                    .to_string()
            }
            InstallError::UnsupportedPlatform(_) => {
                "There is no FFmpeg build for this computer yet. You can use your own copy."
                    .to_string()
            }
            InstallError::NotWorking(_) | InstallError::Io(_) => {
                "Smidge couldn't set up FFmpeg. Please try again, or use your own copy.".to_string()
            }
        }
    }

    /// Whether this is the "damaged download" family that counts toward the
    /// two-in-a-row support message.
    pub fn is_integrity_failure(&self) -> bool {
        matches!(
            self,
            InstallError::BadSignature
                | InstallError::HashMismatch
                | InstallError::BadManifest(_)
                | InstallError::BadArchive(_)
        )
    }
}

/// `manifest.json` from the Smidge-Libraries release (6.4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub schema: u32,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub revision: u32,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub source: Option<ManifestSource>,
    pub assets: BTreeMap<String, ManifestAsset>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManifestSource {
    pub url: String,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManifestAsset {
    pub url: String,
    pub sha256: String,
    #[serde(default)]
    pub bytes: u64,
    #[serde(default)]
    pub unpacked_bytes: u64,
}

impl Manifest {
    /// The asset for this machine; None when the manifest has none.
    pub fn asset_for_this_platform(&self) -> Option<&ManifestAsset> {
        self.assets.get(platform_key()?)
    }
}

/// The 4.8 modal states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallProgress {
    Downloading { done: u64, total: Option<u64> },
    Checking,
    Unpacking,
    Testing,
    Ready,
}

/// Gets bytes from a URL. [`HttpFetcher`] for the app; tests bring their own.
pub trait Fetcher {
    fn fetch(
        &self,
        url: &str,
        on_progress: &mut dyn FnMut(u64, Option<u64>),
        cancel: &Cancel,
    ) -> Result<Vec<u8>, InstallError>;
}

/// Reads `file://` URLs and plain paths; for tests and offline installs.
pub struct FileFetcher;

impl Fetcher for FileFetcher {
    fn fetch(
        &self,
        url: &str,
        on_progress: &mut dyn FnMut(u64, Option<u64>),
        _cancel: &Cancel,
    ) -> Result<Vec<u8>, InstallError> {
        let path = url.strip_prefix("file://").unwrap_or(url);
        let bytes = std::fs::read(path).map_err(|e| InstallError::Network(e.to_string()))?;
        on_progress(bytes.len() as u64, Some(bytes.len() as u64));
        Ok(bytes)
    }
}

/// The manifest asset key for this build.
pub fn platform_key() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => Some("windows-x86_64"),
        ("macos", "aarch64") => Some("macos-aarch64"),
        ("macos", "x86_64") => Some("macos-x86_64"),
        ("linux", "x86_64") => Some("linux-x86_64"),
        _ => None,
    }
}

/// `<app_data>/tools/ffmpeg`.
pub fn tools_dir(app_data: &Path) -> PathBuf {
    app_data.join("tools").join("ffmpeg")
}

/// `<app_data>/tools/ffmpeg/<version>`.
pub fn install_dir(app_data: &Path, version: &str) -> PathBuf {
    tools_dir(app_data).join(version)
}

/// Lower-case hex SHA-256.
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A `.sig` file: 64 raw bytes, or 128 hex characters, or base64 text.
pub fn parse_signature(bytes: &[u8]) -> Result<[u8; 64], InstallError> {
    if bytes.len() == 64 {
        return Ok(bytes.try_into().expect("length checked"));
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|_| InstallError::BadSignature)?
        .trim();
    if text.len() == 128 && text.chars().all(|c| c.is_ascii_hexdigit()) {
        let mut out = [0u8; 64];
        for (i, chunk) in text.as_bytes().chunks(2).enumerate() {
            let s = std::str::from_utf8(chunk).map_err(|_| InstallError::BadSignature)?;
            out[i] = u8::from_str_radix(s, 16).map_err(|_| InstallError::BadSignature)?;
        }
        return Ok(out);
    }
    let decoded = STANDARD
        .decode(text)
        .map_err(|_| InstallError::BadSignature)?;
    decoded.try_into().map_err(|_| InstallError::BadSignature)
}

/// Check the Ed25519 signature over the exact manifest bytes, then parse.
pub fn verify_manifest(
    manifest_bytes: &[u8],
    signature: &[u8],
    public_key: &[u8; 32],
) -> Result<Manifest, InstallError> {
    let sig = ed25519_dalek::Signature::from_bytes(&parse_signature(signature)?);
    let key = ed25519_dalek::VerifyingKey::from_bytes(public_key)
        .map_err(|_| InstallError::BadSignature)?;
    key.verify_strict(manifest_bytes, &sig)
        .map_err(|_| InstallError::BadSignature)?;
    let manifest: Manifest = serde_json::from_slice(manifest_bytes)
        .map_err(|e| InstallError::BadManifest(e.to_string()))?;
    if manifest.schema != 1 {
        return Err(InstallError::BadManifest(format!(
            "schema {} is not 1",
            manifest.schema
        )));
    }
    if manifest.name != "ffmpeg" {
        return Err(InstallError::BadManifest(format!(
            "name {:?} is not ffmpeg",
            manifest.name
        )));
    }
    Ok(manifest)
}

/// Download, verify, unpack and test the pinned FFmpeg. Returns the
/// installed copy; on any error nothing is left in `<version>/`.
pub fn install_with(
    fetcher: &dyn Fetcher,
    pin: &Pin,
    app_data: &Path,
    progress: &mut dyn FnMut(InstallProgress),
    cancel: &Cancel,
) -> Result<InstalledFfmpeg, InstallError> {
    let check = |cancel: &Cancel| {
        if cancel.is_cancelled() {
            Err(InstallError::Cancelled)
        } else {
            Ok(())
        }
    };

    progress(InstallProgress::Downloading {
        done: 0,
        total: None,
    });
    let manifest_bytes = fetcher.fetch(&mirror_url(pin.manifest_url), &mut |_, _| {}, cancel)?;
    let signature = fetcher.fetch(&mirror_url(pin.signature_url), &mut |_, _| {}, cancel)?;
    check(cancel)?;
    let manifest = verify_manifest(&manifest_bytes, &signature, &pin.public_key)?;
    if manifest.version != pin.version {
        return Err(InstallError::BadManifest(format!(
            "manifest is for {} but this build pins {}",
            manifest.version, pin.version
        )));
    }
    let key =
        platform_key().ok_or_else(|| InstallError::UnsupportedPlatform(platform_description()))?;
    let asset = manifest
        .assets
        .get(key)
        .ok_or_else(|| InstallError::UnsupportedPlatform(key.to_string()))?;

    let total = (asset.bytes > 0).then_some(asset.bytes);
    progress(InstallProgress::Downloading { done: 0, total });
    let zip_bytes = fetcher.fetch(
        &mirror_url(&asset.url),
        &mut |done, reported| {
            progress(InstallProgress::Downloading {
                done,
                total: total.or(reported),
            });
        },
        cancel,
    )?;
    check(cancel)?;

    progress(InstallProgress::Checking);
    if !sha256_hex(&zip_bytes).eq_ignore_ascii_case(asset.sha256.trim()) {
        drop(zip_bytes);
        return Err(InstallError::HashMismatch);
    }

    progress(InstallProgress::Unpacking);
    let tools = tools_dir(app_data);
    std::fs::create_dir_all(&tools)?;
    let staging = tools.join(format!(".staging-{}", manifest.version));
    let _ = std::fs::remove_dir_all(&staging);
    let result = (|| -> Result<InstalledFfmpeg, InstallError> {
        std::fs::create_dir_all(&staging)?;
        unpack_zip(&zip_bytes, &staging)?;
        std::fs::write(staging.join("manifest.json"), &manifest_bytes)?;
        check(cancel)?;
        progress(InstallProgress::Testing);
        let mut installed = verify_binary(&staging)?;
        let final_dir = install_dir(app_data, &manifest.version);
        let _ = std::fs::remove_dir_all(&final_dir);
        std::fs::rename(&staging, &final_dir)?;
        installed.ffmpeg = final_dir.join(exe_name("ffmpeg"));
        installed.ffprobe = final_dir.join(exe_name("ffprobe"));
        installed.install_dir = Some(final_dir);
        Ok(installed)
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&staging);
    }
    let installed = result?;
    progress(InstallProgress::Ready);
    Ok(installed)
}

fn platform_description() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

/// Unpack a release zip into `dest`, flattening a single top-level folder,
/// refusing paths that escape `dest`, and marking the binaries executable.
pub fn unpack_zip(bytes: &[u8], dest: &Path) -> Result<(), InstallError> {
    let reader = std::io::Cursor::new(bytes);
    let mut archive =
        zip::ZipArchive::new(reader).map_err(|e| InstallError::BadArchive(e.to_string()))?;
    if archive.is_empty() {
        return Err(InstallError::BadArchive("empty archive".to_string()));
    }

    // Does every entry share one top-level directory?
    let mut top: Option<String> = None;
    let mut flatten = true;
    for i in 0..archive.len() {
        let entry = archive
            .by_index_raw(i)
            .map_err(|e| InstallError::BadArchive(e.to_string()))?;
        let Some(name) = entry.enclosed_name() else {
            return Err(InstallError::BadArchive(format!(
                "unsafe entry name {:?}",
                entry.name()
            )));
        };
        let mut comps = name.components();
        let first = comps
            .next()
            .map(|c| c.as_os_str().to_string_lossy().into_owned());
        let has_more = comps.next().is_some();
        match (first, has_more) {
            (Some(f), true) if top.as_ref().is_none_or(|t| *t == f) => top = Some(f),
            _ => flatten = false,
        }
    }
    let strip = if flatten { top } else { None };

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| InstallError::BadArchive(e.to_string()))?;
        let Some(rel) = entry.enclosed_name() else {
            return Err(InstallError::BadArchive(format!(
                "unsafe entry name {:?}",
                entry.name()
            )));
        };
        let rel: PathBuf = match &strip {
            Some(top) => rel.strip_prefix(top).map(Path::to_path_buf).unwrap_or(rel),
            None => rel,
        };
        if rel.as_os_str().is_empty() {
            continue;
        }
        let target = dest.join(&rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&target)?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = std::fs::File::create(&target)?;
        let mut buf = [0u8; 64 * 1024];
        loop {
            let n = entry
                .read(&mut buf)
                .map_err(|e| InstallError::BadArchive(e.to_string()))?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n])?;
        }
        file.flush()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let name = rel.file_name().map(|n| n.to_string_lossy().into_owned());
            if matches!(name.as_deref(), Some("ffmpeg" | "ffprobe")) {
                std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755))?;
            }
        }
    }
    Ok(())
}

/// Run `ffmpeg -version` and `ffprobe -version` from `dir`. A binary that
/// is missing or cannot be executed right after unpacking is the
/// security-software case.
pub fn verify_binary(dir: &Path) -> Result<InstalledFfmpeg, InstallError> {
    let ffmpeg = dir.join(exe_name("ffmpeg"));
    let ffprobe = dir.join(exe_name("ffprobe"));
    for bin in [&ffmpeg, &ffprobe] {
        if !bin.is_file() {
            return Err(InstallError::Blocked(format!(
                "{} is missing",
                bin.display()
            )));
        }
    }
    let version = version_of(&ffmpeg)?;
    version_of(&ffprobe)?;
    Ok(InstalledFfmpeg {
        ffmpeg,
        ffprobe,
        version,
        user_supplied: false,
        install_dir: Some(dir.to_path_buf()),
    })
}

/// `-version` first token; spawn failures are the "blocked" case.
fn version_of(bin: &Path) -> Result<String, InstallError> {
    match run_capture(bin, version_args(), Duration::from_secs(20), None) {
        Ok(c) if c.success() => {
            let out = c.stdout_str();
            Ok(parse_version(&out)
                .unwrap_or_else(|| out.lines().next().unwrap_or("unknown").trim().to_string()))
        }
        Ok(c) => Err(InstallError::NotWorking(format!(
            "{} exited with {}: {}",
            bin.display(),
            c.code(),
            c.stderr_str().trim()
        ))),
        Err(FfmpegError::Spawn(e)) => Err(InstallError::Blocked(format!("{}: {e}", bin.display()))),
        Err(e) => Err(InstallError::NotWorking(e.to_string())),
    }
}

/// Adopt the user's own `ffmpeg` (a file picker result, or its folder).
/// `ffprobe` must sit next to it.
pub fn use_own_copy(path: &Path) -> Result<InstalledFfmpeg, InstallError> {
    let ffmpeg = if path.is_dir() {
        path.join(exe_name("ffmpeg"))
    } else {
        path.to_path_buf()
    };
    if !ffmpeg.is_file() {
        return Err(InstallError::NotWorking(format!(
            "{} is not a file",
            ffmpeg.display()
        )));
    }
    let dir = ffmpeg.parent().unwrap_or(Path::new("."));
    let probe_name = match ffmpeg.file_name().and_then(|n| n.to_str()) {
        Some(n) if n.to_ascii_lowercase().starts_with("ffmpeg") => {
            n.replacen("ffmpeg", "ffprobe", 1)
        }
        _ => exe_name("ffprobe"),
    };
    let ffprobe = dir.join(probe_name);
    if !ffprobe.is_file() {
        return Err(InstallError::NotWorking(format!(
            "ffprobe not found next to {}",
            ffmpeg.display()
        )));
    }
    let version = version_of(&ffmpeg)?;
    version_of(&ffprobe)?;
    Ok(InstalledFfmpeg {
        ffmpeg,
        ffprobe,
        version,
        user_supplied: true,
        install_dir: None,
    })
}

/// Delete a Smidge install. A user copy is never touched.
pub fn remove(installed: &InstalledFfmpeg) -> std::io::Result<()> {
    match &installed.install_dir {
        Some(dir) if !installed.user_supplied => std::fs::remove_dir_all(dir),
        _ => Ok(()),
    }
}

/// Find a usable FFmpeg without searching `PATH`: the `CIA_FFMPEG`
/// override first (tests, CI), then the pinned version under `app_data`,
/// then any other installed version (newest name first).
pub fn locate(app_data: &Path) -> Option<InstalledFfmpeg> {
    let over = std::env::var_os("CIA_FFMPEG").filter(|v| !v.is_empty());
    locate_with_override(app_data, over.as_deref())
}

/// [`locate`] with the override passed in instead of read from the environment.
pub fn locate_with_override(
    app_data: &Path,
    override_path: Option<&std::ffi::OsStr>,
) -> Option<InstalledFfmpeg> {
    if let Some(over) = override_path {
        return match use_own_copy(Path::new(over)) {
            Ok(i) => Some(i),
            Err(e) => {
                log::warn!("CIA_FFMPEG={:?} is not usable: {e}", over);
                None
            }
        };
    }
    let pinned = install_dir(app_data, crate::pinned::VERSION);
    if let Ok(i) = verify_binary(&pinned) {
        return Some(i);
    }
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(tools_dir(app_data))
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.is_dir()
                && !p
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with('.'))
                && p.file_name().is_some_and(|n| n != "external")
        })
        .collect();
    dirs.sort();
    dirs.into_iter().rev().find_map(|d| verify_binary(&d).ok())
}

/// `reqwest` over rustls, streaming with progress and cancel.
#[cfg(feature = "download")]
pub struct HttpFetcher {
    client: reqwest::blocking::Client,
}

#[cfg(feature = "download")]
impl HttpFetcher {
    pub fn new() -> Result<Self, InstallError> {
        let client = reqwest::blocking::Client::builder()
            .user_agent(concat!("Smidge/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(30))
            // Whole-request cap; a 34 MB zip at 60 kB/s still fits.
            .timeout(Duration::from_secs(600))
            .build()
            .map_err(|e| InstallError::Network(e.to_string()))?;
        Ok(HttpFetcher { client })
    }
}

#[cfg(feature = "download")]
impl Fetcher for HttpFetcher {
    fn fetch(
        &self,
        url: &str,
        on_progress: &mut dyn FnMut(u64, Option<u64>),
        cancel: &Cancel,
    ) -> Result<Vec<u8>, InstallError> {
        if url.starts_with("file://") {
            return FileFetcher.fetch(url, on_progress, cancel);
        }
        let mut response = self
            .client
            .get(url)
            .send()
            .map_err(|e| InstallError::Network(e.to_string()))?;
        if !response.status().is_success() {
            return Err(InstallError::Network(format!(
                "{} returned {}",
                url,
                response.status()
            )));
        }
        let total = response.content_length();
        let mut out = Vec::with_capacity(total.unwrap_or(0).min(256 << 20) as usize);
        let mut buf = [0u8; 64 * 1024];
        loop {
            if cancel.is_cancelled() {
                return Err(InstallError::Cancelled);
            }
            let n = response
                .read(&mut buf)
                .map_err(|e| InstallError::Network(e.to_string()))?;
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n]);
            on_progress(out.len() as u64, total);
        }
        if total.is_some_and(|t| t != out.len() as u64) {
            return Err(InstallError::Network(format!(
                "short read: {} of {} bytes",
                out.len(),
                total.unwrap_or(0)
            )));
        }
        Ok(out)
    }
}

/// Install the pinned FFmpeg over HTTPS.
#[cfg(feature = "download")]
pub fn install(
    app_data: &Path,
    progress: &mut dyn FnMut(InstallProgress),
    cancel: &Cancel,
) -> Result<InstalledFfmpeg, InstallError> {
    let fetcher = HttpFetcher::new()?;
    install_with(&fetcher, &crate::pinned::PIN, app_data, progress, cancel)
}

/// `CIA_FFMPEG_MANIFEST_BASE` (tests, CI, lab runs) points every release URL at a mirror of the
/// pinned release: the directory part of each URL is replaced, the file names and the pinned
/// public key stay, so the signature and hashes are still verified. Unset in normal use.
pub fn mirror_url(url: &str) -> String {
    match std::env::var("CIA_FFMPEG_MANIFEST_BASE") {
        Ok(base) if !base.is_empty() => {
            let name = url.rsplit('/').next().unwrap_or(url);
            format!("{}/{}", base.trim_end_matches('/'), name)
        }
        _ => url.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_formats() {
        let raw = [7u8; 64];
        assert_eq!(parse_signature(&raw).unwrap(), raw);
        let hex: String = raw.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(parse_signature(format!("{hex}\n").as_bytes()).unwrap(), raw);
        let b64 = STANDARD.encode(raw);
        assert_eq!(parse_signature(b64.as_bytes()).unwrap(), raw);
        assert!(matches!(
            parse_signature(b"nope"),
            Err(InstallError::BadSignature)
        ));
        assert!(matches!(
            parse_signature(&[1u8; 63]),
            Err(InstallError::BadSignature)
        ));
    }

    #[test]
    fn sha256_known_answer() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn user_messages_match_design() {
        assert_eq!(
            InstallError::HashMismatch.user_message(false),
            "The download was damaged, so Smidge deleted it. Please try again."
        );
        assert_eq!(
            InstallError::BadSignature.user_message(true),
            "Smidge couldn't get a good copy of FFmpeg. Contact support and we'll help."
        );
        assert_eq!(
            InstallError::DiskFull.user_message(false),
            "There isn't enough free space. FFmpeg needs about 90 MB."
        );
        assert!(InstallError::Blocked(String::new())
            .user_message(false)
            .starts_with("Your security software blocked FFmpeg."));
        assert!(InstallError::Network(String::new())
            .user_message(false)
            .starts_with("The download didn't finish."));
    }

    #[test]
    fn paths() {
        let app = Path::new("/app");
        assert_eq!(tools_dir(app), PathBuf::from("/app/tools/ffmpeg"));
        assert_eq!(
            install_dir(app, "7.1.2"),
            PathBuf::from("/app/tools/ffmpeg/7.1.2")
        );
    }

    #[test]
    fn remove_never_touches_a_user_copy() {
        let own = InstalledFfmpeg {
            ffmpeg: "/usr/bin/ffmpeg".into(),
            ffprobe: "/usr/bin/ffprobe".into(),
            version: "x".into(),
            user_supplied: true,
            install_dir: Some("/usr/bin".into()),
        };
        remove(&own).unwrap();
        assert!(Path::new("/usr/bin").exists() || !cfg!(unix));
    }
}
