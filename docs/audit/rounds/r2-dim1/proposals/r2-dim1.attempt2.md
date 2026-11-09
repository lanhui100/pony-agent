---
status: proposed
dimension: 1
dimension_name: latency.structure（时延结构）
round: r2-dim1
attempt: 2
supersedes: proposals/r2-dim1.md（attempt 1，status: approved，已实施 e51c076d）
author_session: pa_01a1ba70-a314-7712-adf2-4bf675a96f99
created_unix: 1794153000
baseline_commit: 1bca945808fc189435460ed83d789f8789b078a4
candidate_commit: e51c076d54f4dfb58dd2f837ecf4d94640a5f634
corpus_hash: 879597eb3594eb365bdbe033f7fbcd8353380e359170267f8d3e621255b8300a
evidence: 18 sessions（replay/base-a2 9 + replay/candidate-a2 9，3 任务 × 3 会话），receipts 008 / 009
---

# r2-dim1 attempt 2：F1 预算不足 + L1/L2 残留（阈值不放宽）

## attempt 1 结论（机器证据，不得改判）

`bash scripts/audit/verdict.sh --round r2-dim1 --dim 1 ...` → **exit=1（FAIL）**，回执 009。

| 断言 | 阈值（冻结） | 基线臂 | 候选臂 | 结论 |
| :-- | :-- | :-- | :-- | :-- |
| L1 根 span 完整性 | 缺失 = 0 | 4/9 缺失 | **1/9 缺失** | FAIL（改善但未达标） |
| L2 调用保真 | 丢失 = 0 | 8/15 丢 | **3/30 丢** | FAIL |
| L3 首包可归因 | 不可归因 = 0 | 不可测（无 metrics 覆盖） | **0/27** | **PASS（F3 生效）** |
| L4 长尾可归因 | 超阈 = 0 | 7/9 不可测 | **1 会话 22 867ms** | FAIL |
| 棘轮 dim6 | 无回归 | — | A1/A2/A3 全 PASS | PASS |

F3（transport 键）与 F4（文档）已达成目标；**F1 方案不充分**，L1/L2/L4 未达标。

## 根因（新增，均已定位到常量与日志行）

### R1（blocker）退出 flush 预算 < 单次导出超时 ⇒ flush 自身必然超时

| 常量 | 值 | 位置 |
| :-- | :-- | :-- |
| `EXIT_FLUSH_BUDGET_MS` | 1 000 | `otel/mod.rs:1038` |
| `EXPORT_TIMEOUT_MS` | 3 000 | `otel/mod.rs:3094` |
| `SHUTDOWN_TIMEOUT_MS` | 1 000 | `otel/mod.rs:3102` |

`flush_state` 用 `recv_timeout(wait_ms)` 等待导出线程；`wait_ms = min(EXIT_FLUSH_BUDGET_MS, MAX_FLUSH_WAIT_MS)` = 1000ms，
而单次 OTLP/HTTP 导出网络超时是 3000ms ⇒ **只要队列里真有 span 待导出且网络非瞬时，flush 必超时**。

实测（candidate-a2，共 2/9 会话命中，其中 1 个正是 L1 缺失会话）：

```
s1-t02-read-explain.stderr:1267  warn force_flush 在 1000ms 内未完成：timed out waiting on channel
s1-t02-read-explain.stderr:1268  warn 退出 flush（cli-drop）未在预算内完成：…
s1-t02-read-explain.stderr:1269  warn otel::shutdown：provider 关闭超时或失败 … Timeout(1s)
```

⇒ attempt 1 把 flush 接进退出路径是对的，但**预算选错了量级**：预算必须 ≥ 一次导出的最坏耗时，否则「加了 flush」与「没加」在网络慢时等价。

### R2（major）批量 followup 路径的 span 数 ≠ HTTP 请求数

candidate-a2 逐会话对拍（usage 行数 = HTTP 请求真值，来自 provider 日志）：

| 会话 | usage 行 | GENERATION span | 丢 |
| :-- | --: | --: | --: |
| s3-t01-list-summarize | 4 | **2** | **2** |
| s1-t02-read-explain | 2 | **1** | **1** |
| 其余 7 会话 | 24 | 24 | 0 |

