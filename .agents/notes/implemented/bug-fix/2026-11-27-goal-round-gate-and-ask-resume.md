# Agent Note: goal 续轮判据单点化与作答后重入（O1-B/O2/O3/O5/O6）

Status: implemented

## Problem

`/goal` 设好目标之后，一次 `ask_user` 就能把自动续轮打断，且**作答之后目标不再自己推进**。
审核取证（`docs/analysis/goal-ask-blocking-audit-2026-11-26.md`）把根因收成五条：

1. **P0-1 作答后无驱动者**。本仓把「人类输入」建模成 turn 终态：`ask_user` 是 host-mediated
   工具 ⇒ `WaitingHost` ⇒ `suspended` ⇒ 续轮循环按设计退出。TUI 的 `AskResumeTurn` 只跑一次
   run-bound 单步、不重入循环；桌面端 `graph_resume_ask` 只把 run 放回 `Ready`，**没有任何
   线程再发下一轮**。结果是 run 停在 `Ready`、goal 停在 `active + Armed`、UI 仍显示「进行中」。
2. **P0-2 判据双实现、能力不对等**。TUI 的 `run_goal_rounds` 与 core 的
   `execute_graph_run_stream` / `advance_graph_run` 各写一套停止判据：core 缺待作答让位、缺
   连续失败熔断、失败轮照烧预算（先 `advance_goal_round` 再跑），且停止完全静默。
3. **P0-3 正文问号启发式压过 armed goal**。`planner.rs` 在 armed goal 判定之前先跑
   `assistant_requests_user_input`（trim 后以 `?`/`？` 结尾即命中）⇒ goal 轮里模型正文只要以
   问号收尾，循环就**静默**退出且不弹 Ask 面板——一条无 UI 信号的暗通道。
4. **P1-2 blocked 无硬门槛、预算耗尽不动相位**。`blocked` 只有提示词里的「连续 3 轮」软约束
   （代码零校验），模型第 1 轮即可 blocked；`rounds_started >= max_goal_rounds` 只是不再续轮，
   目标永久 `active + Armed`，而 `resume` 又被「预算耗尽」拒绝 ⇒ 死局且无解释。
5. **P1-3 桌面端零停止事件面**。只有 TUI 的 `UiEvent::GoalRoundsStopped`，桌面侧
   `goal://changed` / `goal://round` / `goal://cleared` 之外没有「为什么目标不动了」的通道。

对照 deepseek-harness：它把 ask 建模成 **turn 内的工具调用**（timed 形态超时返回 `pending`），
续轮以 `agent idle` 为条件，作答后同一 turn 继续、driver 自然续轮——所以 DSH 根本不需要
「作答后重入」这段代码。本仓的 turn 终态建模决定了这段接线必须显式存在。

## Decision

**判据单点化（O2）**：`crates/pony-storage/src/goal.rs` 新增纯函数
`goal_round_gate(GoalRoundGateInput) -> GoalRoundGateDecision{Continue, Pause, Stop(reason)}`
与 `goal_round_outcome_gate(GoalRoundStepOutcome)`，判定顺序冻结为
`has_active_goal → cancelled → 待作答宿主请求（Pause）→ 连续失败阈值 → 目标预算 → 每线程预算`。
TUI 的 `run_goal_rounds`、core 的 `execute_graph_run_stream` / `advance_graph_run`、桌面端续轮
驱动者三处全部调用它；`goal_round_stop_reason` 退化为该 gate 的三值投影薄包装。`Pause` 复用既有
`GoalRoundStopReason::TurnSuspended`（**不新增变体**，`all_variants()` 仍为 10），文案改为
「目标已暂停：等你作答｜…｜先回答上面的提问，作答后自动继续」——暂停不等于终止，措辞必须一致。

**作答后重入（O1-B）**：
- 核心层新增 `continue_ask_resume_turn_stream_checked` /
  `continue_approval_resume_turn_stream_checked`（返回 `TurnStreamOutcome`，供宿主判断恢复轮是否
  真正收口；既有 `Result<(), String>` 入口委托它们，行为不变）。
- TUI：`UiEvent::AskResumeTurn` / `ApprovalResumeTurn` 的轮次线程改用 checked 入口，成功后经
  `resume_goal_rounds_after_host_answer` 重入 `run_goal_rounds`（门控仍由同一 gate 决定）。
- 桌面：新增 `src-tauri/src/goal_resume.rs` 作为 Rust 侧驱动者，`graph_resume_ask` 与
  `control_approve_request` 成功后立即按 gate 判定续跑（`prepare_continue_graph_run_stream` +
  `spawn_graph_run_stream`，后者的 `execute_graph_run_stream` 内已含 gate 循环）。授权事实由
  `SessionAuthorizationModes` 继承会话最近一次提交的模式，**无记录 ⇒ `read_only`**（fail-closed）。

