//! 批1（task-5）验收测试：TUI 窄终端崩溃 / 命令解析 / 粘贴归一与粘贴不提交。
//!
//! 沿用 `tests/tui_render_layout.rs` 已确立的 bin-as-module + `TestBackend` 渲染 harness：
//! `src/bin/tui.rs` 是 `[[bin]]` 目标（非 lib target），只能用 `#[path]` 把**同一份真实源码**
//! 引入测试 crate 才能在编译期断言其行为；这比读源码文本的黑盒断言更强（格式化/别名改写下
//! 黑盒断言会假绿）。
//!
//! 用例与契约映射：
//! - N1 窄终端不 panic：`set_input("/")` 后 24x10 / 28x12 / 33x20 三档渲染均不得 panic。
//!   基线缺陷：斜杠菜单宽度 `need_w.clamp(28, content.width.min(64))`，而 `content.width = width - 6`
//!   （GUTTER*2），窄终端下 `min > max` ⇒ `Ord::clamp` panic。
//! - N2 命令解析矩阵：`parse_command_input`（exec-rust 落地后本文件即锁死其口径）。
//! - N3 粘贴归一：`normalize_paste`。
//! - N4 粘贴不提交：`TuiState::insert_paste`。
//!
//! 铁律：每条用例都是真实断言（无 `#[ignore]`、无 skip、无「只运行不报错」）；本文件不修改任何
//! 实现代码，实现侧不得反向修改本文件。

#[path = "../src/bin/tui.rs"]
#[allow(dead_code, unused_imports, unused_variables, unused_mut)]
mod tui;

use ratatui::backend::TestBackend;
use ratatui::Terminal;

/// 渲染一帧并取回最终缓冲区（与 `tests/tui_render_layout.rs::screen` 同源口径）。
fn screen(state: &mut tui::TuiState, width: u16, height: u16) -> ratatui::buffer::Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("create test backend");
    terminal
        .draw(|frame| tui::ui(frame, state))
        .expect("draw frame");
    terminal.backend().buffer().clone()
}

