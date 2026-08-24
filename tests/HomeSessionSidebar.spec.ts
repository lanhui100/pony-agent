import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { defineComponent, nextTick } from "vue";
import { flushPromises, mount } from "@vue/test-utils";
import HomeSessionSidebar from "@/components/HomeSessionSidebar.vue";
import { useRuntimeStore } from "@/stores/runtime";
import { useUpdateStore } from "@/stores/update";
import type {
  ChatMessage,
  HistoryBranch,
  HistoryNode,
  SessionOverview
} from "@/types/runtime";

const tauriMocks = vi.hoisted(() => ({
  mockSafeInvoke: vi.fn(),
  mockSafeListen: vi.fn(),
  mockIsTauriAvailable: vi.fn()
}));

vi.mock("@/lib/tauri", () => ({
  safeInvoke: tauriMocks.mockSafeInvoke,
  safeListen: tauriMocks.mockSafeListen,
  isTauriAvailable: tauriMocks.mockIsTauriAvailable
}));

const ScrollAreaStub = defineComponent({
  template: '<div class="scroll-area-stub"><slot /></div>'
});

function createMessage(partial: Partial<ChatMessage> = {}): ChatMessage {
  return {
    id: partial.id ?? "msg-1",
    turnId: partial.turnId ?? "turn-1",
    role: partial.role ?? "user",
    content: partial.content ?? "hello",
    status: partial.status ?? "done",
    tokenCount: partial.tokenCount ?? null,
    reasoningContent: partial.reasoningContent ?? null,
    modelName: partial.modelName ?? null,
    toolName: partial.toolName ?? null,
    detail: partial.detail ?? null,
    durationSeconds: partial.durationSeconds ?? null
  };
}

function createSession(partial: Partial<SessionOverview> = {}): SessionOverview {
  return {
    conversationId: partial.conversationId ?? "session-1",
    title: partial.title ?? "Session 1",
    summary: partial.summary ?? "Summary",
    turnCount: partial.turnCount ?? 1,
    lastReferencedFile: partial.lastReferencedFile ?? null,
    updatedAtMs: partial.updatedAtMs ?? 1000,
    // PA-081：workspaceId 必须透传（分组归属的输入；缺失 → 默认组）。
    workspaceId: partial.workspaceId ?? null
  };
}

function createHistoryNode(partial: Partial<HistoryNode> = {}): HistoryNode {
  return {
    nodeId: partial.nodeId ?? "node-1",
    sessionId: partial.sessionId ?? "session-current",
    branchId: partial.branchId ?? "branch-main",
    kind: partial.kind ?? "turn_committed",
    summary: partial.summary ?? "summary",
    createdAtMs: partial.createdAtMs ?? 1000,
    parentNodeId: partial.parentNodeId ?? null,
    forkedFromNodeId: partial.forkedFromNodeId ?? null,
    transcriptRef: partial.transcriptRef ?? null,
    runRef: partial.runRef ?? null,
    workspaceRef: partial.workspaceRef ?? { kind: "none", rollbackCapable: false }
  };
}

function createHistoryBranch(partial: Partial<HistoryBranch> = {}): HistoryBranch {
  return {
    branchId: partial.branchId ?? "branch-main",
    sessionId: partial.sessionId ?? "session-current",
    baseNodeId: partial.baseNodeId ?? "node-1",
    headNodeId: partial.headNodeId ?? "node-2",
    forkedFromBranchId: partial.forkedFromBranchId ?? null,
    forkedFromNodeId: partial.forkedFromNodeId ?? null,
    label: partial.label ?? "main",
    createdAtMs: partial.createdAtMs ?? 1000,
    updatedAtMs: partial.updatedAtMs ?? 2000
  };
}

function mountSidebar(currentPage: "home" | "settings" | null = "home", forceCollapsed = false) {
  return mount(HomeSessionSidebar, {
    props: {
      currentPage,
      forceCollapsed
    },
    global: {
      stubs: {
        ScrollArea: ScrollAreaStub
      }
    }
  });
}

function seedSidebarSessions() {
  const runtimeStore = useRuntimeStore();
  runtimeStore.$patch({
    sessionId: "session-current",
    sessionOperation: null,
    isSubmitting: false,
    messages: [createMessage({ content: "existing content" })],
    sessionList: [
      createSession({
        conversationId: "session-current",
        title: "Current session",
        summary: "Current summary"
      }),
      createSession({
        conversationId: "session-other",
        title: "Other session",
        summary: "Other summary",
        updatedAtMs: 2000
      })
    ]
  });
}

function seedManySidebarSessions(count = 7) {
  const runtimeStore = useRuntimeStore();
  runtimeStore.$patch({
    sessionId: "session-1",
    sessionOperation: null,
    isSubmitting: false,
    messages: [createMessage({ content: "existing content" })],
    sessionList: Array.from({ length: count }, (_, index) =>
      createSession({
        conversationId: `session-${index + 1}`,
        title: `Session ${index + 1}`,
        summary: `Summary ${index + 1}`,
        updatedAtMs: new Date(`2026-06-0${Math.min(index + 1, 7)}T12:00:00+08:00`).getTime()
      })
    )
  });
}

