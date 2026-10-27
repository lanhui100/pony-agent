# Wave-54 契约矩阵（token / 速度口径真值与对账门禁）

> 驱动事实：真实 trace `eed9ba47…`（20 次 llm.chat，98.47s）+ 本地 `sessions.db` 复算 +
> 三路审计（Tempo 实测 / 前端口径链 / Rust 物理口径）。见 `docs/audit/observability-audit-2026-10-27b.md`。
> 本文件由 Lead 冻结即只读；实施期任何语义变更走契约演进门禁（新波次）。

## 0. 分级与门禁

- 分级：**B 级**（跨 Rust/TS 多模块，触及 LLM 调用链与对外语义属性、UI 显示契约）。
- 可观测性动线：**命中**（改的是 tracing 属性面 + UI 交互）⇒ 验收追加
  `audit-diff.sh --base <基线>` 与 `check-declaration.sh /home/dm/pony-agent`，均须 Exit 0。
- 回归基线：`.dev-team/regression-baseline-wave54.json`（改动前 `vitest run` 与
  `cargo test -p pony-agent-core --lib` 机器收据）。
- 不在本波：成本计价、Langfuse trace 级聚合、usage_ledger 语义（均保持现状）。

## 1. 冻结契约

### C1 per-call span 真值键（`llm.chat`）
- 新常量：`ATTR_LLM_CALL_DURATION_MS="llm.call.duration_ms"`、
  `ATTR_LLM_CALL_TTFT_MS="llm.call.ttft_ms"`、
  `ATTR_LLM_CALL_GENERATION_MS="llm.call.generation_ms"`、
  `ATTR_AGENT_USAGE_ORIGIN="agent.usage.origin"`。
- `llm.call.duration_ms`：i64，= 该次 provider 调用耗时（与 `ProviderCallCacheRecord.turnDurationMs` 同窗口）。
- `llm.call.ttft_ms`：i64，**仅** `Some` 时输出（buffered/无首包 ⇒ 键不存在）。
- `llm.call.generation_ms`：i64，= `duration - ttft`（当且仅当 ttft 存在且 `duration > ttft`），否则 `= duration`。
- `agent.usage.origin`：字符串，`provider` | `estimated_chars`；未记录 usage ⇒ 键不存在。
- 键不存在即「未知」，**严禁**写 0 占位。

### C2 速度唯一口径（turn 加权生成速度）
- 定义：`gen_i = (ttft_i 存在 且 dur_i > ttft_i) ? dur_i - ttft_i : dur_i`；`out_i = output_tokens_i`。
- `turn.speed = Σout_i / (Σgen_i / 1000)`，仅当 `Σgen_i > 0` 且 `Σout_i > 0`；否则无值。
- 根 span：`agent.turn.speed.tokens_per_second`（f64，>0 才输出）、
  `agent.turn.speed.basis="turn_weighted"`、`agent.turn.duration_ms`、`agent.turn.ttft_ms`（= 首个
  带 ttft 的 per-call 值，不跨 hop 采样）、`agent.turn.calls`（已完成调用数）。
- `agent.turn.ttft_ms` 的语义是「首次模型调用首包」，**不是** turn elapsed 内首包（turn 级 TTFT 旧语义作废）。
- 前端：`sessionTokenGenerationSpeed` 一律用上式；`TokenSpeedCalculationMode` 收敛为
  `"turn_weighted" | "call_only" | "none"`（`call_only` = 无已完成调用时用当前活跃调用的 per-call 值）。
  删除 `call_active_stream` / `call_fallback` / `turn_completed_aggregate` 三分叉。
- 既有 `gen_ai.speed.tokens_per_second`（per-call）语义不变，但**必须**等于 `out_i/gen_i`（同一公式）。

### C3 usage provenance（估算值不得冒充真值）
- `TokenUsage` 新增 `origin: Option<&'static str>`；provider 解析路径一律 `Some("provider")`，
  `estimate_token_usage` 一律 `Some("estimated_chars")`。
- 透传字段：`ProviderCallCacheRecord.usage_origin`、`TurnTraceRecord.usage_origin`（camelCase
  `usageOrigin`，`Option<String>`，缺失不输出）。
- UI：`estimated_chars` 时数字带 `≈` 前缀且 tooltip 说明；不得静默当真值。

### C4 上下文占用口径（协议归一）
- 新字段 `ProviderCallCacheRecord.context_input_tokens` / `TurnTraceRecord.context_input_tokens`
  （camelCase `contextInputTokens`）：
  - `protocol_label` 以 `anthropic` 开头 ⇒ `input + cache_read + cache_creation`（anthropic `input` 既不含
    cache_read 也不含 cache_creation；`cache_creation` 仅进上下文口径，不进 `cache_read` 上报）；
  - 其余协议 ⇒ `input`（已含缓存）。
