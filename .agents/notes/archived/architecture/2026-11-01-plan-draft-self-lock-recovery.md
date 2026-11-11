# Agent Note: Ask 自锁循环的三处收口（续轮让位 / 续轮文案识别 / Draft 撤销与宿主批准）

Status: implemented

> **归档冻结说明**：本文档为原历史决策 `docs/archive/decisions/0092-plan-draft-self-lock-recovery.md` 之归档快照，仅供溯源。

## 背景

TUI 单个会话连续上屏「该提问已失效，已同步最新待处理请求」，模型空转 16 轮。真实库
`~/.local/share/PonyAgent/sessions.db` 的现场证据：

- 该会话 `pending_asks` 17 条 Ask **全部 `superseded`**，且每条都在下一条 goal 轮
  `user/message` 落库前 4–7ms 被作废；末条 Ask 被用户 4 秒内答上才解开。
- 同一会话 `turn_events` 显示 goal 续轮 round 2→16 连续自转。
- 日志中 `ask_state_conflict` / `ask_version_mismatch` / 提交回答失败计数均为 0
  ⇒ 「已失效」不是作答失败，而是 500ms 轮询按「请求已消失」收面板并上屏提示。

三条独立缺陷叠加成自锁：

1. **续轮不看「上一轮停在等用户」**：`bin/tui.rs` 的 goal 续轮循环只判
   `should_abort_goal_rounds(cancelled, consecutive, max)`，模型用 `Ask` 挂起后循环立刻开下一轮。
2. **续轮文案被当成真人消息**：goal 续轮真正下发给模型的是
   `<goal_round round="N" max="M">继续推进目标：…</goal_round>`（`goal::build_goal_round_message`），
   而 `is_host_continuation_prompt` 只认 `display_message` 形态的 `[goal round N] 推进目标：…`。
   于是新一轮入口 `note_user_message_entry` 判定「人类发了新消息」，把会话内所有 Pending Ask
   按 ADR 0074 §3 判为 `Superseded`，面板随之收口。
3. **Draft 计划只上锁、没有出口**：`plan_draft_gate` 用 `Draft` 计划拒绝所有写/执行类工具，
   开锁只有 `start_execution`（需要宿主注入 `PlanExecutionApproval`）或 `abort`；
   而 `TurnInput.plan_execution_approval` 在 `bin/tui.rs`、`bin/cli.rs`、
   `bin/non_tauri_harness.rs`、全部 tauri/e2e probe 里一律写死 `None`，
   `PlanControlOperation` 又没有 `abort` ⇒ 模型只能反复「请求批准」，宿主没有入口、
   模型没有退出机制，形成第 2 条循环的燃料。

## Decision

三处收口，全部落在**宿主/工具面已有的语义**上，不新增旁路：

1. **续轮让位**（`bin/tui.rs`）：新增纯函数
   `goal_rounds_must_yield_to_pending_requests(pending_asks, pending_approvals)`；
   goal 续轮循环在开下一轮前查 `list_pending_asks` / `list_pending_approvals`，非空即 `break`。
   语义：宿主请求待作答时，自动推进让位给人。
2. **续轮文案真值源对齐**（`agent/goal.rs` + `agent/session/store.rs` + `ask_plan_commands.rs`）：
   - `goal::is_goal_round_prompt` 识别 `<goal_round round="…" max="…">继续推进目标：…</goal_round>`
     （结构校验：两个无符号整数属性 + 文案前缀 + 闭合标签），
     `is_host_continuation_prompt` 接线到它（旧的 `[goal round N]` 形态保留，向后兼容）；
   - `note_user_message_entry` 先判续轮文案再 supersede：**只有人的消息推进对话序**。
3. **Draft 出口**（`agent/plan_state.rs` + `agent/tools.rs` + `agent/governed_executor.rs` + `bin/tui.rs`）：
   - `PlanControlOperation` 新增 `abort { session_id, plan_id?, revision? }`：
     省略 `plan_id` 时撤销本会话最早的 `Draft`（`revision` 缺省取当前值），
     使丢失 `plan_id` 的模型仍能一键解锁；`abort` 不触发执行审批门禁（与既有注释一致）。
   - `start_execution` 的执行审批门禁改为**仅在 plan 仍为 `Draft` 时**索要审批：
     宿主已经用 `plan_execution_approve` 直接推进时，模型收到的不再是误导性的
     `plan_execution_approval_required`。
   - TUI 新增 `/approve`（`approve_earliest_draft_plan`）：唯一的宿主盖章入口，
     调既有 `plan_execution_approve(session, plan_id, revision)` 把最早的 `draft` 计划推进到
     `executing`。模型工具参数永远无法伪造该动作（C-141-3）。

## 候选方案 / Alternatives considered

- **A. 只修「提问别被判过期」（把 supersede 整体关掉）**：落选。会让真的陈旧提问永远挂着，
  面板堆积不可消费的句柄；且不解决 Draft 锁死——用户看到提问、回答「批准」也只是聊天文本，
  系统照样拒绝，反而更像假成功。