describe("HomeSessionSidebar", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    window.localStorage.clear();
    setActivePinia(createPinia());
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-06-07T15:00:00+08:00"));
    tauriMocks.mockSafeListen.mockResolvedValue(() => {});
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    vi.spyOn(console, "info").mockImplementation(() => {});
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  it("disables create and delete controls for a transient empty session", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionId: "session-transient",
      sessionList: [],
      sessionOperation: null,
      isSubmitting: false,
      messages: []
    });

        const wrapper = mountSidebar();
    await nextTick();

    expect(wrapper.text()).toContain("未保存");
    expect(wrapper.get('[data-testid="session-sidebar-new-chat"]').attributes("disabled")).toBeDefined();
    expect(wrapper.get('[data-testid="session-delete-session-transient"]').attributes("disabled")).toBeDefined();
  });

  it("keeps the new-chat control available while a turn is submitting", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionId: "session-current",
      sessionList: [
        createSession({
          conversationId: "session-current",
          title: "Current session",
          summary: "Current summary"
        })
      ],
      sessionOperation: null,
      isSubmitting: true,
      messages: [createMessage({ content: "running turn" })]
    });

    const wrapper = mountSidebar();
    await nextTick();

    // 运行中的 turn 应转入后台而不是阻塞新建对话
    expect(wrapper.get('[data-testid="session-sidebar-new-chat"]').attributes("disabled")).toBeUndefined();
  });

  it("keeps a saved failed session visible instead of treating it as transient", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionId: "session-failed-history",
      sessionList: [
        createSession({
          conversationId: "session-failed-history",
          title: "失败历史",
          summary: "hook failed",
          turnCount: 1,
          updatedAtMs: 1000
        })
      ],
      sessionOperation: null,
      isSubmitting: false,
      messages: []
    });

    const wrapper = mountSidebar();
    await nextTick();

    expect(wrapper.text()).not.toContain("未保存");
    expect(wrapper.find('[data-testid="session-switch-session-failed-history"]').exists()).toBe(true);
    expect(
      wrapper.get('[data-testid="session-delete-session-failed-history"]').attributes("disabled")
    ).toBeUndefined();
  });

  it("disables switching and deletion during a session operation", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionId: "session-current",
      sessionOperation: "deleting",
      isSubmitting: false,
      messages: [createMessage({ content: "existing content" })],
      sessionList: [
        createSession({
          conversationId: "session-current",
          title: "Saved session",
          summary: "Current summary"
        }),
        createSession({
          conversationId: "session-other",
          title: "Other session",
          summary: "Other summary",
          updatedAtMs: 2000
        })
      ]
    });

    const wrapper = mountSidebar();
    await nextTick();

    expect(wrapper.get('[data-testid="session-sidebar-new-chat"]').attributes("disabled")).toBeDefined();
    expect(wrapper.get('[data-testid="session-switch-session-current"]').attributes("disabled")).toBeDefined();
    expect(wrapper.get('[data-testid="session-switch-session-other"]').attributes("disabled")).toBeDefined();
    expect(wrapper.get('[data-testid="session-delete-session-current"]').attributes("disabled")).toBeDefined();
    expect(wrapper.get('[data-testid="session-delete-session-other"]').attributes("disabled")).toBeDefined();
  });

  it("renders each saved session as a single compact line with relative time", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionId: "session-current",
      sessionOperation: null,
      isSubmitting: false,
      messages: [createMessage({ content: "existing content" })],
      sessionList: [
        createSession({
          conversationId: "session-current",
          title: "Current session",
          summary: "Current summary",
          updatedAtMs: new Date("2026-06-07T09:30:00+08:00").getTime()
        }),
        createSession({
          conversationId: "session-other",
          title: "Other session",
          summary: "Other summary",
          turnCount: 3,
          lastReferencedFile: "src/demo.ts",
          updatedAtMs: new Date("2026-06-05T12:00:00+08:00").getTime()
        })
      ]
    });

    const wrapper = mountSidebar();
    await nextTick();

    const currentRow = wrapper.get('[data-testid="session-switch-session-current"]').text();
    const otherRow = wrapper.get('[data-testid="session-switch-session-other"]').text();

    expect(currentRow).toContain("Current session");
    expect(currentRow).toMatch(/\d{2}:\d{2}/);
    expect(currentRow).not.toContain("Current summary");
    expect(otherRow).toContain("Other session");
    expect(otherRow).toContain("2天前");
    expect(otherRow).not.toContain("Other summary");
    expect(otherRow).not.toContain("3 轮");
    expect(otherRow).not.toContain("src/demo.ts");
  });

  it("shows only the first five sessions per group and reveals all groups at once via show-all", async () => {
    seedManySidebarSessions(12);

    const wrapper = mountSidebar();
    await nextTick();

    // 组预览上限：每组前 5 条；计数徽标按全量计算。
    expect(wrapper.find('[data-testid="session-switch-session-1"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="session-switch-session-5"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="session-switch-session-6"]').exists()).toBe(false);
    expect(wrapper.get('[data-testid="session-sidebar-group-default"]').text()).toContain("12");
    expect(wrapper.get('[data-testid="session-sidebar-show-more-conversations"]').text()).toContain("显示全部");

    // PA-081："显示全部"一键解除所有组的会话数上限（不做 per-group 分页）。
    await wrapper.get('[data-testid="session-sidebar-show-more-conversations"]').trigger("click");
    await nextTick();

    expect(wrapper.find('[data-testid="session-switch-session-10"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="session-switch-session-12"]').exists()).toBe(true);
    expect(wrapper.get('[data-testid="session-sidebar-group-default"]').text()).toContain("12");
    expect(wrapper.find('[data-testid="session-sidebar-show-more-conversations"]').exists()).toBe(false);
  });

  it("allows collapsing and reopening the conversation section", async () => {
    seedManySidebarSessions(6);

    const wrapper = mountSidebar();
    await nextTick();

    expect(wrapper.find('[data-testid="session-switch-session-1"]').exists()).toBe(true);

    await wrapper.get('[data-testid="session-sidebar-conversation-toggle"]').trigger("click");
    await nextTick();

    expect(wrapper.find('[data-testid="session-switch-session-1"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="session-sidebar-show-more-conversations"]').exists()).toBe(false);

    await wrapper.get('[data-testid="session-sidebar-conversation-toggle"]').trigger("click");
    await nextTick();

    expect(wrapper.find('[data-testid="session-switch-session-1"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="session-switch-session-5"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="session-switch-session-6"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="session-sidebar-show-more-conversations"]').exists()).toBe(true);
  });

  it("requires a second click before deleting a session", async () => {
    seedSidebarSessions();

    const runtimeStore = useRuntimeStore();
    const deleteSessionSpy = vi.spyOn(runtimeStore, "deleteSession").mockResolvedValue();
    const wrapper = mountSidebar();
    await nextTick();

    const deleteButton = wrapper.get('[data-testid="session-delete-session-other"]');
    await deleteButton.trigger("click");
    await nextTick();

    expect(deleteSessionSpy).not.toHaveBeenCalled();
    expect(wrapper.get('[data-testid="session-delete-session-other"]').text()).toContain("确认");

    await wrapper.get('[data-testid="session-delete-session-other"]').trigger("click");

    expect(deleteSessionSpy).toHaveBeenCalledWith("session-other");
  });

  it("shows delete and confirmation controls only while a session row is hovered or focused", async () => {
    seedSidebarSessions();

    const wrapper = mountSidebar();
    await nextTick();

    const deleteButton = wrapper.get('[data-testid="session-delete-session-other"]');
    expect(deleteButton.classes()).toContain("opacity-0");
    expect(deleteButton.classes()).toContain("pointer-events-none");
    expect(deleteButton.classes()).toContain("group-hover:opacity-100");
    expect(deleteButton.classes()).toContain("group-hover:pointer-events-auto");
    expect(deleteButton.classes()).toContain("group-focus-within:opacity-100");

    await deleteButton.trigger("click");
    await nextTick();

    const confirmButton = wrapper.get('[data-testid="session-delete-session-other"]');
    expect(confirmButton.text()).toContain("确认");
    expect(confirmButton.text()).not.toContain("确认？");
    expect(confirmButton.classes()).toContain("opacity-0");
    expect(confirmButton.classes()).toContain("group-hover:opacity-100");

    await confirmButton.element.parentElement?.parentElement?.dispatchEvent(
      new MouseEvent("mouseleave", { bubbles: true })
    );
    await nextTick();

    const restoredButton = wrapper.get('[data-testid="session-delete-session-other"]');
    expect(restoredButton.text()).not.toContain("确认");
    expect(restoredButton.find("svg").exists()).toBe(true);
  });

  it("shows a loading indicator for the session being deleted", async () => {
    seedSidebarSessions();

    const runtimeStore = useRuntimeStore();
    runtimeStore.deletingSessionSet["session-other"] = true;
    let finishDelete: (() => void) | null = null;
    const deleteSessionSpy = vi.spyOn(runtimeStore, "deleteSession").mockImplementation(
      () =>
        new Promise<void>((resolve) => {
          finishDelete = resolve;
        })
    );
    const wrapper = mountSidebar();
    await nextTick();

    expect(wrapper.find('[data-testid="session-delete-loading-session-other"]').exists()).toBe(true);
    expect(wrapper.get('[data-testid="session-delete-session-other"]').attributes("title")).toBe("正在删除");
    expect(wrapper.get('[data-testid="session-delete-session-other"]').classes()).toContain("opacity-100");
    expect(wrapper.get('[data-testid="session-delete-session-other"]').text()).not.toContain("确认");

    delete runtimeStore.deletingSessionSet["session-other"];
    finishDelete?.();
    await Promise.resolve();
    await nextTick();

    expect(wrapper.find('[data-testid="session-delete-loading-session-other"]').exists()).toBe(false);
  });

  it("keeps other saved sessions deletable while one history delete is in flight", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionId: "session-current",
      sessionOperation: null,
      isSubmitting: false,
      messages: [createMessage({ content: "existing content" })],
      sessionList: [
        createSession({
          conversationId: "session-current",
          title: "Current session",
          summary: "Current summary"
        }),
        createSession({
          conversationId: "session-other",
          title: "Other session",
          summary: "Other summary",
          updatedAtMs: 2000
        }),
        createSession({
          conversationId: "session-third",
          title: "Third session",
          summary: "Third summary",
          updatedAtMs: 3000
        })
      ]
    });
    runtimeStore.deletingSessionSet["session-other"] = true;

    const wrapper = mountSidebar();
    await nextTick();

    expect(wrapper.get('[data-testid="session-delete-session-other"]').attributes("disabled")).toBeDefined();
    expect(wrapper.find('[data-testid="session-delete-loading-session-other"]').exists()).toBe(true);
    expect(wrapper.get('[data-testid="session-delete-session-third"]').attributes("disabled")).toBeUndefined();
  });

  it("keeps create actions above workspace and session sections (workspace first, PA-096)", async () => {
    seedSidebarSessions();

    const wrapper = mountSidebar();
    await nextTick();

    const actions = wrapper.get('[data-testid="session-sidebar-actions"]').element;
    const workspace = wrapper.get('[data-testid="session-sidebar-workspace-nav"]').element;
    const sessionList = wrapper.get('[data-testid="session-sidebar-session-list"]').element;

    expect(actions.compareDocumentPosition(workspace) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(workspace.compareDocumentPosition(sessionList) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it("navigates home before switching when a concrete session is opened from another page", async () => {
    seedSidebarSessions();

    const runtimeStore = useRuntimeStore();
    const switchSessionSpy = vi.spyOn(runtimeStore, "switchSession").mockResolvedValue();
    const wrapper = mountSidebar("settings");
    await nextTick();

    await wrapper.get('[data-testid="session-switch-session-other"]').trigger("click");

    expect(wrapper.emitted("navigate")).toEqual([["home"]]);
    expect(switchSessionSpy).toHaveBeenCalledWith("session-other");
  });

  it("switches sessions inside home without emitting a redundant navigation", async () => {
    seedSidebarSessions();

    const runtimeStore = useRuntimeStore();
    const switchSessionSpy = vi.spyOn(runtimeStore, "switchSession").mockResolvedValue();
    const wrapper = mountSidebar("home");
    await nextTick();

    await wrapper.get('[data-testid="session-switch-session-other"]').trigger("click");

    expect(switchSessionSpy).toHaveBeenCalledWith("session-other");
    expect(wrapper.emitted("navigate")).toBeUndefined();
  });

  it("keeps settings as the only first-level nav entry after ADR 0013 slimming", async () => {
    seedSidebarSessions();

    const wrapper = mountSidebar();
    await nextTick();

    // 底部一级导航只剩"设置"；观测/模型配置入口已分别移至右栏浮动按钮与配置页 tab
    expect(wrapper.get('[data-testid="session-sidebar-nav-settings"]').text()).toContain("设置");
    expect(wrapper.find('[data-testid="session-sidebar-nav-telemetry"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="session-sidebar-nav-providers"]').exists()).toBe(false);

    // "模型管理"二级折叠组已移除
    expect(wrapper.find('[data-testid="session-sidebar-model-toggle"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="session-sidebar-nav-model-monitor"]').exists()).toBe(false);
  });

  it("keeps the top brand entry and no longer renders a separate home item", async () => {
    seedSidebarSessions();

    const wrapper = mountSidebar();
    await nextTick();

    expect(wrapper.get('[data-testid="session-sidebar-brand"]').text()).toContain("Pony Agent");
    expect(wrapper.text()).not.toContain("主页");
  });

  it("brand click routes back to the home workspace", async () => {
    seedSidebarSessions();

    const wrapper = mountSidebar("settings");
    await nextTick();

    await wrapper.get('[data-testid="session-sidebar-brand"]').trigger("click");
    expect(wrapper.emitted("navigate")).toEqual([["home"]]);
  });

  it("shares the same warm orange hover and selected guardrails across menu items", async () => {
    seedSidebarSessions();

    // 未选中态：菜单项带暖橙 hover
    const wrapper = mountSidebar("home");
    await nextTick();

    const newChat = wrapper.get('[data-testid="session-sidebar-new-chat"]');
    const settingsItem = wrapper.get('[data-testid="session-sidebar-nav-settings"]');
    const sessionItem = wrapper.get('[data-testid="session-switch-session-current"]').element.parentElement?.parentElement;

    expect(newChat.classes()).toContain("hover:bg-[#f6dfb8]");
    expect(settingsItem.classes()).toContain("hover:bg-[#f6dfb8]");
    // 当前会话行使用选中底色
    expect(sessionItem?.className).toContain("bg-[#f3c98d]");

    // 选中态：设置项切换为实心底色（menuSelectedClass）
    const selectedWrapper = mountSidebar("settings");
    await nextTick();
    const selectedSettings = selectedWrapper.get('[data-testid="session-sidebar-nav-settings"]');

    expect(selectedSettings.classes()).not.toContain("hover:bg-[#f6dfb8]");
    expect(selectedSettings.classes()).toContain("bg-[#f3c98d]");
    expect(selectedSettings.classes()).toContain("rounded-[0.2rem]");
  });

  it("keeps the four key entries in collapsed mode (ADR 0013: observation/providers entries removed)", async () => {
    seedSidebarSessions();

    const wrapper = mountSidebar();
    await nextTick();

    await wrapper.get('[data-testid="session-sidebar-collapse"]').trigger("click");
    await nextTick();

    expect(wrapper.find('[data-testid="session-sidebar-brand"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="session-sidebar-new-chat-collapsed"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="session-sidebar-home-collapsed"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="session-sidebar-nav-telemetry-collapsed"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="session-sidebar-nav-providers-collapsed"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="session-sidebar-nav-settings-collapsed"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="session-sidebar-nav-model-monitor-collapsed"]').exists()).toBe(false);
    expect(wrapper.text()).not.toContain("主页");
  });

  it("uses narrower horizontal padding and smooth width or padding transitions when collapsed", async () => {
    seedSidebarSessions();

    const wrapper = mountSidebar();
    await nextTick();

    expect(wrapper.element.className).toContain("transition-[width]");
    expect(wrapper.element.className).toContain("duration-200");
    expect(wrapper.element.className).toContain("ease-in-out");

    await wrapper.get('[data-testid="session-sidebar-collapse"]').trigger("click");
    await nextTick();

    const innerShell = wrapper.get("aside > div");
    expect(innerShell.element.className).toContain("transition-[padding]");
    expect(innerShell.element.className).toContain("duration-200");
    expect(innerShell.element.className).toContain("ease-in-out");
    expect(innerShell.element.className).toContain("px-1");
    expect(innerShell.element.className).not.toContain("px-1.5");
    expect(innerShell.element.className).not.toContain("px-2");
  });

  it("keeps a fixed-width sidebar in expanded mode instead of stretching full width", async () => {
    seedSidebarSessions();

    const wrapper = mountSidebar();
    await nextTick();

    expect(wrapper.element.className).toContain("w-[17.5rem]");
    expect(wrapper.element.className).not.toContain("w-full");
  });

  it("collapsed home icon routes back to home workspace", async () => {
    seedSidebarSessions();

    const wrapper = mountSidebar("settings");
    await nextTick();

    await wrapper.get('[data-testid="session-sidebar-collapse"]').trigger("click");
    await nextTick();

    await wrapper.get('[data-testid="session-sidebar-home-collapsed"]').trigger("click");

    expect(wrapper.emitted("navigate")).toEqual([["home"]]);
  });

  it("persists collapse state and exposes collapsed settings navigation", async () => {
    seedSidebarSessions();

    const wrapper = mountSidebar("home");
    await nextTick();

    await wrapper.get('[data-testid="session-sidebar-collapse"]').trigger("click");
    await nextTick();

    expect(window.localStorage.getItem("pony-agent.session-sidebar-collapsed.v1")).toBe("1");

    await wrapper.get('[data-testid="session-sidebar-nav-settings-collapsed"]').trigger("click");

    expect(wrapper.emitted("navigate")).toEqual([["settings"]]);
  });

  it("forwards create and delete actions to the runtime store when enabled", async () => {
    seedSidebarSessions();

    const runtimeStore = useRuntimeStore();
    const createSessionSpy = vi.spyOn(runtimeStore, "createSession").mockResolvedValue();
    const deleteSessionSpy = vi.spyOn(runtimeStore, "deleteSession").mockResolvedValue();
    const wrapper = mountSidebar();
    await nextTick();

    await wrapper.get('[data-testid="session-sidebar-new-chat"]').trigger("click");
    await wrapper.get('[data-testid="session-delete-session-other"]').trigger("click");
    await nextTick();
    await wrapper.get('[data-testid="session-delete-session-other"]').trigger("click");

    expect(createSessionSpy).toHaveBeenCalledTimes(1);
    expect(deleteSessionSpy).toHaveBeenCalledWith("session-other");
  });

  it("routes back to the home workspace before creating a new session", async () => {
    seedSidebarSessions();

    const runtimeStore = useRuntimeStore();
    const createSessionSpy = vi.spyOn(runtimeStore, "createSession").mockResolvedValue();
    const wrapper = mountSidebar("settings");
    await nextTick();

    await wrapper.get('[data-testid="session-sidebar-new-chat"]').trigger("click");

    expect(wrapper.emitted("navigate")).toEqual([["home"]]);
    expect(createSessionSpy).toHaveBeenCalledTimes(1);
  });

  it("does not render the legacy sidebar history control surface", async () => {
    seedSidebarSessions();

    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      visibleNodeId: "node-old",
      branchHeadNodeId: "node-head",
      activeBranchId: "branch-main",
      historyCursorMode: "historical_dirty",
      historyNodes: [createHistoryNode({ nodeId: "node-old" })],
      historyBranches: [createHistoryBranch({ branchId: "branch-main", headNodeId: "node-head" })]
    });

    const wrapper = mountSidebar();
    await nextTick();

    expect(wrapper.find('[data-testid="session-sidebar-history-panel"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="session-sidebar-history-toggle"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="session-sidebar-history-graph"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="session-sidebar-history-restore"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="session-sidebar-history-fork"]').exists()).toBe(false);
  });

  // ── PA-081：Workspace 两级树 ──────────────────────────────────────────────

  it("groups sessions under their workspace and keeps orphan sessions in a trailing ungrouped group", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionId: "session-current",
      sessionOperation: null,
      isSubmitting: false,
      messages: [createMessage({ content: "existing content" })],
      workspaceList: [
        { id: "default", name: "默认工作区", rootPath: "D:/default" },
        { id: "ws-demo", name: "演示项目", rootPath: "D:/demo" }
      ],
      workspaceListLoaded: true,
      activeWorkspaceId: "default",
      sessionList: [
        createSession({ conversationId: "session-current", title: "默认组会话" }),
        createSession({ conversationId: "session-demo", title: "演示组会话", workspaceId: "ws-demo" }),
        createSession({ conversationId: "session-orphan", title: "孤儿会话", workspaceId: "ws-gone" })
      ]
    });

    const wrapper = mountSidebar();
    await nextTick();

    // store 侧：patch 的 workspaceId 必须原样保留（防回归探针）。
    expect(runtimeStore.sessionList.map((session) => session.workspaceId)).toEqual([
      null,
      "ws-demo",
      "ws-gone"
    ]);

    // 组顺序：注册表顺序，孤儿"未分组"最后（精确选择器，避免空态提示前缀碰撞）。
    const defaultHeader = wrapper.get('[data-testid="session-sidebar-group-default"]').element;
    const demoHeader = wrapper.get('[data-testid="session-sidebar-group-ws-demo"]').element;
    const orphanHeader = wrapper
      .get('[data-testid="session-sidebar-group-__ungrouped__"]')
      .element;
    expect(defaultHeader.textContent).toContain("默认工作区");
    expect(demoHeader.textContent).toContain("演示项目");
    expect(orphanHeader.textContent).toContain("未分组");
    expect(
      defaultHeader.compareDocumentPosition(demoHeader) & Node.DOCUMENT_POSITION_FOLLOWING
    ).toBeTruthy();
    expect(
      demoHeader.compareDocumentPosition(orphanHeader) & Node.DOCUMENT_POSITION_FOLLOWING
    ).toBeTruthy();

    // 行为断言成员归属：折叠 ws-demo 仅隐藏演示组会话；折叠未分组隐藏孤儿。
    await wrapper.get('[data-testid="session-sidebar-group-ws-demo"]').trigger("click");
    await nextTick();
    expect(wrapper.find('[data-testid="session-switch-session-demo"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="session-switch-session-orphan"]').exists()).toBe(true);

    await wrapper.get('[data-testid="session-sidebar-group-__ungrouped__"]').trigger("click");
    await nextTick();
    expect(wrapper.find('[data-testid="session-switch-session-orphan"]').exists()).toBe(false);
  });

  it("places the transient new-chat entry into the active workspace group", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionId: "session-transient",
      sessionOperation: null,
      isSubmitting: false,
      messages: [],
      workspaceList: [
        { id: "default", name: "默认工作区", rootPath: "D:/default" },
        { id: "ws-demo", name: "演示项目", rootPath: "D:/demo" }
      ],
      workspaceListLoaded: true,      activeWorkspaceId: "ws-demo",
      sessionList: [
        createSession({ conversationId: "session-saved", title: "默认组会话" })
      ]
    });

    const wrapper = mountSidebar();
    await nextTick();

    // 瞬态条目（未保存）应与激活组同现：激活组（演示项目）内可见瞬态行。
    const demoGroupHeader = wrapper.get('[data-testid="session-sidebar-group-ws-demo"]');
    expect(demoGroupHeader.text()).toContain("演示项目");
    expect(wrapper.find('[data-testid="session-switch-session-transient"]').exists()).toBe(true);
    expect(wrapper.text()).toContain("未保存");
    // 默认组仅含已保存会话。
    expect(wrapper.find('[data-testid="session-switch-session-saved"]').exists()).toBe(true);
  });

  it("persists workspace group collapse state across remounts and ignores stale keys", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionId: "session-current",
      sessionOperation: null,
      isSubmitting: false,
      messages: [createMessage({ content: "existing content" })],
      workspaceList: [{ id: "default", name: "默认工作区", rootPath: "D:/default" }],
      workspaceListLoaded: true,
      activeWorkspaceId: "default",
      sessionList: [
        createSession({ conversationId: "session-current", title: "默认组会话" })
      ]
    });

    const wrapper = mountSidebar();
    await nextTick();

    expect(wrapper.find('[data-testid="session-switch-session-current"]').exists()).toBe(true);
    await wrapper.get('[data-testid="session-sidebar-group-default"]').trigger("click");
    await nextTick();
    expect(wrapper.find('[data-testid="session-switch-session-current"]').exists()).toBe(false);
    expect(window.localStorage.getItem("pony-agent.session-sidebar-workspace-groups.v1")).toBe(
      JSON.stringify(["default"])
    );

    wrapper.unmount();

    // 预置过期 key（组已不存在）→ 重挂载时被忽略，不影响合法折叠态。
    window.localStorage.setItem(
      "pony-agent.session-sidebar-workspace-groups.v1",
      JSON.stringify(["default", "ws-stale"])
    );
    const remounted = mountSidebar();
    await nextTick();

    // 过期 key 被读取忽略（不影响合法折叠态）；存储本体允许保留，仅在下次
    // 用户切换时按当前合法组裁剪。
    expect(remounted.find('[data-testid="session-switch-session-current"]').exists()).toBe(false);
  });

  it("activating a workspace only switches the creation target and never hides other groups", async () => {
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "workspace_list") {
        return [
          { id: "default", name: "默认工作区", rootPath: "D:/default" },
          { id: "ws-demo", name: "演示项目", rootPath: "D:/demo" }
        ];
      }
      return null;
    });
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionId: "session-current",
      sessionOperation: null,
      isSubmitting: false,
      messages: [createMessage({ content: "existing content" })],
      activeWorkspaceId: "default",
      sessionList: [
        createSession({ conversationId: "session-current", title: "默认组会话" }),
        createSession({ conversationId: "session-demo", title: "演示组会话", workspaceId: "ws-demo" })
      ]
    });

    const wrapper = mountSidebar();
    await flushPromises();
    await nextTick();

    await wrapper.get('[data-testid="session-sidebar-workspace-toggle"]').trigger("click");
    await wrapper.get('[data-testid="workspace-activate-ws-demo"]').trigger("click");

    expect(runtimeStore.activeWorkspaceId).toBe("ws-demo");
    expect(window.localStorage.getItem("pony-agent.active-workspace.v1")).toBe("ws-demo");
    // AC2：切换激活组不隐藏其他组——两组会话均保持可见。
    expect(wrapper.find('[data-testid="session-switch-session-current"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="session-switch-session-demo"]').exists()).toBe(true);
  });

  it("creates a workspace from the manage form and auto-activates it", async () => {
    const runtimeStore = useRuntimeStore();
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
      if (command === "workspace_list") {
        return [{ id: "default", name: "默认工作区", rootPath: "D:/default" }];
      }
      if (command === "workspace_create") {
        return {
          id: "ws-created",
          name: String(args?.name ?? ""),
          rootPath: String(args?.rootPath ?? "")
        };
      }
      return null;
    });

    runtimeStore.$patch({
      sessionId: "session-current",
      sessionOperation: null,
      isSubmitting: false,
      messages: [createMessage({ content: "existing content" })],
      sessionList: [createSession({ conversationId: "session-current", title: "默认组会话" })]
    });

    const wrapper = mountSidebar();
    await nextTick();
    await nextTick();

    await wrapper.get('[data-testid="session-sidebar-workspace-toggle"]').trigger("click");
    await wrapper.get('[data-testid="workspace-new-open-form"]').trigger("click");
    await wrapper.get('[data-testid="workspace-new-name"]').setValue("新项目");
    await wrapper.get('[data-testid="workspace-new-path"]').setValue("D:/new-project");
    await wrapper.get('[data-testid="workspace-new-submit"]').trigger("click");
    await nextTick();
    await nextTick();

    expect(runtimeStore.activeWorkspaceId).toBe("ws-created");
    expect(runtimeStore.workspaceList.some((workspace) => workspace.id === "ws-created")).toBe(true);
    expect(
      wrapper.get('[data-testid="workspace-row-ws-created"]').text()
    ).toContain("激活");
  });

  it("keeps only the default group and disables workspace management in browser mode", async () => {
    tauriMocks.mockIsTauriAvailable.mockReturnValue(false);
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionId: "session-current",
      sessionOperation: null,
      isSubmitting: false,
      messages: [createMessage({ content: "existing content" })],
      sessionList: [createSession({ conversationId: "session-current", title: "默认组会话" })]
    });

    const wrapper = mountSidebar();
    await nextTick();
    await nextTick();

    // 浏览器模式：仅默认组 + 管理禁用提示。
    expect(wrapper.find('[data-testid="session-sidebar-group-default"]').exists()).toBe(true);
    expect(wrapper.text()).toContain("浏览器");
    expect(wrapper.find('[data-testid="workspace-new-open-form"]').exists()).toBe(false);
  });

  it("auto-expands the persisted-collapsed group containing the current session (explicit collapse this boot wins)", async () => {
    window.localStorage.setItem(
      "pony-agent.session-sidebar-workspace-groups.v1",
      JSON.stringify(["default"])
    );
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionId: "session-a",
      sessionOperation: null,
      isSubmitting: false,
      messages: [],
      workspaceList: [{ id: "default", name: "默认工作区", rootPath: "D:/default" }],
      workspaceListLoaded: true,
      activeWorkspaceId: "default",
      sessionList: [
        createSession({ conversationId: "session-a", title: "会话 A" }),
        createSession({ conversationId: "session-b", title: "会话 B" })
      ]
    });

    const wrapper = mountSidebar();
    await nextTick();

    // 持久化折叠生效：当前会话行不可见。
    expect(wrapper.find('[data-testid="session-switch-session-a"]').exists()).toBe(false);

    // 切换到同组的另一会话 → 该组自动展开（本 boot 未显式折叠过该组）。
    runtimeStore.$patch({ sessionId: "session-b" });
    await nextTick();

    expect(wrapper.find('[data-testid="session-switch-session-b"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="session-switch-session-a"]').exists()).toBe(true);
  });

  it("shows the workspace empty hint for a group without sessions", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionId: "session-current",
      sessionOperation: null,
      isSubmitting: false,
      messages: [createMessage({ content: "existing content" })],
      workspaceList: [
        { id: "default", name: "默认工作区", rootPath: "D:/default" },
        { id: "ws-empty", name: "空项目", rootPath: "D:/empty" }
      ],
      workspaceListLoaded: true,
      activeWorkspaceId: "default",
      sessionList: [createSession({ conversationId: "session-current", title: "默认组会话" })]
    });

    const wrapper = mountSidebar();
    await nextTick();

    expect(wrapper.get('[data-testid="session-sidebar-group-ws-empty"]').text()).toContain("空项目");
    expect(
      wrapper.get('[data-testid="session-sidebar-group-empty-ws-empty"]').text()
    ).toContain("该工作区暂无对话");
  });
});

