# Tasks: Tool System Contract And Exposure Boundary

## 1. Spec And Task-System Alignment

- [x] 1.1 更新 `PA-045` 任务卡，明确工具系统协议、暴露策略与结果合同的目标、范围和验收标准
- [x] 1.2 将工具系统整体规划拆成 `PA-045 ~ PA-049` 五张连续任务卡
- [x] 1.3 为本卡创建 OpenSpec change：`tool-system-contract-and-exposure-boundary`
- [x] 1.4 完成 proposal / design / delta spec / tasks 草案

## 2. Tool Contract

- [x] 2.1 定义统一 `ToolDefinition` 合同
- [x] 2.2 定义统一 `ToolCall` 合同
- [x] 2.3 定义统一 `ToolResult` 合同
- [x] 2.4 定义统一 `ToolFailureKind` 合同

## 3. Taxonomy And Exposure

- [x] 3.1 定义统一 `ToolKind` 分类体系
- [x] 3.2 定义统一 `ToolExposure` 暴露策略
- [x] 3.3 明确模型可见工具名与内部执行原语名的分层规则

## 4. Cross-Source Reuse

- [x] 4.1 明确 builtin tool 如何复用统一合同
- [x] 4.2 明确 capability-backed tool 如何复用统一合同
- [x] 4.3 明确 skill-composed tool 如何复用统一合同
- [x] 4.4 明确 composite tool 与 `ToolPlan` 的统一表达规则

## 5. Presentation Metadata

- [x] 5.1 定义中文短显示名与前端展示元数据字段
- [x] 5.2 明确展示层元数据与底层工具标识的分层规则

## 6. Review And Validation

- [x] 6.1 调用至少一个智能体做独立 spec review
- [x] 6.2 采纳合理意见并优化一轮 proposal / design / spec / tasks
- [x] 6.3 运行 OpenSpec change 校验
- [x] 6.4 在任务卡中同步 spec 状态与下一步动作
