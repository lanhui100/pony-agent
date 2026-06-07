<script setup lang="ts">
import { computed, ref } from "vue";
import { storeToRefs } from "pinia";
import {
  Activity,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  CircleAlert,
  MessageSquareMore,
  Plus,
  Settings2,
  Trash2
} from "lucide-vue-next";
import PonyBrandIcon from "@/components/PonyBrandIcon.vue";
import ScrollArea from "@/components/ui/ScrollArea.vue";
import { useRuntimeStore } from "@/stores/runtime";
import type { ChatMessage, SessionOverview } from "@/types/runtime";

const SESSION_SIDEBAR_STORAGE_KEY = "pony-agent.session-sidebar-collapsed.v1";
const HISTORY_OPEN_STORAGE_KEY = "pony-agent.session-sidebar-history-open.v1";
const MODEL_OPEN_STORAGE_KEY = "pony-agent.session-sidebar-model-open.v1";

type NavigationPage = "home" | "providers" | "model-monitor";
type SectionKey = "history" | "model";
type FeedbackTone = "info" | "warning" | "success";
type ExplainabilityTone = "info" | "warning" | "success";

const props = withDefaults(
  defineProps<{
    currentPage?: NavigationPage;
  }>(),
  {
    currentPage: "home"
  }
);

const emit = defineEmits<{
  (event: "navigate", page: NavigationPage): void;
}>();

const runtimeStore = useRuntimeStore();
const {
  activeBranchId,
  branchHeadNodeId,
  historyBranches,
  historyCursorMode,
  historyNodes,
  isSubmitting,
  latestExecutionCheckpoint,
  latestGraphRunControlBoundaryEvidence,
  latestRunControlAuditSummary,
  latestHistoryStateAuditSummary,
  latestGraphRunSubmissionPlan,
  messages,
  phase,
  sessionId,
  sessionList,
  sessionOperation,
  visibleNodeId
} = storeToRefs(runtimeStore);

const collapsed = ref(loadStoredBoolean(SESSION_SIDEBAR_STORAGE_KEY, false));
const historyOpen = ref(loadStoredBoolean(HISTORY_OPEN_STORAGE_KEY, true));
const modelOpen = ref(loadStoredBoolean(MODEL_OPEN_STORAGE_KEY, true));
const pendingDeleteSessionId = ref<string | null>(null);
const historyFeedback = ref<{
  tone: FeedbackTone;
  title: string;
  text: string;
} | null>(null);
const menuInteractiveClass =
  "rounded-[0.2rem] transition-colors hover:bg-[#f6dfb8] hover:text-stone-900";
const menuSelectedClass = "rounded-[0.2rem] bg-[#f3c98d] text-stone-900";

const hasPersistableCurrentSession = computed(() => hasPersistableMessages(messages.value));
const canCreateSession = computed(
  () => !isSubmitting.value && !sessionOperation.value && hasPersistableCurrentSession.value
);
const createSessionTitle = computed(() =>
  hasPersistableCurrentSession.value
    ? "新建一个空白对话，并保留当前已存在的历史会话。"
    : "当前已经是空白新对话，发送首条消息后才会保存到历史。"
);

const visibleSessions = computed<SessionOverview[]>(() => {
  if (hasPersistableCurrentSession.value) {
    return sessionList.value;
  }

  return [
    {
      conversationId: sessionId.value,
      title: "新对话",
      summary: "发送第一条消息后保存到历史",
      turnCount: 0,
      lastReferencedFile: null,
      updatedAtMs: 0
    },
    ...sessionList.value.filter((session) => session.conversationId !== sessionId.value)
  ];
});

const sortedHistoryNodes = computed(() =>
  [...historyNodes.value].sort((left, right) => right.createdAtMs - left.createdAtMs).slice(0, 8)
);

const sortedHistoryBranches = computed(() =>
  [...historyBranches.value].sort((left, right) => right.updatedAtMs - left.updatedAtMs)
);

const canManageHistory = computed(
  () => !isSubmitting.value && !sessionOperation.value && historyNodes.value.length > 0
);

const hasHistoryNodes = computed(() => historyNodes.value.length > 0);

const isHistoricalMode = computed(() => historyCursorMode.value !== "live");

const historyModeMeta = computed(() => {
  switch (historyCursorMode.value) {
    case "historical":
      return {
        label: "历史浏览",
        summary: "当前正在查看历史节点；恢复后会回到最新分支头。",
        badgeClass: "bg-amber-100 text-amber-700"
      };
    case "historical_dirty":
      return {
        label: "历史分叉待处理",
        summary: "当前视图已偏离最新分支头；请恢复、分叉或切换分支后继续。",
        badgeClass: "bg-rose-100 text-rose-700"
      };
    default:
      return {
        label: "最新节点",
        summary: "当前会话位于最新分支头，可直接开始下一轮执行。",
        badgeClass: "bg-emerald-100 text-emerald-700"
      };
  }
});

