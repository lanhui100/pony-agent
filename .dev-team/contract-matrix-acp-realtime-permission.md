# ACP permission 注入点 + update 实时性契约矩阵（backlog B161 / B162 / task-14）

> 状态：契约冻结（类型 + 入口 + 实时语义文档）。Owner：Protocol Agent。
> 真源：`docs/architecture/acp-client-integration.md` §3.2.3（permission）/ §3.2.6（取消）/ §3.2.7（事件映射）；Lead 2026-11-15 裁决。
> 写域：`crates/pony-agent-core/src/acp/`、`.dev-team/`。`agent/ask_control.rs`、`agent/dispatcher.rs`、`agent/turn_flow.rs` 只读复用。

## 1. 签名清单（path:line）

| 位置 | 内容 |
|---|---|
| `acp/session.rs:141` | `pub struct DispatcherPermissionHandler { dispatcher: Arc<GovernedDispatcher>, authorization: ControlRequestAuthorization }`；`new(...)` 与 `impl PermissionHandler::decide` 体 `unimplemented!()`（executor 落地） |
| `acp/session.rs:293` | `pub fn spawn_with_full_options(spec, cwd, mcp_servers, fs: Option<Arc<dyn FsHandler>>, terminal: Option<Arc<dyn TerminalHandler>>, permission: Option<Arc<dyn PermissionHandler>>) -> Result<Self, AcpTransportError>`（additive） |
| `acp/session.rs:270` | `spawn_with_mcp_and_handlers` 改为 `spawn_with_full_options(.., None)`（既有行为等价）；`spawn` / `spawn_with_mcp` 继续等价全 None |
| `acp/session.rs:382` | `pub fn prompt(&self, blocks, sink: &dyn TurnEventSink) -> ToolResult`：**实时**语义（旁路线程跑 prompt；读线程回调 → mpsc → 调用线程 `recv_timeout` 逐条 `emit_update`） |
| `acp/session.rs:175 / 182` | `UpdateForwarder`（`tx: mpsc::Sender<SessionUpdate>` + `warnings: Arc<Mutex<Vec<String>>>`）实现 `AcpNotificationHandler`：逐帧发送、通道断开即 prompt 终局；非 update/解析失败记 warning（不吞、不 panic） |
| `acp/session.rs:72` | `CANCEL_POLL_INTERVAL = 100ms`（`turn_cancel_poll_ms` 契约；`recv_timeout` 同时服务实时帧与取消轮询） |
| `acp/session.rs:113 / 115` | `FailClosedPermissionHandler`（兜底）：无 permission handler ⇒ 一律 `cancelled`（绝不 allow） |
| `acp/session.rs:519 / 762` | `resolve_permission`（既有显式往返入口）/ `await_permission_decision`（有界轮询 + 取消标志）—— B161 复用同一往返语义 |
| `acp/client.rs:84` | `PermissionHandler` trait（`decide(&RequestPermissionRequest) -> RequestPermissionResponse`） |
| `acp/client.rs:129 / 135 / 378` | `HandlerSlots { permission, fs, terminal, declared: AcpClientCapabilities }`；`declared` 是 fs/terminal 反向请求闸门（permission 无 wire 声明位，不参与闸门、始终有 fail-closed 兜底） |
| 只读往返面 | `agent/ask_control.rs:47 answer_ask(dispatcher, request_id, authorization)`；`agent/dispatcher.rs:969 approve_control_request` / `:997 cancel_control_request`；`agent/turn_flow.rs:19 TurnEventSink`（无 `Send+Sync`） |

## 2. 断言面

### P —— permission（B161）

