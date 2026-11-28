//! 会话运行时视图、历史图与模型监控读取命令。

use crate::agent;
use crate::agent::context::RetrievedContextState;
use crate::agent::control_plane::{
    CheckoutHistoryNodeCommand, DeleteSessionCommand, ForkFromHistoryNodeCommand,
    ForkFromHistoryNodeResponse, GraphRunSubmissionPlan, GraphRunSubmissionPlanQuery,
    HistoryCheckoutMode, HistoryCheckoutResponse, HistoryCursorQuery, HistoryCursorState,
    HistoryGraphQuery, HistoryGraphView, HostControlPlane, ModelMonitorSessionDrilldownQuery,
    ModelMonitorSessionDrilldownView, ModelMonitorSummaryQuery, ModelMonitorSummaryView,
    RestoreBranchHeadCommand, RestoreBranchHeadResponse, RetrievedContextQuery, SessionRuntimeView,
    SessionRuntimeViewQuery, SwitchHistoryBranchCommand, SwitchHistoryBranchResponse,
};
use crate::agent::runtime::ManualCompactionOutcome;
use crate::agent::session::SessionOverview;
use crate::blocking_helper::BlockingHelper;
use tauri::AppHandle;
use tauri::Manager;
use tauri::State;

/// PA-094：大字段外置——按引用加载 build_context_observation 全量 payload。
/// 引用格式 `bco:<turn_id>:<seq>`；未命中返回 null（legacy 内嵌数据走 trace 字段）。
#[tauri::command]
pub(crate) fn load_build_context_observation(
    control_plane: State<'_, HostControlPlane>,
    session_id: String,
    observation_ref: String,
) -> Option<agent::provider::BuildContextObservation> {
    control_plane.load_build_context_observation(&session_id, &observation_ref)
}

/// 会话运行时视图加载（热路径：会话切换/历史导航，底层同步 SQLite + 快照组装）。
///
/// W56-D：原同步命令在主线程内联执行 ⇒ 切换会话时 UI 卡死同类问题（ADR-0032）。
/// 现 async + `BlockingHelper::spawn`；`State` 无法跨线程，故在阻塞闭包内用
/// `AppHandle::state::<HostControlPlane>()` 现取（不 unsafe、不克隆整状态）；
/// 参数名（turnId/sessionId/nodeId/runId）与成功返回形状不变。
#[tauri::command]
pub(crate) async fn load_session_runtime_view(
    app: AppHandle,
    turn_id: Option<String>,
    session_id: Option<String>,
    node_id: Option<String>,
    run_id: Option<String>,
) -> Result<SessionRuntimeView, String> {
    BlockingHelper::spawn(move || {
        app.state::<HostControlPlane>()
            .load_session_runtime_view(SessionRuntimeViewQuery {
                turn_id,
                session_id,
                node_id,
                run_id,
            })
    })
    .await
}

#[tauri::command]
pub(crate) fn resolve_graph_run_submission_plan(
    control_plane: State<'_, HostControlPlane>,
    session_id: Option<String>,
    node_id: Option<String>,
    run_id: Option<String>,
) -> GraphRunSubmissionPlan {
    control_plane.resolve_graph_run_submission_plan(GraphRunSubmissionPlanQuery {
        session_id,
        node_id,
        run_id,
    })
}

#[tauri::command]
pub(crate) fn load_retrieved_context(
    control_plane: State<'_, HostControlPlane>,
    turn_id: Option<String>,
    session_id: Option<String>,
    node_id: Option<String>,
    run_id: Option<String>,
) -> RetrievedContextState {
    control_plane.load_retrieved_context(RetrievedContextQuery {
        turn_id,
        session_id,
        node_id,
        run_id,
    })
}

#[tauri::command]
pub(crate) fn load_history_graph(
    control_plane: State<'_, HostControlPlane>,
    session_id: Option<String>,
) -> HistoryGraphView {
    control_plane.load_history_graph(HistoryGraphQuery { session_id })
}

