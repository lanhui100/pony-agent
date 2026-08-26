pub use pony_agent_core::agent;
mod blocking_helper;
mod platform;
mod tauri_adapter;
mod turn_task_registry;

use crate::blocking_helper::BlockingHelper;
use crate::turn_task_registry::TurnTaskRegistry;
use agent::app_settings::{AppSettings, AppSettingsStore};
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
use agent::frontend_diagnostics::{
    FrontendStallSnapshot, FrontendTraceAppendCommand, FrontendTraceEvent,
    FrontendTraceExportPayload, FrontendTraceQuery, FrontendTraceQueryResult,
};
use agent::graph::GraphAskWaitBinding;
use agent::graph::GraphRunCheckpoint;
use agent::runtime::{TurnInput, TurnResult};
use agent::session::SessionOverview;
use agent::session::TurnTraceRecord;
use agent::tools::{builtin_turn_tool_contract_views, ToolDefinitionContractView};
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
    tauri_adapter::spawn_graph_run_stream(&app, prepared)?;
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
    tauri_adapter::spawn_graph_run_stream(&app, prepared)?;
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
    tauri_adapter::spawn_graph_run_stream(&app, prepared)?;
    Ok(response)
}

#[tauri::command]
fn start_turn_stream(app: AppHandle, turn_id: String, input: TurnInput) -> Result<(), String> {
    tauri_adapter::spawn_turn_stream(&app, StartTurnStreamCommand { turn_id, input })
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

// ── Ask control surface (PA-076 task 4.4) ──────────────────────────────────────────────────

/// Snapshot of every pending Ask (`Interaction`) request, optionally filtered by session.
#[tauri::command]
fn ask_list_pending(
    control_plane: State<'_, HostControlPlane>,
    session_id: Option<String>,
) -> Value {
    control_plane.list_pending_asks(session_id.as_deref())
}

/// Answer a pending Ask by stable `request_id` and compare-and-swap `version`, carrying the
/// user's answer payload.
#[tauri::command]
fn ask_answer(
    control_plane: State<'_, HostControlPlane>,
    request_id: String,
    version: u64,
    answer: Value,
) -> Result<Value, String> {
    control_plane.answer_ask(&request_id, version, answer)
}

/// Cancel a pending Ask by stable `request_id` and compare-and-swap `version`.
#[tauri::command]
fn ask_cancel(
    control_plane: State<'_, HostControlPlane>,
    request_id: String,
    version: u64,
) -> Result<Value, String> {
    control_plane.cancel_ask(&request_id, version)
}

/// Transition every expired pending request to `Expired`. `now_ms` defaults to the dispatcher
/// clock when omitted.
#[tauri::command]
fn ask_expire(control_plane: State<'_, HostControlPlane>, now_ms: Option<u64>) -> Value {
    control_plane.expire_asks(now_ms)
}

// ── Plan control surface (PA-076 task 4.4) ─────────────────────────────────────────────────

/// Create a session-owned `Draft` plan from a `{ kind, summary, steps }` payload.
#[tauri::command]
fn plan_create(
    control_plane: State<'_, HostControlPlane>,
    session_id: String,
    payload: Value,
) -> Result<Value, String> {
    control_plane.plan_create(&session_id, payload)
}

/// Replace a plan's content, preserving its `plan_id`, with a compare-and-swap `revision`.
#[tauri::command]
fn plan_replace(
    control_plane: State<'_, HostControlPlane>,
    session_id: String,
    plan_id: String,
    revision: u64,
    payload: Value,
) -> Result<Value, String> {
    control_plane.plan_replace(&session_id, &plan_id, revision, payload)
}

/// Append a single step to a plan, preserving every existing `step_id`.
#[tauri::command]
fn plan_merge(
    control_plane: State<'_, HostControlPlane>,
    session_id: String,
    plan_id: String,
    revision: u64,
    step: Value,
) -> Result<Value, String> {
    control_plane.plan_merge(&session_id, &plan_id, revision, step)
}

/// Mark a plan step completed (first completion -> Executing, last -> Completed).
#[tauri::command]
fn plan_complete_step(
    control_plane: State<'_, HostControlPlane>,
    session_id: String,
    plan_id: String,
    revision: u64,
    step_id: String,
) -> Result<Value, String> {
    control_plane.plan_complete_step(&session_id, &plan_id, revision, &step_id)
}

/// Every plan owned by a session, in creation order.
#[tauri::command]
fn plan_list(
    control_plane: State<'_, HostControlPlane>,
    session_id: String,
) -> Value {
    control_plane.plan_list(&session_id)
}

/// Read a single plan by stable id.
#[tauri::command]
fn plan_get(
    control_plane: State<'_, HostControlPlane>,
    session_id: String,
    plan_id: String,
) -> Result<Value, String> {
    control_plane.plan_get(&session_id, &plan_id)
}

// ── graph Ask wait surface (PA-076 task 4.4) ───────────────────────────────────────────────

/// Snapshot of every Ask wait bound to a graph run.
#[tauri::command]
fn graph_list_ask_waits(
    control_plane: State<'_, HostControlPlane>,
    run_id: String,
) -> Value {
    control_plane.graph_list_ask_waits(&run_id)
}

/// Bind an Ask `PendingControlRequest` to a run's `wait_user` suspension.
#[tauri::command]
fn graph_bind_ask_wait(
    control_plane: State<'_, HostControlPlane>,
    run_id: String,
    binding: GraphAskWaitBinding,
) -> Result<Value, String> {
    control_plane.graph_bind_ask_wait(&run_id, binding)
}

/// Resolve a bound Ask wait, injecting exactly one terminal tool result for the original call id.
/// A stale `version` is rejected.
#[tauri::command]
fn graph_resume_ask(
    control_plane: State<'_, HostControlPlane>,
    run_id: String,
    request_id: String,
    version: u64,
    answer: Value,
) -> Result<Value, String> {
    control_plane.graph_resume_ask(&run_id, &request_id, version, answer)
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

/// /models 目录拉取（design D3）：key 解析顺序为显式入参 → registry store 已存密钥
/// → 报错"缺少 API Key"；鉴权头尊重该协议 endpoint 的显式 auth_type（Auto 才按家族
/// 推导）；key 不入任何日志。
#[tauri::command]
fn fetch_provider_models(
    provider_id: Option<String>,
    protocol: agent::provider::ProviderProtocol,
    base_url: String,
    api_key: Option<String>,
) -> Result<Vec<String>, String> {
    let explicit_key = api_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let store = ProviderRegistryStore::new();
    let api_key = match explicit_key {
        Some(key) => key,
        None => store
            .provider_api_key(provider_id.as_deref())
            .ok_or_else(|| "缺少 API Key".to_string())?,
    };
    let auth_hint = store.provider_endpoint_auth(provider_id.as_deref(), &protocol);
    let base_url = base_url.trim();
    if base_url.is_empty() {
        return Err("缺少 Base URL；请先在提供商高级设置中填写该协议的 Base URL。".to_string());
    }
    agent::provider::fetch_model_ids(&protocol, base_url, &auth_hint, &api_key)
}

#[tauri::command]
fn load_app_settings() -> AppSettings {
    AppSettingsStore::new().load_view()
}

#[tauri::command]
fn save_app_settings(settings: AppSettings) -> Result<AppSettings, String> {
    AppSettingsStore::new().save_view(settings)
}

#[tauri::command]
fn get_service_api_key(service: String) -> Result<String, String> {
    let store = pony_agent_core::agent::config::ProviderRegistryStore::new();
    Ok(store.get_service_api_key(&service).unwrap_or_default())
}

#[tauri::command]
fn set_service_api_key(service: String, key: String) -> Result<(), String> {
    let store = pony_agent_core::agent::config::ProviderRegistryStore::new();
    store.set_service_api_key(&service, &key)
}

#[tauri::command]
fn open_url(url: String) {
    let _ = platform::open_url_in_browser(&url);
}

#[tauri::command]
fn list_sessions(control_plane: State<'_, HostControlPlane>) -> Vec<SessionOverview> {
    control_plane.list_sessions()
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ImportAttachmentResultView {
    path: String,
    relative_path: Option<String>,
}

#[tauri::command]
fn get_workspace_root(control_plane: State<'_, HostControlPlane>) -> String {
    control_plane.get_workspace_root()
}

#[tauri::command]
fn import_attachment(
    control_plane: State<'_, HostControlPlane>,
    name: String,
    bytes_b64: String,
    mime_type: String,
    workspace_id: Option<String>,
) -> Result<ImportAttachmentResultView, String> {
    control_plane
        .import_attachment(
            &name,
            &bytes_b64,
            &mime_type,
            workspace_id.as_deref(),
            None,
            None,
        )
        .map(|result| ImportAttachmentResultView {
            path: result.path.display().to_string(),
            relative_path: result.relative_path,
        })
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceRecordView {
    id: String,
    name: String,
    root_path: String,
}

#[tauri::command]
fn workspace_list(control_plane: State<'_, HostControlPlane>) -> Vec<WorkspaceRecordView> {
    control_plane
        .list_workspaces()
        .into_iter()
        .map(|workspace| WorkspaceRecordView {
            id: workspace.id,
            name: workspace.name,
            root_path: workspace.root_path,
        })
        .collect()
}

#[tauri::command]
fn workspace_create(
    control_plane: State<'_, HostControlPlane>,
    name: String,
    root_path: String,
) -> Result<WorkspaceRecordView, String> {
    control_plane
        .create_workspace(&name, &root_path)
        .map(|workspace| WorkspaceRecordView {
            id: workspace.id,
            name: workspace.name,
            root_path: workspace.root_path,
        })
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AuthorizedPathEntryView {
    path: String,
    granted_at_ms: u64,
}

#[tauri::command]
fn authorize_path(
    control_plane: State<'_, HostControlPlane>,
    path: String,
    scope: String,
) -> Result<AuthorizedPathEntryView, String> {
    control_plane
        .authorize_path(&path, &scope)
        .map(|entry| AuthorizedPathEntryView {
            path: entry.path.display().to_string(),
            granted_at_ms: entry.granted_at_ms,
        })
}

#[tauri::command]
fn revoke_authorization(
    control_plane: State<'_, HostControlPlane>,
    path: String,
) -> Result<bool, String> {
    control_plane.revoke_authorization(&path)
}

#[tauri::command]
fn list_authorizations(
    control_plane: State<'_, HostControlPlane>,
) -> Vec<AuthorizedPathEntryView> {
    control_plane
        .list_authorizations()
        .into_iter()
        .map(|entry| AuthorizedPathEntryView {
            path: entry.path.display().to_string(),
            granted_at_ms: entry.granted_at_ms,
        })
        .collect()
}

#[tauri::command]
fn load_session_traces(
    control_plane: State<'_, HostControlPlane>,
    session_id: String,
) -> Vec<TurnTraceRecord> {
    control_plane.load_session_traces(&session_id)
}

/// PA-094：大字段外置——按引用加载 build_context_observation 全量 payload。
/// 引用格式 `bco:<turn_id>:<seq>`；未命中返回 null（legacy 内嵌数据走 trace 字段）。
#[tauri::command]
fn load_build_context_observation(
    control_plane: State<'_, HostControlPlane>,
    session_id: String,
    observation_ref: String,
) -> Option<agent::provider::BuildContextObservation> {
    control_plane.load_build_context_observation(&session_id, &observation_ref)
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
fn restore_branch_head(
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
fn fork_from_history_node(
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
fn switch_history_branch(
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
fn delete_session(
    control_plane: State<'_, HostControlPlane>,
    session_id: String,
) -> Vec<SessionOverview> {
    control_plane.delete_session(DeleteSessionCommand { session_id })
}

#[tauri::command]
fn list_available_tools() -> Vec<ToolDefinitionContractView> {
    builtin_turn_tool_contract_views()
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


// ── Frontend Diagnostics (async + spawn_blocking to avoid main-thread SQLite I/O) ──

#[tauri::command]
async fn append_frontend_trace_events(
    app: AppHandle,
    events: Vec<FrontendTraceEvent>,
    stall_snapshots: Vec<FrontendStallSnapshot>,
) -> Result<(), String> {
    BlockingHelper::spawn(move || {
        let control_plane = app.state::<HostControlPlane>();
        control_plane.append_frontend_trace_events(FrontendTraceAppendCommand {
            events,
            stall_snapshots,
        })
    })
    .await?
}

#[tauri::command]
async fn query_frontend_trace_window(
    app: AppHandle,
    session_id: Option<String>,
    turn_id: Option<String>,
    from_wall_ms: Option<i64>,
    to_wall_ms: Option<i64>,
    limit: Option<u32>,
    cursor: Option<String>,
) -> Result<FrontendTraceQueryResult, String> {
    BlockingHelper::spawn(move || {
        let control_plane = app.state::<HostControlPlane>();
        control_plane.query_frontend_trace_window(FrontendTraceQuery {
            session_id,
            turn_id,
            from_wall_ms,
            to_wall_ms,
            limit,
            cursor,
        })
    })
    .await?
}

#[tauri::command]
async fn query_frontend_stall_snapshots(
    app: AppHandle,
    session_id: Option<String>,
    turn_id: Option<String>,
    from_wall_ms: Option<i64>,
    to_wall_ms: Option<i64>,
    limit: Option<u32>,
) -> Result<Vec<FrontendStallSnapshot>, String> {
    BlockingHelper::spawn(move || {
        let control_plane = app.state::<HostControlPlane>();
        control_plane.query_frontend_stall_snapshots(FrontendTraceQuery {
            session_id,
            turn_id,
            from_wall_ms,
            to_wall_ms,
            limit,
            cursor: None,
        })
    })
    .await?
}

#[tauri::command]
async fn clear_frontend_trace_before(app: AppHandle, ts_wall_ms: i64) -> Result<(), String> {
    BlockingHelper::spawn(move || {
        let control_plane = app.state::<HostControlPlane>();
        control_plane.clear_frontend_trace_before(ts_wall_ms)
    })
    .await?
}

#[tauri::command]
async fn export_frontend_trace_json(
    app: AppHandle,
    session_id: Option<String>,
    turn_id: Option<String>,
    from_wall_ms: Option<i64>,
    to_wall_ms: Option<i64>,
    limit: Option<u32>,
) -> Result<FrontendTraceExportPayload, String> {
    BlockingHelper::spawn(move || {
        let control_plane = app.state::<HostControlPlane>();
        control_plane.export_frontend_trace_json(FrontendTraceQuery {
            session_id,
            turn_id,
            from_wall_ms,
            to_wall_ms,
            limit,
            cursor: None,
        })
    })
    .await?
}

#[tauri::command]
async fn export_frontend_trace_chrome_trace(
    app: AppHandle,
    session_id: Option<String>,
    turn_id: Option<String>,
    from_wall_ms: Option<i64>,
    to_wall_ms: Option<i64>,
    limit: Option<u32>,
) -> Result<FrontendTraceExportPayload, String> {
    BlockingHelper::spawn(move || {
        let control_plane = app.state::<HostControlPlane>();
        control_plane.export_frontend_trace_chrome_trace(FrontendTraceQuery {
            session_id,
            turn_id,
            from_wall_ms,
            to_wall_ms,
            limit,
            cursor: None,
        })
    })
    .await?
}

pub fn run() {
    tauri::Builder::default()
        .manage(HostControlPlane::new())
        .manage(StreamDebugMetricsState {
            latest: Mutex::new(json!({})),
        })
        .manage(TurnTaskRegistry::with_max_concurrent(3))
        .setup(|app| {
            if let Some(window) = app.get_webview_window("main") {
                platform::apply_icons(app);
                platform::apply_window_style(&window);
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            health_check,
            list_sessions,
            get_workspace_root,
            import_attachment,
            workspace_list,
            workspace_create,
            authorize_path,
            revoke_authorization,
            list_authorizations,
            load_session_traces,
            load_build_context_observation,
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
            load_app_settings,
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
            ask_list_pending,
            ask_answer,
            ask_cancel,
            ask_expire,
            plan_create,
            plan_replace,
            plan_merge,
            plan_complete_step,
            plan_list,
            plan_get,
            graph_list_ask_waits,
            graph_bind_ask_wait,
            graph_resume_ask,
            load_execution_checkpoint,
            load_graph_run_checkpoint,
            inspect_host,
            record_stream_debug_metrics,
            load_stream_debug_metrics,
            append_frontend_trace_events,
            query_frontend_trace_window,
            query_frontend_stall_snapshots,
            clear_frontend_trace_before,
            export_frontend_trace_json,
            export_frontend_trace_chrome_trace,
            save_app_settings,
            save_provider_registry,
            save_provider_registry_without_env_sync,
            fetch_provider_models,
            get_service_api_key,
            set_service_api_key,
            open_url
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                window.state::<TurnTaskRegistry>().abort_all();
            }
        })
        .run(tauri::generate_context!())
        .expect("failed to run Pony Agent");
}
