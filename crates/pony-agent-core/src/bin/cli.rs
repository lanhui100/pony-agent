//! pony-cli: 独立运行的命令行 Agent 交互程序
//! 支持单轮与多轮交互、直接连接 polyllm/ponyllm 网关，并默认驱动 gemini-3.8-flash 模型与真实 Tool 调度。
//!
//! wave-59 / PA-164：本文件是**无头交付形态**（headless）的宿主接线层 ——
//! `--output-format` / `--result-json` / `--permission-mode` / `--timeout-secs`
//! 四个交付形态参数在这里解析、校验、落盘与映射为进程退出码。纯函数与 sink 在
//! `pony_agent_core::agent::headless`（契约真值源
//! `.dev-team/contract-matrix-wave59-headless.md` v4）。
//!
//! 分模式的硬约束（契约 C1 / 裁定 B4）：
//! - `text`（默认）：与基线**逐字节兼容**（横幅/事件仍走 stdout，ANSI 保留）；
//!   `--permission-mode` 未显式给出时传 `None`（向后兼容）；
//! - `jsonl` / `sse`：**stdout 只承载结构化事件**，横幅/诊断一律改走 stderr；
//!   权限档位默认 `read_only`（fail-closed）。

use pony_agent_core::agent::config::{
    ProviderModelCapabilities, ProviderSelectionResolver, ResolvedProviderSelection,
    ThinkingParamPattern,
};
use pony_agent_core::agent::control_plane::{
    HostControlPlane, HostControlPlaneBuilder, StartTurnStreamCommand,
};
use pony_agent_core::agent::graph::GraphRunStore;
use pony_agent_core::agent::headless::{
    self, ExitCode, HeadlessOptions, HeadlessOutcome, OutputFormat, PermissionMode, ResultEnvelope,
    ToolCallSummary, TurnStatus, UsageSummary, RESULT_ENVELOPE_SCHEMA,
};
use pony_agent_core::agent::provider::{ProviderAuthType, ProviderProtocol};
use pony_agent_core::agent::runtime::{
    compact_outcome_message, AgentRuntimeBuilder, TurnInput, TurnStreamEvent,
};
use pony_agent_core::agent::session::{FileSessionBackend, SessionStore};
use pony_agent_core::agent::turn_flow::TurnEventSink;
use std::io::{self, BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// 人类可读输出：`text` 模式写 stdout（与基线逐字节一致），`jsonl`/`sse` 模式改道
/// stderr —— 保证结构化模式下 stdout 只承载事件（契约 C1）。
macro_rules! human {
    ($format:expr, $($arg:tt)*) => {{
        match $format {
            OutputFormat::Text => println!($($arg)*),
            _ => eprintln!($($arg)*),
        }
    }};
}

/// 同 [`human!`]，但不换行（REPL 提示符等）。
macro_rules! human_inline {
    ($format:expr, $($arg:tt)*) => {{
        match $format {
            OutputFormat::Text => print!($($arg)*),
            _ => eprint!($($arg)*),
        }
    }};
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubCommand {
    Run,
    Init,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlashCommand {
    Clear,
    Compact,
    Help,
    Exit,
    Unknown(String),
}

impl SlashCommand {
    pub fn parse(input: &str) -> Option<Self> {
        let trimmed = input.trim();
        if !trimmed.starts_with('/') {
            return None;
        }
        match trimmed {
            "/clear" => Some(SlashCommand::Clear),
            "/compact" => Some(SlashCommand::Compact),
            "/help" => Some(SlashCommand::Help),
            "/exit" | "/quit" => Some(SlashCommand::Exit),
            other => Some(SlashCommand::Unknown(other.to_string())),
        }
    }
}

/// `--timeout-secs` 上限（24h）。超出即用法错误（exit 2）：无上限的预算不可兑现。
const MAX_TIMEOUT_SECS: u64 = 86_400;

/// 结构化 stdout 的进程级 "output-dead" 标记（v5 A-ADV：EPIPE/写失败置位后不再写、
/// 并在 stderr 留一行诊断，绝不静默截断）。
static OUTPUT_DEAD: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone)]
pub struct CliArgs {
    pub subcommand: SubCommand,
    pub prompt: Option<String>,
    pub model: String,
    pub gateway_url: String,
    pub api_key: Option<String>,
    pub workspace: PathBuf,
    pub continue_last: bool,
    pub resume_session_id: Option<String>,
    /// 契约 C1：输出形态（默认 `text`，与基线逐字节兼容）。
    pub output_format: OutputFormat,
    /// 契约 C3：结果封套落盘路径（`None` ⇒ 不落盘）。
    pub result_json_path: Option<PathBuf>,
    /// 契约 C4：审批档位（默认 `ReadOnly`，fail-closed）。
    pub permission_mode: PermissionMode,
    /// 用户是否**显式**给了 `--permission-mode`（裁定 B4：`text` 模式未显式给出时
    /// 传 `None` 以保持向后兼容；`jsonl`/`sse` 模式则始终使用解析出的档位）。
    pub permission_mode_explicit: bool,
    /// 契约 C5：有界执行预算（秒）；`None` ⇒ 无界。
    pub timeout_secs: Option<u64>,
    /// `-h` / `--help`：打印用法后以 Success(0) 退出。
    pub help_requested: bool,
}

impl Default for CliArgs {
    fn default() -> Self {
        let env_key = std::env::var("PONYLLM_API_KEY")
            .or_else(|_| std::env::var("PONY_API_KEY"))
            .ok()
            .or_else(|| {
                // 读取本地 ~/.config/ponyllm/ponyllm.toml 中配置的 api_key
                let cfg_path = dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join(".config")
                    .join("ponyllm")
                    .join("ponyllm.toml");
                if let Ok(content) = std::fs::read_to_string(cfg_path) {
                    for line in content.lines() {
                        let trimmed = line.trim();
                        if trimmed.starts_with("api_key") {
                            if let Some(val) = trimmed.split('=').nth(1) {
                                return Some(val.trim().trim_matches('"').to_string());
                            }
                        }
                    }
                }
                None
            });

        let env_gateway = std::env::var("PONYLLM_GATEWAY_URL")
            .unwrap_or_else(|_| "http://10.42.0.67:8080".to_string());

        let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let canonical_workspace = std::fs::canonicalize(&current_dir).unwrap_or(current_dir);

        let headless_defaults = HeadlessOptions::default();
        Self {
            subcommand: SubCommand::Run,
            prompt: None,
            model: "gemini-3.8-flash".to_string(),
            gateway_url: env_gateway,
            api_key: env_key,
            workspace: canonical_workspace,
            continue_last: false,
            resume_session_id: None,
            output_format: headless_defaults.output_format,
            result_json_path: headless_defaults.result_json_path,
            permission_mode: headless_defaults.permission_mode,
            permission_mode_explicit: false,
            timeout_secs: headless_defaults.timeout_secs,
            help_requested: false,
        }
    }
}

pub fn parse_cli_args(args: &[String]) -> Result<CliArgs, String> {
    let mut config = CliArgs::default();
    let mut idx = 1;

    if idx < args.len() && args[idx] == "init" {
        config.subcommand = SubCommand::Init;
        idx += 1;
    }

    while idx < args.len() {
        match args[idx].as_str() {
            "-p" | "--prompt" => {
                idx += 1;
                if idx >= args.len() {
                    return Err("--prompt requires a value".to_string());
                }
                config.prompt = Some(args[idx].clone());
            }
            "-m" | "--model" => {
                idx += 1;
                if idx >= args.len() {
                    return Err("--model requires a value".to_string());
                }
                config.model = args[idx].clone();
            }
            "-g" | "--gateway" => {
                idx += 1;
                if idx >= args.len() {
                    return Err("--gateway requires a value".to_string());
                }
                config.gateway_url = args[idx].clone();
            }
            "-w" | "--workspace" => {
                idx += 1;
                if idx >= args.len() {
                    return Err("--workspace requires a value".to_string());
                }
                let raw_path = PathBuf::from(&args[idx]);
                config.workspace = std::fs::canonicalize(&raw_path).unwrap_or(raw_path);
            }
            "-c" | "--continue" => {
                config.continue_last = true;
            }
            "-r" | "--resume" => {
                idx += 1;
                if idx >= args.len() {
                    return Err("--resume requires a session ID".to_string());
                }
                config.resume_session_id = Some(args[idx].clone());
            }
            "--key" | "--api-key" => {
                idx += 1;
                if idx >= args.len() {
                    return Err("--api-key requires a value".to_string());
                }
                config.api_key = Some(args[idx].clone());
            }
            // ---- wave-59 headless 交付形态参数（契约 C1/C3/C4/C5）----
            "--output-format" => {
                idx += 1;
                if idx >= args.len() {
                    return Err("--output-format requires a value (text|jsonl|sse)".to_string());
                }
                config.output_format = OutputFormat::from_str(&args[idx]).map_err(|err| {
                    format!("{err}（非法 --output-format 必须被拒绝，不得静默退化为 text）")
                })?;
            }
            "--result-json" => {
                idx += 1;
                if idx >= args.len() {
                    return Err("--result-json requires a path".to_string());
                }
                config.result_json_path = Some(PathBuf::from(&args[idx]));
            }
            "--permission-mode" => {
                idx += 1;
                if idx >= args.len() {
                    return Err(
                        "--permission-mode requires a value (deny|workspace-write|full-access)"
                            .to_string(),
                    );
                }
                config.permission_mode = PermissionMode::from_str(&args[idx])?;
                config.permission_mode_explicit = true;
            }
            "--timeout-secs" => {
                idx += 1;
                if idx >= args.len() {
                    return Err("--timeout-secs requires a whole number of seconds".to_string());
                }
                let secs = args[idx].parse::<u64>().map_err(|_| {
                    format!(
                        "invalid value `{}` for --timeout-secs: expected a whole number of seconds",
                        args[idx]
                    )
                })?;
                // v5 B-ADV7：无上限的“有界”预算形同虚设（可填 u64::MAX 让 exit 7 永不发生）。
                if secs > MAX_TIMEOUT_SECS {
                    return Err(format!(
                        "--timeout-secs {secs} 超出上限 {MAX_TIMEOUT_SECS}s（24h）；请给出真实可兑现的预算"
                    ));
                }
                config.timeout_secs = Some(secs);
            }
            "-h" | "--help" => {
                config.help_requested = true;
            }
            arg if arg.starts_with('-') => {
                return Err(format!("unknown argument: {arg}"));
            }
            arg => {
                if config.prompt.is_none() {
                    config.prompt = Some(arg.to_string());
                }
            }
        }
        idx += 1;
    }
    Ok(config)
}

struct DynamicProviderResolver {
    gateway_url: String,
    api_key: Option<String>,
    model: String,
}

impl ProviderSelectionResolver for DynamicProviderResolver {
    fn resolve_provider_selection(
        &self,
        _provider_id: Option<&str>,
        _model_id: Option<&str>,
    ) -> ResolvedProviderSelection {
        let mut base = self.gateway_url.trim_end_matches('/').to_string();
        if !base.ends_with("/v1") {
            base.push_str("/v1");
        }

        ResolvedProviderSelection {
            requested_name: "ponyllm".to_string(),
            provider_name: "ponyllm".to_string(),
            protocol: ProviderProtocol::OpenAiCompletions,
            base_url: base,
            auth_type: ProviderAuthType::Auto,
            api_key_env_var: "PONYLLM_API_KEY".to_string(),
            api_key: self.api_key.clone(),
            model: self.model.clone(),
            temperature: 0.2,
            max_output_tokens: 4096,
            reasoning_effort: None,
            reasoning_budget_tokens: None,
            capabilities: ProviderModelCapabilities {
                supports_tools: true,
                supports_streaming: true,
                supports_image_input: true,
                supports_reasoning: false,
                context_window_tokens: Some(1_000_000),
                ..Default::default()
            },
            thinking_param_pattern: ThinkingParamPattern::None,
        }
    }
}

struct TerminalSink {
    is_streaming: Arc<AtomicBool>,
}

impl TerminalSink {
    fn new(is_streaming: Arc<AtomicBool>) -> Self {
        Self { is_streaming }
    }
}

impl TurnEventSink for TerminalSink {
    fn emit(&self, name: &str, payload: TurnStreamEvent) {
        match name {
            "turn:delta" => {
                if let Some(text) = payload.text {
                    print!("{text}");
                    let _ = io::stdout().flush();
                }
            }
            "turn:tool_call" => {
                let call_info = payload.tool_activities
                    .as_ref()
                    .and_then(|a| a.last())
                    .map(|act| format!("{}: {:?}", act.name, act.arguments_text))
                    .or(payload.text);
                println!("\n\x1b[33m⚡ [Tool Call]\x1b[0m {:?}", call_info);
            }
            "turn:tool_result" => {
                let res_info = payload.tool_activities
                    .as_ref()
                    .and_then(|a| a.last())
                    .map(|act| format!("{}: status={:?}, res={:?}", act.name, act.status, act.result_text))
                    .or(payload.text);
                println!("\x1b[32m✔ [Tool Result]\x1b[0m {:?}", res_info);
            }
            "turn:completed" => {
                println!("\n\x1b[36m✔ [Turn Completed]\x1b[0m");
                self.is_streaming.store(false, Ordering::SeqCst);
            }
            "turn:failed" => {
                eprintln!("\n\x1b[31m✘ [Turn Failed]\x1b[0m error={:?}", payload.error);
                self.is_streaming.store(false, Ordering::SeqCst);
            }
            // 契约 C4/D2：`turn:suspended` 是**终态**（审批/ask 挂起，无头宿主无应答方）。
            // 旧实现不认它 ⇒ `is_streaming` 永真 ⇒ CLI 死循环挂死。本分支只收敛状态、
            // 不打印任何内容（text 模式输出逐字节不变），退出码由调用方映射为
            // `ExitCode::PermissionDenied`(6)。
            "turn:suspended" => {
                self.is_streaming.store(false, Ordering::SeqCst);
            }
            "turn:output_end" => {
                self.is_streaming.store(false, Ordering::SeqCst);
            }
            _ => {}
        }
    }
}

/// 结构化 sink 的两种形态（契约 C1）。
enum StructuredSink {
    Jsonl(headless::JsonlSink),
    Sse(headless::SseSink),
}

impl StructuredSink {
    fn emit(&self, name: &str, payload: TurnStreamEvent) {
        match self {
            StructuredSink::Jsonl(sink) => sink.emit(name, payload),
            StructuredSink::Sse(sink) => sink.emit(name, payload),
        }
    }

    /// 取走已缓冲的事件行/帧（一次性写入 stdout）。
    fn finish(&self) -> Vec<String> {
        match self {
            StructuredSink::Jsonl(sink) => sink.finish(),
            StructuredSink::Sse(sink) => sink.finish(),
        }
    }

    fn outcome(&self) -> Option<HeadlessOutcome> {
        match self {
            StructuredSink::Jsonl(sink) => sink.outcome(),
            StructuredSink::Sse(sink) => sink.outcome(),
        }
    }
}

/// 单轮执行的事实汇总（喂给 [`ResultEnvelope`] 与退出码映射）。
///
/// `failure_reason` 严格**透传 wire 原文**（v4 裁定 B3）：`turn:failed` 取
/// `payload.error`，`turn:suspended` / `turn:cancelled` 取 `payload.error`，缺失时
/// 回落 `payload.fallback_reason`（suspended 的 `control_outcome_pending` 落在这里）。
#[derive(Debug, Default)]
struct TurnFacts {
    outcome: Option<HeadlessOutcome>,
    failure_reason: Option<String>,
    final_text: String,
    streamed_text: String,
    input_tokens: u64,
    output_tokens: u64,
    tool_calls: Vec<ToolCallSummary>,
    session_id: Option<String>,
    turn_id: Option<String>,
    duration_ms: Option<u64>,
}

/// 无头 CLI 的 turn sink：`text` 模式行为与基线一致（[`TerminalSink`]），
/// `jsonl`/`sse` 模式经 [`headless::JsonlSink`] / [`headless::SseSink`] 产出事件；
/// 两种模式都并行记录 [`TurnFacts`]，供退出码与结果封套使用。
struct CliTurnSink {
    format: OutputFormat,
    terminal: TerminalSink,
    structured: Option<StructuredSink>,
    facts: Mutex<TurnFacts>,
}

impl CliTurnSink {
    fn new(format: OutputFormat) -> Self {
        let structured = match format {
            OutputFormat::Text => None,
            OutputFormat::Jsonl => Some(StructuredSink::Jsonl(headless::JsonlSink::new())),
            OutputFormat::Sse => Some(StructuredSink::Sse(headless::SseSink::new())),
        };
        Self {
            format,
            terminal: TerminalSink::new(Arc::new(AtomicBool::new(true))),
            structured,
            facts: Mutex::new(TurnFacts::default()),
        }
    }

    fn facts(&self) -> std::sync::MutexGuard<'_, TurnFacts> {
        self.facts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// 取走结构化事件（非结构化模式为空调度）。
    fn take_events(&self) -> Vec<String> {
        self.structured
            .as_ref()
            .map(StructuredSink::finish)
            .unwrap_or_default()
    }

    /// 已观测到的终态：sink 与 facts 双源，任一观测到即生效。
    fn observed_outcome(&self) -> Option<HeadlessOutcome> {
        self.facts()
            .outcome
            .or_else(|| self.structured.as_ref().and_then(StructuredSink::outcome))
    }
}

impl TurnEventSink for CliTurnSink {
    fn emit(&self, name: &str, payload: TurnStreamEvent) {
        // 1. 交付形态输出：text ⇒ 基线行为；jsonl/sse ⇒ 结构化事件（stdout 纯净）。
        match &self.structured {
            Some(structured) => structured.emit(name, payload.clone()),
            None if self.format == OutputFormat::Text => self.terminal.emit(name, payload.clone()),
            None => {}
        }

        // 2. 事实记录（两种模式同一口径）。
        let mut facts = self.facts();
        if facts.session_id.is_none() {
            facts.session_id = payload.session_id.clone();
        }
        if !payload.turn_id.is_empty() {
            facts.turn_id = Some(payload.turn_id.clone());
        }
        if facts.duration_ms.is_none() {
            facts.duration_ms = payload.turn_duration_ms;
        }
        match name {
            "turn:delta" => {
                if let Some(text) = payload.text.as_ref() {
                    facts.streamed_text.push_str(text);
                }
            }
            "turn:tool" => {
                if let Some(activities) = payload.tool_activities.as_ref() {
                    facts.tool_calls = activities
                        .iter()
                        .map(|activity| ToolCallSummary {
                            name: activity.name.clone(),
                            status: activity.status.clone(),
                        })
                        .collect();
                }
            }
            "turn:completed" => {
                facts.outcome = Some(HeadlessOutcome::Completed);
                if let Some(text) = payload.text.as_ref() {
                    facts.final_text = text.clone();
                }
                facts.input_tokens = payload.input_tokens.unwrap_or(0);
                facts.output_tokens = payload.output_tokens.unwrap_or(0);
                if let Some(activities) = payload.tool_activities.as_ref() {
                    facts.tool_calls = activities
                        .iter()
                        .map(|activity| ToolCallSummary {
                            name: activity.name.clone(),
                            status: activity.status.clone(),
                        })
                        .collect();
                }
            }
            "turn:failed" => {
                facts.outcome = Some(HeadlessOutcome::Failed);
                facts.failure_reason = payload
                    .error
                    .clone()
                    .or_else(|| payload.fallback_reason.clone());
            }
            "turn:suspended" => {
                facts.outcome = Some(HeadlessOutcome::Suspended);
                facts.failure_reason = payload
                    .error
                    .clone()
                    .or_else(|| payload.fallback_reason.clone());
            }
            "turn:cancelled" => {
                facts.outcome = Some(HeadlessOutcome::Cancelled);
                facts.failure_reason = payload
                    .error
                    .clone()
                    .or_else(|| payload.fallback_reason.clone());
            }
            _ => {}
        }
    }
}

/// 单轮执行的结果（退出码 + 封套 + 待写 stdout 的结构化事件）。
struct TurnExecution {
    exit_code: ExitCode,
    envelope: ResultEnvelope,
    events: Vec<String>,
}

/// 生效的 `tool_authorization_mode`（裁定 B4：分模式默认）。
///
/// * `text` 模式且用户未显式给 `--permission-mode` ⇒ `None`（与基线逐字节/逐行为兼容）；
/// * 其余情况 ⇒ 解析出的档位（默认 `read_only`，fail-closed）。
fn effective_authorization_mode(args: &CliArgs, options: &HeadlessOptions) -> Option<String> {
    let explicit_or_structured =
        args.permission_mode_explicit || options.output_format != OutputFormat::Text;
    if !explicit_or_structured {
        return None;
    }
    Some(options.permission_mode.tool_authorization_mode().to_string())
}

/// 构造超时终止标记事件（宿主命名空间 `headless:timeout`）。
///
/// 语义边界：本标记只表达「宿主到点停止等待、事件流在此截断」；它**不**表示 worker
/// 线程已结束（不可抢占），也不表示 turn 失败（turn 可能仍在 provider 阻塞读上）。
fn host_timeout_marker(turn_id: &str, session_id: &str) -> Option<TurnStreamEvent> {
    serde_json::from_value(serde_json::json!({
        "turnId": turn_id,
        "sessionId": session_id,
        "kind": "headless:timeout",
        "phase": "timeout",
        "text": "host deadline exceeded: event stream truncated by headless CLI (worker not preemptible)",
        "error": "timeout",
    }))
    .ok()
}

/// 执行一轮 turn（有界），返回终态、退出码、结果封套与待写事件。
fn run_single_turn(
    control_plane: &Arc<HostControlPlane>,
    args: &CliArgs,
    options: &HeadlessOptions,
    session_id: &str,
    prompt: &str,
) -> TurnExecution {
    let turn_id = format!("turn-{}", uuid::Uuid::new_v4());
    let sink = Arc::new(CliTurnSink::new(options.output_format));
    let started = Instant::now();

    human!(options.output_format, "\x1b[1;34m[Pony Agent] Running turn...\x1b[0m");

    let authorization = effective_authorization_mode(args, options);
    // 裁定 B5（v5 收窄）：只有用户**显式**指定了非 read_only 档位时才外显已知局限 ——
    // text 模式默认传 `None`（向后兼容），为它刷 WARN 会误导；jsonl/sse 默认 read_only，
    // 也不该刷屏。禁止静默的是「显式声明了未生效的收紧档位」这一种情况。
    if args.permission_mode_explicit && args.permission_mode != PermissionMode::ReadOnly {
        let mode = args.permission_mode.tool_authorization_mode();
        eprintln!(
            "WARN permission-mode={mode}: workspace_write/full_access 与「不传」在运行时限内判定等价（已知局限，见 PA-167）"
        );
    }

    let cmd = StartTurnStreamCommand {
        turn_id: turn_id.clone(),
        input: TurnInput {
            message: prompt.to_string(),
            display_message: None,
            provider_id: Some("ponyllm".to_string()),
            model_id: None,
            reasoning_effort: None,
            workspace_mode: None,
            session_id: Some(session_id.to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
            plan_execution_approval: None,
            tool_authorization_mode: authorization,
        },
    };

    // C5：`start_turn_stream` 同步阻塞 ⇒ worker 线程跑整轮，调用线程带预算 join。
    let deadline = options.timeout_secs.map(Duration::from_secs);
    let worker_control_plane = Arc::clone(control_plane);
    let worker_sink = Arc::clone(&sink);
    let outcome = headless::run_with_deadline(deadline, move || {
        // runtime 上下文（tokio）在 worker 线程内建立。
        let _guard = pony_agent_core::agent::runtime_helper::TestRuntimeGuard::new();
        worker_control_plane.start_turn_stream(worker_sink.as_ref(), cmd);
    });

    let observed = sink.observed_outcome();

    let facts = {
        let guard = sink.facts();
        TurnFacts {
            outcome: guard.outcome,
            failure_reason: guard.failure_reason.clone(),
            final_text: guard.final_text.clone(),
            streamed_text: guard.streamed_text.clone(),
            input_tokens: guard.input_tokens,
            output_tokens: guard.output_tokens,
            tool_calls: guard.tool_calls.clone(),
            session_id: guard.session_id.clone(),
            turn_id: guard.turn_id.clone(),
            duration_ms: guard.duration_ms,
        }
    };

    // 终态判定（fail-closed）：worker 正常返回但**未观测到终态事件**时绝不记 Completed。
    let status = match outcome {
        HeadlessOutcome::Timeout => TurnStatus::Timeout,
        HeadlessOutcome::Failed => TurnStatus::Failed,
        HeadlessOutcome::Suspended => TurnStatus::Suspended,
        HeadlessOutcome::Cancelled => TurnStatus::Cancelled,
        HeadlessOutcome::Completed => match observed {
            Some(HeadlessOutcome::Completed) => TurnStatus::Completed,
            Some(HeadlessOutcome::Suspended) => TurnStatus::Suspended,
            Some(HeadlessOutcome::Cancelled) => TurnStatus::Cancelled,
            Some(HeadlessOutcome::Timeout) => TurnStatus::Timeout,
            Some(HeadlessOutcome::Failed) | None => TurnStatus::Failed,
        },
    };

    // 超时后事件流不得无声截断（v5 A-ADV「补终止标记」）：补一条**宿主命名空间**的
    // 终止标记（`headless:` 前缀，不冒充 runtime 的 `turn:*` 事件）。它只说明「本进程
    // 到点收工、流在此截断」，不宣称 turn 已结束（worker 不可抢占，见契约 B1）。
    if status == TurnStatus::Timeout && options.output_format != OutputFormat::Text {
        if let Some(marker) = host_timeout_marker(&turn_id, session_id) {
            sink.emit("headless:timeout", marker);
        }
    }
    let events = sink.take_events();

    // 退出码：Failed 按失败原文分档（C2 穷举 match 落在 classify_failure 内），其余走冻结映射。
    let exit_code = match status {
        TurnStatus::Failed => ExitCode::classify_failure(
            facts.failure_reason.as_deref().unwrap_or("turn:failed"),
        ),
        other => headless::exit_code_for(other),
    };

    let final_text = if facts.final_text.is_empty() {
        facts.streamed_text
    } else {
        facts.final_text
    };
    let envelope = ResultEnvelope {
        schema: RESULT_ENVELOPE_SCHEMA.to_string(),
        session_id: facts
            .session_id
            .clone()
            .unwrap_or_else(|| session_id.to_string()),
        turn_id: facts.turn_id.clone().unwrap_or_else(|| turn_id.clone()),
        status,
        // 与进程退出码**同源**（同一变量），不做二次推导（契约 C3 ⟺ 不变量）。
        exit_code: exit_code.code(),
        final_text,
        usage: UsageSummary {
            input_tokens: facts.input_tokens,
            output_tokens: facts.output_tokens,
        },
        tool_calls: facts.tool_calls.clone(),
        duration_ms: facts
            .duration_ms
            .unwrap_or_else(|| started.elapsed().as_millis() as u64),
        failure_reason: facts.failure_reason.clone(),
    };

    TurnExecution {
        exit_code,
        envelope,
        events,
    }
}

/// 原子写结果封套（C3：tmp + rename，父目录按需创建）。
///
/// v5 B-ADV1（安全）：
/// * 临时名**不可预测**（`.name.tmp-<pid>-<随机 uuid>`），避免同目录内被抢占/预置；
/// * `create_new(true)` —— 目标已存在（含符号链接）直接失败，绝不跟随符号链接写入；
/// * 文件权限 `0600`（仅属主可读写；封套含会话/输出内容）；
/// * 写完 flush 再 `rename`，同目录 rename 原子（不会留下半个封套）。
fn write_result_envelope(path: &Path, envelope: &ResultEnvelope) -> Result<(), String> {
    let payload = serde_json::to_string_pretty(envelope)
        .map_err(|err| format!("封套序列化失败: {err}"))?;
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|err| format!("无法创建结果目录 `{}`: {err}", parent.display()))?;
        }
    }
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "pony-headless-result.json".to_string());
    let nonce = format!("{}-{}", std::process::id(), uuid::Uuid::new_v4().simple());
    let tmp_path = path.with_file_name(format!(".{file_name}.tmp-{nonce}"));

    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp_path).map_err(|err| {
        format!(
            "创建临时封套 `{}` 失败（create_new，拒绝覆盖/跟随符号链接）: {err}",
            tmp_path.display()
        )
    })?;
    let write_result = file
        .write_all(payload.as_bytes())
        .and_then(|()| file.flush());
    drop(file);
    if let Err(err) = write_result {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(format!("写入临时封套 `{}` 失败: {err}", tmp_path.display()));
    }
    std::fs::rename(&tmp_path, path).map_err(|err| {
        let _ = std::fs::remove_file(&tmp_path);
        format!("封套原子重命名到 `{}` 失败: {err}", path.display())
    })
}

