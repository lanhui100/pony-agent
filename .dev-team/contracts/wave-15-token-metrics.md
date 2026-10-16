# Wave-15 契约：Token 指标口径（输入上下文 / 生成速度）

- 基线：`3ba3c02`（桌面端 v0.1.146）
- 分级：**B 级**（跨 Rust 运行时 ↔ 前端投影协议语义；单特性域，无数据迁移/安全面）
- 用户裁定（2026-xx）：状态栏「输入」口径 = **当前上下文占用**（最近一次模型调用的 input_tokens），跨跳不累加。
- 证据链：`.dev-team/evidence/wave-15-root-cause.md`（3 路只读侦察 + file:line）

## 1. 症状（用户实测）

1. 输入 token 统计错误：状态栏与输入框上下文圆环数值虚高（多跳工具轮次把每次调用的整段 prompt 累加）。
2. 输出生成速度：不是每次模型调用完成即可用，而是等轮次结束（甚至无值）。

## 2. 根因（已锚定，file:line）

| # | 位置 | 机制 |
|---|---|---|
| RC1 | `crates/pony-agent-core/src/agent/runtime/turn_stream.rs:697-704` | `accumulated_token_usage = merge_token_usage(...)` 后 hop 事件携带**跨跳累加** input/output；`turn_duration_ms` 为整轮 elapsed |
| RC2 | `crates/pony-agent-core/src/agent/runtime/trace_timeline.rs:749-755` | 持久化 timeline 只把**整轮累加**值盖在**最后一条** `call_model` 条目上，中间条目 token 全 None |
| RC3 | `crates/pony-agent-core/src/agent/turn_flow.rs:1146-1152` | 无 `provider_call_records` 时回退构造「伪单次记录」，其 input = 整轮累加（legacy/graph 轮次） |
| RC4 | `src/lib/runtime/useTraceProjection.ts:144-186` + `src/stores/runtime.ts:3698` | 活跃轮 `liveTraceTurn` 无 `providerCallRecords`，回退到 store 全局累计值；hop_complete 把累加值写入 store |
| RC5 | `src/lib/runtime/useTraceProjection.ts:41-74` | 速度降级链会退到 turn 级累加 output / 整轮 duration；活跃轮 timeline 条目无 duration → 只能等轮次结束 |

## 3. 规范语义（冻结，不可就地变更）

- **S1 当前上下文输入** `currentContextInputTokens` := 会话中**最近一次已完成模型调用**的 `input_tokens`（活跃轮优先）。**严禁**跨 provider call 求和，**严禁**跨跳累加；跨轮自然继承（最新一次调用即当前上下文）。
  - 消费方：输入框上下文圆环分子 + tooltip；状态栏「输入」。
- **S2 会话累计计费输入** `sessionBilledInputTokens` := Σ provider call input_tokens（现有 `turn.inputTokens` 累加语义）。**保留在数据模型**（用于消息级/计费），**不得**再作为 S1 的显示来源。
- **S3 单次调用生成速度** `callGenerationSpeed` := `output_tokens / ((duration_ms − first_token_latency_ms)/1000)`；当 TTFT 不可信或 `duration_ms ≤ first_token_latency_ms` 时退化为 `output_tokens / (duration_ms/1000)`；`duration_ms <= 0` 或 `output_tokens == null` → 无值（显示 `—`）。
  - **必须在该次模型调用完成时即产出**（含轮内中间 hop，不等 `turn:completed`）；活跃轮同样可产出。
  - **严禁**用整轮累加 output_tokens 或整轮 elapsed duration 计算。
- **S4 降级纪律**：S1 的候选值若已知是多调用聚合值，**一律不得**作为 S1 显示；此时回退到「最后一条含 per-call 指标的 call_model 条目」，仍无则显示无数据（`—`），不得显示虚高数字。

## 4. 数据面要求（R）

- **R1（Rust·活跃轮）**：`turn:hop_complete` **与 `turn:output_end`** 必须携带已完成调用的 `provider_call_records`（当前均为 `None`，`turn_stream.rs:728` 与 output_end 发射处；语义 = 截至该事件的**全量 per-call 列表**，消费者取末条，严禁累加）。既有累加字段语义**不得变更**（wire 兼容）。
  - 依据（红相物理凭证 `.dev-team/evidence/wave15/wire-red.json`）：3 次模型调用只产生 **2 个 hop_complete**（仅"触发工具后续请求"的调用后发），**末次调用完成只发 `output_end`**；records 现仅在 `phase_changed`/`checkpoint_persisted`/`completed` 出现。仅修 hop_complete 会使末次调用的 per-call 值在活跃轮不可得。
