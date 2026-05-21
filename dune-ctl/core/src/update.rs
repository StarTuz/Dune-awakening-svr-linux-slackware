use anyhow::Result;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

use crate::{backup, battlegroup, config::Config, gateway, health::HealthSnapshot};

#[derive(Debug, Clone, Copy, Default)]
pub struct UpdateOptions {
    pub start_after: bool,
}

/// Run the full update pipeline via scripts/update.sh:
///   steamcmd validate → funcom-patches → battlegroup update → gateway patch
///
/// Returns combined stdout+stderr for display.
pub async fn run(cfg: &Config) -> Result<String> {
    run_with_options(cfg, UpdateOptions::default()).await
}

pub async fn run_with_options(cfg: &Config, options: UpdateOptions) -> Result<String> {
    let script = cfg.scripts_dir.join("update.sh");
    let mut cmd = tokio::process::Command::new(&script);
    cmd.args(["--bg", &cfg.battlegroup]);
    if options.start_after {
        cmd.arg("--start-after");
    }

    let output = cmd
        .output()
        .await
        .map_err(|e| anyhow::anyhow!("failed to run {}: {}", script.display(), e))?;

    let out = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    if !output.status.success() {
        anyhow::bail!("update.sh failed:\n{}", out);
    }
    Ok(out)
}

pub async fn run_streamed(
    cfg: &Config,
    options: UpdateOptions,
    tx: tokio::sync::mpsc::UnboundedSender<String>,
) -> Result<()> {
    if cfg.backup_environment == "live" && cfg.has_capsule() {
        return run_live_capsule_streamed(cfg, options, tx).await;
    }

    run_legacy_streamed(cfg, options, tx).await
}

async fn run_legacy_streamed(
    cfg: &Config,
    options: UpdateOptions,
    tx: tokio::sync::mpsc::UnboundedSender<String>,
) -> Result<()> {
    let script = cfg.scripts_dir.join("update.sh");
    let mut cmd = tokio::process::Command::new(&script);
    cmd.args(["--bg", &cfg.battlegroup]);
    if options.start_after {
        cmd.arg("--start-after");
    }
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| anyhow::anyhow!("failed to spawn {}: {}", script.display(), e))?;

    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take().expect("stderr piped");
    let tx2 = tx.clone();

    let stdout_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let _ = tx.send(line);
        }
    });
    let stderr_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let _ = tx2.send(format!("[err] {}", line));
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

async fn run_live_capsule_streamed(
    cfg: &Config,
    _options: UpdateOptions,
    tx: tokio::sync::mpsc::UnboundedSender<String>,
) -> Result<()> {
    send(&tx, "live capsule update selected");
    send(&tx, "step 1/8: backup");
    backup::run_streamed(cfg, false, None, tx.clone()).await?;

    send(&tx, "step 2/8: install/validate live package");
    run_capsule_command(
        cfg,
        &["package", "install", "--env", "live", "--app-id", "4754530"],
        tx.clone(),
    )
    .await?;

    send(&tx, "step 3/8: import live package images");
    run_capsule_command(
        cfg,
        &["images", "load", "--env", "live", "--app-id", "4754530"],
        tx.clone(),
    )
    .await?;

    send(&tx, "step 4/8: refresh capsule metadata");
    run_capsule_command(
        cfg,
        &[
            "refresh",
            "--env",
            "live",
            "--world-id",
            &cfg.battlegroup,
            "--app-id",
            "4754530",
        ],
        tx.clone(),
    )
    .await?;

    send(&tx, "step 5/8: apply refreshed capsule");
    run_capsule_command(
        cfg,
        &[
            "activate",
            "--env",
            "live",
            "--world-id",
            &cfg.battlegroup,
            "--apply",
            "--force",
        ],
        tx.clone(),
    )
    .await?;

    send(&tx, "step 6/8: start battlegroup");
    battlegroup::start(cfg).await?;

    send(&tx, "step 7/8: apply gateway patch");
    patch_gateway_with_retry(cfg, tx.clone()).await?;

    send(&tx, "step 8/8: wait for preflight-ready state");
    wait_ready(cfg, tx.clone()).await?;

    send(&tx, "live capsule update complete");
    Ok(())
}

async fn run_capsule_command(
    cfg: &Config,
    args: &[&str],
    tx: tokio::sync::mpsc::UnboundedSender<String>,
) -> Result<()> {
    let script = cfg.scripts_dir.join("world-capsules.sh");
    let mut cmd = Command::new("bash");
    cmd.arg(&script).args(args);
    stream_command(cmd, &script.display().to_string(), tx).await
}

async fn stream_command(
    mut cmd: Command,
    label: &str,
    tx: tokio::sync::mpsc::UnboundedSender<String>,
) -> Result<()> {
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| anyhow::anyhow!("failed to spawn {}: {}", label, e))?;

    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take().expect("stderr piped");
    let tx2 = tx.clone();

    let stdout_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let _ = tx.send(line);
        }
    });
    let stderr_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let _ = tx2.send(format!("[err] {}", line));
        }
    });

    let status = child.wait().await?;
    let _ = stdout_task.await;
    let _ = stderr_task.await;

    if !status.success() {
        anyhow::bail!("{} exited with status {}", label, status);
    }
    Ok(())
}

async fn patch_gateway_with_retry(
    cfg: &Config,
    tx: tokio::sync::mpsc::UnboundedSender<String>,
) -> Result<()> {
    let mut last_error = None;
    for attempt in 1..=24 {
        match gateway::patch(cfg).await {
            Ok(true) => {
                send(&tx, "gateway patch applied");
                return Ok(());
            }
            Ok(false) => {
                send(&tx, "gateway patch already present");
                return Ok(());
            }
            Err(e) => {
                last_error = Some(e);
                send(
                    &tx,
                    format!("gateway patch waiting for deployment ({attempt}/24)"),
                );
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        }
    }
    Err(last_error
        .unwrap_or_else(|| anyhow::anyhow!("gateway patch failed"))
        .context("gateway patch did not become available"))
}

async fn wait_ready(cfg: &Config, tx: tokio::sync::mpsc::UnboundedSender<String>) -> Result<()> {
    for attempt in 1..=60 {
        if let Ok(snap) = HealthSnapshot::collect(cfg).await {
            let gateway_ok = snap.gateway.as_ref().map(|gw| gw.patched).unwrap_or(false);
            let sietch_ok = snap.sietches.iter().any(|s| {
                s.primary
                    && s.phase == "Running"
                    && s.ready_replicas.unwrap_or(0) >= 1
                    && s.target_replicas.unwrap_or(1) >= 1
            });
            if !gateway_ok {
                let _ = gateway::patch(cfg).await;
            }
            if !snap.battlegroup_stopped
                && snap.battlegroup_phase == "Healthy"
                && gateway_ok
                && sietch_ok
            {
                send(&tx, "preflight-ready state reached");
                return Ok(());
            }
            send(
                &tx,
                format!(
                    "waiting: phase={} stopped={} gateway={} primary_sietch={} ({attempt}/60)",
                    snap.battlegroup_phase, snap.battlegroup_stopped, gateway_ok, sietch_ok
                ),
            );
        } else {
            send(&tx, format!("waiting for health snapshot ({attempt}/60)"));
        }
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
    }
    anyhow::bail!("timed out waiting for preflight-ready state")
}

fn send(tx: &tokio::sync::mpsc::UnboundedSender<String>, line: impl Into<String>) {
    let _ = tx.send(line.into());
}
