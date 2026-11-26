/**
 * Wave 2e · 等价测试补强（B277 / fingerprint `frontend::stores::runtime::test-coverage-gap`）
 *
 * 覆盖面：**4 条此前登记为「未覆盖」的 action**——
 *   1. `resolveTurnTraceTimeline`（原登记理由「既有死代码」）
 *   2. `findCheckpointTurnIdByNodeId`（原登记理由「夹具需完整分支图」）
 *   3. `runBrowserPreviewTurn`（原登记理由「无宿主分支」）
 *   4. `cancelBrowserPreviewTurn`（原登记理由「无宿主分支」）
 *
 * 三条「不测」的理由在实测后都不成立，本文件用可复现的构造把它们变成真断言：
 *   - `resolveTurnTraceTimeline` 虽有分支语义（历史命中 / 活跃轮回退 / 默认时间线），
 *     分支可被三条独立输入直接驱动，与「是否有调用方」无关；
 *   - `findCheckpointTurnIdByNodeId` 的 getter 会对**没有 history node 的已完成轮**
 *     合成 `synthetic-<turnId>` 条目（`history.ts` 的 `hasSynthetic` 分支），
 *     因此裸 seed 一份 `messages` 就足以造出合法 nodeId，不需要完整分支图；
 *   - 两条浏览器预览 action 的宿主分支由 `isTauriAvailable()` 决定，而它在测试里是
 *     可注入的假 IPC 之一 —— 「无宿主分支」不等于「不可测」，恰恰相反：**降级分支**
 *     才是要冻结的契约。
 *
 * 确定性：假定时器 + 冻结时钟 + 假 IPC；无 sleep、无真实 IPC、无随机。
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

const providerMock = vi.hoisted(() => ({
  state: {
    currentProvider: null as null | { name: string; protocol: string },
    currentModel: null as null | { model: string; name: string }
  }
}));

vi.mock("@/stores/provider", () => ({
  useProviderStore: () => providerMock.state
}));

import { seedStore, setupRuntimeHarness } from "./harness";

const harness = setupRuntimeHarness(tauriMocks);

/** 假定时器推进：`wait(ms)` 用的就是 setTimeout，必须显式推。 */
async function advance(ms: number): Promise<void> {
  await vi.advanceTimersByTimeAsync(ms);
}

// ─────────────────── resolveTurnTraceTimeline ───────────────────

describe("D5 · 轮次 trace 时间线解析（resolveTurnTraceTimeline）", () => {
  it("历史命中优先：即使不是活跃轮，也返回该轮已冻结的时间线", () => {
    const store = harness.getStore();
    seedStore(store, {
      turnTraceHistory: [
        {
          turnId: "t-history",
          traceTimeline: [
            { id: "x", kind: "call_model", label: "历史", state: "completed", sequence: 1 }
          ]
        }
      ],
      // 活跃轮是另一个：历史命中必须优先于「活跃轮回退」。
      activeTurnId: "t-active",
      traceTimeline: [
        { id: "y", kind: "call_model", label: "活跃", state: "active", sequence: 2 }
      ]
    });

    const resolved = store.resolveTurnTraceTimeline("t-history");

    expect(resolved.map((entry) => entry.label)).toEqual(["历史"]);
    expect(resolved.map((entry) => entry.id)).toEqual(["x"]);
  });

  it("历史未命中且是活跃轮 ⇒ 回退到当前 traceTimeline", () => {
    const store = harness.getStore();
    seedStore(store, {
      turnTraceHistory: [],
      activeTurnId: "t-active",
      traceTimeline: [
        { id: "y", kind: "call_model", label: "活跃", state: "active", sequence: 2 }
      ]
    });

    const resolved = store.resolveTurnTraceTimeline("t-active");

    expect(resolved.map((entry) => entry.label)).toEqual(["活跃"]);
  });

  it("历史未命中且非活跃轮 ⇒ 返回默认时间线（build_context completed + call_model active）", () => {
    const store = harness.getStore();
    seedStore(store, {
      turnTraceHistory: [],
      activeTurnId: "t-active",
      traceTimeline: [
        { id: "y", kind: "call_model", label: "活跃", state: "active", sequence: 2 }
      ]
    });

    const resolved = store.resolveTurnTraceTimeline("t-unknown");

    expect(resolved.map((entry) => entry.kind)).toEqual(["build_context", "call_model"]);
    expect(resolved.map((entry) => entry.state)).toEqual(["completed", "active"]);
    // 反相对照：不得把活跃轮的时间线当成未知轮的结果（这正是回退分支的边界）。
    expect(resolved.some((entry) => entry.label === "活跃")).toBe(false);
  });

  it("返回值是副本：改结果不回写 store（resolve 只读）", () => {
    const store = harness.getStore();
    seedStore(store, {
      turnTraceHistory: [
        {
          turnId: "t1",
          traceTimeline: [
            { id: "x", kind: "call_model", label: "原值", state: "completed", sequence: 1 }
          ]
        }
      ]
    });

    const resolved = store.resolveTurnTraceTimeline("t1");
    resolved[0]!.label = "被改";
    resolved.push({ id: "z", kind: "call_model", label: "额外", state: "active", sequence: 9 });

    expect(store.turnTraceHistory[0]!.traceTimeline).toHaveLength(1);
    expect(store.turnTraceHistory[0]!.traceTimeline[0]!.label).toBe("原值");
  });
});

