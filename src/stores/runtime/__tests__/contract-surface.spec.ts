/**
 * 契约门禁 · store 表面**完整性**（L3-B 审查缺口补齐）
 *
 * 缺口：10 个域模块用 `as XActions` 断言接入 Pinia，`tsc` 只做「可比较性」断言 ——
 * 实现里漏搬一条 action、或多出一个重名成员，`npx vue-tsc --noEmit` 仍可全绿，
 * 既有 23 条等价用例也只能证明「已搬部分的正确性」，不能证明「完整性」。
 *
 * 本文件补的是完整性判据（三个正交断言）：
 *   1. `Object.keys(store.$state)` == 84 键（逐项对账）；
 *   2. 18 个 getter 在 store 上齐全，且「函数式 getter / 值式 getter」归属不漂移；
 *   3. store 上全部「函数型自有键」== 97 actions ∪ 2 个函数式 getter —— 漏搬（missing）
 *      与多出/重名（extra）分别以可读列表断言。
 *
 * 唯一清单来源：`src/stores/runtime/types.ts` 的 `FrozenStateKeys`（:174-198）、
 * `FrozenGetterKeys`（:555-560）、`FrozenActionKeys`（:563-594）——本文件用 `satisfies`
 * 保证清单是类型键集的子集（清单写错键名 ⇒ tsc 报错），运行时对照实际 store 抓差异。
 * 清单与 types.ts 漂移时：先改 types.ts 的冻结键集（需事实图修订），再同步本文件。
 *
 * 本文件不修改任何已冻结用例/fixtures，不引入新依赖，不碰生产代码。
 */
import { beforeEach, describe, expect, it } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { useRuntimeStore } from "@/stores/runtime";
import type {
  RuntimeStoreActions,
  RuntimeStoreGetters,
  RuntimeStoreState
} from "@/stores/runtime/types";

/** 事实图 §1 / types.ts `FrozenStateKeys`：84 个状态键。 */
const STATE_KEY_NAMES = [
  "sessionId", "sessionList", "sessionOperation", "sessionHydrating", "deletingSessionSet",
  "sessionSwitchToken", "sessionError", "sessionNotice", "runningSessionMap", "completedSessionSet",
  "failedSessionSet", "workspaceList", "workspaceListLoaded", "sidebarOpInflightSet",
  "activeWorkspaceId", "sessionWorkspaceId", "phase", "isSubmitting", "activeTurnId",
  "activeTurnStartedAtMs", "activeRunId", "activeStreamKind", "retryProgress",
  "browserPreviewRunToken", "streamBufferTurnId", "streamBufferText", "streamBufferReasoning",
  "streamFlushFrameId", "streamFlushTimerId", "deferredPersistTimerId", "submissionWatchdogTimerId",
  "eventsReady", "streamDebugDeltaCount", "streamDebugFlushCount", "streamDebugTextCharsReceived",
  "streamDebugTextCharsFlushed", "messages", "draftMessage", "pendingAttachments", "attachmentAssets",
  "sessionSummary", "error", "toolActivities", "traceSteps", "retrievedContext", "messageRevision",
  "traceTimeline", "turnTraceHistory", "eventCursorByTurnId", "liveMetrics",
  "activeProviderCallRecords", "providerRequestedName", "providerName", "providerProtocol",
  "providerModel", "providerSource", "providerMode", "fallbackReason", "inputTokens", "outputTokens",
  "totalTokens", "firstTokenLatencyMs", "pendingThrottledTraceTimeline",
  "traceTimelineThrottleTimerId", "visibleNodeId", "branchHeadNodeId", "activeBranchId",
  "cursorVersion", "historyCursorMode", "historyNodes", "historyBranches", "initialRollbackActive",
  "latestExecutionCheckpoint", "latestGraphRunSubmissionPlan",
  "latestGraphRunControlBoundaryEvidence", "latestRunControlAuditSummary",
  "latestHistoryStateAuditSummary", "pendingQueuedMessages", "queueDeliveryMode",
  "toolAuthorizationMode", "health", "availableTools", "capabilitySources", "capabilities"
] as const satisfies readonly (keyof RuntimeStoreState)[];

/** 事实图 §2 / types.ts `FrozenGetterKeys`：18 个 getter（12 方法式 + 6 箭头式，同列）。 */
const GETTER_NAMES = [
  // 12 个方法式 getter
  "phaseLabel", "isHistoricalMode", "isSessionDeleting", "conversationCheckpointEntries",
  "isSessionRunning", "failedTurnRetry", "agentRuntimeStatus", "formattedSessionCost",
  "isCostBudgetExceeded", "costBudgetWarningMessage", "formattedGovernanceCost",
  "governanceBlockDetail",
  // 6 个箭头式 getter
  "activeTraceId", "sessionTotalCostUsd", "hasUnpricedTurns", "usageGovernanceSummary",
  "canSubmitPrompt", "isInputDisabled"
] as const satisfies readonly (keyof RuntimeStoreGetters)[];

