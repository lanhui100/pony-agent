/**
 * Wave-15 契约红相验收 —— S1/S3 Token 口径（L2-T，Test Agent 唯一编写方）
 *
 * 契约：.dev-team/contracts/wave-15-token-metrics.md
 *   §3 规范语义（S1–S4）/ §4 数据面（R1–R5）/ §5 验收（T1–T6、A1–A5）/ §7 冻结解析序 v2
 *
 * 冻结解析序（Lead 裁定，2026-xx；A5 采用裁定 A 变体）：
 *   countTurnProviderCalls(turn) := max(providerCallRecords.length, canonical call_model 条目数)
 *   S1 resolveCurrentContextInputTokens(turn):
 *     1) records 非空 且 (records.length > 1 || records.length === countTurnProviderCalls) → 末条 record.inputTokens
 *     2) 否则 → 最后一条「带 inputTokens」的 call_model 条目
 *     3) 否则 → countTurnProviderCalls <= 1 时允许 turn.inputTokens
 *     4) 否则 → null（严禁放行聚合值）
 *   S3 resolveCallGenerationSpeed(record)（仅步骤 1 + 守卫；步骤 2–4 属调用方 useTraceProjection）:
 *     output = record.outputTokens；duration = record.turnDurationMs（本次调用耗时，含 TTFT）
 *     TTFT 仅当 record.latencyKind === "provider_stream" 时参与抵扣（§7.2 门控）
 *     守卫：duration <= 0 或 output == null → null；返回值必须 Number.isFinite
 *
 * 红相纪律：本文件所有失败必须来自 SUT（src/lib/runtime/token-metrics.ts 空桩 throw，
 * 或 SUT 计算/投影断言不成立）。禁止因缺文件 / import 失败 / 语法错误而红。
 * 测试基建（mock、fixture、flush）不得成为红相来源。
 *
 * wave-54 契约演进（ADR 0053）：速度口径由 per-call/整轮聚合改为 Σout/Σgen
 *   `gen_i = (ttft_i 存在 且 dur_i > ttft_i) ? dur_i − ttft_i : dur_i`
 * 本文件的投影速度断言已随之迁移：三跳夹具 Σout = 500 + 600 + 100 = 1200、
 *   Σgen = (1000−150) + (1100−180) + (1200−200) = 850 + 920 + 1000 = 2770ms
 *   ⇒ turn 加权速度 = 1200 / 2.77 = 433.2129963898917 t/s（不再是末跳 per-call 的 100 t/s）。
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ref } from "vue";
import { createPinia, setActivePinia } from "pinia";
import { useRuntimeStore } from "@/stores/runtime";
import { useProviderStore } from "@/stores/providers";
import { useTraceProjection } from "@/lib/runtime/useTraceProjection";
import {
  countTurnProviderCalls,
  resolveCallGenerationSpeed,
  resolveCurrentContextInputTokens
} from "@/lib/runtime/token-metrics";
import type {
  ChatMessage,
  ProviderCallCacheRecord,
  TraceTimelineEntry,
  TurnTraceRecord
} from "@/types/runtime";

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

const SESSION_ID = "session-wave15-red";
const ACTIVE_TURN_ID = "turn-wave15-active";

/** 三跳轮的真实 per-call 值（契约 T1）。 */
const HOP_INPUTS = [350_000, 360_000, 370_000] as const;
const HOP_OUTPUTS = [500, 600, 100] as const;
const TURN_ACCUMULATED_INPUT = 1_080_000;
const TURN_ACCUMULATED_OUTPUT = 1_800;
const TURN_ELAPSED_MS = 5_000;

/**
 * wave-54 契约演进（ADR 0053）：三跳夹具的 turn 加权速度真值（C2 唯一口径）。
 *   Σout = 500 + 600 + 100 = 1200
 *   Σgen = (1000−150) + (1100−180) + (1200−200) = 850 + 920 + 1000 = 2770ms
 *   ⇒ speed = 1200 / 2.77 = 433.2129963898917 t/s（不再是末跳 per-call 的 100 t/s）。
 */
const WAVE54_THREE_HOP_WEIGHTED_SPEED = 433.2129963898917;

