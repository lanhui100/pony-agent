//! Agent Trace 的 OpenTelemetry 装配与门禁面（Wave43 B2 实现落地）。
//!
//! 本模块是 trace 上报的**唯一**契约面：公开签名只使用中性类型（[`AttrValue`]、
//! 各类 `*SpanInput`、[`OtelConfig`]），任何 OTel SDK 类型都被封装在私有实现里，
//! 以保证契约可被无网络、无全局态的单测直接驱动。
//!
//! 隐私门禁总契约：全局上报开关默认**关闭**；关闭态必须做到**零 span 分配、零网络出口**
//! （由 `GatedSampler` 在关闭态直接返回 `Drop` 决策保证）。手动上传
//! （[`force_export_session_trace`]）是逐次授权的旁路动作，不受该开关约束，且**不改变**
//! 开关末态（RAII 旁路守卫：递增 → 作用域结束递减）。
//!
//! 导出通路：OTLP/HTTP-protobuf（reqwest 异步客户端）+ `BatchSpanProcessor` 后台线程。
//! reqwest 异步客户端在发送时需要一个 tokio reactor，而批量导出线程不是 tokio 线程，
//! 因此 [`init`] 建立一个 1-worker runtime，并在导出 future 内部 `Handle::enter()`
//! 之后才 poll 内层导出（`RuntimeExporter`）；runtime 句柄随全局态存活，[`shutdown`]
//! 时才随之释放。

/// 最小 JSON 日志面（D8）：仅服务本次新增 trace 代码路径。
pub mod log;

// ---------------------------------------------------------------- span 名（冻结）

/// Turn root span 名。语义：一个 turn 对应一棵 trace 的根 span，子 span 与之同 trace_id。
pub const SPAN_AGENT_TURN: &str = "agent.turn";

/// LLM 调用子 span 名。语义：一次模型请求一个 span，父 span 为 [`SPAN_AGENT_TURN`]。
pub const SPAN_LLM_CHAT: &str = "llm.chat";

/// 工具调用子 span 名。语义：一次工具执行一个 span，父 span 为 [`SPAN_AGENT_TURN`]。
pub const SPAN_TOOL_CALL: &str = "tool.call";

// ------------------------------------------------------------ 属性键（OTel GenAI 惯例）

/// 模型供应商种类（如 `anthropic` / `openai`）。语义轨路由谓词之一：缺失则 Langfuse 收不到。
pub const ATTR_GEN_AI_SYSTEM: &str = "gen_ai.system";

/// 请求模型名。契约名（非 `gen_ai.model`）。
pub const ATTR_GEN_AI_REQUEST_MODEL: &str = "gen_ai.request.model";

/// 输入 token 数。
pub const ATTR_GEN_AI_USAGE_INPUT_TOKENS: &str = "gen_ai.usage.input_tokens";

/// 输出 token 数。
pub const ATTR_GEN_AI_USAGE_OUTPUT_TOKENS: &str = "gen_ai.usage.output_tokens";

/// 思考（推理）token 数（wave-51 C3 契约键；**semconv 标准名**）。
///
/// semconv 口径（OTel GenAI registry `gen_ai.usage.reasoning.output_tokens`）：该值 SHOULD 已包含在
/// [`ATTR_GEN_AI_USAGE_OUTPUT_TOKENS`] 之内（`output_tokens` 含思考 token，禁止二次相加）。
/// 由 [`llm_usage_detail_attributes`] 在有值时产出；缺失 ⇒ **不得输出该键**。
pub const ATTR_GEN_AI_USAGE_REASONING_TOKENS: &str = "gen_ai.usage.reasoning.output_tokens";

/// 缓存命中输入 token 数（wave-51 C3 契约键；**semconv 标准名**）。
///
/// semconv 口径（OTel GenAI registry `gen_ai.usage.cache_read.input_tokens`）：该值 SHOULD 已包含在
/// [`ATTR_GEN_AI_USAGE_INPUT_TOKENS`] 之内（`input_tokens` 含缓存命中 token）。
/// Langfuse ingest 会从 `input_tokens` 减本值、从 `output_tokens` 减思考值以避免重复计费
/// ⇒ 仅当上游 `input_tokens` **确实包含**缓存命中时才允许上报（协议门控见契约 PA51-C3-T6）。
pub const ATTR_GEN_AI_USAGE_CACHED_INPUT_TOKENS: &str = "gen_ai.usage.cache_read.input_tokens";

/// 模型生成速度（t/s）。
pub const ATTR_GEN_AI_SPEED_TOKENS_PER_SECOND: &str = "gen_ai.speed.tokens_per_second";

/// 上下文窗口总大小（tokens）。
pub const ATTR_GEN_AI_USAGE_CONTEXT_WINDOW: &str = "gen_ai.usage.context_window";

/// 上下文使用比例（0.0 ~ 1.0）。
pub const ATTR_GEN_AI_USAGE_CONTEXT_RATIO: &str = "gen_ai.usage.context_ratio";

/// 整轮综合生成速度（t/s）。
pub const ATTR_AGENT_TURN_SPEED_TOKENS_PER_SECOND: &str = "agent.turn.speed.tokens_per_second";

/// 单次 LLM 调用成本（USD，f64）。Wave-5 K4：未知模型/门禁关闭时**不输出该键**。
pub const ATTR_GEN_AI_COST_USD: &str = "gen_ai.cost.usd";

/// Wave-7 G5：治理决策 span 名（宏观调度入口，唯一允许的治理 span）。
pub const SPAN_GOVERNANCE_DECIDE: &str = "governance.decide";

/// 治理 span 属性白名单（G5.1；**严禁**扩展为金额/阈值等动态数字）。
pub const ATTR_GOVERNANCE_WINDOW: &str = "governance.window";
pub const ATTR_GOVERNANCE_DECISION: &str = "governance.decision";
pub const ATTR_GOVERNANCE_REASON: &str = "governance.reason";
pub const ATTR_GOVERNANCE_POLICY_VERSION: &str = "governance.policy_version";
pub const ATTR_GOVERNANCE_UNPRICED_TURNS: &str = "governance.unpriced_turns";

/// 整轮 turn 聚合成本（USD，f64）。Wave-5 K4：无有效计价时**不输出该键**（严禁写 0.0）。
pub const ATTR_AGENT_TURN_COST_USD: &str = "agent.turn.cost.usd";

/// 工具名（如 `bash`）。
pub const ATTR_GEN_AI_TOOL_NAME: &str = "gen_ai.tool.name";

/// 操作名；turn root span 上的取值恒为 [`OPERATION_AGENT_TURN`]。
pub const ATTR_GEN_AI_OPERATION_NAME: &str = "gen_ai.operation.name";

// ------------------------------------------------------- 属性键（Langfuse 语义轨）

/// Langfuse 会话 id：同一会话的多轮 turn 聚合到一条 Langfuse session。
pub const ATTR_LANGFUSE_SESSION_ID: &str = "langfuse.session.id";

/// Langfuse 用户 id：有则带，无则**不输出该键**。
pub const ATTR_LANGFUSE_USER_ID: &str = "langfuse.user.id";

/// Langfuse trace 名：turn 级展示名。
pub const ATTR_LANGFUSE_TRACE_NAME: &str = "langfuse.trace.name";

/// Langfuse observation 类型；工具 span 固定取 [`OBSERVATION_TYPE_TOOL`]。
pub const ATTR_LANGFUSE_OBSERVATION_TYPE: &str = "langfuse.observation.type";

/// Langfuse observation 模型名：与 [`ATTR_GEN_AI_REQUEST_MODEL`] 同值双写。
pub const ATTR_LANGFUSE_OBSERVATION_MODEL_NAME: &str = "langfuse.observation.model.name";

// ------------------------------------------------------- 属性键（pony-agent 自有维度）

/// turn 标识，用于 Langfuse 侧与本地会话记录的互跳。
pub const ATTR_AGENT_TURN_ID: &str = "agent.turn.id";

/// turn 在会话内的序号（0-based）。
pub const ATTR_AGENT_TURN_SEQUENCE: &str = "agent.turn.sequence";

/// turn 终态相位（如 `completed` / `failed` / `cancelled`）。
pub const ATTR_AGENT_TURN_PHASE: &str = "agent.turn.phase";

/// turn 终态错误聚合结论（PA-154）：**仅**本轮聚合到子 span 错误时输出（值固定 `error`）；
/// 全成功轮次**不输出该键**（保持「有 = 有错」的单义语义，避免污染失败率看板）。
pub const ATTR_AGENT_TURN_OUTCOME: &str = "agent.turn.outcome";

/// 工具执行状态（如 `success` / `failed`）。
pub const ATTR_AGENT_TOOL_STATUS: &str = "agent.tool.status";

/// 工具执行耗时（毫秒整数）。
pub const ATTR_AGENT_TOOL_DURATION_MS: &str = "agent.tool.duration_ms";

/// 工具错误摘要：有则带，无则**不输出该键**。
pub const ATTR_AGENT_TOOL_ERROR: &str = "agent.tool.error";

/// turn root span 上 `gen_ai.operation.name` 的取值（与 span 名同字面量）。
pub const OPERATION_AGENT_TURN: &str = "agent.turn";

/// 工具 span 上 `langfuse.observation.type` 的取值。
pub const OBSERVATION_TYPE_TOOL: &str = "tool";

// ------------------------------------------------------------- 缺省值与环境变量（D3）

/// 集群内 collector 的 OTLP/HTTP 端点常量（4318 端口）。
///
/// Wave-4 C1.4：桌面端**内置缺省**端点已改为公网 [`DEFAULT_PUBLIC_ENDPOINT`]；本常量保留为
/// 「集群内直连」的显式取值（供测试/运维按需经 env 覆盖），**不再**是 [`OtelConfig::default`]
/// 的取值。历史冻结断言（`otel_trace_migration_test.rs` D3 缺省值扫描）逐字锁定本常量，故不改值。
pub const DEFAULT_ENDPOINT: &str = "http://otel-collector.observability.svc.cluster.local:4318";

/// 桌面端内置缺省 OTLP/HTTP 端点（公网 edge，Wave-4 C1.4）。
///
/// 语义：`settings` 与 `env` 均未提供端点时的最终回落值；端点补齐 `/v1/traces` 由导出层负责。
pub const DEFAULT_PUBLIC_ENDPOINT: &str = "https://otel.ponyjob.top";

/// `service.name` 资源属性缺省值。
pub const DEFAULT_SERVICE_NAME: &str = "pony-agent";

/// 采样率缺省值：桌面端属显式授权 + 单机低频场景，缺省全采。
pub const DEFAULT_SAMPLING_RATIO: f64 = 1.0;

/// 端点覆盖用环境变量名；优先级高于 [`DEFAULT_ENDPOINT`]。
pub const ENV_OTLP_ENDPOINT: &str = "OTEL_EXPORTER_OTLP_ENDPOINT";

/// 服务名覆盖用环境变量名。
pub const ENV_SERVICE_NAME: &str = "OTEL_SERVICE_NAME";

/// 采样率覆盖用环境变量名。
pub const ENV_SAMPLER_ARG: &str = "OTEL_TRACES_SAMPLER_ARG";

/// 公网 OTLP 端点鉴权 token 的环境变量名；仅作为 header 注入，不落盘、不入库。
pub const ENV_PONY_OTEL_TOKEN: &str = "PONY_OTEL_TOKEN";

/// `service.version` resource 属性的进程 env 覆盖名（PA-153）；trim 后非空即覆盖默认值。
pub const ENV_PONY_OTEL_SERVICE_VERSION: &str = "PONY_OTEL_SERVICE_VERSION";

/// `deployment.environment` resource 属性的进程 env 覆盖名（PA-153）；trim 后非空即覆盖默认值。
pub const ENV_PONY_OTEL_ENVIRONMENT: &str = "PONY_OTEL_ENVIRONMENT";

/// 探针自报标记环境变量名（O4 探针隔离）；取值为 `1` / `true` / `probe` 时 resource 自动声明 `pony.probe=true`。
pub const ENV_PONY_OTEL_PROBE: &str = "PONY_OTEL_PROBE";

/// Langfuse `langfuse.user.id` 属性覆盖环境变量名（PA-163）；trim 后非空即注入 root span。
pub const ENV_PONY_OTEL_USER_ID: &str = "PONY_OTEL_USER_ID";

// ------------------------------------------------------ resource 归因键（PA-153 / O4）
//
// 归因维度只在 resource 层表达（span 属性不动）；键名取自 OTel resource 语义约定。

/// resource 语义约定键：服务版本（缺省 = 本 crate 的 `CARGO_PKG_VERSION`）。
pub(crate) const RESOURCE_SERVICE_VERSION: &str = "service.version";

/// resource 语义约定键：部署环境（缺省按构建档位推导，见 [`default_deployment_environment`]）。
pub(crate) const RESOURCE_DEPLOYMENT_ENVIRONMENT: &str = "deployment.environment";

/// resource 语义约定键：探针隔离标记（集群边缘 edge 探针隔离契约源侧自报）。
pub(crate) const RESOURCE_PONY_PROBE: &str = "pony.probe";

/// resource 语义约定键：上报客户端归因标记（区分真实设备与边缘探针）。
pub(crate) const RESOURCE_INGEST_CLIENT_KIND: &str = "ingest.client.kind";

/// resource 语义约定键：跨路径稳定产品名。
///
/// 契约依据：公网 edge 会把 `service.name` 改写为 `desktop-client`（原值放入 `client.service.name`），
/// 但不会改写带 `pony.` 前缀的语义约定属性（skill 契约 §1.0）。因此 `pony.app.name`
/// 是两条接入路径（内网直连 collector 与公网 edge）唯一保持一致的跨路径检索键。
pub(crate) const RESOURCE_PONY_APP_NAME: &str = "pony.app.name";

/// 产品稳定名。
pub(crate) const PONY_APP_NAME: &str = "pony-agent";

/// 客户端归因标记取值：真实设备客户端。
pub(crate) const INGEST_CLIENT_KIND_DEVICE: &str = "device";

/// 客户端归因标记取值：边缘探针。
pub(crate) const INGEST_CLIENT_KIND_PROBE: &str = "probe";

/// debug 构建的默认部署环境取值。
pub(crate) const ENVIRONMENT_DEVELOPMENT: &str = "development";

/// release 构建的默认部署环境取值。
pub(crate) const ENVIRONMENT_PRODUCTION: &str = "production";

/// 构建期烧录的准入 token 的 env 名（`option_env!` 读取，仅 release 工作流注入）。
///
/// 为什么需要它：桌面端此前**没有任何 token 分发通道** —— 用户必须在 Settings 里手填
/// 公网 edge 的 OTel token，没填就静默 401、trace 全丢（实测：24h 零 trace，而同期探针正常）。
/// 与 PonySentry 的构建期烧录（ADR 0037）同款取舍：安装包内嵌 token，用户零配置。
/// 优先级最低（settings > env > 构建期），因此用户/运维仍可覆盖或吊销为本地值。
pub const ENV_PONY_OTEL_BUILD_TOKEN: &str = "PONY_OTEL_BUILD_TOKEN";

/// 构建期烧录值（未注入时为 `None`；本地 `cargo build` 恒为 None）。
fn build_time_otel_token() -> Option<&'static str> {
    option_env!("PONY_OTEL_BUILD_TOKEN")
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

/// 鉴权 header 名（token 值由 [`ENV_PONY_OTEL_TOKEN`] 注入）。
pub const AUTH_HEADER_NAME: &str = "authorization";

// ---------------------------------------------------------------------- 中性值类型

/// 属性值的中性表示：调用方/测试只与本类型交互，不感知 OTel `Value`。
#[derive(Clone, Debug, PartialEq)]
pub enum AttrValue {
    /// 字符串属性。
    Str(String),
    /// 有符号整数属性（token 数、耗时毫秒等）。
    I64(i64),
    /// 浮点属性（采样率、比率等）。
    F64(f64),
    /// 布尔属性。
    Bool(bool),
}

/// turn root span 的属性映射输入。
#[derive(Clone, Debug, PartialEq)]
pub struct TurnSpanInput {
    /// 会话 id，映射到 [`ATTR_LANGFUSE_SESSION_ID`]。
    pub session_id: String,
    /// 用户 id；`None` 时不得输出 [`ATTR_LANGFUSE_USER_ID`]。
    pub user_id: Option<String>,
    /// turn id，映射到 [`ATTR_AGENT_TURN_ID`]。
    pub turn_id: String,
    /// 会话内 turn 序号，映射到 [`ATTR_AGENT_TURN_SEQUENCE`]。
    pub sequence: u64,
    /// turn 终态相位，映射到 [`ATTR_AGENT_TURN_PHASE`]。
    pub phase: String,
}

/// LLM 子 span 的属性映射输入。
#[derive(Clone, Debug, PartialEq)]
pub struct LlmSpanInput {
    /// 供应商种类，映射到 [`ATTR_GEN_AI_SYSTEM`]（语义轨路由谓词）。
    pub provider_kind: String,
    /// 模型名；`None` 时不得输出模型相关属性。
    pub model: Option<String>,
    /// 输入 token 数；`None` 时不得输出 [`ATTR_GEN_AI_USAGE_INPUT_TOKENS`]。
    pub input_tokens: Option<u64>,
    /// 输出 token 数；`None` 时不得输出 [`ATTR_GEN_AI_USAGE_OUTPUT_TOKENS`]。
    pub output_tokens: Option<u64>,
    /// 操作名，映射到 [`ATTR_GEN_AI_OPERATION_NAME`]。
    pub operation: String,
}

/// 工具子 span 的属性映射输入。
#[derive(Clone, Debug, PartialEq)]
pub struct ToolSpanInput {
    /// 工具名，映射到 [`ATTR_GEN_AI_TOOL_NAME`]。
    pub tool_name: String,
    /// 执行状态，映射到 [`ATTR_AGENT_TOOL_STATUS`]。
    pub status: String,
    /// 耗时毫秒；`None` 时不得输出 [`ATTR_AGENT_TOOL_DURATION_MS`]。
    pub duration_ms: Option<u64>,
    /// 错误摘要；`None` 时不得输出 [`ATTR_AGENT_TOOL_ERROR`]。
    pub error: Option<String>,
}

/// trace 装配配置。
///
/// 契约：由宿主装配点（Tauri 启动路径）构造后传给 [`init`]；字段可被环境变量覆盖，
/// 见 [`OtelConfig::from_env`]。
#[derive(Clone, PartialEq)]
pub struct OtelConfig {
    /// OTLP/HTTP 端点；缺省 [`DEFAULT_PUBLIC_ENDPOINT`]（桌面端公网出口 `https://otel.ponyjob.top`）。
    ///
    /// 说明：集群内 collector（[`DEFAULT_ENDPOINT`]）**只是**一个常量取值，需经
    /// [`ENV_OTLP_ENDPOINT`] 或 settings 显式指定才会生效，不再是内置缺省。
    pub endpoint: String,
    /// `service.name`；缺省 [`DEFAULT_SERVICE_NAME`]。
    pub service_name: String,
    /// 采样率（0.0~1.0）；缺省 [`DEFAULT_SAMPLING_RATIO`]。
    pub sampling_ratio: f64,
    /// 额外导出 header（名称, 值）；缺省为空（鉴权 token 由 [`ENV_PONY_OTEL_TOKEN`] 注入）。
    pub headers: Vec<(String, String)>,
}

impl fmt::Debug for OtelConfig {
    /// F1：手写 Debug，**绝不**打印任何 header 值（鉴权 token 一律 `[REDACTED]`）。
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let headers: Vec<(&str, &str)> = self
            .headers
            .iter()
            .map(|(name, _value)| (name.as_str(), "[REDACTED]"))
            .collect();
        formatter
            .debug_struct("OtelConfig")
            .field("endpoint", &self.endpoint)
            .field("service_name", &self.service_name)
            .field("sampling_ratio", &self.sampling_ratio)
            .field("headers", &headers)
            .finish()
    }
}

impl Default for OtelConfig {
    /// 契约（缺省语义）：`endpoint` = [`DEFAULT_PUBLIC_ENDPOINT`]（桌面端公网出口）、
    /// `service_name` = [`DEFAULT_SERVICE_NAME`]、`sampling_ratio` = [`DEFAULT_SAMPLING_RATIO`]、
    /// `headers` 为空。
    fn default() -> Self {
        Self {
            endpoint: DEFAULT_PUBLIC_ENDPOINT.to_string(),
            service_name: DEFAULT_SERVICE_NAME.to_string(),
            sampling_ratio: DEFAULT_SAMPLING_RATIO,
            headers: Vec::new(),
        }
    }
}

/// settings 持久化来源的中性快照（Wave-4 C1：`settings > env > 内置默认`）。
///
/// 契约：由宿主（Tauri `load_app_settings` 路径）从 `AppSettings` 构造；
/// 字段为 `None` 表示「该项未配置」——必须继续向下一来源回落，**不得**用空串占位。
/// 凭据（[`OtelSettingsSnapshot::token`]）只允许流入导出 header，绝不进入日志/回显
/// （因此本类型**手写 Debug**，token 一律 `[REDACTED]`）。
#[derive(Clone, Default, PartialEq)]
pub struct OtelSettingsSnapshot {
    /// 用户配置的 OTLP 端点；`None`/空串 ⇒ 回落 env ⇒ 内置公网缺省。
    pub endpoint: Option<String>,
    /// 用户配置的鉴权 token；`None`/空串 ⇒ 回落 env ⇒ 不注入（零 egress 凭证态）。
    pub token: Option<String>,
    /// 用户配置的 `service.name`；`None`/空串 ⇒ 回落 env ⇒ [`DEFAULT_SERVICE_NAME`]。
    pub service_name: Option<String>,
    /// 用户配置的采样率；`None` 或越界 ⇒ 回落 env ⇒ [`DEFAULT_SAMPLING_RATIO`]。
    pub sampling_ratio: Option<f64>,
}

