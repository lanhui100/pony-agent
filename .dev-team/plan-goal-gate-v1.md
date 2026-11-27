# 方案：goal 续轮机制修复（O1-B / O2 / O3 / O5 / O6）

- 输入真源：`docs/analysis/goal-ask-blocking-audit-2026-11-26.md`
- 工作树：`/home/dm/pony-agent/.tmp/wt-goal`（分支 `fix/goal-ask-resume`）。**主工作区有另一会话未提交改动，禁止在那里写。**
- 构建：一律 `bash scripts/cargo-lane.sh <cargo 参数>`（8 job + 全局 flock）；只由本会话串行执行，不并行开两条 lane。

## 0. 冻结契约（不得擅自改签名；要改先找 Lead）

### C1 `crates/pony-storage/src/goal.rs`：单一续轮 gate（O2）

```rust
pub struct GoalRoundGateInput {
    pub has_active_goal: bool,
    pub cancelled: bool,
    pub pending_asks: usize,
    pub pending_approvals: usize,
    pub consecutive_failures: u32,
    pub rounds_started: u32,
    pub max_rounds: u32,
    pub consecutive_auto_continues: u32,
    pub max_auto_continues: u32,
}

pub enum GoalRoundGateDecision {
    Continue,
    Pause,                       // 等人类（Ask/审批待作答）：目标保持 Armed，作答后重入
    Stop(GoalRoundStopReason),
}

pub fn goal_round_gate(input: GoalRoundGateInput) -> GoalRoundGateDecision;

/// 单轮终态 → 决策（首轮门与每步归约共用）
pub enum GoalRoundStepOutcome { Completed, Failed, Suspended, Cancelled, Unclassified }
pub fn goal_round_outcome_gate(outcome: GoalRoundStepOutcome) -> GoalRoundGateDecision;
```

冻结判定顺序（`goal_round_gate`）：has_active_goal=false ⇒ Stop(NoActiveGoal) → cancelled ⇒ Stop(Cancelled) → pending_asks+pending_approvals>0 ⇒ **Pause** → consecutive_failures ≥ `MAX_CONSECUTIVE_GOAL_ROUND_FAILURES` ⇒ Stop(ConsecutiveFailures) → rounds_started ≥ max_rounds ⇒ Stop(BudgetExhausted) → consecutive_auto_continues ≥ max_auto_continues ⇒ Stop(AutoContinueBudget) → Continue。

`goal_round_outcome_gate`：Completed ⇒ Continue；Failed ⇒ Stop(FirstRoundNotSucceeded)；Suspended ⇒ **Pause**；Cancelled ⇒ Stop(Cancelled)；Unclassified ⇒ Stop(TurnUnclassified)。

`Pause` 的可见原因 = 既有 `GoalRoundStopReason::TurnSuspended`（**不新增枚举变体**，`all_variants()` 长度保持 10）；把它的 `headline` 改为「目标已暂停：等你作答」、`action_hint` 改为「作答后自动继续」（detail 保持「本轮在等你作答/审批」）。

### C2 `crates/pony-storage/src/goal.rs`：O5

```rust
pub const GOAL_BLOCK_MIN_ROUNDS: u32 = 3;
pub fn block_goal_for_round_limit(goal_id: &str) -> Result<GoalData, String>; // phase=blocked, activation=Disarmed, blocked_reason=Some("round_limit")
```
`update_goal` 的 `Blocked` 分支：`current.rounds_started < GOAL_BLOCK_MIN_ROUNDS` ⇒ `Err("goal_block_threshold: ...")`（前缀固定，工具层据此给 error_code）。

### C3 `crates/pony-storage/src/goal.rs`：停止事件面（O6 的领域事实）

```rust
pub enum GoalEvent { /* 既有三者 */ RoundsStopped { session_id: String, goal_id: String, reason_code: String, visible_line: String, rounds_started: u32, max_rounds: u32, consecutive_failures: u32 } }
pub fn notify_goal_rounds_stopped(session_id: &str, goal_id: &str, reason: &GoalRoundStopReason, consecutive_failures: u32, rounds_started: u32, max_rounds: u32);
```
（`install_goal_event_observer` 的闭包 `match` 需要补分支——桌面映射层同步。）

### C4 `crates/pony-agent-core`：O2 接线 + O3

