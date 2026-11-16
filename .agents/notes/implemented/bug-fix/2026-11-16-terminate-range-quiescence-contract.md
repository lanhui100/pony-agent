# Agent Note: 终止完成条件收紧为「范围已证明为空」（B206 契约收口）

Status: implemented

工单：B206（缺陷 P0；B205 遗留契约漏洞）
契约真源：`.dev-team/contract-range-quiescence.md`（v1.3 冻结；Lead 冻结口径 C1–C6，本 ADR 记决策与权衡，不复制口径细节）
前序记录：`.agents/notes/proposed/architecture/2026-11-16-process-group-reap-gap-remediation.md`（B205）：
本条是它的**加强**而非替代——B205 解决「补一次刀」，B206 解决「刀有没有真砍到」。
后续记录：`.agents/notes/implemented/architecture/2026-11-16-native-range-owner-systemd-scope.md`（B207 把「谁拥有范围」
从 pgid 换成可替换 owner，语义仍由本条 C1 承担）。
Supersedes: 上述 B205 提案第 3 项（`updater.rs` 进程组 + `killpg`）已由
`.agents/notes/implemented/bug-fix/2026-11-16-residual-orphan-leak-sweep.md`（B208）落地取代，待该文件 owner 同批链回。

## Problem

2026-11-13 的 40 个 PPID=1 孤儿事故之后，B205 把「组长自行退出」这条路径补上了 `killpg`
（`ProcessManager::kill_inner` 的 `already_exited` 分支）。但 B205 修的是**信号有没有发出去**，
不是**范围有没有真的变空**，落地点当时有四条仍然会「谎报成功」的面：

1. `terminate` 在 `killpg(pgid, SIGKILL)` 之后只做「直接子进程 `kill()` + 有界 `try_wait` 轮询」
   （`KILL_REAP_ATTEMPTS=30` × `KILL_REAP_INTERVAL=10ms`），然后 `Ok(())`。`try_wait` 只证明**直接子进程**退了；
   组内孙进程若处于 D 状态（uninterruptible I/O，SIGKILL 也切不断）或 TERM 被吞、KILL 尚未生效，
   **范围仍然非空，却返回成功**。
2. `kill_inner` 的 `already_exited` 分支发完 `killpg` 就**无条件**关 stdin + `remove(handle)` + `Ok(())`；
   `killpg` 返回值被 `let _ =` 丢弃 ⇒ 失败不可见；条目被移除 ⇒ **后续再想补刀已经没有对象**。
3. `shutdown` 把 `terminate` 的错误降级成 `eprintln!` 后**继续 remove 并计入 `count`** ⇒ 上层永远看到「成功 n 个」。
4. `shutdown` 直接调 `terminate`，**绕过** `kill_inner` 的 `already_exited` 分支；而 `terminate` 在状态已 `Exited` 时
   直接 `return Ok(())`（POSIX 侧一个信号都不发）⇒「会话收尾时组长已退出、孙进程仍活」这一**事故原始形态**
   在 `shutdown` 上既不补刀也不探测，直接报成功。

四处的共同失效形态是：**完成条件 = 「信号已发送」**。上游（run 超时、会话结束、取消、插件 host）拿到 `Ok`
后会认为资源已释放，进而复用端口/目录/让新命令进入同一工作区，而孤儿仍在跑（B205 事故的 2 天 16 小时就是这样长出来的）。
DSH 侧的对应教训写在 `scripts/run-gates.ts:1350-1364`：`close` 只意味着直接子进程的 stdio 关了，必须先确认进程组与后代
全部消失才 settle；8s 到点仍活就打印 `gate tree not quiescent after 8s` 并按失败记录——**宁可失败，也不报告静止**。
DSH 更把这点做成结构性的：`packages/subprocess/subprocess-local/src/spawn.ts:330-370` 里 `terminate()` 只负责
「发 TERM、起 grace 定时器、到点发 KILL」，而**完成**由独立的 `waitForExit()`/`observeRangeExit()` 观察
`launch.owner.waitForExit()` 决定（`rangeExitObserved` 首次确认为空即「不再发信号的永久边界」）。
即：**发信号与判完成是两个不同的事实**，本仓曾把它们合并成同一个 `Ok`。

## Decision

`ProcessManager` 的全部终结入口的完成条件，是「受管范围**已证明为空**」；实现上收敛为一条唯一终止路径：

