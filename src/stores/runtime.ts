import { defineStore } from "pinia";
import { isTauriAvailable, safeInvoke, safeListen } from "@/lib/tauri";
import { useProviderStore } from "@/stores/providers";
import { deriveGraphRunFromRunState, extractActiveTaskFocus, normalizeGraphRunPhase } from "../types/runtime";
import type {
  AttachmentAsset,
  AttachmentAssetFilter,
  AvailableTool,
  BuildContextObservation,
  CapabilitySourceView,
  CapabilityView,
  ChatMessage,
  ConversationCheckpointEntry,
  ExecutionCheckpoint,
  GraphRun,
  GraphRunControlResponse,
  GraphRunControlBoundaryEvidence,
  GraphRunSubmissionPlan,
  HookPatchOperation,
  HistoryStateAuditSummary,
  RunControlAuditSummary,
  HistoryStateHookEvidence,
  HookStructuredResult,
  HookTraceRecord,
  GraphRunStreamStartResponse,
  HealthPayload,
  HistoryBranch,
  HistoryBranchSwitchResult,
  HistoryCheckoutMode,
  HistoryCheckoutResult,
  HistoryCursorMode,
  HistoryCursorState,
  HistoryForkResult,
  HistoryNode,
  HistoryRestoreResult,
  RunState,
  ProviderCallCacheRecord,
  RetrievedContextState,
  RuntimePhase,
  SessionOverview,
  SessionRuntimeView,
  SessionSnapshot,
  ToolActivity,
  TraceStep,
  TraceTimelineEntry,
  TurnHistoryMessage,
  TurnInputImage,
  TurnInput,
  TurnStreamEvent,
  TurnTraceRecord
} from "../types/runtime";

type HistoryCheckoutWireResult = {
  sessionId: string;
  nodeId: string;
  requestedMode: HistoryCheckoutMode;
  appliedMode: HistoryCheckoutMode;
  transcriptRestoreApplied: boolean;
  workspaceRollbackCapable: boolean;
  workspaceRollbackApplied: boolean;
  degraded: boolean;
  degradationReason?: string | null;
  historyStateEvidence?: HistoryStateHookEvidence[] | null;
  historyStateAuditSummary?: HistoryStateAuditSummary | null;
  cursor: HistoryCursorState;
};

type HistoryRestoreWireResult = {
  sessionId: string;
  branchId?: string | null;
  restoredNodeId?: string | null;
  transcriptRestoreApplied: boolean;
  workspaceRollbackCapable: boolean;
  workspaceRollbackApplied: boolean;
  degraded: boolean;
  degradationReason?: string | null;
  historyStateEvidence?: HistoryStateHookEvidence[] | null;
  historyStateAuditSummary?: HistoryStateAuditSummary | null;
  cursor: HistoryCursorState;
};

type HistoryForkWireResult = {
  sessionId: string;
  nodeId: string;
  branch: HistoryBranch;
  historyStateEvidence?: HistoryStateHookEvidence[] | null;
  historyStateAuditSummary?: HistoryStateAuditSummary | null;
  cursor: HistoryCursorState;
};

type HistoryBranchSwitchWireResult = {
  sessionId: string;
  branchId: string;
  nodeId?: string | null;
  historyStateEvidence?: HistoryStateHookEvidence[] | null;
  historyStateAuditSummary?: HistoryStateAuditSummary | null;
  cursor: HistoryCursorState;
};

type RuntimeState = {
  sessionId: string;
  sessionList: SessionOverview[];
  sessionOperation: "initializing" | "switching" | "deleting" | null;
  sessionError: string | null;
  phase: RuntimePhase;
  health: HealthPayload | null;
  error: string | null;
  draftMessage: string;
  sessionSummary: string;
  retrievedContext: RetrievedContextState | null;
  providerRequestedName: string;
  providerName: string;
  providerProtocol: string;
  providerModel: string;
  providerSource: string;
  providerMode: string;
  fallbackReason: string | null;
  inputTokens: number | null;
  outputTokens: number | null;
  totalTokens: number | null;
  firstTokenLatencyMs: number | null;
  isSubmitting: boolean;
  messages: ChatMessage[];
  attachmentAssets: AttachmentAsset[];
  availableTools: AvailableTool[];
  capabilitySources: CapabilitySourceView[];
  capabilities: CapabilityView[];
  toolActivities: ToolActivity[];
  traceSteps: TraceStep[];
  traceTimeline: TraceTimelineEntry[];
  turnTraceHistory: TurnTraceRecord[];
  eventCursorByTurnId: Record<string, { eventId: string | null; sequence: number | null; emittedAtMs: number | null }>;
  activeTurnId: string | null;
  activeRunId: string | null;
  latestExecutionCheckpoint: ExecutionCheckpoint | null;
  latestGraphRunSubmissionPlan: GraphRunSubmissionPlan | null;
  latestGraphRunControlBoundaryEvidence: GraphRunControlBoundaryEvidence[];
  latestRunControlAuditSummary: RunControlAuditSummary | null;
  latestHistoryStateAuditSummary: HistoryStateAuditSummary | null;
  visibleNodeId: string | null;
  branchHeadNodeId: string | null;
  activeBranchId: string | null;
  historyCursorMode: HistoryCursorMode;
  historyNodes: HistoryNode[];
  historyBranches: HistoryBranch[];
  eventsReady: boolean;
  deferredPersistTimerId: number | null;
  streamFlushFrameId: number | null;
  streamBufferTurnId: string | null;
  streamBufferText: string;
  streamBufferReasoning: string;
  streamDebugDeltaCount: number;
  streamDebugFlushCount: number;
  streamDebugLastDeltaAtMs: number | null;
  streamDebugLastFlushAtMs: number | null;
  streamDebugReasoningCharsReceived: number;
  streamDebugReasoningCharsFlushed: number;
  streamDebugTextCharsReceived: number;
  streamDebugTextCharsFlushed: number;
  browserPreviewRunToken: number;
};

type PersistedRuntimeState = {
  phase: RuntimePhase;
  messages: ChatMessage[];
  attachmentAssets: AttachmentAsset[];
  turnTraceHistory: TurnTraceRecord[];
  sessionSummary: string;
  providerRequestedName: string;
  providerName: string;
  providerProtocol: string;
  providerModel: string;
  providerSource: string;
  providerMode: string;
  fallbackReason: string | null;
  inputTokens: number | null;
  outputTokens: number | null;
  totalTokens: number | null;
  firstTokenLatencyMs: number | null;
};

type SessionRuntimeSnapshot = {
  sessionId: string;
  sessionList: SessionOverview[];
  phase: RuntimePhase;
  error: string | null;
  draftMessage: string;
  sessionSummary: string;
  retrievedContext: RetrievedContextState | null;
  providerRequestedName: string;
  providerName: string;
  providerProtocol: string;
  providerModel: string;
  providerSource: string;
  providerMode: string;
  fallbackReason: string | null;
  inputTokens: number | null;
  outputTokens: number | null;
  totalTokens: number | null;
  firstTokenLatencyMs: number | null;
  isSubmitting: boolean;
  activeTurnId: string | null;
  activeRunId: string | null;
  latestExecutionCheckpoint: ExecutionCheckpoint | null;
  latestGraphRunSubmissionPlan: GraphRunSubmissionPlan | null;
  latestGraphRunControlBoundaryEvidence: GraphRunControlBoundaryEvidence[];
  latestRunControlAuditSummary: RunControlAuditSummary | null;
  latestHistoryStateAuditSummary: HistoryStateAuditSummary | null;
  visibleNodeId: string | null;
  branchHeadNodeId: string | null;
  activeBranchId: string | null;
  historyCursorMode: HistoryCursorMode;
  historyNodes: HistoryNode[];
  historyBranches: HistoryBranch[];
  messages: ChatMessage[];
  attachmentAssets: AttachmentAsset[];
  toolActivities: ToolActivity[];
  traceSteps: TraceStep[];
  traceTimeline: TraceTimelineEntry[];
  turnTraceHistory: TurnTraceRecord[];
  eventCursorByTurnId: Record<string, { eventId: string | null; sequence: number | null; emittedAtMs: number | null }>;
};

const RUNTIME_STORAGE_KEY = "pony-agent.runtime-history.v1";
const DEFAULT_SESSION_ID = "local-dev-session";

type PersistedRuntimeCache = {
  sessions: Record<string, PersistedRuntimeState>;
};

const DEFAULT_BROWSER_SESSION_SUMMARY = "浏览器预览会话";
const DEFAULT_FAILED_TURN_MESSAGE = "本轮执行失败，请查看右侧 trace。";
const DEFAULT_FAILED_TURN_ERROR = "本轮执行失败。";
const BROWSER_PREVIEW_PROVIDER_NAME = "browser-preview";
const BROWSER_PREVIEW_MODEL_NAME = "mock-stream";
const BROWSER_PREVIEW_FALLBACK_REASON =
  "当前通过 npm run dev 打开的是浏览器预览，而不是 Tauri 桌面窗口，因此不会连接 Rust 后端。";
const BROWSER_PREVIEW_SESSION_SUMMARY = "浏览器预览模式已启用，当前轮次未连接 Rust 后端。";
const RETRIEVED_CONTEXT_FALLBACK_SUMMARY = "当前会话尚未从 Tauri 宿主加载结构化 retrieval 上下文。";
const BROWSER_PREVIEW_TRACE_TITLE = "浏览器预览";
const BROWSER_PREVIEW_CHUNKS = [
  "当前看到的不是前端资源没加载，而是页面运行在普通浏览器里。\n\n",
  "此时 @tauri-apps/api 不会注入原生桥接能力，所以直接调用 invoke/listen 会失败。\n\n",
  "现在已切换到浏览器预览兜底模式：\n",
  "- 可以继续预览 UI 和输入交互\n",
  "- 不会连接 Rust agent core\n",
  "- 真正联调需要运行 tauri dev\n"
];
const TRACE_STEP_LABELS = {
  plan: "接收输入",
  context: "组织上下文",
  contextBrowser: "识别运行环境",
  callModel: "调用模型",
  callModelBrowser: "浏览器预览回放",
  callTool: "调用工具",
  return: "返回结果"
} as const;

function debugLog(event: string, payload?: Record<string, unknown>) {
  const message = {
    event,
    payload: payload ?? {},
    ts: new Date().toISOString()
  };
  console.info("[pony-agent][runtime]", message);
}

function errorLog(event: string, payload?: Record<string, unknown>) {
  const message = {
    event,
    payload: payload ?? {},
    ts: new Date().toISOString()
  };
  console.error("[pony-agent][runtime]", message);
}

const STREAM_DEBUG_STORAGE_KEY = "pony-agent.debug.stream-metrics";

type StreamDebugBucket = {
  runtime?: Record<string, unknown>;
  reveal?: Record<string, unknown>;
};

let scheduledRuntimeMetricsPush = false;
let pendingRuntimeMetricsPatch: Record<string, unknown> | null = null;

function swallowAsyncError(result: unknown) {
  if (result && typeof result === "object" && "catch" in result && typeof result.catch === "function") {
    void result.catch(() => {});
  }
}

function streamDebugEnabled() {
  if (typeof window === "undefined") {
    return false;
  }

  return import.meta.env.DEV || window.localStorage.getItem(STREAM_DEBUG_STORAGE_KEY) === "true";
}

function updateStreamDebugBucket(section: keyof StreamDebugBucket, patch: Record<string, unknown>) {
  if (!streamDebugEnabled() || typeof window === "undefined") {
    return;
  }

  const streamWindow = window as Window & {
    __ponyStreamMetrics?: StreamDebugBucket;
  };

  const current = streamWindow.__ponyStreamMetrics ?? {};
  streamWindow.__ponyStreamMetrics = {
    ...current,
    [section]: {
      ...(current[section] ?? {}),
      ...patch,
      updatedAt: Date.now()
    }
  };

  if (section !== "runtime" || !isTauriAvailable()) {
    return;
  }

  pendingRuntimeMetricsPatch = {
    ...((streamWindow.__ponyStreamMetrics.runtime as Record<string, unknown> | undefined) ?? {})
  };
  if (scheduledRuntimeMetricsPush) {
    return;
  }

  scheduledRuntimeMetricsPush = true;
  window.setTimeout(() => {
    scheduledRuntimeMetricsPush = false;
    const payload = pendingRuntimeMetricsPatch;
    pendingRuntimeMetricsPatch = null;
    if (!payload) {
      return;
    }

    swallowAsyncError(safeInvoke("record_stream_debug_metrics", {
      section: "runtime",
      payload
    }));
  }, 250);
}

function toolStatusToMessageStatus(status: ToolActivity["status"]): ChatMessage["status"] {
  switch (status) {
    case "done":
      return "done";
    case "error":
      return "error";
    default:
      return "pending";
  }
}

function cloneTraceSteps(traceSteps?: TraceStep[] | null) {
  return (traceSteps ?? [])
    .filter((step) => step.id !== "step-return")
    .map((step) => ({ ...step }));
}

function canonicalizeTraceTimelineKind(kind: TraceTimelineEntry["kind"]): TraceTimelineEntry["kind"] {
  switch (kind) {
    case "context":
      return "build_context";
    case "model":
      return "call_model";
    case "tool":
      return "call_tool";
    case "return":
      return "return_result";
    default:
      return kind;
  }
}

function cloneTraceTimeline(traceTimeline?: TraceTimelineEntry[] | null): TraceTimelineEntry[] {
  const normalized = (traceTimeline ?? []).map((entry): TraceTimelineEntry => ({
    ...entry,
    kind: canonicalizeTraceTimelineKind(entry.kind),
    buildContextObservation: cloneBuildContextObservation(entry.buildContextObservation),
    toolActivities: cloneToolActivities(entry.toolActivities)
  }));

  const folded: TraceTimelineEntry[] = [];
  for (const entry of normalized) {
    if (entry.kind !== "return_result") {
      folded.push(entry);
      continue;
    }

    const reverseModelIndex = [...folded].reverse().findIndex((candidate) => candidate.kind === "call_model");
    if (reverseModelIndex === -1) {
      folded.push({
        ...entry,
        id: `model-${entry.sequence}`,
        kind: "call_model",
        label: "CALL MODEL #1"
      });
      continue;
    }

    const modelIndex = folded.length - 1 - reverseModelIndex;
    const modelEntry = folded[modelIndex];
    folded[modelIndex] = {
      ...modelEntry,
      state: entry.state ?? modelEntry.state,
      text: entry.text ?? modelEntry.text ?? null,
      reasoningContent: entry.reasoningContent ?? modelEntry.reasoningContent ?? null,
      fallbackReason: entry.fallbackReason ?? modelEntry.fallbackReason ?? null,
      error: entry.error ?? modelEntry.error ?? null,
      inputTokens: entry.inputTokens ?? modelEntry.inputTokens ?? null,
      cacheHitInputTokens: entry.cacheHitInputTokens ?? modelEntry.cacheHitInputTokens ?? null,
      reasoningTokens: entry.reasoningTokens ?? modelEntry.reasoningTokens ?? null,
      outputTokens: entry.outputTokens ?? modelEntry.outputTokens ?? null,
      totalTokens: entry.totalTokens ?? modelEntry.totalTokens ?? null,
      firstTokenLatencyMs: entry.firstTokenLatencyMs ?? modelEntry.firstTokenLatencyMs ?? null,
      turnDurationMs: entry.turnDurationMs ?? modelEntry.turnDurationMs ?? null
    };
  }

  return folded;
}

function cloneProviderCallRecords(
  providerCallRecords?: ProviderCallCacheRecord[] | null
): ProviderCallCacheRecord[] {
  return (providerCallRecords ?? []).map((record) => ({
    ...record,
    prefixMutationReasons: [...(record.prefixMutationReasons ?? [])]
  }));
}

function cloneHookPatchOperations(operations?: HookPatchOperation[] | null): HookPatchOperation[] {
  return (operations ?? []).map((operation) => ({ ...operation }));
}

function cloneHookStructuredResult(result: HookStructuredResult): HookStructuredResult {
  switch (result.resultKind) {
    case "observe":
    case "allow":
      return {
        resultKind: result.resultKind,
        payload: { ...result.payload }
      };
    case "deny":
      return {
        resultKind: "deny",
        payload: { ...result.payload }
      };
    case "patch":
      return {
        resultKind: "patch",
        payload: {
          operations: cloneHookPatchOperations(result.payload.operations)
        }
      };
    case "side_effect_request":
      return {
        resultKind: "side_effect_request",
        payload: { ...result.payload }
      };
    default:
      return result;
  }
}

function cloneHookTraceRecords(hookTraceRecords?: HookTraceRecord[] | null): HookTraceRecord[] {
  return (hookTraceRecords ?? []).map((record) => ({
    ...record,
    structuredResult: cloneHookStructuredResult(record.structuredResult)
  }));
}

function cloneToolActivities(toolActivities?: ToolActivity[] | null) {
  return (toolActivities ?? []).map((tool) => ({
    ...tool,
    capabilityInvocation: tool.capabilityInvocation ? { ...tool.capabilityInvocation } : null
  }));
}

function cloneBuildContextObservation(buildContextObservation?: TurnTraceRecord["buildContextObservation"]) {
  return buildContextObservation ? { ...buildContextObservation } : null;
}

function clonePayloadTraceTimeline(payload: { traceTimeline?: TraceTimelineEntry[] | null }) {
  return cloneTraceTimeline(payload.traceTimeline);
}

function resolvedStreamStartRunId(
  response: Partial<GraphRunStreamStartResponse> | null | undefined,
  fallbackRunId?: string | null
) {
  const responseRunId = response?.run?.id?.trim();
  if (responseRunId) {
    return responseRunId;
  }

  const normalizedFallback = fallbackRunId?.trim();
  return normalizedFallback || null;
}

function resolveEventTraceTimeline(
  payload: { traceTimeline?: TraceTimelineEntry[] | null },
  fallback: () => TraceTimelineEntry[]
) {
  const payloadTraceTimeline = clonePayloadTraceTimeline(payload);
  return payloadTraceTimeline.length ? payloadTraceTimeline : fallback();
}

function buildFallbackRuntimeTraceTimeline(options: {
  turnId: string;
  eventType?: TurnStreamEvent["eventType"];
  messages: ChatMessage[];
  phase: RuntimePhase | string | null | undefined;
  buildContextObservation?: TurnTraceRecord["buildContextObservation"];
  assistantMessage?: ChatMessage | null;
  toolActivities?: ToolActivity[] | null;
  providerPatch: Pick<
    TraceTimelineEntry,
    "providerName" | "providerProtocol" | "providerModel" | "providerSource" | "providerMode"
  >;
  terminalState?: "completed" | "error" | "cancelled" | null;
  fallbackReason?: string | null;
  error?: string | null;
  inputTokens?: number | null;
  cacheHitInputTokens?: number | null;
  reasoningTokens?: number | null;
  outputTokens?: number | null;
  totalTokens?: number | null;
  firstTokenLatencyMs?: number | null;
  turnDurationMs?: number | null;
}) {
  const {
    turnId,
    eventType,
    messages,
    phase,
    buildContextObservation,
    assistantMessage,
    toolActivities,
    providerPatch,
    terminalState,
    fallbackReason,
    error,
    inputTokens,
    cacheHitInputTokens,
    reasoningTokens,
    outputTokens,
    totalTokens,
    firstTokenLatencyMs,
    turnDurationMs
  } = options;

  const normalizedToolActivities = toolActivities ?? [];
  const topLevelTools = normalizedToolActivities.filter((activity) => !activity.id.includes("-child-") && !activity.id.includes("-planned-"));
  const userInputText = messages.find((message) => message.turnId === turnId && message.role === "user")?.content ?? null;
  const timeline: TraceTimelineEntry[] = [];
  const resolvedRuntimePhase = resolveFallbackTimelineRuntimePhase(eventType, phase);

  timeline.push(createTimelineEntry("input", 1, undefined, {
    state: "completed",
    text: userInputText
  }));
  let sequence = 2;
  if (traceUsesRetrieval(buildContextObservation)) {
    timeline.push(createTimelineEntry("prepare_retrieval", sequence, undefined, {
      state: resolvedRuntimePhase === "connecting" ? "active" : "completed",
      ...providerPatch
    }));
    sequence += 1;
  }
  timeline.push(createTimelineEntry("build_context", sequence, undefined, {
    state: resolvedRuntimePhase === "connecting" ? "active" : "completed",
    buildContextObservation,
    ...providerPatch
  }));
  sequence += 1;

  const isTerminal = terminalState != null;
  const isCallingTool = resolvedRuntimePhase === "calling_tool";
  const isCallingModel = resolvedRuntimePhase === "calling_model";
  const isConnecting = resolvedRuntimePhase === "connecting";
  const modelHopCount = isTerminal || isCallingModel
    ? topLevelTools.length + 1
    : Math.max(topLevelTools.length, 1);

  for (let modelIndex = 0; modelIndex < modelHopCount; modelIndex += 1) {
    const isLastModel = modelIndex === modelHopCount - 1;
    let modelState: TraceTimelineEntry["state"] = "completed";
    if (terminalState === "error" && isLastModel) {
      modelState = "error";
    } else if (terminalState === "cancelled" && isLastModel) {
      modelState = "cancelled";
    } else if (!isTerminal && isCallingModel && isLastModel) {
      modelState = "active";
    } else if (!isTerminal && isConnecting && isLastModel) {
      modelState = "pending";
    }

    timeline.push(createTimelineEntry("call_model", sequence, modelIndex + 1, {
      state: modelState,
      text: null,
      reasoningContent: !isTerminal && isCallingModel && isLastModel ? assistantMessage?.reasoningContent ?? null : null,
      firstTokenLatencyMs: !isTerminal && isCallingModel && isLastModel ? firstTokenLatencyMs ?? null : null,
      ...providerPatch
    }));
    sequence += 1;

    const parentTool = topLevelTools[modelIndex];
    if (parentTool) {
      let toolState: TraceTimelineEntry["state"] = parentTool.status === "error" ? "error" : "completed";
      if (!isTerminal && isCallingTool && modelIndex === topLevelTools.length - 1) {
        toolState = parentTool.status === "error" ? "error" : "active";
      }
      if (terminalState === "cancelled" && toolState === "active") {
        toolState = "cancelled";
      }

      timeline.push(createTimelineEntry("call_tool", sequence, modelIndex + 1, {
        label: `CALL TOOL #${modelIndex + 1} · ${parentTool.name}`,
        state: toolState,
        toolActivities: toolActivitiesForHop(normalizedToolActivities, parentTool.id),
        text: parentTool.summary ?? null,
        error: parentTool.status === "error" ? parentTool.summary : null
      }));
      sequence += 1;
    }
  }

  if (!isTerminal && isCallingTool && topLevelTools.length === 0) {
    timeline.push(createTimelineEntry("call_tool", sequence, 1, { state: "active" }));
    sequence += 1;
  }

  if (terminalState) {
    const reverseModelIndex = [...timeline].reverse().findIndex((entry) => entry.kind === "call_model");
    if (reverseModelIndex !== -1) {
      const modelIndex = timeline.length - 1 - reverseModelIndex;
      const modelEntry = timeline[modelIndex];
      timeline[modelIndex] = {
        ...modelEntry,
        state: terminalState,
        text: assistantMessage?.content ?? modelEntry.text ?? null,
        reasoningContent: assistantMessage?.reasoningContent ?? modelEntry.reasoningContent ?? null,
        fallbackReason: fallbackReason ?? modelEntry.fallbackReason ?? null,
        error: error ?? modelEntry.error ?? null,
        inputTokens: inputTokens ?? modelEntry.inputTokens ?? null,
        cacheHitInputTokens: cacheHitInputTokens ?? modelEntry.cacheHitInputTokens ?? null,
        reasoningTokens: reasoningTokens ?? modelEntry.reasoningTokens ?? null,
        outputTokens: outputTokens ?? modelEntry.outputTokens ?? null,
        totalTokens: totalTokens ?? modelEntry.totalTokens ?? null,
        firstTokenLatencyMs: firstTokenLatencyMs ?? modelEntry.firstTokenLatencyMs ?? null,
        turnDurationMs: turnDurationMs ?? modelEntry.turnDurationMs ?? null,
        ...providerPatch
      };
    }
  }

  return timeline;
}

