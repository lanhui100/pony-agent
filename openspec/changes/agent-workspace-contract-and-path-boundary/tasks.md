# Tasks: Agent Workspace Contract And Path Boundary

## 1. Spec And Task-System Alignment

- [x] 1.1 在 `PA-046` 任务卡中收口 workspace 合同的目标、范围和验收标准
- [x] 1.2 为本卡补 proposal / design / delta spec / tasks 草案

## 2. Workspace Contract

- [x] 2.1 定义 `WorkspaceContext` 或等价正式结构
- [x] 2.2 定义 runtime 中 workspace 注入的正式边界
- [x] 2.3 定义 workspace 作为 tool / trace / permission 共同上下文的合同

## 3. Path Semantics

- [x] 3.1 定义相对路径解析规则
- [x] 3.2 定义绝对路径 canonicalize 与越界拒绝规则
- [x] 3.3 定义 display path 规则
- [x] 3.4 定义 path repair 的边界、停止条件与冲突返回规则

## 4. Host Seam

- [x] 4.1 定义 host workspace policy 的注入边界
- [x] 4.2 明确 host 不得复制工具层路径判断逻辑
- [x] 4.3 明确执行类工具的 workspace 默认工作目录边界

## 5. Review And Validation

- [x] 5.1 调用至少一个智能体做独立 spec review
- [x] 5.2 采纳合理意见并优化一轮 proposal / design / spec / tasks
- [x] 5.3 运行 OpenSpec change 校验
- [x] 5.4 在任务卡中同步 spec 状态与下一步动作