**问号启发式让位（O3）**：`DefaultGraphPlanner::decide_after_turn` 把 armed goal 判定移到
`assistant_requests_user_input` 之前——armed goal 存在时「我要问用户」只能由 `ask_user` 工具
信号收口，停机永远有对应 UI 面板。无 armed goal 时既有启发式保持有效。

**blocked 门槛与预算耗尽（O5）**：`GOAL_BLOCK_MIN_ROUNDS = 3`，`update_goal` 的 `Blocked` 分支
对 `rounds_started < 3` 返回以 `goal_block_threshold` 开头的错误（工具层映射成同名 error code，
对齐 DSH 的 `GOAL_TOOL_BLOCK_THRESHOLD`）；预算耗尽时 gate 返回 `BudgetExhausted`，调用方额外
`block_goal_for_round_limit` 落 `phase=blocked / blocked_reason=round_limit`，使相位与原因都可见。

**桌面可见性（O6）**：`GoalEvent` 新增 `RoundsStopped{session_id, goal_id, reason_code,
visible_line, rounds_started, max_rounds, consecutive_failures}` 领域事件，由
`notify_goal_rounds_stopped` 广播；`src-tauri/src/goal_events.rs` 映射为
`goal://rounds_stopped`（camelCase 载荷），`GoalBar.vue` 在会话匹配时展示 `visibleLine`，并在
`goal://round` / `goal://changed` 到达时清空（新一轮起来后「已暂停」不再成立）。

## Alternatives considered

- **O1-A：把 `ask_user` 改成 DSH 的 timed 形态**（入参加 `timeoutSeconds`，超时返回
  `{pending:true}` 让模型继续做独立工作）。这是**根治**，但改 `ToolDefinition` 入参、
  `WaitingHost` 判定与两端卡片态，属协议级变更。本轮取「先止血、再根治」：O1-B 是可在既有
  协议内验证的最小闭环，O1-A 单独立项（需独立 ADR 与协议迁移）。
- **桌面端继续由前端 auto-continue 驱动**（`ask.ts` 成功后自己调 `continue_graph_run_stream`）。
  否决：前端驱动只在窗口活着、用户没切页时成立，且 `grep autoContinue src` 零命中的现状正说明
  这类驱动很难被发现和维护；Rust 侧驱动者与 turn 生命周期同源，只依赖领域 gate。
- **新增 `GoalRoundStopReason::AwaitingHumanInput` 变体表达暂停**。否决：`TurnSuspended` 的既有
  语义就是「本轮在等你作答/审批」，新增变体会让 `all_variants()`（长度被契约测试钉死）与
  「暂停 vs 终止」两个正交维度混在一个枚举里；改为只调文案 + 用 `Pause` 决策类型区分。
- **把 `blocked` 门槛放进 `pony-storage::goal::update_goal` 之外的独立校验层**。否决：那会让
  「人类 `/goal block`」与「模型 `update_goal blocked`」共用一道不该有的限制；门槛按 DSH 口径
  属于**工具权威**（模型侧），落在工具层（`tools/misc.rs` 映射 error code）+ storage 的
  `rounds_started` 校验是唯一能同时覆盖两条调用路径的位置。
- **给 `GraphRun` 加 `tool_authorization_mode` 字段持久化**（供桌面续轮继承）。否决：改动
  `GraphRun` 形状会波及 19 处构造点与既有 run 存储的兼容性；改为进程内
  `SessionAuthorizationModes`（按会话登记最近一次提交），缺记录 fail-closed 到 `read_only`——
  与 T7「续轮必须继承授权事实、绝不回落 Allow」的既有口径一致。

## Consequences

- 三处续轮判据合并为一个纯函数，`pony-storage` 单测直接锚定真值表；TUI 与 core 的行为不再可能
  各走一路（此前 core 一次 429 会连烧 8 轮预算而用户零感知）。
- `Pause` 使「作答后自动继续」成为可断言的事实：TUI 的两个 resume 分支与桌面两个命令都是
  驱动者，作答后目标不会停在 `active + Armed`。
- O3 是**行为变更**：armed goal 下正文以问号结尾不再停机。既有冻结用例
  `wave_plan_goal_acceptance::question_heuristic_only_blocks_trailing_question_mark` 已按新契约
  改写（同一提交），并保留「无 armed goal 时末尾问号仍 WaitUser」的反向钉子。
- 预算耗尽现在会让目标进入 `blocked(round_limit)`：`resume` 仍被「预算耗尽」拒绝，但拒绝理由与
  相位都可见，用户可 `/goal` 查看或提高上限，不再是「永久进行中」的死局。
- 桌面端首次拥有停止原因通道 `goal://rounds_stopped`；前端文案完全取自
  `GoalRoundStopReason::visible_line()`，不得自拼措辞。
- 留白：连续失败阈值（2 轮）与每线程预算（8 轮）仍是两个独立数，双预算语义统一（审核 O7）与
  `TurnKind` 结构化权威（O4）不在本轮范围，已登记 backlog。
