# W1 GREEN 收据（task-5）

- 角色：Executor（实现；不改冻结测试，不做 git 写）
- 工作区：`/home/dm/pony-agent`
- RED 基线提交：`43176684`（`test(w1): freeze red-phase planner and approval matching tests`）
- 本次改动仅生产代码（5 个文件 / +214 / -2），**未改任何测试模块、未 git add/commit**：
  - `crates/pony-agent-core/src/agent/planner.rs`（+116 / -1）
  - `crates/pony-agent-core/src/agent/runtime/stream_support.rs`（+12 / -3）
  - `crates/pony-agent-core/src/agent/runtime/turn_control.rs`（+32 / -0）
  - `crates/pony-agent-core/src/agent/runtime/turn_sync.rs`（+19 / -0）
  - `crates/pony-agent-core/src/agent/runtime/turn_stream.rs`（+31 / -0）

## 1. 命令与退出码（物理凭证）

### 1.1 planner（契约 C1–C5）

```
$ cargo test -p pony-agent-core --lib agent::planner -- --nocapture
```

退出码：**0**

```
running 33 tests
...
test agent::planner::tests::w1_looks_like_path_rejects_root_and_dot_segments ... ok
test agent::planner::tests::w1_preflight_continue_message_must_not_be_hijacked_by_slash_in_history ... ok
test agent::planner::tests::w1_preflight_progress_check_must_not_be_hijacked_by_slash_in_history ... ok
test agent::planner::tests::w1_preflight_keeps_referential_gather_context_for_history_path ... ok
test agent::planner::tests::w1_preflight_keeps_explicit_path_gather_context ... ok
...
test result: ok. 33 passed; 0 failed; 0 ignored; 0 measured; 1222 filtered out; finished in 0.03s
```

### 1.2 runtime::stream_support（契约 C6）

```
$ cargo test -p pony-agent-core --lib agent::runtime::stream_support -- --nocapture
```

退出码：**0**

```
running 8 tests
test agent::runtime::stream_support::w1_pending_control_match_tests::w1_missing_call_id_matches_pending_approval ... ok
test agent::runtime::stream_support::w1_pending_control_match_tests::w1_missing_call_id_still_falls_back_to_pending_interaction ... ok
test agent::runtime::stream_support::w1_pending_control_match_tests::w1_call_id_still_matches_approval_by_id ... ok
test agent::runtime::stream_support::w1_pending_control_match_tests::w1_non_pending_approval_is_never_matched ... ok
test agent::runtime::stream_support::w1_pending_control_match_tests::w1_mixed_pending_requests_hit_a_pending_one ... ok
test agent::runtime::stream_support::w1_pending_control_match_tests::w1_empty_requests_yield_none ... ok
test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 1247 filtered out; finished in 0.00s
```

### 1.3 编译门禁（覆盖 W1b 的 turn_control / turn_sync / turn_stream）

```
$ cargo check -p pony-agent-core
    Finished `dev` profile [unoptimized] target(s) in 6.42s
```

退出码：**0**

### 1.4 挂起路径回归（W1b 不得破坏正常 Ask/Approval suspended）

| 命令过滤 | 通过数 | 退出码 |
| --- | --- | --- |
| `cargo test -p pony-agent-core --lib -- interactive_ask` | 2 passed | 0 |
| `cargo test -p pony-agent-core --lib -- governed_ask` | 6 passed | 0 |
| `cargo test -p pony-agent-core --lib -- keeps_ask_out_of_parallel` | 1 passed | 0 |

（含 `governed_ask_suspends_turn_...`、`governed_ask_stream_suspends_turn_...`、两个 batch-at-Ask 用例、并行段 Ask 用例。）

## 2. 契约 → 实现 file:line

