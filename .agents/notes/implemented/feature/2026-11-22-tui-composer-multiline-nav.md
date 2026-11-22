# Agent Note: TUI 输入框多行编辑——↑/↓ 跨行导航 + 8 行显示上限

Status: implemented

## Problem

1. **输入框过早进入滚动窗口**：`bin/tui.rs::ui()` 的输入区高度是
   `(visual_lines + 1).clamp(2, 6)`——正文 5 行起就开始截窗。24~30 行的常见终端里，
   用户写一段多行草稿时看不全自己刚敲的那几行，只能靠 `↑`/`↓` 之外的键反复确认。
2. **↑/↓ 被消息区滚动占满**：Normal 模式下 `state.input_buffer` 非空时，
   裸 `↑`/`↓` 直接落到 `scroll_up(2)` / `scroll_down(2, max_scroll_offset)`，
   多行草稿根本无法用方向键在行间移动光标（只有 `Ctrl+A/Ctrl+E` 在**逻辑行**内跳首尾）。
3. **边界语义缺失**：没有「首行再按 ↑ = 文字头部 / 尾行再按 ↓ = 文字末尾」的收敛动作，
   用户在边界处连按方向键只会滚动消息区，误以为输入框失焦。

## Decision

1. **8 行上限单源**：新增 `MAX_COMPOSER_ROWS = 8`（含底部 1 行呼吸行）与纯函数
   `composer_input_height(visual_lines)`（`(lines+1).clamp(2, 8)`）。`ui()` 的
   `target_input_h` 与 `clear_input()` 的收缩动画共用它，替换两处 `clamp(2, 6)`；
   正文超出可视行时窗口仍跟随光标（`render_input` 的 `start_line` 逻辑不变）。
2. **键位判据唯一**：`bin/tui/input.rs::composer_vertical_navigation(key)` 只认**裸** `↑`/`↓`
   并返回 `Option<ComposerVerticalDirection>`（带 Shift/Alt/Ctrl 一律 `None`：Ctrl+↑/←/→ 仍是
   子会话切换，Shift+↑ 仍是终端原生选择）。方向用枚举而不是 `bool`，既消掉
   `clippy::fn_params_excessive_bools` 的新增计数，也让「判据 → 光标目标」共用一个类型。
   命令补全弹窗（`matching_commands`）与历史回填（`composer_history_recall`，空输入框 + 裸 ↑）
   在调用点**先于**本判据生效，二者语义不变。
3. **光标目标纯函数**：`vertical_cursor_target(text, cursor, width, direction, desired_col)` 与
   `render_input` 共用同一个 `layout_input` 折行结果与 active_line 判据（边界取靠前那一行，
   避免「光标画在上一行行尾、↑ 却跳到下一行」）；列按**显示宽度**（CJK = 2 列）映射，
   相邻行更短时落到行尾；`desired_col` 让连续上下移动不横向漂移。
   首行再 ↑ ⇒ 索引 0（文字头部）；尾行再 ↓ ⇒ 字符数（文字末尾）。
4. **期望列的生命周期**：`TuiState::input_desired_col` 只在垂直移动里读写；
   任何编辑（`insert_char` / `insert_paste` / `backspace` / `delete_forward`）、
   水平移动（`←`/`→`）、行首行尾（`Ctrl+A/E`）、整体替换（`set_input` / `clear_input`）
   一律清空，避免按陈旧列跳。
5. **状态接线**：`TuiState::move_cursor_vertical(direction)` 用上一帧渲染回写的
   `last_input_rect.width`（= 提示符 2 列 + 右侧 1 列留白之外的文本区宽度）算列，
   保证「所见即所算」；Normal 模式非空输入框的裸 `↑`/`↓` 走它，空输入框保持
   历史回填 / 消息滚动，滚动仍可用 `PgUp`/`PgDn`/滚轮。
6. **帮助清单同源**：`bin/tui/shortcuts.rs` 的「输入框编辑」组新增
   `↑ / ↓` 条目（快捷键真源在该文件，避免帮助面板与实际键位说谎）。
7. **行为门**：`bin/tui/tests/composer_multiline_nav_tests.rs` 8 例——键位谓词矩阵、
   8 行上限夹取、首/尾行边界、软折行与 CJK 列对齐、期望列不漂移、编辑清空期望列、
   空输入框上下移动不越界、`TuiState` 宽度接线。

## Related

- 折行与渲染：`crates/pony-agent-core/src/bin/tui.rs::layout_input` / `render_input`
- 历史回填契约（本次保持不变）：`bin/tui/tests/composer_history_recall_tests.rs`
- 输入编辑快捷键真源：`bin/tui/input.rs::composer_control_action`、`bin/tui/shortcuts.rs`
- 窗口跟随与呼吸行口径：`ui()` 的 `target_input_h` / `breathing_rect`
- 可观测性归属：`docs/observability.yaml`（2026-11-22 归属登记，四维不变）

## Alternatives considered

- **保留 ↑/↓ 滚动消息，只用 Alt+↑/↓ 跨行**：与主流多行输入（Claude Code、各类编辑器）的直觉
  不一致，且 Alt+方向键在部分终端/tmux 配置里被吞或与窗口管理快捷键冲突。
- **按逻辑行（`\n`）而不是可视行移动**：软折行的长段文本里按一次 ↑ 会跳过整整一屏，
  用户直觉是「按屏幕上的行走」，与 `render_input` 的窗口跟随也对不上。
- **上限写成「8 行正文 + 1 行呼吸」（输入区 9 行）**：与「输入框最多显示 8 行」的视觉口径不符，
  且在 24 行终端里挤占消息区更多空间。
- **每帧把 `layout_input` 结果缓存进 `TuiState` 供按键复用**：多一份必须与文本/宽度同步失效的
  派生状态（忘记失效即错位）；按键时按同一纯函数重算，文本量级 ≤ 数十 KB，代价可忽略。
- **只在边界处（首/尾行）回退到滚动消息**：同一个键在不同光标位置产生两套语义（半个键位两种行为），
  比「↑/↓ 归输入框、滚动归 PgUp/PgDn/滚轮」更难预期，也更容易让用户误判输入框是否失焦。
- **把 `MAX_COMPOSER_ROWS` 做成可配置项**：终端高度与草稿长度的平衡点没有普适值，
  先给单一常量（8 行）；真出现「小终端被输入区吃掉半屏」的现场证据再谈配置面。
