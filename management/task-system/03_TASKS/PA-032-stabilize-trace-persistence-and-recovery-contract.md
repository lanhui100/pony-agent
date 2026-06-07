# PA-032 收口 trace persistence 与 recovery contract

## 状态
- Status: `Done`
- Priority: `P0`
- Owner: `Codex`

## OpenSpec Change
- [add-trace-persistence-and-recovery-contract](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-trace-persistence-and-recovery-contract)

## Delta Spec
- [trace-persistence-and-recovery-contract/spec.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-trace-persistence-and-recovery-contract/specs/trace-persistence-and-recovery-contract/spec.md)

## Canonical Spec
- 交付后沉淀到 `openspec/specs/` 的 trace persistence / recovery canonical spec

## Spec 状态
- Proposal: `ready`
- Spec: `ready`
- Design: `ready`
- Tasks: `ready`

## 目标
让 trace、checkpoint、history、workspace rollback 的语义具备正式边界，使重启恢复、历史回放与前端消费不再依赖脆弱的临时补偿逻辑。

## 输出
- backend trace persistence source-of-truth 边界
- runtime checkpoint 与 recovery checkpoint 的语义区分
- transcript-only / transcript+workspace / degraded 的结果合同
- 前端 hydration / restore 的消费边界
- 恢复与降级测试矩阵

## 验收标准
- 已持久化 trace 在应用重启后可恢复，且以后端状态为准
- 系统能明确区分运行中 checkpoint 与可恢复 checkpoint
- history restore 对 transcript 与 workspace 的结果有显式口径
- 前端 fallback 只在受控环境中存在，且不会生成或覆盖 canonical trace metrics
- Rust/前端测试覆盖 roundtrip、reload、degrade path 与 hydration 保真

## 当前进展
- 已建立 OpenSpec change：`add-trace-persistence-and-recovery-contract`
- 已把问题边界从“session UI 不满意”收口为 persistence / recovery contract 问题
- 已完成独立 spec 审核并采纳修订，见：
  [2026-06-03-pa031-pa032-pa033-spec-review.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/02_REVIEWS/2026-06-03-pa031-pa032-pa033-spec-review.md)
- 已完成 acceptance audit，见：
  [2026-06-04-pa032-acceptance-audit.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/02_REVIEWS/2026-06-04-pa032-acceptance-audit.md)
- 已启动第一刀实现：
  - `ExecutionCheckpoint` 读面开始显式暴露 `checkpointKind / resumable / replayable`
  - 当前 runtime 内控制用 checkpoint 明确标记为 `runtime_control`
  - 前端 `ExecutionCheckpoint` 类型与测试工厂已对齐新合同字段
