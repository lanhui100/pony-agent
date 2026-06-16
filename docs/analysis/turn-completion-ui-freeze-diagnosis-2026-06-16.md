# Turn 完成时 UI 冻结：根因诊断与解决路径

> 日期：2026-06-16
> 状态：诊断完成，待埋点证实
> 关联任务卡：PA-055（待创建）

---

## 1. 问题陈述

每个 agent turn 完成时（`turn:completed` / `turn:failed` / `turn:cancelled` 事件到达前端后），UI 会冻结数秒：

- **无法滚动**主对话区域
- **无法在输入框输入**新内容
- 冻结持续约 2~5 秒，取决于会话历史长度和工具调用复杂度

---

## 2. 问题边界

### 排除项

| 组件 | 结论 | 依据 |
|---|---|---|
| `HomeWorkspace.vue`（主对话区） | ❌ 不是元凶 | 已做 idle 调度、自然边界节流、跳过最终重渲 |
| `MarkdownRenderer.vue` | ❌ 不是元凶 | `requestIdleCallback` + 自然边界门控 + streaming→non-streaming 跳过逻辑完善 |
| Rust 后端事件发射 | ❌ 不是元凶 | Tauri IPC 是异步的，事件到达前端本身不阻塞主线程 |
| DOM 渲染量（消息条数） | ❌ 不是元凶 | 消息列表用 `TransitionGroup` + `v-motion`，每条消息渲染量可控 |

### 确认项

| 组件 | 结论 | 依据 |
|---|---|---|
| `src/stores/runtime.ts` turn 完成链路 | ✅ 首要阻塞源 | STAGE 2 同步段执行了大量深拷贝与序列化 |
| `localStorage` 全量持久化 | ✅ 次要阻塞源 | `persistSessionState` 每次全量 parse + stringify |
| `HomeSidebar.vue` trace 面板响应式重算 | ✅ 放大器 | `orderedTurnTraces` 全量响应式遍历，被 store 突变更触发 |

---

## 3. 根因分析

### 3.1 热点一：STAGE 2 `setTimeout(0)` 中的同步重活（首要阻塞）

**位置**：`src/stores/runtime.ts:4994-5104`

**时序模型**：

```
turn:completed 事件到达
  │
  ├─ STAGE 1（同步，同宏任务）
  │    commitTurnEventCursor + flushBufferedStreamText + ensureAssistantMessage
  │    → 设置 assistant content/status = "done"
  │
  ├─ setTimeout(0) ← 注释说"Yield to browser"
  │    │
  │    ├─ STAGE 2（同步，下一个宏任务）
  │    │    ├─ 15 个响应式赋值（phase, toolActivities, provider*, token* ...）
  │    │    ├─ commitTurnTraceTimeline()
  │    │    │    ├─ cloneTraceTimeline()         ← 重新跑折叠算法
  │    │    │    ├─ upsertTurnTrace()
  │    │    │    │    └─ normalizeTurnTraceRecord() ← 递归深拷贝全量 trace
  │    │    │    └─ buildCacheTelemetryDebugSnapshot() ← 被调 2 次
  │    │    ├─ applyTurnTokenStats + syncToolMessages
  │    │    └─ isSubmitting = false, activeTurnId = null
  │    │
  │    └─ STAGE 3（runLowPriorityTurnWork, 800ms+idle 延迟）
  │         scheduleDeferredPersist + loadSessionCatalog + loadRetrievedContext
  │
  └─ Vue 响应式批处理触发 → 一次大 patch → 主线程阻塞数帧
```

**关键问题**：

`setTimeout(0)` 只把任务挪到下一个宏任务队列，**不保证让出渲染帧**。如果 STAGE 2 的同步执行时间超过 16ms（一个 frame），页面就开始丢帧。实际执行中，STAGE 2 包含的深拷贝和 telemetry 计算通常需要 50~500ms，取决于 trace 数据量。

**具体阻塞点**：

