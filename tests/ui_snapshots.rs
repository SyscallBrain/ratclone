mod common;

use std::time::Instant;

use common::app::*;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratclone::app::{App, AppEvent, Focus};
use ratclone::backend::{BackendError, Entry};
use ratclone::config::IconStyle;

fn render(app: &mut App, w: u16, h: u16) -> String {
    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
    term.draw(|f| ratclone::ui::draw(f, app)).unwrap();
    let buf = term.backend().buffer().clone();
    let mut out = String::new();
    for y in 0..h {
        let mut line = String::new();
        let mut skip = 0;
        for x in 0..w {
            if skip > 0 {
                skip -= 1;
                continue;
            }
            let sym = buf[(x, y)].symbol();
            skip = unicode_width::UnicodeWidthStr::width(sym).saturating_sub(1);
            line.push_str(sym);
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

async fn app() -> (
    tempfile::TempDir,
    std::sync::Arc<ratclone::backend::fake::FakeBackend>,
    App,
) {
    let (d, f, mut app) = demo_app().await;
    app.icons = IconStyle::Ascii;
    app.settings.icons = IconStyle::Ascii;
    (d, f, app)
}

#[tokio::test]
async fn main_local_home() {
    let (_d, _f, mut app) = app().await;
    insta::assert_snapshot!(render(&mut app, 110, 30));
}

#[tokio::test]
async fn main_files_tab() {
    let (_d, _f, mut app) = app().await;
    app.navigate(named("personal", "Photos/2025"), None);
    app.settle().await;
    insta::assert_snapshot!(render(&mut app, 110, 30));
}

#[tokio::test]
async fn space_tab_empty_chip() {
    let (_d, _f, mut app) = app().await;
    press(&mut app, "gt");
    insta::assert_snapshot!(render(&mut app, 110, 30));
}

#[tokio::test]
async fn sidebar_focused_with_usage() {
    let (_d, _f, mut app) = app().await;
    press(&mut app, "<Tab>jj");
    assert_eq!(app.focus, Focus::Sidebar);
    insta::assert_snapshot!(render(&mut app, 110, 30));
}

#[tokio::test]
async fn pane_with_marks_and_register() {
    let (_d, _f, mut app) = app().await;
    app.navigate(named("personal", "Photos/2025"), None);
    app.settle().await;
    move_to(&mut app, "IMG_0012.jpg");
    press(&mut app, "<Space><Space>y");
    move_to(&mut app, "IMG_0020.jpg");
    press(&mut app, "<Space>");
    insta::assert_snapshot!(render(&mut app, 110, 30));
}

#[tokio::test]
async fn pane_loading() {
    let (_d, fake, mut app) = app().await;
    fake.set_list_delay(std::time::Duration::from_secs(5));
    move_to(&mut app, "Downloads");
    press(&mut app, "l");
    insta::assert_snapshot!(render(&mut app, 110, 30));
}

#[tokio::test]
async fn pane_failed() {
    let (_d, fake, mut app) = app().await;
    fake.fail_next(
        "list",
        BackendError::Rclone("couldn't fetch token: invalid_grant".into()),
    );
    move_to(&mut app, "Downloads");
    press(&mut app, "l");
    app.settle().await;
    insta::assert_snapshot!(render(&mut app, 110, 30));
}

#[tokio::test]
async fn pane_empty() {
    let (_d, _f, mut app) = app().await;
    move_to(&mut app, "Documents");
    press(&mut app, "l");
    app.settle().await;
    insta::assert_snapshot!(render(&mut app, 110, 30));
}

async fn with_transfers(
    n: usize,
) -> (
    tempfile::TempDir,
    std::sync::Arc<ratclone::backend::fake::FakeBackend>,
    App,
) {
    let (d, f, mut app) = app().await;
    move_to(&mut app, "Downloads");
    press(&mut app, "l");
    app.settle().await;
    for name in ["video.mkv", "summary.pdf"].iter().take(n) {
        move_to(&mut app, name);
        press(&mut app, "y");
        app.navigate(named("work", ""), None);
        app.settle().await;
        press(&mut app, "p");
        app.settle().await;
        press(&mut app, "-");
        app.settle().await;
    }
    app.handle_event(AppEvent::Tick);
    app.settle().await;
    app.message = None;
    (d, f, app)
}

#[tokio::test]
async fn transfer_bar_single() {
    let (_d, _f, mut app) = with_transfers(1).await;
    insta::assert_snapshot!(render(&mut app, 110, 22));
}

#[tokio::test]
async fn transfer_bar_multi() {
    let (_d, _f, mut app) = with_transfers(2).await;
    insta::assert_snapshot!(render(&mut app, 110, 22));
}

#[tokio::test]
async fn narrow_terminal_60_cols() {
    let (_d, _f, mut app) = app().await;
    move_to(&mut app, "Downloads");
    press(&mut app, "l");
    app.settle().await;
    insta::assert_snapshot!(render(&mut app, 60, 20));
}

#[tokio::test]
async fn narrow_terminal_shows_the_accounts_list_when_it_has_the_keys() {
    let (_d, _f, mut app) = app().await;
    press(&mut app, "<Tab>j");
    assert_eq!(app.focus, Focus::Sidebar);
    let text = render(&mut app, 60, 24);
    assert!(text.contains("personal") && text.contains("work"), "{text}");
    insta::assert_snapshot!(text);
    press(&mut app, "<Tab>");
    assert_eq!(app.focus, Focus::Pane);
    assert!(
        render(&mut app, 60, 24).contains("Downloads"),
        "back to the files"
    );
}

#[tokio::test]
async fn a_long_filter_keeps_its_end_and_cursor_in_sight() {
    use ratclone::app::overlay::{InputPurpose, LineInput, Overlay};
    let (_d, _f, mut app) = app().await;
    let value = format!("{}tail", "x".repeat(86));
    app.overlay = Some(Overlay::Input {
        title: "Filter".into(),
        input: LineInput::new(&value, usize::MAX),
        purpose: InputPurpose::Filter,
    });
    let text = render(&mut app, 80, 24);
    let last = text.lines().last().unwrap();
    assert!(last.starts_with(" /…") && last.contains("xxtail"), "{last}");
    assert!(last.contains("Enter keep"), "{last}");
    // Too narrow for the hint too: the text keeps the line.
    let text = render(&mut app, 30, 24);
    let last = text.lines().last().unwrap();
    assert!(last.contains("xxtail") && !last.contains("Enter"), "{last}");
}

#[tokio::test]
async fn a_long_name_being_typed_keeps_its_end_and_the_keys() {
    use ratclone::app::overlay::{InputPurpose, LineInput, Overlay};
    let (_d, _f, mut app) = app().await;
    let value = format!("{}END.txt", "x".repeat(210));
    app.overlay = Some(Overlay::Input {
        title: "Rename file".into(),
        input: LineInput::new(&value, usize::MAX),
        purpose: InputPurpose::Mkdir,
    });
    let text = render(&mut app, 80, 24);
    assert!(text.contains("END.txt"), "{text}");
    assert!(text.contains("Enter save"), "{text}");
}

#[tokio::test]
async fn unicode_and_long_names() {
    let (_d, _f, mut app) = app().await;
    move_to(&mut app, "Downloads");
    press(&mut app, "l");
    app.settle().await;
    let text = render(&mut app, 110, 30);
    insta::assert_snapshot!(text.clone());
    // Size and date columns stay aligned whatever the name looks like.
    let ends: Vec<usize> = text
        .lines()
        .filter(|l| l.contains(" B ") || l.contains("iB "))
        .map(unicode_width::UnicodeWidthStr::width)
        .collect();
    assert!(ends.len() >= 4, "{text}");
    assert!(
        ends.windows(2).all(|w| w[0] == w[1]),
        "rows end at different columns: {ends:?}\n{text}"
    );
}

#[tokio::test]
async fn render_50k_entries_fast() {
    let (_d, _f, mut app) = app().await;
    let many: Vec<Entry> = (0..50_000)
        .map(|i| Entry {
            name: format!("img{i}.jpg"),
            size: Some(i),
            mod_time: None,
            is_dir: false,
            mime: String::new(),
        })
        .collect();
    app.pane.set_entries(many, None);
    app.pane.cursor = 25_000;
    let mut term = Terminal::new(TestBackend::new(100, 30)).unwrap();
    term.draw(|f| ratclone::ui::draw(f, &mut app)).unwrap();
    let start = Instant::now();
    term.draw(|f| ratclone::ui::draw(f, &mut app)).unwrap();
    let ms = start.elapsed().as_millis();
    let limit = if cfg!(debug_assertions) { 100 } else { 16 };
    assert!(ms < limit, "render took {ms} ms");
}

#[tokio::test]
async fn cursor_and_register_styles() {
    let (_d, _f, mut app) = app().await;
    move_to(&mut app, "Downloads");
    press(&mut app, "y");
    let mut term = Terminal::new(TestBackend::new(100, 30)).unwrap();
    term.draw(|f| ratclone::ui::draw(f, &mut app)).unwrap();
    let t = app.theme.tokens;
    // Row 3 is the first entry (Documents); row 4 is Downloads under the cursor.
    let buf = term.backend().buffer().clone();
    let name_cell = |y: u16| buf[(23, y)].clone();
    assert_eq!(
        name_cell(4).bg,
        t.sel,
        "focused cursor row uses the selection colour"
    );
    assert_eq!(name_cell(4).fg, t.yellow, "yanked entry is yellow");
    assert_ne!(name_cell(3).bg, t.sel);
    press(&mut app, "<Tab>");
    term.draw(|f| ratclone::ui::draw(f, &mut app)).unwrap();
    let buf = term.backend().buffer().clone();
    assert_eq!(buf[(23, 4)].bg, t.hl, "unfocused cursor row is dimmer");
}

async fn in_downloads() -> (
    tempfile::TempDir,
    std::sync::Arc<ratclone::backend::fake::FakeBackend>,
    App,
) {
    let (d, f, mut app) = app().await;
    move_to(&mut app, "Downloads");
    press(&mut app, "l");
    app.settle().await;
    (d, f, app)
}

#[tokio::test]
async fn confirm_delete() {
    let (_d, _f, mut app) = in_downloads().await;
    move_to(&mut app, "summary.pdf");
    press(&mut app, "<Space>");
    move_to(&mut app, "video.mkv");
    press(&mut app, "<Space>dd");
    insta::assert_snapshot!(render(&mut app, 110, 30));
}

#[tokio::test]
async fn rename_input() {
    let (_d, _f, mut app) = in_downloads().await;
    move_to(&mut app, "summary.pdf");
    press(&mut app, "cw");
    insta::assert_snapshot!(render(&mut app, 110, 30));
}

#[tokio::test]
async fn conflict_dialog() {
    let (_d, _f, mut app) = in_downloads().await;
    move_to(&mut app, "video.mkv");
    press(&mut app, "yp");
    app.settle().await;
    insta::assert_snapshot!(render(&mut app, 110, 30));
}

#[tokio::test]
async fn quit_dialog() {
    let (_d, _f, mut app) = with_transfers(1).await;
    press(&mut app, "q");
    insta::assert_snapshot!(render(&mut app, 110, 30));
}

#[tokio::test]
async fn help() {
    let (_d, _f, mut app) = app().await;
    press(&mut app, "?");
    let out = render(&mut app, 100, 34);
    assert!(
        out.contains("add account"),
        "account keys missing from help:\n{out}"
    );
    insta::assert_snapshot!(out);
}

#[tokio::test]
async fn transfers_view_mixed_states() {
    let (_d, fake, mut app) = with_transfers(1).await;
    fake.complete_transfers();
    app.handle_event(AppEvent::Tick);
    app.settle().await;
    // One failed, one running.
    move_to(&mut app, "summary.pdf");
    press(&mut app, "y");
    fake.delete_now(&home().join("Downloads/summary.pdf"));
    app.navigate(named("personal", ""), None);
    app.settle().await;
    press(&mut app, "p");
    app.settle().await;
    fake.complete_transfers();
    app.handle_event(AppEvent::Tick);
    app.settle().await;
    app.navigate(home().join("Downloads"), None);
    app.settle().await;
    move_to(&mut app, "video.mkv");
    press(&mut app, "y");
    app.navigate(named("media", ""), None);
    app.settle().await;
    press(&mut app, "p");
    app.settle().await;
    app.handle_event(AppEvent::Tick);
    app.settle().await;
    app.message = None;
    let fixed: jiff::Timestamp = "2025-08-01T10:00:00Z".parse().unwrap();
    for r in &mut app.store.records {
        r.started = fixed;
        r.finished = r.finished.map(|_| fixed);
    }
    press(&mut app, "T");
    insta::assert_snapshot!(render(&mut app, 110, 30));
}

#[tokio::test]
async fn transfers_view_many_items() {
    let (_d, _fake, mut app) = with_transfers(0).await;
    app.navigate(home().join("Downloads"), None);
    app.settle().await;
    for name in [
        "a very long file name that will certainly not fit in the column.tar.gz",
        "video.mkv",
        "日本語のファイル名.txt",
    ] {
        move_to(&mut app, name);
        press(&mut app, " ");
    }
    press(&mut app, "y");
    app.navigate(named("media", "Music"), None);
    app.settle().await;
    press(&mut app, "p");
    app.settle().await;
    app.handle_event(AppEvent::Tick);
    app.settle().await;
    // The first job has finished; rclone reports the second file as moving.
    let r = app.store.records.last_mut().unwrap();
    let mut jobs = r.jobids.clone();
    jobs.sort_unstable();
    r.done_jobs = vec![jobs[0]];
    r.started = "2025-08-01T10:00:00Z".parse().unwrap();
    let id = r.spec.id;
    if let Some(p) = app.progress.get_mut(&id) {
        p.files[0].name = "Music/video.mkv".into();
        p.files[0].percentage = 60;
    }
    app.message = None;
    press(&mut app, "T");
    insta::assert_snapshot!(render(&mut app, 110, 30));
}

#[tokio::test]
async fn panel_transfers_selected() {
    let (_d, fake, mut app) = with_transfers(1).await;
    fake.complete_transfers();
    app.handle_event(AppEvent::Tick);
    app.settle().await;
    // One failed, one running.
    move_to(&mut app, "summary.pdf");
    press(&mut app, "y");
    fake.delete_now(&home().join("Downloads/summary.pdf"));
    app.navigate(named("personal", ""), None);
    app.settle().await;
    press(&mut app, "p");
    app.settle().await;
    fake.complete_transfers();
    app.handle_event(AppEvent::Tick);
    app.settle().await;
    app.navigate(home().join("Downloads"), None);
    app.settle().await;
    move_to(&mut app, "video.mkv");
    press(&mut app, "y");
    app.navigate(named("media", ""), None);
    app.settle().await;
    press(&mut app, "p");
    app.settle().await;
    app.handle_event(AppEvent::Tick);
    app.settle().await;
    app.message = None;
    let fixed: jiff::Timestamp = "2025-08-01T10:00:00Z".parse().unwrap();
    for r in &mut app.store.records {
        r.started = fixed;
        r.finished = r.finished.map(|_| fixed);
    }
    press(&mut app, "T");
    app.transfers.cursor = 0;
    insta::assert_snapshot!(render(&mut app, 110, 30));
}

#[tokio::test]
async fn transfers_view_wide_title_keeps_columns() {
    let (_d, fake, mut app) = in_downloads().await;
    move_to(&mut app, "日本語のファイル名.txt");
    press(&mut app, "y");
    app.navigate(named("work", ""), None);
    app.settle().await;
    press(&mut app, "p");
    app.settle().await;
    fake.complete_transfers();
    app.handle_event(AppEvent::Tick);
    app.settle().await;
    let fixed: jiff::Timestamp = "2025-08-01T10:00:00Z".parse().unwrap();
    for r in &mut app.store.records {
        r.started = fixed;
        r.finished = Some(fixed);
    }
    press(&mut app, "T");
    let text = render(&mut app, 110, 30);
    let row = text.lines().find(|l| l.contains("✓ copy")).unwrap();
    assert!(row.ends_with("2025-08-01"), "date column cut off: {row:?}");
}

#[tokio::test]
async fn control_characters_in_names_never_reach_the_terminal() {
    let (_d, fake, mut app) = in_downloads().await;
    fake.add_file(
        &home().join("Downloads/evil\u{1b}]52;c;aGk=\u{7}\u{1b}[2J.txt"),
        1,
    );
    press(&mut app, "R");
    app.settle().await;
    let mut term = Terminal::new(TestBackend::new(100, 30)).unwrap();
    term.draw(|f| ratclone::ui::draw(f, &mut app)).unwrap();
    let buf = term.backend().buffer().clone();
    for cell in buf.content() {
        assert!(
            !cell.symbol().chars().any(|c| c.is_control()),
            "control char in cell {:?}",
            cell.symbol()
        );
    }
}

// ---- info panel ----------------------------------------------------------

const GIB: u64 = 1024 * 1024 * 1024;

fn speeds(app: &mut App) {
    for i in 0..60u64 {
        let v = [5, 10, 20, 35, 50, 62, 70, 78, 74, 80, 85, 90, 82, 76, 80][(i % 15) as usize];
        app.speed.push(v as f64 * 160_000.0);
    }
}

async fn in_photos(
    icons: IconStyle,
) -> (
    tempfile::TempDir,
    std::sync::Arc<ratclone::backend::fake::FakeBackend>,
    App,
) {
    let (d, f, mut app) = app().await;
    app.icons = icons;
    app.navigate(named("personal", "Photos/2025"), None);
    app.settle().await;
    move_to(&mut app, "IMG_0013.jpg");
    let personal = ratclone::location::Remote::Named("personal".into());
    let about = ratclone::backend::About {
        total: Some(15 * GIB),
        used: Some(9 * GIB + 300 * 1024 * 1024),
        free: Some(4 * GIB),
        trashed: Some(GIB + 200 * 1024 * 1024),
        other: Some(800 * 1024 * 1024),
        objects: None,
    };
    app.sidebar.about.insert(personal, (about, Instant::now()));
    speeds(&mut app);
    (d, f, app)
}

#[tokio::test]
async fn panel_files_personal() {
    let (_d, _f, mut app) = in_photos(IconStyle::Ascii).await;
    insta::assert_snapshot!(render(&mut app, 110, 30));
}

#[tokio::test]
async fn panel_nerd_icons() {
    let (_d, _f, mut app) = in_photos(IconStyle::Nerd).await;
    insta::assert_snapshot!(render(&mut app, 110, 30));
}

#[tokio::test]
async fn panel_hides_unknown_fields() {
    let (_d, _f, mut app) = app().await;
    app.navigate(named("media", ""), None);
    app.settle().await;
    insta::assert_snapshot!(render(&mut app, 110, 30));
}

#[tokio::test]
async fn panel_no_quota() {
    let (_d, _f, mut app) = in_photos(IconStyle::Ascii).await;
    let personal = ratclone::location::Remote::Named("personal".into());
    let about = ratclone::backend::About {
        used: Some(9 * GIB),
        ..Default::default()
    };
    app.sidebar.about.insert(personal, (about, Instant::now()));
    insta::assert_snapshot!(render(&mut app, 110, 30));
}

#[tokio::test]
async fn panel_compact_short_terminal() {
    let (_d, _f, mut app) = in_photos(IconStyle::Ascii).await;
    insta::assert_snapshot!(render(&mut app, 110, 22));
}

#[tokio::test]
async fn panel_hidden_shows_transfer_bar() {
    let (_d, _f, mut app) = with_transfers(1).await;
    insta::assert_snapshot!(render(&mut app, 110, 14));
}

#[tokio::test]
async fn panel_narrow_drops_selected() {
    let (_d, _f, mut app) = in_photos(IconStyle::Ascii).await;
    insta::assert_snapshot!(render(&mut app, 90, 30));
}

#[tokio::test]
async fn panel_toggled_off() {
    let (_d, _f, mut app) = in_photos(IconStyle::Ascii).await;
    press(&mut app, "i");
    insta::assert_snapshot!(render(&mut app, 110, 30));
}

#[tokio::test]
async fn panel_with_marks() {
    let (_d, _f, mut app) = in_photos(IconStyle::Ascii).await;
    press(&mut app, "<Space><Space><Space>");
    insta::assert_snapshot!(render(&mut app, 110, 30));
}

// ---- space tab -------------------------------------------------------------

async fn space_personal() -> (
    tempfile::TempDir,
    std::sync::Arc<ratclone::backend::fake::FakeBackend>,
    App,
) {
    let (d, f, mut app) = app().await;
    press(&mut app, "<Tab>j<Tab>U");
    app.settle().await;
    (d, f, app)
}

async fn space_scanned() -> (
    tempfile::TempDir,
    std::sync::Arc<ratclone::backend::fake::FakeBackend>,
    App,
) {
    let (d, f, mut app) = space_personal().await;
    press(&mut app, "<CR>");
    app.settle().await;
    app.message = None;
    (d, f, app)
}

#[tokio::test]
async fn space_empty_state() {
    let (_d, _f, mut app) = space_personal().await;
    insta::assert_snapshot!(render(&mut app, 110, 30));
}

#[tokio::test]
async fn space_list_root() {
    let (_d, _f, mut app) = space_scanned().await;
    insta::assert_snapshot!(render(&mut app, 110, 30));
}

#[tokio::test]
async fn space_list_inside_photos() {
    let (_d, _f, mut app) = space_scanned().await;
    press(&mut app, "l");
    insta::assert_snapshot!(render(&mut app, 110, 30));
}

#[tokio::test]
async fn space_scanning_progress() {
    let (_d, fake, mut app) = space_personal().await;
    fake.set_list_delay(std::time::Duration::from_millis(300));
    press(&mut app, "<CR>");
    // Only the root listing: everything after it is still in flight.
    loop {
        let ev = app.next_event().await.unwrap();
        let root_listed = matches!(
            &ev,
            AppEvent::Scan {
                msg: ratclone::scan::walk::ScanMsg::Listed { .. },
                ..
            }
        );
        app.handle_event(ev);
        if root_listed {
            break;
        }
    }
    let scan = app.scan.as_mut().unwrap();
    scan.started = Instant::now() - std::time::Duration::from_secs(48);
    insta::assert_snapshot!(render(&mut app, 110, 30));
    app.cancel_scan();
    app.settle().await;
}

#[tokio::test]
async fn space_partial_and_unreadable() {
    let (_d, fake, mut app) = space_personal().await;
    fake.fail_list_at(
        &named("personal", "Backups"),
        BackendError::Rclone("permission denied".into()),
    );
    press(&mut app, "<CR>");
    app.settle().await;
    app.message = None;
    let root = named("personal", "");
    app.scans.get_mut(&root).unwrap().partial = true;
    insta::assert_snapshot!(render(&mut app, 110, 30));
}

#[tokio::test]
async fn space_changed_since_scan() {
    let (_d, _f, mut app) = space_scanned().await;
    app.note_change(&ratclone::location::Remote::Named("personal".into()));
    insta::assert_snapshot!(render(&mut app, 110, 30));
}

#[tokio::test]
async fn space_unicode_and_long_names() {
    let (_d, fake, mut app) = space_personal().await;
    fake.add_file(&named("personal", "日本語のフォルダ/写真.jpg"), 2_000_000_000);
    fake.add_file(
        &named(
            "personal",
            "a folder with a very long name that will never fit in the name column of the list/x.bin",
        ),
        1_800_000_000,
    );
    press(&mut app, "<CR>");
    app.settle().await;
    app.message = None;
    insta::assert_snapshot!(render(&mut app, 110, 30));
}

#[tokio::test]
async fn space_chip_spinner_on_files_tab() {
    let (_d, fake, mut app) = space_personal().await;
    fake.set_list_delay(std::time::Duration::from_millis(300));
    press(&mut app, "<CR>");
    app.set_tab(ratclone::app::Tab::Files);
    let top = render(&mut app, 110, 30).lines().next().unwrap().to_string();
    insta::assert_snapshot!(top);
    app.cancel_scan();
    app.settle().await;
}

#[tokio::test]
async fn space_marks_the_account_it_shows() {
    let (_d, _f, mut app) = space_personal().await;
    let screen = render(&mut app, 110, 30);
    let marked: Vec<&str> = screen.lines().filter(|l| l.starts_with('▌')).collect();
    assert_eq!(marked.len(), 1);
    assert!(marked[0].contains("personal"), "{marked:?}");
}

#[tokio::test]
async fn scan_progress_survives_narrow_terminals() {
    let (_d, fake, mut app) = space_personal().await;
    fake.set_list_delay(std::time::Duration::from_millis(300));
    press(&mut app, "<CR>");
    if let Some(a) = app.scan.as_mut() {
        a.current = "Music/album/a very long folder name indeed".into();
        a.tree.node.files = 1_234_567;
    }
    for w in [70u16, 90, 100] {
        let header = render(&mut app, w, 30).lines().nth(1).unwrap().to_string();
        assert!(header.contains("Esc cancel"), "{w} cols: {header}");
        assert!(header.contains('%'), "{w} cols: {header}");
    }
    app.cancel_scan();
    app.settle().await;
}

#[tokio::test]
async fn hidden_key_hints_leave_only_help() {
    let (_d, _f, mut app) = app().await;
    app.key_hints = false;
    let screen = render(&mut app, 110, 30);
    let last = screen.lines().last().unwrap().to_string();
    insta::assert_snapshot!(last);
}

#[tokio::test]
async fn panel_shows_limit() {
    let (_d, _f, mut app) = in_photos(IconStyle::Ascii).await;
    app.bwlimit_shown = Some("10M".into());
    insta::assert_snapshot!(render(&mut app, 110, 30));
}

// ---- settings dialog --------------------------------------------------------

fn settings_cursor(app: &mut App, id: ratclone::app::settings::SettingId) {
    let Some(ratclone::app::overlay::Overlay::Settings(dlg)) = app.overlay.as_mut() else {
        panic!("settings not open")
    };
    dlg.cursor = ratclone::app::settings::SettingId::ALL
        .iter()
        .position(|s| *s == id)
        .unwrap();
}

#[tokio::test]
async fn settings_default() {
    let (_d, _f, mut app) = app().await;
    press(&mut app, ",");
    insta::assert_snapshot!(render(&mut app, 110, 30));
}

#[tokio::test]
async fn settings_changed_values() {
    use ratclone::app::settings::SettingId;
    let (_d, _f, mut app) = app().await;
    app.icons = IconStyle::Nerd;
    app.settings.icons = IconStyle::Nerd;
    press(&mut app, ",");
    settings_cursor(&mut app, SettingId::Icons);
    press(&mut app, "l");
    settings_cursor(&mut app, SettingId::Theme);
    press(&mut app, "l");
    insta::assert_snapshot!(render(&mut app, 110, 30));
}

#[tokio::test]
async fn settings_custom_bandwidth_input() {
    use ratclone::app::settings::SettingId;
    let (_d, _f, mut app) = app().await;
    press(&mut app, ",");
    settings_cursor(&mut app, SettingId::Bwlimit);
    press(&mut app, "h<CR>75");
    insta::assert_snapshot!(render(&mut app, 110, 30));
}

#[tokio::test]
async fn settings_small_terminal() {
    let (_d, _f, mut app) = app().await;
    press(&mut app, ",");
    insta::assert_snapshot!(render(&mut app, 50, 16));
}

#[tokio::test]
async fn settings_too_small() {
    let (_d, _f, mut app) = app().await;
    press(&mut app, ",");
    insta::assert_snapshot!(render(&mut app, 38, 10));
}

// ---- account wizard ---------------------------------------------------------

async fn accounts_app() -> (
    tempfile::TempDir,
    std::sync::Arc<ratclone::backend::fake::FakeBackend>,
    App,
) {
    let (d, f, mut app) = app().await;
    app.focus = Focus::Sidebar;
    app.over_ssh = false;
    (d, f, app)
}

async fn open_type(app: &mut App, query: &str) {
    press(app, "a");
    app.settle().await;
    press(app, query);
}

#[tokio::test]
async fn account_type_list() {
    let (_d, _f, mut app) = accounts_app().await;
    open_type(&mut app, "").await;
    insta::assert_snapshot!(render(&mut app, 110, 30));
}

#[tokio::test]
async fn account_type_filtered() {
    let (_d, _f, mut app) = accounts_app().await;
    open_type(&mut app, "dr").await;
    insta::assert_snapshot!(render(&mut app, 110, 30));
}

#[tokio::test]
async fn account_form_add_mega() {
    let (_d, _f, mut app) = accounts_app().await;
    open_type(&mut app, "mega").await;
    press(&mut app, "<CR><Down>me@example.com<Down>hunter2");
    let out = render(&mut app, 110, 30);
    assert!(!out.contains("hunter2"), "password shown:\n{out}");
    insta::assert_snapshot!(out);
}

#[tokio::test]
async fn account_form_advanced_shown() {
    let (_d, _f, mut app) = accounts_app().await;
    open_type(&mut app, "mega").await;
    press(&mut app, "<CR><Tab><Down><Down><Down><Down>");
    insta::assert_snapshot!(render(&mut app, 110, 30));
}

#[tokio::test]
async fn account_form_edit_password_unchanged() {
    let (_d, f, mut app) = accounts_app().await;
    f.set_config_value("media", "user", "old@example.com");
    f.set_config_value("media", "pass", "obscured-old");
    app.sidebar
        .select(&ratclone::location::Remote::Named("media".into()));
    press(&mut app, "e");
    app.settle().await;
    press(&mut app, "<C-u>new@example.com");
    insta::assert_snapshot!(render(&mut app, 110, 30));
}

#[tokio::test]
async fn account_question_with_error() {
    let (_d, _f, mut app) = accounts_app().await;
    app.over_ssh = true;
    open_type(&mut app, "dropbox").await;
    press(&mut app, "<CR><C-s>");
    app.settle().await;
    press(&mut app, "notjson<CR>");
    app.settle().await;
    insta::assert_snapshot!(render(&mut app, 110, 30));
}

#[tokio::test]
async fn account_drive_team_question() {
    let (_d, _f, mut app) = accounts_app().await;
    open_type(&mut app, "drive").await;
    press(&mut app, "<CR><C-s>");
    app.settle().await;
    press(&mut app, "y");
    app.settle().await;
    app.handle_event(AppEvent::Tick);
    app.settle().await;
    insta::assert_snapshot!(render(&mut app, 110, 30));
}

#[tokio::test]
async fn account_waiting_browser() {
    let (_d, f, mut app) = accounts_app().await;
    f.set_auth_delay(std::time::Duration::from_secs(60));
    open_type(&mut app, "dropbox").await;
    press(&mut app, "<CR><C-s>");
    app.settle().await;
    app.spinner = 0;
    insta::assert_snapshot!(render(&mut app, 110, 30));
}

#[tokio::test]
async fn account_token_paste() {
    let (_d, _f, mut app) = accounts_app().await;
    app.over_ssh = true;
    open_type(&mut app, "dropbox").await;
    press(&mut app, "<CR><C-s>");
    app.settle().await;
    press(&mut app, "{\"secret\":1}");
    let out = render(&mut app, 110, 30);
    assert!(!out.contains("secret"), "token shown:\n{out}");
    insta::assert_snapshot!(out);
}

#[tokio::test]
async fn account_drive_warning() {
    let (_d, _f, mut app) = accounts_app().await;
    open_type(&mut app, "drive").await;
    press(&mut app, "<CR><C-s>");
    app.settle().await;
    insta::assert_snapshot!(render(&mut app, 110, 30));
}

#[tokio::test]
async fn account_remove_confirm_with_dependents() {
    let (_d, f, mut app) = accounts_app().await;
    f.add_remote("vault", "crypt");
    f.set_config_value("vault", "remote", "media:Secret");
    app.sidebar
        .select(&ratclone::location::Remote::Named("media".into()));
    press(&mut app, "dd");
    app.settle().await;
    insta::assert_snapshot!(render(&mut app, 110, 30));
}

#[tokio::test]
async fn account_too_small() {
    let (_d, _f, mut app) = accounts_app().await;
    open_type(&mut app, "").await;
    insta::assert_snapshot!(render(&mut app, 39, 11));
}

#[tokio::test]
async fn account_form_ascii_narrow() {
    let (_d, _f, mut app) = accounts_app().await;
    open_type(&mut app, "crypt").await;
    press(&mut app, "<CR>");
    insta::assert_snapshot!(render(&mut app, 60, 20));
}

#[tokio::test]
async fn account_choice_and_suggestion_help_drawn() {
    let (_d, _f, mut app) = accounts_app().await;
    open_type(&mut app, "crypt").await;
    press(&mut app, "<CR><Down><Down>");
    let out = render(&mut app, 110, 30);
    assert!(
        out.contains("Encrypt the filenames."),
        "choice help missing:\n{out}"
    );
    press(&mut app, "<Esc><Esc>");
    open_type(&mut app, "drive").await;
    press(&mut app, "<CR><Down><Down><Down>");
    let out = render(&mut app, 110, 30);
    assert!(
        out.contains("Examples: drive, drive.readonly"),
        "suggestions missing:\n{out}"
    );
    assert!(out.contains("^s"), "save key not shown:\n{out}");
}

#[tokio::test]
async fn account_panel_lists_actions() {
    let (_d, _f, mut app) = accounts_app().await;
    app.sidebar
        .select(&ratclone::location::Remote::Named("media".into()));
    let out = render(&mut app, 140, 34);
    assert!(out.contains("a add · e edit · dd remove"), "{out}");
}

#[tokio::test]
async fn account_long_question_help_keeps_input_row() {
    let (_d, _f, mut app) = accounts_app().await;
    open_type(&mut app, "drive").await;
    press(&mut app, "<CR><C-s>");
    app.settle().await;
    press(&mut app, "y");
    app.settle().await;
    app.handle_event(AppEvent::Tick);
    app.settle().await;
    if let Some(ratclone::app::overlay::Overlay::Account(w)) = app.overlay.as_mut()
        && let ratclone::app::account::Step::Question { field, .. } = &mut w.step
    {
        field.option.help = (1..=40)
            .map(|i| format!("Line {i} of a long help."))
            .collect::<Vec<_>>()
            .join("\n");
    } else {
        panic!("not on a question");
    }
    let out = render(&mut app, 110, 30);
    assert!(
        out.contains("▸ config_team_drive"),
        "input row pushed out:\n{out}"
    );
}

#[tokio::test]
async fn account_long_value_shows_cursor_end() {
    let (_d, _f, mut app) = accounts_app().await;
    open_type(&mut app, "alias").await;
    press(&mut app, "<CR><Down>");
    press(
        &mut app,
        "/a/very/long/path/that/does/not/fit/in/the/box/at/all/END",
    );
    let out = render(&mut app, 60, 20);
    assert!(out.contains("END█"), "cursor end not visible:\n{out}");
}

#[tokio::test]
async fn long_account_list_keeps_the_cursor_on_screen() {
    let (_d, _f, mut app) = app().await;
    for i in 0..30 {
        app.sidebar.remotes.push(ratclone::backend::RemoteInfo {
            remote: ratclone::location::Remote::Named(format!("acct{i:02}")),
            kind: "drive".into(),
        });
    }
    app.focus = Focus::Sidebar;
    press(&mut app, "G");
    let last = app.sidebar.remotes.len() - 1;
    assert_eq!(app.sidebar.cursor, last);
    let out = render(&mut app, 110, 24);
    assert!(
        out.contains("acct29"),
        "the selected account is off screen:\n{out}"
    );
    // Smaller terminal: still there.
    let out = render(&mut app, 110, 12);
    assert!(out.contains("acct29"), "{out}");
    press(&mut app, "gg");
    let out = render(&mut app, 110, 24);
    assert!(out.contains("local") && !out.contains("acct29"), "{out}");
    // Halfway down, one step at a time.
    press(&mut app, "20j");
    let name = match &app.sidebar.current().unwrap().remote {
        ratclone::location::Remote::Named(n) => n.clone(),
        _ => unreachable!(),
    };
    let out = render(&mut app, 110, 24);
    assert!(out.contains(&name), "{name} not shown:\n{out}");
    // Accounts removed under the cursor: the list is drawn without panicking.
    app.sidebar.remotes.truncate(5);
    let _ = render(&mut app, 110, 24);
}

#[tokio::test]
async fn every_key_is_reachable_in_the_help_at_80x24() {
    use ratclone::input::keymap::{KeyContext, bindings};
    let (_d, _f, mut app) = app().await;
    press(&mut app, "?");
    let mut seen = String::new();
    let mut last = String::new();
    for _ in 0..200 {
        let out = render(&mut app, 80, 24);
        seen.push_str(&out);
        if out == last {
            break;
        }
        last = out;
        press(&mut app, "j");
        assert!(app.overlay.is_some(), "a scroll key closed the help");
    }
    for ctx in [
        KeyContext::Pane,
        KeyContext::Sidebar,
        KeyContext::Transfers,
        KeyContext::Space,
    ] {
        for (group, _, desc, _) in bindings(ctx) {
            // The accounts list and Transfers tab move like the file list does.
            let mirrored =
                group == "Navigation" && matches!(ctx, KeyContext::Sidebar | KeyContext::Transfers);
            if desc == "close" || mirrored {
                continue;
            }
            assert!(seen.contains(desc), "{desc:?} ({ctx:?}) never shown at 80x24");
        }
    }
    assert!(seen.contains("Space tab"), "no Space section");
    // Back to the top, then explicit keys close it.
    press(&mut app, "g");
    assert!(render(&mut app, 80, 24).contains("Navigation"));
    press(&mut app, "<Esc>");
    assert!(app.overlay.is_none());
    press(&mut app, "?q");
    assert!(app.overlay.is_none());
}

#[tokio::test]
async fn dialogs_survive_tiny_terminals() {
    use ratclone::app::overlay::{Overlay, PendingOp};
    let (_d, _f, mut app) = app().await;
    let overlays = || {
        vec![
            Overlay::Help { scroll: 0 },
            Overlay::Quit { running: 2, ops: 0 },
            Overlay::Confirm {
                title: "Delete".into(),
                lines: vec!["Delete a.txt?".into()],
                commands: vec!["rclone deletefile x:a.txt".into()],
                danger: true,
                on_yes: PendingOp::Rescan(named("personal", "")),
            },
        ]
    };
    for o in overlays() {
        app.overlay = Some(o);
        for w in 0..=30 {
            for h in [0, 1, 2, 3, 5, 8, 24] {
                let _ = render(&mut app, w, h);
            }
        }
    }
    app.overlay = Some(Overlay::Quit { running: 1, ops: 0 });
    assert!(render(&mut app, 8, 24).contains("small"));
    // From a usable size the dialog itself is drawn.
    assert!(render(&mut app, 80, 24).contains("Quit"));
}

#[tokio::test]
async fn a_confirm_with_long_lines_still_shows_its_keys() {
    use ratclone::app::overlay::{Overlay, PendingOp};
    let (_d, _f, mut app) = app().await;
    app.overlay = Some(Overlay::Confirm {
        title: "Remove from the list".into(),
        lines: vec![
            "Remove phone_backup_2026-08-01.zip → archive:/Backups/Phone/2026 from the list?".into(),
            "It can no longer be resumed; files already copied stay where they are.".into(),
        ],
        commands: Vec::new(),
        danger: false,
        on_yes: PendingOp::RemoveTransfers(vec![1]),
    });
    for w in [40, 60, 80, 120] {
        let out = render(&mut app, w, 30);
        assert!(out.contains("cancel"), "keys hidden at width {w}:\n{out}");
        assert!(out.contains("they are."), "text cut at width {w}:\n{out}");
    }
}

#[tokio::test]
async fn split_two_accounts() {
    let (_d, _f, mut app) = app().await;
    common::app::press(&mut app, "|");
    app.settle().await;
    insta::assert_snapshot!(render(&mut app, 140, 30));
}

#[tokio::test]
async fn split_narrow_shows_focused() {
    let (_d, _f, mut app) = app().await;
    common::app::press(&mut app, "|");
    app.settle().await;
    insta::assert_snapshot!(render(&mut app, 90, 30));
}

/// A list longer than the window keeps its place when the cursor moves back up:
/// only a cursor about to leave the window scrolls it (as in the file list).
#[tokio::test]
async fn long_lists_keep_their_place_when_the_cursor_moves_back() {
    use ratclone::location::{Location, Remote};
    use ratclone::transfer::spec::{TransferItem, TransferKind, TransferSpec};

    let (_d, _f, mut app) = app().await;
    let fixed: jiff::Timestamp = "2025-08-01T10:00:00Z".parse().unwrap();
    for id in 1..=60 {
        let spec = TransferSpec {
            id,
            kind: TransferKind::Copy,
            src_dir: home(),
            dst_dir: Location::new(Remote::Named("work".into()), ""),
            items: vec![TransferItem {
                name: format!("file-{id}.bin"),
                is_dir: false,
                dst_name: format!("file-{id}.bin"),
                size: None,
                replace: false,
            }],
        };
        app.store.push(spec, vec![id], fixed);
        app.store.mark_stopped(id, fixed);
    }
    press(&mut app, "T");
    render(&mut app, 110, 30);
    assert_eq!(app.transfers.offset, 0);
    press(&mut app, "30j");
    render(&mut app, 110, 30);
    let scrolled = app.transfers.offset;
    assert!(scrolled > 0, "the cursor left the window: the list follows");
    press(&mut app, "3k");
    render(&mut app, 110, 30);
    assert_eq!(
        app.transfers.offset, scrolled,
        "still in the window: nothing moves"
    );
    press(&mut app, "gg");
    render(&mut app, 110, 30);
    assert_eq!(app.transfers.offset, 0);
    // A shorter list (or a taller window) never leaves the list scrolled past its end.
    press(&mut app, "G");
    render(&mut app, 110, 30);
    assert!(app.transfers.offset > 0);
    render(&mut app, 110, 200);
    assert_eq!(app.transfers.offset, 0);
}
