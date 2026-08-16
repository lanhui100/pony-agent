// sessions domain：会话快照创建/恢复、localStorage 持久化、retrieved context 派生、
// session overview 构建与初始化策略裁决。依赖各 domain（trace/messages/history/attachments）与 phases 上游模块。
import {
  CACHED_STATE_VERSION,
  DEFAULT_BROWSER_SESSION_SUMMARY,
  DEFAULT_SESSION_ID,
  RETRIEVED_CONTEXT_FALLBACK_SUMMARY,
  RUNTIME_STORAGE_KEY
} from "./constants";
import { debugLog } from "./utils";
import { cloneAttachmentAssets } from "./attachments";
import {
  cloneHistoryBranches,
  cloneHistoryNodes,
  cloneHistoryStateAuditSummary,
  cloneRunControlAuditSummary,
  normalizeHistoryCursorMode,
  resolveHistoryBranchHeadNodeId
} from "./history";
import { buildSessionTitleFromMessages, buildTurnHistory, cloneMessages } from "./messages";
import {
  cloneToolActivities,
  cloneTraceSteps,
  cloneTraceTimeline,
  normalizeTurnTraceRecord
} from "./trace";
import type {
  PersistedRuntimeCache,
  PersistedRuntimeState,
  RunningTurn,
  RuntimeState,
  SessionInitializationStrategy,
  SessionRuntimeSnapshot
} from "./types";
import type {
  ChatMessage,
  GraphRunControlBoundaryEvidence,
  RetrievedContextState,
  RuntimePhase,
  SessionOverview,
  SessionRuntimeView,
  SessionSnapshot,
  TurnStreamEvent,
  TurnTraceRecord
} from "../../types/runtime";

export function cloneGraphRunControlBoundaryEvidence(
  evidence?: GraphRunControlBoundaryEvidence[] | null
): GraphRunControlBoundaryEvidence[] {
  return (evidence ?? []).map((item) => ({
    ...item,
    hookEnvelope: { ...item.hookEnvelope }
  }));
}

export function cloneRetrievedContext(retrievedContext?: RetrievedContextState | null) {
  if (!retrievedContext) {
    return null;
  }

  return {
    turnContext: {
      ...retrievedContext.turnContext,
      images: (retrievedContext.turnContext.images ?? []).map((image) => ({ ...image }))
    },
    sessionContext: {
      ...retrievedContext.sessionContext,
      recentHistory: (retrievedContext.sessionContext.recentHistory ?? []).map((message) => ({
        ...message,
        attachments: (message.attachments ?? []).map((attachment) => ({ ...attachment }))
      })),
      recentAttachmentAssets: cloneAttachmentAssets(retrievedContext.sessionContext.recentAttachmentAssets)
    },
    runState: { ...retrievedContext.runState },
    longTermMemory: {
      ...retrievedContext.longTermMemory,
      entries: (retrievedContext.longTermMemory.entries ?? []).map((entry) => ({ ...entry }))
    },
    transcript: {
      providerNativeMessages: [...(retrievedContext.transcript.providerNativeMessages ?? [])]
    }
  };
}

export function createBlankSessionRuntimeFields() {
  return {
    sessionSummary: "",
    providerRequestedName: "",
    providerName: "",
    providerProtocol: "",
    providerModel: "",
    providerSource: "",
    providerMode: "",
    fallbackReason: null as string | null,
    inputTokens: null as number | null,
    outputTokens: null as number | null,
    totalTokens: null as number | null,
    firstTokenLatencyMs: null as number | null
  };
}

