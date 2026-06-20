# Design: Frontend Flight Recorder And Stall Diagnostics

## 背景

当前前端观测分散在以下几类机制中：

- `src/stores/runtime.ts` 中的 `debugLog(event, payload)`
- `src/stores/runtime.ts` 和 `src/components/HomeWorkspace.vue` 中的 `__ponyStreamMetrics`
- `src-tauri/src/lib.rs` 中的 `record_stream_debug_metrics / load_stream_debug_metrics`

这些机制有价值，但它们有三个结构性限制：

1. 缺乏统一事件模型
2. 缺乏时间序列与卡顿前后窗口回放能力
3. 缺乏面向 stall 的自动“冻结现场”抓取逻辑

因此本 change 不再继续扩展 latest bucket，而是引入正式的 `Frontend Flight Recorder`。

## 设计目标

1. 以低开销方式持续记录前端关键事件
2. 在主线程发生 stall 时自动记录冻结现场
3. 允许按 `sessionId / turnId / 时间窗口` 查询与导出
4. 让诊断体系脱离浏览器 console 和临时 WebView localStorage
5. 保留现有 `debugLog` 和 lightweight debug bucket 的开发便利，但将其降级为辅助层

## 非目标

- 不在本 change 中直接修复所有前端卡顿根因
- 不要求一次性覆盖所有组件和所有响应式链路
- 不把性能分析能力建立在浏览器自动化、DevTools Performance recording 或人工操作之上
- 不将大块 Markdown、完整 store 深拷贝或完整 DOM HTML 作为常规日志负载

## 总体架构

方案分为四层：

1. `Frontend Recorder API`
2. `Stall Detector`
3. `Freeze Snapshot`
4. `Tauri Persistence + Query/Export`

### 1. Frontend Recorder API

前端统一通过一个 recorder 模块上报事件，避免各组件直接零散 `console.info`。

建议事件模型：

```ts
type FrontendTraceEvent = {
  seq: number;
  tsWallMs: number;
  tsPerfMs: number;
  sessionId: string | null;
  turnId: string | null;
  category: string;
  name: string;
  kind: "span" | "instant" | "counter" | "sample" | "stall" | "snapshot";
  durationMs?: number;
  data?: Record<string, string | number | boolean | null>;
};
```

其中：

- `span` 用于耗时区间，例如 `turn-completed-stage2`
- `instant` 用于单点事件，例如 `turn:completed-received`
- `counter` 用于规模快照，例如 `messagesCount`
- `sample` 用于周期采样
- `stall` 用于明确记录检测到的主线程卡顿
- `snapshot` 用于卡顿现场或关键边界现场的轻量状态快照

补充约束：

- `tsPerfMs` 明确定义为 `performance.now()` 相对时间戳，用于同一 WebView 生命周期内的高精度排序
- `tsWallMs` 用于跨重启、跨导出窗口对齐；Chrome Trace 导出时 SHALL 转为微秒
- `seq` 定义为单 session 单调递增序列号；初始化前产生的事件可先进入 pre-session 缓冲，待 `sessionId` 可用后补绑定
- `sessionId` 或 `turnId` 在初始化阶段允许为 `null`，但事件必须显式标记 `scope=pre-session` 或 `scope=global`
- `category` SHALL 直接映射到 Chrome Trace/Perfetto 的 `cat`

#### 1.1 Recorder Lifecycle

recorder 需要定义清晰生命周期，避免真正实现时各组件各自初始化：

1. `init`
   - 在前端 runtime 基础上下文可用后启动
   - 建立 ring buffer、stall detector、flush timer 与 capability probe
2. `bindSession`
   - 在 `sessionId` 可用后绑定当前 session
   - 将 pre-session 缓冲事件补写为正式 session 事件
3. `bindTurn`
   - 在 turn 开始/切换时绑定 `turnId`
4. `flushAndStop`
   - 在页面关闭、应用退出或显式调试导出前触发一次有超时保护的 flush

#### 1.2 Recorder Config

以下参数建议作为显式配置项，而不是硬编码散落在实现中：

- `stallThresholdLightMs`，默认 `250`
- `stallThresholdMediumMs`，默认 `500`
- `stallThresholdHeavyMs`，默认 `1500`
- `timerDriftTickMs`，默认 `100`
- `flushIntervalMs`，默认 `500`
- `ringBufferCapacity`，默认 `2000`
- `snapshotCooldownMs`
- `maxExportBytes`

