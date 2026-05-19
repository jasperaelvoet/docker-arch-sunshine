use anyhow::{Context, Result};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

use crate::paths::{desktop_user, home_dir};

pub struct ShellSession {
    pub parser: Arc<Mutex<vt100::Parser>>,
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    pub rows: u16,
    pub cols: u16,
}

impl ShellSession {
    pub fn spawn(rows: u16, cols: u16, env: &BTreeMap<String, String>) -> Result<Self> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("opening pty")?;

        let mut cmd = CommandBuilder::new("runuser");
        cmd.arg("-u");
        cmd.arg(desktop_user());
        cmd.arg("--");
        cmd.arg("env");
        for (k, v) in env {
            cmd.arg(format!("{k}={v}"));
        }
        cmd.arg("/bin/bash");
        cmd.arg("--login");
        cmd.cwd(home_dir());

        let child = pair.slave.spawn_command(cmd).context("spawning bash")?;
        drop(pair.slave);

        let parser = Arc::new(Mutex::new(vt100::Parser::new(rows, cols, 4000)));
        let reader = pair
            .master
            .try_clone_reader()
            .context("cloning pty reader")?;

        let writer = pair
            .master
            .take_writer()
            .context("taking pty writer")?;

        {
            let parser = parser.clone();
            std::thread::Builder::new()
                .name("pty-reader".into())
                .spawn(move || {
                    let mut reader = reader;
                    let mut buf = [0u8; 8192];
                    loop {
                        match reader.read(&mut buf) {
                            Ok(0) => break,
                            Ok(n) => {
                                if let Ok(mut p) = parser.lock() {
                                    p.process(&buf[..n]);
                                }
                            }
                            Err(_) => break,
                        }
                    }
                })
                .ok();
        }

        Ok(Self {
            parser,
            writer,
            master: pair.master,
            child,
            rows,
            cols,
        })
    }

    pub fn send_key(&mut self, key: KeyEvent) {
        let bytes = key_to_bytes(key);
        if !bytes.is_empty() {
            let _ = self.writer.write_all(&bytes);
            let _ = self.writer.flush();
        }
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        if rows == self.rows && cols == self.cols {
            return;
        }
        let _ = self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        });
        if let Ok(mut p) = self.parser.lock() {
            p.set_size(rows, cols);
        }
        self.rows = rows;
        self.cols = cols;
    }

    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for ShellSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn key_to_bytes(key: KeyEvent) -> Vec<u8> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let mut out: Vec<u8> = Vec::new();
    if alt {
        out.push(0x1b);
    }
    match key.code {
        KeyCode::Char(c) => {
            if ctrl {
                let b = c.to_ascii_lowercase() as u8;
                if (b'a'..=b'z').contains(&b) {
                    out.push(b - b'a' + 1);
                } else if c == ' ' {
                    out.push(0);
                } else if c == '\\' {
                    out.push(0x1c);
                } else if c == ']' {
                    out.push(0x1d);
                } else if c == '^' {
                    out.push(0x1e);
                } else if c == '_' {
                    out.push(0x1f);
                } else {
                    out.push(b);
                }
            } else {
                let mut tmp = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut tmp).as_bytes());
            }
        }
        KeyCode::Enter => out.push(b'\r'),
        KeyCode::Tab => out.push(b'\t'),
        KeyCode::BackTab => out.extend_from_slice(b"\x1b[Z"),
        KeyCode::Backspace => out.push(0x7f),
        KeyCode::Esc => out.push(0x1b),
        KeyCode::Left => out.extend_from_slice(b"\x1b[D"),
        KeyCode::Right => out.extend_from_slice(b"\x1b[C"),
        KeyCode::Up => out.extend_from_slice(b"\x1b[A"),
        KeyCode::Down => out.extend_from_slice(b"\x1b[B"),
        KeyCode::Home => out.extend_from_slice(b"\x1b[H"),
        KeyCode::End => out.extend_from_slice(b"\x1b[F"),
        KeyCode::PageUp => out.extend_from_slice(b"\x1b[5~"),
        KeyCode::PageDown => out.extend_from_slice(b"\x1b[6~"),
        KeyCode::Insert => out.extend_from_slice(b"\x1b[2~"),
        KeyCode::Delete => out.extend_from_slice(b"\x1b[3~"),
        KeyCode::F(n) => {
            let seq: &[u8] = match n {
                1 => b"\x1bOP",
                2 => b"\x1bOQ",
                3 => b"\x1bOR",
                4 => b"\x1bOS",
                5 => b"\x1b[15~",
                6 => b"\x1b[17~",
                7 => b"\x1b[18~",
                8 => b"\x1b[19~",
                9 => b"\x1b[20~",
                10 => b"\x1b[21~",
                11 => b"\x1b[23~",
                12 => b"\x1b[24~",
                _ => b"",
            };
            out.extend_from_slice(seq);
        }
        _ => {}
    }
    out
}