export function createSessionRuntimeSnapshot(state: RuntimeState): SessionRuntimeSnapshot {
  return {
    sessionId: state.sessionId,
    sessionList: state.sessionList.map((session) => ({ ...session })),
    deletingSessionSet: { ...state.deletingSessionSet },
    phase: state.phase,
    error: state.error,
    draftMessage: state.draftMessage,
    sessionSummary: state.sessionSummary,
    retrievedContext: cloneRetrievedContext(state.retrievedContext),
    providerRequestedName: state.providerRequestedName,
    providerName: state.providerName,
    providerProtocol: state.providerProtocol,
    providerModel: state.providerModel,
    providerSource: state.providerSource,
    providerMode: state.providerMode,
    fallbackReason: state.fallbackReason,
    inputTokens: state.inputTokens,
    outputTokens: state.outputTokens,
    totalTokens: state.totalTokens,
    firstTokenLatencyMs: state.firstTokenLatencyMs,
    isSubmitting: state.isSubmitting,
    activeTurnId: state.activeTurnId,
    activeRunId: state.activeRunId,
    latestExecutionCheckpoint: state.latestExecutionCheckpoint ? { ...state.latestExecutionCheckpoint } : null,
    latestGraphRunSubmissionPlan: state.latestGraphRunSubmissionPlan
      ? { ...state.latestGraphRunSubmissionPlan }
      : null,
    latestGraphRunControlBoundaryEvidence: cloneGraphRunControlBoundaryEvidence(
      state.latestGraphRunControlBoundaryEvidence
    ),
    latestRunControlAuditSummary: cloneRunControlAuditSummary(state.latestRunControlAuditSummary),
    latestHistoryStateAuditSummary: cloneHistoryStateAuditSummary(
      state.latestHistoryStateAuditSummary
    ),
    visibleNodeId: state.visibleNodeId,
    branchHeadNodeId: state.branchHeadNodeId,
    activeBranchId: state.activeBranchId,
    cursorVersion: state.cursorVersion,
    messageRevision: state.messageRevision,
    historyCursorMode: state.historyCursorMode,
    historyNodes: cloneHistoryNodes(state.historyNodes),
    historyBranches: cloneHistoryBranches(state.historyBranches),
    initialRollbackActive: state.initialRollbackActive,
    messages: cloneMessages(state.messages),
    attachmentAssets: cloneAttachmentAssets(state.attachmentAssets),
    toolActivities: cloneToolActivities(state.toolActivities),
    traceSteps: cloneTraceSteps(state.traceSteps),
    traceTimeline: cloneTraceTimeline(state.traceTimeline),
    turnTraceHistory: state.turnTraceHistory.map((trace) => normalizeTurnTraceRecord(trace)),
    eventCursorByTurnId: Object.fromEntries(
      Object.entries(state.eventCursorByTurnId).map(([turnId, cursor]) => [turnId, { ...cursor }])
    )
  };
}