| 函数 | 位置 | 问题 |
|---|---|---|
| `cloneTraceTimeline()` | `:379-425` | 遍历 timeline 数组做折叠，每个 entry 做 spread 拷贝；`return_result` 合并逻辑用 `[...folded].reverse().findIndex()` 在循环中反复建临时数组 |
| `normalizeTurnTraceRecord()` | `:1249-1259` | 递归调用 `cloneTraceTimeline` + `cloneToolActivities` + `cloneProviderCallRecords` + `cloneHookTraceRecords`，对每个子数组做 `.map(obj => ({...obj}))` |
| `buildCacheTelemetryDebugSnapshot()` | `:1424-1499` | 对 payload 做 12+ 条 `readNestedNumericTokenValue` 路径扫描（每条路径 2~6 层嵌套），并在 `commitTurnTraceTimeline` 内部和调用方各算一次（共 2 次） |
| `upsertTurnTrace()` | `:4238-4296` | 对已有 turn 做 `Object.assign` + 条件 clone，对新建 turn 做全量 `normalizeTurnTraceRecord` |

### 3.2 热点二：`localStorage` 全量序列化（次要阻塞）

**位置**：`src/stores/runtime.ts:2791-2798`

```typescript
function persistSessionState(sessionId: string, payload: PersistedRuntimeState) {
  const cache = loadPersistedRuntimeCache();           // JSON.parse 全量缓存
  cache.sessions[sessionId] = payload;
  window.localStorage.setItem(RUNTIME_STORAGE_KEY,     // JSON.stringify 全量缓存
    JSON.stringify(cache));
}
```

**问题**：

1. **全量读写**：每次持久化都 parse 整个缓存（所有 session 的消息 + trace），再 stringify 写回
2. **二次膨胀**：缓存体积 ∝ session 数 × 消息数 × trace 深度。一个包含 20 个 turn、每个 turn 有 5 个 tool call 的会话，trace 部分序列化后可达数 MB
3. **触发频繁**：`persistHistory()` 在以下位置被调用：
   - `output_end` → `scheduleDeferredPersist(1200ms)` (`:4427`)
   - `completed` STAGE 3 → `scheduleDeferredPersist()` (`:5074`)
   - `failed` STAGE 3 → `persistHistory()` 直接 (`:5228`)
   - `cancelled` STAGE 3 → `persistHistory()` 直接 (`:5361`)
   - `upsertTurnTrace` (persist=true) (`:4259`, `:4294`)
4. **同步阻塞**：即使 deferred 到 `setTimeout`，到期后 `JSON.stringify` 仍在主线程同步执行

### 3.3 热点三：Trace 面板全量响应式重算（放大器）

**位置**：`src/components/HomeSidebar.vue`

| 代码点 | 行号 | 问题 |
|---|---|---|
| `orderedTurnTraces` computed | `:187-200` | `[...turnTraceHistory.value]` 展开全量历史 turn |
| `orderedTurnTraceSignature` computed | `:1372-1376` | 每次变更 join 一条长字符串，触发 watch |
| `orderedTurnTraceSignature` watch | `:1378-1385` | 匹配 activeTurnId |
| 模板 `v-for="turn in orderedTurnTraces"` | `:1587` | 遍历所有 turn |
| `turnTimeline(turn)` | `:259-307` | 每个 turn 重新跑折叠算法（与 store 的折叠重复） |
| `buildTurnMetricItems` / `buildTimelineRows` / `buildTimelineDetailSections` | 模板内 | 每个 turn 每个 timeline entry 重复计算 |

**放大机制**：当 store 的 `turnTraceHistory`（深层响应式数组）被 STAGE 2 整体替换时，Vue 的依赖追踪会让所有消费该数据的 computed/watch 重新求值。如果历史中有 N 个 turn，`orderedTurnTraces` 重建 O(N)，每个 turn 的 `turnTimeline()` 再做 O(M) 折叠。

---