- UI「输入」位取值序：`contextInputTokens` → `resolveCurrentContextInputTokens()` → `—`。
- `gen_ai.usage.context_window` / `context_ratio`：4 个 `record_speed_and_context` 调用点必须传
  真实 `provider.context_window_tokens()`；无窗口能力 ⇒ 不输出两键（禁止 0）。

### C5 projection §7.4 落地（per-call 不得被整轮覆盖）
- `settle_timeline`：仅当 `entry.turn_duration_ms.is_none()` 时补 turn 值；已有 per-call 值保持不动。
- `MetricsProjection` 的轮级 `first_token_latency_ms` 改为 **first-wins**（与 `TraceProjection` 一致），
  后到的 `None` 不得冲刷已有值。
- 运行态产物（`trace_timeline.rs`）与折叠产物（`projection.rs`）对同一输入必须给出相同 per-call 值。

### C6 显示纪律（`HomeStatusPanel`）
- 无数据一律 `—`（禁止 `0`）；`contextInputTokens`/`output`/`cache` 同法。
- 紧凑格式：`<1000` 原值；`<1e6` 一位小数 `1.9K`；`≥1e6` 一位小数 `1.2M`。
- 缓存比 = 最近一次调用 `cache / context_input_tokens`（与可见「输入」同源），整数百分比。
- 速度位：`data-speed-basis` 输出 basis；tooltip 明确「加权生成速度（N 次调用）」或「当前调用」。
- `data-speed-debug` 扩展为 `basis=…;calls=…;in=…;out=…;dur=…;ttft=…;gen=…;win=…;ratio=…;origin=…`。

## 2. 验收用例（Test Agent 独占写域，冻结后只读）

- **T1** `llm.chat` 四键：stream 调用 ⇒ duration/ttft/generation/origin 同时存在且
  `duration == ttft + generation`（仅当 ttft 存在且 `duration > ttft`；否则 `generation == duration`）；buffered 调用 ⇒ 无 ttft 键、generation == duration。
- **T2** 根 span：`agent.turn.speed.tokens_per_second == Σout/Σgen`（±1e-6）、`basis=turn_weighted`、
  duration/ttft/calls 存在；`gen_ai.usage.context_window/context_ratio` 在有窗口时存在且 ratio≈in/win。
- **T3** 折叠不覆盖：`fold_all::<TraceProjectionState>` 给 step0(dur=500)、step1(无 dur) + TurnEnd(1200)
  ⇒ step0 == 500，step1 == 1200；`MetricsProjection` 的轮级 ttft 首次值不被后续 None 冲刷。
- **T4** 前端单口径：两调用 (out 100, dur 1200, ttft 200) + (out 200, dur 1800, ttft=None)
  ⇒ speed = 300 / ((1000+1800)/1000) = 107.14（±0.01），mode=`turn_weighted`。
- **T5** 显示纪律：null ⇒ `—`；1875 ⇒ `1.9K`；缓存比与可见输入同源；`origin=estimated_chars` ⇒ `≈`。
- **T6** golden replay：以真实 trace `eed9ba47…` 的 20 次调用（`durationMs/outputTokens/inputTokens/
  cacheReadTokens/reasoningTokens/genMs/ttftMs` 见 `tests/acceptance/wave54/fixtures/eed9ba47-calls.json`，
  Σout=4542、Σin=358633、Σcache=327808、Σreason=2075、**Σgen=47580ms**）为输入 ⇒
  `turn.speed = 4542/47.58 = 95.46`（±1.0 t/s）、`basis="turn_weighted"`、根 `agent.turn.duration_ms=98469`、
  `agent.turn.ttft_ms=3047`、`agent.turn.calls=20`。
  **口径区分（必须写进用例注释）**：95.46 是「生成速度」（Σgen 不含 TTFT）；端到端吞吐
  Σout/整轮时长 = 4542/98.47 = 46.13 属另一指标，本波**不**作为 turn.speed 输出。
  另：`gen_i = out_i/attrSpeed_i` 由 span 实测反推，容差按毫秒取整。

