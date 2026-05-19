use anyhow::{anyhow, Result};
use std::fs;
use std::path::Path;
use std::time::Duration;

use crate::desktop::env::desktop_base_env;
use crate::desktop::process::{run_as_desktop_user, run_checked, wait_for_path_sync};

pub struct SessionBus {
    pub address: String,
    pub pid: String,
}

pub fn start_session_bus(runtime_dir: &Path) -> Result<SessionBus> {
    let bus_path = runtime_dir.join("bus");
    let _ = fs::remove_file(&bus_path);

    let env = desktop_base_env(runtime_dir, None, None);
    let bus_path_str = bus_path.to_string_lossy().into_owned();
    let address_arg = format!("--address=unix:path={}", bus_path_str);
    let output = run_as_desktop_user(
        &[
            "dbus-daemon",
            "--session",
            "--fork",
            &address_arg,
            "--print-address",
            "--print-pid",
        ],
        &env,
    )?;

    if !output.status.success() {
        return Err(anyhow!(
            "failed to start session D-Bus: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    if lines.len() < 2 {
        return Err(anyhow!("session D-Bus did not report its address and pid"));
    }
    if !wait_for_path_sync(&bus_path, Duration::from_secs(5)) {
        return Err(anyhow!(
            "timed out waiting for session D-Bus socket: {}",
            bus_path.display()
        ));
    }
    Ok(SessionBus {
        address: lines[0].to_string(),
        pid: lines[1].to_string(),
    })
}

pub fn start_system_bus() -> Result<String> {
    fs::create_dir_all("/run/dbus").ok();
    fs::create_dir_all("/var/lib/dbus").ok();
    let machine_id = Path::new("/var/lib/dbus/machine-id");
    if !machine_id.exists() && Path::new("/etc/machine-id").exists() {
        let _ = fs::copy("/etc/machine-id", machine_id);
    }
    let socket_path = Path::new("/run/dbus/system_bus_socket");
    if socket_path.exists() {
        return Ok(String::new());
    }
    let output = run_checked(&["dbus-daemon", "--system", "--fork", "--print-pid"])?;
    if !output.status.success() {
        return Err(anyhow!(
            "failed to start system D-Bus: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let last_line = stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .last()
        .ok_or_else(|| anyhow!("system D-Bus did not report its pid"))?;
    Ok(last_line.to_string())
}
