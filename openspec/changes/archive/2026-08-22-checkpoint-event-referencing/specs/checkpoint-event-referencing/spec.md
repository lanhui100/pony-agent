# checkpoint-event-referencing Delta

## ADDED Requirements

### Requirement: Node event referencing

Each `HistoryNode` SHALL reference the event range it covers instead of embedding a full snapshot.

#### Scenario: Reference over snapshot

- GIVEN a committed turn that creates a history node
- WHEN the node is persisted
- THEN the node SHALL store `event_seq_range` (start, end) referencing `turn_events`
- AND the embedded snapshot fields SHALL NOT be written for new nodes (legacy nodes without the range SHALL fall back to embedded data)

#### Scenario: Node size bound

- WHEN nodes are created for N turns
- THEN the total node storage SHALL be O(N) metadata (references), not O(N × history size)
- AND a new node's JSON SHALL be bounded (e.g. < 10KB, benchmark assertion)

### Requirement: Checkout as watermark move

Checkout SHALL append a `checkpoint/checkout` event and move the projection watermark to the target node's range end, without copying data.

#### Scenario: Checkout without copy

- GIVEN a checkout to node X with `event_seq_range = (a, b)`
- WHEN checkout executes
- THEN a `checkpoint/checkout` event SHALL be appended
- AND the projection watermark SHALL move to b
- AND no snapshot data SHALL be copied (node embedded fields SHALL be empty for new nodes)

#### Scenario: Checkout reversibility

- WHEN a checkout has been executed
- THEN the checkout itself SHALL be an event in the log
- AND a subsequent checkout SHALL be able to move the watermark back forward (checkout is not destructive)
- AND a double checkout (watermark backward then backward again) SHALL behave correctly (each checkout appends its own event)

#### Scenario: Metrics cache rollback

- WHEN a checkout moves the watermark backward to b
- THEN the `MetricsProjection` cache SHALL roll back in sync (rows above b deleted, incremental re-fold)
- AND history and metrics caches SHALL NOT diverge (both reflect the same watermark)

#### Scenario: Legacy node checkout

- GIVEN a legacy node without `event_seq_range`
- WHEN checkout executes
- THEN the legacy embedded-snapshot path SHALL be used (behavior unchanged)

### Requirement: Fork as event prefix sharing

Fork SHALL share the event prefix and fork only the watermark.

#### Scenario: Fork shares events

- GIVEN a fork from node X at seq b
- WHEN the fork branch continues
- THEN events before b SHALL be shared (not copied)
- AND new events SHALL append to the session event stream with the fork's `branch_id`

#### Scenario: Fork lineage

- WHEN a fork is created
- THEN a `fork/created` event SHALL be appended with `(branch_id, from_node_id)`
- AND the branch SHALL record `forked_from_branch_id` / `forked_from_node_id` (existing fields)

### Requirement: Branch visibility

Projection folds SHALL skip events belonging to branches outside the currently visible branch set.

#### Scenario: Cancelled branch events do not resurrect

- GIVEN a checkout to node X that cancels descendant turns on the same branch
- WHEN the projection folds events after the checkout
- THEN events belonging to the cancelled branch SHALL NOT appear in the folded history
- AND events belonging to the visible branch SHALL appear normally

#### Scenario: Interleaved branch events

- GIVEN branch A at watermark b and branch B appending events after b
- WHEN branch A folds incrementally
- THEN only events with `branch_id` in A's visible set SHALL be applied (B's events SHALL be skipped)

### Requirement: Time travel

Any node SHALL be viewable by folding events to its range end.

#### Scenario: Historical view

- GIVEN a node at seq b
- WHEN the user requests its state
- THEN the projection SHALL fold events [0, b] (filtered by the visible branch set) and present the resulting snapshot
- AND the presentation SHALL be identical to the snapshot the node originally captured (when events are complete)

### Requirement: Cursor version retirement

The manual `cursor_version` optimistic lock SHALL be replaced by the seq watermark as the version identity.

#### Scenario: Watermark as version

- GIVEN a checkout attempted with a stale expected watermark
- WHEN the checkout executes
- THEN the operation SHALL be rejected (expected != current watermark)
- AND the error SHALL include the current watermark
- GIVEN a legacy client that does not send an expected watermark
- WHEN the checkout executes
- THEN the operation SHALL proceed without validation (behavior identical to current)