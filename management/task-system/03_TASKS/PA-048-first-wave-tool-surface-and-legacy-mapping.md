# PA-048 首批基础工具面与旧工具映射收口

## 状态
- Status: `In Progress`
- Priority: `P1`
- Owner: `Codex`

## OpenSpec Change
- 活跃路径：
  [first-wave-tool-surface-and-legacy-mapping](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/first-wave-tool-surface-and-legacy-mapping>)

## Delta Spec
- 活跃路径：
  `openspec/changes/first-wave-tool-surface-and-legacy-mapping/specs/first-wave-tool-surface/spec.md`

## Canonical Spec
- 待实现并归档后同步到：
  `openspec/specs/first-wave-tool-surface/spec.md`

## Spec 状态
- Proposal: `validated`
- Spec: `validated`
- Design: `validated`
- Tasks: `validated`

## 背景
当前 `workspace_*` 工具提供了真实底层能力，但工具名、展示名与模型协议仍偏内部实现导向，不适合作为长期第一层工具面。

## 目标
定义首批基础工具面，收口新旧工具映射、复合工具策略与兼容路径。

## 输出
- `first-wave-tool-surface-and-legacy-mapping` OpenSpec change
- 首批模型可见工具清单
- 新旧工具映射与兼容策略
- `ToolPlan` 在复合工具中的暴露规则
- 内部原语与模型工具面的分层说明

## 范围边界
- 本卡不要求一次实现 Browser / MCP / Workflow 等扩展工具
- 本卡不重写现有底层工具实现，只要求定义首批工具面与映射

## 验收标准
- 系统 SHALL 定义首批模型可见工具面
- 现有 `workspace_*` 工具 SHALL 有明确的内部原语或兼容别名归属
- 复合工具与显式 `ToolPlan` SHALL 有稳定的产品层暴露规则
- 至少完成一轮独立 spec review，并根据采纳意见优化文档

## 当前进展
- 已创建 OpenSpec change：
  [first-wave-tool-surface-and-legacy-mapping](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/first-wave-tool-surface-and-legacy-mapping>)
- 已完成本卡第一版 OpenSpec artifact 草案：
  - `proposal.md`
  - `design.md`
  - `tasks.md`
  - `specs/first-wave-tool-surface/spec.md`
- 已完成至少一轮独立智能体 spec review，并采纳意见收紧一轮文档
- 已沉淀 review 记录：
  - [2026-06-13-pa048-spec-review.md](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/02_REVIEWS/2026-06-13-pa048-spec-review.md>)
- 已完成 OpenSpec strict validate：
  - `npm run openspec -- validate first-wave-tool-surface-and-legacy-mapping --type change --strict --json --no-interactive`

## 下一步动作
将本卡冻结的产品级工具清单继续输入实现排序与前端展示合同落地。

## 当前卡点
- 暂无。`PA-045 ~ PA-047` 已可作为本卡前置母合同使用。

## 断点续跑提示
继续前先看：

- [tools.rs](/C:/Users/HUAWEI/Documents/pony-agent/crates/pony-agent-core/src/agent/tools.rs)
- [planner.rs](/C:/Users/HUAWEI/Documents/pony-agent/crates/pony-agent-core/src/agent/planner.rs)
- [telemetry.rs](/C:/Users/HUAWEI/Documents/pony-agent/crates/pony-agent-core/src/agent/telemetry.rs)
