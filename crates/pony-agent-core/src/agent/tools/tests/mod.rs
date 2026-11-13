//! `ToolRouter` 内置工具测试集（自 `agent/tools.rs` 拆分，纯搬移：断言与夹具逐字不变）。

use super::*;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::{SystemTime, UNIX_EPOCH};
use crate::agent::tool_runtime::FakeResolver;
use std::collections::HashMap;
use std::sync::Mutex;
use crate::agent::tools::composite::MAX_GATHER_CONTEXT_PATHS;
use crate::agent::tools::composite::MAX_WORKSPACE_BATCH_CALLS;
use crate::agent::tools::exec::error_result_with_evidence;
use crate::agent::tools::paths::normalize_if_verbatim;
use crate::agent::tools::run::build_windows_batch_script;
use crate::agent::tools::run::posix_inline_env_assignment_reason;
use crate::agent::tools::run::set_promote_on_timeout_override;
use crate::agent::tools::run::windows_command_not_found_hint;
use crate::agent::tools::run::workspace_command_parts_with_batch;
use crate::agent::tools::web::PinnedFetchError;
use crate::agent::tools::web::PinnedHttpExchange;
use crate::agent::tools::web::PinnedSender;
use crate::agent::tools::web::ReqwestPinnedSender;
use crate::agent::tools::web::WebFetchTransportError;
use crate::agent::tools::web::build_pinned_web_client;
use crate::agent::tools::web::exa_search_request_body;
use crate::agent::tools::web::is_tool_timeout_message;
use crate::agent::tools::web::pinned_web_fetch;
use crate::agent::tools::web::retry_tool_timeout;
use crate::agent::tools::web::verify_pinned_peer;

mod orchestration;
mod run;
mod composite;
mod workspace;
mod web;


fn temp_workspace() -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("pony-agent-tools-test-{}", unique));
    fs::create_dir_all(&dir).expect("create temp workspace");
    dir.canonicalize().unwrap_or(dir)
}

fn serve_single_http_response(status_line: &str, body: &str, content_type: &str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test http server");
    let address = listener.local_addr().expect("read test server addr");
    let status_line = status_line.to_string();
    let body = body.to_string();
    let content_type = content_type.to_string();

    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept test request");
        let mut buffer = [0_u8; 4096];
        let _ = stream.read(&mut buffer);
        let response = format!(
            "{status_line}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream
            .write_all(response.as_bytes())
            .expect("write test response");
        stream.flush().expect("flush test response");
    });

    format!("http://{}", address)
}

fn serve_timeout_http_response(delay_ms: u64) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test http server");
    let address = listener.local_addr().expect("read test server addr");

    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept test request");
        let mut buffer = [0_u8; 4096];
        let _ = stream.read(&mut buffer);
        thread::sleep(Duration::from_millis(delay_ms));
        let _ = stream.write_all(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: 7\r\nConnection: close\r\n\r\ntimeout",
        );
        let _ = stream.flush();
    });

    format!("http://{}", address)
}

fn with_env_var<R>(key: &str, value: &str, operation: impl FnOnce() -> R) -> R {
    let previous = std::env::var(key).ok();
    unsafe {
        std::env::set_var(key, value);
    }
    let result = operation();
    match previous {
        Some(previous_value) => unsafe {
            std::env::set_var(key, previous_value);
        },
        None => unsafe {
            std::env::remove_var(key);
        },
    }
    result
}

/// Runs a single-child `workspace_batch` and asserts the fail-closed rejection:
/// status `error`, error code `unsupported_composite_child`, and a message that
/// names the offending child and its permission scope. Returns the parsed payload.
fn assert_batch_child_rejected(
    router: &ToolRouter,
    child: Value,
    expected_scope: &str,
) -> Value {
    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_BATCH.to_string(),
        arguments: json!({ "calls": [child] }),
        plan: None,
    });
    assert_eq!(result.status, "error");
    let payload = serde_json::from_str::<Value>(&result.output).expect("batch output json");
    let error = payload.get("error").expect("structured rejection error");
    assert_eq!(
        error.get("code").and_then(Value::as_str),
        Some("unsupported_composite_child")
    );
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .expect("rejection message");
    assert!(
        message.contains(expected_scope),
        "rejection message should name scope `{}`: {}",
        expected_scope,
        message
    );
    payload
}

/// Build one scripted [`PinnedHttpExchange`] for use in [`FakePinnedSender`] redirect-chain
/// tests. `status` is the HTTP status code; `location` (when Some) sets the Location header;
/// `body` sets the response body on terminal hops.
fn exchange(
    url: &str,
    status: u16,
    location: Option<&str>,
    body: &str,
) -> Result<PinnedHttpExchange, WebFetchTransportError> {
    let mut headers = reqwest::header::HeaderMap::new();
    if let Some(loc) = location {
        headers.insert(
            reqwest::header::LOCATION,
            reqwest::header::HeaderValue::from_str(loc).unwrap(),
        );
    }
    if !body.is_empty() {
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            reqwest::header::HeaderValue::from_static("text/html; charset=utf-8"),
        );
    }
    Ok(PinnedHttpExchange {
        url: url.to_string(),
        status: reqwest::StatusCode::from_u16(status).unwrap(),
        headers,
        body: body.as_bytes().to_vec(),
        body_truncated: false,
        peer_ip: Some("203.0.113.10".parse().unwrap()),
    })
}

