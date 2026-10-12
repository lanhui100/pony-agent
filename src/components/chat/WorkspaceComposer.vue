<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { storeToRefs } from "pinia";
import { ArrowUp, Check, ChevronDown, LoaderCircle, Paperclip, Search, Square, Undo2, X } from "lucide-vue-next";
import type { ProviderModelConfig, ProviderReasoningEffort } from "@/types/provider";
import type { ToolAuthorizationMode } from "@/types/runtime";
import type { PendingAsk } from "@/types/ask-plan";
import { useProviderStore } from "@/stores/providers";
import { useRuntimeStore } from "@/stores/runtime";
import { useAskStore } from "@/stores/ask";
import { pickFiles } from "@/lib/runtime/file-attachments";
import { useTraceProjection } from "@/lib/runtime/useTraceProjection";
import { recordFrontendInstant } from "@/lib/frontend-flight-recorder";
import Button from "@/components/ui/Button.vue";
import Switch from "@/components/ui/Switch.vue";
import {
  TooltipContent,
  TooltipPortal,
  TooltipRoot,
  TooltipTrigger,
  DialogRoot,
  DialogPortal,
  DialogOverlay,
  DialogContent,
  DialogTitle,
  DialogDescription
} from "reka-ui";
import { AlertTriangle } from "lucide-vue-next";
import PermissionModeIcon from "@/components/chat/PermissionModeIcon.vue";

type ComposerActionKind = "submit" | "resume" | "continue" | "restart";

const props = defineProps<{
  showReasoningContent: boolean;
  canUndoLastTurn: boolean;
  undoShortcutLabel: string;
  handleComposerKeydown: (event: KeyboardEvent) => void;
  handlePrimaryAction: () => void;
  handleUndoLastTurn: () => void;
  setComposerShellRef: (element: unknown) => void;
}>();

const emit = defineEmits<{
  (event: "update:showReasoningContent", value: boolean): void;
}>();

const runtimeStore = useRuntimeStore();
const providerStore = useProviderStore();
const {
  draftMessage,
  isSubmitting,
  latestExecutionCheckpoint,
  latestGraphRunSubmissionPlan,
  latestRunControlAuditSummary,
  pendingAttachments,
  sessionOperation
} = storeToRefs(runtimeStore);
const { currentProvider, currentModel } = storeToRefs(providerStore);

const menuOpen = ref(false);
const modelMenuOpen = ref(false);
const reasoningMenuOpen = ref(false);
const menuRef = ref<HTMLElement | null>(null);
const modelTriggerRef = ref<HTMLElement | null>(null);
const floatingMenuRef = ref<HTMLElement | null>(null);
const modelSubmenuStyle = ref<Record<string, string>>({ top: '0' });
const reasoningSubmenuStyle = ref<Record<string, string>>({ top: '0' });
const floatingMenuStyle = ref<Record<string, string>>({
  left: '0px',
  top: '0px',
  visibility: 'hidden'
});
const modelSubmenuDirection = ref<'left' | 'right'>('right');
const reasoningSubmenuDirection = ref<'left' | 'right'>('right');
const modelSubmenuRef = ref<HTMLElement | null>(null);
const reasoningSubmenuRef = ref<HTMLElement | null>(null);

// 上下文用量接入 (需求 1)
const { contextDisplayTokens, currentContextWindowTokens } = useTraceProjection({ liveTurnEnabled: ref(true) });

function formatCompactTokenInteger(value?: number | null) {
  if (value == null || !Number.isFinite(value)) { return "0"; }
  if (value >= 1_000_000) { return `${Math.round(value / 1_000_000)}M`; }
  if (value >= 1_000) { return `${Math.round(value / 1_000)}K`; }
  return String(value);
}

const contextUsagePercentage = computed(() => {
  if (contextDisplayTokens.value == null || !currentContextWindowTokens.value || currentContextWindowTokens.value <= 0) {
    return 0;
  }
  const ratio = (contextDisplayTokens.value / currentContextWindowTokens.value) * 100;
  return Math.min(100, Math.max(0, ratio < 1 && ratio > 0 ? 1 : Math.round(ratio)));
});

const contextUsageTooltip = computed(() => {
  if (contextDisplayTokens.value == null) {
    return "上下文用量：暂无数据";
  }
  if (!currentContextWindowTokens.value || currentContextWindowTokens.value <= 0) {
    return `上下文用量：${formatCompactTokenInteger(contextDisplayTokens.value)}`;
  }
  const exactPercentage = ((contextDisplayTokens.value / currentContextWindowTokens.value) * 100);
  const displayPercent = exactPercentage > 0 && exactPercentage < 1 ? "<1" : String(Math.round(exactPercentage));
  return `上下文用量：${displayPercent}% · ${formatCompactTokenInteger(contextDisplayTokens.value)} / ${formatCompactTokenInteger(currentContextWindowTokens.value)}`;
});

const modelSearchQuery = ref("");
const modelSearchDebounced = ref("");
let modelSearchDebounceTimer: ReturnType<typeof setTimeout> | undefined;
const INTERNAL_MODEL_NAME_PATTERN = /^model-(?:[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}|\d+-[0-9a-f]{6}|[a-z0-9-]+-default)$/i;

watch(modelSearchQuery, (value) => {
  if (modelSearchDebounceTimer) {
    clearTimeout(modelSearchDebounceTimer);
  }
  modelSearchDebounceTimer = setTimeout(() => {
    modelSearchDebounced.value = value.trim().toLowerCase();
  }, 250);
});

const filteredProviders = computed(() => {
  const keyword = modelSearchDebounced.value;
  if (!keyword) {
    return providerStore.providers;
  }

  return providerStore.providers
    .map((provider) => ({
      ...provider,
      models: (provider.models ?? []).filter((model) =>
        modelDisplayName(model).toLowerCase().includes(keyword) ||
        model.model.toLowerCase().includes(keyword)
      )
    }))
    .filter((provider) => (provider.models?.length ?? 0) > 0);
});

const currentModelSupportsReasoning = computed(
  () => currentModel.value?.capabilities?.supportsReasoning ?? false
);

const providerTooltipDetail = computed(() => {
  const providerName = currentProvider.value?.name?.trim() || "未选择";
  return `当前：${providerName}`;
});

function modelDisplayName(model: Pick<ProviderModelConfig, "name" | "model"> | null | undefined) {
  const displayName = model?.name?.trim() ?? "";
  if (displayName && !INTERNAL_MODEL_NAME_PATTERN.test(displayName)) {
    return displayName;
  }

  return model?.model?.trim() || "未命名模型";
}

