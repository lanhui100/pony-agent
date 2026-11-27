use crate::config::{
    ProviderReasoningEffort, ResolvedProviderSelection, ThinkingParamPattern, UpstreamPolicy,
};
use pony_protocol::input::TurnInputImage;
use crate::retry::{
    compute_delay, is_rate_limit_error, BackoffConfig, JitterKind, ProviderRetryPolicy,
    RetryBudget, RetryDecision, SingleHopRetryReport, Sleeper, StdThreadSleeper, StreamState,
};
use pony_protocol::runtime_helper::block_on;
use pony_protocol::tool_contract::{builtin_tool_surface, ToolCall, ToolDefinition, ToolResult};
use crate::tool_pruner::{split_head_tail, PruneBudget};
use futures_util::StreamExt;
use reqwest::Client;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::error::Error;
#[cfg(test)]
use std::io::BufRead;
use std::time::Duration;
use std::time::Instant;

mod openai_sse;
use self::openai_sse::collect_openai_sse_message_from_response;
#[cfg(test)]
use self::openai_sse::{collect_openai_sse_message, collect_openai_sse_message_from_reader};
mod responses_api;
use self::responses_api::{
    build_responses_request_body, collect_responses_sse_message_from_response,
    parse_responses_output,
};
pub mod model_catalog;
pub use self::model_catalog::fetch_model_ids;

mod fallback_body;
pub use self::fallback_body::{fallback_body_text, local_tool_followup_fallback_response};
mod openai_wire;
mod usage;
pub use self::openai_wire::openai_safe_tool_name;
// `extract_chat_delta_text` 在拆分前即无消费者（原文件已挂 `#[allow(dead_code)]`），
// 随函数一起留在 `openai_wire`，不在此 re-export（避免引入 unused import 噪音）。
pub use self::usage::extract_openai_usage;
pub use self::usage::normalize_token_usage;

pub use pony_protocol::provider_types::{
    tool_calls_from_single, usage_origin_from_label, BuildContextObservation,
    ContextRefreshReason, ConversationCarryMode, PrefixMutationReason, ProviderAuthType,
    ProviderDecision, ProviderMessage, ProviderProtocol, ProviderRequest,
    ProviderRequestObservation, ProviderResponse, ProviderRole, ProviderStreamChunk,
    AnthropicStreamMessage, TokenUsage, USAGE_ORIGIN_ESTIMATED_CHARS, USAGE_ORIGIN_PROVIDER,
};

pub fn build_context_observation(
    request: &ProviderRequest,
    tools: &[ToolDefinition],
) -> BuildContextObservation {
    let observation = if request.observation.is_empty() {
        derive_request_observation(request)
    } else {
        request.observation.clone()
    };
    let (request_format, request_messages_text, message_count) =
        if !request.native_messages.is_empty() {
            (
                "provider-native".to_string(),
                render_native_messages(&request.native_messages),
                request.native_messages.len(),
            )
        } else {
            (
                "normalized-input".to_string(),
                render_input_messages(&request.input),
                request.input.len(),
            )
        };

    BuildContextObservation {
        request_format,
        message_count,
        image_count: request.images.len(),
        tool_count: tools.len(),
        temperature: request.temperature,
        max_output_tokens: request.max_output_tokens,
        stable_prefix_text: observation.stable_prefix_text,
        semi_stable_context_text: observation.semi_stable_context_text,
        volatile_input_text: observation.volatile_input_text,
        prefix_mutation_reasons: observation.prefix_mutation_reasons,
        context_refresh_reason: observation.context_refresh_reason,
        instruction_scope_sources: observation.instruction_scope_sources,
        conversation_carry_mode: observation.conversation_carry_mode,
        request_messages_text,
        tool_definitions_text: render_tool_definitions(tools),
    }
}

#[allow(dead_code)]
pub trait ProviderClient {
    fn requested_name(&self) -> &str;
    fn name(&self) -> &str;
    fn model(&self) -> &str;
    fn protocol_label(&self) -> &'static str;
    /// 当前生效协议枚举（行为分叉位点统一传枚举）。
    fn protocol(&self) -> &ProviderProtocol;
    fn temperature(&self) -> f32;
    fn max_output_tokens(&self) -> u32;
    fn decide_with_tools(
        &self,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
    ) -> Result<ProviderDecision, String>;
    fn decide_with_tools_stream<F>(
        &self,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
        on_delta: F,
    ) -> Result<ProviderDecision, String>
    where
        F: FnMut(ProviderStreamChunk);
    fn continue_with_tool_result(
        &self,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
        accumulated_messages: &mut Vec<Value>,
        assistant_message: Option<&Value>,
        tool_call: &ToolCall,
        tool_result: &ToolResult,
    ) -> Result<ProviderResponse, String>;
    fn continue_with_tool_result_stream<F>(
        &self,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
        accumulated_messages: &mut Vec<Value>,
        assistant_message: Option<&Value>,
        tool_call: &ToolCall,
        tool_result: &ToolResult,
        on_delta: F,
    ) -> Result<ProviderResponse, String>
    where
        F: FnMut(ProviderStreamChunk);

    /// T9：带取消/时长守卫的流式决策。默认实现**忽略守卫**（测试替身无需实现），
    /// 生产实现（`ProviderManager`）必须真正把守卫接进帧循环。
    fn decide_with_tools_stream_guarded<F>(
        &self,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
        guard: &ProviderStreamGuard<'_>,
        on_delta: F,
    ) -> Result<ProviderDecision, String>
    where
        F: FnMut(ProviderStreamChunk),
    {
        let _ = guard;
        self.decide_with_tools_stream(request, tools, on_delta)
    }

    /// T9：带守卫的流式工具跟进（同上：默认忽略守卫）。
    fn continue_with_tool_result_stream_guarded<F>(
        &self,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
        accumulated_messages: &mut Vec<Value>,
        assistant_message: Option<&Value>,
        tool_call: &ToolCall,
        tool_result: &ToolResult,
        guard: &ProviderStreamGuard<'_>,
        on_delta: F,
    ) -> Result<ProviderResponse, String>
    where
        F: FnMut(ProviderStreamChunk),
    {
        let _ = guard;
        self.continue_with_tool_result_stream(
            request,
            tools,
            accumulated_messages,
            assistant_message,
            tool_call,
            tool_result,
            on_delta,
        )
    }

    /// 模型能力里的上下文窗口（tokens）。wave-54 C4：`llm.chat` 据此输出
    /// `gen_ai.usage.context_window` / `context_ratio`；无窗口能力 ⇒ `None`
    /// （禁止 0 占位）。默认实现返回 `None`，由具备能力信息的实现覆盖。
    fn context_window_tokens(&self) -> Option<u32> {
        None
    }
}

/// T9（W56-A）：provider 流式读循环的**取消观测 + 总时长上界**守卫。
///
/// 背景：帧循环里此前既没有取消检查（`is_stop_requested` 在生产路径恒为 `|| false`），
/// 也没有总时长上界 —— 只要上游持续吐字节，连接与 turn 就会被永久占用，`stop_turn`
/// 的取消标志不被观测。
///
/// 设计约束：
/// - 取消源**复用**既有 `ExecutionControlRegistry`（由 runtime 传入闭包），本结构不持有
///   第二套取消状态；
/// - 按 `cancel_tick` **节流**检查（默认 250ms），不是每个 chunk 都查，避免读循环热路径退化；
/// - `max_elapsed` 是**总时长**上界（默认 30min 宽松有限），只在超限时以明确错误码退出，
///   不静默截断正常的长推理流；
/// - 可注入：测试可构造极短上界 / 可控取消闭包，无需 sleep 30 分钟。
#[derive(Clone, Copy)]
pub struct ProviderStreamGuard<'a> {
    max_elapsed: Duration,
    cancel_tick: Duration,
    cancel_check: Option<&'a dyn Fn() -> bool>,
}

impl std::fmt::Debug for ProviderStreamGuard<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderStreamGuard")
            .field("max_elapsed", &self.max_elapsed)
            .field("cancel_tick", &self.cancel_tick)
            .field("has_cancel_check", &self.cancel_check.is_some())
            .finish()
    }
}

impl<'a> ProviderStreamGuard<'a> {
    /// 缺省总时长上界：30min（宽松但有限）。
    pub const DEFAULT_MAX_ELAPSED: Duration = Duration::from_secs(30 * 60);
    /// 缺省取消检查节流间隔。
    pub const DEFAULT_CANCEL_TICK: Duration = Duration::from_millis(250);
    /// 超时错误码（调用方可按前缀分类；不是静默截断）。
    pub const DEADLINE_ERROR_CODE: &'static str = "provider_stream_deadline_exceeded";
    /// 取消错误码。
    pub const CANCELLED_ERROR_CODE: &'static str = "provider_stream_cancelled";

    /// 无取消观测、仅保留缺省时长上界的守卫（等价于"未接取消源"，用于测试/同步路径）。
    pub fn without_cancel() -> ProviderStreamGuard<'static> {
        ProviderStreamGuard {
            max_elapsed: Self::DEFAULT_MAX_ELAPSED,
            cancel_tick: Self::DEFAULT_CANCEL_TICK,
            cancel_check: None,
        }
    }

    /// 接既有取消源（runtime 传 `&ExecutionControlRegistry` 派生的闭包；不新造取消状态）。
    pub fn with_cancel_check(cancel_check: &'a dyn Fn() -> bool) -> Self {
        Self {
            max_elapsed: Self::DEFAULT_MAX_ELAPSED,
            cancel_tick: Self::DEFAULT_CANCEL_TICK,
            cancel_check: Some(cancel_check),
        }
    }

    /// 全参构造（测试注入短上界 / 零节流 / 可控取消）。
    pub fn new(
        max_elapsed: Duration,
        cancel_tick: Duration,
        cancel_check: Option<&'a dyn Fn() -> bool>,
    ) -> Self {
        Self {
            max_elapsed,
            cancel_tick,
            cancel_check,
        }
    }

    pub fn max_elapsed(&self) -> Duration {
        self.max_elapsed
    }

    pub fn cancel_tick(&self) -> Duration {
        self.cancel_tick
    }

    /// 当前是否已请求取消（无取消源时恒 false，绝不静默当成"已取消"）。
    pub fn cancel_requested(&self) -> bool {
        self.cancel_check.map(|check| check()).unwrap_or(false)
    }
}

// ── PA-171 C3：provider 限流冷却（进程级注册表，跨轮/跨 ProviderManager 存活） ──────────

/// 无 `Retry-After` 指示时的默认冷却窗口（C3-P5）。
const COOLDOWN_DEFAULT_MS: u64 = 60_000;
/// 冷却窗口硬上界（C3-P5）：覆盖 Kimi 5 小时窗口，同时防止异常头把 provider 永久冻结。
const COOLDOWN_MAX_MS: u64 = 6 * 60 * 60 * 1_000;

// ── B1795506800002：请求体字节闸门（传输物理防线，与 token 预算正交） ──────────────────

/// 纯文本请求体硬上限（512 KiB）。
///
/// 存在理由：上下文装配只有 **token** 预算（`compression::estimate_text_tokens` = chars/4），
/// 而边缘网关（EdgeOne / Cloudflare / Nginx）按**字节**直接 RST/413。两者口径不重叠：
/// UTF-8 中文 3 字节/字、JSON 转义（`\"`/`\n`/`\uXXXX`）、大型 `tools` schema、base64 图像
/// 与单轮多 hop 的 `accumulated_messages` 膨胀都不进 token 估算。缺陷现场（B1795488338107）
/// 就是 300KB+ 请求在发出后被边缘 10~13ms RST，重试还会放大成防刷封禁。
const MAX_REQUEST_BODY_BYTES: u64 = 512 * 1024;

/// 含图请求体硬上限（10 MiB）：多模态 base64 天然是 MiB 量级。
const MAX_MULTIMODAL_REQUEST_BODY_BYTES: u64 = 10 * 1024 * 1024;

/// 纯文本请求体软告警阈值（256 KiB）：只落结构化 warn，不拦截。
const REQUEST_BODY_WARN_BYTES: u64 = 256 * 1024;

/// 请求体是否含多模态图像块（决定用哪一档字节上限）。
///
/// 判据只用结构特征（`type` 取值 / `data:image/` 前缀），不做内容嗅探：本函数在 HTTP
/// 出口热路径上，必须 O(n) 且无分配放大。
fn request_body_has_images(body: &Value) -> bool {
    fn has_image_block(value: &Value) -> bool {
        match value {
            Value::Array(items) => items.iter().any(has_image_block),
            Value::Object(map) => {
                if let Some(kind) = map.get("type").and_then(Value::as_str) {
                    if matches!(kind, "image" | "image_url" | "input_image") {
                        return true;
                    }
                }
                if let Some(url) = map.get("url").and_then(Value::as_str) {
                    if url.starts_with("data:image/") {
                        return true;
                    }
                }
                map.values().any(has_image_block)
            }
            Value::String(text) => text.starts_with("data:image/"),
            _ => false,
        }
    }
    has_image_block(body)
}

/// B1795506800002：序列化请求体并做**字节闸门**（0 HTTP Fail-Closed）。
///
/// 返回 `Ok(bytes)` = 放行（可直接 `.body(bytes)` 发出，与 `.json(body)` 逐字节等价）；
/// 返回 `Err(message)` = 超硬上限，**一个网络包都不发**。
///
/// 超限动作的取舍（评审结论，已落 ADR）：
/// - 错误文本含 `payload too large` ⇒ 经既有 `UPSTREAM_CONTEXT_OVERFLOW_HINTS` 自动归为
///   `ErrorCode::UpstreamContextOverflow` → `FailureKind::RequiresRequestMutation`
///   （`is_retryable() == false`）⇒ **0 次重试**，不会把同一超大包重放成边缘防刷风暴；
/// - **不在 HTTP 出口静默裁剪**：HTTP 层已脱离会话语义，硬剪会破坏 prompt cache 前缀、
///   破坏 `tool_call`/`tool_result` 闭合（Anthropic 400）或误删 `reasoning_content`
///   （DeepSeek 400）。要自动救回应由上层捕获 `UpstreamContextOverflow` 后跑一轮受控压缩。
fn serialize_request_body_with_size_gate(body: &Value) -> Result<Vec<u8>, String> {
    let bytes = serde_json::to_vec(body).map_err(|error| {
        format!(
            "serialize provider request body failed; payload too large? error={error}"
        )
    })?;
    // `u64::try_from(usize)` 在 64 位平台恒成功；不用 `as` 以守住 clippy 的
    // `as_conversions` 棘轮（该表禁止新增裸转换）。
    let body_bytes = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    let has_images = request_body_has_images(body);
    let limit = if has_images {
        MAX_MULTIMODAL_REQUEST_BODY_BYTES
    } else {
        MAX_REQUEST_BODY_BYTES
    };
    if body_bytes > limit {
        pony_observability::otel::log::log_json_with_fields(
            "warn",
            "provider_request_body_too_large",
            None,
            &[
                ("request_body_bytes", serde_json::json!(body_bytes)),
                ("limit_bytes", serde_json::json!(limit)),
                ("has_images", serde_json::json!(has_images)),
            ],
        );
        return Err(format!(
            "provider request body too large; payload too large: request_body_bytes={body_bytes} \
             limit_bytes={limit} has_images={has_images}; 本次未发起请求"
        ));
    }
    if body_bytes > REQUEST_BODY_WARN_BYTES && !has_images {
        pony_observability::otel::log::log_json_with_fields(
            "warn",
            "provider_request_body_large",
            None,
            &[
                ("request_body_bytes", serde_json::json!(body_bytes)),
                ("warn_bytes", serde_json::json!(REQUEST_BODY_WARN_BYTES)),
            ],
        );
    }
    Ok(bytes)
}
/// 注册表条目容量硬上限（C3-P4）：过期即淘汰，超限按最早到期淘汰。
const COOLDOWN_REGISTRY_CAPACITY: usize = 64;

/// 单条 provider 冷却记录（纯内存态，无 wire 语义）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCooldownEntry {
    pub until_unix_ms: u64,
    /// `retry::classify_error_class` 的返回值（`rate_limited` / `quota`）。
    pub class: &'static str,
    pub retry_after_s: Option<u64>,
}

/// 进程级冷却注册表（`ProviderManager` 每轮新建且只含 config ⇒ 冷却必须脱离实例存活）。
/// 锁只用于 get/insert/evict，**绝不**持锁跨 HTTP/`block_on`/sleep（C3-P4）；
/// poison 恢复沿用既有写法 `lock().unwrap_or_else(|e| e.into_inner())`。
pub struct ProviderCooldownRegistry {
    entries: std::sync::Mutex<std::collections::HashMap<String, ProviderCooldownEntry>>,
}

impl ProviderCooldownRegistry {
    pub fn global() -> &'static Self {
        static REGISTRY: std::sync::OnceLock<ProviderCooldownRegistry> = std::sync::OnceLock::new();
        REGISTRY.get_or_init(|| Self {
            entries: std::sync::Mutex::new(std::collections::HashMap::new()),
        })
    }

    /// 剩余冷却时长；已到期即淘汰并返回 `None`。
    pub fn remaining(&self, key: &str) -> Option<Duration> {
        let mut guard = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let entry = guard.get(key)?;
        let now = now_unix_ms();
        if entry.until_unix_ms <= now {
            guard.remove(key);
            return None;
        }
        Some(Duration::from_millis(entry.until_unix_ms - now))
    }

    /// 冷却条目的类别（供冷却失败文案取 `class`）。
    pub fn class_for(&self, key: &str) -> Option<&'static str> {
        let guard = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.get(key).map(|entry| entry.class)
    }

    /// 登记/覆盖冷却；写入前淘汰过期条目，容量超限按最早到期淘汰。
    pub fn enter(&self, key: &str, entry: ProviderCooldownEntry) {
        let mut guard = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let now = now_unix_ms();
        guard.retain(|_, entry| entry.until_unix_ms > now);
        if guard.len() >= COOLDOWN_REGISTRY_CAPACITY && !guard.contains_key(key) {
            if let Some(oldest) = guard
                .iter()
                .min_by_key(|(_, entry)| entry.until_unix_ms)
                .map(|(key, _)| key.clone())
            {
                guard.remove(&oldest);
            }
        }
        guard.insert(key.to_string(), entry);
    }

    /// 显式解除冷却（恢复开关/测试复位）。
    pub fn clear(&self, key: &str) {
        let mut guard = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.remove(key);
    }

    /// PA-172 §5 R2：清空全表并返回被清条目数（进程级冷却复位入口的注册表侧实现）。
    fn clear_all(&self) -> usize {
        let mut guard = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let cleared = guard.len();
        guard.clear();
        cleared
    }
}

/// PA-172 §5 R2（遗留 #4）：冷却复位**生产入口**——清空进程级全表并返回被清条目数。
///
/// 唯一生产调用点是显式 `/clear` 命令分支（`bin/tui.rs`），**不得**挂进
/// `reset_session_scope`（该函数有 6 个调用点，会让一个会话的 /resume 清掉进程级冷却表、
/// 把别的会话正在冷却的上游重新点燃）。
pub fn reset_global_cooldowns() -> usize {
    let cleared = ProviderCooldownRegistry::global().clear_all();
    // nfr-baseline-wave68 `wave68_logging_events`：复位必须可见（不静默改全局状态）。
    pony_observability::otel::log::log_json_with_fields(
        "info",
        "provider_cooldown_reset",
        None,
        &[
            ("cleared_entries", serde_json::json!(cleared)),
            ("scope", serde_json::json!("clear")),
        ],
    );
    cleared
}

/// PA-172 §5 R1（遗留 #3）：可执行的锁中毒锚点（**仅 debug 构建可见**的测试钩子）。
///
/// 冷却注册表的 4 处 `lock().unwrap_or_else(|poisoned| poisoned.into_inner())` **保持**
/// fail-open（契约声明：临界区仅 `HashMap` get/insert/retain/remove，poison 后继续可接受；
/// fail-closed 会让一次历史 panic 永久冻结该 provider）。本钩子制造 poisoned Mutex，
/// 供锚点用例断言 `enter`/`remaining`/`clear` 仍可用。
#[cfg(debug_assertions)]
#[doc(hidden)]
pub fn poison_registry_for_test() {
    let registry = ProviderCooldownRegistry::global();
    let _ = std::thread::spawn(move || {
        let _guard = registry
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        panic!("PA-172 R1: poison provider cooldown registry for test");
    })
    .join();
}

/// PA-172 §5 R3：单段转义（**单射**，D69-12）。
///
/// 顺序冻结：`%` → `%25` **先**、`|` → `%7C` **后**（否则 `%7C` 字面量会与真 `|` 碰撞），
/// 空段 → `~empty~`（否决 `-` 作占位：与字面 `-` 不可区分）。
fn escape_cooldown_segment(segment: &str) -> String {
    if segment.is_empty() {
        return "~empty~".to_string();
    }
    segment.replace('%', "%25").replace('|', "%7C")
}

/// PA-172 §5 R3：`base_url` 归一——去尾斜杠 + scheme/host 小写化（path 保持原样）。
///
/// host 大小写不敏感（`http://API.Example.com/v1` 与 `http://api.example.com/v1` 同键，
/// 否则冷却可被纯大小写改动绕过）；path 区分大小写（`/V1` ≠ `/v1`），故只小写
/// scheme + authority，path 原样保留。
fn normalize_cooldown_base_url(base_url: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/');
    match trimmed.find("://") {
        Some(scheme_end) => {
            let scheme = trimmed[..scheme_end].to_ascii_lowercase();
            let rest = &trimmed[scheme_end + 3..];
            let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
            let authority = rest[..authority_end].to_ascii_lowercase();
            format!("{scheme}://{authority}{}", &rest[authority_end..])
        }
        None => trimmed.to_string(),
    }
}

/// PA-172 §5 R3：冷却键（四段各自单射转义）。
///
/// 段序冻结：`requested_name|provider_name|model|base_url`——第四段含 base_url 防跨
/// endpoint 误杀（D68-7）；`base_url` 先按 [`normalize_cooldown_base_url`] 归一。
pub fn cooldown_key_for(selection: &ResolvedProviderSelection) -> String {
    format!(
        "{}|{}|{}|{}",
        escape_cooldown_segment(&selection.requested_name),
        escape_cooldown_segment(&selection.provider_name),
        escape_cooldown_segment(&selection.model),
        escape_cooldown_segment(&normalize_cooldown_base_url(&selection.base_url))
    )
}

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

// ── PA-172 §3：跨 provider 有界故障转移（换手） ──────────────────────────────────

/// F-a：最多 2 次换手（候选链最多尝试 3 个 provider：主 + 2）。
pub const MAX_FAILOVER_HOPS: usize = 2;

/// F-c：换手预算（毫秒），**逐 attempt 的 deadline**（不是"换手前查一次"的闸门）。
///
/// 冻结同步路径最坏 wall-clock 上界：`requestTimeoutMs × decisionMaxAttempts +
/// backoff_upper(7.5s) + FAILOVER_BUDGET_MS`，默认值 = `45s×5 + 7.5s + 120s = 352.5s`。
pub const FAILOVER_BUDGET_MS: u64 = 120_000;

/// PA-174 B2：预算耗尽时**唯一允许**的合成错误文案（仅当一次 HTTP 都没发出时使用）。
///
/// 该文案不得出现在「有真实失败」的终态错误里（真实错误优先），因此它同时是
/// `record_provider_error` 的排除标记。
const FAILOVER_BUDGET_EXHAUSTED_MSG: &str =
    "failover budget exhausted before attempt; remaining=0ms; no request sent";

/// PA-172 §3.1：显式有序候选链（`ProviderManager::new` 时为空 ⇒ 换手关闭）。
///
/// `new` 截断到 [`MAX_FAILOVER_HOPS`]：即使装配侧给了更长的链（`providers.json` 上限 8），
/// 单次调用最多只尝试 2 个候选。
#[derive(Debug, Clone, Default)]
pub struct ProviderFailoverChain {
    pub selections: Vec<ResolvedProviderSelection>,
}

impl ProviderFailoverChain {
    pub fn new(selections: Vec<ResolvedProviderSelection>) -> Self {
        let mut selections = selections;
        selections.truncate(MAX_FAILOVER_HOPS);
        Self { selections }
    }

    pub fn is_empty(&self) -> bool {
        self.selections.is_empty()
    }
}

/// PA-174 F2：`Clone` 的唯一用途是给**同步入口**派生一份带「跨 attempt 总 deadline」的
/// 执行体（见 [`ProviderManager::with_aggregate_sync_deadline`]）——入口方法是 `&self`，
/// 而 deadline 是 `attempt_deadline` 字段，故按值派生一份执行体，不改调用面。
#[derive(Clone)]
pub struct ProviderManager {
    config: ResolvedProviderSelection,
    /// PA-172 §2.1：上游调用策略；`new()` 时恒为 `UpstreamPolicy::default()`（G2）。
    policy: UpstreamPolicy,
    /// PA-172 §3.1：换手链；`new()` 时为 `None`（语义冻结：既有构造点零改动）。
    failover: Option<ProviderFailoverChain>,
    /// PA-172 §3.3 F-c（L3-A/L3-P B2）：**绝对截止时刻**。换手候选带的是
    /// `FAILOVER_BUDGET_MS` 预算（PA-174 F2 起，同步入口另派生一份
    /// `aggregate_sync_call_budget_ms` 预算）⇒ 每次 attempt 的 HTTP 超时都在发请求前按
    /// `min(policy, remaining)` 重算（逐 attempt deadline），而不是每次 hop 只算一次。
    attempt_deadline: Option<Instant>,
    /// PA-174 F2：同步入口的**退避上限**（同一聚合 deadline）。`None` ⇒ 退避不截断，
    /// 与既有调度逐字一致。换手候选只带 `attempt_deadline`、不带本字段 ⇒
    /// `FAILOVER_BUDGET_MS` 的既有行为零改动。
    sleep_deadline: Option<Instant>,
    /// PA-174 B2：最近一次**真实** provider 失败（PA-173 的 `last_provider_error` 同构）。
    /// 预算耗尽时用它收口，绝不让合成错误顶替真实失败。`Arc<Mutex>` 是为了让
    /// `#[derive(Clone)]` 派生的执行体与本体共享同一份记录（`post_*` 走 `&self`）。
    last_provider_error: std::sync::Arc<std::sync::Mutex<Option<String>>>,
}

impl ProviderManager {
    pub fn new(config: ResolvedProviderSelection) -> Self {
        Self {
            config,
            policy: UpstreamPolicy::default(),
            failover: None,
            attempt_deadline: None,
            sleep_deadline: None,
            last_provider_error: std::sync::Arc::new(std::sync::Mutex::new(None)),
        }
    }

    /// PA-172 §3.3 F-c：给换手候选注入绝对截止时刻（主 provider 不带 ⇒ 主跳预算不变）。
    fn with_attempt_deadline(mut self, deadline: Instant) -> Self {
        self.attempt_deadline = Some(deadline);
        self
    }

    /// PA-174 F2：给**同步调用入口**派生一份带「跨 attempt 总 deadline」的执行体。
    ///
    /// 预算 = [`crate::config::aggregate_sync_call_budget_ms`]（
    /// `min(timeout × attempts + retry_budget, 180s)`）：此前每次 attempt 各自吃满
    /// `request_timeout_ms`，重试次数由策略决定 ⇒ 同步调用总墙钟无跨 attempt 上界。
    ///
    /// 语义约束（不得扩大）：
    /// - 已带 deadline 的 manager（换手候选，`FAILOVER_BUDGET_MS`）**原样返回** ⇒ 换手预算与
    ///   换手语义零改动；
    /// - deadline 只收紧**每次 attempt 的 HTTP 超时**（`effective_attempt_timeout_ms`），
    ///   剩余为 0 时走既有「不可重试 Abort」错误，不新增重试/换手机制。
    fn with_aggregate_sync_deadline(&self) -> Self {
        if self.attempt_deadline.is_some() {
            return self.clone();
        }
        let budget_ms = crate::config::aggregate_sync_call_budget_ms(&self.policy);
        let deadline = Instant::now() + Duration::from_millis(budget_ms);
        let mut executor = self.clone();
        executor.attempt_deadline = Some(deadline);
        // 同一 deadline 同时收敛退避：否则最后一段退避（最长 8s）会越过 deadline，
        // 总墙钟被拖长一整段 backoff（PA-174 F2 的 Test Agent 物理收据即此）。
        executor.sleep_deadline = Some(deadline);
        executor
    }

    /// PA-172 §3.3 F-c：本次 attempt 的请求超时（毫秒）。
    ///
    /// 逐 attempt 重算：`min(policy.requestTimeoutMs, remaining_ms)`，唯一取值出口是
    /// [`crate::retry::failover_attempt_timeout_ms`]（禁止第二处 `min`）。
    /// 返回 `0` ⇒ 预算已耗尽，调用方必须**立即** Abort（0 次 HTTP）。
    fn effective_attempt_timeout_ms(&self) -> u64 {
        let Some(deadline) = self.attempt_deadline else {
            return self.policy.request_timeout_ms;
        };
        let remaining_ms = deadline
            .saturating_duration_since(Instant::now())
            .as_millis() as u64;
        // P0-2：剩余预算不足**最小可用超时** ⇒ 返回 0（调用方立即收口、0 次 HTTP），
        // 而不是发一个注定超时的秒级请求（实测 6.4s / 0ms 两种自伤终态）。
        //
        // 闸门按配置自适应：`min(全局最小可用值, request_timeout_ms)`。原因：全局 5s 是
        // 「真实上游 p50 4.67s」的物理下界，而测试替身/本地假服务会把单次超时配到 1s——
        // 用全局值兜底会把这类**合法的小超时配置**整体判死（0 次 HTTP），既弄坏换手契约，
        // 也不是本项要修的缺陷。取 `min` 后：45s 配置下末次 6.4s 仍会被拒（低于 5s 才拒），
        // 1s 配置下 1s 预算仍可正常发一次 attempt。
        let min_usable_ms = crate::config::UPSTREAM_MIN_USABLE_ATTEMPT_TIMEOUT_MS
            .min(self.policy.request_timeout_ms);
        if remaining_ms < min_usable_ms {
            return 0;
        }
        crate::retry::failover_attempt_timeout_ms(
            self.policy.request_timeout_ms,
            remaining_ms,
        )
    }

