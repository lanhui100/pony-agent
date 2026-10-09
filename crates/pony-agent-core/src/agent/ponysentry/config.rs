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
            client_token: Some("6aa12e9e4294ddef559fd8f0d74626be9a313fad23a53868d5b07a88363c5d24".to_string()),
            enabled: true,
            environment: "dev".to_string(),
            release: "0.1.109".to_string(),
        }
    }
}

impl PonySentryConfig {
    pub fn from_env() -> Self {
        let endpoint = std::env::var("PONYSENTRY_INGEST_URL")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "https://sentry.ponyjob.top".to_string());

        let client_token = std::env::var("PONYSENTRY_CLIENT_TOKEN")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| Some("6aa12e9e4294ddef559fd8f0d74626be9a313fad23a53868d5b07a88363c5d24".to_string()));

        let enabled = std::env::var("PONYSENTRY_ENABLED")
            .map(|s| s.trim().to_lowercase() != "false" && s.trim() != "0")
            .unwrap_or(true);

        let environment = std::env::var("APP_ENV")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "dev".to_string());

        let release = std::env::var("APP_RELEASE")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "0.1.109".to_string());

        Self {
            endpoint,
            client_token,
            enabled,
            environment,
            release,
        }
    }
}
