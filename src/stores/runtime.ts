import { defineStore } from "pinia";
import { isTauriAvailable, safeInvoke, safeListen } from "@/lib/tauri";
import {
  bindFrontendRecorderSession,
  bindFrontendRecorderTurn,
  initFrontendFlightRecorder,
  recordFrontendInstant
} from "@/lib/frontend-flight-recorder";
import { useProviderStore } from "@/stores/providers";
import { useSettingsStore } from "@/stores/settings";
import { deriveGraphRunFromRunState, extractActiveTaskFocus } from "../types/runtime";
import { isRetryableError } from "@/lib/error-utils";
import type {
  AttachmentAssetFilter,
  AvailableTool,
  CapabilitySourceView,
  CapabilityView,
  ChatMessage,
  ConversationCheckpointEntry,
  ExecutionCheckpoint,
  GraphRun,
  GraphRunControlResponse,
  GraphRunSubmissionPlan,
  GraphRunStreamStartResponse,
  HealthPayload,
  HistoryBranch,
  HistoryCheckoutMode,
  HistoryCheckoutResult,
  HistoryCursorState,
  HistoryNode,
  HistoryStateAuditSummary,
  MessageStateDelta,
  MessageStateSnapshot,
  RetrievedContextState,
  RuntimePhase,
  SessionOverview,
  SessionRuntimeView,
  SessionSnapshot,
  ToolActivity,
  TraceTimelineEntry,
  TurnInput,
  TurnInputImage,
  TurnStreamEvent,
  TurnTraceRecord
} from "../types/runtime";
import type { PersistedRuntimeState, RuntimeState } from "@/lib/runtime/types";
import type { HistoryCheckoutWireResult } from "@/lib/runtime/history";
import {
  debugLog,
  isDebugLoggingEnabled,
  measureHostRead,
  reportSwitchPerf,
  runLowPriorityTurnWork,
  wait,
  waitForNextPaint,
  withTimeout
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
  DEFAULT_FAILED_TURN_MESSAGE,
  DEFAULT_SESSION_ID,
  HYDRATION_TIMEOUT_MS,
  MAX_BG_REASONING_BUFFER_CHARS,
  MAX_BG_TEXT_BUFFER_CHARS,
  OUTPUT_END_PERSIST_DELAY_MS,
  STREAM_FLUSH_EAGER_CHARS,
  STREAM_FLUSH_INTERVAL_MS,
  TIMEOUT_RETRY_PENDING_MESSAGE,
  TRACE_TIMELINE_THROTTLE_MS
} from "@/lib/runtime/constants";
import {
  appendNormalizedReasoningContent,
  applyProviderPatchToTraceTimeline,
  buildAssistantModelLabel,
  buildCacheTelemetryDebugSnapshot,
  buildFallbackRuntimeTraceTimeline,
  canonicalizeTraceTimelineKind,
  cloneBuildContextObservation,
  cloneHookTraceRecords,
  cloneProviderCallRecords,
  cloneTraceTimeline,
  createBrowserPreviewTraceSteps,
  createBrowserPreviewTraceTimeline,
  createDefaultTraceSteps,
  createDefaultTraceTimeline,
  createSubmitFailureTraceSteps,
  createSubmitFailureTraceTimeline,
  createSubmitTraceSteps,
  finalizeCancelledTraceSteps,
  logCacheTelemetryContractViolations,
  normalizeReasoningContent,
  normalizeTurnTraceRecord,
  readNestedNumericTokenValue,
  resolvedStreamStartRunId,
  resolveEventTraceTimeline,
  resolveSemanticEventTraceTimeline,
  resolveProviderReturnedCacheHitInputTokens,
  resolveReasoningTokens,
  resolveTerminalToolActivities,
  toolStatusToMessageStatus
} from "@/lib/runtime/trace";
import {
  buildAttachmentMessageBlocks,
  buildAttachmentMetas,
  buildDisplayedUserMessageWithAttachments,
  buildProviderUserMessageWithAttachments,
  buildToolMessageDetail,
  buildTurnHistory,
  buildTurnTitle,
  buildTurnTraceTitleFromMessages,
  chatMessagesFromMessageState,
  completedPrefixIndex,
  hydrateMessagesFromHistory,
  previewMessageStateDeltaMessages,
  reuseStableChatMessages
} from "@/lib/runtime/messages";
import {
  attachmentDedupKey,
  bytesToDataUrl,
  importAttachment,
  MAX_TURN_IMAGES,
  mimeForSniffed,
  mimeFromExtension,
  readFileAsArrayBuffer,
  resolveAttachmentRoute,
  sniffImageKind,
  truncateTextByBytes,
  type PendingAttachment,
  type PendingAttachmentStatus,
  type ImportAttachmentOptions
} from "@/lib/runtime/file-attachments";
import {
  buildConversationCheckpointEntries,
  cloneHistoryBranches,
  cloneHistoryNodes,
  cloneHistoryStateAuditSummary,
  cloneRunControlAuditSummary,
  hasHostManagedHistoryState,
  historyCursorVersion,
  isHistoricalMode,
  isHistoricalRuntimeView,
  normalizeHistoryBranchSwitchResult,
  normalizeHistoryCheckoutResult,
  normalizeHistoryCursorMode,
  normalizeHistoryForkResult,
  normalizeHistoryRestoreResult,
  previewHistoryMutationUnavailable,
  resolveHistoryBranchHeadNodeId,
  resolveRuntimeViewHistoryProjection
} from "@/lib/runtime/history";
import { filterAttachmentAssets } from "@/lib/runtime/attachments";
import {
  createBrowserPreviewTerminalEnvelope,
  isTerminalPhase,
  normalizeCheckpointPhase,
  reconcileSubmissionWithRecoveryCheckpoint,
  resolveGraphRunSubmissionFromCheckpoint,
  resolveGraphRunSubmissionFromPlan,
  resolveGraphRunSubmissionFromRunState,
  resolveRestoredPersistedPhase,
  resolveRuntimePhaseFromEvent
} from "@/lib/runtime/phases";
import type { GraphRunSubmission } from "@/lib/runtime/phases";
import {
  buildEventCursorByTurnTraceHistory,
  buildRuntimeViewFromPersistedState,
  buildSessionOverviewFromPersistedState,
  buildSessionOverviewFromRuntimeState,
  cloneGraphRunControlBoundaryEvidence,
  cloneRetrievedContext,
  createBlankSessionRuntimeFields,
  createNextSessionId,
  createSessionRuntimeSnapshot,
  createSnapshotFromRuntimeState,
  createTransientSessionOverview,
  deriveInitStrategy,
  deriveRetrievedContextFromSnapshot,
  ensureUniqueSessionList,
  filterDeletingSessions,
  hasPersistableMessages,
  isPersistedMetadataCompatible,
  isPersistedMessageShapeCompatible,
  loadPersistedRuntimeCache,
  loadPersistedRuntimeState,
  loadRunningSessionMap,
  mergeRuntimeViews,
  persistSessionState,
  persistSessionStateAndRuntimeMaps,
  removePersistedSessionState,
  restoreSessionRuntimeSnapshot,
  shouldAcceptTurnEvent
} from "@/lib/runtime/sessions";
import {
  createAvailableTools,
  createCapabilities,
  createCapabilitySources,
  defaultAvailableTools
} from "@/lib/runtime/browser-preview";
// 运行态看门狗：isSubmitting 置位后若在超时窗口内未收到任何终态事件
// （completed/failed/cancelled），强制解锁，杜绝"终态事件被丢弃 → 永久卡死"。
const SUBMISSION_WATCHDOG_TIMEOUT_MS = 120_000;

