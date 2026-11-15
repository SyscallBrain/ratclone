mod common;

use std::time::Duration;

use common::app::*;
use ratclone::app::Focus;
use ratclone::app::pane::Listing;
use ratclone::backend::BackendError;
use ratclone::location::Remote;

#[tokio::test]
async fn starts_on_home_with_remotes_and_usage() {
    let (_d, _f, app) = demo_app().await;
    assert_eq!(app.pane.loc, home());
    assert_eq!(app.pane.listing, Listing::Ready);
    assert!(names(&app).contains(&"Downloads".to_string()));
    assert_eq!(app.sidebar.remotes.len(), 6);
    assert!(app.sidebar.about.contains_key(&Remote::Named("personal".into())));
}

#[tokio::test]
async fn opens_remote_and_navigates() {
    let (_d, _f, mut app) = demo_app().await;
    press(&mut app, "<Tab>");
    assert_eq!(app.focus, Focus::Sidebar);
    press(&mut app, "j");
    press(&mut app, "l");
    app.settle().await;
    assert_eq!(app.focus, Focus::Pane);
    assert_eq!(app.pane.loc.display(), "personal:/");
    move_to(&mut app, "Photos");
    press(&mut app, "l");
    app.settle().await;
    assert_eq!(app.pane.loc.path, "Photos");
    press(&mut app, "h");
    app.settle().await;
    assert_eq!(app.pane.loc.path, "");
    assert_eq!(app.pane.current().unwrap().name, "Photos");
}

#[tokio::test]
async fn stale_listing_is_ignored() {
    let (_d, fake, mut app) = demo_app().await;
    move_to(&mut app, "Downloads");
    fake.set_list_delay(Duration::from_millis(300));
    press(&mut app, "l");
    press(&mut app, "h");
    app.settle().await;
    assert_eq!(app.pane.loc, home());
    assert!(names(&app).contains(&"Downloads".to_string()));
    assert_eq!(app.pane.current().unwrap().name, "Downloads");
}

#[tokio::test]
async fn failed_listing_shows_error_not_spinner() {
    let (_d, fake, mut app) = demo_app().await;
    fake.fail_next("list", BackendError::Rclone("token expired".into()));
    move_to(&mut app, "Documents");
    press(&mut app, "l");
    app.settle().await;
    assert!(matches!(app.pane.listing, Listing::Failed(ref m) if m.contains("token expired")));
    press(&mut app, "R");
    app.settle().await;
    assert_eq!(app.pane.listing, Listing::Ready);
}

#[tokio::test]
async fn back_restores_previous_location() {
    let (_d, _f, mut app) = demo_app().await;
    move_to(&mut app, "Downloads");
    press(&mut app, "l");
    app.settle().await;
    press(&mut app, "-");
    app.settle().await;
    assert_eq!(app.pane.loc, home());
    assert_eq!(app.pane.current().unwrap().name, "Downloads");
}

#[tokio::test]
async fn parent_at_root_stays_in_pane() {
    let (_d, _f, mut app) = demo_app().await;
    press(&mut app, "<Tab>jl");
    app.settle().await;
    let root = app.pane.loc.clone();
    press(&mut app, "hh<Left><BS>");
    assert_eq!(app.focus, Focus::Pane);
    assert_eq!(app.pane.loc, root);
}

#[tokio::test]
async fn counts_and_half_pages() {
    let (_d, _f, mut app) = demo_app().await;
    press(&mut app, "<Tab>jl");
    app.settle().await;
    move_to(&mut app, "Photos");
    press(&mut app, "l");
    app.settle().await;
    move_to(&mut app, "2025");
    press(&mut app, "l");
    app.settle().await;
    press(&mut app, "5j");
    assert_eq!(app.pane.cursor, 5);
    app.pane.viewport_height = 10;
    press(&mut app, "<C-d>");
    assert_eq!(app.pane.cursor, 10);
    press(&mut app, "G");
    assert_eq!(app.pane.cursor, app.pane.len() - 1);
    press(&mut app, "gg");
    assert_eq!(app.pane.cursor, 0);
}

#[tokio::test]
async fn filter_typing_is_live_and_esc_clears() {
    let (_d, _f, mut app) = demo_app().await;
    move_to(&mut app, "Downloads");
    press(&mut app, "l");
    app.settle().await;
    press(&mut app, "/vid");
    assert_eq!(names(&app), ["video.mkv"]);
    press(&mut app, "<CR>");
    assert!(app.overlay.is_none());
    assert_eq!(names(&app), ["video.mkv"]);
    press(&mut app, "<Esc>");
    assert!(names(&app).len() > 1);
}

