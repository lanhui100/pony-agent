// runtime store 拆解契约（Wave 1 冻结面）。
//
// 真源：`.dev-team/runtime-split/fact-map.md`（84 键 / 18 getters / 97 actions）。
// 本文件只声明类型，不含运行时代码；任何键、签名或归属变更都必须先改事实图并回写修订记录。
//
// 【state 形状单一真源】`RuntimeStoreState` 是 `RuntimeStateW59` 的等价迁出物，但它**不是**形状真源：
// state 的形状真源 = `src/stores/runtime/state.ts` 的 `createInitialState()` 返回类型（即本文件的
// `RuntimeStoreState`，由该函数显式标注）。两边若漂移，tsc 会在 `createInitialState` 的返回处报错，
// 因此「逐字迁出的 84 键字面量」永远压过本文件的类型声明；修类型前先看 state.ts。
//
// 归属速查：
//   D1 会话目录与生命周期 → session-catalog.ts
//   D2 工作区树           → workspace.ts
//   D3 轮次执行与流式传输 → turn-execution.ts
//   D4 消息与输入         → messages.ts
//   D5 遥测/指标/Provider → telemetry.ts
//   D6 历史/分支/检查点   → history.ts
//   D7 队列与授权策略     → queue.ts
//   D8 健康与能力目录     → catalog.ts
//   跨域事务入口           → orchestration.ts
//   turn 事件订阅聚合      → turn-events.ts

import type {
  AttachmentAsset,
  AttachmentAssetFilter,
  CapabilitySourceView,
  CapabilityView,
  ChatMessage,
  ConversationCheckpointEntry,
  ExecutionCheckpoint,
  GraphRun,
  HistoryBranch,
  HistoryBranchSwitchResult,
  HistoryCheckoutMode,
  HistoryCheckoutResult,
  HistoryCursorState,
  HistoryForkResult,
  HistoryNode,
  HistoryRestoreResult,
  HistoryStateAuditSummary,
  MessageStateDelta,
  MessageStateSnapshot,
  ProviderCallCacheRecord,
  QueueDeliveryMode,
  ToolAuthorizationMode,
  QueuedMessageItem,
  RetrievedContextState,
  RuntimePhase,
  SessionRuntimeView,
  SessionSnapshot,
  ToolActivity,
  TraceTimelineEntry,
  TurnInputImage,
  TurnMetricsSnapshot,
  TurnRetryEvent,
  TurnStreamEvent,
  TurnTraceRecord,
} from "@/types/runtime";
import type { RuntimeState } from "@/lib/runtime/types";
import type { AgentRuntimeStatus } from "@/lib/runtime/status";
import type { TurnRetryHint } from "@/lib/runtime/turn-retry";
import type { ImportAttachmentOptions, PendingAttachment } from "@/lib/runtime/file-attachments";
import type { WorkspaceRecord } from "@/lib/runtime/workspace-api";
// 第 4 节门面断言用：仅类型引用（编译期擦除），不构成运行时循环依赖。
import type { useRuntimeStore } from "@/stores/runtime";

// ---------------------------------------------------------------------------
// 1. 会话内瞬时态与 store 状态口径
// ---------------------------------------------------------------------------

/**
 * Wave-59 F4 / D-59-3：hop 级流式类型。
 * - `undefined`：未接线（既有夹具 / 历史状态）⇒ 回退旧判据；
 * - `null`：本 hop 尚无 reasoning/text 输出（如工具后的新 hop）⇒ 承载 hop 级等待；
 * - `"reasoning"`：本 hop 正在流出推理；`"content"`：本 hop 已开始流出正文（优先）。
 */
export type ActiveStreamKind = "reasoning" | "content" | null;

/** 上游重试进度（真值来自 Rust `agent::retry::RetryNotice`）：会话内瞬时态，不持久化。 */
export type TurnRetryProgress = { turnId: string; attempt: number; maxAttempts: number };

/** F5b：turn 事件监听注册表（按 store 实例隔离，不进入响应式 state）。 */
export interface TurnEventRegistry {
  initPromise: Promise<void> | null;
  unlisteners: Array<() => void>;
}

