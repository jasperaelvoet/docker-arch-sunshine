pub mod compositor;
pub mod control;
pub mod dbus_bus;
pub mod env;
pub mod fs_setup;
pub mod input_devices;
pub mod kde_config;
pub mod pipewire;
pub mod process;
pub mod session_actions;

use anyhow::{anyhow, Result};
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::process::Child;
use tokio::sync::Mutex;

use crate::env_config::{
    default_audio_channels, default_geometry, resolve_session_scale, sanitize_audio_channels,
    sanitize_positive_int, sanitize_refresh_rate,
};
use crate::paths::{
    control_fifo_path, input_fps_state_path, DEFAULT_FPS, DEFAULT_HEIGHT, DEFAULT_RUNTIME_DIR,
    DEFAULT_SOCKET, DEFAULT_WIDTH,
};

#[derive(Debug, Clone)]
pub struct DesktopArgs {
    pub runtime_dir: PathBuf,
    pub socket: String,
    pub width: u32,
    pub height: u32,
    pub fps: String,
    pub scale: String,
    pub ready_timeout: f64,
    pub smoke_seconds: f64,
    pub no_audio: bool,
    pub no_sunshine: bool,
    pub xwayland: bool,
}

impl DesktopArgs {
    pub fn from_inputs(
        runtime_dir: Option<String>,
        socket: Option<String>,
        width: Option<String>,
        height: Option<String>,
        fps: Option<String>,
        scale: Option<String>,
        ready_timeout: Option<f64>,
        smoke_seconds: Option<f64>,
        no_audio: bool,
        no_sunshine: bool,
        xwayland: bool,
    ) -> Self {
        let (dw, dh, df, ds) = default_geometry();
        let width = sanitize_positive_int(
            &width.unwrap_or_else(|| dw.to_string()),
            DEFAULT_WIDTH,
            8192,
        );
        let height = sanitize_positive_int(
            &height.unwrap_or_else(|| dh.to_string()),
            DEFAULT_HEIGHT,
            8192,
        );
        let fps = sanitize_refresh_rate(&fps.unwrap_or(df), DEFAULT_FPS);
        let scale_input = scale.unwrap_or(ds);
        let scale = resolve_session_scale(width, height, &scale_input);
        Self {
            runtime_dir: runtime_dir
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(DEFAULT_RUNTIME_DIR)),
            socket: socket.unwrap_or_else(|| DEFAULT_SOCKET.into()),
            width,
            height,
            fps,
            scale,
            ready_timeout: ready_timeout.unwrap_or(30.0),
            smoke_seconds: smoke_seconds.unwrap_or(0.0),
            no_audio,
            no_sunshine,
            xwayland,
        }
    }
}

#[derive(Debug, Clone)]
pub struct StreamGeometry {
    pub width: u32,
    pub height: u32,
    pub fps: String,
    pub scale: String,
}

#[derive(Default)]
pub struct DesktopProcesses {
    pub network_status: Option<Child>,
    pub session_actions: Option<Child>,
    pub pipewire: Option<Child>,
    pub wireplumber: Option<Child>,
    pub pulse: Option<Child>,
    pub kwin: Option<Child>,
    pub kded: Option<Child>,
    pub activities: Option<Child>,
    pub plasmashell: Option<Child>,
    pub portal_backend: Option<Child>,
    pub portal: Option<Child>,
    pub sunshine: Option<Child>,
}

#[derive(Clone)]
pub struct DesktopState {
    pub args: Arc<DesktopArgs>,
    pub geometry: Arc<Mutex<StreamGeometry>>,
    pub audio_channels: Arc<Mutex<u32>>,
    pub audio_generation: Arc<AtomicU64>,
    pub compositor_env: Arc<BTreeMap<String, String>>,
    pub desktop_env: Arc<BTreeMap<String, String>>,
    pub processes: Arc<Mutex<DesktopProcesses>>,
}