| 契约 | 实现点 | 说明 |
| --- | --- | --- |
| C5 | `planner.rs:878`（`looks_like_path`）、`planner.rs:911`（`is_degenerate_path_segment`） | `""`/`/`/`//`/`.`/`..`（trim 后比对）判 false；`/home/a`、`a/b`、`src/main.rs`、`Cargo.toml` 仍为 true |
| C3（历史路径闸门） | `planner.rs:239`（`infer_local_tool_call` 的 `referenced_path`） | `explicit_path.or_else(|| has_referential_intent(msg).then(infer_last_referenced_path(history)))`：显式当前消息路径不受影响，历史路径只在指代意图下使用 |
| C3（指代识别） | `planner.rs:516`（`has_referential_intent`）、`planner.rs:917/925`（`contains_word_token` / `is_token_boundary`） | 中文子串：这个文件/这个目录/这个路径/该文件/该目录/该路径/它/上面/刚才/上述；英文**独立词**：this file/that file/the file/this|that|the directory/this|that|the path/it（`git`/`edit`/`item` 不误命中） |
| C1/C2 | `planner.rs:588`（`preflight_tool_decision` 首闸）、`planner.rs:554`（`is_continuation_without_explicit_path`） | 纯续写/追问（继续/接着/然后/检查进展/进展如何/进展情况/怎么样了/进展/continue/go on/go ahead）**且无显式路径** ⇒ 返回 `None`，交还模型 |
| C1/C2 一致性 | `planner.rs:159`（`select_tool_call` → `infer_local_tool_call`） | 两条入口共用同一 `infer_local_tool_call`，历史路径闸门（C3）因此对 `select_tool_call` 同样生效，不出现两套语义 |
| C6 | `stream_support.rs:662–684`（`match_pending_control_request`） | call_id 精确匹配优先不变；`call_id=None` 时回退集合 = `Pending` 的 **Approval ∪ Interaction**，多条取 `max_by_key(expires_at_ms)`（并列时取数组靠后一条）；返回类型不变 |

## 3. W1b「无请求不静默」实现

- 错误码与文案：`turn_control.rs:32`（`CONTROL_OUTCOME_PENDING_WITHOUT_REQUEST_CODE = "control_outcome_pending_without_request"`）、`turn_control.rs:39`（`build_control_outcome_pending_without_request_error()`，文案 `control_outcome_pending_without_request: 工具调用已挂起但未找到可恢复的待处理请求（call_id/审批丢失），本轮已按失败收口；请重试或重新发起该工具调用。`）
- 判据：`turn_control.rs:274`（`suspension_without_recoverable_request`）= `suspension.pending_request.is_none() && governed_dispatcher().is_some()`
- 同步调用点：`turn_sync.rs:190` → 复用 `fail_sync_turn_result`（phase=`failed`），由既有 `emit_sync_turn_failed` 发 `turn:failed`，`turn:end` 配对沿用既有出口
- 流式调用点：`turn_stream.rs:562` → 复用 `fail_stream_turn_gated_dispatch`（terminal-once 闸门 + `turn:failed` + 保留已产出正文 `accumulated_turn_text`）
- `pending_request=Some` 的原路径零改动：仍走 `build_suspended_turn_result` / `turn:suspended`（1.4 已证）
- 未新增事件类型；未改任何事件 schema。

## 4. 偏差与风险（需 Lead 确认）

1. **W1b 判据包含“治理 dispatcher 存在”这一合取项（唯一偏差）。** 严格按字面（`pending_request=None` 一律失败）会让 4 条既有冻结用例变红——它们用自定义 `ToolExecutor` 桩返回 `control_outcome_pending`、runtime 未挂 dispatcher，并断言 `phase == SUSPENDED_TURN_PHASE` / 有 `turn:suspended`：
   `run_turn_stops_batch_at_interactive_ask_and_leaves_remaining_calls_unexecuted`、
   `start_turn_stream_stops_batch_at_interactive_ask_and_leaves_remaining_calls_unexecuted`、
   `start_turn_stream_parallel_tools_keeps_ask_out_of_parallel_segment_and_still_terminates_batch`。
   故对“无 dispatcher ⇒ 本来就没有恢复通道”的桩执行器保留 legacy suspended；对治理路径（真实 `control_outcome_pending` 来源）严格执行失败终态。若要求字面严格语义，需由测试方/Lead 决定是否改这 3 条冻结用例。
2. 未跑全量 `cargo test`（按红线）；`agent::runtime::tests::` / `agent::control_plane` 全量未跑，仅跑 1.4 的挂起相关过滤。
3. 续写关键词含 `进展` / `然后` / `接着` 属宽匹配：无显式路径时会把“然后帮我看看编译错误”这类消息交还模型（宁可让模型答，也不让本地 planner 抢跳）。属行为收紧，可能影响依赖本地 planner 兜底的其它波次用例。
4. 指代意图清单是白名单：无指代词 + 无显式路径 + 历史有路径（如仅“看看”）不再触发历史路径收集。
5. C6 未按 `session_id/run_id/turn_id` 过滤（契约冻结时未要求），也未处理 `expires_at_ms` 过期语义。
