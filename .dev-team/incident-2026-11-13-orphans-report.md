# 2026-11-13 子进程组泄漏事故（40 个 bash 死循环孤儿）取证与收口报告

- 工单：`B205`
- 取证时间：2026-11-16 10:26–11:05 +08:00
- 机器：devserver（16 核 / 30G 内存 / swap 31G），共用开发机
- 证据等级：每条结论均带可回放命令或 `path:line`；无直接证据者标 `[UNVERIFIED]`

---

## 1. 触发源定位（问题 1）

### 1.1 结论：**不是** `.dev-team/review-*.json` 的 `counterexample_cmd`

任务书假设「触发源 = `.dev-team/review-tool-timeout.json` 的 CONCURRENCY 规则反例命令」——**被取证推翻**。

| 事实 | 证据（可回放） |
|---|---|
| Nov 13 17:20–17:45 窗口内 `.dev-team/` **零文件被写** | `find .dev-team -newermt '2026-11-13 16:00' ! -newermt '2026-11-13 19:00' -printf '%T+ %p\n'` → 空 |
| `.dev-team/` 在 Nov 13 最后一次写入是 13:17:45 | `find .dev-team -newermt '2026-11-13 00:00' ! -newermt '2026-11-14 00:00' -printf '%T+ %p\n' \| sort \| tail -1` → `contracts/system-prompt-v2.md` |
| `review-tool-timeout.json` 是 Nov 4 产物 | `ls -la .dev-team/review-tool-timeout.json` → `Nov 4 00:56`；其 `reviewed_commit fecc512f…` → `git log -1 --format='%H %ad' --date=iso fecc512f` → `2026-11-04 00:24:08 +0800` |
| 该 review 的 reviewer 会话最后写入 Nov 4 00:54 | `~/.dsh/sessions/--home-dm-pony-agent--/078b406f-…/session.v4.jsonl.zstd` mtime |
| 该 reviewer 会话里死循环只出现在 reasoning/写盘文本，**无执行痕迹** | `zstd -dc …/078b406f-….zstd \| grep -c 'bash -c while :; do :; done'` → `0` |

⇒ `.dev-team/review-tool-timeout.json:12` 是**同族写法的历史先例（从未被执行）**，不是本次执行者。

### 1.2 真实触发者：teammate `b137-flaky`（B137a 执行者）自建的 CPU 压测

| 事实 | 证据 |
|---|---|
| 全机仅 1 个会话日志含该死循环 | `cd ~/.local/share/PonyAgent/session-logs && grep -rl 'while :; do :; done' .` → 仅 `subagent-2cb8e2a0-a604-4959-89c1-5511d23f23e2/session.jsonl` |
| 由父会话 `spawn_teammate` 派生 | `pa_01a1d41d-15fb-7fc0-9bdc-a85bf1a523ff/session.jsonl:1`（`name="b137-flaky"`），子会话首条 prompt 即 `You are teammate 'b137-flaky'` |
| 该子会话**从未读过任何 review json** | `grep -c 'review-tool-timeout' subagent-2cb8e2a0-…/session.jsonl` → `0`；`grep -o 'review-[a-z0-9-]*\.json' … \| wc -l` → `0` |
| 单 turn 存活窗口 17:29:03 → 17:44:01（898s） | 会话时间线 |

**两批命令的真实形态（与 json 里的 `$(nproc)` 形态不一致）**：

- **批次 A（32 个）**：`for j in $(seq 1 32); do taskset -c 0 bash -c 'while :; do :; done' & done; … pkill -f "taskset -c 0 bash -c" 2>/dev/null; …`
  - 落盘证据：`/tmp/pre_chain_*.log` mtime 17:38:56.62 → 17:39:10.08
  - 该次调用 `state=error`，退出码 `-1`（自杀，见 §2）
- **批次 B（8 个）**：`for j in $(seq 1 8); do taskset -c 8+12-15 … done; HOGS=$(jobs -p); … for p in $HOGS; do kill $p; done; echo "load-round done"`
  - 落盘证据：`/tmp/load_parallel_tools.log` 17:40:23.66 / `/tmp/load_deadline.log` 17:40:27.80 / `/tmp/load_run_command.log` 17:40:28.06
  - exitCode `0`，末行打印 `load-round done`（**看起来"干净结束"**）

**判定性算术（这不是推测，是交叉验证）**：

| 观测 | 计算 | 是否吻合 |
|---|---|---|
| 32 个各占 **3.1%** CPU | 32 个挤 1 核（`taskset -c 0`）→ 100%/32 = **3.125%** | ✓ |
| 8 个各占 **49.5%** CPU | 8 个挤 4 核（`taskset -c 12-15`）→ 2 个/核 → **50%** | ✓ |
| cmdline 恰为 `bash -c while :; do :; done`（**无** `taskset` 前缀） | `taskset … exec` 覆盖 argv；亲和性保留；pgid 未脱离 | ✓ 已实测复现 |

