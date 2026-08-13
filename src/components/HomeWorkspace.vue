<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, shallowReactive, watch } from "vue";
import type { ComponentPublicInstance } from "vue";
import { storeToRefs } from "pinia";
import { ArrowDown, RotateCcw } from "lucide-vue-next";
import type { ChatMessage, ConversationCheckpointEntry, HistoryNode, TraceTimelineEntry } from "@/types/runtime";
import { useRuntimeStore } from "@/stores/runtime";
import { extractErrorMessage } from "@/lib/error-utils";
import { useTimelineAutoScroll } from "@/lib/useTimelineAutoScroll";
import { useStreamingPresentationState } from "@/lib/useStreamingPresentationState";
import { isSimpleTextContent } from "@/lib/markdown";

import ScrollArea from "@/components/ui/ScrollArea.vue";
import WorkspaceTurnItem, {
  type AgentTurnEvent,
  type CheckpointRollbackAction,
  type MergedToolCall,
  type TurnBucket
} from "@/components/chat/WorkspaceTurnItem.vue";
import WorkspaceComposer from "@/components/chat/WorkspaceComposer.vue";
import AskPanel from "@/components/AskPanel.vue";

const SYNTHETIC_KEEP_NODE_PREFIX = "synthetic-keep-";

const runtimeStore = useRuntimeStore();
const runtimeStoreSessionList = storeToRefs(runtimeStore).sessionList;

const {
  conversationCheckpointEntries,
  draftMessage,
  historyNodes,
  isSubmitting,
  messages,
  sessionOperation,
  traceTimeline,
  turnTraceHistory
} = storeToRefs(runtimeStore);

function isTransientSessionOverview(session: {
  title?: string | null;
  summary: string;
  turnCount: number;
  updatedAtMs: number;
  lastReferencedFile?: string | null;
}) {
  return (
    session.title === "新对话" &&
    session.summary === "发送第一条消息后保存到历史" &&
    session.turnCount === 0 &&
    session.updatedAtMs === 0 &&
    session.lastReferencedFile == null
  );
}

const showReasoningContent = ref(false);
const workspaceContentColumnRef = ref<HTMLElement | null>(null);
const ROLLBACK_PROGRESS_MIN_VISIBLE_MS = 0;
const rollbackInFlight = ref<{ turnId: string; action: CheckpointRollbackAction } | null>(null);
const rollbackProgressStyle = ref<Record<string, string | undefined>>({});
const optimisticRollbackTurnId = ref<string | null>(null);
const rollbackExitingTurnIds = ref<Set<string>>(new Set());

const timelineScrollAreaRef = ref<{
  scrollToBottom: (behavior?: ScrollBehavior) => void;
  viewportEl: HTMLElement | null;
} | null>(null);
const scrollAnchorRef = ref<HTMLElement | null>(null);
const composerShellRef = ref<HTMLElement | null>(null);
const latestUserMessageRef = ref<HTMLElement | null>(null);
const latestAgentMessageRef = ref<HTMLElement | null>(null);
const stopRequested = ref(false);
const scrollToLatestHovered = ref(false);
let hoverDebounceTimer: ReturnType<typeof setTimeout> | null = null;
let streamingPresentationTimer: ReturnType<typeof setTimeout> | null = null;
const SHOW_REASONING_STORAGE_KEY = "pony-agent.ui.show-reasoning-content";
const STREAM_RENDER_DISABLE_STORAGE_KEY = "pony-agent.stream-render.disable-optimization";
const STREAM_RENDER_CONFIG_CHANGED_EVENT = "pony:stream-render-config-changed";
const STREAMING_MARKDOWN_FOLLOW_DISTANCE_PX = 96;
const STREAMING_PRESENTATION_TICK_MS = 60;
const COMPOSER_BUFFER_PX = 220;
const streamDebugState = shallowReactive<Record<string, unknown>>({});
const streamingRenderOptimizationDisabled = ref(false);
const terminalTracePhases = new Set(["completed", "failed", "cancelled"]);
const lastTrustedTraceTimelineByTurnId = new Map<string, TraceTimelineEntry[]>();

function resolveTimelineViewport(): HTMLElement | null {
  const viewport = timelineScrollAreaRef.value?.viewportEl ?? null;
  return viewport instanceof HTMLElement ? viewport : null;
}

function collectTimelineScrollMetrics() {
  return timelineAutoScroll.collectMetrics();
}

function setLatestTurnElementRef(
  targetRef: { value: HTMLElement | null },
  element: Element | ComponentPublicInstance | null,
  turnId: string
) {
  if (!isLastTurn.value(turnId)) {
    if (element == null) {
      targetRef.value = null;
    }
    return;
  }

  targetRef.value = element instanceof HTMLElement ? element : null;
}

function setLatestUserMessageRef(element: Element | ComponentPublicInstance | null, turnId: string) {
  setLatestTurnElementRef(latestUserMessageRef, element, turnId);
}

function getLatestUserMessageElement() {
  return latestUserMessageRef.value;
}

function setLatestAgentMessageRef(element: Element | ComponentPublicInstance | null, turnId: string) {
  setLatestTurnElementRef(latestAgentMessageRef, element, turnId);
}

function getLatestAgentMessageElement() {
  return latestAgentMessageRef.value;
}

function updateStreamDebugReveal(patch: Record<string, unknown>) {
  Object.assign(streamDebugState, patch);
}

function emitTimelineScrollDebug(event: string, patch: Record<string, unknown> = {}) {
  const payload = {
    event,
    at: Date.now(),
    ...collectTimelineScrollMetrics(),
    ...patch
  };
  updateStreamDebugReveal(payload);
  if (typeof window !== "undefined") {
    const debugWindow = window as typeof window & {
      __ponyScrollDebugBuffer?: Array<Record<string, unknown>>;
      __ponyScrollDebugLatest?: Record<string, unknown>;
    };
    const nextBuffer = [...(debugWindow.__ponyScrollDebugBuffer ?? []), payload].slice(-200);
    debugWindow.__ponyScrollDebugBuffer = nextBuffer;
    debugWindow.__ponyScrollDebugLatest = payload;
    window.dispatchEvent(new CustomEvent("pony:workspace-scroll-debug", { detail: payload }));
  }
  console.info("[workspace-scroll]", payload);
}

const turns = computed<TurnBucket[]>(() => {
  const buckets = new Map<string, TurnBucket>();

  for (const message of messages.value) {
    const bucket = buckets.get(message.turnId) ?? {
      turnId: message.turnId,
      user: null,
      assistant: null,
      tools: [],
      mergedTools: []
    };

    if (message.role === "user") {
      bucket.user = message;
    } else if (message.role === "assistant") {
      bucket.assistant = message;
    } else if (message.toolName || message.detail || message.content) {
      bucket.tools.push(message);
    }

    buckets.set(message.turnId, bucket);
  }

  for (const bucket of buckets.values()) {
    bucket.mergedTools = mergeToolCalls(bucket.tools);
  }

  if (buckets.size === 0 && hasVisibleHistorySession.value && latestFailedTrace.value) {
    const failedTrace = latestFailedTrace.value;
    const errorSummary = latestFailedTraceSummary.value;
    buckets.set(failedTrace.turnId, {
      turnId: failedTrace.turnId,
      user: null,
      assistant: {
        id: `failed-trace-${failedTrace.turnId}`,
        turnId: failedTrace.turnId,
        role: "assistant",
        content: errorSummary,
        status: "error",
        modelName: failedTrace.providerName && failedTrace.providerModel
          ? `${failedTrace.providerName}/${failedTrace.providerModel}`
          : failedTrace.providerName ?? failedTrace.providerModel ?? null,
        errorDetail: errorSummary
      },
      tools: [],
      mergedTools: []
    });
  }

  return Array.from(buckets.values());
});

