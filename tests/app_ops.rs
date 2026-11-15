mod common;

use common::app::*;
use ratclone::app::overlay::Overlay;
use ratclone::app::{AppEvent, MessageKind};
use ratclone::transfer::store::RecordState;

fn h(p: &str) -> ratclone::location::Location {
    home().join(p)
}

async fn in_downloads() -> (
    tempfile::TempDir,
    std::sync::Arc<ratclone::backend::fake::FakeBackend>,
    ratclone::app::App,
) {
    let (d, f, mut app) = demo_app().await;
    move_to(&mut app, "Downloads");
    press(&mut app, "l");
    app.settle().await;
    (d, f, app)
}

async fn tick(app: &mut ratclone::app::App) {
    app.handle_event(AppEvent::Tick);
    app.settle().await;
}

fn last_message(app: &ratclone::app::App) -> (String, MessageKind) {
    let m = app.message.as_ref().expect("a message");
    (m.text.clone(), m.kind)
}

#[tokio::test]
async fn rename_file_and_reject_existing_name() {
    let (_d, fake, mut app) = in_downloads().await;
    move_to(&mut app, "summary.pdf");
    press(&mut app, "cw");
    match &app.overlay {
        Some(Overlay::Input { input, .. }) => {
            assert_eq!(input.value, "summary.pdf");
            assert_eq!(input.cursor, 7, "cursor sits before the extension");
        }
        other => panic!("expected rename input, got {other:?}"),
    }
    press(&mut app, "<C-u>report<CR>");
    app.settle().await;
    assert!(fake.exists(&h("Downloads/report.pdf")));
    assert!(!fake.exists(&h("Downloads/summary.pdf")));
    assert_eq!(app.pane.current().unwrap().name, "report.pdf");

    press(&mut app, "r<C-e><C-u>video.mkv<CR>");
    app.settle().await;
    assert!(fake.exists(&h("Downloads/report.pdf")));
    let (text, kind) = last_message(&app);
    assert_eq!(kind, MessageKind::Error);
    assert!(text.contains("already exists"), "{text}");
}

#[tokio::test]
async fn rename_rejects_slash_and_empty() {
    let (_d, fake, mut app) = in_downloads().await;
    move_to(&mut app, "summary.pdf");
    press(&mut app, "r<C-e><C-u>a/b<CR>");
    app.settle().await;
    assert_eq!(last_message(&app).1, MessageKind::Error);
    press(&mut app, "r<C-e><C-u><CR>");
    app.settle().await;
    assert!(fake.exists(&h("Downloads/summary.pdf")));
}

#[tokio::test]
async fn mkdir_puts_cursor_on_new_folder() {
    let (_d, fake, mut app) = in_downloads().await;
    press(&mut app, "oNew<CR>");
    app.settle().await;
    assert!(fake.exists(&h("Downloads/New")));
    assert_eq!(app.pane.current().unwrap().name, "New");
}

#[tokio::test]
async fn delete_asks_and_shows_command() {
    let (_d, fake, mut app) = in_downloads().await;
    move_to(&mut app, "summary.pdf");
    press(&mut app, "dd");
    match &app.overlay {
        Some(Overlay::Confirm { commands, lines, .. }) => {
            assert_eq!(commands, &["rclone deletefile /home/demo/Downloads/summary.pdf"]);
            assert_eq!(lines, &["summary.pdf"]);
        }
        other => panic!("expected confirm, got {other:?}"),
    }
    press(&mut app, "n");
    app.settle().await;
    assert!(app.overlay.is_none());
    assert!(fake.exists(&h("Downloads/summary.pdf")));
    press(&mut app, "Dy");
    app.settle().await;
    assert!(!fake.exists(&h("Downloads/summary.pdf")));
    assert!(!names(&app).contains(&"summary.pdf".to_string()));
}

#[tokio::test]
async fn delete_many_lists_first_ten() {
    let (_d, _f, mut app) = demo_app().await;
    app.navigate(named("personal", "Photos/2025"), None);
    app.settle().await;
    press(&mut app, "vG");
    press(&mut app, "dd");
    match &app.overlay {
        Some(Overlay::Confirm { title, lines, .. }) => {
            assert!(title.contains("22 items"), "{title}");
            assert_eq!(lines.len(), 11);
            assert_eq!(lines[10], "…and 12 more");
        }
        other => panic!("expected confirm, got {other:?}"),
    }
}

#[tokio::test]
async fn yank_paste_into_other_remote_starts_transfer() {
    let (_d, fake, mut app) = demo_app().await;
    app.navigate(named("personal", "Photos/2025"), None);
    app.settle().await;
    move_to(&mut app, "IMG_0012.jpg");
    press(&mut app, "y");
    assert!(app.register.is_some());
    app.navigate(named("work", ""), None);
    app.settle().await;
    press(&mut app, "p");
    app.settle().await;
    assert_eq!(app.store.records.len(), 1);
    assert_eq!(app.store.records[0].state, RecordState::Running);
    fake.complete_transfers();
    tick(&mut app).await;
    assert_eq!(app.store.records[0].state, RecordState::Done);
    assert!(fake.exists(&named("work", "IMG_0012.jpg")));
    assert!(
        names(&app).contains(&"IMG_0012.jpg".to_string()),
        "destination pane refreshed"
    );
    assert!(app.register.is_some(), "a copy register survives the paste");
    let (text, kind) = last_message(&app);
    assert_eq!(kind, MessageKind::Success);
    assert!(
        !text.starts_with('✓'),
        "the status line adds the icon itself: {text}"
    );
}

