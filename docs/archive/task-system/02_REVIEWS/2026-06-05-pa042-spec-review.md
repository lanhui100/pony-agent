# PA-042 Spec Review

## 审核对象

- [PA-042 任务卡](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-042-build-session-control-audit-surface-and-history-evidence-summary.md)
- [proposal.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-session-control-audit-surface-and-history-evidence-summary/proposal.md>)
- [design.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-session-control-audit-surface-and-history-evidence-summary/design.md>)
- [tasks.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-session-control-audit-surface-and-history-evidence-summary/tasks.md>)
- [delta spec](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-session-control-audit-surface-and-history-evidence-summary/specs/session-control-audit-surface-and-history-evidence-summary/spec.md>)
- [Session Control Plane 架构基线](</C:/Users/HUAWEI/Documents/pony-agent/docs/architecture/session-control-plane-and-audit-surface.md>)

## 审核方式

- 委派独立子智能体进行只读 spec 审核
- 关注范围边界、可验证性、truth-source guardrail 与任务拆分完整性

## 主要发现

1. `PA-042` 的范围偏宽，`Session Control Plane` 的长期语义与本卡 `v1` 落地面没有切开，存在重新吸收 `PA-037 / PA-038 / PA-041` 范围的风险。
2. summary contract 仍偏描述性，缺少 required/optional 字段、跨读面一致性判定与 unavailable 语义的刚性定义。
3. summary 混入当前 session 上下文字段，但未明确这些字段是否属于动作证据，存在 read model 反向污染 truth-source 的风险。
4. 前端 explainability 与 `PA-037` 的 UX 范围仍可能重叠，需要明确“只切数据源，不重做交互语义”。
5. tasks 缺少 `evidence missing` 与 `summary 不能成为仲裁输入` 两类关键负向验证。

## 采纳结论

以上发现全部采纳，并已据此修订：

- `PA-042 v1` 明确只覆盖 `history checkout / restore / fork / switch`
- run-control summary family 明确移出本卡
- `SessionControlAuditSummary` 明确拆成：
  - `action evidence summary`
  - `current context projection`
- required fields 与跨 `snapshot / runtime view / response` 的一致性要求已写入 design/spec
- tasks 已显式补入：
  - `evidence missing` 负向验证
  - truth-source non-regression 验证
- 前端范围已收紧为：
  - `summary-first consumption`
  - 不重做 `PA-037` 已成立的按钮、disabled reason 与状态语言

## 当前判断

- 修订后可以进入实现
- 实现时仍需优先守住两条 guardrail：
  1. summary 只是审计读面，不是仲裁输入
  2. current context 不能伪装成 action evidence
