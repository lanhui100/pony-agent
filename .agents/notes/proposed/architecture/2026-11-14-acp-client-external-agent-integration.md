# Agent Note: 以 ACP Client 身份接入外部 Agent（opencode / antigravity）

Status: proposed
Date: 2026-11-14

## Problem

Pony 目前只有自有的 agent 循环与 `ProviderClient` 抽象，无法复用外部已成熟的 agent
（`opencode acp`、`agy acp`）。仓库里唯一相关的 `crates/pony-agent-core/src/acp/` 是
**未参与编译**的旧草稿：`lib.rs` 只有 `pub mod agent;`，`server.rs` 把
`session/request_permission` 当成 client→agent 的应答（方向相反），`protocolVersion`
写成字符串 `"0.1"`；`crates/pony-agent-sdk/` 无 `Cargo.toml`、不在 workspace members、
依赖的 `ponycli acp` 子命令不存在。也就是说：**既没有可用实现，死代码还在持续腐化**。

同时必须避免把外部 agent 当成 provider 接进模型选择链——那会把 Pony 的工具治理、
审批、预算与取消链路整体旁路。

## Proposal

按 `docs/architecture/acp-client-integration.md` 分三层落地，方向固定为 **Pony 作为 ACP Client**：

1. **协议层（`acp::protocol`）**：ACP v1 类型全集，`protocolVersion` 为整数 `1`，serde camelCase；
   `session/cancel` 是 notification，`session/update` 是 notification，
   `session/request_permission` 是 **agent→client 请求**。
2. **传输层（`acp::transport`）**：自研同步 stdio transport，newline-delimited JSON-RPC 双向分发
   （`id+method`=入站请求 / `method` 无 `id`=通知 / `id` 无 `method`=响应），
   env 白名单 + Unix 独立进程组（模式与 `agent/plugin_host/transport.rs` 同构），
   stderr 有界环形缓冲，连续非法 JSON 行 fail-closed；不引入 `agent-client-protocol` crate。
3. **会话层（`acp::client` / `acp::session`）**：`initialize`（校验 `protocolVersion==1`，
   声明 fs/terminal 按宿主真实能力）→ `session/new` → `session/prompt` 流式消费 → `session/cancel`；
   反向请求经 `PermissionHandler`/`FsHandler`/`TerminalHandler` 注入；**无 handler ⇒ 一律 `cancelled`**。
4. **宿主形态**：P1 先做 `acp_delegate` 委派工具（与 `subagent`/`workflow` 同构，
   复用 `ask_control` 审批、`BudgetLedger`、`CancellationToken`、`ToolExecutionContext`）；
   P2 再做 `ponycli acp` 子命令宿主形态。协议层按宿主要求设计，不假设一次性任务。
5. **同批清理**：删除方向错误的 `acp/server.rs`；删除 `crates/pony-agent-sdk/`（零消费者死代码）。
   不改 `ProviderProtocol` / `ProviderClient` / provider 配置 schema。

## Alternatives considered

- **方案 A：把外部 agent 作为 `ProviderProtocol::AcpAgent` 接进 ProviderClient 家族** —
  否决：`ProviderClient` 是「messages + tools → ProviderDecision」的无状态单轮抽象，
  而 ACP 是长连接会话型协议（session 生命周期、双向请求、agent 自带工具）。
  引入后 `is_anthropic()` / `is_openai_family()` 等行为分叉位点要加第三分支，
  且 Pony 的 `ToolRouter`/治理链/审批会被整体旁路。
- **方案 B：引入官方 `agent-client-protocol` crate** — 否决（当前阶段）：其 client future 非 `Send`，
  需额外的 tokio `LocalSet` + `spawn_local` 运行时线程；与 Pony「turn 跑独立线程 + 跨线程 `Arc`
  控制面」的并发模型冲突，新依赖收益不抵成本。改为自研 transport，但 `protocol` 的类型形状
  对齐官方 schema，将来换 crate 只换 transport 层。
- **方案 C：沿用现有 `server.rs` 草稿作为起点** — 否决：协议方向相反（把 agent→client 的
  `request_permission` 当入站应答）、`protocolVersion` 与 v1 不符（`"0.1"` vs `1`），
  不可作为起点复用。
- **方案 D：先做 ACP Host 模式（Pony 作为 server 被别的 client 驱动）** — 否决（本阶段）：
  需求是「Pony 使用外部 agent」，方向与 `acp_delegate` 相反；Host 模式留待后续再议。

## Acceptance criteria

- **P0（`backlog:B149`）**：`npm run cargo:check:shared` Exit 0；`protocol` 对官方 v1 样例
  round-trip 全绿；`transport` 对 fake agent 完成 `initialize` 往返；连续非法 JSON 行 fail-closed。
- **P1（`backlog:B150`）**：`acp_delegate` 可真实驱动 `opencode acp` / `agy acp` 各一轮；
  `request_permission` 走 Ask 面板；`stop_turn → session/cancel → cancelled` 链路成立；
  `tests/acp_client_contract.rs` 八项契约全绿；`npm run verify` 通过。
- **P2（`backlog:B151`）**：`ponycli acp --agent <id>` 交互可用；`--list` 列出内置 spec；
  agent 不支持 `session/load` 时显式返回 `acp_unavailable`（不静默）。

## Risks

- **`request_permission` 默认放行会架空治理链** → 硬约束：无 handler 一律 `cancelled`，绝不默认 allow。
- **opencode 单 session 单 prompt 限制被并发打穿** → `AcpAgentSession` 内部 `Mutex` 串行闸；
  并发委派走多进程而非共用 session。
- **外部 agent 自带工具，Pony 看不到中间文件变更** → P2 可选声明 `fs/write_text_file`，
  写操作落 `turn:tool` 记录，使变更可见。
- **`agy` 的 capabilities 未文档化** → 只声明 Pony 真正支持的能力；`initialize` 响应缺字段按 `false` 处理。
- **子进程 stdout 混入非协议日志污染 JSON-RPC** → `opencode --print-logs` 分流；Pony 侧非法行记
  stderr 诊断并跳过（不 panic），连续 N 行非法则 fail-closed。
- **环境泄漏**：子进程继承宿主 `provider key` 会泄密 → env 白名单（复用 plugin_host 的
  `apply_child_env` 模式）。
