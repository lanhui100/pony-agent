# PA-047 收口工具权限事实、审批语义与失败归一化

## 状态
- Status: `Done`
- Priority: `P1`
- Owner: `Codex`

## OpenSpec Change
- 已归档：
  [2026-06-15-tool-permission-facts-and-approval-contract](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/archive/2026-06-15-tool-permission-facts-and-approval-contract>)

## Delta Spec
- 已同步并归档：
  `openspec/changes/archive/2026-06-15-tool-permission-facts-and-approval-contract/specs/tool-permission-contract/spec.md`

## Canonical Spec
- 已同步到：
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
- 已完成实现、测试、实现态审核与一轮采纳调优
- 已完成 OpenSpec strict validate，并已将 delta spec 同步到 canonical spec
- 已完成 change 归档与任务系统收口

## 下一步动作
已完成，无后续动作；后续若引入正式审批 UI 或更细权限策略，应以新 change 承接。

## 当前卡点
- 暂无。当前已完成归档与收口。

## 断点续跑提示
继续前先看：

- [capability_bridge.rs](/C:/Users/HUAWEI/Documents/pony-agent/crates/pony-agent-core/src/agent/capability_bridge.rs)
- [telemetry.rs](/C:/Users/HUAWEI/Documents/pony-agent/crates/pony-agent-core/src/agent/telemetry.rs)
- [runtime.rs](/C:/Users/HUAWEI/Documents/pony-agent/crates/pony-agent-core/src/agent/runtime.rs)
