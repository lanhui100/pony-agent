# Proposal: Adopt per-session async turn tasks

## Why

在 ownership 拆分、provider async 化和 blocking boundary 收口后，Pony Agent 才真正具备按 session / run 维度并发执行 turn 的条件。本 change 负责把当前 blocking spawn + global runtime lock 模式切到 async task 模型。

## Scope

- turn / graph run 执行切到 async task
- per-session / per-run task ownership 落地
- 继续复用事件系统向前端推送状态

## Non-goals

- 前端 transcript 渲染优化
- 非 turn 路径的全仓 async 化

## Cleanup

- 移除 `tauri_adapter.rs` 中旧的 `spawn_blocking` 版 turn 执行函数
- 移除 `spawn_blocking` 在核心 turn 路径的所有残余引用
- 更新集成测试验证多 session 并发执行
