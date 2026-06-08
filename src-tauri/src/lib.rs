mod agent;
pub mod sse_adapter;
mod tauri_adapter;

use agent::capability_bridge::{CapabilitySourceView, CapabilityView, SkillDescriptor};
use agent::config::{ProviderRegistryStore, ProviderRegistryView};
use agent::context::RetrievedContextState;
use agent::control_plane::{
    CapabilityInspectionQuery, CapabilityListQuery, CapabilitySourceInspectionQuery,
    CheckoutHistoryNodeCommand, ContinueGraphRunCommand, ContinueGraphRunStreamCommand,
    DeleteSessionCommand, ExecutionCheckpointQuery, ForkFromHistoryNodeCommand,
    ForkFromHistoryNodeResponse, GraphRunCheckpointQuery, GraphRunControlResponse,
    GraphRunStreamStartResponse, GraphRunSubmissionPlan, GraphRunSubmissionPlanQuery,
    GraphRunTurnResponse, HistoryCheckoutMode, HistoryCheckoutResponse, HistoryCursorQuery,
    HistoryCursorState, HistoryGraphQuery, HistoryGraphView, HostControlPlane, HostHealthSnapshot,
    HostInspectionQuery, HostInspectionSnapshot, ModelMonitorSessionDrilldownQuery,
    ModelMonitorSessionDrilldownView, ModelMonitorSummaryQuery, ModelMonitorSummaryView,
    RestoreBranchHeadCommand, RestoreBranchHeadResponse, ResumeGraphRunCommand,
    ResumeGraphRunStreamCommand, RetrievedContextQuery, RunTurnCommand, SessionRuntimeView,
    SessionRuntimeViewQuery, SkillInspectionQuery, SkillListQuery, StartGraphRunCommand,
    StartGraphRunStreamCommand, StartTurnStreamCommand, StopGraphRunCommand, StopTurnCommand,
    SwitchHistoryBranchCommand, SwitchHistoryBranchResponse,
};
use agent::execution_control::{ExecutionCheckpoint, StopTurnResponse};
use agent::graph::GraphRunCheckpoint;
use agent::runtime::{TurnInput, TurnResult};
use agent::session::SessionOverview;
use agent::tools::ToolDefinition;
use serde_json::{json, Value};
use std::sync::Mutex;
use tauri::{AppHandle, Manager, State};

#[derive(Default)]
struct StreamDebugMetricsState {
    latest: Mutex<Value>,
}

#[tauri::command]
fn health_check(control_plane: State<'_, HostControlPlane>) -> HostHealthSnapshot {
    control_plane.health_snapshot()
}

#[tauri::command]
fn run_turn(control_plane: State<'_, HostControlPlane>, input: TurnInput) -> TurnResult {
    control_plane.run_turn(RunTurnCommand { input })
}