impl fmt::Debug for OtelSettingsSnapshot {
    /// F1/C1.5：**绝不**打印 token 明文（与 [`OtelConfig`] 手写 Debug 同一约束）。
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OtelSettingsSnapshot")
            .field("endpoint", &self.endpoint)
            .field(
                "token",
                &self.token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("service_name", &self.service_name)
            .field("sampling_ratio", &self.sampling_ratio)
            .finish()
    }
}

/// 按「settings > env > 内置默认」解析最终生效配置（纯函数，无全局态、无网络 IO）。
///
/// 调用方：宿主启动装配点（`otel::init` 之前）与单测；`env` 参数显式传入，
/// **不读进程环境**（进程环境只在 [`from_env`] 里采集），保证可测性与可重放性。
/// 语义：
/// - `settings` 的每个非空字段优先于 `env` 同名项；
/// - `env` 覆盖内置缺省（[`ENV_OTLP_ENDPOINT`] / [`ENV_SERVICE_NAME`] / [`ENV_SAMPLER_ARG`] /
///   [`ENV_PONY_OTEL_TOKEN`]）；
/// - token 统一组装为 [`AUTH_HEADER_NAME`] header（值见 [`authorization_header_value`]），
///   settings 与 env 互斥生效，绝不重复叠加；
/// - 采样率越界（非有限值或不在 0.0~1.0）时该来源**拒绝采纳**并降级到下一来源，不 panic。
pub fn resolve_config(
    settings: Option<&OtelSettingsSnapshot>,
    env: Option<&[(&str, &str)]>,
) -> OtelConfig {
    resolve_config_with_build_token(settings, env, build_time_otel_token())
}

/// [`resolve_config`] 的可测内核：把「构建期烧录 token」作为参数显式传入。
///
/// 拆出来是为了让「最低优先级兜底」可被单测锁定——`option_env!` 在测试构建里恒为 `None`，
/// 不拆开就无法验证 "settings/env 都没给时用构建期值" 与 "用户显式配置不会被覆盖"。
pub fn resolve_config_with_build_token(
    settings: Option<&OtelSettingsSnapshot>,
    env: Option<&[(&str, &str)]>,
    build_token: Option<&str>,
) -> OtelConfig {
    let mut config = OtelConfig::default();
    let lookup = |name: &str| -> Option<String> {
        env.and_then(|pairs| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.trim().to_string())
                .filter(|value| !value.is_empty())
        })
    };

    // ---- 第二优先级：进程 env ----
    if let Some(endpoint) = lookup(ENV_OTLP_ENDPOINT) {
        config.endpoint = endpoint;
    }
    if let Some(service_name) = lookup(ENV_SERVICE_NAME) {
        config.service_name = service_name;
    }
    if let Some(raw_ratio) = lookup(ENV_SAMPLER_ARG) {
        match parse_sampling_ratio(&raw_ratio) {
            Some(ratio) => config.sampling_ratio = ratio,
            None => log::log_json(
                "warn",
                &format!(
                    "otel::resolve_config：{ENV_SAMPLER_ARG} 取值非法，已回落缺省采样率 {DEFAULT_SAMPLING_RATIO}：{raw_ratio}"
                ),
                None,
            ),
        }
    }
    if let Some(token) = lookup(ENV_PONY_OTEL_TOKEN) {
        set_authorization_header(&mut config.headers, &token);
    }

    // ---- 第一优先级：settings 持久化 ----
    if let Some(settings) = settings {
        if let Some(endpoint) = non_empty_setting(settings.endpoint.as_deref()) {
            config.endpoint = endpoint;
        }
        if let Some(service_name) = non_empty_setting(settings.service_name.as_deref()) {
            config.service_name = service_name;
        }
        if let Some(ratio) = settings.sampling_ratio {
            match parse_sampling_ratio(&ratio.to_string()) {
                Some(ratio) => config.sampling_ratio = ratio,
                None => log::log_json(
                    "warn",
                    &format!(
                        "otel::resolve_config：settings 采样率越界（须 0.0~1.0），已忽略并沿用上一来源：{ratio}"
                    ),
                    None,
                ),
            }
        }
        // C1.5：settings token 为空串 ⇒ 等同未配置（继续沿用 env 结果 / 零凭证）。
        if let Some(token) = non_empty_setting(settings.token.as_deref()) {
            set_authorization_header(&mut config.headers, &token);
        }
    }

    // 最低优先级兜底：构建期烧录 token（release 产物零配置可用）。
    // 仅在 settings 与 env 都没给凭证时生效，因此不会覆盖用户显式配置。
    if config.headers.is_empty() {
        if let Some(token) = build_token.map(str::trim).filter(|value| !value.is_empty()) {
            set_authorization_header(&mut config.headers, token);
        }
    }

    config
}

impl OtelConfig {
    /// 由 settings 持久化快照 + 进程环境变量解析生效配置（宿主启动装配点）。
    ///
    /// 语义：等价于 `resolve_config(settings, Some(进程 env 白名单))`；先按 env 组装，
    /// 再用 settings 覆盖。失败语义：不返回错误（越界/非法项降级为 warn 日志）。
    pub fn from_settings(settings: Option<&OtelSettingsSnapshot>) -> OtelConfig {
        let pairs = process_env_pairs();
        let borrowed: Vec<(&str, &str)> = pairs
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect();
        resolve_config(settings, Some(borrowed.as_slice()))
    }
}

/// settings 字符串项清洗：`None`/纯空白 ⇒ `None`（回落下一来源）。
fn non_empty_setting(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// 采样率解析：仅接受有限且落在 0.0~1.0 的值，其余返回 `None`（拒绝采纳）。
fn parse_sampling_ratio(raw: &str) -> Option<f64> {
    match raw.trim().parse::<f64>() {
        Ok(ratio) if ratio.is_finite() && (0.0..=1.0).contains(&ratio) => Some(ratio),
        _ => None,
    }
}

/// 设置鉴权 header：先摘除同名的既有项（settings 覆盖 env，绝不叠加）。
fn set_authorization_header(headers: &mut Vec<(String, String)>, token: &str) {
    headers.retain(|(name, _)| !name.eq_ignore_ascii_case(AUTH_HEADER_NAME));
    headers.push((
        AUTH_HEADER_NAME.to_string(),
        authorization_header_value(token),
    ));
}

/// 采集进程环境变量白名单（仅 OTel 相关四项）；未设置/空白项不出现在结果里。
fn process_env_pairs() -> Vec<(String, String)> {
    [ENV_OTLP_ENDPOINT, ENV_SERVICE_NAME, ENV_SAMPLER_ARG, ENV_PONY_OTEL_TOKEN]
        .iter()
        .filter_map(|name| {
            env_non_empty(name).map(|value| ((*name).to_string(), value))
        })
        .collect()
}

/// 默认部署环境（PA-153 / task-13）：
/// - 优先级 1（运行时环境）：`PONY_OTEL_ENVIRONMENT`（由 `resolve_resource_attributes` 处理）；
/// - 优先级 2（构建期发布标记）：若设置了 `PONY_RELEASE_BUILD=1`（release.yml CI 流程注入），则为 `production`；
/// - 优先级 3（构建档位兜底）：仅当 `PONY_RELEASE_BUILD` 未设时，缺省判定为 `development`（即使 debug_assertions=false
///   也是本地 release 构建的开发/调试会话，避免误标 production）。
pub(crate) fn default_deployment_environment_with_release_flag(
    debug_assertions: bool,
    release_build: Option<&str>,
) -> &'static str {
    if let Some(flag) = release_build {
        if flag == "1" || flag.eq_ignore_ascii_case("true") {
            return ENVIRONMENT_PRODUCTION;
        }
    }
    let _ = debug_assertions;
    ENVIRONMENT_DEVELOPMENT
}

pub(crate) fn default_deployment_environment(debug_assertions: bool) -> &'static str {
    default_deployment_environment_with_release_flag(debug_assertions, option_env!("PONY_RELEASE_BUILD"))
}

/// 解析 resource 归因属性（PA-153）：`service.version` + `deployment.environment`。
///
/// 语义：`env` 取值 **trim 后非空**即覆盖默认；空白串等同未设置（回落默认）；
/// `service.version` 默认取本 crate 的 `CARGO_PKG_VERSION`。
/// 纯函数（env 由调用方注入）：实现期单测据此覆盖 release 分支与空白串回退。
pub(crate) fn resolve_resource_attributes(
    env: impl Fn(&str) -> Option<String>,
) -> Vec<(String, String)> {
    let override_value = |name: &str| {
        env(name)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    };
    let service_version = override_value(ENV_PONY_OTEL_SERVICE_VERSION)
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());
    let environment = override_value(ENV_PONY_OTEL_ENVIRONMENT)
        .unwrap_or_else(|| default_deployment_environment(cfg!(debug_assertions)).to_string());
    let mut attrs = vec![
        (RESOURCE_SERVICE_VERSION.to_string(), service_version),
        (RESOURCE_DEPLOYMENT_ENVIRONMENT.to_string(), environment),
        (RESOURCE_PONY_APP_NAME.to_string(), PONY_APP_NAME.to_string()),
    ];
    let mut is_probe = false;
    if let Some(probe) = override_value(ENV_PONY_OTEL_PROBE) {
        if probe.eq_ignore_ascii_case("true") || probe == "1" || probe.eq_ignore_ascii_case("probe") {
            attrs.push((RESOURCE_PONY_PROBE.to_string(), "true".to_string()));
            is_probe = true;
        }
    }
    let client_kind = if is_probe {
        INGEST_CLIENT_KIND_PROBE
    } else {
        INGEST_CLIENT_KIND_DEVICE
    };
    attrs.push((RESOURCE_INGEST_CLIENT_KIND.to_string(), client_kind.to_string()));
    attrs
}

/// 装配入口（wave-51 C1 契约桩）：由调用方显式注入安装包版本后再建立全局 trace provider。
///
/// 优先级契约（`service.version` resource 属性，三级，高者优先）：
/// 1. 显式入参 `service_version`；
/// 2. 进程 env [`ENV_PONY_OTEL_SERVICE_VERSION`]；
/// 3. 本 crate 的 `CARGO_PKG_VERSION`（兜底）。
///
/// 空白语义：`service_version` 与 env 取值 **trim 后为空**一律视为**未设置**，
/// 逐级下落到下一来源（与 [`resolve_resource_attributes`] 既有空白回退语义一致）。
/// 取值合法化（逐级判定，每一级都过同一规则）：① 先 trim；② 拒绝任何 `char::is_control()`
/// 控制字符（含 `\n` / `\r` / `\t` / NUL）；③ trim 后字节长度上限 **64**；
/// 任一条不合规 ⇒ 该级视为**未设置**并下落一级（**不得**把非法值写进 resource、不得 panic）。
/// 关系：[`init`] 等价于本函数传入 `None`；校验/失败/可重入语义与 [`init`] 完全一致，
/// 二者互斥 —— 同一进程生命周期内至多成功装配一次。
///
/// [STUB wave-51] 空桩：仅冻结签名与语义，实现归 Executor。
pub fn init_with_service_version(
    config: OtelConfig,
    service_version: Option<&str>,
) -> Result<(), String> {
    let _ = (config, service_version);
    unimplemented!("wave-51 C1 契约桩：init_with_service_version 待实现")
}

/// 装配入口：安装全局 trace provider。
///
/// 调用方：宿主启动路径（Tauri 装配点），进程生命周期内**至多一次**。
/// 语义：以 `config` 建立导出通路与资源属性；provider 常驻，开关只翻转门禁位
/// （[`init`] **不改变**上报开关，缺省仍为 `false`）。
/// 失败语义：配置非法或端点不可解析时返回 `Err`，调用方记录告警后**继续启动**，
/// 不得 panic、不得阻断主链路；失败时全局态整体回滚为「未 init」，不留半初始化状态。
/// 可重入：本函数返回 `Ok` 后，需先 [`shutdown`] 才能再次 [`init`]。
pub fn init(config: OtelConfig) -> Result<(), String> {
    if config.endpoint.trim().is_empty() {
        return Err("otel::init：OTLP 端点不得为空".to_string());
    }
    // ADVISORY(F1)：绝不把鉴权 token 送上明文 HTTP 外网端点。
    if let Some((name, _)) = config
        .headers
        .iter()
        .find(|(name, _)| is_sensitive_header(name))
    {
        if is_plaintext_remote(&config.endpoint) {
            return Err(format!(
                "otel::init：拒绝在明文 HTTP 的外网端点上发送鉴权 header `{name}`（请改用 https）"
            ));
        }
    }
    if !config.sampling_ratio.is_finite() || !(0.0..=1.0).contains(&config.sampling_ratio) {
        return Err(format!(
            "otel::init：采样率必须落在 0.0~1.0，实际 {}",
            config.sampling_ratio
        ));
    }
    {
        let guard = lock_state();
        if guard.is_some() {
            return Err("otel::init：已初始化，需先调用 shutdown() 才能再次 init()".to_string());
        }
    }

    // 导出用 1-worker runtime：先建 runtime，再在它的上下文里构造 reqwest 客户端。
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .thread_name(RUNTIME_THREAD_NAME)
        .build()
        .map_err(|err| format!("otel::init：导出 runtime 创建失败：{err}"))?;

    // 任一步失败都直接返回 Err：此时全局态仍为 None（整体回滚），runtime 随之释放。
    let exporter = {
        let _runtime_context = runtime.enter();
        build_runtime_exporter(&config, runtime.handle().clone())?
    };

    let resource = opentelemetry_sdk::Resource::builder()
        .with_service_name(config.service_name.clone())
        // PA-153：resource 补齐归因维度（service.version / deployment.environment）；
        // 每份 ResourceSpans 各一次，值来自 env 覆盖或构建档位默认。
        .with_attributes(
            resolve_resource_attributes(env_non_empty)
                .into_iter()
                .map(|(key, value)| KeyValue::new(key, opentelemetry::Value::String(value.into()))),
        )
        .build();
    // ADVISORY：批量管道显式有界（队列/批量/并发/超时都钉死，不依赖 SDK 缺省与 env 漂移）。
    let batch_config = opentelemetry_sdk::trace::BatchConfigBuilder::default()
        .with_max_queue_size(BATCH_MAX_QUEUE_SIZE)
        .with_max_export_batch_size(BATCH_MAX_EXPORT_BATCH_SIZE)
        .with_scheduled_delay(Duration::from_millis(BATCH_SCHEDULED_DELAY_MS))
        .build();
    let batch_processor = BatchSpanProcessor::builder(exporter)
        .with_batch_config(batch_config)
        .build();
    let provider = SdkTracerProvider::builder()
        .with_sampler(GatedSampler {
            ratio: config.sampling_ratio,
        })
        .with_resource(resource)
        .with_span_processor(batch_processor)
        .build();
    let tracer = provider.tracer(TRACER_NAME);

    let state = Arc::new(OtelState {
        provider,
        tracer,
        config,
        _runtime: runtime,
    });

    let mut guard = lock_state();
    if guard.is_some() {
        // 并发 init 竞争：后者整体回滚，不覆盖已生效的全局态。
        return Err("otel::init：并发初始化竞争，已存在生效中的 provider".to_string());
    }

    let auth_configured = state
        .config
        .headers
        .iter()
        .any(|(name, _)| is_sensitive_header(name));
    if !auth_configured && is_remote_endpoint(&state.config.endpoint) {
        log::log_json(
            "warn",
            &format!(
                "otel::init：导出已启用但未配置鉴权 token → 公网 edge 会拒绝（401），\
                 本进程 trace 不会出现在 Tempo/Langfuse；请在 Settings 填 OTel token 或设 PONY_OTEL_TOKEN（端点：{}）",
                state.config.endpoint
            ),
            None,
        );
    }

    *guard = Some(state);
    Ok(())
}

/// 装配逆过程：flush 并释放 provider 及其后台资源。
///
/// 调用方：宿主退出路径（Tauri 退出事件），必须显式调用一次。
/// 语义：幂等，可重复调用；返回后允许再次 [`init`]。
/// 失败语义：不返回错误——任何内部异常都必须在内部吞掉并降级为日志，绝不 panic。
pub fn shutdown() {
    shutdown_bounded(SHUTDOWN_TIMEOUT_MS);
}

/// 带预算的 [`shutdown`]：provider 关闭等待上限由调用方给定（毫秒）。
///
/// 调用方：进程退出路径（Wave-4.1 A5：退出总预算 ≤3000ms 时传入 1000ms 档位）。
/// 语义：与 [`shutdown`] 完全一致（幂等、不 panic、失败只降级 warn），仅把内部
/// `shutdown_with_timeout` 的等待上限替换为 `timeout_ms`（内部仍按 `MAX_FLUSH_WAIT_MS` 夹值）。
pub fn shutdown_bounded(timeout_ms: u64) {
    let state = take_state();
    let Some(state) = state else {
        return;
    };
    let wait_ms = timeout_ms.min(MAX_FLUSH_WAIT_MS);

    let provider = state.provider.clone();
    match provider.shutdown_with_timeout(Duration::from_millis(wait_ms)) {
        Ok(()) => {
            // 正常路径（A7）：先显式释放 provider（导出线程 / reqwest Client），
            // 再释放含 runtime 的全局态 —— runtime 必须活到所有导出 future 结束之后。
            drop(provider);
            if tokio::runtime::Handle::try_current().is_ok() {
                // 若处于异步 runtime 线程中直接 drop Runtime 会因阻塞等待 worker panic；
                // 移交独立 std 线程安全释放。
                std::thread::spawn(move || drop(state));
            } else {
                drop(state);
            }
        }
        Err(err) => {
            // 超时/失败路径（task-10）：**有意不释放** runtime。
            // 关闭中的 runtime 会与仍在退出中的后台导出线程竞争，触发
            // `Tokio ... being shutdown` panic；这里改为保留 runtime 至进程终止，仅记 warn。
            degraded_warn(
                &format!(
                    "otel::shutdown：provider 关闭超时或失败，导出 runtime 保留至进程终止：{err}"
                ),
                None,
            );
            std::mem::forget(state);
            drop(provider);
        }
    }
}

/// 把当前已入队 span 强制导出，等待至多 `timeout_ms` 毫秒。
///
/// 调用方：手动上传旁路与进程退出前的兜底 flush；测试用它取得确定性导出时序。
/// 语义：未调用过 [`init`] 时返回 `Err`；超时视为失败但仍返回（不 panic、不阻塞调用方）。
/// 失败语义：`Err(String)` 描述失败原因，调用方只记日志，不得冒泡到 turn 主链路。
pub fn force_flush(timeout_ms: u64) -> Result<(), String> {
    let state = current_state().ok_or_else(|| "otel::force_flush：尚未 init".to_string())?;
    flush_state(&state, timeout_ms)
}

/// 翻转全局上报开关。
///
/// 调用方：宿主设置读取/保存路径（启动时按持久化值回填 + 用户改开关即时生效）。
/// 语义：`false` 时不得分配 span、不得产生任何网络出口；`true` 时按采样率上报。
/// 幂等：并发/重复调用不得 panic，末态等于最后一次写入值。
pub fn set_reporting_enabled(enabled: bool) {
    REPORTING_ENABLED.store(enabled, Ordering::SeqCst);
}

/// 读取全局上报开关当前值。
///
/// 调用方：设置页回显与门禁判定的只读查询。
/// 语义：恒等于最近一次 [`set_reporting_enabled`] 的写入值（与是否 [`init`] **无关**，
/// 写入是纯内存操作）；进程启动缺省为 `false`。不做任何副作用。
pub fn is_reporting_enabled() -> bool {
    REPORTING_ENABLED.load(Ordering::SeqCst)
}

/// 是否允许采集 span 级属性（Wave-5 K4：成本属性零计算短路判定）。
///
/// 调用方：turn/provider 侧「先判门禁、再算价格」的成本接缝。
/// 语义：`false` ⇒ 调用方**不得**执行价格计算、不得附加任何 span 属性（零分配零计算）。
pub fn is_span_capture_enabled() -> bool {
    spans_allowed()
}

/// 成本值四舍五入到 6 位小数（与 [`crate::agent::cost::COST_PRECISION_DECIMALS`] 对齐）。
fn round_cost_usd(value: f64) -> f64 {
    (value * 1_000_000.0).round() / 1_000_000.0
}

/// 宿主设置桥接：把持久化的 `AppSettings.agent_trace_reporting` 同步到 OTel 门禁。
///
/// 调用方：Tauri 侧 `load_app_settings` / `save_app_settings`（D7：启动即按持久化值生效，
/// 保存即生效）。语义：等价于 [`set_reporting_enabled`]，仅为宿主提供语义明确的入口，
/// 便于静态断言「设置读写双路径都做了同步」。
pub fn set_agent_trace_reporting(enabled: bool) {
    set_reporting_enabled(enabled);
}

/// 导出链路诊断快照：供 Settings 页/诊断入口展示「trace 到底有没有在往外发」。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportDiagnostics {
    pub endpoint: String,
    pub auth_configured: bool, // config.headers 中存在鉴权 header（authorization）
    pub probe: bool,           // 本次是否探针自报（PONY_OTEL_PROBE 生效）
}

