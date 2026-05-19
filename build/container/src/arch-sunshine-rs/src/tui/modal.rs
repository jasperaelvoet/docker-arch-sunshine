use anyhow::Result;
use std::process::Command;

pub fn submit_pin(pin: &str) -> Result<()> {
    let trimmed = pin.trim();
    if trimmed.is_empty() {
        anyhow::bail!("PIN is empty");
    }
    let status = Command::new("/usr/local/bin/arch-sunshine-server")
        .args(["pin", trimmed])
        .status()?;
    if !status.success() {
        anyhow::bail!("PIN was not staged (exit {})", status);
    }
    Ok(())
}