/**
 * runtime store 的完整状态口径 = 共享 `RuntimeState` + store 内局部扩展字段。
 * 局部扩展的原因：`src/lib/runtime/types.ts` 与本波并发写域重叠，故按原实现
 * （原 `runtime.ts` 的 `RuntimeStateW59`）在 store 侧声明，语义与位置等价。
 */
export type RuntimeStoreState = RuntimeState & {
  activeStreamKind?: ActiveStreamKind;
  /** wave-metrics（task-4）：最近一条 `turn` 事件携带的 `TurnMetricsSnapshot`（Rust 单一结算产物）。 */
  liveMetrics: TurnMetricsSnapshot | null;
  retryProgress: TurnRetryProgress | null;
};

// ---------------------------------------------------------------------------
// 2. 各域 State 切片（Pick 自 RuntimeStoreState，零漂移）
// ---------------------------------------------------------------------------

export type SessionCatalogState = Pick<
  RuntimeStoreState,
  "sessionId" | "sessionList" | "sessionOperation" | "sessionHydrating" |
  "deletingSessionSet" | "sessionSwitchToken" | "sessionError" | "sessionNotice" |
  "runningSessionMap" | "completedSessionSet" | "failedSessionSet"
>;

export type WorkspaceState = Pick<
  RuntimeStoreState,
  "workspaceList" | "workspaceListLoaded" | "sidebarOpInflightSet" | "activeWorkspaceId" |
  "sessionWorkspaceId"
>;

export type TurnExecutionState = Pick<
  RuntimeStoreState,
  "phase" | "isSubmitting" | "activeTurnId" | "activeTurnStartedAtMs" | "activeRunId" |
  "activeStreamKind" | "retryProgress" | "browserPreviewRunToken" | "streamBufferTurnId" |
  "streamBufferText" | "streamBufferReasoning" | "streamFlushFrameId" | "streamFlushTimerId" |
  "deferredPersistTimerId" | "submissionWatchdogTimerId" | "eventsReady" |
  "streamDebugDeltaCount" | "streamDebugFlushCount" | "streamDebugTextCharsReceived" |
  "streamDebugTextCharsFlushed"
>;

export type MessagesState = Pick<
  RuntimeStoreState,
  "messages" | "draftMessage" | "pendingAttachments" | "attachmentAssets" | "sessionSummary" |
  "error" | "toolActivities" | "traceSteps" | "retrievedContext" | "messageRevision"
>;

export type TelemetryState = Pick<
  RuntimeStoreState,
  "traceTimeline" | "turnTraceHistory" | "eventCursorByTurnId" | "liveMetrics" |
  "activeProviderCallRecords" | "providerRequestedName" | "providerName" | "providerProtocol" |
  "providerModel" | "providerSource" | "providerMode" | "fallbackReason" | "inputTokens" |
  "outputTokens" | "totalTokens" | "firstTokenLatencyMs" | "pendingThrottledTraceTimeline" |
  "traceTimelineThrottleTimerId"
>;

export type HistoryState = Pick<
  RuntimeStoreState,
  "visibleNodeId" | "branchHeadNodeId" | "activeBranchId" | "cursorVersion" |
  "historyCursorMode" | "historyNodes" | "historyBranches" | "initialRollbackActive" |
  "latestExecutionCheckpoint" | "latestGraphRunSubmissionPlan" |
  "latestGraphRunControlBoundaryEvidence" | "latestRunControlAuditSummary" |
  "latestHistoryStateAuditSummary"
>;

export type QueueState = Pick<
  RuntimeStoreState,
  "pendingQueuedMessages" | "queueDeliveryMode" | "toolAuthorizationMode"
>;

export type CatalogState = Pick<
  RuntimeStoreState,
  "health" | "availableTools" | "capabilitySources" | "capabilities"
>;

