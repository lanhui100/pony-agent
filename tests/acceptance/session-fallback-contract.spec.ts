/**
 * Wave-12 验收契约 — 删除/归档「当前对话」后的落点（PA-SESSION-FALLBACK）
 *
 * 权威契约：`.dev-team/contracts/wave-12-session-fallback.md`
 *   C1 归档当前会话 → 全新空白会话；C2 删除当前会话 → 全新空白会话 + 被删项不复活；
 *   C3 非当前会话不导航；C4 归档内容保全；C5 双路径（Tauri / 本地）一致；C6 失败回滚；C7 空会话边界。
 * 断言映射：A1=C1、A2=C2、A3=C3、A4=C4、A5=C5、A6=C6（A7 为真实浏览器链路，见 tests/e2e/session-fallback.spec.ts）。
 *
 * 用户裁定：方案 A —— 删除/归档当前对话后**一律进入全新空白会话**，
 * 不得切换到任何历史会话（含已归档的）。
 */
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ChatMessage, SessionOverview } from "@/types/runtime";
import { DEFAULT_SESSION_ID } from "@/lib/runtime/constants";
import { loadPersistedRuntimeState } from "@/lib/runtime/sessions";
import { DEFAULT_WORKSPACE_ID } from "@/lib/runtime/workspace-constants";
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

const CURRENT_ID = "session-current";
const OTHER_ID = "session-other";
const ARCHIVED_ID = "session-archived";

interface Invocation {
  command: string;
  payload?: unknown;
}

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

function overview(conversationId: string, partial: Partial<SessionOverview> = {}): SessionOverview {
  return {
    conversationId,
    title: `Title ${conversationId}`,
    summary: "",
    turnCount: 1,
    lastReferencedFile: null,
    updatedAtMs: 1000,
    workspaceId: null,
    archived: false,
    ...partial
  };
}

function writePersistedSessions(payload: Record<string, unknown>) {
  window.localStorage.setItem(RUNTIME_STORAGE_KEY, JSON.stringify({ sessions: payload }));
}

function flushMicrotasks() {
  return new Promise<void>((resolve) => window.setTimeout(resolve, 0));
}

function seedCatalog(): SessionOverview[] {
  return [
    overview(CURRENT_ID, { updatedAtMs: 3000, title: "Current" }),
    overview(OTHER_ID, { updatedAtMs: 2000, title: "Other" }),
    overview(ARCHIVED_ID, { updatedAtMs: 1000, title: "Archived", archived: true })
  ];
}

function seedCurrentSession(messages: ChatMessage[] = [createMessage({ id: "current-user", content: "当前会话内容" })]) {
  const store = useRuntimeStore();
  store.$patch({
    sessionId: CURRENT_ID,
    sessionList: seedCatalog(),
    phase: "ready",
    isSubmitting: false,
    sessionOperation: null,
    sessionError: null,
    error: null,
    messages
  });
  return store;
}

function trackInvocations(): Invocation[] {
  const invocations: Invocation[] = [];
  tauriMocks.mockSafeInvoke.mockImplementation(async (command: string, payload?: unknown) => {
    invocations.push({ command, payload });
    throw new Error(`unexpected command: ${command}`);
  });
  return invocations;
}

/** C1 的公共断言：调用后必须落入一个全新的空白会话。 */
function expectFreshBlankSession(
  store: ReturnType<typeof useRuntimeStore>,
  previousIds: Set<string>,
  replacedId: string
) {
  const newId = store.sessionId;
  expect(newId, "sessionId 必须变为全新会话 id").not.toBe(replacedId);
  expect(previousIds.has(newId), "新会话 id 不得出现在调用前的 sessionList 中").toBe(false);
  expect(store.phase).toBe("idle");
  expect(store.messages).toHaveLength(0);
  expect(store.isSubmitting).toBe(false);
  expect(store.sessionOperation).toBeNull();
  expect(store.sessionError).toBeNull();
  expect(store.sessionWorkspaceId).toBe(DEFAULT_WORKSPACE_ID);
  expect(store.sessionList[0]).toMatchObject({
    conversationId: newId,
    title: "新对话",
    turnCount: 0,
    updatedAtMs: 0
  });
  return newId;
}

beforeEach(() => {
  vi.clearAllMocks();
  window.localStorage.clear();
  vi.spyOn(console, "info").mockImplementation(() => {});
  vi.spyOn(console, "debug").mockImplementation(() => {});
  tauriMocks.mockSafeListen.mockResolvedValue(() => {});
  tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
  setActivePinia(createPinia());
});

