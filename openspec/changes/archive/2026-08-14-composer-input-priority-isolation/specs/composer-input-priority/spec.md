# composer-input-priority-isolation Delta

## ADDED Requirements

### Requirement: Input events responsive under load

Composer input events SHALL be processed promptly while the main thread is busy with trace/conversation rendering; the draft SHALL update synchronously.

#### Scenario: Draft updates synchronously

- GIVEN trace rendering work is scheduled at low priority
- WHEN the user types in the composer
- THEN the draft SHALL update synchronously (no deferral)
- AND the input event handler SHALL not be deferred

#### Scenario: IME composition unaffected

- GIVEN the user is composing with an IME (Chinese input)
- WHEN composition events fire
- THEN composition SHALL proceed without dropped events
- AND Enter during composition (candidate confirmation) SHALL NOT trigger submission

### Requirement: Deferred trace rendering

Trace panel rendering SHALL be schedulable at low priority so it does not block input handling.

#### Scenario: Deferred update final consistency

- GIVEN the trace panel is expanded during streaming
- WHEN trace data updates arrive
- THEN the panel SHALL eventually render the latest data
- AND no update SHALL be lost (deferral is acceptable, dropping is not)

#### Scenario: Merged scheduling

- WHEN multiple trace updates arrive within one idle window
- THEN they SHALL merge into a single render pass (latest-wins)

#### Scenario: Idle scheduling fallback

- WHEN `requestIdleCallback` is unavailable (e.g. test environment)
- THEN the fallback scheduling mechanism SHALL be used
- AND behavior SHALL remain correct

#### Scenario: Session switch cancels pending work

- WHEN a session switch occurs while a low-priority render is pending
- THEN the pending task SHALL be cancelled
- AND the new session SHALL render its own data

### Requirement: Draft persistence unaffected

The composer draft SHALL be written synchronously and readable immediately regardless of rendering work.

#### Scenario: Draft during busy render

- GIVEN rendering is busy
- WHEN the user types
- THEN the draft SHALL be updated and readable immediately