export function restoreSessionRuntimeSnapshot(state: RuntimeState, snapshot: SessionRuntimeSnapshot) {
  if (state.streamFlushFrameId != null) {
    window.cancelAnimationFrame(state.streamFlushFrameId);
  }
  if (state.streamFlushTimerId != null) {
    window.clearTimeout(state.streamFlushTimerId);
  }
  state.sessionId = snapshot.sessionId;
  state.sessionList = snapshot.sessionList.map((session) => ({ ...session }));
  state.deletingSessionSet = { ...snapshot.deletingSessionSet };
  state.phase = snapshot.phase;
  state.error = snapshot.error;
  state.draftMessage = snapshot.draftMessage;
  state.sessionSummary = snapshot.sessionSummary;
  state.retrievedContext = cloneRetrievedContext(snapshot.retrievedContext);
  state.providerRequestedName = snapshot.providerRequestedName;
  state.providerName = snapshot.providerName;
  state.providerProtocol = snapshot.providerProtocol;
  state.providerModel = snapshot.providerModel;
  state.providerSource = snapshot.providerSource;
  state.providerMode = snapshot.providerMode;
  state.fallbackReason = snapshot.fallbackReason;
  state.inputTokens = snapshot.inputTokens;
  state.outputTokens = snapshot.outputTokens;
  state.totalTokens = snapshot.totalTokens;
  state.firstTokenLatencyMs = snapshot.firstTokenLatencyMs;
  state.isSubmitting = snapshot.isSubmitting;
  state.activeTurnId = snapshot.activeTurnId;
  state.activeRunId = snapshot.activeRunId;
  state.streamFlushFrameId = null;
  state.streamFlushTimerId = null;
  state.latestExecutionCheckpoint = snapshot.latestExecutionCheckpoint ? { ...snapshot.latestExecutionCheckpoint } : null;
  state.latestGraphRunSubmissionPlan = snapshot.latestGraphRunSubmissionPlan
    ? { ...snapshot.latestGraphRunSubmissionPlan }
    : null;
  state.latestGraphRunControlBoundaryEvidence = cloneGraphRunControlBoundaryEvidence(
    snapshot.latestGraphRunControlBoundaryEvidence
  );
  state.latestRunControlAuditSummary = cloneRunControlAuditSummary(snapshot.latestRunControlAuditSummary);
  state.latestHistoryStateAuditSummary = cloneHistoryStateAuditSummary(
    snapshot.latestHistoryStateAuditSummary
  );
  state.visibleNodeId = snapshot.visibleNodeId;
  state.branchHeadNodeId = snapshot.branchHeadNodeId;
  state.activeBranchId = snapshot.activeBranchId;
  state.messageRevision = snapshot.messageRevision;
  state.historyCursorMode = snapshot.historyCursorMode;
  state.historyNodes = cloneHistoryNodes(snapshot.historyNodes);
  state.historyBranches = cloneHistoryBranches(snapshot.historyBranches);
  state.initialRollbackActive = snapshot.initialRollbackActive;
  state.messages = cloneMessages(snapshot.messages);
  state.attachmentAssets = cloneAttachmentAssets(snapshot.attachmentAssets);
  state.toolActivities = cloneToolActivities(snapshot.toolActivities);
  state.traceSteps = cloneTraceSteps(snapshot.traceSteps);
  state.traceTimeline = cloneTraceTimeline(snapshot.traceTimeline);
  state.turnTraceHistory = snapshot.turnTraceHistory.map((trace) => normalizeTurnTraceRecord(trace));
  state.eventCursorByTurnId = Object.fromEntries(
    Object.entries(snapshot.eventCursorByTurnId ?? {}).map(([turnId, cursor]) => [turnId, { ...cursor }])
  );
  state.streamFlushFrameId = null;
  state.streamBufferTurnId = null;
  state.streamBufferText = "";
  state.streamBufferReasoning = "";
}

export function filterDeletingSessions(
  sessions: SessionOverview[],
  deletingSessionSet: Record<string, boolean>
): SessionOverview[] {
  return sessions.filter((session) => !deletingSessionSet[session.conversationId]);
}

export function buildEventCursorByTurnTraceHistory(turnTraceHistory: TurnTraceRecord[]) {
  const entries = turnTraceHistory
    .filter((trace) => trace.turnId)
    .map((trace) => [
      trace.turnId,
      {
        eventId: trace.eventId ?? null,
        sequence: trace.sequence ?? null,
        emittedAtMs: trace.emittedAtMs ?? null
      }
    ] as const);

  return Object.fromEntries(entries);
}

