## ADDED Requirements

### Requirement: Read-plane access SHALL be structurally decoupled from full turn execution
Pony Agent 宿主读面在架构上 SHALL 不再与整轮 turn 执行共用同一全局运行时锁边界。

#### Scenario: A turn is actively executing
- **WHEN** 某个 session 的 turn 正在执行
- **THEN** `list_sessions`、`load_session_runtime_view` 与 `load_retrieved_context` SHALL 仍可通过独立读路径执行
- **AND** SHALL NOT 因整轮 turn 生命周期持锁而被结构性阻塞

### Requirement: `AgentRuntime` ownership SHALL be explicitly stratified
`AgentRuntime` / `HostControlPlane` 的状态所有权 SHALL 分层为 session-local、run-local 与 global 三类。

#### Scenario: Ownership mapping is drawn
- **WHEN** 完成当前 runtime state ownership 图
- **THEN** `SessionStore` SHALL 标记为 session-local 域
- **AND** `GraphRunStore` SHALL 标记为 run-local 域
- **AND** provider/tool/planner 执行状态 SHALL 不再隐式捆绑在同一全局锁下

### Requirement: Error isolation during ownership split
ownership 切分过程中，若读路径因锁边界变化而失败，SHALL 优雅降级而非阻塞前端交互。

#### Scenario: Read-plane encounters a stale lock boundary
- **WHEN** 读路径访问尚在切分中的 state
- **THEN** SHALL 返回可预测的错误状态而非 panic
- **AND** SHALL 在前端以缓存数据兜底而非白屏

### Requirement: Test coverage for concurrent read-while-turn
ownership 切分后 SHALL 有集成测试验证并发场景下的读路径可用性。

#### Scenario: Integration test for concurrent access
- **WHEN** thread A 执行全量 turn（含 mock provider 延迟）
- **AND** thread B 并发调用 `list_sessions` 和 `load_session_runtime_view`
- **THEN** read-plane SHALL 成功返回数据
- **AND** SHALL NOT 因 turn 持锁而被阻塞超过阈值

### Requirement: Deprecated locking patterns SHALL be removed
移除旧的全局 `Mutex<AgentRuntime>` 模式代码，清理关联的 dead code 和跳板函数。

#### Scenario: Cleanup after ownership split
- **WHEN** 新的 ownership 模型达到稳态
- **THEN** 所有绕过新模型访问旧 `Mutex<AgentRuntime>` 的代码路径 SHALL 被移除
- **AND** `control_plane.rs` 中不再持有 `Mutex<AgentRuntime>` 结构
- **AND** 相关的 `#[cfg(test)]` 测试 SHALL 更新为不依赖旧锁模式
