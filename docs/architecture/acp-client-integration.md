# Pony 接入外部 ACP Agent 的实现方案（Pony as ACP Client）

> 状态：调研 + 方案（未改代码）
> 核对日期：2026-11-13
> 协议真源：agentclientprotocol.com `protocol/v1`（Stable，`protocolVersion` 为整数 `1`）
>
> **执行账本（本方案拆成三条待办，按依赖顺序推进）**
>
> | 待办 | 阶段 | 对应本文 | 依赖 |
> |---|---|---|---|
> | `backlog:B149` | P0 协议层重建 | §3、§4 P0 表、§5 P0 阶段 | 无 |
> | `backlog:B150` | P1 会话驱动 + `acp_delegate` | §3.2、§4 P1 表、§6 风险 | B149 |
> | `backlog:B151` | P2 宿主形态 | §4 P2 表、§5 P2 阶段 | B149、B150 |

---

## 0. 现状核对（仓库实测）

| 事实 | 证据 |
|---|---|
| `crates/pony-agent-core/src/acp/` 存在 3 个文件（`mod.rs` / `protocol.rs` / `server.rs`） | `ls crates/pony-agent-core/src/acp/` |
| **该模块未参与编译**：`lib.rs` 只有 `pub mod agent;`，全仓 `acp::` 零命中 | `cat crates/pony-agent-core/src/lib.rs`；`grep -rn "crate::acp\|pony_agent_core::acp" --include=*.rs .` → 0 |
| 现有草稿是 **agent 侧（server）**，方向与本需求相反 | `server.rs` 处理 `initialize` / `session/new` / `session/cancel` / `session/request_permission` 的**入站**请求 |
| 草稿协议与 v1 不符：`protocolVersion: "0.1"`（字符串，官方是整数 `1`）；把 `session/request_permission` 当成 client→agent 的审批应答（官方是 **agent→client 的请求**） | `server.rs:57-62`、`server.rs:78-130` |
| `crates/pony-agent-sdk/` 只有 `src/lib.rs`，**无 `Cargo.toml`**，不在 workspace members ⇒ 也未参与构建 | `ls crates/pony-agent-sdk`；`Cargo.toml` members 仅 core + src-tauri |
| SDK 里 `PonyClient::spawn` 传 `acp` 子命令，但 CLI **无此子命令**（`SubCommand` 只有 `Run`/`Init`） | `crates/pony-agent-sdk/src/lib.rs:15-20`；`crates/pony-agent-core/src/bin/cli.rs:64-68` |
| 既有 ACP 测试是空壳：只断言字符串含 `"jsonrpc"` | `crates/pony-agent-core/tests/wave4_4_acceptance.rs:1-6` |
| 无任何 ACP 相关 ADR | `grep -rln "ACP" .agents/notes/` → 仅 2 条无关历史 |
| 核对当时无 ACP 相关 backlog 条目；现已拆为 B149（P0）/ B150（P1）/ B151（P2） | `node scripts/backlog.mjs list` |

**结论**：当前项目**没有任何可用的 ACP 实现**，只有一段未编译、协议方向相反的草稿。

---

## 1. 外部 Agent 的角色（已核对官方文档）

| Agent | 角色 | 启动方式 | 传输 | 备注 |
|---|---|---|---|---|
| **opencode** | ACP **agent（server）** | `opencode acp`（`args: ["acp"]`） | stdio，newline-delimited JSON-RPC，**ACP v1** | `--print-logs` 把日志改到 stderr（stdout 只走协议）；`--login` 走终端认证；capabilities：`loadSession:true`、`mcpCapabilities{http:true,sse:false}`、`promptCapabilities{embeddedContext:true,image:true}`、`sessionCapabilities{additionalDirectories,close,delete,fork,list,resume}`；`cwd` **必须绝对路径**；**一个 session 同时只能跑一个 prompt** |
| **Antigravity** | ACP **agent（server）** | `agy acp` | stdio，ACP **v1**，自称 `Antigravity ACP Server` | `agy acp --login`；认证复用 `agy` 缓存凭据；文档未逐项列出 capabilities ⇒ 按 baseline 保守处理 |

