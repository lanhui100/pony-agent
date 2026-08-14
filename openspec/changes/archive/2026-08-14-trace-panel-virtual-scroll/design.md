# Design

## Decision Summary

1. **单层扁平虚拟行**：turn-header + timeline-entry + detail-row 投影为同一扁平行数组，前缀和高度 + binarySearch 窗口。
2. **内嵌独立滚动容器**：展开后的 Trace body 用独立 `ScrollArea`（不共享侧栏滚动），虚拟化坐标自包含。
3. **行高策略**：固定基准行高（turn-header/timeline-entry）+ 展开详情行 **DOM 实测 + 缓存**。
4. **首挂载定位最新 turn**；流式更新时 Trace 末行底部跟随。
5. 视口度量注入（ref/测试 stub），ResizeObserver polyfill。

## Chosen Direction

### 1. 数据投影（纯函数，可测）

```ts
// 消费 PA-086 投影行（ProjectedTurn[]），投影为扁平虚拟行
interface VirtualRow {
  key: string;            // turnId + entryId 稳定标识
  kind: "turn-header" | "timeline-entry" | "detail-row";
  height: number;         // 基准高度；detail-row 为 DOM 实测缓存值
  turnIndex: number;
  entryIndex?: number;
}
```

- 纯函数 `projectTraceRows(turns, expandedState)`：与渲染解耦，便于单测。
- 行高常量：turn-header 基准、timeline-entry 基准、detail-row 初始估算（挂载后 DOM 实测更新缓存）。

### 2. 虚拟窗口计算

```ts
const viewport = { top: scrollTop, height: clientHeight };
const startIndex = binarySearch(rows, viewport.top - OVERSCAN);   // OVERSCAN = 3 行
const endIndex = binarySearch(rows, viewport.top + viewport.height + OVERSCAN);
const visibleRows = rows.slice(startIndex, endIndex);
```

- `padding-top`（startIndex 前累计高度）+ `padding-bottom`（endIndex 后累计高度）占位，滚动条真实。
- 前缀和数组 `prefixHeights`，`binarySearch` O(log n)。

### 3. 内嵌独立滚动容器

```html
<!-- 现状：整个侧栏一个 ScrollArea，Trace 是其中一段 -->
<ScrollArea class="min-h-0 flex-1" viewport-class="px-4 pt-10 pb-4">
  ...HomeStatusPanel / HomeToolsPanel / HomeTracePanel / ...
</ScrollArea>

<!-- 改为：Trace body 展开时内嵌独立 ScrollArea -->
<ScrollArea class="min-h-0 flex-1" viewport-class="px-4 pt-10 pb-4">
  ...HomeStatusPanel / HomeToolsPanel / HomeTracePanel(header) ...
</ScrollArea>
<!-- HomeTracePanel 内部： -->
<div v-if="open" class="collapsible-body">
  <ScrollArea ref="traceBodyScrollRef" class="max-h-[24rem]">
    <div :style="{ paddingTop, paddingBottom }">
      ...visibleRows 渲染...
    </div>
  </ScrollArea>
</div>
```

- UX 变更显式声明：Trace body 为内部滚动区域（原为整个侧栏滚动）。
- 视口度量从 `traceBodyScrollRef` 的 viewportEl 读取。

### 4. 展开详情行高度

- 首次展开：估算高度渲染 → 挂载后 `getBoundingClientRect()` 实测 → 写回缓存 → 投影重建（前缀和更新）→ 触发行偏移修正（±1 行内）。
- 缓存 key：`turnId + entryId`；会话切换/组件卸载清空。

### 5. 首屏定位与底部跟随

- 首挂载：`watch latestTurnId` 展开后，`nextTick` 滚动到最新 turn 行（binarySearch 定位）。
- 流式更新：视口在 Trace 底部（`scrollTop + clientHeight >= scrollHeight - FOLLOW_THRESHOLD`）时滚动到底；用户上滑后不抢滚。

### 6. 测试

- 投影纯函数单测（行数/高度/前缀和/窗口边界）。
- 视口注入：`traceBodyScrollRef` 提供 stub 契约（clientHeight/scrollTop 可注入）。
- ResizeObserver：jsdom 无实现，测试中 mock 或跳过。
- 50+ turn 渲染行数上限断言（公式化）。

## Edge Cases

- 空 turns：投影为空数组，渲染空态。
- 展开行在视口边缘：OVERSCAN 覆盖，不闪烁。
- 流式新增 turn：投影重建，前缀和更新；底部跟随逻辑处理。
- 会话切换：投影随 turns prop 重建，滚动位置重置到顶（与现状一致）。
- 窗口 resize：clientHeight 变化触发重算（ResizeObserver）。
- 全部 detail 展开：行数上限仍公式化成立（行数多但受视口约束）。
- 焦点/无障碍：虚拟化下视口外元素的键盘焦点不可达——记录为已知限制，后续候选。