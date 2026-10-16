use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PonySentryConfig {
    pub endpoint: String,
    pub client_token: Option<String>,
    pub enabled: bool,
    pub environment: String,
    pub release: String,
}

impl Default for PonySentryConfig {
    fn default() -> Self {
        Self {
            endpoint: "https://sentry.ponyjob.top".to_string(),
            // wave-18 C1：默认 fail-closed —— 不得内置任何凭证，默认零 egress。
            // 凭证的唯一合法来源是环境变量（见 `from_env()`）。
            client_token: None,
            enabled: false,
            environment: "dev".to_string(),
            release: env!("CARGO_PKG_VERSION").to_string(),
        }
    }
}

impl PonySentryConfig {
    pub fn from_env() -> Self {
        let endpoint = std::env::var("PONYSENTRY_INGEST_URL")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "https://sentry.ponyjob.top".to_string());

        // wave-18 C2：token 仅来自 `PONYSENTRY_CLIENT_TOKEN`；未设或纯空白 ⇒ None。
        // 禁止任何硬编码回落（默认零凭证 / fail-closed）。
        let client_token = std::env::var("PONYSENTRY_CLIENT_TOKEN")
            .ok()
            .filter(|s| !s.trim().is_empty());

        // wave-18 C2（r3）：未设 / set-but-empty / 纯空白 ⇒ false（fail-closed 零 egress）；
        // 仅显式非 "false"/"0"（含大小写与首尾空白）才为 true。
        let enabled = std::env::var("PONYSENTRY_ENABLED")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_lowercase() != "false" && s != "0")
            .unwrap_or(false);

        let environment = std::env::var("APP_ENV")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "dev".to_string());

        let release = std::env::var("APP_RELEASE")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());

        Self {
            endpoint,
            client_token,
            enabled,
            environment,
            release,
        }
    }
}