/**
 * 18 个 getter 中**返回值本身是函数**的两个（`isSessionDeleting` / `isSessionRunning` 返回
 * `(sessionId) => boolean`）。它们既是 getter 又出现在「函数型自有键」集合里，
 * 故 action 全集断言必须显式把这 2 个键剔除，否则会误判为「多出 action」。
 */
const FUNCTION_GETTER_NAMES = [
  "isSessionDeleting",
  "isSessionRunning"
] as const satisfies readonly (keyof RuntimeStoreGetters)[];

/** 事实图 §3 / types.ts `FrozenActionKeys`：97 个 action（96 + `performHistoryOperation`）。 */
const ACTION_NAMES = [
  "loadSessionCatalog", "deleteSession", "forkHistoryNode", "switchHistoryBranch", "setSessionNotice",
  "renameSession", "archiveSession", "unarchiveSession", "loadSessionRuntimeViewState",
  "loadWorkspaces", "createNewWorkspace", "renameWorkspace", "deleteWorkspace", "activateWorkspace",
  "normalizeActiveWorkspace", "isSidebarOpInflight", "scheduleStreamFlush",
  "flushBufferedStreamText", "cancelStreamFlush", "startSubmissionWatchdog", "rearmSubmissionWatchdog",
  "clearSubmissionWatchdog", "scheduleDeferredPersist", "cancelDeferredPersist",
  "resetStreamDebugMetrics", "clearTimeoutRetryPlaceholder", "applyTurnRetryProgress",
  "initializeTurnEvents", "disposeTurnEvents", "enterStoppingPhase", "applyMessageStateSnapshot",
  "resolveDerivedSessionRun", "applyTurnTokenStats", "ensureAssistantMessage", "syncToolMessages",
  "setDraftMessage", "setPendingAttachments", "clearPendingAttachments", "removePendingAttachment",
  "addPendingAttachments", "retryLastTurn", "recallLastUserMessage", "getAttachmentAssets",
  "loadRetrievedContextState", "upsertTurnTrace", "publishTraceTimeline",
  "scheduleThrottledTraceTimeline", "flushPendingTraceTimeline", "clearPendingTraceTimeline",
  "applyActiveProviderCallRecords", "applyMetricsSnapshot", "resolveTurnTraceTimeline",
  "commitTurnTraceTimeline", "updateActiveTraceTimeline", "updateActiveModelTraceFromAssistant",
  "overrideGovernanceBlock", "fetchUsageGovernanceSummary", "applyHistoryState", "loadSessionState",
  "findCheckpointTurnIdByNodeId", "performHistoryOperation", "enqueueMessage", "promoteToSteer",
  "removeQueuedMessage", "toggleQueueDeliveryMode", "setQueueDeliveryMode", "setToolAuthorizationMode",
  "dequeueNextMessage", "fetchAvailableTools", "fetchCapabilitySources", "fetchCapabilities",
  "inspectCapability", "inspectCapabilitySource", "resetSessionRuntimeState", "applySessionSnapshot",
  "submitTurn", "stopTurn", "switchSession", "enterFreshSession", "createSession",
  "initializeSessions", "rollbackToInitialState", "rollbackToTurnBoundary", "checkoutHistoryNode",
  "restoreBranchHead", "applyExecutionCheckpoint", "applyMessageStateDelta", "applyOutputEnd",
  "fetchHealth", "runBrowserPreviewTurn", "cancelBrowserPreviewTurn",
  "handleSubmissionWatchdogTimeout", "persistHistory", "updatePersistedBackgroundSession",
  "shouldProcessTurnEvent", "commitTurnEventCursor", "registerTurnEventListeners"
] as const satisfies readonly (keyof RuntimeStoreActions)[];

type StoreRecord = Record<string, unknown>;

/** 取 store 的「自有成员名」：排除 Pinia 内建（`$reset`/`$patch`/`$state`）与内部字段（`_hotUpdate`/`_p`）。 */
function ownMemberNames(store: unknown): string[] {
  return Object.keys(store as StoreRecord).filter(
    (key) => !key.startsWith("$") && !key.startsWith("_")
  );
}

