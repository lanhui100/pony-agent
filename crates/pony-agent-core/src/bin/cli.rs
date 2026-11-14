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
    ProviderModelCapabilities, ProviderRegistryStore, ProviderSelectionResolver,
    ResolvedProviderSelection, ThinkingParamPattern, UpstreamPolicy,
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
use pony_agent_core::agent::session::{
    new_session_id, new_session_id_unless, FileSessionBackend, SessionStore,
};
use pony_agent_core::agent::turn_flow::TurnEventSink;
use std::collections::HashSet;
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
    /// PA-172 §3.5 W1：配置面真值源（上游时限/重试策略 + 故障转移候选链的唯一来源）。
    /// 选择算法本身仍走 gateway 兜底分支（`resolve_provider_selection` 未改）；
    /// 本字段只服务 `upstream_policy()` / `resolve_failover_chain()` 的转发。
    registry: ProviderRegistryStore,
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
                context_window_tokens: Some(
                    pony_agent_core::agent::config::DEFAULT_CONTEXT_WINDOW_TOKENS,
                ),
                ..Default::default()
            },
            thinking_param_pattern: ThinkingParamPattern::None,
        }
    }

    /// PA-172 §3.5 W1（生产注入闭环，无头形态）：上游时限/重试策略必须由注册表真实提供。
    ///
    /// 不覆写 ⇒ trait 默认实现恒返回 [`UpstreamPolicy::default`]，CLI 出来的
    /// `ProviderManager` 永远拿默认值、用户的 `upstreamPolicy` 配置静默失效。
    fn upstream_policy(&self) -> UpstreamPolicy {
        self.registry.upstream_policy()
    }

    /// PA-172 §3.5 W1：换手候选链同样转发注册表 —— 不覆写 ⇒ 链恒空 ⇒
    /// 故障转移在 CLI 形态**永久不触发**（矩阵 W1 判据：两个 bin 都要命中）。
    fn resolve_failover_chain(
        &self,
        primary: &ResolvedProviderSelection,
    ) -> Vec<ResolvedProviderSelection> {
        self.registry.resolve_failover_chain(primary)
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
/// 把 `-w` 绑定成会话的 workspace 归属（r1-dim6 F1 修复）。
///
/// 根因：`run_single_turn` 此前恒传 `workspace_id: None`，会话不盖章 ⇒
/// `resolve_session_workspace_root` 回落注册表 `default`，宿主 `-w` 被覆盖
/// （重放实测 9/9 会话 env cwd = 注册表 default，而非臂 worktree）。
///
/// 语义与 TUI 启动路径同构（`tui.rs:6995-6999`）：先 `ensure_workspace_for_root`
/// 幂等登记 `-w` 指向的目录，再把返回的 `record.id` 作为 `TurnInput.workspace_id`
/// 的唯一来源。登记失败 ⇒ 返回 `None` 并保持旧行为（不 panic，不静默换根）。
fn bind_workspace_id(
    control_plane: &HostControlPlane,
    workspace: &std::path::Path,
) -> Option<String> {
    let root = workspace.display().to_string();
    match control_plane.ensure_workspace_for_root(&root) {
        Ok(record) => Some(record.id),
        Err(error) => {
            eprintln!("[ponycli] 工作区登记失败（沿用默认归属）：{error}");
            None
        }
    }
}

fn run_single_turn(
    control_plane: &Arc<HostControlPlane>,
    args: &CliArgs,
    options: &HeadlessOptions,
    session_id: &str,
    prompt: &str,
    workspace_id: Option<&str>,
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
            workspace_id: workspace_id.map(str::to_string),
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

/// CLI 会话 id：格式与桌面端 / TUI 一致（`pa_<uuidv7>`，真值源
/// [`pony_agent_core::agent::session::new_session_id`]）。
///
/// CLI 用独立的 `cli-sessions.json` 存储，键即 session id；生成前按已有会话键去重，
/// 避免历史格式（32 bit 随机段）那种「新会话覆盖旧会话」的键碰撞风险。
fn new_cli_session_id(control_plane: &HostControlPlane) -> String {
    let existing: HashSet<String> = control_plane
        .list_sessions()
        .into_iter()
        .map(|overview| overview.conversation_id)
        .collect();
    new_session_id_unless(|candidate| existing.contains(candidate)).unwrap_or_else(new_session_id)
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
        // 与 TUI 同一真值源（`~/.config/pony-agent/providers.json`）；只读转发，不写回。
        registry: ProviderRegistryStore::new(),
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
    human!(format, "Maintenance subcommands (ADR 0099):");
    human!(format, "  ponycli sessions backfill [--dry-run] [--db <path>]");
    human!(format, "  ponycli sessions rebuild-index [--dry-run | --explain <session-id>] [--db <path>]");
    human!(format, "                             --explain: per-field diff summary (counts + first 80 chars),");
    human!(format, "                             prints `no difference` when the log fold matches the index");
    human!(format, "  ponycli sessions export <session-id> --out <path> [--db <path>] [--format dir|tar]");
    human!(format, "                             Bundle = manifest.json + events.jsonl + attachments/;");
    human!(format, "                             `tar` is uncompressed ustar (single file, no zstd yet)");
    human!(format, "  ponycli sessions migrate-logs [--dry-run] [--db <path>] [--force]");
    human!(format, "                             Grandfather→log backfill (synthesized streams are");
    human!(format, "                             tagged source.kind=migrated-from-blob; blob kept for rollback)");
    human!(format, "  ponycli sessions vacuum [--dry-run] [--db <path>]");
    human!(format, "                             Reclaim freelist holes (auto_vacuum=FULL +");
    human!(format, "                             VACUUM) and truncate WAL. Disk footprint only —");
    human!(format, "                             holes do NOT slow startup (A/B: identical read bytes).");
    human!(format, "                             Takes `<db>.lock`; exit 3 when held elsewhere");
    human!(format, "  ponycli sessions import <path> [--db <path>] [--dry-run] [--force]");
    human!(format, "                             Import writes ONLY the session log + attachments:");
    human!(format, "                             SQLite index is rebuilt by `sessions rebuild-index`.");
    human!(format, "                             manifest is not self-hashed; the event stream is");
    human!(format, "                             structurally re-validated (header/seq/eventCount)");
    human!(format, "  ponycli config effective   One-line JSON: disk_policy vs effective_policy");
    human!(format, "                             (+ clamped_fields, aggregate_budget_ms,");
    human!(format, "                             per_attempt_effective_timeout_ms); no secrets");
    human!(format, "Sessions subcommand exit codes: 0 ok / 2 invalid bundle / 3 target exists or maintenance lock busy / 4 degraded");
    human!(format, "");
    human!(format, "ACP agent host mode (B151/P2):");
    human!(format, "  ponycli acp --list");
    human!(format, "  ponycli acp --agent <id> -p <text> [--output-format text|jsonl|sse]");
    human!(format, "              [--workspace <dir>] [--timeout-secs <n>] [--resume <session-id>]");
    human!(format, "  --agent     ACP agent id from acp-agents.json (opencode | antigravity | ...)");
    human!(format, "  --list      Print configured agents as JSON lines ({{id,label,command,args,builtin}})");
    human!(format, "  --resume    Resume a session; unsupported agents fail closed with acp_unavailable");
    human!(format, "  --output-format jsonl/sse: stdout = events only (no banner, no REPL)");
    human!(format, "  --timeout-secs <n>  Bound the delegated turn (0..=86400); expiry = exit 7");
    human!(format, "  Run `ponycli acp --help` for the full ACP section.");
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
/// 日志的缺省输出目标改道 stderr（`otel::log::set_stderr_output`）——text 模式行为不变；
/// TUI 在自己的 stderr 成功重定向到运行日志后也调用同一开关（ADR 0063），Tauri 不调用。
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
    /// 退出清理（正常 return / 主循环结束 / Ctrl-C 交由 TUI 循环返回后）：先有界 flush，
    /// 再有界 shutdown（各 1000ms，合计 ≤3000ms，幂等，不因网络悬挂卡死退出）。
    fn drop(&mut self) {
        // 先排空 issue 上报队列（有界 1500ms，避免 `--prompt` 这类「报错即退出」丢 issue），
        // 再有界 flush 未到调度周期的尾批 span（r2-dim1 F1），最后关闭 OTel provider。
        pony_agent_core::agent::ponysentry::flush();
        pony_agent_core::agent::otel::flush_on_exit("cli-drop");
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

/// ADR 0099 阶段 0（E8 / I4）：孤儿会话回填维护入口。
///
/// `ponycli sessions backfill [--dry-run] [--db <path>]`
/// - 默认操作 `~/.local/share/PonyAgent/sessions.db`（`--db` 可指向副本）；
/// - `--dry-run` 只报告影响面（`backfilled` / `retired` / `checksum_mismatch`），**不写库**；
/// - 回填源只允许 `turn_events ∪ normalized_*`，绝不以 `sessions` 表为源。
fn run_sessions_backfill_if_requested() -> Option<i32> {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) != Some("sessions")
        || args.get(2).map(String::as_str) != Some("backfill")
    {
        return None;
    }
    let dry_run = args.iter().any(|arg| arg == "--dry-run");
    let db_path = args
        .iter()
        .position(|arg| arg == "--db")
        .and_then(|index| args.get(index + 1))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(pony_agent_core::agent::sqlite_session::default_sqlite_path);
    // `--db` 只接受「已存在 + .db 后缀 + 会话库结构自检通过」的路径，拒绝任意路径注入。
    if let Err(error) = validate_backfill_db(&db_path) {
        eprintln!("[ponycli] sessions backfill 参数错误：{error}");
        return Some(2);
    }
    let backend = if dry_run {
        // 预演必须零写入：`mode=ro` + `query_only`，不建表、不迁移、不改 journal_mode。
        pony_agent_core::agent::sqlite_session::SqliteSessionBackend::new_read_only(db_path.clone())
    } else {
        pony_agent_core::agent::sqlite_session::SqliteSessionBackend::new_with_trace_mode(
            db_path.clone(),
            pony_agent_core::agent::session::SeparateTraceTableMode::WriteSeparate,
        )
    };
    let result = if dry_run {
        backend.backfill_orphan_sessions_dry_run()
    } else {
        backend.backfill_orphan_sessions()
    };
    match result {
        Ok(report) => {
            println!("db: {}", db_path.display());
            println!(
                "mode: {}",
                if dry_run {
                    "dry-run（只报告，不写库）"
                } else {
                    "apply（写回 blob 行）"
                }
            );
            println!(
                "backfilled({}): {:?}",
                report.backfilled.len(),
                report.backfilled
            );
            println!("retired({}): {:?}", report.retired.len(), report.retired);
            println!(
                "checksum_mismatch({}): {:?}",
                report.checksum_mismatch.len(),
                report.checksum_mismatch
            );
            Some(0)
        }
        Err(error) => {
            eprintln!("[ponycli] sessions backfill 失败：{error}");
            Some(1)
        }
    }
}

/// `ponycli sessions rebuild-index [--dry-run | --explain <session-id>] [--db <path>]`
///
/// ADR 0099 阶段 2：**从会话日志（真源）fold 出状态并重建 SQLite 索引**。
/// - 只覆盖「有日志文件」的会话；索引里有、日志里没有的会话**原封不动**（计入
///   `skipped_no_log`）——存量库的会话都还没有日志文件，误当成空会话会清空真实历史；
/// - 墓碑（`RemoveSession`）不重建：删除单调不可复活；
/// - `--dry-run` 只报告影响面（含 `mismatched` 双读差异），**不写库**；
/// - `--explain <session-id>`（阶段 3-D）：只读诊断，逐字段打印差异摘要
///   （`字段 | fold: … | index: …`，只有条数 + 首条前 80 字符，**不打印大字段全文**），
///   无差异打印 `no difference`；与 `--dry-run` **互斥**（explain 本身零写入）；
/// - `--db` 复用 `sessions backfill` 的同一条校验（不存在/非 .db/缺 sessions 表 ⇒ EXIT=2）。
fn run_sessions_rebuild_index_if_requested() -> Option<i32> {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) != Some("sessions")
        || args.get(2).map(String::as_str) != Some("rebuild-index")
    {
        return None;
    }
    let dry_run = args.iter().any(|arg| arg == "--dry-run");
    let explain_target = flag_value(&args, "--explain");
    let db_path = args
        .iter()
        .position(|arg| arg == "--db")
        .and_then(|index| args.get(index + 1))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(pony_agent_core::agent::sqlite_session::default_sqlite_path);
    if let Err(error) = validate_backfill_db(&db_path) {
        eprintln!("[ponycli] sessions rebuild-index 参数错误：{error}");
        return Some(2);
    }
    // 阶段 3-D：`--explain <id>` 与 `--dry-run` 互斥（explain 只读诊断、本就零写入）。
    if explain_target.is_some() && dry_run {
        eprintln!("[ponycli] `--explain <session-id>` 与 `--dry-run` 互斥：explain 已是零写入诊断");
        return Some(2);
    }
    if let Some(session_id) = explain_target {
        let backend = pony_agent_core::agent::sqlite_session::SqliteSessionBackend::new_read_only(
            db_path.clone(),
        );
        let log_root = pony_agent_core::agent::session::session_log_root_for_db(&db_path);
        match backend.explain_session_parity(&session_id) {
            Ok(lines) => {
                println!("db: {}", db_path.display());
                println!("log-root: {}", log_root.display());
                println!("mode: explain（只读诊断，不写库）");
                println!("session: {session_id}");
                for line in &lines {
                    println!("{line}");
                }
                println!(
                    "note: 摘要只含条数与首条前 80 字符；比较口径与 rebuild-index 的 mismatched 同源"
                );
                return Some(0);
            }
            Err(error) => {
                eprintln!("[ponycli] sessions rebuild-index --explain 失败：{error}");
                return Some(1);
            }
        }
    }
    let backend = if dry_run {
        pony_agent_core::agent::sqlite_session::SqliteSessionBackend::new_read_only(db_path.clone())
    } else {
        pony_agent_core::agent::sqlite_session::SqliteSessionBackend::new_with_trace_mode(
            db_path.clone(),
            pony_agent_core::agent::session::SeparateTraceTableMode::WriteSeparate,
        )
    };
    let options = pony_agent_core::agent::session::RebuildOptions { dry_run };
    match pony_agent_core::agent::session::rebuild_index(&backend, options) {
        Ok(report) => {
            println!("db: {}", db_path.display());
            println!(
                "log-root: {}",
                pony_agent_core::agent::session::session_log_root_for_db(&db_path).display()
            );
            println!(
                "mode: {}",
                if dry_run {
                    "dry-run（只报告，不写索引）"
                } else {
                    "apply（用 fold 结果覆盖索引）"
                }
            );
            println!("summary: {}", report.summary_line(dry_run));
            println!("scanned: {}", report.scanned);
            println!("rebuilt({}): {:?}", report.rebuilt.len(), report.rebuilt);
            println!("skipped_no_log: {}", report.skipped_no_log);
            println!(
                "skipped_deleted({}): {:?}",
                report.skipped_deleted.len(),
                report.skipped_deleted
            );
            println!("failed({}): {:?}", report.failed.len(), report.failed);
            println!("mismatched({}): {:?}", report.mismatched.len(), report.mismatched);
            Some(0)
        }
        Err(error) => {
            eprintln!("[ponycli] sessions rebuild-index 失败：{error}");
            Some(1)
        }
    }
}

/// 会话维护子命令的统一入口（`main` 里只留一行拦截，避免主函数继续膨胀）。
///
/// 顺序即优先级，互斥（命令行只能命中其中一个），返回值 = 进程退出码：
/// `backfill`（阶段 0）、`rebuild-index`（阶段 2-A）、`export` / `import`（阶段 3-A）、
/// `migrate-logs`（阶段 3-B）、`config effective`（PA-174 F3 生效策略观测）。
/// 参数里出现 `--help` / `-h` ⇒ 维护子命令必须让路（零写入、零副作用）。
///
/// 事故现场（2026-11-10）：维护子命令在常规参数解析之前拦截，因此 `ponycli sessions vacuum --help`
/// 曾**真的执行**了破坏性维护——实测把真实库 `~/.local/share/PonyAgent/sessions.db` 从 1144MB
/// VACUUM 到 743MB（`integrity_check=ok`、行数不变，但不是用户意图）。
fn is_help_request(args: &[String]) -> bool {
    args.iter().any(|arg| arg == "--help" || arg == "-h")
}

/// 维护子命令的统一入口。
///
/// **安全护栏（2026-11-10 事故现场）**：维护子命令在常规参数解析之前拦截，因此
/// `ponycli sessions vacuum --help` 曾经**真的执行**了破坏性维护（实测把真实库
/// `~/.local/share/PonyAgent/sessions.db` 从 1144MB VACUUM 到 743MB——数据完好，但不是用户意图）。
/// 现在任何 `--help` / `-h` 一律不进维护分支：交回常规解析器打印用法，零写入、零副作用。
fn run_maintenance_subcommand_if_requested() -> Option<i32> {
    let argv: Vec<String> = std::env::args().collect();
    if is_help_request(&argv) {
        // 维护命名空间下的 `--help` 明确打印用法（而不是把 `--db` 当成未知参数报错）。
        if matches!(argv.get(1).map(String::as_str), Some("sessions") | Some("config")) {
            print_usage(OutputFormat::Text);
            return Some(0);
        }
        return None;
    }
    run_sessions_backfill_if_requested()
        .or_else(run_sessions_rebuild_index_if_requested)
        .or_else(run_sessions_export_if_requested)
        .or_else(run_sessions_import_if_requested)
        .or_else(run_sessions_migrate_logs_if_requested)
        .or_else(run_sessions_vacuum_if_requested)
        .or_else(run_config_effective_if_requested)
}

/// `ponycli config effective`：打印**生效策略**报告（单行 JSON 到 stdout，exit 0）。
///
/// PA-174 F3：`load_view()` 给磁盘原值（PA-173 D1 后越界值不再被改写），`upstream_policy()`
/// 给钳制后的生效值；二者一起由 `upstream_policy_report` 暴露，让「配置被钳制」这件事可见，
/// 而不是用户永远只看到自己填的零值。报告只含三个策略字段，**不含任何密钥**。
fn run_config_effective_if_requested() -> Option<i32> {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) != Some("config")
        || args.get(2).map(String::as_str) != Some("effective")
    {
        return None;
    }
    let store = ProviderRegistryStore::new();
    let disk_policy = store.load_view().upstream_policy;
    let effective_policy = store.upstream_policy();
    let report =
        pony_agent_core::agent::config::upstream_policy_report(&disk_policy, &effective_policy);
    match serde_json::to_string(&report) {
        Ok(line) => {
            println!("{line}");
            Some(0)
        }
        Err(error) => {
            eprintln!("[ponycli] config effective 序列化失败：{error}");
            Some(1)
        }
    }
}

/// `ponycli sessions migrate-logs [--dry-run] [--db <path>] [--force]`
///
/// ADR 0099 阶段 3-B（方案 A）：把**只有索引 blob、没有日志**的存量会话（grandfather）
/// 转正为日志真源 —— 从 blob 状态合成事件流，写临时目录、`fold` 回来**逐字段比对**，
/// 一致才原子发布到正式日志路径。
///
/// 三条用户可见承诺（帮助文本与输出都写明）：
/// - **迁移后保留 blob 作回滚面**（不删 `sessions` 行；出问题删日志目录即回到老读路径）；
/// - 合成日志已在 header 标记 `source.kind = "migrated-from-blob"`，**不是原始过程流**；
/// - 不一致的会话**不落盘、不改原数据**，只在报告里列出。
///
/// 退出码：`0` 全部成功或无事可做 / `2` 参数或库非法 / `5` 存在 mismatched 或 failed。
fn run_sessions_migrate_logs_if_requested() -> Option<i32> {
    use pony_agent_core::agent::session::{migrate_grandfathered_sessions, MigrateOptions};
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) != Some("sessions")
        || args.get(2).map(String::as_str) != Some("migrate-logs")
    {
        return None;
    }
    let dry_run = args.iter().any(|arg| arg == "--dry-run");
    let force = args.iter().any(|arg| arg == "--force");
    let db_path = flag_value(&args, "--db")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(pony_agent_core::agent::sqlite_session::default_sqlite_path);
    if let Err(error) = validate_backfill_db(&db_path) {
        eprintln!("[ponycli] sessions migrate-logs 参数错误：{error}");
        return Some(2);
    }
    // 预演必须零写入：`mode=ro` + `query_only`，不建表、不迁移、不改 journal_mode。
    let backend = if dry_run {
        pony_agent_core::agent::sqlite_session::SqliteSessionBackend::new_read_only(db_path.clone())
    } else {
        pony_agent_core::agent::sqlite_session::SqliteSessionBackend::new_with_trace_mode(
            db_path.clone(),
            pony_agent_core::agent::session::SeparateTraceTableMode::WriteSeparate,
        )
    };
    let log_root = pony_agent_core::agent::session::session_log_root_for_db(&db_path);
    match migrate_grandfathered_sessions(&backend, MigrateOptions { dry_run, force }) {
        Ok(report) => {
            println!("db: {}", db_path.display());
            println!("log-root: {}", log_root.display());
            println!(
                "mode: {}",
                if dry_run {
                    "dry-run（只合成+校验，零写入）"
                } else {
                    "apply（写合成日志；**保留 blob 作回滚面**）"
                }
            );
            println!("summary: {}", report.summary_line());
            println!("scanned: {}", report.scanned);
            println!("migrated({}): {:?}", report.migrated.len(), report.migrated);
            println!("skipped_has_log: {}", report.skipped_has_log);
            println!("remigrated({}): {:?}", report.remigrated.len(), report.remigrated);
            println!("mismatched({}):", report.mismatched.len());
            for (session_id, reason) in &report.mismatched {
                println!("  - {session_id}: {reason}");
            }
            println!("failed({}):", report.failed.len());
            for (session_id, reason) in &report.failed {
                println!("  - {session_id}: {reason}");
            }
            println!(
                "note: 合成日志 header 标记 source.kind = \"migrated-from-blob\"（不是原始过程流）；blob 行保留作回滚面"
            );
            println!(
                "note: 索引不受影响；这些会话此后「删掉索引行仍可读」由日志真源保证"
            );
            Some(report.exit_code())
        }
        Err(error) => {
            eprintln!("[ponycli] sessions migrate-logs 失败：{error}");
            Some(1)
        }
    }
}

