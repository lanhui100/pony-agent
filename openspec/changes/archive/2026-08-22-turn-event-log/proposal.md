# turn-event-log

## Background

Pony Agent 的会话数据是"状态快照"模型：`SessionSnapshot` 整包 JSON blob 存于 SQLite，turn 内中间状态（流式 chunk、工具调用时序、plan 更新）只存在于 `TurnStreamEvent` 推送，落盘后丢失；`DEFAULT_HISTORY_LIMIT` 截断 history 后旧 turn 只剩摘要，无法审计完整过程、无法重放。

ADR 0008（`docs/decisions/0008-event-sourcing-evolution.md`）已决策向事件溯源演进，本 change 是**阶段 1：turn 内事件日志**——把 turn 内的过程事实以 append-only 形式落盘，成为新真源的基础。

关键现状基础（复用而非另起炉灶）：

- `turn_flow.rs` 的 `emit_event`（`turn_flow.rs:500`）已是统一事件出口，所有 `TurnStreamEvent` 都从这里发出；
- `PersistCommand`（`session.rs:714`）已是命令式写入雏形，带 epoch 迁移 barrier；
- `SqliteSessionBackend` 已有单连接复用 + WAL + 统一事务机制（`sqlite_session.rs:32-38`）。

## Goals

- 新增 `turn_events` 表（append-only），turn 内过程事实（chunk、tool 调用、plan 更新、checkpoint 动作、provider 结算）全部落盘。
- 定义事件类型（Rust enum + struct，`domain/action` 命名），作为后续投影层（阶段 2）的契约基础。
- **缓冲模型**：`emit_event` 改造为"推送前端 + 入事件缓冲"，turn 终态统一 flush 与 blob 快照写入**同一事务**（迁移期双写）。
- 事件层**永不截断**；归档语义定案（同库归档表保留 seq 与 PK）。
- 现有功能零回归：所有消费方（前端渲染、历史加载、checkout、trace 视图、模型监控）继续读 blob 快照。

## Non-goals

- 不做投影层（阶段 2：快照降级为缓存、对拍测试）。
- 不做 checkpoint 引用化（阶段 3：`HistoryNode` 引用事件区间、撤回水位回退）。
- 不做 trace 事件化（阶段 4：trace 表降级为投影缓存）。
- 不改前端渲染路径（前端继续消费 `TurnStreamEvent` 与快照）。
- 不做事件查询 API（事件层仅供写入与后续投影消费，本 change 不暴露查询面）。
- 不做事件重试机制（`persist_failed` 仅记录，重试后续）。
- 不做归档迁移实现（仅定案语义与表结构）。

## Scope

- `crates/pony-agent-core/src/agent/`：
  - 新增事件类型定义（`turn_event.rs`）：`TurnEvent` enum + 各事件 payload struct。
  - `turn_flow.rs` / `runtime/mod.rs`：`TurnEventSink::persist` 扩展（默认空实现）+ 缓冲 + turn 终态 flush。
  - `sqlite_session.rs`：`turn_events` 表 schema、`FlushEvents` 命令、与快照写入的统一事务、`#[cfg(test)]` 注入点。
  - `session.rs`：`PersistCommand` 扩展（`FlushEvents`）。
  - `runtime/mod.rs`：`build_provider_call_cache_record` 调用点改为 append `ProviderUsage`（builder 保持纯函数，事件 append 在调用方）。
- 迁移：现有 blob 数据一次性回填事件（承认截断损失，per-session 幂等）。
- 测试：事件落盘、seq 连续性（含并发）、flush 事务原子性（注入崩溃）、回填幂等、写时聚合、事件名映射表驱动测试。

## Risks

- **双写一致性**：事件与快照必须同事务，否则崩溃后两层不一致。缓冲模型 + turn 终态 flush 保证原子性；mid-turn 崩溃丢缓冲是接受的降级（显式声明）。
- **写入放大**：事件 + 快照双写增加每 turn 写入量。缓解：chunk 写时聚合 + 批量 flush；阶段 2 快照改增量折叠后消失。
- **事件表增长**：chunk 级事件量大。缓解：写时聚合；归档表语义定案（本 change 不实现）。
- **迁移回填不完整**：blob 截断导致只回填最近 24 turn；chunk 过程不可恢复。缓解：`chunk_missing` / `backfill_partial` / `turn_boundary_synthetic` 标记，投影层（阶段 2）对缺失数据有显式降级语义。
- **seq 契约**：`seq` 从 0 起、恒等于已落盘事件数；分配由持久化层单写者（连接 Mutex）保证，与事件写入同事务。
- **emit 改造波及面**：`TurnEventSink::persist` 默认空实现避免改 27+ 调用点签名；既有 runtime 事件断言测试需适配（tasks 已列）。

## Validation

- 单元测试：事件类型序列化、seq 连续性（0-based + 并发 Barrier）、flush 事务原子性（`#[cfg(test)]` 注入点模拟崩溃）、回填逐 session 幂等、写时聚合、事件名映射表驱动。
- 集成验证：真实 turn 运行后 `turn_events` 表内容与 `TurnStreamEvent` 推送序列一致。
- 回归：既有 session/trace/checkpoint 测试全绿；验收命令用 Rust 测试（`npm run cargo:test`），`npm run verify` 仅证明前端单测 + 编译通过，**不等于** Rust 回归通过。