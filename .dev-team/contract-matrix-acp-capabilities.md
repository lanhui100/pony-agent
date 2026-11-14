# ACP fs/terminal 能力声明契约矩阵（backlog B175 / task-11）

> 状态：契约冻结（能力声明 additive 入口 + handler 骨架）。Owner：Protocol Agent。
> 真源：`docs/architecture/acp-client-integration.md` §3.2.4 / §3.2.5 / §4 P1 表；Lead 2026-11-14 裁决（initialize 第一条 ⇒ caps 必须在 connect 前决定）。
> 写域：`crates/pony-agent-core/src/acp/`、`.dev-team/`；`agent/path_permission.rs`、`agent/process.rs` 只读复用。

## 1. 声明公式（单一真源）

```
caps.fs       = spec.capabilities_hint.fs       && has_fs_handler
caps.terminal = spec.capabilities_hint.terminal && has_terminal_handler
```

- `capabilities_hint` 是**上限**：hint=false 时即使宿主有 handler 也不声明；
- `has_*` = connect **之前**已构造、握手后一定会装配的 handler；无真实 handler 的能力不得声明；
- 声明在 `initialize`（握手第一条）固化，握手后 `install_*_handler` **不追认**新能力。

## 2. 签名清单（path:line）

| 位置 | 内容 |
|---|---|
| `acp/client.rs:321` | `pub struct AcpClientCapabilities { pub fs: bool, pub terminal: bool }`（`Default` = 全 false；`NONE` 常量） |
| `acp/client.rs:340` | `connect(spec,cwd)`：等价 `connect_with_capabilities(.., AcpClientCapabilities::NONE)`（既有行为不变） |
| `acp/client.rs:356` | `connect_with_capabilities(spec,cwd,capabilities)`：`initialize.clientCapabilities.fs/terminal` 由 caps 推导；`fs` ⇒ `{readTextFile:true,writeTextFile:true}`、`terminal` ⇒ `true`；协议版本 ≠1 仍 fail-closed `acp_protocol_error` |
| `acp/client.rs:534 / 543` | `install_fs_handler(&self, Arc<dyn FsHandler>)` / `install_terminal_handler(&self, Arc<dyn TerminalHandler>)`：仅替换对应槽位，保留 permission/另一槽位 |
| `acp/session.rs:53` | `capabilities_for(spec, has_fs, has_terminal) -> AcpClientCapabilities`（纯函数，公式见上） |
| `acp/session.rs:213` | `spawn(spec,cwd)`：inherit MCP 解析后走 `spawn_with_mcp` |
| `acp/session.rs:223` | `spawn_with_mcp(spec,cwd,mcp_servers)`：等价 `spawn_with_mcp_and_handlers(.., None, None)`（既有行为：不声明、不装配） |
| `acp/session.rs:244` | **NEW（amend）** `spawn_with_mcp_and_handlers(spec,cwd,mcp_servers, fs: Option<Arc<dyn FsHandler>>, terminal: Option<Arc<dyn TerminalHandler>>)`：先 `capabilities_for(spec, fs.is_some(), terminal.is_some())` → `connect_with_capabilities`；握手成功后、`new_session` 前装配；声明 true 而 handler 缺失 ⇒ `AcpTransportError::Unavailable`（`acp_capability_inconsistent`，fail-closed） |
| `acp/session.rs:291 / 296` | `install_fs_handler(&self, Arc<dyn FsHandler>)` / `install_terminal_handler(&self, Arc<dyn TerminalHandler>)`：转发 client 槽位（只对已声明能力生效） |
| `acp/handlers.rs:42 / 55 / 66` | `WorkspaceFsHandler::new(workspace_root, Arc<AuthorizeStore>, PathPurpose)`；`impl FsHandler`（read/write，体 `unimplemented!()`）：read → `PathPermissionChecker::classify(Read)`（内部 `canonicalize_for_read`）；write → `classify(Write)`（内部 `canonicalize_for_write`：词法拒 `..`/绝对逃逸 + 最近存在祖先校验）；越界/未授权 ⇒ `AcpHandlerError::Denied`（fail-closed） |
| `acp/handlers.rs:97 / 104 / 109` | `ManagedTerminalHandler::new(Arc<ProcessManager>)`；`impl TerminalHandler`（create/output/wait_for_exit/kill/release，体 `unimplemented!()`）：create→`ProcessBackend::start`、output→`poll`、wait_for_exit→有界 poll、kill→`kill_if_exists`、release→`kill_if_exists`+条目回收；**禁止裸 `std::process`** |
| `acp/mod.rs:16` | `pub mod handlers;` |

## 3. 断言面（Test Agent 只读本表写断言）

