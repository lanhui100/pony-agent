# Design: Agent Core Infrastructure Boundary

## 背景

当前 `src-tauri/src/agent` 已经形成了较完整的 agent core 主体：

- `AgentRuntime` 承担 turn 执行、tool follow-up、provider interaction 与 trace emission
- `HostControlPlane` 承担 session/run/control/read-plane 的统一入口
- `SessionStore / GraphRunStore / ProviderRegistryStore / ToolRouter` 已具备部分抽象
- `TurnEventSink` 已将 core event production 与宿主 delivery 分开
- `tauri_adapter.rs` 与 `sse_adapter.rs` 已证明事件 delivery 可以有多个 adapter

当前主要风险不是 core 内直接 import Tauri，而是 core 的 package 边界、构造入口和默认依赖仍以桌面宿主为中心。这种状态在早期 workbench 阶段合理，但如果 Pony Agent 要继续作为多端 harness 基础设施，需要把“可被非 Tauri 宿主构造和验证”写成稳定边界。

## 设计目标

1. 明确 agent core 是可独立复用的基础设施层
2. 让 Tauri 桌面端成为 adapter/preset，而不是 core 的所有权边界
3. 让外部宿主能显式注入 storage、workspace、secret、provider、tool、event delivery 与 runtime policy
4. 保留现有桌面端行为，不因抽离边界破坏当前 workbench
5. 用测试和探针证明 core 能脱离 Tauri 编译与运行

## 非目标

- 不在本 change 中重写 provider 协议或模型选择策略
- 不要求一次性实现完整 HTTP server、WebSocket server 或远程多租户产品
- 不重做前端 UI、session control UI 或 trace panel
- 不替换既有 `SessionStore / GraphRunStore / ToolRouter` 的业务语义
- 不把桌面端默认路径视为错误；只要求它们属于 desktop preset，而不是不可替换 core default

## 边界切分

### 1. Core crate 或等价独立 package

目标形态应满足：

- core package 不依赖 `tauri`
- core package 可以被 Tauri adapter、SSE/HTTP adapter、CLI probe 同时依赖
- Tauri-specific code 只存在于 adapter package/module
- build/test 可以单独验证 core，不需要 Tauri build context

推荐路径：

1. 新增 `crates/pony-agent-core` 或等价 workspace member
2. 将 `src-tauri/src/agent/*` 中 Tauri-free 的 core 模块迁入 core package
3. `src-tauri` 继续保留 `lib.rs / main.rs / tauri_adapter.rs` 和桌面 preset
4. 现有 public data contract 尽量保持不变，避免一次性放大前端联调成本

如果暂时不搬 crate，也必须先补一个可审计的 `core` feature/package boundary，并证明 non-Tauri target 可以独立构造运行。

### 2. Runtime builder

当前 `AgentRuntime::new()` 隐含以下默认依赖：

- `SessionStore::new()`
- `ProviderRegistryStore::new()`
- `ToolRouter::new()`
- `LocalTurnPlanner`
- `DefaultTurnContextBuilder`
- `DefaultTurnTelemetryBuilder`

这些默认值适合桌面开发，但不适合作为所有宿主的唯一构造方式。

需要新增稳定 builder 或 equivalent API：

- `AgentRuntimeBuilder`
- `HostControlPlaneBuilder`

builder 至少允许宿主注入：

- session backend
- graph run store backend
- provider resolver / provider registry source
- provider transport policy
- tool executor / workspace root resolver
- secret store
- event sink / stream delivery strategy
- runtime limits / cancellation / concurrency policy

约束：

- builder API 应为非测试可用
- 测试 helper 不能是唯一注入路径
- Tauri desktop default 应实现为 `DesktopRuntimePreset` 或等价 preset
- 非 Tauri probe 应使用 builder 显式构造 core，而不是复用 Tauri state

### 3. Host defaults and presets

以下语义属于 host preset，不属于 core 不可替换默认：

- `LOCALAPPDATA / APPDATA / dirs::data_local_dir`
- `PonyAgent/sessions.json`
- `PonyAgent/graph-runs.json`
- keyring/file secret fallback order
- `std::env::current_dir()` 作为 workspace root
- `tauri::async_runtime::spawn_blocking`
- Tauri event names and `AppHandle`

core 可以提供 recommended defaults，但必须通过 preset 或 config source 注入，并且可以被 HTTP/CLI/service 宿主替换。

### 4. Adapter responsibility

Adapter 只负责：

- 将宿主入口翻译成 core command
- 将 core event/envelope 投递到宿主 delivery mechanism
- 提供宿主 preset，例如 desktop storage path、secret backend、workspace root policy
- 暴露宿主自己的 lifecycle，例如 Tauri command、HTTP route、CLI command

Adapter 不得：

- 复制 provider decision / tool follow-up / session mutation / graph arbitration 语义
- 直接绕过 `HostControlPlane` 调用内部 truth-source 做私有仲裁
- 在前端或 adapter 中重新拼装 core control result
- 让 Tauri-only 类型进入 core public API

### 5. Provider transport boundary

当前 provider 使用 blocking reqwest。该实现可以继续作为 desktop-friendly transport，但需要被视为 provider transport implementation，而不是 core 的唯一并发策略。

v1 要求：

- provider API 层表达 transport boundary
- blocking implementation 可以保留
- HTTP/service 宿主可以在后续替换 async transport 或专用 executor
- cancellation/backpressure 的宿主策略不得被 Tauri runtime 写死

## 迁移策略

### Phase 1: Boundary contract and builders

- 落地本 spec
- 新增 runtime/control-plane builder
- 取消非测试宿主无法注入 workspace/session/provider/tool 的限制
- 增加 non-Tauri probe 使用 builder 构造 core

### Phase 2: Package separation

- 建立 independent core package 或 workspace member
- 移动 Tauri-free modules
- 让 `src-tauri` 依赖 core package
- 保持现有 Tauri commands 和前端 contract 不变

### Phase 3: Second host proof

- 用一个非 Tauri harness 验证：
  - sync turn
  - stream turn
  - graph run stream
  - session persistence
  - tool execution with injected workspace root
  - provider resolver injection
- 该 harness 可以是 CLI probe、HTTP/SSE smoke harness 或 test binary，但必须不依赖 Tauri crate state

## 风险与收敛

### 风险 1：抽 crate 时范围膨胀

收敛方式：

- 先公开 builder 和 dependency seams
- 再搬 package
- 不在同一轮重做前端或 provider protocol

### 风险 2：桌面默认行为被破坏

收敛方式：

- 保留 desktop preset
- Tauri command/event contract 不变
- 所有现有 Tauri-facing tests 继续通过

### 风险 3：抽象过早泛化

收敛方式：

- 只抽宿主边界必需的接口
- 不为尚未出现的产品形态创建过多策略对象
- 至少用一个真实 non-Tauri harness 证明抽象有用途

## 验证策略

最小验证矩阵：

- core package / target 不依赖 Tauri
- Tauri adapter 仍可构造 desktop preset 并通过现有 command/event contract
- non-Tauri harness 能构造 `HostControlPlane` 或 `AgentRuntime`
- non-Tauri harness 能注入 workspace root，而不是使用 process current_dir
- non-Tauri harness 能注入 session/graph storage path
- non-Tauri harness 能消费 `TurnEventSink`
- adapter 层不复制 provider/session/tool/graph 语义
- `rg "tauri::|AppHandle|State<|Emitter|Manager"` 在 core package 内无命中
- reload/persistence 行为在 desktop preset 与 injected backend 下都有测试
