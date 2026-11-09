# Pony Agent Core (crates/pony-agent-core)

Pony Agent 的核心驱动库，包含调度器、执行引擎、沙箱环境、大模型客户端通信与可观测性集成。

## 配置与运行时规范 (Configuration & Semantics)
- 运行在本地独立进程或作为 Tauri 桌面宿主的原生后端。
- 所有大模型 IO 统一经由该 crate 中的 client 处理，禁止前端直接 fetch 模型。
- 提供命令行工具（`cli.rs`）、终端交互（`tui.rs`）及无界面 Harness 测试桩。

## 限制与边界 (Limitations)
- 不包含 GUI 渲染逻辑。
- 必须遵循 Fail-Closed 的外部命令与 URL 打开白名单约束。

## 两个二进制入口（同一 crate、同一 core 版本线）

| 入口 | 源文件 | 面向对象 | stdout 契约 | 公开制品 |
|---|---|---|---|---|
| `pony` | `src/bin/tui.rs` | 人（终端交互） | 交互界面；`pony update` 走「收据在 stdout、阶段日志在 stderr」 | **有**：`pony-tui_<core 版本>_<平台>` + 清单 `pony-cli-latest.json`。平台矩阵真值源 `scripts/cli-platforms.mjs`（[ADR 0108](../../docs/decisions/0108-cli-release-platform-matrix.md)），自更新协议见 [ADR 0087](../../docs/decisions/0087-cli-self-update.md) |
| `ponycli` | `src/bin/cli.rs` | 机器与脚本 | `--output-format jsonl/sse` 时 stdout 只承载结构化事件；`text` 与基线逐字节兼容；权限档位默认 `read_only`（无头形态没人在环确认） | **无**：仓库内无头工具，真实消费者是 `scripts/audit/replay.sh`（trace 审核回放）。需要时本机 `cargo build --release --bin ponycli` 或 `npm run cli:build`（后者一次构建并安装两个入口） |

拆两个入口的理由是**终端控制权契约互斥**（备用屏独占 vs 结构化事件单流），不是「两个包」：二者共享同一
lib、同一依赖树、同一个版本号。`ratatui` / `ratatui-image` / `image` 写在 crate 级 `[dependencies]`，
因此 `ponycli` 也会链接它们 —— cargo 没有 per-`[[bin]]` 依赖，要解耦只能拆 crate（未采纳：收益只有二进制
体积，代价是新增一层跨 crate 契约）。