    /// F-c：预算耗尽的结构化错误（必须**不可重试** ⇒ 重试编排立即 Abort、不再白睡）。
    ///
    /// PA-174 B2：本错误**只允许**在「一次 HTTP 都没发出」（无任何真实失败可交付）时使用；
    /// 有真实失败时必须返回 [`Self::budget_exhausted_error`]（真实错误优先），否则用户会看到
    /// 一句与上游无关的预算文案，且换手判据（`failover_eligible`）永远不成立。
    fn failover_budget_exhausted_error() -> String {
        FAILOVER_BUDGET_EXHAUSTED_MSG.to_string()
    }

    /// PA-174 B2：聚合/换手预算耗尽时的收口错误——**最后一次真实 provider 错误优先**。
    ///
    /// 沿用 PA-173 的 `last_provider_error` 模式：真实错误（超时 / 5xx …）原样返回后，重试
    /// 内核与换手层仍按既有分类处理（超时/5xx 仍 eligible ⇒ 候选用自己的 deadline 继续）；
    /// 仅当从未发出任何 HTTP（无真实错误）时才退回合成错误。
    fn budget_exhausted_error(&self) -> String {
        self.last_provider_error
            .lock()
            .ok()
            .and_then(|slot| slot.clone())
            .unwrap_or_else(Self::failover_budget_exhausted_error)
    }

    /// PA-174 B2：记录最近一次**真实** provider 失败（合成预算错误不覆盖）。
    fn record_provider_error(&self, error: &str) {
        if error == FAILOVER_BUDGET_EXHAUSTED_MSG {
            return;
        }
        if let Ok(mut slot) = self.last_provider_error.lock() {
            *slot = Some(error.to_string());
        }
    }

    /// PA-172 §3.1 builder：注入上游调用策略（超时/尝试次数/退避预算）。
    pub fn with_policy(mut self, policy: UpstreamPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// PA-172 §3.1 builder：注入换手链（空链等价于未开启换手）。
    pub fn with_failover(mut self, chain: ProviderFailoverChain) -> Self {
        self.failover = if chain.is_empty() { None } else { Some(chain) };
        self
    }

    /// 当前生效的上游调用策略。
    pub fn upstream_policy(&self) -> &UpstreamPolicy {
        &self.policy
    }

    /// 当前换手链（未开启时为 `None`）。
    pub fn failover_chain(&self) -> Option<&ProviderFailoverChain> {
        self.failover.as_ref()
    }

    /// PA-171 C3 + PA-172 §5 R3：冷却键四段 `requested_name|provider_name|model|base_url`。
    ///
    /// 第四段 `base_url` 是**含 base_url 防止跨 endpoint 误杀**（裁决 D68-7）：同一
    /// provider 名/模型在不同 endpoint（回环 mock、多环境网关）各有独立配额窗口，
    /// 共享冷却会把 A 环境的 429 误伤到 B 环境（红相 503 反向钉子实测被短路）。
    /// PA-172 起四段各自单射转义 + base_url 归一（见 [`cooldown_key_for`]）。
    fn cooldown_key(&self) -> String {
        cooldown_key_for(&self.config)
    }

    /// PA-171 C3：当前 provider 的剩余冷却时长（宿主/测试可观测）。
    pub fn cooldown_remaining(&self) -> Option<Duration> {
        ProviderCooldownRegistry::global().remaining(&self.cooldown_key())
    }

    /// PA-171 C3：显式解除冷却（恢复开关/测试复位）。
    pub fn clear_cooldown(&self) {
        ProviderCooldownRegistry::global().clear(&self.cooldown_key())
    }

    /// PA-171 C3-P1：冷却期内**发送前**短路（0 次 HTTP），文案含类别 + 恢复时间。
    fn ensure_not_cooling(&self) -> Result<(), String> {
        let Some(remaining) = self.cooldown_remaining() else {
            return Ok(());
        };
        let secs = remaining.as_secs().max(1);
        let class = ProviderCooldownRegistry::global()
            .class_for(&self.cooldown_key())
            .unwrap_or("rate_limited");
        // nfr-baseline-wave68 `wave68_logging_events`：冷却期 0 次 HTTP，结构化日志是唯一
        // 可观测面（metrics 为结构性 N-A）。只打声明字段，不含响应正文/密钥。
        pony_observability::otel::log::log_json_with_fields(
            "warn",
            "provider_cooldown_short_circuit",
            None,
            &[
                (
                    "provider_name",
                    serde_json::json!(self.config.provider_name),
                ),
                ("model", serde_json::json!(self.config.model)),
                ("remaining_ms", serde_json::json!(remaining.as_millis() as u64)),
            ],
        );
        // L3-B ADVISORY：短路错误打标 ⇒ 重试编排全 scope 立即 Abort（不 sleep、不再 HTTP）。
        // B1795506800001：来源标记为 `short_circuit`（本轮 0 次 HTTP），与 HTTP 出口的
        // `entered`（真的收到过响应并登记冷却）区分——短窗口 429 的「预算内等待」只对后者开放。
        Err(crate::retry::append_cooldown_marker(
            &format!(
                "provider {} 处于限流冷却中（{class}），预计 {secs}s 后恢复；本次未发起请求；retry_after={secs}",
                self.config.provider_name
            ),
            crate::retry::CooldownMarkerSource::ShortCircuit,
        ))
    }

    /// PA-171 C3-P1/C3-P6：命中限流/配额才登记冷却（其余类别保留既有重试语义）。
    /// 返回 `true` = 本次确实进入/刷新了冷却（调用方据此给错误文本打冷却标记）。
    fn enter_cooldown(&self, error: &str) -> bool {
        let class = crate::retry::classify_error_class(error);
        if !matches!(class, "rate_limited" | "quota") {
            return false;
        }
        let retry_after_s = crate::retry::parse_retry_after_marker(error);
        let window_ms = crate::retry::rate_limit_window_ms(
            error,
            crate::retry::utc_now_hm(),
        )
        .unwrap_or(COOLDOWN_DEFAULT_MS)
        .clamp(1_000, COOLDOWN_MAX_MS);
        let until_unix_ms = now_unix_ms() + window_ms;
        // nfr-baseline-wave68 `wave68_logging_events`：进入冷却的结构化留痕
        // （事件名逐字；字段 = 声明清单，禁止夹带响应正文/密钥）。
        pony_observability::otel::log::log_json_with_fields(
            "warn",
            "provider_cooldown_entered",
            None,
            &[
                (
                    "provider_name",
                    serde_json::json!(self.config.provider_name),
                ),
                ("model", serde_json::json!(self.config.model)),
                ("error_class", serde_json::json!(class)),
                ("retry_after_s", serde_json::json!(retry_after_s)),
                ("until_unix_ms", serde_json::json!(until_unix_ms)),
            ],
        );
        ProviderCooldownRegistry::global().enter(
            &self.cooldown_key(),
            ProviderCooldownEntry {
                until_unix_ms,
                class,
                retry_after_s,
            },
        );
        true
    }

    /// W2（additive）：当前生效的 provider 选择真值（宿主继承源）。
    ///
    /// 只读借用，不复制、不重解析；队友/子代理继承宿主 provider 时以此为准。
    pub fn selection(&self) -> &ResolvedProviderSelection {
        &self.config
    }

    pub fn requested_name(&self) -> &str {
        &self.config.requested_name
    }

    pub fn name(&self) -> &str {
        &self.config.provider_name
    }

    pub fn model(&self) -> &str {
        &self.config.model
    }

    /// 规范协议名（wire 值）。仅用于遥测/trace 落盘字符串；历史 trace 旧名不迁移。
    pub fn protocol_label(&self) -> &'static str {
        match self.config.protocol {
            ProviderProtocol::OpenAiCompletions => "openai-completions",
            ProviderProtocol::OpenAiResponses => "openai-responses",
            ProviderProtocol::AnthropicMessages => "anthropic-messages",
        }
    }

    /// 当前生效协议（行为分叉位点统一传枚举，禁止裸字符串比较）。
    pub fn protocol(&self) -> &ProviderProtocol {
        &self.config.protocol
    }

    pub fn temperature(&self) -> f32 {
        self.config.temperature
    }

    pub fn max_output_tokens(&self) -> u32 {
        self.config.max_output_tokens
    }

    /// 该 provider 模型是否具备流式输出能力。
    pub fn supports_streaming(&self) -> bool {
        self.config.capabilities.supports_streaming
    }

    pub fn supports_true_streaming_followup(&self) -> bool {
        if !self.config.capabilities.supports_streaming {
            return false;
        }

        // responses 与 completions 同为真流式（openai 家族判断）。
        self.config.protocol.is_openai_family()
    }

    pub fn supports_true_streaming_decision(&self) -> bool {
        self.config.capabilities.supports_streaming && self.config.protocol.is_openai_family()
    }

    pub fn context_window_tokens(&self) -> Option<u32> {
        self.config.capabilities.context_window_tokens
    }

    pub fn supports_image_input(&self) -> bool {
        self.config.capabilities.supports_image_input
    }

    pub fn supports_native_tool_transcript_replay(&self) -> bool {
        // ADR 0123：OpenAI 协议家族且具备工具能力的主流模型支持原生多轮工具流回放
        self.config.protocol.is_openai_family() && self.config.capabilities.supports_tools
    }

    /// ADR 0123 写侧唯一判定：本模型是否应把本轮（含纯文本轮）写入
    /// `provider_native_transcript`。同步与流式两条出口必须共用此判定，否则
    /// 纯文本轮缺失会造成后续工具轮的前缀失忆。
    pub fn persists_native_tool_transcript(&self) -> bool {
        self.requires_provider_native_tool_flow() || self.supports_native_tool_transcript_replay()
    }

    pub fn requires_provider_native_tool_flow(&self) -> bool {
        // responses 与 completions 共用 native tool flow（openai 家族判断）。
        self.config.protocol.is_openai_family() && self.config.capabilities.supports_reasoning
    }

