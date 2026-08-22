# Design

## Decision Summary

1. 投影 trait：`init() -> S` / `apply(&mut S, &TurnEvent)` / `view(&S) -> SessionSnapshot`，纯函数、确定性、增量折叠。
2. 四个投影：`HistoryProjection`（消息历史 + 截断窗口）、`TraceProjection`（`TurnTraceRecord`）、`PlanProjection`（plan 状态）、`MetricsProjection`（token usage / 缓存命中 / 延迟）。
3. `append_turn` 重构：append 事件 → 增量折叠投影 → 快照由投影生成（同事务）；**整包序列化写放大消除推迟到阶段 4**。
4. trace 表降级为投影缓存：`(session_id, turn_id, ver, seq, val)`，seq 水位 + `higher-seq-wins`。
5. 对拍测试：`#[cfg(test)]` 参考实现 + golden fixture 库 + 合成语料矩阵 + 差异记录模式（非 panic）。

## Chosen Direction

### 1. 投影 trait

```rust
pub trait Projection<S> {
    fn init() -> S;
    fn apply(state: &mut S, event: &TurnEvent);
    fn view(&self, state: &S) -> SessionSnapshot;
}
```

- 纯函数约束：`apply` 不得依赖外部状态（时间、随机、IO）；测试断言"增量折叠 == 全量重折叠"。
- 投影注册：`ProjectionRegistry`（阶段 2 先静态四个，插件化后续再议）。

### 2. HistoryProjection

- 状态：`Vec<TurnHistoryMessage>` + 截断窗口（`DEFAULT_HISTORY_LIMIT`）。
- `apply`：`user/message` → push user 消息；`assistant/message` → push assistant 消息（含 turn_id/status/model_name/token_count/reasoning_content 元数据）；`turn/end` → 标记状态；**`history/squash` → 丢弃 base_seq 之前的消息事件并注入摘要**（压缩不复活被压缩历史）；窗口超限 → drain 头部。
- 与现有 `append_turn` 语义对齐：`refresh_session_metadata`、`commit_history_node_from_live_state` 等副作用保留在调用方（投影只负责消息状态）。

### 3. TraceProjection

- 状态：`HashMap<turn_id, TurnTraceRecord>`（或 Vec + 索引）。
- `apply`：`step/start` → `call_model` 条目；`tool/call` → `call_tool` 条目（含 started_at）；`tool/result` → `return_result` 条目 + `tool_activities`（duration/status/artifacts 从事件字段重建）；`assistant/chunk` → 文本聚合；`turn/end` → 结算 token 指标。
- 持久化缓存：trace 表行带 `seq` 水位，重放时 `seq <= watermark` 的事件跳过（`higher-seq-wins`）。

### 4. MetricsProjection

- 状态：`totals`（四桶累计：uncached_input / cache_read / cache_write / output）+ `last`（最近一次调用）+ `by_turn`（per-turn 聚合，模型监控 drilldown 用）。
- `apply`：**totals 只累计 `ProviderUsage`**（`assistant/message` 的 usage 仅消息元数据展示，不参与 totals——防双计数）；`ProviderUsage` 按 `(turn_id, step)` **addReplacing**（同一 step 的 usage 替换而非累加，借鉴 dsh `token-meter` 的 `addReplacing`；step 编号规则：初始请求=0，每次 tool followup +1）；`step/start` → first_token_latency；`turn/end` → turn_duration_ms（以 turn/end 为准，ProviderUsage 的 turn_duration_ms 是单次调用耗时）。
- 持久化缓存：`(session_id, key, ver, seq, val)` 带 seq 水位，重放时 `higher-seq-wins`。
- 消费方：`load_model_monitor_summary` / `load_model_monitor_session_drilldown`（`query_commands.rs`）从 MetricsProjection 缓存读，**不再扫描 trace 表**；`ProviderCallCacheRecord` 降级为 MetricsProjection 的视图产物。

### 5. append_turn 重构

```
append_turn(session_id, user_message, assistant_message, ...)
    ├─▶ append 事件：turn/start, user/message, assistant/message, provider/usage, turn/end（+ tool/* 由 tool 执行路径 append）
    ├─▶ 增量折叠：history/trace/plan/metrics 四个投影 apply 新事件
    └─▶ 同事务：事件落盘 + 投影缓存更新（快照由 view() 生成）
```

- 双写变单写：blob 不再独立 upsert，快照 = `view(投影状态)`。
- **对拍参考实现**：旧 `append_turn` 逻辑保留为 `#[cfg(test)]` 参考实现（`reference_append_turn`），仅测试构建。

### 6. 对拍测试基建

- **golden fixture 库**：`tests/fixtures/projection/` 下合成语料（JSON 事件序列 + 期望快照），覆盖：空 session、单事件 turn、截断窗口边界、tool 链、provider usage（多 hop）、checkout/fork 事件、`chunk_missing`、`history/squash`、10k+ 事件、100KB+ payload。
- **差异记录模式**：`PONY_AGENT_PROJECTION_VERIFY=record`（CI/测试）→ 差异即失败；`=sample`（生产抽样）→ 差异记录日志不 panic。
- **字段集限定**：对拍只比较约定字段集（history / turn_trace_history / plan 状态 / metrics 派生字段）；非投影字段（history_nodes、history_branches、history_cursor、long_term_memory_entries、memory_write_evidence、provider_native_transcript、attachment catalog 等）由既有回归测试覆盖，不在对拍范围。

## Verification Strategy

- 单元：投影纯函数（init/apply/view）、截断窗口、`history/squash` 语义、trace 水位幂等、MetricsProjection addReplacing + 双 usage 防重、增量 == 全量。
- 对拍：golden fixture 合成语料 → 事件重建 vs 参考实现，限定字段集逐字段一致。
- 回归：既有测试全绿；验收命令 `npm run cargo:test`。