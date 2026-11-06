/**
 * Wave-54 前端验收（红相锚定，L3-AT）— T4 / T5 / T6
 *
 * 契约源（只读）：`.dev-team/contract-matrix-wave54.md`
 *   §1 C2（速度唯一口径）、§1 C4（上下文占用归一）、§1 C6（显示纪律）、
 *   §2 T4 / T5 / T6、§4 C2 前端 API 冻结。
 *
 * 本文件由 Test Agent（test-ts）独占写域，红相锚定后冻结只读；
 * 任何修改需 Lead 批准。禁止 skip / todo / sleep / 随机数 / 空断言。
 *
 * 关于导入方式（有意为之，非权宜）：
 *   §4 冻结 API `resolveTurnWeightedSpeedDetail` 在红相期尚未落地。若用静态具名导入，
 *   整个文件会在收集阶段直接失败，导致同文件内的 T5（显示纪律，只依赖既有 props）与
 *   T6 的 fixture 完整性断言无法给出独立失败信号。因此改为命名空间导入 + 显式
 *   「API 必须存在」守卫：缺 API 时抛出带契约条款的明确错误（不是吞异常），
 *   API 落地后行为与静态导入等价（改名/删除同样会被本守卫捕获）。
 */
import { beforeEach, describe, expect, it } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { defineComponent, h, ref, type Component } from "vue";
import { mount } from "@vue/test-utils";
import { TooltipProvider } from "reka-ui";
import HomeStatusPanel from "@/components/HomeStatusPanel.vue";
import { useRuntimeStore } from "@/stores/runtime";
import { useTraceProjection } from "@/lib/runtime/useTraceProjection";
import * as tokenMetricsModule from "@/lib/runtime/token-metrics";
import type {
  ProviderCallCacheRecord,
  TraceTimelineEntry,
  TurnTraceRecord
} from "@/types/runtime";
import fixtureCalls from "./fixtures/eed9ba47-calls.json";

// ── §4 C2 前端 API 冻结（形状即契约，此处仅做运行期桥接，不重定义语义）──────────

type TokenSpeedCalculationMode = "turn_weighted" | "call_only" | "none";

interface TurnWeightedSpeedDetail {
  speed: number | null;
  mode: TokenSpeedCalculationMode;
  /** 口径域（token 速度口径 v2 / I10）：测的是什么，与 mode 的聚合范围不同域。 */
  basis: "stream_decode" | "end_to_end" | "none";
  calls: number;
  outputTokens: number | null;
  genMs: number | null;
  inputTokens: number | null;
  cacheInputTokens: number | null;
  ttftMs: number | null;
  origin: string | null;
}

type TurnWeightedResolver = (
  turn: TurnTraceRecord | null | undefined,
  options?: { isActiveTurn?: boolean }
) => TurnWeightedSpeedDetail;

const RESOLVER_MISSING_MESSAGE =
  "§4 C2 API 未落地：src/lib/runtime/token-metrics.ts 必须导出 " +
  "resolveTurnWeightedSpeedDetail(turn, options?)";

/** 调用 §4 冻结 API；未落地时以契约条款为消息显式失败（不吞异常、不降级）。 */
function resolveTurnWeightedSpeedDetail(
  turn: TurnTraceRecord | null | undefined,
  options?: { isActiveTurn?: boolean }
): TurnWeightedSpeedDetail {
  const candidate = (tokenMetricsModule as unknown as Record<string, unknown>)
    .resolveTurnWeightedSpeedDetail;
  if (typeof candidate !== "function") {
    throw new Error(RESOLVER_MISSING_MESSAGE);
  }
  return (candidate as TurnWeightedResolver)(turn, options);
}

// ── 固件与构造工具 ────────────────────────────────────────────────────────────

type FixtureCall = {
  call: number;
  durationMs: number;
  outputTokens: number;
  inputTokens: number;
  cacheReadTokens: number;
  reasoningTokens: number;
  genMs: number;
  ttftMs: number;
  speedAttr: number;
};

const goldenCalls = fixtureCalls as FixtureCall[];