export function shouldAcceptTurnEvent(
  currentCursor: { eventId: string | null; sequence: number | null; emittedAtMs: number | null } | null | undefined,
  payload: Pick<TurnStreamEvent, "eventId" | "sequence" | "emittedAtMs">,
  options: { allowSameSequenceDifferentEventId?: boolean } = {}
) {
  const nextEventId = payload.eventId?.trim() || null;
  const nextSequence = typeof payload.sequence === "number" && Number.isFinite(payload.sequence) ? payload.sequence : null;
  const nextEmittedAtMs =
    typeof payload.emittedAtMs === "number" && Number.isFinite(payload.emittedAtMs) ? payload.emittedAtMs : null;
  if (!currentCursor) {
    return true;
  }

  if (nextEventId && currentCursor.eventId && nextEventId === currentCursor.eventId) {
    return false;
  }

  if (nextSequence != null && currentCursor.sequence != null) {
    if (nextSequence < currentCursor.sequence) {
      return false;
    }

    if (nextSequence === currentCursor.sequence) {
      // 默认：同 sequence 不同 eventId 视为重复丢弃。
      // 终态事件（completed/failed/cancelled）允许放行——output_end 与终态事件
      // 常在同一 sequence 批次内连发（不同 eventId），误杀会导致状态永久卡死。
      if (!options.allowSameSequenceDifferentEventId) {
        if (!nextEventId || !currentCursor.eventId || nextEventId !== currentCursor.eventId) {
          return false;
        }
      }
    }
  }

  if (
    nextSequence == null &&
    nextEmittedAtMs != null &&
    currentCursor.sequence == null &&
    currentCursor.emittedAtMs != null &&
    nextEmittedAtMs < currentCursor.emittedAtMs
  ) {
    return false;
  }

  return true;
}

export function createSnapshotFromRuntimeState(state: RuntimeState, sessionId: string): SessionSnapshot {
  const retrievedSummary = state.retrievedContext?.sessionContext?.summary?.trim() ?? "";
  return {
    conversationId: sessionId,
    title: buildSessionTitleFromMessages(state.messages),
    summary: retrievedSummary || state.sessionSummary || DEFAULT_BROWSER_SESSION_SUMMARY,
    history: buildTurnHistory(state.messages),
    attachmentAssets: cloneAttachmentAssets(state.attachmentAssets),
    turnTraceHistory: state.turnTraceHistory.map((trace) => normalizeTurnTraceRecord(trace)),
    historyStateEvidence: [],
    historyStateAuditSummary: null,
    turnCount: state.messages.filter((message) => message.role === "user").length,
    lastReferencedFile: null,
    updatedAtMs:
      state.turnTraceHistory.length > 0
        ? state.turnTraceHistory[state.turnTraceHistory.length - 1].updatedAt
        : Date.now()
  };
}

export function deriveRetrievedContextFromSnapshot(snapshot: SessionSnapshot): RetrievedContextState {
  const recentHistory = snapshot.history.slice(-12).map((message) => ({
    ...message,
    attachments: (message.attachments ?? []).map((attachment) => ({ ...attachment }))
  }));
  const recentAttachmentAssets = cloneAttachmentAssets(snapshot.attachmentAssets ?? []).slice(-8);
  const lastUserMessage =
    [...snapshot.history].reverse().find((message) => message.role === "user")?.content ?? "";

  return {
    turnContext: {
      userMessage: lastUserMessage,
      images: [],
      referencesImage: false
    },
    sessionContext: {
      conversationId: snapshot.conversationId,
      title: snapshot.title ?? "新对话",
      summary: snapshot.summary,
      recentHistory,
      recentAttachmentAssets,
      turnCount: snapshot.turnCount,
      lastReferencedFile: snapshot.lastReferencedFile ?? null
    },
    runState: {},
    longTermMemory: {
      status: "empty",
      summary: RETRIEVED_CONTEXT_FALLBACK_SUMMARY,
      entries: []
    },
    transcript: {
      providerNativeMessages: []
    }
  };
}

export function isValidPersistedState(state: unknown): state is PersistedRuntimeState {
  if (state == null || typeof state !== "object") {
    return false;
  }
  const s = state as Record<string, unknown>;
  const version = s.cachedStateVersion;
  if (version !== undefined && version !== CACHED_STATE_VERSION) {
    return false;
  }
  return Array.isArray(s.messages);
}

