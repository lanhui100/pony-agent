# ACP 会话生命周期 + additionalDirectories 契约矩阵（backlog B179 / B181 / task-22）

> 状态：契约冻结（类型 + 入口 + 空桩 + 错误降级）。Owner：Protocol Agent。
> 真源：官方 v1 schema（`/tmp/acp-probe/schema-v1.json`）；真机实测 opencode 2.2.7 / agy 1.5.9（Lead 2026-11-15 探针结论）。
> 写域：`crates/pony-agent-core/src/acp/`、`.dev-team/`。

## 1. 签名清单（path:line）

| 位置 | 内容 |
|---|---|
| `acp/protocol.rs:30-36` | 方法常量：`SESSION_LIST` / `SESSION_LOAD` / `SESSION_RESUME` / `SESSION_CLOSE` / `SESSION_DELETE` / `SESSION_FORK`（fork 仅 opencode） |
| `acp/protocol.rs:38-39` | `AUTHENTICATE` / `LOGOUT`（本轮只做类型 + 不支持时降级，不做交互式登录） |
| `acp/protocol.rs:1015 / 1045` | `ListSessionsRequest { cwd?, cursor?, limit?, _meta? }` / `ListSessionsResponse { sessions: Vec<SessionInfo>, next_cursor?, _meta? }`（`next_cursor` 缺省 = 无更多页） |
| `acp/protocol.rs:1029` | `SessionInfo { session_id, cwd, additional_directories, title?, updated_at?, _meta? }`（agy 只列已落盘会话） |
| `acp/protocol.rs:1056 / 1098` | `LoadSessionRequest { cwd, session_id, mcp_servers, additional_directories?, _meta? }` / `LoadSessionResponse { session_id: Option<String>, config_options?, modes?, _meta? }` —— **`session_id` 必须 Option**（opencode 不回，agy 回超集） |
| `acp/protocol.rs:1112 / 1126` | `ResumeSessionRequest { cwd, session_id, mcp_servers?, additional_directories?, _meta? }` / `ResumeSessionResponse { config_options?, modes?, _meta? }` |
| `acp/protocol.rs:1138 / 1151` | `CloseSessionRequest { session_id }` / `DeleteSessionRequest { session_id }`（+ 空响应类型；`session/fork` 仅常量，类型留 P2） |
| `acp/protocol.rs`（SessionModeState / SessionMode） | `Load/Resume` 响应的 `modes` 结构：`current_mode_id` + `available_modes[{id,name,description?}]` |
| `acp/protocol.rs:1164 / 1176` | `AuthenticateRequest { method_id }` / `LogoutRequest {}`（类型冻结，不实现登录流程） |
| `acp/transport.rs:168 / 206` | `AcpTransportError::Unsupported(String)`（新错误码 `acp_method_not_supported`）；`:921` 在 `extract_result` 中把 JSON-RPC `-32601` **先于认证归类**映射为 `Unsupported`（不再是通用 `acp_agent_error`） |
| `acp/client.rs:470` | **additive** `new_session_with_dirs(cwd, mcp_servers, additional_directories: &[PathBuf])`；`:481` `new_session_inner(..., Option<Vec<String>>)`；既有 `new_session` 等价 `None` |
| `acp/client.rs:657` | `canonicalize_additional_directories(&[PathBuf]) -> Result<Vec<String>, AcpTransportError>`：绝对路径 + 禁 glob（`* ? [ ] { }`）+ 存在且是目录 + `canonicalize` 归一 + 去重保序；失败 ⇒ `AcpTransportError::Spawn`（`acp_additional_directories_invalid`，fail-closed） |
| `acp/session.rs:436 / 463` | `spawn_with_full_options`（既有，等价 `&[]`）/ **additive** `spawn_with_full_options_and_dirs(..., additional_directories: &[PathBuf])`：能力声明在 connect 前定稿、handler 在 `new_session` 前装配，`additionalDirectories` 经 client 校验 |
| `acp/session.rs:530 / 536` | `spawn_for_cli`（既有，等价 `&[]`）/ **additive** `spawn_for_cli_with_dirs(spec, cwd, additional_dirs)`：CLI 宿主形态透传 |
| `acp/session.rs:595 / 608 / 619` | `list_sessions(&self, ListSessionsRequest)` / `load_session(&self, LoadSessionRequest)` / `close_session(&self, CloseSessionRequest)` —— 体 `unimplemented!()`（executor 落地） |