pub async fn start(args: DesktopArgs) -> Result<(DesktopState, ShutdownGuard)> {
    let runtime_dir = args.runtime_dir.clone();
    fs_setup::prepare_desktop_filesystem(&runtime_dir)?;
    process::start_child_reaper();
    input_devices::start_input_device_node_watcher();
    fs_setup::write_wayland_desktop_config()?;
    process::kill_desktop_processes();

    let socket_path = runtime_dir.join(&args.socket);
    let socket_lock = runtime_dir.join(format!("{}.lock", args.socket));
    for stale in [
        socket_path.clone(),
        socket_lock.clone(),
        control_fifo_path(Some(&runtime_dir)),
    ] {
        let _ = fs::remove_file(stale);
    }

    let session = dbus_bus::start_session_bus(&runtime_dir)?;
    let system_pid = dbus_bus::start_system_bus()?;
    let compositor_env = Arc::new(env::desktop_base_env(
        &runtime_dir,
        Some(&session.address),
        None,
    ));
    let desktop_env = Arc::new(env::desktop_base_env(
        &runtime_dir,
        Some(&session.address),
        Some(&args.socket),
    ));

    let control_path = control_fifo_path(Some(&runtime_dir));
    let control = control::ControlChannel::open(control_path)?;

    let mut processes = DesktopProcesses::default();
    processes.network_status = start_network_status();

    let mut session_actions = process::spawn_as_desktop_user(
        &["/usr/local/bin/arch-sunshine", "session-actions"],
        &desktop_env,
        "session-actions.log",
    )?;
    if !process::wait_for_dbus_name(
        "org.kde.Shutdown",
        &desktop_env,
        Some(&mut session_actions),
        Duration::from_secs_f64(args.ready_timeout),
    )
    .await
    {
        return Err(anyhow!("timed out waiting for session action service"));
    }
    if !process::wait_for_dbus_name(
        "org.kde.LogoutPrompt",
        &desktop_env,
        Some(&mut session_actions),
        Duration::from_secs_f64(args.ready_timeout),
    )
    .await
    {
        return Err(anyhow!("timed out waiting for logout prompt service"));
    }
    processes.session_actions = Some(session_actions);

    if !args.no_audio {
        let stack = pipewire::start_pipewire_stack(&runtime_dir, &desktop_env).await?;
        processes.pipewire = stack.pipewire;
        processes.wireplumber = stack.wireplumber;
        processes.pulse = stack.pulse;
    }

    let geometry = Arc::new(Mutex::new(StreamGeometry {
        width: args.width,
        height: args.height,
        fps: args.fps.clone(),
        scale: args.scale.clone(),
    }));
    write_input_fps_state(&runtime_dir, &args.fps);

    compositor::start_desktop_stack(
        &args,
        &runtime_dir,
        &socket_path,
        &compositor_env,
        &desktop_env,
        &geometry.lock().await.clone(),
        &mut processes,
    )
    .await?;

    if !args.no_sunshine {
        let mut sunshine = process::spawn_with_env(
            &["/usr/local/bin/arch-sunshine-server", "serve"],
            &desktop_env,
            "sunshine-wrapper.log",
        )?;
        if !process::wait_for_tcp_port(
            "127.0.0.1",
            47989,
            Some(&mut sunshine),
            Duration::from_secs_f64(args.ready_timeout),
        )
        .await
        {
            return Err(anyhow!("timed out waiting for Sunshine on TCP port 47989"));
        }
        processes.sunshine = Some(sunshine);
    }

    eprintln!(
        "arch-sunshine: Plasma Wayland desktop ready on {}",
        socket_path.display()
    );

    let state = DesktopState {
        args: Arc::new(args),
        geometry,
        audio_channels: Arc::new(Mutex::new(default_audio_channels())),
        audio_generation: Arc::new(AtomicU64::new(0)),
        compositor_env,
        desktop_env,
        processes: Arc::new(Mutex::new(processes)),
    };

    let guard = ShutdownGuard {
        session_dbus_pid: if session.pid.is_empty() {
            None
        } else {
            Some(session.pid)
        },
        system_dbus_pid: if system_pid.is_empty() {
            None
        } else {
            Some(system_pid)
        },
        state: state.clone(),
        control: Arc::new(Mutex::new(Some(control))),
    };
    Ok((state, guard))
}

pub struct ShutdownGuard {
    pub session_dbus_pid: Option<String>,
    pub system_dbus_pid: Option<String>,
    pub state: DesktopState,
    pub control: Arc<Mutex<Option<control::ControlChannel>>>,
}

impl ShutdownGuard {
    pub async fn shutdown(self) {
        let mut control_lock = self.control.lock().await;
        *control_lock = None;
        drop(control_lock);

        let mut procs = self.state.processes.lock().await;
        process::terminate(&mut procs.sunshine).await;
        process::terminate(&mut procs.portal).await;
        process::terminate(&mut procs.portal_backend).await;
        process::terminate(&mut procs.plasmashell).await;
        process::terminate(&mut procs.activities).await;
        process::terminate(&mut procs.kded).await;
        process::terminate(&mut procs.kwin).await;
        process::terminate(&mut procs.pulse).await;
        process::terminate(&mut procs.wireplumber).await;
        process::terminate(&mut procs.pipewire).await;
        process::terminate(&mut procs.session_actions).await;
        process::terminate(&mut procs.network_status).await;
        drop(procs);

        if let Some(pid) = self.session_dbus_pid {
            let _ = process::run_quiet(&["kill", &pid]);
        }
        if let Some(pid) = self.system_dbus_pid {
            let _ = process::run_quiet(&["kill", &pid]);
        }
        process::kill_desktop_processes();
    }
}

fn start_network_status() -> Option<Child> {
    let bin = std::path::Path::new("/usr/local/bin/arch-sunshine-network-status");
    if !bin.exists() {
        return None;
    }
    let bin = bin.to_str()?;
    process::spawn_with_env(&[bin], &BTreeMap::new(), "network-status.log").ok()
}

