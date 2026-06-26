## ADDED Requirements

### Requirement: Provider retry SHALL follow a three-layer architecture with escalation contract
Pony Agent SHALL 将 provider retry 语义拆分为三层：request-level retry、phase-level fallback、turn-level retry，三层之间通过显式 escalation contract 连接，避免隐式异常传播。

#### Scenario: Timeout triggers request-level retry
- **GIVEN** `decision` 或 `followup_sync` 调用遭遇 timeout 类错误
- **WHEN** 当前 stream 尚未发出任何 delta
- **THEN** `ProviderRetryPolicy` SHALL 判定为 `Retry`，以指数退避等待后重试
- **AND** 重试次数与总耗时受 `RetryBudget` 约束

#### Scenario: Request-level retry exhausted escalates to phase-level fallback
- **GIVEN** `followup_stream` 的 request-level retry 因 budget 耗尽而无法继续
- **WHEN** stream 状态处于 `NoDelta` 或 `ReasoningOnly`
- **THEN** 系统 SHALL 自动降级为 `followup_sync`（sync fallback）
- **AND** sync fallback 自身仍持有独立 request-level retry 预算

#### Scenario: Phase-level fallback exhausted escalates to turn-level retry
- **GIVEN** sync fallback 也因 budget 耗尽或非重试性错误而失败
- **WHEN** 无可用 fallback 路径
- **THEN** 系统 SHALL 将失败上报给 turn runtime
- **AND** turn runtime SHALL 将终态设为 `failed`

#### Scenario: Turn-level retry requires explicit control-plane action
- **GIVEN** 一个 turn 因 provider 失败而进入 `failed` 终态
- **WHEN** 用户或控制面希望重新执行该 turn
- **THEN** SHALL 通过 `start_graph_run_stream` 配合显式 `startReason`（如 `retry_failed_turn`）发起
- **AND** SHALL NOT 自动静默重启

#### Scenario: Escalation carries structured reason
- **GIVEN** 某层决策为 Escalate 或 Fallback
- **WHEN** 该决策传递到上一层
- **THEN** SHALL 附带机器可读的 `reason` 字符串
- **AND** 接收层 SHALL 将该 reason 写入 telemetry 日志

### Requirement: Provider error classification SHALL be structured and explicit
所有 provider 响应错误 SHALL 通过 `classify()` 归入四个语义类别之一，而非仅通过 timeout 字符串做二元判断。

#### Scenario: Timeout errors are classified as TransientRetryable
- **GIVEN** 错误消息包含 `type=timeout`、`timeout`、`timed out` 或 `deadline has elapsed`
- **WHEN** `ProviderRetryPolicy.classify()` 被调用
- **THEN** SHALL 返回 `FailureKind::TransientRetryable { .. }`

#### Scenario: Rate-limit errors are classified as TransientRetryable
- **GIVEN** 错误消息包含 `429`、`rate limit` 或 `rate_limit`
- **WHEN** `classify()` 被调用
- **THEN** SHALL 返回 `FailureKind::TransientRetryable { .. }`

#### Scenario: 5xx errors are classified as TransientRetryable
- **GIVEN** 错误消息包含 `502`、`503`、`504` 或 `408`
- **WHEN** `classify()` 被调用
- **THEN** SHALL 返回 `FailureKind::TransientRetryable { .. }`

#### Scenario: Connection errors are classified as TransientRetryable
- **GIVEN** 错误消息包含 `connection reset`、`connection refused` 或 `dns`
- **WHEN** `classify()` 被调用
- **THEN** SHALL 返回 `FailureKind::TransientRetryable { .. }`

#### Scenario: 4xx client errors except 429 are NonRetryable
- **GIVEN** 错误消息包含 `400`、`401`、`403`、`404`、`407`、`413` 或 `422`
- **WHEN** `classify()` 被调用
- **THEN** SHALL 返回 `FailureKind::NonRetryable { .. }`

#### Scenario: Context/payload size errors are RequiresRequestMutation
- **GIVEN** 错误消息包含 `context too large`、`context_length`、`max_tokens` 或 `payload too large`
- **WHEN** `classify()` 被调用
- **THEN** SHALL 返回 `FailureKind::RequiresRequestMutation { .. }`

#### Scenario: Unmatched errors default to NonRetryable
- **GIVEN** 错误消息不与任何已知分类规则匹配
- **WHEN** `classify()` 被调用
- **THEN** SHALL 返回 `FailureKind::NonRetryable { .. }`

#### Scenario: classify() does not consider stream state
- **GIVEN** 任何 provider 错误
- **WHEN** `classify()` 被调用
- **THEN** SHALL 仅根据错误消息内容判定分类
- **AND** SHALL NOT 检查 stream 是否已发出 delta

