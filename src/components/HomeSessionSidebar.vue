<script setup lang="ts">
// PA-三级树：侧边栏单一树（「工作区」一级标题行不可折叠 → 工作区二级 → 会话三级；
// 默认/无归属会话无组头平铺置顶；瞬态"新对话"按显式创建目标钉顶归属）。
// 规格：openspec workspace-sidebar-tree delta；裁决与文案见 design/sidebar-copy。
import { computed, reactive, ref } from "vue";
import { storeToRefs } from "pinia";
import {
  Archive,
  ChevronLeft,
  ChevronRight,
  Folder,
  FolderOpen,
  FolderPlus,
  Pencil,
  Plus,
  Settings,
  Trash2
} from "lucide-vue-next";
import PonyBrandIcon from "@/components/PonyBrandIcon.vue";
import ScrollArea from "@/components/ui/ScrollArea.vue";
import DropdownMenu from "@/components/ui/DropdownMenu.vue";
import type { DropdownMenuItemSpec } from "@/components/ui/DropdownMenu.vue";
import ConfirmPopover from "@/components/ui/ConfirmPopover.vue";
import Tooltip from "@/components/ui/Tooltip.vue";
import SessionRow from "@/components/HomeSessionSidebarRow.vue";
import { TooltipProvider } from "reka-ui";
import { useRuntimeStore } from "@/stores/runtime";
import { useUpdateStore } from "@/stores/update";
import { deriveSidebarTree, isVisibleSession } from "@/lib/runtime/sidebar-groups";
import { SIDEBAR_COPY } from "@/lib/runtime/sidebar-copy";
import { DEFAULT_WORKSPACE_ID } from "@/lib/runtime/workspace-constants";
import { isTauriAvailable } from "@/lib/tauri";
import type { SidebarNavigationPage } from "@/types/config";
import type { ChatMessage, SessionOverview } from "@/types/runtime";

const SESSION_SIDEBAR_STORAGE_KEY = "pony-agent.session-sidebar-collapsed.v1";
/** 分区预览上限（平铺区与每个工作区组各自生效；全局"显示全部"解除）。 */
const PARTITION_PREVIEW_LIMIT = 5;

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
const updateStore = useUpdateStore();
const {
  isSubmitting,
  messages,
  sessionId,
  sessionList,
  sessionOperation,
  sessionWorkspaceId,
  workspaceList
} = storeToRefs(runtimeStore);

const collapsed = ref(loadStoredBoolean(SESSION_SIDEBAR_STORAGE_KEY, false));
const isTauriRuntime = isTauriAvailable();

// ── 受控确认（单例状态，per-target 弹层实例锚定行容器） ───────────────────
type ConfirmKind = "workspace-delete" | "session-archive" | "session-delete";
interface ConfirmTarget {
  key: string;
  kind: ConfirmKind;
  title: string;
  count?: number;
  /** 归档/删除会话的目标 id（行匹配用，避免同名标题歧义）。 */
  sessionId?: string;
  /** 删除工作区的目标 id。 */
  workspaceId?: string;
}
const confirmTarget = ref<ConfirmTarget | null>(null);
const confirmState = reactive({ loading: false, error: "" });
const confirmOpen = computed(() => confirmTarget.value !== null);

function openConfirm(target: ConfirmTarget) {
  confirmTarget.value = target;
  confirmState.loading = false;
  confirmState.error = "";
}
function closeConfirm() {
  confirmTarget.value = null;
  confirmState.error = "";
}

// ── 行内重命名 ────────────────────────────────────────────────────────────
const renamingKey = ref<string | null>(null);
const renameDraft = ref("");
const renameError = ref("");
const renamingBusy = ref(false);

function startRename(key: string, currentName: string) {
  renamingKey.value = key;
  renameDraft.value = currentName;
  renameError.value = "";
}
function cancelRename() {
  renamingKey.value = null;
  renameError.value = "";
}

