use super::config::PonySentryConfig;
use super::models::{Breadcrumb, Exception, Frame, IngestPayload};
use super::sanitizer::{sanitize, sanitize_json};
use std::collections::{HashMap, VecDeque};
use std::panic::PanicHookInfo;
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const MAX_BREADCRUMBS: usize = 64;
const QUEUE_CAPACITY: usize = 1024;
const REQUEST_TIMEOUT_SECS: u64 = 3;

#[derive(Debug)]
enum SentryEvent {
    Ingest(IngestPayload),
}

#[derive(Clone)]
pub struct PonySentryClient {
    pub config: PonySentryConfig,
    sender: Option<SyncSender<SentryEvent>>,
    breadcrumbs: Arc<Mutex<VecDeque<Breadcrumb>>>,
}

impl PonySentryClient {
    pub fn new(config: PonySentryConfig) -> Self {
        let breadcrumbs = Arc::new(Mutex::new(VecDeque::with_capacity(MAX_BREADCRUMBS)));

        // fail-closed 门禁：仅当 client_token 已配置时才 spawn worker（无 token 即零 egress）。
        let sender = if config.enabled
            && config
                .client_token
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .is_some()
        {
            let (tx, rx) = mpsc::sync_channel::<SentryEvent>(QUEUE_CAPACITY);
            let worker_cfg = config.clone();
            let _handle: Option<JoinHandle<()>> = thread::Builder::new()
                .name("ponysentry-worker".to_string())
                .spawn(move || {
                    worker_loop(worker_cfg, rx);
                })
                .ok();
            Some(tx)
        } else {
            None
        };

        Self {
            config,
            sender,
            breadcrumbs,
        }
    }

    pub fn add_breadcrumb(
        &self,
        category: &str,
        message: &str,
        data: Option<HashMap<String, String>>,
    ) {
        let sanitized_msg = sanitize(message);
        let sanitized_data = data.map(|map| {
            map.into_iter()
                .map(|(k, v)| (k, sanitize(&v)))
                .collect::<HashMap<String, String>>()
        });

        let mut queue = self
            .breadcrumbs
            .lock()
            .unwrap_or_else(|e| e.into_inner());

        if queue.len() >= MAX_BREADCRUMBS {
            queue.pop_front();
        }

        queue.push_back(Breadcrumb {
            category: category.to_string(),
            message: sanitized_msg,
            data: sanitized_data,
        });
    }

    fn take_breadcrumbs(&self) -> Vec<Breadcrumb> {
        let mut queue = self
            .breadcrumbs
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        queue.drain(..).collect()
    }

    pub fn capture_error(&self, error_type: &str, message: &str, extra: Option<serde_json::Value>) {
        // fail-closed：无 token 即零 egress（enabled 恒开，门禁由 token 承担）
        if !self.config.enabled || self.config.client_token.is_none() {
            return;
        }

        let sanitized_msg = sanitize(message);
        let sanitized_extra = extra.map(|ex| sanitize_json(&ex));
        let breadcrumbs = self.take_breadcrumbs();

        let mut tags = HashMap::new();
        tags.insert("project".to_string(), "pony-agent".to_string());

        let payload = IngestPayload {
            platform: "rust".to_string(),
            release: self.config.release.clone(),
            environment: self.config.environment.clone(),
            message: Some(sanitized_msg.clone()),
            exception: Some(Exception {
                error_type: error_type.to_string(),
                value: Some(sanitized_msg),
                stacktrace: None,
            }),
            tags: Some(tags),
            extra: sanitized_extra,
            breadcrumbs: if breadcrumbs.is_empty() {
                None
            } else {
                Some(breadcrumbs)
            },
        };

        self.capture_payload(payload);
    }

    pub fn capture_panic(&self, info: &PanicHookInfo) {
        // fail-closed：无 token 即零 egress（enabled 恒开，门禁由 token 承担）
        if !self.config.enabled || self.config.client_token.is_none() {
            return;
        }

        let message = info
            .payload()
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| {
                info.payload()
                    .downcast_ref::<&str>()
                    .map(|s| s.to_string())
            })
            .unwrap_or_else(|| "unknown panic".to_string());

        let location = info.location().map(|l| l.to_string());
        let sanitized_msg = sanitize(&message);
        let breadcrumbs = self.take_breadcrumbs();

        let mut tags = HashMap::new();
        tags.insert("project".to_string(), "pony-agent".to_string());

        let payload = IngestPayload {
            platform: "rust".to_string(),
            release: self.config.release.clone(),
            environment: self.config.environment.clone(),
            message: Some(format!("panic: {sanitized_msg}")),
            exception: Some(Exception {
                error_type: "Panic".to_string(),
                value: Some(sanitized_msg),
                stacktrace: Some(vec![Frame {
                    filename: location,
                    function: Some("panic_hook".to_string()),
                    lineno: None,
                    in_app: Some(true),
                }]),
            }),
            tags: Some(tags),
            extra: None,
            breadcrumbs: if breadcrumbs.is_empty() {
                None
            } else {
                Some(breadcrumbs)
            },
        };

        self.capture_payload(payload);
    }

    pub fn capture_payload(&self, payload: IngestPayload) {
        // fail-closed：无 token 即零 egress（enabled 恒开，门禁由 token 承担）
        if !self.config.enabled || self.config.client_token.is_none() {
            return;
        }

        if let Some(ref tx) = self.sender {
            match tx.try_send(SentryEvent::Ingest(payload)) {
                Ok(()) => {}
                Err(TrySendError::Full(_)) => {
                    eprintln!("[ponysentry] queue full (1024), dropping event to protect runtime");
                }
                Err(TrySendError::Disconnected(_)) => {
                    eprintln!("[ponysentry] worker disconnected");
                }
            }
        }
    }

    pub fn flush(&self) {
        // Queue is continuously processed by the worker thread
    }
}

fn worker_loop(config: PonySentryConfig, receiver: mpsc::Receiver<SentryEvent>) {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("[ponysentry] failed to create worker tokio runtime: {e}");
            return;
        }
    };
    let _guard = rt.enter();

    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[ponysentry] failed to build HTTP client: {e}");
            return;
        }
    };

    let base_url = config.endpoint.trim_end_matches('/');
    let ingest_url = format!("{base_url}/api/v1/ingest");

    while let Ok(event) = receiver.recv() {
        let (url, body_json) = match event {
            SentryEvent::Ingest(payload) => (ingest_url.clone(), serde_json::to_value(&payload)),
        };

        let body = match body_json {
            Ok(b) => b,
            Err(e) => {
                eprintln!("[ponysentry] Failed to serialize event: {e}");
                continue;
            }
        };

        let mut req = client
            .post(&url)
            .header("Content-Type", "application/json");

        if let Some(ref token) = config.client_token {
            req = req.header("X-Client-Token", token);
        }

        let send_fut = req.json(&body).send();

        rt.block_on(async {
            match send_fut.await {
                Ok(resp) => {
                    if !resp.status().is_success() {
                        eprintln!("[ponysentry] Ingest responded with status {}", resp.status());
                    }
                }
                Err(e) => {
                    eprintln!("[ponysentry] Failed to send event to {url}: {e}");
                }
            }
        });
    }
}
