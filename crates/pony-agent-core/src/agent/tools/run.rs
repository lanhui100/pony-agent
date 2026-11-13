//! `workspace_run_command` 面：进程生命周期、超时与提升、危险命令拦截、
//! 退出码分类与 Windows 批处理封装。
use super::*;
use super::exec::aborted_result;
use super::exec::error_result;
use super::exec::error_result_with_evidence;
use super::exec::json_string;

/// 契约 CT-1（`tool-timeout-policy.md` v2）：Run 未显式传 `timeoutMs` 时的统一默认值，
/// 与命令文本无关（CT-2 已删除按命令子串自适应的旧启发式）。
pub(super) const DEFAULT_RUN_TIMEOUT_MS: u64 = 120_000;

/// 契约 CT-1/CT-3：Run 的 `timeoutMs` 协议上限，显式值一律夹紧到闭区间
/// `[1, MAX_RUN_TIMEOUT_MS]`。
pub(super) const MAX_RUN_TIMEOUT_MS: u64 = 600_000;

/// 通用 `ToolExecutionPolicy` 默认 deadline（**非** Run 专属）：契约只覆盖 Run，
/// 其他工具的默认 deadline 维持 10s，不随 [`DEFAULT_RUN_TIMEOUT_MS`] 变化。
pub(super) const DEFAULT_TOOL_TIMEOUT_MS: u64 = 10_000;

/// 契约 v7 / CT-P9：Run 到期「promote 成后台 job」的开关 env。**默认开启（opt-out）**：
/// env 缺失即开启；`0`/`false`/`off`/`no` 关闭后行为与波次一逐字一致
/// （kill + `error.code=timeout`）；未识别值回落默认开启并打 warn。
/// 背景与已知风险（含 Windows 真机 e2e 未验证）见 `run-timeout-promote.md` §5。
pub(super) const RUN_PROMOTE_ON_TIMEOUT_ENV: &str = "PONY_AGENT_RUN_PROMOTE_ON_TIMEOUT";

/// promote 开启时给 `kill_after` 安全网加的宽限期：定时器必须**晚于** poll 循环的 deadline
/// 到达，否则两者同毫秒时定时器线程先醒、会在 promote 之前抢杀进程（进而使 P1/P2/P5 必红）。
pub(super) const PROMOTE_KILL_AFTER_GRACE_MS: u64 = 1_000;

pub(super) const TOOL_TIMEOUT_RETRY_MAX_ATTEMPTS: u32 = 2;

