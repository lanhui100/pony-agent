/**
 * [W2 / task-2] 前端验收：`TurnMetricsSnapshot` 显示解析 + runtime store 集成。
 *
 * 契约源（只读）：`.dev-team/contract-matrix-metrics-snapshot.md`
 *   §0「两端消费约束」（快照存在 ⇒ 以快照为唯一来源，禁止各自重算）
 *   I2/I3（比值恒 ∈[0,1]，缺失 ⇒ 无值）、I6（百分比 = round(ratio*100)）、
 *   I7（旧事件缺 metrics ⇒ 回退既有路径，不 panic/不报错）。
 *
 * 冻结 API 面（Lead 裁决，不得改名）：
 *   - `src/lib/runtime/token-metrics.ts` → `resolveMetricsSnapshotDisplay(snapshot)`
 *   - `src/stores/runtime.ts` → state `liveMetrics` + action `applyMetricsSnapshot(metrics?)`
 *   - `src/types/runtime.ts` → `TurnMetricsSnapshot` 接口
 *
 * 纪律：固定夹具、无随机、无 sleep、无 skip；每条用例均有具体数值断言。
 * 红相锚定：上述 API 尚未落地 ⇒ 本文件必须失败（导出/action 缺失）。
 * 实施者（task-4）落地后本文件应全绿，且**不得修改本文件**（验收测试冻结）。
 *
 * 本文件由 Test Agent（task-2）独占写域。
 */
import { beforeEach, describe, expect, it } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { useRuntimeStore } from "@/stores/runtime";
import * as tokenMetricsModule from "@/lib/runtime/token-metrics";
import type { TurnMetricsSnapshot } from "@/types/runtime";

// ── 契约桥接（命名空间导入 + 「API 必须存在」守卫）───────────────────────────
// 与 `tests/acceptance/wave54/adversarial.spec.ts` 同一纪律：缺 API 时给出独立的
// 契约错误信号，而不是让整个文件在设计之前就失去诊断力。

type MetricsSnapshotDisplay = {
  contextTokens: number | null;
  contextWindow: number | null;
  speed: number | null;
  ttftMs: number | null;
  cacheRatioPercent: string;
  contextPercent: number | null;
  origin: string | null;
};

type ResolveMetricsDisplay = (snapshot: TurnMetricsSnapshot) => MetricsSnapshotDisplay;

function resolveMetricsSnapshotDisplay(
  snapshot: TurnMetricsSnapshot
): MetricsSnapshotDisplay {
  const candidate = (tokenMetricsModule as unknown as Record<string, unknown>)
    .resolveMetricsSnapshotDisplay;
  if (typeof candidate !== "function") {
    throw new Error(
      "契约未落地：src/lib/runtime/token-metrics.ts 必须导出 " +
        "resolveMetricsSnapshotDisplay(snapshot)"
    );
  }
  return (candidate as ResolveMetricsDisplay)(snapshot);
}

type StoreWithLiveMetrics = {
  liveMetrics?: TurnMetricsSnapshot | null;
  applyMetricsSnapshot?: (metrics?: TurnMetricsSnapshot | null) => void;
};

function runtimeStoreWithLiveMetrics(): StoreWithLiveMetrics {
  return useRuntimeStore() as unknown as StoreWithLiveMetrics;
}

function applyMetricsSnapshot(
  store: StoreWithLiveMetrics,
  metrics?: TurnMetricsSnapshot | null
): void {
  if (typeof store.applyMetricsSnapshot !== "function") {
    throw new Error(
      "契约未落地：src/stores/runtime.ts 必须新增 action " +
        "applyMetricsSnapshot(metrics?: TurnMetricsSnapshot | null): void"
    );
  }
  store.applyMetricsSnapshot(metrics);
}

// ── 夹具 ────────────────────────────────────────────────────────────────────

/** 契约夹具：缓存比 60% / 上下文 25%（25k ÷ 100k）/ 速度 120 t/s / TTFT 800ms。 */
function snapshotFixture(
  overrides: Partial<TurnMetricsSnapshot> = {}
): TurnMetricsSnapshot {
  return {
    step: 0,
    contextInputTokens: 25000,
    inputTokens: 25000,
    cacheHitInputTokens: 15000,
    cacheMissInputTokens: 10000,
    outputTokens: 5000,
    reasoningTokens: null,
    totalTokens: 30000,
    firstTokenLatencyMs: 800,
    durationMs: 5000,
    callSpeedTokensPerSecond: 1190.47,
    turnSpeedTokensPerSecond: 120,
    turnOutputTokens: 5000,
    turnGenerationMs: 4200,
    turnCalls: 1,
    cacheHitRatio: 0.6,
    contextWindowTokens: 100000,
    contextRatio: 0.25,
    origin: "provider",
    provider: "fixture-provider",
    model: "fixture-model-unpriced",
    costUsd: null,
    ...overrides
  };
}

beforeEach(() => {
  setActivePinia(createPinia());
});

// ── D1：纯函数逐字段映射 ─────────────────────────────────────────────────────