1. **唯一终止路径（C1-U）**：`terminate_range_path`（`crates/pony-agent-core/src/agent/process.rs:399`）是唯一做
   FENCE + LADDER + PROBE 的地方。五个入口 `kill` / `kill_if_exists` / `cancel` / `kill_after` / `shutdown`
   （以及加法式的 `shutdown_checked`）全部汇聚到它；其它入口不再自带发信号或自带 `Ok` 早退。
   收据：`libc::killpg` 在 `process.rs` 全文件**唯一 1 处**派发（位于 `fn signal_range`）。
2. **完成判据（C1）**：只有「`kill(-pgid, 0)` 返回 `-1` 且 `errno == ESRCH」**或**「扫 `/proc` 无 `pgrp == pgid`
   的非僵尸成员」成立才返回成功；两条判据都拿不到「空」⇒ **`Err`（不可证明 ≠ 空）**。
3. **信号阶梯与总死线（C2）**：`SIGTERM` → `term_grace`（默认 250ms）→ 未空则 `SIGKILL` →
   到 `quiesce_deadline`（默认 5s，自首次信号起算）→ 仍未空 ⇒ **`Err`**；轮询是显式条件轮询（每轮重新探测），
   不是固定 `sleep` 后自称静止。
4. **句柄不提前移除（C3）**：直系子进程已 `Exited` 而范围非空 ⇒ **保留**条目（`is_tracked == true`），
   仅当范围证明为空（或身份栅栏判定我方范围已消失）后才 `remove`。使 `Drop` 有作用对象。
5. **幂等语义不变（C4）**：`kill_if_exists` 对未知 handle 仍 `Ok(())`，`kill` 对未知 handle 仍 `Err`，
   跨 session 句柄仍拒绝；幂等分支不为满足 C1 引入探测或延迟。
6. **start-time 身份栅栏（C5，三态四分支）**：spawn 时记录 `/proc/<pid>/stat` 字段 22（`starttime`），每次终止的
   **第一动作**是 `fence_verdict` 比对。三态：`Ok{reaped:false}`（start-time 相等 ⇒ 继续 LADDER）；
   `Ok{reaped:true}`（`/proc` 不存在**且**组长**已被**本管理器 `wait()` 收尸 ⇒ **栅栏通过**，继续 LADDER，
   记 `code="B206-fence-reaped"` debug）；`Stale`（start-time 不等，**或** ENOENT 且组长**未被**收尸 ⇒
   **零信号**，视为「我方范围已消失」，`Ok` + `code="B206-fence-stale"`）。
   `reaped` 必须算通过：`wait()` 成功即内核已收尸，同一 pid 在 `wait()` 之前不可被复用 ⇒ 身份由内核 wait 语义直接证明；
   若把它判 `Stale`，「组长 exit 0 且已被收尸 + 孙进程仍在组内」这一**事故本形态**永远拿不到收组权 = 回退 B205 的全部价值。
   残余风险的闭环：收尸后 leader pid 若被复用成新组长，`killpg` 会误伤；但 (a) 只要旧组仍有成员，内核不会把该 pgid
   分配给别的组；(b) 一旦真复用，读到的 start-time 必不同 ⇒ 命中 `Stale`（零信号）。⇒ 不是赌运气，是可验证的逻辑闭环。
7. **Windows 语义不变（C6）**：Job Object 的 `KILL_ON_JOB_CLOSE`（`agent/process/windows_job.rs`）+
   `TerminateJobObject(1)` 继续作为 Windows 侧等价判据；POSIX 判据不进入 `#[cfg(windows)]`。
8. **`shutdown` = S-3 加法式 fail-loud**：新增 `shutdown_checked(&self, session_id) -> Result<usize, String>`
   （`process.rs:1046`）只统计**已证明范围为空**的条目，有未静默条目即 `Err`（含未静默 handle 列表 + 各自 pgid +
   已用信号阶梯）；既有 `shutdown(…) -> usize`（`:1034`）**签名不变**、内部委托 checked 版，`Err` 时输出结构化前缀
   `[B206][shutdown-not-quiescent] …`，且返回值只含已证明为空者。既有调用方零改动，新调用方拿得到失败信号。
9. **`Drop` 落在 `Inner` 而非 `ProcessManager`**：`ProcessManager` 是 `#[derive(Clone)]`（`inner: Arc<Inner>`），
   对它实现 `Drop` 会在**每一个克隆**析构时收组 = 新增「随手丢一个克隆就杀掉别的调用方正在用的进程」的破坏面；
   `Inner`（`impl Drop for Inner`）由 `Arc` 共享，其 `Drop` 恰好等价于「最后一个 owner 消失」，是正确时机。
   `Drop` 内的收组**复用唯一终止路径**（不复制 PROBE/FENCE/LADDER），有界、不 panic、不无限阻塞。
