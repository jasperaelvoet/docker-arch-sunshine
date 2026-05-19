use anyhow::{anyhow, Result};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use crate::desktop::process::{
    chown_path, command_available, process_running, run_checked, run_quiet, wait_for_path_sync,
};
use crate::paths::{
    desktop_user, home_dir, log_dir, persistent_home_dir, sunshine_state_dir, DEFAULT_SOCKET,
    SUNSHINE_CONFIG_DIR, USER_DATA,
};

pub fn running_as_root() -> bool {
    nix::unistd::Uid::effective().is_root()
}

pub fn ensure_desktop_user() -> Result<()> {
    let user = desktop_user();
    nix::unistd::User::from_name(user)
        .ok()
        .flatten()
        .ok_or_else(|| anyhow!("desktop user {:?} does not exist", user))?;
    Ok(())
}

fn path_is_mountpoint(path: &Path) -> bool {
    run_quiet(&["mountpoint", "-q", path.to_str().unwrap_or("")])
}

pub fn bind_persistent_home() -> Result<()> {
    let home = home_dir();
    let persistent = persistent_home_dir();
    fs::create_dir_all(&home)?;
    fs::create_dir_all(&persistent)?;
    if path_is_mountpoint(&home) {
        return Ok(());
    }
    let result = run_checked(&[
        "mount",
        "--bind",
        persistent.to_str().unwrap_or(""),
        home.to_str().unwrap_or(""),
    ])?;
    if !result.status.success() {
        return Err(anyhow!(
            "failed to bind {} to {}",
            persistent.display(),
            home.display()
        ));
    }
    Ok(())
}

pub fn sync_input_device_nodes() -> bool {
    let input_dir = Path::new("/dev/input");
    fs::create_dir_all(input_dir).ok();
    let _ = fs::set_permissions(input_dir, fs::Permissions::from_mode(0o755));
    let mut changed = false;

    let entries: Vec<PathBuf> = match fs::read_dir("/sys/class/input") {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.file_name().and_then(|n| n.to_str()).map(|n| n.starts_with("event")).unwrap_or(false))
            .collect(),
        Err(_) => return false,
    };
    let mut entries = entries;
    entries.sort();

    for event_path in entries {
        let dev_file = event_path.join("dev");
        let Ok(text) = fs::read_to_string(&dev_file) else { continue };
        let trimmed = text.trim();
        let Some((major_text, minor_text)) = trimmed.split_once(':') else { continue };
        let Ok(major): Result<u64, _> = major_text.parse() else { continue };
        let Ok(minor): Result<u64, _> = minor_text.parse() else { continue };
        let Some(name) = event_path.file_name() else { continue };
        let node = input_dir.join(name);
        if !node.exists() {
            use nix::sys::stat::{mknod, Mode, SFlag};
            let dev: libc::dev_t = libc::makedev(major as _, minor as _);
            if mknod(&node, SFlag::S_IFCHR, Mode::from_bits_truncate(0o666), dev).is_ok() {
                changed = true;
            }
        }
        let _ = fs::set_permissions(&node, fs::Permissions::from_mode(0o666));
    }
    changed
}

pub fn start_udev_stack() {
    let udevd = Path::new("/usr/lib/systemd/systemd-udevd");
    if !udevd.exists() {
        sync_input_device_nodes();
        return;
    }
    let _ = fs::create_dir_all("/run/udev/data");
    if !process_running("systemd-udevd") {
        let _ = run_checked(&[udevd.to_str().unwrap(), "--daemon"]);
        wait_for_path_sync(Path::new("/run/udev/control"), std::time::Duration::from_secs(5));
    }
    sync_input_device_nodes();
    let _ = run_quiet(&["udevadm", "control", "--reload-rules"]);
    let _ = run_quiet(&["udevadm", "trigger", "--action=add", "-s", "input"]);
    let _ = run_quiet(&["udevadm", "settle", "--timeout=5"]);
    sync_input_device_nodes();
}

pub fn prepare_desktop_filesystem(runtime_dir: &Path) -> Result<()> {
    if !running_as_root() {
        return Err(anyhow!(
            "desktop-session must run as root so it can prepare /run/user and the persistent home bind mount"
        ));
    }
    ensure_desktop_user()?;

    let phome = persistent_home_dir();
    let dirs: Vec<PathBuf> = vec![
        Path::new(USER_DATA).into(),
        phome.clone(),
        log_dir(),
        PathBuf::from(SUNSHINE_CONFIG_DIR),
        sunshine_state_dir(),
        runtime_dir.into(),
        phome.join(".cache"),
        phome.join(".config"),
        phome.join(".local").join("share"),
        phome.join("Desktop"),
        phome.join("Documents"),
        phome.join("Downloads"),
        phome.join("Games"),
        phome.join("Pictures"),
        phome.join("Videos"),
    ];
    for dir in &dirs {
        fs::create_dir_all(dir).ok();
    }

    bind_persistent_home()?;
    chown_path(&phome, true);
    chown_path(&log_dir(), true);
    chown_path(&sunshine_state_dir(), true);
    chown_path(runtime_dir, false);
    fs::set_permissions(runtime_dir, fs::Permissions::from_mode(0o700)).ok();

    let x11_socket_dir = Path::new("/tmp/.X11-unix");
    fs::create_dir_all(x11_socket_dir).ok();
    fs::set_permissions(x11_socket_dir, fs::Permissions::from_mode(0o1777)).ok();

    for stale in [
        runtime_dir.join(DEFAULT_SOCKET),
        runtime_dir.join(format!("{DEFAULT_SOCKET}.lock")),
        runtime_dir.join("bus"),
    ] {
        let _ = fs::remove_file(stale);
    }

    start_udev_stack();

    let mut device_paths: Vec<PathBuf> = vec!["/dev/uinput".into(), "/dev/uhid".into()];
    if let Ok(rd) = fs::read_dir("/dev/dri") {
        for entry in rd.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("card") || name.starts_with("renderD") {
                device_paths.push(entry.path());
            }
        }
    }
    if let Ok(rd) = fs::read_dir("/dev/input") {
        for entry in rd.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("event") {
                device_paths.push(entry.path());
            }
        }
    }
    for dev in device_paths {
        let _ = fs::set_permissions(&dev, fs::Permissions::from_mode(0o666));
    }

    Ok(())
}