#[tokio::test]
async fn cut_paste_moves_and_clears_register() {
    let (_d, fake, mut app) = demo_app().await;
    app.navigate(named("work", ""), None);
    app.settle().await;
    move_to(&mut app, "notes.md");
    press(&mut app, "x");
    app.navigate(named("personal", ""), None);
    app.settle().await;
    press(&mut app, "p");
    app.settle().await;
    assert!(app.register.is_none());
    fake.complete_transfers();
    tick(&mut app).await;
    assert!(fake.exists(&named("personal", "notes.md")));
    assert!(!fake.exists(&named("work", "notes.md")));
}

#[tokio::test]
async fn paste_folder_into_itself_is_refused() {
    let (_d, fake, mut app) = demo_app().await;
    app.navigate(named("personal", ""), None);
    app.settle().await;
    move_to(&mut app, "Photos");
    press(&mut app, "y");
    app.navigate(named("personal", "Photos/2025"), None);
    app.settle().await;
    press(&mut app, "p");
    app.settle().await;
    assert_eq!(last_message(&app).1, MessageKind::Error);
    assert!(!fake.calls().contains(&"start_transfer".to_string()));
}

#[tokio::test]
async fn move_into_same_folder_is_refused() {
    let (_d, fake, mut app) = in_downloads().await;
    move_to(&mut app, "video.mkv");
    press(&mut app, "xp");
    app.settle().await;
    assert_eq!(last_message(&app).1, MessageKind::Error);
    assert!(!fake.calls().contains(&"start_transfer".to_string()));
}

#[tokio::test]
async fn conflict_rename_and_apply_all() {
    let (_d, _f, mut app) = demo_app().await;
    app.navigate(named("personal", "Photos/2025"), None);
    app.settle().await;
    app.navigate(named("work", ""), None);
    app.settle().await;
    // Put two same-named files in work first.
    app.navigate(named("personal", "Photos/2025"), None);
    app.settle().await;
    move_to(&mut app, "IMG_0010.jpg");
    press(&mut app, "<Space><Space>y");
    app.navigate(named("work", ""), None);
    app.settle().await;
    press(&mut app, "p");
    app.settle().await;
    _f.complete_transfers();
    tick(&mut app).await;
    // Paste again: both names now clash.
    press(&mut app, "p");
    app.settle().await;
    match &app.overlay {
        Some(Overlay::Conflict { names, .. }) => assert_eq!(names, &["IMG_0010.jpg", "IMG_0011.jpg"]),
        other => panic!("expected conflict dialog, got {other:?}"),
    }
    press(&mut app, "an");
    app.settle().await;
    assert!(app.overlay.is_none());
    let spec = &app.store.records.last().unwrap().spec;
    let dst: Vec<&str> = spec.items.iter().map(|i| i.dst_name.as_str()).collect();
    assert_eq!(dst, ["IMG_0010 (1).jpg", "IMG_0011 (1).jpg"]);
}

#[tokio::test]
async fn conflict_skip_all_starts_nothing() {
    let (_d, fake, mut app) = in_downloads().await;
    move_to(&mut app, "video.mkv");
    press(&mut app, "y");
    press(&mut app, "p");
    app.settle().await;
    press(&mut app, "as");
    app.settle().await;
    assert!(app.store.records.is_empty());
    assert!(!fake.calls().contains(&"start_transfer".to_string()));
}

#[tokio::test]
async fn copy_into_same_folder_defaults_to_rename() {
    let (_d, fake, mut app) = in_downloads().await;
    move_to(&mut app, "video.mkv");
    press(&mut app, "yp");
    app.settle().await;
    press(&mut app, "<CR>");
    app.settle().await;
    fake.complete_transfers();
    tick(&mut app).await;
    assert!(fake.exists(&h("Downloads/video (1).mkv")));
}

#[tokio::test]
async fn disconnect_marks_running_interrupted() {
    let (_d, fake, mut app) = in_downloads().await;
    move_to(&mut app, "video.mkv");
    press(&mut app, "y");
    app.navigate(named("work", ""), None);
    app.settle().await;
    press(&mut app, "p");
    app.settle().await;
    assert_eq!(app.store.records[0].state, RecordState::Running);

    fake.set_disconnected(true);
    let f2 = fake.clone();
    app.set_reconnect(std::sync::Arc::new(move || {
        let f = f2.clone();
        Box::pin(async move {
            f.set_disconnected(false);
            f.forget_transfers();
            Ok(false)
        })
    }));
    tick(&mut app).await;
    tick(&mut app).await;
    assert_eq!(app.store.records[0].state, RecordState::Interrupted);
    press(&mut app, "k");
    assert_eq!(app.pane.cursor, 0, "still responds to keys");
}

