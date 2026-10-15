/**
 * Wave-12.1 验收契约 — 冷启动落点不得为已归档会话（C8 / A8 / A9）
 *
 * 权威契约：`.dev-team/contracts/wave-12-session-fallback.md` §6（契约演进 wave-12.1）
 *   C8：冷启动/初始化选择落点会话时必须跳过 `archived === true`：
 *       存在未归档会话 → 选其中 `updatedAtMs` 最大者；全部已归档或无会话 → 进入全新空白会话
 *       （`phase === "idle"`、`messages` 为空），不得打开任何已归档会话。
 *
 * 根因（Lead 真实浏览器复现 `.dev-team/evidence/repro-cold-start-archived.mjs`）：
 *   `src/stores/runtime.ts:2451` 与 `src/lib/runtime/sessions.ts:661` 直接取 `sessionList[0]`，
 *   而 catalog 含 archived 项且按 `updatedAtMs` 降序。
 */
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type {
  ChatMessage,
  RetrievedContextState,
  SessionOverview,
  SessionRuntimeView,
  SessionSnapshot
} from "@/types/runtime";
import { deriveInitStrategy } from "@/lib/runtime/sessions";
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

const ARCHIVED_A = "session-archived-alpha";
const ARCHIVED_C = "session-archived-charlie";
const ACTIVE_B = "session-active-bravo";

const A_MESSAGES = ["A 已归档会话的问题", "A 已归档会话的回答"];
const B_MESSAGES = ["B 未归档会话的问题", "B 未归档会话的回答"];

function createMessage(id: string, role: "user" | "assistant", content: string, turnId: string): ChatMessage {
  return {
    id,
    turnId,
    role,
    content,
    attachments: [],
    status: "done",
    tokenCount: null,
    reasoningContent: null,
    modelName: null,
    toolName: null,
    detail: null,
    durationSeconds: null,
    errorDetail: null
  };
}

function turnMessages(turnId: string, contents: string[]): ChatMessage[] {
  return contents.map((content, index) =>
    createMessage(`${turnId}-${index}`, index % 2 === 0 ? "user" : "assistant", content, turnId)
  );
}

/** 持久化会话载荷（非 Tauri 分支的 catalog 由它派生；updatedAtMs 由 turnTraceHistory 末项决定）。 */
function persistedSession(contents: string[], options: { updatedAtMs: number; archived?: boolean }) {
  const turnId = `turn-${options.updatedAtMs}`;
  return {
    messages: turnMessages(turnId, contents),
    turnTraceHistory: [
      {
        turnId,
        sessionId: null,
        eventId: null,
        eventType: null,
        eventVersion: null,
        sequence: null,
        emittedAtMs: null,
        title: "cold-start turn",
        phase: "completed",
        traceSteps: [],
        traceTimeline: [],
        toolActivities: [],
        providerCallRecords: [],
        hookTraceRecords: [],
        providerRequestedName: null,
        providerName: null,
        providerProtocol: null,
        providerModel: null,
        providerSource: null,
        providerMode: null,
        buildContextObservation: null,
        sessionSummary: contents[0],
        fallbackReason: null,
        error: null,
        inputTokens: null,
        cacheHitInputTokens: null,
        reasoningTokens: null,
        outputTokens: null,
        totalTokens: null,
        firstTokenLatencyMs: null,
        turnDurationMs: null,
        updatedAt: options.updatedAtMs
      }
    ],
    sessionSummary: contents[0],
    phase: "ready",
    archived: options.archived ?? false
  };
}

function writePersistedSessions(payload: Record<string, unknown>) {
  window.localStorage.setItem(RUNTIME_STORAGE_KEY, JSON.stringify({ sessions: payload }));
}

function overview(conversationId: string, updatedAtMs: number, archived: boolean): SessionOverview {
  return {
    conversationId,
    title: conversationId,
    summary: conversationId,
    turnCount: 1,
    lastReferencedFile: null,
    updatedAtMs,
    workspaceId: null,
    archived
  };
}

function createSnapshot(conversationId: string, contents: string[]): SessionSnapshot {
  return {
    conversationId,
    title: conversationId,
    summary: conversationId,
    history: contents.map((content, index) => ({
      role: index % 2 === 0 ? "user" : "assistant",
      content
    })),
    attachmentAssets: [],
    historyStateAuditSummary: null,
    runControlAuditSummary: null,
    turnTraceHistory: [],
    turnCount: contents.length,
    lastReferencedFile: null,
    updatedAtMs: 1000
  };
}

function createRetrieved(snapshot: SessionSnapshot): RetrievedContextState {
  return {
    turnContext: { userMessage: "", images: [], referencesImage: false },
    sessionContext: {
      conversationId: snapshot.conversationId,
      title: snapshot.title ?? "Session",
      summary: snapshot.summary,
      recentHistory: [],
      recentAttachmentAssets: [],
      turnCount: snapshot.turnCount,
      lastReferencedFile: null
    },
    runState: {},
    longTermMemory: { status: "empty", summary: "", entries: [] },
    transcript: { providerNativeMessages: [] }
  };
}

