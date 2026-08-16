# PA-088 会话存储去冗余：historyNodes 增量引用 + trace 分离表

## Basic Info

- ID: PA-088
- Status: Review
- Priority: P0
- Complexity: B
- Owner: @orchestrator
- Created At: 2026-08-15
- Updated At: 2026-08-15
- OpenSpec Change: `session-storage-dedup`（待创建）
- Spec 状态: 通过（v7，6 轮对抗审核收敛 + 实施后 2 轮 code-reviewer 审核修复）

## Background

`sessions.db` 最大会话 43.77MB（实测），其中 `historyNodes` 字段占 35.54MB——根因是**每个 historyNode 内嵌"从根到该节点"的全部 `turnTraceHistory`**（9 节点存 36 份 trace，实际 8 个唯一），O(n²) 膨胀。加载该会话时前端 `JSON.parse` 43MB 卡死主线程。

止血（已完成）：`TurnToolActivity.resultText` 写入截断 32KB（telemetry.rs），存量清理脚本 `scripts/compact-sessions-db.mjs`（数据库 520MB → 176MB）。

## Goal

1. **historyNodes 去冗余**：持久化剥离节点 trace（内存保留快照），改轻量引用（turn_id + turn_trace_refs）
2. **trace 分离表**：启用 `SeparateTraceTableMode::WriteSeparate`，trace 持久化到 `session_turn_traces` 表
3. **写路径防清空**：Authoritative 会话 save_store/upsert 跳过 table 替换（防空 trace 清空表）
4. **全量 Union 写表**：顶层 ∪ 节点 trace 写表（>24 轮节点 refs 可解析）
5. **轻量节点投影**：snapshot 节点不携带完整 trace（IPC payload 缩小）
6. **存量迁移**：归 PA-090（本卡不做）

## Scope

- `crates/pony-agent-core/src/agent/session.rs`：TurnTraceRef 类型、HistoryNode/SessionState 字段、session_state_for_backend 剥离、collect_trace_union、project_lightweight_nodes、晋升分派、sync_latest_history_node
- `crates/pony-agent-core/src/agent/sqlite_session.rs`：write_full_store/upsert_session 跳过 Authoritative table 写、upsert 不 prune、load_store materialize
- `crates/pony-agent-core/src/agent/control_plane/graph_projection.rs`：history_node_view turnId 优先 node.turn_id
- 前端：`src/types/runtime.ts`（TurnTraceRef）、`src/lib/runtime/history.ts`（cloneHistoryNodes 复制 refs）
- 生产启用：`session.rs:836` `new_with_trace_mode(WriteSeparate)`

## Non-Goals

- 存量数据迁移 → PA-090
- 完整规范化拆表 / 增量写入 / 写队列 / 前端分页 → PA-089
- 原子写入包三态 API / authority 字段 / terminal 矩阵 / hook JCS identity → 后置（spec 2.4/2.6/3.4 记录）

## Acceptance Criteria

1. 新会话：trace 落表、blob 仅 refs、无 trace 丢失（>24 轮节点 refs 可解析）
2. 存量会话（LegacyBlob/DualWrite）保持可读写（不晋升剥离）
3. 运行中 checkout/fork 语义不变（内存快照）；重启 materialize 正确
4. 前端 checkpoint/回滚正常（turnId 修复）
5. core lib 789 + 前端 vitest 398 全绿

## Review Plan

- spec 6 轮对抗审核（@consultant × 4 + @code-reviewer × 2）→ v7 收敛
- 实施后 2 轮 code-reviewer 审核：P0（Union 写表/轻量投影/晋升分派/prune 竞态/DualWrite 预存转换）全部修复

## Current Progress

- **止血完成**（A1 截断 + A2 清理脚本 + WAL/VACUUM 压缩）
- **spec v7 通过**（6 轮审核收敛）
- **实施完成**（2026-08-15）：
  - 后端：TurnTraceRef/字段/剥离/Union/投影/晋升分派/materialize/不 prune（789 测试通过）
  - 前端：TurnTraceRef 类型 + cloneHistoryNodes 复制 refs（398 测试通过）
  - 生产启用 WriteSeparate
- **实施后审核**：2 轮 code-reviewer 审核，P0/P1 全部修复

## Next Action

- 收口：OpenSpec change 归档、任务板更新、提交准备

## Blockers

- 无

## Resume Hint

- 先读 `crates/pony-agent-core/src/agent/session.rs`（session_state_for_backend/collect_trace_union/project_lightweight_nodes）、`sqlite_session.rs`（write_full_store/load_store materialize）、spec v7
