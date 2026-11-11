# Agent Note: Harness 可扩展性三层模型与不可替换内核

Status: implemented

> **归档冻结说明**：本文档为原历史决策 `docs/archive/decisions/0064-extensibility-three-layer-model.md` 之归档快照，仅供溯源。

## 背景

对照 DeepSeek Harness（Cordis 全插件：产品每一部分都是插件，包括 agent loop 本身，靠 profile + YAML patch 在运行期替换；插件即 npm 包，运行期 `import`）后，本项目的可扩展性缺口是结构性的，而非实现细节：

1. **对外通道断裂**：MCP 有规范 stdio 客户端（`mcp_client.rs:321` 握手 / `:496` tools/list），但没有配置加载、没有 host 命令暴露、工具名最终落到静态 match 变 `unsupported_tool`；资源面在生产路径显式 fail-closed（`runtime/tool_exec.rs:330` 注释自述 `McpResourceSurface` 未接线）。
2. **元数据曾有第二真值源**：PA-076 起以 `ToolDescriptor` / `ToolRegistrySnapshot` 取代名称推导，但 `canonical_tool_name`（`tools.rs:7352`）与 `execute_internal`（`tools.rs:1524`）仍是手写 match，外部名称无法进入治理分派。
3. **无任何插件载体**：全仓 `dylib` / `wasm` / `libloading` 零命中；唯一外部扩展点 hooks 是 `sh -c` 命令（`hook_commands.rs:159`），无沙箱、默认关闭。

本 ADR 只决定**分层模型与边界**，四件具体做法见 0065–0068。

## 决策

### 1. 三层模型（按信任边界分层许诺，而非按语言）

| 层 | 内容 | 扩展方式 | 是否允许第三方运行期替换 |
|---|---|---|---|
| **L0 内核** | turn loop、session 事件日志、治理分派管线、沙箱边界、LLM wire 适配、权限/审批证据 | 只能随版本演进（改 Rust） | 否 —— 这是安全与真值面 |
| **L1 编译期接缝** | 工具、LLM provider、sandbox backend、session store、hook point、skill source | 实现 trait + 注册 descriptor | 否（但新增不需要改内核分派表） |
| **L2 进程外插件** | MCP server、插件宿主进程 | 协议接入（stdio JSON-RPC 2.0） | 是 —— 进程边界即信任边界 |

### 2. 三条不变量

- **单一真值**：工具的身份、顺序、权限事实、schema 只来自 descriptor registry；名称匹配表不得充当元数据来源。
- **前缀稳定**：模型可见工具面在会话内冻结，按 `(source_id, model_name)` 稳定排序，条目有上限；外部能力加入不得让 KV-cache 前缀在轮次中抖动（ADR 0007）。
- **失败闭合**：未知来源、越权、超时、版本不符一律 fail-closed，且错误码必须能区分来源（`mcp_*` / `plugin_*` / `capability_denied` / `unsupported_tool`）。

### 3. 本批明确 Non-Goals

1. **进程内插件**（dylib / wasm / libloading）——Rust ABI 不稳定，公共插件面不采用；WASM 保留为将来载体（协议设计不绑定执行载体）。
2. **运行期替换内核服务**——不做 Cordis 式的任意服务热替换。
3. **UI 插件化**——前端编译进 Tauri 产物；UI 扩展只经 command + 配置驱动渲染。
4. **workflow 编排**——当前 `workflow()` 是桩（`orchestration.rs:616`），本批不处理。
5. **插件市场自动安装**——不做分发与自动更新面。
6. **远程 streamable-http MCP**——v1 只 stdio；远程传输与 OAuth 是独立产品面，另立 ADR。
7. **hooks 迁移到插件协议**——保留 legacy 配置，仅做沙箱加固（见 0067）。
8. **可见性与信任面（T5）**——MCP 服务器管理页、授权记录、通用工具渲染 fallback、错误码用户文案。本批不做，登记 backlog；缺失后果：T2 上线后用户"装了但不知道装了什么"。

### 4. 本批交付物与 ADR 路由

| ADR | 任务 | 一句话 |
|---|---|---|
| 0065 | T1 | 工具面单一真值闭合 + 外部工具通道（`ToolProvider`） |
| 0066 | T2 | MCP 通道闭合（配置 / host 命令 / 工具映射 / 凭据脱敏） |
| 0067 | T3 | 进程外插件宿主 v1 + hooks 沙箱加固 |
| 0068 | T4 | 五份契约版本化 + conformance kit + 扩展点归属表 |

### 5. 交付顺序与硬依赖

| 波次 | 成员 | 并发性依据 |
|---|---|---|
| **W1** | T1（工具面单一真值 + 原子换入接缝）∥ T3（插件宿主新模块） | 写域正交：T1 在 `tools.rs` / `dispatcher.rs` / `governed_executor.rs` / `provider/mod.rs`；T3 在新目录 `plugin_host/` + `hook_commands.rs` |
| **W2** | T2（MCP 闭环）∥ T4 骨架（schema + 正例） | T2 依赖 W1 的 `swap_tool_surface` 与 `is_builtin_tool_definition_set` 拆除；T4 只产骨架不冻结 |
| **W3** | T4 冻结（golden + 12 例全绿） | 必须在 T2/T3 形状落地后冻结，否则立刻返工 |

硬依赖（不可并行）：**T2 依赖 T1**（`dispatcher.rs:471,489` 的 registry 构造期固定，全仓无换表 API）；**T4 冻结依赖 T1/T2/T3**。文件交点只有 `src-tauri/src/lib.rs:1399` 的 `generate_handler!` 列表与 `control_plane/mod.rs`，由单一写主追加。


## 候选方案

- **A 照搬 Cordis（进程内 DI + 热替换 + npm 插件）**：Rust 无稳定 ABI、无运行期反射、无声明合并；只能在"改内核"与"零扩展"之间二选一。否决。
- **B 全量 WASM 组件插件（wasmtime + WIT）**：隔离最好，但生态薄、调试与工具链成本高，且当前没有插件作者。保留为未来载体，不在 v1 落地。
- **C 只做编译期接缝（保持现状）**：第三方零接入，等于宣称的可扩展性不存在。否决。
- **D 三层模型（采纳）**：按信任边界分层许诺，扩展成本与风险可度量，且每层都有机器可验证的契约。

## 后果

- 扩展成本可表述：**新增内建工具 = 改 Rust**；**新增外部能力 = 写插件或 MCP server，不改内核**。
- L2 引入进程与协议 → 必须承担观测、脱敏、孤儿进程回收、Windows 无进程组的平台差异（0067 的已知局限）。
- 半成品风险已识别：T1 单独交付对用户零可见且无法自证外部通道可用（须由 T2 的 MCP fixture 反向验证）；T3 在无外部消费者时冻结协议存在返工风险，故 T3 的验收以"fixture 插件可跑通全链路"为准，真实第三方生态不在本批承诺内。

