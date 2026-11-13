/**
 * [diag-tps / task-4] F1「显示端口径诚实化」验收（独立测试者 L2-T，红相先写、冻结）。
 *
 * 契约真值源（Lead 冻结，只读）：共享任务板 `task-4` 的 I4。
 *   「显示端必须能表达『样本量』与『口径』；当 burstExcluded>0 时显示中必须可辨别
 *    （tooltip / 前缀 / 子行任一）。无值仍不显示占位。」
 *
 * 判据口径（**只断言语义，不锁字段名**，因为新增字段名在契约冻结时尚未定稿）：
 *   1. 口径可读出：`speedBasis` 必须随真值变化（`stream_decode` / `end_to_end` 两向都验，
 *      防「写死 stream_decode」蒙混）；速度本身仍与 `turnSpeedTokensPerSecond` 同源。
 *   2. 样本量可读出：展示结构里必须能读到 total=3 / eligible=1 这两个数字
 *      （独立字段、嵌套字段、或 tooltip 文本中的 `1/3` 形态，任一均可）。
 *   3. burst 剔除可辨别：burstExcluded=1 时展示结构里必须出现「剔除/突发」相关标记且数值=1；
 *      反向钉子：burstExcluded=0 时**不得**声称剔除了任何帧。
 *   4. 无值不占位：speed 缺席 ⇒ `speed` / `speedBasis` 均为 null，展示里不得出现 NaN/Infinity。
 *
 * 目录/后缀说明：本文件落在 `src/lib/runtime/__tests__/`（本任务的专属写域目录），
 * 后缀取 `.spec.ts` 以匹配仓库 `vitest.config.ts` 的 `src/**\/__tests__/**\/*.spec.ts` 收集规则
 * （`.test.ts` 在 `src/` 下**不会**被收集，会变成静默不跑的假测试）。
 *
 * 纪律：固定夹具、无随机、无 sleep、无 skip；每条用例均有非空断言。
 * 红相锚定：当前基线 `resolveMetricsSnapshotDisplay` 只返回 8 个字段
 * （contextTokens/contextWindow/speed/speedBasis/ttftMs/cacheRatioPercent/contextPercent/origin），
 * **没有**样本量与 burst 剔除的可读载体 ⇒ 断言 2/3 必失败。
 */

import { describe, expect, it } from "vitest";
import * as tokenMetricsModule from "@/lib/runtime/token-metrics";
import type { TurnMetricsSnapshot } from "@/types/runtime";

// ── 契约桥接（命名空间导入 + 「API 必须存在」守卫）───────────────────────────

type MetricsSnapshotDisplay = Record<string, unknown> & {
  speed?: number | null;
  speedBasis?: string | null;
};

type ResolveMetricsDisplay = (snapshot: TurnMetricsSnapshot) => MetricsSnapshotDisplay;

function resolveMetricsSnapshotDisplay(snapshot: TurnMetricsSnapshot): MetricsSnapshotDisplay {
  const candidate = (tokenMetricsModule as unknown as Record<string, unknown>)
    .resolveMetricsSnapshotDisplay;
  if (typeof candidate !== "function") {
    throw new Error(
      "契约未落地：src/lib/runtime/token-metrics.ts 必须导出 resolveMetricsSnapshotDisplay(snapshot)"
    );
  }
  return (candidate as ResolveMetricsDisplay)(snapshot);
}

// ── 夹具 ────────────────────────────────────────────────────────────────────
//
// 故意选用 **不含 1 与 3 的底数字**（contextPercent=25 / ttftMs=800 / speed=123.75 /
// cacheRatioPercent="60%"），这样「展示结构里能读到 3 与 1」只能是实现主动暴露的样本量，
// 不会因既有字段的巧合数值而误绿。