`nproc`=16，故任务书推测的「32 = 2 × nproc ⇒ 同一条命令被并发执行 2 次」**不成立**：32 是 `seq 1 32` 字面量，与 nproc 无关。

> 注：任务书写「17:37 / 17:39」，硬锚点日志 mtime 是 17:38:56–17:39:10 与 17:40:23–17:40:28，早约 1.5–2 分钟。`[UNVERIFIED]` 报告时刻可能来自采样/取整口径；以 `/tmp/*.log` mtime 为准。

---

## 2. 为什么清理没生效（问题 1 后半）——三重叠加，缺一层都不复发

### 机制 (i)：`/bin/sh` 是 **dash** ⇒ `$(jobs -p)` 恒为空 ⇒ 清理循环 0 次迭代

```console
$ ls -l /bin/sh
… /bin/sh -> dash
$ /bin/sh -lc 'sleep 2 & sleep 2 & HOGS=$(jobs -p); echo "HOGS=[$HOGS]"'
HOGS=[]
$ bash -lc 'sleep 2 & sleep 2 & HOGS=$(jobs -p); echo "HOGS=[$HOGS]"'
HOGS=[1802233 1802234]
```

受管命令一律由 `sh -lc "cd '<cwd>' && <command>"` 执行（`crates/pony-agent-core/src/agent/tools/run.rs:1080`），故**批次 B 的 `HOGS` 是空集合**，循环 0 次迭代，脚本正常 `exit 0`。

### 机制 (ii)：`taskset`/`exec` 改写 argv ⇒ `pkill -f` 永不匹配，且自匹配自杀

- `taskset -c 0 bash -c 'while :; do :; done' &` 中的 `taskset` 会 `exec` 目标，**argv 变成 `bash -c while :; do :; done`**（已实测：`tr '\0' ' ' < /proc/$PID/cmdline` 无 `taskset`，`psr=0` 亲和性保留）。
- 故批次 A 的 `pkill -f "taskset -c 0 bash -c"` **一个都匹配不到**。
- 更糟：该模式串本身出现在承载它的 wrapper `sh -lc "…pkill -f \"taskset -c 0 bash -c\"…"` 的 argv 里 ⇒ **pkill 先杀掉自己**（`state=error`，退出码 `-1`），后续语句再没执行。
- 事后补救用 argv **精确相等**匹配（`['taskset','-c','0','bash','-c',…]` vs 真实 `['bash','-c','while',':;','do',':;','done']`）⇒ 会话记录里实测输出 **`killed hogs 0`**。

### 机制 (iii)：即使清理语句写对也不够 —— 运行时 `already_exited` 分支不收组

- 受管 spawn 执行了 `command.process_group(0)`（`process.rs:686-690`）⇒ 子进程自成进程组长，**pgid == pid**。
- 但 `libc::killpg` **只在显式 terminate / cancel / timeout / shutdown 时**发生（`process.rs:1040`，位于 `terminate`）。这两条命令**自己 `exit 0`**（批次 B 打印 `load-round done`），于是：
  - `kill_inner`（`process.rs:430`）在 `:440` 判定 `already_exited` 为真 → 进入 `:444` 分支
  - 该分支：Windows 侧 `:447-451` 仍 `entry.job.terminate(1)` **收整个 Job**；**POSIX 侧无任何对应动作**，只「关 stdin + 删条目 + `Ok(())`」
  - ⇒ 无人对进程组发信号 ⇒ 孙进程 PPID=1 孤儿，`cwd=/home/dm/pony-agent`，存活 2d16h

> 关键澄清：孙进程**没有**"逃出进程组"（无 `setsid`/`nohup`，pgid 与父 shell 同组）。它们成为孤儿的原因是**组长死后没人对整组发信号**。

### 后果量化
load average 204；swap 31G 100% 打满；memory PSI some 88%；swap 分区 `%util` 100%；整机 thrashing。2026-11-16 手工 TERM 清理：load 204 → 22.67，swap 31G → 5.8G。

---

## 3. 同类隐患审计（问题 2）

### 3.1 `counterexample_cmd` 全量清点

