//! B148：`SessionStore` 的**稀疏（懒装载）模式**——启动只装载轻量元数据索引，会话内容按需
//! 「单会话定向装载」。
//!
//! 为什么独立成文件：`session/store.rs` 已计入 `scripts/file-size-baseline.json` 的**文件行数
//! 棘轮**（净值只许减），本模块承载新增状态机，`store.rs` 只留分流调用（模块声明见其顶部
//! `#[path = "store_sparse.rs"] mod sparse;`）。
//!
//! ## 不变量（改动前先读）
//!
//! 1. `lazy.is_none()` ⇒ **全量模式**，行为与 B148 之前逐字段等价（回退路径：桌面/CLI/测试）。
//! 2. `lazy.is_some()` ⇒ `sessions` **只含已装载会话**；元数据在 `lazy.meta`，列表读它。
//!    不得用 `sessions` 投影列表（否则列表只剩已装载的几条）。
//! 3. 写路径只写 `loaded`（[`SessionStore::lazy_commit_sessions`]）：未装载会话的 blob 与
//!    trace 表**零写入**。这是数据安全红线——稀疏快照若整库提交，等价于把未装载会话删库。
//! 4. 未装载会话的**内容读取**必须经 [`SessionStore::hydrate_session`]。只读入口
//!    （`snapshot_at_readonly` / `get_session`）遇到未装载会话时**显式记日志**并返回缺省值，
//!    禁止把「没装载」伪装成「空会话」（fail loud，由 TUI 侧先 hydrate）。
//! 5. 后端不具备定向能力（`supports_targeted_session_load() == false`）⇒ 直接回退
//!    `with_backend` 全量语义（内存/文件后端写路径是整包覆盖，稀疏快照会丢会话）。

use super::*;
use crate::agent::session::{
    SeparateTraceTableMode, SessionBackend, SessionMetaEntry, SessionVersion, TargetedSessionLoad,
};

/// 稀疏模式的内存状态（`SessionStore::lazy` 为 `None` ⇒ 全量模式）。
#[derive(Default)]
pub(super) struct LazySessions {
    /// 轻量元数据索引：列表 / 恢复目标 / 控制面骨架的唯一来源（不含会话内容）。
    pub(super) meta: std::collections::HashMap<String, SessionMetaEntry>,
    /// 本实例**已装载内容**的会话 = 允许写入的集合（未装载会话零写入）。
    pub(super) loaded: std::collections::HashSet<String>,
}

/// 全量模式的默认会话集（B148 从 `store.rs` 迁入，腾出行数棘轮配额）。
///
/// 两种初始集合在这里对齐：全量 = 库为空时兜底 `default_sessions()`；稀疏 = 空集 +
/// 按需定向装载（默认会话在首次 `ensure_session` 时按新会话语义创建）。
pub(super) fn default_sessions() -> SessionMap {
    let mut sessions = HashMap::new();
    sessions.insert(
        DEFAULT_SESSION_ID.to_string(),
        SessionState {
            conversation_id: DEFAULT_SESSION_ID.to_string(),
            title: DEFAULT_SESSION_TITLE.to_string(),
            summary: DEFAULT_SESSION_SUMMARY.to_string(),
            history: Vec::new(),
            provider_native_transcript: Vec::new(),
            turn_trace_history: Vec::new(),
            trace_migration_state: TraceMigrationState::default(),
            turn_trace_refs: None,
            long_term_memory_entries: Vec::new(),
            memory_write_evidence: Vec::new(),
            memory_write_hook_trace_records: Vec::new(),
            history_state_evidence: Vec::new(),
            turn_count: 0,
            last_referenced_file: None,
            updated_at_ms: now_timestamp_ms(),
            history_nodes: Vec::new(),
            history_branches: Vec::new(),
            history_cursor: HistoryCursor::default(),
            workspace_id: None,
            title_override: None,
            archived: false,
            event_watermark: 0,
            last_commit_watermark: 0,
        },
    );
    sessions
}

