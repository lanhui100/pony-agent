# Tasks: Add Second Wave Tool Surface

## 执行依赖

任务必须按 `Phase A -> Phase B -> Phase C -> Phase D` 的顺序收敛，不按跨阶段并行方式推进。

- `Phase A`：`Edit / Write / RunShell` 边界先收口
- `Phase B`：在 `Phase A` 稳定后，再讨论 `Glob / Grep`
- `Phase C`：在本地闭环和探索边界稳定后，再讨论 `WebFetch / WebSearch`
- `Phase D`：只定义 `MCP Resource / ToolSearch` 的进入条件与 deferred 范围，不展开实现方案

## 1. Spec And Task-System Alignment

- [x] 1.1 新增 `PA-050` 任务卡，明确第二批工具能力的目标、范围和验收标准
- [x] 1.2 为本卡补 proposal / design / delta spec / tasks 草案
- [x] 1.3 在任务卡中明确当前真实内建工具与已冻结首批工具合同之间的缺口

## 2. First-Wave Gap Closure Scope

- [x] 2.1 明确 `Edit` 的正式边界
- [x] 2.2 明确 `Write` 的正式边界
- [x] 2.3 明确 `RunShell` 作为 `Run` 实现升级路径的正式边界

## 3. Second-Wave Candidate Scope

- [x] 3.1 明确 `Glob / Grep` 的进入范围与优先级
- [x] 3.2 明确 `WebFetch / WebSearch` 的进入范围与优先级
- [x] 3.3 明确 `MCP Resource Read` 的进入范围与优先级
- [x] 3.4 明确 `ToolSearch` 的 deferred 进入条件与后置理由
- [x] 3.5 明确当前 v1 不纳入的扩展能力

## 4. Review And Tightening

- [x] 4.1 使用 `opencode / deepseek-v4-flash` 完成至少一轮独立只读 spec 审核
- [x] 4.2 汇总多维审核意见，明确采纳与不采纳项
- [x] 4.3 根据采纳意见调优一轮 proposal / design / spec / tasks
- [x] 4.4 运行 OpenSpec change 校验
- [x] 4.5 在任务卡中同步 spec 状态、审核记录与下一步动作
