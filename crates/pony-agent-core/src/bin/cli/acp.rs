//! `ponycli acp` 宿主形态（B151/B161/B176/B178/B183/B184/B185）—— 从 `bin/cli.rs` **纯搬迁**。
//!
//! 目的：把 ACP-only 逻辑收进子模块，压回 `cli.rs` 的文件/函数规模门。行为与搬迁前逐字等价。
//!
//! 根模块（`bin/cli.rs`）保留：`mod acp;`、`use acp::run_acp_subcommand_if_requested;`（`main`
//! 的既有拦截行不改），以及共享的 stdout 收口 helper（`emit_stdout_line`：`models list` 也在用）。

use super::*;

// ───────────────────── B151/P2：ACP 宿主形态（`ponycli acp`） ─────────────────────

/// `ponycli acp` 的冻结参数面（Lead 2026-11-14 冻结，不得夹带其它 flag）。
struct AcpCliArgs {
    agent: Option<String>,
    list: bool,
    /// B185：`--list-sessions`（列**该 agent 的会话**；与 `--list` 的 agent 清单语义严格区分）。
    list_sessions: bool,
    /// B185：`--list-sessions` 的分页游标（原样透传给 `session/list`）。
    session_cursor: Option<String>,
    /// B185：`--list-sessions` 的页大小（原样透传；0 为用法错误）。
    session_limit: Option<u32>,
    prompt: Option<String>,
    output_format: OutputFormat,
    workspace: Option<PathBuf>,
    timeout: Option<Duration>,
    resume: Option<String>,
}

