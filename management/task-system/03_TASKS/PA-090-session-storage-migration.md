# PA-090 存量会话存储迁移（三源合并 + refs 保护 prune + 回滚）

## Basic Info

- ID: PA-090
- Status: Review
- Priority: P1
- Complexity: B
- Owner: @orchestrator
- Created At: 2026-08-15
- Updated At: 2026-08-16
- OpenSpec Change: `session-storage-migration`（待创建）
- Spec 状态: 通过（v3，3 轮对抗审核收敛 + 实施后 code-reviewer 审核修复）

## Background

PA-088 已完成新写路径（WriteSeparate + 节点 refs + 轻量投影），存量会话（LegacyBlob/DualWrite）保持原样：最大会话 43.77MB（historyNodes 冗余），打开时前端 JSON.parse 卡死。

## Goal

1. 存量会话迁移为 TraceTableAuthoritative（剥离顶层+节点 trace + 生成 refs + 写表）
2. 三源合并（已有表 ∪ 顶层 blob ∪ 节点 trace），不丢任何 trace 最新版本
3. refs 保护 prune（只删无节点引用的 trace）
4. 迁移后 blob < 5MB，重启后 checkout/fork/branch 语义不变
5. 可回滚（备份 + materialize 回填）

## Scope

- `scripts/compact-sessions-db.mjs`：完整迁移脚本（进程检测 + VACUUM INTO 备份 + 三源合并 + per-session 事务 + 幂等 marker + fail closed + dry-run 不写库）
- `crates/pony-agent-core/src/agent/session.rs`：ReplaceAll 全调用点 union + collect_trace_union 稳定顺序
- `crates/pony-agent-core/src/agent/sqlite_session.rs`：refs 保护 prune + 节点 materialize 用全量表 + 顶层 refs 过滤

## Non-Goals

- 完整规范化拆表 / 增量写入 / 写队列 / 前端分页 → PA-089
- 多版本精确历史（latest-wins 已接受）

## Acceptance Criteria

1. 存量会话迁移后：blob < 5MB、顶层+节点仅 refs、表含全量 Union、状态 `trace_table_authoritative`
2. 迁移幂等可重跑（四条件跳过 + checksum + --force 决策矩阵）
3. 迁移事务原子（崩溃可恢复，fail closed）
4. 迁移后重启：checkout/fork/branch 语义不变（ReplaceAll 全调用点 union + 节点 materialize 全量表）
5. refs 保护 prune：被引用不删、损坏 fail closed
6. 回滚：备份恢复（精确）+ materialize 回填（降级）
7. 迁移后顶层 trace 顺序与迁移前一致

## Review Plan

- spec 3 轮对抗审核（@consultant × 2 + @code-reviewer × 1）→ v3 收敛
- 实施后 code-reviewer 审核：P0-1（节点 materialize 全量表）+ P1-1~P1-5 全部修复

## Current Progress

- **spec v3 通过**（3 轮审核收敛）
- **实施完成**（2026-08-16）：
  - 3.0 ReplaceAll 全调用点 union + collect_trace_union 稳定顺序 + 顶层 refs 过滤
  - 3.1 迁移脚本重写（进程检测 fail-closed + VACUUM INTO 备份 + 三源合并 + per-session 事务 + 幂等四条件 + checksum 比对 + prune 接线 + fail closed）
  - 3.2 refs 保护 prune（3 个测试）
- **真实迁移成功**：12 会话全部迁移，最大会话 43.77MB → 3.77MB（全部 < 5MB），289/289 refs 可解析，幂等重跑 12 跳过
- **实施后审核**：code-reviewer 发现 P0-1（节点 materialize 用过滤后子集）+ P1-1~P1-5，全部修复
- **验证**：core 793 测试通过（含 3 个新 prune 测试 + fork 节点 materialize 回归）

## Next Action

- 收口：OpenSpec change 归档、任务板更新、提交准备

## Blockers

- 无

## Resume Hint

- 先读 `scripts/compact-sessions-db.mjs`（迁移脚本）、`crates/pony-agent-core/src/agent/sqlite_session.rs`（prune/materialize/merge）、spec v3