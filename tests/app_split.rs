mod common;

use std::time::Duration;

use common::app::*;
use ratclone::app::overlay::Overlay;
use ratclone::app::pane::Listing;
use ratclone::app::{App, AppEvent, Focus, MessageKind, Side};
use ratclone::transfer::store::RecordState;

async fn tick(app: &mut App) {
    app.handle_event(AppEvent::Tick);
    app.settle().await;
}

fn last_message(app: &App) -> (String, MessageKind) {
    let m = app.message.as_ref().expect("a message");
    (m.text.clone(), m.kind)
}

fn listed(pane: &ratclone::app::pane::PaneState) -> Vec<String> {
    pane.entries().map(|e| e.name.clone()).collect()
}

/// Left pane on personal:/Photos/2025 with focus, right pane on work:/.
async fn photos_and_work() -> (
    tempfile::TempDir,
    std::sync::Arc<ratclone::backend::fake::FakeBackend>,
    App,
) {
    let (d, f, mut app) = demo_app().await;
    app.navigate(named("personal", "Photos/2025"), None);
    app.settle().await;
    press(&mut app, "|");
    app.settle().await;
    app.navigate(named("work", ""), None);
    app.settle().await;
    press(&mut app, "<Tab><Tab>");
    assert_eq!(app.side, Side::Left);
    (d, f, app)
}

#[tokio::test]
async fn starts_single() {
    let (_d, _f, app) = demo_app().await;
    assert!(!app.is_split());
    assert_eq!(app.side, Side::Left);
}

#[tokio::test]
async fn bar_opens_next_account_on_the_right() {
    let (_d, _f, mut app) = demo_app().await;
    press(&mut app, "|");
    app.settle().await;
    assert!(app.is_split());
    assert_eq!(app.side, Side::Right);
    assert_eq!(app.focus, Focus::Pane);
    assert_eq!(app.pane.loc, named("personal", ""));
    assert_eq!(app.pane.listing, Listing::Ready);
    assert_eq!(app.left_pane().loc, home());
    assert_eq!(app.right_pane().unwrap().loc, named("personal", ""));
}

#[tokio::test]
async fn tab_cycles_sidebar_left_right() {
    let (_d, _f, mut app) = demo_app().await;
    press(&mut app, "|");
    app.settle().await;
    press(&mut app, "<Tab>");
    assert_eq!(app.focus, Focus::Sidebar);
    press(&mut app, "<Tab>");
    assert_eq!(app.focus, Focus::Pane);
    assert_eq!(app.side, Side::Left);
    assert_eq!(app.pane.loc, home());
    press(&mut app, "<Tab>");
    assert_eq!(app.side, Side::Right);
    assert_eq!(app.pane.loc, named("personal", ""));
    press(&mut app, "<S-Tab>");
    assert_eq!(app.side, Side::Left);
    assert_eq!(app.focus, Focus::Pane);
    press(&mut app, "<S-Tab>");
    assert_eq!(app.focus, Focus::Sidebar);
    press(&mut app, "<S-Tab>");
    assert_eq!(app.focus, Focus::Pane);
    assert_eq!(app.side, Side::Right);
}

#[tokio::test]
async fn bar_closes_and_remembers() {
    let (_d, _f, mut app) = demo_app().await;
    press(&mut app, "|");
    app.settle().await;
    app.navigate(named("personal", "Photos"), None);
    app.settle().await;
    press(&mut app, "|");
    assert!(!app.is_split());
    assert_eq!(app.side, Side::Left);
    assert_eq!(app.pane.loc, home());
    press(&mut app, "|");
    app.settle().await;
    assert_eq!(app.pane.loc, named("personal", "Photos"));
}

#[tokio::test]
async fn closing_from_the_left_keeps_the_left() {
    let (_d, _f, mut app) = demo_app().await;
    press(&mut app, "|");
    app.settle().await;
    press(&mut app, "<S-Tab>");
    assert_eq!(app.side, Side::Left);
    press(&mut app, "|");
    assert!(!app.is_split());
    assert_eq!(app.pane.loc, home());
}

