use anyhow::{Context, Result};
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

use crate::{config::Config, kubectl};

const BACKUP_ROOT: &str = "/srv/backups/dune";
const FUNCOM_DB_DUMPS: &str = "/funcom/artifacts/database-dumps";

pub const CRON_MARKER: &str = "# dune-ctl-backup";

pub struct ScheduleInfo {
    pub cron: String,
    pub keep: usize,
}

/// Read the installed backup schedule from the user's crontab.
pub fn read_schedule() -> Option<ScheduleInfo> {
    let out = std::process::Command::new("crontab")
        .arg("-l")
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().find(|l| l.ends_with(CRON_MARKER))?;
    let fields: Vec<&str> = line.split_whitespace().collect();
    if fields.len() < 5 {
        return None;
    }
    let cron = fields[..5].join(" ");
    let keep = fields
        .windows(2)
        .find(|w| w[0] == "--keep")
        .and_then(|w| w[1].parse::<usize>().ok())
        .unwrap_or(14);
    Some(ScheduleInfo { cron, keep })
}

/// Install or update the backup schedule in the user's crontab.
pub fn write_schedule(battlegroup: &str, binary_path: &str, cron: &str, keep: usize) -> Result<()> {
    let current = read_crontab()?;
    let stripped = strip_schedule_line(&current);
    let entry = format!(
        "{}  DUNE_CTL_WORLD={} {} backup run --keep {}  {}",
        cron, battlegroup, binary_path, keep, CRON_MARKER,
    );
    let new_crontab = if stripped.is_empty() {
        format!("{}\n", entry)
    } else {
        format!("{}\n{}\n", stripped, entry)
    };
    write_crontab(&new_crontab)
}

/// Remove the installed backup schedule from the user's crontab.
pub fn remove_schedule() -> Result<()> {
    let current = read_crontab()?;
    let stripped = strip_schedule_line(&current);
    let new = if stripped.is_empty() {
        String::new()
    } else {
        format!("{}\n", stripped)
    };
    write_crontab(&new)
}

/// Delete a backup bundle directory.
pub async fn delete_bundle(path: &std::path::Path) -> Result<()> {
    tokio::fs::remove_dir_all(path)
        .await
        .with_context(|| format!("failed to delete bundle {}", path.display()))
}

fn strip_schedule_line(crontab: &str) -> String {
    crontab
        .lines()
        .filter(|l| !l.ends_with(CRON_MARKER))
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end()
        .to_string()
}

fn read_crontab() -> Result<String> {
    let out = std::process::Command::new("crontab")
        .arg("-l")
        .output()
        .context("failed to run crontab -l")?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Ok(String::new())
    }
}

fn write_crontab(content: &str) -> Result<()> {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = std::process::Command::new("crontab")
        .arg("-")
        .stdin(Stdio::piped())
        .spawn()
        .context("failed to spawn crontab -")?;
    child
        .stdin
        .as_mut()
        .expect("stdin piped")
        .write_all(content.as_bytes())
        .context("failed to write crontab")?;
    let status = child.wait()?;
    if !status.success() {
        anyhow::bail!("crontab - exited with {}", status);
    }
    Ok(())
}

pub struct BackupEntry {
    pub environment: String,
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
    cmd.args(["--env", &cfg.backup_environment]);
    if skip_db {
        cmd.arg("--skip-db");
    }
    if let Some(n) = name {
        cmd.args(["--name", n]);
    }
    stream_command(cmd, &script.display().to_string()).await
}

/// Restore a database backup. `bundle` is a timestamp (e.g. "20260517-142305") or
/// a full path to a bundle directory under /srv/backups/dune/<env>/<bg>/.
/// Stages the .backup file from bundle/database/ into the funcom artifacts dir,
/// then runs battlegroup.sh import.
pub async fn restore(cfg: &Config, bundle: &str) -> Result<()> {
    ensure_battlegroup_stopped(cfg).await?;

    let bundle_path = resolve_bundle(cfg, bundle).await?;
    ensure_bundle_environment(cfg, &bundle_path).await?;

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
    stream_command_with_stdin(cmd, &battlegroup_script.display().to_string(), "yes\n").await
}

async fn ensure_battlegroup_stopped(cfg: &Config) -> Result<()> {
    let bg =
        kubectl::get_json(&["get", "battlegroup", &cfg.battlegroup, "-n", &cfg.namespace]).await?;
    if bg
        .pointer("/spec/stop")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        return Ok(());
    }
    anyhow::bail!(
        "refusing to restore while battlegroup {} is not stopped; run `dune-ctl battlegroup stop` first",
        cfg.battlegroup
    )
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
    cmd.args(["--env", &cfg.backup_environment]);
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
    let mut entries: Vec<BackupEntry> = Vec::new();
    read_backup_entries(
        &backup_env_dir(&cfg.backup_environment, &cfg.battlegroup),
        &cfg.backup_environment,
        &mut entries,
    )
    .await?;

    let legacy_dir = backup_legacy_dir(&cfg.battlegroup);
    if tokio::fs::try_exists(&legacy_dir).await.unwrap_or(false) {
        read_backup_entries(&legacy_dir, "legacy", &mut entries).await?;
    }

    // Timestamps are YYYYMMDD-HHMMSS — lexicographic order = newest first
    entries.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    Ok(entries)
}

