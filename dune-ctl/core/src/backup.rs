use anyhow::{Context, Result};
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

use crate::config::Config;

const BACKUP_ROOT: &str = "/srv/backups/dune";
const FUNCOM_DB_DUMPS: &str = "/funcom/artifacts/database-dumps";

pub struct BackupEntry {
    pub timestamp: String,
    pub path: PathBuf,
    pub has_db: bool,
    pub size_bytes: u64,
}

pub fn format_size(bytes: u64) -> String {
    const KB: u64 = 1_024;
    const MB: u64 = 1_024 * KB;
    const GB: u64 = 1_024 * MB;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

/// Run dune-backup.sh, streaming its output to stdout.
pub async fn run(cfg: &Config, skip_db: bool, name: Option<&str>) -> Result<()> {
    let script = cfg.scripts_dir.join("dune-backup.sh");
    let mut cmd = Command::new("bash");
    cmd.arg(&script);
    cmd.args(["--bg", &cfg.battlegroup]);
    if skip_db {
        cmd.arg("--skip-db");
    }
    if let Some(n) = name {
        cmd.args(["--name", n]);
    }
    stream_command(cmd, &script.display().to_string()).await
}

/// Restore a database backup. `bundle` is a timestamp (e.g. "20260517-142305") or
/// a full path to a bundle directory under /srv/backups/dune/<bg>/.
/// Stages the .backup file from bundle/database/ into the funcom artifacts dir,
/// then runs battlegroup.sh import.
pub async fn restore(cfg: &Config, bundle: &str) -> Result<()> {
    let bundle_path = resolve_bundle(cfg, bundle);

    let db_dir = bundle_path.join("database");
    let backup_file = find_backup_file(&db_dir).await?;
    let backup_name = backup_file
        .file_name()
        .and_then(|n| n.to_str())
        .context("invalid backup filename")?
        .to_string();

    let artifacts_dir = format!("{}/{}", FUNCOM_DB_DUMPS, cfg.battlegroup);
    let dest = format!("{}/{}", artifacts_dir, backup_name);

    // Ensure the artifacts directory exists
    let mkdir = Command::new("sudo")
        .args(["-n", "mkdir", "-p", &artifacts_dir])
        .output()
        .await
        .context("failed to create artifacts dir")?;
    if !mkdir.status.success() {
        anyhow::bail!(
            "mkdir -p {} failed: {}",
            artifacts_dir,
            String::from_utf8_lossy(&mkdir.stderr)
        );
    }

    // Stage the backup file
    let src = backup_file.to_string_lossy().into_owned();
    let copy = Command::new("sudo")
        .args(["-n", "cp", &src, &dest])
        .output()
        .await
        .context("failed to stage backup")?;
    if !copy.status.success() {
        anyhow::bail!(
            "failed to copy backup to {}: {}",
            dest,
            String::from_utf8_lossy(&copy.stderr)
        );
    }
    println!("Staged {} → {}", backup_file.display(), dest);

    // Run battlegroup.sh import <name>
    let battlegroup_script = cfg.repo_root().join("server/scripts/battlegroup.sh");
    let mut cmd = Command::new("bash");
    cmd.arg(&battlegroup_script).args(["import", &backup_name]);
    stream_command(cmd, &battlegroup_script.display().to_string()).await
}

/// Prune oldest bundles, keeping at most `keep` most recent.
/// Returns removed paths. `keep == 0` is a no-op.
pub async fn prune(cfg: &Config, keep: usize) -> Result<Vec<PathBuf>> {
    if keep == 0 {
        return Ok(Vec::new());
    }
    let entries = list(cfg).await?;
    if entries.len() <= keep {
        return Ok(Vec::new());
    }
    let to_remove: Vec<PathBuf> = entries[keep..].iter().map(|e| e.path.clone()).collect();
    for path in &to_remove {
        tokio::fs::remove_dir_all(path)
            .await
            .with_context(|| format!("failed to remove bundle {}", path.display()))?;
    }
    Ok(to_remove)
}

/// Run dune-backup.sh, sending each output line to `tx` instead of stdout.
/// Stderr lines are prefixed with "[err] ". Used by the TUI backups tab.
pub async fn run_streamed(
    cfg: &Config,
    skip_db: bool,
    name: Option<&str>,
    tx: tokio::sync::mpsc::UnboundedSender<String>,
) -> Result<()> {
    use std::process::Stdio;
    use tokio::io::AsyncBufReadExt;

    let script = cfg.scripts_dir.join("dune-backup.sh");
    let mut cmd = Command::new("bash");
    cmd.arg(&script);
    cmd.args(["--bg", &cfg.battlegroup]);
    if skip_db {
        cmd.arg("--skip-db");
    }
    if let Some(n) = name {
        cmd.args(["--name", n]);
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

/// List backup bundles for the current battlegroup, newest first.
pub async fn list(cfg: &Config) -> Result<Vec<BackupEntry>> {
    let bg_dir = PathBuf::from(BACKUP_ROOT).join(&cfg.battlegroup);

    let mut dir = match tokio::fs::read_dir(&bg_dir).await {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => anyhow::bail!("cannot read {}: {}", bg_dir.display(), e),
    };

    let mut entries: Vec<BackupEntry> = Vec::new();
    while let Some(entry) = dir.next_entry().await? {
        let path = entry.path();
        let Ok(meta) = tokio::fs::metadata(&path).await else {
            continue;
        };
        if !meta.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let has_db = tokio::fs::try_exists(path.join("database"))
            .await
            .unwrap_or(false);
        let size_bytes = dir_size_bytes(&path).await;
        entries.push(BackupEntry {
            timestamp: name.to_string(),
            path,
            has_db,
            size_bytes,
        });
    }
    // Timestamps are YYYYMMDD-HHMMSS — lexicographic order = newest first
    entries.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    Ok(entries)
}

async fn dir_size_bytes(path: &PathBuf) -> u64 {
    let out = Command::new("du")
        .args(["-sb", &path.to_string_lossy().as_ref()])
        .output()
        .await;
    match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
            .split_whitespace()
            .next()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0),
        _ => 0,
    }
}

fn resolve_bundle(cfg: &Config, bundle: &str) -> PathBuf {
    if bundle.contains('/') {
        PathBuf::from(bundle)
    } else {
        PathBuf::from(BACKUP_ROOT)
            .join(&cfg.battlegroup)
            .join(bundle)
    }
}

async fn find_backup_file(db_dir: &PathBuf) -> Result<PathBuf> {
    let mut dir = tokio::fs::read_dir(db_dir)
        .await
        .with_context(|| format!("cannot read database dir: {}", db_dir.display()))?;
    while let Some(entry) = dir.next_entry().await? {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("backup") {
            return Ok(path);
        }
    }
    anyhow::bail!("no .backup file found in {}", db_dir.display())
}

async fn stream_command(mut cmd: Command, label: &str) -> Result<()> {
    use std::process::Stdio;
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| anyhow::anyhow!("failed to spawn {}: {}", label, e))?;

    // Stream stdout and stderr concurrently
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
        anyhow::bail!("{} exited with status {}", label, status);
    }
    Ok(())
}
