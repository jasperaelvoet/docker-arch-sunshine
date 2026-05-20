use anyhow::{anyhow, Context, Result};
use std::fs;
use std::time::Duration;

use crate::paths::{log_dir, sunshine_api_password_file, SUNSHINE_API_USER};

#[derive(Debug, Clone)]
pub struct Credentials {
    pub user: String,
    pub password: String,
}

pub fn resolve_credentials(user: Option<&str>, password: Option<&str>) -> Result<Credentials> {
    let user = user.filter(|value| !value.is_empty());
    let password = password.filter(|value| !value.is_empty());

    match (user, password) {
        (Some(user), Some(password)) => Ok(Credentials {
            user: user.to_string(),
            password: password.to_string(),
        }),
        (None, None) => {
            let path = sunshine_api_password_file();
            let password = fs::read_to_string(&path)
                .with_context(|| format!("reading Sunshine API password from {}", path.display()))?
                .trim()
                .to_string();
            if password.is_empty() {
                return Err(anyhow!("Sunshine API password file is empty: {}", path.display()));
            }
            Ok(Credentials {
                user: SUNSHINE_API_USER.to_string(),
                password,
            })
        }
        _ => Err(anyhow!(
            "both Sunshine API user and password must be provided, or neither"
        )),
    }
}

pub fn disconnect(user: Option<&str>, password: Option<&str>) -> Result<()> {
    let credentials = resolve_credentials(user, password)?;
    disconnect_with_credentials(&credentials)
}

pub fn disconnect_with_credentials(credentials: &Credentials) -> Result<()> {
    let urls = [
        "https://127.0.0.1:47990/api/apps/close",
        "http://127.0.0.1:47989/api/apps/close",
        "https://127.0.0.1:47984/api/apps/close",
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
            .basic_auth(&credentials.user, Some(&credentials.password))
            .header("Content-Type", "application/json")
            .body("{}");
        match request.send() {
            Ok(resp) => {
                let status = resp.status();
                log_lines.push(format!("{url} rc=0 response={status}"));
                if status.is_success() {
                    write_disconnect_log(&log_lines);
                    return Ok(());
                }
            }
            Err(err) => {
                log_lines.push(format!("{url} rc=err response={err}"));
            }
        }
    }

    write_disconnect_log(&log_lines);
    Err(anyhow!(
        "Sunshine disconnect API did not accept the close request:\n{}",
        log_lines.join("\n")
    ))
}

fn write_disconnect_log(log_lines: &[String]) {
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
}
