mod common;

use common::RcdFixture;
use ratclone::rc::types::{RcList, RcVersion};
use ratclone::rc::{RcError, UnixRc};
use serde_json::json;

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
        matches!(e, RcError::Api { status: 500, ref message, .. } if message.contains("nosuch")),
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
