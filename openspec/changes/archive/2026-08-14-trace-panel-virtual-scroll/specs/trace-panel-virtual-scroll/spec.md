# trace-panel-virtual-scroll Delta

## ADDED Requirements

### Requirement: Virtualized trace rows

The trace panel SHALL render only the rows intersecting the current viewport; total rendered DOM rows SHALL be bounded regardless of total turn/timeline count.

#### Scenario: Long session bounded DOM

- GIVEN a session with 50+ turns
- WHEN the trace panel renders
- THEN the number of rendered row elements SHALL stay within a bounded window (viewport-based, formula: `ceil(viewportHeight / minRowHeight) + 2 * OVERSCAN`)
- AND the bound SHALL not grow linearly with total turns

#### Scenario: Scroll to arbitrary position

- WHEN the user scrolls to any position in the panel
- THEN the visible turn grouping and timeline entries SHALL be correct at that position
- AND the scroll position SHALL not drift beyond ±1 row after expand/collapse interactions

#### Scenario: Initial position at latest turn

- GIVEN the panel mounts with many turns
- WHEN the latest turn is auto-expanded
- THEN the initial viewport SHALL be positioned at the latest turn

#### Scenario: Streaming keeps bottom follow

- WHEN trace timeline updates during streaming and the panel is scrolled to the trace bottom
- THEN the view SHALL keep following the latest entries without jumping

### Requirement: Interactions preserved under virtualization

Existing expand/collapse interactions SHALL behave identically with virtualization enabled.

#### Scenario: Expand turn

- WHEN the user expands a turn group
- THEN its timeline entries SHALL render correctly (height measured from DOM, cached)

#### Scenario: Expand detail row

- WHEN the user expands a timeline entry detail row
- THEN the detail SHALL render correctly
- AND the scroll position SHALL not drift beyond ±1 row

#### Scenario: Collapse during streaming

- WHEN the panel is collapsed and re-expanded during streaming
- THEN the latest data SHALL render with virtualization intact

### Requirement: Independent scroll container

The expanded trace body SHALL use its own scroll container (not the shared sidebar ScrollArea), so virtualization coordinates are self-contained.

#### Scenario: Nested scroll container

- WHEN the trace body expands
- THEN it SHALL render within its own scroll container
- AND the sidebar's other panels SHALL scroll independently

### Requirement: Viewport measurement injection

Viewport metrics (clientHeight, scrollTop, ResizeObserver) SHALL be injectable for testability.

#### Scenario: Test stub contract

- WHEN tests run in jsdom (no layout)
- THEN viewport metrics SHALL be provided through an injection seam (prop/ref stub)
- AND ResizeObserver SHALL be polyfilled or stubbed