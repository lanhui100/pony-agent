# Design: Split Session Trace Storage And Expand Targeted Persistence

## 背景

当前 persistence 架构已经完成第一轮性能热修：trace 热路径改走 `save_session_to_backend` + SQLite `upsert_session`，避免每次 turn 完成都触发全量 `write_full_store`。

但 `SessionState.turn_trace_history` 仍然作为 `session_data` 的一部分被序列化，这意味着：

- 单 session trace 仍然和 title / summary / history / cursor / history graph 一起共享一个 blob
- 生产写路径虽然从“全 store 重写”降成了“单 session 重写”，但 live trace 仍无法真正做到 trace 级增量写
- 未来要继续扩张 targeted persistence 时，会不断受 `session_data` blob 边界约束

更重要的是，`turn_trace_history` 当前不只是 live session 字段，它还被复制进 `HistoryNode.turn_trace_history`，用于 snapshot、branch restore、history graph backfill 与 control-plane 读面。P3 因此不能只被视为“换个表存数据”，而必须是一轮边界明确的结构改造。

## 设计目标

1. live session trace 具备独立持久化边界
2. 前端与 control-plane 继续读取相同 `SessionSnapshot.turn_trace_history` 形状
3. 历史节点语义不在这一轮被破坏
4. 加载路径支持旧数据 fallback，避免一次性全量迁移阻塞
5. 为后续更多 session-local targeted persistence 和异步写入提供更清晰的 backend seam

## 非目标

- 不在本轮改变前端 runtime/store 读取 contract
- 不在本轮重写 `HistoryNode` 数据模型
- 不在本轮把所有 session-local API 全部切到新 trace 表
- 不在本轮重写 attachment / source snapshot / remove_session 语义
- 不在本轮把 monitor / drilldown 改成独立 trace query API

## 关键边界

### 1. Live Session Trace vs HistoryNode Trace

当前存在两个 trace 真相源：

- live：`SessionState.turn_trace_history`
- historical snapshot：`HistoryNode.turn_trace_history`

本轮建议：

- **仅拆 live trace 的持久化**
- `HistoryNode.turn_trace_history` 仍保留嵌入式快照语义
- `HistoryNode.turn_trace_history` 是历史快照唯一真相源
- `session_turn_traces` 只是当前 live session、当前已 checkout 分支的持久化 materialization，不能跨 branch 累积

这样可以避免在同一轮中连带修改：

- `sync_latest_history_node`
- `hydrate_session_from_node`
- `ensure_history_graph`
- 历史分支恢复 / checkout / fork / switch 的读写语义

同时要明确一个强约束：

- 当 `hydrate_session_from_node`、`restore_branch_head`、`fork_from_history_node`、`switch_history_branch` 改写当前 live session 指向的历史快照时，backend SHALL 在同一事务中用该快照**替换**当前 session 的 `session_turn_traces` 行集合
- 替换后，新表只代表“当前 live session 当前分支”的 trace 集合，不保留其它分支残留行
- 这样历史节点仍保持快照真相源，而 reload 不会把其它分支的 trace 混回当前分支

### 2. Backend Storage Shape

SQLite 新增最小表：

```sql
CREATE TABLE IF NOT EXISTS session_turn_traces (
  session_id TEXT NOT NULL,
  turn_id TEXT NOT NULL,
  updated_at_ms INTEGER NOT NULL DEFAULT 0,
  trace_data TEXT NOT NULL,
  PRIMARY KEY (session_id, turn_id)
);
CREATE INDEX IF NOT EXISTS idx_session_turn_traces_session_updated
  ON session_turn_traces (session_id, updated_at_ms);
```

说明：

- `trace_data` 仍先存完整 JSON blob，避免这一轮再拆 trace 子字段
- `updated_at_ms` 用于 session 内排序和最近 trace 读取
- 暂不加外键 cascade，先保持最小改动和显式删除逻辑

### 3. Backend Capability Expansion

在 `SessionBackend` 上增加 trace 级读写能力，例如：

```rust
fn load_session_traces(&self, session_id: &str) -> Vec<TurnTraceRecord>;
fn upsert_turn_trace(&self, session_id: &str, trace: &TurnTraceRecord) -> bool;
fn annotate_turn_trace_terminal_event(
    &self, session_id: &str, turn_id: &str,
    event_id: Option<&str>, event_type: Option<&str>,
    event_version: Option<&str>, sequence: Option<u64>,
    emitted_at_ms: Option<u64>,
) -> bool;
fn append_turn_trace_hook_records(
    &self, session_id: &str, turn_id: &str,
    hook_records: &[HookTraceRecord],
) -> bool;
fn delete_session_traces(&self, session_id: &str) -> bool;
```

