<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, shallowReactive, watch } from "vue";
import type { ComponentPublicInstance } from "vue";
import { storeToRefs } from "pinia";
import {
  AlertTriangle,
  ArrowDown,
  ArrowUp,
  Activity,
  Check,
  ChevronDown,
  ClipboardList,
  FileSearch,
  FileText,
  Globe,
  History,
  List,
  LoaderCircle,
  Copy,
  MessageSquareMore,
  Pen,
  PenLine,
  Plug,
  RotateCcw,
  ScanSearch,
  Search,
  Square,
  Terminal,
  Undo2,
  UserRound,
  Wrench,
} from "lucide-vue-next";
import type { ProviderConfig, ProviderReasoningEffort } from "@/types/provider";
import type { ChatMessage, ConversationCheckpointEntry, HistoryNode, TraceTimelineEntry } from "@/types/runtime";
import { useProviderStore } from "@/stores/providers";
import { useRuntimeStore } from "@/stores/runtime";
import { extractErrorMessage } from "@/lib/error-utils";
import { useTimelineAutoScroll } from "@/lib/useTimelineAutoScroll";
import { useStreamingPresentationState } from "@/lib/useStreamingPresentationState";

import Button from "@/components/ui/Button.vue";
import { isSimpleTextContent } from "@/lib/markdown";
import MarkdownRenderer from "@/components/MarkdownRenderer.vue";
import ScrollArea from "@/components/ui/ScrollArea.vue";
import {
  PopoverClose,
  PopoverContent,
  PopoverPortal,
  PopoverRoot,
  PopoverTrigger,
  TooltipContent,
  TooltipPortal,
  TooltipRoot,
  TooltipTrigger
} from "reka-ui";

type TurnBucket = {
  turnId: string;
  user: ChatMessage | null;
  assistant: ChatMessage | null;
  tools: ChatMessage[];
  mergedTools: MergedToolCall[];
};

type AgentTurnEvent =
  | { kind: "reasoning"; key: string; order: number; assistant: ChatMessage; reasoningContent: string; streaming: boolean }
  | { kind: "waiting"; key: string; order: number }
  | { kind: "tools"; key: string; order: number; tools: MergedToolCall[] }
  | { kind: "content"; key: string; order: number; assistant: ChatMessage; content: string; streaming: boolean }
  | { kind: "error"; key: string; order: number };

type MergedToolCall = {
  id: string;
  toolName: string;
  canonicalToolName: string | null;
  displayNameZh: string | null;
  mergeKey: string;
  description: string;
  status: ChatMessage["status"];
  durationSeconds: number | null;
  count: number;
};

type ComposerActionKind = "submit" | "resume" | "continue" | "restart";
type CheckpointRollbackAction = "transcript_only" | "transcript_and_workspace";
const SYNTHETIC_KEEP_NODE_PREFIX = "synthetic-keep-";

const runtimeStore = useRuntimeStore();
const providerStore = useProviderStore();
const runtimeStoreSessionList = storeToRefs(runtimeStore).sessionList;

const {
  conversationCheckpointEntries,
  draftMessage,
  historyNodes,
  isSubmitting,
  latestExecutionCheckpoint,
  latestGraphRunSubmissionPlan,
  latestRunControlAuditSummary,
  messages,
  sessionOperation,
  traceTimeline,
  turnTraceHistory
} = storeToRefs(runtimeStore);
const { currentProvider, currentModel } = storeToRefs(providerStore);

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

const providerMenuOpen = ref(false);
const hoveredProviderId = ref<string | null>(null);
const reasoningMenuOpen = ref(false);
const showReasoningContent = ref(false);
const copiedErrorDetailKey = ref<string | null>(null);
const copiedAssistantTurnId = ref<string | null>(null);
const workspaceContentColumnRef = ref<HTMLElement | null>(null);
const ROLLBACK_PROGRESS_MIN_VISIBLE_MS = 0;
const rollbackInFlight = ref<{ turnId: string; action: CheckpointRollbackAction } | null>(null);
const rollbackProgressStyle = ref<Record<string, string | undefined>>({});
const optimisticRollbackTurnId = ref<string | null>(null);
const rollbackExitingTurnIds = ref<Set<string>>(new Set());
const providerMenuRef = ref<HTMLElement | null>(null);
const modelSubmenuStyle = ref<Record<string, string>>({ top: '0' });
const reasoningMenuRef = ref<HTMLElement | null>(null);

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

const currentModelSupportsReasoning = computed(
  () => currentModel.value?.capabilities?.supportsReasoning ?? false
);

const providerLabel = computed(() => {
  const providerName = currentProvider.value?.name?.trim();
  const modelName = currentModel.value?.name?.trim();

  if (providerName && modelName) {
    return `${providerName}/${modelName}`;
  }

  if (providerName) {
    return providerName;
  }

  return "选择 provider/model";
});

const reasoningTriggerTitle = computed(() => {
  if (!currentModel.value) {
    return "当前未选择模型";
  }

  if (!currentModelSupportsReasoning.value) {
    return "当前模型不支持思考强度，可继续设置是否显示思考";
  }

  return "选择思考强度与思考显示方式";
});

const reasoningLabel = computed(() => {
  if (!currentModel.value) {
    return "思考 --";
  }

  if (!currentModelSupportsReasoning.value) {
    return "思考 不支持";
  }

  return providerStore.currentReasoningEffort
    ? `思考 ${reasoningEffortLabelZh(providerStore.currentReasoningEffort)}`
    : "思考 默认";
});

const reasoningOptions: Array<{ label: string; value: ProviderReasoningEffort | null }> = [
  { label: "默认", value: null },
  { label: "低", value: "low" },
  { label: "中", value: "medium" },
  { label: "高", value: "high" },
  { label: "极高", value: "max" }
];

function reasoningEffortLabelZh(value: ProviderReasoningEffort) {
  switch (value) {
    case "low":
      return "低";
    case "medium":
      return "中";
    case "high":
      return "高";
    case "max":
      return "极高";
  }
}