    fn sanitize_request<'a>(&self, request: &'a ProviderRequest) -> std::borrow::Cow<'a, ProviderRequest> {
        if !self.supports_image_input() && !request.images.is_empty() {
            provider_log(format!(
                "warn: non-vision model {} received {} images; stripping for safety",
                self.model(),
                request.images.len()
            ));
            let mut req = request.clone();
            req.images.clear();
            std::borrow::Cow::Owned(req)
        } else {
            std::borrow::Cow::Borrowed(request)
        }
    }

    /// PA-172 §3.4：**唯一参与换手的入口**——决策同步调用（压缩摘要路径
    /// `turn_prep.rs:66` 走同一方法 ⇒ 继承同一链）。
    ///
    /// F-d（位置冻结）：hop 循环在 [`Self::decide_with_tools_single_hop`] **之外**，而后者
    /// 内部才是 `retry_provider_scoped_with_report` 的调用点 ⇒ **每个候选各拿一份独立
    /// `RetryBudget`**（把换手塞进协议 match 闭包会让 3 个 provider 共用同一 5 次/30s
    /// 预算，"有界"变假）。流式入口（`decide_with_tools_stream*` /
    /// `continue_with_tool_result_stream*`）与 followup 入口**不参与**换手（F-g / v3 收窄）。
    pub fn decide_with_tools(
        &self,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
    ) -> Result<ProviderDecision, String> {
        // PA-174 F2：同步决策注入跨 attempt 总 deadline（`aggregate_sync_call_budget_ms`）。
        // 换手候选自带 `FAILOVER_BUDGET_MS` deadline，`with_aggregate_sync_deadline`
        // 对已带 deadline 的实例原样返回 ⇒ 换手语义与既有预算零改动。
        let executor = self.with_aggregate_sync_deadline();
        let (result, raw_error) = executor.decide_with_tools_single_hop(request, tools);
        match result {
            Ok(decision) => Ok(decision),
            Err(detail) => self.failover_decide_with_tools(detail, raw_error, request, tools),
        }
    }

    /// PA-172 §3.3 F-a..F-h：有界换手编排（**只在 `decide_with_tools` 之后调用**）。
    ///
    /// 返回最后一次真实失败（G4：任何路径都不得把失败改写成成功）。
    fn failover_decide_with_tools(
        &self,
        primary_detail: String,
        primary_raw: Option<String>,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
    ) -> Result<ProviderDecision, String> {
        // 未装链 ⇒ 与基线逐字一致（G2：默认零行为变更），返回既有
        // `extract_provider_error_detail` 处理后的文本。
        //
        // PA-174 B2：但**原始错误优先**——聚合预算耗尽时若只剩正文预览，用户就看不到
        // 真实状态码（503/timeout），且「预算耗尽」的合成错误会顶替真实失败。
        let Some(chain) = self.failover.as_ref() else {
            return Err(primary_raw.unwrap_or(primary_detail));
        };
        // 装链后：**不换手**的路径必须"原样上抛"主 provider 的错误（§3.2 反向钉子：
        // 400/500 不得被换手层改写，也不得只剩响应正文）。主 provider 无 key 属配置
        // 错误 ⇒ 快速失败、不换手（F-h，保持既有文案）。
        let verbatim = |detail: String| primary_raw.clone().unwrap_or(detail);
        if self.config.api_key.is_none() {
            return Err(verbatim(primary_detail));
        }
        // §3.2 触发类封闭值域：不在码表内（bad_request / other / HTTP 500）一律不换手。
        //
        // L3-B BLOCKING-2（合入审查）：判定必须喂**原始错误**（含 `provider 返回错误状态：<code>` 状态行），
        // 不能喂 `extract_provider_error_detail` 的正文预览 —— 后者丢掉了状态码，会让「HTTP 500 不换手」
        // 变成由上游正文决定（500 正文含 rate limit 即换手），并让真正的 429（正文无关键词）静默不换手。
        let verdict_source = primary_raw.as_deref().unwrap_or(primary_detail.as_str());
        if !crate::retry::failover_eligible(verdict_source) {
            return Err(verbatim(primary_detail));
        }
        let primary_error = primary_detail;
        // F-c：换手预算从**第一次换手**开始计时，且是逐 attempt 的 deadline。
        let budget_started = Instant::now();
        let deadline = budget_started + Duration::from_millis(FAILOVER_BUDGET_MS);
        let mut last_error = primary_error;
        let mut hops = 0usize;
        let mut attempted: Vec<String> = vec![self.config.provider_name.clone()];
        for candidate in &chain.selections {
            if hops >= MAX_FAILOVER_HOPS {
                break;
            }
            // F-b：候选处于冷却期 ⇒ 跳过、**0 次 HTTP**（不得先发请求再判冷却）。
            if let Some(remaining) =
                ProviderCooldownRegistry::global().remaining(&cooldown_key_for(candidate))
            {
                pony_observability::otel::log::log_json_with_fields(
                    "warn",
                    "provider_failover_skipped_cooldown",
                    None,
                    &[
                        ("provider_name", serde_json::json!(candidate.provider_name)),
                        ("model", serde_json::json!(candidate.model)),
                        ("remaining_ms", serde_json::json!(remaining.as_millis() as u64)),
                        ("hop", serde_json::json!(hops + 1)),
                    ],
                );
                continue;
            }
            // F-c：预算耗尽 ⇒ 立即结束换手（0 次 HTTP）。这里只是"整条链还要不要继续"
            // 的快速判定；**每次 attempt** 的请求超时由 `effective_attempt_timeout_ms`
            // 在发请求前重算（逐 attempt deadline，唯一 `min` 出口在 retry 侧）。
            if deadline.saturating_duration_since(Instant::now()).is_zero() {
                break;
            }
            hops += 1;
            pony_observability::otel::log::log_json_with_fields(
                "warn",
                "provider_failover_switched",
                None,
                &[
                    ("from_provider", serde_json::json!(self.config.provider_name)),
                    ("to_provider", serde_json::json!(candidate.provider_name)),
                    (
                        "error_class",
                        serde_json::json!(crate::retry::classify_error_class(
                            &last_error
                        )),
                    ),
                    ("hop", serde_json::json!(hops)),
                    (
                        "elapsed_ms",
                        serde_json::json!(
                            budget_started.elapsed().as_millis().min(u128::from(u64::MAX))
                                as u64
                        ),
                    ),
                ],
            );
            // F-d：每个候选一个**独立** ProviderManager（独立 RetryBudget），且不带链
            // （候选自身失败不再递归换手 —— hop 上限由本循环统一控制）；策略逐字继承，
            // 只额外带绝对截止时刻（每次 attempt 再按剩余量收紧）。
            let hop_manager = ProviderManager::new(candidate.clone())
                .with_policy(self.policy.clone())
                .with_attempt_deadline(deadline);
            match hop_manager.decide_with_tools_single_hop(request, tools).0 {
                Ok(decision) => {
                    provider_log(format!(
                        "decision:failover-success hop={} from={} to={}",
                        hops, self.config.provider_name, candidate.provider_name
                    ));
                    return Ok(decision);
                }
                Err(error) => {
                    last_error = error;
                    attempted.push(candidate.provider_name.clone());
                }
            }
        }
        if hops == 0 {
            // 全链候选都在冷却期被跳过 ⇒ 没有发生换手，"耗尽"不成立：只保留逐条
            // `provider_failover_skipped_cooldown` 留痕，**不**发 `provider_failover_exhausted`
            // 也不给错误加 `failover_exhausted=` 标记（事件与标记必须同真同假，
            // 禁止"事件说耗尽、错误说没换手"）。
            return Err(last_error);
        }
        pony_observability::otel::log::log_json_with_fields(
            "warn",
            "provider_failover_exhausted",
            None,
            &[
                ("hops", serde_json::json!(hops)),
                (
                    "last_error_class",
                    serde_json::json!(crate::retry::classify_error_class(&last_error)),
                ),
                ("attempted_providers", serde_json::json!(attempted)),
            ],
        );
        Err(crate::retry::append_failover_exhausted_marker(
            &last_error, hops,
        ))
    }

    /// 单跳决策（既有语义逐字保留：无换手、无策略外行为）。
    /// 返回 `(结果, 最后一次 attempt 的**原始**错误文本)`。
    ///
    /// 原始错误（含 `provider 返回错误状态：<code> …；响应正文：` 头部）与重试编排最终返回的
    /// 结构化详情是两份东西：不换手时必须原样上抛原始错误（§3.2 反向钉子：400/500 不得被
    /// 换手层改写成只剩响应正文），换手耗尽时按 F-e 交付结构化详情 + 标记。
    fn decide_with_tools_single_hop(
        &self,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
    ) -> (Result<ProviderDecision, String>, Option<String>) {
        let request_cow = self.sanitize_request(request);
        let request = request_cow.as_ref();
        provider_log(format!(
            "decision:start requested={} provider={} protocol={} model={} tools={} api_key_present={} ",
            self.config.requested_name,
            self.config.provider_name,
            self.protocol_label(),
            request.model,
            tools.len(),
            self.config.api_key.is_some()
        ));
        if self.config.api_key.is_none() {
            let error = format!(
                "provider {} missing API key; please save a key in the model config page (field: {}).",
                self.config.provider_name, self.config.api_key_env_var
            );
            provider_log(format!("decision:error {}", error));
            return (Err(error), None);
        }

        let mut raw_error: Option<String> = None;
        let result = retry_provider_timeout_with_policy_deadline(
            "decision",
            &self.policy,
            self.sleep_deadline,
            || {
                let outcome = match self.config.protocol {
                    ProviderProtocol::OpenAiCompletions => {
                        self.send_openai_tool_decision_request(request, tools)
                    }
                    ProviderProtocol::OpenAiResponses => {
                        self.send_responses_tool_decision_request(request, tools)
                    }
                    ProviderProtocol::AnthropicMessages => {
                        self.send_anthropic_tool_decision_request(request, tools)
                    }
                };
                if let Err(error) = &outcome {
                    raw_error = Some(error.clone());
                }
                outcome
            },
        );

        let result = match result {
            Ok(decision) => {
                provider_log(format!(
                    "decision:success mode={} tool_call={} output_preview={} ",
                    decision.provider_mode,
                    decision
                        .tool_call
                        .as_ref()
                        .map(|call| call.name.as_str())
                        .unwrap_or("none"),
                    preview_text(&decision.output_text, 120)
                ));
                Ok(decision)
            }
            Err(error) => {
                provider_log(format!("decision:error {}", error));
                Err(error)
            }
        };
        (result, raw_error)
    }

    /// 无守卫版本（保持既有调用面与测试替身语义；取消与时长上界不生效）。
    pub fn decide_with_tools_stream<F>(
        &self,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
        on_delta: F,
    ) -> Result<ProviderDecision, String>
    where
        F: FnMut(ProviderStreamChunk),
    {
        self.decide_with_tools_stream_bounded(
            &ProviderStreamGuard::without_cancel(),
            request,
            tools,
            on_delta,
        )
    }

    /// T9：带守卫的流式决策 —— runtime 生产路径由此进入，取消/时长上界真正生效。
    pub fn decide_with_tools_stream_bounded<F>(
        &self,
        guard: &ProviderStreamGuard<'_>,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
        on_delta: F,
    ) -> Result<ProviderDecision, String>
    where
        F: FnMut(ProviderStreamChunk),
    {
        let request_cow = self.sanitize_request(request);
        let request = request_cow.as_ref();
        provider_log(format!(
            "decision:start mode=stream requested={} provider={} protocol={} model={} tools={} api_key_present={} ",
            self.config.requested_name,
            self.config.provider_name,
            self.protocol_label(),
            request.model,
            tools.len(),
            self.config.api_key.is_some()
        ));
        if self.config.api_key.is_none() {
            let error = format!(
                "provider {} missing API key; please save a key in the model config page (field: {}).",
                self.config.provider_name, self.config.api_key_env_var
            );
            provider_log(format!("decision:error {}", error));
            return Err(error);
        }

        let on_delta_cell = std::cell::RefCell::new(on_delta);
        // B1795488338107 评审 B1：与 followup 流式臂同构——已发出可见文本增量后禁止自动
        // 重试（否则 attempt 1 的半截正文与 attempt 2 的输出会重复上屏，且污染首字延迟）。
        let emitted_visible = std::cell::Cell::new(false);
        let result = retry_provider_decision_stream_with_policy(
            "decision_stream",
            &self.policy,
            || {
                let mut delta_sink = on_delta_cell.borrow_mut();
                let mut record_visible = |chunk: ProviderStreamChunk| {
                    if matches!(chunk, ProviderStreamChunk::Text(_)) {
                        emitted_visible.set(true);
                    }
                    delta_sink(chunk);
                };
                match self.config.protocol {
                    ProviderProtocol::OpenAiCompletions => self
                        .send_openai_tool_decision_stream_request(
                            guard,
                            request,
                            tools,
                            &mut record_visible,
                        ),
                    ProviderProtocol::OpenAiResponses => self
                        .send_responses_tool_decision_stream_request(
                            guard,
                            request,
                            tools,
                            &mut record_visible,
                        ),
                    ProviderProtocol::AnthropicMessages => {
                        self.send_anthropic_tool_decision_request(request, tools)
                    }
                }
            },
            &|| emitted_visible.get(),
        );

        match result {
            Ok(decision) => {
                provider_log(format!(
                    "decision:success mode={} tool_call={} output_preview={} ",
                    decision.provider_mode,
                    decision
                        .tool_call
                        .as_ref()
                        .map(|call| call.name.as_str())
                        .unwrap_or("none"),
                    preview_text(&decision.output_text, 120)
                ));
                Ok(decision)
            }
            Err(error) => {
                provider_log(format!("decision:error {}", error));
                Err(error)
            }
        }
    }

    pub fn continue_with_tool_result(
        &self,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
        accumulated_messages: &mut Vec<Value>,
        assistant_message: Option<&Value>,
        tool_call: &ToolCall,
        tool_result: &ToolResult,
    ) -> Result<ProviderResponse, String> {
        // PA-174 F2：同步（1:1）followup 注入跨 attempt 总 deadline；协议分支内的所有
        // 真实请求都经 `executor` 发出（日志/策略仍读 `self`，取值逐字相同）。
        let executor = self.with_aggregate_sync_deadline();
        let request_cow = self.sanitize_request(request);
        let request = request_cow.as_ref();
        provider_log(format!(
            "followup:start mode=sync requested={} provider={} protocol={} model={} tool={} tool_status={} tool_output_preview={} ",
            self.config.requested_name,
            self.config.provider_name,
            self.protocol_label(),
            request.model,
            tool_call.name,
            tool_result.status,
            preview_text(&tool_result.output, 160)
        ));
        if self.config.api_key.is_none() {
            let error = format!(
                "provider {} missing API key; please save a key in the model config page (field: {}).",
                self.config.provider_name, self.config.api_key_env_var
            );
            provider_log(format!("followup:error {}", error));
            return Err(error);
        }

        // PA-174 F1：1:1 与批量同构的两条约束——(1) 每次 attempt 从进入本入口时的
        // pristine base 重建转录（构造器每次尝试都回写 `*accumulated_messages`，
        // 不重置 ⇒ 重试把 assistant + tool 再追加一遍）；(2) 重试内核耗尽时返回
        // `extract_provider_error_detail`（只剩响应正文、空正文 ⇒ 空串），故逐次记录原始
        // 错误并在收口处替换降级结果。成功 = base + 恰好 1 次追加。
        let base_messages = accumulated_messages.clone();
        match self.config.protocol {
            ProviderProtocol::OpenAiCompletions => {
                let mut last_provider_error: Option<String> = None;
                retry_provider_followup_timeout_with_policy_deadline(
                    "followup_sync",
                    &self.policy,
                    executor.sleep_deadline,
                    || {
                        *accumulated_messages = base_messages.clone();
                        executor.send_openai_tool_followup_request(
                            request,
                            tools,
                            accumulated_messages,
                            assistant_message,
                            tool_call,
                            tool_result,
                        )
                        .map_err(|error| {
                            last_provider_error = Some(error.clone());
                            error
                        })
                    },
                )
                .map_err(|degraded| last_provider_error.unwrap_or(degraded))
                .or_else(|error| {
                    provider_log(format!(
                        "followup:local-fallback protocol=openai provider={} model={} reason={}",
                        self.config.provider_name, request.model, error
                    ));
                    Ok(local_tool_followup_fallback_response(
                        request,
                        tool_call,
                        tool_result,
                        error,
                    ))
                })
            }
            ProviderProtocol::OpenAiResponses => {
                // sync 失败本地 fallback 语义与 completions 分支对齐。
                let mut last_provider_error: Option<String> = None;
                retry_provider_followup_timeout_with_policy_deadline(
                    "followup_sync",
                    &self.policy,
                    executor.sleep_deadline,
                    || {
                        *accumulated_messages = base_messages.clone();
                        executor.send_responses_tool_followup_request(
                            request,
                            tools,
                            accumulated_messages,
                            assistant_message,
                            tool_call,
                            tool_result,
                        )
                        .map_err(|error| {
                            last_provider_error = Some(error.clone());
                            error
                        })
                    },
                )
                .map_err(|degraded| last_provider_error.unwrap_or(degraded))
                .or_else(|error| {
                    provider_log(format!(
                        "followup:local-fallback protocol=openai-responses provider={} model={} reason={}",
                        self.config.provider_name, request.model, error
                    ));
                    Ok(local_tool_followup_fallback_response(
                        request,
                        tool_call,
                        tool_result,
                        error,
                    ))
                })
            }
            // PA-174 F1-b/F1-c：Anthropic 1:1 臂此前**无重试壳**（单次 5xx 即终态），
            // 与 OpenAI 臂行为分叉；此臂没有本地 fallback，耗尽后必须原样上抛原始错误。
            ProviderProtocol::AnthropicMessages => {
                let mut last_provider_error: Option<String> = None;
                retry_provider_followup_timeout_with_policy_deadline(
                    "followup_sync",
                    &self.policy,
                    executor.sleep_deadline,
                    || {
                        *accumulated_messages = base_messages.clone();
                        executor.send_anthropic_tool_followup_request(
                            request,
                            tools,
                            accumulated_messages,
                            assistant_message,
                            tool_call,
                            tool_result,
                        )
                        .map_err(|error| {
                            last_provider_error = Some(error.clone());
                            error
                        })
                    },
                )
                .map_err(|degraded| last_provider_error.unwrap_or(degraded))
            }
        }
    }

    /// 无守卫版本（保持既有调用面与测试替身语义）。
    pub fn continue_with_tool_result_stream<F>(
        &self,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
        accumulated_messages: &mut Vec<Value>,
        assistant_message: Option<&Value>,
        tool_call: &ToolCall,
        tool_result: &ToolResult,
        on_delta: F,
    ) -> Result<ProviderResponse, String>
    where
        F: FnMut(ProviderStreamChunk),
    {
        self.continue_with_tool_result_stream_bounded(
            &ProviderStreamGuard::without_cancel(),
            request,
            tools,
            accumulated_messages,
            assistant_message,
            tool_call,
            tool_result,
            on_delta,
        )
    }

    /// T9：带守卫的流式工具跟进（runtime 生产路径）。
    #[allow(clippy::too_many_arguments)]
    pub fn continue_with_tool_result_stream_bounded<F>(
        &self,
        guard: &ProviderStreamGuard<'_>,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
        accumulated_messages: &mut Vec<Value>,
        assistant_message: Option<&Value>,
        tool_call: &ToolCall,
        tool_result: &ToolResult,
        mut on_delta: F,
    ) -> Result<ProviderResponse, String>
    where
        F: FnMut(ProviderStreamChunk),
    {
        // B254 C3：流式 followup 不再回退同步缓冲通道，故这里不派生聚合 deadline
        // （180s 每轮预算只约束同步调用；流式 attempt 沿用 guard 的取消预算）。
        let request_cow = self.sanitize_request(request);
        let request = request_cow.as_ref();
        if !self.config.capabilities.supports_streaming {
            let response = self.continue_with_tool_result(
                request,
                tools,
                accumulated_messages,
                assistant_message,
                tool_call,
                tool_result,
            )?;
            replay_sync_response_as_deltas(&response, &mut on_delta);
            return Ok(response);
        }

        self.dispatch_followup_stream_bounded(
            guard,
            request,
            tools,
            accumulated_messages,
            assistant_message,
            std::slice::from_ref(tool_call),
            std::slice::from_ref(tool_result),
            on_delta,
        )
    }

    fn dispatch_followup_stream_bounded<F>(
        &self,
        guard: &ProviderStreamGuard<'_>,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
        accumulated_messages: &mut Vec<Value>,
        assistant_message: Option<&Value>,
        calls: &[ToolCall],
        results: &[ToolResult],
        mut on_delta: F,
    ) -> Result<ProviderResponse, String>
    where
        F: FnMut(ProviderStreamChunk),
    {
        if calls.len() == 1 && results.len() == 1 {
            let tool_call = &calls[0];
            let tool_result = &results[0];
            provider_log(format!(
                "followup:start mode=stream requested={} provider={} protocol={} model={} tool={} tool_status={} tool_output_preview={} ",
                self.config.requested_name,
                self.config.provider_name,
                self.protocol_label(),
                request.model,
                tool_call.name,
                tool_result.status,
                preview_text(&tool_result.output, 160)
            ));
        } else {
            provider_log(format!(
                "followup:start mode=stream requested={} provider={} protocol={} model={} calls={} ",
                self.config.requested_name,
                self.config.provider_name,
                self.protocol_label(),
                request.model,
                calls.len()
            ));
        }
        if self.config.api_key.is_none() {
            let error = format!(
                "provider {} missing API key; please save a key in the model config page (field: {}).",
                self.config.provider_name, self.config.api_key_env_var
            );
            provider_log(format!("followup:error {}", error));
            return Err(error);
        }

        match self.config.protocol {
            ProviderProtocol::OpenAiCompletions | ProviderProtocol::OpenAiResponses => {
                // 流式断流/超时等网络瞬态错误允许退避重试；仅在用户显式取消或发出可见文本后放弃。
                let on_delta_cell = std::cell::RefCell::new(on_delta);
                // PA-174 F1 的**流式对偶**：每次尝试前重置为入口时的 pristine 快照，
                // 避免重试向 `accumulated_messages` 尾部重复追加 `assistant + tool`。
                let base_messages = accumulated_messages.clone();
                // 跨 attempt 累计：一旦发出可见 Text 增量即置位，禁止自动重试导致重复上屏。
                let emitted_visible = std::cell::Cell::new(false);
                retry_provider_followup_stream_timeout_with_policy(
                    "followup_stream",
                    &self.policy,
                    || {
                        accumulated_messages.clone_from(&base_messages);
                        let mut delta_sink = on_delta_cell.borrow_mut();
                        let mut record_visible = |chunk: ProviderStreamChunk| {
                            if matches!(chunk, ProviderStreamChunk::Text(_)) {
                                emitted_visible.set(true);
                            }
                            delta_sink(chunk);
                        };
                        match self.config.protocol {
                            ProviderProtocol::OpenAiResponses => self
                                .send_responses_tool_followup_stream_request(
                                    guard,
                                    request,
                                    tools,
                                    accumulated_messages,
                                    assistant_message,
                                    calls,
                                    results,
                                    &mut record_visible,
                                ),
                            _ => self.send_openai_tool_followup_stream_request(
                                guard,
                                request,
                                tools,
                                accumulated_messages,
                                assistant_message,
                                calls,
                                results,
                                &mut record_visible,
                            ),
                        }
                    },
                    &|| guard.cancel_requested() || emitted_visible.get(),
                )
            }
            ProviderProtocol::AnthropicMessages => self
                .send_anthropic_tool_followup_stream_request(
                    guard,
                    request,
                    tools,
                    accumulated_messages,
                    assistant_message,
                    calls,
                    results,
                    &mut on_delta,
                ),
        }
    }

    // ───────────────────── D2（P1-1 切片二）：批量工具结果 followup 入口 ─────────────────────
    //
    // 分派顺序**不可变**（设计 `multi-tool-call-execution.md` §6）：
    //   1) `calls.len() != results.len()` ⇒ 立刻 `Err`，**不构造任何请求**（零 HTTP）；
    //   2) `calls.len() == 1` ⇒ 逐字委派既有 1:1 入口（N=1 行为零改动，靠既有用例回归）；
    //   3) `N > 1` ⇒ D1b 批量构造器组装 messages，复用既有 `post_*` 通道与解析尾巴。
    // 既有 1:1 入口与 `turn_stream.rs` / `turn_sync.rs` 调用面**不动**：批量语义只在 D3 接线后生效。

    /// D2：批量工具结果 followup（同步）。
    ///
    /// 参数语义与 1:1 入口一致，只是 `calls`/`results` 为**按执行顺序**的整批；
    /// `accumulated_messages` 与 1:1 路径一样被更新为「含本次回填的完整转录」。
    #[allow(clippy::too_many_arguments)]
    pub fn continue_with_tool_results(
        &self,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
        accumulated_messages: &mut Vec<Value>,
        assistant_message: Option<&Value>,
        calls: &[ToolCall],
        results: &[ToolResult],
    ) -> Result<ProviderResponse, String> {
        // 1) 长度不匹配：先于任何请求构造 / provider 日志 / 鉴权检查早退。
        if calls.len() != results.len() {
            return Err(format!(
                "tool_calls/tool_results length mismatch: calls={} results={}",
                calls.len(),
                results.len()
            ));
        }
        if calls.is_empty() {
            return Err("tool_calls/tool_results must not be empty".to_string());
        }
        // 2) N=1：逐字委派既有 1:1 入口（不复制其逻辑）。
        if calls.len() == 1 {
            return self.continue_with_tool_result(
                request,
                tools,
                accumulated_messages,
                assistant_message,
                &calls[0],
                &results[0],
            );
        }

        // 3) N>1：批量回填。
        // PA-174 F2：同步批量 followup 同样注入跨 attempt 总 deadline（所有真实请求经
        // `executor` 发出）；N=1 分支已委派 1:1 入口，其自身注入同一预算。
        let executor = self.with_aggregate_sync_deadline();
        let request_cow = self.sanitize_request(request);
        let request = request_cow.as_ref();
        provider_log(format!(
            "followup:start mode=sync-batch requested={} provider={} protocol={} model={} calls={} ",
            self.config.requested_name,
            self.config.provider_name,
            self.protocol_label(),
            request.model,
            calls.len()
        ));
        if self.config.api_key.is_none() {
            let error = format!(
                "provider {} missing API key; please save a key in the model config page (field: {}).",
                self.config.provider_name, self.config.api_key_env_var
            );
            provider_log(format!("followup:error {}", error));
            return Err(error);
        }

        // PA-173 D3：批量 followup 必须与 1:1 路径**同等重试**（复用同一
        // `provider_timeout_backoff_config_for` 退避；label 用 `followup_sync_batch`，与 1:1
        // 的 `followup_sync` 不冲突）。预算耗尽后原始错误原样上抛，不做本地 fallback
        // （批量路径无 fallback helper，契约也不新增）。
        //
        // 转录去重（本卡新增，非 1:1 照抄）：批量构造器在**每次尝试内**写回
        // `accumulated_messages`（`openai_followup_messages_for_tool_results` /
        // `anthropic_followup_messages_for_tool_results`）。若重试沿用上次写回结果作 base，
        // 上游会收到重复 tool_use/tool_result——这是加壳后**新引入**的回归，故每次尝试前
        // 都把 `accumulated_messages` 重置为进入批量 dispatch 时的 pristine 快照：
        // 成功 = base + 恰好 1 次追加；**失败 = 停在最后一次尝试写入后的转录（无回滚，
        // 与 1:1 同构）**——调用方拿到 Err 后不应再把该转录当作干净 base 复用。
        // 1:1 路径（`openai_messages_with_tool_result` 同族）的同类重复追加仍是
        // **既有缺陷**，登记后续卡，本卡不改其行为。
        //
        // 错误保真（本卡新增）：重试内核耗尽时返回 `extract_provider_error_detail`（只留
        // 「响应正文：」后 400 字，空正文 ⇒ 空串）。批量路径此前不经过重试壳、直接返回完整
        // 错误，加壳后不得让错误退化，故逐次记录**最后一次尝试的原始错误**并在收口处替换
        // 降级结果（绝不返回空串）。1:1 路径的同类降级是既有问题，本卡不动。
        let base_messages = accumulated_messages.clone();
        let mut last_provider_error: Option<String> = None;
        let outcome = match self.config.protocol {
            ProviderProtocol::OpenAiCompletions => retry_provider_followup_timeout_with_policy_deadline(
                "followup_sync_batch",
                &self.policy,
                executor.sleep_deadline,
                || {
                    *accumulated_messages = base_messages.clone();
                    executor.send_openai_tool_followup_batch_request(
                        request,
                        tools,
                        accumulated_messages,
                        assistant_message,
                        calls,
                        results,
                    )
                    .map_err(|error| {
                        last_provider_error = Some(error.clone());
                        error
                    })
                },
            ),
            ProviderProtocol::OpenAiResponses => retry_provider_followup_timeout_with_policy_deadline(
                "followup_sync_batch",
                &self.policy,
                executor.sleep_deadline,
                || {
                    *accumulated_messages = base_messages.clone();
                    executor.send_responses_tool_followup_batch_request(
                        request,
                        tools,
                        accumulated_messages,
                        assistant_message,
                        calls,
                        results,
                    )
                    .map_err(|error| {
                        last_provider_error = Some(error.clone());
                        error
                    })
                },
            ),
            ProviderProtocol::AnthropicMessages => retry_provider_followup_timeout_with_policy_deadline(
                "followup_sync_batch",
                &self.policy,
                executor.sleep_deadline,
                || {
                    *accumulated_messages = base_messages.clone();
                    executor.send_anthropic_tool_followup_batch_request(
                        request,
                        tools,
                        accumulated_messages,
                        assistant_message,
                        calls,
                        results,
                    )
                    .map_err(|error| {
                        last_provider_error = Some(error.clone());
                        error
                    })
                },
            ),
        };
        // 耗尽时退回最后一次尝试的**原始**错误（含状态码/耗时/正文）。
        outcome.map_err(|degraded| last_provider_error.unwrap_or(degraded))
    }

    /// D2/2026-11-27 修订：批量工具结果 followup（流式入口）。
    ///
    /// 分派顺序：
    /// 1) 长度不匹配 / 空集合 ⇒ 立刻 `Err`（零 HTTP）；
    /// 2) `calls.len() == 1` ⇒ 逐字委派既有 1:1 流式入口；
    /// 3) `!supports_streaming` ⇒ 走同步 `continue_with_tool_results` 并整段回放；
    /// 4) `supports_streaming && N > 1` ⇒ 走真流式 SSE 通道（不回退 45s 同步通道）。
    #[expect(
        clippy::too_many_arguments,
        reason = "沿用 provider tool followup 既有签名形状"
    )]
    pub fn continue_with_tool_results_stream<F>(
        &self,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
        accumulated_messages: &mut Vec<Value>,
        assistant_message: Option<&Value>,
        calls: &[ToolCall],
        results: &[ToolResult],
        on_delta: F,
    ) -> Result<ProviderResponse, String>
    where
        F: FnMut(ProviderStreamChunk),
    {
        self.continue_with_tool_results_stream_bounded(
            &ProviderStreamGuard::without_cancel(),
            request,
            tools,
            accumulated_messages,
            assistant_message,
            calls,
            results,
            on_delta,
        )
    }

    /// 带取消守卫的批量工具结果流式 followup 入口。
    #[expect(
        clippy::too_many_arguments,
        reason = "沿用 provider tool followup 既有签名形状"
    )]
    pub fn continue_with_tool_results_stream_bounded<F>(
        &self,
        guard: &ProviderStreamGuard<'_>,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
        accumulated_messages: &mut Vec<Value>,
        assistant_message: Option<&Value>,
        calls: &[ToolCall],
        results: &[ToolResult],
        mut on_delta: F,
    ) -> Result<ProviderResponse, String>
    where
        F: FnMut(ProviderStreamChunk),
    {
        if calls.len() != results.len() {
            return Err(format!(
                "tool_calls/tool_results length mismatch: calls={} results={}",
                calls.len(),
                results.len()
            ));
        }
        if calls.is_empty() {
            return Err("tool_calls/tool_results must not be empty".to_string());
        }
        if calls.len() == 1 {
            return self.continue_with_tool_result_stream_bounded(
                guard,
                request,
                tools,
                accumulated_messages,
                assistant_message,
                &calls[0],
                &results[0],
                on_delta,
            );
        }

        let request_cow = self.sanitize_request(request);
        let request = request_cow.as_ref();
        if !self.config.capabilities.supports_streaming {
            let response = self.continue_with_tool_results(
                request,
                tools,
                accumulated_messages,
                assistant_message,
                calls,
                results,
            )?;
            replay_sync_response_as_deltas(&response, &mut on_delta);
            return Ok(response);
        }

        self.dispatch_followup_stream_bounded(
            guard,
            request,
            tools,
            accumulated_messages,
            assistant_message,
            calls,
            results,
            on_delta,
        )
    }

    /// D2：OpenAI chat completions 的批量回填（1 assistant + N tool），复用 `post_openai_json`。
    #[allow(clippy::too_many_arguments)]
    fn send_openai_tool_followup_batch_request(
        &self,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
        accumulated_messages: &mut Vec<Value>,
        assistant_message: Option<&Value>,
        calls: &[ToolCall],
        results: &[ToolResult],
    ) -> Result<ProviderResponse, String> {
        let endpoint = format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        );
        provider_log(format!(
            "request:openai followup-sync-batch endpoint={} model={} calls={}",
            endpoint,
            request.model,
            calls.len()
        ));
        let messages = openai_followup_messages_for_tool_results(
            request,
            accumulated_messages,
            assistant_message,
            calls,
            results,
        )?;
        {
            let tool_call_ids = calls
                .iter()
                .map(|call| {
                    call.call_id
                        .clone()
                        .unwrap_or_else(|| "tool_call_local".to_string())
                })
                .collect::<Vec<_>>()
                .join(",");
            provider_log(format!(
                "request:openai followup-sync-batch tool_call_ids={} messages={}",
                tool_call_ids,
                messages.len(),
            ));
        }
        let body = with_openai_request_options(
            json!({
                "model": request.model,
                "messages": messages,
                "temperature": request.temperature,
                "max_tokens": request.max_output_tokens,
                "stream": false,
                "tool_choice": "auto",
                "tools": openai_tools_payload(tools)
            }),
            &self.config,
        );
        let body = apply_openai_tool_capability(body, tools, &self.config);
        provider_log(format!(
            "request:openai followup-sync-batch body_preview={}",
            preview_json(&body, 1600)
        ));
        // 与 `send_openai_tool_followup_request` **同一** post 通道与解析尾巴。
        let payload = self.post_openai_json(&endpoint, &body)?;
        let message = first_openai_message(&payload)?;
        let output_text = extract_openai_message_text(message).unwrap_or_default();
        // D3a：非流式入口**不再丢弃**多余调用——整批 `tool_calls` 全量上报给运行时批执行
        // （`runtime/turn_sync.rs`）；单值字段仍是首元素派生视图（D1a 不变量）。
        let tool_calls = extract_openai_tool_calls(message);
        // 单值字段继续走既有单数入口（= 复数版首元素的兼容包装，行为逐字不变）。
        let tool_call = extract_openai_tool_call(message);
        if output_text.trim().is_empty() && tool_call.is_none() {
            return Err("openai tool follow-up missing text or tool call".to_string());
        }

        let token_usage = extract_openai_usage(&payload)
            .unwrap_or_else(|| estimate_token_usage(request, &output_text));
        provider_log_token_usage("openai followup-sync-batch", &token_usage);

        Ok(ProviderResponse {
            output_text: output_text.clone(),
            tool_calls,
            tool_call,
            reasoning_content: extract_openai_message_reasoning_content(message),
            reasoning_content_value: extract_openai_message_reasoning_value(message),
            assistant_message: Some(normalize_openai_assistant_message(message)),
            provider_source: "provider_followup_sync".to_string(),
            provider_mode: "live".to_string(),
            fallback_reason: None,
            token_usage: Some(token_usage),
        })
    }

    /// D2：OpenAI Responses 的批量回填（chat 形态转录 → 请求边界转换），复用 `post_openai_json`。
    #[allow(clippy::too_many_arguments)]
    fn send_responses_tool_followup_batch_request(
        &self,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
        accumulated_messages: &mut Vec<Value>,
        assistant_message: Option<&Value>,
        calls: &[ToolCall],
        results: &[ToolResult],
    ) -> Result<ProviderResponse, String> {
        let endpoint = self.responses_endpoint();
        provider_log(format!(
            "request:responses followup-sync-batch endpoint={} model={} calls={}",
            endpoint,
            request.model,
            calls.len()
        ));
        let messages = openai_followup_messages_for_tool_results(
            request,
            accumulated_messages,
            assistant_message,
            calls,
            results,
        )?;
        let mut conversion_request = request.clone();
        conversion_request.native_messages = messages;
        let body = build_responses_request_body(&conversion_request, tools, &self.config, false);
        provider_log(format!(
            "request:responses followup-sync-batch body_preview={}",
            preview_json(&body, 1600)
        ));
        let payload = self.post_openai_json(&endpoint, &body)?;
        let parsed = parse_responses_output(&payload)?;
        if parsed.dropped_function_calls > 0 {
            provider_log(format!(
                "followup:responses dropped_function_calls={}",
                parsed.dropped_function_calls
            ));
        }
        let token_usage = parsed
            .token_usage
            .clone()
            .unwrap_or_else(|| estimate_token_usage(request, &parsed.output_text));
        provider_log_token_usage("responses followup-sync-batch", &token_usage);

        let assistant_message = responses_chat_assistant_message(&parsed);

        Ok(ProviderResponse {
            output_text: parsed.output_text.clone(),
            tool_calls: tool_calls_from_single(&parsed.tool_call),
            tool_call: parsed.tool_call,
            reasoning_content: parsed.reasoning_content.clone(),
            reasoning_content_value: parsed.reasoning_content_value.clone(),
            assistant_message: Some(assistant_message),
            provider_source: "provider_followup_sync".to_string(),
            provider_mode: "live".to_string(),
            fallback_reason: None,
            token_usage: Some(token_usage),
        })
    }

    /// D2：Anthropic 的批量回填（1 assistant 含全部 tool_use + 1 user 含 N 个 tool_result），
    /// 复用 `post_anthropic_json` 与既有解析尾巴。
    #[allow(clippy::too_many_arguments)]
    fn send_anthropic_tool_followup_batch_request(
        &self,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
        accumulated_messages: &mut Vec<Value>,
        assistant_message: Option<&Value>,
        calls: &[ToolCall],
        results: &[ToolResult],
    ) -> Result<ProviderResponse, String> {
        let endpoint = format!("{}/messages", self.config.base_url.trim_end_matches('/'));
        provider_log(format!(
            "request:anthropic followup-sync-batch endpoint={} model={} calls={}",
            endpoint,
            request.model,
            calls.len()
        ));
        let messages = anthropic_followup_messages_for_tool_results(
            request,
            accumulated_messages,
            assistant_message,
            calls,
            results,
        )?;
        let body = with_anthropic_request_options(
            json!({
                "model": request.model,
                "system": anthropic_system_text(&request.input),
                "messages": messages,
                "temperature": request.temperature,
                "max_tokens": request.max_output_tokens,
                "tools": anthropic_tools_payload(tools),
                "tool_choice": {
                    "type": "auto",
                    "disable_parallel_tool_use": true
                }
            }),
            &self.config,
        );
        let body = apply_anthropic_tool_capability(body, tools, &self.config);
        provider_log(format!(
            "request:anthropic followup-sync-batch messages={} body_preview={}",
            body.get("messages")
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or(0),
            preview_json(&body, 1600)
        ));
        // 与 `send_anthropic_tool_followup_request` **同一** post 通道与解析尾巴。
        let payload = self.post_anthropic_json(&endpoint, &body)?;

        let output_text = extract_anthropic_output_text(&payload).unwrap_or_default();
        let content = payload
            .get("content")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let reasoning_content = extract_anthropic_thinking_text(&content);
        let reasoning_content_value = extract_anthropic_thinking_value(&content);
        // D3b：多 `tool_use` 不再拒绝——整批上报，由运行时批执行消费（单值字段仍是首元素
        // 视图，D1a 不变量；N 个调用的回填形态见 D1b 批量构造器）。
        let tool_calls = extract_anthropic_tool_calls(&content);
        let tool_call = tool_calls.first().cloned();
        if output_text.trim().is_empty() && tool_call.is_none() {
            return Err("anthropic tool follow-up missing text or tool call".to_string());
        }
        let assistant_message = Some(anthropic_native_assistant_message(&content));

        Ok(ProviderResponse {
            output_text: output_text.clone(),
            tool_calls,
            tool_call,
            reasoning_content,
            reasoning_content_value,
            assistant_message,
            provider_source: "provider_followup_sync".to_string(),
            provider_mode: "live".to_string(),
            fallback_reason: None,
            token_usage: Some(
                extract_anthropic_usage(&payload)
                    .unwrap_or_else(|| estimate_token_usage(request, &output_text)),
            ),
        })
    }

    fn send_openai_tool_followup_request(
        &self,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
        accumulated_messages: &mut Vec<Value>,
        assistant_message: Option<&Value>,
        tool_call: &ToolCall,
        tool_result: &ToolResult,
    ) -> Result<ProviderResponse, String> {
        let endpoint = format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        );
        provider_log(format!(
            "request:openai followup-sync endpoint={} model={} tool={}",
            endpoint, request.model, tool_call.name
        ));
        let messages = normalize_openai_messages(openai_messages_with_tool_result(
            request,
            accumulated_messages,
            assistant_message,
            tool_call,
            tool_result,
        ));
        {
            let tool_call_id = tool_call
                .call_id
                .clone()
                .unwrap_or_else(|| "tool_call_local".to_string());
            provider_log(format!(
                "request:openai followup-sync tool_call_id={} messages={}",
                tool_call_id,
                messages.len(),
            ));
        }
        let body = with_openai_request_options(
            json!({
                "model": request.model,
                "messages": messages,
                "temperature": request.temperature,
                "max_tokens": request.max_output_tokens,
                "stream": false,
                "tool_choice": "auto",
                "tools": openai_tools_payload(tools)
            }),
            &self.config,
        );
        let body = apply_openai_tool_capability(body, tools, &self.config);
        provider_log(format!(
            "request:openai followup-sync body_preview={}",
            preview_json(&body, 1600)
        ));
        let payload = self.post_openai_json(&endpoint, &body)?;
        let message = first_openai_message(&payload)?;
        let output_text = extract_openai_message_text(message).unwrap_or_default();
        // D3a：非流式入口**不再丢弃**多余调用（整批上报，运行时批执行）。
        let tool_calls = extract_openai_tool_calls(message);
        // 单值字段继续走既有单数入口（= 复数版首元素的兼容包装，行为逐字不变）。
        let tool_call = extract_openai_tool_call(message);
        if output_text.trim().is_empty() && tool_call.is_none() {
            return Err("openai tool follow-up missing text or tool call".to_string());
        }

        let token_usage = extract_openai_usage(&payload)
            .unwrap_or_else(|| estimate_token_usage(request, &output_text));
        provider_log_token_usage("openai followup-sync", &token_usage);

        Ok(ProviderResponse {
            output_text: output_text.clone(),
            tool_calls,
            tool_call,
            reasoning_content: extract_openai_message_reasoning_content(message),
            reasoning_content_value: extract_openai_message_reasoning_value(message),
            assistant_message: Some(normalize_openai_assistant_message(message)),
            provider_source: "provider_followup_sync".to_string(),
            provider_mode: "live".to_string(),
            fallback_reason: None,
            token_usage: Some(token_usage),
        })
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "沿用 provider tool followup 既有签名形状"
    )]
    fn send_openai_tool_followup_stream_request<F>(
        &self,
        guard: &ProviderStreamGuard<'_>,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
        accumulated_messages: &mut Vec<Value>,
        assistant_message: Option<&Value>,
        calls: &[ToolCall],
        results: &[ToolResult],
        on_delta: &mut F,
    ) -> Result<ProviderResponse, String>
    where
        F: FnMut(ProviderStreamChunk),
    {
        let endpoint = format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        );
        if calls.len() == 1 {
            provider_log(format!(
                "request:openai followup-stream endpoint={} model={} tool={}",
                endpoint, request.model, calls[0].name
            ));
        } else {
            provider_log(format!(
                "request:openai followup-stream endpoint={} model={} calls={}",
                endpoint,
                request.model,
                calls.len()
            ));
        }
        let messages = build_openai_followup_stream_messages(
            request,
            accumulated_messages,
            assistant_message,
            calls,
            results,
        )?;
        let tool_call_ids = calls
            .iter()
            .map(|call| {
                call.call_id
                    .clone()
                    .unwrap_or_else(|| "tool_call_local".to_string())
            })
            .collect::<Vec<_>>()
            .join(",");
        if calls.len() == 1 {
            provider_log(format!(
                "request:openai followup-stream tool_call_id={} messages={}",
                tool_call_ids,
                messages.len(),
            ));
        } else {
            provider_log(format!(
                "request:openai followup-stream tool_call_ids={} messages={}",
                tool_call_ids,
                messages.len(),
            ));
        }
        let body = with_openai_request_options(
            json!({
                "model": request.model,
                "messages": messages,
                "temperature": request.temperature,
                "max_tokens": request.max_output_tokens,
                "stream": true,
                "stream_options": {
                    "include_usage": true
                },
                "tool_choice": "auto",
                "tools": openai_tools_payload(tools)
            }),
            &self.config,
        );
        let body = apply_openai_tool_capability(body, tools, &self.config);
        provider_log(format!(
            "request:openai followup-stream body_preview={}",
            preview_json(&body, 1600)
        ));

        self.stream_openai_request(guard, &endpoint, &body, request, on_delta)
    }

    fn send_anthropic_tool_followup_request(
        &self,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
        accumulated_messages: &mut Vec<Value>,
        assistant_message: Option<&Value>,
        tool_call: &ToolCall,
        tool_result: &ToolResult,
    ) -> Result<ProviderResponse, String> {
        let endpoint = format!("{}/messages", self.config.base_url.trim_end_matches('/'));
        provider_log(format!(
            "request:anthropic followup-sync endpoint={} model={} tool={}",
            endpoint, request.model, tool_call.name
        ));
        let body = with_anthropic_request_options(
            json!({
                "model": request.model,
                "system": anthropic_system_text(&request.input),
                "messages": anthropic_messages_with_tool_result(request, accumulated_messages, assistant_message, tool_call, tool_result),
                "temperature": request.temperature,
                "max_tokens": request.max_output_tokens,
                "tools": anthropic_tools_payload(tools),
                "tool_choice": {
                    "type": "auto",
                    "disable_parallel_tool_use": true
                }
            }),
            &self.config,
        );
        let body = apply_anthropic_tool_capability(body, tools, &self.config);
        provider_log(format!(
            "request:anthropic followup-sync messages={} body_preview={}",
            body.get("messages")
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or(0),
            preview_json(&body, 1600)
        ));
        let payload = self.post_anthropic_json(&endpoint, &body)?;

        let output_text = extract_anthropic_output_text(&payload).unwrap_or_default();
        let content = payload
            .get("content")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let reasoning_content = extract_anthropic_thinking_text(&content);
        let reasoning_content_value = extract_anthropic_thinking_value(&content);
        // D3b：多 `tool_use` 不再拒绝——整批上报，由运行时批执行消费（单值字段仍是首元素
        // 视图，D1a 不变量；N 个调用的回填形态见 D1b 批量构造器）。
        let tool_calls = extract_anthropic_tool_calls(&content);
        let tool_call = tool_calls.first().cloned();
        if output_text.trim().is_empty() && tool_call.is_none() {
            return Err("anthropic tool follow-up missing text or tool call".to_string());
        }
        let assistant_message = Some(anthropic_native_assistant_message(&content));

        Ok(ProviderResponse {
            output_text: output_text.clone(),
            tool_calls,
            tool_call,
            reasoning_content,
            reasoning_content_value,
            assistant_message,
            provider_source: "provider_followup_sync".to_string(),
            provider_mode: "live".to_string(),
            fallback_reason: None,
            token_usage: Some(
                extract_anthropic_usage(&payload)
                    .unwrap_or_else(|| estimate_token_usage(request, &output_text)),
            ),
        })
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "沿用 provider tool followup 既有签名形状"
    )]
    fn send_anthropic_tool_followup_stream_request<F>(
        &self,
        guard: &ProviderStreamGuard<'_>,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
        accumulated_messages: &mut Vec<Value>,
        assistant_message: Option<&Value>,
        calls: &[ToolCall],
        results: &[ToolResult],
        on_delta: &mut F,
    ) -> Result<ProviderResponse, String>
    where
        F: FnMut(ProviderStreamChunk),
    {
        let endpoint = format!("{}/messages", self.config.base_url.trim_end_matches('/'));
        if calls.len() == 1 {
            provider_log(format!(
                "request:anthropic followup-stream endpoint={} model={} tool={}",
                endpoint, request.model, calls[0].name
            ));
        } else {
            provider_log(format!(
                "request:anthropic followup-stream endpoint={} model={} calls={}",
                endpoint,
                request.model,
                calls.len()
            ));
        }
        let messages = if calls.len() == 1 && results.len() == 1 {
            anthropic_messages_with_tool_result(
                request,
                accumulated_messages,
                assistant_message,
                &calls[0],
                &results[0],
            )
        } else {
            anthropic_followup_messages_for_tool_results(
                request,
                accumulated_messages,
                assistant_message,
                calls,
                results,
            )?
        };
        let body = with_anthropic_request_options(
            json!({
                "model": request.model,
                "system": anthropic_system_text(&request.input),
                "messages": messages,
                "temperature": request.temperature,
                "max_tokens": request.max_output_tokens,
                "stream": true,
                "tools": anthropic_tools_payload(tools),
                "tool_choice": {
                    "type": "auto",
                    "disable_parallel_tool_use": true
                }
            }),
            &self.config,
        );
        let body = apply_anthropic_tool_capability(body, tools, &self.config);
        provider_log(format!(
            "request:anthropic followup-stream messages={} body_preview={}",
            body.get("messages")
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or(0),
            preview_json(&body, 1600)
        ));

        self.stream_anthropic_request(guard, &endpoint, &body, request, on_delta)
    }

    // ------------------------------------------------------------------
    // Responses API（protocol = openai-responses）四路分发实现。
    // assistant_message 统一产出 chat 形态（架构裁决 R1）；请求边界做
    // chat→responses 转换；鉴权沿用 Bearer（post_openai_*）。
    // ------------------------------------------------------------------

    fn responses_endpoint(&self) -> String {
        format!("{}/responses", self.config.base_url.trim_end_matches('/'))
    }

    fn send_responses_tool_decision_request(
        &self,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
    ) -> Result<ProviderDecision, String> {
        let endpoint = self.responses_endpoint();
        provider_log(format!(
            "request:responses decision endpoint={} model={}",
            endpoint, request.model
        ));
        let body = build_responses_request_body(request, tools, &self.config, false);
        provider_log(format!(
            "request:responses decision body_preview={}",
            preview_json(&body, 1200)
        ));
        let payload = self.post_openai_json(&endpoint, &body)?;
        let parsed = parse_responses_output(&payload)?;
        if parsed.dropped_function_calls > 0 {
            provider_log(format!(
                "decision:responses dropped_function_calls={}",
                parsed.dropped_function_calls
            ));
        }
        let token_usage = parsed
            .token_usage
            .clone()
            .unwrap_or_else(|| estimate_token_usage(request, &parsed.output_text));
        provider_log_token_usage("responses decision", &token_usage);

        Ok(ProviderDecision {
            output_text: parsed.output_text.clone(),
            tool_calls: tool_calls_from_single(&parsed.tool_call),
            tool_call: parsed.tool_call.clone(),
            reasoning_content: parsed.reasoning_content.clone(),
            reasoning_content_value: parsed.reasoning_content_value.clone(),
            assistant_message: Some(responses_chat_assistant_message(&parsed)),
            provider_source: "provider_decision".to_string(),
            provider_mode: "live".to_string(),
            fallback_reason: None,
            token_usage: Some(token_usage),
        })
    }

    fn send_responses_tool_decision_stream_request<F>(
        &self,
        guard: &ProviderStreamGuard<'_>,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
        on_delta: &mut F,
    ) -> Result<ProviderDecision, String>
    where
        F: FnMut(ProviderStreamChunk),
    {
        let endpoint = self.responses_endpoint();
        provider_log(format!(
            "request:responses decision-stream endpoint={} model={}",
            endpoint, request.model
        ));
        let body = build_responses_request_body(request, tools, &self.config, true);
        provider_log(format!(
            "request:responses decision-stream body_preview={}",
            preview_json(&body, 1200)
        ));

        let response = self.post_openai_stream_response(&endpoint, &body)?;
        let message = collect_responses_sse_message_from_response(
            response,
            Instant::now(),
            &endpoint,
            on_delta,
            guard,
        )?;
        finalize_responses_stream_message(message, "decision", request).map(
            |(output_text, tool_call, reasoning_content, reasoning_content_value, token_usage)| {
                let assistant_message = match tool_call.as_ref() {
                    Some(call) => provider_native_assistant_tool_call_message_with_reasoning_value(
                        text_if_present(&output_text),
                        reasoning_content_value.as_ref(),
                        call,
                    ),
                    None => provider_native_assistant_message_with_reasoning_value(
                        &output_text,
                        reasoning_content_value.as_ref(),
                    ),
                };
                ProviderDecision {
                    output_text: output_text.clone(),
                    tool_calls: tool_calls_from_single(&tool_call),
                    tool_call,
                    reasoning_content: reasoning_content.clone(),
                    reasoning_content_value: reasoning_content_value.clone(),
                    assistant_message: Some(assistant_message),
                    provider_source: "provider_decision_stream".to_string(),
                    provider_mode: "live".to_string(),
                    fallback_reason: None,
                    token_usage: Some(token_usage),
                }
            },
        )
    }

    fn send_responses_tool_followup_request(
        &self,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
        accumulated_messages: &mut Vec<Value>,
        assistant_message: Option<&Value>,
        tool_call: &ToolCall,
        tool_result: &ToolResult,
    ) -> Result<ProviderResponse, String> {
        let endpoint = self.responses_endpoint();
        provider_log(format!(
            "request:responses followup-sync endpoint={} model={} tool={}",
            endpoint, request.model, tool_call.name
        ));
        // 与 completions 相同的 chat 形态中枢：累积转录 + assistant + tool result，
        // 转换到 Responses input 只发生在请求边界（build_responses_request_body）。
        let messages = openai_messages_with_tool_result(
            request,
            accumulated_messages,
            assistant_message,
            tool_call,
            tool_result,
        );
        {
            let call_id = tool_call
                .call_id
                .clone()
                .unwrap_or_else(|| "tool_call_local".to_string());
            provider_log(format!(
                "request:responses followup-sync tool_call_id={} messages={}",
                call_id,
                messages.len(),
            ));
        }
        let mut conversion_request = request.clone();
        conversion_request.native_messages = messages;
        let body = build_responses_request_body(&conversion_request, tools, &self.config, false);
        provider_log(format!(
            "request:responses followup-sync body_preview={}",
            preview_json(&body, 1600)
        ));
        let payload = self.post_openai_json(&endpoint, &body)?;
        let parsed = parse_responses_output(&payload)?;
        if parsed.dropped_function_calls > 0 {
            provider_log(format!(
                "followup:responses dropped_function_calls={}",
                parsed.dropped_function_calls
            ));
        }
        let token_usage = parsed
            .token_usage
            .clone()
            .unwrap_or_else(|| estimate_token_usage(request, &parsed.output_text));
        provider_log_token_usage("responses followup-sync", &token_usage);

        let assistant_message = responses_chat_assistant_message(&parsed);

        Ok(ProviderResponse {
            output_text: parsed.output_text.clone(),
            tool_calls: tool_calls_from_single(&parsed.tool_call),
            tool_call: parsed.tool_call,
            reasoning_content: parsed.reasoning_content.clone(),
            reasoning_content_value: parsed.reasoning_content_value.clone(),
            assistant_message: Some(assistant_message),
            provider_source: "provider_followup_sync".to_string(),
            provider_mode: "live".to_string(),
            fallback_reason: None,
            token_usage: Some(token_usage),
        })
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "沿用 provider tool followup 既有签名形状"
    )]
    fn send_responses_tool_followup_stream_request<F>(
        &self,
        guard: &ProviderStreamGuard<'_>,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
        accumulated_messages: &mut Vec<Value>,
        assistant_message: Option<&Value>,
        calls: &[ToolCall],
        results: &[ToolResult],
        on_delta: &mut F,
    ) -> Result<ProviderResponse, String>
    where
        F: FnMut(ProviderStreamChunk),
    {
        let endpoint = self.responses_endpoint();
        if calls.len() == 1 {
            provider_log(format!(
                "request:responses followup-stream endpoint={} model={} tool={}",
                endpoint, request.model, calls[0].name
            ));
        } else {
            provider_log(format!(
                "request:responses followup-stream endpoint={} model={} calls={}",
                endpoint,
                request.model,
                calls.len()
            ));
        }
        let messages = build_openai_followup_stream_messages(
            request,
            accumulated_messages,
            assistant_message,
            calls,
            results,
        )?;
        let mut conversion_request = request.clone();
        conversion_request.native_messages = messages;
        let body = build_responses_request_body(&conversion_request, tools, &self.config, true);

        let response = self.post_openai_stream_response(&endpoint, &body)?;
        let message = collect_responses_sse_message_from_response(
            response,
            Instant::now(),
            &endpoint,
            on_delta,
            guard,
        )?;
        finalize_responses_stream_message(message, "followup", request).map(
            |(output_text, tool_call, reasoning_content, reasoning_content_value, token_usage)| {
                let tool_calls = tool_calls_from_single(&tool_call);
                let assistant_message = build_openai_stream_assistant_message(
                    &output_text,
                    reasoning_content_value.as_ref(),
                    tool_call.as_ref(),
                    &tool_calls,
                );
                ProviderResponse {
                    output_text,
                    tool_calls,
                    tool_call,
                    reasoning_content,
                    reasoning_content_value,
                    assistant_message: Some(assistant_message),
                    provider_source: "provider_followup_stream".to_string(),
                    provider_mode: "live".to_string(),
                    fallback_reason: None,
                    token_usage: Some(token_usage),
                }
            },
        )
    }

    fn stream_openai_request<F>(
        &self,
        guard: &ProviderStreamGuard<'_>,
        endpoint: &str,
        body: &Value,
        request: &ProviderRequest,
        on_delta: &mut F,
    ) -> Result<ProviderResponse, String>
    where
        F: FnMut(ProviderStreamChunk),
    {
        let response = self.post_openai_stream_response(endpoint, body)?;
        let message =
            collect_openai_sse_message_from_response(
                response,
                Instant::now(),
                endpoint,
                on_delta,
                guard,
            )?;
        let output_text = message.output_text.clone();
        let token_usage = message
            .token_usage
            .clone()
            .unwrap_or_else(|| estimate_token_usage(request, &output_text));
        provider_log_token_usage("openai followup-stream", &token_usage);
        let assistant_message = Some(build_openai_stream_assistant_message(
            &output_text,
            message.reasoning_content_value.as_ref(),
            message.tool_call.as_ref(),
            &message.tool_calls,
        ));

        Ok(ProviderResponse {
            output_text: output_text.clone(),
            // D3b：流式响应里的**整批**调用全量上报（单值仍是首元素视图）。
            tool_calls: message.tool_calls.clone(),
            tool_call: message.tool_call,
            reasoning_content: message.reasoning_content.clone(),
            reasoning_content_value: message.reasoning_content_value.clone(),
            assistant_message,
            provider_source: "provider_followup_stream".to_string(),
            provider_mode: "live".to_string(),
            fallback_reason: None,
            token_usage: Some(token_usage),
        })
    }

    fn post_openai_stream_response(
        &self,
        endpoint: &str,
        body: &Value,
    ) -> Result<reqwest::Response, String> {
        // PA-171 C3-P1：冷却期在发送前短路（0 次 HTTP）。
        self.ensure_not_cooling()?;
        // F7：15s 建连预算 + 180s **读空闲**预算；不设总 deadline，长推理流不再被 180s 截断。
        let client = build_streaming_http_client(
            Duration::from_secs(15),
            Duration::from_secs(180),
            endpoint,
        )?;
        let started_at = Instant::now();

        let request = client.post(endpoint).bearer_auth(
            self.config
                .api_key
                .as_deref()
                .ok_or_else(|| "provider 缺少 API Key".to_string())?,
        );
        // Wave-4 C3：outbound 级联注入 traceparent（无活跃 turn 时原样透传，零改动）。
        let request = pony_observability::otel::with_current_traceparent(request);
        // B1795506800002：请求体字节闸门（0 HTTP Fail-Closed）。序列化一次并复用，
        // 与 `.json(body)` 逐字节等价；超限则在此返回，不发任何网络包。
        let request_body_bytes = serialize_request_body_with_size_gate(body)?;
        let response = block_on(async move {
            request
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(request_body_bytes)
                .send()
                .await
        }).map_err(|error| {
            format_request_error("调用 provider 流式接口失败", &error, started_at.elapsed())
        })?;

        let status = response.status();
        if !status.is_success() {
            // PA-171 C3-P1：`Retry-After` 在 `text()` 之前读取。
            let retry_after = response_retry_after_header(&response);
            let text = block_on(async move { response.text().await }).map_err(|error| {
                format_request_error("读取 provider 流式返回失败", &error, started_at.elapsed())
            })?;

            let mut error = provider_status_error(
                status,
                started_at.elapsed().as_millis(),
                &text,
                retry_after.as_deref(),
            );
            // L3-B ADVISORY：冷却登记成功 ⇒ 打标；重试编排见标记即**全 scope** 立即 Abort
            // （不 sleep、不再发 HTTP），保证「冷却期内 0 次 HTTP」。
            if self.enter_cooldown(&error) {
                error = crate::retry::append_cooldown_active_marker(&error);
            }
            return Err(error);
        }

        Ok(response)
    }

    fn post_anthropic_stream_response(
        &self,
        endpoint: &str,
        body: &Value,
    ) -> Result<reqwest::Response, String> {
        // PA-171 C3-P1：冷却期在发送前短路（0 次 HTTP）。
        self.ensure_not_cooling()?;
        // F7：15s 建连预算 + 180s **读空闲**预算；不设总 deadline，长推理流不再被 180s 截断。
        let client = build_streaming_http_client(
            Duration::from_secs(15),
            Duration::from_secs(180),
            endpoint,
        )?;
        let started_at = Instant::now();

        let api_key = self
            .config
            .api_key
            .as_deref()
            .ok_or_else(|| "provider 缺少 API Key".to_string())?;

        let mut request = client.post(endpoint);
        match resolve_anthropic_auth_type(&self.config) {
            ProviderAuthType::Bearer => {
                request = request.bearer_auth(api_key);
            }
            ProviderAuthType::XApiKey | ProviderAuthType::Auto => {
                request = request
                    .header("x-api-key", api_key)
                    .header("anthropic-version", "2023-06-01");
            }
        }
        // Wave-4 C3：outbound 级联注入 traceparent。
        let request = pony_observability::otel::with_current_traceparent(request);
        // B1795506800002：请求体字节闸门（0 HTTP Fail-Closed）。序列化一次并复用，
        // 与 `.json(body)` 逐字节等价；超限则在此返回，不发任何网络包。
        let request_body_bytes = serialize_request_body_with_size_gate(body)?;
        let response = block_on(async move {
            request
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(request_body_bytes)
                .send()
                .await
        }).map_err(|error| {
            format_request_error("调用 provider 流式接口失败", &error, started_at.elapsed())
        })?;

        let status = response.status();
        if !status.is_success() {
            // PA-171 C3-P1：`Retry-After` 在 `text()` 之前读取。
            let retry_after = response_retry_after_header(&response);
            let text = block_on(async move { response.text().await }).map_err(|error| {
                format_request_error("读取 provider 流式返回失败", &error, started_at.elapsed())
            })?;

            let mut error = provider_status_error(
                status,
                started_at.elapsed().as_millis(),
                &text,
                retry_after.as_deref(),
            );
            // L3-B ADVISORY：冷却登记成功 ⇒ 打标；重试编排见标记即**全 scope** 立即 Abort
            // （不 sleep、不再发 HTTP），保证「冷却期内 0 次 HTTP」。
            if self.enter_cooldown(&error) {
                error = crate::retry::append_cooldown_active_marker(&error);
            }
            return Err(error);
        }

        Ok(response)
    }

    fn stream_anthropic_request<F>(
        &self,
        guard: &ProviderStreamGuard<'_>,
        endpoint: &str,
        body: &Value,
        request: &ProviderRequest,
        on_delta: &mut F,
    ) -> Result<ProviderResponse, String>
    where
        F: FnMut(ProviderStreamChunk),
    {
        let started_at = Instant::now();
        let response = self.post_anthropic_stream_response(endpoint, body)?;
        let message =
            collect_anthropic_sse_message_from_response(
                response,
                started_at,
                endpoint,
                on_delta,
                guard,
            )?;

        let output_text = message.output_text;
        let reasoning_content = message.reasoning_content;
        let reasoning_content_value = message.reasoning_content_value;
        let tool_call = message.tool_call;

        if output_text.trim().is_empty() && tool_call.is_none() {
            return Err("anthropic stream follow-up missing text or tool call".to_string());
        }

        let assistant_message = match tool_call.as_ref() {
            Some(tool_call) => Some(provider_native_assistant_tool_call_message_for_protocol(
                &ProviderProtocol::AnthropicMessages,
                text_if_present(&output_text),
                reasoning_content_value.as_ref(),
                tool_call,
            )),
            None => Some(anthropic_native_assistant_message_from_parts(
                &output_text,
                reasoning_content_value.as_ref(),
            )),
        };

        Ok(ProviderResponse {
            output_text: output_text.clone(),
            tool_calls: tool_calls_from_single(&tool_call),
            tool_call,
            reasoning_content,
            reasoning_content_value,
            assistant_message,
            provider_source: "provider_followup_stream".to_string(),
            provider_mode: "live".to_string(),
            fallback_reason: None,
            token_usage: Some(
                message
                    .token_usage
                    .unwrap_or_else(|| estimate_token_usage(request, &output_text)),
            ),
        })
    }

    fn send_openai_tool_decision_request(
        &self,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
    ) -> Result<ProviderDecision, String> {
        let endpoint = format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        );
        provider_log(format!(
            "request:openai decision endpoint={} model={} safe_tools={}",
            endpoint,
            request.model,
            provider_tool_contract_views(tools)
                .iter()
                .map(|tool| openai_safe_tool_name(&tool.name))
                .collect::<Vec<_>>()
                .join(",")
        ));
        let messages = normalize_openai_messages(openai_request_messages(request));
        let body = with_openai_request_options(
            json!({
                "model": request.model,
                "messages": messages,
                "temperature": request.temperature,
                "max_tokens": request.max_output_tokens,
                "stream": false,
                "tool_choice": "auto",
                "tools": openai_tools_payload(tools)
            }),
            &self.config,
        );
        let body = apply_openai_tool_capability(body, tools, &self.config);
        provider_log(format!(
            "request:openai decision messages={} body_chars={} body_preview={}",
            request
                .input
                .iter()
                .map(|message| format!("{:?}:{}", message.role, preview_text(&message.content, 48)))
                .collect::<Vec<_>>()
                .join(" | "),
            body.to_string().chars().count(),
            preview_json(&body, 900)
        ));
        let payload = self.post_openai_json(&endpoint, &body)?;
        let message = first_openai_message(&payload)?;

        let token_usage = extract_openai_usage(&payload).unwrap_or_else(|| {
            estimate_token_usage(
                request,
                extract_openai_message_text(message)
                    .as_deref()
                    .unwrap_or_default(),
            )
        });
        provider_log_token_usage("openai decision", &token_usage);

        // D3a：非流式决策**不再丢弃**多余调用——整批 `tool_calls` 全量上报，由运行时批执行。
        let decision_tool_calls = extract_openai_tool_calls(message);
        // 单值字段继续走既有单数入口（= 复数版首元素的兼容包装，行为逐字不变）。
        let decision_tool_call = extract_openai_tool_call(message);

        Ok(ProviderDecision {
            output_text: extract_openai_message_text(message).unwrap_or_default(),
            tool_calls: decision_tool_calls,
            tool_call: decision_tool_call,
            reasoning_content: extract_openai_message_reasoning_content(message),
            reasoning_content_value: extract_openai_message_reasoning_value(message),
            assistant_message: Some(normalize_openai_assistant_message(message)),
            provider_source: "provider_decision".to_string(),
            provider_mode: "live".to_string(),
            fallback_reason: None,
            token_usage: Some(token_usage),
        })
    }

    fn send_openai_tool_decision_stream_request<F>(
        &self,
        guard: &ProviderStreamGuard<'_>,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
        on_delta: &mut F,
    ) -> Result<ProviderDecision, String>
    where
        F: FnMut(ProviderStreamChunk),
    {
        let endpoint = format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        );
        provider_log(format!(
            "request:openai decision-stream endpoint={} model={} safe_tools={}",
            endpoint,
            request.model,
            provider_tool_contract_views(tools)
                .iter()
                .map(|tool| openai_safe_tool_name(&tool.name))
                .collect::<Vec<_>>()
                .join(",")
        ));
        let messages = normalize_openai_messages(openai_request_messages(request));
        let body = with_openai_request_options(
            json!({
                "model": request.model,
                "messages": messages,
                "temperature": request.temperature,
                "max_tokens": request.max_output_tokens,
                "stream": true,
                "stream_options": {
                    "include_usage": true
                },
                "tool_choice": "auto",
                "tools": openai_tools_payload(tools)
            }),
            &self.config,
        );
        let body = apply_openai_tool_capability(body, tools, &self.config);
        provider_log(format!(
            "request:openai decision-stream messages={} body_chars={} body_preview={}",
            request
                .input
                .iter()
                .map(|message| format!("{:?}:{}", message.role, preview_text(&message.content, 48)))
                .collect::<Vec<_>>()
                .join(" | "),
            body.to_string().chars().count(),
            preview_json(&body, 900)
        ));

        let response = self.post_openai_stream_response(&endpoint, &body)?;
        let message = collect_openai_sse_message_from_response(
            response,
            Instant::now(),
            &endpoint,
            on_delta,
            guard,
        )?;
        let output_text = message.output_text.clone();
        let tool_call = message.tool_call.clone();
        // D3b：决策流也可能一次携带多个 `tool_calls`（并行调用），整批上报给运行时批执行。
        let tool_calls = message.tool_calls.clone();
        if let Some(ref tc) = tool_call {
            provider_log(format!(
                "decision:stream-tool-call name={} call_id={} batch={}",
                tc.name,
                tc.call_id.as_deref().unwrap_or("(none)"),
                tool_calls.len()
            ));
        }
        if output_text.trim().is_empty() && tool_call.is_none() {
            return Err("openai streamed decision missing text or tool call".to_string());
        }
        let token_usage = message
            .token_usage
            .clone()
            .unwrap_or_else(|| estimate_token_usage(request, &output_text));
        provider_log_token_usage("openai decision-stream", &token_usage);

        let assistant_message = Some(build_openai_stream_assistant_message(
            &output_text,
            message.reasoning_content_value.as_ref(),
            tool_call.as_ref(),
            &tool_calls,
        ));

        Ok(ProviderDecision {
            output_text,
            tool_calls,
            tool_call,
            reasoning_content: message.reasoning_content,
            reasoning_content_value: message.reasoning_content_value,
            assistant_message,
            provider_source: "provider_decision_stream".to_string(),
            provider_mode: "live".to_string(),
            fallback_reason: None,
            token_usage: Some(token_usage),
        })
    }

    fn send_anthropic_tool_decision_request(
        &self,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
    ) -> Result<ProviderDecision, String> {
        let endpoint = format!("{}/messages", self.config.base_url.trim_end_matches('/'));
        provider_log(format!(
            "request:anthropic decision endpoint={} model={} tools={}",
            endpoint,
            request.model,
            tools
                .iter()
                .map(|tool| tool.name)
                .collect::<Vec<_>>()
                .join(",")
        ));
        let body = with_anthropic_request_options(
            json!({
                "model": request.model,
                "system": anthropic_system_text(&request.input),
                "messages": anthropic_conversation_messages(request),
                "temperature": request.temperature,
                "max_tokens": request.max_output_tokens,
                "tools": anthropic_tools_payload(tools),
                "tool_choice": {
                    "type": "auto",
                    "disable_parallel_tool_use": true
                }
            }),
            &self.config,
        );
        let body = apply_anthropic_tool_capability(body, tools, &self.config);
        provider_log(format!(
            "request:anthropic decision messages={} body_chars={} body_preview={}",
            request
                .input
                .iter()
                .map(|message| format!("{:?}:{}", message.role, preview_text(&message.content, 48)))
                .collect::<Vec<_>>()
                .join(" | "),
            body.to_string().chars().count(),
            preview_json(&body, 900)
        ));
        let payload = self.post_anthropic_json(&endpoint, &body)?;
        let content = payload
            .get("content")
            .and_then(Value::as_array)
            .ok_or_else(|| "anthropic 返回中缺少 content".to_string())?;

        let output_text = extract_anthropic_text_blocks(content);
        let reasoning_content = extract_anthropic_thinking_text(content);
        let reasoning_content_value = extract_anthropic_thinking_value(content);
        // D3b：多 `tool_use` 不再拒绝（批量执行已落地）——整批上报，单值是首元素视图。
        Ok(ProviderDecision {
            output_text: output_text.clone(),
            tool_calls: extract_anthropic_tool_calls(content),
            tool_call: extract_anthropic_tool_call(content),
            reasoning_content,
            reasoning_content_value: reasoning_content_value.clone(),
            assistant_message: Some(anthropic_native_assistant_message(content)),
            provider_source: "provider_decision".to_string(),
            provider_mode: "live".to_string(),
            fallback_reason: None,
            token_usage: Some(
                extract_anthropic_usage(&payload)
                    .unwrap_or_else(|| estimate_token_usage(request, &output_text)),
            ),
        })
    }

    /// PA-174 B2：`post_openai_json` 的收口包装——真实失败逐次记录（合成预算错误除外），
    /// 预算耗尽时由 [`Self::budget_exhausted_error`] 交付最后一次真实错误。
    fn post_openai_json(&self, endpoint: &str, body: &Value) -> Result<Value, String> {
        let result = self.post_openai_json_impl(endpoint, body);
        if let Err(error) = &result {
            self.record_provider_error(error);
        }
        result
    }

    fn post_openai_json_impl(&self, endpoint: &str, body: &Value) -> Result<Value, String> {
        // PA-171 C3-P1：冷却期在发送前短路（0 次 HTTP）。
        self.ensure_not_cooling()?;
        // PA-172 §2.1/§3.3：单次 attempt 总超时配置化（默认 45s，与基线逐字节等价）；
        // 换手候选再按**逐 attempt deadline** 收紧（剩余为 0 ⇒ 立即 Abort，0 次 HTTP）。
        let attempt_timeout_ms = self.effective_attempt_timeout_ms();
        if attempt_timeout_ms == 0 {
            return Err(self.budget_exhausted_error());
        }
        let client = build_http_client(Duration::from_millis(attempt_timeout_ms), endpoint)?;
        let started_at = Instant::now();

        let request = client.post(endpoint).bearer_auth(
            self.config
                .api_key
                .as_deref()
                .ok_or_else(|| "provider 缺少 API Key".to_string())?,
        );
        // Wave-4 C3：outbound 级联注入 traceparent（无活跃 turn 时原样透传，零改动）。
        let request = pony_observability::otel::with_current_traceparent(request);
        // B1795506800002：请求体字节闸门（0 HTTP Fail-Closed）。序列化一次并复用，
        // 与 `.json(body)` 逐字节等价；超限则在此返回，不发任何网络包。
        let request_body_bytes = serialize_request_body_with_size_gate(body)?;
        let response = block_on(async move {
            request
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(request_body_bytes)
                .send()
                .await
        })
        .map_err(|error| {
            format_request_error("调用 provider 失败", &error, started_at.elapsed())
        })?;

        let status = response.status();
        // PA-171 C3-P1：`Retry-After` 在 `text()` 之前读取。
        let retry_after = response_retry_after_header(&response);
        let text = block_on(async move { response.text().await }).map_err(|error| {
            format_request_error("读取 provider 返回失败", &error, started_at.elapsed())
        })?;

        if !status.is_success() {
            let mut error = provider_status_error(
                status,
                started_at.elapsed().as_millis(),
                &text,
                retry_after.as_deref(),
            );
            // L3-B ADVISORY：冷却登记成功 ⇒ 打标；重试编排见标记即**全 scope** 立即 Abort
            // （不 sleep、不再发 HTTP），保证「冷却期内 0 次 HTTP」。
            if self.enter_cooldown(&error) {
                error = crate::retry::append_cooldown_active_marker(&error);
            }
            return Err(error);
        }

        serde_json::from_str::<Value>(&text).map_err(|error| {
            format!(
                "解析 provider 返回失败：{}；耗时={}ms；原始响应：{}",
                error,
                started_at.elapsed().as_millis(),
                text
            )
        })
    }

    /// PA-174 B2：`post_anthropic_json` 的收口包装——真实失败逐次记录（合成预算错误除外），
    /// 预算耗尽时由 [`Self::budget_exhausted_error`] 交付最后一次真实错误。
    fn post_anthropic_json(&self, endpoint: &str, body: &Value) -> Result<Value, String> {
        let result = self.post_anthropic_json_impl(endpoint, body);
        if let Err(error) = &result {
            self.record_provider_error(error);
        }
        result
    }

    fn post_anthropic_json_impl(&self, endpoint: &str, body: &Value) -> Result<Value, String> {
        // PA-171 C3-P1：冷却期在发送前短路（0 次 HTTP）。
        self.ensure_not_cooling()?;
        // PA-172 §2.1/§3.3：单次 attempt 总超时配置化（默认 45s，与基线逐字节等价）；
        // 换手候选再按**逐 attempt deadline** 收紧（剩余为 0 ⇒ 立即 Abort，0 次 HTTP）。
        let attempt_timeout_ms = self.effective_attempt_timeout_ms();
        if attempt_timeout_ms == 0 {
            return Err(self.budget_exhausted_error());
        }
        let client = build_http_client(Duration::from_millis(attempt_timeout_ms), endpoint)?;
        let started_at = Instant::now();

        let api_key = self
            .config
            .api_key
            .as_deref()
            .ok_or_else(|| "provider 缺少 API Key".to_string())?;

        let mut request = client.post(endpoint);
        match resolve_anthropic_auth_type(&self.config) {
            ProviderAuthType::Bearer => {
                request = request.bearer_auth(api_key);
            }
            ProviderAuthType::XApiKey | ProviderAuthType::Auto => {
                request = request
                    .header("x-api-key", api_key)
                    .header("anthropic-version", "2023-06-01");
            }
        }
        // Wave-4 C3：outbound 级联注入 traceparent。
        let request = pony_observability::otel::with_current_traceparent(request);
        // B1795506800002：请求体字节闸门（0 HTTP Fail-Closed）。序列化一次并复用，
        // 与 `.json(body)` 逐字节等价；超限则在此返回，不发任何网络包。
        let request_body_bytes = serialize_request_body_with_size_gate(body)?;
        let response = block_on(async move {
            request
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(request_body_bytes)
                .send()
                .await
        }).map_err(|error| {
            format_request_error("调用 provider 失败", &error, started_at.elapsed())
        })?;

        let status = response.status();
        // PA-171 C3-P1：`Retry-After` 在 `text()` 之前读取。
        let retry_after = response_retry_after_header(&response);
        let text = block_on(async move { response.text().await }).map_err(|error| {
            format_request_error("读取 provider 返回失败", &error, started_at.elapsed())
        })?;

        if !status.is_success() {
            let mut error = provider_status_error(
                status,
                started_at.elapsed().as_millis(),
                &text,
                retry_after.as_deref(),
            );
            // L3-B ADVISORY：冷却登记成功 ⇒ 打标；重试编排见标记即**全 scope** 立即 Abort
            // （不 sleep、不再发 HTTP），保证「冷却期内 0 次 HTTP」。
            if self.enter_cooldown(&error) {
                error = crate::retry::append_cooldown_active_marker(&error);
            }
            return Err(error);
        }

        serde_json::from_str::<Value>(&text).map_err(|error| {
            format!(
                "解析 provider 返回失败：{}；耗时={}ms；原始响应：{}",
                error,
                started_at.elapsed().as_millis(),
                text
            )
        })
    }
}

