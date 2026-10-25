//! pony-cli: 独立运行的命令行 Agent 交互程序
//! 支持单轮与多轮交互、直接连接 polyllm/ponyllm 网关，并默认驱动 gemini-3.8-flash 模型与真实 Tool 调度。

use pony_agent_core::agent::config::{
    ProviderModelCapabilities, ProviderSelectionResolver, ResolvedProviderSelection,
    ThinkingParamPattern,
};
use pony_agent_core::agent::control_plane::{
    HostControlPlane, HostControlPlaneBuilder, StartTurnStreamCommand,
};
use pony_agent_core::agent::graph::GraphRunStore;
use pony_agent_core::agent::provider::{ProviderAuthType, ProviderProtocol};
use pony_agent_core::agent::runtime::{AgentRuntimeBuilder, TurnInput, TurnStreamEvent};
use pony_agent_core::agent::session::{FileSessionBackend, SessionStore};
use pony_agent_core::agent::turn_flow::TurnEventSink;
use std::io::{self, BufRead, Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubCommand {
    Run,
    Init,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlashCommand {
    Clear,
    Compact,
    Cost,
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
            "/cost" => Some(SlashCommand::Cost),
            "/help" => Some(SlashCommand::Help),
            "/exit" | "/quit" => Some(SlashCommand::Exit),
            other => Some(SlashCommand::Unknown(other.to_string())),
        }
    }
}

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

        Self {
            subcommand: SubCommand::Run,
            prompt: None,
            model: "gemini-3.8-flash".to_string(),
            gateway_url: env_gateway,
            api_key: env_key,
            workspace: canonical_workspace,
            continue_last: false,
            resume_session_id: None,
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
            "turn:output_end" => {
                self.is_streaming.store(false, Ordering::SeqCst);
            }
            _ => {}
        }
    }
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

fn handle_init(workspace: &std::path::Path) -> Result<(), String> {
    let target = workspace.join("AGENTS.md");
    if target.exists() {
        println!("\x1b[33mAGENTS.md already exists at {}\x1b[0m", target.display());
        return Ok(());
    }
    std::fs::write(&target, DEFAULT_AGENTS_MD)
        .map_err(|e| format!("failed to write AGENTS.md: {e}"))?;
    println!("\x1b[32m✔ Successfully created AGENTS.md at {}\x1b[0m", target.display());
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

fn run_turn_interactive(
    control_plane: &HostControlPlane,
    session_id: &str,
    input_text: &str,
) {
    let is_streaming = Arc::new(AtomicBool::new(true));
    let sink = TerminalSink::new(is_streaming.clone());
    let turn_id = format!("turn-{}", uuid::Uuid::new_v4());

    println!("\x1b[1;34m[Pony Agent] Running turn...\x1b[0m");

    let cmd = StartTurnStreamCommand {
        turn_id,
        input: TurnInput {
            message: input_text.to_string(),
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
        },
    };

    control_plane.start_turn_stream(&sink, cmd);

    // 等待流结束
    while is_streaming.load(Ordering::SeqCst) {
        std::thread::sleep(std::time::Duration::from_millis(50));
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

fn print_help() {
    println!("\x1b[1;33mCommands & Slash-commands:\x1b[0m");
    println!("  /clear    Clear the current context and start a fresh session");
    println!("  /compact  Compact the current conversation history");
    println!("  /cost     Show total tokens used and cost estimates");
    println!("  /help     Show this help message");
    println!("  /exit     Exit the interactive session");
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
    fn install() -> Self {
        pony_agent_core::agent::ponysentry::install_panic_hook();
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

fn main() {
    let _guard = pony_agent_core::agent::runtime_helper::TestRuntimeGuard::new();
    // Wave-5 P0：可观测性引导（panic hook + OTel provider；失败只降级 warn，不阻塞启动）。
    let _observability = ObservabilityBootstrap::install();
    let raw_args: Vec<String> = std::env::args().collect();
    let args = match parse_cli_args(&raw_args) {
        Ok(a) => a,
        Err(err) => {
            eprintln!("\x1b[31mArgument Error:\x1b[0m {err}");
            std::process::exit(1);
        }
    };

    if args.subcommand == SubCommand::Init {
        if let Err(e) = handle_init(&args.workspace) {
            eprintln!("\x1b[31mInit Error:\x1b[0m {e}");
            std::process::exit(1);
        }
        return;
    }

    println!("\x1b[1;32m=== Pony Agent CLI (Claude-Code Style) ===\x1b[0m");
    println!("  Model:       {}", args.model);
    println!("  Gateway:     {}", args.gateway_url);
    println!("  Workspace:   {}", args.workspace.display());

    let (control_plane, state_dir) = build_control_plane(&args);
    let tracker = SessionTracker::new(&state_dir);

    let mut session_id = if let Some(res) = &args.resume_session_id {
        println!("  Session:     Resuming {}", res);
        res.clone()
    } else if args.continue_last {
        if let Some(last) = tracker.get_last() {
            println!("  Session:     Continuing {}", last);
            last
        } else {
            let fresh = format!("cli-session-{}", uuid::Uuid::new_v4());
            println!("  Session:     No previous session found, created {}", fresh);
            fresh
        }
    } else {
        let fresh = format!("cli-session-{}", uuid::Uuid::new_v4());
        fresh
    };

    tracker.record(&session_id);

    // 检查 stdin 管道是否有数据
    let piped_input = read_stdin_if_piped();
    let final_prompt = match (args.prompt, piped_input) {
        (Some(p), Some(pipe)) => Some(format!("Context piped from stdin:\n```\n{}\n```\n\nPrompt:\n{}", pipe.trim(), p)),
        (None, Some(pipe)) => Some(format!("Context piped from stdin:\n```\n{}\n```", pipe.trim())),
        (Some(p), None) => Some(p),
        (None, None) => None,
    };

    if let Some(prompt) = final_prompt {
        run_turn_interactive(&control_plane, &session_id, &prompt);
        return;
    }

    println!("  Session ID:  {}", session_id);
    println!("\nType your message below (or '/help' for options, '/exit' to quit):");
    let stdin = io::stdin();
    let mut reader = stdin.lock();

    loop {
        print!("\n\x1b[1;35mUser > \x1b[0m");
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
                    println!("Goodbye!");
                    break;
                }
                SlashCommand::Help => {
                    print_help();
                    continue;
                }
                SlashCommand::Clear => {
                    session_id = format!("cli-session-{}", uuid::Uuid::new_v4());
                    tracker.record(&session_id);
                    println!("\x1b[32m✔ Context cleared. Switched to new session: {}\x1b[0m", session_id);
                    continue;
                }
                SlashCommand::Compact => {
                    println!("\x1b[33m⚡ Compact triggered: summarizing prior turns...\x1b[0m");
                    run_turn_interactive(&control_plane, &session_id, "系统内部指令：请提炼总结前面的对话关键背景以压缩上下文。");
                    continue;
                }
                SlashCommand::Cost => {
                    println!("\x1b[36m📊 Session Cost & Usage:\x1b[0m");
                    println!("  Session ID:   {}", session_id);
                    println!("  Model:        {}", args.model);
                    println!("  Tariff:       PolyLLM Standard ($3.00/1M in, $15.00/1M out, $0.75/1M cache)");
                    println!("  Budget Limit: Configured via PONY_MAX_BUDGET_USD / CostTracker");
                    continue;
                }
                SlashCommand::Unknown(u) => {
                    println!("\x1b[31mUnknown command: {}\x1b[0m (type /help for available commands)", u);
                    continue;
                }
            }
        }

        run_turn_interactive(&control_plane, &session_id, trimmed);
    }
}
