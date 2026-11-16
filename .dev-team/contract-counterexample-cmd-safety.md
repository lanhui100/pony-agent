# 契约 CX-SAFE：`counterexample_cmd` 反例命令安全基准

> **唯一编写方**：L2-P（Protocol Agent）｜**冻结方**：Lead｜**机器执行方**：executor（`dev-review-lint.py`）
> **真源链**：`.agents/skills/dev-team/SKILL.md`（schema `:111`、物理反例门禁 `:119`）→ 本文（判定清单）→ `dev-review-lint.py`（机械校验）
> **范围**：仅约束审查产物 `.dev-team/review-*.json` 中 `findings[].counterexample_cmd` 的**文本形态**。本文所有样例均为**文本**，严禁执行。

---

## 1. 事故因果（单一失败点）

2026-11-13 的 `counterexample_cmd`（只读证据：`.dev-team/review-tool-timeout.json` `findings[0]`）：

```bash
for i in $(seq 1 $(nproc)); do (while :; do :; done) & done
for i in $(seq 1 50); do cargo test ...; done
kill $(jobs -p) 2>/dev/null        # ← 清理排在整条链最末尾
```

**为什么是事故**：**链末清理 = 单点失败**。清理语句只在「前面所有语句都成功走到末尾」时执行；一旦前置语句**非零退出** / **被信号中断** / **被外层超时杀进程**（`cargo test` 失败即 `&&` 链短路、reviewer 会话被 timeout 或 Ctrl-C 打断），清理**永不执行** → 已 `&` 脱离前台的孙进程被 **init 收养为 PPID=1 孤儿**，且无任何父进程还能 `wait()` 它。

**实测后果**：40 个 `bash -c while :; do :; done` 孤儿存活 **2 天 16 小时**（PPID=1），load average 204、swap 31G/100%、内存 PSI some 88%、磁盘 `%util` 100%，整机 thrashing；2026-11-16 手工 TERM 清理后 load 204→22.67。

**当前门禁为何没拦住**：`dev-review-lint.py` 仅有「必填字段（`:103`）」与「非空 ≥5 字符（`:116-118`）」两项，事故命令 **Exit Code 0 通过**（本机复跑已确认）。本契约补齐该缺口。

> **B205 取证修正（E1 侦察，2026-11-16）**：`/bin/sh`→dash，受管命令经 `sh -lc`（`tools/run.rs:1077-1081`）执行，
> **dash 下 `$(jobs -p)` 恒为空**（实测 `HOGS=[]`）⇒ 清理循环**恒执行 0 次**，`taskset`/`exec` 又改写 argv ⇒
> `pkill -f` 永不匹配且常自匹配自杀（`exit -1`）。即「链末单点失败」之外还有**清理原语不可用**；
> 且命令**自身正常 exit 0** 时还走 `kill_inner` 的 `already_exited` 分支（`process.rs:440-460`）**不收组**。
> 三层机制与取证见 §9 与 ADR `.agents/notes/implemented/process/2026-11-16-counterexample-cmd-safety-contract.md`。

---

## 2. 规则正文（与 SKILL.md `:119` ② 逐字同源）

- **CX-SAFE-1｜后台作业必带整组回收**：`counterexample_cmd` 含后台作业（`&`、`& done`、`nohup`、`setsid`、`disown`）时，**必须**在命令**最前部（或最外层）**声明 `trap 'kill 0' EXIT INT TERM`，确保**正常退出 / 超时 / Ctrl-C 三条路径**都整组回收（`kill 0` = 向当前进程组全成员发信号）。**禁止**把 `kill $(jobs -p)` 排在链末作为唯一清理手段。
- **CX-SAFE-2｜硬超时兜底**：CX-SAFE-1 之外**或**替代——最外层包 `timeout -k 5 <N> bash -c '...'`（`<N>` 秒，且 `N*1000 < subagent_turn_timeout_ms`＝120000）；两条都没有即违规。
- **CX-SAFE-3｜禁止不可自终止的压力源**：禁止 `while :; do :; done` / `yes` / `sleep infinity` 作为 CPU 压力源；确需压力必须用**可终止形态**（有界计数）且有外层 `timeout` 兜底。
- **成因**：见 §1——链末清理是单点失败，前置语句非零退出/被信号中断/超时被杀时清理永不执行 → 孙进程变 PPID=1 孤儿。
- **CX-SAFE-4 / CX-SAFE-5（清理原语可用性）见 §9**：禁依赖 `$(jobs -p)`（dash 下恒空）、禁 `pkill -f`/`pgrep -f`/`killall`（argv 改写后永不匹配且易自杀）；两条均为无条件硬违规。

---

## 3. 违规模式表（机器判定清单）

判定域：`findings[i].counterexample_cmd`（字符串）。正则均按 **Python `re`** 语义、`re.MULTILINE`，不做 shell 解析。
**所有判据匹配前先做 R3 预处理（剥离行内注释）**，见下表之后的「判据前置处理」。

