//! 轮次/图运行生命周期与 graph Ask 绑定命令。

use crate::agent;
use crate::agent::control_plane::{
    ContinueGraphRunCommand, ContinueGraphRunStreamCommand, FailTurnForWatchdogCommand,
    GraphRunControlResponse, GraphRunStreamStartResponse, GraphRunTurnResponse, HostControlPlane,
    ResumeGraphRunCommand, ResumeGraphRunStreamCommand, RunTurnCommand, StartGraphRunCommand,
    StartGraphRunStreamCommand, StopGraphRunCommand, StopTurnCommand,
};
use crate::agent::execution_control::StopTurnResponse;
use crate::agent::graph::GraphAskWaitBinding;
use crate::agent::runtime::TurnInput;
use crate::agent::runtime::TurnResult;
use crate::goal_resume;
use crate::tauri_adapter;
use crate::turn_task_registry::TurnTaskRegistry;
use serde_json::Value;
use tauri::AppHandle;
use tauri::State;

#[tauri::command]
pub(crate) fn run_turn(control_plane: State<'_, HostControlPlane>, input: TurnInput) -> TurnResult {
    control_plane.run_turn(RunTurnCommand { input })
}

#[tauri::command]
pub(crate) fn start_graph_run(
    control_plane: State<'_, HostControlPlane>,
    run_id: Option<String>,
    goal: String,
    input: TurnInput,
) -> Result<GraphRunTurnResponse, String> {
    control_plane.start_graph_run(StartGraphRunCommand {
        run_id,
        goal,
        input,
    })
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri 命令入参由 IPC 反序列化注入（AppHandle/State 必须按值接收），函数内只借用"
)]
#[tauri::command]
pub(crate) fn start_graph_run_stream(
    app: AppHandle,
    control_plane: State<'_, HostControlPlane>,
    authorization_modes: State<'_, goal_resume::SessionAuthorizationModes>,
    turn_id: String,
    run_id: Option<String>,
    goal: String,
    input: TurnInput,
) -> Result<GraphRunStreamStartResponse, String> {
    authorization_modes.record(
        input.session_id.as_deref(),
        input.tool_authorization_mode.as_deref(),
    );
    let (response, prepared) =
        control_plane.prepare_start_graph_run_stream(StartGraphRunStreamCommand {
            turn_id,
            run_id,
            goal,
            input,
        })?;
    // wave-60 L3-A BLOCKING-4：prepare（已 register_turn ⇒ turn 可被 stop）之后、spawn 之前
    // 同步注册本 turn 的事件出口，消除"可被 stop 但无 sink"的窗口。
    tauri_adapter::register_turn_sink(&app, &prepared.turn_id);
    tauri_adapter::spawn_graph_run_stream(&app, prepared)?;
    Ok(response)
}

#[tauri::command]
pub(crate) fn continue_graph_run(
    control_plane: State<'_, HostControlPlane>,
    run_id: String,
    input: TurnInput,
) -> Result<GraphRunTurnResponse, String> {
    control_plane.continue_graph_run(ContinueGraphRunCommand { run_id, input })
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri 命令入参由 IPC 反序列化注入（AppHandle/State 必须按值接收），函数内只借用"
)]
#[tauri::command]
pub(crate) fn continue_graph_run_stream(
    app: AppHandle,
    control_plane: State<'_, HostControlPlane>,
    authorization_modes: State<'_, goal_resume::SessionAuthorizationModes>,
    turn_id: String,
    run_id: String,
    input: TurnInput,
) -> Result<GraphRunStreamStartResponse, String> {
    authorization_modes.record(
        input.session_id.as_deref(),
        input.tool_authorization_mode.as_deref(),
    );
    let (response, prepared) =
        control_plane.prepare_continue_graph_run_stream(ContinueGraphRunStreamCommand {
            turn_id,
            run_id,
            input,
        })?;
    // wave-60 L3-A BLOCKING-4：同上（prepare → 注册 sink → spawn）。
    tauri_adapter::register_turn_sink(&app, &prepared.turn_id);
    tauri_adapter::spawn_graph_run_stream(&app, prepared)?;
    Ok(response)
}

#[tauri::command]
pub(crate) fn resume_graph_run(
    control_plane: State<'_, HostControlPlane>,
    run_id: String,
    input: TurnInput,
) -> Result<GraphRunTurnResponse, String> {
    control_plane.resume_graph_run(ResumeGraphRunCommand { run_id, input })
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri 命令入参由 IPC 反序列化注入（AppHandle/State 必须按值接收），函数内只借用"
)]
#[tauri::command]
pub(crate) fn resume_graph_run_stream(
    app: AppHandle,
    control_plane: State<'_, HostControlPlane>,
    authorization_modes: State<'_, goal_resume::SessionAuthorizationModes>,
    turn_id: String,
    run_id: String,
    input: TurnInput,
) -> Result<GraphRunStreamStartResponse, String> {
    authorization_modes.record(
        input.session_id.as_deref(),
        input.tool_authorization_mode.as_deref(),
    );
    let (response, prepared) =
        control_plane.prepare_resume_graph_run_stream(ResumeGraphRunStreamCommand {
            turn_id,
            run_id,
            input,
        })?;
    // wave-60 L3-A BLOCKING-4：同上（prepare → 注册 sink → spawn）。
    tauri_adapter::register_turn_sink(&app, &prepared.turn_id);
    tauri_adapter::spawn_graph_run_stream(&app, prepared)?;
    Ok(response)
}

