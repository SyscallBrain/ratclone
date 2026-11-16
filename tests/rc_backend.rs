mod common;

use common::RcdFixture;
use ratclone::backend::rc::RcBackend;
use ratclone::backend::{Backend, BackendError, RemoteInfo};
use ratclone::location::{Location, Remote};

fn tst(p: &str) -> Location {
    Location::new(Remote::Named("tst".into()), p)
}

fn setup() -> Option<(RcdFixture, RcBackend)> {
    let f = RcdFixture::start()?;
    let b = RcBackend::new(f.rc.clone());
    Some((f, b))
}

#[tokio::test]
async fn remotes_include_local_first_and_alias() {
    let Some((_f, backend)) = setup() else { return };
    let r = backend.remotes().await.unwrap();
    assert_eq!(
        r[0],
        RemoteInfo {
            remote: Remote::Local,
            kind: "local".into()
        }
    );
    assert!(r.contains(&RemoteInfo {
        remote: Remote::Named("tst".into()),
        kind: "alias".into()
    }));
}

#[tokio::test]
async fn list_maps_entries() {
    let Some((f, backend)) = setup() else { return };
    std::fs::create_dir(f.root.join("d")).unwrap();
    std::fs::write(f.root.join("f.txt"), "abc").unwrap();
    let e = backend.list(&tst("")).await.unwrap();
    let file = e.iter().find(|e| e.name == "f.txt").unwrap();
    assert_eq!(file.size, Some(3));
    assert!(!file.is_dir);
    assert!(file.mod_time.is_some());
    let dir = e.iter().find(|e| e.name == "d").unwrap();
    assert!(dir.is_dir);
}

#[tokio::test]
async fn list_local_absolute_path() {
    let Some((f, backend)) = setup() else { return };
    std::fs::write(f.root.join("g.txt"), "x").unwrap();
    let loc = Location::new(Remote::Local, &f.root.display().to_string());
    let e = backend.list(&loc).await.unwrap();
    assert_eq!(e.len(), 1);
    assert_eq!(e[0].name, "g.txt");
}

#[tokio::test]
async fn stat_missing_is_none() {
    let Some((f, backend)) = setup() else { return };
    assert_eq!(backend.stat(&tst("nope")).await.unwrap(), None);
    std::fs::write(f.root.join("here.txt"), "x").unwrap();
    assert_eq!(
        backend.stat(&tst("here.txt")).await.unwrap().unwrap().name,
        "here.txt"
    );
}

#[tokio::test]
async fn mkdir_rename_delete_file_and_dir() {
    let Some((f, backend)) = setup() else { return };
    let root = &f.root;
    backend.mkdir(&tst("new dir")).await.unwrap();
    assert!(root.join("new dir").is_dir());
    std::fs::write(root.join("new dir/a b.txt"), "x").unwrap();
    backend
        .rename(&tst("new dir/a b.txt"), "c.txt", false)
        .await
        .unwrap();
    assert!(root.join("new dir/c.txt").exists());
    assert!(!root.join("new dir/a b.txt").exists());
    backend.rename(&tst("new dir"), "renamed", true).await.unwrap();
    assert!(root.join("renamed/c.txt").exists());
    assert!(!root.join("new dir").exists());
    backend.delete(&tst("renamed/c.txt"), false).await.unwrap();
    assert!(!root.join("renamed/c.txt").exists());
    backend.delete(&tst("renamed"), true).await.unwrap();
    assert!(!root.join("renamed").exists());
}

#[tokio::test]
async fn rename_never_lands_on_another_entry() {
    let Some((f, backend)) = setup() else { return };
    let root = &f.root;
    std::fs::write(root.join("a.txt"), "keep me").unwrap();
    std::fs::write(root.join("b.txt"), "b").unwrap();
    std::fs::create_dir_all(root.join("d1")).unwrap();
    std::fs::create_dir_all(root.join("d2")).unwrap();
    std::fs::write(root.join("d1/x.txt"), "x").unwrap();
    // rclone itself would overwrite the file and merge the folders.
    let e = backend.rename(&tst("b.txt"), "a.txt", false).await.unwrap_err();
    assert!(e.to_string().contains("already exists"), "{e}");
    assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "keep me");
    assert!(root.join("b.txt").exists());
    let e = backend.rename(&tst("d1"), "d2", true).await.unwrap_err();
    assert!(e.to_string().contains("already exists"), "{e}");
    assert!(root.join("d1/x.txt").exists());
    assert!(!root.join("d2/x.txt").exists());
    // A change of case only is a rename like any other.
    backend.rename(&tst("b.txt"), "B.txt", false).await.unwrap();
    assert!(root.join("B.txt").exists());
}

