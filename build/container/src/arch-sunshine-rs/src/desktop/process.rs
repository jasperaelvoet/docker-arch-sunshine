use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Once;
use std::time::{Duration, Instant};
use tokio::process::Child;

use crate::paths::{log_dir, DESKTOP_GID, DESKTOP_UID};

static REAPER_STARTED: Once = Once::new();

pub fn start_child_reaper() {
    REAPER_STARTED.call_once(|| {
        let _ = std::thread::Builder::new()
            .name("arch-sunshine-reaper".into())
            .spawn(|| loop {
                reap_orphaned_children();
                std::thread::sleep(Duration::from_secs(5));
            });
    });
}

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

fn desktop_user_argv(command: &[&str], env: &BTreeMap<String, String>) -> Vec<String> {
    let mut argv: Vec<String> = vec![
        "setpriv".into(),
        "--reuid".into(),
        DESKTOP_UID.to_string(),
        "--regid".into(),
        DESKTOP_GID.to_string(),
        "--init-groups".into(),
        "--".into(),
        "env".into(),
    ];
    argv.extend(env_args(env));
    argv.extend(command.iter().map(|s| s.to_string()));
    argv
}

pub fn run_as_desktop_user(
    command: &[&str],
    env: &BTreeMap<String, String>,
) -> std::io::Result<std::process::Output> {
    let argv = desktop_user_argv(command, env);
    Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(Stdio::null())
        .output()
}

pub fn run_as_desktop_user_quiet(command: &[&str], env: &BTreeMap<String, String>) -> bool {
    let argv = desktop_user_argv(command, env);
    Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn max_log_bytes() -> u64 {
    std::env::var("ARCH_SUNSHINE_LOG_MAX_BYTES")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|v| *v >= 1024 * 1024)
        .unwrap_or(16 * 1024 * 1024)
}

fn rotate_log_if_needed(path: &Path) {
    let Ok(metadata) = std::fs::metadata(path) else {
        return;
    };
    if metadata.len() < max_log_bytes() {
        return;
    }
    let rotated = path.with_extension(format!(
        "{}.1",
        path.extension().and_then(|e| e.to_str()).unwrap_or("log")
    ));
    let _ = std::fs::remove_file(&rotated);
    let _ = std::fs::rename(path, rotated);
}

pub fn open_log_for_append(log_name: &str) -> Result<std::fs::File> {
    let path = log_dir().join(log_name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    rotate_log_if_needed(&path);
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("opening log file {}", path.display()))
}

fn start_new_session(command: &mut tokio::process::Command) {
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

pub fn spawn_as_desktop_user(
    command: &[&str],
    env: &BTreeMap<String, String>,
    log_name: &str,
) -> Result<Child> {
    let log_handle = open_log_for_append(log_name)?;
    let stderr = log_handle.try_clone()?;
    let argv = desktop_user_argv(command, env);

    let mut cmd = tokio::process::Command::new(&argv[0]);
    start_new_session(&mut cmd);
    let child = cmd
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

    let mut cmd = tokio::process::Command::new(&argv[0]);
    start_new_session(&mut cmd);
    let child = cmd
        .args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::from(log_handle))
        .stderr(Stdio::from(stderr))
        .spawn()
        .with_context(|| format!("spawning {}", command.join(" ")))?;
    Ok(child)
}

fn signal_process_group(pid: u32, signal: nix::sys::signal::Signal) {
    let pgid = nix::unistd::Pid::from_raw(-(pid as i32));
    if nix::sys::signal::kill(pgid, signal).is_err() {
        let _ = nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid as i32), signal);
    }
}

pub async fn terminate(child: &mut Option<Child>) {
    let Some(mut process) = child.take() else {
        return;
    };
    match process.try_wait() {
        Ok(Some(_)) | Err(_) => return,
        Ok(None) => {}
    }
    let pid = process.id();
    if let Some(pid) = pid {
        signal_process_group(pid, nix::sys::signal::Signal::SIGTERM);
    }
    let wait = tokio::time::timeout(Duration::from_secs(3), process.wait()).await;
    if wait.is_err() {
        if let Some(pid) = pid {
            signal_process_group(pid, nix::sys::signal::Signal::SIGKILL);
        } else {
            let _ = process.kill().await;
        }
        let _ = process.wait().await;
    }
}

pub fn kill_desktop_processes() {
    let _ = run_checked(&[
        "pkill",
        "-TERM",
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
        let _ = run_checked(&["pkill", "-TERM", "-u", &DESKTOP_UID.to_string(), "-x", name]);
    }
    let _ = run_checked(&[
        "pkill",
        "-TERM",
        "-u",
        &DESKTOP_UID.to_string(),
        "-f",
        "/usr/lib/kactivitymanagerd",
    ]);
    reap_orphaned_children();
}

pub fn reap_orphaned_children() {
    loop {
        match nix::sys::wait::waitpid(
            nix::unistd::Pid::from_raw(-1),
            Some(nix::sys::wait::WaitPidFlag::WNOHANG),
        ) {
            Ok(nix::sys::wait::WaitStatus::StillAlive) => break,
            Ok(_) => continue,
            Err(nix::errno::Errno::ECHILD) => break,
            Err(_) => break,
        }
    }
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
            match c.try_wait() {
                Ok(Some(_)) | Err(_) => return false,
                Ok(None) => {}
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

pub async fn wait_for_process_name(
    name: &str,
    child: Option<&mut Child>,
    timeout: Duration,
) -> bool {
    let deadline = Instant::now() + timeout;
    let mut child = child;
    while Instant::now() < deadline {
        if process_name_running(name) {
            return true;
        }
        if let Some(c) = child.as_deref_mut() {
            match c.try_wait() {
                Ok(Some(_)) | Err(_) => return false,
                Ok(None) => {}
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

pub async fn wait_for_tcp_port(
    host: &str,
    port: u16,
    child: Option<&mut Child>,
    timeout: Duration,
) -> bool {
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
            match c.try_wait() {
                Ok(Some(_)) | Err(_) => return false,
                Ok(None) => {}
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
            match c.try_wait() {
                Ok(Some(_)) | Err(_) => return false,
                Ok(None) => {}
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
