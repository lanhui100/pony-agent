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
- `tools.rs`：`run_command` 超时分支按 promote 开关分流；新增 `run_promote_on_timeout_enabled()` 读 env（默认真，`0`/`false`/`off`/`no` 关）。
- 兼容：错误码、JobStart 语义、`job_output` 对外 JSON 键不删不改，只增 `kind`。

### 3.1 实现期修订：`kill_after` 与 poll deadline 的竞态（executor，B005 实测）

设计原文只写「到期撤防」，实测不足：`kill_after` 在 **arming 后**才 sleep `timeout_ms`，而 poll 循环的
`deadline` 是 arming **之后**才算出的、且每 20ms 才检查一次 —— 定时器线程必然先于循环的到期判定醒来，
于是它会在 promote 之前把进程杀掉，CT-P1/CT-P2/CT-P5 必红（且是稳定红，不是偶发）。

因此 promote 开启时给安全网**加宽限**：`kill_after` 的时长取 `timeout_ms + PROMOTE_KILL_AFTER_GRACE_MS`
（1000ms），poll 循环在 deadline 处（≤20ms 内）醒来 → 先 `disarm_kill_after` → 再 `register_promoted`。
定时器保留为「循环崩溃/卡死」的兜底网：若撤防失败或注册失败，promote 分支整体回落波次一的
kill + `error.code=timeout` 路径（CT-P4 兼容语义），不会出现「既报 timeout 又留活进程」的半吊子状态。
promote 关闭时不加宽限，`kill_after` 时长与波次一逐字一致。

## 4. 风险

- `poll` 的「取走即清空」意味着 promote 后的输出只能靠 jobs 侧持续 poll；若两次 poll 间隔内 `BoundedBuffer` 溢出（64 KiB），promoted 输出会丢——首读后的 `truncated` 标记必须透传。
- `kill_after` 线程每 Run 一条（B006-③），promote 后进程存活更久，需确保 `shutdown(session)` 能收到（`ProcessManager` 的 session 仍是原 `legacy_run_session_id()`）。
- turn 结束/取消时 promoted job 的归属未定：本波约定「turn 结束不杀 promoted job，由 session shutdown 收口」，需在测试与文档写明。
- **已知风险：Windows 批处理脚本清理责任移交 job 生命周期（本波未实现）。** Windows 上 Run 先把命令写入
  `<workspace>/.tmp/pony-agent-run-{session_id}.cmd` 再 `cmd /C` 执行；promote 路径按红线**不得**调用
  `remove_windows_batch_script`（`promoted` 进程仍在读该脚本），于是该临时文件在 promoted 进程结束前不再被
  任何代码删除。本波只保证「不误删」，收口方式是随 `ProcessManager::shutdown(session)` / 工作区 `.tmp`
  清理由后续波次（job 生命周期收口）承担；若后续不做，Windows 工作区会累积一次性 `.cmd` 残片。
- **已知风险：截断标记是「粘性 OR」而非逐段精确。** promoted job 的 `stdout_truncated`/`stderr_truncated`
  由每次 `poll` 的布尔标记 OR 累计（`job_output` 以 `Some(bool)` 透传）；`poll` 只给布尔不给丢弃字节数，
  故无法区分「哪一段丢了」。


## 5. v7 收口：promote 为**默认开启（opt-out）**

> v6 曾因 A-3/A-6/B-1/B-2/B-4 五条 BLOCKING 收成「默认关闭（opt-in 预览）」；
> B008–B013 绿相落地后（v7 / CT-P9）默认值翻回**开启**：env 缺失即 promote，
> `0`/`false`/`off`/`no` 才回到波次一语义，未识别值回落默认并打 warn。

> 历史：v6 曾因 L3-A / L3-B 双路复核 `8488d05` 判 FAIL（3+3 BLOCKING），把默认值收成「关闭（opt-in）」。

**v7 实现（CT-P9，opt-out）**：`run_promote_on_timeout_enabled()` 的取值规则 ——
env **缺失** ⇒ 开启；`1`/`true`/`on`/`yes`（trim + 大小写不敏感）⇒ 开启；
`0`/`false`/`off`/`no` ⇒ 关闭；**未识别值 ⇒ 回落默认「开启」并打 warn**
（`[tools] unrecognized PONY_AGENT_RUN_PROMOTE_ON_TIMEOUT=…; falling back to default (promote enabled)`），
杜绝「开关拼错却静默生效」。关闭路径不延后 `kill_after`（宽限只属于开启路径），
行为与波次一逐字一致：`error.code=timeout` + 进程被清理。

