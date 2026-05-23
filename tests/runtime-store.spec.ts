import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import type { ChatMessage, SessionOverview, SessionSnapshot, TurnTraceRecord } from "@/types/runtime";
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
    toolActivities: partial.toolActivities ?? [],
    providerRequestedName: partial.providerRequestedName ?? null,
    providerName: partial.providerName ?? null,
    providerProtocol: partial.providerProtocol ?? null,
    providerModel: partial.providerModel ?? null,
    providerSource: partial.providerSource ?? null,
    providerMode: partial.providerMode ?? null,
    sessionSummary: partial.sessionSummary ?? "",
    fallbackReason: partial.fallbackReason ?? null,
    error: partial.error ?? null,
    inputTokens: partial.inputTokens ?? null,
    outputTokens: partial.outputTokens ?? null,
    totalTokens: partial.totalTokens ?? null,
    firstTokenLatencyMs: partial.firstTokenLatencyMs ?? null,
    updatedAt: partial.updatedAt ?? 1000
  };
}

function createSnapshot(partial: Partial<SessionSnapshot> = {}): SessionSnapshot {
  return {
    conversationId: partial.conversationId ?? "session-1",
    title: partial.title ?? "Session 1",
    summary: partial.summary ?? "Session summary",
    history: partial.history ?? [],
    turnCount: partial.turnCount ?? 0,
    lastReferencedFile: partial.lastReferencedFile ?? null,
    updatedAtMs: partial.updatedAtMs ?? 1000
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
      if (command === "load_session_snapshot") {
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

      if (command === "load_session_snapshot") {
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

      if (command === "load_session_snapshot") {
        return fallbackSnapshot;
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
          { role: "assistant", content: "# 新标题\n\n> 新引用\n\n**已修复**" }
        ],
        turnCount: 1
      })
    );

    expect(store.messages.map((message) => `${message.role}:${message.content}`)).toEqual([
      "user:请整理 markdown",
      "assistant:# 新标题\n\n> 新引用\n\n**已修复**",
      "tool:{\"ok\":true}"
    ]);
    expect(store.messages[1]?.reasoningContent).toBe("旧思考");
    expect(store.messages[1]?.modelName).toBe("ppx/gpt-5.4");
    expect(store.messages[1]?.tokenCount).toBe(128);
    expect(store.messages[2]?.turnId).toBe("turn-restore");

    const persisted = readPersistedSessions().sessions["session-restore"] as {
      messages: ChatMessage[];
    };
    expect(persisted.messages[1]?.content).toBe("# 新标题\n\n> 新引用\n\n**已修复**");
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

      if (command === "load_session_snapshot") {
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

  it("marks the turn as failed when start_turn_stream throws immediately", async () => {
    const store = useRuntimeStore();
    const nowSpy = vi.spyOn(Date, "now").mockReturnValue(9090);

    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "start_turn_stream") {
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

    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeListen.mockImplementation(async (eventName: string, handler: unknown) => {
      eventHandlers.set(eventName, handler as (event: { payload: Record<string, unknown> }) => void);
      return () => {};
    });
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "start_turn_stream") {
        return null;
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
        traceSteps: store.traceSteps
      }
    });

    eventHandlers.get("turn:delta")?.({
      payload: {
        turnId: "6060",
        text: "partial answer",
        reasoningContent: "thinking",
        firstTokenLatencyMs: 321
      }
    });

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
        firstTokenLatencyMs: 321,
        traceSteps: store.traceSteps,
        toolActivities: []
      }
    });

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
      if (command === "start_turn_stream") {
        return null;
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

    nowSpy.mockRestore();
  });
});
