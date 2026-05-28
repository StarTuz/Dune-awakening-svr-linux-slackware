#[derive(Debug, Clone)]
pub struct DiagnosticsSnapshot {
    pub k3s_api: Check,
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
            k3s_api: read_k3s_api().await,
            firewall_backend: read_firewalld_backend().await,
        }
    }
}

async fn read_k3s_api() -> Check {
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(8),
        tokio::process::Command::new("sudo")
            .arg("-n")
            .arg("kubectl")
            .arg("get")
            .arg("--raw=/readyz")
            .output(),
    )
    .await;

    let output = match output {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            return Check {
                state: CheckState::Critical,
                message: format!("cannot spawn kubectl: {}", e),
            };
        }
        Err(_) => {
            return Check {
                state: CheckState::Critical,
                message: "kubectl /readyz timed out; k3s API is not healthy".to_string(),
            };
        }
    };

    if output.status.success() {
        let text = String::from_utf8_lossy(&output.stdout);
        let message = if text.trim() == "ok" {
            "k3s API readyz ok".to_string()
        } else {
            format!("k3s API readyz returned {}", text.trim())
        };
        return Check {
            state: CheckState::Ok,
            message,
        };
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    Check {
        state: CheckState::Critical,
        message: format!("k3s API unavailable: {}", stderr.trim()),
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
