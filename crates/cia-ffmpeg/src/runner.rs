//! Run one ffmpeg command with `-progress pipe:1 -nostats` (DESIGN.md 3.5.9).
//!
//! stdout carries `key=value` blocks ending in `progress=continue|end`;
//! `out_time_us` drives the fraction and the time remaining. No block for
//! [`STALL_TIMEOUT`] kills the tree ("encoder_stalled"). The [`Cancel`] token
//! kills the tree too. Either way the listed partial files are removed.

use crate::cancel::Cancel;
use crate::process::{self, Child};
use crate::FfmpegError;
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Kill after this long without a progress block.
pub const STALL_TIMEOUT: Duration = Duration::from_secs(60);

/// Keep the tail of stderr for error messages.
const STDERR_KEEP: usize = 64 * 1024;

/// How a run reports and what it cleans up.
#[derive(Debug, Clone)]
pub struct RunOptions {
    pub stall_timeout: Duration,
    /// Output duration the fraction is computed against; None reports 0 until the end.
    pub duration_ms: Option<u64>,
    /// This run covers `[fraction_base, fraction_base + fraction_span]` of the whole job.
    pub fraction_base: f64,
    pub fraction_span: f64,
    /// Removed when the run fails, stalls or is cancelled.
    pub partial_files: Vec<PathBuf>,
}

impl Default for RunOptions {
    fn default() -> Self {
        RunOptions {
            stall_timeout: STALL_TIMEOUT,
            duration_ms: None,
            fraction_base: 0.0,
            fraction_span: 1.0,
            partial_files: Vec::new(),
        }
    }
}

/// A finished run.
#[derive(Debug, Clone)]
pub struct RunOutcome {
    pub status: i32,
    pub stderr: String,
    pub elapsed: Duration,
    pub last_out_time_us: Option<u64>,
    pub progress_blocks: u64,
}

/// One parsed progress block.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProgressUpdate {
    /// Fraction of the whole job, already mapped through base and span.
    pub fraction: f64,
    pub eta_ms: Option<u64>,
    pub out_time_us: u64,
    pub speed: Option<f32>,
    pub end: bool,
}

/// Turns `-progress` lines into [`ProgressUpdate`]s, one per block.
#[derive(Debug, Clone)]
pub struct ProgressParser {
    duration_us: Option<u64>,
    started: Instant,
    base: f64,
    span: f64,
    out_time_us: u64,
    speed: Option<f32>,
    pub blocks: u64,
}

impl ProgressParser {
    pub fn new(duration_ms: Option<u64>) -> Self {
        ProgressParser {
            duration_us: duration_ms.map(|ms| ms.saturating_mul(1000)),
            started: Instant::now(),
            base: 0.0,
            span: 1.0,
            out_time_us: 0,
            speed: None,
            blocks: 0,
        }
    }

    pub fn with_window(mut self, base: f64, span: f64) -> Self {
        self.base = base;
        self.span = span;
        self
    }

    /// Feed one line. Returns an update at the end of each block.
    pub fn feed(&mut self, line: &str) -> Option<ProgressUpdate> {
        self.feed_at(line, Instant::now())
    }

    /// [`ProgressParser::feed`] with an explicit clock, for tests.
    pub fn feed_at(&mut self, line: &str, now: Instant) -> Option<ProgressUpdate> {
        let line = line.trim();
        let (key, value) = line.split_once('=')?;
        let value = value.trim();
        match key.trim() {
            "out_time_us" | "out_time_ms" => {
                // Both fields carry microseconds in every FFmpeg release so far.
                if let Ok(v) = value.parse::<i64>() {
                    self.out_time_us = v.max(0) as u64;
                }
                None
            }
            "speed" => {
                self.speed = value
                    .trim_end_matches('x')
                    .parse::<f32>()
                    .ok()
                    .filter(|s| *s > 0.0);
                None
            }
            "progress" => {
                self.blocks += 1;
                let end = value == "end";
                Some(self.update(end, now))
            }
            _ => None,
        }
    }

    fn update(&self, end: bool, now: Instant) -> ProgressUpdate {
        let raw = match self.duration_us {
            Some(d) if d > 0 => (self.out_time_us as f64 / d as f64).clamp(0.0, 1.0),
            _ => 0.0,
        };
        let raw = if end { 1.0 } else { raw };
        let eta_ms = if end { Some(0) } else { self.eta_ms(raw, now) };
        ProgressUpdate {
            fraction: (self.base + raw * self.span).clamp(0.0, 1.0),
            eta_ms,
            out_time_us: self.out_time_us,
            speed: self.speed,
            end,
        }
    }

    fn eta_ms(&self, raw: f64, now: Instant) -> Option<u64> {
        let duration_us = self.duration_us?;
        let remaining_us = duration_us.saturating_sub(self.out_time_us) as f64;
        if let Some(speed) = self.speed {
            return Some((remaining_us / f64::from(speed) / 1000.0).round() as u64);
        }
        if raw <= 0.0 {
            return None;
        }
        let elapsed = now.saturating_duration_since(self.started).as_secs_f64();
        Some((elapsed * (1.0 - raw) / raw * 1000.0).round() as u64)
    }
}