- 语料：`.dev-team/**/*.json` 可解析 **658** 个；含 `counterexample_cmd` **255** 个文件；`findings[]` 级字段 **1007** 条
- 破损 **1** 个：`.dev-team/review-final.json:62`（`Invalid \escape`），未计入
- 会泄漏后台子进程的条目：**13 条 / 10 文件**（C1a 链末清理 2 · C1b 中途清理 2 · C1c 仅 `wait` 3 · C1d 零清理 6 · C2 死循环 1（与 C1a 重叠）· C3 已具 `trap`/`timeout -k` 白名单 **0**）
- **唯一权威口径 = 门禁落地后同一次全量复跑**：对 173 个含 `counterexample_cmd` 的文件跑新门禁 → `exit 0 = 112 / exit 1 = 51 / exit 3 = 10`（Lead 独立复跑，与 executor 报告逐字一致）。**新增红 = 10 个文件**。
  - 中间过程出现过三个不同数字（13 / 34 文件 / 36），原因是判据口径差异与门禁首版正则过宽（`&str` 等 Rust 借用语法被误当后台作业，产生 26 个误红）。**收窄后归零**，以 10 为最终值。

明细中最危险的三条（BLOCKING）：

| 产物 | 目标 `file:line` | 形态 |
|---|---|---|
| `.dev-team/review-tool-timeout.json#0` | `crates/pony-agent-core/src/agent/tools.rs:3029` | 事故同族原件（`$(nproc)` 死循环 + 链末 `kill $(jobs -p)`） |
| `.dev-team/review-join-b.json#0` | `crates/pony-agent-core/src/bin/cli.rs:894` | `python3 m.py & MP=$!` … 链末 `kill $MP`，无 `trap` |
| `.dev-team/review-stage0-l3b.json#1` | `crates/pony-agent-core/src/agent/session/store.rs:212` | `(holder &); …` 零清理 |

### 3.2 门禁侧零判别力

`/home/dm/.agents/skills/dev-team/scripts/dev-review-lint.py` 共 21 项校验（`:22,28,30,32,34,36-40,42,45,50-53,55,57,75,79,83,88,92-94,95-96,103-105,108,112,116-118`），**唯一的内容校验**是 `:116-118` 的 `if len(cmd.strip()) < 5`。全文对 `kill|jobs|nohup|setsid|disown|trap|while` 命中数 = **0**。

⇒ 事故原件今日复跑仍 `[PASS] Exit Code: 0`。**物理反例门禁形同虚设。**

### 3.3 仓库代码 / 脚本层同类模式

- `& done` / `jobs -p` / `disown` / `nohup`：`*.rs/*.ts/*.mjs/*.js/*.py/*.sh` **0 命中**（排除 `node_modules/`、`target*/`、`.git/`）⇒ 代码层不复现事故写法
- `while :;` 仅 1 处：`tests/acceptance/remediation/wave35_gate.sh:496`，有 `-ge $max` 退出（`:511`）+ `sleep 5`，非自旋、无后台 ⇒ 不泄漏
- **真泄漏（验收脚本，每次运行必漏 1 个常驻 server）**：
  - `tests/acceptance/l2at-tui-models-auth.sh:123`：`python3 "$RECEIPTS/strict_server.py" … &`，`trap` 只做 `tmux kill-server`（`:52-53`），全文件无 `kill`/`pkill`
  - `tests/acceptance/l2at-tui-models.sh:93`：`python3 fake_server.py … &`，`trap` 同上（`:50-51`），清理只在 `:359` 中途 `pkill` ⇒ 早退/中断即漏
- 低危（竞态窗口）：`tests/acceptance/l2at-tui-e2e.sh:86,117,191` 无 `trap`，靠紧随的 `kill -INT`(`:89`)/`kill -HUP`(`:129`)/`kill -INT`(`:194`)
- 已正确（`trap` + 真实 bg pid）：`tools/tui-audit/run_audit.sh:101`、`tests/acceptance/wave59/headless-cli-e2e.sh:62`、`l2at-tui-overlay-cancel.sh:75`、`l2at-tui-workspace.sh:82`

---

## 4. 代码收口路径核查（问题 3）

### 4.1 `process.rs` 终止入口判定表

| 入口 | 行号 | 触发场景 | killpg | `child.kill()` 兜底 | 判定 |
|---|---|---|---|---|---|
| `kill` / `kill_if_exists` | `:413` / `:419` → `kill_inner :430` | 显式 terminate | 是（经 `terminate`） | 是 | OK |
| `cancel` | `:466-468` | cancel（`kill` 别名） | 是 | 是 | OK |
| `kill_after` | `:475-493`（线程 `:477`） | 超时 | 是 | 是 | OK |
| `shutdown` | `:538-563` | 会话收尾 | 是（`:557`） | 是 | OK |
| `terminate`（核心汇聚点） | `:1013-1086`；killpg `:1040`、`child.kill` `:1042`、有界 reap `:1043-1056` | 上述全部 | **是**（`:1033` `cfg(not(windows))`） | 是 | OK |
| `kill_inner` **`already_exited` 分支** | `:444-461` | 组长已退出、孙进程仍在 | **否** | 否 | **LEAK（已修，见 §5）** |
| `poll_inner` 正常退出 | `:927-971` | 正常退出、组内无残留 | 否 | 否 | OK |
| spawn 失败清理 | `:66-82` / `:115-174` / 调用点 `:820,838,860` | spawn 后失败 | windows-only | 是 | OK（windows） |
| **unix pipe-missing 分支** | `:815-845`（`:843-844` 置 `job_error=None` → `return Err`） | 取不到 pipe | **否** | **否** | **潜在 LEAK**（当前不可达：`Stdio::piped()` 恒设于 `:673-675`，注入是 `cfg(all(windows,test))` `:811`） |
| **`impl Drop`** | 无（仅 `:1733` windows `NativeProcessHandle`） | 管理器 drop / 宿主退出 | 否 | 否 | **潜在 LEAK**（未收组） |

