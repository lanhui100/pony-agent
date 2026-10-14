# Meta Retrospective: PA-118 进入会话页卡死（graph_runs 锁重入自死锁）修复

> Wave 8 · 任务分级 B（跨语言：Rust core + Tauri 同步命令 + TS 前端；命中并发/锁关键字）
> 红相锚定 `533e7b8`；基线 `506220b`；修复文件 `crates/pony-agent-core/src/agent/control_plane/history_commands.rs`

## 1. 通信拓扑与信噪比（Topology & Noise）

- **拓扑选择**：Lead 先做只读侦察 + 实机复现（后台 `cargo build --features tauri/custom-protocol` 与 Xvfb 无头运行），把「诊断」下沉为 2 个无状态 Subagent（前端 Playwright 复现 / Rust 锁审计），把「实施与验收」留给 3 个持久 Teammate（protocol-agent / test-agent / executor），Roster 峰值 = Lead + 3 = 4 ≤ 8，审查全部走 Subagent 池不占槽位。
- **信噪比关键决策**：Rust 审计 Subagent 用 `HostControlPlaneBuilder` + 内存 `GraphRunStore` 造**可机械判定的探针**（A/D 用例 EXIT=124 挂起 vs B/C EXIT=0），一次就把根因锁死到 `graph_run_for_retrieval` 重入加锁，避免了「猜根因 → 改代码 → 再猜」的高 Token 循环；Lead 随后用 `gdb -p` 主线程 backtrace 独立复核，形成「探针 + 运行时栈 + 实机截图」三重锚定。
- **零废话协议执行**：审查者均以 JSON（`verdict` + `findings[file/line/rule/counterexample]`）落盘（`.dev-team/review-l3p-deadlock.json`），无过程性吹捧；任务板描述全部带「判定方式可机械执行」。
- **Token 水位教训**：前端 Subagent 在「浏览器不可能复现」上花了 20+ 次运行（结论正确但边际收益低）；应在派发时就写明「浏览器无 Tauri 后端 ⇒ 结构性不可能复现桌面主线程阻塞」这一先验，把它降级为反证而非主攻。

## 2. 门禁穿透与误杀率（Gate Penetration / False Negatives & Positives）

- **假绿拦截（本轮最大收获）**：契约 v1 的 G1~G5/G7 用 `cargo test --lib <裸函数名> -- --exact` 逐字执行会选 **0 用例且 Exit 0**（libtest `--exact` 需全路径名、集成测试用例根本不在 `--lib` target 内）；G3 用 `--lib submission_plan_bounded` 同样 `running 0 tests`。是 L3-P 审查（finding #3）与 test-agent 主动声明共同暴露的，契约 v2 已冻结「假绿禁令」四条规则（必须校验 `running N tests` 的 N ≥ 契约值；`--exact` 必须全路径；集成用例禁用 `--lib`；不得以 Exit 0 替代「选中且通过」）。
- **真实拦截 vs 误杀**：红相 3 例超时失败（C1×2 + C2×1）+ 并发 2 例 0/8 失败，修复后 13+2 全绿（同一用例集，耗时 15.2s → 0.31s 是死锁消除的直接物理特征）。误杀 1 例：`mod.rs:2059` 被静态守卫标记 P1，Lead 机械核实 `session_id=None` 时 `load_active_graph_run_for_session` 在加锁前 early return ⇒ **非死锁**，改登记 P1-SAFE（精确到 file+方法+加锁行号，含 STALE 自审计与 `--strict` 升级开关）。
- **对抗审查的真实价值**：并发视角 L3 发现 T3 在 `run_id=Some(命中)` 路径**新引入 TOCTOU 撕裂读**（原本单 guard 原子读且不死锁），并给出非可线性化证明（`Paused∧waits=1` 在两次加锁间被 `graph_resume_ask` 排空 ⇒ 输出 `resume_graph_run_stream`，服务端因 phase=Ready 报 not resumable）。该缺陷**任何单线程门禁原理上都无法观测**，是「互补视角审查」不可替代性的直接证据；T7 已整改为命中路径单 guard 原子读。
- **门禁加固**：C5 守卫从 4 名硬编码白名单升级为「基线 4 ∪ 自动发现 18」，并新增「发现数 < 基线即 Exit 2」的退化防线与 `--self-test` 正负样例；`check-lock-discipline.py` 交叉验证同一结论。
- **NFR-DEGRADED 遗留**：1 项有条件豁免（`logging_legacy_eprintln_exempt=true` + `logging_debt_ref="PA-119"`），到期条款已写入契约 §3.1：PA-119 完成时必须重跑本波 L3-P 五问并关闭豁免。

