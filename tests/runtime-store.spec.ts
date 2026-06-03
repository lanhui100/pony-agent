import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import type {
  AttachmentAsset,
  BuildContextObservation,
  ChatMessage,
  ExecutionCheckpoint,
  HistoryBranch,
  HistoryCursorState,
  HistoryNode,
  ProviderCallCacheRecord,
  RetrievedContextState,
  SessionOverview,
  SessionSnapshot,
  SessionRuntimeView,
  TurnTraceRecord
} from "@/types/runtime";
import { useRuntimeStore } from "@/stores/runtime";

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
    durationSeconds: partial.durationSeconds ?? null
  };
}

function createTrace(partial: Partial<TurnTraceRecord> = {}): TurnTraceRecord {
  return {
    turnId: partial.turnId ?? "turn-1",
    title: partial.title ?? "test turn",
    phase: partial.phase ?? "completed",
    traceSteps: partial.traceSteps ?? [],
    traceTimeline: partial.traceTimeline ?? [],
    toolActivities: partial.toolActivities ?? [],
    providerCallRecords: partial.providerCallRecords ?? [],
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
    turnId: partial.turnId ?? "turn-1",
    sessionId: partial.sessionId ?? "session-1",
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
    transcriptRef: partial.transcriptRef ?? null,
    runRef: partial.runRef ?? null,
    workspaceRef: partial.workspaceRef ?? null,
    summary: partial.summary ?? "History node",
    createdAtMs: partial.createdAtMs ?? 1000
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
    mode: partial.mode ?? "live"
  };
}