**答案（问题 3）**：超时 / 取消 / 中断 / 会话收尾四条公开路径**都已走 killpg**，`child.kill()` 只是兜底 —— 这一点任务书的担心已排除。真实缺口是**第三条路：命令自己退出**，此时 `already_exited` 分支在 POSIX 侧不收组。

### 4.2 `child.id()` 作 pgid 的前提

- 唯一 `Command::new` `:670`、唯一 `.spawn()` `:716`、`process_group(0)` 闸门 `#[cfg(unix)] :686-690` ⇒ **pgid == pid 成立**
- ⚠️ cfg 不对称：spawn 用 `#[cfg(unix)]`，`terminate` 的 killpg 用 `#[cfg(not(windows))] :1033` ⇒ 非 unix 非 windows 目标上会拿非组长 pid 当 pgid（`[UNVERIFIED]` 仅静态推导，无该目标构建验证）
- pid 复用不构成风险：`terminate` 在 `:1017` 对已 `Exited` 提前返回；未 reap 的 zombie 仍占 pid
- 已知局限（非回归）：子进程自行 `setsid` / 双 fork 后脱离该组，killpg 无效 —— `sandbox.rs:11,40` 已显式声明不承诺阻止
- ⚠️ 文档失真：`process.rs:26` 仍写「non-Windows behavior is direct-child lifecycle only」，未反映 B009 已加 killpg ⇒ 后继维护者可能据文档回退

### 4.3 `process.rs` 之外的 spawn 路径

| 站点 | 组化 | 终止 | 判定 |
|---|---|---|---|
| `crates/pony-agent-core/src/agent/updater.rs:1200` 裸 `std::process::Command::new(target)` | **无** `process_group` | 只 `child.kill()`（`:1218`/`:1228`） | **LEAK-RISK**（孙进程逃逸面，待后续） |
| `terminal.rs:407` + `pre_exec setpgid` `:427-433` | 有 | killpg `:217`/`:261` | OK |
| `jobs.rs:609` + `pre_exec setpgid` `:625-631` | 有 | killpg `:431` | OK |
| `acp/transport.rs:282→293`、`plugin_host/transport.rs:177→188`、`mcp_client.rs:400→410`、`hook_commands.rs:408→418`、`lsp.rs:51→67` | 有 `process_group(0)` | 各 kill 分支未逐行深审 | `[UNVERIFIED]` |
| `git_search.rs:114` `.output()` | 无组 | 同步等待 | 无泄漏面 |
| `src-tauri/src/platform.rs:325/334` `.spawn().map(\|_\| ())` | 无组 | 丢弃 `Child` | 僵尸由 init 收，非死循环泄漏 |

### 4.4 既有测试覆盖现状

- **真·组残留断言**：`crates/pony-agent-core/src/agent/jobs.rs:1148`（探针 `killpg(pgid,0)` `:1212` + 断言 `!pgid_alive` `:1221-1226`）
- **真·孙进程断言**：`crates/pony-agent-core/tests/run_timeout_promote_test.rs` CT-P8 `:834/843-848/890-897`
- **名不副实**：`tests/plugin_host_contract.rs:544` 只用插件**子进程** pid（`:578`/`:583`），未枚举孙进程
- **明确不足**：`process.rs:1591`/`:1609`/`:1620` 只断言 `state==Exited` + 条目移除 ⇒ **killpg 退化成 `child.kill()` 仍全绿**
- 无 `Drop` 测试、无 unix pipe-missing 分支测试；descendant 级断言全在 `cfg(windows)`（`:1813/1834/1865/1899`）

---

## 5. 修复方案与实施（问题 4）

### 5.1 CX-SAFE 反例命令安全契约（真源）

