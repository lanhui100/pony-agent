# trace-panel-virtual-scroll

## Background

`HomeTracePanel.vue` 渲染**所有 turn 的完整 timeline**（每 turn 含 model/tool/checkpoint 条目，条目可展开详情行），长会话时 DOM 节点数量爆炸。流式期间 `traceTimeline` 每 200ms 更新一次，全量 DOM 重渲染是主线程卡顿的根因之一。

参考：dsh 的轨迹视图（`TrajectoryTable`）用虚拟滚动（`trajectory-virtual-rows.ts`）只渲染视口内行，长会话不炸 DOM。本卡为 `HomeTracePanel` 引入同类机制。

3 路对抗审核（2026-08-14）确认的关键约束：

- **单层扁平虚拟行**（turn-header + timeline-entry + detail-row 同层），不做两层虚拟化。
- **Trace body 内嵌独立滚动容器**（不共享侧栏 ScrollArea，避免坐标耦合与动态面板高度抖动）。
- **展开详情行高度 DOM 实测 + 缓存**（whitespace-pre-wrap 任意换行，无法从内容行数推导）。
- **首挂载定位最新 turn**（watch latestTurnId 自动展开，50+ turn 下需初始视口定位）。
- 若自研动态高度成本失控，评估 `@tanstack/vue-virtual` 作为备选并记录决策。

## Goals

- `HomeTracePanel` 只渲染视口内的 turn 与 timeline 行，长会话 DOM 数量恒定。
- 滚动位置稳定（漂移 ≤ 1 行）、展开/折叠交互与现状一致。
- 首挂载定位最新 turn；流式更新时底部跟随。

## Non-goals

- 不改对话区（HomeWorkspace）的虚拟化（后续候选）。
- 不做快照投影（PA-086，本卡消费其产物）。
- 不改后端 trace 数据源。

## Scope

- `src/components/HomeTracePanel.vue`：turn 列表 + timeline 行单层扁平虚拟化。
- Trace body 内嵌独立滚动容器（原侧栏 ScrollArea 内保留 header 常驻）。
- 展开详情行高度 DOM 实测 + 缓存。
- 前端单测：渲染行数上限、滚动正确性、展开/折叠行为保持、首屏定位。

## Risks

- 动态行高（详情展开）导致虚拟化偏移：DOM 实测 + 缓存，展开行偏移修正。
- 内嵌滚动容器改变滚动体验（嵌套滚动）：需在 spec 显式声明 UX 变更（原为整个侧栏滚动）。
- 流式更新 + 虚拟滚动叠加：底部跟随策略（Trace 末行位置跟随）。
- jsdom 视口度量缺失（clientHeight/scrollTop/ResizeObserver）：注入策略 + stub 契约。

## Validation

- 前端 vitest：50+ turn 下渲染行数上限断言（公式化）、滚动到任意位置内容正确、展开/折叠交互用例、首屏定位、底部跟随。
- `npm run build` 通过。
- 手动验证：长会话展开面板滚动流畅；流式期间底部自动跟随正常。
- 性能基线：longtask 数量实施前后对比（跨卡）。