/// 落盘结果封套并给出**最终进程退出码**（契约 C3）。
///
/// `--result-json` 未给 ⇒ 原样返回终态退出码。给了但写盘失败 ⇒ 封套这一交付承诺落空，
/// 按 fail-closed 升级为 [`ExitCode::Internal`]（不得把「承诺的产物没落盘」报成 Success）。
fn persist_turn_result(execution: &TurnExecution, options: &HeadlessOptions) -> ExitCode {
    let Some(path) = options.result_json_path.as_deref() else {
        return execution.exit_code;
    };
    match write_result_envelope(path, &execution.envelope) {
        Ok(()) => execution.exit_code,
        Err(err) => {
            eprintln!("\x1b[31mResult Envelope Error:\x1b[0m {err}");
            ExitCode::Internal
        }
    }
}

/// 把结构化事件一次性写到 stdout（C1：stdout 只承载事件）。
///
/// v5 A-ADV（EPIPE 不得静默）：写失败（典型为下游 `| head` 关闭管道）⇒ 置进程级
/// [`OUTPUT_DEAD`] 标记、**停止后续写出**、并在 stderr 留一行诊断 —— 调用方由此可区分
/// 「事件已全量投递」与「事件被下游截断」。写失败不改变终态退出码（契约 C2 仍按 turn 终态）。
fn flush_events(events: &[String]) {
    if events.is_empty() || OUTPUT_DEAD.load(Ordering::SeqCst) {
        return;
    }
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    for line in events {
        if let Err(err) = writeln!(handle, "{line}") {
            mark_output_dead(&err);
            return;
        }
    }
    if let Err(err) = handle.flush() {
        mark_output_dead(&err);
    }
}

