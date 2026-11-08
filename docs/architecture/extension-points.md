# 扩展点归属表（生成物）

> 本文件由 `scripts/gen-extension-points.mjs` 从源码事实生成，**禁止手改**（手写副本即第二真值源，ADR 0068 决策 3 已否决）。
> 重生成：`node scripts/gen-extension-points.mjs --write`；一致性校验：`node scripts/gen-extension-points.mjs --check`。
> 事实源：`agent/tools/contract.rs`（registry source 枚举）、`tool_runtime.rs`（已定义 trait 端口）、`hooks.rs`（钩子点）、`docs/contracts/*.v1.schema.json`（契约版本）、各载体文件是否存在。

## 1. 想加什么，该挂哪（ADR 0064 三层模型）

| 扩展点 | 层 | 扩展方式 | 契约 | 载体 | 当前状态（由文件事实推导） |
|---|---|---|---|---|---|
| 内建工具 | L1 编译期接缝 | 改 Rust：实现 handler + 注册 descriptor | `pony-tool-descriptor-v1` | `governed_executor.rs` | 载体已存在（`governed_executor.rs`） |
| MCP server | L2 进程外插件 | 写 MCP server + `.mcp.json`（v1 只 stdio） | `mcp-config-v1 + pony-tool-descriptor-v1` | `mcp_client.rs` | 客户端已存在（`mcp_client.rs`）；配置面关键词 `mcpServers` 已命中 |
| Skill | L1 编译期接缝 | 写 `SKILL.md`（frontmatter） | `pony-skill-manifest-v1` | `workspace_hotloader.rs` | 载体已存在（`workspace_hotloader.rs`） |
| Hook | L1 编译期接缝 | 写 `.pony/hooks.json`（`command` 或 `argv` 形态） | `agent-hooks-v1` | `hook_commands.rs` | 载体已存在（`hook_commands.rs`） |
| Plugin | L2 进程外插件 | 写 `plugin.json` + 可执行体，stdio JSON-RPC 2.0 | `pony-plugin-v1` | `plugin_host/` | 载体已存在（`plugin_host/`） |

## 2. registry source 枚举（`tools.rs` → `ToolDescriptorSource`，4 个变体）

`builtin` · `mcp` · `skill` · `dynamic`

## 3. 已定义的运行期端口 trait（`tool_runtime.rs`，7 个）

`McpTransport` · `PrimitiveToolHandler` · `ProcessBackend` · `RuntimeClock` · `SandboxBackend` · `ToolDispatcher` · `WebResolver`

## 4. 契约版本（`docs/contracts/*.v1.schema.json`）

| 契约 | version |
|---|---|
| tool-descriptor | `pony-tool-descriptor-v1` |
| hooks | `agent-hooks-v1` |
| mcp-config | `mcp-config-v1` |
| skill-manifest | `pony-skill-manifest-v1` |
| plugin | `pony-plugin-v1` |

## 5. Hook 钩子点（`hooks.rs` → `TurnHookPoint`，19 个）

`turn_prepare_start` · `turn_prepare_end` · `context_build_start` · `context_build_end` · `model_call_start` · `model_response_end` · `tool_call_start` · `tool_call_end` · `checkpoint_persist_start` · `checkpoint_persist_end` · `turn_finalize_start` · `turn_finalize_end` · `planner_turn_preflight` · `planner_tool_selection` · `planner_graph_decision` · `capability_resolve` · `skill_tool_actions_resolve` · `capability_source_ingress` · `skill_source_ingress`
