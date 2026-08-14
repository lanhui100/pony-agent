# Design

## Decision Summary

1. 新增**非响应式投影层** `src/lib/runtime/trace-projection.ts`：`computeTurnTimeline` 归一化 + `turnTimeline` 签名化 memo（key = traceTimeline 引用 + updatedAt）+ `clearTraceProjectionMemo`。
2. store 的 15 条 `traceTimeline` 写路径**收敛为单一发布入口 `publishTraceTimeline()`**。
3. **实现边界裁决（2026-08-14 实现后审核）**：投影层定位为**轻量 memo helper**，而非 spec 原设计的完整 `TraceProjection`（sessionId/generation/orderedTurnIds/byTurnId/revision 对象）。理由：
   - 引用语义：活跃 turn 别名同数组（就地变更可见），memo 以 ref+updatedAt 签名自然失效——所有就地变更路径前都有新克隆赋值（引用已变），P1-3 场景在当前代码路径下不成立。
   - `sessions.ts:212`（snapshot restore）与 `runtime.ts:3226`（terminal $patch）两处直接写路径均为低频恢复路径，且赋值引用为新克隆（memo 自然失效），不构成一致性问题，不强制收敛。
   - 完整 projection 需要 9+ 条写路径全部语义收敛 + 版本管理，成本高于当前收益；memo helper 已解决"流式期间全量重算"的核心问题。

## Chosen Direction

### 1. 投影层（src/lib/runtime/trace-projection.ts）

```ts
// 非响应式模块层：不进入 Pinia state，不参与持久化
interface ProjectedTurn {
  turnId: string;
  title: string;
  phase: string;
  timeline: TraceTimelineEntry[];        // 活跃 turn：别名源数组；历史 turn：轻量派生
  toolActivities: ToolActivity[];         // 渲染所需（含 capabilityInvocation）
  buildContextObservation?: ...;          // 现状消费字段列全
  providerCallRecords?: ...;
  metrics: ...;                           // token/耗时预计算
  error?: string;                         // 预提取错误摘要
  updatedAt: number;
}

interface TraceProjection {
  sessionId: string;
  generation: number;                     // 会话切换 +1，throttle 不跨 generation
  orderedTurnIds: string[];               // 排序后的 id 列表（轻量，不持大对象）
  byTurnId: Map<string, ProjectedTurn>;
  revision: number;                       // 每次发布 +1（供签名比对，非渲染门控）
}
```

- `signature(turn)`: `(timeline 引用, updatedAt)` 签名——memo key。
- `memoizeTurnProjection(turn)`: 签名未变 → 返回缓存引用；变了 → 重建该 turn。

### 2. 单一发布入口（runtime.ts）

```ts
// 所有写 traceTimeline 的路径（9 处）收敛：
// - commitTurnTraceTimeline / updateActiveTraceTimeline / scheduleThrottledTraceTimeline
// - terminal 事件（completed/failed/cancelled）$patch
// - 恢复 / rollback / checkpoint load / 会话切换 / 过滤
// 全部最终调用：
function publishTraceProjection(sessionId: string, activeTurnId: string | null) {
  projectionLayer.rebuild(activeTurnId, sessionId);
}
```

- 9 条路径逐一映射到发布入口（diff 时逐条确认）。
- throttle 回调携带 `{sessionId, turnId, generation}`，执行时校验不跨代。

### 3. 组件消费

- `HomeSidebar`：状态计数链（sessionModelCallCount 等）改读投影层；`liveTraceTurn` 折叠态不 stamp（PA-084 已做）。
- `HomeTracePanel`：`turns` 渲染读投影行；turnTimelineCache 删除，改签名化 memo。
- `HomeWorkspace` / `WorkspaceTurnItem`：工具归因（traceSequencesForToolMessages / modelTraceEntries）读投影行，保留 `toolActivities/capabilityInvocation` 字段。

### 4. 节流参数

- 基线 200ms 不动；实施后用手动/自动化测量记录证据，调整则写回任务卡验证部分。

### 5. 与 PA-085 协调

- 085 的 `projectTraceRows` 消费投影行（`ProjectedTurn`），数据结构对齐：快照先行（本卡），虚拟化消费投影。

## Edge Cases

- 会话切换：`generation++`、投影原子清空重建；pending throttle 校验 generation 丢弃过期更新。
- 活跃 turn 变更：`byTurnId` 中旧活跃 turn 转历史（派生），新活跃 turn 别名源数组。
- turn 删除/裁剪：立即从 `orderedTurnIds`/`byTurnId` 逐出。
- 就地变更（updateActiveModelTraceFromAssistant）：活跃 turn 别名同数组，无需重建投影。
- 折叠态（PA-084 后）：组件未挂载，投影层仍维护（模块层成本低），展开时直接消费。
- 浏览器模式：数据源一致，无特殊处理。
- 无投影回退：投影层缺失时组件回退读原始 store 数据（独立回滚）。