// ─────────────────── findCheckpointTurnIdByNodeId ───────────────────

describe("D6 · 按历史节点反查轮次（findCheckpointTurnIdByNodeId）", () => {
  it("裸 seed 的已完成轮会合成 synthetic-<turnId> 节点 ⇒ 可反查到 turnId", () => {
    const store = harness.getStore();
    seedStore(store, {
      historyNodes: [],
      messages: [
        { role: "user", content: "第一个问题", turnId: "turn-a" },
        { role: "assistant", content: "回答", turnId: "turn-a" }
      ]
    });

    // 前置：确认合成节点真的存在（否则下面的命中是碰巧）。
    const synthetic = store.conversationCheckpointEntries.find(
      (entry) => entry.nodeId === "synthetic-turn-a"
    );
    expect(synthetic, "messages 里的已完成轮必须被合成为 synthetic 节点").toBeDefined();

    expect(store.findCheckpointTurnIdByNodeId("synthetic-turn-a")).toBe("turn-a");
  });

  it("nodeId 未命中 ⇒ 返回 null（不得回退到任意轮）", () => {
    const store = harness.getStore();
    seedStore(store, {
      historyNodes: [],
      messages: [{ role: "user", content: "q", turnId: "turn-a" }]
    });

    expect(store.findCheckpointTurnIdByNodeId("synthetic-does-not-exist")).toBeNull();
    expect(store.findCheckpointTurnIdByNodeId("")).toBeNull();
  });

  it("命中但 turnId 为空白 ⇒ 归 null（trim 后空串不算有效轮次）", () => {
    const store = harness.getStore();
    seedStore(store, {
      historyNodes: [],
      messages: [{ role: "user", content: "q", turnId: "   " }]
    });

    // 合成节点的 nodeId 形如 `synthetic-   `（turnId 原样拼接）；反查必须归 null 而非空串。
    const entry = store.conversationCheckpointEntries.find((item) =>
      item.nodeId.startsWith("synthetic-")
    );
    expect(entry).toBeDefined();

    expect(store.findCheckpointTurnIdByNodeId(entry!.nodeId)).toBeNull();
  });
});

// ─────────────────── runBrowserPreviewTurn ───────────────────

