use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

use clap::Parser;
use crossterm::event::{Event, EventStream, KeyEventKind};
use futures::StreamExt;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio::signal::unix::{SignalKind, signal};
use tokio::time::MissedTickBehavior;

use ratclone::app::{App, AppEvent, ReconnectFn};
use ratclone::backend::Backend;
use ratclone::backend::fake::FakeBackend;
use ratclone::backend::rc::RcBackend;
use ratclone::cli::Cli;
use ratclone::daemon::{self, DaemonError, DaemonStatus};
use ratclone::location::{Location, Remote};
use ratclone::rc::UnixRc;
use ratclone::transfer::store::TransferStore;
use ratclone::{config, paths, reaper};

/// The rcd's log is set aside (one old copy kept) once it is larger than this.
const LOG_MAX_BYTES: u64 = 10 * 1024 * 1024;

fn main() {
    let cli = Cli::parse();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    if let Err(e) = runtime.block_on(run(cli)) {
        // Not `eprintln!`: it panics when the terminal is already gone.
        let _ = writeln!(std::io::stderr(), "ratclone: {e}");
        std::process::exit(1);
    }
}

fn home_location() -> Location {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
    Location::new(Remote::Local, &home)
}

/// Reuse the app's rcd or start one, logging to `log`. The log is rotated only
/// when an rcd is about to be started: a running one keeps writing to the file it
/// opened, wherever it is moved.
async fn start_rcd(rclone: &Path, socket: &Path, log: &Path) -> Result<(UnixRc, DaemonStatus), DaemonError> {
    let args = ["--log-file".to_string(), log.display().to_string()];
    daemon::ensure_running_with(rclone, socket, &args, || paths::rotate_log(log, LOG_MAX_BYTES)).await
}

