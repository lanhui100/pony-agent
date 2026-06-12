use pony_agent_core::agent::config::{
    ProviderModelCapabilities, ProviderSelectionResolver, ResolvedProviderSelection,
};
use pony_agent_core::agent::control_plane::{
    HostControlPlaneBuilder, RunTurnCommand, StartGraphRunStreamCommand, StartTurnStreamCommand,
};
use pony_agent_core::agent::graph::GraphRunStore;
use pony_agent_core::agent::provider::ProviderProtocol;
use pony_agent_core::agent::runtime::{AgentRuntimeBuilder, TurnInput, TurnStreamEvent};
use pony_agent_core::agent::session::{FileSessionBackend, SessionStore};
use pony_agent_core::agent::tools::{ToolCall, ToolRouter};
use pony_agent_core::agent::turn_flow::TurnEventSink;
use serde_json::json;
use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

struct StaticResolver;

impl ProviderSelectionResolver for StaticResolver {
    fn resolve_provider_selection(
        &self,
        _provider_id: Option<&str>,
        _model_id: Option<&str>,
    ) -> ResolvedProviderSelection {
        ResolvedProviderSelection {
            requested_name: "non-tauri-mock".to_string(),
            provider_name: "non-tauri-mock".to_string(),
            protocol: ProviderProtocol::OpenAi,
            base_url: "http://127.0.0.1:1/v1".to_string(),
            api_key_env_var: "NON_TAURI_MOCK_API_KEY".to_string(),
            api_key: None,
            model: "non-tauri-mock-model".to_string(),
            temperature: 0.0,
            max_output_tokens: 64,
            reasoning_effort: None,
            reasoning_budget_tokens: None,
            capabilities: ProviderModelCapabilities::default(),
        }
    }
}

struct RecordingSink {
    events: RefCell<Vec<(String, TurnStreamEvent)>>,
}

impl RecordingSink {
    fn new() -> Self {
        Self {
            events: RefCell::new(Vec::new()),
        }
    }

    fn has_event(&self, name: &str) -> bool {
        self.events
            .borrow()
            .iter()
            .any(|(event_name, _)| event_name == name)
    }
}

impl TurnEventSink for RecordingSink {
    fn emit(&self, name: &str, payload: TurnStreamEvent) {
        self.events.borrow_mut().push((name.to_string(), payload));
    }
}

fn main() {
    let root = unique_temp_root();
    if let Err(error) = run_harness(&root) {
        eprintln!("non_tauri_harness failed: {error}");
        std::process::exit(1);
    }
    let _ = fs::remove_dir_all(root);
}

fn run_harness(root: &Path) -> Result<(), String> {
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).map_err(|error| error.to_string())?;
    fs::write(
        workspace.join("demo.txt"),
        "hello from injected workspace\n",
    )
    .map_err(|error| error.to_string())?;

    let session_path = root.join("state").join("sessions.json");
    let graph_path = root.join("state").join("graph-runs.json");

    let runtime_builder = AgentRuntimeBuilder::new()
        .session_store(SessionStore::with_backend(Box::new(
            FileSessionBackend::new(session_path.clone()),
        )))
        .provider_resolver(Box::new(StaticResolver))
        .workspace_root(workspace.clone());

    let control_plane = HostControlPlaneBuilder::new()
        .runtime_builder(runtime_builder)
        .graph_run_store(GraphRunStore::persistent(graph_path.clone()))
        .build();

    let sync_result = control_plane.run_turn(RunTurnCommand {
        input: turn_input("   ", "non-tauri-sync"),
    });
    if sync_result.phase != "failed" {
        return Err(format!(
            "expected sync failed phase, got {}",
            sync_result.phase
        ));
    }

    let sink = RecordingSink::new();
    control_plane.start_turn_stream(
        &sink,
        StartTurnStreamCommand {
            turn_id: "non-tauri-stream-turn".to_string(),
            input: turn_input("   ", "non-tauri-stream"),
        },
    );
    if !sink.has_event("turn:failed") {
        return Err("stream turn did not emit turn:failed through non-Tauri sink".to_string());
    }

    let graph_response =
        control_plane.prepare_start_graph_run_stream(StartGraphRunStreamCommand {
            turn_id: "non-tauri-graph-turn".to_string(),
            run_id: Some("non-tauri-run".to_string()),
            goal: "prove non-tauri graph construction".to_string(),
            input: turn_input("   ", "non-tauri-graph"),
        })?;
    if graph_response.0.run.id != "non-tauri-run" {
        return Err("graph run id did not roundtrip through injected graph store".to_string());
    }

    let tool_router = ToolRouter::with_workspace_root(workspace);
    let tool_result = tool_router.execute(&ToolCall {
        call_id: None,
        name: "workspace_read_file".to_string(),
        arguments: json!({ "path": "demo.txt" }),
        plan: None,
    });
    if tool_result.status != "ok" || !tool_result.output.contains("hello from injected workspace") {
        return Err("workspace tool did not use injected workspace root".to_string());
    }

    let reloaded_control_plane = HostControlPlaneBuilder::new()
        .runtime_builder(
            AgentRuntimeBuilder::new()
                .session_store(SessionStore::with_backend(Box::new(
                    FileSessionBackend::new(session_path),
                )))
                .provider_resolver(Box::new(StaticResolver)),
        )
        .graph_run_store(GraphRunStore::persistent(graph_path))
        .build();
    let reloaded_checkpoint = reloaded_control_plane
        .load_graph_run_checkpoint(
            pony_agent_core::agent::control_plane::GraphRunCheckpointQuery {
                run_id: Some("non-tauri-run".to_string()),
            },
        )
        .ok_or_else(|| "injected graph persistence did not reload checkpoint".to_string())?;
    if reloaded_checkpoint.run_id != "non-tauri-run" {
        return Err("injected graph persistence did not reload run".to_string());
    }

    println!("non_tauri_harness ok");
    Ok(())
}

fn turn_input(message: &str, session_id: &str) -> TurnInput {
    TurnInput {
        message: message.to_string(),
        display_message: None,
        provider_id: None,
        model_id: None,
        reasoning_effort: None,
        session_id: Some(session_id.to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
    }
}

fn unique_temp_root() -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("pony-agent-non-tauri-harness-{stamp}"))
}