- **B. 让 Ask 的答案自动成为 `PlanExecutionApproval`（文本匹配「批准」）**：落选。
  违反 W36 / C-141-3（审批只能来自宿主注入，模型可控文本不是来源），
  且把「批准」这种授权动作降级成自然语言猜测。
- **C. 在 TUI 一次性把 `plan_execution_approval` 注入每一轮 `TurnInput`**：落选。
  TTL 内的持续放行等于会话级通行证，与 C-141-6「one-shot per plan」冲突；
  直接把状态推进（`plan_execution_approve`）语义更窄、更好审计，且复用已有 Tauri 命令。
- **D. 只靠 `/approve`，不给模型 `abort`**：落选。宿主不在场（headless / CLI / 掉线）时仍会锁死；
  `abort` 是模型侧唯一自洽的退出机制，且使「Draft 只上锁无出口」的结构缺陷消失。
- **E. 给 `should_abort_goal_rounds` 加参数改签名**：落选。既有 `tui_batch2_regression` 的 N5
  真值表逐条锁住三参数语义，改签名等于顺手改契约；新增纯函数可测且不破坏既有断言。

## 影响

- goal 自动续轮在「等用户回答 / 等审批」时停住，不再秒杀提问；用户看到的面板保持可作答。
- 历史会话标题/轮次统计不再把 `<goal_round …>` 当真人消息（与 ADR 0074 的「新消息只指人的消息」一致）。
- Draft 计划有两条正当出口：模型 `abort`（自助）与宿主 `/approve`（授权执行）。
- **headless / CLI 侧的刻意留白**：`bin/cli.rs`、`bin/non_tauri_harness.rs` 与 tauri/e2e probe 的
  `plan_execution_approval` **仍为 `None`**（Alternatives C）。这些入口没有交互式宿主在场，
  注入 TTL 内长期有效的审批等于会话级通行证；它们的自恢复路径是模型 `abort`，
  由 `tests/wave36_draft_gate_session.rs::draft_gate_is_released_by_plan_control_abort_without_host_approval` 锁定。
  即：**桌面/TUI 走 `/approve`（宿主盖章），headless 走 `abort`（模型撤销）**，两条路径都不放宽 c141 门禁。
- 命令表契约同步：`/approve` 进入 `AVAILABLE_COMMANDS`（C-4），`.dev-team/contract-matrix-slash-commands.md`
  C-4 原地改写为 15 条真值并登记流水；`tests/slash_commands_acceptance.rs::CONTRACT_COMMANDS` 同提交同步，断言未放宽。
- 冻结契约 C-141-1..9 不受削弱：无审批时 Draft 的 `start_execution` 仍被拒（`c141_2` 未改）。

## 验证

- `agent/session/host_continuation_goal_round_tests.rs::host_continuation_prompt_recognizes_goal_round_envelope`
  （`<goal_round …>` 正例 + 五个形似反例；单列文件，`session/tests.rs` 已触规模棘轮）。
- `agent/control_plane/ask_plan_commands/goal_round_prompt_tests.rs::goal_round_continuation_does_not_supersede_pending_ask`
  （续轮不 supersede；真人消息仍 supersede；单列文件，`control_plane/mod.rs` 已触规模棘轮）。
- **规模门禁（棘轮）**：`tui_event_loop` 的续轮循环抽成 `run_goal_rounds`（2292 行 < 基线 2320），
  `/approve` 分支抽成 `handle_approve_command`；`governed_executor::decide_with_context` 保持 209 行
  （`start_execution`/`complete_step` 两条 Draft→Executing 迁移合并判定，行数零增长）。
  `agent/session/store.rs`（基线允许 ≤5683，改动前正好 5683）本次**净减 1 行**：
  `is_host_continuation_prompt` 删掉 `trimmed.is_empty()` 早退（空串在 `strip_prefix` 分支自然落回
  `false`，语义等价），与新增的 `is_goal_round_prompt` 判定相抵——棘轮文件不接受「先长 3 行再申请豁免」。
- `agent/plan_state.rs::handler_abort_retires_draft_without_plan_id_and_unblocks_the_draft_gate`。
- `tests/wave36_draft_gate_session.rs::draft_gate_is_released_by_plan_control_abort_without_host_approval`
  （abort 不经审批可达，撤销后写工具恢复）。
- `tests/tui_batch2_regression.rs::n5b_goal_rounds_yield_to_pending_host_requests`。
- `bin/tui/tests/plan_approve_tests.rs`（3 例，真实 `HostControlPlane` + `PlanStore`，无 mock）：
  无 Draft 时如实汇报、Draft 被盖章后 `lifecycle=executing` 且 `revision 1→2`、模型 `abort` 后不再假装可批。
- `tests/slash_commands_acceptance.rs::t1_tui_command_table_drops_cost_and_adds_compact`
  （C-4 命令表 15 条真值，`/approve` 位置与条目受冻结断言约束）。
- `tests/wave36_plan_execution_approval.rs::c141_*` 全绿（审批门禁语义未放宽）。

