use anyhow::{anyhow, Result};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use std::time::Duration;
use tokio::process::Child;

use crate::desktop::process::{
    command_available, open_log_for_append, run_as_desktop_user, run_as_desktop_user_quiet,
    spawn_as_desktop_user, terminate, wait_for_path,
};
use crate::env_config::{default_audio_channels, sanitize_audio_channels};
use crate::paths::{audio_channel_map, log_dir, AUDIO_SINK_DESCRIPTION, AUDIO_SINK_NAME};

pub struct PipewireStack {
    pub pipewire: Option<Child>,
    pub wireplumber: Option<Child>,
    pub pulse: Option<Child>,
}

pub async fn start_pipewire_stack(
    runtime_dir: &Path,
    env: &BTreeMap<String, String>,
) -> Result<PipewireStack> {
    let mut pipewire = Some(spawn_as_desktop_user(&["pipewire"], env, "pipewire.log")?);
    if !wait_for_path(
        &runtime_dir.join("pipewire-0"),
        pipewire.as_mut(),
        Duration::from_secs(10),
    )
    .await
    {
        let exit = exit_summary(pipewire.as_mut());
        terminate(&mut pipewire).await;
        return Err(anyhow!(
            "PipeWire never created {}/pipewire-0 ({}); see {}",
            runtime_dir.display(),
            exit,
            log_dir().join("pipewire.log").display(),
        ));
    }

    let wireplumber = Some(spawn_as_desktop_user(
        &["wireplumber"],
        env,
        "wireplumber.log",
    )?);
    let mut pulse = Some(spawn_as_desktop_user(
        &["pipewire-pulse"],
        env,
        "pipewire-pulse.log",
    )?);
    if !wait_for_path(
        &runtime_dir.join("pulse").join("native"),
        pulse.as_mut(),
        Duration::from_secs(10),
    )
    .await
    {
        let exit = exit_summary(pulse.as_mut());
        let mut p = pulse;
        terminate(&mut p).await;
        let mut wp = wireplumber;
        terminate(&mut wp).await;
        terminate(&mut pipewire).await;
        return Err(anyhow!(
            "PipeWire-Pulse never created {}/pulse/native ({}); see {}",
            runtime_dir.display(),
            exit,
            log_dir().join("pipewire-pulse.log").display(),
        ));
    }

    // Remove orphaned Sunshine-created sinks from a previous runtime before the
    // Sunshine process starts tracking its own PulseAudio module IDs.
    cleanup_sunshine_virtual_sinks(env);
    ensure_audio_sink(env, Some(default_audio_channels()))?;

    Ok(PipewireStack {
        pipewire,
        wireplumber,
        pulse,
    })
}

fn exit_summary(child: Option<&mut Child>) -> String {
    match child {
        Some(c) => match c.try_wait() {
            Ok(Some(status)) => format!("process exited: {status}"),
            Ok(None) => "process still alive after timeout".into(),
            Err(e) => format!("could not query process status: {e}"),
        },
        None => "process handle missing".into(),
    }
}

#[derive(Debug, Default, Clone)]
pub struct SinkDetails {
    pub name: Option<String>,
    pub owner_module: Option<String>,
    pub channels: Option<u32>,
}

