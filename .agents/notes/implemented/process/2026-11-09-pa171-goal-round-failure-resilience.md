# PA-171 goal 续轮失败止损 / 限流冷却 / 失败轮归因

Status: implemented


- 日期：2026-11-09
- 关联：ADR 0103 / 任务卡 `management/task-system/03_TASKS/PA-171-goal-round-failure-resilience.md`
- 前序同类缺陷：`.agents/notes/implemented/bug-fix/2026-11-07-ask-self-lock-goal-rounds.md`（ADR 0092，
  同为「续轮不看上一轮停在何处」导致的空转；本篇补的是「续轮不看上一轮成败」这另一半）
- 波次：wave-68（红相 `e8c460ab`、契约 `e1227763`）

## Decision

1. 轮次 outcome 上浮到宿主：`start_turn_stream_checked -> Result<TurnStreamOutcome, String>`，
   `start_turn_stream` 签名冻结并内部转调（"执行成功"≠"轮次成功"）。
2. `run_goal_rounds` 连续 2 轮失败即停（`MAX_CONSECUTIVE_GOAL_ROUND_FAILURES = 2`），
   停止原因经 `GoalRoundStopReason::visible_line()` 上屏；加 `GoalRoundLoopLease` 保证单会话单续轮循环。
3. 失败轮不消耗目标预算：`advance_goal_round_for_outcome` 在 `Failed` 时只回读快照不写盘。
4. 限流/quota 命中后进入进程级冷却（键含 `base_url`），4 个 HTTP 出口发送前短路；
   首跳 decision 作用域同样生效；quota 与 429 分别映射 `UpstreamQuota` / `UpstreamRateLimited`。
5. 失败轮 trace 三归因字段落真值 + additive `errorClass`，由 `retry::classify_error_class` 单一出口派生。
6. TUI 增 `ERROR_SUMMARY_RATE_LIMITED`（含恢复时间），不再把配额耗尽折叠成"重试已用尽"。

## 为什么

- 现场 16 连败不是单点故障，而是三条缺陷叠加（见 ADR 0103 背景）：无失败感知的续轮循环 +
  派发即计费的轮次预算 + 多线程各自持有 8 轮局部预算（16 = 2×8）+ 失败不可归因。
- 只加退避只能减少撞墙次数，不能阻止"预算被失败轮烧光"与"用户看到满屏错误"；
  必须在**宿主续轮判定**和**预算写点**两处同时收口，且让失败在 trace/界面可读。

## Alternatives considered

- **只调大 `max_goal_rounds`**：把失败轮计费合理化，一次上游故障仍会烧光预算，否决。
- **给 `should_abort_goal_rounds` 加失败参数**：破坏 ADR 0092 冻结的真值表与既有测试，改为新增
  `goal_round_stop_reason` 并把失败判定上移一层，否决原方案。
- **冷却状态放 `ProviderManager` 实例字段**：实例每轮新建，冷却立即失效；改进程级 `static` 注册表。
- **冷却键不含 `base_url`**：红相反向钉子用例实测被跨 endpoint 冷却短路（503 不再重试），否决。
- **失败轮回滚 `rounds_started`（saturating_sub）**：等价于"先记后回滚"，会在并发轮次线程下产生
  读写竞态与审计歧义；改为"仅成功才推进"的单向语义，否决回滚方案。
- **本卡一并做跨 provider 故障转移**：需要 `providers.json` 候选链与新 ADR，超出最小闭包，
  登记 PA-172 候选。

## 落地与证据

- 冻结验收：`crates/pony-agent-core/tests/pa171_*.rs`（C1–C5，红相 exit=101 → 绿相 0）。
- 回执：`.dev-team/pa171-red-receipt.json`、`.dev-team/regression-baseline-wave68.json`、
  `.dev-team/state-wave68.json`、`.dev-team/contract-matrix-pa171.md`。
