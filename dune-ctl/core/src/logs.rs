use anyhow::Result;
use std::process::Stdio;

use crate::{config::Config, kubectl};

pub async fn resolve_pod(cfg: &Config, target: &str) -> Result<String> {
    let search = match target.to_ascii_lowercase().as_str() {
        "gateway" => "gateway",
        "director" => "director",
        "postgres" | "db" => "db-dbdepl-sts",
        "rabbitmq" | "rmq" => "rabbitmq",
        "filebrowser" => "filebrowser",
        "text-router" | "textrouter" => "text-router",
        _ => {
            // Map name (e.g. "Survival_1") → slug ("survival-1") → pod substring search
            let slug = target.to_ascii_lowercase().replace('_', "-");
            return find_pod(cfg, &slug).await;
        }
    };
    find_pod(cfg, search).await
}

async fn find_pod(cfg: &Config, needle: &str) -> Result<String> {
    let out = kubectl::run(&[
        "get",
        "pods",
        "-n",
        &cfg.namespace,
        "--no-headers",
        "-o",
        "custom-columns=NAME:.metadata.name",
    ])
    .await?;
    out.lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && line.contains(needle))
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("no pod matching '{}' in {}", needle, cfg.namespace))
}

/// Fetch the last `lines` log lines from a target (map name, infra alias, or pod fragment).
pub async fn tail(cfg: &Config, target: &str, lines: usize) -> Result<Vec<String>> {
    let pod = resolve_pod(cfg, target).await?;
    let tail_str = lines.to_string();
    let out = kubectl::run(&["logs", "-n", &cfg.namespace, "--tail", &tail_str, &pod]).await?;
    Ok(out.lines().map(str::to_string).collect())
}

/// Stream logs to stdout until the process exits or Ctrl-C.
pub async fn stream(cfg: &Config, target: &str, tail_lines: usize) -> Result<()> {
    use tokio::io::{AsyncBufReadExt, BufReader};

    let pod = resolve_pod(cfg, target).await?;
    let tail_str = tail_lines.to_string();

    let mut child = tokio::process::Command::new("sudo")
        .arg("-n")
        .arg("kubectl")
        .args(["logs", "-n", &cfg.namespace, "-f", "--tail", &tail_str, &pod])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| anyhow::anyhow!("failed to spawn kubectl: {}", e))?;

    let stdout = child.stdout.take().expect("stdout piped");
    let mut reader = BufReader::new(stdout).lines();
    while let Some(line) = reader.next_line().await? {
        println!("{}", line);
    }
    child.wait().await?;
    Ok(())
}