两者都是「被 Pony spawn 的子进程」，Pony 侧要做的是 **ACP Client**。

### 官方 v1 方法面（Pony 必须实现的）

**Client → Agent（Pony 发起）**
- `initialize` → `{protocolVersion: 1, clientCapabilities:{auth:{terminal}, fs:{readTextFile,writeTextFile}, terminal}, clientInfo:{name,title,version}}`；响应含 `protocolVersion`、`agentCapabilities`、`agentInfo`、`authMethods`
- `session/new` `{cwd, mcpServers}` → `{sessionId}`
- `session/prompt` `{sessionId, prompt: ContentBlock[]}` → `{stopReason, usage?}`
- `session/cancel`（**notification**）`{sessionId}` → agent 以 `stopReason: "cancelled"` 回应原 prompt
- 可选：`session/load`、`session/resume`、`session/fork`、`session/list`、`session/close`、`session/delete`、`authenticate`

**Agent → Client（Pony 必须应答/消费）**
- `session/update`（**notification**）：`agent_message_chunk` / `agent_thought_chunk` / `user_message_chunk` / `tool_call` / `tool_call_update` / `plan` / `current_mode_update` / `available_commands_update`
- `session/request_permission`（**请求**）`{sessionId, toolCall, options:[{optionId,name,kind}]}` → `{outcome:{outcome:"selected",optionId}}` 或 `{outcome:{outcome:"cancelled"}}`
- `fs/read_text_file` / `fs/write_text_file`（仅当 client 声明 capability）
- `terminal/create|output|wait_for_exit|kill|release`（仅当 `clientCapabilities.terminal=true`）

**stopReason 枚举**：`end_turn` / `max_tokens` / `max_turn_requests` / `refusal` / `cancelled`

**协议硬约束**：`session/cancel` 后 client **必须**对所有挂起的 `session/request_permission` 回 `cancelled`；agent 在回 prompt 响应前必须发完 pending update；stdout 只承载协议消息。

---

## 2. 架构选型

### 方案 A：作为 Provider 后端（`ProviderProtocol::AcpAgent`）
把外部 agent 当成一个"模型"接进 `ProviderClient` 家族。
- ✗ 语义不合：`ProviderClient` 是「messages + tools → ProviderDecision」的无状态单轮抽象，而 ACP 是**长连接会话型**协议（session 生命周期、双向请求、agent 自带工具）。
- ✗ 工具面冲突：ACP agent 自己执行工具，Pony 的 `ToolRouter` / 治理链 / 审批会被完全旁路，两套工具体系并存。
- ✗ 一旦引入，`is_anthropic()` / `is_openai_family()` 这类行为分叉位点全部要加第三分支。

### 方案 B（推荐）：作为**委派工具 + 子代理引擎**
Pony 模型通过工具 `acp_delegate` 把一段任务交给外部 ACP agent，core 侧 spawn 子进程、走完 ACP 会话、把 `session/update` 流映射成 `turn:tool` 增量，最终返回 tool result。
- ✓ 边界清晰：外部 agent 是**黑盒执行者**，工具即契约边界，与 `subagent` / `workflow` 同构。
- ✓ 天然复用治理链：审批（`ask_control`）、预算（`BudgetLedger`）、取消（`CancellationToken`）、`ToolExecutionContext` 全都能直接套。
- ✓ 不动 `ProviderProtocol` / `ProviderClient` / provider 配置 schema，零回归面。
- ✗ 不能让外部 agent 充当主模型（但对 opencode/agy 这类"自带 agent 循环"的东西，这恰恰是对的）。

### 方案 C（后续阶段）：ACP Host 模式
`ponycli acp` 子命令 + TUI 面板，把外部 agent 的会话**可视**渲染进 Pony（长驻会话、多轮对话、`session/load` 恢复）。
- 与 B 共用同一套 `acp::client` 协议层，只是宿主形态不同（工具调用 vs 交互式会话）。
- 需要新增"外部会话"记录面（Pony 会话库目前只存自有 turn）。

**决策：B 先行，协议层按 C 的需求设计**（协议层不假设"一次性任务"）。

---

