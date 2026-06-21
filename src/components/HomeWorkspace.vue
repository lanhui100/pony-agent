<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, shallowReactive, watch } from "vue";
import { storeToRefs } from "pinia";
import {
  AlertTriangle,
  ArrowUp,
  Brain,
  Bot,
  Check,
  ChevronDown,
  GitBranch,
  GitFork,
  History,
  LoaderCircle,
  Copy,
  RotateCcw,
  Square,
  Undo2,
  UserRound,
  Wrench
} from "lucide-vue-next";
import type { ProviderConfig, ProviderReasoningEffort } from "@/types/provider";
import type { ChatMessage, ConversationCheckpointEntry, HistoryCheckoutMode, HistoryNode } from "@/types/runtime";
import { useProviderStore } from "@/stores/providers";
import { useRuntimeStore } from "@/stores/runtime";

import Button from "@/components/ui/Button.vue";
import MarkdownRenderer from "@/components/MarkdownRenderer.vue";
import ScrollArea from "@/components/ui/ScrollArea.vue";
import {
  PopoverClose,
  PopoverContent,
  PopoverPortal,
  PopoverRoot,
  PopoverTrigger
} from "reka-ui";

type TurnBucket = {
  turnId: string;
  user: ChatMessage | null;
  assistant: ChatMessage | null;
  tools: ChatMessage[];
};

type ComposerActionKind = "submit" | "resume" | "continue" | "restart";
type CheckpointRollbackAction = "transcript_only" | "transcript_and_workspace";

const runtimeStore = useRuntimeStore();
const providerStore = useProviderStore();
const runtimeStoreSessionList = storeToRefs(runtimeStore).sessionList;

const {
  activeBranchId,
  conversationCheckpointEntries,
  draftMessage,
  historyNodes,
  historyBranches,
  branchHeadNodeId,
  historyCursorMode,
  isSubmitting,
  latestExecutionCheckpoint,
  latestGraphRunSubmissionPlan,
  latestRunControlAuditSummary,
  messages,
  sessionOperation,
  visibleNodeId,
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
const rollbackInFlight = ref<{ turnId: string; action: CheckpointRollbackAction } | null>(null);
const rollbackProgressStyle = ref<Record<string, string | undefined>>({});
const optimisticRollbackTurnId = ref<string | null>(null);
const forkInFlightNodeId = ref<string | null>(null);
const providerMenuRef = ref<HTMLElement | null>(null);
const modelSubmenuStyle = ref<Record<string, string>>({ top: '0' });
const reasoningMenuRef = ref<HTMLElement | null>(null);
const checkpointPickerMenuRef = ref<HTMLElement | null>(null);
const forkSummaryMenuRef = ref<HTMLElement | null>(null);
const timelineScrollAreaRef = ref<{
  scrollToBottom: (behavior?: ScrollBehavior) => void;
  viewportEl: HTMLElement | null;
} | null>(null);
const timelineFollowAnchorRef = ref<HTMLElement | null>(null);
const scrollQueued = ref(false);
const scrollAfterPaintFrameId = ref<number | null>(null);
const stopRequested = ref(false);
const checkpointPickerOpen = ref(false);
const forkSummaryOpenForNodeId = ref<string | null>(null);
const SHOW_REASONING_STORAGE_KEY = "pony-agent.ui.show-reasoning-content";
const streamSnapshotTextByMessageId = shallowReactive<Record<string, string>>({});
const streamSnapshotReasoningByMessageId = shallowReactive<Record<string, string>>({});
const streamFadeTextByMessageId = shallowReactive<Record<string, string>>({});
const streamFadeKeyByMessageId = shallowReactive<Record<string, number>>({});
const streamReasoningFadeTextByMessageId = shallowReactive<Record<string, string>>({});
const streamReasoningFadeKeyByMessageId = shallowReactive<Record<string, number>>({});
const AUTO_SCROLL_THRESHOLD_PX = 160;
const STREAM_FADE_MIN_CHARS = 24;
const PROGRAMMATIC_SCROLL_GRACE_MS = 700;

let pendingStreamAutoFollow = true;
let pendingScrollBehavior: ScrollBehavior = "auto";
let scheduledScrollRequestId = 0;
let programmaticScrollUntilMs = 0;
const streamAutoFollowEnabled = ref(true);
let contentResizeObserver: ResizeObserver | null = null;

function updateStreamDebugReveal(_patch: Record<string, unknown>) {
}

function setupContentResizeObserver() {
  teardownContentResizeObserver();
  const contentColumn = workspaceContentColumnRef.value;
  if (!contentColumn || typeof ResizeObserver === "undefined") return;

  contentResizeObserver = new ResizeObserver(() => {
    if (!streamAutoFollowEnabled.value || scrollQueued.value) return;
    queueScrollToLatestTurn("smooth");
  });
  contentResizeObserver.observe(contentColumn);
}

function teardownContentResizeObserver() {
  if (contentResizeObserver) {
    contentResizeObserver.disconnect();
    contentResizeObserver = null;
  }
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
      tools: []
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
      tools: []
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
  if (cutoffIndex <= 0) {
    return [];
  }

  return turns.value.slice(0, cutoffIndex);
});

