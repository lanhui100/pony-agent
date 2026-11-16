# 元复盘 · B205（40 个 bash 死循环孤儿事故：取证 / 审计 / 收口 / 门禁加固）

> 复盘对象：本次 Agent Team 的**协作元框架与组织协议**，不是业务代码细节。
> 成员：Lead + 3 名常驻 teammate（protocol / test-agent / exec-reap·exec-lint）+ 4 名只读 Explorer（Subagent）+ 1 名重派 Executor。工单 B205。

## 1. 通信拓扑与信噪比（Topology & Noise）

- **Explorer 外派是本轮最大收益**。四条只读侦察（触发源取证 / 泄漏面审计 / 宿主机卫生 / 契约对齐）全部下沉为无状态 Subagent，Lead 上下文只保留「一级结论 + 证据指针」。E1 的取证直接推翻了任务书的核心假设；若由 Lead 亲自跑，几百行 `zstd -dc | grep` 输出会把注意力预算吃光。
- **任务书假设被物理证据推翻，这是本轮的方法论要点**：任务书断言「触发源 = `.dev-team/review-tool-timeout.json` 的 `counterexample_cmd`」。E1 用「Nov 13 17:20–17:45 `.dev-team/` 零写入 + 该 json mtime 是 Nov 4 + 全机仅一个会话日志含该死循环 + `taskset -c 0` 亲和性与 3.1%/49.5% CPU 的算术吻合」四证，定位真凶是 teammate `b137-flaky` 的自建压测。**若不做这层取证，全部治理措施都会打在一个从未被执行的 json 上，事故必然复发。** 教训：`反例命令文本存在` ≠ `该命令被执行`，证据链必须落到执行痕迹（mtime / session log / 侧写物）。
- **Test Agent 反向纠正 Lead 同样关键**：我给的 `bash -c '… & … & wait'` 形态今天**已经是绿的**（B009 的 killpg 没坏）。Test Agent 把两条做成正控并指出真实缺口在 `already_exited` 早退分支。若由 Lead 或 Executor 自测，很可能为了让「我以为是缺口的地方」变红而写出假红相。
- **噪声门禁是次级教训**：lint 首版出来 36 个新增红里 26 个误红（`&str`/`&mut`/`&RULES`/裸 `pgrep -af`/`#` 注释）。**75% 噪声率的门禁会被使用者直接关掉，等于没做**——比没有门禁更糟，因为它消耗了「门禁可信」这份社会资本。判据必须先证明「杀得对」再谈「杀得多」。
- **真实损耗**：一次 Executor 上下文耗尽（首派 executor 读完红相基线即用尽配额，零产出）。后续改为「一 Executor 一窄任务」+ briefing 内嵌定点 read 行号才稳定。
- **改进**：Executor brief 应强制「单目标 + 定点行号 + 禁止整文件读 >500 行文件」。

## 2. 门禁穿透与误杀率（Gate Penetration / False Positives & Negatives）

| 门禁 | 穿透情况 | 误杀 | 结论 |
|---|---|---|---|
| `dev-review-lint.py` 物理反例门禁（改前） | **完全穿透**：唯一内容校验是 `len(cmd) < 5`，全文对 `kill|jobs|nohup|setsid|disown|trap|while` 命中数 = 0；事故原件复跑 `[PASS] exit 0` | 0（无判别力 ⇒ 什么也抓不到） | 形同虚设 |
| CX-SAFE 门禁（首版） | 事故原件 → `exit 3` ✅ | **26/36 = 75% 误红** | 不可用 |
| CX-SAFE 门禁（R1/R2/R3 收窄后） | 事故原件 → `exit 3`，四条模式俱在；真实红 10/10 全抓 | **0**；`old exit 1 → new exit 0` 仍为 0（既有结构校验零放宽） | 可用 |
| B205 回归测试（组 A，4 用例） | 红相 2/2 真实失败（`kill(-pgid,0)=0` + 存活 pid 点名）；绿相 4/4 | 正控 2 条全程为绿 ⇒ 判据未漂移 | 有效 |
| B205 门禁用例（组 B，17 用例） | 红相两批（4/9 → 5/17）精确定位；绿相 17/17 两次复跑一致 | 0（5 条「必须过」逐条复核物理上均不泄漏） | 有效 |

- **归因有效性做了五重反证**（未被「4 passed 就算数」糊弄）：测试文件字节不变 / 源码 diff 仅 1 hunk / `--nocapture` 实证 shell `exit 0` 后孙进程仍存活 / 耗时 17.69s→2.12s / 正控红绿两相全绿。**绿灯只证明「现在不报错」，不证明「是我这行改动让它不报错」**。
- **已知误杀/漏杀残余**（exec-lint 主动列 6 条，未靠放宽换绿灯）：`sleep 30&` 无空格写法漏抓、引号掩码导致 `echo "… & …"` 内真实派生不扫、`grep 'a # b'` 的 `#` 被误剥、无引号散文里的 `pkill -f` 字样仍误红、未闭合引号行残留误红、跨多管道 `pgrep|sort|kill` 漏抓。方向性选择：**宁可漏弱形态，不恢复高误红**。
- **三层治理缺一不可**：契约（生成侧，管人怎么写）+ 门禁（校验侧，管写错了能不能落盘）+ 运行时 killpg（兜底）。`already_exited` 缺口证明**只做前两层不够**——命令可能"自己成功退出"，此时没有任何 terminate/timeout 会被触发。

