# PA-091 turn 内事件日志（阶段 1：事件溯源演进）

## Basic Info

- ID: PA-091
- Status: Done
- Priority: P0
- Complexity: C
- Owner: @orchestrator
- Created At: 2026-08-18
- Updated At: 2026-08-18
- OpenSpec Change: `openspec/changes/turn-event-log/`
- Spec 状态: 通过（2 轮对抗审核收敛：spec 3 路 + 实施后 tester 详实）

## Background

ADR 0008 决策向事件溯源演进。阶段 1：turn 内过程事实（chunk、tool 调用、provider 结算）append-only 落盘，成为新真源基础。当前 turn 内中间状态只存在于 `TurnStreamEvent` 推送，落盘后丢失。

## Goal

1. `turn_events` 表（append-only，含 branch_id）+ `turn_events_archive` 表（定案语义）
2. `TurnEvent` enum + payload struct（15 种事件，含 user/message、history/squash、provider/usage）
3. `TurnEventSink::persist` 缓冲模型：turn 终态 flush 与 blob 快照同事务
4. 事件名 → variant 映射表（8 种现有发射名 + 新增 emit 点）
5. 迁移回填（per-session 幂等，承认截断损失）

## Scope

- `crates/pony-agent-core/src/agent/`：`turn_event.rs`（事件类型）、`turn_flow.rs`/`runtime/mod.rs`（persist 扩展 + 缓冲）、`sqlite_session.rs`（表 + FlushEvents + 注入点）、`session.rs`（PersistCommand 扩展）
- 迁移回填 + 测试

## Non-Goals

- 投影层（PA-092）、checkpoint 引用化（PA-093）、trace 事件化（PA-094）
- 事件查询 API、重试机制、归档迁移实现

## Acceptance Criteria

1. `turn_events` 表 schema 正确（seq 0-based 连续，并发无空洞）
2. 事件类型 serde round-trip（语义等价）
3. turn 终态 flush 与快照同事务（注入崩溃回滚断言）
4. 事件名映射表驱动测试通过（8 种发射名）
5. 回填 per-session 幂等 + 数据损失标记
6. 既有测试全绿（`npm run cargo:test`）

## Review Plan

- spec 3 路对抗审核（code-reviewer + tester + 架构）→ 已收敛
- 实施后 3 路对抗审核（code-reviewer + tester + 架构）→ 调优 → 验证 → 下一任务

## Current Progress

- **spec 通过**（2026-08-18 3 路审核收敛，P0×6/P1×12 全部采纳）
- **实施完成**（2026-08-18）：
  - `turn_event.rs`：TurnEvent enum（15 种事件）+ EVENT_SCHEMA_VERSION + 4 测试
  - `session.rs`：PersistCommand::FlushEvents + SessionStore.persist_events
  - `sqlite_session.rs`：turn_events/archive 表 + flush_events_tx（seq 0-based + 计数器同事务 + MAX(seq)+1 兜底）+ 注入点（db 后缀匹配）+ backfill_turn_events + 7 测试
  - `turn_flow.rs`：TurnEventSink::persist + 全局注册表 + emit_event 构造 + 事件名映射（8 种发射名 + reason 断言）+ 链路测试 + 3 测试
  - `control_plane/mod.rs`：build() 注册持久化闭包（缓冲 + 写时聚合 + turn 终态 flush）+ 失败日志增强
- **实施后审核**（2026-08-18）：tester 详实（P0×3/P1×5/P2×8），全部裁决：
  - P0-1 快照同事务 → 显式降级（阶段 2 统一，spec 加 NOTE）
  - P0-2 新 emit 点 → 显式豁免（阶段 2，回填覆盖 legacy）
  - P0-3 persist_failed → 归入重试机制（日志增强落地）
  - P1 全部采纳（链路测试/reason 断言/回填边界/计数器兜底/reasoning-only chunk）
- **验证**：core 811 测试通过（含新增 14 个）；修复并发测试注入竞态（注入按 db 后缀匹配）
- **收口**：spec/design/tasks 降级声明 + 采纳记录已更新

## Next Action

- 无（已完成）

## Blockers

- 无

## Resume Hint

- 先读 `openspec/changes/turn-event-log/design.md`（决策已定稿），再读 `turn_event.rs`、`control_plane/mod.rs:932`（注册闭包）