const sessionControlMeta = computed(() => {
  if (isSubmitting.value) {
    return {
      label: "运行中",
      summary: "当前回合正在执行，可在工作区显式停止并等待安全边界暂停。",
      badgeClass: "bg-sky-100 text-sky-700"
    };
  }

  const actionSummary = latestRunControlAuditSummary.value?.actionEvidenceSummary ?? null;
  const currentContext = latestRunControlAuditSummary.value?.currentContextProjection ?? null;
  const planCommand = latestGraphRunSubmissionPlan.value?.command?.trim().toLowerCase() || null;
  const checkpointCommand = latestExecutionCheckpoint.value?.submissionCommand?.trim().toLowerCase() || null;
  const projectedCommand = actionSummary?.projectedCommand?.trim().toLowerCase() || null;
  const command = projectedCommand || planCommand || checkpointCommand;
  const checkpoint = latestExecutionCheckpoint.value;

  if (actionSummary?.commandKind === "stop_graph_run") {
    return {
      label: actionSummary.blocked ? "停止受阻" : "已请求停止",
      summary:
        actionSummary.summary?.trim() ||
        "最近一次会话控制动作请求停止当前运行，等待下次提交时决定恢复或重放。",
      badgeClass: actionSummary.blocked ? "bg-amber-100 text-amber-700" : "bg-amber-100 text-amber-700"
    };
  }

  if (command === "resume_graph_run_stream") {
    return {
      label: "可恢复",
      summary:
        actionSummary?.summary?.trim() ||
        "存在暂停中的运行；下一次发送会恢复该 run 并继续推进。",
      badgeClass: "bg-sky-100 text-sky-700"
    };
  }

  if (command === "continue_graph_run_stream") {
    return {
      label: "可继续",
      summary:
        actionSummary?.summary?.trim() ||
        "存在可继续的 graph run；下一次发送会接着当前运行推进。",
      badgeClass: "bg-violet-100 text-violet-700"
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
      label: "需重新开始",
      summary:
        actionSummary?.summary?.trim() ||
        "当前恢复点只保留持久化事实；下一次发送会重新开始新的执行。",
      badgeClass: "bg-stone-200 text-stone-700"
    };
  }

  if (currentContext?.phase?.trim().toLowerCase() === "paused") {
    return {
      label: "可恢复",
      summary: "当前运行停在安全边界；下一次发送可继续恢复。",
      badgeClass: "bg-sky-100 text-sky-700"
    };
  }

  if (phase.value === "cancelled") {
    return {
      label: "已停止",
      summary: "上一轮已停止；可在工作区选择恢复继续或重新开始。",
      badgeClass: "bg-amber-100 text-amber-700"
    };
  }

  return {
    label: "当前会话",
    summary: "未检测到暂停或恢复点；下一次发送会启动新的执行。",
    badgeClass: "bg-emerald-100 text-emerald-700"
  };
});

const latestControlBoundaryEvidenceText = computed(() => {
  const actionSummary = latestRunControlAuditSummary.value?.actionEvidenceSummary ?? null;
  const currentContext = latestRunControlAuditSummary.value?.currentContextProjection ?? null;

  if (actionSummary?.summary?.trim()) {
    const badge = actionSummary.status?.trim().toLowerCase() === "missing" ? "控制证据缺失" : "控制摘要";
    const detail = [actionSummary.commandKind, actionSummary.boundary, actionSummary.resultKind]
      .filter(Boolean)
      .join(" · ");
    const contextDetail = [
      currentContext?.phase ? `phase ${currentContext.phase}` : null,
      currentContext?.submissionPlanCommand ? `next ${currentContext.submissionPlanCommand}` : null
    ]
      .filter(Boolean)
      .join(" · ");
    return [badge, actionSummary.summary.trim(), detail || contextDetail].filter(Boolean).join("：");
  }

  const latestEvidence = latestGraphRunControlBoundaryEvidence.value[0];

  if (!latestEvidence) {
    return "";
  }

  const summary = latestEvidence.summary?.trim();
  if (summary) {
    return `边界依据：${summary}`;
  }

  return `边界依据：${latestEvidence.canonicalPhase} · ${latestEvidence.hookPoint}`;
});

const historyActionEvidenceMeta = computed(() => {
  const action = latestHistoryStateAuditSummary.value?.action;

  if (!action) {
    return null;
  }

  const normalizedStatus = action.status?.trim().toLowerCase() || "";
  let badge = "已记录";
  let tone: ExplainabilityTone = "success";

  if (action.blocked) {
    badge = "已阻断";
    tone = "warning";
  } else if (action.degraded) {
    badge = "已降级";
    tone = "warning";
  } else if (normalizedStatus === "missing") {
    badge = "证据缺失";
    tone = "warning";
  } else if (normalizedStatus && normalizedStatus !== "available") {
    badge = normalizedStatus;
    tone = "info";
  }

  return {
    badge,
    tone,
    summary: action.summary?.trim() || "最近一次 history-control 动作尚未返回可展示的证据摘要。",
    detail: [action.commandKind, action.boundary, action.resultKind].filter(Boolean).join(" · ")
  };
});