#[tokio::test]
async fn hidden_and_sort_keys() {
    let (_d, _f, mut app) = demo_app().await;
    assert!(!names(&app).contains(&".bashrc".to_string()));
    press(&mut app, ".");
    assert!(names(&app).contains(&".bashrc".to_string()));
    move_to(&mut app, "Downloads");
    press(&mut app, "l");
    app.settle().await;
    press(&mut app, "ss");
    let files: Vec<String> = app
        .pane
        .entries()
        .filter(|e| !e.is_dir)
        .map(|e| e.name.clone())
        .collect();
    assert_eq!(files[0], "video.mkv");
}

#[tokio::test]
async fn quit_without_transfers_sets_flags() {
    let (_d, _f, mut app) = demo_app().await;
    press(&mut app, "q");
    assert!(app.should_quit);
    assert!(app.shutdown_on_exit);
}

#[tokio::test]
async fn revisited_folder_shows_cached_rows_until_fresh_ones_arrive() {
    let (_d, fake, mut app) = demo_app().await;
    move_to(&mut app, "Downloads");
    press(&mut app, "l");
    app.settle().await;
    // Something changed at home behind our back; the slow listing is still on its way.
    fake.add_file(&home().join("fresh.txt"), 3);
    fake.set_list_delay(Duration::from_millis(300));
    press(&mut app, "h");
    assert_eq!(
        app.pane.listing,
        Listing::Ready,
        "cached rows at once, no spinner"
    );
    assert!(app.pane.refreshing);
    assert_eq!(app.pane.current().unwrap().name, "Downloads");
    assert!(!names(&app).contains(&"fresh.txt".to_string()));
    // Looking around works; changing anything waits for the fresh rows.
    press(&mut app, "j");
    let on = app.pane.current().unwrap().name.clone();
    press(&mut app, "dd");
    assert!(app.overlay.is_none(), "no delete on cached rows");
    app.settle().await;
    assert!(!app.pane.refreshing);
    assert!(names(&app).contains(&"fresh.txt".to_string()));
    assert_eq!(
        app.pane.current().unwrap().name,
        on,
        "cursor kept where the user moved it"
    );
}

#[tokio::test]
async fn a_change_in_a_folder_drops_its_cached_rows() {
    let (_d, fake, mut app) = demo_app().await;
    move_to(&mut app, "Downloads");
    press(&mut app, "l");
    app.settle().await;
    press(&mut app, "aNewDir<CR>");
    app.settle().await;
    assert!(names(&app).contains(&"NewDir".to_string()));
    press(&mut app, "h");
    app.settle().await;
    // Back into Downloads: its cached rows were replaced by the listing after mkdir.
    fake.set_list_delay(Duration::from_millis(300));
    move_to(&mut app, "Downloads");
    press(&mut app, "l");
    assert!(names(&app).contains(&"NewDir".to_string()));
    app.settle().await;
    // After a rename at home, its remembered rows are the ones listed after it.
    press(&mut app, "h");
    app.settle().await;
    fake.set_list_delay(Duration::from_millis(0));
    move_to(&mut app, "Downloads");
    press(&mut app, "r<C-e><C-u>Dl<CR>");
    app.settle().await;
    move_to(&mut app, "Dl");
    press(&mut app, "l");
    app.settle().await;
    fake.set_list_delay(Duration::from_millis(300));
    press(&mut app, "h");
    assert!(app.pane.refreshing);
    assert!(names(&app).contains(&"Dl".to_string()));
    assert!(!names(&app).contains(&"Downloads".to_string()));
    app.settle().await;
}

#[tokio::test]
async fn a_failed_listing_is_not_remembered() {
    let (_d, fake, mut app) = demo_app().await;
    move_to(&mut app, "Downloads");
    press(&mut app, "l");
    app.settle().await;
    press(&mut app, "h");
    app.settle().await;
    fake.fail_next("list", BackendError::Rclone("gone".into()));
    move_to(&mut app, "Downloads");
    press(&mut app, "l");
    app.settle().await;
    assert!(matches!(app.pane.listing, Listing::Failed(_)));
    press(&mut app, "h");
    app.settle().await;
    fake.set_list_delay(Duration::from_millis(300));
    move_to(&mut app, "Downloads");
    press(&mut app, "l");
    assert_eq!(app.pane.listing, Listing::Loading);
    app.settle().await;
}