/// 读取当前导出链路诊断快照；未 [`init`] 时返回 `None`。
///
/// 状态取自现有全局 state（`lock_state()`）；`auth_configured` 用已有的
/// `is_sensitive_header` 判定；**绝不返回 token 真实内容**。
pub fn export_diagnostics() -> Option<ExportDiagnostics> {
    current_state().map(|state| {
        let auth_configured = state
            .config
            .headers
            .iter()
            .any(|(name, _)| is_sensitive_header(name));
        let probe = env_non_empty(ENV_PONY_OTEL_PROBE).is_some_and(|probe| {
            probe.eq_ignore_ascii_case("true")
                || probe == "1"
                || probe.eq_ignore_ascii_case("probe")
        });
        ExportDiagnostics {
            endpoint: state.config.endpoint.clone(),
            auth_configured,
            probe,
        }
    })
}

/// 回显当前生效配置。
///
/// 调用方：设置页/诊断页展示与配置透传断言；未 [`init`] 时返回 `None`。
/// 语义：返回的采样率等字段必须等于实际生效值（env 覆盖后的值，而非常量）。
pub fn config() -> Option<OtelConfig> {
    current_state().map(|state| {
        let mut config = state.config.clone();
        // F1：诊断/设置页回显**不得**返回鉴权材料；token 只用于导出通路。
        config.headers.retain(|(name, _)| !is_sensitive_header(name));
        config
    })
}

/// 按环境变量组装配置（无 settings 来源的兼容入口）。
///
/// 调用方：[`init`] 的宿主侧调用点（无持久化 settings 时，如 TUI/CLI 进程）。
/// 语义：等价于 [`OtelConfig::from_settings`]`(None)` ⇒ 优先级 `env > 内置默认`
/// （[`ENV_OTLP_ENDPOINT`] / [`ENV_SERVICE_NAME`] / [`ENV_SAMPLER_ARG`]，缺省端点为
/// [`DEFAULT_PUBLIC_ENDPOINT`]）；[`ENV_PONY_OTEL_TOKEN`] 存在时以 [`AUTH_HEADER_NAME`]
/// header 注入 headers。不读写全局状态、不发起网络 IO，可被单测直接调用。
pub fn from_env() -> OtelConfig {
    OtelConfig::from_settings(None)
}

/// 把 `PONY_OTEL_TOKEN` 组装成 `Authorization` 头的值。
///
/// 契约：集群公网入口的 collector 用 `bearertokenauth`（`scheme: Bearer`），只认
/// `Authorization: Bearer <token>`；裸 token 会被直接 401（2026-10-24 实测）。因此缺省补
/// `Bearer ` 前缀。若调用方已在 token 里自带 scheme（`bearer x` / `basic x`，大小写不敏感），
/// 原样透传，避免出现 `Bearer Bearer x`。除鉴权 scheme 外不解释 token 内容。
fn authorization_header_value(token: &str) -> String {
    let trimmed = token.trim();
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("bearer ") || lower.starts_with("basic ") {
        trimmed.to_string()
    } else {
        format!("Bearer {trimmed}")
    }
}

/// turn 级 span 守卫。
///
/// 契约：由 [`TurnScope::new`] 构造（外部不可字面量构造），生命周期覆盖一个 turn；
/// 必须显式调用 [`TurnScope::end`] 结束 span（本类型不实现 `Drop`）。
/// 全局开关关闭时，构造出的守卫等价空操作（零分配零导出）。
pub struct TurnScope {
    span: Option<opentelemetry_sdk::trace::Span>,
    trace_id: Option<String>,
}

impl TurnScope {
    /// 开启一个 turn 级 span 并返回守卫。
    ///
    /// 调用方：turn 终态路径；`session_id` / `turn_id` 须与本地 trace 记录一致，
    /// `sequence` 为该 turn 在会话内的序号。
    /// 语义：span 名为 [`SPAN_AGENT_TURN`]，属性由 [`TurnSpanInput`] 描述。
    /// 失败语义：任何情况下都不得 panic；开关关闭时返回空操作守卫。
    pub fn new(session_id: &str, turn_id: &str, sequence: u64) -> Self {
        let empty = Self {
            span: None,
            trace_id: None,
        };
        if !spans_allowed() {
            return empty;
        }
        let Some(state) = current_state() else {
            return empty;
        };

        // phase 由宿主在 turn 终态时补充；此处留空 ⇒ 属性映射不输出该键（禁止空串占位）。
        let attributes = to_key_values(turn_span_attributes(&TurnSpanInput {
            session_id: session_id.to_string(),
            user_id: None,
            turn_id: turn_id.to_string(),
            sequence,
            phase: String::new(),
        }));
        let mut span = state.tracer.build_with_context(
            opentelemetry::trace::SpanBuilder::from_name(SPAN_AGENT_TURN)
                .with_kind(opentelemetry::trace::SpanKind::Internal)
                .with_attributes(attributes),
            &opentelemetry::Context::new(),
        );
        let trace_id = sampled_trace_id(&mut span);

        Self {
            span: Some(span),
            trace_id,
        }
    }

    /// 读取该 turn 的 trace_id（十六进制字符串）。
    ///
    /// 调用方：日志面与 issue 上报的 `tags.trace_id` 回填。
    /// 语义：未采样、未 [`init`] 或守卫已结束时返回 `None`。
    pub fn trace_id(&self) -> Option<String> {
        self.trace_id.clone()
    }

    /// 结束 span 并释放守卫。
    ///
    /// 语义：消费 `self`，重复结束不可能发生；不得 panic。
    pub fn end(self) {
        if let Some(mut span) = self.span {
            span.end();
        }
    }
}

/// 手动上传的强制导出入口（旁路全局开关）。
///
/// 调用方：控制面 `upload_session_trace` 转调（用户点击上传按钮 = 逐次授权）。
/// 语义：把 `traces` 重放为 span 树并强制导出，返回成功导出的 turn 数；
/// **不受** [`is_reporting_enabled`] 约束，且**不得改变**该开关的末态（用后复原）。
/// 失败语义：导出失败返回 `Err(String)`；重复调用幂等（数量契约不变，允许产生多条 trace）。
pub fn force_export_session_trace(
    session_id: &str,
    traces: &[crate::agent::session::TurnTraceRecord],
) -> Result<usize, String> {
    let state = current_state()
        .ok_or_else(|| "otel::force_export_session_trace：尚未 init，无法导出".to_string())?;

    // A8：按批量管道上限分块重放，**每块 flush 成功后才计入已交付**；
    // 返回值是「实际完成 flush 的 turn 数」，不是入队条数。
    //
    // 总预算约束（Bounded Total Flush Budget）：
    // 整个会话导出的总 flush 预算严格钉死为 FORCE_EXPORT_FLUSH_TIMEOUT_MS（3000ms），
    // 跨 chunk 共享倒计时。进入每个 chunk 时若总预算已耗尽则立即返回 Err；
    // 每块传递的等待时间为剩余预算（remaining_ms），从结构上杜绝 N 个 chunk 各自等待
    // 3000ms 导致的 N×3000ms（如 2 块 6s）超时膨胀。
    let total_deadline = std::time::Instant::now()
        + Duration::from_millis(FORCE_EXPORT_FLUSH_TIMEOUT_MS);
    let mut delivered: usize = 0;
    let chunks: Vec<&[crate::agent::session::TurnTraceRecord]> =
        traces.chunks(FORCE_EXPORT_CHUNK_TURNS).collect();
    let total_chunks = chunks.len();

    for (chunk_idx, chunk) in chunks.iter().enumerate() {
        let remaining = total_deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            let skipped = total_chunks - chunk_idx;
            return Err(format!(
                "otel::force_export_session_trace：整体导出预算已耗尽（{FORCE_EXPORT_FLUSH_TIMEOUT_MS}ms），剩余 {skipped} 个 chunk 未发送"
            ));
        }
        let chunk_timeout_ms = (remaining.as_millis() as u64)
            .min(FORCE_EXPORT_FLUSH_TIMEOUT_MS)
            .max(1);

        for (offset, record) in chunk.iter().enumerate() {
            let record_session = record
                .session_id
                .as_deref()
                .filter(|value| !value.is_empty())
                .unwrap_or(session_id);
            let mut attributes = to_key_values(turn_span_attributes(&TurnSpanInput {
                session_id: record_session.to_string(),
                user_id: None,
                turn_id: record.turn_id.clone(),
                sequence: record.sequence.unwrap_or(offset as u64),
                phase: record.phase.clone(),
            }));
            // F6：逐 span 打「用户逐次授权」标记，只有这些 span 在关闭态被采样与放行。
            attributes.push(KeyValue::new(ATTR_PONY_OTEL_FORCED, true));

            let mut span = state.tracer.build_with_context(
                opentelemetry::trace::SpanBuilder::from_name(SPAN_AGENT_TURN)
                    .with_kind(opentelemetry::trace::SpanKind::Internal)
                    .with_attributes(attributes),
                &opentelemetry::Context::new(),
            );
            span.end();
        }
        flush_state(&state, chunk_timeout_ms)
            .map_err(|message| format!("otel::force_export_session_trace：{message}"))?;
        delivered += chunk.len();
    }

    Ok(delivered)
}

/// turn 属性映射（纯函数，无网络、无全局态）。
///
/// 调用方：[`TurnScope::new`] 的 span 属性填充，以及单测直接断言属性全集。
/// 语义：按 [`ATTR_LANGFUSE_SESSION_ID`] / [`ATTR_LANGFUSE_TRACE_NAME`] /
/// [`ATTR_GEN_AI_OPERATION_NAME`]（[`OPERATION_AGENT_TURN`]）/ [`ATTR_AGENT_TURN_ID`] /
/// [`ATTR_AGENT_TURN_SEQUENCE`] / [`ATTR_AGENT_TURN_PHASE`] 输出，`user_id` 非空时
/// 才输出 [`ATTR_LANGFUSE_USER_ID`]；**缺失值不输出键**（禁止空串占位）。
pub fn turn_span_attributes(input: &TurnSpanInput) -> Vec<(String, AttrValue)> {
    let mut attributes: Vec<(String, AttrValue)> = Vec::with_capacity(7);

    push_str(&mut attributes, ATTR_LANGFUSE_SESSION_ID, &input.session_id);
    // trace 名取 turn 级展示名：优先 turn_id，回落到 session_id（两者皆空则不输出）。
    let trace_name = if input.turn_id.is_empty() {
        input.session_id.as_str()
    } else {
        input.turn_id.as_str()
    };
    push_str(&mut attributes, ATTR_LANGFUSE_TRACE_NAME, trace_name);
    push_str(
        &mut attributes,
        ATTR_GEN_AI_OPERATION_NAME,
        OPERATION_AGENT_TURN,
    );
    push_str(&mut attributes, ATTR_AGENT_TURN_ID, &input.turn_id);
    attributes.push((
        ATTR_AGENT_TURN_SEQUENCE.to_string(),
        AttrValue::I64(input.sequence as i64),
    ));
    push_str(&mut attributes, ATTR_AGENT_TURN_PHASE, &input.phase);
    if let Some(user_id) = input.user_id.as_deref() {
        push_str(&mut attributes, ATTR_LANGFUSE_USER_ID, user_id);
    }

    attributes
}

/// LLM 属性映射（纯函数，无网络、无全局态）。
///
/// 调用方：LLM 子 span 的属性填充与单测断言。
/// 语义：恒输出 [`ATTR_GEN_AI_SYSTEM`] 与 [`ATTR_GEN_AI_OPERATION_NAME`]；
/// `model` 非空时同时输出 [`ATTR_GEN_AI_REQUEST_MODEL`] 与
/// [`ATTR_LANGFUSE_OBSERVATION_MODEL_NAME`]（同值双写）；token 字段各自非空才输出。
pub fn llm_span_attributes(input: &LlmSpanInput) -> Vec<(String, AttrValue)> {
    let mut attributes: Vec<(String, AttrValue)> = Vec::with_capacity(6);

    push_str(&mut attributes, ATTR_GEN_AI_SYSTEM, &input.provider_kind);
    if let Some(model) = input.model.as_deref() {
        push_str(&mut attributes, ATTR_GEN_AI_REQUEST_MODEL, model);
        push_str(
            &mut attributes,
            ATTR_LANGFUSE_OBSERVATION_MODEL_NAME,
            model,
        );
    }
    if let Some(input_tokens) = input.input_tokens {
        attributes.push((
            ATTR_GEN_AI_USAGE_INPUT_TOKENS.to_string(),
            AttrValue::I64(input_tokens as i64),
        ));
    }
    if let Some(output_tokens) = input.output_tokens {
        attributes.push((
            ATTR_GEN_AI_USAGE_OUTPUT_TOKENS.to_string(),
            AttrValue::I64(output_tokens as i64),
        ));
    }
    push_str(
        &mut attributes,
        ATTR_GEN_AI_OPERATION_NAME,
        &input.operation,
    );

    attributes
}

/// 「细分用量」属性映射（纯函数，无网络、无全局态；wave-51 C3 契约桩）。
///
/// 调用方：[`LlmSpanGuard::record_usage_details`]（`llm.chat` 子 span）与根 `agent.turn`
/// 聚合路径**共用同一份**映射 ⇒ 键名/取值/出现条件只在一处定义，禁止两条口径漂移。
/// 语义：有值才输出（`None` ⇒ 不输出该键）；`Some(0)` 是有效取值、必须输出 0；
/// 值为 `i64`，`u64 -> i64` **安全饱和**（超出 `i64::MAX` 钳到 `i64::MAX`，禁止 `as` 直转成负数）。
/// 输出键：[`ATTR_GEN_AI_USAGE_REASONING_TOKENS`]、[`ATTR_GEN_AI_USAGE_CACHED_INPUT_TOKENS`]。
///
/// [STUB wave-51] 空桩：仅冻结签名与语义，实现归 Executor。
pub fn llm_usage_detail_attributes(
    reasoning_tokens: Option<u64>,
    cached_input_tokens: Option<u64>,
) -> Vec<(String, AttrValue)> {
    let _ = (reasoning_tokens, cached_input_tokens);
    unimplemented!("wave-51 C3 契约桩：llm_usage_detail_attributes 待实现")
}

/// 工具属性映射（纯函数，无网络、无全局态）。
///
/// 调用方：工具子 span 的属性填充与单测断言。
/// 语义：恒输出 [`ATTR_GEN_AI_TOOL_NAME`]、[`ATTR_LANGFUSE_OBSERVATION_TYPE`]
/// （[`OBSERVATION_TYPE_TOOL`]）、[`ATTR_AGENT_TOOL_STATUS`]；`duration_ms` 与
/// `error` 各自非空才输出。
pub fn tool_span_attributes(input: &ToolSpanInput) -> Vec<(String, AttrValue)> {
    let mut attributes: Vec<(String, AttrValue)> = Vec::with_capacity(5);

    push_str(&mut attributes, ATTR_GEN_AI_TOOL_NAME, &input.tool_name);
    push_str(
        &mut attributes,
        ATTR_LANGFUSE_OBSERVATION_TYPE,
        OBSERVATION_TYPE_TOOL,
    );
    push_str(&mut attributes, ATTR_AGENT_TOOL_STATUS, &input.status);
    if let Some(duration_ms) = input.duration_ms {
        attributes.push((
            ATTR_AGENT_TOOL_DURATION_MS.to_string(),
            AttrValue::I64(duration_ms as i64),
        ));
    }
    if let Some(error) = input.error.as_deref() {
        push_str(&mut attributes, ATTR_AGENT_TOOL_ERROR, error);
    }

    attributes
}

// ---------------------------------------------------------------- W3C Trace Context 契约辅助与空桩

/// W3C traceparent 上下文结构
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct W3cTraceContext {
    pub version: String,
    pub trace_id: String,
    pub parent_id: String,
    pub trace_flags: String,
}

impl W3cTraceContext {
    /// 格式化为标准 W3C traceparent 头部字符串 (例如 00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01)
    pub fn to_header_value(&self) -> String {
        format!("{}-{}-{}-{}", self.version, self.trace_id, self.parent_id, self.trace_flags)
    }
}

/// 将 traceparent 注入到 HTTP 请求头集合中（空桩与契约接口签名）。
///
/// 契约约定：
/// - 若当前上下文存在有效 trace_id 与 span_id，则格式化并设置 `traceparent` 请求头。
/// - 若不存在或上报门禁关闭，保持空桩或透传既有 headers。
pub fn inject_traceparent_header<H>(headers: &mut H, _context: Option<&W3cTraceContext>)
where
    H: AsMut<reqwest::header::HeaderMap>,
{
    if let Some(ctx) = _context {
        if let Ok(val) = reqwest::header::HeaderValue::from_str(&ctx.to_header_value()) {
            headers.as_mut().insert("traceparent", val);
        }
    }
}

// ------------------------------------------- 当前活跃 turn 上下文查询（Wave-4 C3/C4 契约）
//
// 两套 id 语义**必须**严格区分（Wave-4.1 L3-A B1 修复）：
// - [`current_trace_id`] / [`current_span_id`] / [`current_traceparent`]：**本地相关性** id。
//   门禁关闭或 provider 未装配时由轮内本地上下文提供，只用于日志关联与出网 traceparent；
//   这类 id **不保证**可在 Tempo/Langfuse 检索到。
// - [`exported_trace_id`]：**导出态** id。仅当本轮存在真实 SDK root span 且已采样时返回；
//   凡是会让用户/第三方「按 id 去查链路」的下游（PonySentry issue、会话 trace DTO）
//   必须只使用它 —— 否则会出现「有 id 但查不到」的误导。

/// 读取当前活跃 turn 的 trace_id（32 位小写十六进制，**本地相关性**语义）。
///
/// 调用方：结构化日志（C4）与出网 traceparent 组装。
/// 语义：无活跃 turn（或上下文非法）时返回 `None`——**绝不**返回全 0 / dummy 占位值。
/// 注意：本 id 可能是门禁关闭时的轮内本地 id，**不得**用于「按 id 检索上游链路」的场景，
/// 那类场景请用 [`exported_trace_id`]。
pub fn current_trace_id() -> Option<String> {
    let span_context = active_turn_span_context()?;
    if !span_context.is_valid() {
        return None;
    }
    let hex = span_context.trace_id().to_string();
    if is_lower_hex(&hex, 32) {
        Some(hex)
    } else {
        None
    }
}

/// 读取当前 turn 的**导出态** trace_id；不满足导出条件时一律 `None`。
///
/// 调用方：PonySentry issue 的 `trace_id` 回填（C2）、会话 trace DTO（前端按 id 跳转 Tempo）。
/// 语义（三条同时满足才返回）：
/// 1. 存在**真实 SDK root span** 建立的登记（本地降级上下文不算）；
/// 2. `span_context.is_valid()`；
/// 3. `span_context.is_sampled()`（未被采样 ⇒ 上游无此 trace，返回 id 只会误导）。
/// 门禁关闭 / 未 init / 未采样 ⇒ `None`，**严禁**回落到 [`current_trace_id`]。
pub fn exported_trace_id() -> Option<String> {
    let entry = current_active_turn()?;
    if !entry.exported || !entry.span_context.is_valid() || !entry.span_context.is_sampled() {
        return None;
    }
    let hex = entry.span_context.trace_id().to_string();
    if is_lower_hex(&hex, 32) {
        Some(hex)
    } else {
        None
    }
}

/// 读取当前活跃 turn 的 span_id（16 位小写十六进制）。
///
/// 调用方：出网 `traceparent` 组装与诊断透出。语义同 [`current_trace_id`]（无活跃 turn ⇒ `None`）。
pub fn current_span_id() -> Option<String> {
    let span_context = active_turn_span_context()?;
    if !span_context.is_valid() {
        return None;
    }
    let hex = span_context.span_id().to_string();
    if is_lower_hex(&hex, 16) {
        Some(hex)
    } else {
        None
    }
}

/// 组装当前活跃 turn 的 W3C `traceparent` 头值：`00-<trace_id>-<span_id>-<flags>`。
///
/// 调用方：出网注入（[`with_current_traceparent`]）与诊断断言。
/// 语义：无活跃 turn / 任一 id 非法 / 未采样 ⇒ `None`（fail-closed：宁可不注入脏头）。
/// 采样位固定取 `01`：只有可用的（已采样或本轮本地登记的）上下文才会被注入。
pub fn current_traceparent() -> Option<String> {
    let span_context = active_turn_span_context()?;
    if !span_context.is_valid() || !span_context.is_sampled() {
        return None;
    }
    let trace_id = span_context.trace_id().to_string();
    let span_id = span_context.span_id().to_string();
    if !is_lower_hex(&trace_id, 32) || !is_lower_hex(&span_id, 16) {
        return None;
    }
    Some(format!("00-{trace_id}-{span_id}-01"))
}

/// 给 outbound 请求注入 `traceparent`（C3 单点注入入口）。
///
/// 调用方：**所有** outbound `reqwest` 出网点（provider 模型调用、PonySentry ingest、
/// 模型目录查询、Exa 搜索、web_fetch）。语义：无活跃 span 时**原样返回**请求
/// （零分配、不写头）；有活跃 span 时写入 `traceparent` 头；id 非法时 fail-closed（不注入）。
/// 任何情况下不 panic。
///
/// 防重复（Wave-4.1 L3-A #8）：`RequestBuilder::headers(HeaderMap)` 走 reqwest 内部的
/// `replace_headers`（`Entry::insert` 语义），因此**同名学生头会被替换而非追加** ——
/// 调用方自带或重复调用都不会产生两个 `traceparent`（W3C 要求每请求至多一个）。
pub fn with_current_traceparent(request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    let Some(headers) = traceparent_header_map() else {
        return request;
    };
    request.headers(headers)
}

