use anyhow::{Context, Result};
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    path::{Path, PathBuf},
    process::Stdio,
};
use tokio::process::Command;

use crate::{config::Config, kubectl};

pub const HOST_IP_ENV: &str = "HOST_DATACENTER_IP_ADDRESS";
pub const RMQ_HOST_PREFIX: &str = "--RMQGameHostname=";
pub const RMQ_HTTP_ARG: &str = "--RMQGameHttpPort=30196";
pub const DEFAULT_PROVIDERS: &[&str] = &[
    "https://api.ipify.org",
    "https://ifconfig.me/ip",
    "https://icanhazip.com",
];

#[derive(Debug, Clone)]
pub struct PublicIpSummary {
    pub local_ips: Vec<String>,
    pub live_ips: Vec<String>,
    pub gateway_hostname: Option<String>,
    pub gateway_http_patched: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct PlannedFileChange {
    pub path: PathBuf,
    pub required: bool,
    pub exists: bool,
    pub changed: bool,
}

#[derive(Debug, Clone)]
pub struct PublicIpPlan {
    pub new_ip: String,
    pub old_ips: Vec<String>,
    pub files: Vec<PlannedFileChange>,
    pub live: bool,
}

#[derive(Debug, Clone)]
pub struct ProviderObservation {
    pub provider: String,
    pub ip: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DetectionSummary {
    pub detected_ip: String,
    pub observations: Vec<ProviderObservation>,
}

pub fn validate_public_ip(ip: &str) -> Result<IpAddr> {
    let parsed: IpAddr = ip
        .parse()
        .with_context(|| format!("'{}' is not a valid IP address", ip))?;
    if !is_public_ip(parsed) {
        anyhow::bail!("'{}' is not a public Internet address", ip);
    }
    Ok(parsed)
}

pub async fn detect(providers: &[String]) -> Result<DetectionSummary> {
    let providers = if providers.is_empty() {
        DEFAULT_PROVIDERS
            .iter()
            .map(|provider| provider.to_string())
            .collect()
    } else {
        providers.to_vec()
    };

    let mut observations = Vec::new();
    for provider in providers {
        observations.push(fetch_provider(&provider).await);
    }

    let detected_ip = quorum_ip(&observations)?;
    Ok(DetectionSummary {
        detected_ip,
        observations,
    })
}

async fn fetch_provider(provider: &str) -> ProviderObservation {
    let output = Command::new("curl")
        .args(["-fsS", "--max-time", "3", provider])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await;

    match output {
        Ok(output) if output.status.success() => {
            let text = String::from_utf8_lossy(&output.stdout);
            match parse_provider_output(&text) {
                Ok(ip) => ProviderObservation {
                    provider: provider.to_string(),
                    ip: Some(ip),
                    error: None,
                },
                Err(error) => ProviderObservation {
                    provider: provider.to_string(),
                    ip: None,
                    error: Some(error.to_string()),
                },
            }
        }
        Ok(output) => ProviderObservation {
            provider: provider.to_string(),
            ip: None,
            error: Some(String::from_utf8_lossy(&output.stderr).trim().to_string()),
        },
        Err(error) => ProviderObservation {
            provider: provider.to_string(),
            ip: None,
            error: Some(error.to_string()),
        },
    }
}

pub fn parse_provider_output(output: &str) -> Result<String> {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        anyhow::bail!("empty response");
    }
    if trimmed.split_whitespace().count() != 1 {
        anyhow::bail!("response contains more than one token");
    }
    validate_public_ip(trimmed)?;
    Ok(trimmed.to_string())
}

pub fn quorum_ip(observations: &[ProviderObservation]) -> Result<String> {
    let mut counts = BTreeMap::<String, usize>::new();
    for ip in observations
        .iter()
        .filter_map(|observation| observation.ip.as_ref())
    {
        *counts.entry(ip.clone()).or_insert(0) += 1;
    }

    if let Some((ip, _)) = counts.iter().find(|(_, count)| **count >= 2) {
        return Ok(ip.clone());
    }

    let valid = counts
        .iter()
        .map(|(ip, count)| format!("{} ({})", ip, count))
        .collect::<Vec<_>>()
        .join(", ");
    if valid.is_empty() {
        anyhow::bail!("no provider returned a valid public IP");
    }
    anyhow::bail!("provider quorum failed; valid responses: {}", valid);
}

pub async fn show(cfg: &Config) -> Result<PublicIpSummary> {
    let local_ips = local_public_ips(cfg)?;
    let live_ips = live_public_ips(cfg).await.unwrap_or_default();
    let gateway = gateway_state(cfg).await.ok();

    Ok(PublicIpSummary {
        local_ips,
        live_ips,
        gateway_hostname: gateway.as_ref().and_then(|state| state.hostname.clone()),
        gateway_http_patched: gateway.map(|state| state.http_patched),
    })
}

pub async fn plan_set(cfg: &Config, new_ip: &str, skip_live: bool) -> Result<PublicIpPlan> {
    validate_public_ip(new_ip)?;
    let mut old = BTreeSet::new();
    for ip in local_public_ips(cfg)? {
        if ip != new_ip {
            old.insert(ip);
        }
    }
    if !skip_live {
        for ip in live_public_ips(cfg).await.unwrap_or_default() {
            if ip != new_ip {
                old.insert(ip);
            }
        }
        if let Ok(Some(hostname)) = gateway_rmq_hostname(cfg).await {
            if hostname != new_ip && validate_public_ip(&hostname).is_ok() {
                old.insert(hostname);
            }
        }
    }

    let old_ips: Vec<String> = old.into_iter().collect();
    let files = plan_file_changes(cfg, new_ip, &old_ips)?;
    Ok(PublicIpPlan {
        new_ip: new_ip.to_string(),
        old_ips,
        files,
        live: !skip_live,
    })
}

pub async fn apply_set(
    cfg: &Config,
    new_ip: &str,
    skip_files: bool,
    skip_live: bool,
) -> Result<PublicIpPlan> {
    let plan = plan_set(cfg, new_ip, skip_live).await?;
    if !skip_files {
        apply_file_changes(cfg, new_ip, &plan.old_ips)?;
    }
    if !skip_live {
        patch_battlegroup(cfg, new_ip).await?;
        patch_gateway_deployment(cfg, new_ip).await?;
        remove_last_applied(cfg).await.ok();
    }
    Ok(plan)
}

pub fn plan_file_changes(
    cfg: &Config,
    new_ip: &str,
    old_ips: &[String],
) -> Result<Vec<PlannedFileChange>> {
    let mut changes = Vec::new();
    for target in target_files(cfg) {
        let exists = target.path.exists();
        let changed = if exists {
            let current = std::fs::read_to_string(&target.path)
                .with_context(|| format!("failed to read {}", target.path.display()))?;
            rewrite_text(&current, new_ip, old_ips, is_capsule_env(&target.path)) != current
        } else {
            false
        };
        changes.push(PlannedFileChange {
            path: target.path,
            required: target.required,
            exists,
            changed,
        });
    }
    Ok(changes)
}

fn apply_file_changes(cfg: &Config, new_ip: &str, old_ips: &[String]) -> Result<()> {
    for target in target_files(cfg) {
        if !target.path.exists() {
            if target.required {
                anyhow::bail!("required file is missing: {}", target.path.display());
            }
            continue;
        }
        let current = std::fs::read_to_string(&target.path)
            .with_context(|| format!("failed to read {}", target.path.display()))?;
        let rewritten = rewrite_text(&current, new_ip, old_ips, is_capsule_env(&target.path));
        if rewritten != current {
            std::fs::write(&target.path, rewritten)
                .with_context(|| format!("failed to write {}", target.path.display()))?;
        }
    }
    Ok(())
}

fn rewrite_text(text: &str, new_ip: &str, old_ips: &[String], capsule_env: bool) -> String {
    let mut out = text.to_string();
    for old in old_ips {
        out = out.replace(old, new_ip);
    }

    if capsule_env {
        if out.lines().any(|line| line.starts_with("host_ip=")) {
            out = out
                .lines()
                .map(|line| {
                    if line.starts_with("host_ip=") {
                        format!("host_ip={}", new_ip)
                    } else {
                        line.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");
            if text.ends_with('\n') {
                out.push('\n');
            }
        } else {
            if !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(&format!("host_ip={}\n", new_ip));
        }
    }

    out
}

pub fn local_public_ips(cfg: &Config) -> Result<Vec<String>> {
    let mut ips = BTreeSet::new();
    for target in target_files(cfg) {
        if !target.path.exists() {
            continue;
        }
        let text = std::fs::read_to_string(&target.path)
            .with_context(|| format!("failed to read {}", target.path.display()))?;
        ips.extend(public_ips_in_text(&text));
    }
    Ok(ips.into_iter().collect())
}

async fn live_public_ips(cfg: &Config) -> Result<Vec<String>> {
    let bg = crate::battlegroup::raw(cfg).await?;
    let mut ips = BTreeSet::new();
    for util in ["director", "serverGateway", "textRouter"] {
        if let Some(ip) = host_env_from_utility(&bg, util) {
            ips.insert(ip);
        }
    }
    Ok(ips.into_iter().collect())
}

async fn patch_battlegroup(cfg: &Config, new_ip: &str) -> Result<()> {
    let bg = crate::battlegroup::raw(cfg).await?;
    let mut patch = Vec::new();
    for util in ["director", "serverGateway", "textRouter"] {
        let path = format!("/spec/utilities/{}/spec/envVars", util);
        let env = bg
            .pointer(&path)
            .and_then(|value| value.as_array())
            .ok_or_else(|| anyhow::anyhow!("missing BattleGroup envVars at {}", path))?;
        let idx = env
            .iter()
            .position(|item| item.get("name").and_then(|v| v.as_str()) == Some(HOST_IP_ENV))
            .ok_or_else(|| anyhow::anyhow!("missing {} in {}", HOST_IP_ENV, path))?;
        patch.push(json!({
            "op": "replace",
            "path": format!("{}/{}/value", path, idx),
            "value": new_ip,
        }));
    }

    kubectl::run(&[
        "patch",
        "battlegroup",
        &cfg.battlegroup,
        "-n",
        &cfg.namespace,
        "--type=json",
        &format!("-p={}", serde_json::to_string(&patch)?),
    ])
    .await?;
    Ok(())
}

async fn patch_gateway_deployment(cfg: &Config, new_ip: &str) -> Result<()> {
    let name = gateway_deploy_name(cfg);
    let dep = kubectl::get_json(&["get", "deployment", &name, "-n", &cfg.namespace]).await?;
    let args = dep
        .pointer("/spec/template/spec/containers/0/args")
        .and_then(|value| value.as_array())
        .ok_or_else(|| anyhow::anyhow!("gateway deployment has no container args"))?;

    let mut patch = Vec::new();
    if let Some(idx) = args.iter().position(|arg| {
        arg.as_str()
            .map(|value| value.starts_with(RMQ_HOST_PREFIX))
            .unwrap_or(false)
    }) {
        patch.push(json!({
            "op": "replace",
            "path": format!("/spec/template/spec/containers/0/args/{}", idx),
            "value": format!("{}{}", RMQ_HOST_PREFIX, new_ip),
        }));
    } else {
        patch.push(json!({
            "op": "add",
            "path": "/spec/template/spec/containers/0/args/-",
            "value": format!("{}{}", RMQ_HOST_PREFIX, new_ip),
        }));
    }

    if !args.iter().any(|arg| arg.as_str() == Some(RMQ_HTTP_ARG)) {
        patch.push(json!({
            "op": "add",
            "path": "/spec/template/spec/containers/0/args/-",
            "value": RMQ_HTTP_ARG,
        }));
    }

    kubectl::run(&[
        "patch",
        "deployment",
        &name,
        "-n",
        &cfg.namespace,
        "--type=json",
        &format!("-p={}", serde_json::to_string(&patch)?),
    ])
    .await?;
    Ok(())
}

async fn remove_last_applied(cfg: &Config) -> Result<()> {
    kubectl::run(&[
        "annotate",
        "battlegroup",
        &cfg.battlegroup,
        "-n",
        &cfg.namespace,
        "kubectl.kubernetes.io/last-applied-configuration-",
    ])
    .await?;
    Ok(())
}

#[derive(Debug)]
struct GatewayState {
    hostname: Option<String>,
    http_patched: bool,
}

async fn gateway_state(cfg: &Config) -> Result<GatewayState> {
    let name = gateway_deploy_name(cfg);
    let dep = kubectl::get_json(&["get", "deployment", &name, "-n", &cfg.namespace]).await?;
    let args = dep
        .pointer("/spec/template/spec/containers/0/args")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    let hostname = args.iter().find_map(|arg| {
        arg.as_str()
            .and_then(|value| value.strip_prefix(RMQ_HOST_PREFIX))
            .map(str::to_string)
    });
    let http_patched = args.iter().any(|arg| arg.as_str() == Some(RMQ_HTTP_ARG));
    Ok(GatewayState {
        hostname,
        http_patched,
    })
}

async fn gateway_rmq_hostname(cfg: &Config) -> Result<Option<String>> {
    Ok(gateway_state(cfg).await?.hostname)
}

fn gateway_deploy_name(cfg: &Config) -> String {
    format!("{}-sgw-deploy", cfg.battlegroup)
}

fn host_env_from_utility(bg: &serde_json::Value, utility: &str) -> Option<String> {
    bg.pointer(&format!("/spec/utilities/{}/spec/envVars", utility))?
        .as_array()?
        .iter()
        .find(|item| item.get("name").and_then(|v| v.as_str()) == Some(HOST_IP_ENV))?
        .get("value")?
        .as_str()
        .map(str::to_string)
}

#[derive(Debug)]
struct TargetFile {
    path: PathBuf,
    required: bool,
}

fn target_files(cfg: &Config) -> Vec<TargetFile> {
    let mut files = Vec::new();
    let dune = dune_home();
    files.push(TargetFile {
        path: dune.join(format!("{}.yaml", cfg.battlegroup)),
        required: false,
    });
    if let Some(path) = &cfg.world_spec {
        files.push(TargetFile {
            path: path.clone(),
            required: true,
        });
    }
    files.push(TargetFile {
        path: cfg.capsule_dir().join("capsule.env"),
        required: false,
    });
    files.push(TargetFile {
        path: cfg.capsule_dir().join("battlegroup.yaml"),
        required: false,
    });
    files.push(TargetFile {
        path: dune.join("settings.conf"),
        required: false,
    });

    let mut seen = BTreeSet::new();
    files
        .into_iter()
        .filter(|file| seen.insert(file.path.clone()))
        .collect()
}

fn is_capsule_env(path: &Path) -> bool {
    path.file_name().and_then(|name| name.to_str()) == Some("capsule.env")
}

fn dune_home() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/home/dune".into());
    PathBuf::from(home).join(".dune")
}

fn public_ips_in_text(text: &str) -> Vec<String> {
    let mut ips = BTreeSet::new();
    for token in text.split(|ch: char| !(ch.is_ascii_hexdigit() || ch == '.' || ch == ':')) {
        if token.is_empty() {
            continue;
        }
        if let Ok(ip) = token.parse::<IpAddr>() {
            if is_public_ip(ip) {
                ips.insert(ip.to_string());
            }
        }
    }
    ips.into_iter().collect()
}

fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_public_ipv4(ip),
        IpAddr::V6(ip) => is_public_ipv6(ip),
    }
}

fn is_public_ipv4(ip: Ipv4Addr) -> bool {
    !(ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_multicast()
        || ip.is_broadcast()
        || ip.is_documentation()
        || ip.is_unspecified()
        || ip.octets()[0] == 0
        || ip.octets()[0] >= 224
        || ip.octets()[0] == 100 && (64..=127).contains(&ip.octets()[1]))
}

fn is_public_ipv6(ip: Ipv6Addr) -> bool {
    !(ip.is_loopback()
        || ip.is_multicast()
        || ip.is_unspecified()
        || (ip.segments()[0] & 0xfe00) == 0xfc00
        || (ip.segments()[0] & 0xffc0) == 0xfe80)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_public_ips() {
        assert!(validate_public_ip("47.145.31.211").is_ok());
        assert!(validate_public_ip("192.168.254.200").is_err());
        assert!(validate_public_ip("127.0.0.1").is_err());
        assert!(validate_public_ip("not-an-ip").is_err());
    }

    #[test]
    fn extracts_public_ips_only() {
        let ips = public_ips_in_text("old=47.145.51.160 lan=192.168.254.200 loop=127.0.0.1");
        assert_eq!(ips, vec!["47.145.51.160"]);
    }

    #[test]
    fn rewrites_capsule_host_ip() {
        let text = "environment=live\nhost_ip=47.145.51.160\n";
        let rewritten = rewrite_text(
            text,
            "47.145.31.211",
            &[String::from("47.145.51.160")],
            true,
        );
        assert_eq!(rewritten, "environment=live\nhost_ip=47.145.31.211\n");
    }

    #[test]
    fn adds_missing_capsule_host_ip() {
        let text = "environment=live\n";
        let rewritten = rewrite_text(text, "47.145.31.211", &[], true);
        assert_eq!(rewritten, "environment=live\nhost_ip=47.145.31.211\n");
    }

    #[test]
    fn parses_provider_output_strictly() {
        assert_eq!(
            parse_provider_output("47.145.31.211\n").unwrap(),
            "47.145.31.211"
        );
        assert!(parse_provider_output("47.145.31.211 extra").is_err());
        assert!(parse_provider_output("<html>47.145.31.211</html>").is_err());
        assert!(parse_provider_output("192.168.254.200").is_err());
        assert!(parse_provider_output("").is_err());
    }

    #[test]
    fn requires_provider_quorum() {
        let observations = vec![
            observation("a", Some("47.145.31.211"), None),
            observation("b", Some("47.145.31.211"), None),
            observation("c", Some("47.145.31.212"), None),
        ];
        assert_eq!(quorum_ip(&observations).unwrap(), "47.145.31.211");

        let observations = vec![
            observation("a", Some("47.145.31.211"), None),
            observation("b", Some("47.145.31.212"), None),
            observation("c", None, Some("timeout")),
        ];
        assert!(quorum_ip(&observations).is_err());
    }

    fn observation(provider: &str, ip: Option<&str>, error: Option<&str>) -> ProviderObservation {
        ProviderObservation {
            provider: provider.to_string(),
            ip: ip.map(str::to_string),
            error: error.map(str::to_string),
        }
    }
}
