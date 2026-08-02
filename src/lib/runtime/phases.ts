// phase / graph-run domain：生命周期 phase 与 runtime phase 的归一化映射，
// 以及 graph run 提交策略的解析（run-state / plan / checkpoint 三路裁决）。
import { normalizeGraphRunPhase } from "../../types/runtime";
import type {
  ChatMessage,
  ExecutionCheckpoint,
  GraphRunSubmissionPlan,
  RunState,
  RuntimePhase,
  TurnStreamEvent,
  TurnTraceRecord
} from "../../types/runtime";

export function isGraphTerminalPhase(phase?: string | null) {
  return ["completed", "failed", "cancelled"].includes((phase ?? "").trim().toLowerCase());
}

export function normalizeRuntimePhaseValue(phase?: string | null): RuntimePhase | null {
  const normalized = phase?.trim().toLowerCase().replace(/-/g, "_");

  switch (normalized) {
    case "idle":
    case "connecting":
    case "ready":
    case "completed":
    case "cancelled":
    case "calling_model":
    case "calling_tool":
    case "failed":
      return normalized;
    default:
      return null;
  }
}

export function mapLifecyclePhaseToRuntimePhase(phase?: string | null): RuntimePhase | null {
  const normalized = phase?.trim().toLowerCase().replace(/-/g, "_");

  switch (normalized) {
    case "created":
    case "preparing":
    case "building_context":
    case "checkpointing":
    case "queued":
      return "connecting";
    case "calling_model":
    case "streaming_response":
    case "tool_result_integrating":
      return "calling_model";
    case "executing_tool":
      return "calling_tool";
    case "completed":
      return "completed";
    case "failed":
      return "failed";
    case "cancelled":
      return "cancelled";
    default:
      return normalizeRuntimePhaseValue(normalized);
  }
}

export function resolveRuntimePhaseFromEvent(
  payload: Pick<TurnStreamEvent, "eventType" | "phase">,
  fallback: RuntimePhase
): RuntimePhase {
  const phaseFromPayload = mapLifecyclePhaseToRuntimePhase(payload.phase);
  if (phaseFromPayload) {
    return phaseFromPayload;
  }

  switch (payload.eventType) {
    case "turn.created":
    case "turn.context_built":
    case "turn.checkpoint_persisted":
      return "connecting";
    case "turn.model_call_started":
    case "turn.first_token":
    case "turn.output_delta":
      return "calling_model";
    case "turn.tool_call_started":
      return "calling_tool";
    case "turn.tool_call_completed":
      return "calling_model";
    case "turn.completed":
      return "completed";
    case "turn.failed":
      return "failed";
    case "turn.cancelled":
      return "cancelled";
    default:
      return fallback;
  }
}

export function resolveFallbackTimelineRuntimePhase(
  eventType?: TurnStreamEvent["eventType"],
  phase?: RuntimePhase | string | null
): RuntimePhase | "connecting" {
  const phaseFromPayload = mapLifecyclePhaseToRuntimePhase(phase);
  if (phaseFromPayload) {
    return phaseFromPayload;
  }

  switch (eventType) {
    case "turn.created":
    case "turn.context_built":
    case "turn.checkpoint_persisted":
      return "connecting";
    case "turn.model_call_started":
    case "turn.first_token":
    case "turn.output_delta":
      return "calling_model";
    case "turn.tool_call_started":
      return "calling_tool";
    case "turn.tool_call_completed":
      return "calling_model";
    case "turn.completed":
      return "completed";
    case "turn.failed":
      return "failed";
    case "turn.cancelled":
      return "cancelled";
    default:
      return normalizeRuntimePhaseValue(phase) ?? "connecting";
  }
}

export function restorePhaseFromTurnHistory(
  messages: ChatMessage[],
  turnTraceHistory: TurnTraceRecord[]
): RuntimePhase {
  const latestTurnPhase = normalizeRuntimePhaseValue(turnTraceHistory[turnTraceHistory.length - 1]?.phase);
  if (messages.length === 0) {
    return latestTurnPhase ?? "idle";
  }

  if (!latestTurnPhase) {
    return "ready";
  }

  const latestTrace = turnTraceHistory[turnTraceHistory.length - 1];
  const latestTraceHasCanonicalTerminalEnvelope = Boolean(
    latestTrace?.eventId?.trim()
      && latestTrace?.eventVersion?.trim()
      && latestTrace?.sequence != null
      && latestTrace?.emittedAtMs != null
      && (latestTrace?.eventType === "turn.completed"
        || latestTrace?.eventType === "turn.failed"
        || latestTrace?.eventType === "turn.cancelled")
  );

  switch (latestTurnPhase) {
    case "completed":
      if (!latestTraceHasCanonicalTerminalEnvelope) {
        return "ready";
      }
      return "ready";
    case "failed":
    case "cancelled":
      return latestTraceHasCanonicalTerminalEnvelope ? latestTurnPhase : "ready";
    case "idle":
    case "ready":
      return latestTurnPhase;
    default:
      return latestTurnPhase;
  }
}

export function createBrowserPreviewTerminalEnvelope(
  turnId: string,
  eventType: "turn.completed" | "turn.cancelled",
  sequence: number,
  emittedAtMs: number
) {
  return {
    eventId: `${turnId}:${eventType}:${sequence}`,
    eventType,
    eventVersion: "turn-event-v1",
    sequence,
    emittedAtMs
  };
}

export function isTerminalPhase(phase: string): phase is "completed" | "failed" | "cancelled" {
  return phase === "completed" || phase === "failed" || phase === "cancelled";
}