/// `ponycli sessions vacuum [--dry-run] [--db <path>]`
///
/// B046：会话库的**磁盘占用**回收（与启动性能无关，别拿「提速」当理由改它）：
/// `DELETE` 一行大 trace 会一次性释放整块页（实测 +1066 页 / 4.2MB），而库是
/// `auto_vacuum=OFF` ⇒ 这些页**永不复用**、文件只增不减（真实库 1.2GB 里 455MB 是空洞）。
/// 同一份快照 A/B 已证伪「空洞拖慢启动」：首帧前读盘字节完全一致（SQLite 扫表不碰 freelist）。
///
/// 本命令做两件事，**必须一起做**：
/// 1. `PRAGMA auto_vacuum=FULL` + `VACUUM`（只 VACUUM 不改模式 ⇒ 空洞会再涨回来；
///    INCREMENTAL 实测收不动，见 `reclaim_session_store_space` 的对照表）；
/// 2. `wal_checkpoint(TRUNCATE)` 把 `-wal` 归零（真实库曾积 431MB）。
///
/// `--dry-run`：只读探针，打印 `page_size / page_count / freelist / auto_vacuum / -wal`，
/// 对库文件零写入。非 dry-run 是**破坏性维护**：必须拿到 `<db>.lock`，抢不到即拒绝。
///
/// 退出码：`0` 成功 / `2` 参数或库非法 / `3` 维护锁被其他进程持有 / `1` 执行失败。
fn run_sessions_vacuum_if_requested() -> Option<i32> {
    use pony_agent_core::agent::sqlite_session::{
        inspect_session_store, reclaim_session_store_space,
    };
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) != Some("sessions")
        || args.get(2).map(String::as_str) != Some("vacuum")
    {
        return None;
    }
    let dry_run = args.iter().any(|arg| arg == "--dry-run");
    let db_path = flag_value(&args, "--db")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(pony_agent_core::agent::sqlite_session::default_sqlite_path);
    if let Err(error) = validate_backfill_db(&db_path) {
        eprintln!("[ponycli] sessions vacuum 参数错误：{error}");
        return Some(2);
    }
    println!("db: {}", db_path.display());
    if dry_run {
        return match inspect_session_store(&db_path) {
            Ok(footprint) => {
                println!("mode: dry-run（只读探针，零写入）");
                println!("footprint: {}", footprint.summary_line());
                println!(
                    "reclaimable: {}MB（freelist {} 页）",
                    footprint.freelist_bytes() / (1024 * 1024),
                    footprint.freelist_pages
                );
                println!(
                    "note: auto_vacuum={} —— {}",
                    footprint.auto_vacuum,
                    if footprint.auto_vacuum == 1 {
                        "已是 FULL（删除即回收），apply 只做一次 VACUUM 收掉既有空洞"
                    } else {
                        "apply 会切到 `auto_vacuum=FULL` 并整库 VACUUM（约 2 倍临时空间）"
                    }
                );
                Some(0)
            }
            Err(error) => {
                eprintln!("[ponycli] sessions vacuum --dry-run 失败：{error}");
                Some(1)
            }
        };
    }
    match reclaim_session_store_space(&db_path) {
        Ok(report) => {
            println!(
                "mode: apply（{}）",
                if report.switched_to_full {
                    "auto_vacuum=FULL + VACUUM（**模式已切换：此后删除自己回收空间，不再留洞**）"
                } else {
                    "已是 FULL，只做一次 VACUUM 收掉既有空洞"
                }
            );
            println!(
                "before: {}MB freelist={} 页",
                report.db_bytes_before / (1024 * 1024),
                report.freelist_pages_before
            );
            println!(
                "after:  {}MB freelist={} 页",
                report.db_bytes_after / (1024 * 1024),
                report.freelist_pages_after
            );
            println!(
                "auto_vacuum: {} → {}（page_size={}KB elapsed={}ms）",
                report.auto_vacuum_before,
                report.auto_vacuum_after,
                report.page_size_bytes / 1024,
                report.elapsed_ms
            );
            println!("summary: {}", report.summary_line());
            Some(0)
        }
        Err(error) => {
            if error.starts_with("maintenance_lock_busy") {
                eprintln!("[ponycli] sessions vacuum 拒绝执行：{error}");
                return Some(3);
            }
            eprintln!("[ponycli] sessions vacuum 失败：{error}");
            Some(1)
        }
    }
}

