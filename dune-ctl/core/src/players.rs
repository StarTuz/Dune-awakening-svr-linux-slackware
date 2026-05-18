use anyhow::Result;

use crate::{config::Config, kubectl};

pub struct OnlinePlayer {
    pub display_name: String,
    pub last_login: Option<String>,
}

pub async fn find_postgres_pod(cfg: &Config) -> Result<String> {
    let output = kubectl::run(&[
        "get",
        "pods",
        "-n",
        &cfg.namespace,
        "--no-headers",
        "-o",
        "custom-columns=NAME:.metadata.name",
    ])
    .await?;
    output
        .lines()
        .find(|l| l.contains("db-dbdepl-sts"))
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("postgres pod not found in {}", cfg.namespace))
}

pub async fn list_online(cfg: &Config) -> Result<Vec<OnlinePlayer>> {
    let pod = find_postgres_pod(cfg).await?;
    let sql = "SELECT dune.decrypt_user_data(encrypted_character_name), \
               last_login_time \
               FROM dune.encrypted_player_state \
               WHERE online_status IN ('Online','LoggingOut') \
               ORDER BY dune.decrypt_user_data(encrypted_character_name)";
    let output = kubectl::run(&[
        "exec",
        "-n",
        &cfg.namespace,
        &pod,
        "--",
        "psql",
        "-U",
        "postgres",
        "-d",
        "dune",
        "-At",
        "-c",
        sql,
    ])
    .await?;

    let mut players = Vec::new();
    for line in output.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let mut parts = line.splitn(2, '|');
        let display_name = parts.next().unwrap_or("").trim().to_string();
        let last_login = parts.next().map(str::trim).map(str::to_string);
        if !display_name.is_empty() {
            players.push(OnlinePlayer {
                display_name,
                last_login,
            });
        }
    }
    Ok(players)
}

pub async fn count_online(cfg: &Config) -> Result<usize> {
    let pod = find_postgres_pod(cfg).await?;
    let sql = "SELECT COUNT(*) FROM dune.encrypted_player_state \
               WHERE online_status IN ('Online','LoggingOut')";
    let output = kubectl::run(&[
        "exec",
        "-n",
        &cfg.namespace,
        &pod,
        "--",
        "psql",
        "-U",
        "postgres",
        "-d",
        "dune",
        "-At",
        "-c",
        sql,
    ])
    .await?;
    Ok(output.trim().parse::<usize>().unwrap_or(0))
}
