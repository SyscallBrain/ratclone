mod common;

use std::sync::Arc;
use std::time::Duration;

use common::app::*;
use ratclone::app::{AppEvent, Focus};
use ratclone::backend::BackendError;
use ratclone::backend::fake::FakeBackend;
use ratclone::scan::tree::ScanTree;
use ratclone::scan::walk::{ScanMsg, start_walk};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

/// Drain a walk into a tree; returns (tree, cancelled, failed rels).
async fn collect(rx: &mut UnboundedReceiver<AppEvent>, root: &str) -> (ScanTree, bool, Vec<String>) {
    let mut tree = ScanTree::new(named(root, ""));
    let mut failed = Vec::new();
    loop {
        let ev = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("walk stalled")
            .expect("channel closed");
        match ev {
            AppEvent::Scan {
                msg: ScanMsg::Listed { rel, result: Ok(e) },
                ..
            } => tree.add_listing(&rel, &e),
            AppEvent::Scan {
                msg: ScanMsg::Listed { rel, result: Err(_) },
                ..
            } => {
                tree.add_unreadable(&rel);
                failed.push(rel);
            }
            AppEvent::Scan {
                msg: ScanMsg::Finished { cancelled },
                ..
            } => return (tree, cancelled, failed),
            other => panic!("unexpected {other:?}"),
        }
    }
}

#[tokio::test]
async fn walk_lists_every_folder_with_bounded_concurrency() {
    let fake = Arc::new(FakeBackend::demo());
    fake.set_list_delay(Duration::from_millis(20));
    let (tx, mut rx) = unbounded_channel();
    let _h = start_walk(fake.clone(), named("personal", ""), 2, 1, tx);
    let (tree, cancelled, failed) = collect(&mut rx, "personal").await;
    assert!(!cancelled);
    assert!(failed.is_empty());
    assert!(fake.max_concurrent_lists() <= 2);
    assert_eq!(fake.max_concurrent_lists(), 2, "it does run in parallel");
    assert!(tree.node("Photos/2025").is_some());
    assert!(tree.node("").unwrap().bytes > 3_000_000_000);
}

#[tokio::test]
async fn unreadable_folder_is_counted_and_scan_continues() {
    let fake = Arc::new(FakeBackend::demo());
    fake.fail_list_at(
        &named("personal", "Backups"),
        BackendError::Rclone("permission denied".into()),
    );
    let (tx, mut rx) = unbounded_channel();
    let _h = start_walk(fake.clone(), named("personal", ""), 4, 1, tx);
    let (tree, cancelled, failed) = collect(&mut rx, "personal").await;
    assert!(!cancelled);
    assert_eq!(failed, vec!["Backups".to_string()]);
    assert_eq!(tree.unreadable, 1);
    assert!(tree.node("Photos/2025").unwrap().files > 0);
}