/// `acp` 子命令拦截：`main` 里只留一行（与 sessions/config 同一做法）。
pub(crate) fn run_acp_subcommand_if_requested() -> Option<i32> {
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
    let parsed = match parse_acp_cli_args(args) {
        Ok(parsed) => parsed,
        Err(code) => return code,
    };

    // L3-P BLOCKING 修复（wave-59 契约 C1）：`ponycli acp` 由 `main` 在本文件**之前**拦截，
    // 从未经过 `ObservabilityBootstrap` 的日志改道 ⇒ `jsonl/sse` 下 `agent::otel::log` 的结构化
    // 日志会混入 stdout，破坏「stdout 只承载事件」。这里在解析出输出格式后补上同一开关
    // （幂等；text 模式逐字不变，仍走既有 stdout 诊断面）。
    if parsed.output_format != OutputFormat::Text {
        pony_agent_core::agent::otel::log::set_stderr_output(true);
    }

    // 0) B185：`--list`（agent 清单）与 `--list-sessions`（会话清单）语义不同，不得混用。
    if parsed.list && parsed.list_sessions {
        return acp_usage_error(
            "--list 与 --list-sessions 互斥（前者列 agent 清单，后者列某 agent 的会话）",
        );
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

    // 2.5) B185 `--list-sessions`：只列会话，与单轮 `-p` 互斥（择一：**拒绝并给用法错误**）。
    if parsed.list_sessions {
        if parsed.prompt.is_some() {
            return acp_usage_error(
                "--list-sessions 与 -p/--prompt 互斥（择一：列会话 或 跑一轮；不得混用）",
            );
        }
        if parsed.resume.is_some() {
            eprintln!("[ponycli] 提示：--list-sessions 只列会话，忽略 --resume。");
        }
        return acp_list_sessions(
            &spec,
            &workspace,
            parsed.session_cursor.as_deref(),
            parsed.session_limit,
        );
    }

    // 3) 单轮：无 `-p/--prompt` ⇒ 用法错误（本模式无 REPL，绝不在 stdin 上等输入）。
    let Some(prompt) = parsed.prompt.clone() else {
        return acp_usage_error("acp 模式需要 -p/--prompt <text>（本模式无 REPL）");
    };

    // 4) `--resume` ⇒ `session/load` 接续既有会话（**跳过 `session/new`**，绝不静默新建）；
    //    不支持（loadSession=false 或 -32601）⇒ `Unsupported` ⇒ stderr `acp_unavailable` + 非 0。
    let mut session = match parsed.resume.clone() {
        Some(session_id) => match pony_agent_core::acp::session::AcpAgentSession::spawn_for_cli_resume(
            &spec,
            &workspace,
            &session_id,
            &[],
        ) {
            Ok(session) => session,
            Err(error) => return acp_resume_error(&spec, &session_id, error),
        },
        None => match pony_agent_core::acp::session::AcpAgentSession::spawn_for_cli(&spec, &workspace)
        {
            Ok(session) => session,
            Err(error) => return acp_session_error(error),
        },
    };
    // 整轮预算：`--timeout-secs` 是用户显式预算；缺省用 `acp` 层**单一真源**
    // `ACP_DEFAULT_TURN_BUDGET`（与 `acp_delegate` 的 ToolExecutionPolicy 默认同源），
    // **不得**回落到 transport 的 5s 单请求硬顶——否则 slow agent（>5s）会被误判成
    // acp_timeout/exit 7。
    let budget = parsed
        .timeout
        .unwrap_or(pony_agent_core::acp::ACP_DEFAULT_TURN_BUDGET);
    session.set_turn_budget(budget);
    eprintln!(
        "[ponycli] acp: agent={} session={} turn_budget={:?}",
        spec.id,
        session.session_id(),
        budget
    );
    let code = run_acp_single_turn(&session, prompt, parsed.output_format);
    session.close();
    code
}


/// `ponycli acp` 的参数解析（逐字等价于搬迁前）：`Err(退出码)` = 已打印用法/帮助。
fn parse_acp_cli_args(args: &[String]) -> Result<AcpCliArgs, i32> {
    let mut parsed = AcpCliArgs {
        agent: None,
        list: false,
        list_sessions: false,
        session_cursor: None,
        session_limit: None,
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
                return Err(ExitCode::Success.code());
            }
            "--list" => parsed.list = true,
            "--list-sessions" => parsed.list_sessions = true,
            "--session-cursor" => match next_value(args, &mut idx) {
                Ok(value) => parsed.session_cursor = Some(value),
                Err(message) => return Err(acp_usage_error(&message)),
            },
            "--session-limit" => {
                let value = match next_value(args, &mut idx) {
                    Ok(value) => value,
                    Err(message) => return Err(acp_usage_error(&message)),
                };
                match value.parse::<u32>() {
                    Ok(0) => {
                        return Err(acp_usage_error(
                            "--session-limit 必须是 ≥1 的整数（0 不是合法页大小）",
                        ))
                    }
                    Ok(limit) => parsed.session_limit = Some(limit),
                    Err(_) => {
                        return Err(acp_usage_error(&format!(
                            "--session-limit 需要 ≥1 的整数，实际 `{value}`"
                        )))
                    }
                }
            }
            "--agent" => match next_value(args, &mut idx) {
                Ok(value) => parsed.agent = Some(value),
                Err(message) => return Err(acp_usage_error(&message)),
            },
            "-p" | "--prompt" => match next_value(args, &mut idx) {
                Ok(value) => parsed.prompt = Some(value),
                Err(message) => return Err(acp_usage_error(&message)),
            },
            "--output-format" => {
                let value = match next_value(args, &mut idx) {
                    Ok(value) => value,
                    Err(message) => return Err(acp_usage_error(&message)),
                };
                parsed.output_format = match OutputFormat::from_str(&value) {
                    Ok(format) => format,
                    Err(error) => {
                        return Err(acp_usage_error(&format!(
                            "{error}（acp 模式只接受 text|jsonl|sse，不得静默退化）"
                        )))
                    }
                };
            }
            "--workspace" => match next_value(args, &mut idx) {
                Ok(value) => parsed.workspace = Some(PathBuf::from(value)),
                Err(message) => return Err(acp_usage_error(&message)),
            },
            "--timeout-secs" => {
                let value = match next_value(args, &mut idx) {
                    Ok(value) => value,
                    Err(message) => return Err(acp_usage_error(&message)),
                };
                match value.parse::<u64>() {
                    // ADVISORY#6：0 不是合法预算（会立即超时）；显式拒绝为用法错误，绝不静默。
                    Ok(0) => {
                        return Err(acp_usage_error(
                            "--timeout-secs 必须 ≥ 1（0 不是合法预算；缺省预算请省略该参数）",
                        ))
                    }
                    Ok(secs) if secs <= MAX_TIMEOUT_SECS => {
                        parsed.timeout = Some(Duration::from_secs(secs))
                    }
                    Ok(secs) => {
                        return Err(acp_usage_error(&format!(
                            "--timeout-secs {secs} 超出上限 {MAX_TIMEOUT_SECS}（无上限的预算不可兑现）"
                        )))
                    }
                    Err(_) => {
                        return Err(acp_usage_error(&format!(
                            "--timeout-secs 需要 0..={MAX_TIMEOUT_SECS} 的整数，实际 `{value}`"
                        )))
                    }
                }
            }
            "--resume" => match next_value(args, &mut idx) {
                Ok(value) => parsed.resume = Some(value),
                Err(message) => return Err(acp_usage_error(&message)),
            },
            other => return Err(acp_usage_error(&format!("acp 子命令不接受参数 `{other}`"))),
        }
        idx += 1;
    }
    Ok(parsed)
}