async fn run(cli: Cli) -> color_eyre::Result<()> {
    color_eyre::install()?;
    let socket = cli.socket.clone().unwrap_or_else(paths::socket_path);
    if cli.reap_when_idle {
        // Before anything that can fail: a config file broken since the app quit
        // must not leave the rcd running for ever.
        let rc = UnixRc::new(socket);
        reaper::reap_when_idle(&rc, &paths::state_dir(), reaper::INTERVAL).await;
        return Ok(());
    }

    let config = config::load(&paths::config_file())
        .map_err(|e| color_eyre::eyre::eyre!("{}: {e}", paths::config_file().display()))?;
    let rclone = cli
        .rclone
        .clone()
        .or(config.rclone_path.clone())
        .unwrap_or_else(|| "rclone".into());
    let log = paths::state_dir().join("rcd.log");

    let mut instance_lock: Option<std::fs::File> = None;
    let (backend, store, start, demo_store): (Arc<dyn Backend>, TransferStore, Location, Option<PathBuf>) =
        if cli.demo {
            let path = std::env::temp_dir().join(format!("ratclone-demo-{}.json", std::process::id()));
            let store = TransferStore::load(path.clone())?;
            (
                Arc::new(FakeBackend::demo()),
                store,
                Location::new(Remote::Local, "home/demo"),
                Some(path),
            )
        } else {
            let lock = acquire_lock_patiently().map_err(|_| {
                color_eyre::eyre::eyre!("another ratclone is already running (one instance at a time)")
            })?;
            instance_lock = Some(lock);
            let (rc, status) = start_rcd(&rclone, &socket, &log).await?;
            let mut store = TransferStore::load(paths::state_dir().join(reaper::STORE_FILE))?;
            if status == DaemonStatus::Started {
                store.interrupt_all_running();
                store.save()?;
            }
            (
                Arc::new(RcBackend::with_list_dir(rc, paths::state_dir().join("lists"))),
                store,
                home_location(),
                None,
            )
        };

    let mut app = App::new(backend.clone(), store, &config, start);
    // The demo shows the same made-up machine as its made-up accounts.
    app.host = if cli.demo {
        "demo@laptop".into()
    } else {
        ratclone::ui::tabs::host_label()
    };
    if !cli.demo {
        let ui_state = paths::state_dir().join("ui.json");
        let ui = ratclone::app::panel::UiState::load(&ui_state);
        // Older versions remembered `i` in ui.json; honour it until the setting is saved.
        if config.info_panel.is_none() && ui.panel_hidden {
            app.panel_visible = false;
        }
        app.pending_start = match app.settings.start_in.as_str() {
            "home" => None,
            "last" => ui.last_location.clone(),
            name => Some(Location::new(Remote::Named(name.to_string()), "")),
        };
        app.ui_state_path = Some(ui_state);
        app.scan_cache_dir = Some(paths::cache_dir().join("scans"));
        app.config_path = Some(paths::config_file());
        if app.settings.bwlimit != "off" {
            let rate = app.settings.bwlimit.clone();
            app.send_bwlimit(rate);
        } else {
            // A limit set before (rclone keeps it while it runs) still shows in the panel.
            app.query_bwlimit();
        }
        let (rclone, socket, log) = (rclone.clone(), socket.clone(), log.clone());
        let reconnect: ReconnectFn = Arc::new(move || {
            let (rclone, socket, log) = (rclone.clone(), socket.clone(), log.clone());
            Box::pin(async move {
                start_rcd(&rclone, &socket, &log)
                    .await
                    .map(|(_, status)| status == DaemonStatus::Started)
                    .map_err(|e| e.to_string())
            })
        });
        app.set_reconnect(reconnect);
    }
    // Pick up transfers that kept running while the app was closed.
    app.handle_event(AppEvent::Tick);

    install_panic_hook((!cli.demo).then(|| socket.clone()));
    let mut terminal = enter_terminal()?;
    let result = event_loop(&mut terminal, &mut app).await;
    // Dropped, ratatui shows the cursor again and prints when it cannot, and on a
    // terminal that hung up the print itself fails, which is a panic. Nothing of it
    // is left to drop: `leave_terminal` gives the screen back, quietly.
    std::mem::forget(terminal);
    leave_terminal();

    // However the loop ended (a quit, a signal, a terminal that went away), what
    // the app knows is saved and the rcd is stopped, or left to a helper that
    // stops it once it is idle. Nothing below may be skipped by an early return.
    // Stop requests and deletes already sent are answered before rclone goes away.
    app.finish_ops().await;
    if let Some(path) = &app.ui_state_path {
        let mut ui = ratclone::app::panel::UiState::load(path);
        ui.last_location = Some(app.left_pane().loc.clone());
        let _ = ui.save(path);
    }
    let _ = app.store.save();
    // The lock is held until rclone is stopped: an app opened meanwhile would
    // otherwise reuse the rcd this one is about to stop.
    if !cli.demo {
        if result.is_ok() && app.shutdown_on_exit {
            let _ = backend.shutdown().await;
        } else {
            spawn_reaper(&socket);
        }
    }
    drop(instance_lock);
    if let Some(p) = demo_store {
        let _ = std::fs::remove_file(p);
    }
    result
}

/// How long a starting app waits for the lock: a closing app or a reaper holds it
/// while it stops rclone (up to a few seconds).
const LOCK_PATIENCE: Duration = Duration::from_secs(15);

/// Take the instance lock, waiting while another holder is only closing.
fn acquire_lock_patiently() -> std::io::Result<std::fs::File> {
    let start = std::time::Instant::now();
    let mut said = false;
    loop {
        match paths::acquire_instance_lock(&paths::state_dir()) {
            Ok(f) => return Ok(f),
            Err(e) if start.elapsed() >= LOCK_PATIENCE => return Err(e),
            Err(_) => {}
        }
        if !said && start.elapsed() >= Duration::from_secs(1) {
            eprintln!("Waiting for the previous ratclone to close…");
            said = true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Leave a detached `ratclone --reap-when-idle` behind to record how the background
/// transfers end and to stop the rcd once they are done (and no app is open).
fn spawn_reaper(socket: &Path) {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    let Ok(exe) = std::env::current_exe() else { return };
    let mut cmd = Command::new(exe);
    cmd.arg("--reap-when-idle")
        .arg("--socket")
        .arg(socket)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: setsid only detaches the child from our session.
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let _ = cmd.spawn();
}

type Term = Terminal<CrosstermBackend<std::io::Stdout>>;

fn enter_terminal() -> color_eyre::Result<Term> {
    crossterm::terminal::enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    // Bracketed paste: pasted text arrives as one event, not as keys to obey.
    crossterm::execute!(
        stdout,
        crossterm::terminal::EnterAlternateScreen,
        crossterm::event::EnableBracketedPaste,
        crossterm::cursor::Hide
    )?;
    Ok(Terminal::new(CrosstermBackend::new(stdout))?)
}

fn leave_terminal() {
    let mut stdout = std::io::stdout();
    let _ = crossterm::execute!(
        stdout,
        crossterm::event::DisableBracketedPaste,
        crossterm::terminal::LeaveAlternateScreen,
        crossterm::cursor::Show
    );
    let _ = crossterm::terminal::disable_raw_mode();
    let _ = stdout.flush();
}

/// A panic anywhere (the UI or a background task) ends the app: the terminal is
/// given back, the message is shown, and the rcd at `rcd_socket` (if the app has
/// one) is left to a helper instead of running for ever.
fn install_panic_hook(rcd_socket: Option<PathBuf>) {
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        leave_terminal();
        if let Some(socket) = &rcd_socket {
            spawn_reaper(socket);
        }
        hook(info);
        // A background task that died leaves counters that never settle and a
        // screen already torn down: there is no carrying on from here.
        std::process::exit(101);
    }));
}