const visibleTurns = computed<TurnBucket[]>(() => {
  const cutoffTurnId = optimisticRollbackTurnId.value;
  if (!cutoffTurnId) {
    return turns.value;
  }

  const cutoffIndex = turns.value.findIndex((turn) => turn.turnId === cutoffTurnId);
  if (cutoffIndex < 0) {
    return turns.value;
  }

  return turns.value.slice(0, cutoffIndex);
});

const latestVisibleTurnLayoutSignature = computed(() => {
  const latestTurn = visibleTurns.value[visibleTurns.value.length - 1] ?? null;
  if (!latestTurn) {
    return "";
  }
  return [
    latestTurn.turnId,
    latestTurn.user ? `user:${latestTurn.user.id}` : "user:-",
    latestTurn.assistant ? `assistant:${latestTurn.assistant.id}:${latestTurn.assistant.status ?? "done"}` : "assistant:-",
    `events:${agentTurnEvents(latestTurn).map((event) => {
      if (event.kind === "tools") {
        return `tools:${event.tools.map((tool) => `${tool.id}:${tool.description}`).join(",")}`;
      }
      return `${event.kind}:${event.key}`;
    }).join(",")}`
  ].join("|");
});

const hasVisibleHistorySession = computed(() =>
  runtimeStoreSessionList.value.some(
    (session) =>
      session.conversationId === runtimeStore.sessionId &&
      !isTransientSessionOverview(session)
  )
);
const isEmptyWorkspace = computed(() =>
  visibleTurns.value.length === 0 && rollbackExitingTurnIds.value.size === 0
);
const isLastTurn = computed(() => {
  const t = visibleTurns.value;
  if (t.length === 0) {
    return () => true;
  }

  const lastTurnId = t[t.length - 1]!.turnId;
  return (turnId: string) => turnId === lastTurnId;
});
const latestFailedTrace = computed(() => {
  const traces = [...turnTraceHistory.value].reverse();
  return traces.find((trace) => trace.phase === "failed" || Boolean(trace.error)) ?? null;
});
const latestFailedTraceSummary = computed(() => {
  const trace = latestFailedTrace.value;
  if (!trace) {
    return "";
  }

  return (
    trace.error?.trim() ||
    trace.sessionSummary?.trim() ||
    trace.traceTimeline?.find((entry) => entry.error?.trim())?.error?.trim() ||
    "运行失败"
  );
});

const checkpointEntries = computed(() => conversationCheckpointEntries.value ?? []);

const checkpointEntryByTurnId = computed(() => {
  const lookup = new Map<string, ConversationCheckpointEntry>();
  for (const entry of checkpointEntries.value) {
    if (!lookup.has(entry.turnId)) {
      lookup.set(entry.turnId, entry);
    }
  }
  return lookup;
});

const historyNodeById = computed(() => {
  const lookup = new Map<string, HistoryNode>();
  for (const node of historyNodes.value) {
    lookup.set(node.nodeId, node);
  }
  return lookup;
});

const canUndoLastTurn = computed(() => {
  const latestTurn = turns.value[turns.value.length - 1] ?? null;
  return (
    Boolean(latestTurn?.turnId && checkpointEntryForTurn(latestTurn.turnId)) &&
    !isSubmitting.value &&
    !sessionOperation.value &&
    !rollbackInFlight.value
  );
});

const undoShortcutLabel = computed(() => {
  if (typeof navigator !== "undefined" && navigator.platform.toLowerCase().includes("mac")) {
    return "Cmd+Z";
  }

  return "Ctrl+Z";
});

const latestMessageRole = computed<ChatMessage["role"] | null>(() => {
  const latestMessage = messages.value[messages.value.length - 1] ?? null;
  return latestMessage?.role ?? null;
});

const latestTurnSignature = computed(() => {
  const latestMessage = messages.value[messages.value.length - 1] ?? null;
  if (!latestMessage) return "";
  return `${latestMessage.id}:${latestMessage.content.length}:${latestMessage.reasoningContent?.length ?? ""}`;
});

const messageIdentitySignature = computed(() =>
  messages.value.map((message) => `${message.id}:${message.turnId}:${message.role}`).join("|")
);

function extractDescription(detail: string | null | undefined): string {
  return detail?.split("\n")[0]?.trim() ?? "";
}

function extractToolDescription(tool: Pick<ChatMessage, "detail" | "content" | "status">): string {
  if (tool.status === "error") {
    return extractErrorMessage(tool.content)
      || extractErrorMessage(tool.detail)
      || extractDescription(tool.detail)
      || extractDescription(tool.content);
  }

  return extractDescription(tool.detail);
}

function toolMergeKey(tool: Pick<ChatMessage, "canonicalToolName" | "toolName" | "displayNameZh">) {
  return tool.canonicalToolName?.trim() || tool.toolName?.trim() || tool.displayNameZh?.trim() || "";
}

function mergeToolCalls(tools: ChatMessage[]): MergedToolCall[] {
  const result: MergedToolCall[] = [];
  for (const tool of tools) {
    const last = result[result.length - 1];
    const mergeKey = toolMergeKey(tool);
    if (last && mergeKey && last.mergeKey === mergeKey && last.status !== "error") {
      last.description = extractToolDescription(tool);
      last.status = tool.status ?? "done";
      last.durationSeconds = tool.durationSeconds ?? null;
      last.count++;
      last.id = tool.id;
    } else {
      result.push({
        id: tool.id,
        toolName: tool.toolName ?? "",
        canonicalToolName: tool.canonicalToolName ?? null,
        displayNameZh: tool.displayNameZh ?? null,
        mergeKey,
        description: extractToolDescription(tool),
        status: tool.status ?? "done",
        durationSeconds: tool.durationSeconds ?? null,
        count: 1,
      });
    }
  }
  return result;
}

function messageIndexInTurn(message: ChatMessage | null) {
  if (!message) {
    return Number.POSITIVE_INFINITY;
  }

  const index = messages.value.findIndex((item) => item.id === message.id);
  return index >= 0 ? index : Number.POSITIVE_INFINITY;
}

function canonicalTraceKind(entry: TraceTimelineEntry) {
  if (entry.kind === "model") {
    return "call_model";
  }
  if (entry.kind === "tool") {
    return "call_tool";
  }
  return entry.kind;
}

function traceToolMatchKey(entry: TraceTimelineEntry) {
  return entry.toolActivities
    ?.flatMap((activity) => [activity.canonicalToolName?.trim(), activity.name?.trim()])
    .filter((value): value is string => Boolean(value)) ?? [entry.label?.trim()].filter((value): value is string => Boolean(value));
}

function traceSequencesForToolMessages(tools: ChatMessage[], traceTimeline: TraceTimelineEntry[]) {
  const toolEntries = traceTimeline
    .filter((entry) => canonicalTraceKind(entry) === "call_tool")
    .sort((left, right) => left.sequence - right.sequence);
  const claimedActivityKeys = new Set<string>();
  const claimedLabelEntryIndexes = new Set<number>();
  const sequenceByMessageId = new Map<string, number>();

  for (const tool of tools) {
    const toolIdSuffix = tool.id.startsWith(`tool-${tool.turnId}-`)
      ? tool.id.slice(`tool-${tool.turnId}-`.length)
      : tool.id;
    for (const entry of toolEntries) {
      const activity = entry.toolActivities?.find(
        (candidate) =>
          !claimedActivityKeys.has(`${entry.id}:${candidate.id}`) &&
          (candidate.id === toolIdSuffix || `tool-${tool.turnId}-${candidate.id}` === tool.id)
      );
      if (activity) {
        claimedActivityKeys.add(`${entry.id}:${activity.id}`);
        sequenceByMessageId.set(tool.id, entry.sequence);
        break;
      }
    }
  }

  for (const tool of tools) {
    if (sequenceByMessageId.has(tool.id)) {
      continue;
    }
    const toolKey = toolMergeKey(tool).toLocaleLowerCase();
    if (!toolKey) {
      continue;
    }
    for (let index = 0; index < toolEntries.length; index++) {
      const entry = toolEntries[index]!;
      const activity = entry.toolActivities?.find(
        (candidate) =>
          !claimedActivityKeys.has(`${entry.id}:${candidate.id}`) &&
          [candidate.canonicalToolName, candidate.name]
            .filter((value): value is string => Boolean(value?.trim()))
            .some((value) => value.toLocaleLowerCase() === toolKey)
      );
      if (activity) {
        claimedActivityKeys.add(`${entry.id}:${activity.id}`);
        sequenceByMessageId.set(tool.id, entry.sequence);
        break;
      }
      if (
        !entry.toolActivities?.length &&
        !claimedLabelEntryIndexes.has(index) &&
        traceToolMatchKey(entry).some((entryToolKey) => entryToolKey.toLocaleLowerCase() === toolKey)
      ) {
        claimedLabelEntryIndexes.add(index);
        sequenceByMessageId.set(tool.id, entry.sequence);
        break;
      }
    }
  }

  return sequenceByMessageId;
}

