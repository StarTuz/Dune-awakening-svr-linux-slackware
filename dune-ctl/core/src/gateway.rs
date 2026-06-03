use anyhow::Result;

use crate::{config::Config, kubectl};

const RMQ_HOST_PREFIX: &str = "--RMQGameHostname=";

/// Live state of the gateway Deployment.
///
/// Historically dune-ctl tracked whether a manual `--RMQGameHttpPort=30196`
/// "patch" was present. That patch is retired: the value was stale (the live
/// RMQ management NodePort is dynamic, not 30196) and `GameRmqHttpAddress` is
/// off the gameplay path. The address that matters — `--RMQGameHostname` — is
/// derived by the operator from the k3s node external IP, so the useful signal
/// now is the advertised hostname and rollout readiness.
#[derive(Debug, Clone)]
pub struct GatewayStatus {
    /// The `--RMQGameHostname=<ip>` the gateway advertises to FLS, if present.
    pub hostname: Option<String>,
    pub ready_replicas: Option<u32>,
    pub updated_replicas: Option<u32>,
}

fn gateway_deploy_name(cfg: &Config) -> String {
    format!("{}-sgw-deploy", cfg.battlegroup)
}

pub async fn status(cfg: &Config) -> Result<GatewayStatus> {
    let name = gateway_deploy_name(cfg);
    let dep = kubectl::get_json(&["get", "deployment", &name, "-n", &cfg.namespace]).await?;
    Ok(GatewayStatus {
        hostname: deployment_hostname(&dep),
        ready_replicas: dep.pointer("/status/readyReplicas").and_then(as_u32),
        updated_replicas: dep.pointer("/status/updatedReplicas").and_then(as_u32),
    })
}

/// Read the `--RMQGameHostname=<ip>` argument from the gateway Deployment.
fn deployment_hostname(dep: &serde_json::Value) -> Option<String> {
    dep.pointer("/spec/template/spec/containers/0/args")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .find_map(|a| {
            a.as_str()
                .and_then(|v| v.strip_prefix(RMQ_HOST_PREFIX))
                .map(str::to_string)
        })
}

fn as_u32(v: &serde_json::Value) -> Option<u32> {
    v.as_u64().and_then(|n| n.try_into().ok())
}