- **R2（Rust·持久化）**：`build_persisted_trace_timeline` 必须让**每条** `call_model` 条目携带其**自身**的 input/output/first_token_latency/duration；整轮聚合只允许落在 `checkpoint_persist`（及轮级记录）上，**严禁**覆盖 per-call 条目。
- **R3（前端·活跃轮）**：`liveTraceTurn` 必须携带 `providerCallRecords`（per-call），使 S1 不再回退到 store 全局累计值。
- **R4（前端·store）**：活跃轮 store 的 `inputTokens` / `outputTokens` 语义改为「最近一次已完成调用」的 per-call 值（与 S1 一致）；轮终态 `turn:completed` / `turn:output_end` 的轮级累加字段保持原语义（写入消息级 `tokenCount` 与 `S2`）。
- **R5（兼容）**：`ProviderCallCacheRecord` 结构、`turn:completed`/`turn:output_end` 既有字段语义零变更；不得新增必需字段。

## 5. 验收标准（Test Agent 冻结，Executor 只读）

### L2-T 功能契约

- **T1**：3 跳已完成轮（per-call input 350K/360K/370K，turn 累加 1.08M）→ 上下文圆环分子 == 370K；状态栏「输入」 == 370K。
- **T2**：活跃轮（store.inputTokens 已被 hop_complete 置为累加 1.08M）→ S1 仍 == 370K（来自 live per-call 记录），不得显示 1.08M。
- **T3**：速度：最后一次调用 output=100 / duration=1200ms / TTFT=200ms → 100 t/s；同轮累加 output=1800、整轮 elapsed=5000ms 时不得得出 360 t/s 或 1800/((5000−200)/1000)。
- **T4**：活跃轮在**中间 hop 完成**（未收到 `turn:completed`）时即产出速度值（非 `—`）。
- **T5**：多调用轮且无任何 per-call 来源（records 空、timeline 无 per-call 指标）→ S1 不得等于累加值（应无数据）。
- **T6（R2）**：多跳轮持久化 timeline 中每条 `call_model` 条目的 input/output 为其自身调用值；`checkpoint_persist` 承载整轮聚合。

### L2-AT 对抗（红相纯度：raise 点必须落在 SUT 栈）

- **A1**：provider 无 usage（全 None）→ 无崩溃、无 NaN，显示 `—`。
- **A2**：畸形耗时（TTFT ≥ duration / duration=0 / duration 为 None）→ 无负数、无 Infinity/NaN，退化或空值。
- **A3**：重复/乱序 hop_complete（同 step 重放）→ per-call 值幂等稳定，不重复计数。
- **A4**：超大 token 值（u64::MAX 级别）→ saturating，无 panic/溢出。
- **A5**：伪单次聚合记录（RC3）+ 多调用 timeline → S1 不得返回聚合值（S4 纪律）。

### 物理门禁

- `cargo test`（受影响 crate）+ `npx vitest run` 目标用例 Exit 0；
- `npx vue-tsc --noEmit` Exit 0；
- **wire-replay 跨语言证据**：Rust 侧 mock SSE 多跳轮捕获真实 `TurnStreamEvent` JSON → 前端投影/渲染断言（物理收据落盘 `.dev-team/evidence/`）；
- 前端 UI 变更：真实浏览器 Console Error = 0 + 截图（不可行时显式标注 `[SMOKE DEGRADED]` 并说明降级依据）。

## 6. 非目标（Non-Goals）

- 不改 `S2` 计费口径（消息级 `tokenCount`、ModelMonitor/ponysentry 汇总保持累加）；
- 不改状态栏「输出 / 缓存读取」的会话累计口径；
- 不重构 trace 事件模型、不迁移历史数据、不新增依赖。

## 7. 契约演进 v2（recon 锚定）

> 本节由 Protocol Agent 依 Lead 裁定追加（唯一编写方），仅新增语义澄清与解析顺序，**不修改 §3–§5 任何既有条款**。

