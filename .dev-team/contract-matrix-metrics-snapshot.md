# 契约矩阵：Per-Call 指标结算快照（TurnMetricsSnapshot）

- **冻结方**：Protocol Agent（task-1 / W1）
- **单一真值**：`crates/pony-agent-core/src/agent/metrics_snapshot.rs::TurnMetricsSnapshot`
- **构建入口（空桩）**：`build_turn_metrics_snapshot(records: &[ProviderCallCacheRecord], context_window_tokens: Option<u64>, provider_meta: Option<&ProviderEventMeta>) -> Option<TurnMetricsSnapshot>`
- **实现归属**：Executor 在 task-3 实现；验收归属：Test Agent 在 task-2 冻结红相。本文件一经落盘，实施者只读不可改（不变量原文不得修改）。
- **NFR 基准**：`.dev-team/nfr-baseline.json` 的 `metrics_snapshot` 段。

## 0. 传输契约（wire）

| 载体 | 变更性质 | 语义 |
| --- | --- | --- |
| `TurnStreamEvent.metrics` | **additive**（非 breaking） | `#[serde(default, skip_serializing_if = "Option::is_none")]`：`None` ⇒ 键缺席（旧事件线缆字节完全不变）；`Some` ⇒ 携带快照。 |
| `ProviderEventMeta.context_window_tokens` | **additive**（非 breaking） | Rust 内部结构（不上 wire）。`None` ⇒ 窗口未知，消费者回退既有路径。 |

两端消费约束：**旧事件缺 `metrics` ⇒ TUI 与桌面端回退既有路径**；快照存在 ⇒ 两端都以快照为唯一来源，禁止各自重算。

## 1. 冻结不变量（原文，逐字冻结）

```
I1 仅当事件携带非空 provider_call_records 时，TurnStreamEvent.metrics 才为 Some；records 为空或无 records ⇒ None。
I2 cacheHitRatio = cache_hit_input_tokens / context_input_tokens，取值恒 ∈ [0,1]；禁止用「整轮累计 cache_hit ÷ 末次单次 input」的混量纲算法。
I3 contextRatio = context_input_tokens / context_window_tokens，取值恒 ∈ [0,1]；窗口缺失 ⇒ None。
I4 contextInputTokens 为协议归一值：protocol 以 "anthropic" 开头 ⇒ input + cache_read + cache_creation；其余协议 ⇒ input。缺失时回退裸 inputTokens，**禁止跨调用求和**。
I5 turnSpeedTokensPerSecond 必须与 `turn_flow::weighted_turn_speed(records).speed` 完全同源（同一函数产出），gen = dur - ttft（ttft 存在且 dur > ttft）。
I6 TUI 渲染的缓存百分比 = round(cacheHitRatio * 100)，且必须来自快照；禁止 clamp(0,100) 掩盖越界，不得出现 >100%。
I7 wire 向后兼容：旧事件缺 metrics 时，TUI 与桌面端回退既有路径且不 panic/不报错。
```

## 2. 逐条冻结断言说明（可机械化验证的口径）

### I1 — 附着条件
- 断言：`records.is_empty()` ⇒ `build_turn_metrics_snapshot(&[], _, _) == None`；事件层 `provider_call_records` 为 `None` 或 `Some(vec![])` ⇒ `payload.metrics == None`。
- 冻结理由：空记录无法产出任何 per-call 值；用默认快照冒充会让「未知」变成「0」真值。
- 反例：为 `turn:delta`/`turn:started` 等不携带记录的路径补 `Some(Default::default())`。

### I2 — 缓存命中比（同一量纲）
- 断言：`cacheHitRatio == cache_hit_input_tokens / context_input_tokens`（分母 > 0 时），结果 `∈ [0,1]`；分母缺失/为 0 ⇒ `None`。
- 冻结理由：分子分母必须来自**同一次调用**（`last` 记录的 `cache_hit_input_tokens` 与 `context_input_tokens`）。混量纲（轮累计命中 ÷ 末次 input）会算出 >1 的比值，进而被 clamp 掩盖成恒 100%。
- 反例：`total_cache_hit_tokens / last.input_tokens`；`clamp(0, 100)`；把比值乘 100 后塞进 `cache_hit_ratio`。