/** golden trace 的整轮聚合真值（由 fixture 直接求和，见 T6 完整性用例）。 */
const FIXTURE_TOTALS = {
  calls: 20,
  outputTokens: 4542,
  inputTokens: 358633,
  cacheReadTokens: 327808,
  reasoningTokens: 2075,
  /** Σgen 以 fixture 逐条 `durationMs - ttftMs` 求和为准：47579ms（容差 ±60 保留）。 */
  genMs: 47579,
  durationMs: 93154,
  firstCallTtftMs: 3047
} as const;

function buildGoldenTurn(turnId: string): TurnTraceRecord {
  const providerCallRecords: ProviderCallCacheRecord[] = goldenCalls.map((call) => ({
    requestKind: "initial_request",
    latencyKind: "provider_stream",
    inputTokens: call.inputTokens,
    cacheHitInputTokens: call.cacheReadTokens,
    outputTokens: call.outputTokens,
    reasoningTokens: call.reasoningTokens,
    turnDurationMs: call.durationMs,
    // 契约 C1：ttft 仅「有值」时存在；fixture 中 20 次调用均带 ttft。
    firstTokenLatencyMs: call.ttftMs,
    usageOrigin: "provider"
  }));

  return {
    turnId,
    title: "golden replay eed9ba47",
    phase: "completed",
    traceSteps: [],
    traceTimeline: [],
    toolActivities: [],
    providerCallRecords,
    inputTokens: FIXTURE_TOTALS.inputTokens,
    outputTokens: FIXTURE_TOTALS.outputTokens,
    turnDurationMs: FIXTURE_TOTALS.durationMs,
    firstTokenLatencyMs: FIXTURE_TOTALS.firstCallTtftMs,
    usageOrigin: "provider",
    updatedAt: 1
  };
}

function buildTurn(
  turnId: string,
  providerCallRecords: ProviderCallCacheRecord[],
  overrides: Partial<TurnTraceRecord> = {}
): TurnTraceRecord {
  return {
    turnId,
    title: turnId,
    phase: "completed",
    traceSteps: [],
    traceTimeline: [],
    toolActivities: [],
    providerCallRecords,
    updatedAt: 1,
    ...overrides
  };
}

// ── T4 前端单口径（turn 加权生成速度）────────────────────────────────────────

