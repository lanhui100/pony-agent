## MODIFIED Requirements

### Requirement: Turn execution SHALL support per-session concurrent async tasks
Pony Agent 在多会话场景下 SHALL 允许不同 session 的 turn 通过独立 async task 并发运行。

#### Scenario: Two different sessions submit turns concurrently
- **WHEN** 两个不同 session 几乎同时开始 turn
- **THEN** runtime SHALL 为它们创建独立 async task
- **AND** 一个 session 的 turn SHALL NOT 因另一个 session 的 turn 正在运行而被结构性串行阻塞

### Requirement: Turn cancellation SHALL be supported in async task model
在 per-session async task 模型下，turn 取消 SHALL 通过 async cancellation 机制实现。

#### Scenario: A running turn is cancelled
- **WHEN** 用户在前端取消正在执行的 turn
- **THEN** 对应的 async task SHALL 被优雅取消（cooperative cancellation）
- **AND** 正在进行的 provider 网络请求 SHALL 被关闭
- **AND** terminal event SHALL 仍然正确发送到前端
- **AND** session state SHALL 回滚到 turn 开始前的一致状态

### Requirement: Terminal cleanup SHALL preserve event contract
async task 模型下的 terminal cleanup SHALL 保持与现有前端事件合同兼容。

#### Scenario: Turn completes with terminal event
- **WHEN** 一个 turn 在 async task 中到达 terminal 状态
- **THEN** `turn:output_end` 与 terminal 事件 SHALL 仍按现有合同发送
- **AND** task 退出前 SHALL 完成所有 cleanup（包括释放 session-local state）

### Requirement: Multi-session concurrent execution SHALL have regression test coverage
per-session async task 模型 SHALL 有集成测试验证真正的多 session 并发执行。

#### Scenario: Integration test for concurrent turns
- **WHEN** session A 提交一个长时间 mock turn（如模拟 5 秒 provider 延迟）
- **AND** session B 在 session A 执行期间提交另一个 turn
- **THEN** session B 的 turn SHALL 在 session A 完成之前就开始执行
- **AND** 两个 session 的事件流 SHALL 不互相串扰

### Requirement: Error isolation between concurrent turns
不同 session 的 async turn task SHALL 彼此错误隔离。

#### Scenario: One session's turn panics
- **WHEN** session A 的 turn task 因内部错误 panic
- **THEN** session B 的 turn SHALL NOT 受影响
- **AND** session A 的 panic SHALL 被 `catch_unwind` 捕获并转为可预测的错误事件

### Requirement: Deprecated `spawn_blocking` turn model SHALL be removed
旧的 `tauri::async_runtime::spawn_blocking` + `Mutex<AgentRuntime>` 模式 SHALL 被彻底移除。

#### Scenario: Cleanup after PA-068 completion
- **WHEN** per-session async task 模型达到稳态
- **THEN** `tauri_adapter.rs` 中的 `spawn_turn_stream` 和 `spawn_graph_run_stream` SHALL 不再使用 `spawn_blocking`
- **AND** `lib.rs` 中的 frontend diagnostics `spawn_blocking` 调用 SHALL 通过 PA-067 的 blocking helper 统一管理