/// 组装只含单个 `traceparent` 的 header map；无活跃 span / 非法值 ⇒ `None`。
///
/// 调用方：[`with_current_traceparent`] 与 PonySentry worker（已持有 traceparent 字符串时）。
pub fn traceparent_header_map() -> Option<reqwest::header::HeaderMap> {
    let traceparent = current_traceparent()?;
    traceparent_header_map_from(&traceparent)
}

/// 由既有 traceparent 字符串组装单头 map（供跨线程传递 traceparent 的调用点复用）。
pub fn traceparent_header_map_from(traceparent: &str) -> Option<reqwest::header::HeaderMap> {
    let value = reqwest::header::HeaderValue::from_str(traceparent).ok()?;
    let mut headers = reqwest::header::HeaderMap::with_capacity(1);
    headers.insert(reqwest::header::HeaderName::from_static("traceparent"), value);
    Some(headers)
}

/// 是否为指定长度的「小写十六进制」串（W3C trace-id / span-id 形态校验）。
fn is_lower_hex(value: &str, expected_len: usize) -> bool {
    value.len() == expected_len
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// 读取当前活跃 turn 的 span 上下文；无活跃 turn 时 `None`。
fn active_turn_span_context() -> Option<opentelemetry::trace::SpanContext> {
    current_active_turn().map(|entry| entry.span_context)
}

/// 为「不导出但仍需关联」的 turn 生成本地 W3C 上下文（随机、合法、已采样）。
///
/// 语义：门禁关闭或 provider 未装配时，turn 仍需要一个稳定的关联标识，供日志
/// `trace_id`（C4）与出网 `traceparent`（C3）使用；该上下文**只存在于进程内**，
/// 不产生任何 span 分配与网络出口（导出链路仍由门禁严格关闭）。
fn local_span_context() -> opentelemetry::trace::SpanContext {
    let trace_bytes = uuid::Uuid::new_v4().into_bytes();
    let span_bytes = uuid::Uuid::new_v4().into_bytes();
    let mut span_id_bytes = [0u8; 8];
    span_id_bytes.copy_from_slice(&span_bytes[..8]);
    // W3C 要求 id 非全 0；极低概率命中时按位或 1 兜底（保持随机性，避免出现无效上下文）。
    let mut trace_id_bytes = trace_bytes;
    if trace_id_bytes.iter().all(|byte| *byte == 0) {
        trace_id_bytes[15] = 1;
    }
    if span_id_bytes.iter().all(|byte| *byte == 0) {
        span_id_bytes[7] = 1;
    }
    opentelemetry::trace::SpanContext::new(
        opentelemetry::trace::TraceId::from_bytes(trace_id_bytes),
        opentelemetry::trace::SpanId::from_bytes(span_id_bytes),
        opentelemetry::trace::TraceFlags::SAMPLED,
        false,
        opentelemetry::trace::TraceState::default(),
    )
}

// ------------------------------------------------- turn / LLM / 工具 span 装配面（挂载切片）
//
// 运行时装配点（runtime/turn_*.rs、turn_flow.rs 的 provider 调用缝、工具活动记录点）
// 只使用这一组**中性**入口：不感知任何 OTel 类型；门禁关闭时一律退化为零分配空操作。

/// 开始一棵 turn trace，返回在 turn 生命周期内持有的守卫。
///
/// 语义：创建 `agent.turn` root span，并把其上下文登记为「当前活动 turn」，
/// 之后 [`start_llm_span`] / [`record_tool_call_span`] 会自动挂到该 turn 之下；
/// 守卫 **Drop 即结束 span**（turn 有多个出口，避免漏 `end`）。
/// 失败语义：未 [`init`]、门禁关闭、或 `session_id`/`turn_id` 缺失（空串）时返回空操作守卫
/// （缺失即跳过，**禁止**用占位值伪造 session）；任何情况下不 panic。
///
/// Wave-4 C3/C4：门禁关闭或 provider 未装配时，本函数仍为该 turn 登记一个**进程内本地**
/// W3C 上下文（随机 trace_id/span_id、已采样、零导出零网络），使 [`current_trace_id`] /
/// [`current_traceparent`] 在活跃 turn 内可用（日志 trace_id 绑定与出网级联不因「不导出」失效）。
pub fn begin_turn_trace(session_id: &str, turn_id: &str, sequence: u64) -> TurnTraceGuard {
    let empty = TurnTraceGuard {
        span: None,
        slot: None,
        phase: None,
        trace_id: None,
        turn_speed: None,
        turn_cost: None,
    };
    // 契约：session_id / turn_id 缺失（空串）⇒ **不创建** turn span。
    // 绝不用占位值伪造会话——那会在 Langfuse 侧造出不存在的 session，污染数据。
    if session_id.trim().is_empty() || turn_id.trim().is_empty() {
        return empty;
    }
    if spans_allowed() {
        if let Some(state) = current_state() {
            let user_id = env_non_empty(ENV_PONY_OTEL_USER_ID);
            let attributes = to_key_values(turn_span_attributes(&TurnSpanInput {
                session_id: session_id.to_string(),
                user_id,
                turn_id: turn_id.to_string(),
                sequence,
                phase: String::new(),
            }));
            let mut span = state.tracer.build_with_context(
                opentelemetry::trace::SpanBuilder::from_name(SPAN_AGENT_TURN)
                    .with_kind(opentelemetry::trace::SpanKind::Internal)
                    .with_attributes(attributes),
                &opentelemetry::Context::new(),
            );
            let span_context = span.span_context().clone();
            let trace_id = sampled_trace_id(&mut span);
            // 只有真实建立且被采样的 root span 才算「导出态」（B1/B1b）。
            let exported = trace_id.is_some();
            if let Some(trace_id) = trace_id.as_deref() {
                // Wave-6：登记 turn 级导出态 trace_id，供跨上下文失败上报精确回查。
                register_turn_exported_trace_id(turn_id, trace_id);
            }
            let slot = register_active_turn(turn_id, session_id, span_context, exported);

            return TurnTraceGuard {
                span: Some(span),
                slot,
                phase: None,
                trace_id: None,
                turn_speed: None,
                turn_cost: None,
            }
            .with_trace_id(trace_id);
        }
    }

    // 降级通路：本地关联上下文（不导出、不联网），仅承担本轮内的 trace 关联标识。
    // 契约：`exported = false` ⇒ 永不作子 span 的 parent、永不作 issue/前端跳转的 trace_id。
    let span_context = local_span_context();
    let trace_id = span_context.trace_id().to_string();
    let slot = register_active_turn(turn_id, session_id, span_context, false);
    TurnTraceGuard {
        span: None,
        slot,
        phase: None,
        trace_id: Some(trace_id),
        turn_speed: None,
        turn_cost: None,
    }
}

/// turn trace 守卫（`Drop` 时补 phase 属性并结束 span）。
pub struct TurnTraceGuard {
    span: Option<opentelemetry_sdk::trace::Span>,
    /// 该 turn 的登记归属（task id 或线程本地栈）；Drop 时精确摘除自己那一条。
    slot: Option<ActiveTurnSlot>,
    phase: Option<String>,
    trace_id: Option<String>,
    turn_speed: Option<f64>,
    /// Wave-5 K4：整轮聚合成本（USD）；`None` ⇒ 不输出 `agent.turn.cost.usd`。
    turn_cost: Option<f64>,
}

impl TurnTraceGuard {
    fn with_trace_id(mut self, trace_id: Option<String>) -> Self {
        self.trace_id = trace_id;
        self
    }

    /// 记录 turn 综合生成速度（token/s）。
    pub fn record_turn_speed(&mut self, speed: Option<f64>) {
        self.turn_speed = speed.filter(|s| s.is_finite() && *s > 0.0);
    }

    /// 记录 turn 终态相位（`completed` / `failed` / `cancelled` …）。
    ///
    /// 语义：在守卫 Drop 前调用；空串视为「无相位」→ 不输出该键。
    pub fn set_phase(&mut self, phase: &str) {
        if phase.is_empty() {
            self.phase = None;
        } else {
            // F2：phase 同样过脱敏（防御调用方传入不可信内容）。
            self.phase = Some(crate::agent::ponysentry::sanitize(phase));
        }
    }

    /// 记录整轮 turn 的聚合成本（USD）。
    ///
    /// 调用方：turn 终态路径（Wave-5 K4）。语义：`None` / 非有限 / 负值 ⇒ 视为「无有效计价」，
    /// **不输出** `agent.turn.cost.usd`；门禁关闭时守卫为空操作（零导出），此处只做赋值。
    /// 注意：未显式记录时，[`TurnTraceGuard::drop`] 会回落到本轮内各 `llm.chat` span 的累计成本。
    pub fn record_turn_cost(&mut self, cost: Option<f64>) {
        self.turn_cost = cost.filter(|value| value.is_finite() && *value >= 0.0);
    }

    /// 读取该 turn 的 trace_id（十六进制字符串）；未采样/门禁关闭/未 init 时 `None`。
    ///
    /// 注意：门禁关闭时这里返回的是**本地相关性** id（日志/traceparent 用），
    /// 若下游要「按 id 去上游检索链路」，必须改用 [`TurnTraceGuard::exported_trace_id`]。
    pub fn trace_id(&self) -> Option<String> {
        self.trace_id.clone()
    }

    /// 读取该 turn 的**导出态** trace_id：仅真实 SDK span 且已采样时返回。
    ///
    /// 调用方：会话 trace DTO（前端跳转 Tempo/Langfuse）与任何「可检索」语义的下游。
    /// 语义：本地降级上下文（`span == None`）或未采样（`trace_id == None`）一律 `None`。
    pub fn exported_trace_id(&self) -> Option<String> {
        if self.span.is_some() {
            self.trace_id.clone()
        } else {
            None
        }
    }
}

impl Drop for TurnTraceGuard {
    fn drop(&mut self) {
        // PA-154：先取走本轮错误聚合（取走即清零 ⇒ 跨 turn 不污染）；无活动 turn 时无副作用。
        let aggregated_error = take_active_turn_error();
        let phase_failed = self.phase.as_deref() == Some("failed");
        if let Some(phase) = self.phase.take() {
            if let Some(span) = self.span.as_mut() {
                if phase == "failed" {
                    span.set_status(opentelemetry::trace::Status::error(phase.clone()));
                }
                span.set_attribute(KeyValue::new(ATTR_AGENT_TURN_PHASE, phase));
            }
        }
        // PA-154：phase 语义零变更——phase == "failed" 保持既有 `Status::error(phase)`，
        // **不写** `turn_error` 文案但仍在根 span 标出终态聚合键；
        // 其余相位由聚合到的子 span 错误承接 `turn_error: <reason>`。
        if let Some(span) = self.span.as_mut() {
            match (phase_failed, aggregated_error) {
                (true, _) => {
                    span.set_attribute(KeyValue::new(ATTR_AGENT_TURN_OUTCOME, "error"));
                }
                (false, Some(reason)) => {
                    span.set_status(opentelemetry::trace::Status::error(
                        turn_error_status_message(&reason),
                    ));
                    span.set_attribute(KeyValue::new(ATTR_AGENT_TURN_OUTCOME, "error"));
                }
                (false, None) => {}
            }
        }
        if let Some(speed) = self.turn_speed.take() {
            if let Some(span) = self.span.as_mut() {
                span.set_attribute(KeyValue::new(
                    ATTR_AGENT_TURN_SPEED_TOKENS_PER_SECOND,
                    speed,
                ));
            }
        }
        // Wave-5 K4：整轮聚合成本——显式记录优先，否则回落到本轮 llm.chat 累计值；
        // 无有效计价时**不输出该键**（严禁写 0.0）。
        let accumulated_cost = take_active_turn_cost();
        let turn_cost = self.turn_cost.take().or_else(|| {
            if accumulated_cost > 0.0 && accumulated_cost.is_finite() {
                Some(round_cost_usd(accumulated_cost))
            } else {
                None
            }
        });
        // P2：整轮聚合 token 用量（各 llm.chat 子 span 累计值）。
        let (turn_input_tokens, turn_output_tokens) = take_active_turn_usage();
        if let Some(input_tokens) = turn_input_tokens {
            if let Some(span) = self.span.as_mut() {
                span.set_attribute(KeyValue::new(
                    ATTR_GEN_AI_USAGE_INPUT_TOKENS,
                    input_tokens as i64,
                ));
            }
        }
        if let Some(output_tokens) = turn_output_tokens {
            if let Some(span) = self.span.as_mut() {
                span.set_attribute(KeyValue::new(
                    ATTR_GEN_AI_USAGE_OUTPUT_TOKENS,
                    output_tokens as i64,
                ));
            }
        }
        if let Some(cost) = turn_cost {
            if let Some(span) = self.span.as_mut() {
                span.set_attribute(KeyValue::new(ATTR_AGENT_TURN_COST_USD, cost));
                span.set_attribute(KeyValue::new(ATTR_GEN_AI_COST_USD, cost));
            }
        }
        if let Some(mut span) = self.span.take() {
            span.end();
        }
        unregister_active_turn(self.slot.take());
    }
}

/// 开一个 `llm.chat` 子 span（父 = 当前活动 turn），返回守卫（`Drop` 即结束）。
///
/// 语义：属性按 [`llm_span_attributes`] 生成；token 用量在响应侧拿到后调用
/// [`LlmSpanGuard::record_usage`] 补写（缺失则不输出该键）。
/// 失败语义：门禁关闭/未 init 时返回空操作守卫，不 panic。
pub fn start_llm_span(provider_kind: &str, model: Option<&str>, operation: &str) -> LlmSpanGuard {
    let empty = LlmSpanGuard {
        span: None,
        input_tokens: None,
        output_tokens: None,
        speed: None,
        context_window: None,
        context_ratio: None,
        cost_usd: None,
    };
    if !spans_allowed() {
        return empty;
    }
    let mut attributes = to_key_values(llm_span_attributes(&LlmSpanInput {
        provider_kind: provider_kind.to_string(),
        model: model.map(|value| value.to_string()),
        input_tokens: None,
        output_tokens: None,
        operation: operation.to_string(),
    }));
    // Wave-6 K4.1：子 span 下沉会话归属（无活跃 turn / session 为空 ⇒ 不输出该键）。
    push_active_session_attribute(&mut attributes);
    let Some(span) = start_child_span(SPAN_LLM_CHAT, attributes) else {
        return empty;
    };
    LlmSpanGuard {
        span: Some(span),
        input_tokens: None,
        output_tokens: None,
        speed: None,
        context_window: None,
        context_ratio: None,
        cost_usd: None,
    }
}

/// `llm.chat` 子 span 守卫（`Drop` 时补 token 用量并结束 span）。
pub struct LlmSpanGuard {
    span: Option<opentelemetry_sdk::trace::Span>,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    speed: Option<f64>,
    context_window: Option<u64>,
    context_ratio: Option<f64>,
    /// Wave-5 K4：单次调用成本（USD）；`None` ⇒ 不输出 `gen_ai.cost.usd`。
    cost_usd: Option<f64>,
}

impl LlmSpanGuard {
    /// 记录响应侧 token 用量（`None` 表示缺失 → 不输出该键）。
    pub fn record_usage(&mut self, input_tokens: Option<u64>, output_tokens: Option<u64>) {
        self.input_tokens = input_tokens;
        self.output_tokens = output_tokens;
        if self.span.is_some() {
            accumulate_active_turn_usage(input_tokens, output_tokens);
        }
    }

    /// 记录思考 token 与缓存命中输入 token（wave-51 C3 契约桩）。
    ///
    /// 语义：**有值才写** —— `None` ⇒ **不得输出**对应键
    /// （[`ATTR_GEN_AI_USAGE_REASONING_TOKENS`] / [`ATTR_GEN_AI_USAGE_CACHED_INPUT_TOKENS`]）；
    /// `Some(0)` 是**有效取值**，必须原样输出 0（不得当作缺失丢弃）。
    /// 类型契约：属性值为 `i64`，`u64 -> i64` 必须**安全饱和**
    /// （超出 `i64::MAX` 钳到 `i64::MAX`），禁止 `as` 直转产生负数。
    /// 复用契约：属性映射必须复用纯函数 [`llm_usage_detail_attributes`]（与根 `agent.turn`
    /// 聚合同一份口径）；调用方负责**协议门控**缓存键（见契约 PA51-C3-T6）。
    /// 门禁关闭时守卫为空操作（零导出）。
    ///
    /// [STUB wave-51] 空桩：仅冻结签名与语义，实现归 Executor。
    pub fn record_usage_details(
        &mut self,
        reasoning_tokens: Option<u64>,
        cached_input_tokens: Option<u64>,
    ) {
        let _ = (reasoning_tokens, cached_input_tokens);
        unimplemented!("wave-51 C3 契约桩：LlmSpanGuard::record_usage_details 待实现")
    }

    /// 记录单次调用成本（USD）。
    ///
    /// 调用方：provider 调用返回真实 usage 之后（Wave-5 K4）。
    /// 语义：`None` / 非有限 / 负值 ⇒ 不输出 `gen_ai.cost.usd`；门禁关闭时守卫为空操作
    /// （零导出），且调用方应先用 [`is_span_capture_enabled`] 短路掉价格计算。
    /// 同时把该成本累加到当前 turn，供 `agent.turn.cost.usd` 聚合。
    pub fn record_cost(&mut self, cost: Option<f64>) {
        let validated = cost.filter(|value| value.is_finite() && *value >= 0.0);
        self.cost_usd = validated;
        if self.span.is_some() {
            if let Some(cost) = validated {
                accumulate_active_turn_cost(cost);
            }
        }
    }

    /// 记录单次调用生成速度与上下文用量诊断属性。
    pub fn record_speed_and_context(
        &mut self,
        speed: Option<f64>,
        context_window: Option<u64>,
    ) {
        self.speed = speed.filter(|s| s.is_finite() && *s > 0.0);
        self.context_window = context_window.filter(|w| *w > 0);
        if let (Some(input), Some(window)) = (self.input_tokens, self.context_window) {
            if window > 0 {
                self.context_ratio = Some((input as f64) / (window as f64));
            }
        }
    }
}

impl Drop for LlmSpanGuard {
    fn drop(&mut self) {
        if let Some(span) = self.span.as_mut() {
            if let Some(input_tokens) = self.input_tokens {
                span.set_attribute(KeyValue::new(
                    ATTR_GEN_AI_USAGE_INPUT_TOKENS,
                    input_tokens as i64,
                ));
            }
            if let Some(output_tokens) = self.output_tokens {
                span.set_attribute(KeyValue::new(
                    ATTR_GEN_AI_USAGE_OUTPUT_TOKENS,
                    output_tokens as i64,
                ));
            }
            if let Some(speed) = self.speed {
                span.set_attribute(KeyValue::new(
                    ATTR_GEN_AI_SPEED_TOKENS_PER_SECOND,
                    speed,
                ));
            }
            if let Some(context_window) = self.context_window {
                span.set_attribute(KeyValue::new(
                    ATTR_GEN_AI_USAGE_CONTEXT_WINDOW,
                    context_window as i64,
                ));
            }
            if let Some(context_ratio) = self.context_ratio {
                span.set_attribute(KeyValue::new(
                    ATTR_GEN_AI_USAGE_CONTEXT_RATIO,
                    context_ratio,
                ));
            }
            if let Some(cost) = self.cost_usd {
                span.set_attribute(KeyValue::new(ATTR_GEN_AI_COST_USD, cost));
            }
            span.end();
        }
    }
}

/// 打开治理决策 span（`governance.decide`），返回守卫（Drop 即结束）。
///
/// 契约 G5.1：**只**在宏观决策调度入口使用；纯函数内部严禁加 span。
/// 门禁关闭/无 provider 时返回空操作守卫（零分配零导出）。
pub fn start_governance_decide_span() -> GovernanceDecideGuard {
    if !spans_allowed() {
        return GovernanceDecideGuard { span: None };
    }
    let span = start_child_span(
        SPAN_GOVERNANCE_DECIDE,
        vec![KeyValue::new(
            ATTR_GEN_AI_OPERATION_NAME,
            "governance.decide",
        )],
    );
    GovernanceDecideGuard { span }
}

/// 治理决策 span 守卫；只允许写入 G5 白名单属性。
pub struct GovernanceDecideGuard {
    span: Option<opentelemetry_sdk::trace::Span>,
}

impl GovernanceDecideGuard {
    /// 记录决策判定结果（白名单属性；缺失值不输出键）。
    ///
    /// 参数：窗口标签、决策标签、原因标签、策略版本、未计价 turn 数。
    pub fn record_verdict(
        &mut self,
        window: &str,
        decision: &str,
        reason: Option<&str>,
        policy_version: &str,
        unpriced_turns: u64,
    ) {
        let Some(span) = self.span.as_mut() else {
            return;
        };
        if !window.trim().is_empty() {
            span.set_attribute(KeyValue::new(ATTR_GOVERNANCE_WINDOW, window.to_string()));
        }
        if !decision.trim().is_empty() {
            span.set_attribute(KeyValue::new(ATTR_GOVERNANCE_DECISION, decision.to_string()));
        }
        if let Some(reason) = reason.filter(|value| !value.trim().is_empty()) {
            span.set_attribute(KeyValue::new(ATTR_GOVERNANCE_REASON, reason.to_string()));
        }
        if !policy_version.trim().is_empty() {
            span.set_attribute(KeyValue::new(
                ATTR_GOVERNANCE_POLICY_VERSION,
                policy_version.to_string(),
            ));
        }
        span.set_attribute(KeyValue::new(
            ATTR_GOVERNANCE_UNPRICED_TURNS,
            unpriced_turns as i64,
        ));
        if decision == "block" {
            span.set_status(opentelemetry::trace::Status::error("quota_blocked".to_string()));
            // PA-154/F6：治理 block 除子 span 置错外，同时向活动 turn 聚合（根 span 承接）。
            mark_active_turn_error(TURN_ERROR_PREFIX_QUOTA_BLOCKED);
        }
    }
}

impl Drop for GovernanceDecideGuard {
    fn drop(&mut self) {
        if let Some(mut span) = self.span.take() {
            span.end();
        }
    }
}

/// 记一个 `tool.call` 子 span（一次工具执行 = 一个 span，创建即结束）。
///
/// 调用方：工具活动落库点（`TurnToolActivity` 生成处）。门禁关闭时为空操作。
pub fn record_tool_call_span(input: &ToolSpanInput) {
    if !spans_allowed() {
        return;
    }
    let mut attributes = to_key_values(tool_span_attributes(input));
    // Wave-6 K4.1：tool.call 同样携带会话归属。
    push_active_session_attribute(&mut attributes);
    // F5 失败谓词（`failed` / `error` / 带 error 摘要）：同一判定既写子 span 状态，
    // 也向活动 turn 聚合（PA-154：根 span 承接失败，否则按根状态告警全漏）。
    let failed = input.status == "failed" || input.status == "error" || input.error.is_some();
    if failed {
        mark_active_turn_error(TURN_ERROR_PREFIX_TOOL_FAILED);
    }
    // T11：span 必须覆盖**真实执行区间**。此前 `start_child_span` + 立即 `end()` 只覆盖
    // 登记动作（实测：829ms 的工具被导出成 0.002ms 的 span，瀑布图上工具耗时不可见，
    // 真值只存在于属性 `agent.tool.duration_ms`）。用 duration 反推起点并 backdate。
    let (started_at, ended_at) = tool_span_bounds(SystemTime::now(), input.duration_ms);
    if let Some(mut span) = start_child_span_at(SPAN_TOOL_CALL, attributes, started_at) {
        if failed {
            let err_desc = input.error.as_deref().unwrap_or("tool call failed");
            span.set_status(opentelemetry::trace::Status::error(err_desc.to_string()));
        }
        // 失败路径同样 end：`set_status` 之后直接 end，绝不提前 return → span 不泄漏。
        span.end_with_timestamp(ended_at);
    }
}

// ==================================================================== 私有实现
//
// 以下内容全部为模块私有：任何 OTel 类型都不出现在公开签名里。

use std::collections::HashMap;
use std::fmt;
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime};