/// Responses 决策/跟进结果 → chat 形态 assistant 消息（R1：存储中枢统一 chat 形态）。
fn responses_chat_assistant_message(parsed: &responses_api::ResponsesSyncOutput) -> Value {
    match parsed.tool_call.as_ref() {
        Some(tool_call) => provider_native_assistant_tool_call_message_with_reasoning_value(
            text_if_present(&parsed.output_text),
            parsed.reasoning_content_value.as_ref(),
            tool_call,
        ),
        None => provider_native_assistant_message_with_reasoning_value(
            &parsed.output_text,
            parsed.reasoning_content_value.as_ref(),
        ),
    }
}

type FinalizedResponsesStream = (
    String,
    Option<ToolCall>,
    Option<String>,
    Option<Value>,
    TokenUsage,
);

/// 流式消息收口：终态校验（无文本且无 tool_call 报错）、usage 兜底估算。
fn finalize_responses_stream_message(
    message: responses_api::ResponsesStreamMessage,
    label: &str,
    request: &ProviderRequest,
) -> Result<FinalizedResponsesStream, String> {
    let responses_api::ResponsesStreamMessage {
        output_text,
        tool_call,
        reasoning_content,
        reasoning_content_value,
        token_usage,
        dropped_function_calls,
    } = message;

    if let Some(call) = tool_call.as_ref() {
        provider_log(format!(
            "{}:responses-stream-tool-call name={} call_id={}",
            label,
            call.name,
            call.call_id.as_deref().unwrap_or("(none)")
        ));
    }
    if dropped_function_calls > 0 {
        provider_log(format!(
            "{}:responses-stream dropped_function_calls={}",
            label, dropped_function_calls
        ));
    }
    if output_text.trim().is_empty() && tool_call.is_none() {
        return Err(format!(
            "responses streamed {} missing text or tool call",
            label
        ));
    }
    let token_usage = token_usage.unwrap_or_else(|| estimate_token_usage(request, &output_text));
    provider_log_token_usage("responses stream", &token_usage);

    Ok((
        output_text,
        tool_call,
        reasoning_content,
        reasoning_content_value,
        token_usage,
    ))
}