function cloneMessages(messages?: ChatMessage[] | null) {
  return (messages ?? []).map((message) => ({ ...message }));
}

function cloneAttachmentAssets(assets?: AttachmentAsset[] | null) {
  return (assets ?? []).map((asset) => ({ ...asset }));
}

function cloneHistoryNodes(nodes?: HistoryNode[] | null) {
  return (nodes ?? []).map((node) => ({
    ...node,
    workspaceRef: node.workspaceRef ? { ...node.workspaceRef } : node.workspaceRef ?? null,
    history: (node.history ?? []).map((message) => ({
      ...message,
      attachments: (message.attachments ?? []).map((attachment) => ({ ...attachment }))
    })),
    turnTraceHistory: (node.turnTraceHistory ?? []).map((trace) => normalizeTurnTraceRecord(trace))
  }));
}

function cloneHistoryBranches(branches?: HistoryBranch[] | null) {
  return (branches ?? []).map((branch) => ({ ...branch }));
}

function cloneHistoryCursor(cursor?: HistoryCursorState | null) {
  return cursor ? { ...cursor } : null;
}

function cloneHistoryStateEvidence(evidence?: HistoryStateHookEvidence[] | null) {
  return (evidence ?? []).map((item) => ({ ...item }));
}

function cloneHistoryStateAuditSummary(
  summary?: HistoryStateAuditSummary | null
): HistoryStateAuditSummary | null {
  if (!summary) {
    return null;
  }

  return {
    action: { ...summary.action },
    currentContext: { ...summary.currentContext }
  };
}

function cloneRunControlAuditSummary(
  summary?: RunControlAuditSummary | null
): RunControlAuditSummary | null {
  if (!summary) {
    return null;
  }

  return {
    actionEvidenceSummary: { ...summary.actionEvidenceSummary },
    currentContextProjection: { ...summary.currentContextProjection }
  };
}

function normalizeHistoryCheckoutResult(
  payload: HistoryCheckoutWireResult,
  historyNodes?: HistoryNode[] | null,
  historyBranches?: HistoryBranch[] | null
): HistoryCheckoutResult {
  return {
    sessionId: payload.sessionId,
    nodeId: payload.nodeId,
    visibleNodeId: payload.cursor.visibleNodeId ?? null,
    activeBranchId: payload.cursor.activeBranchId ?? null,
    branchHeadNodeId: payload.cursor.branchHeadNodeId ?? null,
    workspaceNodeId: payload.cursor.workspaceNodeId ?? null,
    mode: payload.cursor.mode,
    requestedMode: payload.requestedMode,
    appliedMode: payload.appliedMode,
    transcriptRestoreApplied: payload.transcriptRestoreApplied,
    workspaceRollbackCapable: payload.workspaceRollbackCapable,
    workspaceRestoreCapable: payload.workspaceRollbackCapable,
    workspaceRollbackApplied: payload.workspaceRollbackApplied,
    workspaceRestoreApplied: payload.workspaceRollbackApplied,
    degraded: payload.degraded,
    degradedToTranscriptOnly: payload.degraded,
    degradationReason: payload.degradationReason ?? null,
    historyStateEvidence: cloneHistoryStateEvidence(payload.historyStateEvidence),
    historyStateAuditSummary: cloneHistoryStateAuditSummary(payload.historyStateAuditSummary),
    historyNodes: cloneHistoryNodes(historyNodes),
    historyBranches: cloneHistoryBranches(historyBranches)
  };
}

function normalizeHistoryRestoreResult(
  payload: HistoryRestoreWireResult,
  historyNodes?: HistoryNode[] | null,
  historyBranches?: HistoryBranch[] | null
): HistoryRestoreResult {
  const legacyRestoredFromNodeId = (
    payload as HistoryRestoreWireResult & { restoredFromNodeId?: string | null }
  ).restoredFromNodeId;
  return {
    sessionId: payload.sessionId,
    branchId: payload.branchId ?? null,
    restoredNodeId: payload.restoredNodeId ?? null,
    visibleNodeId: payload.cursor.visibleNodeId ?? null,
    activeBranchId: payload.cursor.activeBranchId ?? null,
    branchHeadNodeId: payload.cursor.branchHeadNodeId ?? null,
    workspaceNodeId: payload.cursor.workspaceNodeId ?? null,
    mode: payload.cursor.mode,
    transcriptRestoreApplied: payload.transcriptRestoreApplied,
    workspaceRollbackCapable: payload.workspaceRollbackCapable,
    workspaceRestoreCapable: payload.workspaceRollbackCapable,
    workspaceRollbackApplied: payload.workspaceRollbackApplied,
    workspaceRestoreApplied: payload.workspaceRollbackApplied,
    degraded: payload.degraded,
    degradedToTranscriptOnly: payload.degraded,
    degradationReason: payload.degradationReason ?? null,
    restoredFromNodeId: legacyRestoredFromNodeId ?? payload.restoredNodeId ?? null,
    historyStateEvidence: cloneHistoryStateEvidence(payload.historyStateEvidence),
    historyStateAuditSummary: cloneHistoryStateAuditSummary(payload.historyStateAuditSummary),
    historyNodes: cloneHistoryNodes(historyNodes),
    historyBranches: cloneHistoryBranches(historyBranches)
  };
}

function normalizeHistoryForkResult(
  payload: HistoryForkWireResult,
  historyNodes?: HistoryNode[] | null,
  historyBranches?: HistoryBranch[] | null
): HistoryForkResult {
  return {
    sessionId: payload.sessionId,
    nodeId: payload.nodeId,
    createdBranchId: payload.branch.branchId,
    branch: { ...payload.branch },
    visibleNodeId: payload.cursor.visibleNodeId ?? null,
    activeBranchId: payload.cursor.activeBranchId ?? null,
    branchHeadNodeId: payload.cursor.branchHeadNodeId ?? null,
    workspaceNodeId: payload.cursor.workspaceNodeId ?? null,
    mode: payload.cursor.mode,
    historyStateEvidence: cloneHistoryStateEvidence(payload.historyStateEvidence),
    historyStateAuditSummary: cloneHistoryStateAuditSummary(payload.historyStateAuditSummary),
    historyNodes: cloneHistoryNodes(historyNodes),
    historyBranches: cloneHistoryBranches(historyBranches)
  };
}

function normalizeHistoryBranchSwitchResult(
  payload: HistoryBranchSwitchWireResult,
  historyNodes?: HistoryNode[] | null,
  historyBranches?: HistoryBranch[] | null
): HistoryBranchSwitchResult {
  return {
    sessionId: payload.sessionId,
    branchId: payload.branchId,
    nodeId: payload.nodeId ?? null,
    visibleNodeId: payload.cursor.visibleNodeId ?? null,
    activeBranchId: payload.cursor.activeBranchId ?? null,
    branchHeadNodeId: payload.cursor.branchHeadNodeId ?? null,
    workspaceNodeId: payload.cursor.workspaceNodeId ?? null,
    mode: payload.cursor.mode,
    historyStateEvidence: cloneHistoryStateEvidence(payload.historyStateEvidence),
    historyStateAuditSummary: cloneHistoryStateAuditSummary(payload.historyStateAuditSummary),
    historyNodes: cloneHistoryNodes(historyNodes),
    historyBranches: cloneHistoryBranches(historyBranches)
  };
}

function cloneRetrievedContext(retrievedContext?: RetrievedContextState | null) {
  if (!retrievedContext) {
    return null;
  }

  return {
    turnContext: {
      ...retrievedContext.turnContext,
      images: (retrievedContext.turnContext.images ?? []).map((image) => ({ ...image }))
    },
    sessionContext: {
      ...retrievedContext.sessionContext,
      recentHistory: (retrievedContext.sessionContext.recentHistory ?? []).map((message) => ({
        ...message,
        attachments: (message.attachments ?? []).map((attachment) => ({ ...attachment }))
      })),
      recentAttachmentAssets: cloneAttachmentAssets(retrievedContext.sessionContext.recentAttachmentAssets)
    },
    runState: { ...retrievedContext.runState },
    longTermMemory: {
      ...retrievedContext.longTermMemory,
      entries: (retrievedContext.longTermMemory.entries ?? []).map((entry) => ({ ...entry }))
    },
    transcript: {
      providerNativeMessages: [...(retrievedContext.transcript.providerNativeMessages ?? [])]
    }
  };
}

function filterAttachmentAssets(assets: AttachmentAsset[], filter?: AttachmentAssetFilter | null) {
  const normalizedMime = filter?.mimeType?.trim().toLowerCase() ?? "";
  const normalizedName = filter?.nameContains?.trim().toLowerCase() ?? "";
  const requestedStatuses = new Set(filter?.statuses ?? []);

  const filtered = assets.filter((asset) => {
    if (filter?.sessionId?.trim() && asset.sessionId !== filter.sessionId.trim()) {
      return false;
    }

    if (normalizedMime && !asset.mimeType.toLowerCase().includes(normalizedMime)) {
      return false;
    }

    if (normalizedName) {
      const assetName = asset.name?.toLowerCase() ?? "";
      const relativePath = asset.relativePath.toLowerCase();
      if (!assetName.includes(normalizedName) && !relativePath.includes(normalizedName)) {
        return false;
      }
    }

    if (filter?.createdAfterMs != null && asset.createdAtMs < filter.createdAfterMs) {
      return false;
    }

    if (filter?.createdBeforeMs != null && asset.createdAtMs > filter.createdBeforeMs) {
      return false;
    }

    if (requestedStatuses.size > 0) {
      const status = asset.status ?? "active";
      if (!requestedStatuses.has(status)) {
        return false;
      }
    }

    return true;
  });

  filtered.sort((left, right) => {
    if (right.createdAtMs !== left.createdAtMs) {
      return right.createdAtMs - left.createdAtMs;
    }
    return left.id.localeCompare(right.id);
  });

  if (filter?.limit != null) {
    return filtered.slice(0, filter.limit);
  }

  return filtered;
}

function buildToolMessageDetail(tool: ToolActivity) {
  const blocks = [tool.summary.trim()];

  if (tool.argumentsText?.trim()) {
    blocks.push(`参数\n${tool.argumentsText.trim()}`);
  }

  if (tool.resultText?.trim()) {
    blocks.push(`结果\n${tool.resultText.trim()}`);
  }

  return blocks.filter(Boolean).join("\n");
}

function buildTurnTitle(message: string) {
  const compact = message.replace(/\s+/g, " ").trim();
  if (!compact) {
    return "空白输入";
  }

  return compact.length > 44 ? `${compact.slice(0, 44)}…` : compact;
}

function buildTurnTraceTitleFromMessages(messages: ChatMessage[], turnId: string) {
  const userMessage = messages.find((message) => message.turnId === turnId && message.role === "user");
  if (!userMessage?.content.trim()) {
    return "未命名轮次";
  }

  return buildTurnTitle(userMessage.content);
}

function summarizeImageNames(images: TurnInputImage[]) {
  return images
    .map((image, index) => image.name?.trim() || `图片 ${index + 1}`)
    .slice(0, 2)
    .join("、");
}

function buildDisplayedUserMessage(message: string, images: TurnInputImage[]) {
  if (!images.length) {
    return message;
  }

  const imageSummary = `[已附图片 ${images.length} 张${summarizeImageNames(images) ? `：${summarizeImageNames(images)}` : ""}]`;
  if (!message.trim()) {
    return imageSummary;
  }

  return `${message}\n\n${imageSummary}`;
}

function buildProviderUserMessage(message: string, images: TurnInputImage[]) {
  if (message.trim()) {
    return message;
  }

  if (!images.length) {
    return message;
  }

  return "请基于附图回答。";
}

function normalizeReasoningContent(content?: string | null) {
  if (content == null) {
    return null;
  }

  const normalized = content.replace(/^thinking\s*[:：]\s*/i, "");
  return normalized.length > 0 ? normalized : null;
}

function appendNormalizedReasoningContent(current: string | null, delta: string) {
  if (!delta) {
    return current;
  }

  const base = current ?? "";
  const combined = `${base}${delta}`;
  if (base.length <= 24) {
    return normalizeReasoningContent(combined);
  }

  return combined;
}

const STREAM_FLUSH_EAGER_CHARS = 4096;

const TRACE_STEP_IDS = {
  plan: "step-plan",
  context: "step-context",
  contextBrowser: "step-context",
  callModel: "step-call-model",
  callModelBrowser: "step-call-model",
  callTool: "step-call-tool",
  return: "step-return"
} as const;

type TraceStepKey = keyof typeof TRACE_STEP_LABELS;
type TraceStepState = TraceStep["state"];

function buildTraceSteps(entries: Array<{ key: TraceStepKey; state: TraceStepState }>) {
  return entries.map(({ key, state }) => ({
    id: TRACE_STEP_IDS[key],
    label: TRACE_STEP_LABELS[key],
    state
  }));
}

function createDefaultTraceSteps() {
  return buildTraceSteps([
    { key: "plan", state: "completed" },
    { key: "context", state: "active" },
    { key: "callModel", state: "pending" },
    { key: "callTool", state: "pending" }
  ]);
}

function createSubmitTraceSteps() {
  return buildTraceSteps([
    { key: "plan", state: "completed" },
    { key: "context", state: "completed" },
    { key: "callModel", state: "active" },
    { key: "callTool", state: "pending" }
  ]);
}

function createBrowserPreviewTraceSteps() {
  return buildTraceSteps([
    { key: "plan", state: "completed" },
    { key: "contextBrowser", state: "completed" },
    { key: "callModelBrowser", state: "completed" },
    { key: "callTool", state: "pending" }
  ]);
}

function createSubmitFailureTraceSteps() {
  return buildTraceSteps([
    { key: "plan", state: "completed" },
    { key: "context", state: "completed" },
    { key: "callModel", state: "error" },
    { key: "callTool", state: "pending" }
  ]);
}

function finalizeCancelledTraceSteps(traceSteps?: TraceStep[] | null): TraceStep[] {
  return (traceSteps ?? []).map((step) => {
    if (step.state === "completed" || step.state === "error") {
      return { ...step };
    }

    const cancelledState: TraceStep["state"] = "cancelled";

    return {
      ...step,
      state: cancelledState
    };
  });
}

function timelineLabel(kind: TraceTimelineEntry["kind"], index?: number) {
  switch (canonicalizeTraceTimelineKind(kind)) {
    case "input":
      return "RECEIVE INPUT";
    case "prepare_retrieval":
      return "PREPARE RETRIEVAL";
    case "build_context":
      return "BUILD CONTEXT";
    case "call_model":
      return `CALL MODEL #${index ?? 1}`;
    case "call_tool":
      return `CALL TOOL #${index ?? 1}`;
    case "return_result":
      return "RETURN RESULT";
    default:
      return "TRACE";
  }
}

function createTimelineEntry(
  kind: TraceTimelineEntry["kind"],
  sequence: number,
  index?: number,
  patch: Partial<TraceTimelineEntry> = {}
): TraceTimelineEntry {
  const canonicalKind = canonicalizeTraceTimelineKind(kind);
  return {
    id: patch.id ?? `${canonicalKind}-${sequence}`,
    kind: canonicalKind,
    label: patch.label ?? timelineLabel(canonicalKind, index),
    state: patch.state ?? "pending",
    sequence,
    providerName: patch.providerName ?? null,
    providerProtocol: patch.providerProtocol ?? null,
    providerModel: patch.providerModel ?? null,
    providerSource: patch.providerSource ?? null,
    providerMode: patch.providerMode ?? null,
    buildContextObservation: cloneBuildContextObservation(patch.buildContextObservation),
    toolActivities: cloneToolActivities(patch.toolActivities),
    text: patch.text ?? null,
    reasoningContent: patch.reasoningContent ?? null,
    fallbackReason: patch.fallbackReason ?? null,
    error: patch.error ?? null,
    inputTokens: patch.inputTokens ?? null,
    cacheHitInputTokens: patch.cacheHitInputTokens ?? null,
    reasoningTokens: patch.reasoningTokens ?? null,
    outputTokens: patch.outputTokens ?? null,
    totalTokens: patch.totalTokens ?? null,
    firstTokenLatencyMs: patch.firstTokenLatencyMs ?? null,
    turnDurationMs: patch.turnDurationMs ?? null
  };
}

function createDefaultTraceTimeline() {
  return [
    createTimelineEntry("input", 1, undefined, { state: "completed" }),
    createTimelineEntry("build_context", 2, undefined, { state: "completed" }),
    createTimelineEntry("call_model", 3, 1, { state: "active" })
  ];
}

function createBrowserPreviewTraceTimeline() {
  return [
    createTimelineEntry("input", 1, undefined, { state: "completed" }),
    createTimelineEntry("build_context", 2, undefined, { state: "completed" }),
    createTimelineEntry("call_model", 3, 1, { state: "completed" })
  ];
}

function createSubmitFailureTraceTimeline() {
  return [
    createTimelineEntry("input", 1, undefined, { state: "completed" }),
    createTimelineEntry("build_context", 2, undefined, { state: "completed" }),
    createTimelineEntry("call_model", 3, 1, { state: "error" })
  ];
}

function traceUsesRetrieval(buildContextObservation?: BuildContextObservation | null) {
  if (!buildContextObservation) {
    return false;
  }

  return (
    buildContextObservation.messageCount > 2 ||
    (buildContextObservation.prefixMutationReasons?.length ?? 0) > 0 ||
    buildContextObservation.semiStableContextText.trim().length > 0
  );
}

function toolActivitiesForHop(toolActivities: ToolActivity[] | null | undefined, parentId?: string | null) {
  if (!toolActivities?.length || !parentId) {
    return [];
  }

  return toolActivities.filter((activity) => activity.id === parentId || activity.id.startsWith(`${parentId}-`));
}

function resolveTerminalToolActivities(
  payloadToolActivities: ToolActivity[] | null | undefined,
  currentToolActivities: ToolActivity[] | null | undefined
) {
  return payloadToolActivities?.length ? payloadToolActivities : (currentToolActivities ?? []);
}

function deriveTraceTimelineFromLegacyTrace(turn: TurnTraceRecord) {
  if (turn.traceTimeline?.length) {
    return cloneTraceTimeline(turn.traceTimeline);
  }

  const timeline: TraceTimelineEntry[] = [];
  let sequence = 1;
  for (const step of turn.traceSteps ?? []) {
    if (step.id === "step-context" && traceUsesRetrieval(turn.buildContextObservation)) {
      timeline.push(
        createTimelineEntry("prepare_retrieval", sequence, undefined, {
          id: `${step.id}-prepare-retrieval`,
          label: "PREPARE RETRIEVAL",
          state: step.state,
          providerName: turn.providerName,
          providerProtocol: turn.providerProtocol,
          providerModel: turn.providerModel,
          providerSource: turn.providerSource,
          providerMode: turn.providerMode,
          fallbackReason: turn.fallbackReason,
          error: turn.error
        })
      );
      sequence += 1;
    }

    const kind: TraceTimelineEntry["kind"] =
      step.id === "step-plan"
        ? "input"
        : step.id === "step-context"
          ? "build_context"
          : step.id === "step-call-model"
            ? "call_model"
            : step.id === "step-call-tool"
              ? "call_tool"
              : "return_result";
    timeline.push(createTimelineEntry(kind, sequence, kind === "call_model" || kind === "call_tool" ? 1 : undefined, {
      id: step.id,
      label: step.label.toUpperCase(),
      state: step.state,
      buildContextObservation: kind === "build_context" ? turn.buildContextObservation : null,
      toolActivities: kind === "call_tool" ? turn.toolActivities : [],
      inputTokens: kind === "return_result" ? turn.inputTokens : null,
      cacheHitInputTokens: kind === "return_result" ? turn.cacheHitInputTokens : null,
      reasoningTokens: kind === "return_result" ? turn.reasoningTokens : null,
      outputTokens: kind === "return_result" ? turn.outputTokens : null,
      totalTokens: kind === "return_result" ? turn.totalTokens : null,
      firstTokenLatencyMs: kind === "call_model" ? turn.firstTokenLatencyMs : null,
      turnDurationMs: kind === "return_result" ? turn.turnDurationMs : null,
      providerName: turn.providerName,
      providerProtocol: turn.providerProtocol,
      providerModel: turn.providerModel,
      providerSource: turn.providerSource,
      providerMode: turn.providerMode,
      fallbackReason: turn.fallbackReason,
      error: turn.error
    }));
    sequence += 1;
  }

  return timeline;
}

function normalizeTurnTraceRecord(trace: TurnTraceRecord): TurnTraceRecord {
  return {
    ...trace,
    buildContextObservation: cloneBuildContextObservation(trace.buildContextObservation),
    traceSteps: cloneTraceSteps(trace.traceSteps),
    traceTimeline: cloneTraceTimeline(deriveTraceTimelineFromLegacyTrace(trace)),
    toolActivities: cloneToolActivities(trace.toolActivities),
    providerCallRecords: cloneProviderCallRecords(trace.providerCallRecords),
    hookTraceRecords: cloneHookTraceRecords(trace.hookTraceRecords)
  };
}

function wait(ms: number) {
  return new Promise((resolve) => window.setTimeout(resolve, ms));
}

function waitForNextPaint() {
  return new Promise<void>((resolve) => {
    window.requestAnimationFrame(() => resolve());
  });
}

const LOW_PRIORITY_TURN_WORK_DELAY_MS = 800;
const LOW_PRIORITY_TURN_WORK_IDLE_TIMEOUT_MS = 2500;
const OUTPUT_END_PERSIST_DELAY_MS = 1200;

function runLowPriorityTurnWork(callback: () => void) {
  if (typeof window === "undefined" || typeof window.requestAnimationFrame !== "function") {
    callback();
    return;
  }

  window.setTimeout(() => {
    const requestIdleCallback = (window as Window & {
      requestIdleCallback?: (handler: IdleRequestCallback, options?: IdleRequestOptions) => number;
    }).requestIdleCallback;

    if (typeof requestIdleCallback === "function") {
      requestIdleCallback(() => callback(), {
        timeout: LOW_PRIORITY_TURN_WORK_IDLE_TIMEOUT_MS
      });
      return;
    }

    window.setTimeout(callback, 0);
  }, LOW_PRIORITY_TURN_WORK_DELAY_MS);
}

function buildAssistantModelLabel(providerName?: string | null, modelName?: string | null) {
  const provider = providerName?.trim();
  const model = modelName?.trim();

  if (provider && model) {
    return `${provider}/${model}`;
  }

  return model || provider || null;
}