| ID | 断言 |
|---|---|
| P1 | **正相**：prompt 期间 agent 发 `session/request_permission`，`DispatcherPermissionHandler` 把它注册为 `PendingControlRequestKind::Approval`；宿主 approve 后返回 `{outcome:{outcome:"selected",optionId}}`，且 `optionId` **∈ request.options**（不得越界编造）；Ask 面板/审批链能看到该请求（不再"一律 cancelled"） |
| P2 | **负相**：宿主 `cancel_control_request` ⇒ `cancelled`；宿主**无响应/超过 `acp.request_timeout_ms`(5000ms)** ⇒ `cancelled`；dispatcher 不可用 ⇒ `cancelled`；**任何路径都不得默认 allow** |
| P3 | **无 handler**：`spawn_with_full_options(.., permission=None)` 回落 `FailClosedPermissionHandler` ⇒ 一律 `cancelled`（与 B161 前语义一致）；非 permission 的 fs/terminal handler 装配与 B175 一致 |
| P4 | **装配顺序**：`initialize`（caps 由 hint∧handler 推导）→ 握手成功 → **同一时点**装配 permission/fs/terminal（`install_handlers` + `install_*`）→ `new_session`；不存在「new_session 期间反向请求无 handler」窗口；`HandlerSlots.declared` 闸门对 fs/terminal 生效（permission 始终有兜底，不被闸门拒绝） |

### R —— 实时性（B162）

| ID | 断言 |
|---|---|
| R1 | **时序**：fake agent 先发 `session/update` 再延迟回 prompt 响应；sink 必须在 **prompt 返回之前**收到 `turn:delta`/`turn:tool`（用 sink 侧 channel/时间戳断言），不得批量 flush 到 prompt 返回之后 |
| R2 | **完整/有序**：多帧 update 按到达顺序逐帧 emit 一次，不丢不重；非 update 通知与非法载荷记 warning（不 panic、不静默）；prompt 线程 panic ⇒ `Unavailable`（不传播 panic） |
| R3 | **选型**：实现必须为「旁路线程 + `mpsc` + 调用线程 `recv_timeout` 循环立即 `emit_update`」；`TurnEventSink` 不引入 `Send+Sync` 约束（冻结面不变） |

### S —— 取消（B162 不得改变 §3.2.6）

| ID | 断言 |
|---|---|
| S1 | `CancellationToken` 触发后仍须 **< 整轮预算** 返回 `status: aborted` + `tool_cancelled`（`recv_timeout` 循环内轮询，不受实时转发影响）；取消时立即发 `session/cancel` notification |
| S2 | stopReason 归约不变（5→4：`end_turn`→ok；`max_tokens`/`max_turn_requests`→ok+truncated；`refusal`→error `acp_agent_refused`；`cancelled`→aborted `tool_cancelled`） |

### N —— 无回归

| ID | 断言 |
|---|---|
| N1 | `spawn` / `spawn_with_mcp` / `spawn_with_mcp_and_handlers` 签名与行为等价（handler 全 None ⇒ 不声明、permission 兜底 cancelled）；`capabilities_for` 公式不变 |
| N2 | 实时转发不改变 `emit_update` 的字段映射（§3.2.7 表）；prompt 返回的 `ToolResult` 字段（status/error code/truncated）与 B162 前逐字段一致 |

## 3. 边界与已知限制

- **permission 无 wire 能力位**：ACP v1 `clientCapabilities` 不含 permission 开关，`request_permission` 恒可被 agent 发起 ⇒ 宿主必须始终持有 fail-closed 兜底 handler（`FailClosedPermissionHandler`），不能靠"未声明"拒收。
- **`TurnEventSink` 冻结不加 `Send+Sync`**：实时性由 mpsc 跨线程搬运实现；sink 的 `emit` 只在调用线程执行。
- **warning 面**：非 update 通知/非法 update 载荷不进 UI 事件流，只记 stderr + 随归约结果记账（不吞）。
- **B161 executor 落地前**：`DispatcherPermissionHandler` 为 `unimplemented!()` 骨架（无调用点，不 panic 生产路径）；`spawn_with_full_options` 装配的是真实 handler 或 fail-closed 兜底。

## 4. 门禁收据

- `cargo check -p pony-agent-core --lib` → **Exit 0**（0 warning）；`npm run cargo:check:shared` → **Exit 0**（`Finished dev profile ... in 35.05s`）。
- 未改 `acp/client.rs`、`acp/transport.rs`、`acp/protocol.rs`、`agent/ask_control.rs`、`agent/dispatcher.rs`、`agent/turn_flow.rs`。
