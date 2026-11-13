/**
 * Token 指标口径纯函数（速度口径 v2 唯一真值 / C4 上下文归一）。
 *
 * 契约：`.dev-team/contract-matrix-token-speed-v2.md`（v2 取代 wave54 §1 C2 的速度定义）
 * - §1 I1–I6：`valid` = `dur > 0 && out > 0`；`decode_eligible` = valid 且 ttft 存在且
 *   `dur > ttft` 且 `out >= 2` 且 `gen >= max(200ms, 5% × dur)`；
 *   轮级 `speed = Σ(out−1) / (Σgen / 1000)`（`basis = stream_decode`），
 *   无 eligible 时回退 `Σout / (Σdur / 1000)`（`basis = end_to_end`），两者皆无 ⇒ `null`（I6，禁止 0 占位）。
 *   `mode ∈ {turn_weighted, call_only, none}` 表示**聚合范围**；`basis` 表示**测的是什么**（不同域）。
 * - §1 C4：「输入」位取值序 `contextInputTokens` → `resolveCurrentContextInputTokens()` → `—`。
 * - §4：`resolveTurnWeightedSpeedDetail` 为唯一速度解析器；旧导出仅为薄委托别名。
 * - §5 歧义裁决：末次调用取 `contextInputTokens`（缺失回退该记录 inputTokens）；
 *   混合 origin 降级为 `estimated_chars`；只累计有效调用（dur>0 且 out>0）；
 *   `ttft>dur` 退化 `gen=dur`。
 *
 * 兼容：`tests/acceptance/wave15/*` 与既有回归测试仍消费 `countTurnProviderCalls` /
 * `resolveCurrentContextInputTokens`（语义不变）；`resolveCallGenerationSpeed` 只保留旧签名，
 * 数值已收敛到 I8（= `resolveCallSpeedDetail(record).speed`），不再有独立的 per-call 公式。
 */
import { canonicalTraceTimelineKind } from "@/lib/runtime/trace-projection";
import type {
  ProviderCallCacheRecord,
  TraceTimelineEntry,
  TurnMetricsSnapshot,
  TurnSpeedBasis,
  TurnTraceRecord
} from "@/types/runtime";

/** C4/§7.6 判定用：轮内 provider call 数 = max(providerCallRecords.length, canonical call_model 条目数)。 */
export function countTurnProviderCalls(
  turn: TurnTraceRecord | null | undefined
): number {
  if (!turn) {
    return 0;
  }

  let timelineCallCount = 0;
  for (const entry of turn.traceTimeline ?? []) {
    if (canonicalTraceTimelineKind(entry.kind) === "call_model") {
      timelineCallCount++;
    }
  }

  return Math.max(turn.providerCallRecords?.length ?? 0, timelineCallCount);
}

/**
 * S1 当前上下文输入（wave-15 §7.6 冻结解析序，**保留**作为 C4 第二步回退）：
 * 1) records 非空 且 (records.length > 1 || records.length === countTurnProviderCalls) → 末条 record.inputTokens
 * 2) 否则最后一条「带 inputTokens」的 call_model 条目；
 * 3) 否则 countTurnProviderCalls <= 1 时允许回退 turn.inputTokens；
 * 4) 否则 null（严禁放行聚合值）。
 */
export function resolveCurrentContextInputTokens(
  turn: TurnTraceRecord | null | undefined
): number | null {
  if (!turn) {
    return null;
  }

  const records = turn.providerCallRecords ?? [];
  const providerCallCount = countTurnProviderCalls(turn);
  if (records.length > 0 && (records.length > 1 || records.length === providerCallCount)) {
    const lastRecord = records[records.length - 1];
    if (lastRecord?.inputTokens != null) {
      return lastRecord.inputTokens;
    }
  }

  const entries = turn.traceTimeline ?? [];
  for (let i = entries.length - 1; i >= 0; i--) {
    const entry = entries[i]!;
    if (canonicalTraceTimelineKind(entry.kind) === "call_model" && entry.inputTokens != null) {
      return entry.inputTokens;
    }
  }

  if (providerCallCount <= 1 && turn.inputTokens != null) {
    return turn.inputTokens;
  }

  return null;
}

/**
 * 契约 C4 第一步：**末次调用**的上下文输入（协议归一口径：anthropic 系 = input + cache_read，
 * 其余 = input）。多调用轮**不得**求和。
 *
 * 逐条判定（禁止跨记录拼接）：从末条 record 向前扫描，命中**第一条**具备 per-call 输入的记录
 * 即停止；该记录内优先 `contextInputTokens`，缺失则用**同一条**的 `inputTokens`
 * ——不得回退到更早记录的 `contextInputTokens`。
 * 无 per-call 记录时回退轮级 `contextInputTokens`；仍无 → null（由调用方回退 S1 或显示 `—`）。
 */