### I3 — 上下文占比
- 断言：`contextRatio == context_input_tokens / context_window_tokens`，结果 `∈ [0,1]`；`context_window_tokens == None` ⇒ `contextRatio == None`（**不得**回退成 0 或 1）。
- 冻结理由：窗口未知时占比无定义；伪装成 0% 会让用户误判还有余量。
- 反例：窗口缺失 ⇒ `Some(0.0)`；用固定 200k 常量兜底。

### I4 — 协议归一口径
- 断言：`protocol` 以 `"anthropic"`（大小写敏感，前缀匹配）开头 ⇒ `context_input_tokens = input + cache_read + cache_creation`；其余协议 ⇒ `input`；三者均缺失时回退裸 `inputTokens`。
- 冻结理由：Anthropic 侧 `input` 不含缓存读写，OpenAI 侧 `input` 已含；不归一会让同一模型在两端显示不同上下文占用。
- 反例：跨 records 累加 `context_input_tokens`；对所有协议统一加 `cache_read`。

### I5 — 轮速度同源
- 断言：`turnSpeedTokensPerSecond == turn_flow::weighted_turn_speed(records).speed`（同一函数产出，禁止另写公式）；`turnGenerationMs` 用 `gen = dur - ttft`（仅当 ttft 存在且 `dur > ttft`）。
- 冻结理由：TUI 与桌面端历史上各自反推速度，产生同一轮两个数；快照必须是唯一来源。
- 反例：`Σoutput / Σduration`（未按有效调用过滤）自算；`out / (dur/1000)`（未扣 ttft）冒充轮速度。

### I6 — TUI 百分比渲染
- 断言：缓存百分比 `== round(cacheHitRatio * 100)`（`cacheHitRatio ∈ [0,1]` ⇒ 百分比恒 `≤ 100`），且该值必须取自快照字段。
- 冻结理由：`clamp(0,100)` 只是把越界藏起来，用户看不到「算法错了」。
- 反例：`min(cache_hit / input * 100, 100)`；组件内重复实现百分比公式。

### I7 — wire 向后兼容
- 断言：仅含旧键（无 `metrics`）的 JSON 可反序列化为 `metrics == None`；TUI 与桌面端在该路径下走既有逻辑，不 panic、不报错、不改变既有展示语义。
- 冻结理由：回放历史会话、旧录制的 SSE 帧、旧快照文件都必须继续可用。

## 3. 空桩与实现边界（本轮冻结）

- `build_turn_metrics_snapshot` 当前为 `unimplemented!`，函数体 ≤ 1 行、无逻辑（`dev-stub-lint.py` Exit 0）。
- `step: u32` 无法从 `records` 推断：**由调用方（task-3 的单一附着点）决定**取值（传入或 `records.len()-1` 语义），契约只冻结字段存在性与 wire 名。
- 空桩注释中固定语义：`context_input` = 协议归一；`cache_hit_ratio`/`context_ratio` = `0.0..=1.0` 单位分数（**非百分数**）；`call_speed` = `out / (gen/1000)`。

## 4. 字段表（Rust ⇄ wire camelCase）

