## Key State Definitions

以下术语在本 spec 中具有特定含义：

- **轻量占位态 (lightweight placeholder)**: store 层面 `phase = "placeholder"`, `sessionId = targetId`, `messages = []`, `transcript = null`；组件层面渲染 `SessionPlaceholderSkeleton`（居中 3-4 条线条状骨架块 + 柔和脉冲动画）。
- **轻量 loading 态 (hydration loading state)**: store 层面 `sessionHydrating = true`；组件层面在 transcript 区域顶部渲染细水平进度条，侧边栏保持完全可交互。不在该阶段挂载完整 transcript 列表。
- **transient 会话**: `phase = "idle"`, `messages = []`, `sessionOperation = "creating"`, 无 `activeTurnId`。
- **后台运行 turn (background turn)**: 用户切换离开时仍在后端执行的 turn，由 `runningSessionMap[sessionId] = { turnId, phase }` 追踪。

## MODIFIED Requirements

### Requirement: Session switching SHALL remain interactive while other sessions are active
Pony Agent 前端在切换历史会话时 SHALL 先完成本地前台切换，而不是等待宿主读取完成后再更新 UI。

#### Scenario: The user switches to a cached historical session
- **WHEN** 目标会话已有本地持久化缓存
- **AND** 缓存数据通过 schema 校验（`cachedStateVersion` 匹配、必需字段完整、JSON 可解析）
- **THEN** 前端 SHALL 立即切换到该会话
- **AND** SHALL NOT 在切换动作中自动触发 `load_session_runtime_view`
- **AND** SHALL NOT 在切换动作中自动触发 `list_sessions`
- **AND** 后台 SHALL 发起一次非阻塞的轻量宿主刷新（仅获取 `turn-status` 及 `checkpoint` 元数据），用于检测缓存是否已 stale

#### Scenario: The user switches to a cached historical session but cache is corrupted
- **WHEN** 目标会话有本地持久化缓存
- **AND** 缓存数据 schema 校验失败（版本不匹配、必需字段缺失、JSON parse 错误）
- **THEN** 前端 SHALL 回退到 uncached 路径
- **AND** SHALL 输出一条 diag warning 但 SHALL NOT 抛异常
- **AND** SHALL NOT 阻塞前台切换

#### Scenario: The user switches to an uncached historical session
- **WHEN** 目标会话没有本地持久化缓存
- **THEN** 前端 SHALL 立即切换到轻量占位态
- **AND** SHALL NOT 因宿主读取阻塞而延迟前台切换反馈
- **AND** 工作区 SHALL 在占位态渲染完成后自动发起一次后台宿主读取（`load_session_runtime_view` 或更轻量的等价端点）
- **AND** 宿主读取完成后 SHALL 自动更新会话状态并替换占位态

#### Scenario: The user switches rapidly between three or more sessions
- **WHEN** 用户在短时间内依次触发 `switchSession(A) → switchSession(B) → switchSession(C)`
- **AND** B 的后台宿主读取在 C 完成后才到达
- **THEN** B 的宿主读取结果 SHALL 被 `sessionSwitchToken` 机制丢弃
- **AND** C SHALL 始终是最终前台会话
- **AND** `sessionOperation` SHALL NOT 卡在 "switching" 状态
- **AND** A 的 `persistHistory()` SHALL 在首次切换时完成

#### Scenario: The user switches away while a turn is streaming, then switches back
- **WHEN** 用户在 turn 正在执行时切换离开
- **THEN** 前端 SHALL 将该 turn 注册为 `runningSessionMap[sessionId]`（含 `turnId` 和 `phase`）
- **AND** SHALL 在 `persistHistory()` 中持久化该标记到 localStorage
- **WHEN** 用户稍后切换回该会话
- **THEN** 前端 SHALL 从 `runningSessionMap` 恢复 `turnId`、`phase` 和 `isSubmitting`
- **AND** transcript 区域 SHALL 显示一个非侵入式横幅："此会话在后台仍有正在进行的轮次"
- **AND** 前端 SHALL 订阅后端 turn 事件以接收流式 delta