#[tokio::test]
async fn tabs_cycle_and_jump() {
    use ratclone::app::Tab;
    let (_d, _f, mut app) = demo_app().await;
    assert_eq!(app.tab, Tab::Files);
    press(&mut app, "gt");
    assert_eq!(app.tab, Tab::Space);
    press(&mut app, "gt");
    assert_eq!(app.tab, Tab::Transfers);
    press(&mut app, "gt");
    assert_eq!(app.tab, Tab::Files);
    press(&mut app, "gT");
    assert_eq!(app.tab, Tab::Transfers);
    press(&mut app, "<Esc>");
    assert_eq!(app.tab, Tab::Files);
    press(&mut app, "T");
    assert_eq!(app.tab, Tab::Transfers);
    press(&mut app, "<Tab>");
    assert_eq!(app.focus, Focus::Sidebar);
    assert_eq!(app.tab, Tab::Transfers);
}

#[tokio::test]
async fn moving_over_an_account_asks_its_usage_once() {
    let (_d, f, mut app) = demo_app().await;
    let work = Remote::Named("work".into());
    app.sidebar.about.remove(&work);
    let before = f.calls().iter().filter(|c| *c == "about").count();
    press(&mut app, "<Tab>");
    // personal, work, personal, work, personal, work: work's usage is still on its way.
    press(&mut app, "jjkjkj");
    app.settle().await;
    let asked = f.calls().iter().filter(|c| *c == "about").count() - before;
    assert_eq!(asked, 1, "one request per account while one is pending");
    assert!(app.sidebar.about.contains_key(&work));
}

#[tokio::test]
async fn ctrl_n_and_ctrl_p_cycle_accounts_from_the_pane() {
    let (_d, _f, mut app) = demo_app().await;
    let remotes: Vec<Remote> = app.sidebar.remotes.iter().map(|r| r.remote.clone()).collect();
    assert_eq!(remotes[0], Remote::Local);
    press(&mut app, "<C-n>");
    app.settle().await;
    assert_eq!(app.focus, Focus::Pane);
    assert_eq!(app.pane.loc.remote, remotes[1]);
    assert_eq!(app.pane.loc.path, "");
    assert_eq!(app.sidebar.cursor, 1);
    press(&mut app, "<C-p>");
    app.settle().await;
    assert_eq!(app.pane.loc, home());
    // Wraps round both ways.
    press(&mut app, "<C-p>");
    app.settle().await;
    assert_eq!(app.pane.loc.remote, *remotes.last().unwrap());
    press(&mut app, "<C-n>");
    app.settle().await;
    assert_eq!(app.pane.loc, home());
    // Counts skip accounts; `-` goes back to where we were.
    press(&mut app, "2<C-n>");
    app.settle().await;
    assert_eq!(app.pane.loc.remote, remotes[2]);
    press(&mut app, "-");
    app.settle().await;
    assert_eq!(app.pane.loc, home());
}

#[tokio::test]
async fn ctrl_n_follows_the_pane_not_a_moved_sidebar_cursor() {
    let (_d, _f, mut app) = demo_app().await;
    let second = app.sidebar.remotes[1].remote.clone();
    // Cursor wandered in the sidebar without opening anything.
    press(&mut app, "<Tab>");
    press(&mut app, "3j");
    press(&mut app, "<Tab>");
    press(&mut app, "<C-n>");
    app.settle().await;
    assert_eq!(app.pane.loc.remote, second);
}

/// Browsing the sidebar without opening anything, then going back to the pane:
/// the info panel describes the pane's account again, while the sidebar cursor
/// stays where it was (Space opens on it).
#[tokio::test]
async fn panel_follows_the_pane_once_the_sidebar_is_left() {
    for leave in ["<Tab>", "<S-Tab>", "<Esc>", "T", "gT"] {
        let (_d, _f, mut app) = demo_app().await;
        press(&mut app, "<Tab>");
        assert_eq!(app.focus, Focus::Sidebar);
        press(&mut app, "3j");
        let browsed = app.sidebar.current().unwrap().remote.clone();
        assert_ne!(browsed, app.pane.loc.remote);
        assert_eq!(
            app.panel_account().unwrap().remote,
            browsed,
            "the sidebar previews"
        );
        press(&mut app, leave);
        app.settle().await;
        assert_eq!(app.focus, Focus::Pane, "after {leave}");
        assert_eq!(
            app.panel_account().unwrap().remote,
            app.pane.loc.remote,
            "after {leave}"
        );
        assert_eq!(app.sidebar.current().unwrap().remote, browsed, "after {leave}");
    }
}