export function resolveContextInputTokens(
  turn: TurnTraceRecord | null | undefined
): number | null {
  if (!turn) {
    return null;
  }

  const records = turn.providerCallRecords ?? [];
  const providerCallCount = countTurnProviderCalls(turn);
  // 采信前置条件（与 S1 步骤 1 同源）：单条记录仅在「轮内调用数也是 1」时才是真 per-call，
  // 否则属 RC3 伪单次聚合记录（其 inputTokens = 整轮累加，严禁作为上下文占用显示）。
  const recordsTrustworthy =
    records.length > 0 && (records.length > 1 || records.length === providerCallCount);

  if (recordsTrustworthy) {
    for (let i = records.length - 1; i >= 0; i--) {
      const record = records[i];
      if (record?.contextInputTokens != null) {
        return record.contextInputTokens;
      }
      if (record?.inputTokens != null) {
        return record.inputTokens;
      }
    }
  }

  // `TraceTimelineEntry` 无 `contextInputTokens` 字段（C4 只给 ProviderCallCacheRecord /
  // TurnTraceRecord 增字段），条目层回退由 S1（resolveCurrentContextInputTokens）承担。
  return turn.contextInputTokens ?? null;
}

/**
 * 契约 C4 + C6：最近一次已完成调用的缓存命中输入（与可见「输入」同源）。
 * 无 per-call 来源 → null（缓存比显示空）。
 */
export function resolveLastCallCacheInputTokens(
  turn: TurnTraceRecord | null | undefined
): number | null {
  if (!turn) {
    return null;
  }

  const records = turn.providerCallRecords ?? [];
  for (let i = records.length - 1; i >= 0; i--) {
    const value = records[i]?.cacheHitInputTokens;
    if (value != null) {
      return value;
    }
  }

  const entries = turn.traceTimeline ?? [];
  for (let i = entries.length - 1; i >= 0; i--) {
    const entry = entries[i]!;
    if (canonicalTraceTimelineKind(entry.kind) === "call_model" && entry.cacheHitInputTokens != null) {
      return entry.cacheHitInputTokens;
    }
  }

  return null;
}

/**
 * 单次调用速度（**数值兼容别名**，语义已随速度口径 v2 收敛到 I8）：
 * 等价于 `resolveCallSpeedDetail(record).speed` ——
 * decode_eligible ⇒ `(out − 1) / (gen / 1000)`；有效但不可解码 ⇒ `out / (dur / 1000)`；
 * 无效（`dur <= 0` 或 `out <= 0`）⇒ null。
 *
 * 旧实现（`out_i / gen_i`、无物理下限、仅 `provider_stream` 抵扣 TTFT）已删除：
 * 它会把突发包（`gen ≈ 1ms`）放大到百万级 t/s，是 I9 禁止的第二种 per-call 公式。
 *
 * @deprecated 速度口径 v2（ADR 0073）：新消费面请用 `resolveCallSpeedDetail`
 * 取 `{ speed, basis }`（basis 自描述口径）；本函数只保留「数值 + null」旧签名。
 */
export function resolveCallGenerationSpeed(
  record: ProviderCallCacheRecord | null | undefined
): number | null {
  if (!record) {
    return null;
  }
  return resolveCallSpeedDetail(componentFromRecord(record)).speed;
}

/** 聚合范围（`turn_weighted` = 轮级加权；`call_only` = 当前单调用；`none` = 无值）。 */
export type TokenSpeedCalculationMode = "turn_weighted" | "call_only" | "none";

/**
 * C6 速度位 `data-speed-basis` 取值（I10：**测的是什么**，与 `mode` 的聚合范围不同域）。
 * - `stream_decode` = 解码窗口口径 `Σ(out−1) / Σgen`（可比）；
 * - `end_to_end` = 端到端口径 `Σout / Σdur`（含排队与 prefill，不可与前直接比较）；
 * - `none` = 无值（禁止 0 占位）。
 */
export type TokenSpeedBasis = TurnSpeedBasis;

/** §4 冻结 API 的返回结构。 */
export interface TurnWeightedSpeedDetail {
  speed: number | null;
  mode: TokenSpeedCalculationMode;
  basis: TokenSpeedBasis;
  /** 参与计算集合的调用数（I7：`stream_decode` ⇒ |eligible|；`end_to_end` ⇒ |valid|）。 */
  calls: number;
  /** 参与计算集合的 Σout（I7）。 */
  outputTokens: number | null;
  /** 分母时长（毫秒，I7）：`stream_decode` ⇒ Σgen；`end_to_end` ⇒ Σdur。 */
  genMs: number | null;
  /** 参与计算集合的 Σdur（毫秒）——C6 调试串 `dur` 位。 */
  durationMs: number | null;
  /** 末次已完成调用的上下文输入（C4 归一值，缺失回退 per-call input）。 */
  inputTokens: number | null;
  /** 末次已完成调用的缓存命中输入（与 inputTokens 同源）。 */
  cacheInputTokens: number | null;
  /** 首次调用 TTFT（不跨 hop 采样）。 */
  ttftMs: number | null;
  /** `estimated_chars` | `provider` | null。 */
  origin: string | null;
}

