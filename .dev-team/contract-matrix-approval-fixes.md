# 契约矩阵: 审批控制面与去墙钟契约修复 (ISSUE-20261111-001 & ISSUE-20261111-002)

## 1. 契约条款清单

| 编号 | 模块 | 承诺/规则 | 验证方式 |
| :--- | :--- | :--- | :--- |
| **C-APP-01** | `control_plane` | `approval_is_listable` 必须对所有状态为 `Pending` 的 `Approval` 返回 `true`，包括非路径授权审批（`approvalKind != "path_authorization"` 或缺省）。 | 单元测试：`approval_is_listable_accepts_non_path_authorization_approval` |
| **C-APP-02** | `control_plane` | 非路径审批通过 `list_pending_approvals` 返回，TUI 500ms 对账不会判定为 stale，不会误报 `APPROVAL_STALE_NOTICE`。 | 控制面集成测试 |
| **C-APP-03** | `turn_control` | 具备合法 `session_id` 的待决 `Approval` 挂起，不被 `suspension_is_unrecoverable` 误判为 unrecoverable（在宿主审批恢复开启时）。 | 单元测试：`turn_control` 挂起可恢复性断言 |
| **C-APP-04** | `governed_executor` | `plan_execution_approval` 授权检查不再校验 `now_ms < approval.expires_at_ms`，去墙钟化（ADR 0074），仅校验 `session_id` 与 `plan_id` 匹配。 | 单元测试：过期墙钟时间戳下的执行放行验证 |
| **C-APP-05** | `dispatcher` | 审批生成时的 `expires_at_ms` 仅作为追踪/审计元数据，消费侧绝不因墙钟时间拒绝合法 CAS。 | 契约对齐回归测试 |

## 2. 验收证据与测试收据

- **C-APP-01 & C-APP-02**: `cargo test -p pony-agent-core --lib wave65_r2_approval_tests`
  - 收据: `approval_is_listable_accepts_non_path_authorization_approval ... ok`
  - 收据: `approval_is_listable_covers_valid_expired_interaction_and_terminal_states ... ok`
- **C-APP-03**: `cargo test -p pony-agent-core --lib approval_suspension_recoverable`
  - 收据: `approval_suspension_recoverable_for_non_path_authorization_when_capability_enabled ... ok`
- **C-APP-04**: `cargo test -p pony-agent-core --lib governed_executor_plan_execution_approval_valid_past_wall_clock_expiry`
  - 收据: `governed_executor_plan_execution_approval_valid_past_wall_clock_expiry ... ok`
- **C-APP-05**: `cargo test -p pony-agent-core --test wave36_plan_execution_approval`
  - 收据: `9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 5.46s`
- **全量无退化回归**: `cargo test -p pony-agent-core --test tui_batch3_regression` (137/137 passed), `cargo test -p pony-agent-core --test approval_resume_e2e` (4/4 passed).
