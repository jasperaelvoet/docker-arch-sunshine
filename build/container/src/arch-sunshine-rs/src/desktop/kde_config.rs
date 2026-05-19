use anyhow::Result;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::desktop::process::{
    chown_path, command_available, run_as_desktop_user, run_as_desktop_user_quiet,
};
use crate::env_config::{
    format_scale, kwin_virtual_geometry, plasma_scale_dpi, plasma_screen_scale_factors, scale_value,
};
use crate::paths::{home_dir, KWIN_OUTPUT_NAMES};

fn config_file(name: &str) -> std::path::PathBuf {
    home_dir().join(".config").join(name)
}

pub fn run_kwriteconfig(
    env: &BTreeMap<String, String>,
    file_name: &str,
    groups: &[&str],
    key: &str,
    value: &str,
) {
    let path = config_file(file_name);
    let path_str = path.to_string_lossy().into_owned();
    let mut args: Vec<&str> = vec!["kwriteconfig6", "--file", &path_str];
    for g in groups {
        args.push("--group");
        args.push(g);
    }
    args.push("--key");
    args.push(key);
    args.push(value);
    let _ = run_as_desktop_user_quiet(&args, env);
}

pub fn sync_x11_resources(env: &BTreeMap<String, String>) {
    if !command_available("kcminit") {
        return;
    }
    let _ = run_as_desktop_user_quiet(&["kcminit", "kcm_fonts_init", "kcm_style_init"], env);
}

pub fn update_kwin_output_config(width: u32, height: u32, fps: &str, scale: &str) -> Result<()> {
    let path = config_file("kwinoutputconfig.json");
    let Ok(text) = fs::read_to_string(&path) else {
        return Ok(());
    };
    let Ok(mut config): std::result::Result<serde_json::Value, _> = serde_json::from_str(&text)
    else {
        return Ok(());
    };

    let refresh_rate = (fps.parse::<f64>().unwrap_or(60.0) * 1000.0).round() as i64;
    let scale_v = scale_value(scale, 1.0);
    let mut changed = false;

    if let Some(sections) = config.as_array_mut() {
        for section in sections {
            if section.get("name").and_then(|v| v.as_str()) != Some("outputs") {
                continue;
            }
            let Some(data) = section.get_mut("data").and_then(|v| v.as_array_mut()) else {
                continue;
            };
            for output in data {
                let Some(connector) = output.get("connectorName").and_then(|v| v.as_str()) else {
                    continue;
                };
                if !KWIN_OUTPUT_NAMES.contains(&connector) {
                    continue;
                }
                let mode = output
                    .as_object_mut()
                    .unwrap()
                    .entry("mode".to_string())
                    .or_insert_with(|| serde_json::json!({}));
                let mode_obj = mode.as_object_mut().unwrap();
                mode_obj.insert("width".into(), serde_json::json!(width));
                mode_obj.insert("height".into(), serde_json::json!(height));
                mode_obj.insert("refreshRate".into(), serde_json::json!(refresh_rate));
                output
                    .as_object_mut()
                    .unwrap()
                    .insert("scale".into(), serde_json::json!(scale_v));
                changed = true;
            }
        }
    }

    if !changed {
        return Ok(());
    }
    let pretty = serde_json::to_string_pretty(&config)?;
    fs::write(&path, pretty + "\n")?;
    chown_path(&path, false);
    Ok(())
}

pub fn apply_kde_scale_config(
    env: &BTreeMap<String, String>,
    width: u32,
    height: u32,
    fps: &str,
    scale: &str,
    live: bool,
) {
    let scale_text = format_scale(scale_value(scale, 1.0));
    let dpi = plasma_scale_dpi(scale).to_string();

    run_kwriteconfig(env, "kdeglobals", &["KScreen"], "ScaleFactor", &scale_text);
    run_kwriteconfig(
        env,
        "kdeglobals",
        &["KScreen"],
        "ScreenScaleFactors",
        &plasma_screen_scale_factors(scale),
    );
    run_kwriteconfig(
        env,
        "kdeglobals",
        &["KScreen"],
        "XwaylandClientsScale",
        "true",
    );
    run_kwriteconfig(env, "kdeglobals", &["General"], "Xft.dpi", &dpi);
    run_kwriteconfig(env, "kwinrc", &["Xwayland"], "Scale", &scale_text);
    let _ = update_kwin_output_config(width, height, fps, scale);

    if live && command_available("kscreen-doctor") {
        for name in KWIN_OUTPUT_NAMES {
            let arg = format!("output.{name}.scale.{scale_text}");
            let _ = run_as_desktop_user_quiet(&["kscreen-doctor", &arg], env);
        }
    }
    if live {
        sync_x11_resources(env);
    }
}

