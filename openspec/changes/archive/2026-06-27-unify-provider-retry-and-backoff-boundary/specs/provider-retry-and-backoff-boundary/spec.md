# Provider Retry And Backoff Boundary

## Requirements

### Requirement: Provider request retry SHALL be owned by `pony-agent-core`
The system SHALL keep provider request retry and provider-phase fallback inside `pony-agent-core`, where provider requests are actually issued and follow-up phases are orchestrated.

#### Scenario: A provider decision request fails before any user-visible output is committed
- **WHEN** a provider `decision` or `followup` request fails inside core before any user-visible output has been committed
- **THEN** the retry decision SHALL be made inside `pony-agent-core`
- **AND** `src-tauri` SHALL NOT independently choose retry delays, retry counts, or failure classification
- **AND** the frontend SHALL NOT independently apply provider request backoff policy

### Requirement: The system SHALL distinguish request-level retry, phase-level fallback, and turn-level retry
The system SHALL treat provider request retry, mode fallback inside a provider phase, and whole-turn resubmission as separate semantics with separate ownership and budgets.

#### Scenario: A provider stream request fails while a turn remains otherwise recoverable
- **WHEN** a provider stream request fails during a turn
- **THEN** the system SHALL first evaluate request-level retry and phase-level fallback rules
- **AND** it SHALL NOT implicitly treat that failure as authority to resubmit the whole turn
- **AND** any whole-turn retry behavior SHALL remain an explicit higher-level orchestration concern

### Requirement: Retry layers SHALL connect through an explicit escalation contract
The system SHALL connect request-level retry, phase-level fallback, and turn-level retry through explicit machine-readable decisions rather than through implicit error bubbling or host-local retry timers.

#### Scenario: Request-level retry is exhausted
- **WHEN** a provider request has exhausted its request-level retry policy
- **THEN** the request-level logic SHALL emit an explicit result equivalent to `Escalate` or `Abort`
- **AND** it SHALL NOT silently trigger whole-turn resubmission

#### Scenario: Phase-level fallback is exhausted
- **WHEN** the current provider phase can no longer safely retry or fallback
- **THEN** the system SHALL emit an explicit result equivalent to `Escalate` or `Abort`
- **AND** any later whole-turn retry SHALL require a separate orchestration decision

### Requirement: Stream retry safety SHALL depend on visible output commitment
The system SHALL decide whether automatic retry or fallback is safe based on whether user-visible output has been committed, not merely on whether any stream metadata has appeared.

#### Scenario: A request fails before connection or before the first stream payload is established
- **WHEN** the provider request fails before stream payload commitment, including connection, DNS, TLS, or pre-body transport failure
- **THEN** the system MAY perform request-level retry subject to budget and failure classification

#### Scenario: A stream fails after reasoning deltas but before visible answer text
- **WHEN** the provider stream has emitted reasoning-only deltas
- **AND** no user-visible answer text has been emitted
- **THEN** the system MAY perform request-level retry or `stream -> sync` fallback

#### Scenario: A single stream chunk contains both reasoning and visible answer text
- **WHEN** the provider emits a chunk containing both reasoning data and user-visible answer text
- **THEN** the system SHALL conservatively treat the phase as having entered `visible_text_started`

#### Scenario: A stream fails after visible answer text has started
- **WHEN** the provider stream has emitted any user-visible answer text
- **AND** the stream later fails
- **THEN** the system SHALL NOT silently auto-retry the provider request
- **AND** the system SHALL NOT silently replay the same phase through sync fallback

#### Scenario: A stream fails after tool-call commitment has begun
- **WHEN** the provider stream has begun emitting tool-call commitment for the current phase
- **THEN** the system SHALL treat the failure as unsafe for automatic replay by default
- **AND** it SHALL require an explicit future idempotency contract before automatic replay is permitted

