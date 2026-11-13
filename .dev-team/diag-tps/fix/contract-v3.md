# F1 契约修订 v3（Lead 裁决 2026-11-12，原文照录）

task-4 原文 I1 对 `turnBurstExcluded` 的定义（`valid 记录数 − decode_eligible 记录数`）与
Test Agent 冻结的验收夹具（突发整包数）冲突。Lead 裁决采纳后者，并把原文口径保留为独立键：

> v3 修订：
> - `turnBurstExcluded` := **真突发整包数**（valid 且 gen<floor）；
> - `turnExcludedTotal` := valid − eligible（全部解码剔除），同帧保留；
> - 两者必须在代码 doc 注释里各写一行定义（禁止让读者从名字猜）。

实现落点：

- 判据唯一出处：`crates/pony-agent-core/src/agent/turn_flow.rs::turn_call_census`
  （`burst_excluded` / `no_ttft_excluded` / `other_excluded` / `excluded_total()`）。
- 落盘事件：`crate::agent::turn_event::TurnEvent::TurnSpeed`（type=`turn/speed`），
  键 = `turnSpeedTokensPerSecond` / `turnSpeedBasis` / `turnCallsTotal` / `turnCallsEligible` /
  `turnBurstExcluded` / `turnExcludedTotal` / `turnCallsTtftMissing`。
- 同帧日志：`otel::log::log_json_with_fields("info", "turn speed settled", …)`，键与事件逐字同名。
- wire 快照：`TurnMetricsSnapshot` 增 `turnCallsTotal` / `turnCallsEligible` / `turnBurstExcluded`
  （`Option` + `skip_serializing_if`，无可计数调用 ⇒ 三键全缺席）。
