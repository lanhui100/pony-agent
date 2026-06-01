import type { ProviderReasoningEffort } from "@/types/provider";

export type HealthPayload = {
  appName: string;
  appVersion: string;
  runtime: string;
  graphEngine: string;
  graphContractVersion: string;
};

export type RuntimePhase =
  | "idle"
  | "connecting"
  | "ready"
  | "completed"
  | "cancelled"
  | "calling_model"
  | "calling_tool"
  | "failed";

export type AttachmentReference = {
  id: string;
  assetId?: string | null;
  name?: string | null;
  mimeType: string;
  relativePath?: string | null;
  sizeBytes?: number | null;
  createdAtMs?: number | null;
};

export type AttachmentAssetStatus =
  | "active"
  | "missing_payload"
  | "expired"
  | "reclaimable";

export type AttachmentAsset = {
  id: string;
  sessionId: string;
  name?: string | null;
  mimeType: string;
  relativePath: string;
  sizeBytes: number;
  createdAtMs: number;
  status?: AttachmentAssetStatus;
  referenceCount?: number;
  lastReferencedAtMs?: number | null;
  expiresAtMs?: number | null;
};

export type AttachmentAssetFilter = {
  sessionId?: string | null;
  mimeType?: string | null;
  nameContains?: string | null;
  createdAfterMs?: number | null;
  createdBeforeMs?: number | null;
  statuses?: AttachmentAssetStatus[];
  limit?: number | null;
};

export type AttachmentMeta = AttachmentReference;

export type ToolActivity = {
  id: string;
  name: string;
  status: "planned" | "running" | "done" | "error";
  summary: string;
  argumentsText?: string | null;
  resultText?: string | null;
  durationSeconds?: number | null;
};

export type AvailableTool = {
  name: string;
  description: string;
  inputSchema: {
    type?: string;
    properties?: Record<string, { type?: string; description?: string }>;
    required?: string[];
    additionalProperties?: boolean;
  };
};

export type TraceStep = {
  id: string;
  label: string;
  state: "completed" | "active" | "pending" | "error" | "cancelled";
};

export type BuildContextObservation = {
  requestFormat: string;
  messageCount: number;
  imageCount: number;
  toolCount: number;
  temperature: number;
  maxOutputTokens: number;
  stablePrefixText: string;
  semiStableContextText: string;
  volatileInputText: string;
  requestMessagesText: string;
  toolDefinitionsText: string;
};

export type TurnInputImage = {
  dataUrl: string;
  mimeType: string;
  name?: string | null;
};

export type ChatMessage = {
  id: string;
  turnId: string;
  role: "user" | "assistant" | "tool";
  content: string;
  attachments?: AttachmentMeta[];
  reasoningContent?: string | null;
  status?: "pending" | "done" | "error";
  modelName?: string | null;
  tokenCount?: number | null;
  toolName?: string | null;
  detail?: string | null;
  durationSeconds?: number | null;
};

export type TurnTraceRecord = {
  turnId: string;
  title: string;
  phase: RuntimePhase;
  traceSteps: TraceStep[];
  toolActivities: ToolActivity[];
  providerRequestedName?: string | null;
  providerName?: string | null;
  providerProtocol?: string | null;
  providerModel?: string | null;
  providerSource?: string | null;
  providerMode?: string | null;
  buildContextObservation?: BuildContextObservation | null;
  sessionSummary?: string | null;
  fallbackReason?: string | null;
  error?: string | null;
  inputTokens?: number | null;
  cacheHitInputTokens?: number | null;
  reasoningTokens?: number | null;
  outputTokens?: number | null;
  totalTokens?: number | null;
  firstTokenLatencyMs?: number | null;
  turnDurationMs?: number | null;
  updatedAt: number;
};

export type TurnInput = {
  message: string;
  displayMessage?: string | null;
  providerId?: string | null;
  modelId?: string | null;
  reasoningEffort?: ProviderReasoningEffort | null;
  sessionId?: string | null;
  history?: TurnHistoryMessage[];
  images?: TurnInputImage[];
};

export type TurnHistoryMessage = {
  role: "user" | "assistant";
  content: string;
  attachments?: AttachmentMeta[];
};

export type SessionOverview = {
  conversationId: string;
  title?: string | null;
  summary: string;
  turnCount: number;
  lastReferencedFile?: string | null;
  updatedAtMs: number;
};

export type SessionSnapshot = {
  conversationId: string;
  title?: string | null;
  summary: string;
  history: TurnHistoryMessage[];
  attachmentAssets?: AttachmentAsset[];
  turnTraceHistory?: TurnTraceRecord[];
  turnCount: number;
  lastReferencedFile?: string | null;
  updatedAtMs: number;
};

export type TurnContext = {
  userMessage: string;
  images: TurnInputImage[];
  referencesImage: boolean;
};

export type SessionContext = {
  conversationId: string;
  title: string;
  summary: string;
  recentHistory: TurnHistoryMessage[];
  recentAttachmentAssets: AttachmentAsset[];
  turnCount: number;
  lastReferencedFile?: string | null;
};