### Requirement: Tool-call commitment SHALL be defined at the runtime execution boundary
The system SHALL define `tool-call commitment` in terms of runtime execution intent rather than UI rendering or trace decoration alone.

#### Scenario: The provider emits a tool-call payload that the runtime is about to schedule or execute
- **WHEN** the runtime has accepted a provider tool-call payload into the tool execution pipeline
- **THEN** the system SHALL treat the phase as having entered `tool_call_started`
- **AND** the system SHALL NOT infer `tool_call_started` solely from frontend rendering state

### Requirement: Provider retry classification SHALL separate retryable, non-retryable, and unsafe-to-retry failures
The system SHALL use a provider-side failure classification that distinguishes transient retryable failures from non-retryable failures and failures whose replay safety is unknown.

#### Scenario: A provider request returns a transient infrastructure failure
- **WHEN** the provider request fails with a transient infrastructure failure such as timeout-before-visible-output, HTTP `408/429/502/503/504`, temporary DNS failure, or pre-commit connection reset
- **THEN** the system SHALL classify the failure as retryable

#### Scenario: A provider request returns a permanent request error
- **WHEN** the provider request fails with an invalid request or permanent client error such as HTTP `400/401/403/404/407/413/422`, unrecoverable TLS mismatch, or invalid request/schema failure
- **THEN** the system SHALL classify the failure as non-retryable

#### Scenario: A provider request cannot succeed again without mutating the request or external preconditions
- **WHEN** the provider request fails in a way that requires context compaction, payload reduction, credential refresh, proxy repair, or other request/external mutation before retrying
- **THEN** the system SHALL classify the failure as requiring request mutation
- **AND** it SHALL NOT automatically replay the exact same provider request

#### Scenario: A provider request fails after the response has partially committed
- **WHEN** the provider request fails after visible text or tool-call commitment has partially committed
- **THEN** the system SHALL classify the failure as unsafe-to-retry by default

#### Scenario: A provider responds with HTTP 200 but the body is empty, malformed, or contains a provider error envelope
- **WHEN** the HTTP status is successful but the body is empty, malformed, or structurally contains a provider/model-layer error such as rate limiting or overload
- **THEN** the system SHALL still map the outcome into the structured retry classification model rather than treating it as an unspecified failure

### Requirement: Structured failure classes SHALL remain distinguishable across layers
The system SHALL preserve the difference between non-retryable, requires-request-mutation, and unsafe-to-retry outcomes when reporting terminal state and escalation results.

#### Scenario: A request-level failure stops automatic replay but a higher layer still needs to decide what to do next
- **WHEN** request-level execution stops with `NonRetryable`, `RequiresRequestMutation`, or `UnsafeToRetry`
- **THEN** the terminal or escalated result SHALL preserve which class occurred
- **AND** upper layers SHALL NOT collapse those classes into a single undifferentiated error code

### Requirement: Retry budget and backoff SHALL be bounded and observable
The system SHALL apply bounded backoff and request-level retry budget rather than retrying indefinitely or without traceable decision evidence.

#### Scenario: A retryable provider failure repeats until the request-level budget is exhausted
- **WHEN** the provider request continues to fail with retryable failures
- **AND** the request-level retry budget or attempt limit is exhausted
- **THEN** the system SHALL stop automatic request-level retry
- **AND** it SHALL expose a stable reason indicating budget exhaustion or attempt exhaustion

#### Scenario: The request consumes time through connection attempts, sleeps, and partial responses
- **WHEN** the system tracks request-level retry budget
- **THEN** it SHALL account for request-level wall-clock consumption rather than only counting sleep duration
- **AND** it SHALL distinguish at least `attempt_limit_exhausted` and `time_budget_exhausted`

#### Scenario: A provider response includes `Retry-After`
- **WHEN** a retryable provider response includes `Retry-After`
- **THEN** the system MAY use it when choosing delay
- **AND** it SHALL NOT let `Retry-After` override remaining retry budget without limit
- **AND** it SHALL only honor `Retry-After` under explicitly allowed response classes