pub fn apply_kde_session_tweaks(env: &BTreeMap<String, String>) {
    for module in [
        "Module-appmenu",
        "Module-baloosearchmodule",
        "Module-bluedevil",
        "Module-device_automounter",
        "Module-gtkconfig",
        "Module-kscreen",
        "Module-kwallet",
        "Module-networkmanagement",
        "Module-powerdevil",
        "Module-printmanager",
        "Module-proxyscout",
        "Module-wacomtablet",
    ] {
        run_kwriteconfig(env, "kded6rc", &[module], "autoload", "false");
    }

    run_kwriteconfig(env, "kdeglobals", &["KDE"], "AnimationDurationFactor", "0");
    run_kwriteconfig(env, "kwinrc", &["Compositing"], "AnimationSpeed", "0");
    run_kwriteconfig(
        env,
        "kwinrc",
        &["Compositing"],
        "LatencyPolicy",
        "ExtremelyLow",
    );
    run_kwriteconfig(env, "kwinrc", &["Compositing"], "GLPreferBufferSwap", "a");
    for plugin in [
        "blurEnabled",
        "contrastEnabled",
        "desktopgridEnabled",
        "fadingpopupsEnabled",
        "fullscreenEnabled",
        "loginEnabled",
        "logoutEnabled",
        "magiclampEnabled",
        "maximizeEnabled",
        "morphingpopupsEnabled",
        "overviewEnabled",
        "presentwindowsEnabled",
        "scaleEnabled",
        "screenedgeEnabled",
        "slideEnabled",
        "slidingpopupsEnabled",
        "squashEnabled",
        "tileseditorEnabled",
        "windowapertureEnabled",
        "windowviewEnabled",
        "wobblywindowsEnabled",
        "zoomEnabled",
    ] {
        run_kwriteconfig(env, "kwinrc", &["Plugins"], plugin, "false");
    }

    run_kwriteconfig(env, "kwalletrc", &["Wallet"], "Enabled", "false");
    run_kwriteconfig(env, "kwalletrc", &["Wallet"], "First Use", "false");

    for key in ["action/lock_screen", "lock_screen"] {
        run_kwriteconfig(
            env,
            "kdeglobals",
            &["KDE Action Restrictions"],
            key,
            "false",
        );
    }
    let kdeglobals = config_file("kdeglobals");
    if let Ok(text) = fs::read_to_string(&kdeglobals) {
        let filtered: Vec<&str> = text
            .lines()
            .filter(|line| !line.starts_with("action/logout") && !line.starts_with("logout"))
            .collect();
        let mut new_text = filtered.join("\n");
        new_text.push('\n');
        let new_text = new_text.replace(
            "[KDE Action Restrictions]\n",
            "[KDE Action Restrictions][$i]\n",
        );
        let _ = fs::write(&kdeglobals, new_text);
        chown_path(&kdeglobals, false);
    }

    let portal_dir = home_dir().join(".config").join("xdg-desktop-portal");
    let portal_config = portal_dir.join("portals.conf");
    if fs::create_dir_all(&portal_dir).is_ok() {
        let _ = fs::write(
            &portal_config,
            "[preferred]\ndefault=kde\norg.freedesktop.impl.portal.ScreenCast=kde\norg.freedesktop.impl.portal.RemoteDesktop=kde\n",
        );
        chown_path(&portal_dir, true);
    }
}

pub fn build_kwin_command(
    socket: &str,
    xwayland: bool,
    width: u32,
    height: u32,
    scale: &str,
) -> Vec<String> {
    let (logical_w, logical_h) = kwin_virtual_geometry(width, height, scale);
    let mut command: Vec<String> = vec![
        "kwin_wayland".into(),
        "--virtual".into(),
        "--width".into(),
        logical_w.to_string(),
        "--height".into(),
        logical_h.to_string(),
        "--scale".into(),
        scale.to_string(),
        "--socket".into(),
        socket.into(),
        "--no-lockscreen".into(),
    ];
    if xwayland {
        command.push("--xwayland".into());
    }
    command
}