/** 84 键自检：各域 State 切片并集必须恰好等于 store 状态键集（编译期断言）。 */
export type RuntimeStoreStateKeys =
  keyof SessionCatalogState | keyof WorkspaceState | keyof TurnExecutionState | keyof MessagesState | keyof TelemetryState | keyof HistoryState | keyof QueueState | keyof CatalogState;

// 断言构造器：把「非法差值」直接写成约束，失配时错误信息**只含键名**
// （形如 `Type '"messages"' does not satisfy the constraint 'never'`），
// 绝不打印巨型交叉类型 —— 否则一次失配就能让 vue-tsc 跑满超时。
type _Nothing<T extends never> = T;
/** 事实图 §1：84 键冻结断言 —— 双向零差集（缺一多一都会以键名报错）。 */
export type StateKeysMissing = _Nothing<Exclude<FrozenStateKeys, RuntimeStoreStateKeys>>;
export type StateKeysUnexpected = _Nothing<Exclude<RuntimeStoreStateKeys, FrozenStateKeys>>;

/** 事实图 §1 的 84 键字面量清单。 */
export type FrozenStateKeys =
  "sessionId" | "sessionList" | "sessionOperation" | "sessionHydrating" |
  "deletingSessionSet" | "sessionSwitchToken" | "sessionError" | "sessionNotice" |
  "runningSessionMap" | "completedSessionSet" | "failedSessionSet" | "workspaceList" |
  "workspaceListLoaded" | "sidebarOpInflightSet" | "activeWorkspaceId" | "sessionWorkspaceId" |
  "phase" | "isSubmitting" | "activeTurnId" | "activeTurnStartedAtMs" | "activeRunId" |
  "activeStreamKind" | "retryProgress" | "browserPreviewRunToken" | "streamBufferTurnId" |
  "streamBufferText" | "streamBufferReasoning" | "streamFlushFrameId" | "streamFlushTimerId" |
  "deferredPersistTimerId" | "submissionWatchdogTimerId" | "eventsReady" |
  "streamDebugDeltaCount" | "streamDebugFlushCount" | "streamDebugTextCharsReceived" |
  "streamDebugTextCharsFlushed" | "messages" | "draftMessage" | "pendingAttachments" |
  "attachmentAssets" | "sessionSummary" | "error" | "toolActivities" | "traceSteps" |
  "retrievedContext" | "messageRevision" | "traceTimeline" | "turnTraceHistory" |
  "eventCursorByTurnId" | "liveMetrics" | "activeProviderCallRecords" |
  "providerRequestedName" | "providerName" | "providerProtocol" | "providerModel" |
  "providerSource" | "providerMode" | "fallbackReason" | "inputTokens" | "outputTokens" |
  "totalTokens" | "firstTokenLatencyMs" | "pendingThrottledTraceTimeline" |
  "traceTimelineThrottleTimerId" | "visibleNodeId" | "branchHeadNodeId" | "activeBranchId" |
  "cursorVersion" | "historyCursorMode" | "historyNodes" | "historyBranches" |
  "initialRollbackActive" | "latestExecutionCheckpoint" | "latestGraphRunSubmissionPlan" |
  "latestGraphRunControlBoundaryEvidence" | "latestRunControlAuditSummary" |
  "latestHistoryStateAuditSummary" | "pendingQueuedMessages" | "queueDeliveryMode" |
  "toolAuthorizationMode" | "health" | "availableTools" | "capabilitySources" |
  "capabilities"
;

export interface SessionCatalogGetters {
  /** :600 */
  isSessionDeleting(this: RuntimeStoreThis, state: RuntimeStoreState): (sessionId: string) => boolean;
  /** :655 */
  isSessionRunning(this: RuntimeStoreThis, state: RuntimeStoreState): (sessionId: string) => boolean;
}

export interface WorkspaceGetters {
}

export interface TurnExecutionGetters {
  /** :581 */
  phaseLabel(this: RuntimeStoreThis, state: RuntimeStoreState): string;
  /** :664 */
  failedTurnRetry(this: RuntimeStoreThis, state: RuntimeStoreState): TurnRetryHint | null;
  /** :670 */
  agentRuntimeStatus(this: RuntimeStoreThis, state: RuntimeStoreState): AgentRuntimeStatus | null;
  /** :906 */
  canSubmitPrompt: (state: RuntimeStoreState) => boolean;
  /** :909 */
  isInputDisabled: (state: RuntimeStoreState) => boolean;
}

