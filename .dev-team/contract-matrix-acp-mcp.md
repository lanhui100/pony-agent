# ACP mcpServers 契约矩阵（backlog B174 / task-10）

> 状态：契约冻结（类型 + 转换 + additive 入口）。Owner：Protocol Agent。真源：`docs/architecture/acp-client-integration.md` §1/§4；Lead 2026-11-14 裁决（三态 + env fail-closed）。
> 写域：`crates/pony-agent-core/src/acp/`、`.dev-team/`。测试属 Test Agent；`agent/mcp_config.rs`、`agent/mcp_client.rs` 只读复用。

## 1. 签名清单（path:line）

| 位置 | 内容 |
|---|---|
| `acp/config.rs:65` | `AcpAgentSpec.mcp_servers: Option<Vec<AcpMcpServerSpec>>`，`#[serde(default, rename="mcpServers", skip_serializing_if="Option::is_none")]`：**None=inherit / Some(vec![])=显式禁用 / Some(list)=仅用条目** |
| `acp/config.rs:285` | `AcpMcpServerSpec { name, transport, auto_approve? }`（camelCase；`autoApprove` 是 Pony 策略提示，**不进 wire**） |
| `acp/config.rs:299` | `AcpMcpTransportSpec`（`type` 判别）：`stdio{command,args,env:BTreeMap}` / `http{url,headers}` / `sse{url,headers}` |
| `acp/config.rs:363` | `resolve_mcp_servers(spec) -> Result<Vec<McpServer>, AcpConfigError>`：三态解析；`workspace_root` = `spec.default_cwd`，缺省进程 cwd |
| `acp/config.rs:378` | `resolve_mcp_servers_in(spec, workspace_root)`（additive）：会话路径版本，inherit 用真实 session `cwd` |
| `acp/config.rs:394` | `inherit_pony_mcp_servers`（私有）：`load_mcp_config(workspace_root)` → 仅 `McpServer::Stdio` |
| `acp/config.rs:456` | `convert_explicit_mcp_server`（私有）：显式条目 → stdio/http/sse wire 三变体；env deny ⇒ fail-closed |
| `acp/session.rs:193` | `AcpAgentSession::spawn(spec,cwd)`：先 `resolve_mcp_servers_in(spec,cwd)` 再走 `spawn_with_mcp`（签名不变；配置错误 → `AcpTransportError::Unavailable`，消息保留 `acp_config_*` 码） |
| `acp/session.rs:203` | `spawn_with_mcp(spec,cwd,mcp_servers)`（additive）：显式 wire 列表 → `client.new_session(cwd, mcp_servers)`；与 `spawn` 同一路径 |
| 只读真源 | `agent/mcp_config.rs:198 load_mcp_config`；`McpServerEntry{id, spec:{command,args,env:Vec<(String,String)>}, source, requires_approval}`；`agent/mcp_client.rs:245 McpServerSpec`；deny 表 `agent/mcp_client.rs::is_denied_spec_env_key`（`MCP_SPEC_ENV_DENYLIST`） |

## 2. 转换断言面（Test Agent 只读本表写断言）

| ID | 断言 |
|---|---|
| M1 | **三态可区分**：`{...}` 无 `mcpServers` ⇒ `None`（inherit）；`"mcpServers": []` ⇒ `Some(vec![])`（禁用）；`"mcpServers": [{...}]` ⇒ `Some(list)`。序列化 `None` 时不落 `mcpServers` 键（旧清单逐字节兼容）。 |
| M2 | **inherit 只产 stdio**：inherit 结果全部是 `McpServer::Stdio`；Pony `McpServerEntry.id → name`、`spec.command/args/env` 逐字段等价（env 顺序保持 loader 顺序）；不出现 Http/Sse。 |
| M3 | **inherit 丢弃 Pony 专属字段**：`source`（Global/Workspace）、`requires_approval`、`trustReadOnlyHint`、`disabled` 条目、`McpConfigDiagnostic` 不进入 wire；loader 已解析的 env 值原样传出（`${env:VAR}` 已在 loader 侧解析）。 |
| M4 | **显式条目三变体透传**：stdio ⇒ `McpServer::Stdio{name,command,args,env:[{name,value}]}`；http ⇒ `McpServer::Http{name,url,headers,type:"http"}`；sse ⇒ `McpServer::Sse{...type:"sse"}`。`autoApprove` 不进 wire。显式 `[]` ⇒ 空 `Vec`，绝不回落 inherit。 |
| M5 | **env deny fail-closed**：显式条目任一 env 键命中 `is_denied_spec_env_key`（provider key / `SSH_AUTH_SOCK` / `LD_*` 等）⇒ 整个 `resolve_mcp_servers` 返回 `Err(AcpConfigError::Invalid)`（`acp_config_invalid`），**不得静默过滤/透传**；错误消息只含键名，绝不含值。 |
| M6 | **diagnostics 不得吞**：inherit 的每条 `McpConfigDiagnostic` 以 `code`+`server`+`message` 记 stderr（前缀 `[acp][config]`）；文件级致命诊断（`invalid_json`/`invalid_root`/`unsupported_schema_version`/`invalid_mcp_servers`，`server==None`）⇒ fail-closed 报错，不降级为空列表。env 值绝不回显。 |
| M7 | **错误码不新增**：仅用 `AcpConfigError` 既有 4 码 —— `acp_config_invalid`（语义/deny fail-closed）、`acp_config_parse`（inherit JSON 层）、`acp_config_io`（取当前目录失败）、`acp_config_agent_not_found`（与本函数无关）。session 边界把 `acp_config_*` 映射为 `AcpTransportError::Unavailable`，消息保真。 |
| M8 | **既有签名不破**：`spawn(spec,cwd)` 签名与调用点不变；缺省 spec 走 inherit 且不 panic；`spawn_with_mcp` 为 additive，显式列表原样下发；`new_session(cwd, mcp_servers)` 保持既有签名。 |

## 3. 边界与已知限制

- **http/sse 仅 wire 透传**：Pony 不启动本地进程、不做 URL 校验之外的健康检查；能力门由外部 agent 的 `mcpCapabilities` 决定（opencode `{http:true,sse:false}`，agy 未文档化 ⇒ 契约按 `false` 保守）。
- **inherit 只读 Pony 配置面**：项目层默认不可信（`PONY_MCP_TRUST_WORKSPACE` 才采信），与既有一致；不扩权。
- **相对路径**：`spec.default_cwd` 已由 `validate_spec` 要求绝对路径；`workspace_root` 传 session `cwd`（transport 已校验绝对路径）。
- **测试字面量衍生写入**：`AcpAgentSpec` 新增字段使 `crates/pony-agent-core/tests/acp_*.rs` / `wave4_4_acceptance.rs` 的 struct 字面量缺字段（E0063），需 Test Agent 补 `mcp_servers: None`（不在本批写域，lib 门禁不受影响）。

## 4. 门禁收据

- `cargo check -p pony-agent-core --lib` → **Exit 0**；`npm run cargo:check:shared` → **Exit 0**（`Finished dev profile ... in 29.69s`）。
- 本批未改 `agent/mcp_config.rs` / `agent/mcp_client.rs` / `acp/protocol.rs` / `acp/client.rs`（只读复用）。