export function resolveRestoredPersistedPhase(
  persistedPhase: RuntimePhase | null | undefined,
  canonicalTerminalPhase: "completed" | "failed" | "cancelled" | null | undefined,
  messages: ChatMessage[],
  turnTraceHistory: TurnTraceRecord[]
): RuntimePhase {
  const restoredPhase = restorePhaseFromTurnHistory(messages, turnTraceHistory);
  if (restoredPhase !== "ready") {
    return restoredPhase;
  }

  if (canonicalTerminalPhase) {
    return canonicalTerminalPhase;
  }

  if (persistedPhase === "cancelled" || persistedPhase === "failed") {
    return persistedPhase;
  }

  return restoredPhase;
}

export type GraphRunSubmission =
  | { command: "start_graph_run_stream"; runId: null }
  | { command: "resume_graph_run_stream"; runId: string }
  | { command: "continue_graph_run_stream"; runId: string };

export function resolveGraphRunSubmissionFromRunState(runState?: RunState | null) {
  if (!runState) {
    return null;
  }

  const runId = runState.runId?.trim() || null;
  const phase = normalizeGraphRunPhase(runState.phase);
  if (!runId || !phase) {
    return null;
  }

  if (isGraphTerminalPhase(phase)) {
    return { command: "start_graph_run_stream" as const, runId: null };
  }

  if (phase === "paused") {
    return { command: "resume_graph_run_stream" as const, runId };
  }

  return { command: "continue_graph_run_stream" as const, runId };
}

export function resolveGraphRunSubmissionFromPlan(
  plan?: GraphRunSubmissionPlan | null
): GraphRunSubmission | null {
  const command = plan?.command?.trim().toLowerCase() || null;
  const runId = plan?.runId?.trim() || null;
  if (command === "start_graph_run_stream") {
    return { command: "start_graph_run_stream" as const, runId: null };
  }
  if (command === "resume_graph_run_stream" && runId) {
    return { command: "resume_graph_run_stream" as const, runId };
  }
  if (command === "continue_graph_run_stream" && runId) {
    return { command: "continue_graph_run_stream" as const, runId };
  }
  return null;
}

export function resolveGraphRunSubmissionFromCheckpoint(
  checkpoint?: ExecutionCheckpoint | null,
  activeRunId?: string | null
): GraphRunSubmission | null {
  if (!checkpoint || checkpoint.checkpointKind !== "recovery") {
    return null;
  }

  const projectedCommand = checkpoint.submissionCommand?.trim().toLowerCase() || null;
  const checkpointRunId = checkpoint.runId?.trim() || null;
  const runId = checkpointRunId || activeRunId?.trim() || null;
  if (projectedCommand === "start_graph_run_stream") {
    return { command: "start_graph_run_stream" as const, runId: null };
  }

  if (projectedCommand === "resume_graph_run_stream" && runId) {
    return { command: "resume_graph_run_stream" as const, runId };
  }

  if (projectedCommand === "continue_graph_run_stream" && runId) {
    return { command: "continue_graph_run_stream" as const, runId };
  }

  const recoveryMode = checkpoint.recoveryMode?.trim().toLowerCase() || null;
  if (recoveryMode === "replay_required") {
    return { command: "start_graph_run_stream" as const, runId: null };
  }

  if (!runId) {
    return null;
  }

  const phase = checkpoint.phase.trim().toLowerCase();
  const status = checkpoint.status.trim().toLowerCase();
  if (status === "failed" || status === "cancelled" || phase === "failed" || phase === "cancelled") {
    return null;
  }

  if (phase === "paused" || (checkpoint.resumable && status === "ready")) {
    return { command: "resume_graph_run_stream" as const, runId };
  }

  if (phase === "ready" || phase === "waiting_user" || phase === "completed") {
    return { command: "continue_graph_run_stream" as const, runId };
  }

  return null;
}

export function reconcileSubmissionWithRecoveryCheckpoint(
  submission: GraphRunSubmission | null,
  checkpoint?: ExecutionCheckpoint | null
): GraphRunSubmission | null {
  if (!submission || !checkpoint || checkpoint.checkpointKind !== "recovery") {
    return submission;
  }

  const recoveryMode = checkpoint.recoveryMode?.trim().toLowerCase() || null;
  if (recoveryMode === "replay_required" && submission.command !== "start_graph_run_stream") {
    return { command: "start_graph_run_stream" as const, runId: null };
  }

  return submission;
}

export function normalizeCheckpointPhase(checkpoint: ExecutionCheckpoint): RuntimePhase {
  const projectedRuntimePhase = normalizeRuntimePhaseValue(checkpoint.projectedRuntimePhase);
  if (projectedRuntimePhase) {
    return projectedRuntimePhase;
  }

  if (checkpoint.checkpointKind === "recovery") {
    const status = checkpoint.status.trim().toLowerCase();
    if (status === "failed") {
      return "failed";
    }

    if (status === "cancelled") {
      return "cancelled";
    }

    return "ready";
  }

  const runtimePhase = normalizeRuntimePhaseValue(checkpoint.phase);
  if (runtimePhase) {
    return runtimePhase;
  }

  if (
    checkpoint.activeToolName?.trim() ||
    checkpoint.toolActivities.some((tool) => tool.status === "running")
  ) {
    return "calling_tool";
  }

  const lifecyclePhase = mapLifecyclePhaseToRuntimePhase(checkpoint.phase);
  if (lifecyclePhase) {
    return lifecyclePhase;
  }

  const status = checkpoint.status.trim().toLowerCase();
  if (status === "cancelled") {
    return "cancelled";
  }

  if (status === "failed") {
    return "failed";
  }

  return "calling_model";
}