export interface MessagesGetters {
}

export interface TelemetryGetters {
  /** :790 */
  activeTraceId: (state: RuntimeStoreState) => string | undefined;
  /** :805 */
  sessionTotalCostUsd: (state: RuntimeStoreState) => number;
  /** :815 */
  hasUnpricedTurns: (state: RuntimeStoreState) => boolean;
  /** :821 */
  formattedSessionCost(this: RuntimeStoreThis): string;
  /** :836 */
  isCostBudgetExceeded(this: RuntimeStoreThis): boolean;
  /** :845 */
  costBudgetWarningMessage(this: RuntimeStoreThis): string | null;
  /** :852 */
  usageGovernanceSummary: (state: RuntimeStoreState) => any;
  /** :877 */
  formattedGovernanceCost(this: RuntimeStoreThis): string;
  /** :893 */
  governanceBlockDetail(this: RuntimeStoreThis, state: RuntimeStoreState): any;
}

export interface HistoryGetters {
  /** :597 */
  isHistoricalMode(this: RuntimeStoreThis, state: RuntimeStoreState): boolean;
  /** :603 */
  conversationCheckpointEntries(this: RuntimeStoreThis, state: RuntimeStoreState): ConversationCheckpointEntry[];
}

export interface QueueGetters {
}

export interface CatalogGetters {
}

export interface OrchestrationGetters {
}

export interface TurnEventsGetters {
}



export interface SessionCatalogActions {
  /** :1545 */
  loadSessionCatalog(): Promise<void>;
  /** :2851 */
  deleteSession(targetSessionId: string): Promise<{ ok: boolean; error?: string | undefined; }>;
  /** :2293 */
  forkHistoryNode(nodeId?: string | null): Promise<HistoryForkResult | null>;
  /** :2312 */
  switchHistoryBranch(branchId: string): Promise<HistoryBranchSwitchResult | null>;
  /** :5722 */
  setSessionNotice(text: string | null): void;
  /** :2637 */
  renameSession(sessionId: string, title: string): Promise<{ ok: boolean; error?: string | undefined; }>;
  /** :2709 */
  archiveSession(sessionId: string): Promise<{ ok: boolean; error?: string | undefined; }>;
  /** :2749 */
  unarchiveSession(sessionId: string): Promise<{ ok: boolean; error?: string | undefined; }>;
  /** :1576 */
  loadSessionRuntimeViewState(sessionId: string, nodeId?: string | null): Promise<SessionRuntimeView>;
}

export interface WorkspaceActions {
  /** :2504 */
  loadWorkspaces(force?: boolean): Promise<void>;
  /** :2544 */
  createNewWorkspace(name: string, rootPath: string): Promise<{ ok: boolean; record?: WorkspaceRecord | undefined; error?: string | undefined; }>;
  /** :2580 */
  renameWorkspace(workspaceId: string, name: string): Promise<{ ok: boolean; error?: string | undefined; }>;
  /** :2608 */
  deleteWorkspace(workspaceId: string): Promise<{ ok: boolean; error?: string | undefined; }>;
  /** :2771 */
  activateWorkspace(workspaceId: string): void;
  /** :2530 */
  normalizeActiveWorkspace(): void;
  /** :2576 */
  isSidebarOpInflight(key: string): boolean;
}