impl ToolRouter {
    pub(super) fn run_command(&self, call: &ToolCall, context: &ToolExecutionContext) -> ToolResult {
        let Some(command) = call.arguments.get("command").and_then(Value::as_str) else {
            return error_result(
                TOOL_WORKSPACE_RUN_COMMAND,
                "missing_argument",
                "缺少必填参数 `command`。".to_string(),
                Some(
                    "参数示例：{\"command\":\"git status\",\"cwd\":\".\",\"timeoutMs\":5000}"
                        .to_string(),
                ),
            );
        };

        let command = command.trim();
        if command.is_empty() {
            return error_result(
                TOOL_WORKSPACE_RUN_COMMAND,
                "empty_command",
                "参数 `command` 不能为空字符串。".to_string(),
                Some("请提供要执行的命令文本。".to_string()),
            );
        }

        if let Some(reason) = denied_run_command_reason(command) {
            return error_result(
                TOOL_WORKSPACE_RUN_COMMAND,
                "command_denied",
                reason,
                Some("请改用只读、非破坏性的工作区内命令。".to_string()),
            );
        }

        // wave-9 P1-3：Windows 专用拦截 —— `cmd` 不支持行首内联 env 赋值。门控为
        // `cfg!(windows)`（运行时布尔），非 Windows 平台该形态是合法 POSIX 语法，严禁拒绝；
        // 这里只拒绝并给出纠正提示，绝不自动改写用户命令。
        if cfg!(windows) {
            if let Some(reason) = posix_inline_env_assignment_reason(command) {
                return error_result(
                    TOOL_WORKSPACE_RUN_COMMAND,
                    "windows_inline_env_unsupported",
                    reason,
                    Some(
                        "请改写为 `set VAR=value && cmd` 形式后重试；本工具不会自动改写你的命令。"
                            .to_string(),
                    ),
                );
            }
        }

        let ws_id = self.extract_call_workspace_id(call, context);
        let cwd_input = call
            .arguments
            .get("cwd")
            .and_then(Value::as_str)
            .unwrap_or(".");
        // PA-080 修复（code-review P1-1）：cwd 用 Write 语义判定，授权的外部目录不得作为
        // 命令工作目录（spec：cwd SHALL be inside workspace root or controlled tmp）。
        let cwd = match self.resolve_workspace_dir_for_execution(cwd_input, context, ws_id) {
            Ok(value) => value,
            Err((code, message)) => {
                return error_result(TOOL_WORKSPACE_RUN_COMMAND, &code, message, None)
            }
        };
        // 契约 CT-1/CT-2/CT-3：超时解析统一走 `resolve_run_timeout_ms`（已删除按命令子串
        // 自适应的旧启发式；未传 `timeoutMs` 时任何命令都取默认值）。
        let timeout_ms = resolve_run_timeout_ms(&call.arguments, command);

        // Sandbox gate (design Decision 7, PA-076 task 5.5, phase-5 review P1-2): the legacy
        // Run always runs `enforce_sandbox`. With no backend registered it fails closed via
        // `NoSandboxBackend` (Unavailable), so a Run never silently downgrades to a
        // full-parent-environment shell. An explicitly registered backend decides the verdict.
        // PA-080（consultant 修复）：sandbox 基准用会话级 workspace root，与 cwd 判定一致。
        let execution_root = match self.resolved_workspace_root(context, ws_id) {
            Ok(root) => root,
            Err((code, message)) => {
                return error_result(TOOL_WORKSPACE_RUN_COMMAND, &code, message, None);
            }
        };
        let sandbox_request = SandboxRequest {
            workspace_root: execution_root.display().to_string(),
            allow_network: false,
            environment_allowlist: Vec::new(),
            isolate_environment: true,
        };
        let backend: &dyn SandboxBackend = match &self.sandbox_backend {
            Some(backend) => backend.as_ref(),
            None => &NoSandboxBackend,
        };
        if let Err(reason) = enforce_sandbox(backend, &sandbox_request) {
            return error_result(
                TOOL_WORKSPACE_RUN_COMMAND,
                "sandbox_denied",
                reason,
                Some("无人值守 Run 需要可用的沙箱；请通过 host 审批显式允许。".to_string()),
            );
        }

        // Session-scoped process lifecycle (PA-076 tasks 5.3-5.5). Each legacy Run gets its own
        // opaque session id so handles can never be replayed across runs.
        let session_id = legacy_run_session_id();

        // Bug A 修复：Windows 上先把命令逐字写入受控 tmp 目录（<root>/.tmp/，PA-080 布局）
        // 的批处理文件（UTF-8 无 BOM，CRLF 换行；`fs::write` 写完即关闭句柄，spawn 前已完整
        // 落盘），再经 `cmd /C <批处理文件>` 执行，绕开 `cmd /C "cd /d … && …"` 单参数内嵌
        // 引号被 CommandLineToArgvW/cmd 双重剥离导致的引号损坏与 `|`/`&` 误切分。文件名用
        // 本次 run 的唯一 session_id（pony-agent-run-{session_id}.cmd）；进程退出（含超时
        // 被杀）后删除。
        let batch_script_path = if cfg!(windows) {
            let shell_cwd = windows_shell_path(&cwd.display().to_string());
            // fail-closed：cwd 不应含 `"`（Windows 路径不可能含；若含，批处理内引号结构
            // 会被破坏且无处转义）。
            if shell_cwd.contains('"') {
                return error_result(
                    TOOL_WORKSPACE_RUN_COMMAND,
                    "spawn_failed",
                    "工作目录路径包含非法字符 `\"`。".to_string(),
                    Some("请更换工作区路径后重试。".to_string()),
                );
            }
            let tmp_dir = self.controlled_tmp_dir(&execution_root);
            let batch_path = tmp_dir.join(format!("pony-agent-run-{session_id}.cmd"));
            // spawn 前 fail-closed 检查批处理文件绝对路径（在写文件之前检查，避免残留）：
            // 含 `" & | < > ( ) @ ^` 任一 cmd 元字符则无法安全放进 `cmd /C` 行（cmd 引号
            // 剥离后会按 `&` 分裂 → 静默坏执行），拒绝执行并提示更换目录。含空格放行：
            // std 以引号包裹该参数，cmd 的 /C 引号算法规则 1 按"可执行文件路径"保留引号。
            // `cmd /C` 行只允许路径，命令体绝不允许回到 `/C` 行（不做 caret 转义）。
            let path_text = batch_path.display().to_string();
            let rejected = ['"', '&', '|', '<', '>', '(', ')', '@', '^']
                .iter()
                .any(|metachar| path_text.contains(*metachar));
            if rejected {
                return error_result(
                    TOOL_WORKSPACE_RUN_COMMAND,
                    "spawn_failed",
                    format!("命令临时目录路径包含 cmd 无法安全引用的特殊字符：{path_text}。"),
                    Some(
                        "请将工作区移动到不含 & | < > ( ) @ ^ 的路径后重试。".to_string(),
                    ),
                );
            }
            if let Err(error) = fs::create_dir_all(&tmp_dir) {
                return error_result(
                    TOOL_WORKSPACE_RUN_COMMAND,
                    "batch_script_write_failed",
                    format!("创建命令临时目录失败：{}。", error),
                    Some("请确认工作区可写，稍后重试。".to_string()),
                );
            }
            let script = build_windows_batch_script(command, &shell_cwd);
            if let Err(error) = fs::write(&batch_path, script.as_bytes()) {
                return error_result(
                    TOOL_WORKSPACE_RUN_COMMAND,
                    "batch_script_write_failed",
                    format!("写入命令批处理文件失败：{}。", error),
                    Some("请确认工作区可写，稍后重试。".to_string()),
                );
            }
            Some(batch_path)
        } else {
            None
        };
        let (program, arguments) =
            workspace_command_parts_with_batch(command, &cwd, batch_script_path.as_deref());
        let handle = match self.process_manager.start(&ProcessStartRequest {
            session_id: session_id.clone(),
            program,
            arguments,
            sandbox: sandbox_request,
        }) {
            Ok(value) => value,
            Err(error) => {
                remove_windows_batch_script(batch_script_path.as_deref());
                return error_result(
                    TOOL_WORKSPACE_RUN_COMMAND,
                    "spawn_failed",
                    format!("启动命令失败：{}。", error),
                    Some("请确认命令语法正确，且当前环境存在对应可执行文件。".to_string()),
                )
            }
        };

        // 启动后立即关闭 stdin 管道（Run 从不写 stdin）：无控制台（CREATE_NO_WINDOW）下的
        // 控制台程序（如 Windows PowerShell 5.1）会在交互输入句柄上永久阻塞等待输入，
        // 管道保持开启是唯一挂起源——EOF 打破阻塞，命令正常执行退出。错误忽略：
        // 无管道/已退出属正常竞态（幂等容忍由调用方决定）。
        let _ = self.process_manager.close_stdin(&session_id, &handle);

        // wave-62 C3：取消观察者（turn 令牌 ⇒ 真杀子进程树），随命令结束自动退出。
        let cancellation = context.cancellation.clone();
        let _cancel_watcher = CommandCancelWatcher::spawn(
            cancellation.clone(),
            self.process_manager.clone(),
            session_id.clone(),
            handle.clone(),
        );

        // 契约 v6 / CT-P7：Run 到期是否 promote 成后台 job（opt-in，默认关闭）。
        // `mut`：若检测到进程是被 `kill_after` 安全网打死的，就必须按超时处理、不得 promote。
        let mut promote_on_timeout = run_promote_on_timeout_enabled();

        // Arm the kill timer as a safety net and run a bounded poll loop until `Exited`. The
        // loop accumulates per-poll output; truncation flags are sticky per stream.
        //
        // promote 开启时安全网延后一个宽限窗口。原因（实测竞态，勿删）：`kill_after` 是
        // 「arming 后 sleep timeout_ms」，而下面的 `deadline` 是 arming **之后**才算的、并且
        // 循环每 20ms 才检查一次到期 ⇒ 定时器线程必然先于循环的到期判定醒来，会在 promote
        // 之前把进程杀掉。宽限让循环始终先到 deadline、先撤防，定时器只作崩溃兜底。
        let kill_after_duration = if promote_on_timeout {
            Duration::from_millis(timeout_ms.saturating_add(PROMOTE_KILL_AFTER_GRACE_MS))
        } else {
            Duration::from_millis(timeout_ms)
        };
        self.process_manager
            .kill_after(&session_id, &handle, kill_after_duration);
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        let mut stdout = String::new();
        let mut stderr = String::new();
        let mut stdout_truncated = false;
        let mut stderr_truncated = false;
        let mut exit_code: Option<i32> = None;
        let mut timed_out = false;
        let mut timeout_cleanup_error: Option<String> = None;
        let mut cancelled = false;

        // 到期后的确认窗口（CT-P7a）：`deadline` 只是「该判超时了」的时点，不是「进程已死」的
        // 证据；极短 timeoutMs（如 1ms）下命令往往在 spawn 后数毫秒内就正常退出。
        const TIMEOUT_CONFIRM_MS: u64 = 20;

        let mut wait_error: Option<String> = None;
        loop {
            // wave-62 C3：**取消优先**——令牌已取消 ⇒ 立即终止进程树并以 aborted 返回，
            // 不等待命令自然结束、也不被 timeout 分支抢先。
            if cancellation
                .as_ref()
                .is_some_and(|token| token.is_cancelled())
            {
                // 幂等补杀：watcher 通常已先杀；此处再兜一次（错误忽略，证据由 aborted 结果承载）。
                let _ = self.process_manager.cancel(&session_id, &handle);
                cancelled = true;
                break;
            }

            // CT-P7a：**先 poll，再判超时**。先看进程是否已经退出，退出即走正常成功路径，
            // 绝不把「已成功退出」误报成 timeout/promoted。
            let polled = poll_run_child(
                &self.process_manager,
                &session_id,
                &handle,
                &mut stdout,
                &mut stderr,
                &mut stdout_truncated,
                &mut stderr_truncated,
            );
            let exited = match polled {
                Ok(RunPollOutcome::Exited { exit_code: code }) => {
                    exit_code = code;
                    if self
                        .process_manager
                        .was_killed_by_kill_after(&session_id, &handle)
                    {
                        // 被 `kill_after` 安全网打死 ≠ 自然退出：仍按超时结果返回，
                        // 否则真超时会被误报成 non_zero_exit(-1)（且绝不会 promote 尸体）。
                        timed_out = true;
                        promote_on_timeout = false;
                    }
                    true
                }
                Ok(RunPollOutcome::Running) => false,
                Err(error) => {
                    wait_error = Some(error);
                    true
                }
            };
            if exited {
                // 取消与自然退出竞态：令牌已取消 ⇒ 仍按取消语义返回（取消优先）。
                if wait_error.is_none()
                    && cancellation
                        .as_ref()
                        .is_some_and(|token| token.is_cancelled())
                {
                    cancelled = true;
                }
                break;
            }

            // 只在「已 poll 且确认仍在运行」之后才判到期。
            if Instant::now() >= deadline {
                // CT-P7a 的确认轮次（仅 promote 开启时）：到期瞬间进程可能正在退出
                // （`echo hi` + `timeoutMs:1` 即此类），再给一个 tick 并 poll 一次；这期间退出
                // ⇒ 正常成功，不判超时、不 promote。
                //
                // 为什么**不**在 promote 关闭时做：契约 CT-P7 要求关闭路径的 `kill_after` 仍按
                // `timeout_ms` 整点开火（不加宽限）。到期后再 poll 只会看到「被安全网 SIGKILL」
                // 的 -1 退出码，无法区分「自然退出」与「被超时杀掉」，反而会把真超时误报成
                // non_zero_exit。关闭路径因此保持到期即杀（与波次一逐字一致）。
                if promote_on_timeout {
                    thread::sleep(Duration::from_millis(TIMEOUT_CONFIRM_MS));
                    let confirmed = poll_run_child(
                        &self.process_manager,
                        &session_id,
                        &handle,
                        &mut stdout,
                        &mut stderr,
                        &mut stdout_truncated,
                        &mut stderr_truncated,
                    );
                    let exited_after_deadline = match confirmed {
                        Ok(RunPollOutcome::Exited { exit_code: code }) => {
                            exit_code = code;
                            if self
                                .process_manager
                                .was_killed_by_kill_after(&session_id, &handle)
                            {
                                timed_out = true;
                                promote_on_timeout = false;
                            }
                            true
                        }
                        Ok(RunPollOutcome::Running) => false,
                        Err(error) => {
                            wait_error = Some(error);
                            true
                        }
                    };
                    if exited_after_deadline {
                        break;
                    }
                    // 契约 CT-P1：到期**不杀**，进程所有权交回 job 侧；清理动作由 promote
                    // 分支决定（红线：此处不得 kill / 删批处理脚本）。
                    timed_out = true;
                    break;
                }
                if let Err(error) = self.process_manager.kill_if_exists(&session_id, &handle) {
                    timeout_cleanup_error = Some(error);
                }
                timed_out = true;
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }

        if let Some(error) = wait_error {
            let cleanup = self.process_manager.kill_if_exists(&session_id, &handle).err();
            let message = match cleanup {
                Some(cleanup) => format!("轮询命令状态失败：{error}；清理进程树失败：{cleanup}。"),
                None => format!("轮询命令状态失败：{error}。"),
            };
            remove_windows_batch_script(batch_script_path.as_deref());
            return error_result(
                TOOL_WORKSPACE_RUN_COMMAND,
                "wait_failed",
                message,
                Some("请重试，或更换更简单的命令。".to_string()),
            );
        }

        if cancelled {
            // wave-62 A3（取消路径对称清理）：与 `timed_out` 分支同口径 —— 幂等二次 kill
            // （首次可能只做终止并保留条目，第二次才真正回收）+ 存活探针留证，
            // 不让被取消命令的进程条目滞留；watcher 已随命令结束退出。
            let first_cleanup_error = self.process_manager.cancel(&session_id, &handle).err();
            let retry_cleanup_error = self
                .process_manager
                .kill_if_exists(&session_id, &handle)
                .err();
            let cleanup_error = first_cleanup_error.or(retry_cleanup_error);
            let still_tracked = self.process_manager.is_tracked(&session_id, &handle);
            if let Some(error) = cleanup_error.as_ref() {
                crate::agent::otel::log::log_json_with_fields(
                    "warn",
                    "run_command cancel path cleanup failed",
                    None,
                    &[
                        ("session_id", json!(session_id)),
                        ("handle", json!(handle)),
                        ("error", json!(error.to_string())),
                    ],
                );
            }
            if still_tracked {
                crate::agent::otel::log::log_json_with_fields(
                    "warn",
                    "run_command process entry still tracked after cancel cleanup",
                    None,
                    &[
                        ("session_id", json!(session_id)),
                        ("handle", json!(handle)),
                    ],
                );
            }
            remove_windows_batch_script(batch_script_path.as_deref());
            return aborted_result(
                TOOL_WORKSPACE_RUN_COMMAND,
                "tool_cancelled",
                "本轮已被停止，命令进程已终止。".to_string(),
            );
        }

        if timed_out {
            if promote_on_timeout {
                // 契约 CT-P1..CT-P5：到期不杀，撤防 `kill_after` 安全网后把仍在运行的进程
                // 登记成后台 job。任一步失败 ⇒ 回落到下面的波次一 kill + timeout 错误路径
                // （CT-P4 兼容语义：绝不出现「既报 timeout 又留活进程」的半吊子状态）。
                let disarm = self.process_manager.disarm_kill_after(&session_id, &handle);
                match disarm {
                    Ok(()) => {
                        match register_promoted(
                            self.process_manager.clone(),
                            session_id.clone(),
                            handle.clone(),
                            command.to_string(),
                            &stdout,
                            &stderr,
                            stdout_truncated,
                            stderr_truncated,
                            // B009：把 turn 取消令牌交给 promoted job，使 Stop 仍能终止它
                            // （前台 Run 的 watcher 会随本次返回被 drop）。
                            cancellation.clone(),
                            // B012：promoted 作业同样记归属会话。
                            context.session_id.as_deref(),
                            // B013：把仍在被读取的临时批处理脚本移交 job 生命周期清理。
                            // POSIX 下 `batch_script_path` 天然为 None；Windows 下是
                            // `<ws>/.tmp/pony-agent-run-{session_id}.cmd`（spawn 前写入）。
                            batch_script_path.clone(),
                        ) {
                            Ok(job_id) => {
                                // 红线：promote 路径不得 kill / cancel / 删除批处理脚本，
                                // 也不进入 cleanupError 逻辑 —— 进程继续由 job 生命周期管辖。
                                return ToolResult {
                                    tool_name: TOOL_WORKSPACE_RUN_COMMAND.to_string(),
                                    status: "ok".to_string(),
                                    output: json_string(json!({
                                        "ok": true,
                                        "cwd": self.display_workspace_relative(&cwd),
                                        "absoluteCwd": cwd.display().to_string(),
                                        "command": command,
                                        "delegateTool": "run_shell",
                                        "timeoutMs": timeout_ms,
                                        // CT-P1：到期事实（timedOut）与接续事实（promoted/jobId）
                                        // 必须同时可见，且不得再出现 error.code=timeout。
                                        "timedOut": true,
                                        "promoted": true,
                                        "jobId": job_id,
                                        "exitCode": Value::Null,
                                        "stdout": stdout,
                                        "stderr": stderr,
                                        "stdoutTruncated": stdout_truncated,
                                        "stderrTruncated": stderr_truncated,
                                        "error": Value::Null,
                                        "summary": {
                                            "text": format!(
                                                "命令在 {} 执行超过超时上限 {} ms，已转为后台作业（jobId={}）：\
用 `job_output` 续读增量输出、`job_kill` 终止；**不要重复执行同一命令**（它已在后台继续运行）。",
                                                self.display_workspace_relative(&cwd),
                                                timeout_ms,
                                                job_id
                                            )
                                        },
                                        "permission": {
                                            "requiresApproval": false,
                                            "permissionScope": "workspace.execute",
                                            "hostMediated": false,
                                            "permissionProfile": "builtin",
                                            "approvalMode": "none",
                                            "decisionSource": "runtime"
                                        }
                                    })),
                                    duration_ms: 0,
                                };
                            }
                            Err(error) => crate::agent::otel::log::log_json_with_fields(
                                "warn",
                                "run_command promote register failed; falling back to timeout kill",
                                None,
                                &[
                                    ("session_id", json!(session_id)),
                                    ("handle", json!(handle)),
                                    ("error", json!(error)),
                                ],
                            ),
                        }
                    }
                    Err(error) => crate::agent::otel::log::log_json_with_fields(
                        "warn",
                        "run_command promote disarm failed; falling back to timeout kill",
                        None,
                        &[
                            ("session_id", json!(session_id)),
                            ("handle", json!(handle)),
                            ("error", json!(error)),
                        ],
                    ),
                }
            }
            // wave-9 P1-1：无条件二次调用幂等清理。首次 `kill_if_exists` 对**仍在运行**的进程
            // 只做终止并保留条目（终态可轮询），第二次调用才真正释放条目；旧的 `or_else` 在首次
            // 清理报错时会短路掉第二次调用，留下被追踪的句柄。此处改为无条件重试，并保留首个
            // 非 None 清理错误作为证据。
            let retry_cleanup_error = self
                .process_manager
                .kill_if_exists(&session_id, &handle)
                .err();
            let cleanup_error = timeout_cleanup_error.or(retry_cleanup_error);
            // 存活探针（平台无关）：清理后句柄仍被追踪 → 执行环境不可用，本轮工具调用必须终止。
            let still_tracked = self.process_manager.is_tracked(&session_id, &handle);
            let (code, message, hint) = if still_tracked {
                let message = match &cleanup_error {
                    Some(error) => format!(
                        "命令执行超过超时上限 {} ms；进程树清理后句柄仍被追踪（清理错误：{}）。",
                        timeout_ms, error
                    ),
                    None => format!(
                        "命令执行超过超时上限 {} ms；进程树清理后句柄仍被追踪。",
                        timeout_ms
                    ),
                };
                (
                    "environment_unavailable",
                    message,
                    "执行环境不可用，已终止本轮工具调用；请重启运行环境后重试。".to_string(),
                )
            } else {
                let message = match &cleanup_error {
                    Some(error) => format!(
                        "命令执行超过超时上限 {} ms；进程树清理失败：{}。",
                        timeout_ms, error
                    ),
                    None => format!("命令执行超过超时上限 {} ms，已终止。", timeout_ms),
                };
                ("timeout", message, classify_timeout_hint(command))
            };
            remove_windows_batch_script(batch_script_path.as_deref());
            return error_result_with_evidence(
                TOOL_WORKSPACE_RUN_COMMAND,
                code,
                message,
                Some(hint),
                json!({
                    "command": command,
                    "timeoutMs": timeout_ms,
                    // 契约 CT-4：超时路径在证据根级显式标记 `timedOut: true`。
                    "timedOut": true,
                    "exitCode": exit_code,
                    "stdout": stdout,
                    "stderr": stderr,
                    "stdoutTruncated": stdout_truncated,
                    "stderrTruncated": stderr_truncated,
                    "cleanupError": cleanup_error,
                }),
            );
        }

        // Exited: clean up the session entry (idempotent when already exited). A Job failure is
        // surfaced rather than silently turning a lifecycle cleanup failure into tool success.
        if let Err(error) = self.process_manager.kill_if_exists(&session_id, &handle) {
            remove_windows_batch_script(batch_script_path.as_deref());
            return error_result(
                TOOL_WORKSPACE_RUN_COMMAND,
                "cleanup_failed",
                format!("命令已退出，但进程树清理失败：{}。", error),
                Some("请重试以完成进程树清理。".to_string()),
            );
        }

        // Windows 批处理临时文件使命结束（Exited 分支）：删除（幂等，失败忽略不阻塞结果）。
        remove_windows_batch_script(batch_script_path.as_deref());

        let succeeded = exit_code == Some(0);
        let error_payload = (!succeeded).then(|| {
            // ADR 0055：Windows 退出码 9009 = 「找不到命令」，属可自愈的错误而非环境故障，
            // 直接给出去向明确的纠错提示（真实 trace 连续两条生产轮次的首个命令即撞 9009）。
            let base_message = format!(
                "命令执行完成，但退出码为 {}。",
                exit_code.map(|value| value.to_string()).unwrap_or_else(|| "null".to_string())
            );
            let message = match windows_command_not_found_hint(cfg!(windows), exit_code, &command) {
                Some(hint) => format!("{base_message}{hint}"),
                None => base_message,
            };
            json!({
                "code": "non_zero_exit",
                "kind": "non_zero_exit",
                "message": message,
                "exitCode": exit_code,
                // P0-3：退出码分类（与 `exitCode` 同层，只加字段；`code`/`kind`/`message` 逐字不改）。
                // `exit_code = None`（拿不到退出码，如被信号终止且无 status code）时为 null。
                "exitCodeClass": exit_code.map(classify_exit_code)
            })
        });

        ToolResult {
            tool_name: TOOL_WORKSPACE_RUN_COMMAND.to_string(),
            status: if succeeded {
                "ok".to_string()
            } else {
                "error".to_string()
            },
            output: json_string(json!({
                "ok": succeeded,
                "cwd": self.display_workspace_relative(&cwd),
                "absoluteCwd": cwd.display().to_string(),
                "command": command,
                "delegateTool": "run_shell",
                "timeoutMs": timeout_ms,
                // 契约 CT-4：未走超时路径 ⇒ 根级显式 `timedOut: false`（不靠缺失推断）。
                "timedOut": false,
                "exitCode": exit_code,
                "stdout": stdout,
                "stderr": stderr,
                "stdoutTruncated": stdout_truncated,
                "stderrTruncated": stderr_truncated,
                "error": error_payload,
                "summary": {
                    "text": format!(
                        "命令在 {} 执行完成，退出码为 {}。",
                        self.display_workspace_relative(&cwd),
                        exit_code.map(|value| value.to_string()).unwrap_or_else(|| "null".to_string())
                    )
                },
                "permission": {
                    "requiresApproval": false,
                    "permissionScope": "workspace.execute",
                    "hostMediated": false,
                    "permissionProfile": "builtin",
                    "approvalMode": "none",
                    "decisionSource": "runtime"
                }
            })),
            duration_ms: 0,
        }
    }
}


pub(super) fn denied_run_command_reason(command: &str) -> Option<String> {
    let normalized = command.trim().to_lowercase();
    let tokens = normalized
        .split_whitespace()
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();

    if tokens.is_empty() {
        return None;
    }

    if is_rm_destructive_command(&tokens) {
        return Some("命令包含高风险删除模式 `rm -rf`，当前 runtime 已拒绝执行。".to_string());
    }
    if is_git_reset_hard_command(&tokens) {
        return Some(
            "命令包含高风险片段 `git reset --hard`，当前 runtime 已拒绝执行。".to_string(),
        );
    }
    if is_git_clean_force_command(&tokens) {
        return Some("命令包含高风险片段 `git clean -fd`，当前 runtime 已拒绝执行。".to_string());
    }

    let deny_markers = [
        "remove-item",
        " del ",
        "erase ",
        "rmdir /s",
        "mkfs",
        "shutdown",
        "reboot",
        "halt",
        "poweroff",
        "diskpart",
    ];

    let padded = format!(" {normalized} ");
    deny_markers
        .iter()
        .find(|marker| padded.contains(**marker))
        .map(|marker| format!("命令包含高风险片段 `{marker}`，当前 runtime 已拒绝执行。"))
        .or_else(|| {
            if is_windows_format_command(&tokens) {
                Some(
                    "命令包含高风险磁盘格式化模式 `format <drive>`，当前 runtime 已拒绝执行。"
                        .to_string(),
                )
            } else {
                None
            }
        })
}

/// wave-9 P1-2：超时 hint 分类（私有纯函数，无 I/O、无全局状态、不做命令改写）。
///
/// 命中网络类子命令标记（`push`/`fetch`/`clone`/`pull`/`ls-remote`）时，给出「网络/认证/代理」
/// 三类可能原因与可自检步骤；未命中时逐字返回既有默认文案，保持既有断言可继续成立。
/// Windows 退出码 9009 的纠错提示（ADR 0055）。
///
/// `9009` 由 `CreateProcessW` 在「可执行文件不存在」时回填，等价于「这条命令在 Windows 上不存在」。
/// 实测（trace `f47fd028` / `a1751e1b`）两条生产轮次的**首个**命令即撞 9009，且模型随后仍反复
/// 试探 POSIX 工具 ⇒ 在错误消息里直接给替换建议，比让模型自行猜测省 1~2 个 hop。
///
/// 语义边界：只**追加上下文提示**，不改变 `code` / `kind` / `exitCode` 任何既有字段，
/// 也不把失败改写成成功（非零退出仍是 error）。
/// `is_windows` 作显式入参 ⇒ 纯函数可在 Linux CI 上被单测覆盖。
pub(super) fn windows_command_not_found_hint(
    is_windows: bool,
    exit_code: Option<i32>,
    command: &str,
) -> Option<String> {
    if !is_windows || exit_code != Some(9009) {
        return None;
    }
    let _ = command;
    Some(
        " 该退出码在 Windows 上表示「命令不存在」（cmd 的 9009）。请改用 Windows 等价写法：\
路径与文件用 dir / where 替代 ls / which；文本搜索用 findstr /s /n /i 关键词 *；\
查看文件用 type 替代 cat；环境变量用 set VAR=value && cmd 而非 NAME=value cmd；\
或整体改用 powershell -NoProfile -Command 执行。"
            .to_string(),
    )
}

/// 契约 CT-5（`tool-timeout-policy.md` v3）：超时 hint 必须同时给出「后台执行」与「显式上限」
/// 两条出路。非网络分支**逐字**等于该文案；网络分支在既有网络自检文案后**追加**同一段文案
/// （单一真源，避免两条分支文案漂移）。
pub(super) const TIMEOUT_HINT_BACKGROUND_ADVICE: &str = "长任务请改用后台执行 `job_start`（`job_output` 取增量输出、`job_kill` 终止，无超时上限）；或显式传入更大的 timeoutMs（上限 600000）；仍不足时缩短或拆分命令。";

/// 契约 CT-5 / 可测接缝 S2：按命令文本分类给出超时纠错 hint。
///
/// 命中网络类子命令标记（`push`/`fetch`/`clone`/`pull`/`ls-remote`）时，先给「网络/认证/代理」
/// 三类可能原因与可自检步骤，再追加与默认分支相同的「后台执行 / 上限 600000」出路；
/// 未命中时逐字返回 [`TIMEOUT_HINT_BACKGROUND_ADVICE`]（非网络命令不得被网络分类污染）。
pub fn classify_timeout_hint(command: &str) -> String {
    const NETWORK_COMMAND_MARKERS: [&str; 5] = ["push", "fetch", "clone", "pull", "ls-remote"];
    if NETWORK_COMMAND_MARKERS
        .iter()
        .any(|marker| command.contains(marker))
    {
        return format!(
            "命令疑似网络类操作，超时可能来自网络/认证/代理三类原因之一：请依次自检 `git ls-remote <remote-url>` 是否可连通、`git config --get remote.origin.url` 是否指向预期远端、SSH 身份自检（`ssh -T git@<host>`）是否有效；确认后再重试。{TIMEOUT_HINT_BACKGROUND_ADVICE}"
        );
    }
    TIMEOUT_HINT_BACKGROUND_ADVICE.to_string()
}

#[cfg(test)]
thread_local! {
    /// `Some(bool)` ⇒ 用例显式覆写 promote 开关（`None` = 未覆写，回落 env）。
    static PROMOTE_ON_TIMEOUT_OVERRIDE: std::cell::Cell<Option<bool>> =
        const { std::cell::Cell::new(None) };
}

/// 测试专用：设置 promote 开关覆写，返回先前的覆写值（便于用例收尾还原）。
#[cfg(test)]
pub(super) fn set_promote_on_timeout_override(value: Option<bool>) -> Option<bool> {
    PROMOTE_ON_TIMEOUT_OVERRIDE.with(|slot| slot.replace(value))
}

/// 契约 v7 / CT-P9：Run 超时 promote 开关（**默认开启，opt-out**）。
///
/// 语义（BT014 默认翻转）：
/// - env **缺失** ⇒ `true`（默认 promote）；
/// - `1`/`true`/`on`/`yes`（trim + 大小写不敏感）⇒ `true`；
/// - `0`/`false`/`off`/`no` ⇒ `false`（回到「kill + `error.code=timeout`」的波次一行为）；
/// - **未识别值** ⇒ 回落默认 `true`，但打一条 warn —— 避免「开关拼错却静默生效」。
pub(super) fn run_promote_on_timeout_enabled() -> bool {
    // 测试覆写为什么用 thread_local 而不是进程级 env：Run 在调用线程上同步执行，thread_local
    // 既能让用例精确注入「promote 关闭」的波次一语义，又不会污染并行跑的其他用例
    // （进程级 `set_var` 会被同二进制的并发用例读到，是负载敏感 flaky 的温床）。
    #[cfg(test)]
    {
        if let Some(override_value) = PROMOTE_ON_TIMEOUT_OVERRIDE.with(|slot| slot.get()) {
            return override_value;
        }
    }

    let Ok(value) = std::env::var(RUN_PROMOTE_ON_TIMEOUT_ENV) else {
        return true;
    };
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "on" | "yes" => true,
        "0" | "false" | "off" | "no" => false,
        _ => {
            eprintln!(
                "[tools] unrecognized {RUN_PROMOTE_ON_TIMEOUT_ENV}={value:?}; falling back to default (promote enabled)"
            );
            true
        }
    }
}

