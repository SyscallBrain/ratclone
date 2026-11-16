mod common;

use common::RcdFixture;
use ratclone::rc::types::{RcList, RcVersion};
use ratclone::rc::{RcError, UnixRc};
use serde_json::json;

#[test]
fn daemon_wrapper_excludes_inherited_credentials_and_remotes() {
    // Spaces and quotes must remain literal paths when invoking the wrapper.
    let dir = tempfile::Builder::new()
        .prefix("ratclone test's ")
        .tempdir()
        .unwrap();
    let wrapper = common::isolated_rclone(std::path::Path::new("/usr/bin/env"), dir.path());
    let output = std::process::Command::new(wrapper)
        .env_clear()
        .env("RCLONE_CONFIG_TST_REMOTE", "/personal/files")
        .env("RCLONE_CONFIG_PASS", "private-password")
        .env("AWS_SECRET_ACCESS_KEY", "private-key")
        .env("HOME", "/personal/home")
        .env("PATH", "/fake/browser/path")
        .output()
        .unwrap();
    assert!(output.status.success());
    let env = String::from_utf8(output.stdout).unwrap();
    assert!(!env.contains("private-") && !env.contains("/personal/"), "{env}");
    assert!(
        env.lines()
            .any(|line| line == format!("HOME={}", dir.path().display()))
    );
    assert!(env.lines().any(|line| line == "PATH=/fake/browser/path"));
}

#[tokio::test]
async fn version_roundtrip() {
    let Some(f) = RcdFixture::start() else { return };
    let v: RcVersion = f.rc.call_typed("core/version", json!({})).await.unwrap();
    assert!(v.version.starts_with('v'));
    assert!(v.decomposed[0] >= 1);
}

#[tokio::test]
async fn list_alias_remote() {
    let Some(f) = RcdFixture::start() else { return };
    std::fs::write(f.root.join("a.txt"), "hi").unwrap();
    let l: RcList =
        f.rc.call_typed("operations/list", json!({"fs": "tst:", "remote": ""}))
            .await
            .unwrap();
    assert_eq!(l.list.len(), 1);
    assert_eq!(l.list[0].name, "a.txt");
    assert_eq!(l.list[0].size, 2);
    assert!(!l.list[0].is_dir);
}

#[tokio::test]
async fn api_error_is_mapped() {
    let Some(f) = RcdFixture::start() else { return };
    let e =
        f.rc.call("operations/list", json!({"fs": "nosuch:", "remote": ""}))
            .await
            .unwrap_err();
    assert!(
        // Older rclone omits the remote name from this error message.
        matches!(e, RcError::Api { status: 500, ref message, ref path }
            if message.contains("didn't find section in config file") && path == "operations/list"),
        "{e:?}"
    );
}

#[tokio::test]
async fn missing_socket_is_connect_error() {
    let rc = UnixRc::new("/nonexistent/rc.sock".into());
    assert!(matches!(
        rc.call("core/version", json!({})).await,
        Err(RcError::Connect(_))
    ));
}