export interface TurnExecutionActions {
  /** :1071 */
  scheduleStreamFlush(turnId: string): void;
  /** :1029 */
  flushBufferedStreamText(turnId?: string | null): void;
  /** :1006 */
  cancelStreamFlush(): void;
  /** :1179 */
  startSubmissionWatchdog(turnId: string): void;
  /** :1189 */
  rearmSubmissionWatchdog(turnId: string): void;
  /** :1311 */
  clearSubmissionWatchdog(): void;
  /** :1165 */
  scheduleDeferredPersist(delay?: number): void;
  /** :995 */
  cancelDeferredPersist(): void;
  /** :1023 */
  resetStreamDebugMetrics(): void;
  /** :3490 */
  clearTimeoutRetryPlaceholder(turnId?: string | null): void;
  /** :3518 */
  applyTurnRetryProgress(payload: TurnRetryEvent): void;
  /** :3981 */
  initializeTurnEvents(): Promise<void>;
  /** :4020 */
  disposeTurnEvents(): Promise<void>;
  /** :5712 */
  enterStoppingPhase(): void;
}

export interface MessagesActions {
  /** :1501 */
  applyMessageStateSnapshot(snapshot?: MessageStateSnapshot | null): boolean;
  /** :1771 */
  resolveDerivedSessionRun(options?: {
      sessionId?: string | null;
      runId?: string | null;
      preferRefresh?: boolean;
      snapshot?: SessionSnapshot | null;
      nodeId?: string | null;
    }): Promise<GraphRun | null>;
  /** :3381 */
  applyTurnTokenStats(turnId: string, inputTokens?: number | null, outputTokens?: number | null, persist?: boolean): void;
  /** :3532 */
  ensureAssistantMessage(turnId: string, modelName?: string | null): ChatMessage;
  /** :3561 */
  syncToolMessages(turnId: string, toolActivities?: ToolActivity[] | null, persist?: boolean): void;
  /** :3657 */
  setDraftMessage(message: string): void;
  /** :3678 */
  setPendingAttachments(attachments: PendingAttachment[]): void;
  /** :3681 */
  clearPendingAttachments(): void;
  /** :3684 */
  removePendingAttachment(id: string): void;
  /** :3692 */
  addPendingAttachments(files: File[], options?: ImportAttachmentOptions): Promise<{ added: number; errors: string[]; }>;
  /** :5733 */
  retryLastTurn(): Promise<boolean>;
  /** :3666 */
  recallLastUserMessage(): string | null;
  /** :1471 */
  getAttachmentAssets(filter?: AttachmentAssetFilter | null): AttachmentAsset[];
  /** :1738 */
  loadRetrievedContextState(sessionId: string, options?: { runId?: string | null; snapshot?: SessionSnapshot | null; nodeId?: string | null }): Promise<RetrievedContextState | null>;
}

export interface TelemetryActions {
  /** :3044 */
  upsertTurnTrace(turnId: string, patch: Partial<Omit<TurnTraceRecord, "turnId" | "updatedAt">> & { updatedAt?: number }, persist?: boolean): void;
  /** :3242 */
  publishTraceTimeline(traceTimeline: TraceTimelineEntry[]): void;
  /** :3294 */
  scheduleThrottledTraceTimeline(traceTimeline: TraceTimelineEntry[]): void;
  /** :3310 */
  flushPendingTraceTimeline(): void;
  /** :3321 */
  clearPendingTraceTimeline(): void;
  /** :3407 */
  applyActiveProviderCallRecords(records: ProviderCallCacheRecord[] | null | undefined, reset?: boolean): void;
  /** :3435 */
  applyMetricsSnapshot(metrics?: TurnMetricsSnapshot | null): void;
  /** :3203 */
  resolveTurnTraceTimeline(turnId: string): TraceTimelineEntry[];
  /** :3215 */
  commitTurnTraceTimeline(turnId: string, traceTimeline: TraceTimelineEntry[], patch?: Partial<Omit<TurnTraceRecord, "turnId" | "updatedAt">> & { updatedAt?: number }, persist?: boolean): void;
  /** :3235 */
  updateActiveTraceTimeline(traceTimeline: TraceTimelineEntry[], skipClone?: boolean): void;
  /** :3328 */
  updateActiveModelTraceFromAssistant(turnId: string): void;
  /** :914 */
  overrideGovernanceBlock(): Promise<void>;
  /** :926 */
  fetchUsageGovernanceSummary(): Promise<void>;
}