/// 契约 CT-1/CT-2/CT-3 的可测接缝：解析一次 Run 调用的生效超时（毫秒）。
///
/// 语义（`tool-timeout-policy.md` v2）：
/// - CT-2：不再按命令文本做子串启发式（旧 `build`/`test`/`cargo`/`check`/`install` ⇒ 60s），
///   未传 `timeoutMs` 时任何命令（含空命令）一律取 [`DEFAULT_RUN_TIMEOUT_MS`]；
/// - CT-3：显式整数按闭区间 `[1, MAX_RUN_TIMEOUT_MS]` 夹紧（`<1` ⇒ 1，含负数；`>MAX` ⇒ MAX）；
///   非数字（如 `"x"`）、非整数（如 `1.5`）、`null` 一律回落 [`DEFAULT_RUN_TIMEOUT_MS`]；
///   区间内原样保留。
///
/// `command` 仅为契约签名的前向兼容入参（CT-2 之后不再参与判定），因此不读其值。
pub fn resolve_run_timeout_ms(arguments: &serde_json::Value, command: &str) -> u64 {
    let _ = command;
    let Some(requested) = arguments.get("timeoutMs") else {
        return DEFAULT_RUN_TIMEOUT_MS;
    };
    if let Some(requested) = requested.as_u64() {
        return requested.clamp(1, MAX_RUN_TIMEOUT_MS);
    }
    // 负整数（JSON `-5`）`as_u64` 为 None，但契约 CT-3 要求走「<1 ⇒ 1」夹紧分支，
    // 而不是非法输入 fallback。
    if let Some(requested) = requested.as_i64() {
        return if requested < 1 {
            1
        } else {
            requested.min(MAX_RUN_TIMEOUT_MS as i64) as u64
        };
    }
    DEFAULT_RUN_TIMEOUT_MS
}

