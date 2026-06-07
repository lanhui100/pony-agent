# Tasks: Add History-State Hooks And Restore Boundaries

## 1. Spec and Task-System Sync

- [x] 1.1 新建 `PA-041` 任务卡并同步到当前任务系统
- [x] 1.2 完成 `add-history-state-hooks-and-restore-boundaries` 的 proposal / design / spec 文档

## 2. Contract Definition

- [x] 2.1 定义 history-state hook point 与 canonical binding
- [x] 2.2 定义 normalized history-control envelope、只读字段与白名单 transform 面
- [x] 2.3 定义 history-state evidence / reload / read-plane / degrade truth-source requirements

## 3. Implementation and Verification

- [x] 3.1 在 `session / runtime / control_plane` 落最小 history-state hook dispatch 闭环
  结果：`session::checkout_history_node(...) / restore_branch_head(...) / fork_from_history_node(...) / switch_history_branch(...)` 已接入 dispatch/evidence 闭环；`control_plane` 已补四类 command response 与 runtime view 的同口径 evidence 投影；前端 runtime store/types 也已对齐新 contract
- [x] 3.2 补 `checkout / restore / fork / switch / degrade` 的 Rust 定向测试
  结果：`checkout / restore / fork / switch / degrade` 已覆盖 success、blocked、degraded truth-source 与缺失 evidence reload 负向路径
- [x] 3.3 补 file-backed reload / runtime-view / control response 的读面回归
  进展目标：至少验证 control-plane 与 runtime view 对同一条 history-state evidence 的投影一致
  结果：file-backed reload、checkout response/runtime-view 一致性、restore/fork/switch response 投影均已覆盖；前端 runtime store/types 也已对齐同一 contract；剩余 host surface 审查转入后续实现跟进
- [x] 3.4 补 source-of-truth non-regression test，确保 `workspace_rollback_applied / degradation_reason / history cursor` 在 hook 存在时仍只由既有合同决定
- [x] 3.5 完成独立 spec 审核并采纳必要修订
- [x] 3.6 回写任务卡、review 文档、日志与验收证据
