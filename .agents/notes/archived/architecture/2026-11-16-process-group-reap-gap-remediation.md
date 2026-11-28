# Agent Note: 子进程组回收缺口补齐（Drop / pipe-missing 分支 / updater 裸 Command / 模块文档失真）

Status: archived

工单：B205

## Problem

2026-11-13 的 40 个 PPID=1 孤儿死循环事故，执行者是 teammate `b137-flaky` 自建的 `taskset` 死循环压测命令
（`subagent-2cb8e2a0-…`，批次 A `seq 1 32` + `taskset -c 0`、批次 B `seq 1 8` + `taskset -c 12-15`）。
它的失效是**三重叠加**：清理原语不可用（dash 下 `$(jobs -p)` 恒空）、`pkill -f` 因 argv 改写永不匹配且
自匹配自杀、以及**运行时侧命令自行退出后不收进程组**。前两层属生成侧，已由
`implemented/process/2026-11-16-counterexample-cmd-safety-contract.md` 的 CX-SAFE 1–5 契约与 lint `exit 3`
治理；本条治第三层与其余同源缺口——本仓进程管理器对「进程组」的收口不完整。

以下五处经 E2 审计确认（`path:line` 均已本机复核），**第 0 项是已确认的现行缺口**（事故实际命中），
其余四项为审计确认的潜在缺口：

0. **`kill_inner` 的 `already_exited` 分支在 POSIX 侧不 `killpg`（现行缺口，事故实际命中）。**
   `process.rs:440-460`：子进程已自行退出时，只做 `#[cfg(windows)] entry.job.terminate(1)`（`:448`）、
   关 stdin（`:452-453`）、删条目（`:455-459`）、`return Ok(())`（`:460`）——**unix 侧无 `killpg`**。
   而压力命令**自己正常 `exit 0`**，于是永远走这条分支、永远到不了 `terminate`(`:1013`)/`killpg`(`:1040`)；
   已 spawn 的 bash 孙进程（`process_group(0)` 建的组，`process.rs:689`）由 `sh -lc` 包裹（
   `tools/run.rs:1077-1081`）→ 全组无人收 → 全部 PPID=1。**这是三层中唯一「无论命令怎么写都必须成立」
   的安全网，也是本次事故真正漏掉的一层**。executor 已认领（task-3 组 A）。

1. **无 `impl Drop for ProcessManager`。** `crates/pony-agent-core/src/agent/process.rs` 只有
   Windows 侧的 `struct NativeProcessHandle` 带 `Drop`（`:1733`），管理器本体没有。后果：管理器随会话
   被 drop（正常收尾、提前返回、panic 展开）时，仍在运行的受管子进程与其进程组**不受影响**——只能指望
   调用方显式 `shutdown`。
2. **unix pipe-missing 分支已 spawn 子进程却直接 `return Err`。** `process.rs:815-845` 的
   `child.stdout.take()` → `None` 分支，在 `:843-844` 置 `let job_error: Option<String> = None;` 后
   `return Err(...)`，Windows 支会 `job.terminate(1)` + `kill_and_reap`，**unix 支什么都不做** ⇒ 已 spawn
   的子进程原样泄漏。**当前不可达**：`Stdio::piped()` 恒设于 `:673-675`，`missing_pipe` 注入是
   `#[cfg(all(windows, test))]`（`:811`）。因此这是**潜在缺陷，不是现行回归**——但它是「改一行 cfg 就
   变成现实」的悬空泄漏路径。
3. **`updater.rs` 用裸 `std::process::Command::new`（`:1200`），无 `process_group(0)`。**
   超时只 `child.kill()`（`:1218`/`:1228`）⇒ 杀的是直接子进程；更新器拉起的进程若自身 fork（安装器
   / 脚本），孙进程逃逸出管理器视野。这与 `process.rs:689` 的 `process_group(0)`（B009）不是同一收口面。
