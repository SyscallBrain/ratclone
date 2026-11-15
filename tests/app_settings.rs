mod common;

use common::app::*;
use ratclone::app::App;
use ratclone::app::overlay::Overlay;
use ratclone::app::settings::{SettingId, display_value};
use ratclone::config::IconStyle;
use ratclone::input::keymap::SortKey;

async fn with_config() -> (
    tempfile::TempDir,
    std::sync::Arc<ratclone::backend::fake::FakeBackend>,
    App,
) {
    let (d, f, mut app) = demo_app().await;
    app.config_path = Some(d.path().join("config.toml"));
    app.themes_dir = d.path().join("themes");
    (d, f, app)
}

fn move_to_setting(app: &mut App, id: SettingId) {
    let Some(Overlay::Settings(dlg)) = app.overlay.as_mut() else {
        panic!("settings not open");
    };
    dlg.cursor = SettingId::ALL.iter().position(|s| *s == id).unwrap();
}

#[tokio::test]
async fn comma_opens_settings_and_esc_restores() {
    let (d, _f, mut app) = with_config().await;
    press(&mut app, ",");
    assert!(matches!(app.overlay, Some(Overlay::Settings(_))));
    press(&mut app, "l");
    assert_eq!(app.theme.name, "tokyo-night-storm", "theme previews live");
    press(&mut app, "<Esc>");
    assert!(app.overlay.is_none());
    assert_eq!(app.theme.name, "tokyo-night");
    assert!(!d.path().join("config.toml").exists());
}

#[tokio::test]
async fn enter_saves_changed_values() {
    let (d, _f, mut app) = with_config().await;
    press(&mut app, ",");
    move_to_setting(&mut app, SettingId::Icons);
    press(&mut app, "l<CR>");
    assert!(app.overlay.is_none());
    assert_eq!(app.icons, IconStyle::Ascii);
    let text = std::fs::read_to_string(d.path().join("config.toml")).unwrap();
    assert!(text.contains("icons = \"ascii\""), "{text}");
    assert_eq!(app.settings.icons, IconStyle::Ascii);
}

#[tokio::test]
async fn key_hints_and_panel_and_hidden_apply_live() {
    let (_d, _f, mut app) = with_config().await;
    press(&mut app, ",");
    move_to_setting(&mut app, SettingId::KeyHints);
    press(&mut app, "l");
    assert!(!app.key_hints);
    move_to_setting(&mut app, SettingId::InfoPanel);
    press(&mut app, "l");
    assert!(!app.panel_visible);
    move_to_setting(&mut app, SettingId::Hidden);
    press(&mut app, "l");
    assert!(app.pane.show_hidden);
    press(&mut app, "<Esc>");
    assert!(app.key_hints && app.panel_visible && !app.pane.show_hidden);
}

#[tokio::test]
async fn default_sort_applies_to_pane() {
    let (_d, _f, mut app) = with_config().await;
    press(&mut app, ",");
    move_to_setting(&mut app, SettingId::Sort);
    press(&mut app, "l");
    assert_eq!(app.pane.sort, SortKey::Size);
    press(&mut app, "<CR>");
    assert_eq!(app.pane.sort, SortKey::Size);
}

#[tokio::test]
async fn save_error_keeps_changes_and_reports() {
    let (d, _f, mut app) = with_config().await;
    let blocker = d.path().join("file");
    std::fs::write(&blocker, "x").unwrap();
    app.config_path = Some(blocker.join("config.toml")); // parent is a file
    press(&mut app, ",");
    move_to_setting(&mut app, SettingId::Icons);
    press(&mut app, "l<CR>");
    assert_eq!(app.icons, IconStyle::Ascii, "still applied");
    let msg = app.message.as_ref().unwrap().text.clone();
    assert!(msg.starts_with("Settings applied but not saved"), "{msg}");
}

