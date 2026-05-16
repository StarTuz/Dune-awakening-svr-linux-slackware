use anyhow::Result;

use crate::{battlegroup, config::Config, diagnostics, fls, gateway};

#[derive(Debug, Clone)]
pub struct HealthSnapshot {
    pub battlegroup_phase: String,
    pub battlegroup_title: Option<String>,
    pub battlegroup_stopped: bool,
    pub battlegroup_size: Option<u32>,
    pub battlegroup_started_at: Option<String>,
    pub maps: Vec<battlegroup::MapEntry>,
    pub sietches: Vec<battlegroup::SietchEntry>,
    pub utilities: Vec<battlegroup::UtilityStatus>,
    pub runtime_servers: Vec<battlegroup::RuntimeServer>,
    pub gateway: Option<gateway::GatewayStatus>,
    pub diagnostics: diagnostics::DiagnosticsSnapshot,
    pub fls: Option<fls::FlsTokenStatus>,
    pub ram_used_bytes: Option<u64>,
    pub ram_total_bytes: Option<u64>,
}

impl HealthSnapshot {
    pub async fn collect(cfg: &Config) -> Result<Self> {
        let mut bg = battlegroup::status(cfg).await?;
        battlegroup::enrich_maps(cfg, &mut bg.maps).await?;
        let sietches = battlegroup::derive_sietches(&bg.maps);

        let fls_status = fls::check(cfg).await.ok();
        let gateway_status = gateway::status(cfg).await.ok();
        let diagnostics = diagnostics::DiagnosticsSnapshot::collect().await;
        let (ram_used, ram_total) = read_meminfo().await;

        Ok(Self {
            battlegroup_phase: bg.phase,
            battlegroup_title: bg.title,
            battlegroup_stopped: bg.stop,
            battlegroup_size: bg.size,
            battlegroup_started_at: bg.start_timestamp,
            maps: bg.maps,
            sietches,
            utilities: bg.utilities,
            runtime_servers: bg.runtime_servers,
            gateway: gateway_status,
            diagnostics,
            fls: fls_status,
            ram_used_bytes: ram_used,
            ram_total_bytes: ram_total,
        })
    }
}

async fn read_meminfo() -> (Option<u64>, Option<u64>) {
    let Ok(text) = tokio::fs::read_to_string("/proc/meminfo").await else {
        return (None, None);
    };
    let mut total_kb: Option<u64> = None;
    let mut avail_kb: Option<u64> = None;
    for line in text.lines() {
        if line.starts_with("MemTotal:") {
            total_kb = parse_kb(line);
        } else if line.starts_with("MemAvailable:") {
            avail_kb = parse_kb(line);
        }
    }
    match (total_kb, avail_kb) {
        (Some(t), Some(a)) => (Some((t - a) * 1024), Some(t * 1024)),
        _ => (None, None),
    }
}

fn parse_kb(line: &str) -> Option<u64> {
    line.split_whitespace().nth(1)?.parse().ok()
}
