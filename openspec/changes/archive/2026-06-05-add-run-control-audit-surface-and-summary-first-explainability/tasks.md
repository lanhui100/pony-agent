# Tasks: Add Run Control Audit Surface And Summary-First Explainability

## 1. Spec and Task-System Sync

- [x] 1.1 新建 `PA-043` 任务卡并同步到当前任务系统
- [x] 1.2 完成 `add-run-control-audit-surface-and-summary-first-explainability` 的 proposal / design / spec 文档
- [x] 1.3 在架构基线文档中补 `run-control summary family` 的下一轮承接说明
- [x] 1.4 完成独立 spec 审核并采纳必要修订

## 2. Contract Definition

- [x] 2.1 定义 `RunControlAuditSummary` 合同与字段口径
- [x] 2.2 明确 `PA-043 v1` 只覆盖 `stop / continue / resume / replay(start)` 四类命令，并排除普通首轮 `start_graph_run_stream`
- [x] 2.3 明确 `action evidence summary` 与 `current context projection` 的分层
- [x] 2.4 明确 summary 与 `submission_plan / execution_checkpoint / graph run phase truth-source` 的职责边界
- [x] 2.5 明确 evidence missing / blocked / degraded / replay_required 四类读面语义
- [x] 2.6 明确 `start_reason`、`request_summary`、`target_summary` 与 `recovery_mode` 的必填条件
- [x] 2.7 明确 reload 只对 `action_evidence_summary` required fields 做 roundtrip 保真

## 3. Implementation and Verification

- [x] 3.1 在后端实现 run-control audit summary 的统一投影
- [x] 3.2 把 summary 接入 `SessionSnapshot / SessionRuntimeView / run-control responses`
- [x] 3.3 把前端主展示改为优先消费 run-control summary contract，不重做 `PA-037` 既有交互语义
- [x] 3.4 补 file-backed reload / runtime-view / response projection 一致性测试，仅对 `action_evidence_summary` required fields 做 roundtrip 比较
- [x] 3.5 补 `evidence missing` 负向测试，证明缺失 summary 不会伪造 continue/resume/replay 结论
- [x] 3.6 补 truth-source non-regression 测试，证明 summary 不影响 submission plan / checkpoint / graph phase 真值
- [x] 3.7 补前端 UI guard / runtime store 对 summary-first consumption 的回归测试
- [x] 3.8 补普通首轮 `start_graph_run_stream` 不进入 run-control summary 的负向测试
- [x] 3.9 补 `blocked + evidence missing`、`degraded + replay_required` 的组合路径测试
- [x] 3.10 补 current-context 漂移不改历史 action evidence 的回归测试
- [x] 3.11 补前端“只换数据源、不改布局/CTA”的 non-regression 测试
- [x] 3.12 完成独立 acceptance audit，逐条覆盖本卡六个审核目标
- [x] 3.13 回写任务卡、review 文档、日志与验收证据