#[tokio::test]
async fn quitting_waits_for_a_delete_the_app_is_doing() {
    for key in ["b", "c"] {
        let (_d, fake, mut app) = in_downloads().await;
        move_to(&mut app, "summary.pdf");
        press(&mut app, "<Space>");
        move_to(&mut app, "video.mkv");
        press(&mut app, "<Space>ddy");
        // The deletes are sent one by one by the app: not answered yet.
        press(&mut app, "q");
        assert!(matches!(app.overlay, Some(Overlay::Quit { running: 1, ops: 1 })));
        press(&mut app, key);
        assert!(!app.should_quit, "{key}: quit before the delete was over");
        app.settle().await;
        assert!(app.should_quit, "{key}");
        assert_eq!(app.shutdown_on_exit, key == "c");
        assert!(!fake.exists(&home().join("Downloads/summary.pdf")));
        assert!(
            !fake.exists(&home().join("Downloads/video.mkv")),
            "{key}: the second delete was dropped"
        );
    }
}

#[tokio::test]
async fn quit_with_running_transfers_asks() {
    let (_d, fake, mut app) = in_downloads().await;
    move_to(&mut app, "video.mkv");
    press(&mut app, "y");
    app.navigate(named("work", ""), None);
    app.settle().await;
    press(&mut app, "p");
    app.settle().await;
    press(&mut app, "q");
    assert!(matches!(app.overlay, Some(Overlay::Quit { running: 1, .. })));
    press(&mut app, "<Esc>");
    assert!(app.overlay.is_none() && !app.should_quit);
    press(&mut app, "qb");
    assert!(app.should_quit);
    assert!(!app.shutdown_on_exit);
    assert!(!fake.calls().contains(&"stop_transfer".to_string()));
}

#[tokio::test]
async fn quit_cancel_stops_transfers() {
    let (_d, fake, mut app) = in_downloads().await;
    move_to(&mut app, "video.mkv");
    press(&mut app, "y");
    app.navigate(named("work", ""), None);
    app.settle().await;
    press(&mut app, "pqc");
    app.settle().await;
    assert!(app.should_quit && app.shutdown_on_exit);
    assert!(fake.calls().contains(&"stop_transfer".to_string()));
    assert_eq!(app.store.records[0].state, RecordState::Stopped);
}

#[tokio::test]
async fn quit_wait_exits_when_done() {
    let (_d, fake, mut app) = in_downloads().await;
    move_to(&mut app, "video.mkv");
    press(&mut app, "y");
    app.navigate(named("work", ""), None);
    app.settle().await;
    press(&mut app, "pqw");
    app.settle().await;
    assert!(!app.should_quit);
    fake.complete_transfers();
    tick(&mut app).await;
    assert!(app.should_quit && app.shutdown_on_exit);
}

#[tokio::test]
async fn escape_clears_register() {
    let (_d, _f, mut app) = in_downloads().await;
    press(&mut app, "y<Esc>");
    assert!(app.register.is_none());
}

async fn one_running_transfer() -> (
    tempfile::TempDir,
    std::sync::Arc<ratclone::backend::fake::FakeBackend>,
    ratclone::app::App,
) {
    let (d, fake, mut app) = in_downloads().await;
    move_to(&mut app, "video.mkv");
    press(&mut app, "y");
    app.navigate(named("work", ""), None);
    app.settle().await;
    press(&mut app, "p");
    app.settle().await;
    (d, fake, app)
}

#[tokio::test]
async fn stop_then_resume_relaunches_with_new_id() {
    let (_d, fake, mut app) = one_running_transfer().await;
    press(&mut app, "T");
    assert_eq!(app.tab, ratclone::app::Tab::Transfers);
    let old = app.store.records[0].spec.id;
    press(&mut app, "s");
    app.settle().await;
    assert_eq!(app.store.records[0].state, RecordState::Stopped);
    assert!(fake.calls().contains(&"stop_transfer".to_string()));
    press(&mut app, "<CR>");
    app.settle().await;
    assert_eq!(app.store.records.len(), 1);
    let rec = &app.store.records[0];
    assert_eq!(rec.state, RecordState::Running);
    assert_ne!(rec.spec.id, old);
    press(&mut app, "q");
    assert_eq!(
        app.tab,
        ratclone::app::Tab::Files,
        "q closes the view, does not quit"
    );
    assert!(!app.should_quit);
}

