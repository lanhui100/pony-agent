// runtime store 共享状态类型：store 与 sessions 模块共同依赖。
// 只定义类型，无运行时代码。
import type {
  AttachmentAsset,
  AvailableTool,
  CapabilitySourceView,
  CapabilityView,
  ChatMessage,
  ExecutionCheckpoint,
  GraphRunControlBoundaryEvidence,
  GraphRunSubmissionPlan,
  HealthPayload,
  HistoryBranch,
  HistoryCursorMode,
  HistoryNode,
  HistoryStateAuditSummary,
  RetrievedContextState,
  RunControlAuditSummary,
  RuntimePhase,
  SessionOverview,
  ToolActivity,
  TraceStep,
  TraceTimelineEntry,
  TurnTraceRecord
} from "../../types/runtime";

export type RunningTurn = {
  turnId: string;
  phase: RuntimePhase;
  textBuffer: string;
  reasoningBuffer: string;
};

export type RuntimeState = {
  sessionId: string;
  sessionList: SessionOverview[];
  // PA-081：当前会话归属的 Workspace（创建时冻结；submitTurn 以会话归属优先，
  // 防止"会话在 B 组、工具在 A root 执行"的跨项目错位）。
  sessionWorkspaceId: string;
  sessionOperation: "initializing" | "switching" | "deleting" | null;
  sessionHydrating: boolean;
  deletingSessionSet: Record<string, boolean>;
  sessionSwitchToken: number;
  sessionError: string | null;
  // PA-081：Workspace 树导航状态。
  workspaceList: import("./workspace-api").WorkspaceRecord[];
  workspaceListLoaded: boolean;
  /** 三级树：侧边栏操作 inflight 守卫（key=op:id）。 */
  sidebarOpInflightSet: Record<string, true>;
  activeWorkspaceId: string;
  phase: RuntimePhase;
  health: HealthPayload | null;
  error: string | null;
  draftMessage: string;
  pendingAttachments: import("./file-attachments").PendingAttachment[];
  sessionSummary: string;
  retrievedContext: RetrievedContextState | null;
  providerRequestedName: string;
  providerName: string;
  providerProtocol: string;
  providerModel: string;
  providerSource: string;
  providerMode: string;
  fallbackReason: string | null;
  inputTokens: number | null;
  outputTokens: number | null;
  totalTokens: number | null;
  firstTokenLatencyMs: number | null;
  isSubmitting: boolean;
  messages: ChatMessage[];
  attachmentAssets: AttachmentAsset[];
  availableTools: AvailableTool[];
  capabilitySources: CapabilitySourceView[];
  capabilities: CapabilityView[];
  toolActivities: ToolActivity[];
  traceSteps: TraceStep[];
  traceTimeline: TraceTimelineEntry[];
  turnTraceHistory: TurnTraceRecord[];
  eventCursorByTurnId: Record<string, { eventId: string | null; sequence: number | null; emittedAtMs: number | null }>;
  activeTurnId: string | null;
  activeRunId: string | null;
  latestExecutionCheckpoint: ExecutionCheckpoint | null;
  latestGraphRunSubmissionPlan: GraphRunSubmissionPlan | null;
  latestGraphRunControlBoundaryEvidence: GraphRunControlBoundaryEvidence[];
  latestRunControlAuditSummary: RunControlAuditSummary | null;
  latestHistoryStateAuditSummary: HistoryStateAuditSummary | null;
  visibleNodeId: string | null;
  branchHeadNodeId: string | null;
  activeBranchId: string | null;
  cursorVersion: number | null;
  messageRevision: string | null;
  historyCursorMode: HistoryCursorMode;
  historyNodes: HistoryNode[];
  historyBranches: HistoryBranch[];
  eventsReady: boolean;
  deferredPersistTimerId: number | null;
  streamFlushFrameId: number | null;
  streamFlushTimerId: number | null;
  streamBufferTurnId: string | null;
  streamBufferText: string;
  streamBufferReasoning: string;
  browserPreviewRunToken: number;
  initialRollbackActive: boolean;
  runningSessionMap: Record<string, RunningTurn>;
  completedSessionSet: Record<string, boolean>;
  failedSessionSet: Record<string, boolean>;
  streamDebugDeltaCount: number;
  streamDebugFlushCount: number;
  streamDebugTextCharsReceived: number;
  streamDebugTextCharsFlushed: number;
  // traceTimeline 节流状态：delta 高频事件中的 timeline 更新先挂起，
  // 由 throttle timer 合并应用；低频语义事件与 terminal 事件会强制冲刷。
  pendingThrottledTraceTimeline: TraceTimelineEntry[] | null;
  traceTimelineThrottleTimerId: number | null;
  // 运行态看门狗：isSubmitting 置位后若超时未收到终态事件，强制解锁。
  submissionWatchdogTimerId: number | null;
};