**L2-AT（对抗）**：A1 `dur<=0` / 全空 usage ⇒ 不得产出 NaN/Inf/负值且 speed 必须为 `null`；
`ttft>dur`（Lead 裁决 2026-10-27 20:40）⇒ **按 C2 退化 `gen=dur`**，断言 speed=`out/dur`（有限正值，
与已冻结的 `tests/unit/token-speed-metrics.spec.ts:101` 一致），**不得**判 null；
A2 `estimated_chars` 与 provider 混合 ⇒ 加权限仍有限且方向不变；A3 同一事件序列重复 fold 10 次幂等
（固定序列、无随机、无 sleep）。

## 3. 写域（正交，禁跨界）

| 角色 | write_scopes |
| :--- | :--- |
| protocol | `.dev-team/nfr-baseline-wave54.json`, `crates/pony-agent-core/src/agent/otel/mod.rs`（仅常量+签名桩）, `src/types/runtime.ts`（仅类型字段） |
| test | `tests/acceptance/wave54/`, `crates/pony-agent-core/tests/wave54_metric_truth.rs` |
| rust-exec | `crates/pony-agent-core/src/agent/otel/mod.rs`, `turn_flow.rs`, `projection.rs`, `runtime/stream_support.rs`, `provider/mod.rs`, `session/types.rs` |
| ui-exec | `src/lib/runtime/token-metrics.ts`, `src/lib/runtime/useTraceProjection.ts`, `src/components/HomeStatusPanel.vue`, `src/components/HomeSidebar.vue` |
| lead | `.dev-team/contract-matrix-wave54.md`, `docs/observability.yaml`, `docs/decisions/`, `docs/audit/`, `.dev-team/state.json` |

## 4. C2 前端 API 冻结（实施前补充，Lead 2026-10-27 同一波次裁定）

`src/lib/runtime/token-metrics.ts` 必须导出（Named export，位置参数不变，旧导出可保留为薄别名）：

```ts
export type TokenSpeedCalculationMode = "turn_weighted" | "call_only" | "none";

export interface TurnWeightedSpeedDetail {
  speed: number | null;
  mode: TokenSpeedCalculationMode;
  basis: "turn_weighted" | "call_only" | "none";
  calls: number;            // 参与计算的已完成调用数
  outputTokens: number | null;  // Σout
  genMs: number | null;         // Σgen
  inputTokens: number | null;   // 末次调用上下文输入（见 C4）
  cacheInputTokens: number | null;
  ttftMs: number | null;        // 首次调用 TTFT
  origin: string | null;        // estimated_chars | provider | null
}

export function resolveTurnWeightedSpeedDetail(
  turn: TurnTraceRecord | null | undefined,
  options?: { isActiveTurn?: boolean }
): TurnWeightedSpeedDetail;
```

- `turn_weighted`：`calls >= 1` 且 `genMs > 0 && Σout > 0`。
- `call_only`：`calls === 0` 且存在活跃调用的 per-call 值（`speed = out/(dur-ttft|dur)`）。
- `none`：其余（`speed = null`）。
- 旧的 `resolveDualModeTokenSpeedDetail` / `resolveTurnGenerationSpeed` 若保留，必须委托本函数，不得再有自己的分支。

### 3.1 编译兼容衍生豁免（Lead 2026-10-27 20:58 追认）

C3/C4 给 `TokenUsage` / `ProviderCallCacheRecord` / `TurnTraceRecord` 增字段后，所有全字段结构体字面量
必须同步补初始化（Rust 无 FRU 时逐点补行）。以下文件纳入**衍生写域**，变更仅限「字段初始化 / 传参」，
严禁改逻辑、改断言、改判定：

`crates/pony-agent-core/src/agent/telemetry.rs`、`provider/responses_api.rs`、
`runtime/tool_recovery.rs`、`runtime/turn_stream.rs`、`runtime/turn_sync.rs`、`runtime/turn_persist.rs`、
`session/store.rs`、`session/tests.rs`、`turn_event.rs`、`sqlite_session.rs`、`control_plane/mod.rs`、
`crates/pony-agent-core/tests/otel_trace_migration_test.rs`、
`crates/pony-agent-core/tests/observability_reporting_fixes_test.rs`。

`crates/pony-agent-core/tests/wave54_metric_truth.rs` **不在**衍生豁免内（冻结只读）。

## 5. 红相期歧义裁决（Lead 2026-10-27 21:10）

1. **`detail.inputTokens` 语义**：恒取**末次已完成调用**的 `contextInputTokens`（C4 口径归一值；
   缺失时回退该记录的 per-call `inputTokens`），多调用轮不得求和。`cacheInputTokens` 同源取末次调用。
2. **混合 origin 的 turn 级 `origin`**：任一分量为 `estimated_chars` ⇒ turn 级 `origin="estimated_chars"`
   （估算不得冒充真值，保守降级）；全部为 `provider` 才输出 `provider`。
