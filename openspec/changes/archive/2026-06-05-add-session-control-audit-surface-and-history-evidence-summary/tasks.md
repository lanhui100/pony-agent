# Tasks: Add Session Control Audit Surface And History Evidence Summary

## 1. Spec and Task-System Sync

- [x] 1.1 新建 `PA-042` 任务卡并同步到当前任务系统
- [x] 1.2 完成 `add-session-control-audit-surface-and-history-evidence-summary` 的 proposal / design / spec 文档
- [x] 1.3 沉淀 `Session Control Plane` 架构基线文档

## 2. Contract Definition

- [x] 2.1 定义 `SessionControlAuditSummary` 合同与字段口径
- [x] 2.2 明确 `PA-042 v1` 只覆盖 history-control 四类命令，不吸收 run-control summary 范围
- [x] 2.3 明确 `action evidence summary` 与 `current context projection` 的分层
- [x] 2.4 明确 summary 与 `history_state_evidence / cursor truth-source` 的职责边界
- [x] 2.5 明确 evidence missing / blocked / degraded 三类读面语义

## 3. Implementation and Verification

- [x] 3.1 在后端实现 history-control audit summary 的统一投影
- [x] 3.2 把 summary 接入 `SessionSnapshot / SessionRuntimeView / history-control responses`
- [x] 3.3 把前端主展示改为优先消费 summary contract，不重做 `PA-037` 既有交互语义
- [x] 3.4 补 file-backed reload / runtime-view / response projection 一致性测试
- [x] 3.5 补 `evidence missing` 负向测试，证明缺失 summary 不会伪造成功结论
- [x] 3.6 补 truth-source non-regression 测试，证明 summary 不影响 cursor / branch / rollback 真值
- [x] 3.7 补前端 UI guard / runtime store 对 summary-first consumption 的回归测试
- [x] 3.8 完成独立 spec 审核并采纳必要修订
- [x] 3.9 回写任务卡、review 文档、日志与验收证据
