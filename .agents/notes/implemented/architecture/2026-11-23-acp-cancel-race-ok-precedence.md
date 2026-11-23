# Agent Note: ACP cancel-race 的归约优先级——Ok 优先，取消不覆盖已到达的终态

Status: implemented

## Problem

`AcpAgentSession::prompt` 在执行期间会按 `turn_cancel_poll_ms` 轮询取消令牌，并在观察到取消时
向外部 agent 发 `session/cancel`。问题出在**竞态的收口**：取消信号置位的同时，agent 可能已经把
`session/prompt` 的终态响应发回来了（`EndTurn` / `MaxTokens` / `Refusal`）。

现有代码在 `Ok(response)` 分支直接按 `response.stop_reason` 归约，**不检查** `cancel_observed`。
这在生产上被宿主 dispatcher 的停止链兜住，所以没人报过事故；但契约本身**没有登记**——
下次有人"顺手"把 `cancel_observed` 加进这个分支，就会把「已完成并有正文」的轮次改写成 aborted，
用户会看到「已取消」而正文被丢弃，`errors` 里还会多出不该有的 `tool_cancelled`。

B169 登记的就是这条隐性契约。

## Decision

1. **Ok 优先**：`Ok(response)` 分支只按 agent 的终态归约，取消信号不参与。
   理由：Ok 表示这一轮**事实已完成**，输出完整可信；改写归约等于用本地意图否定上游事实。
2. **取消的真正收口点不在这一层**：`session/cancel` 已经在轮询里发出；"用户想停"由宿主
   dispatcher 的停止链裁决（`stop_turn` / 停止短语）。本层只对「这一轮的事实」负责。
3. **反向场景不变**：取消已发但 agent 不回包（传输层 timeout/断链）仍归约
   `AcpTurnOutcome::Cancelled` + `tool_cancelled`，这条既有分支与本次登记互不覆盖。
4. 契约以**就地注释 + 本 ADR**双重登记（注释贴着分支，避免读者只看到 ADR 找不到代码）。

## Verification

- 既有 `acp_delegate_p0p2_contract.rs::c0_4c_cancelled_stop_reason_unchanged` 仍覆盖
  「agent 回 cancelled ⇒ status=aborted + tool_cancelled」这条反向契约。
- 本轮**未新增** cancel-race 正例测试，原因见下；本 ADR 属于"契约显性化"而非行为变更，
  生产代码零改动（仅注释）。

## 为什么本轮不加 cancel-race 的自动化测试

现有 ACP 夹具的 fake agent 只支持 `flow end_turn|max_tokens|refusal|cancelled`，**产不出**
「取消已置位 + agent 同时回 Ok」这个竞态。要测它必须给 fake agent 新增 `flow cancel_race_ok`
并在两侧严格同步时序（谁先谁后决定结果），这类"抢跑型"测试在 CI 上是新的 flaky 源。

按本仓既有判据（ADR 关于假红的口径）：**宁可把契约写清楚 + 留可查的注释，也不引入一条
靠时序运气过关的测试**。后续若要加，应做成「夹具显式回放两次事件顺序」的确定性用例，
而不是靠 sleep 抢跑。

## Alternatives considered

- **在 `Ok` 分支加 `cancel_observed` 判定并改判 aborted**：会把已完成的真实输出丢掉，
  且与 `stop_reason` 的事实矛盾；同时让 dispatcher 的停止链与本层职责重叠（两处裁决同一件事）。
- **只写 ADR、代码不加注释**：ADR 是"事后的解释"，读者改代码时看不到；注释贴在分支上才拦得住
  下一个人"顺手加一行"。
- **加靠 sleep 抢跑的竞态测试**：本仓刚刚因为一条时序敏感的断言在 Windows CI 上连红四次
  （trace 假红，见 `bug-fix/2026-11-23-turn-trace-traceid-semantics.md`），不应再引入同类风险。