function latestModelTraceEntry(traceTimeline: TraceTimelineEntry[]) {
  for (let index = traceTimeline.length - 1; index >= 0; index--) {
    const entry = traceTimeline[index]!;
    if (canonicalTraceKind(entry) === "call_model") {
      return entry;
    }
  }
  return null;
}

function modelTraceEntries(traceTimeline: TraceTimelineEntry[]) {
  return traceTimeline
    .filter((entry) => canonicalTraceKind(entry) === "call_model")
    .sort((left, right) => left.sequence - right.sequence);
}

// 纯只读函数：渲染路径不写任何共享状态。
// lastTrustedTraceTimelineByTurnId 缓存的写入由 watch(traceTimeline) 负责，
// 避免渲染期间修改可变 Map 产生时序竞态。
function traceTimelineForTurn(turnId: string) {
  const activeTurnId = runtimeStore.activeTurnId?.trim() || null;
  if (activeTurnId === turnId && traceTimeline.value.length) {
    return traceTimeline.value;
  }

  const historicalTrace = latestTraceForTurn(turnId);
  const historicalTimeline = historicalTrace?.traceTimeline ?? [];
  if (historicalTimeline.length) {
    if (terminalTracePhases.has((historicalTrace?.phase ?? "").trim().toLowerCase())) {
      return historicalTimeline;
    }

    const cachedTimeline = lastTrustedTraceTimelineByTurnId.get(turnId);
    if (cachedTimeline?.length) {
      return cachedTimeline;
    }

    return historicalTimeline;
  }

  const cachedTimeline = lastTrustedTraceTimelineByTurnId.get(turnId);
  if (cachedTimeline?.length) {
    return cachedTimeline;
  }

  return [];
}

function isLatestModelEntry(entry: TraceTimelineEntry, modelEntries: TraceTimelineEntry[]) {
  return modelEntries[modelEntries.length - 1]?.id === entry.id;
}

function isStreamingModelEntry(turn: TurnBucket, entry: TraceTimelineEntry, modelEntries: TraceTimelineEntry[]) {
  return Boolean(
    turn.assistant &&
    isAssistantStreaming(turn.assistant) &&
    isLatestModelEntry(entry, modelEntries) &&
    runtimeStore.activeTurnId === turn.turnId
  );
}

function currentModelHopContent(
  cumulativeContent: string,
  entry: TraceTimelineEntry,
  modelEntries: TraceTimelineEntry[],
  selectContent: (modelEntry: TraceTimelineEntry) => string,
  completeCumulativeContent = cumulativeContent
) {
  const currentIndex = modelEntries.findIndex((modelEntry) => modelEntry.id === entry.id);
  if (currentIndex <= 0) {
    return cumulativeContent;
  }

  const completedContents = modelEntries.slice(0, currentIndex).map(selectContent);
  const completedStartIndex = completedPrefixIndex(completedContents, completeCumulativeContent);
  const completedPrefix =
    completedStartIndex >= 0 ? completedContents.slice(completedStartIndex).join("") : "";
  if (!completedPrefix) {
    return cumulativeContent;
  }
  if (cumulativeContent.startsWith(completedPrefix)) {
    return cumulativeContent.slice(completedPrefix.length);
  }
  if (
    completeCumulativeContent.startsWith(completedPrefix) &&
    completedPrefix.startsWith(cumulativeContent)
  ) {
    return "";
  }
  return cumulativeContent;
}

// 查找 content 以"从某 index 开始的连续拼接"为前缀的 index（与 findIndex + slice(index).join("")
// 语义一致，但避免每个候选都重新拼接字符串，消除 trace 推导链上的 O(n²) 分配）。
function completedPrefixIndex(completedContents: string[], content: string): number {
  if (!content || completedContents.length === 0) {
    return -1;
  }
  const offsets: number[] = [];
  let total = 0;
  for (const part of completedContents) {
    offsets.push(total);
    total += part.length;
  }
  const joined = completedContents.join("");
  for (let index = 0; index < completedContents.length; index++) {
    const candidate = joined.slice(offsets[index]);
    if (candidate.length > 0 && content.startsWith(candidate)) {
      return index;
    }
  }
  return -1;
}

function modelEntryReasoningContent(
  turn: TurnBucket,
  entry: TraceTimelineEntry,
  modelEntries: TraceTimelineEntry[]
) {
  if (isStreamingModelEntry(turn, entry, modelEntries)) {
    const reasoning = assistantDisplayedReasoning(turn.assistant);
    const currentReasoning = currentModelHopContent(
      reasoning,
      entry,
      modelEntries,
      (modelEntry) => modelEntry.reasoningContent ?? ""
    );
    return currentReasoning.trim() ? currentReasoning : "";
  }

  const traceReasoning = entry.reasoningContent ?? "";
  if (traceReasoning.trim()) {
    return traceReasoning;
  }

  if (turn.assistant && isLatestModelEntry(entry, modelEntries)) {
    const reasoning = assistantReasoning(turn.assistant);
    return reasoning.trim() ? reasoning : "";
  }

  return "";
}

function modelEntryContent(
  turn: TurnBucket,
  entry: TraceTimelineEntry,
  modelEntries: TraceTimelineEntry[]
) {
  if (isStreamingModelEntry(turn, entry, modelEntries)) {
    const completeContent = turn.assistant?.content ?? "";
    const content = shouldUseOptimizedAssistantStreaming(turn.assistant)
      ? assistantDisplayContent(turn.assistant)
      : completeContent;
    const currentContent = currentModelHopContent(
      content,
      entry,
      modelEntries,
      (modelEntry) => modelEntry.text ?? "",
      completeContent
    );
    return currentContent.trim() ? currentContent : "";
  }

  const traceText = entry.text ?? "";
  if (traceText.trim()) {
    return traceText;
  }

  if (turn.assistant && isLatestModelEntry(entry, modelEntries)) {
    return turn.assistant.content.trim() ? turn.assistant.content : "";
  }

  return "";
}

function modelEventKey(prefix: "reasoning" | "content", assistant: ChatMessage, _entry: TraceTimelineEntry) {
  return `${prefix}-${assistant.id}`;
}

function streamingReasoningFade(reasoningContent: string, assistant: ChatMessage) {
  const fade = assistantDisplayedReasoningFade(assistant);
  return fade && reasoningContent.endsWith(fade) ? fade : "";
}

function streamingReasoningStable(reasoningContent: string, assistant: ChatMessage) {
  const fade = streamingReasoningFade(reasoningContent, assistant);
  return fade ? reasoningContent.slice(0, -fade.length) : reasoningContent;
}