/// 置 output-dead 标记并落一行 stderr 诊断（只报一次）。
fn mark_output_dead(err: &io::Error) {
    if OUTPUT_DEAD.swap(true, Ordering::SeqCst) {
        return;
    }
    eprintln!(
        "[pony-headless] WARN stdout 写入失败（{err}）：事件流已被下游截断，后续事件不再写出（退出码仍按 turn 终态）"
    );
}

const DEFAULT_AGENTS_MD: &str = r#"# AGENTS.md

Welcome to the project! This document outlines engineering conventions, instructions, and workflows for AI Agents.

## 1. Project Overview & Architecture
- Maintain high code quality, strict typing, and comprehensive test coverage.
- Keep components modular and single-responsibility.

## 2. Engineering Boundaries & Anti-Entropy
- **Read before write**: Always inspect existing code and context before making changes.
- **Minimal closure**: Confine modifications to the minimal necessary files; avoid unnecessary refactoring.
- **Physical proof**: Ground all validations in real machine receipts (Exit Code 0, compiler passes, test assertions).

## 3. Development Workflow & Commands
- **Build**: Define the primary project build command here.
- **Test**: Run all unit and integration tests before finishing tasks.
- **Lint/Check**: Ensure type checks and style checks pass.
"#;

fn handle_init(workspace: &std::path::Path, format: OutputFormat) -> Result<(), String> {
    let target = workspace.join("AGENTS.md");
    if target.exists() {
        human!(
            format,
            "\x1b[33mAGENTS.md already exists at {}\x1b[0m",
            target.display()
        );
        return Ok(());
    }
    std::fs::write(&target, DEFAULT_AGENTS_MD)
        .map_err(|e| format!("failed to write AGENTS.md: {e}"))?;
    human!(
        format,
        "\x1b[32m✔ Successfully created AGENTS.md at {}\x1b[0m",
        target.display()
    );
    Ok(())
}

