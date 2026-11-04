# wave-64 / T3（executor-rs）绿相收据

契约真值源：`.dev-team/contracts/wave-64-stop-terminal-status.md` §2 R1 / R2。
红相真值源：`crates/pony-agent-core/tests/stop_graph_run_wave64.rs`（只读，未改一行）。
日期：2026-11-04（Asia/Shanghai）

## 1. 改动点（仅 3 个文件，均在本任务写域内）

| 文件 | 改动 |
|---|---|
| `crates/pony-agent-core/src/agent/control_plane/mod.rs` | R1：`stop_graph_run` 改为「锁内只读快照 → 出锁受理 turn 停止 → 回填 `turn_stop`」三段式；活跃 turn 的停止改走 `stop_turn`（C2 同步恰好一条 `turn:stop_requested(stopReason=user)` + C5 宽限监督），不再直接 `execution_control.request_stop` |
| `crates/pony-agent-core/src/agent/runtime/turn_control.rs` | R2：新增纯函数判据 `classify_stream_error(cancel_requested, error) -> StreamErrorClass{Cancelled,Failed}`（+ 实现级真值表单测） |
| `crates/pony-agent-core/src/agent/runtime/turn_stream.rs` | R2：①followup provider 失败分支：取消码 + 停止位 ⇒ `cancel_stream_turn`（cancelled），否则逐字走既有 failed；②初始流式决策的两处 `or_else` fallback：停止位为真时不再降级同步决策，直接取消收口并返回哨兵；③决策失败分支：哨兵 ⇒ 只落 cancelled 相位（不重复派发），取消码 + 停止位 ⇒ `cancel_stream_turn`，其余逐字走既有 failed。新增 `cancel_stream_turn_for_stop` 参数装配 helper（语义复用 `cancel_stream_turn`） |

未改动：`crates/pony-agent-core/src/agent/stop_escalation.rs`、`src-tauri/src/lib.rs`（R1 由 core 层承载即足够，tauri 命令 `stop_graph_run` 原样调用 core）；
`runtime/turn_sync.rs` 按 Lead 裁决 A/C 不动；`turn_stream.rs:451-478`（工具 aborted 分支）按 Lead 裁决 A 不动（HEAD 已绿，作回归护栏）。

## 2. 锁序结论
- `stop_graph_run` 的两处 `graph_runs`（`std::sync::Mutex`）临界区内**不再调用** `stop_turn` / `turn_event_sinks` / `stop_escalation` / `load_execution_checkpoint`；只做 `load_run` + 终态判定 + `active_turn_id.clone()`、以及原有的 `graph_runner.request_stop` + evidence 落库。
- `stop_turn` 全程在 `graph_runs` 锁外，其内部按序取 `execution_control`（短锁）→ `turn_event_sinks`（取出后释放再 emit）→ `stop_escalation.emitters`（取走即释放）；监督线程只碰 `execution_control` 短锁，且 **emit 前已释放锁**。
- 因此不存在 `graph_runs ↔ execution_control / sessions_rwlock / turn_event_sinks` 的 AB/BA 反序。
- 附带修正（属 R1 最小闭包内）：原实现把 `load_execution_checkpoint` 放在 `graph_runs` 锁内，当 runtime checkpoint 缺失时会走 `resolve_graph_run_for_retrieval` → 重入同一个非重入 `Mutex` ⇒ **潜在自死锁**；移出锁后消失。

## 3. 门禁（命令原文 + 机器 Exit Code）

```
$ cargo test --manifest-path crates/pony-agent-core/Cargo.toml --test stop_graph_run_wave64 --target-dir target-test
running 5 tests
test r1_stop_graph_run_unknown_run_keeps_error_and_emits_nothing ... ok
test r2_tool_aborted_during_user_stop_terminates_cancelled_not_failed ... ok
test r1_stop_graph_run_dispatches_single_stop_requested_and_grace_cancels_turn ... ok
test r2_non_user_failure_still_terminates_failed ... ok
test r2_provider_stream_cancelled_terminates_cancelled_not_failed ... ok
test result: ok. 5 passed; 0 failed; 0 ignored
EXIT=0
```

```
$ cargo test --manifest-path crates/pony-agent-core/Cargo.toml --test stop_contract_wave60 --test stop_escalation_wave60 --test stop_interruptible_read_wave60 --target-dir target-test
stop_contract_wave60:              7 passed; 0 failed
stop_escalation_wave60:            9 passed; 0 failed
stop_interruptible_read_wave60:    3 passed; 0 failed
EXIT=0   （wave-60 零回归）
```

```
$ cargo check --manifest-path crates/pony-agent-core/Cargo.toml --target-dir target-check
Finished `dev` profile [unoptimized] target(s)
EXIT=0
```

补充（实现级单测，非门禁要求）：
```
$ cargo test --manifest-path crates/pony-agent-core/Cargo.toml --lib wave64_classify_tests --target-dir target-test
test agent::runtime::turn_control::wave64_classify_tests::r2_classify_stream_error_truth_table ... ok
1 passed; 0 failed; 1220 filtered out
EXIT=0
```

基线对照（改动前，同一命令）：`cargo check` EXIT=0；wave-60 三文件 7+9+3 全绿 ⇒ 本波改动未引入既有回归。

