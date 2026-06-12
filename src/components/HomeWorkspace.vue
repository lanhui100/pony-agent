<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, reactive, ref, watch } from "vue";
import { storeToRefs } from "pinia";
import {
  ArrowUp,
  Brain,
  Bot,
  Check,
  ChevronDown,
  GitFork,
  History,
  LoaderCircle,
  RotateCcw,
  Square,
  UserRound,
  Wrench
} from "lucide-vue-next";
import type { ProviderReasoningEffort } from "@/types/provider";
import type { ChatMessage, ConversationCheckpointEntry, HistoryCheckoutMode } from "@/types/runtime";
import { useProviderStore } from "@/stores/providers";
import { useRuntimeStore } from "@/stores/runtime";
import { isTauriAvailable, safeInvoke } from "@/lib/tauri";
import Button from "@/components/ui/Button.vue";
import MarkdownRenderer from "@/components/MarkdownRenderer.vue";
import ScrollArea from "@/components/ui/ScrollArea.vue";

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

const {
  conversationCheckpointEntries,
  draftMessage,
  isSubmitting,
  latestExecutionCheckpoint,
  latestGraphRunSubmissionPlan,
  latestRunControlAuditSummary,
  messages,
  sessionOperation
} = storeToRefs(runtimeStore);
const { currentProvider, currentModel } = storeToRefs(providerStore);

const providerMenuOpen = ref(false);
const hoveredProviderId = ref<string | null>(null);
const reasoningMenuOpen = ref(false);
const showReasoningContent = ref(false);
const providerMenuRef = ref<HTMLElement | null>(null);
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
const streamSnapshotTextByMessageId = reactive<Record<string, string>>({});
const streamSnapshotReasoningByMessageId = reactive<Record<string, string>>({});
const streamFadeTextByMessageId = reactive<Record<string, string>>({});
const streamFadeKeyByMessageId = reactive<Record<string, number>>({});
const streamReasoningFadeTextByMessageId = reactive<Record<string, string>>({});
const streamReasoningFadeKeyByMessageId = reactive<Record<string, number>>({});
const AUTO_SCROLL_THRESHOLD_PX = 160;
const STREAM_FADE_MIN_CHARS = 24;
const STREAM_DEBUG_STORAGE_KEY = "pony-agent.debug.stream-metrics";
const PROGRAMMATIC_SCROLL_GRACE_MS = 700;
let scheduledRevealMetricsPush = false;
let pendingRevealMetricsPatch: Record<string, unknown> | null = null;
let pendingStreamAutoFollow = true;
let pendingScrollBehavior: ScrollBehavior = "auto";
let scheduledScrollRequestId = 0;
let programmaticScrollUntilMs = 0;
const streamAutoFollowEnabled = ref(true);

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

function updateStreamDebugReveal(patch: Record<string, unknown>) {
  if (!streamDebugEnabled() || typeof window === "undefined") {
    return;
  }

  const streamWindow = window as Window & {
    __ponyStreamMetrics?: Record<string, unknown>;
  };

  const current = streamWindow.__ponyStreamMetrics ?? {};
  const reveal = (current.reveal as Record<string, unknown> | undefined) ?? {};
  streamWindow.__ponyStreamMetrics = {
    ...current,
    reveal: {
      ...reveal,
      ...patch,
      updatedAt: Date.now()
    }
  };

  if (!isTauriAvailable()) {
    return;
  }

  pendingRevealMetricsPatch = {
    ...((streamWindow.__ponyStreamMetrics.reveal as Record<string, unknown> | undefined) ?? {})
  };
  if (scheduledRevealMetricsPush) {
    return;
  }

  scheduledRevealMetricsPush = true;
  window.setTimeout(() => {
    scheduledRevealMetricsPush = false;
    const payload = pendingRevealMetricsPatch;
    pendingRevealMetricsPatch = null;
    if (!payload) {
      return;
    }

    swallowAsyncError(safeInvoke("record_stream_debug_metrics", {
      section: "reveal",
      payload
    }));
  }, 250);
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
    ? `思考 ${providerStore.currentReasoningEffort}`
    : "思考 默认";
});

