use crate::agent::provider::{ProviderAuthType, ProviderProtocol};
use crate::agent::secret_store::{default_secret_store, SecretStore};
use dirs::config_dir;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const DEFAULT_MAX_OUTPUT_TOKENS: u32 = 8192;
const LEGACY_DEFAULT_MAX_OUTPUT_TOKENS: u32 = 1200;
const DEFAULT_CONTEXT_WINDOW_TOKENS: u32 = 1_000_000;
const DEFAULT_MODERN_MAX_OUTPUT_TOKENS: u32 = 64000;

// ── PA-172 §2：上游调用策略（超时/重试/退避）配置面 ──────────────────────────────
//
// 默认值 = PA-171 交付后（基线 `458b952e`）的硬编码真值，逐字节等价（矩阵 G2）：
// 45s 同步总超时（`provider/mod.rs` 两处内联字面量）、5 次决策尝试、30s 退避预算。

/// 同步请求单次 attempt 的总超时（毫秒）。默认 45s（基线内联 `Duration::from_secs(45)`）。
pub const UPSTREAM_REQUEST_TIMEOUT_MS_DEFAULT: u64 = 45_000;
/// 决策/跟进单跳的最大尝试次数（含首次）。默认 5（基线 `PROVIDER_TIMEOUT_RETRY_MAX_ATTEMPTS`）。
pub const UPSTREAM_DECISION_MAX_ATTEMPTS_DEFAULT: u32 = 5;
/// 单跳短退避的 sleep 预算（毫秒）。默认 30s（基线 `BackoffConfig::total_budget_ms`）。
pub const UPSTREAM_RETRY_BUDGET_MS_DEFAULT: u64 = 30_000;
/// `requestTimeoutMs` 下界：低于 1s 对真实上游无意义。
pub const UPSTREAM_REQUEST_TIMEOUT_MS_MIN: u64 = 1_000;
/// `requestTimeoutMs` 上界。
pub const UPSTREAM_REQUEST_TIMEOUT_MS_MAX: u64 = 300_000;
/// `decisionMaxAttempts` 下界：至少尝试一次。
pub const UPSTREAM_DECISION_MAX_ATTEMPTS_MIN: u32 = 1;
/// `decisionMaxAttempts` 上界。
pub const UPSTREAM_DECISION_MAX_ATTEMPTS_MAX: u32 = 10;
/// `retryBudgetMs` 下界。v2-4：由 0 上调到 1000 —— 0 预算与 `RetryBudget::is_exhausted`
/// 的 `>=` 判据语义冲突（0 会让"从未尝试"直接判耗尽），边界收紧以消歧义。
pub const UPSTREAM_RETRY_BUDGET_MS_MIN: u64 = 1_000;
/// `retryBudgetMs` 上界。
pub const UPSTREAM_RETRY_BUDGET_MS_MAX: u64 = 300_000;
/// PA-172 §2.3 C2-c：显式候选链的长度上限（超出截断并 `warn failover_candidate_ignored`）。
pub const FAILOVER_CHAIN_MAX_LEN: usize = 8;

fn default_request_timeout_ms() -> u64 {
    UPSTREAM_REQUEST_TIMEOUT_MS_DEFAULT
}

fn default_decision_max_attempts() -> u32 {
    UPSTREAM_DECISION_MAX_ATTEMPTS_DEFAULT
}

fn default_retry_budget_ms() -> u64 {
    UPSTREAM_RETRY_BUDGET_MS_DEFAULT
}

/// 逐字段**容错**反序列化（矩阵 §2.1 审查 B/BLOCKING-1）。
///
/// `#[serde(default)]` 只补缺字段、接不住**类型错**；而 `providers.json` 是整体 `from_str`
/// ⇒ 一句 `"retryBudgetMs":"30s"` 会让整文件解析失败并回落 `default_registry()`，把用户
/// 全部 provider 配置顶替。故每个新增字段各自容错：`number` / 数字字符串 / `null` /
/// 其它 ⇒ 回落默认值并落 warn，**绝不**让新增字段的畸形拖垮既有配置。
/// `u64` 容错的公共实现：解析失败/类型不符 ⇒ `fallback`（**逐字段各自的默认值**，
/// 不得串用其它字段的默认值 —— 否则 `retryBudgetMs:"30s"` 会错落成 45000）。
fn parse_u64_lenient(value: &serde_json::Value, field: &str, fallback: u64) -> u64 {
    let parsed = match value {
        serde_json::Value::Number(number) => number
            .as_u64()
            .or_else(|| number.as_f64().filter(|float| *float >= 0.0).map(|float| float as u64)),
        serde_json::Value::String(text) => text.trim().parse::<u64>().ok(),
        _ => None,
    };
    parsed.unwrap_or_else(|| {
        warn_upstream_policy_invalid_value(field, value);
        fallback
    })
}

fn de_request_timeout_ms_lenient<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value: serde_json::Value = serde::Deserialize::deserialize(deserializer)?;
    Ok(parse_u64_lenient(
        &value,
        "requestTimeoutMs",
        default_request_timeout_ms(),
    ))
}

fn de_retry_budget_ms_lenient<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value: serde_json::Value = serde::Deserialize::deserialize(deserializer)?;
    Ok(parse_u64_lenient(
        &value,
        "retryBudgetMs",
        default_retry_budget_ms(),
    ))
}

fn de_u32_lenient<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value: serde_json::Value = serde::Deserialize::deserialize(deserializer)?;
    Ok(match &value {
        serde_json::Value::Number(number) => number
            .as_u64()
            .map(|value| value.min(u64::from(u32::MAX)) as u32)
            .unwrap_or_else(|| {
                warn_upstream_policy_invalid_value("u32", &value);
                default_decision_max_attempts()
            }),
        serde_json::Value::String(text) => text.trim().parse::<u32>().unwrap_or_else(|_| {
            warn_upstream_policy_invalid_value("u32", &value);
            default_decision_max_attempts()
        }),
        _ => {
            warn_upstream_policy_invalid_value("u32", &value);
            default_decision_max_attempts()
        }
    })
}

/// `failoverProviderIds` 容错：非数组 ⇒ 空链；含非字符串项 ⇒ **逐项丢弃**（其余保序生效）。
fn de_vec_string_lenient<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value: serde_json::Value = serde::Deserialize::deserialize(deserializer)?;
    let Some(items) = value.as_array() else {
        warn_failover_candidate_ignored("<non-array>", "unknown");
        return Ok(Vec::new());
    };
    let mut kept = Vec::with_capacity(items.len());
    for item in items {
        match item.as_str() {
            Some(text) if !text.trim().is_empty() => kept.push(text.trim().to_string()),
            _ => {
                let rendered: String = item.to_string().chars().take(40).collect();
                warn_failover_candidate_ignored(&rendered, "unknown");
            }
        }
    }
    Ok(kept)
}

fn warn_upstream_policy_invalid_value(expected: &str, provided: &serde_json::Value) {
    crate::agent::otel::log::log_json_with_fields(
        "warn",
        "upstream_policy_invalid_value",
        None,
        &[
            ("field", serde_json::json!(expected)),
            ("provided", serde_json::json!(provided)),
        ],
    );
}

/// 策略段整体容错：非对象 / 段内类型全错 ⇒ 回落默认值（C2-a 非致命）。
fn de_upstream_policy_lenient<'de, D>(deserializer: D) -> Result<UpstreamPolicy, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value: serde_json::Value = serde::Deserialize::deserialize(deserializer)?;
    Ok(serde_json::from_value::<UpstreamPolicy>(value.clone()).unwrap_or_else(|_| {
        warn_upstream_policy_invalid_value("upstreamPolicy", &value);
        UpstreamPolicy::default()
    }))
}

/// 存储层策略段整体容错（同 [`de_upstream_policy_lenient`]）。
fn de_upstream_policy_storage_lenient<'de, D>(
    deserializer: D,
) -> Result<UpstreamPolicyStorage, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value: serde_json::Value = serde::Deserialize::deserialize(deserializer)?;
    Ok(
        serde_json::from_value::<UpstreamPolicyStorage>(value.clone()).unwrap_or_else(|_| {
            warn_upstream_policy_invalid_value("upstream_policy", &value);
            UpstreamPolicyStorage::default()
        }),
    )
}

fn warn_failover_candidate_ignored(provider_id: &str, reason: &str) {
    crate::agent::otel::log::log_json_with_fields(
        "warn",
        "failover_candidate_ignored",
        None,
        &[
            ("provider_id", serde_json::json!(provider_id)),
            ("reason", serde_json::json!(reason)),
        ],
    );
}

struct CapabilityCatalogEntry {
    protocol: Option<&'static str>,
    patterns: &'static [&'static str],
    preset: ProviderCapabilityPreset,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProviderReasoningEffort {
    Low,
    Medium,
    High,
    Max,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderCapabilityPreset {
    Auto,
    OpenAiChat,
    OpenAiReasoning,
    AnthropicThinking,
    DeepseekChat,
    DeepseekReasoner,
    Custom,
}

impl Default for ProviderCapabilityPreset {
    fn default() -> Self {
        Self::Auto
    }
}

const CAPABILITY_CATALOG: &[CapabilityCatalogEntry] = &[
    CapabilityCatalogEntry {
        protocol: Some("anthropic"),
        patterns: &["claude-3-7", "claude-sonnet-4", "claude-opus-4"],
        preset: ProviderCapabilityPreset::AnthropicThinking,
    },
    CapabilityCatalogEntry {
        protocol: Some("openai"),
        patterns: &["gpt-5", "gpt-5.4", "gpt-5.5", "o1", "o3", "reason"],
        preset: ProviderCapabilityPreset::OpenAiReasoning,
    },
    CapabilityCatalogEntry {
        protocol: Some("openai"),
        patterns: &["gpt-4.1", "vision"],
        preset: ProviderCapabilityPreset::OpenAiChat,
    },
    CapabilityCatalogEntry {
        protocol: Some("openai"),
        patterns: &["deepseek-reasoner", "deepseek-r1", "deepseek-v4-pro"],
        preset: ProviderCapabilityPreset::DeepseekReasoner,
    },
    CapabilityCatalogEntry {
        protocol: Some("openai"),
        patterns: &["deepseek-chat", "deepseek-v4-flash"],
        preset: ProviderCapabilityPreset::DeepseekChat,
    },
];

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderModelCapabilities {
    pub context_window_tokens: Option<u32>,
    #[serde(default = "default_true")]
    pub supports_tools: bool,
    #[serde(default = "default_true")]
    pub supports_streaming: bool,
    #[serde(default)]
    pub supports_image_input: bool,
    #[serde(default)]
    pub supports_reasoning: bool,
    #[serde(default)]
    pub supports_video_input: bool,
    #[serde(default)]
    pub supports_audio_input: bool,
    #[serde(default)]
    pub supports_image_output: bool,
    #[serde(default)]
    pub supports_video_output: bool,
    #[serde(default)]
    pub supports_audio_output: bool,
}

impl Default for ProviderModelCapabilities {
    fn default() -> Self {
        Self {
            context_window_tokens: Some(DEFAULT_CONTEXT_WINDOW_TOKENS),
            supports_tools: true,
            supports_streaming: true,
            supports_image_input: true,
            supports_reasoning: true,
            supports_video_input: false,
            supports_audio_input: false,
            supports_image_output: false,
            supports_video_output: false,
            supports_audio_output: false,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderProtocolEndpoint {
    pub protocol: ProviderProtocol,
    pub enabled: bool,
    pub base_url: String,
    #[serde(default)]
    pub auth_type: ProviderAuthType,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderModelConfig {
    pub id: String,
    pub name: String,
    pub model: String,
    pub temperature: f32,
    pub max_output_tokens: u32,
    #[serde(default)]
    pub capability_preset: ProviderCapabilityPreset,
    #[serde(default)]
    pub reasoning_effort: Option<ProviderReasoningEffort>,
    #[serde(default)]
    pub reasoning_budget_tokens: Option<u32>,
    #[serde(default)]
    pub protocol: Option<ProviderProtocol>,
    /// 模型级 base_url 覆盖（D4）：非空 trim 值优先于 provider endpoint / 默认值。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default)]
    pub capabilities: ProviderModelCapabilities,
}

#[derive(Clone)]
struct ProviderModelCapabilityDeclaration {
    capability_preset: ProviderCapabilityPreset,
    capabilities: ProviderModelCapabilities,
}

#[derive(Clone)]
struct ProviderModelUserPolicy {
    temperature: f32,
    max_output_tokens: u32,
    reasoning_effort: Option<ProviderReasoningEffort>,
    reasoning_budget_tokens: Option<u32>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConfigView {
    pub id: String,
    pub name: String,
    pub protocol: ProviderProtocol,
    pub base_url: String,
    #[serde(default)]
    pub auth_type: ProviderAuthType,
    pub api_key_env_var: String,
    pub api_key_value: String,
    pub api_key_present: bool,
    #[serde(default)]
    pub supported_protocols: Vec<ProviderProtocol>,
    #[serde(default)]
    pub endpoints: Vec<ProviderProtocolEndpoint>,
    pub models: Vec<ProviderModelConfig>,
    pub selected_model_id: Option<String>,
}

/// PA-172 §2.1：上游调用策略（超时 / 决策尝试次数 / 退避预算）。
///
/// 视图层命名（`providers.json` 的 `upstream_policy` 段在存储层用
/// [`UpstreamPolicyStorage`] 的 snake_case，视图层/Tauri 命令用本类型的 camelCase）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UpstreamPolicy {
    #[serde(
        default = "default_request_timeout_ms",
        deserialize_with = "de_request_timeout_ms_lenient"
    )]
    pub request_timeout_ms: u64,
    #[serde(
        default = "default_decision_max_attempts",
        deserialize_with = "de_u32_lenient"
    )]
    pub decision_max_attempts: u32,
    #[serde(
        default = "default_retry_budget_ms",
        deserialize_with = "de_retry_budget_ms_lenient"
    )]
    pub retry_budget_ms: u64,
}

