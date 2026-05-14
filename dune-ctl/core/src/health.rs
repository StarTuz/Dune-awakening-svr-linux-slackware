use anyhow::Result;

use crate::{battlegroup, config::Config, fls};

#[derive(Debug, Clone)]
pub struct HealthSnapshot {
    pub battlegroup_phase: String,
    pub maps: Vec<battlegroup::MapEntry>,
    pub fls: Option<fls::FlsTokenStatus>,
    pub ram_used_bytes: Option<u64>,
    pub ram_total_bytes: Option<u64>,
}

impl HealthSnapshot {
    pub async fn collect(cfg: &Config) -> Result<Self> {
        let mut bg = battlegroup::status(cfg).await?;
        battlegroup::enrich_phases(cfg, &mut bg.maps).await?;

        let fls_status = fls::check(cfg).await.ok();
        let (ram_used, ram_total) = read_meminfo().await;

        Ok(Self {
            battlegroup_phase: bg.phase,
            maps: bg.maps,
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
