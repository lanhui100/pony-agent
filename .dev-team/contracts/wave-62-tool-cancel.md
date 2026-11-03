# 契约：wave-62 — 工具级取消（wave-60 A9 清偿）

> 冻结方：Lead。Executor 与 Test Agent 只读本契约。
> 分级：**B 级**（跨 dispatcher/tools/process/runtime + 并发；命中 blast 关键字 thread/lock/spawn/Command::new）。
> 前置：wave-60（tag `wave-60`）已合入主干；本波补其 **A9 SCOPE-DEFERRED**：停止后长工具（`workspace_run_command` 等）不再等其自然结束。
> 隔离：实施在 worktree `/home/dm/wt/pony-wave62`，分支 `wave-62/tool-cancel`（主工作区被并发 wave-61 占用，禁止在其上写）。

## 0. 非目标
1. 不改 provider 读通道（wave-60 C4 已完成）。
2. 不引入 async 化改造；沿用同步 `ProcessManager` 轮询模型。
3. 不改 TUI/桌面端停止交互（wave-60 已完成）。
4. 不覆盖所有工具：**仅**要求 `workspace_run_command`（子进程类）与 dispatcher 通用闸门；其它工具沿用"下一检查点退出"。

## 1. 判据（black-box）

### C1 每轮一个取消令牌（core）
- `ExecutionControlRegistry::register_turn` 为该 turn 创建 `Arc<CancellationToken>`（幂等：重复注册沿用同一实例更佳，允许覆盖但必须与 `stop_requested_at_ms` 结转语义一致）。
- `request_stop(turn_id, reason)` 被**接受**时 `cancel()` 该 token；未被接受（not_found/already_terminal）**不得** cancel。
- 提供只读访问器 `cancellation_token(turn_id) -> Option<Arc<CancellationToken>>`；未知 turn 返回 `None`。
- 观测：`token.is_cancelled()`。

### C2 dispatcher 使用 turn 令牌（工具调用可被同轮停止打断）
- `DispatchContext` 增 `turn_cancellation: Option<Arc<CancellationToken>>`（`#[derive(Clone)]` 保持）。
- 该字段为 `Some` 时，dispatcher 的每次调用**不得**再新建独立 token 而不观测它 —— 必须是"turn token 被 cancel ⇒ 本次调用在闸门处 fail-closed 为 `Cancelled`"。
- 该字段为 `None` 时行为**完全不变**（向后兼容，既有测试不得回归）。
- 观测：注入一个"阻塞到取消"的测试工具，`cancel` 后 dispatcher 返回 `cancelled` 语义错误（既有 `DispatchErrorKind::Cancelled` / `ToolOutcome.status=="aborted"` 词表，不得新造）。

### C3 子进程类工具在取消时被真正终止
- `ToolExecutionContext` 增 `cancellation: Option<Arc<CancellationToken>>`。
- `workspace_run_command` 在执行期间**轮询**该 token（间隔 ≤100ms）；被取消时终止子进程，返回 `status="aborted"`、错误码沿用既有 `tool_cancelled`（不新增码）。
- 判据：命令 `sleep 5`（或平台等价）在 token cancel 后 **≤1s** 返回 aborted，且**无残留子进程**（`try_wait` 证实已退出）。
- 未取消时：正常命令行为不变（既有 run_command 用例全绿）。

### C4 回归边界
- 既有 `cargo test --lib` 全绿（基线 1209 passed，见 `.dev-team/regression-baseline-wave60.json`）。
- wave-60 六个验收 test 全绿（tag `wave-60-red` 冻结的用例不得改）。
- `provider_stream_guard_red`、`dispatcher_matrix`、`tool_*` 相关既有套件全绿。

## 2. 写域
| 任务 | write_scopes |
|---|---|
| T0 protocol | `crates/pony-agent-core/src/agent/budget.rs`、`dispatcher.rs`（仅 DispatchContext 字段）、`tools.rs`（仅 ToolExecutionContext 字段）、`execution_control.rs`（仅访问器/字段） |
| T1 test | `crates/pony-agent-core/tests/wave62_*.rs` |
| T2 executor | `crates/pony-agent-core/src/agent/{execution_control.rs,dispatcher.rs,tools.rs,process.rs,runtime/}` |

## 3. NFR（本波增量）
| key | 值 |
|---|---|
| `turn_cancel_poll_ms` | 100（子进程轮询间隔上界，实测 ≤250 视为达标） |
| `turn_cancel_exit_bound_ms` | 1000（cancel 到 run_command 返回 aborted 的硬上界） |
| `terminal_emit_idempotent_per_turn` | 沿用 wave-60 |
| `logging` | 新增行不得裸 eprintln 承载业务错误（沿用 wave-60 判据） |

## 4. 门禁
1. `CARGO_TARGET_DIR=/home/dm/pony-agent/target-check cargo test --manifest-path crates/pony-agent-core/Cargo.toml --test wave62_*` Exit 0（红相阶段必须失败）
2. `cargo test --lib` 无新增失败
3. `cargo check --all-targets` Exit 0
4. wave-60 六测试全绿（不得修改其断言）

## 5. 补遗 v2（Lead 一票裁决，2026-11-03，双路 L3 均 FAIL 后生效）

**BLOCKING（两路独立复现，同一根因）**：`GovernedToolExecutor → GovernedDispatcher → RouterPrimitiveHandler` 在桥接处用 `ToolExecutionContext{..Default::default()}` 重建上下文，且 `PrimitiveToolHandlerRequest` 无取消字段 ⇒ `turn_stream.rs` 注入的 turn 令牌在这一层丢失；真实 app 中 stop 后 `Run` 仍跑完（wave-60 A9 未清偿）。冻结 C3 用例直连 `ToolRouter`，覆盖不到该桥接。

| # | 追加判据 | 说明 |
|---|---|---|
| **C5（新增，端到端）** | 经**生产默认执行器** `build_governed_executor_with_sandbox_backend(...)` 执行 `Run`（`sleep 1.5; echo leaked > marker`），`DispatchContext.turn_cancellation=Some(token)`；执行 200ms 后 `token.cancel()` ⇒ **≤1000ms** 返回 `status=="aborted"` 且错误码 `tool_cancelled`，并且 marker 文件**不存在**（副作用未发生）。 | 判据必须由 Test Agent 写入 `tests/wave62_tool_cancel_e2e.rs`（新文件），实现后必须绿 |
| **A1 桥接修复** | `PrimitiveToolHandlerRequest` 增取消字段并把 `ToolExecutionContext.cancellation` 透传到 `RouterPrimitiveHandler` 重建的上下文；不得用 `Default::default()` 丢掉取消态。 | 允许改 `tool_runtime.rs` / `governed_executor.rs` / `tools.rs` |
| **A2 融合而非复用（L3-A ADVISORY 2）** | dispatcher 不得直接"复用 turn 令牌作为本轮令牌"（否则 composite `children.cancel()` 会 cancel turn 令牌、毒化整轮）。改为：dispatcher 保留自己的调用令牌，闸门/收口判「自有令牌 OR turn 令牌」；`None` 时行为不变。 | 既有 `child_cancellation_fails_closed_as_cancelled` 必须继续绿 |
| **A3 取消路径对称清理（L3-A ADVISORY 1）** | `cancelled` 分支须与 `timed_out` 分支同样做幂等二次 kill + 进程条目回收（`kill_if_exists`/`try_wait` 证据），不得让被取消命令的进程条目滞留。 | |
