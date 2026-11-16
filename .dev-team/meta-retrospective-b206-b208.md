# 元复盘 · B206 / B207 / B208（终止契约收口 + 原生范围归属 + 泄漏面清扫）

> 复盘对象：本次 Agent Team 的**协作元框架与组织协议**，不是业务代码细节。
> 成员：Lead + protocol2（契约/ADR）+ test-agent / test-agent2（独立测试）+ exec-reap（B206）+ exec-lint（B207/B207b）+ exec-b208（B208）。4 名只读 Explorer 供前期调研。前置工单 B205（2026-11-13 孤儿事故取证与门禁加固）。

## 1. 通信拓扑与信噪比（Topology & Noise）

- **Lead 的三次误判全被下游纠正，这是本轮最有价值的信号**：① 我断言「`terminate` 三处提前 `Ok` 就是全部缺口」——protocol2 独立审计发现**第四条路径 `shutdown`**（绕过 `kill_inner` 直调 `terminate`，而 `terminate` 在 `Exited` 时一个信号都不发）；② 我裁决「`quiesce_deadline=ZERO` 必然 `Err`，可作确定性红相」——test-agent2 与 exec-reap 各用实测推翻（SIGKILL 投递窗口 < 一次 `/proc` 扫描，而 PROBE-B 不计僵尸 ⇒ 首扫即判空 ⇒ `Ok`，3/3）；③ 我给的 `BG_OP`/`KILL_F` 正则与 `members` 正则都过宽或过窄 —— protocol2 反向修正（`A && B` 的第二个 `&`、`std::Vec` 的 `{:?}` 逗号空格）。**这三处若被 Lead 一言堂压过去，交付里就会各带一个假绿。**
- **信噪比**：回报普遍 40~120 行，但**几乎每一条都带可回放命令和真实 exit code**，没有套话。噪声主要来自「同一个数字在多处漂移」（`process_range.rs` 941 vs 958 行、`process.rs` 分项 delta vs 总账 +788/−52、`range_quiescence_test` 5 vs 8 用例）——每次都是**下游主动纠正上游的转述值**，规程是对的，但反映出 Lead 在转述数字时没有回读源事实。
- **真实损耗**：首派 executor（B205 阶段）上下文耗尽零产出；本轮 test-agent2 也在写完 B206 测试后耗尽（B208 门禁未落盘，改由 test-agent 补位）。两次都发生在「一个新文件 + 30~40KB 代码」量级的任务上。**改进**：这类任务应一开始就拆成「一个 Agent 一个文件」，并把「省上下文」条款（定点 read 行号、禁止整文件读）写进 brief —— 本轮后半程这么做之后零耗尽。

## 2. 门禁穿透与误杀率（Gate Penetration / False Positives & Negatives）

| 门禁 | 红相是否真实 | 绿相 | 误杀 |
|---|---|---|---|
| `range_quiescence_test`（8 用例） | **真实**：2 条原红（`shutdown` 返回时组仍有 pid、首信号跳过 SIGTERM）+ 后续 3 条追加用例；正控 2 条全程绿 | 8/8 | 0 |
| `process_group_reap_test`（B205，4 用例） | 真实（B205 阶段 2 红 2 正控） | 4/4（不回归） | 0 |
| `b208-leak-surface-check.sh`（15 项静态） | **自带负控**：`git show HEAD:` 断言改前必然失败 | 15/15 | 0 |
| `lib process`（29 条单测） | 既有 22 不回归 + 7 条 B207/B207b | 29/29 | 0 |

- **「误杀合规实现」是本轮最强的一类缺陷，共抓到 3 处**，全部由下游自证并修正：① 契约 `:360` 的 `!contains("quiescent")` 与 `:344` 强制模板 `range-not-quiescent` 直接冲突（照它写断言会把**正确实现判红**）；② 契约 `members=<\[[0-9]+(,[0-9]+)*\]>` 要求紧凑形态，而 Rust `{:?}` 打印是 `[a, b]`（逗号+空格）；③ 我的 `BG_OP` 前置集含 `&` 会把 `A && B` 判成后台作业。**教训：门禁/契约的正则与否定式断言，必须先对「当前合规实现」跑一遍，否则门禁会从「抓不住坏东西」变成「专杀好东西」。**
- **门禁判别力做了主动建构**：test-agent 静态影子的初版假设「`killpg` 必须内联在 `terminate_range_path` 体内」，被 B207 的 owner 抽象层打破 → 自纠为「两文件合计唯一 1 处 + 调用者白名单 + 阶梯两枪」的**结构无关判据**。E9 进而扩成跨文件，并用 `/tmp` 沙箱双变体（缺文件 / 两文件各 1 处）自证判别力。**这是本轮方法论上最正确的一步：断言契约，不断言实现形状。**
- **不可测项被显式登记而非糊过去**：`Err` 分支端到端不可构造（需 D 状态/特权）⇒ 契约 §7.1 R1 标 `[靠 review]` 并**注销那条已被实测推翻的推论**；live cgroup 路径零实测 ⇒ `[SMOKE DEGRADED: NO-USER-SYSTEMD]` + live 验证清单（第 1 项 = `--scope` 是否原地 `exec`，附可一眼定位的症状）。