/// Events handled between two frames at most, so a flood cannot starve drawing.
const MAX_BATCH: usize = 256;

fn handle_terminal_event(app: &mut App, ev: Event) {
    match ev {
        Event::Key(key) if key.kind == KeyEventKind::Press => app.handle_key(key),
        Event::Paste(text) => app.handle_paste(&text),
        // A resize needs nothing but the redraw every event is followed by.
        _ => {}
    }
}

async fn event_loop(terminal: &mut Term, app: &mut App) -> color_eyre::Result<()> {
    let mut events = EventStream::new();
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    // Fast frames only while something animates (spinner, loading).
    let mut frame = tokio::time::interval(Duration::from_millis(120));
    // Ticks nobody waited for are dropped, not made up for: by default the frames
    // missed while nothing animated (8 a second, for as long as the app sat idle)
    // would all fire at once at the next spinner, each one redrawing the screen.
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    frame.set_missed_tick_behavior(MissedTickBehavior::Skip);
    // A closed terminal window, a dropped SSH session, a `kill`: leave in order.
    let mut hangup = signal(SignalKind::hangup())?;
    let mut terminate = signal(SignalKind::terminate())?;
    let mut interrupt = signal(SignalKind::interrupt())?;
    loop {
        if let Some(text) = app.clipboard_out.take() {
            use std::io::Write;
            let mut out = std::io::stdout();
            let _ = out.write_all(ratclone::clipboard::osc52(&text).as_bytes());
            let _ = out.flush();
        }
        terminal.draw(|f| ratclone::ui::draw(f, app))?;
        if app.should_quit {
            return Ok(());
        }
        tokio::select! {
            ev = events.next() => match ev {
                Some(Ok(ev)) => handle_terminal_event(app, ev),
                Some(Err(e)) => return Err(e.into()),
                None => return Ok(()),
            },
            Some(ev) = app.next_event() => app.handle_event(ev),
            _ = tick.tick() => app.handle_event(AppEvent::Tick),
            _ = frame.tick(), if app.is_animating() => app.spinner = app.spinner.wrapping_add(1),
            _ = async {
                tokio::select! {
                    _ = hangup.recv() => {}
                    _ = terminate.recv() => {}
                    _ = interrupt.recv() => {}
                }
            } => {
                app.quit_on_signal();
                return Ok(());
            }
        }
        // Take in whatever else is already waiting (held-down keys, a burst of
        // progress answers) and draw once for all of it.
        for _ in 0..MAX_BATCH {
            if app.should_quit {
                break;
            }
            if let Some(ev) = app.try_next_event() {
                app.handle_event(ev);
                continue;
            }
            // Poll with this task's own waker: crossterm keeps the waker of the
            // poll that found nothing and wakes only that one at the next key.
            // `now_or_never` would hand it a no-op waker, and keys would then
            // wait for the next tick (up to a second) to be seen.
            match futures::poll!(events.next()) {
                Poll::Ready(Some(Ok(ev))) => handle_terminal_event(app, ev),
                Poll::Ready(Some(Err(e))) => return Err(e.into()),
                Poll::Ready(None) => return Ok(()),
                Poll::Pending => break,
            }
        }
    }
}