export interface HistoryActions {
  /** :2109 区域 / 泛型辅助：事实图 §3 未单列，但存在于 actions 对象 */
  performHistoryOperation<
    TInvokeArgs extends Record<string, unknown>,
    TResult extends HistoryCursorState & { historyStateAuditSummary?: HistoryStateAuditSummary | null }
  >(options: {
    cmd: string;
    invokeArgs: TInvokeArgs;
    normalize: (payload: any, nodes: HistoryNode[], branches: HistoryBranch[]) => TResult;
    selectNodeId?: (result: any) => string | null;
    errorMessagePrefix?: string;
  }): Promise<TResult | null>;
  /** :1474 */
  applyHistoryState(_sessionId: string, payload?:
        | (Partial<HistoryCursorState> & {
            historyNodes?: HistoryNode[] | null;
            historyBranches?: HistoryBranch[] | null;
          })
        | null): void;
  /** :1679 */
  loadSessionState(nextSessionId: string, options?: {
        refreshCatalog?: boolean;
        executionCheckpoint?: ExecutionCheckpoint | null;
        runtimeView?: SessionRuntimeView | null;
        nodeId?: string | null;
        preserveMessages?: boolean;
      }): Promise<void>;
  /** :2261 */
  findCheckpointTurnIdByNodeId(nodeId: string): string | null;
}

export interface QueueActions {
  /** :3246 */
  enqueueMessage(content: string, mode?: QueueDeliveryMode): void;
  /** :3264 */
  promoteToSteer(id: string): void;
  /** :3275 */
  removeQueuedMessage(id: string): void;
  /** :3281 */
  toggleQueueDeliveryMode(): void;
  /** :3284 */
  setQueueDeliveryMode(mode: QueueDeliveryMode): void;
  /** :3287 */
  setToolAuthorizationMode(mode: ToolAuthorizationMode): void;
  /** :3290 */
  dequeueNextMessage(): QueuedMessageItem | null;
}

export interface CatalogActions {
  /** :3858 */
  fetchAvailableTools(): Promise<void>;
  /** :3883 */
  fetchCapabilitySources(): Promise<void>;
  /** :3898 */
  fetchCapabilities(filter?: { sourceId?: string | null; kind?: string | null }): Promise<void>;
  /** :3939 */
  inspectCapability(capabilityId: string): Promise<CapabilityView | null>;
  /** :3960 */
  inspectCapabilitySource(sourceId: string): Promise<CapabilitySourceView | null>;
}

export interface OrchestrationActions {
  /** :937 */
  resetSessionRuntimeState(): void;
  /** :1812 */
  applySessionSnapshot(sessionId: string, snapshot: SessionSnapshot, retrieved?: RetrievedContextState | null, runtimeView?:
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
        | null, options?: { preserveMessages?: boolean }): void;
  /** :5745 */
  submitTurn(options?: { images?: TurnInputImage[] }): Promise<boolean>;
  /** :5668 */
  stopTurn(): Promise<boolean>;
  /** :2330 */
  switchSession(nextSessionId: string): Promise<void>;
  /** :2673 */
  enterFreshSession(targetWorkspaceId?: string): void;
  /** :2782 */
  createSession(targetWorkspaceId?: string): Promise<void>;
  /** :2929 */
  initializeSessions(): Promise<void>;
  /** :1959 */
  rollbackToInitialState(): void;
  /** :2026 */
  rollbackToTurnBoundary(turnId: string): boolean;
  /** :2109 */
  checkoutHistoryNode(nodeId: string, mode?: HistoryCheckoutMode, turnId?: string | null): Promise<HistoryCheckoutResult | null>;
  /** :2265 */
  restoreBranchHead(branchId?: string | null): Promise<HistoryRestoreResult | null>;
  /** :1596 */
  applyExecutionCheckpoint(checkpoint: ExecutionCheckpoint | null, persistedMessages?: ChatMessage[] | null): void;
  /** :1510 */
  applyMessageStateDelta(delta?: MessageStateDelta | null): boolean;
  /** :3441 */
  applyOutputEnd(payload: TurnStreamEvent): void;
  /** :3820 */
  fetchHealth(): Promise<void>;
  /** :5574 */
  runBrowserPreviewTurn(requestId: string): Promise<void>;
  /** :1092 */
  cancelBrowserPreviewTurn(turnId: string): boolean;
  /** :1198 */
  handleSubmissionWatchdogTimeout(turnId: string): void;
  /** :1317 */
  persistHistory(): void;
  /** :1384 */
  updatePersistedBackgroundSession(sessionId: string, turnId: string, patch: {
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
      }): void;
}