describe("wave-54 T4：resolveTurnWeightedSpeedDetail 单口径（解码口径 Σ(out−1) / Σgen，无合格样本回退 Σout / Σdur）", () => {
  it("T4-1: 两调用 (out100,dur1200,ttft200) + (out200,dur1800,ttft=null) ⇒ speed=99、mode=turn_weighted/basis=stream_decode、calls=1、genMs=1000、outputTokens=100", () => {
    // Arrange：两条 per-call 记录（第二条无 ttft ⇒ 不可解码，I3 排除出分母）
    const turn = buildTurn("wave54-t4-two-calls", [
      {
        requestKind: "tool_followup",
        latencyKind: "provider_stream",
        inputTokens: 1000,
        outputTokens: 100,
        turnDurationMs: 1200,
        firstTokenLatencyMs: 200,
        usageOrigin: "provider"
      },
      {
        requestKind: "initial_request",
        latencyKind: "provider_stream",
        inputTokens: 2000,
        outputTokens: 200,
        turnDurationMs: 1800,
        firstTokenLatencyMs: null,
        usageOrigin: "provider"
      }
    ]);

    // Act
    const detail = resolveTurnWeightedSpeedDetail(turn);

    // Assert：(100 − 1) / (1000 / 1000) = 99（旧口径 300/2800 = 107.142857 已废止）
    expect(detail.speed).not.toBeNull();
    expect(Math.abs((detail.speed as number) - 99)).toBeLessThanOrEqual(0.01);
    expect(detail.mode).toBe("turn_weighted");
    expect(detail.basis).toBe("stream_decode");
    expect(detail.calls).toBe(1);
    expect(detail.genMs).toBe(1000);
    expect(detail.outputTokens).toBe(100);
    // §4：ttftMs = 首次调用 TTFT（不跨 hop 采样）
    expect(detail.ttftMs).toBe(200);
  });

  it("T4-2: 单调用 ⇒ detail 形状完整（input/cache/ttft/origin 取该次调用真值）", () => {
    // Arrange：单调用，contextInputTokens 与 inputTokens 同值，消除 C4 口径歧义
    const turn = buildTurn("wave54-t4-single-call", [
      {
        requestKind: "initial_request",
        latencyKind: "provider_stream",
        inputTokens: 5000,
        cacheHitInputTokens: 2500,
        outputTokens: 250,
        turnDurationMs: 1500,
        firstTokenLatencyMs: 500,
        usageOrigin: "provider",
        contextInputTokens: 5000
      }
    ]);

    // Act
    const detail = resolveTurnWeightedSpeedDetail(turn);

    // Assert：gen = 1500 - 500 = 1000ms ⇒ (250 − 1) / 1s = 249 t/s（分子扣 1：首 token 已计入 TTFT）
    expect(detail.speed).not.toBeNull();
    expect(Math.abs((detail.speed as number) - 249)).toBeLessThanOrEqual(1e-9);
    expect(detail.mode).toBe("turn_weighted");
    expect(detail.basis).toBe("stream_decode");
    expect(detail.calls).toBe(1);
    expect(detail.genMs).toBe(1000);
    expect(detail.outputTokens).toBe(250);
    expect(detail.inputTokens).toBe(5000);
    expect(detail.cacheInputTokens).toBe(2500);
    expect(detail.ttftMs).toBe(500);
    expect(detail.origin).toBe("provider");
  });

  it("T4-3: 空 turn（null / undefined / 无 per-call 记录）⇒ speed=null、mode/basis=none、calls=0", () => {
    // Act
    const fromNull = resolveTurnWeightedSpeedDetail(null);
    const fromUndefined = resolveTurnWeightedSpeedDetail(undefined);
    const fromEmpty = resolveTurnWeightedSpeedDetail(buildTurn("wave54-t4-empty", []));

    // Assert
    for (const detail of [fromNull, fromUndefined, fromEmpty]) {
      expect(detail.speed).toBeNull();
      expect(detail.mode).toBe("none");
      expect(detail.basis).toBe("none");
      expect(detail.calls).toBe(0);
    }
  });

  it("T4-4（[BLOCKING 7]）: 无已完成调用但存在活跃调用记录 ⇒ mode/basis=call_only、calls=0、speed=out/(dur−ttft)", () => {
    // Arrange：§4 `call_only` 分支 = `calls === 0` 且存在「活跃调用」的 per-call 值。
    // - `providerCallRecords` 为空 ⇒ 无已完成调用（records 是调用完成后的 per-call 真值载体）；
    // - live timeline 的 call_model 条目携带当前活跃调用（state=running）的 per-call 指标；
    // - 轮次处于活跃态（isActiveTurn=true）。
    const activeCallTurn = buildTurn("wave54-t4-call-only", [], {
      phase: "calling_model",
      traceTimeline: [
        {
          id: "wave54-live-call-1",
          turnId: "wave54-t4-call-only",
          sequence: 1,
          kind: "call_model",
          label: "CALL MODEL #1",
          state: "running",
          inputTokens: 4000,
          outputTokens: 150,
          turnDurationMs: 2000,
          firstTokenLatencyMs: 500
        }
      ]
    });

    // Act
    const detail = resolveTurnWeightedSpeedDetail(activeCallTurn, { isActiveTurn: true });

    // Assert：gen = 2000 − 500 = 1500ms ⇒ (150 − 1) / 1.5s ≈ 99.333 t/s（该 per-call 值，而非 Σ 加权）
    expect(detail.speed).not.toBeNull();
    expect(Math.abs((detail.speed as number) - 149 / 1.5)).toBeLessThanOrEqual(1e-9);
    expect(detail.mode).toBe("call_only");
    expect(detail.basis).toBe("stream_decode");
    expect(detail.calls).toBe(0);
    expect(detail.outputTokens).toBe(150);
    expect(detail.genMs).toBe(1500);
    expect(detail.ttftMs).toBe(500);
  });
});

// ── T5 显示纪律（HomeStatusPanel 既有 props 面）───────────────────────────────

