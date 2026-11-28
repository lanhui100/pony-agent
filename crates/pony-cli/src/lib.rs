//! CLI 参数与配置子命令（`ponycli models` 等）的纯解析与执行核心。
//!
//! 归属库模块（`pony_agent_core::cli`）而非 `src/bin/cli.rs`：
//! 使验收测试（`tests/cli_acceptance.rs`）与 `bin/cli.rs` 通过公共库契约耦合，
//! 不再用 `#[path = "../src/bin/cli.rs"]` 触发 bin-as-module 死代码掩蔽。

use pony_agent_core::agent::config::ProviderRegistryStore;
use pony_agent_core::agent::headless::{HeadlessOptions, OutputFormat, PermissionMode};
use pony_agent_core::agent::model_window::{
    list_context_windows, parse_context_window_input, set_context_window, ModelContextWindow,
};
use std::path::PathBuf;
use std::str::FromStr;

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
            "/clear" => Some(Self::Clear),
            "/compact" => Some(Self::Compact),
            "/help" => Some(Self::Help),
            "/exit" | "/quit" => Some(Self::Exit),
            other => Some(Self::Unknown(other.to_string())),
        }
    }
}

/// `--timeout-secs` 上限（24h）。超出即用法错误（exit 2）：无上限的预算不可兑现。
pub const MAX_TIMEOUT_SECS: u64 = 86_400;

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
        // 网关 key 解析链（env > env 别名 > 遗留 ponyllm.toml）单源在 pony-provider：
        // 与 TUI 共用同一条链，避免两个 bin 同机同配置拿到不同的 key。
        let env_key = pony_agent_core::agent::config::gateway_api_key_from_env_or_legacy_config();

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
                config.model.clone_from(&args[idx]);
            }
            "-g" | "--gateway" => {
                idx += 1;
                if idx >= args.len() {
                    return Err("--gateway requires a value".to_string());
                }
                config.gateway_url.clone_from(&args[idx]);
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
                let raw = &args[idx];
                let secs = raw.parse::<u64>().map_err(|_| {
                    format!(
                        "invalid value `{raw}` for --timeout-secs: expected a whole number of seconds"
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

/// 取 `--flag <value>` 的值（缺失或后面又是另一个 flag ⇒ None）。
pub fn flag_value(args: &[String], flag: &str) -> Option<String> {
    let index = args.iter().position(|arg| arg == flag)?;
    let value = args.get(index + 1)?;
    if value.starts_with("--") {
        return None;
    }
    Some(value.clone())
}

/// `ponycli models` 的解析结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelsCommand {
    List {
        provider: Option<String>,
    },
    SetWindow {
        provider: String,
        model: String,
        /// `None` = 清除声明（恢复跟随能力预设）。
        window: Option<u32>,
    },
}

/// 解析 `ponycli models ...`：`Ok(None)` = 这不是 models 子命令。
///
/// 拆成纯函数是为了可测（入口只负责取 argv → 执行 → 打印 → 退出码），
/// 参数错误的文案与「缺哪个参数」也能被用例钉住。
pub fn parse_models_command(args: &[String]) -> Result<Option<ModelsCommand>, String> {
    if args.get(1).map(String::as_str) != Some("models") {
        return Ok(None);
    }
    match args.get(2).map(String::as_str) {
        Some("list") => Ok(Some(ModelsCommand::List {
            provider: flag_value(args, "--provider"),
        })),
        Some("set-window") => {
            let provider = flag_value(args, "--provider").ok_or("缺少 --provider")?;
            let model = flag_value(args, "--model").ok_or("缺少 --model")?;
            let raw = flag_value(args, "--window").ok_or("缺少 --window")?;
            let window = parse_context_window_input(&raw)?;
            Ok(Some(ModelsCommand::SetWindow {
                provider,
                model,
                window,
            }))
        }
        Some(other) => Err(format!("未知的 models 子命令：{other}")),
        None => Err("models 需要子命令（list | set-window）".to_string()),
    }
}

/// 单条模型的窗口报告：`declaredContextWindowTokens`（磁盘事实）与 `contextWindowTokens`
/// （生效值）必须并列，否则「这个数是不是我填的」在 CLI 上无从判断。不含任何密钥。
fn model_window_report(entry: &ModelContextWindow) -> serde_json::Value {
    serde_json::json!({
        "providerId": entry.provider_id,
        "providerName": entry.provider_name,
        "modelId": entry.model_id,
        "modelName": entry.model_name,
        "model": entry.model,
        "capabilityPreset": entry.preset,
        "declaredContextWindowTokens": entry.declared,
        "contextWindowTokens": entry.effective,
        "source": if entry.declared.is_some() { "declared" } else { "preset" },
    })
}

/// 执行解析后的命令，返回要打印的报告行（调用方负责输出）。
pub fn execute_models_command(
    store: &ProviderRegistryStore,
    command: &ModelsCommand,
) -> Result<Vec<serde_json::Value>, String> {
    match command {
        ModelsCommand::List { provider } => Ok(list_context_windows(store)
            .iter()
            .filter(|entry| {
                provider
                    .as_deref()
                    .is_none_or(|wanted| entry.provider_id == wanted)
            })
            .map(model_window_report)
            .collect()),
        ModelsCommand::SetWindow {
            provider,
            model,
            window,
        } => {
            let entry = set_context_window(store, provider, model, *window)?;
            Ok(vec![model_window_report(&entry)])
        }
    }
}
