# wave-69 t-red 红相收据（Test Agent / L2-T·L2-AT）

> 契约真值源：`.dev-team/contract-matrix-pa172.md`（v2 定稿版，含两路计划审查 FAIL 后修订）
> + `.dev-team/contract-matrix-pa171.md` `### Amendment: wave-69-a1`。
> 全部命令在仓库根执行；构建槽位遵守矩阵 §0 F26：**一律带 `--target-dir target-test`**（禁裸 `cargo test`）。
> 取证时间：2026-11-09 13:19–13:27（Asia/Shanghai）。基线 `git rev-parse --short HEAD` = `2b11ceba`（工作区另有他波未提交改动）。

## 1. 交付物与规模

| # | 文件 | 行数 | `#[test]` 条数 | 性质 |
|---|---|---|---|---|
| 1 | `crates/pony-agent-core/tests/pa172_provider_failover_contract.rs` | 810 | 11 | 新增 |
| 2 | `crates/pony-agent-core/tests/pa172_upstream_policy_config_contract.rs` | 694 | 8 | 新增 |
| 3 | `crates/pony-agent-core/tests/pa171_lease_anchor_contract.rs` | 283 | 6 | 新增 |
| 4 | `crates/pony-agent-core/tests/pa172_legacy_closure_contract.rs` | 638 | 13 | 新增 |
| 5 | `crates/pony-agent-core/tests/ts_contract_snapshot.rs` | 局部改（`error_class: None → Some("rate_limited")`） | 1（既有） | 夹具改动 |
| 6 | `docs/contracts/ts-contract.snapshot.json` | +1 键（`errorClass`，38→39） | — | 重生成 |

合计**新增用例 38 条**，全部真实断言；无 `#[ignore]` / `todo!()` / `assert!(true)` / 仅运行不报错。

## 2. 逐 target 红相（命令原文 / 退出码 / 首个错误）

红相形态 = **编译红**（新 API 未实现），即冻结签名锚点，wave-68 先例 D68-5 允许。

| # | 命令 | 退出码 | 错误条数 | 首个错误摘要 |
|---|---|---|---|---|
| T1 | `cargo test -p pony-agent-core --test pa172_provider_failover_contract --target-dir target-test` | **101** | 13 | `error[E0432]: unresolved import pony_agent_core::agent::config::UpstreamPolicy` |
| T2 | `cargo test -p pony-agent-core --test pa172_upstream_policy_config_contract --target-dir target-test` | **101** | 4 | `error[E0432]: unresolved imports ... agent::config::{UpstreamPolicy, UPSTREAM_*（9 个常量）}` |
| T3 | `cargo test -p pony-agent-core --test pa171_lease_anchor_contract --target-dir target-test` | **101** | 2 | `error[E0432]: unresolved import pony_agent_core::agent::goal_lease`；`error[E0603]: function turn_stream_outcome_from_terminal is private`（`control_plane/mod.rs:698`） |
| T4 | `cargo test -p pony-agent-core --test pa172_legacy_closure_contract --target-dir target-test` | **101** | 11 | `error[E0432]: unresolved import pony_agent_core::agent::retry::rate_limit_recovery_hint_at` |

### T1 其余错误（全部为缺失 API）
`ProviderFailoverChain` / `FAILOVER_BUDGET_MS` / `MAX_FAILOVER_HOPS` / `failover_eligible` /
`parse_failover_exhausted_marker` / `FAILOVER_EXHAUSTED_MARKER` 未解析；
`ProviderManager::with_failover`（8 处）+ `with_policy`（1 处）为 `E0599`。

### T2 其余错误
`ProviderRegistryStore::resolve_failover_chain`（3 处，`E0599`）。
（初版曾出现 `E0277 [&str] unsized`，为 `UpstreamPolicy` 未解析引发的级联；重构夹具后已消失，
现仅剩 4 条「纯缺失 API」错误。）

### T3 其余错误
`E0603`：`turn_stream_outcome_from_terminal` 现为 crate-private（`control_plane/mod.rs:698`），
按矩阵 §7.2 应提为 `pub`。

### T4 其余错误（全部为缺失 API）
`poison_registry_for_test`（`E0425`）、`reset_global_cooldowns`（`E0425`）、
`GoalRoundStopReason::{BudgetPersistFailed, NoActiveGoal}`（`E0599`，5 处）、
`UiEvent::GoalRoundsStopped` 无 `session_id`（`E0559`/`E0026`）。
（`tui` bin-as-module、`tui::user_facing_error_summary_at`、`tui::TuiState::default()` /
`reset_session_scope` 均**编译通过** ⇒ #6/#10 的触达路径成立。）

