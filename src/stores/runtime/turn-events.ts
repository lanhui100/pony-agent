// turn 事件订阅聚合（0 键 / 0 getters / 3 actions，事实图 §5）
//
// 实现自 `2f11dc2f:src/stores/runtime.ts` **逐字搬迁**，语义零改动。键清单与签名真源见 `./types.ts`，
// 域归属与切分口径见 `.dev-team/runtime-split/fact-map.md`。与原实现的机械差异：import 路径按新层级
// 调整；action 方法补 `this: RuntimeStoreThis` 形参（Pinia options 式 `this` 口径）；类型
// `RuntimeStateW59` 改名 `RuntimeStoreState`（types.ts 的等价迁出物）。
// 注册表生命周期辅助（`turnEventRegistry` / `turnEventRegistryFor` / `releaseTurnEventRegistry`）
// 与 `initializeTurnEvents` / `disposeTurnEvents` 分处两个模块，故按原调用关系加 `export` 共享；
// `TurnEventRegistry` 接口改由 `./types.ts` 导入（Wave 1 契约单一真源）。监听器聚合体的 `this`
// 另需 Pinia 内建 `$patch`（原 `this` 是 store 实例），以局部 `TurnEventsThis` 别名补齐类型面。
import { safeListen } from "@/lib/tauri";
import { reportError } from "@/lib/telemetry";
import { recordFrontendInstant } from "@/lib/frontend-flight-recorder";
import type {
  ChatMessage,
  TurnRetryEvent,
  TurnStreamEvent,
  TurnTraceRecord,
} from "@/types/runtime";
import { debugLog, isDebugLoggingEnabled, runLowPriorityTurnWork } from "@/lib/runtime/utils";
import {
  DEFAULT_FAILED_TURN_ERROR,
  MAX_BG_REASONING_BUFFER_CHARS,
  MAX_BG_TEXT_BUFFER_CHARS,
  TIMEOUT_RETRY_PENDING_MESSAGE,
} from "@/lib/runtime/constants";
import {
  buildAssistantModelLabel,
  buildCacheTelemetryDebugSnapshot,
  buildFallbackRuntimeTraceTimeline,
  canonicalizeTraceTimelineKind,
  cloneBuildContextObservation,
  cloneHookTraceRecords,
  cloneProviderCallRecords,
  finalizeCancelledTraceSteps,
  logCacheTelemetryContractViolations,
  normalizeReasoningContent,
  readNestedNumericTokenValue,
  resolveEventTraceTimeline,
  resolveSemanticEventTraceTimeline,
  resolveProviderReturnedCacheHitInputTokens,
  resolveReasoningTokens,
  resolveTerminalToolActivities,
  toolStatusToMessageStatus,
} from "@/lib/runtime/trace";
import { buildToolMessageDetail, buildTurnTraceTitleFromMessages } from "@/lib/runtime/messages";
import { isHistoricalMode } from "@/lib/runtime/history";
import { resolveRuntimePhaseFromEvent } from "@/lib/runtime/phases";
import { shouldAcceptTurnEvent } from "@/lib/runtime/sessions";
import { refreshForTurn as refreshPlanForTurn } from "@/stores/plan";
import { refreshForTurn as refreshGoalForTurn } from "@/stores/goal";
import type {
  RuntimeStoreBuiltins,
  RuntimeStoreThis,
  TurnEventRegistry,
  TurnEventsActions,
  TurnEventsGetters,
} from "./types";
/**
 * `registerTurnEventListeners` 的 `this` 口径：监听器聚合体里 6 处 `this.$patch(...)` 是既有实现，
 * 拆解前 `this` 是 Pinia store 实例。Pinia 内建成员的类型面统一由 `./types.ts` 的
 * `RuntimeStoreBuiltins` 登记（B278 契约），此处只取所需成员，不再本地重复定义。
 */
type TurnEventsThis = RuntimeStoreThis & Pick<RuntimeStoreBuiltins, "$patch">;

// Wave-1 C6：turn 结束（turn:completed / graph 流结束）后刷新计划与目标面板。
// - 同一 turnId 只刷新一次（事件去重/重放时避免重复拉取）；
// - 失败只记诊断（console.warn），绝不影响 turn 主流程；
// - 空 sessionId 由各 refreshForTurn 内部直接返回（不发起 IPC）。
const PLAN_GOAL_REFRESHED_TURNS_LIMIT = 64;
const planGoalRefreshedTurnIds = new Set<string>();

function refreshPlanGoalAfterTurn(
  turnId: string | null | undefined,
  sessionId: string | null | undefined
): void {
  const normalizedSessionId = sessionId?.trim() ?? "";
  if (!normalizedSessionId) {
    return;
  }

  const normalizedTurnId = turnId?.trim() ?? "";
  if (normalizedTurnId) {
    if (planGoalRefreshedTurnIds.has(normalizedTurnId)) {
      return;
    }
    planGoalRefreshedTurnIds.add(normalizedTurnId);
    if (planGoalRefreshedTurnIds.size > PLAN_GOAL_REFRESHED_TURNS_LIMIT) {
      const oldestTurnId = planGoalRefreshedTurnIds.values().next().value;
      if (oldestTurnId) {
        planGoalRefreshedTurnIds.delete(oldestTurnId);
      }
    }
  }

  try {
    void Promise.allSettled([
      refreshPlanForTurn(normalizedSessionId),
      refreshGoalForTurn(normalizedSessionId)
    ]).catch((error) => {
      console.warn("[wave-1] plan/goal refresh after turn failed:", error);
    });
  } catch (error) {
    console.warn("[wave-1] plan/goal refresh after turn failed:", error);
  }
}

/**
 * F5b：turn 事件监听注册表（按 store 实例隔离，不进入响应式 state）。
 * - `initPromise`：并发单例锁，避免并发初始化重复注册 12 个监听器；
 * - `unlisteners`：全部 unlisten 句柄，供 `disposeTurnEvents()` 幂等清理。
 */

export const turnEventRegistry = new WeakMap<object, TurnEventRegistry>();

export function turnEventRegistryFor(store: object): TurnEventRegistry {
  let registry = turnEventRegistry.get(store);
  if (!registry) {
    registry = { initPromise: null, unlisteners: [] };
    turnEventRegistry.set(store, registry);
  }
  return registry;
}

export function releaseTurnEventRegistry(registry: TurnEventRegistry): void {
  const unlisteners = registry.unlisteners.splice(0);
  for (const unlisten of unlisteners) {
    try {
      unlisten();
    } catch (error) {
      debugLog("events:unlisten:error", { error: String(error) });
    }
  }
}

/** core 失败轮「零正文兼容锁」文案（`turn_flow.rs` 的 `LEGACY_FAILED_TURN_TEXT`，逐字）。 */
const FAILED_TURN_EMPTY_WIRE_TEXT = "This turn failed.";
/** core 失败轮在已产出正文后追加的说明后缀（`turn_flow.rs` 的 `FAILED_PARTIAL_TEXT_SUFFIX`，含前导空行）。 */
const FAILED_TURN_WIRE_SUFFIX = "\n\n（该轮失败前已产出以上内容）";

/**
 * wave-64 §2 F2（契约修订 / L3-B BLOCKING 收口）：失败轮 wire `text` 归一化。
 *
 * 线上 core **恒发** `text`，但其中没有用户需要的新信息：
 * - 零正文 ⇒ 逐字 `"This turn failed."`（兼容锁）⇒ 返回 `null`：**不写正文**，错误由终态状态行承载；
 * - 有正文 ⇒ `正文 + "\n\n（该轮失败前已产出以上内容）"` ⇒ 剥掉后缀与尾部空行，只返回正文部分。
 *
 * 返回 `null` 时调用方**不得**写 `content`（保留已产出正文）；返回字符串时才写该正文。
 * 前台轮与后台会话轮（`runningSessionMap` / `updatePersistedBackgroundSession`）同等生效。
 */
function normalizeFailedTurnContent(text: string | null | undefined): string | null {
  if (typeof text !== "string") {
    return null;
  }
  if (text.trim() === FAILED_TURN_EMPTY_WIRE_TEXT) {
    return null;
  }
  if (text.endsWith(FAILED_TURN_WIRE_SUFFIX)) {
    const body = text.slice(0, text.length - FAILED_TURN_WIRE_SUFFIX.length).trimEnd();
    return body.trim().length > 0 ? body : null;
  }
  return text.trim().length > 0 ? text : null;
}

/** turn 事件订阅聚合（0 键 / 0 getters / 3 actions，事实图 §5） getters（键清单见 `types.ts` 的 `TurnEventsGetters` 与事实图 §2）。 */
export const turnEventsGetters = {} as TurnEventsGetters;

