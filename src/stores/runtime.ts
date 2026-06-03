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
  ExecutionCheckpoint,
  GraphRun,
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
  activeTurnId: string | null;
  activeRunId: string | null;
  visibleNodeId: string | null;
  branchHeadNodeId: string | null;
  activeBranchId: string | null;
  historyCursorMode: HistoryCursorMode;
  historyNodes: HistoryNode[];
  historyBranches: HistoryBranch[];
  eventsReady: boolean;
  deferredPersistTimerId: number | null;
};

type PersistedRuntimeState = {
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

function resolveEventTraceTimeline(
  payload: { traceTimeline?: TraceTimelineEntry[] | null },
  fallback: () => TraceTimelineEntry[]
) {
  const payloadTraceTimeline = clonePayloadTraceTimeline(payload);
  return payloadTraceTimeline.length ? payloadTraceTimeline : fallback();
}

function buildFallbackRuntimeTraceTimeline(options: {
  turnId: string;
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

  timeline.push(createTimelineEntry("input", 1, undefined, {
    state: "completed",
    text: userInputText
  }));
  let sequence = 2;
  if (traceUsesRetrieval(buildContextObservation)) {
    timeline.push(createTimelineEntry("prepare_retrieval", sequence, undefined, {
      state: "completed",
      ...providerPatch
    }));
    sequence += 1;
  }
  timeline.push(createTimelineEntry("build_context", sequence, undefined, {
    state: "completed",
    buildContextObservation,
    ...providerPatch
  }));
  sequence += 1;

  const isTerminal = terminalState != null;
  const isCallingTool = phase === "calling_tool";
  const modelHopCount = isTerminal || phase === "calling_model"
    ? topLevelTools.length + 1
    : Math.max(topLevelTools.length, 1);

  for (let modelIndex = 0; modelIndex < modelHopCount; modelIndex += 1) {
    const isLastModel = modelIndex === modelHopCount - 1;
    let modelState: TraceTimelineEntry["state"] = "completed";
    if (terminalState === "error" && isLastModel) {
      modelState = "error";
    } else if (terminalState === "cancelled" && isLastModel) {
      modelState = "cancelled";
    } else if (!isTerminal && phase === "calling_model" && isLastModel) {
      modelState = "active";
    }

    timeline.push(createTimelineEntry("call_model", sequence, modelIndex + 1, {
      state: modelState,
      text: null,
      reasoningContent: !isTerminal && phase === "calling_model" && isLastModel ? assistantMessage?.reasoningContent ?? null : null,
      firstTokenLatencyMs: !isTerminal && phase === "calling_model" && isLastModel ? firstTokenLatencyMs ?? null : null,
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
    workspaceRef: node.workspaceRef ? { ...node.workspaceRef } : node.workspaceRef ?? null
  }));
}

function cloneHistoryBranches(branches?: HistoryBranch[] | null) {
  return (branches ?? []).map((branch) => ({ ...branch }));
}

function cloneHistoryCursor(cursor?: HistoryCursorState | null) {
  return cursor ? { ...cursor } : null;
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
    providerCallRecords: cloneProviderCallRecords(trace.providerCallRecords)
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
      turnTraceHistory: state.turnTraceHistory.map((trace) => normalizeTurnTraceRecord(trace))
  };
}

function restoreSessionRuntimeSnapshot(state: RuntimeState, snapshot: SessionRuntimeSnapshot) {
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
    updatedAtMs: 0
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

function normalizeCheckpointPhase(checkpoint: ExecutionCheckpoint): RuntimePhase {
  const phase = checkpoint.phase.trim().toLowerCase().replace(/-/g, "_");

  switch (phase) {
    case "idle":
    case "connecting":
    case "ready":
    case "completed":
    case "cancelled":
    case "calling_model":
    case "calling_tool":
    case "failed":
      return phase;
    default:
      break;
  }

  const status = checkpoint.status.trim().toLowerCase();
  if (status === "cancelled") {
    return "cancelled";
  }

  if (status === "failed") {
    return "failed";
  }

  if (
    checkpoint.activeToolName?.trim() ||
    checkpoint.toolActivities.some((tool) => tool.status === "running")
  ) {
    return "calling_tool";
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

function hydrateMessagesFromHistory(
  history: TurnHistoryMessage[],
  persistedMessages?: ChatMessage[] | null
): ChatMessage[] {
  const messages: ChatMessage[] = [];
  const restoredHistoryMessages = collectPersistedHistoryMessages(persistedMessages);
  const toolMessagesByTurnId = new Map<string, ChatMessage[]>();
  let currentTurnId: string | null = null;
  let turnIndex = 0;

  for (const message of persistedMessages ?? []) {
    if (message.role !== "tool") {
      continue;
    }

    const turnMessages = toolMessagesByTurnId.get(message.turnId) ?? [];
    turnMessages.push({ ...message });
    toolMessagesByTurnId.set(message.turnId, turnMessages);
  }

  const appendToolMessagesForTurn = (turnId: string | null) => {
    if (!turnId) {
      return;
    }

    const toolMessages = toolMessagesByTurnId.get(turnId);
    if (!toolMessages?.length) {
      return;
    }

    messages.push(...toolMessages.map((message) => ({ ...message })));
    toolMessagesByTurnId.delete(turnId);
  };

  for (const item of history) {
    const restoredMessage = restoredHistoryMessages[messages.filter((message) => message.role !== "tool").length];

    if (item.role === "user") {
      currentTurnId = restoredMessage?.turnId ?? createHistoryTurnId(turnIndex);
      turnIndex += 1;
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

    if (!currentTurnId) {
      currentTurnId = restoredMessage?.turnId ?? createHistoryTurnId(turnIndex);
      turnIndex += 1;
    }

    messages.push({
      id: restoredMessage?.id ?? `history-assistant-${turnIndex}`,
      turnId: currentTurnId,
      role: "assistant",
      content: item.content,
      attachments: [],
      status: "done",
      reasoningContent: restoredMessage?.reasoningContent ?? null,
      tokenCount: restoredMessage?.tokenCount ?? null,
      modelName: restoredMessage?.modelName ?? null
    });
    appendToolMessagesForTurn(currentTurnId);
    currentTurnId = null;
  }

  appendToolMessagesForTurn(currentTurnId);
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
      phase: persisted?.messages?.length ? "ready" : "idle",
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
      visibleNodeId: null,
      branchHeadNodeId: null,
      activeBranchId: null,
      historyCursorMode: "live",
      historyNodes: [],
      historyBranches: [],
      eventsReady: false,
      deferredPersistTimerId: null,
      messages: persisted?.messages ?? [],
      attachmentAssets: persisted?.attachmentAssets ?? [],
      availableTools: createAvailableTools(),
      capabilitySources: createCapabilitySources(),
      capabilities: createCapabilities(),
      toolActivities: [],
      traceSteps: createDefaultTraceSteps(),
      traceTimeline: createDefaultTraceTimeline(),
      turnTraceHistory: (persisted?.turnTraceHistory ?? []).map((trace) => normalizeTurnTraceRecord(trace))
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
    },
    cancelDeferredPersist() {
      if (this.deferredPersistTimerId == null) {
        return;
      }

      window.clearTimeout(this.deferredPersistTimerId);
      this.deferredPersistTimerId = null;
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
        lastReferencedFile: null,
        updatedAtMs:
          persisted && persisted.turnTraceHistory.length > 0
            ? persisted.turnTraceHistory[persisted.turnTraceHistory.length - 1].updatedAt
            : Date.now()
      } satisfies SessionSnapshot;

      return {
        session: snapshot,
        retrieved: deriveRetrievedContextFromSnapshot(snapshot),
        checkpoint: null,
        historyNodes: undefined,
        historyBranches: undefined,
        historyCursor: null
      } satisfies SessionRuntimeView;
    },
    applyExecutionCheckpoint(
      checkpoint: ExecutionCheckpoint | null,
      persistedMessages?: ChatMessage[] | null
    ) {
      if (!checkpoint || checkpoint.status.trim().toLowerCase() !== "running") {
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
      assistantMessage.status = "pending";
      assistantMessage.modelName = modelLabel;

      this.phase = normalizeCheckpointPhase(checkpoint);
      this.error = checkpoint.error ?? null;
      this.isSubmitting = true;
      this.activeTurnId = checkpoint.turnId;
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
        phase: this.phase
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
      runtimeView?: Pick<SessionRuntimeView, "historyNodes" | "historyBranches" | "historyCursor"> | null
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

      this.sessionId = sessionId;
      this.error = null;
      this.isSubmitting = false;
      this.activeTurnId = null;
      this.activeRunId = retrieved?.runState?.runId?.trim() || null;
      this.draftMessage = "";
      this.sessionSummary = sessionSummary;
      this.retrievedContext = cloneRetrievedContext(retrieved ?? deriveRetrievedContextFromSnapshot(snapshot));
      this.messages = hydrateMessagesFromHistory(
        snapshot.history,
        canMergePersistedMessages ? persisted?.messages : null
      );
      this.attachmentAssets = snapshot.attachmentAssets ?? restoredState?.attachmentAssets ?? [];
      this.turnTraceHistory = (
        snapshotTurnTraceHistory.length
          ? snapshotTurnTraceHistory
          : restoredState?.turnTraceHistory ?? []
      ).map((trace) => normalizeTurnTraceRecord(trace));
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
      this.phase = this.messages.length ? "ready" : "idle";
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
        result = await safeInvoke<HistoryCheckoutResult>("checkout_history_node", {
          sessionId,
          nodeId,
          mode
        });
      } else {
        result = {
          sessionId,
          visibleNodeId: nodeId,
          activeBranchId: this.activeBranchId,
          branchHeadNodeId: this.branchHeadNodeId,
          workspaceNodeId: this.visibleNodeId,
          mode:
            this.branchHeadNodeId && this.branchHeadNodeId !== nodeId ? "historical" : "live",
          requestedMode: mode,
          appliedMode: "transcript_only",
          workspaceRestoreApplied: false,
          degradedToTranscriptOnly: mode === "transcript_and_workspace",
          historyNodes: this.historyNodes,
          historyBranches: this.historyBranches
        };
      }

      await this.loadSessionState(sessionId, {
        refreshCatalog: false,
        nodeId
      });
      this.applyHistoryState(sessionId, result);
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
        result = await safeInvoke<HistoryRestoreResult>("restore_branch_head", {
          sessionId,
          branchId: targetBranchId ?? null
        });
      } else {
        const branchHeadNodeId =
          resolveHistoryBranchHeadNodeId(targetBranchId, this.historyBranches) ?? this.branchHeadNodeId;
        result = {
          sessionId,
          visibleNodeId: branchHeadNodeId,
          activeBranchId: targetBranchId,
          branchHeadNodeId,
          workspaceNodeId: branchHeadNodeId,
          mode: "live",
          restoredFromNodeId: this.visibleNodeId,
          historyNodes: this.historyNodes,
          historyBranches: this.historyBranches
        };
      }

      await this.loadSessionState(sessionId, {
        refreshCatalog: false,
        nodeId: result.visibleNodeId ?? result.branchHeadNodeId ?? null
      });
      this.applyHistoryState(sessionId, result);
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
        result = await safeInvoke<HistoryForkResult>("fork_from_history_node", {
          sessionId,
          nodeId: targetNodeId
        });
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
          visibleNodeId: targetNodeId,
          activeBranchId: createdBranchId,
          branchHeadNodeId: targetNodeId,
          workspaceNodeId: this.visibleNodeId,
          mode: "live",
          forkedFromNodeId: targetNodeId,
          forkedFromBranchId: this.activeBranchId,
          createdBranchId,
          historyNodes: this.historyNodes,
          historyBranches: [...this.historyBranches, nextBranch]
        };
      }

      this.applyHistoryState(sessionId, result);
      return result;
    },
    async switchHistoryBranch(branchId: string) {
      const sessionId = this.sessionId;
      if (!sessionId || !branchId.trim()) {
        return null;
      }

      let result: HistoryBranchSwitchResult;
      if (isTauriAvailable()) {
        result = await safeInvoke<HistoryBranchSwitchResult>("switch_history_branch", {
          sessionId,
          branchId
        });
      } else {
        const branchHeadNodeId = resolveHistoryBranchHeadNodeId(branchId, this.historyBranches);
        result = {
          sessionId,
          visibleNodeId: branchHeadNodeId,
          activeBranchId: branchId,
          branchHeadNodeId,
          workspaceNodeId: branchHeadNodeId,
          mode: "live",
          previousBranchId: this.activeBranchId,
          historyNodes: this.historyNodes,
          historyBranches: this.historyBranches
        };
      }

      await this.loadSessionState(sessionId, {
        refreshCatalog: false,
        nodeId: result.visibleNodeId ?? result.branchHeadNodeId ?? null
      });
      this.applyHistoryState(sessionId, result);
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
      await this.switchSession(nextSessionId);
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
      patch: Partial<Omit<TurnTraceRecord, "turnId" | "updatedAt">> & { updatedAt?: number }
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
              : existing.providerCallRecords
        });
        this.persistHistory();
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
      this.persistHistory();
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
      patch: Partial<Omit<TurnTraceRecord, "turnId" | "updatedAt">> & { updatedAt?: number } = {}
    ) {
      this.traceTimeline = cloneTraceTimeline(traceTimeline);
      this.upsertTurnTrace(turnId, {
        ...patch,
        traceTimeline: this.traceTimeline
      });
    },
    applyTurnTokenStats(turnId: string, inputTokens?: number | null, outputTokens?: number | null) {
      const userMessage = this.messages.find((item) => item.turnId === turnId && item.role === "user");
      const assistantMessage = this.messages.find((item) => item.turnId === turnId && item.role === "assistant");

      if (userMessage && inputTokens != null) {
        userMessage.tokenCount = inputTokens;
      }

      if (assistantMessage && outputTokens != null) {
        assistantMessage.tokenCount = outputTokens;
      }

      this.persistHistory();
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
      return assistantMessage;
    },
    syncToolMessages(turnId: string, toolActivities?: ToolActivity[] | null) {
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
          this.persistHistory();
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

        this.ensureAssistantMessage(
          payload.turnId,
          buildAssistantModelLabel(payload.providerName, payload.providerModel)
        );
        this.phase = "calling_model";
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
        const cacheHitInputTokens = resolveCacheHitInputTokens(payload);
        const reasoningTokens = resolveReasoningTokens(payload);
        const cacheHitInputTokenPatch = cacheHitInputTokens != null ? { cacheHitInputTokens } : {};
        const reasoningTokenPatch = reasoningTokens != null ? { reasoningTokens } : {};
        const turnDurationPatch = payload.turnDurationMs != null ? { turnDurationMs: payload.turnDurationMs } : {};
        this.traceSteps = payload.traceSteps ?? this.traceSteps;
        this.traceTimeline = resolveEventTraceTimeline(payload, () =>
          buildFallbackRuntimeTraceTimeline({
            turnId: payload.turnId,
            messages: this.messages,
            phase: "calling_model",
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
        this.syncToolMessages(payload.turnId, payload.toolActivities);
        this.commitTurnTraceTimeline(payload.turnId, this.traceTimeline, {
          phase: "calling_model",
          traceSteps: payload.traceSteps ?? this.traceSteps,
          toolActivities: payload.toolActivities ?? this.toolActivities,
          providerRequestedName: payload.providerRequestedName ?? this.providerRequestedName,
          providerName: payload.providerName ?? this.providerName,
          providerProtocol: payload.providerProtocol ?? this.providerProtocol,
          providerModel: payload.providerModel ?? this.providerModel,
          providerSource: payload.providerSource ?? this.providerSource,
          providerMode: payload.providerMode ?? this.providerMode,
          buildContextObservation: cloneBuildContextObservation(payload.buildContextObservation),
          fallbackReason: payload.fallbackReason ?? this.fallbackReason,
          inputTokens: payload.inputTokens ?? this.inputTokens,
          outputTokens: payload.outputTokens ?? this.outputTokens,
          totalTokens: payload.totalTokens ?? this.totalTokens,
          firstTokenLatencyMs: payload.firstTokenLatencyMs ?? this.firstTokenLatencyMs,
          ...cacheHitInputTokenPatch,
          ...reasoningTokenPatch,
          ...turnDurationPatch,
          error: null
        });
      });

      const deltaUnlisten = await safeListen<TurnStreamEvent>("turn:delta", ({ payload }) => {
        if (this.activeTurnId !== payload.turnId) {
          return;
        }

        const assistantMessage = this.ensureAssistantMessage(
          payload.turnId,
          buildAssistantModelLabel(this.providerName, this.providerModel)
        );

        const deltaText = payload.text ?? "";
        const deltaReasoning = payload.reasoningContent ?? "";

        if (deltaReasoning) {
          assistantMessage.reasoningContent = normalizeReasoningContent(
            `${assistantMessage.reasoningContent ?? ""}${deltaReasoning}`
          );
        }

        if (deltaText) {
          assistantMessage.content += deltaText;
        }

        this.firstTokenLatencyMs = payload.firstTokenLatencyMs ?? this.firstTokenLatencyMs;
        const traceTimeline = resolveEventTraceTimeline(payload, () =>
          buildFallbackRuntimeTraceTimeline({
            turnId: payload.turnId,
            messages: this.messages,
            phase: "calling_model",
            assistantMessage,
            toolActivities: this.toolActivities,
            providerPatch: {
              providerName: this.providerName,
              providerProtocol: this.providerProtocol,
              providerModel: this.providerModel,
              providerSource: this.providerSource,
              providerMode: this.providerMode
            },
            firstTokenLatencyMs: payload.firstTokenLatencyMs ?? this.firstTokenLatencyMs
          })
        );
        this.commitTurnTraceTimeline(payload.turnId, traceTimeline, {
          phase: "calling_model",
          firstTokenLatencyMs: payload.firstTokenLatencyMs ?? this.firstTokenLatencyMs,
          error: null
        });
        this.scheduleDeferredPersist();
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

        this.phase = payload.phase === "calling_tool" ? "calling_tool" : "calling_model";
        this.traceSteps = payload.traceSteps ?? this.traceSteps;
        const traceTimeline = resolveEventTraceTimeline(payload, () =>
          buildFallbackRuntimeTraceTimeline({
            turnId: payload.turnId,
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
        this.commitTurnTraceTimeline(payload.turnId, traceTimeline, {
          phase: this.phase,
          traceSteps: payload.traceSteps ?? this.traceSteps,
          toolActivities: this.toolActivities,
          error: null
        });
        debugLog("event:trace", {
          turnId: payload.turnId,
          steps: this.traceSteps.length
        });
      });

      const toolUnlisten = await safeListen<TurnStreamEvent>("turn:tool", ({ payload }) => {
        if (this.activeTurnId !== payload.turnId) {
          return;
        }

        this.phase = payload.phase === "calling_model" ? "calling_model" : "calling_tool";
        debugLog("event:tool", {
          turnId: payload.turnId,
          tools: (payload.toolActivities ?? []).length
        });
        this.toolActivities = payload.toolActivities ?? this.toolActivities;
        this.syncToolMessages(payload.turnId, payload.toolActivities);
        const traceTimeline = resolveEventTraceTimeline(payload, () =>
          buildFallbackRuntimeTraceTimeline({
            turnId: payload.turnId,
            messages: this.messages,
            phase: this.phase,
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
        this.commitTurnTraceTimeline(payload.turnId, traceTimeline, {
          phase: this.phase,
          traceSteps: this.traceSteps,
          toolActivities: payload.toolActivities ?? this.toolActivities,
          error: null
        });
      });

      const completedUnlisten = await safeListen<TurnStreamEvent>("turn:completed", ({ payload }) => {
        if (this.activeTurnId !== payload.turnId) {
          return;
        }

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

        this.phase = "ready";
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
        const cacheHitInputTokens = resolveCacheHitInputTokens(payload);
        const reasoningTokens = resolveReasoningTokens(payload);
        const cacheHitInputTokenPatch = cacheHitInputTokens != null ? { cacheHitInputTokens } : {};
        const reasoningTokenPatch = reasoningTokens != null ? { reasoningTokens } : {};
        const turnDurationPatch = payload.turnDurationMs != null ? { turnDurationMs: payload.turnDurationMs } : {};
        const traceTimeline = resolveEventTraceTimeline(payload, () =>
          buildFallbackRuntimeTraceTimeline({
            turnId: payload.turnId,
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
        this.applyTurnTokenStats(payload.turnId, payload.inputTokens, payload.outputTokens);
        this.syncToolMessages(payload.turnId, payload.toolActivities);
        this.commitTurnTraceTimeline(payload.turnId, traceTimeline, {
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
        });
        this.persistHistory();
        void this.loadSessionCatalog();
        void this.loadRetrievedContextState(this.sessionId, {
          runId: this.activeRunId,
          nodeId: this.visibleNodeId
        }).then((retrieved) => {
          this.retrievedContext = retrieved;
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
        this.isSubmitting = false;
        this.activeTurnId = null;
      });

      const failedUnlisten = await safeListen<TurnStreamEvent>("turn:failed", ({ payload }) => {
        if (this.activeTurnId !== payload.turnId) {
          return;
        }

        const assistantMessage = this.ensureAssistantMessage(
          payload.turnId,
          buildAssistantModelLabel(payload.providerName, payload.providerModel)
        );

        assistantMessage.content = payload.text ?? DEFAULT_FAILED_TURN_MESSAGE;
        assistantMessage.reasoningContent = normalizeReasoningContent(payload.reasoningContent ?? null);
        assistantMessage.status = "error";
        assistantMessage.modelName = buildAssistantModelLabel(payload.providerName, payload.providerModel);

        this.phase = "failed";
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
        const cacheHitInputTokens = resolveCacheHitInputTokens(payload);
        const reasoningTokens = resolveReasoningTokens(payload);
        const cacheHitInputTokenPatch = cacheHitInputTokens != null ? { cacheHitInputTokens } : {};
        const reasoningTokenPatch = reasoningTokens != null ? { reasoningTokens } : {};
        const turnDurationPatch = payload.turnDurationMs != null ? { turnDurationMs: payload.turnDurationMs } : {};
        const traceTimeline = resolveEventTraceTimeline(payload, () =>
          buildFallbackRuntimeTraceTimeline({
            turnId: payload.turnId,
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

        const cancelledTraceSteps = finalizeCancelledTraceSteps(payload.traceSteps ?? this.traceSteps);
        const cancelledCacheHitInputTokens = resolveCacheHitInputTokens(payload);
        const cancelledReasoningTokens = resolveReasoningTokens(payload);

        const assistantMessage = this.ensureAssistantMessage(
          payload.turnId,
          buildAssistantModelLabel(payload.providerName, payload.providerModel)
        );

        assistantMessage.content = payload.text ?? "本轮已停止。";
        assistantMessage.reasoningContent = normalizeReasoningContent(payload.reasoningContent ?? null);
        assistantMessage.status = "done";
        assistantMessage.modelName = buildAssistantModelLabel(payload.providerName, payload.providerModel);

        this.phase = "cancelled";
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
      void toolUnlisten;
      void completedUnlisten;
      void failedUnlisten;
      void cancelledUnlisten;
      this.eventsReady = true;
    },
    async runBrowserPreviewTurn(requestId: string) {
      const providerStore = useProviderStore();
      const provider = providerStore.currentProvider;
      const model = providerStore.currentModel;
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
      const assistantMessage = this.ensureAssistantMessage(requestId, assistantModelLabel);
      assistantMessage.content = "";

      for (const chunk of BROWSER_PREVIEW_CHUNKS) {
        await wait(80);
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
      this.commitTurnTraceTimeline(requestId, this.traceTimeline, {
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
    },
    async stopTurn() {
      if (!this.activeTurnId || !this.isSubmitting || !isTauriAvailable()) {
        return false;
      }

      try {
        if (this.activeRunId) {
          await safeInvoke("stop_graph_run", {
            runId: this.activeRunId
          });
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

        let submission = resolveGraphRunSubmissionFromRunState(this.retrievedContext?.runState);
        if (!submission && this.sessionId) {
          await this.resolveDerivedSessionRun({
            sessionId: this.sessionId,
            runId: this.activeRunId,
            preferRefresh: true,
            nodeId: this.visibleNodeId
          });
          submission = resolveGraphRunSubmissionFromRunState(this.retrievedContext?.runState);
        }
        submission ??= { command: "start_graph_run_stream" as const, runId: null };

        if (submission.command === "start_graph_run_stream") {
          const response = await safeInvoke<GraphRunStreamStartResponse>("start_graph_run_stream", {
            turnId: requestId,
            runId: null,
            goal: displayMessage,
            input: payload
          });
          this.activeRunId = response.run.id;
          return true;
        }

        if (submission.command === "resume_graph_run_stream") {
          const response = await safeInvoke<GraphRunStreamStartResponse>("resume_graph_run_stream", {
            turnId: requestId,
            runId: submission.runId,
            input: payload
          });
          this.activeRunId = response.run.id;
          return true;
        }

        const response = await safeInvoke<GraphRunStreamStartResponse>("continue_graph_run_stream", {
          turnId: requestId,
          runId: submission.runId,
          input: payload
        });
        this.activeRunId = response.run.id;
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
