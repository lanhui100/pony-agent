//! PA-171 **C3 红相**验收：限流分类 + 快速失败 + 冷却窗口内不再发起 HTTP。
//!
//! 契约真值源：`management/task-system/03_TASKS/PA-171-goal-round-failure-resilience.md` C3。
//! 本文件由独立 Test Agent 冻结；Executor 只读，禁改。
//!
//! 缺陷面（已取证，`provider/mod.rs`）：sync 客户端 `.timeout(45s)` 是整请求总时限；
//! 首跳失败只对同一 provider 按 **500ms→8s 短退避重试 ≤5 次**（`PROVIDER_TIMEOUT_RETRY_MAX_ATTEMPTS`），
//! 无冷却/熔断，`Retry-After` 是死码（`retry.rs:270-293` 零生产调用点）⇒ 一次 429 配额耗尽
//! 被放大成"每轮 5 次 HTTP × 16 轮"。
//!
//! 断言口径（**不锁实现、只锁可观察行为**）：
//! 1. 限流/配额文本必须被判为限流类（`retry::is_rate_limit_error`，单一出口）；
//! 2. 限流命中后单次调用**只发 1 次 HTTP**（不得短退避打 5 次），失败文案可读地给出
//!    恢复时间（`Retry-After` / 恢复 / 冷却）；
//! 3. 冷却窗口内再次调用**发 0 次 HTTP**（直接返回可读失败）；
//! 4. 反向钉子：非限流类瞬时错误（503）必须**保留**既有重试（不得一刀切禁重试）。
//!
//! 全部经**本地回环 mock provider**（`127.0.0.1:0`，随机端口）驱动，不联网、不依赖外网。
//! 计数法是真值：mock 侧统计真实到达的 HTTP 请求数。

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use pony_agent_core::agent::config::{
    ProviderModelCapabilities, ResolvedProviderSelection, ThinkingParamPattern,
};
use pony_agent_core::agent::provider::{
    ProviderAuthType, ProviderManager, ProviderMessage, ProviderProtocol, ProviderRequest,
};
use pony_agent_core::agent::retry::is_rate_limit_error;

/// Kimi 真实配额耗尽报文（诊断来源会话 10:28:51 的响应正文形态）。
const QUOTA_BODY: &str = "{\"error\":{\"type\":\"exceeded_current_quota_error\",\"message\":\"You've reached your 5-hour usage limit, resets at 06:01 UTC\"}}";

/// C3 要求被识别为限流类的错误文本（前四条既有语义，后两条是本次新增的可达形态）。
const RATE_LIMIT_TEXTS: [&str; 6] = [
    // ① 标准 429 状态行（provider 同步路径的真实格式）。
    "provider 返回错误状态：429 Too Many Requests；耗时=12ms；响应正文：{\"error\":{\"message\":\"slow down\"}}",
    // ② 429 + rate_limit 措辞。
    "provider 返回错误状态：429；{\"error\":{\"type\":\"rate_limit_exceeded\"}}",
    // ③ 纯措辞（无状态码）。
    "Error: too many requests, please retry later",
    // ④ Kimi/微信网关的配额错误类型（诊断会话真实报文）。
    QUOTA_BODY,
    // ⑤ 5 小时用量上限的**纯文案**（无 quota 字样，历史漏判 ⇒ 误走短退避重试）。
    "{\"error\":{\"message\":\"You've reached your 5-hour usage limit\"}}",
    // ⑥ 额度耗尽措辞（"out of its allowance"，历史漏判）。
    "Request denied: the model is out of its allowance for this billing window",
];

// ── 本地回环 mock provider（计数 + 固定状态码应答） ─────────────────────────────────

/// 固定返回某个状态码的 mock provider；`requests` 统计**真实到达**的 HTTP 请求数。
struct StaticStatusProvider {
    base_url: String,
    addr: String,
    requests: Arc<AtomicUsize>,
    shutdown: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl StaticStatusProvider {
    fn start(status: u16, reason: &str, body: &'static str, retry_after: Option<&'static str>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("PA-171：mock provider 必须能绑定回环端口");
        let port = listener
            .local_addr()
            .expect("PA-171：mock provider 必须能取本地地址")
            .port();
        let requests = Arc::new(AtomicUsize::new(0));
        let shutdown = Arc::new(AtomicBool::new(false));
        let requests_for_thread = Arc::clone(&requests);
        let shutdown_for_thread = Arc::clone(&shutdown);
        let reason = reason.to_string();

        let handle = std::thread::spawn(move || {
            while !shutdown_for_thread.load(Ordering::SeqCst) {
                let Ok((mut stream, _addr)) = listener.accept() else {
                    return;
                };
                // 读完请求（含 body），再应答：避免客户端写一半就被 close。
                let _ = read_http_request(&mut stream);
                requests_for_thread.fetch_add(1, Ordering::SeqCst);
                let mut head = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
                    body.as_bytes().len()
                );
                if let Some(retry_after) = retry_after {
                    head.push_str(&format!("Retry-After: {retry_after}\r\n"));
                }
                head.push_str("\r\n");
                head.push_str(body);
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.flush();
            }
        });

        Self {
            base_url: format!("http://127.0.0.1:{port}/v1"),
            addr: format!("127.0.0.1:{port}"),
            requests,
            shutdown,
            handle: Some(handle),
        }
    }

