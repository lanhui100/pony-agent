<script setup lang="ts">
import { computed, ref } from "vue";
import { storeToRefs } from "pinia";
import {
  Activity,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  LoaderCircle,
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
const MODEL_OPEN_STORAGE_KEY = "pony-agent.session-sidebar-model-open.v1";

type NavigationPage = "home" | "providers" | "model-monitor";

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
  isSubmitting,
  messages,
  sessionId,
  sessionList,
  sessionOperation
} = storeToRefs(runtimeStore);

const collapsed = ref(loadStoredBoolean(SESSION_SIDEBAR_STORAGE_KEY, false));
const modelOpen = ref(loadStoredBoolean(MODEL_OPEN_STORAGE_KEY, true));
const pendingDeleteSessionId = ref<string | null>(null);
const deletingSessionId = ref<string | null>(null);
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

function toggleModelSection() {
  modelOpen.value = !modelOpen.value;
  persistStoredBoolean(MODEL_OPEN_STORAGE_KEY, modelOpen.value);
}

function navigate(page: NavigationPage) {
  emit("navigate", page);
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

function isDeletingSession(session: SessionOverview) {
  return deletingSessionId.value === session.conversationId;
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
  deletingSessionId.value = session.conversationId;
  try {
    await runtimeStore.deleteSession(session.conversationId);
  } finally {
    if (deletingSessionId.value === session.conversationId) {
      deletingSessionId.value = null;
    }
  }
}

function clearPendingDeleteSession(session: SessionOverview) {
  if (pendingDeleteSessionId.value === session.conversationId) {
    pendingDeleteSessionId.value = null;
  }
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
          title="对话工作区"
          data-testid="session-sidebar-home-collapsed"
          @click="navigate('home')"
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
          <section class="rounded-[0.5rem]" data-testid="session-sidebar-session-list">
            <div class="px-1.5 py-2">
              <div class="flex items-center gap-2 text-[12px] font-medium text-stone-800">
                <MessageSquareMore class="h-3.5 w-3.5" />
                <span>对话</span>
              </div>
            </div>
            <div class="space-y-1.5 py-1">
                <div
                  v-for="session in visibleSessions"
                  :key="session.conversationId"
                  class="group rounded-[0.2rem]"
                  :class="[
                    session.conversationId === sessionId ? menuSelectedClass : menuInteractiveClass
                  ]"
                  @mouseleave="clearPendingDeleteSession(session)"
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
                      class="pointer-events-none inline-flex shrink-0 cursor-pointer items-center justify-center text-[10px] text-stone-400 opacity-0 transition hover:cursor-pointer hover:text-rose-600 group-hover:pointer-events-auto group-hover:opacity-100 group-focus-within:pointer-events-auto group-focus-within:opacity-100 disabled:cursor-not-allowed disabled:text-stone-300"
                      :class="
                        isDeletingSession(session)
                          ? 'h-5 rounded-[0.35rem] px-1.5 py-1 opacity-100'
                          : pendingDeleteSessionId === session.conversationId
                          ? 'h-5 rounded-full bg-rose-200 px-1.5 text-rose-800 hover:bg-rose-300 hover:text-rose-900'
                          : 'rounded-[0.35rem] px-1.5 py-1'
                      "
                      type="button"
                      :disabled="!canDeleteSession(session)"
                      :title="
                        isTransientSession(session)
                          ? '空白新对话会在切换后自动丢弃，无需单独删除。'
                          : isDeletingSession(session)
                            ? '正在删除'
                            : pendingDeleteSessionId === session.conversationId
                            ? '确认删除'
                            : '删除对话'
                      "
                      :data-testid="`session-delete-${session.conversationId}`"
                      @click.stop="handleDeleteSession(session)"
                    >
                      <LoaderCircle
                        v-if="isDeletingSession(session)"
                        class="h-3.5 w-3.5 animate-spin text-stone-400"
                        :data-testid="`session-delete-loading-${session.conversationId}`"
                      />
                      <Trash2 v-else-if="pendingDeleteSessionId !== session.conversationId" class="h-3.5 w-3.5" />
                      <span v-else class="inline-flex items-center justify-center text-[10px] font-medium text-rose-800">
                        确认
                      </span>
                    </button>
                  </div>
                </div>
            </div>
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
              @click="toggleModelSection"
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
