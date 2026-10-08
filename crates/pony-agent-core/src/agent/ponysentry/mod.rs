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

pub fn init(config: PonySentryConfig) {
    let _ = config;
    unimplemented!()
}

pub fn capture_error(error_type: &str, message: &str, extra: Option<serde_json::Value>) {
    let _ = (error_type, message, extra);
    unimplemented!()
}

pub fn capture_payload(payload: IngestPayload) {
    let _ = payload;
    unimplemented!()
}

pub fn capture_panic(info: &PanicHookInfo) {
    let _ = info;
    unimplemented!()
}

pub fn add_breadcrumb(
    category: &str,
    message: &str,
    data: Option<HashMap<String, String>>,
) {
    let _ = (category, message, data);
    unimplemented!()
}

pub fn install_panic_hook() {
    unimplemented!()
}
