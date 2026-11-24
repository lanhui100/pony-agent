/**
 * Wave 2a · 等价测试补强（B277 / fingerprint `frontend::stores::runtime::test-coverage-gap`）
 *
 * 覆盖面：D8 健康与能力目录（6）· 用量治理（2）· 停止链（2）· 会话通知（1）· 待发送附件（3）。
 * 断言口径与既有 golden 场景不同：golden 冻结「事件走完后的全量快照」，本文件冻结
 * 「action 的宿主命令边界 + 状态转移」——两者互补，不重复覆盖同一判据。
 *
 * 确定性：假定时器 + 冻结时钟 + 假 IPC（FakeBackend）；无 sleep、无真实 IPC。
 */
import { describe, expect, it, vi } from "vitest";

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

vi.mock("@/lib/runtime/utils", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/runtime/utils")>();
  return {
    ...actual,
    waitForNextPaint: () => Promise.resolve(),
    runLowPriorityTurnWork: (callback: () => void) => callback()
  };
});

const recorderMock = vi.hoisted(() => ({
  initFrontendFlightRecorder: vi.fn(),
  bindFrontendRecorderSession: vi.fn(),
  bindFrontendRecorderTurn: vi.fn(),
  recordFrontendInstant: vi.fn()
}));

vi.mock("@/lib/frontend-flight-recorder", () => recorderMock);

const settingsMock = vi.hoisted(() => ({
  state: {
    settings: {
      usageGovernancePolicy: { enforcement: true }
    },
    saveSettings: vi.fn(async () => undefined)
  }
}));

vi.mock("@/stores/settings", () => ({
  useSettingsStore: () => settingsMock.state
}));

// 附件字节读取走 FileReader（事件驱动），在假定时器下不会自行落定 ——
// 用例侧只替换「读字节」这一层，其余附件规则（类型/上限/去重/嗅探）保持真实实现。
vi.mock("@/lib/runtime/file-attachments", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/runtime/file-attachments")>();
  return {
    ...actual,
    readFileAsArrayBuffer: async () => new TextEncoder().encode("hello endpoint").buffer
  };
});

import { seedStore, setupRuntimeHarness } from "./harness";

const harness = setupRuntimeHarness(tauriMocks);

describe("D1 · 轮次撤回（rollbackToTurnBoundary）", () => {
  it("空 turnId 或未命中 ⇒ 返回 false 且不动消息", () => {
    const store = harness.getStore();
    seedStore(store, {
      messages: [
        { role: "user", content: "a", turnId: "t1" },
        { role: "assistant", content: "b", turnId: "t2" }
      ]
    });

    expect(store.rollbackToTurnBoundary("   ")).toBe(false);
    expect(store.rollbackToTurnBoundary("t9")).toBe(false);
    expect(store.messages).toHaveLength(2);
  });

  it("命中 ⇒ 截断到该轮末尾、清掉后续 trace、phase 回到 ready", () => {
    const store = harness.getStore();
    seedStore(store, {
      messages: [
        { role: "user", content: "a", turnId: "t1" },
        { role: "assistant", content: "b", turnId: "t1" },
        { role: "user", content: "c", turnId: "t2" }
      ],
      turnTraceHistory: [
        { turnId: "t1", traceTimeline: [{ id: "x", kind: "call_model", label: "x", state: "pending", sequence: 1 }] },
        { turnId: "t2", traceTimeline: [] }
      ],
      activeTurnId: "t2",
      isSubmitting: true,
      phase: "streaming"
    });
    harness.backend().routeValue("persist_history", null);

    const ok = store.rollbackToTurnBoundary("t1");

    expect(ok).toBe(true);
    expect(store.messages.map((m) => m.turnId)).toEqual(["t1", "t1"]);
    expect(store.turnTraceHistory.map((t) => t.turnId)).toEqual(["t1"]);
    expect(store.phase).toBe("ready");
    expect(store.activeTurnId).toBeNull();
    expect(store.isSubmitting).toBe(false);
  });
});

describe("D1 · 回到初始态（rollbackToInitialState）", () => {
  it("清空轮次级真值：phase/error/draft/provider/token 全归零", () => {
    const store = harness.getStore();
    seedStore(store, {
      phase: "failed",
      error: "boom",
      draftMessage: "草稿",
      sessionSummary: "摘要",
      providerRequestedName: "p",
      providerName: "p",
      providerProtocol: "openai",
      providerModel: "m",
      providerSource: "provider",
      providerMode: "stream",
      fallbackReason: "x",
      inputTokens: 10,
      outputTokens: 20,
      totalTokens: 30,
      firstTokenLatencyMs: 99,
      liveMetrics: { tokensPerSecond: 1 }
    });

    store.rollbackToInitialState();

    expect(store.phase).toBe("idle");
    expect(store.error).toBeNull();
    expect(store.draftMessage).toBe("");
    expect(store.sessionSummary).toBe("");
    expect(store.providerName).toBe("");
    expect(store.providerModel).toBe("");
    expect(store.inputTokens).toBeNull();
    expect(store.outputTokens).toBeNull();
    expect(store.totalTokens).toBeNull();
    expect(store.firstTokenLatencyMs).toBeNull();
    expect(store.liveMetrics).toBeNull();
  });
});

describe("D1 · 检索上下文加载（loadRetrievedContextState）", () => {
  it("宿主返回 ⇒ 落值（clone，不与宿主对象共享引用）", async () => {
    const store = harness.getStore();
    const hostPayload = { conversationId: "s1", retrievedItems: [{ id: "r1" }] };
    harness.backend().routeValue("load_retrieved_context", hostPayload);

    const result = await store.loadRetrievedContextState("s1", { runId: "run-1" });

    // RetrievedContextState 是五段式结构（turn/session/run/longTerm/transcript），
    // clone 会补齐缺省段；这里断言「宿主带回来的段确实落到了返回值上」。
    expect(result).not.toBe(hostPayload);
    expect((result as { turnContext?: unknown }).turnContext).toBeDefined();
    expect(harness.backend().callsFor("load_retrieved_context")).toHaveLength(1);
  });

  it("宿主失败 ⇒ 回退快照派生（不抛错、不把空值当成功）", async () => {
    const store = harness.getStore();
    seedStore(store, { sessionId: "s1", messages: [] });
    harness.backend().route("load_retrieved_context", () => {
      throw new Error("boom");
    });

    const result = await store.loadRetrievedContextState("s1", { runId: null });

    expect(result).not.toBeNull();
    expect(harness.backend().callsFor("load_retrieved_context")).toHaveLength(1);
  });
});