10. **红相先冻结**：承载用例由 test-agent 冻结在 `crates/pony-agent-core/tests/range_quiescence_test.rs`
    （含 dead-line 耗尽、`already_exited` 非空保留句柄、栅栏 stale 零信号三条红相）+ B205 的
    `tests/process_group_reap_test.rs` 正控，`--test-threads=1` 运行。

## Alternatives considered

- **只加一次 `killpg`、不验静默**：否决。这正是 B205 留下的漏洞本身——`killpg` 返回 `EINVAL/EPERM` 或被 `let _ =`
  丢弃时，调用方看到的仍是 `Ok`。B205 解决「补一次刀」，本条解决「刀有没有真砍到」；把前者当成后者，等于把
  「发过信号」重新定义为「已完成」，是本条明确废除的口径。
- **`waitpid(-pgid, …)` 等待整组退出**：否决。`waitpid` 只能回收**本进程自己的子进程**；`process_group(0)` 建组后，
  组长退出时孙进程已 reparent 到 init（或被 subreaper 收养），对它们 `waitpid` 只会 `ECHILD`。把它当完成判据会在
  「组长已退、孙进程仍活」这个**事故真实形态**上给出假成功，比现状更糟。「进程组是否为空」必须用信号 0 探测或
  `/proc` 扫描观测，不能用 `wait` 家族推导。
- **上 cgroup / systemd scope 做原生归属**：不以本条承载，拆为 B207 独立落地（已实现：
  `.agents/notes/implemented/architecture/2026-11-16-native-range-owner-systemd-scope.md`）。理由：① 需要 spawn 侧改造
  （argv 逐字透传、scope 建立/查询/清理）与 Windows 侧双实现；② 本机 `systemctl --user` 实测 `No medium found`，
  live 路径当时无法验证；③ C1 的语义在 pgid owner 与 native owner 下**完全一致**，先把语义收口不会白做，
  反而是 B207 的前置（B207 落地时只需替换 `wait_quiescent` 的实现——事后证明这条判断成立）。
- **把「范围未空」降级为只记日志**：否决。违反「不谎报静止」这条不可回退边界（契约 §6.1）：`Ok` 的上游会立即
  复用端口/目录，日志救不了竞态；B205 元复盘也已证明「会被关掉的噪声门禁比没有门禁更糟」——没人看的 warn 等于静默放过。
- **固定 `sleep(quiesce_deadline)` 后当作已静止**：否决。固定等待不是判据，只是把「没证明」包装成「等够了」。
  DSH `run-gates.ts:1350-1364` 明确拒绝以 `close`/时间假定静止；B205 测试文件头也禁止「sleep 一段时间后主观认为已清理」。
  等待必须被**探测结果**打断（提前成立即返回），也必须被**探测结果**否决（到点仍非空即 `Err`）。
- **把 `shutdown` 签名改为 `-> Result<usize, String>`**：否决。属破坏性公共 API 变更，按「契约单向门纪律」需先做
  消费方普查 + 兼容证据，本轮不该背这个成本；且它把「拿到失败信号」与「改所有调用方」绑成一件事，扩面大于收益。
  **只保存名 + 未静默条目仅进日志**同样否决：调用方拿不到失败信号，等于把「不谎报静止」降级为「翻日志才知道」。
  S-3 用「新增 checked + 既有签名委托」同时拿到两者，且零兼容成本（已实现）。
- **把 C1 只做成 `--debug`/环境变量开关下的严格模式**：否决。双模式等于把「是否谎报静止」交给部署配置；
  事故形态（孤儿常驻、端口占用）不区分 debug 与 release。降级只允许发生在**平台能力**上（无 `/proc`），且必须显式告警。

## Consequences

**落地事实（2026-11-16，工作树；本仓约定「已落工作树」记 implemented）**
- `process.rs` 侧：唯一终止路径 `terminate_range_path`、`fence_verdict` 三态、`shutdown_checked` 加法式、
  `impl Drop for Inner`；`libc::killpg` 全文件唯一 1 处派发。
- 冻结核对：`range_quiescence_test` **8 passed**、`process_group_reap_test` **4 passed**（B205 正控未漂移）、
  `--lib process` **29 passed**、`cargo check --tests` **零 warning**（Lead 报数）。