#[tokio::test]
async fn cancel_sends_one_finished_and_stops_listing() {
    let fake = Arc::new(FakeBackend::demo());
    fake.set_list_delay(Duration::from_millis(50));
    let (tx, mut rx) = unbounded_channel();
    let h = start_walk(fake.clone(), named("personal", ""), 1, 7, tx);
    // Wait for the first folder, then cancel.
    match rx.recv().await.unwrap() {
        AppEvent::Scan {
            id,
            msg: ScanMsg::Listed { .. },
        } => assert_eq!(id, 7),
        other => panic!("{other:?}"),
    }
    h.cancel();
    let mut finished = 0;
    while let Ok(Some(ev)) = tokio::time::timeout(Duration::from_millis(300), rx.recv()).await {
        match ev {
            AppEvent::Scan {
                msg: ScanMsg::Finished { cancelled },
                ..
            } => {
                assert!(cancelled);
                finished += 1;
            }
            AppEvent::Scan {
                msg: ScanMsg::Listed { .. },
                ..
            } => {
                assert_eq!(finished, 0, "nothing after Finished");
            }
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(finished, 1);
    let lists = fake.calls().iter().filter(|c| *c == "list").count();
    assert!(lists < 6, "listing stopped early ({lists} calls)");
}

#[test]
fn concurrency_defaults() {
    use ratclone::scan::concurrency_for;
    assert_eq!(concurrency_for("drive", None), 4);
    assert_eq!(concurrency_for("mega", None), 2);
    assert_eq!(concurrency_for("mega", Some(8)), 8);
    assert_eq!(concurrency_for("drive", Some(0)), 1);
    assert_eq!(concurrency_for("drive", Some(99)), 16);
}

// ---- the Space tab in the app ---------------------------------------------

use ratclone::app::overlay::{Overlay, PendingOp};
use ratclone::app::{App, Tab};
use ratclone::location::Remote;

fn lists(fake: &FakeBackend) -> usize {
    fake.calls().iter().filter(|c| *c == "list").count()
}

fn personal() -> Remote {
    Remote::Named("personal".into())
}

/// Demo app with personal selected in the sidebar and the Space tab open.
async fn space_on_personal() -> (tempfile::TempDir, Arc<FakeBackend>, App) {
    let (d, fake, mut app) = demo_app().await;
    press(&mut app, "<Tab>j<Tab>U");
    app.settle().await;
    assert_eq!(app.tab, Tab::Space);
    (d, fake, app)
}

async fn scanned_personal() -> (tempfile::TempDir, Arc<FakeBackend>, App) {
    let (d, fake, mut app) = space_on_personal().await;
    press(&mut app, "<CR>");
    app.settle().await;
    assert!(app.scan.is_none());
    (d, fake, app)
}

#[tokio::test]
async fn space_needs_enter_to_scan_then_shows_rows() {
    let (_d, fake, mut app) = demo_app().await;
    let before = lists(&fake);
    press(&mut app, "<Tab>j<Tab>U");
    app.settle().await;
    assert!(app.scan.is_none() && app.space_tree().is_none());
    assert_eq!(lists(&fake), before, "a scan never starts by itself");
    press(&mut app, "<CR>");
    app.settle().await;
    let names: Vec<_> = app.space.shown.iter().map(|r| r.name.clone()).collect();
    assert_eq!(names[0], "Photos", "largest first: {names:?}");
    assert!(names.contains(&"Videos".to_string()));
    assert!(!app.space_tree().unwrap().partial);
    press(&mut app, "l");
    assert_eq!(app.space.rel, "Photos");
    press(&mut app, "h");
    assert_eq!(app.space.rel, "");
    assert_eq!(app.space.shown[app.space.cursor].name, "Photos");
    press(&mut app, "h");
    assert_eq!(app.focus, Focus::Pane, "h at the scan root stays in the panel");
    assert_eq!(app.space.rel, "");
}

#[tokio::test]
async fn esc_cancels_scan_first_then_leaves() {
    let (_d, fake, mut app) = space_on_personal().await;
    fake.set_list_delay(Duration::from_millis(200));
    press(&mut app, "<CR>");
    assert!(app.scan.is_some());
    press(&mut app, "<Esc>");
    app.settle().await;
    assert!(app.scan.is_none());
    assert_eq!(app.tab, Tab::Space);
    assert!(app.space_tree().unwrap().partial);
    press(&mut app, "<Esc>");
    assert_eq!(app.tab, Tab::Files);
}

#[tokio::test]
async fn scan_survives_tab_and_account_switch() {
    let (_d, fake, mut app) = space_on_personal().await;
    fake.set_list_delay(Duration::from_millis(30));
    press(&mut app, "<CR>");
    press(&mut app, "gt");
    assert_eq!(app.tab, Tab::Transfers);
    press(&mut app, "U<Tab>j");
    assert_eq!(app.tab, Tab::Space);
    app.settle().await;
    let root = ratclone::location::Location::new(personal(), "");
    let tree = app.scans.get(&root).expect("personal scan finished");
    assert!(!tree.partial);
    assert!(tree.node("Photos/2025").is_some());
    assert!(app.space_tree().is_none(), "work was never scanned");
}

#[tokio::test]
async fn second_scan_asks_to_replace_first() {
    let (_d, fake, mut app) = space_on_personal().await;
    fake.set_list_delay(Duration::from_millis(200));
    press(&mut app, "<CR>");
    press(&mut app, "<Tab>j<Tab><CR>");
    assert!(matches!(
        app.overlay,
        Some(Overlay::Confirm {
            on_yes: PendingOp::ReplaceScan(_),
            ..
        })
    ));
    press(&mut app, "y");
    let active = app.scan.as_ref().expect("a scan runs");
    assert_eq!(active.tree.root.remote, Remote::Named("work".into()));
    app.settle().await;
}

#[tokio::test]
async fn rescan_asks_when_scan_exists() {
    let (_d, fake, mut app) = scanned_personal().await;
    let before = lists(&fake);
    press(&mut app, "R");
    assert!(matches!(
        app.overlay,
        Some(Overlay::Confirm {
            on_yes: PendingOp::Rescan(_),
            ..
        })
    ));
    press(&mut app, "n");
    assert!(app.overlay.is_none() && app.scan.is_none());
    assert_eq!(lists(&fake), before);
    press(&mut app, "Ry");
    assert!(app.scan.is_some());
    app.settle().await;
}

#[tokio::test]
async fn gf_opens_folder_in_files() {
    let (_d, _f, mut app) = scanned_personal().await;
    assert_eq!(app.space.shown[app.space.cursor].name, "Photos");
    press(&mut app, "gf");
    app.settle().await;
    assert_eq!(app.tab, Tab::Files);
    assert_eq!(app.pane.loc.display(), "personal:/Photos");
}

#[tokio::test]
async fn changes_after_scan_are_flagged() {
    let (_d, _f, mut app) = scanned_personal().await;
    assert!(!app.changed_since_scan.contains_key(&personal()));
    app.set_tab(Tab::Files);
    app.navigate(named("personal", ""), None);
    app.settle().await;
    press(&mut app, "onew folder<CR>");
    app.settle().await;
    assert!(app.changed_since_scan.contains_key(&personal()));
}

#[tokio::test]
async fn scan_is_cached_and_reloaded() {
    let cache = tempfile::tempdir().unwrap();
    {
        let (_d, _f, mut app) = demo_app().await;
        app.scan_cache_dir = Some(cache.path().to_path_buf());
        press(&mut app, "<Tab>j<Tab>U<CR>");
        app.settle().await;
        assert!(app.space_tree().is_some());
    }
    let (_d, fake, mut app) = demo_app().await;
    app.scan_cache_dir = Some(cache.path().to_path_buf());
    let before = lists(&fake);
    press(&mut app, "<Tab>j<Tab>U");
    app.settle().await;
    assert!(app.space_tree().is_some());
    assert_eq!(lists(&fake), before);
    assert_eq!(app.space.shown[0].name, "Photos");
}

#[tokio::test]
async fn space_filter_and_sort() {
    let (_d, _f, mut app) = scanned_personal().await;
    press(&mut app, "sn");
    assert_eq!(app.space.shown[0].name, "Backups");
    press(&mut app, "/vid<CR>");
    let names: Vec<_> = app.space.shown.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names[0], "Videos");
    assert!(!names.contains(&"Photos"));
}

// ---- final review fixes ------------------------------------------------------

#[tokio::test]
async fn local_space_root_never_borrows_a_remote_path() {
    let (_d, _f, mut app) = demo_app().await;
    app.navigate(named("personal", "Photos"), None);
    app.settle().await;
    press(&mut app, "<Tab>gg<Tab>U");
    let root = app.space_root().unwrap();
    assert_eq!(
        root,
        home(),
        "local Space root is the home folder, not personal's path"
    );
}

#[tokio::test]
async fn q_leaves_space_without_cancelling_the_scan() {
    let (_d, fake, mut app) = space_on_personal().await;
    fake.set_list_delay(Duration::from_millis(100));
    press(&mut app, "<CR>");
    press(&mut app, "q");
    assert_eq!(app.tab, Tab::Files);
    assert!(app.scan.is_some(), "scan keeps running");
    app.settle().await;
    let root = ratclone::location::Location::new(personal(), "");
    assert!(!app.scans[&root].partial);
}

#[tokio::test]
async fn rescan_of_another_account_asks_to_replace_the_running_scan() {
    let (_d, fake, mut app) = space_on_personal().await;
    press(&mut app, "<CR>");
    app.settle().await; // personal scanned
    press(&mut app, "<Tab>j<Tab><CR>");
    app.settle().await; // work scanned
    fake.set_list_delay(Duration::from_millis(200));
    press(&mut app, "<Tab>k<Tab>Ry"); // rescan personal, now running
    assert!(app.scan.is_some());
    press(&mut app, "<Tab>j<Tab>R"); // R on work while personal scans
    assert!(matches!(
        app.overlay,
        Some(Overlay::Confirm {
            on_yes: PendingOp::ReplaceScan(_),
            ..
        })
    ));
    press(&mut app, "n<Tab>k<Tab>R"); // R on personal, which is scanning
    assert!(
        app.overlay.is_none(),
        "no rescan prompt for the scan already running"
    );
    app.cancel_scan();
    app.settle().await;
}

#[tokio::test]
async fn cancelled_rescan_keeps_the_complete_scan() {
    let (_d, fake, mut app) = scanned_personal().await;
    let root = ratclone::location::Location::new(personal(), "");
    let full = app.scans[&root].node.bytes;
    fake.set_list_delay(Duration::from_millis(200));
    press(&mut app, "Ry");
    press(&mut app, "<Esc>");
    app.settle().await;
    let kept = &app.scans[&root];
    assert!(!kept.partial);
    assert_eq!(kept.node.bytes, full);
}

// ---- pending items -------------------------------------------------------------

#[tokio::test]
async fn replaced_scan_keeps_its_partial_tree() {
    let (_d, fake, mut app) = space_on_personal().await;
    fake.set_list_delay(Duration::from_millis(100));
    press(&mut app, "<CR>");
    // Let the root listing arrive.
    tokio::time::sleep(Duration::from_millis(150)).await;
    while let Some(ev) = app.try_next_event() {
        app.handle_event(ev);
    }
    press(&mut app, "<Tab>j<Tab><CR>y");
    app.settle().await;
    let root = ratclone::location::Location::new(personal(), "");
    let kept = app.scans.get(&root).expect("partial personal scan kept");
    assert!(kept.partial);
}

#[tokio::test]
async fn change_during_scan_stays_flagged() {
    let (_d, fake, mut app) = space_on_personal().await;
    fake.set_list_delay(Duration::from_millis(100));
    press(&mut app, "<CR>");
    // A change made while the scan runs (it may already have listed that folder).
    app.note_change(&personal());
    app.settle().await;
    assert!(app.changed_since_scan.contains_key(&personal()));
}

#[tokio::test]
async fn a_listing_that_panics_counts_as_unreadable() {
    let fake = Arc::new(FakeBackend::demo());
    fake.panic_list_at(&named("personal", "Music"));
    let (tx, mut rx) = unbounded_channel();
    let _h = start_walk(fake.clone(), named("personal", ""), 2, 1, tx);
    let (tree, cancelled, failed) = collect(&mut rx, "personal").await;
    assert!(!cancelled);
    assert_eq!(failed, vec!["Music".to_string()]);
    assert_eq!(tree.unreadable, 1);
}

#[tokio::test]
async fn panel_title_shows_scan_progress() {
    let (_d, fake, mut app) = space_on_personal().await;
    fake.set_list_delay(Duration::from_millis(300));
    press(&mut app, "<CR>");
    assert!(app.scan.is_some());
    let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(110, 30)).unwrap();
    term.draw(|f| ratclone::ui::draw(f, &mut app)).unwrap();
    let buf = term.backend().buffer().clone();
    let row: String = (0..110).map(|x| buf[(x, 21)].symbol().to_string()).collect();
    assert!(row.contains("scanning"), "{row}");
    app.cancel_scan();
    app.settle().await;
}