/// A deadline far in the future so redirect-budget/chain tests never trip the aggregate
/// chain deadline (they only exercise hop validation, not the time bound).
fn generous_deadline() -> Instant {
    Instant::now() + Duration::from_millis(60_000)
}

/// A scripted [`PinnedSender`] for hermetic tests of the redirect re-validation logic.
/// Entries are keyed by URL; every call is recorded for assertions.
#[derive(Default)]
struct FakePinnedSender {
    responses: HashMap<String, Result<PinnedHttpExchange, WebFetchTransportError>>,
    calls: Mutex<Vec<(String, WebAccessDecision)>>,
}

impl FakePinnedSender {
    fn with_script(
        entries: &[(&str, Result<PinnedHttpExchange, WebFetchTransportError>)],
    ) -> Self {
        Self {
            responses: entries
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect(),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<(String, WebAccessDecision)> {
        self.calls.lock().unwrap().clone()
    }
}

impl PinnedSender for FakePinnedSender {
    fn send(
        &self,
        url: &str,
        decision: &WebAccessDecision,
        _timeout_ms: u64,
    ) -> Result<PinnedHttpExchange, WebFetchTransportError> {
        self.calls
            .lock()
            .unwrap()
            .push((url.to_string(), decision.clone()));
        self.responses.get(url).cloned().unwrap_or_else(|| {
            Err(WebFetchTransportError::Request(format!(
                "no scripted response for `{url}`"
            )))
        })
    }
}

// ---------- redirect re-validation (hermetic, no sockets) ----------

/// A scripted [`PinnedSender`] whose hops take `delay_ms` each and record the per-hop timeout
/// they were granted. Models a slow server chain: when `respect_timeout` is set and the granted
/// hop timeout (the remaining chain budget) cannot cover the delay, the production reqwest
/// client would abort with a timeout, so this fake returns a `timeout:` transport error; when
/// `respect_timeout` is false it always sleeps, letting the chain-level deadline cut the fetch.
struct DelayedPinnedSender {
    responses: HashMap<String, Result<PinnedHttpExchange, WebFetchTransportError>>,
    delay_ms: u64,
    respect_timeout: bool,
    calls: Mutex<Vec<(String, u64)>>,
    /// 每次调用的**采样时刻**（与 `calls` 一一对应）：把时序断言自校准到「同一 deadline 在该
    /// 时刻的剩余量」，取代「测试准备阶段必须在固定 slack 内跑完」这种负载敏感假设。
    call_instants: Mutex<Vec<(String, u64, Instant)>>,
}

impl DelayedPinnedSender {
    fn with_script(
        entries: &[(&str, Result<PinnedHttpExchange, WebFetchTransportError>)],
        delay_ms: u64,
        respect_timeout: bool,
    ) -> Self {
        Self {
            responses: entries
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect(),
            delay_ms,
            respect_timeout,
            calls: Mutex::new(Vec::new()),
            call_instants: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<(String, u64)> {
        self.calls.lock().unwrap().clone()
    }

    /// `(url, 被授予的超时 ms, 采样时刻)`：时序断言以采样时刻的剩余预算为基准。
    fn calls_with_instant(&self) -> Vec<(String, u64, Instant)> {
        self.call_instants.lock().unwrap().clone()
    }
}

impl PinnedSender for DelayedPinnedSender {
    fn send(
        &self,
        url: &str,
        _decision: &WebAccessDecision,
        timeout_ms: u64,
    ) -> Result<PinnedHttpExchange, WebFetchTransportError> {
        let sampled_at = Instant::now();
        self.calls
            .lock()
            .unwrap()
            .push((url.to_string(), timeout_ms));
        self.call_instants.lock().unwrap().push((
            url.to_string(),
            timeout_ms,
            sampled_at,
        ));
        if self.respect_timeout && self.delay_ms >= timeout_ms {
            return Err(WebFetchTransportError::Request(format!(
                "timeout: 抓取 URL 超时（该跳 granted {timeout_ms} ms）。"
            )));
        }
        thread::sleep(Duration::from_millis(self.delay_ms));
        self.responses.get(url).cloned().unwrap_or_else(|| {
            Err(WebFetchTransportError::Request(format!(
                "no scripted response for `{url}`"
            )))
        })
    }
}

// ---------- chain-level aggregate deadline (design Decision 8 / tasks 6.2-6.3) ----------

/// Two-argument convenience form of the `ProcessStartRequest` builder (A3 contract): delegates to
/// [`workspace_command_parts_with_batch`] with no batch file, so on non-Windows it keeps the
/// historical `("sh", ["-lc", "cd '<cwd>' && <command>"])` behavior byte-for-byte. Test-only:
/// Windows production code must use [`workspace_command_parts_with_batch`] (the Windows branch
/// panics if the batch-file path is absent).
#[cfg(test)]
fn workspace_command_parts(command: &str, cwd: &Path) -> (String, Vec<String>) {
    workspace_command_parts_with_batch(command, cwd, None)
}