| Rust | wire | 类型 | 语义 |
| --- | --- | --- | --- |
| `step` | `step` | `u32` | 逻辑 hop 索引（调用方给定） |
| `context_input_tokens` | `contextInputTokens` | `Option<u64>` | 协议归一上下文输入（I4） |
| `input_tokens` | `inputTokens` | `Option<u64>` | 末次调用裸 input |
| `cache_hit_input_tokens` | `cacheHitInputTokens` | `Option<u64>` | 末次调用缓存命中 |
| `cache_miss_input_tokens` | `cacheMissInputTokens` | `Option<u64>` | 末次调用缓存未命中 |
| `output_tokens` | `outputTokens` | `Option<u64>` | 末次调用输出 |
| `reasoning_tokens` | `reasoningTokens` | `Option<u64>` | 末次调用推理 token |
| `total_tokens` | `totalTokens` | `Option<u64>` | 末次调用总量 |
| `first_token_latency_ms` | `firstTokenLatencyMs` | `Option<u64>` | 末次调用 TTFT |
| `duration_ms` | `durationMs` | `Option<u64>` | 末次调用总耗时 |
| `call_speed_tokens_per_second` | `callSpeedTokensPerSecond` | `Option<f64>` | `out / (gen/1000)` |
| `turn_speed_tokens_per_second` | `turnSpeedTokensPerSecond` | `Option<f64>` | 与 `weighted_turn_speed` 同源（I5） |
| `turn_output_tokens` | `turnOutputTokens` | `u64` | 轮累计输出（有效调用 Σ） |
| `turn_generation_ms` | `turnGenerationMs` | `u64` | 轮累计生成时长 |
| `turn_calls` | `turnCalls` | `u32` | 有效调用数 |
| `cache_hit_ratio` | `cacheHitRatio` | `Option<f64>` | 0..=1 单位分数（I2） |
| `context_window_tokens` | `contextWindowTokens` | `Option<u64>` | 窗口容量（I3） |
| `context_ratio` | `contextRatio` | `Option<f64>` | 0..=1 单位分数（I3） |
| `origin` | `origin` | `Option<String>` | usage 来源（`provider`/`estimated_chars`） |
| `provider` | `provider` | `String` | 提供方名 |
| `model` | `model` | `String` | 模型名 |
| `cost_usd` | `costUsd` | `Option<f64>` | 该轮成本（未知 ⇒ `None`，禁 0 伪造） |

## 5. 门禁与收口（task-1 实测）

| 门禁 | 命令 | 结果 |
| --- | --- | --- |
| 空桩纯度 | `python3 ~/.agents/skills/dev-team/scripts/dev-stub-lint.py crates/pony-agent-core/src/agent/metrics_snapshot.rs` | Exit 0 |
| NFR 基准 | `python3 ~/.agents/skills/dev-team/scripts/dev-nfr-lint.py .dev-team/nfr-baseline.json` | Exit 0 |
| 编译 | `cargo check -p pony-agent-core --all-targets` | Exit 0（仅既有 2 条 `tests/mcp_channel_closure.rs` warning） |

## 6. 偏离与后续事项登记（实施者需知）

1. **`skip_serializing_if` 追加**：`TurnStreamEvent.metrics` 采用 `#[serde(default, skip_serializing_if = "Option::is_none")]`（冻结片段仅列 `#[serde(default)]`）。理由：若 `None` 也序列化为 `"metrics": null`，W57 契约快照门禁（`docs/contracts/ts-contract.snapshot.json` + `npm run contract:check`）将因新增线缆键而漂移，且需同步改 `src/types/runtime.ts`（属前端写域）与重生成快照；`skip_serializing_if` 让旧路径线缆字节零变化，最符合 I7 的「旧消费者行为不变」。
2. **契约快照后续动作**：当 task-3/task-2 让快照真正出现在 wire（`Some`）时，需由前端写域补 `metrics?: TurnMetricsSnapshot | null` 到 `src/types/runtime.ts`，并按 `UPDATE_CONTRACT_SNAPSHOT=1 cargo test -p pony-agent-core --test ts_contract_snapshot` 重生成快照。当前夹具（`tests/ts_contract_snapshot.rs`）填 `metrics: None`，快照键集合不变，故本轮 `npm run contract:check` 无漂移。
3. **可观测性归属**：`docs/observability.yaml` 的更新归 task-5（Lead），本轮不改该文件。
4. **`context_window_tokens` 取值**：`provider_event_meta()` 处本轮固定 `None`，真实取值由 task-3 从 provider capabilities 接入；其余 6 处构造点为纯补 `None`，零语义变更。
