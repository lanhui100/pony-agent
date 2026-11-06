# 元复盘 · wave-tui-session-resilience

## 1. 通信拓扑与信噪比
本波按 Lead → Protocol / Test Agent / Executor 三常驻 + Subagent 计算池（侦察 3 路、方案审查 1 路、L3 对抗 2 路、PTY 冒烟 1 路）组织。严苛 JSON 协议在 Protocol 与 Test 的交接处发挥了作用：Test Agent 主动提出「红相不得 E0425」的编译可达性约束，直接促成 7 个纯函数空桩冻结；这是一条高信噪比回执。噪声主要来自三处：Protocol 首轮因过度侦察超时被 Lead 中断、Test Agent 首轮 max-tokens 未落盘、方案审查与两路 L3 审查子代理停滞且未产出收据。重复命令率可控（Lead 对同一门禁仅复跑一次），但重复等待 `wait_agent` 次数偏多，属拓扑层面的空转。

## 2. 门禁穿透与误杀率
红绿双相真实拦截：红相 `cargo test` EXIT=101，12/12 用例为 `unimplemented!()` 原生 panic，0 E0425、0 harness panic；`dev-red-purity.py` 判定 `pure_red=true`。AT1 首次使用固定字节切片触发 UTF-8 char-boundary panic（harness 栈），被 Lead 判为无效红相并要求原地修复，未穿透成假绿。绿相 Lead 独立复跑：`cargo check --all-targets` 0、验收 15/15、回归 19+12+8+8+5=52/52、`RUSTFLAGS=-D warnings` 0、`audit-diff` 0、`check-declaration` 0。机械写域审计发现 1 处收敛：HEAD 既有 `wave58_context_skills.rs` 缺 `shared_sessions` 字段，Lead 做 1 行编译衍生同步并登记豁免；`session/mod.rs` 3 行纯 re-export 登记 `lead-assembly` 例外。L3-P 六问逐项 PASS；L2-AT 真实拦截 1 个 harness 缺陷、0 个超纲误判。独立审查通道因 3 个子代理停滞降级为 Lead 双人复核（review JSON 均标 DEGRADED，不自签 PASS），这是本波最大的门禁弱化，须如实计入误杀/漏判风险；但 PTY 冒烟通道独立补回了 C7/C8 的动态全绿证据（含负控制），未把降级当放行。

## 3. 分工契约与隔离有效性
用户选择独立 worktree 后，本波与主工作树并发会话物理隔离，未发生脏写；红相冻结、绿相实现、Join Barrier 三段式 Git 检查点（66fb8555 / 4f98062e / 待合并）保证断点可续。测试分权有效：Executor 全程未改测试，Lead 独立复跑门禁，避免自证。但并行 ROI 为负：Protocol/Test/Executor 三常驻在共享 worktree 上串行等待，两轮 max-tokens 中断说明「上下文膨胀」比「墙钟等待」更贵；爆炸半径 451 行新增 / 85 行删除，触发 ≥2 路互补审查是正确判据，但审查子代理产出率不足，导致 Join Barrier 由 Lead 兜底。