#[tokio::test]
async fn clear_finished_keeps_failed() {
    let (_d, fake, mut app) = one_running_transfer().await;
    fake.complete_transfers();
    tick(&mut app).await;
    // A second transfer that fails.
    app.navigate(home().join("Downloads"), None);
    app.settle().await;
    move_to(&mut app, "summary.pdf");
    press(&mut app, "y");
    fake.delete_now(&home().join("Downloads/summary.pdf"));
    app.navigate(named("work", ""), None);
    app.settle().await;
    press(&mut app, "p");
    app.settle().await;
    fake.complete_transfers();
    tick(&mut app).await;
    let states: Vec<RecordState> = app.store.records.iter().map(|r| r.state).collect();
    assert_eq!(states, [RecordState::Done, RecordState::Failed]);
    press(&mut app, "Tc");
    assert_eq!(app.store.records.len(), 1);
    assert_eq!(app.store.records[0].state, RecordState::Failed);
}

#[tokio::test]
async fn transfers_view_lists_running_first() {
    let (_d, fake, mut app) = one_running_transfer().await;
    fake.complete_transfers();
    tick(&mut app).await;
    app.navigate(home().join("Downloads"), None);
    app.settle().await;
    move_to(&mut app, "summary.pdf");
    press(&mut app, "y");
    app.navigate(named("personal", ""), None);
    app.settle().await;
    press(&mut app, "p");
    app.settle().await;
    let order = app.transfer_order();
    let first = app.store.records.iter().find(|r| r.spec.id == order[0]).unwrap();
    assert_eq!(first.state, RecordState::Running);
}

#[tokio::test]
async fn back_while_loading_never_acts_on_old_entries() {
    let (_d, fake, mut app) = in_downloads().await;
    fake.add_file(&h("video.mkv"), 10);
    move_to(&mut app, "video.mkv");
    fake.set_list_delay(std::time::Duration::from_millis(500));
    press(&mut app, "-");
    press(&mut app, "dd");
    if app.overlay.is_some() {
        press(&mut app, "y");
    }
    press(&mut app, "y");
    app.settle().await;
    assert!(
        fake.exists(&h("video.mkv")),
        "a file in the new folder was deleted"
    );
    assert!(fake.exists(&h("Downloads/video.mkv")));
    if let Some(r) = &app.register {
        assert!(
            r.items.iter().all(|e| app.pane.all_names().contains(&e.name)),
            "register holds names from another folder"
        );
    }
}

#[tokio::test]
async fn quit_waits_for_running_delete() {
    let (_d, _fake, mut app) = in_downloads().await;
    move_to(&mut app, "summary.pdf");
    press(&mut app, "ddy");
    // The delete is in flight (not settled yet): quitting must ask, not stop rclone under it.
    press(&mut app, "q");
    assert!(!app.should_quit, "quit while a delete is running");
    assert!(matches!(app.overlay, Some(Overlay::Quit { .. })));
}

#[tokio::test]
async fn reconnect_to_a_new_rcd_interrupts_running() {
    let (_d, fake, mut app) = one_running_transfer().await;
    fake.set_disconnected(true);
    let f2 = fake.clone();
    app.set_reconnect(std::sync::Arc::new(move || {
        let f = f2.clone();
        Box::pin(async move {
            f.set_disconnected(false);
            Ok(true) // a fresh rcd was started
        })
    }));
    tick(&mut app).await;
    assert_eq!(app.store.records[0].state, RecordState::Interrupted);
}

#[tokio::test]
async fn failed_resume_keeps_the_record() {
    let (_d, fake, mut app) = one_running_transfer().await;
    press(&mut app, "Ts");
    app.settle().await;
    let old = app.store.records[0].spec.id;
    fake.fail_next(
        "start_transfer",
        ratclone::backend::BackendError::Rclone("token expired".into()),
    );
    press(&mut app, "<CR>");
    app.settle().await;
    assert_eq!(app.store.records.len(), 1, "the stopped transfer vanished");
    assert_eq!(app.store.records[0].spec.id, old);
    assert_eq!(app.store.records[0].state, RecordState::Stopped);
}

#[tokio::test]
async fn a_stopped_transfer_can_be_removed_from_the_list() {
    for keys in ["dd", "D"] {
        let (_d, _fake, mut app) = one_running_transfer().await;
        press(&mut app, "Ts");
        app.settle().await;
        assert_eq!(app.store.records[0].state, RecordState::Stopped);
        press(&mut app, keys);
        assert!(
            matches!(app.overlay, Some(Overlay::Confirm { .. })),
            "{keys} asks first"
        );
        press(&mut app, "n");
        assert_eq!(app.store.records.len(), 1, "cancelled: still listed");
        press(&mut app, keys);
        press(&mut app, "y");
        assert!(app.store.records.is_empty(), "{keys} removed it");
        let (_, kind) = last_message(&app);
        assert_eq!(kind, MessageKind::Success);
    }
}

#[tokio::test]
async fn a_running_transfer_cannot_be_removed() {
    let (_d, _fake, mut app) = one_running_transfer().await;
    press(&mut app, "Tdd");
    assert!(app.overlay.is_none());
    assert_eq!(app.store.records.len(), 1);
    assert_eq!(app.store.records[0].state, RecordState::Running);
    let (text, _) = last_message(&app);
    assert!(text.contains("stop it first"), "{text}");
    press(&mut app, "C");
    assert!(app.overlay.is_none(), "nothing to clear");
    assert_eq!(app.store.records.len(), 1);
}