    fn request_count(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }
}

impl Drop for StaticStatusProvider {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        // 用一次空连接唤醒 accept，令 mock 线程及时退出。
        let _ = TcpStream::connect(self.addr.clone());
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn read_http_request(stream: &mut TcpStream) -> String {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 4096];
    let mut header_end: Option<usize> = None;
    let mut content_length = 0usize;
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => buffer.extend_from_slice(&chunk[..read]),
            Err(_) => break,
        }
        if header_end.is_none() {
            if let Some(index) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
                header_end = Some(index + 4);
                let headers = String::from_utf8_lossy(&buffer[..index + 4]).to_lowercase();
                for line in headers.lines() {
                    if let Some(value) = line.strip_prefix("content-length:") {
                        content_length = value.trim().parse::<usize>().unwrap_or(0);
                    }
                }
            }
        }
        if let Some(header_end) = header_end {
            if buffer.len().saturating_sub(header_end) >= content_length {
                break;
            }
        }
    }
    let body_start = header_end.unwrap_or(buffer.len());
    String::from_utf8_lossy(&buffer[body_start..]).to_string()
}

fn provider_selection(base_url: String) -> ResolvedProviderSelection {
    ResolvedProviderSelection {
        requested_name: "pa171-rate-limit-test".to_string(),
        provider_name: "pa171-rate-limit-test".to_string(),
        protocol: ProviderProtocol::OpenAiCompletions,
        base_url,
        auth_type: ProviderAuthType::Auto,
        api_key_env_var: "PA171_RATE_LIMIT_TEST_API_KEY".to_string(),
        api_key: Some("pa171-dummy-key".to_string()),
        model: "pa171-rate-limit-model".to_string(),
        temperature: 0.0,
        max_output_tokens: 64,
        reasoning_effort: None,
        reasoning_budget_tokens: None,
        capabilities: ProviderModelCapabilities {
            context_window_tokens: Some(8_192),
            supports_tools: true,
            supports_streaming: false,
            supports_image_input: false,
            supports_reasoning: false,
            ..Default::default()
        },
        thinking_param_pattern: ThinkingParamPattern::EffortStandard,
    }
}

fn request_probe() -> ProviderRequest {
    ProviderRequest {
        model: "pa171-rate-limit-model".to_string(),
        input: vec![ProviderMessage::user("pa171 probe")],
        images: Vec::new(),
        native_messages: Vec::new(),
        observation: Default::default(),
        temperature: 0.0,
        max_output_tokens: 64,
    }
}

/// 失败文案必须可读地给出"何时能再试"（`Retry-After` / 恢复时间 / 冷却），
/// 而不是只把原始错误整段丢给用户或折叠成「重试已用尽」。
fn mentions_recovery_window(error: &str) -> bool {
    let lower = error.to_lowercase();
    lower.contains("retry-after")
        || lower.contains("retry_after")
        || error.contains("恢复")
        || error.contains("冷却")
        || lower.contains("reset")
        || lower.contains("06:01")
}

// ── 断言 1：限流类文本分类 ──────────────────────────────────────────────────────────

/// C3-a：六条限流/配额文本必须被判为限流类（`is_rate_limit_error` 单一出口）。
#[test]
fn c3_rate_limit_texts_are_classified_as_rate_limited() {
    for text in RATE_LIMIT_TEXTS {
        assert!(
            is_rate_limit_error(text),
            "C3：文本必须被判为限流类（否则会走 500ms→8s 短退避打 5 次），实际漏判：{text}"
        );
    }
}

/// C3-a 反向钉子：非限流文本不得误判为限流（防止把「所有错误都当限流」的假修复）。
#[test]
fn c3_non_rate_limit_texts_are_not_classified_as_rate_limited() {
    for text in [
        "operation timed out after 45002ms",
        "provider 返回错误状态：401 Unauthorized",
        "connection reset by peer",
        "invalid request: missing field `messages`",
    ] {
        assert!(
            !is_rate_limit_error(text),
            "C3：非限流文本不得判为限流类，实际误判：{text}"
        );
    }
}