### 7.1 per-call 耗时物理字段锚定（禁止改名，Non-Goal）

单次调用的物理耗时字段是 `ProviderCallCacheRecord.turnDurationMs`（**命名历史误导，实为本次 HTTP 调用耗时，含 TTFT**）。
证据：`crates/pony-agent-core/src/agent/runtime/stream_support.rs:226,252`；`crates/pony-agent-core/src/agent/runtime/turn_stream.rs:676,689`。
本波**禁止改名**（Non-Goal §6），S3 一律以该字段作 `duration_ms`。

### 7.2 TTFT 可信门控（S3 强制）

单次调用 TTFT 仅当 `record.latencyKind === "provider_stream"` 时可信；否则按 `first_token_latency_ms = null` 处理（走 S3 退化式）。
前端既有门控仅覆盖 `src/components/HomeTracePanel.vue:609-615`；`src/lib/runtime/useTraceProjection.ts:54-57` **缺该门控** → 本波必须补齐。

### 7.3 `TraceTimelineEntry` per-call 指标现状与 R2 目标

`TraceTimelineEntry` **无** `durationMs` / `durationSeconds` 字段（per-call 耗时字段为 `turnDurationMs`，见 `src/types/runtime.ts:280-310`）。
Rust 侧中间 `call_model` 条目当前硬编码 `output_tokens=None` / `turn_duration_ms=None`（`crates/pony-agent-core/src/agent/runtime/trace_timeline.rs:412-422`、`:639-649`），仅**最后一条**被整轮聚合覆盖（`:732-756`，对应 RC2）。
R2 落地后：**每条** `call_model` 条目携带其自身 input/output/first_token_latency/duration。

### 7.4 事件折叠重建不得覆盖 per-call 值（R2 语义延伸，Lead 裁定）

`crates/pony-agent-core/src/agent/projection.rs:303-307` / `:569` 在 `TurnEnd` 折叠重建时把 per-call duration 覆盖为整轮 duration（历史 PA-094 P2-3）。
**Lead 裁定**：per-call 条目**不得**被整轮值覆盖；该裁定属于 **R2 的语义延伸**，落入本波实施范围，并作为 **T6** 的断言依据（多跳轮持久化/折叠重建后每条 `call_model` 仍为自身值，整轮聚合只落在 `checkpoint_persist` 与轮级记录）。

### 7.5 S1 判定补充（S4 纪律收窄）

仅当**轮内 provider call 数 ≤ 1** 时，轮级 `turn.inputTokens` 才是 S1 的**合法**回退（单调用场景下与 per-call 等价）；
调用数 **> 1** 时**严禁**使用任何聚合值作 S1，应返回 `null`（无数据 `—`，不得显示虚高数字）。

### 7.6 解析顺序（冻结）

`countTurnProviderCalls(turn)` := `max(turn.providerCallRecords?.length ?? 0, canonical call_model 条目数)`；`turn` 为 null/undefined 时按 0 计。canonical 条目 = `traceTimeline` 中 `kind === "call_model"` 的条目。

**S1 `resolveCurrentContextInputTokens(turn)`**：

1. `turn.providerCallRecords` 非空**且满足** `records.length > 1 || records.length === countTurnProviderCalls(turn)` → 取**最后一条**的 `inputTokens`（per-call 真值）。
   - 前置条件用于拦截 RC3「伪单次聚合记录」（无 records 轮次由 `turn_flow.rs:1146-1152` 回退构造 1 条记录、其 input = 整轮累加，而 timeline 有多条 call_model）；不满足时**不得**采信该记录，降级步骤 2。单调用轮（1 record + 1 条目）满足 `1 === count`，正常走步骤 1。
   - **Lead 裁定（采纳 Test Agent 提案 A）**：A5 双变体 = (a) 伪记录 + 3 条**无** per-call 指标的 timeline → `null`；(b) 伪记录 + 3 条带 per-call 指标（350K/360K/370K）的 timeline → `370_000`。
2. 否则取 `traceTimeline` 中**最后一条带 `inputTokens` 的 `call_model` 条目**的 `inputTokens`；
3. 否则若 `countTurnProviderCalls(turn) <= 1` → 允许回退 `turn.inputTokens`（单调用时与 per-call 等价）；
4. 否则返回 `null`（严禁放行聚合值）。