impl SessionStore {
    /// 生产入口：SQLite 库的**稀疏**装载（TUI 启动 / `pony resume`）。
    ///
    /// 与 [`SessionStore::new`] 的唯一差别是「不装载会话内容」；ask store 仍按同一路径打开
    /// （未决 Ask 与会话同库，见 ADR 0074）。
    pub fn open_lazy() -> Self {
        use crate::agent::sqlite_session::{default_sqlite_path, SqliteSessionBackend};

        let sqlite_path = default_sqlite_path();
        if let Some(parent) = sqlite_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let backend: Box<dyn SessionBackend> = Box::new(SqliteSessionBackend::new_with_trace_mode(
            sqlite_path.clone(),
            SeparateTraceTableMode::WriteSeparate,
        ));
        let mut store = Self::open_lazy_with_backend(backend);
        if store.lazy.is_none() || store.backend.storage_read_only() {
            return store;
        }
        store.ask_store = match crate::agent::ask_persistence::open_ask_store(&sqlite_path) {
            Ok(ask_store) => Some(Arc::from(ask_store)),
            Err(error) => {
                eprintln!(
                    "[pony-agent][session] ask store unavailable at {}: {error}",
                    sqlite_path.display()
                );
                None
            }
        };
        store
    }

    /// 稀疏装载的可注入入口（测试与自定义后端；不支持定向装载即回退全量）。
    pub fn open_lazy_with_backend(backend: Box<dyn SessionBackend>) -> Self {
        if !backend.supports_targeted_session_load() {
            return Self::with_backend(backend);
        }
        let attachment_root = backend
            .attachment_root()
            .unwrap_or_else(default_attachment_root);
        // 全局元数据（附件索引 / workspace / 授权 / 版本）：不含会话内容。
        let persisted = backend.load_store_metadata().unwrap_or_default();
        let meta_entries = backend.load_session_meta_index().unwrap_or_default();
        let mut workspaces = persisted.workspaces;
        if crate::agent::workspace::bootstrap_default_workspace(&mut workspaces) {
            // 与 with_backend 同义：派生修复只留内存，交给首次真实写入落盘。
            eprintln!(
                "[pony-agent][session] startup_repair_deferred: reasons=workspace_bootstrap sessions=0 — 稀疏装载不写盘"
            );
        }
        if let Err(error) = backend.validate_event_schema() {
            eprintln!(
                "[pony-agent][session] EVENT SCHEMA VERSION MISMATCH — event-sourced views unavailable: {error}"
            );
        }
        let mut lazy = LazySessions::default();
        for entry in meta_entries {
            lazy.meta.insert(entry.session_id.clone(), entry);
        }
        let lazy_session_count = lazy.meta.len();
        let store = Self {
            sessions: SessionMap::new(),
            attachment_assets: persisted.attachment_assets,
            session_attachment_index: persisted.session_attachment_index,
            mcp_source_snapshots: persisted.mcp_source_snapshots,
            skill_source_snapshots: persisted.skill_source_snapshots,
            workspaces,
            path_authorizations: Arc::new(
                crate::agent::path_permission::AuthorizeStore::from_entries(
                    persisted.path_authorizations,
                    None,
                ),
            ),
            backend,
            attachment_root,
            memory_write_hook_executor: Arc::new(NoopMemoryWriteHookExecutor),
            history_state_hook_executor: Arc::new(NoopHistoryStateHookExecutor),
            projection_cache: HashMap::new(),
            event_stream_degraded: std::sync::Mutex::new(std::collections::HashSet::new()),
            ask_store: None,
            versions: std::sync::Mutex::new(persisted.versions),
            lazy: Some(Box::new(lazy)),
        };
        eprintln!(
            "[pony-agent][session] sparse_store_ready: meta_sessions={lazy_session_count} loaded_sessions=0 — 会话内容按需定向装载"
        );
        store
    }

