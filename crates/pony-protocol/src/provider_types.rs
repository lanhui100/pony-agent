use crate::input::TurnInputImage;
use crate::tool_contract::ToolCall;
use serde::{Deserialize, Serialize};
use serde_json::Value;

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
    /// 记录「本轮工具面含跨 turn 可变来源」这一前缀突变原因（幂等）。
    ///
    /// 只做记账，不改变任何请求内容；由 `runtime::turn_prep` 在工具面解析后调用。
    pub fn note_dynamic_tool_surface(&mut self, dynamic: bool) {
        if dynamic
            && !self
                .prefix_mutation_reasons
                .contains(&PrefixMutationReason::ToolSurfaceDynamic)
        {
            self.prefix_mutation_reasons
                .push(PrefixMutationReason::ToolSurfaceDynamic);
        }
    }

    pub fn is_empty(&self) -> bool {
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
    /// 本轮工具面含跨 turn 可变来源（MCP/插件通道），tools 数组可能已变化。
    ///
    /// 内建面是编译期常量；外部通道在会话中途连接/重载/断开会改变 tools 数组，
    /// 而 tools 属于 provider 前缀缓存的一部分 ⇒ 该轮前缀在工具维度不可复现。
    ToolSurfaceDynamic,
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
#[allow(dead_code)]
pub struct AnthropicStreamMessage {
    pub output_text: String,
    pub tool_call: Option<ToolCall>,
    pub reasoning_content: Option<String>,
    pub reasoning_content_value: Option<Value>,
    pub token_usage: Option<TokenUsage>,
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
