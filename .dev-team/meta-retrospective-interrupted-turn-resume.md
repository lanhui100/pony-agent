# 元框架复盘：wave-interrupted-turn-resume（B 级）

任务：修复 TUI 中断轮次续跑后「原任务消失」（事故 `pa_01a1ce16-cf4e-72d0-a92b-ae9857aac737`）。
基线 `a7393954`，分支 `fix/interrupted-turn-resume`；ADR `2026-11-12-interrupted-turn-content-preservation`。

## 1. 通信拓扑与信噪比

- 拓扑：Lead（裁决/契约/收口）＋ Protocol（事实复核/空桩/NFR）＋ Test（红相/对抗）＋ Executor（实现）；
  三路审查全部下沉 Subagent 池（L3-A 并发与状态流转、L3-B 契约与持久化、L3-C 定点假设验证），常驻名册 4 人。
- 信噪比：三次关键裁决都以「一句话 + 判据 + 落点」下发（C1 断言面冻结、C6/AT2 计数面收敛、O3 修 cursor 一致性），
  Executor/Test 回执均为「改动清单 + 原始 Exit code」，未出现过程性吹捧。
- 重复动作（应改进）：Lead 与 Protocol 就空桩文件产生**并发写风险**（Lead 代写后 Protocol 又在同文件追加），
  靠抢占协调消息才收敛。正确做法：空桩所有权一次性写进任务板并在派生 Prompt 标注「Lead 已代写」。
- 一次真实无效开销：Lead 为核验账本在 worktree 内执行 `backlog record`，脚本按 git 公共根解析，
  实际写入主工作树（并行会话现场）——**工具 cwd 语义未先探活即断言**，是本波最典型的反模式。

## 2. 门禁穿透与误杀率

- **有效拦截（本波最高价值）**：Lead 用 `grep` 发现 `interrupted_turn_assistant_text` /
  `interrupted_turn_transcript_closure` 只有测试在调用，生产收口路径未接线——当时验收已 140/141 绿，属**假绿**。
  处置：下发接线要求 + 由 Test Agent 追加 C9 `include_str!` 结构性 wiring guard（红相 Exit 101），接线后转绿。
- 断言面误判 2 次：C6/AT2 原断言对整段 `ProviderRequest` JSON 计哨兵 ==1，而该往返在 native 模式必然出现 3 次
  （native_messages + input.carry + observation 渲染），数学不可能成立。Lead 裁决收敛为「native 通道内 ==1」，
  Test Agent 迁移并记入 `amendments`（原断言 / 不可能性证明 / 裁决 / 重跑 exit code 全留痕）。
- 未穿透项：Executor 自报一次 `cargo --lib` 漏带 workdir 落到默认目录（已 kill、无副作用），
  说明「所有 bash 必须显式 workdir」只能靠纪律 + 自报，机械门禁覆盖不到。

## 2.1 审查闭环实测（修正轮）

- 首轮三路 L3 独立审查判 **FAIL**：4 BLOCKING（分支头回填漏改使上一轮节点被错标、`turn_stream` 预阶段取消点
  off-by-one 让在飞调用静默消失、普通轮折叠写回未收口、DSML 残片行判定误删 fenced 代码块内正文）+ 13 ADVISORY。
- 修正轮后复核（`review-recheck-*`）**PASS**、无 BLOCKING；我（Lead）复跑 `--lib` 1511 passed / 验收 143 passed 均 Exit 0。
- 另一处真实缺陷由 Test Agent 的**真实 sqlite 重启**用例（C10）抓出，而非审查抓出：说明"审查看代码"与
  "测试看落盘"是互补通道，二者缺一都会漏。
- 误杀/度：C6/AT2 两条断言面被判为"不可能满足"后收敛，属**必要的契约演进**而非降级；`
  workshpace_hotloader` 邻接改动走 11 文件补偿门禁后放行。

## 3. 分工契约与隔离有效性

- 写域隔离有效：Protocol（`.dev-team/` + 空桩）、Test（`tests/`）、Executor（`src/agent/**` + `bin/tui*` + 声明）
  三方无越界写入；源码与测试的写主分离守住了「运动员 / 裁判」边界。
- 契约演进纪律成立：两次断言面修订均由 Lead 一票裁决、Test Agent 迁移、red-phase JSON 留 amendments，
  未出现 Executor 顺手改测试。
- 遗留张力：`workspace_hotloader.rs`（进程本地缓存事实被注入请求体）虽由 C6 确定性判据逼出，仍属
  「最小闭包之外的邻接改动」；处置为要求 11 个 hotloader / 全局配置集成测试逐个 Exit 0 作为补偿门禁。
- 隔离反例：主工作树同时被另一会话改动（`verify/long-running-stress` + `/goal` 未提交变更），故全程在
  `git worktree` 内推进、`CARGO_TARGET_DIR` 复用依赖产物；代价是 `--lib` 基线按 worktree 口径（1510）解释，
  而非主工作树的 1518。

## 4. 元协议迭代建议

1. **wiring guard 固化**：凡「新增纯函数 + 由既有生产路径调用」的修复，验收套件必须含一条 `include_str!`
   结构性守卫（断言生产文件出现调用名）。本波证明它能在 140/141 绿的情况下抓出假绿。
2. **写前探活工具 cwd 语义**：任何会写账本/状态的脚本（backlog、state、queue）必须先确认落点；
   「在 worktree 里跑就写到 worktree」是错误假设，应写入 skill 的 Gate 清单。
3. **断言面冻结必须声明通道**：请求体的多通道重复（native / carry / observation）是复发性误判源；
   契约矩阵应要求「断言必须声明通道」，禁止全局计数类不可能断言。
4. **Owner 冲突前置**：派生 Prompt 必须写明哪些文件已由 Lead 代写，避免两名成员对同一空桩并发写。
5. **邻接改动绑定补偿门禁**：允许必要的 blast-radius 邻接改动，但任务书须预绑「补偿门禁集」
   （本波：11 个集成测试文件）并作为合并前置条件。
