<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, shallowReactive, watch } from "vue";
import type { ComponentPublicInstance } from "vue";
import { storeToRefs } from "pinia";
import { ArrowDown, RotateCcw } from "lucide-vue-next";
import type { ChatMessage, ConversationCheckpointEntry, HistoryNode, TraceTimelineEntry } from "@/types/runtime";
import { useRuntimeStore } from "@/stores/runtime";
import { usePlanStore } from "@/stores/plan";
import { useGoalStore } from "@/stores/goal";
import { useTeamStore } from "@/stores/team";
import { useWorkspacePanelsStore } from "@/stores/workspace-panels";
import { safeInvoke } from "@/lib/tauri";
import {
  describeCompactionOutcome,
  describeGoalOutcome,
  goalCommandArgs,
  parseSlashCommand,
  type DesktopSlashCommand,
  type GoalCommandOutcome,
  type ManualCompactionOutcome
} from "@/lib/chat/slash-commands";
import { extractErrorMessage } from "@/lib/error-utils";
import { useTimelineAutoScroll } from "@/lib/useTimelineAutoScroll";
import { isWorkspaceScrollDebugEnabled } from "@/lib/runtime/utils";
import { StopSequenceState } from "@/lib/runtime/stop-sequence";
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
import QueuedMessagesBubble from "@/components/chat/QueuedMessagesBubble.vue";
import AgentRuntimeStatusBar from "@/components/chat/AgentRuntimeStatusBar.vue";
import TurnDial from "@/components/chat/TurnDial.vue";

const props = withDefaults(
  defineProps<{
    rightSidebarOpen?: boolean;
  }>(),
  {
    rightSidebarOpen: false
  }
);

const SYNTHETIC_KEEP_NODE_PREFIX = "synthetic-keep-";

const runtimeStore = useRuntimeStore();
const planStore = usePlanStore();
const goalStore = useGoalStore();
const teamStore = useTeamStore();
const workspacePanelsStore = useWorkspacePanelsStore();
const runtimeStoreSessionList = storeToRefs(runtimeStore).sessionList;

