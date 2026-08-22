# session-projection-layer

## Background

阶段 1（`turn-event-log`）落地后，turn 内过程事实已 append-only 落盘，但 blob 快照仍是消费方唯一数据源，存在两个问题：

- **快照是真源**：快照与事件层双写，但快照不可从事件重建，双写的一致性只能靠事务保证，无法验证；
- **手工维护不一致风险**：`append_turn` 手工 push 消息 + 整包序列化，快照字段与事件语义可能漂移。

本 change 是 ADR 0008 **阶段 2：投影层**——把 `SessionSnapshot` 从"真源"降级为"从事件折叠的投影缓存"，可失效、可重建、可对拍验证。注意：**整包序列化的写放大消除推迟到阶段 4**（前端改读事件派生数据后），本 change 的目标是"快照由投影生成（消除手工维护不一致）"。

## Goals

- 定义投影 trait（借鉴 dsh 的 `init/apply/view` 三函数模型）：`HistoryProjection`、`TraceProjection`、`PlanProjection`、`MetricsProjection`。
- `append_turn` 从"push 消息 + 整包快照"改为"append 事件 + 增量折叠投影"，快照由投影生成（同事务）。
- `record_turn_trace` 从"组装后存表"改为"事件折叠"，独立 trace 表降级为投影的持久化缓存（带 seq 水位，重放不回归）。
- `MetricsProjection` 折叠 token usage / 缓存命中 / 延迟指标（ADR 0007 缓存命中一等指标的事件化）：模型监控视图从投影缓存读，不再扫描 trace 表。
- 对拍测试：**golden fixture 策略**（旧路径保留为 `#[cfg(test)]` 参考实现 + 合成语料矩阵），事件重建 vs 快照逐字段对比（限定字段集）。
- 双写变单写：快照由投影增量维护，事件层成为唯一真源。

## Non-goals

- 不做 checkpoint 引用化（阶段 3：`HistoryNode` 引用事件区间、撤回水位回退）。
- 不做 trace 事件化（阶段 4：trace 表降级为投影缓存、前端两视图同源）。
- 不改前端渲染路径（前端继续读快照，快照现在可重建）。
- 不做整包序列化消除（写放大消除推迟到阶段 4，前端改读事件派生数据后）。
- 不做投影持久化缓存的完整失效机制（阶段 3 需要时再引入）。

## Scope

- `crates/pony-agent-core/src/agent/`：
  - 投影 trait 定义（`projection.rs`）+ 四个投影实现（含 `MetricsProjection`）。
  - `session.rs`：`append_turn` / `append_failed_turn` / `record_turn_trace` 重构为"事件 + 折叠"。
  - `sqlite_session.rs`：快照写入改为投影增量更新（或保留整包写但由投影生成）。
  - `turn_persist.rs`：trace 组装逻辑迁移到 `TraceProjection`；provider 结算记录迁移到 `MetricsProjection`。
  - `query_commands.rs`：模型监控视图改读投影缓存。
- 对拍测试基建：`#[cfg(test)]` 参考实现（旧 `append_turn` 逻辑）+ golden fixture 库 + 合成语料矩阵 + `PONY_AGENT_PROJECTION_VERIFY` 差异记录模式。

## Risks

- **投影与现有逻辑行为不一致**：折叠逻辑必须复刻 `append_turn` 的现有语义（含 `DEFAULT_HISTORY_LIMIT` 截断、`refresh_session_metadata`、`commit_history_node_from_live_state` 等副作用）。缓解：golden fixture 对拍 + 既有测试全绿。
- **对拍参照物**：阶段 2 重构会删除旧路径，对拍必须有参考实现。缓解：旧 `append_turn` 保留为 `#[cfg(test)]` 参考实现 + golden snapshot fixture 库。
- **真实数据不可得**：双写期真实数据在 CI 拿不到。缓解：合成语料矩阵（覆盖截断窗口、tool 链、provider usage、checkout/fork、`chunk_missing`、空 session、单事件、10k+ 事件）。
- **verify 模式生产风险**：`PONY_AGENT_PROJECTION_VERIFY` 差异即 panic 会炸生产热路径。缓解：CI 用 golden fixture 对拍；生产只开抽样 + 差异记录（不 panic）。
- **增量折叠的边界**：`HistoryProjection` 折叠到截断窗口时，旧事件仍保留（事件层不截断），投影只维护窗口内状态。
- **trace 投影的 seq 水位**：trace 表降级为缓存后，重放必须幂等（`higher-seq-wins`，借鉴 dsh `projection-store`）。

## Validation

- 对拍测试：golden fixture（合成语料矩阵）→ 事件重建快照 vs 参考实现快照，**限定字段集**（history/trace/plan/metrics）逐字段一致。
- 单元测试：四个投影的 init/apply/view 纯函数正确性、截断窗口语义、trace 水位幂等、MetricsProjection addReplacing。
- 回归：既有 session/trace/checkpoint 测试全绿；验收命令 `npm run cargo:test`。