/** turn 事件订阅聚合（0 键 / 0 getters / 3 actions，事实图 §5） actions（键清单见 `types.ts` 的 `TurnEventsActions` 与事实图 §3）。 */
export const turnEventsActions = {
  shouldProcessTurnEvent(this: TurnEventsThis, 
    payload: Pick<TurnStreamEvent, "turnId" | "eventId" | "sequence" | "emittedAtMs" | "kind">
  ) {
    const isTerminalEvent =
      payload.kind === "completed" || payload.kind === "failed" || payload.kind === "cancelled" || payload.kind === "suspended";
    if (isTerminalEvent) {
      // 终态事件：历史模式下仍处理（更新消息状态并解锁），且不被
      // "同 sequence 不同 eventId"的去重规则误杀（output_end 与 completed
      // 常在同一 sequence 批次内连发，后者会被误判为重复而丢弃 → 永久卡死）。
      const accepted = shouldAcceptTurnEvent(this.eventCursorByTurnId[payload.turnId], payload, {
        allowSameSequenceDifferentEventId: true
      });
      debugLog("event:gate:terminal", {
        turnId: payload.turnId,
        kind: payload.kind,
        eventId: payload.eventId ?? null,
        sequence: payload.sequence ?? null,
        emittedAtMs: payload.emittedAtMs ?? null,
        accepted
      });
      // 无条件落库（低频）：终态事件链路是"状态卡死"诊断的核心依据。
      recordFrontendInstant("runtime.event-gate", "terminal", {
        turnId: payload.turnId,
        kind: payload.kind,
        eventId: payload.eventId ?? null,
        sequence: payload.sequence ?? null,
        emittedAtMs: payload.emittedAtMs ?? null,
        accepted,
        historical: isHistoricalMode(this.historyCursorMode)
      });
      return accepted;
    }
    if (isHistoricalMode(this.historyCursorMode)) {
      debugLog("event:gate:dropped-historical", {
        turnId: payload.turnId,
        kind: payload.kind,
        eventId: payload.eventId ?? null,
        sequence: payload.sequence ?? null
      });
      // 无条件落库（低频）：历史模式吞事件是"状态卡死"的另一根因。
      recordFrontendInstant("runtime.event-gate", "dropped-historical", {
        turnId: payload.turnId,
        kind: payload.kind,
        eventId: payload.eventId ?? null,
        sequence: payload.sequence ?? null,
        emittedAtMs: payload.emittedAtMs ?? null
      });
      return false;
    }
    const accepted = shouldAcceptTurnEvent(this.eventCursorByTurnId[payload.turnId], payload);
    debugLog("event:gate", {
      turnId: payload.turnId,
      kind: payload.kind,
      eventId: payload.eventId ?? null,
      sequence: payload.sequence ?? null,
      emittedAtMs: payload.emittedAtMs ?? null,
      accepted
    });
    // 无条件落库（低频）：仅记录被丢弃的事件，避免高频 delta 刷爆缓冲。
    if (!accepted) {
      recordFrontendInstant("runtime.event-gate", "dropped-dedup", {
        turnId: payload.turnId,
        kind: payload.kind,
        eventId: payload.eventId ?? null,
        sequence: payload.sequence ?? null,
        emittedAtMs: payload.emittedAtMs ?? null
      });
    }
    return accepted;
  },
  commitTurnEventCursor(this: TurnEventsThis, payload: Pick<TurnStreamEvent, "turnId" | "eventId" | "sequence" | "emittedAtMs">) {
    this.eventCursorByTurnId[payload.turnId] = {
      eventId: payload.eventId?.trim() || null,
      sequence: typeof payload.sequence === "number" && Number.isFinite(payload.sequence) ? payload.sequence : null,
      emittedAtMs:
        typeof payload.emittedAtMs === "number" && Number.isFinite(payload.emittedAtMs)
          ? payload.emittedAtMs
          : null
    };
  },
  async registerTurnEventListeners(this: TurnEventsThis, registry: TurnEventRegistry) {
    debugLog("events:init");

    const isEventForCurrentSession = (payload: TurnStreamEvent) => {
      // F5a：严格 fail-closed —— sessionId 缺失/为空一律丢弃，不再「缺省放行」。
      // 依据：每个活链路事件都必须携带 sessionId（Rust 侧 turn_stream 用
      // `input.session_id` 下发），缺失即说明事件归属不可判定，宁可丢弃。
      return (
        typeof payload.sessionId === "string" &&
        payload.sessionId.length > 0 &&
        payload.sessionId === this.sessionId
      );
    };

    // F5b（A 级）：**边等边登记** —— 每个句柄一注册成功就立刻进注册表，
    // 任一次 `safeListen` reject 时由 `initializeTurnEvents` 的回滚路径统一 unlisten，
    // 杜绝「前 N-1 个句柄随局部变量栈帧丢失 ⇒ 永久泄漏 + 重试后 delta 双写」。
    const listen = async (
      eventName: string,
      handler: (payload: TurnStreamEvent) => void
    ): Promise<void> => {
      const unlisten = await safeListen<TurnStreamEvent>(eventName, ({ payload }) => handler(payload));
      registry.unlisteners.push(unlisten);
    };

    // 桌面端上游重试进度：`turn:retry` 的负载不是 `TurnStreamEvent`（不参与回合状态机），
    // 故单独注册；句柄同样进 registry，随 `releaseTurnEventRegistry` 统一 unlisten。
    // 事件由 `src-tauri/src/tauri_adapter.rs` 的回合阻塞线程发出（真值 = retry::RetryNotice）。
    const unlistenTurnRetry = await safeListen<TurnRetryEvent>("turn:retry", ({ payload }) => {
      this.applyTurnRetryProgress(payload);
    });
    registry.unlisteners.push(unlistenTurnRetry);

    await listen("turn:started", (payload) => {
      // Wave-59 F4-5 / D-59-9：retry 占位清零**不得**依赖 turnId 门禁。
      // 生产路径里本地 requestId 与后端回传 turnId 可能不同（submitTurn 用本地 id 建消息，
      // 后端事件带自己的 turnId），若把清零放在 `activeTurnId !== payload.turnId` 之后，
      // 该轮永远无法退出「正在重试」。这里只做会话归属判定（不同会话的事件不得动本会话状态）。
      if (isEventForCurrentSession(payload)) {
        this.clearTimeoutRetryPlaceholder();
      }
      if (!isEventForCurrentSession(payload)) {
        return;
      }
      if (this.activeTurnId !== payload.turnId) {
        return;
      }
      if (!this.shouldProcessTurnEvent(payload)) {
        return;
      }
      this.commitTurnEventCursor(payload);
      this.rearmSubmissionWatchdog(payload.turnId);
      this.cancelStreamFlush();
      this.streamBufferTurnId = null;
      this.streamBufferText = "";
      this.streamBufferReasoning = "";
      // Wave-59 F4 / D-59-3：新一轮（含工具后的新 hop）起点 ⇒ 本 hop 尚无输出。
      this.activeStreamKind = null;
      this.resetStreamDebugMetrics();
      this.clearPendingTraceTimeline();

      this.ensureAssistantMessage(
        payload.turnId,
        buildAssistantModelLabel(payload.providerName, payload.providerModel)
      );
      this.phase = resolveRuntimePhaseFromEvent(payload, "calling_model");
      const assistantMessage = this.ensureAssistantMessage(
        payload.turnId,
        buildAssistantModelLabel(payload.providerName, payload.providerModel)
      );
      if (assistantMessage.content === TIMEOUT_RETRY_PENDING_MESSAGE) {
        assistantMessage.content = "";
      }
      assistantMessage.errorDetail = null;
      debugLog("event:started", {
        turnId: payload.turnId
      });
      this.providerRequestedName = payload.providerRequestedName ?? this.providerRequestedName;
      this.providerName = payload.providerName ?? this.providerName;
      this.providerProtocol = payload.providerProtocol ?? this.providerProtocol;
      this.providerModel = payload.providerModel ?? this.providerModel;
      this.providerSource = payload.providerSource ?? this.providerSource;
      this.providerMode = payload.providerMode ?? this.providerMode;
      this.fallbackReason = payload.fallbackReason ?? this.fallbackReason;
      this.inputTokens = payload.inputTokens ?? this.inputTokens;
      this.outputTokens = payload.outputTokens ?? this.outputTokens;
      this.totalTokens = payload.totalTokens ?? this.totalTokens;
      this.firstTokenLatencyMs = payload.firstTokenLatencyMs ?? this.firstTokenLatencyMs;
      // Wave-15 R4：新轮开始 → 复位上一轮残留的 per-call 记录（事件携带记录时以事件为准）。
      this.applyActiveProviderCallRecords(payload.providerCallRecords, true);
      // wave-metrics（task-4）：新轮/新 hop 起点 → 清空上一轮残留快照（与 per-call 记录同一复位点），
      // 本轮快照随 hop_complete / output_end / phase_changed 等事件重新落库。
      // 若本条事件自带 metrics（协议上不会，但防御 wire 演变）则先清后写，不做静默丢弃。
      this.liveMetrics = null;
      this.applyMetricsSnapshot(payload.metrics);
      this.traceSteps = payload.traceSteps ?? this.traceSteps;
      this.publishTraceTimeline(resolveEventTraceTimeline(payload, () =>
        buildFallbackRuntimeTraceTimeline({
          turnId: payload.turnId,
          eventType: payload.eventType,
          messages: this.messages,
          phase: payload.phase ?? this.phase,
          buildContextObservation: cloneBuildContextObservation(payload.buildContextObservation),
          providerPatch: {
            providerName: payload.providerName ?? this.providerName,
            providerProtocol: payload.providerProtocol ?? this.providerProtocol,
            providerModel: payload.providerModel ?? this.providerModel,
            providerSource: payload.providerSource ?? this.providerSource,
            providerMode: payload.providerMode ?? this.providerMode
          }
        })
      ));
      this.toolActivities = payload.toolActivities ?? this.toolActivities;
      this.syncToolMessages(payload.turnId, payload.toolActivities, false);
      this.updateActiveTraceTimeline(this.traceTimeline, true);

      // Wave-4 C2：新轮开始时立即落盘 traceId 到 turnTraceHistory，保证 live turn 期可读
      if (payload.traceId) {
        this.upsertTurnTrace(payload.turnId, { traceId: payload.traceId });
      }
    });

    await listen("turn:delta", (payload) => {
      if (this.activeTurnId !== payload.turnId || !isEventForCurrentSession(payload)) {
        if (payload.sessionId) {
          const bg = this.runningSessionMap[payload.sessionId];
          if (bg && bg.turnId === payload.turnId) {
            const dt = payload.text ?? "";
            const dr = payload.reasoningContent ?? "";
            if (dt) bg.textBuffer += dt;
            if (dr) bg.reasoningBuffer += dr;
            if (bg.textBuffer.length > MAX_BG_TEXT_BUFFER_CHARS) {
              bg.textBuffer = bg.textBuffer.slice(0, MAX_BG_TEXT_BUFFER_CHARS);
            }
            if (bg.reasoningBuffer.length > MAX_BG_REASONING_BUFFER_CHARS) {
              bg.reasoningBuffer = bg.reasoningBuffer.slice(0, MAX_BG_REASONING_BUFFER_CHARS);
            }
          }
        }
        return;
      }
      if (!this.shouldProcessTurnEvent(payload)) {
        return;
      }
      this.commitTurnEventCursor(payload);
      this.rearmSubmissionWatchdog(payload.turnId);

      const deltaText = payload.text ?? "";
      const deltaReasoning = payload.reasoningContent ?? "";

      // Wave-59 F4-5 / RC9：任何真实流式增量到达 ⇒ 「超时重连中...」占位作废（retrying 清零）。
      // 不按 turnId 过滤：占位消息的 turnId 可能仍是本地 requestId（与后端事件 turnId 不同）。
      if (deltaText || deltaReasoning) {
        this.clearTimeoutRetryPlaceholder();
      }

      const assistantMessage = this.ensureAssistantMessage(
        payload.turnId,
        buildAssistantModelLabel(this.providerName, this.providerModel)
      );
      if (deltaText && assistantMessage.content === TIMEOUT_RETRY_PENDING_MESSAGE) {
        assistantMessage.content = "";
      }
      if (deltaText || deltaReasoning) {
        assistantMessage.errorDetail = null;
      }

      if (deltaText || deltaReasoning) {
        this.streamDebugDeltaCount += 1;
        this.streamDebugTextCharsReceived += deltaText.length;
      }

      if (deltaReasoning) {
        this.streamBufferReasoning += deltaReasoning;
      }

      if (deltaText) {
        this.streamBufferText += deltaText;
      }

      // Wave-59 F4 / D-59-3：记录本 hop 当前流式类型。
      // 正文到达即锁定 "content"（同 hop 内后续 reasoning delta 不得把状态退回「思考中」）；
      // 仅有 reasoning 时记 "reasoning" ⇒ 状态条「思考中」；正文一到立刻切「正在回复」。
      if (deltaText) {
        this.activeStreamKind = "content";
      } else if (deltaReasoning && this.activeStreamKind !== "content") {
        this.activeStreamKind = "reasoning";
      }

      this.phase = resolveRuntimePhaseFromEvent(payload, this.phase);
      this.firstTokenLatencyMs = payload.firstTokenLatencyMs ?? this.firstTokenLatencyMs;
      if (payload.traceTimeline?.length) {
        // 节流应用：trace 可观测性数据允许滞后，避免每个 chunk 全量克隆
        this.scheduleThrottledTraceTimeline(payload.traceTimeline);
      }
      if (!deltaText && !deltaReasoning) {
        return;
      }
      this.scheduleStreamFlush(payload.turnId);
      debugLog("event:delta", {
        turnId: payload.turnId,
        deltaLength: deltaText.length,
        reasoningLength: deltaReasoning.length
      });
    });

    await listen("turn:trace", (payload) => {
      if (!isEventForCurrentSession(payload)) {
        return;
      }
      if (this.activeTurnId !== payload.turnId) {
        return;
      }
      if (!this.shouldProcessTurnEvent(payload)) {
        return;
      }
      this.commitTurnEventCursor(payload);
      this.rearmSubmissionWatchdog(payload.turnId);
      this.flushBufferedStreamText(payload.turnId);
      // 低频语义事件：先冲刷节流挂起的 timeline，保证基于最新 timeline 推导
      this.flushPendingTraceTimeline();

      this.phase = resolveRuntimePhaseFromEvent(payload, this.phase);
      this.traceSteps = payload.traceSteps ?? this.traceSteps;
      const traceTimeline = resolveSemanticEventTraceTimeline(payload, () =>
        buildFallbackRuntimeTraceTimeline({
          turnId: payload.turnId,
          eventType: payload.eventType,
          messages: this.messages,
          phase: payload.phase,
          assistantMessage: this.messages.find((message) => message.turnId === payload.turnId && message.role === "assistant") ?? null,
          toolActivities: this.toolActivities,
          providerPatch: {
            providerName: this.providerName,
            providerProtocol: this.providerProtocol,
            providerModel: this.providerModel,
            providerSource: this.providerSource,
            providerMode: this.providerMode
          },
          firstTokenLatencyMs: this.firstTokenLatencyMs
        }),
        this.traceTimeline
      );
      this.updateActiveTraceTimeline(traceTimeline, true);
      this.updateActiveModelTraceFromAssistant(payload.turnId);
      debugLog("event:trace", {
        turnId: payload.turnId,
        steps: this.traceSteps.length
      });
    });

    await listen("turn:phase_changed", (payload) => {
      if (!isEventForCurrentSession(payload)) {
        return;
      }
      // wave-metrics（task-4）：快照必须早于任何 turnId/去重门禁落库 ——
      // 该轮指标不能因后续早退分支被吞掉（`null`/缺键 ⇒ 保持不变，I7）。
      this.applyMetricsSnapshot(payload.metrics);
      if (this.activeTurnId !== payload.turnId) {
        return;
      }
      if (!this.shouldProcessTurnEvent(payload)) {
        return;
      }
      this.commitTurnEventCursor(payload);
      this.rearmSubmissionWatchdog(payload.turnId);
      this.flushBufferedStreamText(payload.turnId);
      this.flushPendingTraceTimeline();
      // Wave-15 R4：携带 per-call 记录的事件（wire 实况：phase_changed 携带全量 3 条）刷新活跃轮记录。
      this.applyActiveProviderCallRecords(payload.providerCallRecords);

      this.phase = resolveRuntimePhaseFromEvent(payload, this.phase);
      this.traceSteps = payload.traceSteps ?? this.traceSteps;
      const traceTimeline = resolveSemanticEventTraceTimeline(payload, () =>
        buildFallbackRuntimeTraceTimeline({
          turnId: payload.turnId,
          eventType: payload.eventType,
          messages: this.messages,
          phase: payload.phase,
          assistantMessage: this.messages.find((message) => message.turnId === payload.turnId && message.role === "assistant") ?? null,
          toolActivities: this.toolActivities,
          providerPatch: {
            providerName: this.providerName,
            providerProtocol: this.providerProtocol,
            providerModel: this.providerModel,
            providerSource: this.providerSource,
            providerMode: this.providerMode
          },
          firstTokenLatencyMs: this.firstTokenLatencyMs
        }),
        this.traceTimeline
      );
      this.updateActiveTraceTimeline(traceTimeline, true);
      this.updateActiveModelTraceFromAssistant(payload.turnId);
    });

    await listen("turn:checkpoint_persisted", (payload) => {
      if (!isEventForCurrentSession(payload)) {
        return;
      }
      // wave-metrics（task-4）：同 phase_changed —— 会话归属判定后立即落库，早于 turnId 门禁。
      this.applyMetricsSnapshot(payload.metrics);
      if (this.activeTurnId !== payload.turnId) {
        return;
      }
      if (!this.shouldProcessTurnEvent(payload)) {
        return;
      }
      this.commitTurnEventCursor(payload);
      this.rearmSubmissionWatchdog(payload.turnId);
      this.flushBufferedStreamText(payload.turnId);
      this.flushPendingTraceTimeline();
      // Wave-15 R4：携带 per-call 记录的事件刷新活跃轮记录。
      this.applyActiveProviderCallRecords(payload.providerCallRecords);

      this.phase = resolveRuntimePhaseFromEvent(payload, this.phase);
      this.traceSteps = payload.traceSteps ?? this.traceSteps;
      const traceTimeline = resolveSemanticEventTraceTimeline(payload, () =>
        buildFallbackRuntimeTraceTimeline({
          turnId: payload.turnId,
          eventType: payload.eventType,
          messages: this.messages,
          phase: payload.phase,
          assistantMessage: this.messages.find((message) => message.turnId === payload.turnId && message.role === "assistant") ?? null,
          toolActivities: payload.toolActivities ?? this.toolActivities,
          providerPatch: {
            providerName: this.providerName,
            providerProtocol: this.providerProtocol,
            providerModel: this.providerModel,
            providerSource: this.providerSource,
            providerMode: this.providerMode
          },
          firstTokenLatencyMs: this.firstTokenLatencyMs
        }),
        this.traceTimeline
      );
      this.updateActiveTraceTimeline(traceTimeline, true);
      this.updateActiveModelTraceFromAssistant(payload.turnId);
    });

    await listen("turn:tool", (payload) => {
      if (!isEventForCurrentSession(payload)) {
        return;
      }
      if (this.activeTurnId !== payload.turnId) {
        return;
      }
      if (!this.shouldProcessTurnEvent(payload)) {
        return;
      }
      this.commitTurnEventCursor(payload);
      this.rearmSubmissionWatchdog(payload.turnId);
      this.flushBufferedStreamText(payload.turnId);
      this.flushPendingTraceTimeline();

      // Wave-59 F4 / D-59-3：工具事件即 hop 边界 ⇒ 复位本 hop 流式类型
      // （工具后的新 hop 在首个 delta 前必须回到「思考中」而不是「正在回复」，RC7/RC8）。
      this.activeStreamKind = null;

      this.phase = resolveRuntimePhaseFromEvent(payload, this.phase);
      debugLog("event:tool", {
        turnId: payload.turnId,
        tools: (payload.toolActivities ?? []).length
      });
      this.toolActivities = payload.toolActivities ?? this.toolActivities;
      this.syncToolMessages(payload.turnId, payload.toolActivities, false);
      // tool 事件需要 timeline 结构演进（新增/更新 call_tool entry）：
      // payload 无 timeline 时基于当前 toolActivities 重建，并保留已有 build_context observation。
      const existingContextObservation = this.traceTimeline.find(
        (entry) => canonicalizeTraceTimelineKind(entry.kind) === "build_context"
      )?.buildContextObservation ?? null;
      const traceTimeline = resolveEventTraceTimeline(payload, () =>
        buildFallbackRuntimeTraceTimeline({
          turnId: payload.turnId,
          eventType: payload.eventType,
          messages: this.messages,
          phase: payload.phase ?? this.phase,
          assistantMessage: this.messages.find((message) => message.turnId === payload.turnId && message.role === "assistant") ?? null,
          toolActivities: payload.toolActivities ?? this.toolActivities,
          buildContextObservation: existingContextObservation,
          providerPatch: {
            providerName: this.providerName,
            providerProtocol: this.providerProtocol,
            providerModel: this.providerModel,
            providerSource: this.providerSource,
            providerMode: this.providerMode
          },
          firstTokenLatencyMs: this.firstTokenLatencyMs
        })
      );
      this.updateActiveTraceTimeline(traceTimeline, true);
      this.updateActiveModelTraceFromAssistant(payload.turnId);
    });

    await listen("turn:output_end", (payload) => {
      // Wave-59 F4-5 / D-59-9：retry 占位清零**不得**依赖 turnId 门禁。
      // 生产路径里本地 requestId 与后端回传 turnId 可能不同（submitTurn 用本地 id 建消息，
      // 后端事件带自己的 turnId），若把清零放在 `activeTurnId !== payload.turnId` 之后，
      // 该轮永远无法退出「正在重试」。这里只做会话归属判定（不同会话的事件不得动本会话状态）。
      if (isEventForCurrentSession(payload)) {
        this.clearTimeoutRetryPlaceholder();
        // wave-metrics（task-4）：会话归属判定后立即落库 —— 后续 background / turnId /
        // 去重门禁都会提前 return，快照不得被这些分支吞掉（`null`/缺键 ⇒ 保持不变，I7）。
        this.applyMetricsSnapshot(payload.metrics);
      }
      if (payload.sessionId) {
        const bg = this.runningSessionMap[payload.sessionId];
        if (bg && bg.turnId === payload.turnId) {
          bg.textBuffer = "";
          bg.reasoningBuffer = "";
          this.updatePersistedBackgroundSession(payload.sessionId, payload.turnId, {
            content: payload.text ?? null,
            reasoningContent: payload.reasoningContent ?? null,
            status: "done",
            modelName: buildAssistantModelLabel(payload.providerName, payload.providerModel),
            tokenCount: payload.outputTokens ?? null,
            phase: resolveRuntimePhaseFromEvent(payload, "completed"),
            providerRequestedName: payload.providerRequestedName ?? null,
            providerName: payload.providerName ?? null,
            providerProtocol: payload.providerProtocol ?? null,
            providerModel: payload.providerModel ?? null,
            providerSource: payload.providerSource ?? null,
            providerMode: payload.providerMode ?? null,
            fallbackReason: payload.fallbackReason ?? undefined,
            inputTokens: payload.inputTokens ?? null,
            outputTokens: payload.outputTokens ?? null,
            totalTokens: payload.totalTokens ?? null,
            firstTokenLatencyMs: payload.firstTokenLatencyMs ?? null
          });
          return;
        }
      }
      if (this.activeTurnId !== payload.turnId) {
        return;
      }
      if (!this.shouldProcessTurnEvent(payload)) {
        return;
      }
      this.commitTurnEventCursor(payload);
      this.rearmSubmissionWatchdog(payload.turnId);
      this.applyOutputEnd(payload);
      debugLog("event:output_end", {
        turnId: payload.turnId,
        finalTextLength: payload.text?.length ?? 0,
        outputTokens: payload.outputTokens ?? null,
        turnDurationMs: payload.turnDurationMs ?? null
      });
    });

    await listen("turn:hop_complete", (payload) => {
      // wave-metrics（task-4）：快照先落库，早于 turnId / 去重门禁（该轮指标不得被早退吞掉）。
      // 会话归属仍严格 fail-closed（缺 sessionId 的事件不得写本会话状态）。
      if (isEventForCurrentSession(payload)) {
        this.applyMetricsSnapshot(payload.metrics);
      }
      if (this.activeTurnId !== payload.turnId) {
        return;
      }
      if (!this.shouldProcessTurnEvent(payload)) {
        return;
      }
      this.commitTurnEventCursor(payload);
      this.rearmSubmissionWatchdog(payload.turnId);
      this.flushBufferedStreamText(payload.turnId);
      // Wave-59 F4 / D-59-3：hop 完成 ⇒ 复位本 hop 流式类型（下一 hop 从「思考中」起算）。
      this.activeStreamKind = null;
      // R4/Wave-15：hop 完成即产出该次调用的 per-call 值（不等 turn:completed）；无记录时保持既有值。
      this.applyActiveProviderCallRecords(payload.providerCallRecords);
      this.totalTokens = payload.totalTokens ?? this.totalTokens;
      this.firstTokenLatencyMs = payload.firstTokenLatencyMs ?? this.firstTokenLatencyMs;
      debugLog("event:hop_complete", {
        turnId: payload.turnId,
        inputTokens: payload.inputTokens ?? null,
        outputTokens: payload.outputTokens ?? null,
        providerCallRecordCount: payload.providerCallRecords?.length ?? 0
      });
    });

    await listen("turn:completed", (payload) => {
      // Wave-59 F4-5 / D-59-9：retry 占位清零**不得**依赖 turnId 门禁。
      // 生产路径里本地 requestId 与后端回传 turnId 可能不同（submitTurn 用本地 id 建消息，
      // 后端事件带自己的 turnId），若把清零放在 `activeTurnId !== payload.turnId` 之后，
      // 该轮永远无法退出「正在重试」。这里只做会话归属判定（不同会话的事件不得动本会话状态）。
      if (isEventForCurrentSession(payload)) {
        this.clearTimeoutRetryPlaceholder();
        // wave-metrics（task-4）：终态快照必须落库 —— 后续 background / 历史模式 /
        // turnId / 去重门禁都会提前 return（`null`/缺键 ⇒ 保持不变，I7）。
        this.applyMetricsSnapshot(payload.metrics);
      }
      // Wave-1 C6：turn 结束（含后台会话 turn）即刷新计划/目标面板；
      // 同一 turnId 只刷新一次，失败不影响主流程。
      refreshPlanGoalAfterTurn(payload.turnId, payload.sessionId ?? this.sessionId);
      // Detect terminal event for a background session's turn
      if (payload.sessionId) {
        const bg = this.runningSessionMap[payload.sessionId];
        if (bg && bg.turnId === payload.turnId) {
          this.updatePersistedBackgroundSession(payload.sessionId, payload.turnId, {
            content: payload.text ?? null,
            reasoningContent: payload.reasoningContent ?? null,
            status: "done",
            modelName: buildAssistantModelLabel(payload.providerName, payload.providerModel),
            tokenCount: payload.outputTokens ?? null,
            phase: "ready",
            sessionSummary: payload.sessionSummary ?? null,
            providerRequestedName: payload.providerRequestedName ?? null,
            providerName: payload.providerName ?? null,
            providerProtocol: payload.providerProtocol ?? null,
            providerModel: payload.providerModel ?? null,
            providerSource: payload.providerSource ?? null,
            providerMode: payload.providerMode ?? null,
            fallbackReason: payload.fallbackReason ?? undefined,
            inputTokens: payload.inputTokens ?? null,
            outputTokens: payload.outputTokens ?? null,
            totalTokens: payload.totalTokens ?? null,
            firstTokenLatencyMs: payload.firstTokenLatencyMs ?? null
          });
          this.completedSessionSet[payload.sessionId] = true;
          delete this.runningSessionMap[payload.sessionId];
          return;
        }
      }
      if (this.activeTurnId !== payload.turnId || !isEventForCurrentSession(payload)) {
        return;
      }
      // Wave-59 B-59-1：终态必须无条件复位 hop 级流式类型。
      // 末次模型调用按协议不保证发 turn:hop_complete，若只在 turn:started/hop_complete 复位，
      // 本轮结束会残留 "content"，使下一轮 awaiting-first-signal 窗口被误判为「正文流式」
      // 而隐藏状态条（F1-2 / F4-2 语义失效）。
      this.activeStreamKind = null;
      if (!this.shouldProcessTurnEvent(payload)) {
        return;
      }

      // 历史模式下终态事件：仅解锁运行态，不污染历史视图（防止状态永久卡死）
      if (isHistoricalMode(this.historyCursorMode)) {
        this.clearSubmissionWatchdog();
        this.$patch((state) => {
          state.isSubmitting = false;
          state.activeTurnId = null;
          state.phase = "ready";
        });
        return;
      }

      // ===== STAGE 1 (sync): Critical chat area mutations only =====
      this.commitTurnEventCursor(payload);
      this.flushBufferedStreamText(payload.turnId);
      // terminal 事件：先冲刷节流挂起的 timeline，保证最终态完整
      this.flushPendingTraceTimeline();
      // Wave-15 R4：终态事件（wire 实况携带全量 per-call 记录）刷新活跃轮记录。
      this.applyActiveProviderCallRecords(payload.providerCallRecords);

      const completedPayloadRecord = payload as Record<string, unknown>;

      const assistantMessage = this.ensureAssistantMessage(
        payload.turnId,
        buildAssistantModelLabel(payload.providerName, payload.providerModel)
      );

      const finalText = payload.text?.trim();
      if (finalText && payload.text !== assistantMessage.content) {
        assistantMessage.content = payload.text ?? assistantMessage.content;
      } else if (assistantMessage.content === TIMEOUT_RETRY_PENDING_MESSAGE) {
        assistantMessage.content = payload.text ?? "";
      }
      assistantMessage.errorDetail = null;
      const nextReasoningContent = normalizeReasoningContent(
        payload.reasoningContent ?? assistantMessage.reasoningContent ?? null
      );
      if (nextReasoningContent !== assistantMessage.reasoningContent) {
        assistantMessage.reasoningContent = nextReasoningContent;
      }
      assistantMessage.status = "done";
      assistantMessage.modelName = buildAssistantModelLabel(payload.providerName, payload.providerModel);

      // Pre-compute values for later stages (captured by closure)
      const completedPhase = resolveRuntimePhaseFromEvent(payload, "completed");
      const terminalToolActivities = resolveTerminalToolActivities(payload.toolActivities, this.toolActivities);
      logCacheTelemetryContractViolations("completed", payload);
      const cacheHitInputTokens = resolveProviderReturnedCacheHitInputTokens(payload);
      const reasoningTokens = resolveReasoningTokens(payload);
      const cacheHitInputTokenPatch = cacheHitInputTokens != null ? { cacheHitInputTokens } : {};
      const reasoningTokenPatch = reasoningTokens != null ? { reasoningTokens } : {};
      const turnDurationPatch = payload.turnDurationMs != null ? { turnDurationMs: payload.turnDurationMs } : {};
      if (isDebugLoggingEnabled()) {
        debugLog("cache-telemetry:terminal-payload", {
          terminalEvent: "completed",
          ...buildCacheTelemetryDebugSnapshot(payload)
        });
      }
      const completedSessionId = this.sessionId;
      const completedRunId = this.activeRunId;
      const completedNodeId = this.visibleNodeId;

      // ===== STAGE 1b (sync): lightweight UI unlock =====
      // 立即解锁 isSubmitting/activeTurnId/phase，让用户马上可以发起新一轮对话；
      // trace 终态投影（深拷贝 + upsert）推迟到 STAGE 2 的宏任务中执行，不再阻塞解锁。
      const unlockedPhase = completedPhase === "completed" ? "ready" : completedPhase;
      this.clearSubmissionWatchdog();
      this.$patch((state) => {
        state.isSubmitting = false;
        state.activeTurnId = null;
        state.phase = unlockedPhase;
      });

      // Yield to browser — let Vue flush reactivity + DOM for chat area
      // Yield to browser — let Vue flush reactivity + DOM for chat area.
      // PA-089 优化：STAGE 2 改 runLowPriorityTurnWork（requestIdleCallback 空闲执行）——
      // completed 后不再用 setTimeout(0) 抢占主线程做 trace 深拷贝（实测 500ms+ 卡顿源）。
      runLowPriorityTurnWork(() => {
        if (
          this.sessionId !== completedSessionId ||
          isHistoricalMode(this.historyCursorMode) ||
          // 该 turn 已被清出消息流（新建会话/撤回/切换历史）时，跳过终态投影。
          // 不能依赖 activeTurnId：STAGE 1b 已解锁，新 turn 可能已经开始。
          !this.messages.some((message) => message.turnId === payload.turnId)
        ) {
          return;
        }

        // ===== STAGE 2 (setTimeout 0): terminal trace projection =====
        // 全局展示字段（provider/tokens/sessionSummary）仅在"没有新 turn 抢占前台"时写入：
        // activeTurnId 为 null（刚解锁空闲）或仍是本 turn 时安全；另一个新 turn 已开始则跳过，
        // 避免旧 turn 的终态值覆盖新 turn 的提交状态。
        const stillOwnsGlobalState = this.activeTurnId == null || this.activeTurnId === payload.turnId;
        const nextTraceSteps = payload.traceSteps ?? this.traceSteps;
        const nextSessionSummary = payload.sessionSummary ?? this.sessionSummary;
        const nextProviderRequestedName = payload.providerRequestedName ?? this.providerRequestedName;
        const nextProviderName = payload.providerName ?? this.providerName;
        const nextProviderProtocol = payload.providerProtocol ?? this.providerProtocol;
        const nextProviderModel = payload.providerModel ?? this.providerModel;
        const nextProviderSource = payload.providerSource ?? this.providerSource;
        const nextProviderMode = payload.providerMode ?? this.providerMode;
        const nextInputTokens = payload.inputTokens ?? this.inputTokens;
        const nextOutputTokens = payload.outputTokens ?? this.outputTokens;
        const nextTotalTokens = payload.totalTokens ?? this.totalTokens;
        const nextFirstTokenLatencyMs = payload.firstTokenLatencyMs ?? this.firstTokenLatencyMs;

        // Pre-compute trace timeline (deep clone, unavoidable but done before $patch)
        const traceTimeline = resolveEventTraceTimeline(payload, () =>
          buildFallbackRuntimeTraceTimeline({
            turnId: payload.turnId,
            eventType: payload.eventType,
            messages: this.messages,
            phase: "completed",
            assistantMessage,
            toolActivities: terminalToolActivities,
            providerPatch: {
              providerName: nextProviderName,
              providerProtocol: nextProviderProtocol,
              providerModel: nextProviderModel,
              providerSource: nextProviderSource,
              providerMode: nextProviderMode
            },
            terminalState: "completed",
            fallbackReason: payload.fallbackReason ?? null,
            inputTokens: nextInputTokens,
            cacheHitInputTokens,
            reasoningTokens,
            outputTokens: nextOutputTokens,
            totalTokens: nextTotalTokens,
            firstTokenLatencyMs: nextFirstTokenLatencyMs,
            turnDurationMs: payload.turnDurationMs ?? null
          })
        );

        // Pre-compute tool message patches for inline syncToolMessages
        const completedActiveTools = terminalToolActivities.filter((tool) => tool.status !== "planned");
        const completedToolPatches = completedActiveTools.map((tool) => ({
          id: `tool-${payload.turnId}-${tool.id}`,
          turnId: payload.turnId,
          role: "tool" as const,
          content: tool.resultText ?? "",
          status: toolStatusToMessageStatus(tool.status),
          toolName: tool.name,
          canonicalToolName: tool.canonicalToolName ?? null,
          displayNameZh: tool.displayNameZh ?? null,
          detail: buildToolMessageDetail(tool),
          durationSeconds: tool.durationSeconds ?? null
        }));

        // Pre-compute trace record patch
        const completedTraceRecordPatch: Partial<TurnTraceRecord> & { updatedAt?: number } = {
          eventId: payload.eventId ?? null,
          eventType: payload.eventType ?? null,
          eventVersion: payload.eventVersion ?? null,
          sequence: payload.sequence ?? null,
          emittedAtMs: payload.emittedAtMs ?? null,
          phase: "completed",
          traceSteps: nextTraceSteps,
          ...(payload.traceId ? { traceId: payload.traceId } : {}),
          toolActivities: terminalToolActivities,
          providerCallRecords: cloneProviderCallRecords(payload.providerCallRecords),
          providerRequestedName: nextProviderRequestedName,
          providerName: nextProviderName,
          providerProtocol: nextProviderProtocol,
          providerModel: nextProviderModel,
          providerSource: nextProviderSource,
          providerMode: nextProviderMode,
          buildContextObservation: cloneBuildContextObservation(payload.buildContextObservation),
          hookTraceRecords: cloneHookTraceRecords(payload.hookTraceRecords),
          sessionSummary: nextSessionSummary,
          fallbackReason: payload.fallbackReason ?? null,
          inputTokens: nextInputTokens,
          outputTokens: nextOutputTokens,
          totalTokens: nextTotalTokens,
          firstTokenLatencyMs: nextFirstTokenLatencyMs,
          ...cacheHitInputTokenPatch,
          ...reasoningTokenPatch,
          ...turnDurationPatch,
          ...(payload.costUsd !== undefined ? { costUsd: payload.costUsd } : {}),
          error: null
        };

        // Single batch: terminal trace projection — only ONE reactive cycle.
        // UI 解锁（isSubmitting/activeTurnId/phase）已在 STAGE 1b 完成，
        // 此处不再触碰这些字段，避免覆盖可能已开始的新 turn 状态。
        this.$patch((state) => {
          // --- applyTurnTokenStats inline ---
          if (payload.inputTokens != null || payload.outputTokens != null) {
            for (let i = 0; i < state.messages.length; i++) {
              const msg = state.messages[i];
              if (msg.turnId !== payload.turnId) continue;
              if (msg.role === "user" && payload.inputTokens != null) {
                state.messages[i] = { ...msg, tokenCount: payload.inputTokens };
              } else if (msg.role === "assistant" && payload.outputTokens != null) {
                state.messages[i] = { ...msg, tokenCount: payload.outputTokens };
              }
            }
          }

          // --- syncToolMessages inline ---
          if (completedToolPatches.length > 0) {
            const existingIds = new Set<string>();
            for (let i = 0; i < state.messages.length; i++) {
              const msg = state.messages[i];
              if (msg.role === "tool" && msg.turnId === payload.turnId) {
                existingIds.add(msg.id);
              }
            }
            const pendingMessages: ChatMessage[] = [];
            for (const patch of completedToolPatches) {
              if (existingIds.has(patch.id)) {
                const idx = state.messages.findIndex((m) => m.id === patch.id);
                if (idx >= 0) {
                  state.messages[idx] = {
                    ...state.messages[idx],
                    content: patch.content,
                    status: patch.status,
                    toolName: patch.toolName,
                    canonicalToolName: patch.canonicalToolName,
                    displayNameZh: patch.displayNameZh,
                    detail: patch.detail,
                    durationSeconds: patch.durationSeconds
                  };
                }
              } else {
                pendingMessages.push({
                  id: patch.id,
                  turnId: patch.turnId,
                  role: "tool",
                  content: patch.content,
                  status: patch.status,
                  toolName: patch.toolName,
                  canonicalToolName: patch.canonicalToolName,
                  displayNameZh: patch.displayNameZh,
                  detail: patch.detail,
                  durationSeconds: patch.durationSeconds
                });
              }
            }
            if (pendingMessages.length > 0) {
              state.messages.push(...pendingMessages);
            }
          }

          // --- Trace timeline ---
          state.traceTimeline = traceTimeline;

          // --- Upsert turn trace inline ---
          const existingIdx = state.turnTraceHistory.findIndex((t) => t.turnId === payload.turnId);
          const traceUpdatedAt = Date.now();
          if (existingIdx >= 0) {
            state.turnTraceHistory[existingIdx] = {
              ...state.turnTraceHistory[existingIdx],
              ...completedTraceRecordPatch,
              traceTimeline: traceTimeline,
              updatedAt: traceUpdatedAt
            };
          } else {
            state.turnTraceHistory.push({
              turnId: payload.turnId,
              title: buildTurnTraceTitleFromMessages(state.messages, payload.turnId),
              phase: "completed",
              traceSteps: nextTraceSteps,
              traceTimeline: traceTimeline,
              toolActivities: terminalToolActivities,
              // 完整展开终态 patch（event envelope / provider / tokens / 深拷贝记录），
              // 覆盖"提交后未建 trace 记录即收到 completed"的异常路径。
              providerCallRecords: cloneProviderCallRecords(payload.providerCallRecords),
              hookTraceRecords: cloneHookTraceRecords(payload.hookTraceRecords),
              buildContextObservation: cloneBuildContextObservation(payload.buildContextObservation),
              sessionSummary: nextSessionSummary,
              eventId: completedTraceRecordPatch.eventId ?? null,
              eventType: completedTraceRecordPatch.eventType ?? null,
              eventVersion: completedTraceRecordPatch.eventVersion ?? null,
              sequence: completedTraceRecordPatch.sequence ?? null,
              emittedAtMs: completedTraceRecordPatch.emittedAtMs ?? null,
              providerRequestedName: completedTraceRecordPatch.providerRequestedName ?? null,
              providerName: completedTraceRecordPatch.providerName ?? null,
              providerProtocol: completedTraceRecordPatch.providerProtocol ?? null,
              providerModel: completedTraceRecordPatch.providerModel ?? null,
              providerSource: completedTraceRecordPatch.providerSource ?? null,
              providerMode: completedTraceRecordPatch.providerMode ?? null,
              fallbackReason: completedTraceRecordPatch.fallbackReason ?? null,
              error: completedTraceRecordPatch.error ?? null,
              inputTokens: completedTraceRecordPatch.inputTokens ?? null,
              cacheHitInputTokens: completedTraceRecordPatch.cacheHitInputTokens ?? null,
              reasoningTokens: completedTraceRecordPatch.reasoningTokens ?? null,
              outputTokens: completedTraceRecordPatch.outputTokens ?? null,
              totalTokens: completedTraceRecordPatch.totalTokens ?? null,
              firstTokenLatencyMs: completedTraceRecordPatch.firstTokenLatencyMs ?? null,
              turnDurationMs: completedTraceRecordPatch.turnDurationMs ?? null,
              updatedAt: traceUpdatedAt
            });
          }

          // --- Metadata: traceSteps/toolActivities 与全局展示字段仅在
          // 该 turn 仍是当前前台 turn 时写入，避免覆盖新 turn 已设置的状态。
          // turnTraceHistory 记录（含 provider/tokens）始终完整写入。
          if (stillOwnsGlobalState) {
            state.traceSteps = nextTraceSteps;
            state.toolActivities = terminalToolActivities;
            state.sessionSummary = nextSessionSummary;
            state.providerRequestedName = nextProviderRequestedName;
            state.providerName = nextProviderName;
            state.providerProtocol = nextProviderProtocol;
            state.providerModel = nextProviderModel;
            state.providerSource = nextProviderSource;
            state.providerMode = nextProviderMode;
            state.fallbackReason = payload.fallbackReason ?? null;
            state.inputTokens = nextInputTokens;
            state.outputTokens = nextOutputTokens;
            state.totalTokens = nextTotalTokens;
            state.firstTokenLatencyMs = nextFirstTokenLatencyMs;
          }
        });

        // ===== STAGE 3 (runLowPriorityTurnWork): Non-urgent async =====
        runLowPriorityTurnWork(() => {
          this.scheduleDeferredPersist();
          void this.loadSessionCatalog().catch(() => {});
          void this.loadRetrievedContextState(completedSessionId, {
            runId: completedRunId,
            nodeId: completedNodeId
          }).then((retrieved) => {
            if (this.sessionId === completedSessionId) {
              this.retrievedContext = retrieved;
            }
          }).catch(() => {});

          // 触发自动消费下一条排队消息
          if (this.pendingQueuedMessages.length > 0 && !this.isSubmitting) {
            const nextMsg = this.dequeueNextMessage();
            if (nextMsg) {
              this.draftMessage = nextMsg.content;
              void this.submitTurn();
            }
          }
        });

        debugLog("event:completed", {
          turnId: payload.turnId,
          inputTokens: payload.inputTokens ?? null,
          cacheHitInputTokensRaw:
            payload.cacheHitInputTokens ??
            readNestedNumericTokenValue(completedPayloadRecord, [["cache_hit_input_tokens"]]) ??
            readNestedNumericTokenValue(completedPayloadRecord, [["promptCacheHitTokens"]]) ??
            readNestedNumericTokenValue(completedPayloadRecord, [["prompt_cache_hit_tokens"]]) ??
            null,
          cacheHitInputTokensResolved: cacheHitInputTokens,
          reasoningTokensResolved: reasoningTokens,
          turnDurationMs: payload.turnDurationMs ?? null,
          finalTextLength: payload.text?.length ?? 0,
          messages: this.messages.length,
          traces: this.turnTraceHistory.length,
          traceCacheHitInputTokens:
            this.turnTraceHistory.find((turn) => turn.turnId === payload.turnId)?.cacheHitInputTokens ?? null
        });
      });
    });

    await listen("turn:failed", (payload) => {
      // Wave-59 F4-5 / D-59-9：retry 占位清零**不得**依赖 turnId 门禁。
      // 生产路径里本地 requestId 与后端回传 turnId 可能不同（submitTurn 用本地 id 建消息，
      // 后端事件带自己的 turnId），若把清零放在 `activeTurnId !== payload.turnId` 之后，
      // 该轮永远无法退出「正在重试」。这里只做会话归属判定（不同会话的事件不得动本会话状态）。
      if (isEventForCurrentSession(payload)) {
        this.clearTimeoutRetryPlaceholder();
        // wave-metrics（task-4）：失败终态若携带结算快照同样落库（`null`/缺键 ⇒ 保持不变，I7）。
        this.applyMetricsSnapshot(payload.metrics);
      }
      if (payload.sessionId) {
        const bg = this.runningSessionMap[payload.sessionId];
        if (bg && bg.turnId === payload.turnId) {
          this.updatePersistedBackgroundSession(payload.sessionId, payload.turnId, {
            // wave-64 §2 F2（契约修订）：后台会话轮同样走 wire text 归一化 ——
            // `null` ⇒ 不写 content（保留已流式产出的正文），后缀说明文案一律剥离。
            content: normalizeFailedTurnContent(payload.text),
            reasoningContent: payload.reasoningContent ?? null,
            status: "error",
            modelName: buildAssistantModelLabel(payload.providerName, payload.providerModel),
            tokenCount: payload.outputTokens ?? null,
            phase: "failed",
            sessionSummary: payload.sessionSummary ?? null,
            providerRequestedName: payload.providerRequestedName ?? null,
            providerName: payload.providerName ?? null,
            providerProtocol: payload.providerProtocol ?? null,
            providerModel: payload.providerModel ?? null,
            providerSource: payload.providerSource ?? null,
            providerMode: payload.providerMode ?? null,
            fallbackReason: payload.fallbackReason ?? undefined,
            inputTokens: payload.inputTokens ?? null,
            outputTokens: payload.outputTokens ?? null,
            totalTokens: payload.totalTokens ?? null,
            firstTokenLatencyMs: payload.firstTokenLatencyMs ?? null,
            errorDetail: payload.error ?? DEFAULT_FAILED_TURN_ERROR
          });
          this.failedSessionSet[payload.sessionId] = true;
          delete this.runningSessionMap[payload.sessionId];
          return;
        }
      }
      if (this.activeTurnId !== payload.turnId || !isEventForCurrentSession(payload)) {
        return;
      }
      // Wave-59 B-59-1：终态必须无条件复位 hop 级流式类型。
      // 末次模型调用按协议不保证发 turn:hop_complete，若只在 turn:started/hop_complete 复位，
      // 本轮结束会残留 "content"，使下一轮 awaiting-first-signal 窗口被误判为「正文流式」
      // 而隐藏状态条（F1-2 / F4-2 语义失效）。
      this.activeStreamKind = null;
      if (!this.shouldProcessTurnEvent(payload)) {
        return;
      }

      // 历史模式下终态事件：仅解锁运行态，不污染历史视图（防止状态永久卡死）
      if (isHistoricalMode(this.historyCursorMode)) {
        this.clearSubmissionWatchdog();
        this.$patch((state) => {
          state.isSubmitting = false;
          state.activeTurnId = null;
          state.phase = "ready";
        });
        return;
      }

      // ===== STAGE 1 (sync): Critical chat area mutations only =====
      this.commitTurnEventCursor(payload);
      this.flushBufferedStreamText(payload.turnId);
      this.flushPendingTraceTimeline();

      const assistantMessage = this.ensureAssistantMessage(
        payload.turnId,
        buildAssistantModelLabel(payload.providerName, payload.providerModel)
      );

      assistantMessage.reasoningContent = normalizeReasoningContent(payload.reasoningContent ?? null);
      assistantMessage.modelName = buildAssistantModelLabel(payload.providerName, payload.providerModel);

      // Pre-compute values for later stages
      const terminalToolActivities = resolveTerminalToolActivities(payload.toolActivities, this.toolActivities);
      logCacheTelemetryContractViolations("failed", payload);
      const cacheHitInputTokens = resolveProviderReturnedCacheHitInputTokens(payload);
      const reasoningTokens = resolveReasoningTokens(payload);
      const cacheHitInputTokenPatch = cacheHitInputTokens != null ? { cacheHitInputTokens } : {};
      const reasoningTokenPatch = reasoningTokens != null ? { reasoningTokens } : {};
      const turnDurationPatch = payload.turnDurationMs != null ? { turnDurationMs: payload.turnDurationMs } : {};
      if (isDebugLoggingEnabled()) {
        debugLog("cache-telemetry:terminal-payload", {
          terminalEvent: "failed",
          ...buildCacheTelemetryDebugSnapshot(payload)
        });
      }
      const failedSessionId = this.sessionId;
      const failedRunId = this.activeRunId;
      const failedNodeId = this.visibleNodeId;

      // --- Retry check: retired per PA-070 contract ---
      // Frontend silent whole-turn auto-retry is retired.
      // Provider request-level retry is handled inside pony-agent-core.
      // If explicit turn-level retry is needed in the future, it must be
      // a separate control-plane orchestration action, not a silent frontend timer.

      // Standard failed path:
      // wave-64 §2 F2：失败**不得**另起消息 —— 占位正文「本轮执行失败，请查看右侧 trace。」
      // 已废除。core 线上恒发 `text`：零正文兼容锁（`"This turn failed."`）与失败说明后缀
      // 均由 `normalizeFailedTurnContent` 归一化；归一化为 `null`（纯占位/零正文）时
      // **不得**写 content，一律保留本轮已产出的正文。失败终态由唯一的状态消息
      // （`agentRuntimeStatus.mode === "error"`）承载。
      const normalizedFailedContent = normalizeFailedTurnContent(payload.text);
      if (normalizedFailedContent != null) {
        assistantMessage.content = normalizedFailedContent;
      }
      assistantMessage.status = "error";
      assistantMessage.errorDetail = payload.error ?? DEFAULT_FAILED_TURN_ERROR;

      // Apply token stats and sync tool messages inline (no deferred reactive cycle)
      this.applyTurnTokenStats(payload.turnId, payload.inputTokens, payload.outputTokens, false);
      this.syncToolMessages(payload.turnId, payload.toolActivities, false);

      this.phase = resolveRuntimePhaseFromEvent(payload, "failed");
      this.error = payload.error ?? DEFAULT_FAILED_TURN_ERROR;
      this.traceSteps = payload.traceSteps ?? this.traceSteps;
      this.toolActivities = terminalToolActivities;
      this.providerRequestedName = payload.providerRequestedName ?? this.providerRequestedName;
      this.providerName = payload.providerName ?? this.providerName;
      this.providerProtocol = payload.providerProtocol ?? this.providerProtocol;
      this.providerModel = payload.providerModel ?? this.providerModel;
      this.providerSource = payload.providerSource ?? this.providerSource;
      this.providerMode = payload.providerMode ?? this.providerMode;
      this.fallbackReason = payload.fallbackReason ?? this.fallbackReason;
      this.inputTokens = payload.inputTokens ?? this.inputTokens;
      this.outputTokens = payload.outputTokens ?? this.outputTokens;
      this.totalTokens = payload.totalTokens ?? this.totalTokens;
      this.firstTokenLatencyMs = payload.firstTokenLatencyMs ?? this.firstTokenLatencyMs;
      this.clearSubmissionWatchdog();
      this.isSubmitting = false;
      const failedTurnTraceId = payload.traceId
        ?? (this.activeTurnId ? this.turnTraceHistory?.find((t) => t.turnId === this.activeTurnId)?.traceId ?? null : null);
      this.activeTurnId = null;
      this.activeRunId = null;
      const failedTraceTimeline = resolveEventTraceTimeline(payload, () =>
        buildFallbackRuntimeTraceTimeline({
          turnId: payload.turnId,
          eventType: payload.eventType,
          messages: this.messages,
          phase: "failed",
          assistantMessage,
          toolActivities: terminalToolActivities,
          providerPatch: {
            providerName: payload.providerName ?? this.providerName,
            providerProtocol: payload.providerProtocol ?? this.providerProtocol,
            providerModel: payload.providerModel ?? this.providerModel,
            providerSource: payload.providerSource ?? this.providerSource,
            providerMode: payload.providerMode ?? this.providerMode
          },
          terminalState: "error",
          fallbackReason: payload.fallbackReason ?? this.fallbackReason,
          error: payload.error ?? DEFAULT_FAILED_TURN_ERROR,
          inputTokens: payload.inputTokens ?? this.inputTokens,
          cacheHitInputTokens,
          reasoningTokens,
          outputTokens: payload.outputTokens ?? this.outputTokens,
          totalTokens: payload.totalTokens ?? this.totalTokens,
          firstTokenLatencyMs: payload.firstTokenLatencyMs ?? this.firstTokenLatencyMs,
          turnDurationMs: payload.turnDurationMs ?? null
        })
      );
      // Pre-computed traceTimeline is already fresh — skip redundant clone inside
      this.commitTurnTraceTimeline(payload.turnId, failedTraceTimeline, {
        eventId: payload.eventId ?? null,
        eventType: payload.eventType ?? null,
        eventVersion: payload.eventVersion ?? null,
        sequence: payload.sequence ?? null,
        emittedAtMs: payload.emittedAtMs ?? null,
        phase: "failed",
        traceSteps: payload.traceSteps ?? this.traceSteps,
        ...(payload.traceId ? { traceId: payload.traceId } : {}),
        toolActivities: terminalToolActivities,
        providerCallRecords: cloneProviderCallRecords(payload.providerCallRecords),
        providerRequestedName: payload.providerRequestedName ?? this.providerRequestedName,
        providerName: payload.providerName ?? this.providerName,
        providerProtocol: payload.providerProtocol ?? this.providerProtocol,
        providerModel: payload.providerModel ?? this.providerModel,
        providerSource: payload.providerSource ?? this.providerSource,
        providerMode: payload.providerMode ?? this.providerMode,
        buildContextObservation: cloneBuildContextObservation(payload.buildContextObservation),
        hookTraceRecords: cloneHookTraceRecords(payload.hookTraceRecords),
        fallbackReason: payload.fallbackReason ?? this.fallbackReason,
        inputTokens: payload.inputTokens ?? this.inputTokens,
        outputTokens: payload.outputTokens ?? this.outputTokens,
        totalTokens: payload.totalTokens ?? this.totalTokens,
        firstTokenLatencyMs: payload.firstTokenLatencyMs ?? this.firstTokenLatencyMs,
        ...cacheHitInputTokenPatch,
        ...reasoningTokenPatch,
        ...turnDurationPatch,
        error: payload.error ?? DEFAULT_FAILED_TURN_ERROR
      }, false);

      window.setTimeout(() => {
        if (this.sessionId !== failedSessionId || isHistoricalMode(this.historyCursorMode)) {
          return;
        }
        runLowPriorityTurnWork(() => {
          this.persistHistory();
          void this.loadRetrievedContextState(failedSessionId, {
            runId: failedRunId,
            nodeId: failedNodeId
          }).then((retrieved) => {
            if (this.sessionId === failedSessionId) {
              this.retrievedContext = retrieved;
            }
          });
        });

        debugLog("event:failed", {
          turnId: payload.turnId,
          error: this.error
        });

        // 宿主层/Harness 捕获 Agent 失败事件上报 PonySentry (ADR 0028)
        // Wave-7 B3：用量治理阻断属于正常预期的用户状态，禁止将含有金额的 error 上报到 Sentry
        const errDetail = payload.error || this.error || "";
        const isGovernanceBlocked =
          errDetail.includes("用量治理拦截") ||
          errDetail.includes("硬限额") ||
          errDetail.includes("limit exceeded") ||
          errDetail.includes("Usage limit") ||
          (payload as any).failureKind === "governance_block";

        if (!isGovernanceBlocked) {
          void reportError({
            errorType: "TurnExecutionFailed",
            message: payload.error || this.error || DEFAULT_FAILED_TURN_ERROR,
            trace_id: failedTurnTraceId || undefined,
            tags: {
              source: "turn_failed_event",
              session_id: failedSessionId || "unknown",
              turn_id: payload.turnId,
              provider: payload.providerName || this.providerName || "unknown",
              model: payload.providerModel || this.providerModel || "unknown"
            },
            extra: {
              turnId: payload.turnId,
              sessionId: failedSessionId,
              providerRequestedName: payload.providerRequestedName ?? this.providerRequestedName,
              providerName: payload.providerName ?? this.providerName,
              providerProtocol: payload.providerProtocol ?? this.providerProtocol,
              providerModel: payload.providerModel ?? this.providerModel,
              errorDetail: payload.error ?? this.error
            }
          });
        }
      }, 0);
    });

    await listen("turn:cancelled", (payload) => {
      // Wave-59 F4-5 / D-59-9：retry 占位清零**不得**依赖 turnId 门禁。
      // 生产路径里本地 requestId 与后端回传 turnId 可能不同（submitTurn 用本地 id 建消息，
      // 后端事件带自己的 turnId），若把清零放在 `activeTurnId !== payload.turnId` 之后，
      // 该轮永远无法退出「正在重试」。这里只做会话归属判定（不同会话的事件不得动本会话状态）。
      if (isEventForCurrentSession(payload)) {
        this.clearTimeoutRetryPlaceholder();
        // wave-metrics（task-4）：取消终态若携带结算快照同样落库（`null`/缺键 ⇒ 保持不变，I7）。
        this.applyMetricsSnapshot(payload.metrics);
      }
      // Detect terminal event for a background session's turn
      if (payload.sessionId) {
        const bg = this.runningSessionMap[payload.sessionId];
        if (bg && bg.turnId === payload.turnId) {
          delete this.runningSessionMap[payload.sessionId];
          return;
        }
      }

      if (this.activeTurnId !== payload.turnId || !isEventForCurrentSession(payload)) {
        return;
      }
      // Wave-59 B-59-1：终态必须无条件复位 hop 级流式类型。
      // 末次模型调用按协议不保证发 turn:hop_complete，若只在 turn:started/hop_complete 复位，
      // 本轮结束会残留 "content"，使下一轮 awaiting-first-signal 窗口被误判为「正文流式」
      // 而隐藏状态条（F1-2 / F4-2 语义失效）。
      this.activeStreamKind = null;
      if (!this.shouldProcessTurnEvent(payload)) {
        return;
      }

      // 历史模式下终态事件：仅解锁运行态，不污染历史视图（防止状态永久卡死）
      if (isHistoricalMode(this.historyCursorMode)) {
        this.clearSubmissionWatchdog();
        this.$patch((state) => {
          state.isSubmitting = false;
          state.activeTurnId = null;
          state.phase = "ready";
        });
        return;
      }

      // ===== STAGE 1 (sync): Critical chat area mutations only =====
      this.commitTurnEventCursor(payload);
      this.flushBufferedStreamText(payload.turnId);
      this.flushPendingTraceTimeline();

      const cancelledTraceSteps = finalizeCancelledTraceSteps(payload.traceSteps ?? this.traceSteps);
      logCacheTelemetryContractViolations("cancelled", payload);
      const cancelledCacheHitInputTokens = resolveProviderReturnedCacheHitInputTokens(payload);
      const cancelledReasoningTokens = resolveReasoningTokens(payload);
      if (isDebugLoggingEnabled()) {
        debugLog("cache-telemetry:terminal-payload", {
          terminalEvent: "cancelled",
          ...buildCacheTelemetryDebugSnapshot(payload)
        });
      }

      const assistantMessage = this.ensureAssistantMessage(
        payload.turnId,
        buildAssistantModelLabel(payload.providerName, payload.providerModel)
      );

      // wave-60 §C7 + wave-64 §2 F2：**禁止**用后端占位文本（`CANCELLED_TURN_MESSAGE` 哨兵句
      // 或任意 text）覆盖已流式正文；已接收正文一律保留。
      // wave-64 §2 F2 起进一步**禁止**在未产出正文时补占位「本轮已停止。」——
      // 取消不再另起消息，终态由唯一的状态消息（`agentRuntimeStatus.mode === "stopped"`）承载。
      // 哨兵句仍留在**落库内容/历史读取**侧（`lib/runtime/cancelled-turn.ts` 与
      // `lib/runtime/messages.ts` 的 strip 行为不动，§0 非目标 2）。
      assistantMessage.reasoningContent = normalizeReasoningContent(payload.reasoningContent ?? null);
      assistantMessage.status = "done";
      assistantMessage.modelName = buildAssistantModelLabel(payload.providerName, payload.providerModel);

      // Pre-compute values for later stages
      const terminalToolActivities = resolveTerminalToolActivities(payload.toolActivities, this.toolActivities);
      const cacheHitInputTokenPatch = cancelledCacheHitInputTokens != null ? { cacheHitInputTokens: cancelledCacheHitInputTokens } : {};
      const reasoningTokenPatch = cancelledReasoningTokens != null ? { reasoningTokens: cancelledReasoningTokens } : {};
      const turnDurationPatch = payload.turnDurationMs != null ? { turnDurationMs: payload.turnDurationMs } : {};
      const cancelledSessionId = this.sessionId;
      const cancelledRunId = this.activeRunId;
      const cancelledNodeId = this.visibleNodeId;

      // Apply token stats and sync tool messages inline (no deferred reactive cycle)
      this.applyTurnTokenStats(payload.turnId, payload.inputTokens, payload.outputTokens, false);
      this.syncToolMessages(payload.turnId, payload.toolActivities, false);

      // Keep terminal UI state consistent even before deferred trace work runs.
      this.phase = resolveRuntimePhaseFromEvent(payload, "cancelled");
      // wave-64 §2 F1：停止终态的唯一承载面是状态消息（`agentRuntimeStatus.mode === "stopped"`，
      // 由 `phase` 派生、刷新/恢复后仍成立）。`sessionNotice` 不再承载停止文案 ——
      // 该通道只保留 C-9 斜杠命令的中性反馈用途。
      this.error = null;
      this.traceSteps = cancelledTraceSteps;
      this.toolActivities = terminalToolActivities;
      this.providerRequestedName = payload.providerRequestedName ?? this.providerRequestedName;
      this.providerName = payload.providerName ?? this.providerName;
      this.providerProtocol = payload.providerProtocol ?? this.providerProtocol;
      this.providerModel = payload.providerModel ?? this.providerModel;
      this.providerSource = payload.providerSource ?? this.providerSource;
      this.providerMode = payload.providerMode ?? this.providerMode;
      this.fallbackReason = payload.fallbackReason ?? this.fallbackReason;
      this.clearSubmissionWatchdog();
      this.isSubmitting = false;
      this.activeTurnId = null;
      this.activeRunId = null;
      const cancelledTraceTimeline = resolveEventTraceTimeline(payload, () =>
        buildFallbackRuntimeTraceTimeline({
          turnId: payload.turnId,
          eventType: payload.eventType,
          messages: this.messages,
          phase: "cancelled",
          assistantMessage,
          toolActivities: terminalToolActivities,
          providerPatch: {
            providerName: payload.providerName ?? this.providerName,
            providerProtocol: payload.providerProtocol ?? this.providerProtocol,
            providerModel: payload.providerModel ?? this.providerModel,
            providerSource: payload.providerSource ?? this.providerSource,
            providerMode: payload.providerMode ?? this.providerMode
          },
          terminalState: "cancelled",
          fallbackReason: payload.fallbackReason ?? this.fallbackReason,
          error: payload.error ?? "stopped_by_user",
          inputTokens: payload.inputTokens ?? null,
          cacheHitInputTokens: cancelledCacheHitInputTokens,
          reasoningTokens: cancelledReasoningTokens,
          outputTokens: payload.outputTokens ?? null,
          totalTokens: payload.totalTokens ?? null,
          firstTokenLatencyMs: payload.firstTokenLatencyMs ?? this.firstTokenLatencyMs,
          turnDurationMs: payload.turnDurationMs ?? null
        })
      );
      // Pre-computed traceTimeline is already fresh — skip redundant clone inside
      this.commitTurnTraceTimeline(payload.turnId, cancelledTraceTimeline, {
        eventId: payload.eventId ?? null,
        eventType: payload.eventType ?? null,
        eventVersion: payload.eventVersion ?? null,
        sequence: payload.sequence ?? null,
        emittedAtMs: payload.emittedAtMs ?? null,
        phase: "cancelled",
        traceSteps: cancelledTraceSteps,
        ...(payload.traceId ? { traceId: payload.traceId } : {}),
        toolActivities: terminalToolActivities,
        providerCallRecords: cloneProviderCallRecords(payload.providerCallRecords),
        providerRequestedName: payload.providerRequestedName ?? this.providerRequestedName,
        providerName: payload.providerName ?? this.providerName,
        providerProtocol: payload.providerProtocol ?? this.providerProtocol,
        providerModel: payload.providerModel ?? this.providerModel,
        providerSource: payload.providerSource ?? this.providerSource,
        providerMode: payload.providerMode ?? this.providerMode,
        buildContextObservation: cloneBuildContextObservation(payload.buildContextObservation),
        hookTraceRecords: cloneHookTraceRecords(payload.hookTraceRecords),
        fallbackReason: payload.fallbackReason ?? this.fallbackReason,
        ...cacheHitInputTokenPatch,
        ...reasoningTokenPatch,
        ...turnDurationPatch,
        error: payload.error ?? "stopped_by_user"
      }, false);

      // Yield to browser
      window.setTimeout(() => {
        if (this.sessionId !== cancelledSessionId || isHistoricalMode(this.historyCursorMode)) {
          return;
        }

        // ===== STAGE 2 (setTimeout 0): Non-urgent async work =====
        runLowPriorityTurnWork(() => {
          this.persistHistory();
          void this.loadRetrievedContextState(cancelledSessionId, {
            runId: cancelledRunId,
            nodeId: cancelledNodeId
          }).then((retrieved) => {
            if (this.sessionId === cancelledSessionId) {
              this.retrievedContext = retrieved;
            }
          });
        });

        debugLog("event:cancelled", {
          turnId: payload.turnId
        });
      }, 0);
    });

    // wave-60 §C2 / §C7：`turn:stop_requested` 是「停止请求已受理」的即时事件（**不是**终态）。
    // 跨窗口/重连路径下本地可能还没进 stopping，这里必须补切；本地已 stopping 则幂等无操作。
    await listen("turn:stop_requested", (payload) => {
      if (!isEventForCurrentSession(payload)) {
        return;
      }
      // 后台会话轮次由 runningSessionMap 承载，不得改动当前会话的进行中态。
      if (payload.sessionId && this.runningSessionMap[payload.sessionId]) {
        return;
      }
      if (this.activeTurnId !== payload.turnId) {
        return;
      }
      this.enterStoppingPhase();
      debugLog("event:stop-requested", {
        turnId: payload.turnId,
        stopReason: payload.stopReason ?? null
      });
    });

    await listen("turn:suspended", (payload) => {
      // Wave-59 F4-5 / D-59-9：retry 占位清零**不得**依赖 turnId 门禁。
      // 生产路径里本地 requestId 与后端回传 turnId 可能不同（submitTurn 用本地 id 建消息，
      // 后端事件带自己的 turnId），若把清零放在 `activeTurnId !== payload.turnId` 之后，
      // 该轮永远无法退出「正在重试」。这里只做会话归属判定（不同会话的事件不得动本会话状态）。
      if (isEventForCurrentSession(payload)) {
        this.clearTimeoutRetryPlaceholder();
        // wave-metrics（task-4）：挂起终态若携带结算快照同样落库（`null`/缺键 ⇒ 保持不变，I7）。
        this.applyMetricsSnapshot(payload.metrics);
      }
      // Detect terminal event for a background session's turn
      if (payload.sessionId) {
        const bg = this.runningSessionMap[payload.sessionId];
        if (bg && bg.turnId === payload.turnId) {
          delete this.runningSessionMap[payload.sessionId];
          return;
        }
      }

      if (this.activeTurnId !== payload.turnId || !isEventForCurrentSession(payload)) {
        return;
      }
      // Wave-59 B-59-1：终态必须无条件复位 hop 级流式类型。
      // 末次模型调用按协议不保证发 turn:hop_complete，若只在 turn:started/hop_complete 复位，
      // 本轮结束会残留 "content"，使下一轮 awaiting-first-signal 窗口被误判为「正文流式」
      // 而隐藏状态条（F1-2 / F4-2 语义失效）。
      this.activeStreamKind = null;
      if (!this.shouldProcessTurnEvent(payload)) {
        return;
      }

      // 历史模式下终态事件：仅解锁运行态，不污染历史视图（防止状态永久卡死）
      if (isHistoricalMode(this.historyCursorMode)) {
        this.clearSubmissionWatchdog();
        this.$patch((state) => {
          state.isSubmitting = false;
          state.activeTurnId = null;
          state.phase = "waiting_user";
        });
        return;
      }

      // ===== STAGE 1 (sync): Critical chat area mutations only =====
      this.commitTurnEventCursor(payload);
      this.flushBufferedStreamText(payload.turnId);
      this.flushPendingTraceTimeline();

      const suspendedTraceSteps = payload.traceSteps ?? this.traceSteps;
      logCacheTelemetryContractViolations("suspended", payload);
      const suspendedCacheHitInputTokens = resolveProviderReturnedCacheHitInputTokens(payload);
      const suspendedReasoningTokens = resolveReasoningTokens(payload);
      if (isDebugLoggingEnabled()) {
        debugLog("cache-telemetry:terminal-payload", {
          terminalEvent: "suspended",
          ...buildCacheTelemetryDebugSnapshot(payload)
        });
      }

      const assistantMessage = this.ensureAssistantMessage(
        payload.turnId,
        buildAssistantModelLabel(payload.providerName, payload.providerModel)
      );

      if (payload.text) {
        assistantMessage.content = payload.text;
      }
      assistantMessage.reasoningContent = normalizeReasoningContent(payload.reasoningContent ?? assistantMessage.reasoningContent ?? null);
      assistantMessage.status = "done";
      assistantMessage.modelName = buildAssistantModelLabel(payload.providerName, payload.providerModel);

      // Pre-compute values for later stages
      const terminalToolActivities = resolveTerminalToolActivities(payload.toolActivities, this.toolActivities);
      const cacheHitInputTokenPatch = suspendedCacheHitInputTokens != null ? { cacheHitInputTokens: suspendedCacheHitInputTokens } : {};
      const reasoningTokenPatch = suspendedReasoningTokens != null ? { reasoningTokens: suspendedReasoningTokens } : {};
      const turnDurationPatch = payload.turnDurationMs != null ? { turnDurationMs: payload.turnDurationMs } : {};
      const suspendedSessionId = this.sessionId;
      const suspendedRunId = this.activeRunId;
      const suspendedNodeId = this.visibleNodeId;

      // Apply token stats and sync tool messages inline (no deferred reactive cycle)
      this.applyTurnTokenStats(payload.turnId, payload.inputTokens, payload.outputTokens, false);
      this.syncToolMessages(payload.turnId, payload.toolActivities, false);

      // Keep terminal UI state consistent: Ask paused the turn waiting for host answer
      this.phase = resolveRuntimePhaseFromEvent(payload, "waiting_user");
      this.error = null;
      this.traceSteps = suspendedTraceSteps;
      this.toolActivities = terminalToolActivities;
      this.providerRequestedName = payload.providerRequestedName ?? this.providerRequestedName;
      this.providerName = payload.providerName ?? this.providerName;
      this.providerProtocol = payload.providerProtocol ?? this.providerProtocol;
      this.providerModel = payload.providerModel ?? this.providerModel;
      this.providerSource = payload.providerSource ?? this.providerSource;
      this.providerMode = payload.providerMode ?? this.providerMode;
      this.fallbackReason = payload.fallbackReason ?? this.fallbackReason;
      this.clearSubmissionWatchdog();
      this.isSubmitting = false;
      this.activeTurnId = null;
      this.activeRunId = null;

      const suspendedTraceTimeline = resolveEventTraceTimeline(payload, () =>
        buildFallbackRuntimeTraceTimeline({
          turnId: payload.turnId,
          eventType: payload.eventType,
          messages: this.messages,
          phase: "waiting_user",
          assistantMessage,
          toolActivities: terminalToolActivities,
          providerPatch: {
            providerName: payload.providerName ?? this.providerName,
            providerProtocol: payload.providerProtocol ?? this.providerProtocol,
            providerModel: payload.providerModel ?? this.providerModel,
            providerSource: payload.providerSource ?? this.providerSource,
            providerMode: payload.providerMode ?? this.providerMode
          },
          terminalState: null,
          fallbackReason: payload.fallbackReason ?? this.fallbackReason,
          error: null,
          inputTokens: payload.inputTokens ?? null,
          cacheHitInputTokens: suspendedCacheHitInputTokens,
          reasoningTokens: suspendedReasoningTokens,
          outputTokens: payload.outputTokens ?? null,
          totalTokens: payload.totalTokens ?? null,
          firstTokenLatencyMs: payload.firstTokenLatencyMs ?? this.firstTokenLatencyMs,
          turnDurationMs: payload.turnDurationMs ?? null
        })
      );

      this.commitTurnTraceTimeline(payload.turnId, suspendedTraceTimeline, {
        eventId: payload.eventId ?? null,
        eventType: payload.eventType ?? null,
        eventVersion: payload.eventVersion ?? null,
        sequence: payload.sequence ?? null,
        emittedAtMs: payload.emittedAtMs ?? null,
        phase: "waiting_user",
        traceSteps: suspendedTraceSteps,
        toolActivities: terminalToolActivities,
        providerCallRecords: cloneProviderCallRecords(payload.providerCallRecords),
        providerRequestedName: payload.providerRequestedName ?? this.providerRequestedName,
        providerName: payload.providerName ?? this.providerName,
        providerProtocol: payload.providerProtocol ?? this.providerProtocol,
        providerModel: payload.providerModel ?? this.providerModel,
        providerSource: payload.providerSource ?? this.providerSource,
        providerMode: payload.providerMode ?? this.providerMode,
        buildContextObservation: cloneBuildContextObservation(payload.buildContextObservation),
        hookTraceRecords: cloneHookTraceRecords(payload.hookTraceRecords),
        sessionSummary: payload.sessionSummary ?? this.sessionSummary,
        fallbackReason: payload.fallbackReason ?? this.fallbackReason,
        ...cacheHitInputTokenPatch,
        ...reasoningTokenPatch,
        ...turnDurationPatch,
        error: null
      }, false);

      // Yield to browser
      window.setTimeout(() => {
        if (this.sessionId !== suspendedSessionId || isHistoricalMode(this.historyCursorMode)) {
          return;
        }

        // ===== STAGE 2 (setTimeout 0): Non-urgent async work =====
        runLowPriorityTurnWork(() => {
          this.persistHistory();
          void this.loadRetrievedContextState(suspendedSessionId, {
            runId: suspendedRunId,
            nodeId: suspendedNodeId
          }).then((retrieved) => {
            if (this.sessionId === suspendedSessionId) {
              this.retrievedContext = retrieved;
            }
          });
        });

        debugLog("event:suspended", {
          turnId: payload.turnId
        });
      }, 0);
    });

    // 12 个句柄已在各自注册成功时逐条入表（见上方 `listen` 辅助函数），此处只需置就绪位。
    this.eventsReady = true;
  },
} as TurnEventsActions;