impl ProviderClient for ProviderManager {
    fn requested_name(&self) -> &str {
        self.requested_name()
    }

    fn decide_with_tools_stream_guarded<F>(
        &self,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
        guard: &ProviderStreamGuard<'_>,
        on_delta: F,
    ) -> Result<ProviderDecision, String>
    where
        F: FnMut(ProviderStreamChunk),
    {
        self.decide_with_tools_stream_bounded(guard, request, tools, on_delta)
    }

    fn continue_with_tool_result_stream_guarded<F>(
        &self,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
        accumulated_messages: &mut Vec<Value>,
        assistant_message: Option<&Value>,
        tool_call: &ToolCall,
        tool_result: &ToolResult,
        guard: &ProviderStreamGuard<'_>,
        on_delta: F,
    ) -> Result<ProviderResponse, String>
    where
        F: FnMut(ProviderStreamChunk),
    {
        self.continue_with_tool_result_stream_bounded(
            guard,
            request,
            tools,
            accumulated_messages,
            assistant_message,
            tool_call,
            tool_result,
            on_delta,
        )
    }

    fn name(&self) -> &str {
        self.name()
    }

    fn model(&self) -> &str {
        self.model()
    }

    fn protocol_label(&self) -> &'static str {
        self.protocol_label()
    }

    fn protocol(&self) -> &ProviderProtocol {
        self.protocol()
    }

    fn temperature(&self) -> f32 {
        self.temperature()
    }

    fn max_output_tokens(&self) -> u32 {
        self.max_output_tokens()
    }

    fn decide_with_tools(
        &self,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
    ) -> Result<ProviderDecision, String> {
        ProviderManager::decide_with_tools(self, request, tools)
    }

    fn decide_with_tools_stream<F>(
        &self,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
        on_delta: F,
    ) -> Result<ProviderDecision, String>
    where
        F: FnMut(ProviderStreamChunk),
    {
        ProviderManager::decide_with_tools_stream(self, request, tools, on_delta)
    }

    fn continue_with_tool_result(
        &self,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
        accumulated_messages: &mut Vec<Value>,
        assistant_message: Option<&Value>,
        tool_call: &ToolCall,
        tool_result: &ToolResult,
    ) -> Result<ProviderResponse, String> {
        ProviderManager::continue_with_tool_result(
            self,
            request,
            tools,
            accumulated_messages,
            assistant_message,
            tool_call,
            tool_result,
        )
    }

    fn continue_with_tool_result_stream<F>(
        &self,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
        accumulated_messages: &mut Vec<Value>,
        assistant_message: Option<&Value>,
        tool_call: &ToolCall,
        tool_result: &ToolResult,
        on_delta: F,
    ) -> Result<ProviderResponse, String>
    where
        F: FnMut(ProviderStreamChunk),
    {
        ProviderManager::continue_with_tool_result_stream(
            self,
            request,
            tools,
            accumulated_messages,
            assistant_message,
            tool_call,
            tool_result,
            on_delta,
        )
    }

    /// wave-54 C4：桥梁转发真实能力（`None` ⇒ 上游不输出 context_window/ratio）。
    fn context_window_tokens(&self) -> Option<u32> {
        ProviderManager::context_window_tokens(self)
    }

    /*
     */
    #[cfg(any())]
    fn openai_stream_message_collects_content_and_reasoning() {
        let raw_text = concat!(
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"先看文件结构。\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"第 3 行是 \"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"use crate::...\"}}]}\n\n",
            "data: [DONE]\n\n"
        );
        let mut deltas = Vec::new();

        let message = collect_openai_sse_message(raw_text, &mut |delta| deltas.push(delta))
            .expect("stream message should parse");

        assert_eq!(message.output_text, "第 3 行是 use crate::...");
        assert_eq!(message.reasoning_content.as_deref(), Some("先看文件结构。"));
        assert_eq!(
            deltas,
            vec!["第 3 行是 ".to_string(), "use crate::...".to_string()]
        );
    }
    // */
}

/// wave-36：回环地址旁路代理。环境代理（HTTP_PROXY/HTTPS_PROXY/ALL_PROXY）会把
/// 对 127.0.0.1/::1/localhost 的请求劫持到代理网关，本地 provider（假服务、Ollama 等）
/// 全部被打穿。判定为纯函数以便单测：不走网络、只看 URL host。
pub fn should_bypass_proxy(base_url: &str) -> bool {
    match url::Url::parse(base_url) {
        Ok(parsed) => match parsed.host_str() {
            Some(host) => {
                let lower = host.to_ascii_lowercase();
                let host = lower.trim_start_matches('[').trim_end_matches(']');
                matches!(host, "127.0.0.1" | "::1" | "localhost" | "::ffff:127.0.0.1")
            }
            None => false,
        },
        Err(_) => false,
    }
}

/// P0-1：同步请求的建连预算上限（毫秒），与流式路径同口径。
const SYNC_CONNECT_TIMEOUT_MS: u64 = 15_000;

/// 同步请求的 HTTP 客户端（**与 [`build_streaming_http_client`] 同口径**）。
///
/// 缺陷（实测 pa_01a1e908 / pa_01a1e928）：旧实现用 `.timeout(idle_timeout)`——那是
/// 「整个请求含读完响应体」的**总时限**，与流式路径（15s 建连 + 180s 读空闲）两套口径。
/// 上游一慢，同步 followup 必被总时限砍掉；每次 attempt 都吃满时限 ⇒ 把跨 attempt
/// 聚合预算吃光，末次 attempt 只剩 6.4s/0ms（`no request sent` 的自伤终态）。
///
/// 现在只用两类预算：
/// - `connect_timeout`：建连预算（`min(idle_timeout, 15s)`），连不上立刻失败；
/// - `read_timeout`：**单次读空闲**预算——只要还有字节持续到达就不会被累计时长杀掉。
///
/// 墙钟护栏仍在：跨 attempt 聚合 deadline（`aggregate_sync_call_budget_ms`）由
/// `effective_attempt_timeout_ms` 逐 attempt 收紧，本函数不承担总时限语义。
fn build_http_client(idle_timeout: Duration, endpoint: &str) -> Result<Client, String> {
    let connect_timeout = idle_timeout.min(Duration::from_millis(SYNC_CONNECT_TIMEOUT_MS));
    let builder = Client::builder()
        .connect_timeout(connect_timeout)
        .read_timeout(idle_timeout)
        .http1_only()
        .pool_max_idle_per_host(0);
    client_for_endpoint(builder, endpoint)
}

/// wave-36：按 endpoint host 决定是否禁用代理（回环直连）。
/// reqwest 默认继承环境代理；对回环地址调用 no_proxy() 强制直连，
/// 外部域名不调用 → 代理行为保持不变。
fn client_for_endpoint(builder: reqwest::ClientBuilder, endpoint: &str) -> Result<Client, String> {
    let builder = if should_bypass_proxy(endpoint) {
        builder.no_proxy()
    } else {
        builder
    };
    builder.build().map_err(|error| format!("创建 HTTP client 失败：{}", error))
}

/// 流式请求的 HTTP 客户端。
///
/// F7：这里**不设总 deadline**。旧实现把 `.timeout()` 设成 180s，那是"整个请求（含读完响应体）"
/// 的总时限 —— 长推理流（reasoning 静默 + 长输出）一旦超过 180s 就会被整体截断，表现为流在
/// 中途断掉而不是超时报错。改为两类预算：
/// - `connect_timeout`：建连预算（15s），连不上立刻失败；
/// - `read_timeout`：**单次读空闲**预算（180s）。只要还有数据持续到达就不会被累计时长杀掉，
///   真正卡死（180s 无任何字节）的连接仍然失败。
///
/// `pool_max_idle_per_host(0)`：provider 端每请求新建连接是刻意的隔离策略（沿用既有行为），
/// 避免复用空闲连接把上一次被取消/半途中断的流状态带进新请求；它只是连接池策略，
/// 不承担任何超时语义。
fn build_streaming_http_client(
    connect_timeout: Duration,
    read_timeout: Duration,
    endpoint: &str,
) -> Result<Client, String> {
    let builder = Client::builder()
        .connect_timeout(connect_timeout)
        .read_timeout(read_timeout)
        .http1_only()
        .pool_max_idle_per_host(0);
    client_for_endpoint(builder, endpoint)
}

fn format_request_error(prefix: &str, error: &reqwest::Error, elapsed: Duration) -> String {
    // Stage3 P1：加 endpoint/elapsed 证据；type=timeout/connect 等 flags 与分类映射不变
    //（classify 仍走 retry 码表子串桥，本函数只做证据增强）。
    let mut flags = Vec::new();
    if error.is_timeout() {
        flags.push("timeout");
    }
    if error.is_connect() {
        flags.push("connect");
    }
    if error.is_request() {
        flags.push("request");
    }
    if error.is_body() {
        flags.push("body");
    }
    if error.is_decode() {
        flags.push("decode");
    }
    if error.is_status() {
        flags.push("status");
    }

    let error_type = if flags.is_empty() {
        "unknown".to_string()
    } else {
        flags.join("+")
    };

    let mut details = vec![
        format!("type={}", error_type),
        format!("elapsed={}ms", elapsed.as_millis()),
    ];

    if let Some(url) = error.url() {
        details.push(format!("url={}", url));
    }

    if let Some(source) = error.source() {
        details.push(format!("source={}", source));
    }

    format!("{}：{}；{}", prefix, error, details.join("；"))
}

fn collect_anthropic_sse_message_from_response<F>(
    response: reqwest::Response,
    started_at: Instant,
    endpoint: &str,
    on_delta: &mut F,
    guard: &ProviderStreamGuard<'_>,
) -> Result<AnthropicStreamMessage, String>
where
    F: FnMut(ProviderStreamChunk),
{
    let mut output_text = String::new();
    let mut reasoning_content = String::new();
    let mut reasoning_signature: Option<String> = None;
    let mut token_usage: Option<TokenUsage> = None;
    let mut tool_call: Option<ToolCall> = None;

    // Track tool_use content blocks by index (for streaming input_json_delta)
    let mut partial_tool_calls: BTreeMap<usize, (String, String, String)> = BTreeMap::new();
    let mut content_block_types: BTreeMap<usize, String> = BTreeMap::new();

    let mut saw_events = false;
    // ADR 0055 P0-2：首包时点信号（仅工具调用块响应不含 text/thinking 增量，须在此兜底）。
    let mut first_chunk_emitted = false;
    let mut current_event = String::new();
    let mut current_data = String::new();
    let mut line_buf: Vec<u8> = Vec::new();
    let mut total_bytes: usize = 0;

    let mut stream = response.bytes_stream();

    // Helper closure to process a complete SSE event
    let mut handle_event = |event_type: &str, data: &str| -> Result<(), String> {
        if !first_chunk_emitted {
            first_chunk_emitted = true;
            on_delta(ProviderStreamChunk::StreamStart);
        }
        match event_type {
            "content_block_start" => {
                saw_events = true;
                let value: Value = serde_json::from_str(data).map_err(|e| {
                    format!("解析 Anthropic SSE content_block_start data 失败: {}", e)
                })?;

                let index = value.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                let block_type = value
                    .get("content_block")
                    .and_then(|b| b.get("type"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();

                content_block_types.insert(index, block_type.clone());

                if block_type == "tool_use" {
                    let block = value.get("content_block").unwrap();
                    let name = block
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let id = block
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    partial_tool_calls.insert(index, (name, id, String::new()));
                }
            }
            "content_block_delta" => {
                saw_events = true;
                let value: Value = serde_json::from_str(data).map_err(|e| {
                    format!("解析 Anthropic SSE content_block_delta data 失败: {}", e)
                })?;

                let index = value.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                let delta = match value.get("delta") {
                    Some(d) => d,
                    None => return Ok(()),
                };

                let delta_type = delta.get("type").and_then(Value::as_str).unwrap_or("");

                match delta_type {
                    "text_delta" => {
                        if let Some(text) = delta.get("text").and_then(Value::as_str) {
                            if !text.is_empty() {
                                output_text.push_str(text);
                                on_delta(ProviderStreamChunk::Text(text.to_string()));
                            }
                        }
                    }
                    "thinking_delta" => {
                        if let Some(thinking) = delta.get("thinking").and_then(Value::as_str) {
                            if !thinking.is_empty() {
                                reasoning_content.push_str(thinking);
                                on_delta(ProviderStreamChunk::Reasoning(thinking.to_string()));
                            }
                        }
                        if let Some(signature) = delta.get("signature").and_then(Value::as_str) {
                            reasoning_signature = Some(signature.to_string());
                        }
                    }
                    "input_json_delta" => {
                        if let Some(partial) = delta.get("partial_json").and_then(Value::as_str) {
                            partial_tool_calls
                                .entry(index)
                                .and_modify(|(_, _, input)| input.push_str(partial));
                        }
                    }
                    _ => {}
                }
            }
            "content_block_stop" => {
                saw_events = true;
                let value: Value = serde_json::from_str(data).map_err(|e| {
                    format!("解析 Anthropic SSE content_block_stop data 失败: {}", e)
                })?;

                let index = value.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;

                // Finalize tool call if this block was a tool_use
                if content_block_types.get(&index).map(|s| s.as_str()) == Some("tool_use") {
                    if let Some((name, id, raw_input)) = partial_tool_calls.remove(&index) {
                        if tool_call.is_some() {
                            return Err(
                                "Anthropic 返回了多个工具调用，但当前运行时仅支持单工具调用。"
                                    .to_string(),
                            );
                        }
                        let arguments: Value =
                            serde_json::from_str(&raw_input).unwrap_or(Value::Null);
                        let arguments = if arguments.is_null() {
                            json!({})
                        } else {
                            arguments
                        };
                        tool_call = Some(ToolCall {
                            call_id: Some(id),
                            name,
                            arguments,
                            plan: None,
                        });
                    }
                }
            }
            "message_start" | "message_stop" => {
                saw_events = true;
            }
            "message_delta" => {
                saw_events = true;
                let value: Value = serde_json::from_str(data)
                    .map_err(|e| format!("解析 Anthropic SSE message_delta data 失败: {}", e))?;

                if let Some(usage) = value.get("usage") {
                    token_usage = Some(normalize_token_usage(TokenUsage {
                        input_tokens: usage.get("input_tokens").and_then(Value::as_u64),
                        cache_hit_input_tokens: None,
                        cache_hit_source: None,
                        reasoning_tokens: None,
                        output_tokens: usage.get("output_tokens").and_then(Value::as_u64),
                        total_tokens: usage.get("total_tokens").and_then(Value::as_u64),
                        // wave-54 C4：anthropic `input` 不含 cache_read / cache_creation，
                        // 两者仅用于 context_input 归一（不并入 cache_read 上报）。
                        cache_creation_input_tokens: usage
                            .get("cache_creation_input_tokens")
                            .and_then(Value::as_u64),
                        // wave-54 C3：上游 usage 真值。
                        origin: Some(USAGE_ORIGIN_PROVIDER),
                    }));
                }
            }
            "ping" => {
                // Keep-alive, ignore
            }
            _ => {}
        }
        Ok(())
    };

    let mut last_cancel_check = started_at;
    let mut parsed_bytes: usize = 0;
    // wave-60 C4：停滞读可中断——单次 `next()` 最多等一个 tick；到点无 chunk 即回到
    // 循环顶部重查取消位与总时长上界（取消后 ≤ 2×tick 内退出）。零节流（测试注入）
    // 退化为缺省 tick，避免 `timeout(0)` 忙等。
    let poll_tick = {
        let cancel_tick = guard.cancel_tick();
        if cancel_tick.is_zero() {
            ProviderStreamGuard::DEFAULT_CANCEL_TICK
        } else {
            cancel_tick
        }
    };
    loop {
        // T9：①总时长上界 ②按 tick 节流的取消观测（与 openai/responses 帧骨架同口径）。
        let elapsed = started_at.elapsed();
        if elapsed > guard.max_elapsed() {
            return Err(format!(
                "{}: provider 流总时长超限; elapsed={}ms; max_elapsed={}ms; parsed_bytes={}; endpoint={}",
                ProviderStreamGuard::DEADLINE_ERROR_CODE,
                elapsed.as_millis(),
                guard.max_elapsed().as_millis(),
                parsed_bytes,
                endpoint,
            ));
        }
        let now = Instant::now();
        if guard.cancel_tick().is_zero() || now.duration_since(last_cancel_check) >= guard.cancel_tick()
        {
            last_cancel_check = now;
            if guard.cancel_requested() {
                return Err(format!(
                    "{}: SSE 流已按请求中止 (stop_requested); elapsed={}ms; parsed_bytes={}; endpoint={}",
                    ProviderStreamGuard::CANCELLED_ERROR_CODE,
                    started_at.elapsed().as_millis(),
                    parsed_bytes,
                    endpoint,
                ));
            }
        }

        // Wave-61 C12：`tokio::time::timeout(...)` 必须在 **async 上下文内**构造 ——
        // 若在 `block_on(...)` 之前就地求值，调用线程没有 reactor 时会直接在 future 构造
        // 阶段 panic（`no reactor running`），而 `runtime_helper::block_on` 的静态 Runtime
        // 兜底只在进入 block_on 之后才生效，救不了这一段。故把构造移进 async 块。
        match block_on(async { tokio::time::timeout(poll_tick, stream.next()).await }) {
            Err(_stalled) => continue,
            Ok(Some(Ok(bytes))) => {
                let chunk_len = bytes.len();
                parsed_bytes += chunk_len;
                for &byte in bytes.iter() {
                    if byte == b'\n' {
                        let line_str = String::from_utf8_lossy(&line_buf);
                        let trimmed = line_str.trim();

                        if trimmed.is_empty() {
                            // Empty line = end of one SSE event
                            if !current_event.is_empty() && !current_data.is_empty() {
                                handle_event(&current_event, &current_data)?;
                                current_event.clear();
                                current_data.clear();
                            }
                        } else if let Some(event_name) = trimmed.strip_prefix("event:") {
                            current_event = event_name.trim().to_string();
                        } else if let Some(data_str) = trimmed.strip_prefix("data:") {
                            current_data = data_str.trim().to_string();
                        }

                        line_buf.clear();
                    } else {
                        if line_buf.len() >= 1 << 20 {
                            return Err(format!(
                                "Anthropic SSE 行缓冲超过 1MB 上限; elapsed={}ms; endpoint={}",
                                started_at.elapsed().as_millis(),
                                endpoint,
                            ));
                        }
                        line_buf.push(byte);
                    }
                }
                total_bytes += chunk_len;
            }
            Ok(Some(Err(error))) => {
                return Err(format!(
                    "读取 Anthropic SSE 流失败: {}; elapsed={}ms; parsed_bytes={}; endpoint={}",
                    error,
                    started_at.elapsed().as_millis(),
                    total_bytes,
                    endpoint,
                ));
            }
            Ok(None) => break,
        }
    }

    // Process any remaining event at end of stream
    if !current_event.is_empty() && !current_data.is_empty() {
        handle_event(&current_event, &current_data)?;
    }

    if !saw_events {
        return Err(format!(
            "Anthropic 流式返回中未找到 SSE 事件；endpoint={}",
            endpoint,
        ));
    }

    // Build final reasoning value
    let final_reasoning = if reasoning_content.trim().is_empty() {
        None
    } else {
        Some(reasoning_content.clone())
    };
    let final_reasoning_value = (!reasoning_content.trim().is_empty()).then(|| {
        let mut block = json!({
            "type": "thinking",
            "thinking": reasoning_content
        });
        if let Some(signature) = reasoning_signature {
            block["signature"] = Value::String(signature);
        }
        Value::Array(vec![block])
    });

    Ok(AnthropicStreamMessage {
        output_text,
        tool_call,
        reasoning_content: final_reasoning,
        reasoning_content_value: final_reasoning_value,
        token_usage,
    })
}

#[test]
fn openai_usage_extracts_cache_hit_and_reasoning_tokens() {
    let payload = json!({
        "usage": {
            "prompt_tokens": 120,
            "prompt_cache_hit_tokens": 48,
            "completion_tokens": 32,
            "total_tokens": 152,
            "completion_tokens_details": {
                "reasoning_tokens": 12
            }
        }
    });

    let usage = extract_openai_usage(&payload).expect("usage should be extracted");
    assert_eq!(usage.input_tokens, Some(120));
    assert_eq!(usage.cache_hit_input_tokens, Some(48));
    assert_eq!(usage.reasoning_tokens, Some(12));
    assert_eq!(usage.output_tokens, Some(32));
    assert_eq!(usage.total_tokens, Some(152));
}

fn with_openai_request_options(mut body: Value, config: &ResolvedProviderSelection) -> Value {
    provider_log(format!(
        "with_openai_request_options pattern={:?} supports_reasoning={} reasoning_effort={:?}",
        config.thinking_param_pattern,
        config.capabilities.supports_reasoning,
        config.reasoning_effort,
    ));

    if thinking_param_needs_thinking_toggle(config) {
        body["thinking"] = json!({
            "type": "enabled"
        });
    }

    match config.thinking_param_pattern {
        ThinkingParamPattern::EffortWithNone => {
            let effort = config
                .reasoning_effort
                .as_ref()
                .map(deepseek_reasoning_effort_label)
                .unwrap_or("medium");
            body["reasoning_effort"] = Value::String(effort.to_string());
        }
        ThinkingParamPattern::EffortStandard => {
            if let Some(effort) = config.reasoning_effort.as_ref() {
                body["reasoning_effort"] =
                    Value::String(reasoning_effort_label(effort).to_string());
            }
        }
        ThinkingParamPattern::AnthropicThinking => {
            if let Some(budget) = config.reasoning_budget_tokens.filter(|b| *b > 0) {
                body["thinking"] = json!({
                    "type": "enabled",
                    "budget_tokens": budget
                });
            }
        }
        ThinkingParamPattern::None | ThinkingParamPattern::ThinkingToggle => {}
    }

    if config.capabilities.supports_reasoning {
        if let Some(obj) = body.as_object_mut() {
            obj.remove("temperature");
        }
    }

    body
}

/// Resolves the effective auth type for anthropic-protocol requests.
///
/// `Auto` derives from the protocol: anthropic → x-api-key, openai → bearer.
/// This allows providers like SenseNova that expose an Anthropic-compatible
/// endpoint but require `Authorization: Bearer` instead of `x-api-key`.
fn resolve_anthropic_auth_type(config: &ResolvedProviderSelection) -> ProviderAuthType {
    match &config.auth_type {
        ProviderAuthType::Auto => {
            if config.protocol.is_anthropic() {
                ProviderAuthType::XApiKey
            } else {
                ProviderAuthType::Bearer
            }
        }
        ProviderAuthType::Bearer => ProviderAuthType::Bearer,
        ProviderAuthType::XApiKey => ProviderAuthType::XApiKey,
    }
}

fn with_anthropic_request_options(mut body: Value, config: &ResolvedProviderSelection) -> Value {
    if config.capabilities.supports_reasoning {
        if let Some(budget_tokens) = config.reasoning_budget_tokens.filter(|budget| *budget > 0) {
            body["thinking"] = json!({
                "type": "enabled",
                "budget_tokens": budget_tokens
            });
        }
    }

    body
}

fn apply_openai_tool_capability(
    mut body: Value,
    tools: &[ToolDefinition],
    config: &ResolvedProviderSelection,
) -> Value {
    let supports_tools = config.capabilities.supports_tools && !tools.is_empty();
    let supports_tool_choice = thinking_param_supports_tool_choice(config);

    if supports_tools {
        if !supports_tool_choice {
            if let Some(object) = body.as_object_mut() {
                object.remove("tool_choice");
            }
        }
        return body;
    }

    if let Some(object) = body.as_object_mut() {
        object.remove("tools");
        object.remove("tool_choice");
    }
    body
}

fn apply_anthropic_tool_capability(
    mut body: Value,
    tools: &[ToolDefinition],
    config: &ResolvedProviderSelection,
) -> Value {
    if config.capabilities.supports_tools && !tools.is_empty() {
        return body;
    }

    if let Some(object) = body.as_object_mut() {
        object.remove("tools");
        object.remove("tool_choice");
    }
    body
}

fn thinking_param_needs_thinking_toggle(config: &ResolvedProviderSelection) -> bool {
    config.thinking_param_pattern == ThinkingParamPattern::ThinkingToggle
}

fn thinking_param_supports_tool_choice(config: &ResolvedProviderSelection) -> bool {
    config.thinking_param_pattern != ThinkingParamPattern::ThinkingToggle
}

fn reasoning_effort_label(effort: &ProviderReasoningEffort) -> &'static str {
    return match effort {
        ProviderReasoningEffort::Low => "low",
        ProviderReasoningEffort::Medium => "medium",
        ProviderReasoningEffort::High => "high",
        ProviderReasoningEffort::Max => "max",
    };

    #[cfg(any())]
    fn openai_stream_message_collects_content_and_reasoning() {
        let raw_text = concat!(
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"先看文件结构。\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"第 3 行是 \"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"use crate::...\"}}]}\n\n",
            "data: [DONE]\n\n"
        );
        let mut deltas = Vec::new();

        let message = collect_openai_sse_message(raw_text, &mut |delta| deltas.push(delta))
            .expect("stream message should parse");

        assert_eq!(message.output_text, "第 3 行是 use crate::...");
        assert_eq!(message.reasoning_content.as_deref(), Some("先看文件结构。"));
        assert_eq!(
            deltas,
            vec!["第 3 行是 ".to_string(), "use crate::...".to_string()]
        );
    }
    // */
}

fn deepseek_reasoning_effort_label(effort: &ProviderReasoningEffort) -> &'static str {
    match effort {
        ProviderReasoningEffort::Low => "low",
        ProviderReasoningEffort::Medium => "medium",
        ProviderReasoningEffort::High => "high",
        ProviderReasoningEffort::Max => "high",
    }
}