export function loadPersistedRuntimeCache(): PersistedRuntimeCache {
  if (typeof window === "undefined") {
    return { sessions: {} };
  }

  try {
    const raw = window.localStorage.getItem(RUNTIME_STORAGE_KEY);
    if (!raw) {
      debugLog("restore:empty");
      return { sessions: {} };
    }

    const parsed = JSON.parse(raw) as PersistedRuntimeCache;
    const allSessions = parsed.sessions ?? {};
    const validCount = Object.values(allSessions).filter((s) => isValidPersistedState(s)).length;
    debugLog("restore:ok", {
      sessions: Object.keys(allSessions).length,
      valid: validCount,
      stale: Object.keys(allSessions).length - validCount
    });
    return {
      sessions: allSessions
    };
  } catch {
    debugLog("restore:error");
    return { sessions: {} };
  }
}

export function loadPersistedRuntimeState(sessionId: string): PersistedRuntimeState | null {
  const raw = loadPersistedRuntimeCache().sessions[sessionId];
  if (raw && isValidPersistedState(raw)) {
    return raw;
  }
  if (raw) {
    debugLog("persist:stale", { sessionId, version: (raw as Partial<PersistedRuntimeState>).cachedStateVersion });
  }
  return null;
}

export function buildRuntimeViewFromPersistedState(
  sessionId: string,
  persisted: PersistedRuntimeState | null,
  nodeId?: string | null
): SessionRuntimeView {
  const persistedVisibleNodeId = persisted?.visibleNodeId?.trim() || null;
  const requestedVisibleNodeId = nodeId?.trim() || persistedVisibleNodeId;
  const persistedBranchHeadNodeId = persisted?.branchHeadNodeId?.trim() || null;
  const persistedActiveBranchId = persisted?.activeBranchId?.trim() || null;
  const persistedHistoryNodes = cloneHistoryNodes(persisted?.historyNodes);
  const persistedHistoryBranches = cloneHistoryBranches(persisted?.historyBranches);
  const persistedHistoryCursor =
    requestedVisibleNodeId || persistedBranchHeadNodeId || persistedActiveBranchId
      ? {
          sessionId,
          visibleNodeId: requestedVisibleNodeId,
          activeBranchId: persistedActiveBranchId,
          branchHeadNodeId:
            persistedBranchHeadNodeId ||
            resolveHistoryBranchHeadNodeId(persistedActiveBranchId, persistedHistoryBranches),
          workspaceNodeId: requestedVisibleNodeId,
          mode: normalizeHistoryCursorMode(
            persisted?.historyCursorMode ??
              (requestedVisibleNodeId && persistedBranchHeadNodeId && requestedVisibleNodeId !== persistedBranchHeadNodeId
                ? "historical"
                : "live")
          ),
          authorityMode: "local_preview",
          cursorVersion: persisted?.cursorVersion ?? null,
          isAtBranchHead:
            !!requestedVisibleNodeId &&
            !!(persistedBranchHeadNodeId || resolveHistoryBranchHeadNodeId(persistedActiveBranchId, persistedHistoryBranches)) &&
            requestedVisibleNodeId ===
              (persistedBranchHeadNodeId || resolveHistoryBranchHeadNodeId(persistedActiveBranchId, persistedHistoryBranches))
        }
      : null;
  const snapshot = {
    conversationId: sessionId,
    title: buildSessionTitleFromMessages(persisted?.messages ?? []),
    summary: persisted?.sessionSummary ?? (persisted?.messages?.length ? DEFAULT_BROWSER_SESSION_SUMMARY : ""),
    history: buildTurnHistory(persisted?.messages ?? []),
    attachmentAssets: persisted?.attachmentAssets ?? [],
    turnTraceHistory: persisted?.turnTraceHistory ?? [],
    turnCount: persisted?.messages?.filter((message) => message.role === "user").length ?? 0,
    historyStateEvidence: [],
    historyStateAuditSummary: null,
    runControlAuditSummary: null,
    lastReferencedFile: null,
    updatedAtMs:
      persisted?.turnTraceHistory?.length
        ? persisted.turnTraceHistory[persisted.turnTraceHistory.length - 1]!.updatedAt
        : Date.now()
  } satisfies SessionSnapshot;

  return {
    session: snapshot,
    historyStateEvidence: snapshot.historyStateEvidence ?? [],
    historyStateAuditSummary: snapshot.historyStateAuditSummary ?? null,
    runControlAuditSummary: snapshot.runControlAuditSummary ?? null,
    retrieved: deriveRetrievedContextFromSnapshot(snapshot),
    checkpoint: null,
    submissionPlan: null,
    controlBoundaryEvidence: null,
    historyNodes: persistedHistoryNodes.length > 0 ? persistedHistoryNodes : undefined,
    historyBranches: persistedHistoryBranches.length > 0 ? persistedHistoryBranches : undefined,
    historyCursor: persistedHistoryCursor,
    authorityMode: "local_preview",
    resolvedVisibleNodeId: requestedVisibleNodeId,
    activeBranchHeadNodeId:
      persistedBranchHeadNodeId || resolveHistoryBranchHeadNodeId(persistedActiveBranchId, persistedHistoryBranches),
    isAtBranchHead:
      !!requestedVisibleNodeId &&
      !!(persistedBranchHeadNodeId || resolveHistoryBranchHeadNodeId(persistedActiveBranchId, persistedHistoryBranches)) &&
      requestedVisibleNodeId ===
        (persistedBranchHeadNodeId || resolveHistoryBranchHeadNodeId(persistedActiveBranchId, persistedHistoryBranches)),
    cursorVersion: persisted?.cursorVersion ?? null
  } satisfies SessionRuntimeView;
}