fn read_stdin_if_piped() -> Option<String> {
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() {
        let mut buffer = String::new();
        if std::io::stdin().read_to_string(&mut buffer).is_ok() && !buffer.trim().is_empty() {
            return Some(buffer);
        }
    }
    None
}

struct SessionTracker {
    last_session_file: PathBuf,
}

impl SessionTracker {
    fn new(state_dir: &std::path::Path) -> Self {
        Self {
            last_session_file: state_dir.join("last_session_id.txt"),
        }
    }

    fn record(&self, session_id: &str) {
        let _ = std::fs::write(&self.last_session_file, session_id);
    }

    fn get_last(&self) -> Option<String> {
        std::fs::read_to_string(&self.last_session_file)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }
}

fn build_control_plane(args: &CliArgs) -> (HostControlPlane, PathBuf) {
    let state_dir = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".pony-agent")
        .join("cli-state");
    let _ = std::fs::create_dir_all(&state_dir);

    let session_path = state_dir.join("cli-sessions.json");
    let graph_path = state_dir.join("cli-graph.json");

    let resolver = DynamicProviderResolver {
        gateway_url: args.gateway_url.clone(),
        api_key: args.api_key.clone(),
        model: args.model.clone(),
    };

    let runtime_builder = AgentRuntimeBuilder::new()
        .session_store(SessionStore::with_backend(Box::new(
            FileSessionBackend::new(session_path),
        )))
        .provider_resolver(Box::new(resolver))
        .workspace_root(args.workspace.clone());

    let control_plane = HostControlPlaneBuilder::new()
        .runtime_builder(runtime_builder)
        .graph_run_store(GraphRunStore::persistent(graph_path))
        .build();

    (control_plane, state_dir)
}