#[tokio::test]
async fn clear_all_keeps_only_running_transfers() {
    let (_d, _fake, mut app) = one_running_transfer().await;
    press(&mut app, "Ts");
    app.settle().await;
    // A second transfer that keeps running.
    press(&mut app, "T");
    app.navigate(home().join("Downloads"), None);
    app.settle().await;
    move_to(&mut app, "summary.pdf");
    press(&mut app, "y");
    app.navigate(named("personal", ""), None);
    app.settle().await;
    press(&mut app, "p");
    app.settle().await;
    let states: Vec<RecordState> = app.store.records.iter().map(|r| r.state).collect();
    assert_eq!(states, [RecordState::Stopped, RecordState::Running]);
    press(&mut app, "Tc");
    assert_eq!(app.store.records.len(), 2, "c keeps the stopped one");
    press(&mut app, "C");
    assert!(matches!(app.overlay, Some(Overlay::Confirm { .. })));
    press(&mut app, "y");
    assert_eq!(app.store.records.len(), 1);
    assert_eq!(app.store.records[0].state, RecordState::Running);
    assert_eq!(app.transfers.cursor, 0);
}

#[tokio::test]
async fn wait_and_quit_still_quits_when_the_last_launch_fails() {
    let (_d, fake, mut app) = in_downloads().await;
    move_to(&mut app, "video.mkv");
    press(&mut app, "y");
    app.navigate(named("work", ""), None);
    app.settle().await;
    fake.fail_next(
        "start_transfer",
        ratclone::backend::BackendError::Rclone("quota exceeded".into()),
    );
    press(&mut app, "pqw");
    app.settle().await;
    assert!(app.should_quit, "waiting forever after the launch failed");
}

#[tokio::test]
async fn resume_twice_relaunches_once() {
    let (_d, fake, mut app) = one_running_transfer().await;
    press(&mut app, "Ts");
    app.settle().await;
    press(&mut app, "<CR><CR>");
    app.settle().await;
    let starts = fake.calls().iter().filter(|c| *c == "start_transfer").count();
    assert_eq!(starts, 2, "initial start plus exactly one resume");
    assert_eq!(app.store.records.len(), 1);
}

#[tokio::test]
async fn a_failing_progress_query_is_reported_once() {
    let (_d, fake, mut app) = one_running_transfer().await;
    fake.set_progress_error(Some(ratclone::backend::BackendError::Rclone("boom".into())));
    tick(&mut app).await;
    assert!(app.message.as_ref().is_some_and(|m| m.text.contains("boom")));
    app.message = None;
    tick(&mut app).await;
    tick(&mut app).await;
    assert!(app.message.is_none(), "the same error is repeated every second");
}

#[tokio::test]
async fn esc_peels_one_layer_at_a_time() {
    let (_d, _f, mut app) = in_downloads().await;
    move_to(&mut app, "video.mkv");
    press(&mut app, "y/i<CR>");
    move_to(&mut app, "video.mkv");
    press(&mut app, "<Space>");
    assert!(!app.pane.marks.is_empty() && !app.pane.filter.is_empty() && app.register.is_some());
    press(&mut app, "<Esc>");
    assert!(app.pane.marks.is_empty());
    assert!(
        !app.pane.filter.is_empty() && app.register.is_some(),
        "Esc cleared more than the marks"
    );
    press(&mut app, "<Esc>");
    assert!(app.pane.filter.is_empty() && app.register.is_some());
    press(&mut app, "<Esc>");
    assert!(app.register.is_none());
}

#[tokio::test]
async fn a_partly_started_paste_is_tracked_and_resumable() {
    let (_d, fake, mut app) = in_downloads().await;
    move_to(&mut app, "summary.pdf");
    press(&mut app, "<Space>");
    move_to(&mut app, "video.mkv");
    press(&mut app, "<Space>y");
    app.navigate(named("work", ""), None);
    app.settle().await;
    // The first item's job starts, then the connection times out.
    fake.fail_start_after(
        1,
        ratclone::backend::BackendError::Other("rclone took too long".into()),
    );
    press(&mut app, "p");
    app.settle().await;
    assert_eq!(app.store.records.len(), 1, "the started job is not tracked");
    assert_eq!(app.store.records[0].state, RecordState::Running);
    press(&mut app, "q");
    assert!(
        matches!(app.overlay, Some(Overlay::Quit { .. })),
        "quit would stop rclone under the started job"
    );
    press(&mut app, "<Esc>");

    fake.complete_transfers();
    tick(&mut app).await;
    let rec = &app.store.records[0];
    assert_eq!(rec.state, RecordState::Failed, "only part of the paste was done");
    assert!(
        rec.error.as_deref().unwrap_or("").contains("not started"),
        "{:?}",
        rec.error
    );
    assert!(fake.exists(&named("work", "summary.pdf")));
    assert!(!fake.exists(&named("work", "video.mkv")));

    press(&mut app, "T<CR>");
    app.settle().await;
    fake.complete_transfers();
    tick(&mut app).await;
    assert!(
        fake.exists(&named("work", "video.mkv")),
        "resume did not finish the rest"
    );
    assert_eq!(app.store.records[0].state, RecordState::Done);
}