#### Scenario: `Retry-After` is invalid, expired, zero, or larger than the remaining budget
- **WHEN** a parsed `Retry-After` value is invalid, already expired, zero, or larger than the remaining request-level budget
- **THEN** the system SHALL NOT blindly sleep for that raw value
- **AND** it SHALL either clamp, ignore, or terminate according to a stable observable merge rule

### Requirement: Fallback result sources SHALL remain explicit
The system SHALL expose whether a result came from live streaming, live sync response, degraded provider fallback, or local synthesis.

#### Scenario: A follow-up stream fails and the system completes through a lower-confidence fallback path
- **WHEN** the provider stream cannot complete through live streaming
- **AND** the system finishes through sync fallback or local synthesis
- **THEN** the terminal result SHALL preserve explicit source metadata
- **AND** monitoring and trace surfaces SHALL be able to distinguish `live_stream`, `live_sync`, `degraded_provider`, and `local_synthesis`

#### Scenario: Frontend and monitoring surfaces consume a terminal retry/fallback result
- **WHEN** frontend or monitoring surfaces consume the terminal result of a request that retried or fell back
- **THEN** they SHALL receive stable machine-readable source values rather than inferring fallback source from human-readable text

### Requirement: Stable retry and orchestration fields SHALL use consistent names and enums across surfaces
The system SHALL expose retry/orchestration facts through stable field names and enum values across command inputs, terminal payloads, trace truth, and read models.

#### Scenario: An explicit whole-turn retry is started through the existing orchestration entrypoint
- **WHEN** a normal start, explicit retry, replay, or restart is initiated through a shared orchestration entrypoint
- **THEN** the request SHALL carry a stable `startReason` field
- **AND** `startReason` SHALL use a stable enum set equivalent to `initial_turn`, `explicit_retry_after_failure`, `replay_from_checkpoint`, and `restart_from_checkpoint`

#### Scenario: A request-level or phase-level failure becomes part of canonical terminal evidence
- **WHEN** retry/fallback evidence is projected into terminal payloads, trace persistence, or drill-down read models
- **THEN** the system SHALL use stable fields equivalent to `failureClass`, `budgetExhaustedReason`, and `fallbackSource`
- **AND** the enum values used across those surfaces SHALL remain identical rather than being renamed per surface

#### Scenario: A field is not semantically applicable to a successful non-retried normal start
- **WHEN** a field such as `budgetExhaustedReason` is not semantically applicable
- **THEN** the system MAY omit it or return null according to the host contract
- **AND** it SHALL NOT synthesize placeholder strings that could be mistaken for canonical enum values

### Requirement: Retry and fallback evidence SHALL align with canonical turn lifecycle and persisted trace truth
The system SHALL project retry/fallback terminal facts through the existing canonical turn terminal vocabulary and persisted trace truth-source rather than inventing parallel terminal semantics in the frontend.

#### Scenario: A provider interaction ultimately fails after retries or fallbacks
- **WHEN** request-level retry or phase-level fallback ultimately ends in failure
- **THEN** the turn SHALL still terminate through the canonical lifecycle terminal vocabulary rather than through a new retry-specific terminal event family
- **AND** the terminal payload or read model SHALL preserve stable retry/fallback facts such as failure class, budget exhausted reason, and fallback source

#### Scenario: A trace is reloaded after a retried or degraded turn has already persisted
- **WHEN** the session is reloaded after backend trace persistence is available
- **THEN** retry/fallback evidence SHALL be restored from persisted backend truth
- **AND** the frontend SHALL NOT recompute canonical retry outcomes from local-only timers or cached inference

### Requirement: Deterministic retry tests SHALL not depend on real wall-clock sleeps
The system SHALL provide a retry test strategy that validates delay selection and decision rules without requiring real-time sleep as the primary correctness mechanism.

