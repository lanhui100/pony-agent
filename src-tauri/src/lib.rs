mod agent;

use agent::config::{ProviderRegistryStore, ProviderRegistryView};
use agent::runtime::{AgentRuntime, TurnInput, TurnResult};
use agent::session::{SessionOverview, SessionSnapshot};
use agent::tools::ToolDefinition;
use serde::Serialize;
use std::sync::Mutex;
use tauri::{AppHandle, Manager, State};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct HealthPayload {
    app_name: String,
    app_version: String,
    runtime: String,
    graph_engine: String,
}

#[tauri::command]
fn health_check(runtime: State<'_, Mutex<AgentRuntime>>) -> HealthPayload {
    let runtime = runtime.lock().expect("runtime lock poisoned");

    HealthPayload {
        app_name: "Pony Agent".to_string(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        runtime: runtime.name().to_string(),
        graph_engine: runtime.graph_engine().to_string(),
    }
}

#[tauri::command]
fn run_turn(runtime: State<'_, Mutex<AgentRuntime>>, input: TurnInput) -> TurnResult {
    let mut runtime = runtime.lock().expect("runtime lock poisoned");
    runtime.run_turn(input)
}

#[tauri::command]
fn start_turn_stream(
    app: AppHandle,
    turn_id: String,
    input: TurnInput,
) -> Result<(), String> {
    let app_handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let runtime_state = app_handle.state::<Mutex<AgentRuntime>>();
        let mut runtime = runtime_state.lock().expect("runtime lock poisoned");
        runtime.start_turn_stream(app_handle.clone(), turn_id, input);
    });
    Ok(())
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
fn list_sessions(runtime: State<'_, Mutex<AgentRuntime>>) -> Vec<SessionOverview> {
    let runtime = runtime.lock().expect("runtime lock poisoned");
    runtime.list_sessions()
}

#[tauri::command]
fn load_session_snapshot(
    runtime: State<'_, Mutex<AgentRuntime>>,
    session_id: Option<String>,
) -> SessionSnapshot {
    let mut runtime = runtime.lock().expect("runtime lock poisoned");
    runtime.load_session_snapshot(session_id.as_deref())
}

#[tauri::command]
fn delete_session(
    runtime: State<'_, Mutex<AgentRuntime>>,
    session_id: String,
) -> Vec<SessionOverview> {
    let mut runtime = runtime.lock().expect("runtime lock poisoned");
    runtime.remove_session(&session_id)
}

#[tauri::command]
fn list_available_tools() -> Vec<ToolDefinition> {
    agent::tools::builtin_tools()
}

pub fn run() {
    tauri::Builder::default()
        .manage(Mutex::new(AgentRuntime::new()))
        .invoke_handler(tauri::generate_handler![
            health_check,
            list_sessions,
            load_session_snapshot,
            delete_session,
            list_available_tools,
            load_provider_registry,
            run_turn,
            start_turn_stream,
            save_provider_registry,
            save_provider_registry_without_env_sync
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Pony Agent");
}
