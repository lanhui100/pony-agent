# B206 契约：终止完成条件 = 「范围已证明为空」（range quiescence）

| 项 | 值 |
|---|---|
| 工单 | B206（缺陷，P0；B205 遗留契约漏洞） |
| 冻结方 | Lead（六条口径 C1–C6 已冻结，**本文不得改口径**） |
| 编写方 | Protocol Agent（L2-P，唯一契约编写方；不写业务实现） |
| 文档状态 | `frozen`（口径层）；实现层 **未落地**（`process.rs` 仍是 B205 状态：`terminate` 发完信号即 `return Ok(())`） |
| 关联 | B205（`already_exited` 补 killpg，已落工作树）／B207（原生归属 owner）／B208（遗留泄漏点清扫） |
| 承载代码 | `crates/pony-agent-core/src/agent/process.rs`（`kill` `:413`、`kill_if_exists` `:419`、`kill_inner` `:430`、`cancel` `:486`、`shutdown` `:558`、`terminate` `:1033`、`lookup` `:1121`） |
| 参照实现（只读外部真源） | DSH `/home/dm/deepseek-harness`：`packages/subprocess/subprocess-local/src/spawn.ts:330-370`（terminate 与 waitForExit 分离）、`managed-owner.ts:7-21`（`BoundProcessOwner` 五方法）、`linux-scope.ts:444-493`（`systemctl show` 判范围活跃）、`scripts/run-gates.ts:1350-1364`（`gate tree not quiescent after 8s` fail loud）、`process-inspector.ts:9,403`（`ProcessIdentity{pid,started}` + `alive()` 比对 start 身份） |

> **行号锚定声明**：B205 改动已落工作树（`kill_inner` 内新增 19 行），`process_group(0)` 的行号由 B205 ADR 里的
> `:689` 漂移到 **`:709`**。本文所有引用以「符号名 + 本机复核行号」双写；后继若再漂移，**以符号名为准**，
> 行号只作定位辅助。
> **v1.3 附加提醒**：B206 实现（`process.rs`，+713 行）在本文修订期间仍在演进，实现侧行号已多次漂移
> （如 `libc::killpg` 派发点 `:339` → `:348`）。凡本文标注「v1.3 复核」的行号均为**单次观测值**，
> 机器影子（§9）一律以符号/计数为判据，不以行号为判据。

---

## 0. 术语与判据原语（后续每一条都只引用这些原语，不再重新解释）

**受管范围（managed range）**：由 `ProcessManager` spawn 时 `command.process_group(0)`（`process.rs:709`）建立的
POSIX 进程组；因此**恒有 `pgid == 受管直接子进程 pid`**（`process.rs:709` 保证）。受管范围 = 「该 pgid 下的全部成员」，
包含组长已退出后被 init 收养的孙进程。Windows 侧对应物 = Job Object（C6）。

| 原语 | 定义（可机器判定） | 承载 |
|---|---|---|
| **PROBE-A** `probe_kill0(pgid)` | 调用 `kill(-pgid, 0)`。返回 `-1` 且 `errno == ESRCH(3)` ⇒ **范围为空**（证明成立）。返回 `0` ⇒ 范围非空。其余 errno（`EPERM=1`、`EACCES=13`…）⇒ **不可证明**（≠ 空，见 §6-D2） | `libc::kill`，参照 B205 测试 `b205_group_probe_evidence`（`process_group_reap_test.rs:287`） |
| **PROBE-B** `scan_proc(pgid)` | 遍历 `/proc/[0-9]*/stat`，以最后一个 `)` 为界取后续字段；判 **字段 5（pgrp）== pgid 且 字段 3（state）!= 'Z'** 的 pid 列表。列表为空 ⇒ **范围为空**（证明成立）。`/proc` 不可读 ⇒ **不可证明** | 只读文件系统，参照 `b205_live_group_members`（`process_group_reap_test.rs:255`） |
| **FENCE** `identity_ok(pid, recorded_start)` | 读 `/proc/<pid>/stat` **字段 22**（`starttime`，boot 后 jiffies）与 spawn 时记录的 `recorded_start` 比较：相等 ⇒ 同一进程；不等或读不到（ENOENT）⇒ 已非同一进程/组 | `/proc/<pid>/stat`，参照 DSH `process-inspector.ts:9,403`（`started` 相同才 `alive`） |
| **LADDER** | `SIGTERM` → 等 `term_grace`（默认 **250ms**）→ 未空则 `SIGKILL` → 等到总死线 `quiesce_deadline`（默认 **5s**，自**首次信号**起算）→ 仍未空则 `Err` | `process.rs` 新增常量（建议 `TERM_GRACE` / `QUIESCE_DEADLINE` / `QUIESCE_POLL_INTERVAL=20ms`） |

两条空证明判据是 **OR** 关系（任一成立即「已证明为空」）；两者都不可用 ⇒ **「不可证明」**，
按 §6-D2 必须走 `Err`，**不得**当成空。僵尸（`state == 'Z'`）**不计入存活成员**（已无 CPU/内存占用，
只等收尸；计入会造成「杀干净却判失败」的假红相——B205 测试已确立同一口径）。

---

## 1. 六条契约（正式表述，可机器判定）

标记约定：**MUST** = 机器可判定的强制条款；**MUST NOT** = 机器可判定的禁止条款。每条给出「判据」。

### C1 —— 范围为空才算完成

**正式表述（冻结口径，逐字落地）**：`terminate` / `kill` / `cancel` / `shutdown` 返回成功
**当且仅当**该受管进程范围**已证明为空**。

**判据（MUST，判定顺序即证据顺序）**：
1. 先过 FENCE（C5）：栅栏通过才拥有「pgid 探针的权威性」。
2. 然后 **PROBE-A 或 PROBE-B 至少一条返回「空」**。
3. 只有 2 成立，才允许返回 `Ok`／`Ok(())`／把该条目计入成功计数。

**MUST NOT**：以「信号已发送」「信号发送成功」「`child.kill()` 返回 Ok」「`try_wait()` 返回 `Some`（那只证明
直接子进程退了，不证明组空）」作为完成依据。以上任何一条单独出现都不构成 §1-C1 的成功。

**`shutdown` 的落地形态 = S-3（加法式 fail-loud）**（Lead 裁决 2026-11-16；**唯一合规解**）

现状 `pub fn shutdown(&self, session_id: &str) -> usize`（`process.rs:558`）**不返回 `Result`**，因此「返回 `Ok`」
无法逐字对应。下表每行都是 MUST：

| 项 | 规定 |
|---|---|
| 新增 API（加法） | `pub fn shutdown_checked(&self, session_id: &str) -> Result<usize, String>`（名字可在实现时定，语义固定）：返回值只统计**已证明范围为空**的条目；存在未静默条目即 `Err`，消息含**未静默 handle 列表 + 各自 pgid + 已用信号阶梯**（§5 格式；多条以 `; ` 分隔） |
| 既有 API（签名不变） | `pub fn shutdown(&self, session_id: &str) -> usize` 内部**委托** `shutdown_checked`；`Err` 时沿用 `eprintln!`，但**必须**输出结构化前缀 `[B206][shutdown-not-quiescent] …`（替换现状 `[process] shutdown termination error for …`，不得并存两套前缀） |
| 兼容性 | 既有调用方（`acp/handlers.rs`、`agent/jobs.rs`、会话收尾路径）**零改动**；需要失败信号的新调用方走 checked 版 |
| 判据（机器可判） | ① 未静默场景下 `shutdown_checked(…).is_err()` 为真；② 同场景下 `shutdown(…)` 的返回值**不含**未静默条目（计数 == 已证明为空者数）；③ 该场景日志出现 `[B206][shutdown-not-quiescent]` 且匹配 §5.3 正则 |

