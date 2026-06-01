# 重构阶段计划

## Phase 0：骨架阶段

目标：

- 整理目录
- 保留 Hermes 参考区
- 建立 Tauri + Rust 基础骨架

状态：

- 已完成

## Phase 1：前端调试台

目标：

- 接入 Vue 3
- 接入 Pinia
- 做出智能体调试台基本布局

状态：

- 下一步执行

## Phase 2：单轮 Runtime

目标：

- 实现 `run_turn()`
- 接入一个 provider
- 在 UI 中可看到状态流

## Phase 3：工具调用

目标：

- 建立 ToolRouter
- 支持结构化工具调用
- 在 UI 中显示工具调用日志

## Phase 4：会话与记忆

目标：

- 多轮会话
- 摘要
- 本地持久化

## Phase 5：Graph 编排

目标：

- 显式状态机
- 更清晰的推理 / 工具 / 观察流程

## Phase 6：高级能力

目标：

- 子代理
- 更丰富工具生态
- 更强调试能力

## Phase 7：行业 Workflow 扩展

目标：

- 在 agent harness 主线完成并稳定后，扩展用户自定义 `workflow` 模式
- 支持面向行业场景的流程编排，而不只是不受约束的 agentic 模式
- 允许用户定义节点、分支、审批、重试、人工介入与可调用能力边界
- 复用既有 graph / runtime / checkpoint / trace / resume 底座，形成可审计、可恢复、可复用的流程执行能力
