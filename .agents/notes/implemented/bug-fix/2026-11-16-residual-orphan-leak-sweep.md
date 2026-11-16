# Agent Note: 遗留孤儿泄漏点清扫（updater 裸 Command 无进程组 + 验收脚本后台 server 无整组回收）

Status: implemented

工单：B208（缺陷 P2；B205 审计遗留泄漏点）
前序记录：`.agents/notes/proposed/architecture/2026-11-16-process-group-reap-gap-remediation.md`（proposed，B205 审计把这两处列为
「潜在缺口/LEAK-RISK」，其提案第 3 项即本条的 updater 收口）
契约约束：`.agents/notes/implemented/process/2026-11-16-counterexample-cmd-safety-contract.md`（反例命令安全契约 CX-SAFE 1–5，implemented）与
`../../../../.dev-team/contract-range-quiescence.md`（B206 范围静默契约） §8.2（C1 不追溯本条，但「不谎报静止」原则适用）

## Problem

B205 治理的是 `ProcessManager` 这条收口面（`spawn` 时 `process_group(0)` + 终止时 `killpg`）。但共用开发机上
「进程组没人收」的形态不止这一处，审计确认还有两条**绕过 `ProcessManager`** 的路径：

1. **`updater.rs` 的自更新器子进程**：`run_version_with_deadline`（`crates/pony-agent-core/src/agent/updater.rs:1197`）
   用裸 `std::process::Command::new(target)` 拉起新二进制的 `--version` 探测，**没有 `process_group(0)`**；
   超时/出错路径只 `child.kill()`（旧代码 `:1218`/`:1228`）⇒ 杀的是直接子进程。更新器拉起的安装器/脚本若自身
   fork，孙进程即逃出管理器视野，成为下一起「PPID=1 常驻」事故的原料。
2. **验收脚本里的常驻 server**：`tests/acceptance/l2at-tui-models.sh` 与 `l2at-tui-models-auth.sh` 都在后台起一个
   python `http.server`。旧的清理只做 `tmux kill-server`，**没有覆盖 server** ⇒ 每次运行（尤其早退/中断路径）
   泄漏 1 个常驻 server，而且泄漏发生在**验收脚本自己身上**——恰恰是最容易被当成「测试通过就没问题」的地方。

两条的共同形态与 2026-11-13 事故同源：**清理动作只覆盖「我直接创建的那个进程」，不覆盖它派生出来的整组**。
本条只清扫这两个具体泄漏点，不重新设计进程管理（那是 B206/B207 的题目）。

## Decision

**① updater 就地补进程组收口**（`updater.rs:1197-1260`）：

- spawn 前把命令构造改为可变 `Command`，在 `#[cfg(unix)]` 下设 `command.process_group(0)`（`:1205-1213`，
  闸门与形态照抄 `agent/process.rs` 的既有写法；Windows 侧语义不变）；
- 超时分支（`:1225-1245`）与 `try_wait` 出错分支（`:1246-1258`）都改为同一回收顺序：
  **`killpg(pgid, SIGKILL)` 收整组 → `child.kill()` → `child.wait()`**；`pgid` 取 `child.id()`（因已
  `process_group(0)`，`pgid == pid`）；组已空时 `killpg` 返回 `ESRCH`，无副作用。
- **不引入 `ProcessManager`**：自更新器是「跑一条命令、有死线、失败即报错」的一次性生命周期，套上会话/句柄/
  poll 语义属扩面重构（见 Alternatives）。

**② 验收脚本的 server 回收改为「整组 + 精确 pid」双保险**（两个脚本同形）：

- server 以 `setsid python3 … &` 启动，使其自成会话/进程组；`setsid` 不可用时降级为普通后台启动并打 WARN；
- 启动后把 pid 登记进 `SERVER_PIDS`，清理体 `b208_kill_servers` 对每个 pid 执行
  `kill -TERM -- "-$pid"`（打整组），再执行 `kill -TERM "$pid"`（setsid 不可用时的精确降级）；
- `cleanup_all() { tmux_cleanup; b208_kill_servers; }` 挂 `trap cleanup_all EXIT`（早退/正常结束都覆盖），
  并单独挂 `trap 'exit 130' INT` / `trap 'exit 143' TERM` 以保留中断语义的退出码。
- **明确不使用 `kill 0`**：脚本自身不一定是进程组长（CI/`sh -c` 链下与调用者同组），`kill 0` 会把
  SIGTERM 打给调用者进程组。该判断有本机实测支撑（见下面 Consequences 的实测收据）。

**③ 适用面声明（诚实边界）**：本条的改动**只保证「信号能覆盖整组」**，不保证「范围已证明为空」——
后者是 B206 的 C1，适用于 `ProcessManager` 的受管范围，**不追溯**本条两处。因此本条涉及的报告/日志措辞
必须止于「已发送 TERM/KILL」，**不得**写「已清理干净」（B206 契约 §6.1 的「不谎报静止」原则对全仓生效）。

**④ 落地形态与状态口径**：改动已落**共享工作树**（与 B205 的已落地部分同样未提交，因工作区存在他人未提交
变更，本轮全程禁止 git 写操作）。按本仓既有口径（对照
`.agents/notes/implemented/process/2026-11-16-counterexample-cmd-safety-contract.md` 同为 implemented + 未提交工作树），
本条记 `implemented`。**B205 提案第 3 项（`updater.rs` 收口二选一）因此已被本条取代**：
B205 提案需由该条 owner 在同批内链回本条（本 Agent 写域只含新建 ADR，不修改既有记录）。

