//! PA-172 **遗留收口红相**：PA-171 遗留 #3/#4/#5/#6/#9/#10/#13 + 词表机器锁（M1）+
//! `TurnTraceRecord.errorClass` 自锁（M3）。
//!
//! 契约真值源：`.dev-team/contract-matrix-pa172.md`（v2 收窄版）
//! - §5 R1（#3）poison 钩子 `provider::poison_registry_for_test()` 为**仅 debug 可见**的
//!   `#[doc(hidden)] pub fn`，锚点：毒化后 `enter`/`remaining`/`clear` 仍可用（fail-open）；
//! - §5 R2（#4）`reset_global_cooldowns() -> usize` 返回被清条目数；
//! - §5 R3（#5）冷却键**单射转义** + `base_url` 去尾斜杠 + **scheme/host 小写归一**（path 保原样）；
//! - §8 B4/B5（#9）`BudgetPersistFailed` / `NoActiveGoal` 逐字文案；
//! - §8 B1（#13）`UiEvent::GoalRoundsStopped` 增 `session_id`；
//! - §8 B2（#10）`TuiState.goal_rounds_stopped` 复位清空；
//! - §3.2 M1 `classify_error_class` 7 值封闭集机器锁；
//! - §6 M3 `TurnTraceRecord` 的 `errorClass` 自锁（不在 `check-ts-contract.mjs` 的 4 类覆盖面内）。
//!
//! 本文件由独立 Test Agent 冻结；Executor 只读，禁改。
//!
//! 只为**纯函数/纯状态**断言；TUI 事件循环本身不可在集成测试中驱动 ⇒ #13 的
//! 「跨会话只记日志不改状态行」退化登记为**部分闭合**（见该用例 doc）。
//!
//! 红相预期（wave-69 实现前）：`poison_registry_for_test` / `reset_global_cooldowns` /
//! `GoalRoundStopReason::BudgetPersistFailed` / `UiEvent::GoalRoundsStopped { session_id }`
//! 均不存在 ⇒ 编译红（E0425/E0432/E0599），属冻结签名锚点的合法红相。

#[path = "../src/bin/tui.rs"]
#[allow(dead_code, unused_imports, unused_variables, unused_mut)]
mod tui;

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use pony_agent_core::agent::config::{
    ProviderModelCapabilities, ResolvedProviderSelection, ThinkingParamPattern,
};
use pony_agent_core::agent::goal::GoalRoundStopReason;
use pony_agent_core::agent::provider::{
    ProviderAuthType, ProviderCooldownEntry, ProviderCooldownRegistry, ProviderManager,
    ProviderMessage, ProviderProtocol, ProviderRequest,
};
use pony_agent_core::agent::retry::{
    append_retry_after_marker, classify_error_class, rate_limit_recovery_hint_at,
};
use pony_agent_core::agent::session::TurnTraceRecord;
use serde_json::Value;

/// 全局冷却注册表是本进程单例 ⇒ 本文件内触及它的用例必须**串行**（否则 `reset` 会清掉
/// 另一个用例刚建立的窗口，制造假红/假绿）。
fn registry_guard() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

// ── 紧凑回环 mock（只为建立/不建立冷却窗口，物理计数） ──────────────────────────────

const RATE_LIMITED_BODY: &str = "{\"error\":{\"type\":\"exceeded_current_quota_error\",\"message\":\"You've reached your 5-hour usage limit, resets at 06:01 UTC\"}}";