function mountStatusPanel(props: Record<string, unknown>) {
  const Host = defineComponent({
    name: "Wave54TooltipHost",
    setup() {
      // Tooltip.vue 依赖 reka-ui TooltipProvider context，缺失会在 setup 抛错。
      return () => h(TooltipProvider, {}, { default: () => h(HomeStatusPanel as Component, props) });
    }
  });
  return mount(Host, { global: { stubs: { teleport: true } } });
}

const STATUS_PANEL_BASE_PROPS: Record<string, unknown> = {
  sessionId: "session-wave54",
  turnCount: 3,
  modelCallCount: 5,
  toolCallCount: 2,
  copied: false,
  inputTokensTotal: 1875,
  outputTokensTotal: 4542,
  cacheHitTokensTotal: 327808,
  cacheHitRatio: "91%",
  tokenGenerationSpeed: 95.46,
  tokenSpeedCalculationMode: "turn_weighted",
  speedDebugInfo: { mode: "turn_weighted", speed: 95.46, origin: "provider" },
  showContextUsage: true,
  contextDisplayTokens: 11143,
  contextWindowTokens: 200_000
};

describe("wave-54 T5：HomeStatusPanel 显示纪律（C6）", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  /**
   * wave-61 §5 授权迁移（Lead 一票，原意图保真）：
   *   原意图：`inputTokensTotal=null` 时输入位**不得渲染 0**（禁止把"无数据"伪装成"0 token"）。
   *   迁移理由：wave-61 §2 E-2/E-5 把空值表达从占位文本 `—` 改为「位点整个不渲染」，
   *             §2 E-5 明令禁止 `—` 占位文本（`formatCompactInteger` 的 `—` 分支随之作废）。
   *   等价断言：输入位整个 DOM 节点不存在（既无 `0`、也无 `—`）＋ 其余 token 位反向哨兵仍在。
   */
  it("T5-1: inputTokensTotal=null ⇒ 输入位整个 DOM 不渲染（禁止渲染 0 / —）", () => {
    // Arrange
    const wrapper = mountStatusPanel({ ...STATUS_PANEL_BASE_PROPS, inputTokensTotal: null });

    // Act
    const inputCarrier = wrapper.find('[data-testid="status-metric-input-tokens"]');

    // Assert（wave-61 §5 迁移后的等价断言：位点不存在）
    expect(
      inputCarrier.exists(),
      "T5-1：inputTokensTotal=null ⇒ 输入位整个 DOM 不渲染（禁止渲染 0 / —）"
    ).toBe(false);
    expect(
      wrapper.findAll('[data-testid="status-metric-input-tokens"]'),
      "T5-1：输入位不得以任何形态残留在 DOM"
    ).toHaveLength(0);

    // Assert（反向哨兵：其余 token 位仍有值 ⇒ 不是整行塌陷掩盖的结果）
    expect(
      wrapper.find('[data-testid="status-metric-output-tokens"]').exists(),
      "T5-1：输出位仍有真值，必须继续渲染"
    ).toBe(true);

    // Assert（E-5：面板内不得出现占位文本）
    expect(wrapper.text(), "T5-1：面板内不得出现 `—` 占位文本").not.toContain("—");

    wrapper.unmount();
  });

  it("T5-2: inputTokensTotal=1875 ⇒ 紧凑格式 `1.9K`（<1e6 一位小数）", () => {
    // Arrange
    const wrapper = mountStatusPanel({ ...STATUS_PANEL_BASE_PROPS, inputTokensTotal: 1875 });

    // Act
    const inputCarrier = wrapper.get('[data-testid="status-metric-input-tokens"]');

    // Assert
    expect(inputCarrier.text().trim()).toBe("1.9K");

    wrapper.unmount();
  });

  it("T5-3: speedDebugInfo.origin=estimated_chars ⇒ 速度值带 `≈` 前缀（估算不得冒充真值）", () => {
    // Arrange
    const wrapper = mountStatusPanel({
      ...STATUS_PANEL_BASE_PROPS,
      speedDebugInfo: {
        mode: "turn_weighted",
        speed: 95.46,
        origin: "estimated_chars"
      }
    });

    // Act
    const speedCarrier = wrapper.get('[data-testid="status-metric-token-speed"]');

    // Assert：`≈` 必须紧贴速度数值（如 `≈95 t/s`），而不是任意位置的装饰字符
    expect(speedCarrier.text()).toContain("≈");
    expect(speedCarrier.text()).toMatch(/≈\s*\d/);

    wrapper.unmount();
  });
});