#### Scenario: The user closes and reopens the app while a background turn was running
- **WHEN** 应用关闭时 `runningSessionMap` 中有非空条目
- **AND** `persistHistory()` 已将该标记写入 localStorage
- **WHEN** 应用重新启动
- **THEN** `initializeSessions()` SHALL 从 localStorage 恢复 `runningSessionId` 和 `turnId`
- **AND** SHALL 发起一次轻量宿主读取以确认该 turn 的当前状态（`running` / `completed` / `failed`）
- **AND** SHALL NOT 默认假设该 turn 仍处于 `running` 状态

#### Scenario: The host read for an uncached session fails
- **WHEN** 占位态已渲染，后台宿主读取失败（网络错误、后端崩溃、会话已被删除）
- **THEN** 工作区 SHALL 在 transcript 区域展示非阻塞的行内错误提示
- **AND** SHALL 提供 "重试" 按钮
- **AND** SHALL NOT 自动导航离开
- **AND** 占位态 SHALL 保持可见直到重试成功或用户主动切换到其他会话

#### Scenario: The deferred hydration exceeds the timeout threshold
- **WHEN** 切换后 15 秒内后台宿主读取未完成
- **THEN** 前端 SHALL 展示行内超时错误状态与重试按钮
- **AND** SHALL NOT 阻塞侧边栏交互

### Requirement: Session creation SHALL preserve visible session history without blocking on host catalog reads
Pony Agent 前端在创建新会话时 SHALL 本地保留当前已保存会话的可见性，而不是同步依赖宿主 catalog 刷新。

#### Scenario: The user creates a new session from a persisted session
- **WHEN** 当前会话已有可持久化内容
- **THEN** 前端 SHALL 立即创建 transient 新会话
- **AND** SHALL 继续在侧边栏中保留旧会话概览条目（指侧边栏中的 session list item，而非完整 transcript）
- **AND** SHALL NOT 为了创建 transient 会话而同步调用 `list_sessions`
- **AND** 旧会话的完整 transcript SHALL 在用户切回时完好保留

#### Scenario: The user creates a new session while a turn is streaming in the current session
- **WHEN** 当前会话有正在执行的 turn
- **THEN** 前端 SHALL 将该 turn 注册为 `runningSessionMap[当前sessionId]`
- **AND** SHALL 创建 transient 新会话
- **AND** 旧会话的会话概览（含 turn 状态标记）SHALL 在侧边栏中可见
- **AND** 用户切回旧会话时 SHALL 看到 turn 的完整执行结果

### Requirement: Session sidebar SHALL never render duplicate conversation entries
Pony Agent 前端 session sidebar 在渲染前 MUST 对 `conversationId` 去重，避免 duplicate key 和多项选中。

#### Scenario: Upstream session state contains duplicate conversation ids
- **WHEN** 前端接收到包含重复 `conversationId` 的 session list
- **THEN** sidebar SHALL 只渲染按数组顺序的第一条稳定项
- **AND** SHALL NOT 在同一次 render 中产生 duplicate key warning

#### Scenario: The session list includes a session that was deleted on the backend
- **WHEN** 后端返回的 session list 不再包含某个本地缓存的会话
- **THEN** 侧边栏 SHALL 将该会话项标记为 "已删除/不可用"（置灰，带删除线）
- **AND** 用户点击该条目时 SHALL 显示信息态："此会话已不再可用"
- **AND** SHALL 提供一个 "从列表中移除" 的确认操作

### Requirement: Workspace hydration SHALL stage transcript rendering
Pony Agent 工作区在切换或恢复会话时 SHALL 先渲染轻量状态，再延迟挂载 transcript 内容。

#### Scenario: The workspace is hydrating a session
- **WHEN** 会话状态刚切换且 transcript 仍在 hydration
- **THEN** 工作区 SHALL 展示轻量 loading 态
- **AND** SHALL NOT 在该阶段立即挂载完整 transcript 列表
- **AND** 轻量 loading 态 SHALL 保持到宿主读取完成或超时

