# Agent Note: ACP 委派结果正文与 trace 完整性修复

Status: proposed

## Problem

真实会话 `pa_01a1e8fd-a6a3-7543-91ef-ae18397fa858` 首轮把任务委派给 Antigravity（`acp_delegate`，317.556s，`updates=111`）：外部 agent 实际产出了约 4.8KB 的 backlog 清单（活跃 18 条 / 归档 155 条 / ID 高水位 B231 / 依赖列 / 退出码证据，见 Antigravity 会话 `1c2c45fa-…` step 20），但 Pony 侧 `ToolResult` 只有硬编码 `summary.text="外部 agent 已完成本轮任务。"`，正文零留存；父模型被迫本地重查（多 2 次 followup、input 66,960 tokens），最终答案漏掉外部 agent 的一手结论。

根因是四个层面的完整性缺口，而非单点笔误：

1. **结果面**：`AcpAgentSession::prompt` 收集了全部 `SessionUpdate`，但 `build_ok_result` 只使用计数；`run_delegate_prompt` 传 `NoopTurnEventSink`，正文既未回传模型、也未进事件流。
2. **可观测面**：委派期间的工具活动无法归集到父 turn（`ToolExecutionContext` 无 sink 注入点；ACP `base_event` 用 ACP session id 且绕过 `emit_event`，即使有 sink 也落错桶）；`UsageUpdate` 只走裸 `eprintln`，用量/成本不可审计。
3. **trace 面**：`build_tool_event` 用 `payload.sequence` 当 `step`，与 `StepStart/AssistantChunk.step` 的 hop 语义冲突（现场 `step=7/8、21/22、27/28` vs `step_start=0..3`）；provider 重试窗口只在运行日志里，`providerCallRecords` 只留成功调用（turn TTFT 25.7s vs 单次调用 5.36s 无法解释）。
4. **持久化面**：整轮事件只在轮末 flush（`is_terminal`），批首取一次 `now` 写全部 `created_at_ms`，会话 `createdAt` 退化为首轮结束时刻；崩溃即丢整轮（与 backlog B209 同源）。

## Proposal

以 additive 契约修复上述四面，契约矩阵与验收口径落 `.dev-team/contract-matrix-acp-delegate-p0p2.md`（C0.1–C6.3）：

- **C0（P0）**：成功结果 JSON 增 `finalText`/`finalTextTruncated`；`AgentMessageChunk` 按到达顺序拼接，32,000 字符上限（单一常量）；空正文明确提示；非 ok 路径不变。
- **C1（P1）**：外部 agent 的 `tool_call`/`tool_call_update` 以父 session/turn id 归集为父 turn `turn:tool` 事件（`parent_activity_id=acp_delegate call_id`，状态词表映射、turn_id 改写），经 `turn_flow::emit_external_turn_event` 走既有持久化通道；无通道/无父上下文静默跳过；文本 chunk 不混入父 assistant 流。
- **C2（P1）**：`usage_update` 的 used/size/cost 进入结果 JSON 与结构化日志（不写 `usage_ledger`，计费口径不同源）。
- **C3（P1）**：`src/acp/**` 诊断从裸 `eprintln!` 迁移到 `agent::otel::log`（json + trace_id）。
- **C4（P2）**：`ToolCall/ToolResult.step` 取产生该调用的 turn step index（`payload.step`，None≡0），不再用事件 sequence。
- **C5（P2）**：`ProviderCallCacheRecord` additive `attempts`/`failed_attempt_ms`/`prelude_ms`；无重试 attempts=1 且另两键缺席，重试时 >1 且 >0，并随 trace 持久化。
- **C6（P2）**：step 边界增量 flush（drain-on-success，事件不重复）、per-event 真实时间戳、会话 `createdAt` 为创建时刻；崩溃后可读到已 flush 事件。

## Alternatives considered

- **把 live `TurnEventSink` 注入 `ToolExecutionContext`**：否决。sink trait 无 `Send+Sync` 上界（测试 sink 用 `RefCell`），加进上下文会破坏并发工具与 41 处构造点；且生产 sink 注册是 per-turn 的 `Arc<dyn TurnEventSink + Send + Sync>`，ACP 深层拿不到。改用既有全局事件持久化通道，只要求落盘与父 turn 归属。
- **委派正文走 `turn:delta` 实时渲染到父会话**：否决/后置。会把子代理正文混入父 assistant 文本流（C1.2 明确禁止），实时 UI 属 B163 宿主面（Non-Goal 4）。
- **外部用量写 `usage_ledger`**：否决。外部 agent 计费口径与 ponyllm 不同源，混记会污染成本账；只做结果/trace/日志可见。
- **把 `step` 继续定义为事件序号**：否决。`StepStart/AssistantChunk/ProviderUsage.step` 已是 hop 语义，同名字段双语义使按 (turn, step) 分组的投影/审计错位。
- **为 per-event 时间戳改造 `TurnEvent` 每个变体**：否决。schema churn 大且影响 fold/日志兼容；改为 `FlushEvents` 携带平行 `eventTimes`（additive，旧日志缺省回落批首时间）。
- **继续整轮 flush，仅修时间戳**：否决。崩溃仍丢整轮（B209 现场），无法满足 C6.3 的崩溃可读要求。

## Acceptance criteria

- 冻结测试 `crates/pony-agent-core/tests/acp_delegate_p0p2_contract.rs`、`acp_delegate_persistence_contract.rs`、`acp_delegate_adversarial.rs` 全绿（红相收据见 `.dev-team/red-phase/*.receipt.json`）。
- 全量回归：`bash scripts/cargo-lane.sh test -p pony-agent-core --lib` 与 ACP 全套 Exit 0；基线 1607 passed/0 failed。
- 可观测性门禁：`audit-diff.sh --base c5761b27` 与 `check-declaration.sh` 通过（`docs/observability.yaml` 同步归属）。
- 现场复验：以 fake agent 复现"外部正文回传 + 子活动落盘 + usage 可见"，并提供 stdout/Exit Code 收据。

## Risks

- C6 触及 checkpoint watermark 与日志命令流：`event_seq_range` 回填与增量 flush 的交互需回归（既有 `sparse_session_store`/`sqlite_session`/`fold` 套件）。
- C1 子活动以 `step=0` 记录（未知 hop），仅在 `parent_activity_id` 维度可关联；若未来需要 hop 精确，需在 `DispatchContext` 增 `turn_step`。
- `finalText` 增大 tool result 体积：32k 字符上限 + 既有工具结果截断双保险，需观察 session blob 增量。
- 旧日志无 `eventTimes`：迁移期回落批首时间，历史时间塌缩数据不可回溯（只保证新数据）。
