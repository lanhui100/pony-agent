<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { storeToRefs } from "pinia";
import {
  Activity,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  Folder,
  FolderOpen,
  LoaderCircle,
  MessageSquareMore,
  Plus,
  Server,
  Settings,
  Settings2,
  Trash2
} from "lucide-vue-next";
import PonyBrandIcon from "@/components/PonyBrandIcon.vue";
import ScrollArea from "@/components/ui/ScrollArea.vue";
import { useRuntimeStore } from "@/stores/runtime";
import { useSettingsStore } from "@/stores/settings";
import type { SidebarNavigationPage } from "@/types/config";
import type { ChatMessage, SessionOverview } from "@/types/runtime";
import {
  groupSessionsByWorkspace,
  loadStoredWorkspaceGroupKeys,
  persistCollapsedWorkspaceGroups
} from "@/lib/runtime/sidebar-groups";
import { isTauriAvailable } from "@/lib/tauri";

const SESSION_SIDEBAR_STORAGE_KEY = "pony-agent.session-sidebar-collapsed.v1";
/** 组内会话预览上限（"显示全部"前）；不做 per-group 分页，全局一键解除。 */
const GROUP_SESSION_PREVIEW_LIMIT = 5;

const props = withDefaults(
  defineProps<{
    currentPage?: SidebarNavigationPage | null;
    forceCollapsed?: boolean;
  }>(),
  {
    currentPage: "home",
    forceCollapsed: false
  }
);

const emit = defineEmits<{
  (event: "navigate", page: SidebarNavigationPage): void;
}>();

const runtimeStore = useRuntimeStore();
const settingsStore = useSettingsStore();
const { workspaceMode } = storeToRefs(settingsStore);
// PA-096：遥测入口按工作模式分档——coding 显"遥测"（Trace+指标），work 显"指标"。
const isCoding = computed(() => workspaceMode.value === "coding");
const {
  isSubmitting,
  messages,
  sessionId,
  sessionList,
  sessionOperation,
  workspaceList,
  activeWorkspaceId
} = storeToRefs(runtimeStore);

const collapsed = ref(loadStoredBoolean(SESSION_SIDEBAR_STORAGE_KEY, false));
const conversationOpen = ref(true);
const pendingDeleteSessionId = ref<string | null>(null);
// PA-081：折叠的组 key 集合（localStorage 持久化；读取忽略未知 key）。
const collapsedGroupKeys = ref<Set<string>>(
  loadStoredWorkspaceGroupKeys(typeof window !== "undefined" ? window.localStorage : undefined)
);
// 本 boot 内用户显式折叠过的组——自动展开逻辑不得覆盖其选择（spec 语义）。
const bootToggledGroupKeys = new Set<string>();
// PA-081：浏览器预览模式禁用 Workspace 管理（Tauri 后端不可用）。
const isTauriRuntime = isTauriAvailable();
// PA-081：Workspace 管理入口展开态 + 新建表单。
const workspaceSectionOpen = ref(false);
const workspaceFormOpen = ref(false);
const newWorkspaceName = ref("");
const newWorkspaceRootPath = ref("");
const workspaceCreating = ref(false);
// PA-081（实施后审核 P2）：新建失败的可见反馈（后端错误如"root 已存在"）。
const workspaceError = ref<string | null>(null);
const menuInteractiveClass =
  "rounded-[0.2rem] transition-colors cursor-pointer hover:bg-[#f6dfb8] hover:text-stone-900";
const menuSelectedClass = "rounded-[0.2rem] bg-[#f3c98d] text-stone-900";

const hasPersistableCurrentSession = computed(() => hasPersistableMessages(messages.value));
const hasVisibleCurrentSession = computed(() =>
  sessionList.value.some((session) => session.conversationId === sessionId.value)
);
const canCreateSession = computed(
  () => !sessionOperation.value && hasPersistableCurrentSession.value
);
const createSessionTitle = computed(() => {
  if (isSubmitting.value) {
    return "当前对话正在运行；新建空白对话后，运行会转入后台继续。";
  }

  return hasPersistableCurrentSession.value
    ? "新建一个空白对话，并保留当前已存在的历史会话。"
    : "当前已经是空白新对话，发送首条消息后才会保存到历史。";
});

