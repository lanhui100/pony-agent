<script setup lang="ts">
import { computed, ref } from "vue";
import { storeToRefs } from "pinia";
import {
  Activity,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
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
  messages,
  sessionId,
  sessionList,
  sessionOperation,
  visibleNodeId
} = storeToRefs(runtimeStore);

const collapsed = ref(loadStoredBoolean(SESSION_SIDEBAR_STORAGE_KEY, false));
const historyOpen = ref(loadStoredBoolean(HISTORY_OPEN_STORAGE_KEY, true));
const modelOpen = ref(loadStoredBoolean(MODEL_OPEN_STORAGE_KEY, true));
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
  if (props.currentPage !== "home") {
    navigate("home");
  }

  runtimeStore.switchSession(conversationId);
}

function formatSessionTime(updatedAtMs?: number) {
  if (!updatedAtMs) {
    return "未保存";
  }

  return new Intl.DateTimeFormat("zh-CN", {
    month: "numeric",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit"
  }).format(updatedAtMs);
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

async function handleCheckoutHistoryNode(nodeId: string) {
  if (!canManageHistory.value) {
    return;
  }
  await runtimeStore.checkoutHistoryNode(nodeId, "transcript_and_workspace");
}

async function handleRestoreBranchHead() {
  if (!canManageHistory.value) {
    return;
  }
  await runtimeStore.restoreBranchHead(activeBranchId.value);
}

async function handleForkFromVisibleNode() {
  if (!canManageHistory.value || !visibleNodeId.value) {
    return;
  }
  await runtimeStore.forkHistoryNode(visibleNodeId.value);
}

async function handleSwitchBranch(branchId: string) {
  if (!canManageHistory.value) {
    return;
  }
  await runtimeStore.switchHistoryBranch(branchId);
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

        <div class="mt-3 flex min-h-0 flex-1 flex-col gap-2">
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
                    </div>
                    <span
                      class="rounded-full px-2 py-0.5 text-[10px]"
                      :class="
                        isHistoricalMode
                          ? 'bg-amber-100 text-amber-700'
                          : 'bg-emerald-100 text-emerald-700'
                      "
                    >
                      {{ historyCursorMode }}
                    </span>
                  </div>

                  <div class="mt-2 flex flex-wrap gap-1.5">
                    <button
                      class="rounded-[0.35rem] border border-stone-200 bg-white px-2 py-1 text-[10px] text-stone-700 transition hover:border-stone-300 hover:text-stone-900 disabled:cursor-not-allowed disabled:text-stone-300"
                      type="button"
                      :disabled="!canManageHistory || !isHistoricalMode"
                      data-testid="session-sidebar-history-restore"
                      @click="handleRestoreBranchHead"
                    >
                      恢复到分支头
                    </button>
                    <button
                      class="rounded-[0.35rem] border border-stone-200 bg-white px-2 py-1 text-[10px] text-stone-700 transition hover:border-stone-300 hover:text-stone-900 disabled:cursor-not-allowed disabled:text-stone-300"
                      type="button"
                      :disabled="!canManageHistory || !visibleNodeId"
                      data-testid="session-sidebar-history-fork"
                      @click="handleForkFromVisibleNode"
                    >
                      从当前节点分叉
                    </button>
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
                  :class="session.conversationId === sessionId ? menuSelectedClass : menuInteractiveClass"
                >
                  <div class="flex items-start gap-2 px-1.5 py-1.5">
                    <button
                      class="min-w-0 flex-1 text-left"
                      type="button"
                      :disabled="isSubmitting || Boolean(sessionOperation)"
                      :data-testid="`session-switch-${session.conversationId}`"
                      @click="openSessionHistory(session.conversationId)"
                    >
                      <div
                        class="truncate text-[12px] leading-5"
                        :class="session.conversationId === sessionId ? 'font-medium text-stone-900' : 'text-stone-700'"
                      >
                        {{ session.title || session.summary || session.conversationId }}
                        <span v-if="isTransientSession(session)" class="ml-1 text-[10px] text-amber-600">
                          未保存
                        </span>
                      </div>
                      <div class="mt-0.5 truncate text-[10px] text-stone-400">
                        {{ session.summary }}
                      </div>
                      <div class="mt-0.5 text-[10px] text-stone-400">
                        {{ session.turnCount }} 轮
                        <span v-if="session.lastReferencedFile"> · {{ session.lastReferencedFile }}</span>
                        <span v-if="session.updatedAtMs"> · {{ formatSessionTime(session.updatedAtMs) }}</span>
                      </div>
                    </button>

                    <button
                      class="mt-0.5 inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-[0.35rem] text-stone-400 transition hover:text-rose-600 disabled:cursor-not-allowed disabled:text-stone-300"
                      type="button"
                      :disabled="!canDeleteSession(session)"
                      :title="isTransientSession(session) ? '空白新对话会在切换后自动丢弃，无需单独删除。' : '删除对话'"
                      :data-testid="`session-delete-${session.conversationId}`"
                      @click.stop="runtimeStore.deleteSession(session.conversationId)"
                    >
                      <Trash2 class="h-3.5 w-3.5" />
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
        </div>
      </template>
    </div>
  </aside>
</template>
