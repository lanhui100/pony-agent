# Agent Note: TUI `/tasks` 看板行渲染 —— 只留状态符号 + 标题，隐藏 uuid

Status: implemented

## Context

`/tasks`（及 `Ctrl+T`）的任务看板条目渲染为
`  [✔] task-3f2a… · 标题 · ⛔1`：左侧 `[ ]` 方括号 + **内部 task uuid**。

uuid 是 `TeamTaskItem.task_id` 透传来的内部标识（`TuiTask.id`），
对使用者没有信息量，却挤占了本已紧张的行宽（面板最宽 84 列，
标题与 `@owner` 右列常被它挤掉/截断）。视觉上也与状态符号混在一起，
一眼看不出「这一行到底是已完成还是待办」。

## Decision

1. `render_task_board_stub` 的条目拼装改为 `  {状态符号} {subject}{blocked_suffix}{scopes_suffix}`，
   **删除 `task.id`** 与它前面的 ` · ` 分隔符；后缀仍由 ` · ⛔n` / ` · 写域n` 自带分隔符。
2. 状态符号去框：Completed `✔` / InProgress `●` / Pending `○` / Blocked `✕`
   （由 `[✔]`/`[●]`/`[○]`/`[✕]` 改为对应裸符号）。
3. 数据层不动：`TuiTask.id` 与 `tui_tasks_from_team_tasks` 原样保留，
   id 仍用于内部排障（日志/事件关联）。
4. 统计头、`@owner` 右列、`⛔n`、`写域n`、聚焦高亮、`panel_window` 滚动、空状态全部保留。
5. 回归测试 `task_board_hides_uuid_and_uses_status_marks`（`w3b_sub_session_tests`）：
   100×30 渲染后断言 `task-` 不上屏、`✔`/`○` 上屏、`[✔]` 不上屏、`@worker-1` 与 `⛔1` 保留。

## Alternatives considered

- **缩短 uuid / 加短码（如前 8 位、缩写 hash）**：仍然是内部标识泄漏到 UI，
  只是「少吵一点」；而且短码需要额外维护唯一性与稳定性规则（截断冲突、重命名后失效），
  对使用者「我要做哪一件事」的决策零帮助 —— 标题 + 状态已经足够定位。
  真正的排障场景有日志与 `team_task_list` 工具输出，不需要靠看板面板。
- **保留 uuid 但改成右列（与 `@owner` 并列）**：右列宽度被 `@owner` 与任务数共享，
  多数终端宽度下会先截断 uuid 变成半截乱码，反而更吵；且行内信息仍然两套标识并列。
- **`[✔]` 带框形态**：方括号多占 2 列且视觉上暗示「复选框/可勾选」，
  与看板只读的性质矛盾；裸符号与既有 `render_plan` 步骤图标（`✔`/`●`/`○`/`✘`）保持一致，
  同类状态在 TUI 各处读起来是同一个字形。
- **Emoji 状态（✅⏳🚫）**：emoji 是双宽字符，在部分终端/字体下宽度不稳定，
  且与 `render_plan` 的既有单宽符号体系冲突。

## Consequences

- 面板行宽多出 ~40 列给标题与 `@owner`，长标题被截断的概率显著下降。
- 看板不再能「肉眼抄 uuid」去人工调用 `team_task_update`；需要 id 时用 `team_task_list`。
  这是有意的取舍：面板面向决策，id 面向调试。
- 无数据结构/协议变更，`TuiTask` 序列化兼容性不受影响。