async function submitRename() {
  if (!renamingKey.value || renamingBusy.value) return;
  const draft = renameDraft.value.trim();
  if (!draft) {
    renameError.value = SIDEBAR_COPY.renameEmptyError;
    return;
  }
  if ([...draft].length > 64) {
    renameError.value = SIDEBAR_COPY.renameTooLongError;
    return;
  }
  renamingBusy.value = true;
  try {
    if (renamingKey.value.startsWith("ws:")) {
      const id = renamingKey.value.slice(3);
      if (workspaceList.value.some((w) => w.id !== id && w.name === draft)) {
        renameError.value = SIDEBAR_COPY.renameDuplicateWorkspaceError;
        return;
      }
      const result = await runtimeStore.renameWorkspace(id, draft);
      if (!result.ok) {
        renameError.value = result.error ?? "重命名失败";
        return;
      }
    } else {
      const result = await runtimeStore.renameSession(renamingKey.value.slice(2), draft);
      if (!result.ok) {
        renameError.value = result.error ?? "重命名失败";
        return;
      }
    }
    cancelRename();
  } finally {
    renamingBusy.value = false;
  }
}

// ── 添加工作区（目录选择 → 名称预填 basename） ───────────────────────────
const workspaceFormOpen = ref(false);
const newWorkspaceName = ref("");
const newWorkspaceRootPath = ref("");
const workspaceCreating = ref(false);
const workspaceError = ref("");

async function openAddWorkspaceFlow() {
  const { pickExistingDirectory } = await import("@/lib/runtime/workspace-api");
  try {
    const picked = await pickExistingDirectory();
    if (!picked) return;
    const base = picked.replace(/[/\\]+$/, "").split(/[/\\]/).pop() ?? "";
    workspaceFormOpen.value = true;
    newWorkspaceRootPath.value = picked;
    newWorkspaceName.value = base;
    workspaceError.value = "";
  } catch (error) {
    workspaceError.value = `选择目录失败：${String(error)}`;
  }
}

async function submitNewWorkspace() {
  if (workspaceCreating.value) return;
  const name = newWorkspaceName.value.trim();
  const rootPath = newWorkspaceRootPath.value.trim();
  if (!name) {
    workspaceError.value = SIDEBAR_COPY.renameEmptyError;
    return;
  }
  if ([...name].length > 64) {
    workspaceError.value = SIDEBAR_COPY.renameTooLongError;
    return;
  }
  if (workspaceList.value.some((w) => w.name === name)) {
    workspaceError.value = SIDEBAR_COPY.renameDuplicateWorkspaceError;
    return;
  }
  workspaceError.value = "";
  workspaceCreating.value = true;
  try {
    const record = await runtimeStore.createNewWorkspace(name, rootPath);
    if (record) {
      newWorkspaceName.value = "";
      newWorkspaceRootPath.value = "";
      workspaceFormOpen.value = false;
    } else {
      workspaceError.value = "创建失败：请检查根路径是否有效（可能已存在或重名）。";
    }
  } catch (error) {
    workspaceError.value = `创建失败：${String(error)}`;
  } finally {
    workspaceCreating.value = false;
  }
}

// ── 树派生 ───────────────────────────────────────────────────────────────
const hasPersistableCurrentSession = computed(() => hasPersistableMessages(messages.value));
const hasVisibleCurrentSession = computed(() =>
  sessionList.value.some((session) => session.conversationId === sessionId.value)
);

const transientEntry = computed<SessionOverview | null>(() => {
  if (hasVisibleCurrentSession.value) return null;
  return {
    conversationId: sessionId.value,
    title: "新对话",
    summary: "发送第一条消息后保存到历史",
    turnCount: 0,
    lastReferencedFile: null,
    updatedAtMs: 0,
    workspaceId: sessionWorkspaceId.value === DEFAULT_WORKSPACE_ID ? null : sessionWorkspaceId.value
  };
});

const tree = computed(() =>
  deriveSidebarTree(
    sessionList.value.filter(isVisibleSession),
    workspaceList.value,
    transientEntry.value
      ? { target: sessionWorkspaceId.value, overview: transientEntry.value }
      : undefined
  )
);

