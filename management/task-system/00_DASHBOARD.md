# Pony Agent Dashboard

## 项目状态

- 项目：`Pony Agent`
- 类型：学习模式智能体重构项目
- 当前主线：完成 `Vue + Pinia + Tauri UI` 调试台骨架，并开始打通 Rust 单轮 runtime
- 当前阶段：`Phase 2 / Runtime Loop`
- 总体状态：`In Progress`

## 当前重点

1. 打通 Rust `run_turn()` 最小闭环
2. 将工作台和真实运行时状态连接起来
3. 定义 provider 与 tool 抽象

## 当前任务摘要

- `PA-003`：实现 Rust 单轮 runtime 骨架
- `PA-004`：定义 provider 与 tool 抽象
- `PA-005`：把 Vue 工作台接入真实 turn 执行链路

## 当前断点

- 前端已切换到 `Vue + Pinia`
- 工作台已有运行时状态、轨迹和工具预演面板
- Rust runtime 仍只有 health check，尚未具备 turn 执行能力

## 下一步最小动作

先执行 `PA-003`：

- 定义 `run_turn()` 领域对象
- 增加前后端真实 turn 命令边界
- 让工作台可展示真实执行状态

## 关联入口

- 项目记忆：`AGENT.md`
- 文档索引：`docs/INDEX.md`
- 任务板：`01_TASK_BOARD.md`
