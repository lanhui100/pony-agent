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

/// 模型生成速度（t/s）。
pub const ATTR_GEN_AI_SPEED_TOKENS_PER_SECOND: &str = "gen_ai.speed.tokens_per_second";

/// 上下文窗口总大小（tokens）。
pub const ATTR_GEN_AI_USAGE_CONTEXT_WINDOW: &str = "gen_ai.usage.context_window";

/// 上下文使用比例（0.0 ~ 1.0）。
pub const ATTR_GEN_AI_USAGE_CONTEXT_RATIO: &str = "gen_ai.usage.context_ratio";

/// 整轮综合生成速度（t/s）。
pub const ATTR_AGENT_TURN_SPEED_TOKENS_PER_SECOND: &str = "agent.turn.speed.tokens_per_second";

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
#[derive(Clone, PartialEq)]
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
    /// 契约（缺省语义）：`endpoint` = [`DEFAULT_ENDPOINT`]、`service_name` =
    /// [`DEFAULT_SERVICE_NAME`]、`sampling_ratio` = [`DEFAULT_SAMPLING_RATIO`]、
    /// `headers` 为空。
    fn default() -> Self {
        Self {
            endpoint: DEFAULT_ENDPOINT.to_string(),
            service_name: DEFAULT_SERVICE_NAME.to_string(),
            sampling_ratio: DEFAULT_SAMPLING_RATIO,
            headers: Vec::new(),
        }
    }
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
    *guard = Some(state);
    Ok(())
}