/// 单轮执行：按输出形态选 sink 并跑一轮 prompt（jsonl/sse 只上行事件行；text 人类可读）。
fn run_acp_single_turn(
    session: &pony_agent_core::acp::session::AcpAgentSession,
    prompt: String,
    output_format: OutputFormat,
) -> i32 {
    let blocks = vec![pony_agent_core::acp::protocol::ContentBlock::Text { text: prompt }];
    match output_format {
        OutputFormat::Jsonl => {
            let sink = headless::JsonlSink::new();
            let code = acp_run_prompt(session, blocks, &sink);
            for line in sink.finish() {
                emit_stdout_line(&line);
            }
            code
        }
        OutputFormat::Sse => {
            let sink = headless::SseSink::new();
            let code = acp_run_prompt(session, blocks, &sink);
            for frame in sink.finish() {
                emit_stdout_line(&frame);
            }
            code
        }
        OutputFormat::Text => {
            let sink = AcpTextSink;
            acp_run_prompt(session, blocks, &sink)
        }
    }
}

/// 取下一个参数值（缺失即用法错误，绝不把下一个 flag 当值）。
fn next_value(args: &[String], idx: &mut usize) -> Result<String, String> {
    let value = args
        .get(*idx + 1)
        .ok_or_else(|| format!("参数 `{}` 缺少取值", args[*idx]))?;
    *idx += 1;
    Ok(value.clone())
}


/// B185 契约：`--list-sessions` 的 JSONL 输出面（**冻结**；执行链路 B185-E 接线）。
///
/// 输出契约（stdout，JSONL；不打印 banner）：
/// - 每会话**一行**：`{"sessionId":..., "cwd":..., "title"?:..., "updatedAt"?:...,
///   "additionalDirectories"?:[...]}`（可选字段缺省时**不出现在该行**，不写 null）；
/// - 若 `session/list` 响应带 `nextCursor`，**额外一行**：`{"nextCursor":"..."}`；
/// - 空列表 ⇒ Exit 0 且**零行**（不打印 `[]`）；
/// - `-32601` ⇒ stderr 输出 `acp_method_not_supported` 前缀 + **非 0** 退出（不静默空列表）；
/// - 绝不调用 `session/new`（只 connect + `session/list`）。
///
/// 当前为契约桩：`AcpAgentSession::connect_for_cli` 在本批是签名空桩，执行链路由 B185-E 接线；
/// 此处显式返回非 0，绝不 panic，也绝不伪造会话行。
/// B185：`ponycli acp --list-sessions` 的 CLI 接线（JSONL 格式化在 `acp/session.rs`，单一真源）。
///
/// 契约：不建会话（`connect_for_cli` 只 initialize）；stdout 只有 JSONL；空列表 Exit 0 零行；
/// `-32601` ⇒ stderr `acp_method_not_supported` + Exit 3 且 stdout 零行。
fn acp_list_sessions(
    spec: &pony_agent_core::acp::config::AcpAgentSpec,
    workspace: &Path,
    cursor: Option<&str>,
    limit: Option<u32>,
) -> i32 {
    use pony_agent_core::acp::protocol::ListSessionsRequest;
    let mut session =
        match pony_agent_core::acp::session::AcpAgentSession::connect_for_cli(spec, workspace, &[]) {
            Ok(session) => session,
            Err(error) => return acp_session_error(error),
        };
    let request = ListSessionsRequest {
        cwd: Some(workspace.to_string_lossy().into_owned()),
        cursor: cursor.map(str::to_string),
        limit,
        meta: None,
    };
    let lines = match session.list_sessions_jsonl(request) {
        Ok(lines) => lines,
        Err(error) => {
            // 错误发生在任何输出之前 ⇒ stdout 零行；Unsupported 由 acp_session_error 打
            // `acp_method_not_supported` + Exit 3。
            session.close();
            return acp_session_error(error);
        }
    };
    for line in lines {
        emit_stdout_line(&line);
    }
    session.close();
    ExitCode::Success.code()
}