fn print_help(format: OutputFormat) {
    human!(format, "\x1b[1;33mCommands & Slash-commands:\x1b[0m");
    human!(format, "  /clear    Clear the current context and start a fresh session");
    human!(format, "  /compact  Compact the current conversation history");
    human!(format, "  /help     Show this help message");
    human!(format, "  /exit     Exit the interactive session");
}

/// `-h` / `--help` 用法说明（裁定 B5：必须外显审批档位的已知局限，禁止静默）。
fn print_usage(format: OutputFormat) {
    human!(format, "\x1b[1;32m=== Pony Agent CLI (Claude-Code Style) ===\x1b[0m");
    human!(format, "Usage: ponycli [init] [OPTIONS] [-p <prompt>]");
    human!(format, "");
    human!(format, "Core options:");
    human!(format, "  -p, --prompt <text>        Run a single turn with the given prompt");
    human!(format, "  -m, --model <id>           Model id (default: gemini-3.8-flash)");
    human!(format, "  -g, --gateway <url>        Provider gateway base url");
    human!(format, "  -w, --workspace <path>     Workspace root (default: current dir)");
    human!(format, "  -c, --continue             Continue the last session");
    human!(format, "  -r, --resume <session-id>  Resume a specific session");
    human!(format, "      --key, --api-key <key> Provider API key");
    human!(format, "");
    human!(format, "Headless delivery form (wave-59 / PA-164):");
    human!(format, "  --output-format <fmt>      text | jsonl | sse (default: text)");
    human!(format, "                             jsonl/sse: stdout carries events only, diagnostics go to stderr");
    human!(format, "  --result-json <path>       Write the result envelope (pony.headless.result.v1) atomically");
    human!(format, "                             tmp `create_new` + 0600 + rename (no predictable temp name)");
    human!(format, "  --permission-mode <mode>   Aliases: deny | read-only | read_only  → read_only");
    human!(format, "                                        workspace-write | workspace_write → workspace_write");
    human!(format, "                                        full-access | full_access         → full_access");
    human!(format, "                             Default is MODE-DEPENDENT:");
    human!(format, "                               text      : NOT passed (None) — unrestricted, byte-compatible baseline");
    human!(format, "                               jsonl/sse : deny (read_only) — fail-closed");
    human!(format, "  --timeout-secs <n>         Bound the turn (0..=86400); expiry exits with code 7 (Timeout)");
    human!(format, "                             jsonl/sse: a `headless:timeout` marker line is emitted on expiry");
    human!(format, "  -h, --help                 Show this help");
    human!(format, "");
    human!(format, "Machine-readable modes (jsonl / sse) require -p or piped stdin — no interactive REPL");
    human!(format, "(without either, the CLI exits 2 Usage).");
    human!(format, "");
    human!(format, "\x1b[33mKnown limitation (PA-167):\x1b[0m `workspace-write` / `full-access` are declared");
    human!(format, "permission tiers that the host currently decides exactly like \"not passed\" — only");
    human!(format, "`deny` (read_only) has an enforced branch. A WARN is printed to stderr only when one of");
    human!(format, "the two non-enforcing tiers is passed EXPLICITLY (text mode default passes None).");
    human!(format, "");
    human!(format, "Exit codes: 0 success / 1 internal / 2 usage / 3 config / 4 provider-network");
    human!(format, "            5 tool-failure / 6 permission-denied (incl. suspended) / 7 timeout");
}

