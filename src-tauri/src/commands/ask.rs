//! Ask（Interaction）与 Approval 控制面命令；会话作用域守卫亦在此。

use crate::agent;
use crate::agent::control_plane::HostControlPlane;
use crate::goal_resume;
use serde_json::Value;
use tauri::AppHandle;
use tauri::State;

/// Normalized session scope: trimmed, empty ⇒ `None` (host default = all sessions).
pub(crate) fn session_scope(session_id: Option<&str>) -> Option<&str> {
    session_id.map(str::trim).filter(|scope| !scope.is_empty())
}

/// Resolve the session scope a mutating Ask call must carry.
///
/// ADR 0074 / contract §4: the persisted CAS is a scoped four-tuple
/// `(session, request, version, nonce)`. The core `*_ask_in_session` entries are the only mutation
/// path — the three-argument variants fail closed with `ask_scope_required`. A missing scope is
/// therefore rejected here with the same stable code instead of being guessed (a wrong guess would
/// mutate another conversation's request).
pub(crate) fn require_ask_scope(session_id: Option<&str>) -> Result<&str, String> {
    session_scope(session_id).ok_or_else(|| {
        "ask_scope_required: the calling UI must pass the session scope it is looking at"
            .to_string()
    })
}

/// Snapshot of every pending Ask (`Interaction`) request, optionally filtered by session.
#[tauri::command]
pub(crate) fn ask_list_pending(
    control_plane: State<'_, HostControlPlane>,
    session_id: Option<String>,
) -> Value {
    control_plane.list_pending_asks(session_id.as_deref())
}

/// Read the durable lifecycle state of a single Ask (`pending` / `answered` / `cancelled` /
/// `superseded` / `orphaned`).
///
/// ADR 0074 / DSH alignment: an Ask never expires by wall clock, so the UI must not infer
/// "expired" from the record leaving the pending snapshot (that snapshot only lists `pending`).
/// `Err` = the truth could not be read (IO / missing scope): the card must stay open instead of
/// being silently dismissed.
#[tauri::command]
pub(crate) fn ask_lifecycle_state(
    control_plane: State<'_, HostControlPlane>,
    request_id: String,
    session_id: Option<String>,
) -> Result<Option<agent::ask_persistence::AskLifecycleState>, String> {
    let scope = require_ask_scope(session_id.as_deref())?;
    control_plane.get_ask_lifecycle_state(scope, &request_id)
}

/// Answer a pending Ask by stable `request_id` and compare-and-swap `version` (or `expected_version`),
/// carrying the user's answer payload.
///
/// `session_id` is the conversation the caller is looking at (ADR 0074 / contract §4). The persisted
/// CAS is session-scoped: a mutation without scope fails closed with `ask_scope_required`, because
/// `request_id` alone cannot prove both sides are talking about the same conversation. The desktop
/// store always sends it (`openSession` scope). A call without it is rejected with the stable
/// `ask_scope_required` code (mapped to the shared Chinese notice by `isStaleAskError`).
#[tauri::command]
pub(crate) fn ask_answer(
    control_plane: State<'_, HostControlPlane>,
    request_id: String,
    version: Option<u64>,
    expected_version: Option<u64>,
    session_id: Option<String>,
    answer: Value,
) -> Result<Value, String> {
    let ver = expected_version
        .or(version)
        .ok_or_else(|| "missing required key `expected_version` or `version`".to_string())?;
    let scope = require_ask_scope(session_id.as_deref())?;
    control_plane.answer_ask_in_session(scope, &request_id, ver, answer)
}

/// Approve an Approval control request by stable `request_id` and compare-and-swap `version` (or `expected_version`).
#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri 命令入参由 IPC 反序列化注入（AppHandle/State 必须按值接收），函数内只借用"
)]
#[tauri::command]
pub(crate) fn control_approve_request(
    app: AppHandle,
    control_plane: State<'_, HostControlPlane>,
    authorization_modes: State<'_, goal_resume::SessionAuthorizationModes>,
    request_id: String,
    version: Option<u64>,
    expected_version: Option<u64>,
) -> Result<Value, String> {
    let ver = expected_version
        .or(version)
        .ok_or_else(|| "missing required key `expected_version` or `version`".to_string())?;
    let approved = control_plane.approve_control_request(&request_id, ver)?;
    // O1-B：审批决策同样是「宿主作答」——决策后必须把 goal 续轮重新接上（与 Ask 同构）。
    // run 身份来自审批投影的 `runId`（缺失 ⇒ 该审批不绑定 run，无需续跑）。
    if let Some(run_id) = approved
        .get("runId")
        .and_then(Value::as_str)
        .map(str::to_string)
    {
        if let Err(error) = goal_resume::resume_goal_rounds_after_host_answer(
            &app,
            &control_plane,
            &run_id,
            &authorization_modes,
        ) {
            eprintln!("[pony-agent] goal continuation after approval failed: {error}");
        }
    }
    Ok(approved)
}

/// Cancel a pending Ask by stable `request_id` and compare-and-swap `version` (or `expected_version`).
///
/// Same session-scope rule as [`ask_answer`]: the scope the UI is looking at must own the request.
#[tauri::command]
pub(crate) fn ask_cancel(
    control_plane: State<'_, HostControlPlane>,
    request_id: String,
    version: Option<u64>,
    expected_version: Option<u64>,
    session_id: Option<String>,
) -> Result<Value, String> {
    let ver = expected_version
        .or(version)
        .ok_or_else(|| "missing required key `expected_version` or `version`".to_string())?;
    let scope = require_ask_scope(session_id.as_deref())?;
    control_plane.cancel_ask_in_session(scope, &request_id, ver)
}

/// Transition every expired pending request to `Expired`. `now_ms` defaults to the dispatcher
/// clock when omitted.
#[tauri::command]
pub(crate) fn ask_expire(control_plane: State<'_, HostControlPlane>, now_ms: Option<u64>) -> Value {
    control_plane.expire_asks(now_ms)
}
