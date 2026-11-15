//! Input that is not a plain key press: pasted text, questions that arrive while
//! another dialog is open, and being told to quit from outside.

mod common;

use std::time::Duration;

use common::app::*;
use ratclone::app::overlay::Overlay;
use ratclone::app::{AppEvent, MessageKind, Tab};
use ratclone::location::Remote;

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

fn abouts(f: &ratclone::backend::fake::FakeBackend) -> usize {
    f.calls().iter().filter(|c| *c == "about").count()
}

#[tokio::test]
async fn pasted_text_is_never_obeyed_as_keys() {
    let (_d, fake, mut app) = in_downloads().await;
    move_to(&mut app, "video.mkv");
    // Typed, this is `D` (delete?) … `y` (yes).
    app.handle_paste("Day 1\nyes\n");
    app.settle().await;
    assert!(app.overlay.is_none(), "{:?}", app.overlay);
    assert!(fake.exists(&home().join("Downloads/video.mkv")));
    let m = app.message.as_ref().expect("says why nothing happened");
    assert_eq!(m.kind, MessageKind::Info);
    assert!(m.text.contains("ignored"), "{}", m.text);
}

#[tokio::test]
async fn pasted_text_goes_into_the_field_being_edited_as_one_line() {
    let (_d, fake, mut app) = in_downloads().await;
    move_to(&mut app, "summary.pdf");
    press(&mut app, "r<C-u>");
    app.handle_paste("yearly\treport\r\n");
    match &app.overlay {
        Some(Overlay::Input { input, .. }) => assert_eq!(input.value, "yearlyreport.pdf"),
        other => panic!("expected the rename input, got {other:?}"),
    }
    press(&mut app, "<CR>");
    app.settle().await;
    assert!(fake.exists(&home().join("Downloads/yearlyreport.pdf")));
}

#[tokio::test]
async fn pasting_into_the_filter_filters() {
    let (_d, _fake, mut app) = in_downloads().await;
    press(&mut app, "/");
    app.handle_paste("VID");
    assert_eq!(names(&app), ["video.mkv"]);
}

#[tokio::test]
async fn a_conflict_question_waits_for_the_dialog_already_open() {
    let (_d, fake, mut app) = demo_app().await;
    // Copying a folder next to itself: its name is taken, so a question follows,
    // but only once the destination has been listed.
    fake.set_list_delay(Duration::from_millis(40));
    press(&mut app, "yp");
    // Meanwhile a new folder is being named. `r` and `s` answer the question.
    press(&mut app, "ors");
    app.settle().await;
    match &app.overlay {
        Some(Overlay::Input { input, .. }) => assert_eq!(input.value, "rs", "typing went to the name"),
        other => panic!("the open dialog was replaced by {other:?}"),
    }
    assert!(!fake.calls().contains(&"start_transfer".to_string()));
    press(&mut app, "<Esc>");
    assert!(
        matches!(app.overlay, Some(Overlay::Conflict { .. })),
        "the question shows once the dialog closes: {:?}",
        app.overlay
    );
    press(&mut app, "<Esc>");
    assert!(app.overlay.is_none());
}

#[tokio::test]
async fn ctrl_c_asks_to_quit_like_q() {
    let (_d, _fake, mut app) = demo_app().await;
    press(&mut app, "T<C-c>");
    assert!(
        app.should_quit && app.shutdown_on_exit,
        "nothing running: just quits"
    );

    let (_d, _fake, mut app) = in_downloads().await;
    move_to(&mut app, "video.mkv");
    press(&mut app, "y");
    app.navigate(named("work", ""), None);
    app.settle().await;
    press(&mut app, "p");
    app.settle().await;
    press(&mut app, "<C-c>");
    assert!(!app.should_quit);
    assert!(matches!(app.overlay, Some(Overlay::Quit { .. })), "asks first");
}

#[tokio::test]
async fn a_signal_quits_at_once_and_leaves_transfers_running() {
    let (_d, _fake, mut app) = demo_app().await;
    app.quit_on_signal();
    assert!(
        app.should_quit && app.shutdown_on_exit,
        "idle: rclone is stopped too"
    );

    let (_d, fake, mut app) = in_downloads().await;
    move_to(&mut app, "video.mkv");
    press(&mut app, "y");
    app.navigate(named("work", ""), None);
    app.settle().await;
    press(&mut app, "p");
    app.settle().await;
    app.quit_on_signal();
    assert!(app.should_quit);
    assert!(!app.shutdown_on_exit, "rclone keeps the transfer going");
    assert!(!fake.calls().contains(&"stop_transfer".to_string()));
}

#[tokio::test]
async fn usage_is_asked_again_when_it_changed_or_on_refresh() {
    let (_d, fake, mut app) = in_downloads().await;
    move_to(&mut app, "video.mkv");
    press(&mut app, "y");
    app.navigate(named("work", ""), None);
    app.settle().await;
    let before = abouts(&fake);
    press(&mut app, "p");
    app.settle().await;
    fake.complete_transfers();
    app.handle_event(AppEvent::Tick);
    app.settle().await;
    assert_eq!(app.tab, Tab::Files);
    assert_eq!(abouts(&fake), before + 1, "the copy changed how much work holds");
    assert!(app.sidebar.about.contains_key(&Remote::Named("work".into())));

    press(&mut app, "R");
    app.settle().await;
    assert_eq!(
        abouts(&fake),
        before + 2,
        "R refreshes the usage shown with the folder"
    );

    move_to(&mut app, "video.mkv");
    press(&mut app, "ddy");
    app.settle().await;
    assert_eq!(abouts(&fake), before + 3, "so does deleting");
}

#[tokio::test]
async fn leaving_does_not_wait_for_what_is_only_being_read() {
    let (_d, fake, mut app) = demo_app().await;
    // A folder that takes long to list, and a quit in the middle of it.
    fake.set_list_delay(Duration::from_secs(30));
    move_to(&mut app, "Downloads");
    press(&mut app, "l");
    let start = std::time::Instant::now();
    app.finish_ops().await;
    assert!(
        start.elapsed() < Duration::from_millis(500),
        "waited for a listing"
    );

    // A delete already sent is waited for.
    fake.set_list_delay(Duration::ZERO);
    let (_d, fake, mut app) = in_downloads().await;
    move_to(&mut app, "video.mkv");
    press(&mut app, "ddy");
    app.finish_ops().await;
    assert!(!fake.exists(&home().join("Downloads/video.mkv")));
    assert!(
        app.message
            .as_ref()
            .is_some_and(|m| m.text.starts_with("Deleted"))
    );
}
