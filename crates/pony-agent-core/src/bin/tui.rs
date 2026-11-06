//! pony-tui: Claude Code 风格的终端全屏 Agent 交互界面
//! 基于 ratatui + crossterm 构建，支持对话历史、流式输出、工具状态与多行输入。

// 作为测试目标编译时 main() 不会被执行，运行期才用到的项会被判定为 dead_code；
// 仅在 test cfg 下放行，生产构建仍严格保持 -D warnings（不掩盖真实死代码）。
#![cfg_attr(test, allow(dead_code))]

use crossterm::{
    event::{
        self, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture, Event,
        KeyboardEnhancementFlags, KeyCode, KeyModifiers, PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen},
};
use std::io::Write;
use pony_agent_core::agent::config::{
    ProviderCapabilityPreset, ProviderModelCapabilities, ProviderModelConfig, ProviderReasoningEffort,
    ProviderRegistryStore, ProviderRegistryView, ProviderSelectionResolver, ResolvedProviderSelection,
    ThinkingParamPattern,
};
use pony_agent_core::agent::control_plane::{
    HostControlPlane, HostControlPlaneBuilder, StartTurnStreamCommand, StopTurnCommand,
};
use pony_agent_core::agent::goal::{GoalActivation as AgentGoalActivation, GoalData};
// W3：Team 可见性数据源（全部来自真实运行时，不造数据）。
use pony_agent_core::agent::metrics_snapshot::TurnMetricsSnapshot;
use pony_agent_core::agent::model_preferences::{ModelPreferences, ModelPreferencesStore};
use pony_agent_core::agent::orchestration::{
    inbox_pending_len, is_driving, list_agents, team_task_list, teammate_session_id, TeamTaskItem,
    TeammateStatus,
};
use pony_agent_core::agent::provider::{ProviderAuthType, ProviderProtocol};
use pony_agent_core::agent::todo::{TodoItem, TodoStatus as StoreTodoStatus};
use pony_agent_core::agent::runtime::{
    compact_outcome_message, AgentRuntimeBuilder, ManualCompactionOutcome, TurnInput,
    TurnStreamEvent,
};
use pony_agent_core::agent::session::SessionStore;
use pony_agent_core::agent::stop_escalation::StopReason;
// wave-60 §C8：双击 Esc 的纯判定状态机（与桌面端 `src/lib/runtime/stop-sequence.ts` 同判据）。
use pony_agent_core::agent::stop_sequence::{StopIdentity, StopSequenceState, STOP_SEQUENCE_MS};
use pony_agent_core::agent::turn_flow::TurnEventSink;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
    Frame, Terminal,
};
// 初始化页品牌标：终端图形协议（sixel / kitty / iTerm2）渲染真实位图；
// 仅支持 halfblocks 的终端仍走自绘字符画（更清晰，配色与 Canvas 一致）。
use ratatui_image::{
    picker::{Picker, ProtocolType},
    protocol::StatefulProtocol,
    FilterType, Resize as ImageResize, StatefulImage,
};
use std::io::IsTerminal;
use std::io;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

// T2c：SIGINT/SIGTERM 信号到达时不直接杀死进程，而是置位该标志；
// 主循环每个 tick（≤16ms）检测后正常 break，走统一的终端恢复保证器。
static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);

/// 终端**输入侧**协议还原串（收到后终端不再把按键编码成转义序列）。
///
/// 残留这些模式时的现象：退出 TUI 后敲任何键都变成 `ESC[117u` / `ESC[9;5u` 这类
/// kitty 键盘协议编码，被 shell 当正文回显，终端看起来"输入全是乱码"。
/// - `<1u` / `<u`：弹掉 kitty 键盘增强协议栈（crossterm 的弹栈写作 `<1u`，部分终端只认 `<u`）
/// - `>4;0m`：关 xterm modifyOtherKeys
/// - `?1000l` `?1002l` `?1003l` `?1006l` `?1015l`：关各级鼠标追踪（含 any-motion 与 SGR）
/// - `?2004l`：关 bracketed paste
pub const TERMINAL_INPUT_MODES_RESET: &[u8] =
    b"\x1b[<1u\x1b[<u\x1b[>4;0m\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?1015l\x1b[?2004l";

/// 终端**输出侧**还原串：退出备用屏 + 显示光标。
pub const TERMINAL_SCREEN_RESET: &[u8] = b"\x1b[?1049l\x1b[?25h";

/// raw mode 之前的 termios 快照指针（信号处理器内直接 `tcsetattr`，不分配内存）。
#[cfg(unix)]
static ORIGINAL_TERMIOS_PTR: std::sync::atomic::AtomicPtr<libc::termios> =
    std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());

/// 预打开的还原写通道（`/dev/tty` 的 fd；-1 ⇒ 不可用）。
///
/// 为什么预打开：信号处理器里只能调用 async-signal-safe 的 `write(2)`（不能 open/分配内存）；
/// 且 `io::stdout()` 被重定向或已返回 EIO 时，只有直写 `/dev/tty` 还能把还原串送达终端。
#[cfg(unix)]
static RESTORE_TTY_FD: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);

/// 进入 raw mode 之前快照 termios 并预打开 `/dev/tty`。
///
/// 还原通道必须在"终端可能已经坏掉"之前就绪，否则信号/panic 路径无通道可用。
#[cfg(unix)]
fn capture_terminal_state() {
    unsafe {
        let mut original: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(libc::STDIN_FILENO, &mut original) == 0 {
            ORIGINAL_TERMIOS_PTR.store(Box::into_raw(Box::new(original)), Ordering::SeqCst);
        }
    }
    if let Ok(tty) = std::fs::OpenOptions::new().write(true).open("/dev/tty") {
        use std::os::unix::io::AsRawFd;
        RESTORE_TTY_FD.store(tty.as_raw_fd(), Ordering::SeqCst);
        // 进程级写通道：生命周期与进程一致，提前 forget 以免 fd 在 handler 中被关闭。
        std::mem::forget(tty);
    }
}

/// 非 unix（Windows）无 `/dev/tty` 文件语义，还原统一走 stdout + crossterm 命令。
#[cfg(not(unix))]
fn capture_terminal_state() {}

/// 信号上下文里的最佳努力还原：只调用 async-signal-safe 的 `write(2)` / `tcsetattr(3)`。
#[cfg(unix)]
fn restore_terminal_for_signal() {
    unsafe {
        let tty_fd = RESTORE_TTY_FD.load(Ordering::SeqCst);
        if tty_fd >= 0 {
            libc::write(
                tty_fd,
                TERMINAL_INPUT_MODES_RESET.as_ptr() as *const libc::c_void,
                TERMINAL_INPUT_MODES_RESET.len(),
            );
        }
        let original = ORIGINAL_TERMIOS_PTR.load(Ordering::SeqCst);
        if !original.is_null() {
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, original);
        }
        if tty_fd >= 0 {
            libc::write(
                tty_fd,
                TERMINAL_SCREEN_RESET.as_ptr() as *const libc::c_void,
                TERMINAL_SCREEN_RESET.len(),
            );
        }
    }
}

/// 信号到达的诊断行：用固定栈缓冲 + `write` 输出，避免在信号上下文里分配内存。
#[cfg(unix)]
fn write_signal_notice(sig: libc::c_int) {
    const PREFIX: &[u8] = b"[pony-tui] shutdown signal ";
    const SUFFIX: &[u8] = b" received\n";
    let mut buf = [0u8; PREFIX.len() + 3 + SUFFIX.len()];
    let mut len = 0usize;
    buf[..PREFIX.len()].copy_from_slice(PREFIX);
    len += PREFIX.len();
    let mut digits = [0u8; 3];
    let mut remaining = sig.max(0) as u32;
    let mut count = 0usize;
    loop {
        digits[count] = b'0' + (remaining % 10) as u8;
        count += 1;
        remaining /= 10;
        if remaining == 0 || count == digits.len() {
            break;
        }
    }
    for index in (0..count).rev() {
        buf[len] = digits[index];
        len += 1;
    }
    buf[len..len + SUFFIX.len()].copy_from_slice(SUFFIX);
    len += SUFFIX.len();
    unsafe {
        libc::write(
            libc::STDERR_FILENO,
            buf.as_ptr() as *const libc::c_void,
            len,
        );
    }
}

#[cfg(unix)]
extern "C" fn handle_shutdown_signal(sig: libc::c_int) {
    // 先做一次 async-signal-safe 的还原：主循环此刻可能正卡在长耗时调用里，
    // 只等下一次 tick 的话，用户的终端会继续停在 raw + kitty 键盘协议态。
    restore_terminal_for_signal();
    SHUTDOWN_REQUESTED.store(true, Ordering::SeqCst);
    write_signal_notice(sig);
}

/// `libc` 是 `[target.'cfg(unix)'.dependencies]` 依赖，Windows 目标不存在该 crate，
/// 因此整套信号接管必须 cfg(unix) 门控（否则 Windows 编译报 E0433）。
#[cfg(unix)]
fn install_shutdown_signal_handlers() {
    unsafe {
        // Rust 禁止把函数项直接 cast 成整数（"direct cast of function item into an integer"），
        // 必须先经 `*const ()` 中转再转成 sighandler_t。
        let handler = handle_shutdown_signal as *const () as libc::sighandler_t;
        libc::signal(libc::SIGINT, handler);
        libc::signal(libc::SIGTERM, handler);
        // pty 会话被关闭（如 `script` 收到 SIGINT 后退出）时，TUI 进程会收到 SIGHUP；
        // 同样走优雅退出路径以执行终端恢复，而不是默认终止并留下 alternate screen。
        libc::signal(libc::SIGHUP, handler);
    }
}

/// 非 unix（Windows）无 POSIX 信号语义：不接管信号，保持控制台默认行为。
#[cfg(not(unix))]
fn install_shutdown_signal_handlers() {}

/// 把还原串写到终端：unix 优先直写 `/dev/tty`（stdout 被重定向 / 返回 EIO 时仍有效），
/// 失败再退回 stdout。
#[cfg(unix)]
fn write_restore_bytes(bytes: &[u8]) -> io::Result<()> {
    let tty_fd = RESTORE_TTY_FD.load(Ordering::SeqCst);
    if tty_fd >= 0 {
        let written = unsafe {
            libc::write(
                tty_fd,
                bytes.as_ptr() as *const libc::c_void,
                bytes.len(),
            )
        };
        if written == bytes.len() as isize {
            return Ok(());
        }
    }
    let mut out = io::stdout();
    out.write_all(bytes)?;
    out.flush()
}

/// 非 unix：无 `/dev/tty` 语义，普通文本（如退出提示）直接写 stdout。
#[cfg(not(unix))]
fn write_restore_bytes(bytes: &[u8]) -> io::Result<()> {
    let mut out = io::stdout();
    out.write_all(bytes)?;
    out.flush()
}

/// 退出后打印的会话续接提示（必须能整行复制执行）。
///
/// 只负责**文案**，写入通道由 [`print_resume_hint`] 负责：两者分离后，文案可被单测
/// 直接断言，不必在无 TTY 环境里伪造终端。
pub fn resume_hint_line(session_id: &str) -> String {
    format!("继续该会话请运行：pony resume {session_id}")
}

/// 把续接提示写到真实终端（stdout 被重定向时仍能送达）。
///
/// 调用时机：**终端还原之后**。提前写会落进备用屏，用户退出后根本看不到。
fn print_resume_hint(session_id: &str) {
    let line = format!("\n{}\n", resume_hint_line(session_id));
    let _ = io::stdout().flush();
    if let Err(error) = write_restore_bytes(line.as_bytes()) {
        eprintln!("[pony-tui] 退出提示写入失败：{error}");
    }
}

/// 契约 F-D 加固：把终端从 TUI 模式彻底还原（幂等，可重复调用）。
///
/// 与旧实现的差异（对应"退出 TUI 后终端输入变乱码"缺陷的因果闭包）：
/// 1) 写通道优先 `/dev/tty`：旧实现只写 `io::stdout()`，stdout 重定向 / EIO 时整串还原丢失；
/// 2) 还原串无条件写全，不再依赖"push 是否成功"的共享标志——宁可多发一次弹栈；
/// 3) 失败向 stderr 记一条，不再 `let _ =` 静默吞掉（旧实现失败后用户零线索）。
///
/// ── wave-tui-session-resilience 冻结空桩（Protocol 契约，待 Executor 实现）──
/// 以下签名是红相测试的唯一可编译接缝；测试经 `#[path = "../src/bin/tui.rs"]`
/// 引入本模块，禁止改名/改签名/删除。实现时只允许替换函数体，不得新增依赖。
/// 语义见 `.dev-team/contract-matrix-tui-session-resilience.md` C1–C8。

const DSML_CALLS_OPEN: &str = "<｜｜DSML｜｜ calls>";
const DSML_CALLS_CLOSE: &str = "</｜｜DSML｜｜ calls>";

/// 只把「已识别 DSML 标记前缀」的半截形态判为 incomplete；普通 `<` / 代码块不误伤。
fn dsml_partial_tail_len(text: &str) -> Option<usize> {
    let trimmed = text.trim_end_matches(['\r', '\n', ' ', '\t']);
    let tail_start = trimmed.rfind('<')?;
    let tail = &trimmed[tail_start..];
    if tail.contains('>') {
        return None;
    }
    // 单个 `<` 是普通正文，不是协议前缀；至少要有 `<` 后的一个字符才算候选。
    if tail.len() <= "<".len() {
        return None;
    }
    let known_tags = [
        DSML_CALLS_OPEN,
        "<｜｜DSML｜｜ invoke",
        "<｜｜DSML｜｜ parameter",
        DSML_CALLS_CLOSE,
        "</｜｜DSML｜｜ invoke>",
        "</｜｜DSML｜｜ parameter>",
    ];
    known_tags
        .iter()
        .any(|tag| tag.starts_with(tail))
        .then_some(text.len() - tail_start)
}

/// C1/C2：清洗上游模型输出内的 DSML 工具调用协议原文（流式与历史回放共用）。
/// 返回：清洗后的用户可见文本；`incomplete=true` 表示尾部可能是被 chunk 切断的协议片段。
#[allow(unused_variables)]
pub fn sanitize_model_protocol_text(text: &str) -> (String, bool) {
    // 半截标记先截掉（流式 chunk 边界），再交给 provider 层同一纯函数做块级折叠；
    // 普通正文与 markdown 代码块逐字保真。
    let (body, incomplete) = match dsml_partial_tail_len(text) {
        Some(len) => (&text[..text.len() - len], true),
        None => (text, false),
    };
    let cleaned = pony_agent_core::agent::provider::sanitize_dsml_protocol_text(body);
    (cleaned, incomplete)
}

/// C3：判定上游错误是否为可重试瞬态错误（与 provider 层既有预算对齐，不新增重试）。
#[allow(unused_variables)]
pub fn is_transient_provider_error(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    let permanent = [
        "401", "403", "400", "unauthorized", "forbidden", "bad request",
        "missing api key", "api key", "invalid_api_key", "nonretryable",
    ];
    if permanent.iter().any(|hint| lower.contains(hint)) {
        return false;
    }
    let transient = [
        "timeout", "timed out", "429", "rate limit", "rate-limited", "503",
        "502", "504", "500", "5xx", "connection refused", "connection reset",
        "dns", "network", "connection", "temporarily unavailable",
    ];
    transient.iter().any(|hint| lower.contains(hint))
}

const ERROR_SUMMARY_GENERIC: &str = "本轮请求未能完成，请稍后再试。";
const ERROR_SUMMARY_TRANSIENT: &str = "上游服务暂时不可用，正在按既有预算重试。";
const ERROR_SUMMARY_AUTH: &str = "凭据不可用，请检查提供商配置。";

/// C4：把原始错误详情收敛为面向用户的一句话摘要（详情默认折叠，不入消息区）。
#[allow(unused_variables)]
pub fn user_facing_error_summary(error: &str) -> String {
    let lower = error.to_ascii_lowercase();
    if lower.contains("401")
        || lower.contains("403")
        || lower.contains("api key")
        || lower.contains("unauthorized")
        || lower.contains("forbidden")
    {
        ERROR_SUMMARY_AUTH.to_string()
    } else if is_transient_provider_error(error) {
        ERROR_SUMMARY_TRANSIENT.to_string()
    } else {
        ERROR_SUMMARY_GENERIC.to_string()
    }
}

/// C4：判定错误详情是否应默认折叠（终态错误 true；面向用户摘要 false）。
#[allow(unused_variables)]
pub fn should_fold_error_detail(error: &str) -> bool {
    let trimmed = error.trim();
    if matches!(
        trimmed,
        ERROR_SUMMARY_GENERIC | ERROR_SUMMARY_TRANSIENT | ERROR_SUMMARY_AUTH
    ) {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    let internal = [
        "error sending request for url", "http://", "https://", "{\"error\"",
        "stacktrace", "response body", "响应正文", "panic", "at provider::",
    ];
    trimmed.chars().count() > 200 || internal.iter().any(|hint| lower.contains(hint))
}

/// C5/C6：把中断 turn 的已落盘步骤投影为可续传检查点（向后兼容：空输入返回 None）。
#[allow(unused_variables)]
pub fn interrupted_turn_checkpoint(
    prior_assistant_text: &str,
    tool_activities: &[String],
) -> Option<String> {
    let text = prior_assistant_text.trim();
    let tools: Vec<&str> = tool_activities
        .iter()
        .map(|tool| tool.trim())
        .filter(|tool| !tool.is_empty())
        .collect();
    if text.is_empty() && tools.is_empty() {
        return None;
    }
    let mut checkpoint = String::from("[中断轮次检查点]");
    if !text.is_empty() {
        checkpoint.push('\n');
        checkpoint.push_str(text);
    }
    if !tools.is_empty() {
        checkpoint.push_str("\n已执行的工具步骤：");
        for tool in tools {
            checkpoint.push_str("\n- ");
            checkpoint.push_str(tool);
        }
    }
    Some(checkpoint)
}

/// C6：resume 时从检查点接续，返回应追加到已完成历史之后的可见文本；不重复已完成 turn。
#[allow(unused_variables)]
pub fn resume_from_checkpoint(completed_history: &str, checkpoint: Option<&str>) -> String {
    let Some(checkpoint) = checkpoint else {
        return completed_history.to_string();
    };
    let checkpoint = checkpoint.trim();
    if checkpoint.is_empty() || completed_history.contains(checkpoint) {
        return completed_history.to_string();
    }
    let mut resumed = String::with_capacity(completed_history.len() + checkpoint.len() + 1);
    resumed.push_str(completed_history);
    if !resumed.is_empty() && !resumed.ends_with('\n') {
        resumed.push('\n');
    }
    resumed.push_str(checkpoint);
    resumed
}

/// C7/C8：退出清理计划（取消在飞 turn → 有界等待 → 终端还原 → 打印 resume 提示 → 退出）。
/// `inflight_turn=true` 时计划必须包含 cancel；等待上限由 NFR 基准 external_call_timeout_ms 约束。
#[allow(unused_variables)]
pub fn exit_cleanup_plan(inflight_turn: bool, timeout_ms: u64) -> Vec<&'static str> {
    let _bounded_timeout_ms = timeout_ms.min(3000);
    if inflight_turn {
        vec!["cancel", "wait", "restore", "hint", "exit"]
    } else {
        vec!["wait", "restore", "hint", "exit"]
    }
}

#[cfg(unix)]
pub fn restore_terminal_modes() {
    // stdout 可能仍有未刷出的帧：先刷干净，避免还原串被后续帧覆盖。
    let _ = io::stdout().flush();
    if let Err(error) = write_restore_bytes(TERMINAL_INPUT_MODES_RESET) {
        eprintln!("[pony-tui] 终端还原（输入模式）失败：{error}");
    }
    if let Err(error) = disable_raw_mode() {
        eprintln!("[pony-tui] disable_raw_mode 失败：{error}");
    }
    if let Err(error) = write_restore_bytes(TERMINAL_SCREEN_RESET) {
        eprintln!("[pony-tui] 终端还原（备用屏/光标）失败：{error}");
    }
}

/// Windows 走 crossterm 命令（老 conhost 下 ANSI 直写不可靠，需走其 WinAPI 分支）。
#[cfg(not(unix))]
pub fn restore_terminal_modes() {
    let mut out = io::stdout();
    let _ = execute!(out, crossterm::event::PopKeyboardEnhancementFlags);
    let _ = execute!(out, crossterm::event::DisableBracketedPaste);
    if let Err(error) = disable_raw_mode() {
        eprintln!("[pony-tui] 终端还原（raw mode）失败：{error}");
    }
    let _ = execute!(
        out,
        crossterm::terminal::LeaveAlternateScreen,
        crossterm::event::DisableMouseCapture,
        crossterm::cursor::Show
    );
}

/// 契约 F-D 加固：panic 也先还原终端。
///
/// `TerminalGuard` 的 Drop 覆盖不到 abort / hook 侧 abort 的路径；panic hook 在 unwind 最外层
/// 之前运行，是"进程要死了也要把终端交还"的最后一道闸。与 guard 的还原互为幂等。
fn install_panic_restore_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal_modes();
        previous(info);
    }));
}

use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageRole {
    User,
    Assistant,
    ToolCall,
    ToolResult,
    System,
}

#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: MessageRole,
    pub content: String,
    pub timestamp_ms: u64,
}

pub enum UiEvent {
    InputKey(crossterm::event::KeyEvent),
    /// C21：**合成自动提交**（派发闸门唯一构造点）。
    ///
    /// 不用裸 `InputKey(Enter)`：本变体在事件循环里会重新校验「界面此刻是否仍是空闲态」，
    /// 只有空闲才回灌真正的 Enter；否则放弃本次回灌并保留输入框正文（用户可见、可再按 Enter），
    /// 杜绝合成 Enter 被中途弹出的模态吃掉（甚至误触发审批同意）。
    QueuedAutoSubmit,
    InputMouse(crossterm::event::MouseEvent),
    /// 终端尺寸变化（SSH 初次上报晚到 / 用户拉伸窗口）：必须立即整屏重绘，否则保留旧尺寸画面。
    Resized,
    /// 契约 F-C：bracketed paste 文本（由输入线程从 `Event::Paste` 转来）。
    /// 只允许插入输入缓冲，**绝不**直接提交（粘贴后仍由用户按 Enter 决定）。
    InputPaste(String),
    /// 契约 F-E：输入泵退出（poll/read 出错）。stderr 已被 dup2 到日志文件，终端上看不到，
    /// 故必须经事件通道把原因送到界面上，不能只 eprintln。
    InputPumpFailed { error: String },
    ScrollWheel(bool), // true = ScrollUp, false = ScrollDown
    StreamDelta(String),
    StreamToolCall(String),
    StreamToolResult(String),
    StreamCompleted,
    StreamFailed(String),
    /// wave-39 R1（契约冻结空桩）：本轮 token 用量回执，源自 `turn:output_end` /
    /// `turn:completed` 载荷。变体与穷尽匹配已冻结；发送侧（R8）由 Executor 接线。
    StreamUsage {
        input_tokens: u64,
        output_tokens: u64,
        cache_hit_input_tokens: u64,
        total_tokens: u64,
    },
    /// P0：per-call 指标结算快照（单一真值）。凡事件载荷携带 `metrics` 即投递本变体，
    /// 主循环写入 `TuiState::live_metrics`，指标行以快照为唯一来源（I6）。
    /// 旧事件（无 `metrics`）不产生本变体 ⇒ 走既有 legacy 路径（I7）。
    StreamMetrics(TurnMetricsSnapshot),
    /// P0：轮次起点（`turn:started`）—— 事件驱动的轮次复位信号。
    ///
    /// 存在的理由：并非所有开轮入口都经过主循环派发点（那里才调 `begin_turn_usage()`）。
    /// goal 自动续轮在**轮次线程内**直接 `start_turn_stream(...)` 开新 turn，绕过派发点；
    /// 若不复位，上一轮快照会整轮泄漏到下一轮。由事件流承担复位 ⇒ 所有入口自动覆盖
    /// （goal 续轮、stop→resume 等），不依赖调用点自觉。
    TurnStarted,
    /// /models 触发的远程模型目录拉取成功：ids 已由后台线程取得，由主循环持久化并打开面板。
    ModelsDiscovered { provider_id: String, ids: Vec<String> },
    /// /models 远程拉取失败：仅上报错误，由主循环以 System 消息呈现，不 panic、不退出。
    ModelsDiscoverFailed { provider_id: String, error: String },
    /// 触发权限请求审批对话框
    RequestPermission(PermissionRequest),
    /// 权限审批完成并回传用户决议
    PermissionDecided { request_id: String, choice: PermissionChoice },
    /// 触发 AskUser 提问交互
    AskUserPrompt { request_id: String, questions: Vec<AskUserQuestion> },
    /// 用户提交 AskUser 回答
    AskUserAnswered { request_id: String, answers: Vec<(String, Vec<String>)> },
    /// Plan 计划更新通知
    PlanUpdated(PlanState),
    /// 任务看板更新通知
    TaskBoardUpdated(Vec<TuiTask>),
    /// Goal 状态更新通知
    GoalUpdated(TuiGoal),
    /// 波次 43：本轮被宿主控制请求挂起（ask_user / approval），主循环据此拉取待决请求。
    TurnSuspended { run_id: Option<String> },
    /// wave-60 §C2/C9：停止请求已受理（**不是**终态）⇒ 进入「正在停止…」。
    /// 由 `TuiTurnSink` 的 `turn:stop_requested` 分支投递。
    StopRequested { turn_id: String },
    /// wave-60 §C9：`turn:cancelled` 终态 ⇒ 由事件驱动「已停止」并退出流式态
    /// （此前该事件被 sink 的 `_ => {}` 丢弃）。`stop_reason` = 结构化停止原因（wire `stopReason`）。
    StopTerminal {
        turn_id: String,
        stop_reason: Option<String>,
    },
    /// 契约 C-7：`/compact` 后台线程回执（真实压缩在后台线程执行，事件循环不阻塞 LLM）。
    CompactFinished {
        result: Result<ManualCompactionOutcome, String>,
    },
    /// `/update` 检查更新后台回执
    UpdateCheckFinished {
        result: Result<UpdateCheckOutcome, String>,
    },
}

/// wave-40 R1（契约冻结空桩）：Composer 中 Enter 键语义。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposerEnterAction {
    Submit,
    Newline,
}

pub fn composer_enter_action(key: &crossterm::event::KeyEvent) -> ComposerEnterAction {
    // Shift+Enter / Alt+Enter 是「换行」的显式语义；其余键（含裸 Enter）落到 Enter 分支的提交语义。
    if key.code == KeyCode::Enter
        && (key.modifiers.contains(KeyModifiers::SHIFT) || key.modifiers.contains(KeyModifiers::ALT))
    {
        ComposerEnterAction::Newline
    } else {
        ComposerEnterAction::Submit
    }
}

/// Composer 编辑类快捷键的语义（Ctrl+U / Ctrl+J / Ctrl+W / Ctrl+A / Ctrl+E）。
///
/// 这些按键改的是**输入框内容**，不是会话控制，因此在流式期间也必须生效。
/// 旧实现把整段编辑快捷键放在 `if state.is_streaming { continue; }` 之后，
/// 于是流式中 Ctrl+U（清空输入框）被静默吞掉。真值源落在这里，主循环与测试共用。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposerControlAction {
    /// Ctrl+U：清空整个输入框（无论光标在哪、是否流式中）。
    ClearInput,
    /// Ctrl+J：插入换行（与 Shift+Enter 同语义）。
    InsertNewline,
    /// Ctrl+W：删除光标前一个词。
    DeleteWordBackward,
    /// Ctrl+A：跳到行首。
    LineHome,
    /// Ctrl+E：跳到行尾。
    LineEnd,
}

pub fn composer_control_action(key: &crossterm::event::KeyEvent) -> Option<ComposerControlAction> {
    if !key.modifiers.contains(KeyModifiers::CONTROL) {
        return None;
    }
    match key.code {
        KeyCode::Char('u') => Some(ComposerControlAction::ClearInput),
        KeyCode::Char('j') => Some(ComposerControlAction::InsertNewline),
        KeyCode::Char('w') => Some(ComposerControlAction::DeleteWordBackward),
        KeyCode::Char('a') => Some(ComposerControlAction::LineHome),
        KeyCode::Char('e') => Some(ComposerControlAction::LineEnd),
        _ => None,
    }
}

/// 流式期该按键是否应被丢弃：只有**会话控制**类按键要等本轮结束，
/// 输入框编辑类快捷键不等（回归护栏见 `tests/tui_six_fixes_acceptance.rs`）。
///
/// 契约变更（`tui-streaming-input` C1，2026-11-06）：主循环已改由
/// [`streaming_key_disposition`] 裁决，本函数**保留为公共符号**（契约 §2 要求不删），
/// 既是既有测试的回归护栏，也是「编辑类快捷键不受流式影响」这一不变量的可测入口。
#[allow(dead_code)]
pub fn should_skip_while_streaming(key: &crossterm::event::KeyEvent) -> bool {
    composer_control_action(key).is_none()
}

// ── 契约区：流式期输入与排队派发（签名由协议冻结；本区只提供真值实现）──
//
// 真值源：`.dev-team/contract-matrix-tui-streaming-input.md` §1 C1–C10 / §2 权威符号表。
// 口径：`streaming_key_disposition` 是 C1–C4 的唯一按键裁决面；命令串的分类
// （`is_panel_command` / `is_session_mutating_command`）在主循环里叠加在裁决之上 ——
// 因为裁决面只拿得到「输入框是否有正文」，拿不到正文本身（冻结签名约束）。

/// C6：排队容量上限（满则拒绝入队并提示，绝不静默丢弃）。
pub const MAX_QUEUED_MESSAGES: usize = 20;

/// 流式期单键处置裁决（三态互斥）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamingKeyDisposition {
    /// 照旧走既有按键处理链（输入框编辑、Esc、面板快捷键等）。
    Forward,
    /// 不在本轮内派发：把输入正文快照入队，等本轮终态事件后按 FIFO 派发。
    QueueSubmit,
    /// 拒绝该键并保持输入框原样（不吞字、不排队）。
    Reject,
}

/// C1–C4 纯函数裁决：按键 + 「输入框是否有正文」⇒ 处置。
///
/// 口径（与契约 §1 逐条对齐）：
/// - **Enter**（裸键 / 非 Shift/Alt）＝唯一可能「提交」的键：有正文 ⇒ `QueueSubmit`
///   （流式中排队，绝不并发 spawn 第二个 turn）；无正文 ⇒ `Reject`（不入队空气泡）。
/// - **Shift/Alt+Enter** ＝ 换行（编辑动作）⇒ `Forward`。
/// - 其余按键（可打印字符、Backspace/Delete、方向键、Home/End、Tab 补全、
///   Ctrl+U/J/W/A/E 编辑快捷键、Ctrl+T/Ctrl+R 面板快捷键、F2/F3、Esc）一律 `Forward`
///   —— 流式守卫不再吞键（C1/C3）。Esc / Ctrl+C / 面板模态分支在调用点**先于**本裁决生效。
pub fn streaming_key_disposition(
    key: &crossterm::event::KeyEvent,
    composer_has_text: bool,
) -> StreamingKeyDisposition {
    if key.code != KeyCode::Enter {
        return StreamingKeyDisposition::Forward;
    }
    if key.modifiers.contains(KeyModifiers::SHIFT) || key.modifiers.contains(KeyModifiers::ALT) {
        // Shift+Enter / Alt+Enter = 换行（编辑语义，不是提交）。
        return StreamingKeyDisposition::Forward;
    }
    if composer_has_text {
        StreamingKeyDisposition::QueueSubmit
    } else {
        StreamingKeyDisposition::Reject
    }
}

/// 排队消息载荷口径（冻结）：**直接复用 `String`**，不另立结构体 ——
/// 内容即提交时刻的输入正文快照，与 `TuiState::queued_user_messages` 同源。
pub type QueuedUserMessage = String;

/// C6：入队原语。返回 `false` 表示**拒绝入队**（调用方必须给出提示，不得静默丢弃）。
///
/// 拒绝口径：空串/纯空白；斜杠命令串（走 C3/C4 分发，不当作聊天消息排队）；
/// 已达 `MAX_QUEUED_MESSAGES` 上限（绝不扩容、绝不覆盖既有条目）。
pub fn push_queued_message(queue: &mut Vec<String>, text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    // 命令串不入队：整串命中命令表（`/clear 这个需求怎么拆` 这类带正文的串不算命令，
    // 与主循环的分发口径 `parse_command_input` 同源，静态下它本来也是当普通消息发出去的）。
    if parse_command_input(trimmed).is_some() {
        return false;
    }
    // C18①：命令表之外的**历史别名**同样是命令语义，不得当成聊天消息排队
    //（`/exit` 走退出分支、`/effort` 走 /thinking 面板分支）。
    if input_is_alias(trimmed, "/exit") || input_is_alias(trimmed, "/effort") {
        return false;
    }
    if queue.len() >= MAX_QUEUED_MESSAGES {
        return false;
    }
    queue.push(trimmed.to_string());
    true
}

/// 取走队首一条待派发消息（FIFO）。空队列立即 `None`，不阻塞。
/// 单飞不变量：同一时刻最多 1 条排队消息在飞（NFR `queue_drain_single_flight`）。
pub fn drain_next_queued(state: &mut TuiState) -> Option<QueuedUserMessage> {
    if state.queued_user_messages.is_empty() {
        return None;
    }
    Some(state.queued_user_messages.remove(0))
}

/// C15：**唯一派发取件点** —— 闸门判定 + 取件收成一个生产符号
/// （主循环只能经它取件；A7 直接断言本符号，而不是测试内自建的复刻品）。
///
/// 三道前哨，任一不过即 `None`（且**不**动队列）：
/// 1. 输入框非空 ⇒ `None`：绝不覆盖用户草稿，也不覆盖「已回灌但尚未被消费」的上一条派发正文；
/// 2. [`should_drain_queued`] 五判据不过 ⇒ `None`（单飞 / 空闲 / 非流式 / 无在飞轮次线程）；
/// 3. 否则 `drain_next_queued`（FIFO、一次 1 条、空队列 `None`）。
///
/// 本符号**只取件、不写回**：写回输入框由调用点负责，且必须经
/// [`composer_buffer_for_dispatch`] 净化（C14 修订裁定：净化口径独立成纯函数，便于机械断言）。
pub fn take_next_queued_for_dispatch(
    state: &mut TuiState,
    is_streaming: bool,
    ui_idle: bool,
    turn_in_flight: bool,
    dispatch_pending: bool,
) -> Option<QueuedUserMessage> {
    if !state.input_buffer.trim().is_empty() {
        return None;
    }
    if !should_drain_queued(
        is_streaming,
        ui_idle,
        state.queued_count(),
        turn_in_flight,
        dispatch_pending,
    ) {
        return None;
    }
    drain_next_queued(state)
}

/// C14：派发正文写回输入框前的**净化缓冲区**（纯函数，契约裁定 2026-11-06）。
///
/// 返回「正文 + 一个尾部空格」，**不改正文内容**（不 trim、不转义）——提交路径自己 `trim()`，
/// 所以发给模型的正文与入队快照逐字一致。尾部空格消除两条「正文永远发不出去」的自救路径：
/// ① 正文以 `\` 结尾 ⇒ 主循环的「行尾反斜杠 + Enter = 换行」分支会吞掉提交；
/// ② 正文是 `/hel` 这类命令前缀 ⇒ `matching_commands()` 补全会把 Enter 劫持成另一条命令。
pub fn composer_buffer_for_dispatch(message: &str) -> String {
    format!("{message} ")
}

// ── C19–C23：第三/四轮契约新增纯函数面（全部不依赖运行态，便于机械断言）──

/// C19：PlanMode 面板真正要「吞掉」的按键（Esc / Up / Down）。
///
/// 其余按键**不得**被面板分支吞掉：计划面板只是界面提示（不限制工具执行），
/// 打开时输入框仍应完整可用（旧实现 `_ => {}` + `continue` 让整段输入失效）。
pub fn plan_mode_consumes_key(key: &crossterm::event::KeyEvent) -> bool {
    matches!(key.code, KeyCode::Esc | KeyCode::Up | KeyCode::Down)
}

/// C20：流式期 Enter 的**前置动作**（在入队裁决之前判定）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamingEnterPre {
    /// 行尾 `\` ⇒ 换行（用户显式要换行，不是提交）。
    Newline,
    /// 只是命令前缀（如 `/he`）且补全可见 ⇒ 接受补全，不是提交。
    Complete,
    /// 其余 ⇒ 交给入队/分发判定。
    Submit,
}

/// C20：流式期 Enter 优先级（纯函数）。
///
/// 顺序即优先级：
/// 1. `input` 以 `\` 结尾 ⇒ [`StreamingEnterPre::Newline`]（走原「反斜杠 + Enter = 换行」分支）；
/// 2. 补全可见 + `/` 开头 + 不是完整命令 ⇒ [`StreamingEnterPre::Complete`]
///    （否则 `/he`+Enter 会把命令前缀当聊天消息排进队列）；
/// 3. 其余 ⇒ [`StreamingEnterPre::Submit`]（交给 C2/C3/C4 裁决）。
pub fn streaming_enter_pre_action(input: &str, completion_visible: bool) -> StreamingEnterPre {
    if input.ends_with('\\') {
        return StreamingEnterPre::Newline;
    }
    if completion_visible
        && input.trim_start().starts_with('/')
        && parse_command_input(input).is_none()
    {
        return StreamingEnterPre::Complete;
    }
    StreamingEnterPre::Submit
}

/// C22：队列反馈的语义分类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueFeedbackKind {
    /// C4：会话变更类命令流式期被拒绝。
    SessionMutating,
    /// C6：队列已满，本条未排队。
    QueueFull,
    /// C2：入队成功。
    Enqueued,
}

/// C22：反馈主通道。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueFeedbackChannel {
    /// 消息区（历史留痕）：静态期经 `messages.push(System)`；流式期改由 `queue_notice` 常驻渲染。
    SystemMessage,
    /// 状态行（`status_text`）。
    StatusText,
}

/// C22：反馈文案与通道的唯一真值（主循环不再手写文案）。
///
/// 通道口径：C4/C6 是「用户操作被拒绝」，必须走消息区（`SystemMessage`）；
/// C2 是「已受理」的短回执，走状态行（`StatusText`）。
pub fn queue_feedback(kind: QueueFeedbackKind) -> (QueueFeedbackChannel, String) {
    match kind {
        QueueFeedbackKind::SessionMutating => (
            QueueFeedbackChannel::SystemMessage,
            "本轮结束后可用 · 流式生成中不切换会话".to_string(),
        ),
        QueueFeedbackKind::QueueFull => (
            QueueFeedbackChannel::SystemMessage,
            format!(
                "排队已满（上限 {MAX_QUEUED_MESSAGES} 条）· 本条未排队，请本轮结束后再发送"
            ),
        ),
        QueueFeedbackKind::Enqueued => (
            QueueFeedbackChannel::StatusText,
            "已排队 · 本轮结束后自动发送".to_string(),
        ),
    }
}

/// C21：合成自动提交是否可以安全回灌（纯函数）。
///
/// 派发闸门用 `UiEvent::QueuedAutoSubmit` 而不是直接注入 `InputKey(Enter)`：
/// 收到该事件时若界面已进入模态/待作答态，回灌的 Enter 会被面板吃掉
/// （甚至误触发「同意审批」），故必须重新校验一次。
pub fn should_reinject_auto_submit(
    mode_is_modal: bool,
    has_pending_request: bool,
    has_ask: bool,
    has_permission: bool,
) -> bool {
    !mode_is_modal && !has_pending_request && !has_ask && !has_permission
}

/// C23：与 mode / `runtime_mode` **无关**的排队指示行（唯一渲染点真值）。
///
/// 为什么必须与运行态无关：`status_word` 在 `RuntimeMode::ExecutingTool(_)` 期整行不渲染，
/// 若指示器只挂在它上面，「流式 + 工具执行期」就一个渲染点都没有（L3-A 新 BLOCKING）。
/// 语义：`queued > 0` 或 `notice` 非空 ⇒ **必** `Some`（两个 bool 不参与判定）；
/// notice 与计数**同时存在时拼接**（notice 在前、计数在后，两者都是用户需要的信息）；
/// 两者都无 ⇒ `None`。
pub fn queue_indicator_line(
    queued: usize,
    notice: Option<&str>,
    is_streaming: bool,
    executing_tool: bool,
) -> Option<String> {
    // 两个运行态布尔只用于「语义自证」：本函数对它们**不做任何分支**，
    // 这正是契约要求的「任何运行态都必须有渲染点」。
    let _runtime_state = (is_streaming, executing_tool);
    let notice = notice.map(str::trim).filter(|text| !text.is_empty());
    let count_label = queued_status_label(queued);
    match (notice, count_label) {
        (Some(text), Some(label)) => Some(format!("{text} · {label}")),
        (Some(text), None) => Some(text.to_string()),
        (None, Some(label)) => Some(format!("{label} · 本轮结束后自动发送")),
        (None, None) => None,
    }
}

/// C24：单飞闩时间窗（毫秒）——「派发 → 合成 Enter 被消费」的最大允许时延。
///
/// 用时间窗而不是「任意 InputKey 清闩」：后者会给「已派发但 Enter 尚未被消费」开单飞缺口
/// （用户此刻按 Esc/Ctrl+U 会清掉闩与输入框 ⇒ 闸门再派发第二条 ⇒ 两条消息同时进提交路径）。
pub const QUEUE_DISPATCH_LATCH_MS: u64 = 2_000;

/// C24：单飞闩是否仍「在飞」（活性 = `Some(deadline) && now < deadline`）。
pub fn queue_dispatch_latch_active(deadline: Option<std::time::Instant>) -> bool {
    matches!(deadline, Some(deadline) if std::time::Instant::now() < deadline)
}

/// C24：把一条已取出/已写回的排队消息**回队到队首**（FIFO 复原），杜绝静默丢正文。
///
/// 使用场景：① `QueuedAutoSubmit` 校验不通过（界面已进入模态/待作答态）；
/// ② 闩窗口内用户按静态 Esc / Ctrl+U 显式撤销本次派发。
/// 长度不变量：调用点总是「刚取走 1 条」或「输入框里那 1 条」，故回队后必然 ≤ `MAX_QUEUED_MESSAGES`。
pub fn requeue_front(state: &mut TuiState, message: QueuedUserMessage) {
    if message.trim().is_empty() {
        return;
    }
    state.queued_user_messages.insert(0, message);
}

/// C26：派发快照与当前输入框是否**已分歧**（纯函数，A25 真值矩阵）。
///
/// 语义：`dispatched` 为 `None`（没有在飞派发）⇒ `false`；
/// 否则 `dispatched.trim() != composer.trim()` ⇒ `true`。
/// 用途：`QueuedAutoSubmit` 消费点据此判断「用户是否在窗口内改过输入框」——
/// 分歧时回队**原快照**（而不是此刻的输入框字节），否则 Ctrl+W/Backspace/Delete
/// 会把已派发正文擦掉且不回队 ⇒ 静默丢正文。
pub fn queued_dispatch_diverged(dispatched: Option<&str>, composer: &str) -> bool {
    match dispatched {
        None => false,
        Some(payload) => payload.trim() != composer.trim(),
    }
}

/// C27：**孤儿载荷收割**（纯函数，A27 真值矩阵）。
///
/// 场景：时间窗闩 2s 过期后闸门把「仍有 QAS 在飞」当成空闲，若此刻再派发就会覆盖单槽
/// `queue_dispatch_payload` ⇒ 前一条在 `remove(0)` 后既未提交也未回队（静默丢正文）。
/// 语义：闩**已过期**（`!latch_active`）但 payload 仍非空 ⇒ 它是孤儿，返回它（trim 后非空才返回；
/// 纯空白 ⇒ `None`）；闩仍活跃（QAS 真在飞）或没有 payload ⇒ `None`。
pub fn orphan_dispatch_payload(payload: Option<&str>, latch_active: bool) -> Option<String> {
    if latch_active {
        return None;
    }
    let payload = payload?.trim();
    if payload.is_empty() {
        return None;
    }
    Some(payload.to_string())
}

/// C25：派发日志的结构化字段（纯函数，A23 直接断言字段名与顺序）。
pub fn queue_dispatch_fields(
    before: usize,
    after: usize,
) -> Vec<(&'static str, serde_json::Value)> {
    vec![
        ("queue_depth_before", serde_json::json!(before)),
        ("queue_depth_after", serde_json::json!(after)),
    ]
}

/// C25：按键的**简化分类**（绝不把用户输入的正文字符写进日志）。
fn key_class_label(key: &crossterm::event::KeyEvent) -> String {
    match key.code {
        KeyCode::Char(_) => "Char".to_string(),
        other => format!("{other:?}"),
    }
}

/// 面板类斜杠命令（只读快照类；流式期允许打开，不排队、不拒绝）：
/// `/agents` `/tasks` `/plan` `/models` `/thinking` `/provider` `/help` `/update`。
pub fn is_panel_command(command: &str) -> bool {
    const PANEL_COMMANDS: &[&str] = &[
        "/agents", "/tasks", "/plan", "/models", "/thinking", "/provider", "/help", "/update",
    ];
    let trimmed = command.trim();
    PANEL_COMMANDS
        .iter()
        .any(|cmd| cmd.eq_ignore_ascii_case(trimmed))
}

/// 会话变更类斜杠命令（会拆掉在飞轮次；流式期拒绝，不得就地执行也不入队）：
/// `/clear` `/compact` `/resume`。`/quit` `/exit` 不属本类（走原退出分支）。
pub fn is_session_mutating_command(command: &str) -> bool {
    const MUTATING_COMMANDS: &[&str] = &["/clear", "/compact", "/resume"];
    let trimmed = command.trim();
    MUTATING_COMMANDS
        .iter()
        .any(|cmd| cmd.eq_ignore_ascii_case(trimmed))
}

/// C7：排队数量的可见文案（`count == 0` ⇒ `None`，不渲染）。
///
/// 渲染点：`status_word` 把它追加到流式活动行（流式期 `status_notice` 按契约 P-5 让位，
/// 故排队信息必须走这一条**流式期也可见**的通道）。
pub fn queued_status_label(count: usize) -> Option<String> {
    if count == 0 {
        None
    } else {
        Some(format!("排队 {count} 条"))
    }
}

/// C10：终态后是否该派发队首（单飞判据，五谓词全部显式入参以便机械断言）。
///
/// `turn_in_flight` = **轮次线程存活标志**（`TurnLoopInFlightGuard` 维护），不是「有没有
/// armed goal」：goal 自动续轮期间 `is_streaming` 不为真，只看 goal 是否存在会让
/// 「armed 但续轮没在跑」的会话排队永不自动派发（契约 C10 修订，2026-11-06）。
/// 只有「静态 + 界面空闲 + 队列非空 + 无在飞轮次线程 + 无在飞派发」才放行。
pub fn should_drain_queued(is_streaming: bool, ui_idle: bool, queued: usize, turn_in_flight: bool, dispatch_pending: bool) -> bool {
    !is_streaming && ui_idle && queued > 0 && !turn_in_flight && !dispatch_pending
}

impl TuiState {
    /// 只读访问器：当前排队消息条数（状态行渲染与测试断言共用）。
    pub fn queued_count(&self) -> usize {
        self.queued_user_messages.len()
    }
}

/// C17：界面是否真的「空闲」到可以派发排队消息（派发闸门的第 2 判据真值）。
///
/// 取景范围（诚实登记，本仓没有单一 `awaiting_answer` 布尔字段，故按现有状态组合）：
/// 1. `mode ∈ {Normal, PlanMode}`：PlanMode 是非阻塞执行模式开关（第三轮裁定，允许派发）；
///    其余模态面板（Selecting* / Ask / Permission / TaskBoard / AgentRoster / AddingProvider）
///    占用时一律不派发；
/// 2. `pending_request.is_none()`：没有待作答/待审批的宿主请求；
/// 3. `ask_user_state.is_none()`：Ask 面板没有未答完的问题；
/// 4. `permission_state.is_none()`：审批面板没有未决选项。
///
/// 为什么必须排除「挂起期」：`UiEvent::TurnSuspended` 会把 `is_streaming` 置 false，但该 run
/// **并未收口**（等用户回答）。此时派发会让新 turn 与挂起的 run 交叠写同一会话历史。
/// 上述 1–4 覆盖了 TUI 侧全部「等用户输入」的可达状态；若将来引入新的挂起通道，
/// 必须同步扩展本判据（这是本函数唯一的真实约束）。
fn is_ui_idle_for_dispatch(state: &TuiState) -> bool {
    // 第三轮裁定：`PlanMode` 是**非阻塞执行模式开关**（不限制工具执行、不占输入框），
    // 不是模态对话框 ⇒ 允许派发；其余非 Normal 模式一律排除。
    matches!(state.mode, InteractionMode::Normal | InteractionMode::PlanMode)
        && state.pending_request.is_none()
        && state.ask_user_state.is_none()
        && state.permission_state.is_none()
}

/// C21：该模式是否属于**模态对话框**（占用输入框、Enter 语义被面板改写）。
///
/// 与 [`is_ui_idle_for_dispatch`] 同源但用途不同：后者判「能否开始派发」，
/// 本函数判「已发出的合成 Enter 会不会被面板吃掉」。`AddingProvider` 虽然不在派发闸门的
/// 允许集合里，这里仍算模态（防御「派发途中弹面板」的窗口）。
fn is_modal_mode(mode: &InteractionMode) -> bool {
    matches!(
        mode,
        InteractionMode::SelectingSession
            | InteractionMode::SelectingProvider
            | InteractionMode::SelectingModel
            | InteractionMode::SelectingReasoningEffort
            | InteractionMode::AddingProvider
            | InteractionMode::PermissionApproval
            | InteractionMode::AskUserQuestion
            | InteractionMode::TaskBoard
            | InteractionMode::AgentRoster
    )
}

/// wave-40 R2（契约冻结空桩）：终端支持键盘增强时返回要 push 的标志。
pub fn keyboard_enhancement_flags(supported: bool) -> Option<KeyboardEnhancementFlags> {
    // kitty 协议的 DISAMBIGUATE_ESCAPE_CODES：把 Shift+Enter 与裸 Enter 区分开。
    if supported {
        Some(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
    } else {
        None
    }
}

/// Shift+Enter 换行依赖 kitty 键盘协议的 DISAMBIGUATE 标志。
///
/// 旧实现用 `supports_keyboard_enhancement()` 做阻塞式探测（发 `CSI ? u` 等最多 2s 回执）：
/// 在 tmux、VS Code 内置终端与部分 VTE 上会被误判为 `false`，标志从未 push，于是 Shift+Enter
/// 被终端降级成裸 Enter（＝发送）。新口径：只要不是 `TERM=dumb` 就无条件 push
/// （未知 CSI 会被终端静默忽略；push 失败只降级告警，不改变其他行为）。
pub fn keyboard_enhancement_should_push() -> bool {
    keyboard_enhancement_should_push_for(std::env::var("TERM").ok().as_deref())
}

/// [`keyboard_enhancement_should_push`] 的纯函数内核：只有 `TERM` 缺失或为 `dumb` 才放弃 push
/// （纯函数便于断言，避免测试里改全局环境变量）。
pub fn keyboard_enhancement_should_push_for(term: Option<&str>) -> bool {
    !matches!(
        term.unwrap_or_default().trim().to_ascii_lowercase().as_str(),
        "" | "dumb"
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeMode {
    Ready,
    Connecting,
    Thinking,
    Responding,
    ExecutingTool(String),
    WaitingApproval,
    Completed,
    Failed(String),
}

pub fn format_elapsed_seconds(ms: u64) -> String {
    let secs = ms / 1000;
    if secs < 60 {
        format!("{}秒", secs)
    } else {
        let mins = secs / 60;
        let rem_secs = secs % 60;
        if rem_secs > 0 {
            format!("{}分{}秒", mins, rem_secs)
        } else {
            format!("{}分钟", mins)
        }
    }
}

pub fn clean_tool_name_display(name: &str) -> String {
    let base = name.split('·').next().unwrap_or(name).trim();
    let cleaned = base
        .trim_start_matches('●')
        .trim_start_matches('✔')
        .trim();
    cleaned.to_string()
}

#[path = "tui/protocol.rs"]
pub mod protocol;
pub use protocol::*;

// ── 波次 43：宿主控制请求 → TUI 视图投影（真实运行时接线）─────────────────
//
// 这里的四个映射是整个「交互面板不再是死代码」的接缝：数据一律来自真实的
// 控制面快照（list_pending_asks）、todo 存储（todo_list）、goal 存储
// （active_goal_for_session / plan_list），而不是测试构造的假数据。

/// 一条待决宿主控制请求的 TUI 句柄：提交/拒绝时按 CAS `version` 回写控制面。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingUiRequest {
    pub request_id: String,
    pub version: u64,
    /// 审批/问答请求绑定的图运行 id；`None` 表示不经过图运行、只能消费不能 resume。
    pub run_id: Option<String>,
    /// `true` = 审批类（approval_required），`false` = 问答类（interaction）。
    pub is_approval: bool,
}

/// 真实 todo 存储 → 任务看板投影。`id` 按存储顺序稳定生成，避免每帧抖动。
/// W3：`/tasks` 已切到**真实 Team 任务看板**（`orchestration::team_task_list`）。
///
/// 本函数保留为 todo 投影能力（todo 模块与 `todo_write`/`todo_list` 工具均未删除，
/// 供需要按会话查看 todo 的调用方复用），因此在此显式标注 dead_code 而非删除。
#[allow(dead_code)]
pub fn tui_tasks_from_todos(todos: Vec<TodoItem>) -> Vec<TuiTask> {
    todos
        .into_iter()
        .enumerate()
        .map(|(index, todo)| TuiTask {
            id: todo
                .step_id
                .clone()
                .filter(|id| !id.trim().is_empty())
                .unwrap_or_else(|| format!("todo-{}", index + 1)),
            subject: todo.content,
            status: match todo.status {
                StoreTodoStatus::Pending => TuiTaskStatus::Pending,
                StoreTodoStatus::InProgress => TuiTaskStatus::InProgress,
                StoreTodoStatus::Completed => TuiTaskStatus::Completed,
            },
            owner: None,
            blocked_by: Vec::new(),
            write_scopes: Vec::new(),
        })
        .collect()
}

/// Team 状态字符串 → TUI 状态枚举（前后端兼容口径）。
///
/// 团队看板的 `status` 是 wire 字符串（`pending/in_progress/completed/blocked`），并兼容
/// 历史/别名写法（`open`/`todo`/`done`/`running`/`failed`…）。**未知值一律降级 `Pending`**，
/// 绝不把状态吞成成功。
fn tui_task_status_from_team_status(status: &str) -> TuiTaskStatus {
    match status.trim().to_ascii_lowercase().as_str() {
        "completed" | "complete" | "done" | "closed" | "resolved" => TuiTaskStatus::Completed,
        "in_progress" | "in-progress" | "inprogress" | "running" | "active" | "started" => {
            TuiTaskStatus::InProgress
        }
        "blocked" | "failed" | "cancelled" | "canceled" | "rejected" => TuiTaskStatus::Blocked,
        _ => TuiTaskStatus::Pending,
    }
}

/// 真实 Team 任务存储 → TUI 任务看板投影（owner / blocked_by / write_scopes 原样透传）。
pub fn tui_tasks_from_team_tasks(tasks: Vec<TeamTaskItem>) -> Vec<TuiTask> {
    tasks
        .into_iter()
        .map(|task| TuiTask {
            id: task.task_id,
            subject: task.subject,
            status: tui_task_status_from_team_status(&task.status),
            owner: task.owner,
            blocked_by: task.blocked_by,
            write_scopes: task.write_scopes,
        })
        .collect()
}

/// 真实 **Team 任务**存储 → 任务看板（模型/Agent 调用 `team_task_*` 后由这里同步）。
///
/// W3：数据源从会话级 todo 切换为**团队共享看板**（`team_task_list()`）；空列表保留 `None`，
/// 让面板走「当前没有任务」空状态而不是 0/0 看板（与既有行为一致）。
pub fn sync_tasks_from_store(state: &mut TuiState) {
    match team_task_list() {
        Ok(list) => {
            let tasks = tui_tasks_from_team_tasks(list.tasks);
            if tasks.is_empty() {
                state.task_board_state = None;
            } else {
                state.update_task_board(tasks);
            }
        }
        Err(error) => {
            state.status_text = format!("任务同步失败: {error}");
        }
    }
}

/// W3：名册状态 → 面板展示文案（wire 小写，与 `TeammateStatus` 的 serde 口径一致）。
fn teammate_status_label(status: &TeammateStatus) -> String {
    match status {
        TeammateStatus::Provisioning => "provisioning",
        TeammateStatus::Active => "active",
        TeammateStatus::Running => "running",
        TeammateStatus::Inactive => "inactive",
        TeammateStatus::Failed => "failed",
    }
    .to_string()
}

/// W3：会话 id 的展示尾号（面板宽度有限，只显示末 4 位便于对照日志）。
fn session_tail(session_id: &str) -> String {
    let chars: Vec<char> = session_id.chars().collect();
    let start = chars.len().saturating_sub(4);
    chars[start..].iter().collect()
}

/// W3：真实运行时 → `/agents` 花名册快照。
///
/// 数据源全部是真实存储（`list_agents` 名册 / 收件箱 / 驱动态 / 最近成功会话），
/// 不构造任何假成员；`lead` 固定排首位（其余按 target 字典序，保证面板顺序稳定）。
pub fn agent_roster_state_from_runtime() -> AgentRosterState {
    let mut entries: Vec<AgentRosterEntry> = match list_agents() {
        Ok(roster) => roster
            .agents
            .into_iter()
            .map(|member| AgentRosterEntry {
                pending_inbox: inbox_pending_len(&member.target),
                driving: is_driving(&member.target),
                session_id: teammate_session_id(&member.target),
                target: member.target,
                name: member.name,
                role: member.role,
                status: teammate_status_label(&member.status),
            })
            .collect(),
        Err(error) => {
            eprintln!("[tui] /agents 名册读取失败: {error}");
            Vec::new()
        }
    };
    entries.sort_by(|left, right| {
        (left.target != "lead")
            .cmp(&(right.target != "lead"))
            .then_with(|| left.target.cmp(&right.target))
    });
    AgentRosterState::new(entries)
}

/// 真实 goal 存储 → TUI goal 投影。phase 用现有字符串契约，未知值降级为 `Active`。
pub fn tui_goal_from_goal(goal: &GoalData) -> TuiGoal {
    TuiGoal {
        id: goal.id.clone(),
        objective: goal.objective.clone(),
        phase: match goal.phase.as_str() {
            "completed" => GoalPhase::Completed,
            "blocked" => GoalPhase::Blocked,
            "paused" => GoalPhase::Paused,
            _ => GoalPhase::Active,
        },
        activation: match goal.activation {
            AgentGoalActivation::Armed => GoalActivation::Armed,
            AgentGoalActivation::Disarmed => GoalActivation::Disarmed,
        },
        round_count: goal.rounds_started,
        max_rounds: Some(goal.max_goal_rounds),
        revision: goal.revision,
    }
}

/// `control_plane.plan_list(session)` 的 JSON 投影 → PlanState。空数组/格式不符返回 `None`。
pub fn tui_plan_from_value(value: &serde_json::Value) -> Option<PlanState> {
    let plan = value.as_array()?.first()?;
    let steps = plan
        .get("steps")
        .and_then(|steps| steps.as_array())
        .map(|steps| {
            steps
                .iter()
                .enumerate()
                .map(|(index, step)| PlanStep {
                    step_id: step
                        .get("stepId")
                        .and_then(|id| id.as_str())
                        .map(str::to_string)
                        .unwrap_or_else(|| format!("step-{}", index + 1)),
                    name: step
                        .get("name")
                        .and_then(|name| name.as_str())
                        .unwrap_or("未命名步骤")
                        .to_string(),
                    summary: step
                        .get("summary")
                        .and_then(|summary| summary.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    status: match step.get("status").and_then(|status| status.as_str()) {
                        Some("completed") => PlanStepStatus::Completed,
                        Some("in_progress") => PlanStepStatus::InProgress,
                        Some("failed") => PlanStepStatus::Failed,
                        _ => PlanStepStatus::Pending,
                    },
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Some(PlanState {
        plan_id: plan
            .get("planId")
            .and_then(|id| id.as_str())
            .unwrap_or("plan")
            .to_string(),
        revision: plan.get("revision").and_then(|rev| rev.as_u64()).unwrap_or(0),
        summary: plan
            .get("summary")
            .and_then(|summary| summary.as_str())
            .unwrap_or_default()
            .to_string(),
        steps,
        is_active_plan_mode: true,
        selected_step_index: 0,
        awaiting_approval: plan
            .get("lifecycle")
            .and_then(|lifecycle| lifecycle.as_str())
            .is_some_and(|lifecycle| lifecycle == "draft"),
    })
}

/// `list_pending_asks` 数组项 → TUI 句柄。缺 `requestId`/`version` 视为不可消费，返回 `None`。
pub fn pending_ui_request_from_value(value: &serde_json::Value) -> Option<PendingUiRequest> {
    let request_id = value.get("requestId")?.as_str()?.to_string();
    let version = value.get("version")?.as_u64()?;
    let kind = value
        .get("requestKind")
        .and_then(|kind| kind.as_str())
        .unwrap_or("interaction");
    let is_approval = kind == "approval" || kind == "approval_required";
    Some(PendingUiRequest {
        request_id,
        version,
        run_id: value
            .get("runId")
            .and_then(|run_id| run_id.as_str())
            .map(str::to_string),
        is_approval,
    })
}

/// 待决请求 → 问答对话框数据。`options` 若已是 `{label,description}` 数组则直接采用。
pub fn ask_question_from_pending(value: &serde_json::Value) -> AskUserQuestion {
    let question = value
        .get("prompt")
        .and_then(|prompt| prompt.as_str())
        .unwrap_or("（宿主请求缺少提问内容）")
        .to_string();
    let options = value
        .get("options")
        .and_then(|options| options.as_array())
        .map(|options| {
            options
                .iter()
                // 生产 schema 的 `ask_user.options` 是 **string 数组**（见 tools.rs 的 input_schema），
                // 而历史实现只认 `{label, description}` 对象 ⇒ 真实模型给的选项会被整批丢掉，
                // 面板只剩「自定义输入」。这里两种形状都接受：字符串 ⇒ 纯标签，对象 ⇒ 标签+说明。
                .filter_map(|option| match option {
                    serde_json::Value::String(label) => {
                        (!label.trim().is_empty()).then(|| QuestionOption {
                            label: label.clone(),
                            description: None,
                        })
                    }
                    other => other
                        .get("label")
                        .and_then(|label| label.as_str())
                        .map(|label| QuestionOption {
                            label: label.to_string(),
                            description: other
                                .get("description")
                                .and_then(|desc| desc.as_str())
                                .map(str::to_string),
                        }),
                })
                .collect::<Vec<_>>()
        })
        // 契约 F-M①：无 options 的提问**不伪造**「确认/取消」两项。
        // 伪造会让一次 Enter 就回一个用户从没选过的"确认"，是真实缺陷；空列表 ⇒ 走自由输入。
        .unwrap_or_default();
    AskUserQuestion {
        id: "q1".to_string(),
        header: value
            .get("descriptorId")
            .and_then(|id| id.as_str())
            .map(str::to_string),
        question,
        multi_select: false,
        options,
    }
}

/// 待决审批请求 → 权限对话框数据。
pub fn permission_request_from_pending(
    value: &serde_json::Value,
    session_id: &str,
) -> PermissionRequest {
    let target_path = value
        .get("options")
        .and_then(|opts| opts.get("path").or_else(|| opts.get("file_path")))
        .and_then(|p| p.as_str())
        .map(str::to_string)
        .or_else(|| {
            value
                .get("callId")
                .and_then(|id| id.as_str())
                .map(str::to_string)
        });
    PermissionRequest {
        request_id: value
            .get("requestId")
            .and_then(|id| id.as_str())
            .unwrap_or_default()
            .to_string(),
        session_id: value
            .get("sessionId")
            .and_then(|id| id.as_str())
            .unwrap_or(session_id)
            .to_string(),
        tool_name: value
            .get("descriptorId")
            .and_then(|id| id.as_str())
            .unwrap_or("tool")
            .to_string(),
        description: value
            .get("prompt")
            .and_then(|prompt| prompt.as_str())
            .unwrap_or("（宿主请求缺少说明）")
            .to_string(),
        command_or_path: target_path,
        timestamp_ms: value
            .get("expiresAtMs")
            .and_then(|ms| ms.as_u64())
            .unwrap_or(0),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InteractionMode {
    Normal,
    SelectingSession,
    SelectingProvider,
    SelectingModel,
    SelectingReasoningEffort,
    AddingProvider,
    PermissionApproval,
    AskUserQuestion,
    PlanMode,
    TaskBoard,
    /// W3：`/agents` Team 角色与状态花名册（↑↓ 选择 · Enter 进入队友子会话 · r 返回主会话 · Esc 关闭）。
    AgentRoster,
}

pub const AVAILABLE_COMMANDS: &[(&str, &str)] = &[
    ("/resume", "选择并恢复历史会话"),
    ("/provider", "管理与切换模型提供商"),
    ("/models", "切换当前活跃模型"),
    ("/thinking", "设置思考强度 (Reasoning Effort)"),
    ("/plan", "切换/查看 Plan 面板（不限制工具执行）"),
    ("/tasks", "切换/查看 Team 任务看板（Ctrl+T）"),
    ("/agents", "查看 Team 角色与状态"),
    ("/clear", "清空当前消息上下文"),
    ("/compact", "压缩当前会话上下文"),
    ("/help", "查看所有支持的快捷指令"),
    ("/update", "检查更新与升级新版本"),
    ("/quit", "安全退出 TUI 界面"),
];

/// 契约 C-5：斜杠命令解析口径（纯函数，可单测）。
///
/// 取 `trim()` 后**首个空白分隔 token** 与命令表做**大小写不敏感**比较；命中返回命令表中的
/// 规范写法（如 `"/plan"`、`"/compact"`）。未命中返回 `None`——调用方按普通消息发送
/// （`/etc/hosts 是什么` 之类不新增拦截）。
pub fn match_command_token(input: &str) -> Option<&'static str> {
    let token = input.trim().split_whitespace().next()?;
    AVAILABLE_COMMANDS
        .iter()
        .map(|(cmd, _)| *cmd)
        .find(|cmd| cmd.eq_ignore_ascii_case(token))
}

/// 契约 F-B：**分发口径**的斜杠命令解析（整串匹配）。
///
/// 只有整串（`trim()` 后、大小写不敏感）恰好等于命令表中的命令才返回规范写法；
/// **首 token 命中但存在额外 token ⇒ `None`**，调用方把整串当普通消息发送给模型，
/// 绝不吞掉正文（`/clear 这个需求怎么拆` 不得清空上下文）。
/// 菜单补全/高亮请用 [`match_command_token`]（首 token 口径）。
pub fn parse_command_input(input: &str) -> Option<&'static str> {
    let trimmed = input.trim();
    if trimmed.split_whitespace().nth(1).is_some() {
        return None;
    }
    match_command_token(trimmed)
}

/// 整串（`trim()` 后）与历史别名大小写不敏感相等（别名不在命令表内，如 `/exit`、`/effort`）。
fn input_is_alias(input: &str, alias: &str) -> bool {
    input.trim().eq_ignore_ascii_case(alias)
}

/// `/update` 检测结果数据
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateCheckOutcome {
    pub current_version: String,
    pub latest_version: String,
    pub has_update: bool,
    pub release_notes: Option<String>,
    pub release_url: Option<String>,
}

/// 解析语义化三元组版本号 `vX.Y.Z` 或 `X.Y.Z`
pub fn parse_semver_triple(v: &str) -> Option<(u64, u64, u64)> {
    let clean = v.trim().trim_start_matches(|c| c == 'v' || c == 'V');
    let mut parts = clean.split('.');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = parts.next()?.parse::<u64>().ok()?;
    let patch = parts.next()?.parse::<u64>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// 比较候选版本是否严格高于当前版本
pub fn is_newer_semver(candidate: &str, current: &str) -> bool {
    match (parse_semver_triple(candidate), parse_semver_triple(current)) {
        (Some(c), Some(cur)) => c > cur,
        _ => false,
    }
}

/// 执行远端发布检查（支持 GitHub Releases API，失败时回退 MinIO/S3）
pub async fn perform_update_check() -> Result<UpdateCheckOutcome, String> {
    let current_version = env!("CARGO_PKG_VERSION").to_string();
    let client = reqwest::Client::builder()
        .user_agent("pony-tui")
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| format!("HTTP 客户端初始化失败: {e}"))?;

    // 1. 优先尝试 GitHub Releases API
    let gh_url = "https://api.github.com/repos/lanhui100/pony-agent/releases/latest";
    let gh_res = client.get(gh_url).send().await;

    let mut latest_version_opt = None;
    let mut notes_opt = None;
    let mut release_url_opt = None;

    if let Ok(resp) = gh_res {
        if resp.status().is_success() {
            if let Ok(json) = resp.json::<serde_json::Value>().await {
                if let Some(tag) = json.get("tag_name").and_then(|v| v.as_str()) {
                    let clean_tag = tag.trim().trim_start_matches(|c| c == 'v' || c == 'V').to_string();
                    latest_version_opt = Some(clean_tag);
                    notes_opt = json.get("body").and_then(|v| v.as_str()).map(|s| s.to_string());
                    release_url_opt = json.get("html_url").and_then(|v| v.as_str()).map(|s| s.to_string());
                }
            }
        }
    }

    // 2. 若 GitHub 不可用/被限流，尝试 S3 fallback
    if latest_version_opt.is_none() {
        let fallback_url = "https://s3.local.ponyjob.top/pony-agent-releases/latest.json";
        if let Ok(resp) = client.get(fallback_url).send().await {
            if resp.status().is_success() {
                if let Ok(json) = resp.json::<serde_json::Value>().await {
                    if let Some(tag) = json.get("tag_name").and_then(|v| v.as_str()) {
                        let clean_tag = tag.trim().trim_start_matches(|c| c == 'v' || c == 'V').to_string();
                        latest_version_opt = Some(clean_tag);
                    }
                }
            }
        }
    }

    let latest_version = match latest_version_opt {
        Some(v) => v,
        None => return Err("无法获取最新版本信息，请检查网络连接。".to_string()),
    };

    let has_update = is_newer_semver(&latest_version, &current_version);
    if release_url_opt.is_none() {
        release_url_opt = Some(format!("https://github.com/lanhui100/pony-agent/releases/tag/v{}", latest_version));
    }

    Ok(UpdateCheckOutcome {
        current_version,
        latest_version,
        has_update,
        release_notes: notes_opt,
        release_url: release_url_opt,
    })
}

/// 格式化面向用户的 /update 检查与升级反馈文案
pub fn format_update_feedback(outcome: &UpdateCheckOutcome) -> String {
    if outcome.has_update {
        let mut msg = format!(
            "✨ 发现新版本：v{}（当前版本：v{}）\n",
            outcome.latest_version, outcome.current_version
        );
        if let Some(notes) = &outcome.release_notes {
            let trimmed = notes.trim();
            if !trimmed.is_empty() {
                msg.push_str(&format!("\n📝 更新说明：\n{}\n", trimmed));
            }
        }
        if let Some(url) = &outcome.release_url {
            msg.push_str(&format!("\n🔗 发布页面：{}", url));
        }
        msg.push_str("\n\n💡 升级指引：\n- 源码开发环境：git pull && npm run cli:build\n- 桌面端：在设置中点击「检查更新」并一键升级");
        msg
    } else {
        format!(
            "当前已是最新版本（v{}），无需升级。",
            outcome.current_version
        )
    }
}

/// 契约 F-C：粘贴文本归一化——CRLF / CR 统一为 LF，其余字符原样保留。
///
/// 终端 pasting 会把换行按平台习惯送上（Windows CRLF / 老终端 CR）；直接在缓冲里插入
/// `\r` 会让后续按行渲染与提交判定出现幽灵空行，故在入口一次性归一。
pub fn normalize_paste(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// 契约 F-D：终端恢复的 RAII 守卫。
///
/// 为什么需要它：`enable_raw_mode()` 之后的任何失败/panic（`execute!` 失败、`Terminal::new()`
/// 失败、`build_control_plane` panic、`?` 提前返回）都会把用户终端留在 raw + 备用屏状态。
/// 守卫在 raw mode 生效后**立刻**建立，作用域覆盖整个运行期：正常退出走显式 `restore()`，
/// 异常路径由 `Drop` 兜底。
pub struct TerminalGuard {
    restore: Option<Box<dyn FnMut()>>,
}

impl TerminalGuard {
    /// 接受任意 `FnMut() + 'static` 恢复闭包。
    ///
    /// 用泛型参数而非 `Box<dyn FnMut()>`：调用点既能直接传闭包
    /// （`TerminalGuard::new(move || …)`），也能传已装箱的 `Box<dyn FnMut()>`，
    /// 内部统一装箱存储，调用方无需关心间接层。
    pub fn new(restore: impl FnMut() + 'static) -> Self {
        Self {
            restore: Some(Box::new(restore)),
        }
    }

    /// 执行终端恢复。**幂等**：闭包只会被执行一次，重复调用静默 no-op。
    ///
    /// 恢复闭包**永不把 panic 传播出去**：在 `Drop`（尤其是 unwind 期间）再 panic 会直接
    /// abort 进程并把终端留在 raw 态，这比丢一条诊断更糟。故这里捕获并写 stderr
    /// （stderr 已被 dup2 到 `~/.pony-agent/tui-logs/tui-runtime.log`）。
    pub fn restore(&mut self) {
        if let Some(mut restore) = self.restore.take() {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| restore()));
            if outcome.is_err() {
                eprintln!(
                    "[pony-tui] terminal restore closure panicked; swallowed to keep Drop panic-free"
                );
            }
        }
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        self.restore();
    }
}

/// 契约 F-E：输入泵轮询结果的处置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputPumpAction {
    Continue,
    Stop,
}

/// 契约 F-E：`event::poll` 结果的确定性分流。
///
/// - `Ok(_)`（含 `Ok(false)` = 本轮无事件）：继续泵；
/// - `Err(_)`：输入源已不可用（stdin 关闭 / 非 TTY / EINTR 等），必须退出线程——
///   旧实现 `unwrap_or(false)` 会把它当成"无事件"，于是 50ms 一轮永久热旋，且用户毫无提示。
pub fn classify_input_poll(result: std::io::Result<bool>) -> InputPumpAction {
    match result {
        Ok(_) => InputPumpAction::Continue,
        Err(_) => InputPumpAction::Stop,
    }
}

/// 契约 F-G：goal 自动续轮的取消令牌（Esc 置位、新一轮用户消息重置）。
///
/// 用 `Arc<AtomicBool>` 而非 channel：Esc 在 UI 线程置位、续轮循环在 turn 线程读取，
/// 语义只要求"下一轮开始前看到"，不需要排队。
#[derive(Clone, Debug, Default)]
pub struct GoalRoundCancelToken {
    cancelled: Arc<AtomicBool>,
}

impl GoalRoundCancelToken {
    pub fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    /// 请求取消：下一次轮次边界生效（已在跑的那一轮由 `stop_turn` 中止）。
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    /// 重置（新一轮用户消息提交时调用）。
    pub fn reset(&self) {
        self.cancelled.store(false, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }
}

/// 契约 F-G：goal 自动续轮的轮次边界判定（纯函数，可单测）。
///
/// `cancelled`（用户已按 Esc）或已用满预算（`consecutive >= max`）都必须中止下一轮。
pub fn should_abort_goal_rounds(cancelled: bool, consecutive: u32, max: u32) -> bool {
    cancelled || consecutive >= max
}

/// 契约 P-1：解析「当前生效 provider id」的优先级（纯函数，可单测）。
///
/// 顺序：`state_id` 命中注册表 → 注册表选中项命中 → 注册表首个 provider → `None`。
/// 为什么 state 优先：`/provider` 切换是用户显式动作，内存里的 `current_provider_id` 若不优先，
/// 界面报"已切换"而发送仍走注册表旧值时就是**假成功**（本波 BLOCKING 项）。
pub fn pick_active_provider_id(
    state_id: Option<&str>,
    registry_selected: Option<&str>,
    provider_ids: &[&str],
) -> Option<String> {
    if let Some(id) = state_id {
        if provider_ids.contains(&id) {
            return Some(id.to_string());
        }
    }
    if let Some(id) = registry_selected {
        if provider_ids.contains(&id) {
            return Some(id.to_string());
        }
    }
    provider_ids.first().map(|id| (*id).to_string())
}

/// 契约 P-4：Ctrl+C 的三态语义。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CtrlCAction {
    InterruptTurn,
    ClearDraft,
    Exit,
}

/// 契约 P-4：Ctrl+C 行为判定（纯函数）。
///
/// 流式中 ⇒ 中断本轮（等价 Esc），**不退出**；否则有草稿 ⇒ 先清草稿（防误触丢整段输入）；
/// 否则才退出。旧实现把「流式中 Ctrl+C」直接映射为 `break`，等于一次误触就杀掉整个 TUI。
pub fn ctrl_c_action(is_streaming: bool, has_draft: bool) -> CtrlCAction {
    if is_streaming {
        CtrlCAction::InterruptTurn
    } else if has_draft {
        CtrlCAction::ClearDraft
    } else {
        CtrlCAction::Exit
    }
}

/// 契约 P-5：一次性状态提示的存活时长（毫秒）。
pub const STATUS_NOTICE_TTL_MS: u64 = 4_000;

/// C23：队列反馈（`queue_notice`）在指标行的存活时长（毫秒）。
///
/// 到期后指示行回落到「排队 N 条 · 本轮结束后自动发送」，避免短时反馈长期顶替队列计数。
pub const QUEUE_NOTICE_TTL_MS: u64 = 5_000;

/// wave-63 §T3：stopping 期间按键的状态提示文案。
///
/// 必须走 `status_text` / `status_word` 通道（流式期唯一可见的状态位），
/// **不得**改用 `status_notice`（流式期被 `should_render_status_notice` 抑制，等于静默吞）。
pub const STOPPING_KEY_HINT: &str = "已请求停止，等待安全收口…";

/// wave-60 §C8：当前墙钟毫秒（供 `StopSequenceState::press` 注入；纯读数，无副作用）。
pub fn now_epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

/// wave-60 §C8：生成本次按键的停止身份，并维护「generation 代次」。
///
/// 口径：`turn_id` 变化（新一轮 / 轮次收口）⇒ generation 自增 ⇒ 旧上膛失效；
/// 同一 turn_id 的连续按键保持同一 generation，双击判定才可能命中。
pub fn next_stop_identity(
    generation: &mut u64,
    current_turn: &mut Option<String>,
    session_id: &str,
    turn_id: Option<&str>,
) -> StopIdentity {
    let turn = turn_id.unwrap_or("").to_string();
    if current_turn.as_deref() != Some(turn.as_str()) {
        *generation = (*generation).wrapping_add(1);
        *current_turn = Some(turn.clone());
    }
    StopIdentity {
        session_id: Some(session_id.to_string()),
        turn_id: turn,
        generation: *generation,
    }
}

/// 契约 P-5：状态提示是否应当渲染（纯函数）。
///
/// 流式期让位给进度短语（思考中/生成中/执行中），非流式期在 TTL 内显示一次性提示。
pub fn should_render_status_notice(is_streaming: bool, age_ms: u64, ttl_ms: u64) -> bool {
    !is_streaming && age_ms < ttl_ms
}

/// 契约 F-N：计划模式横幅（诚实措辞）。
///
/// `/plan` 只是界面提示，**不限制工具执行**；旧文案容易让人误以为进入只读模式。
pub fn plan_mode_banner(active: bool) -> Option<&'static str> {
    if active {
        Some("计划模式 · 仅界面提示，不限制工具执行")
    } else {
        None
    }
}

/// 契约 F-M：Ask 提交判定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AskSubmitVerdict {
    Submit,
    Reject(&'static str),
}

/// 契约 F-M：Ask 提交判定（纯函数）。
///
/// 自由输入非空白 ⇒ 直接可提交（无选项的问题只有这一条路）；
/// 否则必须有选项且已选中一项才可提交 —— 任何「空白提交」都必须被拒绝并给出可读理由，
/// 而不是靠伪造的「确认/取消」两项让一次 Enter 蒙混过关。
pub fn ask_submit_verdict(
    has_options: bool,
    selected: Option<usize>,
    typed: &str,
) -> AskSubmitVerdict {
    if !typed.trim().is_empty() {
        return AskSubmitVerdict::Submit;
    }
    if !has_options {
        return AskSubmitVerdict::Reject("请输入回答后再提交（该问题没有可选项）");
    }
    match selected {
        Some(_) => AskSubmitVerdict::Submit,
        None => AskSubmitVerdict::Reject("请选择一个选项，或输入自定义回答后再提交"),
    }
}

/// 契约 F-L：网关地址的用户可见展示（**必须剥离凭据**）。
///
/// 只保留 `scheme://host[:port]/path`：userinfo（`user:pass@`）、query（`?key=sk-…`）、
/// fragment 全部丢弃，避免密钥/账号出现在屏幕与截图证据里。无 scheme 时走保守清洗分支
/// （剥 `user[:pass]@` 与 `?`/`#` 之后的部分），空串返回空串，任何输入都不 panic。
/// 超过 48 字符按**字符**省略（尾部 `…`，总长 ≤ 48）。
pub fn gateway_display(url: &str) -> String {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let sanitized = match url::Url::parse(trimmed) {
        Ok(parsed) => {
            let host = parsed.host_str().unwrap_or("");
            if host.is_empty() {
                // 非分层 URL（如 `mailto:`）：无可展示的 host，直接不给。
                return String::new();
            }
            let mut out = format!("{}://{}", parsed.scheme(), host);
            if let Some(port) = parsed.port() {
                out.push(':');
                out.push_str(&port.to_string());
            }
            let path = parsed.path();
            if !path.is_empty() && path != "/" {
                out.push_str(path);
            }
            out
        }
        Err(_) => {
            // 无 scheme / 不可解析：保守清洗 —— 丢弃 query 与 fragment，再剥凭据前缀。
            let without_query = trimmed
                .split(['?', '#'])
                .next()
                .unwrap_or("")
                .trim();
            match without_query.rsplit_once('@') {
                Some((_, host_and_path)) => host_and_path.to_string(),
                None => without_query.to_string(),
            }
        }
    };
    gateway_display_ellipsize(&sanitized, 48)
}

/// 按**字符**省略：总长 ≤ `max`（含尾部 `…`）。
fn gateway_display_ellipsize(text: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max - 1).collect();
    out.push('…');
    out
}

pub const REASONING_EFFORT_OPTIONS: &[(Option<ProviderReasoningEffort>, &str, &str)] = &[
    (None, "默认 (Default)", "遵循模型默认"),
    (Some(ProviderReasoningEffort::Low), "低 · low", "轻量思考，快速响应"),
    (Some(ProviderReasoningEffort::Medium), "中 · medium", "平衡思考深度与耗时"),
    (Some(ProviderReasoningEffort::High), "高 · high", "深度思考，适合复杂逻辑"),
    (Some(ProviderReasoningEffort::Max), "极高 · max", "最大推理深度"),
];

#[derive(Debug, Clone)]
pub struct SessionItem {
    pub id: String,
    pub title: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderItem {
    pub id: String,
    pub name: String,
    pub protocol: String,
    pub base_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelItem {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddingProviderStep {
    Name,
    BaseUrl,
    ApiKey,
    Commit,
}

#[derive(Debug, Clone, Default)]
pub struct AddProviderForm {
    pub name: String,
    pub base_url: String,
    pub api_key: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AddProviderPayload {
    pub id: String,
    pub name: String,
    pub protocol: String,
    pub base_url: String,
    pub api_key: Option<String>,
}

impl AddProviderPayload {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.id.trim().is_empty() {
            return Err("Provider ID 不能为空");
        }
        if self.name.trim().is_empty() {
            return Err("Provider 名称不能为空");
        }
        let url = self.base_url.trim();
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err("Base URL 必须是以 http:// 或 https:// 开头的合法地址");
        }
        match self.protocol.trim().to_lowercase().as_str() {
            "openai" | "anthropic" | "gemini" | "ollama" => Ok(()),
            _ => Err("不支持的协议类型"),
        }
    }
}

// ── 鼠标选择与复制契约模型 ───────────────────────────────────────────────────

/// 文本字符坐标（行、字符索引）
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TextCoord {
    pub line: usize,
    pub col: usize,
}

/// 鼠标拖拽选择状态机
#[derive(Debug, Clone, Default)]
pub struct MouseSelectionState {
    pub active: bool,
    pub anchor: Option<TextCoord>,
    pub head: Option<TextCoord>,
    pub debounce_until: Option<std::time::Instant>,
    pub copied_range: Option<(TextCoord, TextCoord)>,
}

impl MouseSelectionState {
    pub fn clear(&mut self) {
        self.active = false;
        self.anchor = None;
        self.head = None;
        self.debounce_until = None;
        self.copied_range = None;
    }

    pub fn normalized_range(&self) -> Option<(TextCoord, TextCoord)> {
        match (self.anchor, self.head) {
            (Some(a), Some(h)) => {
                if a <= h {
                    Some((a, h))
                } else {
                    Some((h, a))
                }
            }
            _ => None,
        }
    }
}

/// 鼠标命中 → 选区坐标（`None` = 不在任何可选择区域内）。
///
/// 坐标空间统一：正文用 0 起始的绝对渲染行号；输入框用 `INPUT_LINE_BASE + 逻辑行号`。
pub fn selection_point_at(state: &TuiState, x: u16, y: u16) -> Option<TextCoord> {
    if let Some(rect) = state.last_input_rect {
        if x >= rect.x
            && x < rect.x.saturating_add(rect.width)
            && y >= rect.y
            && y < rect.y.saturating_add(rect.height)
        {
            let line = INPUT_LINE_BASE + state.last_input_view_start + (y - rect.y) as usize;
            return Some(TextCoord {
                line,
                col: snap_hit_col(state, line, (x - rect.x) as usize),
            });
        }
    }
    let chat_rect = state.last_chat_rect?;
    if x >= chat_rect.x
        && x < chat_rect.x.saturating_add(chat_rect.width)
        && y >= chat_rect.y
        && y < chat_rect.y.saturating_add(chat_rect.height)
    {
        let line = (y - chat_rect.y) as usize + state.effective_scroll_offset as usize;
        return Some(TextCoord {
            line,
            col: snap_hit_col(state, line, (x - chat_rect.x) as usize),
        });
    }
    None
}

/// 拖拽时的命中：越出正文矩形后吸附到可视区首/末行与左右边界，保证连续扩选不中断。
pub fn selection_drag_point(state: &TuiState, x: u16, y: u16) -> Option<TextCoord> {
    if let Some(coord) = selection_point_at(state, x, y) {
        return Some(coord);
    }
    let chat_rect = state.last_chat_rect?;
    if chat_rect.width == 0 || chat_rect.height == 0 {
        return None;
    }
    let x_clamped = x
        .max(chat_rect.x)
        .min(chat_rect.x.saturating_add(chat_rect.width).saturating_sub(1));
    let last_row = chat_rect.height.saturating_sub(1) as usize;
    let row = (y as usize)
        .saturating_sub(chat_rect.y as usize)
        .min(last_row);
    let line = row + state.effective_scroll_offset as usize;
    Some(TextCoord {
        line,
        col: snap_hit_col(state, line, (x_clamped - chat_rect.x) as usize),
    })
}

/// 跨区域（正文 ↔ 输入框）选区提取：按坐标行号拆分两段，避免构造百万级空行占位。
pub fn extract_selection_across(
    chat_lines: &[String],
    input_lines: &[String],
    anchor: TextCoord,
    head: TextCoord,
) -> String {
    let (start, end) = if anchor <= head {
        (anchor, head)
    } else {
        (head, anchor)
    };
    if start.line >= INPUT_LINE_BASE {
        let from = TextCoord {
            line: start.line - INPUT_LINE_BASE,
            col: start.col,
        };
        let to = TextCoord {
            line: end.line - INPUT_LINE_BASE,
            col: end.col,
        };
        return extract_selected_by_display_cols(input_lines, from, to);
    }
    if end.line < INPUT_LINE_BASE {
        return extract_selected_by_display_cols(chat_lines, start, end);
    }
    let chat_tail = extract_selected_by_display_cols(
        chat_lines,
        start,
        TextCoord {
            line: chat_lines.len().saturating_sub(1),
            col: usize::MAX,
        },
    );
    let input_part = extract_selected_by_display_cols(
        input_lines,
        TextCoord { line: 0, col: 0 },
        TextCoord {
            line: end.line - INPUT_LINE_BASE,
            col: end.col,
        },
    );
    match (chat_tail.is_empty(), input_part.is_empty()) {
        (true, _) => input_part,
        (false, true) => chat_tail,
        (false, false) => format!("{chat_tail}\n{input_part}"),
    }
}

/// 将文本编码为 OSC 52 终端剪贴板序列
pub fn encode_osc52_copy(text: &str) -> String {
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
    format!("\x1b]52;c;{}\x07", b64)
}

/// 判断给定坐标是否落在由 anchor 和 head 围成的选区内
pub fn is_coord_selected(coord: TextCoord, anchor: TextCoord, head: TextCoord) -> bool {
    let (start, end) = if anchor <= head {
        (anchor, head)
    } else {
        (head, anchor)
    };
    if coord.line < start.line || coord.line > end.line {
        return false;
    }
    if start.line == end.line {
        coord.col >= start.col && coord.col <= end.col
    } else if coord.line == start.line {
        coord.col >= start.col
    } else if coord.line == end.line {
        coord.col <= end.col
    } else {
        true
    }
}

/// 从多行纯文本切片中截取选区内的文本内容
pub fn extract_selected_text(lines: &[String], anchor: TextCoord, head: TextCoord) -> String {
    let (start, end) = if anchor <= head {
        (anchor, head)
    } else {
        (head, anchor)
    };
    if lines.is_empty() || start.line >= lines.len() {
        return String::new();
    }
    let end_line = end.line.min(lines.len() - 1);
    if start.line == end_line {
        let line = &lines[start.line];
        let chars: Vec<char> = line.chars().collect();
        if chars.is_empty() || start.col >= chars.len() {
            return String::new();
        }
        let end_col = end.col.min(chars.len().saturating_sub(1));
        if start.col > end_col {
            return String::new();
        }
        chars[start.col..=end_col].iter().collect()
    } else {
        let mut result = Vec::new();
        for line_idx in start.line..=end_line {
            let line = &lines[line_idx];
            let chars: Vec<char> = line.chars().collect();
            if line_idx == start.line {
                if !chars.is_empty() && start.col < chars.len() {
                    result.push(chars[start.col..].iter().collect::<String>());
                } else {
                    result.push(String::new());
                }
            } else if line_idx == end_line {
                if !chars.is_empty() {
                    let end_col = end.col.min(chars.len().saturating_sub(1));
                    result.push(chars[..=end_col].iter().collect::<String>());
                } else {
                    result.push(String::new());
                }
            } else {
                result.push(line.clone());
            }
        }
        result.join("\n")
    }
}

/// 显示列语义的精确选区提取（鼠标选区的真实路径）。
///
/// - `anchor` / `head` 的 `col` 是**终端显示列**（宽字符占 2 列），不是 char 索引；
/// - 端点语义为「包含该列所在的字符」，拖到宽字符右半格也整字选中；
/// - 每行右侧的底色填充空格是渲染产物，复制时裁掉，避免粘贴出尾随空白。
pub fn extract_selected_by_display_cols(
    lines: &[String],
    anchor: TextCoord,
    head: TextCoord,
) -> String {
    let (start, end) = if anchor <= head {
        (anchor, head)
    } else {
        (head, anchor)
    };
    if lines.is_empty() || start.line >= lines.len() {
        return String::new();
    }
    let end_line = end.line.min(lines.len() - 1);
    let mut out: Vec<String> = Vec::new();
    for line_idx in start.line..=end_line {
        let line = &lines[line_idx];
        let col_from = if line_idx == start.line { start.col } else { 0 };
        let col_to = if line_idx == end.line {
            end.col
        } else {
            usize::MAX
        };
        let mut buf = String::new();
        let mut display_col = 0usize;
        for ch in line.chars() {
            let width = char_width(ch);
            let ch_start = display_col;
            let ch_end = display_col + width - 1;
            display_col += width;
            if ch_end >= col_from && ch_start <= col_to {
                buf.push(ch);
            } else if !buf.is_empty() {
                break;
            }
        }
        out.push(buf.trim_end().to_string());
    }
    out.join("\n")
}

/// 契约（修订11 步骤2）：布局缓存的逐帧统计。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RenderCacheStats {
    /// 本帧重新布局的消息条数（全量重建时 = 消息数；复用帧 = 1）。
    pub rebuilt_messages: usize,
    /// 本帧直接复用缓存的消息条数。
    pub reused_messages: usize,
    /// 本帧实际布局出的行数（重建块的行和 / 复用帧仅末条的行数）。
    pub laid_out_lines: usize,
    /// 缓存里当前的总行数。
    pub total_lines: usize,
}

/// 契约（修订11 步骤2）：按消息粒度的布局缓存。
///
/// 失效条件：终端宽度变化、消息条数变化、显式 `invalidate_render_cache()`
/// （`/clear`、`/resume`、`reset_session_scope`、`Event::Resize`）。
/// 其余每帧只重算**最后一条** —— 流式追加、工具动效帧、「最后一条 = 执行中」都只影响它。
#[derive(Debug, Clone)]
pub struct RenderCache {
    cache_width: u16,
    cache_msg_count: usize,
    cache_lines: Vec<Vec<Line<'static>>>,
    stats: RenderCacheStats,
}

impl Default for RenderCache {
    fn default() -> Self {
        Self {
            cache_width: 0,
            cache_msg_count: usize::MAX,
            cache_lines: Vec::new(),
            stats: RenderCacheStats::default(),
        }
    }
}

/// 初始化页品牌标（终端图形协议路径）：仅当终端支持 sixel / kitty / iTerm2 时启用。
/// `protocol` 由 ratatui-image 维护编码状态，窗口尺寸变化时按目标矩形重编码。
pub struct SplashImage {
    picker: Picker,
    protocol: StatefulProtocol,
    cols: u16,
    rows: u16,
}

impl std::fmt::Debug for SplashImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SplashImage")
            .field("picker", &self.picker)
            .field("cols", &self.cols)
            .field("rows", &self.rows)
            .finish_non_exhaustive()
    }
}

impl Clone for SplashImage {
    fn clone(&self) -> Self {
        // 位图与终端参数可重建；StatefulProtocol 的编码缓存不参与克隆（不会写终端）。
        let picker = self.picker.clone();
        let protocol = picker.new_resize_protocol(
            image::load_from_memory(SPLASH_LOGO_PNG).expect("embedded splash logo png"),
        );
        Self {
            picker,
            protocol,
            cols: self.cols,
            rows: self.rows,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TuiState {
    pub messages: Vec<ChatMessage>,
    pub input_buffer: String,
    /// 输入光标（按 char 计数的插入点），与 input_buffer 严格同步。
    pub input_cursor: usize,
    pub is_streaming: bool,
    pub spinner_tick: usize,
    pub session_id: String,
    pub model: String,
    pub current_provider_id: String,
    pub gateway_url: String,
    pub workspace: PathBuf,
    /// wave-38 R1.4：启动目录登记后的活跃 workspace id；`None` = 沿用注册表默认工作区。
    /// `main` 在 `control_plane` 构造后用 `ensure_workspace_for_root` 填充，并作为
    /// `TurnInput.workspace_id` 的唯一来源。
    pub active_workspace_id: Option<String>,
    /// wave-38 D1：启动目录对应的 workspace 绑定（`/resume` 回退基准）。
    /// `startup_workspace_path` 与 `workspace` 同源（canonical cwd）；
    /// `startup_workspace_id` 由 `main` 在 `ensure_workspace_for_root` 成功后写入。
    pub startup_workspace_path: PathBuf,
    pub startup_workspace_id: Option<String>,
    /// wave-39 R2 / wave-41 S1：当前上下文输入 token（最近一次模型调用的 input_tokens，禁止跨调用累加，对齐 ADR 0035 S1）。
    pub total_input_tokens: u64,
    /// wave-39 R2：累计已提交轮次的输出 token。
    pub total_output_tokens: u64,
    /// wave-39 R2（契约冻结）：累计缓存命中输入 token。
    pub total_cache_hit_tokens: u64,
    /// 单轮缓存命中输入 token（仅对应当前轮单次调用，用于单轮缓存命中率计算，避免跨轮累计除以单轮输入导致>100%）。
    pub turn_cache_hit_tokens: u64,
    /// P0：本轮最新指标结算快照（`TurnStreamEvent.metrics` 的 TUI 侧唯一副本）。
    /// `Some` ⇒ 指标行完全以快照字段渲染（I6，不再读 legacy 计量字段）；
    /// `None` ⇒ 旧事件 / 历史回放路径，回退既有渲染（I7）。新轮开始与会话复位时清空。
    pub live_metrics: Option<TurnMetricsSnapshot>,
    /// wave-39 R2 / wave-41 S1：最近一次调用的上下文占用 token，用于上下文百分比分子（对齐 ADR 0035 S1）。
    pub last_context_tokens: u64,
    /// wave-39 R2：当前模型上下文窗口；`0` = 未知（指标行渲染 `—`）。
    pub context_window_tokens: u32,
    /// wave-39 R2：本轮去重回执 `(input, output, cache_hit, total)`；`None` = 本轮尚无回执。
    turn_usage: Option<(u64, u64, u64, u64)>,
    pub status_text: String,
    /// wave-36b：鼠标捕获开关（默认开，F2 切换）。
    pub mouse_capture: bool,
    pub mouse_selection: MouseSelectionState,
    pub scroll_offset: u32,
    /// 最近一次渲染可滚动到的最大偏移；事件循环据此判断「是否已回到底部」，
    /// 避免滚轮/方向键下翻时永远无法恢复自动贴底。
    pub max_scroll_offset: u32,
    pub auto_scroll: bool,
    pub mode: InteractionMode,
    pub session_list: Vec<SessionItem>,
    pub selected_session_index: usize,
    pub provider_list: Vec<ProviderItem>,
    pub selected_provider_index: usize,
    pub model_list: Vec<ModelItem>,
    pub selected_model_index: usize,
    /// Ctrl+D 默认模型 / Ctrl+F 优选列表（磁盘持久化，独立于 providers.json 契约）。
    pub model_preferences: ModelPreferences,
    pub reasoning_effort: Option<ProviderReasoningEffort>,
    pub selected_reasoning_effort_index: usize,
    pub selected_command_index: usize,
    pub add_provider_step: AddingProviderStep,
    pub add_provider_form: AddProviderForm,
    pub runtime_mode: RuntimeMode,
    pub turn_started_at: Option<std::time::Instant>,
    /// Composer 高度平滑收拢过渡（每帧衰减 1 行直至 target_input_h）
    pub anim_composer_h: Option<u16>,
    /// 最近一次计算的正文渲染区域 chat_rect
    pub last_chat_rect: Option<Rect>,
    /// 输入框文本区矩形（不含提示符与右侧留白）；鼠标选区据此判断是否落在 Composer 上。
    pub last_input_rect: Option<Rect>,
    /// 输入框本帧可视窗口的逻辑起始行（超过可视高度时窗口跟随光标）。
    pub last_input_view_start: usize,
    /// 输入框本帧可见行的纯文本快照（行 0 对应 `last_input_view_start`），供选区复制。
    pub last_input_lines: Vec<String>,
    /// 最近一次实际生效的滚动 offset（用于鼠标点击映射）
    pub effective_scroll_offset: u32,
    /// 最近一次实际渲染的纯文本行列表（与渲染视觉行严格 1:1 对齐）
    pub rendered_lines_text: Vec<String>,
    // 进阶交互状态
    pub permission_state: Option<PermissionState>,
    pub ask_user_state: Option<AskUserState>,
    pub plan_state: Option<PlanState>,
    pub task_board_state: Option<TaskBoardState>,
    pub goal_state: Option<TuiGoal>,
    /// 波次 43：当前处于待决状态的宿主控制请求（真实 `list_pending_asks` 快照）。
    /// `None` = 没有待决请求；`Some` 时对话框提交/拒绝会回写控制面。
    pub pending_request: Option<PendingUiRequest>,
    /// 波次 44：Goal 横幅首次出现的时间点，用于与指标行做时间片交替（不挤在同一行右侧）。
    pub goal_banner_since: Option<std::time::Instant>,
    /// 契约 P-3：覆盖层（审批 / 问答等模态弹窗）的 mode 栈；退出时按栈回退，空栈 ⇒ Normal。
    pub mode_stack: Vec<InteractionMode>,
    /// 契约 P-5：一次性状态提示（文案 + 产生时刻）。非流式期按 TTL 渲染，提交新消息时清空。
    pub status_notice: Option<(String, std::time::Instant)>,
    /// 契约 P-1：provider 注册表落盘路径覆盖（`None` ⇒ `ProviderRegistryStore::new()` 的默认路径）。
    /// 抽出为字段是为了可注入：测试用临时目录，生产保持默认，不在热路径硬编码用户目录。
    pub provider_registry_path: Option<PathBuf>,
    /// wave-60 §C9：停止请求进行中的轮次（`Some(turn_id)` = 「正在停止…」，**不是**终态）。
    /// 终态由 `turn:cancelled` 事件清空；流式期经 `status_word` 呈现，不走 notice 通道。
    pub stopping_turn_id: Option<String>,
    /// wave-60 §C9：当前流式轮的 turn_id（新一轮开始时覆盖）。
    /// 用于 `apply_stop_terminal` 在**无在飞停止请求**时拒绝旧轮迟到的终态。
    pub streaming_turn_id: Option<String>,
    /// W3：Team 角色花名册面板（`None` = 未打开/已关闭）。
    pub agent_roster_state: Option<AgentRosterState>,
    /// W3：进入 teammate 子会话前保存的主会话 id（`r` 返回用；`None` = 当前不在队友会话）。
    pub return_session_id: Option<String>,
    /// 初始化页品牌标：图形协议可用时以真实位图替换字符画（`None` = 字符画路径）。
    pub splash_image: Option<SplashImage>,
    /// 契约（流式期输入）：流式期间提交的消息排队快照（FIFO）；`Vec::new()` = 空队列。
    /// 元素为提交时刻的输入正文，调度口径见 `drain_next_queued`。
    pub queued_user_messages: Vec<String>,
    /// C23：队列反馈的**短时常驻**文案（C2 已入队 / C4 被拒 / C6 队列满）。
    ///
    /// 与 `status_notice` 的区别：后者在流式期按契约 P-5 被抑制；本字段由
    /// [`queue_indicator_line`] 在**每帧都渲染**的指标行输出，故流式期 / 工具执行期同样可见。
    /// `reset_session_scope` 与 `clear_messages` 必须清空（跨会话不得残留）。
    pub queue_notice: Option<(String, std::time::Instant)>,
    /// 契约（修订11 步骤2）：按消息粒度的布局缓存（宽度/条数变化或显式失效时重建）。
    render_cache: RenderCache,
}

impl Default for TuiState {
    fn default() -> Self {
        let env_gateway = std::env::var("PONYLLM_GATEWAY_URL")
            .unwrap_or_else(|_| "http://10.42.0.67:8080".to_string());
        let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let canonical_workspace = std::fs::canonicalize(&current_dir).unwrap_or(current_dir);

        Self {
            messages: Vec::new(),
            input_buffer: String::new(),
            input_cursor: 0,
            is_streaming: false,
            spinner_tick: 0,
            session_id: format!("session-{}", &uuid::Uuid::new_v4().to_string()[..8]),
            model: "gemini-3.8-flash".to_string(),
            current_provider_id: "ponyllm".to_string(),
            gateway_url: env_gateway,
            workspace: canonical_workspace.clone(),
            active_workspace_id: None,
            startup_workspace_path: canonical_workspace,
            startup_workspace_id: None,
            total_input_tokens: 0,
            total_output_tokens: 0,
            // wave-39 R2：指标字段默认归零 / 未知。
            total_cache_hit_tokens: 0,
            turn_cache_hit_tokens: 0,
            live_metrics: None,
            last_context_tokens: 0,
            context_window_tokens: 0,
            turn_usage: None,
            status_text: "Ready".to_string(),
            mouse_capture: true,
            mouse_selection: MouseSelectionState::default(),
            scroll_offset: 0,
            max_scroll_offset: 0,
            auto_scroll: true,
            mode: InteractionMode::Normal,
            session_list: Vec::new(),
            selected_session_index: 0,
            provider_list: Vec::new(),
            selected_provider_index: 0,
            model_list: Vec::new(),
            selected_model_index: 0,
            // 磁盘偏好由 run_tui 启动时加载并覆盖（Default 不做 IO，保证测试与快照可复现）。
            model_preferences: ModelPreferences::default(),
            reasoning_effort: None,
            selected_reasoning_effort_index: 0,
            selected_command_index: 0,
            add_provider_step: AddingProviderStep::Name,
            add_provider_form: AddProviderForm::default(),
            runtime_mode: RuntimeMode::Ready,
            turn_started_at: None,
            anim_composer_h: None,
            last_chat_rect: None,
            last_input_rect: None,
            last_input_view_start: 0,
            last_input_lines: Vec::new(),
            effective_scroll_offset: 0,
            rendered_lines_text: Vec::new(),
            permission_state: None,
            ask_user_state: None,
            plan_state: None,
            task_board_state: None,
            goal_state: None,
            pending_request: None,
            goal_banner_since: None,
            mode_stack: Vec::new(),
            status_notice: None,
            provider_registry_path: None,
            stopping_turn_id: None,
            streaming_turn_id: None,
            agent_roster_state: None,
            return_session_id: None,
            splash_image: None,
            queued_user_messages: Vec::new(),
            queue_notice: None,
            render_cache: RenderCache::default(),
        }
    }
}

impl TuiState {
    /// 是否存在未完成的 UI 动画（输入框收拢过渡，或平滑滚动追赶）
    pub fn has_pending_animation(&self) -> bool {
        self.anim_composer_h.is_some()
            || (self.auto_scroll && self.scroll_offset < self.max_scroll_offset)
    }

    pub fn transition_runtime_mode(&mut self, next: RuntimeMode) {
        self.runtime_mode = next;
    }

    /// 契约 P-1：取本次状态使用的注册表句柄（路径可注入，测试用临时目录）。
    fn registry_store(&self) -> ProviderRegistryStore {
        match self.provider_registry_path.as_deref() {
            Some(path) => ProviderRegistryStore::with_path(path),
            None => ProviderRegistryStore::new(),
        }
    }

    /// 契约 P-5：写入一次性状态提示（非流式期按 TTL 渲染）。
    pub fn set_status_notice(&mut self, notice: impl Into<String>) {
        self.status_notice = Some((notice.into(), std::time::Instant::now()));
    }

    /// wave-60 §C9：`turn:stop_requested` 驱动的**纯状态迁移** —— 只进入「正在停止…」。
    ///
    /// 不变量：
    /// - `is_streaming` **保持 `true`**：终态只能由 `turn:cancelled|completed|failed` 事件驱动；
    /// - 不写 `messages`：禁止在按键/请求路径本地伪造「已中断」类终态文案。
    pub fn apply_stop_requested(&mut self, turn_id: &str) {
        self.stopping_turn_id = Some(turn_id.to_string());
        // 请求目标即当前流式轮（goal 续轮会换 turn_id，以请求为准同步）。
        self.streaming_turn_id = Some(turn_id.to_string());
        self.status_text = "正在停止…".to_string();
    }

    /// wave-63 §T3：stopping 期间的按键反馈（**不静默吞**、也**不重复派发**停止请求）。
    ///
    /// 返回语义（调用约定）：
    /// - `true` ⇒ 当前**不在** stopping，调用方按原逻辑处理按键，且本调用**不改**任何文案；
    /// - `false` ⇒ 已在 stopping：只把状态提示换成「已请求停止，等待安全收口…」，调用方
    ///   **不得**再发 `stop_turn`、不得改写 `stopping_turn_id`、不得本地伪造终态
    ///   （终态只能由 `turn:cancelled` 事件驱动）。
    pub fn note_stopping_key(&mut self) -> bool {
        if self.stopping_turn_id.is_none() {
            return true;
        }
        self.status_text = STOPPING_KEY_HINT.to_string();
        false
    }

    /// wave-60 §C9：`turn:cancelled` 驱动的**纯状态迁移** —— 收口终态「已停止」。
    ///
    /// 语义：
    /// - 与在飞停止请求的 `turn_id` 不一致（陈旧事件）⇒ 忽略；
    /// - 无在飞停止请求时，只接受**当前流式轮**的终态：`turn_id` 与当前流式轮不等（旧轮终态
    ///   迟到）⇒ 忽略，绝不允许旧轮终态拆掉现行轮；
    /// - `stop_reason == Some("user")` ⇒ 「已停止（用户中断）」；其余（watchdog/parent/hook/legacy/None）
    ///   一律中性「已停止」，不得冒充用户主动停止；
    /// - **不**吞掉已流式正文、**不**追加本地伪造的终态消息。
    pub fn apply_stop_terminal(&mut self, turn_id: &str, stop_reason: Option<&str>) {
        match self.stopping_turn_id.as_deref() {
            // 在飞停止请求：只认同一轮。
            Some(stopping) if stopping != turn_id => return,
            Some(_) => {}
            // 无在飞停止请求：只认当前流式轮（old-turn 终态不得拆新轮）。
            None => {
                if let Some(active) = self.streaming_turn_id.as_deref() {
                    if active != turn_id {
                        return;
                    }
                }
            }
        }
        self.stopping_turn_id = None;
        self.streaming_turn_id = None;
        self.is_streaming = false;
        self.transition_runtime_mode(RuntimeMode::Ready);
        self.status_text = match stop_reason {
            Some("user") => "已停止（用户中断）".to_string(),
            _ => "已停止".to_string(),
        };
        // 非流式期的可见性走既有一致性通道（P-5 一次性提示，受 `should_render_status_notice` 门禁）。
        let notice = self.status_text.clone();
        self.set_status_notice(notice);
    }

    /// 契约 P-2：复位「会话作用域」状态（`/clear` 与 `/resume` 共用）。
    ///
    /// **不改 `session_id`**：切换会话由调用方负责（`/clear` 换新 id，`/resume` 用目标 id）。
    /// 复位面覆盖本轮/本会话的一切瞬时态：正文、流式标志、用量与上下文计量、计划/任务板/目标、
    /// 待决请求、滚动与补全索引、各选择器索引、一次性提示。
    pub fn reset_session_scope(&mut self) {
        self.messages.clear();
        self.status_text = "Ready".to_string();
        self.is_streaming = false;
        self.turn_started_at = None;
        self.runtime_mode = RuntimeMode::Ready;
        self.turn_usage = None;
        self.total_input_tokens = 0;
        self.total_output_tokens = 0;
        self.total_cache_hit_tokens = 0;
        self.turn_cache_hit_tokens = 0;
        // P0：会话作用域复位 ⇒ 指标快照同属会话瞬时态，必须清空（否则跨会话残留旧轮数字）。
        self.live_metrics = None;
        self.last_context_tokens = 0;
        self.context_window_tokens = 0;
        self.plan_state = None;
        self.task_board_state = None;
        self.goal_state = None;
        self.goal_banner_since = None;
        self.pending_request = None;
        self.scroll_offset = 0;
        self.max_scroll_offset = 0;
        self.effective_scroll_offset = 0;
        self.auto_scroll = true;
        self.selected_command_index = 0;
        self.spinner_tick = 0;
        self.selected_session_index = 0;
        self.selected_provider_index = 0;
        self.selected_model_index = 0;
        self.selected_reasoning_effort_index = 0;
        self.status_notice = None;
        // C13：会话作用域复位 ⇒ 旧会话的排队消息必须一并清空。
        // 否则 `/clear` / `/resume` / 队友会话切换（均经本方法 + hydrate）后，上一条会话排的队
        // 会被自动派发进新会话（跨会话正文污染 + 与新会话 turn 交叠）。
        self.queued_user_messages.clear();
        // C23：队列反馈文案同属会话作用域，复位时一并清空（否则「排队已满」会跨会话残留）。
        self.queue_notice = None;
        // wave-60 §C8/C9：会话作用域复位 ⇒ 停止请求态、当前流式轮与双击上膛一并失效。
        self.stopping_turn_id = None;
        self.streaming_turn_id = None;
        // W3：花名册面板与"返回主会话"目标都绑定会话作用域，复位时一并清空
        //（调用方若需要保留 return_session_id，必须在 reset 之后再写入）。
        self.agent_roster_state = None;
        self.return_session_id = None;
        // 会话作用域复位 ⇒ 布局缓存必须失效（/clear 与 /resume 共用本方法）。
        self.invalidate_render_cache();
    }

    /// 契约（修订11 步骤2）：显式失效布局缓存。
    ///
    /// 调用点：`/clear`、`/resume`（二者经 `reset_session_scope`）、`reset_session_scope` 本身、
    /// `UiEvent::Resized`（宽度变化会重建，但这里提前清空可避免「宽度相同但布局语义变了」的边角）。
    pub fn invalidate_render_cache(&mut self) {
        self.render_cache.cache_lines.clear();
        self.render_cache.cache_msg_count = usize::MAX;
        self.render_cache.cache_width = 0;
        self.render_cache.stats = RenderCacheStats::default();
        // 全量文本与缓存同源，必须一起失效（否则复制路径会拿到旧文本）。
        self.rendered_lines_text.clear();
    }

    /// 契约（修订11 步骤2）：当前帧的布局缓存统计（供测试/诊断）。
    pub fn render_cache_stats(&self) -> RenderCacheStats {
        self.render_cache.stats
    }

    /// 契约（修订11 步骤3）：按需重建「全量纯文本行」。
    ///
    /// 旧实现每帧把全部渲染行拼成纯文本（O(总行数) 字符串分配）；虚拟化后只在鼠标选区 /
    /// OSC52 复制这类**真实需要全量文本**的路径调用，渲染热路径不再付出这份成本。
    pub fn ensure_full_text(&mut self) {
        let width = self
            .last_chat_rect
            .map(|rect| rect.width as usize)
            .unwrap_or_else(|| self.render_cache.cache_width as usize);
        self.ensure_render_cache(width, self.spinner_tick);
        let mut text: Vec<String> = Vec::with_capacity(self.render_cache.stats.total_lines);
        for block in &self.render_cache.cache_lines {
            for line in block {
                text.push(
                    line.spans
                        .iter()
                        .map(|span| span.content.as_ref())
                        .collect::<Vec<_>>()
                        .concat(),
                );
            }
        }
        self.rendered_lines_text = text;
    }

    /// 契约（修订11 步骤2）：保证布局缓存与「宽度 + 消息列表」一致。
    ///
    /// - 宽度或消息条数变化 ⇒ 全量重建；
    /// - 否则复用 `0..n-1`，只重算最后一条（流式追加 / 工具动效 / `is_last` 都只影响末条）。
    fn ensure_render_cache(&mut self, width: usize, spinner_tick: usize) {
        let message_count = self.messages.len();
        let width_u16 = u16::try_from(width).unwrap_or(u16::MAX);
        let must_rebuild = self.render_cache.cache_width != width_u16
            || self.render_cache.cache_msg_count != message_count
            || self.render_cache.cache_lines.len() != message_count;

        if must_rebuild {
            self.render_cache.cache_width = width_u16;
            self.render_cache.cache_msg_count = message_count;
            self.render_cache.cache_lines = Vec::with_capacity(message_count);
            let mut rebuilt_lines = 0usize;
            let mut prev_role: Option<MessageRole> = None;
            let mut has_preceding_lines = false;
            for (idx, message) in self.messages.iter().enumerate() {
                let block = message_lines_with_ctx(
                    message,
                    width,
                    MessageRenderCtx {
                        prev_role: prev_role.clone(),
                        is_last: idx + 1 == message_count,
                        has_preceding_lines,
                        spinner_tick,
                    },
                );
                has_preceding_lines = has_preceding_lines || !block.is_empty();
                rebuilt_lines += block.len();
                prev_role = Some(message.role.clone());
                self.render_cache.cache_lines.push(block);
            }
            self.render_cache.stats.rebuilt_messages = message_count;
            self.render_cache.stats.reused_messages = 0;
            self.render_cache.stats.laid_out_lines = rebuilt_lines;
        } else if message_count > 0 {
            let idx = message_count - 1;
            let has_preceding_lines = self.render_cache.cache_lines[..idx]
                .iter()
                .any(|block| !block.is_empty());
            let prev_role = if idx == 0 {
                None
            } else {
                Some(self.messages[idx - 1].role.clone())
            };
            let block = message_lines_with_ctx(
                &self.messages[idx],
                width,
                MessageRenderCtx {
                    prev_role,
                    is_last: true,
                    has_preceding_lines,
                    spinner_tick,
                },
            );
            let laid_out_lines = block.len();
            self.render_cache.cache_lines[idx] = block;
            self.render_cache.stats.rebuilt_messages = 1;
            self.render_cache.stats.reused_messages = idx;
            self.render_cache.stats.laid_out_lines = laid_out_lines;
        } else {
            self.render_cache.stats = RenderCacheStats::default();
        }
        self.render_cache.stats.total_lines = self
            .render_cache
            .cache_lines
            .iter()
            .map(|block| block.len())
            .sum();
    }

    /// 契约 P-3：进入覆盖层（审批 / 问答等模态弹窗），把当前 mode 压栈。
    ///
    /// 重复进入同一层时不重复压栈（幂等）：否则同一弹窗被二次触发后，一次退出只会回到
    /// 被覆盖的同名层，UI 会卡在弹窗态出不来。
    pub fn begin_overlay(&mut self, overlay: InteractionMode) {
        if self.mode == overlay {
            return;
        }
        self.mode_stack.push(self.mode.clone());
        self.mode = overlay;
    }

    /// 契约 P-3：退出覆盖层，按栈回退（空栈 ⇒ `Normal`）。
    pub fn end_overlay(&mut self) {
        self.mode = self.mode_stack.pop().unwrap_or(InteractionMode::Normal);
        if self.mode == InteractionMode::AddingProvider {
            // 回退到「新增提供商」表单时必须清空 composer：否则用户此前输入/粘贴的 api_key
            // 会残留在输入缓冲，被当作普通消息发给模型（密钥外泄）。
            self.clear_input();
        }
    }

    pub fn runtime_status_display(&self) -> String {
        match &self.runtime_mode {
            RuntimeMode::Ready => "Ready".to_string(),
            RuntimeMode::Connecting => "准备就绪中...".to_string(),
            RuntimeMode::Thinking => "思考中...".to_string(),
            RuntimeMode::Responding => "生成中...".to_string(),
            RuntimeMode::ExecutingTool(name) => format!("执行: {}...", name),
            RuntimeMode::WaitingApproval => "等待确认中...".to_string(),
            RuntimeMode::Completed => "完成".to_string(),
            RuntimeMode::Failed(reason) => format!("失败: {}", reason),
        }
    }

    /// wave-39 R3：发送新消息前清空本轮 usage 回执（跨轮由累计字段承载）。
    pub fn begin_turn_usage(&mut self) {
        self.turn_usage = None;
        self.turn_cache_hit_tokens = 0;
        // P0：新轮开始 ⇒ 上一轮快照必须失效，否则指标行会继续显示旧轮数字。
        self.live_metrics = None;
    }

    /// P0：应用一条指标结算快照（`TurnStreamEvent.metrics` → `UiEvent::StreamMetrics`）。
    ///
    /// 快照是唯一来源：本方法**不**做任何二次换算（I6），只保存；渲染期按字段直出。
    pub fn apply_live_metrics(&mut self, metrics: TurnMetricsSnapshot) {
        self.live_metrics = Some(metrics);
    }

    /// wave-39 R3 / wave-41 S1：输入 token 与上下文按单次调用口径（ADR 0035 S1），
    /// `total_input_tokens` 表达当前上下文占用（不跨轮跨跳累加）；
    /// 输出 token 与累计缓存命中维持跨轮会话累计（幂等覆盖本轮增量）；
    /// `turn_cache_hit_tokens` 记录单轮缓存命中，用于单轮缓存率。
    pub fn apply_turn_usage(&mut self, input: u64, output: u64, cache_hit: u64, total: u64) {
        if let Some((_prev_input, prev_output, prev_cache, _prev_total)) = self.turn_usage {
            self.total_output_tokens = self.total_output_tokens.saturating_sub(prev_output);
            self.total_cache_hit_tokens = self.total_cache_hit_tokens.saturating_sub(prev_cache);
        }
        self.total_input_tokens = input;
        self.total_output_tokens = self.total_output_tokens.saturating_add(output);
        self.total_cache_hit_tokens = self.total_cache_hit_tokens.saturating_add(cache_hit);
        self.turn_cache_hit_tokens = cache_hit;
        self.last_context_tokens = input;
        self.turn_usage = Some((input, output, cache_hit, total));
    }
    pub fn start_add_provider(&mut self) {
        self.mode = InteractionMode::AddingProvider;
        self.add_provider_step = AddingProviderStep::Name;
        self.add_provider_form = AddProviderForm::default();
        self.input_buffer.clear();
    }

    pub fn cancel_add_provider(&mut self) {
        self.mode = InteractionMode::Normal;
        self.add_provider_step = AddingProviderStep::Name;
        self.add_provider_form = AddProviderForm::default();
        self.input_buffer.clear();
    }

    pub fn add_provider_input(&mut self, text: &str) {
        match self.add_provider_step {
            AddingProviderStep::Name => {
                self.add_provider_form.name = text.to_string();
            }
            AddingProviderStep::BaseUrl => {
                self.add_provider_form.base_url = text.to_string();
            }
            AddingProviderStep::ApiKey => {
                let trimmed = text.trim();
                self.add_provider_form.api_key = if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string())
                };
            }
            AddingProviderStep::Commit => {}
        }
    }

    pub fn advance_add_provider_step(&mut self) -> Result<(), &'static str> {
        match self.add_provider_step {
            AddingProviderStep::Name => {
                if self.add_provider_form.name.trim().is_empty() {
                    return Err("Provider 名称不能为空");
                }
                self.add_provider_step = AddingProviderStep::BaseUrl;
                Ok(())
            }
            AddingProviderStep::BaseUrl => {
                let url = self.add_provider_form.base_url.trim();
                if url.is_empty() {
                    return Err("Base URL 不能为空");
                }
                if !url.starts_with("http://") && !url.starts_with("https://") {
                    return Err("Base URL 必须是以 http:// 或 https:// 开头的合法地址");
                }
                self.add_provider_step = AddingProviderStep::ApiKey;
                Ok(())
            }
            AddingProviderStep::ApiKey => {
                self.add_provider_step = AddingProviderStep::Commit;
                Ok(())
            }
            AddingProviderStep::Commit => Ok(()),
        }
    }

    pub fn finalize_add_provider(&mut self) -> Option<AddProviderPayload> {
        let name = self.add_provider_form.name.trim().to_string();
        let base_url = self.add_provider_form.base_url.trim().to_string();
        if name.is_empty() || base_url.is_empty() {
            return None;
        }
        let id = name
            .to_lowercase()
            .replace(|c: char| !c.is_alphanumeric(), "-")
            .trim_matches('-')
            .to_string();
        let final_id = if id.is_empty() {
            format!("provider-{}", &uuid::Uuid::new_v4().to_string()[..6])
        } else {
            id
        };

        let payload = AddProviderPayload {
            id: final_id,
            name,
            protocol: "openai".to_string(),
            base_url,
            api_key: self.add_provider_form.api_key.clone(),
        };

        self.mode = InteractionMode::Normal;
        self.add_provider_step = AddingProviderStep::Name;
        self.add_provider_form = AddProviderForm::default();
        Some(payload)
    }

    pub fn open_provider_selector(&mut self, list: Vec<ProviderItem>) {
        self.provider_list = list;
        self.selected_provider_index = 0;
        self.mode = InteractionMode::SelectingProvider;
    }

    pub fn cancel_provider_selector(&mut self) {
        self.mode = InteractionMode::Normal;
        self.provider_list.clear();
    }

    pub fn next_provider(&mut self) {
        if !self.provider_list.is_empty() && self.selected_provider_index + 1 < self.provider_list.len() {
            self.selected_provider_index += 1;
        }
    }

    pub fn prev_provider(&mut self) {
        if self.selected_provider_index > 0 {
            self.selected_provider_index -= 1;
        }
    }

    pub fn delete_current_provider(&mut self) -> Result<String, &'static str> {
        if self.provider_list.is_empty() {
            return Err("当前列表无提供商");
        }
        if self.selected_provider_index >= self.provider_list.len() {
            return Err("选择项无效");
        }
        let target = self.provider_list[self.selected_provider_index].clone();
        if target.id == "ponyllm" {
            return Err("内置提供商 [ponyllm] 不可删除");
        }

        let registry = self.registry_store();
        let mut view = registry.load_view();
        view.providers.retain(|p| p.id != target.id);

        if self.current_provider_id == target.id {
            let next_id = view
                .providers
                .first()
                .map(|p| p.id.clone())
                .unwrap_or_else(|| "ponyllm".to_string());
            self.current_provider_id = next_id.clone();
            view.selected_provider_id = Some(next_id);
        }

        if let Err(_e) = registry.save_view(view.clone()) {
            return Err("保存注册表失败");
        }

        self.provider_list.remove(self.selected_provider_index);
        if self.selected_provider_index >= self.provider_list.len() && !self.provider_list.is_empty() {
            self.selected_provider_index = self.provider_list.len() - 1;
        }

        Ok(format!("已删除提供商 · {}", target.name))
    }

    pub fn select_current_provider(&mut self) -> Option<ProviderItem> {
        if !self.provider_list.is_empty() && self.selected_provider_index < self.provider_list.len() {
            let selected = self.provider_list[self.selected_provider_index].clone();
            self.current_provider_id = selected.id.clone();
            let registry = self.registry_store();
            let mut view = registry.load_view();
            view.selected_provider_id = Some(selected.id.clone());
            match registry.save_view(view) {
                Ok(_) => self.set_status_notice(format!("已切换提供商 · {}", selected.name)),
                Err(error) => {
                    self.set_status_notice(format!("提供商切换保存失败: {error}"));
                }
            }
            // 选择完成后收起选择器（与 cancel_provider_selector 同语义）。
            // 回归来源：22241e9 重构丢失该复位 ⇒ 选中 provider 后菜单不关闭，
            // tui_acceptance::test_provider_selector_state_logic 因此变红。
            self.provider_list.clear();
            self.mode = InteractionMode::Normal;
            Some(selected)
        } else {
            None
        }
    }

    pub fn open_model_selector(&mut self, list: Vec<ModelItem>) {
        self.model_list = list;
        // 每次打开都与磁盘对齐：Ctrl+D / Ctrl+F 之外的外部改动（手改文件、其他实例）也能看到。
        self.model_preferences = ModelPreferencesStore::new().load();
        self.selected_model_index = self
            .model_preferences
            .default_model
            .as_ref()
            .and_then(|id| self.model_list.iter().position(|item| &item.id == id))
            .unwrap_or(0);
        // 覆盖层：进入前模式入栈，关闭时按栈恢复（PlanMode 等非 Normal 模式不被静默丢弃）。
        self.begin_overlay(InteractionMode::SelectingModel);
    }

    pub fn cancel_model_selector(&mut self) {
        self.end_overlay();
        self.model_list.clear();
    }

    pub fn next_model(&mut self) {
        if !self.model_list.is_empty() && self.selected_model_index + 1 < self.model_list.len() {
            self.selected_model_index += 1;
        }
    }

    pub fn prev_model(&mut self) {
        if self.selected_model_index > 0 {
            self.selected_model_index -= 1;
        }
    }

    pub fn select_current_model(&mut self) {
        if !self.model_list.is_empty() && self.selected_model_index < self.model_list.len() {
            let m = &self.model_list[self.selected_model_index];
            self.model = m.id.clone();
            self.status_text = format!("Switched model to {}", m.name);
        }
        // 覆盖层退出必须按栈恢复（不能硬写 Normal，否则会丢掉 PlanMode）。
        self.end_overlay();
    }

    pub fn reasoning_effort_label(&self) -> &'static str {
        match self.reasoning_effort {
            None => "默认",
            Some(ProviderReasoningEffort::Low) => "低",
            Some(ProviderReasoningEffort::Medium) => "中",
            Some(ProviderReasoningEffort::High) => "高",
            Some(ProviderReasoningEffort::Max) => "极高",
        }
    }

    pub fn reasoning_effort_display(&self) -> String {
        match self.reasoning_effort {
            None => "默认 (Default)".to_string(),
            Some(ProviderReasoningEffort::Low) => "低 · low".to_string(),
            Some(ProviderReasoningEffort::Medium) => "中 · medium".to_string(),
            Some(ProviderReasoningEffort::High) => "高 · high".to_string(),
            Some(ProviderReasoningEffort::Max) => "极高 · max".to_string(),
        }
    }

    pub fn cycle_reasoning_effort(&mut self) -> Option<ProviderReasoningEffort> {
        let current_pos = REASONING_EFFORT_OPTIONS
            .iter()
            .position(|(eff, _, _)| *eff == self.reasoning_effort)
            .unwrap_or(0);
        let next_pos = (current_pos + 1) % REASONING_EFFORT_OPTIONS.len();
        let (next_effort, name, _) = &REASONING_EFFORT_OPTIONS[next_pos];
        self.reasoning_effort = next_effort.clone();
        self.selected_reasoning_effort_index = next_pos;
        self.set_status_notice(format!("已切换思考强度: {}", name));
        self.reasoning_effort.clone()
    }

    pub fn open_reasoning_effort_selector(&mut self) {
        let pos = REASONING_EFFORT_OPTIONS
            .iter()
            .position(|(eff, _, _)| *eff == self.reasoning_effort)
            .unwrap_or(0);
        self.selected_reasoning_effort_index = pos;
        self.mode = InteractionMode::SelectingReasoningEffort;
    }

    pub fn cancel_reasoning_effort_selector(&mut self) {
        self.mode = InteractionMode::Normal;
    }

    pub fn next_reasoning_effort(&mut self) {
        if self.selected_reasoning_effort_index + 1 < REASONING_EFFORT_OPTIONS.len() {
            self.selected_reasoning_effort_index += 1;
        }
    }

    pub fn prev_reasoning_effort(&mut self) {
        if self.selected_reasoning_effort_index > 0 {
            self.selected_reasoning_effort_index -= 1;
        }
    }

    pub fn select_current_reasoning_effort(&mut self) {
        if self.selected_reasoning_effort_index < REASONING_EFFORT_OPTIONS.len() {
            let (eff, name, _) = &REASONING_EFFORT_OPTIONS[self.selected_reasoning_effort_index];
            self.reasoning_effort = eff.clone();
            self.set_status_notice(format!("已切换思考强度: {}", name));
        }
        self.mode = InteractionMode::Normal;
    }

    pub fn open_session_selector(&mut self, list: Vec<SessionItem>) {
        self.session_list = list;
        self.selected_session_index = 0;
        self.mode = InteractionMode::SelectingSession;
    }

    pub fn cancel_session_selector(&mut self) {
        self.mode = InteractionMode::Normal;
        self.session_list.clear();
    }

    // ── 进阶交互模式切换与状态桩函数 ──

    pub fn start_permission_approval(&mut self, request: PermissionRequest) {
        self.permission_state = Some(PermissionState::new(request));
        // 契约 P-3：审批弹窗是覆盖层，退出时按栈回退（不强行清成 Normal）。
        self.begin_overlay(InteractionMode::PermissionApproval);
    }

    pub fn cancel_permission_approval(&mut self) {
        self.permission_state = None;
        self.end_overlay();
    }

    pub fn start_ask_user(&mut self, request_id: String, questions: Vec<AskUserQuestion>) {
        self.ask_user_state = Some(AskUserState::new(request_id, questions));
        // 契约 P-3：问答弹窗同为覆盖层。
        self.begin_overlay(InteractionMode::AskUserQuestion);
    }

    pub fn cancel_ask_user(&mut self) {
        self.ask_user_state = None;
        self.end_overlay();
    }

    pub fn toggle_plan_mode(&mut self) {
        if self.mode == InteractionMode::PlanMode {
            self.mode = InteractionMode::Normal;
            if let Some(ref mut plan) = self.plan_state {
                plan.is_active_plan_mode = false;
            }
        } else {
            self.mode = InteractionMode::PlanMode;
            if let Some(ref mut plan) = self.plan_state {
                plan.is_active_plan_mode = true;
            }
        }
    }

    pub fn toggle_task_board(&mut self) {
        // 面板是**覆盖层**，不是模式替换：走 `begin_overlay`/`end_overlay` 记录并恢复进入前的模式。
        // 旧实现直接写 `mode = TaskBoard|Normal` ⇒ 从 PlanMode 打开看板再关闭会静默丢失 PlanMode
        //（plan_state 残留、界面回 Normal）。
        if self.mode == InteractionMode::TaskBoard {
            self.end_overlay();
        } else {
            self.begin_overlay(InteractionMode::TaskBoard);
        }
    }

    /// W3：刷新 `/agents` 花名册快照（每次打开/按 R 重读真实运行时）。
    pub fn refresh_agent_roster(&mut self) {
        self.agent_roster_state = Some(agent_roster_state_from_runtime());
    }

    /// W3：`/agents` 切换（面板内再按 `/agents` 等价于关闭）。
    pub fn toggle_agent_roster(&mut self) {
        if self.mode == InteractionMode::AgentRoster {
            self.close_agent_roster();
        } else {
            self.refresh_agent_roster();
            self.mode = InteractionMode::AgentRoster;
        }
    }

    /// W3：关闭花名册（Esc / 再次切换）；**不清** `return_session_id`（返回目标与会话绑定）。
    pub fn close_agent_roster(&mut self) {
        self.agent_roster_state = None;
        if self.mode == InteractionMode::AgentRoster {
            self.mode = InteractionMode::Normal;
        }
    }

    /// W3：当前花名册选中项的 target（无面板/空表 ⇒ `None`）。
    pub fn selected_agent_target(&self) -> Option<String> {
        self.agent_roster_state
            .as_ref()
            .and_then(|roster| roster.selected_entry())
            .map(|entry| entry.target.clone())
    }

    pub fn update_plan(&mut self, plan: PlanState) {
        self.plan_state = Some(plan);
    }

    pub fn update_task_board(&mut self, tasks: Vec<TuiTask>) {
        if let Some(ref mut board) = self.task_board_state {
            board.tasks = tasks;
        } else {
            self.task_board_state = Some(TaskBoardState::new(tasks));
        }
    }

    pub fn set_goal(&mut self, goal: TuiGoal) {
        // 新目标（或阶段/轮次变化）重置横幅时间片，保证一次切换从 Goal 相位开始。
        let restarted = match &self.goal_state {
            Some(previous) => {
                previous.id != goal.id
                    || previous.round_count != goal.round_count
                    || previous.phase != goal.phase
            }
            None => true,
        };
        if restarted || self.goal_banner_since.is_none() {
            self.goal_banner_since = Some(std::time::Instant::now());
        }
        self.goal_state = Some(goal);
    }

    pub fn next_session(&mut self) {
        if !self.session_list.is_empty() && self.selected_session_index + 1 < self.session_list.len() {
            self.selected_session_index += 1;
        }
    }

    pub fn prev_session(&mut self) {
        if self.selected_session_index > 0 {
            self.selected_session_index -= 1;
        }
    }

    pub fn matching_commands(&self) -> Vec<(&'static str, &'static str)> {
        if self.input_buffer.starts_with('/') {
            let filter = self.input_buffer.to_lowercase();
            // 契约 C-6：必须返回全部匹配命令，行数上限由渲染侧按窗口可用行数裁定。
            AVAILABLE_COMMANDS
                .iter()
                .copied()
                .filter(|(cmd, _)| cmd.to_lowercase().starts_with(&filter))
                .collect()
        } else {
            Vec::new()
        }
    }
    pub fn scroll_up(&mut self, delta: u32) {
        if self.auto_scroll {
            // 如果之前处于自动贴底，第一次上翻时从当前最大偏移量开始扣减
            self.auto_scroll = false;
        }
        self.scroll_offset = self.scroll_offset.saturating_sub(delta);
    }

    pub fn scroll_down(&mut self, delta: u32, max_offset: u32) {
        let next = self.scroll_offset.saturating_add(delta);
        if next >= max_offset {
            self.scroll_offset = max_offset;
            self.auto_scroll = true; // 滚到底部时恢复自动贴底跟踪
        } else {
            self.scroll_offset = next;
        }
    }

    /// End：直接回到最新消息并恢复自动贴底。
    pub fn scroll_to_bottom(&mut self) {
        self.scroll_offset = self.max_scroll_offset;
        self.auto_scroll = true;
    }
}

/// wave-39 R3：解析当前 provider/model 的上下文窗口 token 数。
/// 命中注册表则取 `capabilities.context_window_tokens`，否则回退 `1_000_000`。
pub fn resolve_context_window_tokens(state: &TuiState) -> u32 {
    /// 与 `config::DEFAULT_CONTEXT_WINDOW_TOKENS` 同值；TUI 侧不引入额外依赖。
    const DEFAULT_CONTEXT_WINDOW_TOKENS: u32 = 1_000_000;

    let view = ProviderRegistryStore::new().load_view();
    let provider = resolve_active_provider_id(state)
        .and_then(|id| view.providers.iter().find(|p| p.id == id));
    if let Some(provider) = provider {
        if let Some(model) = provider
            .models
            .iter()
            .find(|m| m.id == state.model || m.model == state.model)
        {
            if let Some(window) = model.capabilities.context_window_tokens {
                return window;
            }
        }
    }
    DEFAULT_CONTEXT_WINDOW_TOKENS
}

/// 契约 F-H：可滚动行数（u32 承载）。
///
/// 旧实现把 `(total - visible)` 直接 `as u16`：超过 65535 视觉行时回绕，滚动位置会突然跳到顶部
/// （真实缺陷）。这里改为饱和转换，并把「是否可以再滚」交给调用方比较，语义不变。
pub fn calculate_scroll_offset(total_lines: usize, visible_height: usize) -> u32 {
    total_lines
        .saturating_sub(visible_height)
        .try_into()
        .unwrap_or(u32::MAX)
}

/// 契约 F-H：喂给 ratatui 的滚动偏移必须回到 u16（`Paragraph::scroll` 的入参类型）。
///
/// 饱和而不是截断：超过 u16::MAX 的逻辑偏移只能饱和渲染（终端行数上限所限），
/// 但**内部状态**仍是 u32 真值，不再因为 cast 而回绕。
pub fn clamp_render_scroll(offset: u32) -> u16 {
    offset.min(u16::MAX as u32) as u16
}

// 状态感知动效字形集（精细化点阵与轨道流）：
// 1. 思考中 (Thinking): 脉冲呼吸星芒 (· ✢ ✦ ✢)
// 2. 生成中 (Responding): 盲文水滴波动 / 动态字符流 (⠋ ⠙ ⠹ ⠸ ⠼ ⠴ ⠦ ⠧ ⠇ ⠏)
// 3. 执行中 (ExecutingTool): 精致平滑弧光流 (◜ ◠ ◝ ◞ ◡ ◟)
const THINKING_FRAMES: &[&str] = &["·", "✢", "✦", "✢"];
const RESPONDING_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const EXECUTING_FRAMES: &[&str] = &["◜", "◠", "◝", "◞", "◡", "◟"];
const DEFAULT_SPINNER_FRAMES: &[&str] = &["·", "•", "●", "•"];

pub fn spinner_frame(tick: usize) -> &'static str {
    DEFAULT_SPINNER_FRAMES[tick % DEFAULT_SPINNER_FRAMES.len()]
}

pub fn mode_status_icon(mode: &RuntimeMode, tick: usize) -> &'static str {
    match mode {
        RuntimeMode::Thinking => THINKING_FRAMES[tick % THINKING_FRAMES.len()],
        RuntimeMode::Responding => RESPONDING_FRAMES[tick % RESPONDING_FRAMES.len()],
        RuntimeMode::ExecutingTool(_) => executing_tool_frame(tick),
        _ => spinner_frame(tick),
    }
}

/// 契约 F-I：执行中工具行的动效帧。
///
/// 渲染路径只需要「当前是执行态 + tick」，不需要工具正文；旧调用点为此
/// `msg.content.clone()` 构造了一个临时 `RuntimeMode::ExecutingTool(String)`（每帧一次正文拷贝）。
fn executing_tool_frame(tick: usize) -> &'static str {
    EXECUTING_FRAMES[tick % EXECUTING_FRAMES.len()]
}

impl TuiState {
    // ── 输入编辑（光标感知，按 char 索引，保证 CJK 安全）──────────────────
    pub fn input_char_len(&self) -> usize {
        self.input_buffer.chars().count()
    }

    fn input_byte_index(&self, char_idx: usize) -> usize {
        self.input_buffer
            .char_indices()
            .nth(char_idx)
            .map(|(byte, _)| byte)
            .unwrap_or(self.input_buffer.len())
    }

    /// 整体替换输入内容，并把光标移到末尾（命令补全/回填专用）。
    pub fn set_input(&mut self, text: &str) {
        self.input_buffer = text.to_string();
        self.input_cursor = self.input_buffer.chars().count();
    }

    pub fn insert_char(&mut self, c: char) {
        let cursor = self.input_cursor.min(self.input_char_len());
        let byte = self.input_byte_index(cursor);
        self.input_buffer.insert(byte, c);
        self.input_cursor = cursor + 1;
    }

    pub fn insert_newline(&mut self) {
        self.insert_char('\n');
    }

    /// 契约 F-C：把粘贴文本整段插入输入缓冲（CRLF/CR → LF），**不提交、不触发发送**。
    ///
    /// 与逐字符 `insert_char` 等价但一次性 `insert_str`：大段粘贴不会退化成 O(n²)，
    /// 光标按字符数（非字节数）前移，CJK 安全。
    pub fn insert_paste(&mut self, text: &str) {
        let normalized = normalize_paste(text);
        if normalized.is_empty() {
            return;
        }
        let cursor = self.input_cursor.min(self.input_char_len());
        let byte = self.input_byte_index(cursor);
        self.input_buffer.insert_str(byte, &normalized);
        self.input_cursor = cursor + normalized.chars().count();
    }

    pub fn backspace(&mut self) {
        if self.input_cursor == 0 {
            return;
        }
        let cursor = self.input_cursor.min(self.input_char_len());
        let byte = self.input_byte_index(cursor - 1);
        self.input_buffer.remove(byte);
        self.input_cursor = cursor - 1;
    }

    pub fn delete_forward(&mut self) {
        let cursor = self.input_cursor.min(self.input_char_len());
        if cursor >= self.input_char_len() {
            return;
        }
        let byte = self.input_byte_index(cursor);
        self.input_buffer.remove(byte);
    }

    pub fn move_cursor_left(&mut self) {
        if self.input_cursor > 0 {
            self.input_cursor -= 1;
        }
    }

    pub fn move_cursor_right(&mut self) {
        if self.input_cursor < self.input_char_len() {
            self.input_cursor += 1;
        }
    }

    pub fn cursor_line_home(&mut self) {
        let chars: Vec<char> = self.input_buffer.chars().collect();
        let mut idx = self.input_cursor.min(chars.len());
        while idx > 0 && chars[idx - 1] != '\n' {
            idx -= 1;
        }
        self.input_cursor = idx;
    }

    pub fn cursor_line_end(&mut self) {
        let chars: Vec<char> = self.input_buffer.chars().collect();
        let mut idx = self.input_cursor.min(chars.len());
        while idx < chars.len() && chars[idx] != '\n' {
            idx += 1;
        }
        self.input_cursor = idx;
    }

    /// Ctrl+W：向左删除一个词。
    pub fn delete_word_backward(&mut self) {
        let chars: Vec<char> = self.input_buffer.chars().collect();
        let mut idx = self.input_cursor.min(chars.len());
        while idx > 0 && chars[idx - 1].is_whitespace() {
            self.backspace();
            idx -= 1;
        }
        while idx > 0 && !chars[idx - 1].is_whitespace() {
            self.backspace();
            idx -= 1;
        }
    }

    pub fn clear_input(&mut self) {
        let lines_count = (self.input_buffer.lines().count() + 1).clamp(2, 6) as u16;
        if lines_count > 2 {
            self.anim_composer_h = Some(lines_count);
        }
        self.input_buffer.clear();
        self.input_cursor = 0;
    }

    /// 应用 Composer 编辑类快捷键（真值源：[`composer_control_action`]）。
    ///
    /// 主循环在**流式守卫之前**调用它，所以这些编辑动作在生成过程中同样生效。
    pub fn apply_composer_control(&mut self, action: ComposerControlAction) {
        match action {
            ComposerControlAction::ClearInput => self.clear_input(),
            ComposerControlAction::InsertNewline => self.insert_newline(),
            ComposerControlAction::DeleteWordBackward => self.delete_word_backward(),
            ComposerControlAction::LineHome => self.cursor_line_home(),
            ComposerControlAction::LineEnd => self.cursor_line_end(),
        }
    }

    pub fn add_user_message(&mut self, text: &str) {
        self.messages.push(ChatMessage {
            role: MessageRole::User,
            content: text.to_string(),
            timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
        });
    }

    pub fn append_assistant_chunk(&mut self, chunk: &str) {
        if let Some(last) = self.messages.last_mut() {
            if last.role == MessageRole::Assistant {
                last.content.push_str(chunk);
                return;
            }
        }
        self.messages.push(ChatMessage {
            role: MessageRole::Assistant,
            content: chunk.to_string(),
            timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
        });
    }

    pub fn add_or_update_tool_event(&mut self, desc: &str, is_done: bool) {
        // 如果末尾就是当前工具条目，直接原位更新状态与文本，彻底解决上下产生两条重复消息的困扰
        if let Some(last) = self.messages.last_mut() {
            if last.role == MessageRole::ToolCall {
                last.content = desc.to_string();
                if is_done {
                    last.role = MessageRole::ToolResult;
                }
                return;
            }
        }
        self.messages.push(ChatMessage {
            role: if is_done { MessageRole::ToolResult } else { MessageRole::ToolCall },
            content: desc.to_string(),
            timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
        });
    }

    pub fn add_tool_event(&mut self, is_call: bool, desc: &str) {
        self.add_or_update_tool_event(desc, !is_call);
    }

    pub fn clear_messages(&mut self) {
        self.messages.clear();
        self.session_id = format!("session-{}", &uuid::Uuid::new_v4().to_string()[..8]);
        self.status_text = "Context cleared".to_string();
        // C13/C23（换 session 的另一个入口）：排队消息与队列反馈同属会话作用域，必须一并清空。
        // 否则旧会话排的队会被派发进新 session（正文污染 + 与新会话 turn 交叠）。
        self.queued_user_messages.clear();
        self.queue_notice = None;
    }
}

/// 协议枚举的展示名：不把 Rust Debug 名直接暴露给用户。
fn protocol_label(protocol: &ProviderProtocol) -> &'static str {
    match protocol {
        ProviderProtocol::OpenAiCompletions => "openai",
        ProviderProtocol::OpenAiResponses => "openai-responses",
        ProviderProtocol::AnthropicMessages => "anthropic",
    }
}

pub fn format_tool_display(
    name: &str,
    display_zh: Option<&str>,
    summary_or_args: Option<&str>,
    _status: Option<&str>,
) -> String {
    let title = if let Some(zh) = display_zh {
        if !zh.trim().is_empty() && zh != name {
            format!("{} ({})", zh.trim(), name)
        } else {
            name.to_string()
        }
    } else {
        name.to_string()
    };

    if let Some(detail) = summary_or_args {
        let trimmed = detail.trim();
        if !trimmed.is_empty() {
            return format!("{} · {}", title, trimmed);
        }
    }
    title
}

/// 工具详情兜底：只接受人类可读纯文本；结构化 JSON（对象/数组）一律不回显。
fn plain_text_detail(arguments_text: Option<&str>, result_text: Option<&str>) -> Option<String> {
    let detail = arguments_text.or(result_text)?.trim();
    if detail.is_empty() {
        return None;
    }
    match serde_json::from_str::<serde_json::Value>(detail) {
        Ok(serde_json::Value::Object(_)) | Ok(serde_json::Value::Array(_)) => None,
        Ok(serde_json::Value::String(text)) => Some(text),
        _ => Some(detail.to_string()),
    }
}

/// 历史恢复时的工具摘要选择：优先面向用户的 `description`，并识别「描述只是工具名」
/// 的占位形态；交互 Ask 的结构化参数/结果不得作为摘要反显。
fn is_interactive_ask_tool(name: &str, canonical_name: Option<&str>) -> bool {
    let key = canonical_name
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(name)
        .to_ascii_lowercase();
    matches!(
        key.as_str(),
        "ask_user" | "ask" | "builtin:ask" | "builtin:ask_user"
    )
}

pub fn history_tool_detail(
    name: &str,
    display_zh: Option<&str>,
    canonical_name: Option<&str>,
    description: &str,
    arguments_text: Option<&str>,
    result_text: Option<&str>,
) -> Option<String> {
    let description = description.trim();
    let description_is_placeholder = description.is_empty()
        || description.eq_ignore_ascii_case(name)
        || display_zh
            .map(|display| description.eq_ignore_ascii_case(display.trim()))
            .unwrap_or(false);
    if !description_is_placeholder {
        return Some(description.to_string());
    }
    if is_interactive_ask_tool(name, canonical_name) {
        return None;
    }
    // 结构化参数内置的人类摘要字段；只提取该字段，绝不把整段 JSON 反显。
    if let Some(summary) = arguments_text
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .and_then(|value| {
            value
                .get("description")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|summary| !summary.is_empty())
                .map(str::to_string)
        })
    {
        return Some(summary);
    }
    if !description.is_empty() {
        // 占位描述只是工具名：宁可不重复展示，也不把结构化参数反显为摘要。
        return None;
    }
    plain_text_detail(arguments_text, result_text)
}

pub fn render_markdown_line(line: &str) -> Vec<Span<'static>> {
    let trimmed = line.trim_start();
    let hash_count = trimmed.chars().take_while(|&c| c == '#').count();
    if hash_count > 0 && trimmed[hash_count..].starts_with(' ') {
        // 标题不用符号噪音，只用字重与微小的色阶差异表达层级。
        let clean_text = trimmed[hash_count..].trim_start();
        let style = match hash_count {
            1 => Style::default().fg(theme::INK).add_modifier(Modifier::BOLD),
            2 => Style::default().fg(theme::INK).add_modifier(Modifier::BOLD),
            3 => Style::default().fg(theme::INK_SOFT).add_modifier(Modifier::BOLD),
            _ => Style::default().fg(theme::INK_SOFT),
        };
        return vec![Span::styled(clean_text.to_string(), style)];
    }

    if let Some(rest) = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")) {
        let mut spans = vec![Span::styled("  · ", faint())];
        spans.extend(parse_inline_formatting(rest));
        return spans;
    }

    // 数字有序列表例如 "1. ", "2. "
    if let Some(dot_idx) = trimmed.find(". ") {
        let num_part = &trimmed[..dot_idx];
        if !num_part.is_empty() && num_part.chars().all(|c| c.is_ascii_digit()) {
            let mut spans = vec![Span::styled(format!("  {}. ", num_part), faint())];
            spans.extend(parse_inline_formatting(&trimmed[dot_idx + 2..]));
            return spans;
        }
    }

    // 分割线 --- 或 ***：一条安静的发丝线
    if trimmed == "---" || trimmed == "***" || trimmed == "___" {
        return vec![Span::styled(
            "─".repeat(48),
            Style::default().fg(theme::HAIRLINE),
        )];
    }

    // 代码块围栏 ```lang 只显示语言标签；裸围栏保持安静（不占视觉噪音）
    if trimmed.starts_with("```") {
        let lang = trimmed.trim_start_matches('`').trim();
        if lang.is_empty() {
            return vec![Span::raw("")];
        }
        return vec![Span::styled(lang.to_string(), faint())];
    }

    parse_inline_formatting(line)
}

fn parse_inline_formatting(text: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut rest = text;
    let plain = |s: &str| Span::styled(s.to_string(), ink_soft());

    while !rest.is_empty() {
        let bold = rest.find("**");
        let code = rest.find('`');
        // 按「先出现的标记」处理，避免先遇到加粗就把前面的行内代码整段吞成纯文本。
        let use_bold = match (bold, code) {
            (Some(b), Some(c)) => b <= c,
            (Some(_), None) => true,
            _ => false,
        };
        if use_bold {
            let b = bold.unwrap();
            if let Some(off) = rest[b + 2..].find("**") {
                if b > 0 {
                    spans.push(plain(&rest[..b]));
                }
                spans.push(Span::styled(
                    rest[b + 2..b + 2 + off].to_string(),
                    Style::default()
                        .fg(theme::EMPHASIS)
                        .add_modifier(Modifier::BOLD),
                ));
                rest = &rest[b + 2 + off + 2..];
                continue;
            }
        }
        if let Some(c) = code {
            if let Some(off) = rest[c + 1..].find('`') {
                if c > 0 {
                    spans.push(plain(&rest[..c]));
                }
                spans.push(Span::styled(
                    rest[c + 1..c + 1 + off].to_string(),
                    Style::default().fg(theme::VSCODE_STRING).bg(theme::CODE_FILL),
                ));
                rest = &rest[c + 1 + off + 1..];
                continue;
            }
        }
        spans.push(plain(rest));
        break;
    }
    spans
}

/// sRGB 8bit 分量 → 线性光分量（IEC 61966-2-1 传输函数）。
fn srgb_channel_to_linear(channel: u8) -> f32 {
    let v = channel as f32 / 255.0;
    if v <= 0.040_45 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

/// 线性光分量 → sRGB 8bit 分量（夹取到 [0,255]，越界插值不会写坏终端色）。
fn linear_channel_to_srgb(value: f32) -> u8 {
    let v = value.clamp(0.0, 1.0);
    let encoded = if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    };
    (encoded * 255.0).round().clamp(0.0, 255.0) as u8
}

/// sRGB → OKLab（Björn Ottosson, 2020）：感知均匀空间，插值不经过发灰的暗带。
fn srgb_to_oklab(rgb: [u8; 3]) -> [f32; 3] {
    let r = srgb_channel_to_linear(rgb[0]);
    let g = srgb_channel_to_linear(rgb[1]);
    let b = srgb_channel_to_linear(rgb[2]);
    let l = 0.412_221_47 * r + 0.536_332_55 * g + 0.051_445_99 * b;
    let m = 0.211_903_5 * r + 0.680_699_54 * g + 0.107_396_96 * b;
    let s = 0.088_302_46 * r + 0.281_718_84 * g + 0.629_978_7 * b;
    let (l_, m_, s_) = (l.cbrt(), m.cbrt(), s.cbrt());
    [
        0.210_454_26 * l_ + 0.793_617_8 * m_ - 0.004_072_047 * s_,
        1.977_998_5 * l_ - 2.428_592_2 * m_ + 0.450_593_7 * s_,
        0.025_904_037 * l_ + 0.782_771_77 * m_ - 0.808_675_8 * s_,
    ]
}

/// OKLab → sRGB（越界分量由 [`linear_channel_to_srgb`] 夹取）。
fn oklab_to_srgb(lab: [f32; 3]) -> [u8; 3] {
    let [l, a, b] = lab;
    let l_ = l + 0.396_337_78 * a + 0.215_803_76 * b;
    let m_ = l - 0.105_561_346 * a - 0.063_854_17 * b;
    let s_ = l - 0.089_484_18 * a - 1.291_485_5 * b;
    let (l3, m3, s3) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);
    [
        linear_channel_to_srgb(4.076_741_7 * l3 - 3.307_711_6 * m3 + 0.230_969_93 * s3),
        linear_channel_to_srgb(-1.268_438 * l3 + 2.609_757_4 * m3 - 0.341_319_4 * s3),
        linear_channel_to_srgb(-0.004_196_086_3 * l3 - 0.703_418_6 * m3 + 1.707_614_7 * s3),
    ]
}

/// 初始化横幅品牌渐变色标（PonyFun 折角标的流光走向，整体提亮并补入高亮色标）：
/// 暖金高光 → 亮橙 → 珊瑚红 → 亮玫红 → 亮洋红紫 → 亮紫 → 亮蓝紫 → 靛蓝收尾。
/// 旧版四级色标（橙 500 → 红 500 → 紫 500 → 靛蓝 600）中段红→紫直插会发灰、发闷；
/// 这里新增暖金 / 亮玫红 / 亮蓝紫三处亮部，收尾也由 `#4f46e5` 提亮到 `#5b5bf0`。
const SPLASH_GRADIENT_STOPS: [(f32, [u8; 3]); 8] = [
    (0.00, [255, 214, 103]), // #ffd667 暖金高光（亮部）
    (0.12, [255, 165, 65]),  // #ffa541 亮橙
    (0.28, [255, 104, 83]),  // #ff6853 珊瑚橙红
    (0.44, [242, 62, 161]),  // #f23ea1 亮玫红（亮部）
    (0.60, [223, 59, 209]),  // #df3bd1 亮洋红紫
    (0.76, [182, 103, 255]), // #b667ff 亮紫
    (0.90, [132, 114, 254]), // #8472fe 亮蓝紫（亮部）
    (1.00, [91, 91, 240]),   // #5b5bf0 靛蓝收尾（提亮）
];

/// 主对角线渐变采样：`t` 已归一化到掩码实际覆盖的 [0,1]。
/// 段内用 smoothstep 缓动 + OKLab 线性插值，色标接缝处一阶连续，不会出现硬色带。
fn splash_gradient_rgb(t: f32) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    let mut idx = 0usize;
    while idx + 2 < SPLASH_GRADIENT_STOPS.len() && t > SPLASH_GRADIENT_STOPS[idx + 1].0 {
        idx += 1;
    }
    let (t0, c0) = SPLASH_GRADIENT_STOPS[idx];
    let (t1, c1) = SPLASH_GRADIENT_STOPS[idx + 1];
    let k = ((t - t0) / (t1 - t0).max(1e-6)).clamp(0.0, 1.0);
    let k = k * k * (3.0 - 2.0 * k);
    let a = srgb_to_oklab(c0);
    let b = srgb_to_oklab(c1);
    oklab_to_srgb([
        a[0] + (b[0] - a[0]) * k,
        a[1] + (b[1] - a[1]) * k,
        a[2] + (b[2] - a[2]) * k,
    ])
}

/// PonyFun 折角标结构掩码（28×22 方形像素）：'#' 为实心像素。
/// 由原 20×16 掩码双线性上采样到 28×22 后按 0.5 阈值重新取整 —— 分辨率提高 1.4×，
/// 折角处的 90° 台阶被压成更细的对角线，边缘看起来更顺滑（终端半块字符的物理上限内）。
const SPLASH_LOGO_SHAPE: [&str; 22] = [
    ".......######...............",
    "......#######...............",
    ".....#########..............",
    "....###########.............",
    "...####...######............",
    "...###.....######...........",
    "...###......#####...........",
    "..####.......####...........",
    ".###..........####..........",
    ".###..........#####.........",
    "####..........#####.........",
    ".........#####..........####",
    ".........#####..........###.",
    "..........####..........###.",
    "...........####.......####..",
    "...........#####......###...",
    "...........######.....###...",
    "............######...####...",
    ".............###########....",
    "..............#########.....",
    "...............#######......",
    "...............######.......",
];
const SPLASH_LOGO_ROWS: usize = SPLASH_LOGO_SHAPE.len();
const SPLASH_LOGO_COLS: usize = SPLASH_LOGO_SHAPE[0].len();
// 折角标覆盖的原始对角区间是 [2/19, 17/19]（分母为 20×16 母版四角）：
// 直接按 0.4x+0.6y 采样会浪费首尾各约 10% 色标，亮部被压在最边缘。
// 归一化后暖金高光与靛蓝收尾都真实落在图形上。
const SPLASH_T_MIN: f32 = 2.0 / 19.0;
const SPLASH_T_SPAN: f32 = 15.0 / 19.0;

/// 折角标坐标 → 归一化对角位置（0 = 暖金高光端，1 = 靛蓝收尾端）。
fn splash_diagonal_t(x: usize, y: usize) -> f32 {
    let raw = (x as f32 / (SPLASH_LOGO_COLS - 1).max(1) as f32) * 0.4
        + (y as f32 / (SPLASH_LOGO_ROWS - 1).max(1) as f32) * 0.6;
    ((raw - SPLASH_T_MIN) / SPLASH_T_SPAN).clamp(0.0, 1.0)
}

/// 图形协议品牌标的字符占位（与字符画横幅同尺寸：28 列 × 11 行）。
const SPLASH_IMAGE_COLS: u16 = SPLASH_LOGO_COLS as u16;
const SPLASH_IMAGE_ROWS: u16 = (SPLASH_LOGO_ROWS / 2) as u16;
/// 品牌标位图（由 `scripts/generate-splash-logo.py` 生成，画布底色不透明，
/// 用以完整覆盖其下方的字符画占位）。
const SPLASH_LOGO_PNG: &[u8] = include_bytes!("../../assets/splash-logo.png");

/// 初始化页品牌标的图形协议路径：仅在终端支持 sixel / kitty / iTerm2 时构建。
fn init_splash_image() -> Option<SplashImage> {
    let mut picker = splash_picker()?;
    if picker.protocol_type() == ProtocolType::Halfblocks {
        // halfblocks 即自绘半块字符，字符画版本的边缘与配色更可控，保留现路径。
        return None;
    }
    // 透明像素按画布底色合成，位图矩形与周围 Canvas 无缝衔接。
    picker.set_background_color(image::Rgba([18, 16, 14, 255]));
    let image = image::load_from_memory(SPLASH_LOGO_PNG).ok()?;
    Some(SplashImage {
        protocol: picker.new_resize_protocol(image),
        picker,
        cols: SPLASH_IMAGE_COLS,
        rows: SPLASH_IMAGE_ROWS,
    })
}

fn splash_picker() -> Option<Picker> {
    // PONY_TUI_SPLASH_IMAGE_PROTOCOL 为测试/手动覆盖入口：
    // auto（默认）走终端能力探测；kitty/sixel/iterm2 强制协议；off 禁用图形路径。
    match std::env::var("PONY_TUI_SPLASH_IMAGE_PROTOCOL").as_deref() {
        Ok("off") | Ok("halfblocks") => None,
        Ok("kitty") => Some(forced_splash_picker(ProtocolType::Kitty)),
        Ok("sixel") => Some(forced_splash_picker(ProtocolType::Sixel)),
        Ok("iterm2") => Some(forced_splash_picker(ProtocolType::Iterm2)),
        _ => {
            // 终端复用器内的图形协议直通依赖额外开关（tmux 的 allow-passthrough 等），
            // 且能力探测会经复用器转发而误判（实测 DA 回包让 picker 选了协议，但图片
            // 实际不显示 → 品牌标消失）。未显式指定协议时不冒险：回退字符画。
            // 复用器用户可用 env 强制开启：PONY_TUI_SPLASH_IMAGE_PROTOCOL=kitty|sixel|iterm2
            if ["TMUX", "STY", "ZELLIJ"]
                .iter()
                .any(|key| std::env::var_os(key).is_some())
            {
                return None;
            }
            let picker = Picker::from_query_stdio().ok()?;
            if picker.protocol_type() == ProtocolType::Halfblocks {
                return None;
            }
            Some(picker)
        }
    }
}

fn forced_splash_picker(protocol: ProtocolType) -> Picker {
    // 强制路径没有终端查询结果，用环境变量给出字符格像素尺寸（默认 8×16）。
    let font_size = std::env::var("PONY_TUI_SPLASH_FONT_PX")
        .ok()
        .and_then(|raw| {
            let (width, height) = raw.split_once('x')?;
            Some((width.parse().ok()?, height.parse().ok()?))
        })
        .unwrap_or((8, 16));
    let mut picker = Picker::from_fontsize(font_size);
    picker.set_protocol_type(protocol);
    picker
}

#[cfg(test)]
mod splash_gradient_tests {
    use super::*;

    fn rgb_at(t: f32) -> [u8; 3] {
        splash_gradient_rgb(t)
    }

    #[test]
    fn splash_logo_mask_shapes_up() {
        assert_eq!(SPLASH_LOGO_ROWS, 22, "折角标像素网格行数");
        assert_eq!(SPLASH_LOGO_COLS, 28, "折角标像素网格列数");
        for (r, row) in SPLASH_LOGO_SHAPE.iter().enumerate() {
            assert_eq!(row.len(), SPLASH_LOGO_COLS, "第 {r} 行掩码长度不符");
            assert!(
                row.bytes().all(|b| b == b'#' || b == b'.'),
                "第 {r} 行掩码只允许 '#' / '.'"
            );
        }
    }

    #[test]
    fn splash_logo_asset_decodes_for_image_protocol() {
        let img = image::load_from_memory(SPLASH_LOGO_PNG).expect("内嵌品牌标必须可解码");
        assert_eq!((img.width(), img.height()), (616, 484));
    }

    #[test]
    fn splash_image_placeholder_matches_banner_grid() {
        assert_eq!(SPLASH_IMAGE_COLS, SPLASH_LOGO_COLS as u16);
        assert_eq!(SPLASH_IMAGE_ROWS, (SPLASH_LOGO_ROWS / 2) as u16);
    }

    #[test]
    fn splash_gradient_covers_full_logo_diagonal() {
        // 归一化必须让折角标真实的两个对角极值恰好落在 t=0 / t=1；
        // 否则首尾亮部色标被裁掉（旧实现浪费约 10%）。
        let (mut min_t, mut max_t) = (f32::MAX, f32::MIN);
        for (y, row) in SPLASH_LOGO_SHAPE.iter().enumerate() {
            for (x, cell) in row.bytes().enumerate() {
                if cell != b'#' {
                    continue;
                }
                let t = splash_diagonal_t(x, y);
                min_t = min_t.min(t);
                max_t = max_t.max(t);
            }
        }
        assert!(
            min_t <= 0.01,
            "折角标最小对角位置应归一化到 0，实际 {min_t}"
        );
        assert!(
            max_t >= 0.99,
            "折角标最大对角位置应归一化到 1，实际 {max_t}"
        );
    }

    #[test]
    fn splash_gradient_keeps_bright_highlights() {
        // 「加入亮色」的可验证判据（OKLab 亮度）：起点暖金高光 ≥ 0.85
        // （旧版橙 #f97316 约 0.70），收尾靛蓝亮度高于旧版 #4f46e5（约 0.51）。
        assert_eq!(rgb_at(0.0), [255, 214, 103], "起点必须是暖金高光色标");
        assert_eq!(rgb_at(1.0), [91, 91, 240], "终点必须是提亮后的靛蓝收尾");
        let start = srgb_to_oklab(rgb_at(0.0));
        assert!(start[0] >= 0.85, "暖金高光亮度不足: {start:?}");
        let end = srgb_to_oklab(rgb_at(1.0));
        assert!(end[0] > 0.53, "靛蓝收尾未提亮: {end:?}");
        // 全程保持彩度，红→紫中段不塌成发灰暗带。
        let mut min_chroma = f32::MAX;
        for step in 0..=128 {
            let lab = srgb_to_oklab(rgb_at(step as f32 / 128.0));
            min_chroma = min_chroma.min((lab[1] * lab[1] + lab[2] * lab[2]).sqrt());
        }
        assert!(
            min_chroma >= 0.12,
            "渐变出现低彩度灰带: min_chroma={min_chroma:.3}"
        );
    }

    #[test]
    fn splash_gradient_steps_are_smooth() {
        // 1/256 采样下每通道跳变 ≤ 6/255：16 行画布逐行过渡，不出现硬色带。
        let mut prev = rgb_at(0.0);
        for step in 1..=256 {
            let t = step as f32 / 256.0;
            let cur = rgb_at(t);
            for (channel, (a, b)) in prev.iter().zip(cur.iter()).enumerate() {
                let delta = (*a as i32 - *b as i32).abs();
                assert!(
                    delta <= 6,
                    "t={t:.4} 通道 {channel} 跳变 {delta}: {prev:?} -> {cur:?}"
                );
            }
            prev = cur;
        }
    }
}

/// 彩色 init 头部（严格忠实还原 PonyFun / PonyGo 官方标志性折线双人字形图案）：
/// 采用 PonyFun 标志性的纯直线几何镜像折角结构（无内旋，干练通透），
/// 配色沿用原版主对角线流光走向（暖金 → 亮橙 → 珊瑚红 → 亮玫红 → 亮洋红紫
/// → 亮紫 → 亮蓝紫 → 靛蓝），并在 OKLab 空间插值保证过渡平滑；
/// 折角标以 28×22 方形像素网格渲染（分辨率较原 20×16 提高 1.4×），
/// 再用「▀」「▄」半块字符逐格双色落到字符网格上，右侧对齐品牌控制台信息列。
/// `draw_logo = false` 时只保留占位空白（由图形协议位图覆盖该矩形）。
fn pony_banner_lines(state: &TuiState, draw_logo: bool) -> Vec<Line<'static>> {
    // 渐变按折角标的归一化主对角线取样（见 `splash_diagonal_t` / `SPLASH_GRADIENT_STOPS`）。
    let pixel_color = |x: usize, y: usize| -> ratatui::style::Color {
        let rgb = splash_gradient_rgb(splash_diagonal_t(x, y));
        ratatui::style::Color::Rgb(rgb[0], rgb[1], rgb[2])
    };

    let filled = |row: usize, col: usize| -> bool {
        SPLASH_LOGO_SHAPE[row]
            .as_bytes()
            .get(col)
            .map(|c| *c == b'#')
            .unwrap_or(false)
    };

    let info = [
        Line::from(vec![
            Span::styled(
                "Pony",
                Style::default()
                    .fg(ratatui::style::Color::Rgb(249, 115, 22))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                " Agent",
                Style::default()
                    .fg(theme::INK)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" · v{}", env!("CARGO_PKG_VERSION")),
                Style::default().fg(theme::MUTED),
            ),
        ]),
        Line::from(vec![
            Span::styled("模型 · ", Style::default().fg(theme::MUTED)),
            Span::styled(state.model.clone(), Style::default().fg(theme::INK).add_modifier(Modifier::BOLD)),
            Span::styled(format!(" ({})", state.current_provider_id), Style::default().fg(theme::GHOST)),
        ]),
        Line::from(vec![
            Span::styled("工作区 · ", Style::default().fg(theme::MUTED)),
            Span::styled(state.workspace.display().to_string(), Style::default().fg(theme::MUTED)),
        ]),
    ];

    let mut out: Vec<Line<'static>> = Vec::new();
    // 信息列垂直居中于折角标的字符行（3 行信息）。
    let info_top = (SPLASH_LOGO_ROWS / 2).saturating_sub(info.len()) / 2;
    for line in 0..(SPLASH_LOGO_ROWS / 2) {
        let (top, bottom) = (line * 2, line * 2 + 1);
        let mut spans: Vec<Span<'static>> = vec![Span::raw(" ")];
        if draw_logo {
            for x in 0..SPLASH_LOGO_COLS {
                match (filled(top, x), filled(bottom, x)) {
                    (true, true) => {
                        let up = pixel_color(x, top);
                        let down = pixel_color(x, bottom);
                        spans.push(Span::styled("▀", Style::default().fg(up).bg(down)));
                    }
                    (true, false) => spans.push(Span::styled(
                        "▀",
                        Style::default().fg(pixel_color(x, top)),
                    )),
                    (false, true) => spans.push(Span::styled(
                        "▄",
                        Style::default().fg(pixel_color(x, bottom)),
                    )),
                    (false, false) => spans.push(Span::raw(" ")),
                }
            }
        } else {
            // 图形协议路径：品牌标矩形由位图覆盖，这里只铺占位空白保持信息列对齐。
            spans.push(Span::raw(" ".repeat(SPLASH_LOGO_COLS)));
        }
        if let Some(info_line) = line.checked_sub(info_top).and_then(|i| info.get(i)) {
            spans.push(Span::raw("    "));
            spans.extend(info_line.spans.clone());
        }
        out.push(Line::from(spans));
    }
    out
}

pub fn ascii_logo() -> Vec<Line<'static>> {
    // 空态由 ui() 组合：pony_banner_lines(state) + 引导块；
    // 此函数保留独立可测的引导块。所有文案缩进 2 列，与消息正文共享文本列。
    let mut lines = vec![
        Line::from(Span::styled("  描述一个任务开始，或输入命令：", ink_soft())),
        blank(),
    ];
    // 直接复用命令表，保证空态提示与斜杠菜单永不漂移。
    for (cmd, desc) in AVAILABLE_COMMANDS.iter().take(4) {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(pad_display(cmd, 11), ink_soft()),
            Span::styled(desc.to_string(), muted()),
        ]));
    }
    lines.push(blank());
    lines.push(Line::from(Span::styled("  键入 / 查看全部命令", faint())));
    lines
}

pub struct DynamicProviderResolver {
    pub gateway_url: String,
    pub api_key: Option<String>,
    pub model: String,
    pub registry: ProviderRegistryStore,
}

impl ProviderSelectionResolver for DynamicProviderResolver {
    fn resolve_provider_selection(
        &self,
        provider_id: Option<&str>,
        model_id: Option<&str>,
    ) -> ResolvedProviderSelection {
        // wave-36b（修订）：注册表优先——只要 provider_id 能在注册表命中（含 "ponyllm"），
        // 一律走 registry.resolve_selection（base_url/api_key_value 全部来自注册表配置）；
        // 仅当注册表无该 provider 时才回落 gateway_url + 环境变量 key 的兜底分支。
        // 旧实现 pid=="ponyllm" 时强制硬编码分支，忽略注册表 key/base_url → 必然 401。
        if let Some(pid) = provider_id {
            let view = self.registry.load_view();
            if view.providers.iter().any(|p| p.id == pid) {
                return self.registry.resolve_selection(Some(pid), model_id);
            }
        } else if self.registry.load_view().selected_provider_id.is_some() {
            return self.registry.resolve_selection(None, model_id);
        }

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
            model: model_id.unwrap_or(&self.model).to_string(),
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

struct TuiTurnSink {
    tx: Sender<UiEvent>,
}

/// 每轮 sink/emitter 的成对生命周期；注册发生在 turn id 对事件循环发布之前。
struct TuiTurnSinkGuard {
    control_plane: Arc<HostControlPlane>,
    turn_id: String,
}

impl TuiTurnSinkGuard {
    fn new(
        control_plane: &Arc<HostControlPlane>,
        turn_id: &str,
        sink: Arc<TuiTurnSink>,
    ) -> Self {
        control_plane.register_turn_event_sink(turn_id, sink);
        Self {
            control_plane: Arc::clone(control_plane),
            turn_id: turn_id.to_string(),
        }
    }
}

impl Drop for TuiTurnSinkGuard {
    fn drop(&mut self) {
        self.control_plane.unregister_turn_event_sink(&self.turn_id);
    }
}

/// C11：从 `catch_unwind` 的负载里取出可读原因（`&str` / `String` / 未知名）。
///
/// panic 是终态的一种：文案只给一句摘要，原始 panic 栈仍走 stderr → 运行日志。
fn panic_payload_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_string()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "未知 panic 负载".to_string()
    }
}

/// C10：轮次线程存活标志的 RAII 守卫（派发闸门的第 4 判据的真值）。
///
/// 契约修订（2026-11-06 Lead）：判据是「**本会话是否真有一个轮次线程活着**」，
/// 不是「有没有 armed goal」——goal armed 但续轮没在跑时，只看 goal 存在会让排队消息
/// 永不自动派发（用户感知为卡住）。
///
/// 不变量：`Drop` 在**正常返回与 panic 展开**两条路径上都会执行 ⇒ 标志绝不会永真
/// （与 C11 的轮次线程 panic 兜底同一条不变量）。
struct TurnLoopInFlightGuard {
    flag: Arc<AtomicBool>,
}

impl TurnLoopInFlightGuard {
    /// 进入轮次线程即刻置位；返回的 guard 生命周期 = 线程体（含 goal 续轮循环）生命周期。
    fn enter(flag: Arc<AtomicBool>) -> Self {
        flag.store(true, Ordering::SeqCst);
        Self { flag }
    }
}

impl Drop for TurnLoopInFlightGuard {
    fn drop(&mut self) {
        self.flag.store(false, Ordering::SeqCst);
    }
}

impl TurnEventSink for TuiTurnSink {
    fn emit(&self, name: &str, payload: TurnStreamEvent) {
        // P0（单一附着点消费）：凡载荷携带快照的事件（hop_complete / output_end / completed /
        // phase_changed …）都先投递快照，驱动指标行实时更新；`metrics` 为 `None` 时不投递，
        // 旧事件路径（I7）行为逐字不变。此处必须在 `match name` **之前**，否则会漏掉非 usage 事件。
        if let Some(metrics) = payload.metrics.as_ref() {
            let _ = self.tx.send(UiEvent::StreamMetrics(metrics.clone()));
        }
        match name {
            // P0：轮次起点 ⇒ 事件驱动的复位（详见 `UiEvent::TurnStarted`）。
            // 必须在轮内任何 `metrics` 事件之前投递：`turn:started` 是该轮首个事件。
            "turn:started" => {
                let _ = self.tx.send(UiEvent::TurnStarted);
            }
            "turn:delta" => {
                if let Some(text) = payload.text {
                    // wave-tui-session-resilience C1：上游 DSML 工具调用协议原文
                    // 不得进入消息区；跨 chunk 的半截标记也不交给用户。
                    let (visible, _incomplete) = sanitize_model_protocol_text(&text);
                    if !visible.is_empty() {
                        let _ = self.tx.send(UiEvent::StreamDelta(visible));
                    }
                }
            }
            "turn:tool" => {
                if let Some(activities) = &payload.tool_activities {
                    if let Some(act) = activities.last() {
                        let is_running = act.status == "running" || act.status == "planned";
                        let desc_text = if !act.description.trim().is_empty() {
                            Some(act.description.trim().to_string())
                        } else {
                            plain_text_detail(
                                act.arguments_text.as_deref(),
                                act.result_text.as_deref(),
                            )
                        };
                        let short_desc = desc_text.and_then(|d| {
                            let t = d.trim();
                            if t.is_empty() {
                                None
                            } else {
                                Some(truncate_chars(t, 60))
                            }
                        });
                        let formatted = format_tool_display(
                            &act.name,
                            act.display_name_zh.as_deref(),
                            short_desc.as_deref(),
                            Some(&act.status),
                        );
                        if is_running {
                            let _ = self.tx.send(UiEvent::StreamToolCall(formatted));
                        } else {
                            let _ = self.tx.send(UiEvent::StreamToolResult(formatted));
                        }
                        return;
                    }
                }
                if let Some(text) = payload.text {
                    let formatted = format_tool_display("Tool", None, Some(&text), None);
                    let _ = self.tx.send(UiEvent::StreamToolCall(formatted));
                }
            }
            "turn:tool_call" => {
                if let Some(activities) = &payload.tool_activities {
                    if let Some(act) = activities.last() {
                        let formatted = format_tool_display(
                            &act.name,
                            act.display_name_zh.as_deref(),
                            Some(&act.description),
                            Some(&act.status),
                        );
                        let _ = self.tx.send(UiEvent::StreamToolCall(formatted));
                        return;
                    }
                }
                if let Some(text) = payload.text {
                    let formatted = format_tool_display("Tool", None, Some(&text), None);
                    let _ = self.tx.send(UiEvent::StreamToolCall(formatted));
                }
            }
            "turn:tool_result" => {
                let (name, display_zh, status, detail) = payload
                    .tool_activities
                    .as_ref()
                    .and_then(|a| a.last())
                    .map(|act| {
                        let desc = if !act.description.trim().is_empty() {
                            act.description.clone()
                        } else {
                            plain_text_detail(
                                act.arguments_text.as_deref(),
                                act.result_text.as_deref(),
                            )
                            .unwrap_or_default()
                        };
                        (
                            act.name.clone(),
                            act.display_name_zh.clone(),
                            act.status.clone(),
                            desc,
                        )
                    })
                    .unwrap_or_else(|| ("Tool".to_string(), None, "ok".to_string(), String::new()));
                let short_detail = truncate_chars(&detail, 60);
                let formatted = format_tool_display(&name, display_zh.as_deref(), Some(&short_detail), Some(&status));
                let _ = self.tx.send(UiEvent::StreamToolResult(formatted));
            }
            "turn:suspended" => {
                // wave-43：Ask/审批把轮次挂起——主循环据此拉取真实待决请求并打开对话框。
                // run_id 以控制面快照为准（此处不猜），因此这里只发信号。
                let _ = self.tx.send(UiEvent::TurnSuspended { run_id: None });
            }
            "turn:stop_requested" => {
                // wave-60 §C2/C9：请求受理事件 ≠ 终态；只驱动「正在停止…」（不置 is_streaming=false）。
                let _ = self.tx.send(UiEvent::StopRequested {
                    turn_id: payload.turn_id,
                });
            }
            "turn:cancelled" => {
                // wave-60 §C9：终态由该事件驱动（此前被 `_ => {}` 静默丢弃 → 只能靠本地伪造终态）。
                let _ = self.tx.send(UiEvent::StopTerminal {
                    turn_id: payload.turn_id,
                    stop_reason: payload.stop_reason,
                });
            }
            "turn:completed" | "turn:output_end" => {
                // wave-39 R8'（契约 v2）：usage 只由 turn:output_end 发射；turn:completed
                // 仅收尾，避免同一 persisted token 值在新旧轮交叠时被二次 apply 而永久多算。
                if let Some(event) = stream_usage_event(name, &payload) {
                    let _ = self.tx.send(event);
                }
                let _ = self.tx.send(UiEvent::StreamCompleted);
            }
            "turn:failed" => {
                let err = payload.error.unwrap_or_else(|| "Unknown error".to_string());
                let _ = self.tx.send(UiEvent::StreamFailed(err));
            }
            _ => {}
        }
    }
}

/// wave-39 R8' / wave-41 S1：仅 `turn:output_end` 是 usage 权威事件；
/// 输入 token 遵循 ADR 0035 S1 规范：多跳时从 provider_call_records 末条提取单次上下文输入，
/// 严禁跨跳跨轮求和；无 records 时回退 payload.input_tokens。
pub fn stream_usage_event(event_name: &str, payload: &TurnStreamEvent) -> Option<UiEvent> {
    if event_name != "turn:output_end" {
        return None;
    }
    if payload.input_tokens.is_none()
        && payload.output_tokens.is_none()
        && payload.cache_hit_input_tokens.is_none()
        && payload.total_tokens.is_none()
        && payload.provider_call_records.is_none()
    {
        return None;
    }

    // ADR 0035 S1：优先从末次 provider call record 提取真实单次上下文输入
    let context_input = payload
        .provider_call_records
        .as_ref()
        .and_then(|records| records.last())
        .and_then(|rec| rec.input_tokens)
        .or(payload.input_tokens)
        .unwrap_or(0);

    Some(UiEvent::StreamUsage {
        input_tokens: context_input,
        output_tokens: payload.output_tokens.unwrap_or(0),
        cache_hit_input_tokens: payload.cache_hit_input_tokens.unwrap_or(0),
        total_tokens: payload.total_tokens.unwrap_or(0),
    })
}

/// wave-35c / 契约 P-1：解析当前活跃 provider id。
///
/// 口径改为 [`pick_active_provider_id`]：**state.current_provider_id 优先**（用户刚在 /provider
/// 里选的必须立刻生效），其后注册表选中项，最后首个 provider。注册表句柄取自 state
/// （路径可注入），不再在热路径上硬编码用户目录。
pub fn resolve_active_provider_id(state: &TuiState) -> Option<String> {
    let view = state.registry_store().load_view();
    let provider_ids: Vec<&str> = view.providers.iter().map(|p| p.id.as_str()).collect();
    pick_active_provider_id(
        Some(state.current_provider_id.as_str()),
        view.selected_provider_id.as_deref(),
        &provider_ids,
    )
}

/// wave-35c：启动同步——把 TuiState 的 current_provider_id / model 对齐注册表
/// 选中项与 selected_model_id，使 header/状态一致，并保证后续 discovery 能命中。
// 已于 wave-slash-1 批6 接线（`run_tui` 启动时调用）：注册表里持久化的选中 provider / model
// 必须真正生效，否则 header 显示与发送口径分叉（此前只有 /models、/provider 面板读注册表）。
fn sync_state_with_registry(state: &mut TuiState) {
    if let Some(id) = resolve_active_provider_id(state) {
        state.current_provider_id = id;
    }
    let registry = state.registry_store();
    let view = registry.load_view();
    if let Some(p) = view.providers.iter().find(|p| p.id == state.current_provider_id) {
        if let Some(sel) = p.selected_model_id.as_ref() {
            if p.models.iter().any(|m| &m.id == sel) {
                state.model = sel.clone();
            }
        }
    }
}

/// wave-38 R1.4 可测接缝：由 `TuiState` 组装发送给 `start_turn_stream` 的唯一 `TurnInput`。
///
/// 契约口径（逐字对齐现发送点，勿增删字段语义）：
/// - `workspace_id` 取 `state.active_workspace_id.clone()`（本期修复的唯一目的字段）；
/// - 其余字段与现发送点一致：`message` 透传、`display_message: None`、
///   `provider_id: Some(provider_id)`、`model_id: Some(model_id)`、
///   `reasoning_effort: state.reasoning_effort.clone()`、`workspace_mode: None`、
///   `session_id: Some(state.session_id.clone())`、`node_id: None`、
///   `history`/`images` 为空 Vec、`plan_execution_approval: None`。
pub fn build_turn_input(
    state: &TuiState,
    message: String,
    provider_id: String,
    model_id: String,
) -> TurnInput {
    TurnInput {
        message,
        display_message: None,
        provider_id: Some(provider_id),
        model_id: Some(model_id),
        reasoning_effort: state.reasoning_effort.clone(),
        workspace_mode: None,
        session_id: Some(state.session_id.clone()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: state.active_workspace_id.clone(),
        plan_execution_approval: None,
        tool_authorization_mode: None,
    }
}

/// wave-38 D1 纯函数：恢复历史会话时的 workspace 绑定解析。
///
/// 契约口径（§5.D1，逐条对齐 E8–E12）：
/// - `session_workspace_id` 为 **已注册** id（命中 `registered_workspace_ids`）→
///   返回 `(Some(id), env cwd 对应的 PathBuf)`；`session_env_cwd` 缺失/空白 → PathBuf 回退
///   `startup_workspace_path`；
/// - `session_workspace_id` 为 `None` / 空串 / 死 id（未注册）→ 回退启动目录绑定
///   `(startup_workspace_id.map(str::to_string), startup_workspace_path.to_path_buf())`；
/// - 不产生任何写入/迁移：会话已有归属原样保留（跨表面静默迁移仍被禁止）。
pub fn resolve_resumed_workspace_binding(
    session_workspace_id: Option<&str>,
    session_env_cwd: Option<&str>,
    registered_workspace_ids: &[String],
    startup_workspace_id: Option<&str>,
    startup_workspace_path: &std::path::Path,
) -> (Option<String>, PathBuf) {
    let registered_id = session_workspace_id
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .filter(|id| registered_workspace_ids.iter().any(|registered| registered == id));
    match registered_id {
        Some(id) => {
            // 已注册归属：header root 取会话 env cwd；缺失/空白才回退启动目录。
            let path = session_env_cwd
                .map(str::trim)
                .filter(|cwd| !cwd.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| startup_workspace_path.to_path_buf());
            (Some(id.to_string()), path)
        }
        // 无归属 / 空 id / 死 id：复用启动目录绑定（死 id 不得沿用会话残留 env cwd）。
        None => (
            startup_workspace_id.map(str::to_string),
            startup_workspace_path.to_path_buf(),
        ),
    }
}

fn build_control_plane(workspace: &std::path::Path, gateway_url: &str, model: &str) -> HostControlPlane {
    // 彻底复用桌面端体系：直接使用与桌面端相同的生产环境默认 SessionStore（默认加载与写入 ~/.pony-agent/sessions.sqlite 或 sessions.json）
    // 使得 TUI 创建和更新的会话在桌面端无缝可见与互通。
    let api_key = std::env::var("PONYLLM_API_KEY")
        .or_else(|_| std::env::var("PONY_API_KEY"))
        .ok()
        .or_else(|| {
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

    let resolver = DynamicProviderResolver {
        gateway_url: gateway_url.to_string(),
        api_key,
        model: model.to_string(),
        registry: ProviderRegistryStore::new(),
    };

    let runtime_builder = AgentRuntimeBuilder::desktop()
        .session_store(SessionStore::new())
        .provider_resolver(Box::new(resolver))
        .workspace_root(workspace.to_path_buf());

    HostControlPlaneBuilder::desktop()
        .runtime_builder(runtime_builder)
        .build()
}

// ─────────────────────────────────────────────────────────────────────────────
// Design tokens —— 极简克制的高级感：
//   · 单色为骨：一块近乎无色的暖黑画布 + 暖白文字，拒绝高饱和撞色；
//   · 单一强调色（骨色/bone）只用于「光标、选中、进行中」三处语义；
//   · 层级靠留白与字重，而非描边、胶囊、色块；
//   · 分隔只用 1px 发丝线（hairline），永不画整块边框卡片。
// ─────────────────────────────────────────────────────────────────────────────
mod theme {
    use ratatui::style::Color;

    pub const CANVAS: Color = Color::Rgb(18, 16, 14);
    pub const SURFACE: Color = Color::Rgb(27, 25, 22);
    pub const SURFACE_SEL: Color = Color::Rgb(40, 37, 32);
    /// 代码块材质底色：加强深沉温暖的陶土黑底，对比鲜明同时与整套陶土暖黑画布自然呼应。
    pub const CODE_FILL: Color = Color::Rgb(36, 25, 22);
    pub const INK: Color = Color::Rgb(232, 229, 224);
    pub const INK_SOFT: Color = Color::Rgb(203, 200, 195);
    pub const MUTED: Color = Color::Rgb(162, 155, 146);
    pub const FAINT: Color = Color::Rgb(124, 118, 110);
    /// wave-39 v3 R10：指标行专用，比 FAINT 再暗一档（终端无法改字号，以低对比度表达「更小更弱」）。
    pub const GHOST: Color = Color::Rgb(104, 98, 90);
    pub const HAIRLINE: Color = Color::Rgb(52, 47, 41);
    /// 强调色：更深沉醒目的陶土色（深赤陶色 Terracotta，温暖有质感）。
    pub const ACCENT: Color = Color::Rgb(214, 114, 80);
    /// 角色竖条等「结构性强调」用次级陶土色。
    pub const ACCENT_DIM: Color = Color::Rgb(180, 92, 62);
    /// 强化强调色：行内 **强调** 专用，明艳饱满的陶土砖红色，凸显视觉重点。
    pub const EMPHASIS: Color = Color::Rgb(232, 128, 92);
    /// 明亮陶土色（Bright Terracotta）：提问标题、重点问题专用，温润明亮且醒目。
    pub const TERRACOTTA_BRIGHT: Color = Color::Rgb(242, 142, 102);
    pub const SUCCESS: Color = Color::Rgb(140, 172, 137);
    pub const DANGER: Color = Color::Rgb(199, 130, 125);
    pub const WARN: Color = Color::Rgb(199, 176, 126);

    // ── VS Code Dark+ / Modern 经典语法配色 ──
    pub const VSCODE_KEYWORD: Color = Color::Rgb(197, 134, 192);   // 紫色 (fn, let, pub, return, if, match)
    pub const VSCODE_CONTROL: Color = Color::Rgb(224, 108, 117);   // 控流深粉
    pub const VSCODE_STRING: Color = Color::Rgb(206, 145, 120);    // 橙粉色 "string"
    pub const VSCODE_COMMENT: Color = Color::Rgb(106, 153, 85);    // 橄榄绿 // comment
    pub const VSCODE_TYPE: Color = Color::Rgb(78, 201, 176);       // 青碧色 Option, Result, Vec, String
    pub const VSCODE_NUMBER: Color = Color::Rgb(181, 206, 168);    // 淡青绿 123, 0.42
    pub const VSCODE_VARIABLE: Color = Color::Rgb(156, 220, 254);  // 天蓝色 self, args, variables
    pub const VSCODE_ATTR: Color = Color::Rgb(86, 156, 214);       // 鲜亮蓝 #[], @decorator
}

/// 左右统一沟槽宽度（视觉留白，禁止内容贴边）。
const GUTTER: u16 = 3;

/// 消息轨道：左侧 2 列承载角色/工具字形，所有正文从轨道之后起排。
/// 这样用户正文、助手正文、工具文本共享同一条文本基线。
const RAIL: usize = 2;

/// 极简字形集：只用三种标记，避免图标噪音。
const GLYPH_RUNNING: &str = "◦";
const GLYPH_DONE: &str = "✓";
const GLYPH_FAIL: &str = "✕";
const CARET: &str = "▏";

fn ink() -> Style {
    Style::default().fg(theme::INK)
}

fn ink_soft() -> Style {
    Style::default().fg(theme::INK_SOFT)
}

fn muted() -> Style {
    Style::default().fg(theme::MUTED)
}

fn faint() -> Style {
    Style::default().fg(theme::FAINT)
}

/// 东亚宽字符按 2 列计宽，保证中文/emoji 折行与滚动偏移精确。
fn char_width(c: char) -> usize {
    let cp = c as u32;
    let wide = matches!(cp,
        0x1100..=0x115F
            | 0x2E80..=0x303E
            | 0x3041..=0x33FF
            | 0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xA000..=0xA4CF
            | 0xA960..=0xA97F
            | 0xAC00..=0xD7A3
            | 0xF900..=0xFAFF
            | 0xFE10..=0xFE19
            | 0xFE30..=0xFE6F
            | 0xFF00..=0xFF60
            | 0xFFE0..=0xFFE6
            | 0x1F300..=0x1F64F
            | 0x1F900..=0x1F9FF
            | 0x20000..=0x3FFFD);
    if wide {
        2
    } else {
        1
    }
}

pub fn str_width(text: &str) -> usize {
    text.chars().map(char_width).sum()
}

/// 行首禁则：这些标点不允许出现在行首（悬挂到上一行末尾）。
fn is_no_line_start(c: char) -> bool {
    matches!(
        c,
        '，' | '。'
            | '、'
            | '；'
            | '：'
            | '？'
            | '！'
            | '）'
            | '】'
            | '》'
            | '」'
            | '』'
            | '…'
            | ','
            | '.'
            | ';'
            | ':'
            | '?'
            | '!'
            | ')'
            | ']'
            | '%'
    )
}

/// 1px 发丝分隔线（不是边框，只是一条极淡的横线）。
fn hairline(width: usize) -> Line<'static> {
    Line::from(Span::styled(
        "▁".repeat(width.max(1)),
        Style::default().fg(theme::HAIRLINE),
    ))
}

fn blank() -> Line<'static> {
    Line::raw("")
}

/// 按显示宽度折行，保留 span 样式；用于所有正文（CJK 不会错位）。
fn wrap_line(line: &Line<'static>, width: usize) -> Vec<Line<'static>> {
    let width = width.max(1);
    if line.spans.is_empty() {
        return vec![blank()];
    }
    let mut out: Vec<Line<'static>> = Vec::new();
    let mut current: Vec<Span<'static>> = Vec::new();
    let mut col = 0usize;
    for span in &line.spans {
        if span.content.is_empty() {
            continue;
        }
        let style = span.style;
        let mut buf = String::new();
        for ch in span.content.chars() {
            let w = char_width(ch);
            if col + w > width && col > 0 && !is_no_line_start(ch) {
                if !buf.is_empty() {
                    current.push(Span::styled(std::mem::take(&mut buf), style));
                }
                out.push(Line::from(std::mem::take(&mut current)));
                col = 0;
            }
            buf.push(ch);
            col += w;
        }
        if !buf.is_empty() {
            current.push(Span::styled(buf, style));
        }
    }
    if !current.is_empty() || out.is_empty() {
        out.push(Line::from(current));
    }
    out
}

/// 保证末尾恰好有 n 个空行（用于区分「段间距 1 行」与「轮间距 2 行」的节拍）。
fn push_gap(lines: &mut Vec<Line<'static>>, n: usize) {
    if lines.is_empty() {
        return;
    }
    let mut blanks = 0usize;
    for line in lines.iter().rev() {
        if line.spans.iter().all(|s| s.content.trim().is_empty()) {
            blanks += 1;
        } else {
            break;
        }
    }
    for _ in blanks..n {
        lines.push(blank());
    }
}

fn push_wrapped(lines: &mut Vec<Line<'static>>, line: Line<'static>, width: usize) {
    let mut wrapped = wrap_line(&line, width);
    // 行首禁则（跨行）：模型自带换行后若行首是标点，悬挂回上一行行尾。
    if !lines.is_empty() {
        let mut pulled: Option<(char, Style)> = None;
        if let Some(first) = wrapped.first_mut() {
            if let Some(span) = first.spans.first_mut() {
                if let Some(ch) = span.content.chars().next() {
                    if is_no_line_start(ch) {
                        span.content = span.content.chars().skip(1).collect();
                        let style = span.style;
                        pulled = Some((ch, style));
                    }
                }
            }
        }
        if let Some((ch, style)) = pulled {
            if let Some(prev) = lines.last_mut() {
                if let Some(last) = prev.spans.last_mut() {
                    let merged = format!("{}{}", last.content, ch);
                    last.content = merged.into();
                } else {
                    prev.spans.push(Span::styled(ch.to_string(), style));
                }
            }
        }
    }
    lines.extend(wrapped);
}

/// 只在不重复的前提下插入空行，保持「行间距呼吸」而不产生连续空行噪音。
fn push_blank(lines: &mut Vec<Line<'static>>) {
    if let Some(last) = lines.last() {
        let is_blank = last.spans.iter().all(|s| s.content.trim().is_empty());
        if !is_blank {
            lines.push(blank());
        }
    }
}

fn line_display_width(line: &Line<'static>) -> usize {
    line.spans.iter().map(|s| str_width(&s.content)).sum()
}

fn pad_line_with_bg(mut line: Line<'static>, width: usize, bg: ratatui::style::Color) -> Line<'static> {
    let used = line_display_width(&line);
    if used < width {
        line.spans.push(Span::styled(
            " ".repeat(width - used),
            Style::default().bg(bg),
        ));
    }
    line
}

/// 轨道缩进折行：首行用 first 前缀、续行用 cont 前缀，正文宽度统一。
fn push_prefixed(
    lines: &mut Vec<Line<'static>>,
    first: Vec<Span<'static>>,
    cont: Vec<Span<'static>>,
    body: Line<'static>,
    body_width: usize,
) {
    for (i, line) in wrap_line(&body, body_width.max(1)).into_iter().enumerate() {
        let mut spans = if i == 0 { first.clone() } else { cont.clone() };
        spans.extend(line.spans);
        lines.push(Line::from(spans));
    }
}

fn blank_rail(style: Style) -> Vec<Span<'static>> {
    vec![Span::styled(" ".repeat(RAIL), style)]
}

/// 代码块色带的右端内缩量：色块不贴内容列右缘，留出呼吸。
const CODE_BAND_INSET: usize = 2;
/// 色带内部左右内边距：代码文字与色块边缘之间留 3 列内边距，提升视觉质感。
const CODE_BAND_PAD: usize = 3;

/// VS Code 风格的高亮代码行着色器（无重依赖的极速词法识别）
pub fn highlight_code_line(raw: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let trimmed = raw.trim_start();
    let leading_spaces = raw.len() - trimmed.len();
    if leading_spaces > 0 {
        spans.push(Span::raw(" ".repeat(leading_spaces)));
    }

    // 单行注释
    if trimmed.starts_with("//") || trimmed.starts_with('#') {
        spans.push(Span::styled(
            trimmed.to_string(),
            Style::default().fg(theme::VSCODE_COMMENT).add_modifier(Modifier::ITALIC),
        ));
        return spans;
    }

    // 属性宏例如 #[derive(...)]
    if trimmed.starts_with("#[") || trimmed.starts_with('@') {
        spans.push(Span::styled(
            trimmed.to_string(),
            Style::default().fg(theme::VSCODE_ATTR),
        ));
        return spans;
    }

    let mut rest = trimmed;
    while !rest.is_empty() {
        // 查找字符串 "..." 或 '...'
        if let Some(quote_idx) = rest.find('"') {
            if let Some(next_quote) = rest[quote_idx + 1..].find('"') {
                let before = &rest[..quote_idx];
                if !before.is_empty() {
                    spans.extend(tokenize_code_segment(before));
                }
                let str_content = &rest[quote_idx..=quote_idx + 1 + next_quote];
                spans.push(Span::styled(
                    str_content.to_string(),
                    Style::default().fg(theme::VSCODE_STRING),
                ));
                rest = &rest[quote_idx + 2 + next_quote..];
                continue;
            }
        }
        spans.extend(tokenize_code_segment(rest));
        break;
    }
    spans
}

fn tokenize_code_segment(segment: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut word = String::new();

    let flush_word = |spans: &mut Vec<Span<'static>>, w: &mut String| {
        if w.is_empty() {
            return;
        }
        let token = w.clone();
        w.clear();

        // 关键字
        let is_keyword = matches!(
            token.as_str(),
            "fn" | "let" | "mut" | "pub" | "struct" | "enum" | "impl" | "use" | "mod"
                | "const" | "static" | "type" | "trait" | "async" | "await" | "return"
                | "if" | "else" | "match" | "for" | "in" | "while" | "loop" | "break"
                | "continue" | "def" | "class" | "import" | "from" | "as" | "function"
                | "var" | "val" | "export" | "default"
        );
        if is_keyword {
            spans.push(Span::styled(
                token,
                Style::default().fg(theme::VSCODE_KEYWORD).add_modifier(Modifier::BOLD),
            ));
            return;
        }

        // 控流特殊词
        if matches!(token.as_str(), "mut" | "ref") {
            spans.push(Span::styled(
                token,
                Style::default().fg(theme::VSCODE_CONTROL),
            ));
            return;
        }

        // 布尔与空值
        if matches!(token.as_str(), "true" | "false" | "None" | "Some" | "Ok" | "Err" | "null" | "nil" | "self" | "Self") {
            spans.push(Span::styled(
                token,
                Style::default().fg(theme::VSCODE_VARIABLE).add_modifier(Modifier::ITALIC),
            ));
            return;
        }

        // 类型 (首字母大写)
        if token.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
            spans.push(Span::styled(
                token,
                Style::default().fg(theme::VSCODE_TYPE),
            ));
            return;
        }

        // 数字
        if token.chars().all(|c| c.is_ascii_digit() || c == '.' || c == '_') {
            spans.push(Span::styled(
                token,
                Style::default().fg(theme::VSCODE_NUMBER),
            ));
            return;
        }

        // 常规函数名 / 标识符：如果后面紧跟着括号，或者作为常用命名
        spans.push(Span::styled(token, Style::default().fg(theme::VSCODE_VARIABLE)));
    };

    for c in segment.chars() {
        if c.is_alphanumeric() || c == '_' {
            word.push(c);
        } else {
            flush_word(&mut spans, &mut word);
            let symbol_style = match c {
                '(' | ')' | '{' | '}' | '[' | ']' => Style::default().fg(theme::VSCODE_ATTR),
                ':' | ';' | ',' | '.' => Style::default().fg(theme::FAINT),
                '=' | '+' | '-' | '*' | '/' | '%' | '&' | '|' | '!' | '<' | '>' | '^' => {
                    Style::default().fg(theme::VSCODE_CONTROL)
                }
                _ => Style::default().fg(theme::INK_SOFT),
            };
            spans.push(Span::styled(c.to_string(), symbol_style));
        }
    }
    flush_word(&mut spans, &mut word);
    spans
}

/// 向代码块色带追加一行整行背景留白，形成顶部或底部的代码块内边距（内边距仍带有 CODE_FILL 底色）。
fn push_code_band_padding(lines: &mut Vec<Line<'static>>, width: usize) {
    let band_w = width
        .saturating_sub(RAIL + CODE_BAND_INSET)
        .max(CODE_BAND_PAD * 2 + 1);
    let mut spans = blank_rail(Style::default());
    spans.push(Span::styled(" ", Style::default().bg(theme::CODE_FILL)));
    lines.push(pad_line_with_bg(
        Line::from(spans),
        RAIL + band_w,
        theme::CODE_FILL,
    ));
}

/// 代码块材质：语言标签行与代码行共用同一条 `CODE_FILL` 陶土暗底色带，无竖线、无边框。
/// 色带从轨道后起排，右端内缩 `CODE_BAND_INSET` 列；超出宽度的代码按显示宽度折行，
/// 内部增加 CODE_BAND_PAD 列内边距，提升呼吸感与专业代码编辑器质感。
fn push_code_band_line(lines: &mut Vec<Line<'static>>, body: Line<'static>, width: usize) {
    let band_w = width
        .saturating_sub(RAIL + CODE_BAND_INSET)
        .max(CODE_BAND_PAD * 2 + 1);
    let body_w = band_w.saturating_sub(CODE_BAND_PAD * 2).max(1);
    let bg = Style::default().bg(theme::CODE_FILL);
    for wrapped in wrap_line(&body, body_w) {
        let mut spans = blank_rail(Style::default());
        spans.push(Span::styled(" ".repeat(CODE_BAND_PAD), bg));
        spans.extend(
            wrapped
                .spans
                .into_iter()
                .map(|span| Span::styled(span.content, span.style.bg(theme::CODE_FILL))),
        );
        lines.push(pad_line_with_bg(
            Line::from(spans),
            RAIL + band_w,
            theme::CODE_FILL,
        ));
    }
}

/// 用户消息：低对比表面块 + 左侧骨色竖条。用「材质」而不是文字标签区分角色。
/// 块内含 1 行上/下呼吸留白，正文回到与助手/工具相同的文本列。
fn push_user_block(lines: &mut Vec<Line<'static>>, content: &str, width: usize) {
    // 用户轮**不再画左侧竖线**：竖线会被终端选中复制带走，粘到剪贴板里污染正文。
    // 仍然用 `RAIL` 列缩进 + SURFACE 底色区分角色，复制出来的就是纯正文。
    let bg = theme::SURFACE;
    let fill_style = Style::default().bg(bg);
    let gutter = || Span::styled(" ".repeat(RAIL), fill_style);
    let body_w = width.saturating_sub(RAIL + 1).max(1);

    let block_pad = |lines: &mut Vec<Line<'static>>| {
        lines.push(pad_line_with_bg(Line::from(vec![gutter()]), width, bg));
    };

    block_pad(lines);
    let mut wrote_body = false;
    for raw in content.lines() {
        let body = Line::from(Span::styled(
            raw.to_string(),
            Style::default().fg(theme::INK_SOFT).bg(bg),
        ));
        for wrapped in wrap_line(&body, body_w) {
            let mut spans = vec![gutter()];
            spans.extend(
                wrapped
                    .spans
                    .into_iter()
                    .map(|s| Span::styled(s.content, s.style.bg(bg))),
            );
            lines.push(pad_line_with_bg(Line::from(spans), width, bg));
            wrote_body = true;
        }
    }
    if !wrote_body {
        let mut spans = vec![gutter()];
        spans.push(Span::styled(String::new(), Style::default().bg(bg)));
        lines.push(pad_line_with_bg(Line::from(spans), width, bg));
    }
    block_pad(lines);
}

/// 工具行分层：动作名保留可读性，参数/结果退到次级墨色。
fn tool_line_spans(
    glyph: &str,
    glyph_style: Style,
    head_style: Style,
    content: &str,
) -> (Vec<Span<'static>>, Vec<Span<'static>>) {
    // 工具描述常带行内 markdown 记号；工具行不是富文本，直接去掉反引号/加粗星号。
    let cleaned = content.replace('`', "").replace("**", "");
    let (head, detail) = cleaned
        .split_once(" · ")
        .or_else(|| content.split_once("  "))
        .map(|(h, d)| (h.to_string(), d.to_string()))
        .unwrap_or_else(|| (content.to_string(), String::new()));
    let prefix = vec![Span::styled(format!("{} ", glyph), glyph_style)];
    let mut body = vec![Span::styled(head, head_style)];
    if !detail.is_empty() {
        // 参数/结果属于中间过程，压到最暗一档。
        body.push(Span::styled(format!(" · {}", detail), faint()));
    }
    (prefix, body)
}

/// 输入框视觉折行：按显示宽度硬折行 + 尊重显式换行符，返回字符区间。
fn layout_input(text: &str, width: usize) -> Vec<(usize, usize)> {
    let chars: Vec<char> = text.chars().collect();
    let width = width.max(1);
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    let mut start = 0usize;
    let mut col = 0usize;
    for (i, ch) in chars.iter().enumerate() {
        if *ch == '\n' {
            ranges.push((start, i));
            start = i + 1;
            col = 0;
            continue;
        }
        let w = char_width(*ch);
        if col + w > width && col > 0 {
            ranges.push((start, i));
            start = i;
            col = 0;
        }
        col += w;
    }
    ranges.push((start, chars.len()));
    ranges
}

/// 浮动面板：发丝描边 + 表面底色 + 标题行（hint 右对齐），返回标题下方的内容区。
fn draw_panel(f: &mut Frame, area: Rect, title: &str, hint: &str) -> Rect {
    // 底图可能含有跨越面板边界的 CJK 宽字符：只用 Clear(area) 时，
    // 被覆盖的「半个宽字符」会在下次重绘时反覆盖边框。向外扩 1 格清底，
    // 让边界两侧都回到干净画布，再绘制面板。
    let screen = f.area();
    // 只在水平方向外扩：宽字符只会跨列，不会跨行；纵向外扩会误清 header 发丝线/输入区上沿。
    let left = area.x.saturating_sub(1).max(screen.x);
    let right = area
        .x
        .saturating_add(area.width)
        .saturating_add(1)
        .min(screen.x.saturating_add(screen.width));
    let clear_area = Rect {
        x: left,
        y: area.y,
        width: right.saturating_sub(left),
        height: area.height,
    };
    // 注意：仅用带背景色的 Block 只会改 style、不清 symbol，必须先 Clear 再铺画布色。
    f.render_widget(ratatui::widgets::Clear, clear_area);
    f.render_widget(
        Block::default().style(Style::default().bg(theme::CANVAS)),
        clear_area,
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(Style::default().fg(theme::HAIRLINE))
        .style(Style::default().bg(theme::SURFACE));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let title_w = str_width(title) + 2;
    let hint_w = str_width(hint);
    let gap = (inner.width as usize)
        .saturating_sub(title_w + hint_w + 3)
        .max(1);
    let head = Line::from(vec![
        Span::styled(
            format!("  {}", title),
            Style::default().fg(theme::INK).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" ".repeat(gap)),
        Span::styled(hint.to_string(), faint()),
        Span::raw("  "),
    ]);
    let head_area = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: inner.height.min(2),
    };
    f.render_widget(
        Paragraph::new(vec![head, blank()]).style(Style::default().bg(theme::SURFACE)),
        head_area,
    );

    Rect {
        x: inner.x,
        y: inner.y.saturating_add(2),
        width: inner.width,
        height: inner.height.saturating_sub(2),
    }
}

fn truncate_width(text: &str, width: usize) -> String {
    let mut out = String::new();
    let mut used = 0usize;
    for ch in text.chars() {
        let w = char_width(ch);
        if used + w > width {
            break;
        }
        out.push(ch);
        used += w;
    }
    out
}

/// 按显示宽度截断，并以省略号收尾（头部元信息/面板右列专用）。
fn ellipsize_width(text: &str, width: usize) -> String {
    if str_width(text) <= width {
        return text.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let mut out = truncate_width(text, width.saturating_sub(1));
    out.push('…');
    out
}

/// wave-39 R7：token 数字缩写 —— `<1000` 原样；`<1e6` 一位小数 `k`（`.0` 省略）；
/// 否则一位小数 `M`（`.0` 省略）。
fn fmt_token_count(n: u64) -> String {
    fn one_decimal(value: f64) -> String {
        let rounded = (value * 10.0).round() / 10.0;
        let text = format!("{:.1}", rounded);
        text.strip_suffix(".0").unwrap_or(&text).to_string()
    }
    if n < 1_000 {
        return n.to_string();
    }
    if n < 1_000_000 {
        let k = n as f64 / 1_000.0;
        // 999_999 四舍五入后会变成 1000k：改以 M 表达，避免「1000k」这种破格写法。
        if (k * 10.0).round() >= 10_000.0 {
            return format!("{}M", one_decimal(k / 1_000.0));
        }
        return format!("{}k", one_decimal(k));
    }
    format!("{}M", one_decimal(n as f64 / 1_000_000.0))
}

/// wave-39 R7：workspace 路径标签 —— `display()` 全路径，`$HOME` 前缀折叠为 `~`。
fn workspace_label(workspace: &std::path::Path) -> String {
    let full = workspace.display().to_string();
    let home = std::env::var("HOME")
        .ok()
        .filter(|h| !h.trim().is_empty())
        .or_else(|| {
            std::env::var("USERPROFILE")
                .ok()
                .filter(|h| !h.trim().is_empty())
        });
    if let Some(home) = home {
        let home = home.trim_end_matches(['/', '\\']);
        if !home.is_empty() {
            if full == home {
                return "~".to_string();
            }
            if let Some(rest) = full.strip_prefix(home) {
                if rest.starts_with('/') || rest.starts_with('\\') {
                    return format!("~{}", rest);
                }
            }
        }
    }
    full
}

/// wave-39 v3 §6 R7/R10/R11 / wave-41 S1：色带下方指标行 —— 左侧 workspace 路径（整行 GHOST），
/// 右侧按「有值才显示」拼装 `↑in ↓out · 缓存 x% · 上下文 y%`；全无值时右簇为空。
/// 输入 token (↑) 与上下文百分比分子对齐 ADR 0035 S1：使用当前上下文输入 token（非会话累计）。
/// 波次 44：Goal 横幅与指标行的**时间片交替**参数。
///
/// 占用同一行会让右侧指标被 Goal 挤掉（真实目标动辄一整句），因此改为轮播：
/// 前 6 秒展示 Goal，随后 8 秒让位给 token / 缓存 / 上下文指标，循环往复。
pub const GOAL_BANNER_HOLD_MS: u64 = 6_000;
pub const METRICS_HOLD_MS: u64 = 8_000;

/// 当前 Goal 轮播相位（`None` = 没有活跃 Goal，整行永远是指标）。
/// 返回 `(是否显示 Goal, 距离下次切换的毫秒数)`，供主循环安排精确重绘。
pub fn goal_banner_phase(state: &TuiState) -> Option<(bool, u64)> {
    let goal = state.goal_state.as_ref()?;
    let _ = goal;
    let since = state.goal_banner_since?;
    let period = GOAL_BANNER_HOLD_MS + METRICS_HOLD_MS;
    let elapsed = since.elapsed().as_millis() as u64 % period;
    if elapsed < GOAL_BANNER_HOLD_MS {
        Some((true, GOAL_BANNER_HOLD_MS - elapsed))
    } else {
        Some((false, period - elapsed))
    }
}

/// Goal 横幅拆成「加重的 Goal 标签」+「正文」两段，避免整行同一灰度。
fn goal_banner_parts(goal: &TuiGoal) -> (String, String) {
    let rounds = match goal.max_rounds {
        Some(max) => format!("{}/{}", goal.round_count, max),
        None => format!("{}", goal.round_count),
    };
    let phase = match goal.phase {
        GoalPhase::Active => "进行中",
        GoalPhase::Completed => "已完成",
        GoalPhase::Blocked => "已阻塞",
        GoalPhase::Paused => "已暂停",
    };
    (
        "Goal".to_string(),
        format!(
            " · {} [{rounds}] ({phase})",
            goal.objective.trim().replace('\n', " ")
        ),
    )
}

fn metrics_line(state: &TuiState, width: usize) -> Line<'static> {
    let ghost = || Style::default().fg(theme::GHOST);
    let mut segments: Vec<String> = Vec::new();
    // P0：快照为唯一来源。`live_metrics` 为 `Some` ⇒ **只**渲染快照段，与 legacy 计量字段
    // 完全隔离（两者不得混排）；比率一律直取快照字段，此处**禁止**任何 clamp（I6）。
    if let Some(metrics) = state.live_metrics.as_ref() {
        let mut token_parts: Vec<String> = Vec::new();
        if let Some(context_input) = metrics.context_input_tokens {
            token_parts.push(format!("↑{}", fmt_token_count(context_input)));
        }
        if let Some(output) = metrics.output_tokens {
            token_parts.push(format!("↓{}", fmt_token_count(output)));
        }
        if !token_parts.is_empty() {
            segments.push(token_parts.join(" "));
        }
        if let Some(ratio) = metrics.cache_hit_ratio {
            segments.push(format!("缓存 {:.0}%", (ratio * 100.0).round()));
        }
        if let Some(ratio) = metrics.context_ratio {
            segments.push(format!("上下文 {:.0}%", (ratio * 100.0).round()));
        }
        if let Some(speed) = metrics.turn_speed_tokens_per_second {
            segments.push(format!("{:.0} t/s", speed.round()));
        }
        if let Some(ttft_ms) = metrics.first_token_latency_ms {
            segments.push(format!("首token {ttft_ms}ms"));
        }
    } else {
        if state.total_input_tokens > 0 || state.total_output_tokens > 0 {
            segments.push(format!(
                "↑{} ↓{}",
                fmt_token_count(state.total_input_tokens),
                fmt_token_count(state.total_output_tokens)
            ));
        }
        // 缓存比只在输入已知时可算（0% 也是有效值，故键控于 in>0 而非 cache>0）。
        // 单轮次计算缓存命中率：turn_cache_hit_tokens / total_input_tokens，并限制在 0.0..=100.0% 避免跨轮或数据异常导致越界。
        if state.total_input_tokens > 0 {
            let cache_hits = if state.turn_cache_hit_tokens > 0 {
                state.turn_cache_hit_tokens
            } else {
                state.total_cache_hit_tokens
            };
            let pct = (cache_hits as f64 / state.total_input_tokens as f64 * 100.0)
                .round()
                .clamp(0.0, 100.0);
            segments.push(format!("缓存 {pct:.0}%"));
        }
        if state.context_window_tokens > 0 {
            // ADR 0035 S1：若显式设置了 last_context_tokens（或单次调用的输入 token），以此为分子计算上下文占用
            let context_tokens = if state.last_context_tokens > 0 {
                state.last_context_tokens
            } else {
                state.total_input_tokens
            };
            let pct = (context_tokens as f64 / state.context_window_tokens as f64 * 100.0)
                .round()
                .clamp(0.0, 100.0);
            segments.push(format!("上下文 {pct:.0}%"));
        }
    }
    // C7/C18③/C23：排队指示器**唯一渲染点**就是本指标行（每帧都渲染，与 `mode`/`runtime_mode`
    // 无关），故 `status_word` 不再重复输出。`queue_indicator_line` 保证流式期 / 工具执行期
    // 都有输出；宽度受限时按 34 列截断，避免挤掉左侧 workspace 标签。
    if let Some(indicator) = queue_indicator_line(
        state.queued_count(),
        state
            .queue_notice
            .as_ref()
            .filter(|(_, at)| at.elapsed().as_millis() as u64 <= QUEUE_NOTICE_TTL_MS)
            .map(|(text, _)| text.as_str()),
        state.is_streaming,
        matches!(state.runtime_mode, RuntimeMode::ExecutingTool(_)),
    ) {
        segments.push(truncate_width(&indicator, 34));
    }
    // 契约 P-5 / F-N：一次性提示（非流式 + TTL 内）与非流式的计划模式横幅占左侧，
    // 数值指标仍在右侧；两者都没有时走既有布局（workspace 左 + 指标右）。
    let transient_left: Option<(String, Style)> = state
        .status_notice
        .as_ref()
        .filter(|(_, at)| {
            should_render_status_notice(
                state.is_streaming,
                at.elapsed().as_millis() as u64,
                STATUS_NOTICE_TTL_MS,
            )
        })
        .map(|(notice, _)| {
            let danger = notice.contains("失败") || notice.contains("错误");
            (
                truncate_width(notice, 34),
                if danger {
                    Style::default().fg(theme::DANGER)
                } else {
                    muted()
                },
            )
        })
        .or_else(|| {
            plan_mode_banner(state.mode == InteractionMode::PlanMode)
                .map(|banner| (banner.to_string(), muted()))
        });
    if let Some((left_text, left_style)) = transient_left {
        let right = segments.join(" · ");
        let right_w = str_width(&right);
        if right.is_empty() || RAIL + 2 + str_width(&left_text) + right_w > width {
            return Line::from(vec![
                Span::raw(" ".repeat(RAIL)),
                Span::styled(
                    ellipsize_width(&left_text, width.saturating_sub(RAIL)),
                    left_style,
                ),
            ]);
        }
        let budget = width.saturating_sub(RAIL + 1 + right_w);
        let left = ellipsize_width(&left_text, budget.max(6));
        let used = RAIL + str_width(&left);
        return Line::from(vec![
            Span::raw(" ".repeat(RAIL)),
            Span::styled(left, left_style),
            Span::raw(" ".repeat(width.saturating_sub(used + right_w))),
            Span::styled(right, ghost()),
        ]);
    }

    // 波次 44：Goal 与指标行按时间片交替（见 goal_banner_phase），
    // 且 Goal 相位**左对齐**成横幅，不与右对齐的数值指标混在同一排版里。
    let label_text = workspace_label(&state.workspace);
    if matches!(goal_banner_phase(state), Some((true, _))) {
        if let Some(goal) = state.goal_state.as_ref() {
            let (tag, body) = goal_banner_parts(goal);
            let left_budget = (width.saturating_sub(RAIL + 2)) / 3;
            let label = ellipsize_width(&label_text, left_budget.max(8));
            let used = RAIL + str_width(&label) + 2;
            let budget = width.saturating_sub(used).max(12);
            // 标签固定完整显示（最多 4 列），正文按剩余宽度省略。
            let body_budget = budget.saturating_sub(str_width(&tag)).max(6);
            let body = ellipsize_width(&body, body_budget);
            let pad = width
                .saturating_sub(used + str_width(&tag) + str_width(&body));
            return Line::from(vec![
                Span::raw(" ".repeat(RAIL)),
                Span::styled(label, ghost()),
                Span::raw("  "),
                // 「Goal」标记加重：BOLD + 比正文亮一档，作为横幅唯一的强调点。
                Span::styled(
                    tag,
                    Style::default()
                        .fg(theme::MUTED)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(body, ghost()),
                Span::raw(" ".repeat(pad)),
            ]);
        }
    }
    let right = segments.join(" · ");
    let right_w = str_width(&right);
    // 契约 F-I：workspace_label 每帧只算一次（含 env::var 加锁 + 分配），本函数两处共用。
    let label = &label_text;

    let mut spans: Vec<Span<'static>> = Vec::new();
    if right_w == 0 {
        // v3 R11：全无值 → 只显示左侧 workspace，不产生孤立分隔符。
        spans.push(Span::raw(" ".repeat(RAIL)));
        spans.push(Span::styled(
            ellipsize_width(label, width.saturating_sub(RAIL)),
            ghost(),
        ));
    } else if width > RAIL + 1 + right_w {
        let label = ellipsize_width(label, width - RAIL - 1 - right_w);
        let used = RAIL + str_width(&label);
        spans.push(Span::raw(" ".repeat(RAIL)));
        spans.push(Span::styled(label, ghost()));
        spans.push(Span::raw(" ".repeat(width.saturating_sub(used + right_w))));
        spans.push(Span::styled(right, ghost()));
    } else if right_w <= width {
        // 左路径放不下：只渲染右簇，仍然严格右对齐到内容列最后一列。
        spans.push(Span::raw(" ".repeat(width - right_w)));
        spans.push(Span::styled(right, ghost()));
    } else {
        spans.push(Span::styled(truncate_width(&right, width), ghost()));
    }
    Line::from(spans)
}

/// 按「字符」截断（绝不按字节切分，避免 CJK 边界 panic），超长时补省略号。
fn truncate_chars(text: &str, max_chars: usize) -> String {
    let mut out: String = text.chars().take(max_chars).collect();
    if text.chars().count() > max_chars {
        out.push('…');
    }
    out
}

fn pad_display(text: &str, width: usize) -> String {
    let used = str_width(text);
    if used >= width {
        text.to_string()
    } else {
        format!("{}{}", text, " ".repeat(width - used))
    }
}

/// 列表行：左侧 emphasis 竖条 + 主文本 + 右侧次要信息；选中行用极淡底色铺满整行。
/// 模型显示名：名称缺失或与 id 相同则回落到 id。
fn model_display_name(item: &ModelItem) -> String {
    if item.name.trim().is_empty() || item.name == item.id {
        item.id.clone()
    } else {
        item.name.clone()
    }
}

/// 行右侧徽标：默认（Ctrl+D）/ 优选（Ctrl+F）/ 当前（正在使用），可叠加。
fn model_badges(state: &TuiState, item: &ModelItem) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if state.model_preferences.default_model.as_deref() == Some(item.id.as_str()) {
        parts.push("默认");
    }
    if state.model_preferences.is_preferred(&item.id) {
        parts.push("优选");
    }
    if item.id == state.model {
        parts.push("当前");
    }
    parts.join(" · ")
}

/// 「优选」栏条目：按当前 provider 的模型顺序过滤已优选 id（保持稳定顺序便于对位）。
fn model_preferred_items(state: &TuiState) -> Vec<ModelItem> {
    state
        .model_list
        .iter()
        .filter(|item| state.model_preferences.is_preferred(&item.id))
        .cloned()
        .collect()
}

/// 模型面板的 Ctrl+D（设默认）/ Ctrl+F（切换优选）：写盘 + 同步到 `state`。
///
/// 抽成函数（store 由调用方注入）是为了让「快捷键 → 持久化 → 面板状态」这条链路
/// 能被测试直接驱动，而不是只能靠手敲终端验证。
pub fn apply_model_panel_shortcut(
    state: &mut TuiState,
    key: &crossterm::event::KeyEvent,
    store: &ModelPreferencesStore,
) -> bool {
    if !key.modifiers.contains(KeyModifiers::CONTROL) {
        return false;
    }
    let is_set_default = key.code == KeyCode::Char('d');
    let is_toggle_preferred = key.code == KeyCode::Char('f');
    if !is_set_default && !is_toggle_preferred {
        return false;
    }
    // 列表为空时仍然消费该按键，避免掉进通用分支产生意外输入。
    let Some(item) = state.model_list.get(state.selected_model_index).cloned() else {
        return true;
    };
    if is_set_default {
        match store.set_default_model(&item.id) {
            Ok(prefs) => {
                state.model_preferences = prefs;
                state.model = item.id.clone();
                state.status_text = format!("默认模型已设为 {}", model_display_name(&item));
            }
            Err(error) => state.status_text = format!("保存默认模型失败：{error}"),
        }
        return true;
    }
    match store.toggle_preferred(&item.id) {
        Ok((prefs, now_preferred)) => {
            state.model_preferences = prefs;
            state.status_text = if now_preferred {
                format!("已加入优选：{}", model_display_name(&item))
            } else {
                format!("已移出优选：{}", model_display_name(&item))
            };
        }
        Err(error) => state.status_text = format!("保存优选出错：{error}"),
    }
    true
}

fn list_row(selected: bool, left: &str, right: &str, width: usize) -> Line<'static> {
    let bg = if selected {
        theme::SURFACE_SEL
    } else {
        theme::SURFACE
    };
    let bar = if selected { "▏" } else { " " };
    let left_style = if selected {
        Style::default()
            .fg(theme::INK)
            .bg(bg)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme::INK_SOFT).bg(bg)
    };
    // 右侧状态/次要信息保持可读（FAINT 只留给纯装饰性 chrome）
    let right_style = Style::default().fg(theme::MUTED).bg(bg);

    // 右侧次要信息也必须截断，否则长 model id 会把左列挤压到不可读。
    let right_txt = ellipsize_width(right, width.saturating_sub(8));
    let right_w = str_width(&right_txt);
    let left_w = width.saturating_sub(3 + right_w);
    let left_txt = pad_display(&truncate_width(left, left_w), left_w);
    let used = 2 + left_w + right_w;
    Line::from(vec![
        Span::styled(
            bar,
            if selected {
                Style::default().fg(theme::ACCENT).bg(bg)
            } else {
                Style::default().bg(bg)
            },
        ),
        Span::styled(format!(" {}", left_txt), left_style),
        Span::styled(right_txt, right_style),
        Span::styled(" ".repeat(width.saturating_sub(used)), Style::default().bg(bg)),
    ])
}

/// 面板滚动窗口：保证选中项始终可见，并尽量让选中项居中。
fn panel_window(total: usize, selected: usize, visible: usize) -> (usize, usize) {
    if total <= visible {
        return (0, total);
    }
    let start = selected
        .saturating_sub(visible / 2)
        .min(total.saturating_sub(visible));
    (start, start + visible)
}

fn fill(f: &mut Frame, area: Rect, color: ratatui::style::Color) {
    f.render_widget(Block::default().style(Style::default().bg(color)), area);
}

/// 输入框行在鼠标选区坐标空间中的行号基址。
///
/// 正文（chat）用 0 起始的绝对渲染行号；输入框行用 `INPUT_LINE_BASE + 逻辑行号`，
/// 二者共享同一个 `TextCoord` 空间，复制时按基址拆分，避免拼接出百万级空行。
pub const INPUT_LINE_BASE: usize = 1_000_000;

/// 渲染带光标的多行输入区（⌘ 提示符 + 软折行 + 细竖线光标）。
fn render_input(f: &mut Frame, area: Rect, state: &mut TuiState) {
    let chars: Vec<char> = state.input_buffer.chars().collect();
    let cursor = state.input_cursor.min(chars.len());
    // 提示符 2 列 + 右侧 1 列呼吸留白：正文永不贴边。
    let text_width = area.width.saturating_sub(3).max(1) as usize;
    let ranges = layout_input(&state.input_buffer, text_width);
    let active_line = ranges
        .iter()
        .position(|(s, e)| cursor >= *s && cursor <= *e)
        .unwrap_or(ranges.len().saturating_sub(1));

    // 输入超过可视高度时，窗口跟随光标，保证正在编辑的行始终可见。
    let visible = area.height as usize;
    let start_line = if ranges.len() > visible {
        active_line
            .saturating_sub(visible.saturating_sub(1))
            .min(ranges.len() - visible)
    } else {
        0
    };

    // 鼠标选区快照：文本区矩形（提示符 2 列 + 右侧 1 列留白之外）、可视首行、全部逻辑行文本。
    state.last_input_rect = Some(Rect {
        x: area.x.saturating_add(2),
        y: area.y,
        width: area.width.saturating_sub(3).max(1),
        height: (visible.min(ranges.len()).max(1)) as u16,
    });
    state.last_input_view_start = start_line;
    state.last_input_lines = ranges
        .iter()
        .map(|(s, e)| chars[*s..*e].iter().collect::<String>())
        .collect();
    let end_line = (start_line + visible).min(ranges.len());

    let mut lines: Vec<Line<'static>> = Vec::new();
    for (idx, (s, e)) in ranges
        .iter()
        .enumerate()
        .take(end_line)
        .skip(start_line)
    {
        let prompt = if idx == 0 {
            Span::styled(
                "› ",
                Style::default()
                    .fg(theme::ACCENT)
                    .add_modifier(Modifier::BOLD),
            )
        } else if idx == start_line && start_line > 0 {
            Span::styled("↑ ", faint())
        } else if idx + 1 == end_line && end_line < ranges.len() {
            Span::styled("↓ ", faint())
        } else {
            Span::raw("  ")
        };
        let mut spans: Vec<Span<'static>> = vec![prompt];

        let text: String = chars[*s..*e].iter().collect();
        if idx == active_line {
            if state.input_buffer.is_empty() {
                // 块光标压在占位符首字上：光标与正文同列，一敲键不左跳。
                let placeholder = truncate_width("写下任务，或输入 / 查看命令", text_width);
                let mut ph = placeholder.chars();
                if let Some(first) = ph.next() {
                    spans.push(Span::styled(
                        first.to_string(),
                        Style::default().fg(theme::CANVAS).bg(theme::ACCENT),
                    ));
                    let rest: String = ph.collect();
                    if !rest.is_empty() {
                        spans.push(Span::styled(rest, Style::default().fg(theme::MUTED)));
                    }
                }
            } else {
                let before: String = chars[*s..cursor.min(*e)].iter().collect();
                if !before.is_empty() {
                    spans.push(Span::styled(before, ink()));
                }
                let on_char = cursor < *e;
                let cursor_text = if on_char {
                    chars[cursor].to_string()
                } else {
                    " ".to_string()
                };
                spans.push(Span::styled(
                    cursor_text,
                    Style::default().fg(theme::CANVAS).bg(theme::ACCENT),
                ));
                let after_start = if on_char { cursor + 1 } else { cursor };
                let after: String = chars[after_start.min(*e)..*e].iter().collect();
                if !after.is_empty() {
                    spans.push(Span::styled(after, ink()));
                }
            }
        } else if !text.is_empty() {
            spans.push(Span::styled(text, ink()));
        }

        lines.push(Line::from(spans));
    }

    // 输入框文本同样支持鼠标选区高亮：行号映射到 `INPUT_LINE_BASE + 逻辑行号`。
    let selection_range = state.mouse_selection.normalized_range();
    let input_first_line = INPUT_LINE_BASE + start_line;
    apply_selection_highlight(&mut lines, input_first_line, selection_range);

    let widget = Paragraph::new(lines).style(Style::default().bg(theme::SURFACE));
    f.render_widget(widget, area);
}

/// 把内部状态文案收敛为「一个短语」：底栏只放节奏信息，错误细节交给正文。
fn compact_status(st: &str) -> String {
    let st = st.trim();
    if st.is_empty() || st == "Ready" || st == "Turn completed" {
        return String::new();
    }
    if st == "Context cleared" {
        return "上下文已清空".to_string();
    }
    if st == "Interrupted" {
        return "已中断".to_string();
    }
    if st.starts_with("Thinking") {
        return "思考中".to_string();
    }
    if let Some(rest) = st.strip_prefix("Resumed ") {
        return format!("已恢复 {}", truncate_chars(rest, 18));
    }
    if let Some(rest) = st.strip_prefix("Added provider ") {
        return format!("已添加 {}", truncate_chars(rest, 18));
    }
    if let Some(rest) = st.strip_prefix("Switched model to ") {
        return format!("模型 · {}", truncate_chars(rest, 22));
    }
    if let Some(rest) = st.strip_prefix("Switched provider to ") {
        return format!("提供商 · {}", truncate_chars(rest, 20));
    }
    if st.starts_with("Turn failed") {
        return "失败".to_string();
    }
    if st.starts_with("模型列表拉取失败") {
        return "模型拉取失败".to_string();
    }
    truncate_width(st, 34)
}

/// 底部状态词：克制到只有一个短语（耗时与动画只在正文活动行出现一次）。
///
/// C7（`tui-streaming-input`）：队列非空时把「排队 N 条」追加在同一行 —— 流式期的
/// `status_notice` 按契约 P-5 让位给进度短语，故排队可见性必须走这条**流式期也渲染**的通道。
fn status_word(state: &TuiState) -> (String, Style) {
    // C23：排队/反馈指示器统一由 `metrics_line` 经 `queue_indicator_line` 渲染
    //（流式 + 工具执行期 `status_word` 整行不渲染，挂在这里会出现「零渲染点」）。
    status_word_base(state)
}

fn status_word_base(state: &TuiState) -> (String, Style) {
    // wave-60 §C9：停止请求受理后，**流式行**优先呈现「正在停止…」（终态未到前不得伪造）。
    // 走本通道而非 `status_notice`：后者被 `should_render_status_notice` 在流式期抑制（P-5 契约）。
    if state.stopping_turn_id.is_some() {
        // wave-63 §T3：stopping 期间若用户再次按键，`status_text` 被换成「等待安全收口」提示，
        // 该提示必须在流式行可见 —— 故 stopping 分支优先透出它，否则回落「正在停止…」。
        let transient = compact_status(&state.status_text);
        let word = if transient.contains("等待安全收口") {
            transient
        } else {
            "正在停止…".to_string()
        };
        return (word, Style::default().fg(theme::WARN));
    }
    // 说明：一次性提示（status_notice）与非流式横幅由 `metrics_line` 渲染 —— 本函数只服务
    // 「流式进行中的那一行状态」，流式期按契约让位给进度短语，故这里不重复判定。
    let transient = compact_status(&state.status_text);
    match &state.runtime_mode {
        RuntimeMode::Ready => {
            if transient.is_empty() {
                ("就绪".to_string(), faint())
            } else {
                let danger = transient.contains("失败") || transient.contains("错误");
                (transient, if danger { Style::default().fg(theme::DANGER) } else { muted() })
            }
        }
        RuntimeMode::Connecting => ("连接中".to_string(), muted()),
        RuntimeMode::Thinking => ("思考中".to_string(), faint()),
        RuntimeMode::Responding => ("生成中".to_string(), muted()),
        RuntimeMode::ExecutingTool(name) => {
            (format!("执行 · {}", truncate_chars(&clean_tool_name_display(name), 20)), faint())
        }
        RuntimeMode::WaitingApproval => ("待确认".to_string(), Style::default().fg(theme::WARN)),
        RuntimeMode::Completed => ("完成".to_string(), Style::default().fg(theme::SUCCESS)),
        RuntimeMode::Failed(_) => ("失败".to_string(), Style::default().fg(theme::DANGER)),
    }
}

fn elapsed_label(state: &TuiState) -> String {
    state
        .turn_started_at
        .map(|t| format_elapsed_seconds(t.elapsed().as_millis() as u64))
        .unwrap_or_default()
}

/// 单条消息渲染所需的跨消息上下文。
///
/// 为什么不是纯 `(message, width)`：轮间距取决于**前一条**消息角色；最后一条 ToolCall 是
/// 「执行中」需要动效帧；System 行的前置空行沿用「前面已有渲染行」的语义。这些状态由调用方
/// （`ui()` 的逐帧循环）注入，纯函数入口 [`message_lines_for`] 用中性默认值。
struct MessageRenderCtx {
    prev_role: Option<MessageRole>,
    is_last: bool,
    has_preceding_lines: bool,
    spinner_tick: usize,
}

/// 把一条消息渲染成行（内部实现，带跨消息上下文）。
fn message_lines_with_ctx(
    message: &ChatMessage,
    width: usize,
    ctx: MessageRenderCtx,
) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let rail_blank = || blank_rail(Style::default());
            let prev_is_tool = matches!(
                ctx.prev_role,
                Some(MessageRole::ToolCall) | Some(MessageRole::ToolResult)
            );
            match message.role {
                MessageRole::User => {
                    // 轮间距 2 行：长会话里一眼能扫到「新一轮提问」的节拍
                    push_gap(&mut lines, 2);
                    push_user_block(&mut lines, &message.content, width);
                }
                MessageRole::Assistant => {
                    // 助手正文不再打 "pony" 字标：角色由轨道与留白承担，避免每轮都有一段品牌噪音。
                    // 工具执行后恢复正文只用一行留白表示节奏。
                    if prev_is_tool {
                        push_blank(&mut lines);
                    } else if ctx.prev_role.is_some() {
                        push_gap(&mut lines, 2);
                    }
                    let body_w = width.saturating_sub(RAIL);
                    let mut in_code = false;
                    for raw in message.content.lines() {
                        let trimmed = raw.trim_start();
                        if trimmed.starts_with("```") {
                            in_code = !in_code;
                            if in_code {
                                // 代码块开始：增加与上方正文文字的 1 行外呼吸留白间距
                                push_blank(&mut lines);
                                // 代码块内部：顶部内边距（带有陶土暗底色）
                                push_code_band_padding(&mut lines, width);

                                let lang = trimmed.trim_start_matches('`').trim();
                                if !lang.is_empty() {
                                    push_code_band_line(
                                        &mut lines,
                                        Line::from(Span::styled(
                                            format!("// {}", lang),
                                            Style::default().fg(theme::VSCODE_COMMENT).add_modifier(Modifier::ITALIC),
                                        )),
                                        width,
                                    );
                                }
                            } else {
                                // 代码块内部：底部内边距（带有陶土暗底色）
                                push_code_band_padding(&mut lines, width);
                                // 代码块结束：增加与下方后续文字的 1 行外呼吸留白间距（必须无条件插入空行，即便上一行是带底色的内衬行）
                                lines.push(blank());
                            }
                            continue;
                        }
                        if in_code {
                            // 代码块材质：使用 VS Code 经典暗色配色高亮与增强陶土色底衬
                            push_code_band_line(
                                &mut lines,
                                Line::from(highlight_code_line(raw)),
                                width,
                            );
                            continue;
                        }
                        if trimmed.is_empty() {
                            push_blank(&mut lines);
                            continue;
                        }
                        let body = Line::from(render_markdown_line(raw));
                        push_prefixed(&mut lines, rail_blank(), rail_blank(), body, body_w);
                    }
                }
                MessageRole::ToolCall | MessageRole::ToolResult => {
                    // 工具行本身属于 assistant turn：若无正文先行，此处只留间距、不打字标。
                    if !prev_is_tool && !matches!(ctx.prev_role, Some(MessageRole::Assistant)) {
                        push_gap(&mut lines, 2);
                    }
                    let is_call = message.role == MessageRole::ToolCall;
                    let running = is_call && ctx.is_last;
                    let is_error = message.content.contains("失败")
                        || message.content.contains("Error")
                        || message.content.contains("error");
                    let (glyph, glyph_style) = if is_call {
                        if running {
                            (
                                executing_tool_frame(ctx.spinner_tick).to_string(),
                                Style::default().fg(theme::ACCENT_DIM),
                            )
                        } else {
                            (GLYPH_RUNNING.to_string(), faint())
                        }
                    } else if is_error {
                        (GLYPH_FAIL.to_string(), Style::default().fg(theme::DANGER))
                    } else {
                        (GLYPH_DONE.to_string(), faint())
                    };
                    // 工具行是中间过程而非重点：动作名压到 MUTED/FAINT，只有失败回到 DANGER。
                    let head_style = if is_error {
                        Style::default().fg(theme::DANGER)
                    } else if is_call {
                        muted()
                    } else {
                        faint()
                    };
                    let (prefix, body) =
                        tool_line_spans(&glyph, glyph_style, head_style, &message.content);
                    push_prefixed(
                        &mut lines,
                        prefix,
                        rail_blank(),
                        Line::from(body),
                        width.saturating_sub(RAIL),
                    );
                }
                MessageRole::System => {
                    let is_error = message.content.contains("Error")
                        || message.content.contains("fail")
                        || message.content.contains("失败");
                    let (glyph, glyph_style) = if is_error {
                        (GLYPH_FAIL, Style::default().fg(theme::DANGER))
                    } else {
                        ("·", faint())
                    };
                    let text_style = if is_error {
                        Style::default().fg(theme::DANGER)
                    } else {
                        muted()
                    };
                    if ctx.has_preceding_lines && !prev_is_tool {
                        push_blank(&mut lines);
                    }
                    let prefix = vec![Span::styled(format!("{} ", glyph), glyph_style)];
                    for (i, raw) in message.content.lines().enumerate() {
                        let (first, cont) = if i == 0 {
                            (prefix.clone(), rail_blank())
                        } else {
                            (rail_blank(), rail_blank())
                        };
                        push_prefixed(
                            &mut lines,
                            first,
                            cont,
                            Line::from(Span::styled(raw.to_string(), text_style)),
                            width.saturating_sub(RAIL),
                        );
                    }
                    if message.content.is_empty() {
                        push_prefixed(
                            &mut lines,
                            prefix,
                            rail_blank(),
                            Line::from(""),
                            width.saturating_sub(RAIL),
                        );
                    }
                }
            }
    lines
}

/// 契约 P-12（修订11 步骤1）：单条消息 → 渲染行（纯函数，可单测）。
///
/// 与 `ui()` 的逐帧渲染共用**同一份**实现；差异仅在于没有跨消息上下文（无前序角色、
/// 不是最后一条、spinner 帧取 0），因此同一消息在会话中首次出现与单独渲染的行内容一致。
pub fn message_lines_for(message: &ChatMessage, width: usize) -> Vec<Line<'static>> {
    message_lines_with_ctx(
        message,
        width,
        MessageRenderCtx {
            prev_role: None,
            is_last: false,
            has_preceding_lines: false,
            spinner_tick: 0,
        },
    )
}

/// 契约（修订11 步骤3）：从按消息分块的布局缓存里取出 `[start, end)` 行。
///
/// 复杂度是「跨块定位」而非「拼接全部行」：只 clone 落在窗口内的行。
fn collect_window_lines(
    blocks: &[Vec<Line<'static>>],
    start: usize,
    end: usize,
) -> Vec<Line<'static>> {
    let mut out: Vec<Line<'static>> = Vec::with_capacity(end.saturating_sub(start));
    if start >= end {
        return out;
    }
    let mut cursor = 0usize;
    for block in blocks {
        let block_start = cursor;
        let block_end = cursor + block.len();
        cursor = block_end;
        if block_end <= start {
            continue;
        }
        if block_start >= end {
            break;
        }
        let local_start = start.saturating_sub(block_start);
        let local_end = (end - block_start).min(block.len());
        out.extend(block[local_start..local_end].iter().cloned());
    }
    out
}

/// 鼠标选区高亮：选区用**绝对行号 + 显示列**表达，窗口可能只是全量行的一段，故需要 `first_line` 偏移。
fn apply_selection_highlight(
    lines: &mut [Line<'static>],
    first_line: usize,
    range: Option<(TextCoord, TextCoord)>,
) {
    let Some((start, end)) = range else {
        return;
    };
    let highlight_style = Style::default().add_modifier(Modifier::REVERSED);
    let last_line = first_line + lines.len().saturating_sub(1);
    for line_idx in start.line..=end.line.min(last_line) {
        let Some(local_idx) = line_idx.checked_sub(first_line) else {
            continue;
        };
        let Some(old_line) = lines.get(local_idx) else {
            continue;
        };
        // 本行参与的显示列窗口（起行取 start.col，止行取 end.col，中间行全选）。
        let col_from = if line_idx == start.line { start.col } else { 0 };
        let col_to = if line_idx == end.line {
            end.col
        } else {
            usize::MAX
        };
        let mut new_spans: Vec<Span<'static>> = Vec::new();
        let mut display_col = 0usize;
        for span in &old_line.spans {
            let base_style = span.style;
            let mut segment = String::new();
            let mut in_sel = false;
            for ch in span.content.chars() {
                // 宽字符占 2 列：旧实现每 char 只 +1 列，导致中文/emoji 行选区整体左漂。
                let width = char_width(ch);
                let ch_start = display_col;
                let ch_end = display_col + width - 1;
                display_col += width;
                let is_sel = ch_end >= col_from && ch_start <= col_to;
                if segment.is_empty() {
                    in_sel = is_sel;
                    segment.push(ch);
                } else if in_sel == is_sel {
                    segment.push(ch);
                } else {
                    let style = if in_sel {
                        base_style.patch(highlight_style)
                    } else {
                        base_style
                    };
                    new_spans.push(Span::styled(std::mem::take(&mut segment), style));
                    in_sel = is_sel;
                    segment.push(ch);
                }
            }
            if !segment.is_empty() {
                let style = if in_sel {
                    base_style.patch(highlight_style)
                } else {
                    base_style
                };
                new_spans.push(Span::styled(segment, style));
            }
        }
        lines[local_idx] = Line::from(new_spans);
    }
}

/// 把「鼠标命中的显示列」吸附到所在字形的起始列。
///
/// 宽字符（CJK / emoji）占 2 个显示列，点在右半格时命中的列落在字形中间：
/// 若直接用该列做选区端点，`起 < 字形起点`、`止 > 字形起点`，高亮与复制的边界
/// 就会各说各话（复制会带上半个字的判定、续选会从字中间开始）。吸附后：
/// 同一个字形内的任意列都归约到该字起点，行尾之后归约到行宽（允许延伸选到行尾）。
pub fn snap_col_to_glyph_start(text: &str, col: usize) -> usize {
    let mut display = 0usize;
    for ch in text.chars() {
        let width = char_width(ch).max(1);
        if col < display + width {
            return display;
        }
        display += width;
    }
    display
}

/// 正文绝对渲染行的纯文本（与 `rendered_lines_text` 同源，但按需只拼一行，
/// 避免为一次点击重建全量文本）。缓存未就绪时返回 `None`，调用方回退到原始列。
fn chat_line_plain_text(state: &TuiState, abs_line: usize) -> Option<String> {
    let mut offset = 0usize;
    for block in &state.render_cache.cache_lines {
        if abs_line < offset + block.len() {
            let line = block.get(abs_line - offset)?;
            return Some(
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>(),
            );
        }
        offset += block.len();
    }
    None
}

/// 命中列 → 选区列：正文行与输入框行都按「字形起点」归约（宽字符整字语义）。
fn snap_hit_col(state: &TuiState, line: usize, raw_col: usize) -> usize {
    if line >= INPUT_LINE_BASE {
        state
            .last_input_lines
            .get(line - INPUT_LINE_BASE)
            .map(|text| snap_col_to_glyph_start(text, raw_col))
            .unwrap_or(raw_col)
    } else {
        chat_line_plain_text(state, line)
            .map(|text| snap_col_to_glyph_start(&text, raw_col))
            .unwrap_or(raw_col)
    }
}

pub fn ui(f: &mut Frame, state: &mut TuiState) {
    let area = f.area();
    fill(f, area, theme::CANVAS);

    // 极小终端降级：不 panic，也不输出半截布局，只给一句可读提示。
    if area.width < 24 || area.height < 10 {
        let lines = vec![
            Line::from(Span::styled("终端窗口过小", muted())),
            blank(),
            Line::from(Span::styled("请调整到至少 24 × 10", faint())),
        ];
        f.render_widget(
            Paragraph::new(lines)
                .alignment(Alignment::Center)
                .style(Style::default().bg(theme::CANVAS)),
            area,
        );
        return;
    }

    // 超宽终端下限制正文行宽（约 120 列），并把内容列居中：
    // 终端再宽，阅读行宽也不失控；左右留白由两侧均分。
    const MAX_CONTENT_WIDTH: u16 = 120;
    let content_w = area
        .width
        .saturating_sub(GUTTER * 2)
        .min(MAX_CONTENT_WIDTH);
    let content = Rect {
        x: area.x + area.width.saturating_sub(content_w) / 2,
        y: area.y,
        width: content_w,
        height: area.height,
    };

    // ── Header：一行、无底色带、无徽章 ─────────────────────────────────────
    let ws_name = state
        .workspace
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("~");
    // header 保留「模型」与当前非默认思考强度信息；session hash 属于 /resume 的上下文。
    let right_meta = match state.reasoning_effort {
        Some(_) => format!("{} · 思考:{}", state.model, state.reasoning_effort_label()),
        None => state.model.clone(),
    };
    let left_w = 4 + 5 + str_width(ws_name);
    let right_w = str_width(&right_meta);
    let pad = content.width as usize;
    let header_line = if pad > left_w + right_w {
        Line::from(vec![
            Span::styled("pony", Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD)),
            Span::styled("  ·  ", faint()),
            Span::styled(ws_name.to_string(), muted()),
            Span::raw(" ".repeat(pad - left_w - right_w)),
            Span::styled(right_meta, faint()),
        ])
    } else if pad > left_w + 12 {
        // 中等宽度：保留右侧模型/会话，截断而不是整块丢弃。
        let meta = ellipsize_width(&right_meta, pad - left_w - 1);
        let gap = pad.saturating_sub(left_w + str_width(&meta));
        Line::from(vec![
            Span::styled("pony", Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD)),
            Span::styled("  ·  ", faint()),
            Span::styled(ws_name.to_string(), muted()),
            Span::raw(" ".repeat(gap)),
            Span::styled(meta, faint()),
        ])
    } else {
        Line::from(vec![
            Span::styled("pony", Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD)),
            Span::styled("  ·  ", faint()),
            Span::styled(ws_name.to_string(), muted()),
        ])
    };
    let header_rect = Rect { height: 1, ..content };
    f.render_widget(Paragraph::new(header_line), header_rect);

    // ── 发丝线 + 呼吸留白 ─────────────────────────────────────────────────
    let rule_rect = Rect { y: content.y + 1, height: 1, ..content };
    f.render_widget(Paragraph::new(hairline(content.width as usize)), rule_rect);

    // ── 底部区块尺寸：色带（1 行顶部内边距 + 输入区）+ 紧贴的指标行 ──────────
    let composer_text_width = content.width.saturating_sub(3).max(1) as usize;
    let visual_lines = layout_input(&state.input_buffer, composer_text_width).len() as u16;
    // wave-39 v3 R9：正文 + 1 行呼吸（单行输入 = 2 行），最多 6 行；色带 = input_h + 顶部内边距。
    // wave-39 v4 R13：删除独立 CANVAS 间距行，input_h 末行即色带内呼吸行 → 间距视觉 2 行收为 1 行。
    // wave-41：发送时 Composer 高度平滑衰减收拢，正文平滑展开，底部平稳下移。
    let target_input_h = (visual_lines + 1).clamp(2, 6);
    let input_h = if let Some(anim_h) = state.anim_composer_h {
        if anim_h > target_input_h {
            let next_h = anim_h.saturating_sub(1);
            state.anim_composer_h = if next_h > target_input_h { Some(next_h) } else { None };
            anim_h
        } else {
            state.anim_composer_h = None;
            target_input_h
        }
    } else {
        target_input_h
    };
    let band_h = input_h + 1;
    // 底部固定基座 = 色带 + 指标行；内联 Ask 面板插在基座之上（信息流底部）。
    let bottom_h = input_h + 2;
    let bottom_top = area.y.saturating_add(area.height).saturating_sub(bottom_h);
    let ask_inline_h = inline_ask_height(state, content.width as usize);

    let chat_top = content.y + 3; // header + rule + 1 行呼吸
    let chat_h = bottom_top.saturating_sub(chat_top);
    let chat_rect = Rect {
        x: content.x,
        y: chat_top,
        width: content.width,
        height: chat_h.max(1),
    };
    // Ask 是正文下部的覆盖层，不参与消息区几何；极小终端也不能越过 header 或 Composer。
    let ask_rect = if ask_inline_h > 0 {
        let ask_y = chat_rect.y.max(bottom_top.saturating_sub(ask_inline_h));
        let ask_bottom = chat_rect.y.saturating_add(chat_rect.height).min(bottom_top);
        Rect {
            x: content.x,
            y: ask_y,
            width: content.width,
            height: ask_bottom.saturating_sub(ask_y).min(ask_inline_h),
        }
    } else {
        Rect::default()
    };
    state.last_chat_rect = Some(chat_rect);

    // ── 会话正文：轨道标记 + 单色正文 + 分层工具行 ─────────────────────────
    let width = chat_rect.width as usize;
    // 契约（修订11 步骤3）：正文行来自布局缓存，按可见窗口切片后渲染；
    // `body_lines` 仅承载「空态引导」（行数很少，无需虚拟化），
    // `trailing_status_lines` 承载正文末尾的流式状态行。
    let mut body_lines: Vec<Line<'static>> = Vec::new();
    let mut trailing_status_lines: Vec<Line<'static>> = Vec::new();
    let rail_blank = || blank_rail(Style::default());

    // 契约 F-I：命令匹配每帧只算一次（空态引导 / 斜杠菜单 / 补全共用同一结果）。
    let frame_matching_commands = state.matching_commands();

    // 图形协议品牌标在正文中的起始行（渲染位图用；`None` = 字符画路径）。
    let mut splash_banner_line: Option<usize> = None;
    if state.messages.is_empty() {
        let mut splash: Vec<Line<'static>> = Vec::new();
        // 短终端不硬塞品牌画：优先保证「一句话 + 命令入口」完整可见，且底部至少留 1 行呼吸。
        // 当前构图 = 1 空行 + 折角标 11 行 + 1 空行 + 一句定位语 + 1 空行 + 引导块 8 行 = 23 行，
        // 故正文区高度必须 ≥ 24 才放品牌画（=23 时引导块会顶到正文区最后一行，紧贴输入带）。
        let show_banner = chat_h as usize >= 24;
        let mut banner_draw_logo = true;
        if show_banner {
            // 图形协议路径要求横幅行不因换行而错位；行宽与是否绘制像素无关
            // （两者都是 28 列占位），先按占位版量宽，再决定最终用位图还是字符画。
            let placeholder = pony_banner_lines(state, false);
            let banner_fits = placeholder.iter().all(|line| line.width() <= width);
            let use_image = state.splash_image.is_some() && banner_fits;
            banner_draw_logo = !use_image;
            let banner_lines = if use_image {
                placeholder
            } else {
                pony_banner_lines(state, true)
            };
            splash.push(blank());
            splash.extend(banner_lines);
            splash.push(blank());
        }
        splash.push(Line::from(Span::styled(
            "  终端里的自主智能体。",
            Style::default().fg(theme::MUTED),
        )));
        if frame_matching_commands.is_empty() {
            splash.push(blank());
            splash.extend(ascii_logo());
        }
        // 空态留白构图：按正文区 1/3 高度锚定，避免顶部挤、底部空。
        let top_pad = (chat_h as usize).saturating_sub(splash.len()) / 3;
        for _ in 0..top_pad {
            body_lines.push(blank());
        }
        // splash[0] 是品牌画上方的留白行，位图从下一行开始铺。
        if show_banner && !banner_draw_logo {
            splash_banner_line = Some(top_pad + 1);
        }
        for line in splash {
            push_wrapped(&mut body_lines, line, width);
        }
    } else {
        // 契约（修订11 步骤2）：命中缓存时只重算最后一条；脏则全量重建。
        // 步骤3：正文行不再整体拷进临时缓冲 —— 渲染时按可见窗口从缓存切片。
        state.ensure_render_cache(width, state.spinner_tick);

        // 进行中的一行极简状态（不打断阅读节奏，紧凑规整，不晃动）
        // wave-60 §C9：stopping 是用户此刻唯一关心的进行中态 ⇒ 即便正处于工具执行也保留该行，
        // 否则「正在停止…」在工具执行期不可见（终态由 turn:cancelled 驱动，见 apply_stop_terminal）。
        if state.is_streaming
            && (state.stopping_turn_id.is_some()
                || !matches!(state.runtime_mode, RuntimeMode::ExecutingTool(_)))
        {
            let need_blank = state
                .render_cache
                .cache_lines
                .last()
                .and_then(|block| block.last())
                .map(|line| {
                    !line.spans.is_empty()
                        && !line.spans.iter().all(|span| span.content.trim().is_empty())
                })
                .unwrap_or(false);
            if need_blank {
                push_blank(&mut trailing_status_lines);
            }
            let (word, style) = status_word(state);
            let elapsed = elapsed_label(state);
            let text = if elapsed.is_empty() {
                word
            } else {
                format!("{}  {}", word, elapsed)
            };
            let prefix = vec![Span::styled(
                format!("{} ", mode_status_icon(&state.runtime_mode, state.spinner_tick)),
                Style::default().fg(theme::ACCENT_DIM),
            )];
            push_prefixed(
                &mut trailing_status_lines,
                prefix,
                rail_blank(),
                Line::from(Span::styled(text, style)),
                width.saturating_sub(RAIL),
            );
        }
    }

    // 精确滚动：总行数 = 布局缓存行数 + 正文末尾的流式状态行（空态用自身的引导行）。
    let visible = chat_rect.height as usize;
    let cache_total = if state.messages.is_empty() {
        0
    } else {
        state.render_cache.stats.total_lines
    };
    let total = if state.messages.is_empty() {
        body_lines.len()
    } else {
        cache_total + trailing_status_lines.len()
    };
    let max_offset = total.saturating_sub(visible);
    // 契约 F-H：max_scroll_offset 以 u32 承载（>65535 视觉行不再回绕跳顶）。
    state.max_scroll_offset = calculate_scroll_offset(total, visible);
    // usize → u32 的统一饱和口径（仅在 >4G 视觉行的不可能场景触发）。
    let to_u32 = |value: usize| u32::try_from(value).unwrap_or(u32::MAX);

    let effective_offset: u32 = if state.auto_scroll {
        let current = state.scroll_offset as usize;
        if current < max_offset {
            let diff = max_offset - current;
            // 阻尼平滑跟随：新消息涌入或状态下移时，以阻尼步长平滑推进行，消除画面撕裂
            let step = if diff > 6 {
                3
            } else if diff > 2 {
                2
            } else {
                1
            };
            let next = to_u32((current + step).min(max_offset));
            state.scroll_offset = next;
            next
        } else {
            let clamped = to_u32(current.min(max_offset));
            state.scroll_offset = clamped;
            clamped
        }
    } else {
        let clamped = to_u32((state.scroll_offset as usize).min(max_offset));
        state.scroll_offset = clamped;
        clamped
    };
    state.effective_scroll_offset = effective_offset;

    // 契约（修订11 步骤3）：只把窗口 `[start, start + visible)` 喂给 Paragraph，scroll 归零。
    // 空态引导行很少，仍整份渲染（保留原有滚动语义），因此这里分两条路径。
    let window_start = (effective_offset as usize).min(max_offset);
    let (mut render_lines, render_scroll, first_line_index) = if state.messages.is_empty() {
        (
            body_lines,
            clamp_render_scroll(effective_offset),
            0usize,
        )
    } else {
        let window_end = (window_start + visible).min(total);
        let mut window = collect_window_lines(&state.render_cache.cache_lines, window_start, window_end);
        // 末尾流式状态行（逻辑行号从 `cache_total` 起）。
        if window_end > cache_total {
            let from = window_start.saturating_sub(cache_total);
            if from < trailing_status_lines.len() {
                window.extend(trailing_status_lines[from..].iter().cloned());
            }
        }
        (window, 0u16, window_start)
    };
    // 鼠标选区高亮：绝对行号 → 当前窗口行号（窗口只是全量行的一段）。
    let selection_range = state.mouse_selection.normalized_range();
    apply_selection_highlight(&mut render_lines, first_line_index, selection_range);
    f.render_widget(
        Paragraph::new(render_lines)
            .style(Style::default().bg(theme::CANVAS))
            .scroll((render_scroll, 0)),
        chat_rect,
    );

    // 图形协议品牌标：铺在字符画占位矩形上（位图画布底色不透明，可完整覆盖占位）。
    if let Some(banner_line) = splash_banner_line {
        let scroll = state.effective_scroll_offset as i32;
        let chat_top = chat_rect.y as i32;
        let chat_bottom = chat_top + chat_rect.height as i32;
        let top = chat_top + banner_line as i32 - scroll;
        if let Some(image) = state.splash_image.as_mut() {
            if top >= chat_top && top < chat_bottom {
                let height = image.rows.min((chat_bottom - top) as u16);
                let width = image.cols.min(chat_rect.width.saturating_sub(1));
                if width > 0 && height > 0 {
                    let image_rect = Rect {
                        x: chat_rect.x + 1,
                        y: top as u16,
                        width,
                        height,
                    };
                    let widget = StatefulImage::new()
                        .resize(ImageResize::Fit(Some(FilterType::Lanczos3)));
                    f.render_stateful_widget(widget, image_rect, &mut image.protocol);
                }
            }
        }
    }

    // 其他浮层仍先清场；Ask 仅覆盖自身矩形，未遮挡的历史消息保持原样。
    if state.mode != InteractionMode::Normal && state.mode != InteractionMode::AskUserQuestion {
        f.render_widget(ratatui::widgets::Clear, chat_rect);
        f.render_widget(
            Block::default().style(Style::default().bg(theme::CANVAS)),
            chat_rect,
        );
    }

    // ── Ask 覆盖层：下缘贴 Composer 上缘，消息几何不随面板开关变化 ──────────
    if ask_rect.height > 0 {
        // 仅清自己的覆盖矩形：既抹掉底层字形/宽字符占位，又不影响未遮挡历史。
        f.render_widget(ratatui::widgets::Clear, ask_rect);
        render_ask_inline(f, ask_rect, state);
    }

    // ── Composer：整宽表面色带 = 1 行顶部内边距 + 输入区；指标行紧贴其下 ──────
    // 色带与内容列严格同宽：与 header 发丝线共用同一条左右基准。
    let band_x = content.x;
    let band_w = content.width;
    let band = Rect {
        x: band_x,
        y: bottom_top,
        width: band_w,
        height: band_h,
    };
    // 普通态渲染输入区；Ask 态保留色带与指标行，只把输入区换成「等待选择」提示，
    // 让问答面板成为信息流底部的一段，而不是遮罩式弹窗。
    if state.mode == InteractionMode::Normal || state.mode == InteractionMode::AskUserQuestion {
        fill(f, band, theme::SURFACE);
        // wave-39 v4 R13：色带末行是呼吸行（整行空白）。v4 起它同时是「输入正文 → 指标行」之间
        // 唯一的 1 行视觉间距；底色按色带 content 宽铺设，左右沟槽列保持 CANVAS。
        let breathing_rect = Rect {
            x: content.x,
            y: bottom_top.saturating_add(band_h).saturating_sub(1),
            width: content.width,
            height: 1,
        };
        f.render_widget(
            Block::default().style(Style::default().bg(theme::SURFACE)),
            breathing_rect,
        );

        let input_rect = Rect {
            x: content.x,
            y: bottom_top.saturating_add(1),
            width: content.width,
            height: input_h,
        };
        if state.mode == InteractionMode::AskUserQuestion {
            render_ask_band_hint(f, input_rect, state);
        } else {
            render_input(f, input_rect, state);
        }

        // wave-39 v4 R13：指标行紧贴色带下方（删除 v3 独立 CANVAS 间距行）；
        // 指标行不在色带内：整行 GHOST 前景 + CANVAS 背景，绝不写出内容列。
        let metrics_rect = Rect {
            x: content.x,
            y: bottom_top.saturating_add(band_h),
            width: content.width,
            height: 1,
        };
        f.render_widget(
            Paragraph::new(metrics_line(state, content.width as usize))
                .style(Style::default().bg(theme::CANVAS).fg(theme::GHOST)),
            metrics_rect,
        );
    }

    // ── 斜杠命令面板：浮于输入区之上，两条发丝线夹一段浅表面 ──────────────
    let match_cmds = &frame_matching_commands;
    if !match_cmds.is_empty() && state.mode == InteractionMode::Normal {
        // 契约 C-6：行数上限随命令表与窗口可用行数自适应，不再硬编码 8。
        // 菜单浮于输入带上方（上下发丝线各 1 + 与输入区呼吸 1），底边不得越过聊天区顶边，
        // 故可用行数由 `band.y - chat_rect.y` 派生（预留 4 行）；窗口不足才截断，最少 3 行。
        let available_rows = band
            .y
            .saturating_sub(chat_rect.y.saturating_add(4)) as usize;
        let rows = match_cmds.len().min(available_rows.max(3)) as u16;
        let menu_h = rows + 2;
        let selected = state
            .selected_command_index
            .min(match_cmds.len().saturating_sub(1));
        let (window_start, window_end) = panel_window(match_cmds.len(), selected, rows as usize);
        let desc_w = match_cmds
            .iter()
            .take(window_end)
            .skip(window_start)
            .map(|(_, d)| str_width(d))
            .max()
            .unwrap_or(0);
        // 几何：竖条 1 + 空格 1 + 命令列 12 + 描述 + 右侧留白 2
        let need_w = (16usize + desc_w + 2).min(u16::MAX as usize) as u16;
        // 契约 F-A：窄终端（24–33 列）下 `clamp(28, content.width.min(64))` 会 min > max
        // 直接 panic（core cmp.rs:1092 min <= max）。先派生安全区间：max_w 至少 1（极端列宽
        // 也不越界），min_w 取 28 但永不超过 max_w ⇒ min_w <= max_w 恒成立，结果必落在 [1, max_w]。
        // 纵向不足时仍由下方 `menu_rect.y > chat_rect.y` 守卫整体跳过（不渲染残缺菜单）。
        let max_w = content.width.min(64).max(1);
        let min_w = 28.min(max_w);
        let menu_w = need_w.clamp(min_w, max_w);
        let menu_y = band.y.saturating_sub(menu_h + 1); // 与输入区留一行呼吸
        let menu_rect = Rect {
            x: content.x,
            y: menu_y,
            width: menu_w,
            height: menu_h,
        };
        if menu_rect.y > chat_rect.y {
            f.render_widget(ratatui::widgets::Clear, menu_rect);
            let block = Block::default()
                .borders(Borders::TOP | Borders::BOTTOM)
                .border_type(BorderType::Plain)
                .border_style(Style::default().fg(theme::HAIRLINE))
                .style(Style::default().bg(theme::SURFACE));
            let inner = block.inner(menu_rect);
            f.render_widget(block, menu_rect);

            let inner_w = inner.width as usize;
            let mut menu_lines: Vec<Line<'static>> = Vec::new();
            for (idx, (cmd, desc)) in match_cmds
                .iter()
                .enumerate()
                .take(window_end)
                .skip(window_start)
            {
                let is_sel = idx == selected;
                let bg = if is_sel { theme::SURFACE_SEL } else { theme::SURFACE };
                let cmd_col = pad_display(cmd, 11);
                let used = 2 + str_width(&cmd_col) + str_width(desc);
                let mut spans = vec![
                    Span::styled(
                        if is_sel { "▏" } else { " " },
                        if is_sel {
                            Style::default().fg(theme::ACCENT).bg(bg)
                        } else {
                            Style::default().bg(bg)
                        },
                    ),
                    Span::styled(
                        format!(" {}", cmd_col),
                        if is_sel {
                            Style::default()
                                .fg(theme::INK)
                                .bg(bg)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(theme::INK_SOFT).bg(bg)
                        },
                    ),
                    Span::styled(
                        desc.to_string(),
                        if is_sel {
                            Style::default().fg(theme::INK_SOFT).bg(bg)
                        } else {
                            Style::default().fg(theme::MUTED).bg(bg)
                        },
                    ),
                    Span::styled(
                        " ".repeat(inner_w.saturating_sub(used)),
                        Style::default().bg(bg),
                    ),
                ];
                spans.retain(|s| !s.content.is_empty());
                menu_lines.push(Line::from(spans));
            }
            f.render_widget(
                Paragraph::new(menu_lines).style(Style::default().bg(theme::SURFACE)),
                inner,
            );
        }
    }

    // ── 模态面板（会话 / 提供商 / 模型 / 新增提供商）──────────────────────
    let max_rows = (area.height.saturating_sub(8)).max(3) as usize;
    let panel_rect = |w: u16, h: u16| -> Rect {
        let pw = w.min(area.width.saturating_sub(4)).max(20);
        let ph = h.min(area.height.saturating_sub(2)).max(6);
        Rect {
            x: area.x + (area.width.saturating_sub(pw)) / 2,
            y: area.y + (area.height.saturating_sub(ph)) / 2,
            width: pw,
            height: ph,
        }
    };

    if state.mode == InteractionMode::SelectingSession {
        let total = state.session_list.len();
        let shown = total.min(10.min(max_rows)).max(1);
        let selected = state
            .selected_session_index
            .min(total.saturating_sub(1));
        let (start, end) = panel_window(total, selected, shown);
        let title = if total > shown {
            format!("会话 · {}/{}", selected + 1, total)
        } else {
            "会话".to_string()
        };
        let rect = panel_rect(84, shown as u16 + 4);
        let body = draw_panel(f, rect, &title, "↑↓ 选择 · ↵ 确认 · esc 取消");
        let inner_w = body.width as usize;
        let mut lines_out: Vec<Line<'static>> = Vec::new();
        if state.session_list.is_empty() {
            lines_out.push(Line::from(Span::styled("  暂无历史会话", faint())));
        } else {
            for (idx, item) in state.session_list.iter().enumerate().take(end).skip(start) {
                let selected_row = idx == selected;
                let id_short: String = item
                    .id
                    .rsplit('-')
                    .next()
                    .unwrap_or(&item.id)
                    .chars()
                    .take(8)
                    .collect();
                let active = item.id == state.session_id;
                let right = if active {
                    format!("{} · 当前", id_short)
                } else {
                    id_short
                };
                let title = truncate_width(&item.title, inner_w.saturating_sub(18));
                lines_out.push(list_row(selected_row, &title, &right, inner_w));
            }
        }
        f.render_widget(
            Paragraph::new(lines_out).style(Style::default().bg(theme::SURFACE)),
            body,
        );
    }

    if state.mode == InteractionMode::SelectingProvider {
        let total = state.provider_list.len();
        let shown = total.min(10.min(max_rows)).max(1);
        let selected = state
            .selected_provider_index
            .min(total.saturating_sub(1));
        let (start, end) = panel_window(total, selected, shown);
        let title = if total > shown {
            format!("提供商 · {}/{}", selected + 1, total)
        } else {
            "提供商".to_string()
        };
        let rect = panel_rect(84, shown as u16 + 4);
        let body = draw_panel(f, rect, &title, "↑↓ 选择 · ↵ 选模型 · a 新增 · d 删除 · esc 取消");
        let inner_w = body.width as usize;
        let mut lines_out: Vec<Line<'static>> = Vec::new();
        if state.provider_list.is_empty() {
            lines_out.push(Line::from(Span::styled(
                "  尚未配置，按 a 新增",
                faint(),
            )));
        } else {
            for (idx, item) in state.provider_list.iter().enumerate().take(end).skip(start) {
                let selected_row = idx == selected;
                let active = item.id == state.current_provider_id;
                let right = if active {
                    format!("{} · 当前", item.protocol)
                } else {
                    item.protocol.clone()
                };
                lines_out.push(list_row(selected_row, &item.name, &right, inner_w));
            }
        }
        f.render_widget(
            Paragraph::new(lines_out).style(Style::default().bg(theme::SURFACE)),
            body,
        );
    }

    if state.mode == InteractionMode::SelectingModel {
        let total = state.model_list.len();
        let shown = total.min(12.min(max_rows)).max(1);
        let selected = state
            .selected_model_index
            .min(total.saturating_sub(1));
        let (start, end) = panel_window(total, selected, shown);
        let title = if total > shown {
            format!(
                "模型 · {}/{} · 优选 {}",
                selected + 1,
                total,
                state.model_preferences.preferred.len()
            )
        } else {
            format!("模型 · 优选 {}", state.model_preferences.preferred.len())
        };
        // Ctrl+F 优选项整体前置于列表之上：跨 provider 的快捷集合，且不改变键盘导航索引空间
        //（↑↓ 仍遍历完整列表，避免同一模型在两个栏位重复出现导致选中映射错乱）。
        let preferred_items = model_preferred_items(&state);
        let pref_rows = if preferred_items.is_empty() {
            0
        } else {
            preferred_items.len() + 2
        };
        let rect = panel_rect(84, (shown + pref_rows) as u16 + 4);
        let body = draw_panel(
            f,
            rect,
            &title,
            "↑↓ 选择 · ↵ 确认 · ⌃D 设默认 · ⌃F 优选 · r 刷新 · esc 取消",
        );
        let inner_w = body.width as usize;
        let mut lines_out: Vec<Line<'static>> = Vec::new();
        if !preferred_items.is_empty() {
            lines_out.push(Line::from(Span::styled(" 优选", faint())));
            for item in &preferred_items {
                lines_out.push(list_row(
                    false,
                    &model_display_name(item),
                    &model_badges(&state, item),
                    inner_w,
                ));
            }
            lines_out.push(Line::from(Span::styled(" 全部模型", faint())));
        }
        if state.model_list.is_empty() {
            lines_out.push(Line::from(Span::styled(
                "  当前提供商没有可用模型",
                faint(),
            )));
        } else {
            for (idx, item) in state.model_list.iter().enumerate().take(end).skip(start) {
                let selected_row = idx == selected;
                let right = model_badges(&state, item);
                lines_out.push(list_row(
                    selected_row,
                    &model_display_name(item),
                    &right,
                    inner_w,
                ));
            }
        }
        f.render_widget(
            Paragraph::new(lines_out).style(Style::default().bg(theme::SURFACE)),
            body,
        );
    }

    if state.mode == InteractionMode::SelectingReasoningEffort {
        let total = REASONING_EFFORT_OPTIONS.len();
        let shown = total.min(10.min(max_rows)).max(1);
        let selected = state
            .selected_reasoning_effort_index
            .min(total.saturating_sub(1));
        let (start, end) = panel_window(total, selected, shown);
        let title = "思考强度 (Reasoning Effort)".to_string();
        let rect = panel_rect(76, shown as u16 + 4);
        let body = draw_panel(f, rect, &title, "↑↓ 选择 · ↵ 确认 · esc 取消");
        let inner_w = body.width as usize;
        let mut lines_out: Vec<Line<'static>> = Vec::new();
        for (idx, (opt_eff, name, desc)) in REASONING_EFFORT_OPTIONS.iter().enumerate().take(end).skip(start) {
            let selected_row = idx == selected;
            let active = opt_eff == &state.reasoning_effort;
            let right = if active {
                format!("{} · 当前", desc)
            } else {
                desc.to_string()
            };
            lines_out.push(list_row(selected_row, name, &right, inner_w));
        }
        f.render_widget(
            Paragraph::new(lines_out).style(Style::default().bg(theme::SURFACE)),
            body,
        );
    }

    if state.mode == InteractionMode::AddingProvider {
        let rect = panel_rect(64, 9);
        let body = draw_panel(f, rect, "新增提供商", "回车 下一步 · esc 取消");
        let inner_w = body.width as usize;
        let (step, prompt) = match state.add_provider_step {
            AddingProviderStep::Name => ("1 / 3  name", "Provider 名称"),
            AddingProviderStep::BaseUrl => ("2 / 3  base url", "Base URL（http:// 或 https://）"),
            AddingProviderStep::ApiKey => ("3 / 3  api key", "API Key（本地服务可留空）"),
            AddingProviderStep::Commit => ("saving", "正在保存…"),
        };
        let mut rows: Vec<Line<'static>> = Vec::new();
        rows.push(Line::from(vec![
            Span::styled(format!("  {:<18}", step), faint()),
            Span::styled(prompt.to_string(), muted()),
        ]));
        // 已填字段作为上下文回显：分步表单不要求用户记住上一步输入。
        match state.add_provider_step {
            AddingProviderStep::Name => {}
            AddingProviderStep::BaseUrl => {
                rows.push(Line::from(Span::styled(
                    truncate_width(&format!("  {}", state.add_provider_form.name), inner_w),
                    faint(),
                )));
            }
            AddingProviderStep::ApiKey | AddingProviderStep::Commit => {
                let ctx = format!(
                    "  {}  ·  {}",
                    state.add_provider_form.name, state.add_provider_form.base_url
                );
                rows.push(Line::from(Span::styled(
                    truncate_width(&ctx, inner_w),
                    faint(),
                )));
            }
        }
        let value = if state.add_provider_step == AddingProviderStep::ApiKey && !state.input_buffer.is_empty()
        {
            "•".repeat(state.input_buffer.chars().count())
        } else {
            state.input_buffer.clone()
        };
        let caret = if state.spinner_tick % 2 == 0 { CARET } else { "" };
        rows.push(Line::from(vec![
            Span::styled("  › ", Style::default().fg(theme::ACCENT)),
            Span::styled(value, ink()),
            Span::styled(caret, Style::default().fg(theme::ACCENT)),
        ]));
        if !state.status_text.is_empty()
            && (state.status_text.contains("错误") || state.status_text.contains("不能"))
        {
            rows.push(Line::from(Span::styled(
                truncate_width(&format!("  {}", state.status_text), inner_w),
                Style::default().fg(theme::DANGER),
            )));
        }
        f.render_widget(
            Paragraph::new(rows).style(Style::default().bg(theme::SURFACE)),
            body,
        );
    }

    // ── 进阶交互组件渲染空桩（由 Executor 在阶段 3 完善实际 UI） ──
    if state.mode == InteractionMode::PermissionApproval {
        render_permission_dialog_stub(f, state);
    }
    if state.mode == InteractionMode::PlanMode {
        render_plan_panel_stub(f, state);
    }
    if state.mode == InteractionMode::TaskBoard {
        render_task_board_stub(f, state);
    }
    if state.mode == InteractionMode::AgentRoster {
        render_agent_roster_panel(f, state);
    }
}

/// 面板在无数据时的统一空状态：不允许整屏空白（那会让用户误判 TUI 挂了）。
fn render_pending_empty_state(f: &mut Frame, title: &str, body: &str, hint: &str) {
    let area = f.area();
    let pw = 72u16.min(area.width.saturating_sub(4)).max(20);
    let ph = 8u16.min(area.height.saturating_sub(2)).max(6);
    let rect = Rect {
        x: area.x + (area.width.saturating_sub(pw)) / 2,
        y: area.y + (area.height.saturating_sub(ph)) / 2,
        width: pw,
        height: ph,
    };
    let inner = draw_panel(f, rect, title, hint);
    f.render_widget(
        Paragraph::new(vec![
            blank(),
            Line::from(Span::styled(format!("  {body}"), faint())),
        ])
        .style(Style::default().bg(theme::SURFACE)),
        inner,
    );
}

pub fn render_permission_dialog_stub(f: &mut Frame, state: &TuiState) {
    let perm = match &state.permission_state {
        Some(p) => p,
        None => {
            render_pending_empty_state(
                f,
                "权限审批请求 (Permission Request)",
                "当前没有待审批的工具调用",
                "Esc 返回",
            );
            return;
        }
    };
    let area = f.area();
    let pw = 76u16.min(area.width.saturating_sub(4)).max(20);
    let ph = 14u16.min(area.height.saturating_sub(2)).max(6);
    let rect = Rect {
        x: area.x + (area.width.saturating_sub(pw)) / 2,
        y: area.y + (area.height.saturating_sub(ph)) / 2,
        width: pw,
        height: ph,
    };
    let body = draw_panel(
        f,
        rect,
        "权限审批请求 (Permission Request)",
        "Enter 确认 · Tab 切换 · Esc 拒绝",
    );
    let inner_w = body.width as usize;

    let mut lines: Vec<Line<'static>> = Vec::new();

    // 1. 工具与描述
    let tool_line = Line::from(vec![
        Span::styled("  工具: ", Style::default().fg(theme::MUTED)),
        Span::styled(
            truncate_chars(&perm.request.tool_name, inner_w.saturating_sub(10)),
            Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD),
        ),
    ]);
    lines.push(tool_line);

    let desc_line = Line::from(vec![
        Span::styled("  说明: ", Style::default().fg(theme::MUTED)),
        Span::styled(
            truncate_chars(&perm.request.description, inner_w.saturating_sub(10)),
            Style::default().fg(theme::INK),
        ),
    ]);
    lines.push(desc_line);

    // 2. 命令或路径
    if let Some(ref cmd) = perm.request.command_or_path {
        let cmd_line = Line::from(vec![
            Span::styled("  目标: ", Style::default().fg(theme::MUTED)),
            Span::styled(
                truncate_chars(cmd, inner_w.saturating_sub(10)),
                Style::default().fg(theme::INK_SOFT),
            ),
        ]);
        lines.push(cmd_line);
    }
    lines.push(blank());

    // 3. 审批选项列表
    let choices = &perm.choices;
    for (idx, choice) in choices.iter().enumerate() {
        let selected = idx == perm.selected_choice_index;
        let (label, desc) = match choice {
            PermissionChoice::AllowOnce => ("AllowOnce (允许一次)", "仅允许当前单次工具执行"),
            PermissionChoice::AllowSession => ("AllowSession (永久允许)", "在此会话中永久放行该工具"),
            PermissionChoice::Deny => ("Deny (拒绝执行)", "拦截并取消本次工具调用"),
        };
        let row = list_row(selected, label, desc, inner_w);
        lines.push(row);
    }

    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme::SURFACE)),
        body,
    );
}

/// Ask 面板的呼吸节奏：2 帧交替（每帧约 450ms），只在问答等待期生效。
fn ask_pulse_glyph(tick: usize) -> &'static str {
    if tick % 2 == 0 {
        "◆"
    } else {
        "◇"
    }
}

/// 内联 Ask 面板高度：分隔线 + 标题 + 选项(含自定义输入) + 键位提示；无问答时为 0。
pub fn inline_ask_height(state: &TuiState, width: usize) -> u16 {
    if state.mode != InteractionMode::AskUserQuestion {
        return 0;
    }
    let Some(ask) = &state.ask_user_state else {
        return 0;
    };
    let option_rows = ask
        .current_question()
        .map(|q| q.options.len() + 1) // +1 自定义输入项
        .unwrap_or(0);
    let cap = if width >= 60 { 12 } else { 8 };
    (2 + option_rows as u16 + 1).min(cap).max(4)
}

/// 单行选项：`▏ ● 标签   说明`。焦点行整行 SURFACE_SEL + INK BOLD，
/// 说明文字跟在标签后（不再右对齐拉满一行），窄终端自然省略。
fn ask_option_row(
    focused: bool,
    marker: &str,
    label: &str,
    description: &str,
    width: usize,
) -> Line<'static> {
    let bg = if focused {
        theme::SURFACE_SEL
    } else {
        theme::CANVAS
    };
    let bar_style = if focused {
        Style::default().fg(theme::ACCENT).bg(bg)
    } else {
        Style::default().bg(bg)
    };
    let label_style = if focused {
        Style::default()
            .fg(theme::INK)
            .bg(bg)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme::INK_SOFT).bg(bg)
    };
    let marker_style = if focused {
        Style::default().fg(theme::ACCENT).bg(bg)
    } else {
        Style::default().fg(theme::MUTED).bg(bg)
    };

    // 列预算：rail(2) + marker(2) + 间隔(1) + 说明前的 2 空格。
    let fixed = 1 + RAIL + 1;
    let desc_budget = width.saturating_sub(fixed + str_width(label) + 3);
    let desc = if description.trim().is_empty() {
        String::new()
    } else {
        ellipsize_width(description.trim(), desc_budget.max(6))
    };
    let desc_w = if desc.is_empty() { 0 } else { str_width(&desc) + 2 };
    let label_budget = width.saturating_sub(fixed + desc_w).max(6);
    let label = ellipsize_width(label, label_budget);
    let used = fixed + str_width(&label) + desc_w;

    let mut spans = vec![
        Span::raw(" "),
        Span::styled(if focused { "▌" } else { " " }, bar_style),
        Span::raw(" "),
        Span::styled(format!("{marker} "), marker_style),
        Span::styled(label, label_style),
    ];
    if !desc.is_empty() {
        spans.push(Span::styled("  ", Style::default().bg(bg)));
        let desc_style = if focused {
            Style::default().fg(theme::INK_SOFT).bg(bg)
        } else {
            Style::default().fg(theme::MUTED).bg(bg)
        };
        spans.push(Span::styled(desc, desc_style));
    }
    spans.push(Span::styled(
        " ".repeat(width.saturating_sub(used)),
        Style::default().bg(bg),
    ));
    Line::from(spans)
}

/// 信息流底部的内联问答面板（无边框、无遮罩，与正文同一 CANVAS 底色）。
pub fn render_ask_inline(f: &mut Frame, area: Rect, state: &TuiState) {
    let Some(ask) = &state.ask_user_state else {
        return;
    };
    fill(f, area, theme::CANVAS);
    let width = area.width as usize;
    let mut lines: Vec<Line<'static>> = Vec::new();

    // ── 顶部发丝线：把问答面板钉在信息流底部，而不是浮在半空 ─────────────────
    if area.height >= 2 {
        lines.push(hairline(width));
    }

    let pulse = ask_pulse_glyph(state.spinner_tick);
    let question = ask
        .current_question()
        .map(|q| q.question.trim().replace('\n', " "))
        .unwrap_or_default();
    // 分组标签只展示真正面向用户的短词；运行时内部 id（`builtin:ask_user`）不暴露到界面。
    let header = ask
        .current_question()
        .and_then(|q| q.header.clone())
        .filter(|header| !header.trim().is_empty())
        .filter(|header| !header.contains(':') && !header.starts_with("builtin"));
    let counter = ask
        .current_question()
        .map(|_| (ask.current_question_index + 1, ask.questions.len()))
        .filter(|(_, total)| *total > 1)
        .map(|(index, total)| format!("{index}/{total}"));

    // ── 标题：呼吸标记 + 提问 chip + 分组 chip + 明亮陶土色问题正文 + 多问题计数 ─────────────
    let mut title_spans: Vec<Span<'static>> = vec![
        Span::raw(" ".repeat(RAIL)),
        Span::styled(
            pulse,
            Style::default()
                .fg(theme::TERRACOTTA_BRIGHT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            "提问",
            Style::default()
                .fg(theme::TERRACOTTA_BRIGHT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
    ];
    let mut used = RAIL + 2 + 1 + 4 + 1;
    if let Some(header) = header {
        let chip = format!("{header} ");
        used += str_width(&chip);
        title_spans.push(Span::styled(chip, muted()));
    }
    let tail = counter
        .as_ref()
        .map(|counter| format!("  {counter}"))
        .unwrap_or_default();
    let question_budget = width.saturating_sub(used + str_width(&tail)).max(8);
    title_spans.push(Span::styled(
        ellipsize_width(&question, question_budget),
        Style::default()
            .fg(theme::TERRACOTTA_BRIGHT)
            .add_modifier(Modifier::BOLD),
    ));
    if !tail.is_empty() {
        title_spans.push(Span::styled(tail, faint()));
    }
    lines.push(Line::from(title_spans));

    // ── 选项 + 自定义输入 ───────────────────────────────────────────────────
    let mut custom_focused = false;
    if let Some(q) = ask.current_question() {
        for (idx, option) in q.options.iter().enumerate() {
            if lines.len() + 1 >= area.height as usize {
                break;
            }
            let focused = idx == ask.selected_option_index;
            let marker = if q.multi_select {
                if ask.multi_selected_indices.contains(&idx) {
                    "▣"
                } else {
                    "▢"
                }
            } else if focused {
                "●"
            } else {
                "○"
            };
            lines.push(ask_option_row(
                focused,
                marker,
                &option.label,
                option.description.as_deref().unwrap_or(""),
                width,
            ));
        }

        if lines.len() + 1 < area.height as usize {
            custom_focused = ask.selected_option_index == q.options.len();
            // 聚焦时在文本尾部显示闪烁的半角细竖线光标（▏），半角且呼吸闪烁。
            let caret = if custom_focused && state.spinner_tick % 2 == 0 {
                CARET
            } else {
                ""
            };
            let label = if ask.custom_input.is_empty() {
                if custom_focused {
                    format!("自定义输入{caret}")
                } else {
                    "自定义输入…".to_string()
                }
            } else if custom_focused {
                format!("{}{caret}", ask.custom_input)
            } else {
                ask.custom_input.clone()
            };
            let description = if custom_focused { "↵ 提交" } else { "" };
            lines.push(ask_option_row(
                custom_focused,
                ">",
                &label,
                description,
                width,
            ));
        }
    } else {
        lines.push(Line::from(Span::styled("  无等待确认的问题", faint())));
    }

    // ── 键位提示：按当前状态给最短的一组 ─────────────────────────────────────
    if lines.len() < area.height as usize {
        let multi = ask
            .current_question()
            .map(|q| q.multi_select)
            .unwrap_or(false);
        let has_options = ask
            .current_question()
            .map(|question| !question.options.is_empty())
            .unwrap_or(false);
        let hint = if !has_options {
            "输入回答 · ↵ 提交 · esc 取消"
        } else if custom_focused {
            "输入自定义回答 · ↵ 提交 · esc 取消"
        } else if multi {
            "↑↓ 移动 · space 勾选 · ↵ 提交 · esc 取消"
        } else {
            "↑↓ 移动 · ↵ 确认 · esc 取消"
        };
        lines.push(Line::from(vec![
            Span::raw(" ".repeat(RAIL + 1)),
            Span::styled(hint, faint()),
        ]));
    }

    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme::CANVAS)),
        area,
    );
}

/// Ask 态下色带输入区的替身：一行「等待选择」，不重复面板里的键位提示。
fn render_ask_band_hint(f: &mut Frame, area: Rect, state: &TuiState) {
    let pulse = ask_pulse_glyph(state.spinner_tick);
    let line = Line::from(vec![
        Span::raw(" ".repeat(RAIL)),
        Span::styled(
            pulse,
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled("›", Style::default().fg(theme::ACCENT)),
        Span::raw(" "),
        Span::styled("等待你的选择…", Style::default().fg(theme::MUTED)),
    ]);
    f.render_widget(
        Paragraph::new(vec![line, blank()]).style(Style::default().bg(theme::SURFACE)),
        area,
    );
}

pub fn render_ask_user_dialog_stub(f: &mut Frame, state: &TuiState) {
    let ask = match &state.ask_user_state {
        Some(a) => a,
        None => {
            render_pending_empty_state(
                f,
                "用户输入确认 (Ask User)",
                "当前没有待回答的提问",
                "Esc 返回",
            );
            return;
        }
    };
    let area = f.area();
    let pw = 76u16.min(area.width.saturating_sub(4)).max(20);
    let ph = 16u16.min(area.height.saturating_sub(2)).max(6);
    let rect = Rect {
        x: area.x + (area.width.saturating_sub(pw)) / 2,
        y: area.y + (area.height.saturating_sub(ph)) / 2,
        width: pw,
        height: ph,
    };
    let body = draw_panel(
        f,
        rect,
        "用户输入确认 (Ask User)",
        "Space 勾选 · Enter 提交 · Esc 取消",
    );
    let inner_w = body.width as usize;

    let mut lines: Vec<Line<'static>> = Vec::new();

    if let Some(q) = ask.current_question() {
        if let Some(ref header) = q.header {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  [{}]", truncate_chars(header, inner_w.saturating_sub(8))),
                    Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD),
                ),
            ]));
        }
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {}", q.question),
                Style::default().fg(theme::INK).add_modifier(Modifier::BOLD),
            ),
        ]));
        lines.push(blank());

        // 渲染候选项
        for (idx, opt) in q.options.iter().enumerate() {
            let is_focused = idx == ask.selected_option_index;
            let check_mark = if q.multi_select {
                if ask.multi_selected_indices.contains(&idx) {
                    "[x] "
                } else {
                    "[ ] "
                }
            } else {
                if is_focused { "(●) " } else { "(○) " }
            };

            let left = format!("  {}{}", check_mark, opt.label);
            let right = opt.description.as_deref().unwrap_or("");
            lines.push(list_row(is_focused, &left, right, inner_w));
        }

        // 自定义输入项
        let custom_idx = q.options.len();
        let is_custom_focused = ask.selected_option_index == custom_idx;
        let caret = if is_custom_focused && state.spinner_tick % 2 == 0 {
            CARET
        } else {
            ""
        };
        let custom_label = if ask.custom_input.is_empty() {
            if is_custom_focused {
                format!("  ✎ 自定义输入{caret}")
            } else {
                "  ✎ 自定义输入...".to_string()
            }
        } else if is_custom_focused {
            format!("  ✎ {}{caret}", ask.custom_input)
        } else {
            format!("  ✎ {}", ask.custom_input)
        };
        lines.push(list_row(
            is_custom_focused,
            &custom_label,
            if is_custom_focused { "直接键入文本" } else { "" },
            inner_w,
        ));
    } else {
        lines.push(Line::from(Span::styled("  无等待确认的问题", faint())));
    }

    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme::SURFACE)),
        body,
    );
}

pub fn render_plan_panel_stub(f: &mut Frame, state: &TuiState) {
    let plan = match &state.plan_state {
        Some(p) => p,
        None => {
            render_pending_empty_state(
                f,
                "Plan 计划模式与步骤展示",
                "当前会话还没有计划；模型调用 Plan 后会实时显示步骤进度",
                "/plan 或 Esc 退出计划模式",
            );
            return;
        }
    };
    let area = f.area();
    let pw = 80u16.min(area.width.saturating_sub(4)).max(20);
    let ph = 18u16.min(area.height.saturating_sub(2)).max(6);
    let rect = Rect {
        x: area.x + (area.width.saturating_sub(pw)) / 2,
        y: area.y + (area.height.saturating_sub(ph)) / 2,
        width: pw,
        height: ph,
    };
    let body = draw_panel(
        f,
        rect,
        "Plan 计划模式与步骤展示",
        "/plan 或 Esc 退出计划模式",
    );
    let inner_w = body.width as usize;

    let mut lines: Vec<Line<'static>> = Vec::new();

    // 计划概要/总纲
    lines.push(Line::from(vec![
        Span::styled(
            format!("  实施计划: {}", plan.summary),
            Style::default().fg(theme::INK).add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(blank());

    // 渲染各步骤
    if plan.steps.is_empty() {
        lines.push(Line::from(Span::styled("  尚无正在执行的步骤", faint())));
    } else {
        for (idx, step) in plan.steps.iter().enumerate() {
            let is_focused = idx == plan.selected_step_index;
            let (icon, _icon_style) = match step.status {
                PlanStepStatus::Completed => ("✔", Style::default().fg(theme::ACCENT)),
                PlanStepStatus::InProgress => ("●", Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD)),
                PlanStepStatus::Pending => ("○", Style::default().fg(theme::MUTED)),
                PlanStepStatus::Failed => ("✘", Style::default().fg(theme::MUTED)),
            };

            let left = format!("  {} {}. {}", icon, idx + 1, step.name);
            let right = step.summary.as_str();
            lines.push(list_row(is_focused, &left, right, inner_w));
        }
    }

    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme::SURFACE)),
        body,
    );
}

pub fn render_task_board_stub(f: &mut Frame, state: &TuiState) {
    let board = match &state.task_board_state {
        Some(b) => b,
        None => {
            render_pending_empty_state(
                f,
                "Team 共享任务看板",
                "当前没有任务；Agent 调用 team_task_create 后会实时汇总到这里",
                "Tab 折叠/展开 · Esc 退出",
            );
            return;
        }
    };
    let area = f.area();
    let pw = 84u16.min(area.width.saturating_sub(4)).max(20);
    let ph = 20u16.min(area.height.saturating_sub(2)).max(6);
    let rect = Rect {
        x: area.x + (area.width.saturating_sub(pw)) / 2,
        y: area.y + (area.height.saturating_sub(ph)) / 2,
        width: pw,
        height: ph,
    };
    let body = draw_panel(
        f,
        rect,
        "Team 共享任务看板",
        "Tab 折叠/展开 · ↑↓ 浏览 · Esc 退出",
    );
    let inner_w = body.width as usize;

    let mut lines: Vec<Line<'static>> = Vec::new();

    // 统计各状态数量
    let total = board.tasks.len();
    let completed_count = board.tasks.iter().filter(|t| t.status == TuiTaskStatus::Completed).count();
    let in_progress_count = board.tasks.iter().filter(|t| t.status == TuiTaskStatus::InProgress).count();
    let pending_count = board.tasks.iter().filter(|t| t.status == TuiTaskStatus::Pending).count();
    let _blocked_count = board.tasks.iter().filter(|t| t.status == TuiTaskStatus::Blocked).count();

    // 汇总状态头
    let summary_text = format!(
        "  任务进度: {}/{} · 已完成: {} · 进行中: {} · 待办: {}",
        completed_count, total, completed_count, in_progress_count, pending_count
    );
    lines.push(Line::from(vec![
        Span::styled(
            summary_text,
            Style::default().fg(theme::INK).add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(blank());

    if board.tasks.is_empty() {
        lines.push(Line::from(Span::styled("  暂无共享任务", faint())));
    } else {
        // 渲染任务条目
        let max_visible_items = ph.saturating_sub(6) as usize;
        let selected = board.selected_index.min(total.saturating_sub(1));
        let (start, end) = panel_window(total, selected, max_visible_items.max(1));

        for (idx, task) in board.tasks.iter().enumerate().take(end).skip(start) {
            let is_focused = idx == selected;
            let status_mark = match task.status {
                TuiTaskStatus::Completed => "[✔]",
                TuiTaskStatus::InProgress => "[●]",
                TuiTaskStatus::Pending => "[○]",
                TuiTaskStatus::Blocked => "[✕]",
            };

            let owner_str = task.owner.as_deref().unwrap_or("unassigned");
            // W3：TeamTaskItem 的 blocked_by / write_scopes 也进面板（阻塞数 + 写域数），
            // 让"团队看板"不再只显示 owner。
            let blocked_suffix = if task.blocked_by.is_empty() {
                String::new()
            } else {
                format!(" · ⛔{}", task.blocked_by.len())
            };
            let scopes_suffix = if task.write_scopes.is_empty() {
                String::new()
            } else {
                format!(" · 写域{}", task.write_scopes.len())
            };
            let left = format!(
                "  {} {} · {}{}{}",
                status_mark, task.id, task.subject, blocked_suffix, scopes_suffix
            );
            let right = format!("@{}", owner_str);
            lines.push(list_row(is_focused, &left, &right, inner_w));
        }

        if total > end {
            lines.push(Line::from(Span::styled(
                format!("  ... 另有 {} 个任务项未展开", total - end),
                faint(),
            )));
        }
    }

    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme::SURFACE)),
        body,
    );
}

/// W3：`/agents` Team 角色与状态面板。
///
/// 每行：`NAME · ROLE · STATUS · 待处理 N · 会话 xxxx`；`driving` 用 `●` 标记后台驱动中。
/// 空表（无 teammate）→ 明确的「当前没有 teammate」空状态，绝不整屏空白。
pub fn render_agent_roster_panel(f: &mut Frame, state: &TuiState) {
    let roster = match &state.agent_roster_state {
        Some(roster) => roster,
        None => {
            render_pending_empty_state(
                f,
                "Team 角色与状态 (/agents)",
                "当前没有 teammate",
                "使用 spawn_teammate 派生队友后，此处显示真实名册 · Esc 关闭",
            );
            return;
        }
    };
    let area = f.area();
    let pw = 84u16.min(area.width.saturating_sub(4)).max(20);
    let ph = 20u16.min(area.height.saturating_sub(2)).max(6);
    let rect = Rect {
        x: area.x + (area.width.saturating_sub(pw)) / 2,
        y: area.y + (area.height.saturating_sub(ph)) / 2,
        width: pw,
        height: ph,
    };
    let body = draw_panel(
        f,
        rect,
        "Team 角色与状态 (/agents)",
        "↑↓ 选择 · Enter 进入队友会话 · r 返回主会话 · Esc 关闭",
    );
    let inner_w = body.width as usize;

    let mut lines: Vec<Line<'static>> = Vec::new();
    let total = roster.entries.len();
    let driving_count = roster.entries.iter().filter(|entry| entry.driving).count();
    let pending_total: usize = roster.entries.iter().map(|entry| entry.pending_inbox).sum();
    lines.push(Line::from(vec![Span::styled(
        format!(
            "  队友 {} · 驱动中 {} · 待处理消息 {}",
            total, driving_count, pending_total
        ),
        Style::default().fg(theme::INK).add_modifier(Modifier::BOLD),
    )]));
    lines.push(blank());

    if total == 0 {
        lines.push(Line::from(Span::styled("  当前没有 teammate", faint())));
    } else {
        let max_visible_items = ph.saturating_sub(6) as usize;
        let selected = roster.selected.min(total.saturating_sub(1));
        let (start, end) = panel_window(total, selected, max_visible_items.max(1));

        for (idx, entry) in roster.entries.iter().enumerate().take(end).skip(start) {
            let is_focused = idx == selected;
            let drive_mark = if entry.driving { "●" } else { "○" };
            let left = format!(
                "  {} {} · {} · {} · 待处理 {}",
                drive_mark, entry.name, entry.role, entry.status, entry.pending_inbox
            );
            let right = match entry.session_id.as_deref() {
                Some(session_id) => format!("会话 {}", session_tail(session_id)),
                None => "无会话".to_string(),
            };
            lines.push(list_row(is_focused, &left, &right, inner_w));
        }

        if total > end {
            lines.push(Line::from(Span::styled(
                format!("  ... 另有 {} 个队友未展开", total - end),
                faint(),
            )));
        }

        // 选中项详情：target + 完整会话 id（尾号之外的定位信息）。
        if let Some(entry) = roster.entries.get(selected) {
            lines.push(blank());
            let session_detail = entry
                .session_id
                .as_deref()
                .map(|session_id| format!("会话 {session_id}"))
                .unwrap_or_else(|| "尚无独立会话（未成功运行）".to_string());
            lines.push(Line::from(Span::styled(
                format!(
                    "  target={} · 待处理消息 {} · {}",
                    entry.target, entry.pending_inbox, session_detail
                ),
                faint(),
            )));
        }
    }

    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme::SURFACE)),
        body,
    );
}

/// 契约（修订13）：审计用 Ask 引导开关（fail-closed）。
///
/// **整条引导路径（含开关本身）都在 `#[cfg(debug_assertions)]` 下**：release 构建里
/// 这个函数、解析函数与派发函数都不存在，环境变量无法开后门。
#[cfg(debug_assertions)]
pub fn audit_ask_bootstrap_enabled() -> bool {
    cfg!(debug_assertions) && std::env::var("PONY_AUDIT_ASK_BOOTSTRAP").is_ok()
}

/// 契约（修订13）：解析 `PONY_AUDIT_ASK_BOOTSTRAP` 的 JSON（`{"prompt":…,"options":[…]}`）。
///
/// 解析失败一律 `None` —— 调用方按「未启用」处理（fail-closed），不猜、不兜底造问题。
#[cfg(debug_assertions)]
fn audit_ask_bootstrap_arguments() -> Option<serde_json::Value> {
    let raw = std::env::var("PONY_AUDIT_ASK_BOOTSTRAP").ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let prompt = parsed.get("prompt")?.as_str()?.trim().to_string();
    if prompt.is_empty() {
        return None;
    }
    // 工具 schema 统一要求 `description`（`with_description` 注入的必填字段，模型每次调用都会带），
    // 引导也必须给出，否则在参数校验阶段就被拒（invalid_arguments），永远到不了 WaitingHost。
    let description = parsed
        .get("description")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("审计引导：拉起一次 Ask 闭环");
    let mut arguments = serde_json::json!({
        "question": prompt,
        "description": description,
    });
    if let Some(options) = parsed.get("options").and_then(|value| value.as_array()) {
        // 生产 schema 里 `ask_user.options` 是 **string 数组**（见 tools.rs 的 input_schema）。
        // 审计 env 允许写 {"label":…,"description":…} 便于人读，这里统一降维成 label 字符串；
        // 无法识别的条目 ⇒ 整体 fail-closed（返回 None），不猜。
        let mut labels: Vec<String> = Vec::with_capacity(options.len());
        for option in options {
            match option {
                serde_json::Value::String(label) => labels.push(label.clone()),
                serde_json::Value::Object(map) => {
                    labels.push(map.get("label")?.as_str()?.to_string());
                }
                _ => return None,
            }
        }
        if !labels.is_empty() {
            arguments["options"] =
                serde_json::Value::Array(labels.into_iter().map(serde_json::Value::String).collect());
        }
    }
    Some(arguments)
}

/// 契约（修订13）：用**生产 API** 造一条真实 pending Ask，之后什么都不做。
///
/// `dispatch_governed("builtin:ask_user")` 经既有治理管道判定 `WaitingHost` →
/// `persist_pending_request` 写入 kind=`Interaction` 的 PendingControlRequest；
/// 随后的面板拉起完全依赖既有 500ms `list_pending_asks` 轮询，
/// 提交走既有 `submit_ask_answers` → `answer_ask` 的真实 CAS。
/// **不往任何队列塞东西、不合成 request id、不绕过 CAS**。
#[cfg(debug_assertions)]
fn audit_ask_bootstrap(control_plane: &HostControlPlane, session_id: &str) {
    let Some(arguments) = audit_ask_bootstrap_arguments() else {
        eprintln!(
            "[pony-tui][audit] PONY_AUDIT_ASK_BOOTSTRAP 解析失败，按未启用处理（fail-closed）"
        );
        return;
    };
    let context = pony_agent_core::agent::dispatcher::DispatchContext {
        session_id: Some(session_id.to_string()),
        host_control_available: true,
        ..Default::default()
    };
    let outcome = control_plane.ask_dispatcher.dispatch_governed(
        pony_agent_core::agent::tool_runtime::ToolDispatchRequest {
            origin: pony_agent_core::agent::tool_runtime::InvocationOrigin::Model,
            descriptor_id: "builtin:ask_user".to_string(),
            call_id: format!("call-audit-ask-{}", uuid::Uuid::new_v4()),
            arguments,
        },
        &context,
    );
    match outcome.control_outcome {
        Some(control) if control.kind == pony_agent_core::agent::tools::ToolControlKind::WaitingHost => {
            eprintln!(
                "[pony-tui][audit] ask 引导已写入真实 pending request: {}（等待既有轮询拉起面板）",
                control.request_id
            );
        }
        other => eprintln!(
            "[pony-tui][audit] ask 引导未产生 WaitingHost（control_outcome={other:?}，\
             status={:?}，result={:?}），面板不会出现",
            outcome.execution_status,
            outcome
                .result
                .as_ref()
                .map(|result| (result.status.clone(), result.output.chars().take(200).collect::<String>()))
        ),
    }
}

/// TUI 运行日志（stderr 被 dup2 到这里）的落盘路径。
///
/// 优先级：`PONY_TUI_RUNTIME_LOG`（审计/测试注入——每个 scenario 独占一个文件，避免把历史
/// panic 算进本次结论）> `<home>/.pony-agent/tui-logs/tui-runtime.log`。
fn tui_runtime_log_path() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("PONY_TUI_RUNTIME_LOG") {
        let path = PathBuf::from(explicit);
        if !path.as_os_str().is_empty() {
            return Some(path);
        }
    }
    Some(
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".pony-agent")
            .join("tui-logs")
            .join("tui-runtime.log"),
    )
}

fn run_tui(resume_session: Option<ResumeTarget>) -> Result<(), Box<dyn std::error::Error>> {
    unsafe {
        std::env::remove_var("NO_COLOR");
        if std::env::var("COLORTERM").is_err() {
            std::env::set_var("COLORTERM", "truecolor");
        }
    }
    // wave-14：非 TTY 环境（stdin 已关闭 / tmux 非交互管道）下 enable_raw_mode 会报
    // "No such device or address (os error 6)" 且诊断信息会被后续 dup2 吞进日志文件，用户侧零输出。
    // 必须在 dup2 重定向之前提前优雅退出并直接向真实 stderr 打印诊断信息。
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        eprintln!(
            "pony TUI: stdin/stdout 不是交互式终端，TUI 无法启动。\n请在真实终端中运行 `pony`，或使用 `ponycli`（非交互 CLI 入口）。"
        );
        std::process::exit(1);
    }

    // 将底层 Rust 内核对 stderr/stdout 的日志直接重定向到文件或忽略，
    // 避免 [pony-runtime]、[pony-provider] 等原生 println/eprintln 打印污染 crossterm 屏幕，
    // 防止全屏 TUI 界面被破坏退化为终端滚屏输出。
    // 日志绑定只在 unix 分支使用（dup2 重定向）；非 unix 目标（Windows CI）不进入该分支。
    let runtime_log_path = tui_runtime_log_path();
    if let Some(path) = runtime_log_path.as_ref() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
    }
    let mut stderr_redirected = false;
    #[cfg(unix)]
    if let Some(path) = runtime_log_path.as_ref() {
        if let Ok(file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            use std::os::unix::io::AsRawFd;
            let fd = file.as_raw_fd();
            unsafe {
                libc::dup2(fd, libc::STDERR_FILENO);
            }
            stderr_redirected = true;
        }
    }
    // 结构化 JSON 日志缺省写 stdout；全屏 TUI 下会把内部协议日志直接盖到消息区。
    // 仅在 stderr 已成功重定向到运行日志后才改道，避免无落点时把日志赶向可见终端。
    if stderr_redirected {
        pony_agent_core::agent::otel::log::set_stderr_output(true);
    }

    // 契约 F-D 加固：还原通道（termios 快照 + /dev/tty 写句柄）必须在 raw mode 生效前就绪；
    // 信号接管也要提前——启动期（加载 SQLite / 构建控制面，数百毫秒级）收到 SIGHUP 时，
    // 旧实现尚未安装处理器，进程被默认终止，终端就留在 raw + 备用屏 + kitty 键盘协议态。
    capture_terminal_state();
    install_shutdown_signal_handlers();
    enable_raw_mode()?;
    // 契约 F-D：raw mode 生效后**立刻**建立 RAII 守卫（早于 Terminal::new / build_control_plane）。
    // 之后任何 `?` 提前返回或 panic 都会经 Drop 恢复终端，不会把用户终端留在 raw + 备用屏。
    // 还原闭包无条件写全还原串（含键盘增强弹栈），不再依赖"push 是否成功"的共享标志。
    install_panic_restore_hook();
    let mut terminal_guard = TerminalGuard::new(restore_terminal_modes);
    let mut stdout = io::stdout();
    if keyboard_enhancement_should_push() {
        if let Err(error) = execute!(
            stdout,
            PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                    | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
            )
        ) {
            eprintln!("[pony-tui] 键盘增强协议 push 失败（降级为传统键码）：{error}");
        }
    }

    // 契约 F-C：启用 bracketed paste —— 大段粘贴作为单个 `Event::Paste` 送达，
    // 而不是被拆成成百上千个按键（后者会触发自动补全/回车提交等误动作）。
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableMouseCapture,
        EnableBracketedPaste
    )?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut state = TuiState::default();
    // 初始化页品牌标：终端支持 sixel / kitty / iTerm2 时用真实位图，否则回退字符画。
    // 必须在进入 alternate screen 之后、读取键盘事件之前调用（picker 会瞬时读写 stdio）。
    state.splash_image = init_splash_image();
    // 启动即与注册表对齐：`selected_provider_id` / `selected_model_id` 必须真正生效，
    // 否则 header 显示的模型与实际发送口径分叉（wave-35c 遗留：sync 函数此前无调用点）。
    sync_state_with_registry(&mut state);
    // Ctrl+D 的默认模型优先于注册表默认：偏好文件缺失时保持注册表口径不变。
    state.model_preferences = ModelPreferencesStore::new().load();
    if let Some(default_model) = state.model_preferences.default_model.clone() {
        state.model = default_model;
    }
    let control_plane = Arc::new(build_control_plane(&state.workspace, &state.gateway_url, &state.model));

    // 契约（修订13）：审计专用 Ask 引导（仅 debug 构建 + 显式 env，fail-closed）。
    // 只负责「经生产 API 造一条真实 pending Ask」，面板/提交/CAS 全走既有链路。
    #[cfg(debug_assertions)]
    if audit_ask_bootstrap_enabled() {
        audit_ask_bootstrap(&control_plane, &state.session_id);
    }

    // wave-38 D1：启动时将当前 workspace 登记到控制面，并记住 id。
    state.active_workspace_id = control_plane
        .ensure_workspace_for_root(&state.workspace.display().to_string())
        .map(|record| record.id)
        .map_err(|error| eprintln!("[pony-tui] 启动目录登记为 workspace 失败：{error}"))
        .ok();
    // wave-38 D1：记住启动目录绑定，作为恢复「无归属/死归属」会话时的回退基准。
    state.startup_workspace_id = state.active_workspace_id.clone();

    // `pony resume <id>`：workspace 归属登记完成后再装载快照（hydrate 依赖 startup_workspace_id）。
    if let Some(resume) = resume_session.as_ref() {
        state.session_id = resume.session_id.clone();
        state.reset_session_scope();
        if resume.existed {
            hydrate_state_from_session(&mut state, &control_plane, &resume.session_id);
        } else {
            // 显式 id 尚无落库记录：沿用该 id 开新会话，并在界面上如实说明。
            state.status_text = format!("新会话（沿用 id）{}", resume.session_id);
            state.messages.push(ChatMessage {
                role: MessageRole::System,
                content: format!(
                    "会话 {} 尚无历史记录，已沿用该 id 开始新会话。",
                    resume.session_id
                ),
                timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
            });
        }
    }

    let (tx, rx): (Sender<UiEvent>, Receiver<UiEvent>) = channel();

    // T2c：无论主循环以何种方式退出（Ctrl-C、/quit、错误 `?` 传播、panic），
    // 都必须恢复终端（disable raw mode + LeaveAlternateScreen + DisableMouseCapture + 光标可见）。
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        tui_event_loop(&mut terminal, state, control_plane, rx, tx)
    }));

    eprintln!("[pony-tui] running terminal restore (T2c)");
    // wave-40 R3 + 契约 F-D：撤销键盘增强 / bracketed paste / raw mode / 备用屏 / 鼠标捕获的
    // 整串恢复已收敛进 TerminalGuard 的闭包；这里只调用一次（幂等），Drop 兜底异常路径。
    terminal_guard.restore();

    match result {
        Ok(inner) => {
            // 还原之后再把续接提示交给真实终端：此时备用屏已退出，提示才留在用户屏幕上。
            // 会话未落库也照常打印——`pony resume <id>` 会以同一 id 继续（不存在的 id 不是错误）。
            if let Ok(session_id) = &inner {
                print_resume_hint(session_id);
            }
            // wave-tui-session-resilience C8：还原与提示之后进程真正退出，
            // 杜绝「主循环已返回、在飞 provider 线程仍持有 /dev/tty」的残留进程。
            if inner.is_ok() {
                pony_agent_core::agent::otel::shutdown();
                std::process::exit(0);
            }
            inner.map(|_| ())
        }
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

fn run_tui_demo_ask() -> Result<(), Box<dyn std::error::Error>> {
    unsafe {
        std::env::remove_var("NO_COLOR");
        if std::env::var("COLORTERM").is_err() {
            std::env::set_var("COLORTERM", "truecolor");
        }
    }
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        eprintln!(
            "pony TUI: stdin/stdout 不是交互式终端，TUI 无法启动。"
        );
        std::process::exit(1);
    }

    let runtime_log_path = tui_runtime_log_path();
    if let Some(path) = runtime_log_path.as_ref() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
    }
    let mut stderr_redirected = false;
    #[cfg(unix)]
    if let Some(path) = runtime_log_path.as_ref() {
        if let Ok(file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            use std::os::unix::io::AsRawFd;
            let fd = file.as_raw_fd();
            unsafe {
                libc::dup2(fd, libc::STDERR_FILENO);
            }
            stderr_redirected = true;
        }
    }
    // 与 run_tui 同一收口：仅在 stderr 已落盘时改道 JSON 日志，严禁污染备用屏。
    if stderr_redirected {
        pony_agent_core::agent::otel::log::set_stderr_output(true);
    }

    // 契约 F-D 加固（与 run_tui 同一闭合）：还原通道先于 raw mode 就绪 + panic 也还原。
    // 演示入口不接管信号（其循环不查 SHUTDOWN_REQUESTED，接管会把 Ctrl-C 吞掉）。
    capture_terminal_state();
    enable_raw_mode()?;
    install_panic_restore_hook();
    let mut terminal_guard = TerminalGuard::new(restore_terminal_modes);
    let mut stdout = io::stdout();
    if keyboard_enhancement_should_push() {
        if let Err(error) = execute!(
            stdout,
            PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                    | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
            )
        ) {
            eprintln!("[pony-tui] 键盘增强协议 push 失败（降级为传统键码）：{error}");
        }
    }

    // 契约 F-C：启用 bracketed paste —— 大段粘贴作为单个 `Event::Paste` 送达，
    // 而不是被拆成成百上千个按键（后者会触发自动补全/回车提交等误动作）。
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableMouseCapture,
        EnableBracketedPaste
    )?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut state = TuiState::default();
    let control_plane = Arc::new(build_control_plane(&state.workspace, &state.gateway_url, &state.model));

    // 预注入 Ask 问题供打磨验证与截图
    let q = AskUserQuestion {
        id: "target_arch".to_string(),
        header: Some("架构与模块选择".to_string()),
        question: "请选择本次交付的执行架构与部署策略：".to_string(),
        multi_select: false,
        options: vec![
            QuestionOption {
                label: "单体轻量架构 (Standalone)".to_string(),
                description: Some("快速轻量启动，无需外部服务依赖".to_string()),
            },
            QuestionOption {
                label: "微服务分布式架构 (Microservices)".to_string(),
                description: Some("高可用集群部署，支持横向弹性扩展".to_string()),
            },
            QuestionOption {
                label: "边缘节点就近调度 (Edge Worker)".to_string(),
                description: Some("超低时延接入，贴近用户地域就近计算".to_string()),
            },
        ],
    };
    state.start_ask_user("demo-ask-101".to_string(), vec![q]);

    let (tx, rx): (Sender<UiEvent>, Receiver<UiEvent>) = channel();

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        tui_event_loop(&mut terminal, state, control_plane, rx, tx)
    }));

    // 契约 F-D：恢复统一走守卫（幂等；Drop 兜底 panic / `?` 路径，避免二次恢复）。
    terminal_guard.restore();

    match result {
        Ok(inner) => inner.map(|_| ()),
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

/// 契约 F-J：把远程发现的模型 id 写入 `view` 中指定 provider 的 `models`。
///
/// **职责边界**：只做**内存视图变换** —— 不读注册表、不落盘、不碰 UI 状态。
/// 持久化由调用方执行（`ProviderRegistryStore::save_view(view)`），面板/状态刷新同样由调用方负责。
///
/// 语义：
/// - 以该 provider 既有首个模型条目为模板克隆（保留协议 / 能力配置语义），无模板时按
///   OpenAiCompletions 兜底构造合法条目；
/// - **幂等**：同一 `(provider_id, models)` 重复调用结果一致（不追加、不改变顺序）；
/// - `provider_id` 不在 `view` 中 ⇒ `Err`，且 `view` **保持原样**（半写入会让注册表状态不可预期）。
pub fn apply_models_discovered(
    view: &mut ProviderRegistryView,
    provider_id: &str,
    models: &[String],
) -> Result<(), String> {
    let Some(provider) = view.providers.iter_mut().find(|p| p.id == provider_id) else {
        return Err(format!(
            "provider {provider_id} 不在注册表中；模型发现结果未写入"
        ));
    };
    let template = provider.models.first().cloned();
    let protocol = provider.protocol.clone();
    provider.models = models
        .iter()
        .map(|id| match &template {
            Some(t) => {
                let mut model = t.clone();
                model.id = id.clone();
                model.name = id.clone();
                model.model = id.clone();
                model
            }
            None => ProviderModelConfig {
                id: id.clone(),
                name: id.clone(),
                model: id.clone(),
                temperature: 0.2,
                max_output_tokens: 8192,
                capability_preset: ProviderCapabilityPreset::Auto,
                reasoning_effort: None,
                reasoning_budget_tokens: None,
                protocol: Some(protocol.clone()),
                base_url: None,
                capabilities: Default::default(),
            },
        })
        .collect();
    Ok(())
}

/// wave-35：后台线程远程拉取 provider 模型目录（15s 超时、自带错误语义）。
/// 复用 fetch_model_ids；成功/失败均经 UiEvent 回主循环，UI 线程不阻塞。
fn spawn_model_discovery(_state: &TuiState, tx: &Sender<UiEvent>, provider_id: &str) {
    let registry = ProviderRegistryStore::new();
    let (protocol, base_url, auth_type) = {
        let view = registry.load_view();
        match view.providers.iter().find(|p| p.id == provider_id) {
            Some(p) => (p.protocol.clone(), p.base_url.clone(), p.auth_type.clone()),
            None => {
                let _ = tx.send(UiEvent::ModelsDiscoverFailed {
                    provider_id: provider_id.to_string(),
                    error: "provider 不在注册表中".to_string(),
                });
                return;
            }
        }
    };
    let api_key = registry.provider_api_key(Some(provider_id)).unwrap_or_default();
    let provider_id = provider_id.to_string();
    let tx = tx.clone();
    std::thread::spawn(move || {
        let _guard = pony_agent_core::agent::runtime_helper::TestRuntimeGuard::new();
        match pony_agent_core::agent::provider::fetch_model_ids(&protocol, &base_url, &auth_type, &api_key) {
            Ok(ids) => {
                let _ = tx.send(UiEvent::ModelsDiscovered { provider_id, ids });
            }
            Err(error) => {
                let _ = tx.send(UiEvent::ModelsDiscoverFailed { provider_id, error });
            }
        }
    });
}

/// 契约 F-M：当前问题已选中的选项索引。
///
/// 未选中 / 选中「自定义输入」槽位 / 无选项 ⇒ `None`（让 `ask_submit_verdict` 拒绝空白提交）。
fn ask_selected_index(ask: &AskUserState) -> Option<usize> {
    let question = ask.current_question()?;
    if question.options.is_empty() || ask.is_custom_input_active {
        return None;
    }
    if question.multi_select {
        ask.multi_selected_indices.first().copied()
    } else if ask.selected_option_index < question.options.len() {
        Some(ask.selected_option_index)
    } else {
        None
    }
}

/// 契约 F-M：当前问题的答案取值（自由输入优先，其次多选 / 单选标签）。
fn ask_answer_values(ask: &AskUserState) -> Vec<String> {
    let typed = ask.custom_input.trim();
    if ask.is_custom_input_active && !typed.is_empty() {
        return vec![typed.to_string()];
    }
    let Some(question) = ask.current_question() else {
        return Vec::new();
    };
    if question.multi_select {
        ask.multi_selected_indices
            .iter()
            .filter_map(|&index| question.options.get(index).map(|option| option.label.clone()))
            .collect()
    } else {
        question
            .options
            .get(ask.selected_option_index)
            .map(|option| vec![option.label.clone()])
            .unwrap_or_default()
    }
}

/// 契约 F-M：多问答案 → 提交载荷。
///
/// 单问保持既有 `{"answer": "<首个取值>"}` 形状（向后兼容控制面/图运行的消费口径）；
/// 多问在其上追加 `answers: [{questionId, values}]`（附加键，不改旧键语义）。
fn ask_answer_json(answers: &[(String, Vec<String>)]) -> serde_json::Value {
    let first_value = answers
        .first()
        .and_then(|(_, values)| values.first().cloned())
        .unwrap_or_default();
    if answers.len() <= 1 {
        return serde_json::json!({ "answer": first_value });
    }
    serde_json::json!({
        "answer": first_value,
        "answers": answers
            .iter()
            .map(|(question_id, values)| serde_json::json!({
                "questionId": question_id,
                "values": values,
            }))
            .collect::<Vec<_>>(),
    })
}

/// 契约 F-M：**全部问题答完后**一次性提交答案并恢复任务流。
///
/// 中途推进（`NextQuestion::Advance`）不得走到这里 —— 否则控制面会拿着半份答案继续执行。
fn submit_ask_answers(
    state: &mut TuiState,
    control_plane: &pony_agent_core::agent::control_plane::HostControlPlane,
    tx: &Sender<UiEvent>,
    answers: Vec<(String, Vec<String>)>,
) {
    let Some((request_id, version, run_id)) = state
        .pending_request
        .as_ref()
        .filter(|handle| !handle.is_approval)
        .map(|handle| (handle.request_id.clone(), handle.version, handle.run_id.clone()))
    else {
        return;
    };
    let answer_json = ask_answer_json(&answers);
    match control_plane.answer_ask(&request_id, version, answer_json.clone()) {
        Ok(_) => {
            if let Some(run_id) = run_id {
                // 查找该 ask wait 当前绑定的期望版本；缺失时回退到提交时的版本。
                let waits_val = control_plane.graph_list_ask_waits(&run_id);
                let resume_version = waits_val
                    .as_array()
                    .and_then(|waits| {
                        waits.iter().find(|wait| {
                            wait.get("requestId").and_then(|value| value.as_str())
                                == Some(request_id.as_str())
                        })
                    })
                    .and_then(|wait| wait.get("expectedVersion").and_then(|value| value.as_u64()))
                    .unwrap_or(version);
                match control_plane.graph_resume_ask(
                    &run_id,
                    &request_id,
                    resume_version,
                    answer_json,
                ) {
                    Ok(_) => {
                        state.is_streaming = true;
                        state.status_text = "Thinking...".to_string();
                    }
                    Err(error) => {
                        state.set_status_notice(format!("恢复任务流失败: {error}"));
                        state.messages.push(ChatMessage {
                            role: MessageRole::System,
                            content: format!("恢复任务流失败: {error}"),
                            timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
                        });
                    }
                }
            }
        }
        Err(error) => {
            state.set_status_notice(format!("提交回答失败: {error}"));
            state.messages.push(ChatMessage {
                role: MessageRole::System,
                content: format!("提交回答失败: {error}"),
                timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
            });
        }
    }
    let _ = tx.send(UiEvent::AskUserAnswered {
        request_id,
        answers,
    });
}

/// W3：目标会话能否从 SessionStore 加载（`/agents` Enter 的硬前置）。
///
/// 只有出现在 `list_sessions()` 里的会话才可 hydrate：teammate 引擎运行在**独立建栈**里，
/// 其会话不保证落到宿主的 SessionStore；不先判定就 hydrate 会得到一个空会话，
/// 用户会误以为是「队友没有产出」。
fn session_is_loadable(
    control_plane: &pony_agent_core::agent::control_plane::HostControlPlane,
    session_id: &str,
) -> bool {
    control_plane
        .list_sessions()
        .iter()
        .any(|session| session.conversation_id == session_id)
}

/// W3：`/agents` Enter —— 进入选中 teammate 的子会话。
///
/// 流程（契约）：保存返回目标 → 切 `session_id` → 作用域复位 → hydrate → `mode=Normal`。
/// 无会话 / 会话不可加载时给出可读提示并**留在花名册**（不静默失败、不进入空会话）。
fn enter_teammate_session(
    state: &mut TuiState,
    control_plane: &pony_agent_core::agent::control_plane::HostControlPlane,
    target: &str,
) {
    let Some(session_id) = teammate_session_id(target) else {
        let message = "该 teammate 尚无独立会话（未成功运行）".to_string();
        state.status_text = message.clone();
        state.status_notice = Some((message, std::time::Instant::now()));
        return;
    };
    if !session_is_loadable(control_plane, &session_id) {
        // 说明：队友引擎跑在**独立建栈**里（独立 SessionStore 实例），其会话不会自动出现在
        // 宿主 control plane 的内存会话表里 —— 这里如实提示"宿主暂不可加载"，绝不 hydrate 空会话。
        let message = format!("该 teammate 会话暂不可由宿主会话存储加载（{session_id}），无法进入");
        state.status_text = message.clone();
        state.status_notice = Some((message, std::time::Instant::now()));
        return;
    }

    // 先保存返回目标（reset_session_scope 会清空 return_session_id，故在其后写回）。
    let main_session_id = state.session_id.clone();
    state.mode = InteractionMode::Normal;
    state.agent_roster_state = None;
    state.session_id = session_id.clone();
    state.reset_session_scope();
    state.return_session_id = Some(main_session_id);
    hydrate_state_from_session(state, control_plane, &session_id);
    let notice = format!("已进入队友 {target} 的会话（r 返回主会话）");
    state.status_text = format!("队友 {target} · {session_id}");
    state.status_notice = Some((notice, std::time::Instant::now()));
}

/// W3：`/agents` r —— 返回进入队友会话前保存的主会话。
///
/// 无保存目标 / 主会话不可加载时给可读提示；成功则清空返回目标（避免二次 `r` 回到自身）。
fn return_to_main_session(
    state: &mut TuiState,
    control_plane: &pony_agent_core::agent::control_plane::HostControlPlane,
) {
    let Some(main_session_id) = state.return_session_id.clone() else {
        let message = "没有可返回的主会话（当前不在队友会话）".to_string();
        state.status_text = message.clone();
        state.status_notice = Some((message, std::time::Instant::now()));
        return;
    };
    if !session_is_loadable(control_plane, &main_session_id) {
        let message = format!("主会话 {main_session_id} 已不可加载");
        state.status_text = message.clone();
        state.status_notice = Some((message, std::time::Instant::now()));
        return;
    }

    state.mode = InteractionMode::Normal;
    state.agent_roster_state = None;
    state.session_id = main_session_id.clone();
    state.reset_session_scope();
    hydrate_state_from_session(state, control_plane, &main_session_id);
    state.status_text = format!("已返回主会话 {main_session_id}");
    state.status_notice = Some((
        "已返回主会话".to_string(),
        std::time::Instant::now(),
    ));
}

/// 把目标会话的权威快照装载进 UI 状态（`/resume` 弹窗与启动期 `pony resume <id>` 共用）。
///
/// **不改 `session_id`**：目标 id 由调用方在调用前写入 `state.session_id`。
/// 职责：还原 workspace 归属 → 回放 history（工具结果先于 assistant 行）→ 空 history 时
/// 回退 `message_state` → 贴底滚动态。会话作用域复位仍由调用方执行（`reset_session_scope`）。
fn hydrate_state_from_session(
    state: &mut TuiState,
    control_plane: &pony_agent_core::agent::control_plane::HostControlPlane,
    session_id: &str,
) {
    // 彻底重构历史会话恢复：优先使用权威会话快照 session.history 与 turn_trace_history，确保 1:1 准确恢复
    let runtime_view = control_plane.load_session_runtime_view(pony_agent_core::agent::control_plane::SessionRuntimeViewQuery {
        session_id: Some(session_id.to_string()),
        turn_id: None,
        node_id: None,
        run_id: None,
    });
    let session_snapshot = runtime_view.session;
    // wave-38 D1：恢复会话的 workspace 绑定——
    // 已注册归属保持该归属（header = 会话 env cwd），否则回退启动目录绑定。
    // 只读决议，不触发任何 owner 改写/静默迁移。
    let registered_ids = control_plane
        .list_workspaces()
        .iter()
        .map(|workspace| workspace.id.clone())
        .collect::<Vec<_>>();
    let (resumed_id, resumed_path) = resolve_resumed_workspace_binding(
        session_snapshot.workspace_id.as_deref(),
        session_snapshot
            .env_info
            .as_ref()
            .map(|env| env.cwd.as_str()),
        &registered_ids,
        state.startup_workspace_id.as_deref(),
        &state.startup_workspace_path,
    );
    state.active_workspace_id = resumed_id;
    state.workspace = resumed_path;
    let mut trace_map: std::collections::HashMap<String, &pony_agent_core::agent::session::TurnTraceRecord> = std::collections::HashMap::new();
    for trace in &session_snapshot.turn_trace_history {
        trace_map.insert(trace.turn_id.clone(), trace);
    }

    for hist in &session_snapshot.history {
        match hist.role.as_str() {
            "user" => {
                // wave-tui-session-resilience C2：历史回放与流式共用同一纯函数清洗，
                // 阻断已持久化的 DSML 原文再次进入 provider 上下文。
                let (content, _) = sanitize_model_protocol_text(&hist.content);
                state.messages.push(ChatMessage {
                    role: MessageRole::User,
                    content,
                    timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
                });
            }
            "assistant" => {
                // 如果该轮包含工具调用，先将该轮的工具执行结果反显在 Assistant 之前
                if let Some(turn_id) = &hist.turn_id {
                    if let Some(trace) = trace_map.get(turn_id) {
                        for tool in &trace.tool_activities {
                            if tool.status != "planned" {
                                let detail_preview = history_tool_detail(
                                    &tool.name,
                                    tool.display_name_zh.as_deref(),
                                    tool.canonical_tool_name.as_deref(),
                                    &tool.description,
                                    tool.arguments_text.as_deref(),
                                    tool.result_text.as_deref(),
                                );
                                let formatted = format_tool_display(
                                    &tool.name,
                                    tool.display_name_zh.as_deref(),
                                    detail_preview.as_deref(),
                                    Some(&tool.status),
                                );
                                state.messages.push(ChatMessage {
                                    role: MessageRole::ToolResult,
                                    content: formatted,
                                    timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
                                });
                            }
                        }
                    }
                }

                // wave-tui-session-resilience C2：assistant 历史同样经 S1 清洗。
                let (content, _) = sanitize_model_protocol_text(&hist.content);
                state.messages.push(ChatMessage {
                    role: MessageRole::Assistant,
                    content,
                    timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
                });
            }
            _ => {}
        }
    }

    // wave-tui-session-resilience C6：未完成 turn 的 additive 检查点随历史回放接续展示；
    // 旧会话无检查点字段 ⇒ None ⇒ 逐字回放既有历史，不重复已完成 turn。
    let mut resumed_checkpoint: Option<String> = None;
    for hist in &session_snapshot.history {
        if hist.role == "assistant" && hist.content.contains("[中断轮次检查点]") {
            resumed_checkpoint = Some(hist.content.clone());
            break;
        }
    }
    if let Some(checkpoint) = resumed_checkpoint.as_deref() {
        let completed = state
            .messages
            .iter()
            .map(|message| message.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let continued = resume_from_checkpoint(&completed, Some(checkpoint));
        if continued.len() > completed.len() {
            let tail = continued[completed.len()..].trim_start_matches('\n');
            if !tail.is_empty() {
                state.messages.push(ChatMessage {
                    role: MessageRole::System,
                    content: format!("已接续中断轮次：\n{tail}"),
                    timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
                });
            }
        }
    }

    // 若 history 为空，再尝试 fallback 到 message_state
    if state.messages.is_empty() {
        for entry in runtime_view.message_state.messages {
            match entry.role.as_str() {
                "user" => {
                    // wave-3 L3-B：message_state fallback 与 session.history 回放同口径清洗。
                    let (content, _) =
                        sanitize_model_protocol_text(&entry.content);
                    state.messages.push(ChatMessage {
                        role: MessageRole::User,
                        content,
                        timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
                    });
                }
                "assistant" => {
                    let (content, _) =
                        sanitize_model_protocol_text(&entry.content);
                    state.messages.push(ChatMessage {
                        role: MessageRole::Assistant,
                        content,
                        timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
                    });
                }
                "tool" | "tool_call" | "tool_result" => {
                    let display = format_tool_display(
                        entry.tool_name.as_deref().unwrap_or("tool"),
                        entry.display_name_zh.as_deref(),
                        entry.detail.as_deref(),
                        entry.status.as_deref(),
                    );
                    state.messages.push(ChatMessage {
                        role: MessageRole::ToolResult,
                        content: display,
                        timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
                    });
                }
                _ => {}
            }
        }
    }

    state.scroll_offset = u32::MAX;
    state.auto_scroll = true; // 确保加载完毕后消息贴底显示
    state.mode = InteractionMode::Normal;
    state.status_text = format!("Resumed {}", session_id);
}

/// C16：队列/派发路径的结构化运行日志（复用 `otel::log` 既有 JSON 通道，不另造轮子）。
///
/// 通道语义（照抄既有实现，勿在调用点重写）：单行 JSON → stdout + 注入的落盘文件
/// （512KiB 轮转 / 写入失败降级 / 输出前统一脱敏）；`trace_id` 传 `None` 时由
/// `otel::log` 回落到当前活跃 turn 的真实 trace_id。
/// 结构化字段：`event`（事件名）、`session_id`、`turn_id`，外加调用方给定的 `extras`。
/// C22+C23：把一条队列反馈落到其主通道，并统一写一份 `queue_notice`。
///
/// - `SystemMessage`（C4/C6）：**流式期不 push System 消息** —— 那会打断流式 assistant 气泡的
///   `messages.last()` 不变量（把正文拆行）；改由 `queue_notice` 经常驻指标行渲染。非流式期
///   额外 push 一条 System 消息保留历史留痕。
/// - `StatusText`（C2）：写 `status_text`（stopping 期间不覆盖「正在安全收口」提示）。
fn apply_queue_feedback(state: &mut TuiState, channel: QueueFeedbackChannel, receipt: String) {
    state.queue_notice = Some((receipt.clone(), std::time::Instant::now()));
    match channel {
        QueueFeedbackChannel::SystemMessage => {
            if !state.is_streaming {
                state.messages.push(ChatMessage {
                    role: MessageRole::System,
                    content: receipt.clone(),
                    timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
                });
            }
        }
        QueueFeedbackChannel::StatusText => {
            if state.stopping_turn_id.is_none() {
                state.status_text = receipt.clone();
            }
        }
    }
    state.set_status_notice(receipt);
}

fn tui_queue_log(
    level: &str,
    event: &str,
    state: &TuiState,
    extras: &[(&str, serde_json::Value)],
) {
    let turn_id = state
        .streaming_turn_id
        .clone()
        .or_else(|| state.stopping_turn_id.clone());
    let mut fields: Vec<(&str, serde_json::Value)> = vec![
        ("event", serde_json::json!(event)),
        ("session_id", serde_json::json!(state.session_id.as_str())),
        ("turn_id", serde_json::json!(turn_id)),
        ("streaming", serde_json::json!(state.is_streaming)),
    ];
    fields.extend(extras.iter().cloned());
    let mut sink: Vec<u8> = Vec::new();
    pony_agent_core::agent::otel::log::log_json_with_extras(
        level,
        event,
        None,
        &fields,
        &mut sink,
    );
}

fn tui_event_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    mut state: TuiState,
    control_plane: Arc<pony_agent_core::agent::control_plane::HostControlPlane>,
    rx: Receiver<UiEvent>,
    tx: Sender<UiEvent>,
) -> Result<String, Box<dyn std::error::Error>> {
    let current_turn_id: Arc<std::sync::Mutex<Option<String>>> = Arc::new(std::sync::Mutex::new(None));
    // wave-tui-session-resilience C7：在飞 turn 句柄按轮次线程收敛，退出时先取消再等待。
    let inflight_turns: Arc<std::sync::Mutex<Vec<std::thread::JoinHandle<()>>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));
    // 契约 F-G：goal 自动续轮的取消令牌（Esc 置位；新一轮用户消息提交时重置）。
    let goal_round_cancel = GoalRoundCancelToken::new();
    // C10：轮次线程存活标志（派发闸门第 4 判据）。轮次线程进入时置位、线程体结束（含 panic
    // 展开）时由 `TurnLoopInFlightGuard::drop` 复位；判据是「有没有在飞轮次线程」而非
    // 「有没有 armed goal」，避免 armed-but-idle 的会话把排队消息永久卡住。
    let turn_loop_in_flight: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));

    // 键盘与鼠标事件轮询线程。
    // 契约 F-F：`NO_COLOR` 的进程级环境变量**只在单线程启动期改一次**（run_tui / demo 入口开头）。
    // 多线程环境里 `env::remove_var` 与 libc 读环境变量竞争是 UB（Rust 2024 起直接标 unsafe），
    // 故这里不再触碰进程环境。
    let tx_events = tx.clone();
    std::thread::spawn(move || {
        loop {
            // 契约 F-E：`unwrap_or(false)` 会把 poll 错误伪装成「无事件」⇒ 50ms 热旋且零提示。
            let poll_result = event::poll(Duration::from_millis(50));
            let poll_error = poll_result.as_ref().err().map(|error| error.to_string());
            if classify_input_poll(poll_result) == InputPumpAction::Stop {
                let _ = tx_events.send(UiEvent::InputPumpFailed {
                    error: poll_error.unwrap_or_else(|| "input poll failed".to_string()),
                });
                break;
            }
            let read_result = event::read();
            let outbound = match read_result {
                Ok(Event::Key(key)) => {
                    if key.kind == crossterm::event::KeyEventKind::Press {
                        Some(UiEvent::InputKey(key))
                    } else {
                        None
                    }
                }
                Ok(Event::Resize(_, _)) => Some(UiEvent::Resized),
                // 契约 F-C：粘贴文本原样交给主循环（插入缓冲，绝不提交）。
                Ok(Event::Paste(text)) => Some(UiEvent::InputPaste(text)),
                Ok(Event::Mouse(mouse)) => {
                    use crossterm::event::MouseEventKind;
                    match mouse.kind {
                        MouseEventKind::ScrollUp => Some(UiEvent::ScrollWheel(true)),
                        MouseEventKind::ScrollDown => Some(UiEvent::ScrollWheel(false)),
                        _ => Some(UiEvent::InputMouse(mouse)),
                    }
                }
                // FocusGained/FocusLost 等事件：无 UI 语义，忽略。
                Ok(_) => None,
                Err(error) => {
                    // 读失败与 poll 失败同类：输入源不可用 ⇒ 提示后退出，不再热旋。
                    let _ = tx_events.send(UiEvent::InputPumpFailed {
                        error: error.to_string(),
                    });
                    break;
                }
            };
            // 契约 F-E：通道对端已关闭（事件循环退出）⇒ 立即结束线程，不做无意义热旋。
            if let Some(event) = outbound {
                if tx_events.send(event).is_err() {
                    break;
                }
            }
        }
    });

    // 辅助函数：根据当前会话消息正文提取行纯文本切片
    let extract_current_rendered_lines = |st: &TuiState| -> Vec<String> {
        let mut lines = Vec::new();
        for msg in &st.messages {
            for line in msg.content.lines() {
                lines.push(line.to_string());
            }
        }
        lines
    };

    let mut last_tick_instant = std::time::Instant::now();
    let mut last_poll_instant = std::time::Instant::now();
    let mut last_goal_phase: Option<bool> = None;
    let mut cap_poll_ms: Option<u64> = None;
    // C24 单飞闩（时间窗）：队首派发后置 `now + QUEUE_DISPATCH_LATCH_MS`，窗口内不再派发。
    // 不再「收到任何 InputKey 就清闩」—— 那会在「派发后、合成 Enter 被消费前」开单飞缺口
    // （用户按 Esc/Ctrl+U 会同时清掉闩与输入框 ⇒ 闸门立刻再派发第二条 ⇒ 并发 turn）。
    // 清除点只有三个：提交成功（is_streaming 置真）、合成提交放弃回灌、显式撤销（Esc/Ctrl+U）。
    let mut queue_dispatch_deadline: Option<std::time::Instant> = None;
    // C26 权威载荷：派发时刻的正文快照。窗口内的撤销/回队**一律**用它（而不是此刻的
    // `input_buffer`）—— 用户可能在窗口内改过输入框，读字节会把已派发正文静默丢掉。
    let mut queue_dispatch_payload: Option<QueuedUserMessage> = None;
    // 信号处理器已在 run_tui 进入 raw mode 之前安装（启动期 SIGHUP 也要能还原终端）。

    // 辅助闭包：检查并同步当前 session 的待决宿主控制请求（Ask / Approval）
    let check_pending_asks = |st: &mut TuiState, cp: &HostControlPlane| -> bool {
        // 如果当前已处于审批或问答模式，暂不覆盖当前操作
        if st.mode == InteractionMode::PermissionApproval || st.mode == InteractionMode::AskUserQuestion {
            return false;
        }
        let pending = cp.list_pending_asks(Some(st.session_id.as_str()));
        let items = match pending.as_array() {
            Some(arr) if !arr.is_empty() => arr,
            _ => {
                st.pending_request = None;
                return false;
            }
        };

        if let Some(first) = items.first() {
            if let Some(handle) = pending_ui_request_from_value(first) {
                st.pending_request = Some(handle.clone());
                if handle.is_approval {
                    let req = permission_request_from_pending(first, &st.session_id);
                    st.start_permission_approval(req);
                } else {
                    let q = ask_question_from_pending(first);
                    st.start_ask_user(handle.request_id, vec![q]);
                }
                return true;
            }
        }
        false
    };

    // wave-60 §C8：流式中双击 Esc 的判定状态机（窗口 `stop_sequence_ms`=500，与桌面端同判据）。
    // generation 承载「同一 turn_id 的一次出现」：turn_id 变化 ⇒ 身份变化 ⇒ 上膛失效。
    let mut stop_sequence = StopSequenceState::new(STOP_SEQUENCE_MS);
    let mut stop_sequence_generation: u64 = 0;
    let mut stop_sequence_turn_id: Option<String> = None;

    // 事件驱动重绘：无输入、无流式时不做无意义的 60fps 全屏重绘（省 CPU / 省电）。
    let mut dirty = true;
    loop {
        // T2c：SIGINT/SIGTERM 置位后优雅退出主循环，走统一的终端恢复路径。
        if SHUTDOWN_REQUESTED.load(Ordering::SeqCst) {
            break;
        }
        // 流式态 160ms 驱动动效；问答等待期用 450ms 的低频呼吸提醒（不闪、不抢注意力）。
        let awaiting_answer = matches!(
            state.mode,
            InteractionMode::AskUserQuestion | InteractionMode::AddingProvider
        );
        let tick_ms = if state.is_streaming { 160 } else { 450 };
        if (state.is_streaming || awaiting_answer)
            && last_tick_instant.elapsed() >= Duration::from_millis(tick_ms)
        {
            state.spinner_tick = state.spinner_tick.wrapping_add(1);
            last_tick_instant = std::time::Instant::now();
            dirty = true;
        }

        // 空闲或间隔约 500ms 时，轮询一次待决的宿主请求
        if last_poll_instant.elapsed() >= Duration::from_millis(500) {
            if check_pending_asks(&mut state, &control_plane) {
                dirty = true;
            }
            last_poll_instant = std::time::Instant::now();
        }

        // 存在平滑阻尼过渡（Composer 柔和收拢或平滑滚动追赶）时持续刷新下一帧
        if state.has_pending_animation() {
            dirty = true;
        }

        // 信号处理器已就地还原终端：此刻绝不能再画帧，否则会把画面涂到用户 shell 上。
        if SHUTDOWN_REQUESTED.load(Ordering::SeqCst) {
            break;
        }
        if dirty {
            terminal.draw(|f| ui(f, &mut state))?;
            dirty = false;
        }

        // 检查鼠标选区防抖与复制
        if let Some(until) = state.mouse_selection.debounce_until {
            if std::time::Instant::now() >= until {
                state.mouse_selection.debounce_until = None;
                if let Some(normalized) = state.mouse_selection.normalized_range() {
                    if state.mouse_selection.copied_range != Some(normalized) {
                        // 契约（修订11 步骤3）：全量纯文本只在复制/选区路径按需重建。
                        state.ensure_full_text();
                        let lines = if !state.rendered_lines_text.is_empty() {
                            state.rendered_lines_text.clone()
                        } else {
                            extract_current_rendered_lines(&state)
                        };
                        let selected_text = extract_selection_across(
                            &lines,
                            &state.last_input_lines,
                            normalized.0,
                            normalized.1,
                        );
                        if !selected_text.is_empty() && selected_text.len() <= 1_000_000 {
                            let osc_seq = encode_osc52_copy(&selected_text);
                            let mut stdout = std::io::stdout();
                            let _ = std::io::Write::write_all(&mut stdout, osc_seq.as_bytes());
                            let _ = std::io::Write::flush(&mut stdout);
                            let _ = terminal.backend_mut();
                            state.mouse_selection.copied_range = Some(normalized);
                            state.set_status_notice("已复制到剪贴板");
                            dirty = true;
                        }
                    }
                }
            }
        }

        // 波次 44：Goal/指标轮播到切换点时立即重绘，并且把等待时长压到不超过剩余相位，
        // 既不靠 16fps 空转，也不会让横幅在相位边界上“卡住不动”。
        if let Some((visible, remaining)) = goal_banner_phase(&state) {
            if last_goal_phase != Some(visible) {
                last_goal_phase = Some(visible);
                dirty = true;
            }
            // 留 1ms 余量，避免取整后再次落在同一相位。
            cap_poll_ms = Some(remaining.saturating_add(1));
        } else {
            last_goal_phase = None;
        }

        let mut poll_ms = if state.is_streaming || state.has_pending_animation() { 16 } else { 120 };
        if let Some(cap) = cap_poll_ms {
            poll_ms = poll_ms.min(cap.max(30));
            cap_poll_ms = None;
        }
        match rx.recv_timeout(Duration::from_millis(poll_ms)) {
            Ok(event) => {
                dirty = true;
                match event {
                UiEvent::Resized => {
                    // 契约（修订11 步骤2）：尺寸变化必须失效布局缓存。
                    state.invalidate_render_cache();
                    // 尺寸变化后先清空物理终端，避免旧尺寸余晖与新画面叠加成「乱排」。
                    let _ = terminal.clear();
                    dirty = true;
                }
                UiEvent::ScrollWheel(is_up) => {
                    if is_up {
                        state.scroll_up(3);
                    } else {
                        state.scroll_down(3, state.max_scroll_offset);
                    }
                }
                UiEvent::InputMouse(mouse) => {
                    use crossterm::event::{MouseButton, MouseEventKind};
                    if state.mode == InteractionMode::AskUserQuestion {
                        // Ask 覆盖正文下部；禁止选中被面板遮住的隐藏历史行。
                        if state.mouse_selection.active || state.mouse_selection.anchor.is_some() {
                            state.mouse_selection.clear();
                            dirty = true;
                        }
                    } else {
                        let x = mouse.column;
                        let y = mouse.row;
                        match mouse.kind {
                            MouseEventKind::Down(MouseButton::Left) => {
                                if let Some(coord) = selection_point_at(&state, x, y) {
                                    state.mouse_selection.active = true;
                                    state.mouse_selection.anchor = Some(coord);
                                    state.mouse_selection.head = Some(coord);
                                    state.mouse_selection.copied_range = None;
                                    state.mouse_selection.debounce_until = None;
                                    dirty = true;
                                } else if state.mouse_selection.active
                                    || state.mouse_selection.anchor.is_some()
                                {
                                    state.mouse_selection.clear();
                                    dirty = true;
                                }
                            }
                            MouseEventKind::Drag(MouseButton::Left) => {
                                if state.mouse_selection.active {
                                    // 拖出正文矩形时吸附到边缘行列：继续拖仍能连续扩选，而不是断在半路。
                                    if let Some(coord) = selection_drag_point(&state, x, y) {
                                        state.mouse_selection.head = Some(coord);
                                        if state.mouse_selection.anchor != state.mouse_selection.head {
                                            state.mouse_selection.debounce_until = Some(
                                                std::time::Instant::now() + Duration::from_millis(200),
                                            );
                                        }
                                        dirty = true;
                                    }
                                }
                            }
                            MouseEventKind::Up(MouseButton::Left) => {
                                if state.mouse_selection.active {
                                    state.mouse_selection.active = false;
                                    if state.mouse_selection.anchor == state.mouse_selection.head {
                                        state.mouse_selection.clear();
                                    } else {
                                        state.mouse_selection.debounce_until =
                                            Some(std::time::Instant::now() + Duration::from_millis(200));
                                    }
                                    dirty = true;
                                }
                            }
                            _ => {}
                        }
                    }
                }
                UiEvent::InputKey(key) => {
                    // C24：**不再**在这里清单飞闩 —— 闩按时间窗自然过期，
                    // 由「提交成功 / 放弃回灌 / 显式撤销」三个语义点显式清除。
                    // wave-36b：F2 顶层处理（任意模式可用）：裸键切换鼠标捕获，
                    // 默认关（终端原生拖选/复制可用）；与退出恢复路径统一 DisableMouseCapture 不冲突。
                    if key.code == KeyCode::F(2) {
                        state.mouse_capture = !state.mouse_capture;
                        if state.mouse_capture {
                            let _ = execute!(terminal.backend_mut(), EnableMouseCapture);
                            state.status_text = "鼠标捕获已开启".to_string();
                        } else {
                            let _ = execute!(terminal.backend_mut(), DisableMouseCapture);
                            state.status_text = "鼠标捕获已关闭 · 可选择文本".to_string();
                        }
                        dirty = true;
                        continue;
                    }

                    // F3 快捷键：顶层免冲突快速切换思考强度档位
                    if key.code == KeyCode::F(3) {
                        state.cycle_reasoning_effort();
                        state.messages.push(ChatMessage {
                            role: MessageRole::System,
                            content: format!("已设置思考强度为: {}", state.reasoning_effort_display()),
                            timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
                        });
                        dirty = true;
                        continue;
                    }

                    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                        // 契约 P-4：三态语义（纯函数裁定）——流式中 = 中断本轮（等价 Esc，不退出）；
                        // 有草稿 = 先清草稿（防误触丢输入）；否则才退出。
                        match ctrl_c_action(state.is_streaming, !state.input_buffer.is_empty()) {
                            CtrlCAction::InterruptTurn => {
                                // wave-60 §C9：Ctrl+C 单按保留「立即中断」语义（不要求双击），
                                // 但同样走「请求 → 事件驱动终态」：此处不置 `is_streaming=false`、
                                // 不打印任何「已中断」类终态文案（防假终态）。
                                // 已受理的停止请求在等终态 ⇒ 不重复派发。
                                // wave-63 §T3：此时按键**不得静默吞** —— `note_stopping_key` 会置
                                // 「等待安全收口」提示（流式行可见）并返回 false，拦下重复派发。
                                if state.note_stopping_key() {
                                    goal_round_cancel.cancel();
                                    // NFR `lock_scope_bounded`：先取再放 —— 绝不在持锁期间调用
                                    // `stop_turn`（其会同步回调 sink.emit，存在锁重入/死锁风险）。
                                    let stopping_turn = { current_turn_id.lock().unwrap().take() };
                                    if let Some(turn_id) = stopping_turn {
                                        control_plane.stop_turn(StopTurnCommand {
                                            turn_id: turn_id.clone(),
                                            stop_reason: Some(StopReason::User),
                                        });
                                        state.apply_stop_requested(&turn_id);
                                    }
                                }
                                dirty = true;
                                continue;
                            }
                            CtrlCAction::ClearDraft => {
                                state.clear_input();
                                continue;
                            }
                            CtrlCAction::Exit => break,
                        }
                    }

                    // 1. 会话选择弹窗模式下的按键处理
                    if state.mode == InteractionMode::SelectingSession {
                        match key.code {
                            KeyCode::Esc => {
                                state.cancel_session_selector();
                            }
                            KeyCode::Up => {
                                state.prev_session();
                            }
                            KeyCode::Down => {
                                state.next_session();
                            }
                            KeyCode::Enter => {
                                if !state.session_list.is_empty() {
                                    let chosen = state.session_list[state.selected_session_index].clone();
                                    state.session_id = chosen.id.clone();
                                    // 契约 P-2：/resume 先复位会话作用域，再装载目标会话快照
                                    // （否则上一个会话的用量/计划/目标/待决请求会串到新会话）。
                                    state.reset_session_scope();

                                    state.reset_session_scope();
                                    hydrate_state_from_session(&mut state, &control_plane, &chosen.id);
                                    state.mode = InteractionMode::Normal;
                                } else {
                                    state.cancel_session_selector();
                                }
                            }
                            _ => {}
                        }
                        continue;
                    }

                    // 1b. Provider 选择弹窗模式下的按键处理
                    if state.mode == InteractionMode::SelectingProvider {
                        match key.code {
                            KeyCode::Esc => {
                                state.cancel_provider_selector();
                            }
                            KeyCode::Char('a') | KeyCode::Char('A') | KeyCode::Char('+') => {
                                state.start_add_provider();
                            }
                            KeyCode::Char('d') | KeyCode::Char('D') | KeyCode::Delete => {
                                match state.delete_current_provider() {
                                    Ok(msg) => {
                                        state.set_status_notice(msg.clone());
                                        state.messages.push(ChatMessage {
                                            role: MessageRole::System,
                                            content: msg,
                                            timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
                                        });
                                    }
                                    Err(err) => {
                                        state.set_status_notice(err);
                                    }
                                }
                            }
                            KeyCode::Up => {
                                state.prev_provider();
                            }
                            KeyCode::Down => {
                                state.next_provider();
                            }
                            KeyCode::Enter => {
                                if let Some(selected) = state.select_current_provider() {
                                    state.messages.push(ChatMessage {
                                        role: MessageRole::System,
                                        content: format!("已切换提供商为: {}", selected.name),
                                        timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
                                    });
                                    // 二级联动：自动打开该 provider 的 model 选择菜单
                                    let registry = state.registry_store();
                                    let view = registry.load_view();
                                    let current_prov = view.providers.iter().find(|p| p.id == selected.id).cloned();
                                    if let Some(p) = current_prov {
                                        let items: Vec<ModelItem> = p
                                            .models
                                            .iter()
                                            .map(|m| ModelItem { id: m.id.clone(), name: m.name.clone() })
                                            .collect();
                                        state.open_model_selector(items);
                                        state.status_text = "正在拉取模型列表…".to_string();
                                        spawn_model_discovery(&state, &tx, &p.id);
                                    } else {
                                        state.open_model_selector(Vec::new());
                                    }
                                } else {
                                    state.mode = InteractionMode::Normal;
                                }
                            }
                            _ => {}
                        }
                        continue;
                    }

                    // 1c. Model 选择弹窗模式下的按键处理
                    if state.mode == InteractionMode::SelectingModel {
                        // Ctrl+D（设默认）/ Ctrl+F（切换优选）是「写盘」动作，走独立可测函数。
                        if apply_model_panel_shortcut(
                            &mut state,
                            &key,
                            &ModelPreferencesStore::new(),
                        ) {
                            continue;
                        }
                        match key.code {
                            KeyCode::Esc => {
                                state.cancel_model_selector();
                            }
                            KeyCode::Up => {
                                state.prev_model();
                            }
                            KeyCode::Down => {
                                state.next_model();
                            }
                            KeyCode::Enter => {
                                state.select_current_model();
                                state.messages.push(ChatMessage {
                                    role: MessageRole::System,
                                    content: format!("已切换模型为: {}", state.model),
                                    timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
                                });
                            }
                            // wave-35b：面板内 `r`/`R` 对当前 provider 无条件重新远程拉取
                            //（线程化，面板保持；不受 is_streaming 影响——分支先于流式守卫）。
                            // 完成后由 ModelsDiscovered 处理器刷新 model_list 与选中项。
                            KeyCode::Char('r') | KeyCode::Char('R') => {
                                let prov_id = resolve_active_provider_id(&state)
                                    .unwrap_or_else(|| state.current_provider_id.clone());
                                state.status_text = "正在拉取模型列表…".to_string();
                                spawn_model_discovery(&state, &tx, &prov_id);
                            }
                            _ => {}
                        }
                        continue;
                    }

// 1c2. ReasoningEffort 选择弹窗模式下的按键处理
                    if state.mode == InteractionMode::SelectingReasoningEffort {
                        match key.code {
                            KeyCode::Esc => {
                                state.cancel_reasoning_effort_selector();
                            }
                            KeyCode::Up => {
                                state.prev_reasoning_effort();
                            }
                            KeyCode::Down => {
                                state.next_reasoning_effort();
                            }
                            KeyCode::Enter => {
                                state.select_current_reasoning_effort();
                                state.messages.push(ChatMessage {
                                    role: MessageRole::System,
                                    content: format!("已设置思考强度为: {}", state.reasoning_effort_display()),
                                    timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
                                });
                            }
                            _ => {}
                        }
                        continue;
                    }

                    // 1d. AddingProvider 模式下的按键处理
                    if state.mode == InteractionMode::AddingProvider {
                        match key.code {
                            KeyCode::Esc => {
                                state.cancel_add_provider();
                            }
                            KeyCode::Backspace => {
                                state.backspace();
                            }
                            KeyCode::Char(c) => {
                                state.insert_char(c);
                            }
                            KeyCode::Enter => {
                                let input_val = state.input_buffer.trim().to_string();
                                state.add_provider_input(&input_val);
                                state.clear_input();

                                match state.advance_add_provider_step() {
                                    Ok(_) => {
                                        state.status_text.clear();
                                        if state.add_provider_step == AddingProviderStep::Commit {
                                            if let Some(payload) = state.finalize_add_provider() {
                                                // 持久化落地保存至 ProviderRegistryStore
                                                let registry = ProviderRegistryStore::new();
                                                let mut view = registry.load_view();
                                                let new_id = payload.id.clone();
                                                let new_name = payload.name.clone();

                                                let config_view = pony_agent_core::agent::config::ProviderConfigView {
                                                    id: new_id.clone(),
                                                    name: new_name.clone(),
                                                    protocol: pony_agent_core::agent::provider::ProviderProtocol::OpenAiCompletions,
                                                    base_url: payload.base_url.clone(),
                                                    auth_type: pony_agent_core::agent::provider::ProviderAuthType::Auto,
                                                    api_key_env_var: format!("{}_API_KEY", new_id.to_uppercase().replace('-', "_")),
                                                    api_key_value: payload.api_key.clone().unwrap_or_default(),
                                                    api_key_present: payload.api_key.is_some(),
                                                    supported_protocols: vec![pony_agent_core::agent::provider::ProviderProtocol::OpenAiCompletions],
                                                    endpoints: vec![],
                                                    models: vec![],
                                                    selected_model_id: None,
                                                };
                                                view.providers.push(config_view);
                                                view.selected_provider_id = Some(new_id.clone());

                                                match registry.save_view(view) {
                                                    Ok(_) => {
                                                        state.current_provider_id = new_id.clone();
                                                        state.messages.push(ChatMessage {
                                                            role: MessageRole::System,
                                                            content: format!("成功添加并切换至新提供商: {} ({})", new_name, new_id),
                                                            timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
                                                        });
                                                        state.status_text = format!("Added provider {}", new_name);
                                                    }
                                                    Err(e) => {
                                                        state.messages.push(ChatMessage {
                                                            role: MessageRole::System,
                                                            content: format!("保存提供商失败: {}", e),
                                                            timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
                                                        });
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    Err(err) => {
                                        state.status_text = err.to_string();
                                    }
                                }
                            }
                            _ => {}
                        }
                        continue;
                    }

                    // 1e. PermissionApproval 权限审批弹窗按键处理
                    if state.mode == InteractionMode::PermissionApproval {
                        match key.code {
                            KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                                if let Some(ref handle) = state.pending_request {
                                    if handle.is_approval {
                                        if let Err(e) = control_plane.cancel_ask(&handle.request_id, handle.version) {
                                            state.set_status_notice(format!("取消审批失败: {e}"));
                                            state.messages.push(ChatMessage {
                                                role: MessageRole::System,
                                                content: format!("取消审批失败: {e}"),
                                                timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
                                            });
                                        }
                                    }
                                }
                                if let Some(ref perm) = state.permission_state {
                                    let req_id = perm.request.request_id.clone();
                                    let _ = tx.send(UiEvent::PermissionDecided {
                                        request_id: req_id,
                                        choice: PermissionChoice::Deny,
                                    });
                                }
                                state.pending_request = None;
                                state.cancel_permission_approval();
                            }
                            KeyCode::Tab | KeyCode::Down => {
                                if let Some(ref mut perm) = state.permission_state {
                                    perm.next_choice();
                                }
                            }
                            KeyCode::BackTab | KeyCode::Up => {
                                if let Some(ref mut perm) = state.permission_state {
                                    perm.prev_choice();
                                }
                            }
                            KeyCode::Char('y') | KeyCode::Char('Y') => {
                                if let Some(ref handle) = state.pending_request {
                                    if handle.is_approval {
                                        if let Err(e) = control_plane.approve_control_request(&handle.request_id, handle.version) {
                                            state.set_status_notice(format!("审批同意失败: {e}"));
                                            state.messages.push(ChatMessage {
                                                role: MessageRole::System,
                                                content: format!("审批同意失败: {e}"),
                                                timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
                                            });
                                        }
                                    }
                                }
                                if let Some(ref perm) = state.permission_state {
                                    let req_id = perm.request.request_id.clone();
                                    let _ = tx.send(UiEvent::PermissionDecided {
                                        request_id: req_id,
                                        choice: PermissionChoice::AllowOnce,
                                    });
                                }
                                state.pending_request = None;
                                state.cancel_permission_approval();
                            }
                            KeyCode::Enter => {
                                let choice = state.permission_state.as_ref().map(|p| p.selected_choice()).unwrap_or(PermissionChoice::Deny);
                                if let Some(ref handle) = state.pending_request {
                                    if handle.is_approval {
                                        match choice {
                                            PermissionChoice::AllowOnce | PermissionChoice::AllowSession => {
                                                if choice == PermissionChoice::AllowSession {
                                                    if let Some(ref perm) = state.permission_state {
                                                        if let Some(ref target) = perm.request.command_or_path {
                                                            let _ = control_plane.authorize_path(target, "read");
                                                        }
                                                    }
                                                }
                                                if let Err(e) = control_plane.approve_control_request(&handle.request_id, handle.version) {
                                                    state.set_status_notice(format!("审批执行失败: {e}"));
                                                    state.messages.push(ChatMessage {
                                                        role: MessageRole::System,
                                                        content: format!("审批执行失败: {e}"),
                                                        timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
                                                    });
                                                }
                                            }
                                            PermissionChoice::Deny => {
                                                if let Err(e) = control_plane.cancel_ask(&handle.request_id, handle.version) {
                                                    state.set_status_notice(format!("审批拒绝失败: {e}"));
                                                    state.messages.push(ChatMessage {
                                                        role: MessageRole::System,
                                                        content: format!("审批拒绝失败: {e}"),
                                                        timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
                                                    });
                                                }
                                            }
                                        }
                                    }
                                }
                                if let Some(ref perm) = state.permission_state {
                                    let req_id = perm.request.request_id.clone();
                                    let _ = tx.send(UiEvent::PermissionDecided {
                                        request_id: req_id,
                                        choice,
                                    });
                                }
                                state.pending_request = None;
                                state.cancel_permission_approval();
                            }
                            _ => {}
                        }
                        continue;
                    }

                    // 1f. AskUserQuestion 问答弹窗按键处理
                    if state.mode == InteractionMode::AskUserQuestion {
                        match key.code {
                            KeyCode::Esc => {
                                if let Some(ref handle) = state.pending_request {
                                    if !handle.is_approval {
                                        if let Err(e) = control_plane.cancel_ask(&handle.request_id, handle.version) {
                                            state.set_status_notice(format!("取消提问失败: {e}"));
                                            state.messages.push(ChatMessage {
                                                role: MessageRole::System,
                                                content: format!("取消提问失败: {e}"),
                                                timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
                                            });
                                        }
                                    }
                                }
                                state.pending_request = None;
                                state.cancel_ask_user();
                            }
                            KeyCode::Down => {
                                if let Some(ref mut ask) = state.ask_user_state {
                                    ask.next_option();
                                }
                            }
                            KeyCode::Up => {
                                if let Some(ref mut ask) = state.ask_user_state {
                                    ask.prev_option();
                                }
                            }
                            KeyCode::Char(' ') => {
                                if let Some(ref mut ask) = state.ask_user_state {
                                    if !ask.is_custom_input_active {
                                        ask.toggle_current_option();
                                    } else {
                                        ask.custom_input.push(' ');
                                    }
                                }
                            }
                            KeyCode::Backspace => {
                                if let Some(ref mut ask) = state.ask_user_state {
                                    if ask.is_custom_input_active {
                                        ask.custom_input.pop();
                                    }
                                }
                            }
                            KeyCode::Char(c) => {
                                if let Some(ref mut ask) = state.ask_user_state {
                                    if ask.is_custom_input_active {
                                        ask.custom_input.push(c);
                                    }
                                }
                            }
                            KeyCode::Enter => {
                                // 契约 F-M：先按纯函数裁定「这次 Enter 是不是一个有效提交」。
                                // 空白回答一律拒绝并保持面板打开（不得靠伪造选项蒙混）。
                                let mut finish_with: Option<Vec<(String, Vec<String>)>> = None;
                                if let Some(ask) = state.ask_user_state.as_ref() {
                                    let has_options = ask
                                        .current_question()
                                        .map(|question| !question.options.is_empty())
                                        .unwrap_or(false);
                                    let typed = if ask.is_custom_input_active {
                                        ask.custom_input.clone()
                                    } else {
                                        String::new()
                                    };
                                    match ask_submit_verdict(has_options, ask_selected_index(ask), &typed) {
                                        AskSubmitVerdict::Reject(reason) => {
                                            state.set_status_notice(reason);
                                            dirty = true;
                                            continue;
                                        }
                                        AskSubmitVerdict::Submit => {
                                            let values = ask_answer_values(ask);
                                            if let Some(ask_mut) = state.ask_user_state.as_mut() {
                                                match ask_mut.record_and_advance(values) {
                                                    // 契约 F-M②：还有后续问题 ⇒ 只推进，**不提交**。
                                                    NextQuestion::Advance(next) => {
                                                        let total = ask_mut.questions.len();
                                                        state.set_status_notice(format!(
                                                            "已记录第 {} 问，请回答第 {}/{} 问",
                                                            next,
                                                            next + 1,
                                                            total
                                                        ));
                                                    }
                                                    NextQuestion::Finish => {
                                                        finish_with = Some(
                                                            ask_mut
                                                                .answers
                                                                .iter()
                                                                .map(|answer| {
                                                                    (answer.question_id.clone(), answer.values.clone())
                                                                })
                                                                .collect(),
                                                        );
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                if let Some(answers) = finish_with {
                                    // 全部问题答完 ⇒ 一次性提交（answer_ask + graph_resume_ask）。
                                    submit_ask_answers(&mut state, &control_plane, &tx, answers);
                                    state.pending_request = None;
                                    state.cancel_ask_user();
                                }
                                dirty = true;
                                continue;
                            }
                            _ => {}
                        }
                        continue;
                    }

                    // 1g. PlanMode 计划模式按键处理
                    // C19（BLOCKING 修复）：计划面板只吞 Esc/Up/Down（`plan_mode_consumes_key`），
                    // **其余按键必须继续落到通用处理链**（流式裁决 / 普通输入 / Enter 提交）。
                    // 旧实现 `_ => {}` 之后无脑 `continue` ⇒ 计划面板打开时输入框整段失效。
                    if state.mode == InteractionMode::PlanMode && plan_mode_consumes_key(&key) {
                        match key.code {
                            KeyCode::Esc => {
                                state.toggle_plan_mode();
                            }
                            KeyCode::Down => {
                                if let Some(ref mut plan) = state.plan_state {
                                    if !plan.steps.is_empty() {
                                        plan.selected_step_index = (plan.selected_step_index + 1) % plan.steps.len();
                                    }
                                }
                            }
                            KeyCode::Up => {
                                if let Some(ref mut plan) = state.plan_state {
                                    if !plan.steps.is_empty() {
                                        if plan.selected_step_index == 0 {
                                            plan.selected_step_index = plan.steps.len() - 1;
                                        } else {
                                            plan.selected_step_index -= 1;
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                        continue;
                    }

                    // 1h. W3：AgentRoster 花名册按键处理
                    //   ↑↓ 选择 · Enter 进入该 teammate 子会话 · r 返回保存的主会话 · Esc 关闭。
                    if state.mode == InteractionMode::AgentRoster {
                        match key.code {
                            KeyCode::Esc => {
                                state.close_agent_roster();
                            }
                            KeyCode::Up => {
                                if let Some(ref mut roster) = state.agent_roster_state {
                                    roster.prev();
                                }
                            }
                            KeyCode::Down => {
                                if let Some(ref mut roster) = state.agent_roster_state {
                                    roster.next();
                                }
                            }
                            KeyCode::Enter => {
                                if let Some(target) = state.selected_agent_target() {
                                    if target == "lead" {
                                        state.status_notice = Some((
                                            "Lead 即宿主主会话，无需进入".to_string(),
                                            std::time::Instant::now(),
                                        ));
                                    } else {
                                        enter_teammate_session(
                                            &mut state,
                                            &control_plane,
                                            &target,
                                        );
                                    }
                                }
                            }
                            KeyCode::Char('r')
                                if !key.modifiers.contains(KeyModifiers::CONTROL) =>
                            {
                                return_to_main_session(&mut state, &control_plane);
                            }
                            _ => {}
                        }
                        dirty = true;
                        continue;
                    }

                    // 1i. TaskBoard 任务看板按键处理
                    if state.mode == InteractionMode::TaskBoard {
                        match key.code {
                            KeyCode::Esc => {
                                state.toggle_task_board();
                            }
                            // 面板内再按 Ctrl+T 直接收起（与命令式 toggle 语义一致）。
                            KeyCode::Char('t')
                                if key.modifiers.contains(KeyModifiers::CONTROL) =>
                            {
                                state.toggle_task_board();
                            }
                            KeyCode::Tab => {
                                if let Some(ref mut board) = state.task_board_state {
                                    board.toggle_collapse();
                                }
                            }
                            KeyCode::Down => {
                                if let Some(ref mut board) = state.task_board_state {
                                    board.next_task();
                                }
                            }
                            KeyCode::Up => {
                                if let Some(ref mut board) = state.task_board_state {
                                    board.prev_task();
                                }
                            }
                            _ => {}
                        }
                        continue;
                    }

                    // 2. 流式态 Esc = 停止本轮（双击）；静态下 Esc 仅用于清空输入缓冲区，不退出程序
                    if key.code == KeyCode::Esc {
                        if state.is_streaming {
                            // 已受理的停止请求在等终态 ⇒ 不重复派发（幂等），也不伪造终态。
                            // wave-63 §T3：此时按键**不得静默吞** —— `note_stopping_key` 置
                            // 「等待安全收口」提示（流式行可见）并返回 false。
                            if state.note_stopping_key() {
                                // wave-60 §C8/C9：**双击** Esc（窗口 `stop_sequence_ms`=500，同身份）才发
                                // **恰好一次**停止请求；第 1 次仅「上膛」。发出请求后只进「正在停止…」，
                                // 终态一律由 `turn:cancelled` 事件驱动 —— 此处**不得**本地伪造终态。
                                let current_turn = current_turn_id.lock().unwrap().clone();
                                let identity = next_stop_identity(
                                    &mut stop_sequence_generation,
                                    &mut stop_sequence_turn_id,
                                    state.session_id.as_str(),
                                    current_turn.as_deref(),
                                );
                                if stop_sequence.press(identity, now_epoch_ms()) {
                                    // 契约 F-G③：先置取消令牌 —— goal 续轮循环在**下一轮开始前**看到它即中止；
                                    // 已经在跑的那一轮仍由 stop_turn 打断。
                                    goal_round_cancel.cancel();
                                    // NFR `lock_scope_bounded`：先取再放 —— 绝不在持锁期间调用
                                    // `stop_turn`（其会同步回调 sink.emit，存在锁重入/死锁风险）。
                                    let stopping_turn = { current_turn_id.lock().unwrap().take() };
                                    if let Some(turn_id) = stopping_turn {
                                        control_plane.stop_turn(StopTurnCommand {
                                            turn_id: turn_id.clone(),
                                            stop_reason: Some(StopReason::User),
                                        });
                                        state.apply_stop_requested(&turn_id);
                                    }
                                }
                            }
                        } else {
                            // C24：闩窗口内 Esc = 显式撤销本次派发 ⇒ 已写回的正文**回队队首**
                            // 再清空输入框，否则「remove(0) 已发生 + 输入框被清」会静默丢正文。
                            // C26⑥：回队**权威 payload**（窗口内输入框可能已被改，读字节会丢正文）。
                            if queue_dispatch_latch_active(queue_dispatch_deadline) {
                                let pending = queue_dispatch_payload
                                    .take()
                                    .unwrap_or_else(|| state.input_buffer.trim().to_string());
                                requeue_front(&mut state, pending);
                                queue_dispatch_deadline = None;
                            }
                            // 静态时不退出，只清空输入框
                            state.clear_input();
                        }
                        continue;
                    }

                    // ── C1–C4 裁决面：流式期按键不再被吞 ─────────────────────────────
                    // 优先级不变：面板/模态分支、双击 Esc 停止、Ctrl+C 三态、F2/F3 都在**本裁决之前**
                    // 已经生效（上面的分支各自 `continue`），故面板内按键不会被吞。
                    // 旧实现是 `if state.is_streaming && should_skip_while_streaming(&key) { continue; }`
                    // ⇒ 流式中除 Ctrl+U/J/W/A/E 外全部按键被静默丢弃。
                    if state.is_streaming {
                        let composer_text = state.input_buffer.trim().to_string();
                        let mut disposition =
                            streaming_key_disposition(&key, !composer_text.is_empty());
                        // C3/C4：命令串走命令分发，绝不排成聊天消息（C6 同口径）。
                        //  - 会话变更类（/clear /compact /resume）⇒ Reject：清空 + 「本轮结束后可用」
                        //  - 面板/只读类（/agents /tasks /plan /models /thinking /provider /help /update）
                        //    ⇒ Forward：落到原 Enter 分发分支照常执行
                        //  - /quit /exit 等其余已登记命令 ⇒ Forward：走原退出/分发分支
                        if disposition == StreamingKeyDisposition::QueueSubmit {
                            // C20 前置优先级（BLOCKING）：入队判定**之前**先让两条自救路径优先，
                            // 否则「行尾 `\` + Enter」会被排成聊天消息、`/he`+Enter 会把命令前缀排进队列。
                            let completion_visible = !state.matching_commands().is_empty();
                            let pre_action = streaming_enter_pre_action(
                                &state.input_buffer,
                                completion_visible,
                            );
                            match pre_action {
                                StreamingEnterPre::Newline | StreamingEnterPre::Complete => {
                                    // 交给下面的原换行分支 / 原补全分支。
                                    disposition = StreamingKeyDisposition::Forward;
                                }
                                StreamingEnterPre::Submit => {
                                    if is_session_mutating_command(&composer_text) {
                                        disposition = StreamingKeyDisposition::Reject;
                                    } else if is_panel_command(&composer_text)
                                        || parse_command_input(&composer_text).is_some()
                                        || input_is_alias(&composer_text, "/exit")
                                        || input_is_alias(&composer_text, "/effort")
                                    {
                                        disposition = StreamingKeyDisposition::Forward;
                                    }
                                }
                            }
                        }
                        match disposition {
                            // 编辑/光标/面板快捷键等：继续走既有按键处理链。
                            StreamingKeyDisposition::Forward => {}
                            StreamingKeyDisposition::QueueSubmit => {
                                // C2：流式中 Enter = 入队（绝不 spawn 第二个并发 turn）。
                                let accepted = push_queued_message(
                                    &mut state.queued_user_messages,
                                    &composer_text,
                                );
                                if accepted {
                                    state.clear_input();
                                    state.selected_command_index = 0;
                                    let count = state.queued_count();
                                    // C22：通道 + 文案由 `queue_feedback` 唯一裁定（C2 ⇒ StatusText）。
                                    let (channel, base) =
                                        queue_feedback(QueueFeedbackKind::Enqueued);
                                    let receipt = format!("{base}（{count} 条）");
                                    // C23：`queue_notice` 保证「与运行态无关」的渲染点。
                                    apply_queue_feedback(&mut state, channel, receipt);
                                    // C16：结构化日志（按键裁决 = 入队）。
                                    tui_queue_log(
                                        "info",
                                        "composer_queue_submit",
                                        &state,
                                        &[
                                            ("queued", serde_json::json!(count)),
                                            ("composer_has_text", serde_json::json!(!composer_text.is_empty())),
                                            (
                                                "chars",
                                                serde_json::json!(composer_text.chars().count()),
                                            ),
                                        ],
                                    );
                                } else {
                                    // C6：队列已满 ⇒ 拒绝入队，**保留输入正文**（不静默丢弃）。
                                    // C22：通道 + 文案由 `queue_feedback` 裁定（SystemMessage）。
                                    let (channel, receipt) =
                                        queue_feedback(QueueFeedbackKind::QueueFull);
                                    apply_queue_feedback(&mut state, channel, receipt);
                                    // C16：结构化日志（按键裁决 = 拒绝，原因 = 队列满）。
                                    tui_queue_log(
                                        "warn",
                                        "composer_queue_rejected",
                                        &state,
                                        &[
                                            ("reason", serde_json::json!("queue_full")),
                                            ("queued", serde_json::json!(state.queued_count())),
                                        ],
                                    );
                                }
                                dirty = true;
                                continue;
                            }
                            StreamingKeyDisposition::Reject => {
                                // C2/C4：无正文可提交、或会撕裂在飞轮次的会话变更命令 ⇒
                                // 清空输入 + 提示「本轮结束后可用」。空输入不产生噪声提示。
                                if !composer_text.is_empty() {
                                    let session_mutating =
                                        is_session_mutating_command(&composer_text);
                                    // C22：C4 拒绝 ⇒ SystemMessage 通道（文案来自 `queue_feedback`）。
                                    let (channel, base) = if session_mutating {
                                        queue_feedback(QueueFeedbackKind::SessionMutating)
                                    } else {
                                        queue_feedback(QueueFeedbackKind::QueueFull)
                                    };
                                    let receipt = if session_mutating {
                                        format!("「{composer_text}」{base}")
                                    } else {
                                        base
                                    };
                                    state.clear_input();
                                    state.selected_command_index = 0;
                                    apply_queue_feedback(&mut state, channel, receipt);
                                    // C16：结构化日志（按键裁决 = 拒绝）。
                                    tui_queue_log(
                                        "warn",
                                        "composer_key_rejected",
                                        &state,
                                        &[
                                            // C25：按键的简化分类（绝不记录用户输入的正文字符）。
                                            ("key_class", serde_json::json!(key_class_label(&key))),
                                            (
                                                "reason",
                                                serde_json::json!(if session_mutating {
                                                    "session_mutating_command"
                                                } else {
                                                    "unqueueable_text"
                                                }),
                                            ),
                                        ],
                                    );
                                }
                                dirty = true;
                                continue;
                            }
                        }
                    }

                    if let Some(action) = composer_control_action(&key) {
                        // C24：闩窗口内 Ctrl+U（清空输入）同样是「显式撤销派发」——
                        // 必须先把已写回的正文回队队首，再清空；否则正文被静默丢弃。
                        // C26⑥：回队**权威 payload**（窗口内输入框可能已被改，读字节会丢正文）。
                        if action == ComposerControlAction::ClearInput
                            && queue_dispatch_latch_active(queue_dispatch_deadline)
                        {
                            let pending = queue_dispatch_payload
                                .take()
                                .unwrap_or_else(|| state.input_buffer.trim().to_string());
                            requeue_front(&mut state, pending);
                            queue_dispatch_deadline = None;
                        }
                        state.apply_composer_control(action);
                        continue;
                    }

                    // 面板类快捷键（Ctrl+T 任务看板 / Ctrl+R 模型面板）在流式期同样可用（C3）。
                    if key.modifiers.contains(KeyModifiers::CONTROL) {
                        match key.code {
                            // 波次 44：Ctrl+T 呼出任务看板（与 /tasks 同路径：先同步真实 todo 存储）。
                            // 不占用裸 `t`，避免无法输入以 t 开头的消息。
                            KeyCode::Char('t') => {
                                sync_tasks_from_store(&mut state);
                                state.toggle_task_board();
                                continue;
                            }
                            // wave-35b：Ctrl+R 打开模型面板并远程刷新（与 /models 同路径）。
                            // 不占用裸 `r`，避免无法输入以 r 开头的消息。
                            KeyCode::Char('r') => {
                                let registry = ProviderRegistryStore::new();
                                let view = registry.load_view();
                                let active_id = resolve_active_provider_id(&state);
                                let prov = active_id
                                    .as_ref()
                                    .and_then(|id| view.providers.iter().find(|p| &p.id == id));
                                if let Some(p) = prov {
                                    let items: Vec<ModelItem> = p
                                        .models
                                        .iter()
                                        .map(|m| ModelItem { id: m.id.clone(), name: m.name.clone() })
                                        .collect();
                                    state.open_model_selector(items);
                                    state.status_text = "正在拉取模型列表…".to_string();
                                    spawn_model_discovery(&state, &tx, &p.id);
                                }
                                continue;
                            }
                            _ => {}
                        }
                    }

                    // wave-40 R4：Enter 语义统一由 composer_enter_action 裁定（Shift/Alt → 换行）。
                    if composer_enter_action(&key) == ComposerEnterAction::Newline {
                        state.insert_newline();
                        continue;
                    }

                    match key.code {
                        KeyCode::Tab => {
                            let match_cmds = state.matching_commands();
                            if !match_cmds.is_empty() {
                                let idx = state.selected_command_index.min(match_cmds.len() - 1);
                                let completed = match_cmds[idx].0.to_string();
                                state.set_input(&completed);
                            }
                        }
                        KeyCode::Enter => {
                            // 行尾反斜杠 + Enter：换行（不给网关发送半句话）
                            if state.input_buffer.ends_with('\\') {
                                state.backspace();
                                state.insert_newline();
                                continue;
                            }
                            let match_cmds = state.matching_commands();
                            let selected_cmd = match_cmds
                                .get(state.selected_command_index.min(match_cmds.len().saturating_sub(1)))
                                .map(|(cmd, _)| *cmd);
                            // 已命中（首 token 口径）选中命令时不补全，直接进入分发判定。
                            if let Some(selected_cmd) = selected_cmd {
                                if state.input_buffer.starts_with('/')
                                    && !state.input_buffer.ends_with(' ')
                                    && match_command_token(&state.input_buffer) != Some(selected_cmd)
                                {
                                    state.set_input(selected_cmd);
                                    continue;
                                }
                            }
                            let input = state.input_buffer.trim().to_string();
                            if !input.is_empty() {
                                state.clear_input();
                                state.selected_command_index = 0;

                                // 契约 F-B：分发口径 = **整串**匹配（大小写不敏感）。
                                // `/clear 这个需求怎么拆` 这类「命令 + 正文」不再被当成命令吞掉，
                                // 而是整串作为普通消息发送给模型。
                                let known_command = parse_command_input(&input);

                                if known_command == Some("/resume") {
                                    let raw_sessions = control_plane.list_sessions();
                                    let items: Vec<SessionItem> = raw_sessions
                                        .into_iter()
                                        // W3b：队友子会话只经 `/agents` 进入；主会话列表不混入
                                        // `subagent-*`，否则用户会在 /resume 里看到一堆无名会话。
                                        .filter(|s| is_main_session_id(&s.conversation_id))
                                        .map(|s| SessionItem {
                                            id: s.conversation_id,
                                            title: s.title,
                                            updated_at: s.updated_at_ms.to_string(),
                                        })
                                        .collect();
                                    state.open_session_selector(items);
                                    continue;
                                }

                                if known_command == Some("/provider") {
                                    let registry = ProviderRegistryStore::new();
                                    let view = registry.load_view();
                                    let items: Vec<ProviderItem> = view
                                        .providers
                                        .into_iter()
                                        .map(|p| ProviderItem {
                                            id: p.id,
                                            name: p.name,
                                            protocol: protocol_label(&p.protocol).to_string(),
                                            base_url: p.base_url,
                                        })
                                        .collect();
                                    state.open_provider_selector(items);
                                    continue;
                                }

                                if known_command == Some("/models") {
                                    // wave-35b：cache-first + 始终后台刷新。
                                    // load_view 对空 models 会注入 default（config.rs:873-889），
                                    // 因此不能以「本地非空」判定免拉取：先开缓存面板，再无条件远程刷新。
                                    // wave-35c：统一用 resolve_active_provider_id 取 provider。
                                    let registry = ProviderRegistryStore::new();
                                    let view = registry.load_view();
                                    let current_prov = resolve_active_provider_id(&state)
                                        .and_then(|id| view.providers.iter().find(|p| p.id == id).cloned());

                                    match current_prov {
                                        Some(p) => {
                                            let items: Vec<ModelItem> = p
                                                .models
                                                .iter()
                                                .map(|m| ModelItem { id: m.id.clone(), name: m.name.clone() })
                                                .collect();
                                            state.open_model_selector(items);
                                            state.status_text = "正在拉取模型列表…".to_string();
                                            spawn_model_discovery(&state, &tx, &p.id);
                                        }
                                        None => {
                                            state.open_model_selector(Vec::new());
                                        }
                                    }
                                    continue;
                                }

                                // 严格遵从 /quit 或 /exit 退出规范
                                if known_command == Some("/quit") || input_is_alias(&input, "/exit") {
                                    break;
                                }
                                // `/effort` 是 `/thinking` 的历史别名（不在命令表内，同样整串匹配）。
                                if known_command == Some("/thinking")
                                    || input_is_alias(&input, "/effort")
                                {
                                    state.open_reasoning_effort_selector();
                                    continue;
                                }
                                if known_command == Some("/plan") {
                                    let plans_val = control_plane.plan_list(&state.session_id);
                                    if let Some(plan_state) = tui_plan_from_value(&plans_val) {
                                        state.update_plan(plan_state);
                                    }
                                    state.toggle_plan_mode();
                                    continue;
                                }
                                if known_command == Some("/tasks") {
                                    // W3：真实 Team 共享看板（team_task_list）。
                                    sync_tasks_from_store(&mut state);
                                    state.toggle_task_board();
                                    continue;
                                }
                                if known_command == Some("/agents") {
                                    // W3：真实名册快照（list_agents / 收件箱 / 驱动态 / 会话）。
                                    state.toggle_agent_roster();
                                    continue;
                                }
                                if known_command == Some("/clear") {
                                    // 契约 P-2：/clear = 复位会话作用域 + 换新 session_id。
                                    state.reset_session_scope();
                                    state.session_id =
                                        format!("session-{}", &uuid::Uuid::new_v4().to_string()[..8]);
                                    state.set_status_notice("上下文已清空");
                                    continue;
                                }
                                if known_command == Some("/help") {
                                    let mut help = String::from("命令列表");
                                    for (cmd, desc) in AVAILABLE_COMMANDS.iter() {
                                        help.push_str(&format!("\n{:<11}{}", cmd, desc));
                                    }
                                    // C18②：流式期可用性说明（旧文案让人以为生成中键盘全失效）。
                                    help.push_str(
                                        "\n流式期仍可输入 · Enter 排队（本轮结束后自动发送） · Ctrl+T/Ctrl+R 可用",
                                    );
                                    // 契约 F-L：/cost 移除后网关需要可见入口；只展示剥离凭据后的
                                    // scheme://host[:port]/path（userinfo/query/fragment 全丢弃）。
                                    let gateway = gateway_display(&state.gateway_url);
                                    if !gateway.is_empty() {
                                        help.push_str(&format!("\n网关        {gateway}"));
                                    }
                                    state.messages.push(ChatMessage {
                                        role: MessageRole::System,
                                        content: help,
                                        timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
                                    });
                                    continue;
                                }
                                if known_command == Some("/compact") {
                                    // 契约 C-7：真实压缩（LLM 摘要）必须在后台线程执行，
                                    // 事件循环内不得同步阻塞；成功/失败均经 UiEvent 回主循环。
                                    state.status_text = "正在压缩上下文…".to_string();
                                    let cp = control_plane.clone();
                                    let compact_target = state.session_id.clone();
                                    let tx_compact = tx.clone();
                                    std::thread::spawn(move || {
                                        let _guard = pony_agent_core::agent::runtime_helper::TestRuntimeGuard::new();
                                        let result = cp.compact_session(Some(compact_target.as_str()), true);
                                        let _ = tx_compact.send(UiEvent::CompactFinished { result });
                                    });
                                    continue;
                                }
                                if known_command == Some("/update") {
                                    state.status_text = "正在检查新版本…".to_string();
                                    let tx_update = tx.clone();
                                    std::thread::spawn(move || {
                                        let rt = tokio::runtime::Builder::new_current_thread()
                                            .enable_all()
                                            .build();
                                        let result = match rt {
                                            Ok(runtime) => runtime.block_on(perform_update_check()),
                                            Err(e) => Err(format!("异步运行时启动失败: {e}")),
                                        };
                                        let _ = tx_update.send(UiEvent::UpdateCheckFinished { result });
                                    });
                                    continue;
                                }

                                // C2 最后一道闸：流式期**绝不**并发 spawn 第二个 turn。
                                // 正常路径已在上面的裁决面把流式期 Enter 分流（Forward 只可能是
                                // 面板/只读命令，它们在下方分支各自 continue），这里兜住任何漏网路径：
                                // 退回「排队」而不是撕裂会话历史。
                                if state.is_streaming {
                                    if push_queued_message(
                                        &mut state.queued_user_messages,
                                        &input,
                                    ) {
                                        state.set_status_notice(format!(
                                            "已排队 {} 条 · 本轮结束后自动发送",
                                            state.queued_count()
                                        ));
                                    } else {
                                        state.set_status_notice(
                                            "本轮仍在进行中 · 该消息未排队，请稍后重试",
                                        );
                                    }
                                    dirty = true;
                                    continue;
                                }

                                // 预先检查当前 provider 是否有 API key
                                let prov_id = resolve_active_provider_id(&state)
                                    .unwrap_or_else(|| state.current_provider_id.clone());
                                {
                                    let registry = state.registry_store();
                                    let key_opt = registry.provider_api_key(Some(&prov_id));
                                    if key_opt.as_deref().unwrap_or("").trim().is_empty() {
                                        let view = registry.load_view();
                                        let prov_cfg = view.providers.iter().find(|p| p.id == prov_id);
                                        let env_name = prov_cfg.map(|p| p.api_key_env_var.as_str()).unwrap_or("API_KEY");
                                        let prov_name = prov_cfg.map(|p| p.name.as_str()).unwrap_or(&prov_id);
                                        let err_msg = format!(
                                            "当前提供商 [{}] 未配置 API Key，请在 /provider 中配置或设置环境变量 [{}]",
                                            prov_name, env_name
                                        );
                                        state.add_user_message(&input);
                                        state.messages.push(ChatMessage {
                                            role: MessageRole::System,
                                            content: err_msg,
                                            timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
                                        });
                                        // C24⑤：提交失败但非模态（未进入流式态）⇒ 必须清闩，
                                        // 否则时间窗内排队消息会被闩挡住、窗口过期前无人推进队列。
                                        queue_dispatch_deadline = None;
                                        // C26⑦：正文已被 add_user_message 消费/已落消息区，载荷清零，
                                        // 避免遗留快照在下一个派发周期被误判为「在飞」。
                                        queue_dispatch_payload = None;
                                        continue;
                                    }
                                }

                                // wave-39 R8：新一轮开始前清空本轮回执，并按当前
                                // provider/model 解析上下文窗口（解析不到则回退默认值）。
                                state.begin_turn_usage();
                                // 契约 P-5：提交新消息即清空一次性提示（不再与新一轮进度抢状态区）。
                                state.status_notice = None;
                                state.context_window_tokens = resolve_context_window_tokens(&state);
                                state.add_user_message(&input);
                                state.is_streaming = true;
                                // C24②：提交成功 ⇒ 立即清单飞闩（此后由 is_streaming 兜住并发）。
                                queue_dispatch_deadline = None;
                                // C26⑦：正文已进入本轮，权威载荷使命结束。
                                queue_dispatch_payload = None;
                                state.auto_scroll = true; // 发送新消息时立即恢复自动贴底滚动
                                state.status_text = "Thinking...".to_string();
                                state.transition_runtime_mode(RuntimeMode::Connecting);
                                state.turn_started_at = Some(std::time::Instant::now());

                                let turn_id_str = format!("turn-{}", uuid::Uuid::new_v4());
                                let sink = Arc::new(TuiTurnSink { tx: tx.clone() });
                                let _turn_sink_guard =
                                    TuiTurnSinkGuard::new(&control_plane, &turn_id_str, Arc::clone(&sink));
                                let turn_control_plane = Arc::clone(&control_plane);
                                // 先注册 sink，再向事件循环发布当前 turn id，闭合停止请求的时序窗口。
                                *current_turn_id.lock().unwrap() = Some(turn_id_str.clone());
                                // wave-60 §C9：新一轮开始必须**清掉上一轮遗留的 stopping 态**，
                                // 否则新轮会误显「正在停止…」并拦掉 Esc/Ctrl+C（旧轮终态早已收口）。
                                state.stopping_turn_id = None;
                                state.streaming_turn_id = Some(turn_id_str.clone());
                                // 契约 F-G④：新一轮用户消息 = 新的取消周期，重置 Esc 令牌。
                                goal_round_cancel.reset();

                                let mdl_id = state.model.clone();
                                // wave-38 R1.4：唯一发送点改用 build_turn_input（携带
                                // workspace_id / reasoning_effort / session_id）。
                                // 在 spawn 前组装，避免把整个 TuiState move 进线程。
                                let turn_input = build_turn_input(&state, input, prov_id, mdl_id);
                                // 契约 F-G：把取消令牌与 current_turn_id 句柄带进轮次线程，
                                // 使 Esc 能取消「下一轮」并把 stop_turn 打到**当前**轮。
                                let cancel_for_rounds = goal_round_cancel.clone();
                                let current_turn_for_rounds = Arc::clone(&current_turn_id);

                                let inflight_for_rounds = Arc::clone(&inflight_turns);
                                // C10：把「轮次线程存活标志」带进线程，进入即置位、线程体（首轮 +
                                // goal 续轮循环）结束时复位；panic 展开同样会复位（RAII）。
                                let turn_loop_in_flight_for_thread = Arc::clone(&turn_loop_in_flight);
                                // C11：轮次线程 panic 兜底 —— 轮次体（首轮 + goal 续轮循环）整体在
                                // `catch_unwind` 内执行，panic 时补发 `StreamFailed` 终态，
                                // 杜绝 `is_streaming` 永真导致输入/排队派发永久失效。
                                let tx_turn = tx.clone();
                                let turn_thread = std::thread::spawn(move || {
                                  let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                                    let _in_flight_guard = TurnLoopInFlightGuard::enter(
                                        turn_loop_in_flight_for_thread,
                                    );
                                    let _guard = pony_agent_core::agent::runtime_helper::TestRuntimeGuard::new();
                                    // 首轮 guard 在主事件分支创建并 move 到这里；本轮调用返回即回收，
                                    // 不得跨 goal 续轮驻留。
                                    let initial_turn_sink_guard = _turn_sink_guard;
                                    let session_id = turn_input
                                        .session_id
                                        .clone()
                                        .unwrap_or_else(|| "default".to_string());
                                    let cmd = StartTurnStreamCommand {
                                        turn_id: turn_id_str,
                                        input: turn_input.clone(),
                                    };
                                    // P0 状态边界：本入口（及下方 goal 自动续轮）都在**轮次线程内**直接
                                    // 开新 turn，绕过主循环派发点的 `begin_turn_usage()`。轮次状态复位
                                    // 因此由事件流承担——新 turn 的首个事件 `turn:started`（见
                                    // `TuiTurnSink::emit` → `UiEvent::TurnStarted`）在主循环复位
                                    // `live_metrics` 与 legacy `turn_usage`/`turn_cache_hit_tokens`，
                                    // 故上一轮快照不会泄漏到本轮，无需在此手工复位。
                                    turn_control_plane.start_turn_stream(sink.as_ref(), cmd);
                                    drop(initial_turn_sink_guard);

                                    let mut consecutive: u32 = 0;
                                    // 契约 F-G①：每轮开始前检查取消令牌（Esc）与预算，二者任一即停。
                                    while !should_abort_goal_rounds(
                                        cancel_for_rounds.is_cancelled(),
                                        consecutive,
                                        pony_agent_core::agent::planner::MAX_CONSECUTIVE_AUTO_CONTINUES as u32,
                                    ) {
                                        let active_goal = match pony_agent_core::agent::goal::active_goal_for_session(&session_id) {
                                            Some(g) => g,
                                            None => break,
                                        };
                                        if active_goal.rounds_started >= active_goal.max_goal_rounds {
                                            break;
                                        }
                                        let updated_goal = match pony_agent_core::agent::goal::advance_goal_round(&active_goal.id) {
                                            Ok(g) => g,
                                            Err(_) => break,
                                        };
                                        consecutive += 1;
                                        let round = updated_goal.rounds_started;
                                        let msg = pony_agent_core::agent::goal::build_goal_round_message(
                                            &updated_goal.objective,
                                            round,
                                            updated_goal.max_goal_rounds,
                                        );
                                        let next_turn_id = format!("turn-goal-{round}-{}", uuid::Uuid::new_v4());
                                        let _next_turn_sink_guard = TuiTurnSinkGuard::new(
                                            &turn_control_plane,
                                            &next_turn_id,
                                            Arc::clone(&sink),
                                        );
                                        // 契约 F-G②：先注册 sink，再回写本轮 turn_id；续轮与首轮同一停止生命周期。
                                        *current_turn_for_rounds
                                            .lock()
                                            .unwrap_or_else(|e| e.into_inner()) = Some(next_turn_id.clone());
                                        let mut next_input = turn_input.clone();
                                        next_input.message = msg;
                                        next_input.display_message = Some(format!("[goal round {round}] 推进目标：{}", updated_goal.objective));
                                        let next_cmd = StartTurnStreamCommand {
                                            turn_id: next_turn_id,
                                            input: next_input,
                                        };
                                        turn_control_plane.start_turn_stream(sink.as_ref(), next_cmd);
                                    }
                                  }));
                                  if let Err(payload) = outcome {
                                      // panic 也是「终态」：必须让事件循环退出流式态（C11）。
                                      let _ = tx_turn.send(UiEvent::StreamFailed(format!(
                                          "轮次线程 panic: {}",
                                          panic_payload_message(payload.as_ref())
                                      )));
                                  }
                                });
                                // MutexGuard 必须在 turn_thread 移入前释放：临时量绑定后显式 drop。
                                let turns = inflight_for_rounds
                                    .lock()
                                    .unwrap_or_else(|error| error.into_inner());
                                let mut turns = turns;
                                turns.push(turn_thread);
                                drop(turns);
                            }
                        }
                        KeyCode::Char(c) => {
                            if c == '\n' {
                                state.insert_newline();
                            } else if c != '\r' {
                                state.insert_char(c);
                            }
                        }
                        KeyCode::Backspace => {
                            state.backspace();
                        }
                        KeyCode::Delete => {
                            state.delete_forward();
                        }
                        KeyCode::Left => {
                            state.move_cursor_left();
                        }
                        KeyCode::Right => {
                            state.move_cursor_right();
                        }
                        KeyCode::Home => {
                            state.cursor_line_home();
                        }
                        KeyCode::End => {
                            if state.auto_scroll {
                                state.cursor_line_end();
                            } else {
                                // 手动上翻时 End 回到最新消息；输入编辑仍可用 Ctrl+E。
                                state.scroll_to_bottom();
                            }
                        }
                        KeyCode::Up => {
                            let match_cmds = state.matching_commands();
                            if !match_cmds.is_empty() {
                                if state.selected_command_index > 0 {
                                    state.selected_command_index -= 1;
                                }
                            } else {
                                state.scroll_up(2);
                            }
                        }
                        KeyCode::Down => {
                            let match_cmds = state.matching_commands();
                            if !match_cmds.is_empty() {
                                if state.selected_command_index + 1 < match_cmds.len() {
                                    state.selected_command_index += 1;
                                }
                            } else {
                                state.scroll_down(2, state.max_scroll_offset);
                            }
                        }
                        KeyCode::PageUp => {
                            state.scroll_up(10);
                        }
                        KeyCode::PageDown => {
                            state.scroll_down(10, state.max_scroll_offset);
                        }
                        _ => {}
                    }
                }
                // C21：合成自动提交（派发闸门的唯一构造点）。
                // 与裸 `InputKey(Enter)` 的区别：这里**重新校验**界面此刻是否仍空闲 ——
                // 派发与消费之间若弹出模态/待作答请求，回灌的 Enter 会被面板吃掉
                //（极端情况甚至误触发「同意审批」）。被挡下时：释放单飞闩、正文留在输入框，
                // 用户可见并可自行按 Enter，不会丢消息、也不会与新 run 交叠。
                UiEvent::QueuedAutoSubmit => {
                    // C26④：**权威载荷比对**。回队/提交都必须用派发时刻的快照，而不是「此刻
                    // 输入框里的字节」——用户可能在窗口内用 Ctrl+W / Backspace / Delete 改（甚至
                    // 擦光）输入框，那时回队 `input_buffer` 会把已派发的正文静默丢掉。
                    if queued_dispatch_diverged(
                        queue_dispatch_payload.as_deref(),
                        &state.input_buffer,
                    ) {
                        // 输入框已偏离派发快照：把**原 payload** 回队队首，保留用户当前输入不动，
                        // 清闩 + 清 payload，等下一次空闲再重试（不得停摆）。
                        if let Some(payload) = queue_dispatch_payload.take() {
                            requeue_front(&mut state, payload);
                        }
                        queue_dispatch_deadline = None;
                        tui_queue_log(
                            "warn",
                            "composer_queue_dispatch_blocked",
                            &state,
                            &[
                                ("reason", serde_json::json!("composer_diverged")),
                                ("requeued", serde_json::json!(state.queued_count())),
                            ],
                        );
                        dirty = true;
                        // C26⑤：消费点必须 `continue`，否则同一轮会继续落到派发闸门，
                        // 在「composer 已空 + 闩已清」时二次派发（1 终态 2 派发）。
                        continue;
                    }
                    let mode_is_modal = is_modal_mode(&state.mode);
                    if should_reinject_auto_submit(
                        mode_is_modal,
                        state.pending_request.is_some(),
                        state.ask_user_state.is_some(),
                        state.permission_state.is_some(),
                    ) {
                        let _ = tx.send(UiEvent::InputKey(crossterm::event::KeyEvent::new(
                            KeyCode::Enter,
                            KeyModifiers::NONE,
                        )));
                        tui_queue_log("info", "composer_queue_autosubmit", &state, &[]);
                    } else {
                        // C24③：放弃本次回灌时**把消息回队队首**并清空输入框 + 清闩，
                        // 待下一次空闲重试 —— 只留稿会让整个队列永久停摆（L3-A F6）。
                        // C26⑦：优先回队权威 payload（无 payload 才回落到输入框内容）。
                        let pending = queue_dispatch_payload
                            .take()
                            .unwrap_or_else(|| state.input_buffer.trim().to_string());
                        requeue_front(&mut state, pending);
                        state.clear_input();
                        queue_dispatch_deadline = None;
                        tui_queue_log(
                            "warn",
                            "composer_queue_autosubmit_blocked",
                            &state,
                            &[
                                ("mode", serde_json::json!(format!("{:?}", state.mode))),
                                ("requeued", serde_json::json!(state.queued_count())),
                            ],
                        );
                    }
                    dirty = true;
                    // C26⑤：无论是否回灌，消费点都必须 `continue`（阻止同轮二次派发）。
                    continue;
                }
                // 契约 F-C：粘贴只插入输入缓冲，**绝不提交**（提交仍由用户按 Enter 决定）。
                // C9（`tui-streaming-input`）：流式期粘贴与键盘编辑同口径 —— 照样插入缓冲，
                // 不再早退（旧实现 `if state.is_streaming { continue; }` 让流式中粘贴被静默丢弃）。
                UiEvent::InputPaste(text) => {
                    state.insert_paste(&text);
                    dirty = true;
                }
                UiEvent::StreamDelta(delta) => {
                    state.append_assistant_chunk(&delta);
                    if !matches!(state.runtime_mode, RuntimeMode::ExecutingTool(_)) {
                        state.transition_runtime_mode(RuntimeMode::Responding);
                    }
                }
                UiEvent::StreamToolCall(call) => {
                    let tool_name = clean_tool_name_display(&call);
                    state.transition_runtime_mode(RuntimeMode::ExecutingTool(tool_name));
                    state.add_or_update_tool_event(&call, false);
                }
                UiEvent::StreamToolResult(res) => {
                    state.add_or_update_tool_event(&res, true);
                    state.transition_runtime_mode(RuntimeMode::Responding);
                }
                UiEvent::StreamCompleted => {
                    state.is_streaming = false;
                    state.stopping_turn_id = None;
                    state.streaming_turn_id = None;
                    // wave-60 §C8：本轮结束 ⇒ 双击 Esc 的上膛失效。
                    stop_sequence.reset();
                    state.transition_runtime_mode(RuntimeMode::Completed);
                    state.status_text = "Turn completed".to_string();
                    sync_tasks_from_store(&mut state);
                    if let Some(goal_data) = pony_agent_core::agent::goal::active_goal_for_session(&state.session_id) {
                        state.set_goal(tui_goal_from_goal(&goal_data));
                    }
                }
                // wave-60 §C2/C9：停止请求受理 ⇒ 只进「正在停止…」（流式态保持，终态未到）。
                UiEvent::StopRequested { turn_id } => {
                    state.apply_stop_requested(&turn_id);
                }
                // wave-60 §C9：`turn:cancelled` 终态 ⇒ 事件驱动「已停止」并退出流式态。
                UiEvent::StopTerminal { turn_id, stop_reason } => {
                    // 无在飞停止请求时，`apply_stop_terminal` 只认「当前流式轮」。goal 续轮在
                    // 轮次线程内换 turn_id（不经过事件循环），故此处以 `current_turn_id` 兜底同步。
                    if state.stopping_turn_id.is_none() {
                        let active_turn = { current_turn_id.lock().unwrap().clone() };
                        if let Some(active_turn) = active_turn {
                            state.streaming_turn_id = Some(active_turn);
                        }
                    }
                    state.apply_stop_terminal(&turn_id, stop_reason.as_deref());
                    // wave-60 §C8：turn 结束 ⇒ 双击 Esc 的上膛失效。
                    stop_sequence.reset();
                }
                // P0：轮次起点事件 ⇒ 与派发点**同一复位点** `begin_turn_usage()`
                //（legacy `turn_usage` / `turn_cache_hit_tokens` 与快照 `live_metrics` 同点清空）。
                // 该闸门覆盖 goal 自动续轮等在轮次线程内直接开轮的入口。
                UiEvent::TurnStarted => {
                    state.begin_turn_usage();
                }
                // P0：本轮指标快照 → 指标行唯一来源（快照存在时不再读 legacy 计量字段）。
                UiEvent::StreamMetrics(metrics) => {
                    state.apply_live_metrics(metrics);
                }
                // wave-39 R8：本轮 token 回执 → 幂等覆盖计入累计指标。
                UiEvent::StreamUsage {
                    input_tokens,
                    output_tokens,
                    cache_hit_input_tokens,
                    total_tokens,
                } => {
                    state.apply_turn_usage(
                        input_tokens,
                        output_tokens,
                        cache_hit_input_tokens,
                        total_tokens,
                    );
                }
                UiEvent::StreamFailed(err) => {
                    state.is_streaming = false;
                    // wave-60 §C7/C9：失败是终态之一 ⇒ 必须清 stopping 兜底态（不得卡在「正在停止…」）。
                    state.stopping_turn_id = None;
                    state.streaming_turn_id = None;
                    stop_sequence.reset();
                    // wave-tui-session-resilience C3/C4：消息区只显示一句话摘要，
                    // 原始 400 字符响应体/堆栈默认折叠，仅留运行日志与 issue 上报通道。
                    let summary = user_facing_error_summary(&err);
                    let folded = should_fold_error_detail(&err);
                    eprintln!("[pony-tui] turn failed detail (folded={folded}): {err}");
                    state.transition_runtime_mode(RuntimeMode::Failed(summary.clone()));
                    state.status_text = if is_transient_provider_error(&err) {
                        format!("重试中（1/5）· {}", summary)
                    } else {
                        summary.clone()
                    };
                    state.messages.push(ChatMessage {
                        role: MessageRole::System,
                        content: summary,
                        timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
                    });
                }
                UiEvent::ModelsDiscovered { provider_id, ids } => {
                    // 契约 F-J：视图变换收敛进 `apply_models_discovered`（幂等、可单测）；
                    // 本臂只负责「调用 → 落盘 → 刷新 UI 状态」。
                    let registry = ProviderRegistryStore::new();
                    let mut view = registry.load_view();
                    let applied = apply_models_discovered(&mut view, &provider_id, &ids);
                    let saved = match applied {
                        Ok(()) => registry.save_view(view),
                        Err(error) => Err(error),
                    };
                    match saved {
                        Ok(saved) => {
                            let items: Vec<ModelItem> = saved
                                .providers
                                .iter()
                                .find(|p| p.id == provider_id)
                                .map(|p| p.models.iter().map(|m| ModelItem { id: m.id.clone(), name: m.name.clone() }).collect())
                                .unwrap_or_default();
                            let n = items.len();
                            if state.mode == InteractionMode::SelectingModel {
                                // 面板已开：刷新列表，保留当前命中项选择，否则回到 0。
                                let prev = state.model_list.get(state.selected_model_index).map(|m| m.id.clone())
                                    .or_else(|| Some(state.model.clone()));
                                state.model_list = items;
                                state.selected_model_index = state
                                    .model_list
                                    .iter()
                                    .position(|m| Some(&m.id) == prev.as_ref())
                                    .unwrap_or(0);
                            }
                            // 面板未打开：仅刷新注册表与状态，不强行弹面板。
                            state.status_text = format!("已加载 {n} 个模型");
                        }
                        Err(e) => {
                            state.set_status_notice(format!("模型目录保存失败: {e}"));
                            state.messages.push(ChatMessage {
                                role: MessageRole::System,
                                content: format!("模型目录保存失败（{provider_id}）: {e}"),
                                timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
                            });
                        }
                    }
                }
                UiEvent::ModelsDiscoverFailed { provider_id, error } => {
                    // 拉取失败：System 消息呈现摘要，不 panic、不退出；
                    // 面板已开则保留缓存列表（cache-first 语义），仅更新状态。
                    let summary: String = error.chars().take(200).collect();
                    state.status_text = format!("模型列表拉取失败: {summary}");
                    state.messages.push(ChatMessage {
                        role: MessageRole::System,
                        content: format!("模型列表拉取失败（{provider_id}）: {summary}"),
                        timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
                    });
                }
                // 契约 F-E：输入泵已退出（poll/read 报错）。stderr 被 dup2 到日志文件，
                // 全屏界面是用户唯一可见面 ⇒ 必须在这里显式告知，不能只写日志。
                UiEvent::InputPumpFailed { error } => {
                    let summary: String = error.chars().take(200).collect();
                    state.status_text = format!("输入通道异常: {summary}");
                    state.messages.push(ChatMessage {
                        role: MessageRole::System,
                        content: format!(
                            "输入通道异常，键盘/鼠标事件已停止（建议退出并重新启动 TUI）: {summary}"
                        ),
                        timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
                    });
                    dirty = true;
                }
                // 契约 C-7：成功/失败都追加 System 消息；未压缩时如实说明，不谎报已压缩。
                // 文案统一由 core 纯函数 `compact_outcome_message` 产出（审查 F4，避免多处漂移）。
                UiEvent::CompactFinished { result } => {
                    state.status_text = match &result {
                        Ok(outcome) if outcome.compacted => {
                            format!("已压缩 {} 条消息", outcome.compressed_count)
                        }
                        Ok(_) => "无需压缩".to_string(),
                        Err(_) => "压缩失败".to_string(),
                    };
                    state.messages.push(ChatMessage {
                        role: MessageRole::System,
                        content: compact_outcome_message(&result),
                        timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
                    });
                }
                UiEvent::UpdateCheckFinished { result } => {
                    let content = match &result {
                        Ok(outcome) => {
                            state.status_text = if outcome.has_update {
                                format!("发现新版本 v{}", outcome.latest_version)
                            } else {
                                "已是最新版本".to_string()
                            };
                            format_update_feedback(outcome)
                        }
                        Err(err) => {
                            state.status_text = "检查更新失败".to_string();
                            format!("检查更新失败：{}", err)
                        }
                    };
                    state.messages.push(ChatMessage {
                        role: MessageRole::System,
                        content,
                        timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
                    });
                }
                UiEvent::RequestPermission(request) => {
                    state.start_permission_approval(request);
                }
                UiEvent::PermissionDecided { .. } => {}
                UiEvent::AskUserPrompt { request_id, questions } => {
                    state.start_ask_user(request_id, questions);
                }
                UiEvent::AskUserAnswered { .. } => {}
                UiEvent::PlanUpdated(plan) => {
                    state.update_plan(plan);
                }
                UiEvent::TaskBoardUpdated(tasks) => {
                    state.update_task_board(tasks);
                }
                UiEvent::GoalUpdated(goal) => {
                    state.set_goal(goal);
                }
                UiEvent::TurnSuspended { .. } => {
                    state.is_streaming = false;
                    state.status_text = "等待用户输入确认...".to_string();
                    let _ = check_pending_asks(&mut state, &control_plane);
                }
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }

        // ── C5/C10/C15/C17：排队消息自动派发闸门（单点、单飞、事件回灌）─────────────
        // 位置契约：**处理完一个事件之后、下一轮计算 `poll_ms` 之前**只在这里判定一次。
        // 派发不内联复用提交逻辑，而是把正文写回输入框 + 向同一 `tx` 回灌一条合成 Enter，
        // 让提交继续走唯一那条路径（API key 校验 → begin_turn_usage → add_user_message → spawn）。
        // C15：取件**只能**经 `take_next_queued_for_dispatch`（闸门 + 「输入框为空」前哨 + FIFO
        // 取件收成一个生产符号，A7 直接断言它，而不是测试内自建的复刻品）。
        {
            // C17：`ui_idle` 包含「无待作答/待审批」——挂起期（TurnSuspended 已把
            // `is_streaming` 置 false 但 run 未收口）派发会让新 turn 与挂起 run 交叠。
            let ui_idle = is_ui_idle_for_dispatch(&state);
            // C10（契约修订 2026-11-06）：判据是「有没有**在飞轮次线程**」，不是「有没有 armed goal」
            // —— goal armed 但续轮没在跑时，只看 goal 存在会让排队消息永不自动派发（用户感知卡住）。
            // 标志由轮次线程内的 `TurnLoopInFlightGuard` 维护（正常结束与 panic 展开都会复位）。
            let turn_in_flight = turn_loop_in_flight.load(Ordering::SeqCst);
            let is_streaming = state.is_streaming;
            // C24：闩活性 = 时间窗未过期（`now < deadline`），不再依赖「收到 InputKey 就清」。
            let dispatch_pending = queue_dispatch_latch_active(queue_dispatch_deadline);
            // C27：**孤儿载荷收割** —— 闩已过期但 payload 仍非空，说明上一条派发的 QAS 还在飞
            // （或已被 composer 变更吃掉）。此刻若继续取件会在下面覆盖单槽 payload ⇒ 上一条既未
            // 提交也未回队（静默丢正文）。故先回队该 payload、清输入框、清 payload，本轮不再派发。
            if let Some(orphan) =
                orphan_dispatch_payload(queue_dispatch_payload.as_deref(), dispatch_pending)
            {
                requeue_front(&mut state, orphan);
                state.clear_input();
                queue_dispatch_payload = None;
                tui_queue_log(
                    "warn",
                    "composer_queue_dispatch_blocked",
                    &state,
                    &[
                        ("reason", serde_json::json!("orphan_payload_requeued")),
                        ("requeued", serde_json::json!(state.queued_count())),
                    ],
                );
                dirty = true;
                continue;
            }
            // C25：派发前后队列深度（纯函数 `queue_dispatch_fields` 承载字段名）。
            let queue_depth_before = state.queued_count();
            if let Some(next_message) = take_next_queued_for_dispatch(
                &mut state,
                is_streaming,
                ui_idle,
                turn_in_flight,
                dispatch_pending,
            ) {
                // C26②：先留**权威载荷快照**（撤销/回队/分歧比对一律用它，不读输入框字节）。
                queue_dispatch_payload = Some(next_message.clone());
                // C14：写回经纯函数净化（正文 + 一个尾部空格）：既不覆盖正文，也不让
                // `ends_with('\\')` 或 `/hel` 前缀劫持这次提交。
                state.set_input(&composer_buffer_for_dispatch(&next_message));
                state.selected_command_index = 0;
                // C24：置时间窗闩（窗口内闸门一律不派发）。
                queue_dispatch_deadline = Some(
                    std::time::Instant::now() + std::time::Duration::from_millis(QUEUE_DISPATCH_LATCH_MS),
                );
                // 派发后队列已消费该条：清掉「已排队…」短时反馈残影，指示行回落到真实计数。
                state.queue_notice = None;
                // C16/C25：结构化日志（派发）。事件名与字段先绑定再一次性写入 ——
                // 深度字段由纯函数 `queue_dispatch_fields` 承载（避免调用点手写字段名漂移）。
                let dispatch_event = "composer_queue_dispatch";
                let mut dispatch_fields =
                    queue_dispatch_fields(queue_depth_before, state.queued_count());
                dispatch_fields.push(("chars", serde_json::json!(next_message.chars().count())));
                tui_queue_log("info", dispatch_event, &state, &dispatch_fields);
                // C21：不直接注入 `InputKey(Enter)`，而是发**带标记的** `QueuedAutoSubmit`；
                // 由事件循环在校验「此刻仍是空闲态」后才回灌真正的 Enter（见该变体分支）。
                let _ = tx.send(UiEvent::QueuedAutoSubmit);
                dirty = true;
                continue;
            }
        }
    }

    // wave-tui-session-resilience C7/C8：退出先取消在飞 turn、有界等待收敛，
    // 再交还终端。NFR external_call_timeout_ms ≤ 3000，绝不留下持有 tty 的活线程。
    let inflight_turn_id = current_turn_id
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    let cleanup_plan = exit_cleanup_plan(inflight_turn_id.is_some(), 3000);
    for step in cleanup_plan {
        match step {
            "cancel" => {
                if let Some(turn_id) = inflight_turn_id.as_deref() {
                    control_plane.stop_turn(StopTurnCommand {
                        turn_id: turn_id.to_string(),
                        stop_reason: Some(StopReason::User),
                    });
                }
            }
            "wait" => {
                let deadline = std::time::Instant::now() + std::time::Duration::from_millis(3000);
                let handles: Vec<std::thread::JoinHandle<()>> = inflight_turns
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .drain(..)
                    .collect();
                for handle in handles {
                    if handle.is_finished() {
                        let _ = handle.join();
                        continue;
                    }
                    // 有界等待：预留一个轮询片给取消生效，剩余预算始终 ≤3000ms。
                    let slice = deadline
                        .saturating_duration_since(std::time::Instant::now())
                        .min(std::time::Duration::from_millis(50));
                    if !slice.is_zero() {
                        std::thread::sleep(slice);
                    }
                    if handle.is_finished() {
                        let _ = handle.join();
                    }
                    // 未结束的 JoinHandle 在此 drop（detach）；进程随后经 restore→hint→exit
                    // 真正退出，不留下持有 tty 的活线程。
                    if std::time::Instant::now() >= deadline {
                        break;
                    }
                }
            }
            _ => {}
        }
    }

    // 终端恢复已统一由 run_tui 的 T2c 保证器执行（catch_unwind 外层）。
    // 返回最终 session id：退出提示 `pony resume <id>` 必须指向**这一轮真实会话**。
    Ok(state.session_id)
}

// wave-34：非交互 CLI 入口。`pony --version / --help / --doctor` 在不初始化终端
//（不 enable_raw_mode、不进出备屏）的前提下输出信息后退出，消除 SSH/脚本排障盲点。
#[derive(Debug)]
pub enum CliAction {
    RunTui,
    /// `pony resume [<session-id>]`：恢复指定会话；`None` = 恢复最近一次会话。
    ResumeTui(Option<String>),
    Version,
    Help,
    Doctor,
    DemoAskUser,
    Invalid(String),
}

pub fn parse_cli_action(args: &[String]) -> CliAction {
    match args.iter().next() {
        None => CliAction::RunTui,
        Some(a) if a == "resume" => match args.get(1) {
            None => CliAction::ResumeTui(None),
            // 第二个位置参数必须是会话 id；`-` 开头说明用户把别的选项拼错了，直接报错，
            // 避免把 `pony resume --foo` 当成「恢复名为 --foo 的会话」。
            Some(id) if id.starts_with('-') => CliAction::Invalid(id.clone()),
            Some(id) => CliAction::ResumeTui(Some(id.clone())),
        },
        Some(a) if a == "--version" || a == "-V" => CliAction::Version,
        Some(a) if a == "--help" || a == "-h" => CliAction::Help,
        Some(a) if a == "--doctor" => CliAction::Doctor,
        Some(a) if a == "--demo-ask-user" => CliAction::DemoAskUser,
        Some(a) if a.starts_with('-') => CliAction::Invalid(a.clone()),
        Some(_) => CliAction::RunTui,
    }
}

/// `pony resume [<id>]` 的解析结果。
pub struct ResumeTarget {
    pub session_id: String,
    /// 该会话是否已有持久化记录（否则只是「沿用这个 id 开新会话」）。
    pub existed: bool,
}

/// `pony resume [<id>]` 的目标解析：`None` 取最近会话；显式 id 一律接受。
///
/// 为什么显式 id 不做「必须存在」校验：退出提示里的 id 是复制粘贴来的，若用户当前会话
/// 还没产生任何轮次（未落库），严格校验会让提示变成假承诺。这里改为「存在就恢复历史、
/// 不存在就以该 id 开新会话」，并在 TUI 内给出可见说明。
fn resolve_resume_target(requested: Option<&str>) -> Result<ResumeTarget, String> {
    let all_sessions = SessionStore::new().list_sessions();
    match requested {
        // 显式 id 一律接受（可能来自 `.dev-team` 诊断/队友会话）；存在性按全量库判定。
        Some(id) => Ok(ResumeTarget {
            session_id: id.to_string(),
            existed: all_sessions
                .iter()
                .any(|session| session.conversation_id == id),
        }),
        // 缺省「最近会话」只取主会话：队友子会话有独立入口 `/agents`，不抢默认续接。
        None => all_sessions
            .iter()
            .find(|session| is_main_session_id(&session.conversation_id))
            .map(|session| ResumeTarget {
                session_id: session.conversation_id.clone(),
                existed: true,
            })
            .ok_or_else(|| "当前没有可恢复的会话。请先运行 `pony` 开始一个新会话。".to_string()),
    }
}

/// W3b：主会话 id 判定（队友/子代理会话只经 `/agents` 进入，不混入 `/resume` 默认列表）。
pub fn is_main_session_id(session_id: &str) -> bool {
    !session_id.starts_with("subagent-") && !session_id.starts_with("teammate-")
}



fn cli_version_string() -> String {
    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "<unknown>".to_string());
    format!(
        "pony {} · {}-{}\nbinary: {}",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        exe
    )
}

fn cli_help_string() -> String {
    format!(
        "pony {} — 交互式 TUI Agent\n\n\
用法: pony [选项]\n       pony resume [<session-id>]   恢复会话（缺省 = 最近一次）\n\n\
选项:\n  --version, -V   显示版本与二进制路径后退出\n  --help, -h      显示本帮助后退出\n  --doctor        终端环境自检（TTY/颜色/尺寸/locale/日志目录/PATH 冲突）\n\n\
退出 TUI 后会打印 `pony resume <session-id>`，复制即可以同一会话继续。\n\n\
交互键位:\n  Enter           发送输入\n  Alt+Enter       输入内换行\n  /               命令前缀（/help /clear /quit /exit 等）\n  Esc             清空输入 / 中断弹窗\n  Ctrl-C          退出 TUI\n  F2              切换鼠标捕获（默认开；关闭后可用鼠标选择文本）\n  ↑ / ↓           历史翻页\n  PageUp/PageDown 消息滚动\n",
        env!("CARGO_PKG_VERSION")
    )
}

fn run_doctor() -> i32 {
    let mut fail = 0usize;

    // a) TTY：stdin 必须是 TTY（TUI 读取键盘）；stdout 允许被重定向（便于把报告存档/管道断言）。
    let in_tty = std::io::stdin().is_terminal();
    let out_tty = std::io::stdout().is_terminal();
    match (in_tty, out_tty) {
        (true, true) => println!("[ok] stdio: stdin=TTY stdout=TTY · 交互终端就绪"),
        (true, false) => println!("[warn] stdio: stdout 已重定向 · 报告可正常输出，TUI 内仍使用真实终端"),
        (false, _) => {
            fail += 1;
            println!("[fail] stdio: stdin 非交互终端（TUI 需要 TTY；CLI 场景可用 ponycli）");
        }
    }

    // b) 颜色环境
    let term = std::env::var("TERM").unwrap_or_default();
    let colorterm = std::env::var("COLORTERM").unwrap_or_default();
    let nocolor = std::env::var("NO_COLOR").unwrap_or_default();
    if term.is_empty() {
        println!("[warn] TERM: <unset> · 真彩色可能降级");
    } else {
        println!("[ok] TERM: {term}");
    }
    if colorterm.contains("truecolor") || colorterm.contains("24bit") {
        println!("[ok] COLORTERM: {colorterm} · 真彩色可用");
    } else {
        let shown = if colorterm.is_empty() { "<unset>" } else { &colorterm };
        println!("[warn] COLORTERM: {shown} · 未启用 truecolor");
    }
    if nocolor.is_empty() {
        println!("[ok] NO_COLOR: <unset> · 颜色输出未被禁用");
    } else {
        println!("[warn] NO_COLOR: {nocolor} · 颜色输出被禁用");
    }

    // c) 终端尺寸
    match crossterm::terminal::size() {
        Ok((w, h)) if w >= 8 && h >= 24 => println!("[ok] size: {w}x{h} · ≥24 行 × 8 列"),
        Ok((w, h)) => {
            fail += 1;
            println!("[fail] size: {w}x{h} · 低于最低要求 24 行 × 8 列，界面将错乱");
        }
        Err(e) => {
            fail += 1;
            println!("[fail] size: <n/a> · 无法读取终端尺寸: {e}");
        }
    }

    // d) locale UTF-8
    let mut utf8_ok = false;
    let mut seen = String::new();
    for v in ["LC_ALL", "LC_CTYPE", "LANG"] {
        if let Ok(val) = std::env::var(v) {
            if !val.is_empty() {
                seen.push_str(&format!("{v}={val} "));
                let lower = val.to_lowercase();
                if lower.contains("utf-8") || lower.contains("utf8") {
                    utf8_ok = true;
                }
            }
        }
    }
    let seen = seen.trim().to_string();
    if utf8_ok {
        println!("[ok] locale: {seen} · UTF-8 就绪");
    } else if seen.is_empty() {
        println!("[warn] locale: <unset> · LC_ALL/LC_CTYPE/LANG 未设置，中文/CJK 可能错位");
    } else {
        println!("[warn] locale: {seen} · 非 UTF-8 locale，中文/CJK 可能错位");
    }

    // e) tui-logs 可写
    let log_dir = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".pony-agent")
        .join("tui-logs");
    if std::fs::create_dir_all(&log_dir).is_ok() {
        let probe = log_dir.join(".doctor-probe");
        match std::fs::write(&probe, b"ok") {
            Ok(_) => {
                let _ = std::fs::remove_file(&probe);
                println!("[ok] tui-logs: {} · 可写", log_dir.display());
            }
            Err(e) => {
                fail += 1;
                println!("[fail] tui-logs: {} · 不可写: {e}", log_dir.display());
            }
        }
    } else {
        fail += 1;
        println!("[fail] tui-logs: {} · 无法创建目录", log_dir.display());
    }

    // f) PATH 冲突：按 canonicalize 去重（同一 realpath 只列一条），且仅列「与当前 exe 不同」者。
    let exe_self = std::env::current_exe().ok().and_then(|p| std::fs::canonicalize(p).ok());
    let mut conflicts: Vec<PathBuf> = Vec::new();
    let mut seen_canon: Vec<PathBuf> = Vec::new();
    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let cand = dir.join("pony");
            if !cand.is_file() {
                continue;
            }
            let canon = std::fs::canonicalize(&cand).unwrap_or_else(|_| cand.clone());
            if Some(&canon) == exe_self.as_ref() || seen_canon.contains(&canon) {
                continue;
            }
            seen_canon.push(canon);
            conflicts.push(cand);
        }
    }
    if conflicts.is_empty() {
        println!("[ok] PATH: 无 pony 冲突");
    } else {
        let list = conflicts.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(" ");
        println!("[warn] PATH: {list} · PATH 冲突：SSH 下可能调到其它 pony 二进制");
    }

    if fail == 0 { 0 } else { 1 }
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

fn main() {
    // Wave-5 P0：可观测性引导（panic hook + OTel provider；失败只降级 warn，不阻塞启动）。
    let _observability = ObservabilityBootstrap::install();
    let args: Vec<String> = std::env::args().skip(1).collect();
    match parse_cli_action(&args) {
        CliAction::Version => {
            println!("{}", cli_version_string());
            std::process::exit(0);
        }
        CliAction::Help => {
            println!("{}", cli_help_string());
            std::process::exit(0);
        }
        CliAction::Doctor => {
            std::process::exit(run_doctor());
        }
        CliAction::DemoAskUser => {
            let _guard = pony_agent_core::agent::runtime_helper::TestRuntimeGuard::new();
            if let Err(e) = run_tui_demo_ask() {
                eprintln!("TUI Error: {e}");
                // `process::exit` 不跑 Drop ⇒ 显式有界 shutdown，保证末批 span 不丢。
                pony_agent_core::agent::otel::shutdown();
                std::process::exit(1);
            }
        }
        CliAction::ResumeTui(requested) => {
            // 目标解析必须在进入 raw mode 之前完成：空库/无参场景的错误要可见，而不是空屏。
            let resume_target = match resolve_resume_target(requested.as_deref()) {
                Ok(target) => target,
                Err(message) => {
                    eprintln!("pony resume: {message}");
                    std::process::exit(2);
                }
            };
            let _guard = pony_agent_core::agent::runtime_helper::TestRuntimeGuard::new();
            if let Err(e) = run_tui(Some(resume_target)) {
                eprintln!("TUI Error: {e}");
                pony_agent_core::agent::otel::shutdown();
                std::process::exit(1);
            }
        }
        CliAction::Invalid(arg) => {
            eprintln!("pony: 未知参数 {arg}\n用法: pony [--version | --help | --doctor | resume [<session-id>]]");
            std::process::exit(2);
        }
        CliAction::RunTui => {
            let _guard = pony_agent_core::agent::runtime_helper::TestRuntimeGuard::new();
            if let Err(e) = run_tui(None) {
                eprintln!("TUI Error: {e}");
                // `process::exit` 不跑 Drop ⇒ 显式有界 shutdown，保证末批 span 不丢。
                pony_agent_core::agent::otel::shutdown();
                std::process::exit(1);
            }
        }
    }
}

#[cfg(test)]
mod wave43_unit_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_pending_ui_request_from_value() {
        let val_ask = json!({
            "requestId": "ask-101",
            "version": 3,
            "runId": "run-xyz",
            "requestKind": "interaction"
        });
        let handle = pending_ui_request_from_value(&val_ask).expect("must parse handle");
        assert_eq!(handle.request_id, "ask-101");
        assert_eq!(handle.version, 3);
        assert_eq!(handle.run_id, Some("run-xyz".to_string()));
        assert!(!handle.is_approval);

        let val_appr = json!({
            "requestId": "appr-202",
            "version": 1,
            "requestKind": "approval"
        });
        let handle_appr = pending_ui_request_from_value(&val_appr).expect("must parse handle");
        assert_eq!(handle_appr.request_id, "appr-202");
        assert!(handle_appr.is_approval);

        let val_appr_req = json!({
            "requestId": "appr-203",
            "version": 1,
            "requestKind": "approval_required"
        });
        let handle_appr_req = pending_ui_request_from_value(&val_appr_req).expect("must parse handle");
        assert!(handle_appr_req.is_approval);
    }

    #[test]
    fn test_ask_question_from_pending_mapping() {
        let val = json!({
            "descriptorId": "confirmation_box",
            "prompt": "确定要继续吗？",
            "options": [
                { "label": "选项 A", "description": "说明 A" },
                { "label": "选项 B" }
            ]
        });
        let q = ask_question_from_pending(&val);
        assert_eq!(q.question, "确定要继续吗？");
        assert_eq!(q.header, Some("confirmation_box".to_string()));
        assert_eq!(q.options.len(), 2);
        assert_eq!(q.options[0].label, "选项 A");
        assert_eq!(q.options[0].description, Some("说明 A".to_string()));
    }

    #[test]
    fn test_permission_request_from_pending_mapping() {
        let val = json!({
            "requestId": "req-999",
            "sessionId": "sess-abc",
            "descriptorId": "bash",
            "prompt": "执行危险删除命令",
            "callId": "rm -rf /",
            "expiresAtMs": 55555
        });
        let req = permission_request_from_pending(&val, "fallback-sess");
        assert_eq!(req.request_id, "req-999");
        assert_eq!(req.session_id, "sess-abc");
        assert_eq!(req.tool_name, "bash");
        assert_eq!(req.description, "执行危险删除命令");
        assert_eq!(req.command_or_path, Some("rm -rf /".to_string()));
        assert_eq!(req.timestamp_ms, 55555);
    }
}