#[tokio::test]
async fn broken_custom_theme_is_listed_not_applied() {
    let (d, _f, mut app) = with_config().await;
    std::fs::create_dir_all(d.path().join("themes")).unwrap();
    std::fs::write(d.path().join("themes/zz-bad.toml"), "not a theme").unwrap();
    press(&mut app, ",");
    press(&mut app, "h"); // wraps to the last theme: the custom one
    let Some(Overlay::Settings(dlg)) = app.overlay.as_ref() else {
        panic!()
    };
    let shown = display_value(SettingId::Theme, &dlg.values, &dlg.themes, &app.sidebar.remotes);
    assert!(shown.ends_with("(broken)"), "{shown}");
    assert_eq!(app.theme.name, "tokyo-night", "a broken theme is not applied");
    press(&mut app, "<Esc>");
}

#[test]
fn display_values() {
    use ratclone::config::{Config, SettingValues};
    let v = SettingValues::from_config(&Config::default());
    let themes = vec![ratclone::app::settings::ThemeChoice {
        name: "tokyo-night".into(),
        broken: false,
    }];
    assert_eq!(display_value(SettingId::Theme, &v, &themes, &[]), "Tokyo Night");
    assert_eq!(
        display_value(SettingId::Transparent, &v, &themes, &[]),
        "theme default"
    );
    assert_eq!(display_value(SettingId::Icons, &v, &themes, &[]), "Nerd Font");
    assert_eq!(
        display_value(SettingId::ScanParallel, &v, &themes, &[]),
        "auto (4, Mega 2)"
    );
    assert_eq!(display_value(SettingId::Bwlimit, &v, &themes, &[]), "off");
    assert_eq!(
        display_value(SettingId::StartIn, &v, &themes, &[]),
        "last location"
    );
    assert_eq!(
        display_value(SettingId::SplitInfo, &v, &themes, &[]),
        "focused pane"
    );
}

#[tokio::test]
async fn split_info_follows_info_panel_and_saves() {
    let all = SettingId::ALL;
    let i = all.iter().position(|s| *s == SettingId::InfoPanel).unwrap();
    assert_eq!(all[i + 1], SettingId::SplitInfo);
    let (d, _f, mut app) = with_config().await;
    press(&mut app, ",");
    move_to_setting(&mut app, SettingId::SplitInfo);
    press(&mut app, "l<CR>");
    assert_eq!(app.settings.split_info, ratclone::config::SplitInfo::Hidden);
    let text = std::fs::read_to_string(d.path().join("config.toml")).unwrap();
    assert!(text.contains("info_panel_split = \"hidden\""), "{text}");
}

// ---- behaviour settings ------------------------------------------------------

use ratclone::app::settings::resolve_start;
use ratclone::config::OnQuit;
use ratclone::location::{Location, Remote};

async fn one_running_transfer() -> (
    tempfile::TempDir,
    std::sync::Arc<ratclone::backend::fake::FakeBackend>,
    App,
) {
    let (d, f, mut app) = with_config().await;
    move_to(&mut app, "Downloads");
    press(&mut app, "l");
    app.settle().await;
    move_to(&mut app, "video.mkv");
    press(&mut app, "y");
    app.navigate(named("work", ""), None);
    app.settle().await;
    press(&mut app, "p");
    app.settle().await;
    assert_eq!(app.store.running().count(), 1);
    (d, f, app)
}

#[tokio::test]
async fn on_quit_background_quits_without_dialog() {
    let (_d, _f, mut app) = one_running_transfer().await;
    app.on_quit = OnQuit::Background;
    press(&mut app, "q");
    assert!(app.overlay.is_none());
    assert!(app.should_quit && !app.shutdown_on_exit);
}

#[tokio::test]
async fn on_quit_wait_sets_quit_when_idle() {
    let (_d, _f, mut app) = one_running_transfer().await;
    app.on_quit = OnQuit::Wait;
    press(&mut app, "q");
    assert!(app.overlay.is_none() && !app.should_quit && app.quit_when_idle);
}

#[tokio::test]
async fn on_quit_ask_shows_dialog() {
    let (_d, _f, mut app) = one_running_transfer().await;
    press(&mut app, "q");
    assert!(matches!(app.overlay, Some(Overlay::Quit { .. })));
}

