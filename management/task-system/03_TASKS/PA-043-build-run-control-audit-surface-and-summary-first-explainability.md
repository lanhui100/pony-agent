# PA-043 构建 run-control audit surface 与 summary-first explainability

## 状态
- Status: `Done`
- Priority: `P1`
- Owner: `Codex`

## OpenSpec Change
- 归档后路径：
  [2026-06-05-add-run-control-audit-surface-and-summary-first-explainability](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/archive/2026-06-05-add-run-control-audit-surface-and-summary-first-explainability>)

## Delta Spec
- 归档后路径：
  [run-control-audit-surface-and-summary-first-explainability/spec.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/archive/2026-06-05-add-run-control-audit-surface-and-summary-first-explainability/specs/run-control-audit-surface-and-summary-first-explainability/spec.md>)

## Canonical Spec
- [run-control-audit-surface-and-summary-first-explainability/spec.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/specs/run-control-audit-surface-and-summary-first-explainability/spec.md>)

## Spec 状态
- Proposal: `ready`
- Spec: `ready`
- Design: `ready`
- Tasks: `ready`

## 目标
把已经完成的 `run hooks / execution-control boundary evidence`、`session control UX` 与 `submission-plan / checkpoint` 恢复语义，收口成一个稳定、可持久化、可 reload、可前端直接消费的 `Run Control audit surface v1`。本卡只覆盖 `stop / continue / resume / replay(start)` 这一组 run-control summary contract，让用户和开发者都能明确看见“刚刚发生了什么运行控制动作、命中了哪条 boundary、结果为何 blocked / degraded / replay_required”，而不是继续依赖前端私有解释链。

## 输出
- run-control audit summary read model
- run-control summary contract
- runtime view / session snapshot / run-control response 的统一读面
- `HomeSessionSidebar` / `HomeWorkspace` 或同等前端面的 summary-first explainability 提升
- 对 run-control audit summary 的前后端验证与 UI guard 测试

## 范围边界
- 本卡不重做 `PA-031 ~ PA-042` 已完成的 lifecycle / trace / hooks / history-control foundation
- 本卡 v1 只覆盖 `stop / continue / resume / replay(start_graph_run_stream)` 四类 run-control 动作
- 普通首轮 `start_graph_run_stream` 明确不属于本卡；只有 `replay_from_checkpoint / restart_from_checkpoint` 语义才进入 run-control summary
- 本卡不重做 `PA-037` 已成立的按钮编排、disabled reason 与状态语言，只切换主展示数据源
- 本卡不重做新的视觉层、交互信息架构或整块 session control UI
- 本卡不新增新的 graph run command、checkpoint command 或 scheduler 能力
- 本卡不让 run-control audit summary 反向成为 `submission plan / checkpoint / graph run phase` 的 truth-source

## 验收标准
- 后端 SHALL 提供稳定的 `run-control audit summary`，且 v1 只针对 `stop / continue / resume / replay(start)` 四类命令生效
- summary SHALL 拆分为 `action evidence summary` 与 `current context projection` 两层，避免把当前 paused/running/ready 现态伪装成历史动作结果
- `action evidence summary` 的必填字段 SHALL 至少包括 `status / sourceFamily / commandKind / boundary / resultKind / summary / elapsedMs / blocked / degraded / evidenceId / observedAtMs`
- summary 顶层 SHALL 固定为 `action_evidence_summary + current_context_projection` 两层
- `action_evidence_summary.target_summary` SHALL 为必填；`request_summary` 在 blocked 场景下 SHALL 为必填；`start_reason` 在 `start_graph_run_stream` 场景下 SHALL 为必填
- session snapshot、runtime view 与 run-control command response SHALL 对 `action evidence summary` 投影同一口径的 required fields，而不是各自重算
- reload 后，前端仍能从后端读回最近一次 run-control action summary；关闭应用再打开 session 时，这份 summary 不得无故消失
- reload roundtrip 默认只对 `action_evidence_summary` required fields 做保真比较；`current_context_projection` 必须按读取时真相源重新投影
- 当前若缺少底层 evidence，系统 SHALL 只表现为“summary unavailable / evidence missing”，而不得伪造 continue/resume/replay 结论
- 前端 SHALL 能明确展示：
  - 最近一次 run-control action
  - 命中的 boundary
  - 是否 blocked / degraded / replay_required
  - 来自动作证据的 target 摘要
  - 来自 current context projection 的 checkpoint / run / phase 摘要
- 前端 SHALL 不再依赖私有字段兼容链去推导 run-control audit feedback；兼容字段可以保留，但主展示必须优先消费新 summary contract
- 前端 SHALL 只在既有展示位完成 summary-first 数据源切换，不新增 panel/section，不改 `HomeSessionSidebar` 结构，也不改 `HomeWorkspace` CTA/disabled reason 语义
- 测试至少覆盖：
  - stop summary
  - continue / resume / replay(start) summary
  - 普通首轮 `start_graph_run_stream` 不进入 run-control summary
  - blocked/degraded summary
  - `blocked + evidence missing`、`degraded + replay_required` 组合路径
  - file-backed reload summary roundtrip
  - reload 后 current-context 漂移但历史 action evidence 不变
  - runtime view / control-plane response summary projection 一致性
  - evidence missing 负向路径
  - summary 不影响 submission plan / checkpoint / graph phase 真值的防回归
  - 前端 summary 展示与 explainability 数据源切换回归
  - 前端“只换数据源、不改布局/CTA”的 non-regression

## 当前进展
- `SessionSnapshot / SessionRuntimeView / GraphRunControlResponse / GraphRunStreamStartResponse` 已统一接入 `run_control_audit_summary`
- `HomeWorkspace` / `HomeSessionSidebar` 已完成 run-control `summary-first` 主消费，旧字段只保留 fallback，不重做 `PA-037` 既有交互结构
- `tests/runtime-store.spec.ts`、`tests/HomeWorkspace.spec.ts`、`tests/HomeSessionSidebar.spec.ts` 已补齐 run-control summary hydration、explainability 与 non-regression 回归
- `control_plane --no-run` 独立 `target` 编译验证已通过；`graph_run_stream_can_start_continue_and_resume` 与普通首轮 `start_graph_run_stream` 不进入 summary 的定向测试已补齐

## 下一步动作
- 当前卡已完成 acceptance audit 与 closeout
- 后续若继续扩展 run-control summary family，应新开任务承接，而不是继续扩写 `PA-043`

## 当前卡点
- 无

## 断点续跑提示
继续前先看：
- `docs/architecture/session-control-plane-and-audit-surface.md`
- `management/task-system/03_TASKS/PA-037-build-session-control-surface-and-feedback-loop.md`
- `management/task-system/03_TASKS/PA-038-build-run-hooks-and-execution-control-boundaries.md`
- `management/task-system/03_TASKS/PA-042-build-session-control-audit-surface-and-history-evidence-summary.md`
- `src-tauri/src/agent/control_plane.rs`
- `src-tauri/src/agent/execution_control.rs`
- `src-tauri/src/agent/graph.rs`
- `src/stores/runtime.ts`
- `src/components/HomeSessionSidebar.vue`
- `src/components/HomeWorkspace.vue`
