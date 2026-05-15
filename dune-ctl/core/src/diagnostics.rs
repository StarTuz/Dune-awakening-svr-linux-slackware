use tokio::process::Command;

const NFT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

#[derive(Debug, Clone)]
pub struct DiagnosticsSnapshot {
    pub firewall_backend: Check,
    pub stale_nft_firewalld: Check,
    pub nft_tables: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Check {
    pub state: CheckState,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckState {
    Ok,
    Warning,
    Critical,
    Unknown,
}

impl CheckState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Warning => "warn",
            Self::Critical => "critical",
            Self::Unknown => "unknown",
        }
    }
}

impl DiagnosticsSnapshot {
    pub async fn collect() -> Self {
        let backend = read_firewalld_backend().await;
        let nft_tables = read_nft_tables().await;
        let stale_nft_firewalld = stale_nft_check(&backend, &nft_tables);

        Self {
            firewall_backend: backend,
            stale_nft_firewalld,
            nft_tables,
        }
    }
}

async fn read_firewalld_backend() -> Check {
    let text = match tokio::fs::read_to_string("/etc/firewalld/firewalld.conf").await {
        Ok(text) => text,
        Err(e) => {
            return Check {
                state: CheckState::Unknown,
                message: format!("cannot read firewalld.conf: {}", e),
            };
        }
    };

    let backend = text
        .lines()
        .find_map(|line| line.strip_prefix("FirewallBackend="))
        .map(str::trim)
        .unwrap_or("unspecified");

    match backend {
        "iptables" => Check {
            state: CheckState::Ok,
            message: "FirewallBackend=iptables".to_string(),
        },
        "nftables" => Check {
            state: CheckState::Warning,
            message: "FirewallBackend=nftables; k3s/flannel conflict risk on this host".to_string(),
        },
        other => Check {
            state: CheckState::Unknown,
            message: format!("FirewallBackend={}", other),
        },
    }
}

async fn read_nft_tables() -> Vec<String> {
    let child = Command::new("sudo")
        .arg("-n")
        .arg("/usr/sbin/nft")
        .arg("list")
        .arg("tables")
        .output();

    let output = tokio::time::timeout(NFT_TIMEOUT, child).await;

    let Ok(Ok(output)) = output else {
        return vec![];
    };
    if !output.status.success() {
        return vec![];
    }

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

fn stale_nft_check(backend: &Check, tables: &[String]) -> Check {
    let has_firewalld_table = tables.iter().any(|line| line == "table inet firewalld");
    let iptables_backend = backend.message.contains("FirewallBackend=iptables");

    match (iptables_backend, has_firewalld_table) {
        (true, true) => Check {
            state: CheckState::Critical,
            message: "stale nft table inet firewalld is active while backend is iptables"
                .to_string(),
        },
        (true, false) => Check {
            state: CheckState::Ok,
            message: "no stale nft firewalld table detected".to_string(),
        },
        (false, true) => Check {
            state: CheckState::Warning,
            message: "nft firewalld table present".to_string(),
        },
        (false, false) => Check {
            state: CheckState::Unknown,
            message: "nft firewalld table absent; backend not confirmed iptables".to_string(),
        },
    }
}
