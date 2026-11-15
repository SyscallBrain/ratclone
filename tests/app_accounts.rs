mod common;

use common::app::*;
use ratclone::app::account::{Step, WizardMode};
use ratclone::app::overlay::Overlay;
use ratclone::app::{App, Focus, MessageKind};
use ratclone::backend::BackendError;
use ratclone::backend::fake::FakeBackend;
use ratclone::location::Remote;
use std::sync::Arc;

type Fake = Arc<FakeBackend>;

fn wizard(app: &App) -> &ratclone::app::account::AccountWizard {
    match app.overlay.as_ref() {
        Some(Overlay::Account(w)) => w,
        other => panic!("wizard not open: {other:?}"),
    }
}

fn step_name(app: &App) -> &'static str {
    match &wizard(app).step {
        Step::Loading => "loading",
        Step::Type { .. } => "type",
        Step::Form { .. } => "form",
        Step::Sending => "sending",
        Step::Question { .. } => "question",
        Step::WaitingBrowser { .. } => "waiting",
        Step::TokenPaste { .. } => "token",
        Step::DriveWarning { .. } => "drive-warning",
        Step::ConfirmCancel { .. } => "confirm-cancel",
        Step::Failed { .. } => "failed",
    }
}

async fn sidebar_app() -> (tempfile::TempDir, Fake, App) {
    let (d, f, mut app) = demo_app().await;
    app.focus = Focus::Sidebar;
    app.over_ssh = false;
    (d, f, app)
}

fn select_account(app: &mut App, name: &str) {
    let r = Remote::Named(name.into());
    app.sidebar.select(&r);
    assert_eq!(app.sidebar.current().unwrap().remote, r);
}

/// `a`, pick the type found by `query`, then replace the suggested name with `name`.
async fn start_add(app: &mut App, query: &str, name: &str) {
    press(app, "a");
    app.settle().await;
    assert_eq!(step_name(app), "type");
    press(app, query);
    press(app, "<CR>");
    assert_eq!(step_name(app), "form");
    press(app, "<C-u>");
    press(app, name);
}

fn message(app: &App) -> (String, MessageKind) {
    let m = app.message.as_ref().expect("a message");
    (m.text.clone(), m.kind)
}

#[tokio::test]
async fn add_mega() {
    let (_d, f, mut app) = sidebar_app().await;
    start_add(&mut app, "mega", "mega3").await;
    press(&mut app, "<Down>me@example.com<Down>hunter2<C-s>");
    app.settle().await;
    assert!(app.overlay.is_none(), "closed when done");
    let c = f.config_of("mega3").expect("created");
    assert_eq!(c["type"], "mega");
    assert_eq!(c["user"], "me@example.com");
    assert_ne!(c["pass"], "hunter2", "obscured");
    assert_eq!(
        app.sidebar.current().unwrap().remote,
        Remote::Named("mega3".into())
    );
    assert_eq!(
        message(&app),
        ("Account mega3 added".into(), MessageKind::Success)
    );
}

#[tokio::test]
async fn suggested_name_is_free() {
    let (_d, _f, mut app) = sidebar_app().await;
    press(&mut app, "a");
    app.settle().await;
    press(&mut app, "mega<CR>");
    assert_eq!(wizard(&app).name.value, "mega");
    press(&mut app, "<Esc>");
    assert_eq!(step_name(&app), "type", "Esc goes back one step");
    press(&mut app, "<Esc>");
    assert!(app.overlay.is_none());
}

#[tokio::test]
async fn add_name_clash_shows_error() {
    let (_d, f, mut app) = sidebar_app().await;
    start_add(&mut app, "mega", "media").await;
    assert_eq!(
        wizard(&app).name_error.as_deref(),
        Some("An account called media already exists")
    );
    press(&mut app, "<Down>u<Down>p<C-s>");
    app.settle().await;
    assert_eq!(step_name(&app), "form", "not sent");
    assert!(
        matches!(wizard(&app).step, Step::Form { cursor: 0, .. }),
        "cursor back on the name"
    );
    assert!(!f.sent_params().iter().any(|(m, _, _)| m == "config_create"));
}

#[tokio::test]
async fn required_field_blocks_save() {
    let (_d, f, mut app) = sidebar_app().await;
    start_add(&mut app, "mega", "m9").await;
    press(&mut app, "<Down>me<C-s>");
    app.settle().await;
    assert_eq!(step_name(&app), "form");
    assert_eq!(wizard(&app).form_error.as_deref(), Some("pass is required"));
    assert!(
        matches!(wizard(&app).step, Step::Form { cursor: 2, .. }),
        "cursor on pass"
    );
    assert!(f.config_of("m9").is_none());
}

