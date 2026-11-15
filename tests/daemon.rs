mod common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use ratclone::daemon::{DaemonError, DaemonStatus, ensure_running, shutdown};
use ratclone::rc::RcError;
use serde_json::json;

struct Env {
    _dir: tempfile::TempDir,
    rclone: PathBuf,
    sock: PathBuf,
    args: Vec<String>,
}

fn env() -> Option<Env> {
    let rclone = common::rclone_bin()?;
    let dir = tempfile::tempdir().unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let conf = dir.path().join("rclone.conf");
    std::fs::write(&conf, "").unwrap();
    let args = vec![
        "--config".to_string(),
        conf.display().to_string(),
        "--log-file".to_string(),
        dir.path().join("rcd.log").display().to_string(),
    ];
    Some(Env {
        sock: dir.path().join("rc.sock"),
        rclone,
        args,
        _dir: dir,
    })
}

#[tokio::test]
async fn starts_then_reuses_then_quits() {
    let Some(e) = env() else { return };
    let (rc, s1) = ensure_running(&e.rclone, &e.sock, &e.args).await.unwrap();
    assert!(matches!(s1, DaemonStatus::Started));
    let (_rc2, s2) = ensure_running(&e.rclone, &e.sock, &e.args).await.unwrap();
    assert!(matches!(s2, DaemonStatus::Reused));
    shutdown(&rc).await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(matches!(
        rc.call("core/version", json!({})).await,
        Err(RcError::Connect(_))
    ));
}

#[tokio::test]
async fn stale_socket_is_replaced() {
    let Some(e) = env() else { return };
    drop(std::os::unix::net::UnixListener::bind(&e.sock).unwrap());
    let (rc, s) = ensure_running(&e.rclone, &e.sock, &e.args).await.unwrap();
    assert!(matches!(s, DaemonStatus::Started));
    shutdown(&rc).await.unwrap();
}

#[tokio::test]
async fn missing_binary() {
    let Some(e) = env() else { return };
    let r = ensure_running(Path::new("/no/rclone"), &e.sock, &[]).await;
    assert!(matches!(r, Err(DaemonError::NotFound(_))));
}

#[tokio::test]
async fn a_regular_file_at_the_socket_path_is_left_alone() {
    let Some(e) = env() else { return };
    std::fs::write(&e.sock, "my notes").unwrap();
    let r = ensure_running(&e.rclone, &e.sock, &e.args).await;
    assert!(matches!(r, Err(DaemonError::StartFailed(_))), "{r:?}");
    assert_eq!(std::fs::read_to_string(&e.sock).unwrap(), "my notes");
}

#[tokio::test]
async fn a_shared_socket_directory_is_refused() {
    let Some(e) = env() else { return };
    use std::os::unix::fs::PermissionsExt;
    let open = e.sock.parent().unwrap().join("open");
    std::fs::create_dir(&open).unwrap();
    std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o777)).unwrap();
    let r = ensure_running(&e.rclone, &open.join("rc.sock"), &e.args).await;
    assert!(
        matches!(r, Err(DaemonError::StartFailed(ref m)) if m.contains("private")),
        "{r:?}"
    );
}

mod reaper {
    use super::*;
    use ratclone::paths::acquire_instance_lock;
    use ratclone::reaper::reap_when_idle;

    const FAST: Duration = Duration::from_millis(100);

    async fn gone(rc: &ratclone::rc::UnixRc) -> bool {
        matches!(rc.call("core/version", json!({})).await, Err(RcError::Connect(_)))
    }

    #[tokio::test]
    async fn stops_an_idle_rcd() {
        let Some(e) = env() else { return };
        let (rc, _) = ensure_running(&e.rclone, &e.sock, &e.args).await.unwrap();
        let locks = e.sock.parent().unwrap().join("state");
        tokio::time::timeout(Duration::from_secs(10), reap_when_idle(&rc, &locks, FAST))
            .await
            .unwrap();
        assert!(gone(&rc).await);
    }

    #[tokio::test]
    async fn leaves_it_alone_while_the_app_is_open() {
        let Some(e) = env() else { return };
        let (rc, _) = ensure_running(&e.rclone, &e.sock, &e.args).await.unwrap();
        let locks = e.sock.parent().unwrap().join("state");
        let app = acquire_instance_lock(&locks).unwrap();
        let r = tokio::time::timeout(Duration::from_secs(1), reap_when_idle(&rc, &locks, FAST)).await;
        assert!(r.is_err(), "reaper stopped the rcd under an open app");
        assert!(!gone(&rc).await);
        drop(app);
        tokio::time::timeout(Duration::from_secs(10), reap_when_idle(&rc, &locks, FAST))
            .await
            .unwrap();
        assert!(gone(&rc).await);
    }

