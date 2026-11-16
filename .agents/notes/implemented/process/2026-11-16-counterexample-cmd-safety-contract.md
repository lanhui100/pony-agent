# Agent Note: 反例命令安全契约（CX-SAFE 1–5）与 2026-11-13 孤儿进程组事故的三重失效

Status: implemented

工单：B205（`.dev-team/review-*` 历史存量只读；契约与门禁增量）

## Problem

2026-11-13 的一批后台死循环在共用开发机上存活 **2 天 16 小时**，40 个进程全部 `PPID=1`，把整机拖进
thrashing：load average 冲到 **204**、swap 31G **100%**、内存 PSI some **88%**、磁盘 `%util` 100%；
2026-11-16 手工 TERM 清理后 load 204 → **22.67**、swap 31G → **5.8G**。

### 真凶不是审查产物，而是执行者自建的压测命令

**E1 独立侦察已推翻「事故源自 `.dev-team/review-*.json` 的 `counterexample_cmd`」这一初判**：

- 执行者是 teammate `b137-flaky`（B137a 执行者）自建的压力命令，会话
  `subagent-2cb8e2a0-a604-4959-89c1-5511d23f23e2`（父会话 `pa_01a1d41d-…` 派生，单 turn
  17:29:03 → 17:44:01）。
  - **批次 A**：`seq 1 32` + `taskset -c 0 bash -c 'while :; do :; done' &` —— 32 个进程全挤 1 核，
    `100% / 32 ≈ 3.1%` 与实测「32 个各占 ~3.1% CPU」吻合；32 是 2×nproc 的巧合，**不是** `$(nproc)` 展开。
  - **批次 B**：`seq 1 8` + `taskset -c 12-15`（2 个/核）—— `≈50%` 与实测「8 个各占 ~49.5%」吻合。
  - 两批合计 40 个，`cgroup=/user.slice/user-1000.slice/session-9.scope`、`cwd=/home/dm/pony-agent`。
- **`.dev-team/` 在 17:20–17:45 零写入**：`find .dev-team -newermt '2026-11-13 16:00' ! -newermt
  '2026-11-13 19:00'` 输出为空，最后一次写入是 13:17:45（`contracts/system-prompt-v2.md`）；
  `.dev-team/review-tool-timeout.json` mtime 为 **11-04 00:56**，其 `reviewed_commit fecc512f` 为
  2026-11-04 00:24，reviewer 会话 `078b406f` 最后写入 11-04 00:54。
  ⇒ **历史 review json 是同族写法的先例，本次未被执行**。这个区分必须保留：它决定了处置不能是
  「清理审查产物」，而是「约束人写命令时的原语选择 + 让门禁与运行时真的收口」。
- 但同族写法并未消失：255 个 `review-*.json` 里仍有 **1007 条** `counterexample_cmd`，随时可被复制执行。

### 三重失效：三层各自都能独立造成事故

**(i) `$(jobs -p)` 在 dash 下恒为空 —— 清理原语不可用。**
本机 `/bin/sh` → `dash`（`ls -l /bin/sh`），而受管命令一律由 `sh -lc "cd '<cwd>' && <command>"`
执行（`crates/pony-agent-core/src/agent/tools/run.rs:1077-1081`）。实测：

```text
/bin/sh -lc 'sleep 2 & sleep 2 & HOGS=$(jobs -p); echo "HOGS=[$HOGS]"'   → HOGS=[]
bash    -lc 'sleep 2 & sleep 2 & HOGS=$(jobs -p); echo "HOGS=[$HOGS]"'   → HOGS=[1831156 1831157]
```

事故批次 B 的收尾是 `HOGS=$(jobs -p); … for p in $HOGS; do kill $p; done` ⇒ **循环体执行 0 次**，
脚本随即正常 `exit 0` 打印 "load-round done"，现场看起来"干净结束"。

**(ii) `pkill -f` 永不匹配真目标，还先杀掉自己。**
`taskset`/`exec` 会改写 argv：`taskset -c 0 bash -c '…'` 执行后目标进程 argv 已是 `bash -c …`。实测：

```text
bash -c 'taskset -c 0 sleep 2 & P=$!; sleep 0.3; tr "\0" " " < /proc/$P/cmdline; wait'   → sleep 2
```

