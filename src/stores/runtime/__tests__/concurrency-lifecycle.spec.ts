/**
 * 场景 6（task-1）· 并发与生命周期 + store 表面契约（84 state / 18 getters / 97 actions）
 *
 * 覆盖：
 * - 并发 `initializeTurnEvents()` 幂等（单例 initPromise，14 个监听器只注册一次）；
 * - `disposeTurnEvents()` 在初始化中途调用：先等在飞 init 落定再清理，全部句柄 unlisten，可重新注册；
 * - delta 风暴 5000 条（固定次数循环 + 固定毫秒推进，确定性，无 sleep）；
 * - store 表面契约：状态键/getter/action 名字集合与 fact-map 逐项对账，
 *   并机械校验「已覆盖 ∪ 未覆盖 == 全部 action」，保证覆盖自评不隐瞒。
 */
import { describe, expect, it, vi } from "vitest";

const tauriMocks = vi.hoisted(() => ({
  mockSafeInvoke: vi.fn(),
  mockSafeListen: vi.fn(),
  mockIsTauriAvailable: vi.fn()
}));

vi.mock("@/lib/tauri", () => ({
  safeInvoke: tauriMocks.mockSafeInvoke,
  safeListen: tauriMocks.mockSafeListen,
  isTauriAvailable: tauriMocks.mockIsTauriAvailable
}));

vi.mock("@/lib/runtime/utils", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/runtime/utils")>();
  return {
    ...actual,
    waitForNextPaint: () => Promise.resolve(),
    runLowPriorityTurnWork: (callback: () => void) => callback()
  };
});

import { advance, BASE_TIME, drainMicrotasks, SESSION_ID, setupRuntimeHarness } from "./harness";

const harness = setupRuntimeHarness(tauriMocks);
const TURN_ID = String(BASE_TIME);

/** fact-map §1 的 84 状态键（按域排序，作为逐项对账基线）。 */
const STATE_KEYS = [
  // D1 会话目录与生命周期（11）
  "sessionId", "sessionList", "sessionOperation", "sessionHydrating", "deletingSessionSet",
  "sessionSwitchToken", "sessionError", "sessionNotice", "runningSessionMap", "completedSessionSet", "failedSessionSet",
  // D2 工作区树（5）
  "workspaceList", "workspaceListLoaded", "sidebarOpInflightSet", "activeWorkspaceId", "sessionWorkspaceId",
  // D3 轮次执行与流式传输（20）
  "phase", "isSubmitting", "activeTurnId", "activeTurnStartedAtMs", "activeRunId", "activeStreamKind",
  "retryProgress", "browserPreviewRunToken", "streamBufferTurnId", "streamBufferText", "streamBufferReasoning",
  "streamFlushFrameId", "streamFlushTimerId", "deferredPersistTimerId", "submissionWatchdogTimerId", "eventsReady",
  "streamDebugDeltaCount", "streamDebugFlushCount", "streamDebugTextCharsReceived", "streamDebugTextCharsFlushed",
  // D4 消息与输入（10）
  "messages", "draftMessage", "pendingAttachments", "attachmentAssets", "sessionSummary", "error",
  "toolActivities", "traceSteps", "retrievedContext", "messageRevision",
  // D5 遥测/指标/Provider 标注（18）
  "traceTimeline", "turnTraceHistory", "eventCursorByTurnId", "liveMetrics", "activeProviderCallRecords",
  "providerRequestedName", "providerName", "providerProtocol", "providerModel", "providerSource", "providerMode",
  "fallbackReason", "inputTokens", "outputTokens", "totalTokens", "firstTokenLatencyMs",
  "pendingThrottledTraceTimeline", "traceTimelineThrottleTimerId",
  // D6 历史/分支/检查点（13）
  "visibleNodeId", "branchHeadNodeId", "activeBranchId", "cursorVersion", "historyCursorMode", "historyNodes",
  "historyBranches", "initialRollbackActive", "latestExecutionCheckpoint", "latestGraphRunSubmissionPlan",
  "latestGraphRunControlBoundaryEvidence", "latestRunControlAuditSummary", "latestHistoryStateAuditSummary",
  // D7 队列与授权策略（3）
  "pendingQueuedMessages", "queueDeliveryMode", "toolAuthorizationMode",
  // D8 健康与能力目录（4）
  "health", "availableTools", "capabilitySources", "capabilities"
];

