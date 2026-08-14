# trace-panel-collapse-by-default Delta

## ADDED Requirements

### Requirement: Trace panel collapsed by default

The trace panel SHALL be collapsed on application start; the trace body (timeline content) SHALL not be mounted until the user expands the panel.

#### Scenario: Default collapsed

- GIVEN the application starts
- WHEN the home workspace renders
- THEN the trace panel SHALL be collapsed
- AND the trace body SHALL not be mounted (no trace timeline DOM)

#### Scenario: Expand restores full behavior

- GIVEN the trace panel is collapsed
- WHEN the user toggles it open
- THEN the trace body SHALL mount and render the full trace timeline
- AND all existing interactions (turn expand, step expand, detail rows, copy) SHALL behave as before

#### Scenario: Collapse skips body rendering

- GIVEN the trace panel is collapsed
- WHEN trace timeline data updates (streaming)
- THEN the trace body SHALL remain unmounted
- AND expanding later SHALL render the latest data

### Requirement: Toggle header always available

The trace panel toggle header SHALL remain mounted regardless of panel state, so the panel can always be re-expanded.

#### Scenario: Toggle from collapsed

- WHEN the user clicks the trace panel header while collapsed
- THEN the panel SHALL expand

#### Scenario: Toggle from expanded

- WHEN the user clicks the trace panel header while expanded
- THEN the panel SHALL collapse
- AND the trace body SHALL unmount

### Requirement: Live trace stamp fix

The sidebar's live trace computation SHALL not stamp `updatedAt: Date.now()` while the trace panel is collapsed, so the memo cache is not invalidated every streaming tick.

#### Scenario: No stamp while collapsed

- GIVEN the trace panel is collapsed
- WHEN streaming updates arrive
- THEN the live trace computation SHALL not stamp a new `updatedAt`
- AND the memo cache SHALL remain valid