export interface TokenSpeedOptions {
  isActiveTurn?: boolean;
  isSubmitting?: boolean;
}

interface CallComponent {
  outputTokens: number;
  durationMs: number;
  firstTokenLatencyMs: number | null;
  inputTokens: number | null;
  contextInputTokens: number | null;
  cacheInputTokens: number | null;
  origin: string | null;
  /** 条目态（record 来源恒 null）：`active`/`running` 表示调用尚未完成。 */
  state: string | null;
}

/**
 * 速度判据所需的最小结构（I1/I3）：`CallComponent` 天然满足；展示层 per-hop 样本可直接
 * 构造该结构复用判据与公式，禁止各自复写（I9）。
 */
export interface CallSpeedSample {
  outputTokens: number;
  durationMs: number;
  firstTokenLatencyMs: number | null;
}

function componentFromRecord(record: ProviderCallCacheRecord): CallComponent {
  return {
    outputTokens: record.outputTokens ?? 0,
    durationMs: record.turnDurationMs ?? 0,
    firstTokenLatencyMs: record.firstTokenLatencyMs ?? null,
    inputTokens: record.inputTokens ?? null,
    contextInputTokens: record.contextInputTokens ?? null,
    cacheInputTokens: record.cacheHitInputTokens ?? null,
    origin: record.usageOrigin ?? null,
    state: null,
  };
}

function componentFromEntry(entry: TraceTimelineEntry): CallComponent {
  return {
    outputTokens: entry.outputTokens ?? 0,
    durationMs: entry.turnDurationMs ?? 0,
    firstTokenLatencyMs: entry.firstTokenLatencyMs ?? null,
    inputTokens: entry.inputTokens ?? null,
    // 条目无 contextInputTokens（C4 字段只落在 record / turn 级）。
    contextInputTokens: null,
    cacheInputTokens: entry.cacheHitInputTokens ?? null,
    origin: null,
    state: entry.state ?? null,
  };
}

/** 已完成分量（§4）：`providerCallRecords` 是该轮已完成调用的 per-call 真值载体。 */
function recordCallComponents(turn: TurnTraceRecord): CallComponent[] {
  return (turn.providerCallRecords ?? []).map(componentFromRecord);
}

/** timeline 的 `call_model` 条目（活跃调用 / legacy 无记录时的 per-call 载体）。 */
function timelineCallComponents(turn: TurnTraceRecord): CallComponent[] {
  const components: CallComponent[] = [];
  for (const entry of turn.traceTimeline ?? []) {
    if (canonicalTraceTimelineKind(entry.kind) === "call_model") {
      components.push(componentFromEntry(entry));
    }
  }
  return components;
}

/** 活跃调用判据：条目态非 completed（`running`/`active`）。 */
function isActiveCallComponent(component: CallComponent): boolean {
  return component.state != null && component.state !== "completed";
}

/** I2 `gen(r)`：ttft 存在且 dur > ttft ⇒ dur − ttft；否则 dur（ttft > dur 畸形不出负值）。 */
function generationMsOf(component: CallSpeedSample): number {
  if (component.firstTokenLatencyMs != null && component.durationMs > component.firstTokenLatencyMs) {
    return component.durationMs - component.firstTokenLatencyMs;
  }
  return component.durationMs;
}

/** I3 解码下限：绝对下限（毫秒）。 */
export const DECODE_FLOOR_MIN_MS = 200;

/** I3 解码下限：相对下限（`5% × dur`）。 */
export const DECODE_FLOOR_RATIO = 0.05;

/**
 * I3 物理下限 `max(200ms, 5% × dur)`。
 *
 * 与 Rust `turn_flow::decode_floor_ms` 同式：Rust 侧为整数运算 `dur * 5 / 100`，
 * 此处用 `Math.floor` 对齐截断，避免浮点边界（如 `20000 * 0.05`）产生分歧。
 */
export function decodeFloorMs(durationMs: number): number {
  if (!Number.isFinite(durationMs) || durationMs <= 0) {
    return DECODE_FLOOR_MIN_MS;
  }
  return Math.max(DECODE_FLOOR_MIN_MS, Math.floor(durationMs * DECODE_FLOOR_RATIO));
}

/** I1 有效调用判据：`dur > 0 && out > 0`（无效记录不参与速度计算，也不计入 calls）。 */
function isValidCall(sample: CallSpeedSample): boolean {
  return sample.durationMs > 0 && sample.outputTokens > 0;
}

/**
 * I3 解码样本判据（**唯一实现**，展示层与聚合层禁止再写一份）：
 * `valid && ttft 存在 && dur > ttft && out >= 2 && gen >= max(200ms, 5% × dur)`。
 */
export function isDecodeEligible(component: CallSpeedSample): boolean {
  if (!isValidCall(component)) {
    return false;
  }
  const firstTokenLatencyMs = component.firstTokenLatencyMs;
  if (firstTokenLatencyMs == null || !(component.durationMs > firstTokenLatencyMs)) {
    return false;
  }
  if (component.outputTokens < 2) {
    return false;
  }
  return generationMsOf(component) >= decodeFloorMs(component.durationMs);
}

