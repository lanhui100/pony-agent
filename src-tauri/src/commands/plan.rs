//! Plan 模式控制面命令（会话级协作模式，非任务清单）。

use crate::agent::control_plane::HostControlPlane;
use serde_json::Value;
use tauri::State;

/// 读取会话的 Plan 模式状态（`active` / `plan` / `planApproved` / `revision`）。
#[tauri::command]
pub(crate) fn plan_mode_get(
    control_plane: State<'_, HostControlPlane>,
    session_id: String,
) -> Value {
    control_plane.plan_mode_get(&session_id)
}

/// 进入（`active = true`）/ 退出（`active = false`）Plan 模式。
///
/// 进入走 `pending`：下一轮 pre-step 才生效（DSH「用户选择待提交」语义）；退出立即生效。
#[tauri::command]
#[expect(
    clippy::fn_params_excessive_bools,
    reason = "IPC 入参签名由前端 `safeInvoke(\"plan_mode_set\", { sessionId, active })` 冻结；内部立刻转成 ModeToggle"
)]
pub(crate) fn plan_mode_set(
    control_plane: State<'_, HostControlPlane>,
    session_id: String,
    active: bool,
) -> Result<Value, String> {
    let toggle = if active {
        pony_agent_core::agent::plan_mode::ModeToggle::Enter
    } else {
        pony_agent_core::agent::plan_mode::ModeToggle::Exit
    };
    control_plane.plan_mode_set(&session_id, toggle)
}

/// 把上一轮的切换意图落地（宿主在每次 pre-step 前调用；幂等）。
#[tauri::command]
pub(crate) fn plan_mode_commit(
    control_plane: State<'_, HostControlPlane>,
    session_id: String,
) -> Value {
    control_plane.plan_mode_commit(&session_id)
}

/// 用户在方案审核卡片上的决定：`approved = true` 退出模式并解锁写/执行；
/// `false` 保持模式激活（「继续规划」）。
#[tauri::command]
#[expect(
    clippy::fn_params_excessive_bools,
    reason = "IPC 入参签名由前端 `safeInvoke(\"plan_mode_review\", { sessionId, approved })` 冻结；内部立刻转成 PlanReviewDecision"
)]
pub(crate) fn plan_mode_review(
    control_plane: State<'_, HostControlPlane>,
    session_id: String,
    approved: bool,
) -> Result<Value, String> {
    let decision = if approved {
        pony_agent_core::agent::plan_mode::PlanReviewDecision::Approve
    } else {
        pony_agent_core::agent::plan_mode::PlanReviewDecision::KeepPlanning
    };
    control_plane.plan_mode_review(&session_id, decision)
}
