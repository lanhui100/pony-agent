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
    SwitchHistoryBranchCommand, SwitchHistoryBranchResponse, FailTurnForWatchdogCommand,
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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::{AppHandle, Manager, State};

/// 退出 flush 的幂等短路位（C5.3）：`ExitRequested` 与 `Exit` 共享，避免预算叠加。
static EXIT_FLUSH_DONE: AtomicBool = AtomicBool::new(false);

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
fn stop_turn(
    control_plane: State<'_, HostControlPlane>,
    task_registry: State<'_, TurnTaskRegistry>,
    turn_id: String,
) -> StopTurnResponse {
    let checkpoint = control_plane.load_execution_checkpoint(ExecutionCheckpointQuery {
        turn_id: Some(turn_id.clone()),
        session_id: None,
    });
    if let Some(ref cp) = checkpoint {
        if let Some(ref session_id) = cp.session_id {
            task_registry.abort_session(session_id);
        }
    }
    task_registry.abort_session(&turn_id);
    control_plane.stop_turn(StopTurnCommand { turn_id })
}

#[tauri::command]
fn stop_graph_run(
    control_plane: State<'_, HostControlPlane>,
    run_id: String,
) -> Result<GraphRunControlResponse, String> {
    control_plane.stop_graph_run(StopGraphRunCommand { run_id })
}