impl Default for UpstreamPolicy {
    fn default() -> Self {
        Self {
            request_timeout_ms: default_request_timeout_ms(),
            decision_max_attempts: default_decision_max_attempts(),
            retry_budget_ms: default_retry_budget_ms(),
        }
    }
}

impl UpstreamPolicy {
    /// 越界钳到边界（fail-safe，矩阵 §2.3）；返回 `(钳制后, 被钳制字段名列表)`。
    ///
    /// 空列表 ⇒ 未发生钳制。字段名用于 `upstream_policy_clamped` 结构化日志与测试断言。
    pub fn clamped(self) -> (Self, Vec<&'static str>) {
        let mut fields = Vec::new();
        let request_timeout_ms =
            if self.request_timeout_ms < UPSTREAM_REQUEST_TIMEOUT_MS_MIN {
                fields.push("request_timeout_ms");
                UPSTREAM_REQUEST_TIMEOUT_MS_MIN
            } else if self.request_timeout_ms > UPSTREAM_REQUEST_TIMEOUT_MS_MAX {
                fields.push("request_timeout_ms");
                UPSTREAM_REQUEST_TIMEOUT_MS_MAX
            } else {
                self.request_timeout_ms
            };
        let decision_max_attempts =
            if self.decision_max_attempts < UPSTREAM_DECISION_MAX_ATTEMPTS_MIN {
                fields.push("decision_max_attempts");
                UPSTREAM_DECISION_MAX_ATTEMPTS_MIN
            } else if self.decision_max_attempts > UPSTREAM_DECISION_MAX_ATTEMPTS_MAX {
                fields.push("decision_max_attempts");
                UPSTREAM_DECISION_MAX_ATTEMPTS_MAX
            } else {
                self.decision_max_attempts
            };
        let retry_budget_ms = if self.retry_budget_ms < UPSTREAM_RETRY_BUDGET_MS_MIN {
            fields.push("retry_budget_ms");
            UPSTREAM_RETRY_BUDGET_MS_MIN
        } else if self.retry_budget_ms > UPSTREAM_RETRY_BUDGET_MS_MAX {
            fields.push("retry_budget_ms");
            UPSTREAM_RETRY_BUDGET_MS_MAX
        } else {
            self.retry_budget_ms
        };
        (
            Self {
                request_timeout_ms,
                decision_max_attempts,
                retry_budget_ms,
            },
            fields,
        )
    }

    /// 是否与基线默认值逐项相等（G2「未配置时行为零变更」的判据）。
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// 钳制并落 `upstream_policy_clamped`（字段级一次事件；C2-b 可见性要求，禁止静默）。
fn clamp_upstream_policy_logging(policy: UpstreamPolicy) -> UpstreamPolicy {
    let (clamped, fields) = policy.clone().clamped();
    if fields.is_empty() {
        return clamped;
    }
    let provided = [
        ("request_timeout_ms", policy.request_timeout_ms as i128),
        ("decision_max_attempts", policy.decision_max_attempts as i128),
        ("retry_budget_ms", policy.retry_budget_ms as i128),
    ];
    let clamped_to = [
        ("request_timeout_ms", clamped.request_timeout_ms as i128),
        ("decision_max_attempts", clamped.decision_max_attempts as i128),
        ("retry_budget_ms", clamped.retry_budget_ms as i128),
    ];
    let bounds = [
        (
            "request_timeout_ms",
            format!(
                "[{UPSTREAM_REQUEST_TIMEOUT_MS_MIN}, {UPSTREAM_REQUEST_TIMEOUT_MS_MAX}]"
            ),
        ),
        (
            "decision_max_attempts",
            format!(
                "[{UPSTREAM_DECISION_MAX_ATTEMPTS_MIN}, {UPSTREAM_DECISION_MAX_ATTEMPTS_MAX}]"
            ),
        ),
        (
            "retry_budget_ms",
            format!("[{UPSTREAM_RETRY_BUDGET_MS_MIN}, {UPSTREAM_RETRY_BUDGET_MS_MAX}]"),
        ),
    ];
    for field in fields {
        let provided_value = provided
            .iter()
            .find(|(name, _)| *name == field)
            .map(|(_, value)| *value)
            .unwrap_or_default();
        let clamped_value = clamped_to
            .iter()
            .find(|(name, _)| *name == field)
            .map(|(_, value)| *value)
            .unwrap_or_default();
        let bound = bounds
            .iter()
            .find(|(name, _)| *name == field)
            .map(|(_, value)| value.clone())
            .unwrap_or_default();
        crate::agent::otel::log::log_json_with_fields(
            "warn",
            "upstream_policy_clamped",
            None,
            &[
                ("field", serde_json::json!(field)),
                ("provided", serde_json::json!(provided_value)),
                ("clamped_to", serde_json::json!(clamped_value)),
                ("bound", serde_json::json!(bound)),
            ],
        );
    }
    clamped
}

/// PA-172 §2.2：`providers.json` **存储层**的策略段（snake_case；视图层见 [`UpstreamPolicy`]）。
///
/// 分层命名（D69-17）：存储文件既有键全是 snake_case（`base_url` / `selected_provider_id`），
/// 新增键禁止把视图层 camelCase 直接写进存储文件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
struct UpstreamPolicyStorage {
    #[serde(
        default = "default_request_timeout_ms",
        deserialize_with = "de_request_timeout_ms_lenient"
    )]
    request_timeout_ms: u64,
    #[serde(
        default = "default_decision_max_attempts",
        deserialize_with = "de_u32_lenient"
    )]
    decision_max_attempts: u32,
    #[serde(
        default = "default_retry_budget_ms",
        deserialize_with = "de_retry_budget_ms_lenient"
    )]
    retry_budget_ms: u64,
}

impl From<UpstreamPolicy> for UpstreamPolicyStorage {
    fn from(policy: UpstreamPolicy) -> Self {
        Self {
            request_timeout_ms: policy.request_timeout_ms,
            decision_max_attempts: policy.decision_max_attempts,
            retry_budget_ms: policy.retry_budget_ms,
        }
    }
}

