# 契约矩阵：Token 速度口径 v2（解码口径 + basis 自描述）

- **版本**：v2（取代 `.dev-team/contract-matrix-wave54.md` §1 C2 的速度定义与 `.dev-team/contract-matrix-metrics-snapshot.md` I5）
- **冻结方**：Lead（本文件一经落盘，实施者只读；不变量原文不得修改）
- **单一真值**：
  - Rust：`crates/pony-agent-core/src/agent/turn_flow.rs::{decode_eligible, weighted_turn_speed, call_speed}`（新增/修订）
  - Rust 快照：`crates/pony-agent-core/src/agent/metrics_snapshot.rs::build_turn_metrics_snapshot`
  - Rust 轮内累加：`crates/pony-agent-core/src/agent/otel/mod.rs::{accumulate_active_turn_call, read_active_turn_call_truth}`
  - TS：`src/lib/runtime/token-metrics.ts::resolveTurnWeightedSpeedDetail`（与 Rust 同式，供历史/回放/未来 HTTP 前端消费）
- **写作域**：Rust 核心 = `executor-core`；Rust/TS 测试 = `test-agent`；TS 实现 = `executor-fe`；文档 = Lead。

## 0. 背景与根因（冻结事实）

1. 旧口径 `gen = (ttft 存在且 dur > ttft) ? dur−ttft : dur` 把**没有 TTFT 观测的调用**（批量同步 followup、非流式决策）整段 `dur` 计入分母，分母被排队+prefill 污染。真实库（`~/.local/share/PonyAgent/sessions.db`，400 轮 / 1998 条 per-call）中 `provider_stream` 也有 537 条无 ttft。
2. 旧口径无物理下限：SSE 被上游代理攒成一包时 `ttft≈dur`，`gen` 塌成 1–2ms，单调用速度出现 51,000 甚至 1,789,000 t/s（真实库 61 条 `gen<=5ms`）。
3. 显示值无 `basis` 自描述，用户无法区分「可比的解码速度」与「被 prefill 污染的端到端吞吐」，同格数字不可比。

真实库实测（同库同轮集合）：旧口径聚合 50.0 t/s；仅有效流式 141.5 t/s；端到端 27.6 t/s。三者量级差异即用户观感「与平时速度差巨大」。

## 1. 冻结不变量（原文，逐字冻结，I1–I10）

```
I1  per-call 有效性：valid(r) := r.dur > 0 && r.out > 0。dur/out 缺失或为 0 的记录不参与任何速度计算，也不计入 calls。
I2  生成窗口：gen(r) := (r.ttft 存在且 r.dur > r.ttft) ? r.dur - r.ttft : r.dur（ttft > dur 畸形时 gen = dur，不出负值）。
I3  解码样本判据：decode_eligible(r) := valid(r) && r.ttft.is_some() && r.dur > r.ttft && r.out >= 2 && gen(r) >= floor(r.dur)，
    其中 floor(dur) := max(200ms, 5% * dur)。floor 是物理下限（突发包/计时抖动不可代表生成速率），不是美化。
I4  轮级解码速度：当存在 >=1 条 decode_eligible 记录时，
    speed = Σ_eligible(out - 1) / (Σ_eligible gen / 1000)，basis = stream_decode；
    分子扣 1 是「首 token 已计入 TTFT」，分母只含真实解码窗口。
I5  端到端回退：当无 decode_eligible 记录但存在 >=1 条 valid 记录时，
    speed = Σ_valid out / (Σ_valid dur / 1000)，basis = end_to_end。
I6  无值纪律：既无 decode_eligible 也无 valid 记录时 speed = None、basis = None（禁止 0 占位、禁止伪造）。
I7  同源恒等式（可机械化断言）：
    basis = stream_decode ⇒ turnOutputTokens = Σ_eligible out 且 turnCalls = |eligible| 且
      speed == (turnOutputTokens - turnCalls) / (turnGenerationMs / 1000)；
    basis = end_to_end ⇒ turnOutputTokens = Σ_valid out 且 turnCalls = |valid| 且 turnGenerationMs = Σ_valid dur 且
      speed == turnOutputTokens / (turnGenerationMs / 1000)；
    basis = None ⇒ turnOutputTokens = 0 && turnCalls == 0 && turnGenerationMs == 0。
I8  单调用速度：call_speed(r) = decode_eligible(r) ? (out - 1) / (gen / 1000) : (valid(r) ? out / (dur / 1000) : None)；
    对应 basis 同 I4/I5；快照 callSpeedBasis 必须同上；speed 必须 is_finite 且 > 0，否则 None。
I9  唯一口径：TUI、桌面端（快照分支与 trace 回放分支）、未来 HTTP/WS/SSE 消费面一律复用 §1 公式；
    禁止任何消费面自写第三种公式（含 per-hop 展示、平均值聚合）。
I10 线缆自描述：TurnMetricsSnapshot 追加 turnSpeedBasis / callSpeedBasis（`stream_decode` | `end_to_end`，缺席 ⇔ 无值）；
    根 span `agent.turn.speed.basis` 取值同上（无 speed 时不出该键）。
```