## 3. TS 契约（§6 T1/T3）

| 命令 | 退出码 | 结论 |
|---|---|---|
| `UPDATE_CONTRACT_SNAPSHOT=1 cargo test -p pony-agent-core --test ts_contract_snapshot --target-dir target-test` | **0** | 快照重生成（`1 passed`） |
| `grep -n errorClass docs/contracts/ts-contract.snapshot.json` | **0** | 命中 `107:  "errorClass"` |
| `cargo test -p pony-agent-core --test ts_contract_snapshot --target-dir target-test`（不带环境变量） | **0** | `1 passed`（13:20 与 13:25 各一次，均 Exit 0） |
| `node scripts/check-ts-contract.mjs` | **1**（必须红） | `[contract:check] TurnStreamEvent: Rust 39 键 / TS 声明 38 键 缺失 1` → `缺少 Rust 已下发的字段：errorClass` |

T3 红绿方向成立：Rust 侧已上线 `errorClass`，前端 `src/types/runtime.ts` 尚未加字段 ⇒ 门禁非零退出。

## 4. spec 冻结了哪些签名（本收据锚定的公共面）

| 契约 | 冻结签名 | 锚定用例 |
|---|---|---|
| §2.1 | `UPSTREAM_{REQUEST_TIMEOUT_MS,DECISION_MAX_ATTEMPTS,RETRY_BUDGET_MS}_{DEFAULT,MIN,MAX}`（`RETRY_BUDGET_MS_MIN=1000`） | `upstream_policy_constants_are_frozen` |
| §2.1 | `UpstreamPolicy { request_timeout_ms, decision_max_attempts, retry_budget_ms }` + `Default` / `clamped()->(Self, Vec<&'static str>)` / `is_default()` | `upstream_policy_defaults_match_frozen_baseline`、`upstream_policy_clamp_truth_table_row_by_row` |
| §2.1 | `ProviderSelectionResolver::{upstream_policy(), resolve_failover_chain(&primary)}`（默认实现） | `..._survive_registry_round_trip`、`invalid_json_like_values_are_clamped_not_fatal`、`failover_provider_ids_tolerates_non_string_items` |
| §2.2/C2-d | `ProviderRegistryView` 携带 `upstreamPolicy` + `failoverProviderIds`（保存→读回不丢） | `upstream_policy_and_failover_ids_survive_registry_round_trip` |
| §3.1 | `retry::{failover_eligible, FAILOVER_EXHAUSTED_MARKER, parse_failover_exhausted_marker, rate_limit_recovery_hint_at}` | `failover_trigger_class_truth_table_is_closed`、`failover_exhausted_returns_last_error_with_marker`、`error_summary_at_varies_with_injected_now` |
| §3.1 | `provider::{MAX_FAILOVER_HOPS=2, FAILOVER_BUDGET_MS=120_000, ProviderFailoverChain::{new,is_empty}}`、`ProviderManager::{with_policy, with_failover, upstream_policy, failover_chain}` | `failover_respects_max_hops`、`failover_budget_stops_handoffs` |
| §5 R1 | `provider::poison_registry_for_test()`（`#[cfg(debug_assertions)] #[doc(hidden)]`） | `poisoned_cooldown_registry_stays_usable` |
| §5 R2 | `provider::reset_global_cooldowns() -> usize` | `reset_global_cooldowns_reports_cleared_entries` |
| §5 R3 | `cooldown_key_for` / 段单射转义 / 去尾斜杠 / scheme·host 小写（`pub(crate)` ⇒ 以行为断言） | `cooldown_key_escapes_pipe_*`、`cooldown_key_normalizes_trailing_slash`、`cooldown_key_lowercases_scheme_and_host` |
| §7.1 | `agent::goal_lease::{GoalRoundLoopLeaseRegistry, new_goal_round_loop_lease_registry, GoalRoundLoopLease::{try_acquire, session_id}}` + `Drop` | 租约 5 例（同会话互斥 / 跨会话 / Drop / panic 展开 / 4 线程 × 50 轮 Barrier 竞态） |
| §7.2 | `pub control_plane::turn_stream_outcome_from_terminal(phase, error, decision) -> TurnStreamOutcome` | `turn_stream_outcome_truth_table_is_frozen`（12 行，含 `Pause⇒Cancelled` v3 更正） |
| §8 B4/B5 | `GoalRoundStopReason::{BudgetPersistFailed, NoActiveGoal}::visible_line()` 逐字 | `*_visible_line_is_frozen_and_distinct` |
| §8 B1/B2 | `UiEvent::GoalRoundsStopped { reason, status_line, session_id: Option<String> }`；`TuiState.goal_rounds_stopped` 复位 | `goal_rounds_stopped_event_carries_session_id`、`reset_session_scope_clears_goal_rounds_stopped` |
| §3.2 M1 / §6 M3 | `classify_error_class` 7 值封闭集；`TurnTraceRecord` wire 键 `errorClass` | `error_class_wordlist_is_locked`、`turn_trace_record_serializes_error_class` |