const composerAction = computed<{
  kind: ComposerActionKind;
  label: string;
  hint: string;
}>(() => {
  const actionSummary = latestRunControlAuditSummary.value?.actionEvidenceSummary ?? null;
  const planCommand = latestGraphRunSubmissionPlan.value?.command?.trim().toLowerCase() || null;
  const checkpointCommand = latestExecutionCheckpoint.value?.submissionCommand?.trim().toLowerCase() || null;
  const projectedCommand = actionSummary?.projectedCommand?.trim().toLowerCase() || null;
  const command = projectedCommand || planCommand || checkpointCommand;
  const checkpoint = latestExecutionCheckpoint.value;

  if (actionSummary?.commandKind === "stop_graph_run" && actionSummary.summary.trim()) {
    return {
      kind: "resume",
      label: "恢复",
      hint: actionSummary.summary.trim()
    };
  }

  if (command === "resume_graph_run_stream") {
    return {
      kind: "resume",
      label: "恢复",
      hint: actionSummary?.summary?.trim() || "检测到暂停中的运行；点击后会恢复该 run 并继续执行。"
    };
  }

  if (command === "continue_graph_run_stream") {
    return {
      kind: "continue",
      label: "继续",
      hint: actionSummary?.summary?.trim() || "检测到可继续的 graph run；点击后会接着当前运行推进。"
    };
  }

  if (
    actionSummary?.startReason === "replay_from_checkpoint" ||
    actionSummary?.startReason === "restart_from_checkpoint" ||
    actionSummary?.degraded ||
    checkpoint?.recoveryMode === "replay_required" ||
    checkpoint?.checkpointKind === "lifecycle_boundary" ||
    (command === "start_graph_run_stream" &&
      latestGraphRunSubmissionPlan.value?.source?.trim().toLowerCase() === "checkpoint")
  ) {
    return {
      kind: "restart",
      label: "重新开始",
      hint: actionSummary?.summary?.trim() || "当前恢复点只保留持久化事实；点击后会重新开始新的执行。"
    };
  }

  return {
    kind: "submit",
    label: "发送",
    hint: "输入消息后开始新一轮执行。"
  };
});

const primaryActionDisabled = computed(
  () => Boolean(sessionOperation.value) || (!isSubmitting.value && draftMessage.value.trim().length === 0)
);

const primaryActionTitle = computed(() =>
  isSubmitting.value ? "请求在安全边界停止当前运行。" : composerAction.value.hint
);

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

