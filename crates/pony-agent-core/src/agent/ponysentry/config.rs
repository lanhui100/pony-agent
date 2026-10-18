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
            // 凭证的唯一合法来源是环境变量或构建期注入（见 `from_env()`）。
            client_token: None,
            // 上报开关恒开：是否真正上报由 client_token 是否配置决定（有 token 才有 egress）。
            enabled: true,
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

        // wave-18 C2：token 来源优先级：运行时 env `PONYSENTRY_CLIENT_TOKEN` >
        // 构建期注入 `PONYSENTRY_BUILD_CLIENT_TOKEN`（release 流水线经 env 烧录，
        // 桌面端用户机器无需配置任何环境变量）；均未设或纯空白 ⇒ None。
        // 禁止任何硬编码回落（默认零凭证 / fail-closed）。
        let client_token = std::env::var("PONYSENTRY_CLIENT_TOKEN")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| {
                option_env!("PONYSENTRY_BUILD_CLIENT_TOKEN")
                    .map(|s| s.to_string())
                    .filter(|s| !s.trim().is_empty())
            });

        // 上报开关恒开（无独立开关变量）；client_token 为 None 时客户端自然零 egress。
        let enabled = true;

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
