/**
 * 场景 1（task-1）· turn 快乐路径：`turn:started → delta×N → turn:tool → turn:output_end → turn:completed`
 *
 * 冻结判据：事件走完后把 store 的 8 域快照（messages / trace / phase / usage / 历史游标 / 队列 /
 * 落盘 payload）逐字比对 fixtures —— fixtures 由当前实现跑出（`UPDATE_RUNTIME_FIXTURES=1`）。
 *
 * 确定性：假定时器 + 冻结时钟 + 假 IPC（`FakeBackend`）；无 sleep、无真实 IPC。
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

// 调度接缝：让「空闲期投影」与「下一帧」在用例内同步落定（无真实 rAF / 800ms 空闲延迟）。
vi.mock("@/lib/runtime/utils", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/runtime/utils")>();
  return {
    ...actual,
    waitForNextPaint: () => Promise.resolve(),
    runLowPriorityTurnWork: (callback: () => void) => callback()
  };
});

import { DEFAULT_WORKSPACE_ID } from "@/lib/runtime/workspace-constants";
import { advance, BASE_TIME, drainMicrotasks, expectGolden, runtimeSnapshot, SESSION_ID, setupRuntimeHarness } from "./harness";

const harness = setupRuntimeHarness(tauriMocks);
const TURN_ID = String(BASE_TIME);
const TURN_REF = { sessionId: SESSION_ID, turnId: TURN_ID };

function metricsFixture(step: number) {
  return {
    step,
    contextInputTokens: 128,
    inputTokens: 128,
    cacheHitInputTokens: 64,
    cacheMissInputTokens: 64,
    outputTokens: 42,
    reasoningTokens: 8,
    totalTokens: 170,
    firstTokenLatencyMs: 320,
    durationMs: 2100,
    callSpeedTokensPerSecond: 26.25,
    callSpeedBasis: "stream_decode",
    turnSpeedTokensPerSecond: 26.25,
    turnSpeedBasis: "stream_decode",
    turnOutputTokens: 42,
    turnGenerationMs: 1600,
    turnCalls: 1,
    turnCallsTotal: 1,
    turnCallsEligible: 1,
    turnBurstExcluded: 0,
    turnContextInputTokens: 128,
    turnCacheHitInputTokens: 64,
    turnCallsWithCache: 1,
    cacheHitRatio: 0.5,
    contextWindowTokens: 200000,
    contextRatio: 0.00064,
    origin: "provider",
    provider: "OpenAI",
    model: "gpt-5",
    costUsd: 0.0042
  };
}

function providerCallRecord() {
  return {
    requestKind: "initial_request",
    providerSource: "provider_decision",
    providerMode: "live",
    inputTokens: 128,
    cacheHitInputTokens: 64,
    cacheMissInputTokens: 64,
    reasoningTokens: 8,
    outputTokens: 42,
    totalTokens: 170,
    firstTokenLatencyMs: 320,
    turnDurationMs: 2100,
    latencyKind: "provider_stream",
    prefixMutationReasons: []
  };
}

const toolActivity = {
  id: "call-1",
  name: "workspace.list_dir",
  canonicalToolName: "workspace.list_dir",
  displayNameZh: "列出目录",
  status: "done" as const,
  description: "列出工作区根目录",
  argumentsText: '{"path":"."}',
  resultText: "a.ts\nb.ts\nc.ts",
  durationSeconds: 0.25
};

describe("场景 1 · turn 快乐路径 golden master", () => {
  it("started → delta×3 → tool → output_end → completed 全量快照与 fixtures 逐字一致", async () => {
    const store = harness.getStore();
    const backend = harness.backend();

    backend
      .routeValue("resolve_graph_run_submission_plan", {
        command: "start_graph_run_stream",
        runId: null
      })
      .routeValue("start_graph_run_stream", { run: { id: "run-happy" }, turnId: TURN_ID })
      .routeValue("list_sessions", [])
      .routeValue("load_retrieved_context", null);

    store.$patch({
      sessionId: SESSION_ID,
      phase: "idle",
      sessionWorkspaceId: DEFAULT_WORKSPACE_ID
    });
    store.setDraftMessage("请总结当前工作区状态");

    await expect(store.submitTurn()).resolves.toBe(true);
    expect(store.phase).toBe("calling_model");
    expect(store.activeTurnId).toBe(TURN_ID);
    expect(store.activeRunId).toBe("run-happy");
    expect(store.isSubmitting).toBe(true);
    expect(store.messages).toHaveLength(1);

    // ── turn:started ────────────────────────────────────────────────────────
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
      providerSource: "primary",
      providerMode: "standard",
      inputTokens: 128,
      traceId: "trace-happy-1",
      buildContextObservation: {
        requestFormat: "response_format=text",
        messageCount: 2,
        imageCount: 0,
        toolCount: 1,
        temperature: 0,
        maxOutputTokens: 4096,
        stablePrefixText: "system: 你是工作区助手",
        semiStableContextText: "",
        volatileInputText: "user: 请总结当前工作区状态",
        prefixMutationReasons: [],
        contextRefreshReason: "initial_build",
        instructionScopeSources: ["thread://base-system"],
        conversationCarryMode: "full_replay",
        requestMessagesText: "system: 你是工作区助手\nuser: 请总结当前工作区状态",
        toolDefinitionsText: "workspace.list_dir(path: string)"
      }
    });

    expect(store.messages).toHaveLength(2);
    expect(store.messages[1]?.id).toBe(`assistant-${TURN_ID}`);
    expect(store.messages[1]?.status).toBe("pending");
    expect(store.activeTraceId).toBe("trace-happy-1");
    expect(store.turnTraceHistory[0]?.traceId).toBe("trace-happy-1");

    // ── turn:delta ×3（含 1 条纯 reasoning）─────────────────────────────────
    backend.emit("turn:delta", {
      ...TURN_REF,
      kind: "delta",
      eventId: "ev-3",
      eventType: "turn.output_delta",
      eventVersion: "1.0",
      sequence: 3,
      emittedAtMs: 1_100,
      text: "工作区共有"
    });
    backend.emit("turn:delta", {
      ...TURN_REF,
      kind: "delta",
      eventId: "ev-4",
      eventType: "turn.output_delta",
      eventVersion: "1.0",
      sequence: 4,
      emittedAtMs: 1_200,
      text: " 3 个文件。",
      firstTokenLatencyMs: 320
    });
    backend.emit("turn:delta", {
      ...TURN_REF,
      kind: "delta",
      eventId: "ev-5",
      eventType: "turn.output_delta",
      eventVersion: "1.0",
      sequence: 5,
      emittedAtMs: 1_300,
      reasoningContent: "先读取工作区目录。"
    });

    expect(store.streamDebugDeltaCount).toBe(3);
    expect(store.streamDebugTextCharsReceived).toBe(12);
    expect(store.activeStreamKind).toBe("content");
    expect(store.streamBufferText).toBe("工作区共有 3 个文件。");
    expect(store.streamBufferReasoning).toBe("先读取工作区目录。");

    // 流式合并落定（120ms 一档）——唯一允许的「等待」是固定毫秒推进。
    await advance(120);
    expect(store.streamBufferText).toBe("");
    expect(store.streamBufferReasoning).toBe("");
    expect(store.streamDebugFlushCount).toBe(1);
    expect(store.streamDebugTextCharsFlushed).toBe(12);
    expect(store.messages[1]?.content).toBe("工作区共有 3 个文件。");
    expect(store.messages[1]?.reasoningContent).toBe("先读取工作区目录。");

    // ── turn:tool ───────────────────────────────────────────────────────────
    backend.emit("turn:tool", {
      ...TURN_REF,
      kind: "tool",
      eventId: "ev-6",
      eventType: "turn.tool_call_completed",
      eventVersion: "1.0",
      sequence: 6,
      emittedAtMs: 1_400,
      toolActivities: [toolActivity]
    });

    expect(store.phase).toBe("calling_model");
    expect(store.toolActivities).toHaveLength(1);
    expect(store.messages.map((message) => message.role)).toEqual(["user", "assistant", "tool"]);
    expect(store.messages[2]?.id).toBe(`tool-${TURN_ID}-call-1`);
    expect(store.messages[2]?.status).toBe("done");

    // ── turn:phase_changed（工具后回到模型调用：刷新 timeline + per-call 记录）────
    backend.emit("turn:phase_changed", {
      ...TURN_REF,
      kind: "trace",
      eventId: "ev-7",
      eventType: "turn.phase_changed",
      eventVersion: "1.0",
      sequence: 7,
      emittedAtMs: 1_450,
      phase: "calling_model",
      providerCallRecords: [providerCallRecord()]
    });
    expect(store.phase).toBe("calling_model");
    expect(store.inputTokens).toBe(128);
    expect(store.outputTokens).toBe(42);

    // ── turn:output_end（终稿正文覆盖流式拼接）──────────────────────────────
    backend.emit("turn:output_end", {
      ...TURN_REF,
      kind: "output_end",
      eventId: "ev-8",
      eventType: "turn.output_end",
      eventVersion: "1.0",
      sequence: 8,
      emittedAtMs: 1_500,
      text: "工作区共有 3 个文件。\n（已读取目录）",
      reasoningContent: "先读取工作区目录。",
      providerRequestedName: "OpenAI",
      providerName: "OpenAI",
      providerProtocol: "openai",
      providerModel: "gpt-5",
      providerSource: "primary",
      providerMode: "standard",
      inputTokens: 128,
      outputTokens: 42,
      totalTokens: 170,
      firstTokenLatencyMs: 320,
      turnDurationMs: 2100,
      providerCallRecords: [providerCallRecord()]
    });

    expect(store.messages[1]?.status).toBe("done");
    expect(store.messages[1]?.content).toBe("工作区共有 3 个文件。\n（已读取目录）");
    expect(store.messages[1]?.tokenCount).toBe(42);
    expect(store.messages[0]?.tokenCount).toBe(128);
    expect(store.inputTokens).toBe(128);
    expect(store.outputTokens).toBe(42);
    expect(store.totalTokens).toBe(170);

    // 落盘延迟窗口（1200ms）+ trace 节流窗口（200ms）落定。
    await advance(1_300);

    // ── turn:completed ─────────────────────────────────────────────────────
    backend.emit("turn:completed", {
      ...TURN_REF,
      kind: "completed",
      eventId: "ev-9",
      eventType: "turn.completed",
      eventVersion: "1.0",
      sequence: 9,
      emittedAtMs: 1_600,
      phase: "completed",
      traceId: "trace-happy-1",
      sessionSummary: "工作区状态汇总：3 个文件",
      providerRequestedName: "OpenAI",
      providerName: "OpenAI",
      providerProtocol: "openai",
      providerModel: "gpt-5",
      providerSource: "primary",
      providerMode: "standard",
      inputTokens: 128,
      outputTokens: 42,
      totalTokens: 170,
      firstTokenLatencyMs: 320,
      turnDurationMs: 2100,
      costUsd: 0.0042,
      toolActivities: [toolActivity],
      providerCallRecords: [providerCallRecord()],
      metrics: metricsFixture(0)
    });

    await drainMicrotasks();
    await advance(200);

    expect(store.phase).toBe("ready");
    expect(store.isSubmitting).toBe(false);
    expect(store.activeTurnId).toBeNull();
    expect(store.activeStreamKind).toBeNull();
    expect(store.eventsReady).toBe(true);
    expect(store.sessionSummary).toBe("工作区状态汇总：3 个文件");
    expect(store.liveMetrics).toEqual(metricsFixture(0));
    expect(store.sessionTotalCostUsd).toBe(0.0042);
    expect(store.hasUnpricedTurns).toBe(false);
    expect(store.formattedSessionCost).toBe("$0.004200");
    expect(store.phaseLabel).toBe("已就绪");
    expect(store.agentRuntimeStatus).toBeNull();
    expect(store.turnTraceHistory).toHaveLength(1);
    expect(store.turnTraceHistory[0]?.phase).toBe("completed");
    expect(store.turnTraceHistory[0]?.costUsd).toBe(0.0042);
    // delta 中的 traceId 在 started 落盘，completed 保持一致。
    expect(store.activeTraceId).toBe("trace-happy-1");
    // 14 个 turn 监听器（13 个生命周期 + turn:retry）。
    expect(backend.listenerCount()).toBe(14);

    // 落盘 payload：messages + 历史游标 + 运行态映射，逐字冻结（见 fixtures/turn-happy-path.json）。
    expect(runtimeSnapshot(store).persistence).toBeTruthy();

    await expectGolden("turn-happy-path", runtimeSnapshot(store));
  });

  it("跨会话与乱序事件被 fail-closed 丢弃，不推进消息与游标", async () => {
    const store = harness.getStore();
    const backend = harness.backend();
    backend
      .routeValue("resolve_graph_run_submission_plan", { command: "start_graph_run_stream", runId: null })
      .routeValue("start_graph_run_stream", { run: { id: "run-gate" }, turnId: TURN_ID })
      .routeValue("list_sessions", [])
      .routeValue("load_retrieved_context", null);

    store.$patch({ sessionId: SESSION_ID, phase: "idle" });
    store.setDraftMessage("门禁测试");
    await store.submitTurn();
    await drainMicrotasks();
    const before = runtimeSnapshot(store);

    // ① 其他会话的事件：sessionId 不匹配 ⇒ 全链路丢弃（快照逐域不变）。
    backend.emit("turn:started", {
      sessionId: "other-session",
      turnId: TURN_ID,
      kind: "started",
      eventId: "ev-x1",
      eventType: "turn.model_call_started",
      sequence: 90,
      emittedAtMs: 9_000
    });
    backend.emit("turn:delta", {
      sessionId: "other-session",
      turnId: TURN_ID,
      kind: "delta",
      eventId: "ev-x2",
      sequence: 91,
      emittedAtMs: 9_100,
      text: "不应落地"
    });
    await drainMicrotasks();

    expect(runtimeSnapshot(store)).toEqual(before);

    // ② 同会话内 sequence 回退 ⇒ 去重门禁丢弃（游标停在 5，迟到 delta 不生效）。
    backend.emit("turn:started", {
      ...TURN_REF,
      kind: "started",
      eventId: "ev-5",
      eventType: "turn.model_call_started",
      sequence: 5,
      emittedAtMs: 2_000,
      phase: "calling_model"
    });
    expect(store.messages).toHaveLength(2);
    expect(store.phase).toBe("calling_model");
    expect(store.eventCursorByTurnId[TURN_ID]?.sequence).toBe(5);

    backend.emit("turn:delta", {
      ...TURN_REF,
      kind: "delta",
      eventId: "ev-4",
      eventType: "turn.output_delta",
      sequence: 4,
      emittedAtMs: 1_000,
      text: "迟到增量"
    });
    await drainMicrotasks();

    expect(store.streamDebugDeltaCount).toBe(0);
    expect(store.streamBufferText).toBe("");
    expect(store.eventCursorByTurnId[TURN_ID]?.sequence).toBe(5);
  });
});
