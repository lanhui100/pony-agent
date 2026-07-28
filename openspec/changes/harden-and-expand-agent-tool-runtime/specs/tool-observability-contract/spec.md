## ADDED Requirements

### Requirement: Dispatcher lifecycle SHALL be observable as one trace family

Pony Agent SHALL 为 resolve、validate、permission、hook、execute、cancel 和 result normalization 提供关联到同一 call id 的结构化证据。

#### Scenario: A child tool runs under a composite
- **WHEN** child invocation 完成、失败、被拒绝或待审批
- **THEN** trace SHALL 同时保留 parent call id 与 child call id
- **AND** child permission/hook/result SHALL 可独立下钻

### Requirement: Resource budgets SHALL be visible

Pony Agent SHALL 展示工具实际消耗和命中的 deadline、bytes、items、depth 与 concurrency budgets。

#### Scenario: Output or scanning is truncated
- **WHEN** process、Web、Search、Glob 或 composite 达到预算
- **THEN** result 与 trace SHALL 标明 truncated、limit、observed amount 和 reason
- **AND** frontend SHALL 能区分成功完整与成功但截断

### Requirement: Deferred elevation SHALL be traceable

Pony Agent SHALL 记录 deferred 工具从候选发现到当前 turn 提升的 lineage。

#### Scenario: A discovered tool becomes model-visible
- **WHEN** ToolSearch elevation 修改 provider tool view
- **THEN** trace SHALL 记录 source、descriptor id、turn id 与 prefix mutation reason
- **AND** reload 后 SHALL 不把过期 elevation 误认为仍有效

### Requirement: Sensitive tool inputs SHALL be summarized by default

Pony Agent SHALL 对 command、environment、stdin、URL credentials、MCP content、Ask answer 与 approval payload 采用字段级脱敏、摘要与预算策略。

#### Scenario: A sensitive tool invocation is persisted
- **WHEN** trace、checkpoint 或 session 保存工具证据
- **THEN** 默认 SHALL 保存必要的 digest、分类、截断计数和 provenance
- **AND** SHALL NOT 默认保存完整敏感正文
