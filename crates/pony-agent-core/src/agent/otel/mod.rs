//! Agent Trace 的 OpenTelemetry 装配与门禁面（Wave43 冻结空桩，实现归 backlog B2）。
//!
//! 本模块是 trace 上报的**唯一**契约面：只含类型、常量与函数签名，全部函数体为
//! `unimplemented!`。任何 OTel SDK 类型都不得出现在本模块的公开签名里，以保证契约
//! 可被无网络、无全局态的单测直接驱动。
//!
//! 隐私门禁总契约：全局上报开关默认**关闭**；关闭态必须做到**零 span 分配、零网络出口**。
//! 手动上传（[`force_export_session_trace`]）是逐次授权的旁路动作，不受该开关约束。

// 空桩期专用：签名参数在 `unimplemented!` 体内不可达，实现落地（B2）时必须删除本行。
#![allow(unused_variables)]

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

/// OTLP/HTTP 端点缺省值（集群内 collector，4318 端口）。
pub const DEFAULT_ENDPOINT: &str = "http://otel-collector.observability.svc.cluster.local:4318";

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
#[derive(Clone, Debug, PartialEq)]
pub struct OtelConfig {
    /// OTLP/HTTP 端点；缺省 [`DEFAULT_ENDPOINT`]。
    pub endpoint: String,
    /// `service.name`；缺省 [`DEFAULT_SERVICE_NAME`]。
    pub service_name: String,
    /// 采样率（0.0~1.0）；缺省 [`DEFAULT_SAMPLING_RATIO`]。
    pub sampling_ratio: f64,
    /// 额外导出 header（名称, 值）；缺省为空（鉴权 token 由 [`ENV_PONY_OTEL_TOKEN`] 注入）。
    pub headers: Vec<(String, String)>,
}

impl Default for OtelConfig {
    /// 契约（缺省语义）：`endpoint` = [`DEFAULT_ENDPOINT`]、`service_name` =
    /// [`DEFAULT_SERVICE_NAME`]、`sampling_ratio` = [`DEFAULT_SAMPLING_RATIO`]、
    /// `headers` 为空。
    fn default() -> Self {
        unimplemented!("Wave43 空桩：OtelConfig::default");
    }
}

/// 装配入口：安装全局 trace provider。
///
/// 调用方：宿主启动路径（Tauri 装配点），进程生命周期内**至多一次**。
/// 语义：以 `config` 建立导出通路与资源属性；provider 常驻，开关只翻转门禁位。
/// 失败语义：配置非法或端点不可解析时返回 `Err`，调用方记录告警后**继续启动**，
/// 不得 panic、不得阻断主链路。
/// 可重入：本函数返回 `Ok` 后，需先 [`shutdown`] 才能再次 [`init`]。
pub fn init(config: OtelConfig) -> Result<(), String> {
    unimplemented!("Wave43 空桩：otel::init");
}

/// 装配逆过程：flush 并释放 provider 及其后台资源。
///
/// 调用方：宿主退出路径（Tauri 退出事件），必须显式调用一次。
/// 语义：幂等，可重复调用；返回后允许再次 [`init`]。
/// 失败语义：不返回错误——任何内部异常都必须在内部吞掉并降级为日志，绝不 panic。
pub fn shutdown() {
    unimplemented!("Wave43 空桩：otel::shutdown");
}

/// 把当前已入队 span 强制导出，等待至多 `timeout_ms` 毫秒。
///
/// 调用方：手动上传旁路与进程退出前的兜底 flush；测试用它取得确定性导出时序。
/// 语义：未调用过 [`init`] 时返回 `Err`；超时视为失败但仍返回（不 panic、不阻塞调用方）。
/// 失败语义：`Err(String)` 描述失败原因，调用方只记日志，不得冒泡到 turn 主链路。
pub fn force_flush(timeout_ms: u64) -> Result<(), String> {
    unimplemented!("Wave43 空桩：otel::force_flush");
}

/// 翻转全局上报开关。
///
/// 调用方：宿主设置读取/保存路径（启动时按持久化值回填 + 用户改开关即时生效）。
/// 语义：`false` 时不得分配 span、不得产生任何网络出口；`true` 时按采样率上报。
/// 幂等：并发/重复调用不得 panic，末态等于最后一次写入值。
pub fn set_reporting_enabled(enabled: bool) {
    unimplemented!("Wave43 空桩：otel::set_reporting_enabled");
}

/// 读取全局上报开关当前值。
///
/// 调用方：设置页回显与门禁判定的只读查询；未 [`init`] 时返回 `false`。
/// 语义：反映最近一次 [`set_reporting_enabled`] 的结果，不做任何副作用。
pub fn is_reporting_enabled() -> bool {
    unimplemented!("Wave43 空桩：otel::is_reporting_enabled");
}