export type PersistedRuntimeState = {
  cachedStateVersion: number;
  phase: RuntimePhase;
  // PA-081：会话归属 Workspace（浏览器持久化往返保持分组稳定）。
  sessionWorkspaceId?: string;
  canonicalTerminalPhase?: "completed" | "failed" | "cancelled";
  messages: ChatMessage[];
  attachmentAssets: AttachmentAsset[];
  // turnTraceHistory is no longer persisted to localStorage — trace data is
  // already stored on the backend (sessions.json) and loaded via Tauri commands.
  // The field remains optional for backward-compatible reads of old cache data.
  turnTraceHistory?: TurnTraceRecord[];
  sessionSummary: string;
  providerRequestedName: string;
  providerName: string;
  providerProtocol: string;
  providerModel: string;
  providerSource: string;
  providerMode: string;
  fallbackReason: string | null;
  inputTokens: number | null;
  outputTokens: number | null;
  totalTokens: number | null;
  firstTokenLatencyMs: number | null;
  branchHeadNodeId?: string | null;
  activeBranchId?: string | null;
  historyCursorMode?: HistoryCursorMode;
  historyNodes?: HistoryNode[];
  historyBranches?: HistoryBranch[];
  visibleNodeId?: string | null;
  cursorVersion?: number | null;
  initialRollbackActive?: boolean;
  checkpoint?: ExecutionCheckpoint | null;
  runningTurnId?: string | null;
};

export type SessionRuntimeSnapshot = {
  sessionId: string;
  sessionList: SessionOverview[];
  deletingSessionSet: Record<string, boolean>;
  phase: RuntimePhase;
  error: string | null;
  draftMessage: string;
  sessionSummary: string;
  retrievedContext: RetrievedContextState | null;
  providerRequestedName: string;
  providerName: string;
  providerProtocol: string;
  providerModel: string;
  providerSource: string;
  providerMode: string;
  fallbackReason: string | null;
  inputTokens: number | null;
  outputTokens: number | null;
  totalTokens: number | null;
  firstTokenLatencyMs: number | null;
  isSubmitting: boolean;
  activeTurnId: string | null;
  activeRunId: string | null;
  latestExecutionCheckpoint: ExecutionCheckpoint | null;
  latestGraphRunSubmissionPlan: GraphRunSubmissionPlan | null;
  latestGraphRunControlBoundaryEvidence: GraphRunControlBoundaryEvidence[];
  latestRunControlAuditSummary: RunControlAuditSummary | null;
  latestHistoryStateAuditSummary: HistoryStateAuditSummary | null;
  visibleNodeId: string | null;
  branchHeadNodeId: string | null;
  activeBranchId: string | null;
  cursorVersion: number | null;
  messageRevision: string | null;
  historyCursorMode: HistoryCursorMode;
  historyNodes: HistoryNode[];
  historyBranches: HistoryBranch[];
  initialRollbackActive: boolean;
  messages: ChatMessage[];
  attachmentAssets: AttachmentAsset[];
  toolActivities: ToolActivity[];
  traceSteps: TraceStep[];
  traceTimeline: TraceTimelineEntry[];
  turnTraceHistory: TurnTraceRecord[];
  eventCursorByTurnId: Record<string, { eventId: string | null; sequence: number | null; emittedAtMs: number | null }>;
};

export type PersistedRuntimeCache = {
  sessions: Record<string, PersistedRuntimeState>;
  runningSessionMap?: Record<string, { turnId: string; phase: RuntimePhase; textBuffer?: string; reasoningBuffer?: string }>;
  completedSet?: Record<string, boolean>;
  failedSet?: Record<string, boolean>;
};

export type SessionInitializationStrategy =
  | { kind: "local-cache"; persistedState: PersistedRuntimeState }
  | { kind: "host-read"; sessionId: string; reason: "no-cache" | "insufficient-checkpoint" }
  | { kind: "empty-fallback"; sessionId: string };
