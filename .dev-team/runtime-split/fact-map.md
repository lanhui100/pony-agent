# runtime store 拆解事实图（Wave 1 真源）

> **修订注记（2026-11-22，B279）**：D3 的 `streamFlushFrameId` 经证伪为死键
> （全仓无 `requestAnimationFrame` 赋值点，只有 `cancelAnimationFrame` 读取），
> 已随 B279 删除，state 键 84 → 83。本文件正文保持拆解前测绘原貌，只加此注记；
> 残余清单见同目录 `contract-residuals.md` 的 R1 / R2。

> 采信口径：Explorer 测绘 + Lead 抽验。抽验结论：state 键 **84**（`src/stores/runtime.ts:468-578`）、
> getters **18**（12 个方法式 `phaseLabel(state)` + 6 个箭头式 `activeTraceId: (state) => ...`，:581-913）、
> actions **96**（:914-6075）、真实消费方 **14** 个文件（`grep -rl useRuntimeStore src` 共 15 命中，含定义自身）。
> 本文件是切分归属的唯一真源；实现与归属冲突时以本文件为准并回写修订记录。

## 1. 域划分与状态键归属（8 域 / 84 键）

| 域 | 模块文件 | 键数 | 键名 |
|---|---|---|---|
| D1 会话目录与生命周期 | `src/stores/runtime/session-catalog.ts` | 11 | sessionId, sessionList, sessionOperation, sessionHydrating, deletingSessionSet, sessionSwitchToken, sessionError, sessionNotice, runningSessionMap, completedSessionSet, failedSessionSet |
| D2 工作区树 | `src/stores/runtime/workspace.ts` | 5 | workspaceList, workspaceListLoaded, sidebarOpInflightSet, activeWorkspaceId, sessionWorkspaceId |
| D3 轮次执行与流式传输 | `src/stores/runtime/turn-execution.ts` | 20 | phase, isSubmitting, activeTurnId, activeTurnStartedAtMs, activeRunId, activeStreamKind, retryProgress, browserPreviewRunToken, streamBufferTurnId, streamBufferText, streamBufferReasoning, streamFlushFrameId, streamFlushTimerId, deferredPersistTimerId, submissionWatchdogTimerId, eventsReady, streamDebugDeltaCount, streamDebugFlushCount, streamDebugTextCharsReceived, streamDebugTextCharsFlushed |
| D4 消息与输入 | `src/stores/runtime/messages.ts` | 10 | messages, draftMessage, pendingAttachments, attachmentAssets, sessionSummary, error, toolActivities, traceSteps, retrievedContext, messageRevision |
| D5 遥测/指标/Provider 标注 | `src/stores/runtime/telemetry.ts` | 18 | traceTimeline, turnTraceHistory, eventCursorByTurnId, liveMetrics, activeProviderCallRecords, providerRequestedName, providerName, providerProtocol, providerModel, providerSource, providerMode, fallbackReason, inputTokens, outputTokens, totalTokens, firstTokenLatencyMs, pendingThrottledTraceTimeline, traceTimelineThrottleTimerId |
| D6 历史/分支/检查点 | `src/stores/runtime/history.ts` | 13 | visibleNodeId, branchHeadNodeId, activeBranchId, cursorVersion, historyCursorMode, historyNodes, historyBranches, initialRollbackActive, latestExecutionCheckpoint, latestGraphRunSubmissionPlan, latestGraphRunControlBoundaryEvidence, latestRunControlAuditSummary, latestHistoryStateAuditSummary |
| D7 队列与授权策略 | `src/stores/runtime/queue.ts` | 3 | pendingQueuedMessages, queueDeliveryMode, toolAuthorizationMode |
| D8 健康与能力目录 | `src/stores/runtime/catalog.ts` | 4 | health, availableTools, capabilitySources, capabilities |

铰链键（归属若改，跨域计数会变，禁止在 Wave 1 改归属）：`messageRevision` → D4。

## 2. getters（18）

方法式 12：`phaseLabel:581`、`isHistoricalMode:597`、`isSessionDeleting:600`、`conversationCheckpointEntries:603`、
`isSessionRunning:655`、`failedTurnRetry:664`、`agentRuntimeStatus:670`、`formattedSessionCost:821`、
`isCostBudgetExceeded:836`、`costBudgetWarningMessage:845`、`formattedGovernanceCost:877`、`governanceBlockDetail:893`。

箭头式 6：`activeTraceId:790`、`sessionTotalCostUsd:805`、`hasUnpricedTurns:815`、`usageGovernanceSummary:852`、
`canSubmitPrompt:906`、`isInputDisabled:909`。

跨域 getter（归属按「主要输出」定，其余域只读）：
- `conversationCheckpointEntries` D6 主（读 messages=D4）→ 落 `history.ts`
- `agentRuntimeStatus` D3 主（读 D1/D4 的 sessionError/sessionNotice/messages/toolActivities）→ 落 `turn-execution.ts`
- `activeTraceId` D5 主（读 activeTurnId=D3）→ 落 `telemetry.ts`
- 其余按主键归域。

## 3. actions（97）归属