所有新方法应提供默认实现（返回 `false` / `Vec::new()`），保持 trait additive。`MemorySessionBackend` 应覆盖实现以通过测试。

这一轮不要求把所有能力一次补全，但至少要建立：

- live trace 读
- trace upsert

以替代当前通过整 session row 序列化实现的 trace 更新。

### 4. Dual Read Strategy (Merge, not Replace)

加载 `SessionStore` 时：

1. 先从 `sessions` 表读 `session_data`
2. 再查 `session_turn_traces`
3. 再读取该 session 的 trace 迁移状态（逻辑枚举即可，不要求本轮先锁死字段名）：
   - `LegacyBlob`
   - `DualWrite`
   - `TraceTableAuthoritative`
4. 若状态为 `LegacyBlob` 或 `DualWrite`，合并两条来源的 `turn_trace_history`，以 `turn_id` 为 key 去重，新表优先
5. 若状态为 `TraceTableAuthoritative`，只读取 `session_turn_traces`，忽略旧 `session_data.turn_trace_history`
6. 如果该 session 不存在 trace rows，且状态不是 `TraceTableAuthoritative`，则 fallback 到 `session_data.turn_trace_history`

合并方式：O(n) 一次遍历（n = turn 总数，受 `DEFAULT_HISTORY_LIMIT` 约束），并遵循以下规则：

- 去重 key 为 `turn_id`
- 同 `turn_id` 冲突时新表行覆盖旧 blob 行
- 排序主键为稳定的 turn 历史顺序；`updated_at_ms` 只能作为新表内部同源排序辅助，不能因为 annotation / hook append 覆盖旧 turn 的相对位置
- 当 legacy blob 顺序与新表更新时间冲突时，以当前 session snapshot 的 turn 历史顺序为准，避免 reload 后重排

这样可以保证：

- 新数据和老数据都可读  
- 迁移中间态不会丢 trace  
- 不必做一次性全库迁移  
- rollout 可渐进推进  
- 崩溃恢复或阶段 rollout 期间不会丢失部分 trace
- authoritative 后不会因为旧 blob fallback 复活已被驱逐的 trace

**注意**：双源合并是迁移期策略，不是长线方案。Phase 3 稳定后应切回单源读取。

### 5. Write Strategy

推荐分阶段：

#### Phase 1
- 新增 schema
- 新增 dual-read load path
- 不改写路径
- 默认 flag = `Off`

#### Phase 2
- 进入 `DualWrite` 窗口，只允许从 `Off` 切到 `DualWrite`
- 所有现有 session 必须先完成可验证回填，且 dual-read 已确认能无损装配
- 将以下热路径切到双写：
  - `record_turn_trace`
  - `annotate_turn_trace_terminal_event`
  - `append_turn_trace_hook_records`
  - `append_failed_turn`
- trace upsert 和 session upsert 必须包裹在同一个 SQLite 事务中，确保要么都提交要么都不提交
- 新增标记控制：`use_separate_trace_table` feature flag = `DualWrite`

#### Phase 2.5 (Dual-Write Validation Exit)
- 双写窗口至少覆盖一个完整发布周期
- 期间必须验证：
  - dual-write 无 drift
  - backfill 完整
  - fallback 命中降到可接受范围
  - orphan / corruption 指标稳定
- 只有通过上述验证，才允许切到 `WriteSeparate`

#### Phase 3
- 启用 `WriteSeparate`
- `upsert_session` 在序列化前跳过 `turn_trace_history`
- session 进入 `TraceTableAuthoritative` 状态后，不再从旧 blob fallback
- 视验证结果，再继续扩展其它 safe session-local 路径

### 6. History Restore And Trace Table Interaction

历史节点 `HistoryNode.turn_trace_history` 在本轮不改存储模型，保持嵌入式快照语义。

当 `hydrate_session_from_node` 被调用时：
- `session.turn_trace_history` 会被替换为历史节点中的快照数据
- backend SHALL 在同一事务中先删除该 session 的旧 trace rows，再用恢复后的快照重建 `session_turn_traces`
- 后续新的 `record_turn_trace` 等热路径继续只在当前分支 materialization 上增量写入

这个设计确保了：
1. 历史节点恢复语义不松动
2. 新表只承载当前 live session 当前分支的 trace
3. checkout / restore / fork / switch 后 reload 不会混入其它分支的 trace

## 7. Session Deletion Trace Row 清理

`remove_session` 和 `save_store` 全量写路径在删除 session 后，SHALL 同时执行：
```sql
DELETE FROM session_turn_traces WHERE session_id = ?;
```
第一轮不依赖外键 cascade，保持显式删除。

## 8. Truncation 与 session_turn_traces 清理

当前 `DEFAULT_HISTORY_LIMIT` 截断只在内存 `turn_trace_history` 向量上生效。Phase 2 后，trace 写入不再经过 blob 序列化，因此截断不会传递到 `session_turn_traces`。