/// 可观测性引导（Wave-5 P0 修复：CLI/TUI 交付形态此前**完全没有** tracing 与 issue 引导）。
///
/// 契约：
/// - 装配 panic hook（崩溃上报 PonySentry）与 OTel provider（trace 导出）；
/// - 配置解析与 Tauri 侧**同一套语义**：settings 持久化 > 环境变量 > 内置公网默认端点；
/// - **上报门禁默认关闭**（`agentTraceReporting` 缺省 `false`），需用户在设置里显式开启
///   （写入 `~/.config/pony-agent/settings.json`）才会产生 span 与导出；关闭态零 span 零 egress；
/// - 失败只降级为 `log_json("warn", ...)`：绝不 panic、绝不阻塞启动（trace 不可用不阻断业务）。
/// CLI/TUI 的运行日志落盘路径（与 Tauri 的 `<app_data_dir>/logs/pony-agent.log` 同口径）。
///
/// wave-59（契约 C1）：`jsonl`/`sse` 模式下 stdout 只承载事件，故本装配点先把 JSON
/// 日志的缺省输出目标改道 stderr（`otel::log::set_stderr_output`）——text 模式与其它宿主
/// （TUI/Tauri）行为逐字节不变。
///
/// 取舍：**统一**落到 `logs/pony-agent.log`，不复用 TUI 既有的 `~/.pony-agent/tui-logs/` ——
/// 后者是会话调试产物（整屏 dump），把结构化 JSON 运行日志混进去会让取证面变脏。
/// 优先级：`dirs::state_dir()`（Linux = `$XDG_STATE_HOME` 或 `~/.local/state`）
/// > `$HOME/.local/state`；都不可得时返回 `None`（仅 stdout，不阻塞启动）。
/// 轮转（512KiB）/ 降级 / 脱敏全部复用 `otel::log` 既有实现。
fn bin_log_file_path() -> Option<std::path::PathBuf> {
    let base = dirs::state_dir()
        .or_else(|| dirs::home_dir().map(|home| home.join(".local").join("state")))?;
    Some(base.join("pony-agent").join("logs").join("pony-agent.log"))
}
struct ObservabilityBootstrap;