/// wave-60 C6：停止一轮（副作用顺序**固定**，顺序反转视为 FAIL）。
///
/// ① `request_stop`（经 `HostControlPlane::stop_turn`：置位唯一取消源 + 同步派发恰好一条
///    `turn:stop_requested` + 启动宽限期监督）→ ② 取消令牌（与①**同一取消源**：checkpoint
///    的停止位即 provider 读通道与监督者观测的取消位，不新建第二套取消状态）→
///    ③ `abort_turn` 该轮句柄（**仅 L2 兜底**：`spawn_blocking` 不可抢占，abort 只是表项回收，
///    绝不据此向用户呈现"已停止"）。
#[tauri::command]
pub(crate) fn stop_turn(
    control_plane: State<'_, HostControlPlane>,
    task_registry: State<'_, TurnTaskRegistry>,
    turn_id: String,
    stop_reason: Option<String>,
) -> StopTurnResponse {
    // ① 请求停止（C1 应答语义 + C2 立即应答事件 + C5 宽限监督）。
    let response = control_plane.stop_turn(StopTurnCommand {
        turn_id: turn_id.clone(),
        stop_reason: stop_reason
            .as_deref()
            .and_then(|wire| wire.parse::<agent::stop_escalation::StopReason>().ok()),
    });

    // ② 取消令牌：已由①置位（唯一取消源）。未知 wire 原因/未接受时也不重复置位——
    //    request_stop 是唯一置位点，避免"两套取消状态"造成观测分叉。

    // ③ L2 兜底：只回收该 turn 的句柄（绝不再按 session 连带 abort 同会话其它轮次）。
    task_registry.abort_turn(&turn_id);

    response
}

#[tauri::command]
pub(crate) fn stop_graph_run(
    control_plane: State<'_, HostControlPlane>,
    run_id: String,
) -> Result<GraphRunControlResponse, String> {
    control_plane.stop_graph_run(StopGraphRunCommand { run_id })
}

/// 契约 watchdog-fix backend：前端 watchdog 触发时调用，后端立即幂等落盘 failed trace
/// + checkpoint failed + request_stop（可选 stop_graph_run），返回 StopTurnResponse 等价结构。
#[tauri::command]
pub(crate) fn fail_turn_for_watchdog(
    control_plane: State<'_, HostControlPlane>,
    turn_id: String,
    session_id: Option<String>,
    run_id: Option<String>,
    error: Option<String>,
    reason: Option<String>,
) -> StopTurnResponse {
    control_plane.fail_turn_for_watchdog(FailTurnForWatchdogCommand {
        turn_id,
        session_id,
        run_id,
        error: error.unwrap_or_else(|| "submission_watchdog_timeout".to_string()),
        reason,
    })
}

/// Snapshot of every Ask wait bound to a graph run.
#[tauri::command]
pub(crate) fn graph_list_ask_waits(
    control_plane: State<'_, HostControlPlane>,
    run_id: String,
) -> Value {
    control_plane.graph_list_ask_waits(&run_id)
}

/// Bind an Ask `PendingControlRequest` to a run's `wait_user` suspension.
#[tauri::command]
pub(crate) fn graph_bind_ask_wait(
    control_plane: State<'_, HostControlPlane>,
    run_id: String,
    binding: GraphAskWaitBinding,
) -> Result<Value, String> {
    control_plane.graph_bind_ask_wait(&run_id, binding)
}

/// Resolve a bound Ask wait, injecting exactly one terminal tool result for the original call id.
/// A stale `version` is rejected.
#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri 命令入参由 IPC 反序列化注入（AppHandle/State 必须按值接收），函数内只借用"
)]
#[tauri::command]
pub(crate) fn graph_resume_ask(
    app: AppHandle,
    control_plane: State<'_, HostControlPlane>,
    authorization_modes: State<'_, goal_resume::SessionAuthorizationModes>,
    run_id: String,
    request_id: String,
    version: u64,
    answer: Value,
) -> Result<Value, String> {
    let resumed = control_plane.graph_resume_ask(&run_id, &request_id, version, answer)?;
    // O1-B（2026-11-27）：答案落地后把 goal 自动续轮重新接上——旧实现到此为止，run 停在
    // `Ready` 且没有任何线程再发下一轮，目标永久 `active + Armed`（审核报告 P0-1）。
    // 续跑失败不得回滚答案（答案已持久化），只做可见留痕。
    if let Err(error) = goal_resume::resume_goal_rounds_after_host_answer(
        &app,
        &control_plane,
        &run_id,
        &authorization_modes,
    ) {
        eprintln!("[pony-agent] goal continuation after ask resume failed: {error}");
    }
    Ok(resumed)
}