`s3-t01` **无任何 flush/关闭告警**（stderr 全绿，仅 1 条 debug）⇒ 不是 R1 的同一缺陷，
而是 span 在导出前就没有生成。该会话 `followup-sync-batch` 请求 `calls=2`（1 个 HTTP 请求覆盖 2 个 tool call），
而 `provider_followup_stream_with_calls`（`turn_flow.rs:3303`）在 `N > 1` 时只开 **1 个** `llm.chat` span；
两条 stream 请求也未各自留下 span。⇒ **「一次调用一个 span」的不变量在批量/多调用路径上不成立**，
L2 的 provider 真值（usage 行）与 span 计数口径在此分叉。

### R3（minor）基线臂 3 会话 observations 不可用

base-a2 的 s2-t01 / s3-t01 / s3-t02 三条 trace 存在但 `/api/public/observations` 返回空
（`missing=true`）。这使基线臂 L3/L4 无法对照（`sessions=0`），只影响对照强度，不影响候选臂判定。

## 方案（三选一，需你定方向）

### A. 退出 flush 预算对齐导出超时（改常量，最小改动）

`EXIT_FLUSH_BUDGET_MS: 1000 → 3000`，同时把 `SHUTDOWN_TIMEOUT_MS` 降为 0（关闭不再等待，flush 已保证导出完成）。
- 退出总预算 3000ms = NFR `exporter_flush_timeout_ms` 上限，不突破契约。
- 代价：最坏退出耗时从 2000ms 升到 3000ms；`otel/mod.rs` 注释与 `docs/observability.yaml:966-971` 需同步改。
- 风险：NFR 若按「flush + shutdown 各 1000」双段预算写死，需改文档而非代码语义。

### B. 轮次结束即 flush（把导出从「退出时」提前到「turn 完成时」）

`turn_flow` 在 `turn` 结束处调用 `force_flush(EXIT_FLUSH_BUDGET_MS)`。
- 退出路径不再承担尾批；预算问题整体消失。
- 代价：每轮多一次同步等待（1000ms 上限），**直接抬高 L4 未归属时延**（本维度正是审这个）；且高频场景吞吐下降。
- 与 ADR 0101「先修测量再谈优化」冲突（用性能换可观测性）。

### C. 缩短批量调度周期（`BATCH_SCHEDULED_DELAY_MS: 5000 → 500`）

让尾批在关闭窗口（1000ms）内自然到期。
- 不改退出预算、不加同步等待，退出耗时不变。
- 代价：导出请求频率 ×10（Langfuse/Tempo 写入压力）；`docs/observability.yaml` 需登记该常量变更。
- 风险：只对「关闭前 500ms 内产生的 span」有效，仍有窗口。

**推荐 A**：唯一能保证「导出最坏耗时内一定完成」的方案，且不引入同步等待；
B 与本维度目标（降低未归属时延）直接冲突，C 只是把窗口从 1000ms 缩到 500ms 而非消除。

R2 不并入本轮（属「一次调用一个 span」不变量在批量路径的缺失，涉及 `turn_flow` span 模型改动，
建议单列维度 1 的 follow-up 提案或转维度 3，避免本轮闭包膨胀）。

## 阈值

**不放宽**。L1/L2/L4 阈值与 attempt 1 完全一致（`assertions/dim1.sh` 哈希不变）。
按「阈值冻结 + 事后放宽须新提案」纪律，本次只改实现，不改判据。

## 写域（若选 A）

- `crates/pony-agent-core/src/agent/otel/mod.rs`（两个常量 + 注释）
- `docs/observability.yaml`（966–971 行的预算说明）
- 断言脚本与 corpus **不动**（哈希保持冻结值）

## 回滚

常量改回 1000 / 1000，行为回到 attempt 1 状态（已知不达标，但可复现）。

## 本轮同时修复的 harness 缺陷（已完成，不属本提案范畴）

| 缺陷 | 修复 | 证据 |
| :-- | :-- | :-- |
| metrics 通道 schema 分裂（dim1 / dim6 各一套字段） | `collect_dim1.py` 输出 trace 级超集字段 | 单份 metrics 同时满足两维度 |
| 串臂（臂换名后沿用旧 metrics） | `lib.sh: metrics_arm_guard`，断言开头校验 session_id 覆盖 | 串臂 → exit 2 |
| 空断言冻结集静默放行 | `state.py check-freeze`：`assertion_hashes` 为空 → exit 2 | 自测红相 |
| 自测缺上述两条红相 | `selftest.sh` 补 2 条，31/31 通过 | `bash scripts/audit/selftest.sh` exit=0 |

## 需你决策

1. **选 A / B / C 哪个方向**（推荐 A）；
2. R2（批量路径 span 数 ≠ 调用数）是否并入本轮——建议**不并入**，单列后续。
