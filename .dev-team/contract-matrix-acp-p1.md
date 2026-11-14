# ACP P1 契约矩阵（backlog B150 / task-8）

> 状态：P1 空桩已冻结（signature-only），待 executor 实现。Owner：Protocol Agent。
> 真源：`docs/architecture/acp-client-integration.md` §3.2 / §4 P1 表 / §6 风险。测试真源：`.dev-team/acp-p0-test-design.md`。
> 写域：`crates/pony-agent-core/src/acp/`、`agent/tools/`、`.dev-team/`。测试仍属 Test Agent。

## 1. 签名清单（path:line，行号 = rustfmt 后）

| 位置 | 内容 |
|---|---|
| `acp/mod.rs:13-17` | `pub mod client; config; protocol; session; transport;`（P1 挂载 client/session） |
| `acp/client.rs:51` | `AcpHandlerError { Denied / NotFound / Internal }`（fs/terminal handler 失败形态） |
| `acp/client.rs:84 / 89 / 102` | 注入点 trait：`PermissionHandler::decide` / `FsHandler::{read_text_file,write_text_file}` / `TerminalHandler::{create,output,wait_for_exit,kill,release}` |
| `acp/client.rs:297` | `AcpAgentClient { transport, agent_id, session_id, agent_capabilities, handlers, prompt_budget: Mutex<Duration> }` |
| `acp/client.rs:323` | `connect(spec,cwd) -> Result<Self, AcpTransportError>`：spawn → initialize → `protocolVersion == 1` 否则 `AcpTransportError::Protocol`（`acp_protocol_error`，不重试不降级） |
| `acp/client.rs:381` | `new_session(&mut self, cwd, mcp_servers) -> Result<SessionNewResponse, _>`（cwd 绝对路径，transport 已校验） |
| `acp/client.rs:426` | **NEW** `set_prompt_budget(&self, budget: Duration)`：写入 `prompt_budget`（默认 `ACP_REQUEST_TIMEOUT`）；文档：prompt 出站请求用它，**不是**固定 5000ms 硬顶 |
| `acp/client.rs:435` | `prompt(&self, session_id, blocks, on_update: Arc<dyn AcpNotificationHandler>) -> Result<PromptResponse, _>`（deadline = `prompt_budget` 当前值，默认 5000ms） |
| `acp/client.rs:466 / 494` | `cancel(&self, session_id)`（发 `session/cancel` **notification**）/ `close(&mut self)` |
| `acp/client.rs:478` | `install_handlers(permission, fs?, terminal?)`；未注入 = 整块 fail-closed |
| `acp/session.rs:39 / 42` | `ACP_ERROR_AGENT_REFUSED="acp_agent_refused"` / `ACP_TOOL_CANCELLED="tool_cancelled"` |
| `acp/session.rs:52` | `AcpTurnOutcome { Ok / Truncated / Refused / Cancelled }`（§3.2.8 五 → 四归约） |
| `acp/session.rs:162 / 173` | `AcpAgentSession { client, prompt_gate: Mutex<()>, session_id, cancellation: Mutex<Option<Arc<CancellationToken>>>, pending_permissions }` |
| `acp/session.rs:182` | `spawn(spec,cwd)`：connect → install_handlers → new_session |
| `acp/session.rs:204` | **NEW** `set_turn_budget(&self, budget: Duration)`：转发 `client.set_prompt_budget`；文档：默认值 = `ACP_REQUEST_TIMEOUT` |
| `acp/session.rs:213` | **NEW** `set_cancellation_token(&self, token: Arc<CancellationToken>)`：写入既有 `cancellation` 槽位；文档：prompt 期间按 `turn_cancel_poll_ms`(≤100ms) 轮询，触发即 `client.cancel` + 归约 aborted/`tool_cancelled` |
| `acp/session.rs:227` | `prompt(&self, blocks, sink: &dyn TurnEventSink) -> ToolResult`（串行闸 + update 映射 + stopReason 归约；预算/取消见上两行） |
| `acp/session.rs:273 / 291` | `cancel(&self)`（cancel notification + 挂起 permission 全回 cancelled）/ `close(&mut self)` |
| `acp/session.rs:301` | `resolve_permission(&self, request, &GovernedDispatcher, &ControlRequestAuthorization)`：`ask_control::answer_ask` 往返，绝不默认 allow |
| `acp/session.rs:364 / 457` | `emit_update(&self, sink, &SessionUpdate)`（8 变体 → turn:delta/turn:tool）/ `reduce_stop_reason(StopReason) -> AcpTurnOutcome` |
| `agent/tools/mod.rs:278` | `pub(crate) const TOOL_ACP_DELEGATE: &str = "acp_delegate";` |
| `agent/tools/mod.rs:436` | `PRIMITIVE_DISPATCH_TABLE` 新增臂 → `router.execute_acp_delegate(call, context)`；表/`builtin_tools()` 双射 45 → 46 |
| `agent/tools/orchestration.rs:85` | `execute_acp_delegate(&self, call, context) -> ToolResult`（结构照抄 `execute_subagent`：spec 解析 → workspace root → tool.call span → session.prompt → 归约） |
| `agent/tools/names.rs:90 / 151` | canonical：`acp_delegate` / `AcpDelegate` / `acp.delegate` → `acp_delegate`；registry 别名 `acp.delegate` |
| `agent/tools/names.rs:199 / 292 / 377 / 446 / 510` | 执行策略（concurrent_safe + 120s 默认）/ 产品名小写透传 / `ToolKind::Composite` / `ToolExposure::ModelVisible` / 中文展示名「外部代理」 |
| `agent/tools/schema.rs:1130 / 1195` | `builtin_tools()` 新增 `acp_delegate`：`agent`+`prompt` 必填、`cwd?`/`timeoutMs?`、`additionalProperties:false`；contract_priority=100 |
| `agent/tools/contract_view_tests.rs:70/119/240/187/191/390` | 工具面清单与计数同步（`builtin_tools()` 46、模型可见面 39；`acp_delegate` 的 exposure/kind 断言） |
| `agent/provider/tests.rs:248/363/378/819` | provider payload 清单（模型面 39、尾部 `skill_load, acp_delegate`） |