## 2. 断言面

| ID | 断言 |
|---|---|
| L1 | **list 分页**：`ListSessionsRequest{limit,cursor}` 原样透传；响应 `sessions` 数组 + `nextCursor` 缺失 ⇒ 无更多页；`cwd` 过滤透传；agy 仅返回已落盘会话不报错；`-32601` ⇒ `Unsupported`（能力降级）。 |
| L2 | **load 两形态**：①响应不含 `sessionId`（opencode：仅 configOptions/modes）⇒ `LoadSessionResponse.session_id == None` 且反序列化成功、调用方沿用请求 id；②响应含 `sessionId`+`models`+`modes`（agy 超集）⇒ `Some(id)`，未知 `models` 字段被容忍。 |
| L3 | **close 幂等**：对不存在/已关闭的 session 回 `{}` ⇒ `Ok(())`；`-32601` ⇒ `Unsupported`（不 panic、不当作会话故障）。`resume` 对不存在会话收到 `-32603 … load it with session/load` ⇒ 调用方可降级到 `load_session`（错误原文保留在 `Agent`/`Unsupported` 分类中）。 |
| L4 | **-32601 降级**：`session/delete` / `session/fork` 收到 `{"code":-32601,...}` ⇒ `AcpTransportError::Unsupported` + 错误码 `acp_method_not_supported`（**不是** `acp_agent_error`）；其他 JSON-RPC 码分类不变（`-32000`/`-32603+auth` ⇒ AuthRequired，其余 ⇒ Agent）。 |
| L5 | **additionalDirectories 校验**：相对路径拒绝；含 glob 元字符（`* ? [ ] { }`）拒绝；不存在/非目录/不可 canonical 化拒绝 ⇒ 均 `AcpTransportError::Spawn`（`acp_additional_directories_invalid`），**不静默丢弃**；合法目录被 canonical 化后出现在 `session/new.additionalDirectories`（绝对路径、去重、保序）。 |
| L6 | **责任归属**：校验在 client（`new_session_with_dirs`/`canonicalize_additional_directories`），agy 不校验也不被信任；既有 `new_session`（无 dirs）报文与 B181 前逐字节一致（不出现 `additionalDirectories`）。 |
| L7 | **无回归**：`spawn_for_cli` / `spawn_with_full_options` / `spawn` / `spawn_with_mcp` 签名与行为不变；能力声明公式与 permission/fs/terminal 装配顺序不变；`Authenticate`/`Logout` 类型可 round-trip（不实现登录）。 |

## 3. 边界与已知限制

- `session/fork` 仅冻结方法常量（实测仅 opencode 支持）；请求/响应类型与 CLI 入口留 P2。
- `session/resume` 的「不存在 ⇒ 降级 load」是**调用方策略**：本层只把 `-32603` 归为 `Agent`（保留原文），不自动重试（避免双写会话状态的隐式行为）。
- `additionalDirectories` 的字节级 wire 形态由 `SessionNewRequest.additional_directories: Option<Vec<String>>` 决定；空列表不发送该字段（等价于无扩展目录）。
- 本批 `list_sessions` / `load_session` / `close_session` 为签名空桩（无调用点，不 panic 生产路径），由 executor 落地。

## 4. 门禁收据

- `cargo check -p pony-agent-core --lib` → **Exit 0**（0 warning）；`npm run cargo:check:shared` → **Exit 0**（`Finished dev profile ... in 36.76s`）。
- 未改 `acp/config.rs`、`acp/handlers.rs` 与 `agent/*`。
