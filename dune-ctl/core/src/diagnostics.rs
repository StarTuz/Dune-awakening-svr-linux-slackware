
#[derive(Debug, Clone)]
pub struct DiagnosticsSnapshot {
    pub firewall_backend: Check,
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
        Self {
            firewall_backend: read_firewalld_backend().await,
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
            message: "FirewallBackend=nftables; k3s/flannel conflict risk on this host"
                .to_string(),
        },
        other => Check {
            state: CheckState::Unknown,
            message: format!("FirewallBackend={}", other),
        },
    }
}