## 5. 偏差与如实登记

1. **存储层与视图层命名不一致（需 Lead 裁决，已用「探针」规避）**：实测
   `~/.config/pony-agent/providers.json` 的**存储层是 snake_case**（`base_url` / `api_key_env_var` /
   `secret_ref` / `selected_provider_id`），而 `ProviderRegistryView` 是 camelCase。
   矩阵 §2.2 写「`providers.json` 顶层 `upstreamPolicy` / `failoverProviderIds`（camelCase）」——
   与存储层既有惯例冲突。测试的做法：**视图层锁 camelCase**（C2-d 冻结），**存储层两种命名都接受**
   （`upstreamPolicy`/`upstream_policy`、`failoverProviderIds`/`failover_provider_ids` 探针命中其一即可），
   避免把实现细节当契约。若 Lead 要求存储层强制 camelCase，需新增 `#[serde(rename)]` 并同步旧文件兼容策略。
2. **非本任务的库编译中断（外部写入干扰）**：13:22–13:24 期间
   `src/agent/session/backend.rs`、`log_command.rs` 被**其它 agent** 修改（新增
   `PersistCommand::{AppendProviderTranscript, UpdateSessionMemory}`），`sqlite_session.rs:1147/1168`
   的 `match` 一度未补齐 ⇒ `--lib` 报 `E0004`。**本 Test Agent 未改动任何 `src/**` 文件**；
   13:24:41 该库已由对方修好，随后 `ts_contract_snapshot` 复跑 Exit 0（§3）。
3. **`Pause` 归约（v3 更正）**：矩阵 §7.2 早期写 `Pause ⇒ Suspended`，Lead 更正为 **`Pause ⇒ Cancelled`**
   （依据 HEAD `control_plane/mod.rs:705-710`）。用例已按 `Cancelled` 冻结，并覆盖 `phase=completed`
   输入以证明「决策优先于 phase」。
4. **原 `failover_not_triggered_after_stream_deltas` 已改名/改义**：按 v2 §3.3 F-g「流式入口**整类**
   不参与换手」⇒ `failover_stream_entry_does_not_handoff`（断言：流式中途失败 ⇒ 备 **0 次** HTTP）。
5. **`failover_budget_stops_handoffs` 的注入面**：矩阵未冻结「预算可注入」接缝
   （`FAILOVER_BUDGET_MS` 为 `const`）⇒ 用 `with_policy(request_timeout_ms=1000,
   decision_max_attempts=1)` + 慢 mock（应答延迟 2.5s）断言 per-attempt deadline 生效且总耗时有界
   （`< 6s`；基线 45s 内联超时下需 `>135s`），另加 `elapsed >= 1000ms` 反向钉子防「假超时」。
6. **日志面未机判**：6 个新结构化日志事件（`provider_failover_switched` /
   `provider_failover_skipped_cooldown` / `provider_failover_exhausted` / `provider_cooldown_reset` /
   `upstream_policy_clamped` / `failover_candidate_ignored`）与「记录被忽略字段 / 候选被忽略原因」
   的日志 sink 是**进程级单例**、不可按用例隔离 ⇒ 未写成断言，登记为 **non-machine（靠 review）**；
   换手行为本身以**物理 HTTP 计数**锚定（更硬）。
7. **#13 部分闭合**：`UiEvent::GoalRoundsStopped.session_id` 字段契约与跨会话构造已机判；
   「`session_id` ≠ 当前会话 ⇒ 只落日志、不改状态行、不进消息区」需驱动 TUI 事件循环 ⇒ 集成测试不可达，
   登记为**部分闭合**（靠 review）。
8. **#10 已机判**：`TuiState::default()` + `reset_session_scope()` 可达（bin-as-module 模式）⇒ 复位清空可断言。
9. **写域自检**：本任务只写 4 个新增 `tests/pa172_*` / `pa171_lease_anchor_contract.rs` 文件 +
   `tests/ts_contract_snapshot.rs`（夹具）+ `docs/contracts/ts-contract.snapshot.json`（重生成）+
   本文件；`crates/pony-agent-core/tests/pa171_*.rs` 五个冻结文件 `git diff` 为空（已核验）。
   未执行任何 git 写操作。