#[test]
fn start_location_rules() {
    let remotes = vec!["personal".to_string()];
    let last = Location::new(Remote::Named("personal".into()), "Photos");
    assert_eq!(resolve_start("last", Some(&last), &remotes), Some(last.clone()));
    let gone = Location::new(Remote::Named("old".into()), "x");
    assert_eq!(resolve_start("last", Some(&gone), &remotes), None);
    assert_eq!(resolve_start("last", None, &remotes), None);
    assert_eq!(resolve_start("home", Some(&last), &remotes), None);
    assert_eq!(
        resolve_start("personal", None, &remotes),
        Some(Location::new(Remote::Named("personal".into()), ""))
    );
    let local = Location::new(Remote::Local, "home/demo/Downloads");
    assert_eq!(resolve_start("last", Some(&local), &remotes), Some(local));
}

#[tokio::test]
async fn last_location_of_missing_remote_falls_back_home() {
    let dir = tempfile::tempdir().unwrap();
    let fake = std::sync::Arc::new(ratclone::backend::fake::FakeBackend::demo());
    let store = ratclone::transfer::store::TransferStore::load(dir.path().join("t.json")).unwrap();
    let mut app = App::new(fake, store, &ratclone::config::Config::default(), home());
    app.pending_start = Some(Location::new(Remote::Named("gone".into()), "x"));
    app.settle().await;
    assert_eq!(app.pane.loc, home());
    let dir2 = tempfile::tempdir().unwrap();
    let fake = std::sync::Arc::new(ratclone::backend::fake::FakeBackend::demo());
    let store = ratclone::transfer::store::TransferStore::load(dir2.path().join("t.json")).unwrap();
    let mut app = App::new(fake, store, &ratclone::config::Config::default(), home());
    app.pending_start = Some(named("personal", "Photos"));
    app.settle().await;
    assert_eq!(app.pane.loc, named("personal", "Photos"));
}

#[tokio::test]
async fn bwlimit_sent_on_save_only_when_changed() {
    let (_d, fake, mut app) = with_config().await;
    press(&mut app, ",");
    move_to_setting(&mut app, SettingId::Bwlimit);
    press(&mut app, "lll<CR>");
    app.settle().await;
    assert_eq!(fake.bwlimit().as_deref(), Some("10M"));
    assert_eq!(app.bwlimit_shown.as_deref(), Some("10M"));
    let sent = |f: &ratclone::backend::fake::FakeBackend| {
        f.calls().iter().filter(|c| c.starts_with("bwlimit")).count()
    };
    assert_eq!(sent(&fake), 1);
    press(&mut app, ",<CR>");
    app.settle().await;
    assert_eq!(sent(&fake), 1, "unchanged: not sent again");
}

#[tokio::test]
async fn invalid_bwlimit_is_rejected() {
    let (_d, fake, mut app) = with_config().await;
    press(&mut app, ",");
    move_to_setting(&mut app, SettingId::Bwlimit);
    press(&mut app, "h<CR>"); // custom…
    let Some(Overlay::Settings(dlg)) = app.overlay.as_ref() else {
        panic!()
    };
    assert!(dlg.input.is_some(), "custom input opens");
    press(&mut app, "fast<CR>");
    let msg = app.message.as_ref().unwrap().text.clone();
    assert!(msg.contains("invalid bandwidth"), "{msg}");
    press(&mut app, "<Esc><Esc>");
    app.settle().await;
    assert_eq!(app.settings.bwlimit, "off");
    assert!(fake.bwlimit().is_none());
}

#[tokio::test]
async fn custom_bwlimit_is_accepted() {
    let (_d, fake, mut app) = with_config().await;
    press(&mut app, ",");
    move_to_setting(&mut app, SettingId::Bwlimit);
    press(&mut app, "h<CR>750k<CR><CR>");
    app.settle().await;
    assert_eq!(app.settings.bwlimit, "750k");
    assert_eq!(fake.bwlimit().as_deref(), Some("750k"));
}

