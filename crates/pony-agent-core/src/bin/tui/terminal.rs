//! 终端生命周期：信号处理、raw mode 捕获/恢复、重入流程、恢复提示（原 `tui.rs` 内联实现，机械外移）。
//! 不变量：任何退出路径都必须能走 [`restore_terminal_modes`]，禁止在别处直接 `disable_raw_mode`。

use super::*;

// T2c：SIGINT/SIGTERM 信号到达时不直接杀死进程，而是置位该标志；
// 主循环每个 tick（≤16ms）检测后正常 break，走统一的终端恢复保证器。
pub static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);

/// 终端重入请求：进程被 `fg` 唤回（SIGCONT）或终端被外部重置时置位。
///
/// 置位方是信号处理器 [`handle_sigcont_signal`]；消费方是主循环每 tick 的
/// [`take_refresh_request`]，取走后按 [`terminal_reentry_plan`] 重建 raw mode /
/// 备用屏 / 鼠标 / bracketed paste / kitty 键盘增强协议。
pub static REFRESH_REQUESTED: AtomicBool = AtomicBool::new(false);

/// 置位「终端需要重建」请求。
pub fn request_terminal_refresh() {
    REFRESH_REQUESTED.store(true, Ordering::SeqCst);
}

/// 一次性取走「终端需要重建」请求：`true` 表示自上次取走后被置位过。
///
/// 必须是单次 `swap` 而不是「先 load 再 store」：两次读之间可能再来一个信号，
/// 拆开会把那一次请求吞掉（多次信号合并成一次重建可接受，丢请求不可接受）。
pub fn take_refresh_request() -> bool {
    REFRESH_REQUESTED.swap(false, Ordering::SeqCst)
}

/// ADR 0075：最近一次终端还原是否**未能完整送达**输入侧/屏幕侧还原串（幂等标记）。
/// 退出提示据此决定是否附加一条可复制执行的手工恢复命令（见 `resume_hint_block`）。
pub static TERMINAL_RESTORE_INCOMPLETE: AtomicBool = AtomicBool::new(false);

/// 终端**输入侧**协议还原串（收到后终端不再把按键编码成转义序列）。
///
/// 残留这些模式时的现象：退出 TUI 后敲任何键都变成 `ESC[117u` / `ESC[9;5u` 这类
/// kitty 键盘协议编码，被 shell 当正文回显，终端看起来"输入全是乱码"。
/// - `<1u`×3 / `<u`：弹掉 kitty 键盘增强协议栈（crossterm 的弹栈写作 `<1u`，部分终端只认 `<u`）；
///   重复三次兜底「代理层吞掉或错配一次 pop」的情形（ADR 0075）
/// - `=0u`：与栈无关地显式把键盘协议标志置零（唯一绝对还原，**必须排在弹栈之后**：
///   弹栈会把栈里保存的旧标志写回终端，顺序反了等于没清）
/// - `>4;0m`：关 xterm modifyOtherKeys
/// - `?1000l` `?1002l` `?1003l` `?1006l` `?1015l`：关各级鼠标追踪（含 any-motion 与 SGR）
/// - `?2004l`：关 bracketed paste
pub const TERMINAL_INPUT_MODES_RESET: &[u8] =
    b"\x1b[<1u\x1b[<1u\x1b[<1u\x1b[<u\x1b[=0u\x1b[>4;0m\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?1015l\x1b[?2004l";

/// 终端**输出侧**还原串：退出备用屏 + 显示光标。
pub const TERMINAL_SCREEN_RESET: &[u8] = b"\x1b[?1049l\x1b[?25h";

