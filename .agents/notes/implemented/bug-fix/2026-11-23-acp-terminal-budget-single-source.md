# Agent Note: `wait_for_exit` 的等待预算改由 turn 预算单源驱动（B177 ⑧）

Status: implemented

## Problem

预算被两处各说各话：

- `AcpAgentSession::set_turn_budget(budget)`（`session.rs`）→ `client.set_prompt_budget`，只影响
  **prompt 的 deadline**；
- `ManagedTerminalHandler::wait_for_exit`（`handlers.rs`）里写死 `ACP_DEFAULT_TURN_BUDGET`（120s）。

于是用户把轮次预算调到 300s 后，`terminal/wait_for_exit` 仍会在 **120s** 报「超过 120s 仍未退出」——
长命令被硬顶截断；反过来把预算调小，wait 依然死等 120s。复审 r2 记作 ADVISORY ⑧。

## Decision

把「整轮预算」做成**单源**，沿注入链路透传给 terminal handler：

1. `TerminalHandler` trait 新增 **默认空实现** `fn set_turn_budget(&self, _budget: Duration) {}` ——
   对既有实现与测试替身零影响（additive）。
2. `ManagedTerminalHandler` 新增 `turn_budget: Mutex<Duration>`（缺省 `ACP_DEFAULT_TURN_BUDGET`），
   实现 `set_turn_budget` 写入槽位；`wait_for_exit` 从槽位读预算（`?` 与超时文案都用该值）。
   用 `Mutex` 而非 `&mut self`：handler 是 `Arc<dyn TerminalHandler>` 共享的，必须支持热替换。
3. `AcpAgentClient::set_terminal_turn_budget(budget)` 转发给已装配的 terminal handler（未装配则不做事）。
4. `AcpAgentSession::set_turn_budget` 同时调 `set_prompt_budget` 与 `set_terminal_turn_budget` ——
   两个 deadline 从此同源。

**缺省行为不变**：不调 `set_turn_budget` 时槽位是 120s，与旧硬编码完全一致。

## Verification

- `bash scripts/cargo-lane.sh check -p pony-agent-core --lib` → Finished（exit 0）
- `bash scripts/cargo-lane.sh test -p pony-agent-core --test acp_config_regressions`
  → **8 passed / 0 failed**，含 `h4_long_running_terminal_wait_respects_budget_and_stays_responsive`、
  `h1_output_after_wait_for_exit_keeps_stdout`、`h5_terminal_output_buffer_is_bounded`、
  `i1_handshake_budget_exceeds_legacy_five_seconds` —— 既有预算语义未被破坏。

## 未做（诚实标注）

没有新增"`set_turn_budget(50ms)` ⇒ wait 提前超时"的专属用例：构造 `CreateTerminalRequest` /
`WaitForTerminalExitRequest` 需要一整套 wire 结构与不退出命令，成本高于收益，而 h4 已经覆盖
「预算内响应」的既有语义。新链路的回归由 CI 的 `--lib` + 全部集成测试兜底。

## Alternatives considered

- **把 `wait_for_exit` 改成 per-request 参数**：wire 协议没有这个字段（ACP v1 固定的
  `terminal/wait_for_exit` params），加字段等于改协议。
- **handler 构造期一次性注入预算**：构造发生在 `session/new` 之前，而 `set_turn_budget` 是
  之后的热替换（`&self` 范式），构造期注入会在用户调整预算后失效——正是本条要修的错位。
- **让 terminal handler 直接读全局配置**：把会话级预算变成进程级全局，多个会话不同预算时互相污染。