## 3. 分工契约与隔离有效性（Contract Isolation）

- **写域正交**：`process.rs`（exec-reap → exec-lint 串行交接）/ `process_range.rs`（exec-lint）/ `tests/**`（test-agent）/ `updater.rs` + 两个 `.sh`（exec-b208）/ 契约 + ADR（protocol2）。零 FS_STALE_VERSION。
- **一处真实的排程失误**：我让 B207b（改 `process.rs`）与 test-agent 的验证**并发**跑，14:33 出现共享工作区不可编译窗口（`E0061` ×2），被 test-agent 精确截获并如实上报。**这不是 executor 的缺陷，是 Lead 的排程缺陷**：单写者模型的中间态对外是不可编译的，验证方必须等写者停笔。后续改为「写者冻结 → 只读 grep 预检 → 一次性全量验收」，零复发。
- **测试分权守住了底线**：`range_quiescence_test.rs` 前 859 行的 sha256 自冻结后**逐字节未变**（`e2e4ff41…`），test-agent 只在其后追加 —— 这是「既有断言不可改」的物理证据，不是口头承诺。
- **契约演化门禁真实运转**：两轮修订（v1.1 → v1.3）都是 Lead 裁决 → protocol2 就地改写并出「改前→改后行号对照表 + 版本指纹（`md5` 可复算）」→ test-agent 才对齐冻结。反向也成立：exec-reap 给 protocol2 的缺陷清单带精确行号（`:337`/`:360`/`:186`/`:156`），protocol2 照单落地并额外抓到第 4 处。
- **ADR 状态轴被严格执行**：B206/B207 落地前是 `proposed`、B208 已落工作树是 `implemented`，**分三篇**而非一篇吞下；落地后再整体迁 `implemented` 并把 Acceptance/Risks 折叠进 Consequences。`protocol2` 主动纠正我「B208 不动」的窄解 —— 断链该修。

## 4. 元协议迭代建议（Self-Evolving Protocol）

1. **门禁/契约正则的「合规实现回放」硬门禁**：任何新增机械断言（正则、否定式 `!contains`、结构假设）必须附「对当前合规实现跑一遍」的收据；若会把合规实现判红，**判 FAIL 打回**。本轮 3 处误杀若靠这条，全部会在落盘前被拦住，而不是靠下游嘴。
2. **「不可测项」升为标准区块**：验收报告必须单列「机器到不了、靠 review」的清单与理由（本轮 `Err` 分支、live cgroup 路径）。常载命约 2 已要求「机器到不了的显式标注」，但缺少「报告必须有一节装它」的**位置约束**——没有位置，就会被塞进脚注而被忽略。
3. **单写者冻结协议（新增）**：共享工作区里，凡涉及「写者 + 验证者」并发，验证方必须先确认写者 mtime 稳定（本轮 test-agent 用「75s 未变」）再开跑；Lead 侧排程不得让改同一 crate 的实现与验证并发。这条应写进 DAG 编排的机械校验项。
4. **Excutor brief 的上下文预算条款**：一个 Executor 只承担「一个文件 + 明确行号锚点」，>500 行文件禁止整文件读。本轮首派 executor 与 test-agent2 两次耗尽，都是这条缺失所致；后半程补上后零耗尽。
5. **Lead 转述数字必须回读源**：本轮 3 次数字漂移全部由下游纠正。建议 Lead 在把任何「行数 / 用例数 / delta」写进 brief 或收尾报告前，跑一条命令回读——成本 1 次工具调用，收益是消除整条链的信任损耗。

## 5. 未闭环项（诚实登记）

- **B207c**：`--unit=<minted>` 可消除 cgroup 解析依赖并去掉两处 item 级 `allow(dead_code)`（`process_range.rs:174`/`:357`），需改冻结 argv + live 验证 ⇒ 独立工单。
- **live cgroup 路径零实测**：上生产前必须在有 user manager 的宿主机跑模块文档的 live 验证清单（第 1 项 = `--scope` 是否原地 `exec`；若 fork，FENCE 读错 pid ⇒ `Stale` ⇒ 零信号）。
- **`Err` 分支端到端不可测**：需 D 状态挂死或特权；契约 §7.1 R1 已标 `[靠 review]`。
- **B208 残余**：`set -m` 下 `setsid` 可能 fork ⇒ pid 不可知 ⇒ 只告警不按名杀（文档化已知限制）；`l2at-tui-models.sh:426` 的存量 `pkill -f "fake_server.py"` 未动（HEAD 既有，另单登记）。
- **非 Linux unix 降级告警**（macOS/BSD：`read_start_time` 恒 `None` + 未收尸 ⇒ 落 `Stale` 零信号，语义近似「FENCE 不可用 ⇒ 视为范围已消失」，与契约 §6.2 的 A-only 降级不完全一致）⇒ 建议单开 macOS 工单。
- **工作区仍有他人未提交变更**（`src/bin/tui.rs`、`src/components/team/*`、`src/stores/team.ts`、`docs/observability.yaml`、`backlog/*`、2 篇 tui ADR），本轮**未做任何 git 提交**，交付物全部留在工作树。
