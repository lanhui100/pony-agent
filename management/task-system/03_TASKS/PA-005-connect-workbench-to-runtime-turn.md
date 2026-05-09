# PA-005 把 Vue 工作台接入真实 turn 执行链路

## 状态

- Status: `Backlog`
- Priority: `P1`
- Owner: `Codex`

## 目标

让当前 Vue 工作台不再只展示静态占位信息，而是能驱动真实的 Rust turn 执行并回显结果。

## 输出

- 前端输入动作
- Tauri command 调用
- 返回结构化 turn 结果
- 面板中展示真实 phase、trace、message

## 验收标准

- 用户可以在工作台中发起一次 turn
- Rust 返回结果后，UI 面板同步更新
- 不需要先引入复杂多轮会话

## 当前进展

前端工作台骨架已完成，但仍基于 `health_check` 和预置数据。

## 下一步动作

待 `PA-003` 完成最小 runtime 闭环后接入。

## 当前卡点

- 依赖 `PA-003`

## 断点续跑提示

继续前先看：

- `src/stores/runtime.ts`
- `docs/architecture/frontend-workbench.md`