/** 单调用速度结果（I8）：`basis` 自描述口径。 */
export interface CallSpeedDetail {
  speed: number | null;
  basis: TokenSpeedBasis;
}

/**
 * I8 单调用速度（**唯一实现**）：
 * `decode_eligible ⇒ (out − 1) / (gen / 1000)`；有效但不可解码 ⇒ `out / (dur / 1000)`；
 * 其余 ⇒ `{ speed: null, basis: "none" }`。`speed` 必须有限且 > 0，否则降级。
 */
export function resolveCallSpeedDetail(sample: CallSpeedSample | null | undefined): CallSpeedDetail {
  if (!sample) {
    return { speed: null, basis: "none" };
  }

  if (isDecodeEligible(sample)) {
    const speed = (sample.outputTokens - 1) / (generationMsOf(sample) / 1000);
    if (Number.isFinite(speed) && speed > 0) {
      return { speed, basis: "stream_decode" };
    }
  }

  if (isValidCall(sample)) {
    const speed = sample.outputTokens / (sample.durationMs / 1000);
    if (Number.isFinite(speed) && speed > 0) {
      return { speed, basis: "end_to_end" };
    }
  }

  return { speed: null, basis: "none" };
}

function emptyDetail(): TurnWeightedSpeedDetail {
  return {
    speed: null,
    mode: "none",
    basis: "none",
    calls: 0,
    outputTokens: null,
    genMs: null,
    durationMs: null,
    inputTokens: null,
    cacheInputTokens: null,
    ttftMs: null,
    origin: null,
  };
}

/**
 * §5 裁决 2：任一分量为 `estimated_chars` ⇒ 整体降级为 `estimated_chars`；
 * 全部为 `provider` 才输出 `provider`；其余（未知/混合未知）⇒ null。
 */
function resolveAggregateOrigin(
  components: CallComponent[],
  turn: TurnTraceRecord
): string | null {
  const origins = components
    .map((component) => component.origin)
    .filter((value): value is string => value != null);
  if (origins.some((origin) => origin === "estimated_chars")) {
    return "estimated_chars";
  }
  if (
    components.length > 0 &&
    origins.length === components.length &&
    origins.every((origin) => origin === "provider")
  ) {
    return "provider";
  }
  if (origins.length === 0 && turn.usageOrigin != null) {
    return turn.usageOrigin;
  }
  return null;
}

/**
 * §4 冻结 API：轮级速度（速度口径 v2 唯一解析器，I4 → I5 → I6）。
 *
 * - `turn_weighted` + `basis = stream_decode`：存在 >=1 条 decode_eligible 记录 ⇒
 *   `speed = Σ_eligible(out − 1) / (Σ_eligible gen / 1000)`；`calls/outputTokens/genMs` 为 eligible 集合；
 * - `turn_weighted` + `basis = end_to_end`：无 eligible 但有 >=1 条 valid 记录 ⇒
 *   `speed = Σ_valid out / (Σ_valid dur / 1000)`；`calls/outputTokens/genMs` 为 valid 集合；
 * - `call_only`：无已完成有效调用且轮次活跃 ⇒ I8 单调用口径（eligible ⇒ 解码速度，否则端到端）；
 * - `none`：其余（`speed = null`，禁止 0 占位）。
 */