impl ObservabilityBootstrap {
    fn install(output_format: OutputFormat) -> Self {
        // 契约 C1：结构化模式下把 JSON 诊断日志改道 stderr（默认 stdout，text 模式不变）。
        if output_format != OutputFormat::Text {
            pony_agent_core::agent::otel::log::set_stderr_output(true);
        }
        // Wave-5 P0-A：日志落盘（复用 otel::log 的 512KiB 轮转 / 降级 / 脱敏实现，不另写一套）。
        if let Some(log_path) = bin_log_file_path() {
            pony_agent_core::agent::otel::log::set_log_file_path(log_path);
        }
        // Wave-5 P0-B：仅 CLI/TUI 打开 core 侧 turn 失败上报（桌面端由前端 `TurnExecutionFailed`
        // 上报，保持关闭以避免重复 issue）。
        pony_agent_core::agent::ponysentry::enable_turn_failure_capture(true);

        // settings 优先（与 `src-tauri` 的 `load_app_settings` 等价的双路径同步）。
        let settings = pony_agent_core::agent::app_settings::AppSettingsStore::new().load_view();
        pony_agent_core::agent::otel::set_agent_trace_reporting(settings.agent_trace_reporting);
        pony_agent_core::agent::cost::set_budget_usd(settings.cost_budget_usd);
        // Wave-7：用量治理策略（None ⇒ 默认：enforcement=false，软告警不阻断）。
        pony_agent_core::agent::usage_governance::set_policy(
            settings.usage_governance_policy.clone(),
        );

        let snapshot = settings.otel_settings_snapshot();
        if let Err(err) = pony_agent_core::agent::otel::init(
            pony_agent_core::agent::otel::OtelConfig::from_settings(Some(&snapshot)),
        ) {
            pony_agent_core::agent::otel::log::log_json(
                "warn",
                &format!("otel 装配失败，trace 上报已禁用：{err}"),
                None,
            );
        }
        // PonySentry 凭据分发（settings > env > 构建期 option_env! > 缺省）。
        // 无 token ⇒ fail-closed 零 egress；token 不进任何日志/Debug。
        // 历史缺口：CLI/TUI 只有构建期烧录通道，桌面端之外的形态一条 issue 都发不出去。
        let ponysentry_settings = settings.ponysentry_settings_snapshot();
        pony_agent_core::agent::ponysentry::init_from_settings(Some(&ponysentry_settings));
        Self
    }
}

impl Drop for ObservabilityBootstrap {
    /// 退出清理（正常 return / 主循环结束 / Ctrl-C 交由 TUI 循环返回后）：有界 shutdown，
    /// 内部会先 drain 批量队列再关 provider（上限 1000ms，幂等，不因网络悬挂卡死退出）。
    fn drop(&mut self) {
        // 先排空 issue 上报队列（有界 1500ms，避免 `--prompt` 这类「报错即退出」丢 issue），
        // 再关闭 OTel provider（有界 1000ms，内部先 drain span 队列）。
        pony_agent_core::agent::ponysentry::flush();
        pony_agent_core::agent::otel::shutdown();
    }
}

/// 进程收口：先显式 drop 可观测性 guard（`std::process::exit` 会跳过 `Drop`，
/// 不显式 drop 就会丢 issue/span flush），再按终态退出码退出。
///
/// 契约 C2：终端 ExitCode 直接传给 `std::process::exit`，不得改写为 0。
fn finish(code: ExitCode, observability: ObservabilityBootstrap) -> ! {
    drop(observability);
    std::process::exit(code.code());
}