export function persistSessionState(sessionId: string, payload: PersistedRuntimeState) {
  if (typeof window === "undefined") {
    return;
  }

  const cache = loadPersistedRuntimeCache();
  cache.sessions[sessionId] = payload;
  window.localStorage.setItem(RUNTIME_STORAGE_KEY, JSON.stringify(cache));
}

export function persistSessionStateAndRuntimeMaps(
  sessionId: string,
  payload: PersistedRuntimeState,
  map: Record<string, RunningTurn>,
  completedSessionSet: Record<string, boolean>,
  failedSessionSet: Record<string, boolean>
) {
  if (typeof window === "undefined") {
    return;
  }

  try {
    const cache = loadPersistedRuntimeCache();
    cache.sessions[sessionId] = payload;
    const stripped: Record<string, { turnId: string; phase: RuntimePhase; textBuffer: string; reasoningBuffer: string }> = {};
    for (const [sid, turn] of Object.entries(map)) {
      stripped[sid] = {
        turnId: turn.turnId,
        phase: turn.phase,
        textBuffer: turn.textBuffer.slice(-5000),
        reasoningBuffer: turn.reasoningBuffer.slice(-2000)
      };
    }
    cache.runningSessionMap = stripped;
    cache.completedSet = { ...completedSessionSet };
    cache.failedSet = { ...failedSessionSet };
    // 合并 sessions + running map 为单次 load-modify-save，
    // 避免 persistHistory 路径上背靠背两次全量 JSON 序列化。
    window.localStorage.setItem(RUNTIME_STORAGE_KEY, JSON.stringify(cache));
  } catch {
    debugLog("persist:session-and-maps:error");
  }
}

export function persistRunningSessionMap(
  map: Record<string, RunningTurn>,
  completedSessionSet: Record<string, boolean>,
  failedSessionSet: Record<string, boolean>
) {
  if (typeof window === "undefined") {
    return;
  }
  try {
    const cache = loadPersistedRuntimeCache();
    const stripped: Record<string, { turnId: string; phase: RuntimePhase; textBuffer: string; reasoningBuffer: string }> = {};
    for (const [sid, turn] of Object.entries(map)) {
      stripped[sid] = {
        turnId: turn.turnId,
        phase: turn.phase,
        textBuffer: turn.textBuffer.slice(-5000),
        reasoningBuffer: turn.reasoningBuffer.slice(-2000)
      };
    }
    cache.runningSessionMap = stripped;
    cache.completedSet = { ...completedSessionSet };
    cache.failedSet = { ...failedSessionSet };
    window.localStorage.setItem(RUNTIME_STORAGE_KEY, JSON.stringify(cache));
  } catch {
    debugLog("persist:running-map:error");
  }
}