fn acp_usage_error(message: &str) -> i32 {
    eprintln!("\x1b[31mArgument Error:\x1b[0m {message}");
    eprintln!("用法：ponycli acp [--agent <id>] [--list] [--list-sessions [--session-cursor <c>] [--session-limit <n>]] [-p|--prompt <text>] [--output-format jsonl|sse] [--workspace <dir>] [--timeout-secs <n>] [--resume <session-id>]");
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
            Ok(text) => emit_stdout_line(&text),
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

/// B184：`--resume` 失败面映射（**绝不静默新建**）。
///
/// `Unsupported`（agent 不支持 load / `-32601` / `loadSession=false`）⇒ stderr `acp_unavailable`
/// 文案 + 既有「配置/不支持」退出码档（`ExitCode::Config`）；其余传输层错误走
/// [`acp_session_error`] 的既有分档。
fn acp_resume_error(
    spec: &pony_agent_core::acp::config::AcpAgentSpec,
    session_id: &str,
    error: pony_agent_core::acp::transport::AcpTransportError,
) -> i32 {
    use pony_agent_core::acp::transport::AcpTransportError;
    if let AcpTransportError::Unsupported(message) = &error {
        eprintln!(
            "[ponycli] acp_unavailable: agent `{}` 无法恢复会话 `{session_id}`（{message}）；不静默新建会话。",
            spec.id
        );
        return ExitCode::Config.code();
    }
    acp_session_error(error)
}

fn acp_run_prompt<S: TurnEventSink>(
    session: &pony_agent_core::acp::session::AcpAgentSession,
    blocks: Vec<pony_agent_core::acp::protocol::ContentBlock>,
    sink: &S,
) -> i32 {
    // 预算（`--timeout-secs` → `session.set_turn_budget`）与取消（宿主令牌 / `cancel()`）统一由
    // session 裁决；CLI 侧不再自建 `recv_timeout + cancel` 兜底（双份预算会与 session 抢发
    // session/cancel）。超预算由 session 归约为 `acp_timeout`，退出码见 `acp_result_exit_code`。
    acp_result_exit_code(&session.prompt(blocks, sink))
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
        AcpTransportError::Unsupported(message) => {
            eprintln!("[ponycli] acp_method_not_supported: {message}");
            ExitCode::Config.code()
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
                    // text 模式同样收口：BrokenPipe ⇒ 安静退出（B184）。
                    emit_stdout_bytes(text.as_bytes());
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
    println!("  --timeout-secs <n>         Bound the delegated turn (1..=86400); expiry exits with code 7");
    println!("  --resume <session-id>      Resume an existing ACP session; unsupported agents fail");
    println!("                             closed with acp_unavailable (never silently starts a new session)");
    println!("  --list-sessions            List this agent's sessions as JSONL (one JSON object per line)");
    println!("                             [--session-cursor <c>] [--session-limit <n>] paging passthrough;");
    println!("                             trailing \"nextCursor\" line when paged; unsupported agent ⇒");
    println!("                             acp_method_not_supported (exit 3); mutually exclusive with -p / --list");
    println!("  -h, --help                 Show this help");
    println!();
    println!("Config discovery: $XDG_CONFIG_HOME/pony-agent/acp-agents.json (missing ⇒ builtin defaults).");
}
