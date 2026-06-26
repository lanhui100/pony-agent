# Tasks: Unify Provider Retry And Backoff Boundary

## 1. Spec And Boundary Alignment

- [x] 1.1 新增 `PA-070` 任务卡，明确 provider request retry / phase fallback / turn-level retry 的边界
- [x] 1.2 新增 OpenSpec change：`unify-provider-retry-and-backoff-boundary`
- [x] 1.3 将本轮代码勘察与 3 路对抗式审核结论转写为 proposal / design / delta spec / tasks
- [x] 1.4 对 spec 做一轮独立审核，确认没有把 turn-level retry 与 provider request retry 混成一个实现要求
- [x] 1.5 将本轮审核采纳项补回 spec / design，收口 escalation contract、structured failure、budget 与 Retry-After 语义

## 2. Request-Level Retry Contract

- [x] 2.1 为 provider request retry 定义稳定的 core-side policy boundary
- [x] 2.2 明确 request-level budget、attempt 语义与 `TransientRetryable / NonRetryable / RequiresRequestMutation / UnsafeToRetry` 分类
- [x] 2.3 明确 `Retry-After` 参与 delay 选择时的限制条件与合并规则
- [x] 2.4 明确 provider retry 不得默认升级为 whole-turn resubmit
- [x] 2.5 定义 request-level 输出决策合同：`Retry / Fallback / Escalate / Abort`

## 3. Stream And Fallback Safety Boundary

- [x] 3.1 定义 `pre_connection / no_delta / reasoning_only / visible_text_started / tool_call_started` 五类 stream state
- [x] 3.2 明确何时允许 `stream -> retry`
- [x] 3.3 明确何时允许 `stream -> sync`
- [x] 3.4 明确何时必须停止自动 retry/fallback，以避免重复文本或重复工具调用
- [x] 3.5 明确 fallback source 等级与 terminal/trace 可观测字段
- [x] 3.6 明确 `tool_call_started` 的 runtime 判定信号，而不是仅停留在 UI 或文本层

## 4. Host And Frontend Responsibility

- [x] 4.1 明确 `src-tauri` 只负责 host bridge，不负责 retry policy
- [x] 4.2 盘点前端 existing whole-turn retry 与新 contract 的冲突点，并明确退场迁移路径
- [x] 4.3 决定前端 whole-turn retry 的后续方向：退场，不再保留静默自动重试；如需 turn-level retry，仅允许显式 control-plane action
- [x] 4.4 确保前端只消费状态，不复制 provider retry classifier/backoff 决策
- [x] 4.5 对齐 turn terminal reason、fallback source 与既有 `turn-lifecycle-event-contract` / `model-monitor-telemetry` 读面
- [x] 4.6 定义显式 turn-level retry 的最小 control-plane 合同，并确认它如何与既有 `run-control audit summary` 对齐
- [x] 4.7 采用复用既有 `start_graph_run_stream` 的方案，定义稳定 `start_reason`：至少区分 `initial_turn / explicit_retry_after_failure / replay_from_checkpoint / restart_from_checkpoint`
- [x] 4.8 定义显式 turn-level retry 的最小结果字段：触发来源、`start_reason`、是否进入 run-control summary family
- [x] 4.9 统一字段命名：采用 `startReason / failureClass / budgetExhaustedReason / fallbackSource`，并明确各字段的出现位置与缺省规则

## 5. Testability Contract

- [x] 5.1 定义最小可测抽象：delay planner、failure classification、next-action decision、sleeper injection
- [x] 5.2 明确 P0/P1/P2 测试分层，避免真实时间 sleep 成为主验证路径
- [x] 5.3 补 deterministic tests 所需的最小约束，而不是一次引入过重 fake time framework
- [x] 5.4 明确需要保留的少量 mock HTTP integration coverage，包括 timeout、Retry-After、empty/invalid body、rate-limit body、stream partial commit
- [x] 5.5 将 cancellation / deadline propagation 提升为显式验证项

## 6. Cross-Spec Alignment

- [x] 6.1 对齐 `turn-lifecycle-event-contract`：明确 retry/fallback 终态仍落到 canonical terminal vocabulary，补足 terminal payload 稳定字段
- [x] 6.2 对齐 `trace-persistence-and-recovery-contract`：明确 retry/fallback 证据进入 persisted truth，且不误称为 recovery checkpoint
- [x] 6.3 对齐 `model-monitor-telemetry`：明确 fallback source / failure class / budget exhausted reason 的监控读面消费方式
- [x] 6.4 对齐 `run-control-audit-surface-and-summary-first-explainability`：明确显式 turn-level retry 如何落入既有 run-control summary family