#[tauri::command]
pub(crate) fn load_model_monitor_summary(
    control_plane: State<'_, HostControlPlane>,
    session_id: Option<String>,
) -> ModelMonitorSummaryView {
    control_plane.load_model_monitor_summary(ModelMonitorSummaryQuery { session_id })
}

#[tauri::command]
pub(crate) fn load_model_monitor_session_drilldown(
    control_plane: State<'_, HostControlPlane>,
    session_id: String,
) -> ModelMonitorSessionDrilldownView {
    control_plane
        .load_model_monitor_session_drilldown(ModelMonitorSessionDrilldownQuery { session_id })
}

#[tauri::command]
pub(crate) fn load_history_cursor(
    control_plane: State<'_, HostControlPlane>,
    session_id: Option<String>,
) -> HistoryCursorState {
    control_plane.load_history_cursor(HistoryCursorQuery { session_id })
}

#[tauri::command]
pub(crate) fn checkout_history_node(
    control_plane: State<'_, HostControlPlane>,
    session_id: Option<String>,
    node_id: String,
    mode: HistoryCheckoutMode,
    expected_cursor_version: Option<u64>,
) -> Result<HistoryCheckoutResponse, String> {
    control_plane.checkout_history_node(CheckoutHistoryNodeCommand {
        session_id,
        node_id,
        mode,
        expected_cursor_version,
    })
}

#[tauri::command]
pub(crate) fn restore_branch_head(
    control_plane: State<'_, HostControlPlane>,
    session_id: Option<String>,
    branch_id: Option<String>,
    expected_cursor_version: Option<u64>,
) -> Result<RestoreBranchHeadResponse, String> {
    control_plane.restore_branch_head(RestoreBranchHeadCommand {
        session_id,
        branch_id,
        expected_cursor_version,
    })
}

#[tauri::command]
pub(crate) fn fork_from_history_node(
    control_plane: State<'_, HostControlPlane>,
    session_id: Option<String>,
    node_id: String,
    expected_cursor_version: Option<u64>,
) -> Result<ForkFromHistoryNodeResponse, String> {
    control_plane.fork_from_history_node(ForkFromHistoryNodeCommand {
        session_id,
        node_id,
        expected_cursor_version,
    })
}

#[tauri::command]
pub(crate) fn switch_history_branch(
    control_plane: State<'_, HostControlPlane>,
    session_id: Option<String>,
    branch_id: String,
    expected_cursor_version: Option<u64>,
) -> Result<SwitchHistoryBranchResponse, String> {
    control_plane.switch_history_branch(SwitchHistoryBranchCommand {
        session_id,
        branch_id,
        expected_cursor_version,
    })
}

#[tauri::command]
pub(crate) fn delete_session(
    control_plane: State<'_, HostControlPlane>,
    session_id: String,
) -> Vec<SessionOverview> {
    control_plane.delete_session(DeleteSessionCommand { session_id })
}

/// 手动上下文压缩（桌面 `/compact`，契约 C-3）。
///
/// 命令名 `compact_session`，参数 `sessionId` / `force`（缺省按 `true` 处理，用户主动触发即强制），
/// 返回 camelCase 的 `ManualCompactionOutcome`（由 core 的 `serde(rename_all = "camelCase")` 保证）。
///
/// 该命令内部会发起真实 LLM 摘要调用（网络 IO，秒级），按 W56-D 惯例不能同步内联在
/// WebView IPC 线程上；`State<'_, _>` 无法跨线程，故 async + `BlockingHelper::spawn`，
/// 在阻塞闭包内用 `AppHandle::state::<HostControlPlane>()` 现取。
#[tauri::command]
pub(crate) async fn compact_session(
    app: AppHandle,
    session_id: Option<String>,
    force: Option<bool>,
) -> Result<ManualCompactionOutcome, String> {
    let force = force.unwrap_or(true);
    BlockingHelper::spawn(move || {
        app.state::<HostControlPlane>()
            .compact_session(session_id.as_deref(), force)
    })
    .await
    .and_then(|outcome| outcome)
}