/// wave-9 P1-3：识别 POSIX 行首内联环境变量赋值形态（私有纯函数，平台无关）。
///
/// 仅识别「行首 `NAME=value` 赋值后接真实命令」形态（值可带单/双引号）；单独赋值、非行首赋值
/// 一律返回 `None`。调用点以 `cfg!(windows)` 门控，非 Windows 平台不得拒绝该合法 POSIX 语法。
/// 本函数只产出纠正提示，**严禁**自动改写用户命令。
pub(super) fn posix_inline_env_assignment_reason(command: &str) -> Option<String> {
    let trimmed = command.trim_start();
    let bytes = trimmed.as_bytes();
    match bytes.first() {
        Some(byte) if byte.is_ascii_alphabetic() || *byte == b'_' => {}
        _ => return None,
    }

    let mut index = 0usize;
    while index < bytes.len() && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_') {
        index += 1;
    }
    let name = &trimmed[..index];
    if bytes.get(index) != Some(&b'=') {
        return None;
    }
    index += 1;

    let value_start = index;
    match bytes.get(index) {
        Some(quote) if *quote == b'"' || *quote == b'\'' => {
            let quote = *quote;
            index += 1;
            let mut closed = false;
            while index < bytes.len() {
                if bytes[index] == quote {
                    closed = true;
                    index += 1;
                    break;
                }
                index += 1;
            }
            if !closed {
                return None;
            }
        }
        Some(_) => {
            while index < bytes.len() && !bytes[index].is_ascii_whitespace() {
                index += 1;
            }
        }
        None => return None,
    }
    let value = &trimmed[value_start..index];

    // 「赋值后接命令」：赋值之后必须由空白分隔且剩余命令非空。
    if !matches!(bytes.get(index), Some(byte) if byte.is_ascii_whitespace()) {
        return None;
    }
    let remainder = trimmed[index..].trim();
    if remainder.is_empty() {
        return None;
    }

    Some(format!(
        "检测到 POSIX 行首内联环境变量赋值 `{name}={value}`，Windows `cmd` 不支持该语法；请改写为 `set VAR=value && cmd` 形式（例如 `set {name}={value} && {remainder}`）后重试。"
    ))
}

