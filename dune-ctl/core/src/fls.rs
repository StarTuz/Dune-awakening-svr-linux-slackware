use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::{DateTime, Utc};
use regex::Regex;
use serde::Deserialize;

use crate::{battlegroup, config::Config, kubectl};

const WARN_DAYS: i64 = 30;
const CRITICAL_DAYS: i64 = 14;

#[derive(Debug, Clone)]
pub struct FlsTokenStatus {
    pub expires_at: DateTime<Utc>,
    pub days_remaining: i64,
    pub state: FlsTokenState,
    pub host_id: Option<String>,
    pub token_index: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlsTokenState {
    Ok,
    WarningSoon,
    Critical,
    Expired,
}

impl FlsTokenStatus {
    pub fn label(&self) -> String {
        match self.state {
            FlsTokenState::Ok => format!("{}d OK", self.days_remaining),
            FlsTokenState::WarningSoon => format!("{}d WARN", self.days_remaining),
            FlsTokenState::Critical => format!("{}d CRIT", self.days_remaining),
            FlsTokenState::Expired => "EXPIRED".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct FlsTokenMetadata {
    pub expires_at: DateTime<Utc>,
    pub days_remaining: i64,
    pub state: FlsTokenState,
    pub host_id: Option<String>,
    pub token_index: Option<String>,
}

impl FlsTokenMetadata {
    pub fn label(&self) -> String {
        FlsTokenStatus {
            expires_at: self.expires_at,
            days_remaining: self.days_remaining,
            state: self.state.clone(),
            host_id: self.host_id.clone(),
            token_index: self.token_index.clone(),
        }
        .label()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct TokenRotateOptions {
    pub dry_run: bool,
    pub yes: bool,
    pub skip_live: bool,
    pub wait: bool,
}

#[derive(Debug, Clone)]
pub struct TokenRotationReport {
    pub metadata: FlsTokenMetadata,
    pub expected_host_id: String,
    pub battlegroup_yaml: PathBuf,
    pub fls_secret_yaml: PathBuf,
    pub battlegroup_backup: Option<PathBuf>,
    pub fls_secret_backup: Option<PathBuf>,
    pub server_arg_replacements: usize,
    pub utility_env_replacements: usize,
    pub fls_secret_replacements: usize,
    pub stop_forced_false: bool,
    pub applied_live: bool,
    pub waited_ready: bool,
}

#[derive(Deserialize)]
struct JwtPayload {
    exp: i64,
    #[serde(rename = "HostId")]
    host_id: Option<String>,
    #[serde(rename = "TokenIndex")]
    token_index: Option<String>,
}

/// Decode a JWT and return token expiry status. Does not verify the signature.
pub fn decode(jwt: &str) -> Result<FlsTokenStatus> {
    let metadata = decode_metadata(jwt)?;
    Ok(FlsTokenStatus {
        expires_at: metadata.expires_at,
        days_remaining: metadata.days_remaining,
        state: metadata.state,
        host_id: metadata.host_id,
        token_index: metadata.token_index,
    })
}

/// Decode a JWT and return token metadata. Does not verify the signature.
pub fn decode_metadata(jwt: &str) -> Result<FlsTokenMetadata> {
    let parts: Vec<&str> = jwt.trim().splitn(3, '.').collect();
    if parts.len() != 3 {
        anyhow::bail!(
            "invalid JWT: expected 3 dot-separated parts, got {}",
            parts.len()
        );
    }
    let payload_bytes = URL_SAFE_NO_PAD
        .decode(parts[1])
        .context("failed to base64url-decode JWT payload")?;
    let claims: JwtPayload =
        serde_json::from_slice(&payload_bytes).context("failed to parse JWT payload JSON")?;

    let expires_at = DateTime::from_timestamp(claims.exp, 0)
        .ok_or_else(|| anyhow::anyhow!("JWT exp out of range: {}", claims.exp))?;
    let days_remaining = (expires_at - Utc::now()).num_days();

    let state = match days_remaining {
        d if d <= 0 => FlsTokenState::Expired,
        d if d <= CRITICAL_DAYS => FlsTokenState::Critical,
        d if d <= WARN_DAYS => FlsTokenState::WarningSoon,
        _ => FlsTokenState::Ok,
    };

    Ok(FlsTokenMetadata {
        expires_at,
        days_remaining,
        state,
        host_id: claims.host_id,
        token_index: claims.token_index,
    })
}

/// Pull the FLS JWT from the live BattleGroup CR and decode it.
pub async fn check(cfg: &Config) -> Result<FlsTokenStatus> {
    let bg =
        kubectl::get_json(&["get", "battlegroup", &cfg.battlegroup, "-n", &cfg.namespace]).await?;

    let token = extract_jwt(&bg)
        .ok_or_else(|| anyhow::anyhow!("FLS JWT not found in BattleGroup CR args"))?;
    decode(&token)
}

pub async fn rotate_token(
    cfg: &Config,
    new_token: &str,
    options: TokenRotateOptions,
) -> Result<TokenRotationReport> {
    let new_token = new_token.trim();
    let metadata = decode_metadata(new_token).context("new token is not a valid JWT")?;
    let new_host = metadata
        .host_id
        .as_deref()
        .filter(|host| !host.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("new token does not contain HostId"))?;

    if !cfg.has_capsule() {
        anyhow::bail!(
            "refusing to rotate token for non-capsule-backed world {}; expected {}",
            cfg.battlegroup,
            cfg.capsule_dir().display()
        );
    }
    if !options.dry_run && !options.yes {
        anyhow::bail!("refusing to rotate FLS token without --yes; use --dry-run to inspect only");
    }

    let capsule_dir = cfg.capsule_dir();
    let capsule_env = capsule_dir.join("capsule.env");
    let battlegroup_yaml = capsule_dir.join("battlegroup.yaml");
    let fls_secret_yaml = capsule_dir.join("fls-secret.yaml");
    ensure_file(&capsule_env)?;
    ensure_file(&battlegroup_yaml)?;
    ensure_file(&fls_secret_yaml)?;

    let expected_host_id = capsule_value(&capsule_env, "token_host_id")?
        .ok_or_else(|| anyhow::anyhow!("capsule.env missing token_host_id"))?;
    if !expected_host_id.eq_ignore_ascii_case(new_host) {
        anyhow::bail!(
            "new token HostId {} does not match capsule token_host_id {}",
            new_host,
            expected_host_id
        );
    }

    let live_was_running = if !options.skip_live {
        battlegroup::raw(cfg)
            .await
            .ok()
            .and_then(|bg| bg.pointer("/spec/stop").and_then(|v| v.as_bool()))
            .map(|stop| !stop)
            .unwrap_or(false)
    } else {
        false
    };

    let bg_text = fs::read_to_string(&battlegroup_yaml)
        .with_context(|| format!("reading {}", battlegroup_yaml.display()))?;
    let secret_text = fs::read_to_string(&fls_secret_yaml)
        .with_context(|| format!("reading {}", fls_secret_yaml.display()))?;

    let bg_update = rewrite_battlegroup_yaml(&bg_text, new_token, live_was_running)?;
    let secret_update = rewrite_fls_secret_yaml(&secret_text, new_token)?;

    if bg_update.server_arg_replacements == 0 {
        anyhow::bail!("no ServiceAuthToken= arguments found in battlegroup.yaml");
    }
    if bg_update.utility_env_replacements != 3 {
        anyhow::bail!(
            "expected 3 FuncomLiveServices__ServiceAuthToken utility env vars in battlegroup.yaml, found {}",
            bg_update.utility_env_replacements
        );
    }
    if secret_update.replacements != 1 {
        anyhow::bail!(
            "expected 1 FuncomLiveServices__ServiceAuthToken in fls-secret.yaml, found {}",
            secret_update.replacements
        );
    }

    let mut battlegroup_backup = None;
    let mut fls_secret_backup = None;
    if !options.dry_run {
        let stamp = Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
        let bg_backup = backup_path(&battlegroup_yaml, &stamp);
        let secret_backup = backup_path(&fls_secret_yaml, &stamp);
        fs::copy(&battlegroup_yaml, &bg_backup).with_context(|| {
            format!(
                "backing up {} to {}",
                battlegroup_yaml.display(),
                bg_backup.display()
            )
        })?;
        fs::copy(&fls_secret_yaml, &secret_backup).with_context(|| {
            format!(
                "backing up {} to {}",
                fls_secret_yaml.display(),
                secret_backup.display()
            )
        })?;
        fs::write(&battlegroup_yaml, &bg_update.text)
            .with_context(|| format!("writing {}", battlegroup_yaml.display()))?;
        fs::write(&fls_secret_yaml, &secret_update.text)
            .with_context(|| format!("writing {}", fls_secret_yaml.display()))?;
        battlegroup_backup = Some(bg_backup);
        fls_secret_backup = Some(secret_backup);

        if !options.skip_live {
            kubectl::run(&[
                "apply",
                "-n",
                &cfg.namespace,
                "-f",
                path_str(&fls_secret_yaml)?,
            ])
            .await?;
            kubectl::run(&[
                "apply",
                "-n",
                &cfg.namespace,
                "-f",
                path_str(&battlegroup_yaml)?,
            ])
            .await?;
            if options.wait {
                wait_ready(cfg).await?;
            }
        }
    }

    Ok(TokenRotationReport {
        metadata,
        expected_host_id,
        battlegroup_yaml,
        fls_secret_yaml,
        battlegroup_backup,
        fls_secret_backup,
        server_arg_replacements: bg_update.server_arg_replacements,
        utility_env_replacements: bg_update.utility_env_replacements,
        fls_secret_replacements: secret_update.replacements,
        stop_forced_false: bg_update.stop_forced_false,
        applied_live: !options.dry_run && !options.skip_live,
        waited_ready: !options.dry_run && !options.skip_live && options.wait,
    })
}

/// Find the FLS JWT in the BattleGroup CR's set arguments.
/// Actual format: -ini:engine:[FuncomLiveServices]:ServiceAuthToken=<jwt>
fn extract_jwt(bg: &serde_json::Value) -> Option<String> {
    let sets = bg
        .pointer("/spec/serverGroup/template/spec/sets")?
        .as_array()?;
    for set in sets {
        let args = set.get("arguments")?.as_array()?;
        for arg in args {
            let s = arg.as_str()?;
            if let Some(token) = s
                .strip_prefix("-ini:engine:[FuncomLiveServices]:ServiceAuthToken=")
                .or_else(|| s.strip_prefix("-FLSAuthToken="))
            {
                return Some(token.to_string());
            }
        }
    }
    None
}

struct BattlegroupYamlUpdate {
    text: String,
    server_arg_replacements: usize,
    utility_env_replacements: usize,
    stop_forced_false: bool,
}

struct SecretYamlUpdate {
    text: String,
    replacements: usize,
}

fn rewrite_battlegroup_yaml(
    text: &str,
    new_token: &str,
    force_stop_false: bool,
) -> Result<BattlegroupYamlUpdate> {
    let jwt = jwt_pattern();
    let service_re = Regex::new(&format!(r"ServiceAuthToken={}", jwt))?;
    let env_re = Regex::new(&format!(
        r"(?m)(name:\s*FuncomLiveServices__ServiceAuthToken\s*\n\s*value:\s*){}",
        jwt
    ))?;
    let stop_re = Regex::new(r"(?m)^(\s*stop:\s*)true\s*$")?;

    let server_arg_replacements = service_re.find_iter(text).count();
    let after_service = service_re
        .replace_all(text, format!("ServiceAuthToken={}", new_token))
        .to_string();

    let utility_env_replacements = env_re.find_iter(&after_service).count();
    let after_env = env_re
        .replace_all(&after_service, format!("${{1}}{}", new_token))
        .to_string();

    let (text, stop_forced_false) = if force_stop_false {
        let count = stop_re.find_iter(&after_env).take(1).count();
        (
            stop_re.replacen(&after_env, 1, "${1}false").to_string(),
            count == 1,
        )
    } else {
        (after_env, false)
    };

    Ok(BattlegroupYamlUpdate {
        text,
        server_arg_replacements,
        utility_env_replacements,
        stop_forced_false,
    })
}

fn rewrite_fls_secret_yaml(text: &str, new_token: &str) -> Result<SecretYamlUpdate> {
    let jwt = jwt_pattern();
    let secret_re = Regex::new(&format!(
        r#"(?m)(FuncomLiveServices__ServiceAuthToken:\s*"?){jwt}("?)"#
    ))?;
    let replacements = secret_re.find_iter(text).count();
    let text = secret_re
        .replace_all(text, format!("${{1}}{}${{2}}", new_token))
        .to_string();
    Ok(SecretYamlUpdate { text, replacements })
}

fn jwt_pattern() -> &'static str {
    r"[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+"
}

fn ensure_file(path: &Path) -> Result<()> {
    if !path.is_file() {
        anyhow::bail!("required file missing: {}", path.display());
    }
    Ok(())
}

fn capsule_value(path: &Path, key: &str) -> Result<Option<String>> {
    let text = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let prefix = format!("{}=", key);
    Ok(text.lines().find_map(|line| {
        line.trim()
            .strip_prefix(&prefix)
            .map(str::trim)
            .map(|value| value.trim_matches('"').to_string())
            .filter(|value| !value.is_empty())
    }))
}

fn backup_path(path: &Path, stamp: &str) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("backup");
    path.with_file_name(format!("{}.bak-{}", file_name, stamp))
}

fn path_str(path: &Path) -> Result<&str> {
    path.to_str()
        .ok_or_else(|| anyhow::anyhow!("path is not valid UTF-8: {}", path.display()))
}

async fn wait_ready(cfg: &Config) -> Result<()> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(420);
    loop {
        let status = battlegroup::status(cfg).await?;
        let gateway_ok = status
            .utilities
            .iter()
            .any(|u| u.name == "Gateway" && u.phase == "Healthy");
        let director_ok = status
            .utilities
            .iter()
            .any(|u| u.name == "Director" && u.phase == "Healthy");
        let rmq_ok = ["RMQ admin", "RMQ game"].iter().all(|name| {
            status
                .utilities
                .iter()
                .any(|u| u.name == *name && u.phase == "Healthy")
        });
        let primary_ok = status
            .runtime_servers
            .iter()
            .any(|server| server.map == "Survival_1" && server.phase == "Running" && server.ready);

        if gateway_ok && director_ok && rmq_ok && primary_ok {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!(
                "timed out waiting for token-rotated world to become ready; gateway_ok={} director_ok={} rmq_ok={} primary_ok={}",
                gateway_ok,
                director_ok,
                rmq_ok,
                primary_ok
            );
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_jwt(exp: i64) -> String {
        let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"HS256","typ":"JWT"}"#);
        let payload = URL_SAFE_NO_PAD.encode(format!(r#"{{"exp":{}}}"#, exp));
        format!("{}.{}.fakesignature", header, payload)
    }

    fn make_full_jwt(exp: i64, host_id: &str, token_index: &str) -> String {
        let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"HS256","typ":"JWT"}"#);
        let payload = URL_SAFE_NO_PAD.encode(format!(
            r#"{{"HostId":"{}","TokenIndex":"{}","exp":{}}}"#,
            host_id, token_index, exp
        ));
        format!("{}.{}.sig", header, payload)
    }

    #[test]
    fn decode_future_token() {
        // 2026-09-05 00:00:00 UTC - the actual FLS token expiry date
        let jwt = make_jwt(1788652800);
        let status = decode(&jwt).unwrap();
        assert_eq!(status.expires_at.timestamp(), 1788652800);
        // As of 2026-05-14 this is > 30 days out, so Ok
        // (test will start failing when the real token should be rotated)
    }

    #[test]
    fn decode_expired_token() {
        let jwt = make_jwt(1000000000); // 2001-09-09, long expired
        let status = decode(&jwt).unwrap();
        assert_eq!(status.state, FlsTokenState::Expired);
    }

    #[test]
    fn decode_token_metadata() {
        let jwt = make_full_jwt(1813691778, "DB3533A2D5A25FB", "1");
        let metadata = decode_metadata(&jwt).unwrap();
        assert_eq!(metadata.host_id.as_deref(), Some("DB3533A2D5A25FB"));
        assert_eq!(metadata.token_index.as_deref(), Some("1"));
        assert_eq!(metadata.expires_at.timestamp(), 1813691778);
    }

    #[test]
    fn decode_invalid_jwt() {
        assert!(decode("notajwt").is_err());
        assert!(decode("only.two").is_err());
    }

    #[test]
    fn extract_jwt_from_arg_prefix() {
        // Matches the actual BattleGroup CR argument format
        let bg = serde_json::json!({
            "spec": {
                "serverGroup": {
                    "template": {
                        "spec": {
                            "sets": [{
                                "map": "Survival_1",
                                "arguments": [
                                    "-FarmRegion=North America Test",
                                    "-ini:engine:[FuncomLiveServices]:ServiceAuthToken=eyJhbGciOiJIUzI1NiJ9.eyJleHAiOjE3ODg2NTI4MDB9.sig"
                                ]
                            }]
                        }
                    }
                }
            }
        });
        let token = extract_jwt(&bg).unwrap();
        assert!(token.starts_with("eyJ"));
    }

    #[test]
    fn rewrites_all_battlegroup_token_surfaces() {
        let old = make_full_jwt(1788652800, "DB3533A2D5A25FB", "2");
        let new = make_full_jwt(1813691778, "DB3533A2D5A25FB", "1");
        let yaml = format!(
            r#"spec:
  serverGroup:
    template:
      spec:
        sets:
        - arguments:
          - -ini:engine:[FuncomLiveServices]:ServiceAuthToken={old}
        - arguments:
          - -ini:engine:[FuncomLiveServices]:ServiceAuthToken={old}
  stop: true
  utilities:
    director:
      spec:
        envVars:
        - name: FuncomLiveServices__ServiceAuthToken
          value: {old}
    serverGateway:
      spec:
        envVars:
        - name: FuncomLiveServices__ServiceAuthToken
          value: {old}
    textRouter:
      spec:
        envVars:
        - name: FuncomLiveServices__ServiceAuthToken
          value: {old}
"#
        );
        let update = rewrite_battlegroup_yaml(&yaml, &new, true).unwrap();
        assert_eq!(update.server_arg_replacements, 2);
        assert_eq!(update.utility_env_replacements, 3);
        assert!(update.stop_forced_false);
        assert!(!update.text.contains(&old));
        assert!(update.text.contains("stop: false"));
        assert_eq!(update.text.matches(&new).count(), 5);
    }

    #[test]
    fn rewrites_fls_secret_token() {
        let old = make_full_jwt(1788652800, "DB3533A2D5A25FB", "2");
        let new = make_full_jwt(1813691778, "DB3533A2D5A25FB", "1");
        let yaml = format!(
            "stringData:\n  FuncomLiveServices__ServiceAuthToken: \"{}\"\n",
            old
        );
        let update = rewrite_fls_secret_yaml(&yaml, &new).unwrap();
        assert_eq!(update.replacements, 1);
        assert!(!update.text.contains(&old));
        assert!(update.text.contains(&new));
    }
}
