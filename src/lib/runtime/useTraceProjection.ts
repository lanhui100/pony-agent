import { computed, ref, watch, type ComputedRef, type Ref } from "vue";
import { storeToRefs } from "pinia";
import type { TraceTimelineEntry, TurnTraceRecord } from "@/types/runtime";
import { useProviderStore } from "@/stores/providers";
import { useRuntimeStore } from "@/stores/runtime";
import {
  canonicalTraceTimelineKind,
  turnTimeline as projectedTurnTimeline
} from "@/lib/runtime/trace-projection";
import { cloneProviderCallRecords, resolveProviderReturnedCacheHitInputTokens } from "@/lib/runtime/trace";
import {
  countTurnProviderCalls,
  resolveCallGenerationSpeed,
  resolveCurrentContextInputTokens,
  resolveDualModeTokenSpeedDetail,
  type TokenSpeedCalculationMode,
  type TokenSpeedDetail,
} from "@/lib/runtime/token-metrics";
import { recordMetricsSnapshot, reportMetricsAnomaly } from "@/lib/runtime/metrics-diagnostics";

export { canonicalTraceTimelineKind, projectedTurnTimeline };

/**
 * PA-096：从 HomeSidebar 抽出的会话 trace 投影管线（原实现逐位搬迁）。
 *
 * - liveTurnEnabled 是 PA-086 冻结守卫的参数化：禁用时活跃 turn 不并入
 *   orderedTurnTraces（切断与 store 可变数组的引用共享、避免 memo 陈旧命中）。
 * - HomeSidebar 消费聚合喂状态面板（liveTurnEnabled 恒 false，与原"trace 面板
 *   折叠时聚合不含进行中 turn"行为一致）；TraceInspector 以 open 作为开关。
 */

// 与 trace.ts 的实现共用单源（PA-096 review B-P3-3：消除逐字节重复）。
export const providerReturnedCacheHitInputTokens = resolveProviderReturnedCacheHitInputTokens;

function turnTraceSortKey(turn: TurnTraceRecord) {
  return turn.updatedAt ?? turn.emittedAtMs ?? turn.sequence ?? 0;
}

function compareTurnTraceOrder(left: TurnTraceRecord, right: TurnTraceRecord) {
  const leftKey = turnTraceSortKey(left);
  const rightKey = turnTraceSortKey(right);
  if (leftKey !== rightKey) {
    return leftKey - rightKey;
  }

  return left.turnId.localeCompare(right.turnId);
}

/**
 * S3 步骤 2：该轮 `call_model` 条目的 per-call 生成速度。
 *
 * `TraceTimelineEntry` 无 `latencyKind` 字段（§7.3），条目 TTFT 沿用既有 per-call 语义
 * （R2 落地后条目携带自身 first_token_latency_ms），因此按 `provider_stream` 参与抵扣。
 */
function entryCallGenerationSpeed(entry: TraceTimelineEntry): number | null {
  return resolveCallGenerationSpeed({
    requestKind: "initial_request",
    outputTokens: entry.outputTokens ?? null,
    turnDurationMs: entry.turnDurationMs ?? null,
    firstTokenLatencyMs: entry.firstTokenLatencyMs ?? null,
    latencyKind: "provider_stream"
  });
}

/** 整轮整体速度兜底（turn 级 outputTokens / turnDurationMs）。 */
function turnTokenSpeed(turn: TurnTraceRecord): number | null {
  if (turn.outputTokens == null || turn.turnDurationMs == null || turn.turnDurationMs <= 0) {
    return null;
  }

  const speed = turn.outputTokens / (Math.max(turn.turnDurationMs, 1) / 1000);
  return Number.isFinite(speed) ? speed : null;
}