const totalVisibleSessions = computed(
  () => tree.value.flatZone.length + tree.value.workspaces.reduce((sum, g) => sum + g.count, 0)
);
const showAllPartitions = ref(false);
const hasHiddenSessions = computed(
  () =>
    tree.value.flatZone.length > PARTITION_PREVIEW_LIMIT
    || tree.value.workspaces.some((g) => g.sessions.length > PARTITION_PREVIEW_LIMIT)
);
function partitionPreview(sessions: readonly SessionOverview[]): readonly SessionOverview[] {
  return showAllPartitions.value ? sessions : sessions.slice(0, PARTITION_PREVIEW_LIMIT);
}

function sessionHeadline(session: SessionOverview) {
  return session.title?.trim() || session.summary?.trim() || session.conversationId;
}

// ── 建会话入口（裁决①：显式目标） ────────────────────────────────────────
function createNewSession() {
  if (props.currentPage !== "home") navigate("home");
  void runtimeStore.createSession(DEFAULT_WORKSPACE_ID);
}
function createSessionInWorkspace(workspaceId: string) {
  if (props.currentPage !== "home") navigate("home");
  void runtimeStore.createSession(workspaceId);
}

// ── 会话守卫与菜单 ────────────────────────────────────────────────────────
function isTransientSession(session: SessionOverview) {
  return session.conversationId === sessionId.value && !hasVisibleCurrentSession.value;
}
function isRunning(session: SessionOverview) {
  return runtimeStore.isSessionRunning(session.conversationId)
    || (isSubmitting.value && session.conversationId === sessionId.value);
}
function guardTitle(session: SessionOverview): string | undefined {
  if (!canMutateSession(session)) {
    if (runtimeStore.isSubmitting && session.conversationId === sessionId.value) {
      return SIDEBAR_COPY.disabledSubmittingTooltip;
    }
    if (isRunning(session)) return SIDEBAR_COPY.disabledRunningTooltip;
    return SIDEBAR_COPY.disabledSubmittingTooltip;
  }
  return undefined;
}
function canMutateSession(session: SessionOverview): boolean {
  if (isTransientSession(session)) return false;
  if (isRunning(session)) return false;
  if (sessionOperation.value) return false;
  return !runtimeStore.isSessionDeleting(session.conversationId);
}
function canDeleteViaMenu(session: SessionOverview): boolean {
  return canMutateSession(session);
}

// 菜单项数组必须保持引用稳定（reka 内部 watcher 对不稳定 props 会循环更新并
// 卸载内容）：按会话/工作区 id 缓存，仅在相关响应式输入变化时重建。
const sessionMenuItemsCache = computed(() => {
  const map = new Map<string, DropdownMenuItemSpec[]>();
  const build = (session: SessionOverview) => {
    const guard = guardTitle(session);
    const mutable = canMutateSession(session);
    if (!isTauriRuntime) {
      map.set(session.conversationId, [
        { id: "delete", label: SIDEBAR_COPY.menuDeleteConversation, icon: Trash2, danger: true }
      ]);
      return;
    }
    map.set(session.conversationId, [
      { id: "rename", label: SIDEBAR_COPY.menuRenameConversation, icon: Pencil, disabled: !mutable, disabledTitle: guard },
      { id: "archive", label: SIDEBAR_COPY.menuArchiveConversation, icon: Archive, disabled: !mutable, disabledTitle: guard },
      { id: "delete", label: SIDEBAR_COPY.menuDeleteConversation, icon: Trash2, danger: true, disabled: !canDeleteViaMenu(session), disabledTitle: guard }
    ]);
  };
  for (const session of tree.value.flatZone) build(session);
  for (const group of tree.value.workspaces) for (const s of group.sessions) build(s);
  return map;
});

function sessionMenuItems(session: SessionOverview): DropdownMenuItemSpec[] {
  return (
    sessionMenuItemsCache.value.get(session.conversationId) ?? [
      { id: "delete", label: SIDEBAR_COPY.menuDeleteConversation, danger: true }
    ]
  );
}

