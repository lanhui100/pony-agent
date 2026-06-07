# PA-042 构建 session control audit surface 与 history evidence summary

## 状态
- Status: `Done`
- Priority: `P1`
- Owner: `Codex`

## OpenSpec Change
- [add-session-control-audit-surface-and-history-evidence-summary](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/archive/2026-06-05-add-session-control-audit-surface-and-history-evidence-summary>)

## Delta Spec
- [session-control-audit-surface-and-history-evidence-summary/spec.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/archive/2026-06-05-add-session-control-audit-surface-and-history-evidence-summary/specs/session-control-audit-surface-and-history-evidence-summary/spec.md>)

## Canonical Spec
- [openspec/specs/session-control-audit-surface-and-history-evidence-summary/spec.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/specs/session-control-audit-surface-and-history-evidence-summary/spec.md)

## Spec 状态
- Proposal: `ready`
- Spec: `ready`
- Design: `ready`
- Tasks: `ready`

## 目标
把已经完成的 `history-state evidence`、`session control UX` 和 `history degrade feedback`，收口成一个稳定、可持久化、可 reload、可前端直接消费的 `Session Control Plane audit surface v1`。本卡的 v1 只覆盖 `history checkout / restore / fork / switch` 这组 history-control summary contract，让用户和开发者都能明确看见“刚刚发生了什么控制动作、命中了哪条 boundary、结果为何 degraded 或 blocked”，而不是继续依赖前端私有解释链。

## 输出
- `Session Control Plane` 正式命名与架构基线文档
- history-state audit summary read model
- history-state evidence summary contract
- runtime view / session snapshot / history-control response 的统一读面
- `HomeSessionSidebar` 或同等前端面的 summary-first explainability 提升
- 对 audit summary 的前后端验证与 UI guard 测试

## 范围边界
- 本卡不重做 `PA-031 ~ PA-041` 已完成的 lifecycle / trace / hooks foundation
- 本卡 v1 只覆盖 `history checkout / branch restore / branch fork / branch switch`
- 本卡不新增 `stop / continue / resume / replay` 的 response contract 或新的 run-control summary family
- 本卡不让前端直接消费完整 persisted evidence chain 并自行仲裁结论
- 本卡不引入新的 history command、resume command 或独立 replay backend 命令
- 本卡不让 audit summary 反向成为 `restore / cursor / branch / rollback` 的 truth-source
- 本卡不重做 `PA-037` 已成立的按钮编排、disabled reason 与状态语言，只验证数据源切换后语义不回退

## 验收标准
- 后端 SHALL 提供稳定的 `history-state audit summary`，且 v1 只针对 `checkout / restore / fork / switch` 四类命令生效
- summary SHALL 拆分为 `action evidence summary` 与 `current context projection` 两层，避免把当前态伪装成历史动作结果
- `action evidence summary` 的必填字段 SHALL 至少包括 `status / sourceFamily / commandKind / boundary / resultKind / summary / elapsedMs / blocked / degraded / evidenceId / observedAtMs`
- session snapshot、runtime view 与 history-control command response SHALL 对 `action evidence summary` 投影同一口径的 required fields，而不是各自重算
- reload 后，前端仍能从后端读回最近一次 control action summary；关闭应用再打开 session 时，这份 summary 不得无故消失
- 当前若缺少底层 evidence，系统 SHALL 只表现为“summary unavailable / evidence missing”，而不得伪造 restore、rollback 或 branch 结论
- 前端 SHALL 能明确展示：
  - 最近一次 control action
  - 命中的 boundary
  - 是否 degraded
  - transcript-only / transcript+workspace 的恢复结果
  - 来自动作证据的 target 摘要
  - 来自 current context projection 的 branch / visible node 摘要
- 前端 SHALL 不再依赖私有字段兼容链去推导 history-control audit feedback；兼容字段可以保留，但主展示必须优先消费新 summary contract
- 测试至少覆盖：
  - history checkout summary
  - branch restore / fork / switch summary
  - blocked/degraded summary
  - file-backed reload summary roundtrip
  - runtime view / control-plane response summary projection 一致性
  - evidence missing 负向路径
  - summary 不影响 cursor / branch / rollback 真值的防回归
  - 前端 summary 展示与 explainability 数据源切换回归

## 当前进展
- `PA-037` 已完成 session control 第一轮显式 UX，但解释面仍偏分散
- `PA-041` 已完成 history-state evidence persistence / reload / read-plane 基线
- 现阶段的主要缺口不是“没有 evidence”，而是“缺少更稳定的 history-control audit summary 与前端 explainability 读面”
- 已把当前讨论沉淀为架构基线文档：
  - [session-control-plane-and-audit-surface.md](/C:/Users/HUAWEI/Documents/pony-agent/docs/architecture/session-control-plane-and-audit-surface.md)
- 后端已完成 `HistoryStateAuditSummary` 投影与接线：
  - `SessionSnapshot.history_state_audit_summary`
  - `SessionRuntimeView.history_state_audit_summary`
  - `HistoryCheckoutResponse / RestoreBranchHeadResponse / ForkFromHistoryNodeResponse / SwitchHistoryBranchResponse`
- 前端已完成 summary-first 主消费：
  - `runtime store` 通过 `latestHistoryStateAuditSummary` 统一承接 runtime view、snapshot 与四类 history-control response
  - `HomeSessionSidebar` 已拆分展示“最近动作证据”与“当前上下文（非动作证据）”
- 已补齐 `PA-042` 本轮关键定向验证：
  - Rust：`history_checkout_response_and_runtime_view_share_same_history_state_evidence_projection`
  - Rust：`missing_history_state_evidence_does_not_reconstruct_restore_conclusion_after_reload`
  - Frontend：`tests/HomeSessionSidebar.spec.ts`
  - Frontend：`tests/runtime-store.spec.ts`
  - Backend compile gate：`cargo check --manifest-path src-tauri/Cargo.toml --lib`
- 已完成 acceptance audit 与 closeout：
  - [2026-06-05-pa042-acceptance-audit.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/02_REVIEWS/2026-06-05-pa042-acceptance-audit.md)
  - [2026-06-05-session-74-pa042-closeout.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/99_LOGS/2026-06-05-session-74-pa042-closeout.md)

## 下一步动作
- 当前卡已完成 closeout；后续如需扩展 run-control summary family，应以新卡承接

## 当前卡点
- 暂无

## 断点续跑提示
继续前先看：
- `docs/architecture/session-control-plane-and-audit-surface.md`
- `management/task-system/03_TASKS/PA-037-build-session-control-surface-and-feedback-loop.md`
- `management/task-system/03_TASKS/PA-038-build-run-hooks-and-execution-control-boundaries.md`
- `management/task-system/03_TASKS/PA-041-build-history-state-hooks-and-restore-boundaries.md`
- `src-tauri/src/agent/session.rs`
- `src-tauri/src/agent/control_plane.rs`
- `src/stores/runtime.ts`
- `src/types/runtime.ts`
- `src/components/HomeSessionSidebar.vue`
