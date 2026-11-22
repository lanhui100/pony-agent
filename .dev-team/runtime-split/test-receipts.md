# runtime store 行为等价测试 · 物理收据（task-1 / Test Agent）

工作树：`/home/dm/pony-agent-worktrees/runtime-split`（分支 `refactor/runtime-store-split`，基线 `b3278e47`）
被测对象：`src/stores/runtime.ts`（6075 行，Pinia options store，id `runtime`）——**零生产代码改动**。
交付物：`src/stores/runtime/__tests__/`（1 harness + 6 spec）、`src/stores/runtime/fixtures/`（15 个 golden JSON）。
红相锚定提交：`2f11dc2f` + tag `wave-runtime-store-split-red`（Lead 侧）。

---

## 0. 基线（冻结后）

| 项 | 命令 | 结果 |
|---|---|---|
| 全目录验收 | `npx vitest run src/stores/runtime/__tests__` | **EXIT 0**；Test Files 6 passed；Tests **23 passed**（日志 `/tmp/rt-test-green-final.log`） |
| 类型门禁 | `npx vue-tsc --noEmit` | **EXIT 0**（我侧 `/tmp/rt-tsc-1.log`；protocol-agent 复跑确认） |
| 落盘 payload 字节稳定 | 捕获前后 `sha256sum fixtures/*.json` | 14 个已有 fixture **逐字节未变**（diff 空），仅新增 `history-checkout-node.json` |

---

## 1. 变异自检 3/3（绿 → 红 → 回滚 → 绿）

备份：`cp src/stores/runtime.ts /tmp/rt-runtime.ts.bak`
基线 sha256：`430aa64e9b8093b8e8756045f67811ae3dee792886bc93b1d8df415a640cfef0`
每组回滚后复算 sha256 **均与基线逐字节一致**（见每组 `RESTORED_SHA`）。

### M1 · `persistHistory` 少写 1 个字段（删除 `cursorVersion: this.cursorVersion`，runtime.ts:1354）

- 命令：`npx vitest run src/stores/runtime/__tests__/history-persistence.spec.ts --no-file-parallelism`
- **EXIT 1**（Test Files 1 failed；Tests 3 failed | 2 passed）
- 关键断言原文（`/tmp/rt-m1-red.log`）：
  ```
  × persistHistory 落盘 payload 形状逐字冻结（含条件字段与运行态映射截断）
  × checkoutHistoryNode 应用 messageDelta：截断消息、推进 revision、写入历史游标并落盘
  × initializeSessions 冷启动走 local-cache：消息/历史游标/用量/运行态映射全量恢复
  AssertionError: expected undefined to be 9 // Object.is equality
  AssertionError: expected undefined to be null
  ```
- 回滚：`cp /tmp/rt-runtime.ts.bak src/stores/runtime.ts` → `RESTORED_SHA 430aa64e…`
- 回绿：同命令 **EXIT 0**（5 passed，`/tmp/rt-m1-green.log`）

### M2 · delta 不推进 `messageRevision`（`delta.targetRevision` → `delta.baseRevision`，runtime.ts:1525）

- 命令：`npx vitest run src/stores/runtime/__tests__/history-persistence.spec.ts --no-file-parallelism`
- **EXIT 1**（Tests 2 failed | 3 passed）
- 关键断言原文（`/tmp/rt-m2-red.log`）：
  ```
  × checkoutHistoryNode 应用 messageDelta：…推进 revision…
  × switchHistoryBranch：成功推进 branch 游标并应用 delta…
  AssertionError: expected 'rev-0' to be 'rev-1' // Object.is equality
  AssertionError: expected 'rev-a' to be 'rev-b' // Object.is equality
  ```
- 回滚：`RESTORED_SHA 430aa64e…`
- 回绿：同命令 **EXIT 0**（5 passed，`/tmp/rt-m2-green.log`）

### M3 · `turn:completed` 不清 phase（`unlockedPhase = completedPhase`，runtime.ts:4639）

- 命令：`npx vitest run src/stores/runtime/__tests__/turn-happy-path.spec.ts --no-file-parallelism`
- **EXIT 1**（Tests 1 failed | 1 passed）
- 关键断言原文（`/tmp/rt-m3-red.log`）：
  ```
  × started → delta×3 → tool → output_end → completed 全量快照与 fixtures 逐字一致
  AssertionError: expected 'completed' to be 'ready' // Object.is equality
  ```
- 回滚：`RESTORED_SHA 430aa64e…`
- 回绿：同命令 **EXIT 0**（2 passed，`/tmp/rt-m3-green.log`）

**收口证据**：`sha256sum src/stores/runtime.ts` = `430aa64e…`（= 基线）；
`git status --porcelain` = 仅 `?? node_modules`；`git diff --stat HEAD -- src/stores/runtime.ts` 为空；
全目录最终 **EXIT 0 / 23 passed**。

