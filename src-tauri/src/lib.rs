mod agent;
pub mod sse_adapter;
mod tauri_adapter;

use agent::config::{ProviderRegistryStore, ProviderRegistryView};
use agent::context::RetrievedContextState;
use agent::control_plane::{
    ContinueGraphRunCommand, ContinueGraphRunStreamCommand, DeleteSessionCommand,
    ExecutionCheckpointQuery, GraphRunCheckpointQuery, GraphRunControlResponse,
    GraphRunStreamStartResponse, GraphRunTurnResponse, HostControlPlane, HostHealthSnapshot,
    HostInspectionQuery, HostInspectionSnapshot, ResumeGraphRunCommand,
    ResumeGraphRunStreamCommand, RetrievedContextQuery, RunTurnCommand, SessionRuntimeView,
    SessionRuntimeViewQuery, StartGraphRunCommand, StartGraphRunStreamCommand,
    StartTurnStreamCommand, StopGraphRunCommand, StopTurnCommand,
};
use agent::execution_control::{ExecutionCheckpoint, StopTurnResponse};
use agent::graph::GraphRunCheckpoint;
use agent::runtime::{TurnInput, TurnResult};
use agent::session::SessionOverview;
use agent::tools::ToolDefinition;
use tauri::{AppHandle, Manager, State};

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
    run_id: Option<String>,
) -> SessionRuntimeView {
    control_plane.load_session_runtime_view(SessionRuntimeViewQuery {
        turn_id,
        session_id,
        run_id,
    })
}

#[tauri::command]
fn load_retrieved_context(
    control_plane: State<'_, HostControlPlane>,
    turn_id: Option<String>,
    session_id: Option<String>,
    run_id: Option<String>,
) -> RetrievedContextState {
    control_plane.load_retrieved_context(RetrievedContextQuery {
        turn_id,
        session_id,
        run_id,
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

pub fn run() {
    tauri::Builder::default()
        .manage(HostControlPlane::new())
        .setup(|app| {
            if let Some(window) = app.get_webview_window("main") {
                if let Some(icon) = app.default_window_icon().cloned() {
                    window.set_icon(icon)?;
                }

                #[cfg(debug_assertions)]
                let _ = window.open_devtools();
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            health_check,
            list_sessions,
            load_session_runtime_view,
            load_retrieved_context,
            delete_session,
            list_available_tools,
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
            save_provider_registry,
            save_provider_registry_without_env_sync
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Pony Agent");
}