- `planner.rs`：**armed goal 判定先于 `assistant_requests_user_input`**（armed goal 存在时不再走文本问号启发式）。
- `control_plane/mod.rs` 的 `execute_graph_run_stream` 与 `advance_graph_run` 两个循环共用 gate：
  - 每步之后用 `turn_stream_outcome_from_terminal(run.phase, error, Some(decision.kind))` 归约，再 `goal_round_outcome_gate`；
  - `Continue` ⇒ 成功轮才推进预算（`advance_goal_round_for_outcome(Succeeded)`；失败轮只累加 `consecutive_failures`，**不写盘**）；
  - `Pause`/`Stop` ⇒ `notify_goal_rounds_stopped(...)` 后返回；
  - `Stop(BudgetExhausted)` ⇒ 额外 `block_goal_for_round_limit(&goal.id)`；
  - 循环边界用 `goal_round_gate`（含 pending asks/approvals 让位、连续失败熔断、8 轮 auto-continue 预算）。
- `ask_resume_commands.rs`：新增 checked 变体，供宿主判断「恢复轮是否成功收口」：
  ```rust
  pub fn continue_ask_resume_turn_stream_checked<S: TurnEventSink>(&self, sink: &S, run_id: &str, command: StartTurnStreamCommand) -> Result<TurnStreamOutcome, String>;
  pub fn continue_approval_resume_turn_stream_checked<S: TurnEventSink>(&self, sink: &S, run_id: &str, command: &StartTurnStreamCommand) -> Result<TurnStreamOutcome, String>;
  ```
  既有返回 `Result<(), String>` 的版本改为委托它们（行为不变），避免两套执行路径。

### C5 `crates/pony-tui/src/tui.rs`：O1-B

- `UiEvent::AskResumeTurn` / `UiEvent::ApprovalResumeTurn` 的轮次线程：单步续跑返回 `Ok(TurnStreamOutcome::Completed)` 且该会话仍有 armed goal ⇒ 在该线程内继续调用 `run_goal_rounds`（克隆 `goal_round_cancel`、`current_turn_id`、`goal_round_loop_lease`）；`Pause`/`Stop`/`Err` ⇒ 走既有 `report_goal_rounds_stopped`（`goal_round_outcome_gate` 决定原因），不再静默。
- `run_goal_rounds` 内部判据全部改调 `goal_round_gate`（`should_abort_goal_rounds`、`goal_round_stop_reason` 及手工拼装保留为薄包装或删除，保持单测口径）。
- `tui/goal_round_stop.rs::first_round_stop_reason` 改为委托 `goal_round_outcome_gate`（Suspended ⇒ Pause ⇒ `TurnSuspended`，其余映射不变）。

### C6 `src-tauri`：O6 事件 + O1-B 桌面驱动

- `goal_events.rs`：新增 `GOAL_ROUNDS_STOPPED_EVENT = "goal://rounds_stopped"`，载荷 `{ sessionId, goalId, reason, visibleLine, roundsStarted, maxRounds, consecutiveFailures }`；映射 `GoalEvent::RoundsStopped`。
- 桌面驱动者（Rust 侧，不依赖前端 auto-continue）：`graph_resume_ask` 成功后，若该 run 的会话仍有 Armed goal 且无待作答 Ask/审批 ⇒ 组装 `build_goal_round_message(...)` 的 `TurnInput`，`prepare_continue_graph_run_stream` + `register_turn_sink` + `spawn_graph_run_stream`（后者的 `execute_graph_run_stream` 已含 C4 的续轮循环）。审批决策（`control_approve_request`）同样在成功且会话有 Armed goal 时触发一次同构续跑。
- 前端：`GoalBar.vue`（或 `src/stores/goal.ts`）监听 `goal://rounds_stopped`，展示 `visibleLine`（停止/暂停原因），并把文案常量与 Rust 侧同源命名。

## 1. 交付与证据

| 项 | 证据 |
|---|---|
| C1 gate 真值表 | `pony-storage` 单测（新增 `#[cfg(test)]`）+ `cargo test -p pony-storage --lib` exit 0 |
| C4 循环接线 | `cargo test -p pony-agent-core --lib` exit 0 + `--test runtime_suite` 既有 goal 契约测试 |
| O3 | planner 单测：armed goal + 正文问号 ⇒ `Continue` |
| C5 TUI | `cargo test -p pony-tui --lib` exit 0（含 goal_round_stop 单测） |
| C2/C3/C6 | storage 单测 + `cargo check -p pony-agent`（src-tauri）exit 0 + `npm test` 相关 spec |
| 文档 | ADR（`.agents/notes/implemented/bug-fix/2026-11-27-goal-round-gate-and-ask-resume.md`）、`docs/observability.yaml` 归属注释、`docs/architecture/plan-goal-dsh-gap.md` 路径更正、`backlog/log/<批次>.jsonl` |

## 2. 禁止事项

- 不改 `GoalRoundStopReason` 的变体数量（保持 10）与其既有 `code()` 字面量。
- 不新增网络出口、后台常驻线程、新依赖。
- 不动主工作区；不改 `.tmp/wt-goal` 之外的任何代码文件。
- 改公开签名前先与 Lead 对齐（本文件 §0 即冻结面）。