## 2. 八条契约断言面（Test Agent 只读本表写断言；真源 §4「测试」表）

| ID | 断言 |
|---|---|
| P1-1 | `initialize` 的 `protocolVersion` 是整数 `1` 且被校验；响应 ≠1 ⇒ `acp_protocol_error`（fail-closed，不重试） |
| P1-2 | `session/new` 传**绝对** `cwd`；相对路径被拒（`acp_spawn_failed`/参数错误，plumbing 不得静默 canonicalize 相对路径） |
| P1-3 | `session/update` 的 `agent_message_chunk` → `turn:delta`(text)、`agent_thought_chunk` → `turn:delta`(reasoning_content)、`tool_call`/`tool_call_update` → `turn:tool` |
| P1-4 | `session/request_permission` 触发 `PendingControlRequestKind::Approval`，经 Ask 面板往返；**无 handler / 未决 ⇒ `cancelled`**，绝不默认 allow |
| P1-5 | `session/cancel` 后全部挂起 `request_permission` 收到 `cancelled`，prompt 以 `stopReason:cancelled` 结束 |
| P1-6 | 五个 stopReason 归约：`end_turn`→ok；`max_tokens`/`max_turn_requests`→ok+truncated；`refusal`→error `acp_agent_refused`；`cancelled`→aborted `tool_cancelled` |
| P1-7 | 子进程崩溃 → `acp_protocol_error`/`acp_unavailable`（非 panic）；stderr 尾部 32 行留有诊断 |
| P1-8 | `AcpAgentSession` 串行闸：并发两次 `prompt` 同一 session 必须排队（第二者阻塞或拒绝），不得交错 |
| P1-9 | **加预算不被 5s 硬顶**：`set_prompt_budget`/`set_turn_budget` 设 `>5000ms` 后，`client.prompt` 的 `transport.request` deadline 取该值（默认才是 5000ms）；`timeoutMs`/`--timeout-secs`/执行策略可真实生效，真实 agent >5s 一轮不被误杀 |
| P1-10 | **取消令牌联动**：`set_cancellation_token` 注入后，prompt 期间按 `turn_cancel_poll_ms`(≤100ms) 轮询；`is_cancelled()` ⇒ 发 `session/cancel` notification、挂起审批回 `cancelled`、本轮归约 `status: aborted` + `tool_cancelled`；未注入时沿用会话自建令牌 |