const workspaceMenuItemsCache = computed(() => {
  const map = new Map<string, DropdownMenuItemSpec[]>();
  for (const workspace of workspaceList.value) {
    if (workspace.id === DEFAULT_WORKSPACE_ID) continue;
    map.set(workspace.id, [
      { id: "rename", label: SIDEBAR_COPY.menuRenameWorkspace, icon: Pencil },
      { id: "delete", label: SIDEBAR_COPY.menuDeleteWorkspace, icon: Trash2, danger: true }
    ]);
  }
  return map;
});

function workspaceMenuItems(workspaceId: string): DropdownMenuItemSpec[] {
  return workspaceMenuItemsCache.value.get(workspaceId) ?? [];
}

function onSessionMenuSelect(session: SessionOverview, itemId: string) {
  const headline = sessionHeadline(session);
  if (itemId === "rename") startRename(`s:${session.conversationId}`, headline);
  if (itemId === "archive") {
    openConfirm({
      key: `s:${session.conversationId}:archive`,
      kind: "session-archive",
      title: headline,
      sessionId: session.conversationId
    });
  }
  if (itemId === "delete") {
    openConfirm({
      key: `s:${session.conversationId}:delete`,
      kind: "session-delete",
      title: headline,
      sessionId: session.conversationId
    });
  }
}

function onWorkspaceMenuSelect(workspaceId: string, name: string, itemId: string) {
  if (itemId === "rename") startRename(`ws:${workspaceId}`, name);
  if (itemId === "delete") {
    const count = tree.value.workspaces.find((g) => g.key === workspaceId)?.count ?? 0;
    openConfirm({
      key: `ws:${workspaceId}`,
      kind: "workspace-delete",
      title: name,
      count,
      workspaceId
    });
  }
}

// ── 确认处理器（受控确认标准模板：inflight→await→成功关/失败停留） ────────
async function runConfirm(kind: ConfirmKind, target: ConfirmTarget) {
  confirmState.loading = true;
  confirmState.error = "";
  let outcome: { ok: boolean; error?: string };
  try {
    if (kind === "workspace-delete") {
      outcome = await runtimeStore.deleteWorkspace(target.workspaceId ?? "");
    } else if (kind === "session-archive") {
      outcome = await runtimeStore.archiveSession(target.sessionId ?? "");
    } else {
      try {
        await runtimeStore.deleteSession(target.sessionId ?? "");
        outcome = { ok: true };
      } catch (error) {
        outcome = { ok: false, error: String(error) };
      }
    }
    if (!outcome.ok && confirmOpen.value) {
      confirmState.error = `${SIDEBAR_COPY.confirmFailurePrefix}${outcome.error ?? "未知错误"}`;
      confirmState.loading = false;
      return;
    }
    closeConfirm();
  } finally {
    confirmState.loading = false;
  }
}

// ── 其余 UI 状态 ──────────────────────────────────────────────────────────
const menuInteractiveClass =
  "rounded-[0.2rem] transition-colors cursor-pointer hover:bg-[#f6dfb8] hover:text-stone-900";
const menuSelectedClass = "rounded-[0.2rem] bg-[#f3c98d] text-stone-900";

const hasPersistableMessages = (list: ChatMessage[]) =>
  list.some(
    (message) =>
      (message.role === "user" || message.role === "assistant") && message.content.trim().length > 0
  );

const canCreateSession = computed(
  () => !sessionOperation.value && hasPersistableMessages(messages.value)
);
const createSessionTitle = computed(() => {
  if (isSubmitting.value) {
    return "当前对话正在运行；新建空白对话后，运行会转入后台继续。";
  }
  return hasPersistableCurrentSession.value
    ? "新建一个空白对话到默认工作区，并保留当前已存在的历史会话。"
    : "当前已经是空白新对话，发送首条消息后才会保存到历史。";
});

function loadStoredBoolean(key: string, fallback: boolean) {
  if (typeof window === "undefined") return fallback;
  return window.localStorage.getItem(key) === "1";
}

const sidebarCollapsed = computed(() => collapsed.value || props.forceCollapsed);
const asideClass = computed(() =>
  sidebarCollapsed.value ? "w-[3.4rem] shrink-0" : "w-[17.5rem] shrink-0 xl:w-[18.5rem]"
);

