## ADDED Requirements

### Requirement: PA-042 v1 SHALL expose a canonical history-control audit summary
Pony Agent SHALL 为 history-control 读面提供 canonical audit summary，使前端和调试面能够直接理解最近一次 `checkout / restore / fork / switch` 动作，而不是自行拼装多份局部字段。

#### Scenario: A history checkout succeeds
- **WHEN** 用户执行一次 history checkout 并成功完成
- **THEN** 系统 SHALL 生成并暴露最近一次 session-control audit summary
- **AND** summary SHALL 至少包含 `command kind / boundary / result kind / duration / target summary`

#### Scenario: A branch switch succeeds
- **WHEN** 用户执行 branch switch
- **THEN** 前端与调试读面 SHALL 能直接读取同口径 summary
- **AND** SHALL 不需要自行拼接 branch、visible node 与 result kind

#### Scenario: A run-control action occurs
- **WHEN** 用户触发 `stop / continue / resume / replay` 一类 run-control 动作
- **THEN** `PA-042 v1` SHALL NOT 被要求为这些动作新增同一 summary family
- **AND** 该范围 SHALL 由后续独立任务承接

### Requirement: Session-control audit summary SHALL be projected from persisted truth-source
Session-control audit summary SHALL 来源于既有 persisted evidence 与 session truth-source 的后端投影，而不是前端私有解释。

#### Scenario: Runtime view and snapshot read the same control result
- **WHEN** 某次 control action 已经留下 persisted evidence
- **THEN** `SessionSnapshot` 与 `SessionRuntimeView` SHALL 读回同一口径的 required summary fields
- **AND** SHALL NOT 各自重算 blocked、degraded 或 rollback 结论

#### Scenario: A history-control command returns a result
- **WHEN** `checkout / restore / fork / switch` 返回控制结果
- **THEN** 响应对象 SHALL 投影与 session snapshot 同口径的 required summary fields
- **AND** 前端 SHALL 不需要通过兼容字段链再推导一次 audit feedback

### Requirement: Audit summary SHALL separate action evidence from current context
Session-control audit summary SHALL 把“动作证据”与“当前现态投影”分离，防止把读取时现态伪装成历史动作结果。

#### Scenario: A summary includes branch and node context
- **WHEN** 系统为最近一次 history-control 动作提供 branch 或 visible node 相关信息
- **THEN** 这些字段 SHALL 被标识为 current context projection 或等价分层结构
- **AND** SHALL NOT 冒充为动作证据本身

#### Scenario: Required action fields are compared across surfaces
- **WHEN** 测试比较 snapshot、runtime view 与 command response 的 summary
- **THEN** 以下 required fields SHALL 逐字一致：
- **AND** `status / source_family / command_kind / boundary / result_kind / summary / elapsed_ms / blocked / degraded / evidence_id / observed_at_ms`

### Requirement: Session-control audit summary SHALL preserve truth-source boundaries
Session-control audit summary SHALL 是只读审计面，不得演化成新的 restore、cursor、branch 或 rollback 仲裁源。

#### Scenario: Workspace rollback degrades to transcript only
- **WHEN** 一次 `transcript_and_workspace` 恢复降级为 transcript-only
- **THEN** summary SHALL 反映 `degraded=true`
- **AND** summary SHALL NOT 伪造 `workspace rollback applied`
- **AND** degraded 结论 SHALL 继续来源于既有 restore truth-source

#### Scenario: A control action is blocked before resolution
- **WHEN** hook 或其他稳定 guard 在命令 resolution 前阻断 control action
- **THEN** summary SHALL 反映 `blocked=true`
- **AND** summary SHALL 保留请求摘要以供解释
- **AND** summary SHALL NOT 改写既有 cursor / branch 真值

#### Scenario: A summary is missing or tampered with
- **WHEN** summary 缺失、为空，或被故意篡改
- **THEN** 既有 cursor / branch / rollback 真值 SHALL 仍来自原始 truth-source
- **AND** 系统 SHALL NOT 把 summary 作为仲裁输入

### Requirement: Session-control audit summary SHALL survive reload when evidence exists
只要底层 evidence 已成功持久化，session-control audit summary SHALL 在 reload 后仍可读回。

#### Scenario: The application is closed and reopened
- **WHEN** 用户关闭应用并重新打开同一个 session
- **THEN** 若最近一次 control action 的 persisted evidence 仍存在
- **THEN** 系统 SHALL 继续读回该 action 的 audit summary

#### Scenario: File-backed roundtrip restores the same summary
- **WHEN** session 经历 file-backed 持久化与 reload
- **THEN** summary SHALL 与 reload 前保持同口径
- **AND** runtime view / command response SHALL 继续投影该 summary

### Requirement: Missing evidence SHALL not be reconstructed into fake control conclusions
若 reload 后缺少底层 evidence，系统 SHALL 仅表现为 evidence 缺失，而不得伪造控制结论。

#### Scenario: Summary evidence is missing after reload
- **WHEN** reload 后缺少最近一次 control action 的 persistence evidence
- **THEN** 系统 SHALL 只表现为 `summary unavailable` 或 `evidence missing`
- **AND** SHALL NOT 据此重建 restore 成功、branch 切换成功或 workspace rollback 结果

### Requirement: Frontend control explainability SHALL consume the summary contract first
前端 session control explainability 主展示 SHALL 优先消费 session-control audit summary，而不是继续依赖私有推理链。

#### Scenario: The user inspects the latest control result
- **WHEN** 前端展示最近一次 control action
- **THEN** UI SHALL 能直接显示 `action / boundary / blocked / degraded / rollback outcome / branch or node summary`
- **AND** 这份展示 SHALL 优先来自 summary contract
- **AND** `PA-042 v1` SHALL NOT 被要求重做 `PA-037` 已成立的按钮编排、disabled reason 与状态语言

#### Scenario: Backward compatibility fields still exist
- **WHEN** 旧兼容字段仍被保留用于测试桩或灰度过渡
- **THEN** 前端主展示 SHALL 仍优先消费新 summary contract
- **AND** 兼容字段 SHALL NOT 成为新的长期真相源