#### Scenario: Engineers validate retry delay behavior in automated tests
- **WHEN** automated tests verify retry/backoff behavior
- **THEN** they SHALL be able to validate delay planning and next-action decisions through deterministic strategy tests
- **AND** real wall-clock sleeps SHALL NOT be the primary validation path

#### Scenario: Automated tests validate cancellation and deadline-aware retry behavior
- **WHEN** automated tests cover retry execution behavior
- **THEN** they SHALL be able to validate cancellation and deadline propagation without relying on uncontrolled wall-clock sleeps

### Requirement: Frontend whole-turn retry SHALL remain distinct from provider request retry
The system SHALL keep frontend or control-plane whole-turn retry semantics distinct from provider request retry semantics.

#### Scenario: A turn fails after provider retry is exhausted
- **WHEN** provider request retry is exhausted for a turn
- **THEN** the system SHALL expose that terminal condition explicitly
- **AND** any later whole-turn retry SHALL be treated as a separate orchestration action rather than another hidden provider request retry attempt

### Requirement: Explicit turn-level retry SHALL reuse existing orchestration and audit boundaries
The system SHALL model any future whole-turn retry as an explicit orchestration action that reuses existing start/replay/restart and audit boundaries rather than creating a silent second retry subsystem.

#### Scenario: A user explicitly retries a failed turn
- **WHEN** a user or control-plane explicitly initiates whole-turn retry after request-level retry has already terminated
- **THEN** the new action SHALL be modeled as a new orchestration action with its own identity and budget
- **AND** it SHALL NOT be treated as an implicit continuation of the prior request-level retry sequence

#### Scenario: An explicit turn-level retry is surfaced in audit or explainability views
- **WHEN** a whole-turn retry becomes part of control-plane or run-control semantics
- **THEN** its explainability and audit projection SHALL align with the existing run-control summary family rather than inventing a parallel summary contract

#### Scenario: A host entrypoint accepts both normal starts and explicit retries
- **WHEN** the same host/control-plane start entrypoint is used for both ordinary starts and explicit whole-turn retry
- **THEN** the request SHALL carry a stable `start_reason` equivalent to `initial_turn`, `explicit_retry_after_failure`, `replay_from_checkpoint`, or `restart_from_checkpoint`
- **AND** the system SHALL NOT infer explicit retry merely from timing, prior failure presence, or frontend-local state

#### Scenario: Engineers choose the minimal explicit retry entrypoint design
- **WHEN** the system adds explicit whole-turn retry after retiring silent frontend auto-retry
- **THEN** it SHOULD reuse the existing start/run orchestration entrypoint rather than introducing a separate retry-only command
- **AND** it SHOULD model retry identity through `start_reason` instead of a parallel retry-specific command family unless a concrete semantic gap is discovered

#### Scenario: A frontend renders an explicit retry action result
- **WHEN** a frontend receives the result of an explicit whole-turn retry action
- **THEN** it SHALL be able to distinguish that action from a normal initial submission using machine-readable fields
- **AND** it SHALL NOT need to reconstruct retry intent from ad hoc text or by comparing adjacent turns

### Requirement: Silent frontend whole-turn auto-retry SHALL be retired
The system SHALL retire silent frontend whole-turn automatic retry rather than keeping it as a second automatic recovery layer beside provider request retry.

#### Scenario: The frontend previously scheduled an automatic whole-turn retry timer after failure
- **WHEN** the implementation migrates to the new retry boundary
- **THEN** the frontend SHALL stop scheduling silent automatic whole-turn retry timers
- **AND** it SHALL instead surface the terminal provider outcome and any eligible explicit retry action separately

#### Scenario: A user or control-plane later chooses to retry the whole turn
- **WHEN** a whole-turn retry is performed after provider retry exhaustion
- **THEN** it SHALL be modeled as an explicit orchestration action
- **AND** it SHALL NOT be treated as an implicit continuation of the prior request-level retry budget
