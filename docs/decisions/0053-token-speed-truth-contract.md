# 0053 token / 速度口径真值契约（wave-54）

Status: implemented

## 背景

桌面端右侧栏「状态」面板的 token 与速度长期被报「不准」。三路审计（Tempo 实测 / 前端口径链 /
Rust 物理口径）确认根因不是算错，而是三件事同时缺失：**口径未声明**（同一格在不同分支下是三套
语义）、**真值未落 span**（根 span 无速度/上下文属性，per-call 无 duration/ttft）、**无对账门禁**
（改完无法证明变准）。真实 trace `eed9ba47…`（20 次 `llm.chat`，98.47s）上：同一轮逐次速度均值
115.81 t/s、加权生成速度 95.46 t/s、端到端吞吐 46.13 t/s，跨度 2.5×。

## 候选方案

- **A. 只修 UI 公式**：把 `token-metrics.ts` 的三分叉收敛为一条。改动最小，但 span 侧仍无真值，
  验收只能靠人眼看数字——与"精准矫正"目标冲突（无法证伪，也无法回归）。
- **B. 只补 span 属性**：让 tracing 面拥有 duration/ttft/speed/context 真值。使对账成为可能，
  但 UI 仍显示三套口径，用户依旧看到"不准"，且重载后被折叠覆盖的 per-call 时长仍在。
- **C. 先补尺子再改数（采纳）**：C1 per-call 真值键 + C2 单一速度口径（Σout/Σgen 加权）+
  C3 usage 来源标记（provider | estimated_chars）+ C4 上下文口径协议归一 + C5 折叠不覆盖 per-call
  + C6 显示纪律；并以 golden replay 与不变量把三者锁死。
  选它的理由：只有在 span 上先有与被显示值同源的属性，才能把"准"写成机器可判定的断言；
  逐条改动以新增属性键与可选字段为主；但速度口径的切换会取代 wave-15 的「末次 per-call / 整轮聚合」
  断言（9 例，由独立 Test Agent 迁移，非实现者改测），属契约演进而非放宽。

## 决策

速度只保留一个口径：`Σoutput / Σgen`（`gen = duration − ttft`，仅 stream 且有 ttft 时抵扣），
以 `agent.turn.speed.tokens_per_second` + `basis=turn_weighted` 落到根 span；token 侧把
「上下文占用」与「缓存」按协议归一后各自同源显示，估算值以 `agent.usage.origin=estimated_chars`
显式标记、UI 前缀 `≈`；`projection.rs` 不再用整轮时长覆盖 per-call 值。

## 影响

- 显示值会变：多数会话的速度数字会与旧版不同（旧版是末次调用或整轮聚合）；这是口径变更，不是回归。
- `gen_ai.speed.tokens_per_second`（per-call）语义不变，但必须与 `out/gen` 同式；对账以
  `trace_id`/`agent.turn.id` 为键做三层 diff。
- 旧会话（无 `contextInputTokens`/`usageOrigin`）回退到既有 S1 解析序，显示 `—` 而不是 0。
- 后续若改口径，必须同时改本契约、golden fixture 与 `.dev-team/contract-matrix-wave54.md`，否则门禁红。