## 3. 空桩禁止实现项（P0 冻结面，越界即拒收）

- `client.rs` / `session.rs`：**由 P1 Executor 实现**（提交 `5948864b`/`8236d37d`）；本批 amend 为 additive 公开面（`set_prompt_budget` / `set_turn_budget` / `set_cancellation_token`），不得改既有公开签名。
- **不得修改**已实现的 `acp/transport.rs`、`acp/config.rs`、`acp/protocol.rs`（P0 已提交 c23b3f4b；契约冻结）。
- `execute_acp_delegate` 不得提前写业务逻辑；参数解析结构由 executor 在实现时新增（不改本签名）。
- 禁止绕过治理链：`request_permission` 必须走 `ask_control`；`fs/*` 必须走 `path_permission`；`terminal/*` 必须走 `ProcessManager`。
- 工具面变更必须同步 `contract_view_tests.rs`（工具面清单唯一真源）与 provider payload 测试，否则 lib 测试红。

## 4. 门禁收据与遗留

- `npm run cargo:check:shared` → **Exit 0**；`cargo test -p pony-agent-core --lib` → **1550 passed / 0 failed, Exit 0**。
- 计数变更：`builtin_tools()` 45 → 46；模型可见产品面 38 → 39（`acp_delegate` 尾插，KV-cache 前缀不变，仅尾部 +1）。
- **Lead 裁决（2026-11-14）**：`team_tool_surface_exact_count` **38 → 39** 已同步（`.dev-team/nfr-baseline.json` 值 39 + schema `const: 39`，保持精确值语义、未放宽为 minimum）；依据 `agent/tools/schema.rs` 的 `builtin_tools()` 与 `agent/provider/tests.rs` 同步后的实际模型可见面。注：该字段此前不在 instance 内属既有漂移，本次按裁决显式登记 39。
- **需 Lead 裁决**：`.dev-team/nfr-baseline.json` 的 `acp` 段不含 P1 的 prompt 预算；`transport::ACP_REQUEST_TIMEOUT`(5000ms) 已覆盖单请求，整轮 prompt 超时由 `timeoutMs`/执行策略承担，未新增键。
- **Gate-1 amend（2026-11-14）**：TIMEOUT — `client.prompt` 已改为读 `prompt_budget`（默认 5000ms），`set_prompt_budget`(`client.rs:426`)/`set_turn_budget`(`session.rs:204`) 为 additive 公开面；CONTRACT_BREAK — `set_cancellation_token`(`session.rs:213`) 写入既有 `cancellation` 槽位（`Mutex<Option<..>>`），`session.prompt`(`session.rs:227`) 已实现 in-prompt 轮询：`CANCEL_POLL_INTERVAL`=100ms(`session.rs:51`)、scoped thread(`session.rs:259`) 轮询 token → `client.cancel` → 归约 aborted/`tool_cancelled`。注：两文件在 amend 前已由 P1 Executor 实现（非空桩），故 setter 用真实赋值而非 `unimplemented!()`（避免给已实现层引入 panic 回归）。`execute_acp_delegate` 读 `context.cancellation` 的接线属 executor fix-round（orchestration.rs 不在本批写域）。
- **Gate-1 amend 验证**：`cargo test -p pony-agent-core --test acp_p1_blockers` → **2/2 passed**（P1-9 预算不被 5s 硬顶、P1-10 取消联动）；ACP 五个测试文件合计 **44 tests passed / 0 failed**；`npm run cargo:check:shared` → **Exit 0**。
