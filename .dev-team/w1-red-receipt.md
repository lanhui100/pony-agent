# W1 RED 收据（task-4）

- 角色：独立 Test Agent（只写测试，不改生产逻辑，不做 git 写）
- 工作区：`/home/dm/pony-agent`
- 变更范围：仅两个文件的**追加**测试（`git diff --stat` = +291 / -0，无删除、无生产逻辑改动）
  - `crates/pony-agent-core/src/agent/planner.rs`（+116，追加 `mod tests` 内 5 个测试）
  - `crates/pony-agent-core/src/agent/runtime/stream_support.rs`（+175，追加 `#[cfg(test)] mod w1_pending_control_match_tests`，6 个测试）
- 未执行 git commit。

## 1. 命令与退出码（物理凭证）

### 1.1 planner（契约 C1–C5）

```
$ cargo test -p pony-agent-core --lib agent::planner -- --nocapture
```

退出码：**101**（测试失败 = 预期 RED）

```
running 33 tests
...
thread 'agent::planner::tests::w1_looks_like_path_rejects_root_and_dot_segments' (1459775) panicked at crates/pony-agent-core/src/agent/planner.rs:1584:13:
契约 C5："/" 不得判定为路径
test agent::planner::tests::w1_looks_like_path_rejects_root_and_dot_segments ... FAILED

thread 'agent::planner::tests::w1_preflight_continue_message_must_not_be_hijacked_by_slash_in_history' (1459776) panicked at crates/pony-agent-core/src/agent/planner.rs:1501:9:
契约 C1：`继续` 必须返回 None（不得被历史里的 `/` 劫持成根路径收集），实际 tool_call = Some(("Read", "{\"description\":\"收集路径 `/` 的上下文。\",\"limit\":40,\"lineCount\":80,\"path\":\"/\"}"))
test agent::planner::tests::w1_preflight_continue_message_must_not_be_hijacked_by_slash_in_history ... FAILED

thread 'agent::planner::tests::w1_preflight_progress_check_must_not_be_hijacked_by_slash_in_history' (1459779) panicked at crates/pony-agent-core/src/agent/planner.rs:1522:9:
契约 C2：`检查tester和reviewer的进展` 必须返回 None，实际 tool_call = Some(("Read", "{\"description\":\"收集路径 `/` 的上下文。\",\"limit\":40,\"lineCount\":80,\"path\":\"/\"}"))
test agent::planner::tests::w1_preflight_progress_check_must_not_be_hijacked_by_slash_in_history ... FAILED

test agent::planner::tests::w1_preflight_keeps_explicit_path_gather_context ... ok
test agent::planner::tests::w1_preflight_keeps_referential_gather_context_for_history_path ... ok

failures:
    agent::planner::tests::w1_looks_like_path_rejects_root_and_dot_segments
    agent::planner::tests::w1_preflight_continue_message_must_not_be_hijacked_by_slash_in_history
    agent::planner::tests::w1_preflight_progress_check_must_not_be_hijacked_by_slash_in_history

test result: FAILED. 30 passed; 3 failed; 0 ignored; 0 measured; 1222 filtered out; finished in 0.07s
```

### 1.2 runtime::stream_support（契约 C6）

```
$ cargo test -p pony-agent-core --lib agent::runtime::stream_support -- --nocapture
```

退出码：**101**（测试失败 = 预期 RED）

```
running 8 tests
test agent::runtime::stream_support::w1_pending_control_match_tests::w1_missing_call_id_still_falls_back_to_pending_interaction ... ok
test agent::runtime::stream_support::w1_pending_control_match_tests::w1_mixed_pending_requests_hit_a_pending_one ... ok
test agent::runtime::stream_support::b010_timeout_signal_tests::promote_result_yields_same_timeout_signal_as_error_path ... ok
test agent::runtime::stream_support::b010_timeout_signal_tests::timeout_ms_buckets_follow_power_of_two ... ok
test agent::runtime::stream_support::w1_pending_control_match_tests::w1_call_id_still_matches_approval_by_id ... ok
test agent::runtime::stream_support::w1_pending_control_match_tests::w1_non_pending_approval_is_never_matched ... ok

thread 'agent::runtime::stream_support::w1_pending_control_match_tests::w1_missing_call_id_matches_pending_approval' (1460169) panicked at crates/pony-agent-core/src/agent/runtime/stream_support.rs:973:9:
契约 C6：tool_call.call_id=None 且存在 Pending Approval 时必须返回 Some(该 Approval)，实际 = None
test agent::runtime::stream_support::w1_pending_control_match_tests::w1_missing_call_id_matches_pending_approval ... FAILED

test agent::runtime::stream_support::w1_pending_control_match_tests::w1_empty_requests_yield_none ... ok

failures:
    agent::runtime::stream_support::w1_pending_control_match_tests::w1_missing_call_id_matches_pending_approval

test result: FAILED. 7 passed; 1 failed; 0 ignored; 0 measured; 1247 filtered out; finished in 0.00s
```

