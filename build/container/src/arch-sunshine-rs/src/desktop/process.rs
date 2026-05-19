use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tokio::process::Child;

use crate::paths::{desktop_user, log_dir, DESKTOP_UID};

pub fn run_checked(command: &[&str]) -> std::io::Result<std::process::Output> {
    Command::new(command[0])
        .args(&command[1..])
        .stdin(Stdio::null())
        .output()
}

pub fn run_quiet(command: &[&str]) -> bool {
    Command::new(command[0])
        .args(&command[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn process_running(name: &str) -> bool {
    run_quiet(&["pgrep", "-x", name])
}

pub fn process_name_running(name: &str) -> bool {
    run_quiet(&["pgrep", "-u", &DESKTOP_UID.to_string(), "-x", name])
}

pub fn command_available(name: &str) -> bool {
    which(name).is_some()
}

pub fn which(name: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

pub fn env_args(env: &BTreeMap<String, String>) -> Vec<String> {
    env.iter().map(|(k, v)| format!("{k}={v}")).collect()
}

pub fn run_as_desktop_user(command: &[&str], env: &BTreeMap<String, String>) -> std::io::Result<std::process::Output> {
    let env_pairs = env_args(env);
    let mut args: Vec<&str> = vec!["runuser", "-u", desktop_user(), "--", "env"];
    let env_str: Vec<&str> = env_pairs.iter().map(|s| s.as_str()).collect();
    args.extend(env_str.iter().copied());
    args.extend(command.iter().copied());
    Command::new(args[0])
        .args(&args[1..])
        .stdin(Stdio::null())
        .output()
}

pub fn run_as_desktop_user_quiet(command: &[&str], env: &BTreeMap<String, String>) -> bool {
    let env_pairs = env_args(env);
    let mut args: Vec<&str> = vec!["runuser", "-u", desktop_user(), "--", "env"];
    let env_str: Vec<&str> = env_pairs.iter().map(|s| s.as_str()).collect();
    args.extend(env_str.iter().copied());
    args.extend(command.iter().copied());
    Command::new(args[0])
        .args(&args[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn open_log_for_append(log_name: &str) -> Result<std::fs::File> {
    let path = log_dir().join(log_name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("opening log file {}", path.display()))
}

pub fn spawn_as_desktop_user(
    command: &[&str],
    env: &BTreeMap<String, String>,
    log_name: &str,
) -> Result<Child> {
    let log_handle = open_log_for_append(log_name)?;
    let stderr = log_handle.try_clone()?;
    let env_pairs = env_args(env);
    let mut argv: Vec<String> = vec!["runuser".into(), "-u".into(), desktop_user().into(), "--".into(), "env".into()];
    argv.extend(env_pairs);
    argv.extend(command.iter().map(|s| s.to_string()));

    let child = tokio::process::Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::from(log_handle))
        .stderr(Stdio::from(stderr))
        .spawn()
        .with_context(|| format!("spawning {}", command.join(" ")))?;
    Ok(child)
}

pub fn spawn_with_env(
    command: &[&str],
    env: &BTreeMap<String, String>,
    log_name: &str,
) -> Result<Child> {
    let log_handle = open_log_for_append(log_name)?;
    let stderr = log_handle.try_clone()?;
    let env_pairs = env_args(env);
    let mut argv: Vec<String> = vec!["env".into()];
    argv.extend(env_pairs);
    argv.extend(command.iter().map(|s| s.to_string()));

    let child = tokio::process::Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::from(log_handle))
        .stderr(Stdio::from(stderr))
        .spawn()
        .with_context(|| format!("spawning {}", command.join(" ")))?;
    Ok(child)
}

pub async fn terminate(child: &mut Option<Child>) {
    let Some(mut process) = child.take() else { return };
    if process.try_wait().ok().flatten().is_some() {
        return;
    }
    let pid = process.id();
    if let Some(pid) = pid {
        let _ = nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(pid as i32),
            nix::sys::signal::Signal::SIGTERM,
        );
    }
    let wait = tokio::time::timeout(Duration::from_secs(3), process.wait()).await;
    if wait.is_err() {
        let _ = process.kill().await;
        let _ = process.wait().await;
    }
}

pub fn kill_desktop_processes() {
    let _ = run_checked(&[
        "pkill",
        "-TERM",
        "-u",
        &DESKTOP_UID.to_string(),
        "-f",
        "/usr/local/bin/arch-sunshine session-actions",
    ]);
    for name in [
        "sunshine",
        "xdg-desktop-portal",
        "xdg-desktop-portal-kde",
        "plasmashell",
        "kded6",
        "kactivitymanage",
        "kactivitymanagerd",
        "kwin_wayland",
        "Xwayland",
        "pipewire-pulse",
        "wireplumber",
        "pipewire",
    ] {
        let _ = run_checked(&[
            "pkill",
            "-TERM",
            "-u",
            &DESKTOP_UID.to_string(),
            "-x",
            name,
        ]);
    }
    let _ = run_checked(&[
        "pkill",
        "-TERM",
        "-u",
        &DESKTOP_UID.to_string(),
        "-f",
        "/usr/lib/kactivitymanagerd",
    ]);
}

pub fn wait_for_path_sync(path: &Path, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if path.exists() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

pub async fn wait_for_path(path: &Path, child: Option<&mut Child>, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    let mut child = child;
    while Instant::now() < deadline {
        if path.exists() {
            return true;
        }
        if let Some(c) = child.as_deref_mut() {
            if let Ok(Some(_)) = c.try_wait() {
                return false;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

pub async fn wait_for_process_name(name: &str, child: Option<&mut Child>, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    let mut child = child;
    while Instant::now() < deadline {
        if process_name_running(name) {
            return true;
        }
        if let Some(c) = child.as_deref_mut() {
            if let Ok(Some(_)) = c.try_wait() {
                return false;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

pub async fn wait_for_tcp_port(host: &str, port: u16, child: Option<&mut Child>, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    let mut child = child;
    while Instant::now() < deadline {
        if tokio::time::timeout(
            Duration::from_millis(250),
            tokio::net::TcpStream::connect((host, port)),
        )
        .await
        .map(|r| r.is_ok())
        .unwrap_or(false)
        {
            return true;
        }
        if let Some(c) = child.as_deref_mut() {
            if let Ok(Some(_)) = c.try_wait() {
                return false;
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    false
}

pub async fn wait_for_dbus_name(
    name: &str,
    env: &BTreeMap<String, String>,
    child: Option<&mut Child>,
    timeout: Duration,
) -> bool {
    let deadline = Instant::now() + timeout;
    let mut child = child;
    while Instant::now() < deadline {
        if let Some(c) = child.as_deref_mut() {
            if let Ok(Some(_)) = c.try_wait() {
                return false;
            }
        }
        let output = run_as_desktop_user(
            &[
                "qdbus6",
                "org.freedesktop.DBus",
                "/",
                "org.freedesktop.DBus.NameHasOwner",
                name,
            ],
            env,
        );
        if let Ok(out) = output {
            if out.status.success() {
                let stdout = String::from_utf8_lossy(&out.stdout);
                if stdout.trim() == "true" {
                    return true;
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

pub fn chown_path(path: &Path, recursive: bool) {
    let uid_gid = format!("{}:{}", DESKTOP_UID, crate::paths::DESKTOP_GID);
    let mut args: Vec<&str> = vec!["chown"];
    if recursive {
        args.push("-R");
    }
    let path_str = path.to_string_lossy().into_owned();
    args.push(&uid_gid);
    args.push(&path_str);
    let _ = run_quiet(&args);
}
