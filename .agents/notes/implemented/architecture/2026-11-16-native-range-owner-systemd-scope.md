# Agent Note: 原生归属 owner（Linux user-systemd scope / cgroup）取代 pgid 作为受管范围真源

Status: implemented

工单：B207（缺陷 P1；进程组作为 container 的结构性上限）
前序记录：`.agents/notes/proposed/architecture/2026-11-16-process-group-reap-gap-remediation.md`（B205）；
`.agents/notes/implemented/bug-fix/2026-11-16-terminate-range-quiescence-contract.md`（B206，C1–C5 的语义，本条只换 owner）
契约真源：`.dev-team/contract-range-quiescence.md` §8.1（v1.3 冻结；本 ADR 只决定「谁承担判据原语」）
落点：`crates/pony-agent-core/src/agent/process_range.rs`（新模块）+ `process.rs` 最小接线

## Problem

本仓长期用 PGID 作为「受管范围」的真源：spawn 时 `command.process_group(0)`，终止时 `killpg`，判据用
`kill(-pgid, 0)` 与 `/proc` 的 `pgrp` 扫描（B009/B205/B206）。这条路有两个**结构性**上限——B206 的契约能把它们
表达清楚，但无法消除：

1. **逃逸**：成员资格只对「还留在该进程组里的人」成立。子进程一旦 `setsid()` 自建会话、或 double-fork + reparent
   之后改换进程组，它就不再出现在判据视野里。范围探测返回「空」，而工作、端口、文件仍被占用——这正是 DSH 在
   `.agents/notes/archived/architecture/2026-08-28-subprocess-native-containment.md:10` 记下的形态：
   *"A child can call `setsid`, reparent, or outlive its direct parent and leave those ranges, so terminating the
   apparent tree can return while work, ports, or files remain active."* 本仓 `sandbox.rs` 也自述 `setsid` 与双 fork
   逃逸是已知局限（best-effort containment），属同族。
2. **身份可复用**：PGID 是「内核临时分配的一个整数」。组长退出且组空后同一 pgid 可被复用；B206 的 C5 只能用
   `starttime` 栅栏把误杀窗口压到微秒级，不能消灭。cgroup 的成员资格是**持久且可独立查询**的（unit 名由我们分配，
   不依赖整数复用）。

DSH 已把这条走通：`packages/subprocess/subprocess-local/src/managed-owner.ts:7-21` 抽出 `BoundProcessOwner`
（`signal` / `waitForExit` / `inspectTaskCount?` / `terminateForHostExit` / `cleanup?`），Linux 实现走 systemd user scope，
活跃性判据是 `systemctl --user show <unit> --property=LoadState --property=ActiveState --property=TasksCurrent`
（`linux-scope.ts:444-493`，`TasksCurrent == 0` 即范围内无任务），终止走 `systemctl --user kill --kill-who=all`。
也就是说 DSH 的「范围」不是整数，而是**由 OS 归属的命名对象**。

**本机约束（已核实）**：`systemctl --user show-environment` 退出码 `1` 且报
`Failed to connect to bus: No medium found` ⇒ 本开发机**没有可用的 user manager**（非 systemd user session），
故 live cgroup 路径在本机无法实测，只能验证降级路径与 fake 腿。

## Decision

新增 `process_range` 模块，把「谁拥有范围」从整数 pgid 提升为可替换 owner，并用一个三态判据统一 B206 语义：

1. **`RangeOwner` 四项正交能力**（`process_range.rs:75-84`）：`signal(sig)` / `wait_quiescent(deadline)` /
   `inspect_task_count() -> Option<u32>` / `cleanup()`。四件事拆开的理由：B206 的 LADDER 需要「先投递、再判定」，
   而不同 owner 的投递与判定原语完全不同（pgid：`killpg` + `/proc` 扫描；cgroup：`systemctl kill` + `systemctl show`）。
   相对 DSH `BoundProcessOwner` 五方法，本仓**不单列** `terminate_for_host_exit`：宿主退出场景由
   `impl Drop for Inner` 经唯一终止路径承担（B206 Decision 9），不需要 owner 再提供一个同步强杀入口。
