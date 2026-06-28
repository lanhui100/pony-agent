import { defineStore } from "pinia";
import { isTauriAvailable, safeInvoke, safeListen } from "@/lib/tauri";
import { initFrontendFlightRecorder } from "@/lib/frontend-flight-recorder";
import { useProviderStore } from "@/stores/providers";
import { useSettingsStore } from "@/stores/settings";
import { deriveGraphRunFromRunState, extractActiveTaskFocus, normalizeGraphRunPhase } from "../types/runtime";
import { isRetryableError } from "@/lib/error-utils";
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

const MAX_BG_TEXT_BUFFER_CHARS = 50000;
const MAX_BG_REASONING_BUFFER_CHARS = 20000;

type RunningTurn = {
  turnId: string;
  phase: RuntimePhase;
  textBuffer: string;
  reasoningBuffer: string;
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
  sessionHydrating: boolean;
  deletingSessionSet: Record<string, boolean>;
  sessionSwitchToken: number;
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
  cursorVersion: number | null;
  historyCursorMode: HistoryCursorMode;
  historyNodes: HistoryNode[];
  historyBranches: HistoryBranch[];
  eventsReady: boolean;
  deferredPersistTimerId: number | null;
  streamFlushFrameId: number | null;
  streamFlushTimerId: number | null;
  streamBufferTurnId: string | null;
  streamBufferText: string;
  streamBufferReasoning: string;
  browserPreviewRunToken: number;
  initialRollbackActive: boolean;
  runningSessionMap: Record<string, RunningTurn>;
  completedSessionSet: Record<string, boolean>;
  failedSessionSet: Record<string, boolean>;
  streamDebugDeltaCount: number;
  streamDebugFlushCount: number;
  streamDebugTextCharsReceived: number;
  streamDebugTextCharsFlushed: number;
};

type PersistedRuntimeState = {
  cachedStateVersion: number;
  phase: RuntimePhase;
  canonicalTerminalPhase?: "completed" | "failed" | "cancelled";
  messages: ChatMessage[];
  attachmentAssets: AttachmentAsset[];
  // turnTraceHistory is no longer persisted to localStorage — trace data is
  // already stored on the backend (sessions.json) and loaded via Tauri commands.
  // The field remains optional for backward-compatible reads of old cache data.
  turnTraceHistory?: TurnTraceRecord[];
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
  branchHeadNodeId?: string | null;
  activeBranchId?: string | null;
  historyCursorMode?: HistoryCursorMode;
  historyNodes?: HistoryNode[];
  historyBranches?: HistoryBranch[];
  visibleNodeId?: string | null;
  initialRollbackActive?: boolean;
  checkpoint?: ExecutionCheckpoint | null;
  runningTurnId?: string | null;
};

type SessionRuntimeSnapshot = {
  sessionId: string;
  sessionList: SessionOverview[];
  deletingSessionSet: Record<string, boolean>;
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
  cursorVersion: number | null;
  historyCursorMode: HistoryCursorMode;
  historyNodes: HistoryNode[];
  historyBranches: HistoryBranch[];
  initialRollbackActive: boolean;
  messages: ChatMessage[];
  attachmentAssets: AttachmentAsset[];
  toolActivities: ToolActivity[];
  traceSteps: TraceStep[];
  traceTimeline: TraceTimelineEntry[];
  turnTraceHistory: TurnTraceRecord[];
  eventCursorByTurnId: Record<string, { eventId: string | null; sequence: number | null; emittedAtMs: number | null }>;
};

const RUNTIME_STORAGE_KEY = "pony-agent.runtime-history.v1";
const CACHED_STATE_VERSION = 2;
const DEFAULT_SESSION_ID = "local-dev-session";

type PersistedRuntimeCache = {
  sessions: Record<string, PersistedRuntimeState>;
  runningSessionMap?: Record<string, { turnId: string; phase: RuntimePhase; textBuffer?: string; reasoningBuffer?: string }>;
  completedSet?: Record<string, boolean>;
  failedSet?: Record<string, boolean>;
};

type SessionInitializationStrategy =
  | { kind: "local-cache"; persistedState: PersistedRuntimeState }
  | { kind: "host-read"; sessionId: string; reason: "no-cache" | "insufficient-checkpoint" }
  | { kind: "empty-fallback"; sessionId: string };

const DEFAULT_BROWSER_SESSION_SUMMARY = "浏览器预览会话";
const DEFAULT_FAILED_TURN_MESSAGE = "本轮执行失败，请查看右侧 trace。";
const DEFAULT_FAILED_TURN_ERROR = "本轮执行失败。";
const TIMEOUT_RETRY_PENDING_MESSAGE = "超时后错误重连中...";
const HYDRATION_TIMEOUT_MS = 15000;

function withTimeout<T>(promise: Promise<T>, ms: number, label: string): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const timer = setTimeout(() => {
      reject(new Error(`${label} 超时 (${ms}ms)`));
    }, ms);
    promise.then(
      (value) => {
        clearTimeout(timer);
        resolve(value);
      },
      (error) => {
        clearTimeout(timer);
        reject(error);
      }
    );
  });
}

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
  if (typeof window !== "undefined" && window.localStorage.getItem("pony-agent.debug.runtime-logs") !== "true") {
    return;
  }
  const message = {
    event,
    payload: payload ?? {},
    ts: new Date().toISOString()
  };
  console.info(`[pony-agent][runtime] ${JSON.stringify(message)}`);
}

function errorLog(event: string, payload?: Record<string, unknown>) {
  if (typeof window !== "undefined" && window.localStorage.getItem("pony-agent.debug.runtime-logs") !== "true") {
    return;
  }
  const message = {
    event,
    payload: payload ?? {},
    ts: new Date().toISOString()
  };
  console.error(`[pony-agent][runtime] ${JSON.stringify(message)}`);
}

function reportSwitchPerf(stage: string, payload: Record<string, unknown>) {
  if (typeof window === "undefined") {
    return;
  }

  const perfWindow = window as Window & {
    __ponySwitchPerf?: Array<Record<string, unknown>>;
  };
  const entry = {
    stage,
    at: Date.now(),
    ...payload
  };
  perfWindow.__ponySwitchPerf = [...(perfWindow.__ponySwitchPerf ?? []), entry].slice(-50);
  const elapsedMs = typeof payload.elapsedMs === "number" ? payload.elapsedMs : 0;
  if (elapsedMs >= 120) {
    console.warn("[pony-agent][perf] session-switch", entry);
  }
}

async function measureHostRead<T>(
  label: string,
  payload: Record<string, unknown>,
  run: () => Promise<T>
): Promise<T> {
  const startedAt = typeof performance !== "undefined" ? performance.now() : Date.now();
  try {
    const result = await run();
    const elapsedMs = (typeof performance !== "undefined" ? performance.now() : Date.now()) - startedAt;
    if (elapsedMs >= 120) {
      console.warn("[pony-agent][perf] host-read", {
        label,
        elapsedMs,
        ...payload
      });
    }
    return result;
  } catch (error) {
    const elapsedMs = (typeof performance !== "undefined" ? performance.now() : Date.now()) - startedAt;
    console.warn("[pony-agent][perf] host-read-error", {
      label,
      elapsedMs,
      error: String(error),
      ...payload
    });
    throw error;
  }
}