/** fact-map §2 的 18 getter。 */
const GETTER_NAMES = [
  "phaseLabel", "isHistoricalMode", "isSessionDeleting", "conversationCheckpointEntries", "isSessionRunning",
  "failedTurnRetry", "agentRuntimeStatus", "formattedSessionCost", "isCostBudgetExceeded",
  "costBudgetWarningMessage", "formattedGovernanceCost", "governanceBlockDetail",
  "activeTraceId", "sessionTotalCostUsd", "hasUnpricedTurns", "usageGovernanceSummary",
  "canSubmitPrompt", "isInputDisabled"
];

/**
 * fact-map §3 标称 96；实测 action = **97**（多出 `performHistoryOperation:2073`，fact-map 未登记）。
 * 见交付报告「fact-map 与代码事实差异」。
 */
const ACTION_NAMES = [
  "overrideGovernanceBlock", "fetchUsageGovernanceSummary", "resetSessionRuntimeState",
  "cancelDeferredPersist", "cancelStreamFlush", "resetStreamDebugMetrics", "flushBufferedStreamText",
  "scheduleStreamFlush", "cancelBrowserPreviewTurn", "scheduleDeferredPersist", "startSubmissionWatchdog",
  "rearmSubmissionWatchdog", "handleSubmissionWatchdogTimeout", "clearSubmissionWatchdog", "persistHistory",
  "updatePersistedBackgroundSession", "getAttachmentAssets", "applyHistoryState", "applyMessageStateSnapshot",
  "applyMessageStateDelta", "loadSessionCatalog", "loadSessionRuntimeViewState", "applyExecutionCheckpoint",
  "loadSessionState", "loadRetrievedContextState", "resolveDerivedSessionRun", "applySessionSnapshot",
  "rollbackToInitialState", "rollbackToTurnBoundary", "performHistoryOperation", "checkoutHistoryNode",
  "findCheckpointTurnIdByNodeId", "restoreBranchHead", "forkHistoryNode", "switchHistoryBranch",
  "switchSession", "loadWorkspaces", "normalizeActiveWorkspace", "createNewWorkspace", "isSidebarOpInflight",
  "renameWorkspace", "deleteWorkspace", "renameSession", "enterFreshSession", "archiveSession",
  "unarchiveSession", "activateWorkspace", "createSession", "deleteSession", "initializeSessions",
  "upsertTurnTrace", "shouldProcessTurnEvent", "commitTurnEventCursor", "resolveTurnTraceTimeline",
  "commitTurnTraceTimeline", "updateActiveTraceTimeline", "publishTraceTimeline", "enqueueMessage",
  "promoteToSteer", "removeQueuedMessage", "toggleQueueDeliveryMode", "setQueueDeliveryMode",
  "setToolAuthorizationMode", "dequeueNextMessage", "scheduleThrottledTraceTimeline",
  "flushPendingTraceTimeline", "clearPendingTraceTimeline", "updateActiveModelTraceFromAssistant",
  "applyTurnTokenStats", "applyActiveProviderCallRecords", "applyMetricsSnapshot", "applyOutputEnd",
  "clearTimeoutRetryPlaceholder", "applyTurnRetryProgress", "ensureAssistantMessage", "syncToolMessages",
  "setDraftMessage", "recallLastUserMessage", "setPendingAttachments", "clearPendingAttachments",
  "removePendingAttachment", "addPendingAttachments", "fetchHealth", "fetchAvailableTools",
  "fetchCapabilitySources", "fetchCapabilities", "inspectCapability", "inspectCapabilitySource",
  "initializeTurnEvents", "disposeTurnEvents", "registerTurnEventListeners", "runBrowserPreviewTurn",
  "stopTurn", "enterStoppingPhase", "setSessionNotice", "retryLastTurn", "submitTurn"
];

/** 本套件实际驱动到的 action（直接调用，或由所驱动流程/事件必然触发）。 */
const COVERED_ACTIONS = [
  "submitTurn", "initializeTurnEvents", "disposeTurnEvents", "registerTurnEventListeners",
  "applyOutputEnd", "applyTurnTokenStats", "ensureAssistantMessage", "syncToolMessages",
  "applyMetricsSnapshot", "applyActiveProviderCallRecords", "applyTurnRetryProgress",
  "clearTimeoutRetryPlaceholder", "publishTraceTimeline", "commitTurnTraceTimeline", "upsertTurnTrace",
  "updateActiveTraceTimeline", "updateActiveModelTraceFromAssistant", "scheduleStreamFlush",
  "flushBufferedStreamText", "cancelStreamFlush", "resetStreamDebugMetrics", "scheduleDeferredPersist",
  "cancelDeferredPersist", "startSubmissionWatchdog", "rearmSubmissionWatchdog",
  "handleSubmissionWatchdogTimeout", "clearSubmissionWatchdog", "persistHistory",
  "resetSessionRuntimeState", "applySessionSnapshot", "applyExecutionCheckpoint", "loadSessionState",
  "loadSessionRuntimeViewState", "loadSessionCatalog", "initializeSessions", "switchSession",
  "checkoutHistoryNode", "switchHistoryBranch", "performHistoryOperation", "applyMessageStateSnapshot",
  "applyMessageStateDelta", "applyHistoryState", "enqueueMessage", "promoteToSteer", "removeQueuedMessage",
  "toggleQueueDeliveryMode", "setQueueDeliveryMode", "setToolAuthorizationMode", "dequeueNextMessage",
  "setDraftMessage", "setPendingAttachments", "clearPendingAttachments", "recallLastUserMessage",
  "flushPendingTraceTimeline", "clearPendingTraceTimeline", "shouldProcessTurnEvent", "commitTurnEventCursor"
];