use opentelemetry::trace::{
    Link, SamplingDecision, SamplingResult, Span, SpanKind, TraceContextExt, TraceId, TraceState,
    Tracer, TracerProvider,
};
use opentelemetry::{Context, KeyValue};
use opentelemetry_otlp::{WithExportConfig, WithHttpConfig};
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::trace::{
    BatchSpanProcessor, ShouldSample, SpanData, SpanExporter, SdkTracerProvider,
};

/// tracer 名（仅用于 provider 内部标识，不进入 wire 契约）。
const TRACER_NAME: &str = "pony-agent.agent.trace";
/// 导出 runtime 的线程名。
const RUNTIME_THREAD_NAME: &str = "pony-otel-export";
/// 单次 OTLP/HTTP 导出的网络超时：与 NFR 冻结的 `external_call_timeout_ms = 3000` 对齐。
/// 导出层自身必须有界（而不是靠调用方干等），黑洞端点下 `force_flush` 才会在 ~3s 内返回 Err。
const EXPORT_TIMEOUT_MS: u64 = 3_000;
/// 手动导出的单块 flush 等待上限（每块 ≤3000ms；总耗时随块数线性增长，属用户显式动作）。
const FORCE_EXPORT_FLUSH_TIMEOUT_MS: u64 = 3_000;
/// 关闭 provider 时的导出等待上限（Wave-4.1 A5：作为退出总预算 3000ms 的 1000ms 档位）。
///
/// 背景：进程退出路径最多经历「ExitRequested flush 1000ms + Exit flush 1000ms（幂等短路后
/// 通常为 0）+ shutdown 1000ms」，三段合计硬上限 ≤3000ms，满足 NFR
/// `observability.exporter_flush_timeout_ms` 与契约 C5 验收判据#1（3000ms 内优雅退出）。
const SHUTDOWN_TIMEOUT_MS: u64 = 1_000;
/// 降级日志单条最大字符数（超出截断，避免把超长响应体写进日志面）。
const WARN_MESSAGE_MAX_CHARS: usize = 512;
/// BatchSpanProcessor 队列上限（显式有界：队列满即丢，绝不无界堆积）。
const BATCH_MAX_QUEUE_SIZE: usize = 2_048;
/// 单批最大 span 数。
const BATCH_MAX_EXPORT_BATCH_SIZE: usize = 512;
/// 批量调度延迟。
const BATCH_SCHEDULED_DELAY_MS: u64 = 5_000;
/// `force_flush` 的等待上限夹值：与 `EXPORT_TIMEOUT_MS` / NFR `external_call_timeout_ms`
/// 对齐为 **3000ms**（task-10：单次 flush 预算不得超基准）。
const MAX_FLUSH_WAIT_MS: u64 = 3_000;
/// 手动上传的分块大小 = 单批导出上限（A8：避免「入队成功但被队列丢弃」被当成已交付）。
const FORCE_EXPORT_CHUNK_TURNS: usize = BATCH_MAX_EXPORT_BATCH_SIZE;

/// 全局上报开关（唯一门禁位；缺省关闭）。
static REPORTING_ENABLED: AtomicBool = AtomicBool::new(false);

/// 已装配的全局态；`None` 表示未 init（或已 shutdown）。
static STATE: Mutex<Option<Arc<OtelState>>> = Mutex::new(None);

/// 全局装配态：provider + tracer + 生效配置 + 导出用 runtime。
struct OtelState {
    provider: SdkTracerProvider,
    tracer: opentelemetry_sdk::trace::Tracer,
    config: OtelConfig,
    /// reqwest 异步客户端所需的 reactor 载体；必须存活到 provider 关闭之后。
    /// 仅用于保活导出 runtime：字段永不读取，drop 时释放 reactor。
    _runtime: tokio::runtime::Runtime,
}

fn lock_state() -> MutexGuard<'static, Option<Arc<OtelState>>> {
    STATE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn current_state() -> Option<Arc<OtelState>> {
    lock_state().as_ref().map(Arc::clone)
}

fn take_state() -> Option<Arc<OtelState>> {
    lock_state().take()
}

/// 门禁判定：只有「用户开启上报」时允许分配 span。
///
/// F6：手动上传**不再**用进程级旁路计数（那会把同期其它 turn 的 span 一起送出去），
/// 改为对重放 span 逐个打 `pony.otel.forced` 标：打标 span 由采样器强制采样、
/// 由导出器在关闭态逐个放行，未打标的 span 在关闭态一律丢弃。
fn spans_allowed() -> bool {
    REPORTING_ENABLED.load(Ordering::SeqCst)
}

/// 手动上传重放 span 的强制标记（仅由 [`force_export_session_trace`] 打标）。
const ATTR_PONY_OTEL_FORCED: &str = "pony.otel.forced";

/// 该 span 是否带「用户逐次授权」强制标记（F6：逐 span 判定，不看全局态）。
fn is_forced_attributes(attributes: &[KeyValue]) -> bool {
    attributes.iter().any(|pair| {
        pair.key.as_str() == ATTR_PONY_OTEL_FORCED
            && matches!(pair.value, opentelemetry::Value::Bool(true))
    })
}

fn env_non_empty(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// 是否为「携带凭据」的 header（F1：Debug / config() 回显必须剥离）。
fn is_sensitive_header(name: &str) -> bool {
    let lower = name.trim().to_ascii_lowercase();
    lower == AUTH_HEADER_NAME
        || lower.contains("token")
        || lower.contains("api-key")
        || lower.contains("apikey")
        || lower.contains("secret")
        || lower.contains("credential")
}

/// 明文 HTTP 且目标是**非回环**主机（ADVISORY：带 token 时拒绝出网）。
fn is_plaintext_remote(endpoint: &str) -> bool {
    let Ok(parsed) = url::Url::parse(endpoint) else {
        return false;
    };
    if parsed.scheme() != "http" {
        return false;
    }
    match parsed.host_str() {
        Some("localhost") | Some("127.0.0.1") | Some("::1") => false,
        _ => true,
    }
}

/// 目标端点是否为远端主机（非本机/回环地址）。用于未配 token 时发起告警。
fn is_remote_endpoint(endpoint: &str) -> bool {
    let Ok(parsed) = url::Url::parse(endpoint) else {
        return true;
    };
    match parsed.host_str() {
        Some("localhost") | Some("127.0.0.1") | Some("::1") => false,
        _ => true,
    }
}

/// 有值才输出该键（A2：缺失值一律不输出键，禁止空串占位）。
fn push_str(attributes: &mut Vec<(String, AttrValue)>, key: &str, value: &str) {
    if !value.is_empty() {
        // F2：所有字符串属性统一过 PonySentry 脱敏（工具错误/tool_name/session_id 等），
        // 复用既有 sanitize 实现，避免相对旧 trace 上报路径的安全退化（D6 退役后统一走本门面）。
        attributes.push((
            key.to_string(),
            AttrValue::Str(crate::agent::ponysentry::sanitize(value)),
        ));
    }
}

/// 中性属性值 → OTel `KeyValue`（OTel 类型只在私有实现里出现）。
fn to_key_values(attributes: Vec<(String, AttrValue)>) -> Vec<KeyValue> {
    attributes
        .into_iter()
        .map(|(key, value)| match value {
            AttrValue::Str(text) => KeyValue::new(key, opentelemetry::Value::String(text.into())),
            AttrValue::I64(number) => KeyValue::new(key, opentelemetry::Value::I64(number)),
            AttrValue::F64(number) => KeyValue::new(key, opentelemetry::Value::F64(number)),
            AttrValue::Bool(flag) => KeyValue::new(key, opentelemetry::Value::Bool(flag)),
        })
        .collect()
}

/// 取 span 的 trace_id 十六进制串；未采样/无效上下文返回 `None`。
fn sampled_trace_id(span: &mut opentelemetry_sdk::trace::Span) -> Option<String> {
    let span_context = opentelemetry::trace::Span::span_context(span);
    if span_context.is_valid() && span_context.is_sampled() {
        Some(span_context.trace_id().to_string())
    } else {
        None
    }
}

/// 一次活动 turn 的登记项。
///
/// `exported` 是两条通路的**语义分水岭**（Wave-4.1 L3-A B1/B1b）：
/// - `true`：由**真实 SDK root span** 建立（可导出、可在 Tempo/Langfuse 检索）；
/// - `false`：门禁关闭 / provider 未装配时的**本地相关性**上下文（只服务日志与出网
///   traceparent 的轮内关联，永不导出）。
/// 只有 `exported == true` 的登记才允许作为子 span 的 parent —— 否则会造出无 root 的孤儿 trace。
#[derive(Clone)]
struct ActiveTurn {
    turn_id: String,
    /// Wave-6 K4.1：所属会话 id（子 span 下沉 `langfuse.session.id` 时读取；空串即不输出）。
    session_id: String,
    span_context: opentelemetry::trace::SpanContext,
    exported: bool,
    /// Wave-5 K4：本轮内各 `llm.chat` 已记录成本之和（USD；Turn span Drop 时回填属性）。
    accumulated_cost_usd: f64,
    /// 本轮内各 `llm.chat` 累积的 input tokens。
    accumulated_input_tokens: u64,
    /// 本轮内各 `llm.chat` 累积的 output tokens。
    accumulated_output_tokens: u64,
    /// PA-154：本轮内子 span 错误聚合（`<prefix>: <detail>`，**先记录者胜**）；
    /// `None` ⇒ 本轮无错误，根 span 不输出 [`ATTR_AGENT_TURN_OUTCOME`]。
    accumulated_error: Option<String>,
}

/// turn 根 span 错误聚合前缀：工具执行失败（PA-154，谓词同 F5）。
const TURN_ERROR_PREFIX_TOOL_FAILED: &str = "tool_failed";

/// turn 根 span 错误聚合前缀：治理决策 block（PA-154，F6）。
const TURN_ERROR_PREFIX_QUOTA_BLOCKED: &str = "quota_blocked";

/// 根 span 错误承接的 `status.message` 前缀（PA-154）：与子 span 的原始错误文案区分，
/// 便于根级告警按 `turn_error` 单口径检索。
const TURN_ERROR_STATUS_PREFIX: &str = "turn_error";

/// 构造根 span 错误承接文案：`turn_error: <reason>`（reason ∈ `tool_failed` / `quota_blocked`）。
fn turn_error_status_message(reason: &str) -> String {
    format!("{TURN_ERROR_STATUS_PREFIX}: {reason}")
}

/// 活动 turn 索引：`tokio::task::Id` → 该任务当前 turn 的登记项。
///
/// F5（跨 turn 污染）：宿主按 turn 并发 spawn，若用「栈顶」选父 span，两个 turn 同时
/// 在跑时 A 的 llm/tool span 会被挂到 B 的 trace 上。改为**按当前 tokio 任务 id 索引**：
/// 守卫登记自己的 task id，Drop 时只摘除自己那条；无 task id（非 tokio 任务上下文）
/// 时退回线程本地栈；两者都取不到时**不加 parent** —— 宁可让子 span 自成 root，也绝不猜父 span。
static ACTIVE_TURNS: std::sync::LazyLock<Mutex<HashMap<tokio::task::Id, ActiveTurn>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

/// 取当前 tokio 任务 id；不在任务上下文时 `None`。
fn current_task_id() -> Option<tokio::task::Id> {
    tokio::task::try_id()
}

thread_local! {
    /// A9：无 tokio 任务上下文（TUI/CLI `run_sync_turn`、`direct_turn_probe`、std 单测）时的
    /// 活动 turn 兜底栈 —— 否则这些路径的子 span 会断链成独立 root。
    static THREAD_TURNS: RefCell<Vec<ActiveTurn>> = const { RefCell::new(Vec::new()) };
}

/// 一次 turn 登记的归属信息（task 表或线程本地栈，二者择一）。
struct ActiveTurnSlot {
    task_id: Option<tokio::task::Id>,
    /// 本登记的 turn_id 与 trace_id：Drop 时用于**精确**判定「当前登记是否仍属于自己」。
    turn_id: String,
    trace_id: String,
    /// 嵌套 turn 场景：本登记覆盖掉的前一条登记，Drop 时恢复。
    previous: Option<ActiveTurn>,
    thread_local: bool,
}

/// 登记活动 turn，返回归属信息（守卫 Drop 时用它精确摘除/回滚）。
///
/// 语义：门禁判定由调用方（[`begin_turn_trace`]）负责；本函数只做登记，
/// 同时支持「真实 span 上下文」（`exported = true`）与「本地降级上下文」（`exported = false`）。
/// 嵌套语义（L3-A#2）：同一 task/线程内再次登记会**保存前值**，Drop 时恢复而非丢弃。
fn register_active_turn(
    turn_id: &str,
    session_id: &str,
    span_context: opentelemetry::trace::SpanContext,
    exported: bool,
) -> Option<ActiveTurnSlot> {
    let trace_id = span_context.trace_id().to_string();
    let entry = ActiveTurn {
        turn_id: turn_id.to_string(),
        session_id: session_id.to_string(),
        span_context,
        exported,
        accumulated_cost_usd: 0.0,
        accumulated_input_tokens: 0,
        accumulated_output_tokens: 0,
        accumulated_error: None,
    };

    // A9：优先按 tokio 任务 id 索引；无任务上下文时退回线程本地栈（不丢父 span）。
    if let Some(task_id) = current_task_id() {
        let mut guard = ACTIVE_TURNS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = guard.insert(task_id, entry);
        return Some(ActiveTurnSlot {
            task_id: Some(task_id),
            turn_id: turn_id.to_string(),
            trace_id,
            previous,
            thread_local: false,
        });
    }
    THREAD_TURNS.with(|stack| {
        stack.borrow_mut().push(entry);
    });
    Some(ActiveTurnSlot {
        task_id: None,
        turn_id: turn_id.to_string(),
        trace_id,
        previous: None,
        thread_local: true,
    })
}

/// 摘除登记：只有「当前登记仍属于自己」时才动，并恢复嵌套时被覆盖的前值。
fn unregister_active_turn(slot: Option<ActiveTurnSlot>) {
    let Some(slot) = slot else {
        return;
    };
    if let Some(task_id) = slot.task_id {
        let mut guard = ACTIVE_TURNS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let owns = guard
            .get(&task_id)
            .map(|current| {
                current.turn_id == slot.turn_id
                    && current.span_context.trace_id().to_string() == slot.trace_id
            })
            .unwrap_or(false);
        if !owns {
            // 登记已被同 task 的后续 turn 覆盖：只放弃自己的归属，绝不动别人的登记。
            return;
        }
        match slot.previous {
            Some(previous) => {
                guard.insert(task_id, previous);
            }
            None => {
                guard.remove(&task_id);
            }
        }
        return;
    }
    if slot.thread_local {
        THREAD_TURNS.with(|stack| {
            let mut stack = stack.borrow_mut();
            // 逆序找到属于自己的那条（支持乱序 Drop），只摘除它自己。
            if let Some(index) = stack.iter().rposition(|entry| {
                entry.turn_id == slot.turn_id
                    && entry.span_context.trace_id().to_string() == slot.trace_id
            }) {
                stack.remove(index);
            }
        });
    }
}

/// 读取当前活动的 turn 登记项（真实或本地，未按导出态过滤）。
fn current_active_turn() -> Option<ActiveTurn> {
    if let Some(task_id) = current_task_id() {
        let guard = ACTIVE_TURNS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        return guard.get(&task_id).cloned();
    }
    THREAD_TURNS.with(|stack| stack.borrow().last().cloned())
}

/// 把一次调用的成本累加到**当前活动 turn**（供 `agent.turn.cost.usd` 聚合）。
///
/// 语义：只在存在活动 turn 时累加；非有限/负值直接忽略。门禁关闭时调用方不会走到这里。
fn accumulate_active_turn_cost(cost: f64) {
    if !cost.is_finite() || cost < 0.0 {
        return;
    }
    if let Some(task_id) = current_task_id() {
        let mut guard = ACTIVE_TURNS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(entry) = guard.get_mut(&task_id) {
            entry.accumulated_cost_usd += cost;
        }
        return;
    }
    THREAD_TURNS.with(|stack| {
        if let Some(entry) = stack.borrow_mut().last_mut() {
            entry.accumulated_cost_usd += cost;
        }
    });
}

/// 把一次调用的 token 用量累加到当前活动 turn。
fn accumulate_active_turn_usage(input_tokens: Option<u64>, output_tokens: Option<u64>) {
    let inp = input_tokens.unwrap_or(0);
    let out = output_tokens.unwrap_or(0);
    if inp == 0 && out == 0 {
        return;
    }
    if let Some(task_id) = current_task_id() {
        let mut guard = ACTIVE_TURNS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(entry) = guard.get_mut(&task_id) {
            entry.accumulated_input_tokens += inp;
            entry.accumulated_output_tokens += out;
        }
        return;
    }
    THREAD_TURNS.with(|stack| {
        if let Some(entry) = stack.borrow_mut().last_mut() {
            entry.accumulated_input_tokens += inp;
            entry.accumulated_output_tokens += out;
        }
    });
}

/// 读取当前活动 turn 的累计 token 用量；无活动 turn 或全为 0 时返回 None。
fn take_active_turn_usage() -> (Option<u64>, Option<u64>) {
    let get_tokens = |entry: &ActiveTurn| {
        let inp = (entry.accumulated_input_tokens > 0).then_some(entry.accumulated_input_tokens);
        let out = (entry.accumulated_output_tokens > 0).then_some(entry.accumulated_output_tokens);
        (inp, out)
    };
    if let Some(task_id) = current_task_id() {
        let guard = ACTIVE_TURNS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        return guard
            .get(&task_id)
            .map(get_tokens)
            .unwrap_or((None, None));
    }
    THREAD_TURNS.with(|stack| {
        stack
            .borrow()
            .last()
            .map(get_tokens)
            .unwrap_or((None, None))
    })
}

/// 读取当前活动 turn 的累计成本；无活动 turn 时返回 0.0。
fn take_active_turn_cost() -> f64 {
    if let Some(task_id) = current_task_id() {
        let guard = ACTIVE_TURNS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        return guard
            .get(&task_id)
            .map(|entry| entry.accumulated_cost_usd)
            .unwrap_or(0.0);
    }
    THREAD_TURNS.with(|stack| {
        stack
            .borrow()
            .last()
            .map(|entry| entry.accumulated_cost_usd)
            .unwrap_or(0.0)
    })
}

/// 把一次子 span 错误聚合到**当前活动 turn**（PA-154：根 span 错误承接）。
///
/// 语义：与成本聚合同源——按 tokio task id 索引，无任务上下文时退回线程本地栈；
/// 无活动 turn 时 **no-op**（绝不新建登记）；已有记录时保留**先记录者**（`reason`），
/// 后续错误不覆盖（双错误并存不拼长文案）。
fn mark_active_turn_error(reason: &str) {
    let message = reason.to_string();
    let record = |entry: &mut ActiveTurn| {
        if entry.accumulated_error.is_none() {
            entry.accumulated_error = Some(message);
        }
    };
    if let Some(task_id) = current_task_id() {
        let mut guard = ACTIVE_TURNS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(entry) = guard.get_mut(&task_id) {
            record(entry);
        }
        return;
    }
    THREAD_TURNS.with(|stack| {
        if let Some(entry) = stack.borrow_mut().last_mut() {
            record(entry);
        }
    });
}

/// 取走并**清空**当前活动 turn 的错误聚合（Turn 守卫 Drop 调用）。
///
/// 语义：取走即清零 ⇒ 跨 turn 不污染；无活动 turn / 本轮无错时返回 `None`。
fn take_active_turn_error() -> Option<String> {
    if let Some(task_id) = current_task_id() {
        let mut guard = ACTIVE_TURNS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        return guard
            .get_mut(&task_id)
            .and_then(|entry| entry.accumulated_error.take());
    }
    THREAD_TURNS.with(|stack| {
        stack
            .borrow_mut()
            .last_mut()
            .and_then(|entry| entry.accumulated_error.take())
    })
}

/// 把当前活跃 turn 的会话 id 作为 `langfuse.session.id` 属性压入子 span 属性表。
///
/// 语义（K4.1）：无活跃 turn / session_id 为空或纯空白 ⇒ **不输出该键**（禁止空串/伪造值）。
fn push_active_session_attribute(attributes: &mut Vec<KeyValue>) {
    let Some(session_id) = current_active_turn()
        .map(|entry| entry.session_id)
        .filter(|value| !value.trim().is_empty())
    else {
        return;
    };
    attributes.push(KeyValue::new(
        ATTR_LANGFUSE_SESSION_ID,
        crate::agent::ponysentry::sanitize(&session_id),
    ));
}

/// turn_id → 该 turn 的**导出态** trace_id（Wave-6 K6/ Issue 互跳修复）。
///
/// 为什么需要：`capture_turn_failure` 可能在与 `begin_turn_trace` **不同的任务/线程上下文**
/// 中执行（或守卫已 Drop），此时 `exported_trace_id()` 取不到 ⇒ issue 丢 trace_id。
/// 登记在 turn 建立真实 root span 时写入，按 turn 精确回查（不是伪造：就是该 turn 的真实 id）。
static TURN_EXPORTED_TRACE_IDS: std::sync::LazyLock<Mutex<HashMap<String, String>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

/// 登记 turn 的导出态 trace_id（有界：超过 [`TURN_TRACE_ID_CAPACITY`] 清空重来）。
fn register_turn_exported_trace_id(turn_id: &str, trace_id: &str) {
    let mut guard = TURN_EXPORTED_TRACE_IDS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if guard.len() >= TURN_TRACE_ID_CAPACITY {
        guard.clear();
    }
    guard.insert(turn_id.to_string(), trace_id.to_string());
}

/// 按 turn_id 回查其**导出态** trace_id；未登记 / 已淘汰 ⇒ `None`。
///
/// 调用方：`ponysentry::capture_turn_failure`（跨上下文补偿，绝不返回本地相关性 id）。
pub fn exported_trace_id_for_turn(turn_id: &str) -> Option<String> {
    let guard = TURN_EXPORTED_TRACE_IDS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard.get(turn_id).cloned()
}

/// turn → trace_id 登记表的容量上限（进程内最多保留最近这么多轮）。
const TURN_TRACE_ID_CAPACITY: usize = 512;

/// 子 span 的父上下文：**只**接受真实导出态 root span 的上下文（B1b：杜绝孤儿 trace）。
///
/// 本地相关性上下文（`exported = false`）永不作父 —— 否则 turn 中途打开开关时，
/// 后续子 span 会以一条从无 root 的 trace_id 真实出网。
fn active_exported_turn_context() -> Option<Context> {
    let entry = current_active_turn()?;
    if !entry.exported || !entry.span_context.is_valid() || !entry.span_context.is_sampled() {
        return None;
    }
    Some(Context::new().with_remote_span_context(entry.span_context))
}

/// 在「当前活动 turn」之下开一个子 span；门禁关闭时返回 `None`（零分配）。
fn start_child_span(
    name: &str,
    attributes: Vec<KeyValue>,
) -> Option<opentelemetry_sdk::trace::Span> {
    if !spans_allowed() {
        return None;
    }
    let state = current_state()?;
    let parent = active_exported_turn_context().unwrap_or_else(Context::new);
    Some(state.tracer.build_with_context(
        opentelemetry::trace::SpanBuilder::from_name(name.to_string())
            .with_kind(SpanKind::Internal)
            .with_attributes(attributes),
        &parent,
    ))
}

/// 回填上限：单次工具调用超过 24h 视为异常数据（时钟跳变 / 脏 duration）。
///
/// 为什么需要它：`SystemTime::checked_sub` 的平台语义不一致——Linux 的 `SystemTime` 允许
/// 负值（i64 秒），`u64::MAX` 毫秒（≈5.8 亿年）依然能"减成功"，会生成一条跨越几亿年的离奇
/// span；Windows 的 FILETIME 不能早于 1601，则退化为零长。为了让行为**跨平台确定**、且不让
/// 脏数据污染 trace 时间轴，这里显式设上限：超出即退化为零长 span（原始值仍原样保留在属性
/// `agent.tool.duration_ms` 里，信息不丢）。
const TOOL_SPAN_MAX_BACKDATE_MS: u64 = 24 * 60 * 60 * 1_000;

/// T11：工具子 span 的时间边界——`end` 取采集时刻，`start = end - duration`。
///
/// 抽成纯函数是为了让「span 覆盖真实执行区间」可被**无线程 / 无全局 provider / 无网络**的
/// 单测锁定：`None`（耗时未知）、超过 [`TOOL_SPAN_MAX_BACKDATE_MS`] 的异常值、以及
/// `checked_sub` 溢出，一律退化为零长 span —— 绝不 panic、绝不凭空造时长。
fn tool_span_bounds(now: SystemTime, duration_ms: Option<u64>) -> (SystemTime, SystemTime) {
    let Some(millis) =
        duration_ms.filter(|millis| *millis <= TOOL_SPAN_MAX_BACKDATE_MS)
    else {
        return (now, now);
    };
    match now.checked_sub(Duration::from_millis(millis)) {
        Some(started_at) => (started_at, now),
        // 平台最早时刻之前的回填：退化为零长 span（不 panic、不生成越界时间戳）。
        None => (now, now),
    }
}

/// 与 [`start_child_span`] 同构，唯一差别是允许**显式起点**（回填真实执行区间起点）。
fn start_child_span_at(
    name: &str,
    attributes: Vec<KeyValue>,
    started_at: SystemTime,
) -> Option<opentelemetry_sdk::trace::Span> {
    if !spans_allowed() {
        return None;
    }
    let state = current_state()?;
    let parent = active_exported_turn_context().unwrap_or_else(Context::new);
    Some(state.tracer.build_with_context(
        opentelemetry::trace::SpanBuilder::from_name(name.to_string())
            .with_kind(SpanKind::Internal)
            .with_start_time(started_at)
            .with_attributes(attributes),
        &parent,
    ))
}

/// 门禁采样器：关闭态返回 `Drop`（零 span 分配、零 egress）；开启态按父级与采样率决策。
#[derive(Clone, Debug)]
struct GatedSampler {
    ratio: f64,
}

impl ShouldSample for GatedSampler {
    fn should_sample(
        &self,
        parent_context: Option<&Context>,
        trace_id: TraceId,
        _name: &str,
        _span_kind: &SpanKind,
        attributes: &[KeyValue],
        _links: &[Link],
    ) -> SamplingResult {
        let result = |decision| SamplingResult {
            decision,
            attributes: Vec::new(),
            trace_state: TraceState::default(),
        };

        // F6：手动上传重放 span 带 `pony.otel.forced`，无条件采样（仅这些 span）。
        if is_forced_attributes(attributes) {
            return result(SamplingDecision::RecordAndSample);
        }
        if !spans_allowed() {
            return result(SamplingDecision::Drop);
        }

        // 父级决定（ParentBased）：已采样的父 span 的下游一律采样，保证子 span 不断链。
        if let Some(parent) = parent_context {
            // 先落局部绑定：`parent.span()` 的临时 SpanRef 不能在语句结束后被借用。
            let parent_span = parent.span();
            let parent_span_context = parent_span.span_context();
            if parent_span_context.is_valid() {
                return result(if parent_span_context.is_sampled() {
                    SamplingDecision::RecordAndSample
                } else {
                    SamplingDecision::Drop
                });
            }
        }

        if self.ratio >= 1.0 {
            result(SamplingDecision::RecordAndSample)
        } else if self.ratio <= 0.0 {
            result(SamplingDecision::Drop)
        } else if trace_fraction(trace_id) < self.ratio {
            result(SamplingDecision::RecordAndSample)
        } else {
            result(SamplingDecision::Drop)
        }
    }
}

/// 以 trace_id 低 64 位把 trace 均匀映射到 `[0.0, 1.0)`。
fn trace_fraction(trace_id: TraceId) -> f64 {
    let bytes = trace_id.to_bytes();
    let mut low = 0u64;
    for byte in &bytes[8..] {
        low = (low << 8) | u64::from(*byte);
    }
    (low as f64) / (u64::MAX as f64)
}

/// 导出器包装：在 `BatchSpanProcessor` 的后台线程里为导出 future 提供 tokio reactor 上下文。
///
/// 背景：批量导出线程由 SDK 自建（非 tokio 线程），而 reqwest 异步客户端 poll 时需要
/// 运行时上下文。这里在 poll 之前 `Handle::enter()`，使连接/驱动注册落在 `init` 建的
/// runtime 上；runtime 句柄随全局态存活，导出期间始终有效。
struct RuntimeExporter {
    inner: opentelemetry_otlp::SpanExporter,
    handle: tokio::runtime::Handle,
}

impl fmt::Debug for RuntimeExporter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // F1：不转发上游 exporter 的 Debug（其内部含 endpoint/header 等敏感信息）。
        formatter
            .debug_struct("RuntimeExporter")
            .field("runtime", &"pony-otel-export")
            .finish_non_exhaustive()
    }
}

