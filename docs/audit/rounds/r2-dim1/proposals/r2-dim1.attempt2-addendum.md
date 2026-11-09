---
status: approved
approved_by: user
approved_unix: 1794190500
approval_note: >-
  用户批准 F5（导出有界重试 + 失败可见性）、F7（批量 transport=buffered）与写域扩展
  src-tauri/src/lib.rs。前提：阈值不放宽（L1/L2/L3/L4 与 attempt1 完全一致，assertion_hashes
  保持冻结）；F5 有效性只由单测与日志面锁定，不得声称重放证明了修复。
dimension: 1
dimension_name: latency.structure（时延结构）
round: r2-dim1
attempt: 2
amends: proposals/r2-dim1.attempt2.md（方案 A，已实施并验证）
author_session: pa_01a1ba70-a314-7712-adf2-4bf675a96f99
created_unix: 1794187400
baseline_commit: 1bca945808fc189435460ed83d789f8789b078a4
candidate_commit: d9425a70
corpus_hash: 879597eb3594eb365bdbe033f7fbcd8353380e359170267f8d3e621255b8300a
---

# r2-dim1 attempt 2 补遗：F5 导出可靠性 / F7 批量 transport / 写域扩展

## 为什么需要补遗

方案 A 已按批准实施，但**单独实施方案 A 不足以让 L1/L2/L4 转绿**。这一点在实施前的单会话诊断中
已取得机器证据（回执 `011-diagnose-f5.pa01a1ba70.json`），不是事后改判：

- 丢失的 2 个 `llm.chat` span 在 turn 中段就已结束（该 trace 根 span start 14:33:14.905，
  缺失 span 对应第 1、2 次调用，早于退出 7–23 秒），**退出 flush 的等待窗口覆盖不到它们**；
- 根 span 聚合 `gen_ai.usage.output_tokens=662 = 186+203+104+169`，而
  `LlmSpanGuard::record_usage` 的累加被 `if self.span.is_some()` 门控
  （`otel/mod.rs:2660`）⇒ 4 个 guard 的 span 都真实生成过，丢失发生在**导出**环节；
- 该会话 stderr 的 warn 计数为 0 —— 导出失败完全静默。

因此本补遗包含三项：F5（导出可靠性，blocker）、F7（批量 transport 口径，major）、
写域扩展（`src-tauri/src/lib.rs`）。

## F5（blocker）：导出失败不重试且不可见

### 根因（两条独立事实，均已定位到源码行）

| 事实 | 证据 |
| :-- | :-- |
| SDK 批量导出**不重试**，失败即丢整批 | `opentelemetry_sdk-0.30.0/src/trace/span_processor.rs:470-510`：`export_batch_sync` 先 `batch.split_off(0)` 清空再 `export`，失败仅返回 `Err`；worker 循环（340-420 行）对 `get_spans_and_export` 返回值一律 `let _ =` |
| 失败**不可见**（零日志） | 本仓 `opentelemetry` 依赖 `default-features=false`（`crates/pony-agent-core/Cargo.toml:45`），未启用 `internal-logs`；`otel_error!` 宏的非该特性分支被编译为空语句（`opentelemetry-0.30.0/src/global/internal_logging.rs:185-195`） |

collector 侧已排除：`otelcol_exporter_send_failed_spans=0`、`otelcol_receiver_refused_spans=0`。

### 方案（已实施）

在**唯一导出出口** `RuntimeExporter::export`（`otel/mod.rs`）做有界重试 + 逐次告警：

- `EXPORT_MAX_ATTEMPTS = 2`（含首次）、`EXPORT_RETRY_DELAY_MS = 200`；
- 每次尝试重新 clone 批次（`SpanData: Clone`），失败不消耗数据；
- 第 1 次失败 → `degraded_warn` 记「第 N/M 次尝试失败，200ms 后重试」；
- 最终失败 → `degraded_warn` 记「M/M 次尝试后放弃，本批 K 个 span 永久丢失」并向上返回 `Err`；
- 退避走导出 runtime 的定时器（不 `std::thread::sleep`，避免挂住 reactor）。

**不变量单测** `export_retry_is_bounded_and_exit_blocking_stays_within_budget` 钉三条：
① `EXPORT_MAX_ATTEMPTS >= 2`（否则等于没加）；② `<= 3`（有界）；③ 退出阻塞由
`MAX_FLUSH_WAIT_MS <= EXIT_FLUSH_BUDGET_MS` 保证 —— 重试发生在批量线程，退出路径只等夹值。