    /// 是否处于稀疏（懒装载）模式。
    pub fn is_sparse(&self) -> bool {
        self.lazy.is_some()
    }

    /// 已装载内容的会话 id（验收用：写集合 = 本集合）。
    pub fn loaded_session_ids(&self) -> Vec<String> {
        let mut ids = self
            .lazy
            .as_ref()
            .map(|lazy| lazy.loaded.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        ids.sort();
        ids
    }

    /// 轻量元数据索引快照（验收用：与全量 `list_sessions` 逐 id 比对）。
    pub fn meta_entries(&self) -> Vec<SessionMetaEntry> {
        let mut entries = self
            .lazy
            .as_ref()
            .map(|lazy| lazy.meta.values().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        entries.sort_by(|left, right| left.session_id.cmp(&right.session_id));
        entries
    }

    /// 单会话**定向装载**（幂等）：`true` = 内容已可用；`false` = 库内确实没有该会话。
    ///
    /// `Unsupported`（后端无定向能力）⇒ 升级为全量装载后重查，绝不返回「空会话」。
    pub fn hydrate_session(&mut self, session_id: &str) -> bool {
        if self.lazy.is_none() {
            return self.sessions.contains_key(session_id);
        }
        if self
            .lazy
            .as_ref()
            .is_some_and(|lazy| lazy.loaded.contains(session_id))
        {
            return true;
        }
        match self.backend.load_session_targeted(session_id) {
            TargetedSessionLoad::Loaded(state) => {
                let mut state = *state;
                // 与 with_backend 的单会话启动修复同源（全是派生量，只改内存不写盘）。
                crate::agent::session::startup_repair::refresh_metadata_if_content_present(
                    &mut state,
                );
                ensure_history_graph(&mut state);
                if state.updated_at_ms == 0 {
                    state.updated_at_ms = now_timestamp_ms();
                }
                sanitize_attachment_references(&mut state);
                backfill_attachment_reference_assets(&mut state);
                let version = self
                    .lazy
                    .as_ref()
                    .and_then(|lazy| lazy.meta.get(session_id))
                    .and_then(|entry| entry.version);
                if let Some(version) = version {
                    if let Ok(mut known) = self.versions.lock() {
                        known.insert(session_id.to_string(), SessionVersion(version));
                    }
                }
                if let Some(lazy) = self.lazy.as_mut() {
                    lazy.loaded.insert(session_id.to_string());
                }
                self.sessions.insert(session_id.to_string(), state);
                eprintln!("[pony-agent][session] targeted_session_hydrated: session={session_id}");
                true
            }
            TargetedSessionLoad::NotFound => {
                // 索引里有、内容层没有：不是可继续的会话，从元数据索引摘除避免列表幽灵。
                if let Some(lazy) = self.lazy.as_mut() {
                    lazy.meta.remove(session_id);
                }
                eprintln!(
                    "[pony-agent][session] targeted_session_not_found: session={session_id} — 视同不存在（不创建空会话）"
                );
                false
            }
            TargetedSessionLoad::Unsupported => {
                eprintln!(
                    "[pony-agent][session] targeted_load_unsupported: session={session_id} — 回退全量装载"
                );
                self.load_all_for_administration() && self.sessions.contains_key(session_id)
            }
        }
    }

    /// 显式管理操作（删工作区 / 归档批量 / 导入导出）需要全量视图时的升级入口。
    ///
    /// 语义等价于全量装载：一次性把全部会话读入内存并做启动期派生修复（不写盘），
    /// 随后本实例退回全量模式。
    pub fn load_all_for_administration(&mut self) -> bool {
        if self.lazy.is_none() {
            return true;
        }
        let Some(mut persisted) = self.backend.load_store() else {
            eprintln!("[pony-agent][session] load_all_for_admin_failed：会话库装载失败（保持稀疏模式）");
            return false;
        };
        let mut sessions = if persisted.sessions.is_empty() {
            default_sessions()
        } else {
            std::mem::take(&mut persisted.sessions)
        };
        for session in sessions.values_mut() {
            crate::agent::session::startup_repair::refresh_metadata_if_content_present(session);
            ensure_history_graph(session);
            if session.updated_at_ms == 0 {
                session.updated_at_ms = now_timestamp_ms();
            }
            sanitize_attachment_references(session);
            backfill_attachment_reference_assets(session);
        }
        self.sessions = sessions;
        self.attachment_assets = persisted.attachment_assets;
        self.session_attachment_index = persisted.session_attachment_index;
        self.mcp_source_snapshots = persisted.mcp_source_snapshots;
        self.skill_source_snapshots = persisted.skill_source_snapshots;
        self.workspaces = persisted.workspaces;
        self.path_authorizations = Arc::new(
            crate::agent::path_permission::AuthorizeStore::from_entries(
                persisted.path_authorizations,
                None,
            ),
        );
        if let Ok(mut known) = self.versions.lock() {
            *known = persisted.versions;
        }
        self.lazy = None;
        eprintln!(
            "[pony-agent][session] sparse_store_escalated: sessions={} — 已升级为全量模式",
            self.sessions.len()
        );
        true
    }

    /// 稀疏模式下的列表：元数据索引 + 已装载会话的内存投影（新会话/改名/归档即时可见）。
    pub(super) fn lazy_list_sessions(&self) -> Option<Vec<SessionOverview>> {
        let lazy = self.lazy.as_ref()?;
        let mut sessions = SessionMap::new();
        for session_id in lazy.meta.keys() {
            if let Some(state) = self.sessions.get(session_id) {
                sessions.insert(session_id.clone(), state.clone());
            }
        }
        for (session_id, state) in &self.sessions {
            if !lazy.meta.contains_key(session_id) {
                sessions.insert(session_id.clone(), state.clone());
            }
        }
        let mut list = sessions
            .values()
            .filter(|session| session_is_persistable(session))
            .map(|session| SessionOverview {
                conversation_id: session.conversation_id.clone(),
                title: session.effective_title().to_string(),
                summary: session.summary.clone(),
                turn_count: session.turn_count,
                last_referenced_file: session.last_referenced_file.clone(),
                updated_at_ms: session.updated_at_ms,
                workspace_id: session.workspace_id.clone(),
                archived: session.archived,
                continuation_only: session_is_continuation_fragment(session),
            })
            .collect::<Vec<_>>();
        for (session_id, entry) in &lazy.meta {
            if self.sessions.contains_key(session_id) || !entry.has_content {
                continue;
            }
            list.push(SessionOverview {
                conversation_id: entry.session_id.clone(),
                title: entry.title.clone(),
                summary: entry.summary.clone(),
                turn_count: entry.turn_count,
                last_referenced_file: entry.last_referenced_file.clone(),
                updated_at_ms: entry.updated_at_ms,
                workspace_id: entry.workspace_id.clone(),
                archived: entry.archived,
                // 元数据不含历史正文，续轮片段判据需要正文 ⇒ **保守取 false**：
                // 宁可把碎片显示出来，也不静默隐藏用户会话（与「不静默空快照」同向）。
                continuation_only: false,
            });
        }
        list.sort_by(|left, right| {
            right
                .updated_at_ms
                .cmp(&left.updated_at_ms)
                .then_with(|| left.conversation_id.cmp(&right.conversation_id))
        });
        Some(list)
    }

    /// 稀疏提交集合：**只含已装载会话**（未装载会话的 blob/trace 零写入）。
    pub(super) fn lazy_commit_sessions(&self) -> Option<SessionMap> {
        let lazy = self.lazy.as_ref()?;
        let mut sessions = SessionMap::new();
        for session_id in &lazy.loaded {
            if let Some(state) = self.sessions.get(session_id) {
                sessions.insert(session_id.clone(), state.clone());
            }
        }
        Some(sessions)
    }

    /// 该会话「索引里有、内容没装载」⇒ 任何内容读取都不得视为空会话。
    pub(super) fn lazy_blocks_read(&self, session_id: &str) -> bool {
        self.lazy.as_ref().is_some_and(|lazy| {
            lazy.meta.contains_key(session_id) && !lazy.loaded.contains(session_id)
        })
    }

    /// 未装载读取的显式拒绝快照（空历史 + 缺省元数据），配合 `session_read_unloaded_denied` 日志。
    pub(super) fn lazy_placeholder_snapshot(
        &self,
        session_id: &str,
        node_id: Option<&str>,
    ) -> SessionSnapshot {
        eprintln!(
            "[pony-agent][session] session_read_unloaded_denied: session={session_id} reader=snapshot_at_readonly — 调用方必须先 hydrate_session（返回缺省快照，不是空会话）"
        );
        default_snapshot_for_session(session_id, node_id, self.workspace_env_cwd(None))
    }

    pub(super) fn lazy_has_session(&self, session_id: &str) -> bool {
        self.lazy
            .as_ref()
            .is_some_and(|lazy| lazy.meta.contains_key(session_id))
    }

    /// 未装载读取（`get_session` 等只读入口）的显式拒绝日志。
    pub(super) fn warn_unloaded_read(&self, session_id: &str, reader: &str) {
        eprintln!(
            "[pony-agent][session] session_read_unloaded_denied: session={session_id} reader={reader} — 返回 None（未装载 ≠ 不存在）"
        );
    }

    /// 元数据索引里的 workspace 归属兜底（未装载会话也能回答归属，不返回伪 None）。
    pub(super) fn lazy_meta_workspace_owner(
        &self,
        session_id: &str,
    ) -> Option<Option<String>> {
        self.lazy
            .as_ref()
            .and_then(|lazy| lazy.meta.get(session_id))
            .map(|entry| entry.workspace_id.clone())
    }

    /// 会话写入前的装载守卫（写路径必须拿到完整内容，禁止用空壳覆盖 blob）。
    pub(super) fn ensure_session_hydrated(&mut self, session_id: &str) {
        if self.lazy.is_none() {
            return;
        }
        if self
            .lazy
            .as_ref()
            .is_some_and(|lazy| lazy.loaded.contains(session_id))
        {
            return;
        }
        let _ = self.hydrate_session(session_id);
    }

    /// 稀疏模式：**只重算已装载会话**的附件索引条目，未装载会话条目原样保留。
    ///
    /// 返回 `true` = 已按增量处理（调用方不得再做整表重建）。
    /// 为什么必须增量：`refresh_attachment_catalog` 的整表重建只看得见已装载会话，
    /// 一次稀疏写入就会把其余会话的索引条目清空；而附件清理以索引判孤儿 ⇒ 真删数据。
    pub(super) fn refresh_loaded_attachment_entries(&mut self) -> bool {
        let Some(lazy) = self.lazy.as_ref() else {
            return false;
        };
        let loaded = lazy.loaded.clone();
        for session_id in loaded {
            let entry = self
                .sessions
                .get(&session_id)
                .map(session_attachment_entry)
                .unwrap_or_default();
            if entry.is_empty() {
                self.session_attachment_index.remove(&session_id);
            } else {
                self.session_attachment_index.insert(session_id, entry);
            }
        }
        true
    }

    /// 删除/归档后的元数据索引清理（避免稀疏列表幽灵行）。
    pub(super) fn forget_lazy_session(&mut self, session_id: &str) {
        if let Some(lazy) = self.lazy.as_mut() {
            lazy.meta.remove(session_id);
            lazy.loaded.remove(session_id);
        }
    }

    /// 管理工作区等全量语义操作前的升级守卫。
    pub(super) fn ensure_all_loaded_for_admin(&mut self) {
        if self.lazy.is_some() {
            self.load_all_for_administration();
        }
    }
}
