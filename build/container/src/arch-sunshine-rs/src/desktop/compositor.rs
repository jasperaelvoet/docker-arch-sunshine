use anyhow::{anyhow, Result};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::time::Duration;

use crate::desktop::kde_config::{
    apply_kde_scale_config, apply_kde_session_tweaks, build_kwin_command,
    ensure_plasma_launcher_defaults, kactivitymanagerd_command,
};
use crate::desktop::pipewire::set_audio_defaults;
use crate::desktop::process::{
    open_log_for_append, run_as_desktop_user, run_as_desktop_user_quiet, spawn_as_desktop_user,
    terminate, wait_for_dbus_name, wait_for_path, wait_for_process_name,
};
use crate::desktop::{DesktopArgs, DesktopProcesses, StreamGeometry};

pub async fn start_desktop_stack(
    args: &DesktopArgs,
    runtime_dir: &Path,
    socket_path: &Path,
    compositor_env: &BTreeMap<String, String>,
    desktop_env: &BTreeMap<String, String>,
    geometry: &StreamGeometry,
    state: &mut DesktopProcesses,
) -> Result<()> {
    let socket_lock = runtime_dir.join(format!("{}.lock", args.socket));
    let _ = fs::remove_file(socket_path);
    let _ = fs::remove_file(&socket_lock);

    apply_kde_scale_config(
        desktop_env,
        geometry.width,
        geometry.height,
        &geometry.fps,
        &geometry.scale,
        false,
    );
    apply_kde_session_tweaks(desktop_env);
    crate::desktop::fs_setup::clear_kde_service_cache();

    let kwin_argv = build_kwin_command(
        &args.socket,
        args.xwayland,
        geometry.width,
        geometry.height,
        &geometry.scale,
    );
    let kwin_argv_str: Vec<&str> = kwin_argv.iter().map(|s| s.as_str()).collect();
    let mut kwin = spawn_as_desktop_user(&kwin_argv_str, compositor_env, "kwin-wayland.log")?;
    if !wait_for_path(
        socket_path,
        Some(&mut kwin),
        Duration::from_secs_f64(args.ready_timeout),
    )
    .await
    {
        return Err(anyhow!(
            "timed out waiting for KWin Wayland socket: {}",
            socket_path.display()
        ));
    }
    state.kwin = Some(kwin);

    apply_kde_scale_config(
        desktop_env,
        geometry.width,
        geometry.height,
        &geometry.fps,
        &geometry.scale,
        true,
    );
    let _ = run_as_desktop_user_quiet(&["xdg-user-dirs-update"], desktop_env);

    if let Ok(output) = run_as_desktop_user(&["kbuildsycoca6", "--noincremental"], desktop_env) {
        if let Ok(mut handle) = open_log_for_append("kbuildsycoca.log") {
            let _ = handle.write_all(&output.stdout);
            let _ = handle.write_all(&output.stderr);
        }
    }

    state.kded = Some(spawn_as_desktop_user(&["kded6"], desktop_env, "kded6.log")?);

    if let Some(cmd) = kactivitymanagerd_command() {
        let mut activities = spawn_as_desktop_user(&cmd, desktop_env, "kactivitymanagerd.log")?;
        if !wait_for_dbus_name(
            "org.kde.ActivityManager",
            desktop_env,
            Some(&mut activities),
            Duration::from_secs_f64(args.ready_timeout),
        )
        .await
        {
            return Err(anyhow!("timed out waiting for kactivitymanagerd"));
        }
        state.activities = Some(activities);
    }

    let mut plasmashell = spawn_as_desktop_user(
        &["plasmashell", "--no-respawn"],
        desktop_env,
        "plasmashell.log",
    )?;
    if !wait_for_process_name(
        "plasmashell",
        Some(&mut plasmashell),
        Duration::from_secs_f64(args.ready_timeout),
    )
    .await
    {
        return Err(anyhow!("timed out waiting for plasmashell"));
    }
    state.plasmashell = Some(plasmashell);
    ensure_plasma_launcher_defaults(desktop_env);
    set_audio_defaults(desktop_env);

    state.portal_backend = Some(spawn_as_desktop_user(
        &["/usr/lib/xdg-desktop-portal-kde"],
        desktop_env,
        "xdg-desktop-portal-kde.log",
    )?);
    state.portal = Some(spawn_as_desktop_user(
        &["/usr/lib/xdg-desktop-portal"],
        desktop_env,
        "xdg-desktop-portal.log",
    )?);
    Ok(())
}

pub async fn stop_desktop_stack(state: &mut DesktopProcesses) {
    terminate(&mut state.portal).await;
    terminate(&mut state.portal_backend).await;
    terminate(&mut state.plasmashell).await;
    terminate(&mut state.activities).await;
    terminate(&mut state.kded).await;
    terminate(&mut state.kwin).await;
}
