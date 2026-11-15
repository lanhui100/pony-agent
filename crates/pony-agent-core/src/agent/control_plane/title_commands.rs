// title_commands: AI 会话标题生成控制面命令（桌面 / TUI 共用入口）。
// 由 control_plane/mod.rs 的 HostControlPlane 巨型 impl 拆分而来，行为与结构保持一致。
use super::*;
use crate::agent::runtime::{
    generate_session_title_with_sessions, SessionTitleOutcome, TITLE_NO_USER_MESSAGE_ERROR_CODE,
};
use crate::agent::session::DEFAULT_SESSION_ID;

impl HostControlPlane {
    /// 解析默认 provider（与生产 turn / 压缩同口径）。
    ///
    /// **锁纪律**：`runtime` 读锁只覆盖 provider 解析（毫秒级），网络调用期间不持任何锁。
    fn title_provider(&self) -> crate::agent::provider::ProviderManager {
        let runtime = self.runtime.read().unwrap_or_else(|e| {
            tracing::warn!(target: "control_plane", error = %e, "[pony-agent] runtime rwlock poisoned");
            e.into_inner()
        });
        runtime.compaction_provider()
    }

    /// 生成并写入当前会话的 AI 标题（手动 `/title` / 桌面按钮）。
    ///
    /// `session_id == None` 回落默认会话。语义见 `generate_session_title_with_sessions`。
    pub fn generate_session_title(
        &self,
        session_id: Option<&str>,
    ) -> Result<SessionTitleOutcome, String> {
        let session_key = session_id.unwrap_or(DEFAULT_SESSION_ID);
        let provider = self.title_provider();
        let sessions = Arc::clone(&self.sessions_rwlock);
        generate_session_title_with_sessions(&sessions, provider, session_key)
    }

    /// 首轮后**自动**生成标题的准入入口：仅当会话已有真实用户轮、且标题来源仍为「派生」
    /// （未被用户改名 / 未生成过 AI 标题）时才发起；否则返回 `None`（no-op）。
    ///
    /// 自动路径吞掉非致命错误（网络抖动不应打断会话），但把失败原因打到 stderr 取证面。
    pub fn maybe_generate_session_title(
        &self,
        session_id: Option<&str>,
    ) -> Option<SessionTitleOutcome> {
        let session_key = session_id.unwrap_or(DEFAULT_SESSION_ID);
        let needed = {
            let mut store = self.sessions_rwlock.write().unwrap_or_else(|e| {
                tracing::warn!(target: "control_plane", error = %e, "[pony-agent] sessions rwlock poisoned");
                e.into_inner()
            });
            store.needs_ai_title(session_key)
        };
        if !needed {
            return None;
        }
        let provider = self.title_provider();
        let sessions = Arc::clone(&self.sessions_rwlock);
        match generate_session_title_with_sessions(&sessions, provider, session_key) {
            Ok(outcome) => Some(outcome),
            Err(error) if error.starts_with(TITLE_NO_USER_MESSAGE_ERROR_CODE) => None,
            Err(error) => {
                eprintln!(
                    "[pony-agent][session] auto_title_generation_failed: session={session_key} error={error}"
                );
                None
            }
        }
    }
}