## 4. 未做 / 遗留风险
1. `runtime/turn_sync.rs`（非流式轮）仍是 `aborted → failed_sync`，按 Lead 裁决不动；桌面流式路径不走它。登记 backlog。
2. `provider_stream_cancelled` 若在**无停止位**时冒泡（例如外部中止且 registry 无停止请求）仍归 failed —— 与契约 R2 一致（不得把真实错误吞成取消）。
3. `stage`：R2 的 fallback 判定只覆盖「初始流式决策」与「followup 流式决策」两处取消码源；provider 读通道其他错误码（deadline/stalled）不参与取消归类。
4. 未见契约与代码现实的其它冲突。

---

# 追加：L3-A 复核整改（BLOCKING ×1 + ADVISORY ×4）

## 1. BLOCKING（terminal-once 闸门）

问题：C5 宽限监督 `escalate_now` 已置 checkpoint `cancelled` 并派发一条 `turn:cancelled`，
阻塞调用返回后 in-band 循环再命中停止位会**第二次**走 `cancel_stream_turn`（两条终态）；
failed 路径同理（C5 收口后真实错误再派发 `turn:failed`）。

修复（契约 §2 R1「恰好一条」）：
- `turn_control.rs` 新增 `claim_terminal_status(control, turn_id, target)`：以 checkpoint `status`
  为唯一真值源，`running` 时在 registry 锁内**原子 CAS** 到 `cancelled`/`failed`；已是任一终态
  ⇒ 返回 `false`（静默退出：不再 persist_trace、不再 emit）；无 checkpoint（未注册轮次）或
  非终态的非 running（如 `suspended`）⇒ 返回 `true`（既有行为不变）。
- 覆盖面：① `cancel_stream_turn` 入口（in-band 取消）；② 全部 `fail_stream_turn_with_hook_dispatch`
  调用点（改为过闸门包装 `fail_stream_turn_gated_dispatch`，参数逐字转发，15 处）；
  ③ 9 处内联 failed 终态派发块（`update_execution_checkpoint(... "failed") + persist + emit_turn_failed`）。
- 与 `stop_escalation::escalate_now` 的互斥：后者同样只在锁内 "`status == running` 才置 cancelled"，
  两者抢同一状态位 ⇒ 至多一方派发终态（`stop_escalation.rs` 无需改动，已核对）。
- 未涉及 `completed`：正常完成语义零改动（契约允许范围外，Lead 明示不改）。
- 沉默派发留痕：`log_terminal_dispatch_suppressed`（debug 级结构化日志，无 eprintln）。

## 2. ADVISORY 收口
- A2：`provider_followup_stream_with_mode` 内部使用 `ProviderStreamGuard::without_cancel()`
  （`continue_with_tool_result_stream`），该通道**永不产生** `provider_stream_cancelled`
  ⇒ 删除此前新增于 followup 失败分支的取消分流死代码，改为原地注释写明可达性依据；
  「停止请求 + 真实错误」在 C5 之后不会覆盖 cancelled，由 terminal-once 闸门兜住（R3 用例验证）。
- A3：`stop_graph_run` 把 graph 侧 `request_stop`（含终态校验）**前置**到 turn 停止之前
  ⇒ 「返回 Err ⇒ 零派发」；快照与受理之间的 run 终态竞态不再出现"已派发却返回 Err"。
- A4：两处「有停止位即取消」的 fallback 收口前补结构化 warn 日志
  （`log_stop_short_circuit`，含 turn_id / 原始 error / scope），不再丢根因，不新增 eprintln 面。
- A5：`stop_graph_run` 中活跃 turn 无 checkpoint（不存在/已回收）时补 warn 级结构化留痕，
  不再静默返回 Ok（图已停、轮未受理停止的事实可见）。

## 3. 整改后门禁（命令原文 + Exit Code）

```
$ cargo test --manifest-path crates/pony-agent-core/Cargo.toml --test stop_graph_run_wave64 --test stop_contract_wave60 --test stop_escalation_wave60 --test stop_interruptible_read_wave60 --target-dir target-test
stop_graph_run_wave64:            6 passed (含新增 r3_in_band_closure_after_grace_escalation_keeps_single_cancelled_terminal)
stop_contract_wave60:             7 passed
stop_escalation_wave60:           9 passed
stop_interruptible_read_wave60:   3 passed
EXIT=0
```

```
$ cargo check -p pony-agent-core --target-dir target-check
Finished `dev` profile [unoptimized] target(s)
CHECK_EXIT=0   （零 warning）
```

补充（实现级单测）：
```
$ cargo test --manifest-path crates/pony-agent-core/Cargo.toml --lib wave64_classify_tests --target-dir target-test
1 passed; 0 failed; 1220 filtered out
LIBTEST_EXIT=0
```

## 4. 整改后遗留风险
1. 闸门让"先到者"独占终态落盘：C5 抢到时 in-band 路径不再补写 cancelled trace（C5 原本就不落 trace），
   与前波行为一致，仅记录一条 debug 日志。
2. 非终态的非 running 状态（`suspended` 等）不参与闸门竞争（无并发终态可言）。
3. `turn_sync.rs`（非流式）仍按 Lead 裁决不动，其 aborted→failed 与 terminal-once 闸门无关。
