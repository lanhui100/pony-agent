## ADDED Requirements

### Requirement: Agent core SHALL be usable as multi-host infrastructure
Pony Agent 的 agent core SHALL 作为多端宿主可复用的基础设施存在，而不是 Tauri desktop app 的内部实现细节。

#### Scenario: A non-Tauri host depends on core
- **WHEN** 外部 HTTP/SSE/CLI/service 宿主需要复用 agent runtime
- **THEN** 该宿主 SHALL 能依赖 core package 或等价 Tauri-free target
- **AND** SHALL NOT 被要求依赖 `tauri`、`tauri-build`、`AppHandle` 或 Tauri command/event runtime

#### Scenario: Tauri desktop uses the same core
- **WHEN** Tauri 桌面端启动 runtime 或 graph run
- **THEN** Tauri adapter SHALL 调用同一套 core command/runtime/control-plane contract
- **AND** SHALL NOT 复制 provider decision、session mutation、tool follow-up 或 graph arbitration 语义

#### Scenario: Core boundary is inspected
- **WHEN** 审核 core package 或等价 Tauri-free target
- **THEN** `tauri::`、`AppHandle`、`State<...>`、`Emitter`、`Manager` 等 Tauri-only 类型 SHALL NOT 出现在 core implementation 或 public API 中

### Requirement: Runtime and control plane SHALL expose host-injectable construction
Agent runtime 与 host control plane SHALL 提供非测试宿主可用的稳定构造入口，使宿主能够注入运行依赖。

#### Scenario: A host constructs runtime with custom dependencies
- **WHEN** 非 Tauri 宿主构造 `AgentRuntime`
- **THEN** 它 SHALL 能注入 session backend、provider resolver、tool executor、planner、context builder 与 telemetry builder
- **AND** 该构造入口 SHALL NOT 只在 `#[cfg(test)]` 下可用

#### Scenario: A host constructs control plane with custom runtime and graph store
- **WHEN** 非 Tauri 宿主构造 `HostControlPlane`
- **THEN** 它 SHALL 能注入 runtime、execution-control policy、graph run store 与 graph planner 或等价 dependencies
- **AND** SHALL NOT 被迫使用 desktop local persistence path

#### Scenario: Workspace root is injected
- **WHEN** 宿主需要在指定 workspace root 中执行 workspace tools
- **THEN** tool executor SHALL 使用宿主注入的 workspace policy
- **AND** SHALL NOT 只能依赖 process `current_dir`

### Requirement: Desktop defaults SHALL be host presets, not immutable core assumptions
桌面端本地路径、密钥存储、本机 workspace 与 Tauri async runtime SHALL 属于 host preset，而不是 core 不可替换默认。

#### Scenario: Desktop app uses local storage
- **WHEN** Tauri 桌面端运行
- **THEN** desktop preset MAY 使用 `dirs::data_local_dir`、`LOCALAPPDATA / APPDATA`、keyring/file secret fallback 与本地 workspace root
- **AND** 这些默认值 SHALL 通过 adapter/preset 注入 core

#### Scenario: Service host uses different storage
- **WHEN** HTTP/SSE/service 宿主使用数据库、容器挂载路径或远程 secret backend
- **THEN** core SHALL 允许宿主替换 session storage、graph storage、provider registry source 与 secret store
- **AND** SHALL NOT 将 `PonyAgent/sessions.json` 或 local keyring 作为唯一可行路径

#### Scenario: Runtime execution policy differs by host
- **WHEN** 不同宿主需要不同并发、取消或 blocking/async 策略
- **THEN** core SHALL 暴露足够的 policy seam
- **AND** SHALL NOT 把 `tauri::async_runtime::spawn_blocking` 写成 core 执行模型

### Requirement: Adapter SHALL translate host protocol without owning core semantics
Adapter SHALL 只负责宿主协议翻译和事件投递，不得成为第二套 runtime。

#### Scenario: Tauri command invokes a turn
- **WHEN** Tauri command 收到 run/turn/control 请求
- **THEN** adapter SHALL 将请求转换为 core command
- **AND** SHALL 由 core 决定 provider、tool、session、graph 与 lifecycle 语义

#### Scenario: SSE or HTTP host emits stream events
- **WHEN** SSE/HTTP adapter 消费 core turn stream
- **THEN** adapter SHALL 通过 `TurnEventSink` 或等价 event contract 投递事件
- **AND** SHALL NOT 在 adapter 内重新推导 terminal event、trace step、tool activity 或 control result

#### Scenario: Adapter exposes host-specific names
- **WHEN** 宿主需要 Tauri event name、SSE event name 或 HTTP response shape
- **THEN** adapter MAY 映射宿主协议名称
- **AND** core SHALL 保持稳定 canonical event/control payload

### Requirement: Non-Tauri harness SHALL prove the boundary
系统 SHALL 至少提供一个非 Tauri harness，用于证明 core 能被第二宿主构造和消费。

#### Scenario: Non-Tauri harness runs a sync turn
- **WHEN** non-Tauri harness 使用 builder 构造 core
- **THEN** 它 SHALL 能执行 sync turn
- **AND** SHALL 使用注入的 provider resolver 或 mock provider

#### Scenario: Non-Tauri harness runs a stream turn
- **WHEN** non-Tauri harness 启动 stream turn
- **THEN** 它 SHALL 能通过非 Tauri sink 消费 started/delta/tool/completed/failed 等事件
- **AND** SHALL NOT 依赖 `AppHandle.emit`

#### Scenario: Non-Tauri harness verifies injected persistence
- **WHEN** non-Tauri harness 使用自定义 session/graph storage path
- **THEN** session 与 graph state SHALL 能 roundtrip
- **AND** reload 后 SHALL 仍通过 core read-plane 读取同一语义

#### Scenario: Non-Tauri harness verifies workspace tools
- **WHEN** non-Tauri harness 注入 workspace root
- **THEN** workspace tool SHALL 在该 root 下执行
- **AND** SHALL NOT 读取运行进程当前目录作为隐含 workspace

### Requirement: Provider transport SHALL be a replaceable implementation detail
Provider transport SHALL 被视为 provider adapter/transport implementation，而不是所有宿主共享的唯一并发模型。

#### Scenario: Desktop keeps blocking transport
- **WHEN** 桌面端使用 blocking reqwest provider transport
- **THEN** 该实现 MAY 保留
- **AND** desktop adapter MAY 使用合适 executor 包裹 blocking work

#### Scenario: Service host needs async or bounded transport
- **WHEN** service host 需要 async transport、bounded executor、request cancellation 或 backpressure
- **THEN** core SHALL 提供可替换 transport 或 policy seam
- **AND** SHALL NOT 要求该宿主复用 Tauri desktop 的 blocking execution strategy

### Requirement: Existing desktop behavior SHALL remain stable during boundary hardening
边界加固 SHALL 不破坏当前 Tauri desktop workbench 的既有 command/event contract。

#### Scenario: Desktop command contract is regression tested
- **WHEN** core package 或 builder/preset 发生迁移
- **THEN** 现有 Tauri command 与 frontend event consumption SHALL 继续通过回归测试
- **AND** 前端 SHALL NOT 因 package boundary 调整而改变用户可见行为

#### Scenario: Existing probes continue to work
- **WHEN** `direct_turn_probe`、`sse_turn_probe` 或等价 probe 被迁移到新边界
- **THEN** 它们 SHALL 继续验证同一套 core 语义
- **AND** probe 的存在 SHALL 不替代正式 non-Tauri construction test
