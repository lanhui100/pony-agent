## ADDED Requirements

### Requirement: PA-043 v1 SHALL expose a canonical run-control audit summary
Pony Agent SHALL 为 run-control 读面提供 canonical audit summary，使前端和调试面能够直接理解最近一次 `stop / continue / resume / replay(start)` 动作，而不是自行拼装多份局部字段。

#### Scenario: A resume action succeeds
- **WHEN** 用户执行一次 resume 并成功完成
- **THEN** 系统 SHALL 生成并暴露最近一次 run-control audit summary
- **AND** summary SHALL 至少包含 `command kind / boundary / result kind / duration / target summary`

#### Scenario: A stop action succeeds
- **WHEN** 用户执行 stop
- **THEN** 前端与调试读面 SHALL 能直接读取同口径 summary
- **AND** SHALL 不需要自行拼接 checkpoint、submission plan 与 result kind

#### Scenario: A history-control action occurs
- **WHEN** 用户触发 `checkout / restore / fork / switch` 一类 history-control 动作
- **THEN** `PA-043 v1` SHALL NOT 被要求为这些动作新增同一 summary family
- **AND** 该范围 SHALL 继续由 `PA-042` 承接

#### Scenario: An initial start begins a normal turn
- **WHEN** 系统执行普通首轮 `start_graph_run_stream`
- **THEN** 该启动 SHALL NOT 生成 `RunControlAuditSummary`
- **AND** 该语义 SHALL 保持在通用 turn/run 启动 contract 中

### Requirement: Run-control audit summary SHALL be projected from persisted truth-source
Run-control audit summary SHALL 来源于既有 persisted evidence 与 run truth-source 的后端投影，而不是前端私有解释。

#### Scenario: Runtime view and snapshot read the same control result
- **WHEN** 某次 run-control action 已经留下 persisted evidence
- **THEN** `SessionSnapshot` 与 `SessionRuntimeView` SHALL 读回同一口径的 required summary fields
- **AND** SHALL NOT 各自重算 blocked、degraded、resume 或 replay 结论

#### Scenario: A run-control command returns a result
- **WHEN** `stop / continue / resume / replay(start)` 返回控制结果
- **THEN** 响应对象 SHALL 投影与 session snapshot 同口径的 required summary fields
- **AND** 前端 SHALL 不需要通过兼容字段链再推导一次 audit feedback

#### Scenario: A replay or restart start is returned
- **WHEN** `start_graph_run_stream` 作为 replay/restart 的 run-control 结果返回
- **THEN** summary SHALL 包含可判定的 `start_reason`
- **AND** `start_reason` SHALL 至少区分 `replay_from_checkpoint` 与 `restart_from_checkpoint`
- **AND** 普通 `initial_turn` SHALL NOT 被纳入 run-control summary

### Requirement: Audit summary SHALL separate action evidence from current context
Run-control audit summary SHALL 把“动作证据”与“当前现态投影”分离，防止把读取时现态伪装成历史动作结果。

#### Scenario: A summary includes checkpoint and phase context
- **WHEN** 系统为最近一次 run-control 动作提供 checkpoint、submission plan 或 graph phase 相关信息
- **THEN** 这些字段 SHALL 被标识为 current context projection 或等价分层结构
- **AND** SHALL NOT 冒充为动作证据本身

#### Scenario: Required action fields are compared across surfaces
- **WHEN** 测试比较 snapshot、runtime view 与 command response 的 summary
- **THEN** 以下 required fields SHALL 逐字一致：
- **AND** `status / source_family / command_kind / boundary / result_kind / summary / elapsed_ms / blocked / degraded / evidence_id / observed_at_ms`

#### Scenario: Required summary structure is returned
- **WHEN** 任一读面返回 `RunControlAuditSummary`
- **THEN** 顶层结构 SHALL 固定包含 `action_evidence_summary` 与 `current_context_projection`
- **AND** SHALL NOT 退化成松散字段集合
- **AND** `action_evidence_summary.target_summary` SHALL 始终存在

#### Scenario: A blocked request is explained
- **WHEN** 最近一次 run-control action 被阻断
- **THEN** `action_evidence_summary.request_summary` SHALL 存在
- **AND** 前端 SHALL 能直接解释“尝试执行了什么，但被阻断”

### Requirement: Run-control audit summary SHALL preserve truth-source boundaries
Run-control audit summary SHALL 是只读审计面，不得演化成新的 submission plan、checkpoint 或 graph phase 仲裁源。

