# Agent Note: 修 Ask 自锁循环——续轮让位、续轮文案识别、Draft 出口

Status: implemented

## Context

TUI 单会话连续上屏「该提问已失效，已同步最新待处理请求」，模型空转 16 轮。库里 17 条 Ask
全部 `superseded`，每条都在下一条 goal 轮开始前 4–7ms 被作废；`ask_state_conflict` /
`ask_version_mismatch` 计数为 0（说明不是作答失败，而是请求已消失后的轮询收口）。

三个缺陷叠加：

1. `bin/tui.rs` 的 goal 续轮循环只判「取消 / 轮次上限」，不问「上一轮是否停在等用户」。
2. `is_host_continuation_prompt` 只认 `[goal round N] 推进目标：`，而真正下发的是
   `<goal_round round="N" max="M">继续推进目标：…`，于是续轮被当成真人新消息，
   `note_user_message_entry` 把待作答 Ask 全部判为过期。
3. Draft 计划锁死所有写/执行工具，开锁需要宿主注入 `PlanExecutionApproval`（三处宿主入口
   全写死 `None`），而 `PlanControlOperation` 没有 `abort` ⇒ 模型无退路。

## Decision

- 续轮前查 `list_pending_asks` / `list_pending_approvals`，非空即停（纯函数
  `goal_rounds_must_yield_to_pending_requests`）。
- `goal::is_goal_round_prompt` 作为 `<goal_round …>` 的真值源，接入 `is_host_continuation_prompt`；
  `note_user_message_entry` 只在真人消息时 supersede。
- `plan_control` 新增 `abort`（`plan_id`/`revision` 可省略，撤销本会话最早的 Draft）；
  `start_execution` 的审批门禁只在 plan 仍为 `Draft` 时生效；
  TUI 新增 `/approve`（`approve_earliest_draft_plan`）走既有 `plan_execution_approve` 盖章。

完整论证与验证清单见 `docs/decisions/0092-plan-draft-self-lock-recovery.md`。

## Alternatives considered

- 只关掉 supersede：会让陈旧提问永久挂着，且不解决 Draft 锁死（用户回答「批准」仍无效）。
- 让 Ask 答案文本自动当审批：违反 C-141-3（审批只能来自宿主注入）。
- TUI 每轮注入 `plan_execution_approval`：TTL 内等于会话级通行证，违反 C-141-6 one-shot。
- 只给 `/approve` 不给 `abort`：宿主不在场（headless/CLI）时依旧锁死。
- 改 `should_abort_goal_rounds` 签名：破坏 `tui_batch2_regression` N5 已锁的三参数真值表。
