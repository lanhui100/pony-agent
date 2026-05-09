# Rust 智能体开发指南

## 开发顺序

1. 定义基础数据结构
2. 定义 provider trait
3. 定义 tool trait
4. 实现单轮 runtime
5. 接入 UI 调试

## 基础数据结构建议

- `UserInput`
- `AssistantMessage`
- `ToolCall`
- `ToolResult`
- `TurnResult`
- `RuntimeState`

## 设计建议

- 用数据结构表达领域，不要把领域逻辑散进 UI
- 用 trait 隔离具体 provider
- 用明确状态枚举表达执行阶段
- 让每次 turn 都可追踪

## 第一批不要急着做的事

- 复杂记忆系统
- 多代理并发
- 大而全的工具集
- 复杂权限系统

这些可以后置，先把最小闭环做扎实。
