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
