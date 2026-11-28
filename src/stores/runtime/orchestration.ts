// 跨域事务入口（0 键 / 0 getters / 21 actions，事实图 §4）
//
// 实现自 `2f11dc2f:src/stores/runtime.ts` **逐字搬迁**，语义零改动。键清单与签名真源见 `./types.ts`，
// 域归属与切分口径见 `.dev-team/runtime-split/fact-map.md`。与原实现的机械差异：import 路径按新层级
// 调整；action 方法补 `this: RuntimeStoreThis` 形参（Pinia options 式 `this` 口径）；类型
// `RuntimeStateW59` 改名 `RuntimeStoreState`（types.ts 的等价迁出物）。
// 看门狗阈值 `SUBMISSION_WATCHDOG_TIMEOUT_MS` 由 `./turn-execution` 导出（原为同文件模块内常量）。
import { isTauriAvailable, safeInvoke } from "@/lib/tauri";
import {
  bindFrontendRecorderSession,
  bindFrontendRecorderTurn,
  initFrontendFlightRecorder,
} from "@/lib/frontend-flight-recorder";
import { useAskStore } from "@/stores/ask";
import { useProviderStore } from "@/stores/providers";
import { useSettingsStore } from "@/stores/settings";
import { isRetryableError } from "@/lib/error-utils";
import type {
  ChatMessage,
  ExecutionCheckpoint,
  GraphRunControlResponse,
  GraphRunSubmissionPlan,
  GraphRunStreamStartResponse,
  HealthPayload,
  HistoryCheckoutMode,
  HistoryCheckoutResult,
  MessageStateDelta,
  RetrievedContextState,
  RuntimePhase,
  SessionOverview,
  SessionRuntimeView,
  SessionSnapshot,
  StopTurnResponse,
  TraceTimelineEntry,
  TurnInput,
  TurnInputImage,
  TurnStreamEvent,
} from "@/types/runtime";
import type { PersistedRuntimeState } from "@/lib/runtime/types";
import type { HistoryCheckoutWireResult } from "@/lib/runtime/history";
import { DEFAULT_WORKSPACE_ID } from "@/lib/runtime/workspace-constants";
import {
  debugLog,
  reportSwitchPerf,
  runLowPriorityTurnWork,
  wait,
  waitForNextPaint,
  withTimeout,
} from "@/lib/runtime/utils";
import {
  BROWSER_PREVIEW_CHUNKS,
  BROWSER_PREVIEW_FALLBACK_REASON,
  BROWSER_PREVIEW_MODEL_NAME,
  BROWSER_PREVIEW_PROVIDER_NAME,
  BROWSER_PREVIEW_SESSION_SUMMARY,
  BROWSER_PREVIEW_TRACE_TITLE,
  CACHED_STATE_VERSION,
  DEFAULT_FAILED_TURN_ERROR,
  HYDRATION_TIMEOUT_MS,
  OUTPUT_END_PERSIST_DELAY_MS,
  TIMEOUT_RETRY_PENDING_MESSAGE,
} from "@/lib/runtime/constants";
import {
  applyProviderPatchToTraceTimeline,
  buildAssistantModelLabel,
  cloneTraceTimeline,
  createBrowserPreviewTraceSteps,
  createBrowserPreviewTraceTimeline,
  createDefaultTraceSteps,
  createDefaultTraceTimeline,
  createSubmitFailureTraceSteps,
  createSubmitFailureTraceTimeline,
  createSubmitTraceSteps,
  finalizeCancelledTraceSteps,
  normalizeReasoningContent,
  normalizeTurnTraceRecord,
  resolvedStreamStartRunId,
} from "@/lib/runtime/trace";
import {
  buildAttachmentMessageBlocks,
  buildAttachmentMetas,
  buildDisplayedUserMessageWithAttachments,
  buildProviderUserMessageWithAttachments,
  buildTurnHistory,
  buildTurnTitle,
  hydrateMessagesFromHistory,
  previewMessageStateDeltaMessages,
  reuseStableChatMessages,
} from "@/lib/runtime/messages";
import {
  cloneHistoryBranches,
  cloneHistoryNodes,
  cloneHistoryStateAuditSummary,
  cloneRunControlAuditSummary,
  hasHostManagedHistoryState,
  isHistoricalRuntimeView,
  normalizeHistoryCheckoutResult,
  normalizeHistoryRestoreResult,
  previewHistoryMutationUnavailable,
  resolveRuntimeViewHistoryProjection,
} from "@/lib/runtime/history";
import {
  createBrowserPreviewTerminalEnvelope,
  isTerminalPhase,
  normalizeCheckpointPhase,
  reconcileSubmissionWithRecoveryCheckpoint,
  resolveGraphRunSubmissionFromCheckpoint,
  resolveGraphRunSubmissionFromPlan,
  resolveGraphRunSubmissionFromRunState,
  resolveRestoredPersistedPhase,
} from "@/lib/runtime/phases";
import type { GraphRunSubmission } from "@/lib/runtime/phases";
import {
  buildEventCursorByTurnTraceHistory,
  buildRuntimeViewFromPersistedState,
  buildSessionOverviewFromRuntimeState,
  cloneGraphRunControlBoundaryEvidence,
  cloneRetrievedContext,
  createBlankSessionRuntimeFields,
  createNextSessionId,
  createSessionRuntimeSnapshot,
  createTransientSessionOverview,
  deriveInitStrategy,
  deriveRetrievedContextFromSnapshot,
  ensureUniqueSessionList,
  filterDeletingSessions,
  hasPersistableMessages,
  hasSelectableSession,
  isPersistedMetadataCompatible,
  isPersistedMessageShapeCompatible,
  loadPersistedRuntimeCache,
  loadPersistedRuntimeState,
  loadRunningSessionMap,
  mergeRuntimeViews,
  persistSessionState,
  persistSessionStateAndRuntimeMaps,
  pickPreferredSessionId,
  removePersistedSessionState,
  restoreSessionRuntimeSnapshot,
} from "@/lib/runtime/sessions";
import { SUBMISSION_WATCHDOG_TIMEOUT_MS } from "./turn-execution";
import type { OrchestrationActions, OrchestrationGetters, RuntimeStoreThis } from "./types";

async function resolveSubmissionForStore(store: RuntimeStoreThis): Promise<GraphRunSubmission> {
  let submission: GraphRunSubmission | null = null;
  if (store.sessionId) {
    try {
      const plan = await safeInvoke<GraphRunSubmissionPlan>("resolve_graph_run_submission_plan", {
        sessionId: store.sessionId,
        nodeId: store.visibleNodeId,
        runId: store.activeRunId
      });
      submission = resolveGraphRunSubmissionFromPlan(plan);
    } catch (planError) {
      debugLog("submit:resolve-plan:error", {
        sessionId: store.sessionId,
        runId: store.activeRunId,
        error: String(planError)
      });
    }
  }

  if (!submission) {
    submission = resolveGraphRunSubmissionFromPlan(store.latestGraphRunSubmissionPlan);
  }

  if (!submission) {
    submission = resolveGraphRunSubmissionFromRunState(store.retrievedContext?.runState);
    submission = reconcileSubmissionWithRecoveryCheckpoint(submission, store.latestExecutionCheckpoint);
    submission ??= resolveGraphRunSubmissionFromCheckpoint(
      store.latestExecutionCheckpoint,
      store.activeRunId
    );
  }

  if (!submission && store.sessionId) {
    await store.resolveDerivedSessionRun({
      sessionId: store.sessionId,
      runId: store.activeRunId,
      preferRefresh: true,
      nodeId: store.visibleNodeId
    });
    submission = resolveGraphRunSubmissionFromRunState(store.retrievedContext?.runState);
    submission = reconcileSubmissionWithRecoveryCheckpoint(submission, store.latestExecutionCheckpoint);
    submission ??= resolveGraphRunSubmissionFromCheckpoint(
      store.latestExecutionCheckpoint,
      store.activeRunId
    );
  }

  return submission ?? { command: "start_graph_run_stream" as const, runId: null };
}

function applyStreamStartOutcome(
  store: RuntimeStoreThis,
  response: GraphRunStreamStartResponse,
  fallbackRunId: string | null
): void {
  store.activeRunId = resolvedStreamStartRunId(response, fallbackRunId);
  store.latestGraphRunControlBoundaryEvidence = cloneGraphRunControlBoundaryEvidence(
    response.controlBoundaryEvidence ? [response.controlBoundaryEvidence] : null
  );
  store.latestRunControlAuditSummary = cloneRunControlAuditSummary(
    response.runControlAuditSummary ?? null
  );
}

/**
 * 由 `/goal <目标>` 或 `/goal resume` 自动触发的目标首轮/续轮入口。
 * 作为模块独立 helper 导出而非挂进 Pinia `orchestrationActions`，以保持 store 冻结表面（83/18/97）不变。
 */
