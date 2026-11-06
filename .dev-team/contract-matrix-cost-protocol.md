# 契约矩阵：成本协议口径统一（B027 / wave-cost-protocol）

- 分级：**A 级（标准路径）**，测试层级 **L2**（契约级）；触发依据：单模块簇（cost / turn_flow / turn_persist / budget），无并发/鉴权/Schema 变更，改动 < 100 行
- 冻结方：Lead（兼任 Test，已加载 `test-expert`）
- 验收文件（**冻结、实施者只读**）：`crates/pony-agent-core/tests/cost_protocol_parity.rs`
- 迁移文件（Lead 执行）：`crates/pony-agent-core/tests/wave33_platform_cost_test.rs`（`record_usage` 增参迁移，数值断言逐字未改）
- 登记来源：`backlog/backlog.md` B027；根因见 `.dev-team/review-plan-wave-metrics.json` BLOCKING-3 与 `metrics-audit` 报告 §E

## 0. 问题事实（改前）

| 面 | 入口 | 口径 | 后果 |
|---|---|---|---|
| wire `TurnStreamEvent.cost_usd` | `turn_flow::turn_event_cost_usd` | 协议感知 ✅ | 正确 |
| 落库 `TurnTraceRecord.cost_usd` | `turn_persist::estimate_trace_cost` → `cost::estimate_cost` | **固定 OpenAI** ❌ | anthropic 下把 `input` 再减 `cache_read` ⇒ 输入费被清零/低估；前端会话累计成本（`runtime.ts:726` 求和）读的就是这个值 |
| budget `CostSummary` | `budget::CostTracker::record_usage` | 本地 `uncached = input - cached` + `calculate_cost_with_pricing`（固定 OpenAI）❌；且 `total_tokens = input + output` 与分项不同源 | anthropic 用错协议；`total_input + total_cached + total_output ≠ total_tokens` |

补：`CostTracker::record_usage` 当前**零生产调用方**（仅测试），故 budget 侧属潜在缺陷；`TurnTraceRecord.cost_usd` 已有真实消费者，故落库面是本波的主修复。

## 1. 冻结不变量

```
C1 单一出口：所有成本计算必须经 cost::estimate_cost_with_protocol（或其包装 estimate_call_cost_usd），
   protocol 由 ProviderEventMeta.protocol → ProviderCostProtocol::from_label 映射。
C2 协议归一：Anthropic ⇒ uncached = input（input 不含 cache，禁止相减）；
   OpenAi / Unknown ⇒ uncached = input.saturating_sub(cache)。
C3 wire 与落库同值：两者必须由同一函数 turn_flow::estimate_call_cost_usd 产出，逐位相等。
C4 budget 自洽：total_tokens == total_input_tokens + total_cached_tokens + total_output_tokens，
   且 total_input_tokens 为 C2 归一后的未缓存输入。
C5 未知不伪造：未收录模型 / 缺 usage / 无 provider_meta ⇒ None（禁止 0 顶替）。
```

## 2. 冻结 API（实施方必须逐字实现这些名字与签名）

```rust
// crates/pony-agent-core/src/agent/cost.rs
pub fn normalized_uncached_input_tokens(
    input_tokens: u64,
    cache_read_tokens: Option<u64>,
    protocol: ProviderCostProtocol,
) -> u64;

// crates/pony-agent-core/src/agent/turn_flow.rs
pub fn estimate_call_cost_usd(
    provider_meta: Option<&ProviderEventMeta>,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cache_hit_input_tokens: Option<u64>,
) -> Option<f64>;

// crates/pony-agent-core/src/agent/budget.rs（签名变更：新增第 6 参）
pub fn record_usage(
    &mut self,
    model: &str,
    pricing: &ModelPricing,
    input_tokens: u64,
    output_tokens: u64,
    cached_tokens: u64,
    protocol: ProviderCostProtocol,
) -> Result<f64, String>;
```

## 3. 数值基准（`claude-3-5-sonnet`：input $3 / output $15 / cache-read $0.30 per Mtok）

