pub mod client;
pub mod config;
pub mod models;
pub mod sanitizer;

pub use client::PonySentryClient;
pub use config::PonySentryConfig;
pub use models::{Breadcrumb, Exception, Frame, IngestPayload};
pub use sanitizer::{sanitize, sanitize_json};

use std::collections::HashMap;
use std::panic::PanicHookInfo;
use std::sync::{OnceLock, RwLock};

static GLOBAL_CLIENT: OnceLock<RwLock<PonySentryClient>> = OnceLock::new();

fn get_global() -> &'static RwLock<PonySentryClient> {
    GLOBAL_CLIENT.get_or_init(|| {
        let cfg = PonySentryConfig::from_env();
        RwLock::new(PonySentryClient::new(cfg))
    })
}

pub fn init(config: PonySentryConfig) {
    let client = PonySentryClient::new(config);
    if let Some(lock) = GLOBAL_CLIENT.get() {
        if let Ok(mut write_guard) = lock.write() {
            *write_guard = client;
            return;
        }
    }
    let _ = GLOBAL_CLIENT.set(RwLock::new(client));
}

pub fn capture_error(error_type: &str, message: &str, extra: Option<serde_json::Value>) {
    if let Ok(client) = get_global().read() {
        client.capture_error(error_type, message, extra);
    }
}

pub fn capture_payload(payload: IngestPayload) {
    if let Ok(client) = get_global().read() {
        client.capture_payload(payload);
    }
}

pub fn capture_panic(info: &PanicHookInfo) {
    if let Ok(client) = get_global().read() {
        client.capture_panic(info);
    }
}

pub fn add_breadcrumb(
    category: &str,
    message: &str,
    data: Option<HashMap<String, String>>,
) {
    if let Ok(client) = get_global().read() {
        client.add_breadcrumb(category, message, data);
    }
}

pub fn install_panic_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        capture_panic(info);
        prev(info);
    }));
}