fn to_chat_role(role: &ProviderRole) -> &'static str {
    match role {
        ProviderRole::System | ProviderRole::Developer => "system",
        ProviderRole::User => "user",
    }
}

/// 工具数组 → provider 契约视图（**唯一**构建入口，5 处 payload 共用）。
///
/// ADR 0065 decision 4 / ADR 0066（T2 交界修复）：此前是二选一分支 ——
/// `is_builtin_tool_definition_set(tools)` 为真走 registry 投影，为假整组掉进逐条 fallback。
/// 一旦外部 descriptor（MCP / 插件 / 动态工具）进入数组，判定为假 ⇒ 45 个内建工具同时丢掉
/// registry 投影的去重与顺序（`Run`/`Read` 等产品名重复出现），模型工具面前缀被破坏。
///
/// 现在的规则是**分层合并**，而不是二选一：
/// 1. 名字被内建快照承认的条目 ⇒ 整体替换为 registry 投影（去重、registry 序）。
///    纯内建数组的输出与旧「is_builtin_tool_definition_set == true」分支**逐字节相同**。
/// 2. 快照不认识的条目（`mcp__*` / `plugin__*` / 动态名）⇒ 按数组顺序追加在投影之后，
///    逐条走 legacy `contract_view()`（名字原样透传，绝不伪造内建产品名）。
/// 3. 两者都没有（旧 fallback 场景：调用方给了全是未知名字且无内建条目的数组）⇒ 保持旧行为。
/// 工具数组 → provider 契约视图（**唯一**构建入口，5 处 payload 共用）。
///
/// **可见性说明**：`pub` 仅为满足契约验收（KV-cache 前缀不变量：追加外部工具后内建 38 项
/// 视图逐字节不变、顺序不变）——它**不是**对外公共 API 承诺，crate 外消费者不要依赖；
/// 语义演进以 `docs/decisions/proposed/0065-tool-surface-single-truth.md` 为准。
pub fn surface_contract_views(
    tools: &[ToolDefinition],
) -> Vec<pony_protocol::tool_contract::ToolDefinitionContractView> {
    if tools.is_empty() {
        return Vec::new();
    }

    let Ok(registry) = pony_protocol::tool_contract::ToolRegistrySnapshot::builtin() else {
        // 内建快照本身不可用属于装配期错误：退回旧 fallback，绝不 panic。
        return legacy_fallback_contract_views(tools);
    };

    let mut owned_primitives: Vec<&str> = Vec::new();
    let mut external_views = Vec::new();
    for tool in tools {
        match registry.resolve(tool.name) {
            Some(descriptor) => owned_primitives.push(descriptor.identity.primitive_name.as_str()),
            None => {
                if !is_internal_builtin_tool_name(&tool.name) {
                    // 外部/动态工具：按数组顺序追加，身份仍是调用方给的名字
                    // （绝不映射成内建产品名）。
                    external_views.push(tool.contract_view());
                }
            }
        }
    }

    if owned_primitives.is_empty() {
        // 全是外部/动态名 ⇒ 旧 fallback 语义（含 internal 过滤），不虚构内建面。
        return legacy_fallback_contract_views(tools);
    }

    // 数组是否覆盖了完整的模型可见内建面：只有此时才用 registry 投影（去重 + registry 序）。
    // 子集调用方（例如给定 [workflow, subagent]）保持旧 fallback 行为——不得把部分工具视图
    // 静默扩张成完整工具面（frozen 用例 `fallback_tool_payload_...` 锁定这一点）。
    let full_builtin_surface = {
        let Ok(views) = builtin_tool_surface().provider_contract_views() else {
            return legacy_fallback_contract_views(tools);
        };
        views.iter().all(|view| {
            owned_primitives
                .iter()
                .any(|primitive| *primitive == view.execution_primitive.as_str())
        })
    };
    if !full_builtin_surface {
        return legacy_fallback_contract_views(tools);
    }

    // 内建面 = registry 投影（去重 + registry 序），外部面追加其后（顺序稳定，见
    // `mcp_config::surface_tool_definitions` 的 (source_id, model_name) 排序）。
    let mut views = builtin_provider_contract_views();
    views.extend(external_views);
    views
}

/// 旧 fallback：逐条 legacy `contract_view()`，并挡掉已知 internal 的内建名。
fn legacy_fallback_contract_views(
    tools: &[ToolDefinition],
) -> Vec<pony_protocol::tool_contract::ToolDefinitionContractView> {
    tools
        .iter()
        .filter(|tool| !is_internal_builtin_tool_name(&tool.name))
        .map(|tool| tool.contract_view())
        .collect()
}

fn openai_tools_payload(tools: &[ToolDefinition]) -> Vec<Value> {
    let contract_views = surface_contract_views(tools);

    contract_views
        .iter()
        .map(|tool| {
            json!({
                "type": "function",
                "function": {
                    "name": openai_safe_tool_name(&tool.name),
                    "description": tool.description,
                    "parameters": tool.input_schema.clone()
                }
            })
        })
        .collect()
}

fn anthropic_tools_payload(tools: &[ToolDefinition]) -> Vec<Value> {
    let contract_views = surface_contract_views(tools);

    contract_views
        .iter()
        .map(|tool| {
            json!({
                "name": tool.name,
                "description": tool.description,
                "input_schema": tool.input_schema.clone()
            })
        })
        .collect()
}

fn provider_tool_contract_views(
    tools: &[ToolDefinition],
) -> Vec<pony_protocol::tool_contract::ToolDefinitionContractView> {
    surface_contract_views(tools)
}

fn builtin_provider_contract_views() -> Vec<pony_protocol::tool_contract::ToolDefinitionContractView> {
    builtin_tool_surface()
        .provider_contract_views()
        .expect("default turn tool view must match its registry snapshot")
}

/// W28 附带必修：非 builtin 工具集走 fallback 分支时，仍必须挡住「已知 builtin 但
/// `exposure=Internal`」的名字——否则内部工具（如 `subagent` 桩）可经动态工具通道回流到
/// 模型可见面，绕过 exposure 策略。未知/动态工具（`mcp__<server>__<tool>` 等）原样透传。
pub fn is_internal_builtin_tool_name(name: &str) -> bool {
    use pony_protocol::tool_contract::{canonical_tool_name, tool_exposure_for_name, ToolExposure};
    canonical_tool_name(name).is_some()
        && matches!(tool_exposure_for_name(name), ToolExposure::Internal)
}

/// 工具集合身份判定（ADR 0065 decision 4）：**按 registry 快照事实**判断调用方交来的数组是不是
/// 内建工具面，不再按数组长度 + 逐字节内容比对。
///
/// 旧判据的坑（T1/T2 共有地雷）：一旦外部 descriptor（MCP / 插件）进入数组，长度或内容比对
/// 立刻失配 ⇒ 静默翻转到 fallback，45 个内建工具同时丢掉 descriptor 事实与 registry 排序，
/// 而排序是 KV-cache 稳定前缀的一部分 ⇒ 前缀抖动。
///
/// 新判据：
/// 1. 数组非空，且每个条目的名字都被内建快照按名字承认（任一 exposure）——名字是快照事实，
///    描述/schema 由快照提供，所以数组里手写的描述内容不参与身份判定；
/// 2. 数组覆盖快照投影的全部模型可见 primitive——子集调用方（planner 只给几个工具）仍走
///    fallback，不会被静默扩张成完整工具面。
#[cfg_attr(not(test), allow(dead_code))]
fn is_builtin_tool_definition_set(tools: &[ToolDefinition]) -> bool {
    use pony_protocol::tool_contract::{canonical_tool_name, ToolSurface};

    if tools.is_empty() {
        return false;
    }
    let Ok(surface) = ToolSurface::builtin() else {
        return false;
    };
    if !tools
        .iter()
        .all(|tool| surface.registry.resolve(tool.name).is_some())
    {
        return false;
    }
    let Ok(views) = surface.provider_contract_views() else {
        return false;
    };
    views.iter().all(|view| {
        tools.iter().any(|tool| {
            canonical_tool_name(tool.name) == Some(view.execution_primitive.as_str())
        })
    })
}

fn first_openai_message<'a>(payload: &'a Value) -> Result<&'a Value, String> {
    payload
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .ok_or_else(|| "openai 返回中缺少 message".to_string())
}

fn normalize_openai_assistant_message(message: &Value) -> Value {
    let tool_calls = message.get("tool_calls").cloned();
    let content_value = match message.get("content") {
        Some(Value::String(text)) => Value::String(text.clone()),
        Some(Value::Array(items)) => Value::Array(items.clone()),
        Some(Value::Null) | None => {
            if tool_calls
                .as_ref()
                .and_then(Value::as_array)
                .map(|calls| !calls.is_empty())
                .unwrap_or(false)
            {
                Value::String(extract_openai_message_text(message).unwrap_or_default())
            } else {
                Value::Null
            }
        }
        Some(other) => other.clone(),
    };

    let reasoning_value = extract_openai_message_reasoning_value(message).unwrap_or(Value::Null);

    let mut normalized = json!({
        "role": "assistant",
        "content": content_value,
        "reasoning_content": reasoning_value,
    });

    if let Some(tool_calls) = tool_calls {
        normalized["tool_calls"] = tool_calls;
    }

    // DeepSeek thinking 模式：带 tool_calls 的 assistant 消息必须回传非空
    // reasoning_content，否则 API 返回 400（实测校验只要求非空，不校验内容一致性）。
    // 本地 planner 决策等来源没有该字段，这里兜底填充占位文本。
    // 注意：结构化 reasoning（数组）是合法值，必须原样保留。
    let reasoning_missing = match normalized.get("reasoning_content") {
        Some(Value::Null) | None => true,
        Some(Value::String(text)) => text.trim().is_empty(),
        Some(_) => false,
    };
    if normalized
        .get("tool_calls")
        .and_then(Value::as_array)
        .map(|calls| !calls.is_empty())
        .unwrap_or(false)
        && reasoning_missing
    {
        normalized["reasoning_content"] =
            Value::String(TOOL_CALL_REASONING_PLACEHOLDER.to_string());
    }

    normalized
}

fn normalize_openai_messages(messages: Vec<Value>) -> Vec<Value> {
    messages
        .into_iter()
        .map(|message| {
            if message.get("role").and_then(Value::as_str) == Some("assistant") {
                normalize_openai_assistant_message(&message)
            } else {
                message
            }
        })
        .collect()
}

fn extract_openai_message_text(message: &Value) -> Option<String> {
    let content = message.get("content")?;

    match content {
        Value::String(text) => Some(text.clone()),
        Value::Array(items) => {
            let parts = items
                .iter()
                .filter_map(|item| {
                    if let Some(text) = item.get("text").and_then(Value::as_str) {
                        return Some(text.to_string());
                    }
                    item.get("text")
                        .and_then(|value| value.get("value"))
                        .and_then(Value::as_str)
                        .map(|text| text.to_string())
                })
                .collect::<Vec<_>>();

            if parts.is_empty() {
                None
            } else {
                Some(parts.join("\n"))
            }
        }
        Value::Null => None,
        _ => None,
    }
}

/// P1-1 最小切片（public 纯函数）：解析 OpenAI assistant message 里的**全部** `tool_calls`。
///
/// 顺序与上游数组一致；缺失/非数组/单条畸形 ⇒ 该条跳过（绝不 panic）。
/// 单数版 [`Self::extract_openai_tool_call`] 只是本函数首元素的兼容包装，行为不变。
pub fn extract_openai_tool_calls(message: &Value) -> Vec<ToolCall> {
    let Some(raw_calls) = message.get("tool_calls").and_then(Value::as_array) else {
        return Vec::new();
    };
    raw_calls
        .iter()
        .filter_map(parse_single_openai_tool_call)
        .collect()
}

/// P1-1：复数解析的兼容包装（既有调用点语义不变：只取首个）。
fn extract_openai_tool_call(message: &Value) -> Option<ToolCall> {
    extract_openai_tool_calls(message).into_iter().next()
}

/// P1-1 最小切片：OpenAI 非流式响应含**多个** `tool_calls` 时不再静默丢弃。
///
/// D3a 之后 OpenAI 的三条非流式入口（decision / followup_sync / followup_sync_batch）都已
/// 整批上报给运行时批执行，**本告警失去真实触发点**（不再有调用被丢弃）；保留纯函数与其单测
/// 作为 D1a「静默截断」的回归凭证，待 D3b（流式站点批执行）落地后连同单测一并删除。
#[allow(dead_code)]
fn warn_on_dropped_openai_tool_calls(stage: &str, message: &Value, kept: Option<&ToolCall>) {
    if let Some(line) = openai_dropped_tool_calls_warning(stage, message, kept) {
        provider_log(line);
    }
}

/// 组装「多调用被丢弃」的结构化告警行（纯函数，便于单测锁字段）。
///
/// 格式：`multi_tool_calls_dropped stage=<stage> received=<N> dropped=<N-1> dropped_names=[…] kept=Some("…")`
/// 语义：`stage` 区分 OpenAl 的两条非流式入口（decision / followup_sync），`dropped_names`
/// 列出**未执行**的调用名（模型侧可据此判断是不是自己给多了）。
///
/// D3a 之后仅由单测调用（生产入口不再丢弃），见 [`warn_on_dropped_openai_tool_calls`] 的说明。
#[allow(dead_code)]
fn openai_dropped_tool_calls_warning(
    stage: &str,
    message: &Value,
    kept: Option<&ToolCall>,
) -> Option<String> {
    let calls = extract_openai_tool_calls(message);
    if calls.len() <= 1 {
        return None;
    }
    let dropped_names: Vec<&str> = calls.iter().skip(1).map(|call| call.name.as_str()).collect();
    Some(format!(
        "multi_tool_calls_dropped stage={stage} received={} dropped={} dropped_names={:?} kept={:?}",
        calls.len(),
        calls.len() - 1,
        dropped_names,
        kept.map(|call| call.name.as_str())
    ))
}

fn parse_single_openai_tool_call(tool_call: &Value) -> Option<ToolCall> {
    let id = tool_call
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .map(str::to_string);
    let function = tool_call.get("function")?;
    // 空洞防护（46040ae 意图收窄）：仅"空名 + 空参数"的上游空洞 tool call 按无工具调用
    // 处理，避免构造 name="" 的 ToolCall 向 normalize 抛错整轮失败；空名但参数非空时
    // 必须保留 name="" 的 ToolCall，交给 normalize_tool_directive 的修复路径
    // （infer_tool_name_from_arguments）收编，不得在此丢弃（否则修复路径不可达）。
    let raw_name = function.get("name").and_then(Value::as_str).unwrap_or("");
    let name = openai_original_tool_name(raw_name);
    let arguments = function
        .get("arguments")
        .and_then(Value::as_str)
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .map(|v| if v.is_null() { json!({}) } else { v })
        .unwrap_or_else(|| json!({}));
    if name.trim().is_empty() && is_hollow_tool_arguments(&arguments) {
        return None;
    }

    Some(ToolCall {
        call_id: id,
        name,
        arguments,
        plan: None,
    })
}

/// 空洞参数判定：与 runtime/stream_support.rs 的 `is_empty_tool_arguments` 语义一致
/// （空名 tool call 是否因"无参数"而判定为空洞、应被丢弃）。
fn is_hollow_tool_arguments(arguments: &Value) -> bool {
    match arguments {
        Value::Null => true,
        Value::Object(map) => map.is_empty(),
        Value::String(raw) => {
            let trimmed = raw.trim();
            trimmed.is_empty() || trimmed == "{}" || trimmed == "null"
        }
        _ => false,
    }
}

