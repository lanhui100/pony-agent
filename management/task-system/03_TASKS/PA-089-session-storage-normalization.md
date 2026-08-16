# PA-089 会话存储规范化：拆表 + 增量写入 + 前端分页

## Basic Info

- ID: PA-089
- Status: Backlog
- Priority: P1
- Complexity: C
- Owner: @orchestrator
- Created At: 2026-08-15
- Updated At: 2026-08-15
- OpenSpec Change: `session-storage-normalization`（待创建）
- Spec 状态: 待审核

## Background

PA-088 完成去冗余后，`session_data` 仍是单行大 JSON blob（预计去冗余后 ~8MB/会话）。完整规范化存储是根治：
- `sessions(conversation_id, title, updated_at_ms, session_data)` 单 blob → 拆表
- 每次 `save_store` 全量序列化所有会话 → 增量 mutation 写入
- 前端 `load_session_runtime_view` 一次返回完整 snapshot → 分页按需加载
- SQLite 单写者 → 写队列/写入 actor（多智能体并发基础）

顾问评估（2026-08-15）结论：SQLite 保留，迁移数据模型而非引擎。推荐架构：SQLite WAL + 规范化 schema + 单写者 actor/bounded write queue + 只读连接池 + tool 大结果外置文件 + 前端分页。

## Goal

1. **规范化 schema**：session/turn/message/trace/tool_activity/history 拆表，大字段外置。
2. **增量写入**：append message / upsert turn / append trace / update node 等 mutation 命令，替代全量快照。
3. **写队列**：单写者 actor + bounded queue，批量合并落库。
4. **前端分页**：`list_sessions` / `load_messages_page` / `load_trace_window` / `load_history_graph` 按需加载。
5. **平滑迁移**：备份 → 建表 → 回填 → 双写 → 影子校验 → 切换 → 删除旧 blob。

## Scope

- `crates/pony-agent-core/src/agent/sqlite_session.rs`：新 schema、mutation 命令、写队列
- `crates/pony-agent-core/src/agent/session.rs`：持久化接口改造
- `crates/pony-agent-core/src/agent/control_plane/*`：宿主命令拆分（分页 API）
- `src/stores/runtime.ts` / `src/lib/runtime/*`：前端按需加载改造
- `scripts/migrate-sessions-schema.mjs`：迁移脚本

## Non-Goals

- 不迁移到非 SQLite 引擎（顾问结论：SQLite 保留）
- 不引入独立 DB 服务器
- tool 大结果外置文件为可选阶段（可先截断预览）

## Acceptance Criteria

1. 新 schema 落库，旧 blob 双写期可回滚。
2. 增量写入生效：turn 热路径不再全量序列化所有会话。
3. 前端分页加载：打开大会话不一次传输全量数据。
4. 影子校验：新旧 loader 产物一致（消息顺序/turn 数/trace 顺序/branch head）。
5. 迁移后数据可回滚（三层回滚：migration_state 开关 / 规范化表保留 / SQLite backup）。
6. 全量测试全绿 + Windows 实机基准（加载/保存耗时、并发写入 p50/p95）。

## Review Plan

- @architect：schema 设计、mutation 接口、写队列架构
- @consultant：迁移策略、回滚、影子校验方案
- @code-reviewer：读写路径回归、并发安全
- @tester：迁移测试、并发基准

## Current Progress

- 顾问评估完成（SQLite 保留 + 规范化方案）
- 任务卡创建

## Next Action

- 创建 spec → 3 路对抗审核 → 分阶段实施（阶段 0 备份 → 1 建表 → 2 回填 → 3 双写 → 4 影子校验 → 5 切换 → 6 删除）

## Blockers

- 依赖 PA-088（去冗余）完成

## Resume Hint

- 先读 `crates/pony-agent-core/src/agent/sqlite_session.rs`（当前 schema）、`session.rs`（SessionState/PersistedStore 结构）、顾问评估结论（SQLite 保留 + 规范化 schema + 写队列 + 前端分页）
