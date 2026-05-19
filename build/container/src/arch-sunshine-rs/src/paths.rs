use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub const DESKTOP_USER_DEFAULT: &str = "sunshine";
pub const DESKTOP_UID: u32 = 1000;
pub const DESKTOP_GID: u32 = 1000;

pub const DEFAULT_CONFIG: &str = "/usr/share/arch-sunshine/pipelines.json";
pub const USER_DATA: &str = "/mnt/user_data";
pub const DEFAULT_RUNTIME_DIR: &str = "/run/user/1000";
pub const DEFAULT_SOCKET: &str = "arch-sunshine-wayland";
pub const CONTROL_FIFO_NAME: &str = "arch-sunshine-control.fifo";
pub const DISCONNECT_DESKTOP_ID: &str = "arch-sunshine-disconnect.desktop";

pub const SUNSHINE_CONFIG_DIR: &str = "/run/arch-sunshine/sunshine";

pub const KWIN_OUTPUT_NAMES: &[&str] = &["Virtual-0", "ArchSunshine"];

pub const DEFAULT_WIDTH: u32 = 1920;
pub const DEFAULT_HEIGHT: u32 = 1080;
pub const DEFAULT_FPS: &str = "60";
pub const DEFAULT_SCALE: &str = "auto";

pub const AUDIO_SINK_NAME: &str = "arch_sunshine_audio";
pub const AUDIO_SINK_DESCRIPTION: &str = "Arch_Stream_Audio";

pub fn audio_channel_map(channels: u32) -> Option<&'static str> {
    match channels {
        2 => Some("front-left,front-right"),
        6 => Some("front-left,front-right,front-center,lfe,rear-left,rear-right"),
        8 => Some("front-left,front-right,front-center,lfe,rear-left,rear-right,side-left,side-right"),
        _ => None,
    }
}

static DESKTOP_USER: OnceLock<String> = OnceLock::new();

pub fn desktop_user() -> &'static str {
    DESKTOP_USER
        .get_or_init(|| {
            std::env::var("SUNSHINE_DESKTOP_USER").unwrap_or_else(|_| DESKTOP_USER_DEFAULT.into())
        })
        .as_str()
}

pub fn home_dir() -> PathBuf {
    PathBuf::from(format!("/home/{}", desktop_user()))
}

pub fn persistent_home_dir() -> PathBuf {
    Path::new(USER_DATA).join("home").join(desktop_user())
}

pub fn log_dir() -> PathBuf {
    Path::new(USER_DATA).join("var").join("log").join("arch-sunshine")
}

pub fn sunshine_state_dir() -> PathBuf {
    Path::new(USER_DATA).join("var").join("lib").join("sunshine")
}

pub fn disconnect_desktop_file() -> PathBuf {
    home_dir()
        .join(".local")
        .join("share")
        .join("applications")
        .join(DISCONNECT_DESKTOP_ID)
}

pub fn disconnect_desktop_icon() -> PathBuf {
    home_dir().join("Desktop").join(DISCONNECT_DESKTOP_ID)
}

pub fn control_fifo_path(runtime_dir: Option<&Path>) -> PathBuf {
    let dir = runtime_dir.map(|p| p.to_path_buf()).unwrap_or_else(|| {
        std::env::var("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(DEFAULT_RUNTIME_DIR))
    });
    dir.join(CONTROL_FIFO_NAME)
}