// ---- scans that could not read everything ----------------------------------------

fn message(app: &App) -> (String, ratclone::app::MessageKind) {
    let m = app.message.as_ref().expect("a message");
    (m.text.clone(), m.kind)
}

#[tokio::test]
async fn rescan_that_cannot_read_the_top_keeps_the_previous_scan() {
    use ratclone::app::MessageKind;
    let cache = tempfile::tempdir().unwrap();
    let (_d, fake, mut app) = space_on_personal().await;
    app.scan_cache_dir = Some(cache.path().to_path_buf());
    press(&mut app, "<CR>");
    app.settle().await;
    let root = ratclone::location::Location::new(personal(), "");
    let full = app.scans[&root].node.bytes;
    assert!(full > 0);
    fake.fail_next("list", BackendError::Disconnected);
    press(&mut app, "Ry");
    app.settle().await;
    assert_eq!(app.scans[&root].node.bytes, full, "replaced by an empty scan");
    let (text, kind) = message(&app);
    assert_eq!(kind, MessageKind::Error);
    assert!(
        text.contains("Could not scan personal") && text.contains("kept the previous"),
        "{text}"
    );
    let cached = ratclone::scan::cache::load(cache.path(), &root).expect("cache kept");
    assert_eq!(cached.node.bytes, full);
}

