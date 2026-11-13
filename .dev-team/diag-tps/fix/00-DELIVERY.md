# 交付：t/s 口径三项修复（F1/F2/F3）

- 依据：`.dev-team/diag-tps/00-SYNTHESIS.md`（r2 诊断裁决）
- 团队：Lead + test-agent（L2-T）+ exec-metrics（F1）+ exec-core（F2/F3），全部只读/写域隔离
- 范围批准：用户选择「P0+P1 三件」（2026-11-12 19:34）
- 窗口：19:36–20:45 (+08:00)。**未做任何 git commit / stash / checkout**；他人 43 项未提交 WIP 原样保留。

---

## 1. 三项修复与契约

| # | 契约 | 实现落点 | 判据 |
|---|---|---|---|
| F1 口径可取证 | 轮终态同事务落 `turn/speed` 事件 + 同帧 `turn speed settled` 日志；键 `turnSpeedTokensPerSecond/turnSpeedBasis/turnCallsTotal/turnCallsEligible/turnBurstExcluded`(+`turnExcludedTotal`/`turnCallsTtftMissing`)；无值整键缺席 | `turn_event.rs:202`、`turn_flow.rs:1033/1281/2806`、`metrics_snapshot.rs:56-66/314-316`、`bin/tui.rs:4075/4189`、`HomeStatusPanel.vue:107/114/481`、`token-metrics.ts:699/722/845` | 唯一真值 `weighted_turn_speed`；样本数唯一出口 `turn_call_census` |
| F1 显示诚实化 | TUI `120 t/s decode n=1` / `≈120 t/s e2e n=3 剔突发N`；桌面 tooltip 显样本量与剔除数 + `data-speed-burst-excluded`；无值不占位 | 同上 | wave-61 §1 行内布局冻结未破（burst 走 tooltip） |
| F2 窗口纯净 | per-call `dur`/`ttft` 收敛到「最后一次成功 attempt」；`attempts`/失败耗时走独立结构化日志 `provider_call_retry_window`；速度公式零改动 | `retry.rs:1149-1395`（账本 + `begin_provider_attempt_window`/`take_pure_call_window`）、`provider/mod.rs:5703-5724`、4 处调用点 + `turn_sync.rs`/`turn_prep.rs` 残差各 ≤2 行 | gen 只覆盖一次成功尝试 |
| F3 节流离开读线程 | 6 处 `thread::sleep(16ms)` 全删 → `StreamDeltaCoalescer`（16ms 时间窗、只拼接不 sleep、收尾无条件 flush） | `stream_support.rs:881-960`、`turn_stream.rs` 3 处、`tool_recovery.rs` 2 处 | 读线程每 delta = 1 次拼接 + 1 次比较 |

**F2 取证结论（成立，file:line）**：原取样点 `turn_stream.rs:2174/2395/978`、`tool_recovery.rs:19/103` 均在重试循环（`provider/mod.rs:5682-5704`）之外。分寸如实记录：`gen=dur−ttft` 因两端同源**近似抵消**，真正被污染的是 `dur` 单独作分母的 `end_to_end`（物理探针 2600ms vs 纯净 500ms = 5.2×）。

## 2. 红相物理收据（本轮方法：缺陷变异注入 + 基线静态门）

因三个修复并发落地，事后无法回到「未修复基线」运行（另有他人 WIP 交叉），改用**变异测试**证明测试真能抓到缺陷（注入→红→复原→绿）：

| 变异 | 注入点 | 结果 |
|---|---|---|
| M1 | 读路径重新插入 `std::thread::sleep` | `f3_stream_throttle_isolation` **FAILED 1**（静态用例命中）→ 复原 Exit 0 |
| M2′ | 快照三个样本键改 `None` | `f1_speed_observability` **FAILED 2**（panicked @:298/:342）→ 复原 4 passed |
| M3 | `rebase_duration_ms` 退回外层时长 | `f2_retry_window_purity` **FAILED 1** → 复原 2 passed |
| 基线静态门 | 修复前 13 文件（含他人 WIP）恢复后 grep | **21 处命中**，Exit 1（行号与诊断逐处吻合：1078/1158/2283/2333 + tool_recovery 214/244） |