| 域 | 数量 | action（含行号） |
|---|---|---|
| D1 | 5 | loadSessionCatalog:1545、deleteSession:2851、forkHistoryNode:2293、switchHistoryBranch:2312、setSessionNotice:5722 |
| D2 | 6 | loadWorkspaces:2504、createNewWorkspace:2544、renameWorkspace:2580、deleteWorkspace:2608、activateWorkspace:2771、normalizeActiveWorkspace:2530 |
| D3 | 14 | scheduleStreamFlush:1071、flushBufferedStreamText:1029、cancelStreamFlush:1006、startSubmissionWatchdog:1179、rearmSubmissionWatchdog:1189、clearSubmissionWatchdog:1311、scheduleDeferredPersist:1165、cancelDeferredPersist:995、resetStreamDebugMetrics:1023、clearTimeoutRetryPlaceholder:3490、applyTurnRetryProgress:3518、initializeTurnEvents:3981、disposeTurnEvents:4020、enterStoppingPhase:5712 |
| D4 | 11 | applyMessageStateSnapshot:1501、resolveDerivedSessionRun:1771、applyTurnTokenStats:3381、ensureAssistantMessage:3532、syncToolMessages:3561、setDraftMessage:3657、setPendingAttachments:3678、clearPendingAttachments:3681、removePendingAttachment:3684、addPendingAttachments:3692、retryLastTurn:5733 |
| D5 | 7 | upsertTurnTrace:3044、publishTraceTimeline:3242、scheduleThrottledTraceTimeline:3294、flushPendingTraceTimeline:3310、clearPendingTraceTimeline:3321、applyActiveProviderCallRecords:3407、applyMetricsSnapshot:3435 |
| D6 | 2 | applyHistoryState:1474、loadSessionState:1679 |
| D7 | 6 | enqueueMessage:3246、promoteToSteer:3264、removeQueuedMessage:3275、toggleQueueDeliveryMode:3281、setQueueDeliveryMode:3284、setToolAuthorizationMode:3287 |
| D8 | 3 | fetchAvailableTools:3858、fetchCapabilitySources:3883、fetchCapabilities:3898 |
| 无状态写（22，按被谁调用就近归域） | 22 | persistHistory:1317、updatePersistedBackgroundSession:1384、shouldProcessTurnEvent:3123、commitTurnEventCursor:3193、resolveTurnTraceTimeline:3203、commitTurnTraceTimeline:3215、updateActiveTraceTimeline:3235、recallLastUserMessage:3666、dequeueNextMessage:3290、updateActiveModelTraceFromAssistant:3328、overrideGovernanceBlock:914、fetchUsageGovernanceSummary:926、inspectCapability:3939、inspectCapabilitySource:3960、renameSession:2637、archiveSession:2709、unarchiveSession:2749、getAttachmentAssets:1471、loadSessionRuntimeViewState:1576、loadRetrievedContextState:1738、findCheckpointTurnIdByNodeId:2261、isSidebarOpInflight:2576 |
| 跨域编排（保留在 orchestration.ts / turn-events.ts） | 见 §4 | — |

**修订记录（Wave 1 实测，Protocol Agent 登记）**：actions 实测 **97**，原「96」漏计泛型辅助
`performHistoryOperation<TInvokeArgs, TResult>`（源码位于 `checkoutHistoryNode` 之前，非 `async`），
归属 `history.ts`；其余 96 条计数与归属不变。

**无状态写 22 条的冻结落位（避免与 `types.ts` 各域 `*Actions` 接口形成双份真源）**：
`session-catalog` ← renameSession / archiveSession / unarchiveSession / loadSessionRuntimeViewState；
`workspace` ← isSidebarOpInflight；`messages` ← recallLastUserMessage / getAttachmentAssets / loadRetrievedContextState；
`telemetry` ← resolveTurnTraceTimeline / commitTurnTraceTimeline / updateActiveTraceTimeline /
updateActiveModelTraceFromAssistant / overrideGovernanceBlock / fetchUsageGovernanceSummary；
`history` ← findCheckpointTurnIdByNodeId（+ 上条 performHistoryOperation）；`queue` ← dequeueNextMessage；
`catalog` ← inspectCapability / inspectCapabilitySource；`turn-events` ← shouldProcessTurnEvent / commitTurnEventCursor；
`orchestration` ← persistHistory / updatePersistedBackgroundSession。

## 4. 切分阻断点（跨域编排，必须在 orchestration.ts / turn-events.ts 落地）

直接同写 ≥2 域：**20 个**；叠加 `this.<action>()` 闭包后 **35 个**；三个总入口 `resetSessionRuntimeState:937`、
`applySessionSnapshot:1812`、`submitTurn:5745` 覆盖 D1-D6 六域。

必须归入 `orchestration.ts`（跨域事务入口，Wave 1 只搬家不改语义）：
`resetSessionRuntimeState:937`、`applySessionSnapshot:1812`、`submitTurn:5745`、`stopTurn:5668`、
`switchSession:2330`、`enterFreshSession:2673`、`createSession:2782`、`initializeSessions:2929`、
`rollbackToInitialState:1959`、`rollbackToTurnBoundary:2026`、`checkoutHistoryNode:2109`、
`restoreBranchHead:2265`、`applyExecutionCheckpoint:1596`、`applyMessageStateDelta:1510`、
`applyOutputEnd:3441`、`fetchHealth:3820`、`runBrowserPreviewTurn:5574`、`cancelBrowserPreviewTurn:1092`、
`handleSubmissionWatchdogTimeout:1198`。