pub(super) fn is_rm_destructive_command(tokens: &[&str]) -> bool {
    if tokens.first().copied() != Some("rm") {
        return false;
    }
    let mut has_recursive = false;
    let mut has_force = false;
    for token in tokens.iter().skip(1) {
        if let Some(flags) = token.strip_prefix('-') {
            has_recursive |= flags.contains('r');
            has_force |= flags.contains('f');
        }
    }
    has_recursive && has_force
}

pub(super) fn is_git_reset_hard_command(tokens: &[&str]) -> bool {
    if tokens.first().copied() != Some("git") {
        return false;
    }
    let has_reset = tokens.contains(&"reset");
    let has_hard = tokens
        .iter()
        .any(|token| *token == "--hard" || *token == "-h");
    has_reset && has_hard
}

pub(super) fn is_git_clean_force_command(tokens: &[&str]) -> bool {
    if tokens.first().copied() != Some("git") {
        return false;
    }
    if !tokens.contains(&"clean") {
        return false;
    }
    tokens.iter().any(|token| {
        if let Some(flags) = token.strip_prefix('-') {
            flags.contains('f') && flags.contains('d')
        } else {
            false
        }
    })
}

pub(super) fn is_windows_format_command(tokens: &[&str]) -> bool {
    if tokens.first().copied() != Some("format") {
        return false;
    }
    tokens.iter().skip(1).any(|token| {
        token.len() == 2
            && token.ends_with(':')
            && token
                .chars()
                .next()
                .map(|value| value.is_ascii_alphabetic())
                .unwrap_or(false)
    })
}

