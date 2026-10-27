/**
 * Wave-54 前端对抗验收（L2-AT）— A1 / A2 / A3
 *
 * 契约源（只读）：`.dev-team/contract-matrix-wave54.md`
 *   §2 T4–T6 之后的 **L2-AT** 段（A1 / A2 / A3）、§1 C2（gen_i 定义）、§1 C3（usage provenance）、
 *   §4 C2 前端 API 冻结。
 *
 * 纪律：固定输入、无随机、无 sleep、无 skip/todo、断言全部为具体数值。
 * 本文件由 Test Agent（test-ts）独占写域，红相锚定后冻结只读。
 *
 * 导入方式同 `token-speed-truth.spec.ts`：命名空间导入 + 「API 必须存在」守卫，
 * 使 A1/A2 的显示纪律用例在红相期仍能给出独立失败信号（缺 API 时显式抛契约错误）。
 */
import { beforeEach, describe, expect, it } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { defineComponent, h, type Component } from "vue";
import { mount } from "@vue/test-utils";
import { TooltipProvider } from "reka-ui";
import HomeStatusPanel from "@/components/HomeStatusPanel.vue";
import * as tokenMetricsModule from "@/lib/runtime/token-metrics";
import type { ProviderCallCacheRecord, TurnTraceRecord } from "@/types/runtime";
import fixtureCalls from "./fixtures/eed9ba47-calls.json";

// ── §4 C2 前端 API 冻结（桥接，不重定义语义）────────────────────────────────

type TokenSpeedCalculationMode = "turn_weighted" | "call_only" | "none";

