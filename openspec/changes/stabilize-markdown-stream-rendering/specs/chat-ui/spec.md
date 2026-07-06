# chat-ui Delta

## ADDED Requirements

### Requirement: Streaming partial markdown stabilization

The main conversation assistant response area SHALL preserve markdown rendering during streaming while stabilizing incomplete fenced code blocks through a streaming-only partial-render path.

#### Scenario: Streaming code fence is still incomplete

- WHEN an assistant message is `pending`
- AND the streamed content contains an unclosed fenced code block
- THEN the UI may render the in-progress block through a streaming-only partial markdown path
- AND the final transcript content is not mutated

#### Scenario: Assistant message reaches a terminal state

- WHEN an assistant message changes to `done`, `error`, or `cancelled`
- THEN the UI SHALL render the final assistant content through the normal markdown path without synthetic fence completion

### Requirement: Buffered streaming reveal cadence

Assistant streaming text SHALL continue to use character and time buffering thresholds before updating the visible assistant response area.

#### Scenario: Small stream deltas arrive rapidly

- WHEN multiple small assistant deltas arrive before the configured threshold
- THEN the UI does not update for every individual delta
- AND the text remains buffered until either the character threshold or time threshold is reached

#### Scenario: First visible batch should not feel stalled

- WHEN the first assistant streaming batch is being accumulated
- THEN the first visible threshold SHALL be lower than the steady-state threshold

#### Scenario: Code-fence-active batches need tighter cadence

- WHEN the assistant message is streaming inside an unclosed fenced code block
- THEN the buffered reveal logic MAY use a lower batch-size threshold than normal prose

### Requirement: Safe reveal presentation

The streaming reveal layer SHALL avoid assuming that arbitrary character boundaries are safe markdown render boundaries.

#### Scenario: Buffered content crosses markdown syntax boundaries

- WHEN a buffered assistant segment intersects markdown syntax markers that are not yet structurally safe to render incrementally
- THEN the UI SHALL fall back to a safe reveal mode for that segment
- AND the UI SHALL NOT rely on arbitrary per-character markdown DOM splitting to preserve correctness

#### Scenario: Reduced motion is preferred

- WHEN the user agent reports `prefers-reduced-motion: reduce`
- THEN reveal animations SHALL be disabled or reduced without hiding content

### Requirement: Streaming state affordance

Assistant messages that are still streaming SHALL show an in-progress visual affordance in the main conversation area.

#### Scenario: Assistant is still pending

- WHEN an assistant message status is `pending`
- THEN an in-progress indicator is visible in the assistant message area

#### Scenario: Assistant message is terminal

- WHEN an assistant message status is `done`, `error`, or `cancelled`
- THEN the in-progress indicator is not shown

### Requirement: Terminal-only assistant actions

Assistant message action controls SHALL remain hidden while the assistant message is pending and may appear after the message becomes terminal.

#### Scenario: Assistant is streaming

- WHEN an assistant message status is `pending`
- THEN copy or similar assistant message actions are not rendered

#### Scenario: Assistant is complete

- WHEN an assistant message status is `done`, `error`, or `cancelled`
- THEN copy or similar assistant message actions may be rendered

### Requirement: Runtime rollback and observability

The streaming render optimization SHALL be tunable and reversible at runtime for local validation and rollback.

#### Scenario: Optimization is disabled locally

- WHEN the runtime rollback flag disables the streaming optimization
- THEN the UI falls back to the baseline streaming render behavior without requiring a rebuild

#### Scenario: Streaming optimization is active

- WHEN the optimized streaming path is enabled
- THEN the frontend exposes enough debug information to inspect flush cadence, partial-render usage, or equivalent reveal metrics during validation