#[tokio::test]
async fn edit_sends_only_changed_fields() {
    let (_d, f, mut app) = sidebar_app().await;
    f.set_config_value("media", "user", "old@example.com");
    f.set_config_value("media", "pass", "obscured-old");
    select_account(&mut app, "media");
    press(&mut app, "e");
    app.settle().await;
    assert_eq!(step_name(&app), "form");
    assert!(matches!(&wizard(&app).mode, WizardMode::Edit { name, .. } if name == "media"));
    // No name row when editing: the cursor starts on `user`.
    press(&mut app, "<C-u>new@example.com<C-s>");
    app.settle().await;
    assert!(app.overlay.is_none());
    let sent = f.sent_params();
    let update = sent.iter().find(|(m, _, _)| m == "config_update").unwrap();
    assert_eq!(update.1, vec!["user".to_string()]);
    assert_eq!(f.config_of("media").unwrap()["pass"], "obscured-old");
    assert_eq!(message(&app).0, "media updated");
}

#[tokio::test]
async fn edit_local_is_refused() {
    let (_d, _f, mut app) = sidebar_app().await;
    app.sidebar.select(&Remote::Local);
    press(&mut app, "e");
    assert!(app.overlay.is_none());
    assert_eq!(message(&app).0, "The local disk is not an rclone account");
}

#[tokio::test]
async fn edit_warns_when_transfers_running() {
    use ratclone::transfer::spec::{TransferItem, TransferKind, TransferSpec};
    let (_d, _f, mut app) = sidebar_app().await;
    let spec = TransferSpec {
        id: 1,
        kind: TransferKind::Copy,
        src_dir: home(),
        dst_dir: named("media", ""),
        items: vec![TransferItem {
            replace: false,
            name: "a".into(),
            dst_name: "a".into(),
            is_dir: false,
            size: None,
        }],
    };
    app.store.push(spec, vec![1], jiff::Timestamp::now());
    select_account(&mut app, "media");
    press(&mut app, "e");
    app.settle().await;
    assert_eq!(
        wizard(&app).transfers_warning.as_deref(),
        Some("1 transfer is using media; changing it may make them fail.")
    );
}

#[tokio::test]
async fn remove_sees_accounts_built_on_others() {
    use ratclone::transfer::spec::{TransferItem, TransferKind, TransferSpec};
    let (_d, f, mut app) = sidebar_app().await;
    f.add_remote("middle", "alias");
    f.set_config_value("middle", "remote", "archive:");
    f.add_remote("outer", "crypt");
    f.set_config_value("outer", "remote", "middle:secret");
    app.handle_event(ratclone::app::AppEvent::Remotes(Ok(
        ratclone::backend::Backend::remotes(f.as_ref()).await.unwrap(),
    )));
    select_account(&mut app, "archive");
    press(&mut app, "dd");
    app.settle().await;
    match app.overlay.as_ref() {
        Some(Overlay::Confirm { lines, .. }) => {
            let text = lines.join("\n");
            assert!(text.contains("middle") && text.contains("outer"), "{text}");
        }
        other => panic!("{other:?}"),
    }
    press(&mut app, "<Esc>");
    // A transfer through outer → middle → archive keeps archive.
    let spec = TransferSpec {
        id: 7,
        kind: TransferKind::Copy,
        src_dir: home(),
        dst_dir: named("outer", ""),
        items: vec![TransferItem {
            replace: false,
            name: "a".into(),
            dst_name: "a".into(),
            is_dir: false,
            size: None,
        }],
    };
    app.store.push(spec, vec![7], jiff::Timestamp::now());
    app.focus = Focus::Sidebar;
    select_account(&mut app, "archive");
    press(&mut app, "dd");
    app.settle().await;
    assert!(app.overlay.is_none(), "asked to remove an account in use");
    assert_eq!(message(&app).0, "archive is in use by running transfers");
    assert!(f.config_of("archive").is_some());
}

#[tokio::test]
async fn question_error_stays_on_step() {
    let (_d, f, mut app) = sidebar_app().await;
    app.over_ssh = true;
    start_add(&mut app, "dropbox", "d1").await;
    press(&mut app, "<C-s>");
    app.settle().await;
    assert_eq!(step_name(&app), "token");
    press(&mut app, "notjson<CR>");
    app.settle().await;
    match &wizard(&app).step {
        Step::TokenPaste { error, input, .. } => {
            assert!(error.as_deref().unwrap().starts_with("Couldn't decode response"));
            assert_eq!(input.value, "notjson", "the answer is kept to fix it");
        }
        _ => panic!("expected to stay on the token step, got {}", step_name(&app)),
    }
    assert!(f.config_of("d1").is_some());
}

#[tokio::test]
async fn a_pasted_token_is_text_even_where_keys_are_commands() {
    let (_d, _f, mut app) = sidebar_app().await;
    app.over_ssh = true;
    start_add(&mut app, "dropbox", "d9").await;
    press(&mut app, "<C-s>");
    app.settle().await;
    assert_eq!(step_name(&app), "token");
    // Typed on the empty field, `y` copies the command and `L` switches to the browser.
    app.handle_paste("yL{\"access_token\":\"x\"}\n");
    match &wizard(&app).step {
        Step::TokenPaste { input, .. } => assert_eq!(input.value, "yL{\"access_token\":\"x\"}"),
        _ => panic!("expected to stay on the token step, got {}", step_name(&app)),
    }
    assert!(app.clipboard_out.is_none(), "nothing was copied");
}

