## ADDED Requirements

### Requirement: Permission SHALL be decided for every actual execution step

Pony Agent SHALL 对每个顶层和 child invocation 在 handler 执行前做独立权限决策。

#### Scenario: A read-classified composite contains a write child
- **WHEN** composite plan 中包含 Write、Edit、Run 或其他更强 scope
- **THEN** 父级权限摘要 SHALL 上收该 scope
- **AND** write child SHALL 仍独立鉴权
- **AND** SHALL NOT 因父工具被标为 read 而放行

### Requirement: Builtin capability registration SHALL preserve descriptor permission truth

Pony Agent SHALL 从 builtin descriptor 注册 capability permission facts。

#### Scenario: Builtin tools have different scopes
- **WHEN** registry 注册 Read、Write、Run、Ask 与 Plan
- **THEN** capability views SHALL 保留各自真实 scope、host mediation 和 approval policy
- **AND** SHALL NOT 把它们统一压缩为 `workspace` 与 `requires_approval=false`

### Requirement: Host-mediated decisions SHALL be resumable and auditable

Pony Agent SHALL 将 waiting_host、approval_required、allow 和 deny 表达为可持久化决策状态。

#### Scenario: The application reloads during an approval or input request
- **WHEN** host-mediated tool call 尚未完成时 session reload
- **THEN** runtime SHALL 恢复或明确失效该请求
- **AND** SHALL NOT 在缺少决策证据时推断为已允许

### Requirement: Pending control requests SHALL bind immutable invocation facts

Pony Agent SHALL 将 Ask 和 approval 表达为同一持久化请求族，但必须以 `request_kind` 区分，并绑定不可变调用事实。

#### Scenario: A host answers or approves a pending request
- **WHEN** host 提交 answer、approve 或 deny
- **THEN** runtime SHALL 验证 session、run、turn、call id、descriptor snapshot id、final args digest、policy digest、expiry 与 one-time nonce/version
- **AND** SHALL 用 compare-and-swap 一次性消费该请求
- **AND** 重复、过期、跨 session、参数变化或 source revision 变化 SHALL fail closed

#### Scenario: Ask and approval are distinct controls
- **WHEN** Ask 收到用户回答
- **THEN** runtime SHALL 只把回答作为 Ask 的结果内容
- **AND** SHALL NOT 将回答解释为其他工具调用的 approval

### Requirement: Permission scope SHALL be typed and monotonic

Pony Agent SHALL 以可计算的 scope set/lattice 表达权限，而不是把自由字符串拼接当作授权依据。

#### Scenario: A composite adds a stronger child scope
- **WHEN** composite 发现或调度具有更强 scope 的 child
- **THEN** 父级 scope SHALL 单调上收并保留 child evidence
- **AND** 未知 scope SHALL fail closed