## 4. 元协议迭代建议
0b. 追加事实：L3-B 审查子代理同样超时未交 JSON，但其 /tmp/l3b_cmds.sh 反例集暴露 wave-2 仍未闭合的 4 处真实缺口（DSML 大小写/半角变体、context.rs 历史回灌、message_state fallback、检查点失败轮 TurnCommitted 漂移），Lead 据此开 wave-3 红绿补丁（4/4 + 15/15 + 2/2 + 52/52 全绿）。这说明「审查通道停滞 ≠ 无缺陷」，半成品证据必须强制复核，且同一现象可能跨多波次才闭合。
0. 追加事实：方案审查子代理虽因 max-tokens 未交 JSON，却留下 `/tmp/l3a-repro/repro` 物理反例，Lead 实跑发现 C1 跨 chunk 分片残留 `<｜` 的真实闭合缺口，随即补 wave-2 红绿补丁（2/2 + 15/15 + 52/52 全绿）。这证明「停滞代理的半成品产物也必须复核」，不能只按是否交收据计价。
1. 给 Subagent 计算池加「产出优先」硬约束：侦察/审查类任务在 N 步内必须落盘目标 JSON，否则由 Lead 中断并代跑，禁止无限展开验证；本波 4 个子代理均因该缺口停滞。
2. 红相空桩应在 Protocol 冻结阶段由机械模板生成（签名 → `unimplemented!("Cx")` → cargo check），避免 Test Agent 因等待签名耗尽上下文。
3. PTY 冒烟 harness 必须区分「pane 销毁导致 capture 为空」与「提示未输出」：本波首次因 tmux 随进程退出销毁 pts 误判 FAIL，后由 pty-holding wrapper（exec sleep 保持 pts、自身不写 termios + 负控制实验）把 termios 读回打通并反转裁决为动态 PASS。该「先隔离 harness 伪失败、再补负控制」应成为标准动作。
4. 对「独立审查通道停滞」建立熔断：连续 2 轮无收据即降级为 Lead 双人复核 + DEGRADED 标记，禁止把缺失的独立审查记成 PASS；该规则应写进 dev-team 的 Join Barrier 条款。
5. 目标锚定（create_goal）有效防止 4 现象诊断漂移，但应在契约矩阵里对每条用户现象标注「事实 / [UNVERIFIED]」，本波已补录 U1–U3，建议成为模板字段。

---

# 元复盘 · wave-tui-streaming-input（TUI 流式期输入 / 面板 / 排队）

## 1. 通信拓扑与信噪比
本波拓扑：Lead → 3 常驻 Teammate（Protocol / Test / Executor）+ Subagent 计算池（E2 侦察 1、PTY 冒烟 2、L3-A 6 轮 / L3-B 4 轮 / L3-P 2 轮 / plan 门禁 1 轮 = 13 个审查轮次；实现方 9 个执行轮）。信噪比最高的一条是 E2 侦察回执「goal 续轮不置 `is_streaming`，故首轮终态后仍可能有轮次在飞」——它直接改写了派发判据（C10 由「有无 goal」改为「轮次线程存活」），避免了一个会撕裂会话历史的并发缺陷；这条事实若由 Lead 自读代码发现，代价是 3~4 次上下文膨胀。噪声主要来自两处：① 契约符号被 Protocol Agent 按自己的收敛口径命名（`streaming_key_disposition`）而矩阵同时刻写的是另一套（`streaming_input_allowed`），Test Agent 因此按作废符号写了一版测试，返工一轮；② 审查轮次偏多（3 条独立通道 × 3 轮），但每一轮都抓到真缺陷（B1 反馈不可见、B2 A7 假绿、C23 渲染点空洞），不算浪费。重复命令率低：同一门禁仅复跑一次，变异实验统一在 /tmp 副本执行，工作区 src 全程未被二次改写。

## 2. 门禁穿透与误杀率
**PHASE-SKEW（本波最大门禁事故）**：Protocol 空桩落盘后与 Executor 实现之间没有真正的「红相冻结窗口」——实现提前落地，导致经典红相在物理上不成立。Test Agent 拒绝伪造红相，改用 `git archive adad39ad` 基线副本跑同一套测试拿到 EXIT=101（E0425/E0432/E0599 ×139），并显式标注「非当前工作区」。这是可接受的替代证据，但它证的是「测试依赖新符号」，不是「测试依赖新行为」；补足靠 **变异敏感度**（6/6 轮全红，11 用例全部被至少 1 个变异杀死）与 **L3-B 独立反向实验**（变异 A/B 各打红 2~3 个 A7 用例）。三处真拦截：L3-B 抓到「A7 断言的是测试内自建闩，生产派发点零引用」的假绿，逼出 C15 把取件收成生产符号；L3-A 抓到「流式 + 工具执行期排队指示零渲染点」，逼出 C23；plan 审查抓到「PlanMode 吞掉除 Esc/Up/Down 外全部按键」（既有缺陷，用户报的同类症状），逼出 C19。误杀方面：plan 审查把「契约矩阵 §2 引用作废符号」「§4 只到 A8」判 BLOCKING，属文档性缺陷被判成阻断级，偏严但正确（真值源不自洽会污染后续所有审查）。

