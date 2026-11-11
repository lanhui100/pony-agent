# PA-084 trace 面板初始化折叠与懒渲染

## Basic Info

- ID: PA-084
- Status: Done
- Priority: P0
- Complexity: B
- Owner: @orchestrator
- Created At: 2026-08-14
- Updated At: 2026-08-14
- OpenSpec Change: `trace-panel-collapse-by-default`（3 路对抗审核已通过，2026-08-14）
- Spec 状态: 通过（proposal/design/spec/tasks 已按采纳意见修订）

## Background

流式输出期间主对话区卡顿、无法及时发送新消息。根因之一是 `HomeSidebar.vue` 的 `activePanel` 默认值为 `"trace"`，`HomeTracePanel`（1488 行巨型组件）**始终挂载、始终计算**（内部用 `:data-open` 折叠，但 computed 全量运行），与 `HomeWorkspace`（1736 行）在同一渲染周期内响应 `traceTimeline` 更新（流式期间每 200ms 一次）。

## Goal

trace 面板**初始化折叠**（默认不展开），折叠时跳过渲染与计算；展开时才挂载/计算。保持轨迹在侧边栏的现有布局，不改为独立 tab。

## Scope

- `src/components/HomeSidebar.vue`：`activePanel` 默认值改为 `""`（或等效折叠态）
- `src/components/HomeTracePanel.vue`：折叠时跳过内部 computed 计算与 DOM 渲染（`v-if` 懒挂载或计算短路）
- 折叠/展开交互保持现有 `togglePanel('trace')` 语义
- 前端单测：默认折叠、展开后正常渲染、折叠时无 trace 计算

## Non-Goals

- 不改为独立 tab / 视图切换（后续候选）
- 不做虚拟滚动（PA-085）
- 不做快照投影（PA-086）
- 不做输入框优先级隔离（PA-087）

## Acceptance Criteria

1. 应用启动后 trace 面板默认折叠，主对话区无 trace 渲染开销。
2. 用户点击展开后 trace 面板正常渲染全部内容，交互与现状一致。
3. 折叠状态下 `HomeTracePanel` 不执行 timeline 计算（可测：折叠时无 computed 求值）。
4. 前端 vitest、`npm run build` 全绿；既有 `HomeSidebar.spec.ts` 更新后通过。

## Review Plan

- @architect：懒挂载 vs 计算短路的边界、与 store 节流机制的配合
- @code-reviewer：折叠状态持久化、展开/折叠切换的回归风险
- @tester：默认折叠、展开渲染、折叠无计算的可测性

## Current Progress

- 3 路对抗审核（2026-08-14）已完成，findings 全部采纳：P0-1 v-if 卸载删展开入口 → header 常驻 + body 懒挂载；P0-2 "无 trace 计算"不可测 → 验收改为"body 未挂载"，父组件计数链归 PA-086；顺手修复 liveTraceTurn stamp（HomeSidebar.vue:123）。详见 `02_REVIEWS/2026-08-14-pa084-087-spec-review.md`。
- **实现完成（2026-08-14）**：`activePanel` 默认值 `""`；`HomeTracePanel` body `v-if="open"` 懒挂载；`liveTraceTurn` 折叠态冻结快照（引用稳定 → memo 命中）。验证：HomeSidebar.spec.ts 26 passed（含新增 2 个默认折叠/展开测试）+ 全量前端 379 passed + build 通过。

## Next Action

- 进入 PA-086（trace 渲染快照投影），本卡已收口。

## Blockers

- 无。

## Resume Hint

- 先读 `src/components/HomeSidebar.vue`（activePanel 默认值）、`src/components/HomeTracePanel.vue`（open prop 与 computed 结构）、`src/stores/runtime.ts`（traceTimeline 节流机制）。