#### Scenario: Replay is required after recovery arbitration
- **WHEN** 一次 run-control 仲裁要求 `start_graph_run_stream` 作为 replay/restart 结果
- **THEN** summary SHALL 反映对应的 degraded 或 replay-required 解释
- **AND** summary SHALL NOT 伪造 `resume already applied`
- **AND** arbitration 结论 SHALL 继续来源于既有 submission plan / checkpoint truth-source

#### Scenario: A control action is blocked before resolution
- **WHEN** hook 或其他稳定 guard 在命令 resolution 前阻断 run-control action
- **THEN** summary SHALL 反映 `blocked=true`
- **AND** summary SHALL 保留请求摘要以供解释
- **AND** summary SHALL NOT 改写既有 submission plan / checkpoint / graph phase 真值

#### Scenario: A summary is missing or tampered with
- **WHEN** summary 缺失、为空，或被故意篡改
- **THEN** 既有 submission plan / checkpoint / graph phase 真值 SHALL 仍来自原始 truth-source
- **AND** 系统 SHALL NOT 把 summary 作为仲裁输入

#### Scenario: Reload changes current context but not historical action evidence
- **WHEN** 某次 run-control action 的 persisted evidence 已存在，且 reload 后 phase 或 checkpoint 已变化
- **THEN** `action_evidence_summary` 的 required fields SHALL 保持不变
- **AND** `current_context_projection` MAY 随 truth-source 更新
- **AND** 系统 SHALL NOT 把旧的 current context 冻结成新的 persisted truth

### Requirement: Run-control audit summary SHALL survive reload when evidence exists
只要底层 evidence 已成功持久化，run-control audit summary SHALL 在 reload 后仍可读回。

#### Scenario: The application is closed and reopened
- **WHEN** 用户关闭应用并重新打开同一个 session
- **THEN** 若最近一次 run-control action 的 persisted evidence 仍存在
- **THEN** 系统 SHALL 继续读回该 action 的 audit summary

#### Scenario: File-backed roundtrip restores the same summary
- **WHEN** session 经历 file-backed 持久化与 reload
- **THEN** `action_evidence_summary` 的 required fields SHALL 与 reload 前保持同口径
- **AND** runtime view / command response SHALL 继续投影该 action evidence summary
- **AND** `current_context_projection` SHALL 继续来源于 reload 后的现态 truth-source

### Requirement: Missing evidence SHALL not be reconstructed into fake run-control conclusions
若 reload 后缺少底层 evidence，系统 SHALL 仅表现为 evidence 缺失，而不得伪造控制结论。

#### Scenario: Summary evidence is missing after reload
- **WHEN** reload 后缺少最近一次 run-control action 的 persistence evidence
- **THEN** 系统 SHALL 只表现为 `summary unavailable` 或 `evidence missing`
- **AND** SHALL NOT 据此重建 continue 成功、resume 成功或 replay 已完成结果

#### Scenario: Missing evidence does not recreate a blocked result
- **WHEN** reload 后既看不到最近一次 run-control action 的 persistence evidence，又无法证明 blocked 请求已持久化
- **THEN** 系统 SHALL 仍只表现为 missing/unavailable
- **AND** SHALL NOT 伪造 blocked request summary

### Requirement: Frontend run-control explainability SHALL consume the summary contract first
前端 session control explainability 主展示 SHALL 优先消费 run-control audit summary，而不是继续依赖私有推理链。

#### Scenario: The user inspects the latest run-control result
- **WHEN** 前端展示最近一次 run-control action
- **THEN** UI SHALL 能直接显示 `action / boundary / blocked / degraded / replay-required / checkpoint or phase summary`
- **AND** 这份展示 SHALL 优先来自 summary contract
- **AND** `PA-043 v1` SHALL NOT 被要求重做 `PA-037` 已成立的按钮编排、disabled reason 与状态语言

#### Scenario: The frontend switches data source without redesign
- **WHEN** 前端接入 `RunControlAuditSummary`
- **THEN** 该改动 SHALL 仅替换既有展示位的数据源
- **AND** SHALL NOT 新增 panel、调整 `HomeSessionSidebar` 结构，或改写 `HomeWorkspace` CTA/disabled reason 语义

#### Scenario: Backward compatibility fields still exist
- **WHEN** 旧兼容字段仍被保留用于测试桩或灰度过渡
- **THEN** 前端主展示 SHALL 仍优先消费新 summary contract
- **AND** 兼容字段 SHALL NOT 成为新的长期真相源