function record(partial: Partial<ProviderCallCacheRecord> = {}): ProviderCallCacheRecord {
  return {
    requestKind: "initial_request",
    providerSource: "primary",
    providerMode: "standard",
    latencyKind: "provider_stream",
    ...partial
  };
}

/** 正常三跳 per-call 记录：input 350K/360K/370K；末跳 output=100 / duration=1200ms / TTFT=200ms。 */
function threeHopRecords(): ProviderCallCacheRecord[] {
  return [
    record({
      requestKind: "initial_request",
      inputTokens: HOP_INPUTS[0],
      outputTokens: HOP_OUTPUTS[0],
      turnDurationMs: 1_000,
      firstTokenLatencyMs: 150,
      latencyKind: "provider_stream"
    }),
    record({
      requestKind: "tool_followup",
      inputTokens: HOP_INPUTS[1],
      outputTokens: HOP_OUTPUTS[1],
      turnDurationMs: 1_100,
      firstTokenLatencyMs: 180,
      latencyKind: "provider_stream"
    }),
    record({
      requestKind: "tool_followup",
      inputTokens: HOP_INPUTS[2],
      outputTokens: HOP_OUTPUTS[2],
      turnDurationMs: 1_200,
      firstTokenLatencyMs: 200,
      latencyKind: "provider_stream"
    })
  ];
}

function modelEntry(
  partial: Partial<TraceTimelineEntry> & Pick<TraceTimelineEntry, "id" | "sequence">
): TraceTimelineEntry {
  return {
    kind: "call_model",
    label: "CALL MODEL",
    state: "completed",
    ...partial
  };
}

/** R2 落地后的持久化 timeline：每条 call_model 携带自身指标 + checkpoint_persist 承载整轮聚合。 */
function persistedThreeHopTimeline(): TraceTimelineEntry[] {
  return [
    modelEntry({ id: "call-1", sequence: 1, inputTokens: HOP_INPUTS[0], outputTokens: HOP_OUTPUTS[0], turnDurationMs: 1_000, firstTokenLatencyMs: 150 }),
    modelEntry({ id: "call-2", sequence: 3, inputTokens: HOP_INPUTS[1], outputTokens: HOP_OUTPUTS[1], turnDurationMs: 1_100, firstTokenLatencyMs: 180 }),
    modelEntry({ id: "call-3", sequence: 5, inputTokens: HOP_INPUTS[2], outputTokens: HOP_OUTPUTS[2], turnDurationMs: 1_200, firstTokenLatencyMs: 200 }),
    {
      id: "checkpoint-6",
      kind: "checkpoint_persist",
      label: "CHECKPOINT",
      state: "completed",
      sequence: 6,
      inputTokens: TURN_ACCUMULATED_INPUT,
      outputTokens: TURN_ACCUMULATED_OUTPUT,
      turnDurationMs: TURN_ELAPSED_MS
    }
  ];
}

/** 活跃轮 live timeline 实况：条目尚无 per-call token 指标（RC4）。 */
function liveTimelineWithoutMetrics(): TraceTimelineEntry[] {
  return [
    modelEntry({ id: "live-call-1", sequence: 1, state: "completed" }),
    { id: "live-tool-1", kind: "call_tool", label: "TOOL", state: "completed", sequence: 2 },
    modelEntry({ id: "live-call-2", sequence: 3, state: "completed" }),
    { id: "live-tool-2", kind: "call_tool", label: "TOOL", state: "completed", sequence: 4 },
    modelEntry({ id: "live-call-3", sequence: 5, state: "completed" })
  ];
}

function completedTurn(partial: Partial<TurnTraceRecord> & Pick<TurnTraceRecord, "turnId">): TurnTraceRecord {
  return {
    title: "wave15",
    phase: "completed",
    traceSteps: [],
    toolActivities: [],
    updatedAt: 1,
    ...partial
  };
}

