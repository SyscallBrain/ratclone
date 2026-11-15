//! Launch, find and stop the app's own `rclone rcd`.

use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::json;

use crate::rc::types::RcVersion;
use crate::rc::{RcError, UnixRc};

/// How long a freshly launched rcd may take to answer.
const START_TIMEOUT: Duration = Duration::from_secs(10);

/// How long the rcd remembers a finished job. Every rc call is a job to rclone and
/// its whole answer is kept until the job expires: each folder listing, for one. A
/// long expiry therefore grows the rcd without bound (300 listings of a 3000-file
/// folder took it from 75 MiB to 670 MiB). The app only needs a finished transfer
/// to be remembered until it next asks, once a second while open and every
/// [`crate::reaper::INTERVAL`] while closed; five minutes leave ample room.
const JOB_EXPIRE: &str = "5m";
/// How often the rcd looks for expired jobs.
const JOB_EXPIRE_CHECK: &str = "30s";

/// Oldest rclone release we support.
pub const MIN_RCLONE: (u32, u32, u32) = (1, 68, 0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonStatus {
    Reused,
    Started,
}

#[derive(thiserror::Error, Debug)]
pub enum DaemonError {
    #[error("rclone not found ({0}): install it, or set `rclone_path` in config.toml")]
    NotFound(PathBuf),
    #[error("rclone {found} is too old; {app} needs v1.68.0 or newer", app = crate::APP_NAME)]
    TooOld { found: String },
    #[error("rclone rcd did not start: {0}")]
    StartFailed(String),
    #[error(transparent)]
    Rc(#[from] RcError),
}

pub fn parse_version_ok(decomposed: [u32; 3]) -> bool {
    let (a, b, c) = MIN_RCLONE;
    (decomposed[0], decomposed[1], decomposed[2]) >= (a, b, c)
}

/// Resolve a bare program name through `PATH`; paths are taken as given.
fn resolve(rclone: &Path) -> Option<PathBuf> {
    if rclone.components().count() > 1 {
        return rclone.is_file().then(|| rclone.to_path_buf());
    }
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .map(|p| p.join(rclone))
        .find(|p| p.is_file())
}

async fn check_version(rc: &UnixRc) -> Result<(), DaemonError> {
    let v: RcVersion = rc.call_typed("core/version", json!({})).await?;
    if parse_version_ok(v.decomposed) {
        Ok(())
    } else {
        Err(DaemonError::TooOld { found: v.version })
    }
}

/// The rcd runs without authentication, so its socket must sit in a directory
/// only we can enter.
fn check_private_dir(socket: &Path) -> Result<(), DaemonError> {
    let dir = socket
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let meta = std::fs::metadata(dir)
        .map_err(|e| DaemonError::StartFailed(format!("socket directory {}: {e}", dir.display())))?;
    // SAFETY: getuid has no preconditions and cannot fail.
    let me = unsafe { libc::getuid() };
    if meta.uid() != me || meta.mode() & 0o077 != 0 {
        return Err(DaemonError::StartFailed(format!(
            "socket directory {} is not private (it must be yours with mode 0700)",
            dir.display()
        )));
    }
    Ok(())
}

/// Reuse the rcd listening on `socket`, or start a detached one.
pub async fn ensure_running(
    rclone: &Path,
    socket: &Path,
    extra_args: &[String],
) -> Result<(UnixRc, DaemonStatus), DaemonError> {
    ensure_running_with(rclone, socket, extra_args, || {}).await
}

/// [`ensure_running`], calling `before_start` right before a new rcd is launched
/// (and not when one is reused): the moment to rotate the log it is about to open.
pub async fn ensure_running_with(
    rclone: &Path,
    socket: &Path,
    extra_args: &[String],
    before_start: impl FnOnce(),
) -> Result<(UnixRc, DaemonStatus), DaemonError> {
    check_private_dir(socket)?;
    if let Ok(meta) = std::fs::symlink_metadata(socket)
        && !meta.file_type().is_socket()
    {
        return Err(DaemonError::StartFailed(format!(
            "{} exists and is not a socket",
            socket.display()
        )));
    }
    let rc = UnixRc::new(socket.to_path_buf());
    if socket.exists() {
        match check_version(&rc).await {
            Ok(()) => return Ok((rc, DaemonStatus::Reused)),
            Err(DaemonError::Rc(RcError::Connect(_))) => {
                let _ = std::fs::remove_file(socket);
            }
            Err(e) => return Err(e),
        }
    }
    let bin = resolve(rclone).ok_or_else(|| DaemonError::NotFound(rclone.to_path_buf()))?;
    before_start();
    let mut cmd = Command::new(bin);
    cmd.arg("rcd")
        .arg("--rc-addr")
        .arg(format!("unix://{}", socket.display()))
        .args([
            "--rc-no-auth",
            "--rc-job-expire-duration",
            JOB_EXPIRE,
            "--rc-job-expire-interval",
            JOB_EXPIRE_CHECK,
        ])
        .args(extra_args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: setsid is async-signal-safe and only detaches the child from our session.
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let mut child = cmd.spawn().map_err(|e| DaemonError::StartFailed(e.to_string()))?;
    let start = Instant::now();
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            return Err(DaemonError::StartFailed(format!("rclone exited with {status}")));
        }
        if rc.call("core/version", json!({})).await.is_ok() {
            break;
        }
        if start.elapsed() > START_TIMEOUT {
            let _ = child.kill();
            return Err(DaemonError::StartFailed("no answer on the socket".into()));
        }
        // The rcd is usually up within a few tens of milliseconds: look often.
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    // Reap the child when it eventually exits so it never lingers as a zombie.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    check_version(&rc).await?;
    Ok((rc, DaemonStatus::Started))
}

#[derive(serde::Deserialize)]
struct Pid {
    pid: i32,
}

/// Stop the rcd and wait (up to 5 s) until it stops answering.
/// A dropped connection means it already went away.
pub async fn shutdown(rc: &UnixRc) -> Result<(), RcError> {
    // `core/quit` answers and then waits a second and a half before exiting, which
    // is how long the shell would hang after every quit. A SIGTERM runs the same
    // exit handlers within milliseconds; the pid is the one the rcd itself reports.
    let signalled = match rc.call_typed::<Pid>("core/pid", json!({})).await {
        // SAFETY: kill has no memory-safety preconditions; the pid is a live
        // process that just answered on our private socket.
        Ok(p) if p.pid > 1 => unsafe { libc::kill(p.pid, libc::SIGTERM) == 0 },
        Err(RcError::Connect(_)) => return Ok(()),
        _ => false,
    };
    if !signalled {
        match rc.call("core/quit", json!({})).await {
            Ok(_) | Err(RcError::Connect(_)) => {}
            Err(e) => return Err(e),
        }
    }
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        if let Err(RcError::Connect(_)) = rc.call("core/version", json!({})).await {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_floor() {
        assert!(parse_version_ok([1, 68, 0]));
        assert!(parse_version_ok([1, 76, 0]));
        assert!(parse_version_ok([2, 0, 0]));
        assert!(!parse_version_ok([1, 67, 9]));
        assert!(!parse_version_ok([0, 99, 0]));
    }
}