**否决 S-1（把 `shutdown` 签名改为 `-> Result<usize, String>`）**：属**破坏性公共 API 变更**，按项目「契约单向门
纪律」需先做消费方普查与兼容证据，本轮不该背这个成本；且它把「拿到失败信号」与「改所有调用方」绑成一件事，
扩面大于收益。
**否决 S-2（保存名 + 未静默条目不计入 + 仅进日志）**：调用方**拿不到**失败信号，等于把「不谎报静止」降级为
「只要翻日志就知道」——这正是 B205 之后要消灭的形态（§6.1 X1）。
**禁止**：把 `shutdown` 现状（吞错 + 计数含未静默条目 + 未证明空却 `remove`）当作合格终态。

现状态对 C1 的偏差（逐条可复核，本机 2026-11-16 复核）：

1. `process.rs:568-570`（`shutdown`）把 `terminate` 的 `Err` 降级为 `eprintln!` 后**继续 remove 条目并计入
   `count`** ⇒ 既违 C1（未证明空却计成功），也违 C3（未证明空却 remove）。
2. `terminate` 自身在状态已 `Exited` 时**直接 `return Ok(())`**（`process.rs:1038-1048`，POSIX 侧**一个信号都不发**）。
3. **B205 的补刀没有覆盖 `shutdown` 路径**（本轮新发现，影响排期判断）：`shutdown` 在 `process.rs:568` 直接调
   `self.terminate(&entry)`，**绕过** `kill_inner` 的 `already_exited` 分支；而 `terminate` 的早退分支（见第 2 条）
   在 POSIX 侧不做 `killpg` ⇒「会话收尾时组长已退出、孙进程仍活」这一形态**既不补刀也不探测，直接报成功**。
   ⇒ B205 的 `killpg` 只覆盖 `kill` / `kill_if_exists` / `cancel` 三条入口；`shutdown` 仍留着与 2026-11-13
   事故完全同族的假成功面。C1/C2 的实现必须**统一到单一终止路径**（`shutdown` 也走同一条 LADDER），
   否则会出现「`kill` 严格、`shutdown` 宽松」的双口径 —— 该要求已升为独立硬约束 **C1-U**（见下）。

### C1-U —— 唯一终止路径（硬约束；executor 逐条自查用）

**硬约束（MUST，单独成条以便逐项自查）**：所有终止入口（`kill` / `kill_if_exists` / `cancel` / `kill_after` /
`shutdown` / `shutdown_checked`）**必须汇聚到同一条终止路径**；该路径是**唯一**做「范围为空证明」（FENCE + LADDER +
PROBE）的地方，其它入口不得自带发信号或自带 `Ok` 早退。

自查清单（每条为可复核的是/否）：

| # | 自查项 | 现状 |
|---|---|---|
| U1 | 五个入口的调用链最终落到同一个终止函数（不许有第二条实现） | **否**：`shutdown`（`process.rs:568`）直调 `terminate`，`kill_inner` 自己另有一套 `already_exited` 早退（`:444-481`） |
| U2 | PROBE / FENCE 只出现在该唯一路径内（别处不得复制判据） | **否**：现状两处各自 `killpg`，**两处都不探测** |
| U3 | 任何「直接 `return Ok(())`」都位于该路径内、且前置有 FENCE/PROBE 证据 | **否**：`terminate` 在 `Exited` 时 `return Ok(())`（`:1038-1048`，POSIX 侧不发信号也不探测）；`kill_inner` 在 `:480` 同样无证明 |
| U4 | `shutdown` 路径与 `kill` 路径对「组长已退 + 孙进程仍活」给出**同一种**结论（同严格度） | **否**：`kill` 至少补刀（B205），`shutdown` 连刀都不补 |

> 该条是「kill 严格 / shutdown 宽松」双口径的**根治条款**：只补 `terminate` 而不动 `kill_inner` 的早退，或只补
> `kill_inner` 而不动 `shutdown`，都会留下一条比 B205 更隐蔽的假成功面（隐蔽在于它只在会话收尾时出现）。

### C2 —— 信号升级阶梯 + 总死线 fail loud

**正式表述**：终止一律按 **LADDER**：`SIGTERM` → 等 `term_grace`（默认 250ms）→ 未空则 `SIGKILL` →
等到总死线 `quiesce_deadline`（默认 5s）→ **仍未空则返回 `Err`**（fail loud，绝不谎报静止）。

**判据（MUST）**：
- 常量默认值可判：`term_grace = 250ms`、`quiesce_deadline = 5s`、轮询间隔 `20ms`（可在 `#[cfg(test)]` 下注入更小值，
  但**默认值不得改小以换绿灯**）。
- 轮询必须是**显式条件轮询**（每轮重新 PROBE-A/PROBE-B），不得写成「`sleep(term_grace)` 后当作已静止」。
- 死线耗尽 ⇒ `Err`，且 `Err` 必须携带**已用掉的阶梯**（`ladder=[TERM]` / `ladder=[TERM,KILL]`）。
- 原则来源：DSH `scripts/run-gates.ts:1350-1364` —— `close` 只代表直接子进程 stdio 关了，必须先确认进程组与捕获的
  后代全部消失才 settle；8s 到点仍活则 `gate tree not quiescent after 8s` 打印后**按失败记录**，而不是报告静止。

**MUST NOT**：把死线耗尽降级为 `warn!` 后返回 `Ok`（见 §6-X1）。

### C3 —— 句柄不提前移除

**正式表述**：直系子进程已 `Exited` 但范围**非空**时，**保留**该条目以保留后续补刀能力；仅当范围
**证明为空**后才 `processes.remove(handle)`。

**判据（MUST）**：
- 「保留」可判定：`is_tracked(session, handle)`（`process.rs:426`）在「组长已退、组内仍有存活成员」时必须返回 `true`。
- 「移除」可判定：`remove(handle)` 的每一次调用点，前驱必须是 `C1 证明成立`（PROBE 成功返回空）或
  `FENCE 判定身份失效`（C5 语义下「我方范围已消失」）或「从未 spawn 成功」（无条目）。
- 现状态偏差：`kill_inner` 的 `already_exited` 分支在 `process.rs:473-480` **无条件 remove**，其前驱只有一次
  `killpg` 发送（`:458-471`），无任何空证明 ⇒ 违 C3。

**前置关系**：C3 是「`impl Drop` 缺口」的**前置条件**（B205 提案第 1 项）。理由：`Drop` 只能对「仍在册的条目」补刀；
条目被提前 remove 的进程组，`Drop` 永远看不见——先修 C3，`Drop` 才有作用对象。
**落点偏差（Lead 核准）**：`Drop` 实现在 `Inner`（`impl Drop for Inner`，v1.3 复核 `process.rs:1655`）而非 `ProcessManager`
（`#[derive(Clone)] pub struct ProcessManager`，v1.3 复核 `:777`）——
对 `Clone` 类型实现 `Drop` 会在每个克隆析构时收组，等于误杀其它调用方正在用的进程；`Inner` 的 `Drop` 才等价于
「最后一个 owner 消失」。详见 §7.1-R2。

### C4 —— 幂等语义不变

**正式表述**：未知 handle + `idempotent_missing = true`（`kill_if_exists`）仍返回 `Ok(())`；
`kill` 对未知 handle 仍返回错误。

**判据（MUST，逐字保留现状行为）**：
- `kill_if_exists(s, "未知")` ⇒ `Ok(())`；既有单测名 `test_kill_if_exists_idempotent_on_missing_handle`
  （`process.rs:1186`）必须继续绿。
- `kill(s, "未知")` ⇒ `Err`，消息含 `unknown process handle`（`process.rs:1129`）。
- 跨 session 句柄 ⇒ `Err`，**不因幂等放宽**（`process.rs:1126-1131`：`cross-session handle use is rejected`）。
- 幂等分支**不得**为满足 C1 而引入探测：无条目 ⇒ 无范围 ⇒ 不探测、不加延迟。

### C5 —— start-time 身份栅栏（防 PID 复用误杀）