fn main() {
    let _runtime_guard = pony_agent_core::agent::runtime_helper::TestRuntimeGuard::new();
    // 尽早装配 panic hook（崩溃上报），随后解析参数以决定是否需把诊断日志改道 stderr。
    pony_agent_core::agent::ponysentry::install_panic_hook();
    let raw_args: Vec<String> = std::env::args().collect();
    let parsed = parse_cli_args(&raw_args);
    // 非法 `--output-format` 会走到 Usage(2) 分支；此时先按 text 装配（改为 stderr 也无害）。
    let early_format = parsed
        .as_ref()
        .map(|args| args.output_format)
        .unwrap_or(OutputFormat::Text);
    let observability = ObservabilityBootstrap::install(early_format);
    let args = match parsed {
        Ok(a) => a,
        Err(err) => {
            eprintln!("\x1b[31mArgument Error:\x1b[0m {err}");
            finish(ExitCode::Usage, observability);
        }
    };

    if args.help_requested {
        print_usage(args.output_format);
        finish(ExitCode::Success, observability);
    }

    if args.subcommand == SubCommand::Init {
        if let Err(e) = handle_init(&args.workspace, args.output_format) {
            eprintln!("\x1b[31mInit Error:\x1b[0m {e}");
            finish(ExitCode::Internal, observability);
        }
        finish(ExitCode::Success, observability);
    }

    let options = HeadlessOptions {
        output_format: args.output_format,
        result_json_path: args.result_json_path.clone(),
        permission_mode: args.permission_mode,
        timeout_secs: args.timeout_secs,
    };
    let format = options.output_format;

    human!(format, "\x1b[1;32m=== Pony Agent CLI (Claude-Code Style) ===\x1b[0m");
    human!(format, "  Model:       {}", args.model);
    human!(format, "  Gateway:     {}", args.gateway_url);
    human!(format, "  Workspace:   {}", args.workspace.display());

    let (control_plane, state_dir) = build_control_plane(&args);
    let control_plane = Arc::new(control_plane);
    let tracker = SessionTracker::new(&state_dir);

    let mut session_id = if let Some(res) = &args.resume_session_id {
        human!(format, "  Session:     Resuming {}", res);
        res.clone()
    } else if args.continue_last {
        if let Some(last) = tracker.get_last() {
            human!(format, "  Session:     Continuing {}", last);
            last
        } else {
            let fresh = format!("cli-session-{}", uuid::Uuid::new_v4());
            human!(
                format,
                "  Session:     No previous session found, created {}",
                fresh
            );
            fresh
        }
    } else {
        let fresh = format!("cli-session-{}", uuid::Uuid::new_v4());
        fresh
    };

    tracker.record(&session_id);

    // 检查 stdin 管道是否有数据
    let piped_input = read_stdin_if_piped();
    let final_prompt = match (args.prompt.clone(), piped_input) {
        (Some(p), Some(pipe)) => Some(format!("Context piped from stdin:\n```\n{}\n```\n\nPrompt:\n{}", pipe.trim(), p)),
        (None, Some(pipe)) => Some(format!("Context piped from stdin:\n```\n{}\n```", pipe.trim())),
        (Some(p), None) => Some(p),
        (None, None) => None,
    };

    if let Some(prompt) = final_prompt {
        // 单发（无头）路径：退出码＝终态映射（契约 C2）。
        let execution = run_single_turn(&control_plane, &args, &options, &session_id, &prompt);
        flush_events(&execution.events);
        let exit_code = persist_turn_result(&execution, &options);
        finish(exit_code, observability);
    }

    // v5 B-BLOCKING-2：jsonl/sse 是机读流模式，不接受交互式 REPL —— 否则失败轮次会以
    // `finish(Success)` 收口，进程退 0 而封套写 failed/4，C2 与 C3 同时被击穿。
    if format != OutputFormat::Text {
        let label = match format {
            OutputFormat::Jsonl => "jsonl",
            OutputFormat::Sse => "sse",
            OutputFormat::Text => unreachable!("text 模式不会进入本分支"),
        };
        eprintln!(
            "[pony-headless] --output-format {label} 是机读流模式，不接受交互式 REPL；请用 -p <prompt> 或通过 stdin 管道传入输入（Usage，exit 2）"
        );
        finish(ExitCode::Usage, observability);
    }

    human!(format, "  Session ID:  {}", session_id);
    human!(format, "\nType your message below (or '/help' for options, '/exit' to quit):");
    let stdin = io::stdin();
    let mut reader = stdin.lock();

    loop {
        human_inline!(format, "\n\x1b[1;35mUser > \x1b[0m");
        let _ = io::stdout().flush();

        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if let Some(cmd) = SlashCommand::parse(trimmed) {
            match cmd {
                SlashCommand::Exit => {
                    human!(format, "Goodbye!");
                    break;
                }
                SlashCommand::Help => {
                    print_help(format);
                    continue;
                }
                SlashCommand::Clear => {
                    session_id = format!("cli-session-{}", uuid::Uuid::new_v4());
                    tracker.record(&session_id);
                    human!(
                        format,
                        "\x1b[32m✔ Context cleared. Switched to new session: {}\x1b[0m",
                        session_id
                    );
                    continue;
                }
                SlashCommand::Compact => {
                    // main 侧契约 C-4：`/compact` 走控制面真实压缩（C-2/C-1），
                    // 不再用「发一条请总结对话的提示词」伪装压缩。
                    // 文案复用 core 纯函数（审查 F4），TUI/CLI 同一套措辞。
                    let result = control_plane.compact_session(Some(session_id.as_str()), true);
                    let color = match &result {
                        Ok(outcome) if outcome.compacted => "\x1b[32m",
                        Ok(_) => "\x1b[33m",
                        Err(_) => "\x1b[31m",
                    };
                    human!(format, "{color}{}\x1b[0m", compact_outcome_message(&result));
                    continue;
                }
                SlashCommand::Unknown(u) => {
                    human!(
                        format,
                        "\x1b[31mUnknown command: {}\x1b[0m (type /help for available commands)",
                        u
                    );
                    continue;
                }
            }
        }

        let execution = run_single_turn(&control_plane, &args, &options, &session_id, trimmed);
        flush_events(&execution.events);
        let _ = persist_turn_result(&execution, &options);
        // 交互式多轮：单轮失败不终止 REPL（退出码沿用基线语义 = 0）。
    }

    finish(ExitCode::Success, observability);
}
