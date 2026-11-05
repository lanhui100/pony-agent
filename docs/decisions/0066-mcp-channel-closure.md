# ADR 0066：MCP 通道闭合（T2）

Status: implemented

## 背景

`crates/pony-agent-core/src/agent/mcp_client.rs` 已实现规范最小子集（`initialize` 握手、`tools/list`、`tools/call`、`resources/list|read`、按 id 匹配、每请求超时、`Drop` 终止子进程、F8a 环境白名单）。但通道三段全部未闭合：

1. **无配置加载**：全仓没有任何 `.mcp.json` / `mcpServers` 读取；`McpServerSpec` 只能在代码里手工构造。
2. **无 host 入口**：`HostControlPlane::apply_mcp_source_snapshot`（`control_plane/capability_commands.rs:82`）存在，但 `src-tauri` 的 91 个 command（`src-tauri/src/lib.rs:1399-1487`）未暴露它，仅测试调用。
3. **工具映射断链**：capability `label` 直接当工具名（`capability_bridge.rs:568`）→ 静态 match → `unsupported_tool`；资源面固定 fail-closed（`runtime/tool_exec.rs:330`）。

## 决策

前置依赖：本 ADR 的 4/5 项依赖 0065 的 `swap_tool_surface` 原子换入接缝与 `is_builtin_tool_definition_set` 拆除，**T2 必须在 T1 之后实施**。

1. **配置面**：两层，子目录/全局优先级与 skills 同构。
   - `<workspace>/.mcp.json`（项目级）
   - `$PONY_AGENT_HOME/mcp.json`（全局，默认 `config_dir/pony-agent`）
   - schema 版本 `mcp-config-v1`，键沿用 `mcpServers`（兼容既有生态写法），字段 `command` / `args` / `env` / `cwd` / `disabled`；**未知字段容忍**（前向兼容），**未知必需字段 fail-closed 仅禁用该条**。
   - 配置层 fail-soft：单条坏配置不得拖垮 app 启动或会话开始；调用层 fail-closed。
2. **凭据**：配置文件只允许 `${env:VAR}` 间接引用，禁止字面量 secret；`env` 最终仍走 F8a 白名单 scrub。日志/诊断只记 server 名、命令哈希、工具名、参数 digest，不记原值；错误信息复用 `mcp_client.rs` 的脱敏（`http://user:pass@` → 去凭据）。
3. **Host 入口**：暴露 `list_capability_sources` / `apply_mcp_source_snapshot` / `mcp_health`（新增：连接状态、最近错误码、工具数）到 tauri command 与 CLI；前端只经 command 调用，不直连 MCP（遵守 AGENTS.md「模型网络流量严禁直接走前端」）。
4. **双注册表职责收敛**：`CapabilityRegistry`（capability_bridge 侧，含 skills/prompts/resources）与 `ToolRegistrySnapshot`（工具面）当前并存且靠 `label == tool_name` 反查回灌（`capability_bridge.rs:595-628`）。收敛规则：**capability 只负责发现与来源元数据，工具身份一律由 `ToolRegistrySnapshot` 决定**；外部工具以 `source_id` 关联，不再用 label 当名字。
5. **工具映射**：MCP `tools/list` → 外部 descriptor（`source_id = mcp:<server>`，`model_name = mcp__<server>__<tool>`，权限事实 = 只读/需授权按 server 配置）→ registry → `ToolProvider`（0065）→ `GovernedDispatcher`。删除 `capability_bridge` 的 label-as-tool-name 路径。
6. **资源面**：v1 保持 fail-closed，但错误码提升为可读的 `mcp_resources_not_wired`（不再是含糊的 `SourceUnavailable`）；接线列入后续任务（非目标）。
7. **传输**：v1 只 stdio。streamable-http 与 OAuth 明确非目标，另立 ADR。

## 候选方案

- **直接复用 DSH `packages/mcp/mcp-client`**：TS 实现，无法在 Rust 内核内加载，且引入 Node 运行时依赖。否决。
- **本轮同时做 streamable-http**：OAuth/凭据/重连语义是独立产品面，且无消费场景，会把 v1 拖成跨季度工程。推迟。
- **让前端直连 MCP**：违反「模型网络流量严禁直接走前端」的架构约束，且无法复用 env scrub 与信任边界。否决。
- **资源面一并接线**：`McpResourceSurface` 已存在但未接生产路径，接线本身不难；但 v1 的验收重点是"工具能装进来"，资源读取无消费方，先保持 fail-closed 更安全（采纳）。

## 后果

- 用户可见：`.mcp.json` 落盘后，MCP 工具出现在模型工具面；错误可区分 `mcp_server_unavailable` / `mcp_tool_not_found` / `mcp_timeout`。
- 可观测性：新增配置加载、后台子进程与网络 IO → 必须在同一提交更新 `docs/observability.yaml` 四维归属，并跑 `audit-diff.sh` / `check-declaration.sh`。
- 已知局限：无 UI 管理面（T5 未做），用户只能手写配置；无远程传输；资源读取仍不可用。
- 安全：MCP server 是外部代码，启动前 `env_clear()` + 白名单；首次加载需显式信任（路径 + 内容哈希），配置变更即重新确认。
