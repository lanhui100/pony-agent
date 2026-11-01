# Pony Agent Core (crates/pony-agent-core)

Pony Agent 的核心驱动库，包含调度器、执行引擎、沙箱环境、大模型客户端通信与可观测性集成。

## 配置与运行时规范 (Configuration & Semantics)
- 运行在本地独立进程或作为 Tauri 桌面宿主的原生后端。
- 所有大模型 IO 统一经由该 crate 中的 client 处理，禁止前端直接 fetch 模型。
- 提供命令行工具（`cli.rs`）、终端交互（`tui.rs`）及无界面 Harness 测试桩。

## 限制与边界 (Limitations)
- 不包含 GUI 渲染逻辑。
- 必须遵循 Fail-Closed 的外部命令与 URL 打开白名单约束。