/// P0-3：把进程退出码归类成可检索的**分类名**（trace 诊断的第一刀）。
///
/// 口径（本卡冻结的最小集合）：
/// - `101` ⇒ `compile_or_panic`（cargo/编译失败、Rust panic 的典型码）
/// - `127` ⇒ `command_not_found`（shell「命令不存在」约定码）
/// - `126` ⇒ `not_executable`（shell「找到但不可执行」约定码）
/// - `130` ⇒ `interrupted`（128 + SIGINT(2)，用户中断）
/// - `137` ⇒ `killed`（128 + SIGKILL(9)，被强杀）
/// - `143` ⇒ `terminated`（128 + SIGTERM(15)，被正常终止）
/// - 其余（含 `0` / 负数）⇒ `general_failure`（兜底；本函数不做「是否成功」判断）
///
/// 纯函数：无 IO、无 env、无状态，便于单测与未来的前端/后端共用口径。
pub fn classify_exit_code(exit_code: i32) -> &'static str {
    match exit_code {
        101 => "compile_or_panic",
        127 => "command_not_found",
        126 => "not_executable",
        130 => "interrupted",
        137 => "killed",
        143 => "terminated",
        _ => "general_failure",
    }
}

/// 一次 Run `poll` 的结论（只关心「是否已退出」与退出码）。
pub(super) enum RunPollOutcome {
    Running,
    Exited { exit_code: Option<i32> },
}

