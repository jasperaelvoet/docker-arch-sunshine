use anyhow::Result;
use std::path::PathBuf;
use tokio::sync::mpsc;

use crate::tui::state::LogSource;

pub struct LogLine {
    pub source: LogSource,
    pub line: String,
}

pub fn spawn(tx: mpsc::UnboundedSender<LogLine>) -> Result<()> {
    let paths: Vec<(LogSource, PathBuf)> = LogSource::ALL
        .iter()
        .map(|s| (*s, s.path()))
        .collect();

    tokio::spawn(async move {
        for (_, path) in &paths {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if !path.exists() {
                let _ = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path);
            }
        }

        let path_strs: Vec<String> = paths
            .iter()
            .map(|(_, p)| p.to_string_lossy().into_owned())
            .collect();
        let mut lookup = std::collections::HashMap::new();
        for ((src, _), s) in paths.iter().zip(path_strs.iter()) {
            lookup.insert(s.clone(), *src);
        }

        let path_refs: Vec<&str> = path_strs.iter().map(|s| s.as_str()).collect();

        let mut lines = match linemux::MuxedLines::new() {
            Ok(l) => l,
            Err(_) => return,
        };
        for path in &path_refs {
            let _ = lines.add_file(path).await;
        }

        loop {
            match lines.next_line().await {
                Ok(Some(line)) => {
                    let source = lookup
                        .get(&line.source().to_string_lossy().into_owned())
                        .copied();
                    if let Some(src) = source {
                        if tx
                            .send(LogLine {
                                source: src,
                                line: line.line().to_string(),
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                }
                Ok(None) => break,
                Err(_) => {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }
            }
        }
    });
    Ok(())
}
