/**
 * 场景 2（task-1）· turn 异常路径：`turn:failed` / `turn:cancelled` / `turn:suspended` /
 * `turn:retry`（上游重试进度） / 看门狗超时（`handleSubmissionWatchdogTimeout`）。
 *
 * 冻结判据：每条异常路径收口后把 8 域快照与 fixtures 逐字比对（含落盘 payload 的
 * `phase` / `canonicalTerminalPhase` 形状）。
 *
 * 关键行为锁（wave-64 §2 F2 契约，拆解后不得漂移）：
 * - 失败轮的零正文兼容锁文案 `"This turn failed."` **不得**写入助手正文；
 * - 取消**不得**补占位正文，终态由 `agentRuntimeStatus.mode === "stopped"` 承载。
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

import { advance, BASE_TIME, drainMicrotasks, expectGolden, runtimeSnapshot, SESSION_ID, setupRuntimeHarness } from "./harness";

const harness = setupRuntimeHarness(tauriMocks);
const TURN_ID = String(BASE_TIME);
const TURN_REF = { sessionId: SESSION_ID, turnId: TURN_ID };

function providerCallRecord() {
  return {
    requestKind: "initial_request",
    providerSource: "provider_decision",
    providerMode: "live",
    inputTokens: 96,
    cacheHitInputTokens: 0,
    cacheMissInputTokens: 96,
    reasoningTokens: 0,
    outputTokens: 17,
    totalTokens: 113,
    firstTokenLatencyMs: 210,
    turnDurationMs: 1_500,
    latencyKind: "provider_stream",
    prefixMutationReasons: []
  };
}

async function startTurn(store: ReturnType<typeof harness.getStore>, backend: ReturnType<typeof harness.backend>) {
  backend
    .routeValue("resolve_graph_run_submission_plan", { command: "start_graph_run_stream", runId: null })
    .routeValue("start_graph_run_stream", { run: { id: "run-exc" }, turnId: TURN_ID })
    .routeValue("list_sessions", [])
    .routeValue("load_retrieved_context", null);
  store.$patch({ sessionId: SESSION_ID, phase: "idle" });
  store.setDraftMessage("异常路径提问");
  await store.submitTurn();
  await drainMicrotasks();

  backend.emit("turn:started", {
    ...TURN_REF,
    kind: "started",
    eventId: "ev-2",
    eventType: "turn.model_call_started",
    eventVersion: "1.0",
    sequence: 2,
    emittedAtMs: 1_000,
    phase: "calling_model",
    providerRequestedName: "OpenAI",
    providerName: "OpenAI",
    providerProtocol: "openai",
    providerModel: "gpt-5",
    traceId: "trace-exc",
    inputTokens: 96
  });
  backend.emit("turn:delta", {
    ...TURN_REF,
    kind: "delta",
    eventId: "ev-3",
    eventType: "turn.output_delta",
    eventVersion: "1.0",
    sequence: 3,
    emittedAtMs: 1_100,
    text: "已产出的部分正文"
  });
  await advance(120);
  expect(store.messages[1]?.content).toBe("已产出的部分正文");
}

describe("场景 2 · turn 异常路径 golden master", () => {
  it("turn:failed 保留已产出正文、不写零正文兼容锁文案，落盘 phase=failed/canonical=failed", async () => {
    const store = harness.getStore();
    const backend = harness.backend();
    await startTurn(store, backend);

    backend.emit("turn:failed", {
      ...TURN_REF,
      kind: "failed",
      eventId: "ev-4",
      eventType: "turn.failed",
      eventVersion: "1.0",
      sequence: 4,
      emittedAtMs: 1_400,
      phase: "failed",
      // core 线上恒发的零正文兼容锁文案：必须被 normalizeFailedTurnContent 丢弃。
      text: "This turn failed.",
      error: "provider timeout",
      errorClass: "timeout",
      providerRequestedName: "OpenAI",
      providerName: "OpenAI",
      providerProtocol: "openai",
      providerModel: "gpt-5",
      traceId: "trace-exc",
      inputTokens: 96,
      outputTokens: 17,
      totalTokens: 113,
      turnDurationMs: 1_500,
      providerCallRecords: [providerCallRecord()]
    });
    await drainMicrotasks();
    await advance(10);

    expect(store.phase).toBe("failed");
    expect(store.isSubmitting).toBe(false);
    expect(store.activeTurnId).toBeNull();
    expect(store.error).toBe("provider timeout");
    expect(store.messages[1]?.status).toBe("error");
    expect(store.messages[1]?.errorDetail).toBe("provider timeout");
    // 零正文兼容锁文案不得进正文；已流式产出的正文必须保留。
    expect(store.messages[1]?.content).toBe("已产出的部分正文");
    expect(store.agentRuntimeStatus).toEqual({
      mode: "error",
      label: "provider timeout · 上游失败，可重试",
      startTime: null
    });
    expect(store.failedTurnRetry?.retryable).toBe(true);
    expect(store.turnTraceHistory[0]?.phase).toBe("failed");
    expect(store.turnTraceHistory[0]?.error).toBe("provider timeout");
    const persisted = runtimeSnapshot(store).persistence as Record<string, any>;
    expect(persisted.payload.phase).toBe("failed");
    expect(persisted.payload.canonicalTerminalPhase).toBe("failed");

    await expectGolden("turn-failed-path", runtimeSnapshot(store));
  });

  it("turn:cancelled 不补占位正文、error 归零，终态由 agentRuntimeStatus.mode=stopped 承载", async () => {
    const store = harness.getStore();
    const backend = harness.backend();
    await startTurn(store, backend);

    backend.emit("turn:cancelled", {
      ...TURN_REF,
      kind: "cancelled",
      eventId: "ev-4",
      eventType: "turn.cancelled",
      eventVersion: "1.0",
      sequence: 4,
      emittedAtMs: 1_400,
      phase: "cancelled",
      text: "本轮已停止。",
      stopReason: "user",
      traceId: "trace-exc",
      inputTokens: 96,
      outputTokens: 17,
      totalTokens: 113,
      turnDurationMs: 1_500,
      providerCallRecords: [providerCallRecord()]
    });
    await drainMicrotasks();
    await advance(10);

    expect(store.phase).toBe("cancelled");
    expect(store.isSubmitting).toBe(false);
    expect(store.activeTurnId).toBeNull();
    expect(store.error).toBeNull();
    expect(store.sessionNotice).toBeNull();
    // 取消不得另起消息/占位：正文保留已产出内容，status 收敛为 done。
    expect(store.messages[1]?.content).toBe("已产出的部分正文");
    expect(store.messages[1]?.status).toBe("done");
    expect(store.agentRuntimeStatus).toEqual({ mode: "stopped", label: "已停止", startTime: null });
    expect(store.turnTraceHistory[0]?.phase).toBe("cancelled");
    expect(store.turnTraceHistory[0]?.error).toBe("stopped_by_user");
    const persisted = runtimeSnapshot(store).persistence as Record<string, any>;
    expect(persisted.payload.phase).toBe("cancelled");
    expect(persisted.payload.canonicalTerminalPhase).toBe("cancelled");

    await expectGolden("turn-cancelled-path", runtimeSnapshot(store));
  });

  it("turn:suspended 进入 waiting_user：解锁提交态但保留消息与 trace", async () => {
    const store = harness.getStore();
    const backend = harness.backend();
    await startTurn(store, backend);

    backend.emit("turn:suspended", {
      ...TURN_REF,
      kind: "suspended",
      eventId: "ev-4",
      eventType: "turn.suspended",
      eventVersion: "1.0",
      sequence: 4,
      emittedAtMs: 1_400,
      text: "需要你确认后继续",
      traceId: "trace-exc",
      inputTokens: 96,
      outputTokens: 17,
      totalTokens: 113,
      turnDurationMs: 1_500,
      providerCallRecords: [providerCallRecord()]
    });
    await drainMicrotasks();
    await advance(10);

    expect(store.phase).toBe("waiting_user");
    expect(store.phaseLabel).toBe("waiting_user");
    expect(store.isSubmitting).toBe(false);
    expect(store.activeTurnId).toBeNull();
    expect(store.error).toBeNull();
    expect(store.messages[1]?.content).toBe("需要你确认后继续");
    expect(store.messages[1]?.status).toBe("done");
    expect(store.agentRuntimeStatus).toBeNull();
    expect(store.turnTraceHistory[0]?.phase).toBe("waiting_user");
    const persisted = runtimeSnapshot(store).persistence as Record<string, any>;
    expect(persisted.payload.phase).toBe("waiting_user");
    expect(persisted.payload.canonicalTerminalPhase).toBeUndefined();

    await expectGolden("turn-suspended-path", runtimeSnapshot(store));
  });

  it("turn:retry 只按真实 RetryNotice 落进度，畸形/跨会话负载丢弃，首个增量清零", async () => {
    const store = harness.getStore();
    const backend = harness.backend();
    await startTurn(store, backend);

    // 本用例已在流式正文中（startTurn 已 flush 过正文）⇒ 状态条为 responding。
    expect(store.agentRuntimeStatus?.mode).toBe("responding");
    backend.emit("turn:retry", {
      ...TURN_REF,
      attempt: 2,
      maxAttempts: 5,
      delayMs: 1_500,
      errorClass: "timeout"
    });
    expect(store.retryProgress).toEqual({ turnId: TURN_ID, attempt: 2, maxAttempts: 5 });
    expect(store.agentRuntimeStatus).toMatchObject({
      mode: "retrying",
      label: "正在重试（2/5）...",
      retryAttempt: 2,
      retryMax: 5
    });

    // 物理上不可信（分母 < 分子 / 分子非正）与跨会话负载一律不得进状态行。
    backend.emit("turn:retry", { ...TURN_REF, attempt: 0, maxAttempts: 5, delayMs: 0, errorClass: "timeout" });
    backend.emit("turn:retry", { ...TURN_REF, attempt: 4, maxAttempts: 3, delayMs: 0, errorClass: "timeout" });
    backend.emit("turn:retry", {
      sessionId: "other-session",
      turnId: TURN_ID,
      attempt: 1,
      maxAttempts: 9,
      delayMs: 0,
      errorClass: "timeout"
    });
    expect(store.retryProgress).toEqual({ turnId: TURN_ID, attempt: 2, maxAttempts: 5 });

    // 首个真实增量 ⇒ retry 占位与进度同点清零。
    backend.emit("turn:delta", {
      ...TURN_REF,
      kind: "delta",
      eventId: "ev-4",
      eventType: "turn.output_delta",
      eventVersion: "1.0",
      sequence: 4,
      emittedAtMs: 1_400,
      text: "重试成功后的正文"
    });
    expect(store.retryProgress).toBeNull();
    expect(store.agentRuntimeStatus?.mode).toBe("responding");
    await advance(120);
    expect(store.messages[1]?.content).toBe("已产出的部分正文重试成功后的正文");
  });

  it("看门狗超时强制解锁：phase=failed、消息收敛为 error 且不发工程文案", async () => {
    const store = harness.getStore();
    const backend = harness.backend();
    backend
      .routeValue("resolve_graph_run_submission_plan", { command: "start_graph_run_stream", runId: null })
      .routeValue("start_graph_run_stream", { run: { id: "run-watchdog" }, turnId: TURN_ID })
      .routeValue("list_sessions", [])
      .routeValue("load_retrieved_context", null);

    store.$patch({ sessionId: SESSION_ID, phase: "idle" });
    store.setDraftMessage("看门狗提问");
    await store.submitTurn();
    await drainMicrotasks();
    expect(store.isSubmitting).toBe(true);
    const watchdogPending = (store as unknown as Record<string, unknown>).submissionWatchdogTimerId;
    expect(watchdogPending).not.toBeNull();

    // 静默 240s（契约冻结阈值）⇒ 看门狗兜底（固定毫秒推进，非 sleep）。
    await advance(240_001);
    await drainMicrotasks();

    expect(store.phase).toBe("failed");
    expect(store.error).toBe("响应超时，已停止等待");
    expect(store.isSubmitting).toBe(false);
    expect(store.activeTurnId).toBeNull();
    const assistant = store.messages.find((message) => message.role === "assistant");
    expect(assistant?.status).toBe("error");
    expect(assistant?.errorDetail).toBe("submission_watchdog_timeout");
    expect(assistant?.content).toBe("");
    expect(store.messageRevision).toBeNull();
    // 后端通知：fail_turn_for_watchdog + stop_graph_run（fire-and-forget，不阻断解锁）。
    expect(backend.commands()).toContain("fail_turn_for_watchdog");
    expect(backend.commands()).toContain("stop_graph_run");
    const persisted = runtimeSnapshot(store).persistence as Record<string, any>;
    expect(persisted.payload.phase).toBe("failed");
    expect(persisted.payload.canonicalTerminalPhase).toBe("failed");

    await expectGolden("turn-watchdog-timeout", runtimeSnapshot(store));
  });
});
