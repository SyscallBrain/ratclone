//! Command-line arguments.

use std::path::PathBuf;

use clap::Parser;

#[derive(Parser, Debug)]
#[command(
    name = "ratclone",
    version,
    about = "A keyboard-driven terminal UI for rclone"
)]
pub struct Cli {
    /// Run with made-up accounts and files; nothing touches rclone.
    #[arg(long)]
    pub demo: bool,
    /// Path to the rclone binary (default: `rclone` on PATH, or `rclone_path` in the config).
    #[arg(long, value_name = "PATH")]
    pub rclone: Option<PathBuf>,
    /// Unix socket for the app's rclone rcd (default: $XDG_RUNTIME_DIR/ratclone/rc.sock).
    #[arg(long, value_name = "PATH")]
    pub socket: Option<PathBuf>,
    /// Internal: stop the rcd once it has nothing left to do, then exit.
    #[arg(long, hide = true)]
    pub reap_when_idle: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_flags() {
        let c = Cli::parse_from([
            "ratclone",
            "--demo",
            "--rclone",
            "/opt/rclone",
            "--socket",
            "/tmp/s",
        ]);
        assert!(c.demo);
        assert_eq!(c.rclone.as_deref(), Some(std::path::Path::new("/opt/rclone")));
        assert_eq!(c.socket.as_deref(), Some(std::path::Path::new("/tmp/s")));
        let d = Cli::parse_from(["ratclone"]);
        assert!(!d.demo && d.rclone.is_none());
    }
}
