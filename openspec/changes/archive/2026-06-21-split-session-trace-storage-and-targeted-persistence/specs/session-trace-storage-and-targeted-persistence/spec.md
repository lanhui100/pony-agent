## ADDED Requirements

### Requirement: Live session traces SHALL support storage independent from session blob
Pony Agent 的 live session trace SHALL 能独立于 `sessions.session_data` blob 持久化，而不要求每次 trace 更新都重写整个 session blob。

#### Scenario: A turn trace is updated after tool or terminal events
- **WHEN** runtime 记录某个 turn 的 trace
- **THEN** backend SHALL 能仅持久化该 session / turn 的 trace 数据
- **AND** SHALL NOT 强制重写无关 session rows

### Requirement: SQLite schema SHALL include a dedicated trace table
系统 SHALL 使用独立的 `session_turn_traces` 表存储 live trace，不嵌入 `sessions.session_data`。

#### Schema
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

- `trace_data` 使用与 `TurnTraceRecord` 相同的 `serde_json` 序列化格式
- `updated_at_ms` 列是规范的 timestamp；JSON blob 内的 `updated_at` 字段应视为建议性，每次写入时覆盖同步

### Requirement: Trace loading SHALL preserve current snapshot contract
系统在拆分 trace 存储后 SHALL 继续向上层暴露当前 `SessionSnapshot.turn_trace_history` 语义，不要求前端和 control-plane 感知底层存储变化。

#### Scenario: Frontend reads a session snapshot
- **WHEN** frontend、monitor 或 control-plane 读取某个 session snapshot
- **THEN** 它们 SHALL 继续从 `turn_trace_history` 读取完整 trace
- **AND** SHALL NOT 需要额外独立 trace API 才能维持现有行为

### Requirement: Historical snapshots SHALL remain authoritative for restored branches
系统 SHALL 保持 `HistoryNode.turn_trace_history` 作为历史快照真相源；`session_turn_traces` 仅代表当前 live session 当前分支的 materialized trace 集合。

#### Scenario: Restoring or switching to a historical branch
- **WHEN** 用户执行 checkout、restore branch head、fork from history node 或 switch branch
- **THEN** backend SHALL 在同一事务中用目标历史快照替换该 session 的 `session_turn_traces`
- **AND** reload SHALL NOT 混入其它分支遗留 trace

### Requirement: Backend SHALL support dual-read (merge) compatibility during migration
在迁移期间，backend SHALL 支持**合并**读取新 trace 存储与旧 `session_data.turn_trace_history`，而不是二选一。

#### Migration states
- `LegacyBlob`: 允许从旧 blob fallback
- `DualWrite`: 允许 merge 新表与旧 blob
- `TraceTableAuthoritative`: 只读取新表，不再 fallback 到旧 blob

#### Scenario: Session has both new trace rows and old blob traces
- **WHEN** 某个 session 同时存在新 `session_turn_traces` rows 和旧 `session_data.turn_trace_history`
- **THEN** backend SHALL 合并两条来源，按 `turn_id` 去重，新表行优先
- **AND** 合并结果 SHALL 保持稳定的 turn 历史顺序
- **AND** SHALL NOT 因 terminal annotation、hook append 或 `updated_at_ms` 变化而重排已有 turn 的相对位置

#### Scenario: Session has only new trace rows
- **WHEN** 某个 session 已经存在独立 trace rows，但没有旧 `session_data.turn_trace_history`
- **THEN** backend SHALL 只使用 trace rows 组装 live `turn_trace_history`

#### Scenario: Session only has legacy trace blob
- **WHEN** 某个 session 还没有独立 trace rows，但 `session_data` 中存在旧 `turn_trace_history`
- **THEN** backend SHALL fallback 读取旧 blob

#### Scenario: Session is authoritative on separate trace table
- **WHEN** 某个 session 已标记为 `TraceTableAuthoritative`
- **THEN** backend SHALL 只从 `session_turn_traces` 组装 `turn_trace_history`
- **AND** SHALL NOT 再 fallback 读取旧 `session_data.turn_trace_history`

### Requirement: Feature flag SHALL gate the trace split behavior
系统 SHALL 使用 feature flag 控制 trace 存储拆分的行为阶段，不使用代码分支或 schema 版本推断。

#### Flag states
- `Off` (Phase 1): 读走 dual-read（可安全 fallback），写只到旧 blob
- `DualWrite` (Phase 2): 读走 dual-read，写同时到新表 + 旧 blob（事务内原子化）
- `WriteSeparate` (Phase 3): 读优先新表；对 `TraceTableAuthoritative` session 不再 fallback，写只到新表，旧 blob 中跳过 trace 字段

#### Scenario: Transition between flag states
- **WHEN** flag 从 `Off` 切到 `DualWrite`
- **THEN** backend SHALL 确保所有会话在切换前已可被 dual-read 正确加载
- **AND** `load_store` 行为在任一 flag 状态下都不应丢失 trace

