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
        unimplemented!()
    }
}

impl PonySentryConfig {
    pub fn from_env() -> Self {
        unimplemented!()
    }
}
