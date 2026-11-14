# ACP P0 契约矩阵（backlog B149 / task-1）

> 状态：P0 空桩已冻结，待 P1 Executor 实现。Owner：Protocol Agent（唯一空桩/类型编写方）。
> 真源：`docs/architecture/acp-client-integration.md` §1/§3/§3.2/§4 P0 表/§6；官方 v1 `schema.json`。
> 写域：`crates/pony-agent-core/src/acp/`、`src/lib.rs`、`.dev-team/nfr-baseline.json`。`tests/` 属 Test Agent，`agent/tools/` 属 P1。

## 1. 文件与签名清单（path:line，行号=rustfmt 后）

| 位置 | 内容 |
|---|---|
| `crates/pony-agent-core/src/lib.rs:1` | `pub mod acp;`（模块首次参与编译） |
| `crates/pony-agent-core/src/acp/mod.rs:11-13` | `pub mod config; pub mod protocol; pub mod transport;`（`server` 已删，`client`/`session` 留给 P1） |
| `acp/protocol.rs:21` | `pub const PROTOCOL_VERSION: u32 = 1;` |
| `acp/protocol.rs:24-41` | `method::{INITIALIZE, SESSION_NEW, SESSION_PROMPT, SESSION_CANCEL, SESSION_UPDATE, SESSION_REQUEST_PERMISSION, FS_*, TERMINAL_*}` 方法名常量 |
| `acp/protocol.rs:183 / 194` | `InitializeRequest { protocol_version: u32, client_capabilities, client_info }` / `InitializeResponse { protocol_version, agent_capabilities, agent_info, auth_methods }` |
| `acp/protocol.rs:88 / 118` | `ClientCapabilities { fs, terminal, auth }`（`FileSystemCapabilities{read_text_file,write_text_file}`、`AuthCapabilities{terminal}`）/ `AgentCapabilities { load_session, prompt_capabilities, mcp_capabilities, session_capabilities }` |
| `acp/protocol.rs:281 / 292` | `SessionNewRequest { cwd, mcp_servers, additional_directories? }` / `SessionNewResponse { session_id }` |
| `acp/protocol.rs:299 / 307` | `PromptRequest { session_id, prompt: Vec<ContentBlock> }` / `PromptResponse { stop_reason }` |
| `acp/protocol.rs:314` | `StopReason` 5 变体：`end_turn / max_tokens / max_turn_requests / refusal / cancelled` |
| `acp/protocol.rs:334` | `ContentBlock` 5 变体（`type` 判别）：`text / image / audio / resource_link / resource`，含 `EmbeddedResourceResource{text,blob}` |
| `acp/protocol.rs:394 / 402` | `SessionNotification { session_id, update }` / `SessionUpdate` 8 变体（`sessionUpdate` 判别）：`user_message_chunk / agent_message_chunk / agent_thought_chunk / tool_call / tool_call_update / plan / available_commands_update / current_mode_update` |
| `acp/protocol.rs:565 / 614 / 621` | `RequestPermissionRequest { session_id, tool_call, options }` / `RequestPermissionResponse { outcome }` / `RequestPermissionOutcome::{selected{optionId}, cancelled}` |
| `acp/protocol.rs:574 / 595` | `ToolCallUpdate` / `PermissionOption { option_id, name, kind }`（`allow_once/allow_always/reject_once/reject_always`） |
| `acp/protocol.rs:634 / 653` | `ReadTextFileRequest{session_id,path,line?,limit?}` / `WriteTextFileRequest{session_id,path,content}` + 对应 Response |
| `acp/protocol.rs:668-748` | `CreateTerminalRequest/Response`、`TerminalOutputRequest/Response`、`TerminalExitStatus`、`WaitForTerminalExit*`、`KillTerminal*`、`ReleaseTerminal*` |
| `acp/protocol.rs:325` | `CancelNotification { session_id }`（notification，无 id） |
| `acp/config.rs:25` | `AcpAgentSpec { id, label, command, args, env, default_cwd?, capabilities_hint }` |
| `acp/config.rs:20 / 113` | 文件名 `acp-agents.json` + `agents_file_path()` = `<用户配置目录>/pony-agent/acp-agents.json`（与 providers.json 同级、独立文件） |
| `acp/config.rs:76` | `BUILTIN_AGENTS`：`opencode → (opencode, ["acp"])`、`antigravity → (agy, ["acp"])` |
| `acp/config.rs:118/123/128/133` | `load_agents` / `save_agents` / `builtin_agent_specs` / `resolve_agent` —— 体全部 `unimplemented!()`（P1） |
| `acp/transport.rs:132` | `AcpTransport`（child/stdin/入站帧/handler/stderr 环形缓冲/id 分配器/非法行计数/终止标记） |
| `acp/transport.rs:156/167/177/182/189/194` | `spawn(spec,cwd,extra_env)` / `request(method,params,deadline)` / `notify` / `set_handler` / `close` / `stderr_tail` —— 体全部 `unimplemented!()` |
| `acp/transport.rs:88` | `AcpInboundRequestHandler`（agent→client 请求分发 trait；未注入 ⇒ fail-closed，绝不 default allow） |
| `acp/transport.rs:94` | `AcpTransportError` 8 变体 → `acp_*` 错误码（§3.2.9） |

