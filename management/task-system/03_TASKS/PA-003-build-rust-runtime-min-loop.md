# PA-003 实现 Rust 单轮 runtime 骨架

## 状态

- Status: `Ready`
- Priority: `P0`
- Owner: `Codex`

## 目标

实现 Pony Agent 的第一条最小 Rust 智能体链路：

- 接收输入
- 调用 provider
- 返回结果
- 向 UI 暴露状态

## 输出

- `run_turn()` 最小实现
- `RuntimeState` 结构
- 与 UI 的命令边界

## 验收标准

- 前端能发起一次 turn
- Rust 能返回结构化结果
- UI 能看到状态变化

## 当前进展

目前 Rust 侧仅有 `health_check`。

## 下一步动作

先定义最小领域对象：

- `UserInput`
- `AssistantMessage`
- `TurnResult`
- `RuntimeState`

## 当前卡点

- 依赖 `PA-001` 和 `PA-002` 完成最小调试台结构

## 断点续跑提示

继续前先看：

- `docs/architecture/runtime.md`
- `docs/guides/rust-agent.md`
