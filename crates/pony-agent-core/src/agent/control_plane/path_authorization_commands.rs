// path_authorization_commands: 路径读授权命令（PA-080）。
// 由 control_plane/mod.rs 的 HostControlPlane impl 拆分，行为与结构保持一致。
// 授权清单与工具执行器共享同一 `Arc<AuthorizeStore>`（经 SessionStore.path_authorizations），
// 变更即时对工具判定生效；持久化由 SessionStore 完成（store_metadata key=path_authorizations.v1）。
use super::*;
use crate::agent::path_permission::AuthorizedPathEntry;

impl HostControlPlane {
    /// 显式授权一个路径。`scope` 接受 `"read"` 或 `"read-write"`（或 `"write"`）。
    /// 目标必须存在（canonicalize 成功）才可授权。
    pub fn authorize_path(&self, path: &str, scope: &str) -> Result<AuthorizedPathEntry, String> {
        // ADR 0029：read / read-write 作用域必须被**持久化并在判定时生效**，
        // 不能只校验字符串后忽略（否则只读授权会静默升级为可写）。
        let normalized_scope = crate::agent::path_permission::PathAuthorizationScope::parse(scope)?;
        let trimmed = path.trim();
        if trimmed.is_empty() {
            return Err("authorize_path 的 path 不能为空。".to_string());
        }
        let canonical = std::fs::canonicalize(trimmed)
            .map_err(|error| format!("无法解析授权路径 {}：{error}。", trimmed))?;
        let canonical = crate::agent::path_permission::normalize_canonical(&canonical);
        self.sessions_rwlock
            .write()
            .unwrap_or_else(|e| {
                eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
                e.into_inner()
            })
            .authorize_path(canonical, normalized_scope)
    }

    /// 撤销授权（精确路径；子授权保留）。返回是否实际移除。
    pub fn revoke_authorization(&self, path: &str) -> Result<bool, String> {
        let trimmed = path.trim();
        if trimmed.is_empty() {
            return Err("revoke_authorization 的 path 不能为空。".to_string());
        }
        let canonical = std::fs::canonicalize(trimmed)
            .map_err(|error| format!("无法解析撤销路径 {}：{error}。", trimmed))?;
        let canonical = crate::agent::path_permission::normalize_canonical(&canonical);
        Ok(self
            .sessions_rwlock
            .write()
            .unwrap_or_else(|e| {
                eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
                e.into_inner()
            })
            .revoke_authorization(&canonical))
    }

    /// 列出全部授权条目（审计面）。
    pub fn list_authorizations(&self) -> Vec<AuthorizedPathEntry> {
        self.sessions_rwlock
            .read()
            .unwrap_or_else(|e| {
                eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
                e.into_inner()
            })
            .list_authorizations()
    }
}