- 契约正文：`.dev-team/contract-counterexample-cmd-safety.md`
- 条款锚点：`/home/dm/.agents/skills/dev-team/SKILL.md:111`（schema）、`:119` ② 段（CX-SAFE-1..5）
- 要点：
  - **CX-SAFE-1** 含后台作业（`&`、`& done`、`nohup`、`setsid`、`disown`）**必须**在命令最前部（或最外层）`trap 'kill 0' EXIT INT TERM`，保证正常退出 / 超时 / Ctrl-C 三条路径都整组回收；**禁止**把 `kill $(jobs -p)` 排在链末作为唯一清理
  - **CX-SAFE-2** 或外层 `timeout -k 5 <N> bash -c '...'`（`N*1000 < subagent_turn_timeout_ms` = 120000）
  - **CX-SAFE-3** 禁 `while :; do :; done` / `yes` / `sleep infinity` 作压力源（**无条件**，即使外层有 `timeout`）
  - **CX-SAFE-4** 禁依赖 `$(jobs -p)`（dash 下恒空）
  - **CX-SAFE-5** 禁 `pkill -f` / `pgrep -f` / `killall` 做清理（argv 改写 + 自匹配自杀）
- **为什么**（写进契约）：链末清理 = 单点失败 —— 前置语句非零退出 / 被信号中断 / 超时杀进程时清理永不执行 → 孙进程被 init 收养为 PPID=1 孤儿。

### 5.2 机械门禁

`dev-review-lint.py` 新增 `lint_counterexample_safety()`：
- 退出码：`0` 全绿 / `1` 既有结构性违规（语义不变）/ **`3` CX-SAFE 违规（新增）** / `4` 判定器内部错误 fail-closed / `2` CLI 用法错误。优先级 `4>3>1>0`
- stderr 每条前缀 `[CX-SAFE:<模式ID>]`；内置 `--self-test` 跑 7 段 fixture
- **验收铁证**：`.dev-team/review-tool-timeout.json` 由 `exit 0`（假放行）转为 **`exit 3`（拦截）**，命中的四条：
  `[CX-SAFE:SPIN_LOOP]` / `[CX-SAFE:CX-SAFE-4]`（`$(jobs -p)`）/ `[CX-SAFE:NO_RECLAIM]` / `[CX-SAFE:TAIL_KILL_ONLY]`
- **误红收窄（本轮中途修掉的真实缺陷）**：首版正则出来 36 个新增红里 **26 个是纯文本性命中**——`&str`/`&mut`/`&self`/`&PathBuf`/`&RULES`/`skills=[]&omitted=[]` 等 Rust 借用语法与 URL 查询串被当作 shell 后台作业、裸 `pgrep -af` 只读观察被当作清理原语、`#` 注释里的 `yes` 被当作压力源。**36 红里 26 个误红 = 75% 噪声率，这种门禁会被使用者直接关掉，等于没做。** 故追加三条收窄规则：
  - **R1** `&` 必须是 **token 边界**（前字符 ∈ {行首, 空白, `;`, `)`, `|`, `"`, `'`, 反引号}，后字符 ∈ {空白, `;`, `)`, `|`, `#`, 行尾}，且 `&&` 排除）
  - **R1+** 引号状态机把**非 shell 代码**的引号内容掩成等长空格（`bash/sh/dash/zsh -c`、`eval` 例外仍扫）—— 这是消除 `grep '… & …'` 类误红的真正解法
  - **R2** 裸 `pgrep -f` 放行，仅 `pgrep -f … | kill` / `| xargs kill` 算清理原语（`pkill -f` 家族含选项乱序 `pkill -9 -f`、`killall` 仍算）
  - **R3** 判据匹配前剥离行内 `#` 注释（仅 `#` 前为行首/空白），用等长空格填空以保留偏移坐标
  - 收窄后：**误红 26 → 0，真实红 10 条一条不少**，且 `old exit 1 → new exit 0` 仍为 **0**（既有结构校验零放宽）。
- **判据说理（写进契约，防后人误以为门禁被削弱）**：`&str` 是 Rust 借用语法、`pgrep -af` 是只读观察、`#` 注释是永不执行的惰性文本 —— 三者**物理上不可能泄漏子进程**，故它们不是「被豁免的违规」，而是「从未是违规」。

### 5.3 运行时收口

`process.rs:444` 的 `already_exited` 分支在 POSIX 侧补 `libc::killpg(pgid, SIGKILL)`，与 Windows 侧 `job.terminate(1)` 语义对称。理由：命令自行 `exit 0` 时不会有任何 terminate/timeout 触发，此处是**唯一**能收组的时机。

### 5.4 回归测试（冻结，由独立 Test Agent 编写）

`crates/pony-agent-core/tests/process_group_reap_test.rs`（4 用例）：

