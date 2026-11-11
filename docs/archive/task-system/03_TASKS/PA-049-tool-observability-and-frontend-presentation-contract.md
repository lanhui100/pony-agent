# PA-049 收口工具观测读面、前端呈现与迁移验收

## 状态
- Status: `Done`
- Priority: `P2`
- Owner: `Codex`

## OpenSpec Change
- 已归档：
  [2026-06-15-tool-observability-and-frontend-presentation-contract](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/archive/2026-06-15-tool-observability-and-frontend-presentation-contract>)

## Delta Spec
- 已同步并归档：
  `openspec/changes/archive/2026-06-15-tool-observability-and-frontend-presentation-contract/specs/tool-observability-contract/spec.md`

## Canonical Spec
- 已同步到：
  `openspec/specs/tool-observability-contract/spec.md`

## Spec 状态
- Proposal: `validated`
- Spec: `validated`
- Design: `validated`
- Tasks: `validated`

## 背景
工具系统不仅需要后端协议，还需要稳定的 trace / telemetry / monitor / session drilldown / 前端活动展示合同，尤其是在引入中文短显示名、复合工具与迁移兼容之后。

## 目标
统一工具观测与前端展示读面，建立工具协议迁移后的验收闭环。

## 输出
- `tool-observability-and-frontend-presentation-contract` OpenSpec change
- 工具活动、trace、monitor、session drilldown 的统一展示字段
- 中文短显示名与前端呈现规则
- 复合工具、部分成功、被拒绝、失败的展示合同
- 工具协议迁移后的验收矩阵

## 范围边界
- 本卡不直接定义首批工具协议本身
- 本卡不重做整个前端信息架构
- 本卡主要收口读面、展示与验收口径

## 验收标准
- 工具活动、trace、monitor 与 session drilldown SHALL 共享稳定工具展示字段
- 中文短显示名 SHALL 被定义为展示层元数据，并进入前端工具活动读面
- 复合工具与部分成功/失败/审批拒绝路径 SHALL 有稳定展示合同
- 至少完成一轮独立 spec review，并根据采纳意见优化文档

## 当前进展
- 已完成实现、测试、实现态审核与一轮采纳调优
- 已完成 OpenSpec strict validate，并已将 delta spec 同步到 canonical spec
- 已完成 change 归档与任务系统收口

## 下一步动作
已完成，无后续动作；后续若扩展更多工具展示读面，应以新 change 承接。

## 当前卡点
- 暂无。当前已完成归档与收口。

## 断点续跑提示
继续前先看：

- [telemetry.rs](/C:/Users/HUAWEI/Documents/pony-agent/crates/pony-agent-core/src/agent/telemetry.rs)
- [control_plane.rs](/C:/Users/HUAWEI/Documents/pony-agent/crates/pony-agent-core/src/agent/control_plane.rs)
- [01_TASK_BOARD.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/01_TASK_BOARD.md)
