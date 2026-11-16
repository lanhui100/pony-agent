# Agent Note: provider 本地兜底轮记为 degraded 终态并在 trace 留 provider_fallback

Status: implemented

## Problem

现场会话 `pa_01a1df2f-8315` 的 `turn-90395c04`：SSE 解码失败 → sync 兜底 connect 31s 失败 → 4×21.2s 超时 → 预算耗尽，最终由 `local_tool_followup_fallback_response`（`crates/pony-agent-core/src/agent/provider/mod.rs:5999`）产出一段本地摘要作为正文。但落盘的 `turn_end.reason` 是 `completed`：

1. **判定闸门只看 mock**：`turn_flow.rs:1496-1509` 的 `provider_failure_message` 仅对 `provider_mode == "mock"` 判失败，`"fallback"` 一律返回 `None` ⇒ 不发 `turn:failed`。
2. **终态事件把 reason 硬编码**：`build_turn_end_event`（`turn_flow.rs:1251-1265`）对 `turn:completed` 恒写 `TurnEndReason::Completed`、`error: None`，`TurnEnd` 变体（`turn_event.rs:44-52`）也没有任何 provider/fallback 字段。虽然 `TurnStreamEvent.fallback_reason` 已经一路传到 `turn:completed` 载荷（`turn_stream.rs:1611`），事件层把它丢弃了。
3. **投影侧后果**：`projection.rs:303/306-309` 只在「非 Completed」时才合成中断检查点，因此降级轮既不落检查点、也没有 `Interrupted` kind；trace/graph 把降级轮当正常完成，续跑与诊断都拿不到事实（`TurnTraceRecord` 只有工具失败派的 `degraded`，语义不同且无法表达 provider 降级）。

## Decision

1. `TurnEndReason` 新增 `Degraded`（wire 值 `degraded`）。**未知 wire 值一律读为 `Completed`**（自定义 `Deserialize`），保证新二进制读旧/未来日志都不会让整会话 `event stream degraded`（`IGNORABLE_EVENT_TYPES` 为空时未知变体会上抛）。
2. 新增 `pub fn completed_turn_end_reason(fallback_reason: Option<&str>) -> TurnEndReason`：`Some(非空)` ⇒ `Degraded`，否则 `Completed`；`build_turn_end_event` 改为消费 `payload.fallback_reason` 计算 reason，不再硬编码。
3. trace 侧：`TurnTraceRecord` 新增 `#[serde(default)] pub provider_fallback: Option<bool>`（`Some(true)` 才写键，旧 JSON 无键即可读；与既有 `degraded` 的「工具失败」语义分离，不借用）；写入点在 `runtime/turn_persist.rs` 的 trace 组装处，来源与 reason 同源（`payload.fallback_reason`）。
4. 历史投影：`projection.rs` 把 `Degraded` 归入「非 Completed」分支——无 assistant 消息时合成检查点；已有 assistant 消息（`turn:completed` 会先落 `AssistantMessage`）时把该消息正文升级为 `failed_turn_checkpoint_text` 形态（保留已交付正文 + 已执行步骤 + 失败原因），确保降级轮历史含 `[中断轮次检查点`。
5. 持久化：`turn_persist` 对 `fallback_reason.is_some()` 的轮次走既有 `build_interrupted_turn_close_out(... failure_reason = fallback_reason)` 收口，`HistoryMessageKind::Interrupted`，与取消/失败轮共用同一检查点真源（`interrupted_turn_checkpoint_text` / `failed_turn_checkpoint_text`），不新写一份文案。

## Alternatives considered

- **A. 复用 `TurnEndReason::Error`**：落选。`Error` 表示「本轮失败、无可用交付物」；降级轮**有**交付物（工具已执行 + 本地摘要），把它记成失败会让续跑策略与用户文案都失真，也让 `turn:failed` 与降级轮不可区分。
- **B. 复用 `TurnEndReason::Cancelled`**：落选。取消是用户/宿主动作，降级是上游故障；混用会让「谁终止了本轮」这一事实永久丢失。
- **C. 保持 reason=completed，只在 trace 加布尔标记**：落选。用户验收与账本口径都要求 `turn_end.reason=degraded`；且 TUI/CLI/图运行多端消费 reason，只加 trace 标记等于让事实只存在于一个消费者里。
- **D. 新增独立字段 `reason_kind`（不改枚举）**：落选。`TurnEnd` 已有 `reason` 是唯一终态语义位，再开一个平行字段会制造两套终态真源；additive 字段留给 trace（`provider_fallback`）而不是事件终态。
- **E. 让 `turn:failed` 承载降级轮**：落选。降级轮会走失败收口并丢掉已交付正文与 assistant 事件，用户可见内容会变成纯错误；且会触发失败轮的重试/上报路径，放大故障面。

## Consequences

- `turn_end.reason` 的取值域从 4 个扩到 5 个（`completed|error|aborted|cancelled|degraded`）：这是跨端持久化契约的**加法**变更。新读者把未知值读为 `completed`；**旧二进制读新日志**会把 `degraded` 判为未知变体——因为 `IGNORABLE_EVENT_TYPES` 当前为空，旧二进制会将该会话判 `event stream degraded`。该降级方向不可回滚，发布说明必须点名。
- trace 新增 `providerFallback` 键（仅 `true` 时写出），`session_turn_traces.trace_data` 每行整行重写的写入路径（`sqlite_session.rs:721`）天然兼容：新键同进 struct、读端 `serde(default)`。
- 降级轮历史含 `[中断轮次检查点]` + 失败原因 + 已执行步骤，续跑上下文不再丢失；TUI/graph 可按 reason 分流展示。
- 不改 provider 的 21s 超时与 5 次重试预算（独立议题）：本卡只解决**可见性与记账真实性**。
- backlog:B217（区分 `Error` 与进程 `Interrupted`）仍独立保留：本卡只新增 `Degraded`，未改 `Error` 的投影映射。