describe("wave-54 T5：缓存比与可见输入同源（C4/C6，projection harness）", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  /**
   * 单轮两调用：
   * - 调用 1：input 1000 / contextInputTokens 1200 / cache 200
   * - 调用 2（末次）：input 8000 / contextInputTokens 5000 / cache 4000
   * 会话累计 inputTokens = 9000（账单口径，**不得**用作缓存比分母）。
   * 期望：可见输入 = 5000（C4 优先 contextInputTokens），缓存比 = 4000/5000 = 80%。
   * （若按旧口径 9000 或 per-call input 8000 计算，比值分别为 47% / 50%，故本断言可判别同源。）
   */
  const timeline: TraceTimelineEntry[] = [
    {
      id: "call-1",
      turnId: "wave54-t5-cache-ratio",
      sequence: 1,
      kind: "call_model",
      label: "CALL MODEL #1",
      state: "completed",
      inputTokens: 1000,
      cacheHitInputTokens: 200,
      outputTokens: 100,
      turnDurationMs: 1200,
      firstTokenLatencyMs: 200
    },
    {
      id: "call-2",
      turnId: "wave54-t5-cache-ratio",
      sequence: 2,
      kind: "call_model",
      label: "CALL MODEL #2",
      state: "completed",
      inputTokens: 8000,
      cacheHitInputTokens: 4000,
      outputTokens: 200,
      turnDurationMs: 1800
    }
  ];

  const turnRecord: TurnTraceRecord = {
    turnId: "wave54-t5-cache-ratio",
    title: "缓存比同源",
    phase: "completed",
    traceSteps: [],
    traceTimeline: timeline,
    toolActivities: [],
    providerCallRecords: [
      {
        requestKind: "tool_followup",
        latencyKind: "provider_stream",
        inputTokens: 1000,
        cacheHitInputTokens: 200,
        outputTokens: 100,
        turnDurationMs: 1200,
        firstTokenLatencyMs: 200,
        usageOrigin: "provider",
        contextInputTokens: 1200
      },
      {
        requestKind: "initial_request",
        latencyKind: "provider_stream",
        inputTokens: 8000,
        cacheHitInputTokens: 4000,
        outputTokens: 200,
        turnDurationMs: 1800,
        usageOrigin: "provider",
        contextInputTokens: 5000
      }
    ],
    inputTokens: 9000,
    outputTokens: 300,
    updatedAt: 2
  };

  function mountProjection() {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      inputTokens: 9000,
      turnTraceHistory: [turnRecord],
      traceTimeline: [],
      activeTurnId: null,
      isSubmitting: false
    });
    return useTraceProjection({ liveTurnEnabled: ref(true) });
  }

  it("T5-4: contextDisplayTokens 取末次调用 contextInputTokens 优先（C4 取值序）", () => {
    // Act
    const projection = mountProjection();

    // Assert
    expect(projection.contextDisplayTokens.value).toBe(5000);
  });

  it("T5-5: sessionCacheHitRatio = 末次调用 cache / 同源可见输入（4000/5000 = 80%）", () => {
    // Act
    const projection = mountProjection();

    // Assert：分母必须与可见「输入」同源（5000），而不是会话累计 9000（47%）或 per-call input 8000（50%）
    expect(projection.contextDisplayTokens.value).toBe(5000);
    expect(projection.sessionCacheHitRatio.value).toBe("80%");
    // 会话累计口径保持不变（账单语义），仅缓存比改为同源（C4/C6）
    expect(projection.sessionInputTokensTotal.value).toBe(9000);
  });
});

// ── T6 golden replay（真实 trace eed9ba47…，20 次调用）───────────────────────

