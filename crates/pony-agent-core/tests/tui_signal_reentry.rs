//! TUI 终端信号闭环的验收测试（Ctrl+Z 挂起 / `fg` 恢复）。
//!
//! 缺陷机理：SIGTSTP 走默认处置直接停进程，终端留在 raw + 备用屏 + kitty 键盘协议态，
//! 用户回到 shell 后每个按键都被编码成 `ESC[117u` 这类转义序列当正文回显；
//! 只有 `fg` 触发 SIGCONT 才修复，且旧实现只 `enable_raw_mode`，备用屏与输入协议不重建，
//! 后续帧直接画到主屏滚屏区。
//!
//! 沿用 `tests/tui_terminal_restore.rs` 的 bin-as-module 模式（`#[path = "../src/bin/tui.rs"]`），
//! 断言的是**真实实现**里的计划与一次性消费语义，不是复制粘贴的副本。
//! 铁律：每条用例都有真实断言；无 `#[ignore]`/skip；不修改被测实现。

#[path = "../src/bin/tui.rs"]
#[allow(dead_code, unused_imports, unused_variables, unused_mut)]
mod tui;

use std::sync::{Mutex, MutexGuard};
use tui::TerminalReentryStep;

/// R1/R2 操作同一个进程级 `AtomicBool`，而 cargo 默认并行跑同文件的用例：
/// 不串行化就会出现「对方刚置位、这边断言 false」的假失败。
static REFRESH_FLAG_LOCK: Mutex<()> = Mutex::new(());

fn lock_refresh_flag() -> MutexGuard<'static, ()> {
    REFRESH_FLAG_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

// ── R1：刷新请求是一次性消费（SIGCONT 置位 → 主循环取走 → 不重复重建） ─────────────

#[test]
fn r1_refresh_request_is_consumed_exactly_once() {
    let _guard = lock_refresh_flag();
    // 先清空：同进程内其它用例/启动路径可能残留置位。
    let _ = tui::take_refresh_request();
    assert!(
        !tui::take_refresh_request(),
        "R1: 无请求时取走必须返回 false（否则主循环每 tick 都会重建终端）"
    );

    tui::request_terminal_refresh();
    assert!(
        tui::take_refresh_request(),
        "R1: 置位后第一次取走必须返回 true"
    );
    assert!(
        !tui::take_refresh_request(),
        "R1: 取走后必须复位（否则会无限重建：清屏 + 重进备用屏每 tick 一次）"
    );
}

// ── R2：多次置位合并为一次重建，但绝不丢请求 ──────────────────────────────────────

#[test]
fn r2_repeated_requests_collapse_into_one_pending_flag() {
    let _guard = lock_refresh_flag();
    let _ = tui::take_refresh_request();
    tui::request_terminal_refresh();
    tui::request_terminal_refresh();
    tui::request_terminal_refresh();
    assert!(
        tui::take_refresh_request(),
        "R2: 多次信号必须至少留下一次可消费的请求（丢请求 = 终端永久错乱）"
    );
    assert!(
        !tui::take_refresh_request(),
        "R2: 合并后只应消费一次（多次重建会闪屏，无收益）"
    );
}

// ── R3：重入计划顺序与启动初始化同序，清屏恒在末尾 ────────────────────────────────

#[test]
fn r3_reentry_plan_rebuilds_capabilities_in_startup_order() {
    let plan = tui::terminal_reentry_plan(true);
    assert_eq!(
        plan,
        vec![
            TerminalReentryStep::RawMode,
            TerminalReentryStep::KeyboardEnhancement,
            TerminalReentryStep::AlternateScreen,
            TerminalReentryStep::MouseCapture,
            TerminalReentryStep::BracketedPaste,
            TerminalReentryStep::ClearAndRedraw,
        ],
        "R3: 重入顺序必须与启动初始化一致（raw → 键盘协议 → 备用屏 → 鼠标 → paste → 清屏重绘）"
    );
    assert_eq!(
        plan.last(),
        Some(&TerminalReentryStep::ClearAndRedraw),
        "R3: 清屏必须是最后一步（先清屏再重建协议会把还原串写到主屏）"
    );
    assert_eq!(
        plan.first(),
        Some(&TerminalReentryStep::RawMode),
        "R3: raw mode 必须最先恢复（否则 EnterAlternateScreen 的响应会落进 shell 回显）"
    );
}

