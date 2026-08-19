# PA-094 trace 事件化（阶段 4：事件溯源演进）

## Basic Info

- ID: PA-094
- Status: Backlog
- Priority: P0
- Complexity: C
- Owner: @orchestrator
- Created At: 2026-08-18
- Updated At: 2026-08-18
- OpenSpec Change: `openspec/changes/trace-event-projection/`
- Spec 状态: 通过（3 路对抗审核收敛，2026-08-18）

## Background

PA-092 已把 trace 组装迁移到 TraceProjection，但 trace 表仍是独立权威存储，前端 trace 视图与消息视图仍是两套数据路径。阶段 4：trace 事件化收尾。

## Goal

1. trace 表降级为投影缓存（事件权威，清表可重建）
2. timeline 事件折叠映射表（表驱动测试）
3. ProviderCallCacheRecord 迁出（MetricsProjection 生成）
4. 大字段外置（build_context_observation 引用 + 按需加载）
5. 前端两视图同源（wire 兼容优先）

## Scope

- `crates/pony-agent-core/src/agent/`：`turn_persist.rs`、`telemetry.rs`、`sqlite_session.rs`
- 前端 `src/lib/runtime/trace.ts` / `trace-projection.ts` 适配（零改动优先）

## Non-Goals

- trace 视图 UI 重构、跨会话聚合查询
- TurnStreamEvent 推送协议变更

## Acceptance Criteria

1. trace 表清空后从事件重建一致（事件权威）
2. timeline 折叠映射表驱动测试通过
3. ProviderCallCacheRecord 重建（字段集 + 豁免清单）
4. 大字段按需加载（查询返回与原始一致）
5. 既有测试全绿（`npm run cargo:test`）

## Review Plan

- spec 3 路对抗审核 → 已收敛
- 实施后 3 路对抗审核 → 调优 → 验证 → 收口

## Current Progress

- spec 通过（2026-08-18 3 路审核收敛）

## Next Action

- 依赖 PA-093 完成后启动

## Blockers

- PA-093（checkpoint 引用化）

## Resume Hint

- 先读 `openspec/changes/trace-event-projection/design.md`，再读 `turn_persist.rs`（record_turn_trace）、`telemetry.rs`（TurnTraceRecord）