## 2. 协议不变量（红相断言面，Test Agent 只读此表写断言）

| ID | 断言 | 关键点 |
|---|---|---|
| A1 | `serde_json::to_value(InitializeRequest{protocol_version:1,..})["protocolVersion"]` 是 `Number(1)` 且 `as_u64()==Some(1)` | **整数**，不是 `"1"`/`"0.1"`；响应 `!=1` 时 fail-closed `acp_initialize_failed` |
| A2 | `session/cancel` 报文**无 `id`** 字段（notification），`method=="session/cancel"` | 误写成请求会让 agent 等待应答 |
| A3 | `session/update` 报文无 `id`，`params.sessionId` + `params.update.sessionUpdate` | 8 变体逐一 round-trip |
| A4 | `session/request_permission` 方向 = **agent→client**；应答体为 `{"outcome":{"outcome":"selected","optionId":"..."}}` 或 `{"outcome":{"outcome":"cancelled"}}`；transport 无 handler 时不得回 `selected` | 治理链唯一审批交汇点，默认拒绝 |
| A5 | 所有结构体字段 camelCase：`mimeType / toolCallId / optionId / sessionId / availableCommands / currentModeId / additionalDirectories` | 官方样例 JSON round-trip 防回归 |
| A6 | 枚举值 snake_case：5×`StopReason`、8×`sessionUpdate`、4×`PermissionOptionKind`、4×`ToolCallStatus`、10×`ToolKind` | 不得出现驼峰/别名 |
| A7 | `request` 超过 `ACP_REQUEST_TIMEOUT`(5000ms) → `AcpTransportError::Timeout`，错误码 `acp_timeout` | 不重试、不静默 |
| A8 | stdout 非法 JSON：连续 <3 行记 stderr 诊断并跳过（不 panic）；连续 ≥3 行 fail-closed `acp_protocol_error` | 阈值语义见 NFR |
| A9 | `close()` 在 `SPAWN_REAP_GRACE`(500ms) 后对 Unix 进程组 SIGKILL 且 `wait()` 回收；超时后无残留子进程 | 平台局限（Windows 无进程组）随 `plugin_host` 既有口径 |

## 3. NFR 数值（真值源：`.dev-team/nfr-baseline.json` 的 `acp` 段 + `transport.rs` 常量）

| 键 | 值 | 代码常量 |
|---|---|---|
| `acp.request_timeout_ms` | 5000 | `transport.rs:31` `ACP_REQUEST_TIMEOUT` |
| `acp.malformed_line_streak_limit` | 3 | `transport.rs:36` `MALFORMED_LINE_STREAK_LIMIT` |
| `acp.spawn_reap_grace_ms` | 500 | `transport.rs:41` `SPAWN_REAP_GRACE` |
| stderr 尾部行数 | 32 | `transport.rs:44` `STDERR_TAIL_LINES` |

> 注意：5000ms 是 **stdio 子进程会话级往返**，不是网络类 `external_call_timeout_ms(≤3000)`；schema 已按 additive-only 登记。

## 4. 空桩禁止实现项（P0 边界，越界即拒收）

- `transport.rs`：禁止出现 `Command`/`spawn` 调用、env 白名单注入、进程组创建、reader 线程、id 配对、kill/wait —— 全部 P1。
- `config.rs`：禁止任何文件读写、JSON 解析、默认合并 —— 全部 P1。
- `protocol.rs`：禁止 I/O、会话状态机、错误码映射；**serde 属性不允许被 P1 就地放宽**（camelCase/snake_case/判别键是冻结契约）。
- 禁止在 P0 新增 `acp/client.rs` / `acp/session.rs`（P1）或恢复 `acp/server.rs`（方向错误旧稿已删）。
- `crates/pony-agent-sdk/` 已整目录删除（无 Cargo.toml、零消费者）；恢复条件 = P2 ACP Host 模式落地。
- 函数体除 `unimplemented!()` 外不得有业务逻辑（本批仅 `#[allow(unused_variables)]` 与 serde derive/常量属契约）。

## 5. 门禁收据

- `npm run cargo:check:shared`（cwd=仓库根）→ **Exit 0**（`Finished dev profile ... in 12.15s`，原始输出见交付回报）。
- 既有漂移（非本批引入）：`python3 .dev-team/nfr-lint.py` 在本批前后均为 **27 errors**（当前 `nfr-baseline.json` 缺历史 required 字段如 `lock_scope_bounded`）；本批只做 additive，未改既有字段、未新增错误。
- 遗留：`protocol.rs` 官方样例 round-trip 测试待 P0-E/Test Agent 落 `tests/acp_client_contract.rs`；本批仅类型编译通过。