### Requirement: Stream safety SHALL follow a monotonic state machine
系统 SHALL 使用 `StreamState` 状态机追踪 stream 输出进度，并在决定 retry/fallback 时引用该状态，不得仅依赖 "任意 delta" 的二元信号。

#### Scenario: PreConnection state allows auto-retry
- **GIVEN** stream 尚未建立连接（StreamState::PreConnection）
- **WHEN** `may_auto_retry()` 被调用
- **THEN** SHALL 返回 `true`

#### Scenario: NoDelta state allows auto-retry and sync-fallback
- **GIVEN** stream 已连接但尚未推送任何 delta（StreamState::NoDelta）
- **WHEN** `may_auto_retry()` 与 `may_stream_to_sync_fallback()` 被调用
- **THEN** 两者 SHALL 都返回 `true`

#### Scenario: ReasoningOnly state allows auto-retry and sync-fallback
- **GIVEN** stream 仅推送了 reasoning delta（StreamState::ReasoningOnly）
- **WHEN** `may_auto_retry()` 与 `may_stream_to_sync_fallback()` 被调用
- **THEN** 两者 SHALL 都返回 `true`

#### Scenario: VisibleTextStarted state prohibits auto-retry and sync-fallback
- **GIVEN** stream 已推送可见文本（StreamState::VisibleTextStarted）
- **WHEN** `may_auto_retry()` 与 `may_stream_to_sync_fallback()` 被调用
- **THEN** 两者 SHALL 都返回 `false`

#### Scenario: ToolCallStarted state prohibits auto-retry and sync-fallback
- **GIVEN** stream 已推送工具调用声明（StreamState::ToolCallStarted）
- **WHEN** `may_auto_retry()` 与 `may_stream_to_sync_fallback()` 被调用
- **THEN** 两者 SHALL 都返回 `false`

#### Scenario: Stream state never rolls back
- **GIVEN** stream 当前状态为 `VisibleTextStarted` 或 `ToolCallStarted`
- **WHEN** 任何 `StreamEventKind` 事件到达
- **THEN** `transition()` SHALL NOT 将状态回退到 `NoDelta` 或 `ReasoningOnly`
- **AND** 状态迁移 SHALL 是单调不可逆的

#### Scenario: decide() aborts auto-retry when stream state is unsafe
- **GIVEN** 当前 `StreamState` 为 `VisibleTextStarted` 或 `ToolCallStarted`
- **WHEN** `ProviderRetryPolicy.decide()` 被调用
- **THEN** SHALL 返回 `RetryDecision::Abort`，即使 `FailureKind` 为 `TransientRetryable`

#### Scenario: Existing streamed_any_delta boolean is an acceptable intermediate guard
- **GIVEN** 当前代码使用 `streamed_any_delta: bool` 而非完整的 `StreamState`
- **WHEN** 实现第一阶段重构
- **THEN** `streamed_any_delta = true` SHALL 等价于 `!may_auto_retry()`
- **AND** 后续阶段 SHALL 迁移到完整 `StreamState`

### Requirement: Retry budget SHALL constrain both attempts and wall-clock time
每个 request-level retry loop SHALL 通过 `RetryBudget` 同时限制重试次数和总耗时，任一维度耗尽即终止。

#### Scenario: Budget exhausts by attempt limit
- **GIVEN** `RetryBudget { max_retries: 2, total_budget_ms: 10000 }`
- **WHEN** 2 次重试均已记录
- **THEN** `is_exhausted()` SHALL 返回 `true`
- **AND** `exhausted_reason()` SHALL 返回 `"attempt_limit_exhausted"`

#### Scenario: Budget exhausts by time
- **GIVEN** `RetryBudget { max_retries: 10, total_budget_ms: 1000 }`
- **WHEN** 已累计 `elapsed_ms >= 1000`
- **THEN** `is_exhausted()` SHALL 返回 `true`
- **AND** `exhausted_reason()` SHALL 返回 `"time_budget_exhausted"`

#### Scenario: Budget exhaustion in decide() leads to Escalate
- **GIVEN** `FailureKind::TransientRetryable` 且 `RetryBudget` 已耗尽
- **WHEN** `ProviderRetryPolicy.decide()` 被调用
- **THEN** SHALL 返回 `RetryDecision::Escalate`

#### Scenario: Provider retry uses default budget
- **GIVEN** `retry_provider_timeout()` 执行
- **THEN** 其 `BackoffConfig` SHALL 使用 `max_retries: 4, total_budget_ms: 30000`
- **AND** `initial_delay_ms: 500, multiplier: 2.0, max_delay_ms: 8000`

#### Scenario: Tool timeout retry uses default budget
- **GIVEN** `retry_tool_timeout()` 执行
- **THEN** 其 `BackoffConfig` SHALL 使用 `max_retries: 1, total_budget_ms: 30000`
- **AND** `initial_delay_ms: 500, multiplier: 2.0, max_delay_ms: 8000`

