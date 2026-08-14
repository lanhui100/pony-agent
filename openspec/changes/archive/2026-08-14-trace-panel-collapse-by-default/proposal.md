# trace-panel-collapse-by-default

## Background

流式输出期间主对话区卡顿、无法及时发送新消息（用户报告的核心问题）。根因之一是 `HomeSidebar.vue` 的 `activePanel` 默认值为 `"trace"`，`HomeTracePanel`（1488 行巨型组件）**始终挂载、始终计算**——内部用 `:data-open` 折叠，但所有 computed（`turnTimelineCache`、`orderedTurnTraces` 等）在折叠状态下仍全量运行，与 `HomeWorkspace`（1736 行）在同一渲染周期内响应 `traceTimeline` 更新（流式期间每 200ms 一次）。

dsh 的对比参考：dsh 用视图 tab 分离（对话/轨迹二选一渲染），pony 保持侧边栏布局，因此采用"初始化折叠 + 折叠时跳过计算"作为等价隔离手段。

## Goals

- trace 面板**初始化折叠**（默认不展开），消除启动即渲染的开销。
- 折叠状态下 `HomeTracePanel` 的 body（timeline 内容）**不挂载、不渲染、不执行面板内 timeline 投影**。
- toggle header 常驻，展开/折叠交互与现状一致（`togglePanel('trace')`），展开后功能完整。

## Non-goals

- 不改为独立 tab / 视图切换（后续候选）。
- 不做虚拟滚动（PA-085）。
- 不做快照投影（PA-086）。
- 不做输入框优先级隔离（PA-087）。
- 不承诺"整个应用零 trace 计算"（HomeStatusPanel 计数链等父组件派生归 PA-086 快照范围）。

## Scope

- `src/components/HomeSidebar.vue`：`activePanel` 默认值 `"trace"` → `""`。
- `src/components/HomeTracePanel.vue`：toggle header 常驻；`collapsible-body` 用 `v-if="open"` 懒挂载（折叠时不挂载 body，不执行面板内 timeline 投影）。
- 顺手修复：`HomeSidebar.vue` 的 `liveTraceTurn` 折叠态不再 stamp `updatedAt: Date.now()`（消除缓存必 miss 的既有问题）。
- 前端单测：默认折叠（body 未挂载）、展开后正常渲染、折叠时无 body DOM。

## Risks

- 折叠状态丢失：`activePanel` 是组件内 ref，无持久化——现状即如此，本卡不引入持久化（保持最小改动）。
- 展开时首次挂载渲染延迟：可接受（展开是低频用户动作）。
- 交互状态（activeTurnId/detail 展开）随 body 卸载清空：与现状折叠行为一致，非回归；文档注明。
- 展开后行为回归：展开路径必须与现状完全一致，测试覆盖。

## Validation

- 前端 vitest：新增默认折叠（body 未挂载）、展开渲染、折叠时无 body DOM 用例；既有 `HomeSidebar.spec.ts` 更新后全绿。
- `npm run build` 通过。
- 手动验证：启动后 trace 折叠；展开后 timeline 完整；流式期间展开面板更新正常。
- 跨卡：实施前用 `setupLongTaskObserver`（App.vue 已有）采集 longtask 基线，实施后对比。