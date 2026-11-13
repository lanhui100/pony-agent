# Agent Note: goal 领域状态机守卫与模型工具权限门（B127 & B128）

Status: implemented

## Problem

在实测与 DSH 架构对齐比对中（`docs/analysis/dsh-vs-pony-goal-2026-11-12.md`），Pony 的 goal（会话目标/长期目标）子系统暴露出两处严重的不设防缺陷：

1. **B127：领域层缺状态迁移守卫与「已存在」守卫**：
   - `crates/pony-agent-core/src/agent/goal.rs::create_goal` 收到请求后直接 `goals.insert`，不做任何前置检查。当已有目标处于 `active`/`paused`/`blocked` 等进行中状态时，第二次调用会静默覆盖现有目标及其版本号与轮次预算。
   - `goal.rs::update_goal` 仅做 id 与 revision 校验，对 `phase` 字段执行无条件覆盖：`UpdateGoalAction::Resume` 无条件将 `phase` 改为 `"active"`，使已经标记为终态的 `completed` 目标被非法复活，破坏了终态不可逆契约。
   - 斜杠命令层（`goal_command.rs`）虽然做了部分上层防御，但领域核心层未设防，导致单一真值断裂。

2. **B128：模型侧 goal 工具无权限门**：
   - `crates/pony-agent-core/src/agent/tools/orchestration.rs::execute_update_goal` 和 `execute_create_goal` 只做反序列化与会话填充，未检查 `ToolExecutionContext.dispatch_context` 传入的调用权限。
   - 模型在普通非 goal 对话轮次中，或子代理（subagent）在后台执行时，可以任意调用 `update_goal` 把目标 `pause`、`resume`、`complete` 或 `blocked`；尤其是人类手动暂停的目标，模型在后续轮次可以擅自 resume，破坏了人类控制权。

## Decision

对齐 DSH 权限与领域层状态机设计，在领域层确立单一真值守卫，并在工具执行层接入权限门：

1. **领域层单一真值迁移守卫**：
   - 在 `crates/pony-agent-core/src/agent/goal.rs::create_goal` 增加前置守卫：若会话中已存在目标且其 `phase != "completed"`，直接拒绝覆盖并返回显式错误；仅当目标已完成或不存在时允许新建。
   - 在 `goal.rs::update_goal` 引入合法的状态迁移检查：
     - `Pause`：仅允许来自 `active`；
     - `Resume`：仅允许来自 `paused` 或 `blocked`，且严禁复活 `completed` 目标，同时校验 `rounds_started < max_goal_rounds`（预算耗尽拒绝 resume）；
     - `Complete`：允许来自 `active`、`paused`、`blocked`，若已 `completed` 则拒绝；
     - `Blocked`：允许来自 `active`、`paused`，要求必填 `blocked_reason`，严禁 block 已完成目标；
     - `Edit`：仅允许编辑非 `completed` 目标。
   - `goal_command.rs` 复用领域层的统一错误，保持语义一致。

2. **模型侧工具权限门（Authority Gate）**：
   - 在 `DispatchContext` 中引入来源事实标志：`is_goal_round: bool`（由 `<goal_round ...>` 提示词协议识别）与 `is_direct_human: bool`（人类直接发起的顶层轮次）。
   - 在 `ChildDispatch`（子代理）中显式将 `is_direct_human` 降级为 `false`。
   - 在 `orchestration.rs::execute_create_goal`：仅允许 `is_direct_human` 轮次调用，非人类发起时返回结构化错误码 `goal_action_unauthorized`。
   - 在 `orchestration.rs::execute_update_goal`：
     - `pause` / `resume` / `edit`：严格要求 `is_direct_human`（模型不可自行 resume 人类暂停的目标）；
     - `complete` / `blocked`：要求 `is_direct_human || is_goal_round`（仅人类发起轮或 goal-round 自主续轮可收官/阻断）。

## Alternatives considered

- **仅在 `/goal` 命令层和 schema 提示词层进行软约束**：
  模型容易因幻觉或指令漂移在非预期轮次调用 `update_goal`，且任何直接调用领域层 API 的客户端或上层代码都会绕过软约束。必须下沉到领域层与工具路由器双重硬防护。
- **引入独立的 GoalAuthority 枚举贯穿整个 RPC**：
  过度设计，当前已有成熟的 `ToolExecutionContext.dispatch_context`（携带 `session_id`、`run_id`、`turn_id` 等），只需扩展布尔/来源属性即可零成本获得精确权限判定。

## Consequences

- 破坏非法调用的静默通过行为：任何越权调用 `update_goal` 或 `create_goal` 将收到明确的 `goal_action_unauthorized` 或非法状态迁移错误。
- 保证已完成目标（`completed`）的终态不可逆性，消除意外复活导致的无界续轮与 Token 浪费。
- 增加覆盖状态机合法迁移矩阵、已存在拒绝、越权调用拦截的单元测试与集成断言。