const visibleSessions = computed<SessionOverview[]>(() => {
  if (hasVisibleCurrentSession.value) {
    return sessionList.value;
  }

  return [
    {
      conversationId: sessionId.value,
      title: "新对话",
      summary: "发送第一条消息后保存到历史",
      turnCount: 0,
      lastReferencedFile: null,
      updatedAtMs: 0,
      // PA-081：瞬态条目归属当前激活 Workspace（空 → default 组）。
      workspaceId: activeWorkspaceId.value || null
    },
    ...sessionList.value.filter((session) => session.conversationId !== sessionId.value)
  ];
});

// PA-081：两级树分组（Workspace → 会话；孤儿 → 未分组）。
const sessionGroups = computed(() =>
  groupSessionsByWorkspace(visibleSessions.value, workspaceList.value)
);
const totalVisibleSessions = computed(() =>
  sessionGroups.value.reduce((total, group) => total + group.sessions.length, 0)
);
// "显示全部"解除所有组的预览上限（组计数徽标恒按全量计算）。
const showAllGroupSessions = ref(false);

interface SidebarGroupRow {
  kind: "group";
  key: string;
  name: string;
  count: number;
  collapsed: boolean;
  /** 关联 workspace id；孤儿组为 null（无快捷新建入口）。 */
  workspaceId: string | null;
}
interface SidebarSessionRow {
  kind: "session";
  groupKey: string;
  session: SessionOverview;
}
type SidebarRow = SidebarGroupRow | SidebarSessionRow | { kind: "group-empty"; key: string };

const sidebarRows = computed<SidebarRow[]>(() => {
  const rows: SidebarRow[] = [];
  for (const group of sessionGroups.value) {
    const collapsedGroup = collapsedGroupKeys.value.has(group.key);
    rows.push({
      kind: "group",
      key: group.key,
      name: group.name,
      count: group.sessions.length,
      collapsed: collapsedGroup,
      workspaceId: group.workspaceId
    });
    if (collapsedGroup) {
      continue;
    }
    const preview =
      showAllGroupSessions.value || !hasAnyHiddenSessions.value
        ? group.sessions
        : group.sessions.slice(0, GROUP_SESSION_PREVIEW_LIMIT);
    for (const session of preview) {
      rows.push({ kind: "session", groupKey: group.key, session });
    }
    if (group.sessions.length === 0) {
      rows.push({ kind: "group-empty", key: group.key });
    }
  }
  return rows;
});

// PA-081 调优：激活 Workspace 显示名（注册表名优先；浏览器模式降级文案）。
const activeWorkspaceName = computed(() => {
  const active = workspaceList.value.find((workspace) => workspace.id === activeWorkspaceId.value);
  if (active?.name) {
    return active.name;
  }
  return isTauriRuntime ? "默认工作区" : "默认";
});

const hasAnyHiddenSessions = computed(() =>
  sessionGroups.value.some((group) => group.sessions.length > GROUP_SESSION_PREVIEW_LIMIT)
);
// PA-081（实施后审核 P1）：当前会话所在组若处于持久化折叠态则自动展开；
// 本 boot 内用户显式折叠过的组不覆盖（显式选择优先）。
watch(
  () => sessionId.value,
  (currentId) => {
    if (!currentId) {
      return;
    }
    const owningGroup = sessionGroups.value.find((group) =>
      group.sessions.some((session) => session.conversationId === currentId)
    );
    if (
      owningGroup &&
      collapsedGroupKeys.value.has(owningGroup.key) &&
      !bootToggledGroupKeys.has(owningGroup.key)
    ) {
      const next = new Set(collapsedGroupKeys.value);
      next.delete(owningGroup.key);
      collapsedGroupKeys.value = next;
    }
  }
);
// 稳定行 key：避免索引 key 在组增删/折叠切换时的 DOM 复用错位。
function sidebarRowKey(row: SidebarRow): string {
  if (row.kind === "group") {
    return `group-${row.key}`;
  }
  if (row.kind === "group-empty") {
    return `empty-${row.key}`;
  }
  return `session-${row.session.conversationId}`;
}
const canShowMoreConversations = computed(
  () => totalVisibleSessions.value > 0 && !showAllGroupSessions.value && hasAnyHiddenSessions.value
);