#[tokio::test]
async fn a_start_whose_answer_was_lost_is_found_again_and_followed() {
    let (_d, fake, mut app) = in_downloads().await;
    move_to(&mut app, "video.mkv");
    press(&mut app, "y");
    app.navigate(named("work", ""), None);
    app.settle().await;
    // rclone takes the job, but the connection drops before its id comes back.
    fake.lose_start_reply_after(0);
    press(&mut app, "p");
    app.settle().await;
    assert_eq!(
        app.store.records.len(),
        1,
        "the record of a job rclone runs was dropped"
    );
    assert_eq!(app.store.records[0].state, RecordState::Starting);
    press(&mut app, "q");
    assert!(
        matches!(app.overlay, Some(Overlay::Quit { .. })),
        "quit would stop rclone under the job"
    );
    press(&mut app, "<Esc>");
    // The next poll asks rclone by group, and follows what it finds.
    tick(&mut app).await;
    let rec = &app.store.records[0];
    assert_eq!(rec.state, RecordState::Running);
    assert_eq!(rec.jobids.len(), 1);
    fake.complete_transfers();
    tick(&mut app).await;
    assert_eq!(app.store.records[0].state, RecordState::Done);
    assert!(fake.exists(&named("work", "video.mkv")));
    let starts = fake.calls().iter().filter(|c| *c == "start_transfer").count();
    assert_eq!(starts, 1, "the start was sent again");
}

#[tokio::test]
async fn finished_transfer_drops_cached_rows_of_its_destination() {
    let (_d, fake, mut app) = demo_app().await;
    app.navigate(named("personal", "Photos/2025"), None);
    app.settle().await;
    move_to(&mut app, "IMG_0012.jpg");
    press(&mut app, "y");
    app.navigate(named("work", ""), None);
    app.settle().await;
    press(&mut app, "p");
    app.settle().await;
    // Remember work as it is while the copy runs, then look elsewhere.
    app.refresh_pane();
    app.settle().await;
    app.navigate(named("personal", ""), None);
    app.settle().await;
    fake.complete_transfers();
    tick(&mut app).await;
    assert_eq!(app.store.records[0].state, RecordState::Done);
    fake.set_list_delay(std::time::Duration::from_millis(300));
    app.navigate(named("work", ""), None);
    assert_eq!(
        app.pane.listing,
        ratclone::app::pane::Listing::Loading,
        "no rows from before the copy"
    );
    app.settle().await;
    assert!(names(&app).contains(&"IMG_0012.jpg".to_string()));
}

// ---- stop, launch and resume reconciled with rclone ----

use ratclone::backend::BackendError;
use ratclone::transfer::spec::{JobsState, TransferProgress};
use ratclone::transfer::store::TransferStore;

/// Handle events until `done` holds (or a second has passed).
async fn until(app: &mut ratclone::app::App, done: impl Fn(&ratclone::app::App) -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while !done(app) && std::time::Instant::now() < deadline {
        if let Ok(Some(ev)) =
            tokio::time::timeout(std::time::Duration::from_millis(50), app.next_event()).await
        {
            app.handle_event(ev);
        }
    }
    assert!(done(app), "condition not reached");
}

#[tokio::test]
async fn a_refused_stop_leaves_the_transfer_running() {
    for err in [BackendError::Other("refused".into()), BackendError::Disconnected] {
        let (_d, fake, mut app) = one_running_transfer().await;
        press(&mut app, "T");
        fake.fail_next("stop_transfer", err.clone());
        press(&mut app, "s");
        assert!(app.is_stopping(app.store.records[0].spec.id));
        assert_eq!(
            app.store.records[0].state,
            RecordState::Running,
            "not before rclone answers"
        );
        app.settle().await;
        let (text, kind) = last_message(&app);
        assert_eq!(kind, MessageKind::Error);
        assert!(
            text.contains("Could not stop") || text.contains("Lost connection"),
            "{text}"
        );
        assert_eq!(app.store.records[0].state, RecordState::Running);
        assert!(!app.is_stopping(app.store.records[0].spec.id));
        assert_eq!(app.active_transfers(), 1);
        fake.set_disconnected(false);
        // Still followed: it runs to its end.
        fake.complete_transfers();
        tick(&mut app).await;
        assert_eq!(app.store.records[0].state, RecordState::Done, "after {err:?}");
    }
}

#[tokio::test]
async fn a_failure_seen_while_stopping_ends_as_stopped() {
    let (_d, _fake, mut app) = one_running_transfer().await;
    let id = app.store.records[0].spec.id;
    press(&mut app, "T");
    press(&mut app, "s");
    // The cancelled job is seen failing before the stop request answers.
    app.handle_event(AppEvent::Progress {
        id,
        result: Ok(TransferProgress {
            state: JobsState::Failed("context canceled".into()),
            bytes: 0,
            total_bytes: 0,
            speed: 0.0,
            eta: None,
            errors: 1,
            files: vec![],
            done_jobs: vec![],
        }),
    });
    assert_eq!(app.store.records[0].state, RecordState::Stopped);
    until(&mut app, |a| !a.is_stopping(id)).await;
    assert_eq!(app.store.records[0].state, RecordState::Stopped);
}

