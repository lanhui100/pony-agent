// runtime store 初始状态工厂。
//
// 逐字迁出自原 `src/stores/runtime.ts:465-580` 的 state 字面量（Wave 1 语义零改动）。
// 迁移边界：
//   - 保留 `loadPersistedRuntimeState` 恢复、localStorage activeWorkspace 读取、
//     `canonicalTerminalPhase` 归一（经 `resolveRestoredPersistedPhase`）；
//   - 保留 :248-260 的「激活 Workspace 初值」私有函数与原注释；
//   - **不得**在此文件写业务逻辑（实现归各域模块的 Executor）。
import type {
  ProviderCallCacheRecord,
  QueueDeliveryMode,
  QueuedMessageItem,
  ToolAuthorizationMode,
  TurnMetricsSnapshot
} from "@/types/runtime";
import type { PendingAttachment } from "@/lib/runtime/file-attachments";
import type { WorkspaceRecord } from "@/lib/runtime/workspace-api";
import {
  ACTIVE_WORKSPACE_STORAGE_KEY,
  DEFAULT_WORKSPACE_ID
} from "@/lib/runtime/workspace-constants";
import { DEFAULT_SESSION_ID } from "@/lib/runtime/constants";
import { loadPersistedRuntimeState, buildEventCursorByTurnTraceHistory } from "@/lib/runtime/sessions";
import { resolveRestoredPersistedPhase } from "@/lib/runtime/phases";
import {
  createDefaultTraceSteps,
  createDefaultTraceTimeline,
  normalizeTurnTraceRecord
} from "@/lib/runtime/trace";
import {
  createAvailableTools,
  createCapabilities,
  createCapabilitySources
} from "@/lib/runtime/browser-preview";
import type { ActiveStreamKind, RuntimeStoreState } from "@/stores/runtime/types";

// PA-081：激活 Workspace 初值——localStorage 单一真相源，缺失/异常回退 default
// （注册表加载后若不在列表内会再次归一并清理残留 key）。
function resolveInitialActiveWorkspaceId(): string {
  if (typeof window === "undefined") {
    return DEFAULT_WORKSPACE_ID;
  }
  try {
    const stored = window.localStorage.getItem(ACTIVE_WORKSPACE_STORAGE_KEY);
    return stored && stored.trim() !== "" ? stored : DEFAULT_WORKSPACE_ID;
  } catch {
    return DEFAULT_WORKSPACE_ID;
  }
}

/** Pinia `state` 选项：返回 store 初始状态（含持久化恢复与终态归一）。 */
export function createInitialState(): RuntimeStoreState {
  const persisted = loadPersistedRuntimeState(DEFAULT_SESSION_ID);

  return {
    sessionId: DEFAULT_SESSION_ID,
    sessionList: [],
    sessionOperation: null,
    sessionHydrating: false,
    deletingSessionSet: {},
    sessionSwitchToken: 0,
    sessionError: null,
    // C-9：成功/中性反馈通道（不得复用 sessionError 承载成功文案）。
    sessionNotice: null,
    // PA-081：Workspace 树导航状态（注册表 + 激活项 localStorage 单一真相源）。
    workspaceList: [] as WorkspaceRecord[],
    workspaceListLoaded: false,
    /** 三级树：工作区/会话操作 inflight 守卫（key=workspaceId|sessionId:op）。 */
    sidebarOpInflightSet: {} as Record<string, true>,
    activeWorkspaceId: resolveInitialActiveWorkspaceId(),
    sessionWorkspaceId: persisted?.sessionWorkspaceId?.trim() || DEFAULT_WORKSPACE_ID,
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
    /**
     * wave-metrics（task-4）：不持久化的会话内瞬时态 —— 启动即无快照，
     * 由 `turn` 事件（hop_complete / output_end / completed / phase_changed /
     * checkpoint_persisted）经 `applyMetricsSnapshot` 落库。
     */
    liveMetrics: null as TurnMetricsSnapshot | null,
    /** R4/Wave-15：活跃轮 per-call 记录（最近一次模型调用的物理指标来源，非累加）。 */
    activeProviderCallRecords: [] as ProviderCallCacheRecord[],
    isSubmitting: false,
    activeTurnId: null,
    activeTurnStartedAtMs: null,
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
    pendingQueuedMessages: [] as QueuedMessageItem[],
    queueDeliveryMode: "queue" as QueueDeliveryMode,
    toolAuthorizationMode: "workspace_write" as ToolAuthorizationMode,
    eventsReady: false,
    deferredPersistTimerId: null,
    streamFlushTimerId: null,
    streamBufferTurnId: null,
    streamBufferText: "",
    streamBufferReasoning: "",
    /** Wave-59 F4 / D-59-3：hop 级流式类型（null=本 hop 无输出；undefined=未接线⇒旧判据）。 */
    activeStreamKind: undefined as ActiveStreamKind | undefined,
    /**
     * 桌面端上游重试进度（`turn:retry` 的唯一落点）。
     *
     * 真值源是 Rust `agent::retry::RetryNotice`（与 TUI 状态行**同一个**出口）：
     * `attempt` = 已失败尝试数（1 起），`maxAttempts` = 本跳允许的尝试总数（含首次）。
     * 只由 `applyTurnRetryProgress` 写入、由 `clearTimeoutRetryPlaceholder` 在回合/跳转边界清空。
     */
    retryProgress: null as { turnId: string; attempt: number; maxAttempts: number } | null,
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
}
