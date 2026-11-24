// D3 轮次执行与流式传输（20 键 / 5 getters / 14 actions）
//
// 实现自 `2f11dc2f:src/stores/runtime.ts` **逐字搬迁**，语义零改动。键清单与签名真源见 `./types.ts`，
// 域归属与切分口径见 `.dev-team/runtime-split/fact-map.md`。与原实现的机械差异：import 路径按新层级
// 调整；action 方法补 `this: RuntimeStoreThis` 形参（Pinia options 式 `this` 口径）；类型
// `RuntimeStateW59` 改名 `RuntimeStoreState`（types.ts 的等价迁出物）。
// 模块级私有辅助（看门狗阈值、失败文案解析、重试提示拼接）随本域落位；其中
// `SUBMISSION_WATCHDOG_TIMEOUT_MS` 与注册表句柄辅助（`./turn-events`）按原有调用关系跨模块共享，
// 只加 `export`，调用点与语义不变。
import { isTauriAvailable } from "@/lib/tauri";
import type { TurnRetryEvent } from "@/types/runtime";
import { debugLog } from "@/lib/runtime/utils";
import {
  failedTurnRetryHint as resolveFailedTurnRetryHint,
  type TurnRetryHint,
} from "@/lib/runtime/turn-retry";
import {
  DEFAULT_FAILED_TURN_ERROR,
  STREAM_FLUSH_EAGER_CHARS,
  STREAM_FLUSH_INTERVAL_MS,
  TIMEOUT_RETRY_PENDING_MESSAGE,
} from "@/lib/runtime/constants";
import { appendNormalizedReasoningContent, buildAssistantModelLabel } from "@/lib/runtime/trace";
import { resolveAgentRuntimeStatus, type AgentRuntimeStatus } from "@/lib/runtime/status";
import {
  releaseTurnEventRegistry,
  turnEventRegistry,
  turnEventRegistryFor
} from "./turn-events";
import type {
  RuntimeStoreState,
  RuntimeStoreThis,
  TurnExecutionActions,
  TurnExecutionGetters,
} from "./types";
// 运行态看门狗：isSubmitting 置位后若在超时窗口内未收到任何终态事件
// （completed/failed/cancelled），强制解锁，杜绝"终态事件被丢弃 → 永久卡死"。
// 阈值 240s > 后端 provider 阻塞读超时 180s + 连接 15s 余量（契约冻结值）；
// 在 rearm 语义下只约束"连续静默窗口"，长任务只要持续有流式事件即不会误杀。
export const SUBMISSION_WATCHDOG_TIMEOUT_MS = 240_000;

/**
 * wave-64 §2 F1：失败终态的**真实错误文案**解析（用于刷新/恢复后 `state.error` 未持久化的场景）。
 *
 * 语义序：最近一条 error 助手消息的 `errorDetail` > 最近一条失败 trace 的 `error` > 默认错误码。
 * 严禁回落到占位正文（`DEFAULT_FAILED_TURN_MESSAGE`「本轮执行失败，请查看右侧 trace。」已废除）。
 * 注意：本函数**不读** `state.error` —— 该字段同时承载「后端连接失败」等非轮次错误，
 * 其展示口径由 `agentRuntimeStatus` 的既有 error 分支（要求 messages 非空）独管。
 */
function resolveTerminalFailureLabel(state: RuntimeStoreState): string {
  const messages = state.messages ?? [];
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index];
    if (message?.role === "assistant" && message.errorDetail?.trim()) {
      return message.errorDetail.trim();
    }
  }

  const traces = state.turnTraceHistory ?? [];
  for (let index = traces.length - 1; index >= 0; index -= 1) {
    const error = traces[index]?.error?.trim();
    if (error) {
      return error;
    }
  }

  return DEFAULT_FAILED_TURN_ERROR;
}

/**
 * P2-7：失败轮可重试提示的**唯一取值出口**（终态状态行标签与续跑动作共用）。
 *
 * 取值优先级与 [`resolveTerminalFailureLabel`] 同序：`error` → 最近一条 assistant
 * `errorDetail` → 最近一条 trace `error`。只在**未提交**时调用（进行中无终态提示）。
 */
function resolveRetryHintForState(state: RuntimeStoreState): TurnRetryHint | null {
  const direct = resolveFailedTurnRetryHint(state.error);
  if (direct) {
    return direct;
  }
  const messages = state.messages ?? [];
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const hint = resolveFailedTurnRetryHint(messages[index]?.errorDetail);
    if (hint) {
      return hint;
    }
  }
  const traces = state.turnTraceHistory ?? [];
  for (let index = traces.length - 1; index >= 0; index -= 1) {
    const hint = resolveFailedTurnRetryHint(traces[index]?.error);
    if (hint) {
      return hint;
    }
  }
  return null;
}

