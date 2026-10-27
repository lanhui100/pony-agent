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
  - `protocol_label` 以 `anthropic` 开头 ⇒ `input + cache_read`（anthropic `input` 不含缓存）；
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
  `duration == ttft + generation`；buffered 调用 ⇒ 无 ttft 键、generation == duration。
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

**L2-AT（对抗）**：A1 `dur<=0` / `ttft>dur` / 全空 usage ⇒ 不得产出 NaN/Inf/负值、不得输出 speed 键；
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
