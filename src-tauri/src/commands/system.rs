//! 系统面命令（外部链接、PonySentry、用量治理汇总）。

use crate::agent;
use crate::agent::control_plane::HostControlPlane;
use crate::platform;
use tauri::State;

#[tauri::command]
pub(crate) fn open_url(url: String) -> Result<(), String> {
    platform::open_url_in_browser(&url)
}

#[tauri::command]
pub(crate) fn ponysentry_capture(mut payload: serde_json::Value) -> Result<(), String> {
    if let Some(obj) = payload.as_object_mut() {
        let tags = obj.entry("tags").or_insert_with(|| serde_json::json!({}));
        if let Some(tags_obj) = tags.as_object_mut() {
            tags_obj
                .entry("project")
                .or_insert_with(|| serde_json::json!("pony-agent"));
        }
    }
    if let Ok(ingest_payload) =
        serde_json::from_value::<agent::ponysentry::IngestPayload>(payload.clone())
    {
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
pub(crate) fn usage_governance_summary(
    control_plane: State<'_, HostControlPlane>,
    session_id: Option<String>,
) -> agent::usage_governance::UsageGovernanceSummaryView {
    control_plane.usage_governance_summary(session_id)
}