function agentTurnEvents(turn: TurnBucket): AgentTurnEvent[] {
  const turnTraceTimeline = traceTimelineForTurn(turn.turnId);
  const modelEntry = latestModelTraceEntry(turnTraceTimeline);
  const modelEntries = modelTraceEntries(turnTraceTimeline);
  const assistantOrder = messageIndexInTurn(turn.assistant);
  const modelOrder = modelEntry?.sequence ?? assistantOrder;
  const isActiveStreamingTurn = runtimeStore.activeTurnId === turn.turnId && turn.assistant?.status === "pending";
  const toolTraceSequenceByMessageId = traceSequencesForToolMessages(turn.tools, turnTraceTimeline);
  const events: AgentTurnEvent[] = [];

  if (turn.assistant && modelEntries.length > 0) {
    for (const entry of modelEntries) {
      const streaming = isStreamingModelEntry(turn, entry, modelEntries);
      const reasoningContent = showReasoningContent.value
        ? modelEntryReasoningContent(turn, entry, modelEntries)
        : "";
      if (reasoningContent) {
        events.push({
          kind: "reasoning",
          key: modelEventKey("reasoning", turn.assistant, entry),
          order: entry.sequence - 0.2,
          assistant: turn.assistant,
          reasoningContent,
          streaming
        });
      }

      const content = modelEntryContent(turn, entry, modelEntries);
      if (content && !shouldRenderAssistantAsError(turn)) {
        events.push({
          kind: "content",
          key: modelEventKey("content", turn.assistant, entry),
          order: entry.sequence,
          assistant: turn.assistant,
          content,
          streaming
        });
      }
    }
  } else if (turn.assistant && shouldShowReasoningBlock(turn.assistant)) {
    events.push({
      kind: "reasoning",
      key: `reasoning-${turn.assistant.id}`,
      order: Number.isFinite(modelOrder) ? modelOrder - 0.2 : assistantOrder - 0.2,
      assistant: turn.assistant,
      reasoningContent: assistantDisplayedReasoning(turn.assistant).trim(),
      streaming: isAssistantReasoningStreaming(turn.assistant)
    });
  }

  for (const tool of turn.tools) {
    const traceSequence = toolTraceSequenceByMessageId.get(tool.id) ?? null;
    const fallbackOrder = messageIndexInTurn(tool);
    const normalizedFallbackOrder =
      modelEntry && Number.isFinite(assistantOrder) && Number.isFinite(fallbackOrder)
        ? modelOrder + (fallbackOrder - assistantOrder)
        : fallbackOrder;
    const streamFallbackOrder =
      isActiveStreamingTurn && modelEntry && tool.status === "done"
        ? modelOrder - 0.1
        : normalizedFallbackOrder;
    events.push({
      kind: "tools",
      key: `tools-${tool.id}`,
      order: modelEntry ? traceSequence ?? streamFallbackOrder : fallbackOrder,
      tools: mergeToolCalls([tool])
    });
  }

  if (assistantAwaitingFirstSignal(turn)) {
    events.push({
      kind: "waiting",
      key: `waiting-${turn.turnId}`,
      order: Number.isFinite(assistantOrder) ? assistantOrder : Number.MAX_SAFE_INTEGER - 2
    });
  }

  if (
    modelEntries.length === 0 &&
    turn.assistant &&
    assistantHasVisibleContent(turn.assistant) &&
    !shouldRenderAssistantAsError(turn)
  ) {
    events.push({
      kind: "content",
      key: `content-${turn.assistant.id}`,
      order: modelOrder,
      assistant: turn.assistant,
      content: shouldUseOptimizedAssistantStreaming(turn.assistant)
        ? assistantDisplayContent(turn.assistant)
        : turn.assistant.content,
      streaming: isAssistantStreaming(turn.assistant)
    });
  }

  if (shouldRenderAssistantAsError(turn) && assistantErrorDetail(turn)) {
    events.push({
      kind: "error",
      key: `error-${turn.turnId}`,
      order: Number.isFinite(modelOrder) ? modelOrder + 0.1 : assistantOrder + 0.1
    });
  }

  const orderedEvents = events.sort((left, right) => {
    if (left.order !== right.order) {
      return left.order - right.order;
    }
    return left.key.localeCompare(right.key);
  });

  const mergedEvents: AgentTurnEvent[] = [];
  for (const event of orderedEvents) {
    const last = mergedEvents[mergedEvents.length - 1];
    if (event.kind === "tools" && last?.kind === "tools") {
      const tail = last.tools[last.tools.length - 1];
      const head = event.tools[0];
      if (tail && head && tail.mergeKey && tail.mergeKey === head.mergeKey && tail.status !== "error") {
        tail.id = head.id;
        tail.description = head.description;
        tail.status = head.status;
        tail.durationSeconds = head.durationSeconds;
        tail.count += head.count;
      } else {
        last.tools.push(...event.tools);
      }
      continue;
    }
    if (event.kind === "reasoning" && last?.kind === "reasoning") {
      last.reasoningContent += "\n\n" + event.reasoningContent;
      last.streaming = last.streaming || event.streaming;
      continue;
    }
    // 合并 content 事件：多 hop 内容累积到同一个 content 事件中，
    // 确保 key 始终为 content-${assistant.id}，避免 DOM 重建闪烁。
    if (event.kind === "content" && last?.kind === "content") {
      last.content += "\n\n" + event.content;
      last.streaming = last.streaming || event.streaming;
      continue;
    }
    mergedEvents.push(event);
  }

  return mergedEvents;
}

// 主对话渲染与 trace 渲染解耦：agentTurnEvents 的推导结果按 turn 缓存，
// 避免 traceTimeline/turnTraceHistory 每次变化都对所有 turn 全量重推导
// （traceTimelineForTurn + modelTraceEntries + 多 hop 内容拼接）。
// 只缓存非活跃 turn：流式中的 turn 内容持续变化且展示状态由 streaming
// presentation 驱动，缓存无收益，直接重算。
const turnEventCache = new Map<
  string,
  { timelineRef: TraceTimelineEntry[] | null; signature: string; events: AgentTurnEvent[] }
>();

function buildTurnEventSignature(
  turn: TurnBucket,
  showReasoning: boolean,
  activeTurnId: string | null,
  streamingRenderOptimized: boolean
) {
  const assistant = turn.assistant;
  const assistantPart = assistant
    ? `${assistant.id}:${assistant.status ?? ""}:${assistant.content.length}:${assistant.reasoningContent?.length ?? ""}:${assistant.modelName ?? ""}`
    : "-";
  const toolsPart = turn.tools
    .map((tool) =>
      `${tool.id}:${tool.status ?? ""}:${tool.durationSeconds ?? ""}:${tool.content?.length ?? 0}:${tool.detail?.length ?? 0}:${tool.toolName ?? ""}`
    )
    .join(",");
  return [
    turn.turnId,
    turn.user?.id ?? "-",
    assistantPart,
    toolsPart,
    `sr:${showReasoning}`,
    `at:${activeTurnId ?? ""}`,
    `last:${isLastTurn.value(turn.turnId)}`,
    `opt:${streamingRenderOptimized}`
  ].join("|");
}

const turnEventsByTurnId = computed(() => {
  const eventsByTurnId = new Map<string, AgentTurnEvent[]>();
  const activeTurnId = runtimeStore.activeTurnId?.trim() || null;
  const showReasoning = showReasoningContent.value;
  const streamingRenderOptimized = !streamingRenderOptimizationDisabled.value;

  for (const turn of turns.value) {
    const isActiveStreaming =
      activeTurnId === turn.turnId || turn.assistant?.status === "pending";
    if (!isActiveStreaming) {
      const signature = buildTurnEventSignature(turn, showReasoning, activeTurnId, streamingRenderOptimized);
      const timeline = traceTimelineForTurn(turn.turnId);
      const cached = turnEventCache.get(turn.turnId);
      if (cached && cached.signature === signature && cached.timelineRef === timeline) {
        eventsByTurnId.set(turn.turnId, cached.events);
        continue;
      }
      const events = agentTurnEvents(turn);
      turnEventCache.set(turn.turnId, { timelineRef: timeline, signature, events });
      eventsByTurnId.set(turn.turnId, events);
      continue;
    }
    eventsByTurnId.set(turn.turnId, agentTurnEvents(turn));
  }
  return eventsByTurnId;
});