export interface TurnEventsActions {
  /** :3123 */
  shouldProcessTurnEvent(payload: Pick<TurnStreamEvent, "turnId" | "eventId" | "sequence" | "emittedAtMs" | "kind">): boolean;
  /** :3193 */
  commitTurnEventCursor(payload: Pick<TurnStreamEvent, "turnId" | "eventId" | "sequence" | "emittedAtMs">): void;
  /** :4042 */
  registerTurnEventListeners(registry: TurnEventRegistry): Promise<void>;
}

// ---------------------------------------------------------------------------
// 3. 契约冻结断言（键名集合一经冻结，增删必须走事实图修订）
// ---------------------------------------------------------------------------

/** 事实图 §2 的 18 个 getter 名。 */
export type FrozenGetterKeys =
  "phaseLabel" | "isHistoricalMode" | "isSessionDeleting" | "conversationCheckpointEntries" |
  "isSessionRunning" | "failedTurnRetry" | "agentRuntimeStatus" | "activeTraceId" |
  "sessionTotalCostUsd" | "hasUnpricedTurns" | "formattedSessionCost" |
  "isCostBudgetExceeded" | "costBudgetWarningMessage" | "usageGovernanceSummary" |
  "formattedGovernanceCost" | "governanceBlockDetail" | "canSubmitPrompt" | "isInputDisabled";

/** 事实图 §3 的 97 个 action 名（96 + performHistoryOperation 泛型辅助）。 */
export type FrozenActionKeys =
  "loadSessionCatalog" | "deleteSession" | "forkHistoryNode" | "switchHistoryBranch" |
  "setSessionNotice" | "renameSession" | "archiveSession" | "unarchiveSession" |
  "loadSessionRuntimeViewState" | "loadWorkspaces" | "createNewWorkspace" | "renameWorkspace" |
  "deleteWorkspace" | "activateWorkspace" | "normalizeActiveWorkspace" |
  "isSidebarOpInflight" | "scheduleStreamFlush" | "flushBufferedStreamText" |
  "cancelStreamFlush" | "startSubmissionWatchdog" | "rearmSubmissionWatchdog" |
  "clearSubmissionWatchdog" | "scheduleDeferredPersist" | "cancelDeferredPersist" |
  "resetStreamDebugMetrics" | "clearTimeoutRetryPlaceholder" | "applyTurnRetryProgress" |
  "initializeTurnEvents" | "disposeTurnEvents" | "enterStoppingPhase" |
  "applyMessageStateSnapshot" | "resolveDerivedSessionRun" | "applyTurnTokenStats" |
  "ensureAssistantMessage" | "syncToolMessages" | "setDraftMessage" | "setPendingAttachments" |
  "clearPendingAttachments" | "removePendingAttachment" | "addPendingAttachments" |
  "retryLastTurn" | "recallLastUserMessage" | "getAttachmentAssets" |
  "loadRetrievedContextState" | "upsertTurnTrace" | "publishTraceTimeline" |
  "scheduleThrottledTraceTimeline" | "flushPendingTraceTimeline" |
  "clearPendingTraceTimeline" | "applyActiveProviderCallRecords" | "applyMetricsSnapshot" |
  "resolveTurnTraceTimeline" | "commitTurnTraceTimeline" | "updateActiveTraceTimeline" |
  "updateActiveModelTraceFromAssistant" | "overrideGovernanceBlock" |
  "fetchUsageGovernanceSummary" | "applyHistoryState" | "loadSessionState" |
  "findCheckpointTurnIdByNodeId" | "performHistoryOperation" | "enqueueMessage" |
  "promoteToSteer" | "removeQueuedMessage" | "toggleQueueDeliveryMode" |
  "setQueueDeliveryMode" | "setToolAuthorizationMode" | "dequeueNextMessage" |
  "fetchAvailableTools" | "fetchCapabilitySources" | "fetchCapabilities" |
  "inspectCapability" | "inspectCapabilitySource" | "resetSessionRuntimeState" |
  "applySessionSnapshot" | "submitTurn" | "stopTurn" | "switchSession" | "enterFreshSession" |
  "createSession" | "initializeSessions" | "rollbackToInitialState" |
  "rollbackToTurnBoundary" | "checkoutHistoryNode" | "restoreBranchHead" |
  "applyExecutionCheckpoint" | "applyMessageStateDelta" | "applyOutputEnd" | "fetchHealth" |
  "runBrowserPreviewTurn" | "cancelBrowserPreviewTurn" | "handleSubmissionWatchdogTimeout" |
  "persistHistory" | "updatePersistedBackgroundSession" | "shouldProcessTurnEvent" |
  "commitTurnEventCursor" | "registerTurnEventListeners";