impl From<UpstreamPolicyStorage> for UpstreamPolicy {
    fn from(storage: UpstreamPolicyStorage) -> Self {
        Self {
            request_timeout_ms: storage.request_timeout_ms,
            decision_max_attempts: storage.decision_max_attempts,
            retry_budget_ms: storage.retry_budget_ms,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderRegistryView {
    #[serde(default, deserialize_with = "de_upstream_policy_lenient")]
    pub upstream_policy: UpstreamPolicy,
    #[serde(default, deserialize_with = "de_vec_string_lenient")]
    pub failover_provider_ids: Vec<String>,
    pub providers: Vec<ProviderConfigView>,
    pub selected_provider_id: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
struct ProviderConfigStorage {
    id: String,
    name: String,
    protocol: ProviderProtocol,
    base_url: String,
    #[serde(default)]
    auth_type: ProviderAuthType,
    api_key_env_var: String,
    #[serde(default)]
    secret_ref: String,
    #[serde(default)]
    api_key_value: String,
    #[serde(default)]
    supported_protocols: Vec<ProviderProtocol>,
    #[serde(default)]
    endpoints: Vec<ProviderProtocolEndpoint>,
    models: Vec<ProviderModelConfig>,
    selected_model_id: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
struct ProviderRegistryStorage {
    /// PA-172 §2.2：存储层 snake_case（视图层 camelCase）。
    #[serde(default, deserialize_with = "de_upstream_policy_storage_lenient")]
    upstream_policy: UpstreamPolicyStorage,
    #[serde(default, deserialize_with = "de_vec_string_lenient")]
    failover_provider_ids: Vec<String>,
    providers: Vec<ProviderConfigStorage>,
    selected_provider_id: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ThinkingParamPattern {
    /// No thinking-related parameters.
    None,
    /// Standard reasoning_effort: low/medium/high/max (OpenAI, e.g., GPT-5, o1, o3).
    EffortStandard,
    /// reasoning_effort: low/medium/high/none (DeepSeek V4, Volcano Engine).
    /// "none" explicitly disables thinking.
    EffortWithNone,
    /// thinking: { type: "enabled/disabled" } (legacy DeepSeek reasoner R1).
    ThinkingToggle,
    /// Anthropic thinking block with budget_tokens.
    AnthropicThinking,
}

#[derive(Clone)]
pub struct ResolvedProviderSelection {
    pub requested_name: String,
    pub provider_name: String,
    pub protocol: ProviderProtocol,
    pub base_url: String,
    pub auth_type: ProviderAuthType,
    pub api_key_env_var: String,
    pub api_key: Option<String>,
    pub model: String,
    pub temperature: f32,
    pub max_output_tokens: u32,
    pub reasoning_effort: Option<ProviderReasoningEffort>,
    pub reasoning_budget_tokens: Option<u32>,
    pub capabilities: ProviderModelCapabilities,
    pub thinking_param_pattern: ThinkingParamPattern,
}

/// W2（机械适配）：`ToolExecutionContext` 派生 `Debug` 后需要本类型的 `Debug`。
///
/// 手写而非 `derive`：`api_key` 必须脱敏，禁止被调试日志/错误上下文带出。
impl std::fmt::Debug for ResolvedProviderSelection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedProviderSelection")
            .field("requested_name", &self.requested_name)
            .field("provider_name", &self.provider_name)
            .field("protocol", &self.protocol)
            .field("base_url", &self.base_url)
            .field("auth_type", &self.auth_type)
            .field("api_key_env_var", &self.api_key_env_var)
            .field("api_key", &self.api_key.as_ref().map(|_| "[redacted]"))
            .field("model", &self.model)
            .finish_non_exhaustive()
    }
}

pub trait ProviderSelectionResolver: Send + Sync {
    fn resolve_provider_selection(
        &self,
        provider_id: Option<&str>,
        model_id: Option<&str>,
    ) -> ResolvedProviderSelection;

    /// PA-172 wave-69 additive（**必须带默认实现** ⇒ 13 个既有实现零改动）：
    /// 上游调用策略。默认 = 基线硬编码真值（G2：未配置时行为零变更）。
    fn upstream_policy(&self) -> UpstreamPolicy {
        UpstreamPolicy::default()
    }

    /// PA-172 wave-69 additive：有序显式候选链。默认空链 ⇒ 关闭换手。
    fn resolve_failover_chain(
        &self,
        primary: &ResolvedProviderSelection,
    ) -> Vec<ResolvedProviderSelection> {
        let _ = primary;
        Vec::new()
    }
}

pub struct ProviderRegistryStore {
    path: PathBuf,
    secret_store: Arc<dyn SecretStore>,
}

impl ProviderRegistryStore {
    pub fn new() -> Self {
        let mut path = config_dir().unwrap_or_else(|| PathBuf::from("."));
        path.push("pony-agent");
        path.push("providers.json");

        Self::with_path(path)
    }

    pub fn with_path(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let secret_store = default_secret_store(secret_file_path(&path));
        Self { path, secret_store }
    }

    #[allow(dead_code)]
    pub fn with_path_and_secret_store(
        path: impl Into<PathBuf>,
        secret_store: Arc<dyn SecretStore>,
    ) -> Self {
        Self {
            path: path.into(),
            secret_store,
        }
    }

    pub fn get_service_api_key(&self, service: &str) -> Option<String> {
        let secret_ref = format!("service-{service}");
        self.secret_store.get(&secret_ref).ok().flatten()
    }

    /// 按 id 精确解析已存 API Key，供 /models 目录拉取等显式入参缺省时的 key 解析链。
    /// 安全约束（code-review B/P1-2）：id 未命中一律返回 None——禁止静默回退到
    /// 选中/首个提供商的密钥，杜绝「A 的密钥发往为 B 填写的 URL」的凭证错配通路。
    pub fn provider_api_key(&self, provider_id: Option<&str>) -> Option<String> {
        let storage = self.load_storage();
        let provider = storage
            .providers
            .iter()
            .find(|item| Some(item.id.as_str()) == provider_id)?;
        resolve_provider_api_key(provider, self.secret_store.as_ref())
    }

    /// 解析指定提供商在指定协议上的显式 auth 覆盖（如网关要求强制 Bearer），
    /// 无匹配时回落 Auto（按家族推导）。code-review A-P3-7/B-P3-4：使 /models
    /// 目录拉取与 chat 通路一样尊重 endpoint 显式 auth_type。
    pub fn provider_endpoint_auth(
        &self,
        provider_id: Option<&str>,
        protocol: &ProviderProtocol,
    ) -> ProviderAuthType {
        self.load_storage()
            .providers
            .iter()
            .find(|item| Some(item.id.as_str()) == provider_id)
            .and_then(|provider| {
                provider
                    .endpoints
                    .iter()
                    .find(|endpoint| &endpoint.protocol == protocol)
            })
            .map(|endpoint| endpoint.auth_type.clone())
            .unwrap_or(ProviderAuthType::Auto)
    }

    pub fn set_service_api_key(&self, service: &str, key: &str) -> Result<(), String> {
        let secret_ref = format!("service-{service}");
        if key.is_empty() {
            self.secret_store.delete(&secret_ref)
        } else {
            self.secret_store.set(&secret_ref, key)
        }
    }

    pub fn load_view(&self) -> ProviderRegistryView {
        self.build_view_from_storage(self.load_storage())
    }

    pub fn save_view(&self, view: ProviderRegistryView) -> Result<ProviderRegistryView, String> {
        let previous_storage = self.load_storage();
        let input_storage = normalize_storage(storage_from_view(view));
        sync_registry_secrets(
            &previous_storage,
            &input_storage,
            self.secret_store.as_ref(),
        )?;
        let storage = sanitized_storage_for_persistence(&input_storage);
        self.write_storage(&storage)?;

        Ok(self.build_view_from_storage(storage))
    }
    pub fn save_view_without_env_sync(
        &self,
        view: ProviderRegistryView,
    ) -> Result<ProviderRegistryView, String> {
        let storage =
            sanitized_storage_for_persistence(&normalize_storage(storage_from_view(view)));

        self.write_storage(&storage)?;

        Ok(self.build_view_from_storage(storage))
    }

    pub fn resolve_selection(
        &self,
        provider_id: Option<&str>,
        model_id: Option<&str>,
    ) -> ResolvedProviderSelection {
        let storage = self.load_storage();
        let provider = storage
            .providers
            .iter()
            .find(|item| Some(item.id.as_str()) == provider_id)
            .or_else(|| {
                storage
                    .selected_provider_id
                    .as_deref()
                    .and_then(|selected| storage.providers.iter().find(|item| item.id == selected))
            })
            .or_else(|| storage.providers.first())
            .expect("default registry should always contain at least one provider");

        let model = provider
            .models
            .iter()
            .find(|item| Some(item.id.as_str()) == model_id)
            .or_else(|| {
                provider
                    .selected_model_id
                    .as_deref()
                    .and_then(|selected| provider.models.iter().find(|item| item.id == selected))
            })
            .or_else(|| provider.models.first())
            .expect("provider should always contain at least one model");

        // D4 解析序：
        // 1. effective protocol：model.protocol ∈ supported_protocols 时用之，否则主协议；
        let effective_protocol = model
            .protocol
            .as_ref()
            .filter(|candidate| {
                provider
                    .supported_protocols
                    .iter()
                    .any(|value| value == *candidate)
            })
            .cloned()
            .unwrap_or_else(|| provider.protocol.clone());

        // 2. base_url 链：model.base_url（非空 trim）> 该协议 enabled endpoint >
        //    provider.base_url（协议一致时）> 协议默认值；
        let model_base_url = model
            .base_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let endpoint = provider
            .endpoints
            .iter()
            .find(|entry| entry.enabled && entry.protocol == effective_protocol);
        let base_url = if let Some(url) = model_base_url {
            url.to_string()
        } else if let Some(endpoint) =
            endpoint.filter(|entry| !entry.base_url.trim().is_empty())
        {
            endpoint.base_url.clone()
        } else if provider.protocol == effective_protocol && !provider.base_url.trim().is_empty() {
            provider.base_url.clone()
        } else {
            default_base_url(&effective_protocol).to_string()
        };

        // 3. auth_type：定位到的 endpoint 显式非 Auto 则尊重，Auto 留给请求期按家族推导。
        let auth_type = match endpoint {
            Some(entry) if entry.auth_type != ProviderAuthType::Auto => entry.auth_type.clone(),
            _ => ProviderAuthType::Auto,
        };

        ResolvedProviderSelection {
            requested_name: provider.name.clone(),
            provider_name: provider.name.clone(),
            protocol: effective_protocol.clone(),
            base_url,
            auth_type,
            api_key_env_var: provider.api_key_env_var.clone(),
            api_key: resolve_provider_api_key(provider, self.secret_store.as_ref()),
            model: model.model.clone(),
            temperature: model.temperature,
            max_output_tokens: model.max_output_tokens,
            reasoning_effort: model.reasoning_effort.clone(),
            reasoning_budget_tokens: model.reasoning_budget_tokens,
            capabilities: model.capabilities.clone(),
            thinking_param_pattern: resolve_thinking_param_pattern(
                &model.capability_preset,
                &effective_protocol,
                &model.model,
            ),
        }
    }

    fn build_view_from_storage(&self, storage: ProviderRegistryStorage) -> ProviderRegistryView {
        build_view_from_storage(storage, self.secret_store.as_ref())
    }

    fn write_storage(&self, storage: &ProviderRegistryStorage) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("create provider config directory failed: {}", error))?;
        }

        let json = serde_json::to_string_pretty(storage)
            .map_err(|error| format!("serialize provider config failed: {}", error))?;

        fs::write(&self.path, json)
            .map_err(|error| format!("write provider config failed: {}", error))
    }

    fn load_storage(&self) -> ProviderRegistryStorage {
        if let Ok(content) = fs::read_to_string(&self.path) {
            match serde_json::from_str::<ProviderRegistryStorage>(&content) {
                Ok(storage) => return normalize_storage(storage),
                Err(_) => backup_invalid_registry_file(&self.path),
            }
        }

        normalize_storage(default_registry())
    }
}

impl ProviderSelectionResolver for ProviderRegistryStore {
    fn resolve_provider_selection(
        &self,
        provider_id: Option<&str>,
        model_id: Option<&str>,
    ) -> ResolvedProviderSelection {
        self.resolve_selection(provider_id, model_id)
    }

    /// PA-172 §2：读存储层策略段（`normalize_storage` 已钳制，故此处只需透传）。
    fn upstream_policy(&self) -> UpstreamPolicy {
        UpstreamPolicy::from(self.load_storage().upstream_policy)
    }