function assistantReasoning(message: ChatMessage | null) {
  return message?.reasoningContent ?? "";
}

function isAssistantReasoningStreaming(message: ChatMessage | null) {
  return message?.status === "pending";
}

const streamingPresentation = useStreamingPresentationState(messages);
const {
  syncStreamingPresentationState,
  assistantDisplayContent,
  assistantDisplayedReasoning,
  assistantDisplayedReasoningFade,
  assistantDisplayedReasoningFadeStyle,
  assistantDisplayedReasoningFadeKey
} = streamingPresentation;

function shouldShowReasoningBlock(message: ChatMessage | null) {
  if (!message || !showReasoningContent.value) {
    return false;
  }

  return assistantDisplayedReasoning(message).trim().length > 0;
}

function assistantHasReasoningSignal(message: ChatMessage | null) {
  return assistantDisplayedReasoning(message).trim().length > 0;
}

function assistantHasVisibleContent(message: ChatMessage | null) {
  if (!message) {
    return false;
  }

  const visibleContent = shouldUseOptimizedAssistantStreaming(message)
    ? assistantDisplayContent(message)
    : message.content;
  return Boolean(visibleContent.trim());
}

function assistantAwaitingFirstSignal(turn: TurnBucket) {
  if (!isLastTurn.value(turn.turnId) || turn.tools.length > 0 || turn.mergedTools.length > 0) {
    return false;
  }

  if (turn.assistant) {
    return (
      turn.assistant.status === "pending" &&
      !assistantHasVisibleContent(turn.assistant) &&
      !assistantHasReasoningSignal(turn.assistant)
    );
  }

  const activeTurnId = runtimeStore.activeTurnId?.trim() || null;
  return Boolean(
    turn.user &&
    isSubmitting.value &&
    !sessionOperation.value &&
    (!activeTurnId || activeTurnId === turn.turnId)
  );
}

function shouldShowAgentArticle(turn: TurnBucket) {
  return Boolean(turn.assistant || turn.tools.length || assistantAwaitingFirstSignal(turn));
}

function isAssistantStreaming(message: ChatMessage | null) {
  return message?.status === "pending";
}

function hasPendingAssistantPresentationWork() {
  return messages.value.some((message) =>
    message.role === "assistant"
    && message.status === "pending"
    && assistantDisplayContent(message).length < message.content.length
  );
}

function stopStreamingPresentationTimer() {
  if (streamingPresentationTimer) {
    clearTimeout(streamingPresentationTimer);
    streamingPresentationTimer = null;
  }
}

function scheduleStreamingPresentationTimer() {
  // 已有定时器运行时不重复创建，避免快速 content 变动持续推迟 60ms tick
  if (streamingPresentationTimer) return;
  if (!hasPendingAssistantPresentationWork()) {
    return;
  }
  streamingPresentationTimer = setTimeout(() => {
    streamingPresentationTimer = null;
    syncStreamingPresentationState();
    scheduleStreamingPresentationTimer();
  }, STREAMING_PRESENTATION_TICK_MS);
}

function loadStreamingRenderConfig() {
  if (typeof window === "undefined") {
    streamingRenderOptimizationDisabled.value = false;
    return;
  }

  streamingRenderOptimizationDisabled.value =
    window.localStorage.getItem(STREAM_RENDER_DISABLE_STORAGE_KEY) === "true";
}

function handleStreamingRenderConfigChanged() {
  loadStreamingRenderConfig();
  scheduleStreamingPresentationTimer();
}

function shouldUseOptimizedAssistantStreaming(message: ChatMessage | null) {
  return Boolean(message && isAssistantStreaming(message) && !streamingRenderOptimizationDisabled.value);
}

const streamingRenderPathLock = new Map<string, boolean>();

function shouldUseMarkdownAssistantRendering(message: ChatMessage | null, _content: string) {
  if (!message) {
    return false;
  }

  if (!isAssistantStreaming(message)) {
    streamingRenderPathLock.delete(message.id);
    return true;
  }

  if (!shouldUseOptimizedAssistantStreaming(message)) {
    streamingRenderPathLock.delete(message.id);
    return false;
  }

  // 缓存锁定路径，防止内容渐进增长导致 isSimpleTextContent 翻转 → DOM 重建闪烁
  if (streamingRenderPathLock.has(message.id)) {
    return streamingRenderPathLock.get(message.id)!;
  }

  const useMarkdown = !isSimpleTextContent(message.content);
  streamingRenderPathLock.set(message.id, useMarkdown);
  return useMarkdown;
}

function latestTraceForTurn(turnId: string) {
  return [...turnTraceHistory.value].reverse().find((trace) => trace.turnId === turnId) ?? null;
}

function assistantErrorDetail(turn: TurnBucket): string {
  if (turn.assistant?.status !== "error") return "";

  const latestTrace = latestTraceForTurn(turn.turnId);
  const modelError = [...(latestTrace?.traceTimeline ?? [])]
    .reverse()
    .find((entry) => entry.kind === "call_model" && entry.error?.trim())
    ?.error?.trim();

  return modelError || latestTrace?.error?.trim() || turn.assistant.errorDetail?.trim() || "";
}

function shouldRenderAssistantAsError(turn: TurnBucket): boolean {
  if (turn.assistant?.status !== "error") {
    return false;
  }

  const latestTrace = latestTraceForTurn(turn.turnId);
  if (latestTrace?.phase === "completed") {
    return false;
  }

  return true;
}

function checkpointEntryForTurn(turnId: string) {
  return checkpointEntryByTurnId.value.get(turnId) ?? null;
}

function previousTurnIdForRollback(turnId: string) {
  const turnIndex = turns.value.findIndex((turn) => turn.turnId === turnId);
  if (turnIndex <= 0) {
    return null;
  }

  return turns.value[turnIndex - 1]?.turnId ?? null;
}

function resolveRollbackCheckoutNodeId(turnId: string, fallbackNodeId: string | null) {
  const latestTurn = turns.value[turns.value.length - 1] ?? null;
  const latestHeadNodeId = runtimeStore.branchHeadNodeId?.trim() || null;
  const latestHeadNode = latestHeadNodeId ? (historyNodeById.value.get(latestHeadNodeId) ?? null) : null;
  if (
    latestTurn?.turnId === turnId &&
    latestHeadNode &&
    latestHeadNode.turnId?.trim() === turnId
  ) {
    const latestParentNodeId = latestHeadNode.parentNodeId?.trim() || null;
    if (latestParentNodeId) {
      const latestParentNode = historyNodeById.value.get(latestParentNodeId) ?? null;
      // Only shortcut when the parent is a real, committed history node with a
      // concrete turnId.  If the parent is the empty root node (no turnId),
      // fall through to the original checkpoint / synthetic logic so the caller
      // still receives a synthetic-initial-like node when there is no meaningful
      // parent to roll back to.
      if (latestParentNode && latestParentNode.turnId?.trim()) {
        return latestParentNodeId;
      }
    }
  }

  const entryNodeId = fallbackNodeId ?? checkpointEntryForTurn(turnId)?.nodeId ?? null;
  if (!entryNodeId) {
    return null;
  }

  if (entryNodeId.startsWith("synthetic-")) {
    const previousTurnId = previousTurnIdForRollback(turnId);
    const previousEntryNodeId = previousTurnId ? checkpointEntryForTurn(previousTurnId)?.nodeId?.trim() : null;
    if (previousEntryNodeId && !previousEntryNodeId.startsWith("synthetic-")) {
      return previousEntryNodeId;
    }
    if (previousTurnId && previousEntryNodeId) {
      return `${SYNTHETIC_KEEP_NODE_PREFIX}${encodeURIComponent(previousTurnId)}`;
    }

    return `synthetic-initial-${turnId}`;
  }

  const entryNode = historyNodeById.value.get(entryNodeId) ?? null;
  const parentNodeId = entryNode?.parentNodeId?.trim() || null;
  if (parentNodeId) {
    return parentNodeId;
  }

  // No parent node means this is the first history node (first turn).
  // The backend always seeds a legacy root node as parent, so this path
  // should only be reached for sessions created before that migration.
  // Fall back to a synthetic initial-state node — the caller will handle
  // it by calling loadSessionState to reload the backend-truncated
  // (empty) snapshot.
  return `synthetic-initial-${turnId}`;
}