function runtimeViewFor(conversationId: string, contents: string[]): SessionRuntimeView {
  const session = createSnapshot(conversationId, contents);
  return {
    session,
    retrieved: createRetrieved(session),
    checkpoint: null,
    submissionPlan: null,
    controlBoundaryEvidence: null,
    historyStateAuditSummary: null,
    runControlAuditSummary: null,
    historyNodes: [],
    historyBranches: [],
    historyCursor: null,
    authorityMode: "host_authoritative",
    resolvedVisibleNodeId: null,
    activeBranchHeadNodeId: null,
    isAtBranchHead: true,
    cursorVersion: null
  };
}

function mockTauriCatalog(catalog: SessionOverview[]) {
  tauriMocks.mockSafeInvoke.mockImplementation(async (command: string, payload?: any) => {
    if (command === "list_sessions") return catalog;
    if (command === "load_session_runtime_view") {
      const sessionId = String(payload?.sessionId ?? "");
      const contents =
        sessionId === ACTIVE_B ? B_MESSAGES : sessionId === ARCHIVED_A ? A_MESSAGES : [];
      return runtimeViewFor(sessionId, contents);
    }
    throw new Error(`unexpected command: ${command}`);
  });
}

beforeEach(() => {
  vi.clearAllMocks();
  window.localStorage.clear();
  vi.spyOn(console, "info").mockImplementation(() => {});
  vi.spyOn(console, "debug").mockImplementation(() => {});
  tauriMocks.mockSafeListen.mockResolvedValue(() => {});
  tauriMocks.mockIsTauriAvailable.mockReturnValue(false);
  setActivePinia(createPinia());
});

describe("Wave-12.1 冷启动落点契约（C8 / A8 / A9）", () => {
  it("A8/C8 非 Tauri 冷启动：catalog=[已归档 A(updatedAt 更高, 有本地持久化), 未归档 B] → 落点必须是 B", async () => {
    writePersistedSessions({
      [ARCHIVED_A]: persistedSession(A_MESSAGES, { updatedAtMs: 5000, archived: true }),
      [ACTIVE_B]: persistedSession(B_MESSAGES, { updatedAtMs: 1000 })
    });

    const store = useRuntimeStore();
    await store.initializeSessions();

    expect(store.sessionId, "冷启动必须跳过已归档会话").toBe(ACTIVE_B);
    expect(store.messages.map((message) => message.content)).toEqual(B_MESSAGES);
    expect(store.sessionId).not.toBe(ARCHIVED_A);
  });

  it("A8/C8 Tauri 冷启动（已归档 A 无本地持久化 → deriveInitStrategy 的 persisted==null 分支）→ 落点必须是 B", async () => {
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    // A 无本地持久化 → loadPersistedRuntimeState(A) === null → 走 deriveInitStrategy 的 sessionList[0] 分支
    writePersistedSessions({});
    mockTauriCatalog([overview(ARCHIVED_A, 5000, true), overview(ACTIVE_B, 1000, false)]);

    const store = useRuntimeStore();
    await store.initializeSessions();

    expect(store.sessionId, "persisted==null 分支同样必须跳过已归档会话").toBe(ACTIVE_B);
    expect(store.messages.map((message) => message.content)).toEqual(B_MESSAGES);
  });

  it("A8/C8 deriveInitStrategy 纯函数（persisted==null）：存在未归档会话时必须选它，而非 sessionList[0]", () => {
    const catalog: SessionOverview[] = [overview(ARCHIVED_A, 5000, true), overview(ACTIVE_B, 1000, false)];

    const strategy = deriveInitStrategy(ARCHIVED_A, null, catalog);

    expect(strategy.sessionId, "不得把已归档的 sessionList[0] 当作落点").toBe(ACTIVE_B);
    expect([ARCHIVED_A, ARCHIVED_C]).not.toContain(strategy.sessionId);
  });

  it("A9/C8 非 Tauri 冷启动：catalog 全为 archived → 全新空白会话（不得打开任何已归档会话）", async () => {
    writePersistedSessions({
      [ARCHIVED_A]: persistedSession(A_MESSAGES, { updatedAtMs: 5000, archived: true }),
      [ARCHIVED_C]: persistedSession(["C 已归档会话的问题", "C 已归档会话的回答"], {
        updatedAtMs: 4000,
        archived: true
      })
    });

    const store = useRuntimeStore();
    await store.initializeSessions();

    expect(store.messages).toEqual([]);
    expect([ARCHIVED_A, ARCHIVED_C]).not.toContain(store.sessionId);
    expect(store.phase).toBe("idle");
  });

  it("A9/C8 Tauri 冷启动：catalog 全为 archived 且无本地持久化 → 全新空白会话", async () => {
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    writePersistedSessions({});
    mockTauriCatalog([overview(ARCHIVED_A, 5000, true), overview(ARCHIVED_C, 4000, true)]);

    const store = useRuntimeStore();
    await store.initializeSessions();

    expect(store.messages).toEqual([]);
    expect([ARCHIVED_A, ARCHIVED_C]).not.toContain(store.sessionId);
    expect(store.phase).toBe("idle");
  });

  it("A9 边界：无任何会话时冷启动 → 空态 idle（守卫，基线与演进后均须成立）", async () => {
    writePersistedSessions({});

    const store = useRuntimeStore();
    await store.initializeSessions();

    expect(store.messages).toEqual([]);
    expect(store.phase).toBe("idle");
  });
});