**正式表述**：在**发出任何终止信号之前**、以及**做任何范围探测之前**，读取受管子进程的 start-time
（`/proc/<pid>/stat` 字段 22）；若与 spawn 时记录的值不一致，判定「已不是同一个进程/组」⇒
**不发信号**，并**视为范围已消失**。

**判据（MUST）**：
- spawn 时**必须**记录 `start_time`（新字段，建议 `start_time_jiffies: u64`），spawn 后立刻读；读不到 ⇒ `start`
  返回 `Err`（fail closed，不允许「记不住就跳过栅栏」）。
- 每次 `terminate`/`kill_inner` 的**第一动作**是 FENCE，早于 LADDER 的第一发信号、早于 PROBE。
- **FENCE 三态（Lead 核准 executor 实现）**——只有两种来源算 `Stale`：
  (i) `/proc/<pid>/stat` **可读**且 `observed_start != recorded_start`；
  (ii) `/proc/<pid>/stat` **不存在（ENOENT）**且组长**未被**本管理器 `wait()` 收尸（安全侧）。
  第三种不算 mismatch：`/proc` 不存在**且**组长**已被** `wait()` 收尸 ⇒ **栅栏通过**（`Ok{reaped:true}`），
  继续走 LADDER。**理由**：`wait()` 成功即内核已收尸，而同一 pid 在 `wait()` 之前不可被复用
  ⇒ 身份由内核 wait 语义直接证明；若把这种情形也判 `Stale`，「组长 exit 0 且已被收尸 + 孙进程仍在组内」
  这一**事故本形态**将永远拿不到收组权（冻结红相永远红）= 回退 B205 全部价值。
- `Stale` 的返回：`Ok(())` + 结构化告警 `code="B206-fence-stale"`（含 `pgid`、`recorded_start`、`observed_start`
  或 `ENOENT`）；`reaped` 通过时记 `code="B206-fence-reaped"`（debug 级，非 mismatch）。
  **证据链必须分开记**：`fence=stale` 不等于 `probe=empty`，**`fence=reaped` 亦不等于 `probe=empty`**
  （`reaped` 只进 debug 日志，`Err` 模板仍打印 `fence=ok`，因为 §5.3 正则要求 `fence=ok` 紧跟 ` — `）。
  日志里不得把任何一种栅栏结论写成「范围已空」。
- FENCE 不匹配时**允许** remove 条目（C3 的第二个合法前驱），因为「我方范围的消失」已由身份判据证明。
- 参照：DSH `process-inspector.ts:9`（`ProcessIdentity{pid, started}`）与 `:403`
  （`alive(identity)` = `row.started === identity.started && !quiescent(row.state)`）——**身份相同**才谈存活。

**残余风险（须知，不因此放宽）**：pgid 复用与 pid 复用不同——只要组内还有成员，内核不会把该 pgid 分配给别的组；
但**组长退出且组已空**后，pid/pgid 均可被复用。FENCE 覆盖的正是这个窗口。

### C6 —— Windows 语义不变

**正式表述**：Windows 侧维持现状：Job Object 的 `KILL_ON_JOB_CLOSE` + 显式 `TerminateJobObject(1)` 即视为
「范围为空」的等价判据，本轮不改。

**判据（MUST NOT）**：
- POSIX 的 PROBE-A/PROBE-B/FENCE **不得**进入 `#[cfg(windows)]` 路径。
- Windows 成功判据 = `job.terminate(1)` 返回 `Ok`（`windows_job.rs:116-131`）或 job 已关闭（`close()` 幂等）。
- 事实校准（与 Lead brief 的差异，见 §8）：当前代码**没有**「active-process 零判定」这类查询
  （`rg -n 'ActiveProcess' crates/` 无命中），Windows 侧判据就是 `TerminateJobObject` 成功 + kill-on-close 兜底。
  本契约按**代码现状**表述 C6，不改其语义，也不新增 ActiveProcesses 查询。

---

## 2. 判定表（逐行给判据；本表是 test-agent 冻结用例与 review 的唯一对照表）

| # | 场景 | 前置判据（可机器判定） | 返回 | 条目处置 | 必须留存的证据 |
|---|---|---|---|---|---|
| T1 | 未知 handle + `idempotent_missing=true` | `lookup` ⇒ 消息含 `unknown process handle` | `Ok(())` | 无条目，无操作 | 无（**禁止**为此探测，C4） |
| T2 | 未知 handle + `kill` | 同上，且 `idempotent_missing=false` | `Err`（含 handle 与 session） | 无 | Err 文本 |
| T3 | 跨 session 句柄（两种入口） | `lookup` ⇒ `cross-session handle use is rejected` | `Err` | 保留 | Err 文本 |
| T4a | FENCE 通过（start-time 相等） | `/proc` 可读 且 `observed_start == recorded_start` | 继续 LADDER（非终态） | 不动 | 无额外日志 |
| T4b | FENCE `Stale`（身份变化） | `/proc` 可读 且 `observed_start != recorded_start` | `Ok(())`（范围已在身份语义下消失） | **允许** remove | `code=B206-fence-stale`，含 `pgid`/`recorded_start`/`observed_start`。**零信号**（断言「信号发送计数 == 0」） |
| T4c | FENCE 通过（组长已被收尸） | `/proc/<pid>/stat` **不存在** 且组长**已被**本管理器 `wait()` 收尸 | 继续 LADDER（`Ok{reaped:true}`，**非** mismatch） | 不动 | `code=B206-fence-reaped`（debug）；**零信号**的约束**不适用**（之后必须照 LADDER 发信号） |
| T4d | FENCE `Stale`（安全侧） | `/proc/<pid>/stat` **不存在** 且组长**未被**我方收尸 | `Ok(())` | **允许** remove | `code=B206-fence-stale`（含 `ENOENT`）。**零信号** |

> **T4 族（T4a–T4d）是「三态四分支」**：`Ok{reaped:false}` / `Ok{reaped:true}` / `Stale`，
> 其中 `Ok{reaped:true}` **不是** mismatch。实现对应 `process.rs` 的 `fence_verdict`（三态枚举）：
> `Some(observed)==recorded` ⇒ `Ok{reaped:false}`；`Some(observed)!=recorded` ⇒ `Stale`；
> `None && leader_reaped` ⇒ `Ok{reaped:true}`；`None && !leader_reaped` ⇒ `Stale`。
> **残余风险的闭环论证（必须保留）**：收尸后 leader pid 若被复用成新组长，`killpg` 会误伤；但
> (a) 只要旧组仍有成员，内核不会把该 pgid 分配给别的组；(b) 一旦真复用，读到的 start-time 必不同
> ⇒ 命中 T4b `Stale`（零信号）。⇒ **不是赌运气，是可验证的逻辑闭环**。
| T5 | 范围已空（判据 A） | `probe_kill0(pgid)` ⇒ `-1` / `errno=ESRCH` | `Ok(())` | remove | `probe=kill0-esrch` |
| T6 | 范围已空（判据 B） | `scan_proc(pgid)` ⇒ 空列表（或仅僵尸） | `Ok(())` | remove | `probe=proc-empty`（附非僵尸计数 0） |
| T7 | 范围非空 → TERM 后在 `term_grace` 内变空 | FENCE ok；`SIGTERM` 后轮询 ≤250ms 命中 T5/T6 | `Ok(())` | remove | `ladder=[TERM]`、`elapsed_ms<=250` |
| T8 | TERM 无效果 → KILL 后在总死线内变空 | FENCE ok；`term_grace` 到期仍非空 ⇒ `SIGKILL`；`elapsed_ms <= 5000` 命中 T5/T6 | `Ok(())` | remove | `ladder=[TERM,KILL]`、`elapsed_ms<=5000` |
| T9 | **总死线耗尽仍非空** | FENCE ok；`elapsed_ms >= 5000` 且 PROBE 仍非空 | **`Err`**（§5 格式） | **保留**（C3） | `ladder=[TERM,KILL]`、`members=[…]`、`probe=kill0-alive|proc-members` |
| T10 | 探针不可用（`/proc` 不可读 + `kill(0)` 返回 `EPERM/EACCES`） | 两条判据都拿不到「空」结论 | **`Err`**（不可证明 ≠ 空） | 保留 | `probe=unavailable`、`errno` |
| T11 | 非 Linux unix（无 `/proc`，如 macOS/BSD） | `/proc` 不存在 ⇒ 只有 PROBE-A 可用；A 给 `ESRCH` ⇒ `Ok`；A 给 `0` 且阶梯耗尽 ⇒ `Err` | 同 T5/T9 | 同 T5/T9 | 一次性告警 `code=B206-probe-degraded`（**每次进程一次**，不刷屏）+ 判据字段 `probe=kill0-only` |
| T12 | Windows | C6 判据（`TerminateJobObject(1)` 成功 / job 已 close） | `Ok(())` | 现状（Job 收口） | Job 相关既有日志 |
| T13 | `shutdown` / `shutdown_checked` 聚合（S-3） | 逐条走 T1–T12；未静默条目按 T9/T10/T11 判定 | `shutdown_checked`：全空 ⇒ `Ok(n)`，任一未静默 ⇒ `Err`（含 handle 列表）；`shutdown`：返回 `n` = **只含已证明为空者**，未静默条目走 `[B206][shutdown-not-quiescent]` | 只 remove 已证明为空 / T4b/T4d 的条目；未静默条目**保留** | 未静默条目逐条 §5 格式；`shutdown` 路径另需 `[B206][shutdown-not-quiescent]` 前缀日志 |
| T14 | **唯一终止路径自查**（C1-U） | `kill` / `kill_if_exists` / `cancel` / `kill_after` / `shutdown(_checked)` 的调用链最终都落到**同一个函数**，且 PROBE/FENCE 只在该函数内出现 | 不适用（结构判据） | 不适用 | 见 M7 的 `rg` 收据：探针符号在终止路径中**只出现一处** |