### Requirement: Retry-After header SHALL influence retry delay
系统 SHALL 解析 provider 返回的 `Retry-After` 响应头，并将其与 backoff 延迟取较大值，但无需等待超过剩余 budget 的时长。

#### Scenario: Retry-After seconds are parsed
- **GIVEN** `Retry-After: 30`
- **WHEN** `RetryAfter::parse()` 被调用
- **THEN** `seconds` SHALL 为 `Some(30)`

#### Scenario: Retry-After 0 produces no effective delay
- **GIVEN** `RetryAfter { seconds: Some(0) }`
- **WHEN** `effective_delay(500, 10000)` 被调用
- **THEN** SHALL 返回 `None`（不重试）

#### Scenario: Retry-After exceeding budget produces no effective delay
- **GIVEN** `RetryAfter { seconds: Some(60) }` 且 `remaining_budget_ms = 10000`
- **WHEN** `effective_delay(500, 10000)` 被调用
- **THEN** SHALL 返回 `None`（不重试）

#### Scenario: Retry-After combines with backoff delay
- **GIVEN** `RetryAfter { seconds: Some(3) }` 且 `remaining_budget_ms = 10000`
- **WHEN** `effective_delay(500, 10000)` 被调用
- **THEN** SHALL 返回 `Some(3000)`（`max(500, 3000) = 3000`）

#### Scenario: decide() respects RetryAfter when provided
- **GIVEN** `ProviderRetryPolicy.decide()` 收到 `Some(&RetryAfter { seconds: Some(2) })`
- **WHEN** 所有其他条件满足 Retry
- **THEN** 返回的 `RetryDecision::Retry { delay_ms }` SHALL 满足 `delay_ms >= 2000`

### Requirement: Tool timeout retry SHALL follow the same pattern as provider timeout retry
`web_fetch` 和 `web_search` 等工具的 timeout retry SHALL 使用与 provider timeout retry 相同的分类与退避逻辑。

#### Scenario: Web fetch timeout triggers retry with tool timeout config
- **GIVEN** `web_fetch` 调用遭遇 timeout 错误
- **WHEN** `retry_tool_timeout` 执行
- **THEN** SHALL 使用 `TOOL_TIMEOUT_RETRY_MAX_ATTEMPTS = 2` 作为最大尝试次数
- **AND** SHALL 仅在 timeout 类错误时重试
- **AND** 非 timeout 错误（如 4xx）SHALL 立即终止

#### Scenario: Web search timeout triggers retry with tool timeout config
- **GIVEN** `web_search` 调用遭遇 timeout 错误
- **WHEN** `retry_tool_timeout` 执行
- **THEN** SHALL 使用与 `web_fetch` 相同的退避配置与分类逻辑

#### Scenario: Non-timeout tool error does not retry
- **GIVEN** 工具调用返回非 timeout 错误（如 HTTP 403）
- **WHEN** `retry_tool_timeout` 执行
- **THEN** SHALL 立即返回错误
- **AND** SHALL NOT 执行任何退避 sleep

#### Scenario: Commands with side effects are not auto-retried
- **GIVEN** `workspace_run_command` 执行超时
- **WHEN** retry 决策被评估
- **THEN** SHALL 不自动重试
- **AND** 工具结果中的 `error.code` SHALL 稳定为 `timeout`

### Requirement: Stream → sync fallback SHALL respect stream safety constraints
当 stream 请求失败且 request-level retry 耗尽时，系统 SHALL 仅当 stream 尚未提交可见文本或工具调用时才允许降级到 sync。

#### Scenario: Stream→sync fallback is allowed before visible text
- **GIVEN** `followup_stream` 在 request-level retry 后仍失败
- **WHEN** stream 状态为 `NoDelta` 或 `ReasoningOnly`
- **THEN** 系统 SHALL 自动降级为 `followup_sync`
- **AND** sync fallback 的 `provider_source` SHALL 标记为 `provider_followup_stream_sync_fallback`

#### Scenario: Stream→sync fallback is blocked after visible text
- **GIVEN** `followup_stream` 在 request-level retry 后仍失败
- **WHEN** stream 已推送可见文本或工具调用
- **THEN** 系统 SHALL NOT 执行 sync fallback
- **AND** 该失败 SHALL 直接上报为 turn 失败

#### Scenario: Sync fallback also fails and produces local synthesis
- **GIVEN** `followup_stream` 降级为 `followup_sync`
- **WHEN** `followup_sync` 的 retry 也耗尽或遭遇非重试性错误
- **THEN** 系统 SHALL 使用 `local_tool_followup_fallback_response` 合成本地兜底响应
- **AND** `provider_mode` SHALL 为 `"fallback"`
- **AND** `fallback_reason` SHALL 包含失败原因

