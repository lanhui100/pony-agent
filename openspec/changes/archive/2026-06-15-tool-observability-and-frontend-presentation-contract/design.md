# Design: Tool Observability And Frontend Presentation Contract

## 背景

当前 Pony Agent 已经有 trace、telemetry、monitor、session drilldown 与前端工具活动展示基础，但这些读面还没有围绕新工具系统合同正式收口。

后续一旦引入：

- 中文短显示名
- 首批产品级工具面
- 复合工具与 `ToolPlan`
- 统一权限事实

就必须明确这些信息如何被前端与观测读面消费，否则后面每一层都会各自做格式兼容和二次推导。

## 设计目标

1. 统一工具可观测读面
2. 统一前端主要展示字段
3. 统一复合工具与失败态展示
4. 建立迁移后的验收矩阵

## 非目标

- 不重做整个前端信息架构
- 不在本 change 中重新定义工具协议本身
- 不要求本轮一次把所有页面都改完

## 统一展示字段

前端与观测读面至少应共享：

- `name`
- `canonical_tool_name`
- `display_name_zh`
- `kind`
- `status`
- `summary`
- `duration_ms`
- `error`
- `artifacts`
- `child_results`

补充约束：

- `canonical_tool_name` 是所有读面的稳定聚合主键
- `display_name_zh` 是产品级主展示标签的首选字段
- `status` 应表达统一状态机，而不是由页面自行推导
- `child_results` 与 `artifacts` 是不同容器，前者表达执行层级，后者表达附件/引用对象

## 展示规则

### 中文短显示名

- 前端工具活动主标签优先使用 `display_name_zh`
- 若缺失，再退回英文名
- 底层 trace/raw data 仍保留稳定英文标识
- `display_name_zh` 应作为正式展示元数据字段进入读面，而不是由前端临时翻译

### 复合工具

- 允许主卡片显示聚合摘要
- 子步骤通过统一 `child_results` 展示
- 不要求前端直接理解底层 primitive 名
- 父结果状态应能独立于子结果状态表达
- 若父结果为部分成功，应允许继续展开查看成功/失败/待审批子步骤

### 状态展示

至少需要统一这些状态：

- 成功
- 部分成功
- 失败
- 被拒绝
- 待审批

状态建议来源：

- `success`
- `partial_success`
- `failed`
- `permission_denied`
- `approval_required`

约束：

- `permission_denied` 与 `approval_required` 不应被普通失败态吞并
- 父结果可为 `partial_success`，即使部分子步骤为 `failed / approval_required / permission_denied`

### 附件与引用对象

- `artifacts` 用于展示路径、片段、可视化对象或引用对象
- 不与 `child_results` 混用
- `artifacts` 应保留统一容器语义，至少能容纳路径、文本片段或引用对象
- `child_results` 至少应保留子步骤名称、状态、摘要与错误/权限结果的展示字段

## 验收矩阵

至少覆盖：

- 简单成功工具调用
- 复合工具调用
- 子步骤部分成功
- 权限拒绝/待审批
- 路径类附件对象展示
- 新旧工具迁移后的展示一致性

并补充：

- 旧 primitive 名触发时的产品级展示回退
- `display_name_zh` 缺失时的英文回退
- `artifacts` 非空且 `child_results` 为空时的展示一致性

## 与前置任务的关系

- `PA-045` 提供统一工具字段
- `PA-046` 提供 display path 规则
- `PA-047` 提供权限展示语义
- `PA-048` 提供首批产品级工具面
