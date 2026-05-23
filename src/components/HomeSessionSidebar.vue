<script setup lang="ts">
import { computed, ref } from "vue";
import { storeToRefs } from "pinia";
import {
  ChevronLeft,
  ChevronRight,
  Clock3,
  MessageSquareMore,
  Plus,
  Trash2
} from "lucide-vue-next";
import { useRuntimeStore } from "@/stores/runtime";
import ScrollArea from "@/components/ui/ScrollArea.vue";
import type { ChatMessage, SessionOverview } from "@/types/runtime";

const SESSION_SIDEBAR_STORAGE_KEY = "pony-agent.session-sidebar-collapsed.v1";

const runtimeStore = useRuntimeStore();
const { isSubmitting, messages, sessionId, sessionList, sessionOperation } = storeToRefs(runtimeStore);

const collapsed = ref(loadCollapsedState());

const hasPersistableCurrentSession = computed(() => hasPersistableMessages(messages.value));
const canCreateSession = computed(
  () => !isSubmitting.value && !sessionOperation.value && hasPersistableCurrentSession.value
);
const createSessionTitle = computed(() =>
  hasPersistableCurrentSession.value
    ? "新建一个空白对话，会保留当前历史会话。"
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

function loadCollapsedState() {
  if (typeof window === "undefined") {
    return false;
  }

  return window.localStorage.getItem(SESSION_SIDEBAR_STORAGE_KEY) === "1";
}

function persistCollapsedState() {
  if (typeof window === "undefined") {
    return;
  }

  window.localStorage.setItem(SESSION_SIDEBAR_STORAGE_KEY, collapsed.value ? "1" : "0");
}

function toggleCollapsed() {
  collapsed.value = !collapsed.value;
  persistCollapsedState();
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
</script>

<template>
  <aside
    class="h-full min-h-0 min-w-0 overflow-hidden rounded-[0.6rem] bg-white/72 transition-[width] duration-200"
    :class="asideClass"
  >
    <div
      class="flex h-full min-h-0 flex-col"
      :class="collapsed ? 'items-center px-2 py-4' : 'px-3 py-4 sm:px-3.5'"
    >
      <div
        class="flex w-full items-center gap-2"
        :class="collapsed ? 'justify-center' : 'justify-between'"
      >
        <div
          v-if="!collapsed"
          class="flex items-center gap-2 text-[11px] uppercase tracking-[0.18em] text-stone-500"
        >
          <MessageSquareMore class="h-3.5 w-3.5" />
          <span>对话历史</span>
        </div>

        <button
          class="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-[0.35rem] bg-[#f6efe3] text-stone-600 transition hover:text-stone-900"
          type="button"
          @click="toggleCollapsed"
        >
          <ChevronLeft v-if="!collapsed" class="h-3.5 w-3.5" />
          <ChevronRight v-else class="h-3.5 w-3.5" />
        </button>
      </div>

      <div v-if="collapsed" class="mt-4 flex flex-1 flex-col items-center gap-2">
        <button
          class="inline-flex h-8 w-8 items-center justify-center rounded-[0.42rem] bg-transparent text-stone-500 transition hover:bg-[#f6efe3] hover:text-stone-900 disabled:cursor-not-allowed disabled:text-stone-300"
          type="button"
          :disabled="!canCreateSession"
          :title="createSessionTitle"
          @click="runtimeStore.createSession()"
        >
          <Plus class="h-4 w-4" />
        </button>

        <button
          v-for="session in visibleSessions"
          :key="session.conversationId"
          class="inline-flex h-8 w-8 items-center justify-center rounded-[0.42rem] transition"
          :class="
            session.conversationId === sessionId
              ? 'bg-[#f6efe3] text-stone-900'
              : 'bg-transparent text-stone-500 hover:bg-[#f6efe3] hover:text-stone-900'
          "
          type="button"
          :disabled="isSubmitting || Boolean(sessionOperation)"
          :title="session.title || session.summary || session.conversationId"
          @click="runtimeStore.switchSession(session.conversationId)"
        >
          <Clock3 class="h-3.5 w-3.5" />
        </button>
      </div>

      <template v-else>
        <div class="mt-4 flex items-center justify-between gap-2">
          <button
            class="inline-flex h-8 items-center gap-2 rounded-[0.42rem] bg-[#f6efe3] px-3 text-[12px] text-stone-700 transition hover:text-stone-950 disabled:cursor-not-allowed disabled:text-stone-300"
            type="button"
            :disabled="!canCreateSession"
            :title="createSessionTitle"
            @click="runtimeStore.createSession()"
          >
            <Plus class="h-3.5 w-3.5" />
            <span>新对话</span>
          </button>
          <div
            v-if="!hasPersistableCurrentSession"
            class="text-[10px] text-stone-400"
          >
            当前为空对话，发送首条消息后才会保存
          </div>
        </div>

        <ScrollArea class="mt-3 min-h-0 flex-1" viewport-class="h-full w-full pr-1">
          <div class="space-y-1.5">
            <div
              v-for="session in visibleSessions"
              :key="session.conversationId"
              class="group rounded-[0.45rem] transition hover:bg-[#f6efe3]/70"
              :class="
                session.conversationId === sessionId
                  ? 'bg-[#f6efe3]'
                  : 'bg-transparent'
              "
            >
              <div class="flex items-start gap-2 px-2 py-2">
                <button
                  class="min-w-0 flex-1 text-left"
                  type="button"
                  :disabled="isSubmitting || Boolean(sessionOperation)"
                  @click="runtimeStore.switchSession(session.conversationId)"
                >
                  <div
                    class="truncate text-[12px] leading-5"
                    :class="session.conversationId === sessionId ? 'text-stone-900' : 'text-stone-700'"
                  >
                    {{ session.title || session.summary || session.conversationId }}
                    <span
                      v-if="isTransientSession(session)"
                      class="ml-1 text-[10px] text-amber-600"
                    >
                      未保存
                    </span>
                  </div>
                  <div class="mt-0.5 truncate text-[10px] text-stone-400">
                    {{ session.summary }}
                  </div>
                  <div class="mt-0.5 text-[10px] text-stone-400">
                    {{ session.turnCount }} 轮<span v-if="session.lastReferencedFile"> · {{ session.lastReferencedFile }}</span>
                    <span v-if="session.updatedAtMs"> · {{ formatSessionTime(session.updatedAtMs) }}</span>
                  </div>
                </button>

                <button
                  class="mt-0.5 inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-[0.35rem] text-stone-400 transition hover:bg-white hover:text-rose-600 disabled:cursor-not-allowed disabled:text-stone-300"
                  type="button"
                  :disabled="!canDeleteSession(session)"
                  :title="isTransientSession(session) ? '空白新对话会在切换后自动丢弃，无需单独删除。' : '删除对话'"
                  @click.stop="runtimeStore.deleteSession(session.conversationId)"
                >
                  <Trash2 class="h-3.5 w-3.5" />
                </button>
              </div>
            </div>
          </div>
        </ScrollArea>
      </template>
    </div>
  </aside>
</template>
