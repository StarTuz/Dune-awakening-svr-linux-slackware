use anyhow::{Context, Result};
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

use crate::config::Config;

fn script_path(cfg: &Config) -> std::path::PathBuf {
    cfg.scripts_dir.join("world-capsules.sh")
}

fn build_command(cfg: &Config, args: &[String]) -> Command {
    let mut cmd = Command::new("bash");
    cmd.arg(script_path(cfg));
    cmd.args(args);
    cmd
}

pub async fn inventory(cfg: &Config) -> Result<String> {
    let output = build_command(cfg, &[String::from("inventory")])
        .output()
        .await
        .with_context(|| format!("failed to run {}", script_path(cfg).display()))?;

    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if !output.status.success() {
        anyhow::bail!("{} failed:\n{}", script_path(cfg).display(), text);
    }
    Ok(text)
}

pub async fn run_stream(cfg: &Config, args: &[String]) -> Result<()> {
    let script = script_path(cfg);
    let mut child = build_command(cfg, args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| anyhow::anyhow!("failed to spawn {}: {}", script.display(), e))?;

    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take().expect("stderr piped");

    let stdout_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            println!("{}", line);
        }
    });
    let stderr_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            eprintln!("{}", line);
        }
    });

    let status = child.wait().await?;
    let _ = stdout_task.await;
    let _ = stderr_task.await;

    if !status.success() {
        anyhow::bail!("{} exited with status {}", script.display(), status);
    }
    Ok(())
}
