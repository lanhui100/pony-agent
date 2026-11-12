# Agent Note: `/goal` 人类斜杠命令与 goal `clear` 动作

Status: implemented

- 日期：2026-11-12
- 交付面：Rust core（`agent/goal_command.rs`、`agent/goal.rs::clear_goal`）、Tauri（`goal_command`）、桌面（`slash-commands.ts` + `HomeWorkspace.vue`）、TUI（`bin/tui/goal_command.rs` + 命令表/队列裁决）
- 契约真源：`.dev-team/contract-matrix-goal-command.md`（C-1..C-7，FROZEN）
- 差距与取舍分析：`docs/architecture/plan-goal-dsh-gap.md`

## Problem

goal 域的能力此前只有**模型工具面**（`create_goal` / `update_goal`，`crates/pony-agent-core/src/agent/goal.rs:559/605`）：
人类要建立、暂停、恢复或结束目标，必须请求模型调用工具；`UpdateGoalAction` 也没有 `clear`，
已完成/已阻塞的目标无法移除，只能被下一次 `create` 静默覆盖。缺失的是一条
「不消耗模型轮次、不污染会话历史」的人类命令面。

## Decision

新增两端一致的 `/goal` 斜杠命令，并把**语法、状态机与文案的唯一真源放在 Rust core**
（`agent/goal_command.rs`），两端只做参数提取与渲染：

- 语法：`/goal [<目标>|edit <目标>|pause|resume|complete|clear]`，空参数 = `show`；
  大小写不敏感，首 token 切分；`/goa`、`/goalx` 不命中（仍按普通消息发送）。
- 执行：`execute_goal_command` 只碰 goal store，不读会话历史、不发起 turn；
  `Create` 在已有非 completed 目标时**拒绝并给出下一步命令**（不再静默覆盖）；
  `Edit` 命中 completed 目标时降级为新建（与 DSH `command-goal` 对齐）。
- `clear` 以 `clear_goal`（`goal.rs:686`）实现：删记录 + 原子持久化，落盘失败回滚内存；
  **不**扩散 `UpdateGoalAction` 枚举（避免污染模型工具 schema 与 `src/types/goal.ts`）。
- 桌面：`goalCommandArgs` / `describeGoalOutcome`（纯函数，`slash-commands.ts`）+
  `HomeWorkspace.vue` 命中即清草稿、走 `/compact` 同一条 info 通道、随后 `fetchGoal` 对齐后端快照。
- TUI：`goal_command_args` / `handle_goal_command`（`bin/tui/goal_command.rs`）+
  命令表登记 + 队列裁决新增 `is_local_control_command`（流式期就地执行，不拆在飞轮次、不入队）。

可机械验收的边界：命令命中后不产生模型请求、不改变会话历史长度；`clear` 后
`active_goal_for_session` 立即 `None`（续轮随之中止）。

## Alternatives considered

1. **把 `/goal` 做成纯前端字符串解析 + 直接调 `goal_create`/`goal_update`**
   否决：状态机（谁能覆盖谁、缺目标时怎么提示、completed 目标 edit 的语义）会在桌面与 TUI
   各写一份，必然漂移；且两端还要各自复刻 `phase` 文案。真值源必须在 core。

2. **在 `UpdateGoalAction` 增加 `Clear` 变体**
   否决：该枚举同时是**模型工具 schema** 与 TS 类型（`src/types/goal.ts`）的输入面，
   扩枚举会把「删除」暴露给模型工具，并迫使前端 store 增加一条无 UI 的路径。
   独立 `clear_goal(session_id)` 保持删除动作只在人类命令面可达。

3. **沿用 sidecar 快照并追加清除墓碑（对位 DSH `GoalClearChangeMeta`）**
   暂缓（登记为 P2 观察项）：墓碑的价值是审计「谁清掉了什么」，本仓当前没有该追责需求；
   先在 backlog 记录触发条件（出现追责诉求时补最小 `cleared_at/cleared_by`），不做完整事件流。
   完整 append-only + fold 重构因与 `GoalData` 序列化契约、TUI 投影、前端类型全面冲突，明确否决。

4. **桌面 `/goal` 回执写成 transcript 里的 system 消息（与 DSH `createUserMessage` 同构）**
   否决：本仓会话消息持久化由 runtime store 承担，新增「本地 system 消息」通道会污染历史长度
   （而契约要求「命令不改变会话历史」）。改用既有 `/compact` 的 info 状态通道，
   多行回执在渲染层折叠为 ` · ` 单行，状态明细由 `GoalBar` 承担。

5. **流式期把 `/goal` 归入 `is_session_mutating_command`（拒绝执行）**
   否决：该分类的语义是「会拆掉在飞轮次」。`/goal` 只写 goal store，拆轮次既无必要也有损
   （用户在流式中查看目标被拒）。改为新增 `is_local_control_command` 并纳入流式 `Forward` 白名单。

6. **不新增 `/goal`，只补 `clear_goal` 工具**
   否决：模型工具面可以删除目标，但用户无法确认「现在有没有目标、轮次走到几」；
   `show` 的回执与 `GOAL_COMMAND_USAGE` 是这一能力的最小可见面。
