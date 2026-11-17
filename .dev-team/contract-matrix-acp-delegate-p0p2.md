# 契约矩阵：ACP 委派缺陷修复 P0–P2（wave-acp-delegate）

- 分支：`fix/acp-delegate-p0p2`（worktree `/home/dm/pony-agent-worktrees/acp-delegate-p0p2`，基线 `c5761b27`）
- 现场：会话 `pa_01a1e8fd-…` 首轮 `acp_delegate` 317.556s / updates=111 / 结果仅硬编码 summary；Antigravity 会话 `1c2c45fa-…` step20 实产 ~4.8KB 清单。
- 变更类型：additive（结果 JSON 新键、trace 元数据新字段、日志结构化、事件时间戳字段）；不删字段、不改 ACP 线协议。
- 设计裁决：不使用"把 live sink 塞进 ToolExecutionContext"方案（sink 无 Send+Sync 上界，且 registry 生产恒单槽）。改用**父 turn 事件持久化通道**：ACP 子活动经 `turn_flow` 新助手函数以父 session/turn id 发 `turn:tool` 并落盘；live UI 渲染不在本期范围（Non-Goal 4）。

## 滚动契约账本（活跃条目）

| ID | 契约陈述（可机械断言） | 用例 | 级别 |
|---|---|---|---|
| C0.1 | `acp_delegate` 成功结果 JSON 必含 `finalText`（AgentMessageChunk 文本按到达顺序拼接）与 `finalTextTruncated: bool`；保留 `ok/sessionId/stopReason/updates/warnings/summary`。 | T-ACP-01 | P0 |
| C0.2 | 无 AgentMessageChunk ⇒ `finalText=""`、`finalTextTruncated=false`，`summary.text` 明示"未返回文本正文"，不得伪装内容。 | T-ACP-02 | P0 |
| C0.3 | 正文 > 32_000 字符 ⇒ 保留前缀 + `finalTextTruncated=true`；上限为单一常量真源。 | T-ACP-03 | P0 |
| C0.4 | `truncated`（max_tokens/max_turn_requests）归约保持 `truncated:true` 且**回传 finalText**（`finalTextTruncated=true`）；refused/cancelled/error 行为不变且不出现 `finalText`。 | T-ACP-04 | P0 |
| C0.5 | 工具描述（模型可见）声明结果含 `finalText` 正文；与实际行为一致。 | T-ACP-05 | P0 |
| C1.1 | 委派期间 ACP `tool_call`/`tool_call_update` 归集为父 turn `turn:tool` 事件：`parent_activity_id == acp_delegate call_id`（`TurnEvent::ToolCall/ToolResult` 增 additive 字段承载），事件 session_id/turn_id = 父会话/父 turn，并落入 `turn_events`。 | T-VIS-01 | P1 |
| C1.2 | 外部 agent 文本 chunk 不得混入父 assistant 正文流（父 `assistant_message` 不含其内容）。 | T-VIS-02 | P1 |
| C1.3 | 无父 session/turn 上下文或持久化通道未注册时，归集静默跳过（返回 false，不 panic、不新增必填构造参数）。 | T-VIS-03 | P1 |
| C1.4 | ACP 状态词表映射：`pending|in_progress→running`、`completed→done`、`failed→failed`、`cancelled→aborted`，确保父 turn 事件被正确判为 ToolCall/ToolResult。 | T-VIS-04 | P1 |
| C1.5 | 归集事件的 `turn_id` 不得为 ACP session id（必须改写为父 turn id）；`parent_activity_id` 不得为空。 | T-VIS-05 | P1 |
| C2.1 | ACP `usage_update` 的 used/size（及可选 cost）进入结果 JSON `usage` 字段并写结构化日志；不写 `usage_ledger`（Non-Goal 3）。 | T-USE-01 | P1 |
| C3.1 | `crates/pony-agent-core/src/acp/**` 非测试代码零裸 `eprintln!`，诊断走 `agent::otel::log`（json + trace_id）；CLI `acp` 在 jsonl/sse 下日志改道 stderr（遵守 wave-59 C1 stdout 纯净）。 | T-LOG-01 | P1 |
| C4.1 | `TurnEvent::ToolCall/ToolResult.step` = 产生该调用的 turn step index（与 `StepStart/AssistantChunk.step` 同源），不得用事件 sequence；ACP 子活动 step=0（未知 hop，None≡0）。 | T-STEP-01 | P2 |
| C5.1 | `ProviderCallCacheRecord` additive 字段：`attempts` 恒 Some(≥1)；`failed_attempt_ms`/`prelude_ms` 仅当发生失败重试时 Some(>0)，否则键缺席；可经 `session_turn_traces.providerCallRecords` 回读。 | T-RETRY-01 | P2 |
| C6.1 | 每 step 结束增量落盘：截至该 step 的事件已在 `turn_events` 可读，终态仍写 `turn_end`；同事件不重复。 | T-PERSIST-01 | P2 |
| C6.2 | 事件 `created_at_ms` = 事件真实产生时间（同批不得全部塌缩到 flush 时刻）；会话 `createdAt` = 会话创建时刻（≠首轮 flush 时刻）。 | T-PERSIST-02 | P2 |
| C6.3 | 模拟崩溃（无 turn_end 即丢弃 store）后重载：已 flush 事件可读、turn 呈未完成/中断，不得整轮 0 行。 | T-PERSIST-03 | P2 |

