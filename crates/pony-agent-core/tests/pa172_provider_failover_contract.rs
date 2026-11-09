//! PA-172 **P3/P4 红相**验收：跨 provider **有界**故障转移（failover）。
//!
//! 契约真值源：`.dev-team/contract-matrix-pa172.md`（**v2 收窄版**，Lead 裁决 2026-11-09）
//! - §3.2 触发类封闭值域（`failover_eligible` 复用既有码表，禁止第二套关键词表；
//!   `{rate_limited, quota, timeout, network, auth}` ∪ 码表层 `UpstreamServerError`，
//!   **HTTP 500 不在其中**）；
//! - §3.3 边界 F-a..F-h（hop 上限 / 冷却跳过 0 次 HTTP / 逐 attempt 预算 / 耗尽标记）；
//! - §3.3 v2：**参与换手的入口收窄为同步入口**（流式入口不换手，登记 deferred）；
//! - §3.3 v2：hop 循环位于 `retry_provider_scoped_with_report` **之外** ⇒ 每候选一份独立
//!   `RetryBudget`（3 个候选全 429 时每个候选各 1 次 HTTP）；
//! - §1 G4「不伪造成功」、G7「无新增线程/定时器」。
//!
//! 本文件由独立 Test Agent 冻结；Executor 只读，禁改。
//!
//! 判据口径（**只锁可观察行为，不锁实现**）：
//! 1. 换手的唯一证据是**对端真实收到的 HTTP 请求计数**（`127.0.0.1:0` 回环 mock）；
//! 2. 冷却跳过必须是 **0 次 HTTP**（不是「先发请求再判冷却」）；
//! 3. 耗尽必须返回**最后一次真实错误** + 可解析的 `failover_exhausted=` 标记，**不得** Ok；
//! 4. 流式入口**不参与**换手（不再区分「已发增量」——整类入口退出换手闭包）。
//!
//! 红相预期（wave-69 实现前）：`ProviderManager::with_failover` / `ProviderFailoverChain` /
//! `MAX_FAILOVER_HOPS` / `retry::failover_eligible` 等**签名锚点**不存在 ⇒ 编译红（E0432/E0599），
//! 这是 wave-68 D68-5 先例的合法红相形态。

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use pony_agent_core::agent::config::{
    ProviderModelCapabilities, ResolvedProviderSelection, ThinkingParamPattern, UpstreamPolicy,
};
use pony_agent_core::agent::provider::{
    ProviderAuthType, ProviderFailoverChain, ProviderManager, ProviderMessage, ProviderProtocol,
    ProviderRequest, FAILOVER_BUDGET_MS, MAX_FAILOVER_HOPS,
};
use pony_agent_core::agent::retry::{
    failover_eligible, parse_failover_exhausted_marker, FAILOVER_EXHAUSTED_MARKER,
};

/// 429 配额耗尽报文（与 PA-171 C3 同族，保证走既有 `rate_limited`/`quota` 码表）。
const RATE_LIMITED_BODY: &str = "{\"error\":{\"type\":\"exceeded_current_quota_error\",\"message\":\"You've reached your 5-hour usage limit, resets at 06:01 UTC\"}}";

/// 200 成功报文：只需满足 `first_openai_message` 的 `choices[0].message` 契约。
const OK_BODY: &str = "{\"id\":\"pa172-ok\",\"choices\":[{\"index\":0,\"message\":{\"role\":\"assistant\",\"content\":\"pa172-pong\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":3,\"total_tokens\":10}}";

/// 400 报文：`bad_request` ⇒ **不换手**（§3.2 封闭值域）。
const BAD_REQUEST_BODY: &str = "{\"error\":{\"type\":\"invalid_request_error\",\"message\":\"missing field messages\"}}";

/// 500 报文：既有冻结的 NonRetryable ⇒ **不换手**（v2 裁决：500 不在 server_error 码内）。
const INTERNAL_ERROR_BODY: &str =
    "{\"error\":{\"type\":\"internal_error\",\"message\":\"internal server error\"}}";

/// 503 报文：码表层 `UpstreamServerError` ⇒ **可换手**。
const SERVER_ERROR_BODY: &str =
    "{\"error\":{\"type\":\"server_error\",\"message\":\"upstream temporarily unavailable\"}}";

