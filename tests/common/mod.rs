//! Test helpers: a real `rclone rcd` with a throwaway config.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use ratclone::rc::UnixRc;
use tempfile::TempDir;

pub fn rclone_bin() -> Option<PathBuf> {
    let binary = std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|p| p.join("rclone"))
            .find(|p| p.is_file())
    });
    if let Ok(expected) = std::env::var("RATCLONE_TEST_RCLONE_VERSION") {
        let binary = binary
            .as_ref()
            .expect("required integration-test rclone is missing from PATH");
        let output = Command::new(binary)
            .arg("version")
            .output()
            .expect("run required rclone");
        assert!(output.status.success(), "required rclone version command failed");
        let output = String::from_utf8(output.stdout).expect("rclone version is UTF-8");
        assert_eq!(
            output.lines().next(),
            Some(format!("rclone v{expected}").as_str()),
            "wrong integration-test rclone"
        );
    } else {
        assert!(
            std::env::var_os("GITHUB_ACTIONS").is_none(),
            "CI must set RATCLONE_TEST_RCLONE_VERSION to require real integration tests"
        );
    }
    binary
}

/// Keep test daemons independent of environment-defined remotes and credentials.
/// The wrapper execs rclone, preserving its PID and the caller's fake-browser PATH.
pub fn isolated_rclone(binary: &Path, directory: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let quote = |path: &Path| format!("'{}'", path.to_str().unwrap().replace('\'', "'\\''"));
    let wrapper = directory.join("rclone-test-wrapper");
    std::fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\nexec /usr/bin/env -i PATH=\"$PATH\" HOME={} XDG_CONFIG_HOME={} XDG_CACHE_HOME={} {} \"$@\"\n",
            quote(directory),
            quote(&directory.join("config")),
            quote(&directory.join("cache")),
            quote(&binary.canonicalize().unwrap()),
        ),
    )
    .unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).unwrap();
    wrapper
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
        let rclone = isolated_rclone(&rclone, dir.path());
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
        cmd.args(["rcd", "--rc-no-auth", "--rc-job-expire-duration", "5m"])
            .arg("--rc-addr")
            .arg(format!("unix://{}", sock.display()))
            .arg("--config")
            .arg(&conf)
            .arg("--log-file")
            .arg(dir.path().join("rcd.log"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let child = spawn(&mut cmd);
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

/// Spawn `cmd`, retrying while its executable is "busy". Tests run in parallel: a
/// wrapper script just written by one can still be open for writing in a child
/// another has forked but not yet exec'd, and exec refuses such a file.
pub fn spawn(cmd: &mut Command) -> Child {
    let start = Instant::now();
    loop {
        match cmd.spawn() {
            Err(e)
                if e.kind() == std::io::ErrorKind::ExecutableFileBusy
                    && start.elapsed() < Duration::from_secs(5) =>
            {
                std::thread::sleep(Duration::from_millis(10));
            }
            other => return other.unwrap(),
        }
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

/// A folder whose filesystem ignores case, like a FAT drive or an SMB share on
/// Linux: the fixture root itself where its filesystem already does (macOS),
/// otherwise an `rclone mount --vfs-case-insensitive` of it. `None` (the test
/// should return early) when neither is possible, as without FUSE.
///
/// Declare it after the fixture, so that it is unmounted before the fixture's
/// folder is removed.
pub struct CaseInsensitiveDir {
    /// The folder to set up files in and hand to rclone: names in it resolve
    /// whatever their case. Files set up straight in `backing` may not show here
    /// (the mount keeps a listing for a while).
    pub path: PathBuf,
    /// Where the files really are, with their real names, for checking.
    pub backing: PathBuf,
    mount: Option<Child>,
}

impl CaseInsensitiveDir {
    pub fn over(fixture: &RcdFixture) -> Option<CaseInsensitiveDir> {
        let backing = fixture.root.clone();
        let probe = backing.join("case-probe");
        std::fs::write(&probe, "").unwrap();
        let found = |dir: &Path| dir.join("CASE-PROBE").exists();
        if found(&backing) {
            std::fs::remove_file(&probe).unwrap();
            return Some(CaseInsensitiveDir {
                path: backing.clone(),
                backing,
                mount: None,
            });
        }
        let path = fixture.dir.path().join("case-insensitive");
        std::fs::create_dir(&path).unwrap();
        // The fixture's own wrapper: writing another one while other tests spawn
        // theirs can fail an exec with "Text file busy".
        let rclone = fixture.dir.path().join("rclone-test-wrapper");
        let mut child = spawn(
            Command::new(rclone)
                .arg("mount")
                .args([&backing, &path])
                .args(["--config", "/dev/null", "--vfs-case-insensitive"])
                // Writes go straight to the backing folder.
                .args(["--vfs-cache-mode", "off"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null()),
        );
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(5) {
            if found(&path) {
                std::fs::remove_file(&probe).unwrap();
                return Some(CaseInsensitiveDir {
                    path,
                    backing,
                    mount: Some(child),
                });
            }
            if child.try_wait().unwrap().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = child.kill();
        let _ = child.wait();
        eprintln!("skipping: no case-insensitive filesystem (FUSE unavailable?)");
        None
    }
}

impl Drop for CaseInsensitiveDir {
    fn drop(&mut self) {
        let Some(child) = &mut self.mount else { return };
        // Killed while mounted, rclone would leave a dead mount behind.
        for fusermount in ["fusermount3", "fusermount"] {
            let unmounted = Command::new(fusermount)
                .arg("-u")
                .arg(&self.path)
                .stderr(Stdio::null())
                .status();
            if unmounted.is_ok_and(|s| s.success()) {
                break;
            }
        }
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(5) {
            if child.try_wait().unwrap().is_some() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = child.kill();
        let _ = child.wait();
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