fn all_text(buf: &ratatui::buffer::Buffer) -> String {
    (0..buf.area.height)
        .map(|y| {
            (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// N1 共用断言：渲染成功（未 panic）、缓冲区尺寸如请求、且帧内确实画出了斜杠输入。
///
/// 最后一条是关键：仅「不 panic」不足以证明修复有效（例如实现可以把斜杠菜单整个跳过而不画），
/// 因此额外要求帧内出现用户已键入的 `/` 字符。
fn assert_narrow_render_survives(width: u16, height: u16) {
    let mut state = tui::TuiState::default();
    state.set_input("/");

    let buf = screen(&mut state, width, height);

    assert_eq!(buf.area.width, width, "N1: 缓冲区宽度必须等于请求宽度");
    assert_eq!(buf.area.height, height, "N1: 缓冲区高度必须等于请求高度");
    let text = all_text(&buf);
    assert!(
        text.contains('/'),
        "N1: {width}x{height} 帧内必须画出用户键入的 `/`（证明渲染真的发生了，而不是空帧）"
    );
    // 斜杠菜单在极窄终端下可以不展示完整命令表，但帧内不得出现「半截布局」式的空白失控：
    // 至少应有一行非空内容（composer 行）。
    assert!(
        text.lines().any(|line| !line.trim().is_empty()),
        "N1: {width}x{height} 帧内必须至少有一行可见内容"
    );
}

// ── N1：窄终端斜杠菜单不得 panic ────────────────────────────────────────────────────

#[test]
fn n1a_narrow_terminal_24x10_renders_without_panic() {
    assert_narrow_render_survives(24, 10);
}

#[test]
fn n1b_narrow_terminal_28x12_renders_without_panic() {
    assert_narrow_render_survives(28, 12);
}

#[test]
fn n1c_narrow_terminal_33x20_renders_without_panic() {
    assert_narrow_render_survives(33, 20);
}

// ── N2：命令解析矩阵（`tui::parse_command_input`） ───────────────────────────────────
//
// 口径（Lead 冻结）：只有「整行就是一个命令词」才算命中命令；带真实参数/路径/前缀相同的词
// 一律返回 None（继续作为普通消息发送），避免 `/clear 这个需求怎么拆` 这类用户输入被静默吞掉。

#[test]
fn n2a_exact_and_case_insensitive_commands_are_recognized() {
    assert_eq!(tui::parse_command_input("/clear"), Some("/clear"));
    assert_eq!(tui::parse_command_input("/CLEAR"), Some("/clear"));
    assert_eq!(tui::parse_command_input("  /plan  "), Some("/plan"));
    // 只允许首尾空白：命令后仅有空白仍属命令。
    assert_eq!(tui::parse_command_input("/plan   "), Some("/plan"));
    assert_eq!(tui::parse_command_input("\t/compact\n"), Some("/compact"));
}

#[test]
fn n2b_command_like_messages_are_not_treated_as_commands() {
    // 命令携带真实参数 ⇒ 普通消息（不得静默吞掉用户正文）。
    assert_eq!(tui::parse_command_input("/clear 这个需求怎么拆"), None);
    assert_eq!(tui::parse_command_input("/plan 帮我拆解X"), None);
    // 双斜杠 / 前缀相同的长词 / 空输入都不是命令。
    assert_eq!(tui::parse_command_input("//clear"), None);
    assert_eq!(tui::parse_command_input("/clearx"), None);
    assert_eq!(tui::parse_command_input(""), None);
    assert_eq!(tui::parse_command_input("   "), None);
    // 非斜杠开头的普通消息。
    assert_eq!(tui::parse_command_input("hello"), None);
}

// ── N3：粘贴文本归一（`tui::normalize_paste`） ──────────────────────────────────────

#[test]
fn n3a_crlf_and_cr_are_normalized_to_lf() {
    assert_eq!(tui::normalize_paste("a\r\nb"), "a\nb");
    assert_eq!(tui::normalize_paste("a\rb"), "a\nb");
    assert_eq!(tui::normalize_paste("a\r\n\r\nb"), "a\n\nb");
}

#[test]
fn n3b_plain_lf_and_empty_input_are_preserved() {
    // 已规范的 LF 必须原样保留（不得二次改写）。
    assert_eq!(tui::normalize_paste("a\nb"), "a\nb");
    assert_eq!(tui::normalize_paste(""), "");
    assert_eq!(tui::normalize_paste("\n"), "\n");
}

#[test]
fn n3c_cjk_multiline_paste_is_not_mangled() {
    assert_eq!(tui::normalize_paste("第一行\n第二行"), "第一行\n第二行");
    assert_eq!(tui::normalize_paste("第一行\r\n第二行"), "第一行\n第二行");
    assert_eq!(
        tui::normalize_paste("缩进：\r\n\t代码块\n结尾"),
        "缩进：\n\t代码块\n结尾"
    );
}

// ── N4：粘贴不得触发提交（`TuiState::insert_paste`） ────────────────────────────────

#[test]
fn n4_paste_inserts_multiline_text_without_submitting() {
    let mut state = tui::TuiState::default();
    // 前置事实（真实断言）：新会话默认无消息、未在流式输出。
    assert_eq!(state.messages.len(), 0, "N4 前置条件：默认会话不应有消息");
    assert!(!state.is_streaming, "N4 前置条件：默认不在流式输出");

    let message_count_before = state.messages.len();
    state.insert_paste("第一行\n第二行");

    // ① 两行都进入输入缓冲，且保留换行（多行粘贴不得被压成一行或被逐行提交）。
    assert!(
        state.input_buffer.contains("第一行"),
        "N4: 粘贴内容必须进入 input_buffer，实际 = {:?}",
        state.input_buffer
    );
    assert!(
        state.input_buffer.contains("第二行"),
        "N4: 第二行必须同样进入 input_buffer，实际 = {:?}",
        state.input_buffer
    );
    assert!(
        state.input_buffer.contains('\n'),
        "N4: 多行粘贴必须保留换行符，实际 = {:?}",
        state.input_buffer
    );

    // ② 粘贴绝不产生提交副作用：不进入流式、不新增消息。
    assert!(!state.is_streaming, "N4: 粘贴后不得进入流式发送状态");
    assert_eq!(
        state.messages.len(),
        message_count_before,
        "N4: 粘贴不得新增任何消息（粘贴不是提交）"
    );

    // ③ 光标落在粘贴内容末尾。
    assert_eq!(
        state.input_cursor,
        state.input_buffer.chars().count(),
        "N4: 粘贴后光标必须落在输入末尾"
    );
}