/// 装配逆过程：flush 并释放 provider 及其后台资源。
///
/// 调用方：宿主退出路径（Tauri 退出事件），必须显式调用一次。
/// 语义：幂等，可重复调用；返回后允许再次 [`init`]。
/// 失败语义：不返回错误——任何内部异常都必须在内部吞掉并降级为日志，绝不 panic。
pub fn shutdown() {
    let state = take_state();
    let Some(state) = state else {
        return;
    };

    let provider = state.provider.clone();
    match provider.shutdown_with_timeout(Duration::from_millis(SHUTDOWN_TIMEOUT_MS)) {
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

/// 宿主设置桥接：把持久化的 `AppSettings.agent_trace_reporting` 同步到 OTel 门禁。
///
/// 调用方：Tauri 侧 `load_app_settings` / `save_app_settings`（D7：启动即按持久化值生效，
/// 保存即生效）。语义：等价于 [`set_reporting_enabled`]，仅为宿主提供语义明确的入口，
/// 便于静态断言「设置读写双路径都做了同步」。
pub fn set_agent_trace_reporting(enabled: bool) {
    set_reporting_enabled(enabled);
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

/// 按环境变量组装配置。
///
/// 调用方：[`init`] 的宿主侧调用点。优先级：环境变量 > 编译期缺省值
/// （[`ENV_OTLP_ENDPOINT`] / [`ENV_SERVICE_NAME`] / [`ENV_SAMPLER_ARG`]）；
/// [`ENV_PONY_OTEL_TOKEN`] 存在时以 [`AUTH_HEADER_NAME`] header 注入 headers。
/// 语义：不读写全局状态、不发起网络 IO，可被单测直接调用。
pub fn from_env() -> OtelConfig {
    let mut config = OtelConfig::default();

    if let Some(endpoint) = env_non_empty(ENV_OTLP_ENDPOINT) {
        config.endpoint = endpoint;
    }
    if let Some(service_name) = env_non_empty(ENV_SERVICE_NAME) {
        config.service_name = service_name;
    }
    if let Some(raw_ratio) = env_non_empty(ENV_SAMPLER_ARG) {
        match raw_ratio.trim().parse::<f64>() {
            Ok(ratio) if ratio.is_finite() && (0.0..=1.0).contains(&ratio) => {
                config.sampling_ratio = ratio;
            }
            _ => log::log_json(
                "warn",
                &format!(
                    "otel::from_env：{ENV_SAMPLER_ARG} 取值非法，已回落缺省采样率 {DEFAULT_SAMPLING_RATIO}：{raw_ratio}"
                ),
                None,
            ),
        }
    }
    if let Some(token) = env_non_empty(ENV_PONY_OTEL_TOKEN) {
        config.headers.push((
            AUTH_HEADER_NAME.to_string(),
            authorization_header_value(&token),
        ));
    }

    config
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
    let mut delivered: usize = 0;
    for chunk in traces.chunks(FORCE_EXPORT_CHUNK_TURNS) {
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
        flush_state(&state, FORCE_EXPORT_FLUSH_TIMEOUT_MS)
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
pub fn begin_turn_trace(session_id: &str, turn_id: &str, sequence: u64) -> TurnTraceGuard {
    let empty = TurnTraceGuard {
        span: None,
        slot: None,
        phase: None,
        trace_id: None,
        turn_speed: None,
    };
    // 契约：session_id / turn_id 缺失（空串）⇒ **不创建** turn span。
    // 绝不用占位值伪造会话——那会在 Langfuse 侧造出不存在的 session，污染数据。
    if session_id.trim().is_empty() || turn_id.trim().is_empty() {
        return empty;
    }
    if !spans_allowed() {
        return empty;
    }
    let Some(state) = current_state() else {
        return empty;
    };

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
    let slot = register_active_turn(turn_id, span.span_context().clone());
    let trace_id = sampled_trace_id(&mut span);

    TurnTraceGuard {
        span: Some(span),
        slot,
        phase: None,
        trace_id: None,
        turn_speed: None,
    }
    .with_trace_id(trace_id)
}

/// turn trace 守卫（`Drop` 时补 phase 属性并结束 span）。
pub struct TurnTraceGuard {
    span: Option<opentelemetry_sdk::trace::Span>,
    /// 该 turn 的登记归属（task id 或线程本地栈）；Drop 时精确摘除自己那一条。
    slot: Option<ActiveTurnSlot>,
    phase: Option<String>,
    trace_id: Option<String>,
    turn_speed: Option<f64>,
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

    /// 读取该 turn 的 trace_id（十六进制字符串）；未采样/门禁关闭/未 init 时 `None`。
    pub fn trace_id(&self) -> Option<String> {
        self.trace_id.clone()
    }
}

impl Drop for TurnTraceGuard {
    fn drop(&mut self) {
        if let Some(phase) = self.phase.take() {
            if let Some(span) = self.span.as_mut() {
                if phase == "failed" {
                    span.set_status(opentelemetry::trace::Status::error(phase.clone()));
                }
                span.set_attribute(KeyValue::new(ATTR_AGENT_TURN_PHASE, phase));
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
    };
    if !spans_allowed() {
        return empty;
    }
    let attributes = to_key_values(llm_span_attributes(&LlmSpanInput {
        provider_kind: provider_kind.to_string(),
        model: model.map(|value| value.to_string()),
        input_tokens: None,
        output_tokens: None,
        operation: operation.to_string(),
    }));
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
}

impl LlmSpanGuard {
    /// 记录响应侧 token 用量（`None` 表示缺失 → 不输出该键）。
    pub fn record_usage(&mut self, input_tokens: Option<u64>, output_tokens: Option<u64>) {
        self.input_tokens = input_tokens;
        self.output_tokens = output_tokens;
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
    let attributes = to_key_values(tool_span_attributes(input));
    if let Some(mut span) = start_child_span(SPAN_TOOL_CALL, attributes) {
        if input.status == "failed" || input.status == "error" || input.error.is_some() {
            let err_desc = input.error.as_deref().unwrap_or("tool call failed");
            span.set_status(opentelemetry::trace::Status::error(err_desc.to_string()));
        }
        span.end();
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
use std::time::Duration;

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
/// 关闭 provider 时的导出等待上限。
const SHUTDOWN_TIMEOUT_MS: u64 = 3_000;
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

/// 活动 turn 索引：`tokio::task::Id` → `(turn_id, 该 turn root span 的 SpanContext)`。
///
/// F5（跨 turn 污染）：宿主按 turn 并发 spawn，若用「栈顶」选父 span，两个 turn 同时
/// 在跑时 A 的 llm/tool span 会被挂到 B 的 trace 上。改为**按当前 tokio 任务 id 索引**：
/// 守卫登记自己的 task id，Drop 时只摘除自己那条；无 task id（非 tokio 任务上下文）
/// 时不加 parent —— 宁可让子 span 自成 root，也绝不猜父 span。
static ACTIVE_TURNS: std::sync::LazyLock<
    Mutex<HashMap<tokio::task::Id, (String, opentelemetry::trace::SpanContext)>>,
> = std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

/// 取当前 tokio 任务 id；不在任务上下文时 `None`。
fn current_task_id() -> Option<tokio::task::Id> {
    tokio::task::try_id()
}

thread_local! {
    /// A9：无 tokio 任务上下文（TUI/CLI `run_sync_turn`、`direct_turn_probe`、std 单测）时的
    /// 活动 turn 兜底栈 —— 否则这些路径的子 span 会断链成独立 root。
    static THREAD_TURNS: RefCell<Vec<(String, opentelemetry::trace::SpanContext)>> =
        const { RefCell::new(Vec::new()) };
}

/// 一次 turn 登记的归属信息（task 表或线程本地栈，二者择一）。
struct ActiveTurnSlot {
    task_id: Option<tokio::task::Id>,
    thread_local: bool,
}

/// 登记活动 turn，返回归属的 task id（守卫 Drop 时用它精确摘除）。
fn register_active_turn(
    turn_id: &str,
    span_context: opentelemetry::trace::SpanContext,
) -> Option<ActiveTurnSlot> {
    if !spans_allowed() {
        return None;
    }
    // A9：优先按 tokio 任务 id 索引；无任务上下文时退回线程本地栈（不丢父 span）。
    if let Some(task_id) = current_task_id() {
        let mut guard = ACTIVE_TURNS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.insert(task_id, (turn_id.to_string(), span_context));
        return Some(ActiveTurnSlot {
            task_id: Some(task_id),
            thread_local: false,
        });
    }
    THREAD_TURNS.with(|stack| {
        stack
            .borrow_mut()
            .push((turn_id.to_string(), span_context));
    });
    Some(ActiveTurnSlot {
        task_id: None,
        thread_local: true,
    })
}

fn unregister_active_turn(slot: Option<ActiveTurnSlot>) {
    let Some(slot) = slot else {
        return;
    };
    if let Some(task_id) = slot.task_id {
        let mut guard = ACTIVE_TURNS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.remove(&task_id);
        return;
    }
    if slot.thread_local {
        THREAD_TURNS.with(|stack| {
            stack.borrow_mut().pop();
        });
    }
}

fn active_turn_context() -> Option<Context> {
    if let Some(task_id) = current_task_id() {
        let guard = ACTIVE_TURNS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        return guard
            .get(&task_id)
            .map(|(_, span_context)| Context::new().with_remote_span_context(span_context.clone()));
    }
    THREAD_TURNS.with(|stack| {
        stack
            .borrow()
            .last()
            .map(|(_, span_context)| Context::new().with_remote_span_context(span_context.clone()))
    })
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
    let parent = active_turn_context().unwrap_or_else(Context::new);
    Some(state.tracer.build_with_context(
        opentelemetry::trace::SpanBuilder::from_name(name.to_string())
            .with_kind(SpanKind::Internal)
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
                let slot = register_active_turn(&format!("turn-{index}"), expected.clone());
                assert!(slot.is_some(), "活动 turn 必须登记成功");
                assert!(
                    slot.as_ref().and_then(|slot| slot.task_id).is_some(),
                    "tokio 任务内必须走 task 表（而非线程本地栈）"
                );
                // 两个 turn 同时处于活动状态，模拟宿主按 turn 并发 spawn。
                barrier.wait().await;
                let observed = active_turn_context().expect("任务内必须有活动 turn");
                if observed.span().span_context().trace_id() != expected.trace_id() {
                    mismatches.fetch_add(1, Ordering::SeqCst);
                }
                unregister_active_turn(slot);
                assert!(
                    active_turn_context().is_none(),
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
}
