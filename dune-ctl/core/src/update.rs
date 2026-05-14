use anyhow::Result;

use crate::config::Config;

/// Run the full update pipeline via scripts/update.sh:
///   steamcmd validate → funcom-patches → battlegroup update → gateway patch
///
/// Returns combined stdout+stderr for display.
pub async fn run(cfg: &Config) -> Result<String> {
    let script = cfg.scripts_dir.join("update.sh");
    let output = tokio::process::Command::new(&script)
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
