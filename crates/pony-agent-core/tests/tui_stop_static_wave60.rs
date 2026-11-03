//! wave-60 C9 **补充证据**（结构级门禁，红相先写）：TUI 按键处理不得再本地伪造终态。
//!
//! 契约真值源：`.dev-team/contracts/wave-60-interruptible-stop.md` §C9：
//! 「**删除现有"当场 `is_streaming=false` + 立即打印已中断"的双写路径**；改为由事件驱动」
//! 「未收到终态事件时不得打印"已中断本轮生成。"（防止假终态）」。
//!
//! 说明（诚实边界）：TUI 的按键处理位于 `main()` 事件循环内，无公开运行时缝，
//! 因此**行为级**判据见 `tests/tui_stop_wave60.rs`（T5 交付 `apply_stop_*` 后转绿）；
//! 本文件提供结构级补充证据：源码里不得残留伪造终态的文案与"正在停止…/已停止"缺失。
//! 本文件不修改 `src/bin/tui.rs`，只读源码文本。

/// 被 C9 明令禁止的本地伪造终态文案。
const FAKE_TERMINAL_MESSAGE: &str = "已中断本轮生成。";

const TUI_SOURCE: &str = include_str!("../src/bin/tui.rs");

/// C9（结构级）：按键处理不得再打印本地伪造的终态文案。
#[test]
fn c9_source_has_no_locally_fabricated_terminal_message() {
    // Arrange
    let occurrences = TUI_SOURCE.matches(FAKE_TERMINAL_MESSAGE).count();

    // Act / Assert
    assert_eq!(
        occurrences, 0,
        "C9：`src/bin/tui.rs` 不得再出现「{FAKE_TERMINAL_MESSAGE}」（本地伪造终态）；\
         终态呈现必须由 turn:cancelled 事件驱动，状态行改为「已停止」"
    );
}

/// C9（结构级）：停止两态的呈现文案必须存在于 TUI 源码（请求后 / 终态后）。
#[test]
fn c9_source_defines_stopping_and_stopped_labels() {
    // Arrange / Act
    let has_stopping = TUI_SOURCE.contains("正在停止");
    let has_stopped = TUI_SOURCE.contains("已停止");

    // Assert
    assert!(
        has_stopping,
        "C9：TUI 必须能呈现「正在停止…」（双击 Esc / Ctrl+C 发出停止请求后的状态行）"
    );
    assert!(
        has_stopped,
        "C9：TUI 必须能呈现「已停止」（收到 turn:cancelled 后的状态行）"
    );
}

/// C9（结构级）：TUI 的 sink 必须消费 `turn:stop_requested` 与 `turn:cancelled`。
#[test]
fn c9_source_handles_stop_requested_and_cancelled_events() {
    // Arrange / Act
    let handles_stop_requested = TUI_SOURCE.contains("turn:stop_requested");
    let handles_cancelled = TUI_SOURCE.contains("turn:cancelled");

    // Assert
    assert!(
        handles_stop_requested,
        "C9：TUI 事件桥必须消费 `turn:stop_requested`（进入「正在停止…」）"
    );
    assert!(
        handles_cancelled,
        "C9：TUI 事件桥必须消费 `turn:cancelled`（由事件驱动「已停止」终态）"
    );
}