#[tokio::test]
async fn first_scan_that_cannot_read_the_top_stores_nothing() {
    use ratclone::app::MessageKind;
    let (_d, fake, mut app) = space_on_personal().await;
    fake.fail_next("list", BackendError::Rclone("token expired".into()));
    press(&mut app, "<CR>");
    app.settle().await;
    assert!(
        app.space_tree().is_none(),
        "no 0 B scan shown as the account's size"
    );
    let (text, kind) = message(&app);
    assert_eq!(kind, MessageKind::Error);
    assert!(text.contains("token expired"), "{text}");
}

#[tokio::test]
async fn scan_missing_a_subfolder_says_it_is_incomplete() {
    use ratclone::app::MessageKind;
    let (_d, fake, mut app) = space_on_personal().await;
    fake.fail_list_at(
        &named("personal", "Photos"),
        BackendError::Rclone("permission denied".into()),
    );
    press(&mut app, "<CR>");
    app.settle().await;
    let tree = app.space_tree().expect("kept: the rest was read");
    assert_eq!(tree.unreadable, 1);
    let (text, kind) = message(&app);
    assert_eq!(kind, MessageKind::Error);
    assert!(
        text.contains("incomplete") && text.contains("1 folder could not"),
        "{text}"
    );
}

#[tokio::test]
async fn empty_account_scan_is_a_complete_success() {
    use ratclone::app::MessageKind;
    let (_d, fake, mut app) = demo_app().await;
    fake.add_remote("void", "drive");
    app.sidebar.remotes.push(ratclone::backend::RemoteInfo {
        remote: Remote::Named("void".into()),
        kind: "drive".into(),
    });
    let root = named("void", "");
    app.start_scan(root.clone());
    app.settle().await;
    let tree = &app.scans[&root];
    assert_eq!(
        (tree.node.bytes, tree.node.files, tree.unreadable, tree.partial),
        (0, 0, 0, false)
    );
    let (text, kind) = message(&app);
    assert_eq!(kind, MessageKind::Success);
    assert!(text.contains("0 B in 0 files"), "{text}");
}

#[tokio::test]
async fn ctrl_n_and_ctrl_p_change_the_space_account() {
    let (_d, _f, mut app) = demo_app().await;
    press(&mut app, "U");
    app.settle().await;
    let second = app.sidebar.remotes[1].remote.clone();
    press(&mut app, "<C-n>");
    app.settle().await;
    assert_eq!(app.focus, ratclone::app::Focus::Pane);
    assert_eq!(app.sidebar.cursor, 1);
    assert_eq!(app.space_root().unwrap().remote, second);
    press(&mut app, "<C-p>");
    app.settle().await;
    assert_eq!(app.sidebar.cursor, 0);
}
