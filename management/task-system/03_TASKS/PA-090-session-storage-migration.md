# PA-090 存量会话存储迁移（三源合并 + refs 保护 prune + 回滚）

## Basic Info

- ID: PA-090
- Status: Backlog
- Priority: P1
- Complexity: B
- Owner: @orchestrator
- Created At: 2026-08-15
- Updated At: 2026-08-15
- OpenSpec Change: `session-storage-migration`（待创建）
- Spec 状态: 待创建

## Background

PA-088 已完成新写路径（WriteSeparate + 剥离 + refs + 轻量投影），但**存量会话**（LegacyBlob/DualWrite，含 43.77MB 大 blob）保持原样可读写，未受益。存量迁移是解决"打开历史大会话卡死"的最后一步。

存量数据现状（实测）：
- 最大会话 43.77MB（historyNodes 35.54MB 冗余）
- 12 个会话、49 条 trace（session_turn_traces 表已有部分数据）
- 节点 trace 与顶层 trace 可能不一致（节点有顶层已淘汰的 >24 轮 trace）

## Goal

1. **存量会话迁移**：LegacyBlob/DualWrite → TraceTableAuthoritative（剥离节点 trace + 生成 refs + 写表）
2. **三源合并**：已有表 ∪ 顶层 blob trace ∪ 所有节点 trace，按 turn_id 去重取最新
3. **refs 保护 prune**：只删无任何节点 refs 引用的 trace（软上限 + fail closed）
4. **回滚能力**：迁移前备份 + 可 materialize 回填
5. 迁移后最大会话 < 5MB（blob 剥离后）

## Scope

- `scripts/compact-sessions-db.mjs`：扩展为完整迁移脚本（事务 + per-session 幂等标记 + 三源合并 + turn_id 回填 + 置 Authoritative）
- `crates/pony-agent-core/src/agent/sqlite_session.rs`：refs 保护 prune 实现（PA-088 已禁用 prune，此处补上）
- 迁移后验证：重启加载、checkout、fork、branch 切换

## Non-Goals

- 完整规范化拆表 / 增量写入 / 写队列 / 前端分页 → PA-089
- 多版本精确历史（latest-wins 已接受）

## Acceptance Criteria

1. 存量会话迁移后：blob < 5MB、节点仅 refs、表含全量 Union
2. 迁移幂等可重跑（per-session 标记 + checksum）
3. 迁移事务原子（中途崩溃可恢复）
4. 迁移后重启：checkout/fork/branch 语义不变（materialize 正确）
5. refs 保护 prune：被节点引用的 trace 不删
6. 回滚：备份恢复 + materialize 回填

## Review Plan

- @consultant：迁移策略（三源 precedence / tie-break / 幂等标记 / 回滚）
- @code-reviewer：脚本安全性（事务 / 崩溃恢复 / 数据完整性）
- @tester：迁移测试（dry-run / 实跑 / 崩溃注入 / 幂等重跑）

## Current Progress

- PA-088 完成（新写路径 + 不 prune 预留）
- 任务卡创建

## Next Action

- 创建 spec → 对抗审核 → 实施

## Blockers

- 依赖 PA-088 完成（已满足）

## Resume Hint

- 先读 `scripts/compact-sessions-db.mjs`（现有止血脚本）、PA-088 spec v7（`management/task-system/03_TASKS/PA-088-spec.md`）、`sqlite_session.rs`（prune 逻辑 348-376）