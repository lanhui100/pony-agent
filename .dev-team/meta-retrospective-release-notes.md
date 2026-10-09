# 元框架复盘 —— wave-release-notes（发布说明生成器修复）

## 1. 通信拓扑与信噪比（Topology & Noise）

- 契约矩阵 + 红相锚定提交（4dafafa）承担了跨角色事实底座：Test Agent 与 Executor 均只读契约即可对齐，未发生跨角色语义往返。Test Agent 主动提请 2 点契约复核（repoName 入参、type 大小写归一化），Protocol 以最小 diff 修订，通信为单次定向消息，信噪比高。
- 失败模式：executor teammate 两次因上下文超限中断（未产出即被销毁），实现转由契约冻结后的确定性投影 subagent 完成；L3 视角 B/C 的 subagent 亦多次超限/未落盘，经重派与 workflow 池兜底收敛。教训：长任务 teammate 的 prompt 应控制信息量（一次只给最小必要上下文），审查类无状态工作优先走 workflow/subagent 池而非常驻槽位。

## 2. 门禁穿透与误杀率（Gate Penetration）

- 红相门禁真实拦截：`28 failed | 5 passed` 恰证功能未实现（5 passed 为空桩红相门禁本身），红相锚定提交在绿相实现前完成，Test Agent 独立冻结、Executor 只读——分权链条成立。
- 静态前置断言（Ops/Infra 形态，标注 `[RED DEGRADED: STATIC-ONLY]`）在绿相转为 7 例有效断言，其中 §6.4 三处路径同源与 checkout fetch-depth 断言直接拦截了致命 CI 缺陷。
- L3 视角 C 的致命发现（checkout 浅克隆 → notes 恒占位）是本次最高价值拦截：本地完整克隆冒烟与显式 --from 无法暴露，只有针对 CI 语义的对抗审查才复现。修复后终检 PASS。误杀率低（仅 parseCommit 空 description 一处契约漂移，属实且已闭环）。
- 工具链缺失记录：dev-stub-lint.py / dev-nfr-lint.py / dev-red-purity.py 在 skill 目录缺失，本波次从 /tmp/dev-team-sandbox 找到副本并使用（stub-lint Exit 0）。polyglot 空桩（JS/Python 双语法）是 Protocol 针对 stub-lint 的合法适配，已在契约矩阵 §8 固化。

## 3. 分工契约与隔离有效性（Contract Isolation）

- Protocol（契约+空桩+ADR）→ Test（红相+验收）→ Executor（实现）→ L3（双视角）串行 DAG 有效：写域无并发交集，task-1/2/3/4 依赖链清晰。
- Executor 无测试写权限、Test 无实现写权限，绿相交接时 RED 门禁 describe 由 Test Agent 移除而非实现方——分权严格执行。
- 并发会话干扰（Cargo.lock 版本改动、ponysentry 测试文件）被写域审计识别并排除出本波次提交，定向 `git add` 未混入无关文件。

## 4. 元协议迭代建议（Self-Evolving Protocol）

1. **Executor/审查 subagent 上下文预算**：spawn 的初始 prompt 应预估目标文件的规模（本波次实现 319 行 + workflow），超长实现拆两段派发；审查类任务统一走 workflow 池并强制 schema 校验落盘，避免"完成但未落盘"。
2. **Ops/Infra 任务的红相形态**：静态前置断言 + 真实 CLI 冒烟 + 浅克隆复现是有效组合，应上升为 Ops 任务的固定模板（含"CI 语义"专项：runner 环境、checkout 深度、shell 退出码传播）。
3. **契约矩阵 live 修订**：Test Agent 提请契约澄清时应默认走"Lead 一票裁决 → Protocol 最小 diff → Test 零迁移"快路径，本波次该路径执行顺畅，可固化为流程。
4. **并发会话防护**：多会话共享工作区时，写域审计 + 定向 add + 独立 state 账本（state-<wave>.json）已证有效，建议沉淀为 skill 默认规则。