// ── 授权/权限模式选择器 ──────────────────────────────────────────
const authMenuOpen = ref(false);
const authTriggerRef = ref<HTMLButtonElement | null>(null);
const authFloatingMenuRef = ref<HTMLElement | null>(null);
const authMenuStyle = ref<Record<string, string>>({});
const fullAccessConfirmOpen = ref(false);

const authModes: Array<{ id: ToolAuthorizationMode; label: string; desc: string }> = [
  { id: "read_only", label: "工作区查看", desc: "仅允许读操作，禁止修改文件与执行环境" },
  { id: "workspace_write", label: "工作区修改", desc: "允许修改工作区文件及常规沙箱操作" },
  { id: "full_access", label: "完全权限", desc: "无限制执行，直接访问系统与外部资源" }
];

const currentAuthModeLabel = computed(() => {
  const current = runtimeStore.toolAuthorizationMode ?? "workspace_write";
  return authModes.find((m) => m.id === current)?.label || "工作区修改";
});

function toggleAuthMenu() {
  authMenuOpen.value = !authMenuOpen.value;
  if (authMenuOpen.value) {
    menuOpen.value = false;
    void nextTick().then(updateAuthMenuPosition);
  }
}

function selectAuthMode(mode: ToolAuthorizationMode) {
  authMenuOpen.value = false;
  if (mode === runtimeStore.toolAuthorizationMode) return;
  if (mode === "full_access") {
    fullAccessConfirmOpen.value = true;
    return;
  }
  runtimeStore.setToolAuthorizationMode(mode);
}

function confirmFullAccess() {
  runtimeStore.setToolAuthorizationMode("full_access");
  fullAccessConfirmOpen.value = false;
}

function cancelFullAccess() {
  fullAccessConfirmOpen.value = false;
}

