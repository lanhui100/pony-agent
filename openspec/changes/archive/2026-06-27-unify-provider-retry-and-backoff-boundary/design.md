# Design: Unify Provider Retry And Backoff Boundary

## 1. Three-Layer Architecture

### Layer 1 — Request-level Retry (Core)

**Owner:** `ProviderRetryPolicy` in `crates/pony-agent-core/src/agent/retry.rs`

Scope: auto-retry a single provider HTTP request (sync or stream establishment) when the failure is classified as transient.

- Runs inside `retry_provider_timeout()` and `retry_tool_timeout()`.
- Uses exponential backoff with configurable jitter, max retries, and total budget.
- Completely synchronous within a single request: no delta has been emitted to any consumer.
- Stream establishment retries only before the first delta; once `streamed_any_delta = true`, the retry loop aborts.

```
retry_provider_timeout(label, || operation())
  ┌─ for attempt in 0..=max_retries
  │   ├─ compute_delay(attempt) → sleep
  │   ├─ operation()
  │   │    ├─ Ok → return Ok
  │   │    └─ Err(err) → classify(err) → decide(failure, budget, attempt, retry_after, stream_state)
  │   │         ├─ Retry → continue loop
  │   │         ├─ Abort  → return Err(abort)
  │   │         ├─ Escalate → return Err(escalate)
  │   │         └─ Fallback → return Err(fallback)
  │   └─ budget.record_attempt(delay)
  └─ return Err(exhausted)
```

### Layer 2 — Phase-level Fallback (Core)

**Owner:** `Provider` methods in `crates/pony-agent-core/src/agent/provider.rs`

Scope: when a request-level retry fails or the error is non-retryable, attempt an alternative strategy within the same turn phase before escalating.

Current fallback paths (all in `provider.rs`):

| Source Phase | Fallback Target | Trigger |
|---|---|---|
| `followup_stream` | `followup_sync` (`continue_with_tool_result`) | Stream retry exhausted or non-retryable stream error |
| `followup_sync` (fallback) | `local_tool_followup_fallback_response` | Sync retry exhausted or non-retryable sync error |
| `decision_stream` | (no fallback today — escalates to turn failure) | Stream retry exhausted |

Layer 2 is where stream→sync fallback safety is enforced: `StreamState::may_stream_to_sync_fallback()` returns `true` only for `NoDelta` and `ReasoningOnly`. If visible text or a tool call has been emitted, stream→sync fallback is prohibited and the phase escalates to Layer 3.

### Layer 3 — Turn-level Retry (Control Plane)

**Owner:** Frontend + `start_graph_run_stream` entry (control-plane action)

Scope: retry an entire turn after terminal failure. This is NOT an automatic retry — it requires explicit user or control-plane action via the existing `start_graph_run_stream` / `startReason` mechanism.

- Frontend existing whole-turn auto-retry timer (`runtime.ts`) SHALL be retired.
- The only way to re-run a failed turn is through an explicit control-plane action with a stable `startReason` (e.g. `retry_failed_turn`).
- Turn-level retry consumes its own budget (not the per-request budget).
- Turn-level retry does NOT replay individual request-level retries — it starts a fresh turn.

## 2. Escalation Contract

```
Layer 1 (request retry) ──exhausted/abort──→ Layer 2 (phase fallback)
Layer 2 (phase fallback) ──exhausted/abort─→ Layer 3 (turn-level retry)
```

Key rules:
- Layer 1 never directly escalates to Layer 3 — Layer 2 always gets a chance.
- Layer 2 may decide there is no valid fallback (e.g. visible text already emitted) and escalate directly to Layer 3.
- Layer 3 is always an explicit action, never automatic.
- Each layer has its own budget independent of others.
- Escalation carries a structured reason string so the receiving layer can log and decide.

Decision flow with escalation:

```
ProviderRetryPolicy.decide()
  ├─ stream state unsafe? → Abort (→ phase fallback / escalation)
  ├─ non-retryable? → Abort (→ phase fallback / escalation)
  ├─ budget exhausted? → Escalate (→ phase fallback / turn-level)
  └─ all clear → Retry (stay in Layer 1)
```

## 3. Error Classification