| 用例 | 角色 | 断言 |
|---|---|---|
| `b205_timeout_path_reaps_group_after_shell_exit` | **红相** | 受管 shell 打印 `$$` 与两个孙进程 pid → shell 自行 `exit 0` → `kill_after` 开火 → 8s 内组内**非僵尸**成员必须为 0 |
| `b205_cancel_path_reaps_group_after_shell_exit` | **红相** | 同上，走 `cancel` |
| `b205_control_timeout_path_reaps_group_while_shell_alive` | 正控（必须绿） | shell 前台 `wait` 时超时收组成功 —— 证明断言机制有效，红相归因成立 |
| `b205_control_cancel_path_reaps_group_while_shell_alive` | 正控（必须绿） | 同上，走 `cancel` |

红相基线（改前）：`exit 101`，`test result: FAILED. 2 passed; 2 failed`
```
B205: 超时终结路径返回后进程组未清空（孙进程泄漏）：进程组 pgid=1832462 在 8s 内未清空，
仍存活的成员 pid=[1832465, 1832466]（kill(-1832462,0)=0, errno=0）—— 这些进程的父进程已退出，
它们就是 PPID=1 孤儿，正是 2026-11-13 事故的病灶
```
绿相（改后）：**`exit 0`，`test result: ok. 4 passed; 0 failed`，`finished in 2.12s`**（复跑 3 次：2.12 / 2.11 / 2.14s；Lead 独立抽验亦 `4 passed / exit 0`）

**归因五证（防「4 passed 就算数」式假绿）**：
1. 红绿之间测试文件**字节不变**（sha256 `00d269e8…`，mtime 10:35:09；红相日志 10:35:33、绿相 10:41:27）
2. 源码 diff 仅 **1 个 hunk / +22 −2**，实质变化只有 `already_exited` 分支新增 `#[cfg(not(windows))] killpg`
3. `--nocapture` 实证：受管 shell `exit 0` 之后两个孙进程**仍存活** `[1885905, 1885906]`，是终结路径才清空 ⇒ 不是「孙进程自己死的」
4. 耗时 **17.69s → 2.12s**（两条红用例各耗尽的 8s 组清空上限整体消失）⇒ 首次探针即已为空
5. 两条正控在红相与绿相中**全程为绿** ⇒ 判据机制未漂移，红→绿翻转只能归因于新增的 killpg

`cargo check -p pony-agent-core --tests` exit 0（零新增 warning）；`cargo test --lib process` exit 0（21 passed，无回归）。

> 注：Test Agent 独立发现并纠正了 Lead briefing 的错误假设 —— `bash -c 'sleep 120 & sleep 120 & wait'` 形态**今天已是绿的**（B009 的 killpg 本身没坏），真实缺口在 `already_exited` 早退分支。

### 5.5 为何**不**重写 255 个历史 review json

它们是**审核证据**（`attestation.self_signed` 溯源语义），改写即篡改证据；且旧产物不会再被执行（`.dev-team/` 在 Nov 13 起基本静默），重写不减少风险 —— **风险只在新产物**。故修在两侧：契约（生成侧）+ lint `exit 3`（门禁侧），历史命中只做「只读留档 + 一次性复跑出清单」。

### 5.6 ADR

- `.agents/notes/implemented/process/2026-11-16-counterexample-cmd-safety-contract.md`（Status: implemented，4 条 Alternatives）
- `.agents/notes/proposed/architecture/2026-11-16-process-group-reap-gap-remediation.md`（Status: proposed，4 条 Alternatives，含「未实施不得引用为已落地」红线）

---

## 6. 顺带核查：僵尸进程与 tui_* 内存（问题 5）

### 6.1 僵尸进程：21 个，**无一个**挂在禁止触碰的服务上

| 归属 | 数量 | 父进程 | 僵尸存活 | 父状态 |
|---|---|---|---|---|
| `docker-mcp` 容器 | 4（`wget`×2 / `grep`×2） | pid 3944 | 15d15h | S，存活 |
| `wb_proxy.py` 容器 | 10（`python`） | pid 3923 | 36m–4d22h | Ssl，存活 |
| `magpie` 容器 | 1（`magpie-bun`，会话首进程 `Zs`） | pid 1617062 | 7d22h | Ssl，存活 |
| **dm 的 pony TUI 会话** | 6（`bash`/`sh`/`npm run typeche`/`cargo`×2） | `3224227`/`3209737`/`2973474`/`941439`/`895290` | 16m–16h | 半数 **T（stopped）** |