// ── R4：用户用 F2 关过鼠标捕获时，重入不得悄悄改回 ────────────────────────────────

#[test]
fn r4_reentry_plan_respects_disabled_mouse_capture() {
    let plan = tui::terminal_reentry_plan(false);
    assert!(
        !plan.contains(&TerminalReentryStep::MouseCapture),
        "R4: 鼠标捕获已由用户关闭，重入不得擅自开启（否则终端原生拖选又被抢走）"
    );
    assert_eq!(
        plan,
        vec![
            TerminalReentryStep::RawMode,
            TerminalReentryStep::KeyboardEnhancement,
            TerminalReentryStep::AlternateScreen,
            TerminalReentryStep::BracketedPaste,
            TerminalReentryStep::ClearAndRedraw,
        ],
        "R4: 除鼠标外其余步骤与顺序必须保持不变"
    );
    // 反向对照：开着鼠标时确实会重建（防「一律不重建」的空实现混过）。
    assert!(
        tui::terminal_reentry_plan(true).contains(&TerminalReentryStep::MouseCapture),
        "R4: 鼠标捕获开启时必须重建，否则挂起恢复后拖选失效"
    );
}

// ── R6：Ctrl+Z 判定 —— raw mode 下 ISIG 被清，只能靠按键路径触发挂起 ───────────────

#[test]
fn r6_suspend_shortcut_matches_only_bare_ctrl_z() {
    use crossterm::event::{KeyCode, KeyModifiers};

    assert!(
        tui::is_suspend_shortcut(KeyCode::Char('z'), KeyModifiers::CONTROL),
        "R6: 裸 Ctrl+Z 必须命中（这是用户唯一的挂起入口）"
    );
    // 反向对照：任何多余修饰键都不得挂起，否则与将来的重做类快捷键抢键。
    for modifiers in [
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        KeyModifiers::CONTROL | KeyModifiers::ALT,
        KeyModifiers::SHIFT,
        KeyModifiers::NONE,
    ] {
        assert!(
            !tui::is_suspend_shortcut(KeyCode::Char('z'), modifiers),
            "R6: {modifiers:?}+z 不得触发挂起"
        );
    }
    // 带 Shift 时字符本身是 'Z'，也必须不命中（兜住终端直接上报大写的情形）。
    assert!(
        !tui::is_suspend_shortcut(
            KeyCode::Char('Z'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT
        ),
        "R6: Ctrl+Shift+Z（上报为 'Z'）不得触发挂起"
    );
    assert!(
        !tui::is_suspend_shortcut(KeyCode::Char('c'), KeyModifiers::CONTROL),
        "R6: Ctrl+C 不得被判成挂起（它有独立三态语义）"
    );
}

// ── R5：挂起还原串必须含「退出备用屏」——与重入计划互为逆操作 ──────────────────────

#[test]
fn r5_suspend_restore_bytes_are_the_inverse_of_reentry() {
    // 挂起侧（信号上下文）用固定的两段还原串；重入侧必须把对应能力全部装回，
    // 否则 `fg` 之后界面会缺一块（最典型：备用屏没重进 ⇒ 画面写进 shell 滚屏区）。
    assert!(
        tui::TERMINAL_SCREEN_RESET
            .windows(b"\x1b[?1049l".len())
            .any(|w| w == b"\x1b[?1049l"),
        "R5: 挂起还原串必须退出备用屏（ESC[?1049l），否则 shell 仍在备用屏里"
    );
    assert!(
        tui::TERMINAL_INPUT_MODES_RESET
            .windows(b"\x1b[?2004l".len())
            .any(|w| w == b"\x1b[?2004l"),
        "R5: 挂起还原串必须关 bracketed paste，重入计划才需要重新开启"
    );
    let plan = tui::terminal_reentry_plan(true);
    for step in [
        TerminalReentryStep::AlternateScreen,
        TerminalReentryStep::BracketedPaste,
        TerminalReentryStep::KeyboardEnhancement,
    ] {
        assert!(
            plan.contains(&step),
            "R5: 挂起拆掉的 {step:?} 必须在重入计划里装回（否则能力单向丢失）"
        );
    }
}
