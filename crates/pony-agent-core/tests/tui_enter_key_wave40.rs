// wave-40 L2-T：TUI Composer Shift+Enter 换行 —— 契约冻结验收（Test Agent 所有，Executor 只读）。
//
// 契约来源：.dev-team/contract-matrix-tui-enter-key.md §3 A1。
// 根因：默认未启用 kitty 键盘增强协议，终端把 Shift+Enter 编码为裸 `\r`，crossterm 解析为
// 无修饰 Enter → 走提交路径。R1/R2 把「Enter 语义」与「增强标志决策」抽成纯函数，便于单测。
//
// 红相目标：Protocol 在 `tui.rs:107-123` 落盘了 `composer_enter_action` /
// `keyboard_enhancement_flags` 空桩（`unimplemented!()`），本文件必须
// 「编译通过（Exit 0）+ 运行失败（panic / 断言失败）」。

#[path = "../src/bin/tui.rs"]
#[allow(dead_code, unused_imports, unused_variables, unused_mut)]
mod tui;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, KeyboardEnhancementFlags};

// ---------------------------------------------------------------------------
// A1-1：无修饰 Enter → Submit
// ---------------------------------------------------------------------------
#[test]
fn a1_plain_enter_submits() {
    // Arrange
    let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);

    // Act
    let action = tui::composer_enter_action(&key);

    // Assert
    assert_eq!(
        action,
        tui::ComposerEnterAction::Submit,
        "无修饰 Enter 必须提交（Enter 的默认语义）"
    );
}

// ---------------------------------------------------------------------------
// A1-2：Shift / Alt / Ctrl+Shift + Enter → Newline（`Shift+Enter` 是用户缺陷主场景）
// ---------------------------------------------------------------------------
#[test]
fn a1_modified_enter_inserts_newline() {
    // Arrange
    let cases: [(&str, KeyModifiers); 3] = [
        ("Shift+Enter", KeyModifiers::SHIFT),
        ("Alt+Enter", KeyModifiers::ALT),
        (
            "Ctrl+Shift+Enter",
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ),
    ];

    for (label, modifiers) in cases {
        // Act
        let action = tui::composer_enter_action(&KeyEvent::new(KeyCode::Enter, modifiers));

        // Assert
        assert_eq!(
            action,
            tui::ComposerEnterAction::Newline,
            "{label} 必须插入换行而不是提交（modifiers 含 SHIFT 或 ALT）"
        );
    }
}

// ---------------------------------------------------------------------------
// A1-3：非 Enter 键 → Submit（调用点只服务 Enter 分支，语义固定便于断言）
// ---------------------------------------------------------------------------
#[test]
fn a1_non_enter_key_submits() {
    // Arrange：Shift+Char('x') 是最容易误伤的“带修饰的普通字符”
    let key = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::SHIFT);

    // Act
    let action = tui::composer_enter_action(&key);

    // Assert
    assert_eq!(
        action,
        tui::ComposerEnterAction::Submit,
        "非 Enter 键不得被误判为换行（只在 Enter 分支调用，Submit 为契约固定值）"
    );
}

// ---------------------------------------------------------------------------
// A1-4：键盘增强标志映射 —— 支持才 push DISAMBIGUATE_ESCAPE_CODES，否则 None
// ---------------------------------------------------------------------------
#[test]
fn a1_keyboard_enhancement_flags_follow_support() {
    // Act
    let supported = tui::keyboard_enhancement_flags(true);
    let unsupported = tui::keyboard_enhancement_flags(false);

    // Assert
    assert_eq!(
        supported,
        Some(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES),
        "终端支持时必须 push DISAMBIGUATE_ESCAPE_CODES（kitty 协议下 Shift+Enter 才能与 Enter 区分）"
    );
    assert_eq!(
        unsupported, None,
        "终端不支持时不得 push 任何标志（保持当前行为，Alt+Enter 兜底）"
    );
}