- 已完成首轮验证：
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib stop_turn_and_checkpoint_queries_share_same_registry_surface` 通过
  - `npm run test:unit -- --run tests/runtime-store.spec.ts` 通过
  - `npx vue-tsc --noEmit` 通过
- 已完成第二轮 recovery 锚点收口：
  - persisted `TurnTraceRecord` 已显式持有 `sessionId / eventId / eventType / eventVersion / sequence / emittedAtMs`
  - terminal lifecycle event metadata 已通过 `control_plane -> runtime -> session store` 回写到已持久化 trace
  - Rust roundtrip 测试已覆盖“先持久化 trace，再回写 terminal event 锚点，reload 后仍完整保留”
- 已完成第三轮 recovery checkpoint 读面收口：
  - 统一 `load_execution_checkpoint(...)` 在缺少 runtime turn checkpoint 时，已可回退投影 graph run checkpoint
  - graph run recovery checkpoint 读面显式标记 `checkpointKind = recovery`
  - `resumable / replayable` 已从 graph checkpoint 语义投影到统一 checkpoint 读面
- 已完成第四轮前端恢复决策收口：
  - `runtime store` 已显式保存最近一次 `ExecutionCheckpoint`，作为 session hydration 后的本地恢复决策输入
  - `submitTurn()` 在 `retrievedContext.runState` 缺失时，已可基于 `recovery checkpoint + activeRunId` 选择 `resume_graph_run_stream`
  - 已新增前端回归测试，覆盖“runState 缺失但 recovery checkpoint 存在时仍正确 resume”
- 已完成第五轮 history restore 结果合同收口：
  - `HistoryCheckoutResult / HistoryRestoreResult` 已统一显式暴露 `transcriptRestoreApplied / workspaceRestoreCapable / workspaceRestoreApplied / degradedToTranscriptOnly / degradationReason`
  - 后端 `checkout_history_node / restore_branch_head` 已返回 transcript/workspace 双维结果，不再要求前端自行推测恢复是否降级
  - 前端 fallback 与 runtime store 已对齐同一套结果字段，避免 restore UI 与桌面 runtime 出现双重口径
- 已完成第六轮 canonical trace recovery 锚点收口：
  - 前端 `runtime store` 在消费 `turn:started / delta / trace / tool / completed / failed / cancelled` 时，已将 `eventId / eventType / eventVersion / sequence / emittedAtMs` 回写到 `turnTraceHistory`
  - 运行中 trace 与 reload 后 persisted trace 现在共享同一组 terminal lifecycle 锚点，不再只由后端 roundtrip 覆盖这部分语义
  - 前端单测已覆盖 canonical event metadata 会随 terminal event 落入 persisted turn trace
- 已完成第七轮 canonical event gate 收口：
  - 前端 `runtime store` 已基于 `eventId / sequence / emittedAtMs` 为单 turn 建立事件游标
  - 重复事件与低序号乱序事件不再覆盖更晚的 canonical trace / tool state / assistant message
  - 前端回归测试已覆盖“先接收较新事件，再收到旧序号 delta/重复 tool 事件时保持当前 trace 不回退”
- 已完成第八轮 recovery mode 读面收口：
  - `ExecutionCheckpoint` 已显式暴露 `recoveryMode`
  - `runtime_control` checkpoint 现在明确标记为 `replay_required`
  - graph run 投影出的 `recovery` checkpoint 现在明确标记为 `persisted_effect`
- 已完成第九轮 replay / resume 前端决策收口：
  - 前端 `runtime store` 已将 `recoveryMode` 接入 submission 决策
  - `persisted_effect` recovery checkpoint 允许自动走 `resume_graph_run_stream`
  - `replay_required` recovery checkpoint 不再误走 resume，而是回退到新开 graph run
- 已完成第十轮恢复合同优先级仲裁：
  - 当 `retrievedContext.runState` 与 recovery checkpoint 合同冲突时，前端 submission 决策现在以 checkpoint 的 `recoveryMode` 为准
  - `paused` runState 不再能覆盖 `replay_required` checkpoint 去误触发 resume
  - 前端回归测试已覆盖“runState 要求 resume，但 checkpoint 要求 replay 时，最终走 start_graph_run_stream”
- 已完成第十一轮 recovery capability 语义校正：
  - graph run 投影出的 `recovery` checkpoint 不再把 `replayable` 错误绑定到 `resumable`
  - `replay_required` recovery checkpoint 现在仍会显式暴露 `replayable=true`
  - 后端单测已覆盖“non-resumable recovery checkpoint 仍必须暴露 replayable contract”
- 已完成第十二轮 session 级 checkpoint 仲裁收口：
  - `load_execution_checkpoint(session_id=...)` 已补齐 session 级查询优先级仲裁
  - 运行中的 turn 现在优先暴露 `runtime_control`；当 graph run 到达可恢复边界后，session 级查询会切换为 graph-projected `recovery`
  - 后端单测已覆盖“同一 session 查询从 runtime_control 切到 recovery”的合同
- 已完成第十三轮 recovery/lifecycle 消费合同后端化：
  - 后端 `ExecutionCheckpoint` 已显式补齐 `contractVersion / runId / projectedRuntimePhase / submissionCommand`
  - 前端 `runtime store` 现在优先消费后端投影的恢复命令与 phase，不再总是依赖 `runState + activeRunId + recoveryMode + phase` 的本地推断
  - 已新增后端单测，覆盖 recovery checkpoint projection 会稳定产出 `submissionCommand / projectedRuntimePhase`
- 已完成第十四轮 submission plan 仲裁收口：
  - 后端已新增 `resolve_graph_run_submission_plan`，在 checkpoint 不足时可统一仲裁 `start / resume / continue`
  - 前端 `submitTurn()` 现在已改为后端 submission plan 优先；仅当后端计划不可用时，才回退到本地 `runState / checkpoint` 兼容路径
  - 已新增后端单测，分别覆盖“缺 checkpoint 时回退 graph run source”与“存在 recovery checkpoint 时优先采用 checkpoint source”
  - 已新增前端回归测试，覆盖“本地 stale runState 与后端 plan 冲突时，以后端 plan 为准”
- 已完成第十五轮 reload/hydration 执行计划收口：
  - `load_session_runtime_view(...)` 已显式承接 `submissionPlan`
  - reload roundtrip 测试已覆盖：当 session 只剩 persisted trace / lifecycle boundary evidence 时，runtime view 仍会稳定给出 `submissionPlan=default -> start_graph_run_stream`
  - 这保证了 hydration 后的执行入口仲裁不再需要前端猜测“完成态 session 下一次应该 continue 还是 fresh start”
- 已完成第十六轮后端最终仲裁证据补强：
  - `control_plane` 已新增 `submission_plan_starts_fresh_run_when_recovery_contract_requires_replay`
  - `control_plane` 已新增 `submission_plan_switches_with_session_checkpoint_boundary`
  - 前者直接验证：当 recovery contract 明确要求 `replay_required` 时，后端 submission plan 会稳定回到 `start_graph_run_stream`
  - 后者直接验证：同一 session 在 turn boundary 前后，submission plan 会从运行中 `graph_run -> continue` 切换到恢复态 `checkpoint -> resume`
  - 已通过独立 `CARGO_TARGET_DIR` 运行 exact Rust 单测，拿到稳定逻辑执行结论

## 当前判断
- `PA-032` 的实现级与验收级证据已经基本闭环：
  - replay / resume 最终仲裁已有后端 exact Rust 证据
  - session 级 checkpoint 切换已有后端 exact Rust 证据
  - reload / hydration 已有后端 exact Rust 证据与前端回归证据
- 当前更适合进入 acceptance audit / review，而不是继续补新的 recovery 主逻辑

## 下一步动作
- 继续把 `PA-032` 作为 recovery contract 真相源供 `PA-034 / PA-037` 与后续任务消费
- 如需归档对应 OpenSpec change，按单独归档流程执行

## 当前卡点
- 暂无；本卡已完成关闭

## 断点续跑提示
继续前先看：
- `src-tauri/src/agent/session.rs`
- `src-tauri/src/agent/execution_control.rs`
- `src-tauri/src/agent/control_plane.rs`
- `src/stores/runtime.ts`
- `docs/architecture/turn-lifecycle-hooks-and-recovery.md`
