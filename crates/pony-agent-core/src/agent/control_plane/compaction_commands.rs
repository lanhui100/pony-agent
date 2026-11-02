// compaction_commands: 手动上下文压缩控制面命令（桌面 /compact、TUI /compact 共用入口）。
// 由 control_plane/mod.rs 的 HostControlPlane 巨型 impl 拆分而来，行为与结构保持一致。
use super::*;
use crate::agent::runtime::{compact_session_with_sessions, ManualCompactionOutcome};
use crate::agent::session::DEFAULT_SESSION_ID;

impl HostControlPlane {
    /// 手动压缩会话上下文。
    ///
    /// `session_id == None` 回落默认会话（与 `SessionStore` 既有默认一致）；
    /// 语义见 `AgentRuntime::compact_session`（C-1）。
    ///
    /// **锁纪律（审查 F2a）**：`runtime` 读锁只覆盖 provider 解析（毫秒级），
    /// 摘要网络调用期间既**不持 runtime 读锁**（否则阻塞 `capability_commands.rs` 的
    /// `runtime.write()`），也**不持 sessions 锁**（快照/回写各自用一次短锁）。
    pub fn compact_session(
        &self,
        session_id: Option<&str>,
        force: bool,
    ) -> Result<ManualCompactionOutcome, String> {
        let session_key = session_id.unwrap_or(DEFAULT_SESSION_ID);
        let provider = {
            let runtime = self.runtime.read().unwrap_or_else(|e| {
                eprintln!("[pony-agent] runtime rwlock poisoned: {e}, recovering");
                e.into_inner()
            });
            runtime.compaction_provider()
        };
        let sessions = Arc::clone(&self.sessions_rwlock);
        compact_session_with_sessions(&sessions, provider, session_key, force)
    }
}