/**
 * P2-7：给终态状态行标签追加「上游失败，可重试」后缀。
 *
 * 幂等：标签里已有该短语时不再拼接；不可重试（配额/鉴权/未知）不追加，避免把
 * 「重试无用」的失败伪装成可重试。
 */
function appendRetryHintLabel(label: string, hint: TurnRetryHint | null): string {
  if (!hint || !hint.retryable || label.includes(hint.label)) {
    return label;
  }
  return `${label} · ${hint.label}`;
}

/** D3 轮次执行与流式传输（20 键 / 5 getters / 14 actions） getters（键清单见 `types.ts` 的 `TurnExecutionGetters` 与事实图 §2）。 */
export const turnExecutionGetters = {
  phaseLabel(state): string {
    const labels: Record<string, string> = {
      idle: "空闲",
      connecting: "连接中",
      ready: "已就绪",
      completed: "本轮完成",
      cancelled: "已停止",
      // wave-60 §C7：停止请求受理后、终态事件之前的进行中文案。
      stopping: "正在停止…",
      calling_model: "模型处理中",
      calling_tool: "工具处理中",
      failed: "失败"
    };

    return labels[state.phase] ?? state.phase;
  },
  /**
   * P2-7：失败轮的「可重试」提示（`null` ⇒ 无失败原因或无可提示项）。
   *
   * 消费面：续跑入口（[`retryLastTurn`]）与状态行（后缀已由 `agentRuntimeStatus`
   * 经同一判定拼接）。判定只在**未提交**时为真——进行中不该出现终态提示。
   */
  failedTurnRetry(state): TurnRetryHint | null {
    if (state.isSubmitting) {
      return null;
    }
    return resolveRetryHintForState(state);
  },
  agentRuntimeStatus(state): AgentRuntimeStatus | null {
    if (!state.isSubmitting) {
      if (state.sessionOperation === "initializing") {
        return {
          mode: "connecting",
          label: "正在加载最近对话…",
          startTime: null
        };
      }
      if (state.sessionOperation === "switching") {
        return {
          mode: "connecting",
          label: "正在切换对话…",
          startTime: null
        };
      }
      if (state.sessionOperation === "deleting") {
        return {
          mode: "connecting",
          label: "正在删除对话…",
          startTime: null
        };
      }
      if (state.sessionError?.trim()) {
        return {
          mode: "error",
          label: state.sessionError.trim(),
          startTime: null
        };
      }
      if (state.error?.trim() && state.messages.length > 0) {
        return {
          mode: "error",
          label: appendRetryHintLabel(state.error.trim(), resolveRetryHintForState(state)),
          startTime: null
        };
      }
      // C-9：无错误、无提交时，以 info 模式展示中性/成功反馈（如斜杠命令提示）。
      if (state.sessionNotice?.trim()) {
        return {
          mode: "info",
          label: state.sessionNotice.trim(),
          startTime: null
        };
      }
      // wave-64 §2 F1：**终态状态消息**是停止/失败的唯一承载面，由 `phase` 派生 ——
      // `canonicalTerminalPhase` 在恢复路径上经 `resolveRestoredPersistedPhase` 喂给 `phase`，
      // 故刷新/恢复后仍成立。优先序（Lead 裁决）：sessionOperation > sessionError > error >
      // sessionNotice（斜杠命令瞬时反馈）> 终态兜底；`stopped` 无 shimmer、无耗时（startTime=null）。
      if (state.phase === "cancelled") {
        return {
          mode: "stopped",
          label: "已停止",
          startTime: null
        };
      }
      // 失败终态：`state.error` 分支已覆盖实时事件路径（且要求 messages 非空），
      // 这里兜底恢复路径（`error` 不持久化），同样要求存在轮次消息，避免把启动期
      // 「后端连接失败」等非轮次错误塞进消息区状态行。
      if (state.phase === "failed" && state.messages.length > 0) {
        return {
          mode: "error",
          label: appendRetryHintLabel(
            resolveTerminalFailureLabel(state),
            resolveRetryHintForState(state)
          ),
          startTime: null
        };
      }
      return null;
    }
    const activeAssistantMessage = state.activeTurnId
      ? state.messages.find((m) => m.turnId === state.activeTurnId && m.role === "assistant")
      : null;
    // Wave-59 F4 / D-59-3：hop 级流式类型优先于「本轮累计内容」判据。
    // RC6（出现过 reasoning 就整轮 thinking）、RC7（多 hop 后仍判正在回复）由此消除；
    // `undefined`（未接线/既有夹具）严格回退旧判据，保证 wave-14 三例语义不变（F4-4）。
    const activeStreamKind = state.activeStreamKind;
    const isStreamingReasoning = activeStreamKind === "reasoning"
      ? true
      : activeStreamKind === "content"
        ? false
        : Boolean(
          state.streamBufferReasoning ||
          (activeAssistantMessage && activeAssistantMessage.reasoningContent)
        );
    const hasStreamingContent = activeStreamKind === "content"
      ? true
      : activeStreamKind === "reasoning" || activeStreamKind === null
        ? false
        : Boolean(
          state.streamBufferText ||
          (activeAssistantMessage && activeAssistantMessage.content)
        );
    // RC9 / F4-5：retrying 的**提交期**信号源 —— 超时（可重试错误）重试提交时，store 把
    // `TIMEOUT_RETRY_PENDING_MESSAGE` 写进 pending assistant（见 submitTurn）；首个 delta 与
    // `turn:started` 都会清空该占位文案 ⇒ 回到 calling_model 后重试信号自然清零。
    const retryingAfterTimeout = activeAssistantMessage?.content === TIMEOUT_RETRY_PENDING_MESSAGE;
    // 流式期的**真实**重试进度（`turn:retry` → `retryProgress`）优先：分母 `maxAttempts` 与
    // `attempt` 同源（后端 `RetryNotice`），故可展示 n/m；无通知时回退到占位判定且不传 retryMax
    // （status.ts 缺省降级为「正在重试…」，不虚构分母）。
    const retryProgress = state.retryProgress;
    return resolveAgentRuntimeStatus({
      isSubmitting: state.isSubmitting,
      phase: state.phase,
      toolActivities: state.toolActivities,
      turnStartedAt: state.activeTurnStartedAtMs ?? null,
      streamingReasoning: isStreamingReasoning,
      hasStreamingContent,
      retryAttempt: retryProgress ? retryProgress.attempt : retryingAfterTimeout ? 1 : undefined,
      retryMax: retryProgress?.maxAttempts
    });
  },
  /** 软告警不阻断原则：始终允许发送与输入 */
  canSubmitPrompt: (state): boolean => {
    return !state.isSubmitting;
  },
  isInputDisabled: (): boolean => {
    return false;
  },
} as TurnExecutionGetters;