## 4. 为什么主对话区"看起来卡住"

主对话区和 trace 处理**共享同一个 JS 主线程**。浏览器的渲染管线（layout → paint → composite）和事件处理（scroll、input）都在主线程上排队。

当 turn 完成时：

```
[事件循环]
  └─ 宏任务：turn:completed handler
       ├─ STAGE 1（同步）     ← ~1ms，可忽略
       └─ setTimeout(0) 注册
  └─ 宏任务：浏览器渲染帧
  └─ 宏任务：STAGE 2 回调      ← 50~500ms 同步重活
       │                         ↑ 这段时间内：
       ├─ commitTurnTraceTimeline    · 滚动事件排队等待
       ├─ persistHistory（deferred） · 输入事件排队等待
       └─ Vue 响应式 patch           · 渲染帧被推迟
  └─ 宏任务：STAGE 3（800ms 后） ← runLowPriorityTurnWork
  └─ 宏任务：persistHistory 到期 ← JSON.stringify 再吃几百 ms
```

用户感知到的"冻结"就是 STAGE 2 + persist 到期这两段同步执行的叠加。

---

## 5. 解决路径

### 阶段一：埋点证实（本轮，零行为变更）

在 turn 完成链路的每个同步段前后取 `performance.now()`，全部走已有的 `debugLog` 输出到 console。

| 埋点 ID | 位置 | 测量目标 |
|---|---|---|
| `perf:stage1-sync` | `turn:completed` STAGE1 入口/出口 | flushBufferedStreamText + ensureAssistantMessage 耗时 |
| `perf:stage2-reactive` | STAGE2 setTimeout 回调内赋值段前/后 | 15 个响应式赋值耗时 |
| `perf:stage2-commit-trace` | `commitTurnTraceTimeline` 入口/出口 | cloneTraceTimeline + upsertTurnTrace 耗时 |
| `perf:stage2-telemetry-snapshot` | `buildCacheTelemetryDebugSnapshot` 入口/出口 | 单次耗时 + 输入 traceTimeline 条目数 |
| `perf:persist-parse` | `loadPersistedRuntimeCache` 内部 | JSON.parse 耗时 |
| `perf:persist-stringify` | `persistSessionState` 内部 | JSON.stringify 耗时 + 输出 KB 数 + 缓存中 session 数 |
| `perf:persist-total` | `persistHistory` 入口/出口 | 包含 clone + 序列化的总耗时 |
| `perf:sidebar-ordered-traces` | `orderedTurnTraces` computed 重算 | 触发次数 + turnTraceHistory 总条目数 |

**产出**：在 console 中形成如下格式的日志组：

```
[pony-agent][runtime] { event: "perf:stage2-commit-trace", payload: { durationMs: 187, traceEntries: 8, turnTraceHistoryLength: 15 }, ts: "..." }
```

### 阶段二：精准优化（埋点数据回来后，按实测权重排序）

| 方向 | 预期收益 | 风险 |
|---|---|---|
| **去重 `buildCacheTelemetryDebugSnapshot`**：当前在 `commitTurnTraceTimeline` 内部和外部各调一次，只需在出口调一次 | 减少 ~30% STAGE2 耗时 | 零风险 |
| **拆 idle**：把 STAGE2 里的 `commitTurnTraceTimeline` 改为 `requestIdleCallback` 分片 | 消除主线程长任务 | 低风险，需注意 trace 数据一致性 |
| **分 key 持久化**：按 sessionId 拆 localStorage key，只写当前 session | 减少 ~90% 序列化量 | 低风险 |
| **trace 深层子结构浅拷贝**：`turnTraceHistory` 中非活跃 turn 的 traceTimeline/toolActivities 改为 shallow ref | 减少 Vue 响应式追踪深度 | 中风险，需确保 trace 面板读取不报错 |
| **Tauri 后端持久化**：前端 localStorage 只留极简 UI 状态，会话历史交 Rust 侧存储 | 彻底消除前端序列化瓶颈 | 需新增后端 API |