- 契约与实现的对齐由 `.dev-team/contract-range-quiescence.md` v1.3 冻结；判定表 T1–T14 的每行返回前必落 `probe=`
  证据字段（不探测的行除外）。

**行为变化（对调用方可感知）**
- `Ok` 的语义从「信号已发」变为「范围已空」。因此在「命令已退出但孙进程僵在 D 状态」等场景下，调用方会**开始收到**
  以前不会出现的 `Err`。已同批收口的两个点：`kill_after` 定时器路径用 `kill_if_exists` + 结构化日志；
  `shutdown` 提供 checked 版给需要失败信号的调用方，legacy 版保持签名但计数只含已证明为空者。
- 收尾最坏耗时从「发信号即返回」变成「最多 `term_grace + quiesce_deadline`」。同一 session 多条目时按条串行处理，
  每条共享同一个总预算上限；`Drop` 内同样有界（不做阻塞式长等）。

**残留风险与「靠 review」（常载命约 2：机器到不了的显式标注）**
- **`Err` 分支端到端不可测（`[靠 review]`）**：`killpg(SIGKILL)` 投递窗口短于一次 `/proc` 扫描，而 PROBE-B 不计僵尸
  ⇒ 首次扫描时目标通常已是僵尸 ⇒ 判空 ⇒ `Ok`（executor 实测 3/3 `Ok`，`nonempty_samples = 0`）。构造「连 SIGKILL
  都杀不掉」的成员需 D 状态或特权，本机不可达。机器可证的只有一半：`ZERO` 死线用例证明「只探测一次、不进入等待」。
  **已作废的推论（防复发）**：「SIGKILL 到清空必有非零延迟 ⇒ 立刻探测必然看到成员 ⇒ ZERO 必然 `Err`」被实测推翻，
  不得据它冻结红相或断言非空。
- **`impl Drop` 落 `Inner` 的偏差**（见 Decision 9）为 Lead 核准的既定形态；用例可判「丢一个克隆仍 `is_tracked`」
  与「丢全部克隆后范围收口」，但「任何中间 owner 析构都不收组」这一全称命题机器判不了，靠 review。
- **探针开销**：`scan_proc` 是 O(进程数) 的 `/proc` 遍历，先跑 `kill(-pgid,0)`（O(1)）作快路径、非空才升级到
  `/proc` 扫描取成员列表；每次收尾的扫描次数上限靠轮询间隔与死线共同约束。
- **平台降级**：macOS/BSD 无 `/proc` ⇒ C5 栅栏不可用、C1 只剩判据 A；「组空后 pgid 被复用」这一场景 A-only 无法区分，
  契约要求显式降级告警 + `probe=kill0-only` 证据。这是能力缺口而非已解决风险（B207 的 native owner 才是结构性答案，
  且其 live 路径本机亦未实测）。
- **栅栏自身竞态**：读 `starttime` 与发信号之间仍有窗口；栅栏把窗口从「无界」压到「微秒级」，不承诺消灭。
- **契约断言的「误杀合规实现」教训**：`Err` 模板用 Rust `{:?}` 打印 `members`（实际是 `[3100220, 3100221]`，逗号+空格），
  契约原正则要求紧凑形态 ⇒ 会把**当轮合规实现判红**；已改为容忍 `, ?`。同类教训：契约里的每条断言都必须先在实现产出的
  真实字符串上验一遍，否则断言本身就是缺陷源。

**与其它记录的关系**
- **B205**：本条加强而非替代；B205 的 `already_exited` 补刀被吸收进唯一终止路径（不再是分散的早退分支）。
- **B207**（`.agents/notes/implemented/architecture/2026-11-16-native-range-owner-systemd-scope.md`）：把「谁拥有范围」
  换成可替换 owner；C1 的语义（`Ok` ⇔ `wait_quiescent()` 成功）逐字不变，只是判据原语由 pgid 换成可插拔实现。
- **B208**（`.agents/notes/implemented/bug-fix/2026-11-16-residual-orphan-leak-sweep.md`）：清扫两处绕过
  `ProcessManager` 的泄漏点；C1 的适用面仍是「受管范围」，不追溯非受管进程，但「不谎报静止」原则对全仓生效。
- **状态轴说明**：本条由 `proposed/` 迁移至 `implemented/`，`## Proposal` 改写为现在时 `## Decision`，
  原 `## Acceptance criteria` / `## Risks` 折叠进本节；迁移时同步校准为落地后的实际形态（C1-U → `terminate_range_path`、
  S-3 → `shutdown_checked` + legacy 委托、FENCE → 三态四分支、`Drop` → `Inner`）。