function checkoutNodeResetsToInitialState(nodeId: string) {
  if (nodeId.startsWith(SYNTHETIC_KEEP_NODE_PREFIX)) {
    return false;
  }
  if (nodeId.startsWith("synthetic-initial-")) {
    return true;
  }
  if (nodeId.startsWith("synthetic-")) {
    return true;
  }

  const targetNode = historyNodeById.value.get(nodeId) ?? null;
  return targetNode ? !targetNode.turnId?.trim() : false;
}

function markRollbackExitingTurns(turnId: string) {
  const cutoffIndex = turns.value.findIndex((turn) => turn.turnId === turnId);
  if (cutoffIndex < 0) {
    rollbackExitingTurnIds.value = new Set();
    return;
  }

  rollbackExitingTurnIds.value = new Set(
    turns.value.slice(cutoffIndex).map((turn) => turn.turnId)
  );
}

function clearRollbackExitingTurn(turnId: string | null) {
  if (!turnId || !rollbackExitingTurnIds.value.has(turnId)) {
    return;
  }

  const next = new Set(rollbackExitingTurnIds.value);
  next.delete(turnId);
  rollbackExitingTurnIds.value = next;
}

function handleTurnFlowAfterLeave(element: Element) {
  const turnId = element instanceof HTMLElement ? element.dataset.turnId ?? null : null;
  clearRollbackExitingTurn(turnId);
}

async function scrollToRemainingConversationTail() {
  await nextTick();
  timelineScrollAreaRef.value?.scrollToBottom("smooth");
}

function updateRollbackProgressPosition() {
  if (typeof window === "undefined" || !rollbackInFlight.value) {
    rollbackProgressStyle.value = {};
    return;
  }

  const contentColumn = workspaceContentColumnRef.value;
  if (!contentColumn || typeof contentColumn.getBoundingClientRect !== "function") {
    rollbackProgressStyle.value = {};
    return;
  }

  const rect = contentColumn.getBoundingClientRect();
  rollbackProgressStyle.value = {
    left: `${rect.left + rect.width / 2}px`,
    top: `${rect.top + rect.height / 2}px`
  };
}

function updateFloatingUiPositions() {
  updateRollbackProgressPosition();
}

async function executeRollback(turnId: string, action: CheckpointRollbackAction) {
  if (rollbackInFlight.value) {
    console.warn("[rollback] already in flight, skipping");
    return;
  }

  const entry = checkpointEntryForTurn(turnId);
  const nodeId = resolveRollbackCheckoutNodeId(turnId, entry?.nodeId ?? null);

  if (!nodeId) {
    console.warn("[rollback] Cannot resolve parent nodeId for turn", turnId, {
      entry: entry ? { nodeId: entry.nodeId, turnId: entry.turnId, isLatest: entry.isLatest } : null,
      checkpointEntryByTurnIdKeys: [...checkpointEntryByTurnId.value.keys()]
    });
    return;
  }

  // Capture the user's message content before truncation. When rolling
  // back to a previous turn, restore that content into the draft so it
  // can be edited and resent; initial-state rollback keeps the draft empty.
  const sourceTurn = turns.value.find(t => t.turnId === turnId);
  const nextDraft = checkoutNodeResetsToInitialState(nodeId) ? "" : sourceTurn?.user?.content ?? "";

  // Fill draft immediately so the user sees their message restored.
  draftMessage.value = nextDraft;
  runtimeStore.setDraftMessage(nextDraft);
  const rollbackStartedAt = Date.now();

  try {
    // Optimistically hide the rolled-back tail immediately while the backend
    // checkout and snapshot correction finish in the background.
    markRollbackExitingTurns(turnId);
    optimisticRollbackTurnId.value = turnId;
    rollbackInFlight.value = { turnId, action };
    void scrollToRemainingConversationTail();
    updateRollbackProgressPosition();
    void nextTick().then(() => updateRollbackProgressPosition());

    // With the backend always seeding a legacy root node, the synthetic
    // path should be unreachable in normal operation.  If reached (stale
    // data / legacy session), try to find the root node and checkout
    // through the backend; fall back to a local reset as a last resort.
    if (nodeId.startsWith(SYNTHETIC_KEEP_NODE_PREFIX)) {
      const keepTurnId = decodeURIComponent(nodeId.slice(SYNTHETIC_KEEP_NODE_PREFIX.length));
      if (!runtimeStore.rollbackToTurnBoundary(keepTurnId)) {
        runtimeStore.rollbackToInitialState();
      }
    } else if (!nodeId.startsWith("synthetic-")) {
      await runtimeStore.checkoutHistoryNode(nodeId, action, turnId);
    } else {
      const rootNode = runtimeStore.historyNodes.find((n) => !n.parentNodeId?.trim() && !n.turnId?.trim());
      if (rootNode) {
        await runtimeStore.checkoutHistoryNode(rootNode.nodeId, action, turnId);
      } else {
        runtimeStore.rollbackToInitialState();
      }
    }
  } catch (err) {
    console.error("[rollback] checkoutHistoryNode failed:", err);
  } finally {
    const remainingProgressMs = ROLLBACK_PROGRESS_MIN_VISIBLE_MS - (Date.now() - rollbackStartedAt);
    if (remainingProgressMs > 0) {
      await new Promise<void>((resolve) => window.setTimeout(resolve, remainingProgressMs));
    }

    optimisticRollbackTurnId.value = null;
    rollbackInFlight.value = null;
    rollbackExitingTurnIds.value = new Set();

    // Ensure the draft survives regardless of success/failure path
    draftMessage.value = nextDraft;
    runtimeStore.setDraftMessage(nextDraft);
  }
}

async function confirmRollback(turnId: string, action: CheckpointRollbackAction) {
  await executeRollback(turnId, action);
}

function rollbackProgressLabel() {
  if (!rollbackInFlight.value) {
    return "";
  }

  return "撤回";
}

async function handleUndoLastTurn() {
  if (!canUndoLastTurn.value || rollbackInFlight.value) {
    console.warn("[undo] canUndoLastTurn false or rollbackInFlight in progress", {
      canUndo: canUndoLastTurn.value,
      rollbackInFlight: !!rollbackInFlight.value
    });
    return;
  }

  // The latest turn is the one being rolled back.
  // executeRollback will resolve the checkpoint to roll back TO via the turn's entry.
  const latestTurn = turns.value[turns.value.length - 1];
  if (!latestTurn?.turnId) {
    console.warn("[undo] no latest turn found, turns count:", turns.value.length);
    return;
  }

  console.warn("[undo] rolling back turn:", latestTurn.turnId, "available checkpoints:", checkpointEntries.value.length);
  await executeRollback(latestTurn.turnId, "transcript_only");
}

function handleComposerKeydown(event: KeyboardEvent) {
  const isUndoShortcut =
    event.key.toLowerCase() === "z" &&
    (event.ctrlKey || event.metaKey) &&
    !event.shiftKey &&
    !event.altKey;
  if (isUndoShortcut) {
    event.preventDefault();
    handleUndoLastTurn();
    return;
  }

  if (event.key === "Enter" && !event.shiftKey) {
    event.preventDefault();
    if (!isSubmitting.value) {
      timelineAutoScroll.setAutoFollowEnabled();
      runtimeStore.submitTurn();
    }
  }
}

