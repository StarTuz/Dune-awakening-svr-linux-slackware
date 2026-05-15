use anyhow::Result;

use crate::{config::Config, kubectl};

const RMQ_HTTP_ARG: &str = "--RMQGameHttpPort=30196";

#[derive(Debug, Clone)]
pub struct GatewayStatus {
    pub patched: bool,
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
        patched: deployment_has_patch(&dep),
        ready_replicas: dep.pointer("/status/readyReplicas").and_then(as_u32),
        updated_replicas: dep.pointer("/status/updatedReplicas").and_then(as_u32),
    })
}

/// Returns true if the gateway Deployment already has the RMQ HTTP port arg.
pub async fn is_patched(cfg: &Config) -> Result<bool> {
    let name = gateway_deploy_name(cfg);
    let dep = kubectl::get_json(&["get", "deployment", &name, "-n", &cfg.namespace]).await?;
    Ok(deployment_has_patch(&dep))
}

fn deployment_has_patch(dep: &serde_json::Value) -> bool {
    let containers = dep
        .pointer("/spec/template/spec/containers")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    for c in &containers {
        if let Some(args) = c.get("args").and_then(|v| v.as_array()) {
            if args.iter().any(|a| a.as_str() == Some(RMQ_HTTP_ARG)) {
                return true;
            }
        }
    }
    false
}

/// Append --RMQGameHttpPort=30196 to the gateway Deployment if missing.
/// Returns true if the patch was applied, false if it was already present.
pub async fn patch(cfg: &Config) -> Result<bool> {
    if is_patched(cfg).await? {
        return Ok(false);
    }
    let p = serde_json::json!([{
        "op": "add",
        "path": "/spec/template/spec/containers/0/args/-",
        "value": RMQ_HTTP_ARG,
    }]);
    let name = gateway_deploy_name(cfg);
    kubectl::run(&[
        "patch",
        "deployment",
        &name,
        "-n",
        &cfg.namespace,
        "--type=json",
        &format!("-p={}", p),
    ])
    .await?;
    Ok(true)
}

fn as_u32(v: &serde_json::Value) -> Option<u32> {
    v.as_u64().and_then(|n| n.try_into().ok())
}