function createSessionRuntimeView(
  session: SessionSnapshot,
  partial: Partial<SessionRuntimeView> = {}
): SessionRuntimeView {
  return {
    session,
    retrieved: partial.retrieved ?? createRetrievedContext(session),
    checkpoint: partial.checkpoint ?? null,
    historyNodes: partial.historyNodes ?? [],
    historyBranches: partial.historyBranches ?? [],
    historyCursor: partial.historyCursor ?? null
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

describe("runtime session resilience", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    window.localStorage.clear();
    vi.spyOn(console, "info").mockImplementation(() => {});
    tauriMocks.mockSafeListen.mockResolvedValue(() => {});
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    setActivePinia(createPinia());
  });

  it("restores the current session when switching fails", async () => {
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

    expect(store.sessionId).toBe("session-current");
    expect(store.phase).toBe("ready");
    expect(store.draftMessage).toBe("draft message");
    expect(store.messages).toEqual(originalMessages);
    expect(store.sessionList).toEqual(originalSessionList);
    expect(store.sessionOperation).toBeNull();
    expect(store.sessionError).toContain("snapshot exploded");
  });

  it("keeps the current session content visible until the next session finishes loading", async () => {
    const store = useRuntimeStore();
    const originalMessages = [
      createMessage({ id: "user-current", turnId: "turn-current", role: "user", content: "current session" }),
      createMessage({
        id: "assistant-current",
        turnId: "turn-current",
        role: "assistant",
        content: "current reply"
      })
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

    expect(store.sessionOperation).toBe("switching");
    expect(store.sessionId).toBe("session-current");
    expect(store.phase).toBe("ready");
    expect(store.messages).toEqual(originalMessages);

    resolveRuntimeView?.(createSessionRuntimeView(nextSnapshot));
    await switchingPromise;

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
    expect(store.sessionError).toContain("delete backend failed");
    expect(readPersistedSessions().sessions["session-delete"]).toBeTruthy();
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
        return [] satisfies SessionOverview[];
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
            phase: "queued",
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
                status: "running",
                summary: "Reading workspace"
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

    await store.stopTurn();

    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledWith("stop_graph_run", {
      runId: "run-running"
    });
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
          capabilityId: "builtin:time_now"
        });
        return {
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
    const capability = await store.inspectCapability("builtin:time_now");
    const source = await store.inspectCapabilitySource("builtin-tools");

    expect(store.capabilitySources).toHaveLength(1);
    expect(store.capabilities.map((item) => item.capabilityId)).toEqual(["builtin:time_now"]);
    expect(capability?.capabilityId).toBe("builtin:time_now");
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
    const capability = await store.inspectCapability("builtin:time_now");
    const source = await store.inspectCapabilitySource("builtin-tools");

    expect(store.capabilitySources.map((item) => item.sourceId)).toEqual(["builtin-tools"]);
    expect(store.capabilities.some((item) => item.capabilityId === "builtin:time_now")).toBe(true);
    expect(capability?.capabilityId).toBe("builtin:time_now");
    expect(source?.sourceId).toBe("builtin-tools");
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
        conversationId: "browser-current",
        title: "browser message",
        summary: "Browser summary",
        turnCount: 1,
        lastReferencedFile: null,
        updatedAtMs: 4000
      }
    ]);
    expect(Object.keys(readPersistedSessions().sessions)).toEqual(["browser-current"]);

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
    expect(store.turnTraceHistory[0]?.fallbackReason).toContain("npm run dev");

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

  it("refreshes host retrieval before falling back to inspect_host when local runState is insufficient", async () => {
    const store = useRuntimeStore();
    const nowSpy = vi.spyOn(Date, "now").mockReturnValue(8585);

    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string, payload?: Record<string, unknown>) => {
      if (command === "load_retrieved_context") {
        expect(payload).toEqual({
          sessionId: "retrieval-refresh-session",
          runId: null,
          turnId: null
        });
        return {
          ...createRetrievedContext(
            createSnapshot({
              conversationId: "retrieval-refresh-session",
              summary: "retrieval refresh summary"
            })
          ),
          runState: {
            runId: "run-refreshed",
            phase: "waiting_user",
            goal: "continue after host retrieval refresh"
          }
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
    expect(store.retrievedContext?.runState.runId).toBe("run-refreshed");
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
      if (command === "load_retrieved_context") {
        expect(payload).toEqual({
          sessionId: "fresh-run-session",
          runId: null,
          turnId: null
        });
        return {
          ...createRetrievedContext(
            createSnapshot({
              conversationId: "fresh-run-session",
              summary: "fresh retrieval summary"
            })
          ),
          runState: {}
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
    expect(tauriMocks.mockSafeInvoke).not.toHaveBeenCalledWith("inspect_host", expect.anything());
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

      if (command === "start_graph_run_stream") {
        expect(payload).toEqual({
          turnId: "8282",
          runId: null,
          goal: "new request",
          input: expect.objectContaining({
            history: [
              {
                role: "user",
                content: "look at these files",
                attachments: [
                  expect.objectContaining({
                    id: "real-1",
                    relativePath: "uploads/demo.png"
                  })
                ]
              },
              {
                role: "assistant",
                content: "ready for the next step",
                attachments: []
              }
            ]
          })
        });
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

    eventHandlers.get("turn:started")?.({
      payload: {
        turnId: "6060",
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

    expect(store.turnTraceHistory[0]?.buildContextObservation).toEqual(startedObservation);
    expect(store.turnTraceHistory[0]?.buildContextObservation).not.toBe(startedObservation);
    expect(store.turnTraceHistory[0]?.traceTimeline?.map((entry) => entry.kind)).toEqual([
      "input",
      "prepare_retrieval",
      "build_context",
      "call_model"
    ]);

    eventHandlers.get("turn:delta")?.({
      payload: {
        turnId: "6060",
        text: "partial answer",
        reasoningContent: "thinking",
        firstTokenLatencyMs: 321
      }
    });

    expect(store.turnTraceHistory[0]?.firstTokenLatencyMs).toBe(321);
    expect(
      store.turnTraceHistory[0]?.traceTimeline?.find((entry) => entry.kind === "call_model" && entry.text)?.text
    ).toBeUndefined();

    eventHandlers.get("turn:completed")?.({
      payload: {
        turnId: "6060",
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
        firstTokenLatencyMs: 321,
        turnDurationMs: 2800,
        traceSteps: store.traceSteps,
        toolActivities: [],
        buildContextObservation: completedObservation
      }
    } as any);

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
    expect(store.turnTraceHistory[0]?.buildContextObservation).toEqual(completedObservation);
    expect(store.turnTraceHistory[0]?.buildContextObservation).not.toBe(completedObservation);
    expect(store.turnTraceHistory[0]?.cacheHitInputTokens).toBe(5);
    expect(store.turnTraceHistory[0]?.reasoningTokens).toBe(8);
    expect(store.turnTraceHistory[0]?.turnDurationMs).toBe(2800);
    expect(store.turnTraceHistory[0]?.traceTimeline?.at(-1)?.label).toBe("CALL MODEL #1");
    expect(store.turnTraceHistory[0]?.traceTimeline?.at(-1)?.outputTokens).toBe(34);

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

  it("prefers accumulated cache-hit tokens over nested cached token details on completion", async () => {
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

    await flushMicrotasks();

    expect(store.turnTraceHistory).toHaveLength(1);
    expect(store.turnTraceHistory[0]?.cacheHitInputTokens).toBe(70);
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

    await flushMicrotasks();

    expect(store.turnTraceHistory[0]?.providerCallRecords?.map((record) => record.requestKind)).toEqual([
      "initial_request",
      "tool_followup"
    ]);
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
          mode: "transcript_and_workspace"
        });
        return {
          sessionId: "checkout-session",
          visibleNodeId: "node-old",
          activeBranchId: "branch-main",
          branchHeadNodeId: "node-head",
          workspaceNodeId: "node-old",
          mode: "historical",
          requestedMode: "transcript_and_workspace",
          appliedMode: "transcript_only",
          workspaceRestoreApplied: false,
          degradedToTranscriptOnly: true
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

    expect(result?.degradedToTranscriptOnly).toBe(true);
    expect(store.visibleNodeId).toBe("node-old");
    expect(store.branchHeadNodeId).toBe("node-head");
    expect(store.activeBranchId).toBe("branch-main");
    expect(store.historyCursorMode).toBe("historical");
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
          branchId: "branch-main"
        });
        return {
          sessionId: "restore-session",
          visibleNodeId: "node-head",
          activeBranchId: "branch-main",
          branchHeadNodeId: "node-head",
          workspaceNodeId: "node-head",
          mode: "live",
          restoredFromNodeId: "node-old"
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

    expect(result?.restoredFromNodeId).toBe("node-old");
    expect(store.visibleNodeId).toBe("node-head");
    expect(store.historyCursorMode).toBe("live");
    expect(store.isHistoricalMode).toBe(false);
  });

  it("supports local fork creation and branch switching before backend fields land", async () => {
    const store = useRuntimeStore();
    const nowSpy = vi.spyOn(Date, "now").mockReturnValue(5151);
    const originalIsTauriAvailable = tauriMocks.mockIsTauriAvailable;
    originalIsTauriAvailable.mockReturnValue(false);

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

    expect(forkResult?.createdBranchId).toBe("branch-5151");
    expect(store.activeBranchId).toBe("branch-5151");
    expect(store.branchHeadNodeId).toBe("node-old");
    expect(store.historyCursorMode).toBe("live");
    expect(store.historyBranches.map((branch) => branch.branchId)).toContain("branch-5151");

    const switchResult = await store.switchHistoryBranch("branch-main");

    expect(switchResult?.previousBranchId).toBe("branch-5151");
    expect(store.activeBranchId).toBe("branch-main");
    expect(store.visibleNodeId).toBe("node-head");
    expect(store.branchHeadNodeId).toBe("node-head");
    expect(store.historyCursorMode).toBe("live");
    nowSpy.mockRestore();
  });
});
