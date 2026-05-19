mod desktop;
mod encoder;
mod env_config;
mod paths;
mod sunshine_api;
mod tui;

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use std::collections::BTreeMap;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::desktop::control::send_control_message_sync;
use crate::desktop::session_actions::Credentials;
use crate::encoder::{
    build_pipeline, encoder_status, hardware_probe, load_config, PipelineConfig, PipelineRequest,
};
use crate::env_config::{
    default_audio_channels, requested_audio_channels, requested_client_geometry,
};
use crate::paths::{control_fifo_path, DEFAULT_CONFIG, DEFAULT_RUNTIME_DIR, DEFAULT_SOCKET};

#[derive(Parser)]
#[command(
    name = "arch-sunshine",
    about = "Arch Sunshine desktop runtime with a pinned Sunshine streaming server.",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Inspect available GStreamer encoders.
    Probe {
        #[arg(long, env = "SUNSHINE_CONFIG", default_value = DEFAULT_CONFIG)]
        config: PathBuf,
    },
    /// Print a GStreamer RTP pipeline.
    Pipeline {
        #[command(flatten)]
        opts: PipelineArgs,
        #[arg(long)]
        json: bool,
    },
    /// Run a full KDE Plasma desktop on the Wayland/GStreamer stack.
    DesktopSession(DesktopSessionArgs),
    /// (internal) Stage a Moonlight client start.
    #[command(name = "client-start", hide = true)]
    ClientStart,
    /// (internal) Stage a Moonlight client stop.
    #[command(name = "client-stop", hide = true)]
    ClientStop,
    /// Close the active Moonlight stream.
    Disconnect {
        #[arg(long, env = "SUNSHINE_WEB_UI_USER", default_value = "sunshine")]
        user: String,
        #[arg(long, env = "SUNSHINE_WEB_UI_PASS", default_value = "sunshine")]
        password: String,
    },
    /// (internal) KDE D-Bus shutdown/logout proxy.
    #[command(name = "session-actions", hide = true)]
    SessionActions {
        #[arg(long, env = "SUNSHINE_WEB_UI_USER", default_value = "sunshine")]
        user: String,
        #[arg(long, env = "SUNSHINE_WEB_UI_PASS", default_value = "sunshine")]
        password: String,
    },
}

#[derive(Args)]
struct PipelineArgs {
    #[arg(long, env = "SUNSHINE_CONFIG", default_value = DEFAULT_CONFIG)]
    config: PathBuf,
    #[arg(long, value_parser = ["h264", "hevc", "av1"], default_value = "h264")]
    codec: String,
    #[arg(long, default_value = "")]
    encoder: String,
    #[arg(long, value_parser = ["test", "pipewire"], default_value = "test")]
    source: String,
    #[arg(
        long = "pipewire-node",
        env = "SUNSHINE_PIPEWIRE_NODE",
        default_value = "0"
    )]
    pipewire_node: String,
    #[arg(long, env = "SUNSHINE_WIDTH", default_value = "1920")]
    width: u32,
    #[arg(long, env = "SUNSHINE_HEIGHT", default_value = "1080")]
    height: u32,
    #[arg(long, env = "SUNSHINE_FPS", default_value = "60")]
    fps: u32,
    #[arg(long, env = "SUNSHINE_BITRATE_KBPS", default_value = "25000")]
    bitrate: u32,
    #[arg(
        long = "vbv-buffer",
        env = "SUNSHINE_VBV_KBITS",
        default_value = "25000"
    )]
    vbv_buffer: u32,
    #[arg(
        long = "payload-mtu",
        env = "SUNSHINE_PAYLOAD_MTU",
        default_value = "1200"
    )]
    payload_mtu: u32,
    #[arg(long, env = "SUNSHINE_UDP_HOST", default_value = "127.0.0.1")]
    host: String,
    #[arg(long, env = "SUNSHINE_UDP_PORT", default_value = "5004")]
    port: u16,
}

#[derive(Args, Default, Clone)]
struct DesktopSessionArgs {
    #[arg(long = "runtime-dir", env = "XDG_RUNTIME_DIR", default_value = DEFAULT_RUNTIME_DIR)]
    runtime_dir: String,
    #[arg(long, env = "SUNSHINE_WAYLAND_SOCKET", default_value = DEFAULT_SOCKET)]
    socket: String,
    #[arg(long)]
    width: Option<String>,
    #[arg(long)]
    height: Option<String>,
    #[arg(long)]
    fps: Option<String>,
    #[arg(long)]
    scale: Option<String>,
    #[arg(
        long = "ready-timeout",
        env = "SUNSHINE_READY_TIMEOUT",
        default_value = "30"
    )]
    ready_timeout: f64,
    #[arg(long = "smoke-seconds", default_value = "0")]
    smoke_seconds: f64,
    #[arg(long = "no-audio", default_value_t = false)]
    no_audio: bool,
    #[arg(long = "no-sunshine", default_value_t = false)]
    no_sunshine: bool,
    #[arg(long = "no-xwayland", default_value_t = false)]
    no_xwayland: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async move { dispatch(cli.command).await })
}

async fn dispatch(command: Command) -> Result<()> {
    match command {
        Command::Probe { config } => probe(&config),
        Command::Pipeline { opts, json } => pipeline(&opts, json),
        Command::DesktopSession(args) => desktop_session(args).await,
        Command::ClientStart => client_start(),
        Command::ClientStop => client_stop(),
        Command::Disconnect { user, password } => disconnect(&user, &password),
        Command::SessionActions { user, password } => {
            desktop::session_actions::run(Credentials { user, password }).await
        }
    }
}

