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

    /// 确保 root 已成 workspace 记录（wave-38 契约 R1.3 宿主入口）。
    /// 语义 = 幂等登记（见 `agent::workspace::ensure_workspace_for_root`）；
    /// 锁纪律与 `create_workspace` 一致：`sessions_rwlock.write()` + poison 恢复。
    pub fn ensure_workspace_for_root(
        &self,
        root: &str,
    ) -> Result<crate::agent::workspace::WorkspaceRecord, String> {
        self.sessions_rwlock
            .write()
            .unwrap_or_else(|e| {
                eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
                e.into_inner()
            })
            .ensure_workspace_for_root(root)
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

    pub fn unarchive_session(&self, session_id: &str) -> Result<(), String> {
        self.sessions_rwlock
            .write()
            .unwrap_or_else(|e| {
                eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
                e.into_inner()
            })
            .unarchive_session(session_id)
    }
}
