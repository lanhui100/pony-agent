// D5 遥测/指标/Provider 标注（18 键 / 9 getters / 13 actions）
//
// 实现自 `2f11dc2f:src/stores/runtime.ts` **逐字搬迁**，语义零改动。键清单与签名真源见 `./types.ts`，
// 域归属与切分口径见 `.dev-team/runtime-split/fact-map.md`。与原实现的机械差异：import 路径按新层级
// 调整；action 方法补 `this: RuntimeStoreThis` 形参（Pinia options 式 `this` 口径）；类型
// `RuntimeStateW59` 改名 `RuntimeStoreState`（types.ts 的等价迁出物）。
import { isTauriAvailable, safeInvoke } from "@/lib/tauri";
import { useSettingsStore } from "@/stores/settings";
import type {
  ProviderCallCacheRecord,
  TraceTimelineEntry,
  TurnMetricsSnapshot,
  TurnTraceRecord,
} from "@/types/runtime";
import { debugLog } from "@/lib/runtime/utils";
import { TRACE_TIMELINE_THROTTLE_MS } from "@/lib/runtime/constants";
import {
  canonicalizeTraceTimelineKind,
  cloneHookTraceRecords,
  cloneProviderCallRecords,
  cloneTraceTimeline,
  createDefaultTraceTimeline,
  normalizeTurnTraceRecord,
} from "@/lib/runtime/trace";
import { buildTurnTraceTitleFromMessages, completedPrefixIndex } from "@/lib/runtime/messages";
import { buildEventCursorByTurnTraceHistory } from "@/lib/runtime/sessions";
import type { RuntimeStoreThis, TelemetryActions, TelemetryGetters } from "./types";