### 2. Stall Detector

stall 检测不能依赖单一浏览器 API，建议三条路径并存：

1. `requestAnimationFrame` gap
2. `setTimeout` drift
3. `PerformanceObserver(longtask)`，仅在环境支持时启用

#### 2.1 raf gap

维护连续帧时间戳：

- `gap >= 250ms` 记录轻度 stall
- `gap >= 500ms` 记录中度 stall
- `gap >= 1500ms` 记录重度 stall，并触发强制 flush

#### 2.2 timer drift

每 `100ms` 运行一个轻量 tick，计算理论时间与实际时间偏差：

- 若偏差显著超过阈值，补记 event loop blocked 证据

#### 2.3 longtask

若 WebView 支持 `PerformanceObserver({ entryTypes: ["longtask"] })`：

- 作为额外证据采集
- 不应成为唯一 stall 触发条件

### 3. Freeze Snapshot

检测到 stall 时，立即抓取“冻结现场”，但仅限轻量结构化数据，禁止：

- 深拷贝整个 Pinia store
- 复制完整 message 文本
- 复制完整 DOM 树

建议快照字段：

- `sessionId`
- `turnId`
- `phase`
- `isSubmitting`
- `activeTurnId`
- `messagesCount`
- `assistantMessageCount`
- `toolMessageCount`
- `pendingAssistantCount`
- `traceTimelineLength`
- `turnTraceHistoryLength`
- `latestTurnSignatureLength`
- `scrollQueued`
- `streamAutoFollowEnabled`
- `renderPendingCount`
- `openDisclosureCount`
- `domNodeCount`
- `visibleTurnCount`
- `heapUsed` / `usedJSHeapSize`（若环境可用）

快照应携带触发来源：

- `raf-gap`
- `timer-drift`
- `longtask`
- `manual-span-threshold`

### 4. Tauri Persistence + Query/Export

诊断数据的权威落点定义为 Tauri 侧持久化，不再依赖 localStorage。

推荐新增两张 SQLite 表：

#### 4.1 `frontend_trace_event`

- `id`
- `session_id`
- `turn_id`
- `ts_wall_ms`
- `ts_perf_ms`
- `seq`
- `category`
- `name`
- `kind`
- `duration_ms`
- `data_json`
- 索引：`INDEX idx_frontend_trace_session_ts (session_id, ts_wall_ms, seq)`
- 索引：`INDEX idx_frontend_trace_turn_ts (turn_id, ts_wall_ms, seq)`

#### 4.2 `frontend_stall_snapshot`

- `id`
- `session_id`
- `turn_id`
- `ts_wall_ms`
- `stall_level`
- `trigger_kind`
- `stall_gap_ms`
- `snapshot_json`
- 索引：`INDEX idx_frontend_stall_session_ts (session_id, ts_wall_ms DESC)`

#### 4.2.1 SQLite 连接策略

为避免诊断链路本身制造 `SQLITE_BUSY`：

- Tauri 侧应复用单一 writer 连接并启用 WAL 模式
- 查询可使用独立 reader，但必须共享同一数据库文件与兼容的 busy timeout 策略
- 不应混用多套彼此未知的 SQLite 访问层同时写同一诊断数据库
- 所有写入命令必须有超时、错误码与 dropped-count 统计

### 4.3 Query API

最小查询能力建议包括：

- `append_frontend_trace_events(events)`
- `query_frontend_trace_window(sessionId, fromMs, toMs, turnId?, limit?, cursor?)`
- `query_frontend_stall_snapshots(sessionId, limit)`
- `clear_frontend_trace_before(tsWallMs)` 或 retention 机制

查询默认约束：

- 默认 `ORDER BY ts_wall_ms ASC, seq ASC`
- 默认分页，避免一次拉回整个 session
- 支持按 `turnId` 进一步收窄窗口
- 若导出窗口超限，返回显式截断标记而不是静默丢尾

### 4.4 Export API

建议至少支持：

- `export_frontend_trace_json(sessionId, fromMs, toMs)`
- `export_frontend_trace_chrome_trace(sessionId, fromMs, toMs)`

Chrome Trace/Perfetto 导出可让一次卡顿的前后窗口直接可视化。

补充约束：

