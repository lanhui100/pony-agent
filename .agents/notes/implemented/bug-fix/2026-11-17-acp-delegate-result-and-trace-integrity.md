# Agent Note: ACP 委派结果正文与 trace 完整性修复

Status: implemented

## Problem

真实会话 `pa_01a1e8fd-a6a3-7543-91ef-ae18397fa858` 首轮把任务委派给 Antigravity（`acp_delegate`，317.556s，`updates=111`）：外部 agent 实际产出了约 4.8KB 的 backlog 清单（活跃 18 条 / 归档 155 条 / ID 高水位 B231 / 依赖列 / 退出码证据，见 Antigravity 会话 `1c2c45fa-…` step 20），但 Pony 侧 `ToolResult` 只有硬编码 `summary.text="外部 agent 已完成本轮任务。"`，正文零留存；父模型被迫本地重查（多 2 次 followup、input 66,960 tokens），最终答案漏掉外部 agent 的一手结论。

根因是四个层面的完整性缺口：结果面（`NoopTurnEventSink` + `build_ok_result` 只输出计数）、可观测面（子活动/用量无法归集父 turn，`UsageUpdate` 只 `eprintln`）、trace 面（`build_tool_event` 用 `payload.sequence` 当 `step`；retry window 不落盘）、持久化面（整轮仅轮末 flush、`created_at_ms` 全塌缩、崩溃丢整轮，B209 同源）。

## Decision

以 additive 契约修复四面（真源 `.dev-team/contract-matrix-acp-delegate-p0p2.md` C0.1–C6.3）：

- **C0**：`acp_delegate` 成功结果增 `finalText`/`finalTextTruncated`/`usage`；`AgentMessageChunk` 按到达序拼接，32,000 字符（`ACP_DELEGATE_FINAL_TEXT_MAX_CHARS` 单一常量）前缀截断；空正文如实提示；`truncated`（撞输出/轮次上限）仍回传 finalText 且 `finalTextTruncated=true`；refused/cancelled/error 不产 finalText。
- **C1**：外部 `tool_call/tool_call_update` 以父 session/turn/call_id 经 `turn_flow::emit_external_turn_event` 落父 turn `turn:tool`（`parent_activity_id=call_id`、状态词表映射、turn_id 不取 ACP session id）；无通道/无上下文静默跳过；文本 chunk 不混入父 assistant 流。
- **C2**：`usage_update` 的 used/size/cost 进结果 JSON 与结构化日志，不写 `usage_ledger`（外部计费口径不同源）。
- **C3**：`src/acp/**` 32 处裸 `eprintln!` 迁移到 `agent::otel::log`（json + trace_id）；CLI `--resume` 补一条结构化诊断；`ponycli acp` 在 jsonl/sse 下入口即改道 stderr（保 wave-59 C1）。
- **C4**：`ToolCall/ToolResult.step` 取产生该调用的 turn step index（`payload.step`，None≡0），删除 `payload.sequence` 误用；`parent_activity_id` 从 activity 透传。
- **C5**：`ProviderCallCacheRecord` 增 `attempts/failed_attempt_ms/prelude_ms`，由 `ProviderCallRetryMeta` 单一真源从 `ProviderAttemptWindow` 结算（无重试 attempts=Some(1)，失败/前奏时长仅重试时 Some(>0)）。
- **C6**：step 边界 drain-on-success 增量 flush；`FlushEvents.eventTimes` 逐事件时间戳；会话 `createdAt` 取最早事件时刻；崩溃后已 flush 事件可回读且 turn 未完成。

实现落点：`acp/{result,delegate_events,session}`、`agent/tools/orchestration`、`turn_flow::emit_external_turn_event/flush_step_boundary_events`、`control_plane` 缓冲带产生时刻、`sqlite_session::flush_events_tx(event_times_ms)`、`session/{log_command,log_sink,backend}`、`telemetry::ProviderCallRetryMeta`。

## Alternatives considered

- **把 live `TurnEventSink` 注入 `ToolExecutionContext`**：否决。sink trait 无 `Send+Sync` 上界（测试 sink 用 `RefCell`），会破坏并发工具与 41 处构造点；生产 sink 是 per-turn `Arc<dyn TurnEventSink + Send + Sync>`，ACP 深层拿不到。改用既有全局事件持久化通道，只要求落盘与父 turn 归属。
- **委派正文走 `turn:delta` 实时渲染到父会话**：否决/后置。会把子代理正文混入父 assistant 流（C1.2 禁止），实时 UI 属 B163 宿主面。
- **外部用量写 `usage_ledger`**：否决。外部 agent 计费口径与 ponyllm 不同源，混记污染成本账。
- **`step` 继续定义为事件序号**：否决。`StepStart/AssistantChunk/ProviderUsage.step` 已是 hop 语义，同名字段双语义使 (turn, step) 分组错位。
- **为 per-event 时间戳改造 `TurnEvent` 每个变体**：否决。schema churn 大且影响 fold/日志兼容；改为 `FlushEvents` 携带平行 `eventTimes`（additive，旧日志回落批首时间）。
- **继续整轮 flush，仅修时间戳**：否决。崩溃仍丢整轮（B209），无法满足 C6.3。

## Consequences

- 验收收据：冻结套件 `acp_delegate_p0p2_contract` 12/12、`acp_delegate_adversarial` 6/6、`acp_delegate_persistence_contract` 3/3；全量 lib 1614 passed/0 failed；ACP 19 套件全绿（`/tmp/full-lib-w2w3.log`、`/tmp/all-acp-final.log`）。
- 迁移：C3.1 介质变更使两条既有测试的 stderr 断言迁移为结构化 JSON 行断言（`acp_session_lifecycle`、`acp_cli_resume_and_epipe`）；L2-AT 用例做一次 test-defect rewrite（违例聚合，只增断言）。
- 残余风险：①同一 (session,turn) 并发 flush 仍有「快照→提交→移除」理论重复窗口（既有结构，step 边界抬高触发频次，留待 L3 复核）；②ACP 子活动 `step=0`（未知 hop），仅 `parent_activity_id` 可关联；③旧日志无 `eventTimes` 时历史时间不可回溯；④`finalText` 32k 上限与工具结果截断双保险，需观察 session blob 增量。
- 台账：backlog B237；契约矩阵 `.dev-team/contract-matrix-acp-delegate-p0p2.md`；L3/L3-P 审查 JSON 见 `.dev-team/review-l3*.json`。