**判定表的可用性约束**：任一行**不允许**出现「`Ok` + `probe` 缺失」。机器判定方式：
每条返回前必须落一个 `probe=` 字段（T1/T2/T3/T14 除外，它们不探测）。

---

## 3. 可回放样例（≥4 组；**逐条本机回放过**，收据见各组 `实测` 行）

> 纪律：以下片段全部**只读**或**只发信号 0**，不创建后台进程、不 `kill`/`pkill` 任何进程、不写仓库。
> 「pending」组标注原因，不得当成已回放。

### S1 合规路径 → `Ok`（判据 A：ESRCH 即证明为空）

```sh
probe_kill0() { python3 - "$1" <<'PY'
import errno, os, sys
pgid = int(sys.argv[1])
try:
    os.kill(-pgid, 0); print("rc=0  errno=0      (NON-EMPTY)")
except ProcessLookupError as e:
    print("rc=-1 errno=%d %-7s (EMPTY -> C1 criterion A satisfied)" % (e.errno, errno.errorcode[e.errno]))
except PermissionError as e:
    print("rc=-1 errno=%d %-7s (non-empty but foreign -> UNPROVABLE)" % (e.errno, errno.errorcode[e.errno]))
PY
}
probe_kill0 987654      # 987654 = 本机无任何进程使用的 pgid
```

- 期望：`rc=-1 errno=3 ESRCH (EMPTY …)`；命令自身退出码 `0`。
- 期望判定：命中 C1 判据 A ⇒ `terminate` 必须返回 `Ok`。
- **实测（2026-11-16，本机）**：`rc=-1 errno=3 ESRCH   (group EMPTY -> C1 criterion A satisfied)`，`members=[]`。

### S2 范围未空 → `Err`（活体样本：用当前 shell 自己的进程组，只发信号 0）

```sh
MYPG=$(ps -o pgid= -p $$ | tr -d ' ')
probe_kill0 "$MYPG"
```

- 期望：`rc=0 errno=0 (NON-EMPTY)` ⇒ 命中判定表 T7/T8/T9 的**起始态**：此刻必须继续阶梯；阶梯耗尽仍非空即 T9 `Err`。
- 反空跑意义：证明「非空探测」在真实机器上真的能表达非空（否则 S1 的「空」可能是探针恒空造成的假绿）。
- **实测（2026-11-16，本机）**：`self_pid=3097404 self_pgid=3097404` → `rc=0  errno=0      (group NON-EMPTY …)`，`member_count=1`。

### S3 合规路径 → `Ok`（判据 B：/proc 扫描 + 僵尸剔除）

```sh
range_members() {   # 打印 pgrp==$1 且 state!='Z' 的 pid（升序自然序）
awk -v p="$1" '{ i=index($0,")"); s=substr($0,i+1); split(s,f," ")
  if (f[3]+0==p+0 && f[1]!="Z") print FILENAME }' /proc/[0-9]*/stat 2>/dev/null \
  | sed 's|/proc/||; s|/stat||'
}
range_members 987654            # 期望空输出
awk '{ i=index($0,")"); s=substr($0,i+1); split(s,f," ")
       if (f[1]=="Z") print "zombie pid=" FILENAME " pgrp=" f[3] }' /proc/[0-9]*/stat 2>/dev/null | head -3
```

- 期望：第一条**无输出**（⇒ C1 判据 B 成立）；第二条打印若干僵尸样本，证明「僵尸存在也不影响空的结论」。
- **实测（2026-11-16，本机）**：`range_members 987654` → 空输出；僵尸样本
  `zombie pid=/proc/1245773/stat pgrp=1245773` 等 5 条；全机非僵尸进程 645（扫描器确实在工作，不是恒空）。

### S4 身份栅栏不匹配 → **不发信号**、视为范围消失（→ `Ok`）

```sh
starttime() { awk '{ i=index($0,")"); s=substr($0,i+1); split(s,f," "); print f[20] }' "/proc/$1/stat" 2>/dev/null; }
RECORDED=$(starttime 1)      # 伪造「spawn 时记录的 start-time」
OBSERVED=$(starttime $$)     # 目标进程当前的 start-time
[ "$RECORDED" = "$OBSERVED" ] && echo "FENCE=ok  -> 允许发信号" || echo "FENCE=stale -> 不发信号，视为范围已消失"
```

- 期望：`FENCE=stale -> 不发信号，视为范围已消失`；判定表 T4 ⇒ `Ok(())` + `code=B206-fence-stale`。
- **实测（2026-11-16，本机）**：`starttime(self=3097404)=136430118 starttime(pid1)=20` ⇒ 不相等 ⇒ `FENCE=stale`。
- 说明：字段 22 = `stat` 中 `)` 之后第 20 个字段（因 `)` 之后从整体字段 3 起算，`22-2=20`）。

### S5 幂等未知 handle → `Ok`（API 级；`[PENDING: 待 test-agent 冻结用例后独占回放]`）

```sh
# 落地后回放（现状已有同名单测，仍可绿）：
cargo test --manifest-path src-tauri/Cargo.toml --lib \
  --target-dir target-test test_kill_if_exists_idempotent_on_missing_handle -- --exact
# 期望 exit 0；断言 manager.kill_if_exists("session-a","proc-nonexistent").is_ok()（process.rs:1191）
# 反向：kill(同参数) 必须 Err（消息含 "unknown process handle"）
```

- 静态判据收据（本轮可给、已给）：`rg -n 'unknown process handle' crates/pony-agent-core/src/agent/process.rs`
  ⇒ `:1129`（唯一产生点）、`:434`（幂等分支判定）、`:418`（文档注释）。
- **未回放原因（如实登记、Lead 已接受）**：本机当前有 2 个 `cargo` 进程在跑（其它 teammate 的构建），
  `cargo test` 会抢 workspace 锁并拖慢全队；本轮的 ≥4 组回放已由 S1–S4 满足，S5 由 test-agent 冻结用例后
  在独占槽位（`--target-dir target-test-exact-*`）回放。**不为凑样例数去跑会拖慢全队的构建**。