    #[tokio::test]
    async fn writes_down_how_a_background_transfer_ended_before_stopping_the_rcd() {
        use ratclone::backend::Backend;
        use ratclone::backend::rc::RcBackend;
        use ratclone::location::{Location, Remote};
        use ratclone::transfer::spec::{TransferItem, TransferKind, TransferSpec};
        use ratclone::transfer::store::{RecordState, TransferStore};

        let Some(e) = env() else { return };
        let (rc, _) = ensure_running(&e.rclone, &e.sock, &e.args).await.unwrap();
        let dir = e.sock.parent().unwrap();
        let state = dir.join("state");
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/a.txt"), "aaa").unwrap();
        // What the app leaves when it quits with a copy still running.
        let spec = TransferSpec {
            id: 1,
            kind: TransferKind::Copy,
            src_dir: Location::new(Remote::Local, &dir.join("src").display().to_string()),
            dst_dir: Location::new(Remote::Local, &dir.join("dst").display().to_string()),
            items: vec![TransferItem {
                name: "a.txt".into(),
                is_dir: false,
                dst_name: "a.txt".into(),
                size: Some(3),
                replace: false,
            }],
        };
        let ids = RcBackend::new(rc.clone())
            .start_transfer(&spec, &|_| {})
            .await
            .unwrap();
        let path = state.join(ratclone::reaper::STORE_FILE);
        let mut store = TransferStore::load(path.clone()).unwrap();
        store.push(spec, ids, jiff::Timestamp::now());
        store.save().unwrap();

        tokio::time::timeout(Duration::from_secs(10), reap_when_idle(&rc, &state, FAST))
            .await
            .unwrap();
        assert!(gone(&rc).await);
        assert_eq!(std::fs::read_to_string(dir.join("dst/a.txt")).unwrap(), "aaa");
        // The rcd took what it knew with it: without this the app, opened later,
        // could only call a copy that went well "interrupted".
        let store = TransferStore::load(path).unwrap();
        assert_eq!(store.records[0].state, RecordState::Done);
        assert!(store.records[0].finished.is_some());
    }

    #[tokio::test]
    async fn one_helper_is_enough() {
        let Some(e) = env() else { return };
        let (rc, _) = ensure_running(&e.rclone, &e.sock, &e.args).await.unwrap();
        let state = e.sock.parent().unwrap().join("state");
        let on_duty = ratclone::paths::acquire_lock(&state, "reaper.lock").unwrap();
        // A second one leaves at once, and leaves the rcd alone.
        tokio::time::timeout(Duration::from_secs(1), reap_when_idle(&rc, &state, FAST))
            .await
            .expect("it should not have stayed");
        assert!(!gone(&rc).await);
        drop(on_duty);
        shutdown(&rc).await.unwrap();
    }

    #[tokio::test]
    async fn waits_for_running_transfers() {
        let Some(e) = env() else { return };
        let (rc, _) = ensure_running(&e.rclone, &e.sock, &e.args).await.unwrap();
        let locks = e.sock.parent().unwrap().join("state");
        let src = e.sock.parent().unwrap().join("src");
        std::fs::create_dir(&src).unwrap();
        std::fs::write(src.join("big.bin"), vec![7u8; 3_000_000]).unwrap();
        rc.call("core/bwlimit", json!({"rate": "1M"})).await.unwrap();
        rc.call(
            "sync/copy",
            json!({"srcFs": src.display().to_string(), "dstFs": src.parent().unwrap().join("dst").display().to_string(),
                   "_async": true, "_group": "rtui/1"}),
        )
        .await
        .unwrap();
        let r = tokio::time::timeout(Duration::from_millis(1200), reap_when_idle(&rc, &locks, FAST)).await;
        assert!(r.is_err(), "reaper stopped the rcd during a transfer");
        assert!(!gone(&rc).await);
        shutdown(&rc).await.unwrap();
    }
}

#[tokio::test]
async fn a_pooled_client_survives_an_rcd_restart() {
    let Some(e) = env() else { return };
    let (rc, _) = ensure_running(&e.rclone, &e.sock, &e.args).await.unwrap();
    // Leave keep-alive connections in the pool.
    for _ in 0..3 {
        rc.call("core/version", json!({})).await.unwrap();
    }
    shutdown(&rc).await.unwrap();
    assert!(matches!(
        rc.call("core/version", json!({})).await,
        Err(RcError::Connect(_))
    ));
    let (_, status) = ensure_running(&e.rclone, &e.sock, &e.args).await.unwrap();
    assert_eq!(status, DaemonStatus::Started);
    // The same client (and its pool) talks to the new rcd.
    for _ in 0..3 {
        rc.call("core/version", json!({})).await.unwrap();
    }
    shutdown(&rc).await.unwrap();
}
