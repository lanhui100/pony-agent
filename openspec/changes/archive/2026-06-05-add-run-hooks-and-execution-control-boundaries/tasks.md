# Tasks: Add Run Hooks And Execution Control Boundaries

## 1. Spec and Task-System Sync

- [x] 1.1 新建 `PA-038` 任务卡并同步到当前任务系统
- [x] 1.2 完成 `add-run-hooks-and-execution-control-boundaries` 的 proposal / design / spec 文档

## 2. Contract Definition

- [x] 2.1 定义 graph run / execution-control hook point 与 canonical binding
- [x] 2.2 定义 normalized run-control envelope 与 submission-plan mediation input
- [x] 2.3 定义 run-level hook failure policy / reload evidence / read-plane requirements

## 3. Implementation and Verification

- [x] 3.1 在 graph / execution_control / control_plane 落最小 hook dispatch 闭环
- [x] 3.2 补 stop / resume / wait_user / submission-plan 的 Rust 定向测试
- [x] 3.3 补 persisted reload / runtime view / session control 读面回归
- [x] 3.4 完成独立 spec 审核并采纳必要修订
- [x] 3.5 回写任务卡、review 文档、日志与验收证据
