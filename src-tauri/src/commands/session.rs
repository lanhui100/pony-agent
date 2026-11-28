//! 会话列表、归档与 trace 上传命令。

use crate::agent::control_plane::HostControlPlane;
use crate::agent::session::SessionOverview;
use crate::blocking_helper::BlockingHelper;
use tauri::AppHandle;
use tauri::Manager;
use tauri::State;

/// 会话列表（热路径：侧栏/启动都读，底层是同步 SQLite 全表读）。
///
/// W56-D：原为同步命令 ⇒ 在主线程内联跑 SQLite，UI 卡顿。现 async +
/// `BlockingHelper::spawn`，SQLite 读落在 blocking 线程池；命令名/参数/成功返回形状不变。
#[tauri::command]
pub(crate) async fn list_sessions(app: AppHandle) -> Result<Vec<SessionOverview>, String> {
    BlockingHelper::spawn(move || app.state::<HostControlPlane>().list_sessions()).await
}

/// Trace tab 一键上传：手动上报指定会话的持久化 trace 至 PonySentry。
///
/// P1-3：返回**结构化回执**（`{ turns, traceIds, failures }`，camelCase），前端据此给出
/// 「全部成功 / 部分成功 / 没有可上传」三态反馈；空会话返回空回执而不是 `0`。
#[tauri::command]
pub(crate) async fn upload_session_trace(
    control_plane: State<'_, HostControlPlane>,
    session_id: String,
) -> Result<pony_agent_core::agent::otel::UploadTraceReceipt, String> {
    control_plane.upload_session_trace(&session_id).await
}

/// 会话重命名：写入持久化 override，后续每轮派生刷新不再覆盖。
#[tauri::command]
pub(crate) fn session_rename(
    control_plane: State<'_, HostControlPlane>,
    session_id: String,
    title: String,
) -> Result<(), String> {
    control_plane.rename_session(&session_id, &title)
}

/// 归档会话：分组面全量隐藏（幂等）；日志与数据不动。
#[tauri::command]
pub(crate) fn session_archive(
    control_plane: State<'_, HostControlPlane>,
    session_id: String,
) -> Result<(), String> {
    control_plane.archive_session(&session_id)
}

#[tauri::command]
pub(crate) fn session_unarchive(
    control_plane: State<'_, HostControlPlane>,
    session_id: String,
) -> Result<(), String> {
    control_plane.unarchive_session(&session_id)
}