/// 回显当前生效配置。
///
/// 调用方：设置页/诊断页展示与配置透传断言；未 [`init`] 时返回 `None`。
/// 语义：返回的采样率等字段必须等于实际生效值（env 覆盖后的值，而非常量）。
pub fn config() -> Option<OtelConfig> {
    unimplemented!("Wave43 空桩：otel::config");
}

/// 按环境变量组装配置。
///
/// 调用方：[`init`] 的宿主侧调用点。优先级：环境变量 > 编译期缺省值
/// （[`ENV_OTLP_ENDPOINT`] / [`ENV_SERVICE_NAME`] / [`ENV_SAMPLER_ARG`]）；
/// [`ENV_PONY_OTEL_TOKEN`] 存在时以 [`AUTH_HEADER_NAME`] header 注入 headers。
/// 语义：不读写全局状态、不发起网络 IO，可被单测直接调用。
pub fn from_env() -> OtelConfig {
    unimplemented!("Wave43 空桩：otel::from_env");
}

/// turn 级 span 守卫。
///
/// 契约：由 [`TurnScope::new`] 构造（外部不可字面量构造），生命周期覆盖一个 turn；
/// 必须显式调用 [`TurnScope::end`] 结束 span（本类型不实现 `Drop`）。
/// 全局开关关闭时，构造出的守卫等价空操作（零分配零导出）。
pub struct TurnScope {
    _opaque: (),
}

impl TurnScope {
    /// 开启一个 turn 级 span 并返回守卫。
    ///
    /// 调用方：turn 终态路径；`session_id` / `turn_id` 须与本地 trace 记录一致，
    /// `sequence` 为该 turn 在会话内的序号。
    /// 语义：span 名为 [`SPAN_AGENT_TURN`]，属性由 [`TurnSpanInput`] 描述。
    /// 失败语义：任何情况下都不得 panic；开关关闭时返回空操作守卫。
    pub fn new(session_id: &str, turn_id: &str, sequence: u64) -> Self {
        unimplemented!("Wave43 空桩：TurnScope::new");
    }

    /// 读取该 turn 的 trace_id（十六进制字符串）。
    ///
    /// 调用方：日志面与 issue 上报的 `tags.trace_id` 回填。
    /// 语义：未采样、未 [`init`] 或守卫已结束时返回 `None`。
    pub fn trace_id(&self) -> Option<String> {
        unimplemented!("Wave43 空桩：TurnScope::trace_id");
    }

    /// 结束 span 并释放守卫。
    ///
    /// 语义：消费 `self`，重复结束不可能发生；不得 panic。
    pub fn end(self) {
        unimplemented!("Wave43 空桩：TurnScope::end");
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
    unimplemented!("Wave43 空桩：otel::force_export_session_trace");
}

/// turn 属性映射（纯函数，无网络、无全局态）。
///
/// 调用方：[`TurnScope::new`] 的 span 属性填充，以及单测直接断言属性全集。
/// 语义：按 [`ATTR_LANGFUSE_SESSION_ID`] / [`ATTR_LANGFUSE_TRACE_NAME`] /
/// [`ATTR_GEN_AI_OPERATION_NAME`]（[`OPERATION_AGENT_TURN`]）/ [`ATTR_AGENT_TURN_ID`] /
/// [`ATTR_AGENT_TURN_SEQUENCE`] / [`ATTR_AGENT_TURN_PHASE`] 输出，`user_id` 非空时
/// 才输出 [`ATTR_LANGFUSE_USER_ID`]；**缺失值不输出键**（禁止空串占位）。
pub fn turn_span_attributes(input: &TurnSpanInput) -> Vec<(String, AttrValue)> {
    unimplemented!("Wave43 空桩：otel::turn_span_attributes");
}

/// LLM 属性映射（纯函数，无网络、无全局态）。
///
/// 调用方：LLM 子 span 的属性填充与单测断言。
/// 语义：恒输出 [`ATTR_GEN_AI_SYSTEM`] 与 [`ATTR_GEN_AI_OPERATION_NAME`]；
/// `model` 非空时同时输出 [`ATTR_GEN_AI_REQUEST_MODEL`] 与
/// [`ATTR_LANGFUSE_OBSERVATION_MODEL_NAME`]（同值双写）；token 字段各自非空才输出。
pub fn llm_span_attributes(input: &LlmSpanInput) -> Vec<(String, AttrValue)> {
    unimplemented!("Wave43 空桩：otel::llm_span_attributes");
}

/// 工具属性映射（纯函数，无网络、无全局态）。
///
/// 调用方：工具子 span 的属性填充与单测断言。
/// 语义：恒输出 [`ATTR_GEN_AI_TOOL_NAME`]、[`ATTR_LANGFUSE_OBSERVATION_TYPE`]
/// （[`OBSERVATION_TYPE_TOOL`]）、[`ATTR_AGENT_TOOL_STATUS`]；`duration_ms` 与
/// `error` 各自非空才输出。
pub fn tool_span_attributes(input: &ToolSpanInput) -> Vec<(String, AttrValue)> {
    unimplemented!("Wave43 空桩：otel::tool_span_attributes");
}