function userMessage(turnId: string): ChatMessage {
  return {
    id: `user-${turnId}`,
    turnId,
    role: "user",
    content: "wave15 活跃轮提问",
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

function projection() {
  return useTraceProjection({ liveTurnEnabled: ref(true) });
}

function flushMicrotasks() {
  return new Promise<void>((resolve) => window.setTimeout(resolve, 0));
}

type TurnEventHandler = (event: { payload: Record<string, unknown> }) => void;

/** 真实事件入口：mock 掉 @/lib/tauri 后由 store.initializeTurnEvents() 注册 handler。 */
async function installTurnEventIngress() {
  const handlers = new Map<string, TurnEventHandler>();
  tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
  tauriMocks.mockSafeInvoke.mockResolvedValue(null);
  tauriMocks.mockSafeListen.mockImplementation(async (eventName: string, handler: TurnEventHandler) => {
    handlers.set(eventName, handler);
    return () => {};
  });
  const store = useRuntimeStore();
  await store.initializeTurnEvents();
  return { store, handlers };
}

function seedActiveTurn(store: ReturnType<typeof useRuntimeStore>, timeline: TraceTimelineEntry[]) {
  store.$patch({
    sessionId: SESSION_ID,
    activeTurnId: ACTIVE_TURN_ID,
    isSubmitting: true,
    phase: "calling_model",
    inputTokens: 0,
    outputTokens: 0,
    totalTokens: 0,
    traceTimeline: timeline,
    messages: [userMessage(ACTIVE_TURN_ID)]
  });
}

function hopCompletePayload(overrides: Record<string, unknown> = {}) {
  return {
    turnId: ACTIVE_TURN_ID,
    sessionId: SESSION_ID,
    kind: "hop_complete",
    eventType: "turn.hop_complete",
    eventVersion: "1.0",
    eventId: "event-hop-2",
    sequence: 2,
    emittedAtMs: 2_000,
    inputTokens: TURN_ACCUMULATED_INPUT,
    outputTokens: TURN_ACCUMULATED_OUTPUT,
    totalTokens: TURN_ACCUMULATED_INPUT + TURN_ACCUMULATED_OUTPUT,
    firstTokenLatencyMs: 200,
    turnDurationMs: 1_200,
    providerCallRecords: threeHopRecords(),
    ...overrides
  };
}

describe("Wave-15 T1/T2 — S1 当前上下文输入 = 最近一次调用的 per-call input（严禁跨跳累加）", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it("T1：三跳已完成轮 → resolveCurrentContextInputTokens == 370000，且 != 1080000；圆环分子同值", () => {
    const turn = completedTurn({
      turnId: "turn-t1-multihop",
      inputTokens: TURN_ACCUMULATED_INPUT,
      outputTokens: TURN_ACCUMULATED_OUTPUT,
      turnDurationMs: TURN_ELAPSED_MS,
      providerCallRecords: threeHopRecords(),
      traceTimeline: persistedThreeHopTimeline()
    });

    expect(countTurnProviderCalls(turn)).toBe(3);
    expect(resolveCurrentContextInputTokens(turn)).toBe(370_000);
    expect(resolveCurrentContextInputTokens(turn)).not.toBe(TURN_ACCUMULATED_INPUT);

    const store = useRuntimeStore();
    store.$patch({
      inputTokens: TURN_ACCUMULATED_INPUT,
      turnTraceHistory: [turn],
      traceTimeline: turn.traceTimeline ?? [],
      activeTurnId: null,
      isSubmitting: false
    });

    const view = projection();
    // 输入框上下文圆环分子 + 状态栏「输入」共同消费 contextDisplayTokens。
    expect(view.contextDisplayTokens.value).toBe(370_000);
    expect(view.contextDisplayTokens.value).not.toBe(TURN_ACCUMULATED_INPUT);
    // S2 会话累计口径保持不变（回归锚点，§5 Non-Goal）。
    expect(view.sessionInputTokensTotal.value).toBe(TURN_ACCUMULATED_INPUT);
  });

  it("T2：活跃轮 hop_complete 写入累加值后，S1 仍取 live per-call 记录（370000），不得显示 1080000", async () => {
    const { store, handlers } = await installTurnEventIngress();
    seedActiveTurn(store, liveTimelineWithoutMetrics());
    const view = projection();

    handlers.get("turn:hop_complete")?.({
      payload: hopCompletePayload()
    });
    await flushMicrotasks();

    expect(view.contextDisplayTokens.value).toBe(370_000);
    expect(view.contextDisplayTokens.value).not.toBe(TURN_ACCUMULATED_INPUT);

    // 第二次 hop 完成（新 per-call 真值 380000 / 累加 1_460_000）→ S1 跟随最近一次调用。
    handlers.get("turn:hop_complete")?.({
      payload: hopCompletePayload({
        eventId: "event-hop-3",
        sequence: 3,
        inputTokens: 1_460_000,
        providerCallRecords: [
          ...threeHopRecords(),
          record({ requestKind: "tool_followup", inputTokens: 380_000, outputTokens: 100, turnDurationMs: 1_200, firstTokenLatencyMs: 200 })
        ]
      })
    });
    await flushMicrotasks();

    expect(view.contextDisplayTokens.value).toBe(380_000);
    expect(view.contextDisplayTokens.value).not.toBe(1_460_000);
  });
});

describe("Wave-15 T3/T4 — S3 单次调用生成速度（该次调用完成即产出）", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it("T3：output=100 / duration=1200 / TTFT=200（provider_stream）→ 100 t/s；退化与守卫分支正确", () => {
    const speedRecord = record({
      inputTokens: 370_000,
      outputTokens: 100,
      turnDurationMs: 1_200,
      firstTokenLatencyMs: 200,
      latencyKind: "provider_stream"
    });

    // (a) 主路径：100 / ((1200-200)/1000) = 100 t/s
    expect(resolveCallGenerationSpeed(speedRecord)).toBe(100);
    // 严禁由整轮累加值/整轮 elapsed 得出 360 t/s 或 1800/((5000-200)/1000)
    expect(resolveCallGenerationSpeed(speedRecord)).not.toBe(360);
    expect(resolveCallGenerationSpeed(speedRecord)).not.toBeCloseTo(375, 5);

    // (b) TTFT >= duration → 退化 duration 不抵扣：100 / (1200/1000) = 83.33
    expect(
      resolveCallGenerationSpeed(record({ outputTokens: 100, turnDurationMs: 1_200, firstTokenLatencyMs: 2_000 }))
    ).toBeCloseTo(83.33, 2);

    // (c) duration = 0 → null（不允许把 0 抬成 1 后除以 0 得 Infinity）
    expect(resolveCallGenerationSpeed(record({ outputTokens: 100, turnDurationMs: 0, firstTokenLatencyMs: 200 }))).toBeNull();
    // (d) duration 缺失 → null
    expect(resolveCallGenerationSpeed(record({ outputTokens: 100, turnDurationMs: null }))).toBeNull();
    // (e) output 缺失 → null
    expect(resolveCallGenerationSpeed(record({ outputTokens: null, turnDurationMs: 1_200 }))).toBeNull();
    // (f) 空记录 → null
    expect(resolveCallGenerationSpeed(null)).toBeNull();
    // (g) output = 0 是合法值（非 null）→ 0 t/s
    expect(resolveCallGenerationSpeed(record({ outputTokens: 0, turnDurationMs: 1_000 }))).toBe(0);
  });

  it("T3-投影：已完成三跳轮状态栏速度为 turn 加权口径 433.213 t/s（Σout/Σgen；不得用末跳 per-call 或整轮 elapsed）", () => {
    const turn = completedTurn({
      turnId: "turn-t3-speed",
      inputTokens: TURN_ACCUMULATED_INPUT,
      outputTokens: TURN_ACCUMULATED_OUTPUT,
      turnDurationMs: TURN_ELAPSED_MS,
      firstTokenLatencyMs: 200,
      providerCallRecords: threeHopRecords(),
      traceTimeline: persistedThreeHopTimeline()
    });

    const store = useRuntimeStore();
    store.$patch({
      turnTraceHistory: [turn],
      traceTimeline: turn.traceTimeline ?? [],
      activeTurnId: null,
      isSubmitting: false
    });

    const view = projection();
    // wave-54 C2 / ADR 0053：速度 = Σout/Σgen = 1200 / 2.77
    expect(view.sessionTokenGenerationSpeed.value).toBe(WAVE54_THREE_HOP_WEIGHTED_SPEED);
    expect(view.tokenSpeedCalculationMode.value).toBe("turn_weighted");
    // 判别式：整轮 elapsed 口径（1800/5.0 = 360）与末跳 per-call（100）都不得出现
    expect(view.sessionTokenGenerationSpeed.value).not.toBe(360);
    expect(view.sessionTokenGenerationSpeed.value).not.toBe(100);
  });

  it("T4：活跃轮仅收到中间 hop_complete（无 turn:completed）即产出 turn 加权速度 433.213（Σout/Σgen），不得为 null 或整轮值", async () => {
    const { store, handlers } = await installTurnEventIngress();
    seedActiveTurn(store, liveTimelineWithoutMetrics());
    const view = projection();

    // 收尾事件（turn:completed / output_end）尚未到达。
    expect(store.isSubmitting).toBe(true);
    handlers.get("turn:hop_complete")?.({
      payload: hopCompletePayload()
    });
    await flushMicrotasks();

    expect(store.isSubmitting).toBe(true);
    // wave-54 C2 / ADR 0053：活跃轮同样走 Σout/Σgen（1200 / 2.77），不是末跳 per-call 值
    expect(view.sessionTokenGenerationSpeed.value).toBe(WAVE54_THREE_HOP_WEIGHTED_SPEED);
    expect(view.tokenSpeedCalculationMode.value).toBe("turn_weighted");
    expect(view.sessionTokenGenerationSpeed.value).not.toBeNull();
    expect(view.sessionTokenGenerationSpeed.value).not.toBe(100);
    expect(view.sessionTokenGenerationSpeed.value).not.toBeCloseTo(375, 5);
  });
});