struct RateLimitedMock {
    base_url: String,
    addr: String,
    requests: Arc<AtomicUsize>,
    shutdown: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl RateLimitedMock {
    fn start() -> Self {
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
                let _ = read_http_request(&mut stream);
                if shutdown_for_thread.load(Ordering::SeqCst) {
                    return;
                }
                requests_for_thread.fetch_add(1, Ordering::SeqCst);
                let head = format!(
                    "HTTP/1.1 429 Too Many Requests\r\nContent-Type: application/json\r\nContent-Length: {}\r\nRetry-After: 3600\r\nConnection: close\r\n\r\n{}",
                    RATE_LIMITED_BODY.as_bytes().len(),
                    RATE_LIMITED_BODY
                );
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

impl Drop for RateLimitedMock {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
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

fn selection(base_url: &str, requested_name: &str, provider_name: &str, model: &str) -> ResolvedProviderSelection {
    ResolvedProviderSelection {
        requested_name: requested_name.to_string(),
        provider_name: provider_name.to_string(),
        protocol: ProviderProtocol::OpenAiCompletions,
        base_url: base_url.to_string(),
        auth_type: ProviderAuthType::Auto,
        api_key_env_var: "PA172_LEGACY_TEST_API_KEY".to_string(),
        api_key: Some("pa172-dummy-key".to_string()),
        model: model.to_string(),
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
        model: "pa172-legacy-model".to_string(),
        input: vec![ProviderMessage::user("pa172 legacy probe")],
        images: Vec::new(),
        native_messages: Vec::new(),
        observation: Default::default(),
        temperature: 0.0,
        max_output_tokens: 64,
    }
}

fn cooldown_entry(window_ms: u64) -> ProviderCooldownEntry {
    ProviderCooldownEntry {
        until_unix_ms: now_unix_ms() + window_ms,
        class: "rate_limited",
        retry_after_s: Some(window_ms / 1_000),
    }
}

// ── M1（#2 / §3.2）：error_class 词表封闭集机器锁 ─────────────────────────────────

/// M1：`classify_error_class` 词表**冻结为 7 值**，5xx 样本一律不得产出 `server_error`。
#[test]
fn error_class_wordlist_is_locked() {
    // Arrange
    const FROZEN_CLASSES: [&str; 7] = [
        "timeout",
        "rate_limited",
        "quota",
        "auth",
        "network",
        "bad_request",
        "other",
    ];
    let samples = [
        "provider 返回错误状态：500 Internal Server Error；耗时=3ms；响应正文：{\"error\":{\"message\":\"boom\"}}".to_string(),
        "provider 返回错误状态：502 Bad Gateway；耗时=3ms；响应正文：{\"error\":{\"message\":\"bad gateway\"}}".to_string(),
        "provider 返回错误状态：503 Service Unavailable；耗时=3ms；响应正文：{\"error\":{\"message\":\"unavailable\"}}".to_string(),
        "provider 返回错误状态：504 Gateway Timeout；耗时=3ms；响应正文：{\"error\":{\"message\":\"gateway timeout\"}}".to_string(),
        "provider 返回错误状态：520；耗时=3ms；响应正文：{\"error\":{\"message\":\"cloudflare\"}}".to_string(),
    ];

    // Act + Assert
    for sample in samples {
        let class = classify_error_class(&sample);
        assert_ne!(
            class, "server_error",
            "M1：`classify_error_class` 不得产出 `server_error`（该值只存在于码表层），样本：{sample}"
        );
        assert!(
            FROZEN_CLASSES.contains(&class),
            "M1：`classify_error_class` 必须落在 7 值封闭集 {FROZEN_CLASSES:?} 内，实际 `{class}`；样本：{sample}"
        );
    }
}

// ── M3（#2 / §6）：TurnTraceRecord.errorClass 自锁 ───────────────────────────────

/// M3：`TurnTraceRecord` 不在 `check-ts-contract.mjs` 的 4 类覆盖面内 ⇒ 必须在 Rust 侧自锁
/// wire 键名（`serde_json::to_value` 真实输出含 `errorClass`）。
#[test]
fn turn_trace_record_serializes_error_class() {
    // Arrange
    let record = TurnTraceRecord {
        error_class: Some("rate_limited".to_string()),
        ..Default::default()
    };

    // Act
    let value = serde_json::to_value(&record).expect("M3：TurnTraceRecord 必须可序列化");
    let object = value
        .as_object()
        .expect("M3：TurnTraceRecord 必须序列化为 JSON object");

    // Assert
    assert!(
        object.contains_key("errorClass"),
        "M3：`TurnTraceRecord` 的 serde 线缆键必须含 `errorClass`，实际键集合：{:?}",
        object.keys().collect::<Vec<_>>()
    );
    assert_eq!(
        object.get("errorClass"),
        Some(&Value::String("rate_limited".to_string())),
        "M3：`errorClass` 的值必须原样上线缆"
    );
    assert!(
        !object.contains_key("error_class"),
        "M3：不得泄漏 snake_case 线缆键 `error_class`"
    );
}

// ── #3 / §5 R1：锁中毒 fail-open 锚点 ───────────────────────────────────────────

/// #3：`poison_registry_for_test()`（仅 debug 构建可见的 `#[doc(hidden)]` 钩子）之后，
/// `ProviderCooldownRegistry` 的 `enter` / `remaining` / `clear` **不得 panic** 且正常工作。
#[test]
fn poisoned_cooldown_registry_stays_usable() {
    // Arrange
    let _guard = registry_guard();
    let registry = ProviderCooldownRegistry::global();
    let key = "pa172-poison|anchor|model|http://127.0.0.1:1/v1";

    // Act 1：制造 poisoned Mutex。
    pony_agent_core::agent::provider::poison_registry_for_test();

    // Act 2：毒化后仍走完整读写路径。
    registry.enter(key, cooldown_entry(60_000));
    let remaining = registry.remaining(key);
    registry.clear(key);
    let after_clear = registry.remaining(key);

    // Assert
    assert!(
        remaining.is_some(),
        "§5 R1：毒化后 `remaining` 必须正常返回冷却窗口（fail-open），实际 None"
    );
    assert!(
        after_clear.is_none(),
        "§5 R1：毒化后 `clear` 必须真的清掉条目，实际仍为 {after_clear:?}"
    );
    assert!(
        registry.remaining("pa172-poison-key-never-entered").is_none(),
        "§5 R1：毒化后查询未登记键必须正常返回 None（不得 panic）"
    );
}

// ── #4 / §5 R2：冷却复位入口返回被清条目数 ───────────────────────────────────────

/// #4：`reset_global_cooldowns()` 清空全表并返回被清条目数（先 enter 2 条 ⇒ 返回 ≥2，
/// 之后 `remaining` 全为 `None`）。
#[test]
fn reset_global_cooldowns_reports_cleared_entries() {
    // Arrange
    let _guard = registry_guard();
    let registry = ProviderCooldownRegistry::global();
    registry.enter("pa172-reset-a", cooldown_entry(60_000));
    registry.enter("pa172-reset-b", cooldown_entry(60_000));

    // Act
    let cleared = pony_agent_core::agent::provider::reset_global_cooldowns();

    // Assert
    assert!(
        cleared >= 2,
        "§5 R2：复位必须返回被清条目数（≥2：刚 enter 的两条），实际 {cleared}"
    );
    assert!(
        registry.remaining("pa172-reset-a").is_none(),
        "§5 R2：复位后 `pa172-reset-a` 必须已清空"
    );
    assert!(
        registry.remaining("pa172-reset-b").is_none(),
        "§5 R2：复位后 `pa172-reset-b` 必须已清空"
    );
}

// ── #5 / §5 R3：冷却键归一（单射转义 + 去尾斜杠 + host 小写） ──────────────────────

/// #5-a：含 `|` 的两个不同 selection **不得同键**（转义必须单射）。
///
/// 构造法：只把 `|` 在 `provider_name` / `model` 之间搬位置 —— 朴素拼接会得到同一字符串
/// `req|a|b|c`，单射转义后必须可区分。行为判据 = A 的冷却不得波及 B。
#[test]
fn cooldown_key_escapes_pipe_so_distinct_selections_do_not_collide() {
    // Arrange
    let _guard = registry_guard();
    let mock = RateLimitedMock::start();
    let base_url = mock.base_url.clone();
    let a = ProviderManager::new(selection(&base_url, "req", "a|b", "c"));
    let b = ProviderManager::new(selection(&base_url, "req", "a", "b|c"));

    // Act：让 A 真实 429 一次 ⇒ 进入冷却。
    let warm = a.decide_with_tools(&request_probe(), &[]);
    let a_cooling = a.cooldown_remaining();
    let b_cooling = b.cooldown_remaining();

    // Assert
    assert!(warm.is_err(), "#5 前置：A 的 429 必须返回 Err");
    assert!(
        mock.request_count() >= 1,
        "#5 前置：A 必须真实发出 ≥1 次 HTTP，实际 {}",
        mock.request_count()
    );
    assert!(
        a_cooling.is_some(),
        "§5 R3：A 必须已进入冷却窗口，实际 None"
    );
    assert!(
        b_cooling.is_none(),
        "§5 R3：含 `|` 的不同 selection 不得同键（B 不得命中 A 的冷却），实际 {b_cooling:?}"
    );
}

/// #5-b：`http://host/v1` 与 `http://host/v1/` 必须同键（`base_url` 去尾斜杠）。
#[test]
fn cooldown_key_normalizes_trailing_slash() {
    // Arrange
    let _guard = registry_guard();
    let mock = RateLimitedMock::start();
    let without_slash = mock.base_url.clone();
    let with_slash = format!("{without_slash}/");
    let plain = ProviderManager::new(selection(&without_slash, "req-slash", "p-slash", "m-slash"));
    let slashed =
        ProviderManager::new(selection(&with_slash, "req-slash", "p-slash", "m-slash"));

    // Act：在无尾斜杠的 manager 上建立冷却，再用带尾斜杠的 manager 读。
    let warm = plain.decide_with_tools(&request_probe(), &[]);
    let plain_cooling = plain.cooldown_remaining();
    let slashed_cooling = slashed.cooldown_remaining();

    // Assert
    assert!(warm.is_err(), "#5 前置：429 必须返回 Err");
    assert!(
        plain_cooling.is_some(),
        "§5 R3：无尾斜杠的 manager 必须已进入冷却，实际 None"
    );
    assert!(
        slashed_cooling.is_some(),
        "§5 R3：`/v1` 与 `/v1/` 必须视为同一 key（去尾斜杠归一），实际 None"
    );
}

/// #5-c（v2 新增）：scheme/host **大小写归一** —— `http://LOCALHOST:port/v1` 与
/// `http://localhost:port/v1` 必须同键（否则冷却可被纯大小写改动绕过）；path 保持原样。
#[test]
fn cooldown_key_lowercases_scheme_and_host() {
    // Arrange：`localhost` 与 `LOCALHOST` 都解析到同一回环 mock（host 大小写不敏感）。
    let _guard = registry_guard();
    let mock = RateLimitedMock::start();
    let port = mock
        .base_url
        .trim_start_matches("http://127.0.0.1:")
        .trim_end_matches("/v1")
        .to_string();
    let lower = format!("http://localhost:{port}/v1");
    let upper = format!("http://LOCALHOST:{port}/v1");
    let lower_manager = ProviderManager::new(selection(&lower, "req-case", "p-case", "m-case"));
    let upper_manager = ProviderManager::new(selection(&upper, "req-case", "p-case", "m-case"));

    // Act：小写 host 建立冷却，大写 host 读。
    let warm = lower_manager.decide_with_tools(&request_probe(), &[]);
    let lower_cooling = lower_manager.cooldown_remaining();
    let upper_cooling = upper_manager.cooldown_remaining();

    // Assert
    assert!(warm.is_err(), "#5 前置：429 必须返回 Err");
    assert!(
        lower_cooling.is_some(),
        "§5 R3：小写 host 的 manager 必须已进入冷却，实际 None"
    );
    assert!(
        upper_cooling.is_some(),
        "§5 R3：host 大小写必须归一（`LOCALHOST` 与 `localhost` 同键），实际 None"
    );
}

// ── #6 / §5：时间敏感的恢复窗口文案 ─────────────────────────────────────────────

/// 含**绝对恢复时刻**的限流错误文本（无 `retry_after=` 结构化后缀）。
const ABSOLUTE_RESET_ERROR: &str = "provider 返回错误状态：429 Too Many Requests；耗时=18ms；响应正文：{\"error\":{\"type\":\"exceeded_current_quota_error\",\"message\":\"You've reached your 5-hour usage limit, resets at 06:01 UTC\"}}";

/// 2026-11-09T05:00:00Z（距 06:01 UTC 还有 61 分钟）。
const NOW_A_MS: u64 = 1_794_200_400_000;
/// 2026-11-09T05:30:00Z（距 06:01 UTC 还有 31 分钟）。
const NOW_B_MS: u64 = 1_794_202_200_000;

/// #6：`user_facing_error_summary_at(err, now)` 必须**真正消费 now** ——
/// 同一错误、不同 now ⇒ 剩余窗口文案不同，且两者都能解析出恢复信息。
#[test]
fn error_summary_at_varies_with_injected_now() {
    // Arrange + Act
    let summary_a = tui::user_facing_error_summary_at(ABSOLUTE_RESET_ERROR, NOW_A_MS);
    let summary_b = tui::user_facing_error_summary_at(ABSOLUTE_RESET_ERROR, NOW_B_MS);
    let hint_a = rate_limit_recovery_hint_at(ABSOLUTE_RESET_ERROR, NOW_A_MS);
    let hint_b = rate_limit_recovery_hint_at(ABSOLUTE_RESET_ERROR, NOW_B_MS);

    // Assert
    assert_ne!(
        summary_a, summary_b,
        "#6：`now_unix_ms` 曾是死参 —— 不同 now 必须产出**不同**剩余窗口文案，\
         实际两处相同：{summary_a}"
    );
    assert!(
        summary_a.contains("恢复") || summary_a.contains("06:01"),
        "#6：now_a 摘要必须解析出恢复信息，实际：{summary_a}"
    );
    assert!(
        summary_b.contains("恢复") || summary_b.contains("06:01"),
        "#6：now_b 摘要必须解析出恢复信息，实际：{summary_b}"
    );
    assert!(
        hint_a.is_some() && hint_b.is_some(),
        "#6：`rate_limit_recovery_hint_at` 对两条 now 都必须返回 Some，实际 {hint_a:?} / {hint_b:?}"
    );
    assert_ne!(
        hint_a, hint_b,
        "#6：恢复提示必须随 now 变化（剩余窗口），实际 {hint_a:?} / {hint_b:?}"
    );
}

/// #6 反向钉子：`user_facing_error_summary(err)` 仍等价于 `_at(err, now_unix_ms())`。
///
/// 用带 `retry_after=` 结构化后缀的文本（时间无关）⇒ 两次调用结果必须逐字相等，
/// 不引入分钟边界抖动。
#[test]
fn error_summary_without_now_matches_at_with_current_clock() {
    // Arrange
    let error = append_retry_after_marker(
        "provider 返回错误状态：429 Too Many Requests；耗时=18ms",
        3_600,
    );

    // Act
    let plain = tui::user_facing_error_summary(&error);
    let at_now = tui::user_facing_error_summary_at(&error, now_unix_ms());

    // Assert
    assert_eq!(
        plain, at_now,
        "#6：`user_facing_error_summary(err)` 必须等价于 `_at(err, now_unix_ms())`"
    );
}

// ── #9 / §8 B4+B5：新停止变体的逐字文案 ─────────────────────────────────────────

/// #9 / B4：`BudgetPersistFailed.visible_line()` 逐字冻结，且**不等于** `BudgetExhausted`。
#[test]
fn budget_persist_failed_visible_line_is_frozen_and_distinct() {
    // Arrange
    let persist_failed = GoalRoundStopReason::BudgetPersistFailed;
    let exhausted = GoalRoundStopReason::BudgetExhausted {
        rounds_started: 20,
        max_rounds: 20,
    };

    // Act
    let persist_line = persist_failed.visible_line();
    let exhausted_line = exhausted.visible_line();

    // Assert
    assert_eq!(
        persist_line, "goal 续轮已停止（本轮预算推进落盘失败，已暂停自动续轮）",
        "§8 B4：`BudgetPersistFailed` 文案逐字冻结"
    );
    assert_ne!(
        persist_line, exhausted_line,
        "§8 B4/遗留 #9：落盘失败**不得**复用 `BudgetExhausted` 文案，实际两者相同：{persist_line}"
    );
}

/// #23 / B5：`NoActiveGoal.visible_line()` 逐字冻结，且不等于 `BudgetExhausted` 与
/// `BudgetPersistFailed` 的文案。
#[test]
fn no_active_goal_visible_line_is_frozen_and_distinct() {
    // Arrange
    let no_active_goal = GoalRoundStopReason::NoActiveGoal;
    let exhausted = GoalRoundStopReason::BudgetExhausted {
        rounds_started: 1,
        max_rounds: 20,
    };

    // Act
    let line = no_active_goal.visible_line();

    // Assert
    assert_eq!(
        line, "goal 续轮已停止（当前会话无进行中的目标）",
        "§8 B5：`NoActiveGoal` 文案逐字冻结"
    );
    assert_ne!(
        line,
        exhausted.visible_line(),
        "§8 B5：`NoActiveGoal` 不得复用 `BudgetExhausted` 文案"
    );
    assert_ne!(
        line,
        GoalRoundStopReason::BudgetPersistFailed.visible_line(),
        "§8 B5：`NoActiveGoal` 不得复用 `BudgetPersistFailed` 文案"
    );
}

// ── #13 / §8 B1：GoalRoundsStopped 携带 session_id ─────────────────────────────

/// #13 / B1：`UiEvent::GoalRoundsStopped` 必须携带 `session_id: Option<String>`。
///
/// **部分闭合（如实登记）**：事件循环的「`session_id` ≠ 当前会话 ⇒ 只落日志、不改状态行、
/// 不进消息区」需要驱动 TUI 事件循环（集成测试不可达）。本用例只锁两条可机判面：
/// (a) 变体字段契约存在；(b) 事件可携带跨会话 `session_id` 构造。
#[test]
fn goal_rounds_stopped_event_carries_session_id() {
    // Arrange + Act
    let event = tui::UiEvent::GoalRoundsStopped {
        reason: GoalRoundStopReason::NoActiveGoal,
        status_line: GoalRoundStopReason::NoActiveGoal.visible_line(),
        session_id: Some("session-other".to_string()),
    };

    // Assert
    match event {
        tui::UiEvent::GoalRoundsStopped {
            reason,
            status_line,
            session_id,
        } => {
            assert_eq!(
                reason,
                GoalRoundStopReason::NoActiveGoal,
                "B1：`reason` 字段必须原样保留"
            );
            assert!(
                !status_line.trim().is_empty(),
                "B1：`status_line` 必须携带可见文案（不得静默空串）"
            );
            assert_eq!(
                session_id.as_deref(),
                Some("session-other"),
                "B1：`GoalRoundsStopped` 必须携带 `session_id`（跨会话上报的判别依据）"
            );
        }
        _ => panic!("B1：构造出的必须是 `GoalRoundsStopped`（变体字段契约被破坏）"),
    }
}

// ── #10 / §8 B2：goal_rounds_stopped 复位 ──────────────────────────────────────

/// #10 / B2：`TuiState.goal_rounds_stopped` 必须在 `reset_session_scope` 清 `None`。
#[test]
fn reset_session_scope_clears_goal_rounds_stopped() {
    // Arrange
    let mut state = tui::TuiState::default();
    state.goal_rounds_stopped = Some(GoalRoundStopReason::BudgetExhausted {
        rounds_started: 20,
        max_rounds: 20,
    });
    assert!(
        state.goal_rounds_stopped.is_some(),
        "#10 前置：状态必须已写入停止原因"
    );

    // Act
    state.reset_session_scope();

    // Assert
    assert!(
        state.goal_rounds_stopped.is_none(),
        "§8 B2/遗留 #10：`reset_session_scope` 必须清空 `goal_rounds_stopped`，实际 {:?}",
        state.goal_rounds_stopped
    );
}
