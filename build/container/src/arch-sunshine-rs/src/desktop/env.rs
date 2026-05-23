use std::collections::BTreeMap;
use std::path::Path;

use crate::paths::{desktop_user, home_dir};

pub fn desktop_base_env(
    runtime_dir: &Path,
    dbus_address: Option<&str>,
    wayland_display: Option<&str>,
) -> BTreeMap<String, String> {
    let home = home_dir();
    let mut env: BTreeMap<String, String> = BTreeMap::new();
    env.insert("HOME".into(), home.to_string_lossy().into());
    env.insert("USER".into(), desktop_user().into());
    env.insert("LOGNAME".into(), desktop_user().into());
    env.insert("LANG".into(), "en_US.UTF-8".into());
    env.insert("LC_CTYPE".into(), "en_US.UTF-8".into());
    env.insert("LC_MESSAGES".into(), "en_US.UTF-8".into());
    env.insert("LANGUAGE".into(), "en_US".into());
    env.insert(
        "XDG_RUNTIME_DIR".into(),
        runtime_dir.to_string_lossy().into(),
    );
    env.insert(
        "XDG_CONFIG_HOME".into(),
        home.join(".config").to_string_lossy().into(),
    );
    env.insert(
        "XDG_DATA_HOME".into(),
        home.join(".local").join("share").to_string_lossy().into(),
    );
    env.insert("XDG_CURRENT_DESKTOP".into(), "KDE".into());
    env.insert("XDG_MENU_PREFIX".into(), "plasma-".into());
    env.insert("XDG_SESSION_DESKTOP".into(), "KDE".into());
    env.insert("XDG_SESSION_TYPE".into(), "wayland".into());
    env.insert("DESKTOP_SESSION".into(), "plasma".into());
    env.insert("KDE_SESSION_VERSION".into(), "6".into());
    env.insert("KDE_FULL_SESSION".into(), "true".into());
    env.insert(
        "DISPLAY".into(),
        std::env::var("SUNSHINE_XWAYLAND_DISPLAY").unwrap_or_else(|_| ":0".into()),
    );
    env.insert("QT_QPA_PLATFORM".into(), "wayland".into());
    env.insert("SDL_VIDEODRIVER".into(), "wayland,x11".into());
    env.insert("MOZ_ENABLE_WAYLAND".into(), "1".into());
    env.insert("GIO_USE_NETWORK_MONITOR".into(), "base".into());
    env.insert(
        "RES_OPTIONS".into(),
        "timeout:1 attempts:2 rotate single-request-reopen".into(),
    );
    env.insert(
        "PULSE_SERVER".into(),
        format!("unix:{}/pulse/native", runtime_dir.display()),
    );
    env.insert("STEAM_RUNTIME".into(), "1".into());
    env.insert("SRT_URLOPEN_PREFER_STEAM".into(), "1".into());
    env.insert("STEAM_DISABLE_AUDIO_DEVICE_SWITCHING".into(), "1".into());

    if let Ok(level) = std::env::var("ARCH_SUNSHINE_PIPEWIRE_DEBUG") {
        let trimmed = level.trim();
        if !trimmed.is_empty() {
            env.insert("PIPEWIRE_DEBUG".into(), trimmed.into());
            env.insert("PIPEWIRE_LOG".into(), "1".into());
        }
    }
    if let Ok(level) = std::env::var("ARCH_SUNSHINE_WIREPLUMBER_LOG_LEVEL") {
        let trimmed = level.trim();
        if !trimmed.is_empty() {
            env.insert("WIREPLUMBER_LOG_LEVEL".into(), trimmed.into());
        }
    }

    if let Ok(compose) = std::env::var("SUNSHINE_KWIN_COMPOSE") {
        let trimmed = compose.trim();
        if !trimmed.is_empty() {
            env.insert("KWIN_COMPOSE".into(), trimmed.into());
        }
    }
    if let Some(address) = dbus_address {
        env.insert("DBUS_SESSION_BUS_ADDRESS".into(), address.into());
    }
    if let Some(display) = wayland_display {
        env.insert("WAYLAND_DISPLAY".into(), display.into());
    }
    env
}