## 3. 实现方案（分层）

```
┌─────────────────────────────────────────────────────────────┐
│ 宿主形态层                                                    │
│  A) acp_delegate 工具（ToolRouter 臂）  ← P1                  │
│  B) ponycli acp 子命令 / TUI 面板       ← P2                  │
└──────────────────────────┬──────────────────────────────────┘
                           │
┌──────────────────────────▼──────────────────────────────────┐
│ acp::session —— 会话驱动（AcpAgentSession）                   │
│  initialize → session/new → prompt（流式消费 update）→ cancel │
│  反向请求分发：request_permission / fs/* / terminal/*          │
└──────────────────────────┬──────────────────────────────────┘
                           │
┌──────────────────────────▼──────────────────────────────────┐
│ acp::transport —— stdio 双向 JSON-RPC 2.0                     │
│  出站请求（id 关联 + 迟到响应缓冲）/ 出站通知                   │
│  入站请求（分发到 handler，回写 result/error）/ 入站通知        │
│  env 白名单 + Unix 独立进程组 + stderr 环形缓冲                 │
└──────────────────────────┬──────────────────────────────────┘
                           │
┌──────────────────────────▼──────────────────────────────────┐
│ acp::protocol —— ACP v1 类型（serde，camelCase）              │
└─────────────────────────────────────────────────────────────┘
```

### 3.1 复用还是引入官方 crate？

| | 自研（复用 `PluginTransport` / `McpClient` 模式） | 引入 `agent-client-protocol` crate |
|---|---|---|
| 依赖面 | 0 新增（`serde_json` + `std::process` 已有） | 新增 `agent-client-protocol`（+ `tokio-util` compat + `async-trait`） |
| 异步模型 | 同步阻塞线程（与现有 `start_turn_stream` 同构） | tokio `LocalSet` + `spawn_local`（SDK 的 future 非 `Send`） |
| 协议维护 | 自己跟 v1 演进 | 官方 schema 跟版 |
| 与现有代码风格 | 一致（`PluginTransport` / `McpClient` 都是同步 stdio） | 需引入第二套并发模型 |

**推荐：P0 自研同步传输**（与 `plugin_host/transport.rs`、`mcp_client.rs` 完全同构，零新依赖、零并发模型冲突）；**同时把 `acp::protocol` 的类型形状对齐官方 schema**，将来若要换 crate，只换 transport 层。

理由：`agent-client-protocol` 的 client future 非 `Send`，而 Pony 的 `HostControlPlane` 是跨线程 `Arc` managed state、turn 跑在独立线程 —— 强行接入要额外维护一个专属 tokio runtime 线程 + `LocalSet`，收益不抵成本。

### 3.2 关键设计点

1. **会话串行化**：opencode 明确"一个 session 同时只能跑一个 prompt"。`AcpAgentSession` 内部加 `Mutex` 串行闸；并发委派走"每任务一个子进程"或"排队"，不并发打同一 session。
2. **cwd 必须绝对路径**：`session/new` 的 `cwd` 从 `ToolExecutionContext.workspace_root` 取，`canonicalize` 后校验（复用 `plugin_host::capability::resolve_workspace_dir`）。
3. **`request_permission` → 走 `ask_control`**：外部 agent 请求权限时，注册一条 `PendingControlRequestKind::Approval`，经既有 `turn:tool` / Ask 面板让用户决定，再回 `{outcome:{outcome:"selected",optionId}}`。**这是 Pony 治理链与外部 agent 的唯一审批交汇点**，不能默认 `allow`。
4. **`fs/*` → 走 `path_permission`**：声明 `clientCapabilities.fs.{readTextFile,writeTextFile}` 时，读走 `classify_path` + `AuthorizeStore`，写走 workspace 边界判定；**不声明**则该能力整块不下放（fail-closed，让 agent 自己在子进程里读）。
5. **`terminal/*` → 走 `ProcessManager`**：声明 `clientCapabilities.terminal=true` 时，`terminal/create` 落到 `agent::process::ProcessManager`（已有 kill/cancel/kill_after/drain_stats）。
6. **`session/cancel` 语义**：Pony 侧 `stop_turn` 被接受 → 对 ACP 发 `session/cancel` notification + 对所有挂起 `request_permission` 回 `cancelled`。
7. **事件映射**（`session/update` → Pony 事件）：
   | ACP update | Pony 事件 |
   |---|---|
   | `agent_message_chunk` | `turn:delta`（`text`） |
   | `agent_thought_chunk` | `turn:delta`（`reasoning_content`） |
   | `tool_call` / `tool_call_update` | `turn:tool`（`TurnToolActivity`，`parent_activity_id` 指向 acp_delegate 调用） |
   | `plan` | `turn:tool` 子活动 或 `TurnTraceStep` |
   | `current_mode_update` / `available_commands_update` | 结构化日志（不进 UI） |
