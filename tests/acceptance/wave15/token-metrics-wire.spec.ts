/**
 * Wave-15 wire-replay 跨语言验收（L2-T，Test Agent 唯一编写方）
 *
 * 证据来源：`.dev-team/evidence/wave15/wire-red.json`
 *   —— 由 Rust 侧 `crates/pony-agent-core/src/agent/runtime/tests.rs`
 *      `wave15_multihop_stream_exposes_per_call_metrics_in_hop_events_and_persisted_timeline`
 *      在真实 mock SSE 三跳回合中捕获的真实 `TurnStreamEvent` JSON 序列 + 持久化 timeline
 *      （同源冻结副本：`wire-red-frozen.json`，红相首次落盘，不被后续运行覆盖）。
 *
 * 协议事实：`turn:hop_complete` 只在「该次模型调用后仍需执行工具」时发出，末次调用走
 * `turn:output_end` / `turn:completed`；三跳回合 = 2 个 hop_complete + 1 个 completed。
 *
 * 运行顺序（硬要求）：必须先跑 Rust 捕获用例再跑本 spec（证据文件由该用例落盘）。
 *   cargo test --manifest-path src-tauri/Cargo.toml --target-dir target-test -p pony-agent-core \
 *     --lib wave15_multihop -- --nocapture
 *
 * 断言性质：跨语言一致性 —— Rust 产出的 hop per-call 记录与持久化 timeline，经前端 S1/S3
 * 解析后必须得到同一 per-call 真值；任何一处回落到整轮聚合值即为失败。
 */
import { existsSync, readFileSync } from "node:fs";
import path from "node:path";
import { beforeEach, describe, expect, it } from "vitest";
import { ref } from "vue";
import { createPinia, setActivePinia } from "pinia";
import { useRuntimeStore } from "@/stores/runtime";
import { useTraceProjection } from "@/lib/runtime/useTraceProjection";
import { countTurnProviderCalls, resolveCallGenerationSpeed, resolveCurrentContextInputTokens } from "@/lib/runtime/token-metrics";
import type { ProviderCallCacheRecord, TraceTimelineEntry, TurnTraceRecord } from "@/types/runtime";

const WIRE_PATH = path.resolve(process.cwd(), ".dev-team/evidence/wave15/wire-red.json");

type WireEvent = { name: string; payload: Record<string, unknown> };

type WireDocument = {
  wave: string;
  phase: string;
  turnId: string;
  sessionId: string;
  expectation: {
    hopInputs: number[];
    hopOutputs: number[];
    hopFirstTokenLatencyMs: number[];
    aggregateInputTokens: number;
    aggregateOutputTokens: number;
  };
  turnAggregate: { inputTokens: number | null; outputTokens: number | null; totalTokens: number | null };
  events: WireEvent[];
  persistedTimeline: TraceTimelineEntry[];
  persistedTurnTrace: TurnTraceRecord;
};

function readWire(): WireDocument {
  if (!existsSync(WIRE_PATH)) {
    throw new Error(
      `WIRE_EVIDENCE_MISSING: ${WIRE_PATH} 不存在；请先运行 Rust 捕获用例（见本文件头部运行顺序）`
    );
  }
  return JSON.parse(readFileSync(WIRE_PATH, "utf8")) as WireDocument;
}

const wire = readWire();
const hopCompleteEvents = wire.events.filter((event) => event.name === "turn:hop_complete");
const completedEvent = wire.events.find((event) => event.name === "turn:completed");
const aggregateInput = wire.expectation.aggregateInputTokens;
const aggregateOutput = wire.expectation.aggregateOutputTokens;
const lastHopInput = wire.expectation.hopInputs[wire.expectation.hopInputs.length - 1]!;

function hopRecords(index: number): ProviderCallCacheRecord[] {
  const payload = hopCompleteEvents[index]?.payload as { providerCallRecords?: ProviderCallCacheRecord[] | null } | undefined;
  return payload?.providerCallRecords ?? [];
}