const historyCurrentContextMeta = computed(() => {
  const currentContext = latestHistoryStateAuditSummary.value?.currentContext ?? {
    mode: historyCursorMode.value,
    visibleNodeId: visibleNodeId.value,
    activeBranchId: activeBranchId.value,
    branchHeadNodeId: branchHeadNodeId.value
  };

  return {
    summary: `模式 ${formatHistoryCursorMode(currentContext.mode)} · 分支 ${
      currentContext.activeBranchId || "main"
    } · 可见 ${currentContext.visibleNodeId || "latest"}`,
    detail: [
      currentContext.branchHeadNodeId ? `分支头 ${currentContext.branchHeadNodeId}` : null,
      currentContext.workspaceNodeId ? `工作区 ${currentContext.workspaceNodeId}` : null
    ]
      .filter(Boolean)
      .join(" · ")
  };
});

const historyCheckoutDisabledReason = computed(() => {
  if (isSubmitting.value) {
    return "运行中不可切换历史节点，请先停止当前执行。";
  }
  if (sessionOperation.value) {
    return "当前正在处理会话切换或删除，暂时不可切换历史节点。";
  }
  if (!hasHistoryNodes.value) {
    return "当前会话还没有可回退的历史节点。";
  }
  return null;
});

const historyRestoreDisabledReason = computed(() => {
  if (isSubmitting.value) {
    return "运行中不可恢复到分支头，请先停止当前执行。";
  }
  if (sessionOperation.value) {
    return "当前正在处理会话切换或删除，暂时不可恢复到分支头。";
  }
  if (!hasHistoryNodes.value) {
    return "当前会话还没有可恢复的历史节点。";
  }
  if (!isHistoricalMode.value) {
    return "只有在历史浏览或历史分叉状态下，才需要恢复到分支头。";
  }
  return null;
});

const historyForkDisabledReason = computed(() => {
  if (isSubmitting.value) {
    return "运行中不可从历史节点分叉，请先停止当前执行。";
  }
  if (sessionOperation.value) {
    return "当前正在处理会话切换或删除，暂时不可分叉。";
  }
  if (!visibleNodeId.value) {
    return "当前没有可见历史节点，无法从中创建分支。";
  }
  return null;
});

const historyBranchSwitchDisabledReason = computed(() => {
  if (isSubmitting.value) {
    return "运行中不可切换分支，请先停止当前执行。";
  }
  if (sessionOperation.value) {
    return "当前正在处理会话切换或删除，暂时不可切换分支。";
  }
  if (!sortedHistoryBranches.value.length) {
    return "当前没有可切换的历史分支。";
  }
  return null;
});

const historyActionDisabledHint = computed(
  () =>
    historyRestoreDisabledReason.value ||
    historyForkDisabledReason.value ||
    historyBranchSwitchDisabledReason.value ||
    historyCheckoutDisabledReason.value
);

const asideClass = computed(() =>
  collapsed.value
    ? "w-[3.4rem] shrink-0"
    : "w-full shrink-0 lg:w-[17.5rem] xl:w-[18.5rem]"
);

function loadStoredBoolean(key: string, fallback: boolean) {
  if (typeof window === "undefined") {
    return fallback;
  }

  const value = window.localStorage.getItem(key);
  if (value == null) {
    return fallback;
  }

  return value === "1";
}

function persistStoredBoolean(key: string, value: boolean) {
  if (typeof window === "undefined") {
    return;
  }

  window.localStorage.setItem(key, value ? "1" : "0");
}

function toggleCollapsed() {
  collapsed.value = !collapsed.value;
  persistStoredBoolean(SESSION_SIDEBAR_STORAGE_KEY, collapsed.value);
}

function toggleSection(section: SectionKey) {
  if (section === "history") {
    historyOpen.value = !historyOpen.value;
    persistStoredBoolean(HISTORY_OPEN_STORAGE_KEY, historyOpen.value);
    return;
  }

  modelOpen.value = !modelOpen.value;
  persistStoredBoolean(MODEL_OPEN_STORAGE_KEY, modelOpen.value);
}

function navigate(page: NavigationPage) {
  emit("navigate", page);
}

function openHistoryHome() {
  historyOpen.value = true;
  persistStoredBoolean(HISTORY_OPEN_STORAGE_KEY, true);
  navigate("home");
}

function handleHistoryToggle() {
  if (props.currentPage !== "home") {
    openHistoryHome();
    return;
  }

  toggleSection("history");
}

function openSessionHistory(conversationId: string) {
  pendingDeleteSessionId.value = null;
  if (props.currentPage !== "home") {
    navigate("home");
  }

  runtimeStore.switchSession(conversationId);
}

function sessionHeadline(session: SessionOverview) {
  return session.title?.trim() || session.summary?.trim() || session.conversationId;
}

