# PA-046 Spec Review

## 审核对象

- [PA-046 任务卡](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-046-agent-workspace-contract-and-path-boundary.md>)
- [proposal.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/agent-workspace-contract-and-path-boundary/proposal.md>)
- [design.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/agent-workspace-contract-and-path-boundary/design.md>)
- [tasks.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/agent-workspace-contract-and-path-boundary/tasks.md>)
- [spec.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/agent-workspace-contract-and-path-boundary/specs/agent-workspace-contract/spec.md>)

## 审核方式

- 独立智能体审查
- 审核智能体：`Mendel`
- 审核结论：`通过，但需收紧 workspace 规范面的闭合度`

## 主要发现

1. `canonicalize` 在 spec 中原本被写成可选语义，与 design 的强约束不一致。
2. `Run` 一类执行工具原本没有正式纳入 workspace 默认边界，合同不闭合。
3. `WorkspaceContext` 原本主要停留在 design 里，没有进入 spec 成为正式要求。
4. path repair 缺少稳定边界、停止条件与多候选冲突语义。
5. display path fallback 与结构化越界失败语义在 spec 中原本不够完整。
6. `tasks.md` 与任务卡状态未及时反映“已审核并已优化”的真实进度。

## 已采纳修改

1. 在 spec 中将 canonicalize 收紧为路径访问前的强制步骤，并明确 containment check 基于 canonical path 执行。
2. 在 spec 与 design 中补入 `Run-like` 工具默认使用 `WorkspaceContext.default_shell_cwd` 的规则。
3. 在 spec 中新增 `WorkspaceContext` 共同上下文要求，至少覆盖 workspace root 与默认执行目录。
4. 在 spec 与 design 中补入 path repair 的 workspace 内搜索边界、停止条件与多候选结构化冲突返回。
5. 在 spec 中补入 display path 的安全回退规则，并明确不得改变内部 canonical path 真相。
6. 同步 `tasks.md` 与任务卡进度表达，避免任务系统与 OpenSpec 实际状态脱节。

## 审核结论

本轮收紧后，`PA-046` 已经从“实现事实描述”提升为“可供权限合同、首批工具面与前端展示复用的正式 workspace 母合同”。当前剩余动作主要是：

1. 运行 OpenSpec change 校验
2. 在后续 `PA-047` 到 `PA-049` 中直接复用本卡的 workspace scope 与 display path 规则
