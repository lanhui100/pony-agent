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

- 反向契约：`acp_delegate_p0p2_contract.rs::c0_4c_cancelled_stop_reason_unchanged` 覆盖
  「agent 回 cancelled ⇒ status=aborted + tool_cancelled」。
- **正例契约（B169 补收口，2026-11-25）**：`acp_p1_blockers.rs::b169_cancel_race_keeps_ok_terminal_state`
  覆盖「取消已观察 + agent 回 Ok ⇒ 仍按 Ok 归约」。
- 生产代码零改动（本 ADR 是契约显性化，非行为变更）。

## cancel-race 正例怎么做到确定性（而不是靠 sleep 抢跑）

原方案被否的理由是「现有夹具产不出该竞态，只能靠两侧抢跑」。补收口时改掉了夹具的因果方向，
问题因此消失：

夹具新增 `cancel_race_ok <markerFile> <sessionId>` 模式——prompt 期间先发一条
`agent_message_chunk`（正文 `race-ok-body`），然后**阻塞读 stdin 等 `session/cancel`**，
收到之后才回 `{"stopReason":"end_turn"}`。

于是两条事件的先后由**因果**而非时钟决定：

- 宿主不发 `session/cancel` ⇒ agent 永不回包 ⇒ 用例等满预算后失败（不会静默通过）；
- 宿主发了 `session/cancel` ⇒ 终态 Ok **必然**在其后到达 ⇒ `cancel_observed == true`
  与 `Ok(response)` 同时成立，竞态窗口被稳定命中。

用例同时断言反相对照（agent 侧 `<marker>.cancel` 证据存在），因此「本用例确实覆盖了竞态」
本身也是可机械验收的——夹具若退化成不回 cancel，用例会以「反相对照失败」判红，而不是假绿。

**鉴别力已实测**：把 `Ok` 分支临时改成 `Ok(_) if cancel_observed => Cancelled` 后，
该用例立刻失败并报出 `B169 违约：cancel-race 下终态 Ok 必须按 Ok 归约`；恢复实现后复绿。

## Alternatives considered

- **在 `Ok` 分支加 `cancel_observed` 判定并改判 aborted**：会把已完成的真实输出丢掉，
  且与 `stop_reason` 的事实矛盾；同时让 dispatcher 的停止链与本层职责重叠（两处裁决同一件事）。
- **只写 ADR、代码不加注释**：ADR 是"事后的解释"，读者改代码时看不到；注释贴在分支上才拦得住
  下一个人"顺手加一行"。
- **加靠 sleep 抢跑的竞态测试**：本仓刚刚因为一条时序敏感的断言在 Windows CI 上连红四次
  （trace 假红，见 `bug-fix/2026-11-23-turn-trace-traceid-semantics.md`），不应再引入同类风险。
