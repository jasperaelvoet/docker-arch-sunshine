use anyhow::Result;
use std::fs;
use std::io::Write;
use zbus::interface;
use zbus::ConnectionBuilder;

use crate::paths::log_dir;

#[derive(Clone)]
pub struct Credentials {
    pub user: String,
    pub password: String,
}

struct Shutdown {
    creds: Credentials,
}

#[interface(name = "org.kde.Shutdown")]
impl Shutdown {
    async fn logout(&self) {
        trigger_disconnect(self.creds.clone());
    }

    #[zbus(name = "logoutAndShutdown")]
    async fn logout_and_shutdown(&self) {
        trigger_disconnect(self.creds.clone());
    }

    #[zbus(name = "logoutAndReboot")]
    async fn logout_and_reboot(&self) {
        trigger_disconnect(self.creds.clone());
    }

    #[zbus(name = "saveSession")]
    async fn save_session(&self) {}
}

struct LogoutPrompt {
    creds: Credentials,
}

#[interface(name = "org.kde.LogoutPrompt")]
impl LogoutPrompt {
    #[zbus(name = "promptLogout")]
    async fn prompt_logout(&self) {
        trigger_disconnect(self.creds.clone());
    }

    #[zbus(name = "promptShutDown")]
    async fn prompt_shutdown(&self) {
        trigger_disconnect(self.creds.clone());
    }

    #[zbus(name = "promptReboot")]
    async fn prompt_reboot(&self) {
        trigger_disconnect(self.creds.clone());
    }

    #[zbus(name = "promptAll")]
    async fn prompt_all(&self) {
        trigger_disconnect(self.creds.clone());
    }
}

fn trigger_disconnect(creds: Credentials) {
    tokio::task::spawn_blocking(move || {
        if let Err(e) = crate::sunshine_api::disconnect(&creds.user, &creds.password) {
            log_failure(&e.to_string());
        }
    });
}

fn log_failure(msg: &str) {
    let log_path = log_dir().join("session-actions.log");
    if let Some(parent) = log_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(mut handle) = fs::OpenOptions::new().create(true).append(true).open(&log_path) {
        let _ = writeln!(handle, "disconnect failed: {msg}");
    }
}

pub async fn run(creds: Credentials) -> Result<()> {
    let _shutdown_conn = ConnectionBuilder::session()?
        .name("org.kde.Shutdown")?
        .serve_at("/Shutdown", Shutdown { creds: creds.clone() })?
        .build()
        .await?;

    let _logout_conn = ConnectionBuilder::session()?
        .name("org.kde.LogoutPrompt")?
        .serve_at(
            "/LogoutPrompt",
            LogoutPrompt {
                creds: creds.clone(),
            },
        )?
        .build()
        .await?;

    let mut sigterm =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    tokio::select! {
        _ = sigterm.recv() => {}
        _ = sigint.recv() => {}
    }
    Ok(())
}