2. **`RangeQuiescence` 三态**（`:59-66`）：`Empty{source}`（source 复用 B206 的 `probe=` 口径）/
   `NotEmpty{members, source}`（cgroup 侧拿不到 pid 时 `members` 为空列表，非空结论由成员数证明）/
   `Unavailable{source}`——**不可证明 ≠ 空**，必须由调用方转 `Err`，不得谎报静止。
3. **`PidRangeOwner`**：**零复制**复用 B206 原语（`kill(-pgid,0)` → ESRCH、`/proc` 非僵尸成员扫描、`killpg`），
   因此默认路径的行为与 B206 逐字一致（`inspect_task_count` 是 `/proc` 口径，见 Consequences 风险 6 的边界说明）。
4. **`SystemdScopeOwner` 四个动作**（argv 逐字、不经 shell）：
   - 建范围：`systemd-run --user --scope --quiet --collect --expand-environment=no -- <argv…>`
     （`--expand-environment=no` 防 systemd 二次展开 `$`/引号；`--collect` 保证 unit 退出后自动回收）；
   - 判活跃：`systemctl --user show <unit> -p LoadState,ActiveState,TasksCurrent`
     （`not-found`+`inactive` ⇒ 范围已消失；`TasksCurrent==0` ⇒ 空；字段缺失/未知状态 ⇒ `Unavailable`，**不猜**）；
   - 发信号：`systemctl --user kill --kill-who=all --signal=<SIG> <unit>`（按 cgroup 整组，不按 argv 模式猜 pid）；
   - 清理：`systemctl --user stop <unit>`。
5. **unit 名解析**：spawn 后从 `/proc/<pid>/cgroup` 取「**最末一个以 `.scope` 结尾的路径段**」并记入
   `ManagedProcess::scope_unit`；解析失败 ⇒ 杀子进程 + spawn 返回 `Err`（fail-loud，绝不静默错管）。
6. **探测与降级**：`OnceLock` **每进程只探测一次**（`detect_systemd_scope_support_with` 支持注入 runner 的 seam）：
   `systemd-run --version` 可用 + `systemctl --user show-environment` 成功才算支持；不支持 ⇒ 打印**一次性**告警
   （`LAST_DEGRADE_WARNING` 保证每进程一条，`degrade_warn_count()` 供测试断言）并**整体降级为 `PidRangeOwner`**。
   降级是显式且可见的，不得静默（与 B206 契约 §6.2 同款要求）。
7. **默认 `Pid` = 零行为变更**：未 opt-in 时走 `PidRangeOwner`，B206 的全部语义与 `probe=` 证据字段不变；
   scope 只是可选的更强范围。`process.rs` 侧是**最小接线**：**B207 分项** delta `+109/−23`、**B207b 分项** ≈`+61/−2`；
   与之对照，**工作树总账**（B205 + B206 + B207 + B207b 合计）为 `+788/−52`（本机 `git diff --stat` 复核，2026-11-16）。
   分项与总账口径不同，引用时须标明是哪一种；两者都不复制任何判据（U2 仍成立）。
8. **「陷阱已消除」的机器证明**：`b207_scope_unavailable_is_fail_loud`（`process_range.rs:845`）证明
   「探测失败 ⇒ 降级 + 一次性告警 + 判据退回 pgid」这条路径不会把 `Unavailable` 当成空。

## Alternatives considered

- **扩展进程组 / 会话（SID）/ 父树 / `/proc` 扫描面**：否决。DSH 已就同一选择给过判定
  （`.agents/notes/archived/architecture/2026-08-28-subprocess-native-containment.md:73`）：
  *"a process can leave each observed relationship; broader SID signalling can also reach unrelated processes when a
  PTY helper shares a launcher session. Native OS membership is persistent and independently queryable."*
  我们能观测的每一种「关系」（pgid / sid / ppid）都是进程可以主动离开的，扩面只会增加误杀面，不会把上限变成保证。