pub fn ensure_plasma_launcher_defaults(env: &BTreeMap<String, String>) {
    let launcher_url = format!(
        "file://{}",
        crate::paths::disconnect_desktop_file().display()
    );
    let old_desktop_url = format!(
        "file://{}",
        crate::paths::disconnect_desktop_icon().display()
    );
    let disconnect_id = crate::paths::DISCONNECT_DESKTOP_ID;
    let taskbar_launchers = [
        "applications:org.kde.dolphin.desktop",
        "applications:firefox.desktop",
        "applications:steam.desktop",
    ];

    let taskbar_json = serde_json::to_string(&taskbar_launchers).unwrap_or_else(|_| "[]".into());
    let disconnect_id_json = serde_json::to_string(disconnect_id).unwrap_or_else(|_| "\"\"".into());
    let launcher_url_json = serde_json::to_string(&launcher_url).unwrap_or_else(|_| "\"\"".into());
    let old_desktop_url_json =
        serde_json::to_string(&old_desktop_url).unwrap_or_else(|_| "\"\"".into());

    let script = format!(
        r#"var disconnectId = {disconnect_id};
var disconnectUrls = [{launcher_url}, {old_desktop_url}];
var taskbarLaunchers = {taskbar};
var result = {{removedPanelIcons: 0, configuredLaunchers: 0, configuredTaskManagers: 0}};
function splitConfig(value) {{
    if (value === undefined || value === null) {{ return []; }}
    if (Array.isArray(value)) {{ value = value.join(","); }}
    return String(value).split(/[;,]/).map(function(entry) {{ return String(entry).trim(); }}).filter(function(entry) {{ return entry.length > 0; }});
}}
function isDisconnectValue(value) {{
    value = String(value);
    return value === disconnectId || value === 'applications:' + disconnectId || value.indexOf(disconnectId) !== -1;
}}
function withoutDisconnect(values) {{
    var seen = {{}}; var entries = [];
    values.forEach(function(value) {{
        if (isDisconnectValue(value) || seen[value]) {{ return; }}
        seen[value] = true; entries.push(value);
    }});
    return entries;
}}
function isDisconnectUrl(value) {{
    value = String(value);
    if (isDisconnectValue(value)) {{ return true; }}
    return disconnectUrls.indexOf(value) !== -1;
}}
function configureLauncher(widget) {{
    widget.currentConfigGroup = ['General'];
    if (widget.type === 'org.kde.plasma.kicker' || widget.type === 'org.kde.plasma.kickerdash') {{
        widget.writeConfig('favoriteApps', withoutDisconnect(splitConfig(widget.readConfig('favoriteApps'))));
        widget.writeConfig('favoriteSystemActions', []); widget.reloadConfig();
        result.configuredLaunchers += 1;
    }} else if (widget.type === 'org.kde.plasma.kickoff' || widget.type === 'metadata') {{
        widget.writeConfig('favorites', withoutDisconnect(splitConfig(widget.readConfig('favorites'))));
        widget.writeConfig('primaryActions', 0); widget.writeConfig('systemFavorites', []); widget.reloadConfig();
        result.configuredLaunchers += 1;
    }}
}}
function configureTaskManager(widget) {{
    if (widget.type !== 'org.kde.plasma.icontasks' && widget.type !== 'org.kde.plasma.taskmanager') {{ return; }}
    widget.currentConfigGroup = ['General'];
    widget.writeConfig('launchers', taskbarLaunchers.join(','));
    widget.reloadConfig();
    result.configuredTaskManagers += 1;
}}
function visitWidgets(container) {{
    container.widgetIds.forEach(function(widgetId) {{
        var widget = container.widgetById(widgetId);
        if (!widget) {{ return; }}
        if (widget.type === 'org.kde.plasma.icon') {{
            widget.currentConfigGroup = ['General'];
            if (isDisconnectUrl(widget.readConfig('url'))) {{
                widget.remove(); result.removedPanelIcons += 1; return;
            }}
        }}
        configureLauncher(widget); configureTaskManager(widget);
    }});
}}
panels().forEach(visitWidgets);
desktops().forEach(visitWidgets);
print(JSON.stringify(result));
JSON.stringify(result);"#,
        disconnect_id = disconnect_id_json,
        launcher_url = launcher_url_json,
        old_desktop_url = old_desktop_url_json,
        taskbar = taskbar_json,
    );

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut combined = String::new();
    loop {
        let result = run_as_desktop_user(
            &[
                "timeout",
                "2s",
                "qdbus6",
                "org.kde.plasmashell",
                "/PlasmaShell",
                "evaluateScript",
                &script,
            ],
            env,
        );
        if let Ok(out) = result {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            combined.push_str(&stdout);
            combined.push_str(&stderr);
            if out.status.success() && stdout.contains("configuredLaunchers") {
                break;
            }
        }
        if std::time::Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }

    let log_path = crate::paths::log_dir().join("plasma-launchers.log");
    if let Some(parent) = log_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(log_path, combined);
}

pub fn kactivitymanagerd_command() -> Option<Vec<&'static str>> {
    let system_path = Path::new("/usr/lib/kactivitymanagerd");
    if system_path.exists() {
        return Some(vec!["/usr/lib/kactivitymanagerd"]);
    }
    if command_available("kactivitymanagerd") {
        return Some(vec!["kactivitymanagerd"]);
    }
    None
}