## 3. 分工契约与隔离有效性
写域三分（tui.rs / tests / .dev-team）在共享工作区里始终互斥，未发生脏写；**唯一一次跨域事故是 Protocol Agent 越界**：它本应只写空桩，实际把实现连同主循环接线一起写了（或与 Executor 的写入混叠，日志无法区分），造成 PHASE-SKEW。根因是「空桩」与「实现」在同一文件的相邻位置，缺少物理隔离（例如空桩写到独立 `tui_contract.rs` 模块再由 Executor 在 tui.rs 引用）。测试分权有效：Executor 全程未改测试，Test Agent 全程未改 src，11+6 个用例的冻结与迁移均由 Test 侧单方完成；Lead 只做裁决与文档，未动业务代码（符合 Lead Write Prohibition）。并行 ROI：Executor 与 Test Agent 并行（src vs tests 正交）确实省了墙钟，但两者都需大量 `cargo` 编译，16 核机器上并发编译互相拖慢，收益被吃掉约一半。

## 4. 元协议迭代建议
1. **空桩必须物理隔离**：凡「改同一文件」的红相冻结场景，空桩应落在独立模块（如 `src/bin/tui_contract.rs`），实现阶段才由 Executor 接线到主文件；否则 Protocol 与 Executor 的写域天然重叠，PHASE-SKEW 必然复发。建议写入 dev-team 的第一阶段条款。
2. **红相替代方案的判据要收紧**：基线副本红相（E0425）只能证「符号缺失」，不能证「行为缺失」。应在 skill 里规定：实现先于红相时，**必须**补变异敏感度（每个契约符号至少 1 个变异能杀死对应用例），否则交付标记为 RED-DEGRADED。
3. **审查复跑成本要打表**：本波 9 个审查轮次里，3 轮是因为「实现方又改了代码」而必须重审。建议在 Join Barrier 前设「实现冻结声明」：Executor 报完成后 Lead 才放行审查，中途改码要显式 reopen 审查任务，避免审查报告与代码版本错配。
4. **假绿判定要有机械探针**：「测试内自建替代实现」是高频假绿形态（本波 A7）。建议给 L2-AT 增加机械规则：断言对象若在 `tests/` 内定义而同类符号在 `src/` 内存在，一律标记 `SUSPECT-SELF-REFERENTIAL` 并要求整改。
5. **计划门禁应在实现之前跑**：本波 plan 审查在实现完成后才跑，抓到的 B3/B5 都是「方案层冲突」，本可在编码前消灭。Phase 1 门禁要真的排在 Phase 2 之前，而不是事后补票。

## 5. 本波最终数字（机器收据）

- 实现轮：9 轮（ROUND 1–9），仅写 `crates/pony-agent-core/src/bin/tui.rs` 一个文件。
- 契约条目：C1–C27；冻结用例：A1–A27（acceptance 32 passed / adversarial 16 passed / six_fixes 18 passed，含 tui.rs 内部 9 单测）。
- 回归：`batch1/batch2/render_layout/six_fixes/stop_wave60` = 18/18/18/18/12，另有 L3-B 侧 15 套 258 用例全绿。
- 判别力：变异 M1–M9 全部至少打红一组用例；A7/A13/A25/A26/A27 有专门变异背书。
- 实机：PTY 假 SSE 9s 流窗口内「流式期输入 `hello`」「流式期 `Ctrl+T` 开 Team 看板」双 PASS。
- 独立验证：Test Agent 四轮自跑绿相（27→30→31→32 passed），最终 `tui.rs` sha256 前后一致 `56d58e81…`。
- 审查收敛代价：L3-A 连开 6 轮（每轮都找出上一轮未覆盖的新路径，末轮 PASS），说明「合成事件 + 共享输入框」这一派发设计的隐蔽状态空间远大于初判；若重做，应优先把提交逻辑抽成函数（一次性消除令牌/载荷/闩三者的耦合），而不是在事件回灌上逐点补洞。
