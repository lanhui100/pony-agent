// Ask (Interaction control request) and Plan projections for the PA-076 host
// control surface (task 4.4 frontend). Field names mirror the backend's camelCase
// serde projections (crates/pony-agent-core/src/agent/tool_runtime.rs and
// plan_state.rs) so `safeInvoke` payloads can be typed directly.

export type PendingAskState = "pending" | "consumed" | "cancelled" | "expired" | string;

export type PendingAsk = {
  requestId: string;
  requestKind: "interaction" | "approval" | string;
  sessionId?: string | null;
  runId?: string | null;
  turnId: string;
  callId: string;
  descriptorSnapshotId?: string | null;
  descriptorId?: string | null;
  finalArgsDigest?: string | null;
  policyDigest?: string | null;
  nonce?: string | null;
  version: number;
  expiresAtMs: number;
  state: PendingAskState;
  prompt?: string | null;
  options?: unknown[] | null;
};

/**
 * Ask lifecycle disposition (ADR 0074). Mirrors the backend `AskLifecycleState` snake_case
 * serde projection (crates/pony-agent-core/src/agent/ask_persistence.rs): only `pending`
 * requests may be surfaced to the user, and the four terminal states are not interchangeable
 * (`cancelled` = explicit ignore, `superseded` = a newer user message made it moot,
 * `orphaned` = owning session was archived and GC reclaimed the record).
 */
export type AskLifecycleState =
  | "pending"
  | "answered"
  | "cancelled"
  | "superseded"
  | "orphaned";

/** Control-request kind: an Ask (`interaction`) or an Approval request. */
export type AskKind = "interaction" | "approval";

/**
 * Durable Ask record (ADR 0074). Mirrors the backend `PersistedAsk` camelCase serde projection
 * (crates/pony-agent-core/src/agent/ask_persistence.rs). Unlike `PendingAsk` this record carries
 * no `expiresAtMs`: wall-clock TTL is no longer product semantics. `version` + `nonce` are
 * persisted so an answer is a cross-process compare-and-swap, and `sessionId` is mandatory
 * because it is the recovery key used to rebuild the Ask panel after a restart.
 */
export type PersistedAsk = {
  requestId: string;
  version: number;
  nonce: string;
  sessionId: string;
  runId?: string | null;
  turnId: string;
  callId: string;
  kind: AskKind;
  prompt: string;
  options: string[];
  state: AskLifecycleState;
  createdAtMs: number;
  answeredAtMs?: number | null;
};

/**
 * Stable Ask rejection codes (contract §4). The host must map these to the Chinese user-facing
 * copy and keep the raw code in runtime logs only — never render it in a panel.
 */
export type AskPersistenceErrorCode =
  | "ask_state_conflict"
  | "ask_version_mismatch"
  | "ask_nonce_mismatch"
  | "ask_not_found";

export type TodoItemStatus = "pending" | "in_progress" | "completed";

export type TodoItem = {
  content: string;
  status: TodoItemStatus;
};

export type PlanStepStatus = "pending" | "inProgress" | "completed" | "failed" | string;
export type PlanLifecycle = "draft" | "executing" | "completed" | "aborted" | string;

export type PlanStep = {
  stepId: string;
  name: string;
  summary: string;
  status: PlanStepStatus;
};

export type Plan = {
  planId: string;
  sessionId: string;
  revision: number;
  kind: string;
  summary: string;
  lifecycle: PlanLifecycle;
  steps: PlanStep[];
};

export type PlanStepSpec = {
  name: string;
  summary?: string;
};

export type PlanPayload = {
  kind: string;
  summary?: string;
  steps?: PlanStepSpec[];
};

/**
 * Graph Ask wait binding (design.md Decision 5, PA-076 phase-4 P0). Mirrors the backend
 * `GraphAskWaitBinding` camelCase serde projection (crates/pony-agent-core/src/agent/graph.rs).
 * The host resumes the bound run with `graph_resume_ask` using `expectedVersion` — the version
 * the binding was created with, not the post-consumption request version.
 */
export type GraphAskWait = {
  requestId: string;
  expectedVersion: number;
  runId: string;
  turnId: string;
  sessionId?: string | null;
  callId: string;
  toolName: string;
  assistantTranscript?: unknown;
  createdAtMs: number;
};

/**
 * Outcome of `graph_resume_ask` — the run is back to `Ready` and the caller receives exactly one
 * terminal tool result to inject for the original `callId` before the run continues.
 */
export type GraphAskResumeOutcome = {
  requestId: string;
  runId: string;
  turnId: string;
  callId: string;
  toolName: string;
  answer: unknown;
  terminalResult: unknown;
};