- 与禁止服务的交集为**空**（`ps` pid 集合比对：k3s-server 3830631、prometheus 1585767、clickhouse 1587386、grafana 1585858、loki 1586332、tempo 1585128、minio 3600/1588814 —— 一个都不在上表 ppid 里）
- 成因（**非推断**）：`/proc/3923/task/3923/children` 至今仍列 10 个已死子进程 ⇒ 父进程从未 `wait()`
- 两个 `cargo` 僵尸的父是 **worker 线程**（`/proc/895290/task/1265986/children → 1629951`、`/proc/941439/task/1454376/children → 1624747`）⇒ 典型 `std::process::Child` 被 drop 未 `wait`（Rust `Child` 无 `Drop` 实现）
- **硬事实**：僵尸已死，`kill -TERM/-KILL <zpid>` **完全无效**；清理对象必须是**父进程**。
- 风险量化：21 ÷ `pid_max` 4194304 ≈ 0.0005% ⇒ **无 PID 耗尽风险**；成本是审计噪音。
- **建议不做**：不为清僵尸 TERM 掉 6 个活着的 dm pony 会话（丢用户会话上下文）；`docker restart` 会连带重启既有服务，需单独审批窗口。
- ⚠️ 注意与 §5.3 的关联：`std::process::Child` 被 drop 不 reap **同样**是本轮 `impl Drop` 缺口的同源问题。

### 6.2 6.8G swap：历史遗留，**已基本停止活跃使用**

- `SwapCached: 1348008 kB`（1.29 GiB 页同时在 RAM 与 swap）⇒ 曾被换出、已读回
- `vmstat 1 3` 的 `si` = 629 → 52 → 8 kB/s（衰减），`so` = 0/0/0；5 秒内 `pswpout` 增量 **0**、`pswpin` 仅 42 页
- memory PSI `full avg10=0.00`、`allocstall_*` 5 秒无增量 ⇒ **当前无 thrash**
- 归属（`smaps_rollup` 可读进程合计 4.56 GiB / 6.80 GiB）：node 961M+715M、pony 会话 296M/240M/214M/134M/103M、python 273M、vite 268M、jellyfin 180M、opencode 146M、minio 93M、7 个 github-runner ≈ 510M；缺口 2.24 GiB 落在 root 进程（k3s/containerd，`smaps` 不可读）`[UNVERIFIED]`

### 6.3 `tui_*` 测试内存：报告的数值**无法复核**，但真实膨胀机制已找到

- `tui_process_flow` **在仓库中不存在**（`grep -rn 'tui_process_flow' --include=*.rs` → 0 命中；`target-test/debug/deps` 亦无）。最接近的真实目标是 `crates/pony-agent-core/tests/tui_process_fold_acceptance.rs`（15 个 `#[test]`，二进制 70,475,232 B）。
- `tui_mouse_selection` 真实存在两份：`tui_mouse_selection_acceptance.rs`（11 个 `#[test]`）、`tui_mouse_selection_adversarial.rs`（4 个）。
- 当前机器上**没有任何 `tui_*` 进程在跑** ⇒ 「13.1GB / 4.1GB」两个数值 `[UNVERIFIED]`（原始观测来源不在仓库内）。
- **真实膨胀机制（仓库内可证）**：49 个 `tui_*.rs` 测试文件中 **39 个**用 `#[path = "../src/bin/tui.rs"] mod tui;`（例：`tui_process_fold_acceptance.rs:19`、`tui_mouse_selection_acceptance.rs:4`）⇒ 每个测试 crate **各自重新编译 12,891 行的 `tui.rs`**（`crates/pony-agent-core/src/bin/tui.rs`，635,128 B），一个文件改动触发 39 个 70MB 级测试二进制重编译。`target-test/debug/deps` 现存 **321 个可执行测试二进制、合计 8.1 GB**。
- 测试性质：全部是普通 `#[test]`，**0 个 `#[ignore]`**，379 个 `#[test]`；显式大批量仅 `tui_interaction_adversarial.rs:97`（`0..1_000`）与 `:122`/`:127`（各 `0..5_000`），量级 1e3，**不足以解释 GB 级**。
- **真·重负载**：5 个文件经 PTY 驱动跑**真实 `pony` 二进制**（`tui_startup_input_guard.rs:94-100`、`tui_overlay_cancel_acceptance.rs:1029`、`tui_audit_no_panic_regression.rs:74/109`、`tui_audit_scenarios_regression.rs:82/374/418`、`tui_audit_hermeticity_regression.rs:85`）⇒ 内存量级由被驱动的 `pony` TUI 决定（历史 ADR 记录单进程 VmHWM 8.5GB）。

### 6.4 并发与 profile 评估（所需事实）

