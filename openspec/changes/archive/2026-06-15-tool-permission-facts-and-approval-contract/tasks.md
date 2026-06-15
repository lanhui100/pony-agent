# Tasks: Tool Permission Facts And Approval Contract

## 1. Spec And Task-System Alignment

- [x] 1.1 在 `PA-047` 任务卡中收口权限合同的目标、范围和验收标准
- [x] 1.2 为本卡补 proposal / design / delta spec / tasks 草案

## 2. Permission Facts

- [x] 2.1 定义统一 `ToolPermissionFacts` 合同
- [x] 2.2 定义工具定义层与权限决策层的边界

## 3. Failure Semantics

- [x] 3.1 定义 `permission_denied`
- [x] 3.2 定义 `approval_required`
- [x] 3.3 定义 `out_of_scope`

## 4. Cross-Source Reuse

- [x] 4.1 明确 builtin tool 的权限事实来源
- [x] 4.2 明确 capability-backed tool 的权限事实保真要求
- [x] 4.3 明确 skill/composite tool 的保守聚合规则

## 5. Review And Validation

- [x] 5.1 调用至少一个智能体做独立 spec review
- [x] 5.2 采纳合理意见并优化一轮 proposal / design / spec / tasks
- [x] 5.3 运行 OpenSpec change 校验
- [x] 5.4 在任务卡中同步 spec 状态与下一步动作