#### Scenario: The workspace hydration completes successfully
- **WHEN** 后台宿主读取完成且会话状态已更新
- **THEN** 工作区 SHALL 从轻量 loading 态转换为完整 transcript 渲染
- **AND** `sessionHydrating` SHALL 设置为 `false`
- **AND** SHOULD 在转换时使用 Vue 的 `<Transition>` 以避免布局抖动

### Requirement: Session initialization SHALL prefer persisted local state before heavy host reads
Pony Agent 在启动或前端重载后恢复最近会话时 SHALL 优先使用本地已持久化 runtime state，而不是无条件触发重量级宿主会话视图读取。

#### Scenario: Persisted state exists for the preferred session during initialization
- **WHEN** 最近会话已有本地持久化 state
- **AND** 该 state 通过 schema 校验
- **THEN** 前端 SHALL 先基于本地 persisted state 恢复最近会话
- **AND** SHALL NOT 为了首屏恢复无条件调用 `load_session_runtime_view`
- **AND** SHALL 仅在恢复完成后发起一次轻量异步宿主刷新以检查新数据

#### Scenario: Initialization needs checkpoint or recovery semantics unavailable locally
- **WHEN** 本地 persisted state 不足以恢复 checkpoint、recovery run 或 terminal phase 语义
- **THEN** 前端 MAY 触发更轻量或更精确的宿主读取
- **AND** SHALL 避免把全量 `load_session_runtime_view` 作为默认初始化路径
- **AND** 充分的判定条件定义如下: persisted state 足够当且仅当 `localState.checkpoint != null && localState.phase !== "idle"`（即 checkpoint 对象存在且会话不处于空闲状态）

#### Scenario: No persisted state exists for the preferred session
- **WHEN** 最近会话没有任何本地 persisted state
- **THEN** 前端 MAY 回退到宿主读取以恢复该会话
- **AND** 该路径 SHALL 被视为异常或冷启动恢复路径，而不是默认热路径

#### Scenario: Initialization runs while localStorage is unavailable
- **WHEN** `localStorage.setItem` 或 `localStorage.getItem` 抛出异常（配额满、隐私模式、磁盘满）
- **THEN** 前端 SHALL 捕获该异常
- **AND** SHALL 输出一条 diag warning
- **AND** SHALL 回退到全量宿主读取路径
- **AND** 该会话 SHALL 在本次会话中被视为 uncached（下次导航走宿主路径）
- **AND** SHALL NOT 因持久化失败而阻塞初始化流程

### Requirement: Cache write-back SHALL persist local state before session switches
Pony Agent 前端 SHALL 在切换会话前将当前会话的变更写回本地持久化存储。

#### Scenario: The user switches away from a modified session
- **WHEN** 用户通过 `switchSession()` 或 `createSession()` 离开当前会话
- **THEN** 前端 SHALL 在加载目标会话之前调用 `persistHistory()`
- **AND** `persistHistory()` 失败（localStorage 不可用）时 SHALL 输出 warning
- **AND** SHALL NOT 阻止切换流程继续执行

#### Scenario: The user closes the app
- **WHEN** `beforeunload` 事件触发
- **THEN** 前端 SHALL 执行一次 `persistHistory()` 以保存当前会话状态
- **AND** SHALL 持久化 `runningSessionMap` 中所有活跃 turn 的状态

### Requirement: Background session turn status SHALL be recoverable across restarts
Pony Agent 前端 SHALL 在持久化缓存中保存后台运行 turn 的状态标记，以支持跨重启恢复。

#### Scenario: App restarts while a session has a background turn
- **WHEN** `initializeSessions()` 执行
- **AND** localStorage 中存有 `runningSessionId` 和 `turnId`
- **THEN** 前端 SHALL 将该信息写入 `runningSessionMap`
- **AND** 在切换到该会话时 SHALL 发起一次轻量宿主读取以核实 turn 的实际状态
- **AND** SHALL NOT 假定该 turn 一定处于 running 状态
