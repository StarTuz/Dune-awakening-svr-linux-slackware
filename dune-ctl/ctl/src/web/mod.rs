use anyhow::Result;
use axum::{extract::State, routing::get, Json, Router};
use dune_ctl_core::{config::Config, health::HealthSnapshot};
use std::net::SocketAddr;

pub async fn run(port: u16, cfg: &Config) -> Result<()> {
    let router = Router::new()
        .route("/", get(root))
        .route("/health", get(health_handler))
        .with_state(cfg.clone());

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    println!("dune-ctl web on http://{}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router).await?;
    Ok(())
}

async fn root() -> &'static str {
    "dune-ctl web — use /health for JSON status"
}

async fn health_handler(State(cfg): State<Config>) -> Json<serde_json::Value> {
    match HealthSnapshot::collect(&cfg).await {
        Ok(snap) => Json(serde_json::json!({
            "battlegroup": cfg.battlegroup,
            "phase": snap.battlegroup_phase,
            "maps": snap.maps.iter().map(|m| serde_json::json!({
                "name": m.name,
                "phase": m.phase,
                "replicas": m.replicas,
            })).collect::<Vec<_>>(),
            "fls": snap.fls.as_ref().map(|f| serde_json::json!({
                "label": f.label(),
                "days_remaining": f.days_remaining,
                "expires_at": f.expires_at.to_rfc3339(),
                "state": format!("{:?}", f.state),
            })),
            "ram": snap.ram_used_bytes.zip(snap.ram_total_bytes).map(|(u, t)| serde_json::json!({
                "used_bytes": u,
                "total_bytes": t,
            })),
        })),
        Err(e) => Json(serde_json::json!({"error": e.to_string()})),
    }
}