function readNumericTokenValue(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function readNestedNumericTokenValue(source: unknown, paths: string[][]): number | null {
  if (!source || typeof source !== "object") {
    return null;
  }

  for (const path of paths) {
    let current: unknown = source;

    for (const segment of path) {
      if (!current || typeof current !== "object") {
        current = null;
        break;
      }

      current = (current as Record<string, unknown>)[segment];
    }

    const resolved = readNumericTokenValue(current);
    if (resolved != null) {
      return resolved;
    }
  }

  return null;
}

function resolveCacheHitInputTokens(source: unknown): number | null {
  const direct = readNestedNumericTokenValue(source, [
    ["cacheHitInputTokens"],
    ["cache_hit_input_tokens"],
    ["promptCacheHitTokens"],
    ["prompt_cache_hit_tokens"],
    ["cachedInputTokens"],
    ["cacheReadInputTokens"],
    ["inputCachedTokens"],
    ["cachedTokens"]
  ]);

  if (direct != null) {
    return direct;
  }

  return readNestedNumericTokenValue(source, [
    ["inputTokensDetails", "cachedTokens"],
    ["input_tokens_details", "cached_tokens"],
    ["promptTokensDetails", "cachedTokens"],
    ["prompt_tokens_details", "cached_tokens"],
    ["usage", "input_tokens_details", "cached_tokens"],
    ["usage", "prompt_tokens_details", "cached_tokens"]
  ]);
}

function resolveProviderReturnedCacheHitInputTokens(source: { providerCallRecords?: ProviderCallCacheRecord[] | null }): number | null {
  const values = (source.providerCallRecords ?? [])
    .map((record) => record.cacheHitInputTokens)
    .filter((value): value is number => typeof value === "number" && Number.isFinite(value));

  return values.length ? values.reduce((sum, value) => sum + value, 0) : null;
}

function resolveReasoningTokens(source: unknown): number | null {
  const direct = readNestedNumericTokenValue(source, [
    ["reasoningTokens"]
  ]);

  if (direct != null) {
    return direct;
  }

  return readNestedNumericTokenValue(source, [
    ["completionTokensDetails", "reasoningTokens"],
    ["completion_tokens_details", "reasoning_tokens"],
    ["outputTokensDetails", "reasoningTokens"],
    ["output_tokens_details", "reasoning_tokens"],
    ["usage", "completion_tokens_details", "reasoning_tokens"],
    ["usage", "output_tokens_details", "reasoning_tokens"]
  ]);
}

function traceTimelineCallModelCacheEvidence(traceTimeline?: TraceTimelineEntry[] | null) {
  return (traceTimeline ?? [])
    .filter((entry) => canonicalizeTraceTimelineKind(entry.kind) === "call_model")
    .map((entry, index) => ({
      index,
      id: entry.id,
      label: entry.label,
      state: entry.state,
      inputTokens: entry.inputTokens ?? null,
      cacheHitInputTokens: entry.cacheHitInputTokens ?? null,
      outputTokens: entry.outputTokens ?? null,
      totalTokens: entry.totalTokens ?? null,
      providerSource: entry.providerSource ?? null,
      providerMode: entry.providerMode ?? null
    }));
}

function providerCallCacheEvidence(providerCallRecords?: ProviderCallCacheRecord[] | null) {
  return (providerCallRecords ?? []).map((record, index) => ({
    index,
    requestKind: record.requestKind,
    providerSource: record.providerSource ?? null,
    providerMode: record.providerMode ?? null,
    inputTokens: record.inputTokens ?? null,
    cacheHitInputTokens: record.cacheHitInputTokens ?? null,
    cacheHitSource: record.cacheHitSource ?? null,
    cacheMissInputTokens: record.cacheMissInputTokens ?? null,
    outputTokens: record.outputTokens ?? null,
    totalTokens: record.totalTokens ?? null,
    latencyKind: record.latencyKind ?? null
  }));
}

function buildCacheTelemetryDebugSnapshot(
  payload: Partial<TurnStreamEvent> | null | undefined,
  traceTimeline?: TraceTimelineEntry[] | null,
  persistedTrace?: TurnTraceRecord | null
) {
  const source = payload as unknown;
  const providerCallRecords = payload?.providerCallRecords ?? [];
  const providerCallsWithCacheHit = providerCallRecords.filter((record) => record.cacheHitInputTokens != null);
  const timelineCallModels = traceTimelineCallModelCacheEvidence(traceTimeline ?? payload?.traceTimeline);
  const timelineCallModelsWithCacheHit = timelineCallModels.filter((entry) => entry.cacheHitInputTokens != null);
  const legacyResolvedCacheHitInputTokens = resolveCacheHitInputTokens(source);
  const providerResolvedCacheHitInputTokens = resolveProviderReturnedCacheHitInputTokens({ providerCallRecords });

  return {
    turnId: payload?.turnId ?? null,
    eventType: payload?.eventType ?? null,
    eventId: payload?.eventId ?? null,
    sequence: payload?.sequence ?? null,
    providerResolvedCacheHitInputTokens,
    legacyResolvedCacheHitInputTokens,
    rawCandidates: {
      camelCase: readNestedNumericTokenValue(source, [["cacheHitInputTokens"]]),
      snakeCase: readNestedNumericTokenValue(source, [["cache_hit_input_tokens"]]),
      promptCacheHitTokens: readNestedNumericTokenValue(source, [["promptCacheHitTokens"]]),
      promptCacheHitTokensSnake: readNestedNumericTokenValue(source, [["prompt_cache_hit_tokens"]]),
      cachedInputTokens: readNestedNumericTokenValue(source, [["cachedInputTokens"]]),
      cacheReadInputTokens: readNestedNumericTokenValue(source, [["cacheReadInputTokens"]]),
      inputCachedTokens: readNestedNumericTokenValue(source, [["inputCachedTokens"]]),
      cachedTokens: readNestedNumericTokenValue(source, [["cachedTokens"]])
    },
    nestedUsageCandidates: {
      inputTokensDetailsCachedTokens: readNestedNumericTokenValue(source, [["inputTokensDetails", "cachedTokens"]]),
      inputTokensDetailsCachedTokensSnake: readNestedNumericTokenValue(source, [["input_tokens_details", "cached_tokens"]]),
      promptTokensDetailsCachedTokens: readNestedNumericTokenValue(source, [["promptTokensDetails", "cachedTokens"]]),
      promptTokensDetailsCachedTokensSnake: readNestedNumericTokenValue(source, [["prompt_tokens_details", "cached_tokens"]]),
      usageInputTokensDetailsCachedTokens: readNestedNumericTokenValue(source, [["usage", "input_tokens_details", "cached_tokens"]]),
      usagePromptTokensDetailsCachedTokens: readNestedNumericTokenValue(source, [["usage", "prompt_tokens_details", "cached_tokens"]])
    },
    topLevelTokens: {
      inputTokens: payload?.inputTokens ?? null,
      cacheHitInputTokens: payload?.cacheHitInputTokens ?? null,
      outputTokens: payload?.outputTokens ?? null,
      totalTokens: payload?.totalTokens ?? null
    },
    providerCallRecords: providerCallCacheEvidence(providerCallRecords),
    traceTimelineCallModels: timelineCallModels,
    persistedTrace: persistedTrace
      ? {
          inputTokens: persistedTrace.inputTokens ?? null,
          cacheHitInputTokens: persistedTrace.cacheHitInputTokens ?? null,
          outputTokens: persistedTrace.outputTokens ?? null,
          totalTokens: persistedTrace.totalTokens ?? null,
          providerCallRecords: providerCallCacheEvidence(persistedTrace.providerCallRecords),
          traceTimelineCallModels: traceTimelineCallModelCacheEvidence(persistedTrace.traceTimeline)
        }
      : null,
    attribution: {
      providerCallRecordCount: providerCallRecords.length,
      providerCallsWithCacheHitCount: providerCallsWithCacheHit.length,
      providerCallsWithCacheHit: providerCallsWithCacheHit.map((record, index) => ({
        index,
        requestKind: record.requestKind,
        providerSource: record.providerSource ?? null,
        cacheHitInputTokens: record.cacheHitInputTokens ?? null
      })),
      timelineCallModelCount: timelineCallModels.length,
      timelineCallModelsWithCacheHitCount: timelineCallModelsWithCacheHit.length,
      hasProviderCacheHitEvidence: providerCallsWithCacheHit.length > 0,
      hasTimelineCacheHitEvidence: timelineCallModelsWithCacheHit.length > 0,
      onlyTopLevelOrNestedEvidence:
        legacyResolvedCacheHitInputTokens != null &&
        providerCallsWithCacheHit.length === 0 &&
        timelineCallModelsWithCacheHit.length === 0
    }
  };
}

function logCacheTelemetryContractViolations(terminalEvent: string, payload: TurnStreamEvent) {
  const legacyResolvedCacheHitInputTokens = resolveCacheHitInputTokens(payload);
  const providerResolvedCacheHitInputTokens = resolveProviderReturnedCacheHitInputTokens(payload);

  if (legacyResolvedCacheHitInputTokens != null && providerResolvedCacheHitInputTokens == null) {
    errorLog("cache-telemetry:error:non-provider-cache-hit", {
      terminalEvent,
      message: "Cache hit tokens were present outside providerCallRecords and were ignored.",
      ...buildCacheTelemetryDebugSnapshot(payload)
    });
    return;
  }

  if (
    legacyResolvedCacheHitInputTokens != null &&
    providerResolvedCacheHitInputTokens != null &&
    legacyResolvedCacheHitInputTokens !== providerResolvedCacheHitInputTokens
  ) {
    errorLog("cache-telemetry:error:cache-hit-mismatch", {
      terminalEvent,
      message: "Top-level or nested cache hit tokens differ from providerCallRecords; providerCallRecords are authoritative.",
      ...buildCacheTelemetryDebugSnapshot(payload)
    });
  }
}

function createBlankSessionRuntimeFields() {
  return {
    sessionSummary: "",
    providerRequestedName: "",
    providerName: "",
    providerProtocol: "",
    providerModel: "",
    providerSource: "",
    providerMode: "",
    fallbackReason: null as string | null,
    inputTokens: null as number | null,
    outputTokens: null as number | null,
    totalTokens: null as number | null,
    firstTokenLatencyMs: null as number | null
  };
}

function cloneGraphRunControlBoundaryEvidence(
  evidence?: GraphRunControlBoundaryEvidence[] | null
): GraphRunControlBoundaryEvidence[] {
  return (evidence ?? []).map((item) => ({
    ...item,
    hookEnvelope: { ...item.hookEnvelope }
  }));
}

function normalizeHistoryCursorMode(mode?: string | null): HistoryCursorMode {
  if (mode === "historical" || mode === "historical_dirty") {
    return mode;
  }

  return "live";
}

function resolveHistoryBranchHeadNodeId(branchId: string | null, branches: HistoryBranch[]) {
  if (!branchId) {
    return null;
  }

  return branches.find((branch) => branch.branchId === branchId)?.headNodeId ?? null;
}

function historyNodeStableTurnId(node: HistoryNode | null | undefined) {
  const explicitTurnId = node?.turnId?.trim() || "";
  if (explicitTurnId) {
    return explicitTurnId;
  }

  const traceTurnId = node?.turnTraceHistory?.[node.turnTraceHistory.length - 1]?.turnId?.trim() || "";
  if (traceTurnId) {
    return traceTurnId;
  }

  return null;
}

function buildConversationCheckpointEntries(
  historyNodes: HistoryNode[],
  historyBranches: HistoryBranch[],
  activeBranchId: string | null,
  visibleNodeId: string | null,
  branchHeadNodeId: string | null
): ConversationCheckpointEntry[] {
  const entriesByNodeId = new Map<string, ConversationCheckpointEntry>();
  const forkBranchesByNodeId = new Map<string, HistoryBranch[]>();

  for (const branch of historyBranches) {
    const sourceNodeId = branch.forkedFromNodeId?.trim() || "";
    if (!sourceNodeId) {
      continue;
    }

    const existing = forkBranchesByNodeId.get(sourceNodeId) ?? [];
    existing.push(branch);
    forkBranchesByNodeId.set(sourceNodeId, existing);
  }

  const latestNodeId = branchHeadNodeId?.trim() || null;

  for (const node of historyNodes) {
    const turnId = historyNodeStableTurnId(node);
    if (!turnId) {
      continue;
    }

    const workspaceRollbackCapable = Boolean(node.workspaceRef?.rollbackCapable);
    const forkTargets = (forkBranchesByNodeId.get(node.nodeId) ?? [])
      .map((branch) => {
        const targetNodeId = branch.headNodeId?.trim() || "";
        if (!targetNodeId) {
          return null;
        }

        const targetNode = historyNodes.find((item) => item.nodeId === targetNodeId) ?? null;
        return {
          branchId: branch.branchId,
          nodeId: targetNodeId,
          label: branch.label?.trim() || branch.branchId,
          summary: targetNode?.summary?.trim() || branch.label?.trim() || branch.branchId,
          isActive: branch.branchId === activeBranchId
        };
      })
      .filter((target): target is ConversationCheckpointEntry["forkTargets"][number] => Boolean(target));

    entriesByNodeId.set(node.nodeId, {
      nodeId: node.nodeId,
      turnId,
      branchId: node.branchId,
      summary: node.summary?.trim() || node.title?.trim() || node.nodeId,
      createdAtMs: node.createdAtMs,
      isLatest: latestNodeId != null && node.nodeId === latestNodeId,
      isVisible: visibleNodeId != null && node.nodeId === visibleNodeId,
      workspaceRollbackCapable,
      availableModes: workspaceRollbackCapable
        ? ["transcript_only", "transcript_and_workspace"]
        : ["transcript_only"],
      forkTargets
    });
  }

  return [...entriesByNodeId.values()].sort((left, right) => right.createdAtMs - left.createdAtMs);
}

function createSessionRuntimeSnapshot(state: RuntimeState): SessionRuntimeSnapshot {
  return {
    sessionId: state.sessionId,
    sessionList: state.sessionList.map((session) => ({ ...session })),
    phase: state.phase,
    error: state.error,
    draftMessage: state.draftMessage,
    sessionSummary: state.sessionSummary,
    retrievedContext: cloneRetrievedContext(state.retrievedContext),
    providerRequestedName: state.providerRequestedName,
    providerName: state.providerName,
    providerProtocol: state.providerProtocol,
    providerModel: state.providerModel,
    providerSource: state.providerSource,
    providerMode: state.providerMode,
    fallbackReason: state.fallbackReason,
    inputTokens: state.inputTokens,
    outputTokens: state.outputTokens,
    totalTokens: state.totalTokens,
    firstTokenLatencyMs: state.firstTokenLatencyMs,
    isSubmitting: state.isSubmitting,
    activeTurnId: state.activeTurnId,
    activeRunId: state.activeRunId,
    latestExecutionCheckpoint: state.latestExecutionCheckpoint ? { ...state.latestExecutionCheckpoint } : null,
    latestGraphRunSubmissionPlan: state.latestGraphRunSubmissionPlan
      ? { ...state.latestGraphRunSubmissionPlan }
      : null,
    latestGraphRunControlBoundaryEvidence: cloneGraphRunControlBoundaryEvidence(
      state.latestGraphRunControlBoundaryEvidence
    ),
    latestRunControlAuditSummary: cloneRunControlAuditSummary(state.latestRunControlAuditSummary),
    latestHistoryStateAuditSummary: cloneHistoryStateAuditSummary(
      state.latestHistoryStateAuditSummary
    ),
    visibleNodeId: state.visibleNodeId,
    branchHeadNodeId: state.branchHeadNodeId,
    activeBranchId: state.activeBranchId,
    historyCursorMode: state.historyCursorMode,
    historyNodes: cloneHistoryNodes(state.historyNodes),
    historyBranches: cloneHistoryBranches(state.historyBranches),
    messages: cloneMessages(state.messages),
    attachmentAssets: cloneAttachmentAssets(state.attachmentAssets),
      toolActivities: cloneToolActivities(state.toolActivities),
      traceSteps: cloneTraceSteps(state.traceSteps),
      traceTimeline: cloneTraceTimeline(state.traceTimeline),
      turnTraceHistory: state.turnTraceHistory.map((trace) => normalizeTurnTraceRecord(trace)),
      eventCursorByTurnId: Object.fromEntries(
        Object.entries(state.eventCursorByTurnId).map(([turnId, cursor]) => [turnId, { ...cursor }])
      )
  };
}

function restoreSessionRuntimeSnapshot(state: RuntimeState, snapshot: SessionRuntimeSnapshot) {
  if (state.streamFlushFrameId != null) {
    window.cancelAnimationFrame(state.streamFlushFrameId);
  }
  state.sessionId = snapshot.sessionId;
  state.sessionList = snapshot.sessionList.map((session) => ({ ...session }));
  state.phase = snapshot.phase;
  state.error = snapshot.error;
  state.draftMessage = snapshot.draftMessage;
  state.sessionSummary = snapshot.sessionSummary;
  state.retrievedContext = cloneRetrievedContext(snapshot.retrievedContext);
  state.providerRequestedName = snapshot.providerRequestedName;
  state.providerName = snapshot.providerName;
  state.providerProtocol = snapshot.providerProtocol;
  state.providerModel = snapshot.providerModel;
  state.providerSource = snapshot.providerSource;
  state.providerMode = snapshot.providerMode;
  state.fallbackReason = snapshot.fallbackReason;
  state.inputTokens = snapshot.inputTokens;
  state.outputTokens = snapshot.outputTokens;
  state.totalTokens = snapshot.totalTokens;
  state.firstTokenLatencyMs = snapshot.firstTokenLatencyMs;
  state.isSubmitting = snapshot.isSubmitting;
  state.activeTurnId = snapshot.activeTurnId;
  state.activeRunId = snapshot.activeRunId;
  state.latestExecutionCheckpoint = snapshot.latestExecutionCheckpoint ? { ...snapshot.latestExecutionCheckpoint } : null;
  state.latestGraphRunSubmissionPlan = snapshot.latestGraphRunSubmissionPlan
    ? { ...snapshot.latestGraphRunSubmissionPlan }
    : null;
  state.latestGraphRunControlBoundaryEvidence = cloneGraphRunControlBoundaryEvidence(
    snapshot.latestGraphRunControlBoundaryEvidence
  );
  state.latestRunControlAuditSummary = cloneRunControlAuditSummary(snapshot.latestRunControlAuditSummary);
  state.latestHistoryStateAuditSummary = cloneHistoryStateAuditSummary(
    snapshot.latestHistoryStateAuditSummary
  );
  state.visibleNodeId = snapshot.visibleNodeId;
  state.branchHeadNodeId = snapshot.branchHeadNodeId;
  state.activeBranchId = snapshot.activeBranchId;
  state.historyCursorMode = snapshot.historyCursorMode;
  state.historyNodes = cloneHistoryNodes(snapshot.historyNodes);
  state.historyBranches = cloneHistoryBranches(snapshot.historyBranches);
  state.messages = cloneMessages(snapshot.messages);
  state.attachmentAssets = cloneAttachmentAssets(snapshot.attachmentAssets);
  state.toolActivities = cloneToolActivities(snapshot.toolActivities);
  state.traceSteps = cloneTraceSteps(snapshot.traceSteps);
  state.traceTimeline = cloneTraceTimeline(snapshot.traceTimeline);
  state.turnTraceHistory = snapshot.turnTraceHistory.map((trace) => normalizeTurnTraceRecord(trace));
  state.eventCursorByTurnId = Object.fromEntries(
    Object.entries(snapshot.eventCursorByTurnId ?? {}).map(([turnId, cursor]) => [turnId, { ...cursor }])
  );
  state.streamFlushFrameId = null;
  state.streamBufferTurnId = null;
  state.streamBufferText = "";
  state.streamBufferReasoning = "";
  state.streamDebugDeltaCount = 0;
  state.streamDebugFlushCount = 0;
  state.streamDebugLastDeltaAtMs = null;
  state.streamDebugLastFlushAtMs = null;
  state.streamDebugReasoningCharsReceived = 0;
  state.streamDebugReasoningCharsFlushed = 0;
  state.streamDebugTextCharsReceived = 0;
  state.streamDebugTextCharsFlushed = 0;
}

function buildEventCursorByTurnTraceHistory(turnTraceHistory: TurnTraceRecord[]) {
  const entries = turnTraceHistory
    .filter((trace) => trace.turnId)
    .map((trace) => [
      trace.turnId,
      {
        eventId: trace.eventId ?? null,
        sequence: trace.sequence ?? null,
        emittedAtMs: trace.emittedAtMs ?? null
      }
    ] as const);

  return Object.fromEntries(entries);
}

function shouldAcceptTurnEvent(
  currentCursor: { eventId: string | null; sequence: number | null; emittedAtMs: number | null } | null | undefined,
  payload: Pick<TurnStreamEvent, "eventId" | "sequence" | "emittedAtMs">
) {
  const nextEventId = payload.eventId?.trim() || null;
  const nextSequence = typeof payload.sequence === "number" && Number.isFinite(payload.sequence) ? payload.sequence : null;
  const nextEmittedAtMs =
    typeof payload.emittedAtMs === "number" && Number.isFinite(payload.emittedAtMs) ? payload.emittedAtMs : null;
  if (!currentCursor) {
    return true;
  }

  if (nextEventId && currentCursor.eventId && nextEventId === currentCursor.eventId) {
    return false;
  }

  if (nextSequence != null && currentCursor.sequence != null) {
    if (nextSequence < currentCursor.sequence) {
      return false;
    }

    if (nextSequence === currentCursor.sequence) {
      if (!nextEventId || !currentCursor.eventId || nextEventId !== currentCursor.eventId) {
        return false;
      }
    }
  }

  if (
    nextSequence == null &&
    nextEmittedAtMs != null &&
    currentCursor.sequence == null &&
    currentCursor.emittedAtMs != null &&
    nextEmittedAtMs < currentCursor.emittedAtMs
  ) {
    return false;
  }

  return true;
}

const defaultAvailableTools: AvailableTool[] = [
  {
    name: "time.now",
    description: "返回当前本机 UNIX 时间戳，适合最小时间查询与运行时校验。",
    inputSchema: {
      type: "object",
      properties: {},
      additionalProperties: false
    }
  },
  {
    name: "echo.input",
    description: "把传入的 text 原样返回，适合验证工具调用链路与参数透传。",
    inputSchema: {
      type: "object",
      properties: {
        text: {
          type: "string",
          description: "需要原样回显的文本"
        }
      },
      required: ["text"],
      additionalProperties: false
    }
  },
  {
    name: "workspace.read_file",
    description: "读取当前工作区内的文本文件全文预览，需要传入相对路径。",
    inputSchema: {
      type: "object",
      properties: {
        path: {
          type: "string",
          description: "当前工作区内的相对文件路径"
        }
      },
      required: ["path"],
      additionalProperties: false
    }
  },
  {
    name: "workspace.read_file_segment",
    description: "按行读取当前工作区文件片段，更适合大文件局部排查与定点观察。",
    inputSchema: {
      type: "object",
      properties: {
        path: {
          type: "string",
          description: "当前工作区内的相对文件路径"
        },
        startLine: {
          type: "integer",
          description: "从第几行开始读取，最小值为 1"
        },
        lineCount: {
          type: "integer",
          description: "读取多少行，默认 40"
        }
      },
      required: ["path"],
      additionalProperties: false
    }
  },
  {
    name: "workspace.list_files",
    description: "列出当前工作区目录中的文件与子目录，可指定相对路径和返回条数。",
    inputSchema: {
      type: "object",
      properties: {
        path: {
          type: "string",
          description: "当前工作区内的相对目录路径，默认 ."
        },
        limit: {
          type: "integer",
          description: "最多返回多少个条目，默认 40"
        }
      },
      additionalProperties: false
    }
  }
];

function createAvailableTools() {
  return defaultAvailableTools.map((tool) => ({
    ...tool,
    inputSchema: {
      ...tool.inputSchema,
      properties: tool.inputSchema.properties ? { ...tool.inputSchema.properties } : {}
    }
  }));
}

const defaultCapabilitySources: CapabilitySourceView[] = [
  {
    sourceId: "builtin-tools",
    sourceKind: "builtin",
    displayName: "Builtin Tools",
    transportKind: "in_process",
    serverIdentity: "pony-agent:builtin-tools",
    availability: "available",
    declaredCapabilities: ["tool"],
    permissionProfile: "host-mediated",
    updatedAtMs: 0,
    lastIngressObservation: null
  }
];

function createCapabilitySources() {
  return defaultCapabilitySources.map((source) => ({
    ...source,
    declaredCapabilities: [...source.declaredCapabilities]
  }));
}

function canonicalizeBuiltinCapabilityName(toolName: string) {
  return toolName.replace(/\./g, "_");
}

function createCapabilities() {
  return defaultAvailableTools.map((tool): CapabilityView => ({
    capabilityId: `builtin:${canonicalizeBuiltinCapabilityName(tool.name)}`,
    sourceId: "builtin-tools",
    sourceKind: "builtin",
    kind: "tool",
    label: canonicalizeBuiltinCapabilityName(tool.name),
    description: tool.description,
    invocationMode: "direct_tool_call",
    inputSchemaSummary: tool.inputSchema.type ?? "object",
    safetyClass: "host_tool",
    visibility: "default",
    observabilityTags: ["builtin", "tool"],
    requiresApproval: false,
    hostMediated: true,
    permissionScope: "workspace"
  }));
}

function buildTurnHistory(messages: ChatMessage[]): TurnHistoryMessage[] {
  return messages
    .filter(
      (message) =>
        (message.role === "user" || message.role === "assistant") &&
        message.status !== "pending" &&
        message.content.trim().length > 0
    )
    .slice(-8)
    .map((message) => ({
      role: message.role === "user" ? "user" : "assistant",
      content: message.content,
      attachments: (message.attachments ?? [])
        .filter(
          (attachment) =>
            typeof attachment.relativePath === "string" && attachment.relativePath.trim().length > 0
        )
        .map((attachment) => ({ ...attachment }))
    }));
}

function createSnapshotFromRuntimeState(state: RuntimeState, sessionId: string): SessionSnapshot {
  const retrievedSummary = state.retrievedContext?.sessionContext?.summary?.trim() ?? "";
  return {
    conversationId: sessionId,
    title: buildSessionTitleFromMessages(state.messages),
    summary: retrievedSummary || state.sessionSummary || DEFAULT_BROWSER_SESSION_SUMMARY,
    history: buildTurnHistory(state.messages),
    attachmentAssets: cloneAttachmentAssets(state.attachmentAssets),
    turnTraceHistory: state.turnTraceHistory.map((trace) => normalizeTurnTraceRecord(trace)),
    historyStateEvidence: [],
    historyStateAuditSummary: null,
    turnCount: state.messages.filter((message) => message.role === "user").length,
    lastReferencedFile: null,
    updatedAtMs:
      state.turnTraceHistory.length > 0
        ? state.turnTraceHistory[state.turnTraceHistory.length - 1].updatedAt
        : Date.now()
  };
}

function deriveRetrievedContextFromSnapshot(snapshot: SessionSnapshot): RetrievedContextState {
  const recentHistory = snapshot.history.slice(-12).map((message) => ({
    ...message,
    attachments: (message.attachments ?? []).map((attachment) => ({ ...attachment }))
  }));
  const recentAttachmentAssets = cloneAttachmentAssets(snapshot.attachmentAssets ?? []).slice(-8);
  const lastUserMessage =
    [...snapshot.history].reverse().find((message) => message.role === "user")?.content ?? "";

  return {
    turnContext: {
      userMessage: lastUserMessage,
      images: [],
      referencesImage: false
    },
    sessionContext: {
      conversationId: snapshot.conversationId,
      title: snapshot.title ?? "新对话",
      summary: snapshot.summary,
      recentHistory,
      recentAttachmentAssets,
      turnCount: snapshot.turnCount,
      lastReferencedFile: snapshot.lastReferencedFile ?? null
    },
    runState: {},
    longTermMemory: {
      status: "empty",
      summary: RETRIEVED_CONTEXT_FALLBACK_SUMMARY,
      entries: []
    },
    transcript: {
      providerNativeMessages: []
    }
  };
}

function isGraphTerminalPhase(phase?: string | null) {
  return ["completed", "failed", "cancelled"].includes((phase ?? "").trim().toLowerCase());
}

function normalizeRuntimePhaseValue(phase?: string | null): RuntimePhase | null {
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

function mapLifecyclePhaseToRuntimePhase(phase?: string | null): RuntimePhase | null {
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

function resolveRuntimePhaseFromEvent(
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

function resolveFallbackTimelineRuntimePhase(
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

function restorePhaseFromTurnHistory(
  messages: ChatMessage[],
  turnTraceHistory: TurnTraceRecord[]
): RuntimePhase {
  if (messages.length === 0) {
    return "idle";
  }

  const latestTurnPhase = normalizeRuntimePhaseValue(turnTraceHistory[turnTraceHistory.length - 1]?.phase);
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

function createBrowserPreviewTerminalEnvelope(
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

function resolveRestoredPersistedPhase(
  persistedPhase: RuntimePhase | null | undefined,
  messages: ChatMessage[],
  turnTraceHistory: TurnTraceRecord[]
): RuntimePhase {
  const restoredPhase = restorePhaseFromTurnHistory(messages, turnTraceHistory);
  if (restoredPhase !== "ready") {
    return restoredPhase;
  }

  if (persistedPhase === "cancelled" || persistedPhase === "failed") {
    return persistedPhase;
  }

  return restoredPhase;
}

function resolveGraphRunSubmissionFromRunState(runState?: RunState | null) {
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

type GraphRunSubmission =
  | { command: "start_graph_run_stream"; runId: null }
  | { command: "resume_graph_run_stream"; runId: string }
  | { command: "continue_graph_run_stream"; runId: string };

function resolveGraphRunSubmissionFromPlan(
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

function resolveGraphRunSubmissionFromCheckpoint(
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

function reconcileSubmissionWithRecoveryCheckpoint(
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

function normalizeCheckpointPhase(checkpoint: ExecutionCheckpoint): RuntimePhase {
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

function loadPersistedRuntimeCache(): PersistedRuntimeCache {
  if (typeof window === "undefined") {
    return { sessions: {} };
  }

  try {
    const raw = window.localStorage.getItem(RUNTIME_STORAGE_KEY);
    if (!raw) {
      debugLog("restore:empty");
      return { sessions: {} };
    }

    const parsed = JSON.parse(raw) as PersistedRuntimeCache;
    debugLog("restore:ok", {
      sessions: Object.keys(parsed.sessions ?? {}).length
    });
    return {
      sessions: parsed.sessions ?? {}
    };
  } catch {
    debugLog("restore:error");
    return { sessions: {} };
  }
}

function loadPersistedRuntimeState(sessionId: string): PersistedRuntimeState | null {
  return loadPersistedRuntimeCache().sessions[sessionId] ?? null;
}

function persistSessionState(sessionId: string, payload: PersistedRuntimeState) {
  if (typeof window === "undefined") {
    return;
  }

  const cache = loadPersistedRuntimeCache();
  cache.sessions[sessionId] = payload;
  window.localStorage.setItem(RUNTIME_STORAGE_KEY, JSON.stringify(cache));
}

function removePersistedSessionState(sessionId: string) {
  if (typeof window === "undefined") {
    return;
  }

  const cache = loadPersistedRuntimeCache();
  delete cache.sessions[sessionId];
  window.localStorage.setItem(RUNTIME_STORAGE_KEY, JSON.stringify(cache));
}

function createHistoryTurnId(index: number) {
  return `history-turn-${index + 1}`;
}

function hasPersistableMessages(messages: ChatMessage[]) {
  return messages.some(
    (message) =>
      (message.role === "user" || message.role === "assistant") && message.content.trim().length > 0
  );
}

function createTransientSessionOverview(sessionId: string): SessionOverview {
  return {
    conversationId: sessionId,
    title: "新对话",
    summary: "发送第一条消息后保存到历史",
    turnCount: 0,
    lastReferencedFile: null,
    updatedAtMs: 0
  };
}

function buildSessionOverviewFromPersistedState(
  conversationId: string,
  state: PersistedRuntimeState
): SessionOverview {
  return {
    conversationId,
    title: buildSessionTitleFromMessages(state.messages),
    summary: state.sessionSummary || DEFAULT_BROWSER_SESSION_SUMMARY,
    turnCount: state.messages.filter((message) => message.role === "user").length,
    lastReferencedFile: null,
    updatedAtMs:
      state.turnTraceHistory.length > 0
        ? state.turnTraceHistory[state.turnTraceHistory.length - 1].updatedAt
        : 0
  };
}

function buildSessionTitleFromMessages(messages: ChatMessage[]) {
  const firstUserMessage = messages.find((message) => message.role === "user");
  return firstUserMessage ? buildTurnTitle(firstUserMessage.content) : "新对话";
}

function isPersistedStateCompatible(
  snapshot: SessionSnapshot,
  persisted: PersistedRuntimeState | null
) {
  if (!persisted) {
    return false;
  }

  const persistedHistory = buildTurnHistory(persisted.messages);
  if (persistedHistory.length !== snapshot.history.length) {
    return false;
  }

  return persistedHistory.every((message, index) => {
    const snapshotMessage = snapshot.history[index];
    return (
      message.role === snapshotMessage?.role &&
      message.content.trim() === snapshotMessage?.content.trim()
    );
  });
}

function collectPersistedHistoryMessages(messages?: ChatMessage[] | null) {
  return (messages ?? []).filter(
    (message) =>
      (message.role === "user" || message.role === "assistant") &&
      message.status !== "pending" &&
      message.content.trim().length > 0
  );
}

function isPersistedMessageShapeCompatible(
  snapshot: SessionSnapshot,
  persisted: PersistedRuntimeState | null
) {
  if (!persisted) {
    return false;
  }

  const persistedHistory = collectPersistedHistoryMessages(persisted.messages);
  if (persistedHistory.length !== snapshot.history.length) {
    return false;
  }

  return persistedHistory.every((message, index) => message.role === snapshot.history[index]?.role);
}

function traceReasoningContent(trace?: TurnTraceRecord | null) {
  if (!trace?.traceTimeline?.length) {
    return null;
  }

  for (const entry of [...trace.traceTimeline].reverse()) {
    if (canonicalizeTraceTimelineKind(entry.kind) !== "call_model") {
      continue;
    }

    const reasoningContent = normalizeReasoningContent(entry.reasoningContent ?? null);
    if (reasoningContent) {
      return reasoningContent;
    }
  }

  return null;
}

function traceModelLabel(trace?: TurnTraceRecord | null) {
  const topLevelLabel = buildAssistantModelLabel(trace?.providerName, trace?.providerModel);
  if (topLevelLabel) {
    return topLevelLabel;
  }

  if (!trace?.traceTimeline?.length) {
    return null;
  }

  for (const entry of [...trace.traceTimeline].reverse()) {
    if (canonicalizeTraceTimelineKind(entry.kind) !== "call_model") {
      continue;
    }

    const modelLabel = buildAssistantModelLabel(entry.providerName, entry.providerModel);
    if (modelLabel) {
      return modelLabel;
    }
  }

  return null;
}

function traceToolActivities(trace?: TurnTraceRecord | null) {
  if (!trace) {
    return [];
  }

  const activities = trace.toolActivities.filter((tool) => tool.status !== "planned");
  if (activities.length) {
    return activities;
  }

  const deduped = new Map<string, ToolActivity>();
  for (const entry of trace.traceTimeline ?? []) {
    if (canonicalizeTraceTimelineKind(entry.kind) !== "call_tool") {
      continue;
    }

    for (const activity of entry.toolActivities ?? []) {
      if (activity.status === "planned") {
        continue;
      }
      deduped.set(activity.id, { ...activity });
    }
  }

  return [...deduped.values()];
}

function buildToolMessagesFromTrace(trace: TurnTraceRecord | null | undefined, turnId: string): ChatMessage[] {
  const activities = traceToolActivities(trace);
  if (!activities.length) {
    return [];
  }

  return activities.map((tool) => ({
    id: `tool-${turnId}-${tool.id}`,
    turnId,
    role: "tool",
    content: tool.resultText ?? "",
    status: toolStatusToMessageStatus(tool.status),
    toolName: tool.name,
    detail: buildToolMessageDetail(tool),
    durationSeconds: tool.durationSeconds ?? null
  }));
}

function hydrateMessagesFromHistory(
  history: TurnHistoryMessage[],
  persistedMessages?: ChatMessage[] | null,
  turnTraceHistory?: TurnTraceRecord[] | null
): ChatMessage[] {
  const messages: ChatMessage[] = [];
  const restoredHistoryMessages = collectPersistedHistoryMessages(persistedMessages);
  const toolMessagesByTurnId = new Map<string, ChatMessage[]>();
  const orderedTurnTraceHistory = [...(turnTraceHistory ?? [])].sort((left, right) => {
    const updatedAtDiff = (left.updatedAt ?? 0) - (right.updatedAt ?? 0);
    if (updatedAtDiff !== 0) {
      return updatedAtDiff;
    }

    return left.turnId.localeCompare(right.turnId);
  });
  let currentTurnId: string | null = null;
  let currentTrace: TurnTraceRecord | null = null;
  let traceIndex = 0;
  let turnIndex = 0;
  let restoredHistoryIndex = 0;

  for (const message of persistedMessages ?? []) {
    if (message.role !== "tool") {
      continue;
    }

    const turnMessages = toolMessagesByTurnId.get(message.turnId) ?? [];
    turnMessages.push({ ...message });
    toolMessagesByTurnId.set(message.turnId, turnMessages);
  }

  const appendToolMessagesForTurn = (turnId: string | null, trace?: TurnTraceRecord | null) => {
    if (!turnId) {
      return;
    }

    const toolMessages = toolMessagesByTurnId.get(turnId);
    if (toolMessages?.length) {
      messages.push(...toolMessages.map((message) => ({ ...message })));
      toolMessagesByTurnId.delete(turnId);
      return;
    }

    messages.push(...buildToolMessagesFromTrace(trace, turnId));
  };

  for (const item of history) {
    const restoredMessage = restoredHistoryMessages[restoredHistoryIndex];

    if (item.role === "user") {
      currentTrace = orderedTurnTraceHistory[traceIndex] ?? null;
      currentTurnId = restoredMessage?.turnId ?? currentTrace?.turnId ?? createHistoryTurnId(turnIndex);
      turnIndex += 1;
      restoredHistoryIndex += 1;
      messages.push({
        id: restoredMessage?.id ?? `history-user-${turnIndex}`,
        turnId: currentTurnId,
        role: "user",
        content: item.content,
        attachments: item.attachments ?? [],
        status: "done",
        tokenCount: restoredMessage?.tokenCount ?? null
      });
      continue;
    }

    currentTrace = currentTrace ?? orderedTurnTraceHistory[traceIndex] ?? null;
    if (!currentTurnId) {
      currentTurnId = restoredMessage?.turnId ?? currentTrace?.turnId ?? createHistoryTurnId(turnIndex);
      turnIndex += 1;
    }

    restoredHistoryIndex += 1;
    messages.push({
      id: restoredMessage?.id ?? `history-assistant-${turnIndex}`,
      turnId: currentTurnId,
      role: "assistant",
      content: item.content,
      attachments: [],
      status: "done",
      reasoningContent: restoredMessage?.reasoningContent ?? traceReasoningContent(currentTrace),
      tokenCount: restoredMessage?.tokenCount ?? currentTrace?.outputTokens ?? null,
      modelName: restoredMessage?.modelName ?? traceModelLabel(currentTrace)
    });
    appendToolMessagesForTurn(currentTurnId, currentTrace);
    currentTurnId = null;
    currentTrace = null;
    traceIndex += 1;
  }

  appendToolMessagesForTurn(currentTurnId, currentTrace);
  return messages;
}

export const useRuntimeStore = defineStore("runtime", {
  state: (): RuntimeState => {
    const persisted = loadPersistedRuntimeState(DEFAULT_SESSION_ID);

    return {
      sessionId: DEFAULT_SESSION_ID,
      sessionList: [],
      sessionOperation: null,
      sessionError: null,
      phase: resolveRestoredPersistedPhase(
        persisted?.phase,
        persisted?.messages ?? [],
        (persisted?.turnTraceHistory ?? []).map((trace) => normalizeTurnTraceRecord(trace))
      ),
      health: null,
      error: null,
      draftMessage: "",
      sessionSummary: persisted?.sessionSummary ?? "",
      retrievedContext: null,
      providerRequestedName: persisted?.providerRequestedName ?? "",
      providerName: persisted?.providerName ?? "",
      providerProtocol: persisted?.providerProtocol ?? "",
      providerModel: persisted?.providerModel ?? "",
      providerSource: persisted?.providerSource ?? "",
      providerMode: persisted?.providerMode ?? "",
      fallbackReason: persisted?.fallbackReason ?? null,
      inputTokens: persisted?.inputTokens ?? null,
      outputTokens: persisted?.outputTokens ?? null,
      totalTokens: persisted?.totalTokens ?? null,
      firstTokenLatencyMs: persisted?.firstTokenLatencyMs ?? null,
      isSubmitting: false,
      activeTurnId: null,
      activeRunId: null,
      latestExecutionCheckpoint: null,
      latestGraphRunSubmissionPlan: null,
      latestGraphRunControlBoundaryEvidence: [],
      latestRunControlAuditSummary: null,
      latestHistoryStateAuditSummary: null,
      visibleNodeId: null,
      branchHeadNodeId: null,
      activeBranchId: null,
      historyCursorMode: "live",
      historyNodes: [],
      historyBranches: [],
      eventsReady: false,
      deferredPersistTimerId: null,
      streamFlushFrameId: null,
      streamBufferTurnId: null,
      streamBufferText: "",
      streamBufferReasoning: "",
      streamDebugDeltaCount: 0,
      streamDebugFlushCount: 0,
      streamDebugLastDeltaAtMs: null,
      streamDebugLastFlushAtMs: null,
      streamDebugReasoningCharsReceived: 0,
      streamDebugReasoningCharsFlushed: 0,
      streamDebugTextCharsReceived: 0,
      streamDebugTextCharsFlushed: 0,
      browserPreviewRunToken: 0,
      messages: persisted?.messages ?? [],
      attachmentAssets: persisted?.attachmentAssets ?? [],
      availableTools: createAvailableTools(),
      capabilitySources: createCapabilitySources(),
      capabilities: createCapabilities(),
      toolActivities: [],
      traceSteps: createDefaultTraceSteps(),
      traceTimeline: createDefaultTraceTimeline(),
      turnTraceHistory: (persisted?.turnTraceHistory ?? []).map((trace) => normalizeTurnTraceRecord(trace)),
      eventCursorByTurnId: buildEventCursorByTurnTraceHistory(
        (persisted?.turnTraceHistory ?? []).map((trace) => normalizeTurnTraceRecord(trace))
      )
    };
  },
  getters: {
    phaseLabel(state): string {
      const labels: Record<string, string> = {
        idle: "空闲",
        connecting: "连接中",
        ready: "已就绪",
        completed: "本轮完成",
        cancelled: "已停止",
        calling_model: "模型处理中",
        calling_tool: "工具处理中",
        failed: "失败"
      };

      return labels[state.phase] ?? state.phase;
    },
    isHistoricalMode(state): boolean {
      return state.historyCursorMode !== "live";
    },
    conversationCheckpointEntries(state): ConversationCheckpointEntry[] {
      return buildConversationCheckpointEntries(
        state.historyNodes,
        state.historyBranches,
        state.activeBranchId,
        state.visibleNodeId,
        state.branchHeadNodeId
      );
    }
  },
  actions: {
    resetSessionRuntimeState() {
      this.cancelDeferredPersist();
      const blankFields = createBlankSessionRuntimeFields();
      this.phase = "idle";
      this.error = null;
      this.draftMessage = "";
      this.sessionSummary = blankFields.sessionSummary;
      this.retrievedContext = null;
      this.providerRequestedName = blankFields.providerRequestedName;
      this.providerName = blankFields.providerName;
      this.providerProtocol = blankFields.providerProtocol;
      this.providerModel = blankFields.providerModel;
      this.providerSource = blankFields.providerSource;
      this.providerMode = blankFields.providerMode;
      this.fallbackReason = blankFields.fallbackReason;
      this.inputTokens = blankFields.inputTokens;
      this.outputTokens = blankFields.outputTokens;
      this.totalTokens = blankFields.totalTokens;
      this.firstTokenLatencyMs = blankFields.firstTokenLatencyMs;
      this.isSubmitting = false;
      this.activeTurnId = null;
      this.activeRunId = null;
      this.latestGraphRunSubmissionPlan = null;
      this.latestGraphRunControlBoundaryEvidence = [];
      this.latestHistoryStateAuditSummary = null;
      this.visibleNodeId = null;
      this.branchHeadNodeId = null;
      this.activeBranchId = null;
      this.historyCursorMode = "live";
      this.historyNodes = [];
      this.historyBranches = [];
      this.messages = [];
      this.attachmentAssets = [];
      this.toolActivities = [];
      this.traceSteps = createDefaultTraceSteps();
      this.traceTimeline = createDefaultTraceTimeline();
      this.turnTraceHistory = [];
      this.eventCursorByTurnId = {};
      this.cancelStreamFlush();
      this.streamBufferTurnId = null;
      this.streamBufferText = "";
      this.streamBufferReasoning = "";
      this.streamDebugDeltaCount = 0;
      this.streamDebugFlushCount = 0;
      this.streamDebugLastDeltaAtMs = null;
      this.streamDebugLastFlushAtMs = null;
      this.streamDebugReasoningCharsReceived = 0;
      this.streamDebugReasoningCharsFlushed = 0;
      this.streamDebugTextCharsReceived = 0;
      this.streamDebugTextCharsFlushed = 0;
      this.browserPreviewRunToken = 0;
    },
    cancelDeferredPersist() {
      if (this.deferredPersistTimerId == null) {
        return;
      }

      window.clearTimeout(this.deferredPersistTimerId);
      this.deferredPersistTimerId = null;
    },
    cancelStreamFlush() {
      if (this.streamFlushFrameId == null) {
        return;
      }

      window.cancelAnimationFrame(this.streamFlushFrameId);
      this.streamFlushFrameId = null;
    },
    resetStreamDebugMetrics() {
      this.streamDebugDeltaCount = 0;
      this.streamDebugFlushCount = 0;
      this.streamDebugLastDeltaAtMs = null;
      this.streamDebugLastFlushAtMs = null;
      this.streamDebugReasoningCharsReceived = 0;
      this.streamDebugReasoningCharsFlushed = 0;
      this.streamDebugTextCharsReceived = 0;
      this.streamDebugTextCharsFlushed = 0;
    },
    flushBufferedStreamText(turnId?: string | null) {
      const bufferedTurnId = this.streamBufferTurnId;
      if (!bufferedTurnId) {
        return;
      }
      if (turnId && bufferedTurnId !== turnId) {
        return;
      }

      this.cancelStreamFlush();
      const flushStartedAt = typeof performance !== "undefined" ? performance.now() : Date.now();
      const bufferedTextLength = this.streamBufferText.length;
      const bufferedReasoningLength = this.streamBufferReasoning.length;

      const assistantMessage = this.ensureAssistantMessage(
        bufferedTurnId,
        buildAssistantModelLabel(this.providerName, this.providerModel)
      );

      if (this.streamBufferReasoning) {
        assistantMessage.reasoningContent = appendNormalizedReasoningContent(
          assistantMessage.reasoningContent ?? null,
          this.streamBufferReasoning
        );
      }

      if (this.streamBufferText) {
        assistantMessage.content += this.streamBufferText;
      }

      this.streamDebugFlushCount += 1;
      this.streamDebugLastFlushAtMs = Date.now();
      this.streamDebugTextCharsFlushed += bufferedTextLength;
      this.streamDebugReasoningCharsFlushed += bufferedReasoningLength;
      updateStreamDebugBucket("runtime", {
        turnId: bufferedTurnId,
        deltaCount: this.streamDebugDeltaCount,
        flushCount: this.streamDebugFlushCount,
        textCharsReceived: this.streamDebugTextCharsReceived,
        textCharsFlushed: this.streamDebugTextCharsFlushed,
        reasoningCharsReceived: this.streamDebugReasoningCharsReceived,
        reasoningCharsFlushed: this.streamDebugReasoningCharsFlushed,
        bufferedTextLength,
        bufferedReasoningLength,
        pendingRaf: this.streamFlushFrameId != null,
        flushDurationMs:
          (typeof performance !== "undefined" ? performance.now() : Date.now()) - flushStartedAt
      });

      this.streamBufferTurnId = null;
      this.streamBufferText = "";
      this.streamBufferReasoning = "";
    },
    scheduleStreamFlush(turnId: string) {
      if (this.streamBufferTurnId && this.streamBufferTurnId !== turnId) {
        this.flushBufferedStreamText(this.streamBufferTurnId);
      }

      this.streamBufferTurnId = turnId;
      const bufferedLength = this.streamBufferText.length + this.streamBufferReasoning.length;
      if (bufferedLength >= STREAM_FLUSH_EAGER_CHARS) {
        this.flushBufferedStreamText(turnId);
        return;
      }

      if (this.streamFlushFrameId != null) {
        return;
      }

      this.streamFlushFrameId = window.requestAnimationFrame(() => {
        this.streamFlushFrameId = null;
        this.flushBufferedStreamText(turnId);
      });
    },
    cancelBrowserPreviewTurn(turnId: string) {
      if (this.activeTurnId !== turnId) {
        return false;
      }

      this.browserPreviewRunToken += 1;
      const cancelledTraceSteps = finalizeCancelledTraceSteps(this.traceSteps);
      const assistantMessage = this.ensureAssistantMessage(
        turnId,
        buildAssistantModelLabel(this.providerName, this.providerModel)
      );
      assistantMessage.content = "本轮已停止。";
      assistantMessage.reasoningContent = null;
      assistantMessage.status = "done";

      const traceTimeline: TraceTimelineEntry[] = createBrowserPreviewTraceTimeline().map((entry) =>
        entry.kind === "call_model"
          ? {
              ...entry,
              state: "cancelled" as const,
              text: assistantMessage.content,
              fallbackReason: this.fallbackReason
            }
          : entry
      );

      this.phase = "cancelled";
      this.error = null;
      this.traceSteps = cancelledTraceSteps;
      this.traceTimeline = traceTimeline;
      this.toolActivities = [];
      const terminalSequence = traceTimeline[traceTimeline.length - 1]?.sequence ?? cancelledTraceSteps.length;
      const terminalEnvelope = createBrowserPreviewTerminalEnvelope(
        turnId,
        "turn.cancelled",
        terminalSequence,
        Date.now()
      );
      this.commitTurnEventCursor({
        turnId,
        eventId: terminalEnvelope.eventId,
        sequence: terminalEnvelope.sequence,
        emittedAtMs: terminalEnvelope.emittedAtMs
      });
      this.commitTurnTraceTimeline(turnId, traceTimeline, {
        eventId: terminalEnvelope.eventId,
        eventType: terminalEnvelope.eventType,
        eventVersion: terminalEnvelope.eventVersion,
        sequence: terminalEnvelope.sequence,
        emittedAtMs: terminalEnvelope.emittedAtMs,
        phase: "cancelled",
        traceSteps: cancelledTraceSteps,
        toolActivities: [],
        sessionSummary: this.sessionSummary,
        fallbackReason: this.fallbackReason,
        title:
          this.messages.find((item) => item.turnId === turnId && item.role === "user")?.content
            ? buildTurnTitle(this.messages.find((item) => item.turnId === turnId && item.role === "user")?.content ?? "")
            : BROWSER_PREVIEW_TRACE_TITLE,
        error: "stopped_by_user"
      });
      this.persistHistory();
      void this.loadSessionCatalog();
      this.isSubmitting = false;
      this.activeTurnId = null;
      this.activeRunId = null;
      debugLog("browser-preview:cancelled", {
        turnId
      });
      return true;
    },
    scheduleDeferredPersist(delay = 140) {
      this.cancelDeferredPersist();
      this.deferredPersistTimerId = window.setTimeout(() => {
        this.deferredPersistTimerId = null;
        this.persistHistory();
      }, delay);
    },
    persistHistory() {
      this.cancelDeferredPersist();
      if (!hasPersistableMessages(this.messages)) {
        removePersistedSessionState(this.sessionId);
        debugLog("persist:skip-empty", {
          sessionId: this.sessionId
        });
        return;
      }

      const payload: PersistedRuntimeState = {
        phase: this.phase,
        messages: this.messages,
        attachmentAssets: this.attachmentAssets,
        turnTraceHistory: this.turnTraceHistory,
        sessionSummary: this.sessionSummary,
        providerRequestedName: this.providerRequestedName,
        providerName: this.providerName,
        providerProtocol: this.providerProtocol,
        providerModel: this.providerModel,
        providerSource: this.providerSource,
        providerMode: this.providerMode,
        fallbackReason: this.fallbackReason,
        inputTokens: this.inputTokens,
        outputTokens: this.outputTokens,
        totalTokens: this.totalTokens,
        firstTokenLatencyMs: this.firstTokenLatencyMs
      };

      try {
        persistSessionState(this.sessionId, payload);
        debugLog("persist", {
          sessionId: this.sessionId,
          messages: this.messages.length,
          traces: this.turnTraceHistory.length,
          phase: this.phase
        });
      } catch {
        // Ignore storage failures and keep runtime in memory.
        debugLog("persist:error");
      }
    },
    getAttachmentAssets(filter?: AttachmentAssetFilter | null) {
      return filterAttachmentAssets(this.attachmentAssets, filter);
    },
    applyHistoryState(
      _sessionId: string,
      payload?:
        | (Partial<HistoryCursorState> & {
            historyNodes?: HistoryNode[] | null;
            historyBranches?: HistoryBranch[] | null;
          })
        | null
    ) {
      const historyNodes = cloneHistoryNodes(payload?.historyNodes);
      const historyBranches = cloneHistoryBranches(payload?.historyBranches);
      const activeBranchId = payload?.activeBranchId?.trim() || null;
      const explicitHeadNodeId = payload?.branchHeadNodeId?.trim() || null;
      const branchHeadNodeId =
        explicitHeadNodeId || resolveHistoryBranchHeadNodeId(activeBranchId, historyBranches);
      const visibleNodeId = payload?.visibleNodeId?.trim() || null;

      this.historyNodes = historyNodes;
      this.historyBranches = historyBranches;
      this.activeBranchId = activeBranchId;
      this.branchHeadNodeId = branchHeadNodeId;
      this.visibleNodeId = visibleNodeId;
      this.historyCursorMode = normalizeHistoryCursorMode(
        payload?.mode ?? (visibleNodeId && branchHeadNodeId && visibleNodeId !== branchHeadNodeId ? "historical" : "live")
      );
    },
    async loadSessionCatalog() {
      if (isTauriAvailable()) {
        this.sessionList = await safeInvoke<SessionOverview[]>("list_sessions");
        return;
      }

      const cache = loadPersistedRuntimeCache();
      this.sessionList = Object.entries(cache.sessions)
        .map(([conversationId, state]) => buildSessionOverviewFromPersistedState(conversationId, state))
        .sort((left, right) => right.updatedAtMs - left.updatedAtMs);
    },
    async loadSessionRuntimeViewState(sessionId: string, nodeId?: string | null) {
      if (isTauriAvailable()) {
        const payload: Record<string, unknown> = {
          turnId: null,
          sessionId,
          runId: null
        };
        if (nodeId) {
          payload.nodeId = nodeId;
        }
        return await safeInvoke<SessionRuntimeView>("load_session_runtime_view", {
          ...payload
        });
      }

      const persisted = loadPersistedRuntimeState(sessionId);
      const snapshot = {
        conversationId: sessionId,
        title: buildSessionTitleFromMessages(persisted?.messages ?? []),
        summary: persisted?.sessionSummary ?? (persisted?.messages?.length ? DEFAULT_BROWSER_SESSION_SUMMARY : ""),
        history: buildTurnHistory(persisted?.messages ?? []),
        attachmentAssets: persisted?.attachmentAssets ?? [],
        turnCount: persisted?.messages?.filter((message) => message.role === "user").length ?? 0,
        historyStateEvidence: [],
        historyStateAuditSummary: null,
        runControlAuditSummary: null,
        lastReferencedFile: null,
        updatedAtMs:
          persisted && persisted.turnTraceHistory.length > 0
            ? persisted.turnTraceHistory[persisted.turnTraceHistory.length - 1].updatedAt
            : Date.now()
      } satisfies SessionSnapshot;

      return {
        session: snapshot,
        historyStateEvidence: snapshot.historyStateEvidence ?? [],
        historyStateAuditSummary: snapshot.historyStateAuditSummary ?? null,
        runControlAuditSummary: snapshot.runControlAuditSummary ?? null,
        retrieved: deriveRetrievedContextFromSnapshot(snapshot),
        checkpoint: null,
        submissionPlan: null,
        controlBoundaryEvidence: null,
        historyNodes: undefined,
        historyBranches: undefined,
        historyCursor: null
      } satisfies SessionRuntimeView;
    },
    applyExecutionCheckpoint(
      checkpoint: ExecutionCheckpoint | null,
      persistedMessages?: ChatMessage[] | null
    ) {
      this.latestExecutionCheckpoint = checkpoint ? { ...checkpoint } : null;
      if (!checkpoint) {
        return;
      }

      const checkpointStatus = checkpoint.status.trim().toLowerCase();
      const isRecoveryCheckpoint = checkpoint.checkpointKind === "recovery";
      if (!isRecoveryCheckpoint && checkpointStatus !== "running") {
        return;
      }

      const restoredTurnMessages = (persistedMessages ?? [])
        .filter((message) => message.turnId === checkpoint.turnId)
        .map((message) => ({
          ...message,
          attachments: message.attachments?.map((attachment) => ({ ...attachment })) ?? []
        }));

      if (restoredTurnMessages.length > 0) {
        this.messages = [
          ...this.messages.filter((message) => message.turnId !== checkpoint.turnId),
          ...restoredTurnMessages
        ];
      }

      const modelLabel = buildAssistantModelLabel(checkpoint.providerName, checkpoint.providerModel);
      const assistantMessage =
        this.messages.find((message) => message.turnId === checkpoint.turnId && message.role === "assistant") ??
        this.ensureAssistantMessage(checkpoint.turnId, modelLabel);
      assistantMessage.status = isRecoveryCheckpoint ? "done" : "pending";
      assistantMessage.modelName = modelLabel;

      const checkpointRunId = checkpoint.runId?.trim() || null;
      if (checkpointRunId) {
        this.activeRunId = checkpointRunId;
      }
      this.phase = normalizeCheckpointPhase(checkpoint);
      this.error = checkpoint.error ?? null;
      this.isSubmitting = !isRecoveryCheckpoint;
      this.activeTurnId = isRecoveryCheckpoint ? null : checkpoint.turnId;
      this.providerRequestedName = checkpoint.providerRequestedName ?? this.providerRequestedName;
      this.providerName = checkpoint.providerName ?? this.providerName;
      this.providerProtocol = checkpoint.providerProtocol ?? this.providerProtocol;
      this.providerModel = checkpoint.providerModel ?? this.providerModel;
      this.providerSource = checkpoint.providerSource ?? this.providerSource;
      this.providerMode = checkpoint.providerMode ?? this.providerMode;
      this.fallbackReason = checkpoint.fallbackReason ?? this.fallbackReason;
      this.traceSteps = checkpoint.traceSteps.length > 0 ? checkpoint.traceSteps : this.traceSteps;
      this.toolActivities = checkpoint.toolActivities;
      const checkpointTimeline = cloneTraceTimeline(
        this.turnTraceHistory.find((trace) => trace.turnId === checkpoint.turnId)?.traceTimeline
      );
      this.traceTimeline = checkpointTimeline.length ? checkpointTimeline : createDefaultTraceTimeline();
      this.upsertTurnTrace(checkpoint.turnId, {
        phase: this.phase,
        traceSteps: this.traceSteps,
        traceTimeline: this.traceTimeline,
        toolActivities: this.toolActivities,
        providerRequestedName: this.providerRequestedName,
        providerName: this.providerName,
        providerProtocol: this.providerProtocol,
        providerModel: this.providerModel,
        providerSource: this.providerSource,
        providerMode: this.providerMode,
        fallbackReason: this.fallbackReason,
        error: checkpoint.error ?? null,
        updatedAt: checkpoint.updatedAtMs
      });
      debugLog("checkpoint:applied", {
        sessionId: this.sessionId,
        turnId: checkpoint.turnId,
        phase: this.phase,
        checkpointKind: checkpoint.checkpointKind
      });
    },
    async loadSessionState(
      nextSessionId: string,
      options?: {
        refreshCatalog?: boolean;
        executionCheckpoint?: ExecutionCheckpoint | null;
        runtimeView?: SessionRuntimeView | null;
        nodeId?: string | null;
      }
    ) {
      const refreshCatalog = options?.refreshCatalog ?? true;
      const runtimeView =
        options?.runtimeView ?? (await this.loadSessionRuntimeViewState(nextSessionId, options?.nodeId ?? null));
      const snapshot = runtimeView.session;
      const retrieved = runtimeView.retrieved;
      const persisted = loadPersistedRuntimeState(nextSessionId);
      const previousHistoryState = {
        activeBranchId: this.activeBranchId,
        branchHeadNodeId: this.branchHeadNodeId,
        historyNodes: this.historyNodes,
        historyBranches: this.historyBranches
      };
      const hasCheckpointOverride =
        options != null && Object.prototype.hasOwnProperty.call(options, "executionCheckpoint");
      this.applySessionSnapshot(nextSessionId, snapshot, retrieved, runtimeView);
      if (
        options?.nodeId &&
        !runtimeView.historyCursor &&
        !runtimeView.historyNodes?.length &&
        !runtimeView.historyBranches?.length
      ) {
        this.applyHistoryState(nextSessionId, {
          ...previousHistoryState,
          visibleNodeId: options.nodeId,
          mode:
            previousHistoryState.branchHeadNodeId &&
            previousHistoryState.branchHeadNodeId !== options.nodeId
              ? "historical"
              : "live"
        });
      }
      this.applyExecutionCheckpoint(
        hasCheckpointOverride
          ? (options?.executionCheckpoint ?? null)
          : (runtimeView.checkpoint ?? null),
        persisted?.messages
      );
      this.latestGraphRunSubmissionPlan = runtimeView.submissionPlan ? { ...runtimeView.submissionPlan } : null;
      this.latestGraphRunControlBoundaryEvidence = cloneGraphRunControlBoundaryEvidence(
        runtimeView.controlBoundaryEvidence
      );
      this.latestRunControlAuditSummary = cloneRunControlAuditSummary(
        runtimeView.runControlAuditSummary ?? snapshot.runControlAuditSummary ?? null
      );
      this.latestHistoryStateAuditSummary = cloneHistoryStateAuditSummary(
        runtimeView.historyStateAuditSummary ?? snapshot.historyStateAuditSummary ?? null
      );

      if (refreshCatalog) {
        await this.loadSessionCatalog();
      }
    },
    async loadRetrievedContextState(
      sessionId: string,
      options?: { runId?: string | null; snapshot?: SessionSnapshot | null; nodeId?: string | null }
    ) {
      const fallbackSnapshot = options?.snapshot ?? createSnapshotFromRuntimeState(this, sessionId);
      if (!isTauriAvailable()) {
        return deriveRetrievedContextFromSnapshot(fallbackSnapshot);
      }

      try {
        const payload: Record<string, unknown> = {
          sessionId,
          runId: options?.runId ?? null,
          turnId: null
        };
        if (options?.nodeId) {
          payload.nodeId = options.nodeId;
        }
        const retrieved = await safeInvoke<RetrievedContextState>("load_retrieved_context", payload);
        return cloneRetrievedContext(retrieved);
      } catch (error) {
        debugLog("retrieved-context:load:error", {
          sessionId,
          runId: options?.runId ?? null,
          error: String(error)
        });
        return deriveRetrievedContextFromSnapshot(fallbackSnapshot);
      }
    },
    async resolveDerivedSessionRun(options?: {
      sessionId?: string | null;
      runId?: string | null;
      preferRefresh?: boolean;
      snapshot?: SessionSnapshot | null;
      nodeId?: string | null;
    }): Promise<GraphRun | null> {
      const targetSessionId = options?.sessionId?.trim() || this.sessionId;
      if (!targetSessionId) {
        return null;
      }

      const deriveRun = (retrieved: RetrievedContextState | null | undefined) => {
        if (!retrieved) {
          return null;
        }

        return deriveGraphRunFromRunState(retrieved.runState, targetSessionId, Date.now(), {
          activeTaskFocus: extractActiveTaskFocus(retrieved.longTermMemory?.entries)
        });
      };

      if (!options?.preferRefresh) {
        const localRun = deriveRun(
          this.sessionId === targetSessionId ? this.retrievedContext : null
        );
        if (localRun) {
          return localRun;
        }
      }

      const refreshedRetrieved = await this.loadRetrievedContextState(targetSessionId, {
        runId: options?.runId ?? this.activeRunId,
        snapshot: options?.snapshot ?? createSnapshotFromRuntimeState(this, targetSessionId),
        nodeId: options?.nodeId ?? this.visibleNodeId
      });
      if (this.sessionId === targetSessionId) {
        this.retrievedContext = refreshedRetrieved;
      }
      return deriveRun(refreshedRetrieved);
    },
    applySessionSnapshot(
      sessionId: string,
      snapshot: SessionSnapshot,
      retrieved?: RetrievedContextState | null,
      runtimeView?:
        | Pick<
            SessionRuntimeView,
            | "historyNodes"
            | "historyBranches"
            | "historyCursor"
            | "submissionPlan"
            | "controlBoundaryEvidence"
            | "runControlAuditSummary"
            | "historyStateAuditSummary"
          >
        | null
    ) {
      const persisted = loadPersistedRuntimeState(sessionId);
      const canReusePersistedState = isPersistedStateCompatible(snapshot, persisted);
      const canMergePersistedMessages = isPersistedMessageShapeCompatible(snapshot, persisted);
      const restoredState = canReusePersistedState ? persisted : null;
      const blankFields = createBlankSessionRuntimeFields();
      const retrievedSummary = retrieved?.sessionContext?.summary?.trim() ?? "";
      const snapshotSummary = snapshot.history.length > 0 ? snapshot.summary : "";
      const sessionSummary = retrievedSummary || restoredState?.sessionSummary || snapshotSummary;
      const snapshotTurnTraceHistory = snapshot.turnTraceHistory ?? [];
      const effectiveTurnTraceHistory = (
        snapshotTurnTraceHistory.length
          ? snapshotTurnTraceHistory
          : restoredState?.turnTraceHistory ?? []
      ).map((trace) => normalizeTurnTraceRecord(trace));

      this.sessionId = sessionId;
      this.error = null;
      this.isSubmitting = false;
      this.activeTurnId = null;
      this.activeRunId = retrieved?.runState?.runId?.trim() || null;
      this.latestExecutionCheckpoint = null;
      this.latestGraphRunSubmissionPlan = null;
      this.latestGraphRunControlBoundaryEvidence = [];
      this.latestRunControlAuditSummary = cloneRunControlAuditSummary(
        runtimeView?.runControlAuditSummary ?? snapshot.runControlAuditSummary ?? null
      );
      this.latestHistoryStateAuditSummary = cloneHistoryStateAuditSummary(
        runtimeView?.historyStateAuditSummary ?? snapshot.historyStateAuditSummary ?? null
      );
      this.draftMessage = "";
      this.sessionSummary = sessionSummary;
      this.retrievedContext = cloneRetrievedContext(retrieved ?? deriveRetrievedContextFromSnapshot(snapshot));
      this.messages = hydrateMessagesFromHistory(
        snapshot.history,
        canMergePersistedMessages ? persisted?.messages : null,
        effectiveTurnTraceHistory
      );
      this.attachmentAssets = snapshot.attachmentAssets ?? restoredState?.attachmentAssets ?? [];
      this.turnTraceHistory = effectiveTurnTraceHistory;
      this.eventCursorByTurnId = buildEventCursorByTurnTraceHistory(this.turnTraceHistory);
      this.providerRequestedName = restoredState?.providerRequestedName ?? blankFields.providerRequestedName;
      this.providerName = restoredState?.providerName ?? blankFields.providerName;
      this.providerProtocol = restoredState?.providerProtocol ?? blankFields.providerProtocol;
      this.providerModel = restoredState?.providerModel ?? blankFields.providerModel;
      this.providerSource = restoredState?.providerSource ?? blankFields.providerSource;
      this.providerMode = restoredState?.providerMode ?? blankFields.providerMode;
      this.fallbackReason = restoredState?.fallbackReason ?? blankFields.fallbackReason;
      this.inputTokens = restoredState?.inputTokens ?? blankFields.inputTokens;
      this.outputTokens = restoredState?.outputTokens ?? blankFields.outputTokens;
      this.totalTokens = restoredState?.totalTokens ?? blankFields.totalTokens;
      this.firstTokenLatencyMs = restoredState?.firstTokenLatencyMs ?? blankFields.firstTokenLatencyMs;
      this.toolActivities = [];
      this.traceSteps = createDefaultTraceSteps();
      const restoredTraceTimeline = cloneTraceTimeline(this.turnTraceHistory[this.turnTraceHistory.length - 1]?.traceTimeline);
      this.traceTimeline = restoredTraceTimeline.length ? restoredTraceTimeline : createDefaultTraceTimeline();
      this.phase = resolveRestoredPersistedPhase(
        restoredState?.phase ?? null,
        this.messages,
        this.turnTraceHistory
      );
      const hydratedRunId = runtimeView?.submissionPlan?.runId?.trim() || null;
      if (!this.activeRunId && hydratedRunId) {
        this.activeRunId = hydratedRunId;
      }
      this.latestGraphRunControlBoundaryEvidence = cloneGraphRunControlBoundaryEvidence(
        runtimeView?.controlBoundaryEvidence
      );
      this.applyHistoryState(sessionId, {
        historyNodes: runtimeView?.historyNodes,
        historyBranches: runtimeView?.historyBranches,
        ...(cloneHistoryCursor(runtimeView?.historyCursor) ?? {})
      });
      this.persistHistory();
    },
    async checkoutHistoryNode(nodeId: string, mode: HistoryCheckoutMode = "transcript_only") {
      const sessionId = this.sessionId;
      if (!sessionId || !nodeId.trim()) {
        return null;
      }

      let result: HistoryCheckoutResult;
      if (isTauriAvailable()) {
        const payload = await safeInvoke<HistoryCheckoutWireResult>("checkout_history_node", {
          sessionId,
          nodeId,
          mode
        });
        await this.loadSessionState(sessionId, {
          refreshCatalog: false,
          nodeId
        });
        result = normalizeHistoryCheckoutResult(payload, this.historyNodes, this.historyBranches);
      } else {
        result = {
          sessionId,
          nodeId,
          visibleNodeId: nodeId,
          activeBranchId: this.activeBranchId,
          branchHeadNodeId: this.branchHeadNodeId,
          workspaceNodeId: this.visibleNodeId,
          mode:
            this.branchHeadNodeId && this.branchHeadNodeId !== nodeId ? "historical" : "live",
          requestedMode: mode,
          appliedMode: "transcript_only",
          transcriptRestoreApplied: true,
          workspaceRollbackCapable: false,
          workspaceRestoreCapable: false,
          workspaceRollbackApplied: false,
          degraded: mode === "transcript_and_workspace",
          degradedToTranscriptOnly: mode === "transcript_and_workspace",
          degradationReason:
            mode === "transcript_and_workspace" ? "workspace_rollback_unsupported" : null,
          historyStateEvidence: null,
          historyStateAuditSummary: null,
          historyNodes: this.historyNodes,
          historyBranches: this.historyBranches
        };
      }

      this.applyHistoryState(sessionId, result);
      this.latestHistoryStateAuditSummary = cloneHistoryStateAuditSummary(
        result.historyStateAuditSummary ?? null
      );
      return result;
    },
    async restoreBranchHead(branchId?: string | null) {
      const sessionId = this.sessionId;
      const targetBranchId = branchId?.trim() || this.activeBranchId;
      if (!sessionId) {
        return null;
      }

      let result: HistoryRestoreResult;
      if (isTauriAvailable()) {
        const payload = await safeInvoke<HistoryRestoreWireResult>("restore_branch_head", {
          sessionId,
          branchId: targetBranchId ?? null
        });
        await this.loadSessionState(sessionId, {
          refreshCatalog: false,
          nodeId: payload.cursor.visibleNodeId ?? payload.cursor.branchHeadNodeId ?? null
        });
        result = normalizeHistoryRestoreResult(payload, this.historyNodes, this.historyBranches);
      } else {
        const branchHeadNodeId =
          resolveHistoryBranchHeadNodeId(targetBranchId, this.historyBranches) ?? this.branchHeadNodeId;
        result = {
          sessionId,
          branchId: targetBranchId,
          visibleNodeId: branchHeadNodeId,
          activeBranchId: targetBranchId,
          branchHeadNodeId,
          workspaceNodeId: branchHeadNodeId,
          mode: "live",
          restoredNodeId: branchHeadNodeId,
          transcriptRestoreApplied: true,
          workspaceRollbackCapable: false,
          workspaceRestoreCapable: false,
          workspaceRollbackApplied: false,
          workspaceRestoreApplied: false,
          degraded: false,
          degradedToTranscriptOnly: false,
          degradationReason: null,
          restoredFromNodeId: this.visibleNodeId,
          historyStateEvidence: null,
          historyStateAuditSummary: null,
          historyNodes: this.historyNodes,
          historyBranches: this.historyBranches
        };
      }
      this.applyHistoryState(sessionId, result);
      this.latestHistoryStateAuditSummary = cloneHistoryStateAuditSummary(
        result.historyStateAuditSummary ?? null
      );
      return result;
    },
    async forkHistoryNode(nodeId?: string | null) {
      const sessionId = this.sessionId;
      const targetNodeId = nodeId?.trim() || this.visibleNodeId;
      if (!sessionId || !targetNodeId) {
        return null;
      }

      let result: HistoryForkResult;
      if (isTauriAvailable()) {
        const payload = await safeInvoke<HistoryForkWireResult>("fork_from_history_node", {
          sessionId,
          nodeId: targetNodeId
        });
        await this.loadSessionState(sessionId, {
          refreshCatalog: false,
          nodeId: payload.cursor.visibleNodeId ?? payload.cursor.branchHeadNodeId ?? null
        });
        result = normalizeHistoryForkResult(payload, this.historyNodes, this.historyBranches);
      } else {
        const createdBranchId = `branch-${Date.now()}`;
        const nextBranch: HistoryBranch = {
          branchId: createdBranchId,
          sessionId,
          baseNodeId: targetNodeId,
          headNodeId: targetNodeId,
          forkedFromBranchId: this.activeBranchId,
          forkedFromNodeId: targetNodeId,
          label: null,
          createdAtMs: Date.now(),
          updatedAtMs: Date.now()
        };
        result = {
          sessionId,
          nodeId: targetNodeId,
          createdBranchId,
          branch: { ...nextBranch },
          visibleNodeId: targetNodeId,
          activeBranchId: createdBranchId,
          branchHeadNodeId: targetNodeId,
          workspaceNodeId: this.visibleNodeId,
          mode: "live",
          historyStateEvidence: null,
          historyStateAuditSummary: null,
          historyNodes: this.historyNodes,
          historyBranches: [...this.historyBranches, nextBranch]
        };
      }

      this.applyHistoryState(sessionId, result);
      this.latestHistoryStateAuditSummary = cloneHistoryStateAuditSummary(
        result.historyStateAuditSummary ?? null
      );
      return result;
    },
    async switchHistoryBranch(branchId: string) {
      const sessionId = this.sessionId;
      if (!sessionId || !branchId.trim()) {
        return null;
      }

      let result: HistoryBranchSwitchResult;
      if (isTauriAvailable()) {
        const payload = await safeInvoke<HistoryBranchSwitchWireResult>("switch_history_branch", {
          sessionId,
          branchId
        });
        await this.loadSessionState(sessionId, {
          refreshCatalog: false,
          nodeId: payload.cursor.visibleNodeId ?? payload.cursor.branchHeadNodeId ?? null
        });
        result = normalizeHistoryBranchSwitchResult(
          payload,
          this.historyNodes,
          this.historyBranches
        );
      } else {
        const branchHeadNodeId = resolveHistoryBranchHeadNodeId(branchId, this.historyBranches);
        result = {
          sessionId,
          branchId,
          previousBranchId: this.activeBranchId,
          nodeId: branchHeadNodeId,
          visibleNodeId: branchHeadNodeId,
          activeBranchId: branchId,
          branchHeadNodeId,
          workspaceNodeId: branchHeadNodeId,
          mode: "live",
          historyStateEvidence: null,
          historyStateAuditSummary: null,
          historyNodes: this.historyNodes,
          historyBranches: this.historyBranches
        };
      }
      this.applyHistoryState(sessionId, result);
      this.latestHistoryStateAuditSummary = cloneHistoryStateAuditSummary(
        result.historyStateAuditSummary ?? null
      );
      return result;
    },
    async switchSession(nextSessionId: string) {
      if (this.isSubmitting || this.sessionOperation) {
        return;
      }
      if (nextSessionId === this.sessionId) {
        return;
      }

      const previousSnapshot = createSessionRuntimeSnapshot(this);
      this.sessionOperation = "switching";
      this.sessionError = null;

      try {
        await this.loadSessionState(nextSessionId);
        debugLog("session:switch", {
          from: previousSnapshot.sessionId,
          to: nextSessionId
        });
      } catch (error) {
        restoreSessionRuntimeSnapshot(this, previousSnapshot);
        this.sessionError = `切换对话失败：${String(error)}`;
        debugLog("session:switch:error", {
          from: previousSnapshot.sessionId,
          to: nextSessionId,
          error: String(error)
        });
      } finally {
        this.sessionOperation = null;
      }
    },
    async createSession() {
      if (this.sessionOperation || !hasPersistableMessages(this.messages)) {
        return;
      }

      const nextSessionId = `session-${Date.now()}`;
      this.persistHistory();
      await this.loadSessionCatalog();
      this.resetSessionRuntimeState();
      this.sessionId = nextSessionId;
      this.phase = "idle";
      this.sessionError = null;
      this.sessionList = [
        createTransientSessionOverview(nextSessionId),
        ...this.sessionList.filter((session) => session.conversationId !== nextSessionId)
      ];
      debugLog("session:create:transient", {
        from: this.sessionList[1]?.conversationId ?? null,
        to: nextSessionId
      });
    },
    async deleteSession(targetSessionId: string) {
      if (this.isSubmitting || this.sessionOperation) {
        return;
      }

      const deletingActiveEmptySession =
        targetSessionId === this.sessionId && !hasPersistableMessages(this.messages);
      const previousSnapshot = createSessionRuntimeSnapshot(this);
      const persistedStateToRestore = loadPersistedRuntimeState(targetSessionId);
      this.sessionOperation = "deleting";
      this.sessionError = null;

      try {
        removePersistedSessionState(targetSessionId);

        if (isTauriAvailable() && !deletingActiveEmptySession) {
          this.sessionList = await safeInvoke<SessionOverview[]>("delete_session", {
            sessionId: targetSessionId
          });
        } else {
          await this.loadSessionCatalog();
        }

        const fallbackSessionId =
          this.sessionList.find((session) => session.conversationId !== targetSessionId)?.conversationId ??
          this.sessionList[0]?.conversationId ??
          DEFAULT_SESSION_ID;

        const shouldLoadFallbackSession =
          deletingActiveEmptySession ||
          this.sessionId === targetSessionId ||
          !this.sessionList.some((session) => session.conversationId === this.sessionId);

        if (!shouldLoadFallbackSession) {
          debugLog("session:delete", {
            targetSessionId
          });
          return;
        }

        this.resetSessionRuntimeState();
        this.sessionId = fallbackSessionId;
        this.phase = "connecting";

        try {
          if (this.sessionList.length === 0 && fallbackSessionId === DEFAULT_SESSION_ID) {
            this.phase = "idle";
            debugLog("session:delete:empty-fallback", {
              targetSessionId
            });
            return;
          }

          await this.loadSessionState(fallbackSessionId, { refreshCatalog: false });
          debugLog("session:delete:fallback", {
            targetSessionId,
            fallbackSessionId
          });
        } catch (error) {
          this.resetSessionRuntimeState();
          this.sessionId = fallbackSessionId;
          this.phase = "idle";
          this.sessionError = `删除对话后加载替代对话失败：${String(error)}`;
          debugLog("session:delete:fallback-error", {
            targetSessionId,
            fallbackSessionId,
            error: String(error)
          });
        }
      } catch (error) {
        if (persistedStateToRestore) {
          persistSessionState(targetSessionId, persistedStateToRestore);
        }
        restoreSessionRuntimeSnapshot(this, previousSnapshot);
        this.sessionError = `删除对话失败：${String(error)}`;
        debugLog("session:delete:error", {
          targetSessionId,
          error: String(error)
        });
      } finally {
        this.sessionOperation = null;
      }
    },
    async initializeSessions() {
      if (this.sessionOperation) {
        return;
      }

      const previousSnapshot = createSessionRuntimeSnapshot(this);
      this.sessionOperation = "initializing";
      this.sessionError = null;

      try {
        await this.loadSessionCatalog();
        const preferredSessionId = this.sessionList[0]?.conversationId ?? this.sessionId;
        const runtimeView =
          this.sessionList.length === 0 ? await this.loadSessionRuntimeViewState(preferredSessionId) : null;

        if (
          this.sessionList.length === 0 &&
          preferredSessionId === this.sessionId &&
          !runtimeView?.checkpoint &&
          runtimeView != null &&
          runtimeView.session.history.length === 0
        ) {
          this.resetSessionRuntimeState();
          this.sessionId = preferredSessionId;
          this.phase = "idle";
          debugLog("session:init:empty");
          return;
        }

        this.resetSessionRuntimeState();
        this.sessionId = preferredSessionId;
        this.phase = "connecting";
        await this.loadSessionState(preferredSessionId, {
          refreshCatalog: false,
          runtimeView
        });
        debugLog("session:init", {
          preferredSessionId
        });
      } catch (error) {
        restoreSessionRuntimeSnapshot(this, previousSnapshot);
        this.sessionError = `初始化对话失败：${String(error)}`;
        debugLog("session:init:error", {
          error: String(error)
        });
      } finally {
        this.sessionOperation = null;
      }
    },
    upsertTurnTrace(
      turnId: string,
      patch: Partial<Omit<TurnTraceRecord, "turnId" | "updatedAt">> & { updatedAt?: number },
      persist = true
    ) {
      const existing = this.turnTraceHistory.find((item) => item.turnId === turnId);
      const updatedAt = patch.updatedAt ?? Date.now();
      const existingTitle = existing?.title?.trim();
      const resolvedTitle =
        patch.title ??
        (existingTitle && existingTitle !== "未命名轮次" ? existingTitle : undefined) ??
        buildTurnTraceTitleFromMessages(this.messages, turnId);

      if (existing) {
        Object.assign(existing, patch, {
          title: resolvedTitle,
          updatedAt,
          traceTimeline: patch.traceTimeline ? cloneTraceTimeline(patch.traceTimeline) : existing.traceTimeline,
          providerCallRecords:
            patch.providerCallRecords != null
              ? cloneProviderCallRecords(patch.providerCallRecords)
              : existing.providerCallRecords,
          hookTraceRecords:
            patch.hookTraceRecords != null
              ? cloneHookTraceRecords(patch.hookTraceRecords)
              : existing.hookTraceRecords
        });
        if (persist) {
          this.persistHistory();
        }
        return;
      }

      this.turnTraceHistory.push(normalizeTurnTraceRecord({
        turnId,
        title: patch.title ?? "未命名轮次",
        phase: patch.phase ?? this.phase,
        traceSteps: cloneTraceSteps(patch.traceSteps),
        traceTimeline: cloneTraceTimeline(patch.traceTimeline),
        toolActivities: cloneToolActivities(patch.toolActivities),
        providerCallRecords: cloneProviderCallRecords(patch.providerCallRecords),
        hookTraceRecords: cloneHookTraceRecords(patch.hookTraceRecords),
        providerRequestedName: patch.providerRequestedName ?? null,
        providerName: patch.providerName ?? null,
        providerProtocol: patch.providerProtocol ?? null,
        providerModel: patch.providerModel ?? null,
        providerSource: patch.providerSource ?? null,
        providerMode: patch.providerMode ?? null,
        buildContextObservation: cloneBuildContextObservation(patch.buildContextObservation),
        sessionSummary: patch.sessionSummary ?? "",
        fallbackReason: patch.fallbackReason ?? null,
        error: patch.error ?? null,
        inputTokens: patch.inputTokens ?? null,
        cacheHitInputTokens: patch.cacheHitInputTokens ?? null,
        reasoningTokens: patch.reasoningTokens ?? null,
        outputTokens: patch.outputTokens ?? null,
        totalTokens: patch.totalTokens ?? null,
        firstTokenLatencyMs: patch.firstTokenLatencyMs ?? null,
        turnDurationMs: patch.turnDurationMs ?? null,
        updatedAt
      }));
      this.turnTraceHistory[this.turnTraceHistory.length - 1]!.title = resolvedTitle;
      if (persist) {
        this.persistHistory();
      }
    },
    shouldProcessTurnEvent(payload: Pick<TurnStreamEvent, "turnId" | "eventId" | "sequence" | "emittedAtMs">) {
      return shouldAcceptTurnEvent(this.eventCursorByTurnId[payload.turnId], payload);
    },
    commitTurnEventCursor(payload: Pick<TurnStreamEvent, "turnId" | "eventId" | "sequence" | "emittedAtMs">) {
      this.eventCursorByTurnId[payload.turnId] = {
        eventId: payload.eventId?.trim() || null,
        sequence: typeof payload.sequence === "number" && Number.isFinite(payload.sequence) ? payload.sequence : null,
        emittedAtMs:
          typeof payload.emittedAtMs === "number" && Number.isFinite(payload.emittedAtMs)
            ? payload.emittedAtMs
            : null
      };
    },
    resolveTurnTraceTimeline(turnId: string) {
      const existingTimeline = cloneTraceTimeline(
        this.turnTraceHistory.find((trace) => trace.turnId === turnId)?.traceTimeline
      );
      if (existingTimeline.length) {
        return existingTimeline;
      }

      return this.activeTurnId === turnId && this.traceTimeline.length
        ? cloneTraceTimeline(this.traceTimeline)
        : createDefaultTraceTimeline();
    },
    commitTurnTraceTimeline(
      turnId: string,
      traceTimeline: TraceTimelineEntry[],
      patch: Partial<Omit<TurnTraceRecord, "turnId" | "updatedAt">> & { updatedAt?: number } = {},
      persist = true
    ) {
      this.traceTimeline = cloneTraceTimeline(traceTimeline);
      this.upsertTurnTrace(turnId, {
        ...patch,
        traceTimeline: this.traceTimeline
      }, persist);
      debugLog("cache-telemetry:trace-committed", {
        phase: patch.phase ?? this.phase,
        ...buildCacheTelemetryDebugSnapshot(
          {
            turnId,
            eventType: patch.eventType ?? undefined,
            eventId: patch.eventId ?? undefined,
            sequence: patch.sequence ?? undefined,
            inputTokens: patch.inputTokens ?? undefined,
            cacheHitInputTokens: patch.cacheHitInputTokens ?? undefined,
            outputTokens: patch.outputTokens ?? undefined,
            totalTokens: patch.totalTokens ?? undefined,
            providerCallRecords: patch.providerCallRecords,
            traceTimeline: this.traceTimeline
          },
          this.traceTimeline,
          this.turnTraceHistory.find((trace) => trace.turnId === turnId) ?? null
        )
      });
    },
    updateActiveTraceTimeline(traceTimeline: TraceTimelineEntry[]) {
      this.traceTimeline = cloneTraceTimeline(traceTimeline);
    },
    updateActiveModelTraceFromAssistant(turnId: string) {
      const assistantMessage = this.messages.find((message) => message.turnId === turnId && message.role === "assistant");
      if (!assistantMessage || !this.traceTimeline.length) {
        return;
      }

      const traceTimeline = cloneTraceTimeline(this.traceTimeline);
      const reverseModelIndex = [...traceTimeline]
        .reverse()
        .findIndex((entry) => canonicalizeTraceTimelineKind(entry.kind) === "call_model");
      if (reverseModelIndex === -1) {
        return;
      }

      const modelIndex = traceTimeline.length - 1 - reverseModelIndex;
      const modelEntry = traceTimeline[modelIndex]!;
      traceTimeline[modelIndex] = {
        ...modelEntry,
        state: assistantMessage.status === "pending" ? "active" : modelEntry.state,
        text: assistantMessage.content || modelEntry.text || null,
        reasoningContent: assistantMessage.reasoningContent ?? modelEntry.reasoningContent ?? null,
        firstTokenLatencyMs: this.firstTokenLatencyMs ?? modelEntry.firstTokenLatencyMs ?? null
      };
      this.traceTimeline = traceTimeline;
    },
    applyTurnTokenStats(turnId: string, inputTokens?: number | null, outputTokens?: number | null, persist = true) {
      const userMessage = this.messages.find((item) => item.turnId === turnId && item.role === "user");
      const assistantMessage = this.messages.find((item) => item.turnId === turnId && item.role === "assistant");

      if (userMessage && inputTokens != null) {
        userMessage.tokenCount = inputTokens;
      }

      if (assistantMessage && outputTokens != null) {
        assistantMessage.tokenCount = outputTokens;
      }

      if (persist) {
        this.persistHistory();
      }
    },
    applyOutputEnd(payload: TurnStreamEvent) {
      this.flushBufferedStreamText(payload.turnId);

      const assistantMessage = this.ensureAssistantMessage(
        payload.turnId,
        buildAssistantModelLabel(payload.providerName, payload.providerModel)
      );

      const finalText = payload.text?.trim();
      if (finalText) {
        assistantMessage.content = payload.text ?? assistantMessage.content;
      }
      assistantMessage.reasoningContent = normalizeReasoningContent(
        payload.reasoningContent ?? assistantMessage.reasoningContent ?? null
      );
      assistantMessage.status = "done";
      assistantMessage.modelName = buildAssistantModelLabel(payload.providerName, payload.providerModel);

      this.providerRequestedName = payload.providerRequestedName ?? this.providerRequestedName;
      this.providerName = payload.providerName ?? this.providerName;
      this.providerProtocol = payload.providerProtocol ?? this.providerProtocol;
      this.providerModel = payload.providerModel ?? this.providerModel;
      this.providerSource = payload.providerSource ?? this.providerSource;
      this.providerMode = payload.providerMode ?? this.providerMode;
      this.fallbackReason = payload.fallbackReason ?? this.fallbackReason;
      this.inputTokens = payload.inputTokens ?? this.inputTokens;
      this.outputTokens = payload.outputTokens ?? this.outputTokens;
      this.totalTokens = payload.totalTokens ?? this.totalTokens;
      this.firstTokenLatencyMs = payload.firstTokenLatencyMs ?? this.firstTokenLatencyMs;
      this.applyTurnTokenStats(payload.turnId, payload.inputTokens, payload.outputTokens, false);
      this.scheduleDeferredPersist(OUTPUT_END_PERSIST_DELAY_MS);
    },
    ensureAssistantMessage(turnId: string, modelName?: string | null) {
      const messageId = `assistant-${turnId}`;
      const existingMessage = this.messages.find((item) => item.id === messageId && item.role === "assistant");

      if (existingMessage) {
        if (modelName?.trim()) {
          existingMessage.modelName = modelName;
        }
        return existingMessage;
      }

      const assistantMessage: ChatMessage = {
        id: messageId,
        turnId,
        role: "assistant",
        reasoningContent: null,
        content: "",
        status: "pending",
        tokenCount: null,
        modelName: modelName?.trim() || undefined
      };

      this.messages.push(assistantMessage);
      this.persistHistory();
      return this.messages.find((item) => item.id === messageId && item.role === "assistant") ?? assistantMessage;
    },
    syncToolMessages(turnId: string, toolActivities?: ToolActivity[] | null, persist = true) {
      if (!toolActivities) {
        return;
      }

      const activeTools = toolActivities.filter((tool) => tool.status !== "planned");
      for (const tool of activeTools) {
        const messageId = `tool-${turnId}-${tool.id}`;
        const existingMessage = this.messages.find((item) => item.id === messageId && item.role === "tool");
        const nextContent = tool.resultText ?? "";
        const nextDetail = buildToolMessageDetail(tool);

        if (existingMessage) {
          existingMessage.content = nextContent;
          existingMessage.status = toolStatusToMessageStatus(tool.status);
          existingMessage.toolName = tool.name;
          existingMessage.detail = nextDetail;
          existingMessage.durationSeconds = tool.durationSeconds ?? null;
          if (persist) {
            this.persistHistory();
          }
          continue;
        }

        this.messages.push({
          id: messageId,
          turnId,
          role: "tool",
          content: nextContent,
          status: toolStatusToMessageStatus(tool.status),
          toolName: tool.name,
          detail: nextDetail,
          durationSeconds: tool.durationSeconds ?? null
        });
        if (persist) {
          this.persistHistory();
        }
      }
    },
    setDraftMessage(message: string) {
      this.draftMessage = message;
    },
    async fetchHealth() {
      if (this.health) {
        return;
      }

      this.phase = "connecting";
      this.error = null;

      try {
        const payload: HealthPayload = isTauriAvailable()
          ? await safeInvoke<HealthPayload>("health_check")
          : {
              appName: "Pony Agent",
              appVersion: "dev-preview",
              runtime: "browser-preview",
              graphEngine: "mock-stream",
              graphContractVersion: "browser-preview"
            };
        this.health = payload;
        this.phase = "completed";
        debugLog("health:ok", {
          runtime: payload.runtime,
          graph: payload.graphEngine
        });
        this.traceSteps = this.traceSteps.map((step) =>
          step.id === "step-context"
            ? { ...step, state: "completed" }
            : step
        );
      } catch (error) {
        this.error = `Rust 后端连接失败：${String(error)}`;
        this.phase = "failed";
        debugLog("health:error", {
          error: String(error)
        });
      }
    },
    async fetchAvailableTools() {
      if (this.availableTools.length > 0 && isTauriAvailable()) {
        const hasDefaultOnly = this.availableTools.every((tool, index) => tool.name === defaultAvailableTools[index]?.name);
        if (!hasDefaultOnly) {
          return;
        }
      }

      try {
        this.availableTools = isTauriAvailable()
          ? await safeInvoke<AvailableTool[]>("list_available_tools")
          : createAvailableTools();
        debugLog("tools:ok", {
          count: this.availableTools.length
        });
      } catch (error) {
        this.availableTools = createAvailableTools();
        debugLog("tools:error", {
          error: String(error)
        });
      }
    },
    async fetchCapabilitySources() {
      try {
        this.capabilitySources = isTauriAvailable()
          ? await safeInvoke<CapabilitySourceView[]>("list_capability_sources")
          : createCapabilitySources();
        debugLog("capability-sources:ok", {
          count: this.capabilitySources.length
        });
      } catch (error) {
        this.capabilitySources = createCapabilitySources();
        debugLog("capability-sources:error", {
          error: String(error)
        });
      }
    },
    async fetchCapabilities(filter?: { sourceId?: string | null; kind?: string | null }) {
      const sourceId = filter?.sourceId?.trim() || null;
      const kind = filter?.kind?.trim() || null;

      try {
        this.capabilities = isTauriAvailable()
          ? await safeInvoke<CapabilityView[]>("list_capabilities", {
            sourceId,
            kind
          })
          : createCapabilities().filter((capability) => {
            if (sourceId && capability.sourceId !== sourceId) {
              return false;
            }
            if (kind && capability.kind !== kind) {
              return false;
            }
            return true;
          });
        debugLog("capabilities:ok", {
          count: this.capabilities.length,
          sourceId,
          kind
        });
      } catch (error) {
        this.capabilities = createCapabilities().filter((capability) => {
          if (sourceId && capability.sourceId !== sourceId) {
            return false;
          }
          if (kind && capability.kind !== kind) {
            return false;
          }
          return true;
        });
        debugLog("capabilities:error", {
          sourceId,
          kind,
          error: String(error)
        });
      }
    },
    async inspectCapability(capabilityId: string) {
      if (!capabilityId.trim()) {
        return null;
      }

      if (!isTauriAvailable()) {
        return createCapabilities().find((capability) => capability.capabilityId === capabilityId) ?? null;
      }

      try {
        return await safeInvoke<CapabilityView | null>("inspect_capability", {
          capabilityId
        });
      } catch (error) {
        debugLog("capability:inspect:error", {
          capabilityId,
          error: String(error)
        });
        return createCapabilities().find((capability) => capability.capabilityId === capabilityId) ?? null;
      }
    },
    async inspectCapabilitySource(sourceId: string) {
      if (!sourceId.trim()) {
        return null;
      }

      if (!isTauriAvailable()) {
        return createCapabilitySources().find((source) => source.sourceId === sourceId) ?? null;
      }

      try {
        return await safeInvoke<CapabilitySourceView | null>("inspect_capability_source", {
          sourceId
        });
      } catch (error) {
        debugLog("capability-source:inspect:error", {
          sourceId,
          error: String(error)
        });
        return createCapabilitySources().find((source) => source.sourceId === sourceId) ?? null;
      }
    },
    async initializeTurnEvents() {
      if (this.eventsReady) {
        return;
      }

      if (!isTauriAvailable()) {
        this.eventsReady = true;
        debugLog("events:browser-preview");
        return;
      }

      debugLog("events:init");

      const startedUnlisten = await safeListen<TurnStreamEvent>("turn:started", ({ payload }) => {
        if (this.activeTurnId !== payload.turnId) {
          return;
        }
        if (!this.shouldProcessTurnEvent(payload)) {
          return;
        }
        this.commitTurnEventCursor(payload);
        this.cancelStreamFlush();
        this.streamBufferTurnId = null;
        this.streamBufferText = "";
        this.streamBufferReasoning = "";
        this.resetStreamDebugMetrics();

        this.ensureAssistantMessage(
          payload.turnId,
          buildAssistantModelLabel(payload.providerName, payload.providerModel)
        );
        this.phase = resolveRuntimePhaseFromEvent(payload, "calling_model");
        debugLog("event:started", {
          turnId: payload.turnId
        });
        this.providerRequestedName = payload.providerRequestedName ?? this.providerRequestedName;
        this.providerName = payload.providerName ?? this.providerName;
        this.providerProtocol = payload.providerProtocol ?? this.providerProtocol;
        this.providerModel = payload.providerModel ?? this.providerModel;
        this.providerSource = payload.providerSource ?? this.providerSource;
        this.providerMode = payload.providerMode ?? this.providerMode;
        this.fallbackReason = payload.fallbackReason ?? this.fallbackReason;
        this.inputTokens = payload.inputTokens ?? this.inputTokens;
        this.outputTokens = payload.outputTokens ?? this.outputTokens;
        this.totalTokens = payload.totalTokens ?? this.totalTokens;
        this.firstTokenLatencyMs = payload.firstTokenLatencyMs ?? this.firstTokenLatencyMs;
        this.traceSteps = payload.traceSteps ?? this.traceSteps;
        this.traceTimeline = resolveEventTraceTimeline(payload, () =>
          buildFallbackRuntimeTraceTimeline({
            turnId: payload.turnId,
            eventType: payload.eventType,
            messages: this.messages,
            phase: payload.phase ?? this.phase,
            buildContextObservation: cloneBuildContextObservation(payload.buildContextObservation),
            providerPatch: {
              providerName: payload.providerName ?? this.providerName,
              providerProtocol: payload.providerProtocol ?? this.providerProtocol,
              providerModel: payload.providerModel ?? this.providerModel,
              providerSource: payload.providerSource ?? this.providerSource,
              providerMode: payload.providerMode ?? this.providerMode
            }
          })
        );
        this.toolActivities = payload.toolActivities ?? this.toolActivities;
        this.syncToolMessages(payload.turnId, payload.toolActivities, false);
        this.updateActiveTraceTimeline(this.traceTimeline);
      });

      const deltaUnlisten = await safeListen<TurnStreamEvent>("turn:delta", ({ payload }) => {
        if (this.activeTurnId !== payload.turnId) {
          return;
        }
        if (!this.shouldProcessTurnEvent(payload)) {
          return;
        }
        this.commitTurnEventCursor(payload);

        this.ensureAssistantMessage(
          payload.turnId,
          buildAssistantModelLabel(this.providerName, this.providerModel)
        );

        const deltaText = payload.text ?? "";
        const deltaReasoning = payload.reasoningContent ?? "";

        if (deltaReasoning) {
          this.streamBufferReasoning += deltaReasoning;
        }

        if (deltaText) {
          this.streamBufferText += deltaText;
        }

        this.streamDebugDeltaCount += 1;
        this.streamDebugLastDeltaAtMs = Date.now();
        this.streamDebugTextCharsReceived += deltaText.length;
        this.streamDebugReasoningCharsReceived += deltaReasoning.length;

        this.phase = resolveRuntimePhaseFromEvent(payload, this.phase);
        this.firstTokenLatencyMs = payload.firstTokenLatencyMs ?? this.firstTokenLatencyMs;
        if (payload.traceTimeline?.length) {
          this.updateActiveTraceTimeline(payload.traceTimeline);
        }
        this.scheduleStreamFlush(payload.turnId);
        updateStreamDebugBucket("runtime", {
          turnId: payload.turnId,
          deltaCount: this.streamDebugDeltaCount,
          flushCount: this.streamDebugFlushCount,
          textCharsReceived: this.streamDebugTextCharsReceived,
          textCharsFlushed: this.streamDebugTextCharsFlushed,
          reasoningCharsReceived: this.streamDebugReasoningCharsReceived,
          reasoningCharsFlushed: this.streamDebugReasoningCharsFlushed,
          lastDeltaTextLength: deltaText.length,
          lastDeltaReasoningLength: deltaReasoning.length,
          bufferTextLength: this.streamBufferText.length,
          bufferReasoningLength: this.streamBufferReasoning.length,
          bufferTurnId: this.streamBufferTurnId,
          pendingRaf: this.streamFlushFrameId != null,
          deltaToLastFlushMs:
            this.streamDebugLastFlushAtMs == null ? null : this.streamDebugLastDeltaAtMs - this.streamDebugLastFlushAtMs
        });
        debugLog("event:delta", {
          turnId: payload.turnId,
          deltaLength: deltaText.length,
          reasoningLength: deltaReasoning.length
        });
      });

      const traceUnlisten = await safeListen<TurnStreamEvent>("turn:trace", ({ payload }) => {
        if (this.activeTurnId !== payload.turnId) {
          return;
        }
        if (!this.shouldProcessTurnEvent(payload)) {
          return;
        }
        this.commitTurnEventCursor(payload);
        this.flushBufferedStreamText(payload.turnId);

        this.phase = resolveRuntimePhaseFromEvent(payload, this.phase);
        this.traceSteps = payload.traceSteps ?? this.traceSteps;
        const traceTimeline = resolveEventTraceTimeline(payload, () =>
          buildFallbackRuntimeTraceTimeline({
            turnId: payload.turnId,
            eventType: payload.eventType,
            messages: this.messages,
            phase: payload.phase,
            assistantMessage: this.messages.find((message) => message.turnId === payload.turnId && message.role === "assistant") ?? null,
            toolActivities: this.toolActivities,
            providerPatch: {
              providerName: this.providerName,
              providerProtocol: this.providerProtocol,
              providerModel: this.providerModel,
              providerSource: this.providerSource,
              providerMode: this.providerMode
            },
            firstTokenLatencyMs: this.firstTokenLatencyMs
          })
        );
        this.updateActiveTraceTimeline(traceTimeline);
        this.updateActiveModelTraceFromAssistant(payload.turnId);
        debugLog("event:trace", {
          turnId: payload.turnId,
          steps: this.traceSteps.length
        });
      });

      const phaseChangedUnlisten = await safeListen<TurnStreamEvent>("turn:phase_changed", ({ payload }) => {
        if (this.activeTurnId !== payload.turnId) {
          return;
        }
        if (!this.shouldProcessTurnEvent(payload)) {
          return;
        }
        this.commitTurnEventCursor(payload);
        this.flushBufferedStreamText(payload.turnId);

        this.phase = resolveRuntimePhaseFromEvent(payload, this.phase);
        this.traceSteps = payload.traceSteps ?? this.traceSteps;
        const traceTimeline = resolveEventTraceTimeline(payload, () =>
          buildFallbackRuntimeTraceTimeline({
            turnId: payload.turnId,
            eventType: payload.eventType,
            messages: this.messages,
            phase: payload.phase,
            assistantMessage: this.messages.find((message) => message.turnId === payload.turnId && message.role === "assistant") ?? null,
            toolActivities: this.toolActivities,
            providerPatch: {
              providerName: this.providerName,
              providerProtocol: this.providerProtocol,
              providerModel: this.providerModel,
              providerSource: this.providerSource,
              providerMode: this.providerMode
            },
            firstTokenLatencyMs: this.firstTokenLatencyMs
          })
        );
        this.updateActiveTraceTimeline(traceTimeline);
        this.updateActiveModelTraceFromAssistant(payload.turnId);
      });

      const checkpointPersistedUnlisten = await safeListen<TurnStreamEvent>("turn:checkpoint_persisted", ({ payload }) => {
        if (this.activeTurnId !== payload.turnId) {
          return;
        }
        if (!this.shouldProcessTurnEvent(payload)) {
          return;
        }
        this.commitTurnEventCursor(payload);
        this.flushBufferedStreamText(payload.turnId);

        this.phase = resolveRuntimePhaseFromEvent(payload, this.phase);
        this.traceSteps = payload.traceSteps ?? this.traceSteps;
        const traceTimeline = resolveEventTraceTimeline(payload, () =>
          buildFallbackRuntimeTraceTimeline({
            turnId: payload.turnId,
            eventType: payload.eventType,
            messages: this.messages,
            phase: payload.phase,
            assistantMessage: this.messages.find((message) => message.turnId === payload.turnId && message.role === "assistant") ?? null,
            toolActivities: payload.toolActivities ?? this.toolActivities,
            providerPatch: {
              providerName: this.providerName,
              providerProtocol: this.providerProtocol,
              providerModel: this.providerModel,
              providerSource: this.providerSource,
              providerMode: this.providerMode
            },
            firstTokenLatencyMs: this.firstTokenLatencyMs
          })
        );
        this.updateActiveTraceTimeline(traceTimeline);
        this.updateActiveModelTraceFromAssistant(payload.turnId);
      });

      const toolUnlisten = await safeListen<TurnStreamEvent>("turn:tool", ({ payload }) => {
        if (this.activeTurnId !== payload.turnId) {
          return;
        }
        if (!this.shouldProcessTurnEvent(payload)) {
          return;
        }
        this.commitTurnEventCursor(payload);
        this.flushBufferedStreamText(payload.turnId);

        this.phase = resolveRuntimePhaseFromEvent(payload, this.phase);
        debugLog("event:tool", {
          turnId: payload.turnId,
          tools: (payload.toolActivities ?? []).length
        });
        this.toolActivities = payload.toolActivities ?? this.toolActivities;
        this.syncToolMessages(payload.turnId, payload.toolActivities, false);
        const traceTimeline = resolveEventTraceTimeline(payload, () =>
          buildFallbackRuntimeTraceTimeline({
            turnId: payload.turnId,
            eventType: payload.eventType,
            messages: this.messages,
            phase: payload.phase ?? this.phase,
            assistantMessage: this.messages.find((message) => message.turnId === payload.turnId && message.role === "assistant") ?? null,
            toolActivities: payload.toolActivities ?? this.toolActivities,
            providerPatch: {
              providerName: this.providerName,
              providerProtocol: this.providerProtocol,
              providerModel: this.providerModel,
              providerSource: this.providerSource,
              providerMode: this.providerMode
            },
            firstTokenLatencyMs: this.firstTokenLatencyMs
          })
        );
        this.updateActiveTraceTimeline(traceTimeline);
        this.updateActiveModelTraceFromAssistant(payload.turnId);
      });

      const outputEndUnlisten = await safeListen<TurnStreamEvent>("turn:output_end", ({ payload }) => {
        if (this.activeTurnId !== payload.turnId) {
          return;
        }
        if (!this.shouldProcessTurnEvent(payload)) {
          return;
        }
        this.commitTurnEventCursor(payload);
        this.applyOutputEnd(payload);
        debugLog("event:output_end", {
          turnId: payload.turnId,
          finalTextLength: payload.text?.length ?? 0,
          outputTokens: payload.outputTokens ?? null,
          turnDurationMs: payload.turnDurationMs ?? null
        });
      });

      const completedUnlisten = await safeListen<TurnStreamEvent>("turn:completed", ({ payload }) => {
        if (this.activeTurnId !== payload.turnId) {
          return;
        }
        if (!this.shouldProcessTurnEvent(payload)) {
          return;
        }
        this.commitTurnEventCursor(payload);
        this.flushBufferedStreamText(payload.turnId);

        const completedPayloadRecord = payload as Record<string, unknown>;

        const assistantMessage = this.ensureAssistantMessage(
          payload.turnId,
          buildAssistantModelLabel(payload.providerName, payload.providerModel)
        );

        const finalText = payload.text?.trim();
        if (finalText) {
          assistantMessage.content = payload.text ?? assistantMessage.content;
        }
        assistantMessage.reasoningContent = normalizeReasoningContent(
          payload.reasoningContent ?? assistantMessage.reasoningContent ?? null
        );
        assistantMessage.status = "done";
        assistantMessage.modelName = buildAssistantModelLabel(payload.providerName, payload.providerModel);

        const completedPhase = resolveRuntimePhaseFromEvent(payload, "completed");
        this.phase = completedPhase === "completed" ? "ready" : completedPhase;
        this.traceSteps = payload.traceSteps ?? this.traceSteps;
        const terminalToolActivities = resolveTerminalToolActivities(payload.toolActivities, this.toolActivities);
        this.toolActivities = terminalToolActivities;
        this.sessionSummary = payload.sessionSummary ?? this.sessionSummary;
        this.providerRequestedName = payload.providerRequestedName ?? this.providerRequestedName;
        this.providerName = payload.providerName ?? this.providerName;
        this.providerProtocol = payload.providerProtocol ?? this.providerProtocol;
        this.providerModel = payload.providerModel ?? this.providerModel;
        this.providerSource = payload.providerSource ?? this.providerSource;
        this.providerMode = payload.providerMode ?? this.providerMode;
        this.fallbackReason = payload.fallbackReason ?? null;
        this.inputTokens = payload.inputTokens ?? this.inputTokens;
        this.outputTokens = payload.outputTokens ?? this.outputTokens;
        this.totalTokens = payload.totalTokens ?? this.totalTokens;
        this.firstTokenLatencyMs = payload.firstTokenLatencyMs ?? this.firstTokenLatencyMs;
        logCacheTelemetryContractViolations("completed", payload);
        const cacheHitInputTokens = resolveProviderReturnedCacheHitInputTokens(payload);
        const reasoningTokens = resolveReasoningTokens(payload);
        const cacheHitInputTokenPatch = cacheHitInputTokens != null ? { cacheHitInputTokens } : {};
        const reasoningTokenPatch = reasoningTokens != null ? { reasoningTokens } : {};
        const turnDurationPatch = payload.turnDurationMs != null ? { turnDurationMs: payload.turnDurationMs } : {};
        debugLog("cache-telemetry:terminal-payload", {
          terminalEvent: "completed",
          ...buildCacheTelemetryDebugSnapshot(payload)
        });
        const completedSessionId = this.sessionId;
        const completedRunId = this.activeRunId;
        const completedNodeId = this.visibleNodeId;
        this.applyTurnTokenStats(payload.turnId, payload.inputTokens, payload.outputTokens, false);
        this.syncToolMessages(payload.turnId, payload.toolActivities, false);
        this.isSubmitting = false;
        this.activeTurnId = null;
        runLowPriorityTurnWork(() => {
          const traceTimeline = resolveEventTraceTimeline(payload, () =>
            buildFallbackRuntimeTraceTimeline({
              turnId: payload.turnId,
              eventType: payload.eventType,
              messages: this.messages,
              phase: "completed",
              assistantMessage,
              toolActivities: terminalToolActivities,
              providerPatch: {
                providerName: payload.providerName ?? this.providerName,
                providerProtocol: payload.providerProtocol ?? this.providerProtocol,
                providerModel: payload.providerModel ?? this.providerModel,
                providerSource: payload.providerSource ?? this.providerSource,
                providerMode: payload.providerMode ?? this.providerMode
              },
              terminalState: "completed",
              fallbackReason: payload.fallbackReason ?? null,
              inputTokens: payload.inputTokens ?? this.inputTokens,
              cacheHitInputTokens,
              reasoningTokens,
              outputTokens: payload.outputTokens ?? this.outputTokens,
              totalTokens: payload.totalTokens ?? this.totalTokens,
              firstTokenLatencyMs: payload.firstTokenLatencyMs ?? this.firstTokenLatencyMs,
              turnDurationMs: payload.turnDurationMs ?? null
            })
          );
          this.commitTurnTraceTimeline(payload.turnId, traceTimeline, {
            eventId: payload.eventId ?? null,
            eventType: payload.eventType ?? null,
            eventVersion: payload.eventVersion ?? null,
            sequence: payload.sequence ?? null,
            emittedAtMs: payload.emittedAtMs ?? null,
            phase: "completed",
            traceSteps: payload.traceSteps ?? this.traceSteps,
            toolActivities: terminalToolActivities,
            providerCallRecords: cloneProviderCallRecords(payload.providerCallRecords),
            providerRequestedName: payload.providerRequestedName ?? this.providerRequestedName,
            providerName: payload.providerName ?? this.providerName,
            providerProtocol: payload.providerProtocol ?? this.providerProtocol,
            providerModel: payload.providerModel ?? this.providerModel,
            providerSource: payload.providerSource ?? this.providerSource,
            providerMode: payload.providerMode ?? this.providerMode,
            buildContextObservation: cloneBuildContextObservation(payload.buildContextObservation),
            hookTraceRecords: cloneHookTraceRecords(payload.hookTraceRecords),
            sessionSummary: payload.sessionSummary ?? this.sessionSummary,
            fallbackReason: payload.fallbackReason ?? null,
            inputTokens: payload.inputTokens ?? this.inputTokens,
            outputTokens: payload.outputTokens ?? this.outputTokens,
            totalTokens: payload.totalTokens ?? this.totalTokens,
            firstTokenLatencyMs: payload.firstTokenLatencyMs ?? this.firstTokenLatencyMs,
            ...cacheHitInputTokenPatch,
            ...reasoningTokenPatch,
            ...turnDurationPatch,
            error: null
          }, false);
          this.scheduleDeferredPersist();
          void this.loadSessionCatalog().catch(() => {});
          void this.loadRetrievedContextState(completedSessionId, {
            runId: completedRunId,
            nodeId: completedNodeId
          }).then((retrieved) => {
            if (this.sessionId === completedSessionId) {
              this.retrievedContext = retrieved;
            }
          }).catch(() => {});
        });
        debugLog("event:completed", {
          turnId: payload.turnId,
          inputTokens: payload.inputTokens ?? null,
          cacheHitInputTokensRaw:
            payload.cacheHitInputTokens ??
            readNestedNumericTokenValue(completedPayloadRecord, [["cache_hit_input_tokens"]]) ??
            readNestedNumericTokenValue(completedPayloadRecord, [["promptCacheHitTokens"]]) ??
            readNestedNumericTokenValue(completedPayloadRecord, [["prompt_cache_hit_tokens"]]) ??
            null,
          cacheHitInputTokensResolved: cacheHitInputTokens,
          reasoningTokensResolved: reasoningTokens,
          turnDurationMs: payload.turnDurationMs ?? null,
          finalTextLength: payload.text?.length ?? 0,
          messages: this.messages.length,
          traces: this.turnTraceHistory.length,
          traceCacheHitInputTokens:
            this.turnTraceHistory.find((turn) => turn.turnId === payload.turnId)?.cacheHitInputTokens ?? null
        });
      });

      const failedUnlisten = await safeListen<TurnStreamEvent>("turn:failed", ({ payload }) => {
        if (this.activeTurnId !== payload.turnId) {
          return;
        }
        if (!this.shouldProcessTurnEvent(payload)) {
          return;
        }
        this.commitTurnEventCursor(payload);
        this.flushBufferedStreamText(payload.turnId);

        const assistantMessage = this.ensureAssistantMessage(
          payload.turnId,
          buildAssistantModelLabel(payload.providerName, payload.providerModel)
        );

        assistantMessage.content = payload.text ?? DEFAULT_FAILED_TURN_MESSAGE;
        assistantMessage.reasoningContent = normalizeReasoningContent(payload.reasoningContent ?? null);
        assistantMessage.status = "error";
        assistantMessage.modelName = buildAssistantModelLabel(payload.providerName, payload.providerModel);

        this.phase = resolveRuntimePhaseFromEvent(payload, "failed");
        this.error = payload.error ?? DEFAULT_FAILED_TURN_ERROR;
        this.traceSteps = payload.traceSteps ?? this.traceSteps;
        const terminalToolActivities = resolveTerminalToolActivities(payload.toolActivities, this.toolActivities);
        this.toolActivities = terminalToolActivities;
        this.providerRequestedName = payload.providerRequestedName ?? this.providerRequestedName;
        this.providerName = payload.providerName ?? this.providerName;
        this.providerProtocol = payload.providerProtocol ?? this.providerProtocol;
        this.providerModel = payload.providerModel ?? this.providerModel;
        this.providerSource = payload.providerSource ?? this.providerSource;
        this.providerMode = payload.providerMode ?? this.providerMode;
        this.fallbackReason = payload.fallbackReason ?? this.fallbackReason;
        this.inputTokens = payload.inputTokens ?? this.inputTokens;
        this.outputTokens = payload.outputTokens ?? this.outputTokens;
        this.totalTokens = payload.totalTokens ?? this.totalTokens;
        this.firstTokenLatencyMs = payload.firstTokenLatencyMs ?? this.firstTokenLatencyMs;
        logCacheTelemetryContractViolations("failed", payload);
        const cacheHitInputTokens = resolveProviderReturnedCacheHitInputTokens(payload);
        const reasoningTokens = resolveReasoningTokens(payload);
        const cacheHitInputTokenPatch = cacheHitInputTokens != null ? { cacheHitInputTokens } : {};
        const reasoningTokenPatch = reasoningTokens != null ? { reasoningTokens } : {};
        const turnDurationPatch = payload.turnDurationMs != null ? { turnDurationMs: payload.turnDurationMs } : {};
        debugLog("cache-telemetry:terminal-payload", {
          terminalEvent: "failed",
          ...buildCacheTelemetryDebugSnapshot(payload)
        });
        const traceTimeline = resolveEventTraceTimeline(payload, () =>
          buildFallbackRuntimeTraceTimeline({
            turnId: payload.turnId,
            eventType: payload.eventType,
            messages: this.messages,
            phase: "failed",
            assistantMessage,
            toolActivities: terminalToolActivities,
            providerPatch: {
              providerName: payload.providerName ?? this.providerName,
              providerProtocol: payload.providerProtocol ?? this.providerProtocol,
              providerModel: payload.providerModel ?? this.providerModel,
              providerSource: payload.providerSource ?? this.providerSource,
              providerMode: payload.providerMode ?? this.providerMode
            },
            terminalState: "error",
            fallbackReason: payload.fallbackReason ?? this.fallbackReason,
            error: payload.error ?? DEFAULT_FAILED_TURN_ERROR,
            inputTokens: payload.inputTokens ?? this.inputTokens,
            cacheHitInputTokens,
            reasoningTokens,
            outputTokens: payload.outputTokens ?? this.outputTokens,
            totalTokens: payload.totalTokens ?? this.totalTokens,
            firstTokenLatencyMs: payload.firstTokenLatencyMs ?? this.firstTokenLatencyMs,
            turnDurationMs: payload.turnDurationMs ?? null
          })
        );
        this.applyTurnTokenStats(payload.turnId, payload.inputTokens, payload.outputTokens);
        this.syncToolMessages(payload.turnId, payload.toolActivities);
        this.commitTurnTraceTimeline(payload.turnId, traceTimeline, {
          eventId: payload.eventId ?? null,
          eventType: payload.eventType ?? null,
          eventVersion: payload.eventVersion ?? null,
          sequence: payload.sequence ?? null,
          emittedAtMs: payload.emittedAtMs ?? null,
          phase: "failed",
          traceSteps: payload.traceSteps ?? this.traceSteps,
          toolActivities: terminalToolActivities,
          providerCallRecords: cloneProviderCallRecords(payload.providerCallRecords),
          providerRequestedName: payload.providerRequestedName ?? this.providerRequestedName,
          providerName: payload.providerName ?? this.providerName,
          providerProtocol: payload.providerProtocol ?? this.providerProtocol,
          providerModel: payload.providerModel ?? this.providerModel,
          providerSource: payload.providerSource ?? this.providerSource,
          providerMode: payload.providerMode ?? this.providerMode,
          buildContextObservation: cloneBuildContextObservation(payload.buildContextObservation),
          hookTraceRecords: cloneHookTraceRecords(payload.hookTraceRecords),
          fallbackReason: payload.fallbackReason ?? this.fallbackReason,
          inputTokens: payload.inputTokens ?? this.inputTokens,
          outputTokens: payload.outputTokens ?? this.outputTokens,
          totalTokens: payload.totalTokens ?? this.totalTokens,
          firstTokenLatencyMs: payload.firstTokenLatencyMs ?? this.firstTokenLatencyMs,
          ...cacheHitInputTokenPatch,
          ...reasoningTokenPatch,
          ...turnDurationPatch,
          error: payload.error ?? DEFAULT_FAILED_TURN_ERROR
        });
        this.persistHistory();
        void this.loadRetrievedContextState(this.sessionId, {
          runId: this.activeRunId,
          nodeId: this.visibleNodeId
        }).then((retrieved) => {
          this.retrievedContext = retrieved;
        });
        debugLog("event:failed", {
          turnId: payload.turnId,
          error: this.error
        });
        this.isSubmitting = false;
        this.activeTurnId = null;
      });

      const cancelledUnlisten = await safeListen<TurnStreamEvent>("turn:cancelled", ({ payload }) => {
        if (this.activeTurnId !== payload.turnId) {
          return;
        }
        if (!this.shouldProcessTurnEvent(payload)) {
          return;
        }
        this.commitTurnEventCursor(payload);
        this.flushBufferedStreamText(payload.turnId);

        const cancelledTraceSteps = finalizeCancelledTraceSteps(payload.traceSteps ?? this.traceSteps);
        logCacheTelemetryContractViolations("cancelled", payload);
        const cancelledCacheHitInputTokens = resolveProviderReturnedCacheHitInputTokens(payload);
        const cancelledReasoningTokens = resolveReasoningTokens(payload);
        debugLog("cache-telemetry:terminal-payload", {
          terminalEvent: "cancelled",
          ...buildCacheTelemetryDebugSnapshot(payload)
        });

        const assistantMessage = this.ensureAssistantMessage(
          payload.turnId,
          buildAssistantModelLabel(payload.providerName, payload.providerModel)
        );

        assistantMessage.content = payload.text ?? "本轮已停止。";
        assistantMessage.reasoningContent = normalizeReasoningContent(payload.reasoningContent ?? null);
        assistantMessage.status = "done";
        assistantMessage.modelName = buildAssistantModelLabel(payload.providerName, payload.providerModel);

        this.phase = resolveRuntimePhaseFromEvent(payload, "cancelled");
        this.error = null;
        this.traceSteps = cancelledTraceSteps;
        const terminalToolActivities = resolveTerminalToolActivities(payload.toolActivities, this.toolActivities);
        this.toolActivities = terminalToolActivities;
        this.providerRequestedName = payload.providerRequestedName ?? this.providerRequestedName;
        this.providerName = payload.providerName ?? this.providerName;
        this.providerProtocol = payload.providerProtocol ?? this.providerProtocol;
        this.providerModel = payload.providerModel ?? this.providerModel;
        this.providerSource = payload.providerSource ?? this.providerSource;
        this.providerMode = payload.providerMode ?? this.providerMode;
        this.fallbackReason = payload.fallbackReason ?? this.fallbackReason;
        const cacheHitInputTokenPatch = cancelledCacheHitInputTokens != null ? { cacheHitInputTokens: cancelledCacheHitInputTokens } : {};
        const reasoningTokenPatch = cancelledReasoningTokens != null ? { reasoningTokens: cancelledReasoningTokens } : {};
        const turnDurationPatch = payload.turnDurationMs != null ? { turnDurationMs: payload.turnDurationMs } : {};
        const traceTimeline = resolveEventTraceTimeline(payload, () =>
          buildFallbackRuntimeTraceTimeline({
            turnId: payload.turnId,
            eventType: payload.eventType,
            messages: this.messages,
            phase: "cancelled",
            assistantMessage,
            toolActivities: terminalToolActivities,
            providerPatch: {
              providerName: payload.providerName ?? this.providerName,
              providerProtocol: payload.providerProtocol ?? this.providerProtocol,
              providerModel: payload.providerModel ?? this.providerModel,
              providerSource: payload.providerSource ?? this.providerSource,
              providerMode: payload.providerMode ?? this.providerMode
            },
            terminalState: "cancelled",
            fallbackReason: payload.fallbackReason ?? this.fallbackReason,
            error: payload.error ?? "stopped_by_user",
            inputTokens: payload.inputTokens ?? null,
            cacheHitInputTokens: cancelledCacheHitInputTokens,
            reasoningTokens: cancelledReasoningTokens,
            outputTokens: payload.outputTokens ?? null,
            totalTokens: payload.totalTokens ?? null,
            firstTokenLatencyMs: payload.firstTokenLatencyMs ?? this.firstTokenLatencyMs,
            turnDurationMs: payload.turnDurationMs ?? null
          })
        );
        this.applyTurnTokenStats(payload.turnId, payload.inputTokens, payload.outputTokens);
        this.syncToolMessages(payload.turnId, payload.toolActivities);
        this.commitTurnTraceTimeline(payload.turnId, traceTimeline, {
          eventId: payload.eventId ?? null,
          eventType: payload.eventType ?? null,
          eventVersion: payload.eventVersion ?? null,
          sequence: payload.sequence ?? null,
          emittedAtMs: payload.emittedAtMs ?? null,
          phase: "cancelled",
          traceSteps: cancelledTraceSteps,
          toolActivities: terminalToolActivities,
          providerCallRecords: cloneProviderCallRecords(payload.providerCallRecords),
          providerRequestedName: payload.providerRequestedName ?? this.providerRequestedName,
          providerName: payload.providerName ?? this.providerName,
          providerProtocol: payload.providerProtocol ?? this.providerProtocol,
          providerModel: payload.providerModel ?? this.providerModel,
          providerSource: payload.providerSource ?? this.providerSource,
          providerMode: payload.providerMode ?? this.providerMode,
          buildContextObservation: cloneBuildContextObservation(payload.buildContextObservation),
          hookTraceRecords: cloneHookTraceRecords(payload.hookTraceRecords),
          fallbackReason: payload.fallbackReason ?? this.fallbackReason,
          ...cacheHitInputTokenPatch,
          ...reasoningTokenPatch,
          ...turnDurationPatch,
          error: payload.error ?? "stopped_by_user"
        });
        this.persistHistory();
        void this.loadRetrievedContextState(this.sessionId, {
          runId: this.activeRunId,
          nodeId: this.visibleNodeId
        }).then((retrieved) => {
          this.retrievedContext = retrieved;
        });
        debugLog("event:cancelled", {
          turnId: payload.turnId
        });
        this.isSubmitting = false;
        this.activeTurnId = null;
      });

      void startedUnlisten;
      void deltaUnlisten;
      void traceUnlisten;
      void phaseChangedUnlisten;
      void checkpointPersistedUnlisten;
      void toolUnlisten;
      void outputEndUnlisten;
      void completedUnlisten;
      void failedUnlisten;
      void cancelledUnlisten;
      this.eventsReady = true;
    },
    async runBrowserPreviewTurn(requestId: string) {
      const providerStore = useProviderStore();
      const provider = providerStore.currentProvider;
      const model = providerStore.currentModel;
      this.browserPreviewRunToken += 1;
      const runToken = this.browserPreviewRunToken;
      const previewProviderName = provider?.name ?? BROWSER_PREVIEW_PROVIDER_NAME;
      const previewModelName = model?.model ?? model?.name ?? BROWSER_PREVIEW_MODEL_NAME;
      const assistantModelLabel = buildAssistantModelLabel(previewProviderName, previewModelName);

      this.providerRequestedName = previewProviderName;
      debugLog("browser-preview:start", {
        turnId: requestId
      });
      this.providerName = previewProviderName;
      this.providerProtocol = provider?.protocol ?? "openai";
      this.providerModel = previewModelName;
      this.providerSource = "browser_preview";
      this.providerMode = "browser_preview";
      this.inputTokens = null;
      this.outputTokens = null;
      this.totalTokens = null;
      this.firstTokenLatencyMs = null;
      this.fallbackReason = BROWSER_PREVIEW_FALLBACK_REASON;

      await wait(120);
      if (runToken !== this.browserPreviewRunToken || this.activeTurnId !== requestId) {
        return;
      }
      const assistantMessage = this.ensureAssistantMessage(requestId, assistantModelLabel);
      assistantMessage.content = "";

      for (const chunk of BROWSER_PREVIEW_CHUNKS) {
        await wait(80);
        if (runToken !== this.browserPreviewRunToken || this.activeTurnId !== requestId) {
          return;
        }
        assistantMessage.content += chunk;
        this.scheduleDeferredPersist();
      }

      assistantMessage.status = "done";
      assistantMessage.modelName = assistantModelLabel;
      assistantMessage.tokenCount = null;

      this.phase = "completed";
      this.sessionSummary = BROWSER_PREVIEW_SESSION_SUMMARY;
      this.traceSteps = createBrowserPreviewTraceSteps();
      this.traceTimeline = createBrowserPreviewTraceTimeline();
      this.toolActivities = [];
      const terminalSequence = this.traceTimeline[this.traceTimeline.length - 1]?.sequence ?? this.traceSteps.length;
      const terminalEnvelope = createBrowserPreviewTerminalEnvelope(
        requestId,
        "turn.completed",
        terminalSequence,
        Date.now()
      );
      this.commitTurnEventCursor({
        turnId: requestId,
        eventId: terminalEnvelope.eventId,
        sequence: terminalEnvelope.sequence,
        emittedAtMs: terminalEnvelope.emittedAtMs
      });
      this.commitTurnTraceTimeline(requestId, this.traceTimeline, {
        eventId: terminalEnvelope.eventId,
        eventType: terminalEnvelope.eventType,
        eventVersion: terminalEnvelope.eventVersion,
        sequence: terminalEnvelope.sequence,
        emittedAtMs: terminalEnvelope.emittedAtMs,
        phase: "completed",
        traceSteps: this.traceSteps,
        toolActivities: [],
        sessionSummary: this.sessionSummary,
        fallbackReason: this.fallbackReason,
        title:
          this.messages.find((item) => item.turnId === requestId && item.role === "user")?.content
            ? buildTurnTitle(this.messages.find((item) => item.turnId === requestId && item.role === "user")?.content ?? "")
            : BROWSER_PREVIEW_TRACE_TITLE,
        error: null
      });
      this.persistHistory();
      void this.loadSessionCatalog();
      debugLog("browser-preview:completed", {
        turnId: requestId
      });
      this.isSubmitting = false;
      this.activeTurnId = null;
      this.activeRunId = null;
    },
    async stopTurn() {
      if (!this.activeTurnId || !this.isSubmitting) {
        return false;
      }

      if (!isTauriAvailable()) {
        return this.cancelBrowserPreviewTurn(this.activeTurnId);
      }

      try {
        if (this.activeRunId) {
          const response = await safeInvoke<GraphRunControlResponse>("stop_graph_run", {
            runId: this.activeRunId
          });
          this.latestGraphRunControlBoundaryEvidence = cloneGraphRunControlBoundaryEvidence(
            response.controlBoundaryEvidence ? [response.controlBoundaryEvidence] : null
          );
          this.latestRunControlAuditSummary = cloneRunControlAuditSummary(
            response.runControlAuditSummary ?? null
          );
        } else {
          await safeInvoke("stop_turn", {
            turnId: this.activeTurnId
          });
        }
        return true;
      } catch (error) {
        this.error = `停止当前轮次失败：${String(error)}`;
        return false;
      }
    },
    async submitTurn(options?: { images?: TurnInputImage[] }) {
      await this.initializeTurnEvents();
      const providerStore = useProviderStore();
      const images = (options?.images ?? []).map((image) => ({ ...image }));
      const message = this.draftMessage.trim();
      const providerMessage = buildProviderUserMessage(message, images);
      const displayMessage = buildDisplayedUserMessage(message, images);
      const payload: TurnInput = {
        message: providerMessage,
        displayMessage,
        providerId: providerStore.currentProvider?.id ?? null,
        modelId: providerStore.currentModel?.id ?? null,
        reasoningEffort: providerStore.currentReasoningEffort ?? null,
        sessionId: this.sessionId,
        nodeId: this.visibleNodeId,
        history: buildTurnHistory(this.messages),
        images
      };

      if (!payload.message.trim() && !images.length) {
        return false;
      }

      const requestId = String(Date.now());
      const userMessageId = `user-${requestId}`;

      this.messages.push({
        id: userMessageId,
        turnId: requestId,
        role: "user",
        content: displayMessage,
        attachments: images.map((image, index) => ({
          id: `pending-${requestId}-${index + 1}`,
          name: image.name ?? null,
          mimeType: image.mimeType,
          relativePath: null,
          sizeBytes: image.dataUrl.length,
          createdAtMs: Date.now()
        })),
        status: "done",
        tokenCount: null
      });
      this.persistHistory();
      debugLog("submit", {
        turnId: requestId,
        messageLength: providerMessage.length,
        imageCount: images.length,
        messages: this.messages.length
      });

      this.isSubmitting = true;
      this.error = null;
      this.phase = "calling_model";
      this.activeTurnId = requestId;
      this.draftMessage = "";
      this.inputTokens = null;
      this.outputTokens = null;
      this.totalTokens = null;
      this.firstTokenLatencyMs = null;
      this.traceSteps = createSubmitTraceSteps();
      this.traceTimeline = createDefaultTraceTimeline();
      this.toolActivities = [];
      this.commitTurnTraceTimeline(requestId, this.traceTimeline, {
        phase: "calling_model",
        traceSteps: this.traceSteps,
        toolActivities: [],
        error: null
      });

      try {
        await waitForNextPaint();
        debugLog("submit:user-painted", {
          turnId: requestId,
          messages: this.messages.length
        });

        if (!isTauriAvailable()) {
          await this.runBrowserPreviewTurn(requestId);
          return true;
        }

        let submission: GraphRunSubmission | null = null;
        if (this.sessionId) {
          try {
            const plan = await safeInvoke<GraphRunSubmissionPlan>("resolve_graph_run_submission_plan", {
              sessionId: this.sessionId,
              nodeId: this.visibleNodeId,
              runId: this.activeRunId
            });
            submission = resolveGraphRunSubmissionFromPlan(plan);
          } catch (planError) {
            debugLog("submit:resolve-plan:error", {
              sessionId: this.sessionId,
              runId: this.activeRunId,
              error: String(planError)
            });
          }
        }

        if (!submission) {
          submission = resolveGraphRunSubmissionFromPlan(this.latestGraphRunSubmissionPlan);
        }

        if (!submission) {
          submission = resolveGraphRunSubmissionFromRunState(this.retrievedContext?.runState);
          submission = reconcileSubmissionWithRecoveryCheckpoint(submission, this.latestExecutionCheckpoint);
          submission ??= resolveGraphRunSubmissionFromCheckpoint(
            this.latestExecutionCheckpoint,
            this.activeRunId
          );
        }

        if (!submission && this.sessionId) {
          await this.resolveDerivedSessionRun({
            sessionId: this.sessionId,
            runId: this.activeRunId,
            preferRefresh: true,
            nodeId: this.visibleNodeId
          });
          submission = resolveGraphRunSubmissionFromRunState(this.retrievedContext?.runState);
          submission = reconcileSubmissionWithRecoveryCheckpoint(submission, this.latestExecutionCheckpoint);
          submission ??= resolveGraphRunSubmissionFromCheckpoint(
            this.latestExecutionCheckpoint,
            this.activeRunId
          );
        }
        submission ??= { command: "start_graph_run_stream" as const, runId: null };

        if (submission.command === "start_graph_run_stream") {
          const response = await safeInvoke<GraphRunStreamStartResponse>("start_graph_run_stream", {
            turnId: requestId,
            runId: null,
            goal: displayMessage,
            input: payload
          });
          this.activeRunId = resolvedStreamStartRunId(response, null);
          this.latestGraphRunControlBoundaryEvidence = cloneGraphRunControlBoundaryEvidence(
            response.controlBoundaryEvidence ? [response.controlBoundaryEvidence] : null
          );
          this.latestRunControlAuditSummary = cloneRunControlAuditSummary(
            response.runControlAuditSummary ?? null
          );
          return true;
        }

        if (submission.command === "resume_graph_run_stream") {
          const response = await safeInvoke<GraphRunStreamStartResponse>("resume_graph_run_stream", {
            turnId: requestId,
            runId: submission.runId,
            input: payload
          });
          this.activeRunId = resolvedStreamStartRunId(response, submission.runId);
          this.latestGraphRunControlBoundaryEvidence = cloneGraphRunControlBoundaryEvidence(
            response.controlBoundaryEvidence ? [response.controlBoundaryEvidence] : null
          );
          this.latestRunControlAuditSummary = cloneRunControlAuditSummary(
            response.runControlAuditSummary ?? null
          );
          return true;
        }

        const response = await safeInvoke<GraphRunStreamStartResponse>("continue_graph_run_stream", {
          turnId: requestId,
          runId: submission.runId,
          input: payload
        });
        this.activeRunId = resolvedStreamStartRunId(response, submission.runId);
        this.latestGraphRunControlBoundaryEvidence = cloneGraphRunControlBoundaryEvidence(
          response.controlBoundaryEvidence ? [response.controlBoundaryEvidence] : null
        );
        this.latestRunControlAuditSummary = cloneRunControlAuditSummary(
          response.runControlAuditSummary ?? null
        );
        return true;
      } catch (error) {
        const assistantMessage = this.ensureAssistantMessage(
          requestId,
          buildAssistantModelLabel(
            providerStore.currentProvider?.name ?? null,
            providerStore.currentModel?.model ?? providerStore.currentModel?.name ?? null
          )
        );
        assistantMessage.content = DEFAULT_FAILED_TURN_MESSAGE;
        assistantMessage.reasoningContent = null;
        assistantMessage.status = "error";
        assistantMessage.modelName = buildAssistantModelLabel(
          providerStore.currentProvider?.name ?? null,
          providerStore.currentModel?.model ?? providerStore.currentModel?.name ?? null
        );
        this.error = `本轮执行失败：${String(error)}`;
        this.phase = "failed";
        this.activeTurnId = null;
        this.activeRunId = null;
        this.traceSteps = createSubmitFailureTraceSteps();
        this.traceTimeline = createSubmitFailureTraceTimeline();
        this.commitTurnTraceTimeline(requestId, this.traceTimeline, {
          phase: "failed",
          traceSteps: this.traceSteps,
          toolActivities: this.toolActivities,
          error: this.error
        });
        this.persistHistory();
        return false;
      } finally {
        if (this.phase === "failed") {
          this.isSubmitting = false;
        }
      }
    }
  }
});