export async function startGoalRound(
  store: RuntimeStoreThis,
  prompt: string,
  displayMessage?: string | null
): Promise<boolean> {
  const normalizedPrompt = prompt.trim();
  if (store.isSubmitting || !normalizedPrompt || !isTauriAvailable()) {
    return false;
  }

  store.sessionNotice = null;
  store.isSubmitting = true;
  await store.initializeTurnEvents();

  const providerStore = useProviderStore();
  const settingsStore = useSettingsStore();
  const selectedProvider = providerStore.currentProvider;
  const selectedModel = providerStore.currentModel;
  const selectedProviderName = selectedProvider?.name ?? null;
  const selectedProviderProtocol = selectedProvider?.protocol ?? null;
  const selectedModelName = selectedModel?.model ?? selectedModel?.name ?? null;
  const selectedProviderTracePatch = {
    providerName: selectedProviderName,
    providerProtocol: selectedProviderProtocol,
    providerModel: selectedModelName
  };

  store.historyCursorMode = "live";
  store.initialRollbackActive = false;

  const requestId = `turn-goal-${Date.now()}`;
  const goalDisplay = displayMessage?.trim() || normalizedPrompt;
  const payload: TurnInput = {
    message: normalizedPrompt,
    displayMessage: goalDisplay,
    providerId: selectedProvider?.id ?? null,
    modelId: selectedModel?.id ?? null,
    reasoningEffort: providerStore.currentReasoningEffort ?? null,
    workspaceMode: settingsStore.workspaceMode,
    toolAuthorizationMode: store.toolAuthorizationMode ?? "workspace_write",
    sessionId: store.sessionId,
    nodeId: store.visibleNodeId,
    history: buildTurnHistory(store.messages),
    images: [],
    workspaceId:
      store.sessionWorkspaceId ||
      store.sessionList.find((session) => session.conversationId === store.sessionId)?.workspaceId?.trim() ||
      store.activeWorkspaceId ||
      DEFAULT_WORKSPACE_ID
  };

  store.activeTurnStartedAtMs = Date.now();
  store.activeStreamKind = null;
  store.startSubmissionWatchdog(requestId);
  bindFrontendRecorderSession(store.sessionId);
  bindFrontendRecorderTurn(requestId);
  store.error = null;
  store.phase = "calling_model";
  store.activeTurnId = requestId;
  store.providerRequestedName = selectedProviderName ?? "";
  store.providerName = selectedProviderName ?? "";
  store.providerProtocol = selectedProviderProtocol ?? "";
  store.providerModel = selectedModelName ?? "";
  store.providerSource = "";
  store.providerMode = "";
  store.fallbackReason = null;
  store.inputTokens = null;
  store.outputTokens = null;
  store.totalTokens = null;
  store.firstTokenLatencyMs = null;
  store.liveMetrics = null;
  store.activeProviderCallRecords = [];
  store.traceSteps = createSubmitTraceSteps();
  store.publishTraceTimeline(applyProviderPatchToTraceTimeline(
    createDefaultTraceTimeline(),
    selectedProviderTracePatch
  ));
  store.toolActivities = [];
  store.commitTurnTraceTimeline(requestId, store.traceTimeline, {
    phase: "calling_model",
    traceSteps: store.traceSteps,
    toolActivities: [],
    providerRequestedName: selectedProviderName,
    providerName: selectedProviderName,
    providerProtocol: selectedProviderProtocol,
    providerModel: selectedModelName,
    providerSource: null,
    providerMode: null,
    fallbackReason: null,
    error: null
  }, false);
  store.scheduleDeferredPersist();

  try {
    const submission = await resolveSubmissionForStore(store);
    if (submission.command === "start_graph_run_stream") {
      const response = await safeInvoke<GraphRunStreamStartResponse>("start_graph_run_stream", {
        turnId: requestId,
        runId: null,
        goal: goalDisplay,
        input: payload
      });
      applyStreamStartOutcome(store, response, null);
      return true;
    }
    if (submission.command === "resume_graph_run_stream") {
      const response = await safeInvoke<GraphRunStreamStartResponse>("resume_graph_run_stream", {
        turnId: requestId,
        runId: submission.runId,
        input: payload
      });
      applyStreamStartOutcome(store, response, submission.runId);
      return true;
    }
    const response = await safeInvoke<GraphRunStreamStartResponse>("continue_graph_run_stream", {
      turnId: requestId,
      runId: submission.runId,
      input: payload
    });
    applyStreamStartOutcome(store, response, submission.runId);
    return true;
  } catch (error) {
    const assistantMessage = store.ensureAssistantMessage(
      requestId,
      buildAssistantModelLabel(selectedProviderName, selectedModelName)
    );
    assistantMessage.reasoningContent = null;
    assistantMessage.status = "error";
    assistantMessage.errorDetail = `目标自动推进失败：${String(error)}`;
    assistantMessage.modelName = buildAssistantModelLabel(selectedProviderName, selectedModelName);
    store.error = `目标自动推进失败：${String(error)}`;
    store.phase = "failed";
    store.activeTurnId = null;
    store.activeRunId = null;
    store.clearSubmissionWatchdog();
    store.traceSteps = createSubmitFailureTraceSteps();
    store.publishTraceTimeline(applyProviderPatchToTraceTimeline(
      createSubmitFailureTraceTimeline(),
      selectedProviderTracePatch
    ));
    store.commitTurnTraceTimeline(requestId, store.traceTimeline, {
      phase: "failed",
      traceSteps: store.traceSteps,
      toolActivities: store.toolActivities,
      providerRequestedName: selectedProviderName,
      providerName: selectedProviderName,
      providerProtocol: selectedProviderProtocol,
      providerModel: selectedModelName,
      error: store.error
    });
    store.persistHistory();
    return false;
  } finally {
    if (store.phase === "failed") {
      store.isSubmitting = false;
    }
  }
}

/** 跨域事务入口（0 键 / 0 getters / 21 actions，事实图 §4） getters（键清单见 `types.ts` 的 `OrchestrationGetters` 与事实图 §2）。 */
export const orchestrationGetters = {} as OrchestrationGetters;