/// 契约 watchdog-fix backend：前端 watchdog 触发时调用，后端立即幂等落盘 failed trace
/// + checkpoint failed + request_stop（可选 stop_graph_run），返回 StopTurnResponse 等价结构。
#[tauri::command]
fn fail_turn_for_watchdog(
    control_plane: State<'_, HostControlPlane>,
    turn_id: String,
    session_id: Option<String>,
    run_id: Option<String>,
    error: Option<String>,
    reason: Option<String>,
) -> StopTurnResponse {
    control_plane.fail_turn_for_watchdog(FailTurnForWatchdogCommand {
        turn_id,
        session_id,
        run_id,
        error: error.unwrap_or_else(|| "submission_watchdog_timeout".to_string()),
        reason,
    })
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

/// Answer a pending Ask by stable `request_id` and compare-and-swap `version` (or `expected_version`),
/// carrying the user's answer payload.
#[tauri::command]
fn ask_answer(
    control_plane: State<'_, HostControlPlane>,
    request_id: String,
    version: Option<u64>,
    expected_version: Option<u64>,
    answer: Value,
) -> Result<Value, String> {
    let ver = expected_version
        .or(version)
        .ok_or_else(|| "missing required key `expected_version` or `version`".to_string())?;
    control_plane.answer_ask(&request_id, ver, answer)
}

/// Approve an Approval control request by stable `request_id` and compare-and-swap `version` (or `expected_version`).
#[tauri::command]
fn control_approve_request(
    control_plane: State<'_, HostControlPlane>,
    request_id: String,
    version: Option<u64>,
    expected_version: Option<u64>,
) -> Result<Value, String> {
    let ver = expected_version
        .or(version)
        .ok_or_else(|| "missing required key `expected_version` or `version`".to_string())?;
    control_plane.approve_control_request(&request_id, ver)
}

/// Cancel a pending Ask by stable `request_id` and compare-and-swap `version` (or `expected_version`).
#[tauri::command]
fn ask_cancel(
    control_plane: State<'_, HostControlPlane>,
    request_id: String,
    version: Option<u64>,
    expected_version: Option<u64>,
) -> Result<Value, String> {
    let ver = expected_version
        .or(version)
        .ok_or_else(|| "missing required key `expected_version` or `version`".to_string())?;
    control_plane.cancel_ask(&request_id, ver)
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

/// Host approval: explicitly transition a Draft plan to Executing (PA-141).
#[tauri::command]
fn plan_execution_approve(
    control_plane: State<'_, HostControlPlane>,
    session_id: String,
    plan_id: String,
    revision: u64,
) -> Result<Value, String> {
    control_plane.plan_execution_approve(&session_id, &plan_id, revision)
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

/// List todos for a session.
#[tauri::command]
fn todo_list(
    session_id: String,
) -> Result<Value, String> {
    let args = pony_agent_core::agent::todo::TodoListArgs {
        session_id: Some(session_id),
    };
    let res = pony_agent_core::agent::todo::todo_list(args)?;
    serde_json::to_value(res.todos).map_err(|e| e.to_string())
}

/// Get current goal for a session.
#[tauri::command]
fn goal_get(
    session_id: String,
) -> Result<Value, String> {
    let args = pony_agent_core::agent::goal::GetGoalArgs {
        session_id: Some(session_id),
    };
    let res = pony_agent_core::agent::goal::get_goal(args)?;
    serde_json::to_value(res.goal).map_err(|e| e.to_string())
}

/// Create a goal for a session.
#[tauri::command]
fn goal_create(
    session_id: String,
    objective: String,
    max_goal_rounds: Option<u32>,
) -> Result<Value, String> {
    let args = pony_agent_core::agent::goal::CreateGoalArgs {
        session_id: Some(session_id),
        objective,
        max_goal_rounds,
    };
    let res = pony_agent_core::agent::goal::create_goal(args)?;
    serde_json::to_value(res.goal).map_err(|e| e.to_string())
}

/// Update a goal for a session using CAS.
#[tauri::command]
fn goal_update(
    session_id: String,
    goal_id: String,
    revision: u64,
    action: String,
    objective: Option<String>,
    blocked_reason: Option<String>,
    max_goal_rounds: Option<u32>,
) -> Result<Value, String> {
    let update_action = match action.to_lowercase().as_str() {
        "complete" => pony_agent_core::agent::goal::UpdateGoalAction::Complete,
        "pause" => pony_agent_core::agent::goal::UpdateGoalAction::Pause,
        "resume" => pony_agent_core::agent::goal::UpdateGoalAction::Resume,
        "edit" => pony_agent_core::agent::goal::UpdateGoalAction::Edit,
        "blocked" => pony_agent_core::agent::goal::UpdateGoalAction::Blocked,
        other => return Err(format!("Unsupported goal action: {}", other)),
    };
    let args = pony_agent_core::agent::goal::UpdateGoalArgs {
        session_id: Some(session_id),
        goal_id,
        revision,
        action: update_action,
        objective,
        blocked_reason,
        max_goal_rounds,
    };
    let res = pony_agent_core::agent::goal::update_goal(args)?;
    serde_json::to_value(res.goal).map_err(|e| e.to_string())
}

// ── Team Control Surface ────────────────────────────────────────────────────────────

#[tauri::command]
fn team_get_roster() -> Result<Value, String> {
    let res = pony_agent_core::agent::tools::list_agents()?;
    serde_json::to_value(res.agents).map_err(|e| e.to_string())
}

#[tauri::command]
fn team_get_tasks() -> Result<Value, String> {
    let res = pony_agent_core::agent::tools::team_task_list()?;
    serde_json::to_value(res.tasks).map_err(|e| e.to_string())
}

#[tauri::command]
fn team_claim_task(task_id: String, expected_revision: u64, owner: String) -> Result<Value, String> {
    let args = pony_agent_core::agent::tools::TeamTaskUpdateArgs {
        task_id,
        expected_revision,
        action: pony_agent_core::agent::tools::TeamTaskAction::Claim,
        owner: Some(owner),
        subject: None,
        description: None,
        write_scopes: None,
        blocked_by: None,
    };
    let res = pony_agent_core::agent::tools::team_task_update(args)?;
    serde_json::to_value(res).map_err(|e| e.to_string())
}

#[tauri::command]
fn team_complete_task(task_id: String, expected_revision: u64) -> Result<Value, String> {
    let args = pony_agent_core::agent::tools::TeamTaskUpdateArgs {
        task_id,
        expected_revision,
        action: pony_agent_core::agent::tools::TeamTaskAction::Complete,
        owner: None,
        subject: None,
        description: None,
        write_scopes: None,
        blocked_by: None,
    };
    let res = pony_agent_core::agent::tools::team_task_update(args)?;
    serde_json::to_value(res).map_err(|e| e.to_string())
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
fn load_app_settings(app: AppHandle) -> AppSettings {
    let settings = AppSettingsStore::new().load_view();
    // D7：启动即按持久化值生效（默认 false ⇒ 零 span 分配、零 egress）。
    pony_agent_core::agent::otel::set_agent_trace_reporting(settings.agent_trace_reporting);
    // Wave-5 K3：成本预算只做软告警（None/<=0 视为未设置），绝不参与任何拦截。
    pony_agent_core::agent::cost::set_budget_usd(settings.cost_budget_usd);
    // Wave-7：用量治理策略同步（None ⇒ 默认策略：enforcement=false，纯软告警）。
    pony_agent_core::agent::usage_governance::set_policy(settings.usage_governance_policy.clone());
    // PonySentry 凭据分发（settings > env > 构建期烧录 > 缺省）：桌面端也可用 settings 覆盖
    // 构建期烧录的 token；无 token ⇒ fail-closed 零 egress，token 不进日志/Debug。
    let ponysentry_settings = settings.ponysentry_settings_snapshot();
    // wave-51 C1（审计 §4 P0-1）：PonySentry `release` 与安装包发布线对齐 —— 以宿主
    // `package_info().version` 显式覆盖 env `APP_RELEASE` / core crate 版本（空白或脏值不覆盖）。
    let app_version = app.package_info().version.to_string();
    let ponysentry_config = pony_agent_core::agent::ponysentry::PonySentryConfig::from_settings(
        Some(&ponysentry_settings),
    )
    .with_release(Some(app_version.as_str()));
    pony_agent_core::agent::ponysentry::init(ponysentry_config);
    settings
}

#[tauri::command]
fn save_app_settings(settings: AppSettings) -> Result<AppSettings, String> {
    let saved = AppSettingsStore::new().save_view(settings)?;
    // D7：保存即生效，无需重启；以落盘后的回显值为准（避免保存失败后门禁被误翻）。
    pony_agent_core::agent::otel::set_agent_trace_reporting(saved.agent_trace_reporting);
    // Wave-5 K3：预算保存即生效（软告警门槛，不阻断请求）。
    pony_agent_core::agent::cost::set_budget_usd(saved.cost_budget_usd);
    // Wave-7：治理策略保存即生效（enforcement 仍需用户显式开启）。
    pony_agent_core::agent::usage_governance::set_policy(saved.usage_governance_policy.clone());
    Ok(saved)
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
fn open_url(url: String) -> Result<(), String> {
    platform::open_url_in_browser(&url)
}

#[tauri::command]
fn ponysentry_capture(mut payload: serde_json::Value) -> Result<(), String> {
    if let Some(obj) = payload.as_object_mut() {
        let tags = obj.entry("tags").or_insert_with(|| serde_json::json!({}));
        if let Some(tags_obj) = tags.as_object_mut() {
            tags_obj.entry("project").or_insert_with(|| serde_json::json!("pony-agent"));
        }
    }
    if let Ok(ingest_payload) = serde_json::from_value::<agent::ponysentry::IngestPayload>(payload.clone()) {
        agent::ponysentry::capture_payload(ingest_payload);
    } else {
        let msg = payload
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("Frontend error");
        let err_type = payload
            .get("exception")
            .and_then(|e| e.get("error_type"))
            .and_then(|t| t.as_str())
            .unwrap_or("FrontendError");
        let extra = payload.get("extra").cloned();
        agent::ponysentry::capture_error(err_type, msg, extra);
    }
    Ok(())
}

/// Wave-7 G4：用量治理汇总（会话维度 + 天桶；空库返回零值结构）。
#[tauri::command]
fn usage_governance_summary(
    control_plane: State<'_, HostControlPlane>,
    session_id: Option<String>,
) -> agent::usage_governance::UsageGovernanceSummaryView {
    control_plane.usage_governance_summary(session_id)
}

#[tauri::command]
fn list_sessions(control_plane: State<'_, HostControlPlane>) -> Vec<SessionOverview> {
    control_plane.list_sessions()
}

/// Trace tab 一键上传：手动上报指定会话的持久化 trace 至 PonySentry。
/// 返回实际上报的 turn 数量（空会话返回 0）。
#[tauri::command]
async fn upload_session_trace(
    control_plane: State<'_, HostControlPlane>,
    session_id: String,
) -> Result<usize, String> {
    control_plane.upload_session_trace(&session_id).await
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

/// 重命名工作区（侧边栏三级树）：仅改显示名，id/root 不变；校验失败返回错误文案。
#[tauri::command]
fn workspace_rename(
    control_plane: State<'_, HostControlPlane>,
    workspace_id: String,
    name: String,
) -> Result<WorkspaceRecordView, String> {
    control_plane
        .rename_workspace(&workspace_id, &name)
        .map(|workspace| WorkspaceRecordView {
            id: workspace.id,
            name: workspace.name,
            root_path: workspace.root_path,
        })
}

/// 删除工作区注册：目录与会话数据不动；名下会话归属重写为 default。
/// default 与未知 id 拒绝。
#[tauri::command]
fn workspace_delete(
    control_plane: State<'_, HostControlPlane>,
    workspace_id: String,
) -> Result<(), String> {
    control_plane.delete_workspace(&workspace_id)
}

/// 会话重命名：写入持久化 override，后续每轮派生刷新不再覆盖。
#[tauri::command]
fn session_rename(
    control_plane: State<'_, HostControlPlane>,
    session_id: String,
    title: String,
) -> Result<(), String> {
    control_plane.rename_session(&session_id, &title)
}

/// 归档会话：分组面全量隐藏（幂等）；日志与数据不动。
#[tauri::command]
fn session_archive(
    control_plane: State<'_, HostControlPlane>,
    session_id: String,
) -> Result<(), String> {
    control_plane.archive_session(&session_id)
}

#[tauri::command]
fn session_unarchive(
    control_plane: State<'_, HostControlPlane>,
    session_id: String,
) -> Result<(), String> {
    control_plane.unarchive_session(&session_id)
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

/// 退出路径的统一 OTel flush 出口（Wave-4 C5 / Wave-4.1 A5）。
///
/// 契约：
/// - 有界：单次 flush 预算 [`EXIT_FLUSH_BUDGET_MS`]（1000ms）；与 `otel::shutdown()`
///   （内部 1000ms 等待档位）合计硬上限 ≤3000ms，满足 NFR
///   `observability.exporter_flush_timeout_ms` 与 C5 验收判据#1；
/// - 幂等：`ExitRequested` 与 `Exit` 共享 [`EXIT_FLUSH_DONE`] 短路，重复事件不再叠加预算；
/// - 未装配（otel 装配失败/非桌面启动路径）时静默跳过；未完成只降级 warn JSON 日志；绝不 panic。
fn flush_otel_on_exit(stage: &str) {
    if pony_agent_core::agent::otel::config().is_none() {
        return;
    }
    if EXIT_FLUSH_DONE.swap(true, Ordering::SeqCst) {
        return;
    }
    if let Err(message) = pony_agent_core::agent::otel::force_flush(EXIT_FLUSH_BUDGET_MS) {
        pony_agent_core::agent::otel::log::log_json(
            "warn",
            &format!("退出 flush（{stage}）未在预算内完成：{message}"),
            None,
        );
    }
}

/// 退出 flush 的单次预算（毫秒）：与 `otel::shutdown()` 的 1000ms 档位合计 ≤3000ms。
const EXIT_FLUSH_BUDGET_MS: u64 = 1_000;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        // PA-103: updater/process plugins are registered so the capability permission IDs
        // (updater:allow-check, updater:allow-download-and-install, process:allow-restart)
        // resolve in this build. The tauri.conf.json updater block stays absent until the
        // release-owner gate provisions a real endpoint and public key; the frontend adapter
        // is disabled (SIGNED_UPDATER_ENABLED=false), so the plugin commands are never invoked.
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(HostControlPlane::new())
        .manage(StreamDebugMetricsState {
            latest: Mutex::new(json!({})),
        })
        .manage(TurnTaskRegistry::with_max_concurrent(3))
        .setup(|app| {
            if let Some(window) = app.get_webview_window("main") {
                platform::apply_icons(app);
                platform::apply_window_style(&window);
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
                // 如需开发时打开 DevTools 调试面板，可解除下行注释或按 F12
                // #[cfg(debug_assertions)]
                // window.open_devtools();
            }
            if let Err(e) = platform::setup_tray(app) {
                eprintln!("[tray-warning] failed to setup tray icon: {e}");
            }

            // Wave43 D2/D3：provider 进程启动时装配一次（开关只翻转 flag，不重建 provider）。
            // 装配失败必须降级为日志：trace 不可用绝不阻塞主业务启动。
            //
            // Wave-4 C4.2：桌面端结构化日志落盘（追加 + 512KiB 轮转），路径由 app_data_dir 解析；
            // 解析/创建失败只影响落盘，不影响 stdout 与主链路。
            if let Ok(app_data_dir) = app.path().app_data_dir() {
                pony_agent_core::agent::otel::log::set_log_file_path(
                    app_data_dir.join("logs").join("pony-agent.log"),
                );
            }

            // Wave-4 C1：配置来源优先级 settings > env > 内置默认（公网端点 https://otel.ponyjob.top）。
            let app_settings = load_app_settings(app.handle().clone());
            let otel_settings = app_settings.otel_settings_snapshot();
            // wave-51 C1（审计 §4 P0-1）：resource `service.version` 取安装包版本（单一来源：
            // 宿主注入 > env `PONY_OTEL_SERVICE_VERSION` > core crate 版本）。
            let app_version = app.package_info().version.to_string();
            if let Err(err) = pony_agent_core::agent::otel::init_with_service_version(
                pony_agent_core::agent::otel::OtelConfig::from_settings(Some(&otel_settings)),
                Some(app_version.as_str()),
            ) {
                // F4：走统一 JSON 日志面（stdout 单行 JSON），不得新增明文 eprintln 边界。
                pony_agent_core::agent::otel::log::log_json(
                    "warn",
                    &format!("otel 装配失败，trace 上报已禁用：{err}"),
                    None,
                );
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            health_check,
            list_sessions,
            upload_session_trace,
            get_workspace_root,
            usage_governance_summary,
            import_attachment,
            workspace_list,
            workspace_create,
            workspace_rename,
            workspace_delete,
            session_rename,
            session_archive,
            session_unarchive,
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
            fail_turn_for_watchdog,
            ask_list_pending,
            ask_answer,
            control_approve_request,
            ask_cancel,
            ask_expire,
            plan_create,
            plan_replace,
            plan_merge,
            plan_complete_step,
            plan_execution_approve,
            plan_list,
            plan_get,
            todo_list,
            goal_get,
            goal_create,
            goal_update,
            team_get_roster,
            team_get_tasks,
            team_claim_task,
            team_complete_task,
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
            open_url,
            ponysentry_capture
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // 当用户点击关闭按钮时，拦截默认的关闭销毁行为，隐藏窗口并保持后台/托盘常驻
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .build(tauri::generate_context!())
        .expect("failed to build Pony Agent")
        .run(|_app_handle, event| {
            // Wave-4 C5：退出拦截——`ExitRequested` 与 `Exit` 两个分支都必须触发
            // **有界** force_flush（硬上限 3000ms，对齐 NFR
            // `observability.exporter_flush_timeout_ms`），保证缓冲区中尚未发出的 span
            // 在进程退出前完成投递；两条路径幂等，重复触发不得 panic / 死锁。
            match event {
                tauri::RunEvent::Exit => {
                    flush_otel_on_exit("exit");
                    // NFR resource_release：flush 后释放 OTel provider 及其导出 runtime（幂等）。
                    pony_agent_core::agent::otel::shutdown();
                }
                // 退出请求（进程可能继续运行，如托盘常驻）同样有界 flush，但不释放 provider。
                tauri::RunEvent::ExitRequested { .. } => flush_otel_on_exit("exit_requested"),
                _ => {}
            }
        });
}
