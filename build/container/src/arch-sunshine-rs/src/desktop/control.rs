use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use crate::desktop::process::chown_path;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "command")]
pub enum ControlMessage {
    #[serde(rename = "client-start")]
    ClientStart {
        width: u32,
        height: u32,
        fps: serde_json::Value,
        scale: serde_json::Value,
        #[serde(default)]
        audio_channels: Option<u32>,
        #[serde(default)]
        ack: Option<String>,
    },
    #[serde(rename = "client-stop")]
    ClientStop {
        #[serde(default)]
        audio_channels: Option<u32>,
    },
}

pub struct ControlChannel {
    pub path: PathBuf,
    read_file: File,
    _keepalive: File,
    buffer: String,
}

impl ControlChannel {
    pub fn open(path: PathBuf) -> Result<Self> {
        let _ = std::fs::remove_file(&path);
        nix::unistd::mkfifo(&path, nix::sys::stat::Mode::from_bits_truncate(0o660))
            .with_context(|| format!("creating fifo {}", path.display()))?;
        chown_path(&path, false);

        let read_file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&path)?;
        let keepalive = OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&path)?;
        Ok(Self {
            path,
            read_file,
            _keepalive: keepalive,
            buffer: String::new(),
        })
    }

    pub async fn read_messages(&mut self) -> Vec<ControlMessage> {
        let mut messages = Vec::new();
        let mut tmp = [0u8; 4096];
        loop {
            match self.read_file.read(&mut tmp) {
                Ok(0) => break,
                Ok(n) => {
                    self.buffer
                        .push_str(&String::from_utf8_lossy(&tmp[..n]));
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(_) => break,
            }
        }
        while let Some(idx) = self.buffer.find('\n') {
            let line: String = self.buffer.drain(..=idx).collect();
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            match serde_json::from_str::<ControlMessage>(trimmed) {
                Ok(msg) => messages.push(msg),
                Err(e) => eprintln!("arch-sunshine: ignored invalid control message: {e}"),
            }
        }
        messages
    }
}

impl Drop for ControlChannel {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub fn send_control_message_sync(path: &Path, message: &serde_json::Value) -> Result<()> {
    let payload = format!("{}\n", serde_json::to_string(message)?);
    let file = OpenOptions::new()
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
        .map_err(|e| anyhow!("desktop control channel is not ready: {} ({e})", path.display()))?;
    use std::io::Write;
    let mut file = file;
    file.write_all(payload.as_bytes())?;
    Ok(())
}