function updateAuthMenuPosition() {
  const trigger = authTriggerRef.value;
  const menu = authFloatingMenuRef.value;
  if (!authMenuOpen.value || !trigger || !menu) return;

  const viewportPadding = 8;
  const triggerRect = trigger.getBoundingClientRect();
  const menuRect = menu.getBoundingClientRect();
  const maxLeft = Math.max(viewportPadding, window.innerWidth - menuRect.width - viewportPadding);
  const maxTop = Math.max(viewportPadding, window.innerHeight - menuRect.height - viewportPadding);
  const left = Math.min(
    Math.max(viewportPadding, triggerRect.left),
    maxLeft
  );
  const top = Math.min(
    Math.max(viewportPadding, triggerRect.top - menuRect.height - viewportPadding),
    maxTop
  );

  authMenuStyle.value = {
    left: `${left}px`,
    top: `${top}px`,
    visibility: "visible"
  };
}
const reasoningSummary = computed(() => {
  if (!currentModelSupportsReasoning.value) {
    return "不支持";
  }

  return providerStore.currentReasoningEffort
    ? reasoningEffortLabelZh(providerStore.currentReasoningEffort)
    : "默认";
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

const primaryActionDisabled = computed(() => {
  if (sessionOperation.value) {
    return true;
  }
  if (isSubmitting.value) {
    return false;
  }
  const hasReadyAttachment = pendingAttachments.value.some((attachment) => attachment.status === "ok");
  return draftMessage.value.trim().length === 0 && !hasReadyAttachment;
});

const primaryActionTitle = computed(() => {
  if (isSubmitting.value) {
    if (draftMessage.value.trim().length > 0) {
      return runtimeStore.queueDeliveryMode === "steer"
        ? "立即插队发送（在当前回合步骤完成后优先执行）"
        : "排队发送（当前回合完成后按序执行）";
    }
    return "请求在安全边界停止当前运行。";
  }
  return composerAction.value.hint;
});

const attachNotice = ref<string | null>(null);

async function handleAttach() {
  attachNotice.value = null;
  try {
    const files = await pickFiles();
    if (!files.length) {
      return;
    }
    const { errors } = await runtimeStore.addPendingAttachments(files);
    if (errors.length) {
      attachNotice.value = errors.join("；");
    }
  } catch (error) {
    attachNotice.value = String(error);
  }
}

function removePendingAttachment(id: string) {
  runtimeStore.removePendingAttachment(id);
  attachNotice.value = null;
}

function formatAttachmentSize(sizeBytes: number): string {
  if (sizeBytes < 1024) {
    return `${sizeBytes} B`;
  }
  if (sizeBytes < 1024 * 1024) {
    return `${(sizeBytes / 1024).toFixed(1)} KB`;
  }
  return `${(sizeBytes / 1024 / 1024).toFixed(1)} MB`;
}

async function toggleMenu() {
  menuOpen.value = !menuOpen.value;
  reasoningMenuOpen.value = false;

  if (menuOpen.value) {
    if (modelSearchDebounceTimer) {
      clearTimeout(modelSearchDebounceTimer);
      modelSearchDebounceTimer = undefined;
    }
    modelSearchQuery.value = "";
    modelSearchDebounced.value = "";
    modelMenuOpen.value = true;
    floatingMenuStyle.value = {
      left: '0px',
      top: '0px',
      visibility: 'hidden'
    };
    await nextTick();
    updateFloatingMenuPosition();
    await nextTick();
    const modelItem = floatingMenuRef.value?.querySelector<HTMLElement>("[data-menu-item='model']");
    if (modelItem) {
      adjustModelSubmenuPosition(modelItem);
    }
    return;
  }

  modelMenuOpen.value = false;
}

function updateFloatingMenuPosition() {
  const trigger = modelTriggerRef.value;
  const floatingMenu = floatingMenuRef.value;
  if (!menuOpen.value || !trigger || !floatingMenu) {
    return;
  }

  const viewportPadding = 8;
  const triggerRect = trigger.getBoundingClientRect();
  const menuRect = floatingMenu.getBoundingClientRect();
  const maxLeft = Math.max(viewportPadding, window.innerWidth - menuRect.width - viewportPadding);
  const maxTop = Math.max(viewportPadding, window.innerHeight - menuRect.height - viewportPadding);
  const left = Math.min(
    Math.max(viewportPadding, triggerRect.right - menuRect.width),
    maxLeft
  );
  const top = Math.min(
    Math.max(viewportPadding, triggerRect.top - menuRect.height - viewportPadding),
    maxTop
  );
  const spaceRight = window.innerWidth - (left + menuRect.width) - viewportPadding;

  modelSubmenuDirection.value = spaceRight >= 248 ? 'right' : 'left';
  reasoningSubmenuDirection.value = spaceRight >= 168 ? 'right' : 'left';
  floatingMenuStyle.value = {
    left: `${left}px`,
    top: `${top}px`,
    visibility: 'visible'
  };
}

async function adjustModelSubmenuPosition(button: HTMLElement) {
  await nextTick();
  const buttonRect = button.getBoundingClientRect();
  const viewportHeight = window.innerHeight;
  const submenu = modelSubmenuRef.value;

  // 实际测量子菜单高度，若尚未挂载则取最大高度 340 兜底
  const submenuHeight = submenu ? submenu.getBoundingClientRect().height : 340;
  const spaceBelow = viewportHeight - buttonRect.top - 8;

  let top = 0;
  if (submenuHeight > spaceBelow) {
    const overflow = submenuHeight - spaceBelow;
    // 向上平移，但不得让子菜单顶部超出视口上缘（距顶部至少 8px）
    const maxShiftUp = Math.max(0, buttonRect.top - 8);
    top = -Math.min(overflow, maxShiftUp);
  }

  // 视口小屏幕自适应最大高度约束，防止视口高度小于菜单高度时截断
  const maxAllowedHeight = Math.max(160, viewportHeight - 16);
  modelSubmenuStyle.value = {
    top: `${top}px`,
    maxHeight: `${maxAllowedHeight}px`,
    display: 'flex',
    flexDirection: 'column'
  };
}

async function adjustReasoningSubmenuPosition(button: HTMLElement) {
  await nextTick();
  const buttonRect = button.getBoundingClientRect();
  const viewportHeight = window.innerHeight;
  const submenu = reasoningSubmenuRef.value;

  const submenuHeight = submenu ? submenu.getBoundingClientRect().height : 180;
  const spaceBelow = viewportHeight - buttonRect.top - 8;

  let top = 0;
  if (submenuHeight > spaceBelow) {
    const overflow = submenuHeight - spaceBelow;
    const maxShiftUp = Math.max(0, buttonRect.top - 8);
    top = -Math.min(overflow, maxShiftUp);
  }

  const maxAllowedHeight = Math.max(160, viewportHeight - 16);
  reasoningSubmenuStyle.value = {
    top: `${top}px`,
    maxHeight: `${maxAllowedHeight}px`
  };
}

function onModelEnter(event: MouseEvent) {
  modelMenuOpen.value = true;
  reasoningMenuOpen.value = false;
  adjustModelSubmenuPosition(event.currentTarget as HTMLElement);
}

function onModelFocus(event: FocusEvent) {
  modelMenuOpen.value = true;
  reasoningMenuOpen.value = false;
  adjustModelSubmenuPosition(event.currentTarget as HTMLElement);
}

function onReasoningEnter(event: MouseEvent) {
  reasoningMenuOpen.value = true;
  modelMenuOpen.value = false;
  adjustReasoningSubmenuPosition(event.currentTarget as HTMLElement);
}

function onReasoningFocus(event: FocusEvent) {
  reasoningMenuOpen.value = true;
  modelMenuOpen.value = false;
  adjustReasoningSubmenuPosition(event.currentTarget as HTMLElement);
}

async function selectModel(providerId: string, modelId: string) {
  providerStore.selectModel(providerId, modelId);
  menuOpen.value = false;
  modelMenuOpen.value = false;
  await providerStore.saveRegistry();
}

function selectReasoningEffort(value: ProviderReasoningEffort | null) {
  providerStore.setCurrentReasoningEffort(value);
  reasoningMenuOpen.value = false;
}

function toggleReasoningVisibility() {
  emit("update:showReasoningContent", !props.showReasoningContent);
  modelMenuOpen.value = false;
  reasoningMenuOpen.value = false;
}

function handleShowReasoningSwitch(value: boolean) {
  emit("update:showReasoningContent", value);
}

function handleClickOutside(event: MouseEvent) {
  const target = event.target as Node | null;

  const clickedTrigger = menuRef.value?.contains(target) ?? false;
  const clickedFloatingMenu = floatingMenuRef.value?.contains(target) ?? false;
  if (!clickedTrigger && !clickedFloatingMenu) {
    menuOpen.value = false;
    modelMenuOpen.value = false;
    reasoningMenuOpen.value = false;
  }

  const clickedAuthTrigger = authTriggerRef.value?.contains(target) ?? false;
  const clickedAuthMenu = authFloatingMenuRef.value?.contains(target) ?? false;
  if (!clickedAuthTrigger && !clickedAuthMenu) {
    authMenuOpen.value = false;
  }
}

// ── Ask 交互面板内聚逻辑 ──────────────────────────────────────────
const askStore = useAskStore();

const activeAsk = computed<PendingAsk | null>(() => {
  const currentSessionId = runtimeStore.sessionId;
  if (!askStore.pendingAsks || askStore.pendingAsks.length === 0) {
    return null;
  }
  if (!currentSessionId) {
    return askStore.pendingAsks[0] ?? null;
  }
  return (
    askStore.pendingAsks.find(
      (ask) => !ask.sessionId || ask.sessionId === currentSessionId
    ) ?? askStore.pendingAsks[0] ?? null
  );
});

export type ResolvedAskOption = {
  raw: unknown;
  label: string;
  description: string;
};

function fallbackOptionDescription(label: string, index: number): string {
  const normalized = label.trim().toLowerCase();
  if (["继续", "确认", "确定", "是", "yes", "ok", "confirm", "continue"].includes(normalized)) {
    return "确认并继续推进当前任务";
  }
  if (["取消", "否", "终止", "放弃", "no", "cancel", "abort", "stop"].includes(normalized)) {
    return "中断或放弃当前操作，返回上一状态";
  }
  if (["重试", "retry"].includes(normalized)) {
    return "重新尝试执行当前失败的步骤";
  }
  if (["跳过", "skip"].includes(normalized)) {
    return "略过当前步骤并进入下一流程";
  }
  if (index === 0) {
    return "系统推荐使用的优选方案";
  }
  return "备选执行方案";
}

function resolveAskOptionItem(item: unknown, index: number): ResolvedAskOption {
  if (typeof item === "object" && item !== null) {
    const obj = item as Record<string, unknown>;
    const label =
      (typeof obj.label === "string" && obj.label.trim()) ||
      (typeof obj.text === "string" && obj.text.trim()) ||
      (typeof obj.title === "string" && obj.title.trim()) ||
      (typeof obj.value === "string" && obj.value.trim()) ||
      String(index + 1);
    const description =
      (typeof obj.description === "string" && obj.description.trim()) ||
      (typeof obj.desc === "string" && obj.desc.trim()) ||
      (typeof obj.hint === "string" && obj.hint.trim()) ||
      (typeof obj.detail === "string" && obj.detail.trim()) ||
      "";
    return {
      raw: item,
      label,
      description: description || fallbackOptionDescription(label, index)
    };
  }

  const rawStr = typeof item === "string" ? item.trim() : String(item ?? "").trim();
  return {
    raw: item,
    label: rawStr,
    description: fallbackOptionDescription(rawStr, index)
  };
}

const askOptions = computed<ResolvedAskOption[]>(() => {
  const ask = activeAsk.value;
  if (!ask || !Array.isArray(ask.options)) {
    return [];
  }
  return ask.options
    .map((option, index) => resolveAskOptionItem(option, index))
    .filter((value): value is ResolvedAskOption => value != null && Boolean(value.label));
});

const selectedOptionIndex = ref<number | null>(null);
const customOptionInput = ref("");
const askTextareaInput = ref("");

watch(
  () => activeAsk.value?.requestId ?? null,
  () => {
    selectedOptionIndex.value = null;
    customOptionInput.value = "";
    askTextareaInput.value = "";
  }
);

const askIsBusy = computed(() => {
  const ask = activeAsk.value;
  if (!ask) return false;
  return (
    askStore.answeringRequestId === ask.requestId ||
    askStore.cancellingRequestId === ask.requestId
  );
});

const askSubmitDisabled = computed(() => {
  if (askIsBusy.value) return true;
  if (!activeAsk.value) return true;
  if (askOptions.value.length > 0) {
    if (
      selectedOptionIndex.value != null &&
      selectedOptionIndex.value >= 0 &&
      selectedOptionIndex.value < askOptions.value.length
    ) {
      return false;
    }
    if (
      selectedOptionIndex.value === askOptions.value.length ||
      customOptionInput.value.trim().length > 0
    ) {
      return customOptionInput.value.trim().length === 0;
    }
    return true;
  }
  return askTextareaInput.value.trim().length === 0;
});

function selectOption(index: number) {
  selectedOptionIndex.value = index;
}

async function handleOptionEnter(index: number) {
  selectedOptionIndex.value = index;
  await submitAskAnswer();
}

function recordAskErrorToTrace(ask: PendingAsk | null, errorMessage: string) {
  if (!errorMessage || !errorMessage.trim()) return;
  const msg = errorMessage.trim();

  // 1. 记录到前端 Flight Recorder (环形缓冲区诊断 trace)
  recordFrontendInstant("ask", "ask_panel_error", {
    requestId: ask?.requestId ?? null,
    turnId: ask?.turnId ?? null,
    callId: ask?.callId ?? null,
    prompt: ask?.prompt ?? null,
    error: msg
  });

  // 2. 记录到运行时 Turn Trace 历史与 TraceTimeline (业务轨迹面板)
  const targetTurnId =
    ask?.turnId ||
    runtimeStore.activeTurnId ||
    (runtimeStore.turnTraceHistory[runtimeStore.turnTraceHistory.length - 1]?.turnId ?? null);

  if (targetTurnId) {
    const existingTimeline = runtimeStore.resolveTurnTraceTimeline(targetTurnId);
    const updatedTimeline = existingTimeline ? [...existingTimeline] : [];

    const targetCallId = ask?.callId;
    let entryFound = false;
    if (targetCallId) {
      for (const entry of updatedTimeline) {
        if (
          entry.kind === "call_tool" &&
          (entry.id === targetCallId ||
            entry.toolActivities?.some(
              (act) => act.id === targetCallId || act.name === "ask_user" || act.name === "ask"
            ))
        ) {
          entry.state = "error";
          entry.error = msg;
          entryFound = true;
          break;
        }
      }
    }

    if (!entryFound) {
      const nextSequence = updatedTimeline.length + 1;
      updatedTimeline.push({
        id: `ask-error-${Date.now()}`,
        kind: "call_tool",
        label: "用户提问交互",
        state: "error",
        sequence: nextSequence,
        error: msg
      });
    }

    runtimeStore.commitTurnTraceTimeline(targetTurnId, updatedTimeline, {
      error: msg
    });
  }
}

watch(
  () => askStore.error,
  (newError) => {
    if (newError && newError.trim() && activeAsk.value) {
      recordAskErrorToTrace(activeAsk.value, newError.trim());
    }
  }
);

async function submitAskAnswer() {
  const ask = activeAsk.value;
  if (!ask || askIsBusy.value) return;

  let answerValue: string | null = null;
  if (askOptions.value.length > 0) {
    if (
      selectedOptionIndex.value != null &&
      selectedOptionIndex.value >= 0 &&
      selectedOptionIndex.value < askOptions.value.length
    ) {
      const opt = askOptions.value[selectedOptionIndex.value];
      answerValue = typeof opt.raw === "string" ? opt.raw : opt.label;
    } else if (
      selectedOptionIndex.value === askOptions.value.length ||
      customOptionInput.value.trim().length > 0
    ) {
      answerValue = customOptionInput.value.trim() || null;
    }
  } else {
    answerValue = askTextareaInput.value.trim() || null;
  }

  if (answerValue === null && askOptions.value.length > 0) {
    return;
  }

  try {
    const result = await askStore.answer(ask, answerValue);
    if (!result && askStore.error) {
      recordAskErrorToTrace(ask, askStore.error);
    }
  } catch (error) {
    const message = `应答异常：${String(error)}`;
    recordAskErrorToTrace(ask, message);
  }
}

async function cancelActiveAsk() {
  const ask = activeAsk.value;
  if (!ask || askIsBusy.value) return;
  try {
    const result = await askStore.cancel(ask);
    if (!result && askStore.error) {
      recordAskErrorToTrace(ask, askStore.error);
    }
  } catch (error) {
    const message = `取消异常：${String(error)}`;
    recordAskErrorToTrace(ask, message);
  }
}

onMounted(() => {
  window.addEventListener("click", handleClickOutside);
  window.addEventListener("resize", updateFloatingMenuPosition);
  window.addEventListener("scroll", updateFloatingMenuPosition, true);
  void askStore.refresh();
  askStore.startPolling();
  void askStore.startWatchingEvents();
});

onBeforeUnmount(() => {
  askStore.stopPolling();
  askStore.stopWatchingEvents();
  if (modelSearchDebounceTimer) {
    clearTimeout(modelSearchDebounceTimer);
  }
  window.removeEventListener("click", handleClickOutside);
  window.removeEventListener("resize", updateFloatingMenuPosition);
  window.removeEventListener("scroll", updateFloatingMenuPosition, true);
});
</script>

<template>
  <div class="absolute bottom-0 left-0 right-0 z-30 px-4 py-3 sm:px-5 pointer-events-none">
    <div
      :ref="setComposerShellRef"
      class="relative mx-auto w-full max-w-[46.4rem] rounded-[0.6rem] bg-white/76 px-4 py-3 shadow-[0_-4px_20px_-2px_rgba(60,40,20,0.06)] backdrop-blur-[8px]"
      data-testid="workspace-composer-shell"
      style="pointer-events: auto;"
    >
      <!-- ASK 模式：直接在对话框的面板中实现，隐藏输入框和下方 action 栏 -->
      <div v-if="activeAsk" data-testid="composer-ask-container" class="flex flex-col p-2.5 sm:p-3">
        <!-- 问题 -->
        <div data-testid="composer-ask-prompt" class="text-[13px] font-semibold leading-[1.6] text-stone-800">
          {{ activeAsk.prompt || "请回答以下问题：" }}
        </div>

        <!-- 选项 / 问题回复框 -->
        <!-- 有选项的按照 1、2、3 的序列垂直排列，最后一个是用户可以输入的选项 -->
        <div
          v-if="askOptions.length > 0"
          data-testid="composer-ask-options"
          class="mt-2.5 flex flex-col space-y-1.5"
        >
          <button
            v-for="(option, index) in askOptions"
            :key="index"
            type="button"
            :data-testid="`composer-ask-option-item-${index}`"
            :class="[
              'flex items-start text-left px-3 py-2 rounded-lg text-[13px] transition-colors cursor-pointer',
              selectedOptionIndex === index
                ? 'bg-stone-200/90 text-stone-900 font-medium'
                : 'bg-stone-100/60 text-stone-700 hover:bg-stone-200/60'
            ]"
            :disabled="askIsBusy"
            @click="selectOption(index)"
            @keydown.enter.prevent="handleOptionEnter(index)"
          >
            <span class="mr-2.5 mt-0.5 flex h-4 w-4 shrink-0 items-center justify-center rounded-full text-[11px] font-mono font-medium text-stone-500">
              {{ index + 1 }}
            </span>
            <div class="flex-1 min-w-0">
              <div class="flex items-center gap-2">
                <span class="leading-snug text-stone-800">{{ option.label }}</span>
                <span
                  v-if="index === 0"
                  data-testid="composer-ask-recommended-badge"
                  class="inline-flex items-center rounded-full bg-amber-500/15 px-2 py-0.5 text-[10px] font-semibold text-amber-800 select-none"
                >
                  推荐
                </span>
              </div>
              <div
                v-if="option.description"
                data-testid="composer-ask-option-desc"
                class="mt-0.5 text-[11px] leading-normal text-stone-400 select-none"
              >
                {{ option.description }}
              </div>
            </div>
          </button>

          <!-- 最后一项：用户可以输入的选项 -->
          <div
            :class="[
              'flex items-start px-3 py-2 rounded-lg text-[13px] transition-colors cursor-text',
              selectedOptionIndex === askOptions.length
                ? 'bg-stone-200/90 text-stone-900 font-medium'
                : 'bg-stone-100/60 text-stone-700 hover:bg-stone-200/60'
            ]"
            @click="selectedOptionIndex = askOptions.length"
          >
            <span class="mr-2.5 mt-0.5 flex h-4 w-4 shrink-0 items-center justify-center rounded-full text-[11px] font-mono font-medium text-stone-500">
              {{ askOptions.length + 1 }}
            </span>
            <div class="flex-1 min-w-0">
              <input
                v-model="customOptionInput"
                data-testid="composer-ask-custom-input"
                type="text"
                placeholder="输入其他内容…"
                :disabled="askIsBusy"
                class="w-full bg-transparent text-[13px] leading-snug text-stone-800 placeholder:text-stone-400 outline-none"
                @focus="selectedOptionIndex = askOptions.length"
                @input="selectedOptionIndex = askOptions.length"
                @keydown.enter.prevent="submitAskAnswer"
              />
              <div
                data-testid="composer-ask-custom-desc"
                class="mt-0.5 text-[11px] leading-normal text-stone-400 select-none"
              >
                手动输入补充说明或专属方案
              </div>
            </div>
          </div>
        </div>

        <!-- 用户直接回答没有选项的则是 textarea -->
        <div v-else class="mt-2.5">
          <textarea
            v-model="askTextareaInput"
            data-testid="composer-ask-textarea"
            rows="3"
            placeholder="输入您的回答，按 Enter 发送，Shift+Enter 换行…"
            :disabled="askIsBusy"
            class="w-full resize-none rounded-lg bg-stone-100/50 p-2.5 text-[13px] leading-[1.6] text-stone-800 placeholder:text-stone-400 outline-none transition"
            @keydown.enter.exact.prevent="submitAskAnswer"
          />
        </div>

        <!-- 底部栏：极简风格，取消与复用原对话框发送键 -->
        <div class="mt-3 flex items-center justify-between">
          <div class="flex items-center gap-2">
            <button
              type="button"
              data-testid="composer-ask-cancel"
              class="text-[12px] text-stone-400 transition hover:text-stone-600 disabled:opacity-40"
              :disabled="askIsBusy"
              @click="cancelActiveAsk"
            >
              <LoaderCircle v-if="askStore.cancellingRequestId === activeAsk.requestId" class="mr-1 inline h-3 w-3 animate-spin" />
              取消
            </button>
            <span v-if="askStore.error" class="text-[11px] text-rose-600">
              {{ askStore.error }}
            </span>
          </div>

          <!-- 复用极简的原对话框发送键 -->
          <Button
            class="h-8 w-8 rounded-full p-0"
            size="sm"
            :disabled="askSubmitDisabled"
            title="确认发送回答 (Enter)"
            data-testid="workspace-submit-action"
            @click="submitAskAnswer"
          >
            <LoaderCircle v-if="askStore.answeringRequestId === activeAsk.requestId" class="h-3.5 w-3.5 animate-spin" />
            <ArrowUp v-else class="h-3.5 w-3.5" />
            <span class="sr-only">发送</span>
          </Button>
        </div>
      </div>

      <!-- 常规普通模式：原输入框与下方 action 栏 -->
      <template v-else>
        <textarea
          :value="draftMessage"
          :disabled="Boolean(sessionOperation)"
          data-testid="workspace-composer-input"
        class="min-h-[82px] w-full resize-none bg-transparent px-0 py-0 text-[13px] leading-[1.55] text-stone-800 outline-none placeholder:text-[12px] placeholder:font-normal placeholder:tracking-[0.01em] placeholder:text-stone-400/70"
        placeholder="输入消息，按 Enter 发送，Shift+Enter 换行。"
        @input="runtimeStore.setDraftMessage(($event.target as HTMLTextAreaElement).value)"
        @keydown="handleComposerKeydown"
      />

      <div v-if="pendingAttachments.length" class="mt-2 flex flex-wrap items-center gap-1.5">
        <div
          v-for="(attachment, index) in pendingAttachments"
          :key="attachment.id"
          class="flex max-w-full items-center gap-1.5 rounded-full border px-2 py-0.5 text-[11px]"
          :class="attachment.status === 'error'
            ? 'border-red-200 bg-red-50 text-red-600'
            : 'border-stone-200/80 bg-stone-100/80 text-stone-700'"
          :data-testid="`workspace-attachment-chip-${index}`"
          :title="attachment.errorDetail ?? attachment.path ?? ''"
        >
          <span class="truncate">{{ attachment.name }}</span>
          <span class="shrink-0 text-stone-400">{{ formatAttachmentSize(attachment.sizeBytes) }}</span>
          <button
            class="shrink-0 text-stone-400 transition-colors hover:text-stone-600"
            type="button"
            :data-testid="`workspace-attachment-remove-${index}`"
            aria-label="移除附件"
            @click="removePendingAttachment(attachment.id)"
          >
            <X class="h-3 w-3" />
          </button>
        </div>
      </div>
      <p
        v-if="attachNotice"
        class="mt-1.5 text-[11px] leading-4 text-amber-600"
        data-testid="workspace-attach-notice"
      >
        {{ attachNotice }}
      </p>

      <div class="mt-3 flex flex-wrap items-center justify-between gap-x-3 gap-y-2 border-t border-stone-200/70 pt-2.5">
        <div class="flex min-w-0 flex-wrap items-center gap-2">
          <TooltipRoot :delay-duration="300">
            <TooltipTrigger as-child>
              <span tabindex="0" class="inline-flex">
                <button
                  class="composer-trigger"
                  type="button"
                  :disabled="isSubmitting"
                  data-testid="workspace-attach-button"
                  @click="handleAttach"
                >
                  <Paperclip class="h-3 w-3" />
                  <span class="sr-only">添加附件</span>
                </button>
              </span>
            </TooltipTrigger>
            <TooltipPortal>
              <TooltipContent side="top" :side-offset="4" class="z-50 overflow-hidden rounded-md border border-stone-200 bg-white px-3 py-1.5 text-xs text-stone-700 shadow-sm">
                {{ isSubmitting ? "等待当前轮次结束后再添加附件" : "添加附件" }}
              </TooltipContent>
            </TooltipPortal>
          </TooltipRoot>

          <!-- 授权/权限模式选择器 -->
          <div class="relative">
            <TooltipRoot :delay-duration="300">
              <TooltipTrigger as-child>
                <button
                  ref="authTriggerRef"
                  class="composer-trigger flex items-center gap-1.5 rounded-md px-2 py-0.5 text-[11px] font-normal text-stone-600 hover:text-stone-900 bg-stone-100/70 hover:bg-stone-200/60 transition"
                  type="button"
                  data-testid="workspace-auth-mode-trigger"
                  @click.stop="toggleAuthMenu"
                >
                  <PermissionModeIcon
                    :mode="runtimeStore.toolAuthorizationMode ?? 'workspace_write'"
                    :size="13"
                    class="shrink-0"
                    :class="(runtimeStore.toolAuthorizationMode === 'full_access') ? 'text-amber-700/80' : 'text-stone-500'"
                  />
                  <span :class="(runtimeStore.toolAuthorizationMode === 'full_access') ? 'text-amber-800 font-medium' : ''">{{ currentAuthModeLabel }}</span>
                  <ChevronDown
                    class="h-2.5 w-2.5 shrink-0 text-stone-400"
                    :class="{ 'rotate-180': authMenuOpen }"
                  />
                </button>
              </TooltipTrigger>
              <TooltipPortal>
                <TooltipContent side="top" :side-offset="4" class="z-50 overflow-hidden rounded-md border border-stone-200 bg-white px-3 py-1.5 text-xs text-stone-700 shadow-sm">
                  权限模式：点击切换
                </TooltipContent>
              </TooltipPortal>
            </TooltipRoot>
          </div>
        </div>

        <div class="flex min-w-0 flex-wrap items-center gap-2">
          <!-- 上下文窗口圆形进度条 (模型左侧) -->
          <TooltipRoot :delay-duration="200">
            <TooltipTrigger as-child>
              <span tabindex="0" class="inline-flex">
                <div
                  class="flex items-center justify-center p-0.5 text-stone-400 hover:text-stone-600 transition-colors cursor-help"
                  data-testid="workspace-context-usage-circle"
                  aria-label="上下文用量"
                >
                  <svg class="h-3.5 w-3.5 -rotate-90 transform" viewBox="0 0 20 20">
                    <circle
                      cx="10"
                      cy="10"
                      r="7.5"
                      fill="none"
                      stroke="currentColor"
                      stroke-width="2.4"
                      class="opacity-20"
                    />
                    <circle
                      cx="10"
                      cy="10"
                      r="7.5"
                      fill="none"
                      stroke="currentColor"
                      stroke-width="2.4"
                      stroke-linecap="round"
                      :stroke-dasharray="2 * Math.PI * 7.5"
                      :stroke-dashoffset="(2 * Math.PI * 7.5) * (1 - contextUsagePercentage / 100)"
                      class="transition-all duration-500 ease-out text-stone-500 hover:text-stone-700"
                    />
                  </svg>
                </div>
              </span>
            </TooltipTrigger>
            <TooltipPortal>
              <TooltipContent side="top" :side-offset="4" class="z-50 overflow-hidden rounded-md border border-stone-200 bg-white px-2.5 py-1 text-xs text-stone-700 shadow-sm">
                {{ contextUsageTooltip }}
              </TooltipContent>
            </TooltipPortal>
          </TooltipRoot>

          <div ref="menuRef" class="relative">
            <TooltipRoot :delay-duration="300">
              <TooltipTrigger as-child>
                <span tabindex="0" class="inline-flex">
                  <button
                    ref="modelTriggerRef"
                    class="composer-trigger composer-model-trigger"
                    type="button"
                    data-testid="workspace-model-menu-trigger"
                    @click.stop="toggleMenu"
                  >
                    <span class="truncate">{{ currentModel ? modelDisplayName(currentModel) : "选择模型" }}</span>
                    <span v-if="currentModel" class="model-effort-hint">{{ reasoningSummary }}</span>
                    <ChevronDown
                      class="h-2.5 w-2.5 shrink-0 text-stone-400"
                      :class="{ 'rotate-180': menuOpen }"
                    />
                  </button>
                </span>
              </TooltipTrigger>
              <TooltipPortal>
                <TooltipContent side="top" :side-offset="4" class="z-50 overflow-hidden rounded-md border border-stone-200 bg-white px-3 py-1.5 text-xs text-stone-700 shadow-sm">
                  <div class="flex flex-col gap-0.5">
                    <span class="font-medium text-stone-700">模型和思考强度</span>
                    <span class="text-stone-500">{{ providerTooltipDetail }}</span>
                  </div>
                </TooltipContent>
              </TooltipPortal>
            </TooltipRoot>
          </div>

          <!-- 权限模式下拉浮窗 -->
          <Teleport to="body">
            <div
              v-if="authMenuOpen"
              ref="authFloatingMenuRef"
              class="composer-menu-panel fixed z-[70] min-w-[14.5rem] p-1 shadow-lg border border-stone-200/90 rounded-lg bg-white"
              :style="authMenuStyle"
            >
              <div class="px-2 py-1 text-[11px] font-semibold text-stone-500 border-b border-stone-100">
                权限模式
              </div>
              <div class="py-1 space-y-0.5">
                <button
                  v-for="item in authModes"
                  :key="item.id"
                  type="button"
                  class="w-full flex items-center justify-between gap-2.5 px-2 py-1.5 rounded text-left transition"
                  :class="[
                    runtimeStore.toolAuthorizationMode === item.id
                      ? (item.id === 'full_access' ? 'text-amber-900 font-medium' : 'text-stone-900 font-medium')
                      : 'text-stone-600'
                  ]"
                  :data-testid="`workspace-auth-mode-item-${item.id}`"
                  @click="selectAuthMode(item.id)"
                >
                  <div class="flex items-start gap-2 min-w-0">
                    <PermissionModeIcon
                      :mode="item.id"
                      :size="15"
                      class="mt-0.5 shrink-0"
                      :class="item.id === 'full_access' ? 'text-amber-700/80' : 'text-stone-500'"
                    />
                    <div class="flex flex-col min-w-0">
                      <span class="text-[12px] leading-4">{{ item.label }}</span>
                      <span class="text-[10px] text-stone-400 leading-tight">{{ item.desc }}</span>
                    </div>
                  </div>
                  <Check
                    v-if="runtimeStore.toolAuthorizationMode === item.id"
                    class="h-3.5 w-3.5 shrink-0"
                    :class="item.id === 'full_access' ? 'text-amber-700' : 'text-stone-700'"
                  />
                </button>
              </div>
            </div>
          </Teleport>

          <!-- 完全权限风险警示确认弹窗 -->
          <DialogRoot v-model:open="fullAccessConfirmOpen">
            <DialogPortal>
              <DialogOverlay class="fixed inset-0 z-[80] bg-black/40 backdrop-blur-sm transition-opacity" />
              <DialogContent class="fixed left-1/2 top-1/2 z-[85] max-h-[85vh] w-[90vw] max-w-[26rem] -translate-x-1/2 -translate-y-1/2 rounded-xl bg-white p-5 shadow-2xl border border-stone-200 focus:outline-none">
                <div class="flex items-start gap-3">
                  <div class="flex h-10 w-10 shrink-0 items-center justify-center rounded-full bg-red-100 text-red-600">
                    <AlertTriangle class="h-5 w-5" />
                  </div>
                  <div class="flex-1 min-w-0">
                    <DialogTitle class="text-[15px] font-semibold text-stone-900">
                      确认启用完全权限？
                    </DialogTitle>
                    <DialogDescription class="mt-2 text-[12.5px] leading-relaxed text-stone-600">
                      启用完全权限后，智能体将减少确认步骤，并且可以直接执行更多操作，包括敏感操作、工作区外的文件修改或执行外部宿主命令。仅建议在你完全信任当前任务时使用。
                    </DialogDescription>
                  </div>
                </div>
                <div class="mt-5 flex items-center justify-end gap-2.5">
                  <button
                    type="button"
                    class="rounded-lg px-3 py-1.5 text-xs font-medium text-stone-600 hover:bg-stone-100 transition"
                    @click="cancelFullAccess"
                  >
                    取消
                  </button>
                  <button
                    type="button"
                    class="rounded-lg bg-red-600 px-3.5 py-1.5 text-xs font-medium text-white hover:bg-red-700 active:bg-red-800 transition shadow-sm"
                    data-testid="confirm-full-access-button"
                    @click="confirmFullAccess"
                  >
                    启用完全权限
                  </button>
                </div>
              </DialogContent>
            </DialogPortal>
          </DialogRoot>

          <Teleport to="body">
            <div
              v-if="menuOpen"
              ref="floatingMenuRef"
              class="composer-menu-panel fixed z-[70] min-w-[14rem]"
              :style="floatingMenuStyle"
            >
              <div class="py-0.5">
                <div class="relative">
                  <button
                    class="composer-menu-item"
                    type="button"
                    data-menu-item="model"
                    @mouseenter="onModelEnter"
                    @focus="onModelFocus"
                  >
                    <span>模型</span>
                    <div class="flex items-center gap-2">
                      <span class="truncate text-[10px] text-stone-400">{{ currentModel ? modelDisplayName(currentModel) : "未选择" }}</span>
                      <ChevronDown class="h-3.5 w-3.5 -rotate-90 text-stone-400" />
                    </div>
                  </button>
                  <div
                    v-if="modelMenuOpen"
                    ref="modelSubmenuRef"
                    :class="[
                      'composer-menu-panel absolute z-50 min-w-[15rem]',
                      modelSubmenuDirection === 'right' ? 'left-full ml-1' : 'right-full mr-1'
                    ]"
                    :style="modelSubmenuStyle"
                  >
                    <div class="composer-search-field mx-2 mt-1.5 mb-1">
                      <Search class="composer-search-icon" aria-hidden="true" />
                      <input
                        v-model="modelSearchQuery"
                        type="text"
                        placeholder="搜索模型名称或 ID"
                        data-testid="workspace-model-search"
                        class="composer-search-input"
                      />
                    </div>
                    <div class="composer-menu-scroll">
                      <template v-for="provider in filteredProviders" :key="provider.id">
                        <div class="composer-provider-caption">{{ provider.name }}</div>
                        <button
                          v-for="model in provider.models ?? []"
                          :key="model.id"
                          class="composer-menu-item"
                          type="button"
                          @click="selectModel(provider.id, model.id)"
                        >
                          <span class="truncate">{{ modelDisplayName(model) }}</span>
                          <Check
                            v-if="currentProvider?.id === provider.id && currentModel?.id === model.id"
                            class="h-3.5 w-3.5 shrink-0 text-stone-700"
                          />
                        </button>
                      </template>
                      <div
                        v-if="!filteredProviders.length"
                        class="px-3 py-2 text-[11px] leading-5 text-stone-400"
                      >
                        未找到匹配的模型
                      </div>
                    </div>
                  </div>
                </div>

                <div class="relative">
                  <button
                    class="composer-menu-item"
                    type="button"
                    data-menu-item="reasoning"
                    @mouseenter="onReasoningEnter"
                    @focus="onReasoningFocus"
                  >
                    <span>思考</span>
                    <div class="flex items-center gap-2">
                      <span class="truncate text-[10px] text-stone-400">{{ reasoningSummary }}</span>
                      <ChevronDown class="h-3.5 w-3.5 -rotate-90 text-stone-400" />
                    </div>
                  </button>
                  <div
                    v-if="reasoningMenuOpen"
                    ref="reasoningSubmenuRef"
                    :class="[
                      'composer-menu-panel absolute z-50 min-w-[10rem]',
                      reasoningSubmenuDirection === 'right' ? 'left-full ml-1' : 'right-full mr-1'
                    ]"
                    :style="reasoningSubmenuStyle"
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
                  </div>
                </div>

                <div
                  class="composer-menu-item"
                  role="menuitem"
                  tabindex="0"
                  data-testid="reasoning-visibility-toggle"
                  @click="toggleReasoningVisibility"
                  @keydown.enter="toggleReasoningVisibility"
                >
                  <div class="flex min-w-0 flex-col">
                    <span>显示思考</span>
                    <span class="composer-menu-item-hint">
                      {{ showReasoningContent ? "已开启" : "已关闭" }}
                    </span>
                  </div>
                  <span class="shrink-0" @click.stop>
                    <Switch
                      :model-value="showReasoningContent"
                      data-testid="reasoning-visibility-switch"
                      @update:model-value="handleShowReasoningSwitch"
                    />
                  </span>
                </div>
              </div>
            </div>
          </Teleport>

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

          <Button
            class="h-8 w-8 rounded-full p-0"
            size="sm"
            :disabled="primaryActionDisabled"
            :title="primaryActionTitle"
            :data-testid="isSubmitting && !draftMessage.trim() ? 'workspace-stop-turn' : 'workspace-submit-action'"
            @click="handlePrimaryAction"
          >
            <Square v-if="isSubmitting && !draftMessage.trim()" class="h-3.5 w-3.5 fill-current" />
            <ArrowUp v-else class="h-3.5 w-3.5" />
            <span class="sr-only">{{ isSubmitting && !draftMessage.trim() ? "停止" : composerAction.label }}</span>
          </Button>
        </div>
      </div>
      </template>
    </div>
  </div>