#[tokio::test]
async fn panes_keep_their_own_marks_and_cursor() {
    let (_d, _f, mut app) = demo_app().await;
    press(&mut app, "|");
    app.settle().await;
    press(&mut app, "<Space>");
    press(&mut app, "<Tab><Tab>");
    assert_eq!(app.side, Side::Left);
    assert!(app.pane.marks.is_empty());
    assert_eq!(app.right_pane().unwrap().marks.len(), 1);
}

#[tokio::test]
async fn sidebar_opens_into_focused_pane() {
    let (_d, _f, mut app) = demo_app().await;
    press(&mut app, "|");
    app.settle().await;
    press(&mut app, "<Tab>");
    assert_eq!(app.focus, Focus::Sidebar);
    let work = app
        .sidebar
        .remotes
        .iter()
        .position(|r| r.remote == named("work", "").remote)
        .unwrap();
    app.sidebar.cursor = work;
    press(&mut app, "l");
    app.settle().await;
    assert_eq!(app.side, Side::Right);
    assert_eq!(app.pane.loc, named("work", ""));
    assert_eq!(app.left_pane().loc, home());
}

#[tokio::test]
async fn late_listing_for_closed_pane_is_ignored() {
    let (_d, fake, mut app) = demo_app().await;
    fake.set_list_delay(Duration::from_millis(300));
    press(&mut app, "|");
    press(&mut app, "|");
    app.settle().await;
    assert!(!app.is_split());
    assert_eq!(app.pane.loc, home());
    assert_eq!(app.pane.listing, Listing::Ready);
    assert!(names(&app).contains(&"Downloads".to_string()));
}

#[tokio::test]
async fn last_location_is_the_left_pane() {
    let (_d, _f, mut app) = demo_app().await;
    press(&mut app, "|");
    app.settle().await;
    assert_eq!(app.side, Side::Right);
    assert_eq!(app.left_pane().loc, home());
}

#[tokio::test]
async fn send_copies_marked_files_to_other_pane() {
    let (_d, fake, mut app) = photos_and_work().await;
    move_to(&mut app, "IMG_0012.jpg");
    press(&mut app, ">");
    app.settle().await;
    assert_eq!(app.store.records.len(), 1);
    fake.complete_transfers();
    tick(&mut app).await;
    assert_eq!(app.store.records[0].state, RecordState::Done);
    assert!(fake.exists(&named("work", "IMG_0012.jpg")));
    assert!(fake.exists(&named("personal", "Photos/2025/IMG_0012.jpg")));
    assert!(listed(app.right_pane().unwrap()).contains(&"IMG_0012.jpg".to_string()));
    assert!(app.register.is_none());
}

#[tokio::test]
async fn f6_moves() {
    let (_d, fake, mut app) = photos_and_work().await;
    move_to(&mut app, "IMG_0012.jpg");
    press(&mut app, "<F6>");
    app.settle().await;
    fake.complete_transfers();
    tick(&mut app).await;
    assert!(fake.exists(&named("work", "IMG_0012.jpg")));
    assert!(!fake.exists(&named("personal", "Photos/2025/IMG_0012.jpg")));
    assert!(!names(&app).contains(&"IMG_0012.jpg".to_string()));
    assert!(listed(app.right_pane().unwrap()).contains(&"IMG_0012.jpg".to_string()));
}

#[tokio::test]
async fn send_without_split_says_how() {
    let (_d, fake, mut app) = demo_app().await;
    press(&mut app, ">");
    app.settle().await;
    assert_eq!(
        last_message(&app),
        ("Open a second pane with | first".to_string(), MessageKind::Info)
    );
    assert!(!fake.calls().contains(&"start_transfer".to_string()));
}

