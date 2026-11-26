//! Composer 按键动作与键盘增强决策（纯函数，从 `bin/tui/input.rs` 抽离）。
//!
//! 提供在无 TTY、无事件循环环境下的纯状态决策，避免集成测试为了断言按键语义
//! 而通过 `#[path = "...src/bin/tui.rs"]` 引入庞大的 TUI 运行时与内嵌测试。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, KeyboardEnhancementFlags};

/// wave-40 R1：Composer 中 Enter 键语义。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposerEnterAction {
    Submit,
    Newline,
}

/// 是否「带修饰键的 Enter」＝换行语义（裸 Enter 才是提交）。
///
/// 口径（B277 统一）：`SHIFT` / `ALT` / `CONTROL` **任一**即换行。Shift+Enter 是对外
/// 宣称键位；Alt+Enter 是传统终端兜底；Ctrl+Enter 在 kitty 协议下上报为
/// `CSI 13;5u`，若落到提交语义会把用户「想换行」的手势直接发出去，故一并归为换行。
/// 判定用 `intersects`（含 `Ctrl+Shift+Enter`），不看具体组合顺序。
pub fn is_newline_enter(key: &KeyEvent) -> bool {
    key.code == KeyCode::Enter
        && key
            .modifiers
            .intersects(KeyModifiers::SHIFT | KeyModifiers::ALT | KeyModifiers::CONTROL)
}

/// 判定按键在输入框中触发的动作（Submit / Newline）。
pub fn composer_enter_action(key: &KeyEvent) -> ComposerEnterAction {
    // 带修饰键 Enter = 换行；其余键（含裸 Enter）落到 Enter 分支的提交语义。
    if is_newline_enter(key) {
        ComposerEnterAction::Newline
    } else {
        ComposerEnterAction::Submit
    }
}

/// wave-40 R2：终端支持键盘增强时返回要 push 的标志。
#[expect(
    clippy::fn_params_excessive_bools,
    reason = "wave-40 R2 契约冻结签名：`supported: bool` 由冻结验收测试 tui_enter_key_wave40 直接调用"
)]
pub fn keyboard_enhancement_flags(supported: bool) -> Option<KeyboardEnhancementFlags> {
    // kitty 协议的 DISAMBIGUATE_ESCAPE_CODES：把 Shift+Enter 与裸 Enter 区分开。
    if supported {
        Some(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
    } else {
        None
    }
}
