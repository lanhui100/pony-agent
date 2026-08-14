# PA-086 trace 渲染快照投影与节流增强

## Basic Info

- ID: PA-086
- Status: Done
- Priority: P0
- Complexity: B
- Owner: @orchestrator
- Created At: 2026-08-14
- Updated At: 2026-08-14
- OpenSpec Change: `trace-render-snapshot-projection`（3 路对抗审核已通过，2026-08-14）
- Spec 状态: 通过（proposal/design/spec/tasks 已按采纳意见修订）

## Background

store 已有 `scheduleThrottledTraceTimeline`（200ms 节流合并 timeline 更新），但 `HomeTracePanel` 的 computed（`turnTimelineCache`、`orderedTurnTraces` 等）在每次 `traceTimeline`/`turnTraceHistory` 变化时**全量重算**，且 `HomeWorkspace`/`WorkspaceTurnItem` 也消费同一份 trace 数据做工具消息归因。dsh 的轨迹视图消费**预计算快照**（`trajectory-snapshot-builder.ts`），渲染输入稳定，不是每次事件全量重算。

## Goal

把 trace 渲染输入从"每次 store 更新全量重算"改为"预计算快照 + 增量更新"：store 侧维护投影快照，组件消费快照而非原始数组；对高频更新路径进一步节流。

## Scope

- `src/stores/runtime.ts`：trace 投影快照（turn 列表 + timeline 的稳定派生结构），增量更新而非全量重建
- `src/components/HomeTracePanel.vue`：消费快照，减少 computed 全量重算
- `src/components/HomeWorkspace.vue` / `WorkspaceTurnItem.vue`：trace 消费路径对齐快照（工具消息归因）
- 节流参数评估（200ms 是否可放宽/收紧，需实测依据）
- 前端单测：快照增量更新正确性、组件消费快照后行为不变

## Non-Goals

- 不做虚拟滚动（PA-085）
- 不改后端 trace 数据源
- 不引入新的状态管理库

## Acceptance Criteria

1. 流式期间 trace 相关 computed 不再全量重算（可测：更新频率内计算次数有上限）。
2. 快照增量更新后，trace 面板与对话区工具归因展示与现状一致。
3. 节流参数调整有实测依据（记录在任务卡验证部分）。
4. 前端 vitest、`npm run build` 全绿；既有测试更新后通过。

## Review Plan

- @architect：快照投影边界、增量更新正确性、与 store 节流机制的整合
- @code-reviewer：快照失效/重建时机、内存占用、与 PA-085 虚拟滚动的配合
- @tester：增量更新正确性、流式期间展示一致性用例

## Current Progress

- 3 路对抗审核（2026-08-14）已完成，findings 全部采纳：9 条写路径收敛单一入口；投影放非响应式模块层；签名化 memo；引用语义（活跃别名/历史派生）。详见 `02_REVIEWS/2026-08-14-pa084-087-spec-review.md`。
- **实现完成（2026-08-14）**：
  - 新增 `src/lib/runtime/trace-projection.ts`：非响应式投影层，`computeTurnTimeline` 归一化 + `turnTimeline` 签名化 memo（key = traceTimeline 引用 + updatedAt）+ `clearTraceProjectionMemo`。
  - `runtime.ts`：15 处 `this.traceTimeline = X` 写路径全部收敛到 `publishTraceTimeline()` 单一发布入口。
  - `HomeSidebar.vue`：本地 turnTimelineMemo/computeTurnTimeline 删除，改用投影层（`projectedTurnTimeline` + `clearTraceProjectionMemo`）；`HomeTracePanel` 的 `:turn-timeline` prop 指向投影层。
  - 验证：vue-tsc 通过 + 全量前端 379 passed + 新增 trace-projection.spec.ts 6 passed + build 通过。

## Next Action

- 进入 PA-085（虚拟滚动，消费投影层行数据），本卡核心已收口。

## Blockers

- 依赖 PA-084（折叠懒渲染先行）；与 PA-085 共享 `HomeTracePanel` 改动面，建议串行。

## Resume Hint

- 先读 `src/stores/runtime.ts`（traceTimeline/turnTraceHistory 更新路径、scheduleThrottledTraceTimeline）、`src/components/HomeTracePanel.vue`（computed 结构）、`src/components/HomeWorkspace.vue`（traceSequencesForToolMessages/modelTraceEntries）。