---

## 2. Fixtures 清单（15 个 ↔ 场景）

| # | fixture | 场景 |
|---|---|---|
| 1 | `turn-happy-path.json` | 场景1 turn 快乐路径（started→delta×3→tool→phase_changed→output_end→completed，messages/trace/phase/usage/游标/落盘全量） |
| 2 | `turn-failed-path.json` | 场景2 `turn:failed`（零正文兼容锁 `"This turn failed."` 不落正文；phase=failed/canonical=failed） |
| 3 | `turn-cancelled-path.json` | 场景2 `turn:cancelled`（不补占位；`agentRuntimeStatus.mode=stopped`） |
| 4 | `turn-suspended-path.json` | 场景2 `turn:suspended`（waiting_user，无 canonicalTerminalPhase） |
| 5 | `turn-watchdog-timeout.json` | 场景2 看门狗超时（240s 假定时器推进；fail_turn_for_watchdog + stop_graph_run） |
| 6 | `session-reset-state.json` | 场景3 `resetSessionRuntimeState` 跨 D1/D3/D4/D5/D6 清空面 + D7 保留面 |
| 7 | `session-switch-cached-background-turn.json` | 场景3 `switchSession` 命中缓存 + 后台运行轮恢复（activeStreamKind 重建） |
| 8 | `session-switch-uncached.json` | 场景3 `switchSession` 未命中缓存（整域重置 + 空快照落点 phase=idle） |
| 9 | `session-apply-snapshot-historical.json` | 场景3 `applySessionSnapshot` + messageState（revision/历史游标/运行计划/审计摘要） |
| 10 | `persist-history-payload.json` | 场景4 `persistHistory` 落盘 payload 形状（条件字段、runningTurnId、无 turnTraceHistory、buffer 截断、运行态映射） |
| 11 | `history-checkout-node.json` | 场景4 `checkoutHistoryNode`（messageDelta 截断 + cursorVersion 清空 + 落盘） |
| 12 | `history-switch-branch.json` | 场景4 `switchHistoryBranch`（append delta + branch 游标；失败分支另测） |
| 13 | `initialize-sessions-restore.json` | 场景4 `initializeSessions` local-cache 冷启动（宿主失败被 contained） |
| 14 | `queue-state-machine.json` | 场景5 队列状态机（queue 追加/steer 插队/promote/remove/模式切换/出队） |
| 15 | `queue-submit-authorization-passthrough.json` | 场景5 `submitTurn` 的 `toolAuthorizationMode`/`workspaceId`/`sessionId` 透传 |

> 场景2 的 `turn:retry`（重试进度）为纯断言用例（无快照 fixture）：进度值/畸形负载/跨会话丢弃/首个增量清零，均由 `turn-exception-paths.spec.ts` 直接断言。
> 用例总数：**23**（happy 2 / exception 5 / session 5 / history 5 / queue 2 / concurrency 4）。

---

## 3. 覆盖自评（可机械对账）

- store 表面：**84 state 键 / 18 getters / 97 actions**（`concurrency-lifecycle.spec.ts` 逐项对账 code 事实）。
- action 覆盖：**57 / 97**（直接调用，或由所驱动流程/事件必然触发）。
- getter 覆盖：**13 / 18**（快照求值）。
- 覆盖清单完整性：用例内机械断言 `COVERED ∪ UNCOVERED == 全部 97 action` 且两侧不相交 —— 不允许漏列。

### 3.1 未覆盖 action（40）与原因