</template>

<style scoped>
.composer-trigger {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 0.3rem;
  height: 1.35rem;
  min-width: 1.35rem;
  border: none;
  border-radius: 0.25rem;
  background: transparent;
  padding: 0 0.3rem;
  font-size: 11px;
  font-weight: 500;
  line-height: 1;
  color: rgb(168 162 158);
  cursor: pointer;
  transition:
    color 0.15s ease,
    background-color 0.15s ease,
    transform 0.1s ease;
}

.composer-trigger:hover {
  color: rgb(87 83 78);
  background: rgba(0, 0, 0, 0.04);
}

.composer-trigger:active {
  transform: scale(0.9);
}

.composer-trigger:disabled {
  cursor: not-allowed;
  opacity: 0.3;
}

.composer-trigger:disabled:hover {
  background: transparent;
  color: rgb(168 162 158);
}

.composer-trigger:disabled:active {
  transform: none;
}

.composer-trigger:focus-visible {
  box-shadow: 0 0 0 2px rgba(231, 229, 228, 0.95);
}

.composer-trigger.composer-model-trigger {
  max-width: 22rem;
  padding: 0 0.45rem;
}

.composer-trigger.composer-model-trigger > .truncate {
  max-width: 16rem;
  color: rgb(87 83 78);
}

