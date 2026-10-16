# Wave-15 根因证据（Lead 汇总，3 路只读侦察 + Lead 复核）

- 基线 commit：`3ba3c02`（桌面端 v0.1.146）
- 侦察方式：3 个独立只读 Subagent（Rust 输入链路 / Rust 输出速度链路 / 前端显示链路），全部 `file:line` 锚定 + Lead 亲自复核关键行
- 契约：`.dev-team/contracts/wave-15-token-metrics.md`

## 一、输入 token 链路（RC1-RC4）

| 锚点 | 事实 |
|---|---|
| `crates/pony-agent-core/src/agent/runtime/stream_support.rs:267-291` | `merge_token_usage` = `add_optional_u64` **求和**（saturating） |
| `crates/pony-agent-core/src/agent/runtime/turn_stream.rs:697-704` | 每跳后 `accumulated_token_usage = merge_token_usage(...)`，`turn:hop_complete` 携带**跨跳累加** input/output 与整轮 elapsed |
| `turn_stream.rs:723-728` | hop_complete 的 `provider_call_records` 槽位传 `None`（per-call 记录当跳已在 :679 构造，但不下发） |
| `crates/pony-agent-core/src/agent/turn_flow.rs:1146-1152` | 无 records 时回退构造一条「伪单次」ProviderUsage，其 input = 整轮累加（legacy/graph 轮次） |
| `crates/pony-agent-core/src/agent/projection.rs:544-551, 687-688` | `merge_tokens` 求和 → `trace.input_tokens` = 轮内 Σ provider call |
| `projection.rs:303-307` | TurnEnd 结算把 `entry.turn_duration_ms` 覆盖为整轮 duration（per-call 值被覆盖） |
| `crates/pony-agent-core/src/agent/runtime/trace_timeline.rs:639-649` | 持久化 timeline 中间 call_model 条目 `input/output/turn_duration = None` |
| `trace_timeline.rs:732-756` | 仅**最后一条** call_model 被盖上整轮聚合值（`input_tokens = input_tokens` 即轮级） |
| `src/lib/runtime/useTraceProjection.ts:203-223` | `extractPhysicalContextTokens`：records → timeline 最后一条含 token 的 call_model → `turn.inputTokens`（聚合） |
| `useTraceProjection.ts:144-183` | 活跃轮 `liveTraceTurn` **无** `providerCallRecords` / `turnDurationMs` |
| `src/stores/runtime.ts:3688-3707` | `turn:hop_complete` 把**累加值**写进 store `inputTokens/outputTokens`，并忽略 `turnDurationMs` |
| `src/stores/runtime.ts:3899,4005` | per-call records 仅在终态事件写入 |
| `useTraceProjection.ts:289-291` | 状态栏「输入」= Σ 各轮 `turn.inputTokens`（每项已是跨跳累加） |

**症状因果**：多跳轮（350K/360K/370K，轮累计 1.08M）
- 输入框圆环：活跃轮无 per-call 数据 → 回退到 store 累加值 1.08M（>100% 窗口）；轮终态因 records 到位才回落，某些 records-less 轮次（legacy/graph/回退单条）**永久虚高**；
- 状态栏「输入」：Σ 轮级累加 → 同样的每次调用累加语义。

既存 `tests/context-display-tokens.spec.ts` 之所以"通过"：其夹具手写了 Rust **从不产出** 的逐条 per-call 时间线 token，故未覆盖真实形态（复核确认 `trace_timeline.rs:749` 会把整轮值写在最后一条上）。

## 二、输出速度链路（RC5）

| 锚点 | 事实 |
|---|---|
| `turn_stream.rs:521/676/679-696` | 每次 provider 调用都有独立耗时 `provider_call_duration_ms`（整次调用含 TTFT）与 per-call usage |
| `stream_support.rs:226/252` | per-call 耗时被存入 `ProviderCallCacheRecord.turn_duration_ms`（字段名误导，实为单次调用耗时） |
| `turn_stream.rs:684-694` | TTFT 仅 `latency_kind == provider_stream` 时可信 |
| `turn_stream.rs:913 continue;` / `1029,1060,1201` | 中间 hop 不发射 records；records 只在 checkpoint/终态事件下发 |
| `trace_timeline.rs:412-422, 639-649, 732-756` | 进度/持久化 timeline 条目无 per-call 耗时（中间条目硬编码 None，最后一条被盖整轮） |
| `useTraceProjection.ts:41-74` | `entryTokenGenerationSpeed` 依赖 entry/turn `turnDurationMs`；`turnTokenSpeed` 依赖轮级累加 |
| `useTraceProjection.ts:312-356` | 活跃轮分支要求 `entry.state === "completed"` 且带耗时 → 实际恒 null，随后 `continue` 落到**上一轮**速度 |
| `src/stores/runtime.ts`（0 处 `this.turnDurationMs`） | store 无轮时长状态；`src/types/runtime.ts:344-360` 的 `ProviderCallCacheRecord.turnDurationMs`（per-call）未被速度路径消费 |

**症状因果**：活跃轮永远算不出速度（无 per-call duration 下发 + 活跃轮分支刻意跳过），只有轮终态/下一轮才可能出现数字；多跳轮即便有数字也是「整轮 output ÷ 整轮 elapsed」。

## 三、文档化不变量（约束本波设计）

- `docs/decisions/0008-event-sourcing-evolution.md:68`：「totals 只累计 `provider/usage`，`assistant/message` usage 仅作消息元数据展示（避免双计数）」。
- 同文件 `:76`：`MetricsProjection` 明确区分 **totals + last + per-turn 聚合（addReplacing）** → "last（单次调用）"与"totals（累计）"是两个语义，UI 现把两者混用。
- `docs/analysis/trace-analysis-pa-01a12a3f.md:69-70`：上下文窗口占用应取"最后一次物理模型调用的 inputTokens，消除累加虚高破百"（本波 S1 的规范意图来源；当时以为已闭环，实际未覆盖活跃轮/records-less 路径）。
- `openspec/changes/archive/2026-08-22-workspace-shell-layout-optimization/design.md:59-64` 与 `useTraceProjection.ts:19-20` 的注释（HomeSidebar `liveTurnEnabled=false`）已与实际 `HomeSidebar.vue:47`（true）漂移——本波不改该可见性契约，仅记录。

## 四、既有测试盲区（Test Agent 必须补）

- `tests/context-display-tokens.spec.ts`：3 条全绿但与生产形态不符（见上）。
- 全仓库**没有任何** `turn:hop_complete` 相关测试；**没有任何**活跃轮（`isSubmitting: true`）速度断言。
- `runtime/tests.rs:6346` 是唯一多跳 token 用例，但只断言轮级累加与 records 数量，不断言 per-hop 时间线指标。
- `tests/TraceInspector.spec.ts:1224` 已有 per-call 速度显示断言（records 存在、已完成轮），方向一致，作为回归锚点保留。