function persistedRecords(): ProviderCallCacheRecord[] {
  return wire.persistedTurnTrace.providerCallRecords ?? [];
}

function lastPersistedRecord(): ProviderCallCacheRecord | null {
  const records = persistedRecords();
  return records.length ? records[records.length - 1]! : null;
}

function turnFrom(records: ProviderCallCacheRecord[]): TurnTraceRecord {
  return {
    ...wire.persistedTurnTrace,
    turnId: wire.turnId,
    inputTokens: aggregateInput,
    outputTokens: aggregateOutput,
    providerCallRecords: records,
    traceTimeline: wire.persistedTimeline
  };
}

/** S3 契约的期望值（与冻结解析序一致：TTFT 可信时抵扣，否则退化）。 */
function expectedSpeed(record: ProviderCallCacheRecord | null): number | null {
  const output = record?.outputTokens ?? null;
  const duration = record?.turnDurationMs ?? null;
  if (output == null || duration == null || duration <= 0) {
    return null;
  }
  const ttft = record?.latencyKind === "provider_stream" ? record?.firstTokenLatencyMs ?? null : null;
  const generative = ttft != null ? duration - ttft : 0;
  const denominator = generative > 0 ? generative : duration;
  return output / (denominator / 1000);
}

describe("Wave-15 wire-replay（Rust 真实捕获 → 前端 S1/S3 投影）", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it("证据结构自检：2 个 hop_complete + 3 条 call_model 条目 + 3 条 per-call 记录（基建门）", () => {
    expect(wire.wave).toBe("wave-15");
    expect(hopCompleteEvents.length).toBe(2);
    expect(wire.persistedTimeline.filter((entry) => entry.kind === "call_model").length).toBe(3);
    expect(persistedRecords().length).toBe(3);
    expect(wire.expectation.hopInputs).toEqual([350_000, 360_000, 370_000]);
    // 轮级累加口径（S2）在 wire 中可见且未被本波改动。
    expect(wire.turnAggregate.inputTokens).toBe(aggregateInput);
    expect(wire.turnAggregate.outputTokens).toBe(aggregateOutput);
  });

  it("R1：真实 hop_complete 必须携带该跳 per-call 记录（input/output/duration/TTFT 均为自身值）", () => {
    expect(hopCompleteEvents.length).toBe(2);
    for (let index = 0; index < hopCompleteEvents.length; index++) {
      const records = hopRecords(index);
      expect(records.length).toBeGreaterThan(0);
      const last = records[records.length - 1]!;
      expect(last.inputTokens).toBe(wire.expectation.hopInputs[index]);
      expect(last.outputTokens).toBe(wire.expectation.hopOutputs[index]);
      expect(last.turnDurationMs ?? 0).toBeGreaterThan(0);
      expect(last.firstTokenLatencyMs ?? 0).toBeGreaterThan(0);
      expect(last.latencyKind).toBe("provider_stream");
      // 严禁携带跨跳累加值（红相实况：hop0=710000 / hop1=1080000）。
      const redAccumulated = wire.expectation.hopInputs.slice(0, index + 1).reduce((sum, value) => sum + value, 0);
      expect(last.inputTokens).not.toBe(redAccumulated);
      expect(last.inputTokens).not.toBe(aggregateInput);
    }
  });

  it("R2/T6：真实持久化 timeline 每条 call_model 为自身 per-call 值，聚合只落 checkpoint_persist", () => {
    const modelEntries = wire.persistedTimeline.filter((entry) => entry.kind === "call_model");
    expect(modelEntries.map((entry) => entry.inputTokens)).toEqual(wire.expectation.hopInputs);
    expect(modelEntries.map((entry) => entry.outputTokens)).toEqual(wire.expectation.hopOutputs);
    for (const entry of modelEntries) {
      expect(entry.turnDurationMs ?? 0).toBeGreaterThan(0);
    }

    const checkpoint = wire.persistedTimeline.find((entry) => entry.kind === "checkpoint_persist");
    expect(checkpoint?.inputTokens).toBe(aggregateInput);
    expect(checkpoint?.outputTokens).toBe(aggregateOutput);
  });

  it("S1：wire 两类来源都不得把整轮累加值当作当前上下文输入", () => {
    // (a) 已持久化的 per-call 记录（R5 既有语义，3 条）→ 步骤 1 生效。
    const turnWithRecords = turnFrom(persistedRecords());
    expect(countTurnProviderCalls(turnWithRecords)).toBe(3);
    expect(resolveCurrentContextInputTokens(turnWithRecords)).toBe(lastHopInput);
    expect(resolveCurrentContextInputTokens(turnWithRecords)).not.toBe(aggregateInput);

    // (b) 仅 hop 事件来源（末次调用无 hop_complete）→ 不得回落到聚合值。
    //     R1 允许两种合规语义：hop 事件携带「本条调用」记录（末条=360000），
    //     或携带截至该跳的累计记录（末条=360000）；若实现改为单条伪记录，
    //     则由 A5 伪聚合守卫降级到 timeline 的最后一条 per-call 值（370000）。
    //     两种都必须是 per-call 真值，任何一处等于 1080000 即为违反 S4。
    const turnFromHops = turnFrom(hopRecords(hopCompleteEvents.length - 1));
    const fromHops = resolveCurrentContextInputTokens(turnFromHops);
    expect(fromHops).not.toBe(aggregateInput);
    expect([wire.expectation.hopInputs[1], lastHopInput]).toContain(fromHops);

    const store = useRuntimeStore();
    store.$patch({
      inputTokens: aggregateInput,
      turnTraceHistory: [turnWithRecords],
      traceTimeline: turnWithRecords.traceTimeline ?? [],
      activeTurnId: null,
      isSubmitting: false
    });
    const view = useTraceProjection({ liveTurnEnabled: ref(true) });
    expect(view.contextDisplayTokens.value).toBe(lastHopInput);
    expect(view.contextDisplayTokens.value).not.toBe(aggregateInput);
  });

  it("S3：速度由真实末跳 per-call 记录算出，投影值与前端口径一致（不得退化到整轮 elapsed）", () => {
    const record = lastPersistedRecord();
    expect(record).not.toBeNull();
    expect(record?.outputTokens).toBe(wire.expectation.hopOutputs[2]);
    expect(record?.latencyKind).toBe("provider_stream");

    const expected = expectedSpeed(record);
    expect(expected).not.toBeNull();
    expect(Number.isFinite(expected as number)).toBe(true);

    const speed = resolveCallGenerationSpeed(record);
    expect(speed).not.toBeNull();
    expect(Number.isFinite(speed as number)).toBe(true);
    expect(speed).toBeCloseTo(expected as number, 6);

    // 整轮累加口径（210 output / 181ms elapsed ≈ 1160 t/s）绝不可出现在 S3。
    const accumulatedSpeed = aggregateOutput / ((wire.persistedTurnTrace.turnDurationMs ?? 1) / 1000);
    expect(speed).not.toBeCloseTo(accumulatedSpeed, 6);

    const turn = turnFrom(persistedRecords());
    const store = useRuntimeStore();
    store.$patch({ turnTraceHistory: [turn], traceTimeline: turn.traceTimeline ?? [], activeTurnId: null, isSubmitting: false });
    const view = useTraceProjection({ liveTurnEnabled: ref(true) });
    expect(view.sessionTokenGenerationSpeed.value).toBeCloseTo(expected as number, 6);
  });

  it("S2 回归：wire 的整轮累加口径（turn:completed / 持久化轮级记录）保持不变", () => {
    const completedPayload = completedEvent?.payload as
      | { inputTokens?: number | null; outputTokens?: number | null }
      | undefined;
    expect(completedPayload?.inputTokens).toBe(aggregateInput);
    expect(completedPayload?.outputTokens).toBe(aggregateOutput);
    expect(wire.persistedTurnTrace.inputTokens).toBe(aggregateInput);
    expect(wire.persistedTurnTrace.outputTokens).toBe(aggregateOutput);
  });
});