/// 对 Run 的受管进程做一次 poll，并把增量输出累加进调用方的累积器（截断标记 sticky）。
///
/// 契约 CT-P7a：超时判定必须建立在「**已 poll 且仍在运行**」之上。把 poll + 累加抽成单一
/// 实现，供「常规轮询」与「到期后的确认轮次」共用，避免两处语义漂移。
pub(super) fn poll_run_child(
    process_manager: &ProcessManager,
    session_id: &str,
    handle: &str,
    stdout: &mut String,
    stderr: &mut String,
    stdout_truncated: &mut bool,
    stderr_truncated: &mut bool,
) -> Result<RunPollOutcome, String> {
    let result = process_manager.poll(session_id, handle)?;
    stdout.push_str(&result.stdout);
    stderr.push_str(&result.stderr);
    *stdout_truncated |= result.stdout_truncated;
    *stderr_truncated |= result.stderr_truncated;
    if result.state == ProcessState::Exited {
        Ok(RunPollOutcome::Exited {
            exit_code: result.exit_code,
        })
    } else {
        Ok(RunPollOutcome::Running)
    }
}

/// Per-run session id for the legacy `Run` path: each invocation gets an opaque session so the
/// `ProcessManager` handle is never shared or replayable across runs.
pub(super) fn legacy_run_session_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_nanos() as u64)
        .unwrap_or(0);
    format!("legacy-run-{nanos:016x}")
}