**v6 曾提出的五条 BLOCKING —— 现均已绿相闭合（默认开启的前提）**

| # | v6 事实 | 闭合卡 | 现状 |
| :-- | :--- | :--- | :--- |
| A-3 | promoted 的 `ManagedProcess` 条目永不回收 | B008 | `release_promoted_entry()` 终态回收（`released` 守卫、放锁后调用） |
| A-6 | promote 后 `CommandCancelWatcher` 已 drop ⇒ 用户 Stop 停不下来 | B009 | job 侧 `spawn_cancel_watcher`（20ms tick）+ POSIX `process_group(0)`/`killpg` 收口 |
| B-1 | promote 摘掉 `error.code=timeout` ⇒ 熔断/指纹/盲重试失去输入 | B010 | `tool_failure_signal` 对 `ok + timedOut:true` 产出 `("…", 归一指纹, "timeout")` |
| B-2 | `timedOut/promoted` 无消费方，UI 判成「成功完成」 | B011 | 前端 `tool-result-meta.ts` + 工具行「已转后台」徽标 |
| B-4 | job 层无会话归属、Windows `.cmd` 残留 | B012 / B013 | `owner` 归属校验（`job_not_owned`）+ 脚本清理移交 job 生命周期 |

**本波同时修掉的两条低成本真缺陷（与默认值无关，关闭态也生效）**

- **CT-P7a：先 poll 再判超时。** 旧实现按「到期即判超时」，`echo hi` + `timeoutMs:1` 这类「命令早已正常退出、
  只是判定时刻已过」的调用会被误报成 timeout/promoted。现改为：每轮先 `poll`，退出即走正常成功路径；
  仅在「已 poll 且确认仍在 Running」之后才看到期，且到期后还有一次 20ms 确认轮（`TIMEOUT_CONFIRM_MS`），
  确认轮里退出同样按成功返回。
- **CT-P7b：ring buffer 溢出必须可见。** `RingBuffer::push` 返回被丢弃字节数；promoted 路径（首读快照 +
  每次 `poll` 追加）一旦丢弃字节就置对应流的 truncated 标记，`job_output` 以
  `stdoutTruncated`/`stderrTruncated` 透传，禁止静默丢字节。

**已知风险（保留）**

- **Windows 真机 e2e 未验证**：本机为 Linux，Windows 侧仅有编译期保证 —— `<ws>/.tmp/pony-agent-run-{sid}.cmd`
  的生命周期清理（B013）走同一终态回收点、Job Object 语义未改，但真机路径（kill/promote/脚本删除）
  尚未实跑；发版前需在 Windows 上补一次 promote 全链路 e2e。
- 关闭开关（`=0`）仍保留，供需要波次一语义（`error.code=timeout` + 立即 kill）的调用方使用。

### 5.1 实现期发现：`kill_after` 与「先 poll 再判超时」的相互作用

把 deadline 检查挪到 poll 之后，关闭路径立刻暴露一个反例（实测红）：关闭路径的 `kill_after` 仍按
`timeout_ms` 整点开火，而定时器比 poll 循环的到期判定早几微秒醒来 ⇒ 循环下一次 poll 看到的是
**被安全网 SIGKILL 的 Exited(-1)**，于是 `sleep 5` + 300ms 被误报成 `non_zero_exit` 而不是 `timeout`。

两条配套修正（都必要）：

1. `ManagedProcess` 增 `kill_after_fired: AtomicBool`：定时器**先留证再杀**，Run 的 poll 循环看到 Exited 时
   查 `ProcessManager::was_killed_by_kill_after(...)`；若为真则仍按超时结果返回，并把 `promote_on_timeout`
   置 false（绝不为「尸体」建 promoted job）。这样「安全网击杀」与「自然退出」不再混淆。
2. 到期后的 20ms 确认轮（`TIMEOUT_CONFIRM_MS`）**只在 promote 开启时执行**：关闭路径不能有确认轮——
   那 20ms 里安全网已经开火，确认轮只能看到 -1，反而把真超时变成 `non_zero_exit`。开启路径有
   `PROMOTE_KILL_AFTER_GRACE_MS` 的 1s 宽限，确认轮安全（也正因如此 `echo hi` + `timeoutMs:1` 在
   开启路径稳定返回成功）。