const hasVisibleHistorySession = computed(() =>
  runtimeStoreSessionList.value.some(
    (session) =>
      session.conversationId === runtimeStore.sessionId &&
      !isTransientSessionOverview(session)
  )
);
const isEmptyWorkspace = computed(() =>
  visibleTurns.value.length === 0 && !rollbackInFlight.value
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

const branchList = computed(() => {
  return historyBranches.value.map((branch, index) => ({
    ...branch,
    displayName: branch.label?.trim() || (index === 0 ? "主分支" : `分支 ${index}`)
  }));
});

const currentBranchDisplay = computed(() => {
  const current = branchList.value.find((b) => b.branchId === activeBranchId.value);
  return current?.displayName || (branchList.value.length > 0 ? branchList.value[0]!.displayName : "主分支");
});

const canUndoLastTurn = computed(() => {
  const nonLatest = checkpointEntries.value.filter(
    (entry) => !entry.isLatest && entry.branchId === activeBranchId.value
  );
  return (
    nonLatest.length > 0 &&
    !isSubmitting.value &&
    !sessionOperation.value &&
    !rollbackInFlight.value &&
    draftMessage.value.trim().length === 0
  );
});

const branchSwitcherDisabled = computed(
  () => branchList.value.length === 0 || isSubmitting.value || Boolean(sessionOperation.value) || Boolean(rollbackInFlight.value)
);

const undoShortcutLabel = computed(() => {
  if (typeof navigator !== "undefined" && navigator.platform.toLowerCase().includes("mac")) {
    return "Cmd+Z";
  }

  return "Ctrl+Z";
});

const latestTurnSignature = computed(() => {
  const latestMessage = messages.value[messages.value.length - 1] ?? null;
  const latestTurnId = latestMessage?.turnId ?? "";
  if (!latestTurnId) {
    return "";
  }

  const parts: string[] = [];
  for (let index = messages.value.length - 1; index >= 0; index -= 1) {
    const message = messages.value[index]!;
    if (message.turnId !== latestTurnId) {
      break;
    }
    parts.push(
      `${message.id}:${message.role}:${message.content.length}:${message.reasoningContent?.length ?? ""}:${message.tokenCount ?? ""}`
    );
  }

  return parts.reverse().join("|");
});

function formatAssistantModelLabel(modelName?: string | null) {
  return modelName?.trim() || "";
}

function formatTokenBadge(tokenCount?: number | null) {
  return `T:${tokenCount != null ? tokenCount : "--"}`;
}

function formatToolDuration(durationSeconds?: number | null) {
  if (durationSeconds == null) {
    return "--";
  }

  return `${Math.round(durationSeconds)}s`;
}

function assistantHeaderModel(message: ChatMessage | null) {
  if (!message) {
    return "";
  }

  return formatAssistantModelLabel(message.modelName);
}

function assistantReasoning(message: ChatMessage | null) {
  return message?.reasoningContent ?? "";
}

function isAssistantReasoningStreaming(message: ChatMessage | null) {
  return message?.status === "pending";
}

function isIncrementalStreamingAppend(previous: string, next: string) {
  return next.length > previous.length && next.startsWith(previous);
}

let lastCleanupAssistantSignature = "";
let lastSettledAssistantSignature = "";

function syncPresentationMapValue<T extends string | number>(
  map: Record<string, T>,
  messageId: string,
  nextValue: T
) {
  if (map[messageId] !== nextValue) {
    map[messageId] = nextValue;
  }
}

function clearStreamingPresentationState(activeMessageIds: Set<string>) {
  for (const map of [
    streamSnapshotTextByMessageId,
    streamSnapshotReasoningByMessageId,
    streamFadeTextByMessageId,
    streamFadeKeyByMessageId,
    streamReasoningFadeTextByMessageId,
    streamReasoningFadeKeyByMessageId,
  ]) {
    for (const messageId of activeMessageIds) {
      if (messageId in map) {
        delete map[messageId];
      }
    }
  }
}

function cleanupStreamingPresentationState(activeMessageIds: Set<string>, assistantSignature: string) {
  if (assistantSignature === lastCleanupAssistantSignature) {
    return;
  }
  lastCleanupAssistantSignature = assistantSignature;
  for (const map of [
    streamSnapshotTextByMessageId,
    streamSnapshotReasoningByMessageId,
    streamFadeTextByMessageId,
    streamFadeKeyByMessageId,
    streamReasoningFadeTextByMessageId,
    streamReasoningFadeKeyByMessageId,
  ]) {
    for (const messageId of Object.keys(map)) {
      if (!activeMessageIds.has(messageId)) {
        delete map[messageId];
      }
    }
  }
}

function syncStreamingPresentationState(source = "unknown") {
  const syncStartAt = typeof performance !== "undefined" ? performance.now() : Date.now();
  const activeMessageIds = new Set<string>();
  const activeAssistantSignatureParts: string[] = [];
  const pendingAssistants: ChatMessage[] = [];

  for (const message of messages.value) {
    if (message.role !== "assistant") {
      continue;
    }

    activeMessageIds.add(message.id);
    activeAssistantSignatureParts.push(message.id);
    if (message.status === "pending") {
      pendingAssistants.push(message);
    }
  }

  const activeAssistantSignature = activeAssistantSignatureParts.join("|");
  if (pendingAssistants.length === 0) {
    if (activeAssistantSignature !== lastSettledAssistantSignature) {
      clearStreamingPresentationState(activeMessageIds);
      lastSettledAssistantSignature = activeAssistantSignature;
    }
    cleanupStreamingPresentationState(activeMessageIds, activeAssistantSignature);

    const syncDurationMs = Math.round(((typeof performance !== "undefined" ? performance.now() : Date.now()) - syncStartAt) * 100) / 100;
    updateStreamDebugReveal({
      syncSource: source,
      syncDurationMs,
      pendingAssistantCount: 0,
      maxTextBacklog: 0,
      maxReasoningBacklog: 0,
      currentTextBacklog: 0,
      currentReasoningBacklog: 0,
      revealLoopActive: false,
      autoScrollQueued: scrollQueued.value
    });
    return;
  }

  lastSettledAssistantSignature = "";
  for (const message of pendingAssistants) {
    const nextReasoning = assistantReasoning(message);
    const nextText = message.content;
    const previousText = streamSnapshotTextByMessageId[message.id] ?? "";
    const previousReasoning = streamSnapshotReasoningByMessageId[message.id] ?? "";

    const appendedText = isIncrementalStreamingAppend(previousText, nextText)
      ? nextText.slice(previousText.length)
      : "";
    syncPresentationMapValue(
      streamFadeTextByMessageId,
      message.id,
      appendedText.length >= STREAM_FADE_MIN_CHARS ? appendedText : ""
    );
    if (!(message.id in streamFadeKeyByMessageId)) {
      streamFadeKeyByMessageId[message.id] = 0;
    }
    if (streamFadeTextByMessageId[message.id]) {
      streamFadeKeyByMessageId[message.id] = (streamFadeKeyByMessageId[message.id] ?? 0) + 1;
    }

    const appendedReasoning = isIncrementalStreamingAppend(previousReasoning, nextReasoning)
      ? nextReasoning.slice(previousReasoning.length)
      : "";
    syncPresentationMapValue(
      streamReasoningFadeTextByMessageId,
      message.id,
      appendedReasoning.length >= STREAM_FADE_MIN_CHARS ? appendedReasoning : ""
    );
    if (!(message.id in streamReasoningFadeKeyByMessageId)) {
      streamReasoningFadeKeyByMessageId[message.id] = 0;
    }
    if (streamReasoningFadeTextByMessageId[message.id]) {
      streamReasoningFadeKeyByMessageId[message.id] =
        (streamReasoningFadeKeyByMessageId[message.id] ?? 0) + 1;
    }

    syncPresentationMapValue(streamSnapshotTextByMessageId, message.id, nextText);
    syncPresentationMapValue(streamSnapshotReasoningByMessageId, message.id, nextReasoning);
  }

  cleanupStreamingPresentationState(activeMessageIds, activeAssistantSignature);

  const syncDurationMs = Math.round(((typeof performance !== "undefined" ? performance.now() : Date.now()) - syncStartAt) * 100) / 100;
  updateStreamDebugReveal({
    syncSource: source,
    syncDurationMs,
    pendingAssistantCount: pendingAssistants.length,
    maxTextBacklog: 0,
    maxReasoningBacklog: 0,
    currentTextBacklog: 0,
    currentReasoningBacklog: 0,
    revealLoopActive: false,
    autoScrollQueued: scrollQueued.value
  });
}

function assistantDisplayContent(message: ChatMessage | null) {
  if (!message) {
    return "";
  }

  return message.content;
}

function assistantDisplayStableContent(message: ChatMessage | null) {
  if (!message) {
    return "";
  }

  const displayText = assistantDisplayContent(message);
  const fadeText = streamFadeTextByMessageId[message.id] ?? "";
  return fadeText ? displayText.slice(0, Math.max(0, displayText.length - fadeText.length)) : displayText;
}

function assistantDisplayFadeContent(message: ChatMessage | null) {
  if (!message) {
    return "";
  }

  return streamFadeTextByMessageId[message.id] ?? "";
}

function assistantDisplayFadeStyle(message: ChatMessage | null) {
  if (!message) {
    return undefined;
  }

  const key = streamFadeKeyByMessageId[message.id] ?? 0;
  return {
    animationName: key % 2 === 0 ? "assistant-stream-fade-in-a" : "assistant-stream-fade-in-b"
  };
}

function assistantDisplayedReasoning(message: ChatMessage | null) {
  if (!message) {
    return "";
  }

  return assistantReasoning(message);
}

function assistantDisplayedReasoningStable(message: ChatMessage | null) {
  if (!message) {
    return "";
  }

  const displayText = assistantDisplayedReasoning(message);
  const fadeText = streamReasoningFadeTextByMessageId[message.id] ?? "";
  return fadeText ? displayText.slice(0, Math.max(0, displayText.length - fadeText.length)) : displayText;
}

function assistantDisplayedReasoningFade(message: ChatMessage | null) {
  if (!message) {
    return "";
  }

  return streamReasoningFadeTextByMessageId[message.id] ?? "";
}

function assistantDisplayedReasoningFadeStyle(message: ChatMessage | null) {
  if (!message) {
    return undefined;
  }

  const key = streamReasoningFadeKeyByMessageId[message.id] ?? 0;
  return {
    animationName: key % 2 === 0 ? "assistant-stream-fade-in-a" : "assistant-stream-fade-in-b"
  };
}

function shouldShowReasoningBlock(message: ChatMessage | null) {
  if (!message || !showReasoningContent.value) {
    return false;
  }

  return message.status === "pending" || assistantDisplayedReasoning(message).length > 0;
}

function shouldOpenReasoningBlock(_message: ChatMessage | null) {
  return false;
}

function reasoningPlaceholder(message: ChatMessage | null) {
  if (!message || message.status !== "pending") {
    return "";
  }

  return "正在思考...";
}

function assistantHasVisibleContent(message: ChatMessage | null) {
  if (!message) {
    return false;
  }

  const visibleContent = message.status === "pending" ? assistantDisplayContent(message) : message.content;
  return Boolean(visibleContent.trim());
}

function isAssistantStreaming(message: ChatMessage | null) {
  return message?.status === "pending";
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

  if (message.status === "pending") {
    return "text-stone-800";
  }

  if (message.status === "error") {
    return "text-rose-800";
  }

  return "text-stone-800";
}

function assistantErrorDetail(message: ChatMessage | null) {
  if (!message) {
    return "";
  }

  return message.errorDetail?.trim() || message.content.trim();
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

function toolStatusIcon(message: ChatMessage) {
  if (message.status === "error") {
    return "error";
  }

  if (message.status === "done") {
    return "done";
  }

  return "pending";
}

function checkpointEntryForTurn(turnId: string) {
  return checkpointEntryByTurnId.value.get(turnId) ?? null;
}

function requireCheckpointEntryForTurn(turnId: string) {
  const entry = checkpointEntryForTurn(turnId);
  if (!entry) {
    throw new Error(`Missing checkpoint entry for turn ${turnId}`);
  }
  return entry;
}

function checkpointModeForPickerEntry(entry: ConversationCheckpointEntry): HistoryCheckoutMode {
  return entry.workspaceRollbackCapable ? "transcript_and_workspace" : "transcript_only";
}

function resolveRollbackCheckoutNodeId(turnId: string, fallbackNodeId: string | null) {
  const entryNodeId = fallbackNodeId ?? checkpointEntryForTurn(turnId)?.nodeId ?? null;
  if (!entryNodeId) {
    return null;
  }

  const entryNode = historyNodeById.value.get(entryNodeId) ?? null;
  return entryNode?.parentNodeId?.trim() || entryNodeId;
}

function updateRollbackProgressPosition() {
  if (typeof window === "undefined") {
    rollbackProgressStyle.value = {};
    return;
  }

  const contentColumn = workspaceContentColumnRef.value;
  const viewport = timelineScrollAreaRef.value?.viewportEl ?? null;
  if (
    !rollbackInFlight.value ||
    !contentColumn ||
    typeof contentColumn.getBoundingClientRect !== "function"
  ) {
    rollbackProgressStyle.value = {};
    return;
  }

  const columnRect = contentColumn.getBoundingClientRect();
  const viewportRect = viewport && typeof viewport.getBoundingClientRect === "function"
    ? viewport.getBoundingClientRect()
    : null;
  const left = viewportRect ? Math.max(columnRect.left, viewportRect.left) : columnRect.left;
  const right = viewportRect ? Math.min(columnRect.right, viewportRect.right) : columnRect.right;
  const top = viewportRect ? Math.max(columnRect.top, viewportRect.top) : columnRect.top;
  const bottom = viewportRect ? Math.min(columnRect.bottom, viewportRect.bottom) : columnRect.bottom;
  const width = Math.max(0, right - left);
  const height = Math.max(0, bottom - top);

  if (import.meta.env.DEV) {
    console.log("[debug-rollback] progress overlay:", {
      columnRect: { left: columnRect.left, top: columnRect.top, right: columnRect.right, width: columnRect.width, height: columnRect.height },
      viewportRect: viewportRect ? { left: viewportRect.left, top: viewportRect.top, right: viewportRect.right, bottom: viewportRect.bottom } : null,
      result: { left, top, width, height }
    });
  }

  if (width < 20 || height < 20) {
    rollbackProgressStyle.value = {};
    if (import.meta.env.DEV) {
      console.log("[debug-rollback] progress overlay: zero-area fallback to full viewport");
    }
    return;
  }

  rollbackProgressStyle.value = {
    left: `${left}px`,
    top: `${top}px`,
    width: `${width}px`,
    height: `${height}px`,
    right: "auto",
    bottom: "auto"
  };
}

function updateFloatingUiPositions() {
  updateRollbackProgressPosition();
}

async function confirmRollback(turnId: string, action: CheckpointRollbackAction) {
  if (rollbackInFlight.value) return;

  const entry = checkpointEntryForTurn(turnId);
  const nodeId = resolveRollbackCheckoutNodeId(turnId, entry?.nodeId ?? null);

  if (!nodeId) {
    console.warn("[rollback] Cannot resolve parent nodeId for turn", turnId);
    return;
  }

  // Source turn message: the one the user clicked on
  const sourceTurn = turns.value.find(t => t.turnId === turnId);
  const userContent = sourceTurn?.user?.content ?? "";

  try {
    forkSummaryOpenForNodeId.value = null;
    checkpointPickerOpen.value = false;
    optimisticRollbackTurnId.value = turnId;
    rollbackInFlight.value = { turnId, action };
    updateRollbackProgressPosition();
    void nextTick().then(() => updateRollbackProgressPosition());

    const isSynthetic = nodeId.startsWith("synthetic-");
    const isFirstTurn =
      isSynthetic
        ? turns.value.findIndex((t) => t.turnId === turnId) <= 0
        : nodeId === entry?.nodeId;

    if (isFirstTurn) {
      runtimeStore.$patch({ messages: [], turnTraceHistory: [] });
      runtimeStore.persistHistory();
    } else if (isSynthetic) {
      const turnIndex = turns.value.findIndex((t) => t.turnId === turnId);
      const previousTurn = turnIndex > 0 ? turns.value[turnIndex - 1] : null;
      if (previousTurn) {
        let lastMsgIdx = -1;
        for (let idx = messages.value.length - 1; idx >= 0; idx--) {
          if (messages.value[idx]!.turnId === previousTurn.turnId) {
            lastMsgIdx = idx;
            break;
          }
        }
        if (lastMsgIdx >= 0) {
          runtimeStore.$patch({
            messages: messages.value.slice(0, lastMsgIdx + 1),
            turnTraceHistory: []
          });
          runtimeStore.persistHistory();
        }
      }
    } else {
      await Promise.all([
        runtimeStore.checkoutHistoryNode(nodeId, action, turnId),
        new Promise<void>(resolve => setTimeout(resolve, 350))
      ]);
    }
  } catch (err) {
    optimisticRollbackTurnId.value = null;
    rollbackInFlight.value = null;
    console.error("[rollback] checkoutHistoryNode failed:", err);
    return;
  }

  optimisticRollbackTurnId.value = null;
  rollbackInFlight.value = null;

  // Hydrate draft with the clicked turn's user message (the source of rollback)
  const nextDraft = userContent;
  draftMessage.value = nextDraft;
  runtimeStore.setDraftMessage(nextDraft);
}

function rollbackProgressLabel() {
  if (!rollbackInFlight.value) {
    return "";
  }

  return rollbackInFlight.value.action === "transcript_and_workspace"
    ? "正在撤回对话和文件..."
    : "正在撤回对话...";
}

function toggleCheckpointPicker() {
  checkpointPickerOpen.value = !checkpointPickerOpen.value;
  if (checkpointPickerOpen.value) {
    providerMenuOpen.value = false;
    hoveredProviderId.value = null;
    reasoningMenuOpen.value = false;
    forkSummaryOpenForNodeId.value = null;
  }
}

function toggleForkSummary(nodeId: string) {
  forkSummaryOpenForNodeId.value = forkSummaryOpenForNodeId.value === nodeId ? null : nodeId;
  checkpointPickerOpen.value = false;
}

async function jumpToForkTarget(
  entry: ConversationCheckpointEntry,
  target: ConversationCheckpointEntry["forkTargets"][number]
) {
  forkSummaryOpenForNodeId.value = null;

  if (target.branchId && !target.isActive) {
    const switched = await runtimeStore.switchHistoryBranch(target.branchId);
    if (switched?.visibleNodeId === target.nodeId || switched?.branchHeadNodeId === target.nodeId) {
      return;
    }
  }

  await runtimeStore.checkoutHistoryNode(target.nodeId, checkpointModeForPickerEntry(entry));
}

function branchLabelForDisplay(branch: { label?: string | null; branchId: string }, index: number) {
  return branch.label?.trim() || (index === 0 ? "主分支" : `分支 ${index}`);
}

async function handleCreateBranch(turnId: string) {
  if (isSubmitting.value || sessionOperation.value || rollbackInFlight.value || forkInFlightNodeId.value) {
    return;
  }

  const entry = checkpointEntryForTurn(turnId);
  if (!entry) {
    return;
  }

  forkInFlightNodeId.value = entry.nodeId;
  try {
    await runtimeStore.forkHistoryNode(entry.nodeId);
  } finally {
    forkInFlightNodeId.value = null;
  }
}

async function handleUndoLastTurn() {
  if (!canUndoLastTurn.value || rollbackInFlight.value) {
    return;
  }

  const sortedEntries = [...checkpointEntries.value]
    .filter((e) => !e.isLatest && e.branchId === activeBranchId.value)
    .sort((a, b) => b.createdAtMs - a.createdAtMs);
  const previousEntry = sortedEntries[0];
  if (!previousEntry) {
    return;
  }

  forkSummaryOpenForNodeId.value = null;
  checkpointPickerOpen.value = false;
  await runtimeStore.checkoutHistoryNode(previousEntry.nodeId, "transcript_only", previousEntry.turnId);
}

async function handleSwitchBranch(branchId: string) {
  if (isSubmitting.value || sessionOperation.value || rollbackInFlight.value) {
    return;
  }

  if (branchId === activeBranchId.value) {
    if (
      historyCursorMode.value !== "live" &&
      visibleNodeId.value &&
      branchHeadNodeId.value &&
      visibleNodeId.value !== branchHeadNodeId.value
    ) {
      checkpointPickerOpen.value = false;
      await runtimeStore.restoreBranchHead(branchId);
    }
    return;
  }

  checkpointPickerOpen.value = false;
  await runtimeStore.switchHistoryBranch(branchId);
}

function handleCheckpointShortcut(event: KeyboardEvent) {
  const isMac = typeof navigator !== "undefined" && navigator.platform.toLowerCase().includes("mac");
  const shortcutPressed = event.key.toLowerCase() === "k" && (isMac ? event.metaKey : event.ctrlKey);

  if (shortcutPressed && !event.shiftKey && !event.altKey) {
    event.preventDefault();
    if (branchList.value.length > 0) {
      checkpointPickerOpen.value = true;
      providerMenuOpen.value = false;
      hoveredProviderId.value = null;
      reasoningMenuOpen.value = false;
      forkSummaryOpenForNodeId.value = null;
    }
    return true;
  }

  return false;
}

function handleComposerKeydown(event: KeyboardEvent) {
  if (handleCheckpointShortcut(event)) {
    return;
  }

  const isUndoShortcut =
    event.key.toLowerCase() === "z" &&
    (event.ctrlKey || event.metaKey) &&
    !event.shiftKey &&
    !event.altKey;
  if (isUndoShortcut && draftMessage.value.trim() === "") {
    event.preventDefault();
    handleUndoLastTurn();
    return;
  }

  if (event.key === "Enter" && !event.shiftKey) {
    event.preventDefault();
    if (!isSubmitting.value) {
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

  handleCheckpointShortcut(event);
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

function selectModel(providerId: string, modelId: string) {
  providerStore.selectModel(providerId, modelId);
  providerMenuOpen.value = false;
  hoveredProviderId.value = providerId;
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

  if (checkpointPickerMenuRef.value && target && !checkpointPickerMenuRef.value.contains(target)) {
    checkpointPickerOpen.value = false;
  }

  if (forkSummaryMenuRef.value && target && !forkSummaryMenuRef.value.contains(target)) {
    forkSummaryOpenForNodeId.value = null;
  }
}

function isTimelineNearBottom() {
  const viewport = timelineScrollAreaRef.value?.viewportEl ?? null;
  if (!viewport) {
    return true;
  }

  const distanceToBottom = viewport.scrollHeight - viewport.scrollTop - viewport.clientHeight;
  return distanceToBottom <= AUTO_SCROLL_THRESHOLD_PX;
}

function queueScrollToLatestTurn(behavior: ScrollBehavior = "smooth") {
  pendingScrollBehavior = pendingScrollBehavior === "smooth" || behavior === "smooth" ? "smooth" : behavior;
  scheduledScrollRequestId += 1;
  const requestId = scheduledScrollRequestId;
  scrollQueued.value = true;
  programmaticScrollUntilMs = Date.now() + PROGRAMMATIC_SCROLL_GRACE_MS;
  void nextTick().then(() => {
    if (requestId !== scheduledScrollRequestId) {
      return;
    }
    if (scrollAfterPaintFrameId.value != null) {
      window.cancelAnimationFrame(scrollAfterPaintFrameId.value);
    }

    scrollAfterPaintFrameId.value = window.requestAnimationFrame(() => {
      if (requestId !== scheduledScrollRequestId) {
        return;
      }
      scrollAfterPaintFrameId.value = null;
      scrollQueued.value = false;
      const scrollArea = timelineScrollAreaRef.value;
      const followAnchor = timelineFollowAnchorRef.value;
      if (followAnchor) {
        followAnchor.scrollIntoView({
          block: "end",
          behavior: pendingScrollBehavior
        });
      } else {
        const viewport = scrollArea?.viewportEl ?? null;
        if (viewport) {
          viewport.scrollTo({
            top: viewport.scrollHeight,
            behavior: pendingScrollBehavior
          });
        } else if (scrollArea && typeof scrollArea.scrollToBottom === "function") {
          scrollArea.scrollToBottom(pendingScrollBehavior);
        }
      }

      updateStreamDebugReveal({
        scrolledToComposerEdge: true,
        scrollBehavior: pendingScrollBehavior,
        nearBottom: isTimelineNearBottom()
      });
      programmaticScrollUntilMs = Date.now() + PROGRAMMATIC_SCROLL_GRACE_MS;
      pendingScrollBehavior = "auto";
    });
  });
}

function handleTimelineViewportScroll() {
  updateFloatingUiPositions();
  if (Date.now() < programmaticScrollUntilMs) {
    streamAutoFollowEnabled.value = true;
    return;
  }

  streamAutoFollowEnabled.value = isTimelineNearBottom();
}

function handleTimelineUserScrollIntent() {
  programmaticScrollUntilMs = 0;
  streamAutoFollowEnabled.value = isTimelineNearBottom();
  updateFloatingUiPositions();
}

onMounted(() => {
  lastCleanupAssistantSignature = "";
  lastSettledAssistantSignature = "";
  if (typeof window !== "undefined") {
    showReasoningContent.value = window.localStorage.getItem(SHOW_REASONING_STORAGE_KEY) === "true";
  }
  syncStreamingPresentationState("mounted");
  window.addEventListener("click", handleClickOutside);
  window.addEventListener("keydown", handleWindowKeydown);
  window.addEventListener("resize", updateFloatingUiPositions);
  handleTimelineViewportScroll();
  setupContentResizeObserver();
  queueScrollToLatestTurn("auto");
});

onBeforeUnmount(() => {
  window.removeEventListener("click", handleClickOutside);
  window.removeEventListener("keydown", handleWindowKeydown);
  window.removeEventListener("resize", updateFloatingUiPositions);
  const viewport = timelineScrollAreaRef.value?.viewportEl ?? null;
  if (viewport && "removeEventListener" in viewport) {
    viewport.removeEventListener("scroll", handleTimelineViewportScroll);
    viewport.removeEventListener("wheel", handleTimelineUserScrollIntent);
    viewport.removeEventListener("touchstart", handleTimelineUserScrollIntent);
  }
  if (scrollAfterPaintFrameId.value != null) {
    window.cancelAnimationFrame(scrollAfterPaintFrameId.value);
  }

  teardownContentResizeObserver();
});

watch(
  () => timelineScrollAreaRef.value?.viewportEl ?? null,
  (viewport, previousViewport) => {
    if (previousViewport && "removeEventListener" in previousViewport) {
      previousViewport.removeEventListener("scroll", handleTimelineViewportScroll);
      previousViewport.removeEventListener("wheel", handleTimelineUserScrollIntent);
      previousViewport.removeEventListener("touchstart", handleTimelineUserScrollIntent);
    }
    if (viewport && "addEventListener" in viewport) {
      viewport.addEventListener("scroll", handleTimelineViewportScroll, { passive: true });
      viewport.addEventListener("wheel", handleTimelineUserScrollIntent, { passive: true });
      viewport.addEventListener("touchstart", handleTimelineUserScrollIntent, { passive: true });
    }
    handleTimelineViewportScroll();
  },
  { flush: "post" }
);

watch(rollbackInFlight, () => {
  void nextTick().then(() => updateRollbackProgressPosition());
}, { flush: "post" });

watch(latestTurnSignature, (signature, previousSignature) => {
  if (signature === previousSignature) {
    return;
  }

  pendingStreamAutoFollow = streamAutoFollowEnabled.value || isTimelineNearBottom();
  void nextTick().then(() => {
    syncStreamingPresentationState("latest-turn-signature:post");
    const behavior: ScrollBehavior = pendingStreamAutoFollow ? "smooth" : "auto";
    streamAutoFollowEnabled.value = pendingStreamAutoFollow;

    if (streamAutoFollowEnabled.value) {
      queueScrollToLatestTurn(behavior);
    }
  });
}, { flush: "pre" });

watch(showReasoningContent, (value) => {
  if (typeof window !== "undefined") {
    window.localStorage.setItem(SHOW_REASONING_STORAGE_KEY, value ? "true" : "false");
  }
});

watch(isSubmitting, (submitting) => {
  syncStreamingPresentationState("is-submitting");
  if (!submitting) {
    stopRequested.value = false;
  }
});
</script>

<template>
  <section class="flex h-full min-h-0 min-w-0 flex-col overflow-hidden rounded-t-[0.6rem]">
    <ScrollArea
      ref="timelineScrollAreaRef"
      class="min-h-0 flex-1 rounded-t-[0.6rem]"
      viewport-class="px-4 py-4 sm:px-5"
    >
      <div ref="workspaceContentColumnRef" class="mx-auto w-full max-w-[58rem]" data-testid="workspace-content-column">
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
      <TransitionGroup name="turn-flow" tag="div" class="relative space-y-5">
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
        <section v-for="turn in visibleTurns" :key="turn.turnId" class="space-y-3">
          <article v-if="turn.user" v-motion :initial="{ opacity: 0, y: 8 }" :animate="{ opacity: 1, y: 0 }" :transition="{ duration: 0.22, ease: 'easeOut' }" class="conversation-user-message ml-auto w-fit max-w-[86%] sm:max-w-[68%]">
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

          <article v-if="turn.assistant || turn.tools.length" v-motion :initial="{ opacity: 0, y: 8 }" :animate="{ opacity: 1, y: 0 }" :transition="{ duration: 0.22, ease: 'easeOut' }" class="conversation-agent-shell w-full px-0 py-1">
            <div v-motion :initial="{ opacity: 0, y: 6 }" :animate="{ opacity: 1, y: 0 }" :transition="{ duration: 0.2, ease: 'easeOut', delay: 0.02 }" class="conversation-agent-header flex items-center justify-between gap-3">
              <div :class="actorLabelClass()" class="min-w-0">
                <Bot class="h-3.5 w-3.5" />
                <span>Agent</span>
              </div>
              <div class="flex items-center gap-2 text-right normal-case tracking-normal">
                <span v-if="assistantHeaderModel(turn.assistant)" class="truncate">
                  <span class="inline-flex rounded-full border border-[#e6d7c3] bg-[#f6efe3] px-2 py-0.5 text-[10px] text-[#8b6b47]">
                    {{ assistantHeaderModel(turn.assistant) }}
                  </span>
                </span>
              </div>
            </div>
            <div class="mt-2 h-px w-full bg-stone-200/70"></div>

            <details v-if="turn.tools.length" v-motion :initial="{ opacity: 0, y: 6 }" :animate="{ opacity: 1, y: 0 }" :transition="{ duration: 0.2, ease: 'easeOut', delay: 0.04 }" class="conversation-disclosure conversation-tool-panel mt-2 group">
              <summary class="conversation-disclosure-summary">
                <Wrench class="h-3.5 w-3.5 shrink-0 text-stone-400" />
                <span>工具调用</span>
                <span class="inline-flex items-center rounded-full border border-stone-200/80 px-2 py-0.5 text-[10px] normal-case tracking-normal text-stone-500">{{ turn.tools.length }} 项</span>
                <ChevronDown class="conversation-disclosure-chevron h-3.5 w-3.5 shrink-0 text-stone-400" />
              </summary>
              <div class="conversation-tool-list mt-1 space-y-0.5">
                <div
                  v-for="(tool, idx) in turn.tools"
                  :key="tool.id"
                  v-motion
                  :initial="{ opacity: 0, y: 4 }"
                  :animate="{ opacity: 1, y: 0 }"
                  :transition="{ duration: 0.18, ease: 'easeOut', delay: 0.04 + idx * 0.025 }"
                  class="conversation-tool-row flex items-center justify-between gap-3 px-1 py-0.5 text-[12px] leading-5 text-stone-500"
                >
                  <div class="flex min-w-0 items-center gap-2">
                    <span class="truncate">{{ tool.toolName || "Tool" }}</span>
                    <LoaderCircle
                      v-if="toolStatusIcon(tool) === 'pending'"
                      class="h-3.5 w-3.5 shrink-0 animate-spin text-stone-400"
                    />
                    <Check v-else-if="toolStatusIcon(tool) === 'done'" class="h-3.5 w-3.5 shrink-0 text-stone-500" />
                    <span v-else class="text-[13px] leading-none text-rose-500">!</span>
                  </div>
                  <div class="flex shrink-0 items-center gap-2 text-[11px] text-stone-400">
                    <span
                      v-if="tool.tokenCount != null"
                      class="inline-flex rounded-full border border-stone-200/80 px-2 py-0.5 text-[10px] normal-case tracking-normal text-stone-500"
                    >
                      {{ formatTokenBadge(tool.tokenCount) }}
                    </span>
                    <span v-if="tool.durationSeconds != null">{{ formatToolDuration(tool.durationSeconds) }}</span>
                  </div>
                </div>
              </div>
            </details>

            <details
              v-if="turn.assistant && shouldShowReasoningBlock(turn.assistant)"
              :open="shouldOpenReasoningBlock(turn.assistant)"
              v-motion
              :initial="{ opacity: 0, y: 6 }"
              :animate="{ opacity: 1, y: 0 }"
              :transition="{ duration: 0.2, ease: 'easeOut', delay: 0.04 }"
              class="conversation-disclosure conversation-reasoning-panel mt-2 group"
            >
              <summary class="conversation-disclosure-summary">
                <div class="flex min-w-0 items-center gap-2">
                  <Brain class="h-3.5 w-3.5 shrink-0 text-stone-400" />
                  <span>思考过程</span>
                </div>
                <ChevronDown class="conversation-disclosure-chevron h-3.5 w-3.5 shrink-0 text-stone-400" />
              </summary>
              <div class="mt-2">
                <template v-if="assistantReasoning(turn.assistant)">
                  <MarkdownRenderer
                    :content="isAssistantReasoningStreaming(turn.assistant) ? assistantDisplayedReasoningStable(turn.assistant) : assistantReasoning(turn.assistant)"
                    wrapper-class="assistant-markdown assistant-reasoning-markdown text-[13px]"
                    :streaming="isAssistantReasoningStreaming(turn.assistant)"
                  />
                  <span
                    v-if="isAssistantReasoningStreaming(turn.assistant) && assistantDisplayedReasoningFade(turn.assistant)"
                    class="assistant-streaming-fade"
                    :style="assistantDisplayedReasoningFadeStyle(turn.assistant)"
                  >
                    {{ assistantDisplayedReasoningFade(turn.assistant) }}
                  </span>
                </template>
                <p
                  v-else-if="reasoningPlaceholder(turn.assistant)"
                  class="assistant-reasoning"
                >
                  {{ reasoningPlaceholder(turn.assistant) }}
                </p>
              </div>
            </details>
            <div
              v-if="turn.assistant && assistantHasVisibleContent(turn.assistant)"
              v-motion
              :initial="{ opacity: 0, y: 6 }"
              :animate="{ opacity: 1, y: 0 }"
              :transition="{ duration: 0.22, ease: 'easeOut', delay: 0.04 }"
              class="assistant-response-panel mt-4"
            >
              <MarkdownRenderer
                :content="isAssistantStreaming(turn.assistant) ? assistantDisplayStableContent(turn.assistant) : turn.assistant.content"
                wrapper-class="assistant-markdown text-sm"
                :tone-class="assistantTone(turn.assistant)"
                :streaming="isAssistantStreaming(turn.assistant)"
              />
              <span
                v-if="isAssistantStreaming(turn.assistant) && assistantDisplayFadeContent(turn.assistant)"
                class="assistant-streaming-fade"
                :style="assistantDisplayFadeStyle(turn.assistant)"
              >
                {{ assistantDisplayFadeContent(turn.assistant) }}
              </span>
            </div>
            <details
              v-if="turn.assistant?.status === 'error' && assistantErrorDetail(turn.assistant)"
              class="conversation-disclosure conversation-error-panel mt-3 group"
            >
              <summary class="conversation-disclosure-summary text-rose-700">
                <div class="flex min-w-0 items-center gap-2">
                  <AlertTriangle class="h-3.5 w-3.5 shrink-0 text-rose-500" />
                  <span>错误详情</span>
                </div>
                <div class="ml-auto flex items-center gap-1">
                  <button
                    class="invisible group-hover:visible inline-flex h-5 w-5 items-center justify-center rounded-[0.35rem] text-stone-400 transition hover:bg-rose-50 hover:text-rose-600"
                    type="button"
                    :data-testid="`workspace-error-copy-${turn.turnId}`"
                    @click.stop="copyErrorDetail(turn.turnId, assistantErrorDetail(turn.assistant))"
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
                {{ assistantErrorDetail(turn.assistant) }}
              </div>
            </details>

            <div
              v-if="turn.assistant"
              class="agent-action-bar ml-auto mt-3 flex flex-wrap items-center justify-end gap-2"
              data-testid="workspace-agent-branch-actions"
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

                <button
                  v-if="!isLastTurn(turn.turnId)"
                  class="checkpoint-icon-button"
                  type="button"
                  :disabled="isSubmitting || !!sessionOperation || !!rollbackInFlight || !!forkInFlightNodeId"
                  title="创建分支"
                  @click="handleCreateBranch(turn.turnId)"
                >
                <GitBranch class="h-3.5 w-3.5" />
                <span class="sr-only">创建分支</span>
              </button>

              <div
                v-if="!isLastTurn(turn.turnId) && checkpointEntryForTurn(turn.turnId)?.forkTargets.length"
                ref="forkSummaryMenuRef"
                class="relative"
              >
                <button
                  class="checkpoint-icon-button"
                  type="button"
                  title="查看和切换分支"
                  @click="toggleForkSummary(requireCheckpointEntryForTurn(turn.turnId).nodeId)"
                >
                  <GitFork class="h-3.5 w-3.5" />
                  <span class="sr-only">查看分支</span>
                </button>

                <div
                  v-if="forkSummaryOpenForNodeId === checkpointEntryForTurn(turn.turnId)?.nodeId"
                  class="checkpoint-popover absolute left-0 top-[calc(100%+0.45rem)] z-20 w-[19rem] max-w-[calc(100vw-2rem)]"
                >
                  <div class="checkpoint-popover-caption">分支</div>
                  <div class="checkpoint-popover-divider"></div>
                  <button
                    v-for="(target, targetIdx) in checkpointEntryForTurn(turn.turnId)?.forkTargets ?? []"
                    :key="`${checkpointEntryForTurn(turn.turnId)?.nodeId}-${target.branchId}-${target.nodeId}`"
                    class="checkpoint-picker-item"
                    type="button"
                    @click="jumpToForkTarget(requireCheckpointEntryForTurn(turn.turnId), target)"
                  >
                    <div class="flex min-w-0 flex-1 flex-col text-left">
                      <span class="truncate text-[12px] text-stone-800">{{ branchLabelForDisplay({ label: target.label, branchId: target.branchId }, targetIdx + 1) }}</span>
                      <span class="mt-0.5 line-clamp-2 text-[10px] leading-4 text-stone-500">
                        {{ target.summary }}
                      </span>
                    </div>
                    <span
                      class="shrink-0 rounded-full border border-stone-200/80 px-2 py-0.5 text-[10px] text-stone-500"
                    >
                      {{ target.isActive ? "当前" : "转到" }}
                    </span>
                  </button>
                </div>
              </div>
            </div>
          </article>
        </section>
      </TransitionGroup>
      <div ref="timelineFollowAnchorRef" class="h-px w-full" aria-hidden="true"></div>
      </div>
    </ScrollArea>

    <div class="px-4 py-3 sm:px-5">
      <div
        class="mx-auto w-full max-w-[58rem] rounded-[0.6rem] bg-white/76 px-4 py-3"
        data-testid="workspace-composer-shell"
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
          <div ref="checkpointPickerMenuRef" class="relative">
            <button
              class="composer-select-trigger"
              type="button"
              :disabled="branchSwitcherDisabled"
              :title="
                branchList.length
                  ? '切换对话分支'
                  : '当前还没有分支'
              "
              data-testid="workspace-branch-switcher-trigger"
              @click.stop="toggleCheckpointPicker"
            >
              <GitBranch class="h-3.5 w-3.5 text-stone-500" />
              <span class="truncate">{{ currentBranchDisplay }}</span>
              <ChevronDown class="h-2.5 w-2.5 text-stone-400" />
            </button>

            <div
              v-if="checkpointPickerOpen"
              class="composer-menu-panel absolute bottom-[calc(100%+0.45rem)] left-0 z-20 min-w-[18rem] max-w-[min(28rem,calc(100vw-2rem))]"
              data-testid="workspace-branch-switcher-menu"
            >
              <div class="composer-menu-caption">分支</div>
              <div class="composer-menu-divider"></div>
              <button
                v-for="branch in branchList"
                :key="`branch-switcher-${branch.branchId}`"
                class="checkpoint-picker-item"
                type="button"
                :data-testid="`workspace-branch-switcher-item-${branch.branchId}`"
                @click="handleSwitchBranch(branch.branchId)"
              >
                <div class="flex min-w-0 flex-1 flex-col text-left">
                  <span class="truncate text-[12px] text-stone-800">{{ branch.displayName }}</span>
                  <span class="mt-0.5 text-[10px] leading-4 text-stone-500">
                    {{ branch.branchId }}
                  </span>
                </div>
                <span
                  class="shrink-0 rounded-full border border-stone-200/80 px-2 py-0.5 text-[10px] text-stone-500"
                >
                  {{ branch.branchId === activeBranchId ? "当前" : "切换" }}
                </span>
              </button>
            </div>
          </div>

            <button
              class="composer-select-trigger"
              type="button"
              :disabled="!canUndoLastTurn"
              :title="canUndoLastTurn ? `撤回上一轮（${undoShortcutLabel}）` : (draftMessage.trim().length > 0 ? '请先处理当前草稿' : '没有可撤回的操作')"
              data-testid="workspace-undo-button"
              @click.stop="handleUndoLastTurn"
            >
            <Undo2 class="h-3.5 w-3.5 text-stone-500" />
            <span class="truncate">撤回</span>
          </button>

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
  line-height: 1.2;
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
  margin: 1.15rem 0 0.65rem;
  font-size: inherit;
  line-height: 1.2;
  letter-spacing: -0.015em;
  color: #241b14;
}

:deep(.assistant-markdown h1),
:deep(.assistant-markdown h2) {
  font-weight: 520;
}

:deep(.assistant-markdown h3),
:deep(.assistant-markdown h4),
:deep(.assistant-markdown h5),
:deep(.assistant-markdown h6) {
  font-weight: 380;
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
  margin: 1rem 0;
  overflow-x: auto;
  border-radius: 0.9rem;
  background: transparent;
  padding: 0;
  font-size: 0.82rem;
  line-height: 1.55;
  color: #2f261d;
}

:deep(.assistant-markdown code) {
  border-radius: 0.42rem;
  background: #f6efe3;
  padding: 0.08rem 0.34rem;
  font-size: 0.82em;
  color: #5b4330;
}

:deep(.assistant-markdown pre code) {
  background: #fefcf6;
  padding: 1rem 1.05rem;
  border-radius: 0.9rem;
  display: block;
  color: #2f261d;
  font-size: 1em;
  white-space: pre-wrap;
  word-break: break-word;
  overflow-wrap: anywhere;
}

:deep(.assistant-markdown .table-scroll-wrapper) {
  overflow-x: auto;
  margin: 1rem 0;
}

:deep(.assistant-markdown .table-scroll-wrapper table) {
  width: 100%;
  table-layout: auto;
  border-collapse: separate;
  border-spacing: 0;
  background: rgba(255, 251, 244, 0.92);
  border-radius: 0.9rem;
}

:deep(.assistant-markdown .table-scroll-wrapper thead tr:first-child th:first-child) {
  border-top-left-radius: 0.9rem;
}

:deep(.assistant-markdown .table-scroll-wrapper thead tr:first-child th:last-child) {
  border-top-right-radius: 0.9rem;
}

:deep(.assistant-markdown .table-scroll-wrapper tbody tr:last-child td:first-child) {
  border-bottom-left-radius: 0.9rem;
}

:deep(.assistant-markdown .table-scroll-wrapper tbody tr:last-child td:last-child) {
  border-bottom-right-radius: 0.9rem;
}

:deep(.assistant-markdown .table-scroll-wrapper thead th) {
  background: rgba(244, 234, 221, 0.88);
  font-weight: 380;
  color: #56463a;
}


:deep(.assistant-markdown .table-scroll-wrapper th),
:deep(.assistant-markdown .table-scroll-wrapper td) {
  padding: 0.62rem 0.8rem;
  vertical-align: top;
  text-align: left;
  white-space: normal;
  word-break: break-word;
  overflow-wrap: anywhere;
  max-width: 320px;
}

:deep(.assistant-markdown .table-scroll-wrapper tbody tr:nth-child(even)) {
  background: rgba(246, 240, 231, 0.7);
}

:deep(.assistant-markdown a) {
  color: #8b5e34;
  text-decoration: underline;
  text-underline-offset: 2px;
}

:deep(.assistant-markdown blockquote) {
  margin: 1rem 0;
  padding: 0.8rem 1rem;
  color: #71665c;
  background: linear-gradient(135deg, rgba(245, 239, 231, 0.92), rgba(250, 246, 240, 0.86));
  border-radius: 0.95rem;
  font-style: italic;
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

.turn-flow-enter-active,
.turn-flow-leave-active {
  transition:
    opacity 280ms ease,
    transform 280ms ease;
}

.turn-flow-leave-active {
  position: absolute;
  left: 0;
  right: 0;
}

.turn-flow-enter-from,
.turn-flow-leave-to {
  opacity: 0;
  transform: translateY(0.45rem);
}

.turn-flow-move {
  transition: transform 260ms ease;
}

.conversation-user-message,
.conversation-agent-shell,
.conversation-agent-header,
.conversation-tool-panel,
.conversation-tool-row,
.conversation-reasoning-panel,
.assistant-response-panel {
  will-change: opacity, transform;
}

.streaming-unrendered-suffix {
  word-break: break-word;
  overflow-wrap: anywhere;
  line-height: 1.2;
  color: #3d342d;
}

.assistant-streaming-content {
  white-space: pre-wrap;
  word-break: break-word;
  overflow-wrap: anywhere;
  line-height: 1.2;
  transition: color 140ms ease;
}

.assistant-streaming-fade {
  display: inline;
  animation-duration: 180ms;
  animation-timing-function: ease-out;
  animation-fill-mode: both;
}

@keyframes assistant-stream-fade-in-a {
  from {
    opacity: 0.16;
    filter: blur(0.14rem);
  }

  to {
    opacity: 1;
    filter: blur(0);
  }
}

@keyframes assistant-stream-fade-in-b {
  from {
    opacity: 0.16;
    filter: blur(0.14rem);
  }

  to {
    opacity: 1;
    filter: blur(0);
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

:deep(.assistant-reasoning-markdown) {
  color: #8d857a;
  font-style: italic;
  line-height: 1;
}

:deep(.assistant-reasoning-markdown p) {
  line-height: 1;
  margin: 0 0 1em;
}

:deep(.assistant-reasoning-markdown h1),
:deep(.assistant-reasoning-markdown h2) {
  font-size: inherit;
  font-weight: 520;
  line-height: 1;
  color: #746d64;
}

:deep(.assistant-reasoning-markdown h3),
:deep(.assistant-reasoning-markdown h4),
:deep(.assistant-reasoning-markdown h5),
:deep(.assistant-reasoning-markdown h6) {
  font-size: inherit;
  font-weight: 380;
  line-height: 1;
  color: #746d64;
}

:deep(.assistant-reasoning-markdown strong) {
  color: #746d64;
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

.checkpoint-popover {
  border: 1px solid rgba(231, 229, 228, 0.95);
  border-radius: 0.8rem;
  background: rgba(255, 255, 255, 0.98);
  box-shadow: 0 16px 36px rgba(41, 37, 36, 0.12);
  backdrop-filter: blur(14px);
}

.checkpoint-popover-caption {
  padding: 0.75rem 0.9rem 0.45rem;
  font-size: 10px;
  line-height: 1;
  color: rgb(168 162 158);
}

.checkpoint-popover-divider {
  margin: 0 0.65rem;
  border-top: 1px solid rgba(231, 229, 228, 0.92);
}

.checkpoint-picker-item {
  display: flex;
  width: 100%;
  align-items: center;
  gap: 0.75rem;
  padding: 0.65rem 0.9rem;
  transition: background-color 0.16s ease;
}

.checkpoint-picker-item:hover {
  background: rgba(245, 245, 244, 0.9);
}

.rollback-progress-overlay {
  position: fixed;
  inset: 0;
  z-index: 30;
  pointer-events: none;
  display: flex;
  align-items: center;
  justify-content: center;
}

.rollback-progress-card {
  display: inline-flex;
  align-items: center;
  gap: 1rem;
  border: 1px solid rgba(231, 229, 228, 0.72);
  border-radius: 1rem;
  background: rgba(255, 255, 255, 0.92);
  color: rgb(87 83 78);
  box-shadow: 0 6px 18px rgba(41, 37, 36, 0.08);
  padding: 1.15rem 1.5rem;
  font-size: 22px;
  font-weight: 500;
  line-height: 1;
  backdrop-filter: blur(8px);
}

.rollback-progress-icon {
  width: 1.6rem;
  height: 1.6rem;
  color: rgb(180 138 112);
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