describe("D8 · 浏览器预览轮（runBrowserPreviewTurn）", () => {
  it("走完全流程：正文按 chunk 累加、phase 收 completed、activeTurnId 归零", async () => {
    const store = harness.getStore();
    harness.backend().routeValue("persist_history", null);
    harness.backend().routeValue("list_sessions", []);
    seedStore(store, {
      sessionId: "rt-session-main",
      activeTurnId: "preview-1",
      isSubmitting: true,
      messages: [{ id: "user-preview-1", role: "user", content: "预览", turnId: "preview-1" }]
    });

    const running = store.runBrowserPreviewTurn("preview-1");
    // 首段 wait(120) + N 段 wait(80)。
    await advance(120 + 80 * 32);
    await running;

    const assistant = store.messages.find(
      (message) => message.turnId === "preview-1" && message.role === "assistant"
    );
    expect(assistant, "预览轮必须产出 assistant 消息").toBeDefined();
    expect(assistant!.status).toBe("done");
    expect(assistant!.content.length).toBeGreaterThan(0);
    expect(store.phase).toBe("completed");
    expect(store.activeTurnId).toBeNull();
    expect(store.isSubmitting).toBe(false);
    expect(store.providerSource).toBe("browser_preview");
    expect(store.providerMode).toBe("browser_preview");
  });

  it("轮次被切走（activeTurnId 变了）⇒ 中途退出，不写 assistant 正文", async () => {
    const store = harness.getStore();
    harness.backend().routeValue("persist_history", null);
    seedStore(store, {
      sessionId: "rt-session-main",
      activeTurnId: "preview-2",
      isSubmitting: true,
      messages: [{ id: "user-preview-2", role: "user", content: "预览", turnId: "preview-2" }]
    });

    const running = store.runBrowserPreviewTurn("preview-2");
    // 在首段 wait(120) 之后、chunk 循环之前把活跃轮换掉。
    await advance(120);
    seedStore(store, { activeTurnId: "other-turn" });
    await advance(80 * 32);
    await running;

    const assistant = store.messages.find(
      (message) => message.turnId === "preview-2" && message.role === "assistant"
    );
    // 守卫在 wait 之后才生效 ⇒ 允许消息已被创建，但正文必须为空（未累加任何 chunk）。
    expect(assistant?.content ?? "").toBe("");
    expect(store.phase).not.toBe("completed");
  });
});

// ─────────────────── cancelBrowserPreviewTurn ───────────────────

describe("D8 · 浏览器预览取消（cancelBrowserPreviewTurn）", () => {
  it("非活跃轮 ⇒ 返回 false 且不改任何状态", () => {
    const store = harness.getStore();
    seedStore(store, { activeTurnId: "preview-1", phase: "streaming" });

    expect(store.cancelBrowserPreviewTurn("other-turn")).toBe(false);
    expect(store.phase).toBe("streaming");
    expect(store.activeTurnId).toBe("preview-1");
  });

  it("活跃轮 ⇒ phase 收 cancelled、终态为 stopped_by_user、正文保留不写占位", () => {
    const store = harness.getStore();
    harness.backend().routeValue("persist_history", null);
    harness.backend().routeValue("list_sessions", []);
    seedStore(store, {
      sessionId: "rt-session-main",
      activeTurnId: "preview-3",
      isSubmitting: true,
      phase: "streaming",
      messages: [
        { id: "user-preview-3", role: "user", content: "预览", turnId: "preview-3" },
        {
          id: "assistant-preview-3",
          role: "assistant",
          content: "已产出的正文",
          turnId: "preview-3",
          status: "pending"
        }
      ]
    });

    const cancelled = store.cancelBrowserPreviewTurn("preview-3");

    expect(cancelled).toBe(true);
    expect(store.phase).toBe("cancelled");
    expect(store.error).toBeNull();
    expect(store.activeTurnId).toBeNull();
    expect(store.isSubmitting).toBe(false);

    const assistant = store.messages.find((message) => message.id === "assistant-preview-3");
    expect(assistant!.status).toBe("done");
    // wave-64 §2 F2 契约：取消不得往正文写占位（如「本轮已停止。」）。
    expect(assistant!.content).toBe("已产出的正文");

    const record = store.turnTraceHistory.find((item) => item.turnId === "preview-3");
    expect(record, "取消必须留下轮次记录").toBeDefined();
    expect(record!.phase).toBe("cancelled");
    expect(record!.error).toBe("stopped_by_user");
  });
});