3. **无效窗口的 `genMs`**：只累计有效调用（`dur>0` 且 `out>0`）；`Σgen===0` ⇒ `genMs=null` 且 `speed=null`；
   `ttft>dur` 按 C2 退化 `gen=dur`（计入 Σgen）。
4. **`≈` 通道**：由既有 `speedDebugInfo.origin` 子字段驱动（不新增 prop），并同步进 `data-speed-debug`。
5. **`data-speed-basis`**：作为 C6 实现细节要求落地，但**不纳入**本波冻结断言（避免超契约扩测）。

## 6. L3 方案审查后的裁决（Lead 2026-10-27 21:30，review-plan.json verdict=FAIL → 逐条闭环）

- **B1 根 span 必须生产接线**：`record_turn_timing` / `record_turn_speed` 必须在 turn 终态真实调用（数据源 =
  该轮 records 的 Σout/Σgen、首个带 ttft 的 per-call、`records.len()`）；仅测试调用视为未实现。
- **B2 既有 lib 断言迁移（§5.6）**：`crates/pony-agent-core/src/agent/projection.rs:1699`
  `Some(1200)` → `Some(500)`；这是 C5 的直接后果，属被取代的旧口径，非放宽。
- **B3 轮级 origin 归并（§5.2 重申）**：任一 record 为 `estimated_chars` ⇒ 轮级 `estimated_chars`。
- **B4 anthropic 归一含缓存写入（§5.7）**：`context_input = input + cache_read + cache_creation`；
  `cache_creation_input_tokens` 只进 context 口径，不进 `gen_ai.usage.cache_read.*`（读≠写）。
- **B5/B6 旧口径用例迁移（§5.8）**：`tests/unit/token-speed-metrics.spec.ts`（3 例）、
  `tests/acceptance/wave15/*`（5 例）、`tests/HomeSidebar.spec.ts`（3 断言）、
  `tests/e2e/metrics-live-verification.spec.ts` 由 Test Agent 迁移；Lead 已在 ADR 0053 记录「9 例旧断言随
  口径变更迁移」，ADR 原文"历史冻结测试零改动"仅限于**未被本契约取代**的用例。
- **B7 `call_only` 必须有断言（§5.9）**：Test Agent 在 wave54 冻结文件**增量**补一条（不改既有断言）。
- **ADVISORY 采纳**：`context_ratio` 用归一化上下文输入（新增 `record_context_normalized_input`）；
  T6 Σgen 以 fixture 求和 **47579** 为准（±60 容差保留）。
- **ADVISORY 记录不修（已知限制，写入交付说明）**：事件折叠重建路径不回填 `contextInputTokens`
  ⇒ 历史会话 C4 走 S1 回退；`TraceTimelineEntry.contextInputTokens` 本波不消费；buffer 与 stream 的
  `gen` 在同一 Σ 内混算（全 buffered 轮的数值趋近端到端吞吐，属口径定义而非缺陷）。
- **衍生豁免增补（§5.10）**：`session/store.rs` 的派生字段计算属「最小编译/持久化闭包」，允许。

## 7. 收口签字（Lead 2026-10-27 22:05）

- **冻结验收文件追加已批准**：`crates/pony-agent-core/tests/wave54_metric_truth.rs` 由 Test Agent 追加
  `t7_root_span_accumulates_without_explicit_turn_calls`（生产接线覆盖）与 `tests/acceptance/wave54/` 追加
  T4-4（`call_only` 可达性），均为**增量**（+159/-0 与 append-only），既有断言零改动 — Lead 签字认可。
- **契约 C4 文案与实现对齐**：`context_input = input + cache_read + cache_creation`（见 §1 C4 修订与 §5.7）。
- **已知限制（不修，交付说明携带）**：① `ActiveTurn` FIFO 乱序 drop 回填 `previous`（wave-53 既有，
  生产 LIFO 不可达）；② `record_call_timing` 与 `record_usage` 的调用顺序隐含「usage 先于 timing」，
  逆序会静默少算一次调用；③ 折叠重建路径 `contextInputTokens` 留空（历史会话回退 S1）；
  ④ `tool_recovery` 两处构造无 protocol_label（按非 anthropic 口径）。
- **审查收据**：`review-plan.json`(FAIL→闭环) / `review-l3a.json`(FAIL→闭环) / `review-l3b.json`(FAIL→闭环) /
  `review-join.json`(PASS，范围 8 条闭合)，四份均通过 `dev-review-lint.py`。