const STREAM_FLUSH_INTERVAL_MS = 16;

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
    // buildContextObservation is intentionally stripped here — it contains the
    // full prompt text (often 50-100KB+) per entry. The backend already persists
    // it, and the frontend never reads it back from stored timeline entries.
    // Keeping it would cause N × deep-clone on every terminal event, blocking
    // the main thread and freezing the page at turn end.
    buildContextObservation: null,
    toolActivities: cloneToolActivities(entry.toolActivities)
  }));

  const folded: TraceTimelineEntry[] = [];
  let lastModelIndex = -1;
  for (const entry of normalized) {
    if (entry.kind !== "return_result") {
      folded.push(entry);
      if (entry.kind === "call_model") {
        lastModelIndex = folded.length - 1;
      }
      continue;
    }

    if (lastModelIndex === -1) {
      folded.push({
        ...entry,
        id: `model-${entry.sequence}`,
        kind: "call_model",
        label: "CALL MODEL #1"
      });
      lastModelIndex = folded.length - 1;
      continue;
    }

    const modelEntry = folded[lastModelIndex];
    folded[lastModelIndex] = {
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
    artifacts: tool.artifacts ? tool.artifacts.map((artifact) => ({ ...artifact })) : null,
    error: tool.error ? { ...tool.error } : null,
    capabilityInvocation: tool.capabilityInvocation
      ? {
          ...tool.capabilityInvocation,
          permissionFacts: tool.capabilityInvocation.permissionFacts
            ? { ...tool.capabilityInvocation.permissionFacts }
            : null
        }
      : null
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
        text: parentTool.description ?? null,
        error: parentTool.status === "error" ? parentTool.description : null
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

function hasHostManagedHistoryState(runtimeView?: Pick<SessionRuntimeView, "historyCursor" | "authorityMode"> | null) {
  return (runtimeView?.authorityMode ?? runtimeView?.historyCursor?.authorityMode ?? "host_authoritative") === "host_authoritative";
}

function previewHistoryMutationUnavailable() {
  return "当前为 browser preview / local preview 降级模式，不支持 branch / restore / fork 等正式宿主历史控制动作。";
}

function historyCursorVersion(cursor?: Pick<HistoryCursorState, "cursorVersion"> | null) {
  return typeof cursor?.cursorVersion === "number" && Number.isFinite(cursor.cursorVersion)
    ? cursor.cursorVersion
    : null;
}

function resolveRuntimeViewHistoryProjection(
  runtimeView?:
    | Pick<
        SessionRuntimeView,
        | "historyCursor"
        | "resolvedVisibleNodeId"
        | "activeBranchHeadNodeId"
        | "isAtBranchHead"
        | "historyNodes"
        | "historyBranches"
      >
    | null
) {
  const runtimeHistoryCursor = cloneHistoryCursor(runtimeView?.historyCursor);
  if (runtimeHistoryCursor) {
    return runtimeHistoryCursor;
  }

  const visibleNodeId = runtimeView?.resolvedVisibleNodeId?.trim() || null;
  const activeBranchId =
    runtimeView?.historyBranches?.find((branch) => branch.headNodeId === runtimeView?.activeBranchHeadNodeId)?.branchId ?? null;
  const branchHeadNodeId = runtimeView?.activeBranchHeadNodeId?.trim() || null;
  if (!visibleNodeId && !branchHeadNodeId && !activeBranchId) {
    return null;
  }

  return {
    sessionId: "",
    visibleNodeId,
    activeBranchId,
    branchHeadNodeId,
    workspaceNodeId: visibleNodeId,
    mode: normalizeHistoryCursorMode(
      runtimeView?.isAtBranchHead === false ? "historical" : "live"
    )
  } satisfies Partial<HistoryCursorState>;
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
  const blocks = [tool.description.trim()];

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

const STREAM_FLUSH_EAGER_CHARS = 50;

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
    // buildContextObservation is intentionally not stored on timeline entries —
    // it contains the full prompt text (50-100KB+) and cloneTraceTimeline strips
    // it anyway. The top-level observation on TurnTraceRecord is the canonical source.
    buildContextObservation: null,
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
    // Caller (normalizeTurnTraceRecord) wraps this in cloneTraceTimeline,
    // so we return the raw reference here to avoid a redundant deep clone.
    return turn.traceTimeline;
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
  return new Promise<void>((resolve) => safeSetTimeout(() => resolve(), ms));
}

function waitForNextPaint() {
  return new Promise<void>((resolve) => {
    const browserWindow = resolveBrowserWindow();
    if (!browserWindow || typeof browserWindow.requestAnimationFrame !== "function") {
      resolve();
      return;
    }
    browserWindow.requestAnimationFrame(() => resolve());
  });
}

const LOW_PRIORITY_TURN_WORK_DELAY_MS = 800;
const LOW_PRIORITY_TURN_WORK_IDLE_TIMEOUT_MS = 2500;
const OUTPUT_END_PERSIST_DELAY_MS = 1200;

function runLowPriorityTurnWork(callback: () => void) {
  const browserWindow = resolveBrowserWindow();
  if (!browserWindow || typeof browserWindow.requestAnimationFrame !== "function") {
    callback();
    return;
  }

  safeSetTimeout(() => {
    const idleWindow = resolveBrowserWindow();
    if (!idleWindow) {
      callback();
      return;
    }

    const requestIdleCallback = (idleWindow as Window & {
      requestIdleCallback?: (handler: IdleRequestCallback, options?: IdleRequestOptions) => number;
    }).requestIdleCallback;

    if (typeof requestIdleCallback === "function") {
      requestIdleCallback(() => callback(), {
        timeout: LOW_PRIORITY_TURN_WORK_IDLE_TIMEOUT_MS
      });
      return;
    }

    safeSetTimeout(callback, 0);
  }, LOW_PRIORITY_TURN_WORK_DELAY_MS);
}

function resolveBrowserWindow(): Window | null {
  return typeof window === "undefined" ? null : window;
}

function safeSetTimeout(callback: () => void, delay: number): ReturnType<typeof setTimeout> {
  const browserWindow = resolveBrowserWindow();
  if (browserWindow) {
    return browserWindow.setTimeout(callback, delay);
  }

  return globalThis.setTimeout(callback, delay);
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
    deletingSessionSet: { ...state.deletingSessionSet },
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
    cursorVersion: state.cursorVersion,
    historyCursorMode: state.historyCursorMode,
    historyNodes: cloneHistoryNodes(state.historyNodes),
    historyBranches: cloneHistoryBranches(state.historyBranches),
    initialRollbackActive: state.initialRollbackActive,
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
  if (state.streamFlushTimerId != null) {
    window.clearTimeout(state.streamFlushTimerId);
  }
  state.sessionId = snapshot.sessionId;
  state.sessionList = snapshot.sessionList.map((session) => ({ ...session }));
  state.deletingSessionSet = { ...snapshot.deletingSessionSet };
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
  state.streamFlushFrameId = null;
  state.streamFlushTimerId = null;
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
  state.initialRollbackActive = snapshot.initialRollbackActive;
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
}

function filterDeletingSessions(
  sessions: SessionOverview[],
  deletingSessionSet: Record<string, boolean>
): SessionOverview[] {
  return sessions.filter((session) => !deletingSessionSet[session.conversationId]);
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
    name: "Run",
    canonicalToolName: "Run",
    executionPrimitive: "workspace_run_command",
    description: "在当前工作区内受控执行命令，并委托到内部 RunShell 执行能力；稳定返回 cwd、timeout、exitCode、stdout 和 stderr。",
    kind: "execute",
    exposure: "model_visible",
    displayMetadata: { displayNameZh: "运行" },
    permissionFacts: {
      requiresApproval: false,
      permissionScope: "workspace.execute",
      hostMediated: false,
      approvalMode: "none",
      decisionSource: "runtime",
      permissionProfile: "builtin"
    },
    inputSchema: {
      type: "object",
      properties: {
        command: {
          type: "string",
          description: "要执行的命令文本"
        },
        cwd: {
          type: "string",
          description: "执行命令时的工作区内相对目录，默认 ."
        },
        timeoutMs: {
          type: "integer",
          description: "命令超时毫秒数，默认 10000，最大 120000"
        }
      },
      required: ["command"],
      additionalProperties: false
    }
  },
  {
    name: "Ask",
    canonicalToolName: "Ask",
    executionPrimitive: "echo_input",
    description: "向用户或宿主请求澄清、确认或补充输入；无宿主中介时会回落为受控澄清提示。",
    kind: "interactive",
    exposure: "model_visible",
    displayMetadata: { displayNameZh: "提问" },
    permissionFacts: {
      requiresApproval: false,
      approvalMode: "none",
      decisionSource: "runtime",
      permissionProfile: "builtin"
    },
    inputSchema: {
      type: "object",
      properties: {
        text: {
          type: "string",
          description: "需要向用户展示或确认的文本"
        },
        question: {
          type: "string",
          description: "当 text 缺失时，用于 fallback 的澄清问题"
        }
      },
      additionalProperties: false
    }
  },
  {
    name: "Read",
    canonicalToolName: "Read",
    executionPrimitive: "workspace_gather_context",
    description: "围绕一个路径自动聚合上下文，适合默认读取文件、目录和局部线索的首选入口。",
    kind: "read",
    exposure: "model_visible",
    displayMetadata: { displayNameZh: "读取" },
    permissionFacts: {
      requiresApproval: false,
      permissionScope: "workspace.read",
      hostMediated: false,
      approvalMode: "none",
      decisionSource: "runtime",
      permissionProfile: "builtin"
    },
    inputSchema: {
      type: "object",
      properties: {
        path: {
          type: "string",
          description: "当前工作区内的相对文件路径"
        },
        query: {
          type: "string",
          description: "可选查询词，用于在聚合上下文时补充相关搜索结果"
        },
        limit: {
          type: "integer",
          description: "最多聚合多少个路径，默认使用运行时内置上限"
        },
        lineCount: {
          type: "integer",
          description: "读取文件片段时的目标行数"
        }
      },
      required: ["path"],
      additionalProperties: false
    }
  },
  {
    name: "Search",
    canonicalToolName: "Search",
    executionPrimitive: "workspace_search_text",
    description: "在当前工作区内递归搜索文本内容，返回命中路径、行号和预览片段。",
    kind: "search",
    exposure: "model_visible",
    displayMetadata: { displayNameZh: "搜索" },
    permissionFacts: {
      requiresApproval: false,
      permissionScope: "workspace.read",
      hostMediated: false,
      approvalMode: "none",
      decisionSource: "runtime",
      permissionProfile: "builtin"
    },
    inputSchema: {
      type: "object",
      properties: {
        query: {
          type: "string",
          description: "要搜索的关键字或文本片段"
        },
        path: {
          type: "string",
          description: "可选相对路径，用于缩小搜索范围"
        },
        limit: {
          type: "integer",
          description: "最多返回多少条命中结果"
        },
        regex: {
          type: "boolean",
          description: "是否按增强模式匹配 query；当前 v1 使用通配符式匹配，默认 false"
        }
      },
      required: ["query"],
      additionalProperties: false
    }
  },
  {
    name: "List",
    canonicalToolName: "List",
    executionPrimitive: "workspace_list_files",
    description: "列出当前工作区目录中的文件与子目录，可指定相对路径和返回条数。",
    kind: "read",
    exposure: "model_visible",
    displayMetadata: { displayNameZh: "列表" },
    permissionFacts: {
      requiresApproval: false,
      permissionScope: "workspace.read",
      hostMediated: false,
      approvalMode: "none",
      decisionSource: "runtime",
      permissionProfile: "builtin"
    },
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
  },
  {
    name: "Glob",
    canonicalToolName: "Glob",
    executionPrimitive: "workspace_glob_files",
    description: "按路径 pattern 递归匹配工作区内文件，适合大代码库中的文件发现。",
    kind: "read",
    exposure: "model_visible",
    displayMetadata: { displayNameZh: "匹配" },
    permissionFacts: {
      requiresApproval: false,
      permissionScope: "capability.discovery",
      hostMediated: false,
      approvalMode: "none",
      decisionSource: "runtime",
      permissionProfile: "builtin"
    },
    inputSchema: {
      type: "object",
      properties: {
        pattern: {
          type: "string",
          description: "要匹配的路径模式，例如 src/*.rs 或 *tool*"
        },
        path: {
          type: "string",
          description: "搜索起点目录，默认为 ."
        },
        limit: {
          type: "integer",
          description: "最多返回多少条路径命中，默认 50"
        }
      },
      required: ["pattern"],
      additionalProperties: false
    }
  },
  {
    name: "WebFetch",
    canonicalToolName: "WebFetch",
    executionPrimitive: "web_fetch_url",
    description: "抓取指定 http/https URL 的正文内容预览，不承担搜索排序职责。",
    kind: "read",
    exposure: "model_visible",
    displayMetadata: { displayNameZh: "抓取" },
    permissionFacts: {
      requiresApproval: false,
      permissionScope: "workspace.read",
      hostMediated: false,
      approvalMode: "none",
      decisionSource: "runtime",
      permissionProfile: "builtin"
    },
    inputSchema: {
      type: "object",
      properties: {
        url: {
          type: "string",
          description: "要抓取的 http/https URL"
        },
        timeoutMs: {
          type: "integer",
          description: "请求超时毫秒数，默认 15000"
        }
      },
      required: ["url"],
      additionalProperties: false
    }
  },
  {
    name: "WebSearch",
    canonicalToolName: "WebSearch",
    executionPrimitive: "web_search_query",
    description: "执行外部搜索并返回结构化结果列表，不把抓取和搜索混为一个工具。",
    kind: "search",
    exposure: "model_visible",
    displayMetadata: { displayNameZh: "外搜" },
    permissionFacts: {
      requiresApproval: false,
      permissionScope: "workspace.read",
      hostMediated: false,
      approvalMode: "none",
      decisionSource: "runtime",
      permissionProfile: "builtin"
    },
    inputSchema: {
      type: "object",
      properties: {
        query: {
          type: "string",
          description: "外部搜索关键词"
        },
        limit: {
          type: "integer",
          description: "最多返回多少条搜索结果，默认 5"
        },
        timeoutMs: {
          type: "integer",
          description: "请求超时毫秒数，默认 15000"
        }
      },
      required: ["query"],
      additionalProperties: false
    }
  },
  {
    name: "MCPResource",
    canonicalToolName: "MCPResource",
    executionPrimitive: "mcp_resource_read",
    description: "通过 capability registry 读取指定 MCP 资源 capability 的只读内容，不混入普通工具执行。",
    kind: "read",
    exposure: "model_visible",
    displayMetadata: { displayNameZh: "资源" },
    permissionFacts: {
      requiresApproval: false,
      permissionScope: "workspace.read",
      hostMediated: false,
      approvalMode: "none",
      decisionSource: "runtime",
      permissionProfile: "builtin"
    },
    inputSchema: {
      type: "object",
      properties: {
        capabilityId: {
          type: "string",
          description: "目标 resource capability id，例如 mcp:resource:repo-index"
        },
        arguments: {
          type: "object",
          description: "传给 resource capability 的结构化参数"
        }
      },
      required: ["capabilityId"],
      additionalProperties: false
    }
  },
  {
    name: "ToolSearch",
    canonicalToolName: "ToolSearch",
    executionPrimitive: "tool_search",
    description: "搜索 capability registry 中可用的工具候选，返回结构化候选项，作为 deferred / dynamic tool discovery 入口。",
    kind: "search",
    exposure: "deferred",
    displayMetadata: { displayNameZh: "找工具" },
    permissionFacts: {
      requiresApproval: false,
      permissionScope: "capability.discovery",
      hostMediated: false,
      approvalMode: "none",
      decisionSource: "runtime",
      permissionProfile: "builtin"
    },
    inputSchema: {
      type: "object",
      properties: {
        query: {
          type: "string",
          description: "可选查询词；为空时返回默认候选列表"
        },
        sourceId: {
          type: "string",
          description: "可选 source id，用于缩小 discovery 范围"
        },
        limit: {
          type: "integer",
          description: "最多返回多少条候选，默认 8"
        }
      },
      additionalProperties: false
    }
  },
  {
    name: "Write",
    canonicalToolName: "Write",
    executionPrimitive: "workspace_write_file",
    description: "在当前工作区内新建或整文件覆写文本文件，可控制是否允许覆盖现有文件。",
    kind: "write",
    exposure: "model_visible",
    displayMetadata: { displayNameZh: "写入" },
    permissionFacts: {
      requiresApproval: false,
      permissionScope: "workspace.write",
      hostMediated: false,
      approvalMode: "none",
      decisionSource: "runtime",
      permissionProfile: "builtin"
    },
    inputSchema: {
      type: "object",
      properties: {
        path: {
          type: "string",
          description: "当前工作区内的相对文件路径"
        },
        content: {
          type: "string",
          description: "要写入文件的完整文本内容"
        },
        overwrite: {
          type: "boolean",
          description: "是否允许覆盖已存在文件，默认 true"
        }
      },
      required: ["path", "content"],
      additionalProperties: false
    }
  },
  {
    name: "Edit",
    canonicalToolName: "Edit",
    executionPrimitive: "workspace_edit_file",
    description: "在当前工作区内按 oldText/newText 对文本文件做受控替换；默认只允许单一匹配。",
    kind: "write",
    exposure: "model_visible",
    displayMetadata: { displayNameZh: "编辑" },
    permissionFacts: {
      requiresApproval: false,
      permissionScope: "workspace.write",
      hostMediated: false,
      approvalMode: "none",
      decisionSource: "runtime",
      permissionProfile: "builtin"
    },
    inputSchema: {
      type: "object",
      properties: {
        path: {
          type: "string",
          description: "当前工作区内的相对文件路径"
        },
        oldText: {
          type: "string",
          description: "需要被替换的原始文本"
        },
        newText: {
          type: "string",
          description: "替换后的新文本"
        },
        replaceAll: {
          type: "boolean",
          description: "是否允许替换全部匹配，默认 false"
        }
      },
      required: ["path", "oldText", "newText"],
      additionalProperties: false
    }
  },
  {
    name: "Plan",
    canonicalToolName: "Plan",
    executionPrimitive: "workspace_batch",
    description: "表达计划并驱动受控的复合子调用执行；通过 ToolPlan 与 child_results 暴露计划和执行细节。",
    kind: "composite",
    exposure: "model_visible",
    displayMetadata: { displayNameZh: "计划" },
    permissionFacts: {
      requiresApproval: false,
      permissionScope: "workspace.read",
      hostMediated: false,
      approvalMode: "none",
      decisionSource: "runtime",
      permissionProfile: "builtin"
    },
    inputSchema: {
      type: "object",
      properties: {
        calls: {
          type: "array",
          description: "待执行的子调用数组"
        },
        parallel: {
          type: "boolean",
          description: "是否并发执行子调用"
        },
        continueOnError: {
          type: "boolean",
          description: "子调用失败后是否继续执行剩余步骤"
        }
      },
      required: ["calls"],
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
    capabilityId: `builtin:${canonicalizeBuiltinCapabilityName(tool.executionPrimitive)}`,
    sourceId: "builtin-tools",
    sourceKind: "builtin",
    kind: "tool",
    label: canonicalizeBuiltinCapabilityName(tool.executionPrimitive),
    canonicalToolName: tool.canonicalToolName,
    displayNameZh: tool.displayMetadata.displayNameZh ?? null,
    description: tool.description,
    invocationMode: "direct_tool_call",
    inputSchemaSummary: tool.inputSchema.type ?? "object",
    safetyClass: "host_tool",
    visibility: "default",
    observabilityTags: ["builtin", "tool"],
    requiresApproval: tool.permissionFacts.requiresApproval ?? false,
    hostMediated: tool.permissionFacts.hostMediated ?? false,
    permissionScope: tool.permissionFacts.permissionScope ?? "--",
    permissionFacts: { ...tool.permissionFacts }
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
        .map((attachment) => ({ ...attachment })),
      turnId: message.turnId,
      status: message.status === "done" || message.status === "error" ? message.status : null,
      modelName: message.modelName,
      tokenCount: message.tokenCount,
      reasoningContent: message.reasoningContent
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

function isTerminalPhase(phase: string): phase is "completed" | "failed" | "cancelled" {
  return phase === "completed" || phase === "failed" || phase === "cancelled";
}

function resolveRestoredPersistedPhase(
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

function isValidPersistedState(state: unknown): state is PersistedRuntimeState {
  if (state == null || typeof state !== "object") {
    return false;
  }
  const s = state as Record<string, unknown>;
  const version = s.cachedStateVersion;
  if (version !== undefined && version !== CACHED_STATE_VERSION) {
    return false;
  }
  return Array.isArray(s.messages);
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
    const allSessions = parsed.sessions ?? {};
    const validCount = Object.values(allSessions).filter((s) => isValidPersistedState(s)).length;
    debugLog("restore:ok", {
      sessions: Object.keys(allSessions).length,
      valid: validCount,
      stale: Object.keys(allSessions).length - validCount
    });
    return {
      sessions: allSessions
    };
  } catch {
    debugLog("restore:error");
    return { sessions: {} };
  }
}

function loadPersistedRuntimeState(sessionId: string): PersistedRuntimeState | null {
  const raw = loadPersistedRuntimeCache().sessions[sessionId];
  if (raw && isValidPersistedState(raw)) {
    return raw;
  }
  if (raw) {
    debugLog("persist:stale", { sessionId, version: (raw as Partial<PersistedRuntimeState>).cachedStateVersion });
  }
  return null;
}

function buildRuntimeViewFromPersistedState(
  sessionId: string,
  persisted: PersistedRuntimeState | null,
  nodeId?: string | null
): SessionRuntimeView {
  const persistedVisibleNodeId = persisted?.visibleNodeId?.trim() || null;
  const requestedVisibleNodeId = nodeId?.trim() || persistedVisibleNodeId;
  const persistedBranchHeadNodeId = persisted?.branchHeadNodeId?.trim() || null;
  const persistedActiveBranchId = persisted?.activeBranchId?.trim() || null;
  const persistedHistoryNodes = cloneHistoryNodes(persisted?.historyNodes);
  const persistedHistoryBranches = cloneHistoryBranches(persisted?.historyBranches);
  const persistedHistoryCursor =
    requestedVisibleNodeId || persistedBranchHeadNodeId || persistedActiveBranchId
      ? {
          sessionId,
          visibleNodeId: requestedVisibleNodeId,
          activeBranchId: persistedActiveBranchId,
          branchHeadNodeId:
            persistedBranchHeadNodeId ||
            resolveHistoryBranchHeadNodeId(persistedActiveBranchId, persistedHistoryBranches),
          workspaceNodeId: requestedVisibleNodeId,
          mode: normalizeHistoryCursorMode(
            persisted?.historyCursorMode ??
              (requestedVisibleNodeId && persistedBranchHeadNodeId && requestedVisibleNodeId !== persistedBranchHeadNodeId
                ? "historical"
                : "live")
          ),
          authorityMode: "local_preview",
          cursorVersion: null,
          isAtBranchHead:
            !!requestedVisibleNodeId &&
            !!(persistedBranchHeadNodeId || resolveHistoryBranchHeadNodeId(persistedActiveBranchId, persistedHistoryBranches)) &&
            requestedVisibleNodeId ===
              (persistedBranchHeadNodeId || resolveHistoryBranchHeadNodeId(persistedActiveBranchId, persistedHistoryBranches))
        }
      : null;
  const snapshot = {
    conversationId: sessionId,
    title: buildSessionTitleFromMessages(persisted?.messages ?? []),
    summary: persisted?.sessionSummary ?? (persisted?.messages?.length ? DEFAULT_BROWSER_SESSION_SUMMARY : ""),
    history: buildTurnHistory(persisted?.messages ?? []),
    attachmentAssets: persisted?.attachmentAssets ?? [],
    turnTraceHistory: persisted?.turnTraceHistory ?? [],
    turnCount: persisted?.messages?.filter((message) => message.role === "user").length ?? 0,
    historyStateEvidence: [],
    historyStateAuditSummary: null,
    runControlAuditSummary: null,
    lastReferencedFile: null,
    updatedAtMs:
      persisted?.turnTraceHistory?.length
        ? persisted.turnTraceHistory[persisted.turnTraceHistory.length - 1]!.updatedAt
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
    historyNodes: persistedHistoryNodes.length > 0 ? persistedHistoryNodes : undefined,
    historyBranches: persistedHistoryBranches.length > 0 ? persistedHistoryBranches : undefined,
    historyCursor: persistedHistoryCursor,
    authorityMode: "local_preview",
    resolvedVisibleNodeId: requestedVisibleNodeId,
    activeBranchHeadNodeId:
      persistedBranchHeadNodeId || resolveHistoryBranchHeadNodeId(persistedActiveBranchId, persistedHistoryBranches),
    isAtBranchHead:
      !!requestedVisibleNodeId &&
      !!(persistedBranchHeadNodeId || resolveHistoryBranchHeadNodeId(persistedActiveBranchId, persistedHistoryBranches)) &&
      requestedVisibleNodeId ===
        (persistedBranchHeadNodeId || resolveHistoryBranchHeadNodeId(persistedActiveBranchId, persistedHistoryBranches)),
    cursorVersion: null
  } satisfies SessionRuntimeView;
}

function persistSessionState(sessionId: string, payload: PersistedRuntimeState) {
  if (typeof window === "undefined") {
    return;
  }

  const cache = loadPersistedRuntimeCache();
  cache.sessions[sessionId] = payload;
  window.localStorage.setItem(RUNTIME_STORAGE_KEY, JSON.stringify(cache));
}

function persistRunningSessionMap(
  map: Record<string, RunningTurn>,
  completedSessionSet: Record<string, boolean>,
  failedSessionSet: Record<string, boolean>
) {
  if (typeof window === "undefined") {
    return;
  }
  try {
    const cache = loadPersistedRuntimeCache();
    const stripped: Record<string, { turnId: string; phase: RuntimePhase; textBuffer: string; reasoningBuffer: string }> = {};
    for (const [sid, turn] of Object.entries(map)) {
      stripped[sid] = {
        turnId: turn.turnId,
        phase: turn.phase,
        textBuffer: turn.textBuffer.slice(-5000),
        reasoningBuffer: turn.reasoningBuffer.slice(-2000)
      };
    }
    cache.runningSessionMap = stripped;
    cache.completedSet = { ...completedSessionSet };
    cache.failedSet = { ...failedSessionSet };
    window.localStorage.setItem(RUNTIME_STORAGE_KEY, JSON.stringify(cache));
  } catch {
    debugLog("persist:running-map:error");
  }
}

function loadRunningSessionMap(): Record<string, RunningTurn> {
  try {
    const raw = loadPersistedRuntimeCache().runningSessionMap ?? {};
    // Normalize old format (without buffers) to new format
    const normalized: Record<string, RunningTurn> = {};
    for (const [sid, turn] of Object.entries(raw)) {
      normalized[sid] = {
        turnId: (turn as any).turnId ?? "",
        phase: (turn as any).phase ?? "idle",
        textBuffer: (turn as any).textBuffer ?? "",
        reasoningBuffer: (turn as any).reasoningBuffer ?? ""
      };
    }
    return normalized;
  } catch {
    return {};
  }
}

function removePersistedSessionState(sessionId: string) {
  if (typeof window === "undefined") {
    return;
  }

  const cache = loadPersistedRuntimeCache();
  delete cache.sessions[sessionId];
  window.localStorage.setItem(RUNTIME_STORAGE_KEY, JSON.stringify(cache));
}

function mergeRuntimeViews(
  cached: SessionRuntimeView,
  host: SessionRuntimeView | null
): SessionRuntimeView {
  if (!host) {
    return cached;
  }
  return {
    ...cached,
    checkpoint: host.checkpoint ?? cached.checkpoint,
    submissionPlan: host.submissionPlan ?? cached.submissionPlan,
    controlBoundaryEvidence: host.controlBoundaryEvidence?.length
      ? host.controlBoundaryEvidence
      : cached.controlBoundaryEvidence,
    runControlAuditSummary: host.runControlAuditSummary ?? cached.runControlAuditSummary,
    historyStateAuditSummary: host.historyStateAuditSummary ?? cached.historyStateAuditSummary,
    retrieved: host.retrieved ?? cached.retrieved,
    authorityMode: host.authorityMode ?? cached.authorityMode,
    resolvedVisibleNodeId: host.resolvedVisibleNodeId ?? cached.resolvedVisibleNodeId,
    activeBranchHeadNodeId: host.activeBranchHeadNodeId ?? cached.activeBranchHeadNodeId,
    isAtBranchHead: host.isAtBranchHead ?? cached.isAtBranchHead,
    historyNodes: host.historyNodes?.length ? host.historyNodes : cached.historyNodes,
    historyBranches: host.historyBranches?.length ? host.historyBranches : cached.historyBranches,
    historyCursor: host.historyCursor ?? cached.historyCursor,
    session: {
      ...cached.session,
      turnTraceHistory: cached.session.turnTraceHistory?.length
        ? cached.session.turnTraceHistory
        : host.session.turnTraceHistory,
      historyStateEvidence: (host.historyStateEvidence?.length
        ? host.historyStateEvidence
        : cached.historyStateEvidence) ?? undefined,
      historyStateAuditSummary: host.historyStateAuditSummary ?? cached.session.historyStateAuditSummary ?? null,
      runControlAuditSummary: host.runControlAuditSummary ?? cached.session.runControlAuditSummary ?? null,
    }
  };
}

function deriveInitStrategy(
  currentSessionId: string,
  persisted: PersistedRuntimeState | null,
  sessionList: SessionOverview[]
): SessionInitializationStrategy {
  if (persisted) {
    const hasValidCheckpoint = persisted.checkpoint != null;
    const isNonIdle = persisted.phase != null && persisted.phase !== "idle";
    if (hasValidCheckpoint && isNonIdle) {
      return { kind: "local-cache", persistedState: persisted };
    }
    return { kind: "host-read", sessionId: currentSessionId, reason: "insufficient-checkpoint" };
  }
  const targetSessionId = sessionList[0]?.conversationId ?? currentSessionId;
  if (sessionList.length > 0 || targetSessionId !== DEFAULT_SESSION_ID) {
    return { kind: "host-read", sessionId: targetSessionId, reason: "no-cache" };
  }
  return { kind: "empty-fallback", sessionId: currentSessionId };
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
  const legacyTraceHistory = state.turnTraceHistory ?? [];
  return {
    conversationId,
    title: buildSessionTitleFromMessages(state.messages),
    summary: state.sessionSummary || DEFAULT_BROWSER_SESSION_SUMMARY,
    turnCount: state.messages.filter((message) => message.role === "user").length,
    lastReferencedFile: null,
    updatedAtMs:
      legacyTraceHistory.length > 0
        ? legacyTraceHistory[legacyTraceHistory.length - 1]!.updatedAt
        : Date.now()
  };
}

function buildSessionOverviewFromRuntimeState(state: Pick<RuntimeState, "sessionId" | "sessionSummary" | "messages" | "turnTraceHistory">): SessionOverview | null {
  if (!hasPersistableMessages(state.messages)) {
    return null;
  }

  const latestTrace = state.turnTraceHistory[state.turnTraceHistory.length - 1] ?? null;
  return {
    conversationId: state.sessionId,
    title: buildSessionTitleFromMessages(state.messages),
    summary: state.sessionSummary || DEFAULT_BROWSER_SESSION_SUMMARY,
    turnCount: state.messages.filter((message) => message.role === "user").length,
    lastReferencedFile: null,
    updatedAtMs: latestTrace?.updatedAt ?? Date.now()
  };
}

function ensureUniqueSessionList(sessions: SessionOverview[]): SessionOverview[] {
  const seen = new Set<string>();
  const deduped: SessionOverview[] = [];
  for (const session of sessions) {
    if (seen.has(session.conversationId)) {
      continue;
    }
    seen.add(session.conversationId);
    deduped.push(session);
  }
  return deduped;
}

function createNextSessionId(existingSessionIds: Iterable<string>): string {
  const existing = new Set(existingSessionIds);
  let candidate = `session-${Date.now()}`;
  let counter = 1;
  while (existing.has(candidate)) {
    candidate = `session-${Date.now()}-${counter}`;
    counter += 1;
  }
  return candidate;
}

function buildSessionTitleFromMessages(messages: ChatMessage[]) {
  const firstUserMessage = messages.find((message) => message.role === "user");
  return firstUserMessage ? buildTurnTitle(firstUserMessage.content) : "新对话";
}

function isPersistedMetadataCompatible(
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

  const persistedHistory = buildTurnHistory(persisted.messages);
  if (persistedHistory.length !== snapshot.history.length) {
    return false;
  }

  return persistedHistory.every((message, index) => message.role === snapshot.history[index]?.role);
}

function isHistoricalRuntimeView(
  runtimeView?:
    | Pick<SessionRuntimeView, "historyCursor">
    | null
) {
  return normalizeHistoryCursorMode(runtimeView?.historyCursor?.mode) !== "live";
}

function isHistoricalMode(mode?: HistoryCursorMode | null) {
  return normalizeHistoryCursorMode(mode) !== "live";
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

function traceErrorDetail(trace?: TurnTraceRecord | null) {
  if (!trace) {
    return null;
  }

  const topLevelError = trace.error?.trim();
  if (topLevelError) {
    return topLevelError;
  }

  const timelineError = [...(trace.traceTimeline ?? [])]
    .reverse()
    .find((entry) => entry.error?.trim())
    ?.error?.trim();
  if (timelineError) {
    return timelineError;
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
    canonicalToolName: tool.canonicalToolName ?? null,
    displayNameZh: tool.displayNameZh ?? null,
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
      currentTurnId = item.turnId ?? restoredMessage?.turnId ?? currentTrace?.turnId ?? createHistoryTurnId(turnIndex);
      turnIndex += 1;
      restoredHistoryIndex += 1;
      messages.push({
        id: restoredMessage?.id ?? `history-user-${turnIndex}`,
        turnId: currentTurnId,
        role: "user",
        content: item.content,
        attachments: item.attachments ?? [],
        status: "done",
        tokenCount: item.tokenCount ?? restoredMessage?.tokenCount ?? null
      });
      continue;
    }

    currentTrace = currentTrace ?? orderedTurnTraceHistory[traceIndex] ?? null;
    if (!currentTurnId) {
      currentTurnId = item.turnId ?? restoredMessage?.turnId ?? currentTrace?.turnId ?? createHistoryTurnId(turnIndex);
      turnIndex += 1;
    }

    restoredHistoryIndex += 1;
    const restoredErrorDetail = restoredMessage?.errorDetail ?? null;
    const traceError = traceErrorDetail(currentTrace);
    const hasTraceError = currentTrace?.phase === "failed" || Boolean(traceError);
    const hasErrorState = item.status === "error" || hasTraceError || (!currentTrace && restoredMessage?.status === "error");
    const errorDetail = hasTraceError ? (traceError || restoredErrorDetail) : (currentTrace ? null : restoredErrorDetail);
    messages.push({
      id: restoredMessage?.id ?? `history-assistant-${turnIndex}`,
      turnId: currentTurnId,
      role: "assistant",
      content: item.content,
      attachments: [],
      status: hasErrorState ? "error" : "done",
      reasoningContent: item.reasoningContent ?? restoredMessage?.reasoningContent ?? traceReasoningContent(currentTrace),
      tokenCount: item.tokenCount ?? restoredMessage?.tokenCount ?? currentTrace?.outputTokens ?? null,
      modelName: item.modelName ?? restoredMessage?.modelName ?? traceModelLabel(currentTrace),
      errorDetail
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
        sessionHydrating: false,
        deletingSessionSet: {},
        sessionSwitchToken: 0,
        sessionError: null,
      phase: resolveRestoredPersistedPhase(
        persisted?.phase ?? null,
        persisted?.canonicalTerminalPhase ?? null,
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
      cursorVersion: null,
      historyCursorMode: "live",
      historyNodes: [],
      historyBranches: [],
      initialRollbackActive: false,
      eventsReady: false,
      deferredPersistTimerId: null,
      streamFlushFrameId: null,
      streamFlushTimerId: null,
      streamBufferTurnId: null,
      streamBufferText: "",
      streamBufferReasoning: "",
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
      ),
      runningSessionMap: {},
      completedSessionSet: {},
      failedSessionSet: {},
      streamDebugDeltaCount: 0,
      streamDebugFlushCount: 0,
      streamDebugTextCharsReceived: 0,
      streamDebugTextCharsFlushed: 0
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
    isSessionDeleting(state) {
      return (sessionId: string) => sessionId in state.deletingSessionSet;
    },
    conversationCheckpointEntries(state): ConversationCheckpointEntry[] {
      const entries = buildConversationCheckpointEntries(
        state.historyNodes,
        state.historyBranches,
        state.activeBranchId,
        state.visibleNodeId,
        state.branchHeadNodeId
      );

      // Synthesize entries for completed turns in messages that lack history nodes.
      // This ensures rollback buttons work even before backend checkpoints arrive.
      const coveredTurnIds = new Set(entries.map((e) => e.turnId));
      const turnUserContent = new Map<string, string>();
      for (const msg of state.messages) {
        if (msg.role === "user" && msg.content && !turnUserContent.has(msg.turnId)) {
          turnUserContent.set(msg.turnId, msg.content);
        }
      }

      const allTurnIds = [...turnUserContent.keys()];
      let hasSynthetic = false;

      for (let index = 0; index < allTurnIds.length; index++) {
        const turnId = allTurnIds[index]!;
        if (coveredTurnIds.has(turnId)) continue;

        entries.push({
          nodeId: `synthetic-${turnId}`,
          turnId,
          branchId: state.activeBranchId ?? "branch-main",
          summary: turnUserContent.get(turnId)!.slice(0, 120) || turnId,
          createdAtMs: Date.now() - (allTurnIds.length - index),
          isLatest: false,
          isVisible: false,
          workspaceRollbackCapable: false,
          availableModes: ["transcript_only"],
          forkTargets: []
        });
        hasSynthetic = true;
      }

      if (hasSynthetic) {
        entries.sort((left, right) => right.createdAtMs - left.createdAtMs);
        const hasRealLatest =
          state.branchHeadNodeId != null && entries.some((e) => e.nodeId === state.branchHeadNodeId);
        if (!hasRealLatest && entries.length > 0) {
          entries[0] = { ...entries[0], isLatest: true };
        }
      }

      return entries;
    },
    isSessionRunning(state): (sessionId: string) => boolean {
      return (sessionId: string) => sessionId in state.runningSessionMap;
    }
  },
  actions: {
    resetSessionRuntimeState() {
      this.cancelDeferredPersist();
      const blankFields = createBlankSessionRuntimeFields();
      this.phase = "idle";
      this.error = null;
      this.draftMessage = "";
      this.sessionHydrating = false;
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
      this.initialRollbackActive = false;
      this.messages = [];
      this.attachmentAssets = [];
      this.toolActivities = [];
      this.traceSteps = createDefaultTraceSteps();
      this.traceTimeline = createDefaultTraceTimeline();
      this.turnTraceHistory = [];
      this.eventCursorByTurnId = {};
      this.cancelStreamFlush();
      this.streamBufferTurnId = null;
      this.streamFlushTimerId = null;
      this.streamBufferText = "";
      this.streamBufferReasoning = "";
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
        if (this.streamFlushTimerId == null) {
          return;
        }
      }

      if (this.streamFlushFrameId != null) {
        window.cancelAnimationFrame(this.streamFlushFrameId);
        this.streamFlushFrameId = null;
      }

      if (this.streamFlushTimerId != null) {
        window.clearTimeout(this.streamFlushTimerId);
        this.streamFlushTimerId = null;
      }
    },
    resetStreamDebugMetrics() {
      this.streamDebugDeltaCount = 0;
      this.streamDebugFlushCount = 0;
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

      if (this.streamFlushFrameId != null || this.streamFlushTimerId != null) {
        return;
      }

      this.streamFlushTimerId = window.setTimeout(() => {
        this.streamFlushTimerId = null;
        this.flushBufferedStreamText(turnId);
      }, STREAM_FLUSH_INTERVAL_MS);
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
      if (!hasPersistableMessages(this.messages) && !this.initialRollbackActive) {
        removePersistedSessionState(this.sessionId);
        debugLog("persist:skip-empty", {
          sessionId: this.sessionId
        });
        return;
      }

      const payload: PersistedRuntimeState = {
        cachedStateVersion: CACHED_STATE_VERSION,
        phase: this.phase,
        canonicalTerminalPhase: !this.initialRollbackActive && isTerminalPhase(this.phase)
          ? this.phase
          : undefined,
        messages: this.messages,
        attachmentAssets: this.attachmentAssets,
        // turnTraceHistory intentionally excluded — trace data lives on the backend
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
        firstTokenLatencyMs: this.firstTokenLatencyMs,
        visibleNodeId: this.visibleNodeId,
        branchHeadNodeId: this.branchHeadNodeId,
        activeBranchId: this.activeBranchId,
        historyCursorMode: this.historyCursorMode,
        historyNodes: cloneHistoryNodes(this.historyNodes),
        historyBranches: cloneHistoryBranches(this.historyBranches),
        initialRollbackActive: this.initialRollbackActive,
        checkpoint: this.latestExecutionCheckpoint ? { ...this.latestExecutionCheckpoint } : null,
        runningTurnId: this.activeTurnId
      };

      try {
        persistSessionState(this.sessionId, payload);
        persistRunningSessionMap(this.runningSessionMap, this.completedSessionSet, this.failedSessionSet);
        debugLog("persist", {
          sessionId: this.sessionId,
          messages: this.messages.length,
          traces: this.turnTraceHistory.length,
          phase: this.phase,
          hasCheckpoint: !!this.latestExecutionCheckpoint,
          hasRunningTurn: !!this.activeTurnId,
          runningSessionCount: Object.keys(this.runningSessionMap).length
        });
      } catch {
        // Ignore storage failures and keep runtime in memory.
        debugLog("persist:error");
      }
    },
    updatePersistedBackgroundSession(
      sessionId: string,
      turnId: string,
      patch: {
        content?: string | null;
        reasoningContent?: string | null;
        status: ChatMessage["status"];
        modelName?: string | null;
        tokenCount?: number | null;
        phase: RuntimePhase;
        sessionSummary?: string | null;
        providerRequestedName?: string | null;
        providerName?: string | null;
        providerProtocol?: string | null;
        providerModel?: string | null;
        providerSource?: string | null;
        providerMode?: string | null;
        fallbackReason?: string | null;
        inputTokens?: number | null;
        outputTokens?: number | null;
        totalTokens?: number | null;
        firstTokenLatencyMs?: number | null;
        errorDetail?: string | null;
      }
    ) {
      const persisted = loadPersistedRuntimeState(sessionId);
      if (!persisted) {
        return;
      }

      const messages = persisted.messages.map((message) => ({
        ...message,
        attachments: message.attachments?.map((attachment) => ({ ...attachment })) ?? []
      }));
      let assistantMessage = messages.find(
        (message) => message.turnId === turnId && message.role === "assistant"
      );

      if (!assistantMessage) {
        assistantMessage = {
          id: `assistant-${turnId}`,
          turnId,
          role: "assistant",
          content: "",
          attachments: [],
          reasoningContent: null,
          status: patch.status,
          tokenCount: null,
          modelName: patch.modelName?.trim() || undefined
        };
        messages.push(assistantMessage);
      }

      const nextContent = patch.content?.trim();
      if (nextContent) {
        assistantMessage.content = patch.content ?? assistantMessage.content;
      }
      assistantMessage.reasoningContent = normalizeReasoningContent(
        patch.reasoningContent ?? assistantMessage.reasoningContent ?? null
      );
      assistantMessage.status = patch.status;
      assistantMessage.modelName = patch.modelName?.trim() || assistantMessage.modelName;
      assistantMessage.tokenCount = patch.tokenCount ?? assistantMessage.tokenCount ?? null;
      assistantMessage.errorDetail = patch.errorDetail ?? assistantMessage.errorDetail ?? null;

      persistSessionState(sessionId, {
        ...persisted,
        phase: patch.phase,
        canonicalTerminalPhase: isTerminalPhase(patch.phase)
          ? patch.phase
          : persisted.canonicalTerminalPhase,
        messages,
        sessionSummary: patch.sessionSummary ?? persisted.sessionSummary,
        providerRequestedName: patch.providerRequestedName ?? persisted.providerRequestedName,
        providerName: patch.providerName ?? persisted.providerName,
        providerProtocol: patch.providerProtocol ?? persisted.providerProtocol,
        providerModel: patch.providerModel ?? persisted.providerModel,
        providerSource: patch.providerSource ?? persisted.providerSource,
        providerMode: patch.providerMode ?? persisted.providerMode,
        fallbackReason:
          patch.fallbackReason === undefined ? persisted.fallbackReason : patch.fallbackReason,
        inputTokens: patch.inputTokens ?? persisted.inputTokens,
        outputTokens: patch.outputTokens ?? persisted.outputTokens,
        totalTokens: patch.totalTokens ?? persisted.totalTokens,
        firstTokenLatencyMs: patch.firstTokenLatencyMs ?? persisted.firstTokenLatencyMs
      });
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
      this.cursorVersion = historyCursorVersion(payload);
      this.historyCursorMode = normalizeHistoryCursorMode(
        payload?.mode ?? (visibleNodeId && branchHeadNodeId && visibleNodeId !== branchHeadNodeId ? "historical" : "live")
      );
    },
    async loadSessionCatalog() {
      if (isTauriAvailable()) {
        this.sessionList = filterDeletingSessions(
          await measureHostRead("list_sessions", {
            deletingCount: Object.keys(this.deletingSessionSet).length
          }, () => safeInvoke<SessionOverview[]>("list_sessions")),
          this.deletingSessionSet
        );
        return;
      }

      const cache = loadPersistedRuntimeCache();
      this.sessionList = filterDeletingSessions(
        Object.entries(cache.sessions)
          .map(([conversationId, state]) => buildSessionOverviewFromPersistedState(conversationId, state))
          .sort((left, right) => right.updatedAtMs - left.updatedAtMs),
        this.deletingSessionSet
      );
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
        return await measureHostRead("load_session_runtime_view", {
          sessionId,
          nodeId: nodeId ?? null
        }, () => safeInvoke<SessionRuntimeView>("load_session_runtime_view", {
          ...payload
        }));
      }

      return buildRuntimeViewFromPersistedState(sessionId, loadPersistedRuntimeState(sessionId), nodeId);
    },
    applyExecutionCheckpoint(
      checkpoint: ExecutionCheckpoint | null,
      persistedMessages?: ChatMessage[] | null
    ) {
      this.latestExecutionCheckpoint = checkpoint ? { ...checkpoint } : null;
      if (!checkpoint) {
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

      if (!this.messages.some((msg) => msg.turnId === checkpoint.turnId)) {
        return;
      }

      const checkpointStatus = checkpoint.status.trim().toLowerCase();
      const isRecoveryCheckpoint = checkpoint.checkpointKind === "recovery";
      if (!isRecoveryCheckpoint && checkpointStatus !== "running") {
        return;
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
        preserveMessages?: boolean;
      }
    ) {
      const refreshCatalog = options?.refreshCatalog ?? true;
      const persistedState = loadPersistedRuntimeState(nextSessionId);
      const shouldPreferLocalInitialRollback = Boolean(persistedState?.initialRollbackActive) && !options?.nodeId;
      const persistedVisibleNodeId = persistedState?.visibleNodeId?.trim() || null;
      const fallbackNodeId =
        options?.nodeId ??
        (!isTauriAvailable()
          ? persistedVisibleNodeId
          : null);
      const runtimeView =
        options?.runtimeView ?? (shouldPreferLocalInitialRollback
          ? buildRuntimeViewFromPersistedState(nextSessionId, persistedState, fallbackNodeId)
          : await this.loadSessionRuntimeViewState(nextSessionId, fallbackNodeId));
      const snapshot = runtimeView.session;
      const retrieved = runtimeView.retrieved;
      const persisted = persistedState;
      const runtimeViewIsHistorical =
        !hasHostManagedHistoryState(runtimeView)
          ? normalizeHistoryCursorMode(runtimeView.historyCursor?.mode ?? null) !== "live"
          : normalizeHistoryCursorMode(runtimeView.historyCursor?.mode ?? null) !== "live" ||
            runtimeView.isAtBranchHead === false;
      const shouldApplyRuntimeViewCheckpoint =
        !runtimeViewIsHistorical && (!options?.nodeId || !runtimeViewIsHistorical);
      const hasCheckpointOverride =
        options != null && Object.prototype.hasOwnProperty.call(options, "executionCheckpoint");
      this.applySessionSnapshot(nextSessionId, snapshot, retrieved, runtimeView, { preserveMessages: options?.preserveMessages });
      this.applyExecutionCheckpoint(
        hasCheckpointOverride
          ? (options?.executionCheckpoint ?? null)
          : shouldApplyRuntimeViewCheckpoint
            ? (runtimeView.checkpoint ?? null)
            : null,
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
        const retrieved = await measureHostRead("load_retrieved_context", {
          sessionId,
          nodeId: options?.nodeId ?? null,
          runId: options?.runId ?? null
        }, () => safeInvoke<RetrievedContextState>("load_retrieved_context", payload));
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
        | null,
      options?: { preserveMessages?: boolean }
    ) {
      const persisted = loadPersistedRuntimeState(sessionId);
      const historicalRuntimeView = isHistoricalRuntimeView(runtimeView);
      const persistedInitialRollbackActive = Boolean(persisted?.initialRollbackActive);
      const canReusePersistedState =
        persistedInitialRollbackActive || (!historicalRuntimeView && isPersistedMetadataCompatible(snapshot, persisted));
      const canMergePersistedMessages =
        persistedInitialRollbackActive || (!historicalRuntimeView && isPersistedMessageShapeCompatible(snapshot, persisted));
      const restoredState = canReusePersistedState ? persisted : null;
      const blankFields = createBlankSessionRuntimeFields();
      const retrievedSummary = retrieved?.sessionContext?.summary?.trim() ?? "";
      const snapshotSummary = snapshot.history.length > 0 ? snapshot.summary : "";
      const sessionSummary = retrievedSummary || restoredState?.sessionSummary || snapshotSummary;
      const snapshotTurnTraceHistory = snapshot.turnTraceHistory ?? [];
      const effectiveTurnTraceHistory = (
        snapshotTurnTraceHistory.length
          ? snapshotTurnTraceHistory
          : []
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
      this.initialRollbackActive = Boolean(restoredState?.initialRollbackActive);
      this.draftMessage = "";
      this.sessionSummary = sessionSummary;
      this.retrievedContext = cloneRetrievedContext(retrieved ?? deriveRetrievedContextFromSnapshot(snapshot));
      if (!options?.preserveMessages || !this.messages.length) {
        this.messages = hydrateMessagesFromHistory(
          snapshot.history,
          canMergePersistedMessages ? persisted?.messages : null,
          effectiveTurnTraceHistory
        );
      }
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
        restoredState?.canonicalTerminalPhase ?? null,
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
      const runtimeHistoryCursor = resolveRuntimeViewHistoryProjection(runtimeView);
      const hasRuntimeHistoryState = Boolean(
        runtimeHistoryCursor || runtimeView?.historyNodes?.length || runtimeView?.historyBranches?.length
      );
      if (hasRuntimeHistoryState) {
        this.applyHistoryState(sessionId, {
          historyNodes: runtimeView?.historyNodes,
          historyBranches: runtimeView?.historyBranches,
          ...(runtimeHistoryCursor ? { ...runtimeHistoryCursor, sessionId } : {})
        });
      } else if (!hasHostManagedHistoryState(runtimeView) && restoredState) {
        this.applyHistoryState(sessionId, {
          historyNodes: restoredState.historyNodes,
          historyBranches: restoredState.historyBranches,
          activeBranchId: restoredState.activeBranchId,
          branchHeadNodeId: restoredState.branchHeadNodeId,
          visibleNodeId: restoredState.visibleNodeId,
          mode: restoredState.historyCursorMode
        });
      } else {
        this.applyHistoryState(sessionId, {
          historyNodes: runtimeView?.historyNodes,
          historyBranches: runtimeView?.historyBranches,
          ...(runtimeHistoryCursor ? { ...runtimeHistoryCursor, sessionId } : {})
        });
      }
      this.persistHistory();
    },
    rollbackToInitialState() {
      this.cancelDeferredPersist();
      this.cancelStreamFlush();
      this.phase = "idle";
      this.error = null;
      this.draftMessage = "";
      this.sessionSummary = "";
      this.retrievedContext = deriveRetrievedContextFromSnapshot({
        conversationId: this.sessionId,
        title: "新对话",
        summary: "",
        history: [],
        attachmentAssets: [],
        turnTraceHistory: [],
        historyStateEvidence: [],
        historyStateAuditSummary: null,
        runControlAuditSummary: null,
        turnCount: 0,
        lastReferencedFile: null,
        updatedAtMs: Date.now()
      });
      this.providerRequestedName = "";
      this.providerName = "";
      this.providerProtocol = "";
      this.providerModel = "";
      this.providerSource = "";
      this.providerMode = "";
      this.fallbackReason = null;
      this.inputTokens = null;
      this.outputTokens = null;
      this.totalTokens = null;
      this.firstTokenLatencyMs = null;
      this.isSubmitting = false;
      this.activeTurnId = null;
      this.activeRunId = null;
      this.latestExecutionCheckpoint = null;
      this.latestGraphRunSubmissionPlan = null;
      this.latestGraphRunControlBoundaryEvidence = [];
      this.latestRunControlAuditSummary = null;
      this.latestHistoryStateAuditSummary = null;
      this.visibleNodeId = null;
      this.cursorVersion = null;
      this.historyCursorMode = "historical_dirty";
      this.messages = [];
      this.attachmentAssets = [];
      this.toolActivities = [];
      this.traceSteps = createDefaultTraceSteps();
      this.traceTimeline = createDefaultTraceTimeline();
      this.turnTraceHistory = [];
      this.eventCursorByTurnId = {};
      this.streamBufferTurnId = null;
      this.streamFlushTimerId = null;
      this.streamBufferText = "";
      this.streamBufferReasoning = "";
      this.initialRollbackActive = true;
      this.persistHistory();
    },
    async performHistoryOperation<TInvokeArgs extends Record<string, unknown>, TResult extends HistoryCursorState & { historyStateAuditSummary?: HistoryStateAuditSummary | null }>(
      options: {
        cmd: string;
        invokeArgs: TInvokeArgs;
        normalize: (payload: any, nodes: HistoryNode[], branches: HistoryBranch[]) => TResult;
        selectNodeId?: (result: any) => string | null;
        errorMessagePrefix?: string;
      }
    ): Promise<TResult | null> {
      try {
        const payload = await safeInvoke(options.cmd, {
          sessionId: this.sessionId,
          expectedCursorVersion: this.cursorVersion,
          ...options.invokeArgs
        });
        await this.loadSessionState(this.sessionId, {
          refreshCatalog: false,
          nodeId: options.selectNodeId?.(payload) ?? null
        });
        const result = options.normalize(payload, this.historyNodes, this.historyBranches);
        this.initialRollbackActive = false;
        this.applyHistoryState(this.sessionId, result);
        this.latestHistoryStateAuditSummary = cloneHistoryStateAuditSummary(
          result.historyStateAuditSummary ?? null
        );
        return result;
      } catch (error) {
        this.sessionError = `${options.errorMessagePrefix ?? "操作"}失败：${String(error)}`;
        return null;
      }
    },
    async checkoutHistoryNode(nodeId: string, mode: HistoryCheckoutMode = "transcript_only", turnId?: string | null) {
      const sessionId = this.sessionId;
      if (!sessionId || !nodeId.trim()) {
        return null;
      }

      let result: HistoryCheckoutResult;
      if (isTauriAvailable()) {
        let payload: HistoryCheckoutWireResult;
        try {
          payload = await safeInvoke<HistoryCheckoutWireResult>("checkout_history_node", {
            sessionId,
            nodeId,
            mode,
            expectedCursorVersion: this.cursorVersion
          });
        } catch (error) {
          this.sessionError = `历史 checkout 冲突或失败：${String(error)}`;
          return null;
        }
        await this.loadSessionState(sessionId, {
          refreshCatalog: false,
          nodeId
        });
        this.initialRollbackActive = false;
        this.turnTraceHistory = this.turnTraceHistory.filter(
          (trace) => this.messages.some((msg) => msg.turnId === trace.turnId)
        );
        this.eventCursorByTurnId = buildEventCursorByTurnTraceHistory(this.turnTraceHistory);
        result = normalizeHistoryCheckoutResult(payload, this.historyNodes, this.historyBranches);
      } else {
        const resolvedTurnId = turnId?.trim() || this.findCheckpointTurnIdByNodeId(nodeId);
        let truncationIndex = -1;
        const targetNode = this.historyNodes.find((item) => item.nodeId === nodeId) ?? null;
        const targetIsInitialState = !targetNode?.turnId?.trim();
        if (resolvedTurnId) {
          for (let i = this.messages.length - 1; i >= 0; i--) {
            if ((this.messages[i] as ChatMessage).turnId === resolvedTurnId) {
              truncationIndex = i;
              break;
            }
          }
        }
        if (targetIsInitialState) {
          this.rollbackToInitialState();
        } else if (truncationIndex >= 0) {
          this.messages = this.messages.slice(0, truncationIndex + 1);
          this.turnTraceHistory = this.turnTraceHistory.filter(
            (trace) => this.messages.some((msg) => msg.turnId === trace.turnId)
          );
          this.eventCursorByTurnId = buildEventCursorByTurnTraceHistory(this.turnTraceHistory);
          this.traceSteps = createDefaultTraceSteps();
          const lastTrace = this.turnTraceHistory[this.turnTraceHistory.length - 1];
          this.traceTimeline = lastTrace?.traceTimeline?.length
            ? cloneTraceTimeline(lastTrace.traceTimeline)
            : createDefaultTraceTimeline();
          this.initialRollbackActive = false;
          this.persistHistory();
        }

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
    findCheckpointTurnIdByNodeId(nodeId: string): string | null {
      const entry = this.conversationCheckpointEntries.find((e) => e.nodeId === nodeId);
      return entry?.turnId?.trim() || null;
    },
    async restoreBranchHead(branchId?: string | null) {
      const targetBranchId = branchId?.trim() || this.activeBranchId;
      if (!this.sessionId || !targetBranchId) {
        return null;
      }

      if (!isTauriAvailable()) {
        this.sessionError = previewHistoryMutationUnavailable();
        return null;
      }

      const result = await this.performHistoryOperation({
        cmd: "restore_branch_head",
        invokeArgs: { branchId: targetBranchId },
        normalize: (payload) => normalizeHistoryRestoreResult(payload, this.historyNodes, this.historyBranches),
        selectNodeId: (payload) => payload.restoredNodeId ?? payload.cursor?.visibleNodeId ?? null,
        errorMessagePrefix: "恢复 branch head"
      });
      if (result) {
        this.historyCursorMode = "live";
        this.activeTurnId = null;
        this.activeRunId = null;
        this.isSubmitting = false;
        this.latestExecutionCheckpoint = null;
      }
      return result;
    },
    async forkHistoryNode(nodeId?: string | null) {
      const targetNodeId = nodeId?.trim() || this.visibleNodeId;
      if (!this.sessionId || !targetNodeId) {
        return null;
      }

      if (!isTauriAvailable()) {
        this.sessionError = previewHistoryMutationUnavailable();
        return null;
      }

      return this.performHistoryOperation({
        cmd: "fork_from_history_node",
        invokeArgs: { nodeId: targetNodeId },
        normalize: (payload) => normalizeHistoryForkResult(payload, this.historyNodes, this.historyBranches),
        selectNodeId: (payload) => payload.cursor.visibleNodeId ?? payload.cursor.branchHeadNodeId ?? null,
        errorMessagePrefix: "创建 branch"
      });
    },
    async switchHistoryBranch(branchId: string) {
      if (!this.sessionId || !branchId.trim()) {
        return null;
      }

      if (!isTauriAvailable()) {
        this.sessionError = previewHistoryMutationUnavailable();
        return null;
      }

      return this.performHistoryOperation({
        cmd: "switch_history_branch",
        invokeArgs: { branchId },
        normalize: (payload) => normalizeHistoryBranchSwitchResult(payload, this.historyNodes, this.historyBranches),
        selectNodeId: (payload) => payload.cursor.visibleNodeId ?? payload.cursor.branchHeadNodeId ?? null,
        errorMessagePrefix: "切换 branch"
      });
    },
    async switchSession(nextSessionId: string) {
      if (this.sessionOperation && this.sessionOperation !== "switching") {
        return;
      }
      if (nextSessionId === this.sessionId) {
        return;
      }

      // Register current running turn as a background turn before switching away
      const switchingFromRunningTurn = this.isSubmitting && this.activeTurnId != null;
      if (switchingFromRunningTurn && this.activeTurnId) {
        this.runningSessionMap[this.sessionId] = {
          turnId: this.activeTurnId,
          phase: this.phase,
          textBuffer: this.streamBufferText,
          reasoningBuffer: this.streamBufferReasoning
        };
        delete this.completedSessionSet[this.sessionId];
        delete this.failedSessionSet[this.sessionId];
      }

      this.persistHistory();
      const previousSnapshot = createSessionRuntimeSnapshot(this);
      const switchToken = this.sessionSwitchToken + 1;
      const switchStartedAt = typeof performance !== "undefined" ? performance.now() : Date.now();
      this.sessionSwitchToken = switchToken;
      this.sessionOperation = "switching";
      this.sessionError = null;

      const cachedPersistedState = loadPersistedRuntimeState(nextSessionId);
      const cachedRuntimeView = buildRuntimeViewFromPersistedState(
        nextSessionId,
        cachedPersistedState
      );
      const hasCachedState =
        Boolean(cachedPersistedState?.initialRollbackActive) ||
        cachedRuntimeView.session.history.length > 0 ||
        (cachedRuntimeView.session.attachmentAssets?.length ?? 0) > 0 ||
        cachedRuntimeView.session.summary.trim().length > 0;

      this.sessionHydrating = true;
      if (hasCachedState) {
        void this.loadSessionState(nextSessionId, {
          refreshCatalog: false,
          runtimeView: cachedRuntimeView
        }).catch((error) => {
          if (this.sessionSwitchToken !== switchToken || this.sessionId !== nextSessionId) {
            return;
          }
          this.sessionHydrating = false;
          this.sessionError = `切换对话失败：${String(error)}`;
          debugLog("session:switch:cache:error", {
            from: previousSnapshot.sessionId,
            to: nextSessionId,
            error: String(error)
          });
        });
        if (isTauriAvailable()) {
          this.loadSessionRuntimeViewState(nextSessionId).then((hostView) => {
            if (this.sessionSwitchToken !== switchToken || this.sessionId !== nextSessionId || !hostView) {
              return;
            }
            return this.loadSessionState(nextSessionId, {
              refreshCatalog: false,
              runtimeView: hostView,
              preserveMessages: hasCachedState
            }).catch(() => {});
          }).catch(() => {});
        }
      } else {
        this.resetSessionRuntimeState();
        this.sessionId = nextSessionId;
        this.phase = "connecting";
        this.sessionHydrating = true;
        const loadPromise = this.loadSessionState(nextSessionId, {
          refreshCatalog: false
        }).then(() => {
          if (this.sessionSwitchToken !== switchToken || this.sessionId !== nextSessionId) {
            return;
          }
          this.sessionHydrating = false;
          this.sessionError = null;
          debugLog("session:switch:no-cache:loaded", {
            from: previousSnapshot.sessionId,
            to: nextSessionId
          });
        });
        void withTimeout(loadPromise, HYDRATION_TIMEOUT_MS, "加载对话").catch((error) => {
          if (this.sessionSwitchToken !== switchToken || this.sessionId !== nextSessionId) {
            return;
          }
          this.sessionHydrating = false;
          this.sessionError = `切换对话失败：${String(error)}`;
          debugLog("session:switch:no-cache:error", {
            from: previousSnapshot.sessionId,
            to: nextSessionId,
            error: String(error)
          });
        });
      }

      delete this.completedSessionSet[nextSessionId];
      delete this.failedSessionSet[nextSessionId];

      const bgTurn = this.runningSessionMap[nextSessionId];
      if (bgTurn) {
        this.activeTurnId = bgTurn.turnId;
        this.isSubmitting = true;
        this.phase = bgTurn.phase;
        if (bgTurn.textBuffer) {
          this.streamBufferText = bgTurn.textBuffer;
          this.streamBufferTurnId = bgTurn.turnId;
        }
        if (bgTurn.reasoningBuffer) {
          this.streamBufferReasoning = bgTurn.reasoningBuffer;
        }
        delete this.runningSessionMap[nextSessionId];
      }

      debugLog("session:switch", {
        from: previousSnapshot.sessionId,
        to: nextSessionId,
        backgroundTurnRegistered: switchingFromRunningTurn,
        restoredBackgroundTurn: Boolean(bgTurn),
        hydratedFromCache: hasCachedState
      });
      reportSwitchPerf("foreground-switched", {
        from: previousSnapshot.sessionId,
        to: nextSessionId,
        hasCachedState,
        restoredBackgroundTurn: Boolean(bgTurn),
        elapsedMs: (typeof performance !== "undefined" ? performance.now() : Date.now()) - switchStartedAt
      });

      this.sessionOperation = null;
      this.sessionHydrating = false;
      reportSwitchPerf("host-refresh-skipped", {
        to: nextSessionId,
        reason: hasCachedState ? "cached-session" : "interactive-switch-no-host-read"
      });
    },
    async createSession() {
      if (this.sessionOperation || !hasPersistableMessages(this.messages)) {
        return;
      }

      // Register current running turn as a background turn before resetting
      if (this.isSubmitting && this.activeTurnId) {
        this.runningSessionMap[this.sessionId] = {
          turnId: this.activeTurnId,
          phase: this.phase,
          textBuffer: this.streamBufferText,
          reasoningBuffer: this.streamBufferReasoning
        };
        delete this.completedSessionSet[this.sessionId];
        delete this.failedSessionSet[this.sessionId];
      }

      const nextSessionId = createNextSessionId([
        this.sessionId,
        ...this.sessionList.map((session) => session.conversationId)
      ]);
      const previousSessionId = this.sessionId;
      this.persistHistory();
      const currentOverview = buildSessionOverviewFromRuntimeState(this);
      this.resetSessionRuntimeState();
      this.sessionId = nextSessionId;
      this.phase = "idle";
      this.sessionError = null;
      const dedupedCurrentSessionList = this.sessionList.filter((session) => session.conversationId !== nextSessionId);
      this.sessionList = ensureUniqueSessionList([
        createTransientSessionOverview(nextSessionId),
        ...(currentOverview ? [currentOverview] : []),
        ...dedupedCurrentSessionList.filter(
          (session) => session.conversationId !== currentOverview?.conversationId
            && session.conversationId !== previousSessionId
        )
      ]);
      debugLog("session:create:transient", {
        from: this.sessionList[1]?.conversationId ?? null,
        to: nextSessionId
      });
    },
    async deleteSession(targetSessionId: string) {
      if (this.isSubmitting || this.deletingSessionSet[targetSessionId]) {
        return;
      }

      const deletingActiveEmptySession =
        targetSessionId === this.sessionId && !hasPersistableMessages(this.messages);
      const requiresGlobalSessionOperation =
        deletingActiveEmptySession ||
        targetSessionId === this.sessionId ||
        !this.sessionList.some((session) => session.conversationId === this.sessionId);
      if (requiresGlobalSessionOperation && this.sessionOperation) {
        return;
      }
      const previousSnapshot = createSessionRuntimeSnapshot(this);
      const persistedStateToRestore = loadPersistedRuntimeState(targetSessionId);
      this.deletingSessionSet[targetSessionId] = true;
      if (requiresGlobalSessionOperation) {
        this.sessionOperation = "deleting";
      } else {
        this.sessionList = this.sessionList.filter((session) => session.conversationId !== targetSessionId);
      }
      this.sessionError = null;

      try {
        removePersistedSessionState(targetSessionId);
        delete this.runningSessionMap[targetSessionId];
        delete this.completedSessionSet[targetSessionId];
        delete this.failedSessionSet[targetSessionId];

        if (isTauriAvailable() && !deletingActiveEmptySession) {
          this.sessionList = filterDeletingSessions(
            await safeInvoke<SessionOverview[]>("delete_session", {
              sessionId: targetSessionId
            }),
            this.deletingSessionSet
          );
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
        delete this.deletingSessionSet[targetSessionId];
        if (requiresGlobalSessionOperation) {
          this.sessionOperation = null;
        }
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
        const restoredRunningMap = loadRunningSessionMap();
        if (Object.keys(restoredRunningMap).length > 0) {
          this.runningSessionMap = { ...restoredRunningMap };
          debugLog("session:init:running-map", { sessions: Object.keys(restoredRunningMap) });
        }
        const persistedCache = loadPersistedRuntimeCache();
        if (persistedCache.completedSet) {
          this.completedSessionSet = { ...persistedCache.completedSet };
        }
        if (persistedCache.failedSet) {
          this.failedSessionSet = { ...persistedCache.failedSet };
        }
        const preferredSessionId = this.sessionList[0]?.conversationId ?? this.sessionId;
        const persisted = loadPersistedRuntimeState(preferredSessionId);
        const strategy = deriveInitStrategy(preferredSessionId, persisted, this.sessionList);

        debugLog("session:init:strategy", { strategy: strategy.kind, preferredSessionId });

        switch (strategy.kind) {
          case "local-cache": {
            const cachedRuntimeView = buildRuntimeViewFromPersistedState(preferredSessionId, strategy.persistedState);
            const hostRuntimeView = await this.loadSessionRuntimeViewState(preferredSessionId).catch((error) => {
              debugLog("session:init:host:error", { sessionId: preferredSessionId, error: String(error) });
              return null;
            });
            const mergedRuntimeView = mergeRuntimeViews(cachedRuntimeView, hostRuntimeView);
            this.resetSessionRuntimeState();
            this.sessionId = preferredSessionId;
            this.phase = "connecting";
            await this.loadSessionState(preferredSessionId, {
              refreshCatalog: false,
              runtimeView: mergedRuntimeView
            });
            debugLog("session:init:cache", { sessionId: preferredSessionId, hostRefresh: hostRuntimeView != null });
            break;
          }
          case "host-read": {
            const runtimeView = await this.loadSessionRuntimeViewState(preferredSessionId);
            if (!runtimeView?.checkpoint && runtimeView?.session.history.length === 0) {
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
            debugLog("session:init:host", { sessionId: preferredSessionId, reason: strategy.reason });
            break;
          }
          case "empty-fallback": {
            this.resetSessionRuntimeState();
            this.sessionId = preferredSessionId;
            this.phase = "idle";
            debugLog("session:init:empty-fallback", { sessionId: preferredSessionId });
            break;
          }
        }
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
        this.eventCursorByTurnId = buildEventCursorByTurnTraceHistory(this.turnTraceHistory);
        if (persist) {
          this.persistHistory();
        }
        return;
      }

      // Pass raw values — normalizeTurnTraceRecord handles all deep cloning internally,
      // so pre-cloning here would result in redundant double-clones.
      this.turnTraceHistory.push(normalizeTurnTraceRecord({
        turnId,
        title: patch.title ?? "未命名轮次",
        phase: patch.phase ?? this.phase,
        traceSteps: patch.traceSteps ?? [],
        traceTimeline: patch.traceTimeline ?? [],
        toolActivities: patch.toolActivities ?? [],
        providerCallRecords: patch.providerCallRecords ?? [],
        hookTraceRecords: patch.hookTraceRecords ?? [],
        providerRequestedName: patch.providerRequestedName ?? null,
        providerName: patch.providerName ?? null,
        providerProtocol: patch.providerProtocol ?? null,
        providerModel: patch.providerModel ?? null,
        providerSource: patch.providerSource ?? null,
        providerMode: patch.providerMode ?? null,
        buildContextObservation: patch.buildContextObservation ?? null,
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
      this.eventCursorByTurnId = buildEventCursorByTurnTraceHistory(this.turnTraceHistory);
      if (persist) {
        this.persistHistory();
      }
    },
    shouldProcessTurnEvent(payload: Pick<TurnStreamEvent, "turnId" | "eventId" | "sequence" | "emittedAtMs">) {
      if (isHistoricalMode(this.historyCursorMode)) {
        return false;
      }
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
      // resolveEventTraceTimeline/buildFallbackRuntimeTraceTimeline already return a fresh timeline snapshot.
      this.traceTimeline = traceTimeline;
      this.upsertTurnTrace(turnId, {
        ...patch,
        traceTimeline: this.traceTimeline
      }, persist);
      // Lightweight debug log — full buildCacheTelemetryDebugSnapshot is already
      // called in STAGE 1 of each terminal event handler (completed/failed/cancelled),
      // so we avoid the redundant expensive computation here.
      debugLog("cache-telemetry:trace-committed", {
        turnId,
        phase: patch.phase ?? this.phase,
        traceTimelineLength: this.traceTimeline.length
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
      let modelIndex = -1;
      for (let i = traceTimeline.length - 1; i >= 0; i--) {
        if (canonicalizeTraceTimelineKind(traceTimeline[i]!.kind) === "call_model") {
          modelIndex = i;
          break;
        }
      }
      if (modelIndex === -1) {
        return;
      }

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
      if (finalText && payload.text !== assistantMessage.content) {
        assistantMessage.content = payload.text ?? assistantMessage.content;
      }
      const nextReasoningContent = normalizeReasoningContent(
        payload.reasoningContent ?? assistantMessage.reasoningContent ?? null
      );
      if (nextReasoningContent !== assistantMessage.reasoningContent) {
        assistantMessage.reasoningContent = nextReasoningContent;
      }
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
      const existingMessagesById = new Map<string, ChatMessage>();
      for (const message of this.messages) {
        if (message.role === "tool" && message.turnId === turnId) {
          existingMessagesById.set(message.id, message);
        }
      }
      const pendingMessages: ChatMessage[] = [];
      let didMutate = false;

      for (const tool of activeTools) {
        const messageId = `tool-${turnId}-${tool.id}`;
        const existingMessage = existingMessagesById.get(messageId);
        const nextContent = tool.resultText ?? "";
        const nextDetail = buildToolMessageDetail(tool);
        const nextStatus = toolStatusToMessageStatus(tool.status);
        const nextDurationSeconds = tool.durationSeconds ?? null;
        const nextCanonicalToolName = tool.canonicalToolName ?? null;
        const nextDisplayNameZh = tool.displayNameZh ?? null;

        if (existingMessage) {
          if (existingMessage.content !== nextContent) {
            existingMessage.content = nextContent;
            didMutate = true;
          }
          if (existingMessage.status !== nextStatus) {
            existingMessage.status = nextStatus;
            didMutate = true;
          }
          if (existingMessage.toolName !== tool.name) {
            existingMessage.toolName = tool.name;
            didMutate = true;
          }
          if (existingMessage.canonicalToolName !== nextCanonicalToolName) {
            existingMessage.canonicalToolName = nextCanonicalToolName;
            didMutate = true;
          }
          if (existingMessage.displayNameZh !== nextDisplayNameZh) {
            existingMessage.displayNameZh = nextDisplayNameZh;
            didMutate = true;
          }
          if (existingMessage.detail !== nextDetail) {
            existingMessage.detail = nextDetail;
            didMutate = true;
          }
          if (existingMessage.durationSeconds !== nextDurationSeconds) {
            existingMessage.durationSeconds = nextDurationSeconds;
            didMutate = true;
          }
          continue;
        }

        pendingMessages.push({
          id: messageId,
          turnId,
          role: "tool",
          content: nextContent,
          status: nextStatus,
          toolName: tool.name,
          canonicalToolName: nextCanonicalToolName,
          displayNameZh: nextDisplayNameZh,
          detail: nextDetail,
          durationSeconds: nextDurationSeconds
        });
        didMutate = true;
      }

      if (pendingMessages.length > 0) {
        this.messages.push(...pendingMessages);
      }

      if (persist && didMutate) {
        this.persistHistory();
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
        initFrontendFlightRecorder();
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
        const defaultToolNames = new Set(defaultAvailableTools.map((tool) => tool.name));
        const hasDefaultOnly =
          this.availableTools.length === defaultAvailableTools.length &&
          this.availableTools.every((tool) => defaultToolNames.has(tool.name));
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
          if (payload.sessionId) {
            const bg = this.runningSessionMap[payload.sessionId];
            if (bg && bg.turnId === payload.turnId) {
              const dt = payload.text ?? "";
              const dr = payload.reasoningContent ?? "";
              if (dt) bg.textBuffer += dt;
              if (dr) bg.reasoningBuffer += dr;
              if (bg.textBuffer.length > MAX_BG_TEXT_BUFFER_CHARS) {
                bg.textBuffer = bg.textBuffer.slice(0, MAX_BG_TEXT_BUFFER_CHARS);
              }
              if (bg.reasoningBuffer.length > MAX_BG_REASONING_BUFFER_CHARS) {
                bg.reasoningBuffer = bg.reasoningBuffer.slice(0, MAX_BG_REASONING_BUFFER_CHARS);
              }
            }
          }
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

        this.phase = resolveRuntimePhaseFromEvent(payload, this.phase);
        this.firstTokenLatencyMs = payload.firstTokenLatencyMs ?? this.firstTokenLatencyMs;
        if (payload.traceTimeline?.length) {
          this.updateActiveTraceTimeline(payload.traceTimeline);
        }
        if (!deltaText && !deltaReasoning) {
          return;
        }
        this.scheduleStreamFlush(payload.turnId);
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
        if (payload.sessionId) {
          const bg = this.runningSessionMap[payload.sessionId];
          if (bg && bg.turnId === payload.turnId) {
            bg.textBuffer = "";
            bg.reasoningBuffer = "";
            this.updatePersistedBackgroundSession(payload.sessionId, payload.turnId, {
              content: payload.text ?? null,
              reasoningContent: payload.reasoningContent ?? null,
              status: "done",
              modelName: buildAssistantModelLabel(payload.providerName, payload.providerModel),
              tokenCount: payload.outputTokens ?? null,
              phase: resolveRuntimePhaseFromEvent(payload, "completed"),
              providerRequestedName: payload.providerRequestedName ?? null,
              providerName: payload.providerName ?? null,
              providerProtocol: payload.providerProtocol ?? null,
              providerModel: payload.providerModel ?? null,
              providerSource: payload.providerSource ?? null,
              providerMode: payload.providerMode ?? null,
              fallbackReason: payload.fallbackReason ?? undefined,
              inputTokens: payload.inputTokens ?? null,
              outputTokens: payload.outputTokens ?? null,
              totalTokens: payload.totalTokens ?? null,
              firstTokenLatencyMs: payload.firstTokenLatencyMs ?? null
            });
            return;
          }
        }
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

      const hopCompleteUnlisten = await safeListen<TurnStreamEvent>("turn:hop_complete", ({ payload }) => {
        if (this.activeTurnId !== payload.turnId) {
          return;
        }
        if (!this.shouldProcessTurnEvent(payload)) {
          return;
        }
        this.commitTurnEventCursor(payload);
        this.flushBufferedStreamText(payload.turnId);
        this.inputTokens = payload.inputTokens ?? this.inputTokens;
        this.outputTokens = payload.outputTokens ?? this.outputTokens;
        this.totalTokens = payload.totalTokens ?? this.totalTokens;
        this.firstTokenLatencyMs = payload.firstTokenLatencyMs ?? this.firstTokenLatencyMs;
        debugLog("event:hop_complete", {
          turnId: payload.turnId,
          inputTokens: payload.inputTokens ?? null,
          outputTokens: payload.outputTokens ?? null
        });
      });

      const completedUnlisten = await safeListen<TurnStreamEvent>("turn:completed", ({ payload }) => {
        // Detect terminal event for a background session's turn
        if (payload.sessionId) {
          const bg = this.runningSessionMap[payload.sessionId];
          if (bg && bg.turnId === payload.turnId) {
            this.updatePersistedBackgroundSession(payload.sessionId, payload.turnId, {
              content: payload.text ?? null,
              reasoningContent: payload.reasoningContent ?? null,
              status: "done",
              modelName: buildAssistantModelLabel(payload.providerName, payload.providerModel),
              tokenCount: payload.outputTokens ?? null,
              phase: "ready",
              sessionSummary: payload.sessionSummary ?? null,
              providerRequestedName: payload.providerRequestedName ?? null,
              providerName: payload.providerName ?? null,
              providerProtocol: payload.providerProtocol ?? null,
              providerModel: payload.providerModel ?? null,
              providerSource: payload.providerSource ?? null,
              providerMode: payload.providerMode ?? null,
              fallbackReason: payload.fallbackReason ?? undefined,
              inputTokens: payload.inputTokens ?? null,
              outputTokens: payload.outputTokens ?? null,
              totalTokens: payload.totalTokens ?? null,
              firstTokenLatencyMs: payload.firstTokenLatencyMs ?? null
            });
            this.completedSessionSet[payload.sessionId] = true;
            delete this.runningSessionMap[payload.sessionId];
            return;
          }
        }
        if (this.activeTurnId !== payload.turnId) {
          return;
        }
        if (!this.shouldProcessTurnEvent(payload)) {
          return;
        }

        // ===== STAGE 1 (sync): Critical chat area mutations only =====
        this.commitTurnEventCursor(payload);
        this.flushBufferedStreamText(payload.turnId);

        const completedPayloadRecord = payload as Record<string, unknown>;

        const assistantMessage = this.ensureAssistantMessage(
          payload.turnId,
          buildAssistantModelLabel(payload.providerName, payload.providerModel)
        );

        const finalText = payload.text?.trim();
        if (finalText && payload.text !== assistantMessage.content) {
          assistantMessage.content = payload.text ?? assistantMessage.content;
        }
        const nextReasoningContent = normalizeReasoningContent(
          payload.reasoningContent ?? assistantMessage.reasoningContent ?? null
        );
        if (nextReasoningContent !== assistantMessage.reasoningContent) {
          assistantMessage.reasoningContent = nextReasoningContent;
        }
        assistantMessage.status = "done";
        assistantMessage.modelName = buildAssistantModelLabel(payload.providerName, payload.providerModel);

        // Pre-compute values for later stages (captured by closure)
        const completedPhase = resolveRuntimePhaseFromEvent(payload, "completed");
        const terminalToolActivities = resolveTerminalToolActivities(payload.toolActivities, this.toolActivities);
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

        // Yield to browser — let Vue flush reactivity + DOM for chat area
        window.setTimeout(() => {
          if (
            this.sessionId !== completedSessionId ||
            this.activeTurnId !== payload.turnId ||
            isHistoricalMode(this.historyCursorMode)
          ) {
            return;
          }

          // ===== STAGE 2 (setTimeout 0): Trace + metadata + UI unlock =====
          const nextPhase = completedPhase === "completed" ? "ready" : completedPhase;
          const nextTraceSteps = payload.traceSteps ?? this.traceSteps;
          const nextSessionSummary = payload.sessionSummary ?? this.sessionSummary;
          const nextProviderRequestedName = payload.providerRequestedName ?? this.providerRequestedName;
          const nextProviderName = payload.providerName ?? this.providerName;
          const nextProviderProtocol = payload.providerProtocol ?? this.providerProtocol;
          const nextProviderModel = payload.providerModel ?? this.providerModel;
          const nextProviderSource = payload.providerSource ?? this.providerSource;
          const nextProviderMode = payload.providerMode ?? this.providerMode;
          const nextInputTokens = payload.inputTokens ?? this.inputTokens;
          const nextOutputTokens = payload.outputTokens ?? this.outputTokens;
          const nextTotalTokens = payload.totalTokens ?? this.totalTokens;
          const nextFirstTokenLatencyMs = payload.firstTokenLatencyMs ?? this.firstTokenLatencyMs;
          this.applyTurnTokenStats(payload.turnId, payload.inputTokens, payload.outputTokens, false);
          this.syncToolMessages(payload.turnId, payload.toolActivities, false);

          const traceTimeline = resolveEventTraceTimeline(payload, () =>
            buildFallbackRuntimeTraceTimeline({
              turnId: payload.turnId,
              eventType: payload.eventType,
              messages: this.messages,
              phase: "completed",
              assistantMessage,
              toolActivities: terminalToolActivities,
              providerPatch: {
                providerName: nextProviderName,
                providerProtocol: nextProviderProtocol,
                providerModel: nextProviderModel,
                providerSource: nextProviderSource,
                providerMode: nextProviderMode
              },
              terminalState: "completed",
              fallbackReason: payload.fallbackReason ?? null,
              inputTokens: nextInputTokens,
              cacheHitInputTokens,
              reasoningTokens,
              outputTokens: nextOutputTokens,
              totalTokens: nextTotalTokens,
              firstTokenLatencyMs: nextFirstTokenLatencyMs,
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
            traceSteps: nextTraceSteps,
            toolActivities: terminalToolActivities,
            providerCallRecords: cloneProviderCallRecords(payload.providerCallRecords),
            providerRequestedName: nextProviderRequestedName,
            providerName: nextProviderName,
            providerProtocol: nextProviderProtocol,
            providerModel: nextProviderModel,
            providerSource: nextProviderSource,
            providerMode: nextProviderMode,
            buildContextObservation: cloneBuildContextObservation(payload.buildContextObservation),
            hookTraceRecords: cloneHookTraceRecords(payload.hookTraceRecords),
            sessionSummary: nextSessionSummary,
            fallbackReason: payload.fallbackReason ?? null,
            inputTokens: nextInputTokens,
            outputTokens: nextOutputTokens,
            totalTokens: nextTotalTokens,
            firstTokenLatencyMs: nextFirstTokenLatencyMs,
            ...cacheHitInputTokenPatch,
            ...reasoningTokenPatch,
            ...turnDurationPatch,
            error: null
          }, false);
          this.$patch((state) => {
            state.phase = nextPhase;
            state.traceSteps = nextTraceSteps;
            state.toolActivities = terminalToolActivities;
            state.sessionSummary = nextSessionSummary;
            state.providerRequestedName = nextProviderRequestedName;
            state.providerName = nextProviderName;
            state.providerProtocol = nextProviderProtocol;
            state.providerModel = nextProviderModel;
            state.providerSource = nextProviderSource;
            state.providerMode = nextProviderMode;
            state.fallbackReason = payload.fallbackReason ?? null;
            state.inputTokens = nextInputTokens;
            state.outputTokens = nextOutputTokens;
            state.totalTokens = nextTotalTokens;
            state.firstTokenLatencyMs = nextFirstTokenLatencyMs;
            state.isSubmitting = false;
            state.activeTurnId = null;
          });

          // ===== STAGE 3 (runLowPriorityTurnWork): Non-urgent async =====
          runLowPriorityTurnWork(() => {
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
        }, 0);
      });

      const failedUnlisten = await safeListen<TurnStreamEvent>("turn:failed", ({ payload }) => {
        if (payload.sessionId) {
          const bg = this.runningSessionMap[payload.sessionId];
          if (bg && bg.turnId === payload.turnId) {
            this.updatePersistedBackgroundSession(payload.sessionId, payload.turnId, {
              content: payload.text ?? DEFAULT_FAILED_TURN_MESSAGE,
              reasoningContent: payload.reasoningContent ?? null,
              status: "error",
              modelName: buildAssistantModelLabel(payload.providerName, payload.providerModel),
              tokenCount: payload.outputTokens ?? null,
              phase: "failed",
              sessionSummary: payload.sessionSummary ?? null,
              providerRequestedName: payload.providerRequestedName ?? null,
              providerName: payload.providerName ?? null,
              providerProtocol: payload.providerProtocol ?? null,
              providerModel: payload.providerModel ?? null,
              providerSource: payload.providerSource ?? null,
              providerMode: payload.providerMode ?? null,
              fallbackReason: payload.fallbackReason ?? undefined,
              inputTokens: payload.inputTokens ?? null,
              outputTokens: payload.outputTokens ?? null,
              totalTokens: payload.totalTokens ?? null,
              firstTokenLatencyMs: payload.firstTokenLatencyMs ?? null,
              errorDetail: payload.error ?? DEFAULT_FAILED_TURN_ERROR
            });
            this.failedSessionSet[payload.sessionId] = true;
            delete this.runningSessionMap[payload.sessionId];
            return;
          }
        }
        if (this.activeTurnId !== payload.turnId) {
          return;
        }
        if (!this.shouldProcessTurnEvent(payload)) {
          return;
        }

        // ===== STAGE 1 (sync): Critical chat area mutations only =====
        this.commitTurnEventCursor(payload);
        this.flushBufferedStreamText(payload.turnId);

        const assistantMessage = this.ensureAssistantMessage(
          payload.turnId,
          buildAssistantModelLabel(payload.providerName, payload.providerModel)
        );

        assistantMessage.reasoningContent = normalizeReasoningContent(payload.reasoningContent ?? null);
        assistantMessage.modelName = buildAssistantModelLabel(payload.providerName, payload.providerModel);

        // Pre-compute values for later stages
        const terminalToolActivities = resolveTerminalToolActivities(payload.toolActivities, this.toolActivities);
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
        const failedSessionId = this.sessionId;
        const failedRunId = this.activeRunId;
        const failedNodeId = this.visibleNodeId;

        // --- Retry check: retired per PA-070 contract ---
        // Frontend silent whole-turn auto-retry is retired.
        // Provider request-level retry is handled inside pony-agent-core.
        // If explicit turn-level retry is needed in the future, it must be
        // a separate control-plane orchestration action, not a silent frontend timer.

        // Standard failed path:
        assistantMessage.content = payload.text ?? DEFAULT_FAILED_TURN_MESSAGE;
        assistantMessage.status = "error";
        assistantMessage.errorDetail = payload.error ?? DEFAULT_FAILED_TURN_ERROR;

        this.phase = resolveRuntimePhaseFromEvent(payload, "failed");
        this.error = payload.error ?? DEFAULT_FAILED_TURN_ERROR;
        this.traceSteps = payload.traceSteps ?? this.traceSteps;
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
        this.isSubmitting = false;
        this.activeTurnId = null;
        this.activeRunId = null;
        const failedTraceTimeline = resolveEventTraceTimeline(payload, () =>
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
        this.commitTurnTraceTimeline(payload.turnId, failedTraceTimeline, {
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
        }, false);

        window.setTimeout(() => {
          if (this.sessionId !== failedSessionId || isHistoricalMode(this.historyCursorMode)) {
            return;
          }
          this.applyTurnTokenStats(payload.turnId, payload.inputTokens, payload.outputTokens, false);
          this.syncToolMessages(payload.turnId, payload.toolActivities, false);
          runLowPriorityTurnWork(() => {
            this.persistHistory();
            void this.loadRetrievedContextState(failedSessionId, {
              runId: failedRunId,
              nodeId: failedNodeId
            }).then((retrieved) => {
              if (this.sessionId === failedSessionId) {
                this.retrievedContext = retrieved;
              }
            });
          });

          debugLog("event:failed", {
            turnId: payload.turnId,
            error: this.error
          });
        }, 0);
      });

      const cancelledUnlisten = await safeListen<TurnStreamEvent>("turn:cancelled", ({ payload }) => {
        // Detect terminal event for a background session's turn
        if (payload.sessionId) {
          const bg = this.runningSessionMap[payload.sessionId];
          if (bg && bg.turnId === payload.turnId) {
            delete this.runningSessionMap[payload.sessionId];
            return;
          }
        }

        if (this.activeTurnId !== payload.turnId) {
          return;
        }
        if (!this.shouldProcessTurnEvent(payload)) {
          return;
        }

        // ===== STAGE 1 (sync): Critical chat area mutations only =====
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

        // Pre-compute values for later stages
        const terminalToolActivities = resolveTerminalToolActivities(payload.toolActivities, this.toolActivities);
        const cacheHitInputTokenPatch = cancelledCacheHitInputTokens != null ? { cacheHitInputTokens: cancelledCacheHitInputTokens } : {};
        const reasoningTokenPatch = cancelledReasoningTokens != null ? { reasoningTokens: cancelledReasoningTokens } : {};
        const turnDurationPatch = payload.turnDurationMs != null ? { turnDurationMs: payload.turnDurationMs } : {};
        const cancelledSessionId = this.sessionId;
        const cancelledRunId = this.activeRunId;
        const cancelledNodeId = this.visibleNodeId;

        // Keep terminal UI state consistent even before deferred trace work runs.
        this.phase = resolveRuntimePhaseFromEvent(payload, "cancelled");
        this.error = null;
        this.traceSteps = cancelledTraceSteps;
        this.toolActivities = terminalToolActivities;
        this.providerRequestedName = payload.providerRequestedName ?? this.providerRequestedName;
        this.providerName = payload.providerName ?? this.providerName;
        this.providerProtocol = payload.providerProtocol ?? this.providerProtocol;
        this.providerModel = payload.providerModel ?? this.providerModel;
        this.providerSource = payload.providerSource ?? this.providerSource;
        this.providerMode = payload.providerMode ?? this.providerMode;
        this.fallbackReason = payload.fallbackReason ?? this.fallbackReason;
        this.isSubmitting = false;
        this.activeTurnId = null;
        this.activeRunId = null;
        const cancelledTraceTimeline = resolveEventTraceTimeline(payload, () =>
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
        this.commitTurnTraceTimeline(payload.turnId, cancelledTraceTimeline, {
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
        }, false);

        // Yield to browser
        window.setTimeout(() => {
          if (this.sessionId !== cancelledSessionId || isHistoricalMode(this.historyCursorMode)) {
            return;
          }

          // ===== STAGE 2 (setTimeout 0): Metadata + trace + UI unlock =====
          this.applyTurnTokenStats(payload.turnId, payload.inputTokens, payload.outputTokens, false);
          this.syncToolMessages(payload.turnId, payload.toolActivities, false);

          // ===== STAGE 3 (runLowPriorityTurnWork): Non-urgent async =====
          runLowPriorityTurnWork(() => {
            this.persistHistory();
            void this.loadRetrievedContextState(cancelledSessionId, {
              runId: cancelledRunId,
              nodeId: cancelledNodeId
            }).then((retrieved) => {
              if (this.sessionId === cancelledSessionId) {
                this.retrievedContext = retrieved;
              }
            });
          });

          debugLog("event:cancelled", {
            turnId: payload.turnId
          });
        }, 0);
      });

      void startedUnlisten;
      void deltaUnlisten;
      void traceUnlisten;
      void phaseChangedUnlisten;
      void checkpointPersistedUnlisten;
      void toolUnlisten;
      void outputEndUnlisten;
      void hopCompleteUnlisten;
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
      if (this.isSubmitting) {
        return false;
      }

      const images = (options?.images ?? []).map((image) => ({ ...image }));
      const message = this.draftMessage.trim();

      if (!message.trim() && !images.length) {
        return false;
      }

      this.isSubmitting = true;

      await this.initializeTurnEvents();
      const providerStore = useProviderStore();
      const settingsStore = useSettingsStore();

      const mode = this.historyCursorMode;
      const needsRestore = this.initialRollbackActive || mode === "historical";
      this.historyCursorMode = "live";
      if (needsRestore && isTauriAvailable()) {
        const restored = await this.restoreBranchHead();
        if (!restored) {
          if (this.initialRollbackActive) {
            this.isSubmitting = false;
            this.sessionError = "无法提交：对话处于历史浏览模式，恢复最新状态失败";
            return false;
          }
        }
      }

      this.initialRollbackActive = false;

      const lastAssistantMessage = [...this.messages]
        .reverse()
        .find((entry) => entry.role === "assistant") ?? null;
      const retryingAfterTimeout = isRetryableError(lastAssistantMessage?.errorDetail);
      const providerMessage = buildProviderUserMessage(message, images);
      const displayMessage = buildDisplayedUserMessage(message, images);
      const payload: TurnInput = {
        message: providerMessage,
        displayMessage,
        providerId: providerStore.currentProvider?.id ?? null,
        modelId: providerStore.currentModel?.id ?? null,
        reasoningEffort: providerStore.currentReasoningEffort ?? null,
        workspaceMode: settingsStore.workspaceMode,
        sessionId: this.sessionId,
        nodeId: this.visibleNodeId,
        history: buildTurnHistory(this.messages),
        images
      };

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

      if (retryingAfterTimeout) {
        const assistantMessage = this.ensureAssistantMessage(
          requestId,
          buildAssistantModelLabel(
            providerStore.currentProvider?.name ?? null,
            providerStore.currentModel?.model ?? providerStore.currentModel?.name ?? null
          )
        );
        assistantMessage.content = TIMEOUT_RETRY_PENDING_MESSAGE;
        assistantMessage.reasoningContent = null;
        assistantMessage.status = "pending";
        assistantMessage.errorDetail = lastAssistantMessage?.errorDetail ?? DEFAULT_FAILED_TURN_ERROR;
      }

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
