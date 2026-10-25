/**
 * Wave-15 Token 指标口径纯函数（S1 / S3 / S4）。
 *
 * 契约：.dev-team/contracts/wave-15-token-metrics.md（S1–S4 与 §7 冻结解析序 v2）。
 * - S1：`resolveCurrentContextInputTokens` —— 当前上下文占用 = 最近一次已完成模型调用的 per-call input。
 * - S3：`resolveCallGenerationSpeed` —— 单次调用生成速度（仅步骤 1 + 守卫；步骤 2–4 属调用方职责，§7.8）。
 * - S4：`countTurnProviderCalls` —— 轮内 provider call 数，用于判定累加值是否可作为回退。
 */
import { canonicalTraceTimelineKind } from "@/lib/runtime/trace-projection";
import type { ProviderCallCacheRecord, TurnTraceRecord } from "@/types/runtime";

/** S4 判定用：轮内 provider call 数 = max(providerCallRecords.length, canonical call_model 条目数)。 */
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
 * S1 当前上下文输入：最近一次已完成模型调用的 `inputTokens`（禁止跨跳累加）。
 *
 * 冻结解析序（§7.6）：
 * 1) records 非空 且 (records.length > 1 || records.length === countTurnProviderCalls) → 末条 record.inputTokens
 *    （前置条件拦截 RC3「伪单次聚合记录」，不满足时降级步骤 2）；
 * 2) 否则最后一条「带 inputTokens」的 call_model 条目；
 * 3) 否则 countTurnProviderCalls <= 1 时允许回退 turn.inputTokens（单调用与 per-call 等价）；
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
 * S3 单次调用生成速度（t/s）：`output_tokens / ((duration_ms − first_token_latency_ms) / 1000)`。
 *
 * 守卫（§7.6）：`duration <= 0` 或 `output == null` → null；返回前必须 `Number.isFinite`。
 * TTFT 可信门控（§7.2）：仅 `latencyKind === "provider_stream"` 时参与抵扣；
 * `duration <= ttft` 时退化为不抵扣（TTFT >= duration 视为不可信）。
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

export type TokenSpeedCalculationMode =
  | "turn_completed_aggregate"
  | "call_active_stream"
  | "call_fallback"
  | "none";

export interface TokenSpeedOptions {
  isActiveTurn?: boolean;
  isSubmitting?: boolean;
}

/**
 * 速度双模计算：
 * - 活跃调用中（isActiveTurn / isSubmitting 为 true）：优先计算最近活跃调用速度 (call_active_stream)。
 * - 终态（isSubmitting === false 或 turn.status === "completed"）：若包含整轮 outputTokens 与 durationMs，
 *   计算整轮综合速度 (turn_completed_aggregate)。无整轮数据时回退单次调用速度 (call_fallback)。
 */
export function resolveTurnGenerationSpeed(
  turn: TurnTraceRecord | null | undefined,
  options?: TokenSpeedOptions
): number | null {
  const result = resolveDualModeTokenSpeedDetail(turn, options);
  return result.speed;
}

export const resolveDualModeTokenSpeed = resolveTurnGenerationSpeed;

export interface TokenSpeedDetail {
  speed: number | null;
  mode: TokenSpeedCalculationMode;
  inputTokens?: number | null;
  outputTokens?: number | null;
  durationMs?: number | null;
  ttftMs?: number | null;
}