### 阶段三：Tauri 后端持久化（长期方向）

将 `persistSessionState` / `loadPersistedRuntimeState` / `loadPersistedRuntimeCache` 的职责迁移到 Tauri 侧：

- 新增 `persist_session_runtime_state` / `load_session_runtime_state` Tauri command
- 前端 `localStorage` 只保留：`draftMessage`、UI 折叠状态、`showReasoningContent` 等 UI-only 状态
- turn trace history 的读写走 Rust 侧的 session store（已有 `list_sessions` 等 API）
- 序列化在 Rust 侧异步执行，不阻塞 WebView 主线程

---

## 6. 关键代码位置索引

| 文件 | 行号 | 角色 |
|---|---|---|
| `src/stores/runtime.ts` | `:4994-5104` | `turn:completed` 三阶段处理（STAGE 1/2/3） |
| `src/stores/runtime.ts` | `:1275-1295` | `runLowPriorityTurnWork`（STAGE 3 调度器） |
| `src/stores/runtime.ts` | `:4322-4352` | `commitTurnTraceTimeline`（trace 写入 + telemetry snapshot） |
| `src/stores/runtime.ts` | `:4238-4296` | `upsertTurnTrace`（upsert + deep clone） |
| `src/stores/runtime.ts` | `:1249-1259` | `normalizeTurnTraceRecord`（递归深拷贝） |
| `src/stores/runtime.ts` | `:379-425` | `cloneTraceTimeline`（折叠算法 + spread 拷贝） |
| `src/stores/runtime.ts` | `:1424-1499` | `buildCacheTelemetryDebugSnapshot`（嵌套数值扫描） |
| `src/stores/runtime.ts` | `:2791-2798` | `persistSessionState`（全量 localStorage 读写） |
| `src/stores/runtime.ts` | `:2762-2785` | `loadPersistedRuntimeCache`（全量 JSON.parse） |
| `src/stores/runtime.ts` | `:3410-3416` | `scheduleDeferredPersist`（延迟持久化调度） |
| `src/stores/runtime.ts` | `:3417-3458` | `persistHistory`（持久化入口） |
| `src/components/HomeSidebar.vue` | `:187-200` | `orderedTurnTraces` computed |
| `src/components/HomeSidebar.vue` | `:1372-1385` | `orderedTurnTraceSignature` + watch |
| `src/components/HomeSidebar.vue` | `:259-307` | `turnTimeline()` 每渲染重算折叠 |
| `src/components/HomeSidebar.vue` | `:1587` | `v-for="turn in orderedTurnTraces"` |
| `src/components/HomeWorkspace.vue` | `:270-293` | `turns` computed（消息归组） |
| `src/components/HomeWorkspace.vue` | `:946-958` | `latestTurnSignature` watch + 自动滚动 |
| `src/components/MarkdownRenderer.vue` | `:77-128` | idle 调度的 markdown 渲染（非阻塞，对比参考） |

---

## 7. 与主对话区优先级的约束

> "主对话区域是第一优先级，必须保证其流畅。"

这意味着任何优化方案都必须满足：

1. **STAGE 1 必须保持同步且极轻量**：确保 `assistantMessage.content` 和 `assistantMessage.status = "done"` 在第一个微任务内完成，让 MarkdownRenderer 能立即开始最终渲染
2. **STAGE 2 的 trace/metadata 处理不得阻塞渲染帧**：如果单次同步执行超过 16ms，必须拆成 `requestIdleCallback` 分片
3. **STAGE 3 的 persist 不得与 STAGE 2 同帧执行**：当前 `scheduleDeferredPersist(140ms)` 的延迟太短，容易与 STAGE 2 的 Vue patch 冲突
4. **trace 面板的 computed 重算不得拖慢对话区**：如果 `orderedTurnTraces` 的重算耗时显著，考虑把 trace 面板改为 lazy computation（只在面板可见时计算）