fn probe(config_path: &std::path::Path) -> Result<()> {
    let config = load_config(config_path)?;
    let mut payload = BTreeMap::new();
    payload.insert(
        "gst_inspect".to_string(),
        serde_json::Value::Bool(which::which("gst-inspect-1.0")),
    );
    payload.insert(
        "gst_launch".to_string(),
        serde_json::Value::Bool(which::which("gst-launch-1.0")),
    );
    payload.insert(
        "weston".to_string(),
        serde_json::Value::Bool(which::which("weston")),
    );
    payload.insert(
        "vainfo".to_string(),
        serde_json::Value::Bool(which::which("vainfo")),
    );
    payload.insert(
        "vulkaninfo".to_string(),
        serde_json::Value::Bool(which::which("vulkaninfo")),
    );
    payload.insert(
        "hardware".to_string(),
        serde_json::to_value(hardware_probe())?,
    );
    payload.insert(
        "encoders".to_string(),
        serde_json::to_value(encoder_status(&config))?,
    );
    let rendered = serde_json::to_string_pretty(&payload)?;
    println!("{rendered}");
    Ok(())
}

mod which {
    pub fn which(name: &str) -> bool {
        std::env::var_os("PATH")
            .map(|path| std::env::split_paths(&path).any(|dir| dir.join(name).is_file()))
            .unwrap_or(false)
    }
}

fn pipeline(args: &PipelineArgs, json: bool) -> Result<()> {
    let config: PipelineConfig = load_config(&args.config)?;
    let request = PipelineRequest {
        codec: &args.codec,
        encoder: if args.encoder.is_empty() {
            None
        } else {
            Some(args.encoder.as_str())
        },
        source: &args.source,
        pipewire_node: &args.pipewire_node,
        width: args.width,
        height: args.height,
        fps: args.fps,
        bitrate: args.bitrate,
        vbv_buffer: args.vbv_buffer,
        payload_mtu: args.payload_mtu,
        host: &args.host,
        port: args.port,
    };
    let (encoder_name, pipeline) = build_pipeline(&request, &config)?;
    if json {
        let value = serde_json::json!({
            "encoder": encoder_name,
            "pipeline": pipeline,
        });
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        println!("{pipeline}");
    }
    Ok(())
}

fn client_start() -> Result<()> {
    let (width, height, fps, scale) = requested_client_geometry();
    let audio_channels = requested_audio_channels();
    let pid = std::process::id();
    let path = control_fifo_path(None);
    let ack_dir = path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("/tmp"));
    let ack_path = ack_dir.join(format!("arch-sunshine-client-start-{pid}.done"));
    let _ = std::fs::remove_file(&ack_path);
    let message = serde_json::json!({
        "command": "client-start",
        "width": width,
        "height": height,
        "fps": fps,
        "scale": scale,
        "audio_channels": audio_channels,
        "ack": ack_path.to_string_lossy(),
    });
    send_control_message_sync(&path, &message)?;
    let timeout = std::env::var("SUNSHINE_CLIENT_READY_TIMEOUT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(30.0);
    let deadline = Instant::now() + Duration::from_secs_f64(timeout);
    while Instant::now() < deadline {
        if ack_path.exists() {
            let _ = std::fs::remove_file(&ack_path);
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    anyhow::bail!("timed out waiting for desktop stream setup")
}

fn client_stop() -> Result<()> {
    let message = serde_json::json!({
        "command": "client-stop",
        "audio_channels": default_audio_channels(),
    });
    let path = control_fifo_path(None);
    if let Err(e) = send_control_message_sync(&path, &message) {
        eprintln!("arch-sunshine: {e}");
    }
    Ok(())
}

fn disconnect(user: &str, password: &str) -> Result<()> {
    sunshine_api::disconnect(user, password)?;
    client_stop()
}

async fn desktop_session(args: DesktopSessionArgs) -> Result<()> {
    let desktop_args = desktop::DesktopArgs::from_inputs(
        Some(args.runtime_dir),
        Some(args.socket),
        args.width,
        args.height,
        args.fps,
        args.scale,
        Some(args.ready_timeout),
        Some(args.smoke_seconds),
        args.no_audio,
        args.no_sunshine,
        !args.no_xwayland,
    );
    let smoke = desktop_args.smoke_seconds;
    let (state, guard) = desktop::start(desktop_args)
        .await
        .context("starting desktop session")?;

    if smoke > 0.0 {
        tokio::time::sleep(Duration::from_secs_f64(smoke)).await;
        guard.shutdown().await;
        return Ok(());
    }

    if std::io::stdin().is_terminal() {
        tui::run(state, guard).await?;
    } else {
        run_headless(state, guard).await?;
    }
    Ok(())
}

async fn run_headless(state: desktop::DesktopState, guard: desktop::ShutdownGuard) -> Result<()> {
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    let mut ticker = tokio::time::interval(Duration::from_millis(250));
    loop {
        tokio::select! {
            _ = sigterm.recv() => break,
            _ = sigint.recv() => break,
            _ = ticker.tick() => {
                let mut control_lock = guard.control.lock().await;
                if let Some(ch) = control_lock.as_mut() {
                    let messages = ch.read_messages().await;
                    drop(control_lock);
                    for msg in messages {
                        if let Err(e) = desktop::handle_control(&state, msg).await {
                            eprintln!("arch-sunshine: control error: {e}");
                        }
                    }
                }
            }
        }
    }
    guard.shutdown().await;
    Ok(())
}
