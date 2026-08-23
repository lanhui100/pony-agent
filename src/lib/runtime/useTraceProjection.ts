import { computed, ref, type ComputedRef, type Ref } from "vue";
import { storeToRefs } from "pinia";
import type { TraceTimelineEntry, TurnTraceRecord } from "@/types/runtime";
import { useProviderStore } from "@/stores/providers";
import { useRuntimeStore } from "@/stores/runtime";
import {
  canonicalTraceTimelineKind,
  turnTimeline as projectedTurnTimeline
} from "@/lib/runtime/trace-projection";
import { resolveProviderReturnedCacheHitInputTokens } from "@/lib/runtime/trace";

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

  const contextDisplayTokens = computed(() => {
    if (inputTokens.value != null) return inputTokens.value;
    if (isSubmitting.value && traceTimeline.value?.length) {
      for (let i = traceTimeline.value.length - 1; i >= 0; i--) {
        const entry = traceTimeline.value[i];
        if (canonicalTraceTimelineKind(entry.kind) === "call_model" && entry.inputTokens != null) {
          return entry.inputTokens;
        }
      }
    }
    for (let i = turnTraceHistory.value.length - 1; i >= 0; i--) {
      const turn = turnTraceHistory.value[i];
      if (turn.turnId !== runtimeActiveTurnId.value && turn.inputTokens != null) {
        return turn.inputTokens;
      }
    }
    return latestTurn.value?.inputTokens ?? null;
  });
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

    return `${((sessionCacheHitTokensTotal.value / sessionInputTokensTotal.value) * 100).toFixed(1)}%`;
  });
  const showContextUsage = computed(() => !!latestTurn.value);

  return {
    orderedTurnTraces,
    sessionTurnCount,
    sessionModelCallCount,
    sessionToolCallCount,
    sessionInputTokensTotal,
    sessionCacheHitTokensTotal,
    sessionOutputTokensTotal,
    sessionCacheHitRatio,
    contextDisplayTokens,
    currentContextWindowTokens,
    showContextUsage
  };
}
