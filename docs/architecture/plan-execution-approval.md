# Plan Execution Approval（宿主执行审批，W36 / PA-141，方案 (b)）

状态：已实施（wave-36，契约 `.dev-team/contracts/wave-36-pa138-145.md` §4，C-141-1..9）。
关联决策：`docs/decisions/0038-wave35-suite-remediation.md` F-7（已裁决条目）。

## 1. 问题

Plan 草稿态写保护（`plan_draft_gate`）豁免 `plan_control` 自身——否则 Plan 永远无法离开
Draft（自锁死）。该豁免带来安全姿态缺口：模型可以在 Draft 态自行调用
`plan_control op=start_execution`（或对 Draft plan `complete_step`）完成
Draft→Executing 迁移，从而解锁普通写/执行工具，无需任何宿主参与。

## 2. 裁决（用户方案 (b)：宿主审批）

Draft→Executing 迁移必须持有**宿主注入**的审批证据，模型不可自行产生。

### 2.1 证据类型与传递（C-141-1）

```rust
// crates/pony-agent-core/src/agent/dispatcher.rs
pub struct PlanExecutionApproval {
    pub session_id: String,
    pub plan_id: String,
    pub expires_at_ms: u64,
}
```

- serde 形状冻结：camelCase（`sessionId` / `planId` / `expiresAtMs`）。
- 唯一传递通道是宿主控制的调用上下文：
  - `DispatchContext.plan_execution_approval`（serde 键 `planExecutionApproval`，缺省 `None`）；
  - `TurnInput.plan_execution_approval`（同键），由 runtime 在 `set_context` 时透传进 `DispatchContext`。
- **模型工具参数不是审批来源**（C-141-3）：策略评估只读 `DispatchContext`；
  把审批塞进 `plan_control` 参数（`planExecutionApproval`/`approval` 等键）不产生任何效力，
  且 `plan_control` schema `additionalProperties:false` 本身会以 `invalid_arguments` 拒绝。

### 2.2 绑定与 TTL（C-141-4/5）

审批有效当且仅当全部成立：

1. `approval.session_id == context.session_id`（会话绑定）；
2. `approval.plan_id == 目标 plan 的 plan_id`（计划绑定，跨 plan 复用无效）；
3. `now_ms < expires_at_ms`（TTL，严格小于；时钟为 `SystemTime::now()`）。

任一不成立（缺失 / 跨会话 / 跨 plan / 过期）⇒ 无效。

### 2.3 触发面与一次性效果（C-141-6/8）

`GovernedPolicyEvaluator`（`governed_executor.rs`）在 `plan_control` 上判定：

- `op == "start_execution"`：始终需要审批（其唯一语义就是 Draft→Executing）。
- `op == "complete_step"` 且目标 plan 当前 `lifecycle == Draft`：需要审批
  （`complete_step` 是第二条 Draft→Executing 迁移路径）。
- 其余 op（`create` / `merge` / `replace` …）与 **Executing 状态的 `complete_step`
  不受影响**（仅迁移受影响）；`abort` 亦不受影响。

审批是**一次性效果**：它只解锁目标 plan 的本次迁移判定，不是会话级通行证——
同一审批不能授权同会话的其它 Draft plan（plan_id 绑定保证）。

### 2.4 拒绝路由与错误码（C-141-2）

审批无效时策略裁定 `PermissionVerdict::Deny`，`decision_source =
"plan_execution_approval_gate"`。`dispatcher.rs::deny_outcome` 按
decision_source 单独路由：该决策产出
`error.code == "plan_execution_approval_required"`；其余 Deny 仍为
`permission_denied`（既有语义不变）。最终消息包含 "approval" 字样，
说明缺失 / 不匹配 / 过期三类原因。

### 2.5 与 plan_draft_gate 的关系（C-141-7）

审批**只**解锁 Draft→Executing 迁移，不放宽其它任何门禁：

- Draft 态普通写/执行工具（Write / Edit / Run 等）仍被 `plan_draft_gate`
  以 `permission_denied`（message 含 `plan_draft_gate`）拒绝；
- 拒绝的 `start_execution` / `complete_step` 不产生状态副作用，plan 保持 Draft，
  写保护继续生效；
- 迁移成功（Executing）后普通写工具按既有 PA-138 语义放行。

## 3. 实现锚点

| 关注点 | 位置 |
|---|---|
| 证据类型与 serde 形状 | `crates/pony-agent-core/src/agent/dispatcher.rs`（`PlanExecutionApproval`、`DispatchContext.plan_execution_approval`） |
| TurnInput 透传 | `crates/pony-agent-core/src/agent/runtime/types.rs`；`runtime/mod.rs` 的 `set_context(...)` |
| 判定核心（触发面 / 绑定 / TTL） | `crates/pony-agent-core/src/agent/governed_executor.rs`（`GovernedPolicyEvaluator::decide_with_context`，`plan_execution_approval_gate`） |
| 错误码路由 | `crates/pony-agent-core/src/agent/dispatcher.rs`（`deny_outcome`：`plan_execution_approval_required`） |
| 宿主执行审批命令与 UI | `crates/pony-agent-core/src/agent/control_plane/ask_plan_commands.rs`（`plan_execution_approve`）、`src-tauri/src/lib.rs`（Tauri command `plan_execution_approve`）、`src/stores/plan.ts`（`approveExecution`）、`src/components/PlanPanel.vue`（「批准执行」按钮） |

## 4. 验收

冻结测试：`crates/pony-agent-core/tests/wave36_plan_execution_approval.rs`
（C-141-1..9 全矩阵：seam/serde、无审批 Deny、参数伪造无效、跨 session/跨 plan 绑定、
TTL 过期/有效、一次性效果、不削弱写门禁、complete_step 迁移面、正向对照）。
