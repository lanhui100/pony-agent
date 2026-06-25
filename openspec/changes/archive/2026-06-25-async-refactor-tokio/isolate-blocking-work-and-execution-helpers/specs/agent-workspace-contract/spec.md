## ADDED Requirements

### Requirement: Blocking work SHALL be isolated behind explicit helper boundaries
Pony Agent runtime 中无法异步化的工作 SHALL 通过统一 helper 边界执行，而不是混入 async task 主路径。

#### Scenario: A runtime path needs blocking or CPU-heavy work
- **WHEN** 某段逻辑必须执行 blocking IO 或 CPU 密集处理
- **THEN** 该逻辑 SHALL 通过显式 helper 边界执行
- **AND** SHALL NOT 直接混入 async task 主路径

### Requirement: Blocking boundary inventory SHALL cover all major categories
所有 blocking 路径 SHALL 被显式盘点并分类。

#### Scenario: Blocking work inventory
- **WHEN** 开始 blocking 盘点
- **THEN** 以下类别 SHALL 被覆盖：
  - 文件 IO（`std::fs`、`tokio::fs`、`File` 操作）
  - SQLite（`rusqlite` 同步调用）
  - 序列化/反序列化（大量 JSON 处理）
  - CPU 密集计算（重计算、加密、压缩）
  - 进程调用（`std::process::Command`）
- **AND** 每个类别 SHALL 标注在代码库的分布位置

### Requirement: Helper API SHALL support both `spawn_blocking` and `block_in_place` patterns
统一的 blocking helper SHALL 提供对 `spawn_blocking` 和 `block_in_place` 两种策略的支持。

#### Scenario: Choosing execution strategy
- **WHEN** blocking 工作在独立的 blocking thread pool 上执行
- **THEN** `spawn_blocking` SHALL 是默认策略
- **AND** `block_in_place` SHOULD 仅用于已知不会长时间持锁的路径

### Requirement: Migration from scattered blocking calls to helper SHALL be explicit
现有的散落 `spawn_blocking` 和同步调用 SHALL 逐步替换为统一 helper。

#### Scenario: Hot path migration
- **WHEN** 替换 `tauri_adapter.rs` 和 `lib.rs` 中 15 处 `spawn_blocking` 调用
- **THEN** 每个替换点 SHOULD 按"盘点 → 分类 → 迁移 → 测试"四步执行
- **AND** `tauri_adapter.rs` 的 `spawn_turn_stream` 和 `spawn_graph_run_stream` SHOULD 标记为 PA-068 接管（而非在此完全重写）

### Requirement: Blocking work changes SHALL coordinate with PA-068 on tauri_adapter
PA-067 对 `tauri_adapter.rs` 的修改 SHALL 仅替换 blocking 执行方式，不改变执行语义，以确保 PA-068 能在此基础上替换为 async task 模型。

#### Scenario: Transition contract with PA-068
- **WHEN** PA-067 修改 `tauri_adapter.rs`
- **THEN** `spawn_turn_stream` 和 `spawn_graph_run_stream` 的函数签名 SHALL 保持外向前端兼容
- **AND** PA-067 的 blocking helper SHALL 提供 `into_async()` / `as_spawn()` 等转换接口供 PA-068 无缝迁移
- **AND** 两个任务共享的中间态 SHALL 有明确文档记录

### Requirement: Deprecated blocking patterns SHALL be removed
旧的散落 `spawn_blocking` / 同步调用模式 SHALL 被统一 helper 替代后彻底移除。

#### Scenario: Cleanup after PA-067 + PA-068 completion
- **WHEN** PA-068 完成 per-session async turn task 模型
- **THEN** 旧的不通过统一 helper 的 `spawn_blocking` 调用 SHALL 被移除
- **AND** `tauri_adapter.rs` 和 `lib.rs` 中的 `std::sync::Mutex` SHALL 在适当位置替换为 `tokio::sync::Mutex`
