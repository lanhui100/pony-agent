# ADR 0084：TUI header 指标双口径——`↑in` 当前上下文 / `↓out` 与缓存命中率会话累计

Status: implemented

日期：2026-11-07
契约真值源：`crates/pony-agent-core/src/bin/tui.rs` 的 `metrics_segments` / `TuiState::session_usage` /
`TuiState::begin_turn_usage` / `TuiState::apply_live_metrics` / `hydrate_state_from_session`；
`crates/pony-agent-core/src/agent/session/usage.rs`（`SessionUsageTotals` / `SessionUsageLedger` /
`session_usage_totals` / `last_context_input_tokens`）；
`crates/pony-agent-core/src/agent/metrics_snapshot.rs`（`turn_context_input_tokens` /
`turn_cache_hit_input_tokens` / `turn_calls_with_cache` / `context_ratio_for_window`）

## 背景

ADR 0035 S1 冻结了「`↑in` = 最近一次调用的上下文输入（禁止跨轮累加）」，但同一条渲染路径把
`↓out`、缓存命中率也绑到了 per-call 快照上，于是 TUI header 的语义变成**全部按轮次/末次调用**：

- `↓out` 显示末次调用输出，而非会话累计 → 跨轮永远不增长，与桌面端 `HomeSidebar.vue`
  的「输出/缓存读会话累计」直接分叉；
- 缓存段用「末次调用命中 ÷ 末次调用上下文输入」，跨轮无意义；legacy 回退路径则在
  「会话累计命中 ÷ 当前上下文输入」上做 `clamp(0,100)` —— 不同量纲相除被 clamp 掩盖成 100%；
- `turn:started`（`begin_turn_usage`）清空 `live_metrics` → 轮内首个快照到达前**整行指标消失**；
- `/resume`/启动恢复（`hydrate_state_from_session`）只回放消息与工具，不回填任何 token/上下文，
  历史会话指标永远空白；后端已有的会话真值 `aggregate_session_metrics` 只被监控页使用。

## 候选方案与拒绝理由

- **A. 全部按轮次（维持现状）**：拒绝——`↓out` 与会话成本/产出直接相关，按轮次统计等于
  永远看不到「这个会话一共写了多少」；且与桌面端口径分叉，同一份数据两个答案。
- **B. 全部按会话累计（含 `↑in`）**：拒绝——`↑in` 是「当前上下文还占多少」，语义上就是最近一次
  调用的上下文输入（ADR 0035 S1）；改成 Σ 会把「上下文占用」变成「累计读取量」，用户会误判还有多少空间。
- **C. 让 TUI 直接读 `aggregate_session_metrics`（落库快照）**：拒绝——它是**事后**聚合：
  只在会话落库快照上可用，实时轮次中的数字会滞后一轮；且需要把 control plane 句柄塞进渲染路径。
- **D. 采用方案**：**双口径 + 单一台账**——`↑in` 保持当前上下文（ADR 0035 S1 不变），
  `↓out`/缓存改读**会话台账**（已结算轮次 Σ + 本轮 Σ），台账原料由 per-call 快照的**轮级 Σ 字段**供给；
  历史恢复用同一函数 `session_usage_totals` 从 `turn_trace_history` 的 per-call records 重算。
  这样实时与恢复、TUI 与桌面端共用同一口径，且不在渲染路径引入新的数据源依赖。

## 决策

1. **快照分两组字段，语义不得混用**：per-call 组（`context_input_tokens` / `output_tokens` /
   `cache_hit_ratio` …）只描述末次调用；新增的轮级 Σ 组（`turn_context_input_tokens` /
   `turn_cache_hit_input_tokens` / `turn_calls_with_cache`，与既有 `turn_output_tokens` /
   `turn_calls` 同级）由生产侧逐条 `provider_call_records` 相加得出 —— 会话累计**只**能从这组累加。
2. **会话台账 `SessionUsageLedger`**（core，`agent/session/usage.rs`）：`committed`（已结算轮次 Σ）
   + `current_turn`（本轮最新快照的 Σ）。`begin_turn` 结算本轮进 `committed` 且**不清零**；
   `set_turn` 幂等覆盖本轮（多跳不重复累加）；`restore` 用历史真值重置；`reset` 会话作用域复位。
