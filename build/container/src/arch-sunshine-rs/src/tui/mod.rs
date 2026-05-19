pub mod keys;
pub mod log_tail;
pub mod modal;
pub mod shell;
pub mod state;
pub mod ui;

use anyhow::Result;
use crossterm::event::{Event, EventStream};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use futures::StreamExt;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io::{stdout, Stdout, Write};
use std::os::fd::IntoRawFd;
use std::time::Duration;
use tokio::sync::mpsc;

use crate::desktop::{handle_control, DesktopState, ShutdownGuard};
use crate::encoder::{chosen_encoder_summary, load_config};
use crate::env_config::kwin_virtual_geometry;
use crate::tui::keys::KeyAction;
use crate::tui::state::{AppState, Health, LogSource, Modal, StatusSnapshot};

fn enter_alt_screen() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(out);
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

fn leave_alt_screen() -> Result<()> {
    disable_raw_mode()?;
    execute!(stdout(), LeaveAlternateScreen)?;
    Ok(())
}

/// While the TUI owns the alternate screen, anything that writes to stderr
/// (e.g. `eprintln!` in the desktop control path) would bleed through the
/// rendered frame and corrupt the layout. We redirect stderr to the wrapper
/// log file for the lifetime of the TUI so those messages show up in the
/// Wrapper panel instead.
struct StderrRedirect {
    saved_fd: libc::c_int,
}

impl StderrRedirect {
    fn install() -> Option<Self> {
        let path = LogSource::Wrapper.path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .ok()?;
        let _ = std::io::stderr().flush();
        let saved = unsafe { libc::dup(libc::STDERR_FILENO) };
        if saved < 0 {
            return None;
        }
        let fd = file.into_raw_fd();
        let rc = unsafe { libc::dup2(fd, libc::STDERR_FILENO) };
        unsafe { libc::close(fd) };
        if rc < 0 {
            unsafe { libc::close(saved) };
            return None;
        }
        Some(Self { saved_fd: saved })
    }
}

impl Drop for StderrRedirect {
    fn drop(&mut self) {
        let _ = std::io::stderr().flush();
        unsafe {
            libc::dup2(self.saved_fd, libc::STDERR_FILENO);
            libc::close(self.saved_fd);
        }
    }
}

pub async fn run(state: DesktopState, guard: ShutdownGuard) -> Result<()> {
    let _stderr_redirect = StderrRedirect::install();
    let mut terminal = enter_alt_screen()?;
    let result = run_loop(&mut terminal, &state, &guard).await;
    leave_alt_screen().ok();
    guard.shutdown().await;
    result
}

async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    state: &DesktopState,
    guard: &ShutdownGuard,
) -> Result<()> {
    let mut app = AppState::new();
    let (log_tx, mut log_rx) = mpsc::unbounded_channel();
    log_tail::spawn(log_tx)?;

    let mut events = EventStream::new();
    let mut fast_ticker = tokio::time::interval(Duration::from_millis(500));
    fast_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut slow_ticker = tokio::time::interval(Duration::from_secs(10));
    slow_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut control_ticker = tokio::time::interval(Duration::from_millis(250));
    control_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut shell_ticker = tokio::time::interval(Duration::from_millis(33));
    shell_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    // Initial poll so the first frame isn't empty. Slow fields are filled here
    // and then only refreshed on the slow ticker.
    app.status = build_fast_status(state).await;
    let (audio, encoders) = build_slow_status(state).await;
    app.status.audio_sink = audio;
    app.status.encoder_summary = encoders;

    loop {
        let mut last_shell_area: Option<ratatui::layout::Rect> = None;
        terminal.draw(|f| {
            last_shell_area = ui::draw(f, &app);
        })?;

        if app.should_quit {
            break;
        }
        if app.should_shell {
            app.should_shell = false;
            if app.shell.is_none() {
                let env = (*state.desktop_env).clone();
                let (rows, cols) = last_shell_area
                    .map(|r| (r.height.max(1), r.width.max(1)))
                    .unwrap_or((24, 80));
                match shell::ShellSession::spawn(rows, cols, &env) {
                    Ok(s) => {
                        app.shell = Some(s);
                        app.modal = Modal::Shell;
                    }
                    Err(e) => eprintln!("arch-sunshine: shell failed to start: {e}"),
                }
            }
        }
        if app.modal == Modal::Shell {
            if let Some(area) = last_shell_area {
                if let Some(shell) = app.shell.as_mut() {
                    shell.resize(area.height.max(1), area.width.max(1));
                    if !shell.is_alive() {
                        app.shell = None;
                        app.modal = Modal::None;
                    }
                }
            }
        }

        tokio::select! {
            biased;
            maybe_line = log_rx.recv() => {
                if let Some(entry) = maybe_line {
                    let focused = app.focused_log;
                    if let Some(buf) = app.logs.get_mut(&entry.source) {
                        buf.push(entry.line, entry.source == focused);
                    }
                }
            }
            maybe_event = events.next() => {
                if let Some(Ok(event)) = maybe_event {
                    if let Event::Key(key) = event {
                        let action = keys::handle(&mut app, key);
                        if let KeyAction::SubmitPin = action {
                            match modal::submit_pin(&app.pin_input) {
                                Ok(()) => {
                                    app.modal = Modal::None;
                                    app.pin_input.clear();
                                    app.pin_message.clear();
                                }
                                Err(e) => {
                                    app.pin_message = e.to_string();
                                }
                            }
                        }
                    }
                }
            }
            _ = fast_ticker.tick() => {
                let next = build_fast_status(state).await;
                let audio = std::mem::take(&mut app.status.audio_sink);
                let encoders = std::mem::take(&mut app.status.encoder_summary);
                app.status = next;
                app.status.audio_sink = audio;
                app.status.encoder_summary = encoders;
            }
            _ = slow_ticker.tick() => {
                let (audio, encoders) = build_slow_status(state).await;
                app.status.audio_sink = audio;
                app.status.encoder_summary = encoders;
            }
            _ = control_ticker.tick() => {
                pump_control(state, guard).await;
            }
            _ = shell_ticker.tick(), if app.modal == Modal::Shell => {
                // wakes the loop so the embedded shell redraws PTY output;
                // when no shell is up, the guard suppresses these ticks.
            }
        }
    }
    Ok(())
}

