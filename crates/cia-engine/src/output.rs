//! Output sinks (DESIGN.md 3.10). The engine asks the sink for a free name,
//! writes bytes atomically and never overwrites anything.

use crate::EngineError;
use cia_core::OutputLocation;
use std::collections::BTreeMap;
use std::sync::Mutex;

/// Where an output goes: a directory (desktop path or OPFS job folder) plus a file name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputDest {
    pub dir: String,
    pub file_name: String,
}

pub trait OutputSink: Send + Sync {
    /// True when a file with this name already exists in `dir`.
    fn exists(&self, dir: &str, file_name: &str) -> bool;
    /// Atomic no-overwrite write. Returns the final location.
    fn write(&self, dest: &OutputDest, bytes: &[u8]) -> Result<OutputLocation, EngineError>;
    /// Size read back from the sink (the honesty rule reads the written bytes, never the buffer).
    fn len(&self, location: &OutputLocation) -> Result<u64, EngineError>;
    fn read(&self, location: &OutputLocation) -> Result<Vec<u8>, EngineError>;
    fn remove(&self, location: &OutputLocation);
    /// Create `dir/sub` and return its path.
    fn make_dir(&self, dir: &str, sub: &str) -> Result<String, EngineError>;
    /// Copy an input (desktop: hard link or copy; web: write bytes) into the sink.
    fn copy_in(&self, bytes: &[u8], dest: &OutputDest) -> Result<OutputLocation, EngineError> {
        self.write(dest, bytes)
    }
}

/// In-memory sink for tests and the WASM host (which hands buffers to OPFS in JS).
#[derive(Default)]
pub struct MemorySink {
    pub files: Mutex<BTreeMap<String, Vec<u8>>>,
}

impl MemorySink {
    fn key(dir: &str, name: &str) -> String {
        if dir.is_empty() {
            name.to_string()
        } else {
            format!("{}/{}", dir.trim_end_matches('/'), name)
        }
    }
    pub fn get(&self, path: &str) -> Option<Vec<u8>> {
        self.files.lock().unwrap().get(path).cloned()
    }
    pub fn paths(&self) -> Vec<String> {
        self.files.lock().unwrap().keys().cloned().collect()
    }
}

impl OutputSink for MemorySink {
    fn exists(&self, dir: &str, file_name: &str) -> bool {
        self.files
            .lock()
            .unwrap()
            .contains_key(&Self::key(dir, file_name))
    }
    fn write(&self, dest: &OutputDest, bytes: &[u8]) -> Result<OutputLocation, EngineError> {
        let key = Self::key(&dest.dir, &dest.file_name);
        let mut files = self.files.lock().unwrap();
        if files.contains_key(&key) {
            return Err(EngineError::Io(format!("{key} already exists")));
        }
        files.insert(key.clone(), bytes.to_vec());
        Ok(OutputLocation::Opfs { path: key })
    }
    fn len(&self, location: &OutputLocation) -> Result<u64, EngineError> {
        let (OutputLocation::Opfs { path } | OutputLocation::Path { path }) = location;
        self.files
            .lock()
            .unwrap()
            .get(path)
            .map(|b| b.len() as u64)
            .ok_or_else(|| EngineError::Io(format!("{path} missing")))
    }
    fn read(&self, location: &OutputLocation) -> Result<Vec<u8>, EngineError> {
        let (OutputLocation::Opfs { path } | OutputLocation::Path { path }) = location;
        self.files
            .lock()
            .unwrap()
            .get(path)
            .cloned()
            .ok_or_else(|| EngineError::Io(format!("{path} missing")))
    }
    fn remove(&self, location: &OutputLocation) {
        let (OutputLocation::Opfs { path } | OutputLocation::Path { path }) = location;
        self.files.lock().unwrap().remove(path);
    }
    fn make_dir(&self, dir: &str, sub: &str) -> Result<String, EngineError> {
        Ok(Self::key(dir, sub))
    }
}

/// Filesystem sink with the atomic no-overwrite write from DESIGN.md 3.10.
#[cfg(feature = "fs")]
pub mod fs_sink {
    use super::*;
    use std::fs;
    use std::io::Write;
    use std::path::{Path, PathBuf};

    pub struct FsSink {
        pub job_id: String,
    }

