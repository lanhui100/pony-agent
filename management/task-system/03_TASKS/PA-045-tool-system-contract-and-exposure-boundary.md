# PA-045 收口工具系统协议、暴露策略与结果合同

## 状态
- Status: `Done`
- Priority: `P1`
- Owner: `Codex`

## OpenSpec Change
- 已归档：
  [2026-06-15-tool-system-contract-and-exposure-boundary](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/archive/2026-06-15-tool-system-contract-and-exposure-boundary>)

## Delta Spec
- 已同步并归档：
  `openspec/changes/archive/2026-06-15-tool-system-contract-and-exposure-boundary/specs/tool-system-contract/spec.md`

## Canonical Spec
- 已同步到：
  `openspec/specs/tool-system-contract/spec.md`

## Spec 状态
- Proposal: `validated`
- Spec: `validated`
- Design: `validated`
- Tasks: `validated`

## 背景
当前 Pony Agent 已经具备 `ToolRouter / ToolCall / ToolResult / ToolPlan / capability bridge / telemetry` 等工具执行底座，但模型可见工具名、内部执行原语名、结果结构、失败语义、工具分类与暴露策略还没有被统一写成正式合同。

现状风险主要有：

- 现有 `workspace_*` 工具名更像内部实现名，不适合作为长期模型协议与前端展示名
- `ToolResult`、工具失败态、权限事实与复合工具结果仍有进一步收口空间
- capability / skill / builtin / composite tool 之间尚未建立统一的工具协议层
- 前端工具活动展示、trace、planner 与 runtime 目前消费的工具口径还没有统一 spec 真相源

## 目标
把 Pony Agent 的工具系统正式收口为“可被模型、runtime、planner、trace、control-plane 与前端共同消费的一套统一协议”，为后续 workspace、权限、首批工具面与迁移实现提供母合同。

## 输出
- `tool-system-contract-and-exposure-boundary` OpenSpec change
- 统一 `ToolDefinition / ToolCall / ToolResult / ToolFailureKind` 合同
- 工具分类体系与暴露策略合同
- 模型可见工具名与内部执行原语名分层规则
- 中文短显示名与前端展示元数据最小合同
- 面向后续 `PA-046 ~ PA-049` 的依赖边界说明

## 范围边界
- 本卡不直接实现所有首批工具，只定义协议与暴露边界
- 本卡不直接完成 workspace 路径策略或权限审批流程细节，它们分别由后续任务承接
- 本卡不重写 capability bridge / skills registry 的主体逻辑，只要求它们复用统一工具合同
- 本卡不要求一次完成所有前端展示；只要求定义展示所需的最小稳定字段

## 验收标准
- 系统 SHALL 明确区分模型可见工具名与内部执行原语名
- 系统 SHALL 为工具定义统一的 `ToolDefinition` 合同，至少覆盖名称、描述、输入、输出、分类、暴露策略与显示元数据
- 系统 SHALL 为工具调用统一 `ToolCall` 合同，至少覆盖 canonical tool name、arguments、plan 与 display metadata
- 系统 SHALL 为工具结果统一 `ToolResult` 合同，至少覆盖 `status / summary / data / error / artifacts / duration_ms`
- 系统 SHALL 明确结构化 `ToolFailureKind`，不得仅依赖自由文本错误
- 系统 SHALL 定义至少 `read / search / write / execute / plan / interactive / composite / external` 八类工具分类
- 系统 SHALL 定义至少 `model-visible / internal / deferred` 三类暴露策略
- capability / skill / builtin / composite tool SHALL 复用同一套工具结果与失败语义，而不是各自发明第二套协议
- 架构文档与 OpenSpec SHALL 明确中文短显示名属于展示层元数据，而不是底层工具标识
- 至少完成一轮独立 spec review，并根据采纳意见优化文档

## 当前进展
- 已完成实现、测试、实现态审核与一轮采纳调优
- 已完成 OpenSpec strict validate，并已将 delta spec 同步到 canonical spec
- 已完成 change 归档与任务系统收口

## 下一步动作
已完成，无后续动作；后续若继续演进工具协议，应以新 change 承接。

## 当前卡点
- 暂无。当前已完成归档与收口。

## 断点续跑提示
继续前先看：

- [01_TASK_BOARD.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/01_TASK_BOARD.md)
- [tools.rs](/C:/Users/HUAWEI/Documents/pony-agent/crates/pony-agent-core/src/agent/tools.rs)
- [runtime.rs](/C:/Users/HUAWEI/Documents/pony-agent/crates/pony-agent-core/src/agent/runtime.rs)
- [capability_bridge.rs](/C:/Users/HUAWEI/Documents/pony-agent/crates/pony-agent-core/src/agent/capability_bridge.rs)
- [telemetry.rs](/C:/Users/HUAWEI/Documents/pony-agent/crates/pony-agent-core/src/agent/telemetry.rs)