export function loadRunningSessionMap(): Record<string, RunningTurn> {
  try {
    const raw = loadPersistedRuntimeCache().runningSessionMap ?? {};
    // Normalize old format (without buffers) to new format
    const normalized: Record<string, RunningTurn> = {};
    for (const [sid, turn] of Object.entries(raw)) {
      normalized[sid] = {
        turnId: (turn as any).turnId ?? "",
        phase: (turn as any).phase ?? "idle",
        textBuffer: (turn as any).textBuffer ?? "",
        reasoningBuffer: (turn as any).reasoningBuffer ?? ""
      };
    }
    return normalized;
  } catch {
    return {};
  }
}

export function removePersistedSessionState(sessionId: string) {
  if (typeof window === "undefined") {
    return;
  }

  const cache = loadPersistedRuntimeCache();
  delete cache.sessions[sessionId];
  window.localStorage.setItem(RUNTIME_STORAGE_KEY, JSON.stringify(cache));
}

export function mergeRuntimeViews(
  cached: SessionRuntimeView,
  host: SessionRuntimeView | null
): SessionRuntimeView {
  if (!host) {
    return cached;
  }
  return {
    ...cached,
    checkpoint: host.checkpoint ?? cached.checkpoint,
    submissionPlan: host.submissionPlan ?? cached.submissionPlan,
    controlBoundaryEvidence: host.controlBoundaryEvidence?.length
      ? host.controlBoundaryEvidence
      : cached.controlBoundaryEvidence,
    runControlAuditSummary: host.runControlAuditSummary ?? cached.runControlAuditSummary,
    historyStateAuditSummary: host.historyStateAuditSummary ?? cached.historyStateAuditSummary,
    retrieved: host.retrieved ?? cached.retrieved,
    authorityMode: host.authorityMode ?? cached.authorityMode,
    resolvedVisibleNodeId: host.resolvedVisibleNodeId ?? cached.resolvedVisibleNodeId,
    activeBranchHeadNodeId: host.activeBranchHeadNodeId ?? cached.activeBranchHeadNodeId,
    isAtBranchHead: host.isAtBranchHead ?? cached.isAtBranchHead,
    historyNodes: host.historyNodes?.length ? host.historyNodes : cached.historyNodes,
    historyBranches: host.historyBranches?.length ? host.historyBranches : cached.historyBranches,
    historyCursor: host.historyCursor ?? cached.historyCursor,
    session: {
      ...cached.session,
      turnTraceHistory: cached.session.turnTraceHistory?.length
        ? cached.session.turnTraceHistory
        : host.session.turnTraceHistory,
      historyStateEvidence: (host.historyStateEvidence?.length
        ? host.historyStateEvidence
        : cached.historyStateEvidence) ?? undefined,
      historyStateAuditSummary: host.historyStateAuditSummary ?? cached.session.historyStateAuditSummary ?? null,
      runControlAuditSummary: host.runControlAuditSummary ?? cached.session.runControlAuditSummary ?? null,
    }
  };
}

export function deriveInitStrategy(
  currentSessionId: string,
  persisted: PersistedRuntimeState | null,
  sessionList: SessionOverview[]
): SessionInitializationStrategy {
  if (persisted) {
    const hasValidCheckpoint = persisted.checkpoint != null;
    const isNonIdle = persisted.phase != null && persisted.phase !== "idle";
    if (hasValidCheckpoint && isNonIdle) {
      return { kind: "local-cache", persistedState: persisted };
    }
    return { kind: "host-read", sessionId: currentSessionId, reason: "insufficient-checkpoint" };
  }
  const targetSessionId = sessionList[0]?.conversationId ?? currentSessionId;
  if (sessionList.length > 0 || targetSessionId !== DEFAULT_SESSION_ID) {
    return { kind: "host-read", sessionId: targetSessionId, reason: "no-cache" };
  }
  return { kind: "empty-fallback", sessionId: currentSessionId };
}