故批次 A 的收尾 `pkill -f "taskset -c 0 bash -c"` **0 命中**；更糟的是该模式字符串出现在承载它的
wrapper `sh -lc "…pkill -f \"taskset -c 0 bash -c\"…"` 的 argv 里 ⇒ `pkill` **自匹配自杀**
（`exit -1`），孙进程一个没死。事后补救改用 argv 精确相等匹配
（`['taskset','-c','0','bash','-c',…]` vs 真实 `['bash','-c','while',':;','do',':;','done']`）
⇒ 实测输出 **`killed hogs 0`**。

**(iii) 即使清理语句写对也不够 —— 命令自己 `exit 0`，运行时不收组。**
这是**代码层真实缺口**（不是潜在项）：`ProcessManager::kill_inner` 的 `already_exited` 分支
（`crates/pony-agent-core/src/agent/process.rs:440-460`）只做三件事——`#[cfg(windows)]` 的
`entry.job.terminate(1)`（`:448`）、关 stdin（`:452-453`）、删条目（`:455-459`）——然后
`return Ok(())`（`:460`）。**POSIX 侧没有 `killpg`**。而 `killpg` 只在 `terminate`（`:1013`，killpg 在
`:1040`）里发生，`terminate` 只被显式 `kill`/`cancel`/`kill_after`/`shutdown` 调用。这两条压力命令
**自己正常退出**，于是走 `already_exited` 分支 ⇒ 已 spawn 的 bash 孙进程（`process_group(0)` 建的组）
全组无人收，全部被 init 收养为 PPID=1。

**门禁侧另有一层零判别力**（不是本次触发者，但是同族风险的放大器）：`scripts/dev-review-lint.py` 对
`counterexample_cmd` 只有必填（`:103`）与非空（`:116-118`，唯一判据 `len(cmd.strip()) < 5`）两项；
事故族命令今日复跑仍 `Exit Code 0`（契约 §6 的 7 段样例回放全部 `exit 0`，含事故形态）。

**存量面（可复跑口径）**：`.dev-team/review-*.json` **255** 个文件，含 `counterexample_cmd` 的
findings **1007** 条；按本契约 §3 判据粗口径命中 **34 文件 / 53 条**（排除 `wait` 收尾形态 32/50）。
审计侧独立口径为 **13 条**——差异源于判据实现未统一，以 `dev-review-lint.py` 落地后同一次全量复跑为
唯一计数口径（不为对齐数字而改判据）。另有 **1 个文件 JSON 破损**：
`.dev-team/review-final.json:62` `Invalid \escape`（已知破损，不在本决策范围）。

## Decision

事故是「**人写压力命令时用错清理原语**」+「**门禁零内容判别力**」+「**运行时 `already_exited` 分支不收组**」
三层叠加；**三层各治一层，缺一层就复发**。

1. **生成侧——CX-SAFE 契约 1–5**，就地写入 `.agents/skills/dev-team/SKILL.md`（`counterexample_cmd`
   schema 行 + 物理反例门禁 ②，不新增章节），判定清单落盘
   `.dev-team/contract-counterexample-cmd-safety.md`（违规模式表 / 白名单 / 判定树 / 可回放样例 / 证据锚）：
   - **CX-SAFE-1**：含后台作业（`&`、`& done`、`nohup`、`setsid`、`disown`）时，**必须**在命令最前部
     （或最外层）声明 `trap 'kill 0' EXIT INT TERM`，覆盖正常退出 / 超时 / Ctrl-C 三条路径；禁止把
     `kill $(jobs -p)` 排在链末作为唯一清理手段。
   - **CX-SAFE-2**：CX-SAFE-1 之外或替代——最外层 `timeout -k 5 <N> bash -c '...'`，`N*1000 < 120000`
     （`subagent_turn_timeout_ms`）。
   - **CX-SAFE-3**：禁 `while :; do :; done` / `yes` / `sleep infinity` 作压力源；确需压力须有界 + 外层
     `timeout`。其中 `while :; do :; done` **无条件违规**（判定树 D1 短路：`timeout` 只杀外壳，杀不掉已
     脱离组的孙进程）。
   - **CX-SAFE-4（B205）**：禁依赖 `$(jobs -p)` 做清理——**无条件硬违规**（dash 下恒空，见 Problem (i)）；
     清理必须 `trap 'kill 0' EXIT INT TERM`（dash 支持、按进程组整组回收）。
   - **CX-SAFE-5（B205）**：禁 `pkill -f` / `pgrep -f` / `killall` 做清理——**无条件硬违规**
     （argv 被 `taskset`/`exec` 改写后永不匹配，且常自匹配 wrapper 自杀，见 Problem (ii)）；清理必须
     持真实 pid（`$!`）或 `kill 0` 整组。
   - 判定对 `severity` 一律生效（事故命令族多为 ADVISORY，ADVISORY 不豁免）。