export function resolveTurnWeightedSpeedDetail(
  turn: TurnTraceRecord | null | undefined,
  options?: TokenSpeedOptions
): TurnWeightedSpeedDetail {
  if (!turn) {
    return emptyDetail();
  }

  const turnStatus = (turn as { status?: string }).status;
  const isActive =
    options?.isActiveTurn ??
    options?.isSubmitting ??
    (turnStatus === "running" || turn.phase === "calling_model" || turn.phase === "calling_tool");

  const recordComponents = recordCallComponents(turn);
  const hasCompletedRecords = recordComponents.length > 0;
  const timelineComponents = timelineCallComponents(turn);

  // 已完成分量（§4 / BLOCKING 7）：
  // - `records` 非空 ⇒ 只认 records（records 是已完成调用的 per-call 真值载体，无效记录不得换源）；
  // - `records` 为空且轮次非活跃 ⇒ 回退 `call_model` 条目（R2 后条目即 per-call 值；legacy/折叠产物）；
  // - `records` 为空且轮次活跃 ⇒ 无已完成分量（活跃调用走 `call_only`）。
  const completedComponents = hasCompletedRecords
    ? recordComponents
    : isActive
      ? []
      : timelineComponents;
  const validCalls = completedComponents.filter(isValidCall);
  const decodeEligible = validCalls.filter(isDecodeEligible);
  // I7：`calls / outputTokens / genMs / durationMs` 均为**参与计算集合**
  // （decode ⇒ eligible 集合；e2e ⇒ valid 集合）。禁止一次 Σ 混两种口径。
  const participants = decodeEligible.length > 0 ? decodeEligible : validCalls;

  let sumOutput = 0;
  let sumGenerationMs = 0;
  let sumDurationMs = 0;
  for (const component of participants) {
    sumOutput += component.outputTokens;
    sumGenerationMs += generationMsOf(component);
    sumDurationMs += component.durationMs;
  }

  const activeCallCandidate = pickActiveCallCandidate(timelineComponents, isActive);
  const detailSource = completedComponents.length > 0 ? completedComponents : timelineComponents;
  const lastComponent = detailSource.length > 0 ? detailSource[detailSource.length - 1] : undefined;
  const firstTtft =
    detailSource.find((component) => component.firstTokenLatencyMs != null)?.firstTokenLatencyMs ?? null;

  const inputTokens = lastComponent
    ? (lastComponent.contextInputTokens ?? lastComponent.inputTokens)
    : countTurnProviderCalls(turn) <= 1
      ? (turn.contextInputTokens ?? turn.inputTokens ?? null)
      : null;
  const cacheInputTokens = lastComponent
    ? lastComponent.cacheInputTokens
    : countTurnProviderCalls(turn) <= 1
      ? (turn.cacheHitInputTokens ?? null)
      : null;

  const origin = resolveAggregateOrigin(completedComponents, turn);

  // I4 解码口径：分子扣 1（首 token 已计入 TTFT），分母只含真实解码窗口。
  if (decodeEligible.length >= 1 && sumGenerationMs > 0 && sumOutput > 0) {
    const speed = (sumOutput - decodeEligible.length) / (sumGenerationMs / 1000);
    if (Number.isFinite(speed) && speed > 0) {
      return {
        speed,
        mode: "turn_weighted",
        basis: "stream_decode",
        calls: decodeEligible.length,
        outputTokens: sumOutput,
        genMs: sumGenerationMs,
        durationMs: sumDurationMs,
        inputTokens,
        cacheInputTokens,
        ttftMs: firstTtft,
        origin,
      };
    }
  }

  // I5 端到端回退：无 decode_eligible 但有 valid 记录 ⇒ 分母为 Σdur（I7：genMs = Σdur）。
  if (validCalls.length >= 1 && sumDurationMs > 0 && sumOutput > 0) {
    const speed = sumOutput / (sumDurationMs / 1000);
    if (Number.isFinite(speed) && speed > 0) {
      return {
        speed,
        mode: "turn_weighted",
        basis: "end_to_end",
        calls: validCalls.length,
        outputTokens: sumOutput,
        genMs: sumDurationMs,
        durationMs: sumDurationMs,
        inputTokens,
        cacheInputTokens,
        ttftMs: firstTtft,
        origin,
      };
    }
  }

  // call_only（§4 / BLOCKING 7）：`calls === 0` 且存在「活跃调用」的 per-call 值
  // ⇒ I8 单调用口径（eligible ⇒ 解码速度，否则端到端）；`calls` 仍为 0（该调用尚未完成，
  // 不被计为已完成分量），`genMs` 随 basis 取 gen / dur（与 I7 同一语义位）。
  if (validCalls.length === 0 && isActive && activeCallCandidate) {
    const callSpeed = resolveCallSpeedDetail(activeCallCandidate);
    if (callSpeed.speed != null) {
      return {
        speed: callSpeed.speed,
        mode: "call_only",
        basis: callSpeed.basis,
        calls: 0,
        outputTokens: activeCallCandidate.outputTokens,
        genMs:
          callSpeed.basis === "stream_decode"
            ? generationMsOf(activeCallCandidate)
            : activeCallCandidate.durationMs,
        durationMs: activeCallCandidate.durationMs,
        inputTokens: activeCallCandidate.contextInputTokens ?? activeCallCandidate.inputTokens ?? inputTokens,
        cacheInputTokens: activeCallCandidate.cacheInputTokens ?? cacheInputTokens,
        ttftMs: activeCallCandidate.firstTokenLatencyMs,
        origin,
      };
    }
  }

  return {
    ...emptyDetail(),
    inputTokens,
    cacheInputTokens,
    ttftMs: firstTtft,
    origin,
  };
}

/**
 * 活跃调用候选：优先「未完成态」的 `call_model` 条目（`running`/`active`），
 * 其次最后一条带有效 per-call 分量的条目。仅在轮次活跃时返回。
 */
function pickActiveCallCandidate(
  timelineComponents: CallComponent[],
  isActive: boolean
): CallComponent | null {
  if (!isActive) {
    return null;
  }

  for (let i = timelineComponents.length - 1; i >= 0; i--) {
    const component = timelineComponents[i]!;
    if (isActiveCallComponent(component) && isValidCall(component)) {
      return component;
    }
  }

  return null;
}

// ── 旧导出（薄委托别名，禁止再有独立分支，§4）─────────────────────────────────