/** D3 轮次执行与流式传输（20 键 / 5 getters / 14 actions） actions（键清单见 `types.ts` 的 `TurnExecutionActions` 与事实图 §3）。 */
export const turnExecutionActions = {
  scheduleStreamFlush(this: RuntimeStoreThis, turnId: string) {
    if (this.streamBufferTurnId && this.streamBufferTurnId !== turnId) {
      this.flushBufferedStreamText(this.streamBufferTurnId);
    }

    this.streamBufferTurnId = turnId;
    const bufferedLength = this.streamBufferText.length + this.streamBufferReasoning.length;
    if (bufferedLength >= STREAM_FLUSH_EAGER_CHARS) {
      this.flushBufferedStreamText(turnId);
      return;
    }

    if (this.streamFlushTimerId != null) {
      return;
    }

    // B279：flush 定时器必须带会话归属守卫 —— 会话切换后旧会话的 setTimeout
    // 不得把缓冲写进新会话（切走时虽已 cancel，仍保留运行时兜底）。
    const ownerSessionId = this.sessionId;
    this.streamFlushTimerId = window.setTimeout(() => {
      this.streamFlushTimerId = null;
      if (this.sessionId !== ownerSessionId || this.streamBufferTurnId !== turnId) {
        return;
      }
      this.flushBufferedStreamText(turnId);
    }, STREAM_FLUSH_INTERVAL_MS);
  },
  flushBufferedStreamText(this: RuntimeStoreThis, turnId?: string | null) {
    const bufferedTurnId = this.streamBufferTurnId;
    if (!bufferedTurnId) {
      return;
    }
    if (turnId && bufferedTurnId !== turnId) {
      return;
    }

    this.cancelStreamFlush();
    const flushedTextChars = this.streamBufferText.length;
    const flushedReasoningChars = this.streamBufferReasoning.length;

    const assistantMessage = this.ensureAssistantMessage(
      bufferedTurnId,
      buildAssistantModelLabel(this.providerName, this.providerModel)
    );

    if (this.streamBufferReasoning) {
      assistantMessage.reasoningContent = appendNormalizedReasoningContent(
        assistantMessage.reasoningContent ?? null,
        this.streamBufferReasoning
      );
    }

    if (this.streamBufferText) {
      if (assistantMessage.content === TIMEOUT_RETRY_PENDING_MESSAGE) {
        assistantMessage.content = "";
      }
      assistantMessage.content += this.streamBufferText;
      assistantMessage.errorDetail = null;
    }

    if (flushedTextChars > 0 || flushedReasoningChars > 0) {
      this.streamDebugFlushCount += 1;
      this.streamDebugTextCharsFlushed += flushedTextChars;
    }

    this.streamBufferTurnId = null;
    this.streamBufferText = "";
    this.streamBufferReasoning = "";
  },
  cancelStreamFlush(this: RuntimeStoreThis) {
    if (this.streamFlushTimerId == null) {
      return;
    }
    window.clearTimeout(this.streamFlushTimerId);
    this.streamFlushTimerId = null;
  },
  startSubmissionWatchdog(this: RuntimeStoreThis, turnId: string) {
    this.clearSubmissionWatchdog();
    this.submissionWatchdogTimerId = window.setTimeout(() => {
      this.handleSubmissionWatchdogTimeout(turnId);
    }, SUBMISSION_WATCHDOG_TIMEOUT_MS);
  },
  // 续期看门狗：对齐 DSH idleWatchdog 模式。收到 activeTurnId 匹配的流式事件或工具心跳时，
  // clear + 重设同一 turnId 的计时器。
  // 特殊保护：若当前轮次正在执行工具（phase 为 calling_tool），工具正在占用进程执行，
  // 看门狗自动给予动态缓冲续期，避免长耗时工具执行过程中被误杀。
  rearmSubmissionWatchdog(this: RuntimeStoreThis, turnId: string) {
    if (!this.isSubmitting || this.activeTurnId !== turnId) {
      return;
    }
    this.clearSubmissionWatchdog();
    this.submissionWatchdogTimerId = window.setTimeout(() => {
      this.handleSubmissionWatchdogTimeout(turnId);
    }, SUBMISSION_WATCHDOG_TIMEOUT_MS);
  },
  clearSubmissionWatchdog(this: RuntimeStoreThis) {
    if (this.submissionWatchdogTimerId != null) {
      window.clearTimeout(this.submissionWatchdogTimerId);
      this.submissionWatchdogTimerId = null;
    }
  },
  scheduleDeferredPersist(this: RuntimeStoreThis, delay = 140) {
    this.cancelDeferredPersist();
    if (typeof window === "undefined") {
      this.deferredPersistTimerId = null;
      return;
    }
    this.deferredPersistTimerId = window.setTimeout(() => {
      this.deferredPersistTimerId = null;
      if (typeof window === "undefined") {
        return;
      }
      this.persistHistory();
    }, delay);
  },
  cancelDeferredPersist(this: RuntimeStoreThis) {
    if (this.deferredPersistTimerId == null) {
      return;
    }

    // wave55：环境销毁后裸解引用 window 会抛 ReferenceError，此处只做清理不触达宿主定时器。
    if (typeof window !== "undefined") {
      window.clearTimeout(this.deferredPersistTimerId);
    }
    this.deferredPersistTimerId = null;
  },
  resetStreamDebugMetrics(this: RuntimeStoreThis) {
    this.streamDebugDeltaCount = 0;
    this.streamDebugFlushCount = 0;
    this.streamDebugTextCharsReceived = 0;
    this.streamDebugTextCharsFlushed = 0;
  },
  /**
   * Wave-59 F4-5 / RC9：清空「超时重连中...」占位文案 —— retrying 的唯一可视信号。
   *
   * 与 `ensureAssistantMessage` 的 `assistant-${turnId}` id 约定**解耦**：清零路径不得依赖
   * 单个 handler 的消息身份/门禁（历史/外部夹具可能使用其他 id），否则重试态会永久滞留。
   * 幂等；给定 turnId 时只清理该轮，避免串轮误清。
   */
  clearTimeoutRetryPlaceholder(this: RuntimeStoreThis, turnId?: string | null) {
    for (const message of this.messages) {
      if (message.role !== "assistant") {
        continue;
      }
      if (turnId && message.turnId !== turnId) {
        continue;
      }
      if (message.content === TIMEOUT_RETRY_PENDING_MESSAGE) {
        message.content = "";
      }
    }
    // 真实重试进度（`turn:retry`）同点清零：两者的生命周期边界一致（回合/跳转/首个输出），
    // 分开清会让状态行滞留「正在重试（3/10）...」。给定的 turnId 与进度所属轮次不符时保留。
    if (!turnId || this.retryProgress?.turnId === turnId) {
      this.retryProgress = null;
    }
  },
  /**
   * 桌面端上游重试进度落地（`turn:retry` → 状态行「正在重试（n/m）...」的唯一写点）。
   *
   * 归属判定沿用流式事件的 fail-closed 口径：`sessionId` 缺失或非本会话 ⇒ 丢弃
   * （后台会话的重试不得污染当前状态行）。**不**按 turnId 门禁 —— 生产路径里本地 requestId
   * 与后端 turnId 可能不同（D-59-9 同因），按 id 卡会让重试态再次不可达；过期由
   * `clearTimeoutRetryPlaceholder` 在各回合/跳转边界统一清空。
   *
   * 数值一律照抄后端通知（`attempt` / `maxAttempts` 同源），不在此推算、不补齐分母。
   */
  applyTurnRetryProgress(this: RuntimeStoreThis, payload: TurnRetryEvent) {
    if (!payload.sessionId || payload.sessionId !== this.sessionId) {
      return;
    }
    // 物理上不可信的负载（分母 < 分子 / 分子非正）不得进状态行。
    if (!(payload.attempt > 0) || payload.maxAttempts < payload.attempt) {
      return;
    }
    this.retryProgress = {
      turnId: payload.turnId,
      attempt: payload.attempt,
      maxAttempts: payload.maxAttempts
    };
  },
  async initializeTurnEvents(this: RuntimeStoreThis) {
    if (this.eventsReady) {
      return;
    }

    if (!isTauriAvailable()) {
      this.eventsReady = true;
      debugLog("events:browser-preview");
      return;
    }

    // F5b：并发单例锁 —— 并发调用共享同一初始化 promise，杜绝重复注册监听器
    // （旧实现每个并发调用都会把 12 个 `safeListen` 再注册一遍，且 unlisten 被 void 丢弃）。
    const registry = turnEventRegistryFor(this);
    if (!registry.initPromise) {
      registry.initPromise = this.registerTurnEventListeners(registry).catch((error) => {
        // A 级：注册中途失败（或本 action 抛错）⇒ 回滚已登记的句柄，避免半注册泄漏 +
        // 后续重试造成 delta 双写；随后释放锁让下次调用可重试。
        releaseTurnEventRegistry(registry);
        registry.initPromise = null;
        throw error;
      });
    }

    try {
      await registry.initPromise;
    } catch (error) {
      // 回滚已在上面完成；这里仅透传错误（也兜住 dispose 期间的 reject）。
      registry.initPromise = null;
      throw error;
    }
  },
  /**
   * F5b：可重入的事件监听清理路径。重建 / HMR / 切会话需要重新初始化前调用。
   *
   * A 级要点：**先等在飞初始化落定再清理**。否则在飞 init 会在 `initPromise = null`
   * 之后继续 push 句柄并置 `eventsReady = true`，导致「dispose 了但监听器仍在」。
   * 幂等：重复调用只会清理当次仍登记在册的句柄。
   */
  async disposeTurnEvents(this: RuntimeStoreThis): Promise<void> {
    const registry = turnEventRegistry.get(this);

    if (!registry) {
      this.eventsReady = false;
      return;
    }

    const inFlight = registry.initPromise;
    registry.initPromise = null;

    if (inFlight) {
      try {
        await inFlight;
      } catch {
        // 在飞 init 自身失败：其 catch 已回滚并释放锁，这里仅等它落定。
      }
    }

    releaseTurnEventRegistry(registry);
    this.eventsReady = false;
  },
  /**
   * wave-60 §C7 / 补遗 v2 A2：进入 `stopping` 进行中态（幂等、无副作用文案）。
   *
   * 分工：进行中态落 `phase="stopping"`（→ `phaseLabel` / `agentRuntimeStatus` 的 stopping 分支），
   * 终态文案落 `sessionNotice`；两者不得互相冒充。
   */
  enterStoppingPhase(this: RuntimeStoreThis) {
    if (!this.activeTurnId || !this.isSubmitting) {
      return;
    }
    if (this.phase === "stopping") {
      return;
    }
    this.phase = "stopping";
  },
} as TurnExecutionActions;