#### Scenario: Transition to WriteSeparate
- **WHEN** flag 从 `DualWrite` 切到 `WriteSeparate`
- **THEN** 系统 SHALL 已完成 backfill、至少一个完整发布周期的 dual-write 验证和 drift 检查
- **AND** SHALL NOT 允许从 `Off` 直接切到 `WriteSeparate`

### Requirement: Dual-write in Phase 2.5 SHALL be transactional
在 Phase 2.5 双写窗口期内，trace 表写入和 session 表写入 SHALL 包在同一个事务中，要么都成功要么都失败。

#### Scenario: Process crash during dual-write
- **WHEN** trace upsert 和 session upsert 之间进程崩溃
- **THEN** 系统重启后 SHALL 不出现新表有行但旧 blob 无 trace（或反之）的不一致状态
- **AND** 单次双写失败 SHALL NOT 导致 trace 永久丢失

### Requirement: session_turn_traces SHALL respect DEFAULT_HISTORY_LIMIT
独立 trace 存储表 SHALL 在行数超过 `DEFAULT_HISTORY_LIMIT` 时清理最旧的条目，避免单 session 无界增长。

#### Scenario: After many turns, oldest traces are evicted
- **WHEN** 某个 session 的 `turn_trace_history` 在内存中被截断（`DEFAULT_HISTORY_LIMIT`）
- **THEN** 系统 SHALL 在可行时同步从 `session_turn_traces` 删除被截断的行
- **AND** SHALL NOT 导致 reload 后双读合并恢复到旧 blob 中已被驱逐的 trace
- **AND** prune 行为 SHALL 只发生在 `TraceTableAuthoritative` session 上

### Requirement: History-node restore semantics SHALL remain stable during live trace split
在 live trace 独立持久化的同时，history restore / branch flows SHALL 继续保留当前 `HistoryNode.turn_trace_history` 快照语义。

#### Scenario: Restoring a branch head after trace split
- **WHEN** 用户 restore branch head 或 checkout history node
- **THEN** 系统 SHALL 继续恢复该历史节点对应的 trace 快照
- **AND** SHALL NOT 因 live trace 存储拆分而丢失历史节点已有 trace 语义

### Requirement: Trace hot paths SHALL migrate before broader session-local APIs
系统 SHALL 先迁移真正的 trace 热路径，再决定是否扩展到更广的 session-local 持久化调用点。

#### Scenario: Migrating persistence write paths
- **WHEN** 系统开始使用独立 trace 存储
- **THEN** 第一批迁移对象 SHALL 至少包括 `record_turn_trace`、terminal annotation、hook trace append 与 failed-turn trace 写入
- **AND** SHALL NOT 在同一轮中强制迁移 attachment、session deletion、source snapshot 等非 trace 热路径

### Requirement: upsert_session SHALL avoid re-serializing split trace data
启用独立 trace 存储后，`upsert_session` SHALL 不再把已由新表持久化的 `turn_trace_history` 重新序列化到 `sessions.session_data` 中。

#### Scenario: After trace-level write, save session metadata without trace
- **WHEN** `record_turn_trace` 或同类 trace 热路径已向 `session_turn_traces` 写入
- **THEN** 紧接着的 `upsert_session` / `save_session_to_backend` SHALL 在序列化 `session_data` 时跳过或置空 `turn_trace_history`
- **AND** SHALL NOT 产生双份漂移

### Requirement: Session deletion SHALL clean up trace rows
删除 session 时 SHALL 同时清理 `session_turn_traces` 中对应的行，不依赖外键 cascade。

#### Scenario: Full-store save removes a session
- **WHEN** `remove_session` 或 `save_store` 全量写移除某个 session
- **THEN** 系统 SHALL 同时执行 `DELETE FROM session_turn_traces WHERE session_id = ?`

### Requirement: Rollout observability SHALL expose migration safety signals
系统 SHALL 暴露 trace 存储迁移的关键观测信号，以支持 cutover、回滚和异常定位。

#### Scenario: Operator evaluates migration readiness
- **WHEN** 系统运行在 `DualWrite` 或 `WriteSeparate` 阶段
- **THEN** 系统 SHALL 能提供至少以下观测：flag 状态、dual-read fallback 命中次数、同 `turn_id` merge 覆盖次数、trace prune 次数、orphan row 数

### Requirement: Corrupt trace rows SHALL degrade safely
当 `session_turn_traces.trace_data` 出现坏行时，系统 SHALL 以可观测、可恢复的方式降级，而不是拖垮整 session 加载。

#### Scenario: One trace row is malformed
- **WHEN** loader 解析某条 `trace_data` 失败
- **THEN** 系统 SHALL 记录结构化错误并跳过该坏行
- **AND** 单个坏行 SHALL NOT 导致整 session reload 失败