export function resolveDualModeTokenSpeedDetail(
  turn: TurnTraceRecord | null | undefined,
  options?: TokenSpeedOptions
): TokenSpeedDetail {
  if (!turn) {
    return { speed: null, mode: "none" };
  }

  const turnStatus = (turn as { status?: string }).status;
  const isActive =
    options?.isActiveTurn ??
    options?.isSubmitting ??
    (turnStatus === "running" || turn.phase === "calling_model" || turn.phase === "calling_tool");

  // 1. 活跃调用生成中：优先取最近一次 ProviderCall 的生成速度
  if (isActive) {
    const records = turn.providerCallRecords ?? [];
    for (let i = records.length - 1; i >= 0; i--) {
      const record = records[i];
      const callSpeed = resolveCallGenerationSpeed(record);
      if (callSpeed != null) {
        return {
          speed: callSpeed,
          mode: "call_active_stream",
          inputTokens: record?.inputTokens,
          outputTokens: record?.outputTokens,
          durationMs: record?.turnDurationMs,
          ttftMs: record?.firstTokenLatencyMs,
        };
      }
    }
  }

  // 2. 终态轮次（已完成或非活跃）：
  // 根据 Lead 裁决（records 判别 + timeline 次级判别）：
  // - records.length > 1: 多调用单轮终态回退末条有效 record 速度（TTFT 抵扣），mode = "call_fallback"（满足 S3 不得用整轮累加）
  // - records.length === 1: 真实单调用终态，若 turn 级 outputTokens 与 turnDurationMs 有效，取整轮综合速度 outputTokens/(turnDurationMs/1000)，mode = "turn_completed_aggregate"；否则该 record 速度，mode = "call_fallback"
  // - records.length === 0:
  //   - 若 traceTimeline 有 canonical call_model 条目且 (firstTokenLatencyMs != null || outputTokens != null):
  //     取该条目单次速度（TTFT 抵扣），mode = "call_fallback"（保住已冻结的 S3 契约 tests/context-display-tokens.spec.ts）
  //   - 否则若 turn 级数据有效: 取整轮综合速度，mode = "turn_completed_aggregate"
  //   - 否则: null, mode = "none"
  const records = turn.providerCallRecords ?? [];
  const turnDurationMs = turn.turnDurationMs;
  const outputTokens = turn.outputTokens;
  const inputTokens = turn.inputTokens;

  if (records.length > 1) {
    for (let i = records.length - 1; i >= 0; i--) {
      const record = records[i];
      const callSpeed = resolveCallGenerationSpeed(record);
      if (callSpeed != null) {
        return {
          speed: callSpeed,
          mode: "call_fallback",
          inputTokens: record?.inputTokens,
          outputTokens: record?.outputTokens,
          durationMs: record?.turnDurationMs,
          ttftMs: record?.firstTokenLatencyMs,
        };
      }
    }
  } else if (records.length === 1) {
    if (turnDurationMs != null && turnDurationMs > 0 && outputTokens != null) {
      const aggregateSpeed = outputTokens / (turnDurationMs / 1000);
      if (Number.isFinite(aggregateSpeed) && aggregateSpeed > 0) {
        return {
          speed: aggregateSpeed,
          mode: "turn_completed_aggregate",
          inputTokens: inputTokens ?? records[0]?.inputTokens,
          outputTokens,
          durationMs: turnDurationMs,
          ttftMs: undefined,
        };
      }
    }
    const callSpeed = resolveCallGenerationSpeed(records[0]);
    if (callSpeed != null) {
      return {
        speed: callSpeed,
        mode: "call_fallback",
        inputTokens: records[0]?.inputTokens,
        outputTokens: records[0]?.outputTokens,
        durationMs: records[0]?.turnDurationMs,
        ttftMs: records[0]?.firstTokenLatencyMs,
      };
    }
  } else {
    // records.length === 0
    const entries = turn.traceTimeline ?? [];
    const hasTimelineCallModel = entries.some(
      (e) => canonicalTraceTimelineKind(e.kind) === "call_model" && (e.firstTokenLatencyMs != null || e.outputTokens != null)
    );

    if (hasTimelineCallModel) {
      for (let i = entries.length - 1; i >= 0; i--) {
        const entry = entries[i]!;
        if (canonicalTraceTimelineKind(entry.kind) === "call_model") {
          const callSpeed = resolveCallGenerationSpeed({
            requestKind: "initial_request",
            latencyKind: "provider_stream",
            inputTokens: entry.inputTokens ?? undefined,
            outputTokens: entry.outputTokens ?? undefined,
            turnDurationMs: entry.turnDurationMs ?? undefined,
            firstTokenLatencyMs: entry.firstTokenLatencyMs ?? undefined,
          });
          if (callSpeed != null) {
            return {
              speed: callSpeed,
              mode: "call_fallback",
              inputTokens: entry.inputTokens,
              outputTokens: entry.outputTokens,
              durationMs: entry.turnDurationMs,
              ttftMs: entry.firstTokenLatencyMs,
            };
          }
        }
      }
    } else if (turnDurationMs != null && turnDurationMs > 0 && outputTokens != null) {
      const aggregateSpeed = outputTokens / (turnDurationMs / 1000);
      if (Number.isFinite(aggregateSpeed) && aggregateSpeed > 0) {
        return {
          speed: aggregateSpeed,
          mode: "turn_completed_aggregate",
          inputTokens,
          outputTokens,
          durationMs: turnDurationMs,
          ttftMs: undefined,
        };
      }
    }
  }

  return { speed: null, mode: "none" };
}

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
  const inputTokens = resolveCurrentContextInputTokens(turn);
  const safeWindow = Number.isFinite(contextWindow) && contextWindow > 0 ? contextWindow : 0;
  let percentage = 0;

  if (safeWindow > 0 && inputTokens != null && inputTokens > 0) {
    const ratio = (inputTokens / safeWindow) * 100;
    percentage = Math.min(100, Math.max(0, ratio < 1 && ratio > 0 ? 1 : Math.round(ratio)));
  }

  const { speed, mode } = resolveDualModeTokenSpeedDetail(turn, options);

  const debugInfo: ContextUsageDebugInfo = {
    inputTokens: inputTokens ?? (turn?.inputTokens ?? null),
    contextWindow: safeWindow,
    calculationMode: mode,
    generationSpeed: speed,
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