2. **门禁侧——违规必须非零退出。** `dev-review-lint.py` 新增 CX-SAFE 校验，退出码 `0` 通过 /
   `1` 既有结构性违规（语义不变）/ **`3` CX-SAFE 违规** / `4` 判定器内部错误（fail-closed）/
   `2` 保留 CLI 用法错误；同时命中优先级 `4 > 3 > 1 > 0`。CX-SAFE-4/5 在判定树 **D1.5**（D2 之前）短路，
   不因另有 trap/timeout 而正当化。实现由 executor（task-3 组 B）交付，禁 `--no-safety` 类旁路。
   *截至本条落盘，`dev-review-lint.py` 尚无 CX-SAFE 判据（复跑事故文件仍 `exit 0`），故本项状态为
   「规格已冻结、实现待落地」——不主张门禁已上线。*

3. **运行时侧——`already_exited` 分支补 `killpg`。** 缺口与验收口径见
   `proposed/architecture/2026-11-16-process-group-reap-gap-remediation.md`；executor 已认领（task-3 组 A），
   红相测试由 test-agent 冻结（`crates/pony-agent-core/tests/process_group_reap_test.rs`）。
   同条登记其余三处：无 `impl Drop for ProcessManager`、unix pipe-missing 分支（`process.rs:815-845`，
   当前不可达）、`agent/updater.rs:1200` 裸 `Command::new` 无 `process_group(0)`；以及模块文档失真
   （`process.rs:26` 仍写 "non-Windows behavior is direct-child lifecycle only"，而 B009 已加
   `process_group(0)`（`:689`）+ `terminate` 内 `killpg`（`:1040`））。
   **已收口、不属缺口**：`kill`(`:413`)、`kill_if_exists`(`:419`)、`cancel`(`:466`)、`kill_after`(`:475`)、
   `shutdown`(`:538`) 五条公开入口汇聚到 `terminate`(`:1013`) → `killpg`(`:1040`) + 有界 reap(`:1043-1056`)；
   `setsid`/双 fork 逃逸是 `sandbox.rs:11`/`:40` 自述的已知局限，不是回归。

4. **不重写历史证据。** 255 个 `review-*.json` 只读：它们是审核证据（`attestation` 自签语义 +
   `reviewed_commit` 溯源链），改写即篡改；且旧产物不会再被执行——重写只毁证据、不降风险。风险集合只在
   **新产物**与**人写命令的原语选择**，故修在生成侧（SKILL.md 契约）+ 门禁侧（lint `exit 3`）。历史 34 个
   含危险命令的文件处置为「只读留档 + 门禁侧一次性复跑出清单交 Lead 裁决」。

5. **不新增 NFR 字段。** 复用既有锚点：`resource_release ⊇ {subagent_processes}`（阈值：残留 PPID=1
   孤儿数 = 0）、`acp.spawn_reap_grace_ms`=500（按**进程组** SIGKILL + 同步 `wait()`）、
   `subagent_turn_timeout_ms`=120000、`subagent_max_concurrent`=2。`.dev-team/nfr-baseline.json` 零变更
   （`dev-nfr-lint.py` `Exit Code 0`）；根因是 `.dev-team/nfr-baseline.schema.json` 根节点
   `additionalProperties: false`，新增字段会被拒。

**落地状态**：CX-SAFE 1–5 契约文本与 SKILL.md 条款已生效并由 Lead 冻结（SKILL.md 533→539 行，就地 3 处）；
契约文档含 §6 的 7 段 lint fixture + §9 追加的 V5/V6/C4（共 10 段）；`exit 3` 实现、`already_exited` 补
`killpg`、红相测试均未落地（分别在 executor task-3 组 B / 组 A / test-agent task-2）。

## Alternatives considered

- **(a) 直接批量改写历史 review json 的 `counterexample_cmd`**：否决。① 证据不可篡改——这些 JSON 的价值
  在 `attestation` + `reviewed_commit` 构成的独立审查溯源链，改写后无法区分「当时真跑了」与「事后被清理过」；
  ② 收益为零——真凶已查明是执行者自建命令（见 Problem），历史文件从未被执行，改文本不降低任何风险；
  ③ 成本不可控（1007 条无法判定哪些属真实取证）。正确处置是「生成侧立契约 + 门禁侧拦新」。
