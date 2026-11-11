# PA-002 设计运行时调试台结构

## 状态

- Status: `Done`
- Priority: `P0`
- Owner: `Codex`

## 目标

定义 Pony Agent 第一版 Tauri UI 工作台结构，使其适合观察 Rust 智能体运行时。

## 输出

- 工作台面板划分
- 状态流展示方案
- 与 Rust runtime 的交互边界

## 验收标准

- 界面结构可覆盖聊天、运行时、工具调用和轨迹展示
- 可直接服务后续 `run_turn()` 接入

## 当前进展

已完成：

- 明确以 Tauri UI 为主测试入口
- 落地聊天、运行时状态、Graph/工具轨迹、策略面板四块结构
- 新增前端工作台架构文档

## 下一步动作

将工作台从静态骨架升级为真实 runtime 调试面板。

## 当前卡点

- 无

## 断点续跑提示

继续前先看：

- `docs/guides/frontend.md`
- `docs/decisions/0002-tauri-ui-over-tui.md`
- `docs/architecture/frontend-workbench.md`