export type RunState = {
  runId?: string | null;
  goal?: string | null;
  phase?: string | null;
  activeTurnId?: string | null;
  lastCompletedTurnId?: string | null;
  resumeCount?: number | null;
  lastDecisionSummary?: string | null;
  executionCheckpointStatus?: string | null;
  executionCheckpointPhase?: string | null;
};

export type LongTermMemoryEntry = {
  kind: string;
  content: string;
  source: string;
  updatedAtMs: number;
};

export type LongTermMemory = {
  status: string;
  summary?: string | null;
  entries: LongTermMemoryEntry[];
};

export type TranscriptContext = {
  providerNativeMessages: unknown[];
};

export type RetrievedContextState = {
  turnContext: TurnContext;
  sessionContext: SessionContext;
  runState: RunState;
  longTermMemory: LongTermMemory;
  transcript: TranscriptContext;
};

export type GraphRunPhase =
  | "ready"
  | "running"
  | "waiting_user"
  | "paused"
  | "completed"
  | "failed"
  | "cancelled";

export type GraphDecision = {
  kind: string;
  reason: string;
  summary: string;
  targetPhase: GraphRunPhase;
};

export type GraphStep = {
  id: string;
  kind?: string | null;
  turnId?: string | null;
  sessionId?: string | null;
  phase: GraphRunPhase;
  title: string;
  updatedAtMs: number;
};

export type GraphTurnHandoff = {
  contractVersion: string;
  turnId?: string | null;
  sessionId?: string | null;
  turnPhase: string;
  checkpointStatus?: string | null;
  checkpointPhase?: string | null;
  userMessage: string;
  assistantMessage: string;
  sessionSummary: string;
  conversationId: string;
  sessionTurnCount: number;
  runId?: string | null;
  runPhase?: string | null;
  activeTaskFocus?: string | null;
  acceptanceFocus?: string | null;
  lastReferencedFile?: string | null;
  recentAttachmentAssetCount: number;
  longTermMemoryStatus: string;
  longTermMemoryEntryCount: number;
  traceStepCount: number;
  toolActivityCount: number;
  providerName: string;
  providerModel: string;
};

export type GraphRun = {
  id: string;
  goal: string;
  sessionId?: string | null;
  phase: GraphRunPhase;
  steps: GraphStep[];
  activeTurnId?: string | null;
  lastCompletedTurnId?: string | null;
  stopReason?: string | null;
  lastHandoff?: GraphTurnHandoff | null;
  resumeCount: number;
  lastDecision?: GraphDecision | null;
  createdAtMs: number;
  updatedAtMs: number;
};

export type GraphRunCheckpoint = {
  contractVersion: string;
  runId: string;
  goal: string;
  sessionId?: string | null;
  phase: GraphRunPhase;
  activeTurnId?: string | null;
  lastCompletedTurnId?: string | null;
  stopReason?: string | null;
  steps: GraphStep[];
  lastDecision?: GraphDecision | null;
  lastHandoff?: GraphTurnHandoff | null;
  resumeCount: number;
  resumable: boolean;
  createdAtMs: number;
  updatedAtMs: number;
};

export type GraphRunEvent = {
  runId: string;
  kind: string;
  phase: GraphRunPhase;
  summary: string;
  stepCount: number;
  updatedAtMs: number;
};

export type GraphRunTurnResponse = {
  run: GraphRun;
  handoff: GraphTurnHandoff;
  decision: GraphDecision;
  event: GraphRunEvent;
};

export type GraphRunControlResponse = {
  run: GraphRun;
  event: GraphRunEvent;
};

export type GraphRunStreamStartResponse = {
  run: GraphRun;
  event: GraphRunEvent;
  turnId: string;
};

export type TurnResult = {
  phase: RuntimePhase;
  providerRequestedName: string;
  providerName: string;
  providerProtocol: string;
  providerModel: string;
  providerSource: string;
  providerMode: string;
  fallbackReason?: string | null;
  buildContextObservation?: BuildContextObservation | null;
  reasoningContent?: string | null;
  inputTokens?: number | null;
  cacheHitInputTokens?: number | null;
  reasoningTokens?: number | null;
  outputTokens?: number | null;
  totalTokens?: number | null;
  firstTokenLatencyMs?: number | null;
  turnDurationMs?: number | null;
  userMessage: string;
  assistantMessage: string;
  traceSteps: TraceStep[];
  toolActivities: ToolActivity[];
  sessionSummary: string;
};