| action | 原因 |
|---|---|
| `loadWorkspaces` / `createNewWorkspace` / `renameWorkspace` / `deleteWorkspace` / `activateWorkspace` / `normalizeActiveWorkspace` | D2 工作区树需宿主注册表 IPC，非 6 类场景 |
| `renameSession` / `archiveSession` / `unarchiveSession` / `deleteSession` / `createSession` / `enterFreshSession` / `forkHistoryNode` / `restoreBranchHead` | D1 目录写操作（依赖宿主删除/归档/分叉语义） |
| `isSidebarOpInflight` | D1/D2 侧边栏 inflight 守卫（需并发 UI 操作） |
| `rollbackToInitialState` / `rollbackToTurnBoundary` | 撤回的**本地**分支；同域宿主分支已由 `checkoutHistoryNode` 覆盖 |
| `findCheckpointTurnIdByNodeId` | 仅浏览器预览（无宿主）分支使用 |
| `loadRetrievedContextState` / `resolveDerivedSessionRun` | 宿主检索上下文/run 派生；submit 路径被 route 短路 |
| `updatePersistedBackgroundSession` | 后台会话轮事件（需切走后持续收流） |
| `resolveTurnTraceTimeline` | **既有死代码**：src 内无调用方（Wave-1 不清理，仅登记） |
| `scheduleThrottledTraceTimeline` | delta 未携带 `traceTimeline` ⇒ 200ms 节流分支未触发 |
| `addPendingAttachments` / `removePendingAttachment` | 附件导入需 File/host import 边界 |
| `getAttachmentAssets` | 纯视图过滤，无 store 内部调用方 |
| `fetchHealth` / `fetchAvailableTools` / `fetchCapabilitySources` / `fetchCapabilities` / `inspectCapability` / `inspectCapabilitySource` | D8 能力目录（启动面/UI 面板驱动） |
| `overrideGovernanceBlock` / `fetchUsageGovernanceSummary` | 用量治理（写 settings 持久化 / 宿主 summary 通道） |
| `runBrowserPreviewTurn` / `cancelBrowserPreviewTurn` | 浏览器预览兜底（无宿主分支） |
| `stopTurn` / `enterStoppingPhase` | 停止链控制面（`stop_turn` / `stop_graph_run` 应答语义） |
| `retryLastTurn` | 续跑入口；判定侧已由 failed + `failedTurnRetry` 覆盖 |
| `setSessionNotice` | 斜杠命令中性反馈；getter info 分支已由快照覆盖 |

### 3.2 未覆盖 getter（5/18）

| getter | 原因 |
|---|---|
| `isSessionDeleting` / `isSessionRunning` | 返回函数的 getter，需 UI/列表侧按 sessionId 调用 |
| `isCostBudgetExceeded` / `costBudgetWarningMessage` | 依赖 `settings` store 的预算字段（本波不驱动 settings 持久化） |
| `governanceBlockDetail` | 依赖 `state.governanceDecision`（既有无类型扩展键，fact-map §8 要求原样保留） |

---

## 4. fact-map 与代码事实的差异（以代码为准，未改 fact-map）

1. **action 数 96 → 实际 97**：`performHistoryOperation`（`src/stores/runtime.ts:2073`）未在 fact-map §3 登记（protocol-agent 已确认修订为 97，归 `history.ts`）。
2. **`loadPersistedRuntimeCache()` 丢弃运行态映射**（`src/lib/runtime/sessions.ts:374-401` 只返回 `{ sessions }`）：`runningSessionMap` / `completedSet` / `failedSet` 写侧照写（`persistSessionStateAndRuntimeMaps`），读侧在 `initializeSessions`（`runtime.ts:2947-2958`）永远拿到空 ⇒ 三处恢复为**死路径**。已冻结为「恢复为空」行为（`initialize-sessions-restore.json`）。
3. **`buildRuntimeViewFromPersistedState()` 恒返回 `checkpoint: null`**（`sessions.ts:474`）：`deriveInitStrategy` 仍用 `persisted.checkpoint` 选策略，但恢复侧不重建 checkpoint 明细 ⇒ 冷启动 `latestExecutionCheckpoint === null`（已冻结）。
4. **`normalizeHistoryCheckoutResult` / `normalizeHistoryBranchSwitchResult` 不返回 `cursorVersion`**（`history.ts:188-215` / `273-292`）⇒ checkout / branch-switch 后 `state.cursorVersion` 被清空为 `null`（不是保留旧值），落盘同值。用例注释已标 `src/lib/runtime/history.ts:188-215`。
5. **`resetSessionRuntimeState` 不清 `latestExecutionCheckpoint`**（`runtime.ts:937-994` 与 `latestGraphRun*` / `latestHistoryStateAuditSummary` 不对称）⇒ 既有行为，已单独用例登记，Wave-1 不统一。

## 5. Wave-2 建议（不在本波改动）

1. 为差异 #2 补一条回归锁定后再决定「修复 or 删除死路径」（修则需 ADR + 迁移，不属等价重构）。
2. 差异 #4 若属缺陷，修复应显式返回 `cursorVersion: payload.cursor?.cursorVersion ?? null`，并同步更新 checkout/branch 的 fixtures（本波禁止）。
3. 补充覆盖缺口优先级：`stopTurn/enterStoppingPhase`（停止链，用户可见）→ `updatePersistedBackgroundSession`（后台轮）→ `addPendingAttachments`（附件边界）。
4. 变异自检可扩展为「属性级变异」：对 `persistHistory` 逐字段删除（本波只做 1 个字段），成本低但能直接定位到未覆盖字段。
5. 调度接缝（`waitForNextPaint` / `runLowPriorityTurnWork`）在测试内被替换为同步执行，使「空闲期投影」可确定性观察；若 Wave 2 变更该调度语义，需重新捕获 fixtures（注意这是**测试脚手架**而非生产语义变化）。