#[tokio::test]
async fn a_stop_that_answers_after_the_end_keeps_the_end() {
    let (_d, fake, mut app) = one_running_transfer().await;
    let id = app.store.records[0].spec.id;
    // It finished in rclone just before the stop arrived; the app had not seen it yet.
    fake.complete_transfers();
    press(&mut app, "T");
    press(&mut app, "s");
    until(&mut app, |a| {
        !a.is_stopping(id) && a.store.records[0].state != RecordState::Running
    })
    .await;
    assert_eq!(app.store.records[0].state, RecordState::Done);
}

async fn two_files_copied_slowly() -> (
    tempfile::TempDir,
    std::sync::Arc<ratclone::backend::fake::FakeBackend>,
    ratclone::app::App,
) {
    let (d, fake, mut app) = in_downloads().await;
    move_to(&mut app, "summary.pdf");
    press(&mut app, "<Space>");
    move_to(&mut app, "video.mkv");
    press(&mut app, "<Space>y");
    app.navigate(named("work", ""), None);
    app.settle().await;
    fake.set_start_delay(std::time::Duration::from_millis(300));
    press(&mut app, "p");
    // The paste check, then the first job.
    until(&mut app, |a| {
        a.store.records.first().is_some_and(|r| r.jobids.len() == 1)
    })
    .await;
    (d, fake, app)
}

#[tokio::test]
async fn each_started_job_is_saved_during_the_launch() {
    let (d, _fake, mut app) = two_files_copied_slowly().await;
    let on_disk = TransferStore::load(d.path().join("transfers.json")).unwrap();
    assert_eq!(on_disk.records[0].state, RecordState::Starting);
    assert_eq!(on_disk.records[0].jobids, app.store.records[0].jobids);
    app.settle().await;
    let on_disk = TransferStore::load(d.path().join("transfers.json")).unwrap();
    assert_eq!(on_disk.records[0].state, RecordState::Running);
    assert_eq!(on_disk.records[0].jobids.len(), 2);
}

#[tokio::test]
async fn quitting_in_the_background_waits_for_the_launch() {
    let (_d, fake, mut app) = two_files_copied_slowly().await;
    press(&mut app, "q");
    assert!(matches!(app.overlay, Some(Overlay::Quit { .. })));
    press(&mut app, "b");
    assert!(!app.should_quit, "the second item is not in rclone yet");
    assert!(app.quit_when_launched);
    app.settle().await;
    assert!(app.should_quit && !app.shutdown_on_exit);
    assert_eq!(app.store.records[0].state, RecordState::Running);
    assert_eq!(app.store.records[0].jobids.len(), 2);
    assert_eq!(
        fake.calls().iter().filter(|c| *c == "start_transfer").count(),
        1,
        "nothing submitted twice"
    );
}

#[tokio::test]
async fn esc_cancels_a_quit_waiting_for_a_launch() {
    let (_d, _fake, mut app) = two_files_copied_slowly().await;
    press(&mut app, "qb<Esc>");
    app.settle().await;
    assert!(!app.should_quit);
}

/// An earlier run saved `spec` as starting; rclone accepted `accepted` of its items.
async fn reopened_after_a_cut_launch(
    accepted: usize,
) -> (
    tempfile::TempDir,
    std::sync::Arc<ratclone::backend::fake::FakeBackend>,
    ratclone::app::App,
) {
    use ratclone::backend::Backend;
    use ratclone::transfer::spec::{TransferItem, TransferKind, TransferSpec};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("transfers.json");
    let fake = std::sync::Arc::new(ratclone::backend::fake::FakeBackend::demo());
    let mut store = TransferStore::load(path.clone()).unwrap();
    let item = |n: &str, size| TransferItem {
        replace: false,
        name: n.into(),
        is_dir: false,
        dst_name: n.into(),
        size: Some(size),
    };
    let spec = TransferSpec {
        id: store.next_id(),
        kind: TransferKind::Copy,
        src_dir: h("Downloads"),
        dst_dir: named("work", ""),
        items: vec![item("summary.pdf", 2_200_000), item("video.mkv", 1_500_000_000)],
    };
    store.push_starting(spec.clone(), None, jiff::Timestamp::now());
    store.save().unwrap();
    // rclone took `accepted` jobs, then the app was gone before it heard their ids.
    if accepted > 0 {
        fake.fail_start_after(accepted, BackendError::Other("app closed".into()));
        let _ = fake.start_transfer(&spec, &|_| {}).await;
    }
    let store = TransferStore::load(path).unwrap();
    let mut app = ratclone::app::App::new(fake.clone(), store, &ratclone::config::Config::default(), home());
    app.settle().await;
    tick(&mut app).await;
    (dir, fake, app)
}