## 3. 绿相门禁（物理收据，2026-11-12 20:39–20:47）

```
cargo check -p pony-agent-core                                  EXIT 0
cargo test -p pony-agent-core --lib                             EXIT 0  1527 passed / 0 failed（45s）
cargo test --test ts_contract_snapshot                          EXIT 0  1 passed（快照已重生成 + 复跑）
cargo test --test tui_metrics_snapshot                          EXIT 0  133 passed
cargo test --test tui_metrics_dual_caliber                      EXIT 0  137 passed
cargo test --test f1_speed_observability                        EXIT 0  4 passed
cargo test --test f2_retry_window_purity                        EXIT 0  2 passed
cargo test --test f3_stream_throttle_isolation                  EXIT 0  2 passed / 1 ignored(ADVISORY)
cargo test --test pa172_provider_failover_contract              EXIT 0  11 passed
npx vue-tsc --noEmit                                            EXIT 0
npx vitest run src/lib/runtime/__tests__ + metrics-*            EXIT 0  22 passed
静态门（读路径零 sleep）                                          EXIT 0
```

**负载敏感说明**：同一 `--lib` 在高负载轮（415s）出现 3 条失败、空闲轮（45s）1527/0；test-agent 收据 §9.3 已逐条单跑全绿并列表（`run_turn_parallel_tools_*`、`run_command_returns_stdout_*` 等 4 条），与本轮修复无因果关系。

## 4. 全量 vitest 的 2 条红：非本单

- `tests/acceptance/wave47-ob2-enforcement.spec.ts T7`：`ponygo observe audit --level ob2` Exit 1，原因为「[D1] 声明在本次变更中被修改，需要显式批准」——工作区 `docs/observability.yaml` 被**他人**改动（+39 行，18:39 起即为 `M`），脏工作区结构上不可绿。
- `tests/acceptance/pds-streaming/v10-baseline-freeze.spec.ts V10-3`：内层再跑全量套件 ⇒ 由 T7 派生。

## 5. 未提交（有意为之）+ 定向提交清单

工作区同时存在他人 43 项 WIP（模块拆分 `turn_flow/`、`session/store/`、`usage.rs`、`openai_wire.rs`、`docs/observability.yaml` 等）。`git add .` 会把这些一并提交，故**不提交**，留待 WIP 落定后定向提交本单文件：

```
crates/pony-agent-core/src/agent/{turn_flow.rs,metrics_snapshot.rs,otel/mod.rs,projection.rs,turn_event.rs,retry.rs}
crates/pony-agent-core/src/agent/provider/{mod.rs,openai_sse.rs}
crates/pony-agent-core/src/agent/runtime/{turn_stream.rs,tool_recovery.rs,stream_support.rs,turn_sync.rs,turn_prep.rs}
crates/pony-agent-core/src/bin/tui.rs
crates/pony-agent-core/tests/{f1_speed_observability.rs,f2_retry_window_purity.rs,f3_stream_throttle_isolation.rs,ts_contract_snapshot.rs}
src/{lib/runtime/token-metrics.ts,lib/runtime/useTraceProjection.ts,types/runtime.ts,components/HomeStatusPanel.vue}
src/lib/runtime/__tests__/f1-token-metrics-sample.spec.ts
docs/contracts/ts-contract.snapshot.json
.dev-team/diag-tps/**
```

## 6. 已知限制与 backlog 建议

1. `llm.chat` span 的 per-call dur 仍含重试（attempt 账本是线程局部一次性台账，在 turn_flow 内取走会毁 F2）。正确修法：把已结算 `PureCallWindow` 传进 span，或 retry 提供 peek/offset。当前 OTel 导出 401 全丢，不属取证面。
2. 慢 sink 仍会串进读循环（emit 内联，契约 v2 已裁决接受；生产 sink 均非阻塞）。彻底解耦 = 发送线程/队列，登记 backlog。
3. 上游侧：`gemini-3.8-flash-high` 8/12 次 429（antigravity 账号池冷却）；dsh 侧测速前应记录 HTTP 码。