必须归入 `turn-events.ts`：`registerTurnEventListeners:4042-5573`（1532 行、14 个监听器聚合体）及其
唯一私有的辅助逻辑；`initializeTurnEvents/disposeTurnEvents` 留在 `turn-execution.ts`，通过注入或
导出调用 `turn-events.ts` 的注册体。

## 5. 事件订阅生命周期（搬家时逐字保留）

14 个监听器：`turn:retry`(safeListen)、`turn:started`、`turn:delta`、`turn:trace`、`turn:phase_changed`、
`turn:checkpoint_persisted`、`turn:tool`、`turn:output_end`、`turn:hop_complete`、`turn:completed`、
`turn:failed`、`turn:cancelled`、`turn:stop_requested`、`turn:suspended`。

防护机制（不得改写）：`eventsReady` 早退、`initPromise` 并发单例、每 store 实例的 WeakMap registry、
`listen()` 边等边登记句柄、`releaseTurnEventRegistry`、`disposeTurnEvents` 先 await 在飞 init 再清理、
注册中失败回滚、HMR `import.meta.hot.dispose`。历史缺陷注释（:306-309、:3993、:6065）必须随代码一起搬家。

## 6. 持久化面（搬家不得改语义）

- 容器键 `pony-agent.runtime-history.v1`（`src/lib/runtime/constants.ts:7`）。
- state 初始化恢复：sessionWorkspaceId、phase（含 canonicalTerminalPhase 归一）、sessionSummary、
  providerRequestedName..providerMode、fallbackReason、inputTokens/outputTokens/totalTokens/firstTokenLatencyMs、
  messages、attachmentAssets、turnTraceHistory、eventCursorByTurnId。
- `persistHistory:1317-1383` 落盘面：另含 visibleNodeId、branchHeadNodeId、activeBranchId、historyCursorMode、
  historyNodes、historyBranches、cursorVersion、initialRollbackActive、checkpoint、runningTurnId。
- `turnTraceHistory` 明确不落盘；`runningTurnId` 只写不读（既有现象，Wave 1 不清理）。
- localStorage：`pony-agent.active-workspace.v1`（读 :255 写 :2776 清 :2537）。

## 7. 消费方（14，Wave 1 零改动）

App.vue、HomeWorkspace.vue（宽 15 键）、lib/runtime/useTraceProjection.ts（宽 20 键）、
chat/WorkspaceComposer.vue（宽 13 键）、HomeSessionSidebar.vue（8 键 + 2 getter）、
chat/WorkspaceTurnItem.vue、config/ConfigToolsSection.vue、HomeSidebar.vue、HomeStatusPanel.vue、
common/ErrorBoundary.vue、main.ts、lib/tauri.ts（activeTraceId，热路径）、telemetry/TelemetryPage.vue、
TraceInspector.vue。

## 8. Wave 1 硬边界

1. **行为等价**：`src/stores/runtime/__tests__` 的 golden fixtures 逐字节不变。
2. **门面不变**：store id `runtime`、导出名 `useRuntimeStore`、84 键 / 18 getters / 96 actions 的名字与语义不变。
3. **禁止**：改 14 个消费方、`tests/` 下既有 107 个用例、`src/lib/**`、`src/types/**`、任何 Rust 文件。
4. **禁止顺手清理**：死分支（`state.backendGovernanceSummary`）、只写不读键（`runningTurnId`）、
   注释里的历史缺陷一律原样保留，登记为后续独立任务。

## 9. 修订记录（Wave 1 实测回写）

- **2026-11-22 · actions 97**：§3 表初版记 96，漏计泛型辅助 `performHistoryOperation`（归 `history.ts`）。
  以 `src/stores/runtime/types.ts` 实测为准：9+7+14+14+13+4+7+5+21+3 = **97**；加 18 getters 共 **115** 成员。
- **2026-11-22 · 铰链键补登 `sidebarOpInflightSet`**：原登记只有 `messageRevision`。Gate-1 审查实测该键归 D2
  （`workspace.ts` 的 state 切片），但由 D1 的 `session-catalog.ts` 写 9 处（:157/:166/:185/:205/:213/:226 等）。
  Wave 2 拆多 store 时它是第二个必须迁移归属的键，已入账本 B280。
- **2026-11-22 · 键集冻结断言的强度边界**：`types.ts` 的类型级断言对 3 个 optional 键（`activeStreamKind`/
  `liveMetrics`/`retryProgress`）不双向闭合 —— 删掉其默认值仍可 `vue-tsc` 通过（Gate-1 反例实测 TSC_EXIT=0）。
  真正的机械兜底是 `__tests__` 里对 `Object.keys(store.$state)` 的运行时断言（84 键），Wave 1 已补键集完整性用例。