| ID | 断言 |
|---|---|
| C1 | **声明矩阵 4 组合**：`initialize` 请求的 `clientCapabilities` 与 `capabilities_for(hint, has_fs, has_terminal)` 一致 —— (F,F)(F,T)(T,F)(T,T) 四组；hint=false + handler 存在 ⇒ wire 仍为 `fs:null`/`terminal:null`（上限语义）。 |
| C2 | **hint 上限**：`spec.capabilities_hint = {fs:false, terminal:false}` 时，即使传 `AcpClientCapabilities{fs:true,..}` 给 `connect_with_capabilities`，wire 声明也必须来自 session 公式（hint=false ⇒ 不声明）；直连 API 的 caps 是「上限内选择」，不得绕过 hint。 |
| C3 | **未声明能力不接受反向请求**：caps.fs=false 时 agent 发 `fs/read_text_file` ⇒ JSON-RPC error（`-32601`/handler 未装配 fail-closed），**不得**执行文件 I/O；terminal 同理。 |
| C4 | **fs 越界拒绝**：`WorkspaceFsHandler::read/write` 对 `../` 逃逸、workspace 外绝对路径、符号链接逃逸 ⇒ `Err(AcpHandlerError::Denied)`，错误码保留 `permission_denied` / `outside_workspace_write_denied` / `requires_authorization` 语义；workspace 内 read/write 成功。 |
| C5 | **terminal 回收无残留**：`release`（或 `wait_for_exit` 后 release）使 `ProcessManager` 不再追踪该 handle；`kill` 幂等；kill/release 后无残留子进程（超时子进程按既有 `kill_if_exists` 回收）；`terminalId` 跨 session 伪造 ⇒ `NotFound`。 |
| C6 | **handler 装配 additive**：`install_fs_handler` / `install_terminal_handler` 只替换对应槽位，既有 permission handler 保留；未声明能力的会话注入 handler 不改变 wire 能力（不追认）。 |
| C7 | **既有行为等价**：`connect(spec,cwd)` 的 initialize 报文与 B175 前逐字节一致（fs/terminal 均不声明）；`spawn(spec,cwd)` / `spawn_with_mcp` 签名与行为不变（handler 全 None ⇒ 不声明）。 |
| C8 | **声明↔装配一致性**：`spawn_with_mcp_and_handlers` 之后，`caps.fs==true` ⇔ 会话 fs 槽位已装配同一个 `FsHandler`，`caps.terminal==true` ⇔ terminal 槽位已装配；声明 true 而 handler 为 None 时必须在 `connect` 前返回 `Unavailable`（不发起 initialize）。握手后注入（`install_*`）不得让未声明能力变为可接受反向请求。 |
| C9 | **该入口 caps 推导**：hint.fs=true + `fs=Some` ⇒ wire `fs:{readTextFile:true,writeTextFile:true}`；hint.fs=true + `fs=None` ⇒ wire `fs:null` 且入口不报错；hint.fs=false + `fs=Some` ⇒ wire `fs:null`（hint 上限，handler 装配不追认）；terminal 同理。 |

## 4. 边界与已知限制

- `ManagedTerminalHandler` 的 `output`/`wait_for_exit` 组合语义（stdout+stderr 拼接顺序、截断标记）以 `ProcessPollResult` 为准；`signal` 无独立字段，契约映射 `exitCode=None`。
- `WorkspaceFsHandler` 的 `tmp_dir` 约定为 `<workspace_root>/.tmp`（受控 tmp 布局，与 `tools/paths.rs` 一致）；多 workspace 场景由调用方按会话 root 构造。
- 本批 `handlers.rs` 只写骨架（体 `unimplemented!()`），由 executor 落地；`install_*` 为真实槽位写入（文件已实现，空桩会给已实现层引入 panic 回归）。
- `AcpAgentSpec` 新字段（B174）使既有测试字面量需补 `mcp_servers: None`（Test Agent 写域）。

## 5. 门禁收据

- `cargo check -p pony-agent-core --lib` → **Exit 0**（0 warning）；`npm run cargo:check:shared` → **Exit 0**（`Finished dev profile ... in 24.66s`）。
- 未改 `agent/path_permission.rs`、`agent/process.rs`、`acp/transport.rs`、`acp/protocol.rs`、`acp/config.rs`。

## 6. B178 真机修复登记（additive，2026-11-15）

| 项 | 真值源 | 语义 |
|---|---|---|
| 握手预算 | `acp/transport.rs` `ACP_HANDSHAKE_TIMEOUT`（NFR `acp.handshake_timeout_ms` = 30000） | `initialize` / `session/new` 用握手预算而非 `ACP_REQUEST_TIMEOUT`（5s）；真机 `agy acp` 冷启动可 >5s，5s 会误杀 |
| ACP 专属 env 扩展 | `acp/transport.rs` `ACP_CHILD_ENV_EXTRA_ALLOWLIST` + `apply_acp_child_env_extras` | 在 `apply_child_env` 之后补 `XDG_* / TERM / LANG / LC_ALL / NO_COLOR`（宿主存在才补）；**不改 MCP/插件共享名单**，每键仍过 `is_denied_env_key`（PATH/LD_*/DYLD_*/AWS_* 照旧拒绝） |
| agent error 归因 | `acp/transport.rs` `extract_result` + `request` 的 deadline 分支 | `-32000` / `-32603` / 文案含 auth/authenticate/unauthorized/login/credential/登录 ⇒ `AuthRequired`；其余 ⇒ `Agent`；deadline 到点时先非阻塞消费已入队帧（`drain_ready_responses`），已收到的 error 帧绝不改报 `acp_timeout` |