export const useRuntimeStore = defineStore("runtime", {
  state: (): RuntimeState => {
    const persisted = loadPersistedRuntimeState(DEFAULT_SESSION_ID);

      return {
        sessionId: DEFAULT_SESSION_ID,
        sessionList: [],
        sessionOperation: null,
        sessionHydrating: false,
        deletingSessionSet: {},
        sessionSwitchToken: 0,
        sessionError: null,
      phase: resolveRestoredPersistedPhase(
        persisted?.phase ?? null,
        persisted?.canonicalTerminalPhase ?? null,
        persisted?.messages ?? [],
        (persisted?.turnTraceHistory ?? []).map((trace) => normalizeTurnTraceRecord(trace))
      ),
      health: null,
      error: null,
      draftMessage: "",
      pendingAttachments: [] as PendingAttachment[],
      sessionSummary: persisted?.sessionSummary ?? "",
      retrievedContext: null,
      providerRequestedName: persisted?.providerRequestedName ?? "",
      providerName: persisted?.providerName ?? "",
      providerProtocol: persisted?.providerProtocol ?? "",
      providerModel: persisted?.providerModel ?? "",
      providerSource: persisted?.providerSource ?? "",
      providerMode: persisted?.providerMode ?? "",
      fallbackReason: persisted?.fallbackReason ?? null,
      inputTokens: persisted?.inputTokens ?? null,
      outputTokens: persisted?.outputTokens ?? null,
      totalTokens: persisted?.totalTokens ?? null,
      firstTokenLatencyMs: persisted?.firstTokenLatencyMs ?? null,
      isSubmitting: false,
      activeTurnId: null,
      activeRunId: null,
      latestExecutionCheckpoint: null,
      latestGraphRunSubmissionPlan: null,
      latestGraphRunControlBoundaryEvidence: [],
      latestRunControlAuditSummary: null,
      latestHistoryStateAuditSummary: null,
      visibleNodeId: null,
      branchHeadNodeId: null,
      activeBranchId: null,
      cursorVersion: null,
      messageRevision: null,
      historyCursorMode: "live",
      historyNodes: [],
      historyBranches: [],
      initialRollbackActive: false,
      eventsReady: false,
      deferredPersistTimerId: null,
      streamFlushFrameId: null,
      streamFlushTimerId: null,
      streamBufferTurnId: null,
      streamBufferText: "",
      streamBufferReasoning: "",
      browserPreviewRunToken: 0,
      messages: persisted?.messages ?? [],
      attachmentAssets: persisted?.attachmentAssets ?? [],
      availableTools: createAvailableTools(),
      capabilitySources: createCapabilitySources(),
      capabilities: createCapabilities(),
      toolActivities: [],
      traceSteps: createDefaultTraceSteps(),
      traceTimeline: createDefaultTraceTimeline(),
      turnTraceHistory: (persisted?.turnTraceHistory ?? []).map((trace) => normalizeTurnTraceRecord(trace)),
      eventCursorByTurnId: buildEventCursorByTurnTraceHistory(
        (persisted?.turnTraceHistory ?? []).map((trace) => normalizeTurnTraceRecord(trace))
      ),
      runningSessionMap: {},
      completedSessionSet: {},
      failedSessionSet: {},
      streamDebugDeltaCount: 0,
      streamDebugFlushCount: 0,
      streamDebugTextCharsReceived: 0,
      streamDebugTextCharsFlushed: 0,
      pendingThrottledTraceTimeline: null,
      traceTimelineThrottleTimerId: null,
      submissionWatchdogTimerId: null
    };
  },
  getters: {
    phaseLabel(state): string {
      const labels: Record<string, string> = {
        idle: "空闲",
        connecting: "连接中",
        ready: "已就绪",
        completed: "本轮完成",
        cancelled: "已停止",
        calling_model: "模型处理中",
        calling_tool: "工具处理中",
        failed: "失败"
      };

      return labels[state.phase] ?? state.phase;
    },
    isHistoricalMode(state): boolean {
      return state.historyCursorMode !== "live";
    },
    isSessionDeleting(state) {
      return (sessionId: string) => sessionId in state.deletingSessionSet;
    },
    conversationCheckpointEntries(state): ConversationCheckpointEntry[] {
      const entries = buildConversationCheckpointEntries(
        state.historyNodes,
        state.historyBranches,
        state.activeBranchId,
        state.visibleNodeId,
        state.branchHeadNodeId
      );

      // Synthesize entries for completed turns in messages that lack history nodes.
      // This ensures rollback buttons work even before backend checkpoints arrive.
      const coveredTurnIds = new Set(entries.map((e) => e.turnId));
      const turnUserContent = new Map<string, string>();
      for (const msg of state.messages) {
        if (msg.role === "user" && msg.content && !turnUserContent.has(msg.turnId)) {
          turnUserContent.set(msg.turnId, msg.content);
        }
      }

      const allTurnIds = [...turnUserContent.keys()];
      let hasSynthetic = false;

      for (let index = 0; index < allTurnIds.length; index++) {
        const turnId = allTurnIds[index]!;
        if (coveredTurnIds.has(turnId)) continue;

        entries.push({
          nodeId: `synthetic-${turnId}`,
          turnId,
          branchId: state.activeBranchId ?? "branch-main",
          summary: turnUserContent.get(turnId)!.slice(0, 120) || turnId,
          createdAtMs: Date.now() - (allTurnIds.length - index),
          isLatest: false,
          isVisible: false,
          workspaceRollbackCapable: false,
          availableModes: ["transcript_only"],
          forkTargets: []
        });
        hasSynthetic = true;
      }

      if (hasSynthetic) {
        entries.sort((left, right) => right.createdAtMs - left.createdAtMs);
        const hasRealLatest =
          state.branchHeadNodeId != null && entries.some((e) => e.nodeId === state.branchHeadNodeId);
        if (!hasRealLatest && entries.length > 0) {
          entries[0] = { ...entries[0], isLatest: true };
        }
      }

      return entries;
    },
    isSessionRunning(state): (sessionId: string) => boolean {
      return (sessionId: string) => sessionId in state.runningSessionMap;
    }
  },
  actions: {
    resetSessionRuntimeState() {
      this.cancelDeferredPersist();
      this.clearSubmissionWatchdog();
      const blankFields = createBlankSessionRuntimeFields();
      this.phase = "idle";
      this.error = null;
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
      this.isSubmitting = false;
      this.activeTurnId = null;
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
    cancelDeferredPersist() {
      if (this.deferredPersistTimerId == null) {
        return;
      }

      window.clearTimeout(this.deferredPersistTimerId);
      this.deferredPersistTimerId = null;
    },
    cancelStreamFlush() {
      if (this.streamFlushFrameId == null) {
        if (this.streamFlushTimerId == null) {
          return;
        }
      }

      if (this.streamFlushFrameId != null) {
        window.cancelAnimationFrame(this.streamFlushFrameId);
        this.streamFlushFrameId = null;
      }

      if (this.streamFlushTimerId != null) {
        window.clearTimeout(this.streamFlushTimerId);
        this.streamFlushTimerId = null;
      }
    },
    resetStreamDebugMetrics() {
      this.streamDebugDeltaCount = 0;
      this.streamDebugFlushCount = 0;
      this.streamDebugTextCharsReceived = 0;
      this.streamDebugTextCharsFlushed = 0;
    },
    flushBufferedStreamText(turnId?: string | null) {
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
        assistantMessage.content += this.streamBufferText;
      }

      if (flushedTextChars > 0 || flushedReasoningChars > 0) {
        this.streamDebugFlushCount += 1;
        this.streamDebugTextCharsFlushed += flushedTextChars;
      }

      this.streamBufferTurnId = null;
      this.streamBufferText = "";
      this.streamBufferReasoning = "";
    },
    scheduleStreamFlush(turnId: string) {
      if (this.streamBufferTurnId && this.streamBufferTurnId !== turnId) {
        this.flushBufferedStreamText(this.streamBufferTurnId);
      }

      this.streamBufferTurnId = turnId;
      const bufferedLength = this.streamBufferText.length + this.streamBufferReasoning.length;
      if (bufferedLength >= STREAM_FLUSH_EAGER_CHARS) {
        this.flushBufferedStreamText(turnId);
        return;
      }

      if (this.streamFlushFrameId != null || this.streamFlushTimerId != null) {
        return;
      }

      this.streamFlushTimerId = window.setTimeout(() => {
        this.streamFlushTimerId = null;
        this.flushBufferedStreamText(turnId);
      }, STREAM_FLUSH_INTERVAL_MS);
    },
    cancelBrowserPreviewTurn(turnId: string) {
      if (this.activeTurnId !== turnId) {
        return false;
      }

      this.browserPreviewRunToken += 1;
      const cancelledTraceSteps = finalizeCancelledTraceSteps(this.traceSteps);
      const assistantMessage = this.ensureAssistantMessage(
        turnId,
        buildAssistantModelLabel(this.providerName, this.providerModel)
      );
      assistantMessage.content = "本轮已停止。";
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
    scheduleDeferredPersist(delay = 140) {
      this.cancelDeferredPersist();
      this.deferredPersistTimerId = window.setTimeout(() => {
        this.deferredPersistTimerId = null;
        this.persistHistory();
      }, delay);
    },
    startSubmissionWatchdog(turnId: string) {
      this.clearSubmissionWatchdog();
      this.submissionWatchdogTimerId = window.setTimeout(() => {
        this.submissionWatchdogTimerId = null;
        if (!this.isSubmitting || this.activeTurnId !== turnId) {
          return;
        }
        debugLog("watchdog:submission-timeout", {
          turnId,
          timeoutMs: SUBMISSION_WATCHDOG_TIMEOUT_MS
        });
        // 兜底：把仍卡 pending 的 assistant 消息收敛为 error，停止逐字渲染续跑。
        const assistantMessage = this.messages.find(
          (message) => message.turnId === turnId && message.role === "assistant"
        );
        if (assistantMessage && assistantMessage.status === "pending") {
          assistantMessage.status = "error";
          assistantMessage.errorDetail = "submission_watchdog_timeout";
          this.messageRevision = null;
        }
        this.isSubmitting = false;
        this.activeTurnId = null;
        this.activeRunId = null;
        this.phase = "failed";
        this.error = "运行超时：长时间未收到终态事件，已强制解锁。";
        this.persistHistory();
      }, SUBMISSION_WATCHDOG_TIMEOUT_MS);
    },
    clearSubmissionWatchdog() {
      if (this.submissionWatchdogTimerId != null) {
        window.clearTimeout(this.submissionWatchdogTimerId);
        this.submissionWatchdogTimerId = null;
      }
    },
    persistHistory() {
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
        runningTurnId: this.activeTurnId
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
    updatePersistedBackgroundSession(
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
    getAttachmentAssets(filter?: AttachmentAssetFilter | null) {
      return filterAttachmentAssets(this.attachmentAssets, filter);
    },
    applyHistoryState(
      _sessionId: string,
      payload?:
        | (Partial<HistoryCursorState> & {
            historyNodes?: HistoryNode[] | null;
            historyBranches?: HistoryBranch[] | null;
          })
        | null
    ) {
      const historyNodes = cloneHistoryNodes(payload?.historyNodes);
      const historyBranches = cloneHistoryBranches(payload?.historyBranches);
      const activeBranchId = payload?.activeBranchId?.trim() || null;
      const explicitHeadNodeId = payload?.branchHeadNodeId?.trim() || null;
      const branchHeadNodeId =
        explicitHeadNodeId || resolveHistoryBranchHeadNodeId(activeBranchId, historyBranches);
      const visibleNodeId = payload?.visibleNodeId?.trim() || null;

      this.historyNodes = historyNodes;
      this.historyBranches = historyBranches;
      this.activeBranchId = activeBranchId;
      this.branchHeadNodeId = branchHeadNodeId;
      this.visibleNodeId = visibleNodeId;
      this.cursorVersion = historyCursorVersion(payload);
      this.historyCursorMode = normalizeHistoryCursorMode(
        payload?.mode ?? (visibleNodeId && branchHeadNodeId && visibleNodeId !== branchHeadNodeId ? "historical" : "live")
      );
    },
    applyMessageStateSnapshot(snapshot?: MessageStateSnapshot | null) {
      if (!snapshot || snapshot.sessionId !== this.sessionId) {
        return false;
      }

      this.messages = reuseStableChatMessages(this.messages, chatMessagesFromMessageState(snapshot));
      this.messageRevision = snapshot.revision;
      return true;
    },
    applyMessageStateDelta(delta?: MessageStateDelta | null) {
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
    async loadSessionCatalog() {
      if (isTauriAvailable()) {
        this.sessionList = filterDeletingSessions(
          await measureHostRead("list_sessions", {
            deletingCount: Object.keys(this.deletingSessionSet).length
          }, () => safeInvoke<SessionOverview[]>("list_sessions")),
          this.deletingSessionSet
        );
        return;
      }

      const cache = loadPersistedRuntimeCache();
      this.sessionList = filterDeletingSessions(
        Object.entries(cache.sessions)
          .map(([conversationId, state]) => buildSessionOverviewFromPersistedState(conversationId, state))
          .sort((left, right) => right.updatedAtMs - left.updatedAtMs),
        this.deletingSessionSet
      );
    },
    async loadSessionRuntimeViewState(sessionId: string, nodeId?: string | null) {
      if (isTauriAvailable()) {
        const payload: Record<string, unknown> = {
          turnId: null,
          sessionId,
          runId: null
        };
        if (nodeId) {
          payload.nodeId = nodeId;
        }
        return await measureHostRead("load_session_runtime_view", {
          sessionId,
          nodeId: nodeId ?? null
        }, () => safeInvoke<SessionRuntimeView>("load_session_runtime_view", {
          ...payload
        }));
      }

      return buildRuntimeViewFromPersistedState(sessionId, loadPersistedRuntimeState(sessionId), nodeId);
    },
    applyExecutionCheckpoint(
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
    async loadSessionState(
      nextSessionId: string,
      options?: {
        refreshCatalog?: boolean;
        executionCheckpoint?: ExecutionCheckpoint | null;
        runtimeView?: SessionRuntimeView | null;
        nodeId?: string | null;
        preserveMessages?: boolean;
      }
    ) {
      const refreshCatalog = options?.refreshCatalog ?? true;
      const persistedState = loadPersistedRuntimeState(nextSessionId);
      const shouldPreferLocalInitialRollback = Boolean(persistedState?.initialRollbackActive) && !options?.nodeId;
      const persistedVisibleNodeId = persistedState?.visibleNodeId?.trim() || null;
      const fallbackNodeId =
        options?.nodeId ??
        (!isTauriAvailable()
          ? persistedVisibleNodeId
          : null);
      const runtimeView =
        options?.runtimeView ?? (shouldPreferLocalInitialRollback
          ? buildRuntimeViewFromPersistedState(nextSessionId, persistedState, fallbackNodeId)
          : await this.loadSessionRuntimeViewState(nextSessionId, fallbackNodeId));
      const snapshot = runtimeView.session;
      const retrieved = runtimeView.retrieved;
      const persisted = persistedState;
      const runtimeViewIsHistorical =
        !hasHostManagedHistoryState(runtimeView)
          ? normalizeHistoryCursorMode(runtimeView.historyCursor?.mode ?? null) !== "live"
          : normalizeHistoryCursorMode(runtimeView.historyCursor?.mode ?? null) !== "live" ||
            runtimeView.isAtBranchHead === false;
      const shouldApplyRuntimeViewCheckpoint =
        !runtimeViewIsHistorical && (!options?.nodeId || !runtimeViewIsHistorical);
      const hasCheckpointOverride =
        options != null && Object.prototype.hasOwnProperty.call(options, "executionCheckpoint");
      this.applySessionSnapshot(nextSessionId, snapshot, retrieved, runtimeView, { preserveMessages: options?.preserveMessages });
      this.applyExecutionCheckpoint(
        hasCheckpointOverride
          ? (options?.executionCheckpoint ?? null)
          : shouldApplyRuntimeViewCheckpoint
            ? (runtimeView.checkpoint ?? null)
            : null,
        persisted?.messages
      );
      this.latestGraphRunSubmissionPlan = runtimeView.submissionPlan ? { ...runtimeView.submissionPlan } : null;
      this.latestGraphRunControlBoundaryEvidence = cloneGraphRunControlBoundaryEvidence(
        runtimeView.controlBoundaryEvidence
      );
      this.latestRunControlAuditSummary = cloneRunControlAuditSummary(
        runtimeView.runControlAuditSummary ?? snapshot.runControlAuditSummary ?? null
      );
      this.latestHistoryStateAuditSummary = cloneHistoryStateAuditSummary(
        runtimeView.historyStateAuditSummary ?? snapshot.historyStateAuditSummary ?? null
      );

      if (refreshCatalog) {
        await this.loadSessionCatalog();
      }
    },
    async loadRetrievedContextState(
      sessionId: string,
      options?: { runId?: string | null; snapshot?: SessionSnapshot | null; nodeId?: string | null }
    ) {
      const fallbackSnapshot = options?.snapshot ?? createSnapshotFromRuntimeState(this, sessionId);
      if (!isTauriAvailable()) {
        return deriveRetrievedContextFromSnapshot(fallbackSnapshot);
      }

      try {
        const payload: Record<string, unknown> = {
          sessionId,
          runId: options?.runId ?? null,
          turnId: null
        };
        if (options?.nodeId) {
          payload.nodeId = options.nodeId;
        }
        const retrieved = await measureHostRead("load_retrieved_context", {
          sessionId,
          nodeId: options?.nodeId ?? null,
          runId: options?.runId ?? null
        }, () => safeInvoke<RetrievedContextState>("load_retrieved_context", payload));
        return cloneRetrievedContext(retrieved);
      } catch (error) {
        debugLog("retrieved-context:load:error", {
          sessionId,
          runId: options?.runId ?? null,
          error: String(error)
        });
        return deriveRetrievedContextFromSnapshot(fallbackSnapshot);
      }
    },
    async resolveDerivedSessionRun(options?: {
      sessionId?: string | null;
      runId?: string | null;
      preferRefresh?: boolean;
      snapshot?: SessionSnapshot | null;
      nodeId?: string | null;
    }): Promise<GraphRun | null> {
      const targetSessionId = options?.sessionId?.trim() || this.sessionId;
      if (!targetSessionId) {
        return null;
      }

      const deriveRun = (retrieved: RetrievedContextState | null | undefined) => {
        if (!retrieved) {
          return null;
        }

        return deriveGraphRunFromRunState(retrieved.runState, targetSessionId, Date.now(), {
          activeTaskFocus: extractActiveTaskFocus(retrieved.longTermMemory?.entries)
        });
      };

      if (!options?.preferRefresh) {
        const localRun = deriveRun(
          this.sessionId === targetSessionId ? this.retrievedContext : null
        );
        if (localRun) {
          return localRun;
        }
      }

      const refreshedRetrieved = await this.loadRetrievedContextState(targetSessionId, {
        runId: options?.runId ?? this.activeRunId,
        snapshot: options?.snapshot ?? createSnapshotFromRuntimeState(this, targetSessionId),
        nodeId: options?.nodeId ?? this.visibleNodeId
      });
      if (this.sessionId === targetSessionId) {
        this.retrievedContext = refreshedRetrieved;
      }
      return deriveRun(refreshedRetrieved);
    },
    applySessionSnapshot(
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
      this.toolActivities = [];
      this.traceSteps = createDefaultTraceSteps();
      // 优化（PA-089）：切换会话时 traceTimeline 深拷贝延迟到 rIC——
      // 避免同步深拷贝（cloneTraceTimeline + cloneToolActivities）阻塞主线程
      // （实测切换会话 1.5-1.9s 卡顿源之一）。先发布默认 timeline，空闲时再深拷贝。
      const lastTraceTimeline = this.turnTraceHistory[this.turnTraceHistory.length - 1]?.traceTimeline;
      this.publishTraceTimeline(createDefaultTraceTimeline());
      if (lastTraceTimeline?.length) {
        runLowPriorityTurnWork(() => {
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
    rollbackToInitialState() {
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
    rollbackToTurnBoundary(turnId: string) {
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
    async performHistoryOperation<TInvokeArgs extends Record<string, unknown>, TResult extends HistoryCursorState & { historyStateAuditSummary?: HistoryStateAuditSummary | null }>(
      options: {
        cmd: string;
        invokeArgs: TInvokeArgs;
        normalize: (payload: any, nodes: HistoryNode[], branches: HistoryBranch[]) => TResult;
        selectNodeId?: (result: any) => string | null;
        errorMessagePrefix?: string;
      }
    ): Promise<TResult | null> {
      try {
        const payload = await safeInvoke(options.cmd, {
          sessionId: this.sessionId,
          expectedCursorVersion: this.cursorVersion,
          ...options.invokeArgs
        });
        const messageDelta = (payload as { messageDelta?: MessageStateDelta | null }).messageDelta ?? null;
        if (!this.applyMessageStateDelta(messageDelta)) {
          const preservedDraft = this.draftMessage;
          await this.loadSessionState(this.sessionId, {
            refreshCatalog: false,
            nodeId: options.selectNodeId?.(payload) ?? null
          });
          this.draftMessage = preservedDraft;
        }
        const result = options.normalize(payload, this.historyNodes, this.historyBranches);
        this.initialRollbackActive = false;
        this.applyHistoryState(this.sessionId, result);
        this.latestHistoryStateAuditSummary = cloneHistoryStateAuditSummary(
          result.historyStateAuditSummary ?? null
        );
        return result;
      } catch (error) {
        this.sessionError = `${options.errorMessagePrefix ?? "操作"}失败：${String(error)}`;
        return null;
      }
    },
    async checkoutHistoryNode(nodeId: string, mode: HistoryCheckoutMode = "transcript_only", turnId?: string | null) {
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
    findCheckpointTurnIdByNodeId(nodeId: string): string | null {
      const entry = this.conversationCheckpointEntries.find((e) => e.nodeId === nodeId);
      return entry?.turnId?.trim() || null;
    },
    async restoreBranchHead(branchId?: string | null) {
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
    async forkHistoryNode(nodeId?: string | null) {
      const targetNodeId = nodeId?.trim() || this.visibleNodeId;
      if (!this.sessionId || !targetNodeId) {
        return null;
      }

      if (!isTauriAvailable()) {
        this.sessionError = previewHistoryMutationUnavailable();
        return null;
      }

      return this.performHistoryOperation({
        cmd: "fork_from_history_node",
        invokeArgs: { nodeId: targetNodeId },
        normalize: (payload) => normalizeHistoryForkResult(payload, this.historyNodes, this.historyBranches),
        selectNodeId: (payload) => payload.cursor.visibleNodeId ?? payload.cursor.branchHeadNodeId ?? null,
        errorMessagePrefix: "创建 branch"
      });
    },
    async switchHistoryBranch(branchId: string) {
      if (!this.sessionId || !branchId.trim()) {
        return null;
      }

      if (!isTauriAvailable()) {
        this.sessionError = previewHistoryMutationUnavailable();
        return null;
      }

      return this.performHistoryOperation({
        cmd: "switch_history_branch",
        invokeArgs: { branchId },
        normalize: (payload) => normalizeHistoryBranchSwitchResult(payload, this.historyNodes, this.historyBranches),
        selectNodeId: (payload) => payload.cursor.visibleNodeId ?? payload.cursor.branchHeadNodeId ?? null,
        errorMessagePrefix: "切换 branch"
      });
    },
    async switchSession(nextSessionId: string) {
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
        delete this.runningSessionMap[nextSessionId];
      }

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
    async createSession() {
      // 新建会话意图即清空待发送附件（即使空消息 no-op 也清，避免残留携带）
      this.clearPendingAttachments();
      if (this.sessionOperation || !hasPersistableMessages(this.messages)) {
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
      const dedupedCurrentSessionList = this.sessionList.filter((session) => session.conversationId !== nextSessionId);
      this.sessionList = ensureUniqueSessionList([
        createTransientSessionOverview(nextSessionId),
        ...(currentOverview ? [currentOverview] : []),
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
    async deleteSession(targetSessionId: string) {
      if (this.isSubmitting || this.deletingSessionSet[targetSessionId]) {
        return;
      }

      const deletingActiveEmptySession =
        targetSessionId === this.sessionId && !hasPersistableMessages(this.messages);
      const requiresGlobalSessionOperation =
        deletingActiveEmptySession ||
        targetSessionId === this.sessionId ||
        !this.sessionList.some((session) => session.conversationId === this.sessionId);
      if (requiresGlobalSessionOperation && this.sessionOperation) {
        return;
      }
      const previousSnapshot = createSessionRuntimeSnapshot(this);
      const persistedStateToRestore = loadPersistedRuntimeState(targetSessionId);
      this.deletingSessionSet[targetSessionId] = true;
      if (requiresGlobalSessionOperation) {
        this.sessionOperation = "deleting";
      } else {
        this.sessionList = this.sessionList.filter((session) => session.conversationId !== targetSessionId);
      }
      this.sessionError = null;

      try {
        removePersistedSessionState(targetSessionId);
        delete this.runningSessionMap[targetSessionId];
        delete this.completedSessionSet[targetSessionId];
        delete this.failedSessionSet[targetSessionId];

        if (isTauriAvailable() && !deletingActiveEmptySession) {
          this.sessionList = filterDeletingSessions(
            await safeInvoke<SessionOverview[]>("delete_session", {
              sessionId: targetSessionId
            }),
            this.deletingSessionSet
          );
        } else {
          await this.loadSessionCatalog();
        }

        const fallbackSessionId =
          this.sessionList.find((session) => session.conversationId !== targetSessionId)?.conversationId ??
          this.sessionList[0]?.conversationId ??
          DEFAULT_SESSION_ID;

        const shouldLoadFallbackSession =
          deletingActiveEmptySession ||
          this.sessionId === targetSessionId ||
          !this.sessionList.some((session) => session.conversationId === this.sessionId);

        if (!shouldLoadFallbackSession) {
          debugLog("session:delete", {
            targetSessionId
          });
          return;
        }

        this.resetSessionRuntimeState();
        this.sessionId = fallbackSessionId;
        this.phase = "connecting";

        try {
          if (this.sessionList.length === 0 && fallbackSessionId === DEFAULT_SESSION_ID) {
            this.phase = "idle";
            debugLog("session:delete:empty-fallback", {
              targetSessionId
            });
            return;
          }

          await this.loadSessionState(fallbackSessionId, { refreshCatalog: false });
          debugLog("session:delete:fallback", {
            targetSessionId,
            fallbackSessionId
          });
        } catch (error) {
          this.resetSessionRuntimeState();
          this.sessionId = fallbackSessionId;
          this.phase = "idle";
          this.sessionError = `删除对话后加载替代对话失败：${String(error)}`;
          debugLog("session:delete:fallback-error", {
            targetSessionId,
            fallbackSessionId,
            error: String(error)
          });
        }
      } catch (error) {
        if (persistedStateToRestore) {
          persistSessionState(targetSessionId, persistedStateToRestore);
        }
        restoreSessionRuntimeSnapshot(this, previousSnapshot);
        this.sessionError = `删除对话失败：${String(error)}`;
        debugLog("session:delete:error", {
          targetSessionId,
          error: String(error)
        });
      } finally {
        delete this.deletingSessionSet[targetSessionId];
        if (requiresGlobalSessionOperation) {
          this.sessionOperation = null;
        }
      }
    },
    async initializeSessions() {
      if (this.sessionOperation) {
        return;
      }

      const previousSnapshot = createSessionRuntimeSnapshot(this);
      this.sessionOperation = "initializing";
      this.sessionError = null;

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
        const preferredSessionId = this.sessionList[0]?.conversationId ?? this.sessionId;
        const persisted = loadPersistedRuntimeState(preferredSessionId);
        const strategy = deriveInitStrategy(preferredSessionId, persisted, this.sessionList);

        debugLog("session:init:strategy", { strategy: strategy.kind, preferredSessionId });

        switch (strategy.kind) {
          case "local-cache": {
            const cachedRuntimeView = buildRuntimeViewFromPersistedState(preferredSessionId, strategy.persistedState);
            const hostRuntimeView = await this.loadSessionRuntimeViewState(preferredSessionId).catch((error) => {
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
            const runtimeView = await this.loadSessionRuntimeViewState(preferredSessionId);
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
        restoreSessionRuntimeSnapshot(this, previousSnapshot);
        this.sessionError = `初始化对话失败：${String(error)}`;
        debugLog("session:init:error", {
          error: String(error)
        });
      } finally {
        this.sessionOperation = null;
      }
    },
    upsertTurnTrace(
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
        updatedAt
      }));
      this.turnTraceHistory[this.turnTraceHistory.length - 1]!.title = resolvedTitle;
      this.eventCursorByTurnId = buildEventCursorByTurnTraceHistory(this.turnTraceHistory);
      if (persist) {
        this.persistHistory();
      }
    },
    shouldProcessTurnEvent(
      payload: Pick<TurnStreamEvent, "turnId" | "eventId" | "sequence" | "emittedAtMs" | "kind">
    ) {
      const isTerminalEvent =
        payload.kind === "completed" || payload.kind === "failed" || payload.kind === "cancelled";
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
    commitTurnEventCursor(payload: Pick<TurnStreamEvent, "turnId" | "eventId" | "sequence" | "emittedAtMs">) {
      this.eventCursorByTurnId[payload.turnId] = {
        eventId: payload.eventId?.trim() || null,
        sequence: typeof payload.sequence === "number" && Number.isFinite(payload.sequence) ? payload.sequence : null,
        emittedAtMs:
          typeof payload.emittedAtMs === "number" && Number.isFinite(payload.emittedAtMs)
            ? payload.emittedAtMs
            : null
      };
    },
    resolveTurnTraceTimeline(turnId: string) {
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
    commitTurnTraceTimeline(
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
    updateActiveTraceTimeline(traceTimeline: TraceTimelineEntry[], skipClone = false) {
      // 低频语义事件路径（turn:trace / phase_changed / checkpoint_persisted / tool）中，
      // resolveEventTraceTimeline 已返回新鲜克隆，skipClone 避免第二次全量深拷贝。
      this.publishTraceTimeline(skipClone ? traceTimeline : cloneTraceTimeline(traceTimeline));
    },
    // 单一 trace timeline 发布入口（PA-086）：所有写路径收敛于此，
    // 保证投影层（签名化 memo）与源数据强一致。
    publishTraceTimeline(traceTimeline: TraceTimelineEntry[]) {
      this.traceTimeline = traceTimeline;
    },
    // delta 高频事件中的 timeline 节流更新：只保留最新一份，定时合并应用。
    scheduleThrottledTraceTimeline(traceTimeline: TraceTimelineEntry[]) {
      this.pendingThrottledTraceTimeline = traceTimeline;
      if (this.traceTimelineThrottleTimerId != null) {
        return;
      }
      this.traceTimelineThrottleTimerId = window.setTimeout(() => {
        this.traceTimelineThrottleTimerId = null;
        const pending = this.pendingThrottledTraceTimeline;
        this.pendingThrottledTraceTimeline = null;
        if (pending) {
          this.updateActiveTraceTimeline(pending);
        }
      }, TRACE_TIMELINE_THROTTLE_MS);
    },
    // 冲刷挂起的 timeline 更新（低频语义事件 / terminal 事件前调用，保证最终态一致）。
    flushPendingTraceTimeline() {
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
    clearPendingTraceTimeline() {
      if (this.traceTimelineThrottleTimerId != null) {
        window.clearTimeout(this.traceTimelineThrottleTimerId);
        this.traceTimelineThrottleTimerId = null;
      }
      this.pendingThrottledTraceTimeline = null;
    },
    updateActiveModelTraceFromAssistant(turnId: string) {
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
    applyTurnTokenStats(turnId: string, inputTokens?: number | null, outputTokens?: number | null, persist = true) {
      const userMessage = this.messages.find((item) => item.turnId === turnId && item.role === "user");
      const assistantMessage = this.messages.find((item) => item.turnId === turnId && item.role === "assistant");

      if (userMessage && inputTokens != null) {
        userMessage.tokenCount = inputTokens;
        this.messageRevision = null;
      }

      if (assistantMessage && outputTokens != null) {
        assistantMessage.tokenCount = outputTokens;
        this.messageRevision = null;
      }

      if (persist) {
        this.persistHistory();
      }
    },
    applyOutputEnd(payload: TurnStreamEvent) {
      this.flushBufferedStreamText(payload.turnId);

      const assistantMessage = this.ensureAssistantMessage(
        payload.turnId,
        buildAssistantModelLabel(payload.providerName, payload.providerModel)
      );

      const finalText = payload.text?.trim();
      if (finalText && payload.text !== assistantMessage.content) {
        assistantMessage.content = payload.text ?? assistantMessage.content;
        this.messageRevision = null;
      }
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
      this.inputTokens = payload.inputTokens ?? this.inputTokens;
      this.outputTokens = payload.outputTokens ?? this.outputTokens;
      this.totalTokens = payload.totalTokens ?? this.totalTokens;
      this.firstTokenLatencyMs = payload.firstTokenLatencyMs ?? this.firstTokenLatencyMs;
      this.applyTurnTokenStats(payload.turnId, payload.inputTokens, payload.outputTokens, false);
      this.scheduleDeferredPersist(OUTPUT_END_PERSIST_DELAY_MS);
    },
    ensureAssistantMessage(turnId: string, modelName?: string | null) {
      const messageId = `assistant-${turnId}`;
      const existingMessage = this.messages.find((item) => item.id === messageId && item.role === "assistant");

      if (existingMessage) {
        if (modelName?.trim()) {
          existingMessage.modelName = modelName;
        }
        return existingMessage;
      }

      const assistantMessage: ChatMessage = {
        id: messageId,
        turnId,
        role: "assistant",
        reasoningContent: null,
        content: "",
        status: "pending",
        tokenCount: null,
        modelName: modelName?.trim() || undefined
      };

      this.messages.push(assistantMessage);
      this.messageRevision = null;
      // 延迟合并落盘：新建 assistant 消息时避免同步全量序列化阻塞主线程
      // （流式期间高频调用 ensureAssistantMessage，同步 persist 会放大卡顿）。
      this.scheduleDeferredPersist();
      return this.messages.find((item) => item.id === messageId && item.role === "assistant") ?? assistantMessage;
    },
    syncToolMessages(turnId: string, toolActivities?: ToolActivity[] | null, persist = true) {
      if (!toolActivities) {
        return;
      }

      const activeTools = toolActivities.filter((tool) => tool.status !== "planned");
      const existingMessagesById = new Map<string, ChatMessage>();
      for (const message of this.messages) {
        if (message.role === "tool" && message.turnId === turnId) {
          existingMessagesById.set(message.id, message);
        }
      }
      const pendingMessages: ChatMessage[] = [];
      let didMutate = false;

      for (const tool of activeTools) {
        const messageId = `tool-${turnId}-${tool.id}`;
        const existingMessage = existingMessagesById.get(messageId);
        const nextContent = tool.resultText ?? "";
        const nextDetail = buildToolMessageDetail(tool);
        const nextStatus = toolStatusToMessageStatus(tool.status);
        const nextDurationSeconds = tool.durationSeconds ?? null;
        const nextCanonicalToolName = tool.canonicalToolName ?? null;
        const nextDisplayNameZh = tool.displayNameZh ?? null;

        if (existingMessage) {
          if (existingMessage.content !== nextContent) {
            existingMessage.content = nextContent;
            didMutate = true;
          }
          if (existingMessage.status !== nextStatus) {
            existingMessage.status = nextStatus;
            didMutate = true;
          }
          if (existingMessage.toolName !== tool.name) {
            existingMessage.toolName = tool.name;
            didMutate = true;
          }
          if (existingMessage.canonicalToolName !== nextCanonicalToolName) {
            existingMessage.canonicalToolName = nextCanonicalToolName;
            didMutate = true;
          }
          if (existingMessage.displayNameZh !== nextDisplayNameZh) {
            existingMessage.displayNameZh = nextDisplayNameZh;
            didMutate = true;
          }
          if (existingMessage.detail !== nextDetail) {
            existingMessage.detail = nextDetail;
            didMutate = true;
          }
          if (existingMessage.durationSeconds !== nextDurationSeconds) {
            existingMessage.durationSeconds = nextDurationSeconds;
            didMutate = true;
          }
          continue;
        }

        pendingMessages.push({
          id: messageId,
          turnId,
          role: "tool",
          content: nextContent,
          status: nextStatus,
          toolName: tool.name,
          canonicalToolName: nextCanonicalToolName,
          displayNameZh: nextDisplayNameZh,
          detail: nextDetail,
          durationSeconds: nextDurationSeconds
        });
        didMutate = true;
      }

      if (pendingMessages.length > 0) {
        this.messages.push(...pendingMessages);
      }

      if (didMutate) {
        this.messageRevision = null;
      }

      if (persist && didMutate) {
        this.persistHistory();
      }
    },
    setDraftMessage(message: string) {
      this.draftMessage = message;
    },
    setPendingAttachments(attachments: PendingAttachment[]) {
      this.pendingAttachments = attachments;
    },
    clearPendingAttachments() {
      this.pendingAttachments = [];
    },
    removePendingAttachment(id: string) {
      this.pendingAttachments = this.pendingAttachments.filter((attachment) => attachment.id !== id);
    },
    /**
     * 处理选中的文件并加入待发送附件列表。逐文件：类型解析（扩展名优先/MIME 兜底）→
     * 大小上限 → 图片数量上限（3）→ 浏览器模式二进制文档拒绝 → 去重 → 图片魔数嗅探 →
     * 文本内容截断 → 宿主导入（Tauri）或内存引用（浏览器）。
     */
    async addPendingAttachments(
      files: File[],
      options?: ImportAttachmentOptions
    ): Promise<{ added: number; errors: string[] }> {
      const errors: string[] = [];
      let added = 0;

      for (const file of files) {
        const resolution = resolveAttachmentRoute(file.name, file.type);
        if (!resolution) {
          errors.push(`暂不支持该文件类型：${file.name}`);
          continue;
        }
        const { route, spec } = resolution;

        if (file.size > spec.maxBytes) {
          errors.push(`${file.name} 超过大小上限`);
          continue;
        }
        if (
          route === "image" &&
          this.pendingAttachments.filter(
            (attachment) => attachment.route === "image" && attachment.status === "ok"
          ).length >= MAX_TURN_IMAGES
        ) {
          errors.push(`最多添加 ${MAX_TURN_IMAGES} 张图片（${file.name} 未添加）`);
          continue;
        }
        if (route === "document" && !isTauriAvailable()) {
          errors.push(`预览模式暂不支持二进制文档：${file.name}`);
          continue;
        }
        // 去重只看成功条目（失败条目允许重试）；键含 lastModified 降低同名同尺寸误伤
        const dedupKey = attachmentDedupKey({
          name: file.name,
          sizeBytes: file.size,
          lastModified: file.lastModified
        });
        if (
          this.pendingAttachments.some(
            (attachment) => attachment.status === "ok" && attachmentDedupKey(attachment) === dedupKey
          )
        ) {
          errors.push(`已添加过该文件：${file.name}`);
          continue;
        }

        let status: PendingAttachmentStatus = "ok";
        let errorDetail: string | null = null;
        let dataUrl: string | null = null;
        let path: string | null = null;
        let relativePath: string | null = null;
        let content: string | null = null;
        let truncated = false;
        let attachmentMime = file.type || mimeFromExtension(file.name);

        try {
          const bytes = new Uint8Array(await readFileAsArrayBuffer(file));

          if (route === "image") {
            const sniffed = sniffImageKind(bytes);
            if (!sniffed) {
              errors.push(`无效图片文件：${file.name}`);
              continue;
            }
            // I-2：dataUrl mime 必须与真实内容一致（由嗅探结果派生），防"PNG 内容改名 .jpg"
            attachmentMime = mimeForSniffed(sniffed);
            dataUrl = bytesToDataUrl(bytes, attachmentMime);
          } else {
            if (route === "text") {
              const decoded = new TextDecoder().decode(bytes);
              const result = truncateTextByBytes(decoded, spec.injectMaxBytes);
              content = result.text;
              truncated = result.truncated;
            }
            if (bytes.length > 0) {
              const importResult = await importAttachment(file.name, bytes, attachmentMime, options);
              path = importResult.path;
              relativePath = importResult.relativePath ?? null;
            }
            // 空文本（0 字节）：跳过导入（宿主拒绝空字节），path/relativePath 为 null，
            // 内容为空——与浏览器模式行为一致（T-8）。
          }
        } catch (error) {
          status = "error";
          errorDetail = String(error);
        }

        // I-6：在途导入期间用户已提交 → 丢弃本次结果，避免 chip 遗留给下一轮
        if (this.isSubmitting) {
          break;
        }

        this.pendingAttachments = [
          ...this.pendingAttachments,
          {
            id: `pending-${Date.now()}-${this.pendingAttachments.length}-${added}`,
            name: file.name,
            sizeBytes: file.size,
            mimeType: attachmentMime,
            route,
            spec,
            path,
            relativePath,
            dataUrl,
            content,
            truncated,
            lastModified: file.lastModified,
            status,
            errorDetail
          }
        ];
        if (status === "ok") {
          added += 1;
        }
      }

      return { added, errors };
    },
    async fetchHealth() {
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
    async fetchAvailableTools() {
      if (this.availableTools.length > 0 && isTauriAvailable()) {
        const defaultToolNames = new Set(defaultAvailableTools.map((tool) => tool.name));
        const hasDefaultOnly =
          this.availableTools.length === defaultAvailableTools.length &&
          this.availableTools.every((tool) => defaultToolNames.has(tool.name));
        if (!hasDefaultOnly) {
          return;
        }
      }

      try {
        this.availableTools = isTauriAvailable()
          ? await safeInvoke<AvailableTool[]>("list_available_tools")
          : createAvailableTools();
        debugLog("tools:ok", {
          count: this.availableTools.length
        });
      } catch (error) {
        this.availableTools = createAvailableTools();
        debugLog("tools:error", {
          error: String(error)
        });
      }
    },
    async fetchCapabilitySources() {
      try {
        this.capabilitySources = isTauriAvailable()
          ? await safeInvoke<CapabilitySourceView[]>("list_capability_sources")
          : createCapabilitySources();
        debugLog("capability-sources:ok", {
          count: this.capabilitySources.length
        });
      } catch (error) {
        this.capabilitySources = createCapabilitySources();
        debugLog("capability-sources:error", {
          error: String(error)
        });
      }
    },
    async fetchCapabilities(filter?: { sourceId?: string | null; kind?: string | null }) {
      const sourceId = filter?.sourceId?.trim() || null;
      const kind = filter?.kind?.trim() || null;

      try {
        this.capabilities = isTauriAvailable()
          ? await safeInvoke<CapabilityView[]>("list_capabilities", {
            sourceId,
            kind
          })
          : createCapabilities().filter((capability) => {
            if (sourceId && capability.sourceId !== sourceId) {
              return false;
            }
            if (kind && capability.kind !== kind) {
              return false;
            }
            return true;
          });
        debugLog("capabilities:ok", {
          count: this.capabilities.length,
          sourceId,
          kind
        });
      } catch (error) {
        this.capabilities = createCapabilities().filter((capability) => {
          if (sourceId && capability.sourceId !== sourceId) {
            return false;
          }
          if (kind && capability.kind !== kind) {
            return false;
          }
          return true;
        });
        debugLog("capabilities:error", {
          sourceId,
          kind,
          error: String(error)
        });
      }
    },
    async inspectCapability(capabilityId: string) {
      if (!capabilityId.trim()) {
        return null;
      }

      if (!isTauriAvailable()) {
        return createCapabilities().find((capability) => capability.capabilityId === capabilityId) ?? null;
      }

      try {
        return await safeInvoke<CapabilityView | null>("inspect_capability", {
          capabilityId
        });
      } catch (error) {
        debugLog("capability:inspect:error", {
          capabilityId,
          error: String(error)
        });
        return createCapabilities().find((capability) => capability.capabilityId === capabilityId) ?? null;
      }
    },
    async inspectCapabilitySource(sourceId: string) {
      if (!sourceId.trim()) {
        return null;
      }

      if (!isTauriAvailable()) {
        return createCapabilitySources().find((source) => source.sourceId === sourceId) ?? null;
      }

      try {
        return await safeInvoke<CapabilitySourceView | null>("inspect_capability_source", {
          sourceId
        });
      } catch (error) {
        debugLog("capability-source:inspect:error", {
          sourceId,
          error: String(error)
        });
        return createCapabilitySources().find((source) => source.sourceId === sourceId) ?? null;
      }
    },
    async initializeTurnEvents() {
      if (this.eventsReady) {
        return;
      }

      if (!isTauriAvailable()) {
        this.eventsReady = true;
        debugLog("events:browser-preview");
        return;
      }

      debugLog("events:init");

      const startedUnlisten = await safeListen<TurnStreamEvent>("turn:started", ({ payload }) => {
        if (this.activeTurnId !== payload.turnId) {
          return;
        }
        if (!this.shouldProcessTurnEvent(payload)) {
          return;
        }
        this.commitTurnEventCursor(payload);
        this.cancelStreamFlush();
        this.streamBufferTurnId = null;
        this.streamBufferText = "";
        this.streamBufferReasoning = "";
        this.resetStreamDebugMetrics();
        this.clearPendingTraceTimeline();

        this.ensureAssistantMessage(
          payload.turnId,
          buildAssistantModelLabel(payload.providerName, payload.providerModel)
        );
        this.phase = resolveRuntimePhaseFromEvent(payload, "calling_model");
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
      });

      const deltaUnlisten = await safeListen<TurnStreamEvent>("turn:delta", ({ payload }) => {
        if (this.activeTurnId !== payload.turnId) {
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

        this.ensureAssistantMessage(
          payload.turnId,
          buildAssistantModelLabel(this.providerName, this.providerModel)
        );

        const deltaText = payload.text ?? "";
        const deltaReasoning = payload.reasoningContent ?? "";

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

      const traceUnlisten = await safeListen<TurnStreamEvent>("turn:trace", ({ payload }) => {
        if (this.activeTurnId !== payload.turnId) {
          return;
        }
        if (!this.shouldProcessTurnEvent(payload)) {
          return;
        }
        this.commitTurnEventCursor(payload);
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

      const phaseChangedUnlisten = await safeListen<TurnStreamEvent>("turn:phase_changed", ({ payload }) => {
        if (this.activeTurnId !== payload.turnId) {
          return;
        }
        if (!this.shouldProcessTurnEvent(payload)) {
          return;
        }
        this.commitTurnEventCursor(payload);
        this.flushBufferedStreamText(payload.turnId);
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
      });

      const checkpointPersistedUnlisten = await safeListen<TurnStreamEvent>("turn:checkpoint_persisted", ({ payload }) => {
        if (this.activeTurnId !== payload.turnId) {
          return;
        }
        if (!this.shouldProcessTurnEvent(payload)) {
          return;
        }
        this.commitTurnEventCursor(payload);
        this.flushBufferedStreamText(payload.turnId);
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

      const toolUnlisten = await safeListen<TurnStreamEvent>("turn:tool", ({ payload }) => {
        if (this.activeTurnId !== payload.turnId) {
          return;
        }
        if (!this.shouldProcessTurnEvent(payload)) {
          return;
        }
        this.commitTurnEventCursor(payload);
        this.flushBufferedStreamText(payload.turnId);
        this.flushPendingTraceTimeline();

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

      const outputEndUnlisten = await safeListen<TurnStreamEvent>("turn:output_end", ({ payload }) => {
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
        this.applyOutputEnd(payload);
        debugLog("event:output_end", {
          turnId: payload.turnId,
          finalTextLength: payload.text?.length ?? 0,
          outputTokens: payload.outputTokens ?? null,
          turnDurationMs: payload.turnDurationMs ?? null
        });
      });

      const hopCompleteUnlisten = await safeListen<TurnStreamEvent>("turn:hop_complete", ({ payload }) => {
        if (this.activeTurnId !== payload.turnId) {
          return;
        }
        if (!this.shouldProcessTurnEvent(payload)) {
          return;
        }
        this.commitTurnEventCursor(payload);
        this.flushBufferedStreamText(payload.turnId);
        this.inputTokens = payload.inputTokens ?? this.inputTokens;
        this.outputTokens = payload.outputTokens ?? this.outputTokens;
        this.totalTokens = payload.totalTokens ?? this.totalTokens;
        this.firstTokenLatencyMs = payload.firstTokenLatencyMs ?? this.firstTokenLatencyMs;
        debugLog("event:hop_complete", {
          turnId: payload.turnId,
          inputTokens: payload.inputTokens ?? null,
          outputTokens: payload.outputTokens ?? null
        });
      });

      const completedUnlisten = await safeListen<TurnStreamEvent>("turn:completed", ({ payload }) => {
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
        if (this.activeTurnId !== payload.turnId) {
          return;
        }
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

        const completedPayloadRecord = payload as Record<string, unknown>;

        const assistantMessage = this.ensureAssistantMessage(
          payload.turnId,
          buildAssistantModelLabel(payload.providerName, payload.providerModel)
        );

        const finalText = payload.text?.trim();
        if (finalText && payload.text !== assistantMessage.content) {
          assistantMessage.content = payload.text ?? assistantMessage.content;
        }
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

      const failedUnlisten = await safeListen<TurnStreamEvent>("turn:failed", ({ payload }) => {
        if (payload.sessionId) {
          const bg = this.runningSessionMap[payload.sessionId];
          if (bg && bg.turnId === payload.turnId) {
            this.updatePersistedBackgroundSession(payload.sessionId, payload.turnId, {
              content: payload.text ?? DEFAULT_FAILED_TURN_MESSAGE,
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
        if (this.activeTurnId !== payload.turnId) {
          return;
        }
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
        assistantMessage.content = payload.text ?? DEFAULT_FAILED_TURN_MESSAGE;
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
        }, 0);
      });

      const cancelledUnlisten = await safeListen<TurnStreamEvent>("turn:cancelled", ({ payload }) => {
        // Detect terminal event for a background session's turn
        if (payload.sessionId) {
          const bg = this.runningSessionMap[payload.sessionId];
          if (bg && bg.turnId === payload.turnId) {
            delete this.runningSessionMap[payload.sessionId];
            return;
          }
        }

        if (this.activeTurnId !== payload.turnId) {
          return;
        }
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

        assistantMessage.content = payload.text ?? "本轮已停止。";
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

      void startedUnlisten;
      void deltaUnlisten;
      void traceUnlisten;
      void phaseChangedUnlisten;
      void checkpointPersistedUnlisten;
      void toolUnlisten;
      void outputEndUnlisten;
      void hopCompleteUnlisten;
      void completedUnlisten;
      void failedUnlisten;
      void cancelledUnlisten;
      this.eventsReady = true;
    },
    async runBrowserPreviewTurn(requestId: string) {
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
      this.providerProtocol = provider?.protocol ?? "openai";
      this.providerModel = previewModelName;
      this.providerSource = "browser_preview";
      this.providerMode = "browser_preview";
      this.inputTokens = null;
      this.outputTokens = null;
      this.totalTokens = null;
      this.firstTokenLatencyMs = null;
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
    async stopTurn() {
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
          await safeInvoke("stop_turn", {
            turnId: this.activeTurnId
          });
        }
        return true;
      } catch (error) {
        this.error = `停止当前轮次失败：${String(error)}`;
        return false;
      }
    },
    async submitTurn(options?: { images?: TurnInputImage[] }) {
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
          name: attachment.name
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
        sessionId: this.sessionId,
        nodeId: this.visibleNodeId,
        history: buildTurnHistory(this.messages),
        images,
        // PA-079 传输通道：本轮恒 null（→ 默认 workspace）；PA-081 起改传 activeWorkspaceId
        workspaceId: null
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

        let submission: GraphRunSubmission | null = null;
        if (this.sessionId) {
          try {
            const plan = await safeInvoke<GraphRunSubmissionPlan>("resolve_graph_run_submission_plan", {
              sessionId: this.sessionId,
              nodeId: this.visibleNodeId,
              runId: this.activeRunId
            });
            submission = resolveGraphRunSubmissionFromPlan(plan);
          } catch (planError) {
            debugLog("submit:resolve-plan:error", {
              sessionId: this.sessionId,
              runId: this.activeRunId,
              error: String(planError)
            });
          }
        }

        if (!submission) {
          submission = resolveGraphRunSubmissionFromPlan(this.latestGraphRunSubmissionPlan);
        }

        if (!submission) {
          submission = resolveGraphRunSubmissionFromRunState(this.retrievedContext?.runState);
          submission = reconcileSubmissionWithRecoveryCheckpoint(submission, this.latestExecutionCheckpoint);
          submission ??= resolveGraphRunSubmissionFromCheckpoint(
            this.latestExecutionCheckpoint,
            this.activeRunId
          );
        }

        if (!submission && this.sessionId) {
          await this.resolveDerivedSessionRun({
            sessionId: this.sessionId,
            runId: this.activeRunId,
            preferRefresh: true,
            nodeId: this.visibleNodeId
          });
          submission = resolveGraphRunSubmissionFromRunState(this.retrievedContext?.runState);
          submission = reconcileSubmissionWithRecoveryCheckpoint(submission, this.latestExecutionCheckpoint);
          submission ??= resolveGraphRunSubmissionFromCheckpoint(
            this.latestExecutionCheckpoint,
            this.activeRunId
          );
        }
        submission ??= { command: "start_graph_run_stream" as const, runId: null };

        if (submission.command === "start_graph_run_stream") {
          const response = await safeInvoke<GraphRunStreamStartResponse>("start_graph_run_stream", {
            turnId: requestId,
            runId: null,
            goal: displayMessage,
            input: payload
          });
          this.activeRunId = resolvedStreamStartRunId(response, null);
          this.latestGraphRunControlBoundaryEvidence = cloneGraphRunControlBoundaryEvidence(
            response.controlBoundaryEvidence ? [response.controlBoundaryEvidence] : null
          );
          this.latestRunControlAuditSummary = cloneRunControlAuditSummary(
            response.runControlAuditSummary ?? null
          );
          return true;
        }

        if (submission.command === "resume_graph_run_stream") {
          const response = await safeInvoke<GraphRunStreamStartResponse>("resume_graph_run_stream", {
            turnId: requestId,
            runId: submission.runId,
            input: payload
          });
          this.activeRunId = resolvedStreamStartRunId(response, submission.runId);
          this.latestGraphRunControlBoundaryEvidence = cloneGraphRunControlBoundaryEvidence(
            response.controlBoundaryEvidence ? [response.controlBoundaryEvidence] : null
          );
          this.latestRunControlAuditSummary = cloneRunControlAuditSummary(
            response.runControlAuditSummary ?? null
          );
          return true;
        }

        const response = await safeInvoke<GraphRunStreamStartResponse>("continue_graph_run_stream", {
          turnId: requestId,
          runId: submission.runId,
          input: payload
        });
        this.activeRunId = resolvedStreamStartRunId(response, submission.runId);
        this.latestGraphRunControlBoundaryEvidence = cloneGraphRunControlBoundaryEvidence(
          response.controlBoundaryEvidence ? [response.controlBoundaryEvidence] : null
        );
        this.latestRunControlAuditSummary = cloneRunControlAuditSummary(
          response.runControlAuditSummary ?? null
        );
        return true;
      } catch (error) {
        const assistantMessage = this.ensureAssistantMessage(
          requestId,
          buildAssistantModelLabel(selectedProviderName, selectedModelName)
        );
        assistantMessage.content = DEFAULT_FAILED_TURN_MESSAGE;
        assistantMessage.reasoningContent = null;
        assistantMessage.status = "error";
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
    }
  }
});