- **`waitpid(-pgid, …)` 阻塞等组**：否决。它只能回收本进程自己的子进程；孙进程已 reparent 到 init，`waitpid` 只会
  `ECHILD`。用 `wait` 家族推导「范围是否为空」在事故真实形态（组长已退、孙进程仍活）上必然给出假成功。
- **PID namespace（`unshare --pid --fork`，ns 空即范围空）**：否决。判据最干净，但需要特权/CAP_SYS_ADMIN 或 user-ns
  叠加，桌面应用（`src-tauri`）在用户机器上无法保证；且会给沙箱面引入一层与 `sandbox.rs` 语义重叠的隔离假设，扩面大于收益。
- **直接写 cgroup v2（`cgroup.procs` / `cgroup.kill`）不经 systemd**：否决为主路径，登记为备选。需要 root 或 delegated
  cgroup 权限；「有 cgroup 可写」与「有 systemd user manager」在不同发行版/容器里并不同步。`systemd-run` 自带 unit
  命名、状态查询与 `--collect` 清理，是把这套机制包装好的最小可维护面。
- **本轮直接上 systemd scope、不等 B206**：否决排期倒置。本机无 user manager ⇒ live 路径无法验证，交付会变成
  「未验证的能力」；且 scope 引入后仍需回答「范围何时算空」——那正是 B206 的语义。先立语义（B206）再换 owner（B207），
  两者互不阻塞。事后看这条判断成立：B207 落地只替换了 `wait_quiescent` 的实现，B206 的 C1–C5 一行未改。
- **保持 PGID 只加告警，不做 owner 抽象**：否决。告警不能把「逃逸成员不可见」变成可见；而且没有 `RangeOwner` 这个缝，
  B206 的判据会硬编码在 `process.rs` 的终止路径里，B207 落地时必然重写。

## Consequences

**落地事实（2026-11-16，工作树；本仓约定「已落工作树」记 implemented）**
- 新模块 `crates/pony-agent-core/src/agent/process_range.rs`：早期 Rev **941 行**，测试与 live 验证清单补齐后为 **958 行**
  （本机复核值）。
- 能力面：`RangeOwner` 四方法 + `RangeQuiescence` 三态 + `PidRangeOwner`（零复制复用 B206 原语）+
  `SystemdScopeOwner`（四动作，argv 逐字）+ `OnceLock` 一次性探测 + 可注入 runner seam + 一次性告警降级。
- 默认关闭/opt-in：默认 `PidRangeOwner` ⇒ **零行为变更**；scope 由探测可用性决定，不可用即显式降级。
- 本机实测降级原文：`systemctl --user show-environment` → 退出码 `Some(1)`：`Failed to connect to bus: No medium found`
  ⇒ 降级 `Pid`，`degrade_warn_count() == 1`（只告警一次，不刷屏）。
- 测试：`b207_scope_unavailable_is_fail_loud` 证明「陷阱已消除」；fake 腿覆盖五条：① 探测（注入 runner）
  ② `systemctl show` 输出解析 ③ `systemd-run` argv 逐字透传 ④ 降级 ⑤ fail-loud。

**Risks（上生产前必须处理，按优先级）**
1. **live cgroup 路径零实测**：本机 `systemctl --user` 报 `Failed to connect to bus: No medium found`。fake 覆盖五条腿，
   但**上生产前必须在有 user manager 的宿主机做一次 live 验证**，清单至少含：真实 `systemd-run --scope` 建 scope、
   `systemctl kill/show/stop` 全链路、`deactivating` 滞后、`--collect` 回收时机、`/proc/<pid>/cgroup` 里 unit 名的真实形态。
   在完成该验证前，本条的能力应按 `[SMOKE DEGRADED]` 解读：**只有降级路径被证明过**。