export function hasPersistableMessages(messages: ChatMessage[]) {
  return messages.some(
    (message) =>
      (message.role === "user" || message.role === "assistant") && message.content.trim().length > 0
  );
}

export function createTransientSessionOverview(sessionId: string): SessionOverview {
  return {
    conversationId: sessionId,
    title: "新对话",
    summary: "发送第一条消息后保存到历史",
    turnCount: 0,
    lastReferencedFile: null,
    updatedAtMs: 0,
    workspaceId: null
  };
}

export function buildSessionOverviewFromPersistedState(
  conversationId: string,
  state: PersistedRuntimeState
): SessionOverview {
  const legacyTraceHistory = state.turnTraceHistory ?? [];
  return {
    conversationId,
    title: buildSessionTitleFromMessages(state.messages),
    summary: state.sessionSummary || DEFAULT_BROWSER_SESSION_SUMMARY,
    turnCount: state.messages.filter((message) => message.role === "user").length,
    lastReferencedFile: null,
    updatedAtMs:
      legacyTraceHistory.length > 0
        ? legacyTraceHistory[legacyTraceHistory.length - 1]!.updatedAt
        : Date.now(),
    workspaceId: null
  };
}

export function buildSessionOverviewFromRuntimeState(state: Pick<RuntimeState, "sessionId" | "sessionSummary" | "messages" | "turnTraceHistory">): SessionOverview | null {
  if (!hasPersistableMessages(state.messages)) {
    return null;
  }

  const latestTrace = state.turnTraceHistory[state.turnTraceHistory.length - 1] ?? null;
  return {
    conversationId: state.sessionId,
    title: buildSessionTitleFromMessages(state.messages),
    summary: state.sessionSummary || DEFAULT_BROWSER_SESSION_SUMMARY,
    turnCount: state.messages.filter((message) => message.role === "user").length,
    lastReferencedFile: null,
    updatedAtMs: latestTrace?.updatedAt ?? Date.now(),
    workspaceId: null
  };
}

export function ensureUniqueSessionList(sessions: SessionOverview[]): SessionOverview[] {
  const seen = new Set<string>();
  const deduped: SessionOverview[] = [];
  for (const session of sessions) {
    if (seen.has(session.conversationId)) {
      continue;
    }
    seen.add(session.conversationId);
    deduped.push(session);
  }
  return deduped;
}

export function createNextSessionId(existingSessionIds: Iterable<string>): string {
  const existing = new Set(existingSessionIds);
  let candidate = `session-${Date.now()}`;
  let counter = 1;
  while (existing.has(candidate)) {
    candidate = `session-${Date.now()}-${counter}`;
    counter += 1;
  }
  return candidate;
}

export function isPersistedMetadataCompatible(
  snapshot: SessionSnapshot,
  persisted: PersistedRuntimeState | null
) {
  if (!persisted) {
    return false;
  }

  const persistedHistory = buildTurnHistory(persisted.messages);
  if (persistedHistory.length !== snapshot.history.length) {
    return false;
  }

  return persistedHistory.every((message, index) => {
    const snapshotMessage = snapshot.history[index];
    return (
      message.role === snapshotMessage?.role &&
      message.content.trim() === snapshotMessage?.content.trim()
    );
  });
}

export function isPersistedMessageShapeCompatible(
  snapshot: SessionSnapshot,
  persisted: PersistedRuntimeState | null
) {
  if (!persisted) {
    return false;
  }

  const persistedHistory = buildTurnHistory(persisted.messages);
  if (persistedHistory.length !== snapshot.history.length) {
    return false;
  }

  return persistedHistory.every((message, index) => message.role === snapshot.history[index]?.role);
}