### S6 反向样例：B205 现状态在 T9 上是**假绿**（回归护栏的动机）

```sh
# 现状（B205 落工作树后）语义等价伪代码，仅说明判据缺口，不执行：
#   already_exited => killpg(pgid, SIGKILL); remove(handle); return Ok(())
# 即使 SIGKILL 因 D 状态（uninterruptible I/O）永远无法收尾，也返回 Ok。
rg -n 'Idempotent cleanup|return Ok\(\(\)\);' crates/pony-agent-core/src/agent/process.rs | head
```

- 期望（B206 落地后）：`already_exited` 分支必须先 PROBE 再 `Ok`；红灯用例必须能在「SIGKILL 无效」的替身进程上
  真实失败（参照 DSH `run-gates.ts` 的 8s fail loud 场景：`uninterruptible I/O that even SIGKILL cannot cut`）。
- 该组**不发信号、不建进程**，是审查用的定位样例（`rg` 收据可即时回放）。

---

## 4. 与 B205 的关系：C1 是**加强**，不是替代

B205 已落工作树的成果（`kill_inner` 的 `already_exited` 分支补 `killpg`，`process.rs:443-479`）解决的是
**「补一次刀」**：组长自行 `exit 0` 后也派发信号，使孙进程不再必然逃逸。
C1 解决的是**「刀有没有真砍到」**：信号发出 ≠ 范围为空。

| 维度 | B205（已落工作树） | B206 / C1（本文契约） |
|---|---|---|
| 触发面 | 「组长已退出」早退分支 | **全部**终结入口（`kill`/`kill_if_exists`/`cancel`/`shutdown`→`terminate`） |
| 完成判据 | 信号已发送（`killpg` 调用未报错 / 忽略返回值） | **范围已证明为空**（PROBE-A/B） |
| 失败表达 | 无：`killpg` 返回值被 `let _ =` 丢弃（`:468-471`），失败不可见 | `Err`（fail loud，§5 格式） |
| 句柄 | 早退分支无条件 `remove`（`:473-480`） | **范围证明为空才 remove**（C3），否则保留以支持补刀 |
| PID 复用 | 以「组内仍有成员则 pgid 不被复用」论证安全（`:454-458` 注释） | FENCE 用 start-time 身份直接判定（C5），不再依赖该论证 |
| 关系 | 机制层（多砍一次） | 语义层（砍到才算完）——**C1 要求 B205 的机制保留**，只是其成功不再由「调用过」判定 |

**必须保留的 B205 成果**：`already_exited` 分支的 `killpg`（Windows 侧 `job.terminate(1)`）不得因本契约回退；
C1 只在其**之后**追加「证明」与「条件式 remove」。**若 executor 以「改为走 `terminate`」的方式实现 C1 而删掉该分支，
必须证明 `terminate` 对「组长已退出」的条目能取到 `pgid`（`child.id()` 在 `try_wait` 已回收后仍可用）**——
否则等于回退 B205。

---

## 5. 失败语义：`Err` 消息格式（可被测试用正则断言）

### 5.1 结构要求（MUST）

单行、稳定前缀 `B206:`，其后全为 ASCII `key=value` 字段（便于 grep/正则/告警解析），中文说明只允许出现在
**行尾 `—` 之后**（人读部分）。必含字段：

| 字段 | 必填 | 语义 | 期望形态 |
|---|---|---|---|
| `pgid` | ✅ | 受管范围标识（POSIX = 组长 pid） | 十进制 |
| `members` | ✅ | **仍存活**成员 pid 升序列表（僵尸不计） | `[1234,1235]`，可为 `[]`（当 PROBE-B 不可用而 PROBE-A 非空时） |
| `ladder` | ✅ | **已用掉**的信号阶梯 | `[TERM]` / `[TERM,KILL]` |
| `elapsed_ms` | ✅ | 自首次信号起的耗时 | 十进制 |
| `deadline_ms` | ✅ | 总死线 | 默认 `5000`；**注入限额时按实际注入值打印**（例如 ZERO 死线 ⇒ `deadline_ms=0`），冻结用例**不得**把 `5000` 写死 |
| `probe` | ✅ | 判定结论来源（全集 6 值） | **成功路径**：`kill0-esrch`（PROBE-A 给 ESRCH）/ `proc-empty`（PROBE-B 给空列表）；**失败路径（仅这 4 个可出现在 `Err` 分支）**：`kill0-alive` / `proc-members` / `unavailable` / `kill0-eperm` |
| `handle` | ✅ | 受管句柄（排障入口） | 原样 handle 串 |
| `fence` | 条件 | 栅栏结论（栅栏通过时应为 `ok`，缺失即视为未做栅栏 ⇒ 视为缺陷） | `ok` / `stale` / `enoent` |

### 5.2 格式模板（建议实现）

```text
B206: range-not-quiescent pgid={pgid} handle={handle} members=[{pid,…}] ladder=[{TERM[,KILL]}] elapsed_ms={n} deadline_ms={n} probe={probe} fence=ok — 信号阶梯已耗尽但范围仍未证明为空，绝不谎报静止
```

**参考示例（可被断言的具体串）**：

```text
B206: range-not-quiescent pgid=3100214 handle=proc-1731…-3100214-3 members=[3100220, 3100221] ladder=[TERM,KILL] elapsed_ms=5012 deadline_ms=5000 probe=proc-members fence=ok — 信号阶梯已耗尽但范围仍未证明为空，绝不谎报静止
```

### 5.3 可机器判定的断言（交 test-agent 冻结）

```text
^B206: range-not-quiescent pgid=[0-9]+ handle=\S+ members=\[[0-9]+(, ?[0-9]+)*\] ladder=\[(TERM|TERM,KILL)\] elapsed_ms=[0-9]+ deadline_ms=[0-9]+ probe=(kill0-alive|proc-members|unavailable|kill0-eperm) fence=ok —
```

- `members` 必须是**升序去重**（同一集合两次打印必须逐字节相同，便于按值断言）。
- **`members` 的分隔符必须容忍 `, ` （逗号 + 空格）**：当前实现用 Rust `{:?}` 打印 `Vec<i32>`
  （`Err(format!("B206: range-not-quiescent …"))`，v1.3 复核 `process.rs:526`），实际输出形如 `members=[3100220, 3100221]`。正则因此写成 `(, ?[0-9]+)*`；
  **不得**断言紧凑无空格形态（那会把当前合规实现判红）。若将来统一为紧凑形态，须同批改实现与该正则。
- **`Err` 文本的正面约束（唯一可用的断言形式）**：必须匹配 §5.3 的正则（= 以 `B206: range-not-quiescent` 开头），
  且必须**同时包含**八个字段 `pgid=`、`handle=`、`members=[…]`、`ladder=[…]`、`elapsed_ms=`、`deadline_ms=`、
  `probe=`、`fence=`。
- **禁止**断言 `!text.contains("quiescent")`——模板本身的否定式标签 `range-not-quiescent` 含该词，
  这样写会把**合规实现判红**（本轮契约早期草稿的缺陷，已修订）。若仍需「防语义反转」的负向断言，
  只能用这两条正向可判形式：`!text.contains("range-quiescent ")` 与 `!text.contains("probe=empty")`
  （同理不得断言 `!text.contains("empty")`：`proc-empty` 是合法成功值）。
- 降级/栅栏路径的独立格式：
  - `B206: range-probe-degraded probe=kill0-only reason=no-proc-fs platform={os} — 该平台只能以 kill(-pgid,0)==ESRCH 证明范围为空`
  - `B206: fence-stale pgid={pgid} handle={handle} recorded_start={n} observed_start={n|ENOENT} — 身份不匹配，未发送任何信号，视为范围已消失`

### 5.4 风格对齐（B205 已有产物）