## 2. 用例清单（路径 / 行号 / 期望 / 实测）

| 契约 | 测试函数 | 路径:行号 | 期望 | 实测（当前代码） |
| --- | --- | --- | --- | --- |
| C1 | `w1_preflight_continue_message_must_not_be_hijacked_by_slash_in_history` | `crates/pony-agent-core/src/agent/planner.rs:1489` | None | **FAILED**（Some `workspace_gather_context`/`Read`, path=`/`） |
| C2 | `w1_preflight_progress_check_must_not_be_hijacked_by_slash_in_history` | `crates/pony-agent-core/src/agent/planner.rs:1509` | None | **FAILED**（同上，path=`/`） |
| C3 | `w1_preflight_keeps_referential_gather_context_for_history_path` | `crates/pony-agent-core/src/agent/planner.rs:1530` | Some, name→`workspace_gather_context`, path=`src/foo.rs` | PASS |
| C4 | `w1_preflight_keeps_explicit_path_gather_context` | `crates/pony-agent-core/src/agent/planner.rs:1558` | Some, name→`workspace_gather_context`, path=`src/foo.rs` | PASS |
| C5 | `w1_looks_like_path_rejects_root_and_dot_segments` | `crates/pony-agent-core/src/agent/planner.rs:1582` | `/`,`//`,`.`,`..`=false；`/home/a`,`a/b`=true | **FAILED**（`/` 被判为路径，首个反例 `"/"`） |
| C6 | `w1_missing_call_id_matches_pending_approval` | `crates/pony-agent-core/src/agent/runtime/stream_support.rs:968` | Some(Pending Approval) | **FAILED**（实际 None） |
| C6 保护 | `w1_missing_call_id_still_falls_back_to_pending_interaction` | `.../stream_support.rs:992` | Some(Interaction) | PASS（不得回归） |
| C6 保护 | `w1_call_id_still_matches_approval_by_id` | `.../stream_support.rs:1007` | Some(call_id 精确命中) | PASS |
| C6 边界 | `w1_non_pending_approval_is_never_matched` | `.../stream_support.rs:1025` | None（Consumed 不命中） | PASS |
| C6 边界 | `w1_mixed_pending_requests_hit_a_pending_one` | `.../stream_support.rs:1044` | 命中 Approval 或 Interaction 之一，且状态 Pending | PASS |
| C6 边界 | `w1_empty_requests_yield_none` | `.../stream_support.rs:1066` | None | PASS |

测试模块入口：`planner.rs:1481`（`// ── W1 RED（task-4）` 段落）、`stream_support.rs:899–905`（`mod w1_pending_control_match_tests`）。

## 3. RED / GREEN 判定

- **RED 成立**：C1、C2、C5、C6 主断言在当前生产代码下失败（planner 101 / stream_support 101）。
- **C3、C4 为保护性断言，当前即通过**：修复不得以「丢弃历史路径能力」或「放弃显式路径能力」为代价。
- 关键证据（C1/C2 根因）：历史 assistant 正文 `tester / reviewer` 经 `extract_path_candidates` 切出候选段 `"/"`，`looks_like_path("/")` 因 `contains('/')` 为真 ⇒ `infer_last_referenced_path` 返回 `"/"` ⇒ 兜底分支产出 `workspace_gather_context(path="/")`。
- C6 根因：`match_pending_control_request`（`stream_support.rs:657`）在 `call_id` 为 `None` 时只回退匹配 `Interaction`，从不回退到 `Approval`。

## 4. 已运行验证范围说明

- 仅按任务要求跑两个过滤（`agent::planner`、`agent::runtime::stream_support`），未跑全量 `cargo test`（机器负载约束）。
- 首次编译报错（`LocalTurnPlanner::preflight_decision` 用关联函数语法调用 trait 方法）已修正为 `LocalTurnPlanner.preflight_decision(...)`，修正后仅各跑一次，未原样重复运行。

## 5. 未覆盖 / 后续应由实现方与 Green 阶段承担

1. C6 的「按 session/turn/最新一条 Pending Approval 择优」——契约未冻结择优顺序，本 RED 只断言「无 call_id 且存在 Pending Approval ⇒ 非 None 且命中该 Approval」，未对多条 Approval 的择优规则下断言（避免过度约束实现）。
2. C6 未覆盖过期（`expires_at_ms < now`）Approval 的语义；当前函数不做时间判定，本波次不引入该约束。
3. 未做集成/端到端回归（`turn_control.rs:234` 调用点的行为回归）——按红线不在本任务范围，留给 Green 阶段。
4. 未新增任何 `pub` API；C5 通过模块内部 `#[cfg(test)] mod tests` 直接测私有 `looks_like_path`。