.composer-trigger.composer-model-trigger:hover > .truncate {
  color: rgb(60 56 52);
}

.model-effort-hint {
  font-size: 10px;
  font-weight: 400;
  line-height: 1;
  color: rgb(168 162 158);
}

.composer-search-field {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  border-bottom: 1px solid rgba(214, 211, 209, 0.85);
  color: rgb(168 162 158);
  transition:
    border-color 0.15s ease,
    box-shadow 0.15s ease,
    color 0.15s ease;
}

.composer-search-field:focus-within {
  border-color: rgba(139, 94, 52, 0.72);
  box-shadow: 0 4px 8px -7px rgba(139, 94, 52, 0.8);
  color: rgb(139 94 52);
}

.composer-search-icon {
  height: 0.8rem;
  width: 0.8rem;
  flex: none;
}

.composer-search-input {
  width: 100%;
  height: 1.6rem;
  border: none;
  background: transparent;
  padding: 0;
  font-size: 11px;
  line-height: 1;
  color: rgb(68 64 60);
  outline: none;
}

.composer-search-input::placeholder {
  color: rgb(168 162 158);
}

.composer-menu-scroll {
  max-height: 16.5rem;
  overflow-y: auto;
  overflow-x: hidden;
  scrollbar-width: none;
  -ms-overflow-style: none;
  flex: 1 1 auto;
  min-height: 0;
}

.composer-menu-scroll::-webkit-scrollbar {
  display: none;
}

.composer-provider-caption {
  padding: 0.6rem 0.9rem 0.3rem;
  font-size: 10px;
  font-weight: 600;
  line-height: 1;
  color: rgb(168 162 158);
}

.composer-provider-caption:first-child {
  padding-top: 0.25rem;
}

.composer-provider-caption:not(:first-child) {
  margin-top: 0.2rem;
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
</style>
