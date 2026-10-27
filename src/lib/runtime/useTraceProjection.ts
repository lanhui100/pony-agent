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
  resolveContextInputTokens,
  resolveCurrentContextInputTokens,
  resolveLastCallCacheInputTokens,
  resolveTurnWeightedSpeedDetail,
  type TokenSpeedCalculationMode,
  type TurnWeightedSpeedDetail,
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
 * wave-54 §4：速度只保留一条解析器 `resolveTurnWeightedSpeedDetail`（C2 加权口径）。
 * 旧的 `entryCallGenerationSpeed` / `turnTokenSpeed` / `resolveTurnCallGenerationSpeed`
 * 与 dual-mode 遮蔽路径已删除——同一语义位不得再有两套分支。
 */

/** 空详情（无任何轮次 / 全部无数据）。 */
const EMPTY_SPEED_DETAIL: TurnWeightedSpeedDetail = {
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

  // C4 取值序（wave-54 §1 C4）：「输入」= 末次调用的 contextInputTokens（协议归一值）
  // → 回退 S1（wave-15 §7.6 冻结解析序）→ 无数据时 null（UI 渲染 `—`）。
  const contextDisplayTokens = computed(
    () => resolveContextInputTokens(latestTurn.value) ?? resolveCurrentContextInputTokens(latestTurn.value)
  );

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
  /**
   * C6 显示纪律：`output` / `cache` 与 `contextInputTokens` 同法——无数据一律 null（UI `—`），
   * 绝不把「无数据」伪装成 `0`。聚合语义（Σ 轮级累加）保持不变。
   */
  const sessionCacheHitTokensTotal = computed<number | null>(() => {
    let hasValue = false;
    let sum = 0;
    for (const turn of orderedTurnTraces.value) {
      const value = providerReturnedCacheHitInputTokens(turn);
      if (value != null) {
        hasValue = true;
        sum += value;
      }
    }

    return hasValue ? sum : null;
  });
  const sessionOutputTokensTotal = computed<number | null>(() => {
    let hasValue = false;
    let sum = 0;
    for (const turn of orderedTurnTraces.value) {
      if (turn.outputTokens != null) {
        hasValue = true;
        sum += turn.outputTokens;
      }
    }

    return hasValue ? sum : null;
  });
  /** C6：缓存比的分子 = 末次已完成调用的缓存命中输入（与可见「输入」同源）。 */
  const sessionLastCallCacheHitTokens = computed(() => resolveLastCallCacheInputTokens(latestTurn.value));
  // C6：缓存比 = 末次调用 cache / 可见「输入」（同源分母），整数百分比。
  // 会话累计口径 sessionInputTokensTotal / sessionCacheHitTokensTotal 保持不变（账单语义）。
  const sessionCacheHitRatio = computed(() => {
    const denominator = contextDisplayTokens.value;
    const cache = sessionLastCallCacheHitTokens.value;
    if (denominator == null || !(denominator > 0) || cache == null) {
      return "";
    }

    return `${Math.round((cache / denominator) * 100)}%`;
  });
  const showContextUsage = computed(() => !!latestTurn.value);

  // C2 单口径：`resolveTurnWeightedSpeedDetail` 是唯一解析器（§4）。
  // 从最新轮次向前取第一条有速度的详情；全部无速度时保留最新轮次的详情（basis/calls/origin 仍可展示）。
  const sessionTokenSpeedState = computed<TurnWeightedSpeedDetail>(() => {
    const turns = orderedTurnTraces.value;
    let latestDetail: TurnWeightedSpeedDetail | null = null;
    for (let t = turns.length - 1; t >= 0; t--) {
      const turn = turns[t]!;
      const isActive = isSubmitting.value && t === turns.length - 1;
      const detail = resolveTurnWeightedSpeedDetail(turn, { isActiveTurn: isActive });
      if (latestDetail == null) {
        latestDetail = detail;
      }
      if (detail.speed != null) {
        return detail;
      }
    }

    return latestDetail ?? EMPTY_SPEED_DETAIL;
  });

  const sessionTokenGenerationSpeed = computed<number | null>(() => sessionTokenSpeedState.value.speed);
  const tokenSpeedCalculationMode = computed<TokenSpeedCalculationMode>(() => sessionTokenSpeedState.value.mode);

  /**
   * C6 调试/`≈` 通道（§5 裁决 4：由 `origin` 子字段驱动，不新增 prop）：
   * `data-speed-debug` = `basis=…;calls=…;in=…;out=…;dur=…;ttft=…;gen=…;win=…;ratio=…;origin=…`。
   */
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
      basis: state.basis,
      calls: state.calls,
      speed: state.speed,
      inputTokens: inTokens,
      outputTokens: state.outputTokens ?? null,
      durationMs: state.durationMs ?? null,
      genMs: state.genMs ?? null,
      ttftMs: state.ttftMs ?? null,
      contextWindow: windowTokens,
      ratio,
      origin: state.origin,
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
        }, latest?.traceId);
      }
      if (speedState.speed != null && speedState.speed < 0) {
        reportMetricsAnomaly("negative_speed", {
          turnId: latest?.turnId,
          speed: speedState.speed,
          mode: speedState.mode,
        }, latest?.traceId);
      }
      if (windowTokens > 0 && inputTokens != null && inputTokens > windowTokens) {
        reportMetricsAnomaly("context_overflow", {
          turnId: latest?.turnId,
          inputTokens,
          contextWindow: windowTokens,
        }, latest?.traceId);
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