async fn read_backup_entries(
    bg_dir: &PathBuf,
    environment: &str,
    entries: &mut Vec<BackupEntry>,
) -> Result<()> {
    let mut dir = match tokio::fs::read_dir(bg_dir).await {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => anyhow::bail!("cannot read {}: {}", bg_dir.display(), e),
    };

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
        let has_db = has_backup_file(&path.join("database")).await;
        let size_bytes = dir_size_bytes(&path).await;
        entries.push(BackupEntry {
            environment: manifest_environment(&path)
                .await
                .unwrap_or_else(|| environment.to_string()),
            timestamp: name.to_string(),
            path,
            has_db,
            size_bytes,
        });
    }
    Ok(())
}

async fn has_backup_file(db_dir: &PathBuf) -> bool {
    let Ok(mut dir) = tokio::fs::read_dir(db_dir).await else {
        return false;
    };
    while let Ok(Some(entry)) = dir.next_entry().await {
        if entry.path().extension().and_then(|e| e.to_str()) == Some("backup") {
            return true;
        }
    }
    false
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

async fn resolve_bundle(cfg: &Config, bundle: &str) -> Result<PathBuf> {
    if bundle.contains('/') {
        return Ok(PathBuf::from(bundle));
    }

    let env_path = backup_env_dir(&cfg.backup_environment, &cfg.battlegroup).join(bundle);
    if tokio::fs::try_exists(&env_path).await.unwrap_or(false) {
        return Ok(env_path);
    }

    let legacy_path = backup_legacy_dir(&cfg.battlegroup).join(bundle);
    if tokio::fs::try_exists(&legacy_path).await.unwrap_or(false) {
        return Ok(legacy_path);
    }

    anyhow::bail!(
        "backup bundle '{}' not found for environment '{}' and battlegroup {}",
        bundle,
        cfg.backup_environment,
        cfg.battlegroup
    )
}

fn backup_env_dir(environment: &str, battlegroup: &str) -> PathBuf {
    PathBuf::from(BACKUP_ROOT)
        .join(environment)
        .join(battlegroup)
}

fn backup_legacy_dir(battlegroup: &str) -> PathBuf {
    PathBuf::from(BACKUP_ROOT).join(battlegroup)
}

async fn ensure_bundle_environment(cfg: &Config, bundle_path: &PathBuf) -> Result<()> {
    let Some(environment) = manifest_environment(bundle_path).await else {
        anyhow::bail!(
            "refusing to restore {} because MANIFEST.txt has no environment marker",
            bundle_path.display()
        );
    };
    if environment == cfg.backup_environment {
        return Ok(());
    }
    anyhow::bail!(
        "refusing to restore {}: backup environment '{}' does not match current world environment '{}'",
        bundle_path.display(),
        environment,
        cfg.backup_environment
    )
}

async fn manifest_environment(bundle_path: &PathBuf) -> Option<String> {
    let text = tokio::fs::read_to_string(bundle_path.join("MANIFEST.txt"))
        .await
        .ok()?;
    parse_manifest_environment(&text)
}

fn parse_manifest_environment(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        line.trim()
            .strip_prefix("environment=")
            .map(str::trim)
            .map(|value| value.trim_matches('"').to_ascii_lowercase())
            .filter(|value| value == "ptc" || value == "live")
    })
}

#[allow(dead_code)]
fn environment_from_path(path: &std::path::Path) -> Option<String> {
    let root = std::path::Path::new(BACKUP_ROOT);
    let rel = path.strip_prefix(root).ok()?;
    let env = rel.components().next()?.as_os_str().to_str()?;
    if env == "ptc" || env == "live" {
        Some(env.to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_manifest_environment() {
        assert_eq!(
            parse_manifest_environment("created_utc=20260519-000000\nenvironment=ptc\n").as_deref(),
            Some("ptc")
        );
        assert_eq!(
            parse_manifest_environment("environment=\"live\"").as_deref(),
            Some("live")
        );
    }

    #[test]
    fn rejects_missing_or_invalid_manifest_environment() {
        assert_eq!(
            parse_manifest_environment("created_utc=20260519-000000"),
            None
        );
        assert_eq!(parse_manifest_environment("environment=dev"), None);
    }

    #[test]
    fn builds_environment_scoped_backup_path() {
        assert_eq!(
            backup_env_dir("ptc", "bg").to_string_lossy().as_ref(),
            "/srv/backups/dune/ptc/bg"
        );
        assert_eq!(
            backup_legacy_dir("bg").to_string_lossy().as_ref(),
            "/srv/backups/dune/bg"
        );
    }

    #[tokio::test]
    async fn restore_environment_guard_rejects_mismatch() {
        let dir = std::env::temp_dir().join(format!("dune-backup-env-{}", std::process::id()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(dir.join("MANIFEST.txt"), "environment=live\n")
            .await
            .unwrap();

        let cfg = Config {
            battlegroup: "bg".to_string(),
            namespace: "funcom-seabass-bg".to_string(),
            title: None,
            backup_environment: "ptc".to_string(),
            world_spec: None,
            explicit_target: false,
            scripts_dir: PathBuf::from("/tmp"),
        };

        let err = ensure_bundle_environment(&cfg, &dir).await.unwrap_err();
        assert!(err.to_string().contains("does not match"));
        let _ = tokio::fs::remove_dir_all(&dir).await;
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

async fn stream_command_with_stdin(mut cmd: Command, label: &str, input: &str) -> Result<()> {
    use std::process::Stdio;
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| anyhow::anyhow!("failed to spawn {}: {}", label, e))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(input.as_bytes()).await?;
    }

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
