pub use pony_agent_core::agent;
mod tauri_adapter;

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
use agent::graph::GraphRunCheckpoint;
use agent::runtime::{TurnInput, TurnResult};
use agent::session::SessionOverview;
use agent::session::TurnTraceRecord;
use agent::tools::{builtin_tool_contract_views, ToolDefinitionContractView};
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
    let _ = open_url_in_browser(&url);
}

fn open_url_in_browser(url: &str) {
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("cmd")
            .args(["/c", "start", &url.replace('&', "^&")])
            .spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(url).spawn();
    }
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    }
}

#[tauri::command]
fn list_sessions(control_plane: State<'_, HostControlPlane>) -> Vec<SessionOverview> {
    control_plane.list_sessions()
}

#[tauri::command]
fn load_session_traces(
    control_plane: State<'_, HostControlPlane>,
    session_id: String,
) -> Vec<TurnTraceRecord> {
    control_plane.load_session_traces(&session_id)
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
    builtin_tool_contract_views()
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

/// Icons embedded at compile time so they don't depend on runtime path resolution.
/// `icon.png` is a 512×512 RGBA PNG; `icon.ico` contains multiplatform frames (16–256).
static ICON_PNG: &[u8] = include_bytes!("../icons/icon.png");
#[cfg(target_os = "windows")]
static ICON_ICO: &[u8] = include_bytes!("../icons/icon.ico");

/// Windows-only: set BOTH ICON_SMALL (title bar) and ICON_BIG (taskbar + Alt+Tab)
/// from the embedded icon.ico.  Tauri's set_icon() only sends ICON_SMALL.
#[cfg(target_os = "windows")]
fn set_taskbar_icon_win32(app: &tauri::AppHandle) {
    use raw_window_handle::HasWindowHandle;

    extern "system" {
        fn LookupIconIdFromDirectoryEx(
            presbits: *const u8,
            ficon: i32,
            cxdesired: i32,
            cydesired: i32,
            flags: u32,
        ) -> i32;
        fn CreateIconFromResourceEx(
            presbits: *const u8,
            dwresSize: u32,
            ficon: i32,
            dwver: u32,
            cxdesired: i32,
            cydesired: i32,
            flags: u32,
        ) -> isize;
        fn SendMessageW(hwnd: isize, msg: u32, wparam: isize, lparam: isize) -> isize;
    }

    const WM_SETICON: u32 = 0x0080;
    const ICON_BIG: isize = 1;
    const ICON_SMALL: isize = 0;
    const LR_DEFAULTSIZE: u32 = 0x0040;

    let window = match app.get_webview_window("main") {
        Some(w) => w,
        None => {
            eprintln!("[icon-debug] main window not found");
            return;
        }
    };
    let hwnd = match window.window_handle() {
        Ok(h) => match h.as_raw() {
            raw_window_handle::RawWindowHandle::Win32(wh) => wh.hwnd.get() as isize,
            _ => {
                eprintln!("[icon-debug] unexpected window handle type");
                return;
            }
        },
        Err(e) => {
            eprintln!("[icon-debug] window_handle err: {e}");
            return;
        }
    };
    eprintln!("[icon-debug] HWND = 0x{hwnd:x}");

    unsafe {
        // LookupIconIdFromDirectoryEx finds the best-matching icon resource
        // entry for the default icon size (SM_CXICON × SM_CYICON).
        let id = LookupIconIdFromDirectoryEx(
            ICON_ICO.as_ptr(),
            1, // fIcon = TRUE (icon, not cursor)
            0, // cx=0 → use system metric
            0, // cy=0 → use system metric
            LR_DEFAULTSIZE,
        );
        eprintln!("[icon-debug] LookupIconIdFromDirectoryEx -> ID offset {id}");

        if id > 0 {
            let data = &ICON_ICO[id as usize..];
            let hicon = CreateIconFromResourceEx(
                data.as_ptr(),
                data.len() as u32,
                1,          // fIcon
                0x00030000, // dwVer (Windows 3.0 format)
                0,
                0, // desired size = default
                LR_DEFAULTSIZE,
            );
            eprintln!(
                "[icon-debug] HICON = {:p}",
                hicon as *const std::ffi::c_void
            );

            if hicon != 0 {
                SendMessageW(hwnd, WM_SETICON, ICON_BIG, hicon);
                eprintln!("[icon-debug] WM_SETICON(ICON_BIG) OK (taskbar)");
                SendMessageW(hwnd, WM_SETICON, ICON_SMALL, hicon);
                eprintln!("[icon-debug] WM_SETICON(ICON_SMALL) OK (title bar)");
                // HICON now owned by the window
            } else {
                eprintln!("[icon-debug] CreateIconFromResourceEx returned NULL");
            }
        } else {
            eprintln!("[icon-debug] LookupIconIdFromDirectoryEx returned {id} (no match)");
        }
    }
}

// ── Frontend Diagnostics (async + spawn_blocking to avoid main-thread SQLite I/O) ──

#[tauri::command]
async fn append_frontend_trace_events(
    app: AppHandle,
    events: Vec<FrontendTraceEvent>,
    stall_snapshots: Vec<FrontendStallSnapshot>,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let control_plane = app.state::<HostControlPlane>();
        control_plane.append_frontend_trace_events(FrontendTraceAppendCommand {
            events,
            stall_snapshots,
        })
    })
    .await
    .map_err(|e| format!("spawn_blocking error: {e}"))?
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
    tauri::async_runtime::spawn_blocking(move || {
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
    .await
    .map_err(|e| format!("spawn_blocking error: {e}"))?
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
    tauri::async_runtime::spawn_blocking(move || {
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
    .await
    .map_err(|e| format!("spawn_blocking error: {e}"))?
}

#[tauri::command]
async fn clear_frontend_trace_before(
    app: AppHandle,
    ts_wall_ms: i64,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let control_plane = app.state::<HostControlPlane>();
        control_plane.clear_frontend_trace_before(ts_wall_ms)
    })
    .await
    .map_err(|e| format!("spawn_blocking error: {e}"))?
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
    tauri::async_runtime::spawn_blocking(move || {
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
    .await
    .map_err(|e| format!("spawn_blocking error: {e}"))?
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
    tauri::async_runtime::spawn_blocking(move || {
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
    .await
    .map_err(|e| format!("spawn_blocking error: {e}"))?
}

pub fn run() {
    tauri::Builder::default()
        .manage(HostControlPlane::new())
        .manage(StreamDebugMetricsState {
            latest: Mutex::new(json!({})),
        })
        .setup(|app| {
            if let Some(window) = app.get_webview_window("main") {
                // === Load icon from compile-time embedded ICON_PNG ===
                // (avoids BaseDirectory::Resource resolving to target/debug/ in dev mode)
                match tauri::image::Image::from_bytes(ICON_PNG) {
                    Ok(icon) => {
                        eprintln!(
                            "[icon-debug] Decoded embedded icon.png: {}x{}",
                            icon.width(),
                            icon.height()
                        );
                        if let Err(e) = window.set_icon(icon) {
                            eprintln!("[icon-debug] set_icon err: {e}");
                            // fallback
                            if let Some(fb) = app.default_window_icon().cloned() {
                                let _ = window.set_icon(fb);
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("[icon-debug] from_bytes err: {e}");
                        if let Some(fb) = app.default_window_icon().cloned() {
                            eprintln!(
                                "[icon-debug] fallback default {}x{}",
                                fb.width(),
                                fb.height()
                            );
                            let _ = window.set_icon(fb);
                        }
                    }
                }

                // Windows: also explicitly set ICON_BIG for the taskbar,
                // since Tauri's set_icon() only sends ICON_SMALL.
                #[cfg(target_os = "windows")]
                set_taskbar_icon_win32(app.handle());
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            health_check,
            list_sessions,
            load_session_traces,
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
            get_service_api_key,
            set_service_api_key,
            open_url
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Pony Agent");
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── ICON_PNG: compiled-time embedded 512×512 icon.png ──

    #[test]
    fn embedded_icon_png_decodes_to_512x512() {
        let icon = tauri::image::Image::from_bytes(ICON_PNG)
            .expect("ICON_PNG must be a valid PNG image decodable by Image::from_bytes");
        assert_eq!(
            icon.width(),
            512,
            "embedded ICON_PNG must be 512px wide, got {}",
            icon.width()
        );
        assert_eq!(
            icon.height(),
            512,
            "embedded ICON_PNG must be 512px tall, got {}",
            icon.height()
        );
    }

    #[test]
    fn embedded_icon_png_is_not_empty() {
        assert!(
            !ICON_PNG.is_empty(),
            "ICON_PNG must not be empty (include_bytes! should embed a real file)"
        );
        assert!(
            ICON_PNG.len() > 1000,
            "ICON_PNG size ({}) suspiciously small for a 512×512 PNG",
            ICON_PNG.len()
        );
    }

    // ── ICON_ICO: compiled-time embedded icon.ico (Windows-only) ──

    #[cfg(target_os = "windows")]
    #[test]
    fn embedded_ico_has_valid_header() {
        // ICO header: 2B reserved(0) + 2B type(1) + 2B count
        assert!(ICON_ICO.len() >= 6, "ICO file too small for header");
        assert_eq!(ICON_ICO[0], 0, "ICO reserved byte 0 must be 0");
        assert_eq!(ICON_ICO[1], 0, "ICO reserved byte 1 must be 0");
        assert_eq!(
            ICON_ICO[2], 1,
            "ICO type must be 1 (icon), got {}",
            ICON_ICO[2]
        );
        assert_eq!(ICON_ICO[3], 0, "ICO type high byte must be 0");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn embedded_ico_contains_256x256_frame() {
        let count = u16::from_le_bytes([ICON_ICO[4], ICON_ICO[5]]);
        assert!(count >= 1, "ICO must have at least 1 frame, has {count}");

        let mut found_256 = false;
        for i in 0..count {
            let entry_off = 6 + (i as usize) * 16;
            if entry_off + 16 > ICON_ICO.len() {
                break;
            }
            let w = ICON_ICO[entry_off] as u32;
            let h = ICON_ICO[entry_off + 1] as u32;
            // In ICO, width/height of 0 means 256
            let w = if w == 0 { 256 } else { w };
            let h = if h == 0 { 256 } else { h };
            if w == 256 && h == 256 {
                found_256 = true;
                // Validate the frame data offset + size
                let data_size =
                    u32::from_le_bytes(ICON_ICO[entry_off + 8..entry_off + 12].try_into().unwrap());
                let data_off = u32::from_le_bytes(
                    ICON_ICO[entry_off + 12..entry_off + 16].try_into().unwrap(),
                );
                assert!(
                    data_size > 1000,
                    "256×256 frame data size ({data_size}) too small"
                );
                assert!(
                    (data_off as usize) + (data_size as usize) <= ICON_ICO.len(),
                    "256×256 frame data extends beyond file"
                );
            }
        }
        assert!(
            found_256,
            "ICO must contain a 256×256 frame (width=0, height=0 in ICO entry)"
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn embedded_ico_has_reasonable_frame_count() {
        let count = u16::from_le_bytes([ICON_ICO[4], ICON_ICO[5]]);
        assert!(
            count >= 3,
            "ICO should have at least 3 frames for multi-res support, has {count}"
        );
        assert!(
            count <= 20,
            "ICO has {count} frames — unusually high, may be accidental"
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn embedded_ico_all_entries_reference_valid_data() {
        let count = u16::from_le_bytes([ICON_ICO[4], ICON_ICO[5]]);
        for i in 0..count {
            let entry_off = 6 + (i as usize) * 16;
            if entry_off + 16 > ICON_ICO.len() {
                panic!("Frame {i} entry truncated");
            }
            let data_size =
                u32::from_le_bytes(ICON_ICO[entry_off + 8..entry_off + 12].try_into().unwrap());
            let data_off =
                u32::from_le_bytes(ICON_ICO[entry_off + 12..entry_off + 16].try_into().unwrap());
            assert!(data_size > 0, "Frame {i} has zero data size");
            assert!(
                (data_off as usize) + (data_size as usize) <= ICON_ICO.len(),
                "Frame {i} data [offset={data_off}, size={data_size}] exceeds file length {}",
                ICON_ICO.len()
            );
        }
    }
}
