/**
 * [W2 / task-2] 对抗用例：畸形 / 旧版 `metrics` payload 穿透（确定性，无 sleep）。
 *
 * 契约源（只读）：`.dev-team/contract-matrix-metrics-snapshot.md`
 *   I7（wire 向后兼容：旧事件缺 metrics ⇒ 回退既有路径，不 panic/不报错）
 *   I2/I3（比值缺失 ⇒ 无值，不得伪造成 0）
 *
 * 目标：**消费端不得因为线缆畸形而抛错或产出假真值**。
 * 红相锚定：`resolveMetricsSnapshotDisplay` / `applyMetricsSnapshot` 尚未落地 ⇒ 本文件必须失败。
 *
 * 本文件由 Test Agent（task-2）独占写域。
 */
import { beforeEach, describe, expect, it } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { useRuntimeStore } from "@/stores/runtime";
import * as tokenMetricsModule from "@/lib/runtime/token-metrics";

type MetricsSnapshotDisplay = {
  contextTokens: number | null;
  contextWindow: number | null;
  speed: number | null;
  ttftMs: number | null;
  cacheRatioPercent: string;
  contextPercent: number | null;
  origin: string | null;
};

function resolveMetricsSnapshotDisplay(snapshot: unknown): MetricsSnapshotDisplay {
  const candidate = (tokenMetricsModule as unknown as Record<string, unknown>)
    .resolveMetricsSnapshotDisplay;
  if (typeof candidate !== "function") {
    throw new Error(
      "契约未落地：src/lib/runtime/token-metrics.ts 必须导出 " +
        "resolveMetricsSnapshotDisplay(snapshot)"
    );
  }
  return (candidate as (s: unknown) => MetricsSnapshotDisplay)(snapshot);
}

type StoreWithLiveMetrics = {
  liveMetrics?: unknown;
  applyMetricsSnapshot?: (metrics?: unknown) => void;
};

function runtimeStoreWithLiveMetrics(): StoreWithLiveMetrics {
  return useRuntimeStore() as unknown as StoreWithLiveMetrics;
}

function applyMetricsSnapshot(store: StoreWithLiveMetrics, metrics?: unknown): void {
  if (typeof store.applyMetricsSnapshot !== "function") {
    throw new Error(
      "契约未落地：src/stores/runtime.ts 必须新增 action applyMetricsSnapshot(metrics?)"
    );
  }
  store.applyMetricsSnapshot(metrics);
}

beforeEach(() => {
  setActivePinia(createPinia());
});

// ── 1. 旧版事件 JSON（缺 `metrics` 键）───────────────────────────────────────

describe("ADV-1 旧事件线缆（无 metrics 键）", () => {
  const legacyEventJson = JSON.stringify({
    turnId: "turn-legacy",
    kind: "output_end",
    inputTokens: 25000,
    outputTokens: 5000,
    cacheHitInputTokens: 20000,
    totalTokens: 30000,
    providerName: "fixture-provider",
    providerModel: "fixture-model-unpriced"
  });

  it("ADV-1-1: 反序列化后 metrics 键必须缺席（而不是 null 占位）", () => {
    // Arrange + Act
    const event = JSON.parse(legacyEventJson) as Record<string, unknown>;

    // Assert：I7 —— 旧线缆字节零变化，键缺席
    expect("metrics" in event).toBe(false);
    expect(event.metrics).toBeUndefined();
  });

  it("ADV-1-2: 消费端收到 undefined 不得抛错，且不产生假快照", () => {
    // Arrange
    const store = runtimeStoreWithLiveMetrics();
    const event = JSON.parse(legacyEventJson) as Record<string, unknown>;

    // Act + Assert
    expect(() => applyMetricsSnapshot(store, event.metrics)).not.toThrow();
    expect(store.liveMetrics ?? null).toBeNull();
  });
});

// ── 2. `metrics` 内含 null 字段的畸形 JSON ───────────────────────────────────

describe("ADV-2 metrics 字段为 null / 缺失", () => {
  it("ADV-2-1: 全 null 字段的 metrics 不得抛错，且无值字段保持无值", () => {
    // Arrange：模拟上游把「未知」错误地写成 null 的畸形帧
    const malformed = JSON.parse(
      '{"step":0,"cacheHitRatio":null,"contextRatio":null,"contextInputTokens":null,' +
        '"contextWindowTokens":null,"turnSpeedTokensPerSecond":null,"firstTokenLatencyMs":null,"origin":null}'
    ) as Record<string, unknown>;

    // Act
    let display: MetricsSnapshotDisplay | undefined;
    expect(() => {
      display = resolveMetricsSnapshotDisplay(malformed);
    }).not.toThrow();

    // Assert：I2/I3 —— 无值不得渲染成「0%」/0
    expect(display).toBeDefined();
    expect(display!.cacheRatioPercent).toBe("");
    expect(display!.contextPercent).toBeNull();
    expect(display!.contextTokens).toBeNull();
    expect(display!.contextWindow).toBeNull();
    expect(display!.speed).toBeNull();
    expect(display!.ttftMs).toBeNull();
    expect(display!.origin).toBeNull();
  });

  it("ADV-2-2: 仅含两个键的最小对象不得抛错（其余键按缺失处理）", () => {
    // Arrange：只有 ratio，没有 token/速度/TTFT
    const minimal = { cacheHitRatio: 0.6, contextRatio: 0.25 };

    // Act
    let display: MetricsSnapshotDisplay | undefined;
    expect(() => {
      display = resolveMetricsSnapshotDisplay(minimal);
    }).not.toThrow();

    // Assert
    expect(display).toBeDefined();
    expect(display!.cacheRatioPercent).toBe("60%");
    expect(display!.contextPercent).toBe(25);
    expect(display!.contextTokens).toBeNull();
    expect(display!.contextWindow).toBeNull();
    expect(display!.speed).toBeNull();
    expect(display!.ttftMs).toBeNull();
  });

  it("ADV-2-3: 畸形快照注入 store 不得抛错、不得清空既有快照", () => {
    // Arrange：先有合法快照，随后到达一条字段全 null 的畸形帧
    const store = runtimeStoreWithLiveMetrics();
    const good = {
      cacheHitRatio: 0.6,
      contextRatio: 0.25,
      contextInputTokens: 25000,
      contextWindowTokens: 100000,
      turnSpeedTokensPerSecond: 120,
      firstTokenLatencyMs: 800
    };
    applyMetricsSnapshot(store, good);
    expect(store.liveMetrics).toEqual(good);

    // Act
    const malformed = {
      cacheHitRatio: null,
      contextRatio: null,
      contextInputTokens: null,
      contextWindowTokens: null,
      turnSpeedTokensPerSecond: null,
      firstTokenLatencyMs: null
    };

    // Assert：畸形对象同样是「有值」的输入，必须被写入（消费侧解析不得抛错）
    expect(() => applyMetricsSnapshot(store, malformed)).not.toThrow();
    expect(store.liveMetrics).toEqual(malformed);
  });
});