/** 剪贴板反馈：HomeSidebar（session-id 复制）与 TraceInspector（trace 复制）各自实例化。 */
export function useCopyFeedback() {
  const copiedKey = ref("");
  let copiedTimer: number | null = null;

  function copyText(key: string, text: string) {
    if (!text.trim()) {
      return;
    }

    void navigator.clipboard.writeText(text);
    copiedKey.value = key;

    if (copiedTimer != null) {
      window.clearTimeout(copiedTimer);
    }

    copiedTimer = window.setTimeout(() => {
      copiedKey.value = "";
      copiedTimer = null;
    }, 1400);
  }

  function resetCopy() {
    copiedKey.value = "";
  }

  function dispose() {
    if (copiedTimer != null) {
      window.clearTimeout(copiedTimer);
      copiedTimer = null;
    }
  }

  return { copiedKey, copyText, resetCopy, dispose };
}

export function useTraceProjection(options: {
  liveTurnEnabled: Ref<boolean> | ComputedRef<boolean>;
}) {
  const runtimeStore = useRuntimeStore();
  const providerStore = useProviderStore();

  const {
    activeTurnId: runtimeActiveTurnId,
    activeProviderCallRecords,
    error,
    fallbackReason,
    firstTokenLatencyMs,
    inputTokens,
    isSubmitting,
    messages,
    outputTokens,
    phase,
    providerMode,
    providerModel,
    providerName,
    providerProtocol,
    retrievedContext,
    toolActivities,
    totalTokens,
    traceSteps,
    traceTimeline,
    turnTraceHistory
  } = storeToRefs(runtimeStore);

  // 守卫关闭时：liveTraceTurn 返回 null，避免与 store 活跃 timeline 共享可变引用
  // （就地修改会污染"冻结"引用，且 memo 缓存会因引用不变而返回陈旧结果——PA-086 约束）。
  // 守卫开启时：正常构造活跃 turn（引用随 store 更新，memo 以 ref+updatedAt 自然失效）。
  const liveTraceTurn = computed<TurnTraceRecord | null>(() => {
    if (!options.liveTurnEnabled.value) {
      return null;
    }
    const turnId = runtimeActiveTurnId.value?.trim() || "";
    if (!isSubmitting.value || !turnId || traceTimeline.value.length === 0) {
      return null;
    }

    // Backward search instead of full reverse+find — avoids copying the entire array
    const allMessages = messages.value;
    let latestUserMessage: (typeof allMessages)[number] | undefined;
    for (let i = allMessages.length - 1; i >= 0; i--) {
      const msg = allMessages[i]!;
      if (msg.turnId === turnId && msg.role === "user") {
        latestUserMessage = msg;
        break;
      }
    }

    return {
      turnId,
      title: latestUserMessage?.content?.trim() || "当前执行中",
      phase: phase.value,
      traceSteps: traceSteps.value,
      traceTimeline: traceTimeline.value,
      toolActivities: toolActivities.value,
      providerName: providerName.value,
      providerProtocol: providerProtocol.value,
      providerModel: providerModel.value,
      providerMode: providerMode.value,
      fallbackReason: fallbackReason.value,
      error: error.value,
      inputTokens: inputTokens.value,
      outputTokens: outputTokens.value,
      totalTokens: totalTokens.value,
      firstTokenLatencyMs: firstTokenLatencyMs.value,
      // R3：活跃轮携带 per-call 记录，使 S1/S3 不再回退到 store 全局累计值。
      providerCallRecords: cloneProviderCallRecords(activeProviderCallRecords.value),
      updatedAt: Date.now()
    };
  });

  const orderedTurnTraces = computed(() => {
    const turns = [...turnTraceHistory.value];
    const activeTurn = liveTraceTurn.value;
    if (activeTurn) {
      const existingIndex = turns.findIndex((turn) => turn.turnId === activeTurn.turnId);
      if (existingIndex >= 0) {
        turns[existingIndex] = activeTurn;
      } else {
        turns.push(activeTurn);
      }
    }
    return turns.sort(compareTurnTraceOrder);
  });
  // latestTurn / turnTimelineCache 仅作内部派生源（PA-096 review B-P3-3：不对外导出）。
  const latestTurn = computed(() => orderedTurnTraces.value[orderedTurnTraces.value.length - 1] ?? null);

  const retrievedSessionContext = computed(() => retrievedContext.value?.sessionContext ?? null);

  // S1（§7.6 冻结解析序）：当前上下文占用 = 最近一次已完成模型调用的 per-call input。
  // 多调用轮且无任何 per-call 来源时返回 null（S4 纪律），严禁回退整轮累加值。
  const contextDisplayTokens = computed(() => resolveCurrentContextInputTokens(latestTurn.value));
  const currentContextWindowTokens = computed(
    () =>
      retrievedSessionContext.value?.contextWindowTokens ??
      providerStore.currentModel?.capabilities?.contextWindowTokens ??
      null
  );
  const sessionTurnCount = computed(() => orderedTurnTraces.value.length);

  // Cached timeline per turn — computed once per orderedTurnTraces change，
  // 供内部聚合计数消费（不对外导出）。
  const turnTimelineCache = computed(() => {
    const cache = new Map<string, TraceTimelineEntry[]>();
    for (const turn of orderedTurnTraces.value) {
      cache.set(turn.turnId, projectedTurnTimeline(turn));
    }
    return cache;
  });

  const sessionModelCallCount = computed(() => {
    const cache = turnTimelineCache.value;
    let sum = 0;
    for (const entries of cache.values()) {
      for (const entry of entries) {
        if (canonicalTraceTimelineKind(entry.kind) === "call_model") {
          sum++;
        }
      }
    }
    return sum;
  });

  const sessionToolCallCount = computed(() => {
    const cache = turnTimelineCache.value;
    let sum = 0;
    for (const entries of cache.values()) {
      for (const entry of entries) {
        if (canonicalTraceTimelineKind(entry.kind) === "call_tool") {
          sum++;
        }
      }
    }
    return sum;
  });

  const sessionInputTokensTotal = computed(() =>
    orderedTurnTraces.value.reduce((sum, turn) => sum + (turn.inputTokens ?? 0), 0)
  );
  const sessionCacheHitTokensTotal = computed(() =>
    orderedTurnTraces.value.reduce(
      (sum, turn) => sum + (providerReturnedCacheHitInputTokens(turn) ?? 0),
      0
    )
  );
  const sessionOutputTokensTotal = computed(() =>
    orderedTurnTraces.value.reduce((sum, turn) => sum + (turn.outputTokens ?? 0), 0)
  );
  const sessionCacheHitRatio = computed(() => {
    if (sessionInputTokensTotal.value <= 0) {
      return "";
    }

    return `${Math.round((sessionCacheHitTokensTotal.value / sessionInputTokensTotal.value) * 100)}%`;
  });
  const showContextUsage = computed(() => !!latestTurn.value);

  /**
   * S3（§7.6 冻结解析序，步骤 2–4 属调用方职责，§7.8）：
   * 1) 最后一条已完成 provider call record（record 级解析 + 守卫在 token-metrics）；
   * 2) 否则该轮最后一条 `call_model` 条目的 per-call 指标；
   * 3) 否则仅当轮内 provider call 数 ≤ 1 时回退 turn 级 output/duration；
   * 4) 否则 null（严禁用整轮累加 output / elapsed 计算）。
   */
  function resolveTurnCallGenerationSpeed(turn: TurnTraceRecord): number | null {
    const records = turn.providerCallRecords ?? [];
    for (let i = records.length - 1; i >= 0; i--) {
      const record = records[i];
      if (!record) {
        continue;
      }
      const speed = resolveCallGenerationSpeed(record);
      if (speed != null) {
        return speed;
      }
    }

    const entries = turnTimelineCache.value.get(turn.turnId) ?? projectedTurnTimeline(turn);
    for (let i = entries.length - 1; i >= 0; i--) {
      const entry = entries[i]!;
      if (canonicalTraceTimelineKind(entry.kind) !== "call_model") {
        continue;
      }
      const speed = entryCallGenerationSpeed(entry);
      if (speed != null) {
        return speed;
      }
    }

    if (countTurnProviderCalls(turn) <= 1) {
      return turnTokenSpeed(turn);
    }

    return null;
  }

  // 当前模型生成速度与计算模式
  const sessionTokenSpeedState = computed<TokenSpeedDetail>(() => {
    const turns = orderedTurnTraces.value;
    for (let t = turns.length - 1; t >= 0; t--) {
      const turn = turns[t]!;
      // 检查最新轮是否为活跃提交状态
      const isActive = isSubmitting.value && t === turns.length - 1;
      const detail = resolveDualModeTokenSpeedDetail(turn, { isSubmitting: isActive, isActiveTurn: isActive });
      if (detail.speed != null) {
        return detail;
      }
      // 回退旧的 timeline per-call 解析
      const legacySpeed = resolveTurnCallGenerationSpeed(turn);
      if (legacySpeed != null) {
        return { speed: legacySpeed, mode: "call_fallback" };
      }
    }
    return { speed: null, mode: "none" };
  });

  const sessionTokenGenerationSpeed = computed<number | null>(() => sessionTokenSpeedState.value.speed);
  const tokenSpeedCalculationMode = computed<TokenSpeedCalculationMode>(() => sessionTokenSpeedState.value.mode);

  const tokenSpeedDebugInfo = computed(() => {
    const state = sessionTokenSpeedState.value;
    const windowTokens = currentContextWindowTokens.value ?? null;
    const inTokens = state.inputTokens ?? contextDisplayTokens.value ?? null;
    let ratio: string | null = null;
    if (windowTokens && windowTokens > 0 && inTokens != null) {
      ratio = (inTokens / windowTokens).toFixed(3);
    }
    return {
      mode: state.mode,
      speed: state.speed,
      inputTokens: inTokens,
      outputTokens: state.outputTokens ?? null,
      durationMs: state.durationMs ?? null,
      ttftMs: state.ttftMs ?? null,
      contextWindow: windowTokens,
      ratio,
    };
  });

  // 状态变化时触发调试埋点与异常检测 (task-11)
  watch(
    [sessionTokenSpeedState, contextDisplayTokens, currentContextWindowTokens],
    ([speedState, inputTokens, contextWindow]) => {
      const turns = orderedTurnTraces.value;
      const latest = turns.length > 0 ? turns[turns.length - 1] : null;
      const windowTokens = contextWindow ?? 0;
      const inTokens = speedState.inputTokens ?? inputTokens;
      const ratio = windowTokens > 0 && inTokens != null ? Number((inTokens / windowTokens).toFixed(4)) : null;

      // 1. 结构化快照记录
      if (speedState.mode !== "none" || inTokens != null) {
        recordMetricsSnapshot({
          turnId: latest?.turnId,
          mode: speedState.mode,
          speed: speedState.speed,
          inputTokens: inTokens,
          contextWindow: windowTokens,
          contextRatio: ratio,
          outputTokens: speedState.outputTokens ?? latest?.outputTokens,
          durationMs: speedState.durationMs ?? latest?.turnDurationMs,
          ttftMs: speedState.ttftMs ?? latest?.firstTokenLatencyMs,
        });
      }

      // 2. 异常值节流上报
      if (latest?.turnDurationMs != null && latest.turnDurationMs <= 0) {
        reportMetricsAnomaly("invalid_duration", {
          turnId: latest.turnId,
          durationMs: latest.turnDurationMs,
          outputTokens: latest.outputTokens,
        });
      }
      if (speedState.speed != null && speedState.speed < 0) {
        reportMetricsAnomaly("negative_speed", {
          turnId: latest?.turnId,
          speed: speedState.speed,
          mode: speedState.mode,
        });
      }
      if (windowTokens > 0 && inputTokens != null && inputTokens > windowTokens) {
        reportMetricsAnomaly("context_overflow", {
          turnId: latest?.turnId,
          inputTokens,
          contextWindow: windowTokens,
        });
      }
    },
    { immediate: true, flush: "post" }
  );

  return {
    orderedTurnTraces,
    sessionTurnCount,
    sessionModelCallCount,
    sessionToolCallCount,
    sessionInputTokensTotal,
    sessionCacheHitTokensTotal,
    sessionOutputTokensTotal,
    sessionCacheHitRatio,
    sessionTokenGenerationSpeed,
    tokenSpeedCalculationMode,
    tokenSpeedDebugInfo,
    contextDisplayTokens,
    currentContextWindowTokens,
    showContextUsage
  };
}
