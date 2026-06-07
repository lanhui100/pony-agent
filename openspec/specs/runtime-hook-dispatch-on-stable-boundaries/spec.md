## ADDED Requirements

### Requirement: Runtime SHALL dispatch hooks only on stable canonical boundaries
runtime SHALL 只在已真实发射并可验证的 canonical lifecycle boundary 上执行 hooks。

#### Scenario: Runtime dispatches a model-call-start hook
- **WHEN** turn 进入 `ModelCallStart`
- **THEN** runtime SHALL 按 registry 顺序执行该 boundary 上的 hooks
- **AND** runtime SHALL NOT 要求 hook 了解 provider 内部流解析细节

#### Scenario: Runtime ignores an unstable early boundary
- **WHEN** prepare/context build 早期 boundary 尚无真实 runtime 发射面
- **THEN** runtime SHALL NOT 为了接 hooks 人造假事件或假 phase
- **AND** 这些 boundary SHALL 保持在 contract 层，直到后续卡单独实现

### Requirement: Hook dispatch SHALL produce real trace evidence
stable-boundary dispatch 的执行结果 SHALL 进入实时事件与 persisted trace。

#### Scenario: A hook runs on a stable boundary
- **WHEN** 某个 hook 在稳定 boundary 上被 runtime 执行
- **THEN** `TurnStreamEvent` SHALL 携带对应 `hookTraceRecords`
- **AND** persisted `TurnTraceRecord` SHALL 保留同一组 records

#### Scenario: Reload reads runtime-produced hook trace records
- **WHEN** session snapshot 被 reload
- **THEN** control-plane / frontend store SHALL 能读回 runtime 真实产出的 hook trace records

### Requirement: Dispatch SHALL preserve ordering and controlled execution
runtime hook dispatch SHALL 维持 foundation 约定的顺序、失败语义与受控扩展边界。

#### Scenario: Multiple hooks share a boundary
- **WHEN** 同一 stable boundary 上注册多个 hooks
- **THEN** runtime SHALL 以 registry 声明的稳定顺序执行它们
- **AND** 该顺序 SHALL 可进入 trace evidence

#### Scenario: Hook execution fails on a stable boundary
- **WHEN** 某个 hook 在 dispatch 期间失败
- **THEN** runtime SHALL 按 descriptor 的 failure policy 处理
- **AND** 该失败结果 SHALL 进入 trace evidence

#### Scenario: First-wave dispatch stays non-destructive by default
- **WHEN** 本卡进行首轮 stable-boundary runtime dispatch
- **THEN** 系统 SHALL 以 observe / non-blocking dispatch 为默认口径
- **AND** 若某个 blocking 行为被纳入首轮范围，它 SHALL 具备独立测试覆盖 terminal outcome 与 trace evidence

#### Scenario: Dispatch completes without becoming a new scheduler
- **WHEN** runtime 完成本轮 hook dispatch 集成
- **THEN** hook SHALL NOT 直接改内部 store
- **AND** hook SHALL NOT 创建新的 lifecycle phase
- **AND** hook SHALL NOT 绕开 canonical model/tool path 触发额外调度

#### Scenario: Patch or side-effect results remain trace-first in this card
- **WHEN** hook 在 stable boundary 上返回 `patch` 或 `side-effect-request`
- **THEN** runtime SHALL 记录归一化 trace evidence
- **AND** 本卡 SHALL NOT 以此视为 patch / side-effect 正式 contract applier 已上线，除非后续变更单独启用

### Requirement: Delivery scope SHALL stay separate from PA-033 and PA-022
本卡 SHALL 只承接 stable-boundary runtime dispatch，不回吞 foundation 或 post-foundation 扩展范围。

#### Scenario: Delivery completes
- **WHEN** `PA-035` 完成交付
- **THEN** `PA-033` 仍保持 foundation/no-op contract 的完成边界
- **AND** `PA-022` 仍保留 `run / memory write / planner / skills / MCP` hooks 等 post-foundation 范围
- **AND** 验收证据 SHALL 回写到任务卡、review 文档与 session 日志