/// `ponycli sessions export <session-id> --out <path> [--db <path>] [--format dir|tar]`
///
/// ADR 0099 阶段 3-A（D5 传输格式的第一块）：把会话日志导出成**目录包**
/// （`manifest.json` + `events.jsonl` + `attachments/`）或**单文件包**（最小 ustar `.tar`，
/// 不压缩；压缩形态需要新增依赖，留作后续 ADR）。
///
/// 退出码：`0` 全成功 / `2` 参数或校验失败（会话不存在、目标已存在）/ `4` 包已产出但
/// 附件降级（附件实体缺失等，报告逐条列出）/ `1` IO 失败。
fn run_sessions_export_if_requested() -> Option<i32> {
    use pony_agent_core::agent::session::{
        export_session, session_log_root_for_db, BundleFormat,
    };
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) != Some("sessions")
        || args.get(2).map(String::as_str) != Some("export")
    {
        return None;
    }
    let Some(session_id) = args.get(3).filter(|arg| !arg.starts_with("--")).cloned() else {
        eprintln!("[ponycli] sessions export 用法：ponycli sessions export <session-id> --out <path> [--db <path>] [--format dir|tar]");
        return Some(2);
    };
    let Some(out_raw) = flag_value(&args, "--out") else {
        eprintln!("[ponycli] sessions export 缺 `--out <path>`（目录形态指向目录，tar 形态指向 .tar 文件）");
        return Some(2);
    };
    if out_raw == "-" {
        eprintln!("[ponycli] sessions export 暂不支持 `--out -`（stdout 字节流）：请指定文件路径；单文件形态用 `--format tar`");
        return Some(2);
    }
    let format_raw = flag_value(&args, "--format").unwrap_or_else(|| "dir".to_string());
    let Some(format) = BundleFormat::parse(&format_raw) else {
        eprintln!("[ponycli] sessions export 不支持的 `--format {format_raw}`（可选 `dir` / `tar`）");
        return Some(2);
    };
    let db_path = flag_value(&args, "--db")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(pony_agent_core::agent::sqlite_session::default_sqlite_path);
    let log_root = session_log_root_for_db(&db_path);
    let out_path = std::path::PathBuf::from(&out_raw);
    let attachment_source = db_path.is_file().then_some(db_path.as_path());
    match export_session(
        &log_root,
        &session_id,
        &out_path,
        format,
        env!("CARGO_PKG_VERSION"),
        attachment_source,
    ) {
        Ok(report) => {
            println!("db: {}", db_path.display());
            println!("log-root: {}", log_root.display());
            println!("session: {}", report.session_id);
            println!("cwd: {}", report.cwd);
            println!("out: {}", report.out.display());
            println!("format: {}", report.format_label);
            println!("events: {}", report.event_count);
            println!("lastSeq: {:?}", report.last_seq);
            println!("attachments({}): {:?}", report.attachments.len(), report.attachments);
            println!("bytes: {}", report.total_bytes);
            println!("files({}): {:?}", report.files.len(), report.files);
            if report.warnings.is_empty() {
                println!("warnings: 0");
            } else {
                println!("warnings({}):", report.warnings.len());
                for warning in &report.warnings {
                    println!("  - {warning}");
                }
                println!("注意：附件降级（EXIT=4）——事件已完整入包，附件缺失/不可读项见上表");
            }
            Some(report.exit_code())
        }
        Err(error) => {
            eprintln!("[ponycli] sessions export 失败（EXIT={}）：{error}", error.exit_code());
            Some(error.exit_code())
        }
    }
}

