# 元复盘 · wave-tui-session-resilience

## 1. 通信拓扑与信噪比
本波按 Lead → Protocol / Test Agent / Executor 三常驻 + Subagent 计算池（侦察 3 路、方案审查 1 路、L3 对抗 2 路、PTY 冒烟 1 路）组织。严苛 JSON 协议在 Protocol 与 Test 的交接处发挥了作用：Test Agent 主动提出「红相不得 E0425」的编译可达性约束，直接促成 7 个纯函数空桩冻结；这是一条高信噪比回执。噪声主要来自三处：Protocol 首轮因过度侦察超时被 Lead 中断、Test Agent 首轮 max-tokens 未落盘、方案审查与两路 L3 审查子代理停滞且未产出收据。重复命令率可控（Lead 对同一门禁仅复跑一次），但重复等待 `wait_agent` 次数偏多，属拓扑层面的空转。

## 2. 门禁穿透与误杀率
红绿双相真实拦截：红相 `cargo test` EXIT=101，12/12 用例为 `unimplemented!()` 原生 panic，0 E0425、0 harness panic；`dev-red-purity.py` 判定 `pure_red=true`。AT1 首次使用固定字节切片触发 UTF-8 char-boundary panic（harness 栈），被 Lead 判为无效红相并要求原地修复，未穿透成假绿。绿相 Lead 独立复跑：`cargo check --all-targets` 0、验收 15/15、回归 19+12+8+8+5=52/52、`RUSTFLAGS=-D warnings` 0、`audit-diff` 0、`check-declaration` 0。机械写域审计发现 1 处收敛：HEAD 既有 `wave58_context_skills.rs` 缺 `shared_sessions` 字段，Lead 做 1 行编译衍生同步并登记豁免；`session/mod.rs` 3 行纯 re-export 登记 `lead-assembly` 例外。L3-P 六问逐项 PASS；L2-AT 真实拦截 1 个 harness 缺陷、0 个超纲误判。独立审查通道因 3 个子代理停滞降级为 Lead 双人复核（review JSON 均标 DEGRADED，不自签 PASS），这是本波最大的门禁弱化，须如实计入误杀/漏判风险。

## 3. 分工契约与隔离有效性
用户选择独立 worktree 后，本波与主工作树并发会话物理隔离，未发生脏写；红相冻结、绿相实现、Join Barrier 三段式 Git 检查点（66fb8555 / 4f98062e / 待合并）保证断点可续。测试分权有效：Executor 全程未改测试，Lead 独立复跑门禁，避免自证。但并行 ROI 为负：Protocol/Test/Executor 三常驻在共享 worktree 上串行等待，两轮 max-tokens 中断说明「上下文膨胀」比「墙钟等待」更贵；爆炸半径 451 行新增 / 85 行删除，触发 ≥2 路互补审查是正确判据，但审查子代理产出率不足，导致 Join Barrier 由 Lead 兜底。

## 4. 元协议迭代建议
1. 给 Subagent 计算池加「产出优先」硬约束：侦察/审查类任务在 N 步内必须落盘目标 JSON，否则由 Lead 中断并代跑，禁止无限展开验证；本波 4 个子代理均因该缺口停滞。
2. 红相空桩应在 Protocol 冻结阶段由机械模板生成（签名 → `unimplemented!("Cx")` → cargo check），避免 Test Agent 因等待签名耗尽上下文。
3. PTY 冒烟 harness 必须区分「pane 销毁导致 capture 为空」与「提示未输出」：断言应优先读 pipe-pane 原始字节，termios 读回仅在 pts 仍存活时可判，否则显式 `[SMOKE DEGRADED]` 而非 FAIL。
4. 对「独立审查通道停滞」建立熔断：连续 2 轮无收据即降级为 Lead 双人复核 + DEGRADED 标记，禁止把缺失的独立审查记成 PASS；该规则应写进 dev-team 的 Join Barrier 条款。
5. 目标锚定（create_goal）有效防止 4 现象诊断漂移，但应在契约矩阵里对每条用户现象标注「事实 / [UNVERIFIED]」，本波已补录 U1–U3，建议成为模板字段。