#[tokio::test]
async fn send_move_keeps_register() {
    let (_d, fake, mut app) = photos_and_work().await;
    move_to(&mut app, "IMG_0013.jpg");
    press(&mut app, "y");
    move_to(&mut app, "IMG_0012.jpg");
    press(&mut app, "<F6>");
    app.settle().await;
    fake.complete_transfers();
    tick(&mut app).await;
    assert!(fake.exists(&named("work", "IMG_0012.jpg")));
    let reg = app.register.as_ref().expect("the yank survives a send");
    assert_eq!(reg.items[0].name, "IMG_0013.jpg");
}

#[tokio::test]
async fn send_into_itself_is_refused() {
    let (_d, fake, mut app) = demo_app().await;
    app.navigate(named("personal", ""), None);
    app.settle().await;
    press(&mut app, "|");
    app.settle().await;
    app.navigate(named("personal", "Photos"), None);
    app.settle().await;
    press(&mut app, "<Tab><Tab>");
    move_to(&mut app, "Photos");
    press(&mut app, "<F6>");
    app.settle().await;
    assert_eq!(
        last_message(&app),
        (
            "Can't paste “Photos” inside itself".to_string(),
            MessageKind::Error
        )
    );
    assert!(!fake.calls().contains(&"start_transfer".to_string()));
}

#[tokio::test]
async fn other_pane_on_same_folder_refreshes_after_op() {
    let (_d, _f, mut app) = demo_app().await;
    press(&mut app, "|");
    app.settle().await;
    app.navigate(home(), None);
    app.settle().await;
    press(&mut app, "o");
    press(&mut app, "newdir<CR>");
    app.settle().await;
    assert!(names(&app).contains(&"newdir".to_string()));
    assert!(listed(app.left_pane()).contains(&"newdir".to_string()));
}

#[tokio::test]
async fn conflict_on_send_asks() {
    let (_d, fake, mut app) = photos_and_work().await;
    move_to(&mut app, "IMG_0012.jpg");
    press(&mut app, ">");
    app.settle().await;
    fake.complete_transfers();
    tick(&mut app).await;
    move_to(&mut app, "IMG_0012.jpg");
    press(&mut app, ">");
    app.settle().await;
    assert!(matches!(app.overlay, Some(Overlay::Conflict { .. })));
}

#[tokio::test]
async fn hidden_setting_hides_panel_while_split() {
    let (_d, _f, mut app) = demo_app().await;
    app.settings.split_info = ratclone::config::SplitInfo::Hidden;
    assert!(app.panel_visible);
    press(&mut app, "|");
    app.settle().await;
    assert!(!app.panel_visible);
    press(&mut app, "i");
    assert!(app.panel_visible, "i still works by hand");
    press(&mut app, "i");
    press(&mut app, "|");
    assert!(app.panel_visible, "closing puts the panel back");
}

#[tokio::test]
async fn focused_setting_keeps_panel() {
    let (_d, _f, mut app) = demo_app().await;
    press(&mut app, "|");
    app.settle().await;
    assert!(app.panel_visible);
}

#[tokio::test]
async fn reconnect_relists_the_unfocused_pane() {
    let (_d, fake, mut app) = demo_app().await;
    press(&mut app, "|");
    app.settle().await;
    fake.fail_next("list", ratclone::backend::BackendError::Disconnected);
    press(&mut app, "R");
    app.settle().await;
    assert!(matches!(app.pane.listing, Listing::Failed(_)));
    press(&mut app, "<Tab><Tab>");
    assert_eq!(app.side, Side::Left);
    app.handle_event(AppEvent::Reconnected(Ok(false)));
    app.settle().await;
    assert_eq!(app.right_pane().unwrap().listing, Listing::Ready);
}

#[tokio::test]
async fn other_pane_inside_a_deleted_folder_is_relisted() {
    let (_d, _f, mut app) = demo_app().await;
    press(&mut app, "|");
    app.settle().await;
    app.navigate(named("personal", "Photos/2025"), None);
    app.settle().await;
    assert!(!listed(&app.pane).is_empty());
    press(&mut app, "<Tab><Tab>");
    app.navigate(named("personal", "Photos"), None);
    app.settle().await;
    move_to(&mut app, "2025");
    press(&mut app, "dd");
    press(&mut app, "y");
    app.settle().await;
    assert!(!names(&app).contains(&"2025".to_string()));
    let right = app.right_pane().unwrap();
    assert!(
        listed(right).is_empty(),
        "no rows of a folder that is gone: {:?}",
        listed(right)
    );
}