## 3. 分工契约与隔离有效性（Contract Isolation）

- **写域正交**：task-1 `.dev-team/`、task-2 `crates/pony-agent-core/tests/` + `scripts/`、task-3/6/7 `history_commands.rs`、task-8 `scripts/check-graph-run-lock-scope.mjs` 互不重叠；`git diff --stat` 终态 = 1 个业务文件 + 1 个守卫脚本 + 3 个契约/基准文件 + 1 个任务板，无越界写入。
- **测试分权有效**：Executor 全程无测试写权；红相锚定 `533e7b8` 先于任何业务写入；T4 发现并修掉「集成测试打开用户生产 `sessions.db`」的卫生缺陷（根因：隔离用 `#[cfg(test)]`，而 `tests/*.rs` 编译依赖 crate 时不启用该 cfg），改由测试进程内重定向 `XDG_DATA_HOME/HOME/LOCALAPPDATA` 到唯一临时目录。
- **契约冻结与演进**：波内发生 2 次契约演进（logging 适用范围收窄、G1~G7 假绿修正），均由 protocol-agent 落盘 + Lead 冻结，未出现「就地改契约迁就实现」。
- **并行隔离成本**：本波并发度实际为 1~2（诊断 Subagent 并行、实施串行），未使用 worktree；事实证明对「单函数锁语义」这类强耦合缺陷，串行 + 强门禁的协调成本远低于并行隔离成本。

## 4. 元协议迭代建议（Self-Evolving Protocol）

1. **「同步 Tauri 命令 = 主线程」应写入项目常载约束**：Tauri v2 中非 async `#[tauri::command]` 在主线程执行，任何阻塞都会冻结窗口。建议在 `AGENTS.md` 核心架构约束中增列一条：新增/修改同步命令必须证明「无重入锁、无长 IO、有界返回」，并纳入 review checklist。
2. **门禁必须带「选中数」断言**：本波暴露的假绿是通用陷阱（过滤器拼错/`--exact` 路径不全/`--lib` 选不中都会 Exit 0）。建议 `dev-team` skill 的 G 门禁模板强制包含 `running N tests` 且 N ≥ 契约值的机械断言。
3. **锁纪律静态守卫应作为共享工具沉淀**：`scripts/check-graph-run-lock-scope.mjs`（自动发现 locker + 括号级作用域 + 退化防线 + 自检）可泛化到其它非重入 Mutex（`sessions_rwlock`、`ask_dispatcher`），建议登记为 `.agents/skills/` 复用件。
4. **诊断类 Subagent 派发时需带「结构性不可能」先验**：避免把「环境上不可能复现」的路径当主攻方向，节省 20+ 次无效运行。
5. **实机 E2E 应固化为「BEFORE/AFTER 同口径」模板**：本波用 Xvfb + `dbus-run-session` + 隔离 HOME + 真实 `sessions.db` + `--features tauri/custom-protocol` 内嵌 dist，主线程 `wchan` + DIAG 心跳 + 截图三件套构成可复用的桌面端卡死判定口径，建议写入 `test-expert` 的桌面端验收章节。