/// 进入 raw mode 之前快照 termios 并预打开 `/dev/tty`。
///
/// 还原通道必须在"终端可能已经坏掉"之前就绪，否则信号/panic 路径无通道可用。
#[cfg(unix)]
pub fn capture_terminal_state() {
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
pub fn capture_terminal_state() {}

/// 信号上下文里的最佳努力还原：只调用 async-signal-safe 的 `write(2)` / `tcsetattr(3)`。
///
/// 唯一调用点是 [`handle_sigtstp_signal`]：Ctrl+Z 挂起前必须先把终端交还 shell，
/// 否则进程停住时终端留在 raw + 备用屏 + kitty 协议态（恢复执行前 shell 全乱）。
/// 正常退出/panic 路径走 [`restore_terminal_modes`]（可分配、可报错），不复用本函数。
#[cfg(unix)]
pub fn restore_terminal_for_signal() {
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
pub fn write_signal_notice(sig: libc::c_int) {
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

/// 安装 SIGTSTP 接管。
///
/// `signal(2)` 属 async-signal-safe，故 SIGCONT 处理器内也可安全调用（用于重新武装）。
#[cfg(unix)]
pub fn install_suspend_signal_handler() {
    unsafe {
        let suspend_handler = handle_sigtstp_signal as *const () as libc::sighandler_t;
        libc::signal(libc::SIGTSTP, suspend_handler);
    }
}

/// `libc` 是 `[target.'cfg(unix)'.dependencies]` 依赖，Windows 目标不存在该 crate，
/// 因此整套信号接管必须 cfg(unix) 门控（否则 Windows 编译报 E0433）。
#[cfg(unix)]
pub fn install_shutdown_signal_handlers() {
    unsafe {
        // Rust 禁止把函数项直接 cast 成整数（"direct cast of function item into an integer"），
        // 必须先经 `*const ()` 中转再转成 sighandler_t。
        let shutdown_handler = handle_shutdown_signal as *const () as libc::sighandler_t;
        libc::signal(libc::SIGINT, shutdown_handler);
        libc::signal(libc::SIGTERM, shutdown_handler);
        // pty 会话被关闭（如 `script` 收到 SIGINT 后退出）时，TUI 进程会收到 SIGHUP；
        // 同样走优雅退出路径以执行终端恢复，而不是默认终止并留下 alternate screen。
        libc::signal(libc::SIGHUP, shutdown_handler);

        let cont_handler = handle_sigcont_signal as *const () as libc::sighandler_t;
        libc::signal(libc::SIGCONT, cont_handler);

        // Ctrl+Z：接管 SIGTSTP 以便挂起前还原终端（默认处置会把终端留在 raw + 备用屏）。
        install_suspend_signal_handler();
    }
}

/// 非 unix（Windows）无 POSIX 信号语义：不接管信号，保持控制台默认行为。
#[cfg(not(unix))]
pub fn install_shutdown_signal_handlers() {}

/// 把还原串写到终端：unix 优先直写 `/dev/tty`（stdout 被重定向 / 返回 EIO 时仍有效），
/// 失败再退回 stdout。
#[cfg(unix)]
pub fn write_restore_bytes(bytes: &[u8]) -> io::Result<()> {
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
pub fn write_restore_bytes(bytes: &[u8]) -> io::Result<()> {
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

/// ADR 0075 C9：还原不完整时追加的手工恢复提示（单行、可直接复制执行）。
///
/// 首选 `reset`（同时复位 raw mode 与终端私有模式）；应急串用 POSIX `printf`，而不是
/// `$'...'`（后者是 bash/zsh 扩展，dash/sh 不认）；显式置零 `ESC[=0u` 与栈无关。
pub fn terminal_recovery_hint_line() -> String {
    "终端按键异常时可执行 `reset` 复位；应急：printf '\\033[<1u\\033[=0u\\033[>4;0m'".to_string()
}

/// ADR 0075 C9：续接提示块 —— 续接行恒在，**仅当**还原不完整时才追加手工恢复命令。
pub fn resume_hint_block(session_id: &str, restore_incomplete: bool) -> String {
    let mut block = resume_hint_line(session_id);
    if restore_incomplete {
        block.push('\n');
        block.push_str(&terminal_recovery_hint_line());
    }
    block
}

/// ADR 0075 C10：读取「最近一次还原是否不完整」（幂等，默认 false）。
pub fn terminal_restore_incomplete() -> bool {
    TERMINAL_RESTORE_INCOMPLETE.load(Ordering::SeqCst)
}

/// 把续接提示写到真实终端（stdout 被重定向时仍能送达）。
///
/// 调用时机：**终端还原之后**。提前写会落进备用屏，用户退出后根本看不到。
/// ADR 0075：还原不完整时在续接行后追加手工恢复命令，给出可自行执行的自救路径。
pub fn print_resume_hint(session_id: &str) {
    let block = resume_hint_block(session_id, terminal_restore_incomplete());
    let line = format!("\n{block}\n");
    let _ = io::stdout().flush();
    if let Err(error) = write_restore_bytes(line.as_bytes()) {
        eprintln!("[pony-tui] 退出提示写入失败：{error}");
    }
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
    let input_ok = match write_restore_bytes(TERMINAL_INPUT_MODES_RESET) {
        Ok(()) => true,
        Err(error) => {
            eprintln!("[pony-tui] 终端还原（输入模式）失败：{error}");
            false
        }
    };
    if let Err(error) = disable_raw_mode() {
        eprintln!("[pony-tui] disable_raw_mode 失败：{error}");
    }
    let screen_ok = match write_restore_bytes(TERMINAL_SCREEN_RESET) {
        Ok(()) => true,
        Err(error) => {
            eprintln!("[pony-tui] 终端还原（备用屏/光标）失败：{error}");
            false
        }
    };
    // ADR 0075：输入侧与屏幕侧还原串都送达才算完整；否则退出提示附手工恢复命令。
    TERMINAL_RESTORE_INCOMPLETE.store(!(input_ok && screen_ok), Ordering::SeqCst);
}

/// Windows 走 crossterm 命令（老 conhost 下 ANSI 直写不可靠，需走其 WinAPI 分支）。
#[cfg(not(unix))]
pub fn restore_terminal_modes() {
    // Windows 分支逐条 crossterm 命令无可靠失败回传，统一标记「已完整还原」（不附恢复命令）。
    TERMINAL_RESTORE_INCOMPLETE.store(false, Ordering::SeqCst);
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

/// 主线程上下文里的挂起：完整还原终端后 `raise(SIGSTOP)`。
///
/// 为什么必须由按键路径主动触发：raw mode 由 `cfmakeraw` 建立，它清掉了 `ISIG`，
/// 于是 Ctrl+Z **不会**产生 SIGTSTP 信号，而是作为 `Char('z')+CONTROL` 从 crossterm
/// 送到事件循环。[`handle_sigtstp_signal`] 只覆盖「外部 `kill -TSTP` / 未进 raw 的窗口」；
/// 用户真正按 Ctrl+Z 的路径是这里。
///
/// 这里用 [`restore_terminal_modes`] 而非信号版还原：主线程上下文可 flush、可报错、
/// 能清 crossterm 的 raw mode 缓存（否则重入时 `enable_raw_mode` 会静默 no-op）。
/// 恢复执行由 SIGCONT → `REFRESH_REQUESTED` → [`terminal_reentry_plan`] 完成。
#[cfg(unix)]
pub fn suspend_tui() {
    restore_terminal_modes();
    unsafe {
        libc::raise(libc::SIGSTOP);
    }
}

/// 非 unix（Windows）无 POSIX 作业控制：Ctrl+Z 不挂起（保持原行为，按键照常插入）。
#[cfg(not(unix))]
pub fn suspend_tui() {}

/// 终端重入（SIGCONT 唤醒 / Ctrl+Z 后 `fg`）必须按序重建的终端能力。
///
/// 挂起时 [`restore_terminal_for_signal`] 拆掉了 raw mode、备用屏与全部输入侧协议；
/// 恢复执行时若只 `enable_raw_mode` 就接着画帧，画面会落进用户 shell 的主屏滚屏区。
/// 因此把「装回哪些能力、什么顺序」表达成显式清单：顺序本身是契约（与启动初始化同序），
/// 且可在无 TTY 环境直接断言，不必伪造终端。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalReentryStep {
    /// 重进 raw mode（挂起时 `tcsetattr` 已回原 termios）。
    RawMode,
    /// 重建 kitty 键盘增强协议栈（还原串弹栈三次后栈已空，必须重新 push）。
    KeyboardEnhancement,
    /// 重进备用屏（挂起时已发 `ESC[?1049l`）。
    AlternateScreen,
    /// 重新开启鼠标捕获（仅当用户未用 F2 主动关闭）。
    MouseCapture,
    /// 重新开启 bracketed paste。
    BracketedPaste,
    /// 清空物理终端 + 失效渲染缓存，强制整屏重绘（备用屏内容已随挂起丢失）。
    ClearAndRedraw,
}

/// 终端重入计划。顺序与启动初始化一致，`ClearAndRedraw` 恒在末尾。
///
/// `mouse_capture=false`（用户按 F2 关过）时跳过鼠标重建：恢复执行不得悄悄改回用户选择。
pub fn terminal_reentry_plan(mouse_capture: bool) -> Vec<TerminalReentryStep> {
    let mut plan = vec![
        TerminalReentryStep::RawMode,
        TerminalReentryStep::KeyboardEnhancement,
        TerminalReentryStep::AlternateScreen,
    ];
    if mouse_capture {
        plan.push(TerminalReentryStep::MouseCapture);
    }
    plan.push(TerminalReentryStep::BracketedPaste);
    plan.push(TerminalReentryStep::ClearAndRedraw);
    plan
}

/// 重入时**强制**重进 raw mode。
///
/// 为什么不能直接 `enable_raw_mode()`：crossterm 在「它自认为已处于 raw mode」时
/// 直接返回 `Ok(())`（`TERMINAL_MODE_PRIOR_RAW_MODE` 有值即早退）。而 SIGTSTP 处理器
/// 是用 `tcsetattr` 直接把 termios 改回原值的（信号上下文里不能用 crossterm 的
/// 带锁实现），于是 crossterm 的缓存与真实状态分叉：`fg` 之后单发 `enable_raw_mode`
/// 会静默 no-op，终端停在 cooked mode 而 TUI 继续画帧 —— 按键全被 tty 回显成正文。
///
/// 故先 `disable_raw_mode()` 让 crossterm 清掉缓存（对已被处理器还原的 termios 是幂等的），
/// 再真正 `enable_raw_mode()`。
pub fn reenter_raw_mode() -> io::Result<()> {
    let _ = disable_raw_mode();
    enable_raw_mode()
}

/// 按 [`terminal_reentry_plan`] 逐项重建终端。
///
/// 键盘增强协议 push 失败不返回错误：部分终端不支持，降级为传统键码是既有约定
/// （与启动路径同口径），不应因此中断整个重建。
pub fn apply_terminal_reentry<B>(
    terminal: &mut Terminal<B>,
    steps: &[TerminalReentryStep],
) -> io::Result<()>
where
    B: ratatui::backend::Backend + io::Write,
{
    for step in steps {
        match step {
            TerminalReentryStep::RawMode => reenter_raw_mode()?,
            TerminalReentryStep::KeyboardEnhancement => {
                if let Some(flags) = keyboard_enhancement_push_flags() {
                    let _ = execute!(terminal.backend_mut(), PushKeyboardEnhancementFlags(flags));
                }
            }
            TerminalReentryStep::AlternateScreen => {
                execute!(terminal.backend_mut(), EnterAlternateScreen)?;
            }
            TerminalReentryStep::MouseCapture => {
                execute!(terminal.backend_mut(), EnableMouseCapture)?;
            }
            TerminalReentryStep::BracketedPaste => {
                execute!(terminal.backend_mut(), EnableBracketedPaste)?;
            }
            TerminalReentryStep::ClearAndRedraw => terminal.clear()?,
        }
    }
    Ok(())
}

/// 契约 F-D 加固：panic 也先还原终端。
///
/// `TerminalGuard` 的 Drop 覆盖不到 abort / hook 侧 abort 的路径；panic hook 在 unwind 最外层
/// 之前运行，是"进程要死了也要把终端交还"的最后一道闸。与 guard 的还原互为幂等。
pub fn install_panic_restore_hook() {
    // 终端属主是主线程：只有主线程 panic 才代表「这个进程要带着终端一起死了」。
    // 轮次线程 / 输入线程等 worker 的 panic 由 `catch_unwind` 兜底，TUI 仍在运行；
    // 若此处无条件还原，会把 raw mode / 备用屏 / kitty 键盘协议栈提前拆掉，
    // 界面还活着但按键已被 tty 回显成 `^[[13u`、`^[[99;5u` 这类字面垃圾
    // （herdr/tmux 下 `\x1b[<1u` 弹栈还可能不被识别，协议残留更久）。
    let terminal_owner = std::thread::current().id();
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if std::thread::current().id() == terminal_owner {
            restore_terminal_modes();
        }
        previous(info);
    }));
}