describe("Wave-12 会话落点契约（A1–A6）", () => {
  it("A1/C1 归档当前会话（Tauri 路径）→ 全新空白会话，且不切换任何历史会话", async () => {
    const store = seedCurrentSession();
    const catalogBefore = store.sessionList.map((session) => session.conversationId);
    const previousIds = new Set(catalogBefore);

    const invocations = trackInvocations();
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string, payload?: unknown) => {
      invocations.push({ command, payload });
      if (command === "session_archive") return null;
      if (command === "list_sessions") {
        return [
          overview(CURRENT_ID, { archived: true, title: "Current" }),
          overview(OTHER_ID, { title: "Other" }),
          overview(ARCHIVED_ID, { archived: true, title: "Archived" })
        ] satisfies SessionOverview[];
      }
      throw new Error(`unexpected command: ${command}`);
    });

    const outcome = await store.archiveSession(CURRENT_ID);
    expect(outcome.ok).toBe(true);

    expectFreshBlankSession(store, previousIds, CURRENT_ID);

    // 被归档会话仍在列表中且 archived === true（可 unarchive）
    const archivedRow = store.sessionList.find((session) => session.conversationId === CURRENT_ID);
    expect(archivedRow, "被归档会话必须仍在 sessionList 中").toBeTruthy();
    expect(archivedRow?.archived).toBe(true);

    // 不得切换到任何历史会话：不得调用 load_session_runtime_view（switchSession 的加载命令）
    expect(invocations.map((item) => item.command)).not.toContain("load_session_runtime_view");
  });

  it("A2/C2 删除当前会话（Tauri 路径）→ 全新空白会话，被删项不在列表且不复活", async () => {
    writePersistedSessions({
      [CURRENT_ID]: { messages: [createMessage({ id: "persisted-user", content: "被删会话的历史消息" })] }
    });
    const store = seedCurrentSession();
    const previousIds = new Set(store.sessionList.map((session) => session.conversationId));

    const invocations = trackInvocations();
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string, payload?: unknown) => {
      invocations.push({ command, payload });
      if (command === "delete_session") {
        return [
          overview(OTHER_ID, { title: "Other" }),
          overview(ARCHIVED_ID, { archived: true, title: "Archived" })
        ] satisfies SessionOverview[];
      }
      throw new Error(`unexpected command: ${command}`);
    });

    await store.deleteSession(CURRENT_ID);

    expectFreshBlankSession(store, previousIds, CURRENT_ID);

    expect(
      store.sessionList.some((session) => session.conversationId === CURRENT_ID),
      "被删会话不得留在 sessionList"
    ).toBe(false);
    expect(loadPersistedRuntimeState(CURRENT_ID), "被删会话不得复活").toBeNull();
    expect(invocations.map((item) => item.command)).not.toContain("load_session_runtime_view");
  });

  it("A3/C3 删除/归档非当前会话 → sessionId / messages / phase 不变（无导航）", async () => {
    const originalMessages = [createMessage({ id: "current-user", content: "当前会话内容" })];
    const store = seedCurrentSession(originalMessages);

    const invocations = trackInvocations();
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string, payload?: unknown) => {
      invocations.push({ command, payload });
      if (command === "delete_session") {
        return [
          overview(CURRENT_ID, { title: "Current" }),
          overview(ARCHIVED_ID, { archived: true, title: "Archived" })
        ] satisfies SessionOverview[];
      }
      if (command === "session_archive") return null;
      if (command === "list_sessions") {
        return [
          overview(CURRENT_ID, { title: "Current" }),
          overview(OTHER_ID, { title: "Other", archived: true }),
          overview(ARCHIVED_ID, { archived: true, title: "Archived" })
        ] satisfies SessionOverview[];
      }
      throw new Error(`unexpected command: ${command}`);
    });

    await store.deleteSession(OTHER_ID);
    expect(store.sessionId).toBe(CURRENT_ID);
    expect(store.messages.map((message) => message.id)).toEqual(["current-user"]);
    expect(store.phase).toBe("ready");

    await store.archiveSession(OTHER_ID);
    expect(store.sessionId).toBe(CURRENT_ID);
    expect(store.messages.map((message) => message.id)).toEqual(["current-user"]);
    expect(store.phase).toBe("ready");
    expect(invocations.map((item) => item.command)).not.toContain("load_session_runtime_view");
  });

  it("A4/C4 归档当前会话后其持久化内容仍完整（unarchive 可恢复）", async () => {
    const originalMessages = [
      createMessage({ id: "keep-user", role: "user", content: "原始问题" }),
      createMessage({ id: "keep-assistant", role: "assistant", content: "原始回答" })
    ];
    writePersistedSessions({ [CURRENT_ID]: { messages: originalMessages } });

    const store = seedCurrentSession(originalMessages);
    const previousIds = new Set(store.sessionList.map((session) => session.conversationId));

    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "session_archive") return null;
      if (command === "list_sessions") {
        return [
          overview(CURRENT_ID, { archived: true, title: "Current" }),
          overview(OTHER_ID, { title: "Other" }),
          overview(ARCHIVED_ID, { archived: true, title: "Archived" })
        ] satisfies SessionOverview[];
      }
      throw new Error(`unexpected command: ${command}`);
    });

    await store.archiveSession(CURRENT_ID);
    expectFreshBlankSession(store, previousIds, CURRENT_ID);

    const persisted = loadPersistedRuntimeState(CURRENT_ID);
    expect(persisted, "归档会话的持久化内容不得被清空").not.toBeNull();
    expect(persisted?.messages.map((message) => `${message.role}:${message.content}`)).toEqual([
      "user:原始问题",
      "assistant:原始回答"
    ]);
  });

  it("A5/C5 非 Tauri（浏览器本地）分支：归档/删除当前会话同样落入全新空白会话", async () => {
    tauriMocks.mockIsTauriAvailable.mockReturnValue(false);
    writePersistedSessions({
      [CURRENT_ID]: { messages: [createMessage({ id: "browser-user", content: "浏览器会话消息" })] },
      [OTHER_ID]: { messages: [createMessage({ id: "other-user", content: "另一个会话消息" })] }
    });

    const store = useRuntimeStore();
    store.$patch({
      sessionId: CURRENT_ID,
      sessionList: [overview(CURRENT_ID, { updatedAtMs: 3000 }), overview(OTHER_ID, { updatedAtMs: 2000 })],
      phase: "ready",
      isSubmitting: false,
      sessionOperation: null,
      sessionError: null,
      error: null,
      messages: [createMessage({ id: "browser-user", content: "浏览器会话消息" })]
    });

    const previousIds = new Set(store.sessionList.map((session) => session.conversationId));
    const outcome = await store.archiveSession(CURRENT_ID);
    expect(outcome.ok).toBe(true);
    const afterArchiveId = expectFreshBlankSession(store, previousIds, CURRENT_ID);

    // 归档内容仍可从本地持久化恢复
    expect(loadPersistedRuntimeState(CURRENT_ID)?.messages.map((message) => message.content)).toEqual([
      "浏览器会话消息"
    ]);

    // 再删除这个新的空会话：同样必须落入又一个全新空白会话（C7 边界）
    const idsBeforeDelete = new Set(store.sessionList.map((session) => session.conversationId));
    await store.deleteSession(afterArchiveId);
    expectFreshBlankSession(store, idsBeforeDelete, afterArchiveId);
  });

  it("A6/C6 归档/删除抛错 → sessionId / messages / phase 回滚到调用前快照", async () => {
    const originalMessages = [createMessage({ id: "current-user", content: "当前会话内容" })];

    // 归档失败
    const archiveStore = seedCurrentSession(originalMessages);
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "session_archive") throw new Error("archive backend failed");
      throw new Error(`unexpected command: ${command}`);
    });
    const archiveOutcome = await archiveStore.archiveSession(CURRENT_ID);
    expect(archiveOutcome.ok).toBe(false);
    expect(archiveStore.sessionId).toBe(CURRENT_ID);
    expect(archiveStore.messages.map((message) => message.id)).toEqual(["current-user"]);
    expect(archiveStore.phase).toBe("ready");

    // 删除失败
    setActivePinia(createPinia());
    const deleteStore = seedCurrentSession(originalMessages);
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "delete_session") throw new Error("delete backend failed");
      throw new Error(`unexpected command: ${command}`);
    });
    await deleteStore.deleteSession(CURRENT_ID);
    expect(deleteStore.sessionId).toBe(CURRENT_ID);
    expect(deleteStore.messages.map((message) => message.id)).toEqual(["current-user"]);
    expect(deleteStore.phase).toBe("ready");
    expect(deleteStore.sessionOperation).toBeNull();
    expect(deleteStore.deletingSessionSet).toEqual({});
  });

  it("A2 边界：当前会话为空且为唯一会话时删除 → 仍落入全新会话（不得停留在被删 id）", async () => {
    const store = useRuntimeStore();
    store.$patch({
      sessionId: CURRENT_ID,
      sessionList: [overview(CURRENT_ID, { updatedAtMs: 3000 })],
      phase: "idle",
      isSubmitting: false,
      sessionOperation: null,
      sessionError: null,
      error: null,
      messages: []
    });

    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "list_sessions") return [] satisfies SessionOverview[];
      throw new Error(`unexpected command: ${command}`);
    });

    await store.deleteSession(CURRENT_ID);

    expect(store.sessionId).not.toBe(CURRENT_ID);
    expect(store.sessionId).not.toBe(DEFAULT_SESSION_ID);
    expect(store.phase).toBe("idle");
    expect(store.messages).toHaveLength(0);
    expect(store.sessionList.some((session) => session.conversationId === CURRENT_ID)).toBe(false);
  });
});