8. **stopReason → Pony 语义**：
   | ACP stopReason | Pony |
   |---|---|
   | `end_turn` | tool result `status: ok` |
   | `max_tokens` / `max_turn_requests` | `status: ok` + `truncated: true` |
   | `refusal` | `status: error`，错误码 `acp_agent_refused` |
   | `cancelled` | `status: aborted`，错误码 `tool_cancelled`（沿用既有） |
9. **错误码分源**（与 `tool_surface_singularity` 的"错误码保留来源"一致）：
   `acp_spawn_failed` / `acp_initialize_failed` / `acp_protocol_error` / `acp_agent_error` / `acp_agent_refused` / `acp_auth_required` / `acp_timeout` / `acp_unavailable`

---

## 4. 文件级改动清单

### P0 —— 协议层重建（不动任何现有行为）

| 文件 | 动作 | 要点 |
|---|---|---|
| `crates/pony-agent-core/src/lib.rs` | 改 | 加 `pub mod acp;`（当前缺，模块是死代码） |
| `crates/pony-agent-core/src/acp/mod.rs` | 重写 | 改为 `pub mod protocol; pub mod transport; pub mod client; pub mod session; pub mod config;`；把现有 `server` 降级为 `pub(crate) mod legacy_server;`（或不挂载，仅保留历史） |
| `crates/pony-agent-core/src/acp/protocol.rs` | 重写 | ACP v1 类型：`InitializeRequest/Response`、`ClientCapabilities`、`AgentCapabilities`、`SessionNewRequest/Response`、`PromptRequest/Response`、`StopReason`、`SessionUpdate`（8 个变体）、`ContentBlock`（text/image/audio/resource/resource_link）、`RequestPermissionRequest/Response`、`RequestPermissionOutcome`、`PermissionOption{optionId,name,kind}`、`ToolCallUpdate`、`ReadTextFileRequest`、`WriteTextFileRequest`、`Terminal*`；**`protocolVersion` 用 `u32`（=1）** |
| `crates/pony-agent-core/src/acp/transport.rs` | 新增 | `AcpTransport`：`spawn(command,args,cwd,env)`（env 白名单 + Unix 进程组，模式抄 `plugin_host/transport.rs:170-260`）、`request(method,params,deadline)`、`notify(method,params)`、入站请求分发回调、stderr 环形缓冲。**双通道**：一个 reader 线程区分 `id+method`（入站请求）/`method` 无 id（通知）/`id` 无 method（响应） |
| `crates/pony-agent-core/src/acp/config.rs` | 新增 | `AcpAgentSpec { id, label, command, args, env, default_cwd?, capabilities_hint }`；存储 `~/.config/pony-agent/acp-agents.json`（与 `providers.json` 同级，**独立文件**，不动 provider schema）；内置两条默认：`opencode → (opencode, ["acp"])`、`antigravity → (agy, ["acp"])` |

### P1 —— 会话驱动 + 委派工具