#[tauri::command]
fn start_graph_run(
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

#[tauri::command]
fn start_graph_run_stream(
    app: AppHandle,
    control_plane: State<'_, HostControlPlane>,
    turn_id: String,
    run_id: Option<String>,
    goal: String,
    input: TurnInput,
) -> Result<GraphRunStreamStartResponse, String> {
    let (response, prepared) =
        control_plane.prepare_start_graph_run_stream(StartGraphRunStreamCommand {
            turn_id,
            run_id,
            goal,
            input,
        })?;
    tauri_adapter::spawn_graph_run_stream(app, prepared);
    Ok(response)
}

#[tauri::command]
fn continue_graph_run(
    control_plane: State<'_, HostControlPlane>,
    run_id: String,
    input: TurnInput,
) -> Result<GraphRunTurnResponse, String> {
    control_plane.continue_graph_run(ContinueGraphRunCommand { run_id, input })
}

#[tauri::command]
fn continue_graph_run_stream(
    app: AppHandle,
    control_plane: State<'_, HostControlPlane>,
    turn_id: String,
    run_id: String,
    input: TurnInput,
) -> Result<GraphRunStreamStartResponse, String> {
    let (response, prepared) =
        control_plane.prepare_continue_graph_run_stream(ContinueGraphRunStreamCommand {
            turn_id,
            run_id,
            input,
        })?;
    tauri_adapter::spawn_graph_run_stream(app, prepared);
    Ok(response)
}

#[tauri::command]
fn resume_graph_run(
    control_plane: State<'_, HostControlPlane>,
    run_id: String,
    input: TurnInput,
) -> Result<GraphRunTurnResponse, String> {
    control_plane.resume_graph_run(ResumeGraphRunCommand { run_id, input })
}

#[tauri::command]
fn resume_graph_run_stream(
    app: AppHandle,
    control_plane: State<'_, HostControlPlane>,
    turn_id: String,
    run_id: String,
    input: TurnInput,
) -> Result<GraphRunStreamStartResponse, String> {
    let (response, prepared) =
        control_plane.prepare_resume_graph_run_stream(ResumeGraphRunStreamCommand {
            turn_id,
            run_id,
            input,
        })?;
    tauri_adapter::spawn_graph_run_stream(app, prepared);
    Ok(response)
}

#[tauri::command]
fn start_turn_stream(app: AppHandle, turn_id: String, input: TurnInput) -> Result<(), String> {
    tauri_adapter::spawn_turn_stream(app, StartTurnStreamCommand { turn_id, input });
    Ok(())
}

#[tauri::command]
fn stop_turn(control_plane: State<'_, HostControlPlane>, turn_id: String) -> StopTurnResponse {
    control_plane.stop_turn(StopTurnCommand { turn_id })
}

#[tauri::command]
fn stop_graph_run(
    control_plane: State<'_, HostControlPlane>,
    run_id: String,
) -> Result<GraphRunControlResponse, String> {
    control_plane.stop_graph_run(StopGraphRunCommand { run_id })
}

#[tauri::command]
fn load_execution_checkpoint(
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
fn load_graph_run_checkpoint(
    control_plane: State<'_, HostControlPlane>,
    run_id: String,
) -> Option<GraphRunCheckpoint> {
    control_plane.load_graph_run_checkpoint(GraphRunCheckpointQuery {
        run_id: Some(run_id),
    })
}

#[tauri::command]
fn inspect_host(
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
fn load_provider_registry() -> ProviderRegistryView {
    ProviderRegistryStore::new().load_view()
}

#[tauri::command]
fn save_provider_registry(registry: ProviderRegistryView) -> Result<ProviderRegistryView, String> {
    ProviderRegistryStore::new().save_view(registry)
}

#[tauri::command]
fn save_provider_registry_without_env_sync(
    registry: ProviderRegistryView,
) -> Result<ProviderRegistryView, String> {
    ProviderRegistryStore::new().save_view_without_env_sync(registry)
}

#[tauri::command]
fn list_sessions(control_plane: State<'_, HostControlPlane>) -> Vec<SessionOverview> {
    control_plane.list_sessions()
}

#[tauri::command]
fn load_session_runtime_view(
    control_plane: State<'_, HostControlPlane>,
    turn_id: Option<String>,
    session_id: Option<String>,
    node_id: Option<String>,
    run_id: Option<String>,
) -> SessionRuntimeView {
    control_plane.load_session_runtime_view(SessionRuntimeViewQuery {
        turn_id,
        session_id,
        node_id,
        run_id,
    })
}

#[tauri::command]
fn resolve_graph_run_submission_plan(
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
fn load_retrieved_context(
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
fn load_history_graph(
    control_plane: State<'_, HostControlPlane>,
    session_id: Option<String>,
) -> HistoryGraphView {
    control_plane.load_history_graph(HistoryGraphQuery { session_id })
}

#[tauri::command]
fn load_model_monitor_summary(
    control_plane: State<'_, HostControlPlane>,
    session_id: Option<String>,
) -> ModelMonitorSummaryView {
    control_plane.load_model_monitor_summary(ModelMonitorSummaryQuery { session_id })
}

#[tauri::command]
fn load_model_monitor_session_drilldown(
    control_plane: State<'_, HostControlPlane>,
    session_id: String,
) -> ModelMonitorSessionDrilldownView {
    control_plane
        .load_model_monitor_session_drilldown(ModelMonitorSessionDrilldownQuery { session_id })
}

#[tauri::command]
fn load_history_cursor(
    control_plane: State<'_, HostControlPlane>,
    session_id: Option<String>,
) -> HistoryCursorState {
    control_plane.load_history_cursor(HistoryCursorQuery { session_id })
}

#[tauri::command]
fn checkout_history_node(
    control_plane: State<'_, HostControlPlane>,
    session_id: Option<String>,
    node_id: String,
    mode: HistoryCheckoutMode,
) -> Result<HistoryCheckoutResponse, String> {
    control_plane.checkout_history_node(CheckoutHistoryNodeCommand {
        session_id,
        node_id,
        mode,
    })
}

#[tauri::command]
fn restore_branch_head(
    control_plane: State<'_, HostControlPlane>,
    session_id: Option<String>,
    branch_id: Option<String>,
) -> Result<RestoreBranchHeadResponse, String> {
    control_plane.restore_branch_head(RestoreBranchHeadCommand {
        session_id,
        branch_id,
    })
}

#[tauri::command]
fn fork_from_history_node(
    control_plane: State<'_, HostControlPlane>,
    session_id: Option<String>,
    node_id: String,
) -> Result<ForkFromHistoryNodeResponse, String> {
    control_plane.fork_from_history_node(ForkFromHistoryNodeCommand {
        session_id,
        node_id,
    })
}

#[tauri::command]
fn switch_history_branch(
    control_plane: State<'_, HostControlPlane>,
    session_id: Option<String>,
    branch_id: String,
) -> Result<SwitchHistoryBranchResponse, String> {
    control_plane.switch_history_branch(SwitchHistoryBranchCommand {
        session_id,
        branch_id,
    })
}

#[tauri::command]
fn delete_session(
    control_plane: State<'_, HostControlPlane>,
    session_id: String,
) -> Vec<SessionOverview> {
    control_plane.delete_session(DeleteSessionCommand { session_id })
}

#[tauri::command]
fn list_available_tools() -> Vec<ToolDefinition> {
    agent::tools::builtin_tools()
}

#[tauri::command]
fn list_capability_sources(
    control_plane: State<'_, HostControlPlane>,
) -> Vec<CapabilitySourceView> {
    control_plane.list_capability_sources()
}

#[tauri::command]
fn list_capabilities(
    source_id: Option<String>,
    kind: Option<String>,
    control_plane: State<'_, HostControlPlane>,
) -> Vec<CapabilityView> {
    control_plane.list_capabilities(CapabilityListQuery { source_id, kind })
}

#[tauri::command]
fn inspect_capability(
    capability_id: String,
    control_plane: State<'_, HostControlPlane>,
) -> Option<CapabilityView> {
    control_plane.inspect_capability(CapabilityInspectionQuery { capability_id })
}

#[tauri::command]
fn inspect_capability_source(
    source_id: String,
    control_plane: State<'_, HostControlPlane>,
) -> Option<CapabilitySourceView> {
    control_plane.inspect_capability_source(CapabilitySourceInspectionQuery { source_id })
}

#[tauri::command]
fn list_skills(
    source_id: Option<String>,
    control_plane: State<'_, HostControlPlane>,
) -> Vec<SkillDescriptor> {
    control_plane.list_skills(SkillListQuery { source_id })
}

#[tauri::command]
fn inspect_skill(
    skill_id: String,
    control_plane: State<'_, HostControlPlane>,
) -> Option<SkillDescriptor> {
    control_plane.inspect_skill(SkillInspectionQuery { skill_id })
}

#[tauri::command]
fn record_stream_debug_metrics(
    section: String,
    payload: Value,
    state: State<'_, StreamDebugMetricsState>,
) -> Result<(), String> {
    let mut latest = state
        .latest
        .lock()
        .map_err(|_| "stream debug lock poisoned".to_string())?;
    let current = latest.as_object().cloned().unwrap_or_default();
    let mut next = serde_json::Map::new();
    for (key, value) in current {
        next.insert(key, value);
    }
    next.insert(section.clone(), payload.clone());
    *latest = Value::Object(next);
    println!(
        "[pony-agent][stream-debug] section={} payload={}",
        section, payload
    );
    Ok(())
}

#[tauri::command]
fn load_stream_debug_metrics(state: State<'_, StreamDebugMetricsState>) -> Result<Value, String> {
    let latest = state
        .latest
        .lock()
        .map_err(|_| "stream debug lock poisoned".to_string())?;
    Ok(latest.clone())
}

pub fn run() {
    tauri::Builder::default()
        .manage(HostControlPlane::new())
        .manage(StreamDebugMetricsState {
            latest: Mutex::new(json!({})),
        })
        .setup(|app| {
            if let Some(window) = app.get_webview_window("main") {
                if let Some(icon) = app.default_window_icon().cloned() {
                    window.set_icon(icon)?;
                }
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            health_check,
            list_sessions,
            load_model_monitor_summary,
            load_model_monitor_session_drilldown,
            load_history_graph,
            load_history_cursor,
            load_session_runtime_view,
            resolve_graph_run_submission_plan,
            load_retrieved_context,
            checkout_history_node,
            restore_branch_head,
            fork_from_history_node,
            switch_history_branch,
            delete_session,
            list_available_tools,
            list_capability_sources,
            list_capabilities,
            inspect_capability,
            inspect_capability_source,
            list_skills,
            inspect_skill,
            load_provider_registry,
            run_turn,
            start_graph_run,
            start_graph_run_stream,
            continue_graph_run,
            continue_graph_run_stream,
            resume_graph_run,
            resume_graph_run_stream,
            start_turn_stream,
            stop_turn,
            stop_graph_run,
            load_execution_checkpoint,
            load_graph_run_checkpoint,
            inspect_host,
            record_stream_debug_metrics,
            load_stream_debug_metrics,
            save_provider_registry,
            save_provider_registry_without_env_sync
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Pony Agent");
}