B205 测试的失败文案风格是「前缀工单号 + 中文结论 + 字段化证据」，见
`crates/pony-agent-core/tests/process_group_reap_test.rs:305-311`：

```text
B205: {what}：进程组 pgid={pgid} 在 {timeout:?} 内未清空，仍存活的成员 pid={members:?}（kill(-pgid,0)={rc}, errno={n} ({error})）
      —— 这些进程的父进程已退出，它们就是 PPID=1 孤儿，正是 2026-11-13 事故的病灶
```

B206 沿用该风格（工单前缀 + 点名存活 pid + 探针原始证据 + `—` 后的人读解释），差异仅是把
「字段改为 ASCII `key=value`」以支持正则断言。B205 测试中的探针函数（`b205_live_group_members:255`、
`b205_group_probe_evidence:287`、`b205_wait_group_empty:297`）应被 B206 用例**复用而非复制**（同口径两处实现必然漂移）。

---

## 6. 不可回退边界（本节的每一条都是「禁止」而非「建议」）

### 6.1 X1 —— 禁止「`Ok` 不代表范围一定为空」

任何形式的下列妥协**一律禁止**（审查时按 BLOCKING 处理）：
- 「返回值保持 `Ok`，但在日志里记一条 warn」；
- 「只在 `--debug` / 环境变量开关下要求为空」；
- 「用一个固定 `sleep`（哪怕 5s）当作静止」——这正是 DSH `run-gates.ts:1350-1364` 明确拒绝的做法：
  `close` 不是静止证据，只有「进程组与后代确认消失」才是；
- 「`Ok` 语义改为『信号已投递』并在文档里说明」——**口径不可改**，此条即本契约的不可回退边界本身。

机器影子（靠 review + 可复跑命令）：`rg -n 'return Ok\(\(\)\)' crates/pony-agent-core/src/agent/process.rs`
的每一处命中都必须能回答「它前面的空证明是什么」。

### 6.2 X2 —— 不能证明为空的平台：必须降级并告警，不得静默放过

| 平台 | 可用判据 | 契约要求 |
|---|---|---|
| Linux（`/proc` 可读） | PROBE-A **或** PROBE-B；FENCE 可用 | 全合同适用（C1–C5） |
| Linux（`/proc` 被 hidepid/namespace 挡住） | 仅 PROBE-A | 判据降级为 A-only + 一次性 `code=B206-probe-degraded`；A 给非空且阶梯耗尽 ⇒ `Err`（T10） |
| 其它 unix（macOS/BSD，无 `/proc`） | 仅 PROBE-A；**FENCE 不可用** | 判据降级为 A-only；FENCE 缺失必须在启动时一次性告警 `code=B206-fence-unavailable`，并且**不得**返回到「信任 pgid 长期有效」的旧论证；A-only 无法区分的场景（组空但 pgid 已被复用）按「视为范围已消失」处理，但必须留 `probe=kill0-only` 证据字段 |
| Windows | Job Object（C6） | 语义不变 |
| 任一平台 | 两条判据都不可用 | **`Err`（不可证明 ≠ 空）**；禁止返回 `Ok` |

降级告警必须是**每进程一次**（`Once`/`OnceLock` 级别），因为高频 spawn 路径上每次告警会把日志刷成噪声，
而噪声门禁会被使用者关掉——这与 B205 元复盘的「75% 误红门禁 = 比没有门禁更糟」是同一条教训的运行时版本。

### 6.3 X3 —— 禁止提前移除句柄以「让测试变绿」

`remove(handle)` 只能用两个前驱（§2 T4/T5/T6）。任何「先 remove 再证明」的顺序都违 C3，并直接摧毁
`impl Drop for ProcessManager` 的作用对象（B205 提案第 1 项）。

### 6.4 X4 —— 禁止把错误降级为 `eprintln!`（唯一例外：S-3 的既有签名适配器）

`process.rs:568-570` 现状（`eprintln!("[process] shutdown termination error for…")` 后继续计数并 remove）
是本契约明确要消灭的形态：`eprintln!` 既不进结构化日志、也不影响返回值 ⇒ 上层永远看到「成功」。

**唯一例外（S-3，Lead 裁决 2026-11-16）**：`shutdown` 的**既有签名**（`-> usize`）因「契约单向门纪律」保持不变，
其适配器允许保留 `eprintln!`，但必须同时满足三条：① 前缀改为结构化 `[B206][shutdown-not-quiescent] …`；
② 同一逻辑另有 `shutdown_checked` 返回 `Err`（失败信号必须存在）；③ `shutdown` 的返回值**只含已证明为空的条目**。
缺任一条即回到「不合格终态」，按 BLOCKING 处理。

---

## 7. 承载测试（红相由 test-agent 冻结，executor 只读；本文只登记用例名与期望）

| 用例名（建议） | 覆盖 | 期望 | 正控/红相 |
|---|---|---|---|
| `b206_deadline_exhausted_returns_err` | T9 | `kill` 返回 `Err` 且匹配 §5.3 正则；`members` 非空 | 红相：B205 现状返回 `Ok` |
| `b206_term_then_kill_ladder_observed` | T8 | `Ok` 且阶梯记录含 `KILL` | 正控（TERM 被忽略的替身：`trap '' TERM`） |
| `b206_already_exited_nonempty_range_retains_handle` | C3 / T9 | 组长已退、组内仍活 ⇒ `is_tracked == true`（**不** remove） | 红相：现状 remove 后 `is_tracked == false` |
| `b206_already_exited_empty_range_removes_handle` | C3 / T5 | 组空 ⇒ `is_tracked == false` 且返回 `Ok` | 正控 |
| `b206_idempotent_unknown_handle_returns_ok` | C4 / T1 | `Ok(())` | 正控（既有 `process.rs:1186` 已覆盖，**勿重写**） |
| `b206_kill_unknown_handle_returns_err` | C4 / T2 | `Err` 含 `unknown process handle` | 正控 |
| `b206_fence_stale_sends_no_signal` | C5 / T4 | `Ok` 且信号发送计数 `== 0`；日志含 `code=B206-fence-stale` | 红相：现状会 `killpg` |
| `b206_fence_reaped_continues_ladder` | T4c | 组长已被本管理器 `wait()` 收尸 ⇒ 栅栏**通过**、继续 LADDER（**零信号约束不适用**）；孙进程仍在组内时最终仍须 `Err` 或真正收空 | 正控：区分「栅栏通过」与「范围已空」 |
| `b206_probe_unavailable_returns_err` | T10 | `Err`（用注入关闭两条判据，**禁止**真的破坏 `/proc`） | 红相：现状返回 `Ok` |
| `b206_shutdown_checked_errs_on_unquiet_range` | T13 / S-3 | `shutdown_checked` 返回 `Err` 且含未静默 handle 列表 | 红相：现状 `shutdown` 计数含未静默条目 |
| `b206_shutdown_legacy_count_excludes_unquiet` | T13 / S-3 | `shutdown` 返回值 == 已证明为空者数；日志含 `[B206][shutdown-not-quiescent]` | 红相：现状无条件计入 |
| `b206_single_termination_path` | C1-U / T14 | 五个入口同一条路径；探针符号在终止路径只出现一处（M7） | 红相：`shutdown` 绕过路径 |

**分权**：本表由 test-agent 冻结为 `crates/pony-agent-core/tests/**` 下的真实用例；executor 只读不改。
`--test-threads=1` 运行（与 B205 同口径：进程组探针并行会互相污染）。

### 7.1 机器到不了的（`[靠 review]` 显式登记，常载命约 2）

