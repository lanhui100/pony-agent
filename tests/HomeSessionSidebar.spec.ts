import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { defineComponent, nextTick } from "vue";
import { mount } from "@vue/test-utils";
import HomeSessionSidebar from "@/components/HomeSessionSidebar.vue";
import { useRuntimeStore } from "@/stores/runtime";
import type {
  ChatMessage,
  ExecutionCheckpoint,
  GraphRunControlBoundaryEvidence,
  GraphRunSubmissionPlan,
  HistoryBranch,
  HistoryCheckoutResult,
  HistoryNode,
  HistoryStateAuditSummary,
  RunControlAuditSummary,
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

function createCheckpoint(partial: Partial<ExecutionCheckpoint> = {}): ExecutionCheckpoint {
  return {
    turnId: partial.turnId ?? "turn-1",
    sessionId: partial.sessionId ?? "session-current",
    runId: partial.runId ?? "run-1",
    checkpointKind: partial.checkpointKind ?? "recovery",
    recoveryMode: partial.recoveryMode ?? "persisted_effect",
    projectedRuntimePhase: partial.projectedRuntimePhase ?? "ready",
    submissionCommand: partial.submissionCommand ?? "resume_graph_run_stream",
    resumable: partial.resumable ?? true,
    replayable: partial.replayable ?? true,
    status: partial.status ?? "ready",
    phase: partial.phase ?? "paused",
    providerRequestedName: partial.providerRequestedName ?? "OpenAI",
    providerName: partial.providerName ?? "OpenAI",
    providerProtocol: partial.providerProtocol ?? "openai",
    providerModel: partial.providerModel ?? "gpt-5",
    providerSource: partial.providerSource ?? "graph_checkpoint",
    providerMode: partial.providerMode ?? "recovery",
    fallbackReason: partial.fallbackReason ?? null,
    completedHops: partial.completedHops ?? 0,
    maxHops: partial.maxHops ?? 0,
    activeToolName: partial.activeToolName ?? null,
    traceSteps: partial.traceSteps ?? [],
    toolActivities: partial.toolActivities ?? [],
    error: partial.error ?? null,
    startedAtMs: partial.startedAtMs ?? 1000,
    updatedAtMs: partial.updatedAtMs ?? 1200,
    stopRequestedAtMs: partial.stopRequestedAtMs ?? null
  };
}

function createSubmissionPlan(
  partial: Partial<GraphRunSubmissionPlan> = {}
): GraphRunSubmissionPlan {
  return {
    command: partial.command ?? "start_graph_run_stream",
    runId: partial.runId ?? null,
    source: partial.source ?? "default"
  };
}

function createBoundaryEvidence(
  partial: Partial<GraphRunControlBoundaryEvidence> = {}
): GraphRunControlBoundaryEvidence {
  return {
    hookPoint: partial.hookPoint ?? "turn.completed",
    canonicalEventType: partial.canonicalEventType ?? "turn.completed",
    canonicalPhase: partial.canonicalPhase ?? "completed",
    summary: partial.summary ?? "在 turn.completed 安全边界暂停",
    hookEnvelope: partial.hookEnvelope ?? {
      sessionId: "session-current",
      runId: "run-1",
      turnId: "turn-1",
      sequence: 1,
      hookPoint: "turn.completed",
      canonicalEventType: "turn.completed",
      canonicalPhase: "completed",
      payloadJson: "{}",
      createdAtMs: 1000
    },
    createdAtMs: partial.createdAtMs ?? 1000
  };
}

function createHistoryStateAuditSummary(
  partial: {
    action?: Partial<HistoryStateAuditSummary["action"]>;
    currentContext?: Partial<HistoryStateAuditSummary["currentContext"]>;
  } = {}
): HistoryStateAuditSummary {
  return {
    action: {
      status: partial.action?.status ?? "available",
      sourceFamily: partial.action?.sourceFamily ?? "history_state",
      commandKind: partial.action?.commandKind ?? "checkout_history_node",
      boundary: partial.action?.boundary ?? "turn_prepare_end",
      resultKind: partial.action?.resultKind ?? "observe",
      summary: partial.action?.summary ?? "已记录最近一次历史控制动作证据",
      elapsedMs: partial.action?.elapsedMs ?? 12,
      blocked: partial.action?.blocked ?? false,
      degraded: partial.action?.degraded ?? false,
      evidenceId: partial.action?.evidenceId ?? "evidence-1",
      observedAtMs: partial.action?.observedAtMs ?? 1000,
      requestedNodeId: partial.action?.requestedNodeId ?? "node-requested",
      requestedBranchId: partial.action?.requestedBranchId ?? "branch-requested",
      resolvedNodeId: partial.action?.resolvedNodeId ?? "node-resolved",
      resolvedBranchId: partial.action?.resolvedBranchId ?? "branch-resolved"
    },
    currentContext: {
      mode: partial.currentContext?.mode ?? "historical",
      visibleNodeId: partial.currentContext?.visibleNodeId ?? "node-context",
      activeBranchId: partial.currentContext?.activeBranchId ?? "branch-context",
      branchHeadNodeId: partial.currentContext?.branchHeadNodeId ?? "node-head",
      workspaceNodeId: partial.currentContext?.workspaceNodeId ?? "workspace-node"
    }
  };
}

function createRunControlAuditSummary(
  partial: {
    action?: Partial<RunControlAuditSummary["actionEvidenceSummary"]>;
    currentContext?: Partial<RunControlAuditSummary["currentContextProjection"]>;
  } = {}
): RunControlAuditSummary {
  return {
    actionEvidenceSummary: {
      status: partial.action?.status ?? "available",
      sourceFamily: partial.action?.sourceFamily ?? "run_control",
      commandKind: partial.action?.commandKind ?? "resume_graph_run_stream",
      boundary: partial.action?.boundary ?? "run_resume",
      resultKind: partial.action?.resultKind ?? "observe",
      summary: partial.action?.summary ?? "存在暂停中的运行；下一次发送会恢复该 run 并继续推进。",
      targetSummary: partial.action?.targetSummary ?? "恢复 run-1",
      elapsedMs: partial.action?.elapsedMs ?? 8,
      blocked: partial.action?.blocked ?? false,
      degraded: partial.action?.degraded ?? false,
      evidenceId: partial.action?.evidenceId ?? "run-control-evidence-1",
      observedAtMs: partial.action?.observedAtMs ?? 1000,
      runId: partial.action?.runId ?? "run-1",
      turnId: partial.action?.turnId ?? "turn-1",
      checkpointTurnId: partial.action?.checkpointTurnId ?? "turn-1",
      checkpointKind: partial.action?.checkpointKind ?? "recovery",
      recoveryMode: partial.action?.recoveryMode ?? "persisted_effect",
      projectedCommand: partial.action?.projectedCommand ?? "resume_graph_run_stream",
      degradationReason: partial.action?.degradationReason ?? null,
      requestSummary: partial.action?.requestSummary ?? "resume run-1",
      startReason: partial.action?.startReason ?? null
    },
    currentContextProjection: {
      phase: partial.currentContext?.phase ?? "paused",
      checkpointStatus: partial.currentContext?.checkpointStatus ?? "ready",
      activeRunId: partial.currentContext?.activeRunId ?? "run-1",
      checkpointKind: partial.currentContext?.checkpointKind ?? "recovery",
      checkpointRecoveryMode: partial.currentContext?.checkpointRecoveryMode ?? "persisted_effect",
      submissionPlanCommand:
        partial.currentContext?.submissionPlanCommand ?? "resume_graph_run_stream"
    }
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

  it("maps control state and history mode to user-facing vocabulary", async () => {
    seedSidebarSessions();

    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      phase: "cancelled",
      visibleNodeId: "node-old",
      branchHeadNodeId: "node-head",
      activeBranchId: "branch-main",
      historyCursorMode: "historical_dirty",
      historyNodes: [createHistoryNode({ nodeId: "node-old" })],
      historyBranches: [createHistoryBranch({ branchId: "branch-main", headNodeId: "node-head" })],
      latestExecutionCheckpoint: createCheckpoint({
        runId: "run-paused",
        submissionCommand: "resume_graph_run_stream",
        phase: "paused",
        status: "ready"
      }),
      latestGraphRunSubmissionPlan: createSubmissionPlan({
        command: "resume_graph_run_stream",
        runId: "run-paused",
        source: "checkpoint"
      }),
      latestRunControlAuditSummary: createRunControlAuditSummary({
        action: {
          summary: "存在暂停中的运行；下一次发送会恢复该 run 并继续推进。",
          runId: "run-paused",
          projectedCommand: "resume_graph_run_stream"
        },
        currentContext: {
          activeRunId: "run-paused",
          phase: "paused"
        }
      })
    });

    const wrapper = mountSidebar();
    await nextTick();

    expect(wrapper.get('[data-testid="session-sidebar-control-status"]').text()).toContain("可恢复");
    expect(wrapper.get('[data-testid="session-sidebar-history-graph"]').text()).toContain("历史分叉待处理");
    expect(wrapper.get('[data-testid="session-sidebar-history-graph"]').text()).not.toContain("historical_dirty");
  });

  it("shows latest control boundary evidence in the session control card", async () => {
    seedSidebarSessions();

    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      latestGraphRunControlBoundaryEvidence: [
        createBoundaryEvidence({
          summary: "已在 tool_result_integrating 边界完成控制确认"
        })
      ],
      latestRunControlAuditSummary: createRunControlAuditSummary({
        action: {
          commandKind: "stop_graph_run",
          boundary: "stop_requested",
          resultKind: "observe",
          summary: "已请求停止当前运行，等待下次提交时决定恢复或重放。"
        },
        currentContext: {
          phase: "running",
          submissionPlanCommand: "resume_graph_run_stream"
        }
      })
    });

    const wrapper = mountSidebar();
    await nextTick();

    expect(wrapper.get('[data-testid="session-sidebar-control-boundary-evidence"]').text()).toContain(
      "控制摘要：已请求停止当前运行，等待下次提交时决定恢复或重放。"
    );
  });

  it("renders summary-first action evidence separately from current context on success", async () => {
    seedSidebarSessions();

    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      latestHistoryStateAuditSummary: createHistoryStateAuditSummary({
        action: {
          summary: "已确认 checkout_history_node 动作证据已持久化",
          commandKind: "checkout_history_node",
          boundary: "turn_prepare_end",
          resultKind: "observe"
        },
        currentContext: {
          mode: "historical",
          activeBranchId: "branch-success",
          visibleNodeId: "node-success",
          branchHeadNodeId: "node-success-head",
          workspaceNodeId: "workspace-success"
        }
      })
    });

    const wrapper = mountSidebar();
    await nextTick();

    expect(wrapper.get('[data-testid="session-sidebar-history-action-evidence"]').text()).toContain(
      "已确认 checkout_history_node 动作证据已持久化"
    );
    expect(wrapper.get('[data-testid="session-sidebar-history-current-context"]').text()).toContain(
      "当前上下文（非动作证据）"
    );
    expect(wrapper.get('[data-testid="session-sidebar-history-current-context"]').text()).toContain(
      "分支 branch-success"
    );
    expect(wrapper.get('[data-testid="session-sidebar-history-current-context"]').text()).toContain(
      "工作区 workspace-success"
    );
  });

  it("shows missing evidence without substituting current context as action proof", async () => {
    seedSidebarSessions();

    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      latestHistoryStateAuditSummary: createHistoryStateAuditSummary({
        action: {
          status: "missing",
          summary: "缺少 history_state hook evidence，暂时无法确认最近动作是否留下审计证据",
          evidenceId: null
        },
        currentContext: {
          mode: "historical",
          activeBranchId: "branch-missing",
          visibleNodeId: "node-context-only",
          branchHeadNodeId: "node-missing-head",
          workspaceNodeId: "workspace-missing"
        }
      })
    });

    const wrapper = mountSidebar();
    await nextTick();

    expect(wrapper.get('[data-testid="session-sidebar-history-action-evidence"]').text()).toContain(
      "证据缺失"
    );
    expect(wrapper.get('[data-testid="session-sidebar-history-action-evidence"]').text()).toContain(
      "缺少 history_state hook evidence"
    );
    expect(wrapper.get('[data-testid="session-sidebar-history-action-evidence"]').text()).not.toContain(
      "node-context-only"
    );
    expect(wrapper.get('[data-testid="session-sidebar-history-current-context"]').text()).toContain(
      "node-context-only"
    );
  });

  it("shows degraded history-control evidence from the audit summary", async () => {
    seedSidebarSessions();

    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      latestHistoryStateAuditSummary: createHistoryStateAuditSummary({
        action: {
          degraded: true,
          summary: "动作已降级为 transcript_only，工作区未执行回滚"
        },
        currentContext: {
          mode: "historical",
          activeBranchId: "branch-degraded",
          visibleNodeId: "node-degraded"
        }
      })
    });

    const wrapper = mountSidebar();
    await nextTick();

    expect(wrapper.get('[data-testid="session-sidebar-history-action-evidence"]').text()).toContain(
      "已降级"
    );
    expect(wrapper.get('[data-testid="session-sidebar-history-action-evidence"]').text()).toContain(
      "动作已降级为 transcript_only"
    );
  });

  it("shows blocked history-control evidence from the audit summary", async () => {
    seedSidebarSessions();

    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      latestHistoryStateAuditSummary: createHistoryStateAuditSummary({
        action: {
          blocked: true,
          summary: "最近一次历史控制动作已被 guard 阻断，未进入恢复阶段"
        },
        currentContext: {
          mode: "historical_dirty",
          activeBranchId: "branch-blocked",
          visibleNodeId: "node-blocked"
        }
      })
    });

    const wrapper = mountSidebar();
    await nextTick();

    expect(wrapper.get('[data-testid="session-sidebar-history-action-evidence"]').text()).toContain(
      "已阻断"
    );
    expect(wrapper.get('[data-testid="session-sidebar-history-action-evidence"]').text()).toContain(
      "最近一次历史控制动作已被 guard 阻断"
    );
    expect(wrapper.get('[data-testid="session-sidebar-history-current-context"]').text()).toContain(
      "历史分叉待处理"
    );
  });

  it("explains why history actions are disabled while a run is active", async () => {
    seedSidebarSessions();

    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      isSubmitting: true,
      visibleNodeId: "node-old",
      branchHeadNodeId: "node-head",
      activeBranchId: "branch-main",
      historyCursorMode: "historical",
      historyNodes: [createHistoryNode({ nodeId: "node-old" })],
      historyBranches: [createHistoryBranch({ branchId: "branch-main", headNodeId: "node-head" })]
    });

    const wrapper = mountSidebar();
    await nextTick();

    expect(wrapper.get('[data-testid="session-sidebar-history-disabled-reason"]').text()).toContain(
      "运行中不可恢复到分支头"
    );
  });

  it("shows transcript-only degradation feedback after checkout falls back", async () => {
    seedSidebarSessions();

    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      visibleNodeId: "node-old",
      branchHeadNodeId: "node-head",
      activeBranchId: "branch-main",
      historyCursorMode: "historical",
      historyNodes: [
        createHistoryNode({ nodeId: "node-old", summary: "旧节点", createdAtMs: 1000 }),
        createHistoryNode({ nodeId: "node-head", summary: "最新节点", createdAtMs: 2000 })
      ],
      historyBranches: [createHistoryBranch({ branchId: "branch-main", headNodeId: "node-head" })]
    });

    const checkoutSpy = vi.spyOn(runtimeStore, "checkoutHistoryNode").mockResolvedValue({
      sessionId: "session-current",
      visibleNodeId: "node-old",
      activeBranchId: "branch-main",
      branchHeadNodeId: "node-head",
      workspaceNodeId: "node-head",
      mode: "historical",
      requestedMode: "transcript_and_workspace",
      appliedMode: "transcript_only",
      transcriptRestoreApplied: true,
      workspaceRestoreCapable: false,
      workspaceRestoreApplied: false,
      degradedToTranscriptOnly: true,
      degradationReason: "workspace_rollback_unsupported",
      historyNodes: runtimeStore.historyNodes,
      historyBranches: runtimeStore.historyBranches
    } satisfies HistoryCheckoutResult);

    const wrapper = mountSidebar();
    await nextTick();

    await wrapper.get('[data-testid="session-sidebar-history-node-node-old"]').trigger("click");
    await nextTick();

    expect(checkoutSpy).toHaveBeenCalledWith("node-old", "transcript_and_workspace");
    expect(wrapper.get('[data-testid="session-sidebar-history-feedback"]').text()).toContain(
      "仅恢复对话，未恢复工作区"
    );
    expect(wrapper.get('[data-testid="session-sidebar-history-feedback"]').text()).toContain(
      "当前工作区暂不支持回滚"
    );
  });

  it("shows success feedback for restore, fork, and branch switch results", async () => {
    seedSidebarSessions();

    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      visibleNodeId: "node-old",
      branchHeadNodeId: "node-head",
      activeBranchId: "branch-main",
      historyCursorMode: "historical",
      historyNodes: [
        createHistoryNode({ nodeId: "node-old", summary: "旧节点", createdAtMs: 1000 }),
        createHistoryNode({ nodeId: "node-head", summary: "最新节点", createdAtMs: 2000 })
      ],
      historyBranches: [
        createHistoryBranch({ branchId: "branch-main", headNodeId: "node-head", label: "main" }),
        createHistoryBranch({ branchId: "branch-fork", headNodeId: "node-old", label: "fork-1" })
      ]
    });

    vi.spyOn(runtimeStore, "restoreBranchHead").mockResolvedValue({
      sessionId: "session-current",
      visibleNodeId: "node-head",
      activeBranchId: "branch-main",
      branchHeadNodeId: "node-head",
      workspaceNodeId: "node-head",
      mode: "live",
      restoredFromNodeId: "node-old",
      transcriptRestoreApplied: true,
      workspaceRestoreCapable: false,
      workspaceRestoreApplied: false,
      degradedToTranscriptOnly: false,
      degradationReason: null,
      historyNodes: runtimeStore.historyNodes,
      historyBranches: runtimeStore.historyBranches
    });
    vi.spyOn(runtimeStore, "forkHistoryNode").mockResolvedValue({
      sessionId: "session-current",
      visibleNodeId: "node-old",
      activeBranchId: "branch-fork-new",
      branchHeadNodeId: "node-old",
      workspaceNodeId: "node-old",
      mode: "live",
      forkedFromNodeId: "node-old",
      forkedFromBranchId: "branch-main",
      createdBranchId: "branch-fork-new",
      historyNodes: runtimeStore.historyNodes,
      historyBranches: runtimeStore.historyBranches
    });
    vi.spyOn(runtimeStore, "switchHistoryBranch").mockResolvedValue({
      sessionId: "session-current",
      visibleNodeId: "node-old",
      activeBranchId: "branch-fork",
      branchHeadNodeId: "node-old",
      workspaceNodeId: "node-old",
      mode: "historical",
      previousBranchId: "branch-main",
      historyNodes: runtimeStore.historyNodes,
      historyBranches: runtimeStore.historyBranches
    });

    const wrapper = mountSidebar();
    await nextTick();

    await wrapper.get('[data-testid="session-sidebar-history-restore"]').trigger("click");
    await nextTick();
    expect(wrapper.get('[data-testid="session-sidebar-history-feedback"]').text()).toContain(
      "已恢复到分支头"
    );
    expect(wrapper.get('[data-testid="session-sidebar-history-feedback"]').text()).toContain(
      "当前分支 branch-main"
    );

    await wrapper.get('[data-testid="session-sidebar-history-fork"]').trigger("click");
    await nextTick();
    expect(wrapper.get('[data-testid="session-sidebar-history-feedback"]').text()).toContain(
      "已创建历史分支"
    );
    expect(wrapper.get('[data-testid="session-sidebar-history-feedback"]').text()).toContain(
      "branch-fork-new"
    );

    await wrapper.get('[data-testid="session-sidebar-history-branch-branch-fork"]').trigger("click");
    await nextTick();
    expect(wrapper.get('[data-testid="session-sidebar-history-feedback"]').text()).toContain(
      "已切换历史分支"
    );
    expect(wrapper.get('[data-testid="session-sidebar-history-feedback"]').text()).toContain(
      "branch-main"
    );
    expect(wrapper.get('[data-testid="session-sidebar-history-feedback"]').text()).toContain(
      "branch-fork"
    );
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
