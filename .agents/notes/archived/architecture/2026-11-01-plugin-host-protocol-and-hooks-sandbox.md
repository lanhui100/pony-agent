# Agent Note: 进程外插件宿主协议 v1 与 hooks 沙箱加固（T3）

Status: implemented

> **归档冻结说明**：本文档为原历史决策 `docs/archive/decisions/0067-plugin-host-protocol-and-hooks-sandbox.md` 之归档快照，仅供溯源。

## 背景

- 全仓无插件载体：`dylib` / `wasm` / `libloading` 零命中，`src-tauri` 无旁路服务，唯一外部扩展点是 hooks。
- hooks 现状（`hook_commands.rs`）：`sh -c` / `cmd /C` 起子进程（`:159`）、JSON 走 stdin、默认超时 5000ms（`:35`）、Unix 独立进程组 + SIGKILL 树（`:404`）、配置 `<ws>/.pony/hooks.json` 且默认关闭（`:433-458`）、**完全没有沙箱**（全环境继承、任意 cwd）。
- 因此当前形态是错配的：**能跑任意命令的地方没有沙箱，该接入外部能力的地方没有通道**。

## 决策

1. **载体与协议**：插件 = 可执行程序，协议 = stdio 上的 JSON-RPC 2.0（与 MCP / hooks 同构，失败语义一致）。manifest 文件 `plugin.json`，契约版本 `pony-plugin-v1`：
   ```json
   {
     "apiVersion": "pony-plugin-v1",
     "name": "string",
     "version": "semver",
     "capabilities": ["tools", "hooks"],
     "tools": [{ "name": "string", "description": "string", "inputSchema": {} }],
     "hookPoints": ["ToolCallStart"]
   }
   ```
2. **生命周期**：`spawn` → `initialize`（协商 apiVersion 与能力，声明外一律拒绝）→ 注册 → `call` → `shutdown`；宿主 drop 时 kill 进程组并回收；插件崩溃 fail-closed（**不重试风暴**），当轮返回 `plugin_unavailable`，不悬挂回合。
3. **能力与隔离（默认拒绝）**：插件默认无任何能力；`fsRoots` / `exec` / `network` 必须显式声明并在首次加载时由用户信任（**路径 + manifest 内容哈希**双重指纹，manifest 变更即失效）；`env_clear()` + 白名单（复用 F8a）；cwd 限定 session workspace；单次调用 deadline 由 `BudgetLedger` 提供，超时杀进程组。
4. **消费方**：插件工具经 0065 的 `ToolProvider` 进 registry，命名空间 `plugin__<name>__<tool>`；hooks 在 v1 **不迁移**（Non-Goal），保持 legacy 配置兼容。
5. **hooks 沙箱加固（同 ADR，最小破坏）**：
   - 新增可选 argv 形态 `{"argv": ["cmd","arg"], "timeoutMs": N, "cwd": "…"}`；`command` 字符串形态保留为 legacy。
   - 两种形态都加：env 白名单（不再全量继承）、cwd 限定、默认仍关闭（`PONY_HOOKS_ENABLE=1` 才启用）。
   - legacy shell 形态记 `hook_deprecated_shell` 审计事件；一个 minor 周期后再评估弃用，**不得**在本批直接拒绝旧配置。
   - 决策明确：hooks 迁移到插件协议不扩大任何授权 —— 迁移前后都必须显式启用。

## 候选方案

- **WASM（wasmtime + WIT）**：隔离与热替换最好，但工具链重、调试差、当前无插件作者；协议设计不绑定载体，保留为将来实现。推迟。
- **进程内 dylib（libloading）**：Rust ABI 不稳定，插件与宿主版本耦合，崩溃拖垮宿主。否决。
- **Unix domain socket 承载**：跨平台（Windows）差，且与现有 stdio 语义不一致。否决。
- **把 hooks 全量迁进插件协议**：会改 cwd/env/stdin 载荷，破坏存量用户配置，且当前插件协议没有第二实现者。否决（改为沙箱加固 + 双形态共存）。
- **不设 manifest 信任指纹（只按路径信任）**：插件被替换后静默获得原授权。否决。

## 后果

- 新增模块（建议 `crates/pony-agent-core/src/agent/plugin_host/`）：`manifest.rs`（schema + 校验）、`transport.rs`（stdio JSON-RPC）、`supervisor.rs`（spawn/超时/回收）、`capability.rs`（白名单）、`mod.rs`（对 `ToolProvider` 的实现）。
- 安全面扩大：首次加载必须显式信任，宿主不随配置文件自动启用；审计日志记录插件名、命令哈希、能力集合、每次调用成败。
- 平台差异（已知局限，必须写进验收）：Windows 无进程组，`cmd /C` 会派生孙进程 → 用 Job Object（已有 `src-tauri/src/platform.rs` 的先例）或明示"尽力回收"。
- 可观测性：新增后台进程与超时取消路径，必须在同一提交更新 `docs/observability.yaml`。
- 风险登记：无真实第三方消费者时冻结 v1 存在返工风险 → 验收以 `tests/fixtures/` 内的 fixture 插件跑通全链路为准，并在 ADR 中显式标注"v1 可能随首个真实插件修订"。