| # | 模式 ID | 判据（正则可直接落地） | 违规码 | 违规理由 |
| :- | :--- | :--- | :--- | :--- |
| 1 | `BG_OP` | `(?:^\|(?<=[\s;)\|"'`]))&(?!&)(?=[\s;)\|#]\|$)` 或 `(?<![\w-])(?:nohup\|setsid\|disown)(?![\w-])` | 前置条件 | `&` 只在**两侧都是 token 边界**时才算后台作业（**R1 收窄**）；`&&`、`2>&1`、`&>`、Rust 借用 `&str`/`&mut`/`&self` 一律不算 |
| 2 | `NO_RECLAIM` | `BG_OP` 命中 且 **不**满足 §5 判定树 D2.1/D2.2 | `CX-SAFE-1/2` | 有后台作业却无「前部 trap 整组回收」也无「外层硬超时」→ 孤儿必然 |
| 3 | `TAIL_KILL_ONLY` | `BG_OP` 命中 且 `(?:kill\s+\$\(\s*jobs\s+-p\s*\)\|kill\s+%[0-9]+)` 命中 且 无前部 trap | `CX-SAFE-1`（追加证据：链末单点清理） | 清理置于链末 = 事故原形；链中途失败即永不执行 |
| 4 | `TRAP_ORDER` | `TRAP_FULL` 命中但 `match.start()` **大于** 首个 `BG_OP` 的 `match.start()` | `CX-SAFE-1` | trap 必须在后台作业**之前**安装，否则作业已逃逸才装陷阱（窗口期孤儿） |
| 5 | `TRAP_WEAK` | `TRAP_FULL` 不满足（未同时含 `EXIT`、`INT`、`TERM`）且 `OUTER_TIMEOUT` 未命中 | `CX-SAFE-1/2` | 仅 `EXIT` 覆盖「正常退出」一条路径，超时/Ctrl-C 路径无回收且无兜底 |
| 6 | `SPIN_LOOP` | `while\s+:\s*;\s*do\s*:\s*;\s*done` 或 `while\s+true\s*;\s*do\s*:\s*;\s*done` | `CX-SAFE-3a`（**无条件**） | 纯内建无界自旋、无自终止条件；与 P0 事故形态文本同构，禁止进入审查产物（即使被 timeout 包裹） |
| 7 | `UNBOUNDED_GEN` | `(?<![\w-])yes(?![\w-])` 或 `sleep\s+infinity`，且 `OUTER_TIMEOUT` 未命中 | `CX-SAFE-3b` | 外部进程可被信号终止，但**无自终止条件**；无 `timeout` 作为唯一生命周期所有者即永续负载 |
| 8 | `FOREVER_SLEEP` | `sleep\s+(?:infinity\|inf)` 出现在任何位置且无 `OUTER_TIMEOUT` | `CX-SAFE-3b` | 同 7；`sleep infinity` 使会话永不自然结束，reviewer 会话被打断即成孤儿 |
| 9 | `JOBS_P` | `\$\(\s*jobs\s+-p\s*\)` | `CX-SAFE-4`（**无条件**） | dash 下 `$(jobs -p)` 恒空 ⇒ 清理 0 次却仍 `exit 0`（见 §9） |
| 10 | `KILL_F` | `(?<![\w-])(?:pkill\s+(?:-[0-9A-Za-z]+\s+)*-{1,2}[A-Za-z]*f\|killall)(?![\w-])\|pgrep\s+-[A-Za-z]*f[^\n\|]*\|\s*(?:xargs\s+)?kill` | `CX-SAFE-5`（**无条件**） | 按 argv 模式猜 pid：`taskset`/`exec` 改写后永不匹配且常自匹配自杀（见 §9）；**`pgrep -f` 单独使用是只读观察，不算**（**R2 收窄**） |

### 判据前置处理（R3｜先剥离行内注释再匹配）

对 `counterexample_cmd` **逐行**剥离「`#` 到行尾」，再交给上表全部判据（`BG_OP` / `KILL_F` / `SPIN_LOOP` / `UNBOUNDED_GEN` / `FOREVER_SLEEP` / `TAIL_KILL` / `JOBS_P` / `TRAP_*` / `OUTER_TIMEOUT`）：

```text
strip = 逐行 re.sub(r'(?:^|(?<=\s))#[^\n]*', '', line)      # 仅当 `#` 前是行首或空白才剥离
```

- **必要性**：`#` 之后是注释文本、物理上不执行——把注释里的 `yes`、`setsid` 当压力源属纯文本误红。
- **保守性**：`#` 前紧邻非空白字符者一律不剥离（`2>&1#`、`$#`、`${#x}`、`a=1#$x`、URL `http://h#f`）。
- **已知代价（诚实声明）**：引号内且 `#` 前是空白的文本会被误剥（如 `grep 'a # b' f`），方向是**假阴性**
  （少报警），符合「注释不执行」这一物理事实的取舍。

### R1/R2 收窄对照表（只杀纯文本误红，不放宽真违规）

**R1 必须杀掉（不构成后台作业）**

| 误红样例 | 为什么不是后台作业 |
| :--- | :--- |
| `&str`、`&mut`、`&self`、`&PathBuf`、`&RULES`、`&baseline`、`&exec_context`、`&state.input_buffer`、`&task_id`、`&backup`、`&e`、`&outcome`、`&workspace`、`&batch_path`、`&signature` | `&` 后紧跟标识符字符（非空白/`;`/`)`/`\|`/`#`/行尾）⇒ 非 token 边界；这是 Rust **借用语法**，物理上不产生子进程 |
| `skills=[]&omitted=[]`（URL query）、`A && B && C` | `&` 前紧邻 `]`/`&` ⇒ 非 token 边界；`&&` 是逻辑与运算符 |
| `grep -n 'pub fn kill(&self' …` | `(` 后紧跟 `&self`，同上（`(` 不在前置边界集内） |

**R1 必须仍命中（真后台作业）**

| 真红样例 | 命中理由 |
| :--- | :--- |
| `for i in $(seq 1 4); do sleep 30 & done`、`for i in 1 2; do cmd & done;` | `&` 两侧均为空白/`)` ⇒ token 边界 |
| `(sleep 1 &) ;`、`(npm run preview:e2e >/tmp/x.log 2>&1 &) ; sleep 6` | `&` 前空白、后 `)` ⇒ 命中 |
| `npm run dev & sleep 6`、`MP=$! ; sleep 1 &` | 前后皆空白 ⇒ 命中 |
| `cmd &#comment` | `&` 后紧跟 `#` 属后侧 token 边界（`#` 此处开注释） |

**不得算后台作业**：`cmd 2>&1 \| tee`、`cmd &> file`（`>` 不是前置边界；`&>` 后侧不是边界）。

**R2 必须杀掉（只读观察）**：`pgrep -af pony`、`pgrep -af acp_fake_agent || echo NO`、`pgrep -f x | grep kill`、`rg -n "pkill" doc.md`、`pkill pony`（无 `-f`）。
**R2 必须仍命中（清理原语）**：`pkill -f x`、`pkill -9 -f "pony"`（选项乱序也要抓）、`killall pony`、`pgrep -af x | xargs kill -9`、`pgrep -f x | kill`。

> **判据说理（防「门禁被削弱」误读）**：`&str` 是 Rust 借用语法、`pgrep -af` 是只读观察、`#` 注释不执行——
> 三者在物理上都**不可能**泄漏子进程。把它们算违规只会制造「36 红里 27 红」的噪声，而噪声门禁会被使用者
> 直接关掉（等于没做）。真违规在收窄后**一条不少**：对 **255 个** `review-*.json` 全量复跑实测
> （`dev-review-lint.py` 已落地 R1/R1+/R2/R3），`exit 3` 的文件从 **36 → 10**；剩余 10 个红文件**全部**是
> 「真实后台作业 + 无 trap/timeout」（含事故原件 `review-tool-timeout.json`），**零新增红、零纯文本性命中**。
>
> **R1+（实现层补强，本契约背书）**：单靠 token 边界无法区分 `grep -n 'WORKSPACE CONSTITUTION & RULES'`
> （引号内是**数据**）与 `… & done`（真作业）——二者都满足「前空后空」。因此实现侧按**引号状态机**把
> 「非 shell 代码」的引号内容掩成等长空格后再扫 `BG_OP`；`bash -c '…'` / `sh -c '…'` / `eval '…'` 的引号
> 内容**仍继续扫描**（那是代码，不是数据）。该补强消除了「grep 模式串里的 `setsid`」「字面量 `&`」两类误红，
> 且不放松任何真违规（12 段 fixture 全绿，见 §9）。

**辅助判定子（放行侧使用，不单独判违规）**

```text
TRAP_ANY    = trap\s+['"]?kill\s+0['"]?[\s;]+([A-Za-z ]+)
TRAP_FULL   = TRAP_ANY 且其信号集合 ⊇ {EXIT, INT, TERM}
OUTER_TIMEOUT = ^\s*timeout\s+-k\s+[1-9]\d*\s+[1-9]\d*\s+bash\s+-c\b     （必须在命令前缀/最外层）
BG_OP_FIRST = 所有 BG_OP 命中中最小的 match.start()
```

---

## 4. 放行白名单（3 类合法形态 + 1 个压力源子形态）

| 形态 | 最小可判定特征 | 说明 |
| :--- | :--- | :--- |
| **W1｜trap 整组回收** | `trap 'kill 0' EXIT INT TERM` 位于命令**最前部**，且其 `start()` < `BG_OP_FIRST` | 正常退出 + 超时 + Ctrl-C 三路径均由 `kill 0` 覆盖；允许叠加有界压力源 |
| **W2｜外层硬超时** | 命令以 `timeout -k 5 <N> bash -c '...'` 开口（`OUTER_TIMEOUT` 前缀命中），`N*1000 < 120000` | `timeout` 成为唯一生命周期所有者；`-k 5` 保证 TERM 后 5s 必 KILL |
| **W3｜无后台作业** | `BG_OP` 未命中（`&&`、`>\&1`、`&>` 不算） | 无逃逸可能，**但仍受 CX-SAFE-3 约束**（不得含 `SPIN_LOOP`） |
| **W3b｜有界压力源** | `for i in $(seq 1 <N>); do ...; done`，`1 ≤ N ≤ 64`，循环体不含 `SPIN_LOOP`/`UNBOUNDED_GEN` | 「可终止形态」的唯一许可写法 |

**W1 与 W2 的降级关系**：`TRAP_FULL` 命中即可单独满足 CX-SAFE-1；仅 `EXIT`-only trap（`TRAP_WEAK`）**必须**叠加 W2。

---

## 5. 判定树（确定性、有序，executor 照此实现）

```text
D0  counterexample_cmd 非字符串 / strip 后为空         → 归既有违规（exit 1），不进 CX-SAFE
D1  SPIN_LOOP 命中                                    → 违规 CX-SAFE-3a（无条件短路）
D2  BG_OP 命中：
    D2.1 TRAP_FULL 命中 且 trap.start() < BG_OP_FIRST  → CX-SAFE-1 满足 ✅
    D2.2 否则 OUTER_TIMEOUT 前缀命中                    → CX-SAFE-2 兜底满足 ✅
    D2.3 否则                                          → 违规 NO_RECLAIM；若 TAIL_KILL_ONLY 亦命中，追加「链末单点清理」证据
D3  (UNBOUNDED_GEN | FOREVER_SLEEP) 命中 且 无 OUTER_TIMEOUT → 违规 CX-SAFE-3b
D4  以上全过                                          → 放行 ✅
```

判定对 `severity` **一律生效**（BLOCKING 与 ADVISORY 同标准；本契约**不因 ADVISORY 豁免**——事故命令正是 ADVISORY）。

---

## 6. 逐条可回放判定样例（`dev-review-lint.py` 可直接消费）

7 段均为**合法 review JSON**（4 违规 V1–V4 + 3 合规 C1–C3；含 `schema_version:2` 与溯源四件套），`reviewed_commit` 固定为 `d21e403aab2e5b149742200e94b3f49cc9ee925b`（本仓库真实提交）。**文本样例，严禁执行。**

| 块序 `k` | 样例 | 形态 | 期望判定码 | 落地后期望退出码 | 今日实测 |
| :--- | :--- | :--- | :--- | :--- | :--- |
| 0 | V1 | 链末清理 + 无界自旋（事故原形） | `CX-SAFE-3a` + `CX-SAFE-1/2` + `TAIL_KILL_ONLY` | `3` | 假放行 `0` |
| 1 | V2 | trap 装在后台作业**之后**（TRAP_ORDER） | `CX-SAFE-3a` + `CX-SAFE-1/2` | `3` | 假放行 `0` |
| 2 | V3 | `setsid nohup` 脱离进程组 + 无 timeout | `CX-SAFE-1/2` | `3` | 假放行 `0` |
| 3 | V4 | `yes` 无 `timeout` 兜底 | `CX-SAFE-3b` + `CX-SAFE-1/2` + `TAIL_KILL_ONLY` | `3` | 假放行 `0` |
| 4 | C1 | W1：前部 `trap 'kill 0' EXIT INT TERM` + 有界压力 | 无（放行） | `0` | `0` |
| 5 | C2 | W2：外层 `timeout -k 5 60 bash -c` + 内层 trap | 无（放行） | `0` | `0` |
| 6 | C3 | W3：无后台作业（含 `2>&1`、`&&`，不得误报） | 无（放行） | `0` | `0` |

### V1 — 链末清理 + 无界自旋（事故原形最小化）

```json
{
  "verdict": "PASS",
  "audited_scopes": ["crates/pony-agent-core/src/agent/tools.rs"],
  "findings": [
    {
      "file": "crates/pony-agent-core/src/agent/tools.rs",
      "line": 3029,
      "rule": "CONCURRENCY",
      "counterexample_cmd": "cd /home/dm/pony-agent && for i in $(seq 1 $(nproc)); do (while :; do :; done) & done; for i in $(seq 1 50); do cargo test -q -p pony-agent-core --test tool_timeout_policy_test 2>&1 | grep -E 'test result'; done; kill $(jobs -p) 2>/dev/null",
      "severity": "ADVISORY"
    }
  ],
  "schema_version": 2,
  "reviewer": {"kind": "subagent", "session_id": "00000000-0000-0000-0000-00000000v1", "fresh_context": true},
  "executed_by": "00000000-0000-0000-0000-00000000v1",
  "reviewed_commit": "d21e403aab2e5b149742200e94b3f49cc9ee925b",
  "attestation": {"self_signed": false, "note": "CX-SAFE 违规样例 V1（文本，不执行）"}
}
```

### V2 — trap 位置错误（装在后台作业之后）

```json
{
  "verdict": "PASS",
  "audited_scopes": ["crates/pony-agent-core/src/agent/process.rs"],
  "findings": [
    {
      "file": "crates/pony-agent-core/src/agent/process.rs",
      "line": 433,
      "rule": "LEAK",
      "counterexample_cmd": "cd /home/dm/pony-agent && (while :; do :; done) & trap 'kill 0' EXIT INT TERM; cargo test -q -p pony-agent-core --lib; wait",
      "severity": "ADVISORY"
    }
  ],
  "schema_version": 2,
  "reviewer": {"kind": "subagent", "session_id": "00000000-0000-0000-0000-00000000v2", "fresh_context": true},
  "executed_by": "00000000-0000-0000-0000-00000000v2",
  "reviewed_commit": "d21e403aab2e5b149742200e94b3f49cc9ee925b",
  "attestation": {"self_signed": false, "note": "CX-SAFE 违规样例 V2（文本，不执行）"}
}
```

### V3 — `setsid nohup` 脱离进程组且无硬超时

```json
{
  "verdict": "PASS",
  "audited_scopes": ["crates/pony-agent-core/src/agent/subagent_runtime.rs"],
  "findings": [
    {
      "file": "crates/pony-agent-core/src/agent/subagent_runtime.rs",
      "line": 251,
      "rule": "LEAK",
      "counterexample_cmd": "cd /home/dm/pony-agent && setsid nohup sh -c 'for i in $(seq 1 50); do cargo test -q -p pony-agent-core; done' > /dev/null 2>&1 & echo started",
      "severity": "ADVISORY"
    }
  ],
  "schema_version": 2,
  "reviewer": {"kind": "subagent", "session_id": "00000000-0000-0000-0000-00000000v3", "fresh_context": true},
  "executed_by": "00000000-0000-0000-0000-00000000v3",
  "reviewed_commit": "d21e403aab2e5b149742200e94b3f49cc9ee925b",
  "attestation": {"self_signed": false, "note": "CX-SAFE 违规样例 V3（文本，不执行）"}
}
```

### V4 — `yes` 压力源无外层 `timeout` 兜底

```json
{
  "verdict": "PASS",
  "audited_scopes": ["crates/pony-agent-core/src/agent/tools.rs"],
  "findings": [
    {
      "file": "crates/pony-agent-core/src/agent/tools.rs",
      "line": 212,
      "rule": "NFR_VIOLATION",
      "counterexample_cmd": "cd /home/dm/pony-agent && yes > /dev/null & cargo test -q -p pony-agent-core --test tool_timeout_policy_test; kill %1",
      "severity": "ADVISORY"
    }
  ],
  "schema_version": 2,
  "reviewer": {"kind": "subagent", "session_id": "00000000-0000-0000-0000-00000000v4", "fresh_context": true},
  "executed_by": "00000000-0000-0000-0000-00000000v4",
  "reviewed_commit": "d21e403aab2e5b149742200e94b3f49cc9ee925b",
  "attestation": {"self_signed": false, "note": "CX-SAFE 违规样例 V4（文本，不执行）"}
}
```

### C1 — 合规 W1：命令最前部 `trap 'kill 0' EXIT INT TERM` + 有界压力源

```json
{
  "verdict": "PASS",
  "audited_scopes": ["crates/pony-agent-core/src/agent/tools.rs"],
  "findings": [
    {
      "file": "crates/pony-agent-core/src/agent/tools.rs",
      "line": 3029,
      "rule": "TIMEOUT",
      "counterexample_cmd": "cd /home/dm/pony-agent && trap 'kill 0' EXIT INT TERM; for i in $(seq 1 4); do (for j in $(seq 1 32); do :; done) & done; cargo test -q -p pony-agent-core --test tool_timeout_policy_test 2>&1 | grep -E 'test result'; wait",
      "severity": "ADVISORY"
    }
  ],
  "schema_version": 2,
  "reviewer": {"kind": "subagent", "session_id": "00000000-0000-0000-0000-00000000c1", "fresh_context": true},
  "executed_by": "00000000-0000-0000-0000-00000000c1",
  "reviewed_commit": "d21e403aab2e5b149742200e94b3f49cc9ee925b",
  "attestation": {"self_signed": false, "note": "CX-SAFE 合规样例 C1（文本，不执行）"}
}
```

### C2 — 合规 W2：外层 `timeout -k 5 <N> bash -c`（含受限 `yes`，由 timeout 独占生命周期）

```json
{
  "verdict": "PASS",
  "audited_scopes": ["crates/pony-agent-core/src/agent/process.rs"],
  "findings": [
    {
      "file": "crates/pony-agent-core/src/agent/process.rs",
      "line": 433,
      "rule": "TIMEOUT",
      "counterexample_cmd": "timeout -k 5 60 bash -c 'trap \"kill 0\" EXIT INT TERM; for i in $(seq 1 4); do (timeout -k 1 10 yes > /dev/null) & done; cd /home/dm/pony-agent; cargo test -q -p pony-agent-core --lib; wait'",
      "severity": "ADVISORY"
    }
  ],
  "schema_version": 2,
  "reviewer": {"kind": "subagent", "session_id": "00000000-0000-0000-0000-00000000c2", "fresh_context": true},
  "executed_by": "00000000-0000-0000-0000-00000000c2",
  "reviewed_commit": "d21e403aab2e5b149742200e94b3f49cc9ee925b",
  "attestation": {"self_signed": false, "note": "CX-SAFE 合规样例 C2（文本，不执行）"}
}
```

### C3 — 合规 W3：无后台作业（`2>&1`、`&&` 不得误报）

```json
{
  "verdict": "PASS",
  "audited_scopes": ["crates/pony-agent-core/src/agent/tools.rs"],
  "findings": [
    {
      "file": "crates/pony-agent-core/src/agent/tools.rs",
      "line": 3006,
      "rule": "NFR_VIOLATION",
      "counterexample_cmd": "cd /home/dm/pony-agent && sed -n '3004,3007p' crates/pony-agent-core/src/agent/tools.rs && grep -o 'DEFAULT_RUN_TIMEOUT_MS: u64 = [0-9_]*' crates/pony-agent-core/src/agent/tools.rs 2>&1 | head -1",
      "severity": "ADVISORY"
    }
  ],
  "schema_version": 2,
  "reviewer": {"kind": "subagent", "session_id": "00000000-0000-0000-0000-00000000c3", "fresh_context": true},
  "executed_by": "00000000-0000-0000-0000-00000000c3",
  "reviewed_commit": "d21e403aab2e5b149742200e94b3f49cc9ee925b",
  "attestation": {"self_signed": false, "note": "CX-SAFE 合规样例 C3（文本，不执行）"}
}
```

### 回放方法（零落盘，不写任何文件）

```bash
cd /home/dm/pony-agent
# 抽取第 <k> 个 json 代码块（0=V1 1=V2 2=V3 3=V4 4=C1 5=C2 6=C3；7=V5 8=V6 9=C4 10=V7 11=C5，见 §9）直接喂给 lint：进程替换，不落地
python3 /home/dm/.agents/skills/dev-team/scripts/dev-review-lint.py \
  <(python3 -c "import re,sys;b=re.findall(chr(96)*3+r'json\n(.*?)'+chr(96)*3,open('.dev-team/contract-counterexample-cmd-safety.md').read(),re.S);print(b[int(sys.argv[1])])" 0)
```

### 物理回执 A：今日 lint 对 7/7 样例全部 `Exit Code 0`（**假放行红基线**）

```text
idx=0 exit=0 :: [PASS] dev-review-lint 校验通过: /dev/fd/63   # V1 事故原形 → 应 3
idx=1 exit=0 :: [PASS] ...                                    # V2 → 应 3
idx=2 exit=0 :: [PASS] ...                                    # V3 → 应 3
idx=3 exit=0 :: [PASS] ...                                    # V4 → 应 3
idx=4 exit=0 :: [PASS] ...                                    # C1 合规
idx=5 exit=0 :: [PASS] ...                                    # C2 合规
idx=6 exit=0 :: [PASS] ...                                    # C3 合规
```

**结论**：现有 `dev-review-lint.py` 无法区分违规与合规（含事故原形），本契约的 §3/§5 判定清单是新增必要门禁。

### 物理回执 B：§3/§5 判据对 7 条样例的判别力（判据自证，未写入仓库代码）

```text
V1: ['CX-SAFE-3a(SPIN_LOOP)', 'CX-SAFE-1/2(NO_RECLAIM)', 'EXHIBIT:TAIL_KILL_ONLY']
V2: ['CX-SAFE-3a(SPIN_LOOP)', 'CX-SAFE-1/2(NO_RECLAIM)']
V3: ['CX-SAFE-1/2(NO_RECLAIM)']
V4: ['CX-SAFE-1/2(NO_RECLAIM)', 'EXHIBIT:TAIL_KILL_ONLY', 'CX-SAFE-3b(UNBOUNDED_GEN)']
C1: ['PASS']   C2: ['PASS']   C3: ['PASS']
```

即：4 条违规样例 100% 命中、3 条合规样例 0 误报（`2>&1`、`&&`、`kill %1` 边界均已覆盖）。

---

## 7. NFR 基准锚点（**复用既有字段，零 schema 变更**）

**结论：判定为「无结构性必要新增字段」**，三条理由：
1. `resource_release` 已能表达「子进程/进程组回收」承诺，且其元素 `subagent_processes` 精确覆盖本事故主体（孤儿由 `reviewer.kind=subagent` 会话派生的后台作业产生）；
2. `.dev-team/nfr-baseline.schema.json` 根节点 `additionalProperties: false`，**新增顶层字段会被 JSON Schema 拒绝**；
3. 该 schema 文件不在本特派员写域（属 Lead/executor），无法配套登记，故新增字段不可行且无必要。

| 复用字段 | 当前值 | 阈值 | 判定口径（机器可判） |
| :--- | :--- | :--- | :--- |
| `resource_release` | `["file_handles","network_connections","subagent_processes"]` | **残留孤儿（PPID=1）数 = 0** | `counterexample_cmd` 执行后，`pgrep -P 1` 中属于该会话进程组的进程数必须为 `0`；`resource_release` 必须仍包含 `subagent_processes`（缺失即 NFR 违规，lint 可断言成员存在） |
| `acp.spawn_reap_grace_ms` | `500` | **宽限期 ≤ 500ms** | 回收宽限口径：先关闭 stdin 等自然退出，超宽限后对**进程组** SIGKILL 并同步 `wait()`，禁止残留子进程（与 CX-SAFE-1 的 `kill 0` 语义同源：按组回收，不按 PID） |
| `subagent_turn_timeout_ms` | `120000` | **外层 `timeout` 的 `N` 必须 `N*1000 < 120000`** | CX-SAFE-2 的硬超时上界不得超过子代理单 turn 预算，否则兜底与 turn 门禁抢生命周期 |
| `subagent_max_concurrent` | `2` | 压力源并发 ≤ 2 | 反例命令内并发的后台作业数不得超过该值（`$(nproc)`=16 即违规放大器） |

**收据**（未改 `nfr-baseline.json`，仅验证基线在门禁下为绿）：

```text
$ python3 /home/dm/.agents/skills/dev-team/scripts/dev-nfr-lint.py /home/dm/pony-agent/.dev-team/nfr-baseline.json
[PASS] dev-nfr-lint 校验通过: /home/dm/pony-agent/.dev-team/nfr-baseline.json
Exit Code: 0
```

---

## 8. 给 executor 的实现要点（判据 + 退出码，不含实现代码）

1. **插入位置**：`dev-review-lint.py` 的 `lint_review()` 内，紧跟现有「反例物理收据硬检」（`:116-118` 非空校验）之后，逐 `finding` 调用 `lint_counterexample_safety(cmd) -> list[str]`；返回错误追加到既有 `errors`。
2. **判据正则**（§3/§4 子判定；`re.MULTILINE`，只对字符串做匹配，**不得 eval/执行命令**）：
   - `BG_OP = r'(?<![&>])&(?!&)(?!>)|(?<![\w-])(?:nohup|setsid|disown)(?![\w-])'`
   - `TRAP_ANY = r"trap\s+['\"]?kill\s+0['\"]?[\s;]+([A-Za-z ]+)"`，信号集按 `split()` 归一化后比对 `{"EXIT","INT","TERM"}`（大小写不敏感）
   - `OUTER_TIMEOUT = r'^\s*timeout\s+-k\s+[1-9]\d*\s+([1-9]\d*)\s+bash\s+-c\b'`（捕获 `N` 秒，校验 `N < 120`）
   - `SPIN_LOOP = r'while\s+:\s*;\s*do\s*:\s*;\s*done|while\s+true\s*;\s*do\s*:\s*;\s*done'`
   - `UNBOUNDED = r'(?<![\w-])yes(?![\w-])|sleep\s+(?:infinity|inf)'`
   - `TAIL_KILL = r'kill\s+\$\(\s*jobs\s+-p\s*\)|kill\s+-?\S*\s*%[0-9]+'`
3. **判定顺序**：严格按 §5 判定树（D1 无条件短路 → D2 后台作业三分支 → D3 压力源 → D4 放行），保证同一命令判定**确定性**。
4. **退出码语义**（README/`--help` 内同步声明，**违规必须非零**）：

   | 退出码 | 语义 | 触发 |
   | :--- | :--- | :--- |
   | `0` | 全部通过 | schema + 闭合 + 溯源 + CX-SAFE 全绿 |
   | `1` | 既有结构性违规（**语义保持现状不变**） | 必填缺失 / 范围未闭合 / 溯源不合规 / `counterexample_cmd` 为空 / 文件不存在 |
   | `3` | **CX-SAFE 安全契约违规**（新增） | §3 任一违规码命中 |
   | `4` | CX-SAFE 判定器内部错误（fail-closed） | 字段非字符串、正则编译/执行异常 |
   | `2` | 保留：CLI 用法错误 | 参数非法（不得与 3/4 混用） |

   **优先级（同时命中时）**：`4 > 3 > 1 > 0`——安全判定失败必须压过结构性违规，杜绝「先修 schema 再蒙过安全项」；`stderr` 需一次性列出全部错误，每条前缀 `[CX-SAFE:<模式ID>]`。
5. **禁止豁免开关**：不得提供 `--no-safety` / `--skip-counterexample-safety` / 环境变量旁路；CX-SAFE 无 `non-machine` 例外（与 SKILL.md「无法机械化的判定项显式标注 `non-machine`」不冲突——本项已机械化）。
6. **自检门禁（负例先决）**：实现须内置 `--self-test`，以 §6 的 7 段 + §9 的 5 段（合计 **12 段**，全部为合法 JSON）为 fixture，断言 `V1..V6 + C5 → 3`、`C1..C4 + V7 → 0`；`--self-test` 通过（Exit Code 0）方可提交流水。
7. **回归要求**：对 `.dev-team/review-*.json` 历史产物只读复跑，输出「新增红」清单交 Lead 裁决（历史证据不回改；`review-tool-timeout.json` 预期由 `0` 转 `3`）。

---

## 9. CX-SAFE-4/5：清理原语取证补强（工单 B205）

> **背景**：2026-11-13 事故的**执行者不是审查产物**，而是执行者 teammate `b137-flaky`（B137a）自建的压测命令
> （子会话 `subagent-2cb8e2a0-a604-4959-89c1-5511d23f23e2`，父会话 `pa_01a1d41d-…`，单 turn 17:29:03→17:44:01）。
> 批次 A：`seq 1 32` + `taskset -c 0 bash -c 'while :; do :; done' &`（32 个挤 1 核 → 100%/32 ≈ **3.1%**/个，与实测吻合）；
> 批次 B：`seq 1 8` + `taskset -c 12-15`（2 个/核 ≈ **50%**，与实测 49.5% 吻合）。
> `.dev-team/` 在 17:20–17:45 **零写入**（`find .dev-team -newermt '2026-11-13 16:00' ! -newermt '2026-11-13 19:00'` 为空，
> 最后写入 13:17:45），`review-tool-timeout.json` mtime 为 11-04 00:56 ⇒ 历史 review json 是**同族写法的先例、
> 本次未被执行**。但本契约仍按「反例命令」治理：同族写法在 255 个产物里仍有 1007 条，随时可被复制执行。
> 本节补齐「清理原语本身不可用」这一层。

### CX-SAFE-4｜禁依赖 `$(jobs -p)` 做清理（**硬违规**）

- **失效机制**：本机 `/bin/sh` → `dash`（`ls -l /bin/sh`），而受管命令一律由
  `sh -lc "cd '<cwd>' && <command>"` 执行（`crates/pony-agent-core/src/agent/tools/run.rs:1077-1081`）。
  **dash 的 `$(jobs -p)` 恒为空** ⇒ `for p in $(jobs -p); do kill $p; done` 的循环体**物理上执行 0 次**，
  脚本还会照常 `exit 0` 打印 "load-round done"，看起来"干净结束"。
- **判据（无条件硬违规）**：`JOBS_P = r'\$\(\s*jobs\s+-p\s*\)'` 命中即违规；**不因**同命令另有
  `trap` / `timeout` 而放行（原语本身不可用，别处写对也不能留）。
- **证据锚（实测原文，可回放）**：

  ```bash
  # ① dash（= 本机 /bin/sh，即受管命令的真实解释器）
  /bin/sh -lc 'sleep 2 & sleep 2 & HOGS=$(jobs -p); echo "HOGS=[$HOGS]"'   # → HOGS=[]
  # ② bash 对照
  bash    -lc 'sleep 2 & sleep 2 & HOGS=$(jobs -p); echo "HOGS=[$HOGS]"'   # → HOGS=[1802233 1802234]
  ```

  本节落盘时复跑收据：`HOGS=[]` / `HOGS=[1831156 1831157]`（`sleep 2` 自终止，无 kill）。
- **合规替代**：`trap 'kill 0' EXIT INT TERM`（dash 支持；向**本进程组**整组发信号，不依赖作业表）。

### CX-SAFE-5｜禁 `pkill -f` / `pgrep -f` / `killall` 做清理（**硬违规**）

- **失效机制（两重叠加）**：
  1. `taskset` / `exec` 会**改写 argv**：`taskset -c 0 bash -c '...'` 之后目标进程 argv 已是
     `bash -c ...`，故 `pkill -f "taskset -c 0 bash -c"` **永不匹配**真目标；
  2. 更糟：该模式字符串出现在**承载它的 wrapper** argv 里
     （`sh -lc "…pkill -f \"taskset -c 0 bash -c\"…"`）⇒ `pkill` **先杀掉自己**（批次 A 实测 `exit -1`），
     而孙进程一个没死。事后补救改用 argv 精确相等匹配
     （`['taskset','-c','0','bash','-c',…]` vs 真实 `['bash','-c','while',':;','do',':;','done']`）
     ⇒ 输出 **`killed hogs 0`**。
- **判据（无条件硬违规，**R2 收窄后**）**：`KILL_F = r'(?<![\w-])(?:pkill\s+(?:-[0-9A-Za-z]+\s+)*-{1,2}[A-Za-z]*f|killall)(?![\w-])|pgrep\s+-[A-Za-z]*f[^\n|]*\|\s*(?:xargs\s+)?kill'`
  —— `pkill -f`（含 `-9 -f` 选项乱序）与 `killall` 直接违规；**`pgrep -f`/`pgrep -af` 单独使用是只读观察，
  不算清理原语**，仅当被管道喂给 `kill`/`xargs kill` 时才判违规（否则 `review-join.json` 的 `pgrep -af`
  这类观察命令被误红）。命中即违规，同 CX-SAFE-4，不因另有 `trap` / `timeout` 而放行。
- **证据锚（实测原文，可回放；**禁止**为取证再跑 `pkill`）**：

  ```bash
  # argv 改写实测（不杀任何进程；sleep 自终止）：taskset 执行后 argv 已无 taskset 前缀
  bash -c 'taskset -c 0 sleep 2 & P=$!; sleep 0.3; tr "\0" " " < /proc/$P/cmdline; wait'
  # → sleep 2          （而非 "taskset -c 0 sleep 2"）
  ```

  事故侧收据：批次 A 收尾 `pkill -f "taskset -c 0 bash -c"` → **0 命中 + 自匹配自杀**（`exit -1`）；
  事后精确匹配补救 → **`killed hogs 0`**。
- **合规替代**：清理必须持有**真实 pid**（spawn 时用 `$!` 收集）或 `kill 0` 整组；禁按命令行模式猜。

### 判定树插入点与实现要求

- 插入 **D1.5**（§5 的 D1 之后、D2 之前）：`JOBS_P` 命中 → `CX-SAFE-4`；`KILL_F` 命中 → `CX-SAFE-5`。
  置于 D2 **之前**是因为这两条不因 trap/timeout 而正当化——先判「清理原语是否可用」，再判「兜底是否在场」。
  **D1.5 使用的是 §3 表第 9/10 行的收窄版判据**（`KILL_F` 已排除裸 `pgrep`），且全部判据在匹配前先做
  §3「判据前置处理」的 `#` 剥离（R3）——否则注释里的 `yes`/`setsid` 与 Rust 借用 `&str` 会制造纯文本误红。
- 退出码沿用 §8：违规 **`exit 3`**；判定器内部错误 **`exit 4`**；优先级 `4 > 3 > 1 > 0` 不变。
- `--self-test` fixture 共 **12 段**（§6 的 7 段 + 本节 5 段）：

  | k | 样例 | 期望 | 用途 |
  | :- | :--- | :--- | :--- |
  | 7 | **V5** `for i in $(seq 1 4); do sleep 30 & done; HOGS=$(jobs -p); for p in $HOGS; do kill $p; done` | `3`（`CX-SAFE-4`） | 清理原语不可用 |
  | 8 | **V6** `for i in $(seq 1 4); do taskset -c 0 sleep 30 & done; pkill -f 'taskset -c 0 sleep 30'` | `3`（`CX-SAFE-5`） | argv 模式猜 pid |
  | 9 | **C4** `trap 'kill 0' EXIT INT TERM; for i in $(seq 1 4); do sleep 30 & done; echo done` | `0` | W1 合规 |
  | 10 | **V7** 借用 `&self`/`&mut` + `pgrep -af` 观察 + `#` 注释里的 `yes` | **`0`（不得误红，golden case）** | R1+R2+R3 收窄回归 |
  | 11 | **C5** `trap 'kill 0' EXIT INT TERM; … pkill -f pony-agent-core` | **`3`（`CX-SAFE-5`）** | 收窄后真违规仍被抓住 |

  期望总账：`V1..V6 + C5 → 3`、`C1..C4 + V7 → 0`。

  **实测收据（12/12 与期望一致，落地实现 `dev-review-lint.py` 逐段回放）**：

  ```text
  k=0(V1) 3  SPIN_LOOP + CX-SAFE-4 + NO_RECLAIM + TAIL_KILL_ONLY
  k=1(V2) 3  SPIN_LOOP + NO_RECLAIM + TRAP_ORDER
  k=2(V3) 3  NO_RECLAIM
  k=3(V4) 3  NO_RECLAIM + TAIL_KILL_ONLY + UNBOUNDED_GEN
  k=4(C1) 0    k=5(C2) 0    k=6(C3) 0
  k=7(V5) 3  CX-SAFE-4 + NO_RECLAIM
  k=8(V6) 3  CX-SAFE-5 + NO_RECLAIM
  k=9(C4) 0
  k=10(V7) 0  ← 收窄 golden case：&self/&mut 借用 + pgrep -af 观察 + # 注释里的 yes 全部放行
  k=11(C5) 3  CX-SAFE-5（trap 在场也不豁免 CX-SAFE-5）
  ```

### V5 / V6 / C4 — §9 三段 fixture（JSON 形式，可直接回放）

```json
{
  "verdict": "PASS",
  "audited_scopes": ["crates/pony-agent-core/src/agent/process.rs"],
  "findings": [
    {
      "file": "crates/pony-agent-core/src/agent/process.rs",
      "line": 419,
      "rule": "LEAK",
      "counterexample_cmd": "cd /home/dm/pony-agent && for i in $(seq 1 4); do sleep 30 & done; HOGS=$(jobs -p); for p in $HOGS; do kill $p; done; echo done",
      "severity": "ADVISORY"
    }
  ],
  "schema_version": 2,
  "reviewer": {"kind": "subagent", "session_id": "00000000-0000-0000-0000-00000000v5", "fresh_context": true},
  "executed_by": "00000000-0000-0000-0000-00000000v5",
  "reviewed_commit": "d21e403aab2e5b149742200e94b3f49cc9ee925b",
  "attestation": {"self_signed": false, "note": "CX-SAFE 违规样例 V5：$(jobs -p) 清理（dash 恒空），文本不执行"}
}
```

```json
{
  "verdict": "PASS",
  "audited_scopes": ["crates/pony-agent-core/src/agent/process.rs"],
  "findings": [
    {
      "file": "crates/pony-agent-core/src/agent/process.rs",
      "line": 538,
      "rule": "LEAK",
      "counterexample_cmd": "cd /home/dm/pony-agent && for i in $(seq 1 4); do taskset -c 0 sleep 30 & done; pkill -f 'taskset -c 0 sleep 30'; echo done",
      "severity": "ADVISORY"
    }
  ],
  "schema_version": 2,
  "reviewer": {"kind": "subagent", "session_id": "00000000-0000-0000-0000-00000000v6", "fresh_context": true},
  "executed_by": "00000000-0000-0000-0000-00000000v6",
  "reviewed_commit": "d21e403aab2e5b149742200e94b3f49cc9ee925b",
  "attestation": {"self_signed": false, "note": "CX-SAFE 违规样例 V6：pkill -f 按 argv 模式猜 pid，文本不执行"}
}
```

```json
{
  "verdict": "PASS",
  "audited_scopes": ["crates/pony-agent-core/src/agent/process.rs"],
  "findings": [
    {
      "file": "crates/pony-agent-core/src/agent/process.rs",
      "line": 466,
      "rule": "TIMEOUT",
      "counterexample_cmd": "cd /home/dm/pony-agent && trap 'kill 0' EXIT INT TERM; for i in $(seq 1 4); do sleep 30 & done; echo done",
      "severity": "ADVISORY"
    }
  ],
  "schema_version": 2,
  "reviewer": {"kind": "subagent", "session_id": "00000000-0000-0000-0000-00000000c4", "fresh_context": true},
  "executed_by": "00000000-0000-0000-0000-00000000c4",
  "reviewed_commit": "d21e403aab2e5b149742200e94b3f49cc9ee925b",
  "attestation": {"self_signed": false, "note": "CX-SAFE 合规样例 C4：W1 前部 trap 整组回收 + 有界压力源，文本不执行"}
}
```

### V7 — 不得误红（R1+R2+R3 收窄 golden case，期望 `exit 0`）

```json
{
  "verdict": "PASS",
  "audited_scopes": ["crates/pony-agent-core/src/agent/process.rs"],
  "findings": [
    {
      "file": "crates/pony-agent-core/src/agent/process.rs",
      "line": 413,
      "rule": "LEAK",
      "counterexample_cmd": "cd /home/dm/pony-agent && grep -n 'pub fn kill(&self' crates/pony-agent-core/src/agent/process.rs | head -3; grep -n 'fn drop(&mut self)' crates/pony-agent-core/src/agent/process.rs; pgrep -af pony-agent-core; rg -n '&RULES|&baseline' .dev-team/contract-counterexample-cmd-safety.md | head -2  # yes 只出现在本注释里，不是压力源",
      "severity": "ADVISORY"
    }
  ],
  "schema_version": 2,
  "reviewer": {"kind": "subagent", "session_id": "00000000-0000-0000-0000-00000000v7", "fresh_context": true},
  "executed_by": "00000000-0000-0000-0000-00000000v7",
  "reviewed_commit": "d21e403aab2e5b149742200e94b3f49cc9ee925b",
  "attestation": {"self_signed": false, "note": "CX-SAFE 合规样例 V7：借用 &self/&mut、pgrep -af 只读观察、# 注释里的 yes（文本，不执行）"}
}
```

### C5 — 收窄后真违规仍被抓（`trap` 在场但用 `pkill -f` 清理，期望 `exit 3`）

```json
{
  "verdict": "PASS",
  "audited_scopes": ["crates/pony-agent-core/src/agent/process.rs"],
  "findings": [
    {
      "file": "crates/pony-agent-core/src/agent/process.rs",
      "line": 1013,
      "rule": "LEAK",
      "counterexample_cmd": "cd /home/dm/pony-agent && trap 'kill 0' EXIT INT TERM; pkill -f pony-agent-core; cargo test -q -p pony-agent-core --lib",
      "severity": "ADVISORY"
    }
  ],
  "schema_version": 2,
  "reviewer": {"kind": "subagent", "session_id": "00000000-0000-0000-0000-00000000c5", "fresh_context": true},
  "executed_by": "00000000-0000-0000-0000-00000000c5",
  "reviewed_commit": "d21e403aab2e5b149742200e94b3f49cc9ee925b",
  "attestation": {"self_signed": false, "note": "CX-SAFE 违规样例 C5：trap 在场但清理走 pkill -f（CX-SAFE-5 无条件；文本，不执行）"}
}
```

### 收窄后的**残留**风险（诚实清单，按落地实现的全量复跑校准）

**实测口径**：255 个 `review-*.json` 逐个跑 `dev-review-lint.py`，`exit 3` 者 **10 个**，逐条人工核对后
**全部为真违规**（真实后台作业 + 无前部 trap / 无外层 timeout）：

| `exit 3` 文件 | 命中码 | 判定 |
| :--- | :--- | :--- |
| `review-tool-timeout.json`（事故原件） | `SPIN_LOOP` + `CX-SAFE-4` + `NO_RECLAIM` + `TAIL_KILL_ONLY` | 真违规（事故原形） |
| `review-join-b.json`、`review-l3a-cli-update.json`、`review-l3a-cli-update-raw.json`、`review-l3p.json`、`review-l3p-round1-invalid.json`、`review-plan-wave61-a.json`、`review-stage0-l3a.json`、`review-stage0-l3b.json`、`review-wave52-v3.json` | `NO_RECLAIM` | 真违规：`(cmd &) ; sleep N ; … ; wait` / `& done; wait` 形态——**持真实 pid 或 `wait` 都不覆盖「被打断 / 被超时杀」路径**，符合 CX-SAFE-1，应靠 W1（前部 trap）或 W2（外层 timeout）修复 |

**残留的边界（不是本轮误红，是下一轮判据的已知盲区）**：

| # | 残留形态 | 方向 | 处置建议 |
| :- | :--- | :--- | :--- |
| 1 | 跨多个管道的 `pgrep -f x \| head -1 \| xargs kill`：R2 只在**同一语句段**内认可「喂给 kill」，跨管道组合可能漏判 | 假阴性（少报警） | 后续若出现真实泄漏案例再扩；宁可漏判弱形态，也不恢复裸 `pgrep` 误红 |
| 2 | heredoc / 反引号 / 嵌套 `$(…)` 内部的 `&` 与 `#`：引号状态机只处理单/双引号，反引号与 heredoc 体未建模 | 双向（可能掩掉或误留） | 需要真正的 shell 词法器；当前取证语料（1007 条）无命中，登记为已知盲区 |
| 3 | 引号内且 `#` 前是空白的文本被当注释剥掉（`grep 'a # b' f`） | 假阴性 | 与 R3「注释不执行」的取舍一致；不修 |
| 4 | `counterexample_cmd` 被写成整段散文（字段语义污染，如 `review-join-a2.json`）——R1+ 掩码后已不再误红，但字段本身仍不是可执行命令 | 语义债 | 应由 review schema 侧约束「必须像命令」，本契约不越界 |

> **已用机器证据消除的误红**（本轮降噪的实证收益，全部来自 R1/R1+/R2/R3）：
> `&str`/`&mut`/`&self`/`&RULES`/`&baseline` 等借用、`grep '…setsid…'` 检索词、`pgrep -af` 只读观察、
> `#` 注释里的 `yes` —— 36 → 10 的差值即此类纯文本性命中。**没有一条真违规因此在收窄中丢失**
> （12 段 fixture：`V1..V6 + C5 → exit 3`、`C1..C4 + V7 → exit 0`，与实现实测一致）。


---

## 10. 变更流水（单行）

- 2026-11-16 L2-P：新建 CX-SAFE 契约（SKILL.md `:111`/`:119` 就地同源）；复用 `resource_release`/`acp.spawn_reap_grace_ms`/`subagent_turn_timeout_ms`/`subagent_max_concurrent`，`nfr-baseline.json` 零变更（`dev-nfr-lint.py` Exit 0）。
- 2026-11-16 L2-P：B205 精修补 §9（CX-SAFE-4 禁 `$(jobs -p)` / CX-SAFE-5 禁 `pkill -f`·`pgrep -f`·`killall`），含 dash/argv 改写实测证据锚与 D1.5 插入点；SKILL.md `:119` 同步 2 行。
- 2026-11-16 L2-P：B205 门禁降噪三收窄（Lead 裁决）——R1 `BG_OP` 改 token 边界判据、R2 `KILL_F` 去裸 `pgrep`、R3 判据前剥离 `#` 注释；§3 增第 9/10 行与 R1/R2 对照表、§9 增 V5/V6/C4 的 JSON 形式与 V7/C5 及残留误红清单（fixture 10→12 段）。
