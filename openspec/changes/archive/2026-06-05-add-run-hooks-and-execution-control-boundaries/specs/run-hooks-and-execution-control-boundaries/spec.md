## ADDED Requirements

### Requirement: Hooks SHALL attach only to stable graph-run and execution-control boundaries
系统 SHALL 只允许在稳定的 graph run / execution control boundary 上执行 run-level hooks。

#### Scenario: A hook observes a wait-user boundary
- **WHEN** graph run 到达 `wait_user`
- **THEN** 系统 SHALL 在对应 canonical boundary 上发射 hook dispatch
- **AND** hook SHALL NOT 依赖 graph 内部临时状态猜测该边界

#### Scenario: A hook tries to attach to an unstable run-internal step
- **WHEN** 某个 hook 试图挂接未承诺的 graph 内部实现步骤
- **THEN** 系统 SHALL 拒绝该 hook 注册或将其视为无效配置

### Requirement: Execution-control hooks SHALL consume normalized control envelopes
run-level hooks SHALL 只消费规范化的 control envelope，而不是直接操作底层 store。

#### Scenario: A hook participates in submission-plan arbitration
- **WHEN** 系统在 stop / resume / replay / continue 间做执行入口仲裁
- **THEN** hook SHALL 读取 normalized submission-plan envelope
- **AND** hooks SHALL NOT 直接绕开既有 arbitration truth-source

#### Scenario: A hook attempts to introduce a new execution command
- **WHEN** 某个 run-level hook 试图新增或替代既有 `start / continue / resume` execution command
- **THEN** 系统 SHALL 拒绝该结果
- **AND** hook SHALL NOT 成为第二 arbitration source

### Requirement: Run-level hook evidence SHALL persist across reload and read-plane surfaces
run/execution-control hooks 的执行证据 SHALL 进入 `GraphRun / GraphRunCheckpoint` persisted evidence，并被 runtime view 与 control-plane 读面读回。

#### Scenario: A session reloads after a resume-capable boundary
- **WHEN** reload 发生在 run-level hook 已执行之后
- **THEN** session drilldown / runtime view SHALL 能读回该 hook 的 boundary、结果与耗时

#### Scenario: Canonical control boundaries are projected as the minimum persisted audit chain
- **WHEN** run-level hooks 命中当前阶段承诺的 `submission_plan / wait_user / stop_requested / run_resume` boundary
- **THEN** 系统 SHALL 通过 `GraphRun / GraphRunCheckpoint` roundtrip 保留这些 boundary 的 evidence
- **AND** runtime view / control-plane SHALL 能读回至少 `boundary / result kind / duration`

### Requirement: Run-level hooks SHALL remain controlled extensions
run-level hooks SHALL 继续保持受控扩展面，而不是新的 graph scheduler。

#### Scenario: A run-level hook denies a resume path
- **WHEN** hook 返回阻断型结果
- **THEN** 系统 SHALL 以结构化 failure/guard contract 记录该决定
- **AND** hook SHALL NOT 直接修改 graph run store 或 turn runtime store
- **AND** hook SHALL NOT 直接推进 run phase 或替代 lifecycle truth-source
