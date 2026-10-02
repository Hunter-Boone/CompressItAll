//! Child-process plumbing shared by the runner, the prober and the installer:
//! no console window on Windows, own process group on Unix, a Windows job
//! object so a cancel kills the whole tree, and a capture helper with a
//! deadline for the short commands (`-version`, `ffprobe`, test encodes).
//!
//! Every argument goes straight to `Command::arg`; nothing is ever passed
//! through a shell, so file names with spaces, emoji or CJK survive intact.

use crate::cancel::Cancel;
use crate::FfmpegError;
use std::ffi::OsStr;
use std::io::Read;
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

/// A `Command` for `program` with stdin closed and the platform flags set.
pub fn command(program: &Path) -> Command {
    let mut cmd = Command::new(program);
    cmd.stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // New process group, so killpg reaches everything ffmpeg might spawn.
        cmd.process_group(0);
    }
    cmd
}

/// A spawned child plus whatever the platform needs to kill its whole tree.
pub struct Child {
    inner: std::process::Child,
    #[cfg(windows)]
    job: Option<win::Job>,
}

impl Child {
    pub fn spawn(mut cmd: Command) -> std::io::Result<Child> {
        let inner = cmd.spawn()?;
        #[cfg(windows)]
        let job = {
            use std::os::windows::io::AsRawHandle;
            let job = win::Job::kill_on_close();
            match &job {
                Some(j) if j.assign(inner.as_raw_handle()) => job,
                _ => None,
            }
        };
        Ok(Child {
            inner,
            #[cfg(windows)]
            job,
        })
    }

    pub fn id(&self) -> u32 {
        self.inner.id()
    }

    pub fn take_stdout(&mut self) -> Option<std::process::ChildStdout> {
        self.inner.stdout.take()
    }

    pub fn take_stderr(&mut self) -> Option<std::process::ChildStderr> {
        self.inner.stderr.take()
    }

    pub fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        self.inner.try_wait()
    }

    pub fn wait(&mut self) -> std::io::Result<ExitStatus> {
        self.inner.wait()
    }

    /// Kill the process and everything it started, then reap it.
    pub fn kill_tree(&mut self) {
        #[cfg(unix)]
        {
            // SAFETY: killpg takes a process-group id and a signal; the group
            // is the one `process_group(0)` created for this child, and it
            // has no other side effects.
            unsafe {
                libc::killpg(self.inner.id() as libc::pid_t, libc::SIGKILL);
            }
        }
        #[cfg(windows)]
        {
            match &self.job {
                Some(job) => job.terminate(),
                None => {
                    // No job object: ask taskkill to take the tree down.
                    let _ = command(Path::new("taskkill"))
                        .args(["/PID", &self.inner.id().to_string(), "/T", "/F"])
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .status();
                }
            }
        }
        let _ = self.inner.kill();
        let _ = self.inner.wait();
    }
}

/// What a short command produced.
#[derive(Debug, Clone)]
pub struct Captured {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub elapsed: Duration,
}

impl Captured {
    pub fn success(&self) -> bool {
        self.status.success()
    }
    pub fn stdout_str(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }
    pub fn stderr_str(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }
    pub fn code(&self) -> i32 {
        self.status.code().unwrap_or(-1)
    }
}

/// Run `program args` to completion, collecting stdout and stderr, with a
/// hard deadline and an optional cancel token. On deadline the tree is
/// killed and [`FfmpegError::Stalled`] returned; on cancel, [`FfmpegError::Cancelled`].
pub fn run_capture<I, S>(
    program: &Path,
    args: I,
    deadline: Duration,
    cancel: Option<&Cancel>,
) -> Result<Captured, FfmpegError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut cmd = command(program);
    cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());
    let started = Instant::now();
    let mut child = Child::spawn(cmd).map_err(FfmpegError::Spawn)?;
    let out = child.take_stdout().map(drain);
    let err = child.take_stderr().map(drain);
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if cancel.is_some_and(Cancel::is_cancelled) {
            child.kill_tree();
            return Err(FfmpegError::Cancelled);
        }
        if started.elapsed() > deadline {
            child.kill_tree();
            return Err(FfmpegError::Stalled);
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let stdout = out
        .map(|t| t.join().unwrap_or_default())
        .unwrap_or_default();
    let stderr = err
        .map(|t| t.join().unwrap_or_default())
        .unwrap_or_default();
    Ok(Captured {
        status,
        stdout,
        stderr,
        elapsed: started.elapsed(),
    })
}

fn drain<R: Read + Send + 'static>(mut r: R) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = r.read_to_end(&mut buf);
        buf
    })
}

#[cfg(windows)]
mod win {
    use std::ffi::c_void;
    use std::mem::{size_of, zeroed};
    use std::ptr::null;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    /// A job object with KILL_ON_JOB_CLOSE: closing the handle (or dropping
    /// this) kills every process assigned to it.
    pub struct Job(HANDLE);

    impl Job {
        pub fn kill_on_close() -> Option<Job> {
            // SAFETY: plain Win32 calls with valid pointers; the handle is
            // checked for null and closed in Drop.
            unsafe {
                let handle = CreateJobObjectW(null(), null());
                if handle.is_null() {
                    return None;
                }
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let ok = SetInformationJobObject(
                    handle,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const c_void,
                    size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                );
                if ok == 0 {
                    CloseHandle(handle);
                    return None;
                }
                Some(Job(handle))
            }
        }

        pub fn assign(&self, process: *mut c_void) -> bool {
            // SAFETY: both handles are live; failure is reported, not assumed.
            unsafe { AssignProcessToJobObject(self.0, process) != 0 }
        }

        pub fn terminate(&self) {
            // SAFETY: valid job handle; exit code 1 for the killed processes.
            unsafe {
                TerminateJobObject(self.0, 1);
            }
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            // SAFETY: the handle was returned by CreateJobObjectW and not closed elsewhere.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}