**R1 —— `Err` 分支端到端不可测 ⇒ `[靠 review]`。** `killpg(pgid, SIGKILL)` 的投递窗口短于一次 `/proc` 扫描；
而 PROBE-B（C1 判据 B）按契约**不计僵尸** ⇒ 首次扫描时目标通常已是僵尸 ⇒ 判空 ⇒ `Ok`。要造「连 SIGKILL 都杀不掉的
成员」需要 D 状态（uninterruptible I/O）或特权，本机不可达 ⇒ **T9 / T10 的端到端只能在具备该条件的机器上验证，
本机靠 review**。可机器证明的只有一半：`b206_zero_deadline_probes_once`（ZERO 死线 = **只探测一次、不进入等待**，
耗时上界断言）。
> **已作废的推论（防复发，务必保留）**：契约早期草稿曾推断「SIGKILL 到进程组清空必有非零延迟 ⇒ 立刻探测必然
> 看到成员 ⇒ ZERO 死线必然 `Err`」。该推断**被实测推翻**（executor 3/3 得 `Ok`，本机 `nonempty_samples = 0`），
> 与本文 §3 的实测口径一致。**作废，不得据它冻结红相或断言非空**；非空的唯一合法构造是「组内存在忽略 TERM
> 且长期存活的成员 + 死线给到足以观察」，而不是「依赖 SIGKILL 的延迟」。

**R2 —— `impl Drop` 落在 `Inner` 而非 `ProcessManager` ⇒ `[靠 review]`（Lead 已核准偏差）。**
`ProcessManager` 是 `#[derive(Clone)]`（`#[derive(Clone)] pub struct ProcessManager`，v1.3 复核 `process.rs:777`，
字段 `inner: Arc<Inner>`），对它实现 `Drop` 会在
**每一个克隆**析构时收组 ⇒ 新增一类破坏面：「随手丢一个克隆，就把别的调用方正在使用的进程杀掉」。
`Inner`（`impl Drop for Inner`，v1.3 复核 `:1655`）由 `Arc` 共享，其 `Drop` 恰好等价于「最后一个 owner 消失」，是**正确时机**，
故核准偏差。可判定的部分写成用例：① 丢一个克隆后条目仍 `is_tracked == true`、进程组仍有成员；
② 丢**全部**克隆后范围被收口（等价于 `Drop` 生效）。剩余靠 review 的部分：上述用例只抽样了 Arc 计数语义，
「任何情况下都不在中间 owner 析构时收组」这一全称命题机器判不了。
`Drop` 内的收组必须**复用唯一终止路径**（C1-U 的 U2：PROBE/FENCE/LADDER 不得在 `Drop` 里复制），
且**不得 panic / 不得无限阻塞**。

---

## 8. 与 B207 / B208 的边界，以及与 Lead brief 的事实校准

### 8.1 B207（原生归属 owner）——语义不变，判据原语可替换

C1–C5 的**语义**不随 owner 实现变化：`ManagedRangeOwner`（B207）只需提供等价原语
（`signal` / `wait_quiescent` / `inspect_task_count` / `cleanup`，对齐 DSH `managed-owner.ts:7-21`
的 `BoundProcessOwner` 五方法），C1 的「已证明为空」由 `wait_quiescent()` 的实现承担：
Linux user-systemd scope 用 `systemctl --user show -p LoadState,ActiveState,TasksCurrent`（DSH `linux-scope.ts:444-493`）
判定 `TasksCurrent == 0`；pgid 版本则用 PROBE-A/PROBE-B。
**本轮**：本机 `systemctl --user` 实测 `Failed to connect to bus: No medium found`（见 §8.3），
故 B207 只能验降级路径，live 路径标 `[SMOKE DEGRADED]`。

### 8.2 B208（遗留泄漏点清扫）——不在 C1 的适用面内，但同受 X1 约束

C1–C5 的适用面是 **`ProcessManager` 的受管范围**。B208 的两处（`updater.rs` 的一次性子进程、
`tests/acceptance/l2at-tui-models*.sh` 的后台 server）不是受管范围，因此**不要求**它们做空证明；
但 X1 的原则同样适用：它们**不得声称**「已静止」。当前落地形态（工作树）只做「发信号」，
所以其报告/日志措辞必须止于「已发送 SIGKILL / 已 `kill 0`」，不得写「已清理干净」。

### 8.3 事实校准（与本轮 Lead brief 的差异，均为本机复核结论）

| Lead brief 表述 | 本机复核事实 | 处理 |
|---|---|---|
| 「`updater.rs:1200` 无进程组」为待修状态 | **工作树已修**（`updater.rs:1200-1212` `process_group(0)`、`:1225-1240` `killpg`+`kill`+`wait`，注释标 B208） | B208 ADR 按 implemented 记；C1 不追溯 |
| 「`l2at-tui-models*.sh` 无 trap」为待修状态 | **工作树已修**（两脚本 `cleanup_all` 挂 `EXIT` + `INT`(130) + `TERM`(143)；server 以 `setsid` 自成会话/组，回收按「`kill -TERM -- -$pid` 再 `kill -TERM $pid`」精确执行，**不含 `kill 0`**） | 同上；中期曾出现的 `kill 0` 形态已被替换，新风险见 §8.4 |
| 「Windows 维持 Job Object kill-on-close + **active-process 零判定**」 | 代码里**没有** ActiveProcesses 计数判据（`rg -n 'ActiveProcess' crates/` 零命中）；判据 = `TerminateJobObject(1)` 成功（`windows_job.rs:116-131`）+ `KILL_ON_JOB_CLOSE`（`:74`） | C6 按代码现状表述，语义仍「维持现状，本轮不改」 |
| 「`shutdown` 返回 `Ok`」 | `shutdown -> usize`（`process.rs:558`），且错误被 `eprintln!` 吞（`:568`） | C1 增加 S-1/S-2 落地方案（§1-C1） |
| 「本机 `systemctl --user` 报 `No medium found`」 | **核实一致**：`systemctl --user show-environment` → `Failed to connect to bus: No medium found`，`is-system-running` exit 1 | B207 只验降级路径 |
| DSH `packages/subprocess/subprocess/README.md:62` | 该句实际在 **`packages/subprocess/subprocess-local/README.md:62`**（`Opted-in root exit does not terminate surviving descendants…`） | 引用路径已更正 |

### 8.4 已登记风险：脚本回收形态（供 Lead/test-agent 决策；本 Agent 不改该文件，写域受限）

**观察记录（工作树在本轮编写期间变化过一次，属实时快照）**：早期形态的清理体是 `trap '' TERM INT; kill 0`。
`kill 0` 回收的是**脚本自身所在进程组**，而脚本并不总是进程组长。实测（只读，仅打印）：

```text
$ bash -c 'bash -c "echo inner_pid=\$\$ inner_pgid=\$(ps -o pgid= -p \$\$ | tr -d \" \") parent_pid=$BASHPID"'
inner_pid=3110634 inner_pgid=3110633 parent_pid=3110634      # 非作业控制调用 ⇒ 与调用者同组
```

⇒ 在**无作业控制**的调用者下（CI runner、`sh -c` 链、脚本内 `bash xxx.sh`），脚本的 `pgid == 调用者 pid`，
`kill 0` 会向调用者所在组发 SIGTERM（交互式终端下每个作业独占进程组才是安全的）。**该形态在当前工作树里已不存在**：
现形态改为「server 用 `setsid` 自成会话 ⇒ `kill -TERM -- -$pid` 只打该组，FAIL 时降级 `kill -TERM $pid`」。

**已知残留风险（`[UNVERIFIED]` → 已加护栏，**未消除**）**：util-linux `setsid` 在**自身已是进程组长**时会 fork
（可用 `-f` 强制），此时 `$!` 是 `setsid` 的 pid 而非 server 的 pid ⇒ `kill -- -$pid` 可能打空、
降级分支也只杀到 `setsid` 而不含 server。脚本注释里「`$! == pid == pgid`」的等式在带作业控制调用时
**不一定成立**。