export type TurnStreamEvent = {
  turnId: string;
  kind: "started" | "delta" | "trace" | "tool" | "completed" | "failed" | "cancelled";
  phase?: RuntimePhase | string | null;
  text?: string | null;
  reasoningContent?: string | null;
  error?: string | null;
  providerRequestedName?: string | null;
  providerName?: string | null;
  providerProtocol?: string | null;
  providerModel?: string | null;
  providerSource?: string | null;
  providerMode?: string | null;
  fallbackReason?: string | null;
  buildContextObservation?: BuildContextObservation | null;
  inputTokens?: number | null;
  cacheHitInputTokens?: number | null;
  reasoningTokens?: number | null;
  outputTokens?: number | null;
  totalTokens?: number | null;
  firstTokenLatencyMs?: number | null;
  turnDurationMs?: number | null;
  traceSteps?: TraceStep[] | null;
  toolActivities?: ToolActivity[] | null;
  sessionSummary?: string | null;
};

export type ExecutionCheckpoint = {
  turnId: string;
  sessionId?: string | null;
  status: string;
  phase: string;
  providerRequestedName?: string | null;
  providerName?: string | null;
  providerProtocol?: string | null;
  providerModel?: string | null;
  providerSource?: string | null;
  providerMode?: string | null;
  fallbackReason?: string | null;
  completedHops: number;
  maxHops: number;
  activeToolName?: string | null;
  traceSteps: TraceStep[];
  toolActivities: ToolActivity[];
  error?: string | null;
  startedAtMs: number;
  updatedAtMs: number;
  stopRequestedAtMs?: number | null;
};

export type SessionRuntimeView = {
  session: SessionSnapshot;
  retrieved: RetrievedContextState;
  checkpoint?: ExecutionCheckpoint | null;
};

export type HostInspectionSnapshot = {
  surface?: string | null;
  turn?: ExecutionCheckpoint | null;
  session?: SessionSnapshot | null;
  retrieved?: RetrievedContextState | null;
  sessions?: SessionOverview[] | null;
  run?: GraphRun | null;
  runs?: GraphRun[] | null;
};

export function normalizeGraphRunPhase(phase?: string | null): GraphRunPhase | null {
  const normalized = phase?.trim().toLowerCase() ?? "";
  if (
    normalized === "ready" ||
    normalized === "running" ||
    normalized === "waiting_user" ||
    normalized === "paused" ||
    normalized === "completed" ||
    normalized === "failed" ||
    normalized === "cancelled"
  ) {
    return normalized;
  }

  return null;
}

export function deriveGraphRunFromRunState(
  runState: RunState | null | undefined,
  currentSessionId: string,
  updatedAtMs: number,
  options?: { activeTaskFocus?: string | null }
): GraphRun | null {
  const runId = runState?.runId?.trim() ?? "";
  const phase = normalizeGraphRunPhase(runState?.phase);
  if (!runId || !phase) {
    return null;
  }

  const activeTaskFocus = options?.activeTaskFocus?.trim() || null;
  const trackedTurnId = runState?.activeTurnId?.trim() || runState?.lastCompletedTurnId?.trim() || "";

  return {
    id: runId,
    goal: runState?.goal?.trim() || runId,
    sessionId: currentSessionId,
    phase,
    steps: trackedTurnId
      ? [
          {
            id: `${runId}:turn`,
            turnId: trackedTurnId,
            sessionId: currentSessionId,
            phase,
            title: runState?.lastDecisionSummary?.trim() || runState?.goal?.trim() || runId,
            updatedAtMs
          }
        ]
      : [],
    activeTurnId: runState?.activeTurnId?.trim() || null,
    lastCompletedTurnId: runState?.lastCompletedTurnId?.trim() || null,
    stopReason: null,
    lastHandoff: trackedTurnId
      ? {
          contractVersion: "retrieved-run-state@v1",
          turnId: trackedTurnId,
          sessionId: currentSessionId,
          turnPhase: phase,
          checkpointStatus: runState?.executionCheckpointStatus?.trim() || null,
          checkpointPhase: runState?.executionCheckpointPhase?.trim() || null,
          userMessage: "",
          assistantMessage: "",
          sessionSummary: "",
          conversationId: currentSessionId,
          sessionTurnCount: 0,
          runId,
          runPhase: phase,
          activeTaskFocus,
          lastReferencedFile: null,
          recentAttachmentAssetCount: 0,
          longTermMemoryStatus: activeTaskFocus ? "available" : "empty",
          longTermMemoryEntryCount: activeTaskFocus ? 1 : 0,
          traceStepCount: 0,
          toolActivityCount: 0,
          providerName: "",
          providerModel: ""
        }
      : null,
    resumeCount: runState?.resumeCount ?? 0,
    lastDecision: runState?.lastDecisionSummary?.trim()
      ? {
          kind: "continue",
          reason: "retrieved_run_state",
          summary: runState.lastDecisionSummary.trim(),
          targetPhase: phase
        }
      : null,
    createdAtMs: updatedAtMs,
    updatedAtMs
  };
}

export function extractActiveTaskFocus(entries?: LongTermMemoryEntry[] | null) {
  const content =
    entries?.find((entry) => entry.kind === "project_focus.active_task")?.content?.trim() ?? "";
  if (!content) {
    return "";
  }

  const taskId = content.match(/\b[A-Z]{2,6}-\d{1,6}\b/)?.[0] ?? "";
  return taskId || content;
}