- Chrome Trace 导出使用微秒 `ts`
- 导出实现需要定义文件体积上限；超过上限时分页导出或显式截断
- Phase 1 先保证 JSON 导出与可验证的 Chrome Trace 结构；诊断入口 UI 可在后续阶段补齐

## Ring Buffer 与 Flush 策略

前端不应每条事件立即跨桥写入 Tauri，避免放大开销。

建议：

- 前端内存 ring buffer：默认保留最近 `N=2000` 条事件，并允许通过配置覆盖
- 批量 flush：
  - 正常情况下每 `500ms` 左右批量 flush
  - 遇到重度 stall 或关键终态事件时强制 flush
- 限流：
  - 高频 sample/counter 事件做 coalescing
  - 同类 snapshot 不在极短时间窗口重复上报

失败降级与背压策略：

- flush 失败不得阻塞前端主流程
- recorder 优先采用“有限次重试 + 丢弃最旧事件 + 记录 dropped count”的降级链
- ring buffer 满时采用 FIFO 丢弃最旧事件，而不是阻塞 producer
- 强制 flush 仍需带超时保护，并记录 flush duration 以便判断诊断链路自身是否加重恢复延迟
- 若主线程正处于重度阻塞，强制 flush 的实现应假设“只能在恢复后立即补刷”，不能把 flush 能力本身作为 stall 检测成功的前提

## 与现有 localStorage 的职责边界

本方案必须显式区分：

- `localStorage`
  - UI 偏好
  - 轻量前端恢复缓存
  - 调试开关
- `SQLite / Tauri`
  - 正式诊断 trace
  - stall 现场快照
  - 事后查询/导出

这一区分必须写成正式边界，避免再次把诊断链路放回 WebView localStorage。

## 首批必须埋点链路

第一批不求全覆盖，但必须覆盖当前最相关热点：

### Runtime turn 完成链路

- `turn:output_end`
- `turn:completed` STAGE 1
- `turn:completed` STAGE 2
- `turn:completed` STAGE 3
- `applyTurnTokenStats`
- `syncToolMessages`
- `resolveEventTraceTimeline`
- `commitTurnTraceTimeline`

### HomeWorkspace 展示链路

- `latestTurnSignature` 计算
- `turns` computed 重建
- `syncStreamingPresentationState`
- `queueScrollToLatestTurn`
- `handleTimelineViewportScroll`

### Markdown 渲染链路

- `MarkdownRenderer.handleContentChange`
- `scheduleStreamingRender`
- `scheduleNonStreamingRender`
- `executeRender`

## 观测开销控制

该方案必须遵守以下约束：

- recorder 默认低开销启用，不要求用户先开浏览器开发者工具
- 高频链路优先记录数字、计数和长度，不记录大文本内容
- snapshot 不得包含 message 全文和完整 HTML
- 任何序列化体积过大的记录必须截断并显式标注 `truncated`
- 重度 stall 触发前后应优先保证数据落盘，而不是继续堆积内存

## Migration Path

分三段落地，避免首期范围过大：

### Phase 1: Core recorder pipeline

- 建统一 recorder API
- 建 stall detector
- 建 ring buffer + 批量 flush
- Tauri 侧落 SQLite
- 提供最小查询命令
- 提供 JSON 导出
- 打通索引、WAL、retention、flush 失败降级

### Phase 1.5: Replay-ready export and validation

- 补 Chrome Trace/Perfetto 导出
- 补人工造 stall 的自动化 smoke 路径
- 补 dropped count、flush duration、longtask availability 等自观测字段

### Phase 2: Critical path instrumentation and operator UX

- 给 `runtime.ts`、`HomeWorkspace.vue`、`MarkdownRenderer.vue` 首批热点埋点
- 增加 stall snapshot 字段
- 增加最小本地诊断入口，例如导出按钮或 debug overlay

## 验证策略

至少覆盖：

- recorder 在高频事件下不会无限增长
- stall detector 能在人工阻塞主线程测试中记录事件
- flush 失败不影响前端主流程
- Tauri 查询能按 sessionId / turnId 拉回事件窗口
- Chrome Trace 导出结构合法
- 关键链路埋点在单测/定向测试下可观察到

建议补充：

- 增加 capability probe：记录 longtask 是否可用
- 增加可复现的自动化 stall 注入方案，而不只依赖手工制造卡顿
- 增加 flush duration、dropped event count、export truncation 等 recorder 自观测校验