function handleWindowKeydown(event: KeyboardEvent) {
  const target = event.target as HTMLElement | null;
  const isEditableTarget =
    target instanceof HTMLTextAreaElement ||
    target instanceof HTMLInputElement ||
    target?.isContentEditable === true;

  if (isEditableTarget && target !== document.activeElement) {
    return;
  }

  const isUndoShortcut =
    event.key.toLowerCase() === "z" &&
    (event.ctrlKey || event.metaKey) &&
    !event.shiftKey &&
    !event.altKey;
  if (isUndoShortcut && !isEditableTarget) {
    event.preventDefault();
    handleUndoLastTurn();
    return;
  }
}

async function handlePrimaryAction() {
  if (isSubmitting.value) {
    const stopped = await runtimeStore.stopTurn();
    if (stopped) {
      stopRequested.value = true;
    }
    return;
  }

  stopRequested.value = false;
  timelineAutoScroll.setAutoFollowEnabled();
  await runtimeStore.submitTurn();
}

function setComposerShellRef(element: unknown) {
  composerShellRef.value = element instanceof HTMLElement ? element : null;
}

function onShowReasoningContentChange(value: boolean) {
  showReasoningContent.value = value;
}

const timelineAutoScroll = useTimelineAutoScroll({
  timelineScrollAreaRef,
  scrollAnchorRef,
  composerShellRef,
  workspaceContentColumnRef,
  isSubmitting,
  latestMessageRole,
  latestTurnSignature,
  latestVisibleTurnLayoutSignature,
  getLatestUserMessageElement,
  getLatestAgentMessageElement,
  showDebug: emitTimelineScrollDebug,
  updateFloatingUiPositions
});

const {
  showScrollToBottom,
  unreadCount,
  streamAutoFollowEnabled,
  handleScrollToBottom,
  handleMarkdownRenderComplete: handleTimelineMarkdownRenderComplete,
  handleLatestTurnSignatureChange,
  handleSubmittingChange,
  handleMessageCountChange
} = timelineAutoScroll;

function handleMarkdownRenderComplete(payload: { contentLength: number; streaming: boolean }) {
  const scrollMetrics = payload.streaming ? collectTimelineScrollMetrics() : null;
  updateStreamDebugReveal({
    markdownRenderCompletedAt: Date.now(),
    markdownRenderContentLength: payload.contentLength,
    markdownRenderStreaming: payload.streaming,
    markdownRenderDistanceToBottom: scrollMetrics?.distanceToBottom ?? null,
    streamingRenderOptimized: !streamingRenderOptimizationDisabled.value
  });

  if (payload.streaming && !streamAutoFollowEnabled.value) {
    return;
  }

  if (
    payload.streaming
    && typeof scrollMetrics?.distanceToBottom === "number"
    && scrollMetrics.distanceToBottom > STREAMING_MARKDOWN_FOLLOW_DISTANCE_PX
  ) {
    return;
  }

  handleTimelineMarkdownRenderComplete(payload);
}

const scrollToLatestHoverClass = computed(() => {
  const w = unreadCount.value > 0 ? 'w-[145px]' : 'w-[110px]';
  return `${w} h-[34px] px-3 py-2 justify-start gap-1.5 bg-white text-stone-900 shadow-[0_4px_12px_rgba(0,0,0,0.12)]`;
});

function handleScrollToLatestMouseEnter() {
  if (hoverDebounceTimer) {
    clearTimeout(hoverDebounceTimer);
    hoverDebounceTimer = null;
  }
  scrollToLatestHovered.value = true;
}

function handleScrollToLatestMouseLeave() {
  hoverDebounceTimer = setTimeout(() => {
    scrollToLatestHovered.value = false;
    hoverDebounceTimer = null;
  }, 400);
}

onMounted(() => {
  if (typeof window !== "undefined") {
    showReasoningContent.value = window.localStorage.getItem(SHOW_REASONING_STORAGE_KEY) === "true";
    loadStreamingRenderConfig();
  }
  syncStreamingPresentationState();
  scheduleStreamingPresentationTimer();
  window.addEventListener("keydown", handleWindowKeydown);
  window.addEventListener("resize", updateFloatingUiPositions);
  window.addEventListener("storage", handleStreamingRenderConfigChanged);
  window.addEventListener(STREAM_RENDER_CONFIG_CHANGED_EVENT, handleStreamingRenderConfigChanged);
});

onBeforeUnmount(() => {
  if (hoverDebounceTimer) clearTimeout(hoverDebounceTimer);
  stopStreamingPresentationTimer();
  window.removeEventListener("keydown", handleWindowKeydown);
  window.removeEventListener("resize", updateFloatingUiPositions);
  window.removeEventListener("storage", handleStreamingRenderConfigChanged);
  window.removeEventListener(STREAM_RENDER_CONFIG_CHANGED_EVENT, handleStreamingRenderConfigChanged);
});

watch(rollbackInFlight, () => {
  void nextTick().then(() => updateRollbackProgressPosition());
}, { flush: "post" });

watch(latestTurnSignature, (signature, previousSignature) => {
  if (signature === previousSignature) {
    return;
  }
  syncStreamingPresentationState();
  scheduleStreamingPresentationTimer();
  handleLatestTurnSignatureChange(signature, previousSignature);
}, { flush: "pre" });

watch(
  messageIdentitySignature,
  (signature, previousSignature) => {
    if (signature === previousSignature) {
      return;
    }
    const visibleTurnIds = new Set(messages.value.map((message) => message.turnId));
    for (const turnId of lastTrustedTraceTimelineByTurnId.keys()) {
      if (!visibleTurnIds.has(turnId)) {
        lastTrustedTraceTimelineByTurnId.delete(turnId);
      }
    }
    for (const turnId of turnEventCache.keys()) {
      if (!visibleTurnIds.has(turnId)) {
        turnEventCache.delete(turnId);
      }
    }
    scheduleStreamingPresentationTimer();
  },
  { flush: "pre" }
);

// 渲染路径外维护"最后信任的 timeline"缓存（traceTimelineForTurn 为纯只读函数）。
// immediate：组件挂载时 store 的 traceTimeline 可能已有值，需要立即建立缓存。
watch(
  traceTimeline,
  (timeline) => {
    const activeTurnId = runtimeStore.activeTurnId?.trim() || null;
    if (activeTurnId && timeline.length) {
      lastTrustedTraceTimelineByTurnId.set(activeTurnId, timeline);
    }
  },
  { flush: "pre", immediate: true }
);

watch(showReasoningContent, (value) => {
  if (typeof window !== "undefined") {
    window.localStorage.setItem(SHOW_REASONING_STORAGE_KEY, value ? "true" : "false");
  }
});

const STREAM_METRICS_STORAGE_KEY = "pony-agent.metrics.stream-sessions";
const MAX_STORED_SESSIONS = 50;

function collectStreamMetrics() {
  const viewport = resolveTimelineViewport();
  return {
    flushedAt: Date.now(),
    streamAutoFollowEnabled: streamAutoFollowEnabled.value,
    viewportScrollTop: viewport?.scrollTop ?? null,
    viewportScrollHeight: viewport?.scrollHeight ?? null,
    viewportClientHeight: viewport?.clientHeight ?? null,
    streamDebugState: { ...streamDebugState },
    streamingRenderOptimized: !streamingRenderOptimizationDisabled.value
  };
}

function flushStreamMetricsToStorage() {
  if (typeof window === "undefined") return;
  try {
    const raw = window.localStorage.getItem(STREAM_METRICS_STORAGE_KEY);
    const sessions: Array<Record<string, unknown>> = raw ? JSON.parse(raw) : [];
    sessions.push(collectStreamMetrics());
    window.localStorage.setItem(
      STREAM_METRICS_STORAGE_KEY,
      JSON.stringify(sessions.slice(-MAX_STORED_SESSIONS))
    );
  } catch {
    // Silently ignore storage errors
  }
}

