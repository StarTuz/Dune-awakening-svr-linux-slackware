use anyhow::{Context, Result};
use std::process::Stdio;
use tokio::process::Command;

/// Run `sudo kubectl <args>` and return stdout as a String.
pub async fn run(args: &[&str]) -> Result<String> {
    let output = Command::new("sudo")
        .arg("kubectl")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("failed to spawn kubectl")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("kubectl {}: {}", args.join(" "), stderr.trim());
    }
    Ok(String::from_utf8(output.stdout)?)
}

/// Run `sudo kubectl <args> -o json` and return parsed JSON.
pub async fn get_json(args: &[&str]) -> Result<serde_json::Value> {
    let mut full: Vec<&str> = args.to_vec();
    full.extend_from_slice(&["-o", "json"]);
    let out = run(&full).await?;
    serde_json::from_str(&out).context("failed to parse kubectl JSON output")
}
