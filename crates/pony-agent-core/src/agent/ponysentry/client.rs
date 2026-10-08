use super::config::PonySentryConfig;
use super::models::{AgentTracePayload, Breadcrumb, Exception, Frame, IngestPayload};
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
    Trace(AgentTracePayload),
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

        let sender = if config.enabled {
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
        if !self.config.enabled {
            return;
        }

        let sanitized_msg = sanitize(message);
        let sanitized_extra = extra.map(|ex| sanitize_json(&ex));
        let breadcrumbs = self.take_breadcrumbs();

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
            tags: None,
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
        if !self.config.enabled {
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
            tags: None,
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
        if !self.config.enabled {
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

    pub fn capture_agent_trace(&self, mut trace: AgentTracePayload) {
        if !self.config.enabled {
            return;
        }

        // 零信任脱敏管道处理
        trace.session_id = sanitize(&trace.session_id);
        trace.run_id = trace.run_id.take().map(|r| sanitize(&r));
        trace.turn_id = trace.turn_id.take().map(|t| sanitize(&t));

        if let Some(tags) = trace.tags.take() {
            trace.tags = Some(
                tags.into_iter()
                    .map(|(k, v)| (sanitize(&k), sanitize(&v)))
                    .collect(),
            );
        }

        if let Some(extra) = trace.extra.take() {
            trace.extra = Some(sanitize_json(&extra));
        }

        for turn in &mut trace.turns {
            turn.turn_id = sanitize(&turn.turn_id);
            turn.phase = turn.phase.take().map(|p| sanitize(&p));
            turn.provider = turn.provider.take().map(|p| sanitize(&p));
            turn.model = turn.model.take().map(|m| sanitize(&m));
            turn.error = turn.error.take().map(|e| sanitize(&e));
            for tool_call in &mut turn.tool_calls {
                tool_call.call_id = tool_call.call_id.take().map(|c| sanitize(&c));
                tool_call.tool_name = sanitize(&tool_call.tool_name);
                tool_call.arguments_summary =
                    tool_call.arguments_summary.take().map(|a| sanitize(&a));
                tool_call.status = sanitize(&tool_call.status);
                tool_call.error = tool_call.error.take().map(|e| sanitize(&e));
            }
        }

        if let Some(ref tx) = self.sender {
            match tx.try_send(SentryEvent::Trace(trace)) {
                Ok(()) => {}
                Err(TrySendError::Full(_)) => {
                    eprintln!("[ponysentry] queue full (1024), dropping trace to protect runtime");
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
    let trace_url = format!("{base_url}/api/v1/traces");

    while let Ok(event) = receiver.recv() {
        let (url, body_json) = match event {
            SentryEvent::Ingest(payload) => (ingest_url.clone(), serde_json::to_value(&payload)),
            SentryEvent::Trace(trace) => (trace_url.clone(), serde_json::to_value(&trace)),
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
