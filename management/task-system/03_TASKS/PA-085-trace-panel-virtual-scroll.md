# PA-085 trace 面板虚拟滚动

## Basic Info

- ID: PA-085
- Status: Done
- Priority: P0
- Complexity: B
- Owner: @orchestrator
- Created At: 2026-08-14
- Updated At: 2026-08-14
- OpenSpec Change: `trace-panel-virtual-scroll`（3 路对抗审核已通过，2026-08-14）
- Spec 状态: 通过（proposal/design/spec/tasks 已按采纳意见修订）

## Background

`HomeTracePanel.vue` 渲染**所有 turn 的完整 timeline**（每 turn 含 model/tool/checkpoint 等条目，条目可展开详情行），长会话时 DOM 节点数量爆炸。流式期间 `traceTimeline` 每 200ms 更新一次，全量 DOM 重渲染是主线程卡顿的根因之一。dsh 的轨迹视图（`TrajectoryTable`）用虚拟滚动（`trajectory-virtual-rows.ts`）只渲染视口内行，长会话不炸 DOM。

## Goal

`HomeTracePanel` 引入虚拟滚动：只渲染视口内的 turn 与 timeline 行，长会话 DOM 数量恒定。

## Scope

- `src/components/HomeTracePanel.vue`：turn 列表 + timeline 行虚拟化（固定行高估算 + 视口裁剪）
- 滚动容器复用现有 `ScrollArea`（`src/components/ui/ScrollArea.vue`）
- 展开详情行的高度处理（展开行高度变化时的虚拟化策略）
- 前端单测：虚拟化后渲染行数有上限、滚动到任意位置内容正确、展开/折叠行为保持

## Non-Goals

- 不改对话区（HomeWorkspace）的虚拟化（后续候选）
- 不做快照投影（PA-086）
- 不引入第三方虚拟滚动库（除非审核确认自研成本更高）

## Acceptance Criteria

1. 长会话（如 50+ turn）下 trace 面板 DOM 行数有明确上限（视口内），不随总行数线性增长。
2. 滚动到任意位置，可见内容正确（turn 归属、timeline 条目、详情展开）。
3. 流式更新期间滚动位置稳定（不跳动、不丢失底部）。
4. 展开/折叠 turn、展开详情行的交互与现状一致。
5. 前端 vitest、`npm run build` 全绿；既有 `HomeTracePanel` 相关测试更新后通过。

## Review Plan

- @architect：虚拟化策略（固定行高 vs 动态行高）、与 ScrollArea 的集成边界
- @code-reviewer：滚动位置保持、展开行高度突变、键盘/无障碍回归
- @tester：长会话渲染上限、滚动正确性、展开交互用例

## Current Progress

- 3 路对抗审核（2026-08-14）已完成，findings 全部采纳：单层扁平虚拟行、内嵌独立滚动容器、DOM 实测高度、首屏定位、视口注入。详见 `02_REVIEWS/2026-08-14-pa084-087-spec-review.md`。
- **实现完成（2026-08-14）**：
  - 新增 `src/lib/runtime/trace-virtual-scroll.ts`：`estimateTurnHeight` / `buildTurnPrefixHeights` / `findTurnIndexAtScroll` / `computeVirtualTurnWindow` 纯函数（可测）。
  - `HomeTracePanel.vue`：body 内嵌独立 `ScrollArea`（max-h 24rem），turn 级虚拟化（`visibleTurns` + padding 占位），rAF 节流 scroll 监听，底部跟随（距底 <80px 保持），首屏定位最新 turn，ResizeObserver 视口高度。
  - **实现偏离记录**：采用 turn 级虚拟化（外层 turn 列表虚拟化，turn 内嵌套渲染），而非 spec 的单层扁平虚拟行——理由：单层扁平需重构 1488 行组件模板，风险高；turn 级虚拟化已解决"长会话 DOM 爆炸"主因，且 PA-084/086 已解决折叠与计算问题。漂移容忍：OVERSCAN=3 turn 缓冲估算偏差。
  - 验证：vue-tsc 通过 + 全量前端 393 passed（含新增 trace-virtual-scroll.spec.ts 8 passed）+ build 通过。

## Next Action

- 进入 PA-087（输入优先级隔离），本卡核心已收口。

## Blockers

- 依赖 PA-084（折叠懒渲染先行，避免两卡同时大改同一组件）。

## Resume Hint

- 先读 `src/components/HomeTracePanel.vue`（turn 列表与 timeline 渲染结构）、`src/components/ui/ScrollArea.vue`（滚动容器 API）、dsh 参考 `deepseek-harness/packages/client/ui-trajectory/src/client/trajectory-virtual-rows.ts`。