## 验收执行口径（Lead 冻结）

- 机器门禁：`bash scripts/cargo-lane.sh test -p pony-agent-core --test <suite>` Exit 0，逐套件列 Exit Code；全量回归 `--lib` + ACP 全套。
- 红相：断言失败（非编译错误/环境缺失）；Lead 锚定 `test(wave-acp-delegate): freeze red-phase acceptance tests`。
- 可观测性：`audit-diff.sh --base c5761b27` + `check-declaration.sh`（本仓有 docs/observability.yaml）。
- 交付面：worktree 分支提交；不合并脏主工作区；交付说明单列合并方式。

## Non-Goals

1. 不做 ACP `session/load` 恢复、Tauri 三命令/前端 ACP 面板（B163）。
2. 不改 ACP v1 线协议；`finalText` 是本侧 ToolResult 契约。
3. 不把外部 agent 用量写入 `usage_ledger`（计费口径不同源）。
4. 不做逐 token 实时 UI 渲染（live sink 注入不在本期）；子活动只要求落盘可审计。
5. 不重构无关 TUI/状态机；不清理存量非关联死代码。

## DAG 与波次（写域正交）

- W1（冻结）：Protocol Agent（NFR 基线 + 空桩/字段声明）；Test Agent（验收 + L2-AT）；Lead 红相锚定。
- W2（并行）：
  - W2a Executor-A：`src/acp/` + `src/agent/tools/`（C0/C1 发射端/C2/C3）
  - W2b Executor-B：`src/agent/runtime/` + `turn_flow.rs` + `turn_flow/` + `turn_event.rs` + `telemetry.rs` + `provider/`（C1 助手函数/C4/C5）
- W3（串行，依赖 W2 barrier）：Executor-C：`sqlite_session*.rs` + `session/` + `control_plane/` + `runtime/turn_persist.rs` + `turn_flow*` + `log_*`（C6）
- W4：Join Barrier（写域审计/blast-radius/NFR 比对/review lint）+ L3/L3-P + 冒烟 + 收口。

## 变更流水（单行追加）

- 2026-11-17 初始冻结：C0.1–C6.3 共 15 条；Non-Goals 5 条；W1–W4 DAG。
- 2026-11-17 修订：C1 改为父 turn 持久化通道（放弃 sink 注入）；C2 去掉 span 硬要求（结果 JSON + 结构化日志）；新增 C0.5/C1.4/C1.5。
- 2026-11-17 裁决：C1.1 允许 TurnEvent 增 parent_activity_id additive 字段（Test 严格断言）；C1.4 允许 ToolCallStatus::Cancelled additive；C5.1 改为 attempts 恒 Some、failed/prelude 仅重试时 Some。L2-AT 独立池落 tests/acp_delegate_adversarial.rs。
