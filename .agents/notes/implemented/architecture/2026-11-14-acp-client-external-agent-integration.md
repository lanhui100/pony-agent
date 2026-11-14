# Agent Note: 以 ACP Client 身份接入外部 Agent（opencode / antigravity）

Status: implemented
Date: 2026-11-14

## Problem

Pony 原先只有自有的 agent 循环与 `ProviderClient` 抽象，无法复用外部已成熟的 agent
（`opencode acp`、`agy acp`）。仓库里唯一相关的 `crates/pony-agent-core/src/acp/` 是
**未参与编译**的旧草稿：`lib.rs` 只有 `pub mod agent;`，`server.rs` 把
`session/request_permission` 当成 client→agent 的应答（方向相反），`protocolVersion`
写成字符串 `"0.1"`；`crates/pony-agent-sdk/` 无 `Cargo.toml`、不在 workspace members、
依赖的 `ponycli acp` 子命令不存在。也就是说：**既没有可用实现，死代码还在持续腐化**。

同时必须避免把外部 agent 当成 provider 接进模型选择链——那会把 Pony 的工具治理、
审批、预算与取消链路整体旁路。

## Decision

Pony 以 **ACP Client** 身份接入外部 agent，分三层落地（真源
`docs/architecture/acp-client-integration.md`）：

1. **协议层 `acp::protocol`**：ACP v1 类型全集，`protocolVersion` 为整数 `1`（`PROTOCOL_VERSION: u32`），
   serde camelCase；`session/cancel`、`session/update` 是 notification，
   `session/request_permission` 是 **agent→client 请求**。
2. **传输层 `acp::transport`**：自研同步 stdio transport（不引入 `agent-client-protocol` crate）：
   newline-delimited JSON-RPC 三路分流（`id+method`=入站请求 / `method` 无 `id`=通知 /
   `id` 无 `method`=响应），env 白名单 + Unix `process_group(0)`（与
   `agent/plugin_host/transport.rs` 同构），stderr 32 行环形缓冲，连续
   `MALFORMED_LINE_STREAK_LIMIT` 行非法 JSON 即时 fail-closed，`close()` 关 stdin →
   `SPAWN_REAP_GRACE` → 进程组 `SIGKILL` → `wait` 回收。
3. **会话层 `acp::client` + `acp::session`**：`connect`（`initialize` 校验 `protocolVersion==1`，
   不等即 `acp_protocol_error` fail-closed，不降级）→ `session/new`（cwd 必须绝对）→
   `session/prompt` 流式消费 → `session/cancel`；`AcpAgentSession` 带 `Mutex` 串行闸、
   `session/update` → `TurnEventSink` 映射、stopReason 5→4 归约、
   `request_permission` → `ask_control` 的 CAS 往返（`approve_control_request` /
   `cancel_control_request`），**无 handler / 超时 / 取消一律 `cancelled`**。
4. **宿主形态**：`acp_delegate` 委派工具（模型可见工具面 38→39、builtin 45→46）+
   `ponycli acp`（`--list` / 单轮 `-p` / `--resume` fail-closed）。
5. **同批清理**：删除方向错误的 `acp/server.rs`、删除零消费者 `crates/pony-agent-sdk/`；
   `tests/wave4_4_acceptance.rs` 的 B084/B085/B086 空壳断言（字符串恒真）替换为真契约。
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

## Consequences

- **落地收据**：`cargo test -p pony-agent-core --test acp_p0_acceptance`（19）、
  `--test acp_p0_adversarial`（7，L2-AT 畸形行/超时/并发）、
  `--test acp_client_contract`（11）、`--test acp_host_surface`（5）、
  `--test wave4_4_acceptance`（3）；`cargo test -p pony-agent-core --lib` 1550 passed / 0 failed
  （与改动前基线一致）；`npm run cargo:check:shared`、`npm test`、`npm run build` Exit 0。
- **能力边界**：外部 agent 是黑盒执行者，工具即契约边界；两套工具体系保持隔离
  （不把外部 agent 的工具输出回灌 `ToolRouter`）；未实现 ACP v2。
- **硬约束（安全）**：`request_permission` 无 handler 一律 `cancelled`，绝不默认 allow；
  `initialize` 版本不匹配 fail-closed；子进程 env 白名单（不继承宿主 provider key）。
- **遗留债（已在 backlog 登记）**：
  - `B161`：transport 在 prompt 期间内联派发的 `request_permission` 无 dispatcher 注入点
    ⇒ 只能 fail-closed；正向 Ask 往返仅经 `session.resolve_permission`，
    且其注入沿用 `hydrate_persisted_ask`（消费仍走真实 CAS）。P2 需一等公民 raise API +
    session 层 permission handler 注入。
  - `B162`：`session/update` 在 prompt 返回后按序 flush（`TurnEventSink` 非 `Send+Sync`），
    零丢帧但非实时，委派期间 UI 无实时 `turn:delta`。
  - `B163`：Tauri 三命令与前端 `src/types/acp.ts`（方案中标注为可选）未做。
- **Fix-round 决策（Gate-1/Gate-2 对抗审查后追加，2026-11-14）**：
  ① **单轮预算单一真源** = `acp/mod.rs:30 ACP_DEFAULT_TURN_BUDGET`（120s），CLI `--timeout-secs`、
  工具入口缺省、`ToolExecutionPolicy` 默认三处引用同一常量（此前三处各自 5s/120s，导致
  `timeoutMs`/`--timeout-secs` 声明可配而实际被 5000ms 硬顶）；`--timeout-secs 0` 显式 Exit 2。
  ② **宿主取消令牌只读**：`session.cancel()` 只翻本地 `local_cancel` 并回 `session/cancel`
  notification，**不**调用宿主 `CancellationToken::cancel()`——否则单次委派超时会取消整个 turn。
  ③ **取消归因优先于传输层错误**：取消已触发而 agent 不配合（不回 `cancelled`）时归约
  `aborted`/`tool_cancelled`，不退化成 `acp_timeout`。
  ④ **取消轮询**在 prompt 旁路线程进行，本线程 100ms 每 tick 重读令牌槽位（支持运行期热替换），
  `thread::scope` 必 join。
- **人工验收缺口**：`opencode acp` 本机不可用（仅有 `agy`），端到端真机联调仍需人工跑一轮。