/// `ponycli sessions import <path> [--db <path>] [--dry-run] [--force]`
///
/// ADR 0099 阶段 3-A：把会话包导入日志根（`<db 目录>/session-logs/`）。
///
/// **只落日志文件与附件，不写 SQLite 索引** —— 索引是派生视图，导入后请运行
/// `ponycli sessions rebuild-index` 补齐（这正是「索引可删可建」的兑现路径）。
///
/// 退出码：`0` 全成功 / `2` 包校验失败（manifest/format/version/sha256/header/seq，
/// **零落盘**）/ `3` 目标会话已存在且未加 `--force` / `4` 事件导入成功但附件降级 /
/// `1` IO 失败。
fn run_sessions_import_if_requested() -> Option<i32> {
    use pony_agent_core::agent::session::{import_bundle, session_log_root_for_db, ImportOptions};
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) != Some("sessions")
        || args.get(2).map(String::as_str) != Some("import")
    {
        return None;
    }
    let Some(bundle_path) = args.get(3).filter(|arg| !arg.starts_with("--")).cloned() else {
        eprintln!("[ponycli] sessions import 用法：ponycli sessions import <path> [--db <path>] [--dry-run] [--force]");
        return Some(2);
    };
    let db_path = flag_value(&args, "--db")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(pony_agent_core::agent::sqlite_session::default_sqlite_path);
    let log_root = session_log_root_for_db(&db_path);
    let options = ImportOptions {
        dry_run: args.iter().any(|arg| arg == "--dry-run"),
        force: args.iter().any(|arg| arg == "--force"),
    };
    match import_bundle(&log_root, std::path::Path::new(&bundle_path), options) {
        Ok(report) => {
            println!("db: {}", db_path.display());
            println!("log-root: {}", log_root.display());
            println!("session: {}", report.session_id);
            println!("cwd: {}", report.cwd);
            println!("target: {}", report.target_dir.display());
            println!("mode: {}", if report.dry_run { "dry-run（只校验，零写入）" } else { "apply（写日志与附件）" });
            println!("events: {}", report.event_count);
            println!("lastSeq: {:?}", report.last_seq);
            println!("attachments({}): {:?}", report.attachments.len(), report.attachments);
            if let Some(backup) = &report.backup {
                println!("backup: {}", backup.display());
            }
            if report.warnings.is_empty() {
                println!("warnings: 0");
            } else {
                println!("warnings({}):", report.warnings.len());
                for warning in &report.warnings {
                    println!("  - {warning}");
                }
                println!("注意：附件降级（EXIT=4）——事件已完整导入，附件缺失/损坏项见上表");
            }
            println!(
                "note: manifest 不参与 sha256 自校验，故导入时对事件流做了结构性校验兜底（header/seq/eventCount）"
            );
            if !report.dry_run {
                println!(
                    "note: 导入只落日志与附件，**不写 SQLite 索引**；请运行 `ponycli sessions rebuild-index` 补齐索引"
                );
            }
            Some(report.exit_code())
        }
        Err(error) => {
            eprintln!("[ponycli] sessions import 失败（EXIT={}）：{error}", error.exit_code());
            Some(error.exit_code())
        }
    }
}

