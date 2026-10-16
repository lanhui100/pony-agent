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
  return Number.isFinite(speed) ? speed : null;
}