**S3 `resolveCallGenerationSpeed(...)`**：

1. 最后一条已完成 provider call record：`output = record.outputTokens`，`duration = record.turnDurationMs`（=本次调用耗时），TTFT 仅当 `record.latencyKind === "provider_stream"` 时参与抵扣（§7.2）；
2. 否则回退该轮**最后一条** `call_model` 条目（`outputTokens` + `turnDurationMs`；R2 落地后这些即 per-call 值）；
3. 否则若 `countTurnProviderCalls(turn) <= 1` → 允许回退 `turn.outputTokens` / `turn.turnDurationMs`；
4. 否则 `null`。
守卫：`duration <= 0` 或 `output == null` → `null`；返回前必须 `Number.isFinite`（禁 NaN / Infinity）。

### 7.7 既存测试锚点（必须保持全绿）

`tests/context-display-tokens.spec.ts` 三条断言为回归锚点，本波不得破坏：
`sessionInputTokensTotal === 1_080_000`（S2 累计值保留）；`contextDisplayTokens === 12_500`（单调用回退）；`sessionTokenGenerationSpeed === 100`（100 output / ((1200-200)/1000)s）。

### 7.8 空桩接口落点与纯度注记（`src/lib/runtime/token-metrics.ts`）

- `resolveCallGenerationSpeed(record)` 仅承载 **S3 步骤 1**（record 级 + 守卫）；**步骤 2–4** 的 timeline / turn 回退属调用方（`src/lib/runtime/useTraceProjection.ts`）职责，不在本任务空桩面。若 Lead 要求 turn 级 S3 解析器，需新增导出（新契约项，待 Lead 裁定后方可加入）。
- 空桩期参数名以 `_` 前缀（`_turn` / `_record`）以通过 `tsconfig.json` 的 `noUnusedParameters`；**位置签名、导出名与返回类型不变**，调用方按位置传参不受影响。
- `TraceTimelineEntry` 未在本模块导入：空桩期无消费者，导入会触发 `noUnusedLocals` 编译错误；S1 步骤 2 实施时由 Executor 直接从 `@/types/runtime` 导入（类型未新增、未变更）。

### 7.10 R1 事件载荷语义（Lead 裁定，2026-xx；修正 7.4 的滞后实现）

**不变量（冻结，机械可判）**：对任意携带 `provider_call_records` 的活跃轮事件（`turn:hop_complete` / `turn:output_end`），其载荷必须同时满足：

- **I1 集合一致**：`Σ records[i].input_tokens == payload.inputTokens`（records 是"截至该事件已完成的全部模型调用"，累加字段由同一批 usage 汇总而来，两者必须同源一致）；output 同理。
- **I2 末条即最新**：`records.last()` 就是**最近一次已完成**的模型调用（消费者取末条作为 S1/S3 的 per-call 真值）。
- **I3 有序前缀**：`records` 是"本轮全部调用记录"按调用时序的有序前缀，长度随 hop 单调不减（`len(records_k) == len(records_{k-1}) + 1`，且 `output_end` 的 records 长度 == 调用总数）。

**事实与修正**：`turn_stream.rs` 的 followup 循环中，当前 hop 的模型调用在其记录 `push` 之后才 `emit hop_complete`（push 在 :679、merge 在 :697、emit 在 :705），因此 `hop_complete` 必须携带 **`provider_call_records.clone()` 全量列表**；执行者先前提交的 `provider_call_records[..len-1]`（排除末条）造成**滞后一跳**，违反 I1（红相捕获：hop0 累加 710000 而 records 仅 350000）。

**映射（供测试断言）**：`turn:hop_complete` #k（0-based）在 followup step k 结束时发出，其 `records.last()` = 第 k+1 次调用（0-based 索引 k+1，即 `hopInputs[k+1]`）；`turn:output_end` 的 `records.last()` = 末次调用。

**残余延迟（已知限制，不扩范围）**：初始调用（call #0）完成时无 hop_complete 事件，故其 per-call 值在首个 followup 完成前不会成为"最新已完成调用"的展示值（此窗口内展示上一轮的最近调用值）。登记为 P3，不在本波修复。