`input=1000, cache_read=8000, output=500`：
- Anthropic：`(1000×3 + 8000×0.30)/1e6 + 500×15/1e6` = `0.0054 + 0.0075` = **0.0129**
- OpenAI：`(0×3 + 8000×0.30)/1e6 + 500×15/1e6` = `0.0024 + 0.0075` = **0.0099**

## 4. 结构护栏（测试以源码断言锁定，防任一面对回退）

- `runtime/turn_persist.rs`：**不得**出现 `estimate_cost(`；**必须**出现 `estimate_call_cost_usd(`
- `budget.rs`：**不得**出现 `calculate_cost_with_pricing(`；**必须**出现 `calculate_cost_with_protocol(` 与 `normalized_uncached_input_tokens(`
- `turn_flow.rs`：**必须**出现 `pub fn estimate_call_cost_usd`

## 5. 契约演进登记（additive / breaking）

| 变更 | 性质 | 兼容性证据 |
|---|---|---|
| 新增 `cost::normalized_uncached_input_tokens` | additive | 新符号，无消费方 |
| 新增 `turn_flow::estimate_call_cost_usd` | additive | 新符号；`turn_event_cost_usd` 委托后 wire 行为不变（数值断言见 `cost_protocol_parity::c3_*`） |
| `budget::CostTracker::record_usage` 增 `protocol` 形参 | **breaking（进程内 API）** | 唯一调用方为 `tests/wave33_platform_cost_test.rs`（本波已迁移）；生产零调用方（`grep record_usage(` 仅 otel 同名无关方法与测试） |
| `TurnTraceRecord.cost_usd` 数值变化 | **用户可见口径变更（非 wire 结构变更）** | anthropic 下成本上升（不再低估）；无持久化 schema 变更，旧记录保留旧值 |

## 6. 明确非目标（不在本波）

- **cache 写入（`cache_creation_input_tokens`）计价**：`ModelPricing` 无 `cache_write_per_mtok` 字段、价格表无该维度 ⇒ 独立立项 **B033**（先补价格维度再谈口径）。
- `usage_ledger` / OTel span 侧口径（已协议感知，无缺陷）。

## 7. 审查处置（L3-A / L3-B，2026-11-06）

| 收据 | 结论 | 处置 |
|---|---|---|
| `.dev-team/review-cost-l3a.json` | PASS（0 BLOCKING / 5 ADVISORY） | 见下 |
| `.dev-team/review-cost-l3b.json` | PASS（0 BLOCKING / 4 ADVISORY） | 见下 |

**本轮修复（文档级）**
- `cost.rs` 模块头第 5 行仍写固定 OpenAI 公式 ⇒ 已改为「按协议归一 + 指向 `normalized_uncached_input_tokens`」。
- `docs/observability.yaml` 称 span/budget 也「收敛到同一出口函数」⇒ 已改为「同一**计价原语**（函数不同、数值等价）」，与实现事实对齐。

**登记为独立事项（不在本波修复）**
- **B036**（P1）：成本聚合粒度统一 —— 同帧存在三种粒度（轮级聚合一次计价 / 快照逐调用 Σ+过滤 / span 逐调用且缺 usage 记 $0），机器反例 0.1575 vs 0.3075。
- **B033**：cache 写入计价维度缺失（价格表无档）。
- **B035**：熔断阈值 `>` vs `>=` 三处不一致（`cost::is_budget_exceeded` 用 `>=`、`CostTracker` 用 `>`、前端用 `>`）——本波未触碰比较符，未被放大。

**基线披露（非本波引入）**
- `cargo test -p pony-agent-core --lib` 在**干净 HEAD 副本**同样失败 1 例（`runtime/tests.rs:6184` 中断检查点文案 vs `CANCELLED_TURN_MESSAGE`）⇒ 属既有基线红测，**不得**以「lib 全绿」作收尾凭证；本波收尾凭证以「目标测试集 + 集成测试集」为准。