/** 未覆盖 action + 结构性原因（不得省略：覆盖自评要与代码事实可机械对账）。 */
const UNCOVERED_ACTIONS: Array<{ name: string; reason: string }> = [
  { name: "loadWorkspaces", reason: "D2 工作区注册表需宿主 IPC（非 6 类场景）" },
  { name: "createNewWorkspace", reason: "D2 注册表写操作（非 6 类场景）" },
  { name: "renameWorkspace", reason: "D2 注册表写操作（非 6 类场景）" },
  { name: "deleteWorkspace", reason: "D2 注册表写操作（非 6 类场景）" },
  { name: "activateWorkspace", reason: "D2 localStorage 激活态（非 6 类场景）" },
  { name: "normalizeActiveWorkspace", reason: "D2 激活态归一（需注册表投影）" },
  { name: "isSidebarOpInflight", reason: "D2/D1 侧边栏 inflight 守卫（需并发 UI 操作）" },
  { name: "renameSession", reason: "D1 目录写操作（依赖会话目录）" },
  { name: "archiveSession", reason: "D1 归档语义（进入全新会话的分支）" },
  { name: "unarchiveSession", reason: "D1 归档语义" },
  { name: "deleteSession", reason: "D1 删除语义（会话目录 + 回落策略）" },
  { name: "createSession", reason: "D1 新建会话（UUID + 列表投影）" },
  { name: "enterFreshSession", reason: "D1 空白会话落点（UUID + 列表投影）" },
  { name: "forkHistoryNode", reason: "D1 分叉（需宿主历史写能力）" },
  { name: "restoreBranchHead", reason: "D1 恢复 branch head（需宿主历史写能力）" },
  { name: "rollbackToInitialState", reason: "撤回的本地分支（checkout 已覆盖宿主分支）" },
  { name: "rollbackToTurnBoundary", reason: "撤回的本地分支（checkout 已覆盖宿主分支）" },
  { name: "findCheckpointTurnIdByNodeId", reason: "仅浏览器预览（无宿主）分支使用" },
  { name: "loadRetrievedContextState", reason: "宿主检索上下文派生（submit 路径被测试路由短路）" },
  { name: "resolveDerivedSessionRun", reason: "宿主 run 派生（同上）" },
  { name: "updatePersistedBackgroundSession", reason: "后台会话轮事件（需先切走后持续收流）" },
  { name: "resolveTurnTraceTimeline", reason: "既有死代码：src 内无调用方（Wave-1 不清理）" },
  { name: "scheduleThrottledTraceTimeline", reason: "delta 未携带 traceTimeline（节流分支未触发）" },
  { name: "addPendingAttachments", reason: "附件导入需 File/host import 边界" },
  { name: "removePendingAttachment", reason: "附件 chip 交互（同上）" },
  { name: "getAttachmentAssets", reason: "纯视图过滤，无 store 内部调用方" },
  { name: "fetchHealth", reason: "D8 健康探针（启动面）" },
  { name: "fetchAvailableTools", reason: "D8 能力目录（启动面）" },
  { name: "fetchCapabilitySources", reason: "D8 能力目录（启动面）" },
  { name: "fetchCapabilities", reason: "D8 能力目录（启动面）" },
  { name: "inspectCapability", reason: "D8 能力目录细节（UI 面板驱动）" },
  { name: "inspectCapabilitySource", reason: "D8 能力目录细节（UI 面板驱动）" },
  { name: "overrideGovernanceBlock", reason: "用量治理（写 settings 持久化）" },
  { name: "fetchUsageGovernanceSummary", reason: "用量治理（宿主 summary 通道）" },
  { name: "runBrowserPreviewTurn", reason: "浏览器预览兜底（无宿主分支）" },
  { name: "cancelBrowserPreviewTurn", reason: "浏览器预览兜底（无宿主分支）" },
  { name: "stopTurn", reason: "停止链（宿主 stop_turn/stop_graph_run 控制面）" },
  { name: "enterStoppingPhase", reason: "停止链（进行中态，随 stopTurn 覆盖）" },
  { name: "retryLastTurn", reason: "续跑入口（判定侧已由 failed + failedTurnRetry 覆盖）" },
  { name: "setSessionNotice", reason: "斜杠命令中性反馈（getter info 分支已由快照覆盖）" }
];

