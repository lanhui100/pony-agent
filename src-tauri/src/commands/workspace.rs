//! 工作区注册、附件导入与路径授权命令。

use crate::agent::control_plane::HostControlPlane;
use tauri::State;

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportAttachmentResultView {
    path: String,
    relative_path: Option<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceRecordView {
    id: String,
    name: String,
    root_path: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AuthorizedPathEntryView {
    path: String,
    granted_at_ms: u64,
}

#[tauri::command]
pub(crate) fn get_workspace_root(control_plane: State<'_, HostControlPlane>) -> String {
    control_plane.get_workspace_root()
}

#[tauri::command]
pub(crate) fn import_attachment(
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

#[tauri::command]
pub(crate) fn workspace_list(
    control_plane: State<'_, HostControlPlane>,
) -> Vec<WorkspaceRecordView> {
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
pub(crate) fn workspace_create(
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
pub(crate) fn workspace_rename(
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
pub(crate) fn workspace_delete(
    control_plane: State<'_, HostControlPlane>,
    workspace_id: String,
) -> Result<(), String> {
    control_plane.delete_workspace(&workspace_id)
}

#[tauri::command]
pub(crate) fn authorize_path(
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
pub(crate) fn revoke_authorization(
    control_plane: State<'_, HostControlPlane>,
    path: String,
) -> Result<bool, String> {
    control_plane.revoke_authorization(&path)
}

#[tauri::command]
pub(crate) fn list_authorizations(
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