watch(isSubmitting, (submitting, wasSubmitting) => {
  if (wasSubmitting && !submitting) {
    // 统计落盘是纯可观测性开销，延迟到下一事件循环执行，避免阻塞主对话完成帧
    window.setTimeout(() => flushStreamMetricsToStorage(), 0);
  }
  syncStreamingPresentationState();
  scheduleStreamingPresentationTimer();
  if (!submitting) {
    stopRequested.value = false;
  }
  handleSubmittingChange(submitting, wasSubmitting);
});

watch(
  () => messages.value.length,
  handleMessageCountChange
);
</script>

<template>
  <section class="relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden rounded-t-[0.6rem]">
    <Teleport to="body">
      <div
        v-if="rollbackInFlight"
        class="rollback-progress-overlay"
        :style="rollbackProgressStyle"
        data-testid="workspace-rollback-progress"
      >
        <div class="rollback-progress-card">
          <RotateCcw class="rollback-progress-icon h-4 w-4" />
          <span>{{ rollbackProgressLabel() }}</span>
        </div>
      </div>
    </Teleport>
    <ScrollArea
      ref="timelineScrollAreaRef"
      class="min-h-0 flex-1 rounded-t-[0.6rem]"
      viewport-class="workspace-timeline-viewport px-4 sm:px-5"
    >
      <div ref="workspaceContentColumnRef" class="mx-auto w-full max-w-[46.4rem] pt-4 sm:pt-5" data-testid="workspace-content-column">
        <TransitionGroup name="turn-flow" tag="div" class="relative space-y-5" @after-leave="handleTurnFlowAfterLeave">
          <section
            v-if="isEmptyWorkspace"
            key="workspace-empty-state"
            class="flex min-h-[46vh] items-center justify-center px-5 py-10 text-center"
            data-testid="workspace-empty-state"
          >
            <h2 class="text-[28px] font-medium tracking-[-0.04em] text-stone-500">
              我能帮你做些什么？
            </h2>
          </section>
          <WorkspaceTurnItem
            v-for="turn in visibleTurns"
            :key="turn.turnId"
            :turn="turn"
            :events="turnEventsByTurnId.get(turn.turnId) ?? []"
            :should-show-agent-article="shouldShowAgentArticle(turn)"
            :rollback-in-flight="!!rollbackInFlight"
            :confirm-rollback="confirmRollback"
            :set-user-message-ref="(element) => setLatestUserMessageRef(element, turn.turnId)"
            :set-agent-message-ref="(element) => setLatestAgentMessageRef(element, turn.turnId)"
            :handle-markdown-render-complete="handleMarkdownRenderComplete"
            :should-use-markdown-assistant-rendering="shouldUseMarkdownAssistantRendering"
            :should-use-optimized-assistant-streaming="shouldUseOptimizedAssistantStreaming"
            :streaming-reasoning-stable="streamingReasoningStable"
            :streaming-reasoning-fade="streamingReasoningFade"
            :assistant-displayed-reasoning-fade-style="assistantDisplayedReasoningFadeStyle"
            :assistant-displayed-reasoning-fade-key="assistantDisplayedReasoningFadeKey"
          />
        </TransitionGroup>
        <AskPanel />
        <div :style="{ height: COMPOSER_BUFFER_PX + 'px' }" aria-hidden="true"></div>
        <div ref="scrollAnchorRef" aria-hidden="true" class="pointer-events-none" style="height:0;width:0"></div>
      </div>
    </ScrollArea>

    <button
      v-show="showScrollToBottom"
      :class="[
        'absolute right-6 top-1/2 z-20 -translate-y-1/2 flex items-center rounded-full border border-stone-200 bg-white/90 text-[12px] font-medium text-stone-600 backdrop-blur-sm transition-all duration-500 ease-out cursor-pointer',
        scrollToLatestHovered
          ? scrollToLatestHoverClass
          : 'w-[34px] h-[34px] p-0 justify-center gap-0 shadow-[0_2px_8px_rgba(0,0,0,0.08)]'
      ]"
      data-testid="workspace-scroll-to-bottom"
      @click="handleScrollToBottom"
      @mouseenter="handleScrollToLatestMouseEnter"
      @mouseleave="handleScrollToLatestMouseLeave"
    >
      <ArrowDown class="h-3.5 w-3.5 shrink-0" />
      <span
        :class="[
          'overflow-hidden whitespace-nowrap transition-all duration-500 ease-out',
          scrollToLatestHovered ? 'max-w-[5em] opacity-100' : 'max-w-0 opacity-0'
        ]"
      >滚动到最新</span>
      <span
        v-if="unreadCount > 0"
        :class="[
          'overflow-hidden whitespace-nowrap transition-all duration-500 ease-out',
          scrollToLatestHovered ? 'max-w-[3.5em] opacity-100' : 'max-w-0 opacity-0'
        ]"
      ><span class="flex h-5 min-w-5 items-center justify-center rounded-full bg-stone-800 px-1.5 text-[10px] font-semibold text-white">{{ unreadCount > 99 ? '99+' : unreadCount }}</span></span>
    </button>

    <WorkspaceComposer
      :show-reasoning-content="showReasoningContent"
      :can-undo-last-turn="canUndoLastTurn"
      :undo-shortcut-label="undoShortcutLabel"
      :handle-composer-keydown="handleComposerKeydown"
      :handle-primary-action="handlePrimaryAction"
      :handle-undo-last-turn="handleUndoLastTurn"
      :set-composer-shell-ref="setComposerShellRef"
      @update:show-reasoning-content="onShowReasoningContentChange"
    />
  </section>
</template>

<style scoped>
/* 每个 turn 独立渲染隔离，防止工具状态变化/思考流式时整列重排 */
:deep([data-testid="workspace-content-column"] > div > section[data-turn-id]) {
  content-visibility: auto;
  contain-intrinsic-size: auto 3rem;
}

/* 内容列创建布局隔离边界，减少流式输出时的连锁重排 */
[data-testid="workspace-content-column"] {
  contain: layout style paint;
  overflow-anchor: none;
}

:deep(.workspace-timeline-viewport) {
  overflow-anchor: none;
}

:deep(.turn-flow-enter-active),
:deep(.turn-flow-leave-active) {
  transition:
    opacity 320ms cubic-bezier(0.25, 0.46, 0.45, 0.94),
    transform 320ms cubic-bezier(0.25, 0.46, 0.45, 0.94);
}

:deep(.turn-flow-leave-active) {
  position: absolute;
  left: 0;
  right: 0;
}

:deep(.turn-flow-enter-from) {
  opacity: 0;
  transform: translateY(0.55rem);
}

:deep(.turn-flow-leave-to) {
  opacity: 0;
  transform: translateY(-0.35rem);
}

:deep(.turn-flow-move) {
  transition: transform 360ms cubic-bezier(0.25, 0.46, 0.45, 0.94);
}

@media (prefers-reduced-motion: reduce) {
  :deep(.turn-flow-enter-active),
  :deep(.turn-flow-leave-active),
  :deep(.turn-flow-move) {
    transition: none !important;
  }

  :deep(.turn-flow-enter-from),
  :deep(.turn-flow-leave-to) {
    opacity: 1;
    transform: none;
  }
}

.rollback-progress-overlay {
  position: fixed;
  z-index: 40;
  pointer-events: none;
  transform: translate(-50%, -50%);
}

.rollback-progress-card {
  display: inline-flex;
  align-items: center;
  gap: 0.6rem;
  border-radius: 12px;
  background: rgba(255, 255, 255, 0.94);
  color: rgba(87, 83, 78, 0.85);
  padding: 0.75rem 1.2rem;
  font-size: 14px;
  font-weight: 500;
  line-height: 1;
  backdrop-filter: blur(14px);
}

.rollback-progress-icon {
  width: 0.9rem;
  height: 0.9rem;
  color: rgba(87, 83, 78, 0.56);
  animation: rollback-progress-spin 0.9s linear infinite;
}

@keyframes rollback-progress-spin {
  from {
    transform: rotate(0deg);
  }

  to {
    transform: rotate(-360deg);
  }
}
</style>
