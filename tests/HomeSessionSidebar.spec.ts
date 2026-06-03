import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { defineComponent, nextTick } from "vue";
import { mount } from "@vue/test-utils";
import HomeSessionSidebar from "@/components/HomeSessionSidebar.vue";
import { useRuntimeStore } from "@/stores/runtime";
import type { ChatMessage, HistoryBranch, HistoryNode, SessionOverview } from "@/types/runtime";

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
    tauriMocks.mockSafeListen.mockResolvedValue(() => {});
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    vi.spyOn(console, "info").mockImplementation(() => {});
  });

  afterEach(() => {
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

  it("keeps create actions above history and model sections", async () => {
    seedSidebarSessions();

    const wrapper = mountSidebar();
    await nextTick();

    const actions = wrapper.get('[data-testid="session-sidebar-actions"]').element;
    const history = wrapper.get('[data-testid="session-sidebar-history-panel"]').element;
    const model = wrapper.get('[data-testid="session-sidebar-model-nav"]').element;

    expect(actions.compareDocumentPosition(history) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(history.compareDocumentPosition(model) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it("navigates back to home when history entry point is clicked from another page", async () => {
    seedSidebarSessions();

    const wrapper = mountSidebar("providers");
    await nextTick();

    await wrapper.get('[data-testid="session-sidebar-history-toggle"]').trigger("click");
    expect(wrapper.emitted("navigate")).toEqual([["home"]]);
  });

  it("toggles history open state in home and persists it", async () => {
    seedSidebarSessions();

    const wrapper = mountSidebar("home");
    await nextTick();

    expect(wrapper.find('[data-testid="session-sidebar-history"]').exists()).toBe(true);

    await wrapper.get('[data-testid="session-sidebar-history-toggle"]').trigger("click");
    await nextTick();

    expect(wrapper.find('[data-testid="session-sidebar-history"]').exists()).toBe(false);
    expect(window.localStorage.getItem("pony-agent.session-sidebar-history-open.v1")).toBe("0");
    expect(wrapper.emitted("navigate")).toBeUndefined();

    await wrapper.get('[data-testid="session-sidebar-history-toggle"]').trigger("click");
    await nextTick();

    expect(wrapper.find('[data-testid="session-sidebar-history"]').exists()).toBe(true);
    expect(window.localStorage.getItem("pony-agent.session-sidebar-history-open.v1")).toBe("1");
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

  it("keeps history and model management at the same primary level", async () => {
    seedSidebarSessions();

    const wrapper = mountSidebar("providers");
    await nextTick();

    const historyToggle = wrapper.get('[data-testid="session-sidebar-history-toggle"]');
    const modelToggle = wrapper.get('[data-testid="session-sidebar-model-toggle"]');
    const providerItem = wrapper.get('[data-testid="session-sidebar-nav-providers"]');
    const monitorItem = wrapper.get('[data-testid="session-sidebar-nav-model-monitor"]');

    expect(historyToggle.classes()).toContain("text-left");
    expect(modelToggle.classes()).toContain("text-left");
    expect(providerItem.classes()).toContain("justify-start");
    expect(monitorItem.classes()).toContain("justify-start");
  });

  it("shares the same warm orange hover and selected guardrails across menu items", async () => {
    seedSidebarSessions();

    const wrapper = mountSidebar("providers");
    await nextTick();

    const newChat = wrapper.get('[data-testid="session-sidebar-new-chat"]');
    const historyToggle = wrapper.get('[data-testid="session-sidebar-history-toggle"]');
    const sessionItem = wrapper.get('[data-testid="session-switch-session-current"]').element.parentElement?.parentElement;
    const modelToggle = wrapper.get('[data-testid="session-sidebar-model-toggle"]');
    const providerItem = wrapper.get('[data-testid="session-sidebar-nav-providers"]');

    expect(newChat.classes()).toContain("hover:bg-[#f6dfb8]");
    expect(historyToggle.classes()).toContain("hover:bg-[#f6dfb8]");
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
    expect(wrapper.find('[data-testid="session-sidebar-history-collapsed"]').exists()).toBe(true);
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

  it("collapsed history icon always routes back to home and reopens history", async () => {
    seedSidebarSessions();
    window.localStorage.setItem("pony-agent.session-sidebar-history-open.v1", "0");

    const wrapper = mountSidebar("model-monitor");
    await nextTick();

    await wrapper.get('[data-testid="session-sidebar-collapse"]').trigger("click");
    await nextTick();

    await wrapper.get('[data-testid="session-sidebar-history-collapsed"]').trigger("click");

    expect(wrapper.emitted("navigate")).toEqual([["home"]]);
    expect(window.localStorage.getItem("pony-agent.session-sidebar-history-open.v1")).toBe("1");
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

    expect(createSessionSpy).toHaveBeenCalledTimes(1);
    expect(deleteSessionSpy).toHaveBeenCalledWith("session-other");
  });

  it("renders history graph actions and status for historical mode", async () => {
    seedSidebarSessions();

    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      visibleNodeId: "node-old",
      branchHeadNodeId: "node-head",
      activeBranchId: "branch-main",
      historyCursorMode: "historical",
      historyNodes: [
        createHistoryNode({ nodeId: "node-old", summary: "旧节点", createdAtMs: 1000 }),
        createHistoryNode({
          nodeId: "node-head",
          summary: "最新节点",
          createdAtMs: 2000
        })
      ],
      historyBranches: [
        createHistoryBranch({
          branchId: "branch-main",
          headNodeId: "node-head",
          label: "main"
        }),
        createHistoryBranch({
          branchId: "branch-fork",
          baseNodeId: "node-old",
          headNodeId: "node-old",
          label: "fork-1",
          updatedAtMs: 3000
        })
      ]
    });

    const wrapper = mountSidebar();
    await nextTick();

    expect(wrapper.get('[data-testid="session-sidebar-history-graph"]').text()).toContain("历史浏览中");
    expect(wrapper.get('[data-testid="session-sidebar-history-restore"]').text()).toContain("恢复到分支头");
    expect(wrapper.get('[data-testid="session-sidebar-history-fork"]').text()).toContain("从当前节点分叉");
    expect(wrapper.get('[data-testid="session-sidebar-history-node-node-old"]').text()).toContain("旧节点");
    expect(wrapper.get('[data-testid="session-sidebar-history-branch-branch-fork"]').text()).toContain("fork-1");
  });

  it("forwards history node, restore, fork, and branch-switch actions to the runtime store", async () => {
    seedSidebarSessions();

    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      visibleNodeId: "node-old",
      branchHeadNodeId: "node-head",
      activeBranchId: "branch-main",
      historyCursorMode: "historical",
      historyNodes: [
        createHistoryNode({ nodeId: "node-old", summary: "旧节点", createdAtMs: 1000 }),
        createHistoryNode({
          nodeId: "node-head",
          summary: "最新节点",
          createdAtMs: 2000
        })
      ],
      historyBranches: [
        createHistoryBranch({
          branchId: "branch-main",
          headNodeId: "node-head",
          label: "main"
        }),
        createHistoryBranch({
          branchId: "branch-fork",
          baseNodeId: "node-old",
          headNodeId: "node-old",
          label: "fork-1",
          updatedAtMs: 3000
        })
      ]
    });

    const checkoutSpy = vi.spyOn(runtimeStore, "checkoutHistoryNode").mockResolvedValue(null);
    const restoreSpy = vi.spyOn(runtimeStore, "restoreBranchHead").mockResolvedValue(null);
    const forkSpy = vi.spyOn(runtimeStore, "forkHistoryNode").mockResolvedValue(null);
    const switchSpy = vi.spyOn(runtimeStore, "switchHistoryBranch").mockResolvedValue(null);

    const wrapper = mountSidebar();
    await nextTick();

    await wrapper.get('[data-testid="session-sidebar-history-node-node-old"]').trigger("click");
    await wrapper.get('[data-testid="session-sidebar-history-restore"]').trigger("click");
    await wrapper.get('[data-testid="session-sidebar-history-fork"]').trigger("click");
    await wrapper.get('[data-testid="session-sidebar-history-branch-branch-fork"]').trigger("click");

    expect(checkoutSpy).toHaveBeenCalledWith("node-old", "transcript_and_workspace");
    expect(restoreSpy).toHaveBeenCalledWith("branch-main");
    expect(forkSpy).toHaveBeenCalledWith("node-old");
    expect(switchSpy).toHaveBeenCalledWith("branch-fork");
  });
});