#[tokio::test]
async fn panel_key_is_session_only() {
    let (d, _f, mut app) = with_config().await;
    app.ui_state_path = Some(d.path().join("ui.json"));
    press(&mut app, "i");
    assert!(!app.panel_visible);
    assert!(!d.path().join("ui.json").exists());
    assert!(!d.path().join("config.toml").exists());
}

#[tokio::test]
async fn changing_one_setting_keeps_the_session_view() {
    let (_d, _f, mut app) = with_config().await;
    press(&mut app, "ss.i"); // sort by size, show dotfiles, hide the panel
    assert_eq!(app.pane.sort, SortKey::Size);
    press(&mut app, ",l<CR>"); // only the theme
    assert_eq!(app.theme.name, "tokyo-night-storm");
    assert_eq!(app.pane.sort, SortKey::Size, "sort kept");
    assert!(app.pane.show_hidden, "dotfiles kept");
    assert!(!app.panel_visible, "panel kept hidden");
}

// ---- pending items -------------------------------------------------------------

#[tokio::test]
async fn broken_theme_is_never_saved() {
    let (d, _f, mut app) = with_config().await;
    std::fs::create_dir_all(d.path().join("themes")).unwrap();
    std::fs::write(d.path().join("themes/zz-bad.toml"), "not a theme").unwrap();
    press(&mut app, ",h<CR>");
    assert_eq!(app.settings.theme, "tokyo-night");
    let text = std::fs::read_to_string(d.path().join("config.toml")).unwrap_or_default();
    assert!(!text.contains("zz-bad"), "{text}");
    let msg = app.message.as_ref().unwrap().text.clone();
    assert!(msg.contains("zz-bad"), "says why: {msg}");
}

#[tokio::test]
async fn navigating_before_remotes_cancels_the_start_jump() {
    let dir = tempfile::tempdir().unwrap();
    let fake = std::sync::Arc::new(ratclone::backend::fake::FakeBackend::demo());
    let store = ratclone::transfer::store::TransferStore::load(dir.path().join("t.json")).unwrap();
    let mut app = App::new(fake, store, &ratclone::config::Config::default(), home());
    app.pending_start = Some(named("personal", "Photos"));
    app.navigate(home().join("Downloads"), None);
    app.settle().await;
    assert_eq!(app.pane.loc, home().join("Downloads"));
}

#[tokio::test]
async fn failed_remote_list_drops_the_start_jump() {
    let dir = tempfile::tempdir().unwrap();
    let fake = std::sync::Arc::new(ratclone::backend::fake::FakeBackend::demo());
    fake.fail_next("remotes", ratclone::backend::BackendError::Rclone("boom".into()));
    let store = ratclone::transfer::store::TransferStore::load(dir.path().join("t.json")).unwrap();
    let mut app = App::new(fake, store, &ratclone::config::Config::default(), home());
    app.pending_start = Some(named("personal", "Photos"));
    app.settle().await;
    assert!(app.pending_start.is_none());
}

#[tokio::test]
async fn demo_mode_says_not_saved() {
    let (_d, _f, mut app) = demo_app().await;
    assert!(app.config_path.is_none());
    press(&mut app, ",l<CR>");
    let msg = app.message.as_ref().unwrap().text.clone();
    assert!(msg.contains("not saved"), "{msg}");
}

#[tokio::test]
async fn save_error_is_one_line() {
    let (d, _f, mut app) = with_config().await;
    std::fs::write(d.path().join("config.toml"), "theme = \n[[[broken\n").unwrap();
    press(&mut app, ",l<CR>");
    let msg = app.message.as_ref().unwrap().text.clone();
    assert!(msg.starts_with("Settings applied but not saved"), "{msg}");
    assert!(!msg.contains('\n'), "{msg:?}");
}

#[tokio::test]
async fn existing_rclone_limit_is_shown() {
    let (_d, fake, mut app) = with_config().await;
    ratclone::backend::Backend::set_bwlimit(fake.as_ref(), "5M")
        .await
        .unwrap();
    app.query_bwlimit();
    app.settle().await;
    assert_eq!(app.bwlimit_shown.as_deref(), Some("5M"));
}
