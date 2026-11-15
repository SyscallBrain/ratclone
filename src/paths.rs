//! XDG locations used by the app.

use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::PathBuf;

use crate::APP_NAME;

fn env_dir(var: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
}

/// Running inside an SSH session.
pub fn over_ssh() -> bool {
    std::env::var_os("SSH_CONNECTION").is_some() || std::env::var_os("SSH_TTY").is_some()
}

fn home() -> PathBuf {
    env_dir("HOME").unwrap_or_else(|| PathBuf::from("/"))
}

/// `$XDG_RUNTIME_DIR/ratclone/rc.sock`, or `/tmp/ratclone-<uid>/rc.sock`.
/// The directory is created with mode 0700.
pub fn socket_path() -> PathBuf {
    let dir = match env_dir("XDG_RUNTIME_DIR") {
        Some(d) => d.join(APP_NAME),
        // SAFETY: getuid has no preconditions and cannot fail.
        None => PathBuf::from(format!("/tmp/{APP_NAME}-{}", unsafe { libc::getuid() })),
    };
    let _ = std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dir);
    let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    dir.join("rc.sock")
}

/// Move `path` to `path.1` once it is larger than `max_bytes` (one old copy kept).
pub fn rotate_log(path: &std::path::Path, max_bytes: u64) {
    if std::fs::metadata(path).is_ok_and(|m| m.len() > max_bytes) {
        let mut old = path.as_os_str().to_owned();
        old.push(".1");
        let _ = std::fs::rename(path, old);
    }
}

/// Hold an exclusive lock on `<dir>/instance.lock` for as long as the file lives.
/// A second app sharing the same rcd and history would lose the other's transfers.
pub fn acquire_instance_lock(dir: &std::path::Path) -> std::io::Result<std::fs::File> {
    acquire_lock(dir, "instance.lock")
}

/// Hold an exclusive lock on `<dir>/<name>` for as long as the file lives; an error
/// when another process holds it.
pub fn acquire_lock(dir: &std::path::Path, name: &str) -> std::io::Result<std::fs::File> {
    use std::os::fd::AsRawFd;
    std::fs::create_dir_all(dir)?;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join(name))?;
    // SAFETY: flock on a file descriptor we own; non-blocking.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(file)
}

/// `$XDG_STATE_HOME/ratclone` (default `~/.local/state/ratclone`), created if missing.
pub fn state_dir() -> PathBuf {
    let dir = env_dir("XDG_STATE_HOME")
        .unwrap_or_else(|| home().join(".local/state"))
        .join(APP_NAME);
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// `$XDG_CACHE_HOME/ratclone` (default `~/.cache/ratclone`), created if missing.
pub fn cache_dir() -> PathBuf {
    let dir = env_dir("XDG_CACHE_HOME")
        .unwrap_or_else(|| home().join(".cache"))
        .join(APP_NAME);
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// `$XDG_CONFIG_HOME/ratclone/config.toml` (default `~/.config/...`).
pub fn config_file() -> PathBuf {
    env_dir("XDG_CONFIG_HOME")
        .unwrap_or_else(|| home().join(".config"))
        .join(APP_NAME)
        .join("config.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn big_logs_are_rotated() {
        let d = tempfile::tempdir().unwrap();
        let log = d.path().join("rcd.log");
        rotate_log(&log, 10); // missing file: nothing to do
        std::fs::write(&log, "0123456789ab").unwrap();
        rotate_log(&log, 10);
        assert!(!log.exists());
        assert_eq!(
            std::fs::read_to_string(d.path().join("rcd.log.1")).unwrap(),
            "0123456789ab"
        );
        std::fs::write(&log, "small").unwrap();
        rotate_log(&log, 10);
        assert!(log.exists());
    }

    #[test]
    fn second_instance_is_refused() {
        let d = tempfile::tempdir().unwrap();
        let first = acquire_instance_lock(d.path()).unwrap();
        assert!(acquire_instance_lock(d.path()).is_err());
        drop(first);
        assert!(acquire_instance_lock(d.path()).is_ok());
    }
}
