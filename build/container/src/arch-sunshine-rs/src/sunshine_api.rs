use anyhow::Result;
use std::time::Duration;

use crate::paths::log_dir;

pub fn disconnect(user: &str, password: &str) -> Result<()> {
    let urls = [
        "https://127.0.0.1:47984/api/apps/close",
        "https://127.0.0.1:47990/api/apps/close",
        "http://127.0.0.1:47989/api/apps/close",
    ];

    let client = reqwest::blocking::Client::builder()
        .danger_accept_invalid_certs(true)
        .timeout(Duration::from_secs(3))
        .connect_timeout(Duration::from_secs(1))
        .build()?;

    let mut log_lines = Vec::with_capacity(urls.len());

    for url in urls {
        let request = client
            .post(url)
            .basic_auth(user, Some(password))
            .header("Content-Type", "application/json")
            .body("{}");
        match request.send() {
            Ok(resp) => {
                let status = resp.status();
                log_lines.push(format!("{url} rc=0 response={status}"));
                if status.as_u16() == 200 {
                    break;
                }
            }
            Err(err) => {
                log_lines.push(format!("{url} rc=err response={err}"));
            }
        }
    }

    let log_path = log_dir().join("disconnect.log");
    if let Some(parent) = log_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut handle) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        use std::io::Write;
        let _ = writeln!(handle, "{}", log_lines.join("\n"));
    }
    Ok(())
}