4. **模块文档失真。** `process.rs:26` 仍写 "non-Windows behavior is direct-child lifecycle only"，
   而 B009 已把 POSIX 侧改为「自成进程组长 + `terminate` 内 `killpg`」（`process.rs:683` 注释、
   `:689` `process_group(0)`、`:1035-1040` `libc::killpg(pgid, SIGKILL)`）。文档比实现落后，后继维护者
   会据此判断「按组回收不存在」并回退——文档失真是会引发回退的缺陷，不是措辞问题。

**不属缺口（已收口，勿当缺陷改）**：`kill`(`:413`)、`kill_if_exists`(`:419`)、`cancel`(`:466`)、
`kill_after`(`:475`)、`shutdown`(`:538`) 五条公开入口均汇聚到 `terminate`(`:1013`)，其 `killpg` 在
`:1040`、有界 reap 在 `:1043-1056`（`KILL_REAP_ATTEMPTS` 轮询 `try_wait`）⇒ 超时/取消/中断路径**已**按组
回收——但「子进程**自行**退出」这条路径在进 `terminate` 之前就被 `already_exited` 分支拦下（见第 0 项），
两者不矛盾：显式终止路径是好的，缺的是自行退出路径。
`setsid` 与双 fork 逃逸是 `sandbox.rs:11`/`:40` 自述的已知局限（Unix 进程组为 best-effort
containment，不承诺阻止逃逸），**不属于回归**。

## Proposal

按「最小闭包 + 先红后绿」补齐五项，逐项独立可验，不夹带重构：

0. **`already_exited` 分支补 `killpg`（第一优先，事故实际命中项）**：`kill_inner` 在 POSIX 侧进入
   `already_exited` 后，仍应 `killpg(pgid, SIGKILL)` 收整组，再关 stdin / 删条目；Windows 侧保持
   `job.terminate(1)` 现状。语义变化须显式声明：`kill` 从「杀直接子进程」升级为「杀整组」，对已自行退出
   的子进程也生效（这正是期望）。executor 已认领，红相由 test-agent 冻结。
1. **补 `impl Drop for ProcessManager`（POSIX 侧）**：drop 时对仍在册的运行中条目 `killpg(pgid, SIGKILL)`，
   **不做**阻塞式 reap（`Drop` 期间不可长时间阻塞）：只发信号 + 关闭句柄，回收交给内核与 init。
   若需要「有界 wait」，必须挂在显式 `shutdown()` 上，不得塞进 `Drop`。
2. **unix pipe-missing 分支二选一**（禁止留着无注释的悬空泄漏）：
   - (a) 与 Windows 支对齐：`return Err` 前先 `killpg` + 有界 `kill_and_reap`，错误并入 `job_error`；或
   - (b) 既然不可达，就在分支处写明不可达前提（`Stdio::piped()` 恒设 + 注入仅
     `cfg(all(windows, test))`）并 `debug_assert!` 守住，一旦有人改 cfg 立刻红。
3. **`updater.rs` 收口**：要么复用 `ProcessManager`（统一收口面，代价是引入会话语义），要么就地补
   `process_group(0)`（`:1200` 之后）+ 超时分支改「先 `killpg` 再 `child.kill()`」（`:1218`/`:1228`）。
   二选一，不做第三条。
4. **模块文档回填**：把 `process.rs:26` 改为反映 B009 现状（POSIX 侧为进程组长 + `terminate` 内 `killpg`，
   并注明 `setsid`/双 fork 不承诺阻止）；以 `rg` 检索收据证明旧表述已无残留。
5. **红相先冻结**：由 test-agent 按 `test-expert` 产出
   `crates/pony-agent-core/tests/process_group_reap_test.rs`（断言 `kill(-pgid, 0)` 轮询返回 `ESRCH`，
   覆盖「超时终止」与「显式 terminate/cancel」两条真实路径，命令派生孙进程），`--test-threads=1`；
   冻结后 executor 只读不改。

## Alternatives considered

- **只在 `shutdown()` 显式收口，不加 `Drop`**：否决。`shutdown` 依赖调用方正确调用，而管理器被 drop 的
  路径（提前返回、panic 展开、会话对象被替换）恰恰是调用方最不可能记得收尾的地方；本事故的教训就是
  「依赖链末的一步」= 单点失败。`Drop` 是「最后一道必达路径」，代价只是信号 + 关句柄（不阻塞）。
