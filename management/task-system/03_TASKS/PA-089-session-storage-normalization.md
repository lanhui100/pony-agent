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

- **spec v3 通过**（3 轮对抗审核：阶段 0-1 有条件通过，阶段 2-6 需补执行级契约）
- **阶段 0-1 完成**（2026-08-16）：
  - 阶段 0：备份逻辑（复用 PA-090 脚本 VACUUM INTO + integrity_check + 进程检测）
  - 阶段 1：`ensure_normalized_schema`——10 张 normalized_* 并行表（方案 A：旧 sessions 保留为 blob 表）+ FK ON DELETE CASCADE + 索引 + PRAGMA foreign_keys=ON
  - 验证：core 793 测试通过（建表不影响旧路径）+ 生产库 DDL 执行成功
- **阶段 2-6 待推进**（需先补执行级契约：回填逐字段矩阵、materialize 原子协议、6a 状态机）

## Next Action

- 阶段 2（回填）：补回填逐字段契约（history_state_evidence/created_at/state_version/turns 并集/message_id 规则）→ 实施回填脚本
- 阶段 3-6：materialize 原子协议 + 双写 + 影子校验 + 切读 + 6a/6b

## Blockers

- 阶段 2-6 需 consultant 审核补充执行级契约

## Resume Hint

- 先读 `crates/pony-agent-core/src/agent/sqlite_session.rs`（ensure_normalized_schema）、`PA-089-spec.md`（v3 审核记录含阶段 2-6 契约清单）、`scripts/compact-sessions-db.mjs`（PA-090 迁移经验）