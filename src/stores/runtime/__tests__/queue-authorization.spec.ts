/**
 * 场景 5（task-1）· 队列与授权策略：`enqueueMessage` / `promoteToSteer` / `removeQueuedMessage` /
 * `toggleQueueDeliveryMode` / `setQueueDeliveryMode` / `setToolAuthorizationMode`（+ `dequeueNextMessage`）。
 *
 * 冻结判据：
 * - 队列是 D7 纯内存状态机（不落盘），steer 插队语义（unshift）与 queue 顺序语义（push）逐字冻结；
 * - `toolAuthorizationMode` / `workspaceId` 必须原样透传进 `TurnInput`（IPC 契约面）。
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

import {
  BASE_TIME,
  drainMicrotasks,
  expectGolden,
  runtimeSnapshot,
  SESSION_ID,
  setupRuntimeHarness
} from "./harness";

const harness = setupRuntimeHarness(tauriMocks);
const TURN_ID = String(BASE_TIME);

describe("场景 5 · 队列与授权 golden master", () => {
  it("队列状态机：queue 追加 / steer 插队 / promote 提权 / remove / 模式切换 / 出队顺序", async () => {
    const store = harness.getStore();
    store.$patch({ sessionId: SESSION_ID, queueDeliveryMode: "queue", toolAuthorizationMode: "workspace_write" });

    // 空内容直接 no-op（不产生条目）。
    store.enqueueMessage("   ");
    expect(store.pendingQueuedMessages).toHaveLength(0);

    store.enqueueMessage("第一条排队消息");
    store.enqueueMessage("第二条排队消息");
    store.enqueueMessage("插队消息", "steer");
    expect(store.pendingQueuedMessages.map((item) => item.content)).toEqual([
      "插队消息",
      "第一条排队消息",
      "第二条排队消息"
    ]);
    expect(store.pendingQueuedMessages.map((item) => item.mode)).toEqual(["steer", "queue", "queue"]);

    const secondId = store.pendingQueuedMessages[2]!.id;
    store.promoteToSteer(secondId);
    expect(store.pendingQueuedMessages.map((item) => item.content)).toEqual([
      "第二条排队消息",
      "插队消息",
      "第一条排队消息"
    ]);
    expect(store.pendingQueuedMessages[0]?.mode).toBe("steer");

    store.removeQueuedMessage(store.pendingQueuedMessages[1]!.id);
    expect(store.pendingQueuedMessages.map((item) => item.content)).toEqual(["第二条排队消息", "第一条排队消息"]);

    // 无匹配 id 的 promote/remove 幂等。
    store.promoteToSteer("queued-unknown");
    store.removeQueuedMessage("queued-unknown");
    expect(store.pendingQueuedMessages).toHaveLength(2);

    store.toggleQueueDeliveryMode();
    expect(store.queueDeliveryMode).toBe("steer");
    store.toggleQueueDeliveryMode();
    expect(store.queueDeliveryMode).toBe("queue");
    store.setQueueDeliveryMode("steer");
    expect(store.queueDeliveryMode).toBe("steer");
    store.setToolAuthorizationMode("read_only");
    expect(store.toolAuthorizationMode).toBe("read_only");

    // 入队项跟随当前默认交付模式（仅在未显式传 mode 时）。
    store.enqueueMessage("跟随默认模式的消息");
    expect(store.pendingQueuedMessages[0]?.mode).toBe("steer");

    const first = store.dequeueNextMessage();
    expect(first?.content).toBe("跟随默认模式的消息");
    expect(store.pendingQueuedMessages).toHaveLength(2);
    store.dequeueNextMessage();
    store.dequeueNextMessage();
    expect(store.pendingQueuedMessages).toEqual([]);
    expect(store.dequeueNextMessage()).toBeNull();
    // 队列不落盘（D7 纯内存）：空会话不产生 payload。
    expect(runtimeSnapshot(store).persistence).toMatchObject({ payload: null, sessionKeys: [] });

    await expectGolden("queue-state-machine", runtimeSnapshot(store));
  });

  it("submitTurn 将 toolAuthorizationMode / workspaceId / sessionId 原样透传进 IPC 入参", async () => {
    const store = harness.getStore();
    const backend = harness.backend();
    backend
      .routeValue("resolve_graph_run_submission_plan", { command: "start_graph_run_stream", runId: null })
      .routeValue("start_graph_run_stream", { run: { id: "run-auth" }, turnId: TURN_ID })
      .routeValue("list_sessions", [])
      .routeValue("load_retrieved_context", null);

    store.$patch({
      sessionId: SESSION_ID,
      sessionWorkspaceId: "ws-authorization",
      toolAuthorizationMode: "read_only",
      queueDeliveryMode: "steer"
    });
    store.setPendingAttachments([]);
    store.setDraftMessage("授权模式透传验证");

    await expect(store.submitTurn()).resolves.toBe(true);
    await drainMicrotasks();

    const startCall = backend.callsFor("start_graph_run_stream")[0];
    expect(startCall).toBeDefined();
    const args = startCall!.args as Record<string, any>;
    expect(args.turnId).toBe(TURN_ID);
    expect(args.runId).toBeNull();
    expect(args.goal).toBe("授权模式透传验证");
    expect(args.input).toMatchObject({
      sessionId: SESSION_ID,
      workspaceId: "ws-authorization",
      toolAuthorizationMode: "read_only",
      message: "授权模式透传验证",
      displayMessage: "授权模式透传验证",
      history: [],
      images: []
    });
    expect(args.input.providerId).toBeNull();
    expect(args.input.modelId).toBeNull();
    // 提交后草稿与待发送附件被清空（防跨轮携带）。
    expect(store.draftMessage).toBe("");
    expect(store.pendingAttachments).toEqual([]);

    await expectGolden("queue-submit-authorization-passthrough", {
      startGraphRunStreamArgs: args
    });
  });
});