/** @deprecated wave-54 §4：请用 `resolveTurnWeightedSpeedDetail`。 */
export function resolveTurnGenerationSpeed(
  turn: TurnTraceRecord | null | undefined,
  options?: TokenSpeedOptions
): number | null {
  return resolveTurnWeightedSpeedDetail(turn, options).speed;
}

/** @deprecated wave-54 §4：请用 `resolveTurnWeightedSpeedDetail`。 */
export const resolveDualModeTokenSpeed = resolveTurnGenerationSpeed;

/** 旧详情结构（字段名保持兼容；`mode` 取值域已随 C2 收敛）。 */
export interface TokenSpeedDetail {
  speed: number | null;
  mode: TokenSpeedCalculationMode;
  inputTokens?: number | null;
  outputTokens?: number | null;
  durationMs?: number | null;
  genMs?: number | null;
  ttftMs?: number | null;
  calls?: number;
  basis?: TokenSpeedBasis;
  cacheInputTokens?: number | null;
  origin?: string | null;
}

/** @deprecated wave-54 §4：薄委托，语义与 `resolveTurnWeightedSpeedDetail` 完全一致。 */
export function resolveDualModeTokenSpeedDetail(
  turn: TurnTraceRecord | null | undefined,
  options?: TokenSpeedOptions
): TokenSpeedDetail {
  const detail = resolveTurnWeightedSpeedDetail(turn, options);
  return {
    speed: detail.speed,
    mode: detail.mode,
    inputTokens: detail.inputTokens,
    outputTokens: detail.outputTokens,
    durationMs: detail.durationMs,
    genMs: detail.genMs,
    ttftMs: detail.ttftMs,
    calls: detail.calls,
    basis: detail.basis,
    cacheInputTokens: detail.cacheInputTokens,
    origin: detail.origin,
  };
}

// ── 上下文占用（C4 取值序）────────────────────────────────────────────────────

export interface ContextUsageDebugInfo {
  inputTokens: number | null;
  contextWindow: number;
  calculationMode: TokenSpeedCalculationMode;
  generationSpeed: number | null;
}

export interface ContextUsageMetrics {
  contextUsagePercentage: number;
  contextUsageDebugInfo: ContextUsageDebugInfo;
}

export function resolveContextUsageMetrics(
  turn: TurnTraceRecord | null | undefined,
  contextWindow: number,
  options?: TokenSpeedOptions
): ContextUsageMetrics {
  const contextInputTokens = resolveContextInputTokens(turn);
  const inputTokens = contextInputTokens ?? resolveCurrentContextInputTokens(turn);
  const safeWindow = Number.isFinite(contextWindow) && contextWindow > 0 ? contextWindow : 0;
  let percentage = 0;

  if (safeWindow > 0 && inputTokens != null && inputTokens > 0) {
    const ratio = (inputTokens / safeWindow) * 100;
    percentage = Math.min(100, Math.max(0, ratio < 1 && ratio > 0 ? 1 : Math.round(ratio)));
  }

  const detail = resolveTurnWeightedSpeedDetail(turn, options);

  const debugInfo: ContextUsageDebugInfo = {
    inputTokens: inputTokens ?? (turn?.inputTokens ?? null),
    contextWindow: safeWindow,
    calculationMode: detail.mode,
    generationSpeed: detail.speed,
  };

  return {
    contextUsagePercentage: percentage,
    contextUsageDebugInfo: debugInfo,
  };
}

export function resolveContextUsageDiagnostics(
  debugInfo: ContextUsageDebugInfo
): string {
  const parts = [
    `模式: ${debugInfo.calculationMode}`,
    `窗口: ${debugInfo.contextWindow}`,
    `输入: ${debugInfo.inputTokens ?? 0}`,
    `速度: ${formatTokenSpeed(debugInfo.generationSpeed)}`,
  ];
  return parts.join(" · ");
}

export function formatTokenSpeed(value?: number | null): string {
  if (value == null || !Number.isFinite(value)) {
    return "—";
  }
  return `${Math.round(value)} t/s`;
}

/**
 * F1 I4：速度**样本量**文本（`N/M 次调用`）。
 *
 * 口径（与 Rust `turn_flow::turn_call_census` 同源，本函数只做展示拼装，**不算样本**）：
 * - `calls` = 参与计算集合的调用数（`stream_decode` ⇒ |eligible|；`end_to_end` ⇒ |valid|）；
 * - `callsTotal` = 该轮 provider 调用总数；旧 payload 缺该键（`null`）⇒ 只显示参与数
 *   `N 次调用`（**不得**把缺失当 0，也**不得**把总数当参与数）；
 * - 两者缺失 ⇒ `""`（无值不占位，I1/I6 同一条诚实纪律）。
 */
export function formatSpeedSampleLabel(
  calls?: number | null,
  callsTotal?: number | null
): string {
  const participants =
    typeof calls === "number" && Number.isFinite(calls) && calls > 0 ? Math.trunc(calls) : null;
  const total =
    typeof callsTotal === "number" && Number.isFinite(callsTotal) && callsTotal > 0
      ? Math.trunc(callsTotal)
      : null;
  if (total != null) {
    return `${participants ?? 0}/${total} 次调用`;
  }
  return participants == null ? "" : `${participants} 次调用`;
}