/// 流式剧本：先产出一个真实 delta，再来一个**畸形 JSON** 帧 ⇒ 流式中途失败。
const STREAM_DELTA_THEN_BROKEN: &str = concat!(
    "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"pa172-partial\"}}]}\n\n",
    "data: {this-is-not-json\n\n"
);

// ── 本地回环 mock provider（物理计数 + 固定应答剧本 + 可注入延迟） ────────────────────

struct MockReply {
    status: u16,
    reason: &'static str,
    body: &'static str,
    content_type: &'static str,
    retry_after: Option<&'static str>,
    /// 应答前的固定延迟（用于验证 per-attempt deadline；禁止测试里 sleep 概率等待）。
    delay: Duration,
}

/// 固定剧本的 mock provider；`requests` 统计**真实到达**的 HTTP 请求数（换手的唯一证据）。
struct MockProvider {
    base_url: String,
    addr: String,
    requests: Arc<AtomicUsize>,
    shutdown: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl MockProvider {
    fn start(reply: MockReply) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .expect("PA-172：mock provider 必须能绑定回环端口");
        let port = listener
            .local_addr()
            .expect("PA-172：mock provider 必须能取本地地址")
            .port();
        let requests = Arc::new(AtomicUsize::new(0));
        let shutdown = Arc::new(AtomicBool::new(false));
        let requests_for_thread = Arc::clone(&requests);
        let shutdown_for_thread = Arc::clone(&shutdown);

        let handle = std::thread::spawn(move || {
            while !shutdown_for_thread.load(Ordering::SeqCst) {
                let Ok((mut stream, _addr)) = listener.accept() else {
                    return;
                };
                // 读完请求（含 body）再应答，避免客户端写一半就被 close。
                let _ = read_http_request(&mut stream);
                // 唤醒用的空连接（Drop 里发起）不得计入真实请求数。
                if shutdown_for_thread.load(Ordering::SeqCst) {
                    return;
                }
                requests_for_thread.fetch_add(1, Ordering::SeqCst);
                if !reply.delay.is_zero() {
                    std::thread::sleep(reply.delay);
                }
                let mut head = format!(
                    "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n",
                    reply.status,
                    reply.reason,
                    reply.content_type,
                    reply.body.as_bytes().len()
                );
                if let Some(retry_after) = reply.retry_after {
                    head.push_str(&format!("Retry-After: {retry_after}\r\n"));
                }
                head.push_str("\r\n");
                head.push_str(reply.body);
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

    fn with_reply(reply: MockReply) -> Self {
        Self::start(reply)
    }

    /// 恒 429（带 `Retry-After` ⇒ 冷却窗口足够长，后续调用只会短路，不会重放）。
    fn rate_limited() -> Self {
        Self::with_reply(MockReply {
            status: 429,
            reason: "Too Many Requests",
            body: RATE_LIMITED_BODY,
            content_type: "application/json",
            retry_after: Some("3600"),
            delay: Duration::ZERO,
        })
    }

    /// 恒 200（换手目标成功）。
    fn healthy() -> Self {
        Self::with_reply(MockReply {
            status: 200,
            reason: "OK",
            body: OK_BODY,
            content_type: "application/json",
            retry_after: None,
            delay: Duration::ZERO,
        })
    }

    /// 恒 400（`bad_request` ⇒ 不换手）。
    fn bad_request() -> Self {
        Self::with_reply(MockReply {
            status: 400,
            reason: "Bad Request",
            body: BAD_REQUEST_BODY,
            content_type: "application/json",
            retry_after: None,
            delay: Duration::ZERO,
        })
    }

    /// 恒 500（NonRetryable ⇒ 不换手）。
    fn internal_error() -> Self {
        Self::with_reply(MockReply {
            status: 500,
            reason: "Internal Server Error",
            body: INTERNAL_ERROR_BODY,
            content_type: "application/json",
            retry_after: None,
            delay: Duration::ZERO,
        })
    }

    /// 慢应答：客户端必须按 per-attempt deadline 超时返回（不得等满 45s 基线）。
    fn slow(delay: Duration) -> Self {
        Self::with_reply(MockReply {
            status: 200,
            reason: "OK",
            body: OK_BODY,
            content_type: "application/json",
            retry_after: None,
            delay,
        })
    }

    /// 流式：先发一个真实 delta，再发畸形帧 ⇒ 流式中途失败。
    fn stream_delta_then_broken() -> Self {
        Self::with_reply(MockReply {
            status: 200,
            reason: "OK",
            body: STREAM_DELTA_THEN_BROKEN,
            content_type: "text/event-stream",
            retry_after: None,
            delay: Duration::ZERO,
        })
    }

    fn request_count(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }

    fn selection(&self, provider_name: &str) -> ResolvedProviderSelection {
        provider_selection(self.base_url.clone(), provider_name)
    }
}

impl Drop for MockProvider {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        // 空连接唤醒 accept，令 mock 线程及时退出。
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

fn provider_selection(base_url: String, provider_name: &str) -> ResolvedProviderSelection {
    ResolvedProviderSelection {
        requested_name: provider_name.to_string(),
        provider_name: provider_name.to_string(),
        protocol: ProviderProtocol::OpenAiCompletions,
        base_url,
        auth_type: ProviderAuthType::Auto,
        api_key_env_var: "PA172_FAILOVER_TEST_API_KEY".to_string(),
        api_key: Some("pa172-dummy-key".to_string()),
        model: format!("{provider_name}-model"),
        temperature: 0.0,
        max_output_tokens: 64,
        reasoning_effort: None,
        reasoning_budget_tokens: None,
        capabilities: ProviderModelCapabilities {
            context_window_tokens: Some(8_192),
            supports_tools: true,
            supports_streaming: true,
            supports_image_input: false,
            supports_reasoning: false,
            ..Default::default()
        },
        thinking_param_pattern: ThinkingParamPattern::EffortStandard,
    }
}

fn request_probe() -> ProviderRequest {
    ProviderRequest {
        model: "pa172-failover-model".to_string(),
        input: vec![ProviderMessage::user("pa172 probe")],
        images: Vec::new(),
        native_messages: Vec::new(),
        observation: Default::default(),
        temperature: 0.0,
        max_output_tokens: 64,
    }
}

/// 主 provider 头段文本（供 `failover_eligible` 真值表用；不发起 HTTP）。
fn status_error_text(status: u16, reason: &str, body: &str) -> String {
    format!("provider 返回错误状态：{status} {reason}；耗时=3ms；响应正文：{body}")
}

// ── F-a / §3.3：主 429 ⇒ 换手到备成功（hop 恰为 1） ────────────────────────────────

/// F-a/F-f：主 provider 恒 429、备 provider 返 200 ⇒ 结果 `Ok`，且**双方都收到真实 HTTP**。
///
/// `hop == 1` 的可观测等价面（比读日志更硬）：候选链给 2 个候选，
/// 断言「主 1 次 + 第一候选 1 次 + 第二候选 0 次」⇒ 恰好一跳。
#[test]
fn failover_primary_429_switches_to_secondary() {
    // Arrange
    let primary = MockProvider::rate_limited();
    let secondary = MockProvider::healthy();
    let third = MockProvider::healthy();
    let manager = ProviderManager::new(primary.selection("pa172-primary"))
        .with_failover(ProviderFailoverChain::new(vec![
            secondary.selection("pa172-secondary"),
            third.selection("pa172-third"),
        ]));
    let request = request_probe();

    // Act
    let result = manager.decide_with_tools(&request, &[]);
    let primary_hits = primary.request_count();
    let secondary_hits = secondary.request_count();
    let third_hits = third.request_count();

    // Assert
    let decision = result.expect("F-a：主 429 + 备 200 必须换手成功（Ok），不得把失败当终态");
    assert!(
        !decision.output_text.trim().is_empty(),
        "F-f：换手成功后必须返回备 provider 的真实产出，实际空文本"
    );
    assert!(
        primary_hits >= 1,
        "F-a：主 provider 必须真实收到 ≥1 次 HTTP（换手的前提是主先失败），实际 {primary_hits} 次"
    );
    assert_eq!(
        secondary_hits, 1,
        "F-a/F-d：首跳候选必须**恰好** 1 次 HTTP（独立 RetryBudget，限流类首跳即 Abort），\
         实际 {secondary_hits} 次"
    );
    assert_eq!(
        third_hits, 0,
        "F-a：hop 必须为 1（首跳即成功）⇒ 第二个候选必须 0 次 HTTP，实际 {third_hits} 次"
    );
}

// ── F-e / G4：全链耗尽返回最后一次真实错误 + 可解析标记 ────────────────────────────

/// F-e / G4：主 429 + 备 429 ⇒ `Err`，错误文本含 `failover_exhausted=` 且标记可解析；**不得** Ok。
#[test]
fn failover_exhausted_returns_last_error_with_marker() {
    // Arrange
    let primary = MockProvider::rate_limited();
    let secondary = MockProvider::rate_limited();
    let manager = ProviderManager::new(primary.selection("pa172-exhaust-primary"))
        .with_failover(ProviderFailoverChain::new(vec![secondary.selection(
            "pa172-exhaust-secondary",
        )]));
    let request = request_probe();

    // Act
    let result = manager.decide_with_tools(&request, &[]);

    // Assert
    let error = result.expect_err("G4：全链耗尽必须返回 Err（禁止把失败改写成成功）");
    assert!(
        error.contains(FAILOVER_EXHAUSTED_MARKER),
        "F-e：耗尽错误必须带结构后缀 `{FAILOVER_EXHAUSTED_MARKER}`，实际：{error}"
    );
    let hops = parse_failover_exhausted_marker(&error).unwrap_or_else(|| {
        panic!("F-e：耗尽标记必须可被 `parse_failover_exhausted_marker` 解析，实际：{error}")
    });
    assert!(
        (1..=MAX_FAILOVER_HOPS).contains(&hops),
        "F-a/F-e：耗尽 hops 必须落在 1..={MAX_FAILOVER_HOPS}，实际 {hops}；错误：{error}"
    );
    assert!(
        primary.request_count() >= 1 && secondary.request_count() >= 1,
        "F-e：耗尽前必须真实尝试过主与备（主 {} 次 / 备 {} 次）",
        primary.request_count(),
        secondary.request_count()
    );
    assert!(
        error.contains("429") || error.to_lowercase().contains("limit"),
        "F-e：耗尽必须返回**最后一次真实错误**原文（含上游 429/限额信息），实际：{error}"
    );
}

// ── F-b：冷却候选跳过 ⇒ 0 次 HTTP ────────────────────────────────────────────────

/// F-b：备 provider 已在冷却窗口 ⇒ 换手必须**跳过它（0 次 HTTP）**，不得先发请求再判冷却。
#[test]
fn failover_skips_cooled_candidate_without_http() {
    // Arrange：先用一次真实 429 把备 provider 置入冷却（生产同一条 `enter_cooldown` 路径）。
    let primary = MockProvider::rate_limited();
    let secondary = MockProvider::rate_limited();
    let secondary_selection = secondary.selection("pa172-cooled-secondary");
    let warmer = ProviderManager::new(secondary_selection.clone());
    let warm = warmer.decide_with_tools(&request_probe(), &[]);
    assert!(
        warm.is_err(),
        "F-b 前置：预热备 provider 的 429 必须返回 Err，实际 Ok"
    );
    let warmed_hits = secondary.request_count();
    assert!(
        warmer.cooldown_remaining().is_some(),
        "F-b 前置：备 provider 必须已进入冷却窗口（`cooldown_remaining` 为 Some），实际 None"
    );

    let manager = ProviderManager::new(primary.selection("pa172-cooled-primary"))
        .with_failover(ProviderFailoverChain::new(vec![secondary_selection]));
    let request = request_probe();

    // Act
    let result = manager.decide_with_tools(&request, &[]);

    // Assert
    let error = result.expect_err("F-b：唯一候选被冷却跳过 ⇒ 链实际为空 ⇒ 必须返回 Err");
    assert_eq!(
        secondary.request_count(),
        warmed_hits,
        "F-b：冷却中的候选必须 0 次 HTTP（不得「先发请求再判冷却」），实际新增 {} 次",
        secondary.request_count() - warmed_hits
    );
    assert!(
        primary.request_count() >= 1,
        "F-b：主 provider 必须先真实失败，实际 {} 次",
        primary.request_count()
    );
    assert!(
        !error.trim().is_empty(),
        "F-b：跳过冷却候选后必须给出可见失败原因，实际空错误"
    );
}

// ── F-a：hop 上限 + 每候选独立 RetryBudget（hop 循环在 retry 之外） ──────────────────

/// F-a / v2 第 3 条：候选链超长 ⇒ `ProviderFailoverChain::new` 截断到 `MAX_FAILOVER_HOPS`；
/// 全 429 时**每个**被尝试的候选各收到**恰好 1 次** HTTP（限流类 Decision 作用域首跳 Abort），
/// 实际发起 HTTP 的 provider 数 ≤ 1 + MAX_FAILOVER_HOPS。
#[test]
fn failover_respects_max_hops() {
    // Arrange
    let primary = MockProvider::rate_limited();
    let candidates: Vec<MockProvider> = (0..3).map(|_| MockProvider::rate_limited()).collect();
    let chain = ProviderFailoverChain::new(
        candidates
            .iter()
            .enumerate()
            .map(|(index, mock)| mock.selection(&format!("pa172-hop-{index}")))
            .collect(),
    );
    assert_eq!(
        chain.selections.len(),
        MAX_FAILOVER_HOPS,
        "F-a：`ProviderFailoverChain::new` 必须把 3 个候选截断到 MAX_FAILOVER_HOPS={MAX_FAILOVER_HOPS}，实际 {} 个",
        chain.selections.len()
    );
    let manager = ProviderManager::new(primary.selection("pa172-hop-primary"))
        .with_failover(ProviderFailoverChain::new(chain.selections.clone()));
    let request = request_probe();

    // Act
    let result = manager.decide_with_tools(&request, &[]);

    // Assert
    assert!(result.is_err(), "F-a：全 429 必须返回 Err");
    assert_eq!(
        primary.request_count(),
        1,
        "F-a/v2-3：主 provider 在限流类首跳必须只发 1 次 HTTP，实际 {} 次",
        primary.request_count()
    );
    let mut providers_with_http = 1usize;
    for (index, mock) in candidates.iter().enumerate() {
        let hits = mock.request_count();
        assert!(
            hits <= 1,
            "F-a/v2-3：候选 #{index} 必须只发 ≤1 次 HTTP（hop 循环在 retry 之外 ⇒ 每候选独立预算，\
             限流类首跳即 Abort），实际 {hits} 次"
        );
        if hits == 1 {
            providers_with_http += 1;
        }
    }
    assert_eq!(
        providers_with_http,
        1 + MAX_FAILOVER_HOPS,
        "F-a：链上 3 个候选被截断为 {MAX_FAILOVER_HOPS} 个 ⇒ 必须恰好尝试 3 个 provider\
         （每个 1 次 HTTP），实际 {providers_with_http}"
    );
}

// ── §3.2：触发类封闭值域（不换手的反向钉子） ─────────────────────────────────────

/// §3.2 真值表：`bad_request` 400 ⇒ **不换手**，备 provider 必须 0 次 HTTP。
#[test]
fn failover_not_triggered_for_bad_request() {
    // Arrange
    let primary = MockProvider::bad_request();
    let secondary = MockProvider::healthy();
    let manager = ProviderManager::new(primary.selection("pa172-badreq-primary"))
        .with_failover(ProviderFailoverChain::new(vec![secondary.selection(
            "pa172-badreq-secondary",
        )]));
    let request = request_probe();

    // Act
    let result = manager.decide_with_tools(&request, &[]);

    // Assert
    let error = result.expect_err("§3.2：400 bad_request 必须失败返回（不得静默换手成 Ok）");
    assert_eq!(
        secondary.request_count(),
        0,
        "§3.2：`bad_request` 不在触发类封闭值域内 ⇒ 备 provider 必须 0 次 HTTP，实际 {} 次",
        secondary.request_count()
    );
    // F-e 已冻结「返回的是既有重试编排处理后的**结构化错误文本**（不承诺原始状态行）」，
    // 故这里只要求错误仍能辨识为上游 400 语义（`invalid_request_error` / `bad request` / `400`）。
    let lower = error.to_lowercase();
    assert!(
        error.contains("400") || lower.contains("bad request") || lower.contains("invalid_request"),
        "§3.2：不换手时必须上抛主 provider 的 400 语义错误（结构化文本），实际：{error}"
    );
}

/// §3.2 / v2 第 7 条：HTTP 500 属既有冻结 NonRetryable ⇒ **不换手**（502/503/504 等才换手）。
#[test]
fn http_500_does_not_trigger_failover() {
    // Arrange
    let primary = MockProvider::internal_error();
    let secondary = MockProvider::healthy();
    let manager = ProviderManager::new(primary.selection("pa172-500-primary"))
        .with_failover(ProviderFailoverChain::new(vec![secondary.selection(
            "pa172-500-secondary",
        )]));

    // Act
    let result = manager.decide_with_tools(&request_probe(), &[]);

    // Assert
    assert!(result.is_err(), "v2-7：HTTP 500 必须失败返回");
    assert_eq!(
        secondary.request_count(),
        0,
        "v2-7：HTTP 500 不在 server_error 码内（既有冻结 NonRetryable）⇒ 不得换手，\
         备实际 {} 次 HTTP",
        secondary.request_count()
    );
    assert!(
        !failover_eligible(&status_error_text(
            500,
            "Internal Server Error",
            INTERNAL_ERROR_BODY
        )),
        "v2-7：`failover_eligible(500)` 必须为 false"
    );
    assert!(
        failover_eligible(&status_error_text(
            503,
            "Service Unavailable",
            SERVER_ERROR_BODY
        )),
        "v2-7 反向钉子：`failover_eligible(503)` 必须为 true（UpstreamServerError 码内）"
    );
}

/// §3.2 触发类真值表（单一码表，禁止第二套关键词表）：
/// `rate_limited`/`quota`/`timeout`/`network`/`auth` 与码表层 `UpstreamServerError` ⇒ 可换手；
/// `bad_request`/`other`/`500` ⇒ 不可换手。
#[test]
fn failover_trigger_class_truth_table_is_closed() {
    // Arrange
    let eligible = [
        status_error_text(429, "Too Many Requests", RATE_LIMITED_BODY),
        status_error_text(403, "Forbidden", "{\"error\":{\"type\":\"insufficient_quota\"}}"),
        "operation timed out after 45002ms".to_string(),
        "error sending request: connection reset by peer".to_string(),
        status_error_text(401, "Unauthorized", "{\"error\":{\"message\":\"invalid api key\"}}"),
        status_error_text(503, "Service Unavailable", SERVER_ERROR_BODY),
        status_error_text(520, "Unknown", "{\"error\":{\"message\":\"cloudflare\"}}"),
    ];
    let ineligible = [
        status_error_text(400, "Bad Request", BAD_REQUEST_BODY),
        "解析 provider SSE chunk 失败: expected value".to_string(),
        status_error_text(500, "Internal Server Error", INTERNAL_ERROR_BODY),
    ];

    // Act + Assert
    for text in eligible {
        assert!(
            failover_eligible(&text),
            "§3.2：该类错误必须可换手，实际判不可换手：{text}"
        );
    }
    for text in ineligible {
        assert!(
            !failover_eligible(&text),
            "§3.2：`bad_request`/`other`/`500` 不得换手（否则 400/500 会打到所有候选），\
             实际判可换手：{text}"
        );
    }
}

/// F-h 反向钉子：主 provider **无 key** ⇒ 不换手（配置错误快速失败，保持既有文案）。
#[test]
fn failover_not_triggered_when_primary_has_no_api_key() {
    // Arrange
    let primary = MockProvider::rate_limited();
    let secondary = MockProvider::healthy();
    let mut primary_selection = primary.selection("pa172-nokey-primary");
    primary_selection.api_key = None;
    let manager = ProviderManager::new(primary_selection).with_failover(
        ProviderFailoverChain::new(vec![secondary.selection("pa172-nokey-secondary")]),
    );

    // Act
    let result = manager.decide_with_tools(&request_probe(), &[]);

    // Assert
    let error = result.expect_err("F-h：缺 key 必须快速失败");
    assert!(
        error.contains("API key") || error.contains("api key") || error.contains("API Key"),
        "F-h：必须保持既有「缺 key」错误文案，实际：{error}"
    );
    assert_eq!(
        primary.request_count(),
        0,
        "F-h：缺 key 时不得先发 HTTP，实际 {} 次",
        primary.request_count()
    );
    assert_eq!(
        secondary.request_count(),
        0,
        "F-h：缺 key 属配置错误 ⇒ 不得换手，备实际 {} 次",
        secondary.request_count()
    );
}

// ── v2 第 1 条：流式入口不参与换手（原 `failover_not_triggered_after_stream_deltas` 改名） ──

/// v2-1：**流式入口不参与换手**（契约从「已发增量才禁换手」收窄为「整类入口退出换手闭包」）。
/// 主 provider 流式中途失败 ⇒ 备 provider 必须 **0 次 HTTP**，错误原样上抛。
#[test]
fn failover_stream_entry_does_not_handoff() {
    // Arrange：主 provider 先吐一个真实 delta，再吐畸形帧 ⇒ 流式中途失败。
    let primary = MockProvider::stream_delta_then_broken();
    let secondary = MockProvider::healthy();
    let manager = ProviderManager::new(primary.selection("pa172-stream-primary"))
        .with_failover(ProviderFailoverChain::new(vec![secondary.selection(
            "pa172-stream-secondary",
        )]));
    let request = request_probe();
    let mut deltas = 0usize;

    // Act
    let result = manager.decide_with_tools_stream(&request, &[], |_chunk| {
        deltas += 1;
    });

    // Assert
    let error = result.expect_err("v2-1：流式中途失败必须原样上抛 Err（流式不换手）");
    assert!(
        deltas >= 1,
        "v2-1 前置：剧本必须先产出 ≥1 个 delta（证明确实进入了流式渲染），实际 {deltas} 个"
    );
    assert_eq!(
        secondary.request_count(),
        0,
        "v2-1/G6：流式入口**不参与**换手（无总 deadline，换手会把 wall-clock 放大到 30min 级）⇒ \
         备必须 0 次 HTTP，实际 {} 次",
        secondary.request_count()
    );
    assert!(
        !error.trim().is_empty(),
        "v2-1：错误必须原样上抛（不得吞成空串），实际空错误"
    );
}

// ── L2-AT：逐 attempt deadline / 并发不超 hop ────────────────────────────────────

/// L2-AT（对抗）：v2 第 2 条 **逐 attempt deadline**——换手总预算 `FAILOVER_BUDGET_MS` 冻结为
/// 120s，且**每次候选 attempt 前重查剩余预算**，per-attempt 超时 = `min(requestTimeoutMs, remaining)`。
///
/// 判据（禁止 sleep 概率等待）：用极小 `requestTimeoutMs`（1000ms）+ 慢 mock（应答延迟 2500ms）
/// 断言三个候选的总耗时有界（≈ 3 × 1s），而基线 45s 内联超时下同样剧本需 >135s。
#[test]
fn failover_budget_stops_handoffs() {
    // Arrange
    assert_eq!(
        FAILOVER_BUDGET_MS, 120_000,
        "F-c：换手总预算常量冻结为 120_000ms，实际 {FAILOVER_BUDGET_MS}"
    );
    assert_eq!(
        MAX_FAILOVER_HOPS, 2,
        "F-a：hop 上限常量冻结为 2，实际 {MAX_FAILOVER_HOPS}"
    );
    let per_attempt_timeout_ms = 1_000u64;
    let policy = UpstreamPolicy {
        request_timeout_ms: per_attempt_timeout_ms,
        // 关掉单跳重试，使「每候选 1 次 attempt」成为确定事实（预算锚点只测 deadline）。
        decision_max_attempts: 1,
        retry_budget_ms: 1_000,
    };
    let slow_delay = Duration::from_millis(2_500);
    let primary = MockProvider::slow(slow_delay);
    let secondary = MockProvider::slow(slow_delay);
    let manager = ProviderManager::new(primary.selection("pa172-budget-primary"))
        .with_policy(policy)
        .with_failover(ProviderFailoverChain::new(vec![secondary.selection(
            "pa172-budget-secondary",
        )]));

    // Act
    let started = Instant::now();
    let result = manager.decide_with_tools(&request_probe(), &[]);
    let elapsed = started.elapsed();

    // Assert
    assert!(result.is_err(), "v2-2：逐 attempt 超时耗尽必须返回 Err");
    assert!(
        secondary.request_count() >= 1,
        "v2-2 前置：预算未耗尽前必须真实尝试第二跳，实际 {} 次",
        secondary.request_count()
    );
    let upper_bound = Duration::from_millis(per_attempt_timeout_ms * 3 + 3_000);
    assert!(
        elapsed < upper_bound,
        "v2-2：per-attempt 超时 = min(requestTimeoutMs={per_attempt_timeout_ms}ms, remaining) ⇒ \
         两个候选的耗尽耗时必须有界（< {upper_bound:?}，基线 45s 内联超时下需 >135s），\
         实际 {elapsed:?}"
    );
    assert!(
        elapsed >= Duration::from_millis(per_attempt_timeout_ms),
        "v2-2 反向钉子：必须真的等到了 per-attempt deadline（≥ {per_attempt_timeout_ms}ms），\
         实际 {elapsed:?}——过短说明超时是假的"
    );
    assert!(
        elapsed < Duration::from_millis(FAILOVER_BUDGET_MS),
        "F-c：不得以 sleep 熬尽换手预算，实际 {elapsed:?}"
    );
}

/// L2-AT（对抗）：固定线程数 + `Barrier` + 固定迭代，断言**总 HTTP 次数上界**
/// （禁止用 sleep 概率等待；不依赖调度顺序）。
#[test]
fn failover_concurrent_calls_do_not_exceed_hops() {
    // Arrange
    const THREADS: usize = 4;
    const ITERATIONS: usize = 8;
    let primary = MockProvider::rate_limited();
    let secondary = MockProvider::rate_limited();
    let primary_selection = primary.selection("pa172-concurrent-primary");
    let secondary_selection = secondary.selection("pa172-concurrent-secondary");
    let barrier = Arc::new(Barrier::new(THREADS));

    // Act
    let handles: Vec<JoinHandle<usize>> = (0..THREADS)
        .map(|_| {
            let barrier = Arc::clone(&barrier);
            let primary_selection = primary_selection.clone();
            let secondary_selection = secondary_selection.clone();
            std::thread::spawn(move || {
                let manager = ProviderManager::new(primary_selection).with_failover(
                    ProviderFailoverChain::new(vec![secondary_selection]),
                );
                barrier.wait();
                let mut errors = 0usize;
                for _ in 0..ITERATIONS {
                    // 并发争用同一对 provider ⇒ 冷却与换手必须仍然有界。
                    if manager.decide_with_tools(&request_probe(), &[]).is_err() {
                        errors += 1;
                    }
                }
                errors
            })
        })
        .collect();
    let total_errors: usize = handles
        .into_iter()
        .map(|handle| handle.join().expect("PA-172：并发线程不得 panic"))
        .sum();

    // Assert
    let calls = THREADS * ITERATIONS;
    let total_http = primary.request_count() + secondary.request_count();
    assert_eq!(
        total_errors, calls,
        "G4：全 429 链下每次调用都必须返回 Err（不得并发下伪造成功），实际 {total_errors}/{calls}"
    );
    assert!(
        total_http <= calls * (1 + MAX_FAILOVER_HOPS),
        "F-a：{calls} 次调用的总 HTTP 次数必须 ≤ calls × (1 + MAX_FAILOVER_HOPS) = {}，实际 {total_http}",
        calls * (1 + MAX_FAILOVER_HOPS)
    );
    assert!(
        total_http >= 1,
        "F-a 反向钉子：并发场景下必须真的发出过 HTTP（否则是「什么都不做」的假通过），实际 {total_http}"
    );
}