#[tokio::test]
async fn a_cut_launch_is_reconciled_and_resumed_without_repeats() {
    let (_d, fake, mut app) = reopened_after_a_cut_launch(1).await;
    let rec = &app.store.records[0];
    assert_eq!(rec.state, RecordState::Running);
    assert_eq!(rec.jobids.len(), 1, "the job rclone accepted is followed");
    assert!(rec.start_error.is_some(), "and the item not started is known");
    fake.complete_transfers();
    tick(&mut app).await;
    assert_eq!(app.store.records[0].state, RecordState::Failed);
    assert!(fake.exists(&named("work", "summary.pdf")));
    fake.delete_now(&named("work", "summary.pdf"));
    press(&mut app, "T<CR>");
    app.settle().await;
    let rec = &app.store.records[0];
    let names: Vec<&str> = rec.spec.items.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names, ["video.mkv"], "only the item that never started");
    fake.complete_transfers();
    tick(&mut app).await;
    assert_eq!(app.store.records[0].state, RecordState::Done);
    assert!(
        !fake.exists(&named("work", "summary.pdf")),
        "not copied a second time"
    );
}

#[tokio::test]
async fn a_launch_cut_before_rclone_had_anything_is_resumable() {
    let (_d, _fake, mut app) = reopened_after_a_cut_launch(0).await;
    let rec = &app.store.records[0];
    assert_eq!(rec.state, RecordState::Interrupted);
    assert_eq!(app.active_transfers(), 0);
    press(&mut app, "T<CR>");
    app.settle().await;
    assert_eq!(app.store.records.len(), 1);
    assert_eq!(app.store.records[0].state, RecordState::Running);
    assert_eq!(app.store.records[0].jobids.len(), 2);
}

async fn moving_two_files() -> (
    tempfile::TempDir,
    std::sync::Arc<ratclone::backend::fake::FakeBackend>,
    ratclone::app::App,
) {
    let (d, fake, mut app) = in_downloads().await;
    move_to(&mut app, "summary.pdf");
    press(&mut app, "<Space>");
    move_to(&mut app, "video.mkv");
    press(&mut app, "<Space>x");
    app.navigate(named("work", ""), None);
    app.settle().await;
    press(&mut app, "p");
    app.settle().await;
    assert_eq!(app.store.records[0].state, RecordState::Running);
    (d, fake, app)
}

#[tokio::test]
async fn resuming_an_interrupted_move_skips_items_already_moved() {
    let (_d, fake, mut app) = moving_two_files().await;
    // rclone moved summary.pdf, then was restarted: nobody saw that job end.
    fake.delete_now(&h("Downloads/summary.pdf"));
    fake.add_file(&named("work", "summary.pdf"), 2_200_000);
    fake.forget_transfers();
    tick(&mut app).await;
    assert_eq!(app.store.records[0].state, RecordState::Interrupted);
    press(&mut app, "T<CR>");
    app.settle().await;
    let rec = &app.store.records[0];
    let names: Vec<&str> = rec.spec.items.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names, ["video.mkv"]);
    fake.complete_transfers();
    tick(&mut app).await;
    assert_eq!(
        app.store.records[0].state,
        RecordState::Done,
        "{:?}",
        app.store.records[0].error
    );
    assert!(fake.exists(&named("work", "video.mkv")));
}

#[tokio::test]
async fn resuming_a_move_skips_jobs_seen_to_succeed() {
    let (_d, fake, mut app) = moving_two_files().await;
    // video.mkv vanished from the source: its job fails, summary.pdf's succeeds.
    fake.delete_now(&h("Downloads/video.mkv"));
    fake.complete_transfers();
    tick(&mut app).await;
    let rec = &app.store.records[0];
    assert_eq!(rec.state, RecordState::Failed);
    assert_eq!(rec.done_jobs.len(), 1);
    press(&mut app, "T<CR>");
    app.settle().await;
    let rec = &app.store.records[0];
    let names: Vec<&str> = rec.spec.items.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names, ["video.mkv"]);
    // It really is gone, with nothing like it at the destination: that still fails.
    fake.complete_transfers();
    tick(&mut app).await;
    assert_eq!(app.store.records[0].state, RecordState::Failed);
}

#[tokio::test]
async fn resuming_a_move_with_everything_arrived_just_ends_it() {
    let (_d, fake, mut app) = moving_two_files().await;
    for (n, size) in [("summary.pdf", 2_200_000), ("video.mkv", 1_500_000_000)] {
        fake.delete_now(&h(&format!("Downloads/{n}")));
        fake.add_file(&named("work", n), size);
    }
    fake.forget_transfers();
    tick(&mut app).await;
    press(&mut app, "T<CR>");
    app.settle().await;
    assert_eq!(app.store.records.len(), 1);
    assert_eq!(app.store.records[0].state, RecordState::Done);
    assert_eq!(
        fake.calls().iter().filter(|c| *c == "start_transfer").count(),
        1,
        "nothing relaunched"
    );
}