/** 快照底线：可解码口径（stream_decode）+ 样本量 total=3 / eligible=1 / burst=1。 */
function baseSnapshot(overrides: Record<string, unknown> = {}): TurnMetricsSnapshot {
  const snapshot: Record<string, unknown> = {
    step: 2,
    contextInputTokens: 25_000,
    inputTokens: 25_000,
    cacheHitInputTokens: 15_000,
    cacheMissInputTokens: 10_000,
    outputTokens: 190,
    reasoningTokens: null,
    totalTokens: 25_190,
    firstTokenLatencyMs: 800,
    durationMs: 1_900,
    callSpeedTokensPerSecond: 118.5,
    callSpeedBasis: "stream_decode",
    turnSpeedTokensPerSecond: 123.75,
    turnSpeedBasis: "stream_decode",
    turnOutputTokens: 190,
    turnGenerationMs: 800,
    turnCalls: 1,
    turnContextInputTokens: 25_000,
    turnCacheHitInputTokens: 15_000,
    turnCallsWithCache: 1,
    cacheHitRatio: 0.6,
    contextWindowTokens: 100_000,
    contextRatio: 0.25,
    origin: "provider",
    provider: "f1-fixture-provider",
    model: "f1-fixture-model",
    costUsd: null,
    // F1 新增的样本量/剔除三键（wire 侧名字由 task-4 定稿；此处按契约语义名注入，
    // 实现必须把这些语义透出到展示结构——名字可自选，语义不可省）。
    turnCallsTotal: 3,
    turnCallsEligible: 1,
    turnBurstExcluded: 1,
    ...overrides,
  };
  // 强制转换：不依赖 TS 接口是否已追加新字段（测试只锁语义，不锁类型面）。
  return snapshot as unknown as TurnMetricsSnapshot;
}

// ── 语义抽取工具（不锁字段名，只锁「可读出」）───────────────────────────────

interface SemanticsScan {
  /** 从展示结构里能读到的全部数字（数值字段 + 文本字段里解析出的整数/小数）。 */
  numbers: number[];
  /** 「剔除 / 突发」相关证据：键名或文本命中，且携带数字。 */
  burstEvidence: Array<{ label: string; numbers: number[] }>;
  /** 展示里出现的 NaN / Infinity（任何一处即为不诚实）。 */
  nonFinite: string[];
}

function numbersInText(text: string): number[] {
  const found: number[] = [];
  for (const match of text.matchAll(/-?\d+(?:\.\d+)?/g)) {
    const parsed = Number(match[0]);
    if (Number.isFinite(parsed)) {
      found.push(parsed);
    }
  }
  return found;
}

const BURST_KEY = /burst|exclud|drop|discard|突发|剔除|排除|滤除/i;

function scanSemantics(display: unknown): SemanticsScan {
  const numbers: number[] = [];
  const burstEvidence: Array<{ label: string; numbers: number[] }> = [];
  const nonFinite: string[] = [];

  const walk = (value: unknown, path: string): void => {
    if (typeof value === "number") {
      if (Number.isFinite(value)) {
        numbers.push(value);
      } else {
        nonFinite.push(`${path}=${String(value)}`);
      }
      return;
    }
    if (typeof value === "string") {
      const parsed = numbersInText(value);
      numbers.push(...parsed);
      if (BURST_KEY.test(value) || BURST_KEY.test(path)) {
        burstEvidence.push({ label: path, numbers: parsed });
      }
      return;
    }
    if (Array.isArray(value)) {
      value.forEach((item, index) => walk(item, `${path}[${index}]`));
      return;
    }
    if (value && typeof value === "object") {
      for (const [key, child] of Object.entries(value as Record<string, unknown>)) {
        const childPath = path ? `${path}.${key}` : key;
        if (BURST_KEY.test(key) && typeof child === "number") {
          burstEvidence.push({ label: childPath, numbers: [child] });
        }
        walk(child, childPath);
      }
    }
  };

  walk(display, "");
  return { numbers, burstEvidence, nonFinite };
}

function hasNumber(scan: SemanticsScan, expected: number): boolean {
  return scan.numbers.some((value) => Math.abs(value - expected) < 1e-9);
}

// ── 判据 1：口径可读出（两向验证，防写死）────────────────────────────────────

