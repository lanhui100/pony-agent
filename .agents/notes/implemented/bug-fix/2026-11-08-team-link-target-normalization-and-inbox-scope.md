# Agent Note: 团队链路 D1/D2/D3 修复（target 归一化 / drain 作用域 / 软删除过滤）

Status: implemented

## Problem

2026-11-08 团队链路诊断（`docs/analysis/team-link-diagnosis-20261108.md`，trace + live 双证据）确认三处缺陷：

- **D1**：`register_teammate` 登记时把 name 归一化为 target（小写 + 空格转连字符），
  但 `enqueue_message` / `interrupt_agent` / `drain_inbox` 精确匹配 map key；
  `list_agents` 同时暴露 `name="Lead"` 与 `target="lead"`，模型拿 name 当 target
  即 `Teammate target not found`（trace：verifier→"Lead" 失败；live：→"Probe" 失败）。
- **D2**：模型侧 `drain_inbox` 可消费**任意成员**信箱。Lead 误 drain 队友信箱会把
  派发消息抢走（trace：Lead drain dev-tui 后被迫重发）；队友信箱本应由驱动循环消费。
- **D3**（排查 /tasks 时新发现）：`team_task_list` 不过滤软删除任务，TUI 投影对未知
  状态一律降级 Pending ⇒ 已删除任务在 /tasks 伪装成待办（live 实证：两条 deleted
  探针任务仍列在板上）。

## Decision

- **D1**：`orchestration::normalize_target`（trim + 小写 + 空格转连字符）成为登记与
  查找的**唯一归一化规则**；`register_teammate` / `enqueue_message` / `interrupt_agent` /
  `drain_inbox` / `teammate_session_id` 入口统一调用。错误语义不放宽：未知 target 仍报错。
- **D2**：工具层 `execute_drain_inbox` 增加调用者作用域守卫——`context.session_id` 经
  `teammate_target_for_session` 反查调用者（宿主会话/无上下文 ⇒ `lead`），
  `target != caller` 返回 `inbox_scope_violation` 且**不消费**任何消息。
  库层 `drain_inbox` 语义不变（驱动循环 `drive_teammate_turn_in` 仍消费队友信箱）。
  工具描述同步改为「调用者自己的信箱」；input_schema 逐字节不变（surface golden 哈希不受影响）。
- **D3**：`team_task_list` 过滤 `status == "deleted"`——模型看板与 TUI /tasks 共用该
  真值源，一处过滤两处生效；`delete` 动作的回执（返回 deleted 项）不变。

## Alternatives considered

- **D1 只改错误提示（列出可用 target）**：模型仍需二次调用才能命中，trace 里的失败
  路径原样保留；归一化让 name 写法直接可用，成本一个纯函数。
- **D2 仅提示词约束（诊断报告原建议）**：靠模型自觉，事故已实证会复发；机器可查的
  承诺必须配非零退出门禁（常载命约 #2），故落为工具层 fail-closed 守卫。
- **D2 给 ToolRouter/引擎链新增 agent_target 字段透传身份**：最"正"，但要改
  `SubagentTurnEngine` trait 签名与 builder→executor→router 全链契约，改动面远超缺陷
  本身；`context.session_id` + 既有 `TEAMMATE_SESSIONS` 反查零新增契约即可判定身份。
  已知边界：队友**首个** turn 期间会话尚未登记，此时其 drain 自身会被判为 lead 调用
  而拒绝——首 turn 消息本就注入 prompt，模型无需 drain，边界可接受。
- **D2 把 drain_inbox 改为非破坏性 peek**：lead 回执将无人消费、重复读取，破坏既有
  驱动回执语义。
- **D3 在 TUI 投影侧过滤 deleted**：只修 UI，模型侧 `team_task_list` 仍泄漏；真值源
  过滤才符合「每个事实一个家」。

## Verification

- `cargo test -p pony-agent-core --test team_link_fixes_test --test team_drain_scope_test`
  （D1 归一化 / D3 过滤 / D2 作用域 + 拒绝不消费 + lead 回执保留）。
- 既有冻结面回归：`tool_surface_singularity`（schema 哈希）、`orchestration_acceptance_test`、
  `pa110_teams_acceptance`、`teammate_e2e_communication_test`、`wave61_team_activation`。
- 实测（2026-11-08）：上述 8 个测试二进制 **58 passed / 0 failed**（exit 0）；
  可观测性门禁 `audit-diff.sh` OK、`check-declaration.sh --declaration docs/observability.yaml` PASS
  （本次无新增外部 IO，声明无需变更）。
- 复验（2026-11-08 第三轮）：扩至 10 个测试二进制（新增 `w3_team_visibility_test` /
  `team_task_order_contract` / `team_history_acceptance`）**152 passed / 0 failed**（exit 0）；
  `target/debug/pony`（16:53:54）含 `normalize_target` / `inbox_scope_violation` 符号。
  注意：存活 TUI 进程（11:41/11:56 启动）仍是旧二进制，live `team_task_list` 仍泄漏 deleted
  任务 ⇒ 面板正确状态需重启新二进制后生效（详见诊断报告 §6）。