/// Spawn `ffmpeg args`, stream progress, watch for stalls and cancel.
pub fn run_ffmpeg(
    ffmpeg: &Path,
    args: &[OsString],
    opts: &RunOptions,
    progress: &mut dyn FnMut(f64, Option<u64>),
    cancel: &Cancel,
) -> Result<RunOutcome, FfmpegError> {
    if cancel.is_cancelled() {
        remove_partials(&opts.partial_files);
        return Err(FfmpegError::Cancelled);
    }
    log::debug!("ffmpeg {}", crate::command::render(args));
    let mut cmd = process::command(ffmpeg);
    cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());
    let started = Instant::now();
    let mut child = Child::spawn(cmd).map_err(FfmpegError::Spawn)?;

    let stderr_thread = child.take_stderr().map(|mut err| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            while let Ok(n) = err.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
                if buf.len() > STDERR_KEEP * 2 {
                    buf.drain(..buf.len() - STDERR_KEEP);
                }
            }
            buf
        })
    });

    let (tx, rx) = mpsc::channel::<String>();
    if let Some(out) = child.take_stdout() {
        std::thread::spawn(move || {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
    }

    let mut parser =
        ProgressParser::new(opts.duration_ms).with_window(opts.fraction_base, opts.fraction_span);
    let mut last_block = Instant::now();
    let mut last_out_time = None;
    let mut stdout_open = true;

    let fail = |child: &mut Child, err: FfmpegError| {
        child.kill_tree();
        remove_partials(&opts.partial_files);
        Err(err)
    };

    loop {
        if stdout_open {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(line) => {
                    if let Some(update) = parser.feed(&line) {
                        last_block = Instant::now();
                        last_out_time = Some(update.out_time_us);
                        progress(update.fraction, update.eta_ms);
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => stdout_open = false,
            }
        } else {
            std::thread::sleep(Duration::from_millis(20));
        }
        if cancel.is_cancelled() {
            return fail(&mut child, FfmpegError::Cancelled);
        }
        if last_block.elapsed() > opts.stall_timeout {
            return fail(&mut child, FfmpegError::Stalled);
        }
        if !stdout_open {
            if let Some(status) = child.try_wait()? {
                let stderr = stderr_thread
                    .map(|t| String::from_utf8_lossy(&t.join().unwrap_or_default()).into_owned())
                    .unwrap_or_default();
                let code = status.code().unwrap_or(-1);
                if !status.success() {
                    remove_partials(&opts.partial_files);
                    if stderr.contains("No space left on device")
                        || stderr.contains("There is not enough space")
                    {
                        return Err(FfmpegError::DiskFull);
                    }
                    return Err(FfmpegError::Exit {
                        status: code,
                        stderr: stderr.trim().to_string(),
                    });
                }
                return Ok(RunOutcome {
                    status: code,
                    stderr,
                    elapsed: started.elapsed(),
                    last_out_time_us: last_out_time,
                    progress_blocks: parser.blocks,
                });
            }
        }
    }
}

/// Delete partial outputs; missing files are fine.
pub fn remove_partials(files: &[PathBuf]) {
    for f in files {
        match std::fs::remove_file(f) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => log::warn!("could not remove {}: {e}", f.display()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLOCK: &str = "frame=150\nfps=75.00\nstream_0_0_q=28.0\nbitrate=1500.0kbits/s\ntotal_size=937500\nout_time_us=5000000\nout_time_ms=5000000\nout_time=00:00:05.000000\ndup_frames=0\ndrop_frames=0\nspeed=2.5x\nprogress=continue\n";

    #[test]
    fn parses_a_block_into_fraction_and_eta() {
        let mut p = ProgressParser::new(Some(10_000));
        let mut updates = Vec::new();
        for line in BLOCK.lines() {
            if let Some(u) = p.feed(line) {
                updates.push(u);
            }
        }
        assert_eq!(updates.len(), 1);
        let u = updates[0];
        assert!((u.fraction - 0.5).abs() < 1e-9);
        assert_eq!(u.out_time_us, 5_000_000);
        assert_eq!(u.speed, Some(2.5));
        // 5 s remaining at 2.5x = 2 s
        assert_eq!(u.eta_ms, Some(2000));
        assert!(!u.end);
    }

    #[test]
    fn eta_falls_back_to_elapsed_when_speed_is_unknown() {
        let start = Instant::now();
        let mut p = ProgressParser::new(Some(10_000));
        p.started = start;
        let now = start + Duration::from_secs(4);
        p.feed_at("out_time_us=2000000", now);
        p.feed_at("speed=N/A", now);
        let u = p.feed_at("progress=continue", now).unwrap();
        assert!((u.fraction - 0.2).abs() < 1e-9);
        // 4 s for 20 percent -> 16 s left
        assert_eq!(u.eta_ms, Some(16_000));
    }

    #[test]
    fn end_block_is_complete_and_windowed() {
        let mut p = ProgressParser::new(Some(10_000)).with_window(0.35, 0.65);
        p.feed("out_time_us=1000000");
        let u = p.feed("progress=continue").unwrap();
        assert!((u.fraction - (0.35 + 0.065)).abs() < 1e-9);
        p.feed("out_time_us=10000000");
        let u = p.feed("progress=end").unwrap();
        assert!((u.fraction - 1.0).abs() < 1e-9);
        assert_eq!(u.eta_ms, Some(0));
        assert!(u.end);
        assert_eq!(p.blocks, 2);
    }

    #[test]
    fn garbage_and_negative_values_are_tolerated() {
        let mut p = ProgressParser::new(None);
        assert!(p.feed("").is_none());
        assert!(p.feed("not a kv line").is_none());
        assert!(p.feed("out_time_us=N/A").is_none());
        assert!(p.feed("out_time_us=-9223372036854775808").is_none());
        let u = p.feed("progress=continue").unwrap();
        assert_eq!(u.fraction, 0.0);
        assert_eq!(u.eta_ms, None);
    }

    #[test]
    fn remove_partials_ignores_missing() {
        remove_partials(&[PathBuf::from("/definitely/not/here.partial.mp4")]);
    }
}
