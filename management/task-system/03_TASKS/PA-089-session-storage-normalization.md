# PA-089 会话存储规范化：拆表 + 增量写入 + 写队列 + 前端分页

## Basic Info

- ID: PA-089
- Status: In Progress（阶段 0-1 完成）
- Priority: P1
- Complexity: C
- Owner: @orchestrator
- Created At: 2026-08-15
- Updated At: 2026-08-16
- OpenSpec Change: `session-storage-normalization`（待创建）
- Spec 状态: 通过（v3，3 轮对抗审核：阶段 0-1 有条件通过，阶段 2-6 需补执行级契约）

## Background

PA-088/090 已把最大会话从 43.77MB 压到 3.77MB，但存储模型仍是"单行大 JSON blob"：全量序列化写放大、无分页、单写者阻塞。PA-089 做存储规范化（C 级）：拆表 + 增量写入 + 写队列 + 前端分页。

## Goal

1. 规范化 schema（session/turn/message/trace/tool_activity/history 拆表）
2. 增量写入（mutation 命令替代全量快照）
3. 写队列（单写者 actor + bounded queue）
4. 前端分页（按需加载）
5. 平滑迁移（6 阶段，回滚能力随阶段递减）

## Scope

- `crates/pony-agent-core/src/agent/sqlite_session.rs`：新 schema、mutation 命令、写队列
- `crates/pony-agent-core/src/agent/session.rs`：持久化接口改造
- `crates/pony-agent-core/src/agent/control_plane/*`：宿主命令拆分（分页 API）
- `src/stores/runtime.ts` / `src/lib/runtime/*`：前端按需加载改造
- `scripts/migrate-sessions-schema.mjs`：迁移脚本

## Non-Goals

- 不迁移到非 SQLite 引擎（顾问结论：SQLite 保留）
- 不引入独立 DB 服务器
- tool 大结果外置文件为可选阶段（先保持 32KB 截断）

## Acceptance Criteria

1. 新 schema 落库（normalized_* 并行表 + 快照模型），旧 blob 双写期可回滚
2. 增量写入生效：turn 热路径不再全量序列化所有会话
3. 前端分页加载：打开大会话不一次传输全量数据
4. 影子校验：新旧 loader 产物一致
5. 迁移后数据可回滚（回滚能力随阶段递减已明确）
6. 全量测试全绿 + Windows 实机基准

## Review Plan

- @architect：schema 设计、mutation 接口、写队列架构
- @consultant：迁移策略、回滚、影子校验方案
- @code-reviewer：读写路径回归、并发安全
- @tester：迁移测试、并发基准

## Current Progress

- **spec v6 定稿**（6 轮对抗审核：阶段 0-1 有条件通过，阶段 2-6 契约定稿）
- **阶段 0-1 完成**（2026-08-16）：备份 + normalized_* 并行表 DDL（v6：复合主键 + raw_json + evidence 列）
- **阶段 2 完成**（2026-08-17）：回填脚本 `scripts/migrate-sessions-schema.mjs`
  - 12 会话全部回填成功（98 messages / 61 turns / 61 traces / 244 steps / 641 timeline / 81 activities / 73 nodes）
  - 幂等 marker（storage.normalized.v1:{session_id}）+ checksum 版本化 SHA-256
  - 幂等重跑 12 跳过，0 失败
- **阶段 3 完成**（2026-08-17）：PersistCommand 双写
  - `PersistCommand` 枚举（10 命令）+ `epoch` barrier（旧 epoch 拒绝）
  - `SessionBackend::persist_command` trait 方法（默认 Unsupported）
  - `SqliteSessionBackend` 统一事务双写：blob（旧 sessions 表）+ 旧 session_turn_traces 表 + normalized_* 表
  - `sync_blob_trace_tx` 双写同步辅助
  - 测试：persist_command_writes_normalized_tables_and_checks_epoch（双写 + epoch barrier）
  - 验证：core 794 测试通过
- **阶段 4 完成**（2026-08-17）：影子校验 `scripts/verify-normalized-shadow.mjs`
  - 对比维度：消息逐条（role/content/status/ordinal/turn_id）、trace 集合、节点集合、cursor、元数据
  - blob 侧 trace 从旧 session_turn_traces 表读（Authoritative 会话 blob 已剥离）
  - **12 会话全部一致，0 处差异**（影子校验通过）
- **阶段 5 完成**（2026-08-17）：切读——`load_store_normalized`
  - load_store 按 `storage.normalized.v1.phase` 分派（observing/retired → 规范化 loader）
  - 规范化 loader：从 normalized_* 表重建 SessionState（messages→history、trace 表→turn_trace_history、snapshot_json→节点快照、refs materialize、cursor）
  - 测试：load_store_normalized_rebuilds_session_from_normalized_tables（切读重建）
  - 验证：core 795 测试通过 + 生产库数据完整性验证（消息 role 交替/节点 snapshot/trace raw_json 可解析）
- **阶段 6 待推进**（6a/6b：tombstone 表 fencing + 删 blob）

## Next Action

- 阶段 6：6a（drain/freeze/barrier + 切换点 backup）/ 6b（tombstone 表 + 删 blob）

## Blockers

- 阶段 2-6 需 consultant 审核补充执行级契约

## Resume Hint

- 先读 `crates/pony-agent-core/src/agent/sqlite_session.rs`（ensure_normalized_schema）、`PA-089-spec.md`（v3 审核记录含阶段 2-6 契约清单）、`scripts/compact-sessions-db.mjs`（PA-090 迁移经验）