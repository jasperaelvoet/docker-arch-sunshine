use std::collections::HashMap;
use std::path::PathBuf;

use crate::paths::log_dir;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LogSource {
    Sunshine,
    Wrapper,
    KWin,
    Plasma,
    PipeWire,
    Pulse,
}

impl LogSource {
    pub const ALL: [LogSource; 6] = [
        LogSource::Sunshine,
        LogSource::Wrapper,
        LogSource::KWin,
        LogSource::Plasma,
        LogSource::PipeWire,
        LogSource::Pulse,
    ];

    pub fn label(self) -> &'static str {
        match self {
            LogSource::Sunshine => "Sunshine",
            LogSource::Wrapper => "Wrapper",
            LogSource::KWin => "KWin",
            LogSource::Plasma => "Plasma",
            LogSource::PipeWire => "PipeWire",
            LogSource::Pulse => "Pulse",
        }
    }

    pub fn path(self) -> PathBuf {
        let base = log_dir();
        match self {
            LogSource::Sunshine => base.join("sunshine.log"),
            LogSource::Wrapper => base.join("sunshine-wrapper.log"),
            LogSource::KWin => base.join("kwin-wayland.log"),
            LogSource::Plasma => base.join("plasmashell.log"),
            LogSource::PipeWire => base.join("pipewire.log"),
            LogSource::Pulse => base.join("pipewire-pulse.log"),
        }
    }
}

const MAX_LOG_LINES: usize = 2000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    Up,
    Down,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct StatusSnapshot {
    pub kwin: Health,
    pub plasmashell: Health,
    pub sunshine: Health,
    pub wayland_socket: bool,
    pub pipewire_socket: bool,
    pub pulse_socket: bool,
    pub audio_sink: String,
    pub render_nodes: String,
    pub encoder_summary: Vec<(String, Option<String>)>,
    pub stream_width: u32,
    pub stream_height: u32,
    pub stream_fps: String,
    pub stream_scale: String,
    pub logical_width: u32,
    pub logical_height: u32,
}

impl Default for StatusSnapshot {
    fn default() -> Self {
        Self {
            kwin: Health::Unknown,
            plasmashell: Health::Unknown,
            sunshine: Health::Unknown,
            wayland_socket: false,
            pipewire_socket: false,
            pulse_socket: false,
            audio_sink: "?".into(),
            render_nodes: "?".into(),
            encoder_summary: Vec::new(),
            stream_width: 0,
            stream_height: 0,
            stream_fps: String::new(),
            stream_scale: String::new(),
            logical_width: 0,
            logical_height: 0,
        }
    }
}

pub struct LogBuffer {
    lines: Vec<String>,
    pub unread: usize,
}

impl LogBuffer {
    pub fn new() -> Self {
        Self {
            lines: Vec::new(),
            unread: 0,
        }
    }

    pub fn push(&mut self, line: String, is_focused: bool) {
        self.lines.push(line);
        if self.lines.len() > MAX_LOG_LINES {
            let excess = self.lines.len() - MAX_LOG_LINES;
            self.lines.drain(..excess);
        }
        if !is_focused {
            self.unread = self.unread.saturating_add(1);
        }
    }

    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    pub fn clear_unread(&mut self) {
        self.unread = 0;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modal {
    None,
    PairPin,
    Shell,
}

pub struct AppState {
    pub status: StatusSnapshot,
    pub logs: HashMap<LogSource, LogBuffer>,
    pub focused_log: LogSource,
    pub log_scroll: usize,
    pub follow_tail: bool,
    pub modal: Modal,
    pub pin_input: String,
    pub pin_message: String,
    pub should_quit: bool,
    pub should_shell: bool,
    pub shell: Option<crate::tui::shell::ShellSession>,
}

impl AppState {
    pub fn new() -> Self {
        let mut logs = HashMap::new();
        for src in LogSource::ALL {
            logs.insert(src, LogBuffer::new());
        }
        Self {
            status: StatusSnapshot::default(),
            logs,
            focused_log: LogSource::Sunshine,
            log_scroll: 0,
            follow_tail: true,
            modal: Modal::None,
            pin_input: String::new(),
            pin_message: String::new(),
            should_quit: false,
            should_shell: false,
            shell: None,
        }
    }

    pub fn focus(&mut self, source: LogSource) {
        self.focused_log = source;
        if let Some(buf) = self.logs.get_mut(&source) {
            buf.clear_unread();
        }
        self.log_scroll = 0;
        self.follow_tail = true;
    }

    pub fn focused_buffer(&self) -> Option<&LogBuffer> {
        self.logs.get(&self.focused_log)
    }
}