/**
 * F1 I4：**突发剔除**提示文案（`burstExcluded > 0` 才出，`0`/缺失/非有限 ⇒ `""`）。
 *
 * `burstExcluded` = `valid − decode_eligible`（因突发整包/无 TTFT/单 token 被剔除、
 * 只能进 `end_to_end` 回退的调用数）：> 0 时这一轮的 t/s **不是**纯解码速度，
 * 显示端必须可辨别。缺失 ≠ 0（旧 payload 无该键）⇒ 不出提示，也不伪造「无突发」。
 */
export function formatBurstExcludedSuffix(burstExcluded?: number | null): string {
  if (typeof burstExcluded !== "number" || !Number.isFinite(burstExcluded) || burstExcluded <= 0) {
    return "";
  }
  return `；已剔除 ${Math.trunc(burstExcluded)} 次非解码调用（突发包 / 无首token / 单token）`;
}

/**
 * 单位分数（`0..=1`）→ 诚实百分比字符串。
 *
 * 与 Rust `agent/metrics_snapshot.rs::format_unit_fraction_percent` 同构（单一真值语义，
 * ADR `.agents/notes/implemented/bug-fix/2026-11-12-metrics-percent-honesty.md` Decision 1/3）：
 * - 缺失 / 非有限 ⇒ `""`（无值不出 `0%`，不抛错，I2/I6）；
 * - `< 0` 或 `> 1` ⇒ `""`（越界 fail-closed，不 clamp 掩盖错误数据）；
 * - `=== 1` ⇒ `"100%"`（只有真全命中才允许出 100%）；
 * - 其余按整数四舍五入（保 D1-4：`0.6667` ⇒ `67%`）；
 * - 四舍五入会顶到 100 而 `ratio < 1` ⇒ 改用 2 位小数**向下取整**并去掉尾随 0
 *   （`0.996` ⇒ `99.6%`，`0.99999` ⇒ `99.99%`），保证**永不高报**。
 *
 * 返回值统一带 `%` 后缀（`MetricsSnapshotDisplay.cacheRatioPercent` 的既有消费契约）。
 */
export function formatUnitFractionPercent(ratio: number | null): string {
  if (ratio == null || !Number.isFinite(ratio)) {
    return "";
  }
  if (ratio < 0 || ratio > 1) {
    return "";
  }
  if (ratio === 1) {
    return "100%";
  }
  const rounded = Math.round(ratio * 100);
  if (rounded < 100) {
    return `${rounded}%`;
  }
  const hundredths = Math.floor(ratio * 10000) / 100;
  return `${hundredths.toFixed(2).replace(/\.?0+$/, "")}%`;
}

// ── 结算快照 → 展示映射（wave-metrics / task-4）─────────────────────────────────

/**
 * 快照展示字段（冻结 API 面，`tests/metrics-snapshot-live.spec.ts` D1 断言逐字段相等）。
 *
 * F1 I4 追加的样本量/剔除字段一律是 **`undefined` 可省**：旧 payload 缺键或「无可计数调用」
 * 时值为 `undefined`，既不入展示也不改变 D1 的逐字段相等断言（`toEqual` 忽略 `undefined` 属性）。
 */
export interface MetricsSnapshotDisplay {
  contextTokens: number | null;
  contextWindow: number | null;
  speed: number | null;
  /** 速度口径（I10）：直取 `turnSpeedBasis`，缺失 ⇒ null（禁止当 `stream_decode` 用）。 */
  speedBasis: TokenSpeedBasis | null;
  ttftMs: number | null;
  cacheRatioPercent: string;
  contextPercent: number | null;
  origin: string | null;
  /** F1 I4：参与计算集合的调用数（快照 `turnCalls`）；`0`/缺失 ⇒ `undefined`。 */
  sampleCalls?: number | undefined;
  /** F1 I4：该轮 provider 调用总数（快照 `turnCallsTotal`）；缺失/`0` ⇒ `undefined`。 */
  sampleCallsTotal?: number | undefined;
  /** F1 I4：`decode_eligible` 样本数（快照 `turnCallsEligible`）；缺失/`0` ⇒ `undefined`。 */
  sampleCallsEligible?: number | undefined;
  /** F1 I4：样本量文本（`eligible/total 次调用`）；两键皆缺 ⇒ `undefined`（不占位）。 */
  speedSampleLabel?: string | undefined;
  /** F1 I4：突发整包剔除数（快照 `turnBurstExcluded`）；`> 0` 才出，`0`/缺失 ⇒ `undefined`。 */
  burstExcluded?: number | undefined;
}

