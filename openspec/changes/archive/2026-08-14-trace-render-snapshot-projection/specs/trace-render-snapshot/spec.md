# trace-render-snapshot-projection Delta

## ADDED Requirements

### Requirement: Single trace publication entry

All trace timeline write paths SHALL converge on a single publication entry; the projection SHALL stay consistent with the source data at all times.

#### Scenario: Write path convergence

- GIVEN any of the trace timeline write paths executes (throttled update, terminal event, restore, rollback, session switch, checkpoint load, filter)
- WHEN the trace data changes
- THEN the change SHALL flow through the single publication entry
- AND the projection SHALL reflect the change immediately

#### Scenario: Projection consistency matrix

- WHEN the projection is updated
- THEN unaffected turns SHALL retain reference equality
- AND deleted turns SHALL be evicted immediately
- AND session switch SHALL atomically clear stale projection data
- AND a pending throttled update SHALL not cross session/turn generations

### Requirement: Projection layer

The trace projection SHALL live in a non-reactive module layer (not Pinia state), exposing stable per-turn references for components.

#### Scenario: Non-reactive storage

- WHEN the projection updates
- THEN it SHALL not be serialized by devtools or persisted in history payloads

#### Scenario: Live turn aliasing

- WHEN the active turn's timeline is updated in place (e.g. model trace merge)
- THEN the projection SHALL alias the same array (in-place mutations visible)
- AND historical turns SHALL use lightweight derived projections (render-only fields)

### Requirement: Bounded recomputation

Trace consumers SHALL use signature-based memoization (reference-level caching) so streaming updates do not trigger wholesale recomputation.

#### Scenario: Bounded compute during streaming

- GIVEN streaming updates arrive at the throttled cadence
- WHEN a trace consumer renders
- THEN unchanged turn projections SHALL be served from cache (reference equality)
- AND only the affected turn SHALL be recomputed

#### Scenario: Memo invalidation correctness

- WHEN a turn's timeline changes
- THEN its memo entry SHALL invalidate
- AND other turns' memo entries SHALL remain valid

### Requirement: Consumer parity

Trace consumers (sidebar status chain, trace panel, workspace tool attribution) SHALL render identical data through the projection as they did from raw store data.

#### Scenario: Sidebar status chain

- WHEN the sidebar computes status counts
- THEN the counts SHALL match the projection's derived data

#### Scenario: Workspace tool attribution

- WHEN the workspace renders tool attribution from trace
- THEN the attribution SHALL match the projection's tool activity data
- AND required fields (toolActivities, capabilityInvocation) SHALL be present in the projection

### Requirement: Throttle parameter evidence

Any change to the trace timeline throttle interval SHALL be backed by measured evidence recorded in the task validation notes.

#### Scenario: Evidence-backed tuning

- WHEN the throttle interval is adjusted
- THEN the task card SHALL record the measurement (before/after) justifying the change