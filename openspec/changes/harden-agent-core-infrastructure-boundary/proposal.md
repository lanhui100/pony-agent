# Proposal: Harden Agent Core Infrastructure Boundary

## Why

Pony Agent 当前已经具备 agent harness 的基础：`runtime / session / graph / planner / provider / tool / control plane` 的主体逻辑已收束到 Rust 侧，且 `TurnEventSink` 已让 Tauri event delivery 与 SSE frame delivery 具备初步 adapter 边界。

但本轮 agent core 审核发现，当前边界仍有几个会让方向慢慢偏向“只为 Tauri 桌面端服务”的风险：

- core 主体仍位于 `src-tauri` crate 中，外部 HTTP/SSE/CLI/服务端宿主无法只依赖一个独立 core package
- `AgentRuntime::with_dependencies`、`HostControlPlane::with_runtime`、`ToolRouter::with_workspace_root` 等关键构造/注入入口对非测试宿主不可用或受限
- 默认构造仍隐含桌面单用户本机假设，例如 `LOCALAPPDATA / APPDATA / dirs::data_local_dir / current_dir`、本地 keyring/file secret 与本地 workspace root
- `sse_adapter` 已能证明事件 sink 可复用，但还不能证明外部宿主能脱离 Tauri crate 构造 core 并消费同一套 control/event contract
- provider 传输目前以 blocking reqwest 为默认实现，桌面端可以通过 `spawn_blocking` 包裹，但服务端/多端宿主需要更明确的 transport 边界与并发策略

这些问题并不表示当前 core 已经偏离方向；相反，代码内依赖方向总体是正确的。但如果不把边界写成 spec，后续继续加桌面功能时，core 很容易在 package、默认配置、存储路径和宿主能力上被 Tauri 语义慢慢粘住。

## What Changes

- 建立 `agent core infrastructure boundary` 的正式 spec，明确 core 必须作为多端基础设施，而不是 Tauri app 内部实现细节
- 要求 core 能以 Tauri-free 的方式构造、编译和被至少一个非 Tauri harness 验证
- 为 runtime/control-plane 暴露稳定 builder 或 equivalent construction API，使宿主能够注入 storage、workspace、provider、tool、secret、event delivery 等依赖
- 把桌面默认路径、keyring、本地 workspace root、Tauri async runtime 等语义降级为 adapter/preset 责任，而不是 core 的不可替换默认假设
- 规定 Tauri adapter、SSE/HTTP/CLI adapter 的职责边界：adapter 只能翻译入口、投递事件、提供宿主 preset，不得复制 provider/session/tool/graph 语义
- 补最小验证矩阵，确保未来新增功能不会把 core 重新绑回 Tauri

## Impact

- 后续桌面端仍可继续使用当前 Tauri workbench，但 Tauri 会成为第一个宿主，而不是 core 的包壳
- 未来 HTTP/SSE、CLI、Linux service 或 Web backend 能复用同一套 `HostControlPlane / AgentRuntime` 语义
- 多端扩展需要的存储、密钥、workspace、provider transport 和事件 delivery 能通过明确接口注入
- 架构审核可以用 spec 直接判断新改动是否破坏 core 基础设施方向

## Tracking

- Task card: `PA-044`
- OpenSpec Change: `harden-agent-core-infrastructure-boundary`
