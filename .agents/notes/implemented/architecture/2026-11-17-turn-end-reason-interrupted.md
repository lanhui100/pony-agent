# Agent Note: 终态三分——「失败」不再共用「已中断」

Status: implemented

## Problem

`TurnEndReason` 只有 `Completed/Error/Aborted/Cancelled/Degraded`，而历史投影把**除 Cancelled 之外**的一切非完成终态都压成 `HistoryMessageKind::Interrupted`：

- `TurnEndReason::Error`（本轮真的失败：上游 5xx、工具链失败）与「进程崩溃/强制退出导致本轮没跑完」共用同一历史行种类；
- `bin/tui/errors.rs` 对 `Interrupted` 一律显示「上一轮已中断，已保留已执行的步骤记录。」——**把失败说成被中断**；
- 续跑判断因此失真：用户/上层看到「已中断」会倾向原样续跑，而真正该做的是先处理失败原因。

现场同源证据（`pa_01a1df2f-8315`）：一次 follow-up 连续超时最终以本地兜底正文收尾，历史与状态条都无法区分「上游挂了」与「进程被杀」。DSH 对照（`packages/core/session/src/types.ts`）把 `error / aborted / blocked / max-tokens / interrupted / forked` 显式分列，并为开放 turn 补 synthetic closer ——它从不让两种语义共用一个终态。

## Decision

1. **库层立法**：`TurnEndReason` 新增 `Interrupted`（wire `"interrupted"`），语义**仅**「本轮没跑完就被进程终止」；`Error` 保持「本轮真的失败」。读取端新增 `"interrupted"` 分支，未知 wire 值仍 fail-soft 退化为 `Completed`（旧读取端读新日志不得炸开）。
2. **历史行三分**：`HistoryMessageKind` 新增 `Error`（wire `"error"`）。投影映射改为 `Cancelled→Cancelled`、`Error→Error`、其余（`Interrupted/Aborted/Degraded`）→`Interrupted`——一次 match 表达三态，不再用 `_` 吞掉失败。
3. **写侧归属**：失败收口（`persist_failed_turn_context`）由 `Interrupted` 改判 `Error`；`Interrupted` 的**写入点只在恢复路径**（进程退出后补写开放 turn），该收口随 B209「轮内增量落盘 + 恢复补写」落地，本条只立法映射与消费，**不假装已经写**。
4. **展示层分流**：`INTERRUPTED_ERROR_LINE`（"上一轮因错误终止，已保留已执行的步骤记录。"）与 `INTERRUPTED_FAILED_LINE`（"上一轮已中断…"）分列，`kind` 优先、文本标记回退（旧会话零迁移）。
5. **trace 信封不扩词表**：`Interrupted` 沿用 `turn.failed/failed` 信封——`CanonicalTurnPhase` 只登记 `completed/failed/cancelled`；「中断 ≠ 失败」由历史行种类与状态条文案承载，不在 trace 里新造未登记 phase。

## Alternatives considered

- **只加 UI 文案、不动库层枚举**：展示层拿不到「失败 vs 中断」的事实，只能嗅探 `error` 文本，等于把语义判断塞回字符串——同一事实两处猜。否决。
- **给 `HistoryMessageKind::Interrupted` 加 `reason: Option<String>` 双字段**：读侧要遍历每个消费点做二次分派，旧数据还得回退嗅探；而终态本身就是有限枚举，扩一个变体更小更硬。否决。
- **新造 `turn.interrupted` trace 信封/phase**：`CanonicalTurnPhase` 是已登记词表，新增值会打穿 trace 对拍与外部消费方（Grafana/Tempo 查询口径）；收益只是把语义放到更低层。否决。
- **在 `reconcile_stale_runs` 里直接补写 turn/end**：该函数只持有 `GraphRunMap`，没有会话存储句柄；强行注入会把存储依赖拖进图运行状态机（高耦合）。恢复补写归属 B209 的加载期收口。否决。

## Consequences

- 失败轮历史/状态条不再自称「已中断」；进程中断轮待 B209 接线后同样可区分（本条的 `Interrupted` 消费端已就绪）。
- 旧会话（无 `kind` 字段）继续走文本标记嗅探：`[中断轮次检查点]` → 中断文案，行为不变。
- 新增 wire 值对旧读取端安全（fail-soft 到 `Completed`），但**不要**据此判定「旧版本能正确解释中断」——它只会当作正常完成。
- 回归口径：`cargo test -p pony-agent-core --lib history_projection_terminal_kind_keeps_three_states_apart`、`... --lib interrupted_reason_round_trips_and_unknown_wire_stays_fail_soft`、`... --bin pony error_kind_is_not_reported_as_interrupted`。

## Related

- `2026-11-16-degraded-turn-reason-and-trace.md`（`Degraded`：provider 兜底收尾的第四态）
- `2026-11-17-followup-phase-observer.md`（降级**过程**的阶段披露）
- `2026-11-22-degraded-turn-kind-separation.md`（同族第三步：`Degraded` 也独立成
  `HistoryMessageKind::Degraded`，不再借用 `Interrupted`；本条把 `Error` 分出，那条把降级分出）
- B209（轮内增量落盘 + 恢复补写，`Interrupted` 的写侧归属）
