## ADDED Requirements

### Requirement: Persisted terminal traces SHALL carry canonical terminal envelope across sync and stream paths
Pony Agent SHALL 为 completed、failed、cancelled 三类 terminal persisted trace 统一持有 canonical terminal event envelope，而不区分该 turn 源自 sync `run_turn()` 还是 streamed turn。

该 canonical terminal envelope 的 `eventType` SHALL 只复用 [PA-031](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-031-define-turn-lifecycle-and-event-contract.md) 已定义的 canonical terminal lifecycle event 集合；本 change SHALL NOT 新增同义 terminal event contract。

#### Scenario: A sync completed turn is persisted
- **WHEN** 一个 sync `run_turn()` 正常完成并写入 persisted trace
- **THEN** 该 `TurnTraceRecord` SHALL 持有 `eventId / eventType / eventVersion / sequence / emittedAtMs`
- **AND** `eventType` SHALL 对齐 canonical terminal event

#### Scenario: A sync failed turn is persisted
- **WHEN** 一个 sync turn 以 failed 终止并写入 persisted trace
- **THEN** 该 `TurnTraceRecord` SHALL 持有 canonical failed terminal envelope
- **AND** 其既有 provider/tool/hook evidence 字段 SHALL 与 terminal trace 一起保留

#### Scenario: A streamed terminal turn is reloaded later
- **WHEN** 一个 streamed terminal turn 已被 annotate 并落盘，稍后 reload session
- **THEN** 重新读出的 persisted trace SHALL 保留同一份 canonical terminal envelope

### Requirement: Monitor and control-plane SHALL aggregate from persisted terminal truth
Pony Agent 的 monitor / control-plane 聚合 SHALL 以 persisted terminal trace 为真相源，而不是为 sync/failed/cancelled 路径再建立额外推测口径。

本 change 只验证 terminal envelope parity 与 persisted truth-source consumption；SHALL NOT 借此扩展 hooks evidence model、monitor 聚合维度或新的 lifecycle contract。

#### Scenario: Mixed sync and streamed sessions are aggregated
- **WHEN** monitor 读取同时包含 sync 与 streamed turn 的 session traces
- **THEN** 聚合 SHALL 不需要依据来源类型分叉 terminal metadata 逻辑
- **AND** terminal metrics SHALL 来自 persisted trace 本身

#### Scenario: A failed trace is shown in session drilldown
- **WHEN** control-plane 读取一个 failed trace 做 session drilldown
- **THEN** drilldown SHALL 能读回其 terminal envelope
- **AND** SHALL 同时读回既有关键 provider/tool/hook evidence

### Requirement: Frontend SHALL NOT fabricate canonical terminal metrics when envelope is absent
当前端读取 session snapshot、monitor drilldown 或 runtime trace history 时，SHALL 消费后端给出的 terminal envelope，而不是为缺失 envelope 的 trace 自行发明 canonical metrics。

#### Scenario: Persisted trace already has terminal envelope
- **WHEN** 前端读取一个已持有 terminal envelope 的 persisted trace
- **THEN** 前端 SHALL 直接消费这些字段
- **AND** SHALL NOT 再从 `phase / message / fallback timeline` 重新推导冲突的 terminal metadata

#### Scenario: Persisted trace is missing terminal envelope
- **WHEN** 前端读取一个缺失 terminal envelope 的 persisted trace
- **THEN** 前端 SHALL 只显示 non-canonical/raw trace 信息
- **AND** SHALL NOT 产出 completed / failed / cancelled canonical terminal metrics