## 3. 分工契约与隔离有效性（Contract Isolation）

- **写域正交完全成立**：`process.rs`（exec-reap）/ `dev-review-lint.py`（exec-lint）/ `tests/process_group_reap_test.rs` + `evals/*.py`（test-agent）/ 契约 + 2 篇 ADR（protocol）/ 事故报告 + backlog（Lead）。**全程零写域冲突、零 FS_STALE_VERSION。**
- **共享工作区污染是真实风险且被正确规避**：工作区存在**他人未提交变更**（`src/bin/tui.rs`、`src/components/team/*`、`src/stores/team.ts`、`docs/observability.yaml`、`backlog/*`、2 篇 tui ADR）。故本轮**全程不执行任何 git 写操作**（commit/stash/checkout 均禁用），并把「禁止 `git stash` 做对照实验」写进 Test Agent brief，改走 `git show HEAD:…` 只读导出过期版本——这条纪律避免了「为做 A/B 对照而 stash 掉别人半天的工作」。
- **契约演化门禁真实生效**：门禁收窄不是 Executor 自行决定，而是 Lead 裁决 → protocol 就地修订契约 §3/§9（并**反向技术修正 Lead 裁决**：我给的 `&` 前置集含 `&` 会把 `A && B` 的第二个 `&` 判成后台作业，protocol 剔除并补 `(?!&)`）→ test-agent 先加 8 条用例锚红相 → exec-lint 才动手。**契约先行、测试第二、实现最后**，使收窄既有红相可证、又有 golden case 可守。
- **一处分工失效**：首派 Executor 上下文耗尽，归因是我把「任务 A + 任务 B」两个大目标塞进一个 briefing 并给了整文件阅读自由度。**这不是容量问题，是 briefing 设计问题。**
- **独立性守住了底线**：Executor 自测绿 → Test Agent 独立复跑（含归因反证）→ Lead 再抽验（`exit 3` / 误红样本 `exit 0` / `--self-test` / `4 passed` / 全量 112·51·10 独立复算）。三级收据链，没有任何一级采信上一级的口头声明。

## 4. 元协议迭代建议（Self-Evolving Protocol）

1. **新增「触发源双证律」**：凡「历史事故溯源」类任务，Explorer 必须同时给出「产物存在」证据与「产物被执行」证据（mtime / session log / 侧写物），**仅凭产物内容推断执行者一律标 `[UNVERIFIED]`**。本轮若缺此条，整套治理会打在一个从未执行的 json 上。
2. **门禁类交付增加「误红率」硬指标**：任何新增机械门禁，验收必须附「全量语料复跑的新增红清单」并**按误红/真红分类**，误红率 > 20% 判 `FAIL` 打回（本轮首版 75% 会被这条直接拦住，而不是靠 Lead 事后发现）。同时把「`old exit 1 → new exit 0` 必须为 0」写成通用断言，防止门禁靠放宽换绿灯。
3. **「红→绿归因反证」升为标准验收项**：现有规范只要求「红相失败 / 绿相通过」，这只证明「不报错」，不证明「是我的改动生效」。建议把本轮五重反证（测试字节不变 / diff 范围 / 正向运行时证据 / 量化耗时变化 / 正控未漂移）提炼为 `test-expert` 的 L2 标准检查表，至少取 3 证。
4. **Executor briefing 模板加两条硬约束**：① 一个 Executor 只承担一个窄目标（本轮首派双目标导致上下文耗尽、零产出）；② 涉及 > 500 行文件时必须给出「定点 read 行号区间 + 禁止整文件读」条款。
5. **共享工作区加「禁止 git stash 做对照实验」条款**：`git stash` 在多人未提交工作区是破坏性操作，A/B 对照应走 `git show HEAD:<path>` 只读导出。本轮是临时口头纪律，应上升为常载条款。

## 5. 未闭环项（诚实登记）

- `process.rs` 无 `impl Drop for ProcessManager`（proposed ADR 已记，风险登记）；`process.rs:815-845` unix pipe-missing 分支不收组（当前不可达）；`updater.rs:1200` 裸 `Command::new` 无进程组（LEAK-RISK）。
- `l2at-tui-models-auth.sh:123` / `l2at-tui-models.sh:93` 的后台 server 无 `trap` 回收（每跑必漏 1 个常驻 server），本轮未修。
- 21 个僵尸（无一个属禁止触碰服务）与 6 个 dm pony 会话：本轮**只给判据不执行清理**（清理对象必须是父进程，会丢用户会话上下文，需用户决策）。
- `.cargo/config.toml` `jobs=3` 与 `~/.cargo/config.toml` `jobs=2` 矛盾且内层覆盖外层（ADR 视之为硬约束但实测失效），未改。
- `--release` 对 tui 测试内存的影响**未实测**（只读约束下不产生构建产物）。
- 本轮**未做任何 git 提交**（共享工作区存在他人未提交变更），交付物全部留在工作树。
