## ADDED Requirements

### Requirement: Session control actions SHALL be explicit in the frontend
Pony Agent 前端 SHALL 为 stop、resume、continue、replay 等 session 控制动作提供显式入口，而不是只依赖下一次用户提交去隐式触发 runtime store 仲裁。

#### Scenario: A paused run is present
- **WHEN** 当前 session 存在 paused graph run、recovery-capable checkpoint，或后端 submission plan 明确指向 non-default resume / continue path
- **THEN** 前端 SHALL 展示明确的 `继续 / 恢复 / 重新开始` 入口
- **AND** 这些入口 SHALL 派发到既有 submission plan / runtime store 边界
- **AND** SHALL NOT 新增独立 replay backend command

#### Scenario: A turn is running
- **WHEN** 当前 session 有运行中的 turn 或 graph run
- **THEN** 前端 SHALL 提供显式 `停止` 入口
- **AND** 用户在 stop 后 SHALL 获得可验证反馈

### Requirement: History checkout and restore results SHALL be user-visible
Pony Agent 前端 SHALL 显式展示 checkout / restore 的结果合同，尤其是 transcript-only 降级与工作区未恢复的事实。

#### Scenario: Checkout degrades to transcript only
- **WHEN** 用户发起 `transcript_and_workspace` checkout，但结果降级为 transcript-only
- **THEN** 前端 SHALL 显式展示“仅恢复对话，未恢复工作区”
- **AND** SHALL 同时展示 degrade reason

#### Scenario: Branch restore or switch succeeds
- **WHEN** 用户执行 restore branch head、fork 或 switch branch
- **THEN** 前端 SHALL 明确反馈当前 `branch / visible node / mode` 已改变

### Requirement: Session control states SHALL share a unified user-facing vocabulary
Pony Agent 前端 SHALL 把 `live / historical / historical_dirty / paused / recovery-capable` 收口为统一用户状态语言，而不是直接暴露零散原始 flag。

其中：
- `historical / historical_dirty` SHALL 来源于既有 `historyCursorMode`
- `paused` SHALL 来源于既有 run / checkpoint / submission-plan 事实
- `recovery-capable` SHALL 由既有 `submissionPlan / checkpoint` 派生，而不是新增前端私有语义位

#### Scenario: Historical dirty mode is active
- **WHEN** 当前 session 处于 `historical_dirty`
- **THEN** 前端 SHALL 显示独立文案与行为提示
- **AND** SHALL NOT 只渲染原始 mode 字符串

#### Scenario: An action is disabled
- **WHEN** 某个 session control action 因运行态、空白会话、缺少可恢复 run / checkpoint 或 workspace rollback 不支持而被禁用
- **THEN** 前端 SHALL 提供用户可读的 disabled reason