| 文件 | 动作 | 要点 |
|---|---|---|
| `crates/pony-agent-core/src/acp/client.rs` | 新增 | `AcpAgentClient`：`connect(spec, cwd)` → `initialize`（声明 `fs`/`terminal` 按宿主实际能力）→ 校验 `protocolVersion == 1`（不等则 fail-closed 报 `acp_protocol_error`）；`new_session(cwd, mcp_servers)`；`prompt(session_id, blocks, on_update)`；`cancel(session_id)`；`close()`。反向请求 handler 注入点：`PermissionHandler` / `FsHandler` / `TerminalHandler` |
| `crates/pony-agent-core/src/acp/session.rs` | 新增 | `AcpAgentSession`：会话串行闸（`Mutex`）、update → `TurnEventSink` 映射（见 3.2.7）、stopReason → `ToolResult` 归约（见 3.2.8）、`request_permission` → `ask_control::answer_ask` 往返、取消令牌联动（`ToolExecutionContext.cancellation`） |
| `crates/pony-agent-core/src/agent/tools/mod.rs` | 改 | 新增 `pub(crate) const TOOL_ACP_DELEGATE: &str = "acp_delegate";` + `PRIMITIVE_DISPATCH_TABLE` 加一条臂（`|router, call, context| router.execute_acp_delegate(call, context)`） |
| `crates/pony-agent-core/src/agent/tools/names.rs` | 改 | `canonical_tool_name` 加 `TOOL_ACP_DELEGATE` 映射（含 `"AcpDelegate"` 产品名与 `"acp.delegate"` 别名） |
| `crates/pony-agent-core/src/agent/tools/schema.rs` | 改 | `builtin_tools()` 新增 `ToolDefinition`：参数 `{ agent: string(必填), prompt: string(必填), cwd?: string, timeoutMs?: integer }`，`additionalProperties: false` |
| `crates/pony-agent-core/src/agent/tools/orchestration.rs` | 改 | `execute_acp_delegate`：解析 `AcpAgentSpec` → 解析 `workspace_root`（与 `subagent` 同源，`resolved_workspace_root`）→ 建 `tool.call` span → 建会话 → 跑 prompt → 归约结果。**结构照抄 `execute_subagent`（orchestration.rs:19-72）** |
| `crates/pony-agent-core/src/agent/tools/contract_view_tests.rs` | 改 | 工具面清单加 `acp_delegate`（该文件硬编码了工具名数组，漏改会红） |

### P2 —— 宿主形态（CLI / TUI）

| 文件 | 动作 | 要点 |
|---|---|---|
| `crates/pony-agent-core/src/bin/cli.rs` | 改 | `SubCommand` 加 `Acp`；`parse_cli_args` 识别 `acp`（在 `run_maintenance_subcommand_if_requested` 之后、常规解析之前拦截，照 `sessions`/`config` 的既有做法）；新增 `run_acp_client_mode()`：连外部 agent，REPL 交互，事件走 `JsonlSink`/`SseSink` |
| `src-tauri/src/lib.rs` | 改（可选 P2） | 新增 `acp_list_agents` / `acp_delegate_start` / `acp_delegate_cancel` 命令 + `invoke_handler` 注册 |
| `src/types/provider.ts` 或新建 `src/types/acp.ts` | 新增（可选 P2） | 前端 `AcpAgentSpec` / `AcpSessionView` 类型 |

### 清理（与 P0 同批，避免死代码继续腐化）

| 文件 | 动作 | 理由 |
|---|---|---|
| `crates/pony-agent-core/src/acp/server.rs` | 删除或移入 `acp/legacy_server.rs` 并标 `#[allow(dead_code)]` | 协议方向错误（把 agent→client 的 `session/request_permission` 当 client→agent 应答），且 `protocolVersion: "0.1"` 与 v1 不符；**不能作为起点复用** |
| `crates/pony-agent-core/tests/wave4_4_acceptance.rs` | 改 | `test_b084_acp_handshake_contract` 只断言字符串含 `"jsonrpc"`，是空壳；替换为真实契约测试 |
| `crates/pony-agent-sdk/`（整目录） | 补 `Cargo.toml` 或删除 | 当前无 `Cargo.toml`、不在 workspace members、`PonyClient::spawn` 依赖不存在的 `acp` 子命令 ⇒ 完全未构建的死代码。恢复条件：P2 的 ACP Host 模式落地后，作为「Pony 被别的 ACP client 驱动」的 SDK 重建 |

### 文档与治理（同批提交）

