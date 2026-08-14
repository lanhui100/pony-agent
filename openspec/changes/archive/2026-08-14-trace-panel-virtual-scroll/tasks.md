# Tasks

- [ ] 新增纯函数投影 `projectTraceRows(turns, expandedState)`（单层扁平虚拟行 + 前缀和高度），消费 PA-086 投影行。
- [ ] `HomeTracePanel.vue`：Trace body 展开时内嵌独立 `ScrollArea`（原侧栏滚动拆出）。
- [ ] 虚拟窗口计算（scrollTop/clientHeight + OVERSCAN + binarySearch），padding 占位渲染 `visibleRows`。
- [ ] 展开详情行高度 DOM 实测 + 缓存（key: turnId+entryId，会话切换/卸载清空）。
- [ ] 首屏定位最新 turn；流式更新 Trace 末行底部跟随（用户上滑不抢滚）。
- [ ] 视口度量注入 seam（ref/stub 契约）+ ResizeObserver 处理。
- [ ] 前端单测：投影纯函数、窗口边界、50+ turn 行数上限（公式化）、滚动正确性、展开交互（±1 行漂移）、首屏定位、底部跟随。
- [ ] `npm run test:unit`、`npm run build` 通过；既有 trace 面板测试更新后全绿。

## Validation Notes

- 3 路对抗审核（2026-08-14）已采纳：P1 两层虚拟化矛盾 → 单层扁平虚拟行；P1 详情高度不可推导 → DOM 实测+缓存（备选 @tanstack/vue-virtual 决策点）；P1 ScrollArea 坐标缺失 → Trace body 内嵌独立滚动容器（UX 变更显式声明）；P1 首屏定位最新 turn；P2 视口注入 stub 契约 + ResizeObserver；P2 漂移阈值 ±1 行；P2 底部跟随定义为 Trace 末行；P3 焦点无障碍记录为已知限制；跨卡依赖顺序 084→086→085→087。详见 `02_REVIEWS/2026-08-14-pa084-087-spec-review.md`。