/// 取 `--flag <value>` 的值（缺失或后面又是另一个 flag ⇒ None）。
fn flag_value(args: &[String], flag: &str) -> Option<String> {
    let index = args.iter().position(|arg| arg == flag)?;
    let value = args.get(index + 1)?;
    if value.starts_with("--") {
        return None;
    }
    Some(value.clone())
}

/// ADR 0099 阶段 0（E8）：回填目标的路径/结构自检。
///
/// 拒绝：不存在、非普通文件、非 `.db` 后缀、缺 `sessions` 表（不是 PonyAgent 会话库）。
/// 自检本身用**只读**连接，不产生任何写入。
fn validate_backfill_db(path: &std::path::Path) -> Result<(), String> {
    if !path.exists() {
        return Err(format!("数据库不存在：{}", path.display()));
    }
    if !path.is_file() {
        return Err(format!("不是普通文件：{}", path.display()));
    }
    let is_db_suffix = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.eq_ignore_ascii_case("db"))
        .unwrap_or(false);
    if !is_db_suffix {
        return Err(format!("拒绝非 .db 文件（防路径注入）：{}", path.display()));
    }
    let conn =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|error| format!("无法以只读方式打开 {}：{error}", path.display()))?;
    let has_sessions: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type IN ('table','view') AND name = 'sessions'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| format!("{} 结构自检失败：{error}", path.display()))?;
    if has_sessions == 0 {
        return Err(format!(
            "{} 不是 PonyAgent 会话库（缺 sessions 表）",
            path.display()
        ));
    }
    Ok(())
}