## Alternatives considered

- **`updater.rs` 复用 `ProcessManager` 统一收口面**：否决（登记为备选）。收益是消灭第二套 spawn 面、
  判据只写一遍；代价是把会话/句柄/poll/缓冲语义塞进一次性更新器，且会让更新器依赖「会话」概念——它没有。
  属扩面重构，违反最小闭包。若出现**第三处**裸 `Command::new` 拉起长寿命进程，再回头统一（该触发条件可判定：
  `rg -n 'Command::new' crates/pony-agent-core/src | grep -v process.rs`）。
- **验收脚本只加 `kill $SRV_PID`（精确 pid）**：否决为主方案，保留为降级分支。python `http.server` 是单进程，
  精确 pid 够用；但脚本还会派生 `curl`/`tmux`/子 shell，清理只覆盖登记过的 pid 时，一旦新增派生点忘记登记就
  重新漏。`setsid` + 组信号是「一次覆盖整组」的形态，精确 pid 作为 setsid 缺失时的兜底。
- **清理体用 `kill 0`（杀自己所在进程组）**：否决，尽管它写起来最短。本机实测：非作业控制调用下子 shell 与
  调用者**同进程组**（收据见 Consequences），`kill 0` 会向调用者组发 SIGTERM——在 CI runner 或 Agent 的
  `bash -c` 链里等于**杀掉调用者**。这不是理论风险，是仓库里已有形态实测可复现的误杀面。
- **不修脚本，只加一条 lint 门禁（检出「后台 `&` + 无 trap」）**：否决为**替代**，保留为**补充**。门禁管的是
  文本（`dev-review-lint.py` 路线已证明：门禁只约束「写出来的反例命令」，管不到「脚本自己泄漏」）；
  本条的两个泄漏点在运行时，必须改脚本本体才消失。门禁若后加，属 B205 的 CX-SAFE 面，不在本条。

## Consequences

- **收益**：两处「绕过 `ProcessManager` 的进程组逃逸面」被补上；验收脚本在早退/中断/正常结束三条路径上
  都会回收后台 server，不再有「每跑必漏 1 个常驻 server」的稳定泄漏。
- **实测收据（本机，2026-11-16，只读，仅打印）**：非作业控制调用下子 shell 与调用者同进程组 ——

  ```text
  $ bash -c 'bash -c "echo inner_pid=\$\$ inner_pgid=\$(ps -o pgid= -p \$\$ | tr -d \" \") parent_pid=$BASHPID"'
  inner_pid=3110634 inner_pgid=3110633 parent_pid=3110634
  ```

  这是「否决 `kill 0`」这一决定的物理依据；同时也是「脚本不能假定自己是进程组长」的证据。
- **残留风险 1（`[UNVERIFIED]` → 已加护栏，状态仍为「残留风险」，不得写成「已消除」）**：util-linux `setsid` 在
  **自身已是进程组长**时会 fork（或可用 `-f` 强制），此时 `$!` 是 `setsid` 的 pid 而非 server 的 pid ⇒ 清体的
  `kill -TERM -- "-$pid"` 可能打空，降级分支也只杀到 `setsid`。脚本注释里「`$! == pid == pgid`」的等式在
  **带作业控制**的调用下不一定成立。**处置（Lead 裁决 2026-11-16）**：已派 `exec-b208` 加护栏——server 启动后用
  `ps -o pgid=` 复核实际进程组，与预期 `$!` 不符则 WARN 并记录**真实 pgid**。护栏把「静默打空」变成「可见的不一致」，
  但**没有**改变 `setsid` 的 fork 语义本身；真正的消除要等「server 自己回写真实 pid / owner 提供精确 pgid 查询」。
  机器影子：`rg -n 'ps -o pgid=' tests/acceptance/l2at-tui-models*.sh`（落地后应命中；缺失即护栏未落地，**不视为风险已消**）。
- **残留风险 2**：`kill -TERM -- "-$pid"` 存在 pgid 复用窗口（server 已退出后 pgid 可能被复用给无关进程）。
  与 B205 在 `already_exited` 分支的 `killpg` 属同一类风险；B206 的 start-time 栅栏（C5）是正解，
  但脚本层拿不到 spawn 时记录的 start-time ⇒ 脚本侧只能接受该窗口（明确登记，不假装已解决）。
- **可观测性**：本条没有新增对外接口/网络 IO/后台任务，两处改动都是既有失败路径的回收顺序；
  若后续接入 `docs/observability.yaml` 的四维归属，应把「整组回收失败」作为 warn 事件（`code="B208-group-reap"`）
  登记，而不是静默 `let _ =`——当前的 `let _ = unsafe { libc::killpg(...) }` 是 B206 C2 精神下的已知妥协，
  在 B206 落地时应改为「回收失败即 `Err`/结构化告警」。
- **未覆盖**：`updater.rs` 之外是否还有裸 `Command::new` 拉起长寿命进程，本条未做全仓清扫（结论是「当前无第三处」，
  收据：`rg -n 'Command::new' crates/pony-agent-core/src/agent | grep -v 'process.rs\|updater.rs'` 只看得到
  短命探测类用法，由 review 兜）。B205 提案的 `impl Drop for ProcessManager` 与 pipe-missing 分支仍为
  待办，未因本条推进。
