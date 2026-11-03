# 设计：Run 超时 promote（真·后台接续）· 契约 v4

> 目标：对齐 DSH `promoteOnTimeout` —— 前台命令到期不杀，转后台 job 继续，模型用 `job_output` 续取、`job_kill` 终止。
> 前置：B004（CT-5 hint 指向后台）已落地，模型已知道 `job_*` 存在；本波补上「到期不丢工作」的真能力。
> 侦察依据见 `.dev-team/contracts/tool-timeout-policy.md` §波次二（Run 进程所有权在 `ProcessManager`、`poll` 取走即清空、`kill_after` 无 disarm）。

## 1. 最小改造方案（不做 ProcessManager 所有权重构）

不搬移 `Child` 所有权，而是让 **jobs 侧持有一个 `(ProcessManager, session_id, handle)` 租借三元组**，输出靠「首读快照 + 后续 poll 追加」：

```
Run 超时命中
  ├─ promote 关闭 / 无 registry → 现状：kill + error.code=timeout（CT-P4 兼容路径）
  └─ promote 开启
       ├─ ProcessManager::disarm_kill_after(session_id, handle)   // 新增：原子标志，定时器线程检查后放弃 kill
       ├─ jobs::register_promoted(manager, session_id, handle, command_line, 首读快照, truncated 标志) -> job_id
       └─ 返回 ok 结果：{ timedOut:true, promoted:true, jobId, timeoutMs, stdout(已读部分), stderr, ... }
```

`job_output(promoted)` = `manager.poll()` 取增量 → 追加进该 job 的 2MiB RingBuffer → 按 cursor 返回；
`job_kill(promoted)` = `manager.cancel()`；
`job_list` 标记 `kind: "promoted"`。

## 2. 契约（v4）

| ID | 契约 | 验收 |
| :--- | :--- | :--- |
| CT-P1 | promote 开启（默认）时，Run 到期返回 **ok**：根级 `timedOut:true` + `promoted:true` + `jobId`（非空）；不再产生 `error.code=timeout` | 集成测试：`timeoutMs:300` 的 `sleep 5` 命令断言三字段 |
| CT-P2 | 该 jobId 可被 `job_output` 续读：进程在 timeout 之后继续产出的内容能被读到（证明进程未被杀） | 命令 `echo A; sleep 2; echo B`，timeoutMs 300 → 先拿 A，`job_output` 后拿到 B |
| CT-P3 | `job_kill` 对 promoted job 生效：之后 `job_output` 显示已退出/被终止，且无残留进程 | 同上命令，kill 后断言 state/exitCode |
| CT-P4 | promote 关闭（`PONY_AGENT_RUN_PROMOTE_ON_TIMEOUT=0`）时行为与波次一完全一致：`error.code=timeout`、无 `jobId`、进程被清理 | 环境变量关闭后跑同一命令断言错误路径 |
| CT-P5 | 原 `kill_after` 定时器对已 promote 的进程**不再开火**：超时时刻之后 2×timeout 仍存活 | 命令 `sleep 2; echo done`，timeoutMs 300；1s 后 `job_output` 仍 running，2.5s 后读到 `done` |

## 3. 实现要点（executor）

- `process.rs`：`ManagedProcess` 增 `kill_after_disarmed: AtomicBool`；`kill_after` 线程醒来先查该标志；新增 `pub fn disarm_kill_after(&self, session_id, handle)`。
- `jobs.rs`：`JobKind { Owned, Promoted { manager: ProcessManager, session_id: String, handle: String } }`；`register_promoted(...)`；`job_output`/`job_kill`/`job_list` 分支；promoted 输出追加进既有 `RingBuffer`。
- `tools.rs`：`run_command` 超时分支按 promote 开关分流；新增 `promote_switch()` 读 env（默认真）。
- 兼容：错误码、JobStart 语义、`job_output` 对外 JSON 键不删不改，只增 `kind`。

## 4. 风险

- `poll` 的「取走即清空」意味着 promote 后的输出只能靠 jobs 侧持续 poll；若两次 poll 间隔内 `BoundedBuffer` 溢出（64 KiB），promoted 输出会丢——首读后的 `truncated` 标记必须透传。
- `kill_after` 线程每 Run 一条（B006-③），promote 后进程存活更久，需确保 `shutdown(session)` 能收到（`ProcessManager` 的 session 仍是原 `legacy_run_session_id()`）。
- turn 结束/取消时 promoted job 的归属未定：本波约定「turn 结束不杀 promoted job，由 session shutdown 收口」，需在测试与文档写明。
