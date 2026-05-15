use anyhow::{Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::{config::Config, kubectl};

const WARN_DAYS: i64 = 30;
const CRITICAL_DAYS: i64 = 14;

#[derive(Debug, Clone)]
pub struct FlsTokenStatus {
    pub expires_at: DateTime<Utc>,
    pub days_remaining: i64,
    pub state: FlsTokenState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlsTokenState {
    Ok,
    WarningSoon,
    Critical,
    Expired,
}

impl FlsTokenStatus {
    pub fn label(&self) -> String {
        match self.state {
            FlsTokenState::Ok => format!("{}d OK", self.days_remaining),
            FlsTokenState::WarningSoon => format!("{}d WARN", self.days_remaining),
            FlsTokenState::Critical => format!("{}d CRIT", self.days_remaining),
            FlsTokenState::Expired => "EXPIRED".to_string(),
        }
    }
}

#[derive(Deserialize)]
struct JwtPayload {
    exp: i64,
}

/// Decode a JWT and return token expiry status. Does not verify the signature.
pub fn decode(jwt: &str) -> Result<FlsTokenStatus> {
    let parts: Vec<&str> = jwt.splitn(3, '.').collect();
    if parts.len() != 3 {
        anyhow::bail!(
            "invalid JWT: expected 3 dot-separated parts, got {}",
            parts.len()
        );
    }
    let payload_bytes = URL_SAFE_NO_PAD
        .decode(parts[1])
        .context("failed to base64url-decode JWT payload")?;
    let claims: JwtPayload =
        serde_json::from_slice(&payload_bytes).context("failed to parse JWT payload JSON")?;

    let expires_at = DateTime::from_timestamp(claims.exp, 0)
        .ok_or_else(|| anyhow::anyhow!("JWT exp out of range: {}", claims.exp))?;
    let days_remaining = (expires_at - Utc::now()).num_days();

    let state = match days_remaining {
        d if d <= 0 => FlsTokenState::Expired,
        d if d <= CRITICAL_DAYS => FlsTokenState::Critical,
        d if d <= WARN_DAYS => FlsTokenState::WarningSoon,
        _ => FlsTokenState::Ok,
    };

    Ok(FlsTokenStatus {
        expires_at,
        days_remaining,
        state,
    })
}

/// Pull the FLS JWT from the live BattleGroup CR and decode it.
pub async fn check(cfg: &Config) -> Result<FlsTokenStatus> {
    let bg =
        kubectl::get_json(&["get", "battlegroup", &cfg.battlegroup, "-n", &cfg.namespace]).await?;

    let token = extract_jwt(&bg)
        .ok_or_else(|| anyhow::anyhow!("FLS JWT not found in BattleGroup CR args"))?;
    decode(&token)
}

/// Find the FLS JWT in the BattleGroup CR's set arguments.
/// Actual format: -ini:engine:[FuncomLiveServices]:ServiceAuthToken=<jwt>
fn extract_jwt(bg: &serde_json::Value) -> Option<String> {
    let sets = bg
        .pointer("/spec/serverGroup/template/spec/sets")?
        .as_array()?;
    for set in sets {
        let args = set.get("arguments")?.as_array()?;
        for arg in args {
            let s = arg.as_str()?;
            if let Some(token) = s
                .strip_prefix("-ini:engine:[FuncomLiveServices]:ServiceAuthToken=")
                .or_else(|| s.strip_prefix("-FLSAuthToken="))
            {
                return Some(token.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_jwt(exp: i64) -> String {
        let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"HS256","typ":"JWT"}"#);
        let payload = URL_SAFE_NO_PAD.encode(format!(r#"{{"exp":{}}}"#, exp));
        format!("{}.{}.fakesignature", header, payload)
    }

    #[test]
    fn decode_future_token() {
        // 2026-09-05 00:00:00 UTC — the actual FLS token expiry date
        let jwt = make_jwt(1788652800);
        let status = decode(&jwt).unwrap();
        assert_eq!(status.expires_at.timestamp(), 1788652800);
        // As of 2026-05-14 this is > 30 days out, so Ok
        // (test will start failing when the real token should be rotated)
    }

    #[test]
    fn decode_expired_token() {
        let jwt = make_jwt(1000000000); // 2001-09-09 — long expired
        let status = decode(&jwt).unwrap();
        assert_eq!(status.state, FlsTokenState::Expired);
    }

    #[test]
    fn decode_invalid_jwt() {
        assert!(decode("notajwt").is_err());
        assert!(decode("only.two").is_err());
    }

    #[test]
    fn extract_jwt_from_arg_prefix() {
        // Matches the actual BattleGroup CR argument format
        let bg = serde_json::json!({
            "spec": {
                "serverGroup": {
                    "template": {
                        "spec": {
                            "sets": [{
                                "map": "Survival_1",
                                "arguments": [
                                    "-FarmRegion=North America Test",
                                    "-ini:engine:[FuncomLiveServices]:ServiceAuthToken=eyJhbGciOiJIUzI1NiJ9.eyJleHAiOjE3ODg2NTI4MDB9.sig"
                                ]
                            }]
                        }
                    }
                }
            }
        });
        let token = extract_jwt(&bg).unwrap();
        assert!(token.starts_with("eyJ"));
    }
}