pub async fn handle_control(state: &DesktopState, msg: control::ControlMessage) -> Result<()> {
    let socket_path = state.args.runtime_dir.join(&state.args.socket);
    match msg {
        control::ControlMessage::ClientStart {
            width,
            height,
            fps,
            scale,
            audio_channels,
            ack,
        } => {
            let fallback = default_audio_channels();
            let requested = audio_channels
                .map(|c| c.to_string())
                .unwrap_or_else(|| fallback.to_string());
            let channels = sanitize_audio_channels(&requested, fallback);
            let mut current_channels = state.audio_channels.lock().await;
            if *current_channels != channels {
                eprintln!(
                    "arch-sunshine: Moonlight client requested {}-channel audio",
                    channels
                );
            }
            *current_channels = channels;
            drop(current_channels);
            let generation = state.audio_generation.fetch_add(1, Ordering::SeqCst) + 1;
            if !state.args.no_audio {
                pipewire::ensure_audio_sink(&state.desktop_env, Some(channels))?;
            }

            let fps_text = match fps {
                serde_json::Value::String(s) => s,
                serde_json::Value::Number(n) => n.to_string(),
                other => other.to_string(),
            };
            let scale_text = match scale {
                serde_json::Value::String(s) => s,
                serde_json::Value::Number(n) => n.to_string(),
                other => other.to_string(),
            };

            restart_geometry(
                state,
                &socket_path,
                width,
                height,
                fps_text,
                scale_text,
                "Moonlight client requested geometry",
            )
            .await?;

            if let Some(ack_path) = ack {
                let path = PathBuf::from(ack_path);
                if let Some(parent) = path.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                let _ = fs::write(&path, "ok\n");
                process::chown_path(&path, false);
            }
            schedule_audio_sink_cleanup(state, channels, generation);
        }
        control::ControlMessage::ClientStop { .. } => {
            let generation = state.audio_generation.fetch_add(1, Ordering::SeqCst) + 1;
            let channels = *state.audio_channels.lock().await;
            if !state.args.no_audio {
                pipewire::ensure_audio_sink(&state.desktop_env, Some(channels))?;
            }
            schedule_audio_sink_cleanup(state, channels, generation);
            let (width, height, fps, scale) = default_geometry();
            restart_geometry(
                state,
                &socket_path,
                width,
                height,
                fps,
                scale,
                "Moonlight client disconnected",
            )
            .await?;
        }
    }
    Ok(())
}

fn schedule_audio_sink_cleanup(state: &DesktopState, channels: u32, generation: u64) {
    if state.args.no_audio {
        return;
    }
    let env = state.desktop_env.clone();
    let current_generation = state.audio_generation.clone();
    tokio::spawn(async move {
        for delay in [
            Duration::from_millis(750),
            Duration::from_secs(2),
            Duration::from_secs(5),
        ] {
            tokio::time::sleep(delay).await;
            // A newer client start/stop owns the sink state now.
            if current_generation.load(Ordering::SeqCst) != generation {
                return;
            }
            let env = env.clone();
            let _ = tokio::task::spawn_blocking(move || {
                if let Err(e) = pipewire::ensure_audio_sink(&env, Some(channels)) {
                    eprintln!("arch-sunshine: delayed audio sink cleanup failed: {e}");
                }
            })
            .await;
        }
    });
}

async fn restart_geometry(
    state: &DesktopState,
    socket_path: &std::path::Path,
    width: u32,
    height: u32,
    fps: String,
    scale: String,
    reason: &str,
) -> Result<()> {
    let fps = sanitize_refresh_rate(&fps, DEFAULT_FPS);
    write_input_fps_state(&state.args.runtime_dir, &fps);

    let mut geometry = state.geometry.lock().await;
    let kwin_alive = state
        .processes
        .lock()
        .await
        .kwin
        .as_mut()
        .map(|c| c.try_wait().ok().flatten().is_none())
        .unwrap_or(false);
    if kwin_alive && geometry.width == width && geometry.height == height && geometry.scale == scale
    {
        geometry.fps = fps;
        return Ok(());
    }

    let (logical_w, logical_h) = crate::env_config::kwin_virtual_geometry(width, height, &scale);
    eprintln!(
        "arch-sunshine: {reason}; switching stream to {width}x{height}@{fps} scale {scale} (KWin {logical_w}x{logical_h} logical)"
    );
    drop(geometry);

    {
        let mut procs = state.processes.lock().await;
        compositor::stop_desktop_stack(&mut procs).await;
    }

    *state.geometry.lock().await = StreamGeometry {
        width,
        height,
        fps: fps.clone(),
        scale: scale.clone(),
    };

    let geometry_now = state.geometry.lock().await.clone();
    let mut procs = state.processes.lock().await;
    compositor::start_desktop_stack(
        &state.args,
        &state.args.runtime_dir,
        socket_path,
        &state.compositor_env,
        &state.desktop_env,
        &geometry_now,
        &mut procs,
    )
    .await?;
    Ok(())
}

fn write_input_fps_state(runtime_dir: &std::path::Path, fps: &str) {
    let path = input_fps_state_path(runtime_dir);
    if fs::write(&path, format!("{fps}\n")).is_ok() {
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o644));
        process::chown_path(&path, false);
    }
}
