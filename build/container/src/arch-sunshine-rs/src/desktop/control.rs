use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{Read, Result as IoResult};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use tokio::io::unix::AsyncFd;

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
    read_file: AsyncFd<File>,
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
            read_file: AsyncFd::new(read_file)?,
            _keepalive: keepalive,
            buffer: String::new(),
        })
    }

    pub async fn read_messages(&mut self) -> Vec<ControlMessage> {
        loop {
            let messages = self.drain_buffer_messages();
            if !messages.is_empty() {
                return messages;
            }

            let mut guard = match self.read_file.readable_mut().await {
                Ok(guard) => guard,
                Err(_) => return Vec::new(),
            };

            match guard.try_io(|inner| read_available(inner.get_mut())) {
                Ok(Ok(chunk)) => {
                    if chunk.is_empty() {
                        return Vec::new();
                    }
                    self.buffer.push_str(&chunk);
                }
                Ok(Err(_)) => return Vec::new(),
                Err(_) => continue,
            }
        }
    }

    fn drain_buffer_messages(&mut self) -> Vec<ControlMessage> {
        let mut messages = Vec::new();
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

fn read_available(file: &mut File) -> IoResult<String> {
    let mut bytes = Vec::new();
    let mut tmp = [0u8; 4096];
    loop {
        match file.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => bytes.extend_from_slice(&tmp[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if bytes.is_empty() {
                    return Err(e);
                }
                break;
            }
            Err(e) => return Err(e),
        }
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
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
        .map_err(|e| {
            anyhow!(
                "desktop control channel is not ready: {} ({e})",
                path.display()
            )
        })?;
    use std::io::Write;
    let mut file = file;
    file.write_all(payload.as_bytes())?;
    Ok(())
}