| 文件 | 动作 |
|---|---|
| `.agents/notes/proposed/architecture/2026-11-13-acp-client-external-agent-integration.md` | 新增 ADR（骨架：`## Problem` → `## Proposal` → `## Alternatives considered` → `## Acceptance criteria` → `## Risks`） |
| `docs/observability.yaml` | 新增归属说明段：新增**网络 IO 类**出口（spawn 外部进程 + stdio 协议）；四维归属：tracing 落点仍 `agent/otel/mod.rs`（新增 `acp.session` / `acp.request` span 走同一构造点）、issues 落点仍 `agent/ponysentry/client.rs`、metrics/logs 维持结构性 N-A（无常驻监听地址）。提交信息带 `[decl-change-approved]` |
| `docs/architecture/extension-points.md` | 新增「外部 Agent 接入（ACP）」一节，指向 `acp/` 模块 |
| `backlog/tasks.jsonl` | record + start（`backlog-gate` 要求非平凡交付留痕） |

### 测试

| 文件 | 内容 |
|---|---|
| `crates/pony-agent-core/tests/acp_client_contract.rs` | ① 用一个 **fake ACP agent**（Rust 内置子进程，或 shell 脚本）验证：`initialize` 的 `protocolVersion` 是整数 1 且被校验；② `session/new` 传绝对 `cwd`，相对路径被拒；③ `session/update` 三变体正确映射为 `turn:delta` / `turn:tool`；④ `request_permission` 触发 `PendingControlRequest` 且默认不自动 allow；⑤ `session/cancel` 后所有挂起 permission 收到 `cancelled`；⑥ 五个 stopReason 各自归约正确；⑦ 子进程崩溃 → `acp_protocol_error`（非 panic）；⑧ `protocolVersion` 不匹配 → fail-closed |
| `crates/pony-agent-core/src/acp/protocol.rs`（`#[cfg(test)]`） | 官方 schema 样例 JSON 的 round-trip（防止字段名 camelCase 写错） |

---

## 5. 落地顺序与验收

**阶段 P0（协议层，1 个 PR）** — `backlog:B149`
- 验收：`cargo check` 通过；`acp::protocol` 对官方 schema 样例 round-trip 全绿；`acp::transport` 对 fake agent 完成 `initialize` 往返。
- 门禁：`npm run cargo:check:shared`、`npm run adr:check`。

**阶段 P1（委派工具，1 个 PR）** — `backlog:B150`
- 验收：`acp_delegate` 能真实驱动 `opencode acp` 与 `agy acp` 各跑通一轮（人工验收）；`request_permission` 走 Ask 面板；取消链路 `stop_turn → session/cancel → cancelled`。
- 门禁：`npm test`、`npm run verify`、`acp_client_contract.rs` 全绿。

**阶段 P2（宿主形态，可选）** — `backlog:B151`
- 验收：`ponycli acp --agent opencode` 交互可用；`session/load` 恢复。

**明确不做（本方案边界）**
- 不改 `ProviderProtocol` / `ProviderClient`（外部 agent 不是 provider）。
- 不把 ACP agent 的工具输出回灌进 Pony 的 `ToolRouter`（两套工具体系保持隔离）。
- 不实现 ACP v2（`unstable_protocol_v2`）。

---

## 6. 风险

| 风险 | 缓解 |
|---|---|
| `request_permission` 默认放行会把治理链架空 | 硬约束：无 handler ⇒ 一律 `cancelled`（fail-closed），绝不 `allow` |
| opencode 单 session 单 prompt 限制被并发打穿 | `AcpAgentSession` 内部串行闸；并发委派走多进程 |
| 外部 agent 自带工具，Pony 看不到中间文件变更 | P2 可选：`fs/write_text_file` 声明 + 写操作落 `turn:tool` 记录，让变更可见 |
| `agy` 的 capabilities 未文档化 | 只声明 Pony 真正支持的能力；`initialize` 响应里的 `agentCapabilities` 缺字段按 `false` 处理 |
| 子进程 stdout 混入非协议日志会污染 JSON-RPC | opencode 用 `--print-logs` 分流；Pony 侧对非法 JSON 行记 stderr 诊断并跳过（不 panic），连续 N 行非法则 fail-closed |