/// Core `ProcessStartRequest` builder for a shell command string. The child runs under the
/// platform shell (`cmd` on Windows, `sh -lc` elsewhere) exactly like the legacy
/// `spawn_workspace_command`, but `ProcessStartRequest` has no `cwd` field, so the resolved
/// workspace directory is baked into the shell command with a leading `cd`.
///
/// Windows branch (Bug A 修复): the user command is executed via a temp batch file instead of
/// being inlined into a `cmd /C "… && …"` single argument.
///
/// 损坏机理：`std::process::Command` 按 CommandLineToArgvW 规则把参数内嵌双引号转义为 `\"`，
/// 而 cmd.exe 按自己的引号剥离规则解析（首尾引号剥离、`\"` 残留），于是含引号的命令
/// （如 `powershell -NoProfile -Command "Get-CimInstance …"`）引号失衡、`|`/`&` 被误切分、
/// 真实输出丢失（stdout 只剩命令文本回显）。把命令原样写入批处理文件再经 `cmd /C` 执行，
/// `/C` 行只携带批处理文件路径（绝不允许命令体回到 `/C` 行），命令文本逐字保留，引号配对
/// 与管道/`&` 切分完全由 cmd 按批处理原文解析。路径不做 caret 转义：含空格时 std 会用一对
/// 引号包裹，cmd 的 `/C` 引号算法规则 1 按"可执行文件路径"保留引号；含 `" & | < > ( ) @ ^`
/// 元字符的路径在 run_command 侧 spawn 前 fail-closed 拒绝。
pub(super) fn workspace_command_parts_with_batch(
    command: &str,
    cwd: &Path,
    batch_script_path: Option<&Path>,
) -> (String, Vec<String>) {
    if cfg!(windows) {
        // run_command 在 Windows 上必定先写好批处理文件再传入其绝对路径。
        let batch_script_path = batch_script_path.expect(
            "workspace_command_parts_with_batch: Windows 分支必须提供批处理文件绝对路径",
        );
        // 批处理文件自身路径同样需要剥 `\\?\` 前缀：批处理内容中的 cwd 已由
        // run_command 经 windows_shell_path 归一化，但 spawn 行 `/C <路径>` 中的
        // 路径此前未处理——canonicalize 产物 `\\?\C:\...` 前缀 cmd.exe 无法执行
        // （无空格 ws 报 "The system cannot find the path specified."，含空格 ws 被
        // cmd 引号算法在空格处截断），导致 Windows 上 Run 整体失败。
        let batch_shell_path = windows_shell_path(&batch_script_path.display().to_string());
        (
            "cmd".to_string(),
            vec!["/C".to_string(), batch_shell_path],
        )
    } else {
        let quoted = format!("'{}'", cwd.display().to_string().replace('\'', "'\\''"));
        (
            "sh".to_string(),
            vec!["-lc".to_string(), format!("cd {quoted} && {command}")],
        )
    }
}

/// Bug A 修复：生成执行 `run_command` 命令的 Windows 批处理脚本文本（UTF-8 无 BOM，CRLF 换行）。
/// 固定行序：
/// ```text
/// @echo off
/// chcp 65001 >nul 2>&1
/// cd /d "<cwd>" || exit /b 1
/// <command 原样>
/// exit /b %errorlevel%
/// ```
/// - `chcp 65001 >nul 2>&1`：在 zh-CN CP936 控制台下把活动代码页切到 UTF-8，避免批处理中的
///   中文路径/输出在读取/回显时乱码（静默，不刷屏）。`2>&1` 兜底"无控制台"场景
///   （子进程经 CREATE_NO_WINDOW 隐藏窗口后不再有控制台句柄，chcp 会失败并写 stderr，
///   重定向后不污染工具输出）。
/// - `cd /d "<cwd>" || exit /b 1`：cwd 写入双引号（不再使用 `cmd_escape_path` 的 caret 转义），
///   cd 失败必须短路退出，等价于旧 `&&` 语义——绝不在错误目录里继续执行（防止删除类命令
///   落到错误路径）。cwd 中的 `%` 双写为 `%%` 防止被批处理当作环境变量展开；Windows 路径
///   不可能含 `"`，若含由调用方 fail-closed。
/// - `command` 逐字保留：不转义双引号、不做 caret 转义、不改写 `%`。`%VAR%` 环境变量展开与
///   `cmd /C` 直跑一致；单 `%` 循环变量、`%%` 折叠与 `%1` 位置参数属于交互式批处理特例，
///   与直跑 `cmd /C` 的语义差异被接受（记录于此处，不处理）。
/// - `exit /b %errorlevel%` 把命令退出码原样传播给 cmd 进程，从而进入进程管理的 exit_code。
pub(super) fn build_windows_batch_script(command: &str, cwd: &str) -> String {
    let escaped_cwd = cwd.replace('%', "%%");
    format!(
        "@echo off\r\nchcp 65001 >nul 2>&1\r\ncd /d \"{escaped_cwd}\" || exit /b 1\r\n{command}\r\nexit /b %errorlevel%\r\n"
    )
}

/// 幂等删除 Windows 批处理临时文件：NotFound 视为已删除忽略，其余失败忽略（不阻塞工具结果，
/// 残留文件由系统 TEMP 清理兜底）。
pub(super) fn remove_windows_batch_script(path: Option<&Path>) {
    if let Some(path) = path {
        let _ = fs::remove_file(path);
    }
}

/// Normalize a Windows path for use inside a cmd command line: strips the `\\?\` extended-length
/// prefix that `fs::canonicalize` produces (cmd's `cd` rejects it) and restores `\\?\UNC\` to the
/// conventional `\\server\share` form.
pub(super) fn windows_shell_path(path: &str) -> String {
    if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{rest}");
    }
    if let Some(rest) = path.strip_prefix(r"\\?\") {
        return rest.to_string();
    }
    path.to_string()
}

/// wave-62 C3：`workspace_run_command` 的取消观察者（子进程生命周期内有效）。
///
/// 背景：`spawn_blocking` 不可抢占，`ProcessManager` 只有在被显式 `cancel`/超时定时器命中时
/// 才会终止子进程；因此"turn 被停止"必须有一条**运行期**观察者把取消翻译成真杀进程。
///
/// 语义：
/// - 每 `TICK_MS` 分片检查一次 turn 令牌（总节拍 ≤ 契约 `turn_cancel_poll_ms` = 100ms）；
/// - 令牌一旦取消 ⇒ `ProcessManager::cancel`（真终止进程树，而非仅回收句柄）；
/// - `Drop` 置位 stop 并 `join` ⇒ 线程随命令结束退出，**不泄漏常驻线程**（join 最长等一个分片）；
/// - 无令牌（`None`）⇒ 不建线程（零开销、行为与 wave-61 及之前完全一致）。
pub(super) struct CommandCancelWatcher {
    stop: Arc<AtomicBool>,
    join: Option<thread::JoinHandle<()>>,
}

impl CommandCancelWatcher {
    /// 分片间隔：≤100ms 轮询上界下仍能快速响应取消，也让 `Drop` 的 join 很快返回。
    const TICK_MS: u64 = 10;

    fn spawn(
        token: Option<Arc<crate::agent::budget::CancellationToken>>,
        process_manager: ProcessManager,
        session_id: String,
        handle: String,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let Some(token) = token else {
            return Self { stop, join: None };
        };
        let stop_for_thread = Arc::clone(&stop);
        let join = thread::spawn(move || {
            while !stop_for_thread.load(Ordering::SeqCst) {
                if token.is_cancelled() {
                    // 幂等：句柄可能已被主循环/超时定时器回收，错误无需上抛。
                    let _ = process_manager.cancel(&session_id, &handle);
                    return;
                }
                thread::sleep(Duration::from_millis(Self::TICK_MS));
            }
        });
        Self {
            stop,
            join: Some(join),
        }
    }
}

impl Drop for CommandCancelWatcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}