interface TurnWeightedSpeedDetail {
  speed: number | null;
  mode: TokenSpeedCalculationMode;
  basis: "turn_weighted" | "call_only" | "none";
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

function resolveTurnWeightedSpeedDetail(
  turn: TurnTraceRecord | null | undefined,
  options?: { isActiveTurn?: boolean }
): TurnWeightedSpeedDetail {
  const candidate = (tokenMetricsModule as unknown as Record<string, unknown>)
    .resolveTurnWeightedSpeedDetail;
  if (typeof candidate !== "function") {
    throw new Error(
      "§4 C2 API 未落地：src/lib/runtime/token-metrics.ts 必须导出 " +
        "resolveTurnWeightedSpeedDetail(turn, options?)"
    );
  }
  return (candidate as TurnWeightedResolver)(turn, options);
}

// ── 构造工具 ─────────────────────────────────────────────────────────────────

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

function buildGoldenTurn(turnId: string): TurnTraceRecord {
  const providerCallRecords: ProviderCallCacheRecord[] = goldenCalls.map((call) => ({
    requestKind: "initial_request",
    latencyKind: "provider_stream",
    inputTokens: call.inputTokens,
    cacheHitInputTokens: call.cacheReadTokens,
    outputTokens: call.outputTokens,
    reasoningTokens: call.reasoningTokens,
    turnDurationMs: call.durationMs,
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
    inputTokens: 358633,
    outputTokens: 4542,
    turnDurationMs: 93154,
    firstTokenLatencyMs: 3047,
    usageOrigin: "provider",
    updatedAt: 1
  };
}

// ── A1：病态输入不得产出 NaN / Infinity / 负值 ────────────────────────────────

describe("wave-54 A1：病态输入（dur<=0 / usage 全空 / ttft>dur）", () => {
  it("A1-1: turnDurationMs=0 与 -500 ⇒ speed 必须为 null（不得 NaN/Infinity/负值）", () => {
    // Arrange
    const zeroDuration = buildTurn("wave54-a1-zero-dur", [
      {
        requestKind: "initial_request",
        latencyKind: "provider_stream",
        inputTokens: 1000,
        outputTokens: 100,
        turnDurationMs: 0,
        firstTokenLatencyMs: null,
        usageOrigin: "provider"
      }
    ]);
    const negativeDuration = buildTurn("wave54-a1-negative-dur", [
      {
        requestKind: "initial_request",
        latencyKind: "provider_stream",
        inputTokens: 1000,
        outputTokens: 100,
        turnDurationMs: -500,
        firstTokenLatencyMs: null,
        usageOrigin: "provider"
      }
    ]);

    // Act
    const zeroDetail = resolveTurnWeightedSpeedDetail(zeroDuration);
    const negativeDetail = resolveTurnWeightedSpeedDetail(negativeDuration);

    // Assert
    expect(zeroDetail.speed).toBeNull();
    expect(zeroDetail.mode).toBe("none");
    expect(negativeDetail.speed).toBeNull();
    expect(negativeDetail.mode).toBe("none");
  });

  it("A1-2: usage 全空（无 outputTokens / 无任何 record）⇒ speed 必须为 null", () => {
    // Arrange：有耗时但无 outputTokens
    const noUsage = buildTurn("wave54-a1-no-usage", [
      {
        requestKind: "initial_request",
        latencyKind: "provider_stream",
        inputTokens: 1000,
        turnDurationMs: 1200,
        firstTokenLatencyMs: 200,
        usageOrigin: "provider"
      }
    ]);
    // Arrange：完全无 per-call 记录，仅有 turn 级聚合字段
    const noRecords = buildTurn(
      "wave54-a1-no-records",
      [],
      { inputTokens: 1000, outputTokens: 100, turnDurationMs: 1200 }
    );

    // Act
    const noUsageDetail = resolveTurnWeightedSpeedDetail(noUsage);
    const noRecordsDetail = resolveTurnWeightedSpeedDetail(noRecords);

    // Assert
    expect(noUsageDetail.speed).toBeNull();
    expect(noUsageDetail.mode).toBe("none");
    expect(noRecordsDetail.speed).toBeNull();
    expect(noRecordsDetail.mode).toBe("none");
  });

  it("A1-3: ttft(1500) > dur(1000)、out=100 ⇒ 按 C2 退化 gen=dur ⇒ speed=100（有限正值，不是 null）", () => {
    // Arrange：Lead 裁决 2026-10-27 20:40 —— ttft 不可信时退化为不抵扣
    const untrustedTtft = buildTurn("wave54-a1-ttft-gt-dur", [
      {
        requestKind: "initial_request",
        latencyKind: "provider_stream",
        inputTokens: 1000,
        outputTokens: 100,
        turnDurationMs: 1000,
        firstTokenLatencyMs: 1500,
        usageOrigin: "provider"
      }
    ]);

    // Act
    const detail = resolveTurnWeightedSpeedDetail(untrustedTtft);

    // Assert
    expect(detail.speed).not.toBeNull();
    expect(detail.speed).toBe(100);
    expect(Number.isFinite(detail.speed as number)).toBe(true);
    expect(detail.mode).toBe("turn_weighted");
    expect(detail.basis).toBe("turn_weighted");
    expect(detail.genMs).toBe(1000);
    expect(detail.calls).toBe(1);
  });
});

// ── A2：provider 与 estimated_chars 混合 ─────────────────────────────────────

function mountStatusPanel(props: Record<string, unknown>) {
  const Host = defineComponent({
    name: "Wave54AdversarialTooltipHost",
    setup() {
      return () => h(TooltipProvider, {}, { default: () => h(HomeStatusPanel as Component, props) });
    }
  });
  return mount(Host, { global: { stubs: { teleport: true } } });
}

const STATUS_PANEL_BASE_PROPS: Record<string, unknown> = {
  sessionId: "session-wave54-adversarial",
  turnCount: 1,
  modelCallCount: 2,
  toolCallCount: 0,
  copied: false,
  inputTokensTotal: 9000,
  outputTokensTotal: 300,
  cacheHitTokensTotal: 4200,
  cacheHitRatio: "47%",
  tokenGenerationSpeed: 107.14,
  tokenSpeedCalculationMode: "turn_weighted",
  speedDebugInfo: { mode: "turn_weighted", speed: 107.14, origin: "provider" },
  showContextUsage: true,
  contextDisplayTokens: 5000,
  contextWindowTokens: 200_000
};

describe("wave-54 A2：estimated_chars 与 provider 混合", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it("A2-1: 混合 origin 的 turn ⇒ speed 仍有限且为正，整体 origin 降级为 estimated_chars（不冒充真值）", () => {
    // Arrange：调用 1 为 provider 真值，调用 2 为字符估算
    const mixedTurn = buildTurn("wave54-a2-mixed-origin", [
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
        usageOrigin: "estimated_chars"
      }
    ]);

    // Act
    const detail = resolveTurnWeightedSpeedDetail(mixedTurn);

    // Assert：数值方向不变（仍是 Σout/Σgen = 107.14），但溯源必须降级为估算
    expect(detail.speed).not.toBeNull();
    expect(Number.isFinite(detail.speed as number)).toBe(true);
    expect(detail.speed as number).toBeGreaterThan(0);
    expect(Math.abs((detail.speed as number) - 107.142857)).toBeLessThanOrEqual(0.01);
    expect(detail.calls).toBe(2);
    expect(detail.origin).toBe("estimated_chars");
  });

  it("A2-2: `≈` 只出现在 estimated 分量 —— provider 真值不得加 `≈`，estimated 必须加", () => {
    // Arrange
    const providerWrapper = mountStatusPanel({
      ...STATUS_PANEL_BASE_PROPS,
      speedDebugInfo: { mode: "turn_weighted", speed: 107.14, origin: "provider" }
    });
    const estimatedWrapper = mountStatusPanel({
      ...STATUS_PANEL_BASE_PROPS,
      speedDebugInfo: { mode: "turn_weighted", speed: 107.14, origin: "estimated_chars" }
    });

    // Act
    const providerSpeedText = providerWrapper
      .get('[data-testid="status-metric-token-speed"]')
      .text();
    const estimatedSpeedText = estimatedWrapper
      .get('[data-testid="status-metric-token-speed"]')
      .text();

    // Assert：真值不加 `≈`，估算值必须加（区分「真值」与「估算」，不得静默等同）
    expect(providerSpeedText).not.toContain("≈");
    expect(estimatedSpeedText).toContain("≈");
    expect(estimatedSpeedText).toMatch(/≈\s*\d/);

    providerWrapper.unmount();
    estimatedWrapper.unmount();
  });
});

// ── A3：同一序列重复 fold 10 次幂等（无随机、无 sleep）──────────────────────

describe("wave-54 A3：同序列 10 次重放幂等", () => {
  it("A3-1: 同一 golden fixture 连续 10 次 resolve ⇒ 结果完全一致（深比较）", () => {
    // Arrange
    const turn = buildGoldenTurn("wave54-a3-idempotent");
    const baseline = resolveTurnWeightedSpeedDetail(turn);
    const results: TurnWeightedSpeedDetail[] = [];

    // Act
    for (let run = 0; run < 10; run += 1) {
      results.push(resolveTurnWeightedSpeedDetail(turn));
    }

    // Assert
    expect(results).toHaveLength(10);
    expect(baseline.speed).not.toBeNull();
    for (const result of results) {
      expect(result).toEqual(baseline);
    }
    // 数值自身不漂移（非近似比较之外还要求位级相等）
    const speeds = results.map((result) => result.speed);
    expect(new Set(speeds).size).toBe(1);
  });

  it("A3-2: 每次重放都基于同一输入不产生副作用（输入对象未被改写）", () => {
    // Arrange
    const turn = buildGoldenTurn("wave54-a3-purity");
    const snapshotRecords = JSON.stringify(turn.providerCallRecords);
    const snapshotTurn = JSON.stringify(turn);

    // Act
    for (let run = 0; run < 10; run += 1) {
      resolveTurnWeightedSpeedDetail(turn);
    }

    // Assert：纯函数不得就地改写入参（否则第二次调用结果会漂移）
    expect(JSON.stringify(turn.providerCallRecords)).toBe(snapshotRecords);
    expect(JSON.stringify(turn)).toBe(snapshotTurn);
  });
});
