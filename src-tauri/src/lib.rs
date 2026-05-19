mod agent;

use agent::config::{ProviderRegistryStore, ProviderRegistryView};
use agent::runtime::{AgentRuntime, TurnInput, TurnResult};
use serde::Serialize;
use tauri::AppHandle;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct HealthPayload {
    app_name: String,
    app_version: String,
    runtime: String,
    graph_engine: String,
}

#[tauri::command]
fn health_check() -> HealthPayload {
    let runtime = AgentRuntime::new();

    HealthPayload {
        app_name: "Pony Agent".to_string(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        runtime: runtime.name().to_string(),
        graph_engine: runtime.graph_engine().to_string(),
    }
}

#[tauri::command]
fn run_turn(input: TurnInput) -> TurnResult {
    let runtime = AgentRuntime::new();
    runtime.run_turn(input)
}

#[tauri::command]
fn start_turn_stream(app: AppHandle, turn_id: String, input: TurnInput) -> Result<(), String> {
    let runtime = AgentRuntime::new();
    runtime.start_turn_stream(app, turn_id, input);
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

pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            health_check,
            load_provider_registry,
            run_turn,
            start_turn_stream,
            save_provider_registry
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Pony Agent");
}
