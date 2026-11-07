# TUI 用户消息上下各留 1 行页面底色间距

Status: implemented

## Context

`pony` TUI 的用户消息与相邻内容贴边：用户气泡自带 1 行 SURFACE 内衬（气泡材质），
而 `MessageRole::User` 分支本身不产生任何空白行，轮间距全靠对侧角色（Assistant 的
`push_gap(2)`、Thinking 块尾呼吸）提供。对侧没有间距时（工具行、状态消息紧接用户消息）
就完全没有视觉分隔，而 SURFACE(`#1B1916`) 与 CANVAS(`#12100E`) 对比极弱，
读者分不清消息边界。

完整决策（背景、候选方案比较、影响与残余风险）见 ADR 0088：
[`docs/decisions/0088-tui-user-message-vertical-gap.md`](../../../docs/decisions/0088-tui-user-message-vertical-gap.md)。

## Decision

1. `message_lines_with_ctx` 的 `MessageRole::User` 分支：`has_preceding_lines` 为真时先 push 1 行
   `blank()`（页面底色）；会话首条用户消息不加，避免顶部凭空留白。
2. 同分支尾部无条件 push 1 行 `blank()`；尾部空行由 `ui()` 的 `scroll_total` 尾部连续空白剔除
   兜底，不产生悬空留白。
3. 删除该分支块首的 `push_gap(2)`（独立 `Vec` 上恒为 no-op 的死代码）。
4. `push_user_block` 的气泡内衬与正文排版零改动（1 行上内衬 + 正文 + 1 行下内衬）。

## Alternatives considered

- **气泡内衬 1 行 → 2 行**：内衬是气泡材质，加深读成「更大的气泡」而非轮间呼吸，且加重 SURFACE 权重。
- **复用 `push_gap(2)` 轮间距**：块首在独立 `Vec` 上恒 no-op；按空白补齐的计数语义会吞掉/叠加对侧既有呼吸。
- **在 `ui()` 逐帧循环里按 role 注入**：渲染缓存按消息切块，缓存外注入会让缓存行数与真实高度分裂。
- **包一层上下 padding 的容器块**：需新块模型与缓存键，可见几何无变化，超出最小闭包。

## Consequences

- 产品代码改动：`crates/pony-agent-core/src/bin/tui.rs`（`MessageRole::User` 分支）。
- 既有断言改判：`tests/tui_thinking_bottom_gap.rs` A12（User→Thinking 2→3、Thinking→User 3→4）。
- 新增验收：`tests/tui_user_block_gap.rs`（agent 正文 / 状态消息 / 工具行 / 首条无顶空行）。
- 回归套件（改判后全绿，唯一失败项为并行工作线的 splash logo 资产尺寸断言，与本次无关）：
  `tui_thinking_bottom_gap`、`tui_render_layout`、`tui_batch3_regression`、`tui_six_fixes_acceptance`、
  `tui_batch6_virtualization`、`tui_acceptance`、`tui_process_fold_acceptance`、`tui_user_block_gap`。
- 残余风险：用户消息渲染分支仍在单文件 `tui.rs` 内；若后续 `tui/` 目录重构把消息渲染域外移，
  注释与 ADR 引用必须随迁。