describe("wave-54 T6：golden replay eed9ba47（20 次调用）", () => {
  /**
   * 口径区分（token 速度口径 v2 / ADR 0073 强制注释）：
   *   - 「解码速度」= Σ_eligible(out−1) / Σ_eligible gen，gen **不含** TTFT，分子扣 1/token
   *     （首 token 已计入 TTFT）。本 fixture 20 条全部 `decode_eligible`
   *     ⇒ (4542 − 20) / 47.579s ≈ 95.042 t/s。
   *   - 端到端吞吐 = Σout / Σdur（含排队 + prefill）≈ 48.758 t/s 属另一量纲，
   *     basis=end_to_end 时消费面必须加 `≈`；本用例不得退化为该值。
   *   - 旧口径 Σout / Σgen = 95.46 t/s（分母混入不可解码调用、分子未扣首 token）已废止。
   */
  it("T6-0: fixture 完整性 —— Σout=4542、Σin=358633、Σcache=327808 由 fixture 直接求和", () => {
    // Act
    const sumOutput = goldenCalls.reduce((sum, call) => sum + call.outputTokens, 0);
    const sumInput = goldenCalls.reduce((sum, call) => sum + call.inputTokens, 0);
    const sumCache = goldenCalls.reduce((sum, call) => sum + call.cacheReadTokens, 0);

    // Assert（防 fixture 漂移）
    expect(goldenCalls).toHaveLength(FIXTURE_TOTALS.calls);
    expect(sumOutput).toBe(FIXTURE_TOTALS.outputTokens);
    expect(sumInput).toBe(FIXTURE_TOTALS.inputTokens);
    expect(sumCache).toBe(FIXTURE_TOTALS.cacheReadTokens);
  });

  it("T6-1: fixture 自洽 —— 逐条 genMs === durationMs - ttftMs（C2 的 gen_i 定义），且 Σgen=47579（容差 ±60）", () => {
    // Act
    const genDelta = goldenCalls.map((call) => call.durationMs - call.ttftMs - call.genMs);
    const sumGen = goldenCalls.reduce((sum, call) => sum + call.genMs, 0);
    const sumReasoning = goldenCalls.reduce((sum, call) => sum + call.reasoningTokens, 0);

    // Assert
    expect(genDelta).toEqual(goldenCalls.map(() => 0));
    expect(sumGen).toBe(FIXTURE_TOTALS.genMs);
    expect(Math.abs(sumGen - 47579)).toBeLessThanOrEqual(60);
    expect(sumReasoning).toBe(FIXTURE_TOTALS.reasoningTokens);
  });

  it("T6-2: 20 次调用 replay ⇒ speed=(4542−20)/47.579≈95.042、basis=stream_decode、outputTokens=4542、genMs=47579", () => {
    // Arrange
    const turn = buildGoldenTurn("wave54-t6-golden");

    // Act
    const detail = resolveTurnWeightedSpeedDetail(turn);

    // Assert
    expect(detail.speed).not.toBeNull();
    const speed = detail.speed as number;
    expect(Number.isFinite(speed)).toBe(true);
    expect(Math.abs(speed - 95.04193026335147)).toBeLessThanOrEqual(1e-6);
    expect(detail.basis).toBe("stream_decode");
    expect(detail.mode).toBe("turn_weighted");
    expect(detail.calls).toBe(FIXTURE_TOTALS.calls);
    expect(detail.outputTokens).toBe(FIXTURE_TOTALS.outputTokens);
    expect(detail.genMs).not.toBeNull();
    expect(Math.abs((detail.genMs as number) - 47579)).toBeLessThanOrEqual(60);
    // §1 C2：turn 级 TTFT = 首个带 ttft 的 per-call 值（不跨 hop 采样）
    expect(detail.ttftMs).toBe(FIXTURE_TOTALS.firstCallTtftMs);
    // 判别：不得退化为端到端吞吐 4542/Σdur ≈ 48.758
    expect(Math.abs(speed - 48.7580)).toBeGreaterThan(1.0);
    // 判别：不得退化为旧口径 Σout/Σgen = 95.46（分子未扣首 token）
    expect(Math.abs(speed - 95.46)).toBeGreaterThan(0.1);
  });
});