function formatSessionTime(updatedAtMs?: number) {
  if (!updatedAtMs) {
    return "未保存";
  }

  const now = new Date();
  const updatedAt = new Date(updatedAtMs);
  const isSameDay =
    now.getFullYear() === updatedAt.getFullYear() &&
    now.getMonth() === updatedAt.getMonth() &&
    now.getDate() === updatedAt.getDate();

  if (isSameDay) {
    return new Intl.DateTimeFormat("zh-CN", {
      hour: "2-digit",
      minute: "2-digit"
    }).format(updatedAt);
  }

  const startOfToday = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  const startOfUpdatedDay = new Date(
    updatedAt.getFullYear(),
    updatedAt.getMonth(),
    updatedAt.getDate()
  ).getTime();
  const elapsedDays = Math.max(1, Math.floor((startOfToday - startOfUpdatedDay) / 86_400_000));
  return `${elapsedDays}天前`;
}

function hasPersistableMessages(sessionMessages: ChatMessage[]) {
  return sessionMessages.some(
    (message) =>
      (message.role === "user" || message.role === "assistant") && message.content.trim().length > 0
  );
}

function isTransientSession(session: SessionOverview) {
  return session.conversationId === sessionId.value && !hasPersistableCurrentSession.value;
}

function canDeleteSession(session: SessionOverview) {
  return !isSubmitting.value && !sessionOperation.value && !isTransientSession(session);
}

async function handleDeleteSession(session: SessionOverview) {
  if (!canDeleteSession(session)) {
    return;
  }

  if (pendingDeleteSessionId.value !== session.conversationId) {
    pendingDeleteSessionId.value = session.conversationId;
    return;
  }

  pendingDeleteSessionId.value = null;
  await runtimeStore.deleteSession(session.conversationId);
}

function formatHistoryNodeTime(createdAtMs?: number | null) {
  if (!createdAtMs) {
    return "刚刚";
  }

  return new Intl.DateTimeFormat("zh-CN", {
    month: "numeric",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit"
  }).format(createdAtMs);
}

function formatHistoryNodeKind(kind: string) {
  switch (kind) {
    case "turn_cancelled":
      return "已中止";
    case "run_paused":
      return "暂停";
    case "checkpoint":
      return "检查点";
    case "manual_snapshot":
      return "手动快照";
    default:
      return "已提交";
  }
}

function formatHistoryCursorMode(mode: string) {
  switch (mode) {
    case "historical":
      return "历史浏览";
    case "historical_dirty":
      return "历史分叉待处理";
    default:
      return "最新节点";
  }
}

function describeDegradationReason(reason?: string | null) {
  switch (reason?.trim().toLowerCase()) {
    case "workspace_rollback_unsupported":
      return "当前工作区暂不支持回滚，所以只恢复了对话";
    default:
      return "当前操作发生了降级";
  }
}

function feedbackClass(tone: FeedbackTone) {
  switch (tone) {
    case "warning":
      return "border-amber-200/80 bg-amber-50 text-amber-800";
    case "success":
      return "border-emerald-200/80 bg-emerald-50 text-emerald-800";
    default:
      return "border-stone-200/80 bg-white text-stone-700";
  }
}

function explainabilityClass(tone: ExplainabilityTone) {
  switch (tone) {
    case "warning":
      return "border-amber-200/80 bg-amber-50 text-amber-800";
    case "success":
      return "border-emerald-200/80 bg-emerald-50 text-emerald-800";
    default:
      return "border-stone-200/80 bg-stone-50 text-stone-700";
  }
}

async function handleCheckoutHistoryNode(nodeId: string) {
  if (!canManageHistory.value) {
    return;
  }
  const result = await runtimeStore.checkoutHistoryNode(nodeId, "transcript_and_workspace");
  if (!result) {
    return;
  }

  if (result.degraded || result.degradedToTranscriptOnly) {
    historyFeedback.value = {
      tone: "warning",
      title: "仅恢复对话，未恢复工作区",
      text: `${describeDegradationReason(result.degradationReason)}。当前分支 ${
        result.activeBranchId || "main"
      }，可见节点 ${result.visibleNodeId || "latest"}。`
    };
    return;
  }

  historyFeedback.value = {
    tone: "success",
    title: "已切换到历史节点",
    text: `当前分支 ${result.activeBranchId || "main"}，可见节点 ${
      result.visibleNodeId || "latest"
    }，状态 ${historyModeMeta.value.label}。`
  };
}

async function handleRestoreBranchHead() {
  if (!canManageHistory.value) {
    return;
  }
  const result = await runtimeStore.restoreBranchHead(activeBranchId.value);
  if (!result) {
    return;
  }

  historyFeedback.value = {
    tone: "success",
    title: "已恢复到分支头",
    text: `当前分支 ${result.activeBranchId || "main"}，可见节点 ${
      result.visibleNodeId || result.branchHeadNodeId || "latest"
    }，已回到最新视图。`
  };
}