#[tokio::test]
async fn empty_token_is_not_sent() {
    let (_d, f, mut app) = sidebar_app().await;
    app.over_ssh = true;
    start_add(&mut app, "dropbox", "d7").await;
    press(&mut app, "<C-s>");
    app.settle().await;
    let before = f.sent_params().len();
    press(&mut app, "<CR>");
    app.settle().await;
    assert_eq!(step_name(&app), "token");
    assert_eq!(f.sent_params().len(), before, "nothing sent");
    match &wizard(&app).step {
        Step::TokenPaste { error, .. } => assert_eq!(error.as_deref(), Some("Paste the token first")),
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn cancel_add_mid_questions_deletes_account() {
    let (_d, f, mut app) = sidebar_app().await;
    app.over_ssh = true;
    start_add(&mut app, "dropbox", "d2").await;
    press(&mut app, "<C-s>");
    app.settle().await;
    assert_eq!(step_name(&app), "token");
    press(&mut app, "<Esc>");
    assert_eq!(step_name(&app), "confirm-cancel");
    press(&mut app, "n");
    assert_eq!(step_name(&app), "token");
    press(&mut app, "<Esc>y");
    app.settle().await;
    assert!(app.overlay.is_none());
    assert!(f.config_of("d2").is_none(), "half-made account removed");
}

#[tokio::test]
async fn cancel_edit_keeps_written_values() {
    let (_d, f, mut app) = sidebar_app().await;
    app.over_ssh = true;
    select_account(&mut app, "shared");
    press(&mut app, "e");
    app.settle().await;
    // client_id changed: rclone asks for a new token, which goes to the paste step.
    press(&mut app, "abc<C-s>");
    app.settle().await;
    assert_eq!(step_name(&app), "token");
    press(&mut app, "<Esc>y");
    app.settle().await;
    assert!(app.overlay.is_none());
    let c = f.config_of("shared").unwrap();
    assert_eq!(c["client_id"], "abc");
}

#[tokio::test]
async fn disconnect_mid_wizard_keeps_overlay() {
    let (_d, f, mut app) = sidebar_app().await;
    start_add(&mut app, "mega", "m5").await;
    press(&mut app, "<Down>u<Down>p");
    f.set_disconnected(true);
    press(&mut app, "<C-s>");
    app.settle().await;
    assert_eq!(step_name(&app), "failed");
    f.set_disconnected(false);
    press(&mut app, "<CR>");
    assert_eq!(step_name(&app), "form", "back to the form to try again");
}

#[tokio::test]
async fn providers_error_shows_message() {
    let (_d, f, mut app) = sidebar_app().await;
    f.fail_next("providers", BackendError::Rclone("boom".into()));
    press(&mut app, "a");
    app.settle().await;
    assert!(app.overlay.is_none());
    assert_eq!(
        message(&app),
        ("Could not load account types: boom".into(), MessageKind::Error)
    );
}

#[tokio::test]
async fn a_twice_opens_one_wizard() {
    let (_d, f, mut app) = sidebar_app().await;
    press(&mut app, "a");
    press(&mut app, "a");
    app.settle().await;
    assert_eq!(f.calls().iter().filter(|c| *c == "providers").count(), 1);
    assert_eq!(step_name(&app), "type");
}

#[tokio::test]
async fn d_alone_is_pending_in_sidebar() {
    let (_d, _f, mut app) = sidebar_app().await;
    press(&mut app, "d");
    assert_eq!(app.keys.pending_keys(), "d");
    assert!(app.overlay.is_none());
}

// ---- OAuth ----

use ratclone::app::AppEvent;

async fn tick(app: &mut App) {
    app.handle_event(AppEvent::Tick);
    app.settle().await;
}

#[test]
fn osc52_encodes() {
    assert_eq!(ratclone::clipboard::osc52("hi"), "\x1b]52;c;aGk=\x07");
    assert_eq!(ratclone::clipboard::osc52("abc"), "\x1b]52;c;YWJj\x07");
    assert_eq!(ratclone::clipboard::osc52("ab"), "\x1b]52;c;YWI=\x07");
}

#[tokio::test]
async fn dropbox_local_waits_then_done() {
    let (_d, f, mut app) = sidebar_app().await;
    start_add(&mut app, "dropbox", "d3").await;
    press(&mut app, "<C-s>");
    app.settle().await;
    assert_eq!(step_name(&app), "waiting");
    assert_eq!(
        wizard(&app).auto_note.as_deref(),
        Some("Using this machine's browser")
    );
    tick(&mut app).await;
    assert!(app.overlay.is_none(), "done after the login");
    assert!(f.config_of("d3").unwrap().contains_key("token"));
    assert_eq!(message(&app).0, "Account d3 added");
}

#[tokio::test]
async fn esc_during_browser_wait_stops_and_deletes() {
    let (_d, f, mut app) = sidebar_app().await;
    f.set_auth_delay(std::time::Duration::from_secs(60));
    start_add(&mut app, "dropbox", "d4").await;
    press(&mut app, "<C-s>");
    app.settle().await;
    tick(&mut app).await;
    assert_eq!(step_name(&app), "waiting");
    press(&mut app, "<Esc>y");
    app.settle().await;
    assert!(app.overlay.is_none());
    assert!(f.calls().contains(&"stop_job".to_string()));
    assert!(f.config_of("d4").is_none());
}

#[tokio::test]
async fn l_flips_browser_guess() {
    let (_d, f, mut app) = sidebar_app().await;
    f.set_auth_delay(std::time::Duration::from_secs(60));
    start_add(&mut app, "dropbox", "d6").await;
    press(&mut app, "<C-s>");
    app.settle().await;
    assert_eq!(step_name(&app), "waiting");
    press(&mut app, "L");
    app.settle().await;
    assert_eq!(step_name(&app), "token", "switched to pasting a token");
    assert_eq!(
        wizard(&app).auto_note.as_deref(),
        Some("Paste a token from another machine")
    );
    assert!(f.calls().contains(&"stop_job".to_string()));
}

#[tokio::test]
async fn ssh_dropbox_paste_token() {
    let (_d, f, mut app) = sidebar_app().await;
    app.over_ssh = true;
    start_add(&mut app, "dropbox", "d5").await;
    press(&mut app, "<C-s>");
    app.settle().await;
    match &wizard(&app).step {
        Step::TokenPaste { command, .. } => assert_eq!(command, "rclone authorize \"dropbox\""),
        _ => panic!("expected the token step"),
    }
    press(&mut app, "y");
    assert_eq!(app.clipboard_out.as_deref(), Some("rclone authorize \"dropbox\""));
    assert_eq!(message(&app).0, "Command copied");
    press(&mut app, "{\"x\":1}<CR>");
    app.settle().await;
    assert!(app.overlay.is_none());
    assert!(f.config_of("d5").unwrap().contains_key("token"));
}

#[tokio::test]
async fn drive_warning_no_goes_back_to_client_id() {
    let (_d, f, mut app) = sidebar_app().await;
    start_add(&mut app, "drive", "g9").await;
    press(&mut app, "<C-s>");
    app.settle().await;
    assert_eq!(step_name(&app), "drive-warning");
    press(&mut app, "n");
    app.settle().await;
    assert_eq!(step_name(&app), "form");
    let w = wizard(&app);
    let rows = w.rows(false);
    let Step::Form { cursor, .. } = w.step else {
        unreachable!()
    };
    let ratclone::app::account::Row::Field(i) = rows[cursor] else {
        panic!("cursor not on a field")
    };
    assert_eq!(w.fields[i].option.name, "client_id");
    assert!(f.config_of("g9").is_none(), "the account just made is removed");
    // Filling a client id and sending again skips the warning.
    press(&mut app, "myid<C-s>");
    app.settle().await;
    assert_eq!(step_name(&app), "waiting");
}

#[tokio::test]
async fn drive_warning_yes_continues() {
    let (_d, _f, mut app) = sidebar_app().await;
    start_add(&mut app, "drive", "g8").await;
    press(&mut app, "<C-s>");
    app.settle().await;
    press(&mut app, "y");
    app.settle().await;
    assert_eq!(step_name(&app), "waiting");
    tick(&mut app).await;
    assert_eq!(step_name(&app), "question", "team drive question");
    press(&mut app, "<CR>");
    app.settle().await;
    assert!(app.overlay.is_none());
}

#[tokio::test]
async fn edit_client_id_triggers_reauth() {
    let (_d, _f, mut app) = sidebar_app().await;
    select_account(&mut app, "shared");
    press(&mut app, "e");
    app.settle().await;
    press(&mut app, "abc<C-s>");
    app.settle().await;
    assert_eq!(step_name(&app), "waiting");
}

#[tokio::test]
async fn edit_other_field_skips_reauth() {
    let (_d, f, mut app) = sidebar_app().await;
    select_account(&mut app, "shared");
    press(&mut app, "e");
    app.settle().await;
    press(&mut app, "<Down>sec<C-s>");
    app.settle().await;
    assert_eq!(
        step_name(&app),
        "waiting",
        "client_secret is part of the client too"
    );
    press(&mut app, "<Esc>y");
    app.settle().await;
    assert!(app.overlay.is_none());
    f.set_config_value("archive", "user", "a");
    f.set_config_value("archive", "pass", "obscured-x");
    select_account(&mut app, "archive");
    press(&mut app, "e");
    app.settle().await;
    press(&mut app, "<C-u>b<C-s>");
    app.settle().await;
    assert!(
        app.overlay.is_none(),
        "no browser for a plain field: {:?}",
        app.overlay
    );
    assert_eq!(f.config_of("archive").unwrap()["user"], "b");
}

#[tokio::test]
async fn reauth_dropbox() {
    let (_d, f, mut app) = sidebar_app().await;
    select_account(&mut app, "shared");
    press(&mut app, "A");
    app.settle().await;
    assert_eq!(step_name(&app), "waiting");
    tick(&mut app).await;
    assert!(app.overlay.is_none());
    assert_eq!(message(&app).0, "shared re-authorised");
    assert!(
        f.sent_params()
            .iter()
            .any(|(m, k, _)| m == "config_update" && k == &vec!["config_refresh_token".to_string()])
    );
}

#[tokio::test]
async fn reauth_alias_refused() {
    let (_d, f, mut app) = sidebar_app().await;
    f.add_remote("al", "alias");
    app.handle_event(AppEvent::Remotes(Ok(ratclone::backend::Backend::remotes(
        f.as_ref(),
    )
    .await
    .unwrap())));
    select_account(&mut app, "al");
    press(&mut app, "A");
    app.settle().await;
    assert!(app.overlay.is_none());
    assert_eq!(message(&app).0, "al doesn't use a browser login");
}

// ---- remove ----

use ratclone::app::overlay::PendingOp;

fn running_copy_to(app: &mut App, remote: &str) {
    use ratclone::transfer::spec::{TransferItem, TransferKind, TransferSpec};
    let spec = TransferSpec {
        id: 7,
        kind: TransferKind::Copy,
        src_dir: home(),
        dst_dir: named(remote, ""),
        items: vec![TransferItem {
            replace: false,
            name: "a".into(),
            dst_name: "a".into(),
            is_dir: false,
            size: None,
        }],
    };
    app.store.push(spec, vec![7], jiff::Timestamp::now());
}

fn confirm_lines(app: &App) -> Vec<String> {
    match &app.overlay {
        Some(Overlay::Confirm {
            lines,
            on_yes: PendingOp::RemoveAccount { .. },
            ..
        }) => lines.clone(),
        other => panic!("remove confirm not open: {other:?}"),
    }
}

#[tokio::test]
async fn remove_asks_and_deletes() {
    let (_d, f, mut app) = sidebar_app().await;
    select_account(&mut app, "archive");
    press(&mut app, "dd");
    app.settle().await;
    let lines = confirm_lines(&app);
    assert_eq!(lines[0], "Remove account archive (Mega)?");
    assert!(lines[1].contains("files in the account are untouched"));
    match &app.overlay {
        Some(Overlay::Confirm { commands, danger, .. }) => {
            assert_eq!(commands, &vec!["rclone config delete archive".to_string()]);
            assert!(*danger);
        }
        _ => unreachable!(),
    }
    press(&mut app, "y");
    app.settle().await;
    assert!(f.config_of("archive").is_none());
    assert!(
        !app.sidebar
            .remotes
            .iter()
            .any(|r| r.remote == Remote::Named("archive".into()))
    );
    assert_eq!(
        message(&app),
        ("Account archive removed".into(), MessageKind::Success)
    );
    assert!(app.sidebar.cursor < app.sidebar.remotes.len());
}

#[tokio::test]
async fn remove_lists_dependents_in_confirm() {
    let (_d, f, mut app) = sidebar_app().await;
    f.add_remote("vault", "crypt");
    f.set_config_value("vault", "remote", "media:Secret");
    select_account(&mut app, "media");
    press(&mut app, "D");
    app.settle().await;
    let lines = confirm_lines(&app);
    assert_eq!(
        lines.last().unwrap(),
        "Also used by: vault — they will stop working."
    );
}

#[tokio::test]
async fn remove_blocked_by_running_transfer() {
    let (_d, f, mut app) = sidebar_app().await;
    running_copy_to(&mut app, "archive");
    select_account(&mut app, "archive");
    press(&mut app, "dd");
    app.settle().await;
    assert!(app.overlay.is_none());
    assert_eq!(message(&app).0, "archive is in use by running transfers");
    assert!(f.config_of("archive").is_some());
}

#[tokio::test]
async fn remove_blocked_by_dependent_transfer() {
    let (_d, f, mut app) = sidebar_app().await;
    f.add_remote("vault", "crypt");
    f.set_config_value("vault", "remote", "media:Secret");
    running_copy_to(&mut app, "vault");
    select_account(&mut app, "media");
    press(&mut app, "dd");
    app.settle().await;
    assert!(app.overlay.is_none());
    assert_eq!(message(&app).0, "media is in use by running transfers");
}

#[tokio::test]
async fn remove_local_refused() {
    let (_d, _f, mut app) = sidebar_app().await;
    app.sidebar.select(&Remote::Local);
    press(&mut app, "dd");
    assert!(app.overlay.is_none());
    assert_eq!(message(&app).0, "The local disk is not an rclone account");
}

#[tokio::test]
async fn remove_current_account_moves_pane_and_cancels_scan() {
    let (_d, f, mut app) = sidebar_app().await;
    app.navigate(named("archive", "Movies"), None);
    app.settle().await;
    f.set_list_delay(std::time::Duration::from_millis(300));
    app.start_scan(named("archive", ""));
    assert!(app.scan.is_some());
    select_account(&mut app, "archive");
    press(&mut app, "dd");
    app.settle().await;
    press(&mut app, "y");
    app.settle().await;
    app.settle().await;
    assert_eq!(app.pane.loc, app.home);
    assert!(app.scan.is_none(), "scan cancelled");
    assert!(
        !app.scans
            .keys()
            .any(|k| k.remote == Remote::Named("archive".into())),
        "no scan kept"
    );
}

// ---- final review fixes ----

#[tokio::test]
async fn cancel_during_browser_then_re_add_keeps_account() {
    let (_d, f, mut app) = sidebar_app().await;
    f.set_auth_delay(std::time::Duration::from_secs(60));
    start_add(&mut app, "dropbox", "d4").await;
    press(&mut app, "<C-s>");
    app.settle().await;
    assert_eq!(step_name(&app), "waiting");
    press(&mut app, "<Esc>y");
    app.settle().await;
    // Add the same name again, this time to the end.
    app.over_ssh = true;
    start_add(&mut app, "dropbox", "d4").await;
    press(&mut app, "<C-s>");
    app.settle().await;
    press(&mut app, "{\"t\":1}<CR>");
    app.settle().await;
    assert!(app.overlay.is_none());
    assert!(f.config_of("d4").is_some());
    // Another wizard cancelled while its request is in flight must not touch d4.
    select_account(&mut app, "shared");
    press(&mut app, "e");
    app.settle().await;
    press(&mut app, "x<C-s><Esc>");
    app.settle().await;
    assert!(f.config_of("d4").is_some(), "a good account was deleted");
}

#[tokio::test]
async fn answer_during_cancel_confirm_is_kept() {
    let (_d, f, mut app) = sidebar_app().await;
    start_add(&mut app, "mega", "m7").await;
    press(&mut app, "<Down>u<Down>p<C-s><Esc>");
    assert_eq!(step_name(&app), "confirm-cancel");
    app.settle().await;
    press(&mut app, "n");
    assert!(
        app.overlay.is_none() || step_name(&app) != "sending",
        "stuck on the spinner"
    );
    assert!(f.config_of("m7").is_some());
}

#[tokio::test]
async fn job_started_during_cancel_confirm_keeps_waiting() {
    let (_d, f, mut app) = sidebar_app().await;
    start_add(&mut app, "dropbox", "d8").await;
    press(&mut app, "<C-s><Esc>");
    app.settle().await;
    assert_eq!(step_name(&app), "confirm-cancel");
    press(&mut app, "n");
    assert!(matches!(
        wizard(&app).step,
        Step::WaitingBrowser { jobid: Some(_), .. }
    ));
    tick(&mut app).await;
    assert!(app.overlay.is_none(), "the login finished");
    assert!(f.config_of("d8").unwrap().contains_key("token"));
}

#[tokio::test]
async fn add_refuses_name_made_outside_app() {
    let (_d, f, mut app) = sidebar_app().await;
    f.add_remote("drive", "sftp");
    f.set_config_value("drive", "host", "keep.me");
    press(&mut app, "a");
    app.settle().await;
    press(&mut app, "drive<CR>");
    assert_eq!(wizard(&app).name.value, "drive2", "names asked afresh");
    // Made outside the app while the form is open: checked again before creating.
    f.add_remote("drive2", "sftp");
    press(&mut app, "<C-s>");
    app.settle().await;
    assert_eq!(step_name(&app), "form");
    assert_eq!(
        wizard(&app).name_error.as_deref(),
        Some("An account called drive2 already exists")
    );
    assert_eq!(f.config_of("drive2").unwrap()["type"], "sftp", "untouched");
    assert_eq!(f.config_of("drive").unwrap()["host"], "keep.me");
}

#[tokio::test]
async fn add_sends_nothing_when_the_name_check_fails() {
    let (_d, f, mut app) = sidebar_app().await;
    press(&mut app, "a");
    app.settle().await;
    press(&mut app, "mega<CR><C-u>new-account<Down>new-user<Down>new-pass");
    // Made outside the app while the form is open, and the check before saving fails.
    f.add_remote("new-account", "sftp");
    f.set_config_value("new-account", "host", "precious-existing-host");
    f.fail_next("remotes", BackendError::Other("temporary read failure".into()));
    press(&mut app, "<C-s>");
    app.settle().await;
    let config = f.config_of("new-account").unwrap();
    assert_eq!(config["type"], "sftp", "an existing account was overwritten");
    assert_eq!(config["host"], "precious-existing-host");
    match &wizard(&app).step {
        Step::Failed { message, .. } => assert!(message.contains("temporary read failure"), "{message}"),
        _ => panic!("{}", step_name(&app)),
    }
    // Back to the form, with what was typed.
    press(&mut app, "<CR>");
    assert_eq!(step_name(&app), "form");
    assert_eq!(wizard(&app).name.value, "new-account");
    // Leaving now removes nothing: the app made nothing.
    press(&mut app, "<Esc>y");
    app.settle().await;
    assert_eq!(f.config_of("new-account").unwrap()["type"], "sftp");
}

#[tokio::test]
async fn enter_saves_form_with_advanced_options() {
    let (_d, f, mut app) = sidebar_app().await;
    start_add(&mut app, "mega", "m8").await;
    press(&mut app, "<CR>u<CR>p<CR><CR>");
    app.settle().await;
    assert!(f.config_of("m8").is_some(), "Enter on the last field saves");
}

#[tokio::test]
async fn late_edit_config_ignored() {
    let (_d, f, mut app) = sidebar_app().await;
    f.set_config_value("media", "user", "from-media");
    f.set_config_value("archive", "user", "from-archive");
    select_account(&mut app, "media");
    press(&mut app, "e<Esc>");
    select_account(&mut app, "archive");
    press(&mut app, "e");
    app.settle().await;
    let w = wizard(&app);
    assert!(matches!(&w.mode, WizardMode::Edit { name, .. } if name == "archive"));
    assert_eq!(w.fields[0].input.value, "from-archive");
}

#[tokio::test]
async fn remove_check_does_not_replace_open_wizard() {
    let (_d, _f, mut app) = sidebar_app().await;
    select_account(&mut app, "archive");
    press(&mut app, "dd");
    app.focus = Focus::Sidebar;
    press(&mut app, "a");
    app.settle().await;
    assert!(
        matches!(app.overlay, Some(Overlay::Account(_))),
        "{:?}",
        app.overlay.is_some()
    );
}

#[tokio::test]
async fn pasted_token_not_in_debug() {
    let (_d, _f, mut app) = sidebar_app().await;
    app.over_ssh = true;
    start_add(&mut app, "dropbox", "d9").await;
    press(&mut app, "<C-s>");
    app.settle().await;
    press(&mut app, "{\"refresh_token\":\"S3CRET\"}");
    let dbg = format!("{:?}", app.overlay);
    assert!(!dbg.contains("S3CRET"), "{dbg}");
}

#[tokio::test]
async fn remove_command_quotes_name_with_space() {
    let (_d, f, mut app) = sidebar_app().await;
    f.add_remote("my drive", "alias");
    f.add_remote("vault", "crypt");
    f.set_config_value("vault", "remote", "my drive:Secret");
    app.handle_event(AppEvent::Remotes(Ok(ratclone::backend::Backend::remotes(
        f.as_ref(),
    )
    .await
    .unwrap())));
    select_account(&mut app, "my drive");
    press(&mut app, "dd");
    app.settle().await;
    match &app.overlay {
        Some(Overlay::Confirm { commands, lines, .. }) => {
            assert_eq!(commands, &vec!["rclone config delete \"my drive\"".to_string()]);
            assert_eq!(
                lines.last().unwrap(),
                "Also used by: vault — they will stop working."
            );
        }
        other => panic!("{other:?}"),
    }
}

// ---- deferred items ----

#[tokio::test]
async fn disconnect_during_add_retries_cleanup_after_reconnect() {
    let (_d, f, mut app) = sidebar_app().await;
    app.over_ssh = true;
    start_add(&mut app, "dropbox", "d10").await;
    press(&mut app, "<C-s>");
    app.settle().await;
    assert!(f.config_of("d10").is_some(), "created");
    f.set_disconnected(true);
    press(&mut app, "{\"t\":1}<CR>");
    app.settle().await;
    assert_eq!(step_name(&app), "failed");
    assert!(
        !message(&app).0.starts_with("Could not remove"),
        "{:?}",
        message(&app)
    );
    f.set_disconnected(false);
    app.handle_event(AppEvent::Reconnected(Ok(false)));
    app.settle().await;
    assert!(f.config_of("d10").is_none(), "cleaned up after reconnecting");
}

#[tokio::test]
async fn same_type_again_keeps_typed_values() {
    let (_d, _f, mut app) = sidebar_app().await;
    start_add(&mut app, "mega", "m11").await;
    press(&mut app, "<Down>typed<Esc>");
    assert_eq!(step_name(&app), "type");
    press(&mut app, "<CR>");
    let w = wizard(&app);
    assert_eq!(w.fields[0].input.value, "typed");
    assert_eq!(w.name.value, "m11");
}

#[tokio::test]
async fn help_scrolls() {
    let (_d, _f, mut app) = sidebar_app().await;
    start_add(&mut app, "mega", "m12").await;
    press(&mut app, "<F1><Down><Down>");
    assert_eq!(wizard(&app).help_scroll, 2);
    press(&mut app, "<Up>");
    assert_eq!(wizard(&app).help_scroll, 1);
    press(&mut app, "<Esc>");
    assert!(matches!(wizard(&app).step, Step::Form { help_open: false, .. }));
    assert_eq!(wizard(&app).help_scroll, 0);
}

#[tokio::test]
async fn failed_refresh_clears_pending_select() {
    let (_d, _f, mut app) = sidebar_app().await;
    app.pending_select = Some(Remote::Named("media".into()));
    app.handle_event(AppEvent::Remotes(Err(BackendError::Rclone("boom".into()))));
    assert!(app.pending_select.is_none());
}

#[tokio::test]
async fn quit_when_idle_waits_for_open_wizard() {
    let (_d, _f, mut app) = sidebar_app().await;
    app.quit_when_idle = true;
    press(&mut app, "a");
    app.settle().await;
    app.handle_event(AppEvent::OpDone {
        what: "x".into(),
        result: Ok(()),
        refresh: vec![],
        cursor: None,
    });
    assert!(!app.should_quit, "quit with the wizard open");
    press(&mut app, "<Esc>");
    assert!(app.overlay.is_none());
    assert!(app.should_quit, "quits once the wizard is closed");
}

#[tokio::test]
async fn name_still_being_cleaned_up_is_refused() {
    let (_d, f, mut app) = sidebar_app().await;
    start_add(&mut app, "mega", "m13").await;
    // Cancelled while the create is in flight: its clean-up waits for the answer.
    press(&mut app, "<Down>u<Down>p<C-s><Esc>y");
    assert!(app.overlay.is_none());
    press(&mut app, "a");
    assert_eq!(
        wizard(&app).busy,
        vec!["m13".to_string()],
        "known as busy at once"
    );
    app.settle().await;
    // Once the clean-up is done the name is free again, even if the list of names
    // was read before it finished.
    assert!(f.config_of("m13").is_none());
    press(&mut app, "mega<CR><C-u>m13");
    assert_eq!(wizard(&app).name_error, None);
}

#[test]
fn busy_name_message() {
    use ratclone::app::account::form::validate_name_busy;
    assert_eq!(
        validate_name_busy("m13", &[], &["m13".to_string()]).unwrap_err(),
        "m13 is still being removed; try again in a moment"
    );
}

#[tokio::test]
async fn a_login_cancelled_mid_poll_does_not_hold_back_the_next() {
    let (_d, f, mut app) = sidebar_app().await;
    f.set_auth_delay(std::time::Duration::from_secs(60));
    start_add(&mut app, "dropbox", "d1").await;
    press(&mut app, "<C-s>");
    app.settle().await;
    assert_eq!(step_name(&app), "waiting");
    // The tick asks about login A; A is cancelled before the answer is handled.
    app.handle_event(AppEvent::Tick);
    press(&mut app, "<Esc>y");
    app.settle().await;
    assert!(app.overlay.is_none());
    // Login B finishes at once in rclone.
    f.set_auth_delay(std::time::Duration::ZERO);
    start_add(&mut app, "dropbox", "d2").await;
    press(&mut app, "<C-s>");
    app.settle().await;
    tick(&mut app).await;
    assert!(app.overlay.is_none(), "login B was never polled");
    assert!(f.config_of("d2").unwrap().contains_key("token"));
}

#[tokio::test]
async fn ssh_login_with_own_client_keeps_rclones_command() {
    let (_d, f, mut app) = sidebar_app().await;
    app.over_ssh = true;
    start_add(&mut app, "dropbox", "d6").await;
    press(&mut app, "<Down>abc<Down>def<C-s>");
    app.settle().await;
    let want = "rclone authorize \"dropbox\" \"enc-abc\"";
    match &wizard(&app).step {
        Step::TokenPaste { command, .. } => assert_eq!(command, want),
        other => panic!("expected the token step, got {other:?}"),
    }
    press(&mut app, "y");
    assert_eq!(app.clipboard_out.as_deref(), Some(want));
    press(&mut app, "{\"x\":1}<CR>");
    app.settle().await;
    assert!(app.overlay.is_none());
    assert_eq!(f.config_of("d6").unwrap()["client_id"], "abc");
}

// ---- two panes ----

#[tokio::test]
async fn removing_an_account_moves_the_unfocused_pane_too() {
    let (_d, _f, mut app) = demo_app().await;
    app.navigate(named("archive", ""), None);
    app.settle().await;
    press(&mut app, "|");
    app.settle().await;
    assert_ne!(app.pane.loc.remote, Remote::Named("archive".into()));
    app.focus = Focus::Sidebar;
    select_account(&mut app, "archive");
    press(&mut app, "dd");
    app.settle().await;
    press(&mut app, "y");
    app.settle().await;
    app.settle().await;
    assert_eq!(app.left_pane().loc, app.home);
}

#[tokio::test]
async fn removing_an_account_forgets_where_the_right_pane_was() {
    let (_d, _f, mut app) = demo_app().await;
    press(&mut app, "|");
    app.settle().await;
    app.navigate(named("archive", "Movies"), None);
    app.settle().await;
    press(&mut app, "|");
    app.focus = Focus::Sidebar;
    select_account(&mut app, "archive");
    press(&mut app, "dd");
    app.settle().await;
    press(&mut app, "y");
    app.settle().await;
    app.settle().await;
    press(&mut app, "<Tab>|");
    app.settle().await;
    assert!(app.is_split());
    assert_ne!(app.pane.loc.remote, Remote::Named("archive".into()));
}