export interface RuntimeStoreGetters
  extends SessionCatalogGetters,
    WorkspaceGetters,
    TurnExecutionGetters,
    MessagesGetters,
    TelemetryGetters,
    HistoryGetters,
    QueueGetters,
    CatalogGetters,
    OrchestrationGetters,
    TurnEventsGetters {}

export type RuntimeStoreActions = SessionCatalogActions &
  WorkspaceActions &
  TurnExecutionActions &
  MessagesActions &
  TelemetryActions &
  HistoryActions &
  QueueActions &
  CatalogActions &
  OrchestrationActions &
  TurnEventsActions;

export type GetterValues<G> = {
  [K in keyof G]: G[K] extends (...args: any[]) => infer R ? R : never;
};

/** store 门面类型：状态 + getter 求值后的只读值 + action 方法（消费方视角）。 */
export type RuntimeStore = RuntimeStoreState &
  GetterValues<RuntimeStoreGetters> &
  RuntimeStoreActions;

/** getter / action 体内 `this` 的口径（Pinia options 式 `this` 的等价交集类型）。 */
export type RuntimeStoreThis = RuntimeStore;

/** getter 名冻结断言（双向零差集，失配只报键名）。 */
export type GetterKeysMissing = _Nothing<Exclude<FrozenGetterKeys, keyof RuntimeStoreGetters>>;
export type GetterKeysUnexpected = _Nothing<Exclude<keyof RuntimeStoreGetters, FrozenGetterKeys>>;
/** action 名冻结断言（双向零差集，失配只报键名）。 */
export type ActionKeysMissing = _Nothing<Exclude<FrozenActionKeys, keyof RuntimeStoreActions>>;
export type ActionKeysUnexpected = _Nothing<Exclude<keyof RuntimeStoreActions, FrozenActionKeys>>;

// ---------------------------------------------------------------------------
// 4. 装配壳门面断言（防 spread 丢成员 / Pinia 推导退化）
// ---------------------------------------------------------------------------
// 只做键集差，不比较结构：即使 Pinia 推导退化，也只报「少了哪些键名」。
type RuntimeStoreFacade = ReturnType<typeof useRuntimeStore>;
type FacadeKeys = keyof RuntimeStoreFacade;

/** 84 键在装配壳上齐全（spread 未丢 state 成员）。 */
export type FacadeStateKeysMissing = _Nothing<Exclude<FrozenStateKeys, FacadeKeys>>;
/** 18 个 getter 在装配壳上齐全。 */
export type FacadeGetterKeysMissing = _Nothing<Exclude<FrozenGetterKeys, FacadeKeys>>;
/** 97 个 action 在装配壳上齐全。 */
export type FacadeActionKeysMissing = _Nothing<Exclude<FrozenActionKeys, FacadeKeys>>;