/** D5 遥测/指标/Provider 标注（18 键 / 9 getters / 13 actions） getters（键清单见 `types.ts` 的 `TelemetryGetters` 与事实图 §2）。 */
export const telemetryGetters = {
  /**
   * Wave-4 C2：读取当前活跃 turn 的 OTel trace_id（若无则返回 undefined）。
   * 语义：
   * 1. 仅当存在 activeTurnId 时，从 turnTraceHistory 中查找该活跃 turn 的 traceId 并返回；
   *    若该活跃 turn 尚未记录 traceId 则严格返回 undefined，杜绝将上一轮的 traceId 误归因给本轮。
   * 2. 无 activeTurnId（空闲态或终态后）时，回落到最近一个已完成 turn 的 traceId 供上下文调试。
   */
  activeTraceId: (state): string | undefined => {
    const activeId = state.activeTurnId;
    if (activeId) {
      const activeTurn = state.turnTraceHistory?.find((t) => t.turnId === activeId);
      return activeTurn?.traceId || undefined;
    }
    for (let i = (state.turnTraceHistory?.length ?? 0) - 1; i >= 0; i--) {
      const turn = state.turnTraceHistory[i];
      if (turn?.phase === "completed" && turn.traceId) {
        return turn.traceId;
      }
    }
    return undefined;
  },
  /** Wave-5 K1/K2：会话累计成本 (USD)，仅累加有效轮次，空值轮跳过 */
  sessionTotalCostUsd: (state): number => {
    let sum = 0;
    for (const turn of state.turnTraceHistory ?? []) {
      if (typeof turn.costUsd === "number" && Number.isFinite(turn.costUsd) && turn.costUsd >= 0) {
        sum += turn.costUsd;
      }
    }
    return Number(sum.toFixed(6));
  },
  /** 标识会话中是否含有未计价（未知模型）的轮次 */
  hasUnpricedTurns: (state): boolean => {
    const history = state.turnTraceHistory ?? [];
    if (history.length === 0) return false;
    return history.some((t) => t.costUsd === undefined || t.costUsd === null);
  },
  /** 格式化输出会话成本；全部轮次无计价或异常时退化为 "—" */
  formattedSessionCost(): string {
    const history = this.turnTraceHistory ?? [];
    const validCosts = history.filter(
      (t) => typeof t.costUsd === "number" && Number.isFinite(t.costUsd) && t.costUsd >= 0
    );
    if (validCosts.length === 0) {
      return "—";
    }
    const total = this.sessionTotalCostUsd;
    if (!Number.isFinite(total) || total < 0) {
      return "—";
    }
    return `$${total.toFixed(6)}`;
  },
  /** Wave-5 K3：预算超限软告警状态 */
  isCostBudgetExceeded(): boolean {
    const settingsStore = useSettingsStore();
    const budget = (settingsStore as any).effectiveCostBudgetUsd ?? (settingsStore as any).costBudgetUsd;
    if (typeof budget !== "number" || !Number.isFinite(budget) || budget <= 0) {
      return false;
    }
    return this.sessionTotalCostUsd > budget;
  },
  /** 预算超限提示文案 */
  costBudgetWarningMessage(): string | null {
    if (!this.isCostBudgetExceeded) return null;
    const settingsStore = useSettingsStore();
    const budget = (settingsStore as any).effectiveCostBudgetUsd ?? (settingsStore as any).costBudgetUsd;
    return `已超出成本预算限额 (当前超预算: $${Number(budget).toFixed(2)})`;
  },
  /** Wave-7 G4：用量治理汇总（纯前端聚合或来自后端 summary） */
  usageGovernanceSummary: (state): any => {
    // 优先消费后端 summary（若存在）
    if ((state as any).backendGovernanceSummary) {
      return (state as any).backendGovernanceSummary;
    }
    let costSum = 0;
    let unpricedCount = 0;
    for (const turn of state.turnTraceHistory ?? []) {
      if (typeof turn.costUsd === "number" && Number.isFinite(turn.costUsd) && turn.costUsd >= 0) {
        costSum += turn.costUsd;
      } else {
        unpricedCount++;
      }
    }
    return {
      session: {
        cost_usd: Number(costSum.toFixed(6)),
        unpriced_turns: unpricedCount,
      },
      days: [],
      policy_version: "v1",
      governance_degraded: false,
    };
  },
  /** Wave-7 G4：格式化展示用量治理成本（并列展示未计价轮数与 ≥ 语义，全未计价显示 "—"） */
  formattedGovernanceCost(): string {
    const summary = this.usageGovernanceSummary;
    const history = this.turnTraceHistory ?? [];
    const validTurns = history.filter(
      (t) => typeof t.costUsd === "number" && Number.isFinite(t.costUsd) && t.costUsd >= 0
    );
    if (validTurns.length === 0) {
      return "—";
    }
    const costStr = `$${summary.session.cost_usd.toFixed(6)}`;
    if (summary.session.unpriced_turns > 0) {
      return `≥ ${costStr} · ${summary.session.unpriced_turns} 轮未计价`;
    }
    return costStr;
  },
  /** Wave-7 G2/G3：拦截态详情 UI 信息 */
  governanceBlockDetail(state): any {
    const decision = (state as any).governanceDecision;
    if (!decision || decision.kind !== "Block") {
      return null;
    }
    return {
      currentText: `$${Number(decision.current).toFixed(3)}`,
      limitText: `$${Number(decision.limit).toFixed(2)}`,
      reason: decision.reason,
      overrideActionAvailable: true,
    };
  },
} as TelemetryGetters;

