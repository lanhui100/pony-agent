# trace-panel-collapse Specification

## Purpose

规范 trace 面板默认折叠与懒挂载行为：应用启动时面板折叠、body 不挂载以消除冻结引用污染与首屏开销；header 常驻可随时展开；折叠时 live trace 计算不污染 memo 缓存。

## Requirements

### Requirement: Trace panel SHALL start collapsed with lazy-mounted body

The trace panel SHALL be collapsed on application start; the trace body (timeline content) SHALL NOT be mounted until the user expands the panel. The toggle header SHALL remain mounted in both states.

#### Scenario: Default collapsed on start

- GIVEN the application starts
- WHEN the home workspace renders
- THEN the trace panel SHALL be collapsed
- AND the trace body SHALL NOT be mounted (no trace timeline DOM)

#### Scenario: Expand mounts full behavior

- GIVEN the trace panel is collapsed
- WHEN the user toggles it open
- THEN the trace body SHALL mount and render the full trace timeline
- AND all existing interactions (turn expand, step expand, detail rows, copy) SHALL behave as before

#### Scenario: Toggle header always available

- WHEN the user clicks the trace panel header while collapsed
- THEN the panel SHALL expand
- WHEN the user clicks the header while expanded
- THEN the panel SHALL collapse AND the trace body SHALL unmount

### Requirement: Collapsed panel SHALL NOT pollute live trace memo

While the trace panel is collapsed, the sidebar live-trace computation SHALL return null (no live turn merged), so streaming ticks do NOT stamp a fresh `updatedAt` and do NOT invalidate the memo cache.

#### Scenario: No stamp while collapsed

- GIVEN the trace panel is collapsed
- WHEN streaming updates arrive
- THEN the live trace computation SHALL return null
- AND the memo cache SHALL remain valid

#### Scenario: Expanding later renders latest data

- GIVEN the trace panel was collapsed during streaming
- WHEN the user expands it
- THEN the latest trace data SHALL render

## Implementation Notes

- As-built: `HomeTracePanel` body `v-if="open"`（header 常驻）；`TraceInspector` 以 `open` 作为 `useTraceProjection({ liveTurnEnabled: open })` 开关；`liveTraceTurn` 返回 null 切断与 store 可变数组的引用共享。
- Retroactive sync: 本规范由 `openspec/changes/archive/2026-08-14-trace-panel-collapse-by-default/specs/trace-panel-collapse/spec.md` 按 as-built 同步，未改变行为。
