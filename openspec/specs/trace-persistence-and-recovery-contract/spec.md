## ADDED Requirements

### Requirement: Backend trace persistence SHALL be the canonical source of truth
Pony Agent 的桌面运行时 SHALL 以后端持久化 trace 数据作为 canonical truth，而不是由前端在 reload 后重新拼装新的指标真相。

#### Scenario: Application restarts after a completed turn
- **WHEN** 用户完成一个含 trace 的 turn，关闭应用并重新打开同一 session
- **THEN** 该 session 的 trace timeline、trace steps、provider call records 与 turn 级指标 SHALL 仍可从后端持久化状态恢复
- **AND** 前端 SHALL NOT 因缺少运行中内存态而丢失该 turn 的已持久化 trace 指标

#### Scenario: Frontend also has cached runtime state
- **WHEN** 前端存在 local cache 且后端 session snapshot 也可用
- **THEN** 桌面运行时 SHALL 以后端已持久化状态为准
- **AND** 前端 cache SHALL NOT 覆盖后端较新的 trace 真相

### Requirement: Runtime checkpoint and recovery checkpoint SHALL be distinct
系统 SHALL 明确区分“运行中控制用 checkpoint”与“可恢复恢复点”，避免错误承诺断点续跑能力。

#### Scenario: Turn is currently running
- **WHEN** turn 仍在进程内运行中
- **THEN** 系统 MAY 维护 runtime checkpoint 以支撑 stop/cancel/pause
- **AND** 该 checkpoint SHALL NOT 自动被视为可跨重启恢复的 recovery checkpoint

#### Scenario: Recovery-capable checkpoint is exposed
- **WHEN** 某个 checkpoint 被读面标记为可恢复
- **THEN** 该 checkpoint SHALL 显式给出 recovery 能力语义，如 `replayable` 或 `resumable`
- **AND** 调用方 SHALL 能区分其不是仅供进程内控制的 runtime checkpoint

#### Scenario: Recovery checkpoint exposes execution mode
- **WHEN** 某个 checkpoint 被读面标记为 `recovery`
- **THEN** 系统 SHALL 显式给出 `recoveryMode`
- **AND** `recoveryMode` 至少区分 `replay_required` 与 `persisted_effect`

#### Scenario: Run state conflicts with recovery contract
- **WHEN** `runState` 看似允许继续或恢复，但 recovery checkpoint 明确要求 `replay_required`
- **THEN** 调用方 SHALL 以 recovery checkpoint 合同为准
- **AND** 系统 SHALL NOT 因旧 `runState` 仍指向 paused/ready run 就误触发 resume path

### Requirement: History checkout SHALL report transcript and workspace outcomes explicitly
历史 checkout / restore SHALL 明确返回 transcript 与 workspace 两个维度的恢复结果，而不是让调用方自行猜测是否已完成工作区回滚。

#### Scenario: User requests transcript-only checkout
- **WHEN** 用户对历史节点执行 transcript-only checkout
- **THEN** 系统 SHALL 恢复 transcript/history/trace 可见状态
- **AND** 系统 SHALL NOT 声称已恢复 workspace

#### Scenario: User requests transcript and workspace checkout but workspace rollback is unavailable
- **WHEN** 用户请求 transcript+workspace checkout，但目标节点没有可用 workspace rollback 载体
- **THEN** 系统 SHALL 返回 `degraded_to_transcript_only` 或等价降级结果
- **AND** 前端 SHALL 明确展示此次恢复并未成功应用 workspace rollback

### Requirement: Frontend trace consumption SHALL not invent canonical metrics
前端 SHALL 逐步退出 canonical trace 指标与恢复语义的发明者角色。

#### Scenario: Canonical trace payload is available from host
- **WHEN** 后端已提供 trace timeline、provider call records 与 turn-level metrics
- **THEN** 前端 SHALL 直接消费这些字段
- **AND** 前端 SHALL NOT 重新折叠、重算或伪造与后端冲突的 canonical 指标

#### Scenario: Desktop runtime falls back while backend persisted snapshot is available
- **WHEN** 桌面运行时能访问后端 persisted snapshot
- **THEN** 前端 fallback SHALL 仅作为非 canonical UI 辅助态
- **AND** fallback SHALL NOT 生成、覆盖或重算 turn-level canonical trace metrics

#### Scenario: Fallback is used in controlled environments only
- **WHEN** 系统使用 fallback 路径
- **THEN** 该路径 SHALL 仅限 `backend unavailable`、`browser preview` 或仓库明确声明的受控环境
- **AND** 该 fallback 路径 SHALL 明确标识自己不是 canonical truth

#### Scenario: Acceptance verification completes
- **WHEN** `PA-032` 完成交付
- **THEN** 测试 SHALL 覆盖 trace 重启恢复、checkpoint 语义区分、history 恢复降级与前端 hydration 保真
- **AND** 验证证据 SHALL 被写回任务卡与 review 文档
