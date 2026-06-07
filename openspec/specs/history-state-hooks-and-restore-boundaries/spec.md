## ADDED Requirements

### Requirement: History-state hooks SHALL attach only to stable session-history control boundaries
系统 SHALL 只允许在稳定的 `history checkout / branch restore / branch fork / branch switch` boundary 上执行 history-state hooks。

#### Scenario: A hook observes history checkout resolution
- **WHEN** 用户执行历史节点 checkout
- **THEN** 系统 SHALL 在对应 canonical history-state boundary 上发射 hook dispatch
- **AND** hook SHALL NOT 依赖 session 内部临时状态猜测该边界

#### Scenario: A hook tries to attach to an unstable history-internal step
- **WHEN** 某个 hook 试图挂接未承诺的 history graph 内部实现步骤
- **THEN** 系统 SHALL 拒绝该 hook 注册或将其视为无效配置

### Requirement: History-state hooks SHALL consume normalized history-control envelopes
history-state hooks SHALL 只消费规范化 history-control envelope，而不是直接操作底层 store。

#### Scenario: A hook evaluates transcript-and-workspace checkout
- **WHEN** 系统处理一次 `TranscriptAndWorkspace` checkout 请求
- **THEN** hook SHALL 读取 normalized history-control envelope
- **AND** hook SHALL NOT 直接改写 history graph、history cursor 或 workspace rollback 真值

#### Scenario: A hook transforms allowed request fields only
- **WHEN** 某个 history-state hook 返回 patch 结果
- **THEN** 该结果 SHALL 只允许修改预先声明的请求侧白名单字段
- **AND** `workspace_rollback_capable / workspace_rollback_applied / degradation_reason` 等 truth-source 字段 SHALL 保持只读

### Requirement: History-state hooks SHALL preserve degrade and rollback truth-source semantics
history-state hooks SHALL 保持既有 degrade / rollback 合同，不得成为第二套恢复真相源。

#### Scenario: Workspace rollback is unsupported
- **WHEN** 用户请求 `TranscriptAndWorkspace` checkout，但目标节点不支持 workspace rollback
- **THEN** 系统 SHALL 继续返回既有的 `degraded_to_transcript_only` 语义
- **AND** hook SHALL NOT 伪造 `workspace_rollback_applied=true`
- **AND** 前端 SHALL NOT 被迫通过私有补偿逻辑猜测真实恢复结果

#### Scenario: A hook blocks a branch switch
- **WHEN** hook 返回阻断型结果
- **THEN** 系统 SHALL 以结构化 guard/failure contract 记录该决定
- **AND** hook SHALL NOT 直接绕开既有 `HistoryCursor` / branch truth-source

#### Scenario: A hook blocks checkout before resolution
- **WHEN** hook 在 checkout start boundary 返回阻断型结果
- **THEN** 系统 SHALL NOT 生成新的 resolved history-state evidence
- **AND** hook SHALL NOT 改写既有 history cursor 真值

### Requirement: History-state hook evidence SHALL persist across reload and read-plane surfaces
history-state hooks 的执行证据 SHALL 进入 session truth-source，并在 reload 后被 control-plane / runtime view 读回。

#### Scenario: A session reloads after a history-state control command
- **WHEN** reload 发生在 history checkout、branch restore、branch fork 或 branch switch 命令之后
- **THEN** 系统 SHALL 能读回对应 hook 的 boundary、result kind、duration 与 degraded 摘要

#### Scenario: Canonical history-state boundaries are projected as the minimum persisted audit chain
- **WHEN** history-state hooks 命中 `history_checkout / branch_restore / branch_fork / branch_switch` boundary
- **THEN** 系统 SHALL 通过 file-backed roundtrip 保留这些 boundary 的 evidence
- **AND** runtime view / control-plane SHALL 能读回至少 `boundary / result kind / duration`

#### Scenario: Missing hooks evidence does not become restore truth
- **WHEN** reload 后缺少某次 history-state hooks 的 persistence evidence
- **THEN** 系统 SHALL 只表现为“无 hooks evidence”
- **AND** 系统 SHALL NOT 据此重建 restore 结论、cursor 状态或 degrade 判定

#### Scenario: Control-plane and runtime view project the same evidence contract
- **WHEN** 某次 history-state control 命令已产生 persisted evidence
- **THEN** control-plane 与 runtime view SHALL 读回同一口径的 boundary、result kind 与 degraded 摘要
- **AND** 前端 SHALL NOT 依赖额外私有状态机修正这份 evidence

### Requirement: History-state hooks SHALL remain controlled extensions
history-state hooks SHALL 继续保持受控扩展面，而不是新的 history scheduler 或 workspace restore 执行器。

#### Scenario: A hook attempts to mutate persisted history topology
- **WHEN** 某个 history-state hook 试图直接修改 history nodes、history branches 或 cursor persisted state
- **THEN** 系统 SHALL 拒绝该结果
- **AND** hook SHALL NOT 成为新的 history truth-source

#### Scenario: Persisted history-state evidence remains audit-only
- **WHEN** 系统把 history-state hooks evidence 写入 session truth-source
- **THEN** 该 evidence SHALL 只作为审计证据载体
- **AND** hook evidence SHALL NOT 成为 restore、submission 或 history cursor 的仲裁输入