为避免无界增长，在 `upsert_turn_trace` 或 `save_session_to_backend` 路径中，当检测到某 session 的 trace 行数超过 `DEFAULT_HISTORY_LIMIT` 时，SHALL 执行：

```sql
DELETE FROM session_turn_traces
WHERE session_id = ?
  AND updated_at_ms <= (
    SELECT COALESCE(MIN(updated_at_ms), 0) FROM (
      SELECT updated_at_ms FROM session_turn_traces
      WHERE session_id = ?
      ORDER BY updated_at_ms DESC
      LIMIT 1 OFFSET ?
    )
  )
```

其中 OFFSET = `DEFAULT_HISTORY_LIMIT`。该清理无需每次 upsert 都执行，可作为周期性或按需任务在写路径中触发。

但清理只允许在该 session 已进入 `TraceTableAuthoritative` 状态后执行；在 `LegacyBlob` / `DualWrite` 状态下，不得因为新表裁剪而让 loader 从旧 blob 重新复活已驱逐 trace。

## 9. Rollback 安全

| Phase | blob 是否携带 trace | 回滚安全性 | 条件 |
|---|---|---|---|
| Phase 1 (flag=Off) | 是（未改动） | 安全 | blob 中仍持有完整 trace |
| Phase 2 (flag=DualWrite) | 是（持续写入） | 有条件安全 | 仅在双写窗口期内且不丢失任何 trace 更新时安全 |
| Phase 3 (flag=WriteSeparate) | 否（已剥离） | 不安全 | 需先把新表 materialize 回 blob 后才能回滚 |

启用 `WriteSeparate` 前，必须对所有现有 session 完成 `session_turn_traces` 回填，确保双读能覆盖全量历史 trace，并且至少完成一轮 DualWrite 验证。禁止从 `Off` 直接切到 `WriteSeparate`。

## 10. 分支隔离规则

当前 `session_turn_traces` 以 `(session_id, turn_id)` 为主键，无 `branch_id` 字段。因此本轮必须明确实现语义，而不能把 branch correctness 留给后续。

本轮强制规则：
- `session_turn_traces` 始终表示“当前 live session 当前分支”的完整 materialized trace 集
- 任何 checkout / restore / fork / switch 导致 live session 基线变化时，backend SHALL 在同一事务中执行 `replace_session_traces(session_id, restored_snapshot.turn_trace_history)`
- reload 期间不得把非当前分支 trace 混入当前 session

如果后续发现 `replace_session_traces` 成本不可接受，再另开 change 讨论 `branch_id` 列或更细粒度 branch-aware schema。

## 11. 验证策略补充

- 新增 Integrity check：启动时可选 `SELECT COUNT(*) FROM session_turn_traces t WHERE NOT EXISTS (SELECT 1 FROM sessions s WHERE s.conversation_id = t.session_id)`，记录孤立行数
- 所有 trace roundtrip 测试必须针对 `SqliteSessionBackend` 覆盖，不能只依赖 `MemorySessionBackend`
- 新增 crash recovery 测试：模拟双写中途崩溃后 reload 仍保持完整 trace
- 新增 rollout 观测：记录 flag 状态、dual-read fallback 命中次数、同 `turn_id` merge 覆盖次数、trace prune 次数、orphan row 数
- 新增 corruption 策略：单条 `trace_data` 解析失败时，系统必须记录结构化错误并跳过坏行；单个坏行不能拖垮整 session reload

在本轮结束前，上层读面不应知道 trace 已拆存：

- `SessionSnapshot.turn_trace_history` 继续保留
- `load_turn_traces` 继续返回 `Vec<TurnTraceRecord>`
- `control_plane`、monitor、history drilldown 继续用现有接口

## 风险与收敛

### 风险 1：live trace 与 history node trace 漂移

收敛方式：

- 这一轮不动 `HistoryNode.turn_trace_history` 模型
- 仅重构 live trace 的 persistence source

### 风险 2：load path 双源组装不一致

收敛方式：

- 统一由 backend 装配 `SessionState.turn_trace_history`
- 上层不感知 trace 存储来源

### 风险 3：session deletion 遗留 orphan trace rows

收敛方式：

- 第一轮先显式删除，不依赖数据库 cascade
- 等 schema 稳定后再决定是否引入 FK

## 验证策略

- 新 schema 创建与 dual-read 不破坏旧 session reload
- 只有 trace rows 的 session reload 正常
- 只有旧 `session_data.turn_trace_history` 的 session reload 正常
- trace upsert / annotate / hook append roundtrip 正常
- history restore / branch switch 不丢 trace
- `load_turn_traces`、monitor、control-plane 继续读到完整 trace
