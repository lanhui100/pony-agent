# ACP CLI --resume + SIGPIPE 契约矩阵（backlog B184 / task-28）

> 状态：契约冻结（additive 入口 + 策略登记）。Owner：Protocol Agent。
> 真源：Lead 2026-11-15 裁决（EPIPE 实测 exit 101；`--resume` 当前固定报 `acp_unavailable`）。写域：`crates/pony-agent-core/src/acp/`、`.dev-team/`。

## 1. 签名清单（path:line）

| 位置 | 内容 |
|---|---|
| `acp/session.rs:593` | **additive** `pub fn spawn_for_cli_resume(spec: &AcpAgentSpec, cwd: &Path, session_id: &str, additional_dirs: &[PathBuf]) -> Result<Self, AcpTransportError>`（签名空桩，体 `unimplemented!()`）：connect（handlers/caps 与 `spawn_for_cli_with_dirs` 同口径）→ **跳过 `session/new`** → `client.load_session(LoadSessionRequest{cwd, session_id, mcp_servers, additional_directories})` → session_id 同步 |
| `acp/session.rs:536` | 参照入口 `spawn_for_cli_with_dirs`（handlers/caps 装配口径真源；resume 复用同一套，禁止另写一套能力推导） |
| `acp/client.rs:656` | `AcpAgentClient::load_session(&mut self, LoadSessionRequest)`（B179 已实现）：两形态兼容（响应缺 `sessionId` ⇒ 沿用请求 id + stderr 诊断）、`additionalDirectories` canonical 化/拒 glob |
| `acp/protocol.rs:1056` | `LoadSessionRequest { cwd, session_id, mcp_servers, additional_directories?, _meta? }` —— 字段已齐（B179/B181），**无需 additive 补齐** |
| `acp/transport.rs:168` | `AcpTransportError::Unsupported(String)`（`acp_method_not_supported`）：`-32601` 由 `extract_result` 先于认证归类降级；resume 的「agent 不支持 load」走此码 |
| `bin/cli.rs:1959` | **SIGPIPE 策略（executor 实现，不需契约定义）**：Unix 在 `main` 起始 `#[cfg(unix)] libc::signal(libc::SIGPIPE, libc::SIG_DFL)`，恢复默认处置；`libc` 已是依赖 |

## 2. 断言面

| ID | 断言 |
|---|---|
| R1 | **resume 成功（不新建）**：`spawn_for_cli_resume` 对 fake agent 的报文序列为 `initialize → session/load`，**绝不出现 `session/new`**；`session/load` 参数含请求 `sessionId`、`cwd`（绝对路径）、`mcpServers`（`resolve_mcp_servers_in` 结果）、canonical 化后的 `additionalDirectories`。 |
| R2 | **不支持降级（绝不静默新建）**：① `initialize.agentCapabilities.loadSession == false` ⇒ `AcpTransportError::Unsupported`；② agent 对 `session/load` 回 `-32601` ⇒ `Unsupported`（`acp_method_not_supported`）；③ 协议版本/响应形态不符 ⇒ `Protocol`。三种情况都**不得**回落 `session/new`、不得返回一个新会话 id。 |
| R3 | **缺 sessionId 沿用请求 id**：`session/load` 响应只有 `configOptions`/`modes`（opencode 形态）⇒ 会话 `session_id()` == 请求传入的 `session_id`（来自响应超集时以响应为准）；不得伪造/生成新 id。 |
| R4 | **additionalDirectories 同口径**：resume 的 dirs 与 `spawn_for_cli_with_dirs` 同一校验（绝对路径、禁 glob、存在且目录、canonical 化、去重保序）；非法 ⇒ `AcpTransportError::Spawn`（fail-closed）。 |
| R5 | **SIGPIPE 策略**：Unix `ponycli ... \| head -1` 在 consumer 提前关管道时不得 panic/exit 101（恢复 `SIG_DFL` 后进程按信号终止或不因写失败 panic）；非 Unix 不受影响。 |
| R6 | **无回归**：`spawn_for_cli` / `spawn_for_cli_with_dirs` / `load_session` 既有签名与行为不变；caps/permission 装配顺序与 B175/B161 契约一致。 |

## 3. 边界与已知限制

- 本批 `spawn_for_cli_resume` 为签名空桩（无调用点，不 panic 生产路径），由 executor 落地；CLI `--resume` 在 executor 接线前仍报 `acp_unavailable`（预期）。
- resume 前置判定 `agentCapabilities.loadSession`：`initialize` 响应缺该字段按 `false`（既有 §6 风险表口径）⇒ 直接 `Unsupported`，不尝试 `session/load`。
- `mcpServers` 在 resume 路径同样走 `resolve_mcp_servers_in`（inherit/显式三态不变），不引入第二套配置解析。

## 4. 门禁收据

- `cargo check -p pony-agent-core --lib` → **Exit 0**（0 warning）；`npm run cargo:check:shared` → **Exit 0**（`Finished dev profile ... in 15.56s`）。
- 未改 `acp/client.rs`、`acp/protocol.rs`、`acp/transport.rs`、`bin/cli.rs`（SIGPIPE 由 executor 落地）。
