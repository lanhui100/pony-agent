# 对抗矩阵：Per-Call 指标结算快照（TurnMetricsSnapshot）

归属：Test Agent（task-2 / W2）｜契约源（只读）：`.dev-team/contract-matrix-metrics-snapshot.md`（I1–I7）

本目录登记「指标快照」这条链路的**对抗破坏面**：畸形 usage、计时倒挂、除零、缺失字段、
以及旧线缆（无 `metrics` 键）的回放路径。规约：**确定性输入、无 `sleep`、无随机、无 skip、
每条用例都有非空断言**；出现 `NaN`/`Infinity`/负值一律视为失败（契约要求收敛为 `None`）。

## 1. 对抗矩阵（Rust 侧：`crates/pony-agent-core/tests/metrics_snapshot_adversarial.rs`）

| # | 畸形类别 | 构造夹具 | 必须成立的不变量 |
| --- | --- | --- | --- |
| A1 | 零时长 + 有输出 | `turn_duration_ms=0`, `output_tokens=100` | 不 panic；单次调用速度 `None`（不得 Infinity）；轮速度同源于 `weighted_turn_speed` |
| A2 | 计时倒挂 | `first_token_latency_ms=5000 > turn_duration_ms=1000` | 不 panic；`gen = dur`（不得 u64 下溢/负值）；速度 = 100 t/s |
| A3 | 比值除零 | `input_tokens=0`, `cache_hit_input_tokens=500` | `cache_hit_ratio == None`（不得 Infinity、不得 clamp 成 1.0/100%） |
| A4 | 全字段 `None` 伪记录（planner-only） | `ProviderCallCacheRecord` 全 `None` | 不 panic；`turn_calls == 0`；上下文/缓存比不得伪造成 `0` |
| A5 | 混合序列 | `[有效, 全 None, ttft>dur]` | `turn_calls` 只计有效调用（=2）；`turn_output_tokens=150`；`turn_generation_ms=2500`；速度 = 60 t/s 且有限为正 |

外加全局卫生断言（每条用例都跑）：`cache_hit_ratio` / `context_ratio` /
`call_speed_tokens_per_second` / `turn_speed_tokens_per_second` / `cost_usd`
必须 `None` 或「有限且非负」；两个比值额外恒 ∈ `[0,1]`（I2/I3）。

## 2. 对抗矩阵（前端侧：`malformed-payload.spec.ts`）

| # | 畸形类别 | 夹具 | 必须成立的不变量 |
| --- | --- | --- | --- |
| ADV-1 | 旧事件线缆无 `metrics` 键 | 旧 `output_end` JSON（真实历史字段集） | 反序列化后键**缺席**（非 `null` 占位）；消费端 `undefined` 不抛错、不产生假快照（I7） |
| ADV-2-1 | `metrics` 字段全 `null` | 精确 JSON 字符串 | 解析不抛错；`cacheRatioPercent === ""`、`contextPercent === null` 等（I2/I3 缺失纪律） |
| ADV-2-2 | 仅含两个键的最小对象 | `{cacheHitRatio, contextRatio}` | 不抛错；缺失字段按无值处理（`null`），不得兜底成 0 |
| ADV-2-3 | 畸形快照注入 store | 先合法后畸形 | 不抛错；非 `null` 入参按「有值」写入，不得崩溃 |

## 3. 如何运行

```bash
# Rust 侧对抗破坏（红相期必须失败：build_turn_metrics_snapshot 当前为 unimplemented!）
cargo test -p pony-agent-core --test metrics_snapshot_adversarial

# 前端侧畸形 payload 穿透（红相期必须失败：resolveMetricsSnapshotDisplay / applyMetricsSnapshot 尚未落地）
npx vitest run tests/acceptance/adversarial/metrics/malformed-payload.spec.ts
```

配套验收（同波次冻结）：

```bash
cargo test -p pony-agent-core --test metrics_snapshot_contract      # I1–I5 + wire/字段映射
cargo test -p pony-agent-core --test tui_metrics_snapshot           # I6/I7 TUI 消费
npx vitest run tests/metrics-snapshot-live.spec.ts                  # 前端纯函数 + store 集成
```

## 4. 边界说明（本轮未覆盖）

- **真实网络 / SSE 帧畸形**：本轮只覆盖「已反序列化的结构」；线缆层（SSE 分帧、截断 JSON）
  属 `sse_adapter` 既有对抗面，不在本目录范围。
- **并发竞态**：快照构建是无共享状态的纯函数（入参 `&[ProviderCallCacheRecord]` + 只读 meta），
  无锁/无定时器，故不构造竞态用例；并发语义由调用方（`turn_stream` 收尾路径）保证。
- **`step` 取值**：契约只冻结字段存在性（由调用方决定），本目录不冻结其数值。