const EXPECTED_LISTENER_NAMES = [
  "turn:cancelled",
  "turn:checkpoint_persisted",
  "turn:completed",
  "turn:delta",
  "turn:failed",
  "turn:hop_complete",
  "turn:output_end",
  "turn:phase_changed",
  "turn:retry",
  "turn:started",
  "turn:stop_requested",
  "turn:suspended",
  "turn:tool",
  "turn:trace"
];

function storeAsRecord(store: ReturnType<typeof harness.getStore>): Record<string, unknown> {
  return store as unknown as Record<string, unknown>;
}

describe("场景 6 · 并发与生命周期", () => {
  it("store 表面契约：84 键 / 18 getters / 97 actions，且覆盖清单与 action 全集可机械对账", () => {
    const store = harness.getStore();
    const record = storeAsRecord(store);
    const storeKeys = Object.keys(record).filter((key) => !key.startsWith("$") && !key.startsWith("_"));
    const stateKeys = Object.keys(store.$state);
    const fnKeys = storeKeys.filter((key) => typeof record[key] === "function");
    const valueKeys = storeKeys.filter((key) => typeof record[key] !== "function");
    const functionGetters = fnKeys.filter((key) => GETTER_NAMES.includes(key));
    const actionKeys = fnKeys.filter((key) => !GETTER_NAMES.includes(key));
    const valueGetters = valueKeys.filter((key) => !stateKeys.includes(key));

    // 状态键：与 fact-map §1 的 84 键（8 域）逐项一致。
    expect([...stateKeys].sort()).toEqual([...STATE_KEYS].sort());
    expect(STATE_KEYS).toHaveLength(84);
    // getter：16 个值式 + 2 个方法式（isSessionDeleting / isSessionRunning）= 18。
    expect([...functionGetters, ...valueGetters].sort()).toEqual([...GETTER_NAMES].sort());
    expect(GETTER_NAMES).toHaveLength(18);
    // action：97（fact-map 记 96；差集 = performHistoryOperation — 见报告差异项）。
    expect([...actionKeys].sort()).toEqual([...ACTION_NAMES].sort());
    expect(ACTION_NAMES).toHaveLength(97);
    // 覆盖自评的完整性：已覆盖 ∪ 未覆盖 == 全部 action，且两侧不相交。
    const uncoveredNames = UNCOVERED_ACTIONS.map((item) => item.name);
    expect([...COVERED_ACTIONS, ...uncoveredNames].sort()).toEqual([...ACTION_NAMES].sort());
    expect(COVERED_ACTIONS.filter((name) => uncoveredNames.includes(name))).toEqual([]);
    expect(COVERED_ACTIONS).toHaveLength(57);
    expect(uncoveredNames).toHaveLength(40);
    // 每个 action 名字确实可调用（类型面契约，非行为门禁）。
    for (const name of ACTION_NAMES) {
      expect(typeof record[name], `action ${name} 应可调用`).toBe("function");
    }
  });

  it("并发 initializeTurnEvents 幂等：单例 initPromise，14 个监听器只注册一次", async () => {
    const store = harness.getStore();
    const backend = harness.backend();

    await Promise.all([
      store.initializeTurnEvents(),
      store.initializeTurnEvents(),
      store.initializeTurnEvents()
    ]);
    expect(store.eventsReady).toBe(true);
    expect(tauriMocks.mockSafeListen).toHaveBeenCalledTimes(14);
    expect(backend.listenerCount()).toBe(14);
    expect(backend.listenerNames()).toEqual(EXPECTED_LISTENER_NAMES);

    // 已就绪后再次调用：早退，不重复注册。
    await store.initializeTurnEvents();
    expect(tauriMocks.mockSafeListen).toHaveBeenCalledTimes(14);
    expect(backend.unlistenCount).toBe(0);
  });

  it("disposeTurnEvents 在初始化中途被调用：等在飞 init 落定后全量注销，且可重新注册", async () => {
    const store = harness.getStore();
    const backend = harness.backend();

    const release = backend.gateNextListen("turn:delta");
    const initPromise = store.initializeTurnEvents();
    await drainMicrotasks();
    // 中途只登记了 turn:retry 与 turn:started（delta 被 gate 挂起）。
    expect(backend.listenerCount()).toBe(2);

    const disposePromise = store.disposeTurnEvents();
    await drainMicrotasks();
    release();
    await initPromise;
    await disposePromise;

    expect(store.eventsReady).toBe(false);
    expect(backend.listenerCount()).toBe(0);
    expect(backend.unlistenCount).toBe(14);

    // 重新初始化：句柄重新登记，事件可再次驱动（无残留/无重复）。
    await store.initializeTurnEvents();
    expect(store.eventsReady).toBe(true);
    expect(backend.listenerCount()).toBe(14);
    expect(tauriMocks.mockSafeListen).toHaveBeenCalledTimes(28);
  });

  it("delta 风暴 5000 条（固定循环 + 固定毫秒推进）：合并写次数/字符数确定，无丢字/无重复", async () => {
    const store = harness.getStore();
    const backend = harness.backend();
    backend
      .routeValue("resolve_graph_run_submission_plan", { command: "start_graph_run_stream", runId: null })
      .routeValue("start_graph_run_stream", { run: { id: "run-storm" }, turnId: TURN_ID })
      .routeValue("list_sessions", [])
      .routeValue("load_retrieved_context", null);

    store.$patch({ sessionId: SESSION_ID, phase: "idle" });
    store.setDraftMessage("delta 风暴");
    await store.submitTurn();
    await drainMicrotasks();

    backend.emit("turn:started", {
      sessionId: SESSION_ID,
      turnId: TURN_ID,
      kind: "started",
      eventId: "storm-started",
      eventType: "turn.model_call_started",
      sequence: 2,
      emittedAtMs: 1_000,
      phase: "calling_model"
    });

    const DELTA_COUNT = 5_000;
    for (let index = 0; index < DELTA_COUNT; index += 1) {
      backend.emit("turn:delta", {
        sessionId: SESSION_ID,
        turnId: TURN_ID,
        kind: "delta",
        eventId: `storm-delta-${index}`,
        eventType: "turn.output_delta",
        sequence: 10 + index,
        emittedAtMs: 1_000 + index,
        text: "x"
      });
    }
    // 仅剩尾缓冲（5000 % 120 = 80 字符）挂在 120ms 定时器上。
    expect(store.streamDebugDeltaCount).toBe(DELTA_COUNT);
    expect(store.streamDebugTextCharsReceived).toBe(DELTA_COUNT);
    expect(store.messages[1]?.content).toHaveLength(DELTA_COUNT - 80);

    backend.emit("turn:output_end", {
      sessionId: SESSION_ID,
      turnId: TURN_ID,
      kind: "output_end",
      eventId: "storm-output-end",
      eventType: "turn.output_end",
      sequence: 10 + DELTA_COUNT,
      emittedAtMs: 2_000 + DELTA_COUNT,
      text: "x".repeat(DELTA_COUNT),
      outputTokens: 5_000
    });
    expect(store.messages[1]?.content).toBe("x".repeat(DELTA_COUNT));
    // 41 次「攒够 120 字符」的合并写 + output_end 的收尾冲刷 = 42。
    expect(store.streamDebugFlushCount).toBe(42);
    expect(store.streamDebugTextCharsFlushed).toBe(DELTA_COUNT);

    backend.emit("turn:completed", {
      sessionId: SESSION_ID,
      turnId: TURN_ID,
      kind: "completed",
      eventId: "storm-completed",
      eventType: "turn.completed",
      sequence: 11 + DELTA_COUNT,
      emittedAtMs: 3_000 + DELTA_COUNT,
      phase: "completed",
      outputTokens: 5_000
    });
    await drainMicrotasks();
    await advance(200);

    expect(store.phase).toBe("ready");
    expect(store.isSubmitting).toBe(false);
    expect(store.activeTurnId).toBeNull();
    expect(store.messages[1]?.content).toBe("x".repeat(DELTA_COUNT));
    expect(store.messageRevision).toBeNull();
    expect(store.turnTraceHistory[0]?.phase).toBe("completed");
    // 风暴结束后无残留计时器（全部落定或被清理）。
    const record = storeAsRecord(store);
    expect(record.streamFlushTimerId).toBeNull();
    expect(record.submissionWatchdogTimerId).toBeNull();
  });
});