pub fn audio_sink_details(env: &BTreeMap<String, String>) -> SinkDetails {
    let Ok(out) = run_as_desktop_user(&["pactl", "list", "sinks"], env) else {
        return SinkDetails::default();
    };
    if !out.status.success() {
        return SinkDetails::default();
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut current = SinkDetails::default();

    for raw_line in stdout.lines() {
        let line = raw_line.trim();
        if raw_line.starts_with("Sink #") {
            if current.name.as_deref() == Some(AUDIO_SINK_NAME) {
                return current;
            }
            current = SinkDetails::default();
            continue;
        }
        if let Some(rest) = line.strip_prefix("Name:") {
            current.name = Some(rest.trim().to_string());
        } else if current.name.as_deref() == Some(AUDIO_SINK_NAME) {
            if let Some(rest) = line.strip_prefix("Owner Module:") {
                let owner = rest.trim().to_string();
                if owner.chars().all(|c| c.is_ascii_digit()) {
                    current.owner_module = Some(owner);
                }
            } else if let Some(rest) = line.strip_prefix("Sample Specification:") {
                for part in rest.split_whitespace() {
                    if let Some(c) = part.strip_suffix("ch") {
                        if let Ok(n) = c.parse() {
                            current.channels = Some(n);
                            break;
                        }
                    }
                }
            }
        }
    }
    if current.name.as_deref() == Some(AUDIO_SINK_NAME) {
        return current;
    }
    SinkDetails::default()
}

pub fn pipewire_node_id(
    env: &BTreeMap<String, String>,
    node_name: &str,
    media_class: &str,
) -> Option<String> {
    if !command_available("pw-dump") {
        return None;
    }
    let out = run_as_desktop_user(&["pw-dump", "Node"], env).ok()?;
    if !out.status.success() {
        return None;
    }
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    let nodes = value.as_array()?;
    for node in nodes {
        let props = node.get("info")?.get("props")?;
        if props.get("node.name").and_then(|v| v.as_str()) != Some(node_name) {
            continue;
        }
        if props.get("media.class").and_then(|v| v.as_str()) != Some(media_class) {
            continue;
        }
        let id = props
            .get("object.id")
            .or_else(|| node.get("id"))
            .map(|v| match v {
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            });
        return id;
    }
    None
}

pub fn set_audio_defaults(env: &BTreeMap<String, String>) {
    let _ = run_as_desktop_user_quiet(&["pactl", "set-default-sink", AUDIO_SINK_NAME], env);
    let monitor = format!("{}.monitor", AUDIO_SINK_NAME);
    let _ = run_as_desktop_user_quiet(&["pactl", "set-default-source", &monitor], env);

    if command_available("wpctl") {
        if let Some(id) = pipewire_node_id(env, AUDIO_SINK_NAME, "Audio/Sink") {
            let _ = run_as_desktop_user_quiet(&["wpctl", "set-default", &id], env);
        }
    }
    move_audio_streams_to_sink(env);
}

pub fn set_audio_volume(env: &BTreeMap<String, String>, channels: u32) {
    let volumes = vec!["100%"; channels as usize];
    let mut args: Vec<&str> = vec!["pactl", "set-sink-volume", AUDIO_SINK_NAME];
    args.extend(volumes.iter().copied());
    let _ = run_as_desktop_user_quiet(&args, env);

    let monitor = format!("{}.monitor", AUDIO_SINK_NAME);
    let mut source_args: Vec<&str> = vec!["pactl", "set-source-volume", &monitor];
    source_args.extend(volumes.iter().copied());
    let _ = run_as_desktop_user_quiet(&source_args, env);
}

pub fn move_audio_streams_to_sink(env: &BTreeMap<String, String>) {
    let Ok(out) = run_as_desktop_user(&["pactl", "list", "short", "sink-inputs"], env) else {
        return;
    };
    if !out.status.success() {
        return;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    for line in stdout.lines() {
        let mut parts = line.split_whitespace();
        let Some(id) = parts.next() else { continue };
        let _ = run_as_desktop_user_quiet(&["pactl", "move-sink-input", id, AUDIO_SINK_NAME], env);
    }
}

#[derive(Debug, Default)]
struct SinkModule {
    name: Option<String>,
    owner_module: Option<String>,
}

fn sunshine_virtual_sink_modules(env: &BTreeMap<String, String>) -> Vec<String> {
    let Ok(out) = run_as_desktop_user(&["pactl", "list", "sinks"], env) else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut modules = Vec::new();
    let mut current = SinkModule::default();

    for raw_line in stdout.lines() {
        let line = raw_line.trim();
        if raw_line.starts_with("Sink #") {
            if current
                .name
                .as_deref()
                .is_some_and(|name| name.starts_with("sink-sunshine-"))
            {
                if let Some(owner) = current.owner_module.take() {
                    modules.push(owner);
                }
            }
            current = SinkModule::default();
            continue;
        }
        if let Some(rest) = line.strip_prefix("Name:") {
            current.name = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("Owner Module:") {
            let owner = rest.trim();
            if owner.chars().all(|c| c.is_ascii_digit()) {
                current.owner_module = Some(owner.to_string());
            }
        }
    }
    if current
        .name
        .as_deref()
        .is_some_and(|name| name.starts_with("sink-sunshine-"))
    {
        if let Some(owner) = current.owner_module {
            modules.push(owner);
        }
    }
    modules.sort();
    modules.dedup();
    modules
}

pub fn cleanup_sunshine_virtual_sinks(env: &BTreeMap<String, String>) {
    for owner in sunshine_virtual_sink_modules(env) {
        let _ = run_as_desktop_user_quiet(&["pactl", "unload-module", &owner], env);
    }
}

pub fn ensure_audio_sink(env: &BTreeMap<String, String>, channels: Option<u32>) -> Result<()> {
    let fallback = default_audio_channels();
    let channels_text = channels.unwrap_or(fallback).to_string();
    let channels = sanitize_audio_channels(&channels_text, fallback);
    let channel_map = audio_channel_map(channels)
        .ok_or_else(|| anyhow!("unsupported audio channel count: {channels}"))?;

    let mut details = audio_sink_details(env);
    if details.name.is_some() && details.channels != Some(channels) {
        let Some(owner) = details.owner_module.clone() else {
            return Err(anyhow!(
                "cannot recreate {AUDIO_SINK_NAME}: owner module is unknown"
            ));
        };
        let log_handle = open_log_for_append("audio-sink.log").ok();
        let result = run_as_desktop_user(&["pactl", "unload-module", &owner], env)?;
        if let Some(mut handle) = log_handle {
            let _ = handle.write_all(&result.stdout);
            let _ = handle.write_all(&result.stderr);
        }
        if !result.status.success() {
            return Err(anyhow!(
                "failed to recreate {AUDIO_SINK_NAME} with {channels} channels"
            ));
        }
        details = SinkDetails::default();
    }

    if details.name.is_none() {
        let sink_arg = format!("sink_name={AUDIO_SINK_NAME}");
        let channels_arg = format!("channels={channels}");
        let channel_map_arg = format!("channel_map={channel_map}");
        let props_arg = format!("sink_properties=device.description={AUDIO_SINK_DESCRIPTION}");
        let result = run_as_desktop_user(
            &[
                "pactl",
                "load-module",
                "module-null-sink",
                &sink_arg,
                "rate=48000",
                &channels_arg,
                &channel_map_arg,
                &props_arg,
            ],
            env,
        )?;
        if !result.status.success() {
            return Err(anyhow!("failed to create {AUDIO_SINK_NAME} PipeWire sink"));
        }
    }

    set_audio_volume(env, channels);
    set_audio_defaults(env);
    Ok(())
}