## 2. 实现要点（逐项，可机械核对）

### 2.1 Rust `turn_flow.rs`

```rust
pub const DECODE_FLOOR_MIN_MS: u64 = 200;      // 绝对下限
pub const DECODE_FLOOR_RATIO_PCT: u64 = 5;     // 相对下限 5%

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnSpeedBasis { StreamDecode, EndToEnd }

impl TurnSpeedBasis { pub fn as_str(self) -> &'static str { ... } }  // "stream_decode" | "end_to_end"

pub fn decode_floor_ms(duration_ms: u64) -> u64 { max(200, duration_ms * 5 / 100) }

pub fn call_generation_ms(duration_ms: Option<u64>, ttft_ms: Option<u64>) -> u64  // 保持既有语义（I2）

pub fn decode_eligible(duration_ms: Option<u64>, ttft_ms: Option<u64>, output_tokens: Option<u64>) -> bool

pub struct TurnSpeedTruth {
    pub speed: Option<f64>,
    pub basis: Option<TurnSpeedBasis>,   // 新增
    pub output_tokens: u64,              // 参与计算调用的 Σout（按 basis 口径）
    pub generation_ms: u64,              // 分母时长（stream_decode ⇒ Σgen；end_to_end ⇒ Σdur）
    pub ttft_ms: Option<u64>,
    pub calls: u32,                      // 参与计算调用数（按 basis 口径）
}

pub fn weighted_turn_speed(records: &[ProviderCallCacheRecord]) -> TurnSpeedTruth
pub fn call_speed(record) -> (Option<f64>, Option<TurnSpeedBasis>)   // 快照 per-call 用
```

- `call_generation_ms` 的文档注释必须改写为「分母候选，不等于有效生成窗口」，避免后人再次误用（I2 只定义 gen，不定义合格性）。
- `weighted_turn_speed` 必须先扫一遍分流 eligible / valid，再按 I4→I5→I6 择一计算；**禁止**在一次 Σ 中混两种口径。
- 浮点卫生：`speed` 必须 `is_finite() && > 0.0`，否则 `None`（与既有 A5 纪律一致）。

### 2.2 Rust `otel/mod.rs`

- `ActiveTurn` 追加 `decode_output_tokens / decode_generation_ms / decode_calls`（`call_output_tokens` 等既有字段改为 end_to_end 口径或直接复用，命名须自解释）。
- `accumulate_active_turn_call` 同时按 eligible 与 valid 两路累加；`read_active_turn_call_truth` 按 I4/I5/I6 择一并与 `weighted_turn_speed` **完全同式**（可抽公共小函数，禁止复制粘贴公式后独立演化）。
- 根 span：`ATTR_AGENT_TURN_SPEED_BASIS` 值改为 `stream_decode` / `end_to_end`；无 speed ⇒ 不写该键。（旧值 `turn_weighted` 作废。）

### 2.3 Rust `metrics_snapshot.rs`

- 新增字段（wire camelCase，`#[serde(default, skip_serializing_if = "Option::is_none")]`）：
  - `turn_speed_basis: Option<TurnSpeedBasis>` → `turnSpeedBasis`
  - `call_speed_basis: Option<TurnSpeedBasis>` → `callSpeedBasis`
