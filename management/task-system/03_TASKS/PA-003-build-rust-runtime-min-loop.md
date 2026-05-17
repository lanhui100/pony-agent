# PA-003 实现 Rust 单轮 runtime 骨架

## 状态

- Status: `Done`
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

已完成：

- 定义 `TurnInput`
- 定义 `TurnResult`
- 增加 Rust `run_turn()` 命令入口
- 打通前端输入到 Rust 结构化回包
- 前端可展示 phase、trace、tool activity、session summary
- `cargo check` 通过

## 下一步动作

转入 `PA-004`，开始定义 provider 与 tool 抽象，并让 `run_turn()` 承接真实模型调用。

## 当前卡点

- 无

## 断点续跑提示

继续前先看：

- `docs/architecture/runtime.md`
- `docs/guides/rust-agent.md`
- `docs/learning/0007-first-run-turn-implementation.md`