/** 跨域事务入口（0 键 / 0 getters / 21 actions，事实图 §4） actions（键清单见 `types.ts` 的 `OrchestrationActions` 与事实图 §3）。 */
export const orchestrationActions = {
  resetSessionRuntimeState(this: RuntimeStoreThis) {
    this.cancelDeferredPersist();
    this.clearSubmissionWatchdog();
    const blankFields = createBlankSessionRuntimeFields();
    this.phase = "idle";
    this.error = null;
    this.sessionNotice = null;
    this.draftMessage = "";
    this.clearPendingAttachments();
    this.sessionHydrating = false;
    this.sessionSummary = blankFields.sessionSummary;
    this.retrievedContext = null;
    this.providerRequestedName = blankFields.providerRequestedName;
    this.providerName = blankFields.providerName;
    this.providerProtocol = blankFields.providerProtocol;
    this.providerModel = blankFields.providerModel;
    this.providerSource = blankFields.providerSource;
    this.providerMode = blankFields.providerMode;
    this.fallbackReason = blankFields.fallbackReason;
    this.inputTokens = blankFields.inputTokens;
    this.outputTokens = blankFields.outputTokens;
    this.totalTokens = blankFields.totalTokens;
    this.firstTokenLatencyMs = blankFields.firstTokenLatencyMs;
    // wave-metrics（task-4）：结算快照属会话内瞬时态 —— 会话切换/重置一律清空，
    // 使投影层回退既有 trace 反推路径（I7），杜绝跨会话串味。
    this.liveMetrics = null;
    // Wave-59 A-59-5：hop 级流式类型属会话内瞬时态，不得跨会话继承（B-59-1 同类缺口）。
    this.activeStreamKind = blankFields.activeStreamKind;
    this.activeProviderCallRecords = [];
    this.isSubmitting = false;
    this.activeTurnId = null;
    this.activeTurnStartedAtMs = null;
    this.activeRunId = null;
    this.latestGraphRunSubmissionPlan = null;
    this.latestGraphRunControlBoundaryEvidence = [];
    this.latestHistoryStateAuditSummary = null;
    this.visibleNodeId = null;
    this.branchHeadNodeId = null;
    this.activeBranchId = null;
    this.messageRevision = null;
    this.historyCursorMode = "live";
    this.historyNodes = [];
    this.historyBranches = [];
    this.initialRollbackActive = false;
    this.messages = [];
    this.attachmentAssets = [];
    this.toolActivities = [];
    this.traceSteps = createDefaultTraceSteps();
    this.publishTraceTimeline(createDefaultTraceTimeline());
    this.turnTraceHistory = [];
    this.eventCursorByTurnId = {};
    this.cancelStreamFlush();
    this.streamBufferTurnId = null;
    this.streamFlushTimerId = null;
    this.streamBufferText = "";
    this.streamBufferReasoning = "";
    this.browserPreviewRunToken = 0;
  },
  applySessionSnapshot(this: RuntimeStoreThis, 
    sessionId: string,
    snapshot: SessionSnapshot,
    retrieved?: RetrievedContextState | null,
    runtimeView?:
      | Pick<
          SessionRuntimeView,
          | "historyNodes"
          | "historyBranches"
          | "historyCursor"
          | "messageState"
          | "submissionPlan"
          | "controlBoundaryEvidence"
          | "runControlAuditSummary"
          | "historyStateAuditSummary"
        >
      | null,
    options?: { preserveMessages?: boolean }
  ) {
    const persisted = loadPersistedRuntimeState(sessionId);
    const historicalRuntimeView = isHistoricalRuntimeView(runtimeView);
    const persistedInitialRollbackActive = Boolean(persisted?.initialRollbackActive);
    const canReusePersistedState =
      persistedInitialRollbackActive || (!historicalRuntimeView && isPersistedMetadataCompatible(snapshot, persisted));
    const canMergePersistedMessages =
      persistedInitialRollbackActive || (!historicalRuntimeView && isPersistedMessageShapeCompatible(snapshot, persisted));
    const restoredState = canReusePersistedState ? persisted : null;
    const blankFields = createBlankSessionRuntimeFields();
    const retrievedSummary = retrieved?.sessionContext?.summary?.trim() ?? "";
    const snapshotSummary = snapshot.history.length > 0 ? snapshot.summary : "";
    const sessionSummary = retrievedSummary || restoredState?.sessionSummary || snapshotSummary;
    const snapshotTurnTraceHistory = snapshot.turnTraceHistory ?? [];
    const effectiveTurnTraceHistory = (
      snapshotTurnTraceHistory.length
        ? snapshotTurnTraceHistory
        : []
    ).map((trace) => normalizeTurnTraceRecord(trace));

    // B279：会话落点统一丢弃旧会话遗留的定时器与待投递节流帧。
    // 只在 sessionId 真正变化时清理，避免同会话刷新打断在飞的流式 flush。
    // 不触碰流式缓冲：目标会话的后台轮缓冲由 switchSession 的 bgTurn 恢复分支重建。
    if (this.sessionId !== sessionId) {
      this.cancelStreamFlush();
      this.clearPendingTraceTimeline();
    }

    this.sessionId = sessionId;
    this.error = null;
    this.clearSubmissionWatchdog();
    this.isSubmitting = false;
    this.activeTurnId = null;
    this.activeRunId = retrieved?.runState?.runId?.trim() || null;
    this.latestExecutionCheckpoint = null;
    this.latestGraphRunSubmissionPlan = null;
    this.latestGraphRunControlBoundaryEvidence = [];
    this.latestRunControlAuditSummary = cloneRunControlAuditSummary(
      runtimeView?.runControlAuditSummary ?? snapshot.runControlAuditSummary ?? null
    );
    this.latestHistoryStateAuditSummary = cloneHistoryStateAuditSummary(
      runtimeView?.historyStateAuditSummary ?? snapshot.historyStateAuditSummary ?? null
    );
    this.initialRollbackActive = Boolean(restoredState?.initialRollbackActive);
    this.draftMessage = "";
    this.sessionSummary = sessionSummary;
    this.retrievedContext = cloneRetrievedContext(retrieved ?? deriveRetrievedContextFromSnapshot(snapshot));
    if (runtimeView?.messageState) {
      this.applyMessageStateSnapshot(runtimeView.messageState);
    } else if (!options?.preserveMessages || !this.messages.length) {
      this.messages = reuseStableChatMessages(
        this.messages,
        hydrateMessagesFromHistory(
          snapshot.history,
          canMergePersistedMessages ? persisted?.messages : null,
          effectiveTurnTraceHistory
        )
      );
      this.messageRevision = null;
    }
    this.attachmentAssets = snapshot.attachmentAssets ?? restoredState?.attachmentAssets ?? [];
    this.turnTraceHistory = effectiveTurnTraceHistory;
    this.eventCursorByTurnId = buildEventCursorByTurnTraceHistory(this.turnTraceHistory);
    this.providerRequestedName = restoredState?.providerRequestedName ?? blankFields.providerRequestedName;
    this.providerName = restoredState?.providerName ?? blankFields.providerName;
    this.providerProtocol = restoredState?.providerProtocol ?? blankFields.providerProtocol;
    this.providerModel = restoredState?.providerModel ?? blankFields.providerModel;
    this.providerSource = restoredState?.providerSource ?? blankFields.providerSource;
    this.providerMode = restoredState?.providerMode ?? blankFields.providerMode;
    this.fallbackReason = restoredState?.fallbackReason ?? blankFields.fallbackReason;
    this.inputTokens = restoredState?.inputTokens ?? blankFields.inputTokens;
    this.outputTokens = restoredState?.outputTokens ?? blankFields.outputTokens;
    this.totalTokens = restoredState?.totalTokens ?? blankFields.totalTokens;
    this.firstTokenLatencyMs = restoredState?.firstTokenLatencyMs ?? blankFields.firstTokenLatencyMs;
    // wave-metrics（task-4）：快照不持久化（会话快照无该字段）⇒ 恢复目标会话时必须清空，
    // 消费侧回退既有 trace 反推路径（I7：历史会话/回放零差异）。
    this.liveMetrics = null;
    // Wave-59 A-59-5：会话快照不携带 hop 级瞬时态；恢复目标会话时一律清空，
    // 由随后的「后台运行轮恢复」分支按该轮缓冲重新推导。
    this.activeStreamKind = blankFields.activeStreamKind;
    this.activeProviderCallRecords = [];
    this.toolActivities = [];
    this.traceSteps = createDefaultTraceSteps();
    // 优化（PA-089）：切换会话时 traceTimeline 深拷贝延迟到 rIC——
    // 避免同步深拷贝（cloneTraceTimeline + cloneToolActivities）阻塞主线程
    // （实测切换会话 1.5-1.9s 卡顿源之一）。先发布默认 timeline，空闲时再深拷贝。
    const lastTraceTimeline = this.turnTraceHistory[this.turnTraceHistory.length - 1]?.traceTimeline;
    this.publishTraceTimeline(createDefaultTraceTimeline());
    if (lastTraceTimeline?.length) {
      // B279 同族守卫：rIC 空闲回调可能晚于下一次会话切换，必须校验归属，
      // 否则旧会话的 timeline 深拷贝会覆盖新会话（PA-089 延迟恢复引入的竞态）。
      const ownerSessionId = sessionId;
      runLowPriorityTurnWork(() => {
        if (this.sessionId !== ownerSessionId) {
          return;
        }
        const restored = cloneTraceTimeline(lastTraceTimeline);
        if (restored.length) {
          this.publishTraceTimeline(restored);
        }
      });
    }
    this.phase = resolveRestoredPersistedPhase(
      restoredState?.phase ?? null,
      restoredState?.canonicalTerminalPhase ?? null,
      this.messages,
      this.turnTraceHistory
    );
    const hydratedRunId = runtimeView?.submissionPlan?.runId?.trim() || null;
    if (!this.activeRunId && hydratedRunId) {
      this.activeRunId = hydratedRunId;
    }
    this.latestGraphRunControlBoundaryEvidence = cloneGraphRunControlBoundaryEvidence(
      runtimeView?.controlBoundaryEvidence
    );
    const runtimeHistoryCursor = resolveRuntimeViewHistoryProjection(runtimeView);
    const hasRuntimeHistoryState = Boolean(
      runtimeHistoryCursor || runtimeView?.historyNodes?.length || runtimeView?.historyBranches?.length
    );
    if (hasRuntimeHistoryState) {
      this.applyHistoryState(sessionId, {
        historyNodes: runtimeView?.historyNodes,
        historyBranches: runtimeView?.historyBranches,
        ...(runtimeHistoryCursor ? { ...runtimeHistoryCursor, sessionId } : {})
      });
    } else if (!hasHostManagedHistoryState(runtimeView) && restoredState) {
      this.applyHistoryState(sessionId, {
        historyNodes: restoredState.historyNodes,
        historyBranches: restoredState.historyBranches,
        activeBranchId: restoredState.activeBranchId,
        branchHeadNodeId: restoredState.branchHeadNodeId,
        visibleNodeId: restoredState.visibleNodeId,
        mode: restoredState.historyCursorMode
      });
    } else {
      this.applyHistoryState(sessionId, {
        historyNodes: runtimeView?.historyNodes,
        historyBranches: runtimeView?.historyBranches,
        ...(runtimeHistoryCursor ? { ...runtimeHistoryCursor, sessionId } : {})
      });
    }
    this.persistHistory();
  },
  async submitTurn(this: RuntimeStoreThis, options?: { images?: TurnInputImage[] }) {
    if (this.isSubmitting) {
      return false;
    }

    const optionsImages = (options?.images ?? []).map((image) => ({ ...image }));
    const readyAttachments = this.pendingAttachments.filter((attachment) => attachment.status === "ok");
    const pendingImages: TurnInputImage[] = readyAttachments
      .filter((attachment) => attachment.route === "image" && attachment.dataUrl != null)
      .map((attachment) => ({
        dataUrl: attachment.dataUrl as string,
        mimeType: attachment.mimeType,
        name: attachment.relativePath ?? attachment.name
      }));
    const images = [...pendingImages, ...optionsImages];
    const message = this.draftMessage.trim();
    const attachmentBlocks = buildAttachmentMessageBlocks(readyAttachments);

    if (
      !message.trim() &&
      !images.length &&
      !attachmentBlocks.text.length &&
      !attachmentBlocks.documents.length
    ) {
      return false;
    }

    // C-9：提交新消息时清空上一条中性反馈，避免过期文案滞留在状态栏。
    this.sessionNotice = null;

    this.isSubmitting = true;

    await this.initializeTurnEvents();
    const providerStore = useProviderStore();
    const settingsStore = useSettingsStore();
    const selectedProvider = providerStore.currentProvider;
    const selectedModel = providerStore.currentModel;
    const selectedProviderName = selectedProvider?.name ?? null;
    const selectedProviderProtocol = selectedProvider?.protocol ?? null;
    const selectedModelName = selectedModel?.model ?? selectedModel?.name ?? null;
    const selectedProviderTracePatch = {
      providerName: selectedProviderName,
      providerProtocol: selectedProviderProtocol,
      providerModel: selectedModelName
    };

    // After backend truncation, the cursor is always at the live branch head.
    // No restore is needed — the truncated state IS the current state.
    this.historyCursorMode = "live";
    this.initialRollbackActive = false;

    const lastAssistantMessage = [...this.messages]
      .reverse()
      .find((entry) => entry.role === "assistant") ?? null;
    const retryingAfterTimeout = isRetryableError(lastAssistantMessage?.errorDetail);
    const providerMessage = buildProviderUserMessageWithAttachments(message, images, attachmentBlocks);
    const displayMessage = buildDisplayedUserMessageWithAttachments(message, images, attachmentBlocks);
    const payload: TurnInput = {
      message: providerMessage,
      displayMessage,
      providerId: selectedProvider?.id ?? null,
      modelId: selectedModel?.id ?? null,
      reasoningEffort: providerStore.currentReasoningEffort ?? null,
      workspaceMode: settingsStore.workspaceMode,
      toolAuthorizationMode: this.toolAuthorizationMode ?? "workspace_write",
      sessionId: this.sessionId,
      nodeId: this.visibleNodeId,
      history: buildTurnHistory(this.messages),
      images,
      // 三级树 #3：冻结归属优先（创建时确定、随删除归一/切换水合同步）；
      // 列表投影次之（防旧快照漂移），激活态仅作历史兜底。
      workspaceId:
        this.sessionWorkspaceId ||
        this.sessionList.find((session) => session.conversationId === this.sessionId)?.workspaceId?.trim() ||
        this.activeWorkspaceId ||
        DEFAULT_WORKSPACE_ID
    };

    const requestId = String(Date.now());
    const userMessageId = `user-${requestId}`;

    this.messages.push({
      id: userMessageId,
      turnId: requestId,
      role: "user",
      content: displayMessage,
      attachments: [
        ...images.map((image, index) => ({
          id: `pending-${requestId}-${index + 1}`,
          name: image.name ?? null,
          mimeType: image.mimeType,
          relativePath: null,
          sizeBytes: image.dataUrl.length,
          createdAtMs: Date.now()
        })),
        ...buildAttachmentMetas(readyAttachments, requestId)
      ],
      status: "done",
      tokenCount: null
    });
    this.messageRevision = null;
    this.scheduleDeferredPersist();
    debugLog("submit", {
      turnId: requestId,
      messageLength: providerMessage.length,
      imageCount: images.length,
      messages: this.messages.length
    });

    this.isSubmitting = true;
    this.activeTurnStartedAtMs = Date.now();
    // Wave-59 B-59-1：提交即进入「本 hop 尚无输出」——
    // 不依赖上一轮的终态是否被消费到，也不等 turn:started，
    // 保证 awaiting-first-signal 窗口一律按「思考中」呈现（F1-2 / F4-2）。
    this.activeStreamKind = null;
    this.startSubmissionWatchdog(requestId);
    bindFrontendRecorderSession(this.sessionId);
    bindFrontendRecorderTurn(requestId);
    this.error = null;
    this.phase = "calling_model";
    this.activeTurnId = requestId;
    this.draftMessage = "";
    this.clearPendingAttachments();
    this.providerRequestedName = selectedProviderName ?? "";
    this.providerName = selectedProviderName ?? "";
    this.providerProtocol = selectedProviderProtocol ?? "";
    this.providerModel = selectedModelName ?? "";
    this.providerSource = "";
    this.providerMode = "";
    this.fallbackReason = null;
    this.inputTokens = null;
    this.outputTokens = null;
    this.totalTokens = null;
    this.firstTokenLatencyMs = null;
    // wave-metrics（task-4）：新轮起点清空上一轮结算快照（与 firstTokenLatencyMs 同一清理点），
    // 待本轮事件携带 metrics 再落库；无快照期间消费侧回退既有 trace 反推路径（I7）。
    this.liveMetrics = null;
    this.activeProviderCallRecords = [];
    this.traceSteps = createSubmitTraceSteps();
    this.publishTraceTimeline(applyProviderPatchToTraceTimeline(
      createDefaultTraceTimeline(),
      selectedProviderTracePatch
    ));
    this.toolActivities = [];
    this.commitTurnTraceTimeline(requestId, this.traceTimeline, {
      phase: "calling_model",
      traceSteps: this.traceSteps,
      toolActivities: [],
      providerRequestedName: selectedProviderName,
      providerName: selectedProviderName,
      providerProtocol: selectedProviderProtocol,
      providerModel: selectedModelName,
      providerSource: null,
      providerMode: null,
      fallbackReason: null,
      error: null
    }, false);
    // trace 初始记录不立即全量写 localStorage（用户消息已在上方 persistHistory 落盘），
    // 延迟合并写避免发送路径连续两次全量序列化阻塞主线程。
    this.scheduleDeferredPersist();

    if (retryingAfterTimeout) {
      const assistantMessage = this.ensureAssistantMessage(
        requestId,
        buildAssistantModelLabel(selectedProviderName, selectedModelName)
      );
      assistantMessage.content = TIMEOUT_RETRY_PENDING_MESSAGE;
      assistantMessage.reasoningContent = null;
      assistantMessage.status = "pending";
      assistantMessage.errorDetail = lastAssistantMessage?.errorDetail ?? DEFAULT_FAILED_TURN_ERROR;
    }

    try {
      await waitForNextPaint();
      debugLog("submit:user-painted", {
        turnId: requestId,
        messages: this.messages.length
      });

      if (!isTauriAvailable()) {
        await this.runBrowserPreviewTurn(requestId);
        return true;
      }

      const submission = await resolveSubmissionForStore(this);

      if (submission.command === "start_graph_run_stream") {
        const response = await safeInvoke<GraphRunStreamStartResponse>("start_graph_run_stream", {
          turnId: requestId,
          runId: null,
          goal: displayMessage,
          input: payload
        });
        applyStreamStartOutcome(this, response, null);
        return true;
      }

      if (submission.command === "resume_graph_run_stream") {
        const response = await safeInvoke<GraphRunStreamStartResponse>("resume_graph_run_stream", {
          turnId: requestId,
          runId: submission.runId,
          input: payload
        });
        applyStreamStartOutcome(this, response, submission.runId);
        return true;
      }

      const response = await safeInvoke<GraphRunStreamStartResponse>("continue_graph_run_stream", {
        turnId: requestId,
        runId: submission.runId,
        input: payload
      });
      applyStreamStartOutcome(this, response, submission.runId);
      return true;
    } catch (error) {
      const assistantMessage = this.ensureAssistantMessage(
        requestId,
        buildAssistantModelLabel(selectedProviderName, selectedModelName)
      );
      // wave-64 §2 F2：提交异常同样不得写占位正文（「本轮执行失败，请查看右侧 trace。」已废除）。
      // 失败终态由唯一的状态消息（`agentRuntimeStatus.mode === "error"`）承载，
      // 错误明细保留在 `errorDetail` 与右侧 Trace。
      assistantMessage.reasoningContent = null;
      assistantMessage.status = "error";
      assistantMessage.errorDetail = `本轮执行失败：${String(error)}`;
      assistantMessage.modelName = buildAssistantModelLabel(selectedProviderName, selectedModelName);
      this.error = `本轮执行失败：${String(error)}`;
      this.phase = "failed";
      this.activeTurnId = null;
      this.activeRunId = null;
      this.clearSubmissionWatchdog();
      this.traceSteps = createSubmitFailureTraceSteps();
      this.publishTraceTimeline(applyProviderPatchToTraceTimeline(
        createSubmitFailureTraceTimeline(),
        selectedProviderTracePatch
      ));
      this.commitTurnTraceTimeline(requestId, this.traceTimeline, {
        phase: "failed",
        traceSteps: this.traceSteps,
        toolActivities: this.toolActivities,
        providerRequestedName: selectedProviderName,
        providerName: selectedProviderName,
        providerProtocol: selectedProviderProtocol,
        providerModel: selectedModelName,
        error: this.error
      });
      this.persistHistory();
      return false;
    } finally {
      if (this.phase === "failed") {
        this.isSubmitting = false;
      }
    }
  },
  async stopTurn(this: RuntimeStoreThis) {
    if (!this.activeTurnId || !this.isSubmitting) {
      return false;
    }

    if (!isTauriAvailable()) {
      return this.cancelBrowserPreviewTurn(this.activeTurnId);
    }

    try {
      if (this.activeRunId) {
        const response = await safeInvoke<GraphRunControlResponse>("stop_graph_run", {
          runId: this.activeRunId
        });
        this.latestGraphRunControlBoundaryEvidence = cloneGraphRunControlBoundaryEvidence(
          response.controlBoundaryEvidence ? [response.controlBoundaryEvidence] : null
        );
        this.latestRunControlAuditSummary = cloneRunControlAuditSummary(
          response.runControlAuditSummary ?? null
        );
      } else {
        const stopResponse = await safeInvoke<StopTurnResponse | null>("stop_turn", {
          turnId: this.activeTurnId
        });
        // wave-60 §C1：`accepted=false`（not_found / already_terminal / 已 stopping）⇒ 不切 stopping。
        if (stopResponse && stopResponse.accepted === false) {
          return false;
        }
      }
      // wave-60 §C7 / NFR `stop_ack_latency_ms`：RPC 被接受后**同一微任务内**进入 stopping，
      // 不等终态事件（终态仍只由 turn:cancelled/completed/failed 驱动）。
      this.enterStoppingPhase();
      return true;
    } catch (error) {
      this.error = `停止当前轮次失败：${String(error)}`;
      return false;
    }
  },
  async switchSession(this: RuntimeStoreThis, nextSessionId: string) {
    if (this.sessionOperation && this.sessionOperation !== "switching") {
      return;
    }
    if (nextSessionId === this.sessionId) {
      return;
    }

    // 切换会话时清空待发送附件，防跨会话携带
    this.clearPendingAttachments();
    bindFrontendRecorderSession(nextSessionId);
    bindFrontendRecorderTurn(null);

    // 切换会话时同步刷新/过滤 pending ask，防止跨会话请求残留导致 CAS 冲突。
    // 静态导入而非 `await import()`：ask store 只依赖 lib/tauri，与 runtime 无环；
    // 动态导入一个同时被组件静态导入的模块不会分包，只会产生 INEFFECTIVE_DYNAMIC_IMPORT 噪音。
    try {
      void useAskStore().refresh({ sessionId: nextSessionId });
    } catch {
      // non-fatal
    }

    // Register current running turn as a background turn before switching away
    const switchingFromRunningTurn = this.isSubmitting && this.activeTurnId != null;
    if (switchingFromRunningTurn && this.activeTurnId) {
      this.runningSessionMap[this.sessionId] = {
        turnId: this.activeTurnId,
        phase: this.phase,
        textBuffer: this.streamBufferText,
        reasoningBuffer: this.streamBufferReasoning
      };
      delete this.completedSessionSet[this.sessionId];
      delete this.failedSessionSet[this.sessionId];
    }

    // B279：切走前先落定本会话的节流 timeline，再丢弃旧会话的定时器与流式缓冲——
    // 否则 setTimeout 会在切换后把旧会话增量写进新会话（跨会话串台）。
    // 运行中轮的缓冲已在上面的 runningSessionMap 登记里留存，可安全清空。
    this.flushPendingTraceTimeline();
    this.cancelStreamFlush();
    this.streamBufferTurnId = null;
    this.streamBufferText = "";
    this.streamBufferReasoning = "";

    this.persistHistory();
    const previousSnapshot = createSessionRuntimeSnapshot(this);
    const switchToken = this.sessionSwitchToken + 1;
    const switchStartedAt = typeof performance !== "undefined" ? performance.now() : Date.now();
    this.sessionSwitchToken = switchToken;
    this.sessionOperation = "switching";
    this.sessionError = null;

    const cachedPersistedState = loadPersistedRuntimeState(nextSessionId);
    const cachedRuntimeView = buildRuntimeViewFromPersistedState(
      nextSessionId,
      cachedPersistedState
    );
    const hasCachedState =
      Boolean(cachedPersistedState?.initialRollbackActive) ||
      cachedRuntimeView.session.history.length > 0 ||
      (cachedRuntimeView.session.attachmentAssets?.length ?? 0) > 0 ||
      cachedRuntimeView.session.summary.trim().length > 0;

    this.sessionHydrating = true;
    if (hasCachedState) {
      void this.loadSessionState(nextSessionId, {
        refreshCatalog: false,
        runtimeView: cachedRuntimeView
      }).catch((error) => {
        if (this.sessionSwitchToken !== switchToken || this.sessionId !== nextSessionId) {
          return;
        }
        this.sessionHydrating = false;
        this.sessionError = `切换对话失败：${String(error)}`;
        debugLog("session:switch:cache:error", {
          from: previousSnapshot.sessionId,
          to: nextSessionId,
          error: String(error)
        });
      });
      if (isTauriAvailable()) {
        this.loadSessionRuntimeViewState(nextSessionId).then((hostView) => {
          if (this.sessionSwitchToken !== switchToken || this.sessionId !== nextSessionId || !hostView) {
            return;
          }
          return this.loadSessionState(nextSessionId, {
            refreshCatalog: false,
            runtimeView: hostView,
            preserveMessages: hasCachedState
          }).catch(() => {});
        }).catch(() => {});
      }
    } else {
      this.resetSessionRuntimeState();
      this.sessionId = nextSessionId;
      this.phase = "connecting";
      this.sessionHydrating = true;
      const loadPromise = this.loadSessionState(nextSessionId, {
        refreshCatalog: false
      }).then(() => {
        if (this.sessionSwitchToken !== switchToken || this.sessionId !== nextSessionId) {
          return;
        }
        this.sessionHydrating = false;
        this.sessionError = null;
        debugLog("session:switch:no-cache:loaded", {
          from: previousSnapshot.sessionId,
          to: nextSessionId
        });
      });
      void withTimeout(loadPromise, HYDRATION_TIMEOUT_MS, "加载对话").catch((error) => {
        if (this.sessionSwitchToken !== switchToken || this.sessionId !== nextSessionId) {
          return;
        }
        this.sessionHydrating = false;
        this.sessionError = `切换对话失败：${String(error)}`;
        debugLog("session:switch:no-cache:error", {
          from: previousSnapshot.sessionId,
          to: nextSessionId,
          error: String(error)
        });
      });
    }

    delete this.completedSessionSet[nextSessionId];
    delete this.failedSessionSet[nextSessionId];

    const bgTurn = this.runningSessionMap[nextSessionId];
    if (bgTurn) {
      this.activeTurnId = bgTurn.turnId;
      this.isSubmitting = true;
      this.startSubmissionWatchdog(bgTurn.turnId);
      this.phase = bgTurn.phase;
      if (bgTurn.textBuffer) {
        this.streamBufferText = bgTurn.textBuffer;
        this.streamBufferTurnId = bgTurn.turnId;
      }
      if (bgTurn.reasoningBuffer) {
        this.streamBufferReasoning = bgTurn.reasoningBuffer;
      }
      // Wave-59 A-59-5：hop 级流式类型必须按**目标会话后台轮的真实缓冲**重新推导，
      // 不得沿用切走前会话的瞬时值（否则本 hop 无正文却判「正文流式」，状态条被隐藏）。
      this.activeStreamKind = bgTurn.textBuffer
        ? "content"
        : bgTurn.reasoningBuffer
          ? "reasoning"
          : null;
      delete this.runningSessionMap[nextSessionId];
    } else {
      // Wave-59 A-59-5：目标会话无后台运行轮 ⇒ 不携带任何 hop 级瞬时态。
      this.activeStreamKind = undefined;
    }

    // 三级树 #3：冻结归属随会话切换同步——切回旧会话时从目录投影恢复，
    // 保证 submitTurn 的 workspaceId 链首项始终代表当前会话的归属。
    this.sessionWorkspaceId =
      this.sessionList.find((session) => session.conversationId === nextSessionId)
        ?.workspaceId?.trim() || DEFAULT_WORKSPACE_ID;

    debugLog("session:switch", {
      from: previousSnapshot.sessionId,
      to: nextSessionId,
      backgroundTurnRegistered: switchingFromRunningTurn,
      restoredBackgroundTurn: Boolean(bgTurn),
      hydratedFromCache: hasCachedState
    });
    reportSwitchPerf("foreground-switched", {
      from: previousSnapshot.sessionId,
      to: nextSessionId,
      hasCachedState,
      restoredBackgroundTurn: Boolean(bgTurn),
      elapsedMs: (typeof performance !== "undefined" ? performance.now() : Date.now()) - switchStartedAt
    });

    this.sessionOperation = null;
    this.sessionHydrating = false;
    reportSwitchPerf("host-refresh-skipped", {
      to: nextSessionId,
      reason: hasCachedState ? "cached-session" : "interactive-switch-no-host-read"
    });
  },

  /**
   * Wave-12 契约（C1/C2/C7，用户裁定方案 A）：
   * 删除/归档「当前对话」后一律进入全新空白会话，不切换到任何历史会话（含已归档的）。
   *
   * 仅做落点重置：**禁止** persistHistory() / loadSessionState() / switchSession()——
   * 前者会把被删/归档会话重新落盘，后两者会把历史会话切回来。
   * resetSessionRuntimeState() 内含 cancelDeferredPersist()，保证旧会话不再被延迟落盘。
   */
  enterFreshSession(this: RuntimeStoreThis, targetWorkspaceId: string = DEFAULT_WORKSPACE_ID) {
    const targetWorkspace = targetWorkspaceId?.trim() || DEFAULT_WORKSPACE_ID;
    const previousSessionId = this.sessionId;
    const nextSessionId = createNextSessionId([
      this.sessionId,
      ...this.sessionList.map((session) => session.conversationId)
    ]);

    this.resetSessionRuntimeState();
    this.sessionId = nextSessionId;
    this.phase = "idle";
    this.sessionError = null;
    this.sessionWorkspaceId = targetWorkspace;
    // F12-1（遥测归属一致性）：与 switchSession 对齐，立即把 flight recorder 重绑到新会话，
    // 避免归档/删除后到下一次提交之间诊断事件仍归属已删除/已归档的 sessionId。
    bindFrontendRecorderSession(nextSessionId);
    bindFrontendRecorderTurn(null);

    const transientOverview: SessionOverview = {
      ...createTransientSessionOverview(nextSessionId),
      workspaceId: targetWorkspace === DEFAULT_WORKSPACE_ID ? null : targetWorkspace
    };
    this.sessionList = ensureUniqueSessionList([
      transientOverview,
      ...filterDeletingSessions(this.sessionList, this.deletingSessionSet).filter(
        (session) => session.conversationId !== nextSessionId
      )
    ]);

    debugLog("session:enter-fresh", {
      from: previousSessionId,
      to: nextSessionId,
      workspaceId: targetWorkspace
    });
  },
  async createSession(this: RuntimeStoreThis, targetWorkspaceId?: string) {
    // 新建会话意图即清空待发送附件（即使空消息 no-op 也清，避免残留携带）
    this.clearPendingAttachments();
    if (this.sessionOperation) {
      return;
    }
    const targetWorkspace = targetWorkspaceId?.trim() || DEFAULT_WORKSPACE_ID;
    if (!hasPersistableMessages(this.messages)) {
      // 当前已是空白新会话：若指定的目标工作区与当前不同，允许无缝重定向归属
      if (this.sessionWorkspaceId !== targetWorkspace) {
        this.sessionWorkspaceId = targetWorkspace;
        const currentProvisional = this.sessionList.find(
          (session) => session.conversationId === this.sessionId
        );
        if (currentProvisional) {
          currentProvisional.workspaceId =
            targetWorkspace === DEFAULT_WORKSPACE_ID ? null : targetWorkspace;
        }
      }
      return;
    }

    // Register current running turn as a background turn before resetting
    if (this.isSubmitting && this.activeTurnId) {
      this.runningSessionMap[this.sessionId] = {
        turnId: this.activeTurnId,
        phase: this.phase,
        textBuffer: this.streamBufferText,
        reasoningBuffer: this.streamBufferReasoning
      };
      delete this.completedSessionSet[this.sessionId];
      delete this.failedSessionSet[this.sessionId];
    }

    const nextSessionId = createNextSessionId([
      this.sessionId,
      ...this.sessionList.map((session) => session.conversationId)
    ]);
    const previousSessionId = this.sessionId;
    this.persistHistory();
    const currentOverview = buildSessionOverviewFromRuntimeState(this);
    this.resetSessionRuntimeState();
    this.sessionId = nextSessionId;
    this.phase = "idle";
    this.sessionError = null;
    const dedupedCurrentSessionList = filterDeletingSessions(
      this.sessionList.filter((session) => session.conversationId !== nextSessionId),
      this.deletingSessionSet
    );
    // 三级树（裁决①）：创建目标显式化——顶层入口落默认工作区，组内＋传该组 id；
    // 不再读取激活态。目标冻结到会话（submitTurn 以会话归属优先）。
    this.sessionWorkspaceId = targetWorkspace;
    const transientOverview = {
      ...createTransientSessionOverview(nextSessionId),
      workspaceId: targetWorkspace === DEFAULT_WORKSPACE_ID ? null : targetWorkspace
    };
    this.sessionList = ensureUniqueSessionList([
      transientOverview,
      ...(currentOverview && !this.deletingSessionSet[currentOverview.conversationId] ? [currentOverview] : []),
      ...dedupedCurrentSessionList.filter(
        (session) => session.conversationId !== currentOverview?.conversationId
          && session.conversationId !== previousSessionId
      )
    ]);
    debugLog("session:create:transient", {
      from: this.sessionList[1]?.conversationId ?? null,
      to: nextSessionId
    });
  },
  async initializeSessions(this: RuntimeStoreThis) {
    if (this.sessionOperation) {
      return;
    }

    const previousSnapshot = createSessionRuntimeSnapshot(this);
    this.sessionOperation = "initializing";
    this.sessionError = null;

    // 安全看门狗：防止初始化阶段任何 IPC 挂起导致输入区长期处于 disabled 锁死状态
    const watchdog = setTimeout(() => {
      if (this.sessionOperation === "initializing") {
        this.sessionOperation = null;
      }
    }, 3500);

    try {
      await this.loadSessionCatalog();
      const restoredRunningMap = loadRunningSessionMap();
      if (Object.keys(restoredRunningMap).length > 0) {
        this.runningSessionMap = { ...restoredRunningMap };
        debugLog("session:init:running-map", { sessions: Object.keys(restoredRunningMap) });
      }
      const persistedCache = loadPersistedRuntimeCache();
      if (persistedCache.completedSet) {
        this.completedSessionSet = { ...persistedCache.completedSet };
      }
      if (persistedCache.failedSet) {
        this.failedSessionSet = { ...persistedCache.failedSet };
      }
      // Wave-12.1 C8：冷启动落点必须跳过 archived 会话（catalog 含 archived 项且按 updatedAtMs 降序）。
      // 存在会话但全部已归档 → 不得打开任何已归档会话，直接进入全新空白会话。
      if (this.sessionList.length > 0 && !hasSelectableSession(this.sessionList)) {
        debugLog("session:init:all-archived", {
          sessionCount: this.sessionList.length
        });
        this.enterFreshSession();
        return;
      }

      const preferredSessionId = pickPreferredSessionId(this.sessionList, this.sessionId);
      const persisted = loadPersistedRuntimeState(preferredSessionId);
      const strategy = deriveInitStrategy(preferredSessionId, persisted, this.sessionList);

      debugLog("session:init:strategy", { strategy: strategy.kind, preferredSessionId });

      switch (strategy.kind) {
        case "local-cache": {
          const cachedRuntimeView = buildRuntimeViewFromPersistedState(preferredSessionId, strategy.persistedState);
          // PA-155：local-cache 的宿主刷新是有界增强项（同 :2696 语义），
          // 超时/失败一律降级为 null ⇒ mergeRuntimeViews 原样保留缓存视图，不得冒泡成 sessionError。
          const hostRuntimeView = await withTimeout(
            this.loadSessionRuntimeViewState(preferredSessionId),
            HYDRATION_TIMEOUT_MS,
            "加载对话"
          ).catch((error) => {
            debugLog("session:init:host:error", { sessionId: preferredSessionId, error: String(error) });
            return null;
          });
          const mergedRuntimeView = mergeRuntimeViews(cachedRuntimeView, hostRuntimeView);
          this.resetSessionRuntimeState();
          this.sessionId = preferredSessionId;
          this.phase = "connecting";
          await this.loadSessionState(preferredSessionId, {
            refreshCatalog: false,
            runtimeView: mergedRuntimeView
          });
          debugLog("session:init:cache", { sessionId: preferredSessionId, hostRefresh: hostRuntimeView != null });
          break;
        }
        case "host-read": {
          // PA-115：宿主读必须有界（同 :2141 语义），避免 IPC 挂起导致 initializeSessions 永不 settle。
          const hostRead = this.loadSessionRuntimeViewState(preferredSessionId);
          const runtimeView = await withTimeout(hostRead, HYDRATION_TIMEOUT_MS, "加载对话");
          if (!runtimeView?.checkpoint && runtimeView?.session.history.length === 0) {
            this.resetSessionRuntimeState();
            this.sessionId = preferredSessionId;
            this.phase = "idle";
            debugLog("session:init:empty");
            return;
          }
          this.resetSessionRuntimeState();
          this.sessionId = preferredSessionId;
          this.phase = "connecting";
          await this.loadSessionState(preferredSessionId, {
            refreshCatalog: false,
            runtimeView
          });
          debugLog("session:init:host", { sessionId: preferredSessionId, reason: strategy.reason });
          break;
        }
        case "empty-fallback": {
          this.resetSessionRuntimeState();
          this.sessionId = preferredSessionId;
          this.phase = "idle";
          debugLog("session:init:empty-fallback", { sessionId: preferredSessionId });
          break;
        }
      }
    } catch (error) {
      // PA-115 Gate-1 BLOCKING：草稿是用户输入而非可回滚状态，快照回滚不得覆盖它。
      const draftBeforeRestore = this.draftMessage;
      restoreSessionRuntimeSnapshot(this, previousSnapshot);
      if (draftBeforeRestore !== previousSnapshot.draftMessage) {
        this.draftMessage = draftBeforeRestore;
      }
      this.sessionError = `初始化对话失败：${String(error)}`;
      debugLog("session:init:error", {
        error: String(error)
      });
    } finally {
      clearTimeout(watchdog);
      this.sessionOperation = null;
    }
  },
  rollbackToInitialState(this: RuntimeStoreThis) {
    this.cancelDeferredPersist();
    this.cancelStreamFlush();
    this.phase = "idle";
    this.error = null;
    this.draftMessage = "";
    this.sessionSummary = "";
    this.retrievedContext = deriveRetrievedContextFromSnapshot({
      conversationId: this.sessionId,
      title: "新对话",
      summary: "",
      history: [],
      attachmentAssets: [],
      turnTraceHistory: [],
      historyStateEvidence: [],
      historyStateAuditSummary: null,
      runControlAuditSummary: null,
      turnCount: 0,
      lastReferencedFile: null,
      updatedAtMs: Date.now()
    });
    this.providerRequestedName = "";
    this.providerName = "";
    this.providerProtocol = "";
    this.providerModel = "";
    this.providerSource = "";
    this.providerMode = "";
    this.fallbackReason = null;
    this.inputTokens = null;
    this.outputTokens = null;
    this.totalTokens = null;
    this.firstTokenLatencyMs = null;
    // wave-metrics（task-4）：新轮起点清空上一轮结算快照（与 firstTokenLatencyMs 同一清理点），
    // 待本轮事件携带 metrics 再落库；无快照期间消费侧回退既有 trace 反推路径（I7）。
    this.liveMetrics = null;
    this.activeProviderCallRecords = [];
    this.clearSubmissionWatchdog();
    this.isSubmitting = false;
    this.activeTurnId = null;
    this.activeRunId = null;
    this.latestExecutionCheckpoint = null;
    this.latestGraphRunSubmissionPlan = null;
    this.latestGraphRunControlBoundaryEvidence = [];
    this.latestRunControlAuditSummary = null;
    this.latestHistoryStateAuditSummary = null;
    this.visibleNodeId = null;
    this.cursorVersion = null;
    this.messageRevision = null;
    this.historyCursorMode = "live";
    this.messages = [];
    this.activeBranchId = null;
    this.branchHeadNodeId = null;
    this.historyNodes = [];
    this.historyBranches = [];
    this.attachmentAssets = [];
    this.toolActivities = [];
    this.traceSteps = createDefaultTraceSteps();
    this.publishTraceTimeline(createDefaultTraceTimeline());
    this.turnTraceHistory = [];
    this.eventCursorByTurnId = {};
    this.streamBufferTurnId = null;
    this.streamFlushTimerId = null;
    this.streamBufferText = "";
    this.streamBufferReasoning = "";
    this.initialRollbackActive = true;
    this.persistHistory();
  },
  rollbackToTurnBoundary(this: RuntimeStoreThis, turnId: string) {
    const targetTurnId = turnId.trim();
    if (!targetTurnId) {
      return false;
    }

    let truncationIndex = -1;
    for (let i = this.messages.length - 1; i >= 0; i--) {
      if (this.messages[i]?.turnId === targetTurnId) {
        truncationIndex = i;
        break;
      }
    }
    if (truncationIndex < 0) {
      return false;
    }

    this.cancelDeferredPersist();
    this.cancelStreamFlush();
    this.messages = this.messages.slice(0, truncationIndex + 1);
    const retainedTurnIds = new Set(this.messages.map((message) => message.turnId));
    this.turnTraceHistory = this.turnTraceHistory.filter((trace) => retainedTurnIds.has(trace.turnId));
    this.eventCursorByTurnId = buildEventCursorByTurnTraceHistory(this.turnTraceHistory);
    this.traceSteps = createDefaultTraceSteps();
    const lastTrace = this.turnTraceHistory[this.turnTraceHistory.length - 1];
    this.publishTraceTimeline(
      lastTrace?.traceTimeline?.length
        ? cloneTraceTimeline(lastTrace.traceTimeline)
        : createDefaultTraceTimeline()
    );
    this.toolActivities = [];
    this.error = null;
    this.clearSubmissionWatchdog();
    this.isSubmitting = false;
    this.activeTurnId = null;
    this.activeRunId = null;
    this.latestExecutionCheckpoint = null;
    this.latestGraphRunSubmissionPlan = null;
    this.latestGraphRunControlBoundaryEvidence = [];
    this.latestRunControlAuditSummary = null;
    this.latestHistoryStateAuditSummary = null;
    this.messageRevision = null;
    this.initialRollbackActive = false;
    this.phase = this.messages.length > 0 ? "ready" : "idle";
    this.persistHistory();
    return true;
  },
  async checkoutHistoryNode(this: RuntimeStoreThis, nodeId: string, mode: HistoryCheckoutMode = "transcript_only", turnId?: string | null) {
    const sessionId = this.sessionId;
    if (!sessionId || !nodeId.trim()) {
      return null;
    }

    let result: HistoryCheckoutResult;
    if (isTauriAvailable()) {
      let payload: HistoryCheckoutWireResult;
      try {
        payload = await safeInvoke<HistoryCheckoutWireResult>("checkout_history_node", {
          sessionId,
          nodeId,
          mode,
          expectedCursorVersion: this.cursorVersion
        });
      } catch (error) {
        this.sessionError = `历史 checkout 冲突或失败：${String(error)}`;
        return null;
      }

      const messageCountBefore = this.messages.length;
      const targetNode = this.historyNodes.find((item) => item.nodeId === nodeId) ?? null;
      const targetIsInitialState = targetNode ? !targetNode.turnId?.trim() : false;
      const previewMessages = previewMessageStateDeltaMessages(
        this.messages,
        payload.messageDelta ?? null,
        this.sessionId,
        this.messageRevision
      );
      const shouldSkipBlankIntermediateDelta =
        !targetIsInitialState &&
        messageCountBefore > 0 &&
        previewMessages !== null &&
        previewMessages.length === 0;
      const deltaApplied = shouldSkipBlankIntermediateDelta
        ? false
        : this.applyMessageStateDelta(payload.messageDelta ?? null);

      if (!deltaApplied) {
        const preservedDraft = this.draftMessage;
        await this.loadSessionState(sessionId, {
          refreshCatalog: false,
          nodeId
        });
        this.draftMessage = preservedDraft;
      }

      // Safety net: when a non-initial checkout unexpectedly empties the
      // transcript, force one more host round-trip. An initial/root checkout
      // is expected to clear messages, so reloading there only causes a
      // visible empty-then-rehydrate jump.
      if (this.messages.length === 0 && messageCountBefore > 0 && !targetIsInitialState) {
        debugLog("checkout:safety-reload", {
          sessionId,
          nodeId,
          messageCountBefore,
          deltaApplied
        });
        const preservedDraft = this.draftMessage;
        await this.loadSessionState(sessionId, {
          refreshCatalog: false,
          nodeId: null
        });
        this.draftMessage = preservedDraft;
      }

      this.historyCursorMode = "live";
      this.initialRollbackActive = false;

      result = normalizeHistoryCheckoutResult(payload, this.historyNodes, this.historyBranches);
    } else {
      const targetTurnId = this.findCheckpointTurnIdByNodeId(nodeId);
      const resolvedTurnId = targetTurnId?.trim() || turnId?.trim() || null;
      let truncationIndex = -1;
      const targetNode = this.historyNodes.find((item) => item.nodeId === nodeId) ?? null;
      const targetIsInitialState = !targetNode?.turnId?.trim();
      if (resolvedTurnId) {
        for (let i = this.messages.length - 1; i >= 0; i--) {
          if ((this.messages[i] as ChatMessage).turnId === resolvedTurnId) {
            truncationIndex = i;
            break;
          }
        }
      }
      if (targetIsInitialState) {
        this.rollbackToInitialState();
        this.initialRollbackActive = false;
      } else if (truncationIndex >= 0) {
        this.messages = this.messages.slice(0, truncationIndex + 1);
        this.turnTraceHistory = this.turnTraceHistory.filter(
          (trace) => this.messages.some((msg) => msg.turnId === trace.turnId)
        );
        this.eventCursorByTurnId = buildEventCursorByTurnTraceHistory(this.turnTraceHistory);
        this.traceSteps = createDefaultTraceSteps();
        const lastTrace = this.turnTraceHistory[this.turnTraceHistory.length - 1];
        this.publishTraceTimeline(
          lastTrace?.traceTimeline?.length
            ? cloneTraceTimeline(lastTrace.traceTimeline)
            : createDefaultTraceTimeline()
        );
        // 1-branch 模式：撤回后 checkout 的 checkpoint 成为新的当前状态
        this.branchHeadNodeId = nodeId;
        this.historyCursorMode = "live";
        this.initialRollbackActive = false;
        // 撤回完成后过滤 historyNodes：只保留目标节点及其祖先节点
        const ancestorIds = new Set<string>();
        const collectAncestors = (startId: string) => {
          let currentId: string | null = startId;
          while (currentId) {
            ancestorIds.add(currentId);
            const node = this.historyNodes.find((n) => n.nodeId === currentId);
            currentId = node?.parentNodeId?.trim() || null;
          }
        };
        collectAncestors(nodeId);
        this.historyNodes = this.historyNodes.filter((n) => ancestorIds.has(n.nodeId));
      }

      result = {
        sessionId,
        nodeId,
        visibleNodeId: nodeId,
        activeBranchId: this.activeBranchId,
        branchHeadNodeId: nodeId,
        workspaceNodeId: this.visibleNodeId,
        mode: "live",
        requestedMode: mode,
        appliedMode: "transcript_only",
        transcriptRestoreApplied: true,
        workspaceRollbackCapable: false,
        workspaceRestoreCapable: false,
        workspaceRollbackApplied: false,
        degraded: mode === "transcript_and_workspace",
        degradedToTranscriptOnly: mode === "transcript_and_workspace",
        degradationReason:
          mode === "transcript_and_workspace" ? "workspace_rollback_unsupported" : null,
        historyStateEvidence: null,
        historyStateAuditSummary: null,
        historyNodes: this.historyNodes,
        historyBranches: this.historyBranches
      };
    }

    this.applyHistoryState(sessionId, result);
    this.latestHistoryStateAuditSummary = cloneHistoryStateAuditSummary(
      result.historyStateAuditSummary ?? null
    );
    // checkout 完成后持久化完整状态，确保崩溃恢复时仍处于撤回后的视图
    this.persistHistory();
    return result;
  },
  async restoreBranchHead(this: RuntimeStoreThis, branchId?: string | null) {
    const targetBranchId = branchId?.trim() || this.activeBranchId;
    if (!this.sessionId || !targetBranchId) {
      return null;
    }

    if (!isTauriAvailable()) {
      this.sessionError = previewHistoryMutationUnavailable();
      return null;
    }

    const result = await this.performHistoryOperation({
      cmd: "restore_branch_head",
      invokeArgs: { branchId: targetBranchId },
      normalize: (payload) => normalizeHistoryRestoreResult(payload, this.historyNodes, this.historyBranches),
      selectNodeId: (payload) => payload.restoredNodeId ?? payload.cursor?.visibleNodeId ?? null,
      errorMessagePrefix: "恢复 branch head"
    });
    if (result) {
      this.historyCursorMode = "live";
      this.clearSubmissionWatchdog();
      this.activeTurnId = null;
      this.activeRunId = null;
      this.isSubmitting = false;
      this.latestExecutionCheckpoint = null;
    }
    return result;
  },
  applyExecutionCheckpoint(this: RuntimeStoreThis, 
    checkpoint: ExecutionCheckpoint | null,
    persistedMessages?: ChatMessage[] | null
  ) {
    this.latestExecutionCheckpoint = checkpoint ? { ...checkpoint } : null;
    if (!checkpoint) {
      return;
    }

    const restoredTurnMessages = (persistedMessages ?? [])
      .filter((message) => message.turnId === checkpoint.turnId)
      .map((message) => ({
        ...message,
        attachments: message.attachments?.map((attachment) => ({ ...attachment })) ?? []
      }));

    if (restoredTurnMessages.length > 0) {
      this.messages = [
        ...this.messages.filter((message) => message.turnId !== checkpoint.turnId),
        ...restoredTurnMessages
      ];
    }

    if (!this.messages.some((msg) => msg.turnId === checkpoint.turnId)) {
      return;
    }

    const checkpointStatus = checkpoint.status.trim().toLowerCase();
    const isRecoveryCheckpoint = checkpoint.checkpointKind === "recovery";
    if (!isRecoveryCheckpoint && checkpointStatus !== "running") {
      return;
    }

    const modelLabel = buildAssistantModelLabel(checkpoint.providerName, checkpoint.providerModel);
    const assistantMessage =
      this.messages.find((message) => message.turnId === checkpoint.turnId && message.role === "assistant") ??
      this.ensureAssistantMessage(checkpoint.turnId, modelLabel);
    assistantMessage.status = isRecoveryCheckpoint ? "done" : "pending";
    assistantMessage.modelName = modelLabel;

    const checkpointRunId = checkpoint.runId?.trim() || null;
    if (checkpointRunId) {
      this.activeRunId = checkpointRunId;
    }
    this.phase = normalizeCheckpointPhase(checkpoint);
    this.error = checkpoint.error ?? null;
    this.isSubmitting = !isRecoveryCheckpoint;
    this.activeTurnId = isRecoveryCheckpoint ? null : checkpoint.turnId;
    this.providerRequestedName = checkpoint.providerRequestedName ?? this.providerRequestedName;
    this.providerName = checkpoint.providerName ?? this.providerName;
    this.providerProtocol = checkpoint.providerProtocol ?? this.providerProtocol;
    this.providerModel = checkpoint.providerModel ?? this.providerModel;
    this.providerSource = checkpoint.providerSource ?? this.providerSource;
    this.providerMode = checkpoint.providerMode ?? this.providerMode;
    this.fallbackReason = checkpoint.fallbackReason ?? this.fallbackReason;
    this.traceSteps = checkpoint.traceSteps.length > 0 ? checkpoint.traceSteps : this.traceSteps;
    this.toolActivities = checkpoint.toolActivities;
    const checkpointTimeline = cloneTraceTimeline(
      this.turnTraceHistory.find((trace) => trace.turnId === checkpoint.turnId)?.traceTimeline
    );
    this.publishTraceTimeline(checkpointTimeline.length ? checkpointTimeline : createDefaultTraceTimeline());
    this.upsertTurnTrace(checkpoint.turnId, {
      phase: this.phase,
      traceSteps: this.traceSteps,
      traceTimeline: this.traceTimeline,
      toolActivities: this.toolActivities,
      providerRequestedName: this.providerRequestedName,
      providerName: this.providerName,
      providerProtocol: this.providerProtocol,
      providerModel: this.providerModel,
      providerSource: this.providerSource,
      providerMode: this.providerMode,
      fallbackReason: this.fallbackReason,
      error: checkpoint.error ?? null,
      updatedAt: checkpoint.updatedAtMs
    });
    debugLog("checkpoint:applied", {
      sessionId: this.sessionId,
      turnId: checkpoint.turnId,
      phase: this.phase,
      checkpointKind: checkpoint.checkpointKind
    });
  },
  applyMessageStateDelta(this: RuntimeStoreThis, delta?: MessageStateDelta | null) {
    if (!delta) {
      return false;
    }
    const nextMessages = previewMessageStateDeltaMessages(
      this.messages,
      delta,
      this.sessionId,
      this.messageRevision
    );
    if (!nextMessages) {
      return false;
    }

    this.messages = reuseStableChatMessages(this.messages, nextMessages);
    this.messageRevision = delta.targetRevision;
    const retainedTurnIds = new Set(this.messages.map((message) => message.turnId));
    this.turnTraceHistory = this.turnTraceHistory.filter((trace) => retainedTurnIds.has(trace.turnId));
    this.eventCursorByTurnId = buildEventCursorByTurnTraceHistory(this.turnTraceHistory);
    this.traceSteps = createDefaultTraceSteps();
    const lastTrace = this.turnTraceHistory[this.turnTraceHistory.length - 1];
    this.publishTraceTimeline(
      lastTrace?.traceTimeline?.length
        ? cloneTraceTimeline(lastTrace.traceTimeline)
        : createDefaultTraceTimeline()
    );
    this.toolActivities = [];
    this.error = null;
    this.clearSubmissionWatchdog();
    this.isSubmitting = false;
    this.activeTurnId = null;
    this.activeRunId = null;
    this.latestExecutionCheckpoint = null;
    return true;
  },
  applyOutputEnd(this: RuntimeStoreThis, payload: TurnStreamEvent) {
    this.flushBufferedStreamText(payload.turnId);

    const assistantMessage = this.ensureAssistantMessage(
      payload.turnId,
      buildAssistantModelLabel(payload.providerName, payload.providerModel)
    );

    const finalText = payload.text?.trim();
    if (finalText && payload.text !== assistantMessage.content) {
      assistantMessage.content = payload.text ?? assistantMessage.content;
      this.messageRevision = null;
    } else if (assistantMessage.content === TIMEOUT_RETRY_PENDING_MESSAGE) {
      assistantMessage.content = payload.text ?? "";
      this.messageRevision = null;
    }
    assistantMessage.errorDetail = null;
    const nextReasoningContent = normalizeReasoningContent(
      payload.reasoningContent ?? assistantMessage.reasoningContent ?? null
    );
    if (nextReasoningContent !== assistantMessage.reasoningContent) {
      assistantMessage.reasoningContent = nextReasoningContent;
      this.messageRevision = null;
    }
    assistantMessage.status = "done";
    assistantMessage.modelName = buildAssistantModelLabel(payload.providerName, payload.providerModel);
    this.messageRevision = null;

    this.providerRequestedName = payload.providerRequestedName ?? this.providerRequestedName;
    this.providerName = payload.providerName ?? this.providerName;
    this.providerProtocol = payload.providerProtocol ?? this.providerProtocol;
    this.providerModel = payload.providerModel ?? this.providerModel;
    this.providerSource = payload.providerSource ?? this.providerSource;
    this.providerMode = payload.providerMode ?? this.providerMode;
    this.fallbackReason = payload.fallbackReason ?? this.fallbackReason;
    // R4/Wave-15：活跃轮 input/output 口径改为该次调用的 per-call 值（无记录时保持既有值）。
    this.applyActiveProviderCallRecords(payload.providerCallRecords);
    this.totalTokens = payload.totalTokens ?? this.totalTokens;
    this.firstTokenLatencyMs = payload.firstTokenLatencyMs ?? this.firstTokenLatencyMs;
    this.applyTurnTokenStats(payload.turnId, payload.inputTokens, payload.outputTokens, false);
    this.scheduleDeferredPersist(OUTPUT_END_PERSIST_DELAY_MS);
  },
  async fetchHealth(this: RuntimeStoreThis) {
    if (this.health) {
      return;
    }

    this.phase = "connecting";
    this.error = null;

    try {
      const payload: HealthPayload = isTauriAvailable()
        ? await safeInvoke<HealthPayload>("health_check")
        : {
            appName: "Pony Agent",
            appVersion: "dev-preview",
            runtime: "browser-preview",
            graphEngine: "mock-stream",
            graphContractVersion: "browser-preview"
          };
      this.health = payload;
      this.phase = "completed";
      initFrontendFlightRecorder();
      debugLog("health:ok", {
        runtime: payload.runtime,
        graph: payload.graphEngine
      });
      this.traceSteps = this.traceSteps.map((step) =>
        step.id === "step-context"
          ? { ...step, state: "completed" }
          : step
      );
    } catch (error) {
      this.error = `Rust 后端连接失败：${String(error)}`;
      this.phase = "failed";
      debugLog("health:error", {
        error: String(error)
      });
    }
  },
  async runBrowserPreviewTurn(this: RuntimeStoreThis, requestId: string) {
    const providerStore = useProviderStore();
    const provider = providerStore.currentProvider;
    const model = providerStore.currentModel;
    this.browserPreviewRunToken += 1;
    const runToken = this.browserPreviewRunToken;
    const previewProviderName = provider?.name ?? BROWSER_PREVIEW_PROVIDER_NAME;
    const previewModelName = model?.model ?? model?.name ?? BROWSER_PREVIEW_MODEL_NAME;
    const assistantModelLabel = buildAssistantModelLabel(previewProviderName, previewModelName);

    this.providerRequestedName = previewProviderName;
    debugLog("browser-preview:start", {
      turnId: requestId
    });
    this.providerName = previewProviderName;
    this.providerProtocol = provider?.protocol ?? "openai-completions";
    this.providerModel = previewModelName;
    this.providerSource = "browser_preview";
    this.providerMode = "browser_preview";
    this.inputTokens = null;
    this.outputTokens = null;
    this.totalTokens = null;
    this.firstTokenLatencyMs = null;
    // wave-metrics（task-4）：新轮起点清空上一轮结算快照（与 firstTokenLatencyMs 同一清理点），
    // 待本轮事件携带 metrics 再落库；无快照期间消费侧回退既有 trace 反推路径（I7）。
    this.liveMetrics = null;
    this.activeProviderCallRecords = [];
    this.fallbackReason = BROWSER_PREVIEW_FALLBACK_REASON;

    await wait(120);
    if (runToken !== this.browserPreviewRunToken || this.activeTurnId !== requestId) {
      return;
    }
    const assistantMessage = this.ensureAssistantMessage(requestId, assistantModelLabel);
    assistantMessage.content = "";

    for (const chunk of BROWSER_PREVIEW_CHUNKS) {
      await wait(80);
      if (runToken !== this.browserPreviewRunToken || this.activeTurnId !== requestId) {
        return;
      }
      assistantMessage.content += chunk;
      this.scheduleDeferredPersist();
    }

    assistantMessage.status = "done";
    assistantMessage.modelName = assistantModelLabel;
    assistantMessage.tokenCount = null;

    this.phase = "completed";
    this.sessionSummary = BROWSER_PREVIEW_SESSION_SUMMARY;
    this.traceSteps = createBrowserPreviewTraceSteps();
    this.publishTraceTimeline(createBrowserPreviewTraceTimeline());
    this.toolActivities = [];
    const terminalSequence = this.traceTimeline[this.traceTimeline.length - 1]?.sequence ?? this.traceSteps.length;
    const terminalEnvelope = createBrowserPreviewTerminalEnvelope(
      requestId,
      "turn.completed",
      terminalSequence,
      Date.now()
    );
    this.commitTurnEventCursor({
      turnId: requestId,
      eventId: terminalEnvelope.eventId,
      sequence: terminalEnvelope.sequence,
      emittedAtMs: terminalEnvelope.emittedAtMs
    });
    this.commitTurnTraceTimeline(requestId, this.traceTimeline, {
      eventId: terminalEnvelope.eventId,
      eventType: terminalEnvelope.eventType,
      eventVersion: terminalEnvelope.eventVersion,
      sequence: terminalEnvelope.sequence,
      emittedAtMs: terminalEnvelope.emittedAtMs,
      phase: "completed",
      traceSteps: this.traceSteps,
      toolActivities: [],
      sessionSummary: this.sessionSummary,
      fallbackReason: this.fallbackReason,
      title:
        this.messages.find((item) => item.turnId === requestId && item.role === "user")?.content
          ? buildTurnTitle(this.messages.find((item) => item.turnId === requestId && item.role === "user")?.content ?? "")
          : BROWSER_PREVIEW_TRACE_TITLE,
      error: null
    });
    this.persistHistory();
    void this.loadSessionCatalog();
    debugLog("browser-preview:completed", {
      turnId: requestId
    });
    this.clearSubmissionWatchdog();
    this.isSubmitting = false;
    this.activeTurnId = null;
    this.activeRunId = null;
  },
  cancelBrowserPreviewTurn(this: RuntimeStoreThis, turnId: string) {
    if (this.activeTurnId !== turnId) {
      return false;
    }

    this.browserPreviewRunToken += 1;
    const cancelledTraceSteps = finalizeCancelledTraceSteps(this.traceSteps);
    const assistantMessage = this.ensureAssistantMessage(
      turnId,
      buildAssistantModelLabel(this.providerName, this.providerModel)
    );
    // wave-64 §2 F2：取消**不得**另起消息 —— 禁止向助手消息正文写占位（如「本轮已停止。」）。
    // 终态由唯一的状态消息（`agentRuntimeStatus.mode === "stopped"`）承载，正文保持已产出内容。
    assistantMessage.reasoningContent = null;
    assistantMessage.status = "done";

    const traceTimeline: TraceTimelineEntry[] = createBrowserPreviewTraceTimeline().map((entry) =>
      entry.kind === "call_model"
        ? {
            ...entry,
            state: "cancelled" as const,
            text: assistantMessage.content,
            fallbackReason: this.fallbackReason
          }
        : entry
    );

    this.phase = "cancelled";
    this.error = null;
    this.traceSteps = cancelledTraceSteps;
    this.publishTraceTimeline(traceTimeline);
    this.toolActivities = [];
    const terminalSequence = traceTimeline[traceTimeline.length - 1]?.sequence ?? cancelledTraceSteps.length;
    const terminalEnvelope = createBrowserPreviewTerminalEnvelope(
      turnId,
      "turn.cancelled",
      terminalSequence,
      Date.now()
    );
    this.commitTurnEventCursor({
      turnId,
      eventId: terminalEnvelope.eventId,
      sequence: terminalEnvelope.sequence,
      emittedAtMs: terminalEnvelope.emittedAtMs
    });
    this.commitTurnTraceTimeline(turnId, traceTimeline, {
      eventId: terminalEnvelope.eventId,
      eventType: terminalEnvelope.eventType,
      eventVersion: terminalEnvelope.eventVersion,
      sequence: terminalEnvelope.sequence,
      emittedAtMs: terminalEnvelope.emittedAtMs,
      phase: "cancelled",
      traceSteps: cancelledTraceSteps,
      toolActivities: [],
      sessionSummary: this.sessionSummary,
      fallbackReason: this.fallbackReason,
      title:
        this.messages.find((item) => item.turnId === turnId && item.role === "user")?.content
          ? buildTurnTitle(this.messages.find((item) => item.turnId === turnId && item.role === "user")?.content ?? "")
          : BROWSER_PREVIEW_TRACE_TITLE,
      error: "stopped_by_user"
    });
    this.persistHistory();
    void this.loadSessionCatalog();
    this.clearSubmissionWatchdog();
    this.isSubmitting = false;
    this.activeTurnId = null;
    this.activeRunId = null;
    debugLog("browser-preview:cancelled", {
      turnId
    });
    return true;
  },
  handleSubmissionWatchdogTimeout(this: RuntimeStoreThis, turnId: string) {
    this.submissionWatchdogTimerId = null;
    if (!this.isSubmitting || this.activeTurnId !== turnId) {
      return;
    }
    debugLog("watchdog:submission-timeout", {
      turnId,
      timeoutMs: SUBMISSION_WATCHDOG_TIMEOUT_MS
    });
    // 兜底（RC3）：优先收敛同一 turn 下已存在的 assistant 消息（兼容既有消息 id
    // 命名，如 `${turnId}-assistant`）；不存在时用 ensureAssistantMessage 创建兜底消息
    // （后端从未 emit turn:started 的场景），再统一收敛为 error + 兜底文本，使其通过
    // buildTurnHistory 过滤（status!=='pending' && content 非空）进入历史与模型上下文。
    const assistantMessage =
      this.messages.find((message) => message.turnId === turnId && message.role === "assistant") ??
      this.ensureAssistantMessage(
        turnId,
        buildAssistantModelLabel(this.providerName, this.providerModel)
      );
    assistantMessage.status = "error";
    assistantMessage.errorDetail = "submission_watchdog_timeout";
    // 保持消息流纯净：严禁向助手消息内容注入工程内部看门狗文案，保持已接收流式片段或留空
    if (!assistantMessage.content) {
      assistantMessage.content = "";
    }
    this.messageRevision = null;

    // Trace 终态收敛：收敛 traceSteps 与 traceTimeline，将进行态标记为 error
    this.traceSteps = createSubmitFailureTraceSteps();
    const baseFailureTimeline = this.traceTimeline && this.traceTimeline.length > 0
      ? cloneTraceTimeline(this.traceTimeline)
      : createSubmitFailureTraceTimeline();

    let updatedCallModel = false;
    for (let i = baseFailureTimeline.length - 1; i >= 0; i--) {
      const entry = baseFailureTimeline[i];
      if (entry && (entry.kind === "call_model" || entry.kind === "call_tool")) {
        if (entry.state === "active" || entry.state === "pending") {
          entry.state = "error";
          entry.error = "submission_watchdog_timeout";
          updatedCallModel = true;
        }
      }
    }
    if (!updatedCallModel && baseFailureTimeline.length > 0) {
      const lastEntry = baseFailureTimeline[baseFailureTimeline.length - 1];
      if (lastEntry) {
        lastEntry.state = "error";
        lastEntry.error = "submission_watchdog_timeout";
      }
    }

    const failurePatch = {
      providerName: this.providerName || null,
      providerProtocol: this.providerProtocol || null,
      providerModel: this.providerModel || null
    };
    const patchedTimeline = applyProviderPatchToTraceTimeline(baseFailureTimeline, failurePatch);
    this.publishTraceTimeline(patchedTimeline);
    this.commitTurnTraceTimeline(turnId, this.traceTimeline, {
      phase: "failed",
      traceSteps: this.traceSteps,
      toolActivities: this.toolActivities,
      providerRequestedName: this.providerRequestedName,
      providerName: this.providerName,
      providerProtocol: this.providerProtocol,
      providerModel: this.providerModel,
      error: "submission_watchdog_timeout"
    });

    // 尝试向后端发出取消/停止指令，避免后端孤儿任务持续占用。
    // 先做调用时点快照，避免下方状态复位后取到 null。
    const activeRunIdToStop = this.activeRunId;
    const watchdogSessionId = this.sessionId ?? null;
    const watchdogRunId = this.activeRunId ?? null;
    if (isTauriAvailable()) {
      try {
        // RC2：fire-and-forget 调用后端 fail_turn_for_watchdog 命令，使后端持久化 failed
        // trace（后端权威 trace 落盘）。后端不可用/失败不得影响前端解锁；保留既有
        // stop_graph_run/stop_turn 调用不变（契约边界：不停用旧命令）。
        const failRes = safeInvoke("fail_turn_for_watchdog", {
          turnId,
          sessionId: watchdogSessionId,
          runId: watchdogRunId,
          error: "submission_watchdog_timeout"
        });
        if (failRes && typeof (failRes as Promise<unknown>).catch === "function") {
          void (failRes as Promise<unknown>).catch(() => {});
        }

        if (activeRunIdToStop) {
          const res = safeInvoke("stop_graph_run", { runId: activeRunIdToStop });
          if (res && typeof (res as Promise<unknown>).catch === "function") {
            void (res as Promise<unknown>).catch(() => {});
          }
        } else {
          const res = safeInvoke("stop_turn", { turnId });
          if (res && typeof (res as Promise<unknown>).catch === "function") {
            void (res as Promise<unknown>).catch(() => {});
          }
        }
      } catch {
        // ignore
      }
    }

    this.isSubmitting = false;
    this.activeTurnId = null;
    this.activeRunId = null;
    this.phase = "failed";
    this.error = "响应超时，已停止等待";
    this.persistHistory();
  },
  persistHistory(this: RuntimeStoreThis) {
    this.cancelDeferredPersist();
    if (!hasPersistableMessages(this.messages) && !this.initialRollbackActive) {
      removePersistedSessionState(this.sessionId);
      debugLog("persist:skip-empty", {
        sessionId: this.sessionId
      });
      return;
    }

    const payload: PersistedRuntimeState = {
      cachedStateVersion: CACHED_STATE_VERSION,
      phase: this.phase,
      canonicalTerminalPhase: !this.initialRollbackActive && isTerminalPhase(this.phase)
        ? this.phase
        : undefined,
      messages: this.messages,
      attachmentAssets: this.attachmentAssets,
      // turnTraceHistory intentionally excluded — trace data lives on the backend
      sessionSummary: this.sessionSummary,
      providerRequestedName: this.providerRequestedName,
      providerName: this.providerName,
      providerProtocol: this.providerProtocol,
      providerModel: this.providerModel,
      providerSource: this.providerSource,
      providerMode: this.providerMode,
      fallbackReason: this.fallbackReason,
      inputTokens: this.inputTokens,
      outputTokens: this.outputTokens,
      totalTokens: this.totalTokens,
      firstTokenLatencyMs: this.firstTokenLatencyMs,
      visibleNodeId: this.visibleNodeId,
      branchHeadNodeId: this.branchHeadNodeId,
      activeBranchId: this.activeBranchId,
      historyCursorMode: this.historyCursorMode,
      historyNodes: cloneHistoryNodes(this.historyNodes),
      historyBranches: cloneHistoryBranches(this.historyBranches),
      cursorVersion: this.cursorVersion,
      initialRollbackActive: this.initialRollbackActive,
      checkpoint: this.latestExecutionCheckpoint ? { ...this.latestExecutionCheckpoint } : null,
      runningTurnId: this.activeTurnId,
      // PA-081：会话归属随持久化往返（浏览器恢复保持分组稳定）。
      sessionWorkspaceId: this.sessionWorkspaceId
    };

    try {
      persistSessionStateAndRuntimeMaps(
        this.sessionId,
        payload,
        this.runningSessionMap,
        this.completedSessionSet,
        this.failedSessionSet
      );
      debugLog("persist", {
        sessionId: this.sessionId,
        messages: this.messages.length,
        traces: this.turnTraceHistory.length,
        phase: this.phase,
        hasCheckpoint: !!this.latestExecutionCheckpoint,
        hasRunningTurn: !!this.activeTurnId,
        runningSessionCount: Object.keys(this.runningSessionMap).length
      });
    } catch {
      // Ignore storage failures and keep runtime in memory.
      debugLog("persist:error");
    }
  },
  updatePersistedBackgroundSession(this: RuntimeStoreThis, 
    sessionId: string,
    turnId: string,
    patch: {
      content?: string | null;
      reasoningContent?: string | null;
      status: ChatMessage["status"];
      modelName?: string | null;
      tokenCount?: number | null;
      phase: RuntimePhase;
      sessionSummary?: string | null;
      providerRequestedName?: string | null;
      providerName?: string | null;
      providerProtocol?: string | null;
      providerModel?: string | null;
      providerSource?: string | null;
      providerMode?: string | null;
      fallbackReason?: string | null;
      inputTokens?: number | null;
      outputTokens?: number | null;
      totalTokens?: number | null;
      firstTokenLatencyMs?: number | null;
      errorDetail?: string | null;
    }
  ) {
    const persisted = loadPersistedRuntimeState(sessionId);
    if (!persisted) {
      return;
    }

    const messages = persisted.messages.map((message) => ({
      ...message,
      attachments: message.attachments?.map((attachment) => ({ ...attachment })) ?? []
    }));
    let assistantMessage = messages.find(
      (message) => message.turnId === turnId && message.role === "assistant"
    );

    if (!assistantMessage) {
      assistantMessage = {
        id: `assistant-${turnId}`,
        turnId,
        role: "assistant",
        content: "",
        attachments: [],
        reasoningContent: null,
        status: patch.status,
        tokenCount: null,
        modelName: patch.modelName?.trim() || undefined
      };
      messages.push(assistantMessage);
    }

    const nextContent = patch.content?.trim();
    if (nextContent) {
      assistantMessage.content = patch.content ?? assistantMessage.content;
    }
    assistantMessage.reasoningContent = normalizeReasoningContent(
      patch.reasoningContent ?? assistantMessage.reasoningContent ?? null
    );
    assistantMessage.status = patch.status;
    assistantMessage.modelName = patch.modelName?.trim() || assistantMessage.modelName;
    assistantMessage.tokenCount = patch.tokenCount ?? assistantMessage.tokenCount ?? null;
    assistantMessage.errorDetail = patch.errorDetail ?? assistantMessage.errorDetail ?? null;

    persistSessionState(sessionId, {
      ...persisted,
      phase: patch.phase,
      canonicalTerminalPhase: isTerminalPhase(patch.phase)
        ? patch.phase
        : persisted.canonicalTerminalPhase,
      messages,
      sessionSummary: patch.sessionSummary ?? persisted.sessionSummary,
      providerRequestedName: patch.providerRequestedName ?? persisted.providerRequestedName,
      providerName: patch.providerName ?? persisted.providerName,
      providerProtocol: patch.providerProtocol ?? persisted.providerProtocol,
      providerModel: patch.providerModel ?? persisted.providerModel,
      providerSource: patch.providerSource ?? persisted.providerSource,
      providerMode: patch.providerMode ?? persisted.providerMode,
      fallbackReason:
        patch.fallbackReason === undefined ? persisted.fallbackReason : patch.fallbackReason,
      inputTokens: patch.inputTokens ?? persisted.inputTokens,
      outputTokens: patch.outputTokens ?? persisted.outputTokens,
      totalTokens: patch.totalTokens ?? persisted.totalTokens,
      firstTokenLatencyMs: patch.firstTokenLatencyMs ?? persisted.firstTokenLatencyMs
    });
  },
} as OrchestrationActions;
