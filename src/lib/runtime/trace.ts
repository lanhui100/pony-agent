// trace / timeline domain：trace 记录、timeline 构建、token 指标解析与克隆归一化。
// 依赖 phases（phase 归一化）与 utils（日志），不反向依赖任何上层模块。
import type {
  BuildContextObservation,
  ChatMessage,
  GraphRunStreamStartResponse,
  HookPatchOperation,
  HookStructuredResult,
  HookTraceRecord,
  ProviderCallCacheRecord,
  RuntimePhase,
  ToolActivity,
  TraceStep,
  TraceTimelineEntry,
  TurnStreamEvent,
  TurnTraceRecord
} from "../../types/runtime";
import { errorLog } from "./utils";
import { resolveFallbackTimelineRuntimePhase } from "./phases";

export function toolStatusToMessageStatus(status: ToolActivity["status"]): ChatMessage["status"] {
  switch (status) {
    case "done":
      return "done";
    case "error":
      return "error";
    default:
      return "pending";
  }
}

export function cloneTraceSteps(traceSteps?: TraceStep[] | null) {
  return (traceSteps ?? [])
    .filter((step) => step.id !== "step-return")
    .map((step) => ({ ...step }));
}

export function canonicalizeTraceTimelineKind(kind: TraceTimelineEntry["kind"]): TraceTimelineEntry["kind"] {
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

export function cloneTraceTimeline(traceTimeline?: TraceTimelineEntry[] | null): TraceTimelineEntry[] {
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

export function cloneProviderCallRecords(
  providerCallRecords?: ProviderCallCacheRecord[] | null
): ProviderCallCacheRecord[] {
  return (providerCallRecords ?? []).map((record) => ({
    ...record,
    prefixMutationReasons: [...(record.prefixMutationReasons ?? [])]
  }));
}

export function cloneHookPatchOperations(operations?: HookPatchOperation[] | null): HookPatchOperation[] {
  return (operations ?? []).map((operation) => ({ ...operation }));
}

export function cloneHookStructuredResult(result: HookStructuredResult): HookStructuredResult {
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

export function cloneHookTraceRecords(hookTraceRecords?: HookTraceRecord[] | null): HookTraceRecord[] {
  return (hookTraceRecords ?? []).map((record) => ({
    ...record,
    structuredResult: cloneHookStructuredResult(record.structuredResult)
  }));
}

export function cloneToolActivities(toolActivities?: ToolActivity[] | null) {
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

export function cloneBuildContextObservation(buildContextObservation?: TurnTraceRecord["buildContextObservation"]) {
  return buildContextObservation ? { ...buildContextObservation } : null;
}

export function clonePayloadTraceTimeline(payload: { traceTimeline?: TraceTimelineEntry[] | null }) {
  return cloneTraceTimeline(payload.traceTimeline);
}

export function resolvedStreamStartRunId(
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

export function resolveEventTraceTimeline(
  payload: { traceTimeline?: TraceTimelineEntry[] | null },
  fallback: () => TraceTimelineEntry[]
) {
  const payloadTraceTimeline = clonePayloadTraceTimeline(payload);
  return payloadTraceTimeline.length ? payloadTraceTimeline : fallback();
}

// 低频语义事件（turn:trace / phase_changed / checkpoint_persisted / tool）的 timeline 解析：
// Rust 端已不再携带全量 timeline（IPC 瘦身），payload 无 timeline 时优先保留当前
// timeline（真实数据 + 避免重建/冗余克隆），仅在当前 timeline 为空时走 fallback 兜底。
export function resolveSemanticEventTraceTimeline(
  payload: { traceTimeline?: TraceTimelineEntry[] | null },
  fallback: () => TraceTimelineEntry[],
  currentTimeline: TraceTimelineEntry[]
): TraceTimelineEntry[] {
  if (payload.traceTimeline?.length) {
    return cloneTraceTimeline(payload.traceTimeline);
  }
  if (currentTimeline.length) {
    return currentTimeline;
  }
  return fallback();
}

export function buildFallbackRuntimeTraceTimeline(options: {
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

export function normalizeReasoningContent(content?: string | null) {
  if (content == null) {
    return null;
  }

  const normalized = content.replace(/^thinking\s*[:：]\s*/i, "");
  return normalized.length > 0 ? normalized : null;
}

export function appendNormalizedReasoningContent(current: string | null, delta: string) {
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

const TRACE_STEP_LABELS = {
  plan: "接收输入",
  context: "组织上下文",
  contextBrowser: "识别运行环境",
  callModel: "调用模型",
  callModelBrowser: "浏览器预览回放",
  callTool: "调用工具",
  return: "返回结果"
} as const;

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

export function buildTraceSteps(entries: Array<{ key: TraceStepKey; state: TraceStepState }>) {
  return entries.map(({ key, state }) => ({
    id: TRACE_STEP_IDS[key],
    label: TRACE_STEP_LABELS[key],
    state
  }));
}

export function createDefaultTraceSteps() {
  return buildTraceSteps([
    { key: "plan", state: "completed" },
    { key: "context", state: "active" },
    { key: "callModel", state: "pending" },
    { key: "callTool", state: "pending" }
  ]);
}

export function createSubmitTraceSteps() {
  return buildTraceSteps([
    { key: "plan", state: "completed" },
    { key: "context", state: "completed" },
    { key: "callModel", state: "active" },
    { key: "callTool", state: "pending" }
  ]);
}

export function createBrowserPreviewTraceSteps() {
  return buildTraceSteps([
    { key: "plan", state: "completed" },
    { key: "contextBrowser", state: "completed" },
    { key: "callModelBrowser", state: "completed" },
    { key: "callTool", state: "pending" }
  ]);
}

export function createSubmitFailureTraceSteps() {
  return buildTraceSteps([
    { key: "plan", state: "completed" },
    { key: "context", state: "completed" },
    { key: "callModel", state: "error" },
    { key: "callTool", state: "pending" }
  ]);
}

export function finalizeCancelledTraceSteps(traceSteps?: TraceStep[] | null): TraceStep[] {
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

export function createTimelineEntry(
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

export function createDefaultTraceTimeline() {
  return [
    createTimelineEntry("build_context", 1, undefined, { state: "completed" }),
    createTimelineEntry("call_model", 2, 1, { state: "active" })
  ];
}

export function applyProviderPatchToTraceTimeline(
  traceTimeline: TraceTimelineEntry[],
  providerPatch: Pick<TraceTimelineEntry, "providerName" | "providerProtocol" | "providerModel">
) {
  return traceTimeline.map((entry) => {
    const kind = canonicalizeTraceTimelineKind(entry.kind);
    if (kind === "call_tool") {
      return { ...entry };
    }

    return {
      ...entry,
      ...providerPatch
    };
  });
}

export function createBrowserPreviewTraceTimeline() {
  return [
    createTimelineEntry("input", 1, undefined, { state: "completed" }),
    createTimelineEntry("build_context", 2, undefined, { state: "completed" }),
    createTimelineEntry("call_model", 3, 1, { state: "completed" })
  ];
}

export function createSubmitFailureTraceTimeline() {
  return [
    createTimelineEntry("input", 1, undefined, { state: "completed" }),
    createTimelineEntry("build_context", 2, undefined, { state: "completed" }),
    createTimelineEntry("call_model", 3, 1, { state: "error" })
  ];
}

export function traceUsesRetrieval(buildContextObservation?: BuildContextObservation | null) {
  if (!buildContextObservation) {
    return false;
  }

  return (
    buildContextObservation.messageCount > 2 ||
    (buildContextObservation.prefixMutationReasons?.length ?? 0) > 0 ||
    buildContextObservation.semiStableContextText.trim().length > 0
  );
}

export function toolActivitiesForHop(toolActivities: ToolActivity[] | null | undefined, parentId?: string | null) {
  if (!toolActivities?.length || !parentId) {
    return [];
  }

  return toolActivities.filter((activity) => activity.id === parentId || activity.id.startsWith(`${parentId}-`));
}

export function resolveTerminalToolActivities(
  payloadToolActivities: ToolActivity[] | null | undefined,
  currentToolActivities: ToolActivity[] | null | undefined
) {
  return payloadToolActivities?.length ? payloadToolActivities : (currentToolActivities ?? []);
}

export function deriveTraceTimelineFromLegacyTrace(turn: TurnTraceRecord) {
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

export function normalizeTurnTraceRecord(trace: TurnTraceRecord): TurnTraceRecord {
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

export function buildAssistantModelLabel(providerName?: string | null, modelName?: string | null) {
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

export function readNestedNumericTokenValue(source: unknown, paths: string[][]): number | null {
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

export function resolveCacheHitInputTokens(source: unknown): number | null {
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

export function resolveProviderReturnedCacheHitInputTokens(source: { providerCallRecords?: ProviderCallCacheRecord[] | null }): number | null {
  const values = (source.providerCallRecords ?? [])
    .map((record) => record.cacheHitInputTokens)
    .filter((value): value is number => typeof value === "number" && Number.isFinite(value));

  return values.length ? values.reduce((sum, value) => sum + value, 0) : null;
}

export function resolveReasoningTokens(source: unknown): number | null {
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

export function buildCacheTelemetryDebugSnapshot(
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

export function logCacheTelemetryContractViolations(terminalEvent: string, payload: TurnStreamEvent) {
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

export function traceReasoningContent(trace?: TurnTraceRecord | null) {
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

export function traceErrorDetail(trace?: TurnTraceRecord | null) {
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

export function traceModelLabel(trace?: TurnTraceRecord | null) {
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

export function traceToolActivities(trace?: TurnTraceRecord | null) {
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