// ── PA-099：更新角标（仅"设置"入口，amber 静态点）─────────────────────
describe("HomeSessionSidebar update badge", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    window.localStorage.clear();
    setActivePinia(createPinia());
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-06-07T15:00:00+08:00"));
    tauriMocks.mockSafeListen.mockResolvedValue(() => {});
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    vi.spyOn(console, "info").mockImplementation(() => {});
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  function seedUpdateAvailable() {
    const updateStore = useUpdateStore();
    updateStore.$patch({
      status: "available",
      latest: { tagName: "v99.0.0", name: null, publishedAtMs: null }
    });
  }

  it("有新版本时在设置两个入口显示角标（展开态）", async () => {
    seedUpdateAvailable();

    const wrapper = mountSidebar();
    await nextTick();

    expect(wrapper.get('[data-testid="session-sidebar-nav-settings-update-badge"]').exists()).toBe(true);
    // ADR 0013：模型配置入口已从左栏移除，角标只随"设置"存在
    expect(wrapper.find('[data-testid="session-sidebar-nav-providers"]').exists()).toBe(false);
    expect(wrapper.text()).not.toContain("模型配置");
  });

  it("有新版本时在折叠态设置图标显示角标", async () => {
    seedUpdateAvailable();

    const wrapper = mountSidebar("home", true);
    await nextTick();

    expect(
      wrapper.get('[data-testid="session-sidebar-nav-settings-collapsed-update-badge"]').exists()
    ).toBe(true);
  });

  it("无更新时任何入口都没有角标", async () => {
    const wrapper = mountSidebar();
    await nextTick();

    expect(wrapper.find('[data-testid="session-sidebar-nav-settings-update-badge"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="session-sidebar-nav-settings-collapsed-update-badge"]').exists()).toBe(false);
  });

  it("缓存版本不高于当前版本时角标保持隐藏（现算 hasUpdate）", async () => {
    const updateStore = useUpdateStore();
    updateStore.$patch({
      status: "up-to-date",
      latest: { tagName: "v0.0.1", name: null, publishedAtMs: null }
    });

    const wrapper = mountSidebar();
    await nextTick();

    expect(wrapper.find('[data-testid="session-sidebar-nav-settings-update-badge"]').exists()).toBe(false);
  });
});
