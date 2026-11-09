# ADR 0102：goal 续轮失败止损、失败轮不烧预算与上游限流冷却

Status: implemented

## 背景

TUI 会话 `pa_01a1ba70-…`（"按 trace 做维度审核与优化"，即 PA-168~170 trace 审核 loop 的驱动会话）
在 2026-11-09 10:28–10:30 连续上屏 16 条同型失败，用户侧只看到「上游服务暂时不可用，重试已用尽」。
真实库 `~/.local/share/PonyAgent/sessions.db`（`session_turn_traces` / `turn_events`）的现场证据：

- 10:13–10:26 五轮均在跑到 `CALL MODEL #52–#92` 时被 **45002ms 总超时**杀死（sync POST 的
  `.timeout(45s)` 整请求时限）；10:28:51 起上游转为 429 `exceeded_current_quota_error`
  （Kimi 5 小时配额，报文含 `not asked again before 06:01 UTC`）。
- 10:29:08–10:30:22 `goal-8` 至 `goal-20` 共 **16 轮**依次 `turn/end reason=error`，
  每轮首个模型调用即失败，`turn_duration_ms` 多为 null；轮号乱序（8→5），`roundsStarted` 于
  10:30:22 触顶 20/20，之后 planner 判 `WaitUser`、横幅仍显示「进行中 [20/20]」。

三条独立缺陷叠加成"一次限流 ⇒ 满屏错误"：

1. **续轮不看轮次成败**：`bin/tui.rs::run_goal_rounds` 的循环判据只有 Esc / 8 轮局部预算 /
   待作答宿主请求；`control_plane::start_turn_stream` 返回 `()`（`mod.rs:2145` 显式丢弃 Result），
   续轮循环根本拿不到上一轮终态 ⇒ 失败被"重放"。
2. **失败轮照样烧目标预算**：`goal::advance_goal_round` 在**派发时**即 `rounds_started += 1`，
   与轮次结果无关；且 `consecutive` 是线程局部计数、`rounds_started` 是跨线程全局计数 ⇒
   本会话出现多个轮次线程各自持有 8 轮预算，叠加成 16 轮并把目标预算烧光。
3. **失败不可归因、不可读**：`turn_flow` 的失败事件把 `provider_source/provider_mode/fallback_reason`
   硬编码为 `None`；TUI 的 `user_facing_error_summary` 把 429/配额类折叠成
   「上游服务暂时不可用，重试已用尽」，真实原因只进 stderr。

## Decision

1. **轮次 outcome 上浮**（`agent/control_plane/mod.rs`）：新增
   `start_turn_stream_checked -> Result<TurnStreamOutcome, String>`，
   `TurnStreamOutcome { Completed, Failed{error}, Cancelled, Suspended, Unknown }`，
   由 `GraphRunTurnResponse.decision.kind` 与轮次终态派生；`start_turn_stream` 签名冻结（内部转调）。
   语义：**"执行成功"不等于"轮次成功"**——429 失败轮是"执行成功 + failed 终态"，只看 `Result` 仍判不出来。
2. **续轮失败止损 + 单循环租约**（`bin/tui.rs`）：`run_goal_rounds` 累计连续失败，
   达 `MAX_CONSECUTIVE_GOAL_ROUND_FAILURES = 2` 即停；新增 `GoalRoundStopReason`
   （`Cancelled` / `ConsecutiveFailures{failures}` / `BudgetExhausted{…}` / `AutoContinueBudget`）
   与 `visible_line()`，停止原因经 `UiEvent::GoalRoundsStopped` 上屏（**不再静默 break**）。
   同时加 `GoalRoundLoopLease`（`compare_exchange` + RAII）保证**一个会话同时只有一个续轮循环**，
   消除「多线程各持一份 8 轮预算」的叠加；`should_abort_goal_rounds` 语义零变更。
3. **失败不烧预算**（`agent/goal.rs`）：新增 `advance_goal_round_for_outcome(goal_id, outcome)`，
   `Failed` 只回读快照不写盘；`advance_goal_round` 及其 CAS/持久化逐字未动。