fn main() {
    // ADR 0099 维护子命令必须在常规参数解析之前拦截（否则会被当成非法参数）。
    if let Some(code) = run_maintenance_subcommand_if_requested() {
        std::process::exit(code);
    }
    // B151/P2：ACP 宿主形态同属「解析之前拦截」的命名空间（`ponycli acp ...`）。
    if let Some(code) = run_acp_subcommand_if_requested() {
        std::process::exit(code);
    }
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
    // r1-dim6 F1：把 `-w` 登记为 workspace 并作为会话归属的唯一来源（与 TUI 同构）。
    let bound_workspace_id = bind_workspace_id(&control_plane, &args.workspace);
    let tracker = SessionTracker::new(&state_dir);

    let mut session_id = if let Some(res) = &args.resume_session_id {
        human!(format, "  Session:     Resuming {}", res);
        res.clone()
    } else if args.continue_last {
        if let Some(last) = tracker.get_last() {
            human!(format, "  Session:     Continuing {}", last);
            last
        } else {
            let fresh = new_cli_session_id(&control_plane);
            human!(
                format,
                "  Session:     No previous session found, created {}",
                fresh
            );
            fresh
        }
    } else {
        let fresh = new_cli_session_id(&control_plane);
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
        let execution = run_single_turn(
            &control_plane,
            &args,
            &options,
            &session_id,
            &prompt,
            bound_workspace_id.as_deref(),
        );
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
                    session_id = new_cli_session_id(&control_plane);
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

        let execution = run_single_turn(
            &control_plane,
            &args,
            &options,
            &session_id,
            trimmed,
            bound_workspace_id.as_deref(),
        );
        flush_events(&execution.events);
        let _ = persist_turn_result(&execution, &options);
        // 交互式多轮：单轮失败不终止 REPL（退出码沿用基线语义 = 0）。
    }

    finish(ExitCode::Success, observability);
}

#[cfg(test)]
mod workspace_binding_tests {
    use super::*;

    /// r1-dim6 F1 回归：`-w` 必须被登记成 workspace，且返回的 id 解析出的 root
    /// 就是 `-w` 本身 —— 而不是注册表 `default`（缺陷形态：会话不盖章 ⇒ 回落 default，
    /// 重放 9/9 会话的 env cwd 落在 `/home/dm/pony_agent`）。
    #[test]
    fn bind_workspace_id_maps_workspace_root_to_its_own_record() {
        let _guard = pony_agent_core::agent::runtime_helper::TestRuntimeGuard::new();
        let raw = std::env::temp_dir().join(format!("ponycli-ws-bind-{}", std::process::id()));
        std::fs::create_dir_all(&raw).expect("temp workspace");
        let root = std::fs::canonicalize(&raw).unwrap_or(raw);

        let runtime = AgentRuntimeBuilder::new()
            .session_store(SessionStore::with_backend(Box::new(
                FileSessionBackend::new(root.join("cli-sessions.json")),
            )))
            .workspace_root(root.clone())
            .build();
        let control_plane = HostControlPlaneBuilder::new().runtime(runtime).build();

        let bound = bind_workspace_id(&control_plane, &root)
            .expect("`-w` 必须能登记为 workspace（失败即 F1 未修复）");

        let resolved = control_plane
            .list_workspaces()
            .into_iter()
            .find(|record| record.id == bound)
            .map(|record| record.root_path)
            .expect("绑定 id 必须命中注册表记录");
        assert_eq!(
            std::fs::canonicalize(&resolved).unwrap_or_else(|_| PathBuf::from(&resolved)),
            root,
            "绑定 id 解析出的 root 必须是 `-w` 本身，不得是注册表 default"
        );

        // 幂等：同一 root 再绑定一次必须得到同一个 id（不新增记录）。
        let again = bind_workspace_id(&control_plane, &root).expect("重复绑定必须成功");
        assert_eq!(again, bound, "同一 root 的绑定必须幂等");

        std::fs::remove_dir_all(&root).ok();
    }
}

/// 维护子命令的 `--help` 护栏（B046 事故现场回归）。
#[cfg(test)]
mod maintenance_help_guard_tests {
    use super::*;

    fn argv(raw: &[&str]) -> Vec<String> {
        raw.iter().map(|arg| arg.to_string()).collect()
    }

    #[test]
    fn help_flags_never_enter_maintenance_subcommands() {
        // 任一形态的 `--help` / `-h` 都必须让维护分支让路（否则会真的执行 VACUUM / rebuild）。
        for raw in [
            vec!["ponycli", "sessions", "vacuum", "--help"],
            vec!["ponycli", "sessions", "vacuum", "-h"],
            vec!["ponycli", "--help", "sessions", "vacuum"],
            vec!["ponycli", "sessions", "rebuild-index", "--db", "x.db", "-h"],
        ] {
            assert!(
                is_help_request(&argv(&raw)),
                "{raw:?} 必须被判定为 help 请求（不得进入维护分支）"
            );
        }
    }

    #[test]
    fn normal_invocations_are_not_treated_as_help() {
        for raw in [
            vec!["ponycli", "sessions", "vacuum", "--dry-run"],
            vec!["ponycli", "sessions", "vacuum", "--db", "x.db"],
            vec!["ponycli", "sessions", "rebuild-index"],
        ] {
            assert!(
                !is_help_request(&argv(&raw)),
                "{raw:?} 不是 help 请求，必须正常进入维护分支"
            );
        }
    }
}

// ───────────────────── B151/P2：ACP 宿主形态（`ponycli acp`） ─────────────────────

/// `ponycli acp` 的冻结参数面（Lead 2026-11-14 冻结，不得夹带其它 flag）。
struct AcpCliArgs {
    agent: Option<String>,
    list: bool,
    prompt: Option<String>,
    output_format: OutputFormat,
    workspace: Option<PathBuf>,
    timeout: Option<Duration>,
    resume: Option<String>,
}

/// `acp` 子命令拦截：`main` 里只留一行（与 sessions/config 同一做法）。
fn run_acp_subcommand_if_requested() -> Option<i32> {
    let argv: Vec<String> = std::env::args().collect();
    if argv.get(1).map(String::as_str) != Some("acp") {
        return None;
    }
    Some(run_acp_client_mode(&argv[2..]))
}