### Requirement: Provider retry SHALL be owned by pony-agent-core
Provider retry 策略、分类器、退避计算和预算管理 SHALL 全部位于 `pony-agent-core`，`src-tauri` 不持有这些逻辑。

#### Scenario: Core owns ProviderRetryPolicy
- **GIVEN** `ProviderRetryPolicy` 定义在 `crates/pony-agent-core/src/agent/retry.rs`
- **WHEN** 任何 retry 决策需要执行
- **THEN** SHALL 通过 `ProviderRetryPolicy.decide()` 完成
- **AND** `src-tauri` SHALL NOT 包含退避计算或分类逻辑的副本

#### Scenario: src-tauri bridges retry results to frontend
- **GIVEN** core 返回 retry 相关事件（如 budget 耗尽、fallback 触发）
- **WHEN** 这些事件需要前端消费
- **THEN** `src-tauri` SHALL 透传而不改写语义
- **AND** `src-tauri` SHALL NOT 自行判定是否应该重试

### Requirement: Frontend whole-turn auto-retry SHALL be retired
前端现有基于计时器的 whole-turn 自动重试（`runtime.ts`）SHALL 退场；turn-level retry 仅允许作为显式 control-plane action。

#### Scenario: Frontend does not auto-retry failed turns
- **GIVEN** 一个 turn 进入 `failed` 终态
- **WHEN** 前端收到 `turn:failed` 事件
- **THEN** SHALL NOT 自动启动重试计时器
- **AND** 前端 SHALL 展示失败状态而非 pending retry 状态

#### Scenario: Explicit retry uses start_graph_run_stream
- **GIVEN** 用户主动点击"重试"按钮
- **WHEN** 前端构造 retry 请求
- **THEN** SHALL 通过 `start_graph_run_stream(turn_id, { startReason: "retry_failed_turn" })` 发起
- **AND** SHALL NOT 绕过 core 执行路径

### Requirement: Telemetry SHALL capture retry events at all three layers
系统 SHALL 在每个 retry attempt、fallback 触发和 escalation 发生时输出结构化日志。

#### Scenario: Each retry attempt logs structured data
- **GIVEN** `retry_provider_timeout` 执行一次 retry attempt
- **WHEN** 该 attempt 完成（成功或失败）
- **THEN** 日志 SHALL 包含：`label`、`attempt`、`max_attempts`、`failure`、`decision`、`delay_ms`、`elapsed_ms`、`budget_remaining`、`stream_state`

#### Scenario: Fallback transitions log structured data
- **GIVEN** 系统从 `followup_stream` 降级到 `followup_sync`
- **WHEN** 降级发生
- **THEN** 日志 SHALL 包含：`source`、`target`、`reason`、`stream_state`、`attempts_used`、`budget_remaining`

#### Scenario: Escalation events log structured data
- **GIVEN** request-level retry 因 budget 耗尽而 escalate
- **WHEN** `RetryDecision::Escalate` 被返回
- **THEN** 日志 SHALL 包含：`layer`、`reason`、`attempts_used`、`elapsed_ms`、`budget_exhausted_reason`

### Requirement: Tests SHALL be deterministic and avoid real-time sleep
所有 retry 相关测试 SHALL 使用 `Sleeper` trait 的 fake 实现，不得依赖真实 `thread::sleep` 验证退避行为。

#### Scenario: Pure strategy tests verify decision logic
- **GIVEN** `ProviderRetryPolicy` 和给定输入（failure、budget、attempt、retry_after、stream_state）
- **WHEN** `decide()` 被调用
- **THEN** 返回的 `RetryDecision` SHALL 与预期值一致
- **AND** 此测试 SHALL 不包含任何 sleep I/O

#### Scenario: FakeSleeper records total delay
- **GIVEN** `FakeSleeper` 实例
- **WHEN** `sleep(Duration::from_millis(N), _)` 被调用
- **THEN** SHALL 立即返回 `Ok`
- **AND** `total_slept` SHALL 增加 N

#### Scenario: retry_with_policy uses injected sleeper
- **GIVEN** `ProviderRetryPolicy::with_sleeper(config, Box::new(FakeSleeper::new()))`
- **WHEN** `retry_with_policy` 执行多次 retry
- **THEN** 测试 SHALL 验证 `total_slept` 与预期退避总和一致
- **AND** 测试 SHALL 在毫秒级完成

#### Scenario: Cancellation test verifies AtomicBool guard
- **GIVEN** `cancelled` 为 `true` 的 `AtomicBool`
- **WHEN** `retry_with_policy` 开始执行
- **THEN** SHALL 立即返回 `Err(RetryDecision::Abort { .. })`
- **AND** SHALL NOT 调用操作闭包