const reasoningOptions: Array<{ label: string; value: ProviderReasoningEffort | null }> = [
  { label: "默认", value: null },
  { label: "minimal", value: "minimal" },
  { label: "low", value: "low" },
  { label: "medium", value: "medium" },
  { label: "high", value: "high" }
];

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

  return Array.from(buckets.values());
});

const isEmptyWorkspace = computed(() => turns.value.length === 0);

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

const checkpointPickerEntries = computed(() => checkpointEntries.value.filter((entry) => !entry.isLatest));

const checkpointShortcutLabel = computed(() => {
  if (typeof navigator !== "undefined" && navigator.platform.toLowerCase().includes("mac")) {
    return "Cmd+K";
  }

  return "Ctrl+K";
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

function syncStreamingPresentationState() {
  const activeMessageIds = new Set<string>();
  let pendingAssistantCount = 0;

  for (const message of messages.value) {
    if (message.role !== "assistant") {
      continue;
    }

    activeMessageIds.add(message.id);
    const nextReasoning = assistantReasoning(message);
    const nextText = message.content;

    if (message.status !== "pending") {
      streamSnapshotTextByMessageId[message.id] = nextText;
      streamSnapshotReasoningByMessageId[message.id] = nextReasoning;
      streamFadeTextByMessageId[message.id] = "";
      streamReasoningFadeTextByMessageId[message.id] = "";
      continue;
    }

    pendingAssistantCount += 1;
    const previousText = streamSnapshotTextByMessageId[message.id] ?? "";
    const previousReasoning = streamSnapshotReasoningByMessageId[message.id] ?? "";

    const appendedText = isIncrementalStreamingAppend(previousText, nextText)
      ? nextText.slice(previousText.length)
      : "";
    streamFadeTextByMessageId[message.id] = appendedText.length >= STREAM_FADE_MIN_CHARS ? appendedText : "";
    if (!(message.id in streamFadeKeyByMessageId)) {
      streamFadeKeyByMessageId[message.id] = 0;
    }
    if (streamFadeTextByMessageId[message.id]) {
      streamFadeKeyByMessageId[message.id] = (streamFadeKeyByMessageId[message.id] ?? 0) + 1;
    }

    const appendedReasoning = isIncrementalStreamingAppend(previousReasoning, nextReasoning)
      ? nextReasoning.slice(previousReasoning.length)
      : "";
    streamReasoningFadeTextByMessageId[message.id] =
      appendedReasoning.length >= STREAM_FADE_MIN_CHARS ? appendedReasoning : "";
    if (!(message.id in streamReasoningFadeKeyByMessageId)) {
      streamReasoningFadeKeyByMessageId[message.id] = 0;
    }
    if (streamReasoningFadeTextByMessageId[message.id]) {
      streamReasoningFadeKeyByMessageId[message.id] =
        (streamReasoningFadeKeyByMessageId[message.id] ?? 0) + 1;
    }

    streamSnapshotTextByMessageId[message.id] = nextText;
    streamSnapshotReasoningByMessageId[message.id] = nextReasoning;
  }

  for (const messageId of Object.keys(streamSnapshotTextByMessageId)) {
    if (!activeMessageIds.has(messageId)) {
      delete streamSnapshotTextByMessageId[messageId];
    }
  }

  for (const messageId of Object.keys(streamSnapshotReasoningByMessageId)) {
    if (!activeMessageIds.has(messageId)) {
      delete streamSnapshotReasoningByMessageId[messageId];
    }
  }

  for (const messageId of Object.keys(streamFadeTextByMessageId)) {
    if (!activeMessageIds.has(messageId)) {
      delete streamFadeTextByMessageId[messageId];
    }
  }

  for (const messageId of Object.keys(streamFadeKeyByMessageId)) {
    if (!activeMessageIds.has(messageId)) {
      delete streamFadeKeyByMessageId[messageId];
    }
  }

  for (const messageId of Object.keys(streamReasoningFadeTextByMessageId)) {
    if (!activeMessageIds.has(messageId)) {
      delete streamReasoningFadeTextByMessageId[messageId];
    }
  }

  for (const messageId of Object.keys(streamReasoningFadeKeyByMessageId)) {
    if (!activeMessageIds.has(messageId)) {
      delete streamReasoningFadeKeyByMessageId[messageId];
    }
  }
  updateStreamDebugReveal({
    pendingAssistantCount,
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

function messageCheckpointActionTitle(
  entry: ConversationCheckpointEntry,
  action: CheckpointRollbackAction
) {
  if (action === "transcript_only") {
    if (isSubmitting.value) {
      return "运行中暂不可回退 checkpoint（仅对话）";
    }
    if (sessionOperation.value) {
      return "当前正在处理会话操作，暂不可回退 checkpoint（仅对话）";
    }
    return "回到此 checkpoint（仅对话）";
  }

  if (isSubmitting.value) {
    return "运行中暂不可回退 checkpoint（对话 + 文件）";
  }
  if (sessionOperation.value) {
    return "当前正在处理会话操作，暂不可回退 checkpoint（对话 + 文件）";
  }
  if (!entry.workspaceRollbackCapable) {
    return "该 checkpoint 不支持文件回退，将仅恢复对话历史";
  }
  return "回到此 checkpoint（对话 + 文件）";
}

function canUseCheckpointAction(_entry: ConversationCheckpointEntry, _action: CheckpointRollbackAction) {
  return !isSubmitting.value && !sessionOperation.value;
}

function checkpointPickerEntryTitle(entry: ConversationCheckpointEntry) {
  return entry.workspaceRollbackCapable
    ? "回到此 checkpoint（优先恢复对话与文件）"
    : "回到此 checkpoint（仅恢复对话历史）";
}

function checkpointModeForPickerEntry(entry: ConversationCheckpointEntry): HistoryCheckoutMode {
  return entry.workspaceRollbackCapable ? "transcript_and_workspace" : "transcript_only";
}

function checkpointMetaLabel(entry: ConversationCheckpointEntry) {
  return entry.workspaceRollbackCapable ? "对话 + 文件" : "仅对话";
}

async function rollbackToCheckpoint(
  entry: ConversationCheckpointEntry,
  action: CheckpointRollbackAction
) {
  if (!canUseCheckpointAction(entry, action)) {
    return;
  }

  forkSummaryOpenForNodeId.value = null;
  checkpointPickerOpen.value = false;
  await runtimeStore.checkoutHistoryNode(entry.nodeId, action);
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

async function selectCheckpointPickerEntry(entry: ConversationCheckpointEntry) {
  await rollbackToCheckpoint(entry, checkpointModeForPickerEntry(entry));
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

function handleCheckpointShortcut(event: KeyboardEvent) {
  const isMac = typeof navigator !== "undefined" && navigator.platform.toLowerCase().includes("mac");
  const shortcutPressed = event.key.toLowerCase() === "k" && (isMac ? event.metaKey : event.ctrlKey);

  if (shortcutPressed && !event.shiftKey && !event.altKey) {
    event.preventDefault();
    if (checkpointPickerEntries.value.length > 0) {
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
    hoveredProviderId.value = currentProvider.value?.id ?? providerStore.providers[0]?.id ?? null;
    reasoningMenuOpen.value = false;
    return;
  }

  hoveredProviderId.value = null;
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
  if (Date.now() < programmaticScrollUntilMs) {
    streamAutoFollowEnabled.value = true;
    return;
  }

  streamAutoFollowEnabled.value = isTimelineNearBottom();
}

function handleTimelineUserScrollIntent() {
  programmaticScrollUntilMs = 0;
  streamAutoFollowEnabled.value = isTimelineNearBottom();
}

onMounted(() => {
  if (typeof window !== "undefined") {
    showReasoningContent.value = window.localStorage.getItem(SHOW_REASONING_STORAGE_KEY) === "true";
  }
  syncStreamingPresentationState();
  window.addEventListener("click", handleClickOutside);
  window.addEventListener("keydown", handleWindowKeydown);
  handleTimelineViewportScroll();
  queueScrollToLatestTurn("auto");
});

onBeforeUnmount(() => {
  window.removeEventListener("click", handleClickOutside);
  window.removeEventListener("keydown", handleWindowKeydown);
  const viewport = timelineScrollAreaRef.value?.viewportEl ?? null;
  if (viewport && "removeEventListener" in viewport) {
    viewport.removeEventListener("scroll", handleTimelineViewportScroll);
    viewport.removeEventListener("wheel", handleTimelineUserScrollIntent);
    viewport.removeEventListener("touchstart", handleTimelineUserScrollIntent);
  }
  if (scrollAfterPaintFrameId.value != null) {
    window.cancelAnimationFrame(scrollAfterPaintFrameId.value);
  }
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

watch(latestTurnSignature, () => {
  pendingStreamAutoFollow = streamAutoFollowEnabled.value || isTimelineNearBottom();
}, { flush: "pre" });

watch(latestTurnSignature, () => {
  syncStreamingPresentationState();
  const behavior: ScrollBehavior = pendingStreamAutoFollow ? "smooth" : "auto";
  streamAutoFollowEnabled.value = pendingStreamAutoFollow;

  if (streamAutoFollowEnabled.value) {
    queueScrollToLatestTurn(behavior);
  }
}, { flush: "post" });

watch(showReasoningContent, (value) => {
  if (typeof window !== "undefined") {
    window.localStorage.setItem(SHOW_REASONING_STORAGE_KEY, value ? "true" : "false");
  }
});

watch(isSubmitting, (submitting) => {
  syncStreamingPresentationState();
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
      <div class="mx-auto w-full max-w-[58rem]" data-testid="workspace-content-column">
      <TransitionGroup name="turn-flow" tag="div" class="space-y-5">
        <section
          v-if="isEmptyWorkspace"
          key="workspace-empty-state"
          class="flex min-h-[46vh] items-center justify-center px-5 py-10 text-center"
          data-testid="workspace-empty-state"
        >
          <h2 class="text-[28px] font-medium tracking-[-0.04em] text-stone-500">
            需要我帮你做什么？
          </h2>
        </section>
        <section v-for="turn in turns" :key="turn.turnId" class="space-y-3">
          <article v-if="turn.user" class="conversation-user-message ml-auto w-fit max-w-[86%] sm:max-w-[68%]">
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
            </div>
          </article>

          <article v-if="turn.assistant || turn.tools.length" class="conversation-agent-shell w-full px-0 py-1">
            <div class="conversation-agent-header flex items-center justify-between gap-3">
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

            <details v-if="turn.tools.length" class="conversation-disclosure conversation-tool-panel mt-2 group">
              <summary class="conversation-disclosure-summary">
                <Wrench class="h-3.5 w-3.5 shrink-0 text-stone-400" />
                <span>工具调用</span>
                <span class="inline-flex items-center rounded-full border border-stone-200/80 px-2 py-0.5 text-[10px] normal-case tracking-normal text-stone-500">{{ turn.tools.length }} 项</span>
                <ChevronDown class="conversation-disclosure-chevron h-3.5 w-3.5 shrink-0 text-stone-400" />
              </summary>
              <div class="conversation-tool-list mt-1 space-y-0.5">
                <div
                  v-for="tool in turn.tools"
                  :key="tool.id"
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

            <div
              v-if="turn.assistant && checkpointEntryForTurn(turn.turnId) && !checkpointEntryForTurn(turn.turnId)?.isLatest"
              class="mt-3 flex flex-wrap items-center gap-2 border-t border-stone-200/60 pt-2.5"
              data-testid="workspace-checkpoint-actions"
            >
              <button
                class="checkpoint-icon-button"
                type="button"
                :disabled="!canUseCheckpointAction(requireCheckpointEntryForTurn(turn.turnId), 'transcript_only')"
                :title="messageCheckpointActionTitle(requireCheckpointEntryForTurn(turn.turnId), 'transcript_only')"
                :data-testid="`workspace-checkpoint-transcript-${checkpointEntryForTurn(turn.turnId)?.nodeId}`"
                @click="rollbackToCheckpoint(requireCheckpointEntryForTurn(turn.turnId), 'transcript_only')"
              >
                <History class="h-3.5 w-3.5" />
                <span class="sr-only">仅回退对话历史</span>
              </button>

              <button
                class="checkpoint-icon-button"
                type="button"
                :disabled="!canUseCheckpointAction(requireCheckpointEntryForTurn(turn.turnId), 'transcript_and_workspace')"
                :title="messageCheckpointActionTitle(requireCheckpointEntryForTurn(turn.turnId), 'transcript_and_workspace')"
                :data-testid="`workspace-checkpoint-workspace-${checkpointEntryForTurn(turn.turnId)?.nodeId}`"
                @click="rollbackToCheckpoint(requireCheckpointEntryForTurn(turn.turnId), 'transcript_and_workspace')"
              >
                <RotateCcw class="h-3.5 w-3.5" />
                <span class="sr-only">回退对话历史并尝试恢复文件改动</span>
              </button>

              <div
                v-if="checkpointEntryForTurn(turn.turnId)?.forkTargets.length"
                ref="forkSummaryMenuRef"
                class="relative"
              >
                <button
                  class="checkpoint-icon-button"
                  type="button"
                  title="查看从该 checkpoint 分叉出来的对话轨迹"
                  :data-testid="`workspace-checkpoint-forks-${checkpointEntryForTurn(turn.turnId)?.nodeId}`"
                  @click="toggleForkSummary(requireCheckpointEntryForTurn(turn.turnId).nodeId)"
                >
                  <GitFork class="h-3.5 w-3.5" />
                  <span class="sr-only">查看 fork 对话摘要</span>
                </button>

                <div
                  v-if="forkSummaryOpenForNodeId === checkpointEntryForTurn(turn.turnId)?.nodeId"
                  class="checkpoint-popover absolute left-0 top-[calc(100%+0.45rem)] z-20 w-[19rem] max-w-[calc(100vw-2rem)]"
                  :data-testid="`workspace-checkpoint-fork-menu-${checkpointEntryForTurn(turn.turnId)?.nodeId}`"
                >
                  <div class="checkpoint-popover-caption">Fork 对话</div>
                  <div class="checkpoint-popover-divider"></div>
                  <button
                    v-for="target in checkpointEntryForTurn(turn.turnId)?.forkTargets ?? []"
                    :key="`${checkpointEntryForTurn(turn.turnId)?.nodeId}-${target.branchId}-${target.nodeId}`"
                    class="checkpoint-picker-item"
                    type="button"
                    :data-testid="`workspace-checkpoint-fork-target-${target.branchId}`"
                    @click="jumpToForkTarget(requireCheckpointEntryForTurn(turn.turnId), target)"
                  >
                    <div class="flex min-w-0 flex-1 flex-col text-left">
                      <span class="truncate text-[12px] text-stone-800">{{ target.label }}</span>
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
              :disabled="checkpointPickerEntries.length === 0"
              :title="
                checkpointPickerEntries.length
                  ? `打开 checkpoint 菜单（${checkpointShortcutLabel}）`
                  : '当前还没有可回退的 checkpoint'
              "
              data-testid="workspace-checkpoint-picker-trigger"
              @click.stop="toggleCheckpointPicker"
            >
              <History class="h-3.5 w-3.5 text-stone-500" />
              <span class="truncate">Checkpoint</span>
              <ChevronDown class="h-2.5 w-2.5 text-stone-400" />
            </button>

            <div
              v-if="checkpointPickerOpen"
              class="composer-menu-panel absolute bottom-[calc(100%+0.45rem)] left-0 z-20 min-w-[18rem] max-w-[min(28rem,calc(100vw-2rem))]"
              data-testid="workspace-checkpoint-picker-menu"
            >
              <div class="composer-menu-caption">Checkpoint 菜单</div>
              <div class="composer-menu-divider"></div>
              <button
                v-for="entry in checkpointPickerEntries"
                :key="`picker-${entry.nodeId}`"
                class="checkpoint-picker-item"
                type="button"
                :title="checkpointPickerEntryTitle(entry)"
                :data-testid="`workspace-checkpoint-picker-item-${entry.nodeId}`"
                @click="selectCheckpointPickerEntry(entry)"
              >
                <div class="flex min-w-0 flex-1 flex-col text-left">
                  <span class="truncate text-[12px] text-stone-800">{{ entry.summary }}</span>
                  <span class="mt-0.5 text-[10px] leading-4 text-stone-500">
                    分支 {{ entry.branchId }} · {{ checkpointMetaLabel(entry) }}
                  </span>
                </div>
                <span
                  class="shrink-0 rounded-full border border-stone-200/80 px-2 py-0.5 text-[10px] text-stone-500"
                >
                  {{ entry.isVisible ? "当前" : "回退" }}
                </span>
              </button>
              <div class="composer-menu-divider"></div>
              <div class="composer-menu-caption">快捷键：{{ checkpointShortcutLabel }}</div>
            </div>
          </div>

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
              <div class="relative py-0.5">
                <button
                  v-for="provider in providerStore.providers"
                  :key="provider.id"
                  class="composer-menu-item"
                  type="button"
                  @mouseenter="hoveredProviderId = provider.id"
                  @focus="hoveredProviderId = provider.id"
                >
                  <span class="truncate">{{ provider.name }}</span>
                  <div class="flex items-center gap-2">
                    <Check v-if="currentProvider?.id === provider.id" class="h-3.5 w-3.5 text-stone-700" />
                    <ChevronDown class="h-3.5 w-3.5 -rotate-90 text-stone-400" />
                  </div>
                </button>
                <div
                  v-if="hoveredProviderId"
                  class="composer-menu-panel absolute left-full top-0 ml-1 min-w-[14rem]"
                >
              <div class="composer-menu-caption">模型</div>
                  <div class="composer-menu-divider"></div>
                  <button
                    v-for="model in providerStore.providers.find((provider) => provider.id === hoveredProviderId)?.models ?? []"
                    :key="model.id"
                    class="composer-menu-item"
                    type="button"
                    @click="selectModel(hoveredProviderId, model.id)"
                  >
                    <span class="truncate">{{ model.name }}</span>
                    <Check
                      v-if="currentProvider?.id === hoveredProviderId && currentModel?.id === model.id"
                      class="h-3.5 w-3.5 text-stone-700"
                    />
                  </button>
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
  font-weight: 700;
}

:deep(.assistant-markdown h3),
:deep(.assistant-markdown h4),
:deep(.assistant-markdown h5),
:deep(.assistant-markdown h6) {
  font-weight: 600;
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
  font-weight: 600;
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
  font-weight: 700;
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
    opacity 260ms ease,
    transform 260ms ease;
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
  animation: conversation-element-rise 220ms ease-out both;
  will-change: opacity, transform;
}

.conversation-agent-header {
  animation-delay: 20ms;
}

.conversation-tool-panel,
.conversation-reasoning-panel,
.assistant-response-panel {
  animation-delay: 40ms;
}

.conversation-tool-row {
  animation-duration: 180ms;
}

@keyframes conversation-element-rise {
  from {
    opacity: 0;
    transform: translateY(0.35rem);
  }

  to {
    opacity: 1;
    transform: translateY(0);
  }
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
  font-weight: 700;
  line-height: 1;
  color: #746d64;
}

:deep(.assistant-reasoning-markdown h3),
:deep(.assistant-reasoning-markdown h4),
:deep(.assistant-reasoning-markdown h5),
:deep(.assistant-reasoning-markdown h6) {
  font-size: inherit;
  font-weight: 600;
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
  height: 1.9rem;
  width: 1.9rem;
  align-items: center;
  justify-content: center;
  border-radius: 9999px;
  border: 1px solid rgba(231, 229, 228, 0.95);
  background: rgba(255, 255, 255, 0.92);
  color: rgb(120 113 108);
  transition:
    border-color 0.16s ease,
    background-color 0.16s ease,
    color 0.16s ease;
}

.checkpoint-icon-button:hover {
  border-color: rgba(214, 188, 146, 0.95);
  background: rgba(251, 244, 232, 0.98);
  color: rgb(68 64 60);
}

.checkpoint-icon-button:disabled {
  cursor: not-allowed;
  opacity: 0.45;
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
</style>