function finiteNumberOrNull(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

/** 「可展示的正计数」：非有限 / 非正 ⇒ `undefined`（`undefined` 属性不入展示、不进 `toEqual`）。 */
function positiveCountOrUndefined(value: unknown): number | undefined {
  const parsed = finiteNumberOrNull(value);
  return parsed != null && parsed > 0 ? Math.trunc(parsed) : undefined;
}

/**
 * `TurnMetricsSnapshot` → 展示值的**唯一**映射（契约 §0「快照存在 ⇒ 快照为唯一来源，禁止各自重算」）。
 *
 * 冻结语义：
 * - `contextTokens` ← `contextInputTokens`（协议归一值，I4）；
 * - `contextWindow` ← `contextWindowTokens`（缺失 ⇒ null，不得兜底常量，I3）；
 * - `speed` ← `turnSpeedTokensPerSecond`（与 Rust `weighted_turn_speed` 同源，I5）；
 * - `ttftMs` ← `firstTokenLatencyMs`；
 * - `cacheRatioPercent` ← `formatUnitFractionPercent(cacheHitRatio)`（`cacheHitRatio` 为 `0..=1`
 *   单位分数）：整数四舍五入，但四舍五入会顶到 100 而 `ratio < 1` 时降级为 1/2 位小数且**向下取整**
 *   （`0.996` ⇒ `99.6%`），只有 `=== 1` 才输出 `100%`——**部分命中不得谎报全命中**；
 *   缺失 / 非有限 ⇒ `""`（I2/I6：禁止伪造成 `0%`，也禁止 clamp 掩盖越界）；
 * - `contextPercent` ← `round(contextRatio * 100)`，缺失 ⇒ null（I3）；
 * - `origin` ← `origin`；
 * - F1 I4（样本量/口径诚实化）：`sampleCalls` ← `turnCalls`、`sampleCallsTotal` ← `turnCallsTotal`、
 *   `sampleCallsEligible` ← `turnCallsEligible`、`speedSampleLabel` = 二者合成的 `eligible/total 次调用`、
 *   `burstExcluded` ← `turnBurstExcluded`。**全部 `undefined` 可省**：旧 payload 缺键 / 无可计数调用 ⇒
 *   属性值 `undefined`（既不出占位、也不打破既有 D1 逐字段断言）。
 *
 * 防御：入参可为 `null` / `undefined` / 缺键的畸形线缆 —— 一律按「无值」处理，不抛错（I7）。
 */
export function resolveMetricsSnapshotDisplay(
  snapshot: TurnMetricsSnapshot | null | undefined
): MetricsSnapshotDisplay {
  const source = (snapshot ?? {}) as Partial<TurnMetricsSnapshot> | null;
  const cacheHitRatio = finiteNumberOrNull(source?.cacheHitRatio);
  const contextRatio = finiteNumberOrNull(source?.contextRatio);
  const origin = typeof source?.origin === "string" ? source.origin : null;
  // F1 I4：样本量三键与突发剔除数（负数/0/缺失一律 `undefined`，不伪造「0 次调用 / 无突发」）。
  //
  // 门控：F1 三键（`turnCallsTotal` / `turnCallsEligible` / `turnBurstExcluded`）**全部缺席**
  // （= 旧 payload）⇒ 本节所有字段一律 `undefined`：样本量无从谈起，且不改变既有展示面
  // （`tests/metrics-snapshot-live.spec.ts` D1 的逐字段相等断言依赖「旧快照无新增属性」）。
  const hasF1Sample =
    source?.turnCallsTotal != null ||
    source?.turnCallsEligible != null ||
    source?.turnBurstExcluded != null;
  const sampleCalls = hasF1Sample ? positiveCountOrUndefined(source?.turnCalls) : undefined;
  const sampleCallsTotal = hasF1Sample
    ? positiveCountOrUndefined(source?.turnCallsTotal)
    : undefined;
  const sampleCallsEligible = hasF1Sample
    ? positiveCountOrUndefined(source?.turnCallsEligible)
    : undefined;
  const burstExcluded = hasF1Sample
    ? positiveCountOrUndefined(source?.turnBurstExcluded)
    : undefined;
  const speedSampleLabel = hasF1Sample
    ? formatSpeedSampleLabel(sampleCallsEligible ?? sampleCalls, sampleCallsTotal)
    : "";

  return {
    contextTokens: finiteNumberOrNull(source?.contextInputTokens),
    contextWindow: finiteNumberOrNull(source?.contextWindowTokens),
    speed: finiteNumberOrNull(source?.turnSpeedTokensPerSecond),
    speedBasis: source?.turnSpeedBasis ?? null,
    ttftMs: finiteNumberOrNull(source?.firstTokenLatencyMs),
    cacheRatioPercent: formatUnitFractionPercent(cacheHitRatio),
    contextPercent: contextRatio == null ? null : Math.round(contextRatio * 100),
    origin,
    sampleCalls,
    sampleCallsTotal,
    sampleCallsEligible,
    speedSampleLabel: speedSampleLabel === "" ? undefined : speedSampleLabel,
    burstExcluded,
  };
}