**处置（Lead 裁决 3，2026-11-16）**：已派 `exec-b208` 加护栏——server 启动后用 `ps -o pgid=` 复核实际进程组，
与预期 `$!` 不符则打 WARN 并记录**真实 pgid**（供清理与排障使用）。
**状态口径：已知残留风险 + 已加护栏，不得写成「已消除」**——护栏把「静默打空」变成「可见的不一致」，
但没有改变 `setsid` 的 fork 语义本身；真正的消除要等「server 自己回写真实 pid / 由 owner 提供精确 pgid 查询」。
此项不影响 B206 契约（脚本不在 C1 适用面内，§8.2），仅作泄漏点清扫的残留风险登记。

**机器影子（落地后可查）**：`rg -n 'ps -o pgid=' tests/acceptance/l2at-tui-models*.sh` 应命中护栏；
命中缺失即视为护栏未落地（不视为风险已消）。

---

## 9. 机器影子清单（本契约的「可查」部分；非机械项靠 review）

| # | 可查命令 | 期望 |
|---|---|---|
| M1 | `rg -n 'term_grace|TERM_GRACE|quiesce' crates/pony-agent-core/src/agent/process.rs` | 落地后非空（常量 + 使用点） |
| M2 | `rg -n 'return Ok\(\(\)\)' crates/pony-agent-core/src/agent/process.rs` | 每处命中前必须有 PROBE/FENCE 证据（靠 review 逐处确认） |
| M3 | `rg -n 'start_time|starttime' crates/pony-agent-core/src/agent/process.rs` | 落地后含 spawn 侧记录 + 终止侧比对两处 |
| M4 | `rg -n 'eprintln!\("\[process\] shutdown termination error' crates/pony-agent-core/src/agent/process.rs` | 落地后**零命中**（X4） |
| M5 | `cargo test --manifest-path src-tauri/Cargo.toml --target-dir target-test --test process_range_quiescence -- --test-threads=1` | exit 0（用例名以 test-agent 冻结为准） |
| M6 | 本文件 §3 的 S1–S4 片段 | 逐条回放，输出与「实测」行一致 |
| M7 | 唯一终止路径收据：`rg -n 'killpg|PROBE|probe_kill0|proc_live_members' crates/pony-agent-core/src/agent/process.rs` | 探针/信号派发**只出现在一条终止路径内**（C1-U / T14） |
| M8 | `rg -n '\[B206\]\[shutdown-not-quiescent\]' crates/pony-agent-core/src/agent/process.rs` | **代码行**恰好 1 处（S-3 适配器的 `eprintln!`），**排除注释行**（`///`、`//`、`#`、`*`）后再计数；实测原始命中 2 行 = 1 代码 + 1 文档引用（v1.3 复核：代码 `process.rs:1027` 的 `eprintln!` / doc `:1023`），故**不得**按「恰好 1 处原始命中」判定；行号会随实现漂移，**判据只看「代码行计数 == 1」**。旧前缀 `[process] shutdown termination error` 仍要求零命中 |

**靠 review 而非机器**：`Err` 是否真的意味着「范围未空」而不是「探针写错了」；FENCE 是否在**第一发信号之前**；
阶梯默认值是否被偷偷改小以换绿灯；降级告警是否每进程仅一次。以上四条机器影子管不到，只靠 change-review 抽查。

---

## 10. 变更记录

| 日期 | 变更 | 依据 |
|---|---|---|
| 2026-11-16 | 首版：C1–C6 正式化、判定表 T1–T13、回放样例 S1–S6、失败语义格式、不可回退边界、事实校准 | Lead 冻结口径（B206 工单）+ 本机复核（§3 实测行、§8.3/§8.4） |
| 2026-11-16（裁决后修订） | ① C1 的 `shutdown` 落地形态定为 **S-3 加法式 fail-loud**（新增 `shutdown_checked`，既有签名不变、结构化前缀 `[B206][shutdown-not-quiescent]`），并写明否决 S-1/S-2 的理由；② 新增 **C1-U 唯一终止路径硬约束**（含 U1–U4 自查清单）与判定表 **T14**；③ 判定表 T13 改写为 S-3 口径；④ 承载测试补 3 条（`shutdown_checked` / legacy 计数 / 单一路径）；⑤ 机器影子补 M7/M8；⑥ X4 增加 S-3 唯一例外；⑦ §8.4 setsid 残余风险改登记为「已知残留风险 + 已加护栏」 | Lead 裁决 1/2/3（本轮消息）+ 本机复核 |
| 2026-11-16（executor 行号清单落地，v1.3） | ① §5.1 `probe` 字段全集补两个**成功路径**合法值（`kill0-esrch` / `proc-empty`），并注明 `Err` 分支只可出现 `kill0-alive|proc-members|unavailable|kill0-eperm`；② §5.3 删除自相矛盾的 `!text.contains("quiescent")` 断言（会误杀合规实现），改为八字段正面约束 + 两条正向负向断言；③ §2-T4 拆为 **T4a–T4d 三态四分支**（含 `Ok{reaped:true}` 通过分支与闭环论证）；③b §1-C5 正文同步三态，并补「`fence=reaped` 亦不等于 `probe=empty`」；④ §5.1 `deadline_ms` 注入口径；⑤ §7.1 新增两处 `[靠 review]` 登记（`Err` 分支端到端不可测 + 已作废推论；`impl Drop` 落 `Inner` 的核准偏差）；⑥ §9-M8 放宽为「代码行恰好 1 处」；⑦ §7 追加 `b206_fence_reaped_continues_ladder`（additive，未改原 T4 行用例）；⑧ §5.3 正则与示例改为容忍 `members` 的 `, ` 分隔（实测实现用 `{:?}`，原正则会把**当前合规实现判红**） | Lead 裁决 + executor 行号清单 + 本机复核（`libc::killpg` 全文件唯一 1 处派发，位于 `fn signal_range`，v1.3 复核 `process.rs:348`；M8 原始命中 2 行 = 1 代码 + 1 doc） |


### 10.1 版本指纹与机械对齐口径

| 版本 | 行数 | 指纹 |
|---|---|---|
| v1.1（首版，C1–C6 落地） | 529 | `md5sum` = `6fe9388c0cd9450d7588498c09ded2a2` |
| v1.2（S-3 + C1-U 裁决修订） | 529 | 并入 v1.1 基线后重算，见 §10 行 |
| v1.3（executor 行号清单落地，本版） | `622` | 见下行 |

指纹口径（**自指规避**，任何人可复算）：

```text
grep -v '^\[VERIFY\]' .dev-team/contract-range-quiescence.md | md5sum
```

[VERIFY] version=v1.3 lines=622 md5=0e5fc4b959056af684b26aedc25fe19d

**改前 → 改后行号对照（本轮五处，便于复核与后续机器影子对齐；改前 = v1.1 基线 529 行）**

| 位置 | 改前行 | 改后行 | 改动 |
|---|---|---|---|
| §1-C5 正文（FENCE 三态） | 156 | `163` | 单条 `Stale` 口径 → 三态四分支 + `reaped` 说明 + 闭环论证 |
| §2-T4（判定表） | 186 | `203` | 单行 → T4a–T4d 四行 + 说明块（T4c 为 `Ok{reaped:true}` 通过分支） |
| §5.1 `deadline_ms` | 336 | `364` | 补「注入限额按实际值打印（ZERO ⇒ `deadline_ms=0`）」 |
| §5.1 `probe` | 337 | `365` | 4 值 → 6 值（成功 2 + 失败 4） |
| §5.3 `Err` 断言 | 360 | `391` | 删除 `!contains("quiescent")` → 八字段正面约束 + 两条可判负向断言 |
| §9-M8 | 517 | `578` | 放宽为「**代码行**恰好 1 处（排除注释行）」 |
| §2-T13（引用 T4 → T4b/T4d） | 195 | `223` | 复核性微调，语义不变 |
| §7.1（新增 `[靠 review]`） | — | `482` | 新增小节：R1 `Err` 不可测 + R2 `Drop` 落 `Inner` |
| §7 表（新增 T4c 用例） | — | `473` | additive：`b206_fence_reaped_continues_ladder` |
