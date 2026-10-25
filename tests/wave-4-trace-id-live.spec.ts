import { describe, it, expect, vi, beforeEach } from "vitest";
import { setActivePinia, createPinia } from "pinia";
import { useRuntimeStore } from "@/stores/runtime";
import * as telemetryLib from "@/lib/telemetry";

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

describe("Wave-4.1 live trace_id contract & store error reporting suite", () => {
  let listeners: Record<string, (event: { payload: any }) => void> = {};

  beforeEach(async () => {
    vi.restoreAllMocks();
    listeners = {};
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeListen.mockImplementation((event: string, handler: any) => {
      listeners[event] = handler;
      return Promise.resolve(vi.fn());
    });
    setActivePinia(createPinia());
  });

  it("1. live 链路：turn:started 携带 traceId 时，store.activeTraceId getter 立即等于该值（无需等 completed）", async () => {
    const store = useRuntimeStore();
    await store.initializeTurnEvents();

    const turnId = "turn-live-101";
    store.activeTurnId = turnId;

    // 触发 turn:started
    expect(listeners["turn:started"]).toBeDefined();
    listeners["turn:started"]({
      payload: {
        turnId,
        eventType: "turn:started",
        traceId: "4bf92f3577b34da6a3ce929d0e0e4736",
        providerName: "anthropic",
        providerModel: "claude-3-7-sonnet"
      }
    });

    // 断言 getter 立即返回该 traceId
    expect(store.activeTraceId).toBe("4bf92f3577b34da6a3ce929d0e0e4736");
  });

  it("2. 失败上报携带：turn:failed 时，reportError 接收到的 payload.trace_id 精确等于当前活跃 turn 的 traceId", async () => {
    const reportErrorSpy = vi.spyOn(telemetryLib, "reportError").mockResolvedValue(undefined as any);

    const store = useRuntimeStore();
    await store.initializeTurnEvents();

    const turnId = "turn-live-fail";
    store.activeTurnId = turnId;

    // 先进入 started，写入 traceId
    listeners["turn:started"]({
      payload: {
        turnId,
        eventType: "turn:started",
        sequence: 1,
        eventId: "evt-started-1",
        traceId: "4bf92f3577b34da6a3ce929d0e0e4736",
        providerName: "anthropic",
        providerModel: "claude-3-7-sonnet"
      }
    });

    expect(store.activeTraceId).toBe("4bf92f3577b34da6a3ce929d0e0e4736");

    // 触发 turn:failed
    expect(listeners["turn:failed"]).toBeDefined();
    listeners["turn:failed"]({
      payload: {
        turnId,
        eventType: "turn:failed",
        kind: "failed",
        sequence: 2,
        eventId: "evt-failed-2",
        error: "Sandbox execution timeout",
        providerName: "anthropic",
        providerModel: "claude-3-7-sonnet"
      }
    });

    // 异步宏任务/定时器等待 reportError 触发
    await vi.waitFor(() => {
      expect(reportErrorSpy).toHaveBeenCalled();
    });

    const callArgs = reportErrorSpy.mock.calls[0][0];
    expect(callArgs.trace_id).toBe("4bf92f3577b34da6a3ce929d0e0e4736");
    expect(callArgs.errorType).toBe("TurnExecutionFailed");
  });

  it("3. 不跨轮误归因：新轮 turn:started 若未携带 traceId 字段，activeTraceId 不得返回上一轮的 traceId（必须为 undefined）", async () => {
    const store = useRuntimeStore();
    await store.initializeTurnEvents();

    // 第一轮：带 traceId
    const turn1 = "turn-1";
    store.activeTurnId = turn1;
    listeners["turn:started"]({
      payload: {
        turnId: turn1,
        eventType: "turn:started",
        traceId: "11111111111111111111111111111111"
      }
    });
    expect(store.activeTraceId).toBe("11111111111111111111111111111111");

    // 第二轮：开启新轮，但未携带 traceId（例如无采样或普通轮）
    const turn2 = "turn-2";
    store.activeTurnId = turn2;
    listeners["turn:started"]({
      payload: {
        turnId: turn2,
        eventType: "turn:started"
      }
    });

    // 绝不能错误回退到 turn1 的 traceId
    expect(store.activeTraceId).toBeUndefined();
  });

  it("4. 无 activeTurn 时回落：activeTurnId = null 时，只回落到最近一个已完成 (phase === 'completed') 且带 traceId 的轮次", async () => {
    const store = useRuntimeStore();

    store.activeTurnId = null;
    store.turnTraceHistory = [
      {
        turnId: "turn-A-completed",
        title: "turn A",
        phase: "completed",
        traceSteps: [],
        toolActivities: [],
        updatedAt: Date.now() - 1000,
        traceId: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
      },
      {
        turnId: "turn-B-running",
        title: "turn B",
        phase: "running", // 非 completed 状态
        traceSteps: [],
        toolActivities: [],
        updatedAt: Date.now(),
        traceId: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
      }
    ];

    // 断言必须精确返回已完成轮次 turn-A 的 traceId，不得返回未完成的 turn-B
    expect(store.activeTraceId).toBe("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
  });

  it("5. 导出态纪律（L2-AT）：turn:started 完全缺席 traceId 键（门禁关闭真实 Wire）时，activeTraceId 为 undefined，上报 payload.trace_id 必须为 undefined", async () => {
    const reportErrorSpy = vi.spyOn(telemetryLib, "reportError").mockResolvedValue(undefined as any);

    const store = useRuntimeStore();
    await store.initializeTurnEvents();

    const turnId = "turn-live-disabled";
    store.activeTurnId = turnId;

    // 真实 wire：Rust 端 skip_serializing_if，字段完全缺席（不带 traceId 键）
    listeners["turn:started"]({
      payload: {
        turnId,
        eventType: "turn:started",
        sequence: 1,
        eventId: "evt-started-5"
      }
    });

    expect(store.activeTraceId).toBeUndefined();

    listeners["turn:failed"]({
      payload: {
        turnId,
        eventType: "turn:failed",
        kind: "failed",
        sequence: 2,
        eventId: "evt-failed-5",
        error: "Some failure without trace"
      }
    });

    await vi.waitFor(() => {
      expect(reportErrorSpy).toHaveBeenCalled();
    });

    const callArgs = reportErrorSpy.mock.calls[0][0];
    expect(callArgs.trace_id).toBeUndefined();
  });

  it("6. F1 防跨轮误归因（L2-AT）：历史存在 completed turn（带 traceId X），触发无 traceId 的 turn:failed 时，上报 payload.trace_id 严禁误归因为 X", async () => {
    const reportErrorSpy = vi.spyOn(telemetryLib, "reportError").mockResolvedValue(undefined as any);

    const store = useRuntimeStore();
    await store.initializeTurnEvents();

    // 1. 先布好陷阱：历史存量已完成 turn，带 traceId X，此时 activeTurnId 必须为 null
    store.activeTurnId = null;
    store.turnTraceHistory = [
      {
        turnId: "turn-prev-finished",
        title: "prev",
        phase: "completed",
        traceSteps: [],
        toolActivities: [],
        updatedAt: Date.now() - 2000,
        traceId: "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"
      }
    ];

    // 2. 证明陷阱已布好：此时 activeTurnId 为 null，activeTraceId getter 必须确实回落读到了该历史 traceId
    expect(store.activeTraceId).toBe("xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx");

    // 3. 产生一个活跃轮次（activeTurnId 设为该轮，但该轮本身完全不带 traceId）
    const failedTurnId = "turn-new-failed-no-trace";
    store.activeTurnId = failedTurnId;

    listeners["turn:failed"]({
      payload: {
        turnId: failedTurnId,
        eventType: "turn:failed",
        kind: "failed",
        sequence: 1,
        eventId: "evt-failed-6",
        error: "Execution failed on un-traced turn"
      }
    });

    await vi.waitFor(() => {
      expect(reportErrorSpy).toHaveBeenCalled();
    });

    const callArgs = reportErrorSpy.mock.calls[0][0];
    // 必须为 undefined，绝不能回落将已完成的上一轮 traceId "xxxxxxxx..." 误扣给本轮报错
    expect(callArgs.trace_id).toBeUndefined();
    expect(callArgs.trace_id).not.toBe("xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx");
  });
});
