/**
 * Wave-54 Token 指标口径纯函数（C2 速度唯一口径 / C4 上下文归一）。
 *
 * 契约：`.dev-team/contract-matrix-wave54.md`
 * - §1 C2：`turn.speed = Σout_i / (Σgen_i / 1000)`，`gen_i = (ttft_i 存在且 dur_i > ttft_i)
 *   ? dur_i - ttft_i : dur_i`；`mode ∈ {turn_weighted, call_only, none}`，旧的
 *   `call_active_stream` / `call_fallback` / `turn_completed_aggregate` 三分叉已删除。
 * - §1 C4：「输入」位取值序 `contextInputTokens` → `resolveCurrentContextInputTokens()` → `—`。
 * - §4：`resolveTurnWeightedSpeedDetail` 为唯一速度解析器；旧导出仅为薄委托别名。
 * - §5 歧义裁决：末次调用取 `contextInputTokens`（缺失回退该记录 inputTokens）；
 *   混合 origin 降级为 `estimated_chars`；只累计有效调用（dur>0 且 out>0）；
 *   `ttft>dur` 退化 `gen=dur`。
 *
 * 兼容：`tests/acceptance/wave15/*` 与既有回归测试仍消费 `countTurnProviderCalls` /
 * `resolveCurrentContextInputTokens` / `resolveCallGenerationSpeed`，语义保持不变
 * （`resolveCallGenerationSpeed` 即 C2 的 per-call 分量公式 out_i/gen_i）。
 */
import { canonicalTraceTimelineKind } from "@/lib/runtime/trace-projection";
import type { ProviderCallCacheRecord, TraceTimelineEntry, TurnTraceRecord } from "@/types/runtime";

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
 * 单次调用生成速度（C2 per-call 分量）：`out_i / (gen_i / 1000)`。
 *
 * 守卫：`duration <= 0` 或 `output == null` → null；返回前必须 `Number.isFinite`。
 * TTFT 门控（wave-15 §7.2）：仅 `latencyKind === "provider_stream"` 时参与抵扣；
 * `duration <= ttft` 时退化为不抵扣。
 */
export function resolveCallGenerationSpeed(
  record: ProviderCallCacheRecord | null | undefined
): number | null {
  if (!record) {
    return null;
  }

  const outputTokens = record.outputTokens;
  const durationMs = record.turnDurationMs;
  if (outputTokens == null || durationMs == null || !(durationMs > 0)) {
    return null;
  }

  let generationMs = durationMs;
  if (record.latencyKind === "provider_stream") {
    const firstTokenLatencyMs = record.firstTokenLatencyMs;
    if (firstTokenLatencyMs != null && durationMs - firstTokenLatencyMs > 0) {
      generationMs = durationMs - firstTokenLatencyMs;
    }
  }

  const speed = outputTokens / (generationMs / 1000);
  if (!Number.isFinite(speed) || speed < 0) {
    return null;
  }
  return speed;
}

/** C2 单口径计算模式（旧三分叉已删除）。 */
export type TokenSpeedCalculationMode = "turn_weighted" | "call_only" | "none";

/** C6 速度位 `data-speed-basis` 取值（与 mode 同域）。 */
export type TokenSpeedBasis = "turn_weighted" | "call_only" | "none";

/** §4 冻结 API 的返回结构。 */
export interface TurnWeightedSpeedDetail {
  speed: number | null;
  mode: TokenSpeedCalculationMode;
  basis: TokenSpeedBasis;
  /** 参与计算的已完成调用数。 */
  calls: number;
  /** Σout（有效调用）。 */
  outputTokens: number | null;
  /** Σgen（有效调用，毫秒）。 */
  genMs: number | null;
  /** Σdur（有效调用，毫秒）——C6 调试串 `dur` 位。 */
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

/** C2 `gen_i`：ttft 存在且 dur > ttft ⇒ dur - ttft；否则 dur。 */
function generationMsOf(component: CallComponent): number {
  if (component.firstTokenLatencyMs != null && component.durationMs > component.firstTokenLatencyMs) {
    return component.durationMs - component.firstTokenLatencyMs;
  }
  return component.durationMs;
}

/** 有效调用：dur > 0 且 out > 0（§5 裁决 3：只累计有效调用）。 */
function isCountableCall(component: CallComponent): boolean {
  return component.durationMs > 0 && component.outputTokens > 0;
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

function resolveComponentSpeed(component: CallComponent): number | null {
  if (!(component.durationMs > 0) || component.outputTokens <= 0) {
    return null;
  }
  const speed = component.outputTokens / (generationMsOf(component) / 1000);
  return Number.isFinite(speed) && speed > 0 ? speed : null;
}

/**
 * §4 冻结 API：turn 加权生成速度（C2 唯一口径）。
 *
 * - `turn_weighted`：`calls >= 1` 且 `Σgen > 0` 且 `Σout > 0` ⇒ `speed = Σout / (Σgen / 1000)`；
 * - `call_only`：`calls === 0` 且轮次活跃且存在可用的当前调用 per-call 值；
 * - `none`：其余（`speed = null`）。
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
  const countable = completedComponents.filter(isCountableCall);

  let sumOutput = 0;
  let sumGenerationMs = 0;
  let sumDurationMs = 0;
  for (const component of countable) {
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

  if (countable.length >= 1 && sumGenerationMs > 0 && sumOutput > 0) {
    const speed = sumOutput / (sumGenerationMs / 1000);
    if (Number.isFinite(speed) && speed > 0) {
      return {
        speed,
        mode: "turn_weighted",
        basis: "turn_weighted",
        calls: countable.length,
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

  // call_only（§4 / BLOCKING 7）：`calls === 0` 且存在「活跃调用」的 per-call 值
  // ⇒ speed = out/(dur−ttft|dur)，`calls` 仍为 0（该调用尚未完成，不被计为已完成分量）。
  if (countable.length === 0 && isActive && activeCallCandidate) {
    const componentSpeed = resolveComponentSpeed(activeCallCandidate);
    if (componentSpeed != null) {
      return {
        speed: componentSpeed,
        mode: "call_only",
        basis: "call_only",
        calls: 0,
        outputTokens: activeCallCandidate.outputTokens,
        genMs: generationMsOf(activeCallCandidate),
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
    if (isActiveCallComponent(component) && isCountableCall(component)) {
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
