# ADR 0083：TUI 子会话导航——Ctrl+↑ 返回主会话、Ctrl+←/→ 切换队友、输入区身份徽标

Status: implemented

日期：2026-11-07
契约真值源：`crates/pony-agent-core/src/bin/tui.rs` 的 `sub_session_shortcut` / `sub_session_candidates` /
`step_sub_session_target` / `sub_session_badge_text` / `is_agent_roster_command`；
测试 `crates/pony-agent-core/tests/w3_sub_session_nav_test.rs`

## 背景

`/agents`（花名册）允许 Enter 进入某个 teammate 的会话，但进入之后**没有任何出路**：

- 没有返回主会话的入口 —— 用户只能 `/resume` 重新挑会话，而 `/resume` 列的是宿主会话，
  队友会话跑在独立建栈（独立 `SessionStore`）里，常常根本不在列表中；
- 界面上没有身份标识 —— 输入框、header、状态行都仍是宿主口径，用户无法判断自己正在看谁的会话；
- 无法在多个队友会话之间横向比较 —— 必须退出花名册重新进入，且退出路径本身缺失。

结果是「可进入、不可返回、不可辨识」的单向门。本决策补齐导航闭环与身份可见性。

## 候选方案与拒绝理由

- **A. 复用 `/resume` 作为唯一返回路径**：拒绝——队友会话不在宿主会话表内（独立建栈），
  `/resume` 结构性看不到；且它要求用户记住主会话的短 id，属把导航成本转嫁给用户。
- **B. 用 Esc 返回**：拒绝——Esc 在 TUI 已承载取消弹窗 / 双击停止 / 关闭面板三重语义，
  再叠一层「回到主会话」会造成同一键在不同上下文下破坏性不一致。
- **C. 用 Alt+方向键 / 裸方向键**：拒绝——裸方向键是光标移动，Alt+↑/↓ 在部分终端被窗口管理器吞掉，
  且 crossterm 对 Alt+方向键的上报在 Windows Terminal 与 iTerm2 上不一致，无法形成可断言契约。
- **D. 采用方案**：**Ctrl + 方向键**作为子会话导航专用面——
  `Ctrl+↑` 返回主会话、`Ctrl+←/→` 在候选队友会话间循环；**只在子会话内消费**这些键
  （主会话里 `Ctrl+←/→` 保持原有光标语义）；同时在输入区下方渲染一行身份徽标给出
  「我是谁 / 第几个 / 怎么返回 / 怎么切换」。Ctrl+方向键在主流终端与 crossterm 下上报稳定，
  且不与既有快捷键冲突（已由 `tui::w3b_sub_session_tests::shortcut_only_ctrl_arrows` 与黑盒用例机械守住）。

## 决策

1. **按键裁决纯函数**：`sub_session_shortcut(&KeyEvent) -> Option<SubSessionShortcut>`，
   仅当 `modifiers.contains(CONTROL)` 时映射 `Up => ReturnToMain`、`Right => Switch(Next)`、
   `Left => Switch(Prev)`；裸方向键、Alt+方向键、`Ctrl+Char` 一律返回 `None`（调用方**不消费**）。
2. **消费边界**：分发点排在文本插入、斜杠菜单与光标移动**之前**，但仅在 `state.sub_session.is_some()`
   时生效。因此主会话中 `Ctrl+←/→` 的原语义（按词移动光标）不受影响。
3. **候选口径**：`sub_session_candidates(entries, loadable)` 剔除三类条目——`lead`（宿主主会话本身）、
   尚无 `session_id`（未成功运行过）、`loadable(session_id) == false`（宿主 `SessionStore` 加载不到，
   hydrate 只会得到空会话）。顺序沿用花名册，不重排、不去重，保证 `n/m` 序号稳定可对照。
4. **循环步进**：`step_sub_session_target(candidates, current, step)` 取模循环；
   `current` 不在候选内（或为 `None`）时 `Next` 取首、`Prev` 取末；无候选返回 `None`（安全 no-op）。
5. **唯一进入路径**：`enter_sub_session_at(state, control_plane, candidates, index)` 是 Enter 与
   `Ctrl+←/→` 的共用收口：保存返回目标 → 切 `session_id` → `reset_session_scope()` → **写回**
   `return_session_id`（仅当原为 `None`）+ `sub_session` → `hydrate_state_from_session` →
   `sync_pending_control_request`（换会话即重建面板）。
   `return_session_id` 只在首次进入时记录：队友会话之间切换必须保留最初的主会话 id，
   否则 `Ctrl+↑` 会退回到上一个队友会话。
