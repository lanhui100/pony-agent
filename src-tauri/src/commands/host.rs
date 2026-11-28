//! 宿主健康与检查点读取命令（`HostControlPlane` 只读面）。

use crate::agent::control_plane::{
    ExecutionCheckpointQuery, GraphRunCheckpointQuery, HostControlPlane, HostHealthSnapshot,
    HostInspectionQuery, HostInspectionSnapshot,
};
use crate::agent::execution_control::ExecutionCheckpoint;
use crate::agent::graph::GraphRunCheckpoint;
use crate::blocking_helper::BlockingHelper;
use tauri::AppHandle;
use tauri::Manager;
use tauri::State;

/// 健康快照读（热路径：启动即调）。
///
/// W56-D：同步命令被 Tauri 内联执行在 WebView IPC 线程上（tauri-2.11.5
/// ipc/protocol.rs:314 → webview/mod.rs:1742 on_message → manager/mod.rs:471
/// run_invoke_handler，全程无 spawn），故改 async + `BlockingHelper::spawn`。
/// 命令名/参数/成功返回形状不变；`State<'_, _>` 无法跨线程，改为在阻塞闭包内用
/// `AppHandle::state::<HostControlPlane>()` 现取（不 unsafe、不克隆整状态）。
#[tauri::command]
pub(crate) async fn health_check(app: AppHandle) -> Result<HostHealthSnapshot, String> {
    BlockingHelper::spawn(move || app.state::<HostControlPlane>().health_snapshot()).await
}

#[tauri::command]
pub(crate) fn inspect_host(
    control_plane: State<'_, HostControlPlane>,
    turn_id: Option<String>,
    session_id: Option<String>,
    run_id: Option<String>,
    include_session: Option<bool>,
    include_retrieved: Option<bool>,
    include_sessions: Option<bool>,
    include_run: Option<bool>,
    include_runs: Option<bool>,
) -> HostInspectionSnapshot {
    control_plane.inspect(HostInspectionQuery {
        turn_id,
        session_id,
        run_id,
        include_session: include_session.unwrap_or(true),
        include_retrieved: include_retrieved.unwrap_or(false),
        include_sessions: include_sessions.unwrap_or(false),
        include_run: include_run.unwrap_or(false),
        include_runs: include_runs.unwrap_or(false),
    })
}

#[tauri::command]
pub(crate) fn load_execution_checkpoint(
    control_plane: State<'_, HostControlPlane>,
    turn_id: Option<String>,
    session_id: Option<String>,
) -> Option<ExecutionCheckpoint> {
    control_plane.load_execution_checkpoint(ExecutionCheckpointQuery {
        turn_id,
        session_id,
    })
}

#[tauri::command]
pub(crate) fn load_graph_run_checkpoint(
    control_plane: State<'_, HostControlPlane>,
    run_id: String,
) -> Option<GraphRunCheckpoint> {
    control_plane.load_graph_run_checkpoint(GraphRunCheckpointQuery {
        run_id: Some(run_id),
    })
}