4. **限流分类与冷却**（`agent/retry.rs` + `agent/provider/mod.rs`）：quota-only 措辞
   （`exceeded_current_quota_error` / `usage limit` / `out of its allowance` / `quota` / `billing window`）
   归既有 `ErrorCode::UpstreamQuota`，`429/rate_limit/too many requests` 归 `UpstreamRateLimited`；
   命中后登记**进程级** `ProviderCooldownRegistry`（Key 含 `base_url`，防跨 endpoint 误杀，
   窗口取 `Retry-After` > 报文恢复时间 > 默认 60s，上限 6h），4 个 HTTP 出口**发送前** gate 短路
   （冷却期内 0 次 HTTP），首跳 decision 作用域同样生效。新增公共面
   `classify_error_class / is_rate_limit_cooldown_error / rate_limit_recovery_hint / rate_limit_window_ms`。
5. **失败轮归因真值**（`agent/turn_flow.rs`、`runtime/types.rs`、`session/types.rs`）：
   新增 `FailedTurnAttribution{provider_source,provider_mode,fallback_reason,error_class}` 与
   `emit_stream_failed_attributed` 旁路（14 参 `emit_stream_failed` 冻结）；
   `error_class ∈ {timeout,rate_limited,quota,network,auth,bad_request,other}` 由
   `retry::classify_error_class` 单一出口派生；`TurnStreamEvent.errorClass` 为 additive 字段（`serde(default)`）。
6. **文案如实**（`bin/tui/errors.rs`）：新增 `ERROR_SUMMARY_RATE_LIMITED`（含恢复时间），
   插在 auth 之后、transient 之前；`is_transient_provider_error` 不动（被既有验收冻结）。

## 候选方案 / Alternatives considered

- **只加退避、不改续轮**（落选）：退避不能解决"轮次预算被失败轮吃掉"与"多循环叠加"，
  5 小时配额窗口内任何退避都是徒劳；且用户仍会看到 16 条错误。
- **把 `should_abort_goal_rounds` 加失败参数**（落选）：会破坏既有冻结真值表与 ADR 0092 的语义；
  改为新增 `goal_round_stop_reason` 并把失败判定上移一层。
- **失败轮不回滚预算，只调大 `max_goal_rounds`**（落选）：等于承认失败轮计费，目标预算仍会被
  一次上游故障烧光；且掩盖真实缺陷。
- **跨 provider 自动故障转移**（不在本篇，登记 PA-172）：需要 `providers.json` 增候选链、
  选择器与健康度评分，属新契约与新 ADR，超出"最小闭包"范围。
- **冷却状态放 `ProviderManager` 字段**（落选）：ProviderManager 每轮新建，冷却会随实例消失 ⇒
  改为进程级 `static` 注册表。
- **冷却键不含 `base_url`**（落选）：同进程内不同 endpoint 各自有独立配额窗口，同名同 model
  共享冷却会产生跨环境误杀（红相反向钉子用例实测被短路）。

## 影响

- 行为：上游限流从"每轮 5 次短退避 + 继续续轮"变为"1 次失败即冷却 + 连续 2 轮失败即停并写明原因"；
  goal 轮次预算只统计有效轮次；同一会话不会再有第二个续轮循环。
- 兼容性：全部 additive（新增函数/枚举变体/可选字段）；`start_turn_stream`、
  `emit_stream_failed`、`should_abort_goal_rounds`、`advance_goal_round` 四个签名冻结，既有测试零改动。
- 可观测性：失败轮 trace 的 `providerSource/providerMode/fallbackReason/errorClass` 可机判；
  不新增 enforcement_point（仍属 tracing 维度，落点 `agent/otel/mod.rs` 不变）。
- 待办：跨 provider 故障转移与 45s/5 次重试配置化 → 登记 PA-172 候选（不在本 ADR 范围）。

## 验证

- 冻结验收（红相先锚定）：`crates/pony-agent-core/tests/pa171_goal_round_failure_contract.rs`（7 例）、
  `pa171_provider_rate_limit_cooldown_contract.rs`（loopback mock 物理计数：限流单次 1 次 HTTP、
  冷却期 0 次 HTTP、503 仍重试）、`pa171_failed_turn_attribution_contract.rs`、
  `pa171_error_copy_contract.rs`。
- 回归：`cargo check -p pony-agent-core --all-targets` Exit 0；
  `cargo test -p pony-agent-core --test tui_batch2_regression --test tui_session_resilience_acceptance`
  = 96 passed / 98 passed。
- 现场对照：`sessions.db` 中 `pa_01a1ba70` 的 16 连败形态在修复后不再可能（连续 2 轮失败即停）。