const {
  conversationCheckpointEntries,
  draftMessage,
  historyNodes,
  isSubmitting,
  messages,
  pendingQueuedMessages,
  activeTurnId,
  agentRuntimeStatus,
  phase,
  sessionId,
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
// wave-60 §C8：双击 Esc 的纯判定状态机（窗口 `stop_sequence_ms=500`，可注入语义与 Rust 侧一致）。
// 上膛/命中只在这里判定，停止态本身由 store 的 `phase="stopping"` 单一真源承载。
const stopSequenceState = new StopSequenceState();
const scrollToLatestHovered = ref(false);
let hoverDebounceTimer: ReturnType<typeof setTimeout> | null = null;
let streamingPresentationTimer: ReturnType<typeof setTimeout> | null = null;
const SHOW_REASONING_STORAGE_KEY = "pony-agent.ui.show-reasoning-content";
const STREAM_RENDER_DISABLE_STORAGE_KEY = "pony-agent.stream-render.disable-optimization";
const STREAM_RENDER_CONFIG_CHANGED_EVENT = "pony:stream-render-config-changed";
const STREAMING_MARKDOWN_FOLLOW_DISTANCE_PX = 96;
// wave-pds-3 §4：释放节拍 60ms → 33ms（≈30Hz）。平滑感的另一半由 MarkdownRenderer 的
// 尾巴遮罩浮现（§3.1）承担；`DEFAULT_REVEAL_BUDGET.charsPerTick` 保持 4 不动（契约 v3 裁定 ①）。
const STREAMING_PRESENTATION_TICK_MS = 33;
const COMPOSER_BUFFER_PX = 220;
const streamDebugState = shallowReactive<Record<string, unknown>>({});
const streamingRenderOptimizationDisabled = ref(false);
const terminalTracePhases = new Set(["completed", "failed", "cancelled"]);
const lastTrustedTraceTimelineByTurnId = new Map<string, TraceTimelineEntry[]>();

const activeTurnIdForDial = ref<string | null>(null);

/**
 * 收集时间线里已渲染的轮次节点（按 DOM 顺序 = 会话顺序）。
 * 优先从内容列取（真实视口与单测视口都能覆盖到），退化到滚动视口。
 */
function getTimelineTurnElements(): HTMLElement[] {
  const root = workspaceContentColumnRef.value ?? resolveTimelineViewport();
  if (!root) {
    return [];
  }
  return Array.from(root.querySelectorAll<HTMLElement>("[data-turn-id]"));
}

/** 把视口平滑滚动到指定轮次节点，并同步拨盘激活轮。 */
function scrollToTurnElement(targetTurnEl: HTMLElement) {
  const viewport = resolveTimelineViewport();
  if (!viewport) return;

  const turnId = targetTurnEl.getAttribute("data-turn-id");
  if (turnId) {
    activeTurnIdForDial.value = turnId;
  }
  const targetTop = targetTurnEl.offsetTop - 16;
  viewport.scrollTo({
    top: Math.max(0, targetTop),
    behavior: "smooth"
  });
}

function handleSelectDialTurn(turnId: string) {
  const targetTurnEl = getTimelineTurnElements().find(
    (el) => el.getAttribute("data-turn-id") === turnId
  );
  if (!targetTurnEl) return;
  scrollToTurnElement(targetTurnEl);
}

/** Alt+↑ / Alt+↓：在相邻轮次之间跳转（首尾夹取，不越界）。 */
function handleNavigateTurn(direction: -1 | 1) {
  const turnElements = getTimelineTurnElements();
  if (turnElements.length === 0) return;

  const currentId = activeTurnIdForDial.value;
  let currentIndex = currentId
    ? turnElements.findIndex((el) => el.getAttribute("data-turn-id") === currentId)
    : turnElements.length - 1;
  if (currentIndex < 0) {
    currentIndex = turnElements.length - 1;
  }

  const targetIndex = Math.min(
    turnElements.length - 1,
    Math.max(0, currentIndex + direction)
  );
  const targetTurnEl = turnElements[targetIndex];
  if (!targetTurnEl) return;

  scrollToTurnElement(targetTurnEl);
  emitTimelineScrollDebug("keyboard-nav:turn", {
    direction: direction < 0 ? "prev" : "next",
    fromTurnId: currentId ?? null,
    toTurnId: targetTurnEl.getAttribute("data-turn-id"),
    fromIndex: currentIndex,
    toIndex: targetIndex,
    turnCount: turnElements.length
  });
}

/** Ctrl/Cmd+T：平滑滚动到会话顶部（滚动事件会视为用户意图，暂停自动跟随）。 */
function handleTimelineScrollToTop() {
  const viewport = resolveTimelineViewport();
  if (!viewport) return;
  viewport.scrollTo({ top: 0, behavior: "smooth" });
  emitTimelineScrollDebug("keyboard-nav:scroll-to-top");
}

/**
 * Ctrl/Cmd+B：复用“滚动到最新”的既有收口（清未读 + 恢复自动跟随），
 * 不另起一套到底逻辑，保证与按钮行为单一真源。
 */
function handleTimelineScrollToBottom() {
  emitTimelineScrollDebug("keyboard-nav:scroll-to-bottom");
  handleScrollToBottom();
}

function updateActiveTurnFromViewport() {
  const viewport = resolveTimelineViewport();
  if (!viewport) return;

  const turnElements = viewport.querySelectorAll("[data-turn-id]");
  if (turnElements.length === 0) return;

  const viewportRect = viewport.getBoundingClientRect();
  const thresholdY = viewportRect.top + 80;

  let currentTurnId: string | null = null;
  for (let i = 0; i < turnElements.length; i++) {
    const el = turnElements[i] as HTMLElement;
    const rect = el.getBoundingClientRect();
    if (rect.top <= thresholdY && rect.bottom > viewportRect.top) {
      currentTurnId = el.getAttribute("data-turn-id");
    }
  }

  if (!currentTurnId && turnElements.length > 0) {
    const firstRect = (turnElements[0] as HTMLElement).getBoundingClientRect();
    if (firstRect.top > thresholdY) {
      currentTurnId = turnElements[0]!.getAttribute("data-turn-id");
    } else {
      currentTurnId = turnElements[turnElements.length - 1]!.getAttribute("data-turn-id");
    }
  }

  if (currentTurnId) {
    activeTurnIdForDial.value = currentTurnId;
  }
}

function resolveTimelineViewport(): HTMLElement | null {
  const viewport = timelineScrollAreaRef.value?.viewportEl ?? null;
  return typeof HTMLElement !== "undefined" && viewport instanceof HTMLElement ? viewport : null;
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

  // 守卫：WebView 拆除 / 非 DOM 环境（SSR、v8 插桩单测）下 `HTMLElement` 可能不存在，
  // 裸 `instanceof` 会抛 ReferenceError（未捕获错误），故先判类型存在再做 instanceof。
  targetRef.value =
    typeof HTMLElement !== "undefined" && element instanceof HTMLElement ? element : null;
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
  // PA-117：关闭态不得写入 streamDebugState（markdown render-complete 路径也经此函数）
  if (!isWorkspaceScrollDebugEnabled()) {
    return;
  }
  Object.assign(streamDebugState, patch);
}

function emitTimelineScrollDebug(event: string, patch: Record<string, unknown> = {}) {
  // PA-117：关闭态下 console / 诊断缓冲 / streamDebugState 该通道全部停用，
  // 且不得为构造 payload 触发布局读数（collectMetrics 会读 rect）。
  if (!isWorkspaceScrollDebugEnabled()) {
    return;
  }
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
  return tools.map((tool) => ({
    id: tool.id,
    toolName: tool.toolName ?? "",
    canonicalToolName: tool.canonicalToolName ?? null,
    displayNameZh: tool.displayNameZh ?? null,
    mergeKey: toolMergeKey(tool),
    description: extractToolDescription(tool),
    status: tool.status ?? "done",
    durationSeconds: tool.durationSeconds ?? null,
    count: 1,
    callId: tool.callId ?? null,
    argumentsText: tool.argumentsText ?? null
  }));
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
  if (cumulativeContent.length <= completedPrefix.length) {
    return "";
  }
  if (
    completeCumulativeContent.startsWith(completedPrefix) &&
    completedPrefix.startsWith(cumulativeContent)
  ) {
    return "";
  }
  return cumulativeContent.slice(completedPrefix.length);
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

function cleanHopText(text: string): string {
  if (!text) return "";
  return text.replace(/^[\r\n]+/, "").replace(/[\r\n]+$/, "");
}

/**
 * wave-pds-3 §1：**流式正文**的块边界语义依赖尾随空行。
 *
 * `splitMarkdownBlocks` 用「空行」判定「当前块已闭合、内容可移入 stableHtml」；而 `cleanHopText`
 * 会把尾随换行一并剥掉，于是渲染层永远看不到「块已闭合」，`[data-stream-tail]` 在块闭合帧
 * 无法归零（契约 §1：「存在未闭合尾巴文本时 =1；否则 =0」）。
 *
 * 故流式正文只剥**前导**换行（前导空行会让首块凭空多出间距），保留尾随换行 —— 对渲染结果
 * 无影响（`marked` 会忽略尾随空行），只为把「块是否闭合」这一位信息透到渲染层。
 */
function cleanStreamingHopText(text: string): string {
  if (!text) return "";
  return text.replace(/^[\r\n]+/, "");
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
    return cleanHopText(currentReasoning);
  }

  const traceReasoning = entry.reasoningContent ?? "";
  if (traceReasoning.trim()) {
    return cleanHopText(traceReasoning);
  }

  if (turn.assistant && isLatestModelEntry(entry, modelEntries)) {
    const reasoning = assistantReasoning(turn.assistant);
    return cleanHopText(reasoning);
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
    // wave-pds-3：流式帧保留尾随换行（块闭合位信息透给 MarkdownRenderer 的尾巴切分）。
    return cleanStreamingHopText(currentContent);
  }

  const traceText = entry.text ?? "";
  if (traceText.trim()) {
    return cleanHopText(traceText);
  }

  if (turn.assistant && isLatestModelEntry(entry, modelEntries)) {
    return cleanHopText(turn.assistant.content);
  }

  return "";
}

function modelEventKey(
  prefix: "reasoning" | "content",
  assistant: ChatMessage,
  entry: TraceTimelineEntry,
  index = 0
) {
  return index === 0
    ? `${prefix}-${assistant.id}`
    : `${prefix}-${assistant.id}-hop-${index}-${entry.id}`;
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
  const toolTraceSequenceByMessageId = traceSequencesForToolMessages(turn.tools, turnTraceTimeline);
  const events: AgentTurnEvent[] = [];

  if (turn.assistant && modelEntries.length > 0) {
    let entryIndex = 0;
    for (const entry of modelEntries) {
      const streaming = isStreamingModelEntry(turn, entry, modelEntries);
      const reasoningContent = showReasoningContent.value
        ? modelEntryReasoningContent(turn, entry, modelEntries)
        : "";
      if (reasoningContent.trim()) {
        events.push({
          kind: "reasoning",
          key: modelEventKey("reasoning", turn.assistant, entry, entryIndex),
          order: entry.sequence - 0.2,
          assistant: turn.assistant,
          reasoningContent,
          streaming
        });
      }

      const content = modelEntryContent(turn, entry, modelEntries);
      // wave-64 §2 F2：失败轮已产出的正文照常渲染（不再按错误态抑制正文）。
      if (content.trim()) {
        events.push({
          kind: "content",
          key: modelEventKey("content", turn.assistant, entry, entryIndex),
          order: entry.sequence,
          assistant: turn.assistant,
          content,
          streaming
        });
      }
      entryIndex++;
    }
  } else if (turn.assistant && shouldShowReasoningBlock(turn.assistant)) {
    const fallbackReasoning = cleanHopText(assistantDisplayedReasoning(turn.assistant));
    if (fallbackReasoning.trim()) {
      // 无 model trace 时展示权重以 assistant 数组下标为锚点：
      // 工具无条件锚定在 assistantOrder - 0.1（见下方 toolOrder 回退分支），
      // 故 reasoning 取 assistantOrder - 0.2，保证 Reasoning < Tools < 正文。
      events.push({
        kind: "reasoning",
        key: `reasoning-${turn.assistant.id}`,
        order: Number.isFinite(modelOrder) ? modelOrder - 0.2 : assistantOrder - 0.2,
        assistant: turn.assistant,
        reasoningContent: fallbackReasoning,
        streaming: isAssistantReasoningStreaming(turn.assistant)
      });
    }
  }

  for (let toolIdx = 0; toolIdx < turn.tools.length; toolIdx++) {
    const tool = turn.tools[toolIdx]!;
    const traceSequence = toolTraceSequenceByMessageId.get(tool.id) ?? null;
    let toolOrder: number;

    if (traceSequence != null) {
      // 有 call_tool trace 链接：保持多 hop 交错顺序（不变量：Reasoning < Tools < 正文）。
      toolOrder = traceSequence + toolIdx * 0.001;
    } else if (modelEntries.length > 0) {
      // 无 trace 链接但存在 model hop：锚定到最后一个 model 条目的正文之前，
      // 使工具不依赖数组下标即可稳定落在正文之上。
      const lastModelSequence = modelEntries[modelEntries.length - 1]!.sequence;
      toolOrder = lastModelSequence - 0.1 + toolIdx * 0.001;
    } else {
      // 无 trace 链接且无 model hop：无条件锚定到正文之前，
      // 规范化顺序 Reasoning < Tools < 正文 不依赖 reasoning 面板是否渲染。
      toolOrder = assistantOrder - 0.1 + toolIdx * 0.001;
    }

    events.push({
      kind: "tools",
      key: `tools-${tool.id}`,
      order: toolOrder,
      tools: mergeToolCalls([tool])
    });
  }

  // Wave-59 F1：`waiting` 事件已删除 —— 「思考中」的唯一指示由底部状态条承载
  // （workspace-runtime-status-slot），不再向事件流注入三点动画条目。

  if (
    modelEntries.length === 0 &&
    turn.assistant &&
    assistantHasVisibleContent(turn.assistant)
  ) {
    const rawContent = shouldUseOptimizedAssistantStreaming(turn.assistant)
      ? assistantDisplayContent(turn.assistant)
      : turn.assistant.content;
    const streamingContent = isAssistantStreaming(turn.assistant);
    // wave-pds-3：只有在**会走 markdown 渲染路径**时才保留尾随换行（块闭合位信息透给
    // MarkdownRenderer 的尾巴切分）。纯文本快路径（`whitespace-pre-wrap` 直接渲染）保持原样，
    // 否则尾随空行会在流式期渲染成底部空行、完成帧又被剥掉 ⇒ 收尾回撤。
    const keepsTrailingNewlines = streamingContent && !isSimpleTextContent(rawContent);
    const cleanedContent = keepsTrailingNewlines
      ? cleanStreamingHopText(rawContent)
      : cleanHopText(rawContent);
    if (cleanedContent.trim()) {
      events.push({
        kind: "content",
        key: `content-${turn.assistant.id}`,
        order: modelOrder,
        assistant: turn.assistant,
        content: cleanedContent,
        streaming: streamingContent
      });
    }
  }

  // wave-64 §2 F2：不再向事件流注入 `error` 条目 —— 消息区的「错误详情」折叠面板已移除，
  // 错误明细改由唯一的状态消息（`agentRuntimeStatus.mode === "error"`）与右侧 Trace 承载。

  const orderedEvents = events.sort((left, right) => {
    if (left.order !== right.order) {
      return left.order - right.order;
    }
    return left.key.localeCompare(right.key);
  });

  return orderedEvents;
}

// 主对话渲染与 trace 渲染解耦：agentTurnEvents 的推导结果按 turn 缓存，
// 避免 traceTimeline/turnTraceHistory 每次变化都对所有 turn 全量重推导
// （traceTimelineForTurn + modelTraceEntries + 多 hop 内容拼接）。
// 只缓存非活跃 turn：流式中的 turn 内容持续变化且多 hop content 结构复杂，
// 增量更新易破坏 hop 语义，直接全量重算。
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

function assistantHasVisibleContent(message: ChatMessage | null) {
  if (!message) {
    return false;
  }

  const visibleContent = shouldUseOptimizedAssistantStreaming(message)
    ? assistantDisplayContent(message)
    : message.content;
  return Boolean(visibleContent.trim());
}


function shouldShowAgentArticle(turn: TurnBucket) {
  // PA-118：agent 文章仅在“有可见内容 / 有可见推理 / 有工具”时渲染；
  // 取消回合（内容空、无推理、无工具）不渲染空 agent 壳/复制按钮行。
  // Wave-59 F1-3：等待首个信号（awaiting-first-signal）时同样返回 false ——
  // 该状态不渲染空 agent 壳（无 article、无 workspace-agent-actions），等待指示只在底部状态条。
  // wave-64 §2 F2：错误态不再是文章渲染条件（「错误详情」面板已移除，空正文不再渲染空壳）。
  if (turn.tools.length > 0) {
    return true;
  }
  if (!turn.assistant) {
    return false;
  }
  return (
    assistantHasVisibleContent(turn.assistant) ||
    shouldShowReasoningBlock(turn.assistant)
  );
}

function isAssistantStreaming(message: ChatMessage | null) {
  return message?.status === "pending";
}

const shouldShowAgentRuntimeStatus = computed(() => {
  return Boolean(agentRuntimeStatus.value);
});

/**
 * 需求3优化：agent消息stream时，底部的状态消息无需显示。
 * 当有活跃工具执行/等待审批/会话操作/错误态时，保持可见；
 * 当处于纯回复/思考流式输出正文时，平滑隐藏底部状态条，避免与正在流式的内容视觉竞争和抖动。
 */
const isRuntimeStatusVisible = computed(() => {
  if (!agentRuntimeStatus.value) {
    return false;
  }

  // wave-60 §C7 / A2：stopping 是**进行中**态，其唯一可见载体就是状态条
  // （终态改由 wave-64 §2 F1 的 stopped 模式承载）。故不得被下方「正文流式时隐藏状态条」规则吞掉。
  if (phase.value === "stopping") {
    return true;
  }

  // wave-64 §2 F1：终态状态行（stopped / error）必须**常显**，位置固定在消息区末尾；
  // 不得被下方「正文流式时隐藏状态条」规则误吞（终态期间 assistant.status 已非 pending，
  // 但仍在此处显式短路，避免未来流式判据变化造成静默回归）。
  if (
    agentRuntimeStatus.value.mode === "stopped" ||
    agentRuntimeStatus.value.mode === "error"
  ) {
    return true;
  }

  // 会话全局操作（如切换中、初始化中）或错误态，必须显示
  if (sessionOperation.value) {
    return true;
  }

  // 工具执行或等待授权等高优先级交互模式，必须显示
  if (
    agentRuntimeStatus.value.mode === "executing_tool" ||
    agentRuntimeStatus.value.mode === "tool_drafting" ||
    agentRuntimeStatus.value.mode === "waiting_approval" ||
    agentRuntimeStatus.value.mode === "retrying"
  ) {
    return true;
  }

  const hasActiveToolActivity = runtimeStore.toolActivities.some(
    (activity) => activity.status === "running" || activity.status === "planned"
  );
  if (hasActiveToolActivity) {
    return true;
  }

  // Wave-59 F4（D-59-3）：以 hop 级流式类型（Store.activeStreamKind）判定可见性。
  // - "content"   ：本 hop 正文正在流式且已可见 ⇒ 隐藏（F4-3，沿用优化3）
  // - "reasoning" ：本 hop 仅思考流式 ⇒ 状态条承载唯一「思考中」指示，保持可见
  // - null        ：本 hop 尚无任何输出（如工具后的新 hop）⇒ 必须重新可见（F4-2 / RC8）
  // - undefined   ：未接线（既有夹具）⇒ 回退下方旧判据，保证 wave-14 三例语义不变（F4-4）
  // 仅在运行期生效：mode="info" 等非运行态不得被上一轮残留的 "content" 误隐藏。
  if (isSubmitting.value) {
    const activeStreamKind = runtimeStore.activeStreamKind;
    if (activeStreamKind === "content") {
      return false;
    }
    if (activeStreamKind === "reasoning" || activeStreamKind === null) {
      return true;
    }
  }

  // 检查最后一个可见轮次的 assistant 消息：如果正在 stream 且已有内容展示，则隐藏状态条
  const latestTurn = visibleTurns.value[visibleTurns.value.length - 1] ?? null;
  const assistant = latestTurn?.assistant ?? null;
  if (assistant && assistant.status === "pending") {
    const hasStreamingVisibleContent =
      assistantHasVisibleContent(assistant) || Boolean(assistant.content?.trim());
    if (hasStreamingVisibleContent) {
      return false;
    }
  }

  return true;
});

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

function shouldUseMarkdownAssistantRendering(message: ChatMessage | null, content: string, streaming = false) {
  if (!message) {
    return false;
  }

  // Wave-59 F3-3 / RC5：非流式（完成态 / 非最新轮次的中间消息）时，若流式期间已锁定为「纯文本」，
  // **沿用同一条纯文本路径**（该路径自建 `.markdown-body` 承载节点），完成帧不替换承载节点。
  // 锁不删除：后续任意一次重推导都必须得到同一结论，否则会在 trace 更新时来回切换。
  // 注意：流式期间**不**短路，保留既有「检测到 markdown 语法即可平滑升级」的行为。
  //
  // Wave-59 A-59-2（审查裁决的**已批准例外**）：锁只在流式期有效。若 markdown 语法只在最后一个
  // delta 才出现（流式期始终被判为纯文本），完成态必须切回 markdown 渲染 —— 此时允许**一次**子树替换，
  // 因为「把 `**` / `|` 原样当纯文本显示」是正确性缺陷，优先级高于完成帧的节点身份（F3-3 在
  // markdown 路径上仍成立：该路径的 `.markdown-body` 全程由 MarkdownRenderer 常驻承载）。
  if (!streaming && !isAssistantStreaming(message)) {
    if (streamingRenderPathLock.get(message.id) === false && isSimpleTextContent(content)) {
      return false;
    }
    streamingRenderPathLock.delete(message.id);
    return true;
  }

  // 如果当前处于流式，但不是流式模式
  if (!shouldUseOptimizedAssistantStreaming(message)) {
    streamingRenderPathLock.delete(message.id);
    return false;
  }

  // 缓存锁定路径：如果已经升级为 Markdown 渲染，则锁定为 true 防止退回闪烁；
  // 但若此前为纯文本路径，当后续内容检测到 Markdown 语法时，允许平滑升级为 Markdown 渲染。
  if (streamingRenderPathLock.get(message.id) === true) {
    return true;
  }

  const useMarkdown = !isSimpleTextContent(message.content);
  streamingRenderPathLock.set(message.id, useMarkdown);
  return useMarkdown;
}

function latestTraceForTurn(turnId: string) {
  return [...turnTraceHistory.value].reverse().find((trace) => trace.turnId === turnId) ?? null;
}

// wave-64 §2 F2：`assistantErrorDetail` 与 `shouldRenderAssistantAsError` 已随消息区
// 「错误详情」折叠面板一并移除 —— 错误由唯一的状态消息与右侧 Trace 承载（本文件不再消费）。

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
  const turnId =
    typeof HTMLElement !== "undefined" && element instanceof HTMLElement
      ? element.dataset.turnId ?? null
      : null;
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

/** 斜杠命令在运行中被拒绝时的统一文案（Enter 与发送按钮共用）。 */
const SLASH_COMMAND_BUSY_NOTICE = "当前正在运行，请等待本轮结束后再执行斜杠命令";

/**
 * C-8：桌面端斜杠命令通道 —— **唯一入口**，Enter 与发送按钮共用。
 *
 * 命中已知命令（`/plan`、`/tasks`、`/compact`、`/goal`）时不进入 `submitTurn`：
 * 先清空文本草稿（不动待发送附件），再按命令执行动作，反馈统一走
 * `runtimeStore.setSessionNotice()`（C-9，info 模式），不复用 sessionError。
 *
 * 运行中（isSubmitting）一律拒绝执行并给出统一提示：`/compact` 会在本轮进行中
 * 改写会话历史，`/plan`、`/tasks` 的面板切换也与本轮状态无关，故三条命令同权拦截。
 * 守卫只在此处存在一份，禁止在调用方重复判定。
 */
async function executeSlashCommand(command: DesktopSlashCommand) {
  // `/goal` 是唯一带语义参数的命令：必须在清空草稿**之前**留住原文（其余命令不用参数）。
  const commandInput = runtimeStore.draftMessage;

  if (isSubmitting.value) {
    runtimeStore.setSessionNotice(SLASH_COMMAND_BUSY_NOTICE);
    return;
  }

  runtimeStore.draftMessage = "";
  runtimeStore.setSessionNotice(null);

  if (command === "plan") {
    const hasPlanData =
      planStore.plans.length > 0 || planStore.todos.length > 0 || goalStore.hasGoal;
    if (!hasPlanData) {
      runtimeStore.setSessionNotice("当前会话暂无计划");
      return;
    }
    // 契约 C-8「切换面板」：与 HomeSidebar 点击标题同一 toggle 语义
    // （同一面板再执行一次即收起，不同面板则切到目标面板）。
    workspacePanelsStore.togglePanel("plan");
    return;
  }

  if (command === "tasks") {
    if (teamStore.tasks.length === 0) {
      runtimeStore.setSessionNotice("当前暂无 Team 任务");
      return;
    }
    workspacePanelsStore.togglePanel("tasks");
    return;
  }

  // /goal：目标命令（语法/状态机真值源在 Rust `agent::goal_command`，前端只接线与渲染）。
  // 不消耗模型轮次、不写会话历史：草稿已清空且不调用 submitTurn。
  if (command === "goal") {
    try {
      const outcome = await safeInvoke<GoalCommandOutcome>("goal_command", {
        sessionId: runtimeStore.sessionId,
        args: goalCommandArgs(commandInput)
      });
      runtimeStore.setSessionNotice(describeGoalOutcome(outcome));
      // 目标条以后端快照为准（`clear` 后必须归零，续轮随之停止）。
      await goalStore.fetchGoal(runtimeStore.sessionId);
    } catch (error) {
      const reason = extractErrorMessage(String(error)) ?? String(error);
      runtimeStore.setSessionNotice(`目标命令执行失败：${reason}`);
    }
    return;
  }

  // /compact：强制压缩当前会话上下文（真实压缩由后端 agent/compression.rs 执行）
  try {
    const outcome = await safeInvoke<ManualCompactionOutcome>("compact_session", {
      sessionId: runtimeStore.sessionId,
      force: true
    });
    runtimeStore.setSessionNotice(describeCompactionOutcome(outcome));
  } catch (error) {
    const reason = extractErrorMessage(String(error)) ?? String(error);
    runtimeStore.setSessionNotice(`压缩失败：${reason}`);
  }
}

/** 读取当前草稿命中的斜杠命令；未命中返回 null（保持既有普通消息提交行为）。 */
function resolveSlashCommand(): DesktopSlashCommand | null {
  return parseSlashCommand(runtimeStore.draftMessage);
}

function handleComposerKeydown(event: KeyboardEvent) {
  const isUndoShortcut =
    event.key.toLowerCase() === "z" &&
    (event.ctrlKey || event.metaKey) &&
    !event.shiftKey &&
    !event.altKey;
  if (isUndoShortcut) {
    // PA-135：撤销快捷键不受 IME 组合态影响（组合态守卫只服务 Enter 提交）
    event.preventDefault();
    handleUndoLastTurn();
    return;
  }

  // 壳层历史回填：仅当输入框为空 + 裸 ArrowUp 时，回填最近一条用户消息。
  // 输入框非空 / 带修饰键时一律放行，保持原生（桌面=光标移动、多行编辑不受影响）。
  if (
    event.key === "ArrowUp" &&
    !event.shiftKey &&
    !event.altKey &&
    !event.ctrlKey &&
    !event.metaKey
  ) {
    if (
      runtimeStore.draftMessage.length === 0 &&
      !event.isComposing &&
      event.keyCode !== 229
    ) {
      const recalled = runtimeStore.recallLastUserMessage();
      if (recalled !== null) {
        event.preventDefault();
        runtimeStore.setDraftMessage(recalled);
        // 受控 textarea 更新后把光标移到末尾，便于继续编辑或直接发送。
        const target = event.target as HTMLTextAreaElement | null;
        void nextTick(() => {
          if (target && typeof target.setSelectionRange === "function") {
            const end = target.value.length;
            target.setSelectionRange(end, end);
          }
        });
      }
    }
    return;
  }

  if (event.key === "Enter" && !event.shiftKey) {
    // IME 组合中（中文候选确认等）不提交，避免误发送；
    // 旧式 IME 只给 keyCode=229，与 isComposing 同权拦截。
    if (event.isComposing || event.keyCode === 229) {
      return;
    }
    event.preventDefault();
    // C-8：斜杠命令与发送按钮共用同一入口（运行中拒绝执行的守卫在 executeSlashCommand 内）。
    const slashCommand = resolveSlashCommand();
    if (slashCommand) {
      void executeSlashCommand(slashCommand);
      return;
    }
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

  // wave-60 §C8：流式中双击 Esc ⇒ 停止本轮（窗口 `stop_sequence_ms`=500，同身份）。
  // 非流式态**不参与**双击判定、不 preventDefault —— 既有 Esc 语义（各弹窗/输入框）保持不变。
  if (event.key === "Escape") {
    handleStopSequenceEscape(event);
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

  // 全局 Alt+S：切换排队 (Queue) 与插队 (Steer) 发送模式
  if (event.altKey && event.key.toLowerCase() === "s") {
    event.preventDefault();
    runtimeStore.toggleQueueDeliveryMode();
    return;
  }

  // 会话时间线快速导航（组合键同时可作为「用户滚动意图」，由滚动事件接管自动跟随）：
  //   Ctrl/Cmd+T：滚动到会话顶部；Ctrl/Cmd+B：滚动到会话底部（最新）；
  //   Alt+↑：跳到上一轮；Alt+↓：跳到下一轮。
  // IME 组合态（isComposing / keyCode 229）不参与，避免吞掉候选词输入。
  if (!event.isComposing && event.keyCode !== 229) {
    const hasPrimaryModifier = event.ctrlKey || event.metaKey;

    if (hasPrimaryModifier && !event.altKey && !event.shiftKey) {
      const key = event.key.toLowerCase();
      if (key === "t") {
        event.preventDefault();
        handleTimelineScrollToTop();
        return;
      }
      if (key === "b") {
        event.preventDefault();
        handleTimelineScrollToBottom();
        return;
      }
    }

    if (event.altKey && !hasPrimaryModifier && !event.shiftKey) {
      if (event.key === "ArrowUp") {
        event.preventDefault();
        handleNavigateTurn(-1);
        return;
      }
      if (event.key === "ArrowDown") {
        event.preventDefault();
        handleNavigateTurn(1);
        return;
      }
    }
  }
}

/**
 * wave-60 §C8：「流式」判据（双击 Esc 的准入条件）。
 *
 * 只认**正在流式输出**（思考 / 回复 / 重试）的轮次；工具执行态与非流式态不参与双击停止。
 * 与 Rust 侧 `stop_sequence.rs` 判据表对齐：窗口内同身份第 2 次才发**恰好一次**停止请求。
 */
const STOP_SEQUENCE_ELIGIBLE_MODES = new Set(["thinking", "responding", "retrying"]);

function canStopByDoubleEscape(): boolean {
  if (!isSubmitting.value) {
    return false;
  }
  // 已进入 stopping ⇒ 不再重复发请求（幂等由 store 的 phase 单一真源承担）。
  if (phase.value === "stopping") {
    return false;
  }
  return STOP_SEQUENCE_ELIGIBLE_MODES.has(agentRuntimeStatus.value?.mode ?? "");
}

/** wave-60 §C8：Esc 双击 → 触发一次 `stopTurn()`；第 1 次仅「上膛」。 */
function handleStopSequenceEscape(event: KeyboardEvent) {
  if (!canStopByDoubleEscape()) {
    // 非流式 Esc 现有行为不变：不参与双击判定，也不拦截事件。
    stopSequenceState.reset();
    return;
  }
  event.preventDefault();
  const triggered = stopSequenceState.press(
    {
      sessionId: sessionId.value ?? null,
      turnId: activeTurnId.value ?? null,
      // generation 用本轮开始时间戳承载身份代次：同一 turnId 被重新拉起时也判为身份变化。
      generation: runtimeStore.activeTurnStartedAtMs ?? 0
    },
    Date.now()
  );
  if (!triggered) {
    return;
  }
  // 命中即发**一次**停止请求；`stopping` 态由 store 在同一微任务内落地（单一真源）。
  void runtimeStore.stopTurn();
}

async function handlePrimaryAction() {
  // C-8：发送按钮同样走斜杠命令唯一入口（运行中拒绝执行的守卫在 executeSlashCommand 内）。
  const slashCommand = resolveSlashCommand();
  if (slashCommand) {
    await executeSlashCommand(slashCommand);
    return;
  }

  if (isSubmitting.value) {
    // 正在运行/忙碌时，如果用户在输入框有内容，点击发送/回车视为排队/插队发送
    if (draftMessage.value.trim().length > 0) {
      runtimeStore.enqueueMessage(draftMessage.value);
      draftMessage.value = "";
      return;
    }

    // wave-60 §C7：停止请求被受理后 由 store 在同一微任务内置 `phase="stopping"`
    // （单一真源），此处不再维护第二份「已请求停止」的组件内状态（补遗 v2 A7 死变量已删除）。
    await runtimeStore.stopTurn();
    return;
  }

  timelineAutoScroll.setAutoFollowEnabled();
  await runtimeStore.submitTurn();
}

function setComposerShellRef(element: unknown) {
  composerShellRef.value =
    typeof HTMLElement !== "undefined" && element instanceof HTMLElement ? element : null;
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
  isDebugEnabled: isWorkspaceScrollDebugEnabled,
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
  // B098/B105「UI 状态恢复」尚未实现：e3bfca93 只落了本调用而无 resetState 定义，
  // 运行期会抛 ReferenceError 打断挂载。此处按「挂载即初始态」处理，语义与未实现一致。
  if (typeof window !== "undefined") {
    showReasoningContent.value = window.localStorage.getItem(SHOW_REASONING_STORAGE_KEY) === "true";
    loadStreamingRenderConfig();
  }
  syncStreamingPresentationState();
  scheduleStreamingPresentationTimer();
  window.addEventListener("keydown", handleWindowKeydown);
  window.addEventListener("resize", updateFloatingUiPositions);
  const viewport = resolveTimelineViewport();
  if (viewport) {
    viewport.addEventListener("scroll", updateActiveTurnFromViewport, { passive: true });
  }
  window.addEventListener("storage", handleStreamingRenderConfigChanged);
  window.addEventListener(STREAM_RENDER_CONFIG_CHANGED_EVENT, handleStreamingRenderConfigChanged);
});

onBeforeUnmount(() => {
  if (hoverDebounceTimer) clearTimeout(hoverDebounceTimer);
  stopStreamingPresentationTimer();
  window.removeEventListener("keydown", handleWindowKeydown);
  window.removeEventListener("resize", updateFloatingUiPositions);
  const viewport = resolveTimelineViewport();
  if (viewport) {
    viewport.removeEventListener("scroll", updateActiveTurnFromViewport);
  }
  window.removeEventListener("storage", handleStreamingRenderConfigChanged);
  window.removeEventListener(STREAM_RENDER_CONFIG_CHANGED_EVENT, handleStreamingRenderConfigChanged);
  // Wave-59 F3-3：渲染路径锁随组件生命周期清理（键为 message.id，跨会话不复用）。
  streamingRenderPathLock.clear();
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
    // wave-60 §C8：本轮结束（终态/unlock）⇒ 双击 Esc 的上膛失效。
    stopSequenceState.reset();
  }
  handleSubmittingChange(submitting, wasSubmitting);
});

// wave-60 §C8：身份（session/turn/generation）或流式态变化 ⇒ 上膛失效，避免跨轮误触发。
watch(
  () => [sessionId.value, activeTurnId.value, runtimeStore.activeTurnStartedAtMs ?? 0, isSubmitting.value] as const,
  () => {
    stopSequenceState.reset();
  }
);

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
    <QueuedMessagesBubble
      :messages="pendingQueuedMessages"
      @steer="(id) => runtimeStore.promoteToSteer(id)"
      @remove="(id) => runtimeStore.removeQueuedMessage(id)"
    />
    <ScrollArea
      ref="timelineScrollAreaRef"
      class="min-h-0 flex-1 rounded-t-[0.6rem]"
      viewport-class="workspace-timeline-viewport px-4 sm:px-5"
      :hide-scrollbar="true"
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
        <!-- wave-14（用户裁定）：状态条位于消息流最下方（紧贴最后一条消息、随消息流滚动），
             不再固定在输入框上方；运行期间始终可见（含最终回答 streaming）。 -->
        <div
          v-if="shouldShowAgentRuntimeStatus && agentRuntimeStatus"
          class="runtime-status-row w-full min-w-0 pt-2"
          data-testid="workspace-runtime-status-row"
        >
          <div
            class="runtime-status-slot w-full min-w-0"
            data-testid="workspace-runtime-status-slot"
            :data-visible="isRuntimeStatusVisible ? 'true' : 'false'"
            :aria-hidden="isRuntimeStatusVisible ? undefined : 'true'"
            style="pointer-events: none;"
          >
            <div
              class="runtime-status-slot-inner"
              :class="{ 'runtime-status-slot-inner-hidden': !isRuntimeStatusVisible }"
            >
              <AgentRuntimeStatusBar :status="agentRuntimeStatus" />
            </div>
          </div>
        </div>
        <div :style="{ height: COMPOSER_BUFFER_PX + 'px' }" aria-hidden="true"></div>
        <div ref="scrollAnchorRef" aria-hidden="true" class="pointer-events-none" style="height:0;width:0"></div>
      </div>
    </ScrollArea>

    <TurnDial
      :turns="visibleTurns"
      :active-turn-id="activeTurnIdForDial"
      :stick-to-sidebar="rightSidebarOpen"
      @select-turn="handleSelectDialTurn"
    />

    <WorkspaceComposer
      :show-reasoning-content="showReasoningContent"
      :can-undo-last-turn="canUndoLastTurn"
      :undo-shortcut-label="undoShortcutLabel"
      :handle-composer-keydown="handleComposerKeydown"
      :handle-primary-action="handlePrimaryAction"
      :handle-undo-last-turn="handleUndoLastTurn"
      :set-composer-shell-ref="setComposerShellRef"
      @update:show-reasoning-content="onShowReasoningContentChange"
    >
      <template #top-right>
        <button
          v-show="showScrollToBottom"
          :class="[
            'absolute right-0 -top-11 z-20 flex items-center rounded-full border border-stone-200 bg-white/95 text-[12px] font-medium text-stone-600 backdrop-blur-sm transition-all duration-500 ease-out cursor-pointer shadow-[0_2px_8px_rgba(0,0,0,0.08)]',
            scrollToLatestHovered
              ? scrollToLatestHoverClass
              : 'w-[34px] h-[34px] p-0 justify-center gap-0'
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
      </template>
    </WorkspaceComposer>
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

  .runtime-status-slot,
  .runtime-status-slot-inner {
    transition: none !important;
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

/* Wave-11 §2.3/§2.6 + 优化1：底部状态行下移采用硬件加速的平滑过渡，并在出现时平滑淡入 */
.runtime-status-row {
  transition: transform 240ms cubic-bezier(0.23, 1, 0.32, 1);
  will-change: transform;
}

.runtime-status-slot {
  transition:
    opacity 200ms cubic-bezier(0.23, 1, 0.32, 1),
    visibility 0s linear 200ms;
}

.runtime-status-slot[data-visible="true"] {
  opacity: 1;
  visibility: visible;
  transition:
    opacity 200ms cubic-bezier(0.23, 1, 0.32, 1),
    visibility 0s linear 0s;
}

.runtime-status-slot[data-visible="false"] {
  opacity: 0;
  visibility: hidden;
}

.runtime-status-slot-inner {
  transform: translateY(0);
  transition: transform 200ms cubic-bezier(0.23, 1, 0.32, 1);
}

.runtime-status-slot-inner-hidden {
  transform: translateY(2px);
}

/* wave-11.2 F6：隐藏态暂停 orbit 动效，避免不可见时仍持续消耗合成/重绘。 */
.runtime-status-slot[data-visible="false"] :deep(.runtime-status-orbit),
.runtime-status-slot[data-visible="false"] :deep(.runtime-status-orbit *) {
  animation-play-state: paused;
}
</style>