6. **流式守卫**：本轮在飞时（`is_streaming`）只给一次性提示「本轮进行中：先停止或等待结束后再切换会话」，
   **不动 `session_id`、不 hydrate**（与 `/clear`、`/resume` 的流式期拒绝口径一致）。
7. **身份徽标**：`sub_session_badge_text` 为文案**唯一真值**——
   `子智能体 · <name>（<role>）[ i/n] · Ctrl+↑ 返回主会话 · Ctrl+←/→ 切换`；
   序号段仅在 `position` 已知时出现（不得输出半截 `n/`）。渲染层（`ui()`）只负责分段配色与按内容宽截断：
   子会话时在输入区下方占 **1 行**，主会话不占行（几何与改动前一致），窄终端截断不 panic。
8. **作用域复位**：`reset_session_scope()` 一并清空 `agent_roster_state` / `return_session_id` /
   `sub_session`，返回主会话后不残留队友身份与返回目标。
9. **`/teams` 别名**：`is_agent_roster_command` 让 `/teams` 与 `/agents` 等价（用户从「团队」直觉出发
   也能到达花名册）；花名册与本条提示同步暴露上述快捷键，`/help` 一并更新。

## 影响

- **代码面**：`crates/pony-agent-core/src/bin/tui.rs`（快捷键裁决、候选/步进、徽标文案与渲染、
  进入/切换/返回三条运行时路径、`reset_session_scope` 清理面、`/teams` 别名、提示与 `/help` 文案）、
  `crates/pony-agent-core/src/tui/protocol.rs`（`SubSessionContext` / `SubSessionShortcut` /
  `SubSessionStep` / `SubSessionCandidate` 类型契约）。
- **契约面**：新增三个纯函数（快捷键裁决、候选过滤、循环步进）与一个文案真值函数，均可单测；
  子会话状态（`sub_session` / `return_session_id`）绑定会话作用域，随 `reset_session_scope` 归零。
- **不改的部分**：队友引擎仍跑在独立建栈里，宿主**不会**因此能读到队友会话库；
  「不可加载」仍是如实提示，不伪造空会话。花名册数据源（`list_agents` / 收件箱 / 驱动态）不变。
- **§3 观测性**：结构性 N-A（纯本地终端渲染 + 内存状态迁移，无新增对外接口 / 网络 IO /
  后台任务 / LLM 调用），登记见 `docs/observability.yaml` 的 `wave-tui-sub-session-nav` 条目。

## 验收

| 契约 | 断言 |
| :-- | :-- |
| C1 徽标文案含身份、序号与两条快捷键提示 | `a1_badge_text_carries_identity_and_shortcut_hints` |
| C2 序号未知时不输出半截 `n/m` | `a2_badge_text_omits_sequence_when_position_unknown` |
| C3 子会话渲染出徽标行、主会话不占行 | `a3_sub_session_renders_badge_row_and_main_session_does_not` |
| C4 极窄终端截断不 panic | `a4_badge_survives_extremely_narrow_terminal` |
| C5 `Ctrl+↑` 映射为返回主会话 | `b1_ctrl_up_is_return_to_main_shortcut` |
| C6 裸/Alt 方向键不被消费（不抢主会话语义） | `b2_plain_and_alt_arrows_are_not_consumed` |
| C7 返回主会话后清空作用域与返回目标 | `b3_returning_to_main_resets_session_scope` |
| C8 `Ctrl+←/→` 映射 Next/Prev | `c1_ctrl_arrows_map_to_next_and_prev` |
| C9 两端取模回绕正确 | `c2_switching_wraps_around_at_both_ends` |
| C10 单候选步进到自身（不误切） | `c3_single_candidate_steps_to_itself` |
| C11 候选剔除 `lead` / 无会话 / 不可加载 | `d1_candidates_exclude_lead_sessionless_and_unloadable_entries` |
| C12 无候选时步进为安全 no-op | `d2_empty_candidates_make_stepping_a_safe_no_op` |
| C13 作用域复位不残留身份与返回目标 | `e1_reset_session_scope_clears_identity_and_return_target` |
| C14 `/agents` 与 `/teams` 等价 | `f1_agents_and_teams_commands_are_equivalent` |
