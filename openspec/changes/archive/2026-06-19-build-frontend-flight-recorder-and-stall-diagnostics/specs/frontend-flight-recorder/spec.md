## ADDED Requirements

### Requirement: Unified Frontend Trace Event Model

Pony Agent SHALL 使用统一的前端 trace event 模型记录关键前端事件。

#### Scenario: Recorder emits structured events

- **WHEN** 前端 recorder 记录一次事件
- **THEN** 该事件 SHALL 至少包含 `seq`、`tsWallMs`、`tsPerfMs`、`sessionId`、`turnId`、`category`、`name`、`kind`
- **AND** `kind` SHALL 属于 `span`、`instant`、`counter`、`sample`、`stall`、`snapshot`
- **AND** `tsPerfMs` SHALL 表示当前 WebView 生命周期内的高精度相对时间
- **AND** `category` SHALL 可直接映射到 Chrome Trace/Perfetto 的 `cat`

#### Scenario: Recorder binds late session context safely

- **WHEN** recorder 早于 `sessionId` 初始化开始记录事件
- **THEN** 系统 SHALL 允许事件暂存于 pre-session 缓冲
- **AND** SHALL 在 `sessionId` 可用后补绑定并落盘

### Requirement: Low-Overhead Event Recording

Pony Agent SHALL 以低开销方式记录前端事件，而不是将每条事件同步写入宿主。

#### Scenario: Recorder buffers and flushes in batches

- **WHEN** 前端产生高频 trace 事件
- **THEN** recorder SHALL 先写入内存 ring buffer
- **AND** SHALL 使用批量 flush 将事件送往 Tauri
- **AND** SHALL 支持 coalescing、限流或截断以控制负载

#### Scenario: Flush failure does not backpressure the UI

- **WHEN** 一次批量 flush 失败、超时或宿主持久化层繁忙
- **THEN** recorder SHALL NOT 阻塞前端主流程
- **AND** SHALL 采用有限重试、丢弃最旧事件或 dropped-count 标记等受控降级策略

### Requirement: Stall Detection Without Single-API Dependency

Pony Agent SHALL 支持不依赖单一浏览器专有 API 的 stall 检测。

#### Scenario: Raf gap detects a stall

- **WHEN** 前端连续两帧之间的 `requestAnimationFrame` 时间间隔超过定义阈值
- **THEN** recorder SHALL 记录一条 `stall` 事件

#### Scenario: Timer drift supplements stall evidence

- **WHEN** 周期 timer 的理论触发时间与实际触发时间产生显著漂移
- **THEN** recorder SHALL 记录 event loop blocked 的补充证据

#### Scenario: Longtask is available

- **WHEN** 运行环境支持 `PerformanceObserver(longtask)`
- **THEN** Pony Agent MAY 记录额外 longtask 证据
- **BUT** stall 检测 SHALL NOT 仅依赖该能力

#### Scenario: Longtask capability is unknown at startup

- **WHEN** recorder 初始化 stall detector
- **THEN** 系统 SHALL 记录 longtask 能力是否可用

### Requirement: Freeze Snapshot On Stall

Pony Agent SHALL 在检测到 stall 时记录轻量冻结现场。

#### Scenario: Freeze snapshot captures bounded context

- **WHEN** stall detector 触发卡顿记录
- **THEN** Pony Agent SHALL 同时记录一份 stall snapshot
- **AND** snapshot SHALL 包含当前 `sessionId`、`turnId`、`phase`、消息规模、trace 规模、渲染/滚动相关状态等轻量字段
- **AND** snapshot SHALL NOT 包含完整 message 文本、完整 HTML 或完整 store 深拷贝

### Requirement: Tauri Persistence As Diagnostic Source Of Truth

Pony Agent SHALL 使用 Tauri 宿主持久化前端诊断事件与 stall 快照。

#### Scenario: Frontend trace events are persisted outside localStorage

- **WHEN** 前端 recorder flush 事件
- **THEN** 事件 SHALL 写入 Tauri 宿主的正式持久化层
- **AND** 该持久化层 SHALL NOT 以 WebView localStorage 作为权威诊断来源

#### Scenario: Persistence remains queryable at scale

- **WHEN** 诊断事件持续累积
- **THEN** 持久化层 SHALL 为 `sessionId / turnId / tsWallMs` 主查询路径提供索引、分页或等效性能保护
- **AND** SHALL 定义 retention 或清理策略以防止诊断数据无限增长

### Requirement: Query By Session Turn And Time Window

Pony Agent SHALL 支持按会话、turn 和时间窗口查询前端诊断数据。

#### Scenario: Query trace window for one session

- **WHEN** 调用诊断查询接口并提供 `sessionId` 与时间窗口
- **THEN** 系统 SHALL 返回该窗口内的前端 trace events
- **AND** SHALL 支持拉取 stall snapshots

#### Scenario: Query trace window for one turn

- **WHEN** 调用诊断查询接口并额外提供 `turnId`
- **THEN** 系统 SHALL 支持将结果收窄到该 turn 对应的事件窗口
- **AND** SHALL 支持分页或 limit 以避免一次返回整段长会话

### Requirement: Export Replayable Trace

Pony Agent SHALL 支持将前端诊断数据导出为可分析格式。

#### Scenario: Export frontend trace as JSON

- **WHEN** 请求导出某个 session 的诊断窗口
- **THEN** 系统 SHALL 支持导出 JSON

#### Scenario: Export frontend trace as Chrome Trace

- **WHEN** 请求导出某个 session 的诊断窗口为性能时间线
- **THEN** 系统 SHALL 支持导出 Chrome Trace 或 Perfetto 兼容格式
- **AND** 导出时间戳 SHALL 满足目标格式要求的精度

### Requirement: Explicit Boundary Between UI Cache And Diagnostics

Pony Agent SHALL 明确区分 UI 偏好/轻量缓存与正式诊断持久化。

#### Scenario: UI preference remains local, diagnostics do not

- **WHEN** 前端保存 UI 偏好或轻量恢复缓存
- **THEN** 这些信息 MAY 继续使用 localStorage
- **BUT** 正式前端性能诊断事件与 stall 快照 SHALL NOT 依赖 localStorage 作为主落点

### Requirement: First-Wave Mandatory Instrumentation Coverage

Pony Agent SHALL 为首批关键链路提供正式埋点覆盖。

#### Scenario: Turn completion path is instrumented

- **WHEN** 一次 turn 进入完成态收尾
- **THEN** recorder SHALL 能观测 `turn:output_end`、`turn:completed` 各阶段与相关关键子步骤

#### Scenario: Rendering hot paths are instrumented

- **WHEN** 前端执行 `HomeWorkspace` 的关键展示链路或 `MarkdownRenderer` 的关键渲染链路
- **THEN** recorder SHALL 能观测这些链路的关键耗时与状态规模

### Requirement: Recorder Lifecycle And Operability

Pony Agent SHALL 定义 recorder 的生命周期与最小可操作诊断路径，而不是只定义事件格式。

#### Scenario: Recorder starts and stops with explicit lifecycle

- **WHEN** 前端应用初始化、绑定 session、切换 turn 或退出页面
- **THEN** recorder SHALL 具有明确的 `init`、`bindSession`、`bindTurn` 与 `flushAndStop` 行为

#### Scenario: Team can validate a stall path end to end

- **WHEN** 团队需要验证 stall 检测与回放链路
- **THEN** 系统 SHALL 提供可重复的人工或自动化 stall 注入/验收路径