- `call_speed_tokens_per_second` 改用 `turn_flow::call_speed(last)`，`call_speed_basis` 同源（I8）。
- `turn_*` 三项继续取自 `weighted_turn_speed`（I7 恒等式）。
- `docs/contracts/ts-contract.snapshot.json` 由 `UPDATE_CONTRACT_SNAPSHOT=1 cargo test -p pony-agent-core --test ts_contract_snapshot` 重生成（**回归用增字段，禁止手改 JSON**）。

### 2.4 TUI `bin/tui.rs`

- `metrics_line` 速度段：`basis == end_to_end` ⇒ `≈{n} t/s`；`stream_decode` ⇒ `{n} t/s`；`None` ⇒ 不渲染（保持既有 `if let Some(speed)` 结构）。
- 禁止在 TUI 侧另算（I9）；只读快照字段。

### 2.5 TS `src/lib/runtime/token-metrics.ts`

- 新增导出：
  ```ts
  export const DECODE_FLOOR_MIN_MS = 200;
  export const DECODE_FLOOR_RATIO = 0.05;
  export function decodeFloorMs(durationMs: number): number
  export function isDecodeEligible(c: CallComponent): boolean
  ```
- `resolveTurnWeightedSpeedDetail`：
  - `mode` 域不变（`turn_weighted` | `call_only` | `none`，表示**聚合范围**）；
  - `basis` 域**变更**为 `stream_decode` | `end_to_end` | `none`（表示**测的是什么**）；
  - 计算按 I4/I5/I6，`calls/outputTokens/genMs` 为参与计算集合（I7）；
  - `ttftMs` 语义保持（首个带 ttft 的 per-call 值）。
- `resolveMetricsSnapshotDisplay` 追加 `speedBasis`（直取 `turnSpeedBasis`，缺失 ⇒ null）。
- `tokenSpeedBasisOf(...)` 之类别名需与新域一致；注释中「与 mode 同域」的表述必须删除（`token-metrics.ts:190`）。

### 2.6 TS 展示层

- `HomeStatusPanel.vue::formatTokenSpeed`：`≈` 前缀条件 = `isEstimatedUsage || speedBasis === "stream_decode" ? 无 : "≈"`，即 `speedBasis === "end_to_end"` 或估算来源 ⇒ 单字符 `≈`（两者同时为真只出现一次 `≈`）。
- `HomeTracePanel.vue` per-hop：`activeGenerationDurationMs` / `formatEntryTokenSpeed` 必须改用 `isDecodeEligible` + 同一 floor（I9），标签：eligible ⇒ `生成速度`，valid 但不可解码 ⇒ `整体速度`，无效 ⇒ 不渲染。
- `useTraceProjection.ts`：快照分支的 `data-speed-basis` 取 `snapshot.turnSpeedBasis`（缺失回退 `"none"`）；trace 回退分支取 `detail.basis`。**DOM 属性名不变**，取值域变更。

### 2.7 未来 HTTP/WS 消费面（本轮不实现，契约必须自描述）

- 单位自述：wire 字段名以 `tokensPerSecond` 结尾；`turnSpeedBasis` / `callSpeedBasis` 为口径标记。
- 新端消费纪律：`basis === "end_to_end"` ⇒ 必须渲染 `≈` 或 `e2e` 前缀；`basis` 缺失 ⇒ 渲染 `—`，不得当 `stream_decode` 用。

## 3. 冻结范例（Test Agent 与 Executor 共用；容差 1e-9）

| # | 记录（out / dur / ttft） | 期望 speed | basis | calls | outputTokens | generationMs |
|---|---|---|---|---|---|---|
| E1 | 单条 250 / 1500 / 500 | 249 | stream_decode | 1 | 250 | 1000 |
| E2 | 100 / 1200 / 200 + 200 / 1800 / null | 99 | stream_decode | 1 | 100 | 1000 |
| E3 | 仅 200 / 1800 / null | 111.111111… | end_to_end | 1 | 200 | 1800 |
| E4 | 1789 / 16685 / 16684（突发包） | 107.221456… | end_to_end | 1 | 1789 | 16685 |
| E5 | E2 的第 1 条 + E4 的突发包 | 99 | stream_decode | 1 | 100 | 1000 |
| E6 | out=0 或 dur=0 | None | None | 0 | 0 | 0 |
| E7 | 1 / 1000 / 100（out<2） | 1 | end_to_end | 1 | 1 | 1000 |
| E8 | 100 / 1000 / 1200（ttft>dur） | 100 | end_to_end | 1 | 100 | 1000 |
| E9 | 空记录 [] | None | None | 0 | 0 | 0 |
| E10 | 50 / 1000 / 800（gen=200 恰等于下限） | 245 | stream_decode | 1 | 50 | 200 |