// ── 断言 2：限流命中后快速失败（单次 HTTP） ────────────────────────────────────────

/// C3-b：一次 429 配额耗尽只允许发 **1 次** HTTP（不得短退避重试 5 次），
/// 且失败文案可读地给出恢复窗口。
#[test]
fn c3_rate_limited_call_fails_fast_with_a_single_http_attempt() {
    // Arrange
    let server = StaticStatusProvider::start(429, "Too Many Requests", QUOTA_BODY, Some("3600"));
    let manager = ProviderManager::new(provider_selection(server.base_url.clone()));
    let request = request_probe();

    // Act
    let started = Instant::now();
    let result = manager.decide_with_tools(&request, &[]);
    let elapsed = started.elapsed();
    let attempts = server.request_count();

    // Assert
    let error = result.expect_err("C3：429 配额耗尽必须返回 Err（不得当成功）");
    assert_eq!(
        attempts, 1,
        "C3：限流命中后必须快速失败（只发 1 次 HTTP），实际 {attempts} 次；\
         调用耗时 {elapsed:?}（500ms→8s 短退避 ×5 的指纹是 ~7.5s/5 次）；错误：{error}"
    );
    assert!(
        elapsed < Duration::from_secs(3),
        "C3：限流失败必须快速返回（<3s），实际 {elapsed:?}（短退避指纹）"
    );
    assert!(
        mentions_recovery_window(&error),
        "C3：限流失败必须可读地给出恢复时间（Retry-After / 恢复 / 冷却），实际：{error}"
    );
}

// ── 断言 3：冷却窗口内不再发起 HTTP ───────────────────────────────────────────────

/// C3-c：首次 429 后进入冷却；冷却窗口内的再调用必须 **发 0 次** HTTP，
/// 直接返回可读失败（历史缺陷：每轮再打 5 次 ⇒ 16 轮 × 5 次）。
#[test]
fn c3_cooldown_window_blocks_further_http_attempts() {
    // Arrange
    let server = StaticStatusProvider::start(429, "Too Many Requests", QUOTA_BODY, Some("3600"));
    let manager = ProviderManager::new(provider_selection(server.base_url.clone()));
    let request = request_probe();

    // Act：第一次失败 ⇒ 进入冷却。
    let first = manager.decide_with_tools(&request, &[]);
    let after_first = server.request_count();
    assert!(
        first.is_err(),
        "C3：第一次 429 必须返回 Err，实际 {:?}",
        first.map(|decision| decision.provider_mode)
    );
    assert!(
        after_first <= 1,
        "C3：首次限流失败就应进入冷却（≤1 次 HTTP），实际 {after_first} 次"
    );

    let second = manager.decide_with_tools(&request, &[]);
    let after_second = server.request_count();

    // Assert：冷却窗口内第二条调用不得产生任何新 HTTP 请求。
    let second_error = second.expect_err("C3：冷却窗口内的调用必须返回可读失败（不得当作成功）");
    assert_eq!(
        after_second, after_first,
        "C3：冷却窗口内不得再发起 HTTP（历史缺陷：每轮重放 5 次），实际新增 {} 次",
        after_second - after_first
    );
    assert!(
        mentions_recovery_window(&second_error),
        "C3：冷却期失败必须可读地给出恢复窗口（Retry-After / 恢复 / 冷却），实际：{second_error}"
    );
}

// ── 断言 4：反向钉子——非限流瞬时错误保留既有重试 ─────────────────────────────────

/// C3-d：503 仍属可重试瞬时错误，必须保留既有重试（禁止用"全部快速失败"通过本卡）。
#[test]
fn c3_generic_transient_error_keeps_existing_retries() {
    // Arrange
    let server = StaticStatusProvider::start(
        503,
        "Service Unavailable",
        "{\"error\":{\"message\":\"upstream temporarily unavailable\"}}",
        None,
    );
    let manager = ProviderManager::new(provider_selection(server.base_url.clone()));
    let request = request_probe();

    // Act
    let result = manager.decide_with_tools(&request, &[]);

    // Assert
    assert!(result.is_err(), "C3：503 必须最终返回 Err");
    assert!(
        server.request_count() >= 2,
        "C3 反向钉子：非限流类瞬时错误必须保留既有重试（≥2 次 HTTP），实际 {} 次\
         ——不得为了通过限流快速失败而一刀切禁掉所有重试",
        server.request_count()
    );
}