> 记录一次自我纠正：初版断言写成「attempts × 导出超时 ≤ 退出预算」，实测直接失败
> （6200ms > 3000ms）。该断言是**错误建模**：重试在后台线程，不会让退出延迟随 attempts
> 线性增长。改为断言夹值上界后通过。这条纠正本身已写进单测注释。

## F7（major）：批量路径 transport 错标为 stream

### 根因

`provider_followup_stream_with_calls`（`turn_flow.rs:3303`）的 `N > 1` 分支经
`ProviderManager::continue_with_tool_results_stream` 内部的**同步** `continue_with_tool_results`
发请求（`provider/mod.rs:1278-1288`），结果一次性回放为增量，但 span 上硬编码
`LLM_REQUEST_TRANSPORT_STREAM`。

实测：`s3-t01` 第 3 次调用日志为 `followup:start mode=sync-batch … calls=2`，其到达 span 却写
`llm.request.transport=stream`。

### 后果

L3（首包缺失必须可归因）把批量调用归入「流式首包测量丢包」而非「缓冲本无首包」⇒
**L3 判绿部分依赖错标**。这与 F3（新增 transport 键）要解决的语义混淆是同一个问题的残余。

### 方案（已实施）

抽纯函数 `batch_followup_transport() -> &'static str` 返回 `LLM_REQUEST_TRANSPORT_BUFFERED`，
批量分支改为调用它；单测 `batch_followup_transport_is_buffered_not_stream` 钉住
「不得为 stream」+「四键同生同灭」。

## 写域扩展（1 项）

| 路径 | 为什么必须纳入 |
| :-- | :-- |
| `src-tauri/src/lib.rs` | 方案 A 要求桌面端与 CLI/TUI 同源：原文件自持 `EXIT_FLUSH_BUDGET_MS = 1000`，与 core 的 3000ms 不一致 —— 这正是 attempt 1「两处各写各的」缺陷来源。改为复用 `pony_agent_core::agent::otel::EXIT_FLUSH_BUDGET_MS` 并删除本地常量。**不纳入则桌面端与 CLI/TUI 再次不同构**。 |

其余改动均在既有写域内：`otel/mod.rs`、`turn_flow.rs`、`runtime/tests.rs`、`docs/observability.yaml`。

## 阈值

**不放宽**。L1/L2/L3/L4 阈值与 attempt 1/2 完全一致，`assertions/dim1.sh` 哈希已冻结
（`assertion_hashes` 14 项，冻结值 `879597eb…`）。本补遗只改实现与写域，不改判据。

## 候选臂实测（candidate-a3 = d9425a70，9 会话）

| 断言 | 阈值 | 结果 | 结论 |
| :-- | :-- | :-- | :-- |
| L1 根 span 缺失 | = 0 | **0/9** | PASS（attempt1 为 1/9） |
| L2 span 丢失 | = 0 | **0/23** | PASS（attempt1 为 3/30） |
| L3 不可归因 | = 0 | **0/23**（ttft_missing=3，全部落在 buffered 调用） | PASS |
| L4 缺口 >5s 会话数 | = 0 | **0/9**（最大 587ms，attempt1 为 1 个 22.9s） | PASS |

`dim1.sh` 退出码 **0**；`selftest.sh` pass=34 fail=0；`agent::otel` 45 passed。

诚实说明：本轮 9 会话 stderr 中**零条重试日志**，即本轮没有发生导出失败 —— F5 的价值在于
把「静默丢整批」变成「有界重试 + 失败可见」，其有效性由单测与日志面锁定，而非由本轮重放
「恰好没触发」证明。不能声称「重放证明 F5 修复了丢 span」。

## 回滚

- F5：删除 `RuntimeExporter::export` 的重试循环，恢复单次 `inner.export(...).await`；删两个常量与单测。
- F7：`batch_followup_transport()` 改回 `LLM_REQUEST_TRANSPORT_STREAM`。
- 写域：`src-tauri/src/lib.rs` 恢复本地常量（但会重新引入两处漂移）。

## 需你决策

批准本补遗（F5 + F7 + 写域扩展 `src-tauri/src/lib.rs`），使 attempt 2 的 verdict 可在
冻结集与写域检查下跑完。