function toggleCollapsed() {
  collapsed.value = !collapsed.value;
  if (typeof window !== "undefined") {
    window.localStorage.setItem(SESSION_SIDEBAR_STORAGE_KEY, collapsed.value ? "1" : "0");
  }
}

function navigate(page: SidebarNavigationPage) {
  emit("navigate", page);
}

function workspaceDeleteDescription(name: string, count: number): string {
  return SIDEBAR_COPY.deleteWorkspaceDescription(name, count);
}

function confirmPopoverProps(
  kind: ConfirmKind,
  target: ConfirmTarget | null
): { title: string; description: string; confirmText: string } {
  if (!target) {
    return { title: "", description: "", confirmText: "" };
  }
  if (kind === "workspace-delete") {
    return {
      title: SIDEBAR_COPY.deleteWorkspaceTitle,
      description: workspaceDeleteDescription(target.title, target.count ?? 0),
      confirmText: SIDEBAR_COPY.deleteWorkspaceConfirm
    };
  }
  if (kind === "session-archive") {
    return {
      title: SIDEBAR_COPY.archiveTitle,
      description: SIDEBAR_COPY.archiveDescription,
      confirmText: SIDEBAR_COPY.archiveConfirm
    };
  }
  return { title: SIDEBAR_COPY.deleteConversationTitle, description: "", confirmText: SIDEBAR_COPY.deleteConversationConfirm };
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

      <!-- 折叠 rail -->
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
          :class="props.currentPage === 'home' ? 'bg-[#f7e3bf] text-stone-900' : 'bg-transparent text-stone-500 hover:bg-[#f7e3bf] hover:text-stone-900'"
          type="button"
          title="对话工作区"
          data-testid="session-sidebar-home-collapsed"
          @click="navigate('home')"
        >
          <FolderOpen class="h-4 w-4" />
          <span
            v-if="Object.keys(runtimeStore.runningSessionMap).length > 0"
            class="absolute -right-0.5 -top-0.5 h-2 w-2 animate-pulse rounded-full bg-amber-500"
            data-testid="session-sidebar-collapsed-running-indicator"
          />
        </button>
        <button
          class="relative mt-auto inline-flex h-8 w-8 items-center justify-center rounded-[0.42rem] transition"
          :class="props.currentPage === 'settings' ? 'bg-[#f7e3bf] text-stone-900' : 'bg-transparent text-stone-500 hover:bg-[#f7e3bf] hover:text-stone-900'"
          type="button"
          title="设置"
          data-testid="session-sidebar-nav-settings-collapsed"
          @click="navigate('settings')"
        >
          <Settings class="h-4 w-4" />
          <span
            v-if="updateStore.hasUpdate"
            class="absolute right-1 top-1 h-2 w-2 rounded-full bg-amber-500"
            title="发现新版本"
            data-testid="session-sidebar-nav-settings-collapsed-update-badge"
          />
        </button>
      </div>

      <template v-else>
        <!-- 展开态：顶层新对话（恒落默认工作区平铺区） -->
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

        <ScrollArea class="mt-3 min-h-0 flex-1" viewport-class="pr-1.5">
          <section data-testid="session-sidebar-tree">
            <!-- 一级标题行（不可折叠）+ 右端添加工作区 -->
            <div class="flex w-full items-center justify-between gap-2 px-1.5 py-2">
              <div class="flex min-w-0 items-center gap-2 text-[12px] font-medium text-stone-800">
                <FolderOpen class="h-3.5 w-3.5 shrink-0" />
                <span class="shrink-0">{{ SIDEBAR_COPY.sectionTitle }}</span>
              </div>
              <TooltipProvider v-if="isTauriRuntime" :delay-duration="300">
                <Tooltip :text="SIDEBAR_COPY.addWorkspaceTooltip" side="bottom">
                  <button
                    class="inline-flex h-6 w-6 shrink-0 items-center justify-center rounded-[0.35rem] text-stone-500 transition hover:bg-[#f7e3bf] hover:text-stone-900"
                    type="button"
                    aria-label="添加工作区"
                    data-testid="workspace-add-button"
                    @click="openAddWorkspaceFlow"
                  >
                    <FolderPlus class="h-3.5 w-3.5" />
                  </button>
                </Tooltip>
              </TooltipProvider>
            </div>

            <!-- 添加表单 -->
            <div v-if="workspaceFormOpen && isTauriRuntime" class="space-y-1 rounded-[0.35rem] bg-[#fbf4e8]/70 p-1.5">
              <input
                v-model="newWorkspaceName"
                class="w-full rounded-[0.25rem] border border-stone-200 bg-white px-1.5 py-1 text-[11px] text-stone-800 outline-none focus:border-amber-300"
                placeholder="名称"
                maxlength="64"
                data-testid="workspace-new-name"
              />
              <input
                v-model="newWorkspaceRootPath"
                class="w-full rounded-[0.25rem] border border-stone-200 bg-white px-1.5 py-1 text-[11px] text-stone-800 outline-none focus:border-amber-300"
                placeholder="根路径"
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
                  {{ workspaceCreating ? "创建中…" : "创建" }}
                </button>
                <button
                  class="h-6 rounded-[0.3rem] px-2 text-[11px] text-stone-500 transition hover:text-stone-900"
                  type="button"
                  data-testid="workspace-new-cancel"
                  @click="workspaceFormOpen = false"
                >
                  取消
                </button>
              </div>
              <p v-if="workspaceError" class="px-0.5 pt-1 text-[10px] leading-4 text-rose-600" data-testid="workspace-new-error">
                {{ workspaceError }}
              </p>
            </div>

            <p
              v-if="totalVisibleSessions === 0"
              class="px-1.5 py-2 text-[11px] leading-4 text-stone-400"
              data-testid="session-sidebar-empty"
            >
              {{ SIDEBAR_COPY.emptyTreeHint }}
            </p>

            <!-- 平铺区 -->
            <div class="space-y-0.5 pt-1">
              <div
                v-for="session in partitionPreview(tree.flatZone)"
                :key="`flat-${session.conversationId}`"
                class="group relative rounded-[0.2rem]"
                :class="session.conversationId === sessionId ? menuSelectedClass : menuInteractiveClass"
                data-testid="flat-zone-session-row"
              >
                <SessionRow
                  :session="session"
                  :menu-items="sessionMenuItems(session)"
                  :selected="session.conversationId === sessionId"
                  :renaming="renamingKey === `s:${session.conversationId}`"
                  :rename-draft="renameDraft"
                  :rename-error="renameError"
                  :renaming-busy="renamingBusy"
                  :hide-menu="isTransientSession(session)"
                  @open-session="(id: string) => { if (currentPage !== 'home') navigate('home'); runtimeStore.switchSession(id); }"
                  @menu-select="(id: string) => onSessionMenuSelect(session, id)"
                  @update:rename-draft="(v: string) => (renameDraft = v)"
                  @submit-rename="submitRename"
                  @cancel-rename="cancelRename"
                />
                <template v-if="confirmTarget?.kind === 'session-archive' && confirmTarget.sessionId === session.conversationId">
                  <ConfirmPopover
                    v-bind="confirmPopoverProps('session-archive', confirmTarget)"
                    :open="true"
                    :loading="confirmState.loading"
                    :error="confirmState.error"
                    @confirm="runConfirm('session-archive', confirmTarget)"
                    @update:open="(v: boolean) => { if (!v) closeConfirm(); }"
                  >
                    <span class="pointer-events-none absolute inset-y-0 right-0 w-0" aria-hidden="true" />
                  </ConfirmPopover>
                </template>
                <template v-else-if="confirmTarget?.kind === 'session-delete' && confirmTarget.sessionId === session.conversationId">
                  <ConfirmPopover
                    v-bind="confirmPopoverProps('session-delete', confirmTarget)"
                    :open="true"
                    :loading="confirmState.loading"
                    :error="confirmState.error"
                    @confirm="runConfirm('session-delete', confirmTarget)"
                    @update:open="(v: boolean) => { if (!v) closeConfirm(); }"
                  >
                    <span class="pointer-events-none absolute inset-y-0 right-0 w-0" aria-hidden="true" />
                  </ConfirmPopover>
                </template>
              </div>
            </div>

            <!-- 各工作区组 -->
            <div
              v-for="group in tree.workspaces"
              :key="group.key"
              class="pt-1.5"
              :data-testid="`workspace-group-${group.key}`"
            >
              <div class="relative flex w-full items-center justify-between gap-2 rounded-[0.2rem] px-1.5 py-1 hover:bg-[#f6dfb8]/60">
                <span class="flex min-w-0 flex-1 items-center gap-1.5 text-[11px] font-medium text-stone-700">
                  <Folder class="h-3 w-3 shrink-0 text-stone-400" />
                  <input
                    v-if="renamingKey === `ws:${group.key}`"
                    v-model="renameDraft"
                    class="min-w-0 flex-1 rounded-[0.2rem] border border-amber-300 bg-white px-1 py-0.5 text-[11px] outline-none"
                    maxlength="64"
                    :disabled="renamingBusy"
                    data-testid="workspace-rename-input"
                    @keydown.enter.prevent="submitRename"
                    @keydown.esc.prevent="cancelRename"
                  />
                  <span v-else class="truncate">{{ group.name }}</span>
                  <span class="shrink-0 text-[10px] text-stone-400">{{ group.count }}</span>
                </span>
                <span v-if="renamingKey !== `ws:${group.key}`" class="flex shrink-0 items-center gap-0.5">
                  <button
                    v-if="isTauriRuntime"
                    class="inline-flex h-4 w-4 items-center justify-center rounded-[0.2rem] text-stone-400 transition hover:bg-[#f7e3bf] hover:text-stone-900"
                    type="button"
                    :title="SIDEBAR_COPY.newConversationHere"
                    :data-testid="`workspace-row-new-${group.key}`"
                    @click="createSessionInWorkspace(group.key)"
                  >
                    <Plus class="h-3 w-3" />
                  </button>
                  <DropdownMenu
                    v-if="isTauriRuntime"
                    :items="workspaceMenuItems(group.key)"
                    @select="(id: string) => onWorkspaceMenuSelect(group.key, group.name, id)"
                  >
                    <button
                      class="inline-flex h-4 w-4 items-center justify-center rounded-[0.2rem] text-stone-400 transition hover:bg-[#f7e3bf] hover:text-stone-900"
                      type="button"
                      :aria-label="`${group.name} 的操作`"
                      :data-testid="`workspace-row-menu-${group.key}`"
                    >
                      <Ellipsis class="h-3 w-3" />
                    </button>
                  </DropdownMenu>
                </span>

                <span v-if="renamingKey === `ws:${group.key}`" class="flex shrink-0 items-center gap-1">
                  <button class="text-[10px] text-stone-500 transition hover:text-stone-900 disabled:opacity-50" type="button" :disabled="renamingBusy" data-testid="workspace-rename-submit" @click="submitRename">确认</button>
                  <button class="text-[10px] text-stone-400 transition hover:text-stone-700" type="button" @click="cancelRename">取消</button>
                </span>
                <p
                  v-if="renamingKey === `ws:${group.key}` && renameError"
                  class="absolute -bottom-3.5 left-6 z-10 text-[10px] leading-3 text-rose-600"
                  data-testid="workspace-rename-error"
                >
                  {{ renameError }}
                </p>

                <ConfirmPopover
                  v-if="confirmTarget?.key === `ws:${group.key}`"
                  :title="SIDEBAR_COPY.deleteWorkspaceTitle"
                  :description="workspaceDeleteDescription(confirmTarget.title, confirmTarget.count ?? 0)"
                  :confirm-text="SIDEBAR_COPY.deleteWorkspaceConfirm"
                  :open="true"
                  :loading="confirmState.loading"
                  :error="confirmState.error"
                  align="end"
                  @confirm="runConfirm('workspace-delete', confirmTarget)"
                  @update:open="(v: boolean) => { if (!v) closeConfirm(); }"
                >
                  <span class="pointer-events-none absolute inset-y-0 left-0 w-0" aria-hidden="true" />
                </ConfirmPopover>
              </div>

              <p
                v-if="group.count === 0"
                class="px-6 py-1 text-[10px] leading-4 text-stone-400"
                :data-testid="`workspace-group-empty-${group.key}`"
              >
                {{ SIDEBAR_COPY.emptyGroupHint }}
              </p>

              <div class="space-y-0.5 pt-0.5">
                <div
                  v-for="session in partitionPreview(group.sessions)"
                  :key="session.conversationId"
                  class="group relative rounded-[0.2rem]"
                  :class="session.conversationId === sessionId ? menuSelectedClass : menuInteractiveClass"
                  :data-testid="`workspace-session-row-${group.key}-${session.conversationId}`"
                >
                  <SessionRow
                    :session="session"
                    :menu-items="sessionMenuItems(session)"
                    :selected="session.conversationId === sessionId"
                    :renaming="renamingKey === `s:${session.conversationId}`"
                    :rename-draft="renameDraft"
                    :rename-error="renameError"
                    :renaming-busy="renamingBusy"
                    :hide-menu="isTransientSession(session)"
                    @open-session="(id: string) => { if (currentPage !== 'home') navigate('home'); runtimeStore.switchSession(id); }"
                    @menu-select="(id: string) => onSessionMenuSelect(session, id)"
                    @update:rename-draft="(v: string) => (renameDraft = v)"
                    @submit-rename="submitRename"
                    @cancel-rename="cancelRename"
                  />
                  <template v-if="confirmTarget?.kind === 'session-archive' && confirmTarget.sessionId === session.conversationId">
                    <ConfirmPopover
                      v-bind="confirmPopoverProps('session-archive', confirmTarget)"
                      :open="true"
                      :loading="confirmState.loading"
                      :error="confirmState.error"
                      @confirm="runConfirm('session-archive', confirmTarget)"
                      @update:open="(v: boolean) => { if (!v) closeConfirm(); }"
                    >
                      <span class="pointer-events-none absolute inset-y-0 right-0 w-0" aria-hidden="true" />
                    </ConfirmPopover>
                  </template>
                  <template v-else-if="confirmTarget?.kind === 'session-delete' && confirmTarget.sessionId === session.conversationId">
                    <ConfirmPopover
                      v-bind="confirmPopoverProps('session-delete', confirmTarget)"
                      :open="true"
                      :loading="confirmState.loading"
                      :error="confirmState.error"
                      @confirm="runConfirm('session-delete', confirmTarget)"
                      @update:open="(v: boolean) => { if (!v) closeConfirm(); }"
                    >
                      <span class="pointer-events-none absolute inset-y-0 right-0 w-0" aria-hidden="true" />
                    </ConfirmPopover>
                  </template>
                </div>
              </div>
            </div>

            <button
              v-if="hasHiddenSessions && !showAllPartitions"
              class="w-full px-1.5 py-1 text-left text-[11px] font-normal leading-5 text-stone-500 transition hover:text-stone-900"
              type="button"
              data-testid="session-sidebar-show-more-conversations"
              @click="showAllPartitions = true"
            >
              显示全部
            </button>
          </section>
        </ScrollArea>

        <div class="mt-auto space-y-0.5 pt-2">
          <button
            class="relative flex w-full items-center justify-start gap-2 px-1.5 py-2 text-left"
            :class="props.currentPage === 'settings' ? menuSelectedClass : `${menuInteractiveClass} text-stone-800`"
            type="button"
            data-testid="session-sidebar-nav-settings"
            @click="navigate('settings')"
          >
            <Settings class="h-3.5 w-3.5" />
            <span class="text-[12px] font-bold leading-4">设置</span>
            <span
              v-if="updateStore.hasUpdate"
              class="absolute right-1.5 top-1.5 h-2 w-2 rounded-full bg-amber-500"
              title="发现新版本"
              data-testid="session-sidebar-nav-settings-update-badge"
            />
          </button>
        </div>
      </template>
    </div>
  </aside>
</template>