describe("Wave-15 T5/T6 — S4 降级纪律与 R2 载荷", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it("T5：多调用轮且无任何 per-call 来源（records 空 + 条目无指标）→ S1 无数据，不得等于累加值", () => {
    const turn = completedTurn({
      turnId: "turn-t5-no-per-call",
      inputTokens: TURN_ACCUMULATED_INPUT,
      outputTokens: TURN_ACCUMULATED_OUTPUT,
      turnDurationMs: TURN_ELAPSED_MS,
      providerCallRecords: [],
      traceTimeline: [
        modelEntry({ id: "t5-call-1", sequence: 1 }),
        modelEntry({ id: "t5-call-2", sequence: 3 }),
        modelEntry({ id: "t5-call-3", sequence: 5 })
      ]
    });

    expect(countTurnProviderCalls(turn)).toBe(3);
    expect(resolveCurrentContextInputTokens(turn)).toBeNull();
    expect(resolveCurrentContextInputTokens(turn)).not.toBe(TURN_ACCUMULATED_INPUT);

    const store = useRuntimeStore();
    store.$patch({
      inputTokens: TURN_ACCUMULATED_INPUT,
      turnTraceHistory: [turn],
      traceTimeline: turn.traceTimeline ?? [],
      activeTurnId: null,
      isSubmitting: false
    });

    const view = projection();
    expect(view.contextDisplayTokens.value).toBeNull();
    expect(view.contextDisplayTokens.value).not.toBe(TURN_ACCUMULATED_INPUT);
  });

  it("T6/R2：持久化 timeline 每条 call_model 携带自身指标、checkpoint_persist 承载整轮聚合", () => {
    const timeline = persistedThreeHopTimeline();
    const callEntries = timeline.filter((entry) => entry.kind === "call_model");
    const checkpoint = timeline.find((entry) => entry.kind === "checkpoint_persist");

    // 每条 call_model 条目 = 自身调用值（R2 / RC2 反例守卫）。
    expect(callEntries.map((entry) => entry.inputTokens)).toEqual([350_000, 360_000, 370_000]);
    expect(callEntries.map((entry) => entry.outputTokens)).toEqual([500, 600, 100]);
    expect(callEntries[2]?.turnDurationMs).toBe(1_200);
    expect(callEntries[0]?.inputTokens).not.toBe(TURN_ACCUMULATED_INPUT);
    // 整轮聚合只允许落在 checkpoint_persist。
    expect(checkpoint?.inputTokens).toBe(TURN_ACCUMULATED_INPUT);
    expect(checkpoint?.outputTokens).toBe(TURN_ACCUMULATED_OUTPUT);

    const turn = completedTurn({
      turnId: "turn-t6-persisted",
      inputTokens: TURN_ACCUMULATED_INPUT,
      outputTokens: TURN_ACCUMULATED_OUTPUT,
      turnDurationMs: TURN_ELAPSED_MS,
      providerCallRecords: threeHopRecords(),
      traceTimeline: timeline
    });

    expect(resolveCurrentContextInputTokens(turn)).toBe(370_000);
    const store = useRuntimeStore();
    store.$patch({ turnTraceHistory: [turn], traceTimeline: timeline, activeTurnId: null, isSubmitting: false });
    const view = projection();
    expect(view.contextDisplayTokens.value).toBe(370_000);
    // wave-54 C2 / ADR 0053：持久化 timeline 的 per-call 分量参与 Σout/Σgen 加权
    expect(view.sessionTokenGenerationSpeed.value).toBe(WAVE54_THREE_HOP_WEIGHTED_SPEED);
    expect(view.tokenSpeedCalculationMode.value).toBe("turn_weighted");
  });
});

