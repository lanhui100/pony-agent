mod agent;

use serde::Serialize;

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
    let runtime = agent::runtime::AgentRuntime::new();

    HealthPayload {
        app_name: "Pony Agent".to_string(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        runtime: runtime.name().to_string(),
        graph_engine: runtime.graph_engine().to_string(),
    }
}

pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![health_check])
        .run(tauri::generate_context!())
        .expect("failed to run Pony Agent");
}