    impl FsSink {
        pub fn new(job_id: &str) -> Self {
            Self {
                job_id: job_id.to_string(),
            }
        }
        /// Remove `.smidge-*.partial` files older than one hour in `dir`.
        pub fn clean_partials(dir: &Path) {
            let Ok(rd) = fs::read_dir(dir) else { return };
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if name.contains(".smidge-") && name.ends_with(".partial") {
                    if let Ok(m) = e.metadata() {
                        if m.modified()
                            .ok()
                            .and_then(|t| t.elapsed().ok())
                            .is_some_and(|age| age.as_secs() > 3600)
                        {
                            let _ = fs::remove_file(e.path());
                        }
                    }
                }
            }
        }
    }

    impl OutputSink for FsSink {
        fn exists(&self, dir: &str, file_name: &str) -> bool {
            Path::new(dir).join(file_name).exists()
        }
        fn write(&self, dest: &OutputDest, bytes: &[u8]) -> Result<OutputLocation, EngineError> {
            let dir = PathBuf::from(&dest.dir);
            fs::create_dir_all(&dir).map_err(|e| map_io(&e, "not_writable"))?;
            let stem = dest
                .file_name
                .rsplit_once('.')
                .map(|(s, _)| s.to_string())
                .unwrap_or(dest.file_name.clone());
            let tmp = dir.join(format!(".{}.smidge-{}.partial", stem, self.job_id));
            {
                let mut f = fs::File::create(&tmp).map_err(|e| map_io(&e, "not_writable"))?;
                f.write_all(bytes).map_err(|e| {
                    let _ = fs::remove_file(&tmp);
                    map_io(&e, "disk_full")
                })?;
                f.sync_all().map_err(|e| {
                    let _ = fs::remove_file(&tmp);
                    map_io(&e, "disk_full")
                })?;
            }
            let final_path = dir.join(&dest.file_name);
            // hard_link fails with AlreadyExists and never replaces; fall back to create_new + copy where unsupported.
            match fs::hard_link(&tmp, &final_path) {
                Ok(()) => {
                    let _ = fs::remove_file(&tmp);
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let _ = fs::remove_file(&tmp);
                    return Err(EngineError::Io(format!(
                        "{} already exists",
                        final_path.display()
                    )));
                }
                Err(_) => {
                    let mut f = fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&final_path)
                        .map_err(|e| {
                            let _ = fs::remove_file(&tmp);
                            if e.kind() == std::io::ErrorKind::AlreadyExists {
                                EngineError::Io("exists".into())
                            } else {
                                map_io(&e, "not_writable")
                            }
                        })?;
                    f.write_all(bytes).map_err(|e| {
                        let _ = fs::remove_file(&tmp);
                        let _ = fs::remove_file(&final_path);
                        map_io(&e, "disk_full")
                    })?;
                    f.sync_all().ok();
                    let _ = fs::remove_file(&tmp);
                }
            }
            Ok(OutputLocation::Path {
                path: final_path.to_string_lossy().to_string(),
            })
        }
        fn len(&self, location: &OutputLocation) -> Result<u64, EngineError> {
            let (OutputLocation::Opfs { path } | OutputLocation::Path { path }) = location;
            fs::metadata(path)
                .map(|m| m.len())
                .map_err(|e| EngineError::Io(e.to_string()))
        }
        fn read(&self, location: &OutputLocation) -> Result<Vec<u8>, EngineError> {
            let (OutputLocation::Opfs { path } | OutputLocation::Path { path }) = location;
            fs::read(path).map_err(|e| EngineError::Io(e.to_string()))
        }
        fn remove(&self, location: &OutputLocation) {
            let (OutputLocation::Opfs { path } | OutputLocation::Path { path }) = location;
            let _ = fs::remove_file(path);
        }
        fn make_dir(&self, dir: &str, sub: &str) -> Result<String, EngineError> {
            let p = Path::new(dir).join(sub);
            fs::create_dir_all(&p).map_err(|e| map_io(&e, "not_writable"))?;
            Ok(p.to_string_lossy().to_string())
        }
    }

    fn map_io(e: &std::io::Error, default_code: &str) -> EngineError {
        let code = match e.kind() {
            std::io::ErrorKind::PermissionDenied => "not_writable",
            std::io::ErrorKind::StorageFull => "disk_full",
            std::io::ErrorKind::NotFound => "source_vanished",
            _ => default_code,
        };
        EngineError::Io(format!("{code}: {e}"))
    }

    /// Filesystem reader.
    pub struct FsReader;
    impl crate::InputReader for FsReader {
        fn read(&self, source: &cia_core::SourceRef) -> Result<Vec<u8>, EngineError> {
            match source {
                cia_core::SourceRef::Path { path } => fs::read(path).map_err(|e| {
                    if e.kind() == std::io::ErrorKind::NotFound {
                        EngineError::Io("source_vanished".into())
                    } else {
                        EngineError::Io(e.to_string())
                    }
                }),
                cia_core::SourceRef::Handle { .. } => {
                    Err(EngineError::Unsupported("handle on desktop".into()))
                }
            }
        }
        fn read_head(
            &self,
            source: &cia_core::SourceRef,
            n: usize,
        ) -> Result<Vec<u8>, EngineError> {
            use std::io::Read;
            match source {
                cia_core::SourceRef::Path { path } => {
                    let mut f = fs::File::open(path).map_err(|e| EngineError::Io(e.to_string()))?;
                    let mut buf = vec![0u8; n];
                    let mut got = 0;
                    while got < n {
                        let k = f
                            .read(&mut buf[got..])
                            .map_err(|e| EngineError::Io(e.to_string()))?;
                        if k == 0 {
                            break;
                        }
                        got += k;
                    }
                    buf.truncate(got);
                    Ok(buf)
                }
                cia_core::SourceRef::Handle { .. } => {
                    Err(EngineError::Unsupported("handle on desktop".into()))
                }
            }
        }
        fn len(&self, source: &cia_core::SourceRef) -> Result<u64, EngineError> {
            match source {
                cia_core::SourceRef::Path { path } => fs::metadata(path)
                    .map(|m| m.len())
                    .map_err(|e| EngineError::Io(e.to_string())),
                cia_core::SourceRef::Handle { .. } => {
                    Err(EngineError::Unsupported("handle on desktop".into()))
                }
            }
        }
        fn parent_dir(&self, source: &cia_core::SourceRef) -> Option<String> {
            match source {
                cia_core::SourceRef::Path { path } => Path::new(path)
                    .parent()
                    .map(|p| p.to_string_lossy().to_string()),
                _ => None,
            }
        }
    }
}
