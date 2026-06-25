## MODIFIED Requirements

### Requirement: Provider IO SHALL use asynchronous network boundaries
Pony Agent 的 provider 网络调用 SHALL 使用异步 IO 边界，而不是同步阻塞 HTTP client。

#### Scenario: A provider followup stream is executed
- **WHEN** runtime 触发 provider followup streaming 请求
- **THEN** 该请求 SHALL 通过 async network client 执行
- **AND** SHALL NOT 通过 blocking HTTP client 占用执行线程直到响应结束

### Requirement: Tools HTTP IO SHALL also use asynchronous network boundaries
`tools.rs` 中涉及 HTTP 网络调用的路径（如 `web_fetch_url`）SHALL 从 `reqwest::blocking` 迁移至 async `reqwest`。

#### Scenario: A tool fetches a web URL
- **WHEN** agent 调用 `web_fetch_url` 或其他 HTTP tool
- **THEN** 该请求 SHALL 通过 async network client 执行
- **AND** SHALL NOT 在 async task 主路径上做同步阻塞

### Requirement: Cargo dependency SHALL be updated to reflect blocking removal
`pony-agent-core/Cargo.toml` SHALL 在完成 migration 后移除 `reqwest` 的 `blocking` feature。

#### Scenario: Post-migration dependency audit
- **WHEN** provider.rs 和 tools.rs 均已迁移至 async reqwest
- **THEN** `pony-agent-core/Cargo.toml` 中 `reqwest` 依赖 SHALL 移除 `blocking` feature
- **AND** `cargo check` / `cargo test` SHALL 在无 `blocking` 特性下通过

### Requirement: Provider cancellation semantics SHALL be preserved in async model
async 化后 provider 的 cancellation、timeout、retry 语义 SHALL 使用 async-native 实现而非包裹 blocking 调用。

#### Scenario: A provider request times out
- **WHEN** provider 请求超过配置的超时时间
- **THEN** SHALL 通过 async timeout（`tokio::time::timeout` 等价）触发取消
- **AND** SHOULD NOT 依赖 `std::thread::spawn` + `join` 模拟超时

### Requirement: Blocking-dependent tests SHALL be migrated
原有依赖 `reqwest::blocking` 的 `#[cfg(test)]` 模块 SHALL 被迁移到 async test 等价形式。

#### Scenario: Unit tests for provider HTTP
- **WHEN** 存在依赖 blocking HTTP client 的单元测试
- **THEN** 这些测试 SHALL 被更新为使用 async HTTP mock 或 test server
- **AND** 旧的 blocking test helper SHALL 被移除

### Requirement: Deprecated blocking HTTP code SHALL be removed
`reqwest::blocking` 的所有 import 和引用 SHALL 在 migration 完成后被彻底清理。

#### Scenario: Cleanup pass
- **WHEN** provider.rs 和 tools.rs 均完成 async 迁移
- **THEN** 所有 `use reqwest::blocking::*` 语句 SHALL 被移除
- **AND** 所有返回 `reqwest::blocking::Response` 的函数签名 SHALL 被更新
- **AND** 无 dead code 残留
