//! Test helpers: a real `rclone rcd` with a throwaway config.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use ratclone::rc::UnixRc;
use tempfile::TempDir;

pub fn rclone_bin() -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|p| p.join("rclone"))
            .find(|p| p.is_file())
    })
}

pub struct RcdFixture {
    pub dir: TempDir,
    pub root: PathBuf,
    pub rc: UnixRc,
    child: Child,
}

impl RcdFixture {
    /// Start an rcd whose config has one alias remote `tst` pointing at `root`.
    /// Returns `None` (and the test should return early) when rclone is missing.
    pub fn start() -> Option<RcdFixture> {
        Self::start_with(false)
    }

    /// Like [`RcdFixture::start`], with a fake `xdg-open` that only records the URLs
    /// it was asked to open (in `<dir>/opened.txt`), so OAuth logins open no browser.
    pub fn start_with_fake_browser() -> Option<RcdFixture> {
        Self::start_with(true)
    }

    fn start_with(fake_browser: bool) -> Option<RcdFixture> {
        let Some(rclone) = rclone_bin() else {
            eprintln!("skipping: rclone not found");
            return None;
        };
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        std::fs::create_dir(&root).unwrap();
        let conf = dir.path().join("rclone.conf");
        std::fs::write(
            &conf,
            format!("[tst]\ntype = alias\nremote = {}\n", root.display()),
        )
        .unwrap();
        let sock = dir.path().join("rc.sock");
        let mut cmd = Command::new(&rclone);
        if fake_browser {
            use std::os::unix::fs::PermissionsExt;
            let bin = dir.path().join("bin");
            std::fs::create_dir(&bin).unwrap();
            let opener = bin.join("xdg-open");
            let log = dir.path().join("opened.txt");
            std::fs::write(
                &opener,
                format!("#!/bin/sh\necho \"$@\" >> '{}'\n", log.display()),
            )
            .unwrap();
            std::fs::set_permissions(&opener, std::fs::Permissions::from_mode(0o755)).unwrap();
            std::os::unix::fs::symlink(&rclone, bin.join("rclone")).unwrap();
            cmd.env("PATH", &bin)
                .env_remove("DISPLAY")
                .env_remove("WAYLAND_DISPLAY");
        }
        let child = cmd
            .args(["rcd", "--rc-no-auth", "--rc-job-expire-duration", "5m"])
            .arg("--rc-addr")
            .arg(format!("unix://{}", sock.display()))
            .arg("--config")
            .arg(&conf)
            .arg("--log-file")
            .arg(dir.path().join("rcd.log"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        wait_for_socket(&sock);
        Some(RcdFixture {
            rc: UnixRc::new(sock),
            dir,
            root,
            child,
        })
    }

    pub fn conf(&self) -> PathBuf {
        self.dir.path().join("rclone.conf")
    }

    /// Append `text` (one or more `[section]`s) to the rcd's config file.
    pub fn add_section(&self, text: &str) {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(self.conf())
            .unwrap();
        f.write_all(text.as_bytes()).unwrap();
    }
}

pub fn wait_for_socket(sock: &Path) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        if std::os::unix::net::UnixStream::connect(sock).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("rcd did not open {}", sock.display());
}

impl Drop for RcdFixture {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub mod app {
    use std::sync::Arc;

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratclone::app::App;
    use ratclone::backend::fake::FakeBackend;
    use ratclone::config::Config;
    use ratclone::input::keymap::tokenize;
    use ratclone::location::{Location, Remote};
    use ratclone::transfer::store::TransferStore;

    pub fn key(token: &str) -> KeyEvent {
        let code = match token {
            "<CR>" => KeyCode::Enter,
            "<Esc>" => KeyCode::Esc,
            "<Tab>" => KeyCode::Tab,
            "<BS>" => KeyCode::Backspace,
            "<Space>" => KeyCode::Char(' '),
            "<Down>" => KeyCode::Down,
            "<Up>" => KeyCode::Up,
            "<Left>" => KeyCode::Left,
            "<Right>" => KeyCode::Right,
            "<S-Tab>" => KeyCode::BackTab,
            t if t.starts_with("<F") => KeyCode::F(t[2..t.len() - 1].parse().unwrap()),
            t if t.starts_with("<C-") => {
                let c = t.chars().nth(3).unwrap();
                return KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL);
            }
            t => KeyCode::Char(t.chars().next().unwrap()),
        };
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    /// Type a key sequence such as `"5j"`, `"cw"` or `"name<CR>"`.
    pub fn press(app: &mut App, keys: &str) {
        for t in tokenize(keys) {
            app.handle_key(key(&t));
        }
    }

    pub fn home() -> Location {
        Location::new(Remote::Local, "home/demo")
    }

    pub fn named(n: &str, p: &str) -> Location {
        Location::new(Remote::Named(n.into()), p)
    }

    /// App over the demo fake, already settled on the demo home folder.
    pub async fn demo_app() -> (tempfile::TempDir, Arc<FakeBackend>, App) {
        let dir = tempfile::tempdir().unwrap();
        let fake = Arc::new(FakeBackend::demo());
        let store = TransferStore::load(dir.path().join("transfers.json")).unwrap();
        let mut app = App::new(fake.clone(), store, &Config::default(), home());
        // Not the machine's zone: what the screen shows must not depend on where
        // the tests run. An hour off UTC, so a time shown unconverted would show.
        app.tz = jiff::tz::TimeZone::fixed(jiff::tz::offset(1));
        app.settle().await;
        (dir, fake, app)
    }

    /// Move the cursor to the entry called `name` in the pane.
    pub fn move_to(app: &mut App, name: &str) {
        let i = app
            .pane
            .entries()
            .position(|e| e.name == name)
            .unwrap_or_else(|| panic!("{name} not in pane"));
        app.pane.cursor = i;
    }

    pub fn names(app: &App) -> Vec<String> {
        app.pane.entries().map(|e| e.name.clone()).collect()
    }
}