- E1 判别力：旧口径同为 250（gen 相同），差额来自分子 `out−1` ⇒ 249。
- E2 判别力：旧口径 107.142857（把无 ttft 的 1800ms 计入分母）。
- E4 判别力：旧口径 1,789,000 t/s ⇒ 新口径回退 end_to_end 107.22。
- E10 判别力：`gen == floor` 边界必须包含（>= 而非 >）；速度为 `(50−1)/(200/1000) = 245`。
  （v2.1 勘误：初版误写 49，是把分母当成 `dur` 而不是 `gen` 的笔误，公式与不变量未变。）

## 4. 字段表增量（Rust ⇄ wire）

| Rust | wire | 类型 | 语义 |
| --- | --- | --- | --- |
| `turn_speed_basis` | `turnSpeedBasis` | `Option<TurnSpeedBasis>` | `stream_decode` / `end_to_end`；缺席 ⇔ 无 speed（I10） |
| `call_speed_basis` | `callSpeedBasis` | `Option<TurnSpeedBasis>` | 同上，per-call（I8） |

`turnOutputTokens` / `turnGenerationMs` / `turnCalls` 语义收紧为「参与计算集合」（I7），旧口径的「全部有效调用」不再成立。

## 5. 需要迁移的既有断言（迁移而非删除）

| 位置 | 旧断言 | 新断言 |
|---|---|---|
| `crates/pony-agent-core/tests/metrics_snapshot_adversarial.rs` a2 | `ttft > dur` ⇒ gen=dur 参与加权 | `ttft > dur` ⇒ 不可解码 ⇒ end_to_end 回退 |
| 同文件 a5 | 无 ttft 记录计入 Σgen | 无 ttft 记录不计入 decode，仅进 e2e 回退 |
| `crates/pony-agent-core/tests/metrics_snapshot_contract.rs` I5 | turn 速度同源 | 同源 + I7 恒等式 + `turnSpeedBasis` 一致 |
| `crates/pony-agent-core/tests/wave54_metric_truth.rs` T2/T7 | `basis == "turn_weighted"` | `basis ∈ {stream_decode, end_to_end}`，且与 truth 同源 |
| `tests/acceptance/wave54/token-speed-truth.spec.ts` T4-1/T4-2/T6-2 | 107.142857 / 250 / 95.46 | 99 / 249 / 由 fixture 按新口径重算并冻结 |
| `tests/acceptance/wave54/adversarial.spec.ts` | `basis=turn_weighted` | 新域 |
| `tests/e2e/metrics-live-verification.spec.ts` | `data-speed-basis="turn_weighted"` | `data-speed-basis` ∈ 新域 |

## 6. 门禁（Lead 终审执行）

| 门禁 | 命令 |
| --- | --- |
| Rust 编译 | `cargo check -p pony-agent-core --all-targets` |
| Rust 契约/对抗测试 | `cargo test -p pony-agent-core --test metrics_snapshot_contract --test metrics_snapshot_adversarial --test tui_metrics_snapshot --test wave54_metric_truth --test ts_contract_snapshot` |
| Rust 全量 | `cargo test -p pony-agent-core` |
| TS 单测 | `npx vitest run tests/acceptance/wave54 tests/unit/token-speed-metrics.spec.ts tests/metrics-snapshot-live.spec.ts` |
| TS 全量单测 | `npx vitest run` |
| 类型检查 | `npx vue-tsc --noEmit` |
| 契约快照 | `npm run contract:check` |

## 7. 明确不做（Non-Goals）

- 不改 `first_token_latency_ms` 的「末次 hop 取值」语义（首 token 展示口径属另一议题，登记 backlog）。
- 不新增 `/metrics` 或任何导出面；不改 `docs/observability.yaml` 的归属判定（本轮无新增对外接口/网络 IO）。
- 不引入流式 delta 级 TPOT 采样（需要 provider 层时间戳采集，属独立议题）。