    /// PA-172 §2.3 C2-c：候选链卫生——未知 id / 重复（含与主 provider 同身份）/ 自身
    /// 一律忽略并 `warn failover_candidate_ignored`；链长上限 8（超出截断并 warn）。
    fn resolve_failover_chain(
        &self,
        primary: &ResolvedProviderSelection,
    ) -> Vec<ResolvedProviderSelection> {
        let storage = self.load_storage();
        let known_ids: Vec<String> = storage
            .providers
            .iter()
            .map(|provider| provider.id.clone())
            .collect();
        let mut chain: Vec<ResolvedProviderSelection> = Vec::new();
        let mut identities: Vec<(String, String)> = vec![(
            primary.provider_name.clone(),
            primary.base_url.clone(),
        )];
        for provider_id in storage.failover_provider_ids {
            if !known_ids.iter().any(|known| *known == provider_id) {
                warn_failover_candidate_ignored(&provider_id, "unknown");
                continue;
            }
            let selection = self.resolve_selection(Some(provider_id.as_str()), None);
            let identity = (selection.provider_name.clone(), selection.base_url.clone());
            if identity
                == (primary.provider_name.clone(), primary.base_url.clone())
            {
                warn_failover_candidate_ignored(&provider_id, "self");
                continue;
            }
            if identities.contains(&identity) {
                warn_failover_candidate_ignored(&provider_id, "duplicate");
                continue;
            }
            if chain.len() >= FAILOVER_CHAIN_MAX_LEN {
                warn_failover_candidate_ignored(&provider_id, "truncated");
                break;
            }
            identities.push(identity);
            chain.push(selection);
        }
        chain
    }
}

fn storage_from_view(view: ProviderRegistryView) -> ProviderRegistryStorage {
    ProviderRegistryStorage {
        // PA-172 §2.2：视图层 camelCase → 存储层 snake_case（全透传，round-trip 不丢）。
        upstream_policy: UpstreamPolicyStorage::from(view.upstream_policy),
        failover_provider_ids: view.failover_provider_ids,
        providers: view
            .providers
            .into_iter()
            .map(|provider| {
                let ProviderConfigView {
                    id,
                    name,
                    protocol,
                    base_url,
                    auth_type,
                    api_key_env_var: _,
                    api_key_value,
                    api_key_present: _,
                    supported_protocols,
                    endpoints,
                    models,
                    selected_model_id,
                } = provider;

                ProviderConfigStorage {
                    secret_ref: String::new(),
                    id,
                    api_key_env_var: derive_env_var_name(&name),
                    name,
                    protocol,
                    base_url,
                    auth_type,
                    api_key_value,
                    supported_protocols,
                    endpoints,
                    models,
                    selected_model_id,
                }
            })
            .collect(),
        selected_provider_id: view.selected_provider_id,
    }
}

fn secret_file_path(registry_path: &Path) -> PathBuf {
    registry_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("secrets.json")
}

/// R6 缓解：registry 反序列化失败时，先把原文件 best-effort 拷贝为同目录
/// `providers.json.invalid-<unix_ts>` 再回落默认注册表，避免后续保存以出厂
/// 模板覆写用户配置且无任何恢复线索。拷贝失败不 panic（只丢备份，不丢行为）。
/// 同一损坏文件的重复加载只保留首个备份（code-review A-P3-8：防跨秒累积），
/// 后续修复后用户可自行删除 invalid-* 文件。
fn backup_invalid_registry_file(registry_path: &Path) {
    let Some(file_name) = registry_path.file_name().and_then(|name| name.to_str()) else {
        return;
    };
    let Some(parent) = registry_path.parent() else {
        return;
    };
    if let Ok(entries) = fs::read_dir(parent) {
        let prefix = format!("{file_name}.invalid-");
        if entries
            .flatten()
            .any(|entry| entry.file_name().to_string_lossy().starts_with(&prefix))
        {
            return;
        }
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let backup_path = registry_path.with_file_name(format!("{file_name}.invalid-{stamp}"));
    if let Err(error) = fs::copy(registry_path, &backup_path) {
        eprintln!(
            "[pony-agent] backup invalid provider registry failed: {} (target: {})",
            error,
            backup_path.display()
        );
    }
}

fn default_secret_ref(provider_id: &str) -> String {
    format!("provider/{}/api-key", provider_id.trim())
}

fn build_view_from_storage(
    storage: ProviderRegistryStorage,
    secret_store: &dyn SecretStore,
) -> ProviderRegistryView {
    ProviderRegistryView {
        upstream_policy: UpstreamPolicy::from(storage.upstream_policy),
        failover_provider_ids: storage.failover_provider_ids,
        selected_provider_id: storage.selected_provider_id.clone(),
        providers: storage
            .providers
            .into_iter()
            .map(|provider| {
                let api_key_value = provider.api_key_value.clone();
                let api_key_present = resolve_provider_api_key(&provider, secret_store).is_some();

                ProviderConfigView {
                    id: provider.id,
                    name: provider.name,
                    protocol: provider.protocol,
                    base_url: provider.base_url,
                    auth_type: provider.auth_type.clone(),
                    api_key_env_var: provider.api_key_env_var,
                    api_key_present,
                    api_key_value,
                    supported_protocols: provider.supported_protocols,
                    endpoints: provider.endpoints,
                    models: provider.models,
                    selected_model_id: provider.selected_model_id,
                }
            })
            .collect(),
    }
}

fn resolve_provider_api_key(
    provider: &ProviderConfigStorage,
    secret_store: &dyn SecretStore,
) -> Option<String> {
    let stored = provider.api_key_value.trim();
    if !stored.is_empty() {
        return Some(stored.to_string());
    }

    if let Ok(Some(secret)) = secret_store.get(&provider.secret_ref) {
        let trimmed = secret.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    read_env_var_best_effort(&provider.api_key_env_var)
}

fn sync_registry_secrets(
    previous: &ProviderRegistryStorage,
    next: &ProviderRegistryStorage,
    secret_store: &dyn SecretStore,
) -> Result<(), String> {
    for provider in &next.providers {
        let value = provider.api_key_value.trim();
        if value.is_empty() {
            continue;
        }

        secret_store.set(&provider.secret_ref, value)?;
    }

    for provider in &previous.providers {
        if next
            .providers
            .iter()
            .any(|item| item.secret_ref == provider.secret_ref)
        {
            continue;
        }

        let _ = secret_store.delete(&provider.secret_ref);
    }

    Ok(())
}

fn sanitized_storage_for_persistence(storage: &ProviderRegistryStorage) -> ProviderRegistryStorage {
    let mut sanitized = storage.clone();
    for provider in &mut sanitized.providers {
        provider.api_key_value.clear();
    }
    sanitized
}

fn read_env_var_best_effort(name: &str) -> Option<String> {
    let process_value = std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let persistent_value = read_persistent_env_var(name);

    if persistent_value.is_some() {
        return persistent_value;
    }

    process_value
}

#[cfg(not(windows))]
fn read_persistent_env_var(_name: &str) -> Option<String> {
    None
}

#[cfg(windows)]
fn read_persistent_env_var(name: &str) -> Option<String> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;

    let user = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey("Environment")
        .ok()
        .and_then(|key| key.get_value::<String, _>(name).ok());
    let machine = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey("SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment")
        .ok()
        .and_then(|key| key.get_value::<String, _>(name).ok());

    user.or(machine)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn default_registry() -> ProviderRegistryStorage {
    ProviderRegistryStorage {
        upstream_policy: UpstreamPolicyStorage::default(),
        failover_provider_ids: Vec::new(),
        selected_provider_id: Some("provider-ppx".to_string()),
        providers: default_provider_templates(),
    }
}

fn default_protocol_endpoint(protocol: &ProviderProtocol) -> ProviderProtocolEndpoint {
    ProviderProtocolEndpoint {
        protocol: protocol.clone(),
        enabled: false,
        base_url: default_base_url(protocol).to_string(),
        auth_type: if protocol.is_anthropic() {
            ProviderAuthType::XApiKey
        } else {
            ProviderAuthType::Auto
        },
    }
}

fn normalize_provider_endpoints(
    protocol: &ProviderProtocol,
    base_url: &str,
    auth_type: &ProviderAuthType,
    supported_protocols: &[ProviderProtocol],
    endpoints: &[ProviderProtocolEndpoint],
) -> Vec<ProviderProtocolEndpoint> {
    // 三协议各占一个 endpoint 槽位（openai-completions / openai-responses /
    // anthropic-messages）；既有配置按 protocol 匹配回填，未命中的保持默认。
    let mut normalized = vec![
        default_protocol_endpoint(&ProviderProtocol::OpenAiCompletions),
        default_protocol_endpoint(&ProviderProtocol::OpenAiResponses),
        default_protocol_endpoint(&ProviderProtocol::AnthropicMessages),
    ];

    for entry in &mut normalized {
        if let Some(existing) = endpoints
            .iter()
            .find(|item| item.protocol == entry.protocol)
        {
            entry.enabled = existing.enabled;
            entry.base_url = if existing.base_url.trim().is_empty() {
                default_base_url(&entry.protocol).to_string()
            } else {
                existing.base_url.clone()
            };
            entry.auth_type = existing.auth_type.clone();
        }
    }

    for item in &mut normalized {
        if &item.protocol == protocol {
            item.enabled = true;
            item.base_url = if base_url.trim().is_empty() {
                default_base_url(protocol).to_string()
            } else {
                base_url.to_string()
            };
            item.auth_type = auth_type.clone();
        }
        if supported_protocols
            .iter()
            .any(|value| value == &item.protocol)
        {
            item.enabled = true;
        }
    }

    normalized
}

fn normalize_supported_protocols(
    protocol: &ProviderProtocol,
    supported_protocols: &[ProviderProtocol],
    endpoints: &[ProviderProtocolEndpoint],
) -> Vec<ProviderProtocol> {
    let mut list = Vec::new();

    for item in supported_protocols {
        if !list.iter().any(|existing| existing == item) {
            list.push(item.clone());
        }
    }

    for item in endpoints.iter().filter(|entry| entry.enabled) {
        if !list.iter().any(|existing| existing == &item.protocol) {
            list.push(item.protocol.clone());
        }
    }

    if list.is_empty() {
        list.push(protocol.clone());
    }

    list
}

fn resolve_model_protocol(
    provider_protocol: &ProviderProtocol,
    supported_protocols: &[ProviderProtocol],
    model_protocol: Option<&ProviderProtocol>,
) -> ProviderProtocol {
    if let Some(protocol) = model_protocol {
        if supported_protocols.iter().any(|value| value == protocol) {
            return protocol.clone();
        }
    }

    supported_protocols
        .first()
        .cloned()
        .unwrap_or_else(|| provider_protocol.clone())
}

fn normalize_storage(mut storage: ProviderRegistryStorage) -> ProviderRegistryStorage {
    // PA-172 §2.3：策略段越界钳制（C2-a 非致命 + C2-b 落 `upstream_policy_clamped`）。
    // 钳制在**唯一归一入口**做 ⇒ 落盘与读回都看到同一份合法值；候选链只做透传，卫生
    // （未知/重复/自身/超长）在 `resolve_failover_chain` 解析时判定。
    let policy = UpstreamPolicy::from(storage.upstream_policy);
    storage.upstream_policy = UpstreamPolicyStorage::from(clamp_upstream_policy_logging(policy));
    if storage.providers.is_empty() {
        return default_registry();
    }

    for provider in &mut storage.providers {
        if provider.id.trim().is_empty() {
            provider.id = slugify(&provider.name, "provider");
        }
        if provider.name.trim().is_empty() {
            provider.name = provider.id.clone();
        }
        if provider.base_url.trim().is_empty() {
            provider.base_url = default_base_url(&provider.protocol).to_string();
        }
        provider.endpoints = normalize_provider_endpoints(
            &provider.protocol,
            &provider.base_url,
            &provider.auth_type,
            &provider.supported_protocols,
            &provider.endpoints,
        );
        provider.supported_protocols = normalize_supported_protocols(
            &provider.protocol,
            &provider.supported_protocols,
            &provider.endpoints,
        );
        if let Some(primary_protocol) = provider.supported_protocols.first().cloned() {
            provider.protocol = primary_protocol.clone();
            if let Some(primary_endpoint) = provider
                .endpoints
                .iter()
                .find(|item| item.protocol == primary_protocol)
            {
                provider.base_url = primary_endpoint.base_url.clone();
                provider.auth_type = primary_endpoint.auth_type.clone();
            }
        }
        provider.api_key_env_var = derive_env_var_name(&provider.name);
        if provider.secret_ref.trim().is_empty() {
            provider.secret_ref = default_secret_ref(&provider.id);
        }
        provider.api_key_value = provider.api_key_value.trim().to_string();
        if provider.models.is_empty() {
            provider.models.push(ProviderModelConfig {
                id: format!("{}-model-default", provider.id),
                name: "默认模型".to_string(),
                model: default_model(&provider.protocol).to_string(),
                temperature: 0.2,
                max_output_tokens: DEFAULT_MAX_OUTPUT_TOKENS,
                capability_preset: infer_capability_preset(
                    &provider.protocol,
                    default_model(&provider.protocol),
                ),
                reasoning_effort: None,
                reasoning_budget_tokens: None,
                protocol: Some(provider.protocol.clone()),
                base_url: None,
                capabilities: default_model_capabilities(
                    &provider.protocol,
                    default_model(&provider.protocol),
                ),
            });
        }
        for model in &mut provider.models {
            if model.id.trim().is_empty() {
                model.id = slugify(&model.name, "model");
            }
            if model.name.trim().is_empty() {
                model.name = model.model.clone();
            }
            if model.model.trim().is_empty() {
                model.model = default_model(&provider.protocol).to_string();
            }
            // 模型级 base_url 覆盖清洗：trim 后为空则置 None（回落解析链）。
            model.base_url = model
                .base_url
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            let model_protocol = resolve_model_protocol(
                &provider.protocol,
                &provider.supported_protocols,
                model.protocol.as_ref(),
            );
            if model.max_output_tokens == 0
                || model.max_output_tokens == LEGACY_DEFAULT_MAX_OUTPUT_TOKENS
                || model.max_output_tokens == DEFAULT_MAX_OUTPUT_TOKENS
            {
                model.max_output_tokens = DEFAULT_MODERN_MAX_OUTPUT_TOKENS;
            }
            if model.reasoning_budget_tokens == Some(0) {
                model.reasoning_budget_tokens = None;
            }
            let declaration = resolve_model_capability_declaration(
                &model_protocol,
                &model.model,
                &model.capability_preset,
                model.capabilities.clone(),
            );
            let user_policy = normalize_model_user_policy(model, &declaration.capabilities);
            model.protocol = Some(model_protocol);
            model.capability_preset = declaration.capability_preset;
            model.capabilities = declaration.capabilities;
            model.temperature = user_policy.temperature;
            model.max_output_tokens = user_policy.max_output_tokens;
            model.reasoning_effort = user_policy.reasoning_effort;
            model.reasoning_budget_tokens = user_policy.reasoning_budget_tokens;
        }
        if provider.id == "provider-deepseek" {
            let mut has_deepseek_v4_pro = false;
            provider.models.retain(|model| {
                if !is_deepseek_v4_pro_model_value(&model.model) {
                    return true;
                }
                if has_deepseek_v4_pro {
                    return false;
                }
                has_deepseek_v4_pro = true;
                true
            });
            if !has_deepseek_v4_pro {
                provider.models.push(default_deepseek_v4_pro_model());
            }
        }
        dedupe_provider_models(provider);
        if provider.selected_model_id.is_none()
            || provider
                .models
                .iter()
                .all(|model| Some(model.id.as_str()) != provider.selected_model_id.as_deref())
        {
            provider.selected_model_id = provider.models.first().map(|model| model.id.clone());
        }
    }

    if storage.selected_provider_id.is_none()
        || storage
            .providers
            .iter()
            .all(|provider| Some(provider.id.as_str()) != storage.selected_provider_id.as_deref())
    {
        storage.selected_provider_id = storage
            .providers
            .first()
            .map(|provider| provider.id.clone());
    }

    storage
}

fn dedupe_provider_models(provider: &mut ProviderConfigStorage) {
    let selected_model_id = provider.selected_model_id.clone();
    let mut seen: Vec<(String, String)> = Vec::new();
    let mut selected_replacement_id: Option<String> = None;

    provider.models.retain(|model| {
        let key = model_uniqueness_key(&model.model);
        if let Some((_, kept_id)) = seen.iter().find(|(seen_key, _)| seen_key == &key) {
            if Some(model.id.as_str()) == selected_model_id.as_deref() {
                selected_replacement_id = Some(kept_id.clone());
            }
            return false;
        }

        seen.push((key, model.id.clone()));
        true
    });

    if selected_model_id
        .as_deref()
        .is_some_and(|selected| provider.models.iter().all(|model| model.id != selected))
    {
        provider.selected_model_id = selected_replacement_id
            .or_else(|| provider.models.first().map(|model| model.id.clone()));
    }
}

fn default_deepseek_v4_pro_model() -> ProviderModelConfig {
    ProviderModelConfig {
        id: "model-deepseek-v4-pro".to_string(),
        name: "DeepSeek V4 Pro".to_string(),
        model: "deepseek-v4-pro".to_string(),
        temperature: 0.2,
        max_output_tokens: DEFAULT_MODERN_MAX_OUTPUT_TOKENS,
        capability_preset: ProviderCapabilityPreset::DeepseekReasoner,
        reasoning_effort: Some(ProviderReasoningEffort::Medium),
        reasoning_budget_tokens: None,
        protocol: Some(ProviderProtocol::OpenAiCompletions),
        base_url: None,
        capabilities: default_model_capabilities(&ProviderProtocol::OpenAiCompletions, "deepseek-v4-pro"),
    }
}

fn is_deepseek_v4_pro_model_value(model_name: &str) -> bool {
    let lower = model_name.trim().to_ascii_lowercase();
    lower == "deepseek-v4-pro" || lower.ends_with("/deepseek-v4-pro")
}

fn model_uniqueness_key(model_name: &str) -> String {
    if is_deepseek_v4_pro_model_value(model_name) {
        "deepseek-v4-pro".to_string()
    } else {
        model_name.trim().to_ascii_lowercase()
    }
}

fn default_provider_templates() -> Vec<ProviderConfigStorage> {
    vec![
        ProviderConfigStorage {
            id: "provider-ppx".to_string(),
            name: "ppx".to_string(),
            protocol: ProviderProtocol::OpenAiCompletions,
            base_url: "https://api.psydo.top/v1".to_string(),
            auth_type: ProviderAuthType::Auto,
            api_key_env_var: "PPX_API_KEY".to_string(),
            secret_ref: default_secret_ref("provider-ppx"),
            api_key_value: String::new(),
            selected_model_id: Some("model-ppx-default".to_string()),
            models: vec![ProviderModelConfig {
                id: "model-ppx-default".to_string(),
                name: "GPT 5.4".to_string(),
                model: "gpt-5.4".to_string(),
                temperature: 0.2,
                max_output_tokens: DEFAULT_MAX_OUTPUT_TOKENS,
                capability_preset: ProviderCapabilityPreset::OpenAiReasoning,
                reasoning_effort: None,
                reasoning_budget_tokens: None,
                protocol: Some(ProviderProtocol::OpenAiCompletions),
                base_url: None,
                capabilities: default_model_capabilities(&ProviderProtocol::OpenAiCompletions, "gpt-5.4"),
            }],
            supported_protocols: vec![ProviderProtocol::OpenAiCompletions],
            endpoints: vec![
                ProviderProtocolEndpoint {
                    protocol: ProviderProtocol::OpenAiCompletions,
                    enabled: true,
                    base_url: "https://api.psydo.top/v1".to_string(),
                    auth_type: ProviderAuthType::Auto,
                },
                default_protocol_endpoint(&ProviderProtocol::AnthropicMessages),
            ],
        },
        ProviderConfigStorage {
            id: "provider-openai".to_string(),
            name: "openai".to_string(),
            protocol: ProviderProtocol::OpenAiCompletions,
            base_url: "https://api.openai.com/v1".to_string(),
            auth_type: ProviderAuthType::Auto,
            api_key_env_var: "OPENAI_API_KEY".to_string(),
            secret_ref: default_secret_ref("provider-openai"),
            api_key_value: String::new(),
            selected_model_id: Some("model-openai-default".to_string()),
            models: vec![ProviderModelConfig {
                id: "model-openai-default".to_string(),
                name: "GPT 4.1 Mini".to_string(),
                model: "gpt-4.1-mini".to_string(),
                temperature: 0.2,
                max_output_tokens: DEFAULT_MAX_OUTPUT_TOKENS,
                capability_preset: ProviderCapabilityPreset::OpenAiChat,
                reasoning_effort: None,
                reasoning_budget_tokens: None,
                protocol: Some(ProviderProtocol::OpenAiCompletions),
                base_url: None,
                capabilities: default_model_capabilities(&ProviderProtocol::OpenAiCompletions, "gpt-4.1-mini"),
            }],
            supported_protocols: vec![ProviderProtocol::OpenAiCompletions],
            endpoints: vec![
                ProviderProtocolEndpoint {
                    protocol: ProviderProtocol::OpenAiCompletions,
                    enabled: true,
                    base_url: "https://api.openai.com/v1".to_string(),
                    auth_type: ProviderAuthType::Auto,
                },
                default_protocol_endpoint(&ProviderProtocol::AnthropicMessages),
            ],
        },
        ProviderConfigStorage {
            id: "provider-openrouter".to_string(),
            name: "openrouter".to_string(),
            protocol: ProviderProtocol::OpenAiCompletions,
            base_url: "https://openrouter.ai/api/v1".to_string(),
            auth_type: ProviderAuthType::Auto,
            api_key_env_var: "OPENROUTER_API_KEY".to_string(),
            secret_ref: default_secret_ref("provider-openrouter"),
            api_key_value: String::new(),
            selected_model_id: Some("model-openrouter-default".to_string()),
            models: vec![ProviderModelConfig {
                id: "model-openrouter-default".to_string(),
                name: "OpenAI GPT-4.1 Mini".to_string(),
                model: "openai/gpt-4.1-mini".to_string(),
                temperature: 0.2,
                max_output_tokens: DEFAULT_MAX_OUTPUT_TOKENS,
                capability_preset: ProviderCapabilityPreset::OpenAiChat,
                reasoning_effort: None,
                reasoning_budget_tokens: None,
                protocol: Some(ProviderProtocol::OpenAiCompletions),
                base_url: None,
                capabilities: default_model_capabilities(
                    &ProviderProtocol::OpenAiCompletions,
                    "openai/gpt-4.1-mini",
                ),
            }],
            supported_protocols: vec![ProviderProtocol::OpenAiCompletions],
            endpoints: vec![
                ProviderProtocolEndpoint {
                    protocol: ProviderProtocol::OpenAiCompletions,
                    enabled: true,
                    base_url: "https://openrouter.ai/api/v1".to_string(),
                    auth_type: ProviderAuthType::Auto,
                },
                default_protocol_endpoint(&ProviderProtocol::AnthropicMessages),
            ],
        },
        ProviderConfigStorage {
            id: "provider-deepseek".to_string(),
            name: "deepseek".to_string(),
            protocol: ProviderProtocol::OpenAiCompletions,
            base_url: "https://api.deepseek.com/v1".to_string(),
            auth_type: ProviderAuthType::Auto,
            api_key_env_var: "DEEPSEEK_API_KEY".to_string(),
            secret_ref: default_secret_ref("provider-deepseek"),
            api_key_value: String::new(),
            selected_model_id: Some("model-deepseek-default".to_string()),
            models: vec![
                ProviderModelConfig {
                    id: "model-deepseek-default".to_string(),
                    name: "DeepSeek V4 Flash".to_string(),
                    model: "deepseek-v4-flash".to_string(),
                    temperature: 0.2,
                    max_output_tokens: DEFAULT_MAX_OUTPUT_TOKENS,
                    capability_preset: ProviderCapabilityPreset::DeepseekChat,
                    reasoning_effort: None,
                    reasoning_budget_tokens: None,
                    protocol: Some(ProviderProtocol::OpenAiCompletions),
                    base_url: None,
                    capabilities: default_model_capabilities(
                        &ProviderProtocol::OpenAiCompletions,
                        "deepseek-v4-flash",
                    ),
                },
                default_deepseek_v4_pro_model(),
            ],
            supported_protocols: vec![ProviderProtocol::OpenAiCompletions],
            endpoints: vec![
                ProviderProtocolEndpoint {
                    protocol: ProviderProtocol::OpenAiCompletions,
                    enabled: true,
                    base_url: "https://api.deepseek.com/v1".to_string(),
                    auth_type: ProviderAuthType::Auto,
                },
                default_protocol_endpoint(&ProviderProtocol::AnthropicMessages),
            ],
        },
        ProviderConfigStorage {
            id: "provider-anthropic".to_string(),
            name: "anthropic".to_string(),
            protocol: ProviderProtocol::AnthropicMessages,
            base_url: "https://api.anthropic.com/v1".to_string(),
            auth_type: ProviderAuthType::Auto,
            api_key_env_var: "ANTHROPIC_API_KEY".to_string(),
            secret_ref: default_secret_ref("provider-anthropic"),
            api_key_value: String::new(),
            selected_model_id: Some("model-anthropic-default".to_string()),
            models: vec![ProviderModelConfig {
                id: "model-anthropic-default".to_string(),
                name: "Claude Sonnet".to_string(),
                model: "claude-3-7-sonnet-latest".to_string(),
                temperature: 0.2,
                max_output_tokens: DEFAULT_MAX_OUTPUT_TOKENS,
                capability_preset: ProviderCapabilityPreset::AnthropicThinking,
                reasoning_effort: None,
                reasoning_budget_tokens: None,
                protocol: Some(ProviderProtocol::AnthropicMessages),
                base_url: None,
                capabilities: default_model_capabilities(
                    &ProviderProtocol::AnthropicMessages,
                    "claude-3-7-sonnet-latest",
                ),
            }],
            supported_protocols: vec![ProviderProtocol::AnthropicMessages],
            endpoints: vec![
                default_protocol_endpoint(&ProviderProtocol::OpenAiCompletions),
                ProviderProtocolEndpoint {
                    protocol: ProviderProtocol::AnthropicMessages,
                    enabled: true,
                    base_url: "https://api.anthropic.com/v1".to_string(),
                    auth_type: ProviderAuthType::Auto,
                },
            ],
        },
    ]
}

fn default_model_capabilities(
    protocol: &ProviderProtocol,
    model_name: &str,
) -> ProviderModelCapabilities {
    preset_model_capabilities(
        &infer_capability_preset(protocol, model_name),
        protocol,
        model_name,
    )
}

fn infer_capability_preset(
    protocol: &ProviderProtocol,
    model_name: &str,
) -> ProviderCapabilityPreset {
    let lower = model_name.to_ascii_lowercase();

    if let Some(entry) = CAPABILITY_CATALOG.iter().find(|entry| {
        catalog_matches_protocol(protocol, entry.protocol)
            && entry.patterns.iter().any(|pattern| lower.contains(pattern))
    }) {
        return entry.preset.clone();
    }

    if protocol.is_anthropic() {
        return ProviderCapabilityPreset::AnthropicThinking;
    }

    ProviderCapabilityPreset::Auto
}

fn is_deepseek_v4_by_model(model_name: &str) -> bool {
    let lower = model_name.to_ascii_lowercase();
    lower.contains("deepseek-v4") || lower.contains("v4-flash") || lower.contains("v4-pro")
}

fn resolve_thinking_param_pattern(
    preset: &ProviderCapabilityPreset,
    protocol: &ProviderProtocol,
    model_name: &str,
) -> ThinkingParamPattern {
    match preset {
        ProviderCapabilityPreset::OpenAiChat => ThinkingParamPattern::None,
        ProviderCapabilityPreset::OpenAiReasoning => ThinkingParamPattern::EffortStandard,
        ProviderCapabilityPreset::AnthropicThinking => ThinkingParamPattern::AnthropicThinking,
        ProviderCapabilityPreset::DeepseekChat => ThinkingParamPattern::EffortWithNone,
        ProviderCapabilityPreset::DeepseekReasoner => {
            if is_deepseek_v4_by_model(model_name) {
                ThinkingParamPattern::EffortWithNone
            } else {
                ThinkingParamPattern::ThinkingToggle
            }
        }
        ProviderCapabilityPreset::Auto | ProviderCapabilityPreset::Custom => {
            let lower = model_name.to_ascii_lowercase();
            if !lower.contains("deepseek") {
                if protocol.is_anthropic() {
                    ThinkingParamPattern::AnthropicThinking
                } else if preset == &ProviderCapabilityPreset::Auto {
                    ThinkingParamPattern::None
                } else {
                    ThinkingParamPattern::None
                }
            } else if is_deepseek_v4_by_model(model_name) {
                ThinkingParamPattern::EffortWithNone
            } else {
                ThinkingParamPattern::ThinkingToggle
            }
        }
    }
}

fn preset_model_capabilities(
    preset: &ProviderCapabilityPreset,
    protocol: &ProviderProtocol,
    _model_name: &str,
) -> ProviderModelCapabilities {
    let inferred_preset = if matches!(preset, ProviderCapabilityPreset::Auto) {
        infer_capability_preset(protocol, _model_name)
    } else {
        preset.clone()
    };

    match inferred_preset {
        ProviderCapabilityPreset::OpenAiChat => ProviderModelCapabilities {
            context_window_tokens: Some(DEFAULT_CONTEXT_WINDOW_TOKENS),
            supports_tools: true,
            supports_streaming: true,
            supports_image_input: true,
            supports_reasoning: false,
            supports_video_input: false,
            supports_audio_input: false,
            supports_image_output: false,
            supports_video_output: false,
            supports_audio_output: false,
        },
        ProviderCapabilityPreset::OpenAiReasoning => ProviderModelCapabilities {
            context_window_tokens: Some(DEFAULT_CONTEXT_WINDOW_TOKENS),
            supports_tools: true,
            supports_streaming: true,
            supports_image_input: false,
            supports_reasoning: true,
            supports_video_input: false,
            supports_audio_input: false,
            supports_image_output: false,
            supports_video_output: false,
            supports_audio_output: false,
        },
        ProviderCapabilityPreset::AnthropicThinking => ProviderModelCapabilities {
            context_window_tokens: Some(DEFAULT_CONTEXT_WINDOW_TOKENS),
            supports_tools: true,
            supports_streaming: true,
            supports_image_input: true,
            supports_reasoning: true,
            supports_video_input: false,
            supports_audio_input: false,
            supports_image_output: false,
            supports_video_output: false,
            supports_audio_output: false,
        },
        ProviderCapabilityPreset::DeepseekChat => ProviderModelCapabilities {
            context_window_tokens: Some(DEFAULT_CONTEXT_WINDOW_TOKENS),
            supports_tools: true,
            supports_streaming: true,
            supports_image_input: false,
            supports_reasoning: false,
            supports_video_input: false,
            supports_audio_input: false,
            supports_image_output: false,
            supports_video_output: false,
            supports_audio_output: false,
        },
        ProviderCapabilityPreset::DeepseekReasoner => ProviderModelCapabilities {
            context_window_tokens: Some(DEFAULT_CONTEXT_WINDOW_TOKENS),
            supports_tools: true,
            supports_streaming: true,
            supports_image_input: false,
            supports_reasoning: true,
            supports_video_input: false,
            supports_audio_input: false,
            supports_image_output: false,
            supports_video_output: false,
            supports_audio_output: false,
        },
        ProviderCapabilityPreset::Auto => ProviderModelCapabilities {
            context_window_tokens: Some(DEFAULT_CONTEXT_WINDOW_TOKENS),
            supports_tools: true,
            supports_streaming: true,
            supports_image_input: true,
            supports_reasoning: true,
            supports_video_input: false,
            supports_audio_input: false,
            supports_image_output: false,
            supports_video_output: false,
            supports_audio_output: false,
        },
        ProviderCapabilityPreset::Custom => ProviderModelCapabilities::default(),
    }
}

fn default_true() -> bool {
    true
}

/// CAPABILITY_CATALOG 的 protocol 字符串按家族匹配：
/// catalog 条目 "openai" 命中 openai-completions 与 openai-responses 两个协议。
fn catalog_matches_protocol(protocol: &ProviderProtocol, expected: Option<&str>) -> bool {
    match expected {
        None => true,
        Some("openai") => protocol.is_openai_family(),
        Some("anthropic") => protocol.is_anthropic(),
        Some(_) => false,
    }
}

fn normalize_capability_preset(
    protocol: &ProviderProtocol,
    model_name: &str,
    capability_preset: &ProviderCapabilityPreset,
) -> ProviderCapabilityPreset {
    match capability_preset {
        ProviderCapabilityPreset::Custom => ProviderCapabilityPreset::Custom,
        ProviderCapabilityPreset::Auto => {
            if model_name.trim().is_empty() {
                infer_capability_preset(protocol, default_model(protocol))
            } else {
                ProviderCapabilityPreset::Auto
            }
        }
        preset => preset.clone(),
    }
}

fn normalize_model_capabilities(
    protocol: &ProviderProtocol,
    model_name: &str,
    capability_preset: &ProviderCapabilityPreset,
    capabilities: ProviderModelCapabilities,
) -> ProviderModelCapabilities {
    if matches!(capability_preset, ProviderCapabilityPreset::Custom) {
        let defaults = default_model_capabilities(protocol, model_name);
        return ProviderModelCapabilities {
            context_window_tokens: capabilities
                .context_window_tokens
                .or(defaults.context_window_tokens),
            supports_tools: capabilities.supports_tools,
            supports_streaming: capabilities.supports_streaming,
            supports_image_input: capabilities.supports_image_input,
            supports_reasoning: capabilities.supports_reasoning,
            supports_video_input: capabilities.supports_video_input,
            supports_audio_input: capabilities.supports_audio_input,
            supports_image_output: capabilities.supports_image_output,
            supports_video_output: capabilities.supports_video_output,
            supports_audio_output: capabilities.supports_audio_output,
        };
    }

    let defaults = preset_model_capabilities(capability_preset, protocol, model_name);
    let looks_like_legacy_empty = capabilities.context_window_tokens.is_none()
        && !capabilities.supports_tools
        && !capabilities.supports_streaming
        && !capabilities.supports_image_input
        && !capabilities.supports_reasoning
        && !capabilities.supports_video_input
        && !capabilities.supports_audio_input
        && !capabilities.supports_image_output
        && !capabilities.supports_video_output
        && !capabilities.supports_audio_output;
    let looks_like_implicit_defaults = capabilities.context_window_tokens.is_none()
        && capabilities.supports_tools
        && capabilities.supports_streaming
        && !capabilities.supports_image_input
        && !capabilities.supports_reasoning
        && !capabilities.supports_video_input
        && !capabilities.supports_audio_input
        && !capabilities.supports_image_output
        && !capabilities.supports_video_output
        && !capabilities.supports_audio_output;

    if looks_like_legacy_empty || looks_like_implicit_defaults {
        return defaults;
    }

    defaults
}

fn resolve_model_capability_declaration(
    protocol: &ProviderProtocol,
    model_name: &str,
    capability_preset: &ProviderCapabilityPreset,
    capabilities: ProviderModelCapabilities,
) -> ProviderModelCapabilityDeclaration {
    let normalized_preset = normalize_capability_preset(protocol, model_name, capability_preset);
    let normalized_capabilities =
        normalize_model_capabilities(protocol, model_name, &normalized_preset, capabilities);

    ProviderModelCapabilityDeclaration {
        capability_preset: normalized_preset,
        capabilities: normalized_capabilities,
    }
}

fn normalize_model_user_policy(
    model: &ProviderModelConfig,
    capabilities: &ProviderModelCapabilities,
) -> ProviderModelUserPolicy {
    ProviderModelUserPolicy {
        temperature: model.temperature,
        max_output_tokens: model.max_output_tokens,
        reasoning_effort: if capabilities.supports_reasoning {
            model.reasoning_effort.clone()
        } else {
            None
        },
        reasoning_budget_tokens: if capabilities.supports_reasoning {
            model.reasoning_budget_tokens
        } else {
            None
        },
    }
}

fn slugify(value: &str, fallback: &str) -> String {
    let mut slug = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();

    slug = slug
        .split('-')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("-");

    if slug.is_empty() {
        fallback.to_string()
    } else {
        slug
    }
}

fn derive_env_var_name(provider_name: &str) -> String {
    let mut env_name = provider_name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect::<String>();

    env_name = env_name
        .split('_')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("_");

    if env_name.is_empty() {
        "CUSTOM_PROVIDER_API_KEY".to_string()
    } else {
        format!("{}_API_KEY", env_name)
    }
}

fn default_base_url(protocol: &ProviderProtocol) -> &'static str {
    // responses 复用 api.openai.com/v1（同一 API 根，endpoint 为 {base}/responses）。
    match protocol {
        ProviderProtocol::OpenAiCompletions | ProviderProtocol::OpenAiResponses => {
            "https://api.openai.com/v1"
        }
        ProviderProtocol::AnthropicMessages => "https://api.anthropic.com/v1",
    }
}

fn default_model(protocol: &ProviderProtocol) -> &'static str {
    match protocol {
        // responses 默认模型沿用 gpt-4.1-mini。
        ProviderProtocol::OpenAiCompletions | ProviderProtocol::OpenAiResponses => "gpt-4.1-mini",
        ProviderProtocol::AnthropicMessages => "claude-3-7-sonnet-latest",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_capabilities_fields_use_stable_defaults() {
        let storage = serde_json::from_str::<ProviderRegistryStorage>(
            r#"
            {
              "providers": [
                {
                  "id": "provider-openai",
                  "name": "openai",
                  "protocol": "openai",
                  "base_url": "https://api.openai.com/v1",
                  "api_key_env_var": "OPENAI_API_KEY",
                  "models": [
                    {
                      "id": "model-openai-default",
                      "name": "GPT 5.5",
                      "model": "gpt-5.5",
                      "temperature": 0.2,
                      "maxOutputTokens": 8192,
                      "capabilities": {
                        "supportsReasoning": true
                      },
                      "reasoningEffort": "medium",
                      "reasoningBudgetTokens": 2048
                    }
                  ],
                  "selected_model_id": "model-openai-default"
                }
              ],
              "selected_provider_id": "provider-openai"
            }
            "#,
        )
        .expect("storage should deserialize");

        let normalized = normalize_storage(storage);
        let model = &normalized.providers[0].models[0];

        assert_eq!(model.capabilities.context_window_tokens, Some(1_000_000));
        assert!(model.capabilities.supports_tools);
        assert!(model.capabilities.supports_streaming);
        assert!(model.capabilities.supports_reasoning);
        assert_eq!(
            model
                .reasoning_effort
                .as_ref()
                .map(|effort| matches!(effort, ProviderReasoningEffort::Medium)),
            Some(true)
        );
        assert_eq!(model.reasoning_budget_tokens, Some(2048));
    }

    #[test]
    fn auto_preset_is_preserved_while_capabilities_are_inferred() {
        let storage = serde_json::from_str::<ProviderRegistryStorage>(
            r#"
            {
              "providers": [
                {
                  "id": "provider-openai",
                  "name": "openai",
                  "protocol": "openai",
                  "base_url": "https://api.openai.com/v1",
                  "api_key_env_var": "OPENAI_API_KEY",
                  "models": [
                    {
                      "id": "model-openai-auto",
                      "name": "GPT 5 Auto",
                      "model": "gpt-5.5",
                      "temperature": 0.2,
                      "maxOutputTokens": 8192,
                      "capabilityPreset": "auto"
                    }
                  ],
                  "selected_model_id": "model-openai-auto"
                }
              ],
              "selected_provider_id": "provider-openai"
            }
            "#,
        )
        .expect("fixture should parse");

        let normalized = normalize_storage(storage);
        let model = &normalized.providers[0].models[0];

        assert!(matches!(
            model.capability_preset,
            ProviderCapabilityPreset::Auto
        ));
        assert_eq!(model.capabilities.context_window_tokens, Some(1_000_000));
        assert!(model.capabilities.supports_reasoning);
    }

    #[test]
    fn disabling_reasoning_clears_reasoning_fields() {
        let storage = serde_json::from_str::<ProviderRegistryStorage>(
            r#"
            {
              "providers": [
                {
                  "id": "provider-openai",
                  "name": "openai",
                  "protocol": "openai",
                  "base_url": "https://api.openai.com/v1",
                  "api_key_env_var": "OPENAI_API_KEY",
                  "models": [
                    {
                      "id": "model-openai-default",
                      "name": "GPT 4.1 Mini",
                      "model": "gpt-4.1-mini",
                      "temperature": 0.2,
                      "maxOutputTokens": 8192,
                      "capabilities": {
                        "supportsReasoning": false
                      },
                      "reasoningEffort": "high",
                      "reasoningBudgetTokens": 4096
                    }
                  ],
                  "selected_model_id": "model-openai-default"
                }
              ],
              "selected_provider_id": "provider-openai"
            }
            "#,
        )
        .expect("storage should deserialize");

        let normalized = normalize_storage(storage);
        let model = &normalized.providers[0].models[0];

        assert!(!model.capabilities.supports_reasoning);
        assert!(model.reasoning_effort.is_none());
        assert!(model.reasoning_budget_tokens.is_none());
        assert_eq!(model.temperature, 0.2);
        assert_eq!(model.max_output_tokens, 64000);
    }

    #[test]
    fn custom_capability_preset_preserves_manual_capabilities() {
        let storage = serde_json::from_str::<ProviderRegistryStorage>(
            r#"
            {
              "providers": [
                {
                  "id": "provider-openai",
                  "name": "openai",
                  "protocol": "openai",
                  "base_url": "https://api.openai.com/v1",
                  "api_key_env_var": "OPENAI_API_KEY",
                  "models": [
                    {
                      "id": "model-openai-custom",
                      "name": "Custom Chat",
                      "model": "gpt-4.1-mini",
                      "temperature": 0.2,
                      "maxOutputTokens": 8192,
                      "capabilityPreset": "custom",
                      "capabilities": {
                        "contextWindowTokens": 64000,
                        "supportsTools": false,
                        "supportsStreaming": false,
                        "supportsImageInput": true,
                        "supportsReasoning": false
                      }
                    }
                  ],
                  "selected_model_id": "model-openai-custom"
                }
              ],
              "selected_provider_id": "provider-openai"
            }
            "#,
        )
        .expect("storage should deserialize");

        let normalized = normalize_storage(storage);
        let model = &normalized.providers[0].models[0];

        assert!(matches!(
            model.capability_preset,
            ProviderCapabilityPreset::Custom
        ));
        assert_eq!(model.capabilities.context_window_tokens, Some(64_000));
        assert!(!model.capabilities.supports_tools);
        assert!(!model.capabilities.supports_streaming);
        assert!(model.capabilities.supports_image_input);
    }

    #[test]
    fn existing_custom_registry_does_not_reinsert_default_providers() {
        let storage = serde_json::from_str::<ProviderRegistryStorage>(
            r#"
            {
              "providers": [
                {
                  "id": "acme-router",
                  "name": "Acme Router",
                  "protocol": "openai",
                  "base_url": "https://router.example/v1",
                  "api_key_env_var": "ACME_ROUTER_API_KEY",
                  "api_key_value": "secret-token",
                  "models": [
                    {
                      "id": "reasoning-alpha",
                      "name": "Reasoning Alpha",
                      "model": "gpt-5-alpha",
                      "temperature": 0.2,
                      "maxOutputTokens": 8192,
                      "capabilityPreset": "open-ai-reasoning"
                    }
                  ],
                  "selected_model_id": "reasoning-alpha"
                }
              ],
              "selected_provider_id": "acme-router"
            }
            "#,
        )
        .expect("storage should deserialize");

        let normalized = normalize_storage(storage);

        assert_eq!(normalized.providers.len(), 1);
        assert_eq!(normalized.providers[0].id, "acme-router");
        assert_eq!(
            normalized.selected_provider_id.as_deref(),
            Some("acme-router")
        );
    }

    #[test]
    fn default_deepseek_template_includes_distinct_v4_pro_model() {
        let storage = normalize_storage(ProviderRegistryStorage {
            upstream_policy: UpstreamPolicyStorage::default(),
            failover_provider_ids: Vec::new(),
            providers: Vec::new(),
            selected_provider_id: Some("provider-deepseek".to_string()),
        });
        let deepseek = storage
            .providers
            .iter()
            .find(|provider| provider.id == "provider-deepseek")
            .expect("default deepseek provider should exist");
        let pro = deepseek
            .models
            .iter()
            .find(|model| model.id == "model-deepseek-v4-pro")
            .expect("deepseek v4 pro model should be present");

        assert_eq!(pro.model, "deepseek-v4-pro");
        assert_ne!(pro.model, "deepseek-v4-flash");
        assert!(matches!(
            pro.capability_preset,
            ProviderCapabilityPreset::DeepseekReasoner
        ));
        assert!(pro.capabilities.supports_reasoning);

        let migrated = normalize_storage(ProviderRegistryStorage {
            upstream_policy: UpstreamPolicyStorage::default(),
            failover_provider_ids: Vec::new(),
            providers: vec![ProviderConfigStorage {
                id: "provider-deepseek".to_string(),
                name: "deepseek".to_string(),
                protocol: ProviderProtocol::OpenAiCompletions,
                base_url: "https://api.deepseek.com/v1".to_string(),
                auth_type: ProviderAuthType::Auto,
                api_key_env_var: "DEEPSEEK_API_KEY".to_string(),
                secret_ref: default_secret_ref("provider-deepseek"),
                api_key_value: String::new(),
                selected_model_id: Some("model-deepseek-default".to_string()),
                models: vec![ProviderModelConfig {
                    id: "model-deepseek-default".to_string(),
                    name: "DeepSeek V4 Flash".to_string(),
                    model: "deepseek-v4-flash".to_string(),
                    temperature: 0.2,
                    max_output_tokens: DEFAULT_MAX_OUTPUT_TOKENS,
                    capability_preset: ProviderCapabilityPreset::DeepseekChat,
                    reasoning_effort: None,
                    reasoning_budget_tokens: None,
                    protocol: Some(ProviderProtocol::OpenAiCompletions),
                    base_url: None,
                    capabilities: ProviderModelCapabilities::default(),
                }],
                supported_protocols: vec![ProviderProtocol::OpenAiCompletions],
                endpoints: vec![ProviderProtocolEndpoint {
                    protocol: ProviderProtocol::OpenAiCompletions,
                    enabled: true,
                    base_url: "https://api.deepseek.com/v1".to_string(),
                    auth_type: ProviderAuthType::Auto,
                }],
            }],
            selected_provider_id: Some("provider-deepseek".to_string()),
        });
        let migrated_deepseek = migrated
            .providers
            .iter()
            .find(|provider| provider.id == "provider-deepseek")
            .expect("migrated provider should exist");

        assert!(migrated_deepseek
            .models
            .iter()
            .any(|model| model.model == "deepseek-v4-pro"));

        let prefixed_existing = normalize_storage(ProviderRegistryStorage {
            upstream_policy: UpstreamPolicyStorage::default(),
            failover_provider_ids: Vec::new(),
            providers: vec![ProviderConfigStorage {
                id: "provider-deepseek".to_string(),
                name: "deepseek".to_string(),
                protocol: ProviderProtocol::OpenAiCompletions,
                base_url: "https://api.deepseek.com/v1".to_string(),
                auth_type: ProviderAuthType::Auto,
                api_key_env_var: "DEEPSEEK_API_KEY".to_string(),
                secret_ref: default_secret_ref("provider-deepseek"),
                api_key_value: String::new(),
                selected_model_id: Some("model-deepseek-v4-pro-prefixed".to_string()),
                models: vec![
                    ProviderModelConfig {
                        id: "model-deepseek-default".to_string(),
                        name: "DeepSeek V4 Flash".to_string(),
                        model: "deepseek-v4-flash".to_string(),
                        temperature: 0.2,
                        max_output_tokens: DEFAULT_MAX_OUTPUT_TOKENS,
                        capability_preset: ProviderCapabilityPreset::DeepseekChat,
                        reasoning_effort: None,
                        reasoning_budget_tokens: None,
                        protocol: Some(ProviderProtocol::OpenAiCompletions),
                        base_url: None,
                        capabilities: ProviderModelCapabilities::default(),
                    },
                    ProviderModelConfig {
                        id: "model-deepseek-v4-pro-prefixed".to_string(),
                        name: "DeepSeek V4 Pro".to_string(),
                        model: "deepseek/deepseek-v4-pro".to_string(),
                        temperature: 0.2,
                        max_output_tokens: DEFAULT_MAX_OUTPUT_TOKENS,
                        capability_preset: ProviderCapabilityPreset::DeepseekReasoner,
                        reasoning_effort: Some(ProviderReasoningEffort::Medium),
                        reasoning_budget_tokens: None,
                        protocol: Some(ProviderProtocol::OpenAiCompletions),
                        base_url: None,
                        capabilities: ProviderModelCapabilities::default(),
                    },
                ],
                supported_protocols: vec![ProviderProtocol::OpenAiCompletions],
                endpoints: vec![ProviderProtocolEndpoint {
                    protocol: ProviderProtocol::OpenAiCompletions,
                    enabled: true,
                    base_url: "https://api.deepseek.com/v1".to_string(),
                    auth_type: ProviderAuthType::Auto,
                }],
            }],
            selected_provider_id: Some("provider-deepseek".to_string()),
        });
        let prefixed_deepseek = prefixed_existing
            .providers
            .iter()
            .find(|provider| provider.id == "provider-deepseek")
            .expect("prefixed provider should exist");

        assert_eq!(
            prefixed_deepseek
                .models
                .iter()
                .filter(|model| is_deepseek_v4_pro_model_value(&model.model))
                .count(),
            1
        );
    }

    #[test]
    fn normalize_storage_deduplicates_model_values_within_provider() {
        let normalized = normalize_storage(ProviderRegistryStorage {
            upstream_policy: UpstreamPolicyStorage::default(),
            failover_provider_ids: Vec::new(),
            providers: vec![ProviderConfigStorage {
                id: "provider-openai".to_string(),
                name: "openai".to_string(),
                protocol: ProviderProtocol::OpenAiCompletions,
                base_url: "https://api.openai.com/v1".to_string(),
                auth_type: ProviderAuthType::Auto,
                api_key_env_var: "OPENAI_API_KEY".to_string(),
                secret_ref: default_secret_ref("provider-openai"),
                api_key_value: String::new(),
                selected_model_id: Some("model-gpt-5-copy".to_string()),
                models: vec![
                    ProviderModelConfig {
                        id: "model-gpt-5".to_string(),
                        name: "GPT-5".to_string(),
                        model: "gpt-5".to_string(),
                        temperature: 0.2,
                        max_output_tokens: DEFAULT_MAX_OUTPUT_TOKENS,
                        capability_preset: ProviderCapabilityPreset::OpenAiReasoning,
                        reasoning_effort: Some(ProviderReasoningEffort::Medium),
                        reasoning_budget_tokens: None,
                        protocol: Some(ProviderProtocol::OpenAiCompletions),
                        base_url: None,
                        capabilities: ProviderModelCapabilities::default(),
                    },
                    ProviderModelConfig {
                        id: "model-gpt-5-copy".to_string(),
                        name: "GPT-5 Copy".to_string(),
                        model: " GPT-5 ".to_string(),
                        temperature: 0.2,
                        max_output_tokens: DEFAULT_MAX_OUTPUT_TOKENS,
                        capability_preset: ProviderCapabilityPreset::OpenAiReasoning,
                        reasoning_effort: Some(ProviderReasoningEffort::Medium),
                        reasoning_budget_tokens: None,
                        protocol: Some(ProviderProtocol::OpenAiCompletions),
                        base_url: None,
                        capabilities: ProviderModelCapabilities::default(),
                    },
                ],
                supported_protocols: vec![ProviderProtocol::OpenAiCompletions],
                endpoints: vec![ProviderProtocolEndpoint {
                    protocol: ProviderProtocol::OpenAiCompletions,
                    enabled: true,
                    base_url: "https://api.openai.com/v1".to_string(),
                    auth_type: ProviderAuthType::Auto,
                }],
            }],
            selected_provider_id: Some("provider-openai".to_string()),
        });
        let provider = &normalized.providers[0];

        assert_eq!(provider.models.len(), 1);
        assert_eq!(provider.models[0].id, "model-gpt-5");
        assert_eq!(provider.selected_model_id.as_deref(), Some("model-gpt-5"));
    }

    fn temp_registry_root(label: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        let root = { let raw = std::env::temp_dir().join(format!("pony-agent-config-{label}-{stamp}")); raw.canonicalize().unwrap_or(raw) };
        fs::create_dir_all(&root).expect("create temp registry root");
        root
    }

    /// B6 别名四位置往返：provider.protocol / supportedProtocols[] /
    /// endpoints[].protocol / models[].protocol 含旧值 → 读 → 存 → 全规范名。
    #[test]
    fn legacy_protocol_aliases_roundtrip_to_canonical_names_in_four_positions() {
        let root = temp_registry_root("alias-roundtrip");
        let path = root.join("providers.json");
        fs::write(
            &path,
            r#"
            {
              "providers": [
                {
                  "id": "legacy-provider",
                  "name": "legacy",
                  "protocol": "openai",
                  "base_url": "https://legacy.example/v1",
                  "api_key_env_var": "LEGACY_API_KEY",
                  "supported_protocols": ["anthropic"],
                  "endpoints": [
                    { "protocol": "openai", "enabled": true, "baseUrl": "https://legacy.example/v1", "authType": "auto" },
                    { "protocol": "anthropic", "enabled": true, "baseUrl": "https://legacy.example/v1", "authType": "x-api-key" }
                  ],
                  "models": [
                    {
                      "id": "model-legacy",
                      "name": "Legacy Model",
                      "model": "claude-3-7-sonnet-latest",
                      "temperature": 0.2,
                      "maxOutputTokens": 8192,
                      "capabilityPreset": "anthropic-thinking",
                      "protocol": "anthropic"
                    }
                  ],
                  "selected_model_id": "model-legacy"
                }
              ],
              "selected_provider_id": "legacy-provider"
            }
            "#,
        )
        .expect("write legacy registry");

        let store = ProviderRegistryStore::with_path(&path);
        // 反序列化接受旧值（serde alias）。
        let view = store.load_view();
        assert_eq!(view.providers.len(), 1);
        assert!(matches!(
            view.providers[0].models[0].protocol,
            Some(ProviderProtocol::AnthropicMessages)
        ));

        // 保存后只写规范名（View 与落盘文件两层校验）。
        let saved = store
            .save_view_without_env_sync(view)
            .expect("save normalized registry");
        assert_eq!(
            saved.providers[0].supported_protocols,
            vec![
                ProviderProtocol::AnthropicMessages,
                ProviderProtocol::OpenAiCompletions
            ]
        );
        for endpoint in &saved.providers[0].endpoints {
            assert!(matches!(
                endpoint.protocol,
                ProviderProtocol::OpenAiCompletions
                    | ProviderProtocol::OpenAiResponses
                    | ProviderProtocol::AnthropicMessages
            ));
        }
        assert_eq!(
            saved.providers[0].models[0].protocol,
            Some(ProviderProtocol::AnthropicMessages)
        );

        let persisted: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).expect("read persisted registry"))
                .expect("persisted registry should be JSON");
        let provider_json = &persisted["providers"][0];
        assert_eq!(provider_json["protocol"], "anthropic-messages");
        // ProviderConfigStorage（落盘层）provider 级字段为 snake_case。
        assert_eq!(
            provider_json["supported_protocols"][0],
            "anthropic-messages"
        );
        // 三协议槽位各一个 endpoint；所有协议字段只允许规范名。
        let endpoint_protocols: Vec<String> = provider_json["endpoints"]
            .as_array()
            .expect("endpoints array")
            .iter()
            .map(|entry| entry["protocol"].as_str().expect("endpoint protocol").to_string())
            .collect();
        assert_eq!(
            endpoint_protocols,
            vec![
                "openai-completions".to_string(),
                "openai-responses".to_string(),
                "anthropic-messages".to_string(),
            ]
        );
        assert_eq!(
            provider_json["models"][0]["protocol"],
            "anthropic-messages"
        );

        let _ = fs::remove_dir_all(root);
    }

