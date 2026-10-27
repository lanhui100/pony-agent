/**
 * Wave-15 wire-replay 跨语言验收（L2-T，Test Agent 唯一编写方）
 *
 * 证据来源：`.dev-team/evidence/wave15/wire-red.json`
 *   —— 由 Rust 侧 `crates/pony-agent-core/src/agent/runtime/tests.rs`
 *      `wave15_multihop_stream_exposes_per_call_metrics_in_hop_events_and_persisted_timeline`
 *      在真实 mock SSE 三跳回合中捕获的真实 `TurnStreamEvent` JSON 序列 + 持久化 timeline。
 *
 * 证据文件角色（三份，不得混用）：
 *   1. `wire-red-frozen.json` —— **红相冻结凭证**：第一次红相运行落盘后不再改写，
 *      其 `providerCallRecords` 为 `null`（R1 缺口实况），是"修正后对 pre-fix 代码仍为红"的对照物。
 *   2. `wire-red.json` —— **最新捕获快照**：Rust 捕获用例每次运行都会覆盖刷新，
 *      本 spec 读取的即是它。因此绿相门禁顺序硬性要求「先跑 cargo 捕获用例，再跑 vitest」
 *      （顺序颠倒会读到陈旧/缺失快照 → 结构性失败）。
 *      红相纯度复证可用环境变量改读冻结快照（只读，不覆盖任何文件）：
 *        WAVE15_WIRE_PATH=.dev-team/evidence/wave15/wire-red-frozen.json npx vitest run tests/acceptance/wave15/token-metrics-wire.spec.ts
 *      → 预期 R1/R2 红（冻结快照 per-call 记录为 null / timeline 为 [null,null,1080000]）。
 *   3. `wire-red-postrust.json` —— executor-rust 侧实施后的额外留档（由实施方落盘，本 spec 不消费）。
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

const WIRE_PATH = process.env.WAVE15_WIRE_PATH
  ? path.resolve(process.cwd(), process.env.WAVE15_WIRE_PATH)
  : path.resolve(process.cwd(), ".dev-team/evidence/wave15/wire-red.json");

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
const outputEndEvent = wire.events.find((event) => event.name === "turn:output_end");

function completedOutputEnd(): WireEvent | undefined {
  return outputEndEvent;
}

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

  it("R1（§7.10 I1/I2/I3）：hop_complete/output_end 的 records 是截至该事件的**全量**已完成调用", () => {
    expect(hopCompleteEvents.length).toBe(2);

    // 跨源真值：持久化 timeline 的 per-call 序列（R2）作为有序前缀基准。
    const persistedPairs = wire.persistedTimeline
      .filter((entry) => entry.kind === "call_model")
      .map((entry) => [entry.inputTokens, entry.outputTokens]);

    let previousRecordLen = 0;
    for (let index = 0; index < hopCompleteEvents.length; index++) {
      const payload = hopCompleteEvents[index]!.payload as { inputTokens?: number | null; outputTokens?: number | null };
      const records = hopRecords(index);

      // ---- I1 集合一致：Σ records[i].input_tokens == payload.inputTokens（output 同理） ----
      // 该断言先于 I2/I3，使滞后实现（records[..len-1]）的失败原因落在 I1。
      const sumInput = records.reduce((sum, record) => sum + (record.inputTokens ?? 0), 0);
      const sumOutput = records.reduce((sum, record) => sum + (record.outputTokens ?? 0), 0);
      expect(sumInput).toBe(payload.inputTokens);
      expect(sumOutput).toBe(payload.outputTokens);

      // ---- I3 有序前缀：长度随 hop 单调 +1（首 hop = 2：call#0 + call#1） ----
      const expectedRecordLen = index === 0 ? 2 : previousRecordLen + 1;
      expect(records.length).toBe(expectedRecordLen);
      previousRecordLen = records.length;
      expect(records.map((record) => [record.inputTokens, record.outputTokens])).toEqual(
        persistedPairs.slice(0, records.length)
      );
      expect(records[0]?.inputTokens).toBe(wire.expectation.hopInputs[0]);

      // ---- I2 末条即最新已完成调用：hop #k 的末条 = 第 k+1 次调用 ----
      const last = records[records.length - 1]!;
      expect(last.inputTokens).toBe(wire.expectation.hopInputs[index + 1]);
      expect(last.outputTokens).toBe(wire.expectation.hopOutputs[index + 1]);
      expect(last.turnDurationMs ?? 0).toBeGreaterThan(0);
      expect(last.firstTokenLatencyMs ?? 0).toBeGreaterThan(0);
      expect(last.latencyKind).toBe("provider_stream");

      // 恒等冗余守卫：末条既不是累加值也不是整轮聚合值。
      expect(last.inputTokens).not.toBe(payload.inputTokens);
      expect(last.inputTokens).not.toBe(aggregateInput);
    }

    // ---- §7.10：turn:output_end 的 records 长度 == 调用总数（3），且满足 I1/I2 ----
    const outputEndRecords =
      (completedOutputEnd()?.payload as { providerCallRecords?: ProviderCallCacheRecord[] | null } | undefined)
        ?.providerCallRecords ?? [];
    expect(outputEndRecords.length).toBe(3);
    const outputEndPayload = completedOutputEnd()?.payload as
      | { inputTokens?: number | null; outputTokens?: number | null }
      | undefined;
    expect(outputEndRecords.reduce((sum, record) => sum + (record.inputTokens ?? 0), 0)).toBe(
      outputEndPayload?.inputTokens
    );
    expect(outputEndRecords.reduce((sum, record) => sum + (record.outputTokens ?? 0), 0)).toBe(
      outputEndPayload?.outputTokens
    );
    expect(outputEndRecords[outputEndRecords.length - 1]?.inputTokens).toBe(wire.expectation.hopInputs[2]);
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

    // (b) 仅 hop 事件来源（末次调用无 hop_complete，由 output_end 承载全量）→ §7.10 要求
    //     末个 hop_complete 的 records 已是全量前缀，故 S1 必须取到末次调用 370000，
    //     严禁回落到累加值 1080000。
    const turnFromHops = turnFrom(hopRecords(hopCompleteEvents.length - 1));
    expect(turnFromHops.providerCallRecords?.length).toBe(3);
    const fromHops = resolveCurrentContextInputTokens(turnFromHops);
    expect(fromHops).toBe(lastHopInput);
    expect(fromHops).not.toBe(aggregateInput);

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

  it("S3：per-call 速度由末跳记录算出，投影速度取 wire 全量调用的 turn 加权口径（Σout/Σgen，不得退化到整轮 elapsed）", () => {
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
    // wave-54 契约演进（ADR 0053）口径来源：投影速度不再是末跳 per-call（100000 t/s），
    // 而是 wire 三条已完成调用的 turn 加权值：
    //   Σout = 50 + 60 + 100 = 210
    //   Σgen = 55 + (48−47) + (49−48) = 57ms  ← 第 1 条 ttft 55 === dur 55 ⇒ 不抵扣，gen = dur
    //   ⇒ speed = 210 / 0.057 = 3684.210526315789 t/s
    expect(view.sessionTokenGenerationSpeed.value).toBe(3684.210526315789);
    expect(view.tokenSpeedCalculationMode.value).toBe("turn_weighted");
    // 判别式：不得等于末跳 per-call 值（100000）或整轮 elapsed 口径（1257.49）
    expect(view.sessionTokenGenerationSpeed.value).not.toBeCloseTo(expected as number, 6);
    expect(view.sessionTokenGenerationSpeed.value).not.toBeCloseTo(accumulatedSpeed, 6);
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
