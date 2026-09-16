# trace-panel-virtual-scroll Specification

## Purpose

规范 trace 面板 turn 级虚拟滚动：外层 turn 列表只渲染视口内窗口（估算高度 + OVERSCAN 缓冲吸收偏差），turn 内部保持嵌套渲染；展开后面板使用独立滚动容器，坐标自包含。

## Requirements

### Requirement: Trace panel SHALL virtualize at turn granularity

The trace panel SHALL render only the turn window intersecting the current viewport. Turn heights SHALL be estimated (turn header + per-entry heights + expanded-detail estimates); the virtual window SHALL be padded on both sides by an overscan buffer to absorb estimation error. Rendered DOM SHALL NOT grow linearly with total turn count.

#### Scenario: Long session bounded DOM

- GIVEN a session with 50+ turns
- WHEN the trace panel renders
- THEN only the viewport window of turns (plus overscan buffer) SHALL be mounted
- AND the mounted count SHALL NOT grow linearly with total turns

#### Scenario: Scroll to arbitrary position

- WHEN the user scrolls to any position in the panel
- THEN the visible turn grouping and timeline entries SHALL be correct at that position
- AND the scroll position SHALL NOT drift beyond ±1 row after expand/collapse interactions

#### Scenario: Initial position at latest turn

- GIVEN the panel mounts with many turns
- WHEN the latest turn is auto-expanded
- THEN the initial viewport SHALL be positioned at the latest turn

#### Scenario: Streaming keeps bottom follow

- WHEN trace timeline updates during streaming and the panel is scrolled to the trace bottom
- THEN the view SHALL keep following the latest entries without jumping

### Requirement: Interactions SHALL be preserved under virtualization

Existing expand/collapse interactions SHALL behave identically with virtualization enabled.

#### Scenario: Expand turn

- WHEN the user expands a turn group
- THEN its timeline entries SHALL render correctly with measured heights

#### Scenario: Expand detail row

- WHEN the user expands a timeline entry detail row
- THEN the detail SHALL render correctly
- AND the scroll position SHALL NOT drift beyond ±1 row

#### Scenario: Collapse during streaming

- WHEN the panel is collapsed and re-expanded during streaming
- THEN the latest data SHALL render with virtualization intact

### Requirement: Expanded trace body SHALL use an independent scroll container

The expanded trace body SHALL render within its own scroll container (not the shared sidebar ScrollArea), so virtualization coordinates are self-contained and other sidebar panels scroll independently.

#### Scenario: Nested scroll container

- WHEN the trace body expands
- THEN it SHALL render within its own scroll container
- AND the sidebar's other panels SHALL scroll independently

## Implementation Notes

- As-built（与归档 delta 的偏离，显式声明）：
  - 归档 delta 写"单层扁平虚拟行 + `ceil(viewportHeight / minRowHeight) + 2 * OVERSCAN` 行数 bound"；实际落地为 **turn 级虚拟化**（`src/lib/runtime/trace-virtual-scroll.ts`：`estimateTurnHeight` 高度估算 + `buildTurnPrefixHeights` 前缀和 + `findTurnIndexAtScroll` 二分定位 + `computeVirtualTurnWindow` 窗口，`OVERSCAN_TURNS = 3` 双侧缓冲），turn 内部保持嵌套渲染。行数 bound 公式不成立，未同步。
  - 归档 delta 的"Viewport measurement injection"（jsdom 注入 seam、ResizeObserver stub）是可测性手段而非用户可见行为，未同步为长期行为需求；覆盖它的单测位于 `tests/trace-virtual-scroll.spec.ts`。
- Retroactive sync: 本规范由 `openspec/changes/archive/2026-08-14-trace-panel-virtual-scroll/specs/trace-panel-virtual-scroll/spec.md` 按 as-built 重写同步。
