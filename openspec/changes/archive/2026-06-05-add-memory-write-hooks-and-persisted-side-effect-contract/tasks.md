# Tasks: Add Memory Write Hooks And Persisted Side-Effect Contract

## 1. Spec and Task-System Sync

- [x] 1.1 新建 `PA-039` 任务卡并同步到当前任务系统
- [x] 1.2 完成 `add-memory-write-hooks-and-persisted-side-effect-contract` 的 proposal / design / spec 文档

## 2. Contract Definition

- [x] 2.1 定义 normalized memory write intent、hook point 与 structured result envelope
- [x] 2.2 定义 `persisted_effect / replay_required` 的 evidence 与 recovery requirements
- [x] 2.3 定义 memory-write hooks 的 traceability / reload / read-plane contract
  进展：`memory_write_hook_trace_records` 已进入 `SessionState / SessionSnapshot / HistoryNode`，并随 session snapshot、history checkout、file backend reload 一起读回

## 3. Implementation and Verification

- [x] 3.1 在 session / store / control-plane 读面落最小 memory-write hook 闭环
- [x] 3.2 补 allow/deny/transform/persisted_effect/replay_required 的 Rust 定向测试
- [x] 3.3 补 reload / recovery / runtime-view 读面回归
- [x] 3.4 完成独立 spec 审核并采纳必要修订
- [x] 3.5 回写任务卡、review 文档、日志与验收证据
