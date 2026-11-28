//! 滚动几何与状态动效字形（B1795831358336 Phase 2）。
//!
//! 从 `tui.rs` 整块切出的**无状态纯函数**：滚动偏移的饱和换算（契约 F-H）与
//! 状态感知动效帧（契约 F-I）。不碰 `TuiState`、不做 IO，可在无 TTY 环境独立断言。
//!
//! 切出契约：函数名、签名、饱和语义与字形常量逐字保留（`pub(crate)` 供跨 `tui` 模块调用）。

use crate::tui::RuntimeMode;

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
pub(crate) fn executing_tool_frame(tick: usize) -> &'static str {
    EXECUTING_FRAMES[tick % EXECUTING_FRAMES.len()]
}
