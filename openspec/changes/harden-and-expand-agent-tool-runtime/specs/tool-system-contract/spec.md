## ADDED Requirements

### Requirement: Tool metadata SHALL have one authoritative source

Pony Agent SHALL 将 identity、schema、kind、exposure、permission declaration、execution policy 与 handler provenance 放入同一工具 descriptor 或其不可分割的正式结构。

#### Scenario: A tool contract is projected for a provider
- **WHEN** provider 构建 tool schema
- **THEN** 它 SHALL 从 registry descriptor snapshot 投影
- **AND** SHALL NOT 依据工具数量、注册顺序或名称 switch 选择另一套合同

### Requirement: Tool execution policy SHALL be explicit

Pony Agent SHALL 为每个工具显式定义并发安全、取消行为、默认 deadline 与结果预算。

#### Scenario: A tool omits an execution policy
- **WHEN** 新工具没有声明某项执行策略
- **THEN** registry SHALL 使用 fail-closed 默认值
- **AND** SHALL 默认视为不可并发、可取消且受有限预算约束

### Requirement: Internal aliases SHALL be unambiguous

Pony Agent SHALL 保证任一输入工具名在同一 registry snapshot 中只解析到一个 canonical descriptor。

#### Scenario: Two primitives map to the same product name
- **WHEN** registry 构建时发现别名冲突或反向解析不唯一
- **THEN** 注册 SHALL 失败或显式隐藏低优先级兼容别名
- **AND** SHALL NOT 通过静默覆盖让调用落到错误 primitive

### Requirement: Registry snapshots SHALL be versioned and namespace-safe

Pony Agent SHALL 为 descriptor/source snapshot 赋予稳定 revision，并保留 builtin/internal namespace。

#### Scenario: An external source collides with reserved or existing identity
- **WHEN** MCP 或其他外部 source 声明 builtin/internal prefix、重复 descriptor id、冲突 model alias 或 source mismatch
- **THEN** registry SHALL 原子拒绝整个 snapshot
- **AND** SHALL NOT 静默覆盖已有 capability 或 handler
