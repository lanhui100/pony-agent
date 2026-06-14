# PA-047 收口工具权限事实、审批语义与失败归一化

## 状态
- Status: `In Progress`
- Priority: `P1`
- Owner: `Codex`

## OpenSpec Change
- 活跃路径：
  [tool-permission-facts-and-approval-contract](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/tool-permission-facts-and-approval-contract>)

## Delta Spec
- 活跃路径：
  `openspec/changes/tool-permission-facts-and-approval-contract/specs/tool-permission-contract/spec.md`

## Canonical Spec
- 待实现并归档后同步到：
  `openspec/specs/tool-permission-contract/spec.md`

## Spec 状态
- Proposal: `validated`
- Spec: `validated`
- Design: `validated`
- Tasks: `validated`

## 背景
当前 capability bridge、runtime、telemetry 已经出现 `requires_approval / permission_scope / host_mediated / permission_profile` 等字段，但 built-in / capability / skill / composite tool 仍需通过统一合同收口。

## 目标
统一工具权限事实、审批语义与失败归一化，明确工具定义层、权限决策层与前端读面的边界。

## 输出
- `tool-permission-facts-and-approval-contract` OpenSpec change
- 统一 `ToolPermissionFacts` 或等价合同
- `permission_denied / approval_required / out_of_scope` 等失败语义
- built-in / capability / skill / composite tool 的统一权限口径
- trace / monitor / control-plane 权限读面要求

## 范围边界
- 本卡不直接实现完整审批 UI
- 本卡不重做 capability bridge 主逻辑
- 本卡不要求所有权限策略一次落地，只要求正式写清合同与失败语义

## 验收标准
- 系统 SHALL 定义统一工具权限事实合同
- 工具定义层与权限决策层 SHALL 保持分层
- 技能、能力、内建工具与复合工具 SHALL 复用同一套权限与失败语义
- 至少完成一轮独立 spec review，并根据采纳意见优化文档

## 当前进展
- 已创建 OpenSpec change：
  [tool-permission-facts-and-approval-contract](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/tool-permission-facts-and-approval-contract>)
- 已完成本卡第一版 OpenSpec artifact 草案：
  - `proposal.md`
  - `design.md`
  - `tasks.md`
  - `specs/tool-permission-contract/spec.md`
- 已完成至少一轮独立智能体 spec review，并采纳意见收紧一轮文档
- 已沉淀 review 记录：
  - [2026-06-13-pa047-spec-review.md](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/02_REVIEWS/2026-06-13-pa047-spec-review.md>)
- 已完成 OpenSpec strict validate：
  - `npm run openspec -- validate tool-permission-facts-and-approval-contract --type change --strict --json --no-interactive`

## 下一步动作
把权限 envelope 继续复用于 `PA-048` 与 `PA-049` 的工具面和前端读面设计，并准备进入实现排序。

## 当前卡点
- 暂无。`PA-045` 与 `PA-046` 已可作为本卡前置母合同使用。

## 断点续跑提示
继续前先看：

- [capability_bridge.rs](/C:/Users/HUAWEI/Documents/pony-agent/crates/pony-agent-core/src/agent/capability_bridge.rs)
- [telemetry.rs](/C:/Users/HUAWEI/Documents/pony-agent/crates/pony-agent-core/src/agent/telemetry.rs)
- [runtime.rs](/C:/Users/HUAWEI/Documents/pony-agent/crates/pony-agent-core/src/agent/runtime.rs)