- **(b) 只在 SKILL.md 写软建议、不加机械门禁**：否决。SKILL.md 是提示词，模型可以「读懂了但不遵守」；
  现状已实证唯一物理反例门禁对事故族命令 `exit 0`。这违反常载命约第 2 条「凡机械可查的承诺，配一条非零
  退出的命令」——本项完全可机械判定（正则判定树 + 10 段 fixture，4 违规 100% 命中、3 合规 0 误报）。
- **(c) 用 cgroup 或外部 supervisor 隔离反例命令**：本轮否决，记为未来选项。收益真实（内核级整组回收），
  但需 cgroup v2 写权限 / 系统级 supervisor，Windows 须另一套 Job Object 实现，CI 与开发机权限模型不同；
  更关键是它**只兜底不治因**——压力命令还会被写出来，在无 cgroup 兜底的机器上照样复现。先做零依赖的
  文本契约 + lint，是把同等收益拿到手的更短路径。
- **(d) 把受管命令的 shell 从 `sh -lc` 换成 `bash -lc`，让 `$(jobs -p)` 可用**：否决。这是把正确性押在
  shell 方言上：① 换 shell 只让**错误清理原语碰巧生效**，`for p in $(jobs -p)` 在别的宿主/容器
  （`/bin/sh`=dash 的机器满地都是）依旧静默 0 命中；② 完全不解决 (ii) `pkill -f` 的 argv 改写与自匹配自杀，
  也不解决 (iii) 运行时 `already_exited` 分支不收组；③ 引入启动开销与跨平台行为差异，等于拿一个新的环境
  依赖去换一个不该存在的依赖。正解是禁掉这两类原语（CX-SAFE-4/5），而不是给它们找更好的土壤。
- **(e) 只修运行时 `already_exited` 分支、不做生成侧契约**：否决。缺口是真缺口，但只补它等于默认「压力
  命令可以继续乱写」：危险文本仍会进审查产物与 teammate 脚本，在没有该修复合集的宿主（旧版本、其它仓、
  手工 ssh 命令）上原样泄漏；且 `pkill` 自杀、`jobs -p` 空循环两层各自独立生效，补一层不能覆盖。
- **(f) 在 `process.rs` 加 `impl Drop for ProcessManager` 强制 `killpg`**：本轮不裁决、不并入本条。
  `Drop` 内发信号涉及「不得长时间阻塞、不得 panic」与既有有界 reap（`try_wait` + `sleep` 轮询）的冲突，
  需要单独设计接缝；且属代码变更而非契约变更，按测试分权与写域纪律归 executor。已拆入对应提案 note。

## Consequences

- **生成侧**：`counterexample_cmd` 与队友自建的压力命令都有可判定的硬形态——要么前部 `trap 'kill 0'
  EXIT INT TERM`，要么外层 `timeout -k 5`，且清理不得依赖 `$(jobs -p)` / `pkill -f`。「看起来像事故命令」
  的样例不再进入产物；「清理写了但 0 命中」这类**假收尾**被显式命名为违规。
- **门禁侧**：`exit 3` 让违规可被 CI/Join Barrier 机械拦截，且与既有 `exit 1` 语义正交（不回退既有校验）。
  代价是 lint 复杂度上升（判定树 5 步 + 8 条正则），由 10 段 fixture + `--self-test` 约束。
- **运行时侧**：`already_exited` 补 `killpg` 后，「命令正常退出」不再等于「孙进程无人管」——这是最容易
  漏的一层：前两层都能被"写对命令"绕过，只有这一层是无论命令怎么写都必须成立的安全网。
- **证据面**：255 个历史 JSON 保持只读，audit 溯源链完整；代价是存量 34 个文件仍含危险命令，若被复制
  粘贴仍可能复发——缓解只有门禁拦新 + 一次性复跑清单，不承诺历史清零。
- **口径风险**：命中数存在两个口径（审计侧 13 条 vs 本文粗口径 34 文件/53 条），说明「危险」是判据函数
  而非自然事实；统一口径的唯一方式是 executor 落地后对全量历史复跑一次，其输出即唯一计数收据。
- **已知破损**：`.dev-team/review-final.json:62` `Invalid \escape` 会让任何解析型审计报错；属历史证据且
  不产生进程风险，登记为已知破损，避免后来者误判为本次引入。
- **接续**：进程组回收的代码侧缺口（`already_exited`、无 `Drop`、pipe-missing 分支、`updater.rs:1200`、
  模块文档失真）见 `proposed/architecture/2026-11-16-process-group-reap-gap-remediation.md`。
