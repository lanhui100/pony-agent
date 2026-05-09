# PA-004 定义 provider 与 tool 抽象

## 状态

- Status: `Backlog`
- Priority: `P1`
- Owner: `Codex`

## 目标

在 Pony Agent 中建立统一的模型与工具抽象，避免后续实现强耦合。

## 输出

- `Provider` trait
- `ToolRouter` trait
- `ToolCall` / `ToolResult`

## 验收标准

- Rust runtime 不直接依赖某个具体模型厂商
- 工具执行结果是结构化的

## 当前进展

尚未开始。

## 下一步动作

在 `PA-003` 基本跑通后开始设计。

## 当前卡点

- 当前优先级低于前端工作台和最小 runtime

## 断点续跑提示

开始前先复查：

- `docs/architecture/runtime.md`
- `docs/tauri-rust-refactor.md`
