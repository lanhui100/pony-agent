//! 流式调试指标与前端诊断追踪命令。

use crate::agent::control_plane::HostControlPlane;
use crate::agent::frontend_diagnostics::{
    FrontendStallSnapshot, FrontendTraceAppendCommand, FrontendTraceEvent,
    FrontendTraceExportPayload, FrontendTraceQuery, FrontendTraceQueryResult,
};
use crate::blocking_helper::BlockingHelper;
use serde_json::Value;
use std::sync::Mutex;
use tauri::AppHandle;
use tauri::Manager;
use tauri::State;

#[derive(Default)]
pub(crate) struct StreamDebugMetricsState {
    latest: Mutex<Value>,
}

impl StreamDebugMetricsState {
    /// 空快照装配（`run()` 里 `.manage(...)` 用；`{}` 而非 `null`，前端按键读段）。
    pub(crate) fn new() -> Self {
        Self {
            latest: Mutex::new(serde_json::json!({})),
        }
    }
}

#[tauri::command]
pub(crate) fn record_stream_debug_metrics(
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
pub(crate) fn load_stream_debug_metrics(
    state: State<'_, StreamDebugMetricsState>,
) -> Result<Value, String> {
    let latest = state
        .latest
        .lock()
        .map_err(|_| "stream debug lock poisoned".to_string())?;
    Ok(latest.clone())
}

#[tauri::command]
pub(crate) async fn append_frontend_trace_events(
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
pub(crate) async fn query_frontend_trace_window(
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
pub(crate) async fn query_frontend_stall_snapshots(
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
pub(crate) async fn clear_frontend_trace_before(
    app: AppHandle,
    ts_wall_ms: i64,
) -> Result<(), String> {
    BlockingHelper::spawn(move || {
        let control_plane = app.state::<HostControlPlane>();
        control_plane.clear_frontend_trace_before(ts_wall_ms)
    })
    .await?
}

#[tauri::command]
pub(crate) async fn export_frontend_trace_json(
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
pub(crate) async fn export_frontend_trace_chrome_trace(
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