pub fn write_wayland_desktop_config() -> Result<()> {
    let config_dir = home_dir().join(".config");
    let env_dir = config_dir.join("plasma-workspace").join("env");
    fs::create_dir_all(&env_dir)?;
    let disconnect_file = crate::paths::disconnect_desktop_file();
    if let Some(parent) = disconnect_file.parent() {
        fs::create_dir_all(parent)?;
    }

    fs::write(
        config_dir.join("ksmserverrc"),
        "[General]\nconfirmLogout=false\nloginMode=emptySession\n",
    )?;
    fs::write(config_dir.join("ksplashrc"), "[KSplash]\nEngine=none\n")?;

    let env_script = [
        "export XDG_SESSION_TYPE=wayland",
        "export XDG_CURRENT_DESKTOP=KDE",
        "export XDG_SESSION_DESKTOP=KDE",
        "export XDG_MENU_PREFIX=plasma-",
        "export DESKTOP_SESSION=plasma",
        "export KDE_SESSION_VERSION=6",
        "export KDE_FULL_SESSION=true",
        "export LC_MESSAGES=en_US.UTF-8",
        "export LANGUAGE=en_US",
        "export DISPLAY=${SUNSHINE_XWAYLAND_DISPLAY:-:0}",
        "export QT_QPA_PLATFORM=wayland",
        "export SDL_VIDEODRIVER=wayland,x11",
        "export MOZ_ENABLE_WAYLAND=1",
        "export GIO_USE_NETWORK_MONITOR=base",
        "export STEAM_RUNTIME=1",
        "export SRT_URLOPEN_PREFER_STEAM=1",
        "export STEAM_DISABLE_AUDIO_DEVICE_SWITCHING=1",
        "",
    ]
    .join("\n");
    fs::write(env_dir.join("arch-sunshine.sh"), env_script)?;

    let desktop_entry = [
        "[Desktop Entry]",
        "Type=Application",
        "Name=Disconnect",
        "Comment=Close the Moonlight stream",
        "Exec=/usr/local/bin/arch-sunshine disconnect",
        "Icon=system-log-out",
        "Terminal=false",
        "Categories=System;",
        "StartupNotify=false",
        "NoDisplay=false",
        "",
    ]
    .join("\n");
    fs::write(&disconnect_file, desktop_entry)?;
    fs::set_permissions(&disconnect_file, fs::Permissions::from_mode(0o644))?;
    let _ = fs::remove_file(crate::paths::disconnect_desktop_icon());

    cleanup_disconnect_desktop_mapping(&config_dir.join("plasma-org.kde.plasma.desktop-appletsrc"));
    cleanup_kickoff_launcher_config(&config_dir.join("plasma-org.kde.plasma.desktop-appletsrc"));
    chown_path(&config_dir, true);
    chown_path(&disconnect_file, false);
    Ok(())
}

fn cleanup_disconnect_desktop_mapping(path: &Path) {
    let Ok(text) = fs::read_to_string(path) else { return };
    let filtered: Vec<&str> = text
        .lines()
        .filter(|line| !line.contains(crate::paths::DISCONNECT_DESKTOP_ID))
        .collect();
    if filtered.len() == text.lines().count() {
        return;
    }
    let _ = fs::write(path, filtered.join("\n") + "\n");
}

fn cleanup_kickoff_launcher_config(path: &Path) {
    let Ok(text) = fs::read_to_string(path) else { return };
    let mut fixed = Vec::with_capacity(text.lines().count());
    let mut changed = false;
    for line in text.lines() {
        if line == "plugin=metadata" {
            fixed.push("plugin=org.kde.plasma.kickoff".to_string());
            changed = true;
        } else if line.starts_with("primaryActions=") {
            fixed.push("primaryActions=0".to_string());
            changed = true;
        } else if line.starts_with("systemFavorites=") {
            fixed.push("systemFavorites=".to_string());
            changed = true;
        } else {
            fixed.push(line.to_string());
        }
    }
    if changed {
        let _ = fs::write(path, fixed.join("\n") + "\n");
    }
}

pub fn clear_kde_service_cache() {
    let cache = home_dir().join(".cache");
    let Ok(rd) = fs::read_dir(&cache) else { return };
    for entry in rd.flatten() {
        let name = entry.file_name();
        if name.to_string_lossy().starts_with("ksycoca6") {
            let path = entry.path();
            if path.is_dir() {
                let _ = fs::remove_dir_all(&path);
            } else {
                let _ = fs::remove_file(&path);
            }
        }
    }
}

pub fn _is_command_available(name: &str) -> bool {
    command_available(name)
}