describe("D1 resolveMetricsSnapshotDisplay 纯函数映射", () => {
  it("D1-1: 完整快照逐字段映射（缓存 60% / 上下文 25 / 速度 120 / TTFT 800）", () => {
    // Arrange
    const snapshot = snapshotFixture();

    // Act
    const display = resolveMetricsSnapshotDisplay(snapshot);

    // Assert
    expect(display).toEqual({
      contextTokens: 25000,
      contextWindow: 100000,
      speed: 120,
      ttftMs: 800,
      cacheRatioPercent: "60%",
      contextPercent: 25,
      origin: "provider"
    });
    expect(display.cacheRatioPercent).toBe("60%");
    expect(display.contextPercent).toBe(25);
    expect(display.cacheRatioPercent.includes("100")).toBe(false);
  });

  it("D1-2: 缺失值纪律（ratio 为 null ⇒ 空串 / null，不得伪造成 0）", () => {
    // Arrange：缓存比与上下文比均缺失
    const snapshot = snapshotFixture({
      cacheHitRatio: null,
      contextRatio: null
    });

    // Act
    const display = resolveMetricsSnapshotDisplay(snapshot);

    // Assert：I2/I3 —— 无值不得渲染成「0%」
    expect(display.cacheRatioPercent).toBe("");
    expect(display.contextPercent).toBeNull();
    // 其余字段仍按快照透传
    expect(display.contextTokens).toBe(25000);
    expect(display.speed).toBe(120);
    expect(display.ttftMs).toBe(800);
  });

  it("D1-3: 窗口缺失 ⇒ contextWindow 为 null（不得兜底常量）", () => {
    // Arrange
    const snapshot = snapshotFixture({
      contextWindowTokens: null,
      contextRatio: null
    });

    // Act
    const display = resolveMetricsSnapshotDisplay(snapshot);

    // Assert
    expect(display.contextWindow).toBeNull();
    expect(display.contextPercent).toBeNull();
  });

  it("D1-4: 缓存比必须四舍五入（Math.round）而非截断", () => {
    // Arrange：0.6667 * 100 = 66.67 ⇒ 67（截断会得 66）
    const snapshot = snapshotFixture({ cacheHitRatio: 0.6667 });

    // Act
    const display = resolveMetricsSnapshotDisplay(snapshot);

    // Assert
    expect(display.cacheRatioPercent).toBe("67%");
  });
});

// ── D3：runtime store 集成（快照为唯一来源 + null 不清空）─────────────────────

describe("D3 runtime store applyMetricsSnapshot 集成", () => {
  it("D3-1: 应用快照后 liveMetrics 深等于快照", () => {
    // Arrange
    const store = runtimeStoreWithLiveMetrics();
    const snapshot = snapshotFixture();

    // Act
    applyMetricsSnapshot(store, snapshot);

    // Assert
    expect(store.liveMetrics).toEqual(snapshot);
  });

  it("D3-2: 契约语义 —— applyMetricsSnapshot(null) 必须保持不变（不得清成 null）", () => {
    // Arrange
    const store = runtimeStoreWithLiveMetrics();
    const snapshot = snapshotFixture();
    applyMetricsSnapshot(store, snapshot);
    expect(store.liveMetrics).toEqual(snapshot);

    // Act
    applyMetricsSnapshot(store, null);

    // Assert：null 是「无事发生」的信号，不是「清空」指令
    expect(store.liveMetrics).toEqual(snapshot);
  });

  it("D3-3: 新快照覆盖旧快照（step 前进的同一轮/下一轮）", () => {
    // Arrange
    const store = runtimeStoreWithLiveMetrics();
    applyMetricsSnapshot(store, snapshotFixture({ step: 0, cacheHitRatio: 0.6 }));
    const next = snapshotFixture({ step: 1, cacheHitRatio: 0.9, contextRatio: 0.5 });

    // Act
    applyMetricsSnapshot(store, next);

    // Assert
    expect(store.liveMetrics).toEqual(next);
    expect(store.liveMetrics?.cacheHitRatio).toBe(0.9);
  });
});

// ── D4：I7 向后兼容 —— undefined 不抛错 ─────────────────────────────────────

describe("D4 I7 旧事件（无 metrics）兼容", () => {
  it("D4-1: applyMetricsSnapshot(undefined) 不抛错且不改动既有状态", () => {
    // Arrange
    const store = runtimeStoreWithLiveMetrics();

    // Act + Assert
    expect(() => applyMetricsSnapshot(store, undefined)).not.toThrow();
    expect(store.liveMetrics ?? null).toBeNull();
  });

  it("D4-2: 已有快照时 applyMetricsSnapshot(undefined) 同样保持不变", () => {
    // Arrange
    const store = runtimeStoreWithLiveMetrics();
    const snapshot = snapshotFixture();
    applyMetricsSnapshot(store, snapshot);

    // Act + Assert
    expect(() => applyMetricsSnapshot(store, undefined)).not.toThrow();
    expect(store.liveMetrics).toEqual(snapshot);
  });
});
