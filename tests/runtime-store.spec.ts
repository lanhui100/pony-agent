import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import type {
  AttachmentAsset,
  BuildContextObservation,
  ChatMessage,
  ExecutionCheckpoint,
  HistoryBranch,
  HistoryStateAuditSummary,
  HistoryCursorState,
  HistoryNode,
  HookTraceRecord,
  ProviderCallCacheRecord,
  RetrievedContextState,
  RunControlAuditSummary,
  SessionOverview,
  SessionSnapshot,
  SessionRuntimeView,
  TurnStreamEvent,
  TurnTraceRecord
} from "@/types/runtime";
import { __resetFrontendFlightRecorderForTests, getFrontendRecorderCapabilitySnapshot as getFrontendRecorderCapability, getFrontendRecorderStats } from "@/lib/frontend-flight-recorder";
import { useRuntimeStore } from "@/stores/runtime";
import { useSettingsStore } from "@/stores/settings";

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

const RUNTIME_STORAGE_KEY = "pony-agent.runtime-history.v1";

function createMessage(partial: Partial<ChatMessage> = {}): ChatMessage {
  return {
    id: partial.id ?? `msg-${Math.random().toString(16).slice(2, 8)}`,
    turnId: partial.turnId ?? "turn-1",
    role: partial.role ?? "user",
    content: partial.content ?? "hello",
    attachments: partial.attachments ?? [],
    status: partial.status ?? "done",
    tokenCount: partial.tokenCount ?? null,
    reasoningContent: partial.reasoningContent ?? null,
    modelName: partial.modelName ?? null,
    toolName: partial.toolName ?? null,
    detail: partial.detail ?? null,
    durationSeconds: partial.durationSeconds ?? null,
    errorDetail: partial.errorDetail ?? null
  };
}

function createTrace(partial: Partial<TurnTraceRecord> = {}): TurnTraceRecord {
  return {
    turnId: partial.turnId ?? "turn-1",
    sessionId: partial.sessionId ?? null,
    eventId: partial.eventId ?? null,
    eventType: partial.eventType ?? null,
    eventVersion: partial.eventVersion ?? null,
    sequence: partial.sequence ?? null,
    emittedAtMs: partial.emittedAtMs ?? null,
    title: partial.title ?? "test turn",
    phase: partial.phase ?? "completed",
    traceSteps: partial.traceSteps ?? [],
    traceTimeline: partial.traceTimeline ?? [],
    toolActivities: partial.toolActivities ?? [],
    providerCallRecords: partial.providerCallRecords ?? [],
    hookTraceRecords: partial.hookTraceRecords ?? [],
    providerRequestedName: partial.providerRequestedName ?? null,
    providerName: partial.providerName ?? null,
    providerProtocol: partial.providerProtocol ?? null,
    providerModel: partial.providerModel ?? null,
    providerSource: partial.providerSource ?? null,
    providerMode: partial.providerMode ?? null,
    buildContextObservation: partial.buildContextObservation ?? null,
    sessionSummary: partial.sessionSummary ?? "",
    fallbackReason: partial.fallbackReason ?? null,
    error: partial.error ?? null,
    inputTokens: partial.inputTokens ?? null,
    cacheHitInputTokens: partial.cacheHitInputTokens ?? null,
    reasoningTokens: partial.reasoningTokens ?? null,
    outputTokens: partial.outputTokens ?? null,
    totalTokens: partial.totalTokens ?? null,
    firstTokenLatencyMs: partial.firstTokenLatencyMs ?? null,
    turnDurationMs: partial.turnDurationMs ?? null,
    updatedAt: partial.updatedAt ?? 1000
  };
}

function createHookTraceRecord(partial: Partial<HookTraceRecord> = {}): HookTraceRecord {
  return {
    hookName: partial.hookName ?? "observe.test",
    hookClass: partial.hookClass ?? "observe",
    hookPoint: partial.hookPoint ?? "model_call_start",
    hookOrder: partial.hookOrder ?? 1,
    resultKind: partial.resultKind ?? "observe",
    structuredResult:
      partial.structuredResult ??
      {
        resultKind: "observe",
        payload: {
          summary: "hook observed lifecycle boundary without mutation"
        }
      },
    blocked: partial.blocked ?? false,
    elapsedMs: partial.elapsedMs ?? 2,
    inputSummary: partial.inputSummary ?? "payload summary",
    persistenceEvidenceRef: partial.persistenceEvidenceRef ?? null,
    summary: partial.summary ?? "hook observed lifecycle boundary without mutation"
  };
}

function createProviderCallRecord(
  partial: Partial<ProviderCallCacheRecord> = {}
): ProviderCallCacheRecord {
  return {
    requestKind: partial.requestKind ?? "initial_request",
    providerSource: partial.providerSource ?? "provider_decision",
    providerMode: partial.providerMode ?? "live",
    inputTokens: partial.inputTokens ?? 12,
    cacheHitInputTokens: partial.cacheHitInputTokens ?? 5,
    cacheMissInputTokens: partial.cacheMissInputTokens ?? 7,
    reasoningTokens: partial.reasoningTokens ?? 3,
    outputTokens: partial.outputTokens ?? 10,
    totalTokens: partial.totalTokens ?? 22,
    firstTokenLatencyMs: partial.firstTokenLatencyMs ?? 180,
    turnDurationMs: partial.turnDurationMs ?? 420,
    latencyKind: partial.latencyKind ?? "provider_stream",
    prefixMutationReasons: partial.prefixMutationReasons ?? ["session_summary_changed"]
  };
}

function createBuildContextObservation(
  partial: Partial<BuildContextObservation> = {}
): BuildContextObservation {
  return {
    requestFormat: partial.requestFormat ?? "response_format=text",
    messageCount: partial.messageCount ?? 2,
    imageCount: partial.imageCount ?? 0,
    toolCount: partial.toolCount ?? 1,
    temperature: partial.temperature ?? 0,
    maxOutputTokens: partial.maxOutputTokens ?? 4096,
    stablePrefixText: partial.stablePrefixText ?? "system: stable system rule",
    semiStableContextText: partial.semiStableContextText ?? "developer: retrieval summary",
    volatileInputText: partial.volatileInputText ?? "user: stream request",
    prefixMutationReasons: partial.prefixMutationReasons ?? [],
    contextRefreshReason: partial.contextRefreshReason ?? "initial_build",
    instructionScopeSources: partial.instructionScopeSources ?? ["thread://base-system"],
    conversationCarryMode: partial.conversationCarryMode ?? "full_replay",
    requestMessagesText: partial.requestMessagesText ?? "system: runtime check\nuser: stream request",
    toolDefinitionsText: partial.toolDefinitionsText ?? "workspace.read_file(path: string)"
  };
}

function createSnapshot(partial: Partial<SessionSnapshot> = {}): SessionSnapshot {
  return {
    conversationId: partial.conversationId ?? "session-1",
    title: partial.title ?? "Session 1",
    summary: partial.summary ?? "Session summary",
    history: partial.history ?? [],
    attachmentAssets: partial.attachmentAssets ?? [],
    historyStateAuditSummary: partial.historyStateAuditSummary ?? null,
    runControlAuditSummary: partial.runControlAuditSummary ?? null,
    turnTraceHistory: partial.turnTraceHistory ?? [],
    turnCount: partial.turnCount ?? 0,
    lastReferencedFile: partial.lastReferencedFile ?? null,
    updatedAtMs: partial.updatedAtMs ?? 1000
  };
}

function createRetrievedContext(snapshot: SessionSnapshot): RetrievedContextState {
  const lastUserMessage =
    [...snapshot.history].reverse().find((message) => message.role === "user")?.content ?? "";

  return {
    turnContext: {
      userMessage: lastUserMessage,
      images: [],
      referencesImage: false
    },
    sessionContext: {
      conversationId: snapshot.conversationId,
      title: snapshot.title ?? "Session",
      summary: snapshot.summary,
      recentHistory: snapshot.history.map((message) => ({
        ...message,
        attachments: (message.attachments ?? []).map((attachment) => ({ ...attachment }))
      })),
      recentAttachmentAssets: (snapshot.attachmentAssets ?? []).map((asset) => ({ ...asset })),
      turnCount: snapshot.turnCount,
      lastReferencedFile: snapshot.lastReferencedFile ?? null
    },
    runState: {},
    longTermMemory: {
      status: "empty",
      summary: "No long-term memory entries are stored for this session yet.",
      entries: []
    },
    transcript: {
      providerNativeMessages: []
    }
  };
}

function createCheckpoint(partial: Partial<ExecutionCheckpoint> = {}): ExecutionCheckpoint {
  return {
    contractVersion: partial.contractVersion ?? "execution-checkpoint-v1",
    turnId: partial.turnId ?? "turn-1",
    sessionId: partial.sessionId ?? "session-1",
    runId: partial.runId ?? null,
    checkpointKind: partial.checkpointKind ?? "runtime_control",
    recoveryMode: partial.recoveryMode ?? "replay_required",
    projectedRuntimePhase: partial.projectedRuntimePhase ?? null,
    submissionCommand: partial.submissionCommand ?? null,
    resumable: partial.resumable ?? false,
    replayable: partial.replayable ?? false,
    status: partial.status ?? "running",
    phase: partial.phase ?? "calling_model",
    providerRequestedName: partial.providerRequestedName ?? null,
    providerName: partial.providerName ?? null,
    providerProtocol: partial.providerProtocol ?? null,
    providerModel: partial.providerModel ?? null,
    providerSource: partial.providerSource ?? null,
    providerMode: partial.providerMode ?? null,
    fallbackReason: partial.fallbackReason ?? null,
    completedHops: partial.completedHops ?? 0,
    maxHops: partial.maxHops ?? 0,
    activeToolName: partial.activeToolName ?? null,
    traceSteps: partial.traceSteps ?? [],
    toolActivities: partial.toolActivities ?? [],
    error: partial.error ?? null,
    startedAtMs: partial.startedAtMs ?? 1000,
    updatedAtMs: partial.updatedAtMs ?? 1001,
    stopRequestedAtMs: partial.stopRequestedAtMs ?? null
  };
}

function createHistoryNode(partial: Partial<HistoryNode> = {}): HistoryNode {
  return {
    nodeId: partial.nodeId ?? "node-1",
    sessionId: partial.sessionId ?? "session-1",
    parentNodeId: partial.parentNodeId ?? null,
    branchId: partial.branchId ?? "branch-main",
    forkedFromNodeId: partial.forkedFromNodeId ?? null,
    kind: partial.kind ?? "turn_committed",
    turnId: partial.turnId ?? null,
    transcriptRef: partial.transcriptRef ?? null,
    runRef: partial.runRef ?? null,
    workspaceRef: partial.workspaceRef ?? null,
    summary: partial.summary ?? "History node",
    title: partial.title ?? "History node title",
    history: partial.history ?? [],
    turnTraceHistory: partial.turnTraceHistory ?? [],
    turnCount: partial.turnCount ?? 1,
    lastReferencedFile: partial.lastReferencedFile ?? null,
    createdAtMs: partial.createdAtMs ?? 1000
  };
}

function createTurnTraceRecord(partial: Partial<TurnTraceRecord> = {}): TurnTraceRecord {
  return {
    turnId: partial.turnId ?? "turn-1",
    sessionId: partial.sessionId ?? "session-1",
    title: partial.title ?? "Turn trace",
    phase: partial.phase ?? "completed",
    traceSteps: partial.traceSteps ?? [],
    toolActivities: partial.toolActivities ?? [],
    updatedAt: partial.updatedAt ?? 1000
  };
}

function createHistoryBranch(partial: Partial<HistoryBranch> = {}): HistoryBranch {
  return {
    branchId: partial.branchId ?? "branch-main",
    sessionId: partial.sessionId ?? "session-1",
    baseNodeId: partial.baseNodeId ?? "node-1",
    headNodeId: partial.headNodeId ?? "node-2",
    forkedFromBranchId: partial.forkedFromBranchId ?? null,
    forkedFromNodeId: partial.forkedFromNodeId ?? null,
    label: partial.label ?? "Main",
    createdAtMs: partial.createdAtMs ?? 1000,
    updatedAtMs: partial.updatedAtMs ?? 2000
  };
}