/// `ponycli acp [--agent <id>] [--list] [-p|--prompt <text>] [--output-format jsonl|sse]
///              [--workspace <dir>] [--timeout-secs <n>] [--resume <session-id>]`
///
/// 失败一律「非 0 + stderr 带真源错误码」，绝不 panic、绝不在 stdin 上等输入（无 REPL）。
fn run_acp_client_mode(args: &[String]) -> i32 {
    let mut parsed = AcpCliArgs {
        agent: None,
        list: false,
        prompt: None,
        output_format: OutputFormat::Text,
        workspace: None,
        timeout: None,
        resume: None,
    };
    let mut idx = 0;
    while idx < args.len() {
        match args[idx].as_str() {
            "-h" | "--help" => {
                print_acp_usage();
                return ExitCode::Success.code();
            }
            "--list" => parsed.list = true,
            "--agent" => match next_value(args, &mut idx) {
                Ok(value) => parsed.agent = Some(value),
                Err(message) => return acp_usage_error(&message),
            },
            "-p" | "--prompt" => match next_value(args, &mut idx) {
                Ok(value) => parsed.prompt = Some(value),
                Err(message) => return acp_usage_error(&message),
            },
            "--output-format" => {
                let value = match next_value(args, &mut idx) {
                    Ok(value) => value,
                    Err(message) => return acp_usage_error(&message),
                };
                parsed.output_format = match OutputFormat::from_str(&value) {
                    Ok(format) => format,
                    Err(error) => {
                        return acp_usage_error(&format!(
                            "{error}（acp 模式只接受 text|jsonl|sse，不得静默退化）"
                        ))
                    }
                };
            }
            "--workspace" => match next_value(args, &mut idx) {
                Ok(value) => parsed.workspace = Some(PathBuf::from(value)),
                Err(message) => return acp_usage_error(&message),
            },
            "--timeout-secs" => {
                let value = match next_value(args, &mut idx) {
                    Ok(value) => value,
                    Err(message) => return acp_usage_error(&message),
                };
                match value.parse::<u64>() {
                    Ok(secs) if secs <= MAX_TIMEOUT_SECS => {
                        parsed.timeout = Some(Duration::from_secs(secs))
                    }
                    Ok(secs) => {
                        return acp_usage_error(&format!(
                            "--timeout-secs {secs} 超出上限 {MAX_TIMEOUT_SECS}（无上限的预算不可兑现）"
                        ))
                    }
                    Err(_) => {
                        return acp_usage_error(&format!(
                            "--timeout-secs 需要 0..={MAX_TIMEOUT_SECS} 的整数，实际 `{value}`"
                        ))
                    }
                }
            }
            "--resume" => match next_value(args, &mut idx) {
                Ok(value) => parsed.resume = Some(value),
                Err(message) => return acp_usage_error(&message),
            },
            other => return acp_usage_error(&format!("acp 子命令不接受参数 `{other}`")),
        }
        idx += 1;
    }

    // 1) `--list`：内置 + 用户清单逐行 JSON（缺文件回落内置两条默认）。
    if parsed.list {
        return acp_list_agents();
    }

    // 2) 非 `--list` 必须显式 --agent（不静默进入默认流程）。
    let Some(agent_id) = parsed.agent.clone() else {
        return acp_usage_error("acp 模式必须显式给出 --agent <id>（--list 除外）");
    };
    let spec = match pony_agent_core::acp::config::resolve_agent(&agent_id) {
        Ok(spec) => spec,
        Err(error) => {
            // 保留清单层的真源错误码（`acp_config_agent_not_found` 等），不塌缩成通用错误。
            eprintln!("[ponycli] {}: {error}", error.code());
            return ExitCode::Config.code();
        }
    };
    let workspace = match acp_workspace(parsed.workspace.as_deref()) {
        Ok(workspace) => workspace,
        Err(message) => {
            eprintln!("[ponycli] acp_unavailable: {message}");
            return ExitCode::Usage.code();
        }
    };

    // 3) `--resume`：按 agentCapabilities.loadSession 判定；不支持即显式降级（不静默新会话）。
    if let Some(session_id) = parsed.resume.clone() {
        return acp_resume(&spec, &workspace, &session_id);
    }

    // 4) 单轮：无 `-p/--prompt` ⇒ 用法错误（本模式无 REPL，绝不在 stdin 上等输入）。
    let Some(prompt) = parsed.prompt.clone() else {
        return acp_usage_error("acp 模式需要 -p/--prompt <text>（本模式无 REPL）");
    };

    let mut session = match pony_agent_core::acp::session::AcpAgentSession::spawn(&spec, &workspace)
    {
        Ok(session) => session,
        Err(error) => return acp_session_error(error),
    };
    let blocks = vec![pony_agent_core::acp::protocol::ContentBlock::Text {
        text: prompt,
    }];
    let code = match parsed.output_format {
        OutputFormat::Jsonl => {
            let sink = headless::JsonlSink::new();
            let code = acp_run_prompt(&session, blocks, &sink, parsed.timeout);
            // jsonl 契约：stdout 每行一个事件对象（sink 已保证 JSONL 形状）。
            for line in sink.finish() {
                println!("{line}");
            }
            code
        }
        OutputFormat::Sse => {
            let sink = headless::SseSink::new();
            let code = acp_run_prompt(&session, blocks, &sink, parsed.timeout);
            for frame in sink.finish() {
                println!("{frame}");
            }
            code
        }
        OutputFormat::Text => {
            let sink = AcpTextSink;
            acp_run_prompt(&session, blocks, &sink, parsed.timeout)
        }
    };
    session.close();
    code
}

/// 取下一个参数值（缺失即用法错误，绝不把下一个 flag 当值）。
fn next_value(args: &[String], idx: &mut usize) -> Result<String, String> {
    let value = args
        .get(*idx + 1)
        .ok_or_else(|| format!("参数 `{}` 缺少取值", args[*idx]))?;
    *idx += 1;
    Ok(value.clone())
}

fn acp_usage_error(message: &str) -> i32 {
    eprintln!("\x1b[31mArgument Error:\x1b[0m {message}");
    eprintln!("用法：ponycli acp [--agent <id>] [--list] [-p|--prompt <text>] [--output-format jsonl|sse] [--workspace <dir>] [--timeout-secs <n>] [--resume <session-id>]");
    ExitCode::Usage.code()
}

/// `--list`：逐行 JSON `{id,label,command,args,builtin}`（缺文件回落到内置两条默认）。
fn acp_list_agents() -> i32 {
    let agents = match pony_agent_core::acp::config::load_agents() {
        Ok(agents) => agents,
        Err(error) => {
            eprintln!("[ponycli] {}: {error}", error.code());
            return ExitCode::Config.code();
        }
    };
    for agent in agents {
        let builtin = acp_is_builtin(&agent);
        let line = serde_json::json!({
            "id": agent.id,
            "label": agent.label,
            "command": agent.command,
            "args": agent.args,
            "builtin": builtin,
        });
        match serde_json::to_string(&line) {
            Ok(text) => println!("{text}"),
            Err(error) => {
                eprintln!("[ponycli] acp_unavailable: --list 序列化失败：{error}");
                return ExitCode::Internal.code();
            }
        }
    }
    ExitCode::Success.code()
}

/// 是否为内置默认项（id + 启动命令 + 参数三者都与 `BUILTIN_AGENTS` 一致才算）。
fn acp_is_builtin(spec: &pony_agent_core::acp::config::AcpAgentSpec) -> bool {
    pony_agent_core::acp::config::BUILTIN_AGENTS
        .iter()
        .any(|builtin| {
            builtin.id == spec.id
                && builtin.command == spec.command
                && builtin.args.len() == spec.args.len()
                && builtin
                    .args
                    .iter()
                    .zip(spec.args.iter())
                    .all(|(left, right)| *left == right.as_str())
        })
}

/// 工作目录：缺省当前目录；必须绝对路径 + 存在 + 是目录（canonicalize 后交给 ACP）。
fn acp_workspace(explicit: Option<&Path>) -> Result<PathBuf, String> {
    let candidate = match explicit {
        Some(path) => path.to_path_buf(),
        None => std::env::current_dir()
            .map_err(|error| format!("无法获取当前工作目录（{error}），请显式传 --workspace"))?,
    };
    if !candidate.is_absolute() {
        return Err(format!(
            "--workspace 必须是绝对路径，实际 `{}`。",
            candidate.display()
        ));
    }
    let canonical = std::fs::canonicalize(&candidate)
        .map_err(|error| format!("--workspace `{}` 不可用（{error}）。", candidate.display()))?;
    if !canonical.is_dir() {
        return Err(format!("--workspace `{}` 不是目录。", canonical.display()));
    }
    Ok(canonical)
}