impl SpanExporter for RuntimeExporter {
    fn export(&self, batch: Vec<SpanData>) -> impl std::future::Future<Output = OTelSdkResult> + Send {
        let handle = self.handle.clone();
        // F3 + F6：门禁关闭 ⇒ 逐 span 放行判定（只认 `pony.otel.forced` 打标的重放 span），
        // 未打标的一律丢弃；过滤后为空则直接成功返回，不产生任何网络出口。
        let exportable = retain_exportable(batch);
        if exportable.is_empty() {
            return futures_util::future::Either::Left(std::future::ready(Ok(())));
        }
        let inner_export = self.inner.export(exportable);
        futures_util::future::Either::Right(async move {
            // 守卫**只在单次 poll 内**存活：不能跨 await 持有 `EnterGuard`（它不是 `Send`，
            // 会让整个导出 future 失去 `Send`）。因此手动逐次 poll，每次 poll 前进入
            // runtime 上下文，poll 返回后守卫立即释放。
            let mut inner_export = std::pin::pin!(inner_export);
            std::future::poll_fn(move |context| {
                let _entered = handle.enter();
                std::future::Future::poll(inner_export.as_mut(), context)
            })
            .await
        })
    }

    /// **必须转发**：`SpanExporter::set_resource` 缺省是空实现，而 OTLP exporter 正是用它
    /// 得知道「这批 span 的 Resource」，再用它分组（`group_spans_by_resource_and_scope`）。
    /// 曾经的 bug：包装器没转发 ⇒ 导出的 ResourceSpans 没有 resource ⇒ Tempo 里没有
    /// `service.name`（公网 edge 侧只能靠它自己改写成 `desktop-client` 才「看起来正常」），
    /// Langfuse 侧也丢了 service 归因。2026-10-24 由公网端到端探针实测发现。
    fn set_resource(&mut self, resource: &opentelemetry_sdk::Resource) {
        self.inner.set_resource(resource);
    }

    fn shutdown_with_timeout(&mut self, timeout: Duration) -> OTelSdkResult {
        self.inner.shutdown_with_timeout(timeout)
    }
}

/// 把 base 端点补成 OTLP/HTTP trace 导出路径。
///
/// OTLP/HTTP 规范：程序化 `with_endpoint` 的取值被**原样**使用（不像
/// `OTEL_EXPORTER_OTLP_ENDPOINT` 那样自动补 `/v1/traces`），因此这里显式补全；
/// 已带 `/v1/traces` 的端点保持不变（幂等，避免出现 `/v1/traces/v1/traces`）。
fn traces_endpoint(base: &str) -> String {
    let trimmed = base.trim().trim_end_matches('/');
    if trimmed.ends_with("/v1/traces") {
        trimmed.to_string()
    } else {
        format!("{trimmed}/v1/traces")
    }
}

/// 导出前的逐 span 放行判定（F3 + F6）。
///
/// 门禁开启时全量放行；门禁关闭时**只**放行带 `pony.otel.forced` 的手动上传重放 span，
/// 其余（含已在飞、刚从批量队列取出的正常 turn span）一律丢弃 ⇒ 「关 = 零出网」。
fn retain_exportable(batch: Vec<SpanData>) -> Vec<SpanData> {
    if spans_allowed() {
        return batch;
    }
    batch
        .into_iter()
        .filter(|span| is_forced_attributes(&span.attributes))
        .collect()
}

/// 构造 OTLP/HTTP-protobuf 导出器（带鉴权 header 与网络超时）。
fn build_runtime_exporter(
    config: &OtelConfig,
    handle: tokio::runtime::Handle,
) -> Result<RuntimeExporter, String> {
    let mut builder = opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .with_endpoint(traces_endpoint(&config.endpoint))
        .with_protocol(opentelemetry_otlp::Protocol::HttpBinary)
        .with_timeout(Duration::from_millis(EXPORT_TIMEOUT_MS));

    let headers: HashMap<String, String> = config.headers.iter().cloned().collect();
    if !headers.is_empty() {
        builder = builder.with_headers(headers);
    }

    let inner = builder
        .build()
        .map_err(|err| format!("otel::init：OTLP/HTTP 导出器构建失败：{err}"))?;

    Ok(RuntimeExporter { inner, handle })
}

/// 执行 `force_flush`，并按 `timeout_ms` 给出超时判定。
///
/// 有界性由导出层自身超时保证（`EXPORT_TIMEOUT_MS` = nfr `external_call_timeout_ms` = 3000），
/// 因此**不再起辅助线程**（辅助线程在超时后无人 join，属泄漏）；这里改为直接调用 + 实测耗时，
/// 超过 `timeout_ms` 时如实返回 `Err`。失败只降级为 warn JSON 日志，绝不冒泡、不 panic。
fn flush_state(state: &OtelState, timeout_ms: u64) -> Result<(), String> {
    let provider = state.provider.clone();
    // A6：硬截止必须真的落在**等待**路径上（不能「先干等再判超时」）。
    // 等待上限再夹一层 `MAX_FLUSH_WAIT_MS`，保证调用方（async command）最长阻塞有界。
    let wait_ms = timeout_ms.min(MAX_FLUSH_WAIT_MS);
    let (sender, receiver) = std::sync::mpsc::channel::<OTelSdkResult>();

    let worker = std::thread::Builder::new()
        .name("pony-otel-flush".to_string())
        .spawn(move || {
            let _ = sender.send(provider.force_flush());
        })
        .map_err(|err| format!("force_flush 线程启动失败：{err}"))?;

    let outcome = match receiver.recv_timeout(Duration::from_millis(wait_ms)) {
        Ok(Ok(())) => {
            let _ = worker.join();
            Ok(())
        }
        Ok(Err(err)) => {
            let _ = worker.join();
            Err(format!("OTLP 导出失败：{err}"))
        }
        Err(err) => Err(format!(
            "force_flush 在 {wait_ms}ms 内未完成：{err}（导出线程将在导出层自身超时后自行退出）"
        )),
    };
    if let Err(message) = outcome.as_ref() {
        degraded_warn(message, None);
    }
    outcome
}

/// 失败降级日志面：先脱敏、再截断（避免把密钥或超长响应体写进日志）。
fn degraded_warn(message: &str, trace_id: Option<&str>) {
    let sanitized = crate::agent::ponysentry::sanitize(message);
    let mut text: String = sanitized.chars().take(WARN_MESSAGE_MAX_CHARS).collect();
    if sanitized.chars().count() > WARN_MESSAGE_MAX_CHARS {
        text.push_str("…[truncated]");
    }
    log::log_json("warn", &text, trace_id);
}

