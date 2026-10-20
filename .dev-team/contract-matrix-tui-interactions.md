# TUI 权限审批、AskUser、Plan、Task看板与Goal交互协议契约矩阵 (PA-120 / Wave 42)

## 1. 契约目标与范围
在全屏终端 TUI (`crates/pony-agent-core/src/bin/tui.rs`) 中引入企业级交互范式：
- 权限审批交互 (`PermissionRequest`, `PermissionChoice`, `PermissionState`)
- AskUser 结构化问答 (`AskUserQuestion`, `QuestionOption`, `AskUserState`)
- Plan 展示与模式交互 (`PlanState`, `PlanStep`, `PlanStepStatus`, `/plan`)
- Task 任务展示与折叠 (`TuiTask`, `TuiTaskStatus`, `TaskBoardState`, `/tasks`)
- Goal 目标交互与轮次展示 (`TuiGoal`, `GoalPhase`, `GoalActivation`)

## 2. 交互模式扩充 (`InteractionMode`)
- `InteractionMode::PermissionApproval`: 权限审批模态窗口
- `InteractionMode::AskUserQuestion`: 结构化问答交互模态窗口
- `InteractionMode::PlanMode`: Plan 步骤审查与模式
- `InteractionMode::TaskBoard`: 任务看板查看与交互

## 3. UI 事件拓展 (`UiEvent`)
- `UiEvent::RequestPermission(PermissionRequest)`
- `UiEvent::PermissionDecided { request_id, choice }`
- `UiEvent::AskUserPrompt { request_id, questions }`
- `UiEvent::AskUserAnswered { request_id, answers }`
- `UiEvent::PlanUpdated(PlanState)`
- `UiEvent::TaskBoardUpdated(Vec<TuiTask>)`
- `UiEvent::GoalUpdated(TuiGoal)`

## 4. 状态与空桩
- `TuiState` 拓展字段：
  - `permission_state: Option<PermissionState>`
  - `ask_user_state: Option<AskUserState>`
  - `plan_state: Option<PlanState>`
  - `task_board_state: Option<TaskBoardState>`
  - `goal_state: Option<TuiGoal>`
- 渲染空桩：
  - `render_permission_dialog_stub`
  - `render_ask_user_dialog_stub`
  - `render_plan_panel_stub`
  - `render_task_board_stub`
- 状态操作桩：
  - `start_permission_approval` / `cancel_permission_approval`
  - `start_ask_user` / `cancel_ask_user`
  - `toggle_plan_mode`
  - `toggle_task_board`
  - `update_plan`
  - `update_task_board`
  - `set_goal`

## 5. 验收门禁与交付
- Rust 编译器检查通过 (`cargo check --bin pony`, `cargo check --tests`) Exit Code: 0
- 结构清晰，正交解耦，不破坏现有 TUI 渲染与事件调度流程。