Defined in `FailureKind` enum in `retry.rs`:

| Variant | Classification Logic | Examples |
|---|---|---|
| `TransientRetryable` | timeout, 429, 5xx, connection errors | "type=timeout", "503 Service Unavailable", "connection reset" |
| `NonRetryable` | 4xx (except 429), server-side semantic errors | "400 Bad Request", "401 Unauthorized", "404 Not Found" |
| `RequiresRequestMutation` | context/payload size errors | "context too large", "max_tokens exceeded", "413 Payload Too Large" |
| `UnsafeToRetry` | Stream has delivered partial output | (not used by `classify()` — set by `decide()` based on stream state) |

`classify()` does NOT consider stream state — it only inspects the error message string. Stream safety is enforced by `decide()`.

`RequiresRequestMutation` is functionally non-retryable in the current retry loop (no mutation logic exists), but it is separated so that future code can distinguish "retry with smaller context" from "permanent failure".

## 4. Stream Safety Boundary

Defined in `StreamState` enum in `retry.rs`.

### State Machine

```
PreConnection ──ConnectionEstablished──→ NoDelta
NoDelta ──Reasoning──→ ReasoningOnly
NoDelta ──Text/MixedReasoningText──→ VisibleTextStarted
NoDelta ──ToolCall──→ ToolCallStarted
PreConnection ──Text──→ VisibleTextStarted
PreConnection ──ToolCall──→ ToolCallStarted
ReasoningOnly ──Text/MixedReasoningText──→ VisibleTextStarted
ReasoningOnly ──ToolCall──→ ToolCallStarted
```

### Safety Rules

| Stream State | `may_auto_retry()` | `may_stream_to_sync_fallback()` |
|---|---|---|
| `PreConnection` | ✅ true | ❌ (no connection yet) |
| `NoDelta` | ✅ true | ✅ true |
| `ReasoningOnly` | ✅ true | ✅ true |
| `VisibleTextStarted` | ❌ false | ❌ false |
| `ToolCallStarted` | ❌ false | ❌ false |

Once `VisibleTextStarted` or `ToolCallStarted` is reached:
- No request-level auto-retry.
- No stream→sync fallback.
- Any failure at this point escalates to turn-level failure.
- State is monotonic (never rolls back to a less-committed state).

### Implementation Boundary

- `streamed_any_delta: bool` in `provider.rs` is a simplified pre-StateMachine guard used today. The new `StreamState` in `retry.rs` provides a richer model that differentiates reasoning from visible text.
- The transition to `StreamState`-based safety checking is a spec goal for the implementation phase; the existing boolean guard is an acceptable intermediate step.

## 5. Budget / Retry-After Semantics

### Budget (`RetryBudget`)

Each request-level retry loop carries its own `RetryBudget`:

| Parameter | Default (provider) | Default (tool) | Description |
|---|---|---|---|
| `max_retries` | 4 (5 total attempts) | 1 (2 total attempts) | Max number of retry attempts |
| `total_budget_ms` | 30,000 | 30,000 | Total wall-clock budget for retry loop (including execution time) |

Budget exhaustion checks both dimensions: `attempts_used >= max_retries || elapsed_ms >= total_budget_ms`.

- `record_attempt(delay_ms)` — called after each sleep before the next attempt.
- `record_execution(execution_ms)` — called after success to track total wall time.
- `exhausted_reason()` returns `attempt_limit_exhausted` or `time_budget_exhausted` for telemetry.

### Retry-After

`RetryAfter` parser handles the `Retry-After` HTTP header (seconds format):

- `effective_delay(backoff_delay_ms, remaining_budget_ms)`:
  - If `Retry-After` is 0 or exceeds remaining budget → return `None` (do not retry).
  - Otherwise → return `max(backoff_delay, retry_after_ms)` capped to `remaining_budget_ms`.

The `RetryAfter` value is respected by `ProviderRetryPolicy.decide()` when provided. Today the existing `retry_provider_timeout` wrapper passes `None` for `retry_after`; the spec requires the caller to parse and forward the header value.

## 6. Telemetry Requirements

### Per-retry attempt events

Each retry attempt SHALL produce a structured log entry containing:

| Field | Description |
|---|---|
| `label` | Operation name (e.g. "decision", "followup_sync") |
| `attempt` | 1-based attempt number |
| `max_attempts` | Configured max attempts |
| `failure` | Classified failure kind |
| `decision` | Retry decision (Retry / Abort / Escalate / Fallback) |
| `delay_ms` | Computed delay before this attempt |
| `elapsed_ms` | Total elapsed time in retry loop |
| `budget_remaining` | Remaining budget in ms |
| `stream_state` | Current stream state (if applicable) |

### Layer boundary events

Each escalation or fallback transition SHALL produce a structured event:

| Event | Meaning |
|---|---|
| `request_retry_exhausted` | Layer 1 exhausted → entering Layer 2 |
| `phase_fallback_triggered` | Layer 2 activating alternative path |
| `phase_fallback_exhausted` | Layer 2 exhausted → escalating to Layer 3 |
| `turn_retry_requested` | Layer 3 explicit retry initiated |
| `stream_safety_blocked_retry` | Stream state blocked an otherwise valid retry |
| `unsafe_to_retry_aborted` | UnsafeToRetry classification led to immediate abort |

### Metrics (for monitoring dashboard)

- `retry.total_attempts` — counter
- `retry.failures_by_class` — counter tagged by `FailureKind`
- `retry.decision` — counter tagged by `RetryDecision`
- `retry.budget_exhausted` — counter tagged by reason
- `retry.latency_ms` — histogram of retry loop duration
- `retry.delay_ms` — histogram of per-attempt delay

## 7. Deterministic Test Strategy

### Principle

Avoid real-time `thread::sleep()` in tests. Use dependency injection for time.

### Abstractions

| Abstraction | Role | Test Double |
|---|---|---|
| `Sleeper` trait | Injectable sleep | `FakeSleeper` (records total slept, returns immediately) |
| `ProviderRetryPolicy` | Pure decision function | No mocking needed — decision logic is deterministic given inputs |
| `RetryBudget` | Pure budget tracking | Direct construction |
| `RetryAfter` | Pure parse + compute | Direct construction |
| `FailureKind` | Pure classification | Direct construction |
| `StreamState` | Pure state machine | Direct construction |

### Test Categories

**A. Pure strategy tests** (no sleep, no I/O):

| Test | Coverage |
|---|---|
| `compute_delay` | Exponential values, max clamping, jitter kinds |
| `stream_state_transition` | All valid transitions, monotonicity, no-rollback |
| `stream_state_may_auto_retry` | Each state's safety predicate |
| `retry_budget_exhausted` | Attempt limit and time budget exhaustion |
| `failure_classification` | Each classifier rule (timeout, 429, 4xx, 5xx, context_too_large) |
| `decide` | All code paths: retry, abort, escalate, fallback combinations |
| `retry_after` | Parse, effective delay with/without budget |
| `should_fallback_to_sync` | True/false for each stream state + failure kind |

**B. Integration tests with injected sleeper:**

| Test | Coverage |
|---|---|
| `retry_with_policy_succeeds_on_first_try` | No retry needed |
| `retry_with_policy_aborts_on_non_retryable` | Classification leads to immediate abort |
| `retry_with_policy_exhausts_and_escalates` | Full retry loop exhaustion |
| `retry_with_policy_cancelled` | Cancellation via AtomicBool |

**C. Behavior-driven integration tests** (via `retry_with_policy`):

| Test | Coverage |
|---|---|
| Timeout → retry → success on attempt N | Backoff behavior |
| Non-retryable error → immediate abort | No backoff delay |
| Stream state at VisibleTextStarted → abort | Stream safety |
| Retry-After > budget → skip retry | Budget enforcement |
| Multiple transient failures → budget exhausted → Escalate | Escalation contract |
| Cancellation mid-retry → immediate Abort | Cooperative cancellation |

### Anti-patterns (explicitly excluded from test strategy)

- `std::thread::sleep(Duration::from_secs(N))` in any test (use `FakeSleeper`).
- Tests that depend on wall-clock timing to verify backoff (verify budget records instead).
- Integration tests that exercise the full network stack to verify retry (mock the operation closure).