- `cargo test` 默认 libtest 线程 = `nproc` = **16**；仓库/CI/Makefile **无任何强制 `--test-threads`**
- 编译并发**已被限流**：`./.cargo/config.toml:32` → `jobs = 3`（注释 `:31` 明写"避免 OOM"）；`~/.cargo/config.toml` → `jobs = 2`
  - ⚠️ **优先级陷阱**：cargo 配置由浅入深合并，在 `/home/dm/pony-agent` 内执行时**仓库 `jobs=3` 覆盖 `~/.cargo` 的 2**；而 `[build] jobs` **只限 rustc 进程数，不限 libtest 线程数与 rustc 内部 codegen 线程**
  - ⚠️ `~/.cargo/config.toml` 的 `jobs=2` 是 ADR `.agents/notes/implemented/bug-fix/2026-11-11-cross-platform-session-db-compaction-and-devserver-resource-governance.md:22` 的落地物（`:23` 记录 `vm.swappiness 60→15`），ADR 视其为硬约束，**实测在仓库内被覆盖**
- `[profile.dev]`/`[profile.test]`：`incremental=true`、`debug=0`（仅影响体积，不影响运行时堆量级）；`src-tauri/Cargo.toml` 无 `[profile]` 段
- **`--release` 对比无仓库内量化证据** `[UNVERIFIED]`：无任何文件记录 `--release` 下 tui 测试的 RSS

**结论（评估，非强制）**：
1. 限制测试并发**有立即收益且零风险**：`--test-threads=N`（建议 4~6）+ 统一 `CARGO_BUILD_JOBS`，`.cargo/config.toml` 的 `jobs=3` 与 `~/.cargo` 的 `jobs=2` 应归一（当前互相矛盾，且外层被内层覆盖，ADR 的"硬约束"实际未生效）。
2. 改 `--release` **证据不足**：`debug=0` 已关符号，release 主要省的是 `debug_assertions`/边界检查，与「PTY 驱动 pony TUI 常驻堆」的量级无直接因果 ⇒ 应先加 `--test-threads` 限制与 `#[ignore]` 隔离重负载用例，而非改 profile。
3. 真正的结构性浪费是 **39/49 个测试文件 `#[path]` 重编 12,891 行 `tui.rs`**（321 个测试二进制 / 8.1 GB）—— 这是构建时间与内存的双重乘数，比 profile 选择影响大得多。

---

## 7. 未处置 / 后续排期

| 项 | 状态 | 理由 |
|---|---|---|
| `process.rs` 无 `impl Drop for ProcessManager` | **未做**（proposed ADR #2 记录） | 需定 Drop 语义（是否阻塞 reap、宿主退出路径），本轮不扩闭包；风险已登记 |
| `process.rs:815-845` unix pipe-missing 分支不收组 | **未做** | 当前不可达（`Stdio::piped()` 恒设 + 注入 cfg 限 windows）；已登记「若未来放宽 piped 前置必须补收组」 |
| `updater.rs:1200` 裸 `Command::new` 无进程组 | **未做** | 不在本轮写域；LEAK-RISK 已登记 |
| `terminate` 的 killpg 用 `cfg(not(windows))` 与 spawn 的 `cfg(unix)` 不对称 | 见执行收据 | 若本轮未改，登记为后续 |
| `l2at-tui-models-auth.sh` / `l2at-tui-models.sh` 后台 server 无 `trap` 回收 | **未做** | 验收脚本层泄漏，每跑必漏 1 个常驻 server；建议同批修 |
| 6 个 dm pony 会话僵尸 | **未清理** | 需 TERM 父进程，会丢用户会话上下文；建议由用户决定 |
| `.dev-team/review-final.json` JSON 破损（`:62`） | **未修** | 历史证据，只读 |
| `.cargo/config.toml` `jobs=3` 与 `~/.cargo/config.toml` `jobs=2` 矛盾 | **未改** | 属既有资源治理决策，需单独复核 ADR |
| `--release` 对 tui 测试内存的影响 | **未实测** | 只读约束下不产生构建产物；需授权后实测 |

---

## 8. 覆盖盲区（诚实声明）

- 未查看：17:37 前后那 40 个 PID 的 `/proc`（Nov 16 已被清理，无残留）；`/var/log/sysstat` 原始档；`journal` 之外的 `/var/log/**`；PonyAgent 应用内部数据库中的任务/团队状态历史；其他主机/远端日志
- root 进程（k3s/containerd 等）`smaps_rollup` 不可读 ⇒ 2.24 GiB swap 精确归属 `[UNVERIFIED]`
- 6 个 `process_group(0)` 站点（acp / plugin_host / mcp / hook / lsp）的 kill 分支未逐行深审 `[UNVERIFIED]`
- 40 个使用 legacy `counterexample` 键（非 `counterexample_cmd`）的历史产物语义面未逐条审计
- 未跑 `docker stats`/`docker inspect`（需 socket 权限）
- **证据污染说明**：`~/.dsh/storages/session_projcache/sessions/{b7c62c28,606f7044,62566d1c,…}.json` 也含事故命令字符串，但 mtime 全为 2026-11-16 10:29（本会话族投影重写），**不可**作为证据。
