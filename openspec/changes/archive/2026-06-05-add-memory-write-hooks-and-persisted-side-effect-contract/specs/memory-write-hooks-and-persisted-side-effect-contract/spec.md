## ADDED Requirements

### Requirement: Memory-write hooks SHALL consume normalized write intents
系统 SHALL 只允许 memory-write hooks 消费规范化写入意图，而不是直接操作底层 store。

#### Scenario: A hook validates a project-memory write
- **WHEN** 系统准备写入 project-level memory fact
- **THEN** hook SHALL 读取 normalized write intent
- **AND** hook SHALL NOT 直接改写 session store

### Requirement: Persisted side effects SHALL declare recovery evidence
声明 `persisted_effect` 的 hook SHALL 同时声明最小持久化证据与 recovery 判定依据。

#### Scenario: A hook leaves a persisted side effect
- **WHEN** `PA-039` 当前覆盖的 memory-write 路径产生真实副作用
- **THEN** 系统 SHALL 持久化可审计 evidence
- **AND** reload 后 SHALL 能判断是否需要 replay

#### Scenario: Persisted evidence is recorded for a side effect
- **WHEN** 系统认定当前 memory-write 路径上的 side effect 已进入 `persisted_effect`
- **THEN** evidence SHALL 至少包含 hook 标识、boundary、写入目标摘要、持久化结果引用与 replay 判定依据

### Requirement: Replay-required hooks SHALL default to safe recovery semantics
当 hook 缺少足够 persisted evidence 时，系统 SHALL 默认回退到 `replay_required`。

#### Scenario: Reload sees incomplete persisted evidence
- **WHEN** recovery 链路无法证明某个 side effect 已安全保留
- **THEN** 系统 SHALL 以 `replay_required` 处理该 hook
- **AND** 不得隐式假设副作用已恢复
- **AND** 前端或 read-plane SHALL NOT 自行猜测该副作用已完成恢复

### Requirement: Memory-write hook evidence SHALL be readable after reload
memory-write hooks 的 evidence、结果与 replay 判定依据 SHALL 能在 reload 后被 control-plane / runtime view 读回。

#### Scenario: A session reloads after a guarded memory write
- **WHEN** reload 完成
- **THEN** runtime view / session drilldown SHALL 能读回 hook 的 boundary、结果与 evidence

### Requirement: Memory-write hook trace SHALL persist as session truth-source
memory-write hook 的 deny / transform / observe 决策 SHALL 以独立 trace 记录进入 session truth-source，而不是只停留在运行时结果对象。

#### Scenario: A memory-write hook blocks a write
- **WHEN** guard hook 阻断某次 memory write
- **THEN** 系统 SHALL 持久化该 hook 的 trace record
- **AND** reload 后 SHALL 仍能读回 hook 名称、结果类型、阻断状态与摘要

#### Scenario: A historical node is reloaded
- **WHEN** 用户 checkout 到包含 memory-write hook 决策的历史节点
- **THEN** 系统 SHALL 恢复该历史节点对应的 memory-write hook trace 集合
- **AND** 不得错误暴露为当前 live session 的最新 hook trace 集合