async fn pump_control(state: &DesktopState, guard: &ShutdownGuard) {
    let mut control_lock = guard.control.lock().await;
    let Some(channel) = control_lock.as_mut() else { return };
    let messages = channel.read_messages().await;
    drop(control_lock);
    for msg in messages {
        if let Err(e) = handle_control(state, msg).await {
            eprintln!("arch-sunshine: control error: {e}");
        }
    }
}

async fn build_fast_status(state: &DesktopState) -> StatusSnapshot {
    let runtime_dir = state.args.runtime_dir.clone();
    let socket_path = runtime_dir.join(&state.args.socket);
    let pipewire_socket = runtime_dir.join("pipewire-0").exists();
    let pulse_socket = runtime_dir.join("pulse").join("native").exists();
    let wayland_socket = socket_path.exists();
    let geometry = state.geometry.lock().await.clone();
    let (logical_w, logical_h) =
        kwin_virtual_geometry(geometry.width, geometry.height, &geometry.scale);

    let mut procs = state.processes.lock().await;
    let kwin = child_health(&mut procs.kwin);
    let plasmashell = child_health(&mut procs.plasmashell);
    let sunshine = child_health(&mut procs.sunshine);
    drop(procs);

    StatusSnapshot {
        kwin,
        plasmashell,
        sunshine,
        wayland_socket,
        pipewire_socket,
        pulse_socket,
        audio_sink: String::new(),
        render_nodes: render_nodes_summary(),
        encoder_summary: Vec::new(),
        stream_width: geometry.width,
        stream_height: geometry.height,
        stream_fps: geometry.fps,
        stream_scale: geometry.scale,
        logical_width: logical_w,
        logical_height: logical_h,
    }
}

async fn build_slow_status(state: &DesktopState) -> (String, Vec<(String, Option<String>)>) {
    let env = (*state.desktop_env).clone();
    let audio_sink = tokio::task::spawn_blocking(move || {
        crate::desktop::pipewire::audio_status_summary(&env)
    })
    .await
    .unwrap_or_else(|_| "?".into());
    let encoders = compute_encoder_summary().await;
    (audio_sink, encoders)
}

fn child_health(child: &mut Option<tokio::process::Child>) -> Health {
    let Some(c) = child.as_mut() else { return Health::Unknown };
    match c.try_wait() {
        Ok(None) => Health::Up,
        Ok(Some(_)) => Health::Down,
        Err(_) => Health::Unknown,
    }
}

fn render_nodes_summary() -> String {
    let Ok(rd) = std::fs::read_dir("/dev/dri") else {
        return "none".into();
    };
    let mut nodes: Vec<String> = rd
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with("renderD") {
                Some(format!("/dev/dri/{name}"))
            } else {
                None
            }
        })
        .collect();
    nodes.sort();
    if nodes.is_empty() {
        "none".into()
    } else {
        nodes.join(" ")
    }
}

async fn compute_encoder_summary() -> Vec<(String, Option<String>)> {
    tokio::task::spawn_blocking(|| {
        let config = match load_config(std::path::Path::new(crate::paths::DEFAULT_CONFIG)) {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };
        chosen_encoder_summary(&config)
            .into_iter()
            .map(|s| (s.codec, s.chosen))
            .collect()
    })
    .await
    .unwrap_or_default()
}