describe("F1 显示端：口径（basis）可读出", () => {
  it("decode 口径：speedBasis 必须是 stream_decode，速度仍取快照真值", () => {
    const display = resolveMetricsSnapshotDisplay(baseSnapshot());
    expect(display.speedBasis).toBe("stream_decode");
    expect(display.speed).toBeCloseTo(123.75, 6);
  });

  it("端到端口径：speedBasis 必须能表达 end_to_end（不得一律写死 decode）", () => {
    const display = resolveMetricsSnapshotDisplay(
      baseSnapshot({
        turnSpeedBasis: "end_to_end",
        callSpeedBasis: "end_to_end",
        turnSpeedTokensPerSecond: 41.5,
        turnCallsEligible: 0,
      })
    );
    expect(display.speedBasis).toBe("end_to_end");
    expect(display.speed).toBeCloseTo(41.5, 6);
  });
});

// ── 判据 2：样本量可读出（total=3 / eligible=1）──────────────────────────────

describe("F1 显示端：样本量可读出", () => {
  it("必须能从展示结构读出 total=3 与 eligible=1（字段 / 嵌套 / tooltip 文本任一路径）", () => {
    const display = resolveMetricsSnapshotDisplay(baseSnapshot());
    const scan = scanSemantics(display);

    expect(
      hasNumber(scan, 3),
      `契约 I4：展示结构必须能读出「样本量 total=3」（新字段或 tooltip 文本），实际可读数字 = ${JSON.stringify(scan.numbers)}，展示键 = ${JSON.stringify(Object.keys(display))}`
    ).toBe(true);
    expect(
      hasNumber(scan, 1),
      `契约 I4：展示结构必须能读出「参与计算的 eligible 样本数 =1」，实际可读数字 = ${JSON.stringify(scan.numbers)}，展示键 = ${JSON.stringify(Object.keys(display))}`
    ).toBe(true);
  });
});

// ── 判据 3：burstExcluded 可辨别 + 反向钉子 ─────────────────────────────────

describe("F1 显示端：burst 剔除可辨别", () => {
  it("burstExcluded=1 时展示必须能辨别（标记 + 数值 1）", () => {
    const display = resolveMetricsSnapshotDisplay(baseSnapshot({ turnBurstExcluded: 1 }));
    const scan = scanSemantics(display);

    const distinguishable = scan.burstEvidence.some((entry) =>
      entry.numbers.some((value) => Math.abs(value - 1) < 1e-9)
    );
    expect(
      distinguishable,
      `契约 I4：burstExcluded>0 时显示必须可辨别（tooltip/前缀/子行任一），实际剔除类证据 = ${JSON.stringify(scan.burstEvidence)}，展示键 = ${JSON.stringify(Object.keys(display))}`
    ).toBe(true);
  });

  it("反向钉子：burstExcluded=0 时不得声称剔除了任何突发帧", () => {
    const display = resolveMetricsSnapshotDisplay(
      baseSnapshot({ turnBurstExcluded: 0, turnCallsTotal: 1, turnCallsEligible: 1 })
    );
    const scan = scanSemantics(display);

    const falseClaim = scan.burstEvidence.some((entry) =>
      entry.numbers.some((value) => value > 0)
    );
    expect(
      falseClaim,
      `契约 I4：burstExcluded=0 时不得出现「剔除 N（N>0）」的假声明，实际剔除类证据 = ${JSON.stringify(scan.burstEvidence)}`
    ).toBe(false);
  });
});

// ── 判据 4：无值不占位 ──────────────────────────────────────────────────────

describe("F1 显示端：无值不占位", () => {
  it("speed 缺席 ⇒ speed/speedBasis 为 null，且展示中不得出现 NaN/Infinity", () => {
    const display = resolveMetricsSnapshotDisplay(
      baseSnapshot({
        turnSpeedTokensPerSecond: null,
        turnSpeedBasis: null,
        callSpeedTokensPerSecond: null,
        callSpeedBasis: null,
        turnCallsTotal: 0,
        turnCallsEligible: 0,
        turnBurstExcluded: 0,
      })
    );

    expect(display.speed).toBeNull();
    expect(display.speedBasis).toBeNull();

    const scan = scanSemantics(display);
    expect(
      scan.nonFinite,
      `契约 I4：无值场景不得把 NaN/Infinity 泄漏到展示结构，实际 ${JSON.stringify(scan.nonFinite)}`
    ).toEqual([]);
  });
});