/** 将已按 model-hop 裁剪的可见文本拆为逐字数组，用于逐字淡入渲染 */
function streamingVisibleChars(text: string) {
  if (!text) return [];
  return text.split("");
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

// 主对话渲染与 trace 渲染解耦：agentTurnEvents 的推导结果集中缓存为 computed，
// template 按 turnId 取缓存，避免每次渲染都内联重算整条 trace timeline 推导链
// （traceTimelineForTurn + modelTraceEntries + 多 hop 内容拼接）。
const turnEventsByTurnId = computed(() => {
  const eventsByTurnId = new Map<string, AgentTurnEvent[]>();
  for (const turn of turns.value) {
    eventsByTurnId.set(turn.turnId, agentTurnEvents(turn));
  }
  return eventsByTurnId;
});

const toolIconByCanonicalName: Record<string, any> = {
  Run: Terminal,
  Ask: MessageSquareMore,
  Read: FileText,
  Search: Search,
  List: List,
  Glob: FileSearch,
  WebFetch: Globe,
  WebSearch: ScanSearch,
  MCPResource: Plug,
  ToolSearch: Wrench,
  Write: Pen,
  Edit: PenLine,
  Plan: ClipboardList,
};

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

function shouldOpenReasoningBlock(_message: ChatMessage | null) {
  return false;
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

function userShellClass() {
  return "rounded-[0.45rem] bg-stone-900 px-3 py-2 text-stone-50 shadow-[0_1px_0_rgba(28,25,23,0.03)] sm:px-4";
}

function actorLabelClass() {
  return "inline-flex items-center gap-2 text-[10px] uppercase tracking-[0.18em] text-stone-500";
}

function assistantTone(message: ChatMessage | null) {
  if (!message) {
    return "";
  }

  return "text-stone-800";
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

function assistantErrorCopyKey(turnId: string) {
  return `assistant-error-${turnId}`;
}

function copyErrorDetail(turnId: string, text: string) {
  copiedErrorDetailKey.value = assistantErrorCopyKey(turnId);

  if (typeof navigator !== "undefined" && navigator.clipboard?.writeText) {
    void navigator.clipboard.writeText(text);
  }

  if (typeof window !== "undefined") {
    window.setTimeout(() => {
      if (copiedErrorDetailKey.value === assistantErrorCopyKey(turnId)) {
        copiedErrorDetailKey.value = null;
      }
    }, 1200);
  }
}

function copyAssistantResponse(turnId: string, content: string) {
  copiedAssistantTurnId.value = turnId;

  if (typeof navigator !== "undefined" && navigator.clipboard?.writeText) {
    void navigator.clipboard.writeText(content);
  }

  if (typeof window !== "undefined") {
    window.setTimeout(() => {
      if (copiedAssistantTurnId.value === turnId) {
        copiedAssistantTurnId.value = null;
      }
    }, 1200);
  }
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

function toggleProviderMenu() {
  providerMenuOpen.value = !providerMenuOpen.value;

  if (providerMenuOpen.value) {
    const initialId = currentProvider.value?.id ?? providerStore.providers[0]?.id ?? null;
    hoveredProviderId.value = initialId;
    reasoningMenuOpen.value = false;

    // Adjust initial submenu position after DOM renders
    if (initialId) {
      nextTick(() => {
        const buttonEl = providerMenuRef.value?.querySelector<HTMLElement>(
          `[data-provider-id="${initialId}"]`
        );
        const provider = providerStore.providers.find((p) => p.id === initialId);
        if (provider && buttonEl) {
          adjustModelSubmenuPosition(provider, buttonEl);
        }
      });
    }
    return;
  }

  hoveredProviderId.value = null;
}

function adjustModelSubmenuPosition(provider: ProviderConfig, button: HTMLElement) {
  const buttonRect = button.getBoundingClientRect();
  const viewportHeight = window.innerHeight;

  // Estimate submenu height from its content
  const modelCount = provider.models?.length ?? 0;
  const captionHeight = 28;
  const itemHeight = 30;
  const padding = 12;
  const estimatedHeight = captionHeight + (modelCount > 0 ? modelCount * itemHeight + padding : itemHeight);

  const spaceBelow = viewportHeight - buttonRect.bottom - 8;

  if (estimatedHeight > spaceBelow) {
    const overflow = estimatedHeight - spaceBelow;
    modelSubmenuStyle.value = { top: `-${overflow}px` };
  } else {
    modelSubmenuStyle.value = { top: '0' };
  }
}

function onProviderEnter(provider: ProviderConfig, event: MouseEvent) {
  hoveredProviderId.value = provider.id;
  adjustModelSubmenuPosition(provider, event.currentTarget as HTMLElement);
}

function onProviderFocus(provider: ProviderConfig, event: FocusEvent) {
  hoveredProviderId.value = provider.id;
  adjustModelSubmenuPosition(provider, event.currentTarget as HTMLElement);
}

function toggleReasoningMenu() {
  if (!currentModel.value) {
    return;
  }

  reasoningMenuOpen.value = !reasoningMenuOpen.value;
  if (reasoningMenuOpen.value) {
    providerMenuOpen.value = false;
    hoveredProviderId.value = null;
  }
}

async function selectModel(providerId: string, modelId: string) {
  providerStore.selectModel(providerId, modelId);
  providerMenuOpen.value = false;
  hoveredProviderId.value = providerId;
  await providerStore.saveRegistry();
}

function selectReasoningEffort(value: ProviderReasoningEffort | null) {
  providerStore.setCurrentReasoningEffort(value);
  reasoningMenuOpen.value = false;
}

function toggleReasoningVisibility() {
  showReasoningContent.value = !showReasoningContent.value;
  reasoningMenuOpen.value = false;
}

function handleClickOutside(event: MouseEvent) {
  const target = event.target as Node | null;

  if (providerMenuRef.value && target && !providerMenuRef.value.contains(target)) {
    providerMenuOpen.value = false;
    hoveredProviderId.value = null;
  }

  if (reasoningMenuRef.value && target && !reasoningMenuRef.value.contains(target)) {
    reasoningMenuOpen.value = false;
  }
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
  window.addEventListener("click", handleClickOutside);
  window.addEventListener("keydown", handleWindowKeydown);
  window.addEventListener("resize", updateFloatingUiPositions);
  window.addEventListener("storage", handleStreamingRenderConfigChanged);
  window.addEventListener(STREAM_RENDER_CONFIG_CHANGED_EVENT, handleStreamingRenderConfigChanged);
});

onBeforeUnmount(() => {
  if (hoverDebounceTimer) clearTimeout(hoverDebounceTimer);
  stopStreamingPresentationTimer();
  window.removeEventListener("click", handleClickOutside);
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
        <section v-for="turn in visibleTurns" :key="turn.turnId" class="space-y-3" :data-turn-id="turn.turnId">
          <article v-if="turn.user" :ref="(element) => setLatestUserMessageRef(element, turn.turnId)" class="conversation-user-message ml-auto w-fit max-w-[68.8%] sm:max-w-[54.4%]">
            <div class="flex flex-col items-end">
              <div :class="actorLabelClass()" class="mb-1">
                <span>User</span>
                <UserRound class="h-3.5 w-3.5" />
              </div>
              <div :class="userShellClass()">
                <div class="text-left whitespace-pre-wrap text-sm leading-6">
                  {{ turn.user.content }}
                </div>
              </div>
              <div
                class="message-action-bar relative self-start mt-1.5 flex items-center gap-1.5"
                data-testid="workspace-user-checkpoint-actions"
              >
                <PopoverRoot>
                  <PopoverTrigger as-child>
                    <button
                      class="checkpoint-icon-button"
                      type="button"
                      :disabled="isSubmitting || !!sessionOperation || !!rollbackInFlight"
                      :title="isSubmitting ? '运行中暂不可撤回' : '仅撤回对话'"
                    >
                      <History class="h-3.5 w-3.5" />
                      <span class="sr-only">仅撤回对话</span>
                    </button>
                  </PopoverTrigger>
                  <PopoverPortal>
                    <PopoverContent side="top" align="center" :side-offset="6" class="z-50 rounded-[0.3rem] border border-stone-200/70 bg-white/97 px-2.5 py-1.5 shadow-md backdrop-blur">
                      <div class="flex items-center gap-1 text-nowrap">
                        <span class="text-[11px] leading-none text-stone-400 select-none">确认仅撤回对话？</span>
                        <PopoverClose as-child>
                          <button type="button" class="inline-flex items-center justify-center w-5 h-5 rounded-[0.25rem] text-rose-500 hover:bg-rose-100/80 hover:text-rose-600 transition cursor-pointer cursor-pointer" @click="confirmRollback(turn.turnId, 'transcript_only')">
                            <Check class="h-3 w-3" />
                          </button>
                        </PopoverClose>
                      </div>
                    </PopoverContent>
                  </PopoverPortal>
                </PopoverRoot>
                <PopoverRoot>
                  <PopoverTrigger as-child>
                    <button
                      class="checkpoint-icon-button"
                      type="button"
                      :disabled="isSubmitting || !!sessionOperation || !!rollbackInFlight"
                      :title="isSubmitting ? '运行中暂不可撤回' : '撤回对话和修改'"
                    >
                      <RotateCcw class="h-3.5 w-3.5" />
                      <span class="sr-only">撤回对话和修改</span>
                    </button>
                  </PopoverTrigger>
                  <PopoverPortal>
                    <PopoverContent side="top" align="center" :side-offset="6" class="z-50 rounded-[0.3rem] border border-stone-200/70 bg-white/97 px-2.5 py-1.5 shadow-md backdrop-blur">
                      <div class="flex items-center gap-1 text-nowrap">
                        <span class="text-[11px] leading-none text-stone-400 select-none">确认撤回对话和文件？</span>
                        <PopoverClose as-child>
                          <button type="button" class="inline-flex items-center justify-center w-5 h-5 rounded-[0.25rem] text-rose-500 hover:bg-rose-100/80 hover:text-rose-600 transition cursor-pointer cursor-pointer" @click="confirmRollback(turn.turnId, 'transcript_and_workspace')">
                            <Check class="h-3 w-3" />
                          </button>
                        </PopoverClose>
                      </div>
                    </PopoverContent>
                  </PopoverPortal>
                </PopoverRoot>
                </div>
              </div>
            </article>

          <article v-if="shouldShowAgentArticle(turn)" :ref="(element) => setLatestAgentMessageRef(element, turn.turnId)" class="conversation-agent-shell flex w-full flex-col gap-2 px-0 py-1">


            <template v-for="event in (turnEventsByTurnId.get(turn.turnId) ?? [])" :key="event.key">
              <details
                v-if="event.kind === 'reasoning'"
                :open="shouldOpenReasoningBlock(event.assistant)"
                v-motion
                :initial="{ opacity: 0, y: 6 }"
                :animate="{ opacity: 1, y: 0 }"
                :transition="{ duration: 0.2, ease: 'easeOut', delay: 0.32 }"
                class="conversation-disclosure conversation-reasoning-panel group p-0"
                style="contain: layout;"
              >
                <summary class="conversation-disclosure-summary">
                  <div class="flex min-w-0 items-center gap-2">
                    <Activity class="h-3 w-3 shrink-0 text-stone-400" />
                    <span>思考过程</span>
                  </div>
                  <ChevronDown class="conversation-disclosure-chevron h-3.5 w-3.5 shrink-0 text-stone-400" />
                </summary>
                <div class="mt-1 pl-5 whitespace-pre-wrap break-words text-[13px] leading-[1.4] text-stone-400">
                  <template v-if="event.reasoningContent">
                    <span class="reasoning-italic">{{ event.streaming ? streamingReasoningStable(event.reasoningContent, event.assistant) : event.reasoningContent }}</span>
                    <span
                      v-if="event.streaming && streamingReasoningFade(event.reasoningContent, event.assistant)"
                      :key="`rfade-${event.assistant.id}-${assistantDisplayedReasoningFadeKey(event.assistant)}`"
                      class="assistant-streaming-fade reasoning-italic"
                      :style="assistantDisplayedReasoningFadeStyle(event.assistant)"
                    >
                      {{ streamingReasoningFade(event.reasoningContent, event.assistant) }}
                    </span>
                  </template>
                </div>
              </details>

              <Transition v-else-if="event.kind === 'waiting'" name="assistant-waiting-signal">
                <div
                  class="assistant-waiting-panel"
                  role="status"
                  aria-label="等待回复开始"
                  data-testid="assistant-awaiting-first-signal"
                >
                  <span class="assistant-waiting-dots" aria-hidden="true">
                    <span
                      v-for="index in 3"
                      :key="index"
                      class="assistant-waiting-dot"
                      :style="{ animationDelay: `${(index - 1) * 150}ms` }"
                    ></span>
                  </span>
                </div>
              </Transition>

              <div
                v-else-if="event.kind === 'tools'"
                class="conversation-tool-panel space-y-0.5"
              >
                <div
                  v-for="tool in event.tools"
                  :key="tool.id"
                  class="flex flex-col py-0.5 text-[12px] leading-5"
                >
                  <div class="flex items-center gap-2">
                    <component :is="toolIconByCanonicalName[tool.canonicalToolName ?? ''] ?? Wrench" class="h-3 w-3 shrink-0 text-stone-400" />
                    <span
                      v-if="tool.displayNameZh || tool.canonicalToolName || tool.toolName || tool.description"
                      class="conversation-tool-name min-w-0 truncate"
                      :class="tool.status === 'error' ? 'text-rose-600' : 'text-stone-400'"
                    >
                      {{ tool.displayNameZh || tool.canonicalToolName || tool.toolName || tool.description }}
                    </span>
                    <span
                      v-if="tool.description && tool.description !== (tool.displayNameZh || tool.canonicalToolName || tool.toolName)"
                      class="conversation-tool-detail min-w-0 truncate text-stone-400"
                    >
                      {{ tool.description }}
                    </span>
                    <span v-if="tool.count > 1" class="shrink-0 text-[11px] text-stone-300">({{ tool.count }}x)</span>
                    <span class="conversation-tool-status flex shrink-0 items-center gap-1 leading-none">
                      <span class="text-[11px] text-stone-400" :class="tool.durationSeconds != null ? 'visible' : 'invisible'">{{ tool.durationSeconds != null ? (tool.durationSeconds).toFixed(1) + 's' : '0.0s' }}</span>
                      <LoaderCircle v-if="tool.status === 'pending'" class="h-3 w-3 animate-spin text-stone-400" />
                      <Check v-else-if="tool.status === 'done'" class="h-3 w-3 text-stone-400" />
                      <AlertTriangle v-else-if="tool.status === 'error'" class="h-3 w-3 shrink-0 text-rose-400" aria-label="工具调用失败" :aria-hidden="false" />
                    </span>
                  </div>
                </div>
              </div>

              <div
                v-else-if="event.kind === 'content'"
                class="assistant-response-panel"
              >
                <!-- 统一 shell：流式/完成共享同一外容器，避免 v-if/v-else 导致的 DOM 子树替换 -->
                <div
                  class="assistant-plain-text text-sm"
                  :class="assistantTone(event.assistant)"
                  :data-streaming="event.streaming ? 'true' : undefined"
                >
                  <div
                    v-if="shouldUseMarkdownAssistantRendering(event.assistant, event.content)"
                    :class="event.streaming ? 'assistant-streaming-content' : undefined"
                    :data-testid="event.streaming ? 'assistant-streaming-flow' : undefined"
                  >
                    <MarkdownRenderer
                      :content="event.content"
                      :streaming="event.streaming"
                      :force-markdown-streaming="event.streaming"
                      :wrapper-class="[
                        'assistant-markdown',
                        event.streaming ? 'assistant-streaming-markdown' : ''
                      ].filter(Boolean).join(' ')"
                      :tone-class="assistantTone(event.assistant)"
                      @render-complete="handleMarkdownRenderComplete"
                    />
                  </div>
                  <div
                    v-else-if="event.streaming"
                    class="assistant-streaming-content"
                    data-testid="assistant-streaming-flow"
                  >
                    <template v-if="shouldUseOptimizedAssistantStreaming(event.assistant)">
                      <span v-for="(char, i) in streamingVisibleChars(event.content)" :key="i" class="assistant-streaming-char">{{ char }}</span>
                    </template>
                    <template v-else>{{ event.content }}</template>
                  </div>
                  <MarkdownRenderer
                    v-else
                    :content="event.content"
                    :streaming="false"
                    wrapper-class="assistant-markdown"
                    :tone-class="assistantTone(event.assistant)"
                    @render-complete="handleMarkdownRenderComplete"
                  />
                </div>
              </div>

              <!-- Error detail panel (raw error for debugging) -->
              <details
                v-else-if="event.kind === 'error'"
                class="conversation-disclosure conversation-error-panel mt-3 group"
              >
                <summary class="conversation-disclosure-summary text-rose-700">
                  <div class="flex min-w-0 items-center gap-2">
                    <AlertTriangle class="h-3.5 w-3.5 shrink-0 text-rose-500" />
                    <span class="text-rose-700">错误详情</span>
                  </div>
                  <div class="ml-auto flex items-center gap-1">
                    <button
                      class="invisible group-hover:visible inline-flex h-5 w-5 items-center justify-center rounded-[0.35rem] text-stone-400 transition hover:bg-rose-50 hover:text-rose-600"
                      type="button"
                      :data-testid="`workspace-error-copy-${turn.turnId}`"
                      @click.stop="copyErrorDetail(turn.turnId, assistantErrorDetail(turn))"
                    >
                      <component
                        :is="copiedErrorDetailKey === assistantErrorCopyKey(turn.turnId) ? Check : Copy"
                        class="h-3 w-3"
                      />
                    </button>
                    <ChevronDown class="conversation-disclosure-chevron h-3.5 w-3.5 shrink-0 text-stone-400" />
                  </div>
                </summary>
                <div
                  class="whitespace-pre-wrap break-words px-3 py-2 text-[11px] leading-4 text-rose-900"
                  data-testid="workspace-error-detail"
                >
                  {{ assistantErrorDetail(turn) }}
                </div>
              </details>
            </template>

            <div
              v-if="turn.assistant && !isAssistantStreaming(turn.assistant)"
              class="agent-action-bar ml-auto mt-3 flex flex-wrap items-center justify-end gap-2"
              data-testid="workspace-agent-actions"
            >
              <button
                class="checkpoint-icon-button"
                type="button"
                :title="copiedAssistantTurnId === turn.turnId ? '已复制' : '复制回复'"
                @click="copyAssistantResponse(turn.turnId, turn.assistant?.content ?? '')"
              >
                <component :is="copiedAssistantTurnId === turn.turnId ? Check : Copy" class="h-3.5 w-3.5" />
                <span class="sr-only">复制回复</span>
              </button>
            </div>
          </article>
        </section>
      </TransitionGroup>
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

    <div class="absolute bottom-0 left-0 right-0 z-10 px-4 py-3 sm:px-5 pointer-events-none">
      <div
        ref="composerShellRef"
        class="relative mx-auto w-full max-w-[38.4rem] rounded-[0.6rem] bg-white/76 px-4 py-3 shadow-[0_-4px_20px_-2px_rgba(60,40,20,0.06)] backdrop-blur-[8px]"
        data-testid="workspace-composer-shell"
        style="pointer-events: auto;"
      >
      <textarea
        :value="draftMessage"
        :disabled="Boolean(sessionOperation)"
        data-testid="workspace-composer-input"
        class="min-h-[82px] w-full resize-none bg-transparent px-0 py-0 text-[13px] leading-[1.55] text-stone-800 outline-none placeholder:text-[12px] placeholder:font-normal placeholder:tracking-[0.01em] placeholder:text-stone-400/70"
        placeholder="输入消息，按 Enter 发送，Shift+Enter 换行。"
        @input="runtimeStore.setDraftMessage(($event.target as HTMLTextAreaElement).value)"
        @keydown="handleComposerKeydown"
      />

      <div class="mt-3 flex flex-wrap items-center justify-between gap-x-3 gap-y-2 border-t border-stone-200/70 pt-2.5">
        <div class="flex min-w-0 flex-wrap items-center gap-2">


          <div ref="providerMenuRef" class="relative">
            <button
              class="composer-select-trigger"
              type="button"
              @click.stop="toggleProviderMenu"
            >
              <span class="truncate">{{ providerLabel }}</span>
              <ChevronDown class="h-2.5 w-2.5 text-stone-400" />
            </button>

            <div
              v-if="providerMenuOpen"
              class="composer-menu-panel absolute bottom-[calc(100%+0.45rem)] left-0 z-20 min-w-[14rem]"
            >
              <div class="composer-menu-caption">提供商</div>
              <div class="composer-menu-divider"></div>
              <div class="py-0.5">
                <div
                  v-for="provider in providerStore.providers"
                  :key="provider.id"
                  class="relative"
                >
                  <button
                    class="composer-menu-item"
                    type="button"
                    :data-provider-id="provider.id"
                    @mouseenter="onProviderEnter(provider, $event)"
                    @focus="onProviderFocus(provider, $event)"
                  >
                    <span class="truncate">{{ provider.name }}</span>
                    <div class="flex items-center gap-2">
                      <Check v-if="currentProvider?.id === provider.id" class="h-3.5 w-3.5 text-stone-700" />
                      <ChevronDown class="h-3.5 w-3.5 -rotate-90 text-stone-400" />
                    </div>
                  </button>
                  <div
                    v-if="hoveredProviderId === provider.id"
                    class="composer-menu-panel absolute left-full ml-1 min-w-[14rem]"
                    :style="modelSubmenuStyle"
                  >
                    <div class="composer-menu-caption">模型</div>
                    <div class="composer-menu-divider"></div>
                    <button
                      v-for="model in provider.models ?? []"
                      :key="model.id"
                      class="composer-menu-item"
                      type="button"
                      @click="selectModel(provider.id, model.id)"
                    >
                      <span class="truncate">{{ model.name }}</span>
                      <Check
                        v-if="currentProvider?.id === provider.id && currentModel?.id === model.id"
                        class="h-3.5 w-3.5 text-stone-700"
                      />
                    </button>
                  </div>
                </div>
              </div>
            </div>
          </div>

          <div ref="reasoningMenuRef" class="relative">
            <button
              class="composer-select-trigger"
              type="button"
              :disabled="!currentModel"
              :title="reasoningTriggerTitle"
              @click.stop="toggleReasoningMenu"
            >
              <span class="truncate">{{ reasoningLabel }}</span>
              <ChevronDown class="h-2.5 w-2.5 text-stone-400" />
            </button>

            <div
              v-if="currentModel && reasoningMenuOpen"
              class="composer-menu-panel absolute bottom-[calc(100%+0.45rem)] left-0 z-20 min-w-[10rem]"
            >
              <div class="composer-menu-caption">思考强度</div>
              <div class="composer-menu-divider"></div>
              <template v-if="currentModelSupportsReasoning">
                <button
                  v-for="option in reasoningOptions"
                  :key="option.label"
                  class="composer-menu-item"
                  type="button"
                  @click="selectReasoningEffort(option.value)"
                >
                  <span>{{ option.label }}</span>
                  <Check
                    v-if="(providerStore.currentReasoningEffort ?? null) === option.value"
                    class="h-3.5 w-3.5 text-stone-700"
                  />
                </button>
              </template>
              <div
                v-else
                class="composer-menu-note px-3 py-2 text-[11px] leading-5 text-stone-400"
                data-testid="reasoning-unsupported-note"
              >
                当前模型不支持思考强度
              </div>
              <div class="composer-menu-divider"></div>
              <div class="composer-menu-caption">显示设置</div>
              <button
                class="composer-menu-item"
                data-testid="reasoning-visibility-toggle"
                type="button"
                @click="toggleReasoningVisibility"
              >
                <div class="flex min-w-0 flex-col">
                  <span>显示思考</span>
                  <span class="composer-menu-item-hint">
                    {{ showReasoningContent ? "已开启" : "已关闭" }}
                  </span>
                </div>
                <Check v-if="showReasoningContent" class="h-3.5 w-3.5 text-stone-700" />
              </button>
            </div>
          </div>

          <TooltipRoot :delay-duration="300">
            <TooltipTrigger as-child>
              <span tabindex="0" class="inline-flex">
                <button
                  class="checkpoint-icon-button !rounded-full"
                  type="button"
                  :disabled="!canUndoLastTurn"
                  data-testid="workspace-undo-button"
                  @click="handleUndoLastTurn"
                >
                  <Undo2 class="h-3.5 w-3.5" />
                  <span class="sr-only">{{ canUndoLastTurn ? '撤回' : '撤回不可用' }}</span>
                </button>
              </span>
            </TooltipTrigger>
            <TooltipPortal>
              <TooltipContent side="top" :side-offset="4" class="z-50 overflow-hidden rounded-md border border-stone-200 bg-white px-3 py-1.5 text-xs text-stone-700 shadow-sm">
                {{ canUndoLastTurn ? `${undoShortcutLabel} 撤回` : '没有可撤回的对话' }}
              </TooltipContent>
            </TooltipPortal>
          </TooltipRoot>
        </div>

        <Button
          class="h-8 w-8 rounded-full p-0"
          size="sm"
          :disabled="primaryActionDisabled"
          :title="primaryActionTitle"
          :data-testid="isSubmitting ? 'workspace-stop-turn' : 'workspace-submit-action'"
          @click="handlePrimaryAction"
        >
          <Square v-if="isSubmitting" class="h-3.5 w-3.5 fill-current" />
          <ArrowUp v-if="!isSubmitting" class="h-3.5 w-3.5" />
          <span class="sr-only">{{ isSubmitting ? "停止" : composerAction.label }}</span>
        </Button>
        </div>
      </div>
    </div>
  </section>
</template>

<style scoped>
:deep(.markdown-body) {
  min-width: 0;
  overflow-wrap: anywhere;
  word-break: break-word;
  line-height: 1.7;
  color: #3d342d;
}

:deep(.assistant-markdown > :first-child) {
  margin-top: 0;
}

:deep(.assistant-markdown > :last-child) {
  margin-bottom: 0;
}

:deep(.assistant-markdown p) {
  margin: 0 0 1.2em;
  color: inherit;
}

:deep(.assistant-markdown p:last-child) {
  margin-bottom: 0;
}

:deep(.assistant-markdown h1),
:deep(.assistant-markdown h2),
:deep(.assistant-markdown h3),
:deep(.assistant-markdown h4),
:deep(.assistant-markdown h5),
:deep(.assistant-markdown h6) {
  margin: 1rem 0 0.45rem;
  font-size: inherit;
  line-height: 1.5;
  letter-spacing: 0;
  color: #241b14;
}

:deep(.assistant-markdown h1),
:deep(.assistant-markdown h2) {
  font-weight: 600;
}

:deep(.assistant-markdown h3),
:deep(.assistant-markdown h4),
:deep(.assistant-markdown h5),
:deep(.assistant-markdown h6) {
  font-weight: 520;
  color: #3c3028;
}

:deep(.assistant-markdown ul),
:deep(.assistant-markdown ol) {
  margin: 0.5rem 0 0.95rem 1.35rem;
  padding: 0;
}

:deep(.assistant-markdown li + li) {
  margin-top: 0.38rem;
}

:deep(.assistant-markdown li > p) {
  margin: 0.35rem 0;
}

:deep(.assistant-markdown pre) {
  margin: 0.8rem 0;
  overflow-x: auto;
  border-radius: 0.45rem;
  background: #faf5eb;
  padding: 0.8rem 0.9rem;
  font-size: 0.82rem;
  line-height: 1.55;
  color: #2f261d;
}

:deep(.assistant-markdown code) {
  border-radius: 0.3rem;
  background: #faf5eb;
  padding: 0.08rem 0.34rem;
  font-size: 0.82em;
  color: #5b4330;
}

:deep(.assistant-markdown pre code) {
  background: transparent;
  padding: 0;
  border-radius: 0;
  display: block;
  color: #2f261d;
  font-size: 1em;
  white-space: pre-wrap;
  word-break: break-word;
  overflow-wrap: anywhere;
}

:deep(.assistant-markdown .table-scroll-wrapper) {
  overflow-x: auto;
  margin: 0.8rem 0;
}

:deep(.assistant-markdown .table-scroll-wrapper table) {
  width: 100%;
  table-layout: auto;
  border-collapse: collapse;
  border-spacing: 0;
  background: transparent;
  border-radius: 0;
}

:deep(.assistant-markdown .table-scroll-wrapper thead th) {
  background: transparent;
  font-weight: 520;
  color: #56463a;
  border-bottom: 2px solid rgba(112, 76, 40, 0.98);
}


:deep(.assistant-markdown .table-scroll-wrapper th),
:deep(.assistant-markdown .table-scroll-wrapper td) {
  padding: 0.2rem 0.8rem;
  vertical-align: top;
  text-align: left;
  white-space: normal;
  word-break: break-word;
  overflow-wrap: anywhere;
  max-width: 320px;
}

:deep(.assistant-markdown .table-scroll-wrapper tbody td) {
  border-bottom: 0;
}

:deep(.assistant-markdown .table-scroll-wrapper tbody tr:last-child td) {
  border-bottom: 1px solid rgba(232, 210, 175, 0.98);
  background-image: none;
  background-position: initial;
  background-size: auto;
  background-repeat: repeat;
}

:deep(.assistant-markdown a) {
  color: #8b5e34;
  text-decoration: underline;
  text-underline-offset: 2px;
}

:deep(.assistant-markdown blockquote) {
  margin: 0.8rem 0;
  padding: 0.2rem 0 0.2rem 0.8rem;
  color: #71665c;
  background: transparent;
  border-left: 2px solid rgba(139, 94, 52, 0.28);
  border-radius: 0;
  font-style: normal;
}

:deep(.assistant-markdown blockquote p:last-child) {
  margin-bottom: 0;
}

:deep(.assistant-markdown hr) {
  margin: 1.15rem 0;
  height: 0.5px;
  border: 0;
  background: linear-gradient(90deg, transparent, rgba(198, 174, 147, 0.18), transparent);
}

:deep(.assistant-markdown img) {
  display: block;
  max-width: 100%;
  height: auto;
  border-radius: 0.55rem;
}

:deep(.assistant-markdown strong) {
  color: #1f1712;
  font-weight: 520;
}

:deep(.assistant-markdown del) {
  color: #8a7c6c;
}

:deep(.assistant-markdown input[type="checkbox"]) {
  margin: 0 0.35rem 0 0;
  transform: translateY(1px);
  accent-color: #8b5e34;
}

.assistant-plain-text {
  overflow-wrap: anywhere;
  word-break: break-word;
  line-height: 1.7;
  color: #3d342d;
}

/* 每个 turn 独立渲染隔离，防止工具状态变化/思考流式时整列重排 */
[data-testid="workspace-content-column"] > div > section[data-turn-id] {
  content-visibility: auto;
  contain-intrinsic-size: auto 3rem;
}

/* 工具状态与时长容器：预留足够宽度避免 duration 出现时布局偏移 */
.conversation-tool-status {
  min-width: 5.5rem;
  text-align: right;
  justify-content: flex-end;
}

/* 内容列创建布局隔离边界，减少流式输出时的连锁重排 */
[data-testid="workspace-content-column"] {
  contain: layout style paint;
  overflow-anchor: none;
}

:deep(.workspace-timeline-viewport) {
  overflow-anchor: none;
}

.turn-flow-enter-active,
.turn-flow-leave-active {
  transition:
    opacity 320ms cubic-bezier(0.25, 0.46, 0.45, 0.94),
    transform 320ms cubic-bezier(0.25, 0.46, 0.45, 0.94);
}

.turn-flow-leave-active {
  position: absolute;
  left: 0;
  right: 0;
}

.turn-flow-enter-from {
  opacity: 0;
  transform: translateY(0.55rem);
}

.turn-flow-leave-to {
  opacity: 0;
  transform: translateY(-0.35rem);
}

.turn-flow-move {
  transition: transform 360ms cubic-bezier(0.25, 0.46, 0.45, 0.94);
}

/* v-motion 面板：预声明 transform 层以减少首次动画跳变 */
.conversation-tool-row,
.conversation-tool-panel,
.conversation-reasoning-panel,
.assistant-response-panel {
  will-change: opacity, transform;
}

/* 非流式 content 入场使用极简动画，避免 `animation-fill-mode: both` 在流式→完成过渡时
   将已渲染的可见元素重置为 opacity:0 导致闪烁。初始渲染靠 streaming char 逐字淡入完成，
   完成后不再需要额外入场动画。 */
.assistant-response-panel.motion-entrance {
  animation: panel-fade-in 0.18s ease-out;
}

@keyframes panel-fade-in {
  from {
    opacity: 0;
    transform: translateY(3px);
  }
  to {
    opacity: 1;
    transform: translateY(0);
  }
}

.streaming-unrendered-suffix {
  word-break: break-word;
  overflow-wrap: anywhere;
  line-height: 1.7;
  color: #3d342d;
}

.assistant-streaming-content {
  word-break: break-word;
  overflow-wrap: anywhere;
  line-height: 1.7;
  transition: color 140ms ease;
}

.assistant-streaming-markdown {
  display: block;
  white-space: normal;
}

.assistant-streaming-content .assistant-streaming-markdown:empty {
  display: none;
}

.assistant-streaming-fade {
  display: inline;
  white-space: pre-wrap;
}

.assistant-streaming-char {
  display: inline;
  white-space: pre-wrap;
  animation-name: assistant-stream-char-fade-in;
  animation-duration: 50ms;
  animation-timing-function: ease-out;
  animation-fill-mode: both;
}

@keyframes assistant-stream-char-fade-in {
  from {
    opacity: 0;
    transform: translateY(-0.04em);
  }
  to {
    opacity: 1;
    transform: translateY(0);
  }
}

@keyframes assistant-stream-fade-in {
  from { opacity: 0; }
  to   { opacity: 1; }
}

.assistant-waiting-panel {
  display: inline-flex;
  min-height: 1.7rem;
  align-items: center;
  padding: 0.1rem 0;
}

.assistant-waiting-dots {
  display: inline-flex;
  align-items: center;
  gap: 0.34rem;
}

.assistant-waiting-dot {
  width: 0.46rem;
  height: 0.46rem;
  border-radius: 9999px;
  background: rgba(87, 83, 78, 0.58);
  animation: assistant-waiting-dot-bounce 900ms cubic-bezier(0.2, 0.7, 0.35, 1) infinite both;
}

.assistant-waiting-signal-enter-active,
.assistant-waiting-signal-leave-active {
  transition:
    opacity 220ms ease,
    transform 220ms ease;
}

.assistant-waiting-signal-enter-from,
.assistant-waiting-signal-leave-to {
  opacity: 0;
  transform: translateY(0.25rem);
}

@keyframes assistant-waiting-dot-bounce {
  0%,
  70%,
  100% {
    opacity: 0.42;
    transform: translateY(0) scale(0.86);
  }

  35% {
    opacity: 1;
    transform: translateY(-0.34rem) scale(1.08);
  }
}

@media (prefers-reduced-motion: reduce) {
  .conversation-tool-panel,
  .conversation-reasoning-panel,
  .assistant-response-panel {
    animation: none !important;
    opacity: 1 !important;
    transform: none !important;
    transition: none !important;
  }

  .assistant-streaming-fade,
  .assistant-streaming-char,
  .assistant-waiting-dot {
    animation: none !important;
    opacity: 1 !important;
    transform: none !important;
  }

  .assistant-waiting-signal-enter-active,
  .assistant-waiting-signal-leave-active {
    transition: opacity 120ms ease !important;
  }

  .turn-flow-enter-active,
  .turn-flow-leave-active,
  .turn-flow-move {
    transition: none !important;
  }

  .turn-flow-enter-from,
  .turn-flow-leave-to {
    opacity: 1;
    transform: none;
  }
}

.conversation-disclosure {
  color: rgb(120 113 108);
}

.conversation-disclosure-summary {
  display: flex;
  align-items: center;
  justify-content: flex-start;
  gap: 0.5rem;
  cursor: pointer;
  list-style: none;
  font-size: 12px;
  line-height: 1.4;
  color: rgb(120 113 108);
}

.conversation-disclosure-summary::-webkit-details-marker {
  display: none;
}

.conversation-disclosure-chevron {
  transition: transform 180ms ease;
}

.conversation-disclosure[open] .conversation-disclosure-chevron {
  transform: rotate(180deg);
}

.assistant-reasoning {
  margin: 0;
  color: #8d857a;
  font-size: 0.82rem;
  line-height: 1;
  font-style: italic;
}

:deep(.assistant-reasoning-markdown pre),
:deep(.assistant-reasoning-markdown table),
:deep(.assistant-reasoning-markdown blockquote) {
  background: rgba(245, 240, 233, 0.72);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.65);
}

.composer-select-trigger {
  display: inline-flex;
  max-width: 12rem;
  min-height: 1.75rem;
  align-items: center;
  gap: 0.35rem;
  border: 1px solid rgba(214, 211, 209, 0.85);
  border-radius: 9999px;
  background: rgba(255, 255, 255, 0.7);
  padding: 0 0.7rem;
  font-size: 11px;
  font-weight: 500;
  line-height: 1;
  color: rgb(87 83 78);
  outline: none;
  transition:
    border-color 0.18s ease,
    background-color 0.18s ease,
    color 0.18s ease;
}

.composer-select-trigger:hover {
  border-color: rgba(168, 162, 158, 0.7);
  background: rgba(250, 250, 249, 0.96);
}

.composer-select-trigger:disabled {
  opacity: 0.45;
}

.composer-select-trigger:focus-visible {
  box-shadow: 0 0 0 2px rgba(231, 229, 228, 0.95);
}

.composer-menu-panel {
  border: 1px solid rgba(231, 229, 228, 0.95);
  border-radius: 0.7rem;
  background: rgba(255, 255, 255, 0.98);
  padding: 0.35rem 0;
  color: rgb(87 83 78);
  box-shadow: 0 12px 32px rgba(41, 37, 36, 0.08);
  backdrop-filter: blur(14px);
}

.composer-menu-caption {
  padding: 0 0.9rem 0.35rem;
  font-size: 10px;
  line-height: 1;
  color: rgb(168 162 158);
}

.composer-menu-divider {
  margin: 0 0.55rem 0.2rem;
  border-top: 1px solid rgba(231, 229, 228, 0.92);
}

.composer-menu-item {
  display: flex;
  width: 100%;
  align-items: center;
  justify-content: space-between;
  gap: 0.75rem;
  padding: 0.5rem 0.9rem;
  text-align: left;
  font-size: 12px;
  line-height: 1.2;
  color: rgb(87 83 78);
  transition: background-color 0.16s ease;
}

.composer-menu-item:hover {
  background: rgba(245, 245, 244, 0.9);
}

.composer-menu-item-hint {
  margin-top: 0.12rem;
  font-size: 10px;
  line-height: 1.2;
  color: rgb(168 162 158);
}

.checkpoint-icon-button {
  display: inline-flex;
  height: 1.35rem;
  width: 1.35rem;
  align-items: center;
  justify-content: center;
  border-radius: 0.25rem;
  border: none;
  background: transparent;
  color: rgb(168 162 158);
  cursor: pointer;
  transition:
    color 0.15s ease,
    background-color 0.15s ease,
    transform 0.1s ease;
}

.checkpoint-icon-button:hover {
  color: rgb(87 83 78);
  background: rgba(0, 0, 0, 0.04);
}

.checkpoint-icon-button:active {
  transform: scale(0.82);
  color: rgb(68 64 60);
}

.checkpoint-icon-button:disabled {
  cursor: not-allowed;
  opacity: 0.3;
}

.checkpoint-icon-button:disabled:hover {
  background: transparent;
  color: rgb(168 162 158);
}

.checkpoint-icon-button:disabled:active {
  transform: none;
}

/* Hover-reveal for action bars — opacity preserves layout */
.conversation-user-message .message-action-bar {
  opacity: 0;
  transition: opacity 0.2s ease;
}

.conversation-user-message:hover .message-action-bar {
  opacity: 1;
}

.conversation-agent-shell .agent-action-bar {
  opacity: 0;
  transition: opacity 0.2s ease;
}

.conversation-agent-shell:hover .agent-action-bar {
  opacity: 1;
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

.reasoning-italic {
  font-style: italic !important;
}

</style>
