// workspace_commands: Workspace 注册表命令（PA-079）。
// 由 control_plane/mod.rs 的 HostControlPlane impl 拆分，行为与结构保持一致。
use super::*;

impl HostControlPlane {
    pub fn list_workspaces(&self) -> Vec<crate::agent::workspace::WorkspaceRecord> {
        self.sessions_rwlock
            .read()
            .unwrap_or_else(|e| {
                eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
                e.into_inner()
            })
            .list_workspaces()
    }

    pub fn create_workspace(
        &self,
        name: &str,
        root_path: &str,
    ) -> Result<crate::agent::workspace::WorkspaceRecord, String> {
        self.sessions_rwlock
            .write()
            .unwrap_or_else(|e| {
                eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
                e.into_inner()
            })
            .create_workspace(name, root_path)
    }

    pub fn rename_workspace(
        &self,
        workspace_id: &str,
        name: &str,
    ) -> Result<crate::agent::workspace::WorkspaceRecord, String> {
        self.sessions_rwlock
            .write()
            .unwrap_or_else(|e| {
                eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
                e.into_inner()
            })
            .rename_workspace(workspace_id, name)
    }

    pub fn delete_workspace(&self, workspace_id: &str) -> Result<(), String> {
        self.sessions_rwlock
            .write()
            .unwrap_or_else(|e| {
                eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
                e.into_inner()
            })
            .delete_workspace(workspace_id)
    }

    pub fn rename_session(&self, session_id: &str, title: &str) -> Result<(), String> {
        self.sessions_rwlock
            .write()
            .unwrap_or_else(|e| {
                eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
                e.into_inner()
            })
            .rename_session(session_id, title)
    }

    pub fn archive_session(&self, session_id: &str) -> Result<(), String> {
        self.sessions_rwlock
            .write()
            .unwrap_or_else(|e| {
                eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
                e.into_inner()
            })
            .archive_session(session_id)
    }
}