describe("契约门禁 · store 表面完整性（84 state / 18 getters / 97 actions）", () => {
  let store: ReturnType<typeof useRuntimeStore>;
  let record: StoreRecord;

  beforeEach(() => {
    window.localStorage.clear();
    setActivePinia(createPinia());
    store = useRuntimeStore();
    record = store as unknown as StoreRecord;
  });

  it("state：Object.keys(store.$state) 与 84 键冻结清单双向零差集", () => {
    const actual: string[] = Object.keys(store.$state).sort();
    // 显式放宽为 string[]：`satisfies` 元组的 `includes` 只接受字面量联合，此处需要按任意键名比对。
    const expected: string[] = [...STATE_KEY_NAMES].sort();

    const missing = expected.filter((key) => !actual.includes(key));
    const extra = actual.filter((key) => !expected.includes(key));

    // 先断「缺失 / 多余」清单（失败信息直接给出键名），再断计数。
    expect(missing, `store.$state 缺少状态键：${JSON.stringify(missing)}`).toEqual([]);
    expect(extra, `store.$state 多出状态键：${JSON.stringify(extra)}`).toEqual([]);
    expect(STATE_KEY_NAMES).toHaveLength(84);
    expect(new Set(STATE_KEY_NAMES).size).toBe(84);
    expect(actual).toHaveLength(84);
  });

  it("getters：18 个 getter 齐全，且函数式/值式归属不漂移（多出即报）", () => {
    const missing = GETTER_NAMES.filter((name) => !(name in record));
    const functionGetters = GETTER_NAMES.filter((name) => typeof record[name] === "function");
    const valueGetters = GETTER_NAMES.filter((name) => typeof record[name] !== "function");

    // 值式 getter 的「多出」检测：store 自有成员里既不是 state、也不是函数 ⇒ 只能是值式 getter。
    const stateKeys = Object.keys(store.$state);
    const surfaceValueGetters = ownMemberNames(store)
      .filter((key) => !stateKeys.includes(key) && typeof record[key] !== "function")
      .sort();
    const extraValueGetters = surfaceValueGetters.filter(
      (key) => !(GETTER_NAMES as readonly string[]).includes(key)
    );

    // 先断「缺失 / 归属漂移 / 多余」清单（失败信息直接给出名字），再断计数。
    expect(missing, `store 缺少 getter：${JSON.stringify(missing)}`).toEqual([]);
    expect(
      [...functionGetters].sort(),
      `函数式 getter 归属漂移（应为 ${JSON.stringify([...FUNCTION_GETTER_NAMES].sort())}）`
    ).toEqual([...FUNCTION_GETTER_NAMES].sort());
    expect(
      [...valueGetters].sort(),
      "值式 getter 归属漂移（函数式 getter 被误判为值式）"
    ).toEqual([...GETTER_NAMES].filter((name) => !(FUNCTION_GETTER_NAMES as readonly string[]).includes(name)).sort());
    expect(
      surfaceValueGetters,
      `store 上多出未登记的值式 getter：${JSON.stringify(extraValueGetters)}`
    ).toEqual([...valueGetters].sort());
    expect(GETTER_NAMES).toHaveLength(18);
    expect(new Set(GETTER_NAMES).size).toBe(18);
  });

  it("actions：97 个 action 全部可调用，「函数型自有键」集合无漏搬/无多出重名", () => {
    const functionKeys = ownMemberNames(store)
      .filter((key) => typeof record[key] === "function")
      .sort();
    const expectedFunctionKeys: string[] = [...ACTION_NAMES, ...FUNCTION_GETTER_NAMES].sort();

    const missingActions = ACTION_NAMES.filter((name) => typeof record[name] !== "function");
    const missingFunctionKeys = expectedFunctionKeys.filter((key) => !functionKeys.includes(key));
    const extraFunctionKeys = functionKeys.filter((key) => !expectedFunctionKeys.includes(key));

    // 先断「缺失 / 漏搬 / 多出重名」清单（失败信息直接给出名字），再断计数。
    expect(missingActions, `store 缺少 action：${JSON.stringify(missingActions)}`).toEqual([]);
    expect(
      missingFunctionKeys,
      `函数型成员缺失（漏搬 action 或 getter 变成非函数）：${JSON.stringify(missingFunctionKeys)}`
    ).toEqual([]);
    expect(
      extraFunctionKeys,
      `store 上多出未登记的函数型成员（漏登记 / 重名 action）：${JSON.stringify(extraFunctionKeys)}`
    ).toEqual([]);
    expect(ACTION_NAMES).toHaveLength(97);
    expect(new Set(ACTION_NAMES).size, "ACTION_NAMES 清单内部有重名").toBe(97);
    // 双向集合相等（不只是「各方向无差」的重复表述）：函数型自有键 == 97 actions ∪ 2 函数式 getter。
    expect(functionKeys).toHaveLength(99);
  });
});
