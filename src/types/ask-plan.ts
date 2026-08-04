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