/// `--resume`：按 `agentCapabilities.loadSession` 判定；不支持/未实现一律显式降级。
fn acp_resume(
    spec: &pony_agent_core::acp::config::AcpAgentSpec,
    workspace: &Path,
    session_id: &str,
) -> i32 {
    let mut client = match pony_agent_core::acp::client::AcpAgentClient::connect(spec, workspace) {
        Ok(client) => client,
        Err(error) => return acp_session_error(error),
    };
    let loadable = client.agent_capabilities().load_session;
    client.close();
    if !loadable {
        eprintln!(
            "[ponycli] acp_unavailable: agent `{}` 声明 loadSession=false，无法恢复会话 `{session_id}`（不静默降级为新会话）。",
            spec.id
        );
        return ExitCode::Config.code();
    }
    // fail-closed：本波次未实现 session/load 恢复时，必须显式降级而不是悄悄开新会话。
    eprintln!(
        "[ponycli] acp_unavailable: agent `{}` 支持 loadSession，但 session/load 恢复尚未实现（显式降级，不静默新会话）。",
        spec.id
    );
    ExitCode::Config.code()
}

/// 跑一轮 prompt；`--timeout-secs` 给出时是整轮预算（超预算 → session/cancel + Timeout 码）。
fn acp_run_prompt<S: TurnEventSink + Sync>(
    session: &pony_agent_core::acp::session::AcpAgentSession,
    blocks: Vec<pony_agent_core::acp::protocol::ContentBlock>,
    sink: &S,
    timeout: Option<Duration>,
) -> i32 {
    let Some(limit) = timeout else {
        return acp_result_exit_code(&session.prompt(blocks, sink));
    };
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::scope(|scope| {
        scope.spawn(move || {
            let _ = sender.send(session.prompt(blocks, sink));
        });
        match receiver.recv_timeout(limit) {
            Ok(result) => acp_result_exit_code(&result),
            Err(_) => {
                if let Err(error) = session.cancel() {
                    eprintln!("[ponycli] acp 超时后 session/cancel 失败：{error}");
                }
                // 有界等 prompt 线程收尾（agent 以 cancelled 应答）；无论如何按 Timeout 退出。
                let _ = receiver.recv_timeout(pony_agent_core::acp::transport::ACP_REQUEST_TIMEOUT);
                eprintln!(
                    "[ponycli] acp_timeout: 委派超过 {}s 未返回，已发出 session/cancel。",
                    limit.as_secs()
                );
                ExitCode::Timeout.code()
            }
        }
    })
}

/// 归约 → 进程退出码（`ok` = 0；`aborted` = 6；其余 = 5 ToolFailure）。
fn acp_result_exit_code(result: &pony_agent_core::agent::tools::ToolResult) -> i32 {
    match result.status.as_str() {
        "ok" => ExitCode::Success.code(),
        "aborted" => {
            eprintln!("[ponycli] {}: {}", result.status, result.output);
            ExitCode::PermissionDenied.code()
        }
        _ => {
            eprintln!("[ponycli] {}: {}", result.status, result.output);
            // 退出码契约：`acp_timeout`（传输层/整轮预算超时）属于 timeout 档 = 7，
            // 不得塌进通用 tool-failure(5)——否则 usage 文档「expiry = exit 7」与实现不符。
            if acp_result_error_code(result).as_deref() == Some("acp_timeout") {
                ExitCode::Timeout.code()
            } else {
                ExitCode::ToolFailure.code()
            }
        }
    }
}

/// 从归约后的 `ToolResult.output`（JSON 封套）取 `error.code`；解析失败回退文本匹配。
///
/// 只读产物、不构造第二套错误码：`error.code` 由 `acp::session` 按 §3.2.9 分源写入。
fn acp_result_error_code(result: &pony_agent_core::agent::tools::ToolResult) -> Option<String> {
    match serde_json::from_str::<serde_json::Value>(&result.output) {
        Ok(value) => value
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(|code| code.as_str())
            .map(str::to_string),
        Err(_) => {
            if result.output.contains("\"acp_timeout\"") {
                Some("acp_timeout".to_string())
            } else {
                None
            }
        }
    }
}

/// 传输/会话层错误 → 退出码（保留 `acp_*` 错误码来源，不塌缩）。
fn acp_session_error(error: pony_agent_core::acp::transport::AcpTransportError) -> i32 {
    use pony_agent_core::acp::transport::AcpTransportError;
    eprintln!("[ponycli] {}: {error}", error.code());
    match error {
        AcpTransportError::Spawn(_) | AcpTransportError::Initialize(_) => ExitCode::Config.code(),
        AcpTransportError::AuthRequired(_) => ExitCode::PermissionDenied.code(),
        AcpTransportError::Timeout(_) => ExitCode::Timeout.code(),
        AcpTransportError::Protocol(_) => ExitCode::Internal.code(),
        AcpTransportError::Agent(_) | AcpTransportError::Refused(_) | AcpTransportError::Unavailable(_) => {
            ExitCode::ToolFailure.code()
        }
    }
}

/// text 模式的 ACP sink：正文走 stdout，思考/工具活动走 stderr（stdout 只留人类可读正文）。
struct AcpTextSink;

impl TurnEventSink for AcpTextSink {
    fn emit(&self, name: &str, payload: TurnStreamEvent) {
        match name {
            "turn:delta" => {
                if let Some(text) = payload.text.as_deref() {
                    print!("{text}");
                    let _ = io::stdout().flush();
                }
                if let Some(reasoning) = payload.reasoning_content.as_deref() {
                    eprintln!("\x1b[2m[thinking] {reasoning}\x1b[0m");
                }
            }
            "turn:tool" => {
                if let Some(activities) = payload.tool_activities.as_ref() {
                    for activity in activities {
                        eprintln!(
                            "\x1b[36m[tool]\x1b[0m {} :: {} ({})",
                            activity.id, activity.description, activity.status
                        );
                    }
                }
            }
            _ => {}
        }
    }
}

/// `ponycli acp --help` 的用法段（冻结拼写：`--agent` / `--list` / `--resume` 等）。
fn print_acp_usage() {
    println!("\x1b[1;32m=== Pony Agent CLI — ACP agent host mode ===\x1b[0m");
    println!("Usage: ponycli acp [OPTIONS]");
    println!();
    println!("Options:");
    println!("  --agent <id>               ACP agent id from the acp-agents.json list (opencode | antigravity | ...)");
    println!("                             required unless --list is given (no REPL in this mode)");
    println!("  --list                     Print every configured ACP agent as one JSON object per line");
    println!("                             {{id,label,command,args,builtin}}; exit 0");
    println!("  -p, --prompt <text>        Run a single delegated turn with the given prompt");
    println!("  --output-format <fmt>      text | jsonl | sse (default: text; jsonl/sse: stdout = events only)");
    println!("  --workspace <dir>          Absolute workspace root for session/new (default: current dir)");
    println!("  --timeout-secs <n>         Bound the delegated turn (0..=86400); expiry exits with code 7");
    println!("  --resume <session-id>      Resume an existing ACP session; unsupported agents fail");
    println!("                             closed with acp_unavailable (never silently starts a new session)");
    println!("  -h, --help                 Show this help");
    println!();
    println!("Config discovery: $XDG_CONFIG_HOME/pony-agent/acp-agents.json (missing ⇒ builtin defaults).");
}