- **`updater.rs` 直接复用 `ProcessManager` 统一收口**：暂不采纳，登记为备选。收益是消灭第二套 spawn 面，
  代价是把会话/句柄/poll 语义塞进一次性更新器（当前它只需「跑一个命令、超时就停」），属扩面重构，
  违反最小闭包；先就地补 `process_group(0)` + `killpg`，若后续出现第三处裸 `Command` 再回头统一。
- **用外部 supervisor / cgroup 做整组回收**（与 CX-SAFE 契约里同一备选）：不作为本仓代码路径。收益是
  内核级兜底，但要求宿主级基础设施与跨平台双实现（Windows 走 Job Object），本仓当前不承诺该运行时前提。
- **只做 CX-SAFE 契约、不动运行时代码**：否决。契约约束的是**审查产物文本**，运行时子进程由代码路径
  拉起——两条面没有交集，事故的两条成因必须分别治理。

## Acceptance criteria

- 红相先红：`process_group_reap_test.rs` 在植入缺口时真实失败（打印 `ESRCH` 不成立的实测输出），
  修复后转绿；`cargo test --test-threads=1` **Exit 0**。
- 逐项可验证据：
  - ⓿ `already_exited`（第一优先）：命令**自行正常退出**（`exit 0`）后，其孙进程进程组必须在有界时间内清空
    （`kill(-pgid, 0) == ESRCH` 轮询）；红相须先在现状代码上真实失败——这正是 2026-11-13 事故的复现路径
    （`sh -lc 'sleep 30 & sleep 30 &'` 之类自行退出 + 后台孙进程）；
  - ① `Drop`：管理器 drop 后 `kill(-pgid, 0)` 返回 `ESRCH`（或信号已送达且组内无存活），测试覆盖
    「drop 前未显式 kill」的场景；
  - ② pipe-missing 分支：命中 (a) 时该分支补 `killpg` + reap 并可被单测触发；命中 (b) 时有注释 +
    `debug_assert!`，且 `rg` 收据显示该分支仍不可达；
  - ③ `updater.rs`：子进程派生孙进程的替身（脚本 fork 一个 sleeper）在超时收口后不留存活进程，断言
    `kill(-pgid, 0) == ESRCH`；
  - ④ 文档：`rg -n "direct-child lifecycle only" crates/pony-agent-core/src/agent/process.rs` 无命中
    （检索收据贴入交付报告）。
- 不作恶：测试与反例命令自身不得泄漏后台进程（受 CX-SAFE 约束，禁 `while :; do :; done`）。

## Risks

- `already_exited` 补 `killpg` 会**扩大 `kill` 的语义**（从「杀直接子进程」到「杀整组」）：若某个调用方
  依赖「子进程已退出则 kill 是空操作」，改动后它会顺带杀掉该组内其它后继进程。上线前须检索 `kill` /
  `kill_if_exists` 的调用点，确认没有「同组复用」场景（进程组 ID 回收同理）。
- `Drop` 里发 `killpg` 存在 **pgid 复用**窗口：子进程若已退出并被 reap，`pgid` 可能已被内核复用给他人，
  盲发 `SIGKILL` 会误伤无关进程。必须与现有 `terminate` 同款防护（先 `try_wait` 确认未退出，再取 pgid）。
- `Drop` 期间不可 panic：所有 `Result` 只能丢弃，`Mutex` 中毒需按既有风格处理，不得 `unwrap`。
- Windows 侧语义不同（Job Object 已由 `NativeProcessHandle` drop 收口），改动必须 `cfg` 隔离，避免把
  POSIX 假设带进 Windows 路径。
- 若 ⓿①③ 改动触及 spawn 主路径，可能影响既有 `process` 单测与 `--test-threads=1` 下的时序断言；需要
  先跑全量再改，不得靠放宽断言过关。
- 本条是提案：**未实施部分不得被引用为已落地能力**（`implemented/` 那侧只登记契约与门禁决策）。
