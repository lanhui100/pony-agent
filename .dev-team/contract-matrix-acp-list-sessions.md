# ACP CLI `--list-sessions` 契约矩阵（backlog B185 / task-31）

> 状态：契约冻结（additive 入口 + CLI 参数面 + JSONL 输出面 + usage）。Owner：Protocol Agent；执行链路 B185-E 接线。
> 真源：`session/list` 已实现（`AcpAgentSession::list_sessions` / `ListSessionsResponse`）；Lead 2026-11-15 裁决（`--list-sessions` 不得建会话）。
> 写域：`crates/pony-agent-core/src/acp/`、`bin/cli.rs`、`.dev-team/`。

## 1. 签名清单（path:line）

| 位置 | 内容 |
|---|---|
| `acp/session.rs:602` | **additive** `connect_for_cli(spec, cwd, additional_dirs) -> Result<Self, AcpTransportError>`（签名空桩）：与 `spawn_for_cli_with_dirs` 同 caps/handlers 口径 → connect → 装配 handler → **不建会话**（禁 `session/new` / `session/load`）；返回对象 `session_id` 为空串占位，供 `--list-sessions` / 将来 `--close-session` 复用 |
| `acp/session.rs:709` | `list_sessions(&self, ListSessionsRequest)`（已实现；本 CLI 面复用，不另立路径） |
| `acp/client.rs:634` | `AcpAgentClient::list_sessions`（B179 已实现；`-32601` ⇒ `Unsupported` 归因） |
| `acp/protocol.rs:1015 / 1029 / 1045` | `ListSessionsRequest { cwd?, cursor?, limit? }` / `SessionInfo { session_id, cwd, additional_directories, title?, updated_at? }` / `ListSessionsResponse { sessions, next_cursor? }` |
| `bin/cli.rs:2505` | `AcpCliArgs` 新增 `list_sessions: bool` / `session_cursor: Option<String>` / `session_limit: Option<u32>`（--list 的 agent 清单语义不动） |
| `bin/cli.rs:2555` | 参数解析：`--list-sessions` / `--session-cursor <c>` / `--session-limit <n>`（n=0 或非整数 ⇒ 用法错误；cursor 原样透传） |
| `bin/cli.rs:2668` | 互斥（择一：**拒绝并给用法错误**）：`--list-sessions` + `-p/--prompt` ⇒ Usage(2)；`--resume` 同时出现 ⇒ stderr 提示忽略 |
| `bin/cli.rs:1279` | usage（`print_acp_usage`）：`--list-sessions` 说明（JSONL 形状 / nextCursor / -32601 / 与 `-p` 互斥 / 与 `--list` 区分），并列出 `--session-cursor` / `--session-limit` |
| `bin/cli.rs:2797` | `fn acp_list_sessions(spec, workspace, cursor, limit) -> i32`（契约桩）：JSONL 契约冻结在文档；**B185-E 接线前**显式 stderr `acp_unavailable` + ExitCode::Config(3)，绝不 panic、绝不伪造会话行 |

## 2. JSONL 输出（冻结）

1. 每会话**一行**：`{"sessionId":...,"cwd":...,"title"?:...,"updatedAt"?:...,"additionalDirectories"?:[...]}`
   （可选字段缺省时**不出现该键**，不写 `null`；字段名严格 camelCase）。
2. 若 `session/list` 响应带 `nextCursor`，**额外一行**：`{"nextCursor":"..."}`（在会话行之后）。
3. 空列表 ⇒ **Exit 0 且零行**（不打印 `[]`）。
4. `-32601` ⇒ **stderr** 输出 `acp_method_not_supported`（前缀 `[ponycli]`）+ **非 0** 退出（不静默空列表）。
5. stdout 只有 JSONL（无 banner/进度）；诊断一律 stderr。

## 3. 断言面

| ID | 断言 |
|---|---|
| L1 | **两行输出**：2 个会话 ⇒ stdout 恰好 2 行 JSON，字段与 `ListSessionsResponse.sessions` 一一对应（sessionId/cwd 必含；title/updatedAt/additionalDirectories 缺失时无该键）；退出码 0。 |
| L2 | **nextCursor 行**：响应含 `nextCursor` ⇒ 会话行之后**额外一行** `{"nextCursor":"..."}`；无 `nextCursor` ⇒ 不出现该行。 |
| L3 | **-32601 降级**：fake agent 对 `session/list` 回 `-32601` ⇒ stderr 含 `acp_method_not_supported`、退出码非 0、stdout 零行（**不得**当空列表）。 |
| L4 | **不建会话**：`--list-sessions` 报文序列只有 `initialize → session/list`；**绝不出现 `session/new`**（也不出现 `session/prompt`）。 |
| L5 | **空列表**：0 个会话 ⇒ Exit 0 且 stdout 零行；不打印 `[]`、不打印 nextCursor。 |
| L6 | **`--list` 语义未混淆**：`--list` 仍只列 agent 清单（JSONL `{id,label,command,args,builtin}`）；`--list` + `--list-sessions` ⇒ Usage(2)（语义不同，不得混用）；`--list-sessions` 必须显式 `--agent`（缺 ⇒ Usage）；与 `-p` 混用 ⇒ Usage(2)。 |

## 4. 边界与已知限制

- 本批为契约桩：`connect_for_cli` 与 `acp_list_sessions` 的执行链路在 B185-E 接线；接线前 `--list-sessions` 显式非 0（`acp_unavailable`），**不会**回落到建新会话。
- `session/list` 的 `cwd` 过滤：CLI 不自动带 `--workspace`（保持 agent 全量列表语义）；如需过滤由 B185-E 决定是否透传 `cwd`，本契约不预设。
- `--session-cursor` 是不透明 token：CLI 只透传，不解析、不校验格式。

## 5. 门禁收据

- `cargo check -p pony-agent-core --lib` → **Exit 0**；`npm run cargo:check:shared` → **Exit 0**（`Finished dev profile ... in 22.89s`）。
- 未改 `acp/client.rs`、`acp/protocol.rs`、`acp/transport.rs`（复用既有实现）。