#[tokio::test]
async fn about_has_totals() {
    let Some((_f, backend)) = setup() else { return };
    let a = backend.about(&Remote::Named("tst".into())).await.unwrap();
    assert!(a.total.is_some());
    assert!(a.used_ratio().is_some());
}

#[tokio::test]
async fn unknown_remote_is_rclone_error() {
    let Some((_f, backend)) = setup() else { return };
    let r = backend
        .list(&Location::new(Remote::Named("nosuch".into()), ""))
        .await;
    assert!(matches!(r, Err(BackendError::Rclone(_))), "{r:?}");
}

mod transfers {
    use super::*;
    use ratclone::transfer::spec::{JobsState, TransferItem, TransferKind, TransferProgress, TransferSpec};
    use std::time::{Duration, Instant};

    fn item(name: &str, is_dir: bool, dst: &str) -> TransferItem {
        TransferItem {
            replace: false,
            name: name.into(),
            is_dir,
            dst_name: dst.into(),
            size: None,
        }
    }

    fn tree(f: &RcdFixture) {
        std::fs::create_dir_all(f.root.join("src/d")).unwrap();
        std::fs::create_dir_all(f.root.join("dst")).unwrap();
        std::fs::write(f.root.join("src/a.txt"), "aaa").unwrap();
        std::fs::write(f.root.join("src/d/x.txt"), "xx").unwrap();
    }