async function handleForkFromVisibleNode() {
  if (!canManageHistory.value || !visibleNodeId.value) {
    return;
  }
  const result = await runtimeStore.forkHistoryNode(visibleNodeId.value);
  if (!result) {
    return;
  }

  historyFeedback.value = {
    tone: "success",
    title: "已创建历史分支",
    text: `已从节点 ${result.nodeId || result.forkedFromNodeId} 分叉到分支 ${
      result.branch?.branchId || result.createdBranchId
    }，当前可见节点 ${
      result.visibleNodeId || result.nodeId || result.forkedFromNodeId
    }。`
  };
}

async function handleSwitchBranch(branchId: string) {
  if (!canManageHistory.value) {
    return;
  }
  const result = await runtimeStore.switchHistoryBranch(branchId);
  if (!result) {
    return;
  }

  historyFeedback.value = {
    tone: "info",
    title: "已切换历史分支",
    text: `已从分支 ${result.previousBranchId || "main"} 切换到 ${
      result.activeBranchId || result.branchId || branchId
    }，当前可见节点 ${result.visibleNodeId || result.branchHeadNodeId || "latest"}。`
  };
}
</script>

<template>
  <aside
    class="h-full min-h-0 min-w-0 overflow-hidden rounded-[0.6rem] bg-white/72 transition-[width] duration-200 ease-in-out"
    :class="asideClass"
  >
    <div
      class="flex h-full min-h-0 flex-col transition-[padding] duration-200 ease-in-out"
      :class="collapsed ? 'items-center px-1 py-4' : 'px-3 py-4 sm:px-3.5'"
    >
      <div class="flex w-full items-center gap-2" :class="collapsed ? 'justify-center' : 'justify-between'">
        <button
          v-if="!collapsed"
          class="flex min-w-0 items-center gap-2 text-left"
          type="button"
          data-testid="session-sidebar-brand"
          @click="navigate('home')"
        >
          <PonyBrandIcon class-name="h-7 w-7 shrink-0 rounded-[0.65rem]" />
          <div class="truncate text-[0.95rem] font-semibold tracking-[-0.03em] text-stone-950">Pony Agent</div>
        </button>

        <button
          class="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-[0.35rem] bg-transparent text-stone-500 transition hover:text-stone-900"
          type="button"
          data-testid="session-sidebar-collapse"
          @click="toggleCollapsed"
        >
          <ChevronLeft v-if="!collapsed" class="h-3.5 w-3.5" />
          <ChevronRight v-else class="h-3.5 w-3.5" />
        </button>
      </div>

      <div
        v-if="collapsed"
        class="mt-4 flex flex-1 flex-col items-center gap-2"
        data-testid="session-sidebar-collapsed"
      >
        <button
          class="inline-flex h-9 w-9 items-center justify-center rounded-[0.75rem] bg-[#fbf4e8] shadow-[0_1px_0_rgba(28,25,23,0.03)]"
          type="button"
          title="Pony Agent"
          data-testid="session-sidebar-brand-collapsed"
          @click="navigate('home')"
        >
          <PonyBrandIcon class-name="h-7 w-7 shrink-0 rounded-[0.65rem]" />
        </button>

        <button
          class="inline-flex h-8 w-8 items-center justify-center rounded-[0.42rem] bg-transparent text-stone-500 transition hover:bg-[#f7e3bf] hover:text-stone-900 disabled:cursor-not-allowed disabled:text-stone-300"
          type="button"
          :disabled="!canCreateSession"
          :title="createSessionTitle"
          data-testid="session-sidebar-new-chat-collapsed"
          @click="runtimeStore.createSession()"
        >
          <Plus class="h-4 w-4" />
        </button>

        <button
          class="inline-flex h-8 w-8 items-center justify-center rounded-[0.42rem] transition"
          :class="
            props.currentPage === 'home'
              ? 'bg-[#f7e3bf] text-stone-900'
              : 'bg-transparent text-stone-500 hover:bg-[#f7e3bf] hover:text-stone-900'
          "
          type="button"
          title="对话历史"
          data-testid="session-sidebar-history-collapsed"
          @click="openHistoryHome()"
        >
          <MessageSquareMore class="h-4 w-4" />
        </button>

        <button
          class="inline-flex h-8 w-8 items-center justify-center rounded-[0.42rem] transition"
          :class="
            props.currentPage === 'providers'
              ? 'bg-[#f7e3bf] text-stone-900'
              : 'bg-transparent text-stone-500 hover:bg-[#f7e3bf] hover:text-stone-900'
          "
          type="button"
          title="模型配置"
          data-testid="session-sidebar-nav-providers-collapsed"
          @click="navigate('providers')"
        >
          <Settings2 class="h-4 w-4" />
        </button>

        <button
          class="inline-flex h-8 w-8 items-center justify-center rounded-[0.42rem] transition"
          :class="
            props.currentPage === 'model-monitor'
              ? 'bg-[#f7e3bf] text-stone-900'
              : 'bg-transparent text-stone-500 hover:bg-[#f7e3bf] hover:text-stone-900'
          "
          type="button"
          title="模型监控"
          data-testid="session-sidebar-nav-model-monitor-collapsed"
          @click="navigate('model-monitor')"
        >
          <Activity class="h-4 w-4" />
        </button>
      </div>

      <template v-else>
        <div class="mt-4 flex items-center justify-between gap-2" data-testid="session-sidebar-actions">
          <button
            class="inline-flex h-8 items-center gap-2 px-1.5 text-[12px] font-medium text-stone-700 disabled:cursor-not-allowed disabled:text-stone-300"
            :class="menuInteractiveClass"
            type="button"
            :disabled="!canCreateSession"
            :title="createSessionTitle"
            data-testid="session-sidebar-new-chat"
            @click="runtimeStore.createSession()"
          >
            <Plus class="h-3.5 w-3.5" />
            <span>新对话</span>
          </button>
        </div>

        <ScrollArea class="mt-3 min-h-0 flex-1" viewport-class="pr-1.5"><div class="flex flex-col gap-2">
          <section class="rounded-[0.5rem]" data-testid="session-sidebar-history-panel">
            <button
              class="flex w-full items-center justify-between gap-2 px-1.5 py-2 text-left"
              :class="[menuInteractiveClass, props.currentPage === 'home' ? menuSelectedClass : 'text-stone-800']"
              type="button"
              data-testid="session-sidebar-history-toggle"
              @click="handleHistoryToggle"
            >
              <div class="flex items-center gap-2 text-[12px] font-medium text-stone-800">
                <MessageSquareMore class="h-3.5 w-3.5" />
                <span>对话历史</span>
              </div>
              <ChevronDown class="h-4 w-4 text-stone-400 transition" :class="{ 'rotate-180': historyOpen }" />
            </button>

            <ScrollArea
              v-if="historyOpen"
              class="min-h-0 flex-1"
              viewport-class="h-full w-full pr-1"
              data-testid="session-sidebar-history"
            >
              <div class="space-y-1.5 py-1">
                <div
                  class="rounded-[0.45rem] border border-stone-200/80 bg-[#fcf7ef] px-2 py-2"
                  data-testid="session-sidebar-history-graph"
                >
                  <div class="flex items-center justify-between gap-2">
                    <div class="min-w-0">
                      <div class="text-[11px] font-medium text-stone-800">
                        {{ isHistoricalMode ? "历史浏览中" : "当前在最新节点" }}
                      </div>
                      <div class="mt-0.5 truncate text-[10px] text-stone-500">
                        分支 {{ activeBranchId || "main" }} · 可见 {{ visibleNodeId || "latest" }}
                      </div>
                      <div class="mt-1 text-[10px] leading-4 text-stone-500">
                        {{ historyModeMeta.summary }}
                      </div>
                    </div>
                    <span class="rounded-full px-2 py-0.5 text-[10px]" :class="historyModeMeta.badgeClass">
                      {{ historyModeMeta.label }}
                    </span>
                  </div>

                  <div
                    class="mt-2 rounded-[0.4rem] border border-stone-200/80 bg-white/90 px-2 py-2"
                    data-testid="session-sidebar-control-status"
                  >
                    <div class="flex items-center justify-between gap-2">
                      <div class="text-[10px] font-medium text-stone-700">会话控制</div>
                      <span class="rounded-full px-2 py-0.5 text-[10px]" :class="sessionControlMeta.badgeClass">
                        {{ sessionControlMeta.label }}
                      </span>
                    </div>
                    <div class="mt-1 text-[10px] leading-4 text-stone-500">
                      {{ sessionControlMeta.summary }}
                    </div>
                    <div
                      v-if="latestControlBoundaryEvidenceText"
                      class="mt-1 rounded-[0.35rem] bg-stone-50 px-2 py-1 text-[10px] leading-4 text-stone-500"
                      data-testid="session-sidebar-control-boundary-evidence"
                    >
                      {{ latestControlBoundaryEvidenceText }}
                    </div>
                  </div>

                  <div
                    v-if="historyActionEvidenceMeta"
                    class="mt-2 rounded-[0.4rem] border px-2 py-2"
                    :class="explainabilityClass(historyActionEvidenceMeta.tone)"
                    data-testid="session-sidebar-history-action-evidence"
                  >
                    <div class="flex items-center justify-between gap-2">
                      <div class="text-[10px] font-medium">最近动作证据</div>
                      <span class="rounded-full px-2 py-0.5 text-[10px]">{{ historyActionEvidenceMeta.badge }}</span>
                    </div>
                    <div class="mt-1 text-[10px] leading-4">
                      {{ historyActionEvidenceMeta.summary }}
                    </div>
                    <div v-if="historyActionEvidenceMeta.detail" class="mt-1 text-[10px] leading-4 opacity-75">
                      {{ historyActionEvidenceMeta.detail }}
                    </div>
                  </div>

                  <div
                    class="mt-2 rounded-[0.4rem] border border-stone-200/80 bg-stone-50 px-2 py-2 text-stone-700"
                    data-testid="session-sidebar-history-current-context"
                  >
                    <div class="text-[10px] font-medium">当前上下文（非动作证据）</div>
                    <div class="mt-1 text-[10px] leading-4">
                      {{ historyCurrentContextMeta.summary }}
                    </div>
                    <div v-if="historyCurrentContextMeta.detail" class="mt-1 text-[10px] leading-4 opacity-75">
                      {{ historyCurrentContextMeta.detail }}
                    </div>
                  </div>

                  <div class="mt-2 flex flex-wrap gap-1.5">
                    <button
                      class="rounded-[0.35rem] border border-stone-200 bg-white px-2 py-1 text-[10px] text-stone-700 transition hover:border-stone-300 hover:text-stone-900 disabled:cursor-not-allowed disabled:text-stone-300"
                      type="button"
                      :disabled="!canManageHistory || !isHistoricalMode"
                      :title="historyRestoreDisabledReason || '恢复到当前分支头，并回到最新视图。'"
                      data-testid="session-sidebar-history-restore"
                      @click="handleRestoreBranchHead"
                    >
                      恢复到分支头
                    </button>
                    <button
                      class="rounded-[0.35rem] border border-stone-200 bg-white px-2 py-1 text-[10px] text-stone-700 transition hover:border-stone-300 hover:text-stone-900 disabled:cursor-not-allowed disabled:text-stone-300"
                      type="button"
                      :disabled="!canManageHistory || !visibleNodeId"
                      :title="historyForkDisabledReason || '从当前可见历史节点创建新分支。'"
                      data-testid="session-sidebar-history-fork"
                      @click="handleForkFromVisibleNode"
                    >
                      从当前节点分叉
                    </button>
                  </div>

                  <div
                    v-if="historyActionDisabledHint"
                    class="mt-2 rounded-[0.35rem] border border-dashed border-stone-200 bg-stone-50 px-2 py-1.5 text-[10px] leading-4 text-stone-500"
                    data-testid="session-sidebar-history-disabled-reason"
                  >
                    {{ historyActionDisabledHint }}
                  </div>

                  <div
                    v-if="historyFeedback"
                    class="mt-2 rounded-[0.4rem] border px-2 py-2"
                    :class="feedbackClass(historyFeedback.tone)"
                    data-testid="session-sidebar-history-feedback"
                  >
                    <div class="flex items-start gap-2">
                      <CircleAlert class="mt-0.5 h-3.5 w-3.5 shrink-0" />
                      <div class="min-w-0">
                        <div class="text-[10px] font-medium">{{ historyFeedback.title }}</div>
                        <div class="mt-0.5 text-[10px] leading-4">
                          {{ historyFeedback.text }}
                        </div>
                      </div>
                    </div>
                  </div>

                  <div v-if="sortedHistoryBranches.length" class="mt-2 flex flex-wrap gap-1.5">
                    <button
                      v-for="branch in sortedHistoryBranches"
                      :key="branch.branchId"
                      class="rounded-full border px-2 py-0.5 text-[10px] transition"
                      :class="
                        branch.branchId === activeBranchId
                          ? 'border-[#d8a15d] bg-[#f3c98d] text-stone-900'
                          : 'border-stone-200 bg-white text-stone-500 hover:border-stone-300 hover:text-stone-800'
                      "
                      type="button"
                      :disabled="!canManageHistory"
                      :title="historyBranchSwitchDisabledReason || '切换到该分支的当前头节点。'"
                      :data-testid="`session-sidebar-history-branch-${branch.branchId}`"
                      @click="handleSwitchBranch(branch.branchId)"
                    >
                      {{ branch.label || branch.branchId }}
                    </button>
                  </div>

                  <div v-if="hasHistoryNodes" class="mt-2 space-y-1">
                    <button
                      v-for="node in sortedHistoryNodes"
                      :key="node.nodeId"
                      class="w-full rounded-[0.35rem] border px-2 py-1.5 text-left transition"
                      :class="
                        node.nodeId === visibleNodeId
                          ? 'border-[#d8a15d] bg-white text-stone-900'
                          : 'border-transparent bg-white/70 text-stone-600 hover:border-stone-200 hover:text-stone-900'
                      "
                      type="button"
                      :disabled="!canManageHistory"
                      :title="historyCheckoutDisabledReason || '切换到该历史节点，并尝试恢复工作区。'"
                      :data-testid="`session-sidebar-history-node-${node.nodeId}`"
                      @click="handleCheckoutHistoryNode(node.nodeId)"
                    >
                      <div class="flex items-center justify-between gap-2">
                        <span class="truncate text-[11px] font-medium">
                          {{ node.summary || node.nodeId }}
                        </span>
                        <span class="text-[10px] text-stone-400">{{ formatHistoryNodeTime(node.createdAtMs) }}</span>
                      </div>
                      <div class="mt-0.5 flex items-center gap-2 text-[10px] text-stone-400">
                        <span>{{ formatHistoryNodeKind(node.kind) }}</span>
                        <span>{{ node.branchId }}</span>
                        <span v-if="node.nodeId === branchHeadNodeId">branch head</span>
                      </div>
                    </button>
                  </div>

                  <div v-else class="mt-2 text-[10px] text-stone-400">当前会话还没有可回退的历史节点。</div>
                </div>

                <div
                  v-for="session in visibleSessions"
                  :key="session.conversationId"
                  class="group rounded-[0.2rem]"
                  :class="[
                    session.conversationId === sessionId ? menuSelectedClass : menuInteractiveClass,
                    pendingDeleteSessionId === session.conversationId ? 'bg-rose-50 text-rose-700' : ''
                  ]"
                >
                  <div class="flex items-center gap-2 px-1.5 py-1.5">
                    <button
                      class="min-w-0 flex-1 text-left"
                      type="button"
                      :disabled="isSubmitting || Boolean(sessionOperation)"
                      :data-testid="`session-switch-${session.conversationId}`"
                      @click="openSessionHistory(session.conversationId)"
                    >
                      <div class="flex items-center gap-2 text-[12px] leading-5">
                        <span
                          class="truncate"
                          :class="session.conversationId === sessionId ? 'font-medium text-stone-900' : 'text-stone-700'"
                        >
                          {{ sessionHeadline(session) }}
                        </span>
                        <span v-if="isTransientSession(session)" class="shrink-0 text-[10px] text-amber-600">
                          未保存
                        </span>
                        <span class="shrink-0 text-[10px] text-stone-400">
                          {{ formatSessionTime(session.updatedAtMs) }}
                        </span>
                      </div>
                    </button>

                    <button
                      class="inline-flex shrink-0 cursor-pointer items-center justify-center text-[10px] text-stone-400 transition hover:cursor-pointer hover:text-rose-600 disabled:cursor-not-allowed disabled:text-stone-300"
                      :class="
                        pendingDeleteSessionId === session.conversationId
                          ? 'h-5 rounded-full bg-rose-200 px-1.5 text-rose-800 hover:bg-rose-300 hover:text-rose-900'
                          : 'rounded-[0.35rem] px-1.5 py-1'
                      "
                      type="button"
                      :disabled="!canDeleteSession(session)"
                      :title="
                        isTransientSession(session)
                          ? '空白新对话会在切换后自动丢弃，无需单独删除。'
                          : pendingDeleteSessionId === session.conversationId
                            ? '再点一次确认删除。'
                            : '删除对话'
                      "
                      :data-testid="`session-delete-${session.conversationId}`"
                      @click.stop="handleDeleteSession(session)"
                    >
                      <Trash2 v-if="pendingDeleteSessionId !== session.conversationId" class="h-3.5 w-3.5" />
                      <span v-else class="inline-flex items-center justify-center text-[10px] font-medium text-rose-800">
                        确认？
                      </span>
                    </button>
                  </div>
                </div>
              </div>
            </ScrollArea>
          </section>

          <section class="rounded-[0.5rem]" data-testid="session-sidebar-model-nav">
            <button
              class="flex w-full items-center justify-between gap-2 px-1.5 py-2 text-left"
              :class="[
                menuInteractiveClass,
                props.currentPage === 'providers' || props.currentPage === 'model-monitor'
                  ? menuSelectedClass
                  : 'text-stone-800'
              ]"
              type="button"
              data-testid="session-sidebar-model-toggle"
              @click="toggleSection('model')"
            >
              <div class="flex items-center gap-2 text-[12px] font-medium text-stone-800">
                <Settings2 class="h-3.5 w-3.5" />
                <span>模型管理</span>
              </div>
              <ChevronDown class="h-4 w-4 text-stone-400 transition" :class="{ 'rotate-180': modelOpen }" />
            </button>

            <div v-if="modelOpen" class="space-y-0.5 py-1 pl-1">
              <button
                class="flex w-full items-center justify-start gap-1.5 px-1.5 py-1 text-left"
                :class="
                  props.currentPage === 'providers'
                    ? menuSelectedClass
                    : `${menuInteractiveClass} text-stone-500`
                "
                type="button"
                data-testid="session-sidebar-nav-providers"
                @click="navigate('providers')"
              >
                <Settings2 class="h-3 w-3" />
                <span class="text-[12px] leading-4">模型配置</span>
              </button>

              <button
                class="flex w-full items-center justify-start gap-1.5 px-1.5 py-1 text-left"
                :class="
                  props.currentPage === 'model-monitor'
                    ? menuSelectedClass
                    : `${menuInteractiveClass} text-stone-500`
                "
                type="button"
                data-testid="session-sidebar-nav-model-monitor"
                @click="navigate('model-monitor')"
              >
                <Activity class="h-3 w-3" />
                <span class="text-[12px] leading-4">模型监控</span>
              </button>
            </div>
          </section>

          <section
            v-if="collapsed"
            class="hidden"
            aria-hidden="true"
          />
        </div></ScrollArea>
      </template>
    </div>
  </aside>
</template>