    /// B6 resolve_selection 解析序：模型覆盖 > enabled endpoint > 主协议 base_url /
    /// 默认值；endpoint 显式 auth 尊重、Auto 按家族推导（请求期）。
    #[test]
    fn resolve_selection_prefers_model_overrides_then_endpoint_chain() {
        let root = temp_registry_root("resolve-selection");
        let path = root.join("providers.json");
        let storage = ProviderRegistryStorage {
            upstream_policy: UpstreamPolicyStorage::default(),
            failover_provider_ids: Vec::new(),
            providers: vec![ProviderConfigStorage {
                id: "multi-protocol".to_string(),
                name: "multi".to_string(),
                protocol: ProviderProtocol::OpenAiCompletions,
                // 与 completions endpoint 一致（normalize 会把主协议 endpoint URL
                // 同步为 provider.base_url，两者必须同值才能测出链路而非同步效应）。
                base_url: "https://chat.example/v1".to_string(),
                auth_type: ProviderAuthType::Auto,
                api_key_env_var: "MULTI_API_KEY".to_string(),
                secret_ref: default_secret_ref("multi-protocol"),
                api_key_value: String::new(),
                selected_model_id: Some("model-inherit".to_string()),
                models: vec![
                    ProviderModelConfig {
                        id: "model-inherit".to_string(),
                        name: "Inherit".to_string(),
                        model: "gpt-4.1-mini".to_string(),
                        temperature: 0.2,
                        max_output_tokens: DEFAULT_MAX_OUTPUT_TOKENS,
                        capability_preset: ProviderCapabilityPreset::OpenAiChat,
                        reasoning_effort: None,
                        reasoning_budget_tokens: None,
                        protocol: None,
                        base_url: None,
                        capabilities: ProviderModelCapabilities::default(),
                    },
                    ProviderModelConfig {
                        id: "model-anthropic-override".to_string(),
                        name: "Anthropic Override".to_string(),
                        model: "claude-3-7-sonnet-latest".to_string(),
                        temperature: 0.2,
                        max_output_tokens: DEFAULT_MAX_OUTPUT_TOKENS,
                        capability_preset: ProviderCapabilityPreset::AnthropicThinking,
                        reasoning_effort: None,
                        reasoning_budget_tokens: None,
                        protocol: Some(ProviderProtocol::AnthropicMessages),
                        base_url: Some("  https://override.example/  ".to_string()),
                        capabilities: ProviderModelCapabilities::default(),
                    },
                    ProviderModelConfig {
                        id: "model-responses-endpoint".to_string(),
                        name: "Responses Endpoint".to_string(),
                        model: "gpt-5.4".to_string(),
                        temperature: 0.2,
                        max_output_tokens: DEFAULT_MAX_OUTPUT_TOKENS,
                        capability_preset: ProviderCapabilityPreset::OpenAiReasoning,
                        reasoning_effort: None,
                        reasoning_budget_tokens: None,
                        protocol: Some(ProviderProtocol::OpenAiResponses),
                        base_url: None,
                        capabilities: ProviderModelCapabilities::default(),
                    },
                    ProviderModelConfig {
                        id: "model-explicit-auth".to_string(),
                        name: "Explicit Auth".to_string(),
                        model: "gpt-4.1-mini".to_string(),
                        temperature: 0.2,
                        max_output_tokens: DEFAULT_MAX_OUTPUT_TOKENS,
                        capability_preset: ProviderCapabilityPreset::OpenAiChat,
                        reasoning_effort: None,
                        reasoning_budget_tokens: None,
                        protocol: Some(ProviderProtocol::OpenAiCompletions),
                        base_url: None,
                        capabilities: ProviderModelCapabilities::default(),
                    },
                ],
                supported_protocols: vec![
                    ProviderProtocol::OpenAiCompletions,
                    ProviderProtocol::OpenAiResponses,
                    ProviderProtocol::AnthropicMessages,
                ],
                endpoints: vec![
                    ProviderProtocolEndpoint {
                        protocol: ProviderProtocol::OpenAiCompletions,
                        enabled: true,
                        base_url: "https://chat.example/v1".to_string(),
                        auth_type: ProviderAuthType::Auto,
                    },
                    ProviderProtocolEndpoint {
                        protocol: ProviderProtocol::OpenAiResponses,
                        enabled: true,
                        base_url: "https://responses.example/v1".to_string(),
                        auth_type: ProviderAuthType::Bearer,
                    },
                    ProviderProtocolEndpoint {
                        protocol: ProviderProtocol::AnthropicMessages,
                        enabled: false,
                        base_url: default_base_url(&ProviderProtocol::AnthropicMessages)
                            .to_string(),
                        auth_type: ProviderAuthType::XApiKey,
                    },
                ],
            }],
            selected_provider_id: Some("multi-protocol".to_string()),
        };
        fs::write(&path, serde_json::to_string(&storage).expect("serialize storage"))
            .expect("write storage");
        let store = ProviderRegistryStore::with_path(&path);

        // 无覆盖：走 completions enabled endpoint 的 URL，Auto 保持 Auto。
        let inherited = store.resolve_selection(Some("multi-protocol"), Some("model-inherit"));
        assert_eq!(inherited.base_url, "https://chat.example/v1");
        assert_eq!(inherited.auth_type, ProviderAuthType::Auto);

        // 模型级协议 + base_url 覆盖：trim 后生效，thinking pattern 按 anthropic 家族解析。
        let overridden =
            store.resolve_selection(Some("multi-protocol"), Some("model-anthropic-override"));
        assert_eq!(overridden.protocol, ProviderProtocol::AnthropicMessages);
        assert_eq!(overridden.base_url, "https://override.example/");
        assert_eq!(
            overridden.thinking_param_pattern,
            ThinkingParamPattern::AnthropicThinking
        );

        // 模型协议无覆盖：该协议 enabled endpoint 提供URL + 显式 Bearer 尊重。
        let responses = store.resolve_selection(Some("multi-protocol"), Some("model-responses-endpoint"));
        assert_eq!(responses.protocol, ProviderProtocol::OpenAiResponses);
        assert_eq!(responses.base_url, "https://responses.example/v1");
        assert_eq!(responses.auth_type, ProviderAuthType::Bearer);

        // anthropic endpoint 未启用：回落主协议 base_url？协议不一致 → 协议默认值；
        // 但显式 x-api-key auth 来自未启用 endpoint 不应生效 → Auto。
        // （本用例中 anthropic 走的是模型覆盖路径；这里补一个无覆盖且 endpoint 关闭的路径。）
        let disabled_endpoint_storage = ProviderRegistryStorage {
            upstream_policy: UpstreamPolicyStorage::default(),
            failover_provider_ids: Vec::new(),
            providers: vec![ProviderConfigStorage {
                id: "closed-shop".to_string(),
                name: "closed".to_string(),
                protocol: ProviderProtocol::OpenAiCompletions,
                base_url: "https://main.example/v1".to_string(),
                auth_type: ProviderAuthType::Auto,
                api_key_env_var: "CLOSED_API_KEY".to_string(),
                secret_ref: default_secret_ref("closed-shop"),
                api_key_value: String::new(),
                selected_model_id: Some("model-anthropic".to_string()),
                models: vec![ProviderModelConfig {
                    id: "model-anthropic".to_string(),
                    name: "Anthropic No Endpoint".to_string(),
                    model: "claude-3-7-sonnet-latest".to_string(),
                    temperature: 0.2,
                    max_output_tokens: DEFAULT_MAX_OUTPUT_TOKENS,
                    capability_preset: ProviderCapabilityPreset::AnthropicThinking,
                    reasoning_effort: None,
                    reasoning_budget_tokens: None,
                    protocol: Some(ProviderProtocol::AnthropicMessages),
                    base_url: None,
                    capabilities: ProviderModelCapabilities::default(),
                }],
                supported_protocols: vec![ProviderProtocol::OpenAiCompletions],
                endpoints: vec![ProviderProtocolEndpoint {
                    protocol: ProviderProtocol::OpenAiCompletions,
                    enabled: true,
                    base_url: "https://chat.example/v1".to_string(),
                    auth_type: ProviderAuthType::Auto,
                }],
            }],
            selected_provider_id: Some("closed-shop".to_string()),
        };
        let closed_path = root.join("closed-providers.json");
        fs::write(
            &closed_path,
            serde_json::to_string(&disabled_endpoint_storage).expect("serialize storage"),
        )
        .expect("write storage");
        let closed_store = ProviderRegistryStore::with_path(&closed_path);
        let resolved = closed_store.resolve_selection(Some("closed-shop"), Some("model-anthropic"));
        // anthropic 模型协议 ∉ supported（仅 completions）→ 回落主协议 openai-completions，
        // base_url 走其 enabled endpoint（normalize 已将其与 provider.base_url 同步为
        // main.example，此处验证回落链而非覆盖链）。
        assert_eq!(resolved.protocol, ProviderProtocol::OpenAiCompletions);
        assert_eq!(resolved.base_url, "https://main.example/v1");

        let _ = fs::remove_dir_all(root);
    }

