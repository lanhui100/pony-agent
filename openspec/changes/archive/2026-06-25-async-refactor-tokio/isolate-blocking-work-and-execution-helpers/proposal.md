# Proposal: Isolate blocking work and execution helpers

## Why

异步 provider 只是并发基础的一部分。runtime 里仍存在本地文件 IO、SQLite、序列化和 CPU 密集逻辑，需要统一识别和隔离，避免 async task 又被同步工作重新拖住。

## Scope

- 盘点 blocking 与 CPU 密集边界
- 建立统一的 blocking execution helpers
- 替换散落的临时 blocking 调用

## Non-goals

- 本 change 不直接完成 provider async 化
- 本 change 不直接完成 per-session task 编排

## Transition contract

- 本 change 对 `tauri_adapter.rs` 的 turn 路径只替换 blocking 执行机制，不改变执行语义
- 提供 PA-068 可接管的 `into_async()` 转换接口
- 标注 `spawn_turn_stream` / `spawn_graph_run_stream` 为 PA-068 接管目标

## Cleanup

- 移除迁移后的散落 `spawn_blocking` 调用（不含 PA-068 接管路径）
- 更新依赖旧 blocking 模式的 `#[cfg(test)]` 测试