/** D5 遥测/指标/Provider 标注（18 键 / 9 getters / 13 actions） actions（键清单见 `types.ts` 的 `TelemetryActions` 与事实图 §3）。 */
export const telemetryActions = {
  upsertTurnTrace(this: RuntimeStoreThis, 
    turnId: string,
    patch: Partial<Omit<TurnTraceRecord, "turnId" | "updatedAt">> & { updatedAt?: number },
    persist = true
  ) {
    const existing = this.turnTraceHistory.find((item) => item.turnId === turnId);
    const updatedAt = patch.updatedAt ?? Date.now();
    const existingTitle = existing?.title?.trim();
    const resolvedTitle =
      patch.title ??
      (existingTitle && existingTitle !== "未命名轮次" ? existingTitle : undefined) ??
      buildTurnTraceTitleFromMessages(this.messages, turnId);

    // When patch doesn't include traceTimeline (e.g. from commitTurnTraceTimeline
    // where the caller already set this.traceTimeline and guarantees freshness),
    // fallback to this.traceTimeline. This avoids redundant double clones.
    const patchTimeline = Object.prototype.hasOwnProperty.call(patch, "traceTimeline")
      ? (patch as any).traceTimeline
      : this.traceTimeline;

    if (existing) {
      Object.assign(existing, patch, {
        title: resolvedTitle,
        updatedAt,
        traceTimeline: patchTimeline ? cloneTraceTimeline(patchTimeline) : existing.traceTimeline,
        providerCallRecords:
          patch.providerCallRecords != null
            ? cloneProviderCallRecords(patch.providerCallRecords)
            : existing.providerCallRecords,
        hookTraceRecords:
          patch.hookTraceRecords != null
            ? cloneHookTraceRecords(patch.hookTraceRecords)
            : existing.hookTraceRecords
      });
      this.eventCursorByTurnId = buildEventCursorByTurnTraceHistory(this.turnTraceHistory);
      if (persist) {
        this.persistHistory();
      }
      return;
    }

    // Pass raw values — normalizeTurnTraceRecord handles all deep cloning internally,
    // so pre-cloning here would result in redundant double-clones.
    this.turnTraceHistory.push(normalizeTurnTraceRecord({
      turnId,
      title: patch.title ?? "未命名轮次",
      phase: patch.phase ?? this.phase,
      traceSteps: patch.traceSteps ?? [],
      traceTimeline: patchTimeline ?? [],
      traceId: patch.traceId ?? null,
      toolActivities: patch.toolActivities ?? [],
      providerCallRecords: patch.providerCallRecords ?? [],
      hookTraceRecords: patch.hookTraceRecords ?? [],
      providerRequestedName: patch.providerRequestedName ?? null,
      providerName: patch.providerName ?? null,
      providerProtocol: patch.providerProtocol ?? null,
      providerModel: patch.providerModel ?? null,
      providerSource: patch.providerSource ?? null,
      providerMode: patch.providerMode ?? null,
      buildContextObservation: patch.buildContextObservation ?? null,
      sessionSummary: patch.sessionSummary ?? "",
      fallbackReason: patch.fallbackReason ?? null,
      error: patch.error ?? null,
      inputTokens: patch.inputTokens ?? null,
      cacheHitInputTokens: patch.cacheHitInputTokens ?? null,
      reasoningTokens: patch.reasoningTokens ?? null,
      outputTokens: patch.outputTokens ?? null,
      totalTokens: patch.totalTokens ?? null,
      firstTokenLatencyMs: patch.firstTokenLatencyMs ?? null,
      turnDurationMs: patch.turnDurationMs ?? null,
      costUsd: patch.costUsd ?? undefined,
      updatedAt
    }));
    this.turnTraceHistory[this.turnTraceHistory.length - 1]!.title = resolvedTitle;
    this.eventCursorByTurnId = buildEventCursorByTurnTraceHistory(this.turnTraceHistory);
    if (persist) {
      this.persistHistory();
    }
  },
  // 单一 trace timeline 发布入口（PA-086）：所有写路径收敛于此，
  // 保证投影层（签名化 memo）与源数据强一致。
  publishTraceTimeline(this: RuntimeStoreThis, traceTimeline: TraceTimelineEntry[]) {
    this.traceTimeline = traceTimeline;
  },
  // delta 高频事件中的 timeline 节流更新：只保留最新一份，定时合并应用。
  scheduleThrottledTraceTimeline(this: RuntimeStoreThis, traceTimeline: TraceTimelineEntry[]) {
    this.pendingThrottledTraceTimeline = traceTimeline;
    if (this.traceTimelineThrottleTimerId != null) {
      return;
    }
    this.traceTimelineThrottleTimerId = window.setTimeout(() => {
      this.traceTimelineThrottleTimerId = null;
      const pending = this.pendingThrottledTraceTimeline;
      this.pendingThrottledTraceTimeline = null;
      if (pending) {
        // 直接更新活跃 traceTimeline，避免空闲回调延迟造成中间工具/思考链长时间不呈现
        this.updateActiveTraceTimeline(pending);
      }
    }, TRACE_TIMELINE_THROTTLE_MS);
  },
  // 冲刷挂起的 timeline 更新（低频语义事件 / terminal 事件前调用，保证最终态一致）。
  flushPendingTraceTimeline(this: RuntimeStoreThis) {
    if (this.traceTimelineThrottleTimerId != null) {
      window.clearTimeout(this.traceTimelineThrottleTimerId);
      this.traceTimelineThrottleTimerId = null;
    }
    const pending = this.pendingThrottledTraceTimeline;
    this.pendingThrottledTraceTimeline = null;
    if (pending) {
      this.updateActiveTraceTimeline(pending);
    }
  },
  clearPendingTraceTimeline(this: RuntimeStoreThis) {
    if (this.traceTimelineThrottleTimerId != null) {
      window.clearTimeout(this.traceTimelineThrottleTimerId);
      this.traceTimelineThrottleTimerId = null;
    }
    this.pendingThrottledTraceTimeline = null;
  },
  /**
   * R4/Wave-15：以事件携带的 per-call 记录维护「活跃轮最近一次已完成模型调用」的物理指标。
   *
   * - `reset=true`（新轮开始）：先复位列表；事件未携带记录时列表保持为空。
   * - 携带记录时整体覆盖（不累加、幂等），并把活跃轮 `inputTokens` / `outputTokens`
   *   置为该列表**末条**的 per-call 值。
   * - 未携带记录时不触碰既有 token 值（严禁回退整轮累加值）。
   */
  applyActiveProviderCallRecords(this: RuntimeStoreThis, 
    records: ProviderCallCacheRecord[] | null | undefined,
    reset = false
  ): void {
    if (reset) {
      this.activeProviderCallRecords = [];
    }
    if (records?.length) {
      this.activeProviderCallRecords = cloneProviderCallRecords(records);
    }

    const lastRecord = this.activeProviderCallRecords[this.activeProviderCallRecords.length - 1];
    if (lastRecord?.inputTokens != null) {
      this.inputTokens = lastRecord.inputTokens;
    }
    if (lastRecord?.outputTokens != null) {
      this.outputTokens = lastRecord.outputTokens;
    }
  },
  /**
   * wave-metrics（task-4）：落库 `turn` 事件携带的结算快照（契约 §0 单一消费位）。
   *
   * **冻结语义**（`tests/metrics-snapshot-live.spec.ts` D3/D4 + 对抗用例 ADV-2-3）：
   * - `metrics == null`（`null` / `undefined`，即旧事件线缆键缺席）⇒ **保持既有快照不变**，
   *   绝不把「本轮无快照」当成「清空」指令（I7）。清空只发生在显式的新轮/会话切换点。
   * - `metrics` 为对象 ⇒ 整体覆盖（同一轮 step 前进、或下一轮的新快照都以最新为准）。
   * 不在此处做字段校验/归一：畸形线缆同样是「有值输入」，消费侧解析按无值处理且不得抛错。
   */
  applyMetricsSnapshot(this: RuntimeStoreThis, metrics?: TurnMetricsSnapshot | null): void {
    if (metrics == null) {
      return;
    }
    this.liveMetrics = metrics;
  },
  resolveTurnTraceTimeline(this: RuntimeStoreThis, turnId: string) {
    const existingTimeline = cloneTraceTimeline(
      this.turnTraceHistory.find((trace) => trace.turnId === turnId)?.traceTimeline
    );
    if (existingTimeline.length) {
      return existingTimeline;
    }

    return this.activeTurnId === turnId && this.traceTimeline.length
      ? cloneTraceTimeline(this.traceTimeline)
      : createDefaultTraceTimeline();
  },
  commitTurnTraceTimeline(this: RuntimeStoreThis, 
    turnId: string,
    traceTimeline: TraceTimelineEntry[],
    patch: Partial<Omit<TurnTraceRecord, "turnId" | "updatedAt">> & { updatedAt?: number } = {},
    persist = true
  ) {
    // resolveEventTraceTimeline/buildFallbackRuntimeTraceTimeline already return a fresh timeline snapshot.
    this.publishTraceTimeline(traceTimeline);
    // Don't pass traceTimeline in patch — upsertTurnTrace will read this.traceTimeline
    // to avoid a redundant deep clone (the caller already guarantees freshness).
    this.upsertTurnTrace(turnId, patch, persist);
    // Lightweight debug log — full buildCacheTelemetryDebugSnapshot is already
    // called in STAGE 1 of each terminal event handler (completed/failed/cancelled),
    // so we avoid the redundant expensive computation here.
    debugLog("cache-telemetry:trace-committed", {
      turnId,
      phase: patch.phase ?? this.phase,
      traceTimelineLength: this.traceTimeline.length
    });
  },
  updateActiveTraceTimeline(this: RuntimeStoreThis, traceTimeline: TraceTimelineEntry[], skipClone = false) {
    // 低频语义事件路径（turn:trace / phase_changed / checkpoint_persisted / tool）中，
    // resolveEventTraceTimeline 已返回新鲜克隆，skipClone 避免第二次全量深拷贝。
    this.publishTraceTimeline(skipClone ? traceTimeline : cloneTraceTimeline(traceTimeline));
  },
  updateActiveModelTraceFromAssistant(this: RuntimeStoreThis, turnId: string) {
    const assistantMessage = this.messages.find((message) => message.turnId === turnId && message.role === "assistant");
    if (!assistantMessage || !this.traceTimeline.length) {
      return;
    }

    // 调用点（低频语义事件）保证 this.traceTimeline 已是新鲜克隆，
    // 此处就地替换 model entry，避免第三次全量深拷贝。
    const traceTimeline = this.traceTimeline;
    let modelIndex = -1;
    for (let i = traceTimeline.length - 1; i >= 0; i--) {
      if (canonicalizeTraceTimelineKind(traceTimeline[i]!.kind) === "call_model") {
        modelIndex = i;
        break;
      }
    }
    if (modelIndex === -1) {
      return;
    }

    const modelContentForCurrentHop = (content: string | null | undefined, field: "text" | "reasoningContent") => {
      if (!content) {
        return null;
      }

      const completedContents = traceTimeline
        .slice(0, modelIndex)
        .filter((entry) => canonicalizeTraceTimelineKind(entry.kind) === "call_model")
        .map((entry) => entry[field] ?? "");
      const completedStartIndex = completedPrefixIndex(completedContents, content);
      const completedPrefix =
        completedStartIndex >= 0 ? completedContents.slice(completedStartIndex).join("") : "";
      if (!completedPrefix || content.startsWith(completedPrefix)) {
        return content.slice(completedPrefix.length) || null;
      }

      return content;
    };
    const modelEntry = traceTimeline[modelIndex]!;
    const currentText = modelContentForCurrentHop(assistantMessage.content, "text");
    const currentReasoning = modelContentForCurrentHop(
      assistantMessage.reasoningContent,
      "reasoningContent"
    );
    traceTimeline[modelIndex] = {
      ...modelEntry,
      state: assistantMessage.status === "pending" ? "active" : modelEntry.state,
      text: currentText ?? modelEntry.text ?? null,
      reasoningContent: currentReasoning ?? modelEntry.reasoningContent ?? null,
      firstTokenLatencyMs: this.firstTokenLatencyMs ?? modelEntry.firstTokenLatencyMs ?? null
    };
    this.publishTraceTimeline(traceTimeline);
  },
  async overrideGovernanceBlock(this: RuntimeStoreThis) {
    (this as any).governanceDecision = null;
    try {
      const settingsStore = useSettingsStore();
      if (settingsStore.settings.usageGovernancePolicy?.enforcement) {
        settingsStore.settings.usageGovernancePolicy.enforcement = false;
        await settingsStore.saveSettings();
      }
    } catch {
      // ignore
    }
  },
  async fetchUsageGovernanceSummary(this: RuntimeStoreThis) {
    if (!isTauriAvailable()) return;
    try {
      const res = await safeInvoke<any>("usage_governance_summary", { sessionId: this.sessionId });
      if (res) {
        (this as any).backendGovernanceSummary = res;
      }
    } catch {
      // 优雅降级，未接线时不报错
    }
  },
} as TelemetryActions;