fn text_if_present(text: &str) -> Option<&str> {
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

fn is_system_reminder_message(content: &str) -> bool {
    let trimmed = content.trim_start();
    trimmed.starts_with("<system-reminder>")
}

fn anthropic_system_text(input: &[ProviderMessage]) -> String {
    input
        .iter()
        .filter(|message| matches!(message.role, ProviderRole::System | ProviderRole::Developer))
        // 过滤掉 Previous assistant reply（assistant 轮次）与尾部 <system-reminder>（易变提醒进 user 消息，避免打穿 Anthropic system 缓存）
        .filter(|message| {
            !message.content.starts_with("Previous assistant reply:")
                && (!matches!(message.role, ProviderRole::Developer)
                    || !is_system_reminder_message(&message.content))
        })
        .map(|message| message.content.clone())
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub fn anthropic_system_payload(input: &[ProviderMessage]) -> Value {
    let text = anthropic_system_text(input);
    if text.trim().is_empty() {
        Value::String(String::new())
    } else {
        // 声明 cache_control 断点，触发 Anthropic Prompt Caching
        json!([
            {
                "type": "text",
                "text": text,
                "cache_control": {
                    "type": "ephemeral"
                }
            }
        ])
    }
}

pub fn build_anthropic_test_request_payload(system: &str, user_text: &str) -> Value {
    json!({
        "system": anthropic_system_payload(&[ProviderMessage::system(system)]),
        "messages": [
            {
                "role": "user",
                "content": user_text
            }
        ]
    })
}

pub fn extract_anthropic_usage_from_sse_payload(payload_str: &str) -> TokenUsage {
    for line in payload_str.lines() {
        if let Some(data) = line.strip_prefix("data: ") {
            if let Ok(v) = serde_json::from_str::<Value>(data) {
                if let Some(usage) = extract_anthropic_usage(&v) {
                    return usage;
                }
            }
        }
    }
    TokenUsage::default()
}

/// 构建 Anthropic API 的 messages 数组，按原始顺序包含：
/// - user 消息（含图片处理）
/// - "Previous assistant reply" 作为 assistant 角色消息
/// 这些消息会按它们在 input 中出现的先后顺序排列，确保对话历史连贯。
fn anthropic_conversation_messages(request: &ProviderRequest) -> Vec<Value> {
    if !request.native_messages.is_empty()
        && request.native_messages.iter().all(|message| {
            matches!(
                message.get("role").and_then(Value::as_str),
                Some("user" | "assistant")
            ) && message.get("tool_calls").is_none()
                && message.get("reasoning_content").is_none()
        })
    {
        return request.native_messages.clone();
    }

    let user_indexes: Vec<usize> = request
        .input
        .iter()
        .enumerate()
        .filter_map(|(i, m)| matches!(m.role, ProviderRole::User).then_some(i))
        .collect();
    let last_user_index = user_indexes.last().copied();

    let mut messages = Vec::new();
    for (index, message) in request.input.iter().enumerate() {
        match message.role {
            ProviderRole::User => {
                messages.push(json!({
                    "role": "user",
                    "content": anthropic_user_content(
                        message.content.as_str(),
                        is_last_user_with_images(index, last_user_index, &request.images),
                        &request.images,
                    )
                }));
            }
            ProviderRole::Developer => {
                // "Previous assistant reply" 作为 assistant 角色加入 messages 数组
                if let Some(reply) = message.content.strip_prefix("Previous assistant reply: ") {
                    messages.push(json!({
                        "role": "assistant",
                        "content": reply
                    }));
                } else if is_system_reminder_message(&message.content) {
                    // 易变运行时提醒作为 user <system-reminder> 加入对话尾部，避免污染 system 前缀缓存
                    messages.push(json!({
                        "role": "user",
                        "content": message.content
                    }));
                }
                // 其他稳定 Developer 消息继续留在 system text 中（由 anthropic_system_text 处理）
            }
            ProviderRole::System => {
                // System 消息留在 system text 中
            }
        }
    }
    messages
}

fn extract_anthropic_text_blocks(content: &[Value]) -> String {
    content
        .iter()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .map(str::to_string)
        .collect::<Vec<_>>()
        .join("\n")
}

fn extract_anthropic_thinking_value(content: &[Value]) -> Option<Value> {
    let blocks = content
        .iter()
        .filter_map(|block| {
            if block.get("type").and_then(Value::as_str) != Some("thinking") {
                return None;
            }

            block
                .get("thinking")
                .and_then(Value::as_str)
                .or_else(|| block.get("text").and_then(Value::as_str))?;
            Some(block.clone())
        })
        .collect::<Vec<_>>();

    if blocks.is_empty() {
        None
    } else {
        Some(Value::Array(blocks))
    }
}

fn extract_anthropic_thinking_text(content: &[Value]) -> Option<String> {
    extract_anthropic_thinking_value(content)
        .as_ref()
        .and_then(extract_reasoning_text_from_value)
}

/// P1-1 最小切片（public 纯函数）：解析 Anthropic content blocks 里的**全部** `tool_use`。
///
/// 顺序按出现顺序；`text` / `thinking` 等其它块被忽略；畸形块跳过（绝不 panic）。
/// 单数版 [`Self::extract_anthropic_tool_call`] 只是本函数首元素的兼容包装，行为不变。
pub fn extract_anthropic_tool_calls(content: &[Value]) -> Vec<ToolCall> {
    content
        .iter()
        .filter_map(|block| {
            if block.get("type").and_then(Value::as_str) != Some("tool_use") {
                return None;
            }
            Some(ToolCall {
                call_id: block.get("id").and_then(Value::as_str).map(str::to_string),
                name: block.get("name").and_then(Value::as_str)?.to_string(),
                arguments: block.get("input").cloned().unwrap_or_else(|| json!({})),
                plan: None,
            })
        })
        .collect()
}

/// P1-1：复数解析的兼容包装（既有调用点语义不变：只取首个）。
fn extract_anthropic_tool_call(content: &[Value]) -> Option<ToolCall> {
    extract_anthropic_tool_calls(content).into_iter().next()
}

fn openai_messages_with_tool_result(
    request: &ProviderRequest,
    accumulated_messages: &mut Vec<Value>,
    assistant_message: Option<&Value>,
    tool_call: &ToolCall,
    tool_result: &ToolResult,
) -> Vec<Value> {
    let mut messages = if !accumulated_messages.is_empty() {
        accumulated_messages.clone()
    } else {
        openai_request_messages(request)
    };

    messages.push(openai_assistant_message_for_tool_result(
        assistant_message,
        tool_call,
    ));

    messages.push(openai_followup_tool_result_message(tool_call, tool_result));

    *accumulated_messages = messages.clone();

    messages
}

fn openai_assistant_message_for_tool_result(
    assistant_message: Option<&Value>,
    tool_call: &ToolCall,
) -> Value {
    let Some(message) = assistant_message else {
        // 本地 planner 决策没有 reasoning_content；DeepSeek thinking 模式要求
        // 带 tool_calls 的 assistant 消息必须回传非空 reasoning_content，
        // 这里兜底填充占位文本（实测 API 只校验非空，不校验内容一致性）。
        return provider_native_assistant_tool_call_message_with_reasoning_value(
            None,
            Some(&Value::String(TOOL_CALL_REASONING_PLACEHOLDER.to_string())),
            tool_call,
        );
    };

    let mut normalized = normalize_openai_assistant_message(message);
    let filtered_tool_calls = normalized
        .get("tool_calls")
        .and_then(Value::as_array)
        .map(|calls| {
            calls
                .iter()
                .filter(|call| openai_tool_call_matches_executed_call(call, tool_call))
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    if filtered_tool_calls.is_empty()
        || tool_call
            .call_id
            .as_deref()
            .map_or(true, |id| id.is_empty())
    {
        return provider_native_assistant_tool_call_message_with_reasoning_value(
            normalized.get("content").and_then(Value::as_str),
            Some(&normalized_reasoning_or_placeholder(&normalized)),
            tool_call,
        );
    }

    normalized["tool_calls"] = Value::Array(filtered_tool_calls);
    normalized
}

/// 提取 assistant 消息中的 reasoning_content；为空或缺失时用占位文本兜底，
/// 保证 DeepSeek thinking 模式的工具调用回传始终非空。
/// 结构化 reasoning（数组）是合法值，原样保留。
fn normalized_reasoning_or_placeholder(normalized: &Value) -> Value {
    match normalized.get("reasoning_content") {
        Some(value @ Value::String(text)) if !text.trim().is_empty() => value.clone(),
        Some(Value::Null) | None => Value::String(TOOL_CALL_REASONING_PLACEHOLDER.to_string()),
        Some(Value::String(_)) => Value::String(TOOL_CALL_REASONING_PLACEHOLDER.to_string()),
        Some(other) => other.clone(),
    }
}

fn openai_tool_call_matches_executed_call(call: &Value, executed_call: &ToolCall) -> bool {
    if let Some(executed_call_id) = executed_call.call_id.as_deref() {
        return call.get("id").and_then(Value::as_str) == Some(executed_call_id);
    }

    call.get("function")
        .and_then(|function| function.get("name"))
        .and_then(Value::as_str)
        .map(openai_original_tool_name)
        .as_deref()
        == Some(executed_call.name.as_str())
}

fn openai_request_messages(request: &ProviderRequest) -> Vec<Value> {
    if !request.native_messages.is_empty() {
        return request.native_messages.clone();
    }

    let user_indexes = request
        .input
        .iter()
        .enumerate()
        .filter_map(|(index, message)| {
            if matches!(message.role, ProviderRole::User) {
                Some(index)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    let last_user_index = user_indexes.last().copied();

    request
        .input
        .iter()
        .enumerate()
        .map(|(index, message)| {
            // PA-174 F4：历史 assistant 轮在 `context.rs` 侧被降级成 `Developer` + 文本
            // 前缀；Anthropic 路径早已剥前缀还原成 `assistant`（见 `anthropic_chat_messages`），
            // OpenAI 路径此前不还原 ⇒ 历史退化成「一串 system 消息」。此处做**同一还原**，
            // 顺序不变；不带该前缀的 Developer / System 仍按 system 发送。
            if let Some(reply) = history_assistant_reply(message) {
                return json!({
                    "role": "assistant",
                    "content": reply
                });
            }
            if matches!(message.role, ProviderRole::Developer)
                && is_system_reminder_message(&message.content)
            {
                return json!({
                    "role": "user",
                    "content": message.content
                });
            }
            json!({
                "role": to_chat_role(&message.role),
                "content": openai_message_content(message.content.as_str(), is_last_user_with_images(index, last_user_index, &request.images), &request.images)
            })
        })
        .collect::<Vec<_>>()
}

/// PA-174 F4：历史 assistant 轮的降级形态是 `ProviderRole::Developer` + 前缀文本
/// （`context.rs` 的 `to_provider_history_message`）；返回**去前缀正文**，非该形态返回 None。
///
/// 前缀字面量与 Anthropic 路径（`anthropic_chat_messages`）的剥离点保持一致——两条协议
/// 必须还原出同一条 assistant 历史，否则同一份历史在 openai-completions / anthropic 下
/// 会走两套语义。
fn history_assistant_reply(message: &ProviderMessage) -> Option<&str> {
    if !matches!(message.role, ProviderRole::Developer) {
        return None;
    }
    message.content.strip_prefix("Previous assistant reply: ")
}

fn is_last_user_with_images(
    index: usize,
    last_user_index: Option<usize>,
    images: &[TurnInputImage],
) -> bool {
    !images.is_empty() && Some(index) == last_user_index
}

fn openai_message_content(content: &str, include_images: bool, images: &[TurnInputImage]) -> Value {
    if !include_images {
        return Value::String(content.to_string());
    }

    Value::Array(openai_user_content_blocks(content, images))
}

fn openai_user_content_blocks(content: &str, images: &[TurnInputImage]) -> Vec<Value> {
    let mut blocks = Vec::new();
    if !content.trim().is_empty() {
        blocks.push(json!({
            "type": "text",
            "text": content
        }));
    }

    blocks.extend(images.iter().map(|image| {
        json!({
            "type": "image_url",
            "image_url": {
                "url": image.data_url
            }
        })
    }));
    blocks
}

fn anthropic_user_content(content: &str, include_images: bool, images: &[TurnInputImage]) -> Value {
    if !include_images {
        return Value::String(content.to_string());
    }

    let mut blocks = Vec::new();
    if !content.trim().is_empty() {
        blocks.push(json!({
            "type": "text",
            "text": content
        }));
    }

    for image in images {
        let Some(base64_data) = image.base64_data() else {
            continue;
        };

        blocks.push(json!({
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": image.mime_type,
                "data": base64_data
            }
        }));
    }

    Value::Array(blocks)
}

pub fn provider_native_user_message(content: &str) -> Value {
    json!({
        "role": "user",
        "content": content
    })
}

pub fn provider_native_assistant_message(content: &str) -> Value {
    provider_native_assistant_message_with_reasoning(content, None)
}

pub fn provider_native_assistant_message_with_reasoning(
    content: &str,
    reasoning_content: Option<&str>,
) -> Value {
    provider_native_assistant_message_with_reasoning_value(
        content,
        reasoning_content
            .map(|value| Value::String(value.to_string()))
            .as_ref(),
    )
}

pub fn provider_native_assistant_message_with_reasoning_value(
    content: &str,
    reasoning_content: Option<&Value>,
) -> Value {
    json!({
        "role": "assistant",
        "content": content,
        "reasoning_content": reasoning_content
    })
}

pub fn provider_native_assistant_tool_call_message(
    content: Option<&str>,
    reasoning_content: Option<&str>,
    tool_call: &ToolCall,
) -> Value {
    provider_native_assistant_tool_call_message_with_reasoning_value(
        content,
        reasoning_content
            .map(|value| Value::String(value.to_string()))
            .as_ref(),
        tool_call,
    )
}

pub fn provider_native_assistant_tool_call_message_with_reasoning_value(
    content: Option<&str>,
    reasoning_content: Option<&Value>,
    tool_call: &ToolCall,
) -> Value {
    provider_native_assistant_tool_calls_message_with_reasoning_value(
        content,
        reasoning_content,
        std::slice::from_ref(tool_call),
    )
}

/// 批量版 OpenAI native assistant tool_calls 消息：单条 assistant 承载整批 `tool_calls`，
/// 不为并行调用克隆 N 份 `content` 与 `reasoning_content`。
pub fn provider_native_assistant_tool_calls_message_with_reasoning_value(
    content: Option<&str>,
    reasoning_content: Option<&Value>,
    tool_calls: &[ToolCall],
) -> Value {
    json!({
        "role": "assistant",
        "content": content,
        "reasoning_content": reasoning_content,
        "tool_calls": openai_tool_calls_payload(tool_calls),
    })
}

pub fn provider_native_assistant_tool_call_message_for_protocol(
    protocol: &ProviderProtocol,
    content: Option<&str>,
    reasoning_content: Option<&Value>,
    tool_call: &ToolCall,
) -> Value {
    provider_native_assistant_tool_calls_message_for_protocol(
        protocol,
        content,
        reasoning_content,
        std::slice::from_ref(tool_call),
    )
}

/// 批量版按协议构造 native assistant 工具调用消息：
/// - OpenAI：`role = "assistant"` + `tool_calls: [...]`；
/// - Anthropic：`role = "assistant"` + `[thinking?, text?, tool_use...]`。
pub fn provider_native_assistant_tool_calls_message_for_protocol(
    protocol: &ProviderProtocol,
    content: Option<&str>,
    reasoning_content: Option<&Value>,
    tool_calls: &[ToolCall],
) -> Value {
    if !protocol.is_anthropic() {
        return provider_native_assistant_tool_calls_message_with_reasoning_value(
            content,
            reasoning_content,
            tool_calls,
        );
    }

    let mut blocks = reasoning_content
        .map(anthropic_reasoning_blocks)
        .unwrap_or_default();
    if let Some(content) = content.filter(|content| !content.trim().is_empty()) {
        blocks.push(json!({
            "type": "text",
            "text": content
        }));
    }
    for tool_call in tool_calls {
        blocks.push(json!({
            "type": "tool_use",
            "id": tool_call.call_id.clone().unwrap_or_else(|| "toolu_local".to_string()),
            "name": tool_call.name,
            "input": tool_call.arguments
        }));
    }
    json!({
        "role": "assistant",
        "content": blocks
    })
}

pub fn provider_native_tool_result_message(
    tool_call: &ToolCall,
    tool_result: &ToolResult,
) -> Value {
    json!({
        "role": "tool",
        "tool_call_id": tool_call.call_id.clone().unwrap_or_else(|| "tool_call_local".to_string()),
        "content": tool_result.output.clone()
    })
}

pub fn provider_native_tool_result_message_for_protocol(
    protocol: &ProviderProtocol,
    tool_call: &ToolCall,
    tool_result: &ToolResult,
) -> Value {
    provider_native_tool_results_messages_for_protocol(protocol, &[(tool_call, tool_result)])
        .into_iter()
        .next()
        .unwrap_or(Value::Null)
}

/// 批量版按协议构造工具结果回填消息：
/// - OpenAI：每个 `(call, result)` 产出一条 `role = "tool"`；
/// - Anthropic：同一批所有 `tool_result` 聚合进**单条** `role = "user"` 消息，满足
///   `1 assistant(N tool_use) -> 1 user(N tool_result)` 配对约束。
pub fn provider_native_tool_results_messages_for_protocol(
    protocol: &ProviderProtocol,
    pairs: &[(&ToolCall, &ToolResult)],
) -> Vec<Value> {
    if pairs.is_empty() {
        return Vec::new();
    }
    if !protocol.is_anthropic() {
        return pairs
            .iter()
            .map(|(tool_call, tool_result)| provider_native_tool_result_message(tool_call, tool_result))
            .collect();
    }

    let blocks: Vec<Value> = pairs
        .iter()
        .map(|(tool_call, tool_result)| {
            json!({
                "type": "tool_result",
                "tool_use_id": tool_call.call_id.clone().unwrap_or_else(|| "toolu_local".to_string()),
                "content": anthropic_tool_result_content(tool_result),
                "is_error": tool_result.status != "ok"
            })
        })
        .collect();
    vec![json!({
        "role": "user",
        "content": blocks
    })]
}

fn anthropic_native_assistant_message(content: &[Value]) -> Value {
    json!({
        "role": "assistant",
        "content": content
    })
}

fn anthropic_native_assistant_message_from_parts(
    output_text: &str,
    reasoning_content: Option<&Value>,
) -> Value {
    let mut content = reasoning_content
        .map(anthropic_reasoning_blocks)
        .unwrap_or_default();
    if !output_text.trim().is_empty() {
        content.push(json!({
            "type": "text",
            "text": output_text
        }));
    }
    anthropic_native_assistant_message(&content)
}

fn anthropic_tool_result_content(tool_result: &ToolResult) -> Value {
    json!([{
        "type": "text",
        "text": tool_result.output
    }])
}

// 工具结果修剪预算的单一真源在 `crate::tool_pruner::PruneBudget`（ADR 0123），
// provider 同轮 follow-up 复用其 `PROVIDER_FOLLOWUP` 预设，禁止在此重复硬编码阈值。
const OPENAI_TOOL_RESULT_SUMMARY_PREVIEW_CHARS: usize = 240;
/// DeepSeek thinking 模式要求带 tool_calls 的 assistant 消息回传非空
/// reasoning_content（实测 API 只校验非空，不校验内容一致性）。
/// 本地 planner 决策等来源没有该字段时用此占位文本兜底。
const TOOL_CALL_REASONING_PLACEHOLDER: &str = "工具调用前的思考过程已省略。";
/// 短退避曲线的形状与「策略驱动」取舍：见 `config::UpstreamPolicy::short_backoff_config`。

/// PA-171 C3-S4/C3-P1：非 2xx 错误文本的**唯一装配点**。
///
/// `Retry-After` 以 [`crate::retry::RETRY_AFTER_MARKER`] 结构化后缀承载（解析同源，
/// 禁止第二处拼装）；冷却窗口与 TUI 文案都只消费这一份结构化时间。
fn provider_status_error(
    status: reqwest::StatusCode,
    elapsed_ms: u128,
    body: &str,
    retry_after: Option<&str>,
) -> String {
    // B139：状态码结构化留痕（唯一写入点）——per-call 记录据此把配额抖动样本筛出来。
    pony_protocol::telemetry::record_upstream_status(status.as_u16());
    let base = format!("provider 返回错误状态：{status}；耗时={elapsed_ms}ms；响应正文：{body}");
    match retry_after.and_then(parse_retry_after_header_seconds) {
        Some(secs) => crate::retry::append_retry_after_marker(&base, secs),
        None => base,
    }
}

/// `Retry-After` 头只接受秒数形态（HTTP-date 形态推迟，不影响冷却默认窗口）。
fn parse_retry_after_header_seconds(value: &str) -> Option<u64> {
    value.trim().parse::<u64>().ok()
}

/// PA-171 C3-P1：`Retry-After` 必须在 `response.text()` **之前**读取（text 消费响应体）。
fn response_retry_after_header(response: &reqwest::Response) -> Option<String> {
    response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

fn extract_provider_error_detail(err: &str) -> String {
    // 透传错误响应正文给前端，由前端根据 JSON 结构自适应展示
    if let Some(body_start) = err.find("响应正文：") {
        let body = &err[body_start + "响应正文：".len()..];
        return format!("{}", preview_text(body, 400));
    }
    preview_text(err, 240)
}

pub use pony_protocol::dsml::{display_dsml_protocol_text, sanitize_dsml_protocol_text, strip_dsml_protocol_text};


/// PA-100/PA-171：重试作用域。rate-limit 长退避只对 followup 生效——对话首跳
/// （decision/decision_stream）命中限流/配额时**首跳即 Abort**（C3-P2：0 次 sleep，
/// 冷却由 HTTP 出口登记），非限流瞬时错误仍走短退避快败进既有 fallback，
/// 避免用户在看到任何输出前先白等一分钟。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderRetryScope {
    Decision,
    Followup,
}

/// 基线默认包装（未注入策略的调用点/既有单测保持 `UpstreamPolicy::default()` 语义）。
#[cfg(test)]
fn provider_timeout_backoff_config() -> BackoffConfig {
    UpstreamPolicy::default().short_backoff_config()
}

/// rate-limit 专用调度（仅 Followup 作用域）：两次重试 sleep 20s→40s（budget 90s），
/// 覆盖分钟级 TPM 窗口的大部分。切换到该配置时重建 RetryBudget 并结转 elapsed_ms，
/// 指数基数以切换点为基准重置（见 retry_provider_scoped_with_sleeper）。
/// 口径说明（PA-100 code-review A P2-1）：budget 只计 sleep 时间（record_attempt），
/// 不含每次请求自身的执行耗时；最坏 wall-clock = 60s sleep + 各次 followup 的
/// provider 超时耗时，且同步路径无取消通道（既有限制）。
fn provider_rate_limit_backoff_config() -> BackoffConfig {
    BackoffConfig {
        max_retries: 2,
        initial_delay_ms: 20_000,
        multiplier: 2.0,
        max_delay_ms: 40_000,
        total_budget_ms: 90_000,
        jitter_kind: JitterKind::None,
    }
}

#[cfg(test)]
fn retry_provider_timeout<T, F>(label: &str, operation: F) -> Result<T, String>
where
    F: FnMut() -> Result<T, String>,
{
    retry_provider_timeout_with_policy(label, &UpstreamPolicy::default(), operation)
}

/// PA-172 §2.1：策略驱动的 decision 作用域重试（`ProviderManager` 内部唯一入口）。
///
/// 生产路径已改为 [`retry_provider_decision_stream_with_policy`]（带可见增量守卫，
/// B1795488338107 评审 B1）；本包装仅保留给既有单测的调度断言（无 hint 语义）。
#[cfg(test)]
fn retry_provider_timeout_with_policy<T, F>(
    label: &str,
    policy: &UpstreamPolicy,
    operation: F,
) -> Result<T, String>
where
    F: FnMut() -> Result<T, String>,
{
    retry_provider_scoped_with_sleeper_and_config(
        label,
        ProviderRetryScope::Decision,
        policy.short_backoff_config(),
        policy.retry_mode,
        operation,
        &StdThreadSleeper,
        None,
    )
}

/// B1795488338107 评审 B1：带 `no_more_retries_hint` 的 decision 流式重试包装。
///
/// 决策流式臂此前传 `None`（`decide_with_tools_stream_bounded`），意味着「已向调用方
/// 发出可见文本增量」这一既成事实对重试循环不可见——断流后重试会把半截增量与新
/// attempt 的输出重复上屏。与 followup 流式臂同一接线方式（不新增调度语义）。
fn retry_provider_decision_stream_with_policy<T, F>(
    label: &str,
    policy: &UpstreamPolicy,
    operation: F,
    no_more_retries_hint: &dyn Fn() -> bool,
) -> Result<T, String>
where
    F: FnMut() -> Result<T, String>,
{
    retry_provider_scoped_with_sleeper_and_config(
        label,
        ProviderRetryScope::Decision,
        policy.short_backoff_config(),
        policy.retry_mode,
        operation,
        &StdThreadSleeper,
        Some(no_more_retries_hint),
    )
}

/// PA-174 F2：把单次退避 sleep 夹到**注入的聚合 deadline** 剩余预算内。
///
/// 背景：聚合 deadline 若只参与「每次 attempt 的 HTTP 超时」，最后一段退避（最长 8s）仍会
/// 整段睡过 deadline，同步调用总墙钟被拖长（Test Agent 实测 8600ms 预算 → 11.77s 墙钟）。
/// 截断到 `min(delay, remaining)` 后，下一次 attempt 在发请求前即命中既有
/// 「剩余=0 ⇒ 不可重试 Abort」路径收尾。
///
/// `deadline == None` ⇒ 与 [`StdThreadSleeper`] **逐字等价**（既有调用点零变化）。
struct DeadlineSleeper {
    deadline: Option<Instant>,
}

impl Sleeper for DeadlineSleeper {
    fn sleep(
        &self,
        duration: Duration,
        cancelled: &std::sync::atomic::AtomicBool,
    ) -> Result<(), crate::retry::Cancelled> {
        let bounded = match self.deadline {
            Some(deadline) => duration.min(deadline.saturating_duration_since(Instant::now())),
            None => duration,
        };
        StdThreadSleeper.sleep(bounded, cancelled)
    }
}

/// PA-174 F2：decision 作用域的 deadline 版包装（`deadline=None` 等价既有包装）。
fn retry_provider_timeout_with_policy_deadline<T, F>(
    label: &str,
    policy: &UpstreamPolicy,
    deadline: Option<Instant>,
    operation: F,
) -> Result<T, String>
where
    F: FnMut() -> Result<T, String>,
{
    retry_provider_scoped_with_sleeper_and_config(
        label,
        ProviderRetryScope::Decision,
        policy.short_backoff_config(),
        policy.retry_mode,
        operation,
        &DeadlineSleeper { deadline },
        None,
    )
}

/// PA-174 F2：followup 作用域的 deadline 版包装（`deadline=None` 等价 PA-172 的无 deadline 语义）。
///
/// 无 deadline 的旧包装 `retry_provider_followup_timeout_with_policy` 已随 PA-174 的
/// 「所有同步入口都必须带聚合预算」收口而删除（调用点全部改为传 `deadline`）。
fn retry_provider_followup_timeout_with_policy_deadline<T, F>(
    label: &str,
    policy: &UpstreamPolicy,
    deadline: Option<Instant>,
    operation: F,
) -> Result<T, String>
where
    F: FnMut() -> Result<T, String>,
{
    retry_provider_scoped_with_sleeper_and_config(
        label,
        ProviderRetryScope::Followup,
        policy.short_backoff_config(),
        policy.retry_mode,
        operation,
        &DeadlineSleeper { deadline },
        None,
    )
}

/// 流式 followup 专用包装（PA-100 code-review B P1-2）：`no_more_retries_hint`
/// 返回 true 表示本次流已向调用方发出增量、任何后续重试都必然失败——循环在
/// sleep 前立即放弃，且禁止切换 rate-limit 长退避。否则"增量已发出后撞限流"
/// 会白睡 20s+40s，再经 sync fallback 又是 60s，单工具跳不可取消阻塞 >2 分钟。
/// PA-172 §2.1：策略驱动的流式 followup 重试（`ProviderManager` 内部唯一入口）。
fn retry_provider_followup_stream_timeout_with_policy<T, F>(
    label: &str,
    policy: &UpstreamPolicy,
    operation: F,
    no_more_retries_hint: &dyn Fn() -> bool,
) -> Result<T, String>
where
    F: FnMut() -> Result<T, String>,
{
    retry_provider_scoped_with_sleeper_and_config(
        label,
        ProviderRetryScope::Followup,
        policy.short_backoff_config(),
        policy.retry_mode,
        operation,
        &StdThreadSleeper,
        Some(no_more_retries_hint),
    )
}

#[cfg(test)]
fn retry_provider_scoped_with_sleeper<T, F>(
    label: &str,
    scope: ProviderRetryScope,
    operation: F,
    sleeper: &dyn Sleeper,
    no_retry_hint: Option<&dyn Fn() -> bool>,
) -> Result<T, String>
where
    F: FnMut() -> Result<T, String>,
{
    retry_provider_scoped_with_sleeper_and_config(
        label,
        scope,
        provider_timeout_backoff_config(),
        crate::retry::RetryMode::Normal,
        operation,
        sleeper,
        no_retry_hint,
    )
}

/// PA-172 §2.1：策略驱动版重试（配置由调用方给出；其余调度语义与旧入口逐字一致）。
fn retry_provider_scoped_with_sleeper_and_config<T, F>(
    label: &str,
    scope: ProviderRetryScope,
    config: BackoffConfig,
    mode: crate::retry::RetryMode,
    operation: F,
    sleeper: &dyn Sleeper,
    no_retry_hint: Option<&dyn Fn() -> bool>,
) -> Result<T, String>
where
    F: FnMut() -> Result<T, String>,
{
    // T2-B：旧入口委托带报告版本并丢弃报告，生产行为（重试/退避/日志/返回）零变。
    retry_provider_scoped_with_report_config(
        label,
        scope,
        config,
        mode,
        operation,
        sleeper,
        no_retry_hint,
    )
    .0
}

/// T2-B：单跳重试 + 观测报告。调度语义与旧函数完全一致，仅额外累计报告字段：
///
/// - `attempts_total` = `operation` 调用次数（含最终成功的一次）；
/// - `slept_ms` = sleep-only 累计（成功执行的 sleep 之和，不含请求自身耗时）；
/// - `switched_to_rl` = 本跳内是否切换过 rate-limit 长退避；
/// - `exhausted_reason` = 终态 `budget.exhausted_reason()`。
#[cfg(test)]
fn retry_provider_scoped_with_report<T, F>(
    label: &str,
    scope: ProviderRetryScope,
    operation: F,
    sleeper: &dyn Sleeper,
    no_retry_hint: Option<&dyn Fn() -> bool>,
) -> (Result<T, String>, SingleHopRetryReport)
where
    F: FnMut() -> Result<T, String>,
{
    retry_provider_scoped_with_report_config(
        label,
        scope,
        provider_timeout_backoff_config(),
        crate::retry::RetryMode::Normal,
        operation,
        sleeper,
        no_retry_hint,
    )
}

/// B1795506800001：**短窗口 429 的预算内等待**准入判定（唯一处）。
///
/// 准入必须**同时**满足（任一不满足 ⇒ 维持既有冷却短路 Abort）：
/// Followup 作用域 + `entered` 来源 + `rate_limited` + `0 < Retry-After <= SHORT_429_WAIT_MAX_MS`
/// + `window + 抖动余量` 装得进剩余预算 + 本跳等待次数未超上限。
fn eligible_short_429_wait_ms(
    scope: ProviderRetryScope,
    err: &str,
    short_429_waits: u32,
    budget: &RetryBudget,
) -> Option<u64> {
    if scope != ProviderRetryScope::Followup
        || crate::retry::cooldown_marker_source(err) != Some(crate::retry::CooldownMarkerSource::Entered)
        || crate::retry::classify_error_class(err) != "rate_limited"
        || short_429_waits >= crate::retry::SHORT_429_MAX_WAITS
    {
        return None;
    }
    crate::retry::parse_retry_after_marker(err)
        .map(|secs| secs.saturating_mul(1_000))
        .filter(|window| *window > 0 && *window <= crate::retry::SHORT_429_WAIT_MAX_MS)
        .map(|window| window + crate::retry::COOLDOWN_EXPIRY_SLACK_MS)
        .filter(|wait| *wait <= budget.total_budget_ms.saturating_sub(budget.elapsed_ms))
}

struct ScopedRetryDecisionCtx<'a> {
    attempts_total: u32,
    budget: &'a mut RetryBudget,
    active_config: &'a mut BackoffConfig,
    rl_active: &'a mut bool,
    attempt: &'a mut u32,
    retry_notify: &'a crate::retry::RetryNotifier,
}

/// 单次失败的退避判决：按需切换 RL 曲线并调用 `decide`，返回下一次 sleep 毫秒数（`None` 表示 Abort）。
fn decide_scoped_retry_next_delay(
    label: &str,
    scope: ProviderRetryScope,
    mode: crate::retry::RetryMode,
    err: &str,
    failure: &crate::retry::FailureKind,
    ctx: &mut ScopedRetryDecisionCtx<'_>,
) -> Option<u64> {
    let use_rate_limit =
        scope == ProviderRetryScope::Followup && failure.is_retryable() && is_rate_limit_error(err);
    let retry_after = if failure.is_retryable() {
        crate::retry::retry_after_from_error(err)
    } else {
        None
    };

    if use_rate_limit && !*ctx.rl_active {
        *ctx.rl_active = true;
        *ctx.active_config = provider_rate_limit_backoff_config();
        let mut carried =
            RetryBudget::new(ctx.active_config.max_retries, ctx.active_config.total_budget_ms);
        carried.elapsed_ms = ctx.budget.elapsed_ms;
        *ctx.budget = carried;
        *ctx.attempt = 1;
        let rl_policy = ProviderRetryPolicy::new(*ctx.active_config).with_mode(mode);
        let decision = rl_policy.decide(
            failure,
            ctx.budget,
            *ctx.attempt,
            retry_after.as_ref(),
            StreamState::NoDelta,
        );
        provider_log(format!(
            "{label}:attempt={} failed class={failure:?} \
             decision={decision:?} scope={scope:?} switch=rate_limit",
            ctx.attempts_total
        ));
        return match decision {
            RetryDecision::Retry { delay_ms } => {
                ctx.retry_notify
                    .notify(ctx.attempts_total, ctx.active_config, err, delay_ms);
                Some(delay_ms)
            }
            _ => None,
        };
    }

    let active_policy = ProviderRetryPolicy::new(*ctx.active_config).with_mode(mode);
    // `decide` 的 `attempt` 是 1-based 下一次尝试序号；`attempt` 在普通分支后自增。
    let decision = active_policy.decide(
        failure,
        ctx.budget,
        ctx.attempt.saturating_add(1),
        retry_after.as_ref(),
        StreamState::NoDelta,
    );
    provider_log(format!(
        "{}:attempt={}/{} failed class={:?} decision={:?} scope={:?}",
        label,
        ctx.attempts_total,
        ctx.active_config.max_retries + 1,
        failure,
        decision,
        scope
    ));
    match decision {
        RetryDecision::Retry { delay_ms } => {
            ctx.retry_notify
                .notify(ctx.attempts_total, ctx.active_config, err, delay_ms);
            *ctx.attempt += 1;
            Some(delay_ms)
        }
        _ => None,
    }
}

/// PA-172 §2.1：策略驱动版单跳重试（`base_config` 由调用方给出，其余语义逐字不变）。
fn retry_provider_scoped_with_report_config<T, F>(
    label: &str,
    scope: ProviderRetryScope,
    base_config: BackoffConfig,
    mode: crate::retry::RetryMode,
    mut operation: F,
    sleeper: &dyn Sleeper,
    no_retry_hint: Option<&dyn Fn() -> bool>,
) -> (Result<T, String>, SingleHopRetryReport)
where
    F: FnMut() -> Result<T, String>,
{
    let scope_label: &'static str = match scope {
        ProviderRetryScope::Decision => "decision",
        ProviderRetryScope::Followup => "followup",
    };
    let report = |budget: &RetryBudget,
                  attempts_total: u32,
                  slept_ms: u64,
                  switched_to_rl: bool|
     -> SingleHopRetryReport {
        SingleHopRetryReport {
            attempts_total,
            slept_ms,
            switched_to_rl,
            scope: scope_label,
            exhausted_reason: budget.exhausted_reason(),
        }
    };
    let policy = ProviderRetryPolicy::new(base_config).with_mode(mode);
    let mut budget = RetryBudget::new(base_config.max_retries, base_config.total_budget_ms);
    let mut active_config = base_config;
    let mut rl_active = false;
    let mut attempt = 0_u32;
    let mut attempts_total = 0_u32;
    let mut slept_ms = 0_u64;
    let cancelled = std::sync::atomic::AtomicBool::new(false);
    let mut last_failure: Option<String> = None;
    let mut pending_delay_ms: Option<u64> = None;
    let mut short_429_waits = 0_u32;
    let retry_notify = crate::retry::RetryNotifier::new(label, scope_label);

    loop {
        if attempt > 0 {
            if no_retry_hint.is_some_and(|hint| hint()) {
                let terminal =
                    last_failure.unwrap_or_else(|| "retry aborted by caller".to_string());
                return (
                    Err(terminal),
                    report(&budget, attempts_total, slept_ms, rl_active),
                );
            }
            let delay = match pending_delay_ms.take() {
                Some(delay_ms) => Duration::from_millis(delay_ms),
                None => compute_delay(attempt, &active_config, 0.5),
            };
            if sleeper.sleep(delay, &cancelled).is_err() {
                return (
                    Err(format!("{label}: retry sleep cancelled")),
                    report(&budget, attempts_total, slept_ms, rl_active),
                );
            }
            let slept = delay.as_millis() as u64;
            budget.record_attempt(slept);
            slept_ms += slept;
            crate::retry::note_provider_attempt_backoff_slept(slept);
            if let Some(last) = last_failure.as_deref() {
                retry_notify.notify_started(attempts_total, &active_config, last);
            }
        }
        let attempt_started_at = Instant::now();
        let attempt_result = operation();
        let attempt_elapsed_ms = attempt_started_at.elapsed().as_millis() as u64;
        match attempt_result {
            Ok(value) => {
                crate::retry::note_provider_attempt_succeeded(
                    attempt_started_at,
                    attempt_elapsed_ms,
                );
                return (
                    Ok(value),
                    report(&budget, attempts_total + 1, slept_ms, rl_active),
                );
            }
            Err(err) => {
                attempts_total += 1;
                crate::retry::note_provider_attempt_failed(attempt_elapsed_ms);
                let last_error = extract_provider_error_detail(&err);
                last_failure = Some(err.clone());
                let failure = policy.classify(&err);
                if no_retry_hint.is_some_and(|hint| hint()) {
                    return (
                        Err(last_error),
                        report(&budget, attempts_total, slept_ms, rl_active),
                    );
                }
                let short_429_wait_ms =
                    eligible_short_429_wait_ms(scope, &err, short_429_waits, &budget);
                if crate::retry::is_cooldown_active_error(&err) && short_429_wait_ms.is_none() {
                    return (
                        Err(last_error),
                        report(&budget, attempts_total, slept_ms, rl_active),
                    );
                }
                if let Some(wait_ms) = short_429_wait_ms {
                    short_429_waits += 1;
                    provider_log(format!(
                        "{label}:attempt={attempts_total} cooldown_entered_short_window=true \
                         short_429_wait_ms={wait_ms} waits_used={short_429_waits} \
                         scope={scope:?} wait_then_retry"
                    ));
                    pending_delay_ms = Some(wait_ms);
                    retry_notify.notify(attempts_total, &active_config, &err, wait_ms);
                    attempt += 1;
                    continue;
                }
                if scope == ProviderRetryScope::Decision
                    && crate::retry::is_rate_limit_cooldown_error(&err)
                {
                    return (
                        Err(last_error),
                        report(&budget, attempts_total, slept_ms, rl_active),
                    );
                }
                match decide_scoped_retry_next_delay(
                    label,
                    scope,
                    mode,
                    &err,
                    &failure,
                    &mut ScopedRetryDecisionCtx {
                        attempts_total,
                        budget: &mut budget,
                        active_config: &mut active_config,
                        rl_active: &mut rl_active,
                        attempt: &mut attempt,
                        retry_notify: &retry_notify,
                    },
                ) {
                    Some(delay_ms) => pending_delay_ms = Some(delay_ms),
                    None => {
                        return (
                            Err(last_error),
                            report(&budget, attempts_total, slept_ms, rl_active),
                        );
                    }
                }
            }
        }
    }
}

fn openai_followup_tool_result_message(tool_call: &ToolCall, tool_result: &ToolResult) -> Value {
    json!({
        "role": "tool",
        "tool_call_id": tool_call.call_id.clone().unwrap_or_else(|| "tool_call_local".to_string()),
        "content": openai_followup_tool_result_content(tool_result)
    })
}

fn openai_followup_tool_result_content(tool_result: &ToolResult) -> String {
    let Some((head, tail, original_chars)) =
        split_head_tail(&tool_result.output, PruneBudget::PROVIDER_FOLLOWUP)
    else {
        return tool_result.output.clone();
    };

    let included_head_chars = head.chars().count();
    let included_tail_chars = tail.chars().count();
    let total_lines = tool_result.output.lines().count();
    let payload = json!({
        "tool_name": tool_result.tool_name,
        "status": tool_result.status,
        "compression": {
            "applied": true,
            "strategy": "head_tail_excerpt",
            "reason": "provider_followup_size_guard",
            "original_chars": original_chars,
            "original_lines": total_lines,
            "included_head_chars": included_head_chars,
            "included_tail_chars": included_tail_chars,
            "omitted_chars": original_chars.saturating_sub(included_head_chars + included_tail_chars)
        },
        "summary": summarize_large_tool_output(&tool_result.output),
        "head": head,
        "tail": tail
    });

    serde_json::to_string(&payload).unwrap_or_else(|_| {
        format!(
            "{{\"tool_name\":\"{}\",\"status\":\"{}\",\"compression\":{{\"applied\":true,\"reason\":\"provider_followup_size_guard\",\"original_chars\":{}}}}}",
            escape_json_fragment(&tool_result.tool_name),
            escape_json_fragment(&tool_result.status),
            original_chars
        )
    })
}

fn local_tool_followup_summary(tool_result: &ToolResult) -> String {
    if let Ok(value) = serde_json::from_str::<Value>(&tool_result.output) {
        if let Some(summary_text) = value
            .get("summary")
            .and_then(|summary| summary.get("text"))
            .and_then(Value::as_str)
        {
            return summary_text.to_string();
        }

        if let Some(plan_summary) = value
            .get("plan")
            .and_then(|plan| plan.get("summary"))
            .and_then(Value::as_str)
        {
            return plan_summary.to_string();
        }

        return summarize_json_value(&value).to_string();
    }

    format!(
        "status={}；tool={}；chars={}",
        tool_result.status,
        tool_result.tool_name,
        tool_result.output.chars().count()
    )
}

fn summarize_large_tool_output(output: &str) -> Value {
    if let Ok(value) = serde_json::from_str::<Value>(output) {
        return summarize_json_value(&value);
    }

    let first_non_empty_line = output
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("");

    json!({
        "kind": "text",
        "first_line_preview": preview_text(first_non_empty_line, OPENAI_TOOL_RESULT_SUMMARY_PREVIEW_CHARS)
    })
}

fn summarize_json_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => json!({
            "kind": "json_object",
            "key_count": map.len(),
            "keys": map.keys().take(12).cloned().collect::<Vec<_>>()
        }),
        Value::Array(items) => json!({
            "kind": "json_array",
            "length": items.len(),
            "first_item_kind": items.first().map(json_value_kind).unwrap_or("empty")
        }),
        Value::String(_) => json!({ "kind": "json_string" }),
        Value::Number(_) => json!({ "kind": "json_number" }),
        Value::Bool(_) => json!({ "kind": "json_boolean" }),
        Value::Null => json!({ "kind": "json_null" }),
    }
}

fn json_value_kind(value: &Value) -> &'static str {
    match value {
        Value::Object(_) => "object",
        Value::Array(_) => "array",
        Value::String(_) => "string",
        Value::Number(_) => "number",
        Value::Bool(_) => "boolean",
        Value::Null => "null",
    }
}

fn escape_json_fragment(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

fn anthropic_messages_with_tool_result(
    request: &ProviderRequest,
    accumulated_messages: &mut Vec<Value>,
    assistant_message: Option<&Value>,
    tool_call: &ToolCall,
    tool_result: &ToolResult,
) -> Vec<Value> {
    let mut messages = if accumulated_messages.is_empty() {
        anthropic_conversation_messages(request)
    } else {
        accumulated_messages.clone()
    };
    let assistant_content =
        anthropic_assistant_content_for_tool_result(assistant_message, tool_call);

    messages.push(json!({
        "role": "assistant",
        "content": assistant_content
    }));

    messages.push(json!({
        "role": "user",
        "content": [
            {
                "type": "tool_result",
                "tool_use_id": tool_call.call_id.clone().unwrap_or_else(|| "toolu_local".to_string()),
                "content": anthropic_tool_result_content(tool_result),
                "is_error": tool_result.status != "ok"
            }
        ]
    }));

    *accumulated_messages = messages.clone();

    messages
}

// ───────────────────────── D1b（P1-1 切片一）：批量工具结果的**纯函数**回填构造器 ─────────────
//
// 为什么单列一套（而不是改旧 1:1 构造器）：旧路径被 8 个 provider 入口与两条运行时循环直接调用，
// 改签名等于把「批量执行」的风险提前注入当前行为。这里只**新增**纯函数，供后续波次的批量执行
// 调用；单调用路径继续走旧构造器，行为逐字不变。
//
// 回填形态（唯一会让上游 400 的地方，务必按此）：
// - OpenAI：1 条 assistant（**全部** tool_calls）+ N 条 `role="tool"`（各带 tool_call_id）；
// - Anthropic：1 条 assistant（**全部** tool_use）+ 1 条 `role="user"`，其 content 为 N 个
//   `tool_result` 块。**绝不**为每个调用各发一条 assistant。

/// OpenAI 的 `tool_calls` 数组载荷（保序，index 按位置补齐）。
fn openai_tool_calls_payload(calls: &[ToolCall]) -> Vec<Value> {
    calls
        .iter()
        .map(|call| {
            json!({
                "id": call.call_id.clone().unwrap_or_else(|| "tool_call_local".to_string()),
                "type": "function",
                "function": {
                    "name": openai_safe_tool_name(&call.name),
                    "arguments": call.arguments.to_string()
                }
            })
        })
        .collect()
}

/// 批量版 assistant 消息：**一条**消息承载全部 `tool_calls`。
fn openai_assistant_message_for_tool_results(
    assistant_message: Option<&Value>,
    calls: &[ToolCall],
) -> Value {
    let Some(message) = assistant_message else {
        return json!({
            "role": "assistant",
            "content": Value::Null,
            "reasoning_content": Value::String(TOOL_CALL_REASONING_PLACEHOLDER.to_string()),
            "tool_calls": openai_tool_calls_payload(calls)
        });
    };

    let mut normalized = normalize_openai_assistant_message(message);
    let matched: Vec<Value> = normalized
        .get("tool_calls")
        .and_then(Value::as_array)
        .map(|raw| {
            raw.iter()
                .filter(|item| {
                    calls
                        .iter()
                        .any(|call| openai_tool_call_matches_executed_call(item, call))
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default();

    // 上游原始消息已含**全部**待回填调用 ⇒ 原样（去掉无关/未执行的调用，防串号）。
    if matched.len() == calls.len() && !calls.is_empty() {
        normalized["tool_calls"] = Value::Array(matched);
        normalized["role"] = Value::String("assistant".to_string());
        return normalized;
    }

    // 否则按执行列表重建（顺序 = 执行顺序）。
    json!({
        "role": "assistant",
        "content": normalized.get("content").cloned().unwrap_or(Value::Null),
        "reasoning_content": normalized_reasoning_or_placeholder(&normalized),
        "tool_calls": openai_tool_calls_payload(calls)
    })
}

/// D1b：构造 **1 assistant + N tool 消息** 的 OpenAI 回填序列（纯函数，不触网、无全局态）。
///
/// - `messages`：已经解析好的基础消息（原样前置，函数不读 `ProviderRequest`）；
/// - `assistant`：上游 assistant 消息（含 tool_calls；缺失时按 `calls` 合成）；
/// - 长度不匹配（`calls.len() != results.len()`）⇒ `Err`（消息里带两个真实长度）。
pub fn openai_messages_with_tool_results(
    messages: &[Value],
    assistant: &Value,
    calls: &[ToolCall],
    results: &[ToolResult],
) -> Result<Vec<Value>, String> {
    if calls.len() != results.len() {
        return Err(format!(
            "tool_calls/tool_results length mismatch: calls={} results={}",
            calls.len(),
            results.len()
        ));
    }

    let mut out: Vec<Value> = messages.to_vec();
    out.push(openai_assistant_message_for_tool_results(
        Some(assistant),
        calls,
    ));
    for (call, result) in calls.iter().zip(results.iter()) {
        out.push(openai_followup_tool_result_message(call, result));
    }
    Ok(out)
}

/// Anthropic 批量版 assistant content：一条 assistant 消息的 content 含**全部** tool_use 块。
fn anthropic_assistant_content_for_tool_results(
    assistant_message: Option<&Value>,
    calls: &[ToolCall],
) -> Vec<Value> {
    let all_ids: Vec<&str> = calls
        .iter()
        .map(|call| call.call_id.as_deref().unwrap_or("toolu_local"))
        .collect();

    // 上游 content 已含全部 tool_use 块 ⇒ 原样使用（保留 text/thinking 块与 id）。
    if let Some(content) = assistant_message
        .and_then(|message| message.get("content"))
        .and_then(Value::as_array)
        .filter(|content| {
            all_ids.iter().all(|id| {
                content.iter().any(|block| {
                    block.get("type").and_then(Value::as_str) == Some("tool_use")
                        && block.get("id").and_then(Value::as_str) == Some(*id)
                })
            })
        })
    {
        return content.clone();
    }

    let mut content = assistant_message
        .and_then(|message| message.get("reasoning_content"))
        .map(anthropic_reasoning_blocks)
        .unwrap_or_default();
    if let Some(text) = assistant_message
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
    {
        content.push(json!({ "type": "text", "text": text }));
    }
    if content.is_empty() {
        content.push(json!({ "type": "text", "text": "" }));
    }
    for call in calls {
        content.push(json!({
            "type": "tool_use",
            "id": call.call_id.clone().unwrap_or_else(|| "toolu_local".to_string()),
            "name": call.name,
            "input": call.arguments
        }));
    }
    content
}

/// D1b：构造 **1 assistant（全部 tool_use）+ 1 user（N 个 tool_result）** 的 Anthropic 回填序列。
///
/// 红线：**绝不能**为每个调用各发一条 assistant（上游会以「同内容重复 + tool_use id 失配」400）。
/// 长度不匹配 ⇒ `Err`（消息里带两个真实长度）。
pub fn anthropic_messages_with_tool_results(
    messages: &[Value],
    assistant: &Value,
    calls: &[ToolCall],
    results: &[ToolResult],
) -> Result<Vec<Value>, String> {
    if calls.len() != results.len() {
        return Err(format!(
            "tool_calls/tool_results length mismatch: calls={} results={}",
            calls.len(),
            results.len()
        ));
    }

    let mut out: Vec<Value> = messages.to_vec();
    out.push(json!({
        "role": "assistant",
        "content": anthropic_assistant_content_for_tool_results(Some(assistant), calls)
    }));
    let blocks: Vec<Value> = calls
        .iter()
        .zip(results.iter())
        .map(|(call, result)| {
            json!({
                "type": "tool_result",
                "tool_use_id": call.call_id.clone().unwrap_or_else(|| "toolu_local".to_string()),
                "content": anthropic_tool_result_content(result),
                "is_error": result.status != "ok"
            })
        })
        .collect();
    out.push(json!({ "role": "user", "content": blocks }));
    Ok(out)
}

/// 将非流式回填响应按 `reasoning -> text` 顺序回放为增量事件（用于 `!supports_streaming` 分支）。
fn replay_sync_response_as_deltas<F>(response: &ProviderResponse, on_delta: &mut F)
where
    F: FnMut(ProviderStreamChunk),
{
    if let Some(reasoning_content) = response.reasoning_content.clone() {
        on_delta(ProviderStreamChunk::Reasoning(reasoning_content));
    }
    on_delta(ProviderStreamChunk::Text(response.output_text.clone()));
}

/// 为 OpenAI 流式 followup（1:1 与批量共用）构造标准化消息转录并更新 `accumulated_messages`。
fn build_openai_followup_stream_messages(
    request: &ProviderRequest,
    accumulated_messages: &mut Vec<Value>,
    assistant_message: Option<&Value>,
    calls: &[ToolCall],
    results: &[ToolResult],
) -> Result<Vec<Value>, String> {
    if calls.len() == 1 && results.len() == 1 {
        Ok(normalize_openai_messages(openai_messages_with_tool_result(
            request,
            accumulated_messages,
            assistant_message,
            &calls[0],
            &results[0],
        )))
    } else {
        openai_followup_messages_for_tool_results(
            request,
            accumulated_messages,
            assistant_message,
            calls,
            results,
        )
    }
}

/// 按流式响应中的完整 `tool_calls` 列表（无则回落单值）构造 OpenAI native assistant 消息。
fn build_openai_stream_assistant_message(
    output_text: &str,
    reasoning_content_value: Option<&Value>,
    tool_call: Option<&ToolCall>,
    tool_calls: &[ToolCall],
) -> Value {
    if !tool_calls.is_empty() {
        provider_native_assistant_tool_calls_message_with_reasoning_value(
            text_if_present(output_text),
            reasoning_content_value,
            tool_calls,
        )
    } else if let Some(call) = tool_call {
        provider_native_assistant_tool_call_message_with_reasoning_value(
            text_if_present(output_text),
            reasoning_content_value,
            call,
        )
    } else {
        provider_native_assistant_message_with_reasoning_value(output_text, reasoning_content_value)
    }
}

/// D2：批量回填的 OpenAI chat 转录（基础转录 + 1 assistant + N tool），并写回累积转录。
///
/// 基础转录口径与 1:1 路径一致：`accumulated_messages` 为空时用请求自带的消息，
/// 否则在其后追加；构造完成后 `accumulated_messages` 更新为本次完整转录。
fn openai_followup_messages_for_tool_results(
    request: &ProviderRequest,
    accumulated_messages: &mut Vec<Value>,
    assistant_message: Option<&Value>,
    calls: &[ToolCall],
    results: &[ToolResult],
) -> Result<Vec<Value>, String> {
    let base = if accumulated_messages.is_empty() {
        openai_request_messages(request)
    } else {
        accumulated_messages.clone()
    };
    // D1b 构造器要求 `&Value`：缺失 assistant 时用 `Null` 触发「按 calls 重建」分支
    // （与 1:1 路径 `assistant_message=None` 的合成形态一致）。
    let assistant = assistant_message.cloned().unwrap_or(Value::Null);
    let messages = normalize_openai_messages(openai_messages_with_tool_results(
        &base,
        &assistant,
        calls,
        results,
    )?);
    *accumulated_messages = messages.clone();
    Ok(messages)
}

/// D2：批量回填的 Anthropic 转录（基础转录 + 1 assistant 含全部 tool_use + 1 user 含 N 个
/// `tool_result`），并写回累积转录。基础转录口径与 1:1 路径一致。
fn anthropic_followup_messages_for_tool_results(
    request: &ProviderRequest,
    accumulated_messages: &mut Vec<Value>,
    assistant_message: Option<&Value>,
    calls: &[ToolCall],
    results: &[ToolResult],
) -> Result<Vec<Value>, String> {
    let base = if accumulated_messages.is_empty() {
        anthropic_conversation_messages(request)
    } else {
        accumulated_messages.clone()
    };
    let assistant = assistant_message.cloned().unwrap_or(Value::Null);
    let messages = anthropic_messages_with_tool_results(&base, &assistant, calls, results)?;
    *accumulated_messages = messages.clone();
    Ok(messages)
}

fn anthropic_assistant_content_for_tool_result(
    assistant_message: Option<&Value>,
    tool_call: &ToolCall,
) -> Vec<Value> {
    let call_id = tool_call.call_id.as_deref().unwrap_or("toolu_local");
    if let Some(content) = assistant_message
        .and_then(|message| message.get("content"))
        .and_then(Value::as_array)
        .filter(|content| {
            content.iter().any(|block| {
                block.get("type").and_then(Value::as_str) == Some("tool_use")
                    && block.get("id").and_then(Value::as_str) == Some(call_id)
            })
        })
    {
        return content.clone();
    }

    let mut content = assistant_message
        .and_then(|message| message.get("reasoning_content"))
        .map(anthropic_reasoning_blocks)
        .unwrap_or_default();
    if let Some(text) = assistant_message
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
    {
        content.push(json!({
            "type": "text",
            "text": text
        }));
    }
    if content.is_empty() {
        content.push(json!({
            "type": "text",
            "text": ""
        }));
    }
    content.push(json!({
        "type": "tool_use",
        "id": tool_call.call_id.clone().unwrap_or_else(|| "toolu_local".to_string()),
        "name": tool_call.name,
        "input": tool_call.arguments
    }));
    content
}

fn anthropic_reasoning_blocks(reasoning_value: &Value) -> Vec<Value> {
    match reasoning_value {
        Value::Array(items) => items
            .iter()
            .filter_map(|item| {
                if item.get("type").and_then(Value::as_str) == Some("thinking") {
                    return Some(item.clone());
                }

                extract_reasoning_text_from_value(item).map(|text| {
                    json!({
                        "type": "thinking",
                        "thinking": text,
                    })
                })
            })
            .collect(),
        Value::String(text) if !text.trim().is_empty() => vec![json!({
            "type": "thinking",
            "thinking": text
        })],
        _ => extract_reasoning_text_from_value(reasoning_value)
            .filter(|text| !text.trim().is_empty())
            .map(|text| {
                vec![json!({
                    "type": "thinking",
                    "thinking": text
                })]
            })
            .unwrap_or_default(),
    }
}

#[allow(dead_code)]
fn extract_chat_output_text(payload: &Value) -> Option<String> {
    let message = payload
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))?;
    let content = message.get("content")?;

    match content {
        Value::String(text) => Some(text.clone()),
        Value::Array(items) => {
            let parts = items
                .iter()
                .filter_map(|item| {
                    if let Some(text) = item.get("text").and_then(Value::as_str) {
                        return Some(text.to_string());
                    }
                    item.get("text")
                        .and_then(|value| value.get("value"))
                        .and_then(Value::as_str)
                        .map(|text| text.to_string())
                })
                .collect::<Vec<_>>();

            if parts.is_empty() {
                None
            } else {
                Some(parts.join("\n"))
            }
        }
        _ => None,
    }
}

fn extract_openai_message_reasoning_content(message: &Value) -> Option<String> {
    extract_openai_message_reasoning_value(message)
        .as_ref()
        .and_then(extract_reasoning_text_from_value)
}

fn extract_openai_message_reasoning_value(message: &Value) -> Option<Value> {
    message.get("reasoning_content").cloned()
}

fn extract_reasoning_text_from_value(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.to_string()),
        Value::Array(items) => {
            let parts = items
                .iter()
                .filter_map(|item| {
                    if let Some(text) = item.get("text").and_then(Value::as_str) {
                        return Some(text.to_string());
                    }
                    item.get("text")
                        .and_then(|nested| nested.get("value"))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .collect::<Vec<_>>();

            if parts.is_empty() {
                None
            } else {
                Some(parts.join(""))
            }
        }
        _ => None,
    }
}

fn openai_original_tool_name(name: &str) -> String {
    name.replace("__dot__", ".")
}

fn extract_anthropic_output_text(payload: &Value) -> Option<String> {
    let parts = payload
        .get("content")?
        .as_array()?
        .iter()
        .filter_map(|item| item.get("text").and_then(Value::as_str))
        .map(|text| text.to_string())
        .collect::<Vec<_>>();

    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n"))
    }
}

fn extract_anthropic_usage(payload: &Value) -> Option<TokenUsage> {
    let usage = payload
        .get("usage")
        .or_else(|| {
            payload
                .get("message")
                .and_then(|message| message.get("usage"))
        })
        .or_else(|| payload.get("delta").and_then(|delta| delta.get("usage")))?;

    let cache_hit = usage
        .get("cache_read_input_tokens")
        .and_then(Value::as_u64)
        .map(|v| (v, "usage.cache_read_input_tokens"));
    // wave-54 C4：anthropic 的 `input_tokens` 既不含 cache_read 也不含 cache_creation，
    // 后者仅参与 context_input 归一（不计入 cache 命中上报）。
    let cache_creation_input_tokens = usage
        .get("cache_creation_input_tokens")
        .and_then(Value::as_u64);

    Some(normalize_token_usage(TokenUsage {
        input_tokens: usage.get("input_tokens").and_then(Value::as_u64),
        cache_hit_input_tokens: cache_hit.map(|(v, _)| v),
        cache_hit_source: cache_hit.map(|(_, s)| s.to_string()),
        reasoning_tokens: None,
        output_tokens: usage.get("output_tokens").and_then(Value::as_u64),
        total_tokens: usage.get("total_tokens").and_then(Value::as_u64),
        cache_creation_input_tokens,
        // wave-54 C3：上游 usage 真值。
        origin: Some(USAGE_ORIGIN_PROVIDER),
    }))
}

#[allow(dead_code)]
fn extract_anthropic_delta_text(payload: &Value) -> Option<String> {
    let delta = payload.get("delta")?;
    let delta_type = delta.get("type").and_then(Value::as_str)?;

    if delta_type != "text_delta" {
        return None;
    }

    delta
        .get("text")
        .and_then(Value::as_str)
        .map(|text| text.to_string())
}

fn estimate_token_usage(request: &ProviderRequest, output_text: &str) -> TokenUsage {
    let input_chars = if !request.native_messages.is_empty() {
        request
            .native_messages
            .iter()
            .map(estimate_native_message_chars)
            .sum::<usize>()
    } else {
        request
            .input
            .iter()
            .map(|message| message.content.chars().count())
            .sum::<usize>()
    };
    let output_chars = output_text.chars().count();

    normalize_token_usage(TokenUsage {
        input_tokens: Some(estimate_tokens_from_chars(input_chars)),
        cache_hit_input_tokens: None,
        cache_hit_source: None,
        reasoning_tokens: None,
        output_tokens: Some(estimate_tokens_from_chars(output_chars)),
        total_tokens: None,
        cache_creation_input_tokens: None,
        // wave-54 C3：估算值必须可辨识（UI 以 `≈` 呈现，不得冒充真值）。
        origin: Some(USAGE_ORIGIN_ESTIMATED_CHARS),
    })
}

fn estimate_tokens_from_chars(char_count: usize) -> u64 {
    if char_count == 0 {
        0
    } else {
        char_count.div_ceil(4) as u64
    }
}

fn estimate_native_message_chars(message: &Value) -> usize {
    if let Some(content) = message.get("content").and_then(Value::as_str) {
        return content.chars().count();
    }

    if let Some(content) = message.get("content").and_then(Value::as_array) {
        return content.iter().map(estimate_native_content_chars).sum();
    }

    message.to_string().chars().count()
}

fn estimate_native_content_chars(block: &Value) -> usize {
    if block.get("type").and_then(Value::as_str) == Some("image")
        || block.get("type").and_then(Value::as_str) == Some("image_url")
    {
        return 1024;
    }

    if let Some(text) = block.get("text").and_then(Value::as_str) {
        return text.chars().count();
    }

    if let Some(text) = block
        .get("text")
        .and_then(|value| value.get("value"))
        .and_then(Value::as_str)
    {
        return text.chars().count();
    }

    block.to_string().chars().count()
}

fn render_input_messages(messages: &[ProviderMessage]) -> String {
    messages
        .iter()
        .enumerate()
        .map(|(index, message)| {
            format!(
                "[{}] {}\n{}",
                index,
                to_chat_role(&message.role),
                message.content.trim()
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn render_native_messages(messages: &[Value]) -> String {
    messages
        .iter()
        .enumerate()
        .map(|(index, message)| {
            let role = message
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let content = match message.get("content") {
                Some(Value::String(text)) => text.trim().to_string(),
                Some(other) => {
                    serde_json::to_string_pretty(other).unwrap_or_else(|_| other.to_string())
                }
                None => message.to_string(),
            };
            format!("[{}] {}\n{}", index, role, content)
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn render_tool_definitions(tools: &[ToolDefinition]) -> String {
    if tools.is_empty() {
        return "none".to_string();
    }

    let contract_views = surface_contract_views(tools);

    contract_views
        .iter()
        .enumerate()
        .map(|(index, tool)| {
            format!(
                "[{}] {}\ndescription: {}\nschema:\n{}",
                index,
                tool.name,
                tool.description,
                serde_json::to_string_pretty(&tool.input_schema)
                    .unwrap_or_else(|_| tool.input_schema.to_string())
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn derive_request_observation(request: &ProviderRequest) -> ProviderRequestObservation {
    if !request.native_messages.is_empty() {
        derive_native_request_observation(&request.native_messages)
    } else {
        derive_input_request_observation(&request.input)
    }
}

fn derive_input_request_observation(messages: &[ProviderMessage]) -> ProviderRequestObservation {
    if messages.is_empty() {
        return ProviderRequestObservation::default();
    }

    let stable_prefix_end = messages
        .iter()
        .enumerate()
        .take_while(|(index, message)| is_stable_input_prefix_message(*index, message))
        .count()
        .min(messages.len().saturating_sub(1));
    let volatile_input_start = messages
        .iter()
        .rposition(|message| matches!(message.role, ProviderRole::User))
        .unwrap_or(messages.len().saturating_sub(1));

    ProviderRequestObservation {
        stable_prefix_text: render_input_messages(&messages[..stable_prefix_end]),
        semi_stable_context_text: render_input_messages(
            &messages[stable_prefix_end..volatile_input_start],
        ),
        volatile_input_text: render_input_messages(&messages[volatile_input_start..]),
        prefix_mutation_reasons: Vec::new(),
        context_refresh_reason: None,
        instruction_scope_sources: Vec::new(),
        conversation_carry_mode: None,
    }
}

fn derive_native_request_observation(messages: &[Value]) -> ProviderRequestObservation {
    if messages.is_empty() {
        return ProviderRequestObservation::default();
    }

    let stable_prefix_end = messages
        .iter()
        .enumerate()
        .take_while(|(index, message)| is_stable_native_prefix_message(*index, message))
        .count()
        .min(messages.len().saturating_sub(1));
    let volatile_input_start = messages
        .iter()
        .rposition(|message| {
            message.get("role").and_then(Value::as_str) == Some("user")
                && !message
                    .get("content")
                    .and_then(Value::as_str)
                    .is_some_and(is_system_reminder_message)
        })
        .unwrap_or_else(|| {
            messages
                .iter()
                .rposition(|message| message.get("role").and_then(Value::as_str) == Some("user"))
                .unwrap_or(messages.len().saturating_sub(1))
        });

    ProviderRequestObservation {
        stable_prefix_text: render_native_messages(&messages[..stable_prefix_end]),
        semi_stable_context_text: render_native_messages(
            &messages[stable_prefix_end..volatile_input_start],
        ),
        volatile_input_text: render_native_messages(&messages[volatile_input_start..]),
        prefix_mutation_reasons: Vec::new(),
        context_refresh_reason: None,
        instruction_scope_sources: Vec::new(),
        conversation_carry_mode: None,
    }
}

fn is_stable_input_prefix_message(index: usize, message: &ProviderMessage) -> bool {
    match message.role {
        ProviderRole::System => index == 0 || !looks_like_dynamic_prefix_content(&message.content),
        ProviderRole::Developer => !looks_like_dynamic_prefix_content(&message.content),
        ProviderRole::User => false,
    }
}

fn is_stable_native_prefix_message(index: usize, message: &Value) -> bool {
    if message.get("role").and_then(Value::as_str) != Some("system") {
        return false;
    }

    let content = match message.get("content") {
        Some(Value::String(text)) => text,
        _ => return index == 0,
    };

    index == 0 || !looks_like_dynamic_prefix_content(content)
}

fn looks_like_dynamic_prefix_content(content: &str) -> bool {
    if content.starts_with("Capability profile:") {
        return false;
    }

    [
        "Session summary:",
        "Run goal:",
        "Long-term memory status:",
        "Older context was truncated",
        "supportsImageInput=false for this model.",
        "This model is marked as image-capable.",
        "The user appears to reference image content",
        "graph=",
        "session=",
    ]
    .iter()
    .any(|marker| content.contains(marker))
}

fn provider_log(message: String) {
    eprintln!("[pony-provider] {}", message);
}

/// Stage3 P1–P5 协议/上游可观测前缀（error_code 注册表线码，String wire 兼容）。
/// P6 门禁：本文件内不新增 `lower.contains`/`to_ascii_lowercase`（分类仍走 retry 码表）。
// Stage3 协议/上游可观测包装器五件套：本任务(P1-P5)仅落符号，调用点由 task-7 接线，
// 故当前 crate 内未引用，临时 allow(dead_code) 通过 CI deny-warnings；task-7 接线后移除本标注。
#[allow(dead_code)]
pub const PROTOCOL_SSE_PARSE_CODE: &str = "protocol_sse_parse";
#[allow(dead_code)]
pub const PROTOCOL_TOOL_CALL_SCHEMA_CODE: &str = "protocol_tool_call_schema";
#[allow(dead_code)]
pub const PROTOCOL_MODEL_CATALOG_CODE: &str = "protocol_model_catalog";
#[allow(dead_code)]
pub const PROTOCOL_UNKNOWN_MODEL_CODE: &str = "protocol_unknown_model";
#[allow(dead_code)]
pub const PROTOCOL_REASONING_EFFORT_CODE: &str = "protocol_reasoning_effort";

/// Stage3 P2：SSE 失败可观测包装——code=protocol_sse_parse，
/// message 含原始错误 + endpoint/parsed_bytes/elapsed 证据。fallback 链不动
/// （调用方仍按 `Err(String)` 走原有 sync/stream 回退）。
#[allow(dead_code)]
pub fn protocol_sse_parse_error(
    detail: &str,
    endpoint: &str,
    parsed_bytes: usize,
    elapsed_ms: u64,
) -> String {
    format!(
        "{}: {}; endpoint={}; parsed_bytes={}; elapsed={}ms",
        PROTOCOL_SSE_PARSE_CODE, detail, endpoint, parsed_bytes, elapsed_ms
    )
}

/// Stage3 P3：tool_call schema 不匹配可观测包装——code=protocol_tool_call_schema。
/// 消灭静默 default `{}`：坏 JSON / 多调 / 空输出一律显式错误，由调用方走既有回退链。
#[allow(dead_code)]
pub fn protocol_tool_call_schema_error(detail: &str, evidence: &str) -> String {
    format!(
        "{}: {}; evidence={}",
        PROTOCOL_TOOL_CALL_SCHEMA_CODE,
        detail,
        preview_text(evidence, 400)
    )
}

/// Stage3 P4：model catalog 可观测包装——code=protocol_model_catalog。
#[allow(dead_code)]
pub fn protocol_model_catalog_error(detail: &str, evidence: &str) -> String {
    format!(
        "{}: {}; evidence={}",
        PROTOCOL_MODEL_CATALOG_CODE,
        detail,
        preview_text(evidence, 400)
    )
}

/// Stage3 P4：选择期未知模型可观测包装——code=protocol_unknown_model。
/// provider/ 内无选择期校验点（选择在 config 层），本函数为调用方预留构造器，
/// 不改变现有选择流程。
#[allow(dead_code)]
pub fn protocol_unknown_model_error(model: &str, known_count: usize) -> String {
    format!(
        "{}: 未知模型 `{}`（已知 {} 个候选）；evidence=model={}",
        PROTOCOL_UNKNOWN_MODEL_CODE,
        preview_text(model, 120),
        known_count,
        preview_text(model, 120)
    )
}

/// Stage3 P5：reasoning effort 冲突本地报错构造器——code=protocol_reasoning_effort。
#[allow(dead_code)]
pub fn protocol_reasoning_effort_error(detail: &str, evidence: &str) -> String {
    format!(
        "{}: {}; evidence={}",
        PROTOCOL_REASONING_EFFORT_CODE,
        detail,
        preview_text(evidence, 200)
    )
}

fn provider_log_token_usage(context: &str, usage: &TokenUsage) {
    provider_log(format!(
        "usage:{} input={:?} cache_hit_input={:?} cache_hit_source={:?} reasoning={:?} output={:?} total={:?}",
        context,
        usage.input_tokens,
        usage.cache_hit_input_tokens,
        usage.cache_hit_source,
        usage.reasoning_tokens,
        usage.output_tokens,
        usage.total_tokens
    ));
}

fn preview_text(text: &str, max_chars: usize) -> String {
    let normalized = text.replace('\n', "\\n");
    let count = normalized.chars().count();
    if count <= max_chars {
        normalized
    } else {
        let preview = normalized.chars().take(max_chars).collect::<String>();
        format!("{}...(+{} chars)", preview, count - max_chars)
    }
}

fn preview_json(value: &Value, max_chars: usize) -> String {
    match serde_json::to_string(value) {
        Ok(text) => preview_text(&text, max_chars),
        Err(error) => format!("json-serialize-error: {}", error),
    }
}

#[cfg(test)]
mod tests;