    /// R6：registry 文件反序列化失败时先备份 `*.invalid-<ts>` 再回落默认注册表。
    #[test]
    fn load_storage_backs_up_unparseable_registry_before_default_fallback() {
        let root = temp_registry_root("invalid-backup");
        let path = root.join("providers.json");
        fs::write(&path, "{ this is definitely not valid json !!").expect("write broken registry");

        let store = ProviderRegistryStore::with_path(&path);
        let view = store.load_view();

        // 回落默认注册表。
        assert!(view
            .providers
            .iter()
            .any(|provider| provider.id == "provider-ppx"));

        // 同目录存在 invalid 备份，内容即原文件。
        let backup = fs::read_dir(&root)
            .expect("list registry dir")
            .filter_map(|entry| entry.ok())
            .find(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("providers.json.invalid-")
            })
            .expect("invalid registry backup should exist");
        let backed_up = fs::read_to_string(backup.path()).expect("read backup");
        assert!(backed_up.contains("definitely not valid json"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn provider_api_key_resolves_exact_id_only_and_never_falls_back() {
        // code-review B/P1-2：id 未命中必须返回 None，禁止回退选中/首个提供商，
        // 否则构成「A 的密钥发往为 B 填写的 URL」的凭证错配通路。
        let root = temp_registry_root("api-key-chain");
        let path = root.join("providers.json");
        fs::create_dir_all(&root).expect("mkdir");
        // 单一自定义提供商、无 secret_ref/无存储密钥；避免默认注册表与本机环境变量干扰。
        fs::write(
            &path,
            r#"{"providers":[{"id":"p-a","name":"a","protocol":"openai-completions","base_url":"https://a.example.invalid/v1","api_key_env_var":"PONY_TEST_UNSET_KEY","models":[]}],"selectedProviderId":"p-a"}"#,
        )
        .expect("write registry");

        let store = ProviderRegistryStore::with_path(&path);
        assert_eq!(store.provider_api_key(Some("p-a")), None);
        assert_eq!(store.provider_api_key(Some("provider-not-exists")), None);
        assert_eq!(store.provider_api_key(None), None);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn provider_endpoint_auth_prefers_explicit_override_and_defaults_to_auto() {
        // 缺省 registry → 默认模板无显式 auth 覆盖：Auto；未知提供商/协议同样 Auto。
        let root = temp_registry_root("endpoint-auth");
        let store = ProviderRegistryStore::with_path(root.join("providers.json"));
        let view = store.load_view();
        let provider = &view.providers[0];
        let protocol = provider.protocol.clone();

        assert!(matches!(
            store.provider_endpoint_auth(Some(&provider.id), &protocol),
            ProviderAuthType::Auto
        ));
        assert!(matches!(
            store.provider_endpoint_auth(Some("provider-not-exists"), &protocol),
            ProviderAuthType::Auto
        ));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn default_added_model_capabilities_have_reasoning_images_1m_context_64k_output() {
        let default_caps = default_model_capabilities(&ProviderProtocol::OpenAiCompletions, "custom-model");
        assert_eq!(default_caps.context_window_tokens, Some(1_000_000));
        assert!(default_caps.supports_reasoning);
        assert!(default_caps.supports_image_input);

        let custom_caps = preset_model_capabilities(
            &ProviderCapabilityPreset::Custom,
            &ProviderProtocol::OpenAiCompletions,
            "custom-model",
        );
        assert_eq!(custom_caps.context_window_tokens, Some(1_000_000));
        assert!(custom_caps.supports_reasoning);
        assert!(custom_caps.supports_image_input);
    }
}