2. **`--scope` 是否原地 `exec` 必须首要确认（live 验证清单第 1 项）**：`--scope` 下 `systemd-run` 应**原地 `exec`**，
   使 `child.id()` 就是目标进程 pid。若某 systemd 版本 **fork** 而非 exec，`child.id()` 会是 `systemd-run` 的 pid ⇒
   FENCE 的 start-time 会读到**错的进程** ⇒ 判 `Stale` ⇒ **零信号**（对已存在的范围从不下手）。
   **症状（一眼定位）**：spawn 成功但终止路径从不发信号、日志出现 `code=B206-fence-stale` 且
   `observed_start != recorded_start`；`/proc/<child_pid>/comm` 是 `systemd-run` 而不是目标程序。
3. **unit 名解析依赖 cgroup v2 形态**：现按「`/proc/<pid>/cgroup` 最末一个 `.scope` 段」解析；**hybrid cgroup**
   或嵌套 `app.slice` 形态未验证。解析失败 ⇒ 杀子进程 + spawn 返回 `Err`（fail-loud，不会静默错管），
   但「fail-loud 到不可用」在 hybrid 宿主上表现为「开了 scope 就起不了进程」，属可用性风险。
4. **B207c 候选（本轮不做，登记后续）**：改用 `--unit=<minted>`（unit 名 spawn 前即确定）可消除第 3 条的解析依赖，
   并顺带去掉两处 item 级 `allow(dead_code)`（`process_range.rs:174`、`:357`）；代价是**改冻结的 argv 形态**
   且需 live 验证 ⇒ 须在有 user manager 的宿主机就绪后单独一轮做。
5. **探测只看控制面可用性，不看策略层拒绝**：`systemctl --user show-environment` 的退出码不覆盖
   `Delegate=` / `TasksMax=` / `MemoryMax=` 这类**策略层**拒绝。当前由「spawn 失败 ⇒ `Err`」兜住
   （不会静默错管，但用户看到的会是 spawn 失败而不是「scope 被策略拒绝」这一更精确的原因）。
6. **`PidRangeOwner::inspect_task_count` 是 `/proc` 口径**（组内非僵尸成员数），**不等价**于 cgroup 的 `TasksCurrent`
   （cgroup 还包含已 `setsid` 逃逸但仍在 cgroup 内的成员）。该值**仅供非 Pid owner 的诊断对照**，不得用于在两种 owner 之间
   做「谁更空」的比较，也不得作为契约判据的替代。

**其它后果与保留风险**
- **依赖宿主环境能力**：systemd user manager 在容器/精简发行版/WSL 里常缺席。降级路径是一等公民（本轮本机就跑在降级路径上），
  否则一旦 live 走不通，整个终止链会 fail closed 到不可用。
- **`activating/deactivating` 窗口**：判据按 DSH `linux-scope.ts:444-493` 的分类处理
  `active|activating|reloading|deactivating`；未知状态一律抛 `Unavailable` 而不是猜（否则会在 `deactivating` 窗口谎报静止）。
- **argv 透传与既有 `sh -lc` 包裹**：现有 `tools/run.rs` 用 `sh -lc "cd … && …"` 包裹命令；scope 化后 `sh -lc`
  仍由我们自己的 argv 传递，scope 只包外层，`--expand-environment=no` 防止第二层引号/变量展开（有含引号命令的回归用例）。
- **两套 owner 的判据漂移**：`PidRangeOwner` 零复制复用 B206 原语，`SystemdScopeOwner` 复用同一个三态状态机；
  要求「到死线仍活 ⇒ `Err`」的状态机只有一处，差异只在「如何观测」。
- **语义不变已验证**：B206 的 C1–C5 一行未改；B207 落地只换 `wait_quiescent` 的实现（这也是 Decision 中「先 B206 后 B207」
  排期的回报）。
