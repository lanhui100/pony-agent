# PA-043 Spec Review

## 审核对象

- [PA-043 任务卡](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-043-build-run-control-audit-surface-and-summary-first-explainability.md)
- [proposal.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/archive/2026-06-05-add-run-control-audit-surface-and-summary-first-explainability/proposal.md>)
- [design.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/archive/2026-06-05-add-run-control-audit-surface-and-summary-first-explainability/design.md>)
- [tasks.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/archive/2026-06-05-add-run-control-audit-surface-and-summary-first-explainability/tasks.md>)
- [delta spec](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/archive/2026-06-05-add-run-control-audit-surface-and-summary-first-explainability/specs/run-control-audit-surface-and-summary-first-explainability/spec.md>)
- [canonical spec](</C:/Users/HUAWEI/Documents/pony-agent/openspec/specs/run-control-audit-surface-and-summary-first-explainability/spec.md>)
- [Session Control Plane 架构基线](</C:/Users/HUAWEI/Documents/pony-agent/docs/architecture/session-control-plane-and-audit-surface.md>)

## 审核方式

- 委派独立子智能体进行只读 spec 审核
- 关注范围边界、schema 刚性、truth-source guardrail、前端范围与验证矩阵

## 主要发现

1. `start_graph_run_stream` 被纳入 `PA-043 v1`，但 replay/restart 与普通首轮启动只有文案区分，没有机器可判定的强制字段，也缺少明确排除普通首轮启动的负向场景。
2. `RunControlAuditSummary` 的 schema 还不够刚：顶层结构、required/optional、枚举值域、`missing` 形态，以及 `target_summary / request_summary / recovery_mode` 的必填条件都没有被写死。
3. `current_context_projection` 与 reload roundtrip 的持久化边界不清晰，存在把 read-time context 冻结成伪 truth-source 的风险。
4. 前端范围虽然口头上已收紧，但还没被写成硬约束，仍可能在实现时回流成 `PA-037` 的 UI 重设计。
5. tasks 已有部分负向验证，但还缺普通首轮 start 排除、组合路径、current-context 漂移与前端 non-regression 的明确任务化。

## 采纳结论

以上发现全部采纳，并已据此修订：

- 为 `start_graph_run_stream` 新增硬性 `start_reason` 语义边界，明确：
  - `initial_turn` 不进入 run-control summary
  - 只有 `replay_from_checkpoint / restart_from_checkpoint` 才进入 `PA-043`
- `RunControlAuditSummary` 固定为：
  - `action_evidence_summary`
  - `current_context_projection`
- 明确 `target_summary` 为必填、`request_summary` 在 blocked 场景必填、`start_reason` 在 start 场景必填，并补充 `missing` 状态的固定 shape 约束
- 明确 reload roundtrip 只比较 `action_evidence_summary` required fields；`current_context_projection` 必须按 read-time truth-source 重新投影
- 前端范围进一步收紧为：
  - 只替换既有展示位的数据源
  - 不新增 panel/section
  - 不改 `HomeSessionSidebar` 结构
  - 不改 `HomeWorkspace` CTA/disabled reason 语义
- tasks 已显式补入：
  - 普通首轮 start 排除测试
  - `blocked + evidence missing`
  - `degraded + replay_required`
  - current-context 漂移不改历史 action evidence
  - 前端“只换数据源、不改布局/CTA” non-regression
  - 独立 acceptance audit

## 当前判断

- 修订后可以进入实现准备
- 实现前仍需继续守住三条 guardrail：
  1. run-control summary 是 audit read-model，不是 arbitration input
  2. `current_context_projection` 不能伪装成历史动作证据
  3. 普通 `start_graph_run_stream` 不能被误吸收入 run-control summary
