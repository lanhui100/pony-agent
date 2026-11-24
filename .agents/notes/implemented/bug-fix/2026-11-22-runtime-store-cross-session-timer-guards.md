# Agent Note: runtime store 跨会话定时器守卫

Status: implemented

## Problem

runtime store 从单文件拆成 10 个域模块（B271）后，所有延迟执行点都变成了「定时器 + 共享可变 state」的组合，
但定时器回调没有携带归属信息。结果是三类跨会话串台路径：

1. `telemetry.ts` 的 trace 节流（`TRACE_TIMELINE_THROTTLE_MS = 200ms`）回调直接
   `updateActiveTraceTimeline(pending)`，不校验 `sessionId` / `activeTurnId`。切会话后旧 pending 落到新会话的活跃轮。
2. `orchestration.ts` 的 `switchSession` 只把运行中轮的缓冲存进 `runningSessionMap`，**不 cancel** stream flush 定时器，
   也不落定 pending trace。切走后的 `setTimeout` 把旧会话增量 `ensureAssistantMessage` 进新会话消息列表。
3. `applySessionSnapshot`（会话落点）只写 `sessionId`，不清旧会话定时器；PA-089 的 rIC 延迟 timeline 恢复
   （`runLowPriorityTurnWork`）同样无归属校验，空闲回调晚于下一次切换会覆盖新会话 timeline。

同域还有一处纯死码：`streamFlushFrameId` 只有 `cancelAnimationFrame` 读取，全仓无 `requestAnimationFrame` 赋值点
（`rg -n "requestAnimationFrame" src/` 零命中），却占着一个 state 键并出现在 13 个 golden fixture 里。

## Decision

四处延迟执行点统一采用「发起时捕获归属、回调时校验」：

1. `scheduleThrottledTraceTimeline` 捕获 `ownerSessionId` + `ownerTurnId`，回调不匹配即丢弃本次投递
   （pending 留给新归属的下一次调度重建定时器，不清空以免误删新数据）。
2. `switchSession` 在 `persistHistory()` 之前 `flushPendingTraceTimeline()` → `cancelStreamFlush()` →
   清 `streamBufferTurnId/Text/Reasoning`（运行中轮缓冲已在 `runningSessionMap` 留存）。
3. `applySessionSnapshot` 仅在 `sessionId` 真正变化时 `cancelStreamFlush()` + `clearPendingTraceTimeline()`；
   同会话刷新不打断在飞 flush。缓冲不在落点清空，由 `switchSession` 的 bgTurn 恢复分支负责重建。
4. rIC 延迟 timeline 恢复回调校验 `this.sessionId === ownerSessionId`。
5. 删除 `streamFlushFrameId` 死字段及其分支（`state.ts` / `types.ts` / `lib/runtime/{types,sessions}.ts` /
   harness / 两个契约测试 / 13 个 fixture），state 键 84 → 83，`FrozenStateKeys` 与两处测试字面量同步。

回归判据：`src/stores/runtime/__tests__/timer-session-guard.spec.ts`（4 条，假定时器 + 固定时钟），
覆盖 trace 节流、stream flush、会话落点清理、rIC 晚到回调四个场景；`npx vitest run src/stores/runtime` 全绿。

## Alternatives considered

1. **只在切换处 cancel，回调不加守卫**：切走路径漏一条（例如 host 事件直接改 `sessionId`）就复发，
   且无法覆盖同一会话内的轮次更替。守卫放在回调内是唯一与调用路径解耦的形态。
2. **新增 `sessionEpoch` state 字段做全局版本号**：能统一守卫，但要再加一个 state 键并改全部 fixture，
   而 `sessionId + activeTurnId` 已经是现成的归属真源，引入第二个版本号等于制造第二事实源。
3. **保留 `streamFlushFrameId`、只加注释说明是预留**：与「恢复条件立法、拒绝怀旧」相悖；
   真要启用 rAF 时应作为新决策重新引入，而不是留一个永远为 null 的字段误导读者。
4. **在 `applySessionSnapshot` 清空流式缓冲**：会与 `switchSession` 尾部的 bgTurn 恢复竞态
   （异步 `loadSessionState` 可能晚于同步恢复分支执行），清掉刚恢复的后台轮缓冲。

## Consequences

- 切会话不再出现「旧会话半句话出现在新会话」「timeline 被上一个会话覆盖」两类串台；
  同会话刷新（`preserveMessages`）行为不变。
- state 键从 84 降到 83，`FrozenStateKeys` 与两个契约测试的冻结面同步收窄；
  `.dev-team/runtime-split/fact-map.md` 保留原测绘正文，仅在顶部追加 84 → 83 的修订注记。
- 遗留：会话状态恢复仍有两个家（store action + `src/lib/runtime/sessions.ts`），已登记于
  `.dev-team/runtime-split/contract-residuals.md` R8，合并需另立决策。