// ==================================================================== 实现级单测
//
// 只覆盖「安全门禁」的实现细节（不构成验收门禁；验收断言在 tests/ 下由 Test Agent 独占）。
#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::atomic::AtomicUsize;

    /// 全局门禁/provider 是进程级状态，本模块单测必须串行（避免并行用例互相污染）。
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn test_lock() -> MutexGuard<'static, ()> {
        TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn span_context(trace: u128) -> opentelemetry::trace::SpanContext {
        opentelemetry::trace::SpanContext::new(
            opentelemetry::trace::TraceId::from(trace),
            opentelemetry::trace::SpanId::from((trace as u64) | 1),
            opentelemetry::trace::TraceFlags::SAMPLED,
            true,
            opentelemetry::trace::TraceState::default(),
        )
    }

    fn str_attr(attrs: &[(String, AttrValue)], key: &str) -> String {
        match attrs.iter().find(|(name, _)| name == key).map(|(_, value)| value) {
            Some(AttrValue::Str(text)) => text.clone(),
            other => panic!("属性 `{key}` 必须是字符串，实际 {other:?}"),
        }
    }

    /// F5 回归：两个并发 turn 的活动上下文必须各自独立（按 tokio task id 索引，而非栈顶）。
    #[test]
    fn concurrent_turns_do_not_share_parent_span() {
        let _guard = test_lock();
        REPORTING_ENABLED.store(true, Ordering::SeqCst);
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .enable_all()
            .build()
            .expect("测试 runtime 必须可创建");
        let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
        let mismatches = std::sync::Arc::new(AtomicUsize::new(0));

        let mut handles = Vec::new();
        for index in 0..2u128 {
            let barrier = barrier.clone();
            let mismatches = mismatches.clone();
            handles.push(runtime.spawn(async move {
                let expected = span_context(0x1000 + index);
                // exported = true：本用例模拟真实 root span 登记（父子链语义）。
                let slot = register_active_turn(
                    &format!("turn-{index}"),
                    &format!("sess-{index}"),
                    expected.clone(),
                    true,
                );
                assert!(slot.is_some(), "活动 turn 必须登记成功");
                assert!(
                    slot.as_ref().and_then(|slot| slot.task_id).is_some(),
                    "tokio 任务内必须走 task 表（而非线程本地栈）"
                );
                // 两个 turn 同时处于活动状态，模拟宿主按 turn 并发 spawn。
                barrier.wait().await;
                let observed = active_exported_turn_context().expect("任务内必须有活动 turn");
                if observed.span().span_context().trace_id() != expected.trace_id() {
                    mismatches.fetch_add(1, Ordering::SeqCst);
                }
                unregister_active_turn(slot);
                assert!(
                    current_active_turn().is_none(),
                    "摘除后该任务不得再有活动 turn"
                );
            }));
        }
        for handle in handles {
            runtime.block_on(handle).expect("并发测试任务不得 panic");
        }
        assert_eq!(
            mismatches.load(Ordering::SeqCst),
            0,
            "F5：并发 turn 的父 span 串线（跨 turn 污染）"
        );
        REPORTING_ENABLED.store(false, Ordering::SeqCst);
    }

    /// F2 回归：字符串属性必须统一过 PonySentry 脱敏（令牌/家目录不得明文上报）。
    #[test]
    fn string_attributes_are_sanitized() {
        let _guard = test_lock();
        let attrs = turn_span_attributes(&TurnSpanInput {
            session_id: "sess Bearer sk-live-ABCDEFGHIJK".to_string(),
            user_id: Some("/Users/alice/.ssh/id_rsa".to_string()),
            turn_id: "turn-1".to_string(),
            sequence: 0,
            phase: "failed".to_string(),
        });
        let session = str_attr(&attrs, ATTR_LANGFUSE_SESSION_ID);
        assert!(
            session.contains("[REDACTED_SECRET]"),
            "F2：Bearer token 必须脱敏，实际 `{session}`"
        );
        assert!(
            !session.contains("sk-live-ABCDEFGHIJK"),
            "F2：令牌明文泄漏 `{session}`"
        );
        let user = str_attr(&attrs, ATTR_LANGFUSE_USER_ID);
        assert!(!user.contains("alice"), "F2：家目录明文泄漏 `{user}`");

        let tool_attrs = tool_span_attributes(&ToolSpanInput {
            tool_name: "bash".to_string(),
            status: "failed".to_string(),
            duration_ms: Some(3),
            error: Some("curl -H 'authorization: Bearer sk-live-ABCDEFGHIJK'".to_string()),
        });
        let error = str_attr(&tool_attrs, ATTR_AGENT_TOOL_ERROR);
        assert!(
            !error.contains("sk-live-ABCDEFGHIJK"),
            "F2：工具错误里的令牌必须脱敏 `{error}`"
        );
    }

    /// 公网入口回归：`PONY_OTEL_TOKEN` 必须组装成 `Authorization: Bearer <token>`。
    ///
    /// 背景：集群公网 edge collector 用 `bearertokenauth`（`scheme: Bearer`），
    /// 裸 token 实测 401；本用例钉死「补 scheme」与「已带 scheme 不重复」两种形态。
    #[test]
    fn authorization_header_adds_bearer_scheme_once() {
        assert_eq!(authorization_header_value("raw-token-123"), "Bearer raw-token-123");
        assert_eq!(authorization_header_value("  raw-token-123  "), "Bearer raw-token-123");
        assert_eq!(authorization_header_value("Bearer already"), "Bearer already");
        assert_eq!(authorization_header_value("bearer lower"), "bearer lower");
        assert_eq!(authorization_header_value("Basic dXNlcjpwYXNz"), "Basic dXNlcjpwYXNz");
    }

    /// F1 回归：`Debug` 与敏感 header 判定不得泄漏凭据；明文外网判定覆盖回环豁免。
    #[test]
    fn config_debug_redacts_headers_and_detects_sensitive_names() {
        let _guard = test_lock();
        let config = OtelConfig {
            endpoint: "https://collector.example/v1/traces".to_string(),
            service_name: "svc".to_string(),
            sampling_ratio: 1.0,
            headers: vec![(AUTH_HEADER_NAME.to_string(), "SUPER_SECRET_TOKEN_123".to_string())],
        };
        let shown = format!("{config:?}");
        assert!(
            !shown.contains("SUPER_SECRET_TOKEN_123"),
            "F1：Debug 明文泄漏 header 值：{shown}"
        );
        assert!(shown.contains("[REDACTED]"), "F1：Debug 必须显式标出脱敏位");

        assert!(is_sensitive_header("authorization"));
        assert!(is_sensitive_header("X-Api-Key"));
        assert!(is_sensitive_header("X-Auth-Token"));
        assert!(!is_sensitive_header("content-type"));

        assert!(is_plaintext_remote("http://collector.example:4318"));
        assert!(!is_plaintext_remote("http://127.0.0.1:4318"));
        assert!(!is_plaintext_remote("https://collector.example:4318"));
    }

    /// F6 回归（采样面）：关闭态下**只有**带 `pony.otel.forced` 的重放 span 被采样，
    /// 未打标的 span 一律 Drop —— 手动上传不再借全局旁路污染同期其它 turn。
    #[test]
    fn forced_spans_sample_while_gate_is_closed() {
        let _guard = test_lock();
        REPORTING_ENABLED.store(false, Ordering::SeqCst);
        let sampler = GatedSampler { ratio: 1.0 };
        let trace_id = opentelemetry::trace::TraceId::from(7u128);
        let noop_context = Context::new();

        let forced = [KeyValue::new(ATTR_PONY_OTEL_FORCED, true)];
        let decision = sampler
            .should_sample(
                Some(&noop_context),
                trace_id,
                SPAN_AGENT_TURN,
                &opentelemetry::trace::SpanKind::Internal,
                &forced,
                &[],
            )
            .decision;
        assert_eq!(
            decision,
            opentelemetry::trace::SamplingDecision::RecordAndSample,
            "F6：打标的重放 span 必须被强制采样"
        );

        // 同期「正常 turn」的 span（未打标）在关闭态必须 Drop，即使此刻正在上传。
        let normal = [KeyValue::new(ATTR_LANGFUSE_SESSION_ID, "sess-other")];
        let decision = sampler
            .should_sample(
                Some(&noop_context),
                trace_id,
                SPAN_LLM_CHAT,
                &opentelemetry::trace::SpanKind::Internal,
                &normal,
                &[],
            )
            .decision;
        assert_eq!(
            decision,
            opentelemetry::trace::SamplingDecision::Drop,
            "F6：未打标 span 在关闭态必须被丢弃"
        );
        assert!(!is_reporting_enabled(), "F6：采样判定不得改写门禁末态");
    }

    /// F6 回归（导出面，wire 级）：关闭态手动上传只把带 `pony.otel.forced` 的 span 送上线路。
    #[test]
    fn closed_gate_manual_upload_exports_only_forced_spans() {
        let _guard = test_lock();
        shutdown();
        REPORTING_ENABLED.store(false, Ordering::SeqCst);

        let (port, sink) = spawn_local_otlp_sink(Duration::from_millis(1_500));
        init(OtelConfig {
            endpoint: format!("http://127.0.0.1:{port}"),
            service_name: "pony-agent-otel-unit".to_string(),
            sampling_ratio: 1.0,
            headers: Vec::new(),
        })
        .expect("回环端点必须可装配");

        // 同期「另一个 turn」：关闭态下连 span 都不该分配（零分配契约）。
        let foreign_scope = TurnScope::new("sess-foreign-turn", "turn-foreign", 0);
        assert!(
            foreign_scope.trace_id().is_none(),
            "关闭态不得分配 span（否则就有被顺手导出的风险）"
        );
        foreign_scope.end();

        let traces = vec![
            crate::agent::session::TurnTraceRecord {
                turn_id: "turn-forced-1".to_string(),
                session_id: Some("sess-forced-upload".to_string()),
                title: "unit".to_string(),
                phase: "completed".to_string(),
                ..Default::default()
            },
            crate::agent::session::TurnTraceRecord {
                turn_id: "turn-forced-2".to_string(),
                session_id: Some("sess-forced-upload".to_string()),
                title: "unit".to_string(),
                phase: "failed".to_string(),
                ..Default::default()
            },
        ];
        let exported = force_export_session_trace("sess-forced-upload", &traces)
            .expect("关闭态手动上传必须成功（旁路按 span 打标）");
        assert_eq!(exported, 2, "A8：返回值必须是实际完成 flush 的 turn 数");
        assert!(!is_reporting_enabled(), "F6：手动上传不得改写门禁末态");

        shutdown();
        let bodies = sink.join().expect("sink 线程不得 panic");
        let wire = String::from_utf8_lossy(&bodies).to_string();
        assert!(!wire.is_empty(), "手动上传必须真的产生 OTLP 出口");
        assert!(
            wire.contains(ATTR_PONY_OTEL_FORCED),
            "F6：导出体必须带 `pony.otel.forced` 标记"
        );
        assert!(
            wire.contains("sess-forced-upload"),
            "F6：导出体必须含被上传会话 id"
        );
        assert!(
            !wire.contains("sess-foreign-turn"),
            "F6：关闭态手动上传**不得**带出同期其它 turn 的 span"
        );
        // 回归（2026-10-24 公网 E2E 发现）：包装导出器若忘记转发 `set_resource`，
        // 导出的 ResourceSpans 会没有任何 resource —— Tempo 里没有 `service.name`，
        // 公网 edge 只能把它改写成 `desktop-client`，`client.service.name` 也随之丢失。
        assert!(
            wire.contains("service.name"),
            "导出体必须携带 Resource 的 `service.name`（RuntimeExporter 必须转发 set_resource）"
        );
        assert!(
            wire.contains("pony-agent-otel-unit"),
            "导出体的 `service.name` 必须等于配置值"
        );
    }

    /// 本地 OTLP/HTTP sink：读满一个请求后回 200，返回累积的 body 字节。
    fn spawn_local_otlp_sink(budget: Duration) -> (u16, std::thread::JoinHandle<Vec<u8>>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("sink 必须能绑定回环端口");
        listener.set_nonblocking(true).expect("sink 必须非阻塞");
        let port = listener.local_addr().expect("sink local_addr").port();

        let handle = std::thread::spawn(move || {
            use std::io::Read as _;
            use std::io::Write as _;
            let mut bodies: Vec<u8> = Vec::new();
            let deadline = std::time::Instant::now() + budget;
            while std::time::Instant::now() < deadline {
                match listener.accept() {
                    Ok((mut stream, _peer)) => {
                        let _ = stream.set_read_timeout(Some(Duration::from_millis(250)));
                        let mut raw: Vec<u8> = Vec::new();
                        let mut buffer = [0u8; 4096];
                        let mut expected_total: Option<usize> = None;
                        loop {
                            if let Some(total) = expected_total {
                                if raw.len() >= total {
                                    break;
                                }
                            }
                            match stream.read(&mut buffer) {
                                Ok(0) => break,
                                Ok(read) => {
                                    raw.extend_from_slice(&buffer[..read]);
                                    if expected_total.is_none() {
                                        if let Some(header_end) = raw
                                            .windows(4)
                                            .position(|window| window == b"\r\n\r\n")
                                        {
                                            let headers =
                                                String::from_utf8_lossy(&raw[..header_end])
                                                    .to_lowercase();
                                            let length = headers
                                                .lines()
                                                .find_map(|line| {
                                                    line.strip_prefix("content-length:")
                                                })
                                                .and_then(|value| value.trim().parse::<usize>().ok())
                                                .unwrap_or(0);
                                            expected_total = Some(header_end + 4 + length);
                                        }
                                    }
                                }
                                Err(_) => break,
                            }
                        }
                        let body_start = raw
                            .windows(4)
                            .position(|window| window == b"\r\n\r\n")
                            .map(|index| index + 4)
                            .unwrap_or(raw.len());
                        bodies.extend_from_slice(&raw[body_start..]);
                        let _ = stream.write_all(
                            b"HTTP/1.1 200 OK\r\ncontent-type: application/x-protobuf\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                        );
                        let _ = stream.flush();
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(10)),
                }
            }
            bodies
        });

        (port, handle)
    }

    /// task-10 探针：黑洞端点下 600 轮上传 —— 单次 flush 必须 ≤3000ms（+容差），
    /// 且 `shutdown()` 的超时/失败分支不得 panic（runtime 有意保留至进程终止）。
    #[test]
    fn blackhole_600_turn_upload_stays_within_flush_budget() {
        let _guard = test_lock();
        shutdown();
        REPORTING_ENABLED.store(false, Ordering::SeqCst);

        let (port, blackhole) = spawn_blackhole_sink(Duration::from_millis(12_000));
        init(OtelConfig {
            endpoint: format!("http://127.0.0.1:{port}"),
            service_name: "pony-agent-otel-probe".to_string(),
            sampling_ratio: 1.0,
            headers: Vec::new(),
        })
        .expect("回环端点必须可装配");

        let traces: Vec<crate::agent::session::TurnTraceRecord> = (0..600)
            .map(|index| crate::agent::session::TurnTraceRecord {
                turn_id: format!("turn-probe-{index}"),
                session_id: Some("sess-probe-600".to_string()),
                title: "probe".to_string(),
                phase: "completed".to_string(),
                ..Default::default()
            })
            .collect();

        let started = std::time::Instant::now();
        let result = force_export_session_trace("sess-probe-600", &traces);
        let elapsed = started.elapsed();

        assert!(
            result.is_err(),
            "黑洞端点下单块导出必须返回 Err（不得 panic）：{result:?}"
        );
        assert!(
            elapsed <= Duration::from_millis(3_250),
            "task-10：单次 flush 必须 ≤3000ms(+容差)，实测 {elapsed:?}"
        );
        assert!(
            elapsed >= Duration::from_millis(2_000),
            "task-10：必须真的发出去了，否则探针无效（实测 {elapsed:?}）"
        );

        // 超时/失败分支：只记 warn，不 drop runtime ⇒ 不得 panic。
        shutdown();
        assert!(!is_reporting_enabled(), "shutdown 不得改写上报门禁");
        drop(blackhole);
    }

    /// 回归断言：多 chunk（4 个 chunk = 2048 条 trace）在黑洞端点下的总耗时
    /// 仍被总预算严格限制在 ≤3250ms，彻底杜绝 N×3000ms 放大。
    #[test]
    fn multi_chunk_blackhole_upload_stays_within_total_flush_budget() {
        let _guard = test_lock();
        shutdown();
        REPORTING_ENABLED.store(false, Ordering::SeqCst);

        let (port, blackhole) = spawn_blackhole_sink(Duration::from_millis(15_000));
        init(OtelConfig {
            endpoint: format!("http://127.0.0.1:{port}"),
            service_name: "unit-multi-chunk-probe".to_string(),
            sampling_ratio: 1.0,
            headers: Vec::new(),
        })
        .expect("回环端点必须可装配");

        let traces: Vec<crate::agent::session::TurnTraceRecord> = (0..2048)
            .map(|index| crate::agent::session::TurnTraceRecord {
                turn_id: format!("turn-probe-4chunk-{index}"),
                session_id: Some("sess-probe-4chunk".to_string()),
                title: "probe".to_string(),
                phase: "completed".to_string(),
                ..Default::default()
            })
            .collect();

        let started = std::time::Instant::now();
        let result = force_export_session_trace("sess-probe-4chunk", &traces);
        let elapsed = started.elapsed();

        assert!(
            result.is_err(),
            "黑洞端点下多块导出必须返回 Err：{result:?}"
        );
        assert!(
            elapsed <= Duration::from_millis(3_250),
            "多 chunk 导出总耗时必须 ≤3000ms(+容差)，实测 {elapsed:?}"
        );
        assert!(
            elapsed >= Duration::from_millis(2_000),
            "总预算倒计时必须切实执行，实测 {elapsed:?}"
        );

        shutdown();
        drop(blackhole);
    }

    // ------------------------------- Wave-4.1 L3-A B1 / B1b / A2 实现级行为断言
    //
    // 说明：这是实现级回归（非验收门禁），与 tests/ 下的冻结验收用例互不替代。

    /// B1：门禁关闭（且未 init）时只有「本地相关性」id，导出态 id 必须为 `None`。
    #[test]
    fn b1_local_context_exposes_current_ids_but_no_exported_trace_id() {
        let _guard = test_lock();
        shutdown();
        REPORTING_ENABLED.store(false, Ordering::SeqCst);

        let scope = begin_turn_trace("sess-b1", "turn-b1", 0);
        assert!(
            current_trace_id().is_some(),
            "B1：本地相关性 trace_id 必须存在（日志/traceparent 依赖它）"
        );
        assert!(
            current_traceparent().is_some(),
            "B1：本地上下文仍必须能组装 W3C traceparent"
        );
        assert_eq!(
            exported_trace_id(),
            None,
            "B1：门禁关闭时导出态 trace_id 必须为 None（否则 issue 会带不可检索的假关联）"
        );
        assert_eq!(
            scope.trace_id().is_some(),
            true,
            "本地守卫的 trace_id() 仍应可读（日志用）"
        );
        assert_eq!(
            scope.exported_trace_id(),
            None,
            "B1b：本地守卫的导出态 id 必须为 None"
        );
        drop(scope);
        assert_eq!(current_trace_id(), None, "守卫 Drop 后不得残留活动 turn");
    }

    /// B1 反向：门禁开启 + provider 已装配时，导出态 id 与守卫/本地读取一致。
    #[test]
    fn b1_exported_trace_id_matches_real_span_when_gate_open() {
        let _guard = test_lock();
        shutdown();
        REPORTING_ENABLED.store(true, Ordering::SeqCst);
        init(OtelConfig {
            endpoint: "http://127.0.0.1:9".to_string(),
            service_name: "unit-b1".to_string(),
            sampling_ratio: 1.0,
            headers: Vec::new(),
        })
        .expect("回环端点必须可装配");

        let scope = begin_turn_trace("sess-b1-on", "turn-b1-on", 0);
        let exported = scope.exported_trace_id();
        assert!(
            exported.as_deref().map(|id| id.len()) == Some(32),
            "B1：门禁开启时必须产出 32 位导出态 trace_id，实际 {exported:?}"
        );
        assert_eq!(exported_trace_id(), exported, "全局读取必须等于守卫的导出态 id");
        assert_eq!(current_trace_id(), exported, "导出态与本地读取在此场景应同源");
        drop(scope);

        shutdown();
        REPORTING_ENABLED.store(false, Ordering::SeqCst);
    }

    /// B1b：本地上下文**绝不**作为子 span 的 parent（否则会造出无 root 的孤儿 trace）。
    #[test]
    fn b1b_local_context_never_parents_child_spans() {
        let _guard = test_lock();
        shutdown();
        REPORTING_ENABLED.store(false, Ordering::SeqCst);

        // 1) 门禁关闭时开启 turn ⇒ 本地上下文。
        let scope = begin_turn_trace("sess-b1b", "turn-b1b", 0);
        let local_trace = current_trace_id().expect("本地上下文必须存在");
        assert!(
            active_exported_turn_context().is_none(),
            "B1b：本地上下文不得成为子 span 的父上下文"
        );

        // 2) turn 中途打开开关并装配 provider（模拟用户运行中开启上报）。
        REPORTING_ENABLED.store(true, Ordering::SeqCst);
        init(OtelConfig {
            endpoint: "http://127.0.0.1:9".to_string(),
            service_name: "unit-b1b".to_string(),
            sampling_ratio: 1.0,
            headers: Vec::new(),
        })
        .expect("回环端点必须可装配");
        assert!(
            active_exported_turn_context().is_none(),
            "B1b：门禁翻转后本地登记仍不得充作父上下文"
        );

        // 3) 子 span 必须自成 root（trace_id 与本地伪造 id 不同）。
        let mut child =
            start_child_span("llm.chat", Vec::new()).expect("门禁开启时应能开子 span");
        let child_trace = opentelemetry::trace::Span::span_context(&child)
            .trace_id()
            .to_string();
        assert_ne!(
            child_trace, local_trace,
            "B1b：子 span 不得落在本地上下文这条「无 root」trace 上"
        );
        child.end();

        drop(scope);
        shutdown();
        REPORTING_ENABLED.store(false, Ordering::SeqCst);
    }

    /// A2：嵌套 turn 必须「保存前值 + 精确摘除」，内层结束后恢复外层的上下文。
    #[test]
    fn a2_nested_turns_restore_previous_context() {
        let _guard = test_lock();
        shutdown();
        REPORTING_ENABLED.store(false, Ordering::SeqCst);

        let outer = begin_turn_trace("sess-a2", "turn-outer", 0);
        let outer_trace = current_trace_id().expect("外层 turn 必须有上下文");
        let inner = begin_turn_trace("sess-a2", "turn-inner", 1);
        let inner_trace = current_trace_id().expect("内层 turn 必须有上下文");
        assert_ne!(outer_trace, inner_trace, "内外层必须是不同的 trace");

        drop(inner);
        assert_eq!(
            current_trace_id(),
            Some(outer_trace),
            "A2：内层结束后必须恢复外层上下文（而不是留下空洞）"
        );
        drop(outer);
        assert_eq!(current_trace_id(), None, "A2：全部结束后不得残留登记");
    }

    /// K4.1 反向：无活跃 turn（或 session 为空）时**不得**输出 `langfuse.session.id`。
    #[test]
    fn k41_child_spans_omit_session_id_without_active_turn() {
        let _guard = test_lock();
        shutdown();
        REPORTING_ENABLED.store(false, Ordering::SeqCst);

        // 1) 无活跃 turn ⇒ 不压入该键。
        let mut attributes: Vec<KeyValue> = Vec::new();
        push_active_session_attribute(&mut attributes);
        assert!(
            attributes
                .iter()
                .all(|pair| pair.key.as_str() != ATTR_LANGFUSE_SESSION_ID),
            "无活跃 turn 时不得输出 langfuse.session.id"
        );

        // 2) 活跃 turn 但 session 为空白 ⇒ 同样不输出。
        let scope = begin_turn_trace("   ", "turn-k41-blank", 0);
        let mut attributes: Vec<KeyValue> = Vec::new();
        push_active_session_attribute(&mut attributes);
        assert!(
            attributes
                .iter()
                .all(|pair| pair.key.as_str() != ATTR_LANGFUSE_SESSION_ID),
            "session_id 为空白时不得输出 langfuse.session.id"
        );
        drop(scope);

        // 3) 正常 session ⇒ 输出且值一致。
        let scope = begin_turn_trace("sess-k41-ok", "turn-k41-ok", 0);
        let mut attributes: Vec<KeyValue> = Vec::new();
        push_active_session_attribute(&mut attributes);
        let session_attr = attributes
            .iter()
            .find(|pair| pair.key.as_str() == ATTR_LANGFUSE_SESSION_ID)
            .expect("活跃 turn 必须输出 langfuse.session.id");
        assert_eq!(
            session_attr.value,
            opentelemetry::Value::String("sess-k41-ok".into())
        );
        drop(scope);

        // 4) turn 级导出态 trace_id 可按 turn 回查（跨上下文补偿的基础）。
        assert_eq!(exported_trace_id_for_turn("no-such-turn"), None);
    }

    /// Wave-6 BLOCKING：`capture_turn_failure` 在**跨线程/无活跃 turn 上下文**时
    /// 仍必须上报该 turn 的导出态 trace_id（按 turn_id 精确回查，绝不用本地 id / 绝不丢）。
    #[test]
    fn wave6_capture_turn_failure_uses_turn_scoped_trace_id_across_threads() {
        let _guard = test_lock();
        let _sentry_guard = crate::agent::ponysentry::test_sink_lock();
        shutdown();
        REPORTING_ENABLED.store(true, Ordering::SeqCst);
        init(OtelConfig {
            endpoint: "http://127.0.0.1:9".to_string(),
            service_name: "unit-wave6".to_string(),
            sampling_ratio: 1.0,
            headers: Vec::new(),
        })
        .expect("回环端点必须可装配");
        // PonySentry：dummy token 让上报路径不被 fail-closed 短路（不产生真实外网流量）。
        crate::agent::ponysentry::init(crate::agent::ponysentry::PonySentryConfig {
            client_token: Some("dummy-token".to_string()),
            enabled: true,
            endpoint: "http://127.0.0.1:9".to_string(),
            environment: "test".to_string(),
            release: "test".to_string(),
        });
        crate::agent::ponysentry::enable_turn_failure_capture(true);

        let scope = begin_turn_trace("sess-wave6-x", "turn-wave6-x", 0);
        let expected = exported_trace_id().expect("活跃 turn 必须有导出态 trace_id");
        assert_eq!(exported_trace_id_for_turn("turn-wave6-x"), Some(expected.clone()));

        // 在**另一个线程**（无活跃 turn 登记）触发失败上报。
        let handle = std::thread::spawn(|| {
            assert!(
                exported_trace_id().is_none(),
                "该线程不应有活跃 turn 上下文（模拟真实跨任务场景）"
            );
            crate::agent::ponysentry::capture_turn_failure("turn-wave6-x", "boom", Some("gpt-4o"));
            crate::agent::ponysentry::testing_last_enqueued_payload()
        });
        let payload = handle.join().expect("上报线程不得 panic").expect("必须入队 payload");
        assert_eq!(
            payload.trace_id,
            Some(expected),
            "跨线程上报必须回查到该 turn 的导出态 trace_id"
        );

        // 反向：未知 turn 不得凭空产生 trace_id（禁止伪造）。
        let handle = std::thread::spawn(|| {
            crate::agent::ponysentry::capture_turn_failure("turn-not-registered", "boom", None);
            crate::agent::ponysentry::testing_last_enqueued_payload()
        });
        let payload = handle.join().expect("上报线程不得 panic").expect("必须入队 payload");
        assert_eq!(payload.trace_id, None, "未登记的 turn 不得输出 trace_id");

        crate::agent::ponysentry::enable_turn_failure_capture(false);
        crate::agent::ponysentry::init(crate::agent::ponysentry::PonySentryConfig::default());
        drop(scope);
        shutdown();
        REPORTING_ENABLED.store(false, Ordering::SeqCst);
    }

    /// Wave-7 G5 判据 1：`governance.decide` span 的 **5 项白名单属性真实上 wire**，
    /// 且非白名单属性（如金额/阈值）不得出现。
    #[test]
    fn g5_governance_decide_span_exports_whitelisted_attributes_to_wire() {
        let _guard = test_lock();
        shutdown();
        let (port, sink) = spawn_local_otlp_sink(Duration::from_millis(1_500));
        init(OtelConfig {
            endpoint: format!("http://127.0.0.1:{port}"),
            service_name: "unit-governance".to_string(),
            sampling_ratio: 1.0,
            headers: Vec::new(),
        })
        .expect("回环端点必须可装配");
        REPORTING_ENABLED.store(true, Ordering::SeqCst);

        {
            let _turn = begin_turn_trace("sess-gov-wire", "turn-gov-wire", 0);
            let mut span = start_governance_decide_span();
            span.record_verdict(
                "session",
                "block",
                Some("hard_limit_reached"),
                "abc123def456",
                3,
            );
        }

        let flushed = force_flush(3_000);
        assert!(flushed.is_ok(), "flush 必须成功：{flushed:?}");
        shutdown();
        REPORTING_ENABLED.store(false, Ordering::SeqCst);

        let wire = sink.join().expect("sink 线程不得 panic");
        let text = String::from_utf8_lossy(&wire).to_string();
        for attribute in [
            ATTR_GOVERNANCE_WINDOW,
            ATTR_GOVERNANCE_DECISION,
            ATTR_GOVERNANCE_REASON,
            ATTR_GOVERNANCE_POLICY_VERSION,
            ATTR_GOVERNANCE_UNPRICED_TURNS,
        ] {
            assert!(
                text.contains(attribute),
                "G5：wire payload 缺少白名单属性 `{attribute}`"
            );
        }
        assert!(text.contains("hard_limit_reached"), "reason 取值必须上 wire");
        assert!(text.contains("abc123def456"), "policy_version 取值必须上 wire");
        for forbidden in ["governance.limit_usd", "governance.current_usd", "governance.cost_usd"] {
            assert!(
                !text.contains(forbidden),
                "G5：非白名单属性 `{forbidden}` 不得出现在 wire payload"
            );
        }
    }

    /// A8：`traceparent` 注入必须是「替换」语义，重复注入不得产生两个同名头。
    #[test]
    fn a8_traceparent_injection_replaces_instead_of_appending() {
        let _guard = test_lock();
        shutdown();
        REPORTING_ENABLED.store(false, Ordering::SeqCst);

        let scope = begin_turn_trace("sess-a8", "turn-a8", 0);
        let expected = current_traceparent().expect("本地上下文应可组装 traceparent");

        let request = reqwest::Client::new()
            .get("http://127.0.0.1:9/probe")
            .header("traceparent", "00-00000000000000000000000000000000-0000000000000000-00");
        let request = with_current_traceparent(with_current_traceparent(request));
        let built = request.build().expect("请求必须可构建");
        let values: Vec<&reqwest::header::HeaderValue> =
            built.headers().get_all("traceparent").iter().collect();
        assert_eq!(
            values.len(),
            1,
            "A8：traceparent 必须至多一个（预防重复头），实际 {}",
            values.len()
        );
        assert_eq!(
            values[0].to_str().expect("ASCII header"),
            expected,
            "A8：注入值必须覆盖既有头"
        );

        drop(scope);
    }

    /// A6-adjacent：settings 快照的 Debug 必须打码 token（L3-A #6）。
    #[test]
    fn otel_settings_snapshot_debug_redacts_token() {
        let snapshot = OtelSettingsSnapshot {
            endpoint: Some("https://otel.example".to_string()),
            token: Some("SUPER_SECRET_OTEL_TOKEN_123".to_string()),
            service_name: Some("svc".to_string()),
            sampling_ratio: Some(0.5),
        };
        let shown = format!("{snapshot:?}");
        assert!(
            !shown.contains("SUPER_SECRET_OTEL_TOKEN_123"),
            "L3-A#6：Debug 不得泄漏 token：{shown}"
        );
        assert!(shown.contains("[REDACTED]"), "必须显式标出脱敏位：{shown}");
    }

    #[test]
    fn test_resolve_resource_attributes_probe_marking() {
        let attrs = resolve_resource_attributes(|k| match k {
            ENV_PONY_OTEL_PROBE => Some("true".to_string()),
            _ => None,
        });
        assert!(attrs.iter().any(|(k, v)| k == RESOURCE_PONY_PROBE && v == "true"));
        assert!(attrs.iter().any(|(k, v)| k == RESOURCE_INGEST_CLIENT_KIND && v == INGEST_CLIENT_KIND_PROBE));
        assert!(attrs.iter().any(|(k, v)| k == RESOURCE_PONY_APP_NAME && v == PONY_APP_NAME));

        let attrs_off = resolve_resource_attributes(|k| match k {
            ENV_PONY_OTEL_PROBE => Some("false".to_string()),
            _ => None,
        });
        assert!(!attrs_off.iter().any(|(k, _)| k == RESOURCE_PONY_PROBE));
        assert!(attrs_off.iter().any(|(k, v)| k == RESOURCE_INGEST_CLIENT_KIND && v == INGEST_CLIENT_KIND_DEVICE));
        assert!(attrs_off.iter().any(|(k, v)| k == RESOURCE_PONY_APP_NAME && v == PONY_APP_NAME));
    }

    /// P1 & P2 单测：三种环境来源优先级判定，以及根 span usage/cost 聚合。
    #[test]
    fn test_deployment_environment_precedence() {
        // 1. env 显式覆盖优先
        let attrs_env = resolve_resource_attributes(|k| match k {
            ENV_PONY_OTEL_ENVIRONMENT => Some("staging".to_string()),
            _ => None,
        });
        assert_eq!(
            attrs_env.iter().find(|(k, _)| k == RESOURCE_DEPLOYMENT_ENVIRONMENT).map(|(_, v)| v.as_str()),
            Some("staging")
        );

        // 2. 无 env 时根据 default_deployment_environment
        let env_default = default_deployment_environment(false);
        // 本地开发测试环境无 PONY_RELEASE_BUILD，即使 debug_assertions 为 false 也判 development
        assert_eq!(env_default, "development");
    }

    #[test]
    fn test_turn_root_span_aggregates_usage_and_cost() {
        let _guard = test_lock();
        shutdown();
        REPORTING_ENABLED.store(true, Ordering::SeqCst);

        init(OtelConfig {
            endpoint: "http://127.0.0.1:9".to_string(),
            service_name: "test-usage-agg".to_string(),
            sampling_ratio: 1.0,
            headers: Vec::new(),
        })
        .expect("init");

        // 1. 有 usage 与 cost 场景
        {
            let mut turn = begin_turn_trace("sess-usage", "turn-usage-1", 0);
            {
                let mut llm1 = start_llm_span("openai", Some("gpt-4o"), "chat");
                llm1.record_usage(Some(100), Some(25));
                llm1.record_cost(Some(0.005));
            }
            {
                let mut llm2 = start_llm_span("openai", Some("gpt-4o"), "chat");
                llm2.record_usage(Some(50), Some(15));
                llm2.record_cost(Some(0.003));
            }
            turn.set_phase("completed");
            // Drop turn guard 触发聚合回填
        }

        // 2. 无 usage 与 cost 场景（不输出对应属性键）
        {
            let mut turn = begin_turn_trace("sess-usage-none", "turn-usage-none", 0);
            turn.set_phase("completed");
        }

        shutdown();
    }

    #[test]
    fn test_export_diagnostics_uninit_returns_none() {
        let _guard = test_lock();
        shutdown();
        assert!(export_diagnostics().is_none());
    }

    #[test]
    fn test_export_diagnostics_with_and_without_auth() {
        let _guard = test_lock();
        shutdown();

        // 1. 无 auth_token 初始化
        init(OtelConfig {
            endpoint: "https://collector.example.com".to_string(),
            service_name: "test-svc".to_string(),
            sampling_ratio: 1.0,
            headers: Vec::new(),
        })
        .expect("init without auth should succeed");

        let diag = export_diagnostics().expect("diagnostics should exist after init");
        assert_eq!(diag.endpoint, "https://collector.example.com");
        assert!(!diag.auth_configured, "无鉴权头时 auth_configured 必须为 false");

        shutdown();

        // 2. 带 auth_token 初始化
        init(OtelConfig {
            endpoint: "https://collector.example.com".to_string(),
            service_name: "test-svc".to_string(),
            sampling_ratio: 1.0,
            headers: vec![("authorization".to_string(), "Bearer secret-123".to_string())],
        })
        .expect("init with auth should succeed");

        let diag = export_diagnostics().expect("diagnostics should exist after init");
        assert_eq!(diag.endpoint, "https://collector.example.com");
        assert!(diag.auth_configured, "带 authorization 头时 auth_configured 必须为 true");

        shutdown();
    }

    /// 黑洞 OTLP 端点：只 accept、不回包（导出必然超时），held 到预算耗尽。
    fn spawn_blackhole_sink(budget: Duration) -> (u16, std::thread::JoinHandle<usize>) {
        let listener =
            std::net::TcpListener::bind("127.0.0.1:0").expect("黑屏端点必须能绑定回环端口");
        listener.set_nonblocking(true).expect("黑屏端点必须非阻塞");
        let port = listener.local_addr().expect("blackhole local_addr").port();

        let handle = std::thread::spawn(move || {
            let mut held: Vec<std::net::TcpStream> = Vec::new();
            let deadline = std::time::Instant::now() + budget;
            while std::time::Instant::now() < deadline {
                match listener.accept() {
                    Ok((stream, _peer)) => held.push(stream),
                    Err(_) => std::thread::sleep(Duration::from_millis(5)),
                }
            }
            held.len()
        });

        (port, handle)
    }

    // ------------------------------------------------ PA-153 · resource 归因（实现期单测）

    /// PA-153：默认值 = crate 版本 + debug 构建 `development` + 稳定产品名 + 默认 client.kind=device。
    #[test]
    fn resource_attributes_default_to_crate_version_and_build_environment() {
        let attrs = resolve_resource_attributes(|_| None);
        assert_eq!(attrs.len(), 4, "resource 归因包含四键：{attrs:?}");
        assert_eq!(attrs[0].0, "service.version");
        assert_eq!(attrs[0].1, env!("CARGO_PKG_VERSION"));
        assert_eq!(attrs[1].0, "deployment.environment");
        assert_eq!(
            attrs[1].1,
            default_deployment_environment(cfg!(debug_assertions))
        );
        assert_eq!(attrs[2].0, "pony.app.name");
        assert_eq!(attrs[2].1, "pony-agent");
        assert_eq!(attrs[3].0, "ingest.client.kind");
        assert_eq!(attrs[3].1, INGEST_CLIENT_KIND_DEVICE);
    }

    /// PA-153：两个 env（trim 后非空）各自覆盖默认，且互不影响。
    #[test]
    fn resource_attributes_env_overrides_are_independent() {
        let attrs = resolve_resource_attributes(|name| match name {
            "PONY_OTEL_SERVICE_VERSION" => Some("1.2.3-custom".to_string()),
            "PONY_OTEL_ENVIRONMENT" => Some("staging".to_string()),
            _ => None,
        });
        assert_eq!(attrs[0].1, "1.2.3-custom");
        assert_eq!(attrs[1].1, "staging");
    }

    /// PA-153：空白串（含仅 trim 后为空）等同未设置 ⇒ 回落默认；非空值 trim 后生效。
    #[test]
    fn resource_attributes_blank_env_falls_back_to_defaults() {
        for blank in ["", "   ", "\t\n "] {
            let attrs = resolve_resource_attributes(|name| match name {
                "PONY_OTEL_SERVICE_VERSION" | "PONY_OTEL_ENVIRONMENT" => Some(blank.to_string()),
                _ => None,
            });
            assert_eq!(attrs[0].1, env!("CARGO_PKG_VERSION"), "空白串 `{blank:?}` 必须回落默认版本");
            assert_eq!(
                attrs[1].1,
                default_deployment_environment(cfg!(debug_assertions)),
                "空白串 `{blank:?}` 必须回落默认环境"
            );
        }
        let padded = resolve_resource_attributes(|name| match name {
            "PONY_OTEL_SERVICE_VERSION" => Some("  9.9.9  ".to_string()),
            _ => None,
        });
        assert_eq!(padded[0].1, "9.9.9", "env 取值必须 trim 后生效");
    }

    /// PA-153 / task-13：release 标记判定（PONY_RELEASE_BUILD=1 为 production，否则一律 development）。
    #[test]
    fn default_deployment_environment_switches_on_build_profile() {
        assert_eq!(default_deployment_environment_with_release_flag(true, None), "development");
        assert_eq!(default_deployment_environment_with_release_flag(false, None), "development");
        assert_eq!(default_deployment_environment_with_release_flag(false, Some("1")), "production");
        assert_eq!(default_deployment_environment_with_release_flag(true, Some("1")), "production");
        assert_eq!(default_deployment_environment_with_release_flag(false, Some("true")), "production");
    }

    // -------------------------------------------- PA-154 · turn 错误聚合（实现期单测）

    /// PA-154：聚合先记录者胜、取走即清零；无活动 turn 时 mark 为 no-op。
    #[test]
    fn turn_error_aggregation_first_wins_and_take_clears() {
        let _guard = test_lock();
        // 无活动 turn：mark / take 都必须是安全 no-op。
        mark_active_turn_error(TURN_ERROR_PREFIX_TOOL_FAILED);
        assert_eq!(take_active_turn_error(), None, "无活动 turn 时不得产生聚合");

        let slot = register_active_turn("turn-err", "sess-err", span_context(0x2000), true);
        mark_active_turn_error(TURN_ERROR_PREFIX_TOOL_FAILED);
        mark_active_turn_error(TURN_ERROR_PREFIX_QUOTA_BLOCKED);
        assert_eq!(
            take_active_turn_error().as_deref(),
            Some("tool_failed"),
            "双错误并存时必须保留先记录者"
        );
        assert_eq!(take_active_turn_error(), None, "取走后必须清零（跨 turn 不污染）");
        unregister_active_turn(slot);
    }

    /// PA-154：根 span 文案必须是 `turn_error: <reason>`（与子 span 原始文案可区分）。
    #[test]
    fn turn_error_status_message_uses_turn_error_prefix() {
        assert_eq!(
            turn_error_status_message(TURN_ERROR_PREFIX_TOOL_FAILED),
            "turn_error: tool_failed"
        );
        assert_eq!(
            turn_error_status_message(TURN_ERROR_PREFIX_QUOTA_BLOCKED),
            "turn_error: quota_blocked"
        );
        // 值契约：`agent.turn.outcome` 恒定取值 `error`（键名/值均须与冻结字面量一致）。
        assert_eq!(ATTR_AGENT_TURN_OUTCOME, "agent.turn.outcome");
        assert_eq!(TURN_ERROR_PREFIX_TOOL_FAILED, "tool_failed");
        assert_eq!(TURN_ERROR_PREFIX_QUOTA_BLOCKED, "quota_blocked");
    }

    /// wave-49 回归：构建期烧录 token 是**最低优先级**兜底 —— 用户零配置也能上报，
    /// 但绝不覆盖 settings/env 的显式配置。
    /// 守护的线上故障：桌面端无 token 分发通道 ⇒ 用户不手填 Settings 就静默 401、trace 全丢。
    #[test]
    fn build_time_token_is_lowest_priority_fallback() {
        // ① settings 与 env 都没给 ⇒ 用构建期值
        let with_build = resolve_config_with_build_token(None, None, Some("build-token-xyz"));
        assert_eq!(
            with_build
                .headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(AUTH_HEADER_NAME))
                .map(|(_, value)| value.as_str()),
            Some("Bearer build-token-xyz"),
            "无 settings/env 时必须回落到构建期 token"
        );

        // ② env 给了 ⇒ 构建期值不得覆盖
        let env_pairs = [(ENV_PONY_OTEL_TOKEN, "env-token-abc")];
        let with_env = resolve_config_with_build_token(None, Some(&env_pairs), Some("build-token-xyz"));
        assert_eq!(
            with_env
                .headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(AUTH_HEADER_NAME))
                .map(|(_, value)| value.as_str()),
            Some("Bearer env-token-abc"),
            "env 显式配置必须压过构建期 token"
        );

        // ③ 构建期值为空/空白 ⇒ 视为未烧录（零凭证，fail-closed）
        for blank in ["", "   "] {
            let config = resolve_config_with_build_token(None, None, Some(blank));
            assert!(
                config.headers.is_empty(),
                "空白构建期 token 必须视为未配置（{blank:?}）"
            );
        }

        // ④ 完全没有构建期 token（本地 cargo build 的常态）⇒ 仍然零凭证
        let none = resolve_config_with_build_token(None, None, None);
        assert!(none.headers.is_empty(), "无构建期 token 时不得凭空产生凭证");
    }

    /// T11 回归：工具 span 的时间边界必须覆盖真实执行区间；退化/溢出路径不得 panic。
    #[test]
    fn tool_span_bounds_covers_execution_interval() {
        let now = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);

        // 正常路径：end - start 精确等于 duration_ms（这正是 0.002ms → 829ms 的修复点）。
        let (start, end) = tool_span_bounds(now, Some(829));
        assert_eq!(end, now, "end 必须是采集时刻");
        assert_eq!(
            end.duration_since(start).expect("end 必须晚于 start"),
            std::time::Duration::from_millis(829),
            "工具 span 必须覆盖真实执行区间（修复前恒为 0 时长）"
        );

        // 零耗时：唯一允许的零长 span。
        let (start, end) = tool_span_bounds(now, Some(0));
        assert_eq!((start, end), (now, now), "duration=0 必须 start == end");

        // 耗时未知：退化为零长 span（不得凭空造时长、不得 panic）。
        let (start, end) = tool_span_bounds(now, None);
        assert_eq!((start, end), (now, now), "duration=None 必须退化为 start == end");

        // 极大值：异常大 duration 必须退化为零长 span（不得 panic）。
        // 说明：Linux 的 SystemTime 允许负值，`u64::MAX` 毫秒并不会让 checked_sub 失败，
        // 因此这里额外有跨平台确定的上限（TOOL_SPAN_MAX_BACKDATE_MS = 24h）。
        let (start, end) = tool_span_bounds(now, Some(u64::MAX));
        assert_eq!((start, end), (now, now), "超上限必须退化为 start == end");
        let (start, end) = tool_span_bounds(now, Some(TOOL_SPAN_MAX_BACKDATE_MS + 1));
        assert_eq!((start, end), (now, now), "刚超上限也必须退化为 start == end");
        let (start, end) = tool_span_bounds(now, Some(TOOL_SPAN_MAX_BACKDATE_MS));
        assert_eq!(
            end.duration_since(start).expect("end 必须晚于 start"),
            std::time::Duration::from_millis(TOOL_SPAN_MAX_BACKDATE_MS),
            "上限内的长耗时必须完整保留（不得误伤长任务）"
        );

        // 秒级量级同样精确（证明不是毫秒量级的巧合）。
        let (start, end) = tool_span_bounds(now, Some(1_000));
        assert_eq!(
            end.duration_since(start).expect("end 必须晚于 start"),
            std::time::Duration::from_secs(1)
        );
    }
}