3. **header 双口径**：`↑in` = 当前上下文（快照 `context_input_tokens` → legacy 当前输入）；
   `↓out` = 台账 Σ输出（无台账回退 legacy 会话累计 → 本轮 Σ）。
4. **缓存命中率 = Σ命中 / Σ上下文输入**（`SessionUsageTotals::cache_hit_ratio`），
   fail-closed：无命中上报 / 无输入 / 比值 > 1.0 ⇒ **无值**（不展示），**禁止** clamp 成 100%。
   缺 `cache_hit_input_tokens` 的调用计入 `calls` 但不计入 `calls_with_cache`——「未知」不得被当成「0 命中」。
5. **轮首保留末值 + 自描述**：`begin_turn_usage` 不再清空 `live_metrics`，只置 `live_metrics_stale`，
   header 追加「（上轮）」；新快照到达即复位。会话作用域复位（`/clear`、切会话）仍清空快照与台账。
6. **恢复回填**：`hydrate_state_from_session` 用 `session_usage_totals(snapshot)` 恢复会话累计、
   用 `last_context_input_tokens` 恢复当前上下文、用 `resolve_context_window_tokens` 恢复窗口；
   历史会话因此与实时会话同口径（不再指标空白）。
7. **窗口回退**：快照缺 `context_window_tokens`（provider 未声明能力）时，渲染期用调用方解析出的
   注册表真实窗口回退；比例仍由 core 的 `context_ratio_for_window`（复用 `unit_ratio`）算出，
   UI 不自写第三种公式、不猜常量窗口。
8. **取行口径不变**：指标簇仍锚定 header(row 0) 右槽（ADR 0078），底面最后一行只承载模型身份。

## 影响

- **代码面**：`agent/session/usage.rs`（新增，台账与会话累计单一真值）、`agent/metrics_snapshot.rs`
  （轮级 Σ 三字段 + `context_ratio_for_window`）、`bin/tui.rs`（`session_usage` /
  `live_metrics_stale` 状态、`metrics_segments` 双口径、轮首/恢复路径）、`src/types/runtime.ts`
  与 `docs/contracts/ts-contract.snapshot.json`（新增 3 个 wire 键）。
- **契约面**：`TurnMetricsSnapshot` 新增 3 个 additive 字段（`#[serde(default)]`），旧消费方读取不受影响；
  `metrics_snapshot_contract.rs` 的 wire 键表同步到 27 个键。
- **行为面**：跨轮 `↓out` 与缓存段不再归零/失真；轮首指标不再整行消失；`/resume` 后指标有值。
- **不改的部分**：`↑in` 的 ADR 0035 S1 语义、`t/s` 与 `首token` 的 per-call/轮级口径、
  legacy 事件路径（无 `metrics` 载荷）的单轮比语义（保留以兼容旧线缆）。
- **§3 观测性**：结构性 N-A（纯本地终端渲染 + 内存计量合成，无新增对外接口 / 网络 IO /
  后台任务 / LLM 调用），登记见 `docs/observability.yaml` 的 `wave-metrics-dual-caliber` 条目。

## 验收

| 契约 | 断言 | 用例 |
| :-- | :-- | :-- |
| C1 `↑in` 跟随当前上下文，`↓out` 跨轮累计 | `p0_header_output_is_session_cumulative_while_input_tracks_current_context` |
| C2 缓存段是会话累计命中率 | `p0_cache_segment_uses_session_cumulative_ratio` |
| C3 越界命中率 fail-closed（不 clamp） | `p0_session_cache_ratio_fails_closed_instead_of_clamping` |
| C4 快照生产侧给出轮级 Σ | `p0_snapshot_exposes_turn_level_sums_for_session_ledger` |
| C5 轮首保留末值并标 stale | `p1_turn_start_keeps_last_values_and_marks_them_stale` |
| C6 恢复回填累计与上下文 | `p1_restored_session_renders_cumulative_usage_and_context` |
| C7 快照缺窗口回退真实窗口 | `p2_snapshot_without_window_falls_back_to_resolved_window` |
| C8 台账 per-call Σ / 幂等覆盖 / 恢复 | `agent::session::usage::tests`（5 例） |
| C9 旧路径逐字不回归 | `tui_metrics_snapshot::c3_legacy_path_without_snapshot_is_unchanged` |
| C10 线缆键表与 TS 声明同步 | `metrics_snapshot_contract::wire_keys_are_camel_case_and_roundtrip`、`ts_contract_snapshot` |
