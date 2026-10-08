use super::config::PonySentryConfig;
use std::collections::HashMap;
use std::panic::PanicHookInfo;

#[derive(Clone)]
pub struct PonySentryClient {
    pub config: PonySentryConfig,
}

impl PonySentryClient {
    pub fn new(config: PonySentryConfig) -> Self {
        let _ = config;
        unimplemented!()
    }

    pub fn capture_error(&self, error_type: &str, message: &str, extra: Option<serde_json::Value>) {
        let _ = (error_type, message, extra);
        unimplemented!()
    }

    pub fn capture_payload(&self, payload: super::models::IngestPayload) {
        let _ = payload;
        unimplemented!()
    }

    pub fn capture_panic(&self, info: &PanicHookInfo) {
        let _ = info;
        unimplemented!()
    }

    pub fn add_breadcrumb(
        &self,
        category: &str,
        message: &str,
        data: Option<HashMap<String, String>>,
    ) {
        let _ = (category, message, data);
        unimplemented!()
    }

    pub fn flush(&self) {
        unimplemented!()
    }
}
