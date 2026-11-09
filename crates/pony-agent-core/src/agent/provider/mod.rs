use crate::agent::config::{
    ProviderReasoningEffort, ResolvedProviderSelection, ThinkingParamPattern, UpstreamPolicy,
};
use crate::agent::input::TurnInputImage;
use crate::agent::retry::{
    compute_delay, is_rate_limit_error, BackoffConfig, JitterKind, ProviderRetryPolicy,
    RetryBudget, RetryDecision, SingleHopRetryReport, Sleeper, StdThreadSleeper, StreamState,
};
use crate::agent::runtime_helper::block_on;
use crate::agent::tools::{builtin_tool_surface, ToolCall, ToolDefinition, ToolResult};
use futures_util::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProviderProtocol {
    #[serde(rename = "openai-completions", alias = "openai")]
    OpenAiCompletions,
    #[serde(rename = "openai-responses")]
    OpenAiResponses,
    #[serde(rename = "anthropic-messages", alias = "anthropic")]
    AnthropicMessages,
}

impl ProviderProtocol {
    /// Anthropic Messages 家族判断（行为分叉位点统一入口）。
    pub fn is_anthropic(&self) -> bool {
        matches!(self, ProviderProtocol::AnthropicMessages)
    }

    /// OpenAI 家族判断：completions 与 responses 同待遇
    /// （真流式、native tool flow、Bearer 默认鉴权等门控共用）。
    pub fn is_openai_family(&self) -> bool {
        matches!(
            self,
            ProviderProtocol::OpenAiCompletions | ProviderProtocol::OpenAiResponses
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderAuthType {
    Auto,
    Bearer,
    #[serde(rename = "x-api-key")]
    XApiKey,
}

impl Default for ProviderAuthType {
    fn default() -> Self {
        Self::Auto
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderRole {
    System,
    Developer,
    User,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderMessage {
    pub role: ProviderRole,
    pub content: String,
}

impl ProviderMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: ProviderRole::System,
            content: content.into(),
        }
    }

    pub fn developer(content: impl Into<String>) -> Self {
        Self {
            role: ProviderRole::Developer,
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: ProviderRole::User,
            content: content.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderRequest {
    pub model: String,
    pub input: Vec<ProviderMessage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<TurnInputImage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub native_messages: Vec<Value>,
    #[serde(default)]
    pub observation: ProviderRequestObservation,
    pub temperature: f32,
    pub max_output_tokens: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderRequestObservation {
    #[serde(default)]
    pub stable_prefix_text: String,
    #[serde(default)]
    pub semi_stable_context_text: String,
    #[serde(default)]
    pub volatile_input_text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prefix_mutation_reasons: Vec<PrefixMutationReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_refresh_reason: Option<ContextRefreshReason>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub instruction_scope_sources: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_carry_mode: Option<ConversationCarryMode>,
}

impl ProviderRequestObservation {
    fn is_empty(&self) -> bool {
        self.stable_prefix_text.is_empty()
            && self.semi_stable_context_text.is_empty()
            && self.volatile_input_text.is_empty()
            && self.prefix_mutation_reasons.is_empty()
            && self.context_refresh_reason.is_none()
            && self.instruction_scope_sources.is_empty()
            && self.conversation_carry_mode.is_none()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildContextObservation {
    pub request_format: String,
    pub message_count: usize,
    pub image_count: usize,
    pub tool_count: usize,
    pub temperature: f32,
    pub max_output_tokens: u32,
    #[serde(default)]
    pub stable_prefix_text: String,
    #[serde(default)]
    pub semi_stable_context_text: String,
    #[serde(default)]
    pub volatile_input_text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prefix_mutation_reasons: Vec<PrefixMutationReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_refresh_reason: Option<ContextRefreshReason>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub instruction_scope_sources: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_carry_mode: Option<ConversationCarryMode>,
    pub request_messages_text: String,
    pub tool_definitions_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrefixMutationReason {
    SessionSummaryChanged,
    RunGoalChanged,
    LongTermMemoryChanged,
    PlannerSkillsChanged,
    ImageNoteChanged,
    TruncationNoteChanged,
    HistoryBoundaryShifted,
    NativeTranscriptBoundaryShifted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextRefreshReason {
    InitialBuild,
    SessionSummaryChanged,
    RunGoalChanged,
    LongTermMemoryChanged,
    PlannerSkillsChanged,
    ImageNoteChanged,
    TruncationNoteChanged,
    HistoryBoundaryShifted,
    NativeTranscriptBoundaryShifted,
    WorkspaceScopeChanged,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationCarryMode {
    FullReplay,
    ProviderNativeTranscriptReplay,
    ProviderContinuationPreferred,
    ProviderContinuationFallbackReplay,
    CompactedReplay,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderResponse {
    pub output_text: String,
    pub tool_call: Option<ToolCall>,
    /// D1a（P1-1 切片一）：本次响应里的**全部**工具调用（顺序即上游顺序）。
    ///
    /// 不变量：`tool_calls` 与 `tool_call` 永远一致 —— 单值字段是首元素的派生视图
    /// （单调用路径逐字不变；批量执行在后续波次接这个字段）。
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
    pub reasoning_content: Option<String>,
    pub reasoning_content_value: Option<Value>,
    pub assistant_message: Option<Value>,
    pub provider_source: String,
    pub provider_mode: String,
    pub fallback_reason: Option<String>,
    pub token_usage: Option<TokenUsage>,
}

/// D1a：由单值 `tool_call` 派生复数视图（保证两字段永远一致）。
pub fn tool_calls_from_single(tool_call: &Option<ToolCall>) -> Vec<ToolCall> {
    tool_call.iter().cloned().collect()
}

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderDecision {
    pub output_text: String,
    pub tool_call: Option<ToolCall>,
    /// D1a（P1-1 切片一）：本次响应里的**全部**工具调用（顺序即上游顺序）。
    ///
    /// 不变量：`tool_calls` 与 `tool_call` 永远一致 —— 单值字段是首元素的派生视图
    /// （单调用路径逐字不变；批量执行在后续波次接这个字段）。
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
    pub reasoning_content: Option<String>,
    pub reasoning_content_value: Option<Value>,
    pub assistant_message: Option<Value>,
    pub provider_source: String,
    pub provider_mode: String,
    pub fallback_reason: Option<String>,
    pub token_usage: Option<TokenUsage>,
}

#[derive(Debug, Clone)]
struct AnthropicStreamMessage {
    output_text: String,
    tool_call: Option<ToolCall>,
    reasoning_content: Option<String>,
    reasoning_content_value: Option<Value>,
    token_usage: Option<TokenUsage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderStreamChunk {
    /// 首个上游流事件（**不含内容**）：仅承载「首包已到达」的时点信号（ADR 0055 P0-2）。
    ///
    /// 存在理由：纯工具调用轮次没有 text / reasoning 增量，此前 `on_delta` 一次都不触发，
    /// 于是 `llm.call.ttft_ms` 缺失、`generation_ms` 退化成整段耗时，速度被系统性低估
    /// （实测 trace `f47fd028`：15 次调用中 6 次无 ttft，整轮速度显示 31.8 tok/s，
    /// 按真实首包口径约 52 tok/s）。该变体**不得**被当作内容消费，消费者必须忽略。
    StreamStart,
    Text(String),
    Reasoning(String),
}

/// usage 来源标记：上游 usage 真值（协议解析得到）。
pub const USAGE_ORIGIN_PROVIDER: &str = "provider";
/// usage 来源标记：本地字符数估算（**不是**真值，UI 必须以 `≈` 呈现）。
pub const USAGE_ORIGIN_ESTIMATED_CHARS: &str = "estimated_chars";

/// wave-54 C3：把来源字符串归一为静态标记（未知/自定义值 ⇒ `None`，禁止冒充）。
pub fn usage_origin_from_label(label: &str) -> Option<&'static str> {
    match label {
        USAGE_ORIGIN_PROVIDER => Some(USAGE_ORIGIN_PROVIDER),
        USAGE_ORIGIN_ESTIMATED_CHARS => Some(USAGE_ORIGIN_ESTIMATED_CHARS),
        _ => None,
    }
}

/// token 用量（响应侧）。
///
/// wave-54 C3：`origin` 以 `&'static str` 静态标记承载（`provider` | `estimated_chars`），
/// 该类型无法由 serde derive 反序列化（`'de` 需为 `'static`），故 `Serialize`/`Deserialize`
/// 手写：wire 上 `origin` 为字符串，反序列化经 [`usage_origin_from_label`] 归一为静态标记。
#[derive(Debug, Clone, Default)]
pub struct TokenUsage {
    pub input_tokens: Option<u64>,
    pub cache_hit_input_tokens: Option<u64>,
    pub cache_hit_source: Option<String>,
    pub reasoning_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    /// wave-54 C4（anthropic 归一）：`cache_creation_input_tokens` **仅**用于
    /// `context_input_tokens`（anthropic 的 `input` 既不含 cache_read 也不含 cache_creation）。
    /// 语义边界：**不并入** `cache_hit_input_tokens`，**不**进 `gen_ai.usage.cache_read.*`。
    pub cache_creation_input_tokens: Option<u64>,
    /// wave-54 C3：usage 来源。`None` ⇒ 未知/未标注（估算值不得静默冒充真值）。
    pub origin: Option<&'static str>,
}

impl serde::Serialize for TokenUsage {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        // 字段数与出现条件与既有 wire 形态保持一致（cache_hit_source/origin 缺失即缺键）。
        let mut state = serializer.serialize_struct("TokenUsage", 9)?;
        state.serialize_field("inputTokens", &self.input_tokens)?;
        state.serialize_field("cacheHitInputTokens", &self.cache_hit_input_tokens)?;
        if self.cache_hit_source.is_some() {
            state.serialize_field("cacheHitSource", &self.cache_hit_source)?;
        }
        state.serialize_field("reasoningTokens", &self.reasoning_tokens)?;
        state.serialize_field("outputTokens", &self.output_tokens)?;
        state.serialize_field("totalTokens", &self.total_tokens)?;
        if self.cache_creation_input_tokens.is_some() {
            state.serialize_field("cacheCreationInputTokens", &self.cache_creation_input_tokens)?;
        }
        if self.origin.is_some() {
            state.serialize_field("origin", &self.origin)?;
        }
        state.end()
    }
}

impl<'de> serde::Deserialize<'de> for TokenUsage {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct TokenUsageWire {
            #[serde(default)]
            input_tokens: Option<u64>,
            #[serde(default)]
            cache_hit_input_tokens: Option<u64>,
            #[serde(default)]
            cache_hit_source: Option<String>,
            #[serde(default)]
            reasoning_tokens: Option<u64>,
            #[serde(default)]
            output_tokens: Option<u64>,
            #[serde(default)]
            total_tokens: Option<u64>,
            #[serde(default)]
            cache_creation_input_tokens: Option<u64>,
            #[serde(default)]
            origin: Option<String>,
        }

        let wire = TokenUsageWire::deserialize(deserializer)?;
        Ok(TokenUsage {
            input_tokens: wire.input_tokens,
            cache_hit_input_tokens: wire.cache_hit_input_tokens,
            cache_hit_source: wire.cache_hit_source,
            reasoning_tokens: wire.reasoning_tokens,
            output_tokens: wire.output_tokens,
            total_tokens: wire.total_tokens,
            cache_creation_input_tokens: wire.cache_creation_input_tokens,
            origin: wire.origin.as_deref().and_then(usage_origin_from_label),
        })
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
    crate::agent::otel::log::log_json_with_fields(
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
pub(crate) fn cooldown_key_for(selection: &ResolvedProviderSelection) -> String {
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

pub struct ProviderManager {
    config: ResolvedProviderSelection,
    /// PA-172 §2.1：上游调用策略；`new()` 时恒为 `UpstreamPolicy::default()`（G2）。
    policy: UpstreamPolicy,
    /// PA-172 §3.1：换手链；`new()` 时为 `None`（语义冻结：既有构造点零改动）。
    failover: Option<ProviderFailoverChain>,
    /// PA-172 §3.3 F-c（L3-A/L3-P B2）：换手预算的**绝对截止时刻**。只有换手候选的
    /// manager 会带它 ⇒ 每次 attempt 的 HTTP 超时都在发请求前按 `min(policy, remaining)`
    /// 重算（逐 attempt deadline），而不是每次 hop 只算一次。
    attempt_deadline: Option<Instant>,
}

impl ProviderManager {
    pub fn new(config: ResolvedProviderSelection) -> Self {
        Self {
            config,
            policy: UpstreamPolicy::default(),
            failover: None,
            attempt_deadline: None,
        }
    }

    /// PA-172 §3.3 F-c：给换手候选注入绝对截止时刻（主 provider 不带 ⇒ 主跳预算不变）。
    fn with_attempt_deadline(mut self, deadline: Instant) -> Self {
        self.attempt_deadline = Some(deadline);
        self
    }

    /// PA-172 §3.3 F-c：本次 attempt 的请求超时（毫秒）。
    ///
    /// 逐 attempt 重算：`min(policy.requestTimeoutMs, remaining_ms)`，唯一取值出口是
    /// [`crate::agent::retry::failover_attempt_timeout_ms`]（禁止第二处 `min`）。
    /// 返回 `0` ⇒ 预算已耗尽，调用方必须**立即** Abort（0 次 HTTP）。
    fn effective_attempt_timeout_ms(&self) -> u64 {
        let Some(deadline) = self.attempt_deadline else {
            return self.policy.request_timeout_ms;
        };
        let remaining_ms = deadline
            .saturating_duration_since(Instant::now())
            .as_millis() as u64;
        crate::agent::retry::failover_attempt_timeout_ms(
            self.policy.request_timeout_ms,
            remaining_ms,
        )
    }

    /// F-c：预算耗尽的结构化错误（必须**不可重试** ⇒ 重试编排立即 Abort、不再白睡）。
    fn failover_budget_exhausted_error() -> String {
        "failover budget exhausted before attempt; remaining=0ms; no request sent".to_string()
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
        crate::agent::otel::log::log_json_with_fields(
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
        Err(crate::agent::retry::append_cooldown_active_marker(&format!(
            "provider {} 处于限流冷却中（{class}），预计 {secs}s 后恢复；本次未发起请求；retry_after={secs}",
            self.config.provider_name
        )))
    }

    /// PA-171 C3-P1/C3-P6：命中限流/配额才登记冷却（其余类别保留既有重试语义）。
    /// 返回 `true` = 本次确实进入/刷新了冷却（调用方据此给错误文本打冷却标记）。
    fn enter_cooldown(&self, error: &str) -> bool {
        let class = crate::agent::retry::classify_error_class(error);
        if !matches!(class, "rate_limited" | "quota") {
            return false;
        }
        let retry_after_s = crate::agent::retry::parse_retry_after_marker(error);
        let window_ms = crate::agent::retry::rate_limit_window_ms(
            error,
            crate::agent::retry::utc_now_hm(),
        )
        .unwrap_or(COOLDOWN_DEFAULT_MS)
        .clamp(1_000, COOLDOWN_MAX_MS);
        let until_unix_ms = now_unix_ms() + window_ms;
        // nfr-baseline-wave68 `wave68_logging_events`：进入冷却的结构化留痕
        // （事件名逐字；字段 = 声明清单，禁止夹带响应正文/密钥）。
        crate::agent::otel::log::log_json_with_fields(
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
        let (result, raw_error) = self.decide_with_tools_single_hop(request, tools);
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
        // 未装链 ⇒ 与基线逐字一致（G2：默认零行为变更），仍返回既有
        // `extract_provider_error_detail` 处理后的文本。
        let Some(chain) = self.failover.as_ref() else {
            return Err(primary_detail);
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
        if !crate::agent::retry::failover_eligible(verdict_source) {
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
                crate::agent::otel::log::log_json_with_fields(
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
            crate::agent::otel::log::log_json_with_fields(
                "warn",
                "provider_failover_switched",
                None,
                &[
                    ("from_provider", serde_json::json!(self.config.provider_name)),
                    ("to_provider", serde_json::json!(candidate.provider_name)),
                    (
                        "error_class",
                        serde_json::json!(crate::agent::retry::classify_error_class(
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
        crate::agent::otel::log::log_json_with_fields(
            "warn",
            "provider_failover_exhausted",
            None,
            &[
                ("hops", serde_json::json!(hops)),
                (
                    "last_error_class",
                    serde_json::json!(crate::agent::retry::classify_error_class(&last_error)),
                ),
                ("attempted_providers", serde_json::json!(attempted)),
            ],
        );
        Err(crate::agent::retry::append_failover_exhausted_marker(
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
        let result = retry_provider_timeout_with_policy("decision", &self.policy, || {
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
        });

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
        mut on_delta: F,
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

        let mut streamed_any_delta = false;
        let result = retry_provider_timeout_with_policy("decision_stream", &self.policy, || {
            if streamed_any_delta {
                // PA-100：措辞刻意不含 timeout/rate-limit 特征——classify 判不可重试，
                // 退避循环立即 Abort，不再为必然失败的哨兵白睡（原 ≤7.5s 浪费归零）。
                return Err("stream already emitted delta; auto-retry forbidden".to_string());
            }

            match self.config.protocol {
                ProviderProtocol::OpenAiCompletions => self
                    .send_openai_tool_decision_stream_request(guard, request, tools, &mut |chunk| {
                        streamed_any_delta = true;
                        on_delta(chunk);
                    }),
                ProviderProtocol::OpenAiResponses => self
                    .send_responses_tool_decision_stream_request(guard, request, tools, &mut |chunk| {
                        streamed_any_delta = true;
                        on_delta(chunk);
                    }),
                ProviderProtocol::AnthropicMessages => {
                    self.send_anthropic_tool_decision_request(request, tools)
                }
            }
        });

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

        match self.config.protocol {
            ProviderProtocol::OpenAiCompletions => {
                retry_provider_followup_timeout_with_policy("followup_sync", &self.policy, || {
                    self.send_openai_tool_followup_request(
                        request,
                        tools,
                        accumulated_messages,
                        assistant_message,
                        tool_call,
                        tool_result,
                    )
                })
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
                retry_provider_followup_timeout_with_policy("followup_sync", &self.policy, || {
                    self.send_responses_tool_followup_request(
                        request,
                        tools,
                        accumulated_messages,
                        assistant_message,
                        tool_call,
                        tool_result,
                    )
                })
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
            ProviderProtocol::AnthropicMessages => self.send_anthropic_tool_followup_request(
                request,
                tools,
                accumulated_messages,
                assistant_message,
                tool_call,
                tool_result,
            ),
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
            if let Some(reasoning_content) = response.reasoning_content.clone() {
                on_delta(ProviderStreamChunk::Reasoning(reasoning_content));
            }
            on_delta(ProviderStreamChunk::Text(response.output_text.clone()));
            return Ok(response);
        }

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
                let protocol_label = match self.config.protocol {
                    ProviderProtocol::OpenAiResponses => "openai-responses",
                    _ => "openai-completions",
                };
                // PA-100（code-review B P1-2）：流已发出增量后任何重试都必然失败，
                // 用共享 AtomicBool 让重试循环在 sleep 前放弃、并禁止 RL 长退避切换。
                let streamed_any_delta = std::sync::atomic::AtomicBool::new(false);
                let mark_streamed =
                    || streamed_any_delta.store(true, std::sync::atomic::Ordering::Relaxed);
                let stream_result = retry_provider_followup_stream_timeout_with_policy(
                    "followup_stream",
                    &self.policy,
                    || {
                        if streamed_any_delta.load(std::sync::atomic::Ordering::Relaxed) {
                            // 措辞刻意不含 timeout/rate-limit 特征：classify 应判为
                            // 不可重试，避免被退避循环消费。
                            return Err(
                                "stream already emitted delta; auto-retry forbidden".to_string()
                            );
                        }

                        let stream_fn_result = match self.config.protocol {
                            ProviderProtocol::OpenAiResponses => self
                                .send_responses_tool_followup_stream_request(
                                    guard,
                                    request,
                                    tools,
                                    accumulated_messages,
                                    assistant_message,
                                    tool_call,
                                    tool_result,
                                    &mut |chunk| {
                                        mark_streamed();
                                        on_delta(chunk);
                                    },
                                ),
                            _ => self.send_openai_tool_followup_stream_request(
                                guard,
                                request,
                                tools,
                                accumulated_messages,
                                assistant_message,
                                tool_call,
                                tool_result,
                                &mut |chunk| {
                                    mark_streamed();
                                    on_delta(chunk);
                                },
                            ),
                        };
                        stream_fn_result
                    },
                    &|| streamed_any_delta.load(std::sync::atomic::Ordering::Relaxed),
                );
                match stream_result {
                    Ok(response) => Ok(response),
                    Err(stream_error) => {
                        provider_log(format!(
                            "followup:stream-fallback protocol={} provider={} model={} reason={}",
                            protocol_label, self.config.provider_name, request.model, stream_error
                        ));
                        // stream→sync 回退与本地 fallback 语义对齐 completions 分支。
                        let response = match retry_provider_followup_timeout_with_policy(
                            "followup_sync_fallback",
                            &self.policy,
                            || match self.config.protocol {
                                ProviderProtocol::OpenAiResponses => self
                                    .send_responses_tool_followup_request(
                                        request,
                                        tools,
                                        accumulated_messages,
                                        assistant_message,
                                        tool_call,
                                        tool_result,
                                    ),
                                _ => self.send_openai_tool_followup_request(
                                    request,
                                    tools,
                                    accumulated_messages,
                                    assistant_message,
                                    tool_call,
                                    tool_result,
                                ),
                            },
                        ) {
                            Ok(response) => Ok(response),
                            Err(sync_error) => {
                                provider_log(format!(
                                    "followup:stream-error protocol={} provider={} model={} reason={}",
                                    protocol_label, self.config.provider_name, request.model, sync_error
                                ));
                                Ok(local_tool_followup_fallback_response(
                                    request,
                                    tool_call,
                                    tool_result,
                                    sync_error,
                                ))
                            }
                        };
                        response
                    }
                }
            }
            ProviderProtocol::AnthropicMessages => self
                .send_anthropic_tool_followup_stream_request(
                    guard,
                    request,
                    tools,
                    accumulated_messages,
                    assistant_message,
                    tool_call,
                    tool_result,
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
            ProviderProtocol::OpenAiCompletions => retry_provider_followup_timeout_with_policy(
                "followup_sync_batch",
                &self.policy,
                || {
                    *accumulated_messages = base_messages.clone();
                    self.send_openai_tool_followup_batch_request(
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
            ProviderProtocol::OpenAiResponses => retry_provider_followup_timeout_with_policy(
                "followup_sync_batch",
                &self.policy,
                || {
                    *accumulated_messages = base_messages.clone();
                    self.send_responses_tool_followup_batch_request(
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
            ProviderProtocol::AnthropicMessages => retry_provider_followup_timeout_with_policy(
                "followup_sync_batch",
                &self.policy,
                || {
                    *accumulated_messages = base_messages.clone();
                    self.send_anthropic_tool_followup_batch_request(
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

    /// D2：批量工具结果 followup（流式入口）。分派顺序与同步入口逐字一致。
    ///
    /// N>1 的传输沿用 §6 指定的既有同步 `post_*` 通道（不新写 HTTP 层），整段结果按
    /// 「reasoning → text」顺序回放为增量——与既有 `!supports_streaming` 分支同一语义
    /// （`continue_with_tool_result_stream_bounded`），因此批量路径**不是**逐 token 增量。
    #[allow(clippy::too_many_arguments)]
    pub fn continue_with_tool_results_stream<F>(
        &self,
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
            return self.continue_with_tool_result_stream(
                request,
                tools,
                accumulated_messages,
                assistant_message,
                &calls[0],
                &results[0],
                on_delta,
            );
        }

        let response = self.continue_with_tool_results(
            request,
            tools,
            accumulated_messages,
            assistant_message,
            calls,
            results,
        )?;
        if let Some(reasoning_content) = response.reasoning_content.clone() {
            on_delta(ProviderStreamChunk::Reasoning(reasoning_content));
        }
        on_delta(ProviderStreamChunk::Text(response.output_text.clone()));
        Ok(response)
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

    fn send_openai_tool_followup_stream_request<F>(
        &self,
        guard: &ProviderStreamGuard<'_>,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
        accumulated_messages: &mut Vec<Value>,
        assistant_message: Option<&Value>,
        tool_call: &ToolCall,
        tool_result: &ToolResult,
        on_delta: &mut F,
    ) -> Result<ProviderResponse, String>
    where
        F: FnMut(ProviderStreamChunk),
    {
        let endpoint = format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        );
        provider_log(format!(
            "request:openai followup-stream endpoint={} model={} tool={}",
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
                "request:openai followup-stream tool_call_id={} messages={}",
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

    fn send_anthropic_tool_followup_stream_request<F>(
        &self,
        guard: &ProviderStreamGuard<'_>,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
        accumulated_messages: &mut Vec<Value>,
        assistant_message: Option<&Value>,
        tool_call: &ToolCall,
        tool_result: &ToolResult,
        on_delta: &mut F,
    ) -> Result<ProviderResponse, String>
    where
        F: FnMut(ProviderStreamChunk),
    {
        let endpoint = format!("{}/messages", self.config.base_url.trim_end_matches('/'));
        provider_log(format!(
            "request:anthropic followup-stream endpoint={} model={} tool={}",
            endpoint, request.model, tool_call.name
        ));
        let body = with_anthropic_request_options(
            json!({
                "model": request.model,
                "system": anthropic_system_text(&request.input),
                "messages": anthropic_messages_with_tool_result(request, accumulated_messages, assistant_message, tool_call, tool_result),
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

    fn send_responses_tool_followup_stream_request<F>(
        &self,
        guard: &ProviderStreamGuard<'_>,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
        accumulated_messages: &mut Vec<Value>,
        assistant_message: Option<&Value>,
        tool_call: &ToolCall,
        tool_result: &ToolResult,
        on_delta: &mut F,
    ) -> Result<ProviderResponse, String>
    where
        F: FnMut(ProviderStreamChunk),
    {
        let endpoint = self.responses_endpoint();
        provider_log(format!(
            "request:responses followup-stream endpoint={} model={} tool={}",
            endpoint, request.model, tool_call.name
        ));
        let messages = openai_messages_with_tool_result(
            request,
            accumulated_messages,
            assistant_message,
            tool_call,
            tool_result,
        );
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
                ProviderResponse {
                    output_text,
                    tool_calls: tool_calls_from_single(&tool_call),
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
        let assistant_message = if let Some(tool_call) = message.tool_call.as_ref() {
            Some(
                provider_native_assistant_tool_call_message_with_reasoning_value(
                    text_if_present(&output_text),
                    message.reasoning_content_value.as_ref(),
                    tool_call,
                ),
            )
        } else {
            Some(provider_native_assistant_message_with_reasoning_value(
                &output_text,
                message.reasoning_content_value.as_ref(),
            ))
        };

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
        let request = crate::agent::otel::with_current_traceparent(request);
        let response = block_on(async move { request.json(body).send().await }).map_err(|error| {
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
                error = crate::agent::retry::append_cooldown_active_marker(&error);
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
        let request = crate::agent::otel::with_current_traceparent(request);
        let response = block_on(async move { request.json(body).send().await }).map_err(|error| {
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
                error = crate::agent::retry::append_cooldown_active_marker(&error);
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

        let assistant_message = if let Some(tool_call) = tool_call.as_ref() {
            Some(
                provider_native_assistant_tool_call_message_with_reasoning_value(
                    text_if_present(&output_text),
                    message.reasoning_content_value.as_ref(),
                    tool_call,
                ),
            )
        } else {
            Some(provider_native_assistant_message_with_reasoning_value(
                &output_text,
                message.reasoning_content_value.as_ref(),
            ))
        };

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

    fn post_openai_json(&self, endpoint: &str, body: &Value) -> Result<Value, String> {
        // PA-171 C3-P1：冷却期在发送前短路（0 次 HTTP）。
        self.ensure_not_cooling()?;
        // PA-172 §2.1/§3.3：单次 attempt 总超时配置化（默认 45s，与基线逐字节等价）；
        // 换手候选再按**逐 attempt deadline** 收紧（剩余为 0 ⇒ 立即 Abort，0 次 HTTP）。
        let attempt_timeout_ms = self.effective_attempt_timeout_ms();
        if attempt_timeout_ms == 0 {
            return Err(Self::failover_budget_exhausted_error());
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
        let request = crate::agent::otel::with_current_traceparent(request);
        let response = block_on(async move { request.json(body).send().await })
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
                error = crate::agent::retry::append_cooldown_active_marker(&error);
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

    fn post_anthropic_json(&self, endpoint: &str, body: &Value) -> Result<Value, String> {
        // PA-171 C3-P1：冷却期在发送前短路（0 次 HTTP）。
        self.ensure_not_cooling()?;
        // PA-172 §2.1/§3.3：单次 attempt 总超时配置化（默认 45s，与基线逐字节等价）；
        // 换手候选再按**逐 attempt deadline** 收紧（剩余为 0 ⇒ 立即 Abort，0 次 HTTP）。
        let attempt_timeout_ms = self.effective_attempt_timeout_ms();
        if attempt_timeout_ms == 0 {
            return Err(Self::failover_budget_exhausted_error());
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
        let request = crate::agent::otel::with_current_traceparent(request);
        let response = block_on(async move { request.json(body).send().await }).map_err(|error| {
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
                error = crate::agent::retry::append_cooldown_active_marker(&error);
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

fn build_http_client(timeout: Duration, endpoint: &str) -> Result<Client, String> {
    let builder = Client::builder()
        .timeout(timeout)
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
) -> Vec<crate::agent::tools::ToolDefinitionContractView> {
    if tools.is_empty() {
        return Vec::new();
    }

    let Ok(registry) = crate::agent::tools::ToolRegistrySnapshot::builtin() else {
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
) -> Vec<crate::agent::tools::ToolDefinitionContractView> {
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
) -> Vec<crate::agent::tools::ToolDefinitionContractView> {
    surface_contract_views(tools)
}

fn builtin_provider_contract_views() -> Vec<crate::agent::tools::ToolDefinitionContractView> {
    builtin_tool_surface()
        .provider_contract_views()
        .expect("default turn tool view must match its registry snapshot")
}

/// W28 附带必修：非 builtin 工具集走 fallback 分支时，仍必须挡住「已知 builtin 但
/// `exposure=Internal`」的名字——否则内部工具（如 `subagent` 桩）可经动态工具通道回流到
/// 模型可见面，绕过 exposure 策略。未知/动态工具（`mcp__<server>__<tool>` 等）原样透传。
pub(crate) fn is_internal_builtin_tool_name(name: &str) -> bool {
    use crate::agent::tools::{canonical_tool_name, tool_exposure_for_name, ToolExposure};
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
    use crate::agent::tools::{canonical_tool_name, ToolSurface};

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

fn anthropic_system_text(input: &[ProviderMessage]) -> String {
    input
        .iter()
        .filter(|message| matches!(message.role, ProviderRole::System | ProviderRole::Developer))
        // 过滤掉 Previous assistant reply，它们应该出现在 messages 数组中作为 assistant 轮次
        .filter(|message| !message.content.starts_with("Previous assistant reply:"))
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
                }
                // 其他 Developer 消息继续留在 system text 中（由 anthropic_system_text 处理）
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
            json!({
                "role": to_chat_role(&message.role),
                "content": openai_message_content(message.content.as_str(), is_last_user_with_images(index, last_user_index, &request.images), &request.images)
            })
        })
        .collect::<Vec<_>>()
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
    json!({
        "role": "assistant",
        "content": content,
        "reasoning_content": reasoning_content,
        "tool_calls": [
            {
                "id": tool_call.call_id.clone().unwrap_or_else(|| "tool_call_local".to_string()),
                "type": "function",
                "function": {
                    "name": openai_safe_tool_name(&tool_call.name),
                    "arguments": tool_call.arguments.to_string()
                }
            }
        ]
    })
}

pub fn provider_native_assistant_tool_call_message_for_protocol(
    protocol: &ProviderProtocol,
    content: Option<&str>,
    reasoning_content: Option<&Value>,
    tool_call: &ToolCall,
) -> Value {
    if !protocol.is_anthropic() {
        return provider_native_assistant_tool_call_message_with_reasoning_value(
            content,
            reasoning_content,
            tool_call,
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
    blocks.push(json!({
        "type": "tool_use",
        "id": tool_call.call_id.clone().unwrap_or_else(|| "toolu_local".to_string()),
        "name": tool_call.name,
        "input": tool_call.arguments
    }));
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
    if !protocol.is_anthropic() {
        return provider_native_tool_result_message(tool_call, tool_result);
    }

    json!({
        "role": "user",
        "content": [{
            "type": "tool_result",
            "tool_use_id": tool_call.call_id.clone().unwrap_or_else(|| "toolu_local".to_string()),
            "content": anthropic_tool_result_content(tool_result),
            "is_error": tool_result.status != "ok"
        }]
    })
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

const OPENAI_TOOL_RESULT_INLINE_LIMIT_CHARS: usize = 12_000;
const OPENAI_TOOL_RESULT_HEAD_CHARS: usize = 4_500;
const OPENAI_TOOL_RESULT_TAIL_CHARS: usize = 2_500;
const OPENAI_TOOL_RESULT_SUMMARY_PREVIEW_CHARS: usize = 240;
/// DeepSeek thinking 模式要求带 tool_calls 的 assistant 消息回传非空
/// reasoning_content（实测 API 只校验非空，不校验内容一致性）。
/// 本地 planner 决策等来源没有该字段时用此占位文本兜底。
const TOOL_CALL_REASONING_PLACEHOLDER: &str = "工具调用前的思考过程已省略。";
// PA-172 §2.1：尝试次数与退避预算不再是硬编码常量 —— 取值来自 `UpstreamPolicy`
// （默认 `UPSTREAM_DECISION_MAX_ATTEMPTS_DEFAULT = 5` / `UPSTREAM_RETRY_BUDGET_MS_DEFAULT = 30_000`，
// 与基线 `PROVIDER_TIMEOUT_RETRY_MAX_ATTEMPTS = 5` + `total_budget_ms = 30_000` 逐字节等价，G2）。
/// 短退避曲线的固定形状（初始延迟 / 倍率 / 单次上限 / 无抖动），与基线一致；
/// 只有 `max_retries` 与 `total_budget_ms` 由策略驱动。
const PROVIDER_TIMEOUT_BACKOFF_INITIAL_DELAY_MS: u64 = 500;
const PROVIDER_TIMEOUT_BACKOFF_MULTIPLIER: f64 = 2.0;
const PROVIDER_TIMEOUT_BACKOFF_MAX_DELAY_MS: u64 = 8_000;

/// PA-171 C3-S4/C3-P1：非 2xx 错误文本的**唯一装配点**。
///
/// `Retry-After` 以 [`crate::agent::retry::RETRY_AFTER_MARKER`] 结构化后缀承载（解析同源，
/// 禁止第二处拼装）；冷却窗口与 TUI 文案都只消费这一份结构化时间。
fn provider_status_error(
    status: reqwest::StatusCode,
    elapsed_ms: u128,
    body: &str,
    retry_after: Option<&str>,
) -> String {
    let base = format!("provider 返回错误状态：{status}；耗时={elapsed_ms}ms；响应正文：{body}");
    match retry_after.and_then(parse_retry_after_header_seconds) {
        Some(secs) => crate::agent::retry::append_retry_after_marker(&base, secs),
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

/// wave-tui-session-resilience C1：DSML 工具调用协议原文的**唯一**清洗入口。
/// 二元隔离在 SSE 解析层完成，流式增量、最终累计文本与历史回放共用同一语义；
/// 纯函数、不新增依赖、不误伤 markdown 代码块 / 普通含尖括号正文。
const DSML_PLACEHOLDER: &str = "［工具调用］";

/// DSML 标记的规范写法的「归一化签名」：大小写不敏感 + 全角/半角竖线统一。
/// 只用于判定，绝不改写用户正文（输出仍逐字保真原字符）。
fn dsml_signature(text: &str) -> String {
    text.to_ascii_lowercase().replace('｜', "|")
}

/// 尾部「已识别 DSML 协议前缀」的半截形态：只认已知标签的合法前缀，
/// 普通正文以单个 `<` 结尾 / markdown 代码块 / `<tag>` 一律不判命中。
fn dsml_partial_tail_len(text: &str) -> Option<usize> {
    let trimmed = text.trim_end_matches(['\r', '\n', ' ', '\t']);
    let known_tags = [
        "<||dsml|| calls>",
        "</||dsml|| calls>",
        "<||dsml|| invoke",
        "</||dsml|| invoke>",
        "<||dsml|| parameter",
        "</||dsml|| parameter>",
        "<||dsml||",
        "|dsml||",
        "||ds",
        "<|",
    ];
    // 只检查尾部有限窗口内、合法 UTF-8 字符边界上的候选，
    // 绝不按裸 byte offset 切片；单个 `<` 属普通正文，不判命中。
    let max_tag_len = known_tags.iter().map(|tag| tag.len()).max().unwrap_or(0) * 3 + 4;
    for (start, _) in trimmed.char_indices().rev() {
        if trimmed.len() - start > max_tag_len {
            break;
        }
        let tail = &trimmed[start..];
        if tail.contains('>') || tail.len() <= "<".len() {
            continue;
        }
        // `<|` / `<｜` 这类最短前缀只在行首或换行后判为协议碎片；
        // 行中普通正文（如 `a <| 半角竖线`）逐字保真。
        if start > 0 && !trimmed[..start].ends_with('\n') && !tail.starts_with("<｜") {
            continue;
        }
        let signature = dsml_signature(tail);
        if known_tags.iter().any(|tag| tag.starts_with(&signature)) {
            return Some(text.len() - start);
        }
    }
    None
}

/// 协议行判定：归一化后必须含 `|dsml|`（覆盖大小写 / 半角 / 全角 / 混合变体），
/// 普通 `<tag>`、markdown 代码块、含 `<｜不是 DSML` 之类正文不命中。
fn is_dsml_protocol_line(line: &str) -> bool {
    dsml_signature(line).contains("|dsml|")
}

pub fn sanitize_dsml_protocol_text(text: &str) -> String {
    // 跨 chunk 被切断的半截标记先截掉：分片累加时不得残留 `<｜` / `｜DSML` 等前缀。
    let body = match dsml_partial_tail_len(text) {
        Some(len) => &text[..text.len() - len],
        None => text,
    };
    // 整行折叠：DSML 协议块（含大小写/半角/混合变体）按行剔除，
    // 块外正文逐字保真；至少折叠一次时给一个面向用户的占位。
    let mut cleaned = String::with_capacity(body.len());
    let mut removed_protocol = false;
    for line in body.split_inclusive('\n') {
        if is_dsml_protocol_line(line) {
            removed_protocol = true;
            continue;
        }
        cleaned.push_str(line);
    }
    if removed_protocol && !cleaned.contains(DSML_PLACEHOLDER) {
        cleaned.push_str(DSML_PLACEHOLDER);
    }
    // 收尾：孤立半截尾巴（如 `｜DSML`）再截一次。
    if let Some(len) = dsml_partial_tail_len(&cleaned) {
        cleaned.truncate(cleaned.len() - len);
    }
    cleaned
}

/// PA-100/PA-171：重试作用域。rate-limit 长退避只对 followup 生效——对话首跳
/// （decision/decision_stream）命中限流/配额时**首跳即 Abort**（C3-P2：0 次 sleep，
/// 冷却由 HTTP 出口登记），非限流瞬时错误仍走短退避快败进既有 fallback，
/// 避免用户在看到任何输出前先白等一分钟。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderRetryScope {
    Decision,
    Followup,
}

/// PA-172 §2.1：短退避配置由 [`UpstreamPolicy`] 驱动。
///
/// `decision_max_attempts` 是"含首次"的尝试次数 ⇒ `max_retries = attempts - 1`
/// （基线 5 ⇒ 4，逐字节等价）；`retry_budget_ms` 是 sleep-only 预算（基线 30_000ms）。
fn provider_timeout_backoff_config_for(policy: &UpstreamPolicy) -> BackoffConfig {
    BackoffConfig {
        max_retries: policy.decision_max_attempts.saturating_sub(1),
        initial_delay_ms: PROVIDER_TIMEOUT_BACKOFF_INITIAL_DELAY_MS,
        multiplier: PROVIDER_TIMEOUT_BACKOFF_MULTIPLIER,
        max_delay_ms: PROVIDER_TIMEOUT_BACKOFF_MAX_DELAY_MS,
        total_budget_ms: policy.retry_budget_ms,
        jitter_kind: JitterKind::None,
    }
}

/// 基线默认包装（未注入策略的调用点/既有单测保持 `UpstreamPolicy::default()` 语义）。
#[cfg(test)]
fn provider_timeout_backoff_config() -> BackoffConfig {
    provider_timeout_backoff_config_for(&UpstreamPolicy::default())
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
        provider_timeout_backoff_config_for(policy),
        operation,
        &StdThreadSleeper,
        None,
    )
}

/// followup 专用包装：允许 rate-limit 长退避。
/// PA-172 §2.1：策略驱动的 followup 作用域重试（`ProviderManager` 内部唯一入口）。
fn retry_provider_followup_timeout_with_policy<T, F>(
    label: &str,
    policy: &UpstreamPolicy,
    operation: F,
) -> Result<T, String>
where
    F: FnMut() -> Result<T, String>,
{
    retry_provider_scoped_with_sleeper_and_config(
        label,
        ProviderRetryScope::Followup,
        provider_timeout_backoff_config_for(policy),
        operation,
        &StdThreadSleeper,
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
        provider_timeout_backoff_config_for(policy),
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
        operation,
        sleeper,
        no_retry_hint,
    )
}

/// PA-172 §2.1：策略驱动版单跳重试（`base_config` 由调用方给出，其余语义逐字不变）。
fn retry_provider_scoped_with_report_config<T, F>(
    label: &str,
    scope: ProviderRetryScope,
    base_config: BackoffConfig,
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
    let policy = ProviderRetryPolicy::new(base_config);
    let mut budget = RetryBudget::new(base_config.max_retries, base_config.total_budget_ms);
    let mut active_config = base_config;
    let mut rl_active = false;
    // attempt：当前配置下的尝试序号——驱动延迟曲线与预算门禁；配置切换时归零
    // （指数基数以切换点为基准重置）。attempts_total 仅用于日志。
    let mut attempt = 0_u32;
    let mut attempts_total = 0_u32;
    // T2-B：sleep-only 累计（成功执行的 sleep 之和），与 budget.elapsed_ms 同步累加、
    // 单独计数字段，避免未来 scoped 路径引入 record_execution 时口径漂移。
    let mut slept_ms = 0_u64;
    let cancelled = std::sync::atomic::AtomicBool::new(false);
    let mut last_failure: Option<String> = None;

    loop {
        if attempt > 0 {
            // 调用方提示"重试已无意义"（如流式增量已发出）→ 放弃最近一次错误。
            if no_retry_hint.is_some_and(|hint| hint()) {
                let terminal = last_failure
                    .unwrap_or_else(|| "retry aborted by caller".to_string());
                let outcome = report(&budget, attempts_total, slept_ms, rl_active);
                return (Err(terminal), outcome);
            }
            let delay = compute_delay(attempt, &active_config, 0.5);
            if let Err(cancelled_err) = sleeper.sleep(delay, &cancelled) {
                let _ = cancelled_err;
                let outcome = report(&budget, attempts_total, slept_ms, rl_active);
                return (
                    Err(format!("{label}: retry sleep cancelled")),
                    outcome,
                );
            }
            let slept = delay.as_millis() as u64;
            budget.record_attempt(slept);
            slept_ms += slept;
        }
        match operation() {
            Ok(value) => {
                let outcome = report(&budget, attempts_total + 1, slept_ms, rl_active);
                // attempts_total 口径 = operation 调用次数：成功这次同样计入。
                return (Ok(value), outcome);
            }
            Err(err) => {
                attempts_total += 1;
                let hint_blocking = no_retry_hint.is_some_and(|hint| hint());
                let last_error = extract_provider_error_detail(&err);
                last_failure = Some(err.clone());
                let failure = policy.classify(&err);
                // L3-B ADVISORY：该错误已把 provider 置入冷却（HTTP 出口/短路打标）⇒
                // **全 scope** 立即 Abort：不 sleep、不再发 HTTP（冷却期 0 次 HTTP）。
                if crate::agent::retry::is_cooldown_active_error(&err) {
                    let outcome = report(&budget, attempts_total, slept_ms, rl_active);
                    return (Err(last_error), outcome);
                }
                // PA-171 C3-P2：Decision（对话首跳）作用域首跳限流/配额即 Abort——
                // 冷却已在 4 个 HTTP 出口登记，重试循环不得再 sleep（否则
                // 0.5+1+2+4=7.5s 违反 "限流快速失败(<3s)" 契约）。
                if scope == ProviderRetryScope::Decision
                    && crate::agent::retry::is_rate_limit_cooldown_error(&err)
                {
                    let outcome = report(&budget, attempts_total, slept_ms, rl_active);
                    return (Err(last_error), outcome);
                }
                let use_rate_limit = scope == ProviderRetryScope::Followup
                    && failure.is_retryable()
                    && is_rate_limit_error(&err)
                    && !hint_blocking;

                if use_rate_limit && !rl_active {
                    // 切换 RL 配置（一次性）：重建 budget 并结转 elapsed_ms（时间
                    // 预算跨配置累计，不重置）；attempts_used 归零（不同配置的
                    // 次数上限不可比）；attempt 归一——下一次尝试前先等
                    // initial_delay_ms，形成 20s→重试→40s→重试 的调度。
                    rl_active = true;
                    active_config = provider_rate_limit_backoff_config();
                    let mut carried =
                        RetryBudget::new(active_config.max_retries, active_config.total_budget_ms);
                    carried.elapsed_ms = budget.elapsed_ms;
                    budget = carried;
                    attempt = 1;
                    let rl_policy = ProviderRetryPolicy::new(active_config);
                    let decision =
                        rl_policy.decide(&failure, &budget, attempt, None, StreamState::NoDelta);
                    provider_log(format!(
                        "{label}:attempt={attempts_total} failed class={failure:?} \
                         decision={decision:?} scope={scope:?} switch=rate_limit"
                    ));
                    match decision {
                        RetryDecision::Retry { .. } => continue,
                        _ => {
                            let outcome = report(&budget, attempts_total, slept_ms, rl_active);
                            return (Err(last_error), outcome);
                        }
                    }
                }

                let active_policy = ProviderRetryPolicy::new(active_config);
                let decision =
                    active_policy.decide(&failure, &budget, attempt, None, StreamState::NoDelta);
                provider_log(format!(
                    "{}:attempt={}/{} failed class={:?} decision={:?} scope={:?}",
                    label,
                    attempts_total,
                    active_config.max_retries + 1,
                    failure,
                    decision,
                    scope
                ));
                match decision {
                    RetryDecision::Retry { .. } => {
                        attempt += 1;
                    }
                    _ => {
                        let outcome = report(&budget, attempts_total, slept_ms, rl_active);
                        return (Err(last_error), outcome);
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
    let output_chars = tool_result.output.chars().count();
    if output_chars <= OPENAI_TOOL_RESULT_INLINE_LIMIT_CHARS {
        return tool_result.output.clone();
    }

    let total_lines = tool_result.output.lines().count();
    let head = take_chars(
        &tool_result.output,
        OPENAI_TOOL_RESULT_HEAD_CHARS.min(output_chars),
    );
    let remaining_chars = output_chars.saturating_sub(head.chars().count());
    let tail = take_last_chars(
        &tool_result.output,
        OPENAI_TOOL_RESULT_TAIL_CHARS.min(remaining_chars),
    );
    let payload = json!({
        "tool_name": tool_result.tool_name,
        "status": tool_result.status,
        "compression": {
            "applied": true,
            "strategy": "head_tail_excerpt",
            "reason": "provider_followup_size_guard",
            "original_chars": output_chars,
            "original_lines": total_lines,
            "included_head_chars": head.chars().count(),
            "included_tail_chars": tail.chars().count(),
            "omitted_chars": output_chars.saturating_sub(head.chars().count() + tail.chars().count())
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
            output_chars
        )
    })
}

fn local_tool_followup_fallback_response(
    request: &ProviderRequest,
    tool_call: &ToolCall,
    tool_result: &ToolResult,
    error: String,
) -> ProviderResponse {
    let fallback_reason = format!("provider_followup_failed: {}", preview_text(&error, 240));
    let summary = local_tool_followup_summary(tool_result);
    let output_text = format!(
        "工具 `{}` 已执行完成，但 provider 在整合工具结果时失败。\n失败原因：{}\n\n本地兜底摘要：{}\n\n结果预览：\n{}",
        tool_call.name,
        preview_text(&error, 240),
        summary,
        preview_text(&tool_result.output, 2200)
    );

    ProviderResponse {
        output_text: output_text.clone(),
        tool_calls: Vec::new(),
        tool_call: None,
        reasoning_content: None,
        reasoning_content_value: None,
        assistant_message: Some(provider_native_assistant_message(&output_text)),
        provider_source: "provider_followup_local_fallback".to_string(),
        provider_mode: "fallback".to_string(),
        fallback_reason: Some(fallback_reason),
        token_usage: Some(estimate_token_usage(request, &output_text)),
    }
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

fn take_chars(text: &str, count: usize) -> String {
    text.chars().take(count).collect()
}

fn take_last_chars(text: &str, count: usize) -> String {
    let total = text.chars().count();
    text.chars().skip(total.saturating_sub(count)).collect()
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

#[allow(dead_code)]
fn extract_chat_delta_text(payload: &Value) -> Option<String> {
    let delta = payload
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("delta"))?;
    let content = delta.get("content")?;

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
                Some(parts.join(""))
            }
        }
        _ => None,
    }
}

fn extract_openai_usage(payload: &Value) -> Option<TokenUsage> {
    let usage = payload.get("usage")?;
    provider_log(format!("usage:openai raw={}", usage));
    let cache_hit = usage
        .get("prompt_cache_hit_tokens")
        .and_then(Value::as_u64)
        .map(|value| (value, "usage.prompt_cache_hit_tokens"))
        .or_else(|| {
            usage
                .get("input_tokens_details")
                .and_then(|value| value.get("cached_tokens"))
                .and_then(Value::as_u64)
                .map(|value| (value, "usage.input_tokens_details.cached_tokens"))
        })
        .or_else(|| {
            usage
                .get("prompt_tokens_details")
                .and_then(|value| value.get("cached_tokens"))
                .and_then(Value::as_u64)
                .map(|value| (value, "usage.prompt_tokens_details.cached_tokens"))
        });
    Some(normalize_token_usage(TokenUsage {
        input_tokens: usage.get("prompt_tokens").and_then(Value::as_u64),
        cache_hit_input_tokens: cache_hit.map(|(value, _source)| value),
        cache_hit_source: cache_hit.map(|(_value, source)| source.to_string()),
        reasoning_tokens: usage
            .get("completion_tokens_details")
            .and_then(|value| value.get("reasoning_tokens"))
            .and_then(Value::as_u64)
            .or_else(|| {
                usage
                    .get("output_tokens_details")
                    .and_then(|value| value.get("reasoning_tokens"))
                    .and_then(Value::as_u64)
            }),
        output_tokens: usage.get("completion_tokens").and_then(Value::as_u64),
        total_tokens: usage.get("total_tokens").and_then(Value::as_u64),
        cache_creation_input_tokens: None,
        // wave-54 C3：上游 usage 真值。
        origin: Some(USAGE_ORIGIN_PROVIDER),
    }))
}

fn openai_safe_tool_name(name: &str) -> String {
    name.replace('.', "__dot__")
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

fn normalize_token_usage(usage: TokenUsage) -> TokenUsage {
    let total_tokens =
        usage
            .total_tokens
            .or_else(|| match (usage.input_tokens, usage.output_tokens) {
                (Some(input_tokens), Some(output_tokens)) => Some(input_tokens + output_tokens),
                _ => None,
            });

    TokenUsage {
        input_tokens: usage.input_tokens,
        cache_hit_input_tokens: usage.cache_hit_input_tokens,
        cache_hit_source: usage.cache_hit_source,
        reasoning_tokens: usage.reasoning_tokens,
        output_tokens: usage.output_tokens,
        total_tokens,
        cache_creation_input_tokens: usage.cache_creation_input_tokens,
        origin: usage.origin,
    }
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
        .rposition(|message| message.get("role").and_then(Value::as_str) == Some("user"))
        .unwrap_or(messages.len().saturating_sub(1));

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
pub(crate) const PROTOCOL_SSE_PARSE_CODE: &str = "protocol_sse_parse";
#[allow(dead_code)]
pub(crate) const PROTOCOL_TOOL_CALL_SCHEMA_CODE: &str = "protocol_tool_call_schema";
#[allow(dead_code)]
pub(crate) const PROTOCOL_MODEL_CATALOG_CODE: &str = "protocol_model_catalog";
#[allow(dead_code)]
pub(crate) const PROTOCOL_UNKNOWN_MODEL_CODE: &str = "protocol_unknown_model";
#[allow(dead_code)]
pub(crate) const PROTOCOL_REASONING_EFFORT_CODE: &str = "protocol_reasoning_effort";

/// Stage3 P2：SSE 失败可观测包装——code=protocol_sse_parse，
/// message 含原始错误 + endpoint/parsed_bytes/elapsed 证据。fallback 链不动
/// （调用方仍按 `Err(String)` 走原有 sync/stream 回退）。
#[allow(dead_code)]
pub(crate) fn protocol_sse_parse_error(
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
pub(crate) fn protocol_tool_call_schema_error(detail: &str, evidence: &str) -> String {
    format!(
        "{}: {}; evidence={}",
        PROTOCOL_TOOL_CALL_SCHEMA_CODE,
        detail,
        preview_text(evidence, 400)
    )
}

/// Stage3 P4：model catalog 可观测包装——code=protocol_model_catalog。
#[allow(dead_code)]
pub(crate) fn protocol_model_catalog_error(detail: &str, evidence: &str) -> String {
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
pub(crate) fn protocol_unknown_model_error(model: &str, known_count: usize) -> String {
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
pub(crate) fn protocol_reasoning_effort_error(detail: &str, evidence: &str) -> String {
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
mod tests {
    use super::*;
    use crate::agent::config::ProviderModelCapabilities;
    use crate::agent::config::ResolvedProviderSelection;
    use crate::agent::input::TurnInputImage;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;

    #[test]
    fn test_reasoning_model_omits_temperature() {
        let reasoning_config = ResolvedProviderSelection {
            requested_name: "test-reasoning".to_string(),
            provider_name: "openai".to_string(),
            protocol: ProviderProtocol::OpenAiCompletions,
            base_url: "https://api.openai.com/v1".to_string(),
            auth_type: ProviderAuthType::Bearer,
            api_key_env_var: "OPENAI_API_KEY".to_string(),
            api_key: Some("test-key".to_string()),
            model: "o1".to_string(),
            temperature: 0.2,
            max_output_tokens: 1024,
            reasoning_effort: None,
            reasoning_budget_tokens: None,
            capabilities: ProviderModelCapabilities {
                supports_reasoning: true,
                ..Default::default()
            },
            thinking_param_pattern: ThinkingParamPattern::None,
        };

        let non_reasoning_config = ResolvedProviderSelection {
            capabilities: ProviderModelCapabilities {
                supports_reasoning: false,
                ..Default::default()
            },
            ..reasoning_config.clone()
        };

        let initial_body = json!({
            "model": "test-model",
            "messages": [],
            "temperature": 0.2,
            "max_tokens": 1024,
        });

        let reasoning_body = with_openai_request_options(initial_body.clone(), &reasoning_config);
        assert!(
            reasoning_body.get("temperature").is_none(),
            "Expected 'temperature' to be omitted when supports_reasoning is true, but got: {:?}",
            reasoning_body.get("temperature")
        );

        let non_reasoning_body = with_openai_request_options(initial_body, &non_reasoning_config);
        assert!(
            non_reasoning_body.get("temperature").is_some(),
            "Expected 'temperature' to be present when supports_reasoning is false"
        );
        assert_eq!(
            non_reasoning_body.get("temperature").and_then(Value::as_f64),
            Some(0.2)
        );
    }

    #[test]
    fn openai_request_messages_encode_image_blocks_for_last_user_message() {
        let request = ProviderRequest {
            model: "gpt-5.4".to_string(),
            input: vec![
                ProviderMessage::developer("Keep answers short."),
                ProviderMessage::user("请描述这张图"),
            ],
            images: vec![TurnInputImage {
                data_url: "data:image/png;base64,Zm9v".to_string(),
                mime_type: "image/png".to_string(),
                name: Some("demo.png".to_string()),
            }],
            native_messages: Vec::new(),
            observation: ProviderRequestObservation::default(),
            temperature: 0.2,
            max_output_tokens: 1024,
        };

        let messages = openai_request_messages(&request);
        let content = messages[1]
            .get("content")
            .and_then(Value::as_array)
            .expect("last user message should use content blocks");

        assert_eq!(content[0].get("type").and_then(Value::as_str), Some("text"));
        assert_eq!(
            content[1]
                .get("image_url")
                .and_then(|value| value.get("url"))
                .and_then(Value::as_str),
            Some("data:image/png;base64,Zm9v")
        );
    }

    #[test]
    fn anthropic_conversation_messages_encode_base64_image_blocks_for_last_user_message() {
        let request = ProviderRequest {
            model: "claude-3-7-sonnet".to_string(),
            input: vec![ProviderMessage::user("请看图回答")],
            images: vec![TurnInputImage {
                data_url: "data:image/png;base64,Zm9v".to_string(),
                mime_type: "image/png".to_string(),
                name: Some("demo.png".to_string()),
            }],
            native_messages: Vec::new(),
            observation: ProviderRequestObservation::default(),
            temperature: 0.2,
            max_output_tokens: 1024,
        };

        let messages = anthropic_conversation_messages(&request);
        let content = messages[0]
            .get("content")
            .and_then(Value::as_array)
            .expect("anthropic user message should use content blocks");

        assert_eq!(content[0].get("type").and_then(Value::as_str), Some("text"));
        assert_eq!(
            content[1].get("type").and_then(Value::as_str),
            Some("image")
        );
        assert_eq!(
            content[1]
                .get("source")
                .and_then(|value| value.get("data"))
                .and_then(Value::as_str),
            Some("Zm9v")
        );
    }

    #[test]
    fn build_context_observation_exposes_layered_request_segments() {
        let request = ProviderRequest {
            model: "gpt-4.1-mini".to_string(),
            input: vec![
                ProviderMessage::system("Stable system instruction"),
                ProviderMessage::developer("Stable capability prefix"),
                ProviderMessage::user("Recent context summary"),
                ProviderMessage::user("Actual volatile request"),
            ],
            images: Vec::new(),
            native_messages: Vec::new(),
            observation: ProviderRequestObservation::default(),
            temperature: 0.2,
            max_output_tokens: 1024,
        };

        let observation = build_context_observation(
            &request,
            &[ToolDefinition {
                name: "workspace.read_file",
                description: "read file",
                input_schema: json!({ "type": "object" }),
            }],
        );

        assert_eq!(observation.request_format, "normalized-input");
        assert_eq!(observation.message_count, 4);
        assert!(observation
            .stable_prefix_text
            .contains("Stable system instruction"));
        assert!(observation
            .stable_prefix_text
            .contains("Stable capability prefix"));
        assert!(!observation
            .stable_prefix_text
            .contains("Actual volatile request"));
        assert!(observation
            .semi_stable_context_text
            .contains("Recent context summary"));
        assert!(observation
            .volatile_input_text
            .contains("Actual volatile request"));
        assert_eq!(observation.tool_count, 1);
    }

    #[test]
    fn openai_tools_payload_uses_product_tool_surface_for_builtin_tools() {
        let payload = openai_tools_payload(&crate::agent::tools::builtin_tools());
        let names = payload
            .iter()
            .filter_map(|entry| {
                entry
                    .get("function")
                    .and_then(|value| value.get("name"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .collect::<Vec<_>>();

        assert_eq!(
            names,
            vec![
                "Run".to_string(),
                "Ask".to_string(),
                "Read".to_string(),
                "ReadDocument".to_string(),
                "List".to_string(),
                "Search".to_string(),
                "Glob".to_string(),
                "WebFetch".to_string(),
                "WebSearch".to_string(),
                "MCPResource".to_string(),
                "ToolSearch".to_string(),
                "Write".to_string(),
                "Edit".to_string(),
                "BatchExecute".to_string(),
                "Plan".to_string(),
                "ViewImage".to_string(),
                // Wave-61 C1：9 项团队工具进入模型可见面（按 builtin_tools 定义序插在
                // ViewImage 之后、TodoWrite 之前；`workflow` 桩实现保持 Internal）。
                "subagent".to_string(),
                "spawn_teammate".to_string(),
                "list_agents".to_string(),
                "send_message".to_string(),
                "interrupt_agent".to_string(),
                "drain_inbox".to_string(),
                "team_task_create".to_string(),
                "team_task_update".to_string(),
                "team_task_list".to_string(),
                "TodoWrite".to_string(),
                "TodoList".to_string(),
                // W20 goal 工具为模型可见面成员（实测 surface 尾部）。
                "create_goal".to_string(),
                "get_goal".to_string(),
                "update_goal".to_string(),
                // W37-C1：七件新模型可见工具（present/fuzzy_file_search/git_diff_remote/job_*）。
                "Present".to_string(),
                "Find".to_string(),
                "GitDiff".to_string(),
                "JobStart".to_string(),
                "JobOutput".to_string(),
                "JobKill".to_string(),
                "JobList".to_string(),
                // W42/PA-149：技能正文按需加载（builtin_tools 追加序末位）。
                "skill_load".to_string()
            ]
        );
    }

    #[test]
    fn fallback_tool_payload_filters_internal_builtin_but_keeps_dynamic_tools() {
        // W28 附带必修 + Wave-61 C1/C1.7 迁移（契约 C4.4「面④」）。
        //
        // 迁移理由（真值源）：C1 之后团队工具并入 `ModelVisible`，但 `workflow` 因
        // `orchestration::workflow()` 仍是桩实现（只回显 step.id + status="success"）被
        // L3-P 裁决**保持 Internal**。因此 `builtin_tools()` 的 Internal 集合恰为
        // {`workflow`}：既保留了本用例「Internal builtin 必须被 fallback 摘除」的原始立意，
        // 又把集合钉成精确值（回归守卫：任何 builtin 的 exposure 漂移都会让本用例变红）。
        let builtin_names: Vec<String> = crate::agent::tools::builtin_tools()
            .into_iter()
            .map(|tool| tool.name.to_string())
            .collect();
        let internal_builtins: Vec<&String> = builtin_names
            .iter()
            .filter(|name| is_internal_builtin_tool_name(name))
            .collect();
        let internal_names: Vec<&str> = internal_builtins.iter().map(|name| name.as_str()).collect();
        assert_eq!(
            internal_names,
            vec!["workflow"],
            "Wave-61 L3-P 后 builtin_tools() 的 exposure=Internal 集合必须恰为 [workflow]\
             （模型面只允许通过 fallback 摘除这一个桩实现）；实际：{internal_names:?}"
        );

        let tools = vec![
            ToolDefinition {
                name: "workflow",
                description: "wave-61：仍为 Internal 的桩实现",
                input_schema: json!({"type": "object"}),
            },
            ToolDefinition {
                name: "subagent",
                description: "wave-61：已 ModelVisible 的 builtin",
                input_schema: json!({"type": "object"}),
            },
            ToolDefinition {
                name: "mcp__demo__echo",
                description: "dynamic tool",
                input_schema: json!({"type": "object"}),
            },
        ];
        let payload = anthropic_tools_payload(&tools);
        let names: Vec<&str> = payload
            .iter()
            .filter_map(|entry| entry.get("name").and_then(Value::as_str))
            .collect();
        assert_eq!(names, vec!["subagent", "mcp__demo__echo"]);
    }

    #[test]
    fn anthropic_tools_payload_uses_product_tool_surface_for_builtin_tools() {
        let payload = anthropic_tools_payload(&crate::agent::tools::builtin_tools());
        let names = payload
            .iter()
            .filter_map(|entry| {
                entry
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .collect::<Vec<_>>();

        assert_eq!(
            names,
            vec![
                "Run".to_string(),
                "Ask".to_string(),
                "Read".to_string(),
                "ReadDocument".to_string(),
                "List".to_string(),
                "Search".to_string(),
                "Glob".to_string(),
                "WebFetch".to_string(),
                "WebSearch".to_string(),
                "MCPResource".to_string(),
                "ToolSearch".to_string(),
                "Write".to_string(),
                "Edit".to_string(),
                "BatchExecute".to_string(),
                "Plan".to_string(),
                "ViewImage".to_string(),
                // Wave-61 C1：9 项团队工具进入模型可见面（按 builtin_tools 定义序插在
                // ViewImage 之后、TodoWrite 之前；`workflow` 桩实现保持 Internal）。
                "subagent".to_string(),
                "spawn_teammate".to_string(),
                "list_agents".to_string(),
                "send_message".to_string(),
                "interrupt_agent".to_string(),
                "drain_inbox".to_string(),
                "team_task_create".to_string(),
                "team_task_update".to_string(),
                "team_task_list".to_string(),
                "TodoWrite".to_string(),
                "TodoList".to_string(),
                // W20 goal 工具为模型可见面成员（实测 surface 尾部）。
                "create_goal".to_string(),
                "get_goal".to_string(),
                "update_goal".to_string(),
                // W37-C1：七件新模型可见工具。
                "Present".to_string(),
                "Find".to_string(),
                "GitDiff".to_string(),
                "JobStart".to_string(),
                "JobOutput".to_string(),
                "JobKill".to_string(),
                "JobList".to_string(),
                // W42/PA-149：技能正文按需加载（builtin_tools 追加序末位）。
                "skill_load".to_string()
            ]
        );
        assert!(payload.iter().all(|tool| {
            tool.get("description")
                .and_then(Value::as_str)
                .is_some_and(|description| !description.trim().is_empty())
        }));
        // PA-076 P2-6: Plan and ViewImage 在既有 13-tool 前缀后追加；W25 起 team 工具已被
        // 摘除（未接线），W37-C1 job_* 七件成为尾部，W42 追加 skill_load 成为当前尾项
        //（cache-friendly stable prefix invariant from design Decision 10 仍保持：
        // 前缀顺序不变，仅尾部变化）。
        let last_two = &names[names.len() - 2..];
        assert_eq!(last_two, &["JobList".to_string(), "skill_load".to_string()]);
    }

    #[test]
    fn anthropic_native_messages_preserve_tool_use_result_and_thinking_signature() {
        let tool_call = ToolCall {
            call_id: Some("toolu_123".to_string()),
            name: "List".to_string(),
            arguments: json!({ "path": ".", "description": "列出当前目录" }),
            plan: None,
        };
        let tool_result = ToolResult {
            tool_name: "List".to_string(),
            status: "ok".to_string(),
            output: "Cargo.toml\nsrc".to_string(),
            duration_ms: 1,
        };
        let thinking = json!([{
            "type": "thinking",
            "thinking": "先读取目录。",
            "signature": "signature_123"
        }]);

        let assistant = provider_native_assistant_tool_call_message_for_protocol(
            &ProviderProtocol::AnthropicMessages,
            Some("我先列出目录。"),
            Some(&thinking),
            &tool_call,
        );
        let result = provider_native_tool_result_message_for_protocol(
            &ProviderProtocol::AnthropicMessages,
            &tool_call,
            &tool_result,
        );
        let request = ProviderRequest {
            model: "claude-sonnet-5".to_string(),
            input: vec![ProviderMessage::user("当前文件夹下有哪些文件？")],
            images: Vec::new(),
            native_messages: vec![assistant.clone(), result.clone()],
            observation: ProviderRequestObservation::default(),
            temperature: 0.0,
            max_output_tokens: 1024,
        };

        assert_eq!(
            anthropic_conversation_messages(&request),
            vec![assistant, result.clone()]
        );
        assert_eq!(result["content"][0]["type"].as_str(), Some("tool_result"));
        assert_eq!(
            result["content"][0]["tool_use_id"].as_str(),
            Some("toolu_123")
        );
        assert_eq!(
            request.native_messages[0]["content"][0]["signature"].as_str(),
            Some("signature_123")
        );
    }

    #[test]
    fn anthropic_conversation_messages_skip_openai_native_transcripts() {
        let request = ProviderRequest {
            model: "claude-sonnet-5".to_string(),
            input: vec![ProviderMessage::user("当前文件夹下有哪些文件？")],
            images: Vec::new(),
            native_messages: vec![
                json!({
                    "role": "assistant",
                    "content": "我先列出目录。",
                    "reasoning_content": null,
                    "tool_calls": []
                }),
                json!({
                    "role": "tool",
                    "tool_call_id": "call_list",
                    "content": "Cargo.toml\nsrc"
                }),
            ],
            observation: ProviderRequestObservation::default(),
            temperature: 0.0,
            max_output_tokens: 1024,
        };

        assert_eq!(
            anthropic_conversation_messages(&request),
            vec![json!({
                "role": "user",
                "content": "当前文件夹下有哪些文件？"
            })]
        );
    }

    /// P1-1：多调用被丢弃时必须有**结构化告警行**（不再静默截断）。
    #[test]
    fn openai_multiple_tool_calls_emit_structured_dropped_warning() {
        let message = json!({
            "tool_calls": [
                { "id": "call_1", "type": "function",
                  "function": { "name": "Read", "arguments": "{\"path\":\"a.txt\"}" } },
                { "id": "call_2", "type": "function",
                  "function": { "name": "List", "arguments": "{}" } },
            ]
        });
        let kept = extract_openai_tool_call(&message);
        let line = openai_dropped_tool_calls_warning(
            "openai_decision",
            &message,
            kept.as_ref(),
        )
        .expect("P1-1：2 个 tool_calls 必须产出告警行");
        // 真实日志通道（与生产同一路径），便于 --nocapture 取证。
        provider_log(line.clone());
        eprintln!("[D] dropped-warning line = {line}");

        assert!(line.contains("received=2"), "P1-1：必须带 received=2，实际 {line}");
        assert!(line.contains("dropped=1"), "P1-1：必须带 dropped=1，实际 {line}");
        assert!(
            line.contains("List"),
            "P1-1：dropped_names 必须列出被丢弃的调用名，实际 {line}"
        );
        assert!(
            line.contains(r#"kept=Some("Read")"#),
            "P1-1：kept 必须标明实际执行的首个调用，实际 {line}"
        );

        // 单调用/无调用 ⇒ 不得产生噪音告警。
        let single = json!({ "tool_calls": [ { "id": "call_1", "type": "function",
            "function": { "name": "Read", "arguments": "{}" } } ] });
        assert!(openai_dropped_tool_calls_warning("s", &single, None).is_none());
        assert!(openai_dropped_tool_calls_warning("s", &json!({}), None).is_none());
    }

    // ───────────── D1b：批量回填构造器（纯函数）的形状锁 ─────────────

    fn batch_fixture() -> (Vec<Value>, Value, Vec<ToolCall>, Vec<ToolResult>) {
        let base = vec![json!({ "role": "user", "content": "do three things" })];
        let calls = vec![
            ToolCall {
                call_id: Some("call_1".to_string()),
                name: "Read".to_string(),
                arguments: json!({ "path": "a.txt" }),
                plan: None,
            },
            ToolCall {
                call_id: Some("call_2".to_string()),
                name: "List".to_string(),
                arguments: json!({}),
                plan: None,
            },
            ToolCall {
                call_id: Some("call_3".to_string()),
                name: "Search".to_string(),
                arguments: json!({ "query": "x" }),
                plan: None,
            },
        ];
        let results = ["A", "B", "C"]
            .iter()
            .map(|text| ToolResult {
                tool_name: "t".to_string(),
                status: "ok".to_string(),
                output: text.to_string(),
                duration_ms: 0,
            })
            .collect();
        let assistant = json!({
            "role": "assistant",
            "content": null,
            "reasoning_content": "thinking",
            "tool_calls": calls.iter().map(|call| json!({
                "id": call.call_id.clone().unwrap(),
                "type": "function",
                "function": {
                    "name": call.name.clone(),
                    "arguments": call.arguments.to_string()
                }
            })).collect::<Vec<_>>()
        });
        (base, assistant, calls, results)
    }

    /// OpenAI：恰 1 条 assistant（含全部 tool_calls）+ N 条 role=tool（各自 tool_call_id）。
    #[test]
    fn openai_batch_tool_results_shape_is_one_assistant_and_n_tool_messages() {
        let (base, assistant, calls, results) = batch_fixture();
        let messages = openai_messages_with_tool_results(&base, &assistant, &calls, &results)
            .expect("D1b：长度一致必须成功");
        eprintln!(
            "[D1b] openai batch messages = {}",
            serde_json::to_string(&messages).expect("serialize")
        );

        assert_eq!(
            messages.len(),
            1 + 1 + calls.len(),
            "D1b：= 基础消息 + 1 assistant + N tool"
        );
        let assistants = messages
            .iter()
            .filter(|message| message.get("role").and_then(Value::as_str) == Some("assistant"))
            .count();
        assert_eq!(assistants, 1, "D1b：assistant 必须恰 1 条（N 条即为上游 400 事故）");
        let tool_messages: Vec<&Value> = messages
            .iter()
            .filter(|message| message.get("role").and_then(Value::as_str) == Some("tool"))
            .collect();
        assert_eq!(tool_messages.len(), calls.len(), "D1b：role=tool 必须恰 N 条");
        for (index, (message, result)) in tool_messages.iter().zip(results.iter()).enumerate() {
            assert_eq!(
                message.get("tool_call_id").and_then(Value::as_str),
                calls[index].call_id.as_deref(),
                "D1b：tool_call_id 必须与调用一一对应（保序）"
            );
            assert_eq!(
                message.get("content").and_then(Value::as_str),
                Some(result.output.as_str()),
                "D1b：tool 消息 content 必须是该调用结果"
            );
        }
        let assistant_message = messages
            .iter()
            .find(|message| message.get("role").and_then(Value::as_str) == Some("assistant"))
            .expect("assistant 存在");
        assert_eq!(
            assistant_message
                .get("tool_calls")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(calls.len()),
            "D1b：assistant 必须承载全部 tool_calls"
        );
    }

    /// Anthropic：恰 1 条 assistant（N 个 tool_use）+ 恰 1 条 user（N 个 tool_result）。
    #[test]
    fn anthropic_batch_tool_results_shape_is_one_assistant_and_one_user() {
        let (base, assistant, calls, results) = batch_fixture();
        let messages = anthropic_messages_with_tool_results(&base, &assistant, &calls, &results)
            .expect("D1b：长度一致必须成功");
        eprintln!(
            "[D1b] anthropic batch messages = {}",
            serde_json::to_string(&messages).expect("serialize")
        );

        assert_eq!(messages.len(), 1 + 2, "D1b：= 基础消息 + 1 assistant + 1 user");
        let assistants = messages
            .iter()
            .filter(|message| message.get("role").and_then(Value::as_str) == Some("assistant"))
            .count();
        assert_eq!(
            assistants, 1,
            "D1b（负向护栏）：anthropic 批量回填的 assistant 必须恰 1 条，N 条会触发上游 400"
        );
        let users: Vec<&Value> = messages
            .iter()
            .filter(|message| message.get("role").and_then(Value::as_str) == Some("user"))
            .collect();
        assert_eq!(users.len(), 2, "D1b：基础 user + 回填 user");
        let tool_results: Vec<&Value> = users
            .last()
            .and_then(|message| message.get("content"))
            .and_then(Value::as_array)
            .expect("回填 user 的 content 必须是数组")
            .iter()
            .collect();
        assert_eq!(tool_results.len(), calls.len(), "D1b：tool_result 块必须恰 N 个");
        let tool_uses = messages
            .iter()
            .find(|message| message.get("role").and_then(Value::as_str) == Some("assistant"))
            .and_then(|message| message.get("content"))
            .and_then(Value::as_array)
            .expect("assistant content 必须是数组");
        assert_eq!(
            tool_uses
                .iter()
                .filter(|block| block.get("type").and_then(Value::as_str) == Some("tool_use"))
                .count(),
            calls.len(),
            "D1b：assistant 必须承载全部 tool_use"
        );
        for (index, block) in tool_results.iter().enumerate() {
            assert_eq!(
                block.get("tool_use_id").and_then(Value::as_str),
                calls[index].call_id.as_deref(),
                "D1b：tool_use_id 必须与调用一一对应（保序）"
            );
        }
    }

    /// 长度不匹配 ⇒ Err（带两个真实长度）。
    #[test]
    fn batch_tool_results_reject_length_mismatch() {
        let (base, assistant, calls, mut results) = batch_fixture();
        results.pop();
        let openai_error = openai_messages_with_tool_results(&base, &assistant, &calls, &results)
            .expect_err("D1b：长度不匹配必须 Err");
        let anthropic_error =
            anthropic_messages_with_tool_results(&base, &assistant, &calls, &results)
                .expect_err("D1b：长度不匹配必须 Err");
        eprintln!("[D1b] mismatch errors = {openai_error} | {anthropic_error}");
        for error in [openai_error, anthropic_error] {
            assert!(error.contains("calls=3"), "D1b：错误必须带 calls 长度：{error}");
            assert!(error.contains("results=2"), "D1b：错误必须带 results 长度：{error}");
        }
    }

    /// N=1 时与既有 1:1 构造器的形状语义等价（避免第二套口径）。
    #[test]
    fn batch_tool_results_single_call_matches_legacy_shape() {
        let (base, assistant, calls, results) = batch_fixture();
        let single_call = vec![calls[0].clone()];
        let single_result = vec![results[0].clone()];

        let batch = openai_messages_with_tool_results(
            &base,
            &assistant,
            &single_call,
            &single_result,
        )
        .expect("D1b：单调用批量构造必须成功");
        let legacy_assistant = openai_assistant_message_for_tool_result(
            Some(&assistant),
            &calls[0],
        );
        let legacy_tool = openai_followup_tool_result_message(&calls[0], &results[0]);
        eprintln!(
            "[D1b] single-call openai: batch_assistant={} legacy_assistant={}",
            batch[batch.len() - 2], legacy_assistant
        );
        assert_eq!(
            batch[batch.len() - 2].get("tool_calls"),
            Some(&json!([legacy_assistant
                .get("tool_calls")
                .and_then(Value::as_array)
                .and_then(|arr| arr.first())
                .cloned()
                .expect("legacy assistant 含 1 个 tool_call")])),
            "D1b：单调用时 assistant 的 tool_calls 必须与 legacy 单值构造一致"
        );
        assert_eq!(
            batch[batch.len() - 1], legacy_tool,
            "D1b：单调用时 tool 消息必须与 legacy 单值构造逐字一致"
        );

        let anthropic_batch = anthropic_messages_with_tool_results(
            &base,
            &assistant,
            &single_call,
            &single_result,
        )
        .expect("D1b：单调用批量构造必须成功");
        let legacy_content = anthropic_assistant_content_for_tool_result(Some(&assistant), &calls[0]);
        assert_eq!(
            anthropic_batch[anthropic_batch.len() - 2].get("content"),
            Some(&Value::Array(legacy_content)),
            "D1b：单调用时 anthropic assistant content 必须与 legacy 一致"
        );
    }

    /// D3b 收尾（原 `anthropic_rejects_multiple_tool_calls`，缺 `#[test]` 时从未运行）：
    /// 批量执行已落地 ⇒ 多 `tool_use` **不再拒绝**，解析层按出现顺序返回整批。
    #[test]
    fn anthropic_accepts_multiple_tool_calls() {
        let content = vec![
            json!({ "type": "tool_use", "id": "toolu_1", "name": "List", "input": {} }),
            json!({ "type": "tool_use", "id": "toolu_2", "name": "Read", "input": {} }),
        ];

        let calls = extract_anthropic_tool_calls(&content);
        assert_eq!(
            calls.len(),
            2,
            "D3b：2 个 tool_use 必须原样返回 2 个调用（不再 Err），实际 {calls:?}"
        );
        assert_eq!(calls[0].name, "List");
        assert_eq!(calls[0].call_id.as_deref(), Some("toolu_1"));
        assert_eq!(calls[1].name, "Read");
        assert_eq!(calls[1].call_id.as_deref(), Some("toolu_2"));
    }

    #[test]
    fn render_tool_definitions_uses_product_tool_surface_for_builtin_tools() {
        let rendered = render_tool_definitions(&crate::agent::tools::builtin_tools());

        for tool_name in [
            "Run",
            "Ask",
            "Read",
            "List",
            "Search",
            "Glob",
            "WebFetch",
            "WebSearch",
            "MCPResource",
            "ToolSearch",
            "Write",
            "Edit",
            "BatchExecute",
        ] {
            assert!(
                rendered.contains(&format!("] {}", tool_name)),
                "missing tool heading for {tool_name}: {rendered}"
            );
        }

        for legacy_name in [
            "time_now",
            "echo_input",
            "workspace_list_files",
            "workspace_gather_context",
            "workspace_search_text",
            "workspace_glob_files",
            "web_fetch_url",
            "web_search_query",
            "mcp_resource_read",
            "tool_search",
            "workspace_write_file",
            "workspace_edit_file",
            "workspace_run_command",
            "workspace_batch",
        ] {
            assert!(
                !rendered.contains(&format!("] {}", legacy_name)),
                "legacy tool heading leaked into rendered definitions: {legacy_name}"
            );
        }
    }

    /// ADR 0065 decision 4 / T2 交界不变量一：**纯内建数组**的输出必须与旧
    /// `is_builtin_tool_definition_set == true` 分支逐字节相同（38 项产品面、去重、registry 序）。
    #[test]
    fn surface_contract_views_is_byte_identical_for_the_pure_builtin_array() {
        let builtin = crate::agent::tools::builtin_tools();
        assert_eq!(
            surface_contract_views(&builtin),
            builtin_provider_contract_views(),
            "内建数组必须原样走 registry 投影（不得因重构丢去重/顺序/字段）"
        );
        assert_eq!(
            surface_contract_views(&builtin).len(),
            38,
            "模型可见产品面仍是 38 项"
        );
    }

    /// 交界不变量二：外部工具**只追加、不改序**。追加一个 `mcp__*` 条目后，内建 38 项必须
    /// 逐字节不变且保持前缀，外部条目按数组顺序排在最后（KV-cache 前缀稳定）。
    #[test]
    fn surface_contract_views_appends_external_tools_after_an_unchanged_builtin_prefix() {
        let mut tools = crate::agent::tools::builtin_tools();
        tools.push(ToolDefinition {
            name: "mcp__demo__echo",
            description: "external echo",
            input_schema: json!({ "type": "object", "properties": {} }),
        });
        tools.push(ToolDefinition {
            name: "mcp__demo__zebra",
            description: "external zebra",
            input_schema: json!({ "type": "object", "properties": {} }),
        });

        let views = surface_contract_views(&tools);
        let builtin_views = builtin_provider_contract_views();
        assert_eq!(
            views.len(),
            builtin_views.len() + 2,
            "外部条目只能追加"
        );
        assert_eq!(
            &views[..builtin_views.len()],
            &builtin_views[..],
            "内建前缀必须逐字节不变（顺序/去重/字段）"
        );
        assert_eq!(
            views[builtin_views.len()..]
                .iter()
                .map(|view| view.name.as_str())
                .collect::<Vec<_>>(),
            vec!["mcp__demo__echo", "mcp__demo__zebra"],
            "外部工具按数组顺序追加，且名字原样透传（不得被映射成内建产品名）"
        );

        // 单个外部工具也不得把内建面拖进 fallback（旧的二选一分支会产出 44 条重复视图）。
        assert_ne!(
            views.len(),
            crate::agent::tools::builtin_tools().len(),
            "绝不能退化成逐条 legacy fallback（那会重复 Run/Read 等产品名）"
        );
    }

    #[test]
    fn provider_does_not_infer_builtin_surface_from_tool_count() {
        let decoys = (0..crate::agent::tools::builtin_tools().len())
            .map(|index| ToolDefinition {
                name: "decoy",
                description: "decoy",
                input_schema: json!({
                    "type": "object",
                    "properties": { "index": { "const": index } }
                }),
            })
            .collect::<Vec<_>>();

        let payload = openai_tools_payload(&decoys);
        assert_eq!(payload.len(), decoys.len());
        assert!(payload.iter().all(|entry| {
            entry
                .get("function")
                .and_then(|function| function.get("name"))
                .and_then(Value::as_str)
                == Some("decoy")
        }));
    }

    /// ADR 0065 decision 4：集合身份来自 registry 快照事实，不再来自数组长度/内容比对。
    #[test]
    fn builtin_tool_set_identity_comes_from_registry_facts_not_array_shape() {
        let builtin = crate::agent::tools::builtin_tools();
        assert!(
            is_builtin_tool_definition_set(&builtin),
            "the registry's own definition list must be recognized as the builtin surface"
        );

        // 内容（description / schema）不参与身份判定：名字由快照承认即可。
        let rewritten = builtin
            .iter()
            .map(|definition| ToolDefinition {
                name: definition.name,
                description: "rewritten by a caller",
                input_schema: json!({ "type": "object", "properties": {} }),
            })
            .collect::<Vec<_>>();
        assert!(
            is_builtin_tool_definition_set(&rewritten),
            "identity is a snapshot fact, so caller-rewritten descriptions/schemas must not flip the branch"
        );

        // 子集不算内建面：否则会把 planner 的部分工具视图静默扩张成完整工具面。
        let subset = builtin.iter().take(5).cloned().collect::<Vec<_>>();
        assert!(!is_builtin_tool_definition_set(&subset));

        // 快照不认识的动态名字一律不属于内建面。
        let mut with_dynamic = builtin.clone();
        with_dynamic.push(ToolDefinition {
            name: "mcp__demo__echo",
            description: "dynamic tool",
            input_schema: json!({ "type": "object" }),
        });
        assert!(!is_builtin_tool_definition_set(&with_dynamic));

        assert!(!is_builtin_tool_definition_set(&[]));
    }

    #[test]
    fn build_context_observation_prefers_explicit_layer_metadata() {
        let request = ProviderRequest {
            model: "gpt-5.4".to_string(),
            input: vec![ProviderMessage::user("fallback text should not win")],
            images: Vec::new(),
            native_messages: Vec::new(),
            observation: ProviderRequestObservation {
                stable_prefix_text: "stable prefix".to_string(),
                semi_stable_context_text: "semi-stable context".to_string(),
                volatile_input_text: "actual request".to_string(),
                prefix_mutation_reasons: vec![PrefixMutationReason::SessionSummaryChanged],
                context_refresh_reason: None,
                instruction_scope_sources: Vec::new(),
                conversation_carry_mode: None,
            },
            temperature: 0.2,
            max_output_tokens: 1024,
        };

        let observation = build_context_observation(&request, &[]);

        assert_eq!(observation.stable_prefix_text, "stable prefix");
        assert_eq!(observation.semi_stable_context_text, "semi-stable context");
        assert_eq!(observation.volatile_input_text, "actual request");
        assert!(!observation
            .stable_prefix_text
            .contains("fallback text should not win"));
        assert_eq!(
            observation.prefix_mutation_reasons,
            vec![PrefixMutationReason::SessionSummaryChanged]
        );
    }

    #[test]
    fn build_context_observation_fallback_keeps_dynamic_system_and_developer_text_out_of_stable_prefix(
    ) {
        let request = ProviderRequest {
            model: "gpt-5.4".to_string(),
            input: vec![
                ProviderMessage::system("Stable system instruction"),
                ProviderMessage::developer(
                    "Capability profile: contextWindowTokens=128000 / supportsImageInput=true.",
                ),
                ProviderMessage::developer(
                    "Session summary: volatile summary / graph=graph-a / session=session-1",
                ),
                ProviderMessage::user("actual request"),
            ],
            images: Vec::new(),
            native_messages: Vec::new(),
            observation: ProviderRequestObservation::default(),
            temperature: 0.2,
            max_output_tokens: 1024,
        };

        let observation = build_context_observation(&request, &[]);

        assert!(observation
            .stable_prefix_text
            .contains("Stable system instruction"));
        assert!(observation
            .stable_prefix_text
            .contains("Capability profile:"));
        assert!(!observation
            .stable_prefix_text
            .contains("Session summary: volatile summary"));
        assert!(observation
            .semi_stable_context_text
            .contains("Session summary: volatile summary"));
    }

    #[test]
    fn tool_capability_disabled_removes_openai_tool_fields() {
        let config = ResolvedProviderSelection {
            requested_name: "openai".to_string(),
            provider_name: "openai".to_string(),
            protocol: ProviderProtocol::OpenAiCompletions,
            base_url: "https://api.openai.com/v1".to_string(),
            auth_type: ProviderAuthType::Auto,
            api_key_env_var: "OPENAI_API_KEY".to_string(),
            api_key: Some("test".to_string()),
            model: "gpt-4.1-mini".to_string(),
            temperature: 0.2,
            max_output_tokens: 8192,
            reasoning_effort: None,
            reasoning_budget_tokens: None,
            capabilities: ProviderModelCapabilities {
                context_window_tokens: Some(128_000),
                supports_tools: false,
                supports_streaming: true,
                supports_image_input: true,
                supports_reasoning: false,
                ..Default::default()
            },
            thinking_param_pattern: ThinkingParamPattern::None,
        };

        let body = apply_openai_tool_capability(
            json!({
                "tool_choice": "auto",
                "tools": [{ "type": "function" }]
            }),
            &[ToolDefinition {
                name: "workspace.read_file",
                description: "read file",
                input_schema: json!({ "type": "object" }),
            }],
            &config,
        );

        assert!(body.get("tool_choice").is_none());
        assert!(body.get("tools").is_none());
    }

    #[test]
    fn deepseek_v4_keeps_openai_tool_choice() {
        let config = ResolvedProviderSelection {
            requested_name: "deepseek".to_string(),
            provider_name: "deepseek".to_string(),
            protocol: ProviderProtocol::OpenAiCompletions,
            base_url: "https://api.deepseek.com/v1".to_string(),
            auth_type: ProviderAuthType::Auto,
            api_key_env_var: "DEEPSEEK_API_KEY".to_string(),
            api_key: Some("test".to_string()),
            model: "deepseek-v4-pro".to_string(),
            temperature: 0.2,
            max_output_tokens: 8192,
            reasoning_effort: None,
            reasoning_budget_tokens: None,
            capabilities: ProviderModelCapabilities {
                context_window_tokens: Some(128_000),
                supports_tools: true,
                supports_streaming: true,
                supports_image_input: true,
                supports_reasoning: true,
                ..Default::default()
            },
            thinking_param_pattern: ThinkingParamPattern::EffortWithNone,
        };

        let body = apply_openai_tool_capability(
            json!({
                "tool_choice": "auto",
                "tools": [{ "type": "function", "function": { "name": "workspace_read_file" } }]
            }),
            &[ToolDefinition {
                name: "workspace.read_file",
                description: "read file",
                input_schema: json!({ "type": "object" }),
            }],
            &config,
        );

        assert!(body.get("tool_choice").is_some());
        assert!(body.get("tools").is_some());
    }

    #[test]
    fn tool_capability_disabled_removes_anthropic_tool_fields() {
        let config = ResolvedProviderSelection {
            requested_name: "anthropic".to_string(),
            provider_name: "anthropic".to_string(),
            protocol: ProviderProtocol::AnthropicMessages,
            base_url: "https://api.anthropic.com/v1".to_string(),
            auth_type: ProviderAuthType::Auto,
            api_key_env_var: "ANTHROPIC_API_KEY".to_string(),
            api_key: Some("test".to_string()),
            model: "claude-3-7-sonnet-latest".to_string(),
            temperature: 0.2,
            max_output_tokens: 8192,
            reasoning_effort: None,
            reasoning_budget_tokens: None,
            capabilities: ProviderModelCapabilities {
                context_window_tokens: Some(200_000),
                supports_tools: false,
                supports_streaming: true,
                supports_image_input: true,
                supports_reasoning: true,
                ..Default::default()
            },
            thinking_param_pattern: ThinkingParamPattern::AnthropicThinking,
        };

        let body = apply_anthropic_tool_capability(
            json!({
                "tools": [{ "name": "workspace.read_file" }],
                "tool_choice": { "type": "auto" }
            }),
            &[ToolDefinition {
                name: "workspace.read_file",
                description: "read file",
                input_schema: json!({ "type": "object" }),
            }],
            &config,
        );

        assert!(body.get("tools").is_none());
        assert!(body.get("tool_choice").is_none());
    }

    #[test]
    fn openai_tool_followup_replays_reasoning_content() {
        let request = ProviderRequest {
            model: "deepseek-v4-pro".to_string(),
            input: vec![ProviderMessage::user(
                "检查 src-tauri/src/agent/provider.rs",
            )],
            images: Vec::new(),
            native_messages: Vec::new(),
            observation: ProviderRequestObservation::default(),
            temperature: 0.2,
            max_output_tokens: 1024,
        };
        let tool_call = ToolCall {
            call_id: Some("call_123".to_string()),
            name: "workspace.read_file".to_string(),
            arguments: json!({ "path": "src-tauri/src/agent/provider.rs" }),
            plan: None,
        };
        let tool_result = ToolResult {
            tool_name: "workspace.read_file".to_string(),
            status: "ok".to_string(),
            output: "file content".to_string(),
            duration_ms: 12,
        };

        let messages = openai_messages_with_tool_result(
            &request,
            &mut vec![],
            Some(&provider_native_assistant_tool_call_message(
                Some("先读取文件，再给出结论。"),
                Some("先确认 provider 协议分支，再继续调用工具。"),
                &tool_call,
            )),
            &tool_call,
            &tool_result,
        );

        assert_eq!(messages.len(), 3);
        assert_eq!(
            messages[1].get("reasoning_content").and_then(Value::as_str),
            Some("先确认 provider 协议分支，再继续调用工具。")
        );
    }

    #[test]
    fn openai_tool_followup_preserves_structured_reasoning_content() {
        let request = ProviderRequest {
            model: "deepseek-v4-pro".to_string(),
            input: vec![ProviderMessage::user("继续处理工具结果")],
            images: Vec::new(),
            native_messages: Vec::new(),
            observation: ProviderRequestObservation::default(),
            temperature: 0.2,
            max_output_tokens: 1024,
        };
        let tool_call = ToolCall {
            call_id: Some("call_structured".to_string()),
            name: "workspace.read_file".to_string(),
            arguments: json!({ "path": "src-tauri/src/agent/provider.rs" }),
            plan: None,
        };
        let tool_result = ToolResult {
            tool_name: "workspace.read_file".to_string(),
            status: "ok".to_string(),
            output: "file content".to_string(),
            duration_ms: 12,
        };
        let structured_reasoning = json!([
            { "type": "reasoning", "text": "先确认工具结果。" },
            { "type": "reasoning", "text": "再决定下一步。" }
        ]);
        let assistant_message = json!({
            "role": "assistant",
            "content": "先读取文件，再给出结论。",
            "reasoning_content": structured_reasoning.clone(),
            "tool_calls": [
                {
                    "id": "call_structured",
                    "type": "function",
                    "function": {
                        "name": "workspace_read_file",
                        "arguments": "{\"path\":\"src-tauri/src/agent/provider.rs\"}"
                    }
                }
            ]
        });

        let messages = openai_messages_with_tool_result(
            &request,
            &mut vec![],
            Some(&assistant_message),
            &tool_call,
            &tool_result,
        );

        assert_eq!(messages.len(), 3);
        assert_eq!(
            messages[1].get("reasoning_content"),
            Some(&structured_reasoning)
        );
    }

    #[test]
    fn anthropic_tool_followup_replays_reasoning_content_as_thinking_blocks() {
        let tool_call = ToolCall {
            call_id: Some("toolu_reasoning".to_string()),
            name: "List".to_string(),
            arguments: json!({ "path": "." }),
            plan: None,
        };
        let tool_result = ToolResult {
            tool_name: "List".to_string(),
            status: "ok".to_string(),
            output: "README.md\nsrc".to_string(),
            duration_ms: 0,
        };
        let request = ProviderRequest {
            model: "deepseek-v4-flash".to_string(),
            input: vec![ProviderMessage {
                role: ProviderRole::User,
                content: "当前文件夹下有哪些文件？".to_string(),
            }],
            images: vec![],
            native_messages: vec![],
            observation: ProviderRequestObservation::default(),
            temperature: 0.0,
            max_output_tokens: 1024,
        };
        let assistant_message = json!({
            "role": "assistant",
            "content": "我先列出当前目录。",
            "reasoning_content": [
                {
                    "type": "thinking",
                    "thinking": "先调用目录列表工具。"
                }
            ]
        });

        let messages = anthropic_messages_with_tool_result(
            &request,
            &mut Vec::new(),
            Some(&assistant_message),
            &tool_call,
            &tool_result,
        );

        let assistant_content = messages[1]
            .get("content")
            .and_then(Value::as_array)
            .expect("assistant content should be blocks");
        assert_eq!(
            assistant_content[0].get("type").and_then(Value::as_str),
            Some("thinking")
        );
        assert_eq!(
            assistant_content[0].get("thinking").and_then(Value::as_str),
            Some("先调用目录列表工具。")
        );
        assert!(assistant_content
            .iter()
            .any(|block| block.get("type").and_then(Value::as_str) == Some("tool_use")));
    }

    #[test]
    fn anthropic_tool_followup_accumulates_prior_tool_results_across_hops() {
        let request = ProviderRequest {
            model: "claude-sonnet-5".to_string(),
            input: vec![ProviderMessage::user("当前文件夹下有哪些文件？")],
            images: Vec::new(),
            native_messages: Vec::new(),
            observation: ProviderRequestObservation::default(),
            temperature: 0.0,
            max_output_tokens: 1024,
        };
        let list_call = ToolCall {
            call_id: Some("toolu_list".to_string()),
            name: "List".to_string(),
            arguments: json!({ "path": ".", "description": "列出当前目录" }),
            plan: None,
        };
        let read_call = ToolCall {
            call_id: Some("toolu_read".to_string()),
            name: "Read".to_string(),
            arguments: json!({ "path": "Cargo.toml", "description": "读取项目配置" }),
            plan: None,
        };
        let list_message = json!({
            "role": "assistant",
            "content": [{
                "type": "tool_use",
                "id": "toolu_list",
                "name": "List",
                "input": list_call.arguments
            }]
        });
        let read_message = json!({
            "role": "assistant",
            "content": [{
                "type": "tool_use",
                "id": "toolu_read",
                "name": "Read",
                "input": read_call.arguments
            }]
        });
        let list_result = ToolResult {
            tool_name: "List".to_string(),
            status: "ok".to_string(),
            output: "Cargo.toml\nsrc".to_string(),
            duration_ms: 1,
        };
        let read_result = ToolResult {
            tool_name: "Read".to_string(),
            status: "ok".to_string(),
            output: "[workspace]\nmembers = []".to_string(),
            duration_ms: 1,
        };
        let mut accumulated = Vec::new();

        let first = anthropic_messages_with_tool_result(
            &request,
            &mut accumulated,
            Some(&list_message),
            &list_call,
            &list_result,
        );
        let second = anthropic_messages_with_tool_result(
            &request,
            &mut accumulated,
            Some(&read_message),
            &read_call,
            &read_result,
        );

        assert_eq!(first.len(), 3);
        assert_eq!(second.len(), 5);
        assert_eq!(second[1]["content"][0]["id"].as_str(), Some("toolu_list"));
        assert_eq!(
            second[2]["content"][0]["tool_use_id"].as_str(),
            Some("toolu_list")
        );
        assert_eq!(second[3]["content"][0]["id"].as_str(), Some("toolu_read"));
        assert_eq!(
            second[4]["content"][0]["tool_use_id"].as_str(),
            Some("toolu_read")
        );
    }

    #[test]
    fn anthropic_live_followup_sends_tool_use_and_result_in_one_conversation() {
        let runtime = tokio::runtime::Runtime::new().expect("create Tokio runtime");
        let _guard = runtime.enter();
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock Anthropic server");
        let address = listener.local_addr().expect("read mock server address");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let requests_for_thread = Arc::clone(&requests);
        let responses = vec![
            json!({
                "content": [{
                    "type": "tool_use",
                    "id": "toolu_list",
                    "name": "List",
                    "input": { "path": ".", "description": "列出当前目录" }
                }],
                "usage": { "input_tokens": 1, "output_tokens": 1 }
            }),
            json!({
                "content": [{ "type": "text", "text": "Cargo.toml 和 src。" }],
                "usage": { "input_tokens": 2, "output_tokens": 2 }
            }),
        ];
        let handle = thread::spawn(move || {
            for response in responses {
                let (mut stream, _) = listener.accept().expect("accept mock request");
                let mut bytes = Vec::new();
                let mut chunk = [0_u8; 4096];
                let mut header_end = None;
                let mut content_length = 0usize;
                loop {
                    let read = stream.read(&mut chunk).expect("read mock request");
                    if read == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&chunk[..read]);
                    if header_end.is_none() {
                        header_end = bytes
                            .windows(4)
                            .position(|window| window == b"\r\n\r\n")
                            .map(|index| index + 4);
                        if let Some(end) = header_end {
                            let headers = String::from_utf8_lossy(&bytes[..end]);
                            content_length = headers
                                .lines()
                                .find_map(|line| {
                                    let (name, value) = line.split_once(':')?;
                                    name.eq_ignore_ascii_case("content-length")
                                        .then(|| value.trim().parse::<usize>().ok())
                                        .flatten()
                                })
                                .expect("content length");
                        }
                    }
                    if let Some(end) = header_end {
                        if bytes.len() >= end + content_length {
                            break;
                        }
                    }
                }
                let end = header_end.expect("HTTP header end");
                requests_for_thread
                    .lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&bytes[end..end + content_length]).to_string());
                let body = response.to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("write response");
                stream.flush().expect("flush response");
            }
        });
        let config = ResolvedProviderSelection {
            requested_name: "anthropic-test".to_string(),
            provider_name: "anthropic-test".to_string(),
            protocol: ProviderProtocol::AnthropicMessages,
            base_url: format!("http://{}", address),
            auth_type: ProviderAuthType::XApiKey,
            api_key_env_var: "ANTHROPIC_API_KEY".to_string(),
            api_key: Some("test".to_string()),
            model: "claude-sonnet-5".to_string(),
            temperature: 0.0,
            max_output_tokens: 1024,
            reasoning_effort: None,
            reasoning_budget_tokens: None,
            capabilities: ProviderModelCapabilities {
                supports_tools: true,
                supports_streaming: false,
                ..Default::default()
            },
            thinking_param_pattern: ThinkingParamPattern::None,
        };
        let manager = ProviderManager::new(config);
        let request = ProviderRequest {
            model: "claude-sonnet-5".to_string(),
            input: vec![ProviderMessage::user("当前文件夹下有哪些文件？")],
            images: Vec::new(),
            native_messages: Vec::new(),
            observation: ProviderRequestObservation::default(),
            temperature: 0.0,
            max_output_tokens: 1024,
        };
        let tools = vec![ToolDefinition {
            name: "List",
            description: "列出当前目录。",
            input_schema: json!({ "type": "object", "properties": {} }),
        }];

        let decision = manager
            .decide_with_tools(&request, &tools)
            .expect("first tool decision");
        let tool_call = decision.tool_call.expect("List tool call");
        let response = manager
            .continue_with_tool_result(
                &request,
                &tools,
                &mut Vec::new(),
                decision.assistant_message.as_ref(),
                &tool_call,
                &ToolResult {
                    tool_name: "List".to_string(),
                    status: "ok".to_string(),
                    output: "Cargo.toml\nsrc".to_string(),
                    duration_ms: 1,
                },
            )
            .expect("follow-up response");
        handle.join().expect("join mock server");

        let requests = requests.lock().unwrap();
        let followup_body: Value = serde_json::from_str(&requests[1]).expect("follow-up JSON");
        let messages = followup_body["messages"]
            .as_array()
            .expect("follow-up messages");
        assert_eq!(response.output_text, "Cargo.toml 和 src。");
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0]["role"].as_str(), Some("user"));
        assert_eq!(messages[1]["role"].as_str(), Some("assistant"));
        assert_eq!(messages[1]["content"][0]["type"].as_str(), Some("tool_use"));
        assert_eq!(messages[1]["content"][0]["id"].as_str(), Some("toolu_list"));
        assert_eq!(messages[2]["role"].as_str(), Some("user"));
        assert_eq!(
            messages[2]["content"][0]["tool_use_id"].as_str(),
            Some("toolu_list")
        );
        assert_eq!(
            messages[2]["content"][0]["content"][0]["text"].as_str(),
            Some("Cargo.toml\nsrc")
        );
    }

    #[test]
    fn openai_tool_followup_filters_unexecuted_parallel_tool_calls() {
        let request = ProviderRequest {
            model: "gpt-5.4".to_string(),
            input: vec![ProviderMessage::user("检查 capabilities 目录")],
            images: Vec::new(),
            native_messages: Vec::new(),
            observation: ProviderRequestObservation::default(),
            temperature: 0.2,
            max_output_tokens: 1024,
        };
        let tool_call = ToolCall {
            call_id: Some("call_path_info".to_string()),
            name: "workspace_path_info".to_string(),
            arguments: json!({ "path": "capabilities" }),
            plan: None,
        };
        let tool_result = ToolResult {
            tool_name: "workspace_path_info".to_string(),
            status: "ok".to_string(),
            output: "{\"kind\":\"directory\"}".to_string(),
            duration_ms: 10,
        };
        let assistant_message = json!({
            "role": "assistant",
            "content": "",
            "reasoning_content": "需要查看目录信息。",
            "tool_calls": [
                {
                    "id": "call_path_info",
                    "type": "function",
                    "function": {
                        "name": "workspace_path_info",
                        "arguments": "{\"path\":\"capabilities\"}"
                    }
                },
                {
                    "id": "call_list_files",
                    "type": "function",
                    "function": {
                        "name": "workspace_list_files",
                        "arguments": "{\"path\":\"capabilities\"}"
                    }
                }
            ]
        });

        let messages = openai_messages_with_tool_result(
            &request,
            &mut vec![],
            Some(&assistant_message),
            &tool_call,
            &tool_result,
        );

        let assistant_tool_calls = messages[1]
            .get("tool_calls")
            .and_then(Value::as_array)
            .expect("assistant tool calls");
        assert_eq!(assistant_tool_calls.len(), 1);
        assert_eq!(
            assistant_tool_calls[0].get("id").and_then(Value::as_str),
            Some("call_path_info")
        );
        assert_eq!(
            messages[2].get("tool_call_id").and_then(Value::as_str),
            Some("call_path_info")
        );
        assert!(!serde_json::to_string(&messages)
            .expect("messages serialize")
            .contains("call_list_files"));
    }

    #[test]
    fn openai_tool_followup_fallback_rebuild_keeps_structured_reasoning_content() {
        let tool_call = ToolCall {
            call_id: Some("call_repaired".to_string()),
            name: "workspace_read_file".to_string(),
            arguments: json!({ "path": "Cargo.toml" }),
            plan: None,
        };
        let structured_reasoning = json!([
            { "type": "reasoning", "text": "先修正工具调用，再继续。" }
        ]);
        let assistant_message = json!({
            "role": "assistant",
            "content": "继续读取文件",
            "reasoning_content": structured_reasoning.clone(),
            "tool_calls": [
                {
                    "id": "call_other",
                    "type": "function",
                    "function": {
                        "name": "workspace_list_files",
                        "arguments": "{\"path\":\".\"}"
                    }
                }
            ]
        });

        let rebuilt =
            openai_assistant_message_for_tool_result(Some(&assistant_message), &tool_call);

        assert_eq!(
            rebuilt.get("reasoning_content"),
            Some(&structured_reasoning)
        );
        assert_eq!(
            rebuilt
                .get("tool_calls")
                .and_then(Value::as_array)
                .and_then(|calls| calls.first())
                .and_then(|call| call.get("function"))
                .and_then(|function| function.get("name"))
                .and_then(Value::as_str),
            Some("workspace_read_file")
        );
    }

    #[test]
    fn openai_tool_followup_keeps_small_tool_output_raw() {
        let tool_call = ToolCall {
            call_id: Some("call_small".to_string()),
            name: "workspace.read_file".to_string(),
            arguments: json!({ "path": "src/main.rs" }),
            plan: None,
        };
        let tool_result = ToolResult {
            tool_name: "workspace.read_file".to_string(),
            status: "ok".to_string(),
            output: "small output".to_string(),
            duration_ms: 3,
        };

        let message = openai_followup_tool_result_message(&tool_call, &tool_result);

        assert_eq!(
            message.get("content").and_then(Value::as_str),
            Some("small output")
        );
    }

    /// DeepSeek thinking 模式：本地 planner 决策（assistant_message=None）的
    /// follow-up 必须回传非空 reasoning_content，否则 API 返回 400。
    #[test]
    fn openai_tool_followup_without_provider_assistant_message_still_echoes_reasoning() {
        let tool_call = ToolCall {
            call_id: Some("call_planner_batch".to_string()),
            name: "workspace_batch".to_string(),
            arguments: json!({
                "calls": [
                    { "name": "workspace_gather_context", "arguments": { "path": "src" } }
                ]
            }),
            plan: None,
        };

        let rebuilt = openai_assistant_message_for_tool_result(None, &tool_call);

        assert_eq!(
            rebuilt.get("role").and_then(Value::as_str),
            Some("assistant")
        );
        let reasoning = rebuilt
            .get("reasoning_content")
            .and_then(Value::as_str)
            .expect("planner follow-up must echo non-empty reasoning_content");
        assert!(!reasoning.trim().is_empty());
        assert!(rebuilt.get("tool_calls").is_some());
    }

    /// DeepSeek thinking 模式：assistant 消息带 tool_calls 但 reasoning_content
    /// 缺失或为空时，normalize 必须兜底填充非空占位文本。
    #[test]
    fn openai_normalize_fills_reasoning_placeholder_for_tool_call_message_without_reasoning() {
        let message = json!({
            "role": "assistant",
            "content": "",
            "tool_calls": [
                {
                    "id": "call_x",
                    "type": "function",
                    "function": {
                        "name": "WebSearch",
                        "arguments": "{\"query\":\"migration\"}"
                    }
                }
            ]
        });

        let normalized = normalize_openai_assistant_message(&message);

        let reasoning = normalized
            .get("reasoning_content")
            .and_then(Value::as_str)
            .expect("tool-call assistant message must carry non-empty reasoning_content");
        assert!(!reasoning.trim().is_empty());
        assert_eq!(reasoning, TOOL_CALL_REASONING_PLACEHOLDER);
    }

    #[test]
    fn openai_tool_followup_truncates_large_tool_output() {
        let tool_call = ToolCall {
            call_id: Some("call_large".to_string()),
            name: "workspace.gather_context".to_string(),
            arguments: json!({ "paths": ["src"] }),
            plan: None,
        };
        let large_output = format!(
            "{{\"files\":\"{}\",\"tail\":\"{}\"}}",
            "a".repeat(OPENAI_TOOL_RESULT_INLINE_LIMIT_CHARS),
            "z".repeat(4_000)
        );
        let tool_result = ToolResult {
            tool_name: "workspace.gather_context".to_string(),
            status: "ok".to_string(),
            output: large_output.clone(),
            duration_ms: 42,
        };

        let message = openai_followup_tool_result_message(&tool_call, &tool_result);
        let content = message
            .get("content")
            .and_then(Value::as_str)
            .expect("content string");
        let payload: Value = serde_json::from_str(content).expect("compressed json payload");

        assert!(content.chars().count() < large_output.chars().count());
        assert_eq!(
            payload
                .get("compression")
                .and_then(|value| value.get("applied"))
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            payload
                .get("summary")
                .and_then(|value| value.get("kind"))
                .and_then(Value::as_str),
            Some("json_object")
        );
        assert_eq!(
            payload
                .get("summary")
                .and_then(|value| value.get("keys"))
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(2)
        );
        assert!(payload.get("head").and_then(Value::as_str).is_some());
        assert!(payload.get("tail").and_then(Value::as_str).is_some());
    }

    #[test]
    fn openai_sse_reader_streams_reasoning_and_text_in_order() {
        let raw_text = concat!(
            ": keep-alive\n\n",
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"先看工具输出。\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"第一段\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"第二段\"}}]}\n\n",
            "data: [DONE]\n\n"
        );
        let mut deltas = Vec::new();

        let message = collect_openai_sse_message_from_reader(
            std::io::Cursor::new(raw_text.as_bytes()),
            "unit-test",
            &mut |delta| deltas.push(delta),
        )
        .expect("stream message should parse");

        assert_eq!(message.output_text, "第一段第二段");
        assert_eq!(message.reasoning_content.as_deref(), Some("先看工具输出。"));
        assert_eq!(
            deltas,
            vec![
                ProviderStreamChunk::Reasoning("先看工具输出。".to_string()),
                ProviderStreamChunk::Text("第一段".to_string()),
                ProviderStreamChunk::Text("第二段".to_string())
            ]
        );
    }

    #[test]
    fn openai_sse_string_collector_still_parses_full_payload() {
        let raw_text = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"alpha\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"beta\"}}]}\n\n",
            "data: [DONE]\n\n"
        );
        let mut deltas = Vec::new();

        let message = collect_openai_sse_message(raw_text, &mut |delta| deltas.push(delta))
            .expect("string helper should parse");

        assert_eq!(message.output_text, "alphabeta");
        assert_eq!(message.reasoning_content, None);
        assert_eq!(
            deltas,
            vec![
                ProviderStreamChunk::Text("alpha".to_string()),
                ProviderStreamChunk::Text("beta".to_string())
            ]
        );
    }

    #[test]
    fn openai_sse_reader_extracts_usage_from_terminal_chunk() {
        let raw_text = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"alpha\"}}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":120,\"prompt_cache_hit_tokens\":48,\"completion_tokens\":32,\"total_tokens\":152}}\n\n",
            "data: [DONE]\n\n"
        );

        let message = collect_openai_sse_message_from_reader(
            std::io::Cursor::new(raw_text.as_bytes()),
            "unit-test",
            &mut |_delta| {},
        )
        .expect("stream message should parse");

        let usage = message.token_usage.expect("usage should be captured");
        assert_eq!(usage.input_tokens, Some(120));
        assert_eq!(usage.cache_hit_input_tokens, Some(48));
        assert_eq!(usage.output_tokens, Some(32));
        assert_eq!(usage.total_tokens, Some(152));
    }

    #[test]
    fn openai_reasoning_tool_followup_stream_attempts_live_stream_before_fallback() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _guard = rt.enter();
        let config = ResolvedProviderSelection {
            requested_name: "ppx".to_string(),
            provider_name: "ppx".to_string(),
            protocol: ProviderProtocol::OpenAiCompletions,
            base_url: "http://127.0.0.1:1/v1".to_string(),
            auth_type: ProviderAuthType::Auto,
            api_key_env_var: "PPX_API_KEY".to_string(),
            api_key: Some("test".to_string()),
            model: "gpt-5.4".to_string(),
            temperature: 0.2,
            max_output_tokens: 8192,
            reasoning_effort: None,
            reasoning_budget_tokens: None,
            capabilities: ProviderModelCapabilities {
                context_window_tokens: Some(200_000),
                supports_tools: true,
                supports_streaming: true,
                supports_image_input: false,
                supports_reasoning: true,
                ..Default::default()
            },
            thinking_param_pattern: ThinkingParamPattern::EffortStandard,
        };
        let manager = ProviderManager::new(config);
        let request = ProviderRequest {
            model: "gpt-5.4".to_string(),
            input: vec![ProviderMessage::user("继续总结工具结果")],
            images: Vec::new(),
            native_messages: Vec::new(),
            observation: ProviderRequestObservation::default(),
            temperature: 0.2,
            max_output_tokens: 1024,
        };
        let tool_call = ToolCall {
            call_id: Some("call_followup".to_string()),
            name: "workspace.gather_context".to_string(),
            arguments: json!({ "paths": ["src-tauri/src/agent/provider.rs"] }),
            plan: None,
        };
        let tool_result = ToolResult {
            tool_name: "workspace.gather_context".to_string(),
            status: "ok".to_string(),
            output: "{\"summary\":{\"text\":\"已读取 provider follow-up 相关实现\"}}".to_string(),
            duration_ms: 18,
        };
        let mut deltas = Vec::new();

        let response = manager
            .continue_with_tool_result_stream(
                &request,
                &[ToolDefinition {
                    name: "workspace.gather_context",
                    description: "gather context",
                    input_schema: json!({ "type": "object" }),
                }],
                &mut vec![],
                None,
                &tool_call,
                &tool_result,
                |delta| deltas.push(delta),
            )
            .expect("follow-up should degrade after stream failure");

        assert_eq!(response.provider_source, "provider_followup_local_fallback");
        assert_eq!(response.provider_mode, "fallback");
        assert!(response
            .fallback_reason
            .as_deref()
            .unwrap_or_default()
            .contains("provider_followup_failed"));
        assert!(deltas.is_empty());
    }

    #[test]
    fn provider_timeout_classifier_matches_timeout_errors() {
        let policy = ProviderRetryPolicy::new(BackoffConfig::default());
        assert!(policy
            .classify("调用 provider 失败：operation timed out；type=timeout；elapsed=45000ms")
            .is_retryable());
        assert!(!policy.classify("provider 返回错误状态：500").is_retryable());
    }

    #[test]
    fn retry_provider_timeout_stops_after_non_timeout_error() {
        let attempts = std::cell::Cell::new(0);
        let result: Result<(), String> = retry_provider_timeout("decision", || {
            attempts.set(attempts.get() + 1);
            Err("provider returned 500".to_string())
        });

        assert!(result.is_err());
        assert_eq!(attempts.get(), 1);
    }

    #[test]
    fn retry_provider_timeout_retries_until_success() {
        let attempts = std::cell::Cell::new(0);
        let result = retry_provider_timeout("decision", || {
            let next = attempts.get() + 1;
            attempts.set(next);
            if next < 3 {
                Err("调用 provider 失败：operation timed out；type=timeout".to_string())
            } else {
                Ok("ok")
            }
        })
        .expect("timeout retry should eventually succeed");

        assert_eq!(result, "ok");
        assert_eq!(attempts.get(), 3);
    }

    // ── PA-100：rate-limit 感知的 followup 长退避 ───────────────────────────────────────────

    #[test]
    fn is_rate_limit_error_matches_provider_wording() {
        // PA-143：命中必须来自**措辞**（tpm），而不是 JSON 错误码里的数字子串。
        assert!(is_rate_limit_error(
            "{\"error\":{\"message\":\"inference tpm exhausted\",\"code\":\"1001\"}}"
        ));
        // PA-143：`"code":"429001"` 只是更长数字串内的 `429` 片段，不构成限流契约
        // （旧断言把该子串命中当契约，正是缺陷所在）。
        assert!(!is_rate_limit_error(
            "{\"error\":{\"message\":\"invalid request\",\"code\":\"429001\"}}"
        ));
        assert!(is_rate_limit_error("HTTP 429 Too Many Requests"));
        assert!(is_rate_limit_error("rate limit exceeded for org"));
        assert!(is_rate_limit_error("monthly quota exhausted"));
        assert!(!is_rate_limit_error("provider returned 500"));
        assert!(!is_rate_limit_error("operation timed out"));
    }

    #[test]
    fn followup_rate_limit_switches_to_long_backoff_schedule() {
        let sleeper = crate::agent::retry::FakeSleeper::new();
        let attempts = std::cell::Cell::new(0);
        let (result, report): (Result<(), String>, SingleHopRetryReport) =
            retry_provider_scoped_with_report(
                "followup_sync",
                ProviderRetryScope::Followup,
                || {
                    attempts.set(attempts.get() + 1);
                    Err(
                        "{\"error\":{\"message\":\"inference tpm exhausted\",\"code\":\"429001\"}}"
                            .to_string(),
                    )
                },
                &sleeper,
                None,
            );

        assert!(result.is_err());
        // 切换后调度：sleep 20s → 重试 → sleep 40s → 重试 → 预算耗尽。
        assert_eq!(attempts.get(), 3);
        assert_eq!(
            sleeper
                .total_slept
                .load(std::sync::atomic::Ordering::Relaxed),
            60_000
        );
        // T2-B：单跳报告口径（attempts=operation 调用次数；slept=sleep-only 累计）。
        assert_eq!(report.attempts_total, 3);
        assert_eq!(report.slept_ms, 60_000);
        assert!(report.switched_to_rl);
        assert_eq!(report.scope, "followup");
        assert_eq!(report.exhausted_reason, Some("attempt_limit_exhausted"));
    }

    #[test]
    fn decision_scope_aborts_immediately_on_rate_limit_errors() {
        let sleeper = crate::agent::retry::FakeSleeper::new();
        let attempts = std::cell::Cell::new(0);
        let (result, report): (Result<(), String>, SingleHopRetryReport) =
            retry_provider_scoped_with_report(
                "decision",
                ProviderRetryScope::Decision,
                || {
                    attempts.set(attempts.get() + 1);
                    Err("HTTP 429 too many requests".to_string())
                },
                &sleeper,
                None,
            );

        assert!(result.is_err());
        // PA-171 C3-P2（反转 PA-100 的「首跳短退避」）：限流/配额首跳即 Abort——
        // 只 1 次尝试、0 次 sleep（冷却窗口由 provider HTTP 出口登记，不再白烧 7.5s）。
        assert_eq!(attempts.get(), 1);
        assert_eq!(
            sleeper
                .total_slept
                .load(std::sync::atomic::Ordering::Relaxed),
            0
        );
        // T2-B：Decision 作用域从不切换 RL 配置。
        assert_eq!(report.attempts_total, 1);
        assert_eq!(report.slept_ms, 0);
        assert!(!report.switched_to_rl);
        assert_eq!(report.scope, "decision");
    }

    /// PA-171 C3-P6 反向钉子：Decision 作用域的非限流瞬时错误（503）必须保留既有重试。
    #[test]
    fn decision_scope_keeps_retries_for_non_rate_limit_transient_errors() {
        let sleeper = crate::agent::retry::FakeSleeper::new();
        let attempts = std::cell::Cell::new(0);
        let result: Result<(), String> = retry_provider_scoped_with_sleeper(
            "decision",
            ProviderRetryScope::Decision,
            || {
                attempts.set(attempts.get() + 1);
                Err("provider 返回错误状态：503 Service Unavailable".to_string())
            },
            &sleeper,
            None,
        );

        assert!(result.is_err());
        // 503 不是限流 ⇒ 维持既有 5 次短退避（0.5+1+2+4=7.5s）。
        assert_eq!(attempts.get(), 5);
        assert_eq!(
            sleeper
                .total_slept
                .load(std::sync::atomic::Ordering::Relaxed),
            7_500
        );
    }

    #[test]
    fn followup_timeout_failures_do_not_switch_to_rate_limit_schedule() {
        let sleeper = crate::agent::retry::FakeSleeper::new();
        let attempts = std::cell::Cell::new(0);
        let result: Result<(), String> = retry_provider_scoped_with_sleeper(
            "followup_sync",
            ProviderRetryScope::Followup,
            || {
                attempts.set(attempts.get() + 1);
                Err("调用 provider 失败：operation timed out；type=timeout".to_string())
            },
            &sleeper,
            None,
        );

        assert!(result.is_err());
        assert_eq!(attempts.get(), 5);
        assert_eq!(
            sleeper
                .total_slept
                .load(std::sync::atomic::Ordering::Relaxed),
            7_500
        );
    }

    /// 裁决记录（PA-100 审核）：报文同时含 timeout 与 rate-limit 特征时按
    /// rate-limit 处理——长退避对两类瞬时故障都安全，反向必然撞限流窗口。
    #[test]
    fn mixed_timeout_and_rate_limit_message_takes_rate_limit_schedule_in_followup() {
        let sleeper = crate::agent::retry::FakeSleeper::new();
        let attempts = std::cell::Cell::new(0);
        let result: Result<(), String> = retry_provider_scoped_with_sleeper(
            "followup_stream",
            ProviderRetryScope::Followup,
            || {
                attempts.set(attempts.get() + 1);
                Err(
                    "request to upstream timed out; upstream said: 429 inference tpm exhausted"
                        .to_string(),
                )
            },
            &sleeper,
            None,
        );

        assert!(result.is_err());
        assert_eq!(attempts.get(), 3);
        assert_eq!(
            sleeper
                .total_slept
                .load(std::sync::atomic::Ordering::Relaxed),
            60_000
        );
    }

    /// PA-100 审核 A 缺失测试①：RL 切换发生在 base 阶段已消耗部分时间预算之后，
    /// 结转的 elapsed 必须计入 RL budget（时间口径跨配置累计）。
    #[test]
    fn followup_rate_limit_switch_carries_elapsed_budget_across_configs() {
        let sleeper = crate::agent::retry::FakeSleeper::new();
        let attempts = std::cell::Cell::new(0);
        let result: Result<(), String> = retry_provider_scoped_with_sleeper(
            "followup_sync",
            ProviderRetryScope::Followup,
            || {
                let next = attempts.get() + 1;
                attempts.set(next);
                if next <= 2 {
                    Err("调用 provider 失败：operation timed out；type=timeout".to_string())
                } else {
                    Err(
                        "{\"error\":{\"message\":\"inference tpm exhausted\",\"code\":\"429001\"}}"
                            .to_string(),
                    )
                }
            },
            &sleeper,
            None,
        );

        assert!(result.is_err());
        // 序列：t/o(无睡) → t/o(sleep0.5s) → 429 切换 → sleep20s → 429 → sleep40s → 429 耗尽。
        assert_eq!(attempts.get(), 5);
        assert_eq!(
            sleeper
                .total_slept
                .load(std::sync::atomic::Ordering::Relaxed),
            61_500
        );
    }

    /// PA-100 code-review B P1-2：流式增量已发出后，重试必须在 sleep 前放弃，
    /// 且不得切换 RL 长退避（否则白睡 60s 后仍必然失败）。
    #[test]
    fn followup_stream_hint_stops_retries_without_rate_limit_backoff() {
        let sleeper = crate::agent::retry::FakeSleeper::new();
        let streamed = std::sync::atomic::AtomicBool::new(false);
        let attempts = std::cell::Cell::new(0);
        let result: Result<(), String> = retry_provider_scoped_with_sleeper(
            "followup_stream",
            ProviderRetryScope::Followup,
            || {
                let next = attempts.get() + 1;
                attempts.set(next);
                if next == 1 {
                    streamed.store(true, std::sync::atomic::Ordering::Relaxed);
                    Err(
                        "{\"error\":{\"message\":\"inference tpm exhausted\",\"code\":\"429001\"}}"
                            .to_string(),
                    )
                } else {
                    Err("stream already emitted delta; auto-retry forbidden".to_string())
                }
            },
            &sleeper,
            Some(&|| streamed.load(std::sync::atomic::Ordering::Relaxed)),
        );

        assert!(result.is_err());
        // hint 阻断切换与 sleep：立即返回首跳错误进 sync fallback。
        assert_eq!(attempts.get(), 1);
        assert_eq!(
            sleeper
                .total_slept
                .load(std::sync::atomic::Ordering::Relaxed),
            0
        );
    }

    /// T2-B：quota-only 报文不触发 RL 长退避——`classify` 不含 quota 特征，
    /// 直接 Abort（安全方向：不在永久配额耗尽上烧退避）。
    #[test]
    fn single_hop_report_quota_only_aborts_without_retry() {
        let sleeper = crate::agent::retry::FakeSleeper::new();
        let attempts = std::cell::Cell::new(0);
        let (result, report): (Result<(), String>, SingleHopRetryReport) =
            retry_provider_scoped_with_report(
                "followup_sync",
                ProviderRetryScope::Followup,
                || {
                    attempts.set(attempts.get() + 1);
                    Err("monthly quota exhausted".to_string())
                },
                &sleeper,
                None,
            );

        assert!(result.is_err());
        assert_eq!(attempts.get(), 1);
        assert_eq!(
            sleeper
                .total_slept
                .load(std::sync::atomic::Ordering::Relaxed),
            0
        );
        assert_eq!(report.attempts_total, 1);
        assert_eq!(report.slept_ms, 0);
        assert!(!report.switched_to_rl);
        assert_eq!(report.scope, "followup");
        assert_eq!(report.exhausted_reason, None);
    }

    /// T2-B F2：quota+429 混合走 RL 长退避——报文同时含 quota 与 429 特征，
    /// `classify` 先判 TransientRetryable，再由 `is_rate_limit_error` 命中切 RL。
    /// 与 quota-only Abort 测试并列，锁住混合语义（残余：永久配额场景白烧 ~60s）。
    #[test]
    fn single_hop_report_quota_and_429_mixed_follows_rate_limit_schedule() {
        let sleeper = crate::agent::retry::FakeSleeper::new();
        let attempts = std::cell::Cell::new(0);
        let (result, report): (Result<(), String>, SingleHopRetryReport) =
            retry_provider_scoped_with_report(
                "followup_sync",
                ProviderRetryScope::Followup,
                || {
                    attempts.set(attempts.get() + 1);
                    Err("HTTP 429 quota exceeded for org".to_string())
                },
                &sleeper,
                None,
            );

        assert!(result.is_err());
        // RL 调度：sleep 20s → 重试 → sleep 40s → 重试 → 预算耗尽。
        assert_eq!(attempts.get(), 3);
        assert_eq!(
            sleeper
                .total_slept
                .load(std::sync::atomic::Ordering::Relaxed),
            60_000
        );
        assert_eq!(report.attempts_total, 3);
        assert_eq!(report.slept_ms, 60_000);
        assert!(report.switched_to_rl);
        assert_eq!(report.scope, "followup");
        assert_eq!(report.exhausted_reason, Some("attempt_limit_exhausted"));
    }

    /// T2-B：首跳 429 切 RL（sleep 20s），第二跳前 hint 变真则在 sleep 前放弃——
    /// attempts==2、sleep==20_000、switched==true。
    #[test]
    fn single_hop_report_rl_then_hint_aborts_before_second_sleep() {
        let sleeper = crate::agent::retry::FakeSleeper::new();
        let hint = std::sync::atomic::AtomicBool::new(false);
        let attempts = std::cell::Cell::new(0);
        let (result, report): (Result<(), String>, SingleHopRetryReport) =
            retry_provider_scoped_with_report(
                "followup_stream",
                ProviderRetryScope::Followup,
                || {
                    let next = attempts.get() + 1;
                    attempts.set(next);
                    if next == 2 {
                        // 第二跳已进入 RL 调度（首跳后 sleep 20s），此时增量发出，
                        // hint 变真 → 循环顶在第二次 sleep 前放弃。
                        hint.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                    Err(
                        "{\"error\":{\"message\":\"inference tpm exhausted\",\"code\":\"429001\"}}"
                            .to_string(),
                    )
                },
                &sleeper,
                Some(&|| hint.load(std::sync::atomic::Ordering::Relaxed)),
            );

        assert!(result.is_err());
        assert_eq!(attempts.get(), 2);
        assert_eq!(
            sleeper
                .total_slept
                .load(std::sync::atomic::Ordering::Relaxed),
            20_000
        );
        assert_eq!(report.attempts_total, 2);
        assert_eq!(report.slept_ms, 20_000);
        assert!(report.switched_to_rl);
        assert_eq!(report.scope, "followup");
        assert_eq!(report.exhausted_reason, None);
    }

    /// T2-B F4：首试成功——覆盖 report 成功分支 `attempts_total + 1` 口径
    /// （attempts==1、slept==0、switched==false、exhausted==None）。
    #[test]
    fn single_hop_report_first_attempt_success() {
        let sleeper = crate::agent::retry::FakeSleeper::new();
        let attempts = std::cell::Cell::new(0);
        let (result, report): (Result<&str, String>, SingleHopRetryReport) =
            retry_provider_scoped_with_report(
                "decision",
                ProviderRetryScope::Decision,
                || {
                    attempts.set(attempts.get() + 1);
                    Ok("ok")
                },
                &sleeper,
                None,
            );

        assert_eq!(result.expect("first attempt should succeed"), "ok");
        assert_eq!(attempts.get(), 1);
        assert_eq!(
            sleeper
                .total_slept
                .load(std::sync::atomic::Ordering::Relaxed),
            0
        );
        assert_eq!(report.attempts_total, 1);
        assert_eq!(report.slept_ms, 0);
        assert!(!report.switched_to_rl);
        assert_eq!(report.scope, "decision");
        assert_eq!(report.exhausted_reason, None);
    }

    /// T2-B F4：重试后成功——timeout 一次后成功（attempts==2、slept==500、scope 正确）。
    #[test]
    fn single_hop_report_success_after_timeout_retry() {
        let sleeper = crate::agent::retry::FakeSleeper::new();
        let attempts = std::cell::Cell::new(0);
        let (result, report): (Result<&str, String>, SingleHopRetryReport) =
            retry_provider_scoped_with_report(
                "decision",
                ProviderRetryScope::Decision,
                || {
                    let next = attempts.get() + 1;
                    attempts.set(next);
                    if next < 2 {
                        Err("调用 provider 失败：operation timed out；type=timeout".to_string())
                    } else {
                        Ok("ok")
                    }
                },
                &sleeper,
                None,
            );

        assert_eq!(result.expect("retry should eventually succeed"), "ok");
        assert_eq!(attempts.get(), 2);
        assert_eq!(
            sleeper
                .total_slept
                .load(std::sync::atomic::Ordering::Relaxed),
            500
        );
        assert_eq!(report.attempts_total, 2);
        assert_eq!(report.slept_ms, 500);
        assert!(!report.switched_to_rl);
        assert_eq!(report.scope, "decision");
        assert_eq!(report.exhausted_reason, None);
    }

    #[test]
    fn openai_sse_reader_handles_keepalive_and_comment_lines() {
        let raw_text = concat!(
            ": keep-alive\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"alpha\"}}]}\n\n",
            ": some comment\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"beta\"}}]}\n\n",
            "data: [DONE]\n\n"
        );
        let mut deltas = Vec::new();

        let message = collect_openai_sse_message_from_reader(
            std::io::Cursor::new(raw_text.as_bytes()),
            "unit-test",
            &mut |delta| deltas.push(delta),
        )
        .expect("stream with comments should parse");

        assert_eq!(message.output_text, "alphabeta");
        assert_eq!(
            deltas,
            vec![
                ProviderStreamChunk::Text("alpha".to_string()),
                ProviderStreamChunk::Text("beta".to_string()),
            ]
        );
    }

    /// DeepSeek 思考模式会逐片流式返回 reasoning_content。
    /// follow-up 请求必须把完整思考文本回传给 API（否则 DeepSeek 返回
    /// "The `reasoning_content` in the thinking mode must be passed back to the API."），
    /// 因此 reasoning_content_value 不能只保留首个分片。
    #[test]
    fn openai_stream_reasoning_value_echoes_full_reasoning_text() {
        let raw_text = concat!(
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"第一步\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"检查文件结构\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"。\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"结论：已了解。\"}}]}\n\n",
            "data: [DONE]\n\n"
        );
        let mut deltas = Vec::new();

        let message = collect_openai_sse_message_from_reader(
            std::io::Cursor::new(raw_text.as_bytes()),
            "unit-test",
            &mut |delta| deltas.push(delta),
        )
        .expect("stream with reasoning chunks should parse");

        assert_eq!(
            message.reasoning_content.as_deref(),
            Some("第一步检查文件结构。")
        );
        assert_eq!(
            message
                .reasoning_content_value
                .as_ref()
                .and_then(Value::as_str),
            Some("第一步检查文件结构。")
        );
        assert_eq!(
            deltas,
            vec![
                ProviderStreamChunk::Reasoning("第一步".to_string()),
                ProviderStreamChunk::Reasoning("检查文件结构".to_string()),
                ProviderStreamChunk::Reasoning("。".to_string()),
                ProviderStreamChunk::Text("结论：已了解。".to_string()),
            ]
        );
    }

    /// 流式决策后发起 follow-up 时，assistant 消息中的 reasoning_content
    /// 必须携带完整思考文本（DeepSeek thinking 模式的回传校验）。
    #[test]
    fn openai_followup_message_carries_full_reasoning_content() {
        let request = ProviderRequest {
            model: "deepseek-v4-flash".to_string(),
            input: vec![ProviderMessage::user("批量读取文件")],
            images: Vec::new(),
            native_messages: Vec::new(),
            observation: ProviderRequestObservation::default(),
            temperature: 0.0,
            max_output_tokens: 1024,
        };
        let tool_call = ToolCall {
            call_id: Some("call_batch".to_string()),
            name: "workspace_batch".to_string(),
            arguments: json!({
                "calls": [
                    { "name": "workspace_gather_context", "arguments": { "path": "1.0" } }
                ]
            }),
            plan: None,
        };
        let tool_result = ToolResult {
            tool_name: "workspace_batch".to_string(),
            status: "error".to_string(),
            output: "{\"ok\":false}".to_string(),
            duration_ms: 5,
        };
        let full_reasoning = "完整思考文本：先看文件结构。";
        let assistant_message = provider_native_assistant_tool_call_message_with_reasoning_value(
            None,
            Some(&Value::String(full_reasoning.to_string())),
            &tool_call,
        );

        let messages = openai_messages_with_tool_result(
            &request,
            &mut vec![],
            Some(&assistant_message),
            &tool_call,
            &tool_result,
        );

        assert_eq!(messages.len(), 3);
        assert_eq!(
            messages[1].get("reasoning_content").and_then(Value::as_str),
            Some(full_reasoning)
        );
        assert_eq!(
            messages[2].get("tool_call_id").and_then(Value::as_str),
            Some("call_batch")
        );
    }

    /// Simulates a fragmented HTTP chunk stream by feeding a BufReader with
    /// a Read implementation that yields data in fixed-size pieces.
    /// The underlying BufReader handles line buffering — this tests that
    /// the SSE parser correctly processes data regardless of chunk boundaries.
    #[test]
    fn openai_sse_reader_parses_across_read_boundaries() {
        let raw_text = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\" World\"}}]}\n\n",
            "data: [DONE]\n\n"
        );
        let mut deltas = Vec::new();

        // Use a 3-byte Read chunk to force BufReader to refill mid-line
        struct SmallChunks<'a>(&'a [u8], usize);
        impl std::io::Read for SmallChunks<'_> {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                if self.0.is_empty() {
                    return Ok(0);
                }
                let end = self.1.min(self.0.len());
                let chunk = &self.0[..end];
                let len = chunk.len().min(buf.len());
                buf[..len].copy_from_slice(&chunk[..len]);
                self.0 = &self.0[end..];
                Ok(len)
            }
        }

        // Wrap in BufReader so BufRead::lines() works incrementally
        let reader = std::io::BufReader::new(SmallChunks(raw_text.as_bytes(), 3));

        let message = collect_openai_sse_message_from_reader(reader, "unit-test", &mut |delta| {
            deltas.push(delta)
        })
        .expect("fragmented stream should parse");

        assert_eq!(message.output_text, "Hello World");
        assert_eq!(
            deltas,
            vec![
                ProviderStreamChunk::Text("Hello".to_string()),
                ProviderStreamChunk::Text(" World".to_string()),
            ]
        );
    }

    /// Verifies the SSE reader can handle multi-byte UTF-8 content
    /// when the underlying Read yields data in small fragments that
    /// split characters across boundaries.
    #[test]
    fn openai_sse_reader_handles_utf8_across_read_boundaries() {
        let raw_text = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"你好世界\"}}]}\n\n",
            "data: [DONE]\n\n"
        );
        let mut deltas = Vec::new();

        // 3-byte fragments: Chinese UTF-8 is 3 bytes per char, so fragments
        // will regularly split characters (handled by BufReader's buffering)
        struct SmallChunks<'a>(&'a [u8], usize);
        impl std::io::Read for SmallChunks<'_> {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                if self.0.is_empty() {
                    return Ok(0);
                }
                let end = self.1.min(self.0.len());
                let chunk = &self.0[..end];
                let len = chunk.len().min(buf.len());
                buf[..len].copy_from_slice(&chunk[..len]);
                self.0 = &self.0[end..];
                Ok(len)
            }
        }

        let reader = std::io::BufReader::new(SmallChunks(raw_text.as_bytes(), 3));

        let message = collect_openai_sse_message_from_reader(reader, "unit-test", &mut |delta| {
            deltas.push(delta)
        })
        .expect("UTF-8 split across read boundaries should parse");

        assert_eq!(message.output_text, "你好世界");
        assert_eq!(
            deltas,
            vec![ProviderStreamChunk::Text("你好世界".to_string())]
        );
    }

    #[test]
    fn sanitize_request_strips_images_for_non_vision_model() {
        let manager = ProviderManager::new(ResolvedProviderSelection {
            requested_name: "test-deepseek".to_string(),
            provider_name: "deepseek".to_string(),
            protocol: ProviderProtocol::OpenAiCompletions,
            base_url: "https://api.deepseek.com/v1".to_string(),
            auth_type: ProviderAuthType::Auto,
            api_key_env_var: "DEEPSEEK_API_KEY".to_string(),
            api_key: Some("test-key".to_string()),
            model: "deepseek-chat".to_string(),
            temperature: 0.0,
            max_output_tokens: 4096,
            reasoning_effort: None,
            reasoning_budget_tokens: None,
            capabilities: ProviderModelCapabilities {
                supports_image_input: false,
                ..Default::default()
            },
            thinking_param_pattern: ThinkingParamPattern::None,
        });

        let request = ProviderRequest {
            model: "deepseek-chat".to_string(),
            input: vec![ProviderMessage::user("请看图".to_string())],
            images: vec![TurnInputImage {
                data_url: "data:image/png;base64,abc".to_string(),
                mime_type: "image/png".to_string(),
                name: Some("test.png".to_string()),
            }],
            native_messages: Vec::new(),
            observation: Default::default(),
            temperature: 0.0,
            max_output_tokens: 4096,
        };

        let sanitized = manager.sanitize_request(&request);
        assert!(sanitized.images.is_empty(), "images must be stripped by sanitize_request");

        // 如果模型支持图片，则不剥离
        let vision_manager = ProviderManager::new(ResolvedProviderSelection {
            capabilities: ProviderModelCapabilities {
                supports_image_input: true,
                ..Default::default()
            },
            ..manager.config.clone()
        });
        let vision_sanitized = vision_manager.sanitize_request(&request);
        assert_eq!(vision_sanitized.images.len(), 1, "images must be preserved for vision model");
    }

    /// F7 真实验证：`build_streaming_http_client` 的 180s 语义必须是「读空闲」而不是
    /// 「总 deadline」。用一个小 idle 预算 + 真本地 SSE 服务复现两件事：
    /// ① 总时长 > idle（2.5s vs 1s）但间隙 < idle 的流必须完整读完（旧实现会被总 deadline 截断）；
    /// ② 单次间隙 > idle 的连接必须失败为 timeout（真正卡死仍然有兜底）。
    #[test]
    fn streaming_client_uses_idle_read_timeout_not_total_deadline() {
        fn spawn_dribbling_sse_server(frames: usize, gap: Duration) -> (String, thread::JoinHandle<Vec<u8>>) {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind dribbling sse server");
            let address = listener.local_addr().expect("read address");
            let handle = thread::spawn(move || {
                let (mut stream, _) = listener.accept().expect("accept sse request");
                let mut buf = [0_u8; 4096];
                let mut consumed = Vec::new();
                loop {
                    let read = stream.read(&mut buf).expect("read request");
                    if read == 0 {
                        break;
                    }
                    consumed.extend_from_slice(&buf[..read]);
                    if consumed.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n",
                    )
                    .expect("write headers");
                stream.flush().expect("flush headers");
                for index in 0..frames {
                    // 先睡后写：模拟「首包前后都有静默」的长推理流。
                    thread::sleep(gap);
                    let frame = format!(
                        "data: {{\"choices\":[{{\"delta\":{{\"content\":\"c{index}\"}}}}]}}\n\n"
                    );
                    stream.write_all(frame.as_bytes()).expect("write frame");
                    stream.flush().expect("flush frame");
                }
                consumed
            });
            (format!("http://{}", address), handle)
        }

        let _guard = crate::agent::runtime_helper::TestRuntimeGuard::new();

        // ① 总耗时 10 × 250ms = 2.5s ≫ idle 1s，但每次间隙只有 250ms → 必须成功。
        let (success_endpoint, success_handle) =
            spawn_dribbling_sse_server(10, Duration::from_millis(250));
        let client = build_streaming_http_client(
            Duration::from_secs(5),
            Duration::from_secs(1),
            &success_endpoint,
        )
        .expect("build streaming client");
        let started = Instant::now();
        let response = block_on(client.get(&success_endpoint).send()).expect("stream must connect");
        assert!(response.status().is_success());
        let body = block_on(async move { response.text().await }).expect("long stream must not be cut by a total deadline");
        let elapsed = started.elapsed();
        assert!(
            elapsed > Duration::from_secs(1),
            "用例必须真的超过 idle 预算才有证明力，实际 {elapsed:?}"
        );
        assert!(
            body.contains("c9"),
            "10 帧必须全部读完（旧的总 deadline 会截断），实际正文={body:?}"
        );
        let _ = success_handle.join();

        // ② 单次静默 1.5s > idle 300ms → 必须 timeout 失败（卡死兜底仍在）。
        let (stalled_endpoint, stalled_handle) =
            spawn_dribbling_sse_server(1, Duration::from_millis(1500));
        let client = build_streaming_http_client(
            Duration::from_secs(5),
            Duration::from_millis(300),
            &stalled_endpoint,
        )
        .expect("build stalled client");
        let error = match block_on(client.get(&stalled_endpoint).send()) {
            Ok(response) => block_on(async move { response.text().await }).expect_err("stalled stream must time out"),
            Err(error) => error,
        };
        assert!(
            error.is_timeout(),
            "180s 语义必须是读空闲超时，卡死的流要报 timeout，实际错误={error:?}"
        );
        let _ = stalled_handle.join();
    }

    // ── PA-171 L3-B：冷却期全 scope 短路（ADVISORY） ────────────────────────────────

    /// L3-B ADVISORY：冷却标记错误在 **Followup** 作用域同样一失败即 Abort（不 sleep、不再发请求）。
    #[test]
    fn followup_cooldown_active_error_aborts_without_sleep() {
        let sleeper = crate::agent::retry::FakeSleeper::new();
        let attempts = std::cell::Cell::new(0);
        let cooldown_error = crate::agent::retry::append_cooldown_active_marker(
            "provider test 处于限流冷却中（quota），预计 60s 后恢复；本次未发起请求；retry_after=60",
        );
        let (result, report): (Result<(), String>, SingleHopRetryReport) =
            retry_provider_scoped_with_report(
                "followup_sync",
                ProviderRetryScope::Followup,
                || {
                    attempts.set(attempts.get() + 1);
                    Err(cooldown_error.clone())
                },
                &sleeper,
                None,
            );

        println!(
            "[L3-B Followup abort] attempts={} slept_ms={}",
            attempts.get(),
            sleeper.total_slept.load(std::sync::atomic::Ordering::Relaxed)
        );
        assert!(result.is_err());
        assert_eq!(attempts.get(), 1, "冷却期 Followup 必须只尝试一次（不得 sleep 后重放）");
        assert_eq!(
            sleeper.total_slept.load(std::sync::atomic::Ordering::Relaxed),
            0,
            "冷却期不得发生任何退避 sleep"
        );
        assert_eq!(report.attempts_total, 1);
        assert!(!report.switched_to_rl);
    }

    /// L3-B ADVISORY：冷却窗口内第 2 次调用**0 次 HTTP**（用回环 mock 物理计数）。
    #[test]
    fn cooldown_window_short_circuits_without_http() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind cooldown mock");
        let address = listener.local_addr().expect("read cooldown mock address");
        let requests = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let requests_for_thread = Arc::clone(&requests);
        let shutdown = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let shutdown_for_thread = Arc::clone(&shutdown);
        let handle = thread::spawn(move || {
            listener
                .set_nonblocking(true)
                .expect("set cooldown mock nonblocking");
            let mut buffer = [0_u8; 4096];
            while !shutdown_for_thread.load(std::sync::atomic::Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
                        let _ = stream.read(&mut buffer);
                        requests_for_thread.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        let body = "{\"error\":{\"type\":\"exceeded_current_quota_error\"}}";
                        let response = format!(
                            "HTTP/1.1 429 Too Many Requests\r\nContent-Type: application/json\r\nContent-Length: {}\r\nRetry-After: 60\r\nConnection: close\r\n\r\n{}",
                            body.len(),
                            body
                        );
                        let _ = stream.write_all(response.as_bytes());
                        let _ = stream.flush();
                    }
                    Err(_) => thread::sleep(Duration::from_millis(10)),
                }
            }
        });

        let config = ResolvedProviderSelection {
            requested_name: "pa171-cooldown-test".to_string(),
            provider_name: "pa171-cooldown-test".to_string(),
            protocol: ProviderProtocol::OpenAiCompletions,
            base_url: format!("http://{address}/v1"),
            auth_type: ProviderAuthType::Auto,
            api_key_env_var: "PA171_COOLDOWN_TEST_API_KEY".to_string(),
            api_key: Some("pa171-dummy".to_string()),
            model: "pa171-cooldown-model".to_string(),
            temperature: 0.0,
            max_output_tokens: 64,
            reasoning_effort: None,
            reasoning_budget_tokens: None,
            capabilities: ProviderModelCapabilities::default(),
            thinking_param_pattern: ThinkingParamPattern::None,
        };
        let manager = ProviderManager::new(config);
        let endpoint = format!("http://{address}/v1/chat/completions");
        let body = serde_json::json!({});

        let first = manager.post_openai_json(&endpoint, &body);
        assert!(first.is_err(), "首次 429 必须返回 Err");
        let after_first = requests.load(std::sync::atomic::Ordering::SeqCst);
        assert_eq!(after_first, 1, "首次 429 只应发 1 次 HTTP");

        let second = manager.post_openai_json(&endpoint, &body);
        let after_second = requests.load(std::sync::atomic::Ordering::SeqCst);
        let error = second.expect_err("冷却窗口内必须直接返回可读失败");
        assert_eq!(
            after_second, after_first,
            "冷却窗口内不得再发起 HTTP，实际新增 {} 次",
            after_second - after_first
        );
        println!(
            "[L3-B Followup] http first={} second={} err={}",
            after_first, after_second, error
        );
        assert!(error.contains("冷却"), "冷却失败文案必须可读：{error}");
        assert!(manager.cooldown_remaining().is_some());
        manager.clear_cooldown();

        shutdown.store(true, std::sync::atomic::Ordering::SeqCst);
        let _ = handle.join();
    }
}