const sidebarCollapsed = computed(() => collapsed.value || props.forceCollapsed);

const asideClass = computed(() =>
  sidebarCollapsed.value
    ? "w-[3.4rem] shrink-0"
    : "w-[17.5rem] shrink-0 xl:w-[18.5rem]"
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

function toggleWorkspaceGroup(groupKey: string) {
  const next = new Set(collapsedGroupKeys.value);
  if (next.has(groupKey)) {
    next.delete(groupKey);
  } else {
    next.add(groupKey);
    // 本 boot 显式折叠：自动展开逻辑不得覆盖。
    bootToggledGroupKeys.add(groupKey);
  }
  collapsedGroupKeys.value = next;
  // 持久化时仅保留当前仍存在的组 key（未知 key 忽略，防止无限增长）。
  const validKeys = new Set(sessionGroups.value.map((group) => group.key));
  persistCollapsedWorkspaceGroups(next, validKeys, typeof window !== "undefined" ? window.localStorage : undefined);
}

function toggleWorkspaceSection() {
  workspaceSectionOpen.value = !workspaceSectionOpen.value;
}

function activateWorkspaceById(workspaceId: string) {
  runtimeStore.activateWorkspace(workspaceId);
}

// design §2：组头快捷入口——在该组 Workspace 下新建对话（激活目标随组切换）。
function createSessionInWorkspace(workspaceId: string) {
  pendingDeleteSessionId.value = null;
  if (props.currentPage !== "home") {
    navigate("home");
  }
  runtimeStore.activateWorkspace(workspaceId);
  void runtimeStore.createSession();
}

async function submitNewWorkspace() {
  if (workspaceCreating.value) {
    return;
  }
  workspaceError.value = null;
  workspaceCreating.value = true;
  try {
    const record = await runtimeStore.createNewWorkspace(
      newWorkspaceName.value,
      newWorkspaceRootPath.value
    );
    if (record) {
      newWorkspaceName.value = "";
      newWorkspaceRootPath.value = "";
      workspaceFormOpen.value = false;
    } else {
      workspaceError.value = "创建失败：请检查名称与根路径是否有效（根路径可能已存在）。";
    }
  } catch (error) {
    workspaceError.value = `创建失败：${String(error)}`;
  } finally {
    workspaceCreating.value = false;
  }
}

function toggleCollapsed() {
  collapsed.value = !collapsed.value;
  persistStoredBoolean(SESSION_SIDEBAR_STORAGE_KEY, collapsed.value);
}

function toggleConversationSection() {
  conversationOpen.value = !conversationOpen.value;
}

// PA-081："显示全部"解除所有组的会话数上限（组计数徽标恒按全量）。
function showMoreConversations() {
  showAllGroupSessions.value = true;
}

onMounted(() => {
  // Workspace 注册表加载（浏览器模式 contained：空列表 + 仅默认组）。
  void runtimeStore.loadWorkspaces();
});

function navigate(page: SidebarNavigationPage) {
  emit("navigate", page);
}

function createNewSession() {
  pendingDeleteSessionId.value = null;
  if (props.currentPage !== "home") {
    navigate("home");
  }

  void runtimeStore.createSession();
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
  return session.conversationId === sessionId.value && !hasVisibleCurrentSession.value;
}

function canDeleteSession(session: SessionOverview) {
  if (isSubmitting.value || isTransientSession(session) || runtimeStore.isSessionDeleting(session.conversationId)) {
    return false;
  }

  return !sessionOperation.value;
}

function isDeletingSession(session: SessionOverview) {
  return runtimeStore.isSessionDeleting(session.conversationId);
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
      :class="sidebarCollapsed ? 'items-center px-1 py-4' : 'px-3 py-4 sm:px-3.5'"
    >
      <div class="flex w-full items-center gap-2" :class="sidebarCollapsed ? 'justify-center' : 'justify-between'">
        <button
          v-if="!sidebarCollapsed"
          class="flex min-w-0 items-center gap-2 text-left"
          type="button"
          data-testid="session-sidebar-brand"
          @click="navigate('home')"
        >
          <PonyBrandIcon class-name="h-7 w-7 shrink-0 rounded-[0.65rem]" />
          <div class="truncate text-[0.95rem] font-semibold tracking-[-0.03em] text-stone-950">Pony Agent</div>
        </button>

        <button
          class="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-[0.35rem] bg-transparent text-stone-500 transition hover:cursor-pointer hover:bg-[#f7e3bf] hover:text-stone-900"
          type="button"
          data-testid="session-sidebar-collapse"
          @click="toggleCollapsed"
        >
          <ChevronLeft v-if="!collapsed" class="h-3.5 w-3.5" />
          <ChevronRight v-else class="h-3.5 w-3.5" />
        </button>
      </div>

      <div
        v-if="sidebarCollapsed"
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
          @click="createNewSession"
        >
          <Plus class="h-4 w-4" />
        </button>

        <button
          class="relative inline-flex h-8 w-8 items-center justify-center rounded-[0.42rem] transition"
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
          <span
            v-if="Object.keys(runtimeStore.runningSessionMap).length > 0"
            class="absolute -right-0.5 -top-0.5 h-2 w-2 animate-pulse rounded-full bg-amber-500"
            data-testid="session-sidebar-collapsed-running-indicator"
          />
        </button>

        <button
          class="inline-flex h-8 w-8 items-center justify-center rounded-[0.42rem] transition"
          :class="
            props.currentPage === 'telemetry'
              ? 'bg-[#f7e3bf] text-stone-900'
              : 'bg-transparent text-stone-500 hover:bg-[#f7e3bf] hover:text-stone-900'
          "
          type="button"
          :title="isCoding ? '遥测' : '指标'"
          :aria-label="isCoding ? '打开遥测页' : '打开指标监控'"
          data-testid="session-sidebar-nav-telemetry-collapsed"
          @click="navigate('telemetry')"
        >
          <Activity class="h-4 w-4" />
        </button>

        <button
          class="inline-flex h-8 w-8 items-center justify-center rounded-[0.42rem] transition"
          :class="
            props.currentPage === 'models'
              ? 'bg-[#f7e3bf] text-stone-900'
              : 'bg-transparent text-stone-500 hover:bg-[#f7e3bf] hover:text-stone-900'
          "
          type="button"
          title="模型配置"
          aria-label="打开模型配置"
          data-testid="session-sidebar-nav-providers-collapsed"
          @click="navigate('models')"
        >
          <Settings2 class="h-4 w-4" />
        </button>

        <button
          class="mt-auto inline-flex h-8 w-8 items-center justify-center rounded-[0.42rem] transition"
          :class="
            props.currentPage === 'settings'
              ? 'bg-[#f7e3bf] text-stone-900'
              : 'bg-transparent text-stone-500 hover:bg-[#f7e3bf] hover:text-stone-900'
          "
          type="button"
          title="设置"
          data-testid="session-sidebar-nav-settings-collapsed"
          @click="navigate('settings')"
        >
          <Settings class="h-4 w-4" />
        </button>
      </div>

      <template v-else>
        <!-- PA-096：操作行只保留"新对话"；激活工作区名并入工作区 section 头部，
             消除相邻双开关（UX 审核 P2-2）。 -->
        <div class="mt-4 flex items-center gap-1.5" data-testid="session-sidebar-actions">
          <button
            class="flex h-8 min-w-0 flex-1 items-center gap-2 px-1.5 text-[12px] font-medium text-stone-700 disabled:cursor-not-allowed disabled:text-stone-300"
            :class="menuInteractiveClass"
            type="button"
            :disabled="!canCreateSession"
            :title="createSessionTitle"
            data-testid="session-sidebar-new-chat"
            @click="createNewSession"
          >
            <Plus class="h-3.5 w-3.5" />
            <span>新对话</span>
          </button>
        </div>

        <ScrollArea class="mt-3 min-h-0 flex-1" viewport-class="pr-1.5"><div class="flex flex-col gap-2">
          <!-- PA-096：工作区第一优先（需求 #3）——先于会话列表；激活工作区名并入头部。 -->
          <section class="rounded-[0.5rem]" data-testid="session-sidebar-workspace-nav">
            <button
              class="flex w-full items-center justify-between gap-2 px-1.5 py-2 text-left"
              :class="[menuInteractiveClass, 'text-stone-800']"
              type="button"
              data-testid="session-sidebar-workspace-toggle"
              @click="toggleWorkspaceSection"
            >
              <div class="flex min-w-0 items-center gap-2 text-[12px] font-medium text-stone-800">
                <Folder class="h-3.5 w-3.5 shrink-0" />
                <span class="shrink-0">工作区</span>
                <span v-if="!isTauriRuntime" class="shrink-0 text-[10px] font-normal text-stone-400">浏览器模式不可用</span>
              </div>
              <span class="flex min-w-0 shrink-0 items-center gap-1">
                <span
                  class="max-w-[7.5rem] truncate text-[10px] text-stone-400"
                  :title="`当前工作区：${activeWorkspaceName}`"
                  data-testid="session-sidebar-active-workspace-name"
                >{{ activeWorkspaceName }}</span>
                <ChevronDown
                  class="h-4 w-4 shrink-0 text-stone-400 transition"
                  :class="{ 'rotate-180': workspaceSectionOpen }"
                />
              </span>
            </button>

            <div v-if="workspaceSectionOpen" class="space-y-1 py-0.5">
              <div
                v-for="workspace in workspaceList"
                :key="workspace.id"
                class="flex items-center justify-between gap-2 rounded-[0.2rem] px-1.5 py-1"
                :class="workspace.id === activeWorkspaceId ? 'bg-[#f7e3bf]/60' : ''"
                :data-testid="`workspace-row-${workspace.id}`"
              >
                <span class="min-w-0 truncate text-[11px] leading-4 text-stone-700" :title="workspace.rootPath">
                  {{ workspace.name || workspace.id }}
                  <span v-if="workspace.id === activeWorkspaceId" class="ml-1 text-[10px] text-amber-600">激活</span>
                </span>
                <button
                  v-if="workspace.id !== activeWorkspaceId"
                  class="h-5 shrink-0 rounded-[0.35rem] px-1.5 text-[10px] text-stone-500 transition hover:bg-[#f7e3bf] hover:text-stone-900"
                  type="button"
                  :data-testid="`workspace-activate-${workspace.id}`"
                  @click="activateWorkspaceById(workspace.id)"
                >
                  切换
                </button>
              </div>

              <template v-if="isTauriRuntime">
                <button
                  v-if="!workspaceFormOpen"
                  class="flex w-full items-center justify-start gap-1.5 px-1.5 py-1 text-left text-[11px] text-stone-500 transition hover:text-stone-900"
                  type="button"
                  data-testid="workspace-new-open-form"
                  @click="workspaceFormOpen = true"
                >
                  <Plus class="h-3 w-3" />
                  <span>新建工作区</span>
                </button>
                <div v-else class="space-y-1 rounded-[0.35rem] bg-[#fbf4e8]/70 p-1.5">
                  <input
                    v-model="newWorkspaceName"
                    class="w-full rounded-[0.25rem] border border-stone-200 bg-white px-1.5 py-1 text-[11px] text-stone-800 outline-none focus:border-amber-300"
                    placeholder="名称"
                    data-testid="workspace-new-name"
                  />
                  <input
                    v-model="newWorkspaceRootPath"
                    class="w-full rounded-[0.25rem] border border-stone-200 bg-white px-1.5 py-1 text-[11px] text-stone-800 outline-none focus:border-amber-300"
                    placeholder="根路径（如 D:\\projects\\demo）"
                    data-testid="workspace-new-path"
                  />
                  <div class="flex items-center gap-1.5">
                    <button
                      class="h-6 rounded-[0.3rem] bg-[#f3c98d] px-2 text-[11px] font-medium text-stone-900 transition hover:bg-[#f6dfb8] disabled:cursor-not-allowed disabled:opacity-50"
                      type="button"
                      :disabled="workspaceCreating || !newWorkspaceName.trim() || !newWorkspaceRootPath.trim()"
                      data-testid="workspace-new-submit"
                      @click="submitNewWorkspace"
                    >
                      {{ workspaceCreating ? "创建中…" : "创建并激活" }}
                    </button>
                    <button
                      class="h-6 rounded-[0.3rem] px-2 text-[11px] text-stone-500 transition hover:text-stone-900"
                      type="button"
                      @click="workspaceFormOpen = false"
                    >
                      取消
                    </button>
                  </div>
                  <!-- PA-081（实施后审核 P2）：新建失败的可见反馈。 -->
                  <p
                    v-if="workspaceError"
                    class="px-0.5 pt-1 text-[10px] leading-4 text-rose-600"
                    data-testid="workspace-new-error"
                  >
                    {{ workspaceError }}
                  </p>
                </div>
              </template>
              <p v-else class="px-1.5 py-1 text-[10px] leading-4 text-stone-400">
                浏览器预览模式仅提供默认工作区；启动桌面端后可管理项目。
              </p>
            </div>
          </section>

          <section class="rounded-[0.5rem]" data-testid="session-sidebar-session-list">
            <button
              class="flex w-full items-center justify-between gap-2 px-1.5 py-2 text-left"
              :class="[menuInteractiveClass, 'text-stone-800']"
              type="button"
              data-testid="session-sidebar-conversation-toggle"
              @click="toggleConversationSection"
            >
              <div class="flex items-center gap-2 text-[12px] font-medium text-stone-800">
                <MessageSquareMore class="h-3.5 w-3.5" />
                <span>对话</span>
              </div>
              <ChevronDown
                class="h-4 w-4 text-stone-400 transition"
                :class="{ 'rotate-180': conversationOpen }"
              />
            </button>
            <div v-if="conversationOpen" class="space-y-1 py-0.5">
                <p
                  v-if="totalVisibleSessions === 0"
                  class="px-1.5 py-2 text-[11px] leading-4 text-stone-400"
                  data-testid="session-sidebar-empty"
                >
                  暂无对话；发送第一条消息后会自动保存到当前工作区。
                </p>
                <!-- PA-081（实施后审核 P2）：v-else 防止全空时与空组提示双渲染。 -->
                <template v-else v-for="row in sidebarRows" :key="sidebarRowKey(row)">
                  <button
                    v-if="row.kind === 'group'"
                    class="flex w-full items-center justify-between gap-2 rounded-[0.2rem] px-1.5 py-1 text-left transition-colors cursor-pointer hover:bg-[#f6dfb8] hover:text-stone-900"
                    type="button"
                    :data-testid="`session-sidebar-group-${row.key}`"
                    @click="toggleWorkspaceGroup(row.key)"
                  >
                    <span class="flex min-w-0 items-center gap-1.5 text-[11px] font-medium text-stone-700">
                      <FolderOpen v-if="!row.collapsed" class="h-3 w-3 shrink-0 text-stone-400" />
                      <Folder v-else class="h-3 w-3 shrink-0 text-stone-400" />
                      <span class="truncate">{{ row.name }}</span>
                      <span class="shrink-0 text-[10px] text-stone-400">{{ row.count }}</span>
                    </span>
                    <span class="flex shrink-0 items-center gap-1">
                      <!-- design §2：组头"新建对话"快捷入口（在该组 Workspace 下新建）。 -->
                      <button
                        v-if="row.workspaceId"
                        class="inline-flex h-4 w-4 items-center justify-center rounded-[0.2rem] text-stone-400 transition hover:bg-[#f7e3bf] hover:text-stone-900"
                        type="button"
                        title="在此工作区新建对话"
                        :data-testid="`session-sidebar-group-new-${row.key}`"
                        @click.stop="createSessionInWorkspace(row.workspaceId)"
                      >
                        <Plus class="h-3 w-3" />
                      </button>
                      <ChevronDown
                        class="h-3.5 w-3.5 transition"
                        :class="{ 'rotate-180': !row.collapsed }"
                      />
                    </span>
                  </button>
                  <p
                    v-else-if="row.kind === 'group-empty'"
                    class="px-6 py-1 text-[10px] leading-4 text-stone-400"
                    :data-testid="`session-sidebar-group-empty-${row.key}`"
                  >
                    该工作区暂无对话
                  </p>
                  <div
                    v-else
                    class="group rounded-[0.2rem]"
                    :class="[
                      row.session.conversationId === sessionId ? menuSelectedClass : menuInteractiveClass
                    ]"
                    @mouseleave="clearPendingDeleteSession(row.session)"
                  >
                    <div class="flex items-center gap-2 px-1.5 py-1">
                      <button
                        class="min-w-0 flex-1 text-left"
                        type="button"
                        :disabled="Boolean(sessionOperation)"
                        :data-testid="`session-switch-${row.session.conversationId}`"
                        @click="openSessionHistory(row.session.conversationId)"
                      >
                        <div class="flex items-center gap-2 text-[12px] leading-5">
                          <span
                            class="truncate"
                            :class="row.session.conversationId === sessionId ? 'font-medium text-stone-900' : 'text-stone-700'"
                          >
                            {{ sessionHeadline(row.session) }}
                          </span>
                          <span v-if="isTransientSession(row.session)" class="shrink-0 text-[10px] text-amber-600">
                            未保存
                          </span>
                          <span class="shrink-0 text-[10px] text-stone-400">
                            {{ formatSessionTime(row.session.updatedAtMs) }}
                          </span>
                        </div>
                      </button>

                      <span
                        v-if="row.session.conversationId in runtimeStore.completedSessionSet"
                        class="h-2 w-2 shrink-0 cursor-pointer rounded-full bg-emerald-500"
                        title="后台任务已完成，点击查看"
                        :data-testid="`session-completed-${row.session.conversationId}`"
                        @click="openSessionHistory(row.session.conversationId)"
                      />
                      <span
                        v-else-if="row.session.conversationId in runtimeStore.failedSessionSet"
                        class="h-2 w-2 shrink-0 cursor-pointer rounded-full bg-rose-500"
                        title="后台任务执行失败，点击查看"
                        :data-testid="`session-failed-${row.session.conversationId}`"
                        @click="openSessionHistory(row.session.conversationId)"
                      />

                      <button
                        class="pointer-events-none inline-flex shrink-0 cursor-pointer items-center justify-center text-[10px] text-stone-400 opacity-0 transition hover:cursor-pointer hover:text-rose-600 group-hover:pointer-events-auto group-hover:opacity-100 group-focus-within:pointer-events-auto group-focus-within:opacity-100 disabled:cursor-not-allowed disabled:text-stone-300 min-w-[2rem]"
                        :class="
                          isDeletingSession(row.session)
                            ? 'h-5 rounded-[0.35rem] px-1.5 py-1 opacity-100'
                            : pendingDeleteSessionId === row.session.conversationId
                            ? 'h-5 rounded-full bg-rose-200 px-1.5 text-rose-800 hover:bg-rose-300 hover:text-rose-900'
                            : runtimeStore.isSessionRunning(row.session.conversationId) || (isSubmitting && row.session.conversationId === sessionId)
                            ? 'pointer-events-auto h-5 rounded-[0.35rem] px-1.5 py-1 opacity-100 hover:text-amber-600'
                            : 'h-5 rounded-[0.35rem] px-1.5 py-1'
                        "
                        type="button"
                        :disabled="!canDeleteSession(row.session)"
                        :title="
                          isTransientSession(row.session)
                            ? '空白新对话会在切换后自动丢弃，无需单独删除。'
                            : isDeletingSession(row.session)
                              ? '正在删除'
                              : pendingDeleteSessionId === row.session.conversationId
                              ? '确认删除'
                              : '删除对话'
                        "
                        :data-testid="`session-delete-${row.session.conversationId}`"
                        @click.stop="handleDeleteSession(row.session)"
                      >
                        <LoaderCircle
                          v-if="isDeletingSession(row.session)"
                          class="h-3.5 w-3.5 animate-spin text-stone-400"
                          :data-testid="`session-delete-loading-${row.session.conversationId}`"
                        />
                        <LoaderCircle
                          v-else-if="runtimeStore.isSessionRunning(row.session.conversationId) || (isSubmitting && row.session.conversationId === sessionId)"
                          class="h-3.5 w-3.5 animate-spin text-amber-600"
                          :data-testid="`session-running-${row.session.conversationId}`"
                        />
                        <Trash2 v-else-if="pendingDeleteSessionId !== row.session.conversationId" class="h-3.5 w-3.5" />
                        <span v-else class="inline-flex items-center justify-center text-[10px] font-medium text-rose-800">
                          确认
                        </span>
                      </button>
                    </div>
                  </div>
                </template>
                <button
                  v-if="canShowMoreConversations"
                  class="w-full px-1.5 py-1 text-left text-[11px] font-normal leading-5 text-stone-500 transition hover:text-stone-900"
                  type="button"
                  data-testid="session-sidebar-show-more-conversations"
                  @click="showMoreConversations"
                >
                  显示全部
                </button>
            </div>
          </section>

        </div></ScrollArea>

          <!-- PA-096：底部一级导航——遥测/指标、模型配置（一级菜单，需求 #3）、设置。 -->
          <div class="mt-auto space-y-0.5 pt-2">
            <button
              class="flex w-full items-center justify-start gap-2 px-1.5 py-2 text-left"
              :class="
                props.currentPage === 'telemetry'
                  ? menuSelectedClass
                  : `${menuInteractiveClass} text-stone-800`
              "
              type="button"
              :title="isCoding ? 'Trace 与指标遥测读面' : '模型指标监控'"
              data-testid="session-sidebar-nav-telemetry"
              @click="navigate('telemetry')"
            >
              <Activity class="h-3.5 w-3.5" />
              <span class="text-[12px] font-bold leading-4">{{ isCoding ? "遥测" : "指标" }}</span>
            </button>

            <button
              class="flex w-full items-center justify-start gap-2 px-1.5 py-2 text-left"
              :class="
                props.currentPage === 'models'
                  ? menuSelectedClass
                  : `${menuInteractiveClass} text-stone-800`
              "
              type="button"
              title="提供商接入与模型挂载"
              data-testid="session-sidebar-nav-providers"
              @click="navigate('models')"
            >
              <Server class="h-3.5 w-3.5" />
              <span class="text-[12px] font-bold leading-4">模型配置</span>
            </button>

            <button
              class="flex w-full items-center justify-start gap-2 px-1.5 py-2 text-left"
              :class="
                props.currentPage === 'settings'
                  ? menuSelectedClass
                  : `${menuInteractiveClass} text-stone-800`
              "
              type="button"
              data-testid="session-sidebar-nav-settings"
              @click="navigate('settings')"
            >
              <Settings class="h-3.5 w-3.5" />
              <span class="text-[12px] font-bold leading-4">设置</span>
            </button>
          </div>
      </template>
    </div>
  </aside>
</template>
