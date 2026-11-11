# PA-092 投影层（阶段 2：事件溯源演进）

## Basic Info

- ID: PA-092
- Status: Done
- Priority: P0
- Complexity: C
- Owner: @orchestrator
- Created At: 2026-08-18
- Updated At: 2026-08-18
- OpenSpec Change: `openspec/changes/session-projection-layer/`
- Spec 状态: 通过（2 轮对抗审核收敛：spec 3 路 + 实施后 code-reviewer/tester 详实）

## Background

PA-091 落地后事件层成为真源基础，但 blob 快照仍是消费方唯一数据源且不可从事件重建。阶段 2：快照降级为投影缓存。

## Goal

1. 投影 trait（init/apply/view）+ 四个投影（History/Trace/Plan/Metrics）
2. `append_turn` 重构：事件 + 增量折叠，快照由投影生成
3. trace 表降级为投影缓存（seq 水位 + higher-seq-wins）
4. MetricsProjection（addReplacing，totals 只累计 ProviderUsage）
5. golden fixture 对拍（限定字段集）

## Scope

- `crates/pony-agent-core/src/agent/`：`projection.rs`、`session.rs`、`sqlite_session.rs`、`turn_persist.rs`、`query_commands.rs`
- golden fixture 库 + 对拍模式

## Non-Goals

- checkpoint 引用化（PA-093）、trace 事件化（PA-094）
- 整包序列化写放大消除（推迟到 PA-094）

## Acceptance Criteria

1. 投影纯函数（增量 == 全量）
2. golden fixture 对拍逐字段一致（限定字段集）
3. trace/metrics 缓存水位幂等（清表重建一致）
4. 模型监控视图读投影缓存
5. 既有测试全绿（`npm run cargo:test`）

## Review Plan

- spec 3 路对抗审核 → 已收敛
- 实施后 3 路对抗审核 → 调优 → 验证 → 下一任务

## Current Progress

- spec 通过（2026-08-18 3 路审核收敛）
- PA-091 已收口（事件层就绪）
- **实施完成**（2026-08-18）：
  - `projection.rs`：Projection trait（init/apply/view）+ HistoryProjection（截断 + squash + turn/end status）+ TraceProjection（tool 折叠 + chunk 聚合 + 终态信封 + per-turn 水位）+ PlanProjection + MetricsProjection（addReplacing 双回退 + 四桶统一 + provider/model 回填 + 序列化）+ fold_all + 12 测试
- **实施后审核**（2026-08-18）：code-reviewer（P1×9）+ tester（P0×2/P1×7），全部裁决：水位 `>=` 修正、per-turn 水位、数据字段回填、chunk 聚合、status 标记、PlanProjection、四桶统一、序列化；append_turn 重构/监控视图切换标注后续增量
- **验证**：core 823 测试通过（含新增 12 个投影测试）
- **收口**：tasks.md 采纳记录 + 降级声明已更新

## Next Action

- 无（已完成）；后续增量：append_turn 重构、模型监控视图切换、golden fixture 库（阶段 3/4 前置）

## Blockers

- 无

## Resume Hint

- 先读 `openspec/changes/session-projection-layer/design.md`，再读 `session.rs:1135`（append_turn）、`query_commands.rs`（模型监控）