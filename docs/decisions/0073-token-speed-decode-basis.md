# 0073 token 速度口径 v2：解码窗口分母 + 物理下限 + basis 自描述

Status: implemented（2026-11-06）

## 背景

ADR 0053 把速度收敛成单一口径 `Σout / Σgen`（`gen = dur − ttft`，仅「stream 且有 ttft」时抵扣），
解决的是「同一格三套语义」。但这条口径在真实数据上产生了两个次生缺陷，用户直接可见：

1. **分母被非解码时长污染**。`gen` 在 `ttft` 缺失时退化为整段 `dur`，而 agent 轮次里绝大多数 hop
   是多工具批量的**非流式同步 followup**（`provider/mod.rs` 在多工具结果时必走 `provider_followup_sync`），
   这类调用没有 TTFT 可测。真实库（`~/.local/share/PonyAgent/sessions.db`，400 轮 / 1998 条 per-call）中
   无 ttft 记录 856 条，其中 537 条仍标着 `provider_stream`（首包未采到）。它们把排队、预填充与首字延迟
   全部算成「生成时间」，整轮速度被系统性压低。
2. **分母可以被压到计时抖动**。上游代理把 SSE 攒成一包下发时 `ttft ≈ dur`，`gen` 塌成 1–2ms，
   没有下限保护，单调用速度出现 51,000 甚至 1,789,000 t/s（库中 61 条 `gen ≤ 5ms`）。
3. **显示值不自描述**。TUI 与桌面端都渲染成同一个 `NNN t/s`，消费方无法区分「可比的解码速度」与
   「被 prefill 污染的端到端吞吐」，于是「最近这轮跟平时差好几倍」既无法解释也无法自检。

同库同轮集合实测：旧口径聚合 **50.0 t/s**，仅有效流式解码 **141.5 t/s**，端到端 **27.6 t/s**。

## 候选方案

- **A（采纳）分母只取有效解码窗口 + 物理下限**：分母只累计「有 TTFT 观测且 `gen` 不小于物理下限」的调用，
  分子扣掉首 token（`Σ(out−1)`）；无合格样本时回退端到端而不是伪造解码值。
- **B（落选）只做 A 不做下限**：把无 ttft 调用剔出分母后，突发包（`ttft≈dur`）全部变成分母≈0，
  同一轮会从 40.4 跳到 905.8 t/s —— 比现状更离谱。下限是 A 的必要条件，不是可选优化。
- **C（落选）只改展示（加 `≈` 前缀）**：不修分母，数字仍不可比；`≈` 会变成掩盖错误的装饰。
- **D（落选）改回端到端吞吐单一指标**：可定义性最好，但丢失「模型解码有多快」这一用户真正关心、
  且与厂商 spec 可比的信息。
- **E（落选）在 provider 层采流式 delta 时间戳做 TPOT**：物理上最准，但要改 provider 流式解析与
  时间戳采集，跨模块面远大于本次缺陷；作为独立议题登记，不在本轮。

## 决策

速度口径升级为 v2，唯一真值仍只在一处（Rust `turn_flow`），三端复用同一式子：

```text
valid(r)          := dur > 0 && out > 0
gen(r)            := (ttft 存在 && dur > ttft) ? dur − ttft : dur
floor(dur)        := max(200ms, 5% × dur)
decode_eligible(r):= valid(r) && ttft 存在 && dur > ttft && out >= 2 && gen >= floor(dur)

有 eligible ⇒ speed = Σ(out−1) / (Σgen/1000)      basis = stream_decode
无 eligible 但有 valid ⇒ speed = Σout / (Σdur/1000)  basis = end_to_end
都没有 ⇒ speed = None                              basis = None
```

设计取舍：

- **为什么要求 `ttft.is_some()` 而不用 `latency_kind == stream`**：`latency_kind` 是派生标签，
  库里有 537 条标着 stream 却没有首包时间的记录；物理上能算解码窗口的充要条件就是「观测到了首包时刻」。
- **为什么下限取 `max(200ms, 5%×dur)`**：200ms 是「能测出一个速率」的最短窗口；5% 相对下限挡的是
  长请求尾部一次性吐包。真实库上二者合计只排除 1 条原本可用的调用（825→824），代价可忽略。
- **为什么分子扣 1**：首 token 已经计入 TTFT，`(out−1)/(dur−ttft)` 才是标准 TPOT 的倒数
  （vLLM / MLPerf / GenAI-Perf 口径）。只扣 1 不扣 2，保证 `out=2` 的调用仍可参与。
- **为什么无合格样本时回退端到端、而不是显示 `—`**：批量同步 followup 为主的轮次（真实库里 28/111 轮）
  仍有「这轮总体多快」的信息价值，但**必须**声明量纲，不能冒充解码速度。
- **为什么 `basis` 复用 `turnSpeedBasis` 而不是新增第二套字段**：`basis` 原本与 `mode` 同值，
  是纯冗余；让它承载「这个数测的是什么」比再引入 `decodeBasis` 更不易混淆。

线缆与展示：

- `TurnMetricsSnapshot` 追加 `turnSpeedBasis` / `callSpeedBasis`（`stream_decode` | `end_to_end`，缺席 ⇔ 无值），
  旧字段名与旧消费者不变（additive + `skip_serializing_if`）。
- TUI：`stream_decode` 渲染 `156 t/s`，`end_to_end` 渲染 `≈156 t/s`，无值不渲染。
- 桌面端：`≈` 前缀条件 = 估算来源 **或** `basis == end_to_end`（两者同时成立只出现一次 `≈`）。
- 未来 HTTP/WS 消费面：`basis` 缺失 ⇒ 渲染 `—`；`end_to_end` ⇒ 必须带 `≈`/`e2e` 标记。
- 根 span `agent.turn.speed.basis` 取值同域，旧值 `turn_weighted` 作废；无 speed 时不写该键。
- `turnOutputTokens` / `turnGenerationMs` / `turnCalls` 语义收紧为「参与计算集合」，
  恒等式 `speed == (out − calls)/gen`（decode）或 `speed == out/gen`（e2e）可机械化断言。

## 影响

- **显示值会变，且是本轮要的结果**：真实库聚合 50.0 → 132.1 t/s（decode）、端到端回退 27.6 t/s。
  同一轮内跳变幅度也会显著收敛（旧口径单调用速度跨度 0.34 ~ 1.79e6 t/s）。
- 契约演进：`.dev-team/contract-matrix-wave54.md` §1 C2 与 `.dev-team/contract-matrix-metrics-snapshot.md`
  I5 的速度定义由 `.dev-team/contract-matrix-token-speed-v2.md` 取代；`tests/acceptance/wave54/*`、
  `tests/unit/token-speed-metrics.spec.ts`、`wave54_metric_truth.rs` T2/T7、`metrics_snapshot_*` 的 a2/a5
  与 I5 断言按新口径迁移（迁移而非删除，判别力保留）。
- 声明式后果：`turnSpeedBasis` / `callSpeedBasis` 是新增线缆键，`docs/contracts/ts-contract.snapshot.json`
  与 `src/types/runtime.ts` 必须同提交更新。
- 不改变 `first_token_latency_ms` 的取值语义（快照里仍是末次 hop 的 TTFT）。「首 token 展示应为
  本轮首包」是相邻但独立的缺陷，登记 backlog，不在本 ADR 处理。
