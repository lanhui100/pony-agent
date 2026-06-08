import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { defineComponent, nextTick } from "vue";
import { mount } from "@vue/test-utils";
import HomeSessionSidebar from "@/components/HomeSessionSidebar.vue";
import { useRuntimeStore } from "@/stores/runtime";
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
    updatedAtMs: partial.updatedAtMs ?? 1000
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

function mountSidebar(currentPage: "home" | "providers" | "model-monitor" = "home") {
  return mount(HomeSessionSidebar, {
    props: {
      currentPage
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

  it("keeps create actions above session list and model sections", async () => {
    seedSidebarSessions();

    const wrapper = mountSidebar();
    await nextTick();

    const actions = wrapper.get('[data-testid="session-sidebar-actions"]').element;
    const sessionList = wrapper.get('[data-testid="session-sidebar-session-list"]').element;
    const model = wrapper.get('[data-testid="session-sidebar-model-nav"]').element;

    expect(actions.compareDocumentPosition(sessionList) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(sessionList.compareDocumentPosition(model) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it("navigates home before switching when a concrete session is opened from another page", async () => {
    seedSidebarSessions();

    const runtimeStore = useRuntimeStore();
    const switchSessionSpy = vi.spyOn(runtimeStore, "switchSession").mockResolvedValue();
    const wrapper = mountSidebar("model-monitor");
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

  it("renders provider config and model monitor inside model management", async () => {
    seedSidebarSessions();

    const wrapper = mountSidebar();
    await nextTick();

    expect(wrapper.get('[data-testid="session-sidebar-nav-providers"]').text()).toContain("配置");
    expect(wrapper.get('[data-testid="session-sidebar-nav-model-monitor"]').text()).toContain("监控");
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

    const wrapper = mountSidebar("providers");
    await nextTick();

    await wrapper.get('[data-testid="session-sidebar-brand"]').trigger("click");
    expect(wrapper.emitted("navigate")).toEqual([["home"]]);
  });

  it("shares the same warm orange hover and selected guardrails across menu items", async () => {
    seedSidebarSessions();

    const wrapper = mountSidebar("providers");
    await nextTick();

    const newChat = wrapper.get('[data-testid="session-sidebar-new-chat"]');
    const sessionItem = wrapper.get('[data-testid="session-switch-session-current"]').element.parentElement?.parentElement;
    const modelToggle = wrapper.get('[data-testid="session-sidebar-model-toggle"]');
    const providerItem = wrapper.get('[data-testid="session-sidebar-nav-providers"]');

    expect(newChat.classes()).toContain("hover:bg-[#f6dfb8]");
    expect(modelToggle.classes()).toContain("bg-[#f3c98d]");
    expect(providerItem.classes()).toContain("bg-[#f3c98d]");
    expect(providerItem.classes()).toContain("rounded-[0.2rem]");
    expect(sessionItem?.className).toContain("bg-[#f3c98d]");
  });

  it("keeps only the four key entries in collapsed mode", async () => {
    seedSidebarSessions();

    const wrapper = mountSidebar("model-monitor");
    await nextTick();

    await wrapper.get('[data-testid="session-sidebar-collapse"]').trigger("click");
    await nextTick();

    expect(wrapper.find('[data-testid="session-sidebar-brand"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="session-sidebar-new-chat-collapsed"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="session-sidebar-home-collapsed"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="session-sidebar-nav-providers-collapsed"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="session-sidebar-nav-model-monitor-collapsed"]').exists()).toBe(true);
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

  it("collapsed home icon routes back to home workspace", async () => {
    seedSidebarSessions();

    const wrapper = mountSidebar("model-monitor");
    await nextTick();

    await wrapper.get('[data-testid="session-sidebar-collapse"]').trigger("click");
    await nextTick();

    await wrapper.get('[data-testid="session-sidebar-home-collapsed"]').trigger("click");

    expect(wrapper.emitted("navigate")).toEqual([["home"]]);
  });

  it("persists collapse state and exposes collapsed provider or monitor navigation", async () => {
    seedSidebarSessions();

    const wrapper = mountSidebar("home");
    await nextTick();

    await wrapper.get('[data-testid="session-sidebar-collapse"]').trigger("click");
    await nextTick();

    expect(window.localStorage.getItem("pony-agent.session-sidebar-collapsed.v1")).toBe("1");

    await wrapper.get('[data-testid="session-sidebar-nav-providers-collapsed"]').trigger("click");
    await wrapper.get('[data-testid="session-sidebar-nav-model-monitor-collapsed"]').trigger("click");

    expect(wrapper.emitted("navigate")).toEqual([["providers"], ["model-monitor"]]);
  });

  it("toggles model management open state and persists it", async () => {
    seedSidebarSessions();

    const wrapper = mountSidebar("providers");
    await nextTick();

    expect(wrapper.find('[data-testid="session-sidebar-nav-providers"]').exists()).toBe(true);

    await wrapper.get('[data-testid="session-sidebar-model-toggle"]').trigger("click");
    await nextTick();

    expect(wrapper.find('[data-testid="session-sidebar-nav-providers"]').exists()).toBe(false);
    expect(window.localStorage.getItem("pony-agent.session-sidebar-model-open.v1")).toBe("0");

    await wrapper.get('[data-testid="session-sidebar-model-toggle"]').trigger("click");
    await nextTick();

    expect(wrapper.find('[data-testid="session-sidebar-nav-providers"]').exists()).toBe(true);
    expect(window.localStorage.getItem("pony-agent.session-sidebar-model-open.v1")).toBe("1");
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
});