    async fn wait_done(b: &RcBackend, spec: &TransferSpec, ids: &[u64]) -> TransferProgress {
        let start = Instant::now();
        loop {
            let p = b.transfer_progress(&spec.group(), ids).await.unwrap();
            if p.state != JobsState::Running || start.elapsed() > Duration::from_secs(10) {
                return p;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    #[tokio::test]
    async fn each_job_is_reported_as_it_starts_and_found_by_group() {
        let Some((f, backend)) = setup() else { return };
        tree(&f);
        std::fs::write(f.root.join("src/b.txt"), "b").unwrap();
        let spec = TransferSpec {
            id: 41,
            kind: TransferKind::Copy,
            src_dir: tst("src"),
            dst_dir: tst("dst"),
            items: vec![
                item("a.txt", false, "a.txt"),
                item("d", true, "d"),
                item("b.txt", false, "b.txt"),
            ],
        };
        let heard = std::sync::Mutex::new(Vec::new());
        let ids = backend
            .start_transfer(&spec, &|id| heard.lock().unwrap().push(id))
            .await
            .unwrap();
        // a.txt and b.txt share one batched job; d has its own.
        assert_eq!(spec.job_units(), [vec![0, 2], vec![1]]);
        assert_eq!(ids.len(), 2);
        assert_eq!(*heard.lock().unwrap(), ids);
        let mut found = backend.group_jobs(&spec.group()).await.unwrap();
        found.sort_unstable();
        let mut want = ids.clone();
        want.sort_unstable();
        assert_eq!(found, want);
        assert!(backend.group_jobs("rtui/999").await.unwrap().is_empty());
        let p = wait_done(&backend, &spec, &ids).await;
        assert_eq!(p.state, JobsState::Succeeded);
        let mut done = p.done_jobs.clone();
        done.sort_unstable();
        assert_eq!(done, want);
    }

    /// A move stopped half-way: `a.txt` already went (renamed), `b.txt` did not.
    #[tokio::test]
    async fn resuming_a_partial_move_skips_what_arrived() {
        let Some((f, backend)) = setup() else { return };
        std::fs::create_dir_all(f.root.join("src")).unwrap();
        std::fs::create_dir_all(f.root.join("dst/D")).unwrap();
        std::fs::write(f.root.join("dst/a-renamed.txt"), "aaa").unwrap();
        std::fs::write(f.root.join("dst/D/x.txt"), "x").unwrap();
        std::fs::write(f.root.join("src/b.txt"), "bb").unwrap();
        std::fs::write(f.root.join("dst/gone.txt"), "different").unwrap();
        let sized = |name: &str, dst: &str, size: u64| TransferItem {
            replace: false,
            size: Some(size),
            ..item(name, false, dst)
        };
        let mut spec = TransferSpec {
            id: 42,
            kind: TransferKind::Move,
            src_dir: tst("src"),
            dst_dir: tst("dst"),
            items: vec![
                sized("a.txt", "a-renamed.txt", 3),
                item("D", true, "D"),
                sized("b.txt", "b.txt", 2),
                // Gone from the source, and what the destination has is something else.
                sized("gone.txt", "gone.txt", 3),
            ],
        };
        let arrived = ratclone::backend::moved_already(&backend, &spec).await.unwrap();
        assert_eq!(arrived, [true, true, false, false]);
        let mut keep = arrived.iter();
        spec.items.retain(|_| !keep.next().unwrap());
        // Without the arrived items the move of b.txt succeeds...
        let only_b = TransferSpec {
            items: spec.items[..1].to_vec(),
            ..spec.clone()
        };
        let ids = backend.start_transfer(&only_b, &|_| {}).await.unwrap();
        assert_eq!(
            wait_done(&backend, &only_b, &ids).await.state,
            JobsState::Succeeded
        );
        assert!(f.root.join("dst/b.txt").exists());
        assert!(f.root.join("dst/a-renamed.txt").exists());
        // ...and an item really missing still fails.
        let only_gone = TransferSpec {
            id: 43,
            items: spec.items[1..].to_vec(),
            ..spec
        };
        let ids = backend.start_transfer(&only_gone, &|_| {}).await.unwrap();
        assert!(matches!(
            wait_done(&backend, &only_gone, &ids).await.state,
            JobsState::Failed(_)
        ));
    }

    #[tokio::test]
    async fn copy_file_and_dir_to_other_folder() {
        let Some((f, backend)) = setup() else { return };
        tree(&f);
        let spec = TransferSpec {
            id: 1,
            kind: TransferKind::Copy,
            src_dir: tst("src"),
            dst_dir: tst("dst"),
            items: vec![item("a.txt", false, "a.txt"), item("d", true, "d")],
        };
        let ids = backend.start_transfer(&spec, &|_| {}).await.unwrap();
        assert_eq!(ids.len(), 2);
        let p = wait_done(&backend, &spec, &ids).await;
        assert_eq!(p.state, JobsState::Succeeded);
        assert!(f.root.join("dst/a.txt").exists());
        assert!(f.root.join("dst/d/x.txt").exists());
        assert!(f.root.join("src/a.txt").exists());
        assert_eq!(p.total_bytes, 5);
    }

    #[tokio::test]
    async fn move_with_rename_removes_source() {
        let Some((f, backend)) = setup() else { return };
        tree(&f);
        let spec = TransferSpec {
            id: 2,
            kind: TransferKind::Move,
            src_dir: tst("src"),
            dst_dir: tst("dst"),
            items: vec![item("a.txt", false, "a (1).txt"), item("d", true, "d2")],
        };
        let ids = backend.start_transfer(&spec, &|_| {}).await.unwrap();
        assert_eq!(wait_done(&backend, &spec, &ids).await.state, JobsState::Succeeded);
        assert!(f.root.join("dst/a (1).txt").exists());
        assert!(f.root.join("dst/d2/x.txt").exists());
        assert!(!f.root.join("src/a.txt").exists());
        assert!(!f.root.join("src/d").exists());
    }

    #[tokio::test]
    async fn copy_between_local_and_remote() {
        let Some((f, backend)) = setup() else { return };
        tree(&f);
        let other = tempfile::tempdir().unwrap();
        std::fs::write(other.path().join("up.bin"), "up").unwrap();
        let spec = TransferSpec {
            id: 3,
            kind: TransferKind::Copy,
            src_dir: Location::new(Remote::Local, &other.path().display().to_string()),
            dst_dir: tst("dst"),
            items: vec![item("up.bin", false, "up.bin")],
        };
        let ids = backend.start_transfer(&spec, &|_| {}).await.unwrap();
        assert_eq!(wait_done(&backend, &spec, &ids).await.state, JobsState::Succeeded);
        assert_eq!(std::fs::read(f.root.join("dst/up.bin")).unwrap(), b"up");
    }

    #[tokio::test]
    async fn failed_job_reports_error() {
        let Some((f, backend)) = setup() else { return };
        tree(&f);
        let spec = TransferSpec {
            id: 4,
            kind: TransferKind::Copy,
            src_dir: tst("src"),
            dst_dir: tst("dst"),
            items: vec![item("missing.txt", false, "missing.txt")],
        };
        let ids = backend.start_transfer(&spec, &|_| {}).await.unwrap();
        match wait_done(&backend, &spec, &ids).await.state {
            JobsState::Failed(msg) => assert!(!msg.is_empty()),
            other => panic!("expected failure, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn unknown_jobids_are_not_found() {
        let Some((_f, backend)) = setup() else { return };
        let p = backend.transfer_progress("rtui/999", &[999_999]).await.unwrap();
        assert_eq!(p.state, JobsState::NotFound);
    }

    #[tokio::test]
    async fn a_thousand_listings_do_not_cost_a_running_transfer_its_progress() {
        let Some((f, backend)) = setup() else { return };
        tree(&f);
        std::fs::write(f.root.join("src/big.bin"), vec![7u8; 4_000_000]).unwrap();
        backend.set_bwlimit("200k").await.unwrap();
        let spec = TransferSpec {
            id: 9,
            kind: TransferKind::Copy,
            src_dir: tst("src"),
            dst_dir: tst("dst"),
            items: vec![item("big.bin", false, "big.bin")],
        };
        let ids = backend.start_transfer(&spec, &|_| {}).await.unwrap();
        let start = Instant::now();
        loop {
            let p = backend.transfer_progress(&spec.group(), &ids).await.unwrap();
            if p.bytes > 0 {
                break;
            }
            assert!(start.elapsed() < Duration::from_secs(10), "the copy never moved");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        // rclone keeps the stats of the newest 1000 groups only, and a listing left
        // to itself makes a group of its own: browsing or a Space scan pushed the
        // transfer's group out, and its progress then read as zero until it ended.
        for _ in 0..1100 {
            backend.list(&tst("dst")).await.unwrap();
        }
        let p = backend.transfer_progress(&spec.group(), &ids).await.unwrap();
        assert_eq!(p.state, JobsState::Running);
        assert!(p.bytes > 0, "the transfer's stats were thrown away");
        assert_eq!(p.total_bytes, 4_000_000);
        backend.stop_transfer(&spec.group()).await.unwrap();
    }

    #[tokio::test]
    async fn stop_transfer_stops_group() {
        let Some((f, backend)) = setup() else { return };
        tree(&f);
        let spec = TransferSpec {
            id: 5,
            kind: TransferKind::Copy,
            src_dir: tst("src"),
            dst_dir: tst("dst"),
            items: vec![item("d", true, "d")],
        };
        let ids = backend.start_transfer(&spec, &|_| {}).await.unwrap();
        backend.stop_transfer(&spec.group()).await.unwrap();
        let p = wait_done(&backend, &spec, &ids).await;
        assert_ne!(p.state, JobsState::Running);
    }

    #[tokio::test]
    async fn job_ids_of_another_group_are_not_ours() {
        let Some((f, backend)) = setup() else { return };
        tree(&f);
        let spec = TransferSpec {
            id: 6,
            kind: TransferKind::Copy,
            src_dir: tst("src"),
            dst_dir: tst("dst"),
            items: vec![item("a.txt", false, "a.txt")],
        };
        let ids = backend.start_transfer(&spec, &|_| {}).await.unwrap();
        wait_done(&backend, &spec, &ids).await;
        // Same job ids, but recorded under another transfer: a reused id after an rcd restart.
        let p = backend.transfer_progress("rtui/999", &ids).await.unwrap();
        assert_eq!(p.state, JobsState::NotFound);
    }

    #[tokio::test]
    async fn many_files_go_in_one_job() {
        let Some((f, backend)) = setup() else { return };
        tree(&f);
        for n in ["b.txt", "c d.txt", "[x]*.txt"] {
            std::fs::write(f.root.join("src").join(n), n).unwrap();
        }
        let spec = TransferSpec {
            id: 7,
            kind: TransferKind::Move,
            src_dir: tst("src"),
            dst_dir: tst("dst"),
            items: vec![
                item("a.txt", false, "a.txt"),
                item("b.txt", false, "b.txt"),
                item("c d.txt", false, "c d.txt"),
                item("[x]*.txt", false, "[x]*.txt"),
                item("d", true, "d"),
            ],
        };
        let ids = backend.start_transfer(&spec, &|_| {}).await.unwrap();
        assert_eq!(ids.len(), 2, "one job for the four files, one for the folder");
        assert_eq!(wait_done(&backend, &spec, &ids).await.state, JobsState::Succeeded);
        for n in ["a.txt", "b.txt", "c d.txt", "[x]*.txt", "d/x.txt"] {
            assert!(f.root.join("dst").join(n).exists(), "{n} missing");
            assert!(!f.root.join("src").join(n).exists(), "{n} not moved");
        }
        assert!(f.root.join("src").is_dir(), "the source folder itself stays");
    }

    /// Two files rclone thinks equal (same size and time, other bytes).
    fn same_looking(f: &RcdFixture, src: &[u8], dst: &[u8]) {
        std::fs::create_dir_all(f.root.join("src")).unwrap();
        std::fs::create_dir_all(f.root.join("dst")).unwrap();
        std::fs::write(f.root.join("src/same"), src).unwrap();
        std::fs::write(f.root.join("dst/same"), dst).unwrap();
        let stamp = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        for p in [f.root.join("src/same"), f.root.join("dst/same")] {
            std::fs::File::options()
                .write(true)
                .open(p)
                .unwrap()
                .set_times(std::fs::FileTimes::new().set_modified(stamp))
                .unwrap();
        }
    }

    #[tokio::test]
    async fn a_chosen_replace_writes_over_a_same_looking_file() {
        for kind in [TransferKind::Move, TransferKind::Copy] {
            let Some((f, backend)) = setup() else { return };
            same_looking(&f, b"GOOD", b"EVIL");
            // With another file, so both the single-file and the batched job are covered.
            for batch in [false, true] {
                if batch {
                    same_looking(&f, b"GOOD", b"EVIL");
                    std::fs::write(f.root.join("src/other"), "o").unwrap();
                }
                let mut items = vec![TransferItem {
                    replace: true,
                    ..item("same", false, "same")
                }];
                if batch {
                    items.push(item("other", false, "other"));
                }
                let spec = TransferSpec {
                    id: 60 + u64::from(batch),
                    kind,
                    src_dir: tst("src"),
                    dst_dir: tst("dst"),
                    items,
                };
                let ids = backend.start_transfer(&spec, &|_| {}).await.unwrap();
                let p = wait_done(&backend, &spec, &ids).await;
                assert_eq!(p.state, JobsState::Succeeded, "{kind:?} batch={batch}");
                assert_eq!(
                    std::fs::read(f.root.join("dst/same")).unwrap(),
                    b"GOOD",
                    "{kind:?} batch={batch}: the destination was not replaced"
                );
            }
        }
    }

    #[tokio::test]
    async fn a_replaced_folder_overwrites_same_looking_files_inside() {
        let Some((f, backend)) = setup() else { return };
        same_looking(&f, b"GOOD", b"EVIL");
        // src/top/same over dst/top/same.
        for side in ["src", "dst"] {
            std::fs::create_dir(f.root.join(side).join("top")).unwrap();
            std::fs::rename(f.root.join(side).join("same"), f.root.join(side).join("top/same")).unwrap();
        }
        let spec = TransferSpec {
            id: 62,
            kind: TransferKind::Move,
            src_dir: tst("src"),
            dst_dir: tst("dst"),
            items: vec![TransferItem {
                replace: true,
                ..item("top", true, "top")
            }],
        };
        let ids = backend.start_transfer(&spec, &|_| {}).await.unwrap();
        assert_eq!(wait_done(&backend, &spec, &ids).await.state, JobsState::Succeeded);
        assert_eq!(std::fs::read(f.root.join("dst/top/same")).unwrap(), b"GOOD");
    }

    #[tokio::test]
    async fn a_batch_with_a_file_gone_from_the_source_is_refused() {
        let Some((f, backend)) = setup() else { return };
        std::fs::create_dir_all(f.root.join("src")).unwrap();
        std::fs::create_dir_all(f.root.join("dst")).unwrap();
        std::fs::write(f.root.join("src/exists"), "e").unwrap();
        let spec = TransferSpec {
            id: 63,
            kind: TransferKind::Copy,
            src_dir: tst("src"),
            dst_dir: tst("dst"),
            items: vec![item("exists", false, "exists"), item("missing", false, "missing")],
        };
        let err = backend.start_transfer(&spec, &|_| {}).await.unwrap_err();
        assert!(err.started.is_empty() && !err.uncertain, "{err:?}");
        assert!(err.error.to_string().contains("missing"), "{err:?}");
        assert!(!f.root.join("dst/exists").exists(), "part of the batch was sent");
    }
}

/// A socket that answers the first rc call with a job id, then drops every connection.
#[tokio::test]
async fn jobs_started_before_a_failure_are_reported() {
    use ratclone::rc::UnixRc;
    use ratclone::transfer::spec::{TransferItem, TransferKind, TransferSpec};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("rc.sock");
    let listener = tokio::net::UnixListener::bind(&sock).unwrap();
    tokio::spawn(async move {
        let mut first = true;
        loop {
            let Ok((mut conn, _)) = listener.accept().await else {
                return;
            };
            let mut buf = vec![0u8; 8192];
            let _ = conn.read(&mut buf).await;
            if first {
                first = false;
                let body = r#"{"jobid":7}"#;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                    body.len()
                );
                let _ = conn.write_all(resp.as_bytes()).await;
            }
            // Later connections are dropped without an answer.
        }
    });
    let backend = RcBackend::new(UnixRc::new(sock));
    let spec = TransferSpec {
        id: 9,
        kind: TransferKind::Copy,
        src_dir: tst("src"),
        dst_dir: tst("dst"),
        items: vec![
            TransferItem {
                replace: false,
                name: "a.txt".into(),
                is_dir: false,
                dst_name: "a.txt".into(),
                size: None,
            },
            TransferItem {
                replace: false,
                name: "d".into(),
                is_dir: true,
                dst_name: "d".into(),
                size: None,
            },
        ],
    };
    let err = backend.start_transfer(&spec, &|_| {}).await.unwrap_err();
    assert_eq!(err.started, vec![7]);
    assert_eq!(err.error, BackendError::Disconnected);
}

// ---- account config calls ----

use ratclone::backend::{ConfigParams, ConfigStep};
use std::collections::BTreeMap;

fn params(kv: &[(&str, &str)], obscure: bool) -> ConfigParams {
    ConfigParams {
        values: kv
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect::<BTreeMap<_, _>>(),
        obscure,
    }
}

fn question_name(step: &ConfigStep) -> &str {
    match step {
        ConfigStep::Question { option, .. } => &option.name,
        other => panic!("expected a question, got {other:?}"),
    }
}

#[tokio::test]
async fn providers_include_drive_mega_dropbox() {
    let Some((_f, backend)) = setup() else { return };
    let p = backend.providers().await.unwrap();
    for name in ["drive", "mega", "dropbox"] {
        assert!(p.iter().any(|p| p.name == name), "{name} missing");
    }
    let mega = p.iter().find(|p| p.name == "mega").unwrap();
    let user = mega.options.iter().find(|o| o.name == "user").unwrap();
    assert!(user.required);
    assert!(
        mega.options
            .iter()
            .find(|o| o.name == "pass")
            .unwrap()
            .is_password
    );
    // Additional provider fields (such as Mega's 2fa) depend on rclone's
    // version. The form renders the options returned by that rclone.
}

#[tokio::test]
async fn create_alias_then_get_then_delete() {
    let Some((f, backend)) = setup() else { return };
    let root = f.root.display().to_string();
    let step = backend
        .config_create("a2", "alias", params(&[("remote", &root)], false))
        .await
        .unwrap();
    assert_eq!(step, ConfigStep::Done);
    assert_eq!(backend.remote_config("a2").await.unwrap()["remote"], root);
    let names = backend.remotes().await.unwrap();
    assert!(names.iter().any(|r| r.remote == Remote::Named("a2".into())));
    backend.config_delete("a2").await.unwrap();
    let names = backend.remotes().await.unwrap();
    assert!(!names.iter().any(|r| r.remote == Remote::Named("a2".into())));
}

#[tokio::test]
async fn create_and_delete_name_with_space() {
    let Some((f, backend)) = setup() else { return };
    let root = f.root.display().to_string();
    let step = backend
        .config_create("my drive", "alias", params(&[("remote", &root)], false))
        .await
        .unwrap();
    assert_eq!(step, ConfigStep::Done);
    let names = backend.remotes().await.unwrap();
    assert!(names.iter().any(|r| r.remote == Remote::Named("my drive".into())));
    backend.config_delete("my drive").await.unwrap();
    assert!(
        backend
            .remote_config("my drive")
            .await
            .map(|c| c.is_empty())
            .unwrap_or(true)
    );
}

#[tokio::test]
async fn mega_password_is_obscured() {
    let Some((_f, backend)) = setup() else { return };
    let step = backend
        .config_create("m", "mega", params(&[("user", "x"), ("pass", "secret")], true))
        .await
        .unwrap();
    assert_eq!(step, ConfigStep::Done);
    let c = backend.remote_config("m").await.unwrap();
    assert_eq!(c["user"], "x");
    assert_ne!(c["pass"], "secret");
}

#[tokio::test]
async fn dropbox_asks_is_local_then_token() {
    let Some((_f, backend)) = setup() else { return };
    let step = backend
        .config_create("d", "dropbox", params(&[], false))
        .await
        .unwrap();
    assert_eq!(question_name(&step), "config_is_local");
    let ConfigStep::Question { state, option, .. } = step else {
        unreachable!()
    };
    assert_eq!(option.kind, "bool");
    let next = backend
        .config_continue("d", &state, "false", false)
        .await
        .unwrap();
    assert_eq!(question_name(&next), "config_token");
}

const FAKE_TOKEN_SECTION: &str = "[d]\ntype = dropbox\ntoken = {\"access_token\":\"x\",\"token_type\":\"bearer\",\"refresh_token\":\"y\",\"expiry\":\"2020-01-01T00:00:00Z\"}\n";

#[tokio::test]
async fn update_sets_value_and_asks_refresh() {
    let Some((f, backend)) = setup() else { return };
    f.add_section(FAKE_TOKEN_SECTION);
    let step = backend
        .config_update("d", params(&[("client_id", "abc")], false))
        .await
        .unwrap();
    assert_eq!(question_name(&step), "config_refresh_token");
    assert_eq!(backend.remote_config("d").await.unwrap()["client_id"], "abc");
}

#[tokio::test]
async fn continue_job_runs_and_stops() {
    let Some((_f, backend)) = setup() else { return };
    let step = backend
        .config_create("d", "dropbox", params(&[], false))
        .await
        .unwrap();
    let ConfigStep::Question { state, .. } = step else {
        panic!()
    };
    let job = backend
        .config_continue_job("d", &state, "false", false)
        .await
        .unwrap();
    let start = std::time::Instant::now();
    let result = loop {
        if let Some(r) = backend.config_job(job).await.unwrap() {
            break r;
        }
        assert!(
            start.elapsed() < std::time::Duration::from_secs(10),
            "job never finished"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    };
    assert_eq!(question_name(&result.unwrap()), "config_token");
    let _ = backend.stop_job(job).await;
}

/// Older rclone must use the token flow; supported browser logins stop cleanly
/// without leaving a listener or token behind.
#[tokio::test]
async fn local_login_requires_cancellable_rclone_or_frees_port() {
    let Some(f) = RcdFixture::start_with_fake_browser() else {
        return;
    };
    let backend = RcBackend::new(f.rc.clone());
    let port_open = || std::net::TcpStream::connect(("127.0.0.1", 53682)).is_ok();
    if port_open() {
        assert!(
            std::env::var_os("RATCLONE_TEST_RCLONE_VERSION").is_none(),
            "required OAuth integration test cannot run: port 53682 already in use"
        );
        eprintln!("skipping: port 53682 already in use");
        return;
    }
    let step = backend
        .config_create("d", "dropbox", params(&[], false))
        .await
        .unwrap();
    let ConfigStep::Question { state, .. } = step else {
        panic!()
    };
    let version: ratclone::rc::types::RcVersion =
        f.rc.call_typed("core/version", serde_json::json!({}))
            .await
            .unwrap();
    let started = backend.config_continue_job("d", &state, "true", false).await;
    if version.decomposed < [1, 75, 0] {
        assert_eq!(started, Err(BackendError::LocalOAuthUnsupported));
        assert!(!port_open(), "unsupported login must not open a listener");
        assert!(!f.dir.path().join("opened.txt").exists(), "browser must not open");
        let token_step = backend
            .config_continue("d", &state, "false", false)
            .await
            .unwrap();
        assert_eq!(question_name(&token_step), "config_token");
        assert!(!backend.remote_config("d").await.unwrap().contains_key("token"));
        return;
    }
    let job = started.unwrap();
    let start = std::time::Instant::now();
    while !port_open() {
        assert!(
            start.elapsed() < std::time::Duration::from_secs(10),
            "login server never started"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(
        backend.config_job(job).await.unwrap(),
        None,
        "still waiting for the browser"
    );
    backend.stop_job(job).await.unwrap();
    let start = std::time::Instant::now();
    let result = loop {
        if let Some(r) = backend.config_job(job).await.unwrap() {
            break r;
        }
        assert!(
            start.elapsed() < std::time::Duration::from_secs(10),
            "stopped job never finished"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    };
    assert!(result.is_err(), "a cancelled login is an error: {result:?}");
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    assert!(!port_open(), "login server still listening");
    assert!(!backend.remote_config("d").await.unwrap().contains_key("token"));
}

#[tokio::test]
async fn invalid_token_is_rejected() {
    let Some((_f, backend)) = setup() else { return };
    let step = backend
        .config_create("d", "dropbox", params(&[], false))
        .await
        .unwrap();
    let ConfigStep::Question { state, .. } = step else {
        panic!()
    };
    let step = backend
        .config_continue("d", &state, "false", false)
        .await
        .unwrap();
    let ConfigStep::Question { state, .. } = step else {
        panic!()
    };
    let step = backend
        .config_continue("d", &state, "notjson", false)
        .await
        .unwrap();
    assert!(
        matches!(step, ConfigStep::Rejected { ref error, .. } if error.contains("decode")),
        "{step:?}"
    );
}

#[tokio::test]
async fn token_question_carries_the_authorize_command() {
    let Some((_f, backend)) = setup() else { return };
    for (name, kv) in [
        ("dd", vec![]),
        ("dc", vec![("client_id", "abc"), ("client_secret", "def")]),
    ] {
        let step = backend
            .config_create(name, "dropbox", params(&kv, false))
            .await
            .unwrap();
        let ConfigStep::Question { state, .. } = step else {
            unreachable!()
        };
        let next = backend
            .config_continue(name, &state, "false", false)
            .await
            .unwrap();
        let ConfigStep::Question { option, .. } = next else {
            unreachable!()
        };
        assert_eq!(option.name, "config_token");
        let cmd = ratclone::app::account::form::authorize_command(&option.help, "dropbox");
        if kv.is_empty() {
            assert_eq!(cmd, "rclone authorize \"dropbox\"");
        } else {
            // The client travels encoded as a second argument.
            let rest = cmd.strip_prefix("rclone authorize \"dropbox\" \"").expect(&cmd);
            assert!(rest.len() > 10 && rest.ends_with('"'), "{cmd}");
        }
    }
}