describe("Wave-15 A1–A5 — 对抗（红相纯度：失败必须落在 SUT 栈）", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it("A1：provider 无 usage（全 None）→ 无崩溃、无 NaN、显示无数据", () => {
    expect(
      resolveCallGenerationSpeed(
        record({ inputTokens: null, outputTokens: null, turnDurationMs: null, firstTokenLatencyMs: null, latencyKind: "unknown" })
      )
    ).toBeNull();

    const turn = completedTurn({
      turnId: "turn-a1-none-usage",
      inputTokens: null,
      outputTokens: null,
      turnDurationMs: null,
      providerCallRecords: [record({ inputTokens: null, outputTokens: null, turnDurationMs: null, latencyKind: "unknown" })],
      traceTimeline: [modelEntry({ id: "a1-call-1", sequence: 1 }), modelEntry({ id: "a1-call-2", sequence: 3 })]
    });

    const resolved = resolveCurrentContextInputTokens(turn);
    expect(resolved).not.toBeUndefined();
    expect(Number.isNaN(resolved as number)).toBe(false);
    expect(resolved).toBeNull();

    const store = useRuntimeStore();
    store.$patch({ turnTraceHistory: [turn], traceTimeline: turn.traceTimeline ?? [], activeTurnId: null, isSubmitting: false });
    const view = projection();
    expect(view.contextDisplayTokens.value).toBeNull();
    expect(view.sessionTokenGenerationSpeed.value === null || Number.isFinite(view.sessionTokenGenerationSpeed.value)).toBe(true);
  });

  it("A2：畸形耗时（TTFT>=duration / duration=0 / duration=None / 非 stream 门控）→ 无负值、无 NaN/Infinity", () => {
    const cases: Array<ProviderCallCacheRecord> = [
      record({ outputTokens: 100, turnDurationMs: 1_200, firstTokenLatencyMs: 2_000, latencyKind: "provider_stream" }),
      record({ outputTokens: 100, turnDurationMs: 0, firstTokenLatencyMs: 0, latencyKind: "provider_stream" }),
      record({ outputTokens: 100, turnDurationMs: -5, firstTokenLatencyMs: 10, latencyKind: "provider_stream" }),
      record({ outputTokens: 100, turnDurationMs: null, firstTokenLatencyMs: 200, latencyKind: "provider_stream" }),
      record({ outputTokens: 100, turnDurationMs: 1_200, firstTokenLatencyMs: null, latencyKind: "provider_stream" }),
      // §7.2 TTFT 门控：非 provider_stream 时 TTFT 不可信，必须按 duration 全量计算（83.33，不得抵扣成 100）。
      record({ outputTokens: 100, turnDurationMs: 1_200, firstTokenLatencyMs: 200, latencyKind: "buffered_response" }),
      record({ outputTokens: 100, turnDurationMs: 1_200, firstTokenLatencyMs: 200, latencyKind: "unknown" })
    ];

    const results = cases.map((item) => resolveCallGenerationSpeed(item));
    for (const value of results) {
      expect(value === null || (Number.isFinite(value) && value >= 0)).toBe(true);
      expect(Number.isNaN(value as number)).toBe(false);
    }

    expect(results[0]).toBeCloseTo(83.33, 2); // TTFT >= duration → 退化
    expect(results[1]).toBeNull(); // duration = 0
    expect(results[2]).toBeNull(); // duration < 0
    expect(results[3]).toBeNull(); // duration = None
    expect(results[4]).toBeCloseTo(83.33, 2); // TTFT = None → 不抵扣
    expect(results[5]).toBeCloseTo(83.33, 2); // buffered_response → TTFT 不可信（门控）
    expect(results[5]).not.toBe(100);
    expect(results[6]).toBeCloseTo(83.33, 2); // unknown → TTFT 不可信（门控）
    expect(results[6]).not.toBe(100);
  });

  it("A3：重复/乱序同 step hop_complete 重放 → per-call 值幂等稳定，不重复计数", async () => {
    // 记录级：末条记录重复出现（同 step 重放）不得改变 S1。
    const duplicated = [...threeHopRecords(), threeHopRecords()[2]!];
    const turn = completedTurn({
      turnId: "turn-a3-duplicated",
      inputTokens: TURN_ACCUMULATED_INPUT,
      providerCallRecords: duplicated,
      traceTimeline: persistedThreeHopTimeline()
    });
    expect(resolveCurrentContextInputTokens(turn)).toBe(370_000);
    expect(resolveCurrentContextInputTokens(turn)).not.toBe(740_000);

    // 事件级：同 step 数据重放（不同 eventId/sequence，避开事件去重门）→ 展示值稳定。
    const { store, handlers } = await installTurnEventIngress();
    seedActiveTurn(store, liveTimelineWithoutMetrics());
    const view = projection();

    handlers.get("turn:hop_complete")?.({ payload: hopCompletePayload() });
    await flushMicrotasks();
    const first = view.contextDisplayTokens.value;

    handlers.get("turn:hop_complete")?.({
      payload: hopCompletePayload({ eventId: "event-hop-2-replay", sequence: 3, emittedAtMs: 2_100 })
    });
    await flushMicrotasks();

    expect(first).toBe(370_000);
    expect(view.contextDisplayTokens.value).toBe(370_000);
    // wave-54 C2 / ADR 0053：同 step 重放保持加权口径（1200 / 2.77）且幂等
    expect(view.sessionTokenGenerationSpeed.value).toBe(WAVE54_THREE_HOP_WEIGHTED_SPEED);
    expect(view.tokenSpeedCalculationMode.value).toBe("turn_weighted");
  });

  it("A4：u64::MAX 级数值 → saturating 语义下不 panic/不溢出，前端无 NaN/Infinity", () => {
    const huge = Number.MAX_SAFE_INTEGER;
    // 夹具自洽性（TEST-DEFECT 修正）：1 条 record + 空 timeline → countTurnProviderCalls = 1，
    // §7.6 步骤 1 前置条件（records.length===count）成立 → 命中该 record 的超大值。
    // 若配 3 条 call_model 条目，count=3 会触发 A5 伪聚合守卫而降级到步骤 2（370000），与本用例意图冲突。
    const turn = completedTurn({
      turnId: "turn-a4-huge",
      inputTokens: huge,
      outputTokens: huge,
      turnDurationMs: huge,
      providerCallRecords: [record({ inputTokens: huge, outputTokens: huge, turnDurationMs: 1_000, firstTokenLatencyMs: 1 })],
      traceTimeline: []
    });

    expect(countTurnProviderCalls(turn)).toBe(1);
    expect(resolveCurrentContextInputTokens(turn)).toBe(huge);
    expect(Number.isFinite(resolveCurrentContextInputTokens(turn) as number)).toBe(true);

    const speed = resolveCallGenerationSpeed(record({ outputTokens: huge, turnDurationMs: 1_000, firstTokenLatencyMs: 1 }));
    expect(speed).not.toBeNull();
    expect(Number.isFinite(speed as number)).toBe(true);
    expect(Number.isNaN(speed as number)).toBe(false);

    const store = useRuntimeStore();
    store.$patch({ turnTraceHistory: [turn], traceTimeline: turn.traceTimeline ?? [], activeTurnId: null, isSubmitting: false });
    const view = projection();
    expect(view.contextDisplayTokens.value).toBe(huge);
    expect(Number.isFinite(view.sessionTokenGenerationSpeed.value as number)).toBe(true);
  });

  it("A5-a：RC3 伪单次聚合记录（records=1 / 条目=3，且条目无指标）→ S1 = null，不得放行 1080000", () => {
    const turn = completedTurn({
      turnId: "turn-a5a-pseudo-aggregate",
      inputTokens: TURN_ACCUMULATED_INPUT,
      outputTokens: TURN_ACCUMULATED_OUTPUT,
      turnDurationMs: TURN_ELAPSED_MS,
      // RC3：legacy/graph 轮次回退构造的「伪单次记录」，input = 整轮累加。
      providerCallRecords: [
        record({
          requestKind: "initial_request",
          inputTokens: TURN_ACCUMULATED_INPUT,
          outputTokens: TURN_ACCUMULATED_OUTPUT,
          turnDurationMs: TURN_ELAPSED_MS,
          firstTokenLatencyMs: 200
        })
      ],
      traceTimeline: [
        modelEntry({ id: "a5a-call-1", sequence: 1 }),
        modelEntry({ id: "a5a-call-2", sequence: 3 }),
        modelEntry({ id: "a5a-call-3", sequence: 5 })
      ]
    });

    expect(countTurnProviderCalls(turn)).toBe(3);
    expect(resolveCurrentContextInputTokens(turn)).toBeNull();
    expect(resolveCurrentContextInputTokens(turn)).not.toBe(TURN_ACCUMULATED_INPUT);

    const store = useRuntimeStore();
    store.$patch({ turnTraceHistory: [turn], traceTimeline: turn.traceTimeline ?? [], activeTurnId: null, isSubmitting: false });
    const view = projection();
    expect(view.contextDisplayTokens.value).toBeNull();
    expect(view.contextDisplayTokens.value).not.toBe(TURN_ACCUMULATED_INPUT);
  });

  it("A5-b：同伪聚合记录 + timeline 3 条带 per-call 指标 → 降级到步骤 2 取 370000（而非 null 了事）", () => {
    const turn = completedTurn({
      turnId: "turn-a5b-pseudo-aggregate-with-timeline",
      inputTokens: TURN_ACCUMULATED_INPUT,
      outputTokens: TURN_ACCUMULATED_OUTPUT,
      turnDurationMs: TURN_ELAPSED_MS,
      providerCallRecords: [
        record({
          requestKind: "initial_request",
          inputTokens: TURN_ACCUMULATED_INPUT,
          outputTokens: TURN_ACCUMULATED_OUTPUT,
          turnDurationMs: TURN_ELAPSED_MS,
          firstTokenLatencyMs: 200
        })
      ],
      traceTimeline: persistedThreeHopTimeline()
    });

    expect(countTurnProviderCalls(turn)).toBe(3);
    expect(resolveCurrentContextInputTokens(turn)).toBe(370_000);
    expect(resolveCurrentContextInputTokens(turn)).not.toBe(TURN_ACCUMULATED_INPUT);

    const store = useRuntimeStore();
    store.$patch({ turnTraceHistory: [turn], traceTimeline: turn.traceTimeline ?? [], activeTurnId: null, isSubmitting: false });
    const view = projection();
    expect(view.contextDisplayTokens.value).toBe(370_000);
  });

  it("A5-守卫：records=3/条目=3 与 records=1/条目=1 均须走步骤 1（前置条件不得误伤正常 per-call 轮）", () => {
    const normalThree = completedTurn({
      turnId: "turn-a5-guard-three",
      inputTokens: TURN_ACCUMULATED_INPUT,
      providerCallRecords: threeHopRecords(),
      traceTimeline: persistedThreeHopTimeline()
    });
    expect(countTurnProviderCalls(normalThree)).toBe(3);
    expect(resolveCurrentContextInputTokens(normalThree)).toBe(370_000);

    const single = completedTurn({
      turnId: "turn-a5-guard-single",
      inputTokens: 12_500,
      providerCallRecords: [record({ inputTokens: 12_500, outputTokens: 400, turnDurationMs: 1_000 })],
      traceTimeline: [modelEntry({ id: "a5-single-call", sequence: 1, inputTokens: 12_500, outputTokens: 400, turnDurationMs: 1_000 })]
    });
    expect(countTurnProviderCalls(single)).toBe(1);
    expect(resolveCurrentContextInputTokens(single)).toBe(12_500);
  });
});
