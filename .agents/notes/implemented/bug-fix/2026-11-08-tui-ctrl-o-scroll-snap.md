# Agent Note: TUI `Ctrl+O` 展开 Agent 过程后当帧贴底（阻尼滚动只服务增量追加）

Status: implemented

## Problem

历史较长时按 `Ctrl+O` 展开 Agent 过程，画面会**从顶部逐帧滚到底部**：用户看到一段
「播放出来的滚动动画」，而不是打开后就停在底部。

根因是滚动状态机把两种性质完全不同的变化混为一谈：

- **增量追加**（流式新行、状态行下移）：`max_scroll_offset` 缓慢增长，`auto_scroll`
  的阻尼跟随每帧最多推进 3 行，视觉上是连续推进行，属于设计意图；
- **整体重排**（`hide_process` / `thinking_collapsed` 翻转）：历史行数一次性跃增/缩减，
  `max_scroll_offset` 从 121 直接跳到 218，但 `scroll_offset` 仍是旧底部。
  `set_hide_process` 只让渲染缓存失效、不重置偏移，下一帧起阻尼以每帧 3 行追赶，
  97 行的差距需要 30+ 帧——即用户看到的「从头滚到底」。

## Decision

- 新增 `TuiState::snap_to_bottom_if_following()`（`crates/pony-agent-core/src/bin/tui.rs`）：
  仍在自动贴底跟踪（`auto_scroll == true`）时把 `scroll_offset` 置为**末尾哨兵**
  `u32::MAX`，当帧渲染再钳制到真实 `max_scroll_offset`（与 `/resume` 恢复同一写法，
  不依赖上一帧的陈旧最大值）。
- `set_hide_process` / `set_thinking_collapsed` 在 `invalidate_render_cache()` 之后调用它：
  过程可见性与思考折叠都是**整体重排**，一律当帧贴底，不再复用阻尼。
- `ui()` 的阻尼跟随逻辑**保持不动**：流式增量追加的平滑推进是既有契约（滚动条几何、
  `has_pending_animation` 动画持续刷新都依赖它）。
- 手动上翻（`auto_scroll == false`）时不动偏移：开关不得把用户的阅读位置强行拉回底部。

## Alternatives considered

- **在事件循环里给 `Ctrl+O` 分支加 `scroll_to_bottom()`**：只覆盖快捷键入口，
  `/settings` 面板切换、`apply_tui_preferences` 启动注入等路径仍会「滚到底」；
  且 `scroll_to_bottom()` 依赖上一帧的 `max_scroll_offset`，重排当帧还未计算。
- **把阻尼步长改成「差距大时一次性到顶」**：会让流式期真正的增量追加也失去平滑推进，
  用一处策略改动破坏既有动效契约，收益只在非增量场景。
- **整体重排时清零 `scroll_offset`**：方向相反——会先跳到顶部再追赶，正是本缺陷的更强版本。
- 选「非增量变化走吸附、增量变化保留阻尼」：两类变化在语义上本就不同（重排 vs 追加），
  判据落在 `auto_scroll` 单一字段上，无新增状态、无事件循环分支。

## Consequences

- 展开/收起过程（含 `/settings` 面板切换）当帧即停在最新内容，不再有可见滚动动画。
- 流式输出、状态行下移的阻尼跟随与动画刷新口径不变。
- 用户手动上翻阅读历史时开关不打断阅读位置。
- 新增验收 `crates/pony-agent-core/tests/tui_process_toggle_scroll_snap.rs`：展开/收起
  单帧贴底（`scroll_offset == max_scroll_offset` 且末条内容可见）、手动上翻不被强拉。
- 无新增网络 IO / 后台任务 / LLM 调用 / 依赖 / 持久化字段；`docs/observability.yaml`
  同提交追加归属登记（四维不变、无新增收口点）。