#[tokio::test]
async fn start_jump_does_not_move_a_pane_opened_before_it() {
    use ratclone::config::Config;
    use ratclone::transfer::store::TransferStore;
    let dir = tempfile::tempdir().unwrap();
    let fake = std::sync::Arc::new(ratclone::backend::fake::FakeBackend::demo());
    let store = TransferStore::load(dir.path().join("transfers.json")).unwrap();
    let mut app = App::new(fake.clone(), store, &Config::default(), home());
    app.pending_start = Some(named("personal", ""));
    press(&mut app, "|");
    app.settle().await;
    assert!(app.is_split());
    assert_eq!(
        app.right_pane().unwrap().loc,
        home(),
        "the jump is dropped once the user acts"
    );
    assert_eq!(app.left_pane().loc, home());
}

#[tokio::test]
async fn spinner_runs_while_only_the_unfocused_pane_loads() {
    let (_d, fake, mut app) = demo_app().await;
    fake.set_list_delay(Duration::from_millis(300));
    press(&mut app, "|");
    press(&mut app, "<Tab><Tab>");
    assert_eq!(app.side, Side::Left);
    assert_eq!(app.pane.listing, Listing::Ready);
    assert!(app.is_animating());
    app.settle().await;
}

fn move_to_setting(app: &mut App, id: ratclone::app::settings::SettingId) {
    let Some(Overlay::Settings(dlg)) = app.overlay.as_mut() else {
        panic!("settings not open");
    };
    dlg.cursor = ratclone::app::settings::SettingId::ALL
        .iter()
        .position(|s| *s == id)
        .unwrap();
}

#[tokio::test]
async fn hidden_and_sort_settings_apply_to_both_panes() {
    use ratclone::app::settings::SettingId;
    use ratclone::input::keymap::SortKey;
    let (_d, _f, mut app) = demo_app().await;
    press(&mut app, "|");
    app.settle().await;
    press(&mut app, ",");
    move_to_setting(&mut app, SettingId::Hidden);
    press(&mut app, "l");
    move_to_setting(&mut app, SettingId::Sort);
    press(&mut app, "l");
    assert!(app.left_pane().show_hidden && app.right_pane().unwrap().show_hidden);
    assert_eq!(app.left_pane().sort, SortKey::Size);
    assert_eq!(app.right_pane().unwrap().sort, SortKey::Size);
    press(&mut app, "<Esc>");
    assert!(!app.left_pane().show_hidden && !app.right_pane().unwrap().show_hidden);
    assert_eq!(app.left_pane().sort, SortKey::Name);
    assert_eq!(app.right_pane().unwrap().sort, SortKey::Name);
}

/// From the right pane to the sidebar and back: the panel follows that pane
/// again, not the account the sidebar cursor was left on.
#[tokio::test]
async fn back_from_the_sidebar_to_the_right_pane_shows_its_account() {
    let (_d, _f, mut app) = photos_and_work().await;
    press(&mut app, "<Tab>");
    assert_eq!(app.side, Side::Right);
    press(&mut app, "<Tab>");
    assert_eq!(app.focus, Focus::Sidebar);
    press(&mut app, "gg");
    press(&mut app, "<S-Tab>");
    app.settle().await;
    assert_eq!(app.focus, Focus::Pane);
    assert_eq!(app.side, Side::Right);
    assert_eq!(app.pane.loc, named("work", ""));
    assert_eq!(app.panel_account().unwrap().remote, app.pane.loc.remote);
    // Tab to the left pane: the panel switches to personal.
    press(&mut app, "<Tab><Tab>");
    assert_eq!(app.side, Side::Left);
    assert_eq!(app.panel_account().unwrap().remote, named("personal", "").remote);
}
