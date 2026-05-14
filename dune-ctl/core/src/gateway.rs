use anyhow::Result;

use crate::{config::Config, kubectl};

const RMQ_HTTP_ARG: &str = "--RMQGameHttpPort=30196";

/// Returns true if the gateway Deployment already has the RMQ HTTP port arg.
pub async fn is_patched(cfg: &Config) -> Result<bool> {
    let dep = kubectl::get_json(&["get", "deployment", "gateway", "-n", &cfg.namespace]).await?;
    let containers = dep
        .pointer("/spec/template/spec/containers")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    for c in &containers {
        if let Some(args) = c.get("args").and_then(|v| v.as_array()) {
            if args.iter().any(|a| a.as_str() == Some(RMQ_HTTP_ARG)) {
                return Ok(true);
            }
        }
    }
    Ok(false)
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
    kubectl::run(&[
        "patch", "deployment", "gateway",
        "-n", &cfg.namespace,
        "--type=json",
        &format!("-p={}", p),
    ])
    .await?;
    Ok(true)
}