function createHistoryCursor(partial: Partial<HistoryCursorState> = {}): HistoryCursorState {
  return {
    sessionId: partial.sessionId ?? "session-1",
    visibleNodeId: partial.visibleNodeId ?? "node-2",
    activeBranchId: partial.activeBranchId ?? "branch-main",
    branchHeadNodeId: partial.branchHeadNodeId ?? "node-2",
    workspaceNodeId: partial.workspaceNodeId ?? "node-2",
    mode: partial.mode ?? "live",
    authorityMode: partial.authorityMode ?? "host_authoritative",
    cursorVersion: Object.prototype.hasOwnProperty.call(partial, "cursorVersion") ? (partial.cursorVersion ?? null) : 0,
    isAtBranchHead:
      Object.prototype.hasOwnProperty.call(partial, "isAtBranchHead")
        ? Boolean(partial.isAtBranchHead)
        : (partial.visibleNodeId ?? "node-2") === (partial.branchHeadNodeId ?? "node-2")
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
      boundary: partial.action?.boundary ?? "history.checkout.resolved",
      resultKind: partial.action?.resultKind ?? "observe",
      summary: partial.action?.summary ?? "已记录最近一次 history-control 动作证据",
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
  const action = partial.action ?? {};
  const currentContext = partial.currentContext ?? {};
  return {
    actionEvidenceSummary: {
      status: action.status ?? "available",
      sourceFamily: action.sourceFamily ?? "run_control",
      commandKind: action.commandKind ?? "resume_graph_run_stream",
      boundary: action.boundary ?? "run_resume",
      resultKind: action.resultKind ?? "observe",
      summary: action.summary ?? "已记录最近一次 run-control 动作摘要",
      targetSummary: action.targetSummary ?? "恢复 run-1",
      elapsedMs: action.elapsedMs ?? 12,
      blocked: action.blocked ?? false,
      degraded: action.degraded ?? false,
      evidenceId: action.evidenceId ?? "run-control-evidence-1",
      observedAtMs: action.observedAtMs ?? 1000,
      runId: Object.prototype.hasOwnProperty.call(action, "runId") ? (action.runId ?? null) : "run-1",
      turnId: Object.prototype.hasOwnProperty.call(action, "turnId") ? (action.turnId ?? null) : "turn-1",
      checkpointTurnId: Object.prototype.hasOwnProperty.call(action, "checkpointTurnId")
        ? (action.checkpointTurnId ?? null)
        : "turn-1",
      checkpointKind: Object.prototype.hasOwnProperty.call(action, "checkpointKind")
        ? (action.checkpointKind ?? null)
        : "recovery",
      recoveryMode: Object.prototype.hasOwnProperty.call(action, "recoveryMode")
        ? (action.recoveryMode ?? null)
        : "persisted_effect",
      projectedCommand: Object.prototype.hasOwnProperty.call(action, "projectedCommand")
        ? (action.projectedCommand ?? null)
        : "resume_graph_run_stream",
      degradationReason: action.degradationReason ?? null,
      requestSummary: action.requestSummary ?? null,
      startReason: action.startReason ?? null
    },
    currentContextProjection: {
      phase: currentContext.phase ?? "paused",
      checkpointStatus: currentContext.checkpointStatus ?? "ready",
      activeRunId: Object.prototype.hasOwnProperty.call(currentContext, "activeRunId")
        ? (currentContext.activeRunId ?? null)
        : "run-1",
      checkpointKind: Object.prototype.hasOwnProperty.call(currentContext, "checkpointKind")
        ? (currentContext.checkpointKind ?? null)
        : "recovery",
      checkpointRecoveryMode: Object.prototype.hasOwnProperty.call(currentContext, "checkpointRecoveryMode")
        ? (currentContext.checkpointRecoveryMode ?? null)
        : "persisted_effect",
      submissionPlanCommand: Object.prototype.hasOwnProperty.call(currentContext, "submissionPlanCommand")
        ? (currentContext.submissionPlanCommand ?? null)
        : "resume_graph_run_stream"
    }
  };
}

function createSessionRuntimeView(
  session: SessionSnapshot,
  partial: Partial<SessionRuntimeView> = {}
): SessionRuntimeView {
  const historyCursor = partial.historyCursor ?? null;
  return {
    session,
    retrieved: partial.retrieved ?? createRetrievedContext(session),
    checkpoint: partial.checkpoint ?? null,
    submissionPlan: partial.submissionPlan ?? null,
    controlBoundaryEvidence: partial.controlBoundaryEvidence ?? null,
    historyStateAuditSummary:
      partial.historyStateAuditSummary ?? session.historyStateAuditSummary ?? null,
    runControlAuditSummary: partial.runControlAuditSummary ?? session.runControlAuditSummary ?? null,
    historyNodes: partial.historyNodes ?? [],
    historyBranches: partial.historyBranches ?? [],
    historyCursor,
    authorityMode: partial.authorityMode ?? historyCursor?.authorityMode ?? "host_authoritative",
    resolvedVisibleNodeId:
      Object.prototype.hasOwnProperty.call(partial, "resolvedVisibleNodeId")
        ? (partial.resolvedVisibleNodeId ?? null)
        : (historyCursor?.visibleNodeId ?? null),
    activeBranchHeadNodeId:
      Object.prototype.hasOwnProperty.call(partial, "activeBranchHeadNodeId")
        ? (partial.activeBranchHeadNodeId ?? null)
        : (historyCursor?.branchHeadNodeId ?? null),
    isAtBranchHead:
      Object.prototype.hasOwnProperty.call(partial, "isAtBranchHead")
        ? Boolean(partial.isAtBranchHead)
        : (historyCursor?.isAtBranchHead ?? false),
    cursorVersion:
      Object.prototype.hasOwnProperty.call(partial, "cursorVersion")
        ? (partial.cursorVersion ?? null)
        : (historyCursor?.cursorVersion ?? null)
  };
}

function readPersistedSessions() {
  return JSON.parse(window.localStorage.getItem(RUNTIME_STORAGE_KEY) ?? "{\"sessions\":{}}") as {
    sessions: Record<string, unknown>;
  };
}

function writePersistedSessions(payload: Record<string, unknown>) {
  window.localStorage.setItem(
    RUNTIME_STORAGE_KEY,
    JSON.stringify({
      sessions: payload
    })
  );
}

function flushMicrotasks() {
  return new Promise<void>((resolve) => window.setTimeout(resolve, 0));
}

function flushDeferredTurnWork() {
  return new Promise<void>((resolve) => {
    window.setTimeout(() => {
      const requestIdleCallback = (window as Window & {
        requestIdleCallback?: (handler: IdleRequestCallback, options?: IdleRequestOptions) => number;
      }).requestIdleCallback;

      if (typeof requestIdleCallback === "function") {
        requestIdleCallback(() => resolve());
        return;
      }

      window.setTimeout(resolve, 0);
    }, 820);
  });
}

describe("runtime session resilience", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    __resetFrontendFlightRecorderForTests();
    window.localStorage.clear();
    vi.spyOn(console, "info").mockImplementation(() => {});
    tauriMocks.mockSafeListen.mockResolvedValue(() => {});
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    setActivePinia(createPinia());
  });

  it("shows the target session placeholder and surfaces an error when switching fails without cached state", async () => {
    const store = useRuntimeStore();
    const originalMessages = [
      createMessage({ id: "user-1", turnId: "turn-1", role: "user", content: "keep this context" }),
      createMessage({
        id: "assistant-1",
        turnId: "turn-1",
        role: "assistant",
        content: "current session reply"
      })
    ];
    const originalSessionList: SessionOverview[] = [
      {
        conversationId: "session-current",
        title: "Current session",
        summary: "Has context",
        turnCount: 1,
        lastReferencedFile: null,
        updatedAtMs: 1000
      }
    ];

    store.$patch({
      sessionId: "session-current",
      sessionList: originalSessionList,
      phase: "ready",
      draftMessage: "draft message",
      sessionSummary: "current summary",
      messages: originalMessages,
      turnTraceHistory: [createTrace({ turnId: "turn-1", sessionSummary: "current summary" })]
    });

    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "load_session_runtime_view") {
        throw new Error("snapshot exploded");
      }
      throw new Error(`unexpected command: ${command}`);
    });

    await store.switchSession("session-next");
    await flushMicrotasks();

    expect(store.sessionId).toBe("session-next");
    expect(store.phase).toBe("connecting");
    expect(store.draftMessage).toBe("");
    expect(store.messages).toEqual([]);
    expect(store.sessionList).toEqual(originalSessionList);
    expect(store.sessionOperation).toBeNull();
    expect(store.sessionError).toContain("snapshot exploded");
  });

  it("switches immediately from cached session state while host refresh is still pending", async () => {
    const store = useRuntimeStore();
    const originalMessages = [
      createMessage({ id: "user-current", turnId: "turn-current", role: "user", content: "current session" }),
      createMessage({ id: "assistant-current", turnId: "turn-current", role: "assistant", content: "current reply" })
    ];
    const nextSnapshot = createSnapshot({
      conversationId: "session-next",
      title: "Next session",
      summary: "Next summary",
      history: [
        { role: "user", content: "next session" },
        { role: "assistant", content: "next reply" }
      ],
      turnCount: 1,
      updatedAtMs: 2400
    });
    const cachedNextMessages = [
      createMessage({ id: "user-next", turnId: "turn-next", role: "user", content: "cached next session" }),
      createMessage({ id: "assistant-next", turnId: "turn-next", role: "assistant", content: "cached next reply" })
    ];
    const nextSessionList: SessionOverview[] = [
      {
        conversationId: "session-next",
        title: "Next session",
        summary: "Next summary",
        turnCount: 1,
        lastReferencedFile: null,
        updatedAtMs: 2400
      },
      {
        conversationId: "session-current",
        title: "Current session",
        summary: "Current summary",
        turnCount: 1,
        lastReferencedFile: null,
        updatedAtMs: 1000
      }
    ];

    writePersistedSessions({
      "session-next": {
        phase: "ready",
        messages: cachedNextMessages,
        attachmentAssets: [],
        sessionSummary: "Cached next summary",
        providerRequestedName: "",
        providerName: "",
        providerProtocol: "",
        providerModel: "",
        providerSource: "",
        providerMode: "",
        fallbackReason: null,
        inputTokens: null,
        outputTokens: null,
        totalTokens: null,
        firstTokenLatencyMs: null
      }
    });

    let resolveRuntimeView: ((value: SessionRuntimeView) => void) | null = null;
    const runtimeViewPromise = new Promise<SessionRuntimeView>((resolve) => {
      resolveRuntimeView = resolve;
    });

    store.$patch({
      sessionId: "session-current",
      sessionList: [nextSessionList[1]],
      phase: "ready",
      sessionSummary: "Current summary",
      messages: originalMessages
    });

    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "load_session_runtime_view") {
        return runtimeViewPromise;
      }

      if (command === "list_sessions") {
        return nextSessionList;
      }

      throw new Error(`unexpected command: ${command}`);
    });

    const switchingPromise = store.switchSession("session-next");
    await flushMicrotasks();

    expect(store.sessionOperation).toBeNull();
    expect(store.sessionId).toBe("session-next");
    expect(store.phase).toBe("ready");
    expect(store.messages.map((message) => `${message.role}:${message.content}`)).toEqual([
      "user:cached next session",
      "assistant:cached next reply"
    ]);

    resolveRuntimeView?.(createSessionRuntimeView(nextSnapshot));
    await switchingPromise;
    await flushMicrotasks();

    expect(store.sessionOperation).toBeNull();
    expect(store.sessionId).toBe("session-next");
    expect(store.phase).toBe("ready");
    expect(store.retrievedContext?.sessionContext.conversationId).toBe("session-next");
    expect(store.sessionList).toEqual(nextSessionList);
    expect(store.messages.map((message) => `${message.role}:${message.content}`)).toEqual([
      "user:next session",
      "assistant:next reply"
    ]);
  });

  it("preserves historical checkout when switching away and back in browser fallback mode", async () => {
    const store = useRuntimeStore();
    tauriMocks.mockIsTauriAvailable.mockReturnValue(false);

    const sessionId = "session-current";
    const otherSessionId = "session-other";
    const currentMessages = [
      createMessage({ id: "user-old", turnId: "turn-old", role: "user", content: "old question" }),
      createMessage({ id: "assistant-old", turnId: "turn-old", role: "assistant", content: "old answer" })
    ];

    store.$patch({
      sessionId,
      sessionList: [
        {
          conversationId: sessionId,
          title: "Current session",
          summary: "current summary",
          turnCount: 1,
          lastReferencedFile: null,
          updatedAtMs: 2000
        },
        {
          conversationId: otherSessionId,
          title: "Other session",
          summary: "other summary",
          turnCount: 1,
          lastReferencedFile: null,
          updatedAtMs: 1000
        }
      ],
      activeBranchId: "branch-main",
      branchHeadNodeId: "node-head",
      visibleNodeId: "node-old",
      historyCursorMode: "historical",
      historyNodes: [
        createHistoryNode({ nodeId: "node-old", sessionId, turnId: "turn-old", createdAtMs: 1000 }),
        createHistoryNode({ nodeId: "node-head", sessionId, turnId: "turn-head", createdAtMs: 2000 })
      ],
      historyBranches: [createHistoryBranch({ branchId: "branch-main", sessionId, headNodeId: "node-head" })],
      messages: currentMessages,
      turnTraceHistory: [createTrace({ turnId: "turn-old", title: "old turn", updatedAt: 1000 })],
      sessionSummary: "current summary",
      phase: "ready"
    });
    store.persistHistory();

    writePersistedSessions({
      [sessionId]: {
        phase: "ready",
        messages: currentMessages,
        attachmentAssets: [],
        sessionSummary: "current summary",
        providerRequestedName: "",
        providerName: "",
        providerProtocol: "",
        providerModel: "",
        providerSource: "",
        providerMode: "",
        fallbackReason: null,
        inputTokens: null,
        outputTokens: null,
        totalTokens: null,
        firstTokenLatencyMs: null,
        visibleNodeId: "node-old",
        branchHeadNodeId: "node-head",
        activeBranchId: "branch-main",
        historyCursorMode: "historical",
        historyNodes: [
          createHistoryNode({ nodeId: "node-old", sessionId, turnId: "turn-old", createdAtMs: 1000 }),
          createHistoryNode({ nodeId: "node-head", sessionId, turnId: "turn-head", createdAtMs: 2000 })
        ],
        historyBranches: [createHistoryBranch({ branchId: "branch-main", sessionId, headNodeId: "node-head" })]
      },
      [otherSessionId]: {
        phase: "ready",
        messages: [
          createMessage({ id: "user-other", turnId: "turn-other", role: "user", content: "other question" }),
          createMessage({ id: "assistant-other", turnId: "turn-other", role: "assistant", content: "other answer" })
        ],
        attachmentAssets: [],
        sessionSummary: "other summary",
        providerRequestedName: "",
        providerName: "",
        providerProtocol: "",
        providerModel: "",
        providerSource: "",
        providerMode: "",
        fallbackReason: null,
        inputTokens: null,
        outputTokens: null,
        totalTokens: null,
        firstTokenLatencyMs: null
      }
    });

    await store.switchSession(otherSessionId);
    await store.switchSession(sessionId);

    expect(store.sessionId).toBe(sessionId);
    expect(store.visibleNodeId).toBe("node-old");
    expect(store.branchHeadNodeId).toBe("node-head");
    expect(store.activeBranchId).toBe("branch-main");
    expect(store.historyCursorMode).toBe("historical");
    expect(store.messages.map((message) => message.content)).toEqual(["old question", "old answer"]);

    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
  });

  it("preserves the rolled-back initial empty state when switching away and back in browser fallback mode", async () => {
    const store = useRuntimeStore();
    tauriMocks.mockIsTauriAvailable.mockReturnValue(false);

    const sessionId = "session-current";
    const otherSessionId = "session-other";

    writePersistedSessions({
      [otherSessionId]: {
        phase: "ready",
        messages: [
          createMessage({ id: "user-other", turnId: "turn-other", role: "user", content: "other question" }),
          createMessage({ id: "assistant-other", turnId: "turn-other", role: "assistant", content: "other answer" })
        ],
        attachmentAssets: [],
        sessionSummary: "other summary",
        providerRequestedName: "",
        providerName: "",
        providerProtocol: "",
        providerModel: "",
        providerSource: "",
        providerMode: "",
        fallbackReason: null,
        inputTokens: null,
        outputTokens: null,
        totalTokens: null,
        firstTokenLatencyMs: null
      }
    });

    store.$patch({
      sessionId,
      sessionList: [
        {
          conversationId: sessionId,
          title: "Current session",
          summary: "current summary",
          turnCount: 2,
          lastReferencedFile: null,
          updatedAtMs: 2000
        },
        {
          conversationId: otherSessionId,
          title: "Other session",
          summary: "other summary",
          turnCount: 1,
          lastReferencedFile: null,
          updatedAtMs: 1000
        }
      ],
      activeBranchId: "branch-main",
      branchHeadNodeId: "node-head",
      visibleNodeId: "node-head",
      historyCursorMode: "live",
      historyNodes: [
        createHistoryNode({ nodeId: "node-old", sessionId, turnId: "turn-old", createdAtMs: 1000 }),
        createHistoryNode({ nodeId: "node-head", sessionId, turnId: "turn-head", createdAtMs: 2000 })
      ],
      historyBranches: [
        createHistoryBranch({ branchId: "branch-main", sessionId, headNodeId: "node-head", baseNodeId: "node-old" })
      ],
      messages: [
        createMessage({ id: "user-old", turnId: "turn-old", role: "user", content: "old question" }),
        createMessage({ id: "assistant-old", turnId: "turn-old", role: "assistant", content: "old answer" }),
        createMessage({ id: "user-head", turnId: "turn-head", role: "user", content: "head question" }),
        createMessage({ id: "assistant-head", turnId: "turn-head", role: "assistant", content: "head answer" })
      ],
      turnTraceHistory: [
        createTrace({ turnId: "turn-old", title: "old turn", updatedAt: 1000 }),
        createTrace({ turnId: "turn-head", title: "head turn", updatedAt: 2000 })
      ],
      sessionSummary: "current summary",
      phase: "ready"
    });

    store.rollbackToInitialState();

    await store.switchSession(otherSessionId);
    await store.switchSession(sessionId);

    expect(store.sessionId).toBe(sessionId);
    expect(store.messages).toEqual([]);
    expect(store.initialRollbackActive).toBe(true);
    expect(store.historyCursorMode).toBe("historical_dirty");
    expect(store.visibleNodeId).toBeNull();
  });

  it("prefers retrieval session summary over raw snapshot summary when loading a session", async () => {
    const store = useRuntimeStore();
    const snapshot = createSnapshot({
      conversationId: "session-retrieved-summary",
      title: "Retrieved summary session",
      summary: "legacy snapshot summary",
      history: [
        { role: "user", content: "请继续推进 PA-018" },
        { role: "assistant", content: "继续处理中" }
      ],
      turnCount: 1,
      updatedAtMs: 2600
    });
    const retrieved = createRetrievedContext({
      ...snapshot,
      summary: "retrieval summary from host"
    });

    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "load_session_runtime_view") {
        return createSessionRuntimeView(snapshot, { retrieved });
      }

      if (command === "list_sessions") {
        return [] satisfies SessionOverview[];
      }

      throw new Error(`unexpected command: ${command}`);
    });

    await store.loadSessionState("session-retrieved-summary");

    expect(store.retrievedContext?.sessionContext.summary).toBe("retrieval summary from host");
    expect(store.sessionSummary).toBe("retrieval summary from host");
  });

  it("rolls back initializeSessions when loading the latest session fails", async () => {
    const store = useRuntimeStore();
    const originalMessages = [
      createMessage({ id: "user-init", turnId: "turn-init", role: "user", content: "keep original session" })
    ];

    store.$patch({
      sessionId: "session-existing",
      sessionList: [
        {
          conversationId: "session-existing",
          title: "Existing session",
          summary: "Original summary",
          turnCount: 1,
          lastReferencedFile: null,
          updatedAtMs: 900
        }
      ],
      phase: "ready",
      sessionSummary: "Original summary",
      messages: originalMessages
    });

    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "list_sessions") {
        return [
          {
            conversationId: "session-latest",
            title: "Latest session",
            summary: "New summary",
            turnCount: 2,
            lastReferencedFile: null,
            updatedAtMs: 1200
          }
        ] satisfies SessionOverview[];
      }

      if (command === "load_session_runtime_view") {
        throw new Error("load latest failed");
      }

      throw new Error(`unexpected command: ${command}`);
    });

    await store.initializeSessions();

    expect(store.sessionId).toBe("session-existing");
    expect(store.phase).toBe("ready");
    expect(store.messages).toEqual(originalMessages);
    expect(store.sessionList).toEqual([
      {
        conversationId: "session-existing",
        title: "Existing session",
        summary: "Original summary",
        turnCount: 1,
        lastReferencedFile: null,
        updatedAtMs: 900
      }
    ]);
    expect(store.sessionOperation).toBeNull();
    expect(store.sessionError).toContain("load latest failed");
  });

  it("falls back to a derived retrieved context when runtime view omits retrieved context", async () => {
    const store = useRuntimeStore();
    const snapshot = createSnapshot({
      conversationId: "session-derived",
      title: "Derived session",
      summary: "Derived summary",
      history: [
        { role: "user", content: "请继续推进 PA-018" },
        { role: "assistant", content: "继续处理中" }
      ],
      turnCount: 1,
      updatedAtMs: 2400
    });

    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "load_session_runtime_view") {
        return {
          session: snapshot,
          checkpoint: null
        } as SessionRuntimeView;
      }

      if (command === "list_sessions") {
        return [] satisfies SessionOverview[];
      }

      throw new Error(`unexpected command: ${command}`);
    });

    await store.loadSessionState("session-derived");

    expect(store.sessionId).toBe("session-derived");
    expect(store.retrievedContext?.turnContext.userMessage).toBe("请继续推进 PA-018");
    expect(store.retrievedContext?.sessionContext.summary).toBe("Derived summary");
    expect(store.retrievedContext?.longTermMemory.status).toBe("empty");
    expect(store.sessionList).toEqual([]);
  });

  it("restores failed phase for a saved session that only has failed trace history", async () => {
    const store = useRuntimeStore();
    const snapshot = createSnapshot({
      conversationId: "session-failed-legacy",
      title: "Failed legacy session",
      summary: "legacy failed summary",
      history: [],
      turnTraceHistory: [
        createTrace({
          turnId: "turn-failed",
          phase: "failed",
          error: "legacy failed trace",
          eventId: "turn-failed:1",
          eventType: "turn.failed",
          eventVersion: "turn-event-v1",
          sequence: 1,
          emittedAtMs: 1000
        })
      ],
      turnCount: 0,
      updatedAtMs: 1000
    });

    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "load_session_runtime_view") {
        return createSessionRuntimeView(snapshot);
      }

      if (command === "list_sessions") {
        return [
          {
            conversationId: "session-failed-legacy",
            title: "Failed legacy session",
            summary: "legacy failed summary",
            turnCount: 0,
            lastReferencedFile: null,
            updatedAtMs: 1000
          }
        ] satisfies SessionOverview[];
      }

      throw new Error(`unexpected command: ${command}`);
    });

    await store.loadSessionState("session-failed-legacy");

    expect(store.phase).toBe("failed");
    expect(store.messages).toEqual([]);
    expect(store.turnTraceHistory).toHaveLength(1);
    expect(store.turnTraceHistory[0]?.error).toBe("legacy failed trace");
  });

  it("restores session state and cache when delete_session fails", async () => {
    const store = useRuntimeStore();
    const targetMessages = [
      createMessage({ id: "target-user", turnId: "target-turn", role: "user", content: "target session" })
    ];

    window.localStorage.setItem(
      RUNTIME_STORAGE_KEY,
      JSON.stringify({
        sessions: {
          "session-delete": {
            messages: targetMessages,
            turnTraceHistory: [createTrace({ turnId: "target-turn", updatedAt: 2000 })],
            sessionSummary: "delete summary",
            providerRequestedName: "",
            providerName: "",
            providerProtocol: "",
            providerModel: "",
            providerSource: "",
            providerMode: "",
            fallbackReason: null,
            inputTokens: null,
            outputTokens: null,
            totalTokens: null,
            firstTokenLatencyMs: null
          }
        }
      })
    );

    const originalSessionList: SessionOverview[] = [
      {
        conversationId: "session-current",
        title: "Current session",
        summary: "Current summary",
        turnCount: 1,
        lastReferencedFile: null,
        updatedAtMs: 1000
      },
      {
        conversationId: "session-delete",
        title: "Delete me",
        summary: "Delete summary",
        turnCount: 1,
        lastReferencedFile: null,
        updatedAtMs: 2000
      }
    ];

    store.$patch({
      sessionId: "session-current",
      sessionList: originalSessionList,
      phase: "ready",
      sessionSummary: "Current summary",
      messages: [createMessage({ id: "current-user", turnId: "current-turn", role: "user", content: "current session" })]
    });

    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "delete_session") {
        throw new Error("delete backend failed");
      }
      throw new Error(`unexpected command: ${command}`);
    });

    await store.deleteSession("session-delete");

    expect(store.sessionId).toBe("session-current");
    expect(store.phase).toBe("ready");
    expect(store.sessionList).toEqual(originalSessionList);
    expect(store.sessionOperation).toBeNull();
    expect(store.deletingSessionSet).toEqual({});
    expect(store.sessionError).toContain("delete backend failed");
    expect(readPersistedSessions().sessions["session-delete"]).toBeTruthy();
  });

  it("allows deleting another saved session while an inactive delete is already in flight", async () => {
    const store = useRuntimeStore();

    store.$patch({
      sessionId: "session-current",
      sessionList: [
        {
          conversationId: "session-current",
          title: "Current session",
          summary: "Current summary",
          turnCount: 1,
          lastReferencedFile: null,
          updatedAtMs: 1000
        },
        {
          conversationId: "session-delete-a",
          title: "Delete A",
          summary: "Delete A summary",
          turnCount: 1,
          lastReferencedFile: null,
          updatedAtMs: 2000
        },
        {
          conversationId: "session-delete-b",
          title: "Delete B",
          summary: "Delete B summary",
          turnCount: 1,
          lastReferencedFile: null,
          updatedAtMs: 3000
        }
      ],
      phase: "ready",
      sessionSummary: "Current summary",
      messages: [createMessage({ id: "current-user", turnId: "current-turn", role: "user", content: "current session" })]
    });

    let releaseFirstDelete: (() => void) | null = null;
    let releaseSecondDelete: (() => void) | null = null;
    tauriMocks.mockSafeInvoke.mockImplementation((command: string, payload?: { sessionId?: string }) => {
      if (command !== "delete_session") {
        throw new Error(`unexpected command: ${command}`);
      }

      if (payload?.sessionId === "session-delete-a") {
        return new Promise<SessionOverview[]>((resolve) => {
          releaseFirstDelete = () =>
            resolve([
              {
                conversationId: "session-current",
                title: "Current session",
                summary: "Current summary",
                turnCount: 1,
                lastReferencedFile: null,
                updatedAtMs: 1000
              },
              {
                conversationId: "session-delete-b",
                title: "Delete B",
                summary: "Delete B summary",
                turnCount: 1,
                lastReferencedFile: null,
                updatedAtMs: 3000
              }
            ]);
        });
      }

      if (payload?.sessionId === "session-delete-b") {
        return new Promise<SessionOverview[]>((resolve) => {
          releaseSecondDelete = () =>
            resolve([
              {
                conversationId: "session-current",
                title: "Current session",
                summary: "Current summary",
                turnCount: 1,
                lastReferencedFile: null,
                updatedAtMs: 1000
              }
            ]);
        });
      }

      throw new Error(`unexpected session delete: ${payload?.sessionId}`);
    });

    const firstDelete = store.deleteSession("session-delete-a");
    expect(store.sessionOperation).toBeNull();
    expect(store.deletingSessionSet).toEqual({ "session-delete-a": true });

    const secondDelete = store.deleteSession("session-delete-b");
    expect(store.deletingSessionSet).toEqual({
      "session-delete-a": true,
      "session-delete-b": true
    });

    releaseFirstDelete?.();
    await firstDelete;
    expect(store.sessionList.map((session) => session.conversationId)).toEqual(["session-current"]);
    expect(store.deletingSessionSet).toEqual({ "session-delete-b": true });

    releaseSecondDelete?.();
    await secondDelete;
    expect(store.sessionList.map((session) => session.conversationId)).toEqual(["session-current"]);
    expect(store.deletingSessionSet).toEqual({});
    expect(store.sessionOperation).toBeNull();
  });

  it("optimistically removes an inactive session before delete_session resolves", async () => {
    const store = useRuntimeStore();

    store.$patch({
      sessionId: "session-current",
      sessionList: [
        {
          conversationId: "session-current",
          title: "Current session",
          summary: "Current summary",
          turnCount: 1,
          lastReferencedFile: null,
          updatedAtMs: 1000
        },
        {
          conversationId: "session-delete",
          title: "Delete me",
          summary: "Delete summary",
          turnCount: 1,
          lastReferencedFile: null,
          updatedAtMs: 2000
        }
      ],
      phase: "ready",
      sessionSummary: "Current summary",
      messages: [createMessage({ id: "current-user", turnId: "current-turn", role: "user", content: "current session" })]
    });

    let releaseDelete: (() => void) | null = null;
    tauriMocks.mockSafeInvoke.mockImplementation((command: string) => {
      if (command !== "delete_session") {
        throw new Error(`unexpected command: ${command}`);
      }

      return new Promise<SessionOverview[]>((resolve) => {
        releaseDelete = () =>
          resolve([
            {
              conversationId: "session-current",
              title: "Current session",
              summary: "Current summary",
              turnCount: 1,
              lastReferencedFile: null,
              updatedAtMs: 1000
            }
          ]);
      });
    });

    const deletePromise = store.deleteSession("session-delete");
    expect(store.sessionList.map((session) => session.conversationId)).toEqual(["session-current"]);
    expect(store.deletingSessionSet).toEqual({ "session-delete": true });

    releaseDelete?.();
    await deletePromise;
    expect(store.sessionList.map((session) => session.conversationId)).toEqual(["session-current"]);
    expect(store.deletingSessionSet).toEqual({});
  });

  it("loads a fallback session after deleting the active session", async () => {
    const store = useRuntimeStore();
    const fallbackSnapshot = createSnapshot({
      conversationId: "session-fallback",
      title: "Fallback session",
      summary: "Recovered summary",
      history: [
        { role: "user", content: "open fallback" },
        { role: "assistant", content: "fallback reply" }
      ],
      turnCount: 1,
      updatedAtMs: 2500
    });

    store.$patch({
      sessionId: "session-current",
      sessionList: [
        {
          conversationId: "session-current",
          title: "Current session",
          summary: "Current summary",
          turnCount: 1,
          lastReferencedFile: null,
          updatedAtMs: 1000
        },
        {
          conversationId: "session-fallback",
          title: "Fallback session",
          summary: "Recovered summary",
          turnCount: 1,
          lastReferencedFile: null,
          updatedAtMs: 2500
        }
      ],
      phase: "ready",
      sessionError: "stale error",
      messages: [
        createMessage({ id: "current-user", turnId: "current-turn", role: "user", content: "current session" })
      ]
    });

    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "delete_session") {
        return [
          {
            conversationId: "session-fallback",
            title: "Fallback session",
            summary: "Recovered summary",
            turnCount: 1,
            lastReferencedFile: null,
            updatedAtMs: 2500
          }
        ] satisfies SessionOverview[];
      }

      if (command === "load_session_runtime_view") {
        return createSessionRuntimeView(fallbackSnapshot);
      }

      throw new Error(`unexpected command: ${command}`);
    });

    await store.deleteSession("session-current");

    expect(store.sessionId).toBe("session-fallback");
    expect(store.phase).toBe("ready");
    expect(store.sessionOperation).toBeNull();
    expect(store.sessionError).toBeNull();
    expect(store.sessionList).toEqual([
      {
        conversationId: "session-fallback",
        title: "Fallback session",
        summary: "Recovered summary",
        turnCount: 1,
        lastReferencedFile: null,
        updatedAtMs: 2500
      }
    ]);
    expect(store.messages.map((message) => `${message.role}:${message.content}`)).toEqual([
      "user:open fallback",
      "assistant:fallback reply"
    ]);
  });

  it("hydrates assistant history from snapshot content instead of stale persisted markdown text", () => {
    const store = useRuntimeStore();

    writePersistedSessions({
      "session-restore": {
        messages: [
          createMessage({
            id: "user-restore",
            turnId: "turn-restore",
            role: "user",
            content: "请整理 markdown"
          }),
          createMessage({
            id: "assistant-restore",
            turnId: "turn-restore",
            role: "assistant",
            content: "```md\\n#旧标题\\n>旧引用\\n```",
            reasoningContent: "旧思考",
            modelName: "ppx/gpt-5.4",
            tokenCount: 128
          }),
          createMessage({
            id: "tool-restore",
            turnId: "turn-restore",
            role: "tool",
            content: "{\"ok\":true}",
            status: "done",
            toolName: "workspace.read_file",
            detail: "结果"
          })
        ],
        turnTraceHistory: [createTrace({ turnId: "turn-restore", updatedAt: 3000 })],
        sessionSummary: "restore summary",
        providerRequestedName: "ppx",
        providerName: "ppx",
        providerProtocol: "openai",
        providerModel: "gpt-5.4",
        providerSource: "saved",
        providerMode: "chat",
        fallbackReason: null,
        inputTokens: 12,
        outputTokens: 34,
        totalTokens: 46,
        firstTokenLatencyMs: 56
      }
    });

    store.applySessionSnapshot(
      "session-restore",
      createSnapshot({
        conversationId: "session-restore",
        summary: "snapshot summary",
        history: [
          { role: "user", content: "请整理 markdown" },
          { role: "assistant", content: "# 新标题\n> 新引用\n**已修复**" }
        ],
        turnCount: 1
      })
    );

    expect(store.messages.map((message) => `${message.role}:${message.content}`)).toEqual([
      "user:请整理 markdown",
      "assistant:# 新标题\n> 新引用\n**已修复**",
      "tool:{\"ok\":true}"
    ]);
    expect(store.messages[1]?.reasoningContent).toBe("旧思考");
    expect(store.messages[1]?.modelName).toBe("ppx/gpt-5.4");
    expect(store.messages[1]?.tokenCount).toBe(128);
    expect(store.messages[2]?.turnId).toBe("turn-restore");

    const persisted = readPersistedSessions().sessions["session-restore"] as {
      messages: ChatMessage[];
    };
    expect(persisted.messages[1]?.content).toBe("# 新标题\n> 新引用\n**已修复**");
  });

  it("hydrates reasoning, model label, and tool history from trace when persisted cache is unavailable", () => {
    const store = useRuntimeStore();

    store.applySessionSnapshot(
      "session-trace-recovery",
      createSnapshot({
        conversationId: "session-trace-recovery",
        summary: "trace recovery summary",
        history: [
          { role: "user", content: "请检查入口文件" },
          { role: "assistant", content: "我已经检查了入口文件，并整理了结果。" }
        ],
        turnTraceHistory: [
          createTrace({
            turnId: "turn-trace-recovery",
            providerName: "OpenAI",
            providerModel: "gpt-5",
            outputTokens: 64,
            updatedAt: 5000,
            toolActivities: [
              {
                id: "tool-read-entry",
                name: "workspace.read_file",
                canonicalToolName: "Read",
                displayNameZh: "读取",
                status: "done",
                summary: "已读取 src/main.ts",
                argumentsText: "{\"path\":\"src/main.ts\"}",
                resultText: "import { createApp } from 'vue';",
                durationSeconds: 2,
                parentActivityId: null,
                artifacts: [{ kind: "snippet", path: "src/main.ts" }],
                error: null,
                capabilityInvocation: {
                  toolName: "workspace.read_file",
                  capabilityId: "builtin:workspace_read_file",
                  sourceId: "builtin-tools",
                  sourceKind: "builtin",
                  capabilityKind: "tool",
                  invocationMode: "direct_tool_call",
                  failureKind: null,
                  requiresApproval: false,
                  hostMediated: false,
                  permissionScope: "workspace.read",
                  permissionFacts: {
                    requiresApproval: false,
                    permissionScope: "workspace.read",
                    hostMediated: false,
                    permissionProfile: "builtin",
                    approvalMode: "none",
                    decisionSource: "runtime"
                  }
                }
              }
            ],
            traceTimeline: [
              {
                id: "model-trace-recovery",
                kind: "call_model",
                label: "CALL MODEL #1",
                state: "completed",
                sequence: 3,
                reasoningContent: "先读取入口文件，再总结关键初始化流程。"
              }
            ]
          })
        ],
        turnCount: 1
      })
    );

    expect(store.messages.map((message) => `${message.role}:${message.content}`)).toEqual([
      "user:请检查入口文件",
      "assistant:我已经检查了入口文件，并整理了结果。",
      "tool:import { createApp } from 'vue';"
    ]);
    expect(store.messages[1]?.reasoningContent).toBe("先读取入口文件，再总结关键初始化流程。");
    expect(store.messages[1]?.modelName).toBe("OpenAI/gpt-5");
    expect(store.messages[1]?.tokenCount).toBe(64);
    expect(store.messages[2]?.toolName).toBe("workspace.read_file");
    expect(store.messages[2]?.detail).toContain("参数");
    expect(store.messages[2]?.detail).toContain("结果");
    expect(store.turnTraceHistory[0]?.toolActivities[0]?.canonicalToolName).toBe("Read");
    expect(store.turnTraceHistory[0]?.toolActivities[0]?.displayNameZh).toBe("读取");
    expect(store.turnTraceHistory[0]?.toolActivities[0]?.artifacts).toEqual([
      { kind: "snippet", path: "src/main.ts" }
    ]);
    expect(store.turnTraceHistory[0]?.toolActivities[0]?.capabilityInvocation?.permissionFacts).toEqual({
      requiresApproval: false,
      permissionScope: "workspace.read",
      hostMediated: false,
      permissionProfile: "builtin",
      approvalMode: "none",
      decisionSource: "runtime"
    });
  });

  it("hydrates message metadata from history when snapshot carries enriched TurnHistoryMessage", () => {
    const store = useRuntimeStore();
    store.applySessionSnapshot(
      "session-enriched-history",
      createSnapshot({
        conversationId: "session-enriched-history",
        summary: "enriched history summary",
        history: [
          {
            role: "user",
            content: "你好",
            turnId: "turn-enriched-1",
            status: "done" as const,
            modelName: null,
            tokenCount: null,
            reasoningContent: null
          },
          {
            role: "assistant",
            content: "收到。",
            turnId: "turn-enriched-1",
            status: "done" as const,
            modelName: "gpt-5",
            tokenCount: 42,
            reasoningContent: "思考过程"
          }
        ],
        turnCount: 1
      })
    );
    expect(store.messages.length).toBe(2);
    expect(store.messages[0].turnId).toBe("turn-enriched-1");
    expect(store.messages[0].status).toBe("done");
    expect(store.messages[1].turnId).toBe("turn-enriched-1");
    expect(store.messages[1].modelName).toBe("gpt-5");
    expect(store.messages[1].tokenCount).toBe(42);
    expect(store.messages[1].reasoningContent).toBe("思考过程");
    expect(store.messages[1].status).toBe("done");
  });

  it("prefers backend snapshot trace history over stale persisted runtime trace cache", () => {
    const store = useRuntimeStore();
    const staleTrace = createTrace({
      turnId: "turn-trace-truth",
      title: "stale trace",
      phase: "failed",
      error: "stale local error",
      outputTokens: 7,
      updatedAt: 1200
    });
    const canonicalTrace = createTrace({
      turnId: "turn-trace-truth",
      title: "canonical trace",
      phase: "completed",
      error: null,
      outputTokens: 42,
      providerModel: "gpt-5",
      updatedAt: 3200
    });

    writePersistedSessions({
      "session-trace-truth": {
        messages: [
          createMessage({
            id: "user-trace-truth",
            turnId: "turn-trace-truth",
            role: "user",
            content: "stale local question"
          }),
          createMessage({
            id: "assistant-trace-truth",
            turnId: "turn-trace-truth",
            role: "assistant",
            content: "stale local answer"
          })
        ],
        turnTraceHistory: [staleTrace],
        sessionSummary: "stale summary",
        providerRequestedName: "saved",
        providerName: "saved",
        providerProtocol: "openai",
        providerModel: "old-model",
        providerSource: "saved",
        providerMode: "chat",
        fallbackReason: null,
        inputTokens: 1,
        outputTokens: 7,
        totalTokens: 8,
        firstTokenLatencyMs: 99
      }
    });

    store.applySessionSnapshot(
      "session-trace-truth",
      createSnapshot({
        conversationId: "session-trace-truth",
        summary: "canonical summary",
        history: [
          { role: "user", content: "fresh backend question" },
          { role: "assistant", content: "fresh backend answer" }
        ],
        turnTraceHistory: [canonicalTrace],
        turnCount: 1,
        updatedAtMs: 3200
      })
    );

    expect(store.turnTraceHistory).toHaveLength(1);
    expect(store.turnTraceHistory[0]?.title).toBe("canonical trace");
    expect(store.turnTraceHistory[0]?.phase).toBe("completed");
    expect(store.turnTraceHistory[0]?.outputTokens).toBe(42);
    expect(store.turnTraceHistory[0]?.error).toBeNull();

    const persisted = readPersistedSessions().sessions["session-trace-truth"] as {
      turnTraceHistory?: TurnTraceRecord[];
    };
    // turnTraceHistory is no longer persisted to localStorage — it lives on the backend.
    expect(persisted.turnTraceHistory).toBeUndefined();
  });

  it("does not restore canonical failed runtime phase from raw persisted trace without terminal envelope", async () => {
    const store = useRuntimeStore();
    const rawFailedTrace = createTrace({
      turnId: "turn-raw-failed",
      phase: "failed",
      eventId: null,
      eventType: null,
      eventVersion: null,
      sequence: null,
      emittedAtMs: null,
      error: "raw failed trace",
      updatedAt: 2200
    });

    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "load_session_runtime_view") {
        return createSessionRuntimeView(
          createSnapshot({
            conversationId: "session-raw-failed",
            summary: "raw failed summary",
            history: [
              { role: "user", content: "raw failed question" },
              { role: "assistant", content: "raw failed answer" }
            ],
            turnTraceHistory: [rawFailedTrace],
            turnCount: 1,
            updatedAtMs: 2200
          })
        );
      }
      if (command === "list_sessions") {
        return [] satisfies SessionOverview[];
      }
      throw new Error(`unexpected command: ${command}`);
    });

    await store.loadSessionState("session-raw-failed");

    expect(store.turnTraceHistory).toHaveLength(1);
    expect(store.turnTraceHistory[0]?.phase).toBe("failed");
    expect(store.turnTraceHistory[0]?.eventType).toBeNull();
    expect(store.phase).toBe("ready");
    expect(store.error).toBeNull();
  });

  it("restores failed runtime phase only when terminal envelope is canonical", async () => {
    const store = useRuntimeStore();
    const canonicalFailedTrace = createTrace({
      turnId: "turn-canonical-failed",
      phase: "failed",
      eventId: "turn-canonical-failed:9",
      eventType: "turn.failed",
      eventVersion: "turn-event-v1",
      sequence: 9,
      emittedAtMs: 9009,
      error: "canonical failed trace",
      updatedAt: 2400
    });

    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "load_session_runtime_view") {
        return createSessionRuntimeView(
          createSnapshot({
            conversationId: "session-canonical-failed",
            summary: "canonical failed summary",
            history: [
              { role: "user", content: "canonical failed question" },
              { role: "assistant", content: "canonical failed answer" }
            ],
            turnTraceHistory: [canonicalFailedTrace],
            turnCount: 1,
            updatedAtMs: 2400
          })
        );
      }
      if (command === "list_sessions") {
        return [] satisfies SessionOverview[];
      }
      throw new Error(`unexpected command: ${command}`);
    });

    await store.loadSessionState("session-canonical-failed");

    expect(store.turnTraceHistory).toHaveLength(1);
    expect(store.turnTraceHistory[0]?.eventType).toBe("turn.failed");
    expect(store.phase).toBe("failed");
  });

  it("hydrates failed assistant history entries with error semantics and detail", async () => {
    const store = useRuntimeStore();
    const failedTrace = createTrace({
      turnId: "turn-hydrated-failed",
      phase: "failed",
      eventId: "turn-hydrated-failed:3",
      eventType: "turn.failed",
      eventVersion: "turn-event-v1",
      sequence: 3,
      emittedAtMs: 3003,
      error: "hydrated failed stack",
      providerName: "OpenAI",
      providerModel: "gpt-5",
      updatedAt: 3003
    });

    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "load_session_runtime_view") {
        return createSessionRuntimeView(
          createSnapshot({
            conversationId: "session-hydrated-failed",
            summary: "hydrated failed summary",
            history: [
              { role: "user", content: "hydrated failed question" },
              { role: "assistant", content: "hydrated failed answer" }
            ],
            turnTraceHistory: [failedTrace],
            turnCount: 1,
            updatedAtMs: 3003
          })
        );
      }
      if (command === "list_sessions") {
        return [] satisfies SessionOverview[];
      }
      throw new Error(`unexpected command: ${command}`);
    });

    await store.loadSessionState("session-hydrated-failed");

    const assistantMessage = store.messages.find((message) => message.role === "assistant");
    expect(assistantMessage).toBeTruthy();
    expect(assistantMessage?.status).toBe("error");
    expect(assistantMessage?.content).toBe("hydrated failed answer");
    expect(assistantMessage?.errorDetail).toBe("hydrated failed stack");
    expect(assistantMessage?.modelName).toBe("OpenAI/gpt-5");
  });

  it("does not hydrate successful assistant history entries with error semantics", () => {
    const store = useRuntimeStore();
    store.applySessionSnapshot(
      "session-no-error",
      createSnapshot({
        conversationId: "session-no-error",
        summary: "no error summary",
        history: [
          { role: "user", content: "normal question" },
          { role: "assistant", content: "normal answer" }
        ],
        turnTraceHistory: [
          createTrace({
            turnId: "turn-no-error",
            phase: "completed",
            sessionSummary: "normal answer / graph=main / session=session-no-error / turns=1 / provider=test / mode=chat"
          })
        ],
        turnCount: 1
      })
    );

    const assistantMessage = store.messages.find((message) => message.role === "assistant");
    expect(assistantMessage).toBeTruthy();
    expect(assistantMessage?.content).toBe("normal answer");
    expect(assistantMessage?.status).toBe("done");
    expect(assistantMessage?.errorDetail).toBeFalsy();
  });

  it("cleans up stale persisted error state when trace has no error", () => {
    const store = useRuntimeStore();
    writePersistedSessions({
      "session-stale-error": {
        messages: [
          createMessage({
            id: "user-stale",
            turnId: "turn-stale",
            role: "user",
            content: "question"
          }),
          createMessage({
            id: "assistant-stale",
            turnId: "turn-stale",
            role: "assistant",
            content: "answer",
            status: "error",
            errorDetail: "stale error detail from old bug"
          })
        ],
        turnTraceHistory: [
          createTrace({
            turnId: "turn-stale",
            phase: "completed",
            sessionSummary: "answer / graph=main / session=session-stale-error / turns=1 / provider=test / mode=chat"
          })
        ],
        sessionSummary: "stale summary",
        providerRequestedName: null,
        providerName: null,
        providerProtocol: null,
        providerModel: null,
        providerSource: null,
        providerMode: null,
        fallbackReason: null,
        inputTokens: null,
        outputTokens: null,
        totalTokens: null,
        firstTokenLatencyMs: null
      }
    });

    store.applySessionSnapshot(
      "session-stale-error",
      createSnapshot({
        conversationId: "session-stale-error",
        summary: "fresh summary",
        history: [
          { role: "user", content: "question" },
          { role: "assistant", content: "answer" }
        ],
        turnTraceHistory: [
          createTrace({
            turnId: "turn-stale",
            phase: "completed",
            sessionSummary: "answer / graph=main / session=session-stale-error / turns=1 / provider=test / mode=chat"
          })
        ],
        turnCount: 1
      })
    );

    const assistantMessage = store.messages.find((message) => message.role === "assistant");
    expect(assistantMessage).toBeTruthy();
    expect(assistantMessage?.status).toBe("done");
    expect(assistantMessage?.errorDetail).toBeFalsy();
  });

  it("preserves attachment asset lifecycle metadata from snapshots and supports local filtering", () => {
    const store = useRuntimeStore();
    const attachmentAssets: AttachmentAsset[] = [
      {
        id: "asset:session-assets/keep.dataurl",
        sessionId: "session-assets",
        name: "keep.png",
        mimeType: "image/png",
        relativePath: "session-assets/keep.dataurl",
        sizeBytes: 4,
        createdAtMs: 3000,
        status: "active",
        referenceCount: 1,
        lastReferencedAtMs: 3001,
        expiresAtMs: null
      },
      {
        id: "asset:session-assets/draft.dataurl",
        sessionId: "session-assets",
        name: "draft.webp",
        mimeType: "image/webp",
        relativePath: "session-assets/draft.dataurl",
        sizeBytes: 4,
        createdAtMs: 1000,
        status: "reclaimable",
        referenceCount: 0,
        lastReferencedAtMs: null,
        expiresAtMs: 1000 + 7 * 24 * 60 * 60 * 1000
      }
    ];

    store.applySessionSnapshot(
      "session-assets",
      createSnapshot({
        conversationId: "session-assets",
        summary: "asset summary",
        history: [
          { role: "user", content: "inspect image" },
          { role: "assistant", content: "done" }
        ],
        attachmentAssets
      })
    );

    expect(store.attachmentAssets).toEqual(attachmentAssets);
    expect(
      store.getAttachmentAssets({
        mimeType: "webp",
        statuses: ["reclaimable"]
      })
    ).toEqual([attachmentAssets[1]]);

    const persisted = readPersistedSessions().sessions["session-assets"] as {
      attachmentAssets: AttachmentAsset[];
    };
    expect(persisted.attachmentAssets).toEqual(attachmentAssets);
  });

  it("keeps the fallback target but surfaces an error when fallback loading fails", async () => {
    const store = useRuntimeStore();

    store.$patch({
      sessionId: "session-current",
      sessionList: [
        {
          conversationId: "session-current",
          title: "Current session",
          summary: "Current summary",
          turnCount: 1,
          lastReferencedFile: null,
          updatedAtMs: 1000
        },
        {
          conversationId: "session-fallback",
          title: "Fallback session",
          summary: "Recovered summary",
          turnCount: 1,
          lastReferencedFile: null,
          updatedAtMs: 2500
        }
      ],
      phase: "ready",
      sessionSummary: "Current summary",
      messages: [
        createMessage({ id: "current-user", turnId: "current-turn", role: "user", content: "current session" })
      ]
    });

    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "delete_session") {
        return [
          {
            conversationId: "session-fallback",
            title: "Fallback session",
            summary: "Recovered summary",
            turnCount: 1,
            lastReferencedFile: null,
            updatedAtMs: 2500
          }
        ] satisfies SessionOverview[];
      }

      if (command === "load_session_runtime_view") {
        throw new Error("fallback load failed");
      }

      throw new Error(`unexpected command: ${command}`);
    });

    await store.deleteSession("session-current");

    expect(store.sessionId).toBe("session-fallback");
    expect(store.phase).toBe("idle");
    expect(store.sessionOperation).toBeNull();
    expect(store.sessionError).toContain("fallback load failed");
    expect(store.sessionList).toEqual([
      {
        conversationId: "session-fallback",
        title: "Fallback session",
        summary: "Recovered summary",
        turnCount: 1,
        lastReferencedFile: null,
        updatedAtMs: 2500
      }
    ]);
    expect(store.messages).toEqual([]);
    expect(store.turnTraceHistory).toEqual([]);
  });

  it("stays idle when initializeSessions finds no saved sessions", async () => {
    const store = useRuntimeStore();

    store.$patch({
      sessionId: "session-empty",
      phase: "ready",
      sessionError: "old error",
      messages: [
        createMessage({ id: "temp-user", turnId: "temp-turn", role: "user", content: "temporary message" })
      ]
    });

    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "list_sessions") {
        return [] satisfies SessionOverview[];
      }

      if (command === "load_session_runtime_view") {
        return createSessionRuntimeView(
          createSnapshot({
            conversationId: "session-empty",
            summary: "",
            history: [],
            turnCount: 0,
            updatedAtMs: 1000
          })
        );
      }

      throw new Error(`unexpected command: ${command}`);
    });

    await store.initializeSessions();

    expect(store.sessionId).toBe("session-empty");
    expect(store.phase).toBe("idle");
    expect(store.sessionOperation).toBeNull();
    expect(store.sessionError).toBeNull();
    expect(store.messages).toEqual([]);
    expect(store.sessionList).toEqual([]);
  });

  it("records segmented frontend diagnostics during initializeSessions", async () => {
    const store = useRuntimeStore();
    vi.useFakeTimers();
    try {
      tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
        if (command === "health_check") {
          return {
            appName: "Pony Agent",
            appVersion: "dev-preview",
            runtime: "tauri",
            graphEngine: "graph-runtime",
            graphContractVersion: "v1"
          };
        }

        if (command === "list_sessions") {
          return [
            {
              conversationId: "session-latest",
              title: "Latest session",
              summary: "Latest summary",
              turnCount: 1,
              lastReferencedFile: null,
              updatedAtMs: 2000
            }
          ] satisfies SessionOverview[];
        }

        if (command === "load_session_runtime_view") {
          return createSessionRuntimeView(
            createSnapshot({
              conversationId: "session-latest",
              title: "Latest session",
              summary: "Latest summary",
              history: [
                { role: "user", content: "最近一次问题" },
                { role: "assistant", content: "最近一次回复" }
              ],
              turnTraceHistory: [createTrace({ turnId: "turn-latest", updatedAt: 2000 })],
              turnCount: 1,
              updatedAtMs: 2000
            }),
            {
              historyNodes: [createHistoryNode({ nodeId: "node-1", turnId: "turn-latest" })],
              historyBranches: [createHistoryBranch({ branchId: "branch-main", headNodeId: "node-1" })]
            }
          );
        }

        if (command === "append_frontend_trace_events") {
          return undefined;
        }

        throw new Error(`unexpected command: ${command}`);
      });

      await store.fetchHealth();
      await store.initializeSessions();
      await vi.advanceTimersByTimeAsync(500);
      await Promise.resolve();

      const appendCall = tauriMocks.mockSafeInvoke.mock.calls.find(
        ([command]) => command === "append_frontend_trace_events"
      );
      const payload = appendCall?.[1] as
        | { events?: Array<{ category: string; name: string }> }
        | undefined;
      const eventNames = (payload?.events ?? []).map((event) => `${event.category}:${event.name}`);
      // Flight recorder traces are recorded asynchronously; runtime events are now
      // recorded inline in store methods rather than through the flight recorder.
      expect(eventNames).toContain("frontend.recorder:init");
    } finally {
      vi.useRealTimers();
    }
  });

  it("restores the active turn from an execution checkpoint during initialization", async () => {
    const store = useRuntimeStore();

    writePersistedSessions({
      "session-running": {
        messages: [
          createMessage({
            id: "running-user",
            turnId: "turn-running",
            role: "user",
            content: "continue this run"
          }),
          createMessage({
            id: "running-assistant",
            turnId: "turn-running",
            role: "assistant",
            content: "partial answer",
            status: "pending",
            modelName: "OpenAI/gpt-5"
          }),
          createMessage({
            id: "running-tool",
            turnId: "turn-running",
            role: "tool",
            content: "{\"status\":\"working\"}",
            status: "pending",
            toolName: "workspace.read_file"
          })
        ],
        turnTraceHistory: [],
        sessionSummary: "Running summary",
        providerRequestedName: "OpenAI",
        providerName: "OpenAI",
        providerProtocol: "openai",
        providerModel: "gpt-5",
        providerSource: "primary",
        providerMode: "standard",
        fallbackReason: null,
        inputTokens: null,
        outputTokens: null,
        totalTokens: null,
        firstTokenLatencyMs: null
      }
    });

    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string, payload?: Record<string, unknown>) => {
      if (command === "list_sessions") {
        return [
          {
            conversationId: "session-running",
            title: "Running session",
            summary: "Running summary",
            turnCount: 1,
            lastReferencedFile: null,
            updatedAtMs: 2200
          }
        ] satisfies SessionOverview[];
      }

      if (command === "stop_graph_run") {
        expect(payload).toEqual({ runId: "run-running" });
        return null;
      }

      if (command === "load_session_runtime_view") {
        const snapshot = createSnapshot({
          conversationId: "session-running",
          summary: "Running summary",
          history: [],
          turnCount: 0,
          updatedAtMs: 2200
        });
        return createSessionRuntimeView(snapshot, {
          authorityMode: "host_authoritative",
          isAtBranchHead: true,
          retrieved: {
            ...createRetrievedContext(snapshot),
            runState: {
              runId: "run-running",
              goal: "continue this run",
              phase: "running",
              activeTurnId: "turn-running",
              lastCompletedTurnId: null,
              resumeCount: 0,
              lastDecisionSummary: "Continue current run",
              executionCheckpointStatus: "running",
              executionCheckpointPhase: "queued"
            }
          },
          checkpoint: createCheckpoint({
            turnId: "turn-running",
            sessionId: "session-running",
            runId: "run-running",
            phase: "queued",
            projectedRuntimePhase: "calling_tool",
            providerRequestedName: "OpenAI",
            providerName: "OpenAI",
            providerProtocol: "openai",
            providerModel: "gpt-5",
            providerSource: "primary",
            providerMode: "standard",
            activeToolName: "workspace.read_file",
            traceSteps: [{ id: "step-call-tool", label: "Call tool", state: "active" }],
            toolActivities: [
              {
                id: "tool-1",
                name: "workspace.read_file",
                canonicalToolName: "Read",
                displayNameZh: "读取",
                status: "running",
                summary: "Reading workspace",
                parentActivityId: null,
                artifacts: [{ kind: "pending", path: "README.md" }],
                error: null,
                capabilityInvocation: {
                  toolName: "workspace.read_file",
                  capabilityId: "builtin:workspace_read_file",
                  sourceId: "builtin-tools",
                  sourceKind: "builtin",
                  capabilityKind: "tool",
                  invocationMode: "direct_tool_call",
                  failureKind: null,
                  requiresApproval: false,
                  hostMediated: false,
                  permissionScope: "workspace.read",
                  permissionFacts: {
                    requiresApproval: false,
                    permissionScope: "workspace.read",
                    hostMediated: false,
                    permissionProfile: "builtin",
                    approvalMode: "none",
                    decisionSource: "runtime"
                  }
                }
              }
            ],
            updatedAtMs: 2200
          })
        });
      }

      throw new Error(`unexpected command: ${command}`);
    });

    store.$patch({
      sessionId: "session-running",
      phase: "idle",
      messages: []
    });

    await store.initializeSessions();

    expect(store.sessionId).toBe("session-running");
    expect(store.phase).toBe("calling_tool");
    expect(store.isSubmitting).toBe(true);
    expect(store.activeTurnId).toBe("turn-running");
    expect(store.activeRunId).toBe("run-running");
    expect(store.messages.map((message) => `${message.role}:${message.content}`)).toEqual([
      "user:continue this run",
      "assistant:partial answer",
      "tool:{\"status\":\"working\"}"
    ]);
    expect(store.providerName).toBe("OpenAI");
    expect(store.providerModel).toBe("gpt-5");
    expect(store.turnTraceHistory[0]?.turnId).toBe("turn-running");
    expect(store.turnTraceHistory[0]?.phase).toBe("calling_tool");
    expect(store.toolActivities[0]?.canonicalToolName).toBe("Read");
    expect(store.toolActivities[0]?.displayNameZh).toBe("读取");
    expect(store.toolActivities[0]?.capabilityInvocation?.permissionFacts?.permissionScope).toBe("workspace.read");
    expect(store.turnTraceHistory[0]?.toolActivities[0]?.artifacts).toEqual([{ kind: "pending", path: "README.md" }]);

    await store.stopTurn();

    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledWith("stop_graph_run", {
      runId: "run-running"
    });
  });

  it("maps canonical checkpoint phases to runtime phases during initialization", async () => {
    const store = useRuntimeStore();

    writePersistedSessions({
      "session-checkpoint-phase": {
        messages: [
          createMessage({
            id: "checkpoint-user",
            turnId: "turn-checkpoint-phase",
            role: "user",
            content: "resume canonical checkpoint"
          }),
          createMessage({
            id: "checkpoint-assistant",
            turnId: "turn-checkpoint-phase",
            role: "assistant",
            content: "partial canonical answer",
            status: "pending",
            modelName: "OpenAI/gpt-5"
          })
        ],
        turnTraceHistory: [],
        sessionSummary: "Checkpoint summary",
        providerRequestedName: "OpenAI",
        providerName: "OpenAI",
        providerProtocol: "openai",
        providerModel: "gpt-5",
        providerSource: "primary",
        providerMode: "standard",
        fallbackReason: null,
        inputTokens: null,
        outputTokens: null,
        totalTokens: null,
        firstTokenLatencyMs: null
      }
    });

    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "list_sessions") {
        return [
          {
            conversationId: "session-checkpoint-phase",
            title: "Checkpoint session",
            summary: "Checkpoint summary",
            turnCount: 1,
            lastReferencedFile: null,
            updatedAtMs: 2300
          }
        ] satisfies SessionOverview[];
      }

      if (command === "load_session_runtime_view") {
        const snapshot = createSnapshot({
          conversationId: "session-checkpoint-phase",
          summary: "Checkpoint summary",
          history: [],
          turnCount: 0,
          updatedAtMs: 2300
        });
        return createSessionRuntimeView(snapshot, {
          authorityMode: "host_authoritative",
          isAtBranchHead: true,
          retrieved: {
            ...createRetrievedContext(snapshot),
            runState: {
              runId: "run-checkpoint-phase",
              goal: "resume canonical checkpoint",
              phase: "running",
              activeTurnId: "turn-checkpoint-phase",
              lastCompletedTurnId: null,
              resumeCount: 0,
              lastDecisionSummary: "Resume from executing tool",
              executionCheckpointStatus: "running",
              executionCheckpointPhase: "executing_tool"
            }
          },
          checkpoint: createCheckpoint({
            turnId: "turn-checkpoint-phase",
            sessionId: "session-checkpoint-phase",
            runId: "run-checkpoint-phase",
            phase: "executing_tool",
            projectedRuntimePhase: "calling_tool",
            providerRequestedName: "OpenAI",
            providerName: "OpenAI",
            providerProtocol: "openai",
            providerModel: "gpt-5",
            providerSource: "primary",
            providerMode: "standard",
            toolActivities: [
              {
                id: "tool-1",
                name: "workspace.read_file",
                status: "running",
                summary: "Reading workspace"
              }
            ],
            updatedAtMs: 2300
          })
        });
      }

      throw new Error(`unexpected command: ${command}`);
    });

    store.$patch({
      sessionId: "session-checkpoint-phase",
      phase: "idle",
      messages: []
    });

    await store.initializeSessions();

    expect(store.phase).toBe("calling_tool");
    expect(store.isSubmitting).toBe(true);
    expect(store.activeTurnId).toBe("turn-checkpoint-phase");
  });

  it("treats recovery checkpoints as resumable ready-state instead of active submission", async () => {
    const store = useRuntimeStore();

    writePersistedSessions({
      "session-recovery-checkpoint": {
        messages: [
          createMessage({
            id: "recovery-user",
            turnId: "turn-recovery-phase",
            role: "user",
            content: "resume paused graph run"
          }),
          createMessage({
            id: "recovery-assistant",
            turnId: "turn-recovery-phase",
            role: "assistant",
            content: "paused graph run summary",
            status: "done",
            modelName: "OpenAI/gpt-5"
          })
        ],
        turnTraceHistory: [
          createTrace({
            turnId: "turn-recovery-phase",
            phase: "ready",
            providerName: "OpenAI",
            providerModel: "gpt-5",
            updatedAt: 2600
          })
        ],
        sessionSummary: "Recovery summary",
        providerRequestedName: "OpenAI",
        providerName: "OpenAI",
        providerProtocol: "openai",
        providerModel: "gpt-5",
        providerSource: "graph_checkpoint",
        providerMode: "recovery",
        fallbackReason: "graph_user_stop",
        inputTokens: null,
        outputTokens: null,
        totalTokens: null,
        firstTokenLatencyMs: null
      }
    });

    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "list_sessions") {
        return [] satisfies SessionOverview[];
      }

      if (command === "load_session_runtime_view") {
        const snapshot = createSnapshot({
          conversationId: "session-recovery-checkpoint",
          summary: "Recovery summary",
          history: [
            { role: "user", content: "resume paused graph run" },
            { role: "assistant", content: "paused graph run summary" }
          ],
          turnCount: 1,
          updatedAtMs: 2600,
          turnTraceHistory: [
            createTrace({
              turnId: "turn-recovery-phase",
              phase: "ready",
              providerName: "OpenAI",
              providerModel: "gpt-5",
              updatedAt: 2600
            })
          ]
        });
        return createSessionRuntimeView(snapshot, {
          retrieved: {
            ...createRetrievedContext(snapshot),
            runState: {
              runId: "run-recovery-phase",
              goal: "resume paused graph run",
              phase: "paused",
              activeTurnId: null,
              lastCompletedTurnId: "turn-recovery-phase",
              resumeCount: 1,
              lastDecisionSummary: "Waiting to resume graph run",
              executionCheckpointStatus: "ready",
              executionCheckpointPhase: "paused"
            }
          },
          checkpoint: createCheckpoint({
            turnId: "turn-recovery-phase",
            sessionId: "session-recovery-checkpoint",
            checkpointKind: "recovery",
            recoveryMode: "persisted_effect",
            resumable: true,
            replayable: true,
            status: "ready",
            phase: "paused",
            providerRequestedName: "OpenAI",
            providerName: "OpenAI",
            providerProtocol: "openai",
            providerModel: "gpt-5",
            providerSource: "graph_checkpoint",
            providerMode: "recovery",
            fallbackReason: "graph_user_stop",
            updatedAtMs: 2600
          })
        });
      }

      throw new Error(`unexpected command: ${command}`);
    });

    store.$patch({
      sessionId: "session-recovery-checkpoint",
      phase: "idle",
      messages: []
    });

    await store.initializeSessions();

    expect(store.phase).toBe("ready");
    expect(store.isSubmitting).toBe(false);
    expect(store.activeTurnId).toBeNull();
    expect(store.activeRunId).toBe("run-recovery-phase");
    expect(store.providerSource).toBe("graph_checkpoint");
    expect(store.providerMode).toBe("recovery");
    expect(store.fallbackReason).toBe("graph_user_stop");
  });

  it("restores the last persisted terminal phase when no execution checkpoint exists", async () => {
    const store = useRuntimeStore();

    writePersistedSessions({
      "session-terminal-phase": {
        messages: [
          createMessage({
            id: "terminal-user",
            turnId: "turn-terminal-phase",
            role: "user",
            content: "last turn failed"
          }),
          createMessage({
            id: "terminal-assistant",
            turnId: "turn-terminal-phase",
            role: "assistant",
            content: "failure response",
            status: "error",
            modelName: "OpenAI/gpt-5"
          })
        ],
        turnTraceHistory: [
          createTrace({
            turnId: "turn-terminal-phase",
            phase: "failed",
            error: "tool exploded",
            updatedAt: 2400
          })
        ],
        sessionSummary: "Terminal summary",
        providerRequestedName: "OpenAI",
        providerName: "OpenAI",
        providerProtocol: "openai",
        providerModel: "gpt-5",
        providerSource: "primary",
        providerMode: "standard",
        fallbackReason: null,
        inputTokens: null,
        outputTokens: null,
        totalTokens: null,
        firstTokenLatencyMs: null
      }
    });

    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "list_sessions") {
        return [] satisfies SessionOverview[];
      }

      if (command === "load_session_runtime_view") {
        const snapshot = createSnapshot({
          conversationId: "session-terminal-phase",
          summary: "Terminal summary",
          history: [
            { role: "user", content: "last turn failed" },
            { role: "assistant", content: "failure response" }
          ],
          turnCount: 1,
          updatedAtMs: 2400,
          turnTraceHistory: [
            createTrace({
              turnId: "turn-terminal-phase",
              eventId: "turn-terminal-phase:6",
              eventType: "turn.failed",
              eventVersion: "turn-event-v1",
              sequence: 6,
              emittedAtMs: 2406,
              phase: "failed",
              error: "tool exploded",
              updatedAt: 2400
            })
          ]
        });
        return createSessionRuntimeView(snapshot);
      }

      throw new Error(`unexpected command: ${command}`);
    });

    store.$patch({
      sessionId: "session-terminal-phase",
      phase: "idle",
      messages: []
    });

    await store.initializeSessions();

    expect(store.phase).toBe("failed");
    expect(store.isSubmitting).toBe(false);
    expect(store.activeTurnId).toBeNull();
  });

  it("loads normalized capability sources and capabilities through the unified host read-plane", async () => {
    const store = useRuntimeStore();

    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string, payload?: Record<string, unknown>) => {
      if (command === "list_capability_sources") {
        return [
          {
            sourceId: "builtin-tools",
            sourceKind: "builtin",
            displayName: "Builtin Tools",
            transportKind: "in_process",
            serverIdentity: "pony-agent:builtin-tools",
            availability: "available",
            declaredCapabilities: ["tool"],
            permissionProfile: "host-mediated",
            updatedAtMs: 1234
          }
        ];
      }
      if (command === "list_capabilities") {
        expect(payload).toEqual({
          sourceId: "builtin-tools",
          kind: "tool"
        });
        return [
          {
            capabilityId: "builtin:time_now",
            sourceId: "builtin-tools",
            sourceKind: "builtin",
            kind: "tool",
            label: "time_now",
            description: "返回当前本机 UNIX 时间戳",
            invocationMode: "direct_tool_call",
            inputSchemaSummary: "object",
            safetyClass: "host_tool",
            visibility: "default",
            observabilityTags: ["builtin", "tool"],
            requiresApproval: false,
            hostMediated: true,
            permissionScope: "workspace"
          }
        ];
      }
      if (command === "inspect_capability") {
        expect(payload).toEqual({
          capabilityId: "builtin:workspace_run_command"
        });
        return {
          capabilityId: "builtin:workspace_run_command",
          sourceId: "builtin-tools",
          sourceKind: "builtin",
          kind: "tool",
          label: "workspace_run_command",
          description: "在当前工作区内受控执行命令",
          invocationMode: "direct_tool_call",
          inputSchemaSummary: "object",
          safetyClass: "host_tool",
          visibility: "default",
          observabilityTags: ["builtin", "tool"],
          requiresApproval: false,
          hostMediated: true,
          permissionScope: "workspace"
        };
      }
      if (command === "inspect_capability_source") {
        expect(payload).toEqual({
          sourceId: "builtin-tools"
        });
        return {
          sourceId: "builtin-tools",
          sourceKind: "builtin",
          displayName: "Builtin Tools",
          transportKind: "in_process",
          serverIdentity: "pony-agent:builtin-tools",
          availability: "available",
          declaredCapabilities: ["tool"],
          permissionProfile: "host-mediated",
          updatedAtMs: 1234
        };
      }
      throw new Error(`unexpected command: ${command}`);
    });

    await store.fetchCapabilitySources();
    await store.fetchCapabilities({
      sourceId: "builtin-tools",
      kind: "tool"
    });
    const capability = await store.inspectCapability("builtin:workspace_run_command");
    const source = await store.inspectCapabilitySource("builtin-tools");

    expect(store.capabilitySources).toHaveLength(1);
    expect(store.capabilities.map((item) => item.capabilityId)).toEqual(["builtin:time_now"]);
    expect(capability?.capabilityId).toBe("builtin:workspace_run_command");
    expect(source?.sourceId).toBe("builtin-tools");
  });

  it("falls back to builtin capability defaults when capability read-plane commands fail", async () => {
    const store = useRuntimeStore();

    tauriMocks.mockSafeInvoke.mockRejectedValue(new Error("capability bridge offline"));

    await store.fetchCapabilitySources();
    await store.fetchCapabilities({
      sourceId: "builtin-tools",
      kind: "tool"
    });
    const capability = await store.inspectCapability("builtin:workspace_run_command");
    const source = await store.inspectCapabilitySource("builtin-tools");

    expect(store.capabilitySources.map((item) => item.sourceId)).toEqual(["builtin-tools"]);
    expect(store.capabilities.map((item) => item.capabilityId)).toEqual([
      "builtin:workspace_run_command",
      "builtin:echo_input",
      "builtin:workspace_gather_context",
      "builtin:workspace_search_text",
      "builtin:workspace_list_files",
      "builtin:workspace_glob_files",
      "builtin:web_fetch_url",
      "builtin:web_search_query",
      "builtin:mcp_resource_read",
      "builtin:tool_search",
      "builtin:workspace_write_file",
      "builtin:workspace_edit_file",
      "builtin:workspace_batch"
    ]);
    expect(store.capabilities.find((item) => item.capabilityId === "builtin:workspace_gather_context")).toMatchObject({
      canonicalToolName: "Read",
      displayNameZh: "读取",
      requiresApproval: false,
      hostMediated: false,
      permissionScope: "workspace.read",
      permissionFacts: {
        requiresApproval: false,
        permissionScope: "workspace.read",
        hostMediated: false,
        approvalMode: "none",
        decisionSource: "runtime",
        permissionProfile: "builtin"
      }
    });
    expect(capability?.capabilityId).toBe("builtin:workspace_run_command");
    expect(source?.sourceId).toBe("builtin-tools");
  });

  it("keeps second-wave builtin capability defaults aligned with product tool metadata", async () => {
    const store = useRuntimeStore();

    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeInvoke.mockRejectedValue(new Error("capability bridge offline"));

    await store.fetchAvailableTools();
    await store.fetchCapabilities({
      sourceId: "builtin-tools",
      kind: "tool"
    });

    expect(
      store.availableTools.map((tool) => ({
        name: tool.name,
        exposure: tool.exposure,
        executionPrimitive: tool.executionPrimitive
      }))
    ).toEqual(
      expect.arrayContaining([
        {
          name: "MCPResource",
          exposure: "model_visible",
          executionPrimitive: "mcp_resource_read"
        },
        {
          name: "ToolSearch",
          exposure: "deferred",
          executionPrimitive: "tool_search"
        }
      ])
    );

    expect(
      store.capabilities.find((item) => item.capabilityId === "builtin:mcp_resource_read")
    ).toMatchObject({
      canonicalToolName: "MCPResource",
      displayNameZh: "资源"
    });
    expect(
      store.capabilities.find((item) => item.capabilityId === "builtin:tool_search")
    ).toMatchObject({
      canonicalToolName: "ToolSearch",
      displayNameZh: "找工具",
      permissionScope: "capability.discovery",
      permissionFacts: {
        permissionScope: "capability.discovery"
      }
    });
  });

  it("creates a transient browser-preview session while keeping the previous session persisted", async () => {
    const store = useRuntimeStore();
    const nowSpy = vi.spyOn(Date, "now").mockReturnValue(4242);

    tauriMocks.mockIsTauriAvailable.mockReturnValue(false);
    store.$patch({
      sessionId: "browser-current",
      phase: "ready",
      sessionSummary: "Browser summary",
      messages: [
        createMessage({ id: "browser-user", turnId: "browser-turn", role: "user", content: "browser message" })
      ],
      turnTraceHistory: [createTrace({ turnId: "browser-turn", sessionSummary: "Browser summary", updatedAt: 4000 })]
    });
    store.persistHistory();

    await store.createSession();

    expect(store.sessionId).toBe("session-4242");
    expect(store.phase).toBe("idle");
    expect(store.sessionOperation).toBeNull();
    expect(store.sessionError).toBeNull();
    expect(store.messages).toEqual([]);
    expect(store.sessionList).toEqual([
      {
        conversationId: "session-4242",
        title: "新对话",
        summary: "发送第一条消息后保存到历史",
        turnCount: 0,
        lastReferencedFile: null,
        updatedAtMs: 0
      },
      {
        conversationId: "browser-current",
        title: "browser message",
        summary: "Browser summary",
        turnCount: 1,
        lastReferencedFile: null,
        // turnTraceHistory is no longer persisted to localStorage, so updatedAtMs
        // falls back to Date.now() (mocked to 4242) instead of trace's updatedAt (4000).
        updatedAtMs: 4242
      }
    ]);
    expect(Object.keys(readPersistedSessions().sessions)).toEqual(["browser-current"]);

    nowSpy.mockRestore();
  });

  it("creates a transient tauri session without loading the runtime view", async () => {
    const store = useRuntimeStore();
    const nowSpy = vi.spyOn(Date, "now").mockReturnValue(5151);

    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "list_sessions") {
        return [
          {
            conversationId: "session-existing",
            title: "Existing session",
            summary: "Existing summary",
            turnCount: 1,
            lastReferencedFile: null,
            updatedAtMs: 1000
          }
        ] satisfies SessionOverview[];
      }

      if (command === "load_session_runtime_view") {
        throw new Error("should not load runtime view");
      }

      throw new Error(`unexpected command: ${command}`);
    });

    store.$patch({
      sessionId: "session-existing",
      phase: "ready",
      sessionSummary: "Existing summary",
      messages: [
        createMessage({ id: "existing-user", turnId: "turn-existing", role: "user", content: "existing message" })
      ],
      turnTraceHistory: [createTrace({ turnId: "turn-existing", sessionSummary: "Existing summary", updatedAt: 1000 })]
    });

    await store.createSession();

    expect(store.sessionId).toBe("session-5151");
    expect(store.phase).toBe("idle");
    expect(store.messages).toEqual([]);
    expect(store.sessionOperation).toBeNull();
    expect(store.sessionList.map((session) => session.conversationId)).toEqual([
      "session-5151",
      "session-existing"
    ]);
    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledTimes(1);
    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledWith("list_sessions");

    nowSpy.mockRestore();
  });

  it("returns from a transient browser-preview session to the saved session when deleted", async () => {
    const store = useRuntimeStore();

    tauriMocks.mockIsTauriAvailable.mockReturnValue(false);
    writePersistedSessions({
      "browser-current": {
        messages: [
          createMessage({ id: "browser-user", turnId: "browser-turn", role: "user", content: "browser message" }),
          createMessage({
            id: "browser-assistant",
            turnId: "browser-turn",
            role: "assistant",
            content: "browser reply"
          })
        ],
        turnTraceHistory: [createTrace({ turnId: "browser-turn", sessionSummary: "Browser summary", updatedAt: 4000 })],
        sessionSummary: "Browser summary",
        providerRequestedName: "",
        providerName: "",
        providerProtocol: "",
        providerModel: "",
        providerSource: "",
        providerMode: "",
        fallbackReason: null,
        inputTokens: null,
        outputTokens: null,
        totalTokens: null,
        firstTokenLatencyMs: null
      }
    });

    store.$patch({
      sessionId: "session-transient",
      phase: "idle",
      messages: [],
      sessionList: [
        {
          conversationId: "browser-current",
          title: "browser message",
          summary: "Browser summary",
          turnCount: 1,
          lastReferencedFile: null,
          updatedAtMs: 4000
        }
      ]
    });

    await store.deleteSession("session-transient");

    expect(store.sessionId).toBe("browser-current");
    expect(store.phase).toBe("ready");
    expect(store.sessionOperation).toBeNull();
    expect(store.sessionError).toBeNull();
    expect(store.messages.map((message) => `${message.role}:${message.content}`)).toEqual([
      "user:browser message",
      "assistant:browser reply"
    ]);
    expect(store.sessionList).toEqual([
      {
        conversationId: "browser-current",
        title: "browser message",
        summary: "Browser summary",
        turnCount: 1,
        lastReferencedFile: null,
        updatedAtMs: 4000
      }
    ]);
  });

  it("sorts browser-preview sessions by most recent update time", async () => {
    const store = useRuntimeStore();

    tauriMocks.mockIsTauriAvailable.mockReturnValue(false);
    writePersistedSessions({
      "session-older": {
        messages: [createMessage({ id: "older-user", turnId: "older-turn", role: "user", content: "older message" })],
        turnTraceHistory: [createTrace({ turnId: "older-turn", updatedAt: 1000 })],
        sessionSummary: "Older summary",
        providerRequestedName: "",
        providerName: "",
        providerProtocol: "",
        providerModel: "",
        providerSource: "",
        providerMode: "",
        fallbackReason: null,
        inputTokens: null,
        outputTokens: null,
        totalTokens: null,
        firstTokenLatencyMs: null
      },
      "session-newer": {
        messages: [createMessage({ id: "newer-user", turnId: "newer-turn", role: "user", content: "newer message" })],
        turnTraceHistory: [createTrace({ turnId: "newer-turn", updatedAt: 5000 })],
        sessionSummary: "Newer summary",
        providerRequestedName: "",
        providerName: "",
        providerProtocol: "",
        providerModel: "",
        providerSource: "",
        providerMode: "",
        fallbackReason: null,
        inputTokens: null,
        outputTokens: null,
        totalTokens: null,
        firstTokenLatencyMs: null
      }
    });

    await store.loadSessionCatalog();

    expect(store.sessionList.map((session) => session.conversationId)).toEqual([
      "session-newer",
      "session-older"
    ]);
  });

  it("initializes browser-preview mode from the latest persisted session", async () => {
    const store = useRuntimeStore();

    tauriMocks.mockIsTauriAvailable.mockReturnValue(false);
    writePersistedSessions({
      "session-older": {
        messages: [
          createMessage({ id: "older-user", turnId: "older-turn", role: "user", content: "older message" }),
          createMessage({
            id: "older-assistant",
            turnId: "older-turn",
            role: "assistant",
            content: "older reply"
          })
        ],
        turnTraceHistory: [createTrace({ turnId: "older-turn", sessionSummary: "Older summary", updatedAt: 1000 })],
        sessionSummary: "Older summary",
        providerRequestedName: "",
        providerName: "",
        providerProtocol: "",
        providerModel: "",
        providerSource: "",
        providerMode: "",
        fallbackReason: null,
        inputTokens: null,
        outputTokens: null,
        totalTokens: null,
        firstTokenLatencyMs: null
      },
      "session-newer": {
        messages: [
          createMessage({ id: "newer-user", turnId: "newer-turn", role: "user", content: "newer message" }),
          createMessage({
            id: "newer-assistant",
            turnId: "newer-turn",
            role: "assistant",
            content: "newer reply"
          })
        ],
        turnTraceHistory: [createTrace({ turnId: "newer-turn", sessionSummary: "Newer summary", updatedAt: 5000 })],
        sessionSummary: "Newer summary",
        providerRequestedName: "",
        providerName: "",
        providerProtocol: "",
        providerModel: "",
        providerSource: "",
        providerMode: "",
        fallbackReason: null,
        inputTokens: null,
        outputTokens: null,
        totalTokens: null,
        firstTokenLatencyMs: null
      }
    });

    store.$patch({
      sessionId: "local-dev-session",
      phase: "idle",
      messages: []
    });

    await store.initializeSessions();

    expect(store.sessionId).toBe("session-newer");
    expect(store.phase).toBe("ready");
    expect(store.sessionOperation).toBeNull();
    expect(store.sessionError).toBeNull();
    expect(store.sessionList.map((session) => session.conversationId)).toEqual([
      "session-newer",
      "session-older"
    ]);
    expect(store.messages.map((message) => `${message.role}:${message.content}`)).toEqual([
      "user:newer message",
      "assistant:newer reply"
    ]);
  });

  it("completes submitTurn in browser-preview mode and records the turn", async () => {
    const store = useRuntimeStore();
    const nowSpy = vi.spyOn(Date, "now").mockReturnValue(8080);

    tauriMocks.mockIsTauriAvailable.mockReturnValue(false);
    store.$patch({
      sessionId: "browser-session",
      draftMessage: "preview request",
      phase: "idle",
      messages: []
    });

    await store.submitTurn();

    expect(store.phase).toBe("completed");
    expect(store.isSubmitting).toBe(false);
    expect(store.activeTurnId).toBeNull();
    expect(store.draftMessage).toBe("");
    expect(store.messages.length).toBe(2);
    expect(store.messages[0]?.role).toBe("user");
    expect(store.messages[0]?.content).toBe("preview request");
    expect(store.messages[1]?.role).toBe("assistant");
    expect(store.messages[1]?.status).toBe("done");
    expect(store.sessionSummary.length).toBeGreaterThan(0);
    expect(store.turnTraceHistory).toHaveLength(1);
    expect(store.turnTraceHistory[0]?.phase).toBe("completed");
    expect(store.turnTraceHistory[0]?.eventType).toBe("turn.completed");
    expect(store.turnTraceHistory[0]?.eventVersion).toBe("turn-event-v1");
    expect(store.turnTraceHistory[0]?.fallbackReason).toContain("npm run dev");

    nowSpy.mockRestore();
  });

  it("restores a cancelled browser-preview turn from persisted canonical terminal evidence", async () => {
    const store = useRuntimeStore();
    const nowSpy = vi.spyOn(Date, "now").mockReturnValue(9090);

    tauriMocks.mockIsTauriAvailable.mockReturnValue(false);
    store.$patch({
      sessionId: "browser-cancelled-session",
      draftMessage: "please stop this preview turn",
      phase: "idle",
      messages: []
    });

    const submitPromise = store.submitTurn();
    await flushMicrotasks();
    await store.stopTurn();
    await submitPromise;

    expect(store.phase).toBe("cancelled");
    expect(store.messages[1]?.content).toBe("本轮已停止。");
    expect(store.turnTraceHistory[0]?.eventType).toBe("turn.cancelled");
    expect(store.turnTraceHistory[0]?.eventVersion).toBe("turn-event-v1");

    setActivePinia(createPinia());
    const restoredStore = useRuntimeStore();
    tauriMocks.mockIsTauriAvailable.mockReturnValue(false);

    await restoredStore.initializeSessions();

    expect(restoredStore.sessionId).toBe("browser-cancelled-session");
    expect(restoredStore.phase).toBe("cancelled");
    expect(restoredStore.messages[1]?.content).toBe("本轮已停止。");

    nowSpy.mockRestore();
  });

  it("forwards image attachments to start_graph_run_stream and records an image summary", async () => {
    const store = useRuntimeStore();
    const nowSpy = vi.spyOn(Date, "now").mockReturnValue(8181);

    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "inspect_host") {
        return { runs: [] };
      }

      if (command === "start_graph_run_stream") {
        return {
          run: { id: "run-image" },
          turnId: "8181"
        };
      }

      throw new Error(`unexpected command: ${command}`);
    });

    store.$patch({
      sessionId: "image-session",
      draftMessage: "",
      phase: "idle",
      messages: []
    });

    const started = await store.submitTurn({
      images: [
        {
          dataUrl: "data:image/png;base64,Zm9v",
          mimeType: "image/png",
          name: "demo.png"
        }
      ]
    });

    expect(started).toBe(true);
    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledWith("start_graph_run_stream", {
      turnId: "8181",
      runId: null,
      goal: "[已附图片 1 张：demo.png]",
      input: expect.objectContaining({
        message: "请基于附图回答。",
        images: [
          {
            dataUrl: "data:image/png;base64,Zm9v",
            mimeType: "image/png",
            name: "demo.png"
          }
        ]
      })
    });
    expect(store.messages[0]?.content).toContain("[已附图片 1 张：demo.png]");
    nowSpy.mockRestore();
  });

  it("forwards workspace mode to start_graph_run_stream input payload", async () => {
    const store = useRuntimeStore();
    const settingsStore = useSettingsStore();
    const nowSpy = vi.spyOn(Date, "now").mockReturnValue(8282);

    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string, payload?: Record<string, unknown>) => {
      if (command === "inspect_host") {
        return { runs: [] };
      }

      if (command === "start_graph_run_stream") {
        expect(payload).toEqual({
          turnId: "8282",
          runId: null,
          goal: "new request",
          input: expect.objectContaining({
            message: "new request",
            workspaceMode: "work"
          })
        });
        return {
          run: { id: "run-workspace-mode" },
          turnId: "8282"
        };
      }

      throw new Error(`unexpected command: ${command}`);
    });

    settingsStore.$patch({
      settings: {
        workspaceMode: "work"
      }
    });
    store.$patch({
      sessionId: "workspace-mode-session",
      draftMessage: "new request",
      phase: "idle",
      messages: []
    });

    const started = await store.submitTurn();

    expect(started).toBe(true);
    nowSpy.mockRestore();
  });

  it("prefers retrievedContext.runState when continuing an existing graph run", async () => {
    const store = useRuntimeStore();
    const nowSpy = vi.spyOn(Date, "now").mockReturnValue(8383);

    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string, payload?: Record<string, unknown>) => {
      if (command === "continue_graph_run_stream") {
        expect(payload).toEqual({
          turnId: "8383",
          runId: "run-existing",
          input: expect.objectContaining({
            message: "keep going"
          })
        });
        return {
          run: { id: "run-existing" },
          turnId: "8383"
        };
      }

      throw new Error(`unexpected command: ${command}`);
    });

    const snapshot = createSnapshot({
      conversationId: "existing-run-session",
      summary: "existing run summary"
    });
    store.$patch({
      sessionId: "existing-run-session",
      draftMessage: "keep going",
      phase: "idle",
      messages: [],
      retrievedContext: {
        ...createRetrievedContext(snapshot),
        runState: {
          runId: "run-existing",
          phase: "waiting_user",
          goal: "continue existing run"
        }
      }
    });

    const started = await store.submitTurn();

    expect(started).toBe(true);
    expect(tauriMocks.mockSafeInvoke).not.toHaveBeenCalledWith("inspect_host", expect.anything());
    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledWith(
      "continue_graph_run_stream",
      expect.objectContaining({
        turnId: "8383",
        runId: "run-existing"
      })
    );
    nowSpy.mockRestore();
  });

  it("prefers retrievedContext.runState when resuming a paused graph run", async () => {
    const store = useRuntimeStore();
    const nowSpy = vi.spyOn(Date, "now").mockReturnValue(8484);

    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string, payload?: Record<string, unknown>) => {
      if (command === "resume_graph_run_stream") {
        expect(payload).toEqual({
          turnId: "8484",
          runId: "run-paused",
          input: expect.objectContaining({
            message: "resume please"
          })
        });
        return {
          run: { id: "run-paused" },
          turnId: "8484"
        };
      }

      throw new Error(`unexpected command: ${command}`);
    });

    const snapshot = createSnapshot({
      conversationId: "paused-run-session",
      summary: "paused run summary"
    });
    store.$patch({
      sessionId: "paused-run-session",
      draftMessage: "resume please",
      phase: "idle",
      messages: [],
      retrievedContext: {
        ...createRetrievedContext(snapshot),
        runState: {
          runId: "run-paused",
          phase: "paused",
          goal: "resume paused run"
        }
      }
    });

    const started = await store.submitTurn();

    expect(started).toBe(true);
    expect(tauriMocks.mockSafeInvoke).not.toHaveBeenCalledWith("inspect_host", expect.anything());
    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledWith(
      "resume_graph_run_stream",
      expect.objectContaining({
        turnId: "8484",
        runId: "run-paused"
      })
    );
    nowSpy.mockRestore();
  });

  it("prefers replay_required checkpoint over paused runState when recovery contract conflicts", async () => {
    const store = useRuntimeStore();
    const nowSpy = vi.spyOn(Date, "now").mockReturnValue(8494);

    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string, payload?: Record<string, unknown>) => {
      if (command === "start_graph_run_stream") {
        expect(payload).toEqual({
          turnId: "8494",
          runId: null,
          goal: "replay conflicted checkpoint",
          input: expect.objectContaining({
            message: "replay conflicted checkpoint"
          })
        });
        return {
          run: { id: "run-replayed-conflict" },
          turnId: "8494"
        };
      }

      throw new Error(`unexpected command: ${command}`);
    });

    const snapshot = createSnapshot({
      conversationId: "paused-run-conflict-session",
      summary: "paused run conflict summary"
    });
    store.$patch({
      sessionId: "paused-run-conflict-session",
      draftMessage: "replay conflicted checkpoint",
      phase: "idle",
      messages: [],
      activeRunId: "run-paused-conflict",
      retrievedContext: {
        ...createRetrievedContext(snapshot),
        runState: {
          runId: "run-paused-conflict",
          phase: "paused",
          goal: "resume paused run"
        }
      },
      latestExecutionCheckpoint: createCheckpoint({
        turnId: "turn-paused-conflict",
        sessionId: "paused-run-conflict-session",
        checkpointKind: "recovery",
        recoveryMode: "replay_required",
        resumable: true,
        replayable: true,
        status: "ready",
        phase: "paused",
        providerRequestedName: "OpenAI",
        providerName: "OpenAI",
        providerProtocol: "openai",
        providerModel: "gpt-5",
        providerSource: "graph_checkpoint",
        providerMode: "recovery",
        fallbackReason: "graph_user_stop",
        updatedAtMs: 2600
      })
    });

    const started = await store.submitTurn();

    expect(started).toBe(true);
    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledWith(
      "start_graph_run_stream",
      expect.objectContaining({
        turnId: "8494",
        runId: null,
        goal: "replay conflicted checkpoint"
      })
    );
    expect(tauriMocks.mockSafeInvoke).not.toHaveBeenCalledWith(
      "resume_graph_run_stream",
      expect.anything()
    );
    nowSpy.mockRestore();
  });

  it("resumes a recovery checkpoint when retrievedContext.runState is unavailable", async () => {
    const store = useRuntimeStore();
    const nowSpy = vi.spyOn(Date, "now").mockReturnValue(8535);

    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string, payload?: Record<string, unknown>) => {
      if (command === "resume_graph_run_stream") {
        expect(payload).toEqual({
          turnId: "8535",
          runId: "run-checkpoint-only",
          input: expect.objectContaining({
            message: "resume from checkpoint"
          })
        });
        return {
          run: { id: "run-checkpoint-only" },
          turnId: "8535"
        };
      }

      throw new Error(`unexpected command: ${command}`);
    });

    const snapshot = createSnapshot({
      conversationId: "checkpoint-only-session",
      summary: "checkpoint-only summary"
    });
    store.$patch({
      sessionId: "checkpoint-only-session",
      draftMessage: "resume from checkpoint",
      phase: "ready",
      messages: [],
      activeRunId: null,
      retrievedContext: {
        ...createRetrievedContext(snapshot),
        runState: {}
      },
      latestExecutionCheckpoint: createCheckpoint({
        turnId: "turn-checkpoint-only",
        sessionId: "checkpoint-only-session",
        runId: "run-checkpoint-only",
        checkpointKind: "recovery",
        recoveryMode: "persisted_effect",
        projectedRuntimePhase: "ready",
        submissionCommand: "resume_graph_run_stream",
        resumable: true,
        replayable: true,
        status: "ready",
        phase: "paused",
        providerRequestedName: "OpenAI",
        providerName: "OpenAI",
        providerProtocol: "openai",
        providerModel: "gpt-5",
        providerSource: "graph_checkpoint",
        providerMode: "recovery",
        fallbackReason: "graph_user_stop",
        updatedAtMs: 2600
      })
    });

    const started = await store.submitTurn();

    expect(started).toBe(true);
    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledWith(
      "resume_graph_run_stream",
      expect.objectContaining({
        turnId: "8535",
        runId: "run-checkpoint-only"
      })
    );
    expect(tauriMocks.mockSafeInvoke).not.toHaveBeenCalledWith("load_retrieved_context", expect.anything());
    nowSpy.mockRestore();
  });

  it("replays from a recovery checkpoint when recoveryMode requires replay", async () => {
    const store = useRuntimeStore();
    const nowSpy = vi.spyOn(Date, "now").mockReturnValue(8545);

    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string, payload?: Record<string, unknown>) => {
      if (command === "start_graph_run_stream") {
        expect(payload).toEqual({
          turnId: "8545",
          runId: null,
          goal: "replay from checkpoint",
          input: expect.objectContaining({
            message: "replay from checkpoint"
          })
        });
        return {
          run: { id: "run-replayed" },
          turnId: "8545"
        };
      }

      throw new Error(`unexpected command: ${command}`);
    });

    const snapshot = createSnapshot({
      conversationId: "checkpoint-replay-session",
      summary: "checkpoint replay summary"
    });
    store.$patch({
      sessionId: "checkpoint-replay-session",
      draftMessage: "replay from checkpoint",
      phase: "ready",
      messages: [],
      activeRunId: null,
      retrievedContext: {
        ...createRetrievedContext(snapshot),
        runState: {}
      },
      latestExecutionCheckpoint: createCheckpoint({
        turnId: "turn-checkpoint-replay",
        sessionId: "checkpoint-replay-session",
        runId: "run-checkpoint-replay",
        checkpointKind: "recovery",
        recoveryMode: "replay_required",
        projectedRuntimePhase: "ready",
        submissionCommand: "start_graph_run_stream",
        resumable: true,
        replayable: true,
        status: "ready",
        phase: "paused",
        providerRequestedName: "OpenAI",
        providerName: "OpenAI",
        providerProtocol: "openai",
        providerModel: "gpt-5",
        providerSource: "graph_checkpoint",
        providerMode: "recovery",
        fallbackReason: "graph_user_stop",
        updatedAtMs: 2600
      })
    });

    const started = await store.submitTurn();

    expect(started).toBe(true);
    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledWith(
      "start_graph_run_stream",
      expect.objectContaining({
        turnId: "8545",
        runId: null,
        goal: "replay from checkpoint"
      })
    );
    expect(tauriMocks.mockSafeInvoke).not.toHaveBeenCalledWith(
      "resume_graph_run_stream",
      expect.anything()
    );
    nowSpy.mockRestore();
  });

  it("refreshes host retrieval before falling back to inspect_host when local runState is insufficient", async () => {
    const store = useRuntimeStore();
    const nowSpy = vi.spyOn(Date, "now").mockReturnValue(8585);

    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string, payload?: Record<string, unknown>) => {
      if (command === "resolve_graph_run_submission_plan") {
        expect(payload).toEqual({
          sessionId: "retrieval-refresh-session",
          runId: null,
          nodeId: null
        });
        return {
          command: "continue_graph_run_stream",
          runId: "run-refreshed",
          source: "graph_run"
        };
      }

      if (command === "continue_graph_run_stream") {
        expect(payload).toEqual({
          turnId: "8585",
          runId: "run-refreshed",
          input: expect.objectContaining({
            message: "use refreshed retrieval"
          })
        });
        return {
          run: { id: "run-refreshed" },
          turnId: "8585"
        };
      }

      if (command === "inspect_host") {
        throw new Error("inspect_host should not be required");
      }

      throw new Error(`unexpected command: ${command}`);
    });

    store.$patch({
      sessionId: "retrieval-refresh-session",
      draftMessage: "use refreshed retrieval",
      phase: "idle",
      messages: [],
      retrievedContext: {
        ...createRetrievedContext(
          createSnapshot({
            conversationId: "retrieval-refresh-session",
            summary: "stale retrieval summary"
          })
        ),
        runState: {}
      }
    });

    const started = await store.submitTurn();

    expect(started).toBe(true);
    expect(tauriMocks.mockSafeInvoke).not.toHaveBeenCalledWith(
      "load_retrieved_context",
      expect.anything()
    );
    expect(tauriMocks.mockSafeInvoke).not.toHaveBeenCalledWith("inspect_host", expect.anything());
    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledWith(
      "continue_graph_run_stream",
      expect.objectContaining({
        turnId: "8585",
        runId: "run-refreshed"
      })
    );
    nowSpy.mockRestore();
  });

  it("starts a new graph run without inspect_host when refreshed retrieval has no active run", async () => {
    const store = useRuntimeStore();
    const nowSpy = vi.spyOn(Date, "now").mockReturnValue(8686);

    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string, payload?: Record<string, unknown>) => {
      if (command === "resolve_graph_run_submission_plan") {
        expect(payload).toEqual({
          sessionId: "fresh-run-session",
          runId: null,
          nodeId: null
        });
        return {
          command: "start_graph_run_stream",
          runId: null,
          source: "default"
        };
      }

      if (command === "start_graph_run_stream") {
        expect(payload).toEqual({
          turnId: "8686",
          runId: null,
          goal: "brand new request",
          input: expect.objectContaining({
            message: "brand new request"
          })
        });
        return {
          run: { id: "run-fresh" },
          turnId: "8686"
        };
      }

      if (command === "inspect_host") {
        throw new Error("inspect_host should not be required");
      }

      throw new Error(`unexpected command: ${command}`);
    });

    store.$patch({
      sessionId: "fresh-run-session",
      draftMessage: "brand new request",
      phase: "idle",
      messages: [],
      retrievedContext: {
        ...createRetrievedContext(
          createSnapshot({
            conversationId: "fresh-run-session",
            summary: "stale retrieval summary"
          })
        ),
        runState: {}
      }
    });

    const started = await store.submitTurn();

    expect(started).toBe(true);
    expect(store.activeRunId).toBe("run-fresh");
    expect(tauriMocks.mockSafeInvoke).not.toHaveBeenCalledWith(
      "load_retrieved_context",
      expect.anything()
    );
    expect(tauriMocks.mockSafeInvoke).not.toHaveBeenCalledWith("inspect_host", expect.anything());
    nowSpy.mockRestore();
  });

  it("hydrates submissionPlan from session runtime view and reuses its runId", async () => {
    const store = useRuntimeStore();

    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "list_sessions") {
        return [] satisfies SessionOverview[];
      }

      if (command === "load_session_runtime_view") {
        const snapshot = createSnapshot({
          conversationId: "submission-plan-hydration",
          summary: "hydration summary",
          history: [{ role: "user", content: "hydrate me" }],
          turnCount: 1,
          updatedAtMs: 2400
        });
        return createSessionRuntimeView(snapshot, {
          retrieved: {
            ...createRetrievedContext(snapshot),
            runState: {}
          },
          submissionPlan: {
            command: "continue_graph_run_stream",
            runId: "run-hydrated-plan",
            source: "graph_run"
          },
          controlBoundaryEvidence: [
            {
              hookPoint: "stop_requested",
              canonicalEventType: "graph_run.stop_requested",
              canonicalPhase: "paused",
              summary: "user requested stop",
              hookEnvelope: {
                sessionId: "submission-plan-hydration",
                runId: "run-hydrated-plan",
                phase: "paused",
                command: "stop_graph_run",
                source: "graph_run",
                resumable: true,
                replayable: false
              },
              createdAtMs: 2401
            }
          ],
          runControlAuditSummary: createRunControlAuditSummary({
            action: {
              commandKind: "stop_graph_run",
              boundary: "stop_requested",
              resultKind: "observe",
              summary: "已请求停止当前运行，等待恢复或重放。",
              targetSummary: "暂停 run-hydrated-plan",
              runId: "run-hydrated-plan",
              projectedCommand: "continue_graph_run_stream"
            },
            currentContext: {
              phase: "paused",
              activeRunId: "run-hydrated-plan",
              submissionPlanCommand: "continue_graph_run_stream"
            }
          })
        });
      }

      throw new Error(`unexpected command: ${command}`);
    });

    store.$patch({
      sessionId: "submission-plan-hydration",
      phase: "idle",
      messages: []
    });

    await store.initializeSessions();

    expect(store.latestGraphRunSubmissionPlan).toEqual({
      command: "continue_graph_run_stream",
      runId: "run-hydrated-plan",
      source: "graph_run"
    });
    expect(store.latestGraphRunControlBoundaryEvidence).toEqual([
      expect.objectContaining({
        hookPoint: "stop_requested",
        canonicalEventType: "graph_run.stop_requested",
        canonicalPhase: "paused"
      })
    ]);
    expect(store.activeRunId).toBe("run-hydrated-plan");
    expect(store.latestRunControlAuditSummary).toEqual(
      expect.objectContaining({
        actionEvidenceSummary: expect.objectContaining({
          commandKind: "stop_graph_run",
          projectedCommand: "continue_graph_run_stream"
        }),
        currentContextProjection: expect.objectContaining({
          activeRunId: "run-hydrated-plan"
        })
      })
    );
  });

  it("hydrates default submissionPlan without reviving an active run id", async () => {
    const store = useRuntimeStore();

    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "list_sessions") {
        return [] satisfies SessionOverview[];
      }

      if (command === "load_session_runtime_view") {
        const snapshot = createSnapshot({
          conversationId: "submission-plan-default",
          summary: "completed session",
          history: [{ role: "user", content: "hydrate completed session" }],
          turnCount: 1,
          updatedAtMs: 2500
        });
        return createSessionRuntimeView(snapshot, {
          retrieved: {
            ...createRetrievedContext(snapshot),
            runState: {}
          },
          submissionPlan: {
            command: "start_graph_run_stream",
            runId: null,
            source: "default"
          },
          runControlAuditSummary: createRunControlAuditSummary({
            action: {
              commandKind: null,
              boundary: null,
              resultKind: null,
              status: "missing",
              summary: "run-control audit summary unavailable",
              targetSummary: "target unavailable",
              evidenceId: null,
              observedAtMs: null,
              runId: null,
              turnId: null,
              checkpointTurnId: null,
              checkpointKind: null,
              recoveryMode: null,
              projectedCommand: null,
              startReason: null
            },
            currentContext: {
              phase: "ready",
              checkpointStatus: "completed",
              activeRunId: null,
              checkpointKind: "lifecycle_boundary",
              checkpointRecoveryMode: "persisted_effect",
              submissionPlanCommand: "start_graph_run_stream"
            }
          })
        });
      }

      throw new Error(`unexpected command: ${command}`);
    });

    store.$patch({
      sessionId: "submission-plan-default",
      phase: "idle",
      activeRunId: null,
      messages: []
    });

    await store.initializeSessions();

    expect(store.latestGraphRunSubmissionPlan).toEqual({
      command: "start_graph_run_stream",
      runId: null,
      source: "default"
    });
    expect(store.latestGraphRunControlBoundaryEvidence).toEqual([]);
    expect(store.activeRunId).toBeNull();
    expect(store.latestRunControlAuditSummary?.actionEvidenceSummary.status).toBe("missing");
    expect(store.latestRunControlAuditSummary?.actionEvidenceSummary.projectedCommand).toBeNull();
  });

  it("hydrates run-control audit summary from runtime view over snapshot fallback", async () => {
    const store = useRuntimeStore();
    const snapshot = createSnapshot({
      conversationId: "run-control-audit-session",
      summary: "run control summary",
      history: [{ role: "user", content: "resume me" }],
      turnCount: 1,
      updatedAtMs: 2700,
      runControlAuditSummary: createRunControlAuditSummary({
        action: {
          summary: "来自 snapshot 的旧 run-control summary"
        }
      })
    });
    const runtimeSummary = createRunControlAuditSummary({
      action: {
        commandKind: "resume_graph_run_stream",
        boundary: "run_resume",
        resultKind: "resumed",
        summary: "来自 runtime view 的最新 run-control summary",
        targetSummary: "恢复 run-runtime"
      },
      currentContext: {
        phase: "paused",
        checkpointStatus: "ready",
        activeRunId: "run-runtime",
        submissionPlanCommand: "resume_graph_run_stream"
      }
    });

    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "list_sessions") {
        return [] satisfies SessionOverview[];
      }

      if (command === "load_session_runtime_view") {
        return createSessionRuntimeView(snapshot, {
          runControlAuditSummary: runtimeSummary,
          submissionPlan: {
            command: "resume_graph_run_stream",
            runId: "run-runtime",
            source: "checkpoint"
          }
        });
      }

      throw new Error(`unexpected command: ${command}`);
    });

    store.$patch({
      sessionId: "run-control-audit-session",
      phase: "idle",
      messages: []
    });

    await store.initializeSessions();

    expect(store.latestRunControlAuditSummary).toEqual(runtimeSummary);
    expect(store.latestRunControlAuditSummary?.actionEvidenceSummary.summary).toBe(
      "来自 runtime view 的最新 run-control summary"
    );
  });

  it("prefers backend submission plan over stale local runState during submit", async () => {
    const store = useRuntimeStore();
    const nowSpy = vi.spyOn(Date, "now").mockReturnValue(8787);

    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string, payload?: Record<string, unknown>) => {
      if (command === "resolve_graph_run_submission_plan") {
        expect(payload).toEqual({
          sessionId: "backend-plan-session",
          runId: "run-stale-local",
          nodeId: null
        });
        return {
          command: "start_graph_run_stream",
          runId: null,
          source: "checkpoint"
        };
      }

      if (command === "start_graph_run_stream") {
        expect(payload).toEqual({
          turnId: "8787",
          runId: null,
          goal: "backend decides fresh start",
          input: expect.objectContaining({
            message: "backend decides fresh start"
          })
        });
        return {
          run: { id: "run-backend-plan" },
          turnId: "8787"
        };
      }

      throw new Error(`unexpected command: ${command}`);
    });

    const snapshot = createSnapshot({
      conversationId: "backend-plan-session",
      summary: "backend plan summary"
    });
    store.$patch({
      sessionId: "backend-plan-session",
      draftMessage: "backend decides fresh start",
      phase: "idle",
      messages: [],
      activeRunId: "run-stale-local",
      retrievedContext: {
        ...createRetrievedContext(snapshot),
        runState: {
          runId: "run-stale-local",
          phase: "paused",
          goal: "stale paused run"
        }
      }
    });

    const started = await store.submitTurn();

    expect(started).toBe(true);
    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledWith(
      "start_graph_run_stream",
      expect.objectContaining({
        turnId: "8787",
        runId: null
      })
    );
    expect(tauriMocks.mockSafeInvoke).not.toHaveBeenCalledWith(
      "resume_graph_run_stream",
      expect.anything()
    );
    nowSpy.mockRestore();
  });

  it("filters placeholder attachments out of replayed turn history", async () => {
    const store = useRuntimeStore();
    const nowSpy = vi.spyOn(Date, "now").mockReturnValue(8282);

    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string, payload?: Record<string, unknown>) => {
      if (command === "inspect_host") {
        return { runs: [] };
      }

      if (command === "resolve_graph_run_submission_plan") {
        return { command: "start_graph_run_stream" };
      }

      if (command === "load_retrieved_context") {
        return {
          turnContext: { userMessage: "", images: [], referencesImage: false },
          sessionContext: {
            conversationId: "history-attachment-session",
            title: "",
            summary: "",
            recentHistory: [],
            recentAttachmentAssets: [],
            turnCount: 0,
            lastReferencedFile: null
          },
          runState: {},
          longTermMemory: { status: "empty", summary: "", entries: [] },
          transcript: { providerNativeMessages: [] }
        };
      }

      if (command === "start_graph_run_stream") {
        return {
          run: { id: "run-history" },
          turnId: "8282"
        };
      }

      throw new Error(`unexpected command: ${command}`);
    });

    store.$patch({
      sessionId: "history-attachment-session",
      draftMessage: "new request",
      phase: "idle",
      messages: [
        createMessage({
          id: "history-user",
          turnId: "turn-history",
          role: "user",
          content: "look at these files",
          attachments: [
            {
              id: "pending-1",
              name: "pending.png",
              mimeType: "image/png",
              relativePath: null
            },
            {
              id: "real-1",
              name: "demo.png",
              mimeType: "image/png",
              relativePath: "uploads/demo.png"
            }
          ]
        }),
        createMessage({
          id: "history-assistant",
          turnId: "turn-history",
          role: "assistant",
          content: "ready for the next step"
        })
      ]
    });

    const started = await store.submitTurn();

    expect(started).toBe(true);
    nowSpy.mockRestore();
  });

  it("marks the turn as failed when start_graph_run_stream throws immediately", async () => {
    const store = useRuntimeStore();
    const nowSpy = vi.spyOn(Date, "now").mockReturnValue(9090);

    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "inspect_host") {
        return { runs: [] };
      }

      if (command === "start_graph_run_stream") {
        throw new Error("stream bootstrap failed");
      }

      throw new Error(`unexpected command: ${command}`);
    });

    store.$patch({
      sessionId: "tauri-session",
      draftMessage: "tauri request",
      phase: "idle",
      messages: []
    });

    await store.submitTurn();

    expect(store.phase).toBe("failed");
    expect(store.isSubmitting).toBe(false);
    expect(store.activeTurnId).toBeNull();
    expect(store.error).toContain("stream bootstrap failed");
    expect(store.messages).toHaveLength(2);
    expect(store.messages[0]?.role).toBe("user");
    expect(store.messages[1]?.role).toBe("assistant");
    expect(store.messages[1]?.status).toBe("error");
    expect(store.turnTraceHistory).toHaveLength(1);
    expect(store.turnTraceHistory[0]?.phase).toBe("failed");
    expect(store.turnTraceHistory[0]?.error).toContain("stream bootstrap failed");

    nowSpy.mockRestore();
  });

  it("applies started, delta and completed stream events to the active turn", async () => {
    const store = useRuntimeStore();
    const nowSpy = vi.spyOn(Date, "now").mockReturnValue(6060);
    const eventHandlers = new Map<string, (event: { payload: Record<string, unknown> }) => void>();
    const startedObservation = createBuildContextObservation();
    const completedObservation = createBuildContextObservation({
      requestFormat: "response_format=json_schema",
      messageCount: 4,
      toolCount: 2,
      requestMessagesText: "system: summarize retrieval\nuser: stream request\nassistant: partial answer",
      toolDefinitionsText: "workspace.read_file(path: string)\nworkspace.search(query: string)"
    });

    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeListen.mockImplementation(async (eventName: string, handler: unknown) => {
      eventHandlers.set(eventName, handler as (event: { payload: Record<string, unknown> }) => void);
      return () => {};
    });
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "inspect_host") {
        return { runs: [] };
      }

      if (command === "start_graph_run_stream") {
        return {
          run: { id: "run-stream" },
          turnId: "6060"
        };
      }

      if (command === "list_sessions") {
        return [] satisfies SessionOverview[];
      }

      throw new Error(`unexpected command: ${command}`);
    });

    store.$patch({
      sessionId: "stream-session",
      draftMessage: "stream request",
      phase: "idle",
      messages: []
    });

    await store.submitTurn();

    expect(store.phase).toBe("calling_model");
    expect(store.activeTurnId).toBe("6060");
    expect(store.isSubmitting).toBe(true);
    expect(store.turnTraceHistory).toHaveLength(1);
    expect(store.turnTraceHistory[0]?.turnId).toBe("6060");
    expect(store.turnTraceHistory[0]?.phase).toBe("calling_model");

    store.$patch({
      streamDebugDeltaCount: 42,
      streamDebugFlushCount: 9,
      streamDebugTextCharsReceived: 120,
      streamDebugTextCharsFlushed: 100
    });

    eventHandlers.get("turn:started")?.({
      payload: {
        turnId: "6060",
        eventId: "event-turn-started-6060",
        eventType: "turn.model_call_started",
        eventVersion: "1.0",
        sequence: 3,
        emittedAtMs: 1200,
        providerName: "OpenAI",
        providerModel: "gpt-5",
        providerProtocol: "openai",
        providerSource: "primary",
        providerMode: "standard",
        providerRequestedName: "OpenAI",
        inputTokens: 12,
        traceSteps: store.traceSteps,
        buildContextObservation: startedObservation
      }
    });

    expect(store.streamDebugDeltaCount).toBe(0);
    expect(store.streamDebugFlushCount).toBe(0);
    expect(store.streamDebugTextCharsReceived).toBe(0);
    expect(store.streamDebugTextCharsFlushed).toBe(0);

    expect(store.traceTimeline.find((entry) => entry.kind === "build_context")?.buildContextObservation).toBeNull();
    expect(store.traceTimeline.map((entry) => entry.kind)).toEqual([
      "input",
      "prepare_retrieval",
      "build_context",
      "call_model"
    ]);

    eventHandlers.get("turn:delta")?.({
      payload: {
        turnId: "6060",
        eventId: "event-turn-delta-6060",
        eventType: "turn.first_token",
        eventVersion: "1.0",
        sequence: 4,
        emittedAtMs: 1521,
        text: "partial answer",
        reasoningContent: "thinking",
        firstTokenLatencyMs: 321
      }
    });

    await new Promise((resolve) => window.setTimeout(resolve, 50));

    expect(store.firstTokenLatencyMs).toBe(321);
    expect(store.messages.find((message) => message.role === "assistant")?.content).toBe("partial answer");
    expect(
      store.traceTimeline.find((entry) => entry.kind === "call_model" && entry.text)?.text
    ).toBeUndefined();

    eventHandlers.get("turn:phase_changed")?.({
      payload: {
        turnId: "6060",
        eventId: "event-turn-phase-changed-6060",
        eventType: "turn.phase_changed",
        eventVersion: "1.0",
        sequence: 5,
        emittedAtMs: 2500,
        phase: "checkpointing"
      }
    });

    expect(store.phase).toBe("connecting");
    expect(store.eventCursorByTurnId["6060"]?.eventId).toBe("event-turn-phase-changed-6060");
    expect(store.eventCursorByTurnId["6060"]?.sequence).toBe(5);
    expect(
      store.traceTimeline.find((entry) => entry.kind === "call_model" && entry.text)?.text
    ).toBe("partial answer");

    eventHandlers.get("turn:checkpoint_persisted")?.({
      payload: {
        turnId: "6060",
        eventId: "event-turn-checkpoint-6060",
        eventType: "turn.checkpoint_persisted",
        eventVersion: "1.0",
        sequence: 6,
        emittedAtMs: 2600,
        phase: "checkpointing"
      }
    });

    expect(store.phase).toBe("connecting");
    expect(store.eventCursorByTurnId["6060"]?.eventId).toBe("event-turn-checkpoint-6060");
    expect(store.eventCursorByTurnId["6060"]?.sequence).toBe(6);

    eventHandlers.get("turn:completed")?.({
      payload: {
        turnId: "6060",
        eventId: "event-turn-completed-6060",
        eventType: "turn.completed",
        eventVersion: "1.0",
        sequence: 7,
        emittedAtMs: 2800,
        text: "final answer",
        reasoningContent: "thinking done",
        providerName: "OpenAI",
        providerModel: "gpt-5",
        providerProtocol: "openai",
        providerSource: "primary",
        providerMode: "standard",
        providerRequestedName: "OpenAI",
        sessionSummary: "Completed summary",
        inputTokens: 12,
        outputTokens: 34,
        totalTokens: 46,
        cache_hit_input_tokens: 5,
        inputTokensDetails: {
          cachedTokens: 3
        },
        completionTokensDetails: {
          reasoningTokens: 8
        },
        providerCallRecords: [
          createProviderCallRecord({
            requestKind: "initial_request",
            cacheHitInputTokens: 5
          })
        ],
        firstTokenLatencyMs: 321,
        turnDurationMs: 2800,
        traceSteps: store.traceSteps,
        toolActivities: [],
        buildContextObservation: completedObservation
      }
    } as any);

    await flushDeferredTurnWork();
    await flushMicrotasks();

    expect(store.phase).toBe("ready");
    expect(store.isSubmitting).toBe(false);
    expect(store.activeTurnId).toBeNull();
    expect(store.sessionSummary).toBe("Completed summary");
    expect(store.providerName).toBe("OpenAI");
    expect(store.providerModel).toBe("gpt-5");
    expect(store.firstTokenLatencyMs).toBe(321);
    expect(store.totalTokens).toBe(46);
    expect(store.messages).toHaveLength(2);
    expect(store.messages[1]?.content).toBe("final answer");
    expect(store.messages[1]?.reasoningContent).toBe("thinking done");
    expect(store.messages[1]?.status).toBe("done");
    expect(store.turnTraceHistory).toHaveLength(1);
    expect(store.turnTraceHistory[0]?.phase).toBe("completed");
    expect(store.turnTraceHistory[0]?.sessionSummary).toBe("Completed summary");
    expect(store.turnTraceHistory[0]?.title).toBe("stream request");
    expect(store.turnTraceHistory[0]?.eventId).toBe("event-turn-completed-6060");
    expect(store.turnTraceHistory[0]?.eventType).toBe("turn.completed");
    expect(store.turnTraceHistory[0]?.eventVersion).toBe("1.0");
    expect(store.turnTraceHistory[0]?.sequence).toBe(7);
    expect(store.turnTraceHistory[0]?.emittedAtMs).toBe(2800);
    expect(store.turnTraceHistory[0]?.buildContextObservation).toEqual(completedObservation);
    expect(store.turnTraceHistory[0]?.buildContextObservation).not.toBe(completedObservation);
    expect(store.turnTraceHistory[0]?.cacheHitInputTokens).toBe(5);
    expect(store.turnTraceHistory[0]?.reasoningTokens).toBe(8);
    expect(store.turnTraceHistory[0]?.turnDurationMs).toBe(2800);
    expect(store.turnTraceHistory[0]?.traceTimeline?.at(-1)?.label).toBe("CALL MODEL #1");
    expect(store.turnTraceHistory[0]?.traceTimeline?.at(-1)?.outputTokens).toBe(34);

    nowSpy.mockRestore();
  });

  it("shows finalized output before terminal trace work completes", async () => {
    const nowSpy = vi.spyOn(Date, "now").mockReturnValue(8080);
    const eventHandlers = new Map<string, (event: { payload: Record<string, unknown> }) => void>();
    tauriMocks.mockSafeListen.mockImplementation(async (eventName: string, handler: unknown) => {
      eventHandlers.set(eventName, handler as (event: { payload: Record<string, unknown> }) => void);
      return () => {};
    });
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "inspect_host") {
        return { runs: [] };
      }

      if (command === "resolve_graph_run_submission_plan") {
        return { command: "start_graph_run_stream", runId: null };
      }

      if (command === "start_graph_run_stream") {
        return {
          run: { id: "run-output-end" },
          turnId: "8080"
        };
      }

      if (command === "list_sessions") {
        return [] satisfies SessionOverview[];
      }

      throw new Error(`unexpected command: ${command}`);
    });

    const store = useRuntimeStore();
    store.$patch({
      sessionId: "session-output-end",
      draftMessage: "stream request",
      phase: "idle",
      messages: [],
      turnTraceHistory: []
    });

    await store.submitTurn();

    eventHandlers.get("turn:delta")?.({
      payload: {
        turnId: "8080",
        eventId: "event-turn-delta-8080",
        eventType: "turn.output_delta",
        sequence: 1,
        emittedAtMs: 1000,
        text: "partial "
      }
    });

    await new Promise((resolve) => window.setTimeout(resolve, 50));

    eventHandlers.get("turn:output_end")?.({
      payload: {
        turnId: "8080",
        eventId: "event-turn-output-end-8080",
        eventType: "turn.output_end",
        eventVersion: "1.0",
        sequence: 2,
        emittedAtMs: 1200,
        text: "final answer",
        reasoningContent: "done thinking",
        providerName: "OpenAI",
        providerModel: "gpt-5",
        providerProtocol: "openai",
        providerSource: "primary",
        providerMode: "standard",
        outputTokens: 9
      }
    });

    await flushMicrotasks();

    const assistant = store.messages.find((message) => message.role === "assistant");
    expect(assistant?.content).toBe("final answer");
    expect(assistant?.reasoningContent).toBe("done thinking");
    expect(assistant?.status).toBe("done");
    expect(assistant?.tokenCount).toBe(9);
    expect(store.activeTurnId).toBe("8080");
    expect(store.isSubmitting).toBe(true);
    expect(store.turnTraceHistory).toHaveLength(1);
    expect(store.turnTraceHistory[0]?.phase).toBe("calling_model");
    expect(store.turnTraceHistory[0]?.eventType).not.toBe("turn.completed");

    eventHandlers.get("turn:completed")?.({
      payload: {
        turnId: "8080",
        eventId: "event-turn-completed-8080",
        eventType: "turn.completed",
        eventVersion: "1.0",
        sequence: 3,
        emittedAtMs: 1500,
        text: "final answer",
        providerName: "OpenAI",
        providerModel: "gpt-5",
        providerProtocol: "openai",
        providerSource: "primary",
        providerMode: "standard",
        outputTokens: 9,
        traceSteps: store.traceSteps,
        toolActivities: []
      }
    });

    await flushDeferredTurnWork();
    await flushMicrotasks();

    expect(store.activeTurnId).toBeNull();
    expect(store.isSubmitting).toBe(false);
    expect(store.turnTraceHistory).toHaveLength(1);
    expect(store.turnTraceHistory[0]?.eventType).toBe("turn.completed");
    nowSpy.mockRestore();
  });

  it("keeps terminal assistant payload stable when output_end and completed carry the same content", async () => {
    const nowSpy = vi.spyOn(Date, "now").mockReturnValue(9090);
    const eventHandlers = new Map<string, (event: { payload: Record<string, unknown> }) => void>();
    tauriMocks.mockSafeListen.mockImplementation(async (eventName: string, handler: unknown) => {
      eventHandlers.set(eventName, handler as (event: { payload: Record<string, unknown> }) => void);
      return () => {};
    });
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "inspect_host") {
        return { runs: [] };
      }

      if (command === "resolve_graph_run_submission_plan") {
        return { command: "start_graph_run_stream", runId: null };
      }

      if (command === "start_graph_run_stream") {
        return {
          run: { id: "run-stable-terminal" },
          turnId: "9090"
        };
      }

      if (command === "list_sessions") {
        return [] satisfies SessionOverview[];
      }

      throw new Error(`unexpected command: ${command}`);
    });

    const store = useRuntimeStore();
    store.$patch({
      sessionId: "session-stable-terminal",
      draftMessage: "stream request",
      phase: "idle",
      messages: [],
      turnTraceHistory: []
    });

    await store.submitTurn();

    eventHandlers.get("turn:output_end")?.({
      payload: {
        turnId: "9090",
        eventId: "event-turn-output-end-9090",
        eventType: "turn.output_end",
        eventVersion: "1.0",
        sequence: 2,
        emittedAtMs: 1200,
        text: "final answer",
        reasoningContent: "stable reasoning",
        providerName: "OpenAI",
        providerModel: "gpt-5",
        providerProtocol: "openai",
        providerSource: "primary",
        providerMode: "standard",
        outputTokens: 9
      }
    });

    await flushMicrotasks();

    eventHandlers.get("turn:completed")?.({
      payload: {
        turnId: "9090",
        eventId: "event-turn-completed-9090",
        eventType: "turn.completed",
        eventVersion: "1.0",
        sequence: 3,
        emittedAtMs: 1500,
        text: "final answer",
        reasoningContent: "stable reasoning",
        providerName: "OpenAI",
        providerModel: "gpt-5",
        providerProtocol: "openai",
        providerSource: "primary",
        providerMode: "standard",
        outputTokens: 9,
        traceSteps: store.traceSteps,
        toolActivities: []
      }
    });

    await flushDeferredTurnWork();
    await flushMicrotasks();

    const assistant = store.messages.find((message) => message.role === "assistant");
    expect(assistant?.content).toBe("final answer");
    expect(assistant?.reasoningContent).toBe("stable reasoning");
    expect(assistant?.status).toBe("done");
    nowSpy.mockRestore();
  });

  it("defers persistence for delta stream updates instead of flushing every chunk immediately", async () => {
    const store = useRuntimeStore();
    const nowSpy = vi.spyOn(Date, "now").mockReturnValue(7070);
    const eventHandlers = new Map<string, (event: { payload: Record<string, unknown> }) => void>();

    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeListen.mockImplementation(async (eventName: string, handler: unknown) => {
      eventHandlers.set(eventName, handler as (event: { payload: Record<string, unknown> }) => void);
      return () => {};
    });
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "inspect_host") {
        return { runs: [] };
      }

      if (command === "start_graph_run_stream") {
        return {
          run: { id: "run-stream" },
          turnId: "7070"
        };
      }

      if (command === "list_sessions") {
        return [] satisfies SessionOverview[];
      }

      throw new Error(`unexpected command: ${command}`);
    });

    store.$patch({
      sessionId: "stream-session-deferred-persist",
      draftMessage: "stream request",
      phase: "idle",
      messages: []
    });

    await store.submitTurn();

    eventHandlers.get("turn:started")?.({
      payload: {
        turnId: "7070",
        eventId: "event-turn-started-7070",
        eventType: "turn.model_call_started",
        eventVersion: "1.0",
        sequence: 1,
        emittedAtMs: 1000,
        providerName: "OpenAI",
        providerModel: "gpt-5",
        providerProtocol: "openai",
        providerSource: "primary",
        providerMode: "standard",
        providerRequestedName: "OpenAI",
        traceSteps: store.traceSteps
      }
    });

    await new Promise((resolve) => setTimeout(resolve, 180));

    let persisted = readPersistedSessions().sessions["stream-session-deferred-persist"] as {
      messages: ChatMessage[];
    };
    expect(persisted.messages.find((message) => message.role === "assistant")?.content).toBe("");

    eventHandlers.get("turn:delta")?.({
      payload: {
        turnId: "7070",
        eventId: "event-turn-delta-7070",
        eventType: "turn.first_token",
        eventVersion: "1.0",
        sequence: 2,
        emittedAtMs: 1200,
        text: "partial answer",
        reasoningContent: "thinking"
      }
    });

    persisted = readPersistedSessions().sessions["stream-session-deferred-persist"] as {
      messages: ChatMessage[];
    };
    expect(persisted.messages.find((message) => message.role === "assistant")?.content).toBe("");

    await new Promise((resolve) => setTimeout(resolve, 180));

    persisted = readPersistedSessions().sessions["stream-session-deferred-persist"] as {
      messages: ChatMessage[];
    };
    expect(persisted.messages.find((message) => message.role === "assistant")?.content).toBe("");
    expect(persisted.messages.find((message) => message.role === "assistant")?.reasoningContent ?? null).toBeNull();

    nowSpy.mockRestore();
  }, 10000);

  it("prefers canonical event metadata over legacy phase guessing while streaming", async () => {
    const store = useRuntimeStore();
    const nowSpy = vi.spyOn(Date, "now").mockReturnValue(7272);
    const eventHandlers = new Map<string, (event: { payload: Record<string, unknown> }) => void>();

    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeListen.mockImplementation(async (eventName: string, handler: unknown) => {
      eventHandlers.set(eventName, handler as (event: { payload: Record<string, unknown> }) => void);
      return () => {};
    });
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "inspect_host") {
        return { runs: [] };
      }

      if (command === "start_graph_run_stream") {
        return {
          run: { id: "run-stream-canonical-phase" },
          turnId: "7272"
        };
      }

      if (command === "list_sessions") {
        return [] satisfies SessionOverview[];
      }

      throw new Error(`unexpected command: ${command}`);
    });

    store.$patch({
      sessionId: "canonical-phase-session",
      draftMessage: "canonical phase request",
      phase: "idle",
      messages: []
    });

    await store.submitTurn();

    eventHandlers.get("turn:started")?.({
      payload: {
        turnId: "7272",
        eventType: "turn.created",
        phase: "building_context",
        providerName: "OpenAI",
        providerModel: "gpt-5"
      }
    });
    expect(store.phase).toBe("connecting");
    expect(store.traceTimeline.map((entry) => `${entry.kind}:${entry.state}`)).toEqual([
      "input:completed",
      "build_context:active",
      "call_model:pending"
    ]);

    eventHandlers.get("turn:trace")?.({
      payload: {
        turnId: "7272",
        eventType: "turn.model_call_started"
      }
    });
    expect(store.phase).toBe("calling_model");
    expect(store.traceTimeline.at(-1)?.kind).toBe("call_model");
    expect(store.traceTimeline.at(-1)?.state).toBe("active");

    eventHandlers.get("turn:tool")?.({
      payload: {
        turnId: "7272",
        eventType: "turn.tool_call_started",
        phase: "executing_tool",
        toolActivities: [
          {
            id: "tool-workspace-read-file",
            name: "workspace.read_file",
            status: "running",
            summary: "read file running"
          }
        ]
      }
    });
    expect(store.phase).toBe("calling_tool");
    expect(store.traceTimeline.at(-1)?.kind).toBe("call_tool");
    expect(store.traceTimeline.at(-1)?.state).toBe("active");

    eventHandlers.get("turn:tool")?.({
      payload: {
        turnId: "7272",
        eventType: "turn.tool_call_completed",
        phase: "tool_result_integrating",
        toolActivities: [
          {
            id: "tool-workspace-read-file",
            name: "workspace.read_file",
            status: "done",
            summary: "read file done"
          }
        ]
      }
    });
    expect(store.phase).toBe("calling_model");
    expect(store.traceTimeline.at(-2)?.kind).toBe("call_tool");
    expect(store.traceTimeline.at(-2)?.state).toBe("completed");
    expect(store.traceTimeline.at(-1)?.kind).toBe("call_model");
    expect(store.traceTimeline.at(-1)?.state).toBe("active");

    nowSpy.mockRestore();
  });

  it("returns the reactive assistant message instance for browser-preview updates", () => {
    const store = useRuntimeStore();

    const assistantMessage = store.ensureAssistantMessage("turn-reactive", "ppx/gpt-5.4");

    expect(store.messages).toHaveLength(1);
    expect(assistantMessage).toBe(store.messages[0]);
  });

  it("ignores duplicate or older-sequence stream events once a newer canonical event was accepted", async () => {
    const store = useRuntimeStore();
    const nowSpy = vi.spyOn(Date, "now").mockReturnValue(7373);
    const eventHandlers = new Map<string, (event: { payload: Record<string, unknown> }) => void>();

    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeListen.mockImplementation(async (eventName: string, handler: unknown) => {
      eventHandlers.set(eventName, handler as (event: { payload: Record<string, unknown> }) => void);
      return () => {};
    });
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "inspect_host") {
        return { runs: [] };
      }

      if (command === "start_graph_run_stream") {
        return {
          run: { id: "run-stream-dedup" },
          turnId: "7373"
        };
      }

      if (command === "list_sessions") {
        return [] satisfies SessionOverview[];
      }

      throw new Error(`unexpected command: ${command}`);
    });

    store.$patch({
      sessionId: "stream-dedup-session",
      draftMessage: "dedup request",
      phase: "idle",
      messages: []
    });

    await store.submitTurn();

    eventHandlers.get("turn:started")?.({
      payload: {
        turnId: "7373",
        eventId: "event-turn-started-7373",
        eventType: "turn.model_call_started",
        eventVersion: "1.0",
        sequence: 3,
        emittedAtMs: 1200,
        providerName: "OpenAI",
        providerModel: "gpt-5"
      }
    });

    eventHandlers.get("turn:tool")?.({
      payload: {
        turnId: "7373",
        eventId: "event-turn-tool-7373",
        eventType: "turn.tool_call_completed",
        eventVersion: "1.0",
        sequence: 6,
        emittedAtMs: 1800,
        toolActivities: [
          {
            id: "tool-workspace-read-file",
            name: "workspace.read_file",
            status: "done",
            summary: "read file done"
          }
        ]
      }
    });

    const assistantBeforeStaleDelta = store.messages.find(
      (message) => message.turnId === "7373" && message.role === "assistant"
    )?.content;

    eventHandlers.get("turn:delta")?.({
      payload: {
        turnId: "7373",
        eventId: "event-turn-delta-7373-stale",
        eventType: "turn.first_token",
        eventVersion: "1.0",
        sequence: 4,
        emittedAtMs: 1500,
        text: "stale delta should be ignored",
        reasoningContent: "stale reasoning"
      }
    });

    expect(
      store.messages.find((message) => message.turnId === "7373" && message.role === "assistant")?.content
    ).toBe(assistantBeforeStaleDelta);
    expect(store.eventCursorByTurnId["7373"]?.eventId).toBe("event-turn-tool-7373");
    expect(store.eventCursorByTurnId["7373"]?.sequence).toBe(6);

    eventHandlers.get("turn:tool")?.({
      payload: {
        turnId: "7373",
        eventId: "event-turn-tool-7373",
        eventType: "turn.tool_call_completed",
        eventVersion: "1.0",
        sequence: 6,
        emittedAtMs: 1800,
        toolActivities: [
          {
            id: "tool-workspace-read-file",
            name: "workspace.read_file",
            status: "done",
            summary: "duplicate should be ignored"
          }
        ]
      }
    });

    expect(store.toolActivities[0]?.summary).toBe("read file done");
    expect(store.eventCursorByTurnId["7373"]?.eventId).toBe("event-turn-tool-7373");
    expect(store.eventCursorByTurnId["7373"]?.sequence).toBe(6);

    nowSpy.mockRestore();
  });

  it("records model and tool hops in timeline order without merging", async () => {
    const store = useRuntimeStore();
    const nowSpy = vi.spyOn(Date, "now").mockReturnValue(7070);
    const eventHandlers = new Map<string, (event: { payload: Record<string, unknown> }) => void>();

    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeListen.mockImplementation(async (eventName: string, handler: unknown) => {
      eventHandlers.set(eventName, handler as (event: { payload: Record<string, unknown> }) => void);
      return () => {};
    });
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "inspect_host") {
        return { runs: [] };
      }

      if (command === "start_graph_run_stream") {
        return {
          run: { id: "run-stream-hops" },
          turnId: "7070"
        };
      }

      if (command === "list_sessions") {
        return [] satisfies SessionOverview[];
      }

      throw new Error(`unexpected command: ${command}`);
    });

    store.$patch({
      sessionId: "stream-session",
      draftMessage: "trace hops",
      phase: "idle",
      messages: []
    });

    await store.submitTurn();

    eventHandlers.get("turn:started")?.({
      payload: {
        turnId: "7070",
        providerName: "OpenAI",
        providerModel: "gpt-5",
        providerProtocol: "openai",
        providerSource: "primary",
        providerMode: "standard",
        traceSteps: store.traceSteps,
        buildContextObservation: createBuildContextObservation()
      }
    });

    eventHandlers.get("turn:trace")?.({
      payload: {
        turnId: "7070",
        phase: "calling_tool",
        traceSteps: store.traceSteps
      }
    });

    eventHandlers.get("turn:tool")?.({
      payload: {
        turnId: "7070",
        phase: "calling_tool",
        toolActivities: [
          {
            id: "tool-workspace-list-files",
            name: "workspace.list_files",
            status: "running",
            summary: "list files running",
            argumentsText: "{\"path\":\".\"}",
            resultText: null,
            durationSeconds: null
          }
        ]
      }
    });

    eventHandlers.get("turn:tool")?.({
      payload: {
        turnId: "7070",
        phase: "calling_model",
        toolActivities: [
          {
            id: "tool-workspace-list-files",
            name: "workspace.list_files",
            status: "done",
            summary: "list files done",
            argumentsText: "{\"path\":\".\"}",
            resultText: "{\"entries\":[\"src\"]}",
            durationSeconds: 0.2
          }
        ]
      }
    });

    eventHandlers.get("turn:trace")?.({
      payload: {
        turnId: "7070",
        phase: "calling_model",
        traceSteps: store.traceSteps
      }
    });

    eventHandlers.get("turn:delta")?.({
      payload: {
        turnId: "7070",
        text: "second model answer",
        firstTokenLatencyMs: 123
      }
    });

    eventHandlers.get("turn:completed")?.({
      payload: {
        turnId: "7070",
        text: "final answer",
        providerName: "OpenAI",
        providerModel: "gpt-5",
        providerProtocol: "openai",
        providerSource: "primary",
        providerMode: "standard",
        inputTokens: 20,
        outputTokens: 10,
        totalTokens: 30,
        firstTokenLatencyMs: 123,
        turnDurationMs: 1000,
        traceSteps: store.traceSteps,
        toolActivities: []
      }
    } as any);

    await flushDeferredTurnWork();
    await flushMicrotasks();

    expect(store.turnTraceHistory[0]?.traceTimeline?.map((entry) => entry.kind)).toEqual([
      "input",
      "build_context",
      "call_model",
      "call_tool",
      "call_model"
    ]);
    nowSpy.mockRestore();
  });

  it("prefers backend-provided trace timeline over local timeline assembly", async () => {
    const store = useRuntimeStore();
    const nowSpy = vi.spyOn(Date, "now").mockReturnValue(7171);
    const eventHandlers = new Map<string, (event: { payload: Record<string, unknown> }) => void>();

    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeListen.mockImplementation(async (eventName: string, handler: unknown) => {
      eventHandlers.set(eventName, handler as (event: { payload: Record<string, unknown> }) => void);
      return () => {};
    });
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "inspect_host") {
        return { runs: [] };
      }

      if (command === "start_graph_run_stream") {
        return {
          run: { id: "run-stream-backend-timeline" },
          turnId: "7171"
        };
      }

      if (command === "list_sessions") {
        return [] satisfies SessionOverview[];
      }

      throw new Error(`unexpected command: ${command}`);
    });

    store.$patch({
      sessionId: "stream-session",
      draftMessage: "backend timeline",
      phase: "idle",
      messages: []
    });

    await store.submitTurn();

    eventHandlers.get("turn:started")?.({
      payload: {
        turnId: "7171",
        providerName: "OpenAI",
        providerModel: "gpt-5",
        providerProtocol: "openai",
        providerSource: "primary",
        providerMode: "standard",
        traceSteps: store.traceSteps,
        buildContextObservation: createBuildContextObservation(),
        traceTimeline: [
          { id: "input-1", kind: "input", label: "RECEIVE INPUT", state: "completed", sequence: 1, text: "backend timeline" },
          { id: "retrieval-2", kind: "prepare_retrieval", label: "PREPARE RETRIEVAL", state: "completed", sequence: 2 },
          { id: "context-3", kind: "build_context", label: "BUILD CONTEXT", state: "completed", sequence: 3 },
          { id: "model-4", kind: "call_model", label: "CALL MODEL #1", state: "active", sequence: 4 }
        ]
      }
    });

    eventHandlers.get("turn:tool")?.({
      payload: {
        turnId: "7171",
        phase: "calling_tool",
        toolActivities: [
          {
            id: "tool-workspace-read-file",
            name: "workspace.read_file",
            status: "running",
            summary: "read file running",
            argumentsText: "{\"path\":\"src/main.ts\"}",
            resultText: null,
            durationSeconds: null
          }
        ],
        traceTimeline: [
          { id: "input-1", kind: "input", label: "RECEIVE INPUT", state: "completed", sequence: 1, text: "backend timeline" },
          { id: "retrieval-2", kind: "prepare_retrieval", label: "PREPARE RETRIEVAL", state: "completed", sequence: 2 },
          { id: "context-3", kind: "build_context", label: "BUILD CONTEXT", state: "completed", sequence: 3 },
          { id: "model-4", kind: "call_model", label: "CALL MODEL #1", state: "completed", sequence: 4 },
          { id: "tool-5", kind: "call_tool", label: "CALL TOOL #1 · workspace.read_file", state: "active", sequence: 5, text: "read file running", toolActivities: [
            {
              id: "tool-workspace-read-file",
              name: "workspace.read_file",
              status: "running",
              summary: "read file running",
              argumentsText: "{\"path\":\"src/main.ts\"}",
              resultText: null,
              durationSeconds: null
            }
          ] }
        ]
      }
    });

    eventHandlers.get("turn:completed")?.({
      payload: {
        turnId: "7171",
        text: "backend final answer",
        providerName: "OpenAI",
        providerModel: "gpt-5",
        providerProtocol: "openai",
        providerSource: "primary",
        providerMode: "standard",
        inputTokens: 11,
        cacheHitInputTokens: 5,
        outputTokens: 7,
        totalTokens: 18,
        firstTokenLatencyMs: 99,
        turnDurationMs: 900,
        traceSteps: store.traceSteps,
        toolActivities: [],
        traceTimeline: [
          { id: "input-1", kind: "input", label: "RECEIVE INPUT", state: "completed", sequence: 1, text: "backend timeline" },
          { id: "retrieval-2", kind: "prepare_retrieval", label: "PREPARE RETRIEVAL", state: "completed", sequence: 2 },
          { id: "context-3", kind: "build_context", label: "BUILD CONTEXT", state: "completed", sequence: 3 },
          { id: "model-4", kind: "call_model", label: "CALL MODEL #1", state: "completed", sequence: 4 },
          { id: "tool-5", kind: "call_tool", label: "CALL TOOL #1 · workspace.read_file", state: "completed", sequence: 5, text: "read file done", toolActivities: [] },
          { id: "model-6", kind: "call_model", label: "CALL MODEL #2", state: "completed", sequence: 6, text: "backend final answer", firstTokenLatencyMs: 99 },
          { id: "return-7", kind: "return_result", label: "RETURN RESULT", state: "completed", sequence: 7, text: "backend final answer", cacheHitInputTokens: 5, outputTokens: 7, totalTokens: 18, firstTokenLatencyMs: 99, turnDurationMs: 900 }
        ]
      }
    } as any);

    await flushDeferredTurnWork();
    await flushMicrotasks();

    expect(store.turnTraceHistory[0]?.traceTimeline?.map((entry) => entry.kind)).toEqual([
      "input",
      "prepare_retrieval",
      "build_context",
      "call_model",
      "call_tool",
      "call_model"
    ]);
    expect(store.turnTraceHistory[0]?.traceTimeline?.[3]?.text).toBeUndefined();
    expect(store.turnTraceHistory[0]?.traceTimeline?.[4]?.label).toContain("workspace.read_file");
    expect(store.turnTraceHistory[0]?.traceTimeline?.[5]?.text).toBe("backend final answer");
    expect(store.turnTraceHistory[0]?.traceTimeline?.[5]?.cacheHitInputTokens).toBe(5);
    expect(store.turnTraceHistory[0]?.traceTimeline?.[5]?.turnDurationMs).toBe(900);
    nowSpy.mockRestore();
  });

  it("ignores non-provider cache-hit candidates on completion", async () => {
    const store = useRuntimeStore();
    const nowSpy = vi.spyOn(Date, "now").mockReturnValue(6262);
    const eventHandlers = new Map<string, (event: { payload: Record<string, unknown> }) => void>();

    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeListen.mockImplementation(async (eventName: string, handler: unknown) => {
      eventHandlers.set(eventName, handler as (event: { payload: Record<string, unknown> }) => void);
      return () => {};
    });
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "inspect_host") {
        return { runs: [] };
      }

      if (command === "start_graph_run_stream") {
        return {
          run: { id: "run-stream-accumulated-cache" },
          turnId: "6262"
        };
      }

      if (command === "list_sessions") {
        return [] satisfies SessionOverview[];
      }

      throw new Error(`unexpected command: ${command}`);
    });

    store.$patch({
      sessionId: "stream-session-accumulated-cache",
      draftMessage: "trace accumulated cache hit",
      phase: "idle",
      messages: []
    });

    await store.submitTurn();
    expect(store.activeTurnId).toBe("6262");

    eventHandlers.get("turn:completed")?.({
      payload: {
        turnId: "6262",
        text: "final answer",
        providerName: "OpenAI",
        providerModel: "gpt-5",
        providerProtocol: "openai",
        providerSource: "provider_followup_stream",
        providerMode: "standard",
        providerRequestedName: "OpenAI",
        sessionSummary: "Completed summary",
        inputTokens: 240,
        outputTokens: 70,
        totalTokens: 310,
        cacheHitInputTokens: 70,
        inputTokensDetails: {
          cachedTokens: 15
        },
        traceSteps: store.traceSteps,
        toolActivities: [],
        turnDurationMs: 2800
      }
    } as any);

    await flushDeferredTurnWork();
    await flushMicrotasks();

    expect(store.turnTraceHistory).toHaveLength(1);
    expect(store.turnTraceHistory[0]?.cacheHitInputTokens).toBeNull();
    expect(store.turnTraceHistory[0]?.inputTokens).toBe(240);
    expect(store.turnTraceHistory[0]?.outputTokens).toBe(70);
    expect(store.turnTraceHistory[0]?.totalTokens).toBe(310);

    nowSpy.mockRestore();
  });

  it("stores call-level model hops with request kinds from host payload", async () => {
    const store = useRuntimeStore();
    const nowSpy = vi.spyOn(Date, "now").mockReturnValue(6363);
    const eventHandlers = new Map<string, (event: { payload: Record<string, unknown> }) => void>();

    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeListen.mockImplementation(async (eventName: string, handler: unknown) => {
      eventHandlers.set(eventName, handler as (event: { payload: Record<string, unknown> }) => void);
      return () => {};
    });
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "inspect_host") {
        return { runs: [] };
      }
      if (command === "start_graph_run_stream") {
        return { run: { id: "run-stream-provider-calls" }, turnId: "6363" };
      }
      if (command === "list_sessions") {
        return [] satisfies SessionOverview[];
      }
      throw new Error(`unexpected command: ${command}`);
    });

    store.$patch({
      sessionId: "stream-session-provider-calls",
      draftMessage: "provider call records",
      phase: "idle",
      messages: []
    });

    await store.submitTurn();
    eventHandlers.get("turn:completed")?.({
      payload: {
        turnId: "6363",
        text: "final answer",
        providerName: "OpenAI",
        providerModel: "gpt-5",
        providerProtocol: "openai",
        providerSource: "provider_followup_stream",
        providerMode: "standard",
        providerRequestedName: "OpenAI",
        sessionSummary: "Completed summary",
        inputTokens: 240,
        outputTokens: 70,
        totalTokens: 310,
        cacheHitInputTokens: 70,
        traceSteps: store.traceSteps,
        toolActivities: [],
        providerCallRecords: [
          createProviderCallRecord({ requestKind: "initial_request" }),
          createProviderCallRecord({
            requestKind: "tool_followup",
            providerSource: "provider_followup_stream",
            cacheHitInputTokens: 10
          })
        ]
      }
    } as any);

    await flushDeferredTurnWork();
    await flushMicrotasks();

    expect(store.turnTraceHistory[0]?.providerCallRecords?.map((record) => record.requestKind)).toEqual([
      "initial_request",
      "tool_followup"
    ]);
    expect(store.turnTraceHistory[0]?.cacheHitInputTokens).toBe(15);
    expect(store.turnTraceHistory[0]?.providerCallRecords?.[0]?.turnDurationMs).toBe(420);
    expect(store.turnTraceHistory[0]?.providerCallRecords?.[1]?.turnDurationMs).toBe(420);

    nowSpy.mockRestore();
  });

  it("preserves prefix mutation reasons on completed trace history", async () => {
    const store = useRuntimeStore();
    const snapshot = createSnapshot({
      conversationId: "trace-session",
      turnTraceHistory: [
        createTrace({
          providerCallRecords: [
            createProviderCallRecord({
              prefixMutationReasons: [
                "session_summary_changed",
                "run_goal_changed",
                "truncation_note_changed"
              ]
            })
          ]
        })
      ]
    });

    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "load_session_runtime_view") {
        return createSessionRuntimeView(snapshot);
      }
      if (command === "load_retrieved_context") {
        return createRetrievedContext(snapshot);
      }
      if (command === "list_sessions") {
        return [] satisfies SessionOverview[];
      }
      throw new Error(`unexpected command: ${command}`);
    });

    await store.loadSessionState("trace-session");

    expect(store.turnTraceHistory[0]?.providerCallRecords?.[0]?.prefixMutationReasons).toEqual([
      "session_summary_changed",
      "run_goal_changed",
      "truncation_note_changed"
    ]);
  });

  it("hydrates hook trace records from backend session snapshots", async () => {
    const store = useRuntimeStore();
    const snapshot = createSnapshot({
      conversationId: "hook-hydration-session",
      summary: "hook hydration summary",
      history: [
        { role: "user", content: "resume hook session" },
        { role: "assistant", content: "hook session restored" }
      ],
      turnTraceHistory: [
        createTrace({
          turnId: "turn-hook-hydrated",
          phase: "completed",
          hookTraceRecords: [
            createHookTraceRecord({
              hookName: "observe.checkpoint",
              hookPoint: "checkpoint_persist_end",
              summary: "checkpoint hook persisted"
            }),
            createHookTraceRecord({
              hookName: "observe.finalize",
              hookPoint: "turn_finalize_end",
              hookOrder: 2,
              summary: "finalize hook persisted"
            })
          ],
          updatedAt: 4100
        })
      ],
      turnCount: 1,
      updatedAtMs: 4100
    });

    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "load_session_runtime_view") {
        return createSessionRuntimeView(snapshot);
      }

      if (command === "list_sessions") {
        return [] satisfies SessionOverview[];
      }

      throw new Error(`unexpected command: ${command}`);
    });

    await store.loadSessionState("hook-hydration-session");

    expect(store.turnTraceHistory).toHaveLength(1);
    expect(store.turnTraceHistory[0]?.hookTraceRecords?.map((record) => record.hookName)).toEqual([
      "observe.checkpoint",
      "observe.finalize"
    ]);
    expect(store.turnTraceHistory[0]?.hookTraceRecords?.[1]?.hookPoint).toBe("turn_finalize_end");
  });

  it("strips the leading thinking prefix from streamed reasoning content", async () => {
    const store = useRuntimeStore();
    const nowSpy = vi.spyOn(Date, "now").mockReturnValue(6161);
    const eventHandlers = new Map<string, (event: { payload: Record<string, unknown> }) => void>();

    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeListen.mockImplementation(async (eventName: string, handler: unknown) => {
      eventHandlers.set(eventName, handler as (event: { payload: Record<string, unknown> }) => void);
      return () => {};
    });
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "inspect_host") {
        return { runs: [] };
      }

      if (command === "start_graph_run_stream") {
        return {
          run: { id: "run-reasoning" },
          turnId: "6161"
        };
      }

      if (command === "list_sessions") {
        return [] satisfies SessionOverview[];
      }

      throw new Error(`unexpected command: ${command}`);
    });

    store.$patch({
      sessionId: "reasoning-prefix-session",
      draftMessage: "reasoning prefix request",
      phase: "idle",
      messages: []
    });

    await store.submitTurn();

    eventHandlers.get("turn:started")?.({
      payload: {
        turnId: "6161",
        providerName: "OpenAI",
        providerModel: "gpt-5",
        providerRequestedName: "OpenAI"
      }
    });

    eventHandlers.get("turn:delta")?.({
      payload: {
        turnId: "6161",
        reasoningContent: "thinking: first pass"
      }
    });

    eventHandlers.get("turn:completed")?.({
      payload: {
        turnId: "6161",
        text: "final answer",
        reasoningContent: "thinking: final pass",
        providerName: "OpenAI",
        providerModel: "gpt-5",
        providerRequestedName: "OpenAI",
        traceSteps: store.traceSteps,
        toolActivities: []
      }
    });

    await flushMicrotasks();

    expect(store.messages[1]?.reasoningContent).toBe("final pass");
    nowSpy.mockRestore();
  });

  it("starts a fresh browser-preview turn after rollback to initial state without reviving old messages", async () => {
    const store = useRuntimeStore();
    const nowSpy = vi.spyOn(Date, "now").mockReturnValue(8081);

    tauriMocks.mockIsTauriAvailable.mockReturnValue(false);
    store.$patch({
      sessionId: "browser-session",
      draftMessage: "new request",
      phase: "ready",
      visibleNodeId: "node-head",
      branchHeadNodeId: "node-head",
      activeBranchId: "branch-main",
      historyCursorMode: "live",
      messages: [
        createMessage({ id: "user-old", turnId: "turn-old", role: "user", content: "old question" }),
        createMessage({ id: "assistant-old", turnId: "turn-old", role: "assistant", content: "old answer" })
      ],
      turnTraceHistory: [createTrace({ turnId: "turn-old", title: "old turn", updatedAt: 1000 })],
      historyNodes: [
        createHistoryNode({ nodeId: "node-old", sessionId: "browser-session", turnId: "turn-old", createdAtMs: 1000 }),
        createHistoryNode({ nodeId: "node-head", sessionId: "browser-session", turnId: "turn-head", createdAtMs: 2000 })
      ],
      historyBranches: [
        createHistoryBranch({ branchId: "branch-main", sessionId: "browser-session", headNodeId: "node-head", baseNodeId: "node-old" })
      ]
    });

    store.rollbackToInitialState();
    store.draftMessage = "new request";

    await store.submitTurn();

    expect(store.initialRollbackActive).toBe(false);
    expect(store.phase).toBe("completed");
    expect(store.messages).toHaveLength(2);
    expect(store.messages.map((message) => message.content)).toEqual([
      "new request",
      expect.any(String)
    ]);
    expect(store.messages.some((message) => message.content === "old question")).toBe(false);
    expect(store.messages.some((message) => message.content === "old answer")).toBe(false);

    nowSpy.mockRestore();
  });

  it("processes streamed turn events after submitting from a historical root checkout", async () => {
    const nowSpy = vi.spyOn(Date, "now").mockReturnValue(9191);
    const eventHandlers = new Map<string, (event: { payload: Record<string, unknown> }) => void>();
    tauriMocks.mockSafeListen.mockImplementation(async (eventName: string, handler: unknown) => {
      eventHandlers.set(eventName, handler as (event: { payload: Record<string, unknown> }) => void);
      return () => {};
    });
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "resolve_graph_run_submission_plan") {
        return { command: "start_graph_run_stream", runId: null };
      }

      if (command === "start_graph_run_stream") {
        return {
          run: { id: "run-root-restart" },
          turnId: "9191"
        };
      }

      if (command === "list_sessions") {
        return [] satisfies SessionOverview[];
      }

      throw new Error(`unexpected command: ${command}`);
    });

    const store = useRuntimeStore();
    store.$patch({
      sessionId: "session-root-restart",
      draftMessage: "restart from root",
      phase: "ready",
      visibleNodeId: "node-root",
      branchHeadNodeId: "node-head",
      activeBranchId: "branch-main",
      historyCursorMode: "historical",
      messages: [],
      turnTraceHistory: []
    });

    await store.submitTurn();

    expect(store.historyCursorMode).toBe("live");

    eventHandlers.get("turn:delta")?.({
      payload: {
        turnId: "9191",
        eventId: "event-turn-delta-9191",
        eventType: "turn.output_delta",
        sequence: 1,
        emittedAtMs: 1000,
        text: "partial answer"
      }
    });

    await new Promise((resolve) => window.setTimeout(resolve, 50));

    expect(store.messages.find((message) => message.role === "assistant")?.content).toBe("partial answer");

    eventHandlers.get("turn:completed")?.({
      payload: {
        turnId: "9191",
        eventId: "event-turn-completed-9191",
        eventType: "turn.completed",
        eventVersion: "1.0",
        sequence: 2,
        emittedAtMs: 1200,
        text: "final answer",
        providerName: "OpenAI",
        providerModel: "gpt-5",
        providerProtocol: "openai",
        providerSource: "primary",
        providerMode: "standard",
        outputTokens: 9,
        traceSteps: store.traceSteps,
        toolActivities: []
      }
    });

    await flushDeferredTurnWork();
    await flushMicrotasks();

    expect(store.messages.find((message) => message.role === "assistant")?.content).toBe("final answer");
    expect(store.messages.find((message) => message.role === "assistant")?.status).toBe("done");
    nowSpy.mockRestore();
  });

  it("writes final assistant text into persisted background sessions before the user switches back", async () => {
    const store = useRuntimeStore();
    const eventHandlers = new Map<string, (event: { payload: Record<string, unknown> }) => void>();

    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeListen.mockImplementation(async (eventName: string, handler: unknown) => {
      eventHandlers.set(eventName, handler as (event: { payload: Record<string, unknown> }) => void);
      return () => {};
    });
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "inspect_host") {
        return { runs: [] };
      }

      if (command === "load_session_runtime_view") {
        return createSessionRuntimeView(
          createSnapshot({
            conversationId: "session-other",
            title: "Other session",
            summary: "Other summary",
            history: [
              { role: "user", content: "other question" },
              { role: "assistant", content: "other answer" }
            ],
            turnCount: 1,
            updatedAtMs: 2000
          })
        );
      }

      if (command === "list_sessions") {
        return [
          {
            conversationId: "session-background",
            title: "Background session",
            summary: "Background summary",
            turnCount: 1,
            lastReferencedFile: null,
            updatedAtMs: 1000
          },
          {
            conversationId: "session-other",
            title: "Other session",
            summary: "Other summary",
            turnCount: 1,
            lastReferencedFile: null,
            updatedAtMs: 2000
          }
        ] satisfies SessionOverview[];
      }

      throw new Error(`unexpected command: ${command}`);
    });

    const backgroundMessages = [
      createMessage({ id: "user-bg", turnId: "turn-bg", role: "user", content: "background question" }),
      createMessage({
        id: "assistant-turn-bg",
        turnId: "turn-bg",
        role: "assistant",
        content: "",
        status: "pending",
        modelName: "OpenAI / gpt-5"
      })
    ];

    writePersistedSessions({
      "session-background": {
        phase: "calling_model",
        messages: backgroundMessages,
        attachmentAssets: [],
        sessionSummary: "Background summary",
        providerRequestedName: "OpenAI",
        providerName: "OpenAI",
        providerProtocol: "openai",
        providerModel: "gpt-5",
        providerSource: "primary",
        providerMode: "standard",
        fallbackReason: null,
        inputTokens: null,
        outputTokens: null,
        totalTokens: null,
        firstTokenLatencyMs: null
      }
    });

    await store.initializeTurnEvents();
    store.$patch({
      sessionId: "session-background",
      sessionList: [
        {
          conversationId: "session-background",
          title: "Background session",
          summary: "Background summary",
          turnCount: 1,
          lastReferencedFile: null,
          updatedAtMs: 1000
        },
        {
          conversationId: "session-other",
          title: "Other session",
          summary: "Other summary",
          turnCount: 1,
          lastReferencedFile: null,
          updatedAtMs: 2000
        }
      ],
      messages: backgroundMessages,
      isSubmitting: true,
      activeTurnId: "turn-bg",
      phase: "calling_model"
    });

    await store.switchSession("session-other");

    eventHandlers.get("turn:completed")?.({
      payload: {
        sessionId: "session-background",
        turnId: "turn-bg",
        text: "background final answer",
        reasoningContent: "background final reasoning",
        providerName: "OpenAI",
        providerModel: "gpt-5",
        providerProtocol: "openai",
        providerSource: "primary",
        providerMode: "standard",
        providerRequestedName: "OpenAI",
        sessionSummary: "Background completed summary",
        outputTokens: 42,
        totalTokens: 100,
        traceSteps: [],
        toolActivities: []
      }
    } as any);

    await flushMicrotasks();

    const persisted = JSON.parse(window.localStorage.getItem(RUNTIME_STORAGE_KEY) ?? "{}") as {
      sessions?: Record<string, { messages?: ChatMessage[]; sessionSummary?: string; phase?: string }>;
    };
    const persistedBackground = persisted.sessions?.["session-background"];
    const assistant = persistedBackground?.messages?.find(
      (message) => message.turnId === "turn-bg" && message.role === "assistant"
    );

    expect(assistant?.content).toBe("background final answer");
    expect(assistant?.reasoningContent).toBe("background final reasoning");
    expect(assistant?.status).toBe("done");
    expect(persistedBackground?.sessionSummary).toBe("Background completed summary");
    expect(persistedBackground?.phase).toBe("ready");
  });

  it("does not fail the turn when stream start response omits run id but streamed completion still arrives", async () => {
    const store = useRuntimeStore();
    const nowSpy = vi.spyOn(Date, "now").mockReturnValue(9191);
    const eventHandlers = new Map<string, (event: { payload: Record<string, unknown> }) => void>();

    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeListen.mockImplementation(async (eventName: string, handler: unknown) => {
      eventHandlers.set(eventName, handler as (event: { payload: Record<string, unknown> }) => void);
      return () => {};
    });
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "inspect_host") {
        return { runs: [] };
      }

      if (command === "start_graph_run_stream") {
        return {
          event: { kind: "continue", reason: "planner_requested_continue", summary: "accepted" },
          turnId: "9191"
        };
      }

      if (command === "list_sessions") {
        return [] satisfies SessionOverview[];
      }

      throw new Error(`unexpected command: ${command}`);
    });

    store.$patch({
      sessionId: "stream-session-missing-run-id",
      draftMessage: "stream start without run id",
      phase: "idle",
      messages: []
    });

    await expect(store.submitTurn()).resolves.toBe(true);
    expect(store.phase).toBe("calling_model");
    expect(store.activeTurnId).toBe("9191");
    expect(store.activeRunId).toBeNull();
    expect(store.error).toBeNull();

    eventHandlers.get("turn:completed")?.({
      payload: {
        turnId: "9191",
        text: "final answer after missing run id",
        providerName: "OpenAI",
        providerModel: "gpt-5",
        providerProtocol: "openai",
        providerSource: "primary",
        providerMode: "standard",
        traceSteps: store.traceSteps,
        toolActivities: []
      }
    } as any);

    await flushDeferredTurnWork();
    await flushMicrotasks();

    expect(store.phase).toBe("ready");
    expect(store.messages[1]?.content).toBe("final answer after missing run id");
    expect(store.turnTraceHistory[0]?.phase).toBe("completed");
    expect(store.turnTraceHistory[0]?.error).toBeNull();

    nowSpy.mockRestore();
  });

  it("persists runtime-produced hook trace records across streamed event hydration", async () => {
    const store = useRuntimeStore();
    const nowSpy = vi.spyOn(Date, "now").mockReturnValue(6262);
    const eventHandlers = new Map<string, (event: { payload: Record<string, unknown> }) => void>();

    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeListen.mockImplementation(async (eventName: string, handler: unknown) => {
      eventHandlers.set(eventName, handler as (event: { payload: Record<string, unknown> }) => void);
      return () => {};
    });
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "inspect_host") {
        return { runs: [] };
      }

      if (command === "start_graph_run_stream") {
        return {
          run: { id: "run-hook-stream" },
          turnId: "6262"
        };
      }

      if (command === "list_sessions") {
        return [] satisfies SessionOverview[];
      }

      throw new Error(`unexpected command: ${command}`);
    });

    store.$patch({
      sessionId: "hook-stream-session",
      draftMessage: "trace hook request",
      phase: "idle",
      messages: []
    });

    await store.submitTurn();

    eventHandlers.get("turn:started")?.({
      payload: {
        turnId: "6262",
        providerName: "OpenAI",
        providerModel: "gpt-5",
        providerRequestedName: "OpenAI"
      }
    });

    eventHandlers.get("turn:trace")?.({
      payload: {
        turnId: "6262",
        eventId: "6262:2",
        eventType: "turn.model_call_started",
        eventVersion: "turn-event-v1",
        sequence: 2,
        emittedAtMs: 6262002,
        phase: "calling_model",
        providerName: "OpenAI",
        providerModel: "gpt-5",
        providerRequestedName: "OpenAI",
        traceSteps: store.traceSteps,
        hookTraceRecords: [
          createHookTraceRecord({
            hookName: "observe.model",
            hookPoint: "model_call_start",
            summary: "model call hook observed"
          })
        ]
      }
    });

    expect(store.traceTimeline.length).toBeGreaterThan(0);
    expect(store.toolActivities).toEqual([]);
    expect(store.turnTraceHistory[0]?.hookTraceRecords ?? []).toEqual([]);

    eventHandlers.get("turn:tool")?.({
      payload: {
        turnId: "6262",
        eventId: "6262:3",
        eventType: "turn.tool_call_started",
        eventVersion: "turn-event-v1",
        sequence: 3,
        emittedAtMs: 6262003,
        phase: "executing_tool",
        toolActivities: [
          {
            id: "tool-1",
            name: "workspace.read_file",
            status: "running",
            summary: "running",
            argumentsText: "{\"path\":\"README.md\"}",
            resultText: null,
            durationSeconds: null,
            capabilityInvocation: null
          }
        ],
        hookTraceRecords: [
          createHookTraceRecord({
            hookName: "observe.tool-start",
            hookPoint: "tool_call_start",
            summary: "tool start hook observed"
          })
        ]
      }
    });

    expect(store.toolActivities.map((tool) => tool.name)).toEqual(["workspace.read_file"]);
    expect(store.turnTraceHistory[0]?.hookTraceRecords ?? []).toEqual([]);

    eventHandlers.get("turn:completed")?.({
      payload: {
        turnId: "6262",
        eventId: "6262:4",
        eventType: "turn.completed",
        eventVersion: "turn-event-v1",
        sequence: 4,
        emittedAtMs: 6262004,
        phase: "completed",
        text: "hook stream done",
        providerName: "OpenAI",
        providerModel: "gpt-5",
        providerRequestedName: "OpenAI",
        sessionSummary: "hook stream summary",
        traceSteps: store.traceSteps,
        toolActivities: [
          {
            id: "tool-1",
            name: "workspace.read_file",
            status: "done",
            summary: "done",
            argumentsText: "{\"path\":\"README.md\"}",
            resultText: "ok",
            durationSeconds: 0.12,
            capabilityInvocation: null
          }
        ],
        hookTraceRecords: [
          createHookTraceRecord({
            hookName: "observe.checkpoint",
            hookPoint: "checkpoint_persist_end",
            summary: "checkpoint hook observed"
          }),
          createHookTraceRecord({
            hookName: "observe.finalize",
            hookPoint: "turn_finalize_end",
            hookOrder: 2,
            summary: "finalize hook observed"
          })
        ]
      }
    });

    await flushDeferredTurnWork();
    await flushMicrotasks();

    expect(store.turnTraceHistory[0]?.hookTraceRecords?.map((record) => record.hookName)).toEqual([
      "observe.checkpoint",
      "observe.finalize"
    ]);
    expect(store.turnTraceHistory[0]?.eventType).toBe("turn.completed");
    nowSpy.mockRestore();
  });

  it("applies failed stream events to the active turn", async () => {
    const store = useRuntimeStore();
    const nowSpy = vi.spyOn(Date, "now").mockReturnValue(7070);
    const eventHandlers = new Map<string, (event: { payload: Record<string, unknown> }) => void>();

    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeListen.mockImplementation(async (eventName: string, handler: unknown) => {
      eventHandlers.set(eventName, handler as (event: { payload: Record<string, unknown> }) => void);
      return () => {};
    });
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "inspect_host") {
        return { runs: [] };
      }

      if (command === "start_graph_run_stream") {
        return {
          run: { id: "run-failed" },
          turnId: "7070"
        };
      }

      throw new Error(`unexpected command: ${command}`);
    });

    store.$patch({
      sessionId: "failed-stream-session",
      draftMessage: "failing stream request",
      phase: "idle",
      messages: []
    });

    await store.submitTurn();

    eventHandlers.get("turn:started")?.({
      payload: {
        turnId: "7070",
        providerName: "OpenAI",
        providerModel: "gpt-5",
        providerRequestedName: "OpenAI"
      }
    });

    eventHandlers.get("turn:failed")?.({
      payload: {
        turnId: "7070",
        text: "failure response",
        reasoningContent: "failure reasoning",
        error: "tool chain exploded",
        providerName: "OpenAI",
        providerModel: "gpt-5",
        providerRequestedName: "OpenAI",
        traceSteps: store.traceSteps,
        toolActivities: []
      }
    });

    expect(store.phase).toBe("failed");
    expect(store.isSubmitting).toBe(false);
    expect(store.activeTurnId).toBeNull();
    expect(store.error).toContain("tool chain exploded");
    expect(store.messages).toHaveLength(2);
    expect(store.messages[1]?.role).toBe("assistant");
    expect(store.messages[1]?.content).toBe("failure response");
    expect(store.messages[1]?.reasoningContent).toBe("failure reasoning");
    expect(store.messages[1]?.status).toBe("error");
    expect(store.turnTraceHistory).toHaveLength(1);
    expect(store.turnTraceHistory[0]?.phase).toBe("failed");
    expect(store.turnTraceHistory[0]?.error).toContain("tool chain exploded");
    expect(store.turnTraceHistory[0]?.title).toBe("failing stream request");

    nowSpy.mockRestore();
  });

  it("stops the active turn and handles cancelled stream events", async () => {
    const store = useRuntimeStore();
    const nowSpy = vi.spyOn(Date, "now").mockReturnValue(8080);
    const eventHandlers = new Map<string, (event: { payload: Record<string, unknown> }) => void>();

    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeListen.mockImplementation(async (eventName: string, handler: unknown) => {
      eventHandlers.set(eventName, handler as (event: { payload: Record<string, unknown> }) => void);
      return () => {};
    });
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "inspect_host") {
        return { runs: [] };
      }

      if (command === "start_graph_run_stream") {
        return {
          run: { id: "run-cancelled" },
          turnId: "8080"
        };
      }

      if (command === "stop_graph_run") {
        return null;
      }

      throw new Error(`unexpected command: ${command}`);
    });

    store.$patch({
      sessionId: "cancelled-stream-session",
      draftMessage: "cancel this turn",
      phase: "idle",
      messages: []
    });

    await store.submitTurn();
    await store.stopTurn();

    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledWith("stop_graph_run", {
      runId: "run-cancelled"
    });

    eventHandlers.get("turn:started")?.({
      payload: {
        turnId: "8080",
        providerName: "OpenAI",
        providerModel: "gpt-5",
        providerRequestedName: "OpenAI"
      }
    });
    eventHandlers.get("turn:cancelled")?.({
      payload: {
        turnId: "8080",
        text: "This turn was cancelled.",
        error: "stopped_by_user",
        providerName: "OpenAI",
        providerModel: "gpt-5",
        providerRequestedName: "OpenAI",
        toolActivities: []
      }
    });

    expect(store.phase).toBe("cancelled");
    expect(store.isSubmitting).toBe(false);
    expect(store.activeTurnId).toBeNull();
    expect(store.messages[1]?.content).toBe("This turn was cancelled.");
    expect(store.turnTraceHistory[0]?.phase).toBe("cancelled");
    expect(store.turnTraceHistory[0]?.error).toBe("stopped_by_user");
    expect(store.traceSteps.map((step) => step.state)).toEqual([
      "completed",
      "completed",
      "cancelled",
      "cancelled"
    ]);
    expect(store.turnTraceHistory[0]?.traceSteps.map((step) => step.state)).toEqual([
      "completed",
      "completed",
      "cancelled",
      "cancelled"
    ]);

    nowSpy.mockRestore();
  });

  it("ignores stale failed and cancelled terminal events after a newer turn is active", async () => {
    const store = useRuntimeStore();
    const nowSpy = vi.spyOn(Date, "now").mockReturnValue(9090);
    const eventHandlers = new Map<string, (event: { payload: Record<string, unknown> }) => void>();

    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeListen.mockImplementation(async (eventName: string, handler: unknown) => {
      eventHandlers.set(eventName, handler as (event: { payload: Record<string, unknown> }) => void);
      return () => {};
    });
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "inspect_host") {
        return { runs: [] };
      }

      if (command === "start_graph_run_stream") {
        return {
          run: { id: "run-stale-old" },
          turnId: "9090"
        };
      }

      throw new Error(`unexpected command: ${command}`);
    });

    store.$patch({
      sessionId: "stale-terminal-session",
      draftMessage: "first request",
      phase: "idle",
      messages: []
    });

    await store.submitTurn();
    store.$patch({
      activeTurnId: "newer-turn",
      activeRunId: "newer-run",
      isSubmitting: true,
      phase: "calling_model"
    });

    eventHandlers.get("turn:failed")?.({
      payload: {
        turnId: "9090",
        text: "old failed event",
        error: "old failure"
      }
    });
    eventHandlers.get("turn:cancelled")?.({
      payload: {
        turnId: "9090",
        text: "old cancelled event",
        error: "old cancelled"
      }
    });

    expect(store.activeTurnId).toBe("newer-turn");
    expect(store.activeRunId).toBe("newer-run");
    expect(store.isSubmitting).toBe(true);
    expect(store.phase).toBe("calling_model");
    expect(store.error).toBeNull();

    nowSpy.mockRestore();
  });

  it("seeds a timeout retrying assistant message when submitting after timeout failure", async () => {
    const store = useRuntimeStore();
    const nowSpy = vi.spyOn(Date, "now").mockReturnValue(9191);

    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "inspect_host") {
        return { runs: [] };
      }

      if (command === "start_graph_run_stream") {
        return {
          run: { id: "run-timeout-retry" },
          turnId: "9191"
        };
      }

      throw new Error(`unexpected command: ${command}`);
    });

    store.$patch({
      sessionId: "timeout-retry-session",
      draftMessage: "retry this timeout",
      phase: "failed",
      error: "timeout: previous call timed out",
      messages: [
        createMessage({
          id: "assistant-timeout-old",
          turnId: "turn-timeout-old",
          role: "assistant",
          content: "旧失败",
          status: "error",
          errorDetail: "timeout: previous call timed out"
        })
      ]
    });

    const started = await store.submitTurn();

    expect(started).toBe(true);
    const assistantMessage = store.messages.find((message) => message.id === "assistant-9191");
    expect(assistantMessage?.status).toBe("pending");
    expect(assistantMessage?.content).toBe("超时后错误重连中...");
    expect(assistantMessage?.errorDetail).toBe("timeout: previous call timed out");
    nowSpy.mockRestore();
  });

  it("passes nodeId through runtime and retrieved context requests and hydrates history cursor state", async () => {
    const store = useRuntimeStore();
    const snapshot = createSnapshot({
      conversationId: "history-session",
      summary: "History summary",
      history: [
        { role: "user", content: "old question" },
        { role: "assistant", content: "old answer" }
      ],
      turnCount: 1,
      updatedAtMs: 3000
    });
    const historyNodes = [
      createHistoryNode({ nodeId: "node-root", sessionId: "history-session", createdAtMs: 1000 }),
      createHistoryNode({
        nodeId: "node-old",
        sessionId: "history-session",
        parentNodeId: "node-root",
        createdAtMs: 2000
      }),
      createHistoryNode({
        nodeId: "node-head",
        sessionId: "history-session",
        parentNodeId: "node-old",
        createdAtMs: 3000
      })
    ];
    const historyBranches = [
      createHistoryBranch({
        branchId: "branch-main",
        sessionId: "history-session",
        baseNodeId: "node-root",
        headNodeId: "node-head"
      })
    ];
    const historyCursor = createHistoryCursor({
      sessionId: "history-session",
      visibleNodeId: "node-old",
      activeBranchId: "branch-main",
      branchHeadNodeId: "node-head",
      workspaceNodeId: "node-old",
      mode: "historical"
    });

    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string, payload?: Record<string, unknown>) => {
      if (command === "load_session_runtime_view") {
        expect(payload).toEqual({
          turnId: null,
          sessionId: "history-session",
          runId: null,
          nodeId: "node-old"
        });
        return createSessionRuntimeView(snapshot, {
          historyNodes,
          historyBranches,
          historyCursor
        });
      }

      if (command === "load_retrieved_context") {
        expect(payload).toEqual({
          sessionId: "history-session",
          runId: null,
          turnId: null,
          nodeId: "node-old"
        });
        return createRetrievedContext(snapshot);
      }

      if (command === "list_sessions") {
        return [] satisfies SessionOverview[];
      }

      throw new Error(`unexpected command: ${command}`);
    });

    await store.loadSessionState("history-session", { nodeId: "node-old" });
    const retrieved = await store.loadRetrievedContextState("history-session", { nodeId: "node-old" });

    expect(retrieved.sessionContext.conversationId).toBe("history-session");
    expect(store.visibleNodeId).toBe("node-old");
    expect(store.branchHeadNodeId).toBe("node-head");
    expect(store.activeBranchId).toBe("branch-main");
    expect(store.historyCursorMode).toBe("historical");
    expect(store.isHistoricalMode).toBe(true);
    expect(store.historyNodes.map((node) => node.nodeId)).toEqual(["node-root", "node-old", "node-head"]);
  });

  it("preserves host authority metadata on runtime views", async () => {
    const store = useRuntimeStore();
    const snapshot = createSnapshot({
      conversationId: "authority-session",
      summary: "Authority summary",
      history: [
        { role: "user", content: "old question" },
        { role: "assistant", content: "old answer" }
      ],
      turnCount: 1,
      updatedAtMs: 3300
    });

    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string, payload?: Record<string, unknown>) => {
      if (command === "load_session_runtime_view") {
        expect(payload).toEqual({
          turnId: null,
          sessionId: "authority-session",
          runId: null
        });
        return createSessionRuntimeView(snapshot, {
          authorityMode: "host_authoritative",
          resolvedVisibleNodeId: "node-old",
          activeBranchHeadNodeId: "node-head",
          isAtBranchHead: false,
          cursorVersion: 7,
          historyCursor: createHistoryCursor({
            sessionId: "authority-session",
            visibleNodeId: "node-old",
            activeBranchId: "branch-main",
            branchHeadNodeId: "node-head",
            mode: "historical",
            authorityMode: "host_authoritative",
            cursorVersion: 7,
            isAtBranchHead: false
          })
        });
      }

      if (command === "list_sessions") {
        return [] satisfies SessionOverview[];
      }

      throw new Error(`unexpected command: ${command}`);
    });

    await store.loadSessionState("authority-session");

    expect(store.visibleNodeId).toBe("node-old");
    expect(store.branchHeadNodeId).toBe("node-head");
    expect(store.historyCursorMode).toBe("historical");
  });

  it("hydrates host-projected history state even when legacy historyCursor mirror is omitted", async () => {
    const store = useRuntimeStore();
    const sessionId = "authority-projection-session";
    const snapshot = createSnapshot({
      conversationId: sessionId,
      summary: "Projection summary",
      history: [
        { role: "user", content: "old question" },
        { role: "assistant", content: "old answer" }
      ],
      turnCount: 1,
      updatedAtMs: 3300
    });

    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string, payload?: Record<string, unknown>) => {
      if (command === "load_session_runtime_view") {
        expect(payload).toEqual({
          turnId: null,
          sessionId,
          runId: null
        });
        return createSessionRuntimeView(snapshot, {
          authorityMode: "host_authoritative",
          resolvedVisibleNodeId: "node-old",
          activeBranchHeadNodeId: "node-head",
          isAtBranchHead: false,
          cursorVersion: null,
          historyCursor: null,
          historyNodes: [
            createHistoryNode({ nodeId: "node-old", sessionId, turnId: "turn-old", createdAtMs: 2000 }),
            createHistoryNode({ nodeId: "node-head", sessionId, turnId: "turn-head", createdAtMs: 3200 })
          ],
          historyBranches: [
            createHistoryBranch({ branchId: "branch-main", sessionId, headNodeId: "node-head", baseNodeId: "node-old" })
          ]
        });
      }

      if (command === "list_sessions") {
        return [] satisfies SessionOverview[];
      }

      throw new Error(`unexpected command: ${command}`);
    });

    await store.loadSessionState(sessionId);

    expect(store.visibleNodeId).toBe("node-old");
    expect(store.branchHeadNodeId).toBe("node-head");
    expect(store.activeBranchId).toBe("branch-main");
    expect(store.historyCursorMode).toBe("historical");
  });

  it("clears stale local historical state when host-authoritative runtime view omits history state", async () => {
    const store = useRuntimeStore();
    const sessionId = "host-live-session";
    const snapshot = createSnapshot({
      conversationId: sessionId,
      summary: "Live summary",
      history: [
        { role: "user", content: "latest question" },
        { role: "assistant", content: "latest answer" }
      ],
      turnCount: 1,
      updatedAtMs: 3600
    });

    store.$patch({
      sessionId,
      visibleNodeId: "node-old",
      branchHeadNodeId: "node-head",
      activeBranchId: "branch-main",
      historyCursorMode: "historical",
      historyNodes: [createHistoryNode({ nodeId: "node-old", sessionId })],
      historyBranches: [createHistoryBranch({ branchId: "branch-main", sessionId, headNodeId: "node-head" })]
    });

    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string, payload?: Record<string, unknown>) => {
      if (command === "load_session_runtime_view") {
        expect(payload).toEqual({
          turnId: null,
          sessionId,
          runId: null
        });
        return createSessionRuntimeView(snapshot, {
          authorityMode: "host_authoritative",
          resolvedVisibleNodeId: null,
          activeBranchHeadNodeId: null,
          isAtBranchHead: true,
          cursorVersion: null,
          historyCursor: null,
          historyNodes: [],
          historyBranches: []
        });
      }

      if (command === "list_sessions") {
        return [] satisfies SessionOverview[];
      }

      throw new Error(`unexpected command: ${command}`);
    });

    await store.loadSessionState(sessionId);

    expect(store.visibleNodeId).toBeNull();
    expect(store.branchHeadNodeId).toBeNull();
    expect(store.activeBranchId).toBeNull();
    expect(store.historyCursorMode).toBe("live");
  });

  it("hydrates latest history audit summary from runtime view when loading a session", async () => {
    const store = useRuntimeStore();
    const snapshot = createSnapshot({
      conversationId: "history-audit-session",
      summary: "History audit summary",
      history: [
        { role: "user", content: "history question" },
        { role: "assistant", content: "history answer" }
      ],
      turnCount: 1,
      updatedAtMs: 3300,
      historyStateAuditSummary: createHistoryStateAuditSummary({
        action: {
          summary: "来自 snapshot 的旧 summary，不应覆盖 runtime view"
        }
      })
    });
    const runtimeSummary = createHistoryStateAuditSummary({
      action: {
        commandKind: "restore_branch_head",
        boundary: "history.restore.resolved",
        summary: "来自 runtime view 的最新 history-control summary"
      },
      currentContext: {
        mode: "live",
        visibleNodeId: "node-live",
        activeBranchId: "branch-runtime",
        branchHeadNodeId: "node-head-runtime"
      }
    });

    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string, payload?: Record<string, unknown>) => {
      if (command === "load_session_runtime_view") {
        expect(payload).toEqual({
          turnId: null,
          sessionId: "history-audit-session",
          runId: null
        });
        return createSessionRuntimeView(snapshot, {
          historyStateAuditSummary: runtimeSummary
        });
      }

      if (command === "list_sessions") {
        return [] satisfies SessionOverview[];
      }

      throw new Error(`unexpected command: ${command}`);
    });

    await store.loadSessionState("history-audit-session");

    expect(store.latestHistoryStateAuditSummary).toEqual(runtimeSummary);
    expect(store.latestHistoryStateAuditSummary?.action.summary).toBe(
      "来自 runtime view 的最新 history-control summary"
    );
  });

  it("checks out a history node with backward-compatible cursor fallback", async () => {
    const store = useRuntimeStore();
    const snapshot = createSnapshot({
      conversationId: "checkout-session",
      summary: "Checkout summary",
      history: [
        { role: "user", content: "current question" },
        { role: "assistant", content: "current answer" }
      ],
      turnCount: 1,
      updatedAtMs: 3200
    });

    store.$patch({
      sessionId: "checkout-session",
      activeBranchId: "branch-main",
      branchHeadNodeId: "node-head",
      visibleNodeId: "node-head",
      cursorVersion: 3,
      historyCursorMode: "live",
      historyBranches: [
        createHistoryBranch({
          branchId: "branch-main",
          sessionId: "checkout-session",
          headNodeId: "node-head"
        })
      ],
      historyNodes: [
        createHistoryNode({ nodeId: "node-old", sessionId: "checkout-session", createdAtMs: 2000 }),
        createHistoryNode({ nodeId: "node-head", sessionId: "checkout-session", createdAtMs: 3200 })
      ]
    });

    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string, payload?: Record<string, unknown>) => {
      if (command === "checkout_history_node") {
        expect(payload).toEqual({
          sessionId: "checkout-session",
          nodeId: "node-old",
          mode: "transcript_and_workspace",
          expectedCursorVersion: 3
        });
        return {
          sessionId: "checkout-session",
          nodeId: "node-old",
          requestedMode: "transcript_and_workspace",
          appliedMode: "transcript_only",
          transcriptRestoreApplied: true,
          workspaceRollbackCapable: false,
          workspaceRollbackApplied: false,
          degraded: true,
          degradationReason: "workspace_rollback_unsupported"
          ,
          cursor: createHistoryCursor({
            sessionId: "checkout-session",
            visibleNodeId: "node-old",
            activeBranchId: "branch-main",
            branchHeadNodeId: "node-head",
            workspaceNodeId: "node-old",
            mode: "historical"
          })
        };
      }

      if (command === "load_session_runtime_view") {
        expect(payload).toEqual({
          turnId: null,
          sessionId: "checkout-session",
          runId: null,
          nodeId: "node-old"
        });
        return createSessionRuntimeView(snapshot);
      }

      throw new Error(`unexpected command: ${command}`);
    });

    const result = await store.checkoutHistoryNode("node-old", "transcript_and_workspace");

    expect(result?.transcriptRestoreApplied).toBe(true);
    expect(result?.workspaceRestoreCapable).toBe(false);
    expect(result?.degradedToTranscriptOnly).toBe(true);
    expect(result?.degradationReason).toBe("workspace_rollback_unsupported");
    expect(store.visibleNodeId).toBe("node-old");
    expect(store.branchHeadNodeId).toBe("node-head");
    expect(store.activeBranchId).toBe("branch-main");
    expect(store.historyCursorMode).toBe("historical");
  });

  it("surfaces cursor revision conflicts from host history checkout", async () => {
    const store = useRuntimeStore();

    store.$patch({
      sessionId: "checkout-session",
      activeBranchId: "branch-main",
      branchHeadNodeId: "node-head",
      visibleNodeId: "node-head",
      cursorVersion: 9,
      historyCursorMode: "live"
    });

    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string, payload?: Record<string, unknown>) => {
      if (command === "checkout_history_node") {
        expect(payload).toEqual({
          sessionId: "checkout-session",
          nodeId: "node-old",
          mode: "transcript_only",
          expectedCursorVersion: 9
        });
        throw new Error("history cursor conflict: expected revision 9, actual revision 10");
      }

      throw new Error(`unexpected command: ${command}`);
    });

    const result = await store.checkoutHistoryNode("node-old");

    expect(result).toBeNull();
    expect(store.sessionError).toContain("history cursor conflict");
  });

  it("does not reapply latest session checkpoint over historical checkout snapshots", async () => {
    const store = useRuntimeStore();
    const historicalSnapshot = createSnapshot({
      conversationId: "checkout-session",
      summary: "Historical summary",
      history: [
        { role: "user", content: "old question" },
        { role: "assistant", content: "old answer" }
      ],
      turnTraceHistory: [
        createTrace({
          turnId: "turn-old",
          title: "old turn",
          phase: "completed",
          updatedAt: 2000
        })
      ],
      turnCount: 1,
      updatedAtMs: 2000
    });

    store.$patch({
      sessionId: "checkout-session",
      activeBranchId: "branch-main",
      branchHeadNodeId: "node-head",
      visibleNodeId: "node-head",
      cursorVersion: null,
      historyCursorMode: "live",
      historyBranches: [
        createHistoryBranch({
          branchId: "branch-main",
          sessionId: "checkout-session",
          headNodeId: "node-head"
        })
      ],
      historyNodes: [
        createHistoryNode({ nodeId: "node-old", sessionId: "checkout-session", turnId: "turn-old", createdAtMs: 2000 }),
        createHistoryNode({ nodeId: "node-head", sessionId: "checkout-session", turnId: "turn-latest", createdAtMs: 3200 })
      ],
      messages: [
        createMessage({ id: "user-latest", turnId: "turn-latest", role: "user", content: "latest question" }),
        createMessage({ id: "assistant-latest", turnId: "turn-latest", role: "assistant", content: "latest answer" })
      ],
      turnTraceHistory: [createTrace({ turnId: "turn-latest", title: "latest turn", updatedAt: 3200 })],
      latestExecutionCheckpoint: createCheckpoint({ turnId: "turn-latest", status: "running" })
    });

    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string, payload?: Record<string, unknown>) => {
      if (command === "checkout_history_node") {
        expect(payload).toEqual({
          sessionId: "checkout-session",
          nodeId: "node-old",
          mode: "transcript_only",
          expectedCursorVersion: null
        });
        return {
          sessionId: "checkout-session",
          nodeId: "node-old",
          requestedMode: "transcript_only",
          appliedMode: "transcript_only",
          transcriptRestoreApplied: true,
          workspaceRollbackCapable: false,
          workspaceRollbackApplied: false,
          degraded: false,
          degradationReason: null,
          cursor: createHistoryCursor({
            sessionId: "checkout-session",
            visibleNodeId: "node-old",
            activeBranchId: "branch-main",
            branchHeadNodeId: "node-head",
            workspaceNodeId: "node-old",
            mode: "historical"
          })
        };
      }

      if (command === "load_session_runtime_view") {
        expect(payload).toEqual({
          turnId: null,
          sessionId: "checkout-session",
          runId: null,
          nodeId: "node-old"
        });
        return createSessionRuntimeView(historicalSnapshot, {
          isAtBranchHead: false,
          checkpoint: createCheckpoint({
            turnId: "turn-latest",
            sessionId: "checkout-session",
            status: "running",
            checkpointKind: "runtime_control",
            phase: "calling_model"
          }),
          historyNodes: [
            createHistoryNode({ nodeId: "node-old", sessionId: "checkout-session", turnId: "turn-old", createdAtMs: 2000 }),
            createHistoryNode({ nodeId: "node-head", sessionId: "checkout-session", turnId: "turn-latest", createdAtMs: 3200 })
          ],
          historyBranches: [
            createHistoryBranch({ branchId: "branch-main", sessionId: "checkout-session", headNodeId: "node-head" })
          ],
          historyCursor: createHistoryCursor({
            sessionId: "checkout-session",
            visibleNodeId: "node-old",
            activeBranchId: "branch-main",
            branchHeadNodeId: "node-head",
            workspaceNodeId: "node-old",
            mode: "historical"
          })
        });
      }

      throw new Error(`unexpected command: ${command}`);
    });

    await store.checkoutHistoryNode("node-old");

    expect(store.messages.map((message) => message.turnId)).toEqual(["turn-old", "turn-old"]);
    expect(store.turnTraceHistory).toHaveLength(1);
    expect(store.turnTraceHistory[0]?.turnId).toBe("turn-old");
    expect(store.latestExecutionCheckpoint).toBeNull();
    expect(store.visibleNodeId).toBe("node-old");
    expect(store.historyCursorMode).toBe("historical");
  });

  it("clears browser-preview transcript when checking out the root node", async () => {
    const store = useRuntimeStore();
    tauriMocks.mockIsTauriAvailable.mockReturnValue(false);

    store.$patch({
      sessionId: "checkout-session",
      activeBranchId: "branch-main",
      branchHeadNodeId: "node-head",
      visibleNodeId: "node-head",
      historyCursorMode: "live",
      historyNodes: [
        createHistoryNode({ nodeId: "node-root", sessionId: "checkout-session", turnId: null, createdAtMs: 500 }),
        createHistoryNode({
          nodeId: "node-old",
          sessionId: "checkout-session",
          parentNodeId: "node-root",
          turnId: "turn-old",
          createdAtMs: 1000
        }),
        createHistoryNode({
          nodeId: "node-head",
          sessionId: "checkout-session",
          parentNodeId: "node-old",
          turnId: "turn-head",
          createdAtMs: 2000
        })
      ],
      historyBranches: [
        createHistoryBranch({ branchId: "branch-main", sessionId: "checkout-session", headNodeId: "node-head" })
      ],
      messages: [
        createMessage({ id: "user-old", turnId: "turn-old", role: "user", content: "old question" }),
        createMessage({ id: "assistant-old", turnId: "turn-old", role: "assistant", content: "old answer" }),
        createMessage({ id: "user-head", turnId: "turn-head", role: "user", content: "head question" }),
        createMessage({ id: "assistant-head", turnId: "turn-head", role: "assistant", content: "head answer" })
      ],
      turnTraceHistory: [
        createTrace({ turnId: "turn-old", title: "old turn", updatedAt: 1000 }),
        createTrace({ turnId: "turn-head", title: "head turn", updatedAt: 2000 })
      ]
    });

    const result = await store.checkoutHistoryNode("node-root", "transcript_only", "turn-old");

    expect(result).not.toBeNull();
    expect(result?.nodeId).toBe("node-root");
    expect(result?.appliedMode).toBe("transcript_only");
    expect(store.messages).toEqual([]);
    expect(store.turnTraceHistory).toEqual([]);
    expect(store.visibleNodeId).toBe("node-root");
    expect(store.historyCursorMode).toBe("historical");
  });

  it("does not merge persisted live messages back into a historical reload", async () => {
    const store = useRuntimeStore();
    const sessionId = "historical-reload-session";
    const historicalSnapshot = createSnapshot({
      conversationId: sessionId,
      summary: "Historical summary",
      history: [
        { role: "user", content: "old question" },
        { role: "assistant", content: "old answer" }
      ],
      turnTraceHistory: [
        createTrace({
          turnId: "turn-old",
          title: "old turn",
          phase: "completed",
          updatedAt: 2000
        })
      ],
      turnCount: 1,
      updatedAtMs: 2000
    });

    writePersistedSessions({
      [sessionId]: {
        phase: "ready",
        messages: [
          createMessage({ id: "user-old", turnId: "turn-old", role: "user", content: "old question" }),
          createMessage({ id: "assistant-old", turnId: "turn-old", role: "assistant", content: "old answer" }),
          createMessage({ id: "user-latest", turnId: "turn-latest", role: "user", content: "latest question" }),
          createMessage({
            id: "assistant-latest",
            turnId: "turn-latest",
            role: "assistant",
            content: "latest answer"
          })
        ],
        attachmentAssets: [],
        sessionSummary: "Live summary",
        providerRequestedName: "",
        providerName: "",
        providerProtocol: "",
        providerModel: "",
        providerSource: "",
        providerMode: "",
        fallbackReason: null,
        inputTokens: null,
        outputTokens: null,
        totalTokens: null,
        firstTokenLatencyMs: null,
        visibleNodeId: "node-head"
      }
    });

    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string, payload?: Record<string, unknown>) => {
      if (command === "load_session_runtime_view") {
        expect(payload).toEqual({
          turnId: null,
          sessionId,
          runId: null,
          nodeId: "node-old"
        });
        return createSessionRuntimeView(historicalSnapshot, {
          historyNodes: [
            createHistoryNode({ nodeId: "node-old", sessionId, turnId: "turn-old", createdAtMs: 2000 }),
            createHistoryNode({ nodeId: "node-head", sessionId, turnId: "turn-latest", createdAtMs: 3200 })
          ],
          historyBranches: [
            createHistoryBranch({ branchId: "branch-main", sessionId, headNodeId: "node-head" })
          ],
          historyCursor: createHistoryCursor({
            sessionId,
            visibleNodeId: "node-old",
            activeBranchId: "branch-main",
            branchHeadNodeId: "node-head",
            workspaceNodeId: "node-old",
            mode: "historical"
          })
        });
      }

      if (command === "list_sessions") {
        return [] satisfies SessionOverview[];
      }

      throw new Error(`unexpected command: ${command}`);
    });

    await store.loadSessionState(sessionId, { nodeId: "node-old" });

    expect(store.messages.map((message) => message.turnId)).toEqual(["turn-old", "turn-old"]);
    expect(store.messages.map((message) => message.content)).toEqual(["old question", "old answer"]);
    expect(store.turnTraceHistory).toHaveLength(1);
    expect(store.turnTraceHistory[0]?.turnId).toBe("turn-old");
    expect(store.historyCursorMode).toBe("historical");
  });

  it("ignores realtime turn events while viewing historical checkout", async () => {
    const store = useRuntimeStore();
    const eventHandlers = new Map<string, (event: { payload: TurnStreamEvent }) => void>();

    tauriMocks.mockSafeListen.mockImplementation(async (eventName: string, handler: (event: { payload: TurnStreamEvent }) => void) => {
      eventHandlers.set(eventName, handler);
      return () => {
        eventHandlers.delete(eventName);
      };
    });

    store.$patch({
      sessionId: "history-session",
      historyCursorMode: "historical",
      visibleNodeId: "node-old",
      branchHeadNodeId: "node-head",
      activeBranchId: "branch-main",
      activeTurnId: "turn-head",
      phase: "ready",
      messages: [
        createMessage({ id: "user-old", turnId: "turn-old", role: "user", content: "old question" }),
        createMessage({ id: "assistant-old", turnId: "turn-old", role: "assistant", content: "old answer" })
      ],
      turnTraceHistory: [createTrace({ turnId: "turn-old", title: "old turn", updatedAt: 2000 })]
    });

    await store.initializeTurnEvents();

    eventHandlers.get("turn:started")?.({
      payload: {
        turnId: "turn-head",
        eventId: "turn-head:1",
        eventType: "turn.started",
        eventVersion: "turn-event-v1",
        sequence: 1,
        emittedAtMs: 3000,
        phase: "calling_model",
        traceSteps: createCheckpoint({ turnId: "turn-head" }).traceSteps,
        toolActivities: []
      } as TurnStreamEvent
    });

    eventHandlers.get("turn:completed")?.({
      payload: {
        turnId: "turn-head",
        eventId: "turn-head:2",
        eventType: "turn.completed",
        eventVersion: "turn-event-v1",
        sequence: 2,
        emittedAtMs: 3200,
        phase: "completed",
        text: "latest answer",
        traceSteps: createCheckpoint({ turnId: "turn-head" }).traceSteps,
        toolActivities: [],
        providerCallRecords: [],
        hookTraceRecords: []
      } as TurnStreamEvent
    });

    expect(store.messages.map((message) => message.turnId)).toEqual(["turn-old", "turn-old"]);
    expect(store.messages.some((message) => message.turnId === "turn-head")).toBe(false);
    expect(store.turnTraceHistory).toHaveLength(1);
    expect(store.turnTraceHistory[0]?.turnId).toBe("turn-old");
  });

  it("treats historical_dirty as historical for cache merge and realtime gating", async () => {
    const store = useRuntimeStore();
    const sessionId = "historical-dirty-session";
    const eventHandlers = new Map<string, (event: { payload: TurnStreamEvent }) => void>();

    writePersistedSessions({
      [sessionId]: {
        phase: "ready",
        messages: [
          createMessage({ id: "user-old", turnId: "turn-old", role: "user", content: "old question" }),
          createMessage({ id: "assistant-old", turnId: "turn-old", role: "assistant", content: "old answer" }),
          createMessage({ id: "user-live", turnId: "turn-live", role: "user", content: "live question" }),
          createMessage({ id: "assistant-live", turnId: "turn-live", role: "assistant", content: "live answer" })
        ],
        attachmentAssets: [],
        sessionSummary: "Live summary",
        providerRequestedName: "",
        providerName: "",
        providerProtocol: "",
        providerModel: "",
        providerSource: "",
        providerMode: "",
        fallbackReason: null,
        inputTokens: null,
        outputTokens: null,
        totalTokens: null,
        firstTokenLatencyMs: null,
        visibleNodeId: "node-head"
      }
    });

    tauriMocks.mockSafeListen.mockImplementation(async (eventName: string, handler: (event: { payload: TurnStreamEvent }) => void) => {
      eventHandlers.set(eventName, handler);
      return () => {
        eventHandlers.delete(eventName);
      };
    });

    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string, payload?: Record<string, unknown>) => {
      if (command === "load_session_runtime_view") {
        expect(payload).toEqual({
          turnId: null,
          sessionId,
          runId: null,
          nodeId: "node-old"
        });
        const snapshot = createSnapshot({
          conversationId: sessionId,
          summary: "Historical summary",
          history: [
            { role: "user", content: "old question" },
            { role: "assistant", content: "old answer" }
          ],
          turnTraceHistory: [createTrace({ turnId: "turn-old", title: "old turn", updatedAt: 2000 })],
          turnCount: 1,
          updatedAtMs: 2000
        });
        return createSessionRuntimeView(snapshot, {
          historyCursor: createHistoryCursor({
            sessionId,
            visibleNodeId: "node-old",
            activeBranchId: "branch-main",
            branchHeadNodeId: "node-head",
            workspaceNodeId: "node-old",
            mode: "historical_dirty"
          })
        });
      }

      if (command === "list_sessions") {
        return [] satisfies SessionOverview[];
      }

      throw new Error(`unexpected command: ${command}`);
    });

    await store.loadSessionState(sessionId, { nodeId: "node-old" });
    await store.initializeTurnEvents();

    eventHandlers.get("turn:started")?.({
      payload: {
        turnId: "turn-live",
        eventId: "turn-live:1",
        eventType: "turn.started",
        eventVersion: "turn-event-v1",
        sequence: 1,
        emittedAtMs: 3000,
        phase: "calling_model",
        traceSteps: createCheckpoint({ turnId: "turn-live" }).traceSteps,
        toolActivities: []
      } as TurnStreamEvent
    });

    expect(store.historyCursorMode).toBe("historical_dirty");
    expect(store.messages.map((message) => message.turnId)).toEqual(["turn-old", "turn-old"]);
    expect(store.messages.some((message) => message.turnId === "turn-live")).toBe(false);
  });

  it("updates latest history audit summary from history-control responses", async () => {
    const store = useRuntimeStore();
    const restoreSnapshot = createSnapshot({
      conversationId: "audit-response-session",
      summary: "Audit response summary",
      history: [
        { role: "user", content: "latest question" },
        { role: "assistant", content: "latest answer" }
      ],
      turnCount: 1,
      updatedAtMs: 3601
    });
    const checkoutSummary = createHistoryStateAuditSummary({
      action: {
        commandKind: "checkout_history_node",
        boundary: "history.checkout.resolved",
        summary: "checkout summary has been projected"
      },
      currentContext: {
        mode: "historical",
        visibleNodeId: "node-old",
        activeBranchId: "branch-main",
        branchHeadNodeId: "node-head"
      }
    });
    const restoreSummary = createHistoryStateAuditSummary({
      action: {
        commandKind: "restore_branch_head",
        boundary: "history.restore.resolved",
        summary: "restore summary has been projected"
      },
      currentContext: {
        mode: "live",
        visibleNodeId: "node-head",
        activeBranchId: "branch-main",
        branchHeadNodeId: "node-head"
      }
    });

    store.$patch({
      sessionId: "audit-response-session",
      activeBranchId: "branch-main",
      branchHeadNodeId: "node-head",
      visibleNodeId: "node-head",
      historyCursorMode: "live",
      historyBranches: [
        createHistoryBranch({
          branchId: "branch-main",
          sessionId: "audit-response-session",
          headNodeId: "node-head"
        })
      ],
      historyNodes: [
        createHistoryNode({ nodeId: "node-old", sessionId: "audit-response-session", createdAtMs: 2000 }),
        createHistoryNode({ nodeId: "node-head", sessionId: "audit-response-session", createdAtMs: 3200 })
      ]
    });

    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string, payload?: Record<string, unknown>) => {
      if (command === "checkout_history_node") {
        expect(payload).toEqual({
          sessionId: "audit-response-session",
          nodeId: "node-old",
          mode: "transcript_only",
          expectedCursorVersion: null
        });
        return {
          sessionId: "audit-response-session",
          nodeId: "node-old",
          requestedMode: "transcript_only",
          appliedMode: "transcript_only",
          transcriptRestoreApplied: true,
          workspaceRollbackCapable: false,
          workspaceRollbackApplied: false,
          degraded: false,
          degradationReason: null,
          historyStateEvidence: null,
          historyStateAuditSummary: checkoutSummary,
          cursor: createHistoryCursor({
            sessionId: "audit-response-session",
            visibleNodeId: "node-old",
            activeBranchId: "branch-main",
            branchHeadNodeId: "node-head",
            workspaceNodeId: "node-old",
            mode: "historical"
          })
        };
      }

      if (command === "restore_branch_head") {
        expect(payload).toEqual({
          sessionId: "audit-response-session",
          branchId: "branch-main",
          expectedCursorVersion: null
        });
        return {
          sessionId: "audit-response-session",
          branchId: "branch-main",
          restoredNodeId: "node-head",
          transcriptRestoreApplied: true,
          workspaceRollbackCapable: false,
          workspaceRollbackApplied: false,
          degraded: false,
          degradationReason: null,
          historyStateEvidence: null,
          historyStateAuditSummary: restoreSummary,
          cursor: createHistoryCursor({
            sessionId: "audit-response-session",
            visibleNodeId: "node-head",
            activeBranchId: "branch-main",
            branchHeadNodeId: "node-head",
            workspaceNodeId: "node-head",
            mode: "live"
          })
        };
      }

      if (command === "load_session_runtime_view") {
        expect(payload).toEqual({
          turnId: null,
          sessionId: "audit-response-session",
          runId: null,
          nodeId: expect.any(String)
        });
        return createSessionRuntimeView(restoreSnapshot, {
          historyStateAuditSummary: restoreSummary
        });
      }

      throw new Error(`unexpected command: ${command}`);
    });

    const checkoutResult = await store.checkoutHistoryNode("node-old");
    expect(checkoutResult?.historyStateAuditSummary).toEqual(checkoutSummary);
    expect(store.latestHistoryStateAuditSummary).toEqual(checkoutSummary);

    const restoreResult = await store.restoreBranchHead();
    expect(restoreResult?.historyStateAuditSummary).toEqual(restoreSummary);
    expect(store.latestHistoryStateAuditSummary).toEqual(restoreSummary);
  });

  it("restores the branch head and exits historical mode", async () => {
    const store = useRuntimeStore();
    const snapshot = createSnapshot({
      conversationId: "restore-session",
      summary: "Restore summary",
      history: [
        { role: "user", content: "latest question" },
        { role: "assistant", content: "latest answer" }
      ],
      turnCount: 1,
      updatedAtMs: 3600
    });

    store.$patch({
      sessionId: "restore-session",
      activeBranchId: "branch-main",
      branchHeadNodeId: "node-head",
      visibleNodeId: "node-old",
      historyCursorMode: "historical",
      historyBranches: [
        createHistoryBranch({
          branchId: "branch-main",
          sessionId: "restore-session",
          headNodeId: "node-head"
        })
      ]
    });

    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string, payload?: Record<string, unknown>) => {
      if (command === "restore_branch_head") {
        expect(payload).toEqual({
          sessionId: "restore-session",
          branchId: "branch-main",
          expectedCursorVersion: null
        });
        return {
          sessionId: "restore-session",
          branchId: "branch-main",
          restoredNodeId: "node-head",
          transcriptRestoreApplied: true,
          workspaceRollbackCapable: false,
          workspaceRollbackApplied: false,
          degraded: false,
          degradationReason: null,
          historyStateEvidence: null,
          historyStateAuditSummary: null,
          cursor: createHistoryCursor({
            sessionId: "restore-session",
            visibleNodeId: "node-head",
            activeBranchId: "branch-main",
            branchHeadNodeId: "node-head",
            workspaceNodeId: "node-head",
            mode: "live"
          })
        };
      }

      if (command === "load_session_runtime_view") {
        expect(payload).toEqual({
          turnId: null,
          sessionId: "restore-session",
          runId: null,
          nodeId: "node-head"
        });
        return createSessionRuntimeView(snapshot);
      }

      throw new Error(`unexpected command: ${command}`);
    });

    const result = await store.restoreBranchHead();

    expect(result?.restoredNodeId).toBe("node-head");
    expect(result?.restoredFromNodeId).toBe("node-head");
    expect(result?.transcriptRestoreApplied).toBe(true);
    expect(result?.workspaceRestoreCapable).toBe(false);
    expect(result?.workspaceRestoreApplied).toBe(false);
    expect(result?.degradedToTranscriptOnly).toBe(false);
    expect(store.visibleNodeId).toBe("node-head");
    expect(store.historyCursorMode).toBe("live");
    expect(store.isHistoricalMode).toBe(false);
  });

  it("disables restore, fork, and branch switching in browser-preview degraded mode", async () => {
    const store = useRuntimeStore();
    tauriMocks.mockIsTauriAvailable.mockReturnValue(false);

    store.$patch({
      sessionId: "fork-session",
      activeBranchId: "branch-main",
      branchHeadNodeId: "node-head",
      visibleNodeId: "node-old",
      historyCursorMode: "historical",
      historyNodes: [
        createHistoryNode({ nodeId: "node-old", sessionId: "fork-session", createdAtMs: 2000 }),
        createHistoryNode({ nodeId: "node-head", sessionId: "fork-session", createdAtMs: 3000 })
      ],
      historyBranches: [
        createHistoryBranch({
          branchId: "branch-main",
          sessionId: "fork-session",
          baseNodeId: "node-old",
          headNodeId: "node-head"
        })
      ],
      messages: [
        createMessage({ turnId: "turn-1", role: "user", content: "fork point" }),
        createMessage({ turnId: "turn-1", role: "assistant", content: "fork answer" })
      ]
    });

    const forkResult = await store.forkHistoryNode("node-old");
    const switchResult = await store.switchHistoryBranch("branch-main");
    const restoreResult = await store.restoreBranchHead();

    expect(forkResult).toBeNull();
    expect(switchResult).toBeNull();
    expect(restoreResult).toBeNull();
    expect(store.activeBranchId).toBe("branch-main");
    expect(store.visibleNodeId).toBe("node-old");
    expect(store.branchHeadNodeId).toBe("node-head");
    expect(store.historyCursorMode).toBe("historical");
    expect(store.sessionError).toContain("browser preview / local preview");
  });

  it("derives conversation checkpoint entries from explicit turn ids and trace-backed turn ids", () => {
    const store = useRuntimeStore();
    store.$patch({
      activeBranchId: "branch-main",
      visibleNodeId: "node-head",
      branchHeadNodeId: "node-head",
      historyNodes: [
        createHistoryNode({
          nodeId: "node-old",
          sessionId: "checkpoint-session",
          branchId: "branch-main",
          turnId: "turn-old",
          summary: "旧 checkpoint",
          workspaceRef: { kind: "host_snapshot", rollbackCapable: false },
          createdAtMs: 1000
        }),
        createHistoryNode({
          nodeId: "node-fork-source",
          sessionId: "checkpoint-session",
          branchId: "branch-main",
          summary: "会产生 fork 的 checkpoint",
          workspaceRef: { kind: "host_snapshot", rollbackCapable: true },
          turnTraceHistory: [createTurnTraceRecord({ turnId: "turn-fork-source", updatedAt: 2000 })],
          createdAtMs: 2000
        }),
        createHistoryNode({
          nodeId: "node-head",
          sessionId: "checkpoint-session",
          branchId: "branch-main",
          turnId: "turn-head",
          summary: "最新 checkpoint",
          workspaceRef: { kind: "host_snapshot", rollbackCapable: true },
          createdAtMs: 3000
        }),
        createHistoryNode({
          nodeId: "node-unmapped",
          sessionId: "checkpoint-session",
          branchId: "branch-main",
          turnId: null,
          turnTraceHistory: [],
          summary: "不应进入消息级 affordance",
          createdAtMs: 4000
        }),
        createHistoryNode({
          nodeId: "node-fork-target",
          sessionId: "checkpoint-session",
          branchId: "branch-fork",
          turnId: "turn-fork-target",
          summary: "fork 目标 checkpoint",
          createdAtMs: 2500
        })
      ],
      historyBranches: [
        createHistoryBranch({
          branchId: "branch-main",
          sessionId: "checkpoint-session",
          headNodeId: "node-head",
          label: "main"
        }),
        createHistoryBranch({
          branchId: "branch-fork",
          sessionId: "checkpoint-session",
          baseNodeId: "node-fork-source",
          headNodeId: "node-fork-target",
          forkedFromNodeId: "node-fork-source",
          label: "fork-1"
        })
      ]
    });

    const entries = store.conversationCheckpointEntries;

    expect(entries.map((entry) => entry.nodeId)).toEqual(["node-head", "node-fork-target", "node-fork-source", "node-old"]);
    expect(entries.find((entry) => entry.nodeId === "node-unmapped")).toBeUndefined();
    expect(entries.find((entry) => entry.nodeId === "node-old")?.availableModes).toEqual(["transcript_only"]);
    expect(entries.find((entry) => entry.nodeId === "node-fork-source")?.turnId).toBe("turn-fork-source");
    expect(entries.find((entry) => entry.nodeId === "node-head")?.isLatest).toBe(true);
    expect(entries.find((entry) => entry.nodeId === "node-head")?.isVisible).toBe(true);
    expect(entries.find((entry) => entry.nodeId === "node-fork-source")?.forkTargets).toEqual([
      {
        branchId: "branch-fork",
        nodeId: "node-fork-target",
        label: "fork-1",
        summary: "fork 目标 checkpoint",
        isActive: false
      }
    ]);
  });

  it("keeps checkpoint entries empty when no stable turn mapping exists", () => {
    const store = useRuntimeStore();
    store.$patch({
      activeBranchId: "branch-main",
      visibleNodeId: "node-unmapped",
      branchHeadNodeId: "node-unmapped",
      historyNodes: [
        createHistoryNode({
          nodeId: "node-unmapped",
          sessionId: "checkpoint-empty-session",
          branchId: "branch-main",
          turnId: null,
          turnTraceHistory: [],
          summary: "缺少映射"
        })
      ],
      historyBranches: [
        createHistoryBranch({
          branchId: "branch-main",
          sessionId: "checkpoint-empty-session",
          headNodeId: "node-unmapped",
          label: "main"
        })
      ]
    });

    expect(store.conversationCheckpointEntries).toEqual([]);
  });

  it("initializes frontend recorder state on health fetch", async () => {
    const store = useRuntimeStore();
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "health_check") {
        return {
          appName: "Pony Agent",
          appVersion: "0.1.1",
          runtime: "desktop",
          graphEngine: "graph",
          graphContractVersion: "v1"
        };
      }
      throw new Error(`unexpected command: ${command}`);
    });

    await store.fetchHealth();

    const capability = getFrontendRecorderCapability();
    const stats = getFrontendRecorderStats();
    expect(capability.tauriAvailable).toBe(true);
    expect(stats.initialized).toBe(true);
  });
});
