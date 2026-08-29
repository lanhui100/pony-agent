use super::backend::{
    PersistCommand, PersistCommandOutcome, SeparateTraceTableMode, SessionBackend,
    SessionBackendMutationResult, SessionMetaPatch, SessionTraceMutation, TraceMigrationState,
    TraceTerminalPatch,
};
#[cfg(test)]
use super::file_backend::MemorySessionBackend;
use super::types::{
    collect_env_info, AttachmentAsset, AttachmentAssetMap, AttachmentAssetQuery,
    AttachmentCleanupRequest, AttachmentCleanupResult, AttachmentLifecycleStatus, HistoryBranch,
    HistoryCheckoutMode, HistoryCheckoutStatus, HistoryCursor, HistoryCursorMode, HistoryNode,
    HistoryNodeKind, HistoryStateAuditActionSummary, HistoryStateAuditCurrentContext,
    HistoryStateAuditSummary, LongTermMemoryRecord, MessageStatus, RunControlAuditActionSummary,
    RunControlAuditCurrentContext, RunControlAuditSummary, SessionAttachment,
    SessionAttachmentIndex, SessionError, SessionMap, SessionOverview, SessionSnapshot, SessionState,
    TurnHistoryMessage, TurnTraceRecord, TurnTraceRef, WorkspaceRef,
    DEFAULT_ATTACHMENT_RECLAIM_TTL_MS, DEFAULT_HISTORY_BRANCH_ID, DEFAULT_HISTORY_LIMIT,
    DEFAULT_SESSION_ID, DEFAULT_SESSION_SUMMARY, DEFAULT_SESSION_TITLE, TITLE_MAX_CHARS,
};
use crate::agent::capability_bridge::{McpSourceSnapshot, SkillSourceSnapshot};
use crate::agent::hooks::{
    merge_patch_results, HistoryStateCommandKind, HistoryStateCursorSummary,
    HistoryStateHookEnvelope, HistoryStateHookEvidence, HistoryStateHookExecutor,
    HistoryStateHookPoint, HookPatchConflictPolicy, HookPatchOperationKind, HookPatchTarget,
    HookResultKind, HookStructuredResult, HookTraceRecord, MemoryWriteHookEnvelope,
    MemoryWriteHookExecutor, MemoryWriteHookPoint, MemoryWriteIntentRecord, MemoryWriteOperation,
    MemoryWriteTarget, NoopHistoryStateHookExecutor, NoopMemoryWriteHookExecutor,
    PersistedEffectEvidence,
};
use crate::agent::input::TurnInputImage;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// PA-095 #6：cursor_version 退役——乐观锁改水位比较。seq 水位本身单调递增
/// （checkout/fork/restore/switch 发射 history-control 事件使日志水位只增不减，
/// 消除 ABA 窗口），cursor_version 字段保留 wire 兼容但值恒等于水位镜像。
/// 权威比较源是 session.event_watermark（cursor 字段仅在命令结算时镜像）。
fn reject_stale_cursor_version(
    session: &SessionState,
    expected_cursor_version: Option<u64>,
) -> Result<(), String> {
    let Some(expected) = expected_cursor_version else {
        return Ok(());
    };

    if expected == session.event_watermark {
        return Ok(());
    }

    Err(format!(
        "history cursor conflict: expected watermark {}, actual watermark {}",
        expected, session.event_watermark
    ))
}

pub struct SessionStore {
    pub(super) sessions: SessionMap,
    pub(super) attachment_assets: AttachmentAssetMap,
    pub(super) session_attachment_index: SessionAttachmentIndex,
    mcp_source_snapshots: HashMap<String, McpSourceSnapshot>,
    skill_source_snapshots: HashMap<String, SkillSourceSnapshot>,
    /// Workspace 注册表（PA-079）。
    workspaces: Vec<crate::agent::workspace::WorkspaceRecord>,
    /// 路径读授权清单（PA-080）：内存 `AuthorizeStore`，持久化经 PersistedStore。
    path_authorizations: Arc<crate::agent::path_permission::AuthorizeStore>,
    backend: Box<dyn SessionBackend>,
    pub(super) attachment_root: PathBuf,
    memory_write_hook_executor: Arc<dyn MemoryWriteHookExecutor>,
    history_state_hook_executor: Arc<dyn HistoryStateHookExecutor>,
    /// PA-093：节点 → 投影状态缓存 `(end_seq, history, trace)`。节点 commit 后其
    /// 事件区间不再追加 → 缓存天然有效（无需失效协议）；重启后由重折叠重建。
    /// 内存态，不参与持久化。
    projection_cache: HashMap<String, (u64, Vec<TurnHistoryMessage>, Vec<TurnTraceRecord>)>,
    /// PA-095：事件流 degraded 会话集合——`load_turn_events_checked` 遇坏 payload /
    /// 版本不匹配时标记（fail loud 不静默跳过）；内存态，重启后随首次读取重建。
    event_stream_degraded: std::sync::Mutex<std::collections::HashSet<String>>,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistedStore {
    pub(crate) sessions: SessionMap,
    #[serde(default)]
    pub(crate) attachment_assets: AttachmentAssetMap,
    #[serde(default)]
    pub(crate) session_attachment_index: SessionAttachmentIndex,
    #[serde(default)]
    pub(crate) mcp_source_snapshots: HashMap<String, McpSourceSnapshot>,
    #[serde(default)]
    pub(crate) skill_source_snapshots: HashMap<String, SkillSourceSnapshot>,
    /// Workspace 注册表（PA-079），随 full-store 持久化（SQLite store_metadata key=workspaces）。
    #[serde(default)]
    pub(crate) workspaces: Vec<crate::agent::workspace::WorkspaceRecord>,
    /// 路径读授权清单（PA-080），随 full-store 持久化（SQLite store_metadata
    /// key=`path_authorizations.v1`）；重启后生效。
    #[serde(default)]
    pub(crate) path_authorizations: Vec<crate::agent::path_permission::AuthorizedPathEntry>,
}
/// PA-095 #2：从事件流物化"最近一个 turn"的消息文本（append_turn 事件源）。
/// 返回 `(Option<user 文本>, Option<(assistant 文本, reasoning)>)`；外层 None =
/// 事件流无 turn 归属事件（直连 store 的 legacy 用法，调用方整体回退）；
/// 内层 None = 该 turn 缺对应事件（如 failed turn 仅 user/message——assistant
/// 按调用方参数补占位，与 append_failed_turn 语义对齐）。事件按 seq 升序，
/// 最近 turn 取最后一个带归属的事件之 turn_id。
pub(super) fn materialize_last_turn_messages(
    events: &[(u64, String, crate::agent::turn_event::TurnEvent)],
    min_watermark: u64,
) -> Option<(Option<String>, Option<(String, Option<String>)>)> {
    use crate::agent::turn_event::TurnEvent;
    let fresh_events: Vec<&(u64, String, TurnEvent)> = events
        .iter()
        .filter(|(seq, _, _)| *seq >= min_watermark)
        .collect();
    if fresh_events.is_empty() {
        return None;
    }
    let last_turn_id = fresh_events
        .iter()
        .rev()
        .find_map(|(_, _, event)| event.turn_id().map(str::to_string))?;
    let mut user_text: Option<String> = None;
    let mut assistant: Option<(String, Option<String>)> = None;
    for (_, _, event) in fresh_events {
        if event.turn_id() != Some(last_turn_id.as_str()) {
            continue;
        }
        match event {
            TurnEvent::UserMessage { text, .. } => {
                user_text = Some(text.clone());
            }
            TurnEvent::AssistantMessage {
                text,
                reasoning_content,
                ..
            } => {
                assistant = Some((text.clone(), reasoning_content.clone()));
            }
            _ => {}
        }
    }
    Some((user_text, assistant))
}
impl SessionStore {
    pub fn new() -> Self {
        use crate::agent::sqlite_session::{default_sqlite_path, SqliteSessionBackend};

        let sqlite_path = default_sqlite_path();
        // Ensure the parent directory exists for SQLite
        if let Some(parent) = sqlite_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let backend: Box<dyn SessionBackend> = Box::new(SqliteSessionBackend::new_with_trace_mode(
            sqlite_path,
            SeparateTraceTableMode::WriteSeparate,
        ));
        Self::with_backend(backend)
    }

    #[cfg(test)]
    pub fn memory_only() -> Self {
        Self::with_backend(Box::new(MemorySessionBackend {
            attachment_root: unique_test_session_dir("pony-agent-memory-attachments"),
        }))
    }

    pub fn with_backend(backend: Box<dyn SessionBackend>) -> Self {
        let attachment_root = backend
            .attachment_root()
            .unwrap_or_else(default_attachment_root);
        let persisted = backend.load_store().unwrap_or_default();
        let mut sessions = if persisted.sessions.is_empty() {
            default_sessions()
        } else {
            persisted.sessions
        };
        let mut attachment_assets = persisted.attachment_assets;
        let mut session_attachment_index = persisted.session_attachment_index;
        let mcp_source_snapshots = persisted.mcp_source_snapshots;
        let skill_source_snapshots = persisted.skill_source_snapshots;
        let mut should_save = false;
        // Workspace 注册表：加载 + 默认工作区引导（三级树安装期行为：Windows
        // Documents\pony_agent / Unix ~/pony_agent，缺失即自动创建并登记；
        // 既有 default 登记永不静默迁移）。
        let mut workspaces = persisted.workspaces;
        if crate::agent::workspace::bootstrap_default_workspace(&mut workspaces) {
            should_save = true;
        }
        for session in sessions.values_mut() {
            refresh_session_metadata(session, false);
            if ensure_history_graph(session) {
                should_save = true;
            }
            if session.updated_at_ms == 0 {
                session.updated_at_ms = now_timestamp_ms();
            }
            if sanitize_attachment_references(session) {
                should_save = true;
            }
            if backfill_attachment_reference_assets(session) {
                should_save = true;
            }
        }
        // PA-094：trace 表降级为投影缓存——加载时若 trace 缓存为空但事件流存在，
        // 从事件全量折叠重建（事件权威，清表可重建，spec 1a）。
        // 审核 P1：折叠全量（不按 active branch 过滤，避免多分支 trace 丢失）；
        // 折叠结果为空时不置 should_save（避免每次启动全量 refold + 全量 save）。
        // 审核 P1-3：重建结果回写 trace 表（AppendTrace，sequence=终态 seq 作水位）
        // ——否则被清过缓存的会话每次启动都全量重折叠，"投影缓存"定位失效。
        for session in sessions.values_mut() {
            if session.turn_trace_history.is_empty() {
                let events = backend.load_turn_events(&session.conversation_id, None);
                if !events.is_empty() {
                    let traces = fold_session_traces_all(&events);
                    if !traces.is_empty() {
                        for (order, trace) in traces.iter().enumerate() {
                            let mut cache_trace = trace.clone();
                            cache_trace.sequence =
                                Some(events.last().map(|(seq, _, _)| *seq).unwrap_or(0));
                            let _ = backend.persist_command(PersistCommand::AppendTrace {
                                epoch: 1,
                                session_id: session.conversation_id.clone(),
                                trace: cache_trace,
                                trace_order: order,
                            });
                        }
                        session.turn_trace_history = traces;
                        should_save = true;
                    }
                }
            }
        }
        let rebuilt_assets = rebuild_attachment_assets_from_sessions(&sessions, &attachment_assets);
        let rebuilt_index = rebuild_session_attachment_index(&sessions);
        if attachment_assets != rebuilt_assets {
            attachment_assets = rebuilt_assets;
            should_save = true;
        }
        if session_attachment_index != rebuilt_index {
            session_attachment_index = rebuilt_index;
            should_save = true;
        }
        // PA-095：启动时事件 schema 版本校验——不匹配 fail loud（强提示），
        // 数据不可用而非部分视图；缺失 key 由 backend 回填（视为 v1）。
        if let Err(error) = backend.validate_event_schema() {
            eprintln!(
                "[pony-agent][session] EVENT SCHEMA VERSION MISMATCH — event-sourced views unavailable: {error}"
            );
        }
        let store = Self {
            sessions,
            attachment_assets,
            session_attachment_index,
            mcp_source_snapshots,
            skill_source_snapshots,
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
            event_stream_degraded: std::sync::Mutex::new(std::collections::HashSet::new()),
            history_state_hook_executor: Arc::new(NoopHistoryStateHookExecutor),
            projection_cache: HashMap::new(),
        };
        if should_save {
            store.save_to_backend();
        }
        store
    }

    #[allow(dead_code)]
    pub fn snapshot(
        &mut self,
        session_id: Option<&str>,
        fallback_history: &[TurnHistoryMessage],
    ) -> SessionSnapshot {
        self.snapshot_at(session_id, None, fallback_history)
    }

    pub fn snapshot_at(
        &mut self,
        session_id: Option<&str>,
        node_id: Option<&str>,
        fallback_history: &[TurnHistoryMessage],
    ) -> SessionSnapshot {
        let session_key = session_id.unwrap_or(DEFAULT_SESSION_ID);
        let mut should_save = false;
        let mut should_refresh_catalog = false;
        {
            let session = self.ensure_session(session_key);
            if sanitize_provider_native_transcript(session) {
                should_save = true;
            }
            if session.history.is_empty() && !fallback_history.is_empty() {
                session.history = fallback_history.to_vec();
                refresh_session_metadata(session, false);
                should_save = true;
                should_refresh_catalog = true;
            }
            if ensure_history_graph(session) {
                should_save = true;
            }
        }
        if should_refresh_catalog {
            self.refresh_attachment_catalog();
        }
        let snapshot = self.snapshot_for_session_at(session_key, node_id);

        if should_save {
            self.save_to_backend();
        }

        snapshot
    }

    /// 只读快照：不创建 session、不修正数据、不触发任何落盘。
    /// 供可观测性/展示类查询使用（读锁即可），避免 trace 面板等
    /// 查询路径占用写锁或意外触发全量写库而阻塞主对话执行路径。
    pub fn snapshot_at_readonly(
        &self,
        session_id: Option<&str>,
        node_id: Option<&str>,
        fallback_history: &[TurnHistoryMessage],
    ) -> SessionSnapshot {
        let session_key = session_id.unwrap_or(DEFAULT_SESSION_ID);
        let Some(session) = self.sessions.get(session_key) else {
            return default_snapshot_for_session(session_key, node_id);
        };
        let attachment_assets = attachment_assets_for_query(
            &self.sessions,
            &self.attachment_assets,
            &self.session_attachment_index,
            &self.attachment_root,
            &AttachmentAssetQuery {
                session_id: Some(session_key.to_string()),
                ..AttachmentAssetQuery::default()
            },
            now_timestamp_ms(),
        );
        let mut snapshot = snapshot_from_state(session, attachment_assets, node_id);
        if snapshot.history.is_empty() && !fallback_history.is_empty() {
            snapshot.history = fallback_history.to_vec();
        }
        snapshot
    }

    pub fn append_turn_fallible(
        &mut self,
        session_id: Option<&str>,
        user_message: &str,
        assistant_message: &str,
        provider_native_transcript: Option<Vec<Value>>,
        attachments: Vec<SessionAttachment>,
    ) -> Result<SessionSnapshot, SessionError> {
        let session_key = session_id.unwrap_or(DEFAULT_SESSION_ID).to_string();
        // PA-095 #2：事件源物化——事件能力后端先请求提交本会话缓冲（commit 先于
        // 清空，由注册闭包保证），再按事件表物化本 turn 的消息文本；消息文本以
        // 事件为源。回退仅限三类（均记录可观测日志）：真无事件后端
        // （Memory/File）；事件流无 turn 归属事件（直连 store 的 legacy 用法）；
        // 缓冲 flush 失败（提交不完整时按事件物化有错 turn 风险，整体回退更安全
        // ——并行测试下全局注册表被外来 control plane 抢占亦落入此路径）。
        // 事件读取失败（degraded/版本不匹配）属数据完整性错误，fail loud 上抛。
        let (mut event_user, mut event_assistant) = (None, None);
        let current_watermark = self.sessions.get(&session_key).map(|s| s.event_watermark).unwrap_or(0);
        if self.backend.supports_turn_events() {
            match crate::agent::turn_flow::flush_session_buffered_events(&session_key) {
                Ok(_) => match self.load_turn_events_checked(&session_key, None) {
                    Ok(events) => match materialize_last_turn_messages(&events, current_watermark) {
                        Some(materialized) => {
                            (event_user, event_assistant) = materialized;
                        }
                        None => {
                            eprintln!(
                                "[pony-agent][session] append_turn: no turn-scoped events for session={session_key} (direct store usage?), falling back to caller text"
                            );
                        }
                    },
                    Err(error) => {
                        return Err(SessionError::EventStreamDegraded(format!(
                            "[pony-agent][session] append_turn materialization: event read failed: {error}"
                        )));
                    }
                },
                Err(error) => {
                    eprintln!(
                        "[pony-agent][session] append_turn: buffered event flush failed ({error}); falling back to caller text"
                    );
                }
            }
        }
        let user_content = event_user.unwrap_or_else(|| user_message.to_string());
        let (assistant_content, assistant_reasoning) =
            event_assistant.unwrap_or_else(|| (assistant_message.to_string(), None));
        let memory_write_hook_executor = Arc::clone(&self.memory_write_hook_executor);
        let history_len_before;
        let mut appended_user: Option<TurnHistoryMessage>;
        let mut appended_assistant: Option<TurnHistoryMessage>;
        let meta_patch: Option<SessionMetaPatch>;

        // 两阶段暂存提交（Staging Commit）：在克隆状态上计算新 turn，持久化成功后才提交到 live
        let mut staged_session = self.ensure_session(&session_key).clone();
        {
            ensure_history_graph(&mut staged_session);
            prepare_session_for_new_turn(&mut staged_session);
            history_len_before = staged_session.history.len();
            staged_session.history.push(TurnHistoryMessage {
                role: "user".to_string(),
                content: user_content,
                attachments,
                ..Default::default()
            });
            staged_session.history.push(TurnHistoryMessage {
                role: "assistant".to_string(),
                content: assistant_content,
                attachments: Vec::new(),
                reasoning_content: assistant_reasoning,
                ..Default::default()
            });

            if staged_session.history.len() > DEFAULT_HISTORY_LIMIT {
                let keep_from = staged_session.history.len() - DEFAULT_HISTORY_LIMIT;
                staged_session.history.drain(..keep_from);
            }

            if let Some(messages) = provider_native_transcript {
                staged_session.provider_native_transcript.extend(messages);
            }

            update_long_term_memory_from_user_message(
                &mut staged_session,
                user_message,
                memory_write_hook_executor.as_ref(),
            );
            refresh_session_metadata(&mut staged_session, true);
            commit_history_node_from_live_state(&mut staged_session, HistoryNodeKind::TurnCommitted, None);
            let window_start = history_len_before.min(staged_session.history.len());
            appended_user = staged_session.history.get(window_start).cloned();
            appended_assistant = staged_session.history.get(window_start + 1).cloned();
            meta_patch = Some(SessionMetaPatch {
                title: Some(staged_session.title.clone()),
                title_override: Some(staged_session.title_override.clone()),
                archived: Some(staged_session.archived),
                summary: Some(staged_session.summary.clone()),
                turn_count: Some(staged_session.turn_count),
                last_referenced_file: staged_session.last_referenced_file.clone(),
                updated_at_ms: Some(staged_session.updated_at_ms),
            });
        }

        let latest_node = staged_session.history_nodes.last().cloned();
        let latest_cursor = staged_session.history_cursor.clone();

        // 增量批事务持久化（消除写放大）
        if self.backend.supports_turn_events() {
            let mut commands = Vec::new();
            for (ordinal, message) in [
                (history_len_before, appended_user.take()),
                (history_len_before + 1, appended_assistant.take()),
            ] {
                let Some(message) = message else { continue };
                commands.push(PersistCommand::AppendMessage {
                    epoch: 1,
                    session_id: session_key.clone(),
                    message,
                    ordinal,
                });
            }
            if let Some(meta_patch) = meta_patch {
                commands.push(PersistCommand::UpdateSessionMeta {
                    epoch: 1,
                    session_id: session_key.clone(),
                    meta_patch,
                });
            }
            if let Some(node) = latest_node {
                commands.push(PersistCommand::UpdateHistoryNode {
                    epoch: 1,
                    session_id: session_key.clone(),
                    node,
                });
            }
            commands.push(PersistCommand::UpdateCursor {
                epoch: 1,
                session_id: session_key.clone(),
                cursor: latest_cursor,
            });
            for branch in &staged_session.history_branches {
                commands.push(PersistCommand::UpdateBranch {
                    epoch: 1,
                    session_id: session_key.clone(),
                    branch: branch.clone(),
                });
            }

            let outcome = self.backend.persist_commands_batch(commands);
            if matches!(outcome, PersistCommandOutcome::Failed | PersistCommandOutcome::StaleEpoch) {
                return Err(SessionError::BackendFailure(format!(
                    "append_turn batch persist failed: {outcome:?}"
                )));
            }
            self.save_session_state_to_backend(&session_key, &staged_session);
        } else {
            self.save_session_state_to_backend(&session_key, &staged_session);
        }

        // 持久化成功，原子写入主内存并更新快照
        self.sessions.insert(session_key.clone(), staged_session);
        self.refresh_attachment_catalog();
        Ok(self.snapshot_for_session(&session_key))
    }

    pub fn append_turn(
        &mut self,
        session_id: Option<&str>,
        user_message: &str,
        assistant_message: &str,
        provider_native_transcript: Option<Vec<Value>>,
        attachments: Vec<SessionAttachment>,
    ) -> SessionSnapshot {
        match self.append_turn_fallible(
            session_id,
            user_message,
            assistant_message,
            provider_native_transcript,
            attachments,
        ) {
            Ok(snapshot) => snapshot,
            Err(e) => {
                eprintln!("[pony-agent][session] append_turn error: {e}");
                self.snapshot_for_session(session_id.unwrap_or(DEFAULT_SESSION_ID))
            }
        }
    }

    pub fn append_failed_turn_fallible(
        &mut self,
        session_id: Option<&str>,
        user_message: &str,
        assistant_message: &str,
        mut trace: TurnTraceRecord,
    ) -> Result<SessionSnapshot, SessionError> {
        let session_key = session_id.unwrap_or(DEFAULT_SESSION_ID).to_string();
        let mut staged_session = self.ensure_session(&session_key).clone();
        {
            ensure_history_graph(&mut staged_session);
            prepare_session_for_new_turn(&mut staged_session);
            staged_session.history.push(TurnHistoryMessage {
                role: "user".to_string(),
                content: user_message.to_string(),
                attachments: Vec::new(),
                ..Default::default()
            });
            staged_session.history.push(TurnHistoryMessage {
                role: "assistant".to_string(),
                content: assistant_message.to_string(),
                attachments: Vec::new(),
                ..Default::default()
            });
            trace.updated_at = now_timestamp_ms();
            staged_session.turn_trace_history.push(trace);
            if staged_session.history.len() > DEFAULT_HISTORY_LIMIT {
                let keep_from = staged_session.history.len() - DEFAULT_HISTORY_LIMIT;
                staged_session.history.drain(..keep_from);
            }
            if staged_session.turn_trace_history.len() > DEFAULT_HISTORY_LIMIT {
                let keep_from = staged_session.turn_trace_history.len() - DEFAULT_HISTORY_LIMIT;
                staged_session.turn_trace_history = staged_session.turn_trace_history[keep_from..].to_vec();
            }
            refresh_session_metadata(&mut staged_session, true);
            commit_history_node_from_live_state(
                &mut staged_session,
                classify_turn_node_kind(assistant_message),
                None,
            );
        }

        self.sessions.insert(session_key.clone(), staged_session.clone());
        let mutation = SessionTraceMutation::ReplaceAll {
            traces: staged_session.turn_trace_history.clone(),
        };
        self.save_session_state_to_backend(&session_key, &staged_session);
        self.persist_session_and_trace_change(&session_key, mutation);

        Ok(self.snapshot_for_session(&session_key))
    }

    pub fn append_failed_turn(
        &mut self,
        session_id: Option<&str>,
        user_message: &str,
        assistant_message: &str,
        trace: TurnTraceRecord,
    ) -> SessionSnapshot {
        match self.append_failed_turn_fallible(session_id, user_message, assistant_message, trace) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[pony-agent][session] append_failed_turn error: {e}");
                self.snapshot_for_session(session_id.unwrap_or(DEFAULT_SESSION_ID))
            }
        }
    }

    /// 替换会话的历史记录（用于上下文压缩后更新历史）
    pub fn replace_session_history(
        &mut self,
        session_id: Option<&str>,
        new_history: Vec<TurnHistoryMessage>,
    ) {
        let session_key = session_id.unwrap_or(DEFAULT_SESSION_ID).to_string();
        {
            let session = self.ensure_session(&session_key);
            session.history = new_history;
            refresh_session_metadata(session, true);
            commit_history_node_from_live_state(session, HistoryNodeKind::TurnCommitted, None);
        }
        self.save_to_backend();
    }

    /// 仅做内存更新并返回待持久化的 trace 变更（SQLite 落库由调用方决定，
    /// 主对话热路径应通过后台队列异步落库，避免可观测性写盘阻塞主对话）。
    pub fn record_turn_trace_in_memory(
        &mut self,
        session_id: Option<&str>,
        mut trace: TurnTraceRecord,
    ) -> (String, SessionTraceMutation) {
        let session_key = session_id.unwrap_or(DEFAULT_SESSION_ID).to_string();
        {
            let session = self.ensure_session(&session_key);
            ensure_history_graph(session);
            trace.updated_at = now_timestamp_ms();
            // PA-095 #7：live 写入剥离 observation 全量 payload（R4b No duplicate
            // storage）——payload 已由事件 flush 外置到 build_context_observations 表，
            // 引用由 flush 同事务回填到 trace 行；缓存行不再内嵌副本
            // （legacy 内嵌数据读取兼容不变，读路径优先 ref、缺失回退内嵌）。
            trace.build_context_observation = None;

            if let Some(existing) = session
                .turn_trace_history
                .iter_mut()
                .find(|item| item.turn_id == trace.turn_id)
            {
                *existing = trace.clone();
            } else {
                session.turn_trace_history.push(trace.clone());
            }

            if session.turn_trace_history.len() > DEFAULT_HISTORY_LIMIT {
                let keep_from = session.turn_trace_history.len() - DEFAULT_HISTORY_LIMIT;
                session.turn_trace_history = session.turn_trace_history[keep_from..].to_vec();
            }

            refresh_session_metadata(session, true);
            sync_latest_history_node(session, Some(trace.turn_id.clone()));
        }
        let snapshot = self.snapshot_for_session(&session_key);
        let trace_order = snapshot
            .turn_trace_history
            .iter()
            .position(|item| item.turn_id == trace.turn_id)
            .unwrap_or(snapshot.turn_trace_history.len().saturating_sub(1));
        (
            session_key,
            SessionTraceMutation::UpsertOne {
                trace: trace.clone(),
                trace_order,
            },
        )
    }

    pub fn record_turn_trace(
        &mut self,
        session_id: Option<&str>,
        trace: TurnTraceRecord,
    ) -> SessionSnapshot {
        let (session_key, mutation) = self.record_turn_trace_in_memory(session_id, trace);
        let snapshot = self.snapshot_for_session(&session_key);
        self.persist_session_and_trace_change(&session_key, mutation);
        snapshot
    }

    /// 后台 trace 持久化 worker 的落库入口：与 persist_session_and_trace_change
    /// 行为一致（含失败回退），但只被后台线程调用，不阻塞主对话执行路径。
    pub fn persist_trace_mutation_from_worker(
        &mut self,
        session_id: &str,
        mutation: SessionTraceMutation,
    ) {
        self.persist_session_and_trace_change(session_id, mutation);
    }

    /// 仅做内存更新并返回待持久化的 trace 变更（供异步落库使用）。
    pub fn annotate_turn_trace_terminal_event_in_memory(
        &mut self,
        session_id: Option<&str>,
        turn_id: &str,
        event_id: Option<String>,
        event_type: Option<String>,
        event_version: Option<String>,
        sequence: Option<u64>,
        emitted_at_ms: Option<u64>,
    ) -> Option<(String, SessionTraceMutation)> {
        let session_key = session_id.unwrap_or(DEFAULT_SESSION_ID).to_string();
        let persisted_event_id = event_id.clone();
        let persisted_event_type = event_type.clone();
        let persisted_event_version = event_version.clone();
        let persisted_event_watermark = {
            let session = self.sessions.get_mut(&session_key)?;
            let trace = session
                .turn_trace_history
                .iter_mut()
                .find(|item| item.turn_id == turn_id)?;
            trace.session_id = Some(session_key.clone());
            trace.event_id = event_id;
            trace.event_type = event_type;
            trace.event_version = event_version;
            trace.sequence = sequence;
            trace.emitted_at_ms = emitted_at_ms;
            trace.updated_at = now_timestamp_ms();
            refresh_session_metadata(session, true);
            // PA-094：trace 缓存行水位 = 会话事件日志水位（终态 flush 后已推进）。
            session.event_watermark
        };
        let updated_at = self
            .sessions
            .get(&session_key)
            .and_then(|session| {
                session
                    .turn_trace_history
                    .iter()
                    .find(|item| item.turn_id == turn_id)
                    .map(|trace| trace.updated_at)
            })
            .unwrap_or_default();
        Some((
            session_key,
            SessionTraceMutation::UpdateTerminalEvent {
                turn_id: turn_id.to_string(),
                event_id: persisted_event_id,
                event_type: persisted_event_type,
                event_version: persisted_event_version,
                sequence,
                emitted_at_ms,
                updated_at,
                event_watermark: Some(persisted_event_watermark),
            },
        ))
    }

    pub fn annotate_turn_trace_terminal_event(
        &mut self,
        session_id: Option<&str>,
        turn_id: &str,
        event_id: Option<String>,
        event_type: Option<String>,
        event_version: Option<String>,
        sequence: Option<u64>,
        emitted_at_ms: Option<u64>,
    ) -> Option<SessionSnapshot> {
        let session_key = session_id.unwrap_or(DEFAULT_SESSION_ID).to_string();
        let persisted_event_id = event_id.clone();
        let persisted_event_type = event_type.clone();
        let persisted_event_version = event_version.clone();
        {
            let session = self.sessions.get_mut(&session_key)?;
            let trace = session
                .turn_trace_history
                .iter_mut()
                .find(|item| item.turn_id == turn_id)?;
            trace.session_id = Some(session_key.clone());
            trace.event_id = event_id;
            trace.event_type = event_type;
            trace.event_version = event_version;
            trace.sequence = sequence;
            trace.emitted_at_ms = emitted_at_ms;
            trace.updated_at = now_timestamp_ms();
            refresh_session_metadata(session, true);
        }
        let updated_at = self
            .sessions
            .get(&session_key)
            .and_then(|session| {
                session
                    .turn_trace_history
                    .iter()
                    .find(|item| item.turn_id == turn_id)
                    .map(|trace| trace.updated_at)
            })
            .unwrap_or_default();
        let snapshot = self.snapshot_for_session(&session_key);
        self.persist_session_and_trace_change(
            &session_key,
            SessionTraceMutation::UpdateTerminalEvent {
                turn_id: turn_id.to_string(),
                event_id: persisted_event_id,
                event_type: persisted_event_type,
                event_version: persisted_event_version,
                sequence,
                emitted_at_ms,
                updated_at,
                event_watermark: None,
            },
        );
        Some(snapshot)
    }

    /// 仅做内存更新并返回待持久化的 trace 变更（供异步落库使用）。
    /// hook 记录为空或无匹配 trace 时返回 None，表示无需持久化。
    pub fn append_turn_trace_hook_records_in_memory(
        &mut self,
        session_id: Option<&str>,
        turn_id: &str,
        hook_trace_records: Vec<HookTraceRecord>,
    ) -> Option<(String, SessionTraceMutation)> {
        if hook_trace_records.is_empty() {
            return None;
        }

        let session_key = session_id.unwrap_or(DEFAULT_SESSION_ID).to_string();
        let persisted_hook_trace_records = hook_trace_records.clone();
        {
            let session = self.sessions.get_mut(&session_key)?;
            let trace = session
                .turn_trace_history
                .iter_mut()
                .find(|item| item.turn_id == turn_id)?;
            trace.session_id = Some(session_key.clone());
            trace.hook_trace_records.extend(hook_trace_records);
            trace.updated_at = now_timestamp_ms();
            refresh_session_metadata(session, true);
            sync_latest_history_node(session, Some(turn_id.to_string()));
        }
        let updated_at = self
            .sessions
            .get(&session_key)
            .and_then(|session| {
                session
                    .turn_trace_history
                    .iter()
                    .find(|item| item.turn_id == turn_id)
                    .map(|trace| trace.updated_at)
            })
            .unwrap_or_default();
        Some((
            session_key,
            SessionTraceMutation::AppendHookRecords {
                turn_id: turn_id.to_string(),
                hook_trace_records: persisted_hook_trace_records,
                updated_at,
            },
        ))
    }

    pub fn append_turn_trace_hook_records(
        &mut self,
        session_id: Option<&str>,
        turn_id: &str,
        hook_trace_records: Vec<HookTraceRecord>,
    ) -> Option<SessionSnapshot> {
        if hook_trace_records.is_empty() {
            return Some(self.snapshot(session_id, &[]));
        }

        let session_key = session_id.unwrap_or(DEFAULT_SESSION_ID).to_string();
        let persisted_hook_trace_records = hook_trace_records.clone();
        {
            let session = self.sessions.get_mut(&session_key)?;
            let trace = session
                .turn_trace_history
                .iter_mut()
                .find(|item| item.turn_id == turn_id)?;
            trace.session_id = Some(session_key.clone());
            trace.hook_trace_records.extend(hook_trace_records);
            trace.updated_at = now_timestamp_ms();
            refresh_session_metadata(session, true);
            sync_latest_history_node(session, Some(turn_id.to_string()));
        }
        let updated_at = self
            .sessions
            .get(&session_key)
            .and_then(|session| {
                session
                    .turn_trace_history
                    .iter()
                    .find(|item| item.turn_id == turn_id)
                    .map(|trace| trace.updated_at)
            })
            .unwrap_or_default();
        let snapshot = self.snapshot_for_session(&session_key);
        self.persist_session_and_trace_change(
            &session_key,
            SessionTraceMutation::AppendHookRecords {
                turn_id: turn_id.to_string(),
                hook_trace_records: persisted_hook_trace_records,
                updated_at,
            },
        );
        Some(snapshot)
    }

    pub fn persist_mcp_source_snapshot(&mut self, snapshot: McpSourceSnapshot) {
        self.mcp_source_snapshots
            .insert(snapshot.source.source_id.clone(), snapshot);
        self.save_to_backend();
    }

    pub fn persist_skill_source_snapshot(&mut self, snapshot: SkillSourceSnapshot) {
        self.skill_source_snapshots
            .insert(snapshot.source.source_id.clone(), snapshot);
        self.save_to_backend();
    }

    pub fn list_persisted_mcp_source_snapshots(&self) -> Vec<McpSourceSnapshot> {
        self.mcp_source_snapshots.values().cloned().collect()
    }

    pub fn list_persisted_skill_source_snapshots(&self) -> Vec<SkillSourceSnapshot> {
        self.skill_source_snapshots.values().cloned().collect()
    }

    #[cfg(test)]
    pub fn set_memory_write_hook_executor_for_test(
        &mut self,
        executor: Box<dyn MemoryWriteHookExecutor>,
    ) {
        self.memory_write_hook_executor = executor.into();
    }

    #[cfg(test)]
    pub fn set_history_state_hook_executor_for_test(
        &mut self,
        executor: Box<dyn HistoryStateHookExecutor>,
    ) {
        self.history_state_hook_executor = executor.into();
    }

    #[allow(dead_code)]
    pub fn replace_long_term_memory(
        &mut self,
        session_id: Option<&str>,
        entries: Vec<LongTermMemoryRecord>,
    ) -> SessionSnapshot {
        let session_key = session_id.unwrap_or(DEFAULT_SESSION_ID).to_string();
        {
            let session = self.ensure_session(&session_key);
            ensure_history_graph(session);
            session.long_term_memory_entries = entries;
            session.memory_write_evidence.clear();
            session.memory_write_hook_trace_records.clear();
            session.history_state_evidence.clear();
            refresh_session_metadata(session, true);
            sync_latest_history_node(session, None);
        }
        let snapshot = self.snapshot_for_session(&session_key);

        self.save_session_to_backend(&session_key);
        snapshot
    }

    #[allow(dead_code)]
    pub fn checkout_history_node(
        &mut self,
        session_id: Option<&str>,
        node_id: &str,
        mode: HistoryCheckoutMode,
        expected_cursor_version: Option<u64>,
    ) -> Result<SessionSnapshot, String> {
        let session_key = session_id.unwrap_or(DEFAULT_SESSION_ID).to_string();
        let hook_executor = Arc::clone(&self.history_state_hook_executor);
        let mut blocked_error = None;
        // PA-093：块外预折叠——引用化节点由事件流重建视图（&mut self 与
        // ensure_session 借用互斥，先算结果，块内以纯函数应用）。
        let fold_result: Option<(Vec<TurnHistoryMessage>, Vec<TurnTraceRecord>)> = {
            let pre_node = self
                .sessions
                .get(&session_key)
                .and_then(|s| history_node(s, node_id))
                .cloned();
            pre_node
                .as_ref()
                .filter(|n| n.event_seq_range.is_some())
                .map(|n| self.fold_node_views(&session_key, n))
        };
        {
            let session = self.ensure_session(&session_key);
            ensure_history_graph(session);
            reject_stale_cursor_version(&session, expected_cursor_version)?;
            let requested_mode = mode.clone();
            if let Some(start_envelope) = build_history_state_hook_envelope(
                session,
                HistoryStateHookPoint::HistoryCheckoutStart,
                HistoryStateCommandKind::CheckoutHistoryNode,
                Some(node_id),
                None,
                Some(&requested_mode),
                None,
                None,
                false,
                false,
                false,
                false,
                None,
            ) {
                let hook_results = hook_executor.execute(&start_envelope).unwrap_or_default();
                persist_history_state_hook_evidence(session, &start_envelope, &hook_results);
                if history_state_hook_results_blocked(&hook_results) {
                    refresh_session_metadata(session, true);
                    blocked_error = Some(format!(
                        "history checkout blocked by hook before resolving node: {node_id}"
                    ));
                }
            }
            if blocked_error.is_some() {
                // Keep the current truth-source unchanged when guard hooks block checkout.
                sync_latest_history_node(session, None);
            } else {
                // ── Build node index for O(1) lookups ──
                let node_index: HashMap<&str, usize> = session
                    .history_nodes
                    .iter()
                    .enumerate()
                    .map(|(i, n)| (n.node_id.as_str(), i))
                    .collect();

                let Some(&node_idx) = node_index.get(node_id) else {
                    return Err(format!("unknown history node: {node_id}"));
                };
                let node = session.history_nodes[node_idx].clone();

                // ── Truncate history: mark descendant nodes as cancelled ──
                // Collect all ancestor node IDs (including the target node itself)
                // using the index for O(1) lookups instead of O(n) linear scans.
                let mut ancestor_ids = HashSet::new();
                let mut current = Some(node.node_id.as_str());
                while let Some(id) = current {
                    ancestor_ids.insert(id.to_string());
                    current = node_index
                        .get(id)
                        .and_then(|&i| session.history_nodes[i].parent_node_id.as_deref());
                }
                let branch_id = node.branch_id.clone();
                // Drop the index to release the immutable borrow before mutating
                drop(node_index);

                for n in &mut session.history_nodes {
                    if n.branch_id == branch_id && !ancestor_ids.contains(&n.node_id) {
                        n.kind = HistoryNodeKind::TurnCancelled;
                    }
                }
                // ── Update branch head to point to the target node ──
                // This makes the truncation permanent: the target node IS now the
                // branch head, so subsequent restore_branch_head / new turns will
                // build from here instead of resurrecting the cancelled nodes.
                if let Some(branch) = session
                    .history_branches
                    .iter_mut()
                    .find(|b| b.branch_id == branch_id)
                {
                    branch.head_node_id = Some(node.node_id.clone());
                }
                // PA-093：引用化节点 → 事件折叠重建视图（水位回退）；legacy → 旧路径。
                match &fold_result {
                    Some((history, trace)) => hydrate_session_from_projection(
                        session,
                        &node,
                        history.clone(),
                        trace.clone(),
                    ),
                    None => hydrate_session_from_node(session, &node),
                }
                session.history_cursor.visible_node_id = Some(node.node_id.clone());
                session.history_cursor.active_branch_id = Some(branch_id.clone());
                session.history_cursor.branch_head_node_id = Some(node.node_id.clone());
                session.history_cursor.workspace_node_id = Some(node.node_id.clone());
                // Target node is now the branch head, so mode is always Live.
                session.history_cursor.mode = HistoryCursorMode::Live;
                session.history_cursor.checkout_mode = requested_mode.clone();
                session.history_cursor.checkout_status = match requested_mode {
                    HistoryCheckoutMode::TranscriptOnly => HistoryCheckoutStatus::Applied,
                    HistoryCheckoutMode::TranscriptAndWorkspace => {
                        if node.workspace_ref.rollback_capable {
                            HistoryCheckoutStatus::Applied
                        } else {
                            HistoryCheckoutStatus::DegradedToTranscriptOnly
                        }
                    }
                };
                // PA-095 #6：cursor_version 退役——值来源切换 event_watermark
                // （wire 字段名不变，前端不透明使用）；冲突检测改水位比较。
                session.history_cursor.event_watermark = session.event_watermark;
                session.history_cursor.cursor_version = session.event_watermark;
                // PA-093：checkout 是视图切换事实，append 事件（分支可见性推导基石）。
                crate::agent::turn_flow::emit_global_event(
                    &session_key,
                    "checkpoint",
                    crate::agent::turn_event::TurnEvent::CheckpointCheckout {
                        node_id: node.node_id.clone(),
                        mode: serde_json::to_string(&requested_mode)
                            .unwrap_or_else(|_| "transcript_only".to_string()),
                    },
                );
                refresh_session_metadata(session, true);
                if let Some(resolved_envelope) = build_history_state_hook_envelope(
                    session,
                    HistoryStateHookPoint::HistoryCheckoutResolved,
                    HistoryStateCommandKind::CheckoutHistoryNode,
                    Some(node_id),
                    None,
                    Some(&requested_mode),
                    Some(node.node_id.as_str()),
                    Some(node.branch_id.as_str()),
                    true,
                    node.workspace_ref.rollback_capable,
                    matches!(
                        session.history_cursor.checkout_status,
                        HistoryCheckoutStatus::Applied
                    ),
                    matches!(
                        session.history_cursor.checkout_status,
                        HistoryCheckoutStatus::DegradedToTranscriptOnly
                    ),
                    matches!(
                        session.history_cursor.checkout_status,
                        HistoryCheckoutStatus::DegradedToTranscriptOnly
                    )
                    .then_some("workspace_rollback_unsupported"),
                ) {
                    let hook_results = hook_executor
                        .execute(&resolved_envelope)
                        .unwrap_or_default();
                    persist_history_state_hook_evidence(session, &resolved_envelope, &hook_results);
                }
            }
        }
        self.persist_session_and_trace_change(
            &session_key,
            SessionTraceMutation::ReplaceAll {
                traces: self.load_turn_traces(&session_key),
            },
        );
        if let Some(error) = blocked_error {
            return Err(error);
        }
        let snapshot = self.snapshot_for_session(&session_key);
        Ok(snapshot)
    }

    pub fn load_history_graph(
        &mut self,
        session_id: Option<&str>,
    ) -> (Vec<HistoryNode>, Vec<HistoryBranch>, HistoryCursor) {
        let session_key = session_id.unwrap_or(DEFAULT_SESSION_ID).to_string();
        let mut should_save = false;
        let graph = {
            let session = self.ensure_session(&session_key);
            if ensure_history_graph(session) {
                should_save = true;
            }
            (
                session.history_nodes.clone(),
                session.history_branches.clone(),
                session.history_cursor.clone(),
            )
        };
        if should_save {
            self.save_to_backend();
        }
        graph
    }

    pub fn load_history_cursor(&mut self, session_id: Option<&str>) -> HistoryCursor {
        self.load_history_graph(session_id).2
    }

    pub fn restore_branch_head(
        &mut self,
        session_id: Option<&str>,
        branch_id: Option<&str>,
        expected_cursor_version: Option<u64>,
    ) -> Result<SessionSnapshot, String> {
        let session_key = session_id.unwrap_or(DEFAULT_SESSION_ID).to_string();
        let hook_executor = Arc::clone(&self.history_state_hook_executor);
        let mut blocked_error = None;
        let mut restored_node_id = None;
        // PA-093：块外预折叠（引用化分支头节点由事件流重建视图）。
        let fold_result: Option<(Vec<TurnHistoryMessage>, Vec<TurnTraceRecord>)> = {
            let pre_node = self.sessions.get(&session_key).and_then(|s| {
                let target = branch_id
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
                    .or_else(|| s.history_cursor.active_branch_id.clone())
                    .unwrap_or_else(|| DEFAULT_HISTORY_BRANCH_ID.to_string());
                s.history_branches
                    .iter()
                    .find(|b| b.branch_id == target)
                    .and_then(|b| b.head_node_id.clone())
                    .and_then(|nid| history_node(s, &nid).cloned())
            });
            pre_node
                .as_ref()
                .filter(|n| n.event_seq_range.is_some())
                .map(|n| self.fold_node_views(&session_key, n))
        };
        {
            let session = self.ensure_session(&session_key);
            ensure_history_graph(session);
            reject_stale_cursor_version(&session, expected_cursor_version)?;
            let target_branch_id = branch_id
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .or_else(|| session.history_cursor.active_branch_id.clone())
                .unwrap_or_else(|| DEFAULT_HISTORY_BRANCH_ID.to_string());
            if let Some(start_envelope) = build_history_state_hook_envelope(
                session,
                HistoryStateHookPoint::BranchRestoreStart,
                HistoryStateCommandKind::RestoreBranchHead,
                None,
                Some(target_branch_id.as_str()),
                None,
                None,
                None,
                false,
                false,
                false,
                false,
                None,
            ) {
                let hook_results = hook_executor.execute(&start_envelope).unwrap_or_default();
                persist_history_state_hook_evidence(session, &start_envelope, &hook_results);
                if history_state_hook_results_blocked(&hook_results) {
                    refresh_session_metadata(session, true);
                    blocked_error = Some(format!(
                        "history branch restore blocked by hook before resolving branch: {target_branch_id}"
                    ));
                }
            }
            if blocked_error.is_some() {
                sync_latest_history_node(session, None);
            } else {
                let branch = session
                    .history_branches
                    .iter()
                    .find(|item| item.branch_id == target_branch_id)
                    .cloned()
                    .ok_or_else(|| format!("unknown history branch: {target_branch_id}"))?;
                let node_id = branch.head_node_id.clone().ok_or_else(|| {
                    format!("history branch has no head node: {target_branch_id}")
                })?;
                let node = history_node(session, &node_id)
                    .cloned()
                    .ok_or_else(|| format!("unknown history node: {node_id}"))?;
                // PA-093：引用化节点 → 事件折叠重建视图；legacy → 旧路径。
                match &fold_result {
                    Some((history, trace)) => hydrate_session_from_projection(
                        session,
                        &node,
                        history.clone(),
                        trace.clone(),
                    ),
                    None => hydrate_session_from_node(session, &node),
                }
                session.history_cursor.visible_node_id = Some(node.node_id.clone());
                session.history_cursor.active_branch_id = Some(branch.branch_id.clone());
                session.history_cursor.branch_head_node_id = Some(node.node_id.clone());
                session.history_cursor.workspace_node_id = Some(node.node_id.clone());
                session.history_cursor.mode = HistoryCursorMode::Live;
                session.history_cursor.checkout_mode = HistoryCheckoutMode::TranscriptOnly;
                session.history_cursor.checkout_status = HistoryCheckoutStatus::NotRequested;
                // PA-095 #6：cursor_version 退役——值来源切换 event_watermark
                // （wire 字段名不变，前端不透明使用）；冲突检测改水位比较。
                session.history_cursor.event_watermark = session.event_watermark;
                session.history_cursor.cursor_version = session.event_watermark;
                // PA-093：分支恢复 = 视图切换事实，append 事件。
                crate::agent::turn_flow::emit_global_event(
                    &session_key,
                    "checkpoint",
                    crate::agent::turn_event::TurnEvent::CheckpointCheckout {
                        node_id: node.node_id.clone(),
                        mode: "transcript_only".to_string(),
                    },
                );
                refresh_session_metadata(session, true);
                if let Some(resolved_envelope) = build_history_state_hook_envelope(
                    session,
                    HistoryStateHookPoint::BranchRestoreResolved,
                    HistoryStateCommandKind::RestoreBranchHead,
                    None,
                    Some(target_branch_id.as_str()),
                    None,
                    Some(node.node_id.as_str()),
                    Some(branch.branch_id.as_str()),
                    true,
                    false,
                    false,
                    false,
                    None,
                ) {
                    let hook_results = hook_executor
                        .execute(&resolved_envelope)
                        .unwrap_or_default();
                    persist_history_state_hook_evidence(session, &resolved_envelope, &hook_results);
                }
                restored_node_id = Some(node.node_id);
            }
        }
        self.persist_session_and_trace_change(
            &session_key,
            SessionTraceMutation::ReplaceAll {
                traces: self.load_turn_traces(&session_key),
            },
        );
        if let Some(error) = blocked_error {
            return Err(error);
        }
        let restored_node_id = restored_node_id.expect("restored node id should be available");
        let snapshot = self.snapshot_for_session_at(&session_key, Some(restored_node_id.as_str()));
        Ok(snapshot)
    }

    pub fn fork_from_history_node(
        &mut self,
        session_id: Option<&str>,
        node_id: &str,
        expected_cursor_version: Option<u64>,
    ) -> Result<SessionSnapshot, String> {
        let session_key = session_id.unwrap_or(DEFAULT_SESSION_ID).to_string();
        let hook_executor = Arc::clone(&self.history_state_hook_executor);
        let mut blocked_error = None;
        // PA-093：块外预折叠（引用化源节点由事件流重建视图）。
        let fold_result: Option<(Vec<TurnHistoryMessage>, Vec<TurnTraceRecord>)> = {
            let pre_node = self
                .sessions
                .get(&session_key)
                .and_then(|s| history_node(s, node_id))
                .cloned();
            pre_node
                .as_ref()
                .filter(|n| n.event_seq_range.is_some())
                .map(|n| self.fold_node_views(&session_key, n))
        };
        {
            let session = self.ensure_session(&session_key);
            ensure_history_graph(session);
            reject_stale_cursor_version(&session, expected_cursor_version)?;
            if let Some(start_envelope) = build_history_state_hook_envelope(
                session,
                HistoryStateHookPoint::BranchForkStart,
                HistoryStateCommandKind::ForkFromHistoryNode,
                Some(node_id),
                None,
                None,
                None,
                None,
                false,
                false,
                false,
                false,
                None,
            ) {
                let hook_results = hook_executor.execute(&start_envelope).unwrap_or_default();
                persist_history_state_hook_evidence(session, &start_envelope, &hook_results);
                if history_state_hook_results_blocked(&hook_results) {
                    refresh_session_metadata(session, true);
                    blocked_error = Some(format!(
                        "history branch fork blocked by hook before resolving node: {node_id}"
                    ));
                }
            }
            if blocked_error.is_some() {
                sync_latest_history_node(session, None);
            } else {
                let source_node = history_node(session, node_id)
                    .cloned()
                    .ok_or_else(|| format!("unknown history node: {node_id}"))?;
                let source_branch_id = source_node.branch_id.clone();
                let created_at_ms = now_timestamp_ms();
                let label_index = session.history_branches.len() + 1;
                let new_branch_id = new_history_branch_id(session, label_index);
                session.history_branches.push(HistoryBranch {
                    branch_id: new_branch_id.clone(),
                    session_id: session.conversation_id.clone(),
                    base_node_id: Some(source_node.node_id.clone()),
                    head_node_id: Some(source_node.node_id.clone()),
                    forked_from_branch_id: Some(source_branch_id),
                    forked_from_node_id: Some(source_node.node_id.clone()),
                    label: format!("fork-{label_index}"),
                    created_at_ms,
                    updated_at_ms: created_at_ms,
                });
                // PA-093：引用化节点 → 事件折叠重建视图；legacy → 旧路径。
                match &fold_result {
                    Some((history, trace)) => hydrate_session_from_projection(
                        session,
                        &source_node,
                        history.clone(),
                        trace.clone(),
                    ),
                    None => hydrate_session_from_node(session, &source_node),
                }
                session.history_cursor.visible_node_id = Some(source_node.node_id.clone());
                session.history_cursor.active_branch_id = Some(new_branch_id.clone());
                session.history_cursor.branch_head_node_id = Some(source_node.node_id.clone());
                session.history_cursor.workspace_node_id = Some(source_node.node_id.clone());
                session.history_cursor.mode = HistoryCursorMode::Live;
                session.history_cursor.checkout_mode = HistoryCheckoutMode::TranscriptOnly;
                session.history_cursor.checkout_status = HistoryCheckoutStatus::NotRequested;
                // PA-095 #6：cursor_version 退役——值来源切换 event_watermark
                // （wire 字段名不变，前端不透明使用）；冲突检测改水位比较。
                session.history_cursor.event_watermark = session.event_watermark;
                session.history_cursor.cursor_version = session.event_watermark;
                // PA-093：fork 是分支事实，append 事件（新分支事件携带 branch_id，
                // 折叠时按血缘链推导可见集合）。
                crate::agent::turn_flow::emit_global_event(
                    &session_key,
                    "fork",
                    crate::agent::turn_event::TurnEvent::ForkCreated {
                        branch_id: new_branch_id.clone(),
                        from_node_id: source_node.node_id.clone(),
                    },
                );
                refresh_session_metadata(session, true);
                if let Some(resolved_envelope) = build_history_state_hook_envelope(
                    session,
                    HistoryStateHookPoint::BranchForkResolved,
                    HistoryStateCommandKind::ForkFromHistoryNode,
                    Some(node_id),
                    None,
                    None,
                    Some(source_node.node_id.as_str()),
                    Some(new_branch_id.as_str()),
                    true,
                    false,
                    false,
                    false,
                    None,
                ) {
                    let hook_results = hook_executor
                        .execute(&resolved_envelope)
                        .unwrap_or_default();
                    persist_history_state_hook_evidence(session, &resolved_envelope, &hook_results);
                }
            }
        }
        self.persist_session_and_trace_change(
            &session_key,
            SessionTraceMutation::ReplaceAll {
                traces: self.load_turn_traces(&session_key),
            },
        );
        if let Some(error) = blocked_error {
            return Err(error);
        }
        let snapshot = self.snapshot_for_session(&session_key);
        Ok(snapshot)
    }

    pub fn switch_history_branch(
        &mut self,
        session_id: Option<&str>,
        branch_id: &str,
        expected_cursor_version: Option<u64>,
    ) -> Result<SessionSnapshot, String> {
        let session_key = session_id.unwrap_or(DEFAULT_SESSION_ID).to_string();
        let hook_executor = Arc::clone(&self.history_state_hook_executor);
        let mut blocked_error = None;
        let mut target_node_id = None;
        // PA-093：块外预折叠（引用化分支头节点由事件流重建视图）。
        let fold_result: Option<(Vec<TurnHistoryMessage>, Vec<TurnTraceRecord>)> = {
            let pre_node = self.sessions.get(&session_key).and_then(|s| {
                s.history_branches
                    .iter()
                    .find(|b| b.branch_id == branch_id)
                    .and_then(|b| b.head_node_id.clone())
                    .and_then(|nid| history_node(s, &nid).cloned())
            });
            pre_node
                .as_ref()
                .filter(|n| n.event_seq_range.is_some())
                .map(|n| self.fold_node_views(&session_key, n))
        };
        {
            let session = self.ensure_session(&session_key);
            ensure_history_graph(session);
            reject_stale_cursor_version(&session, expected_cursor_version)?;
            if let Some(start_envelope) = build_history_state_hook_envelope(
                session,
                HistoryStateHookPoint::BranchSwitchStart,
                HistoryStateCommandKind::SwitchHistoryBranch,
                None,
                Some(branch_id),
                None,
                None,
                None,
                false,
                false,
                false,
                false,
                None,
            ) {
                let hook_results = hook_executor.execute(&start_envelope).unwrap_or_default();
                persist_history_state_hook_evidence(session, &start_envelope, &hook_results);
                if history_state_hook_results_blocked(&hook_results) {
                    refresh_session_metadata(session, true);
                    blocked_error = Some(format!(
                        "history branch switch blocked by hook before resolving branch: {branch_id}"
                    ));
                }
            }
            if blocked_error.is_some() {
                sync_latest_history_node(session, None);
            } else {
                let branch = session
                    .history_branches
                    .iter()
                    .find(|item| item.branch_id == branch_id)
                    .cloned()
                    .ok_or_else(|| format!("unknown history branch: {branch_id}"))?;
                let node_id = branch
                    .head_node_id
                    .clone()
                    .ok_or_else(|| format!("history branch has no head node: {branch_id}"))?;
                let node = history_node(session, &node_id)
                    .cloned()
                    .ok_or_else(|| format!("unknown history node: {node_id}"))?;
                // PA-093：引用化节点 → 事件折叠重建视图；legacy → 旧路径。
                match &fold_result {
                    Some((history, trace)) => hydrate_session_from_projection(
                        session,
                        &node,
                        history.clone(),
                        trace.clone(),
                    ),
                    None => hydrate_session_from_node(session, &node),
                }
                session.history_cursor.visible_node_id = Some(node.node_id.clone());
                session.history_cursor.active_branch_id = Some(branch.branch_id.clone());
                session.history_cursor.branch_head_node_id = Some(node.node_id.clone());
                session.history_cursor.workspace_node_id = Some(node.node_id.clone());
                session.history_cursor.mode = HistoryCursorMode::Live;
                session.history_cursor.checkout_mode = HistoryCheckoutMode::TranscriptOnly;
                session.history_cursor.checkout_status = HistoryCheckoutStatus::NotRequested;
                // PA-095 #6：cursor_version 退役——值来源切换 event_watermark
                // （wire 字段名不变，前端不透明使用）；冲突检测改水位比较。
                session.history_cursor.event_watermark = session.event_watermark;
                session.history_cursor.cursor_version = session.event_watermark;
                // PA-093：分支切换 = 视图切换事实，append 事件。
                crate::agent::turn_flow::emit_global_event(
                    &session_key,
                    "checkpoint",
                    crate::agent::turn_event::TurnEvent::CheckpointCheckout {
                        node_id: node.node_id.clone(),
                        mode: "transcript_only".to_string(),
                    },
                );
                refresh_session_metadata(session, true);
                if let Some(resolved_envelope) = build_history_state_hook_envelope(
                    session,
                    HistoryStateHookPoint::BranchSwitchResolved,
                    HistoryStateCommandKind::SwitchHistoryBranch,
                    None,
                    Some(branch_id),
                    None,
                    Some(node.node_id.as_str()),
                    Some(branch.branch_id.as_str()),
                    true,
                    false,
                    false,
                    false,
                    None,
                ) {
                    let hook_results = hook_executor
                        .execute(&resolved_envelope)
                        .unwrap_or_default();
                    persist_history_state_hook_evidence(session, &resolved_envelope, &hook_results);
                }
                target_node_id = Some(node.node_id);
            }
        }
        self.persist_session_and_trace_change(
            &session_key,
            SessionTraceMutation::ReplaceAll {
                traces: self.load_turn_traces(&session_key),
            },
        );
        if let Some(error) = blocked_error {
            return Err(error);
        }
        let target_node_id = target_node_id.expect("target node id should be available");
        let snapshot = self.snapshot_for_session_at(&session_key, Some(target_node_id.as_str()));
        Ok(snapshot)
    }

    pub fn list_sessions(&self) -> Vec<SessionOverview> {
        let mut sessions = self
            .sessions
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
            })
            .collect::<Vec<_>>();

        sessions.sort_by(|left, right| {
            right
                .updated_at_ms
                .cmp(&left.updated_at_ms)
                .then_with(|| left.conversation_id.cmp(&right.conversation_id))
        });
        sessions
    }

    pub fn load_turn_traces(&self, session_id: &str) -> Vec<TurnTraceRecord> {
        self.sessions
            .get(session_id)
            .map(|session| session.turn_trace_history.clone())
            .unwrap_or_default()
    }

    pub fn remove_session(&mut self, session_id: &str) -> Vec<SessionOverview> {
        if self.sessions.remove(session_id).is_some() {
            delete_session_attachment_dir(&self.attachment_root, session_id);
            self.remove_session_attachment_catalog(session_id);

            if !self.backend.remove_session(
                session_id,
                &self.attachment_assets,
                &self.session_attachment_index,
                &self.mcp_source_snapshots,
                &self.skill_source_snapshots,
            ) {
                self.save_to_backend();
            }
        }

        if self.sessions.is_empty() {
            self.sessions = default_sessions();
        }
        self.list_sessions()
    }

    pub fn save_input_attachments(
        &mut self,
        session_id: &str,
        images: &[TurnInputImage],
    ) -> Result<Vec<SessionAttachment>, String> {
        if images.is_empty() {
            return Ok(Vec::new());
        }

        let session_dir = self.attachment_root.join(session_id);
        fs::create_dir_all(&session_dir)
            .map_err(|error| format!("failed to create attachment directory: {error}"))?;

        let created_at_ms = now_timestamp_ms();
        let mut attachments = Vec::with_capacity(images.len());
        for (index, image) in images.iter().enumerate() {
            let attachment_id = format!("att-{created_at_ms}-{}", index + 1);
            let file_name = format!("{attachment_id}.dataurl");
            let relative_path = format!("{session_id}/{file_name}");
            let asset_id = attachment_asset_id(&relative_path);
            let absolute_path = session_dir.join(&file_name);
            fs::write(&absolute_path, &image.data_url)
                .map_err(|error| format!("failed to persist attachment payload: {error}"))?;
            self.attachment_assets.insert(
                asset_id.clone(),
                AttachmentAsset {
                    id: asset_id.clone(),
                    session_id: session_id.to_string(),
                    name: image.name.clone(),
                    mime_type: image.mime_type.clone(),
                    relative_path: relative_path.clone(),
                    size_bytes: image.payload_size_bytes(),
                    created_at_ms,
                    status: AttachmentLifecycleStatus::Reclaimable,
                    reference_count: 0,
                    last_referenced_at_ms: None,
                    expires_at_ms: Some(
                        created_at_ms.saturating_add(DEFAULT_ATTACHMENT_RECLAIM_TTL_MS),
                    ),
                },
            );
            attachments.push(SessionAttachment {
                id: attachment_id,
                asset_id,
                name: image.name.clone(),
                mime_type: image.mime_type.clone(),
                relative_path,
                size_bytes: image.payload_size_bytes(),
                created_at_ms,
            });
        }
        Ok(attachments)
    }

    pub fn load_recent_images(
        &self,
        session_id: Option<&str>,
        limit: usize,
    ) -> Vec<TurnInputImage> {
        let Some(session_id) = session_id else {
            return Vec::new();
        };
        let Some(session) = self.sessions.get(session_id) else {
            return Vec::new();
        };
        if limit == 0 {
            return Vec::new();
        }

        session
            .history
            .iter()
            .rev()
            .find(|message| message.role == "user")
            .map(|message| {
                if message.attachments.is_empty() {
                    return Vec::new();
                }
                message
                    .attachments
                    .iter()
                    .take(limit)
                    .filter_map(|attachment| {
                        load_attachment_image(
                            &self.attachment_root,
                            &self.attachment_assets,
                            attachment,
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    }

    #[allow(dead_code)]
    pub fn list_attachment_assets(&self, session_id: Option<&str>) -> Vec<AttachmentAsset> {
        self.query_attachment_assets(&AttachmentAssetQuery {
            session_id: session_id.map(str::to_string),
            ..AttachmentAssetQuery::default()
        })
    }

    #[allow(dead_code)]
    pub fn query_attachment_assets(&self, query: &AttachmentAssetQuery) -> Vec<AttachmentAsset> {
        let mut assets = attachment_assets_for_query(
            &self.sessions,
            &self.attachment_assets,
            &self.session_attachment_index,
            &self.attachment_root,
            query,
            now_timestamp_ms(),
        );
        if let Some(limit) = query.limit {
            assets.truncate(limit);
        }
        assets
    }

    #[allow(dead_code)]
    pub fn cleanup_attachment_assets(
        &mut self,
        request: &AttachmentCleanupRequest,
    ) -> AttachmentCleanupResult {
        if !request.include_expired && !request.include_reclaimable {
            return AttachmentCleanupResult::default();
        }

        let mut query = AttachmentAssetQuery {
            session_id: request.session_id.clone(),
            ..AttachmentAssetQuery::default()
        };
        if request.include_expired {
            query.statuses.push(AttachmentLifecycleStatus::Expired);
        }
        if request.include_reclaimable {
            query.statuses.push(AttachmentLifecycleStatus::Reclaimable);
        }
        query.limit = request.limit;

        let candidates = self.query_attachment_assets(&query);
        let mut removed_asset_ids = Vec::new();
        let mut removed_file_count = 0;

        for asset in candidates {
            if asset.reference_count > 0 {
                continue;
            }

            let matches_cleanup_policy = match asset.status {
                AttachmentLifecycleStatus::Reclaimable => request.include_reclaimable,
                AttachmentLifecycleStatus::Expired => {
                    request.include_expired
                        && request.expire_before_ms.map_or(true, |cutoff| {
                            asset
                                .expires_at_ms
                                .map(|expires_at_ms| expires_at_ms <= cutoff)
                                .unwrap_or(asset.created_at_ms <= cutoff)
                        })
                }
                AttachmentLifecycleStatus::Active | AttachmentLifecycleStatus::MissingPayload => {
                    false
                }
            };
            if !matches_cleanup_policy {
                continue;
            }

            let path = self.attachment_root.join(&asset.relative_path);
            if path.is_file() && fs::remove_file(&path).is_ok() {
                removed_file_count += 1;
            }
            if let Some(parent) = path.parent() {
                let _ = fs::remove_dir(parent);
            }

            if self.attachment_assets.remove(&asset.id).is_some() {
                removed_asset_ids.push(asset.id);
            }
        }

        if !removed_asset_ids.is_empty() {
            self.refresh_attachment_catalog();
            self.save_to_backend();
        }

        AttachmentCleanupResult {
            removed_catalog_count: removed_asset_ids.len(),
            removed_asset_ids,
            removed_file_count,
        }
    }

    pub(super) fn ensure_session(&mut self, session_id: &str) -> &mut SessionState {
        let initial_trace_migration_state = match self.backend.trace_storage_mode() {
            SeparateTraceTableMode::Off => TraceMigrationState::LegacyBlob,
            SeparateTraceTableMode::DualWrite => TraceMigrationState::DualWrite,
            // PA-088：新会话无旧数据，直接表权威（blob 只存 refs，不膨胀）。
            // 存量会话（load_store 反序列化）保持其 blob 中的状态，不在此晋升。
            SeparateTraceTableMode::WriteSeparate => TraceMigrationState::TraceTableAuthoritative,
        };
        self.sessions
            .entry(session_id.to_string())
            .or_insert_with(|| SessionState {
                conversation_id: session_id.to_string(),
                title: DEFAULT_SESSION_TITLE.to_string(),
                summary: DEFAULT_SESSION_SUMMARY.to_string(),
                history: Vec::new(),
                provider_native_transcript: Vec::new(),
                turn_trace_history: Vec::new(),
                trace_migration_state: initial_trace_migration_state,
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
                history_cursor: HistoryCursor {
                    session_id: session_id.to_string(),
                    ..HistoryCursor::default()
                },
                event_watermark: 0,
                last_commit_watermark: 0,
                workspace_id: None,
                title_override: None,
                archived: false,
            })
    }

    /// Workspace 注册表（PA-079）。
    pub fn list_workspaces(&self) -> Vec<crate::agent::workspace::WorkspaceRecord> {
        self.workspaces.clone()
    }

    pub fn create_workspace(
        &mut self,
        name: &str,
        root_path: &str,
    ) -> Result<crate::agent::workspace::WorkspaceRecord, String> {
        let record =
            crate::agent::workspace::create_workspace_entry(&mut self.workspaces, name, root_path)?;
        self.save_to_backend();
        Ok(record)
    }

    /// 重命名工作区（侧边栏三级树）：校验与 id/root 不变性由 workspace 域负责，
    /// 成功即全量落盘。未知 id 返回错误。
    pub fn rename_workspace(
        &mut self,
        workspace_id: &str,
        name: &str,
    ) -> Result<crate::agent::workspace::WorkspaceRecord, String> {
        let record = crate::agent::workspace::rename_workspace_entry(&mut self.workspaces, workspace_id, name)?;
        self.save_to_backend();
        Ok(record)
    }

    /// 删除工作区注册（仅注册表语义）：default 与未知 id 拒绝；名下会话的
    /// workspace_id 在同一次落盘内批量重写为 default——附件导入与工具根两条
    /// 运行时路径随之收敛到默认工作区，不再出现"硬失败 vs 静默换根"分叉。
    pub fn delete_workspace(&mut self, workspace_id: &str) -> Result<(), String> {
        crate::agent::workspace::delete_workspace_entry(&mut self.workspaces, workspace_id)?;
        for session in self.sessions.values_mut() {
            if session.workspace_id.as_deref() == Some(workspace_id) {
                session.workspace_id = Some(crate::agent::workspace::DEFAULT_WORKSPACE_ID.to_string());
            }
        }
        self.save_to_backend();
        Ok(())
    }

    /// 会话重命名：写入独立 override 字段（每轮派生刷新不再冲掉）。
    /// 校验：trim 后非空、≤64 字符；会话必须已存在（不隐式创建）；同值重命名
    /// 视为成功 no-op。
    pub fn rename_session(&mut self, session_id: &str, title: &str) -> Result<(), String> {
        let trimmed = crate::agent::workspace::validate_display_name(title)
            .map_err(|error| format!("会话重命名非法：{error}"))?;
        let session = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| format!("会话不存在：{session_id}"))?;
        if session.title_override.as_deref() != Some(trimmed.as_str()) {
            session.title_override = Some(trimmed);
            self.save_to_backend();
        }
        Ok(())
    }

    /// 归档会话：置 archived 标记并落盘；幂等（重复归档为 no-op 成功）。
    /// 日志、数据与工作区账户位不动。
    pub fn archive_session(&mut self, session_id: &str) -> Result<(), String> {
        let session = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| format!("会话不存在：{session_id}"))?;
        if !session.archived {
            session.archived = true;
            self.save_to_backend();
        }
        Ok(())
    }

    pub fn resolve_workspace_root(&self, workspace_id: Option<&str>) -> Result<String, String> {
        crate::agent::workspace::resolve_workspace_root(&self.workspaces, workspace_id)
    }

    /// 读取会话的权威 workspace 归属（不创建、不落盘）。None = 尚未盖章。
    pub fn read_workspace_owner(&self, session_id: &str) -> Option<String> {
        self.sessions
            .get(session_id)
            .and_then(|session| session.workspace_id.clone())
    }

    /// 首次持久化盖章 workspace_id（PA-079）：会话 workspace_id 为 None 时写入并落盘；
    /// 已盖章则 no-op（幂等，不影响既有会话的后续轮）。
    /// **先 `ensure_session` 再盖章**：turn 提交时全新会话尚未被 `prepare_turn` 创建，
    /// 若只对已存在会话盖章，首轮 workspace_id 会丢失。
    pub fn stamp_workspace_id(&mut self, session_id: &str, workspace_id: &str) {
        // 纵深防御（三级树裁决②）：非 default 且未注册的 id 一律归一为
        // default——工作区删除后，携带死 id 的陈旧/竞态提交不得把会话重新
        // 盖成孤儿（附件导入硬失败 vs 工具根软回退的分叉随之不可达）。
        let normalized = if workspace_id == crate::agent::workspace::DEFAULT_WORKSPACE_ID
            || self.workspaces.iter().any(|record| record.id == workspace_id)
        {
            workspace_id.to_string()
        } else {
            // L-C（ADR 0015）：id 来自外部输入——压平换行并截断，防日志伪造。
            let safe_echo: String = workspace_id
                .chars()
                .map(|ch| if ch.is_control() { ' ' } else { ch })
                .take(80)
                .collect();
            eprintln!(
                "[pony-agent] stamp_workspace_id: 未注册的 workspace '{safe_echo}'，归一为 default"
            );
            crate::agent::workspace::DEFAULT_WORKSPACE_ID.to_string()
        };
        let changed = {
            let session = self.ensure_session(session_id);
            if session.workspace_id.is_none() {
                session.workspace_id = Some(normalized);
                true
            } else {
                false
            }
        };
        if changed {
            self.save_to_backend();
        }
    }

    /// 共享路径授权清单（PA-080）：host control plane 与工具执行器共享同一 `Arc`。
    pub fn path_authorizations(&self) -> Arc<crate::agent::path_permission::AuthorizeStore> {
        Arc::clone(&self.path_authorizations)
    }

    /// 显式授权一个路径（仅读；目标必须存在）。变更即持久化（store_metadata
    /// key=`path_authorizations.v1`），重启后生效。
    pub fn authorize_path(
        &mut self,
        canonical: PathBuf,
    ) -> Result<crate::agent::path_permission::AuthorizedPathEntry, String> {
        let entry = self.path_authorizations.grant(canonical)?;
        self.save_to_backend();
        Ok(entry)
    }

    /// 撤销授权（精确路径；子授权保留）。返回是否实际移除。
    pub fn revoke_authorization(&mut self, canonical: &Path) -> bool {
        let removed = self.path_authorizations.revoke(canonical);
        if removed {
            self.save_to_backend();
        }
        removed
    }

    /// 列出全部授权条目。
    pub fn list_authorizations(&self) -> Vec<crate::agent::path_permission::AuthorizedPathEntry> {
        self.path_authorizations.entries()
    }

    /// PA-093：读取会话事件流（backend 支持时）；否则空。
    pub fn load_turn_events(
        &self,
        session_id: &str,
        up_to_seq: Option<u64>,
    ) -> Vec<(u64, String, crate::agent::turn_event::TurnEvent)> {
        self.backend.load_turn_events(session_id, up_to_seq)
    }

    /// PA-094：按引用加载 build_context_observation 全量 payload（大字段外置）。
    pub fn load_build_context_observation(
        &self,
        session_id: &str,
        observation_ref: &str,
    ) -> Option<crate::agent::provider::BuildContextObservation> {
        self.backend
            .load_build_context_observation(session_id, observation_ref)
    }

    /// PA-093：当前事件水位（backend 支持时）；否则 0。
    pub fn load_event_watermark(&self, session_id: &str) -> u64 {
        self.backend.load_event_watermark(session_id)
    }

    /// PA-095：事件 schema 版本校验（宿主读面入口；不匹配 Err 上抛）。
    pub fn validate_event_schema(&self) -> Result<u64, String> {
        self.backend.validate_event_schema()
    }

    /// PA-095：带契约校验的事件加载（坏 payload / 版本不匹配 fail loud）。
    /// 失败时标记会话 event-stream-degraded（宿主可经 `is_event_stream_degraded`
    /// 查询），错误经宿主 load API 上抛——不静默跳过。
    pub fn load_turn_events_checked(
        &self,
        session_id: &str,
        up_to_seq: Option<u64>,
    ) -> Result<Vec<(u64, String, crate::agent::turn_event::TurnEvent)>, String> {
        match self.backend.load_turn_events_checked(session_id, up_to_seq) {
            Ok(events) => Ok(events),
            Err(error) => {
                self.event_stream_degraded
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(session_id.to_string());
                Err(error)
            }
        }
    }

    /// PA-095：会话事件流是否已标记 degraded（坏 payload / 版本不匹配后置位；
    /// 内存态，重启后随首次读取重建）。
    pub fn is_event_stream_degraded(&self, session_id: &str) -> bool {
        self.event_stream_degraded
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(session_id)
    }

    /// PA-093：当前 active 分支（事件落列用）；会话不存在时 None。
    pub fn session_active_branch_id(&self, session_id: &str) -> Option<String> {
        self.sessions.get(session_id).and_then(|session| {
            session
                .history_cursor
                .active_branch_id
                .clone()
                .or_else(|| Some(DEFAULT_HISTORY_BRANCH_ID.to_string()))
        })
    }

    /// PA-093：flush 成功后同步会话水位 + 将 branch head 节点升级为引用化
    /// （commit 先于 flush 的时序下，节点先以 legacy 快照提交，事件落盘后
    /// 清空快照并记录事件区间——flush 失败则节点保持 legacy，数据不丢）。
    /// 幂等：水位未增长或无新节点时无操作。
    pub fn finalize_event_watermark(&mut self, session_id: &str, _turn_id: &str) {
        let Some(session) = self.sessions.get_mut(session_id) else {
            return;
        };
        let new_watermark = self.backend.load_event_watermark(session_id);
        if new_watermark <= session.event_watermark {
            return;
        }
        session.event_watermark = new_watermark;
        // PA-095 #6：写入点统一——cursor_version 镜像水位（wire 字段名不变）。
        session.history_cursor.cursor_version = new_watermark;
        session.history_cursor.event_watermark = new_watermark;
        // branch head 节点 = 最近一次 commit 的节点（单线程 turn 流程保证）。
        if let Some(head_id) = session.history_cursor.branch_head_node_id.clone() {
            if let Some(node) = session
                .history_nodes
                .iter_mut()
                .find(|n| n.node_id == head_id)
            {
                if !_turn_id.is_empty() {
                    node.turn_id = Some(_turn_id.to_string());
                }
                if node.event_seq_range.is_none() && new_watermark > session.last_commit_watermark {
                    node.event_seq_range = Some((session.last_commit_watermark, new_watermark - 1));
                    // 引用化：事件可重建的视图字段不再内嵌快照。
                    node.history.clear();
                    node.provider_native_transcript.clear();
                    node.turn_trace_history.clear();
                }
            }
        }
        session.last_commit_watermark = new_watermark;
    }

    /// PA-093：折叠事件流到节点区间终点，得到会话视图（history/trace）。
    /// 优先命中投影缓存（节点区间不可变 → 缓存天然有效）；未命中时全量重折叠。
    fn fold_node_views(
        &mut self,
        session_id: &str,
        node: &HistoryNode,
    ) -> (Vec<TurnHistoryMessage>, Vec<TurnTraceRecord>) {
        let Some((_start, end)) = node.event_seq_range else {
            return (node.history.clone(), node.turn_trace_history.clone());
        };
        if let Some((cached_end, history, trace)) = self.projection_cache.get(&node.node_id) {
            if *cached_end == end {
                return (history.clone(), trace.clone());
            }
        }
        let events = self.backend.load_turn_events(session_id, Some(end));
        if events.is_empty() {
            eprintln!(
                "[pony-agent][session] event stream empty for referenced node {} (backend without event support?); falling back to empty view",
                node.node_id
            );
        }
        let (history, trace) = {
            let session = self.sessions.get(session_id).expect("session exists");
            fold_session_views(
                &events,
                &node.node_id,
                &session.history_nodes,
                &session.history_branches,
            )
        };
        self.projection_cache
            .insert(node.node_id.clone(), (end, history.clone(), trace.clone()));
        (history, trace)
    }

    /// PA-091：turn 终态 flush 事件批次（与快照写入同事务，由 backend 保证）。
    /// 返回是否成功；失败时调用方只记日志（contained，不阻断流）。
    pub fn persist_events(
        &self,
        session_id: &str,
        turn_id: &str,
        branch_id: &str,
        events: Vec<crate::agent::turn_event::TurnEvent>,
    ) -> bool {
        if events.is_empty() {
            return true;
        }
        let outcome = self.backend.persist_command(PersistCommand::FlushEvents {
            epoch: 1,
            session_id: session_id.to_string(),
            turn_id: turn_id.to_string(),
            branch_id: branch_id.to_string(),
            events,
        });
        matches!(outcome, PersistCommandOutcome::Succeeded)
    }

    /// PA-095：后端是否具备事件持久化能力（Memory/File 为 false）。
    pub fn supports_turn_events(&self) -> bool {
        self.backend.supports_turn_events()
    }

    /// PA-095 #6（实施后审核 P1）：当前事件水位的只读访问（响应 cursor 版本
    /// 重建用；不触发 load 路径的 ensure/save 副作用）。会话不存在返回 None。
    pub fn current_event_watermark(&self, session_id: &str) -> Option<u64> {
        self.sessions
            .get(session_id)
            .map(|session| session.event_watermark)
    }

    /// PA-095 #7（实施后审核 P2）：把该 turn 已外置的 observation 引用附加到
    /// 内存 trace 缓存行——异步 trace worker 整行 REPLACE 时会带上引用，
    /// 避免覆盖掉 flush 同事务回填的 ref。幂等（已有 ref 不覆盖）。
    pub fn attach_observation_refs_to_turn_trace(&mut self, session_id: &str, turn_id: &str) {
        if !self.backend.supports_turn_events() {
            return;
        }
        let prefix = format!("bco:{turn_id}:");
        let reference = match self
            .backend
            .load_observation_refs(session_id)
            .into_iter()
            .find(|reference| reference.starts_with(&prefix))
        {
            Some(reference) => reference,
            None => return,
        };
        if let Some(session) = self.sessions.get_mut(session_id) {
            if let Some(trace) = session
                .turn_trace_history
                .iter_mut()
                .find(|trace| trace.turn_id == turn_id)
            {
                if trace.build_context_observation_ref.is_none() {
                    trace.build_context_observation_ref = Some(reference);
                }
            }
        }
    }

    pub(super) fn save_to_backend(&self) {
        if matches!(
            self.backend.trace_storage_mode(),
            SeparateTraceTableMode::WriteSeparate
        ) {
            for (session_id, session) in &self.sessions {
                if matches!(
                    session.trace_migration_state,
                    TraceMigrationState::TraceTableAuthoritative
                ) {
                    // PA-088：写表用全量 Union（顶层 ∪ 全部节点 trace，按 turn_id 去重取最新）。
                    // 仅写顶层会导致 >24 轮会话的旧节点 refs 在重启 materialize 时解析不到
                    // （表被 prune 到 24 行），节点 trace 丢失。
                    let union = collect_trace_union(session);
                    let _ = self.backend.replace_session_traces(session_id, &union);
                }
            }
        }
        let trace_mode = self.backend.trace_storage_mode();
        let store = PersistedStore {
            sessions: self
                .sessions
                .iter()
                .filter(|(_, session)| session_is_persistable(session))
                .map(|(session_id, session)| {
                    (
                        session_id.clone(),
                        session_state_for_backend(session, trace_mode),
                    )
                })
                .collect::<SessionMap>(),
            attachment_assets: self.attachment_assets.clone(),
            session_attachment_index: self.session_attachment_index.clone(),
            mcp_source_snapshots: self.mcp_source_snapshots.clone(),
            skill_source_snapshots: self.skill_source_snapshots.clone(),
            workspaces: self.workspaces.clone(),
            path_authorizations: self.path_authorizations.entries(),
        };
        self.backend.save_store(&store);
    }

    fn save_session_to_backend(&self, session_id: &str) {
        let Some(session) = self.sessions.get(session_id) else {
            return;
        };
        self.save_session_state_to_backend(session_id, session);
    }

    fn save_session_state_to_backend(&self, session_id: &str, session: &SessionState) {
        let prepared = session_state_for_backend(session, self.backend.trace_storage_mode());
        if matches!(
            prepared.trace_migration_state,
            TraceMigrationState::TraceTableAuthoritative
        ) && matches!(
            self.backend.persist_session_with_trace_mutation(
                session_id,
                &prepared,
                // PA-090：Authoritative 会话 ReplaceAll 用全量 Union（同主事务路径）。
                SessionTraceMutation::ReplaceAll {
                    traces: collect_trace_union(session),
                },
            ),
            SessionBackendMutationResult::Succeeded
        ) {
            return;
        }
        if session_is_persistable(session) && self.backend.upsert_session(session_id, &prepared) {
            return;
        }
        let mut sessions_map = self.sessions.clone();
        sessions_map.insert(session_id.to_string(), session.clone());
        let trace_mode = self.backend.trace_storage_mode();
        let store = PersistedStore {
            sessions: sessions_map
                .into_iter()
                .filter(|(_, session)| session_is_persistable(session))
                .map(|(session_id, session)| {
                    (
                        session_id,
                        session_state_for_backend(&session, trace_mode),
                    )
                })
                .collect::<SessionMap>(),
            attachment_assets: self.attachment_assets.clone(),
            session_attachment_index: self.session_attachment_index.clone(),
            mcp_source_snapshots: self.mcp_source_snapshots.clone(),
            skill_source_snapshots: self.skill_source_snapshots.clone(),
            workspaces: self.workspaces.clone(),
            path_authorizations: self.path_authorizations.entries(),
        };
        self.backend.save_store(&store);
    }

    fn persist_session_and_trace_change(
        &mut self,
        session_id: &str,
        mutation: SessionTraceMutation,
    ) {
        let Some(session) = self.sessions.get(session_id) else {
            return;
        };
        let prepared = session_state_for_backend(session, self.backend.trace_storage_mode());
        // PA-090：Authoritative 会话的 ReplaceAll 改用全量 Union——
        // 否则用顶层 trace（24 行截断）替换表会清空节点 refs 指向的 trace，
        // 重启 materialize 全部 missing（checkout/restore/fork/switch 均走此路径）。
        let effective_mutation = if matches!(
            prepared.trace_migration_state,
            TraceMigrationState::TraceTableAuthoritative
        ) && matches!(mutation, SessionTraceMutation::ReplaceAll { .. })
        {
            SessionTraceMutation::ReplaceAll {
                traces: collect_trace_union(session),
            }
        } else {
            mutation.clone()
        };
        // PA-089 阶段 3 接线：Authoritative 会话的增量 trace mutation 走 PersistCommand
        // 增量命令（AppendTrace/UpdateTraceTerminal/AppendHookRecords）——只写变化行，
        // 替代全量 persist_session_with_trace_mutation（每次全量序列化 blob + 重建表，
        // 是"生成结束后卡顿"的根因：completed 后全量写 3.77MB blob + 全部 trace 行）。
        let is_authoritative = matches!(
            prepared.trace_migration_state,
            TraceMigrationState::TraceTableAuthoritative
        );
        if is_authoritative && session_is_persistable(session) {
            let command = match &effective_mutation {
                SessionTraceMutation::UpsertOne { trace, trace_order } => {
                    Some(PersistCommand::AppendTrace {
                        epoch: 1,
                        session_id: session_id.to_string(),
                        trace: trace.clone(),
                        trace_order: *trace_order,
                    })
                }
                SessionTraceMutation::UpdateTerminalEvent {
                    turn_id,
                    event_id,
                    event_type,
                    event_version,
                    sequence,
                    emitted_at_ms,
                    updated_at,
                    event_watermark,
                } => Some(PersistCommand::UpdateTraceTerminal {
                    epoch: 1,
                    session_id: session_id.to_string(),
                    turn_id: turn_id.clone(),
                    terminal_patch: TraceTerminalPatch {
                        event_id: event_id.clone(),
                        event_type: event_type.clone(),
                        event_version: event_version.clone(),
                        sequence: *sequence,
                        emitted_at_ms: *emitted_at_ms,
                        updated_at: *updated_at,
                        phase: None,
                        event_watermark: *event_watermark,
                    },
                }),
                SessionTraceMutation::AppendHookRecords {
                    turn_id,
                    hook_trace_records,
                    updated_at,
                } => Some(PersistCommand::AppendHookRecords {
                    epoch: 1,
                    session_id: session_id.to_string(),
                    turn_id: turn_id.clone(),
                    hook_trace_records: hook_trace_records.clone(),
                    updated_at: *updated_at,
                }),
                _ => None,
            };
            if let Some(command) = command {
                let outcome = self.backend.persist_command(command);
                if matches!(outcome, PersistCommandOutcome::Succeeded) {
                    return;
                }
                // 命令失败 → 回退全量路径（不丢数据）
            }
        }
        let result = if session_is_persistable(session) {
            self.backend.persist_session_with_trace_mutation(
                session_id,
                &prepared,
                effective_mutation,
            )
        } else {
            SessionBackendMutationResult::Unsupported
        };

        if matches!(result, SessionBackendMutationResult::Succeeded) {
            if matches!(
                self.backend.trace_storage_mode(),
                SeparateTraceTableMode::WriteSeparate
            ) {
                if let Some(session) = self.sessions.get_mut(session_id) {
                    // PA-088：按原状态分派晋升——存量 LegacyBlob 首次写 → DualWrite
                    // （保留 blob 双写兜底窗口，不直接表权威）；DualWrite 保持；
                    // 新会话（ensure_session 已置 Authoritative）保持。
                    session.trace_migration_state = match session.trace_migration_state {
                        TraceMigrationState::LegacyBlob => TraceMigrationState::DualWrite,
                        TraceMigrationState::DualWrite => TraceMigrationState::DualWrite,
                        TraceMigrationState::TraceTableAuthoritative => {
                            TraceMigrationState::TraceTableAuthoritative
                        }
                    };
                }
            }
            return;
        }

        if matches!(result, SessionBackendMutationResult::NotFound)
            && matches!(
                prepared.trace_migration_state,
                TraceMigrationState::TraceTableAuthoritative
            )
        {
            let replace_all = SessionTraceMutation::ReplaceAll {
                // PA-090：Authoritative 会话 NotFound 重试同样用全量 Union。
                traces: collect_trace_union(session),
            };
            if matches!(
                self.backend.persist_session_with_trace_mutation(
                    session_id,
                    &prepared,
                    replace_all
                ),
                SessionBackendMutationResult::Succeeded
            ) {
                return;
            }
        }

        if matches!(result, SessionBackendMutationResult::Failed) {
            eprintln!(
                "[pony-agent][session] persist_session_and_trace_change backend write failed for session {}; falling back to separate blob+table writes",
                session_id
            );
        }

        self.persist_trace_change(session_id, trace_action_from_mutation(mutation));
        self.save_session_to_backend(session_id);
    }

    fn persist_trace_change(&self, session_id: &str, action: TracePersistenceAction) {
        let result = match action {
            TracePersistenceAction::ReplaceAll => self
                .sessions
                .get(session_id)
                .map(|session| {
                    // PA-088：ReplaceAll 用全量 Union（顶层 ∪ 节点 trace），
                    // 避免用内存顶层（24 行截断）替换表导致节点 refs 解析不到。
                    let union = collect_trace_union(session);
                    self.backend.replace_session_traces(session_id, &union)
                })
                .unwrap_or(SessionBackendMutationResult::Unsupported),
            TracePersistenceAction::UpsertOne { trace, trace_order } => self
                .backend
                .upsert_turn_trace(session_id, &trace, trace_order),
            TracePersistenceAction::UpdateTerminalEvent {
                turn_id,
                event_id,
                event_type,
                event_version,
                sequence,
                emitted_at_ms,
                updated_at,
            } => self.backend.update_turn_trace_terminal_event(
                session_id,
                &turn_id,
                event_id.as_deref(),
                event_type.as_deref(),
                event_version.as_deref(),
                sequence,
                emitted_at_ms,
                updated_at,
            ),
            TracePersistenceAction::AppendHookRecords {
                turn_id,
                hook_trace_records,
                updated_at,
            } => self.backend.append_turn_trace_hook_records(
                session_id,
                &turn_id,
                &hook_trace_records,
                updated_at,
            ),
        };

        if matches!(
            result,
            SessionBackendMutationResult::Failed | SessionBackendMutationResult::NotFound
        ) {
            eprintln!(
                "[pony-agent][session] trace-level persistence {:?} for session {}",
                result, session_id
            );
        }
    }

    pub(super) fn snapshot_for_session(&self, session_id: &str) -> SessionSnapshot {
        self.snapshot_for_session_at(session_id, None)
    }

    pub(super) fn snapshot_for_session_at(&self, session_id: &str, node_id: Option<&str>) -> SessionSnapshot {
        let session = self
            .sessions
            .get(session_id)
            .expect("session must exist before snapshot");
        let attachment_assets = attachment_assets_for_query(
            &self.sessions,
            &self.attachment_assets,
            &self.session_attachment_index,
            &self.attachment_root,
            &AttachmentAssetQuery {
                session_id: Some(session_id.to_string()),
                ..AttachmentAssetQuery::default()
            },
            now_timestamp_ms(),
        );
        // PA-093：时间旅行——引用化节点由事件流折叠重建视图（克隆会话，不污染
        // 当前内存态；legacy 节点走 snapshot_from_state 内嵌快照路径）。
        if let Some(nid) = node_id {
            let is_referenced = history_node(session, nid)
                .map(|node| node.event_seq_range.is_some())
                .unwrap_or(false);
            if is_referenced {
                let mut view = session.clone();
                let node = history_node(&view, nid)
                    .cloned()
                    .expect("node exists after check");
                let (history, trace) = fold_session_views(
                    &self
                        .backend
                        .load_turn_events(session_id, node.event_seq_range.map(|(_, e)| e)),
                    &node.node_id,
                    &view.history_nodes,
                    &view.history_branches,
                );
                hydrate_session_from_projection(&mut view, &node, history, trace);
                // 节点视角 cursor（与 snapshot_from_state 节点路径语义对齐），
                // 然后走会话路径组装（折叠视图在 session 字段，非节点快照）。
                let branch_head_node_id = view
                    .history_branches
                    .iter()
                    .find(|b| b.branch_id == node.branch_id)
                    .and_then(|b| b.head_node_id.clone());
                view.history_cursor.visible_node_id = Some(node.node_id.clone());
                view.history_cursor.active_branch_id = Some(node.branch_id.clone());
                view.history_cursor.branch_head_node_id = branch_head_node_id.clone();
                view.history_cursor.workspace_node_id = Some(node.node_id.clone());
                view.history_cursor.mode =
                    if branch_head_node_id.as_deref() == Some(node.node_id.as_str()) {
                        HistoryCursorMode::Live
                    } else {
                        HistoryCursorMode::Historical
                    };
                view.history_cursor.checkout_mode = HistoryCheckoutMode::TranscriptOnly;
                view.history_cursor.checkout_status = HistoryCheckoutStatus::NotRequested;
                return snapshot_from_state(&view, attachment_assets, None);
            }
        }
        snapshot_from_state(session, attachment_assets, node_id)
    }

    pub(super) fn refresh_attachment_catalog(&mut self) {
        self.attachment_assets = rebuild_attachment_assets(
            &self.sessions,
            &self.attachment_assets,
            &self.attachment_root,
        );
        self.session_attachment_index = rebuild_session_attachment_index(&self.sessions);
    }

    fn remove_session_attachment_catalog(&mut self, session_id: &str) {
        let removed_asset_ids = self
            .session_attachment_index
            .remove(session_id)
            .unwrap_or_default()
            .into_iter()
            .collect::<HashSet<_>>();
        self.attachment_assets.retain(|asset_id, asset| {
            asset.session_id != session_id && !removed_asset_ids.contains(asset_id)
        });
    }
}
/// 从 trace_timeline 提取最终 assistant 消息的 reasoning_content。
/// 取最后一条 `call_model` 类型 timeline entry 的 reasoning_content，
/// 与前端 traceReasoningContent 逻辑一致。
fn extract_reasoning_content(trace: &TurnTraceRecord) -> Option<String> {
    trace
        .trace_timeline
        .iter()
        .rev()
        .find(|e| e.kind == "call_model")
        .and_then(|e| e.reasoning_content.clone())
}

/// TurnTraceRecord.phase → MessageStatus 映射
fn derive_status_from_trace(trace: &TurnTraceRecord) -> MessageStatus {
    match trace.phase.as_str() {
        "completed" => MessageStatus::Done,
        "failed" | "cancelled" => MessageStatus::Error,
        _ => MessageStatus::Error,
    }
}

/// 用 turn_trace_history 中的元数据补全 TurnHistoryMessage 列表。
/// 使用末端对齐策略处理 history 与 turn_trace_history 截断步长不一致的问题。
/// 幂等：已携带元数据的条目不覆写。
pub(super) fn enrich_history_from_traces(
    history: &mut [TurnHistoryMessage],
    turn_trace_history: &[TurnTraceRecord],
) {
    let msg_count = history.len();
    let trace_count = turn_trace_history.len();
    if msg_count == 0 || trace_count == 0 {
        return;
    }
    let offset = trace_count.saturating_sub(msg_count / 2);

    for (i, msg) in history.iter_mut().enumerate() {
        let turn_idx = i / 2;
        let trace_idx = turn_idx + offset;

        if msg.turn_id.is_some() {
            continue; // 幂等：跳过已携带元数据的条目
        }

        if let Some(trace) = turn_trace_history.get(trace_idx) {
            if msg.role == "assistant" {
                msg.turn_id = Some(trace.turn_id.clone());
                msg.model_name = trace.provider_model.clone();
                msg.token_count = trace.output_tokens;
                msg.reasoning_content = extract_reasoning_content(trace);
                msg.status = Some(derive_status_from_trace(trace));
            } else if msg.role == "user" {
                msg.turn_id = Some(trace.turn_id.clone());
            }
        }
    }
}

/// 会话尚不存在时的只读默认快照（与 ensure_session 默认结构一致，但不创建、不落盘）。
fn default_snapshot_for_session(session_key: &str, node_id: Option<&str>) -> SessionSnapshot {
    let session = SessionState {
        conversation_id: session_key.to_string(),
        title: default_session_title(),
        summary: DEFAULT_SESSION_SUMMARY.to_string(),
        history: Vec::new(),
        provider_native_transcript: Vec::new(),
        turn_trace_history: Vec::new(),
        trace_migration_state: TraceMigrationState::LegacyBlob,
        turn_trace_refs: None,
        long_term_memory_entries: Vec::new(),
        memory_write_evidence: Vec::new(),
        memory_write_hook_trace_records: Vec::new(),
        history_state_evidence: Vec::new(),
        turn_count: 0,
        last_referenced_file: None,
        updated_at_ms: 0,
        history_nodes: Vec::new(),
        history_branches: Vec::new(),
        history_cursor: HistoryCursor {
            session_id: session_key.to_string(),
            ..HistoryCursor::default()
        },
        event_watermark: 0,
        last_commit_watermark: 0,
        workspace_id: None,
        title_override: None,
        archived: false,
    };
    snapshot_from_state(&session, Vec::new(), node_id)
}

fn snapshot_from_state(
    session: &SessionState,
    attachment_assets: Vec<AttachmentAsset>,
    node_id: Option<&str>,
) -> SessionSnapshot {
    let latest_node_id = session
        .history_branches
        .iter()
        .find(|branch| {
            session
                .history_cursor
                .active_branch_id
                .as_deref()
                .map(|active| active == branch.branch_id)
                .unwrap_or(branch.branch_id == DEFAULT_HISTORY_BRANCH_ID)
        })
        .and_then(|branch| branch.head_node_id.clone())
        .or_else(|| {
            session
                .history_nodes
                .last()
                .map(|node| node.node_id.clone())
        });

    if let Some(selected_node) = node_id.and_then(|id| history_node(session, id)) {
        let branch_head_node_id = session
            .history_branches
            .iter()
            .find(|branch| branch.branch_id == selected_node.branch_id)
            .and_then(|branch| branch.head_node_id.clone());
        let checkout_status = if session.history_cursor.visible_node_id.as_deref()
            == Some(selected_node.node_id.as_str())
        {
            session.history_cursor.checkout_status.clone()
        } else {
            HistoryCheckoutStatus::NotRequested
        };
        let history_cursor = HistoryCursor {
            session_id: session.conversation_id.clone(),
            visible_node_id: Some(selected_node.node_id.clone()),
            active_branch_id: Some(selected_node.branch_id.clone()),
            branch_head_node_id: branch_head_node_id.clone(),
            workspace_node_id: Some(selected_node.node_id.clone()),
            cursor_version: session.history_cursor.cursor_version,
            event_watermark: session.history_cursor.event_watermark,
            mode: if branch_head_node_id.as_deref() == Some(selected_node.node_id.as_str()) {
                HistoryCursorMode::Live
            } else {
                HistoryCursorMode::Historical
            },
            checkout_mode: HistoryCheckoutMode::TranscriptOnly,
            checkout_status,
        };
        let mut enriched_history = selected_node.history.clone();
        enrich_history_from_traces(&mut enriched_history, &selected_node.turn_trace_history);
        return SessionSnapshot {
            conversation_id: session.conversation_id.clone(),
            // 顶层标题走单一投影点：override 优先于节点冻结的派生标题
            // （历史节点列表本身仍保留提交时刻标题，见 project_lightweight_nodes）。
            title: session.effective_title().to_string(),
            summary: selected_node.summary.clone(),
            history: enriched_history,
            attachment_assets,
            provider_native_transcript: selected_node.provider_native_transcript.clone(),
            turn_trace_history: selected_node.turn_trace_history.clone(),
            long_term_memory_entries: selected_node.long_term_memory_entries.clone(),
            memory_write_evidence: selected_node.memory_write_evidence.clone(),
            memory_write_hook_trace_records: selected_node.memory_write_hook_trace_records.clone(),
            history_state_evidence: session.history_state_evidence.clone(),
            history_state_audit_summary: build_history_state_audit_summary(
                &history_cursor,
                &session.history_state_evidence,
            ),
            run_control_audit_summary: build_missing_run_control_audit_summary(),
            turn_count: selected_node.turn_count,
            last_referenced_file: selected_node.last_referenced_file.clone(),
            updated_at_ms: session.updated_at_ms,
            // PA-088：轻量投影——节点不携带完整 trace（避免 IPC payload 膨胀导致前端
            // JSON.parse 卡死）；选中节点的 trace 已放快照顶层 turn_trace_history。
            history_nodes: project_lightweight_nodes(&session.history_nodes),
            history_branches: session.history_branches.clone(),
            history_cursor,
            resolved_node_id: Some(selected_node.node_id.clone()),
            latest_node_id,
            env_info: Some(collect_env_info()),
            workspace_id: session.workspace_id.clone(),
        };
    }

    let mut enriched_history = session.history.clone();
    enrich_history_from_traces(&mut enriched_history, &session.turn_trace_history);
    SessionSnapshot {
        conversation_id: session.conversation_id.clone(),
        title: session.effective_title().to_string(),
        summary: session.summary.clone(),
        history: enriched_history,
        attachment_assets,
        provider_native_transcript: session.provider_native_transcript.clone(),
        turn_trace_history: session.turn_trace_history.clone(),
        long_term_memory_entries: session.long_term_memory_entries.clone(),
        memory_write_evidence: session.memory_write_evidence.clone(),
        memory_write_hook_trace_records: session.memory_write_hook_trace_records.clone(),
        history_state_evidence: session.history_state_evidence.clone(),
        history_state_audit_summary: build_history_state_audit_summary(
            &session.history_cursor,
            &session.history_state_evidence,
        ),
        run_control_audit_summary: build_missing_run_control_audit_summary(),
        turn_count: session.turn_count,
        last_referenced_file: session.last_referenced_file.clone(),
        updated_at_ms: session.updated_at_ms,
        // PA-088：轻量投影（同选中节点路径）。
        history_nodes: project_lightweight_nodes(&session.history_nodes),
        history_branches: session.history_branches.clone(),
        history_cursor: session.history_cursor.clone(),
        resolved_node_id: session.history_cursor.visible_node_id.clone(),
        latest_node_id,
        env_info: Some(collect_env_info()),
        workspace_id: session.workspace_id.clone(),
    }
}

fn build_history_state_audit_summary(
    cursor: &HistoryCursor,
    evidence: &[HistoryStateHookEvidence],
) -> HistoryStateAuditSummary {
    let action = evidence.last().map_or_else(
        || HistoryStateAuditActionSummary {
            status: "missing".to_string(),
            source_family: "history_state".to_string(),
            command_kind: None,
            boundary: None,
            result_kind: None,
            summary: "history-state audit summary unavailable".to_string(),
            elapsed_ms: None,
            blocked: false,
            degraded: false,
            evidence_id: None,
            observed_at_ms: None,
            requested_node_id: None,
            requested_branch_id: None,
            resolved_node_id: None,
            resolved_branch_id: None,
        },
        |latest| HistoryStateAuditActionSummary {
            status: "available".to_string(),
            source_family: "history_state".to_string(),
            command_kind: Some(latest.command_kind.clone()),
            boundary: Some(latest.boundary.clone()),
            result_kind: Some(latest.result_kind.clone()),
            summary: latest.summary.clone(),
            elapsed_ms: Some(latest.elapsed_ms),
            blocked: latest.blocked,
            degraded: latest.degraded,
            evidence_id: Some(latest.evidence_id.clone()),
            observed_at_ms: Some(latest.recorded_at_ms),
            requested_node_id: latest.requested_node_id.clone(),
            requested_branch_id: latest.requested_branch_id.clone(),
            resolved_node_id: latest.resolved_node_id.clone(),
            resolved_branch_id: latest.resolved_branch_id.clone(),
        },
    );

    HistoryStateAuditSummary {
        action,
        current_context: HistoryStateAuditCurrentContext {
            mode: history_cursor_mode_label(&cursor.mode).to_string(),
            visible_node_id: cursor.visible_node_id.clone(),
            active_branch_id: cursor.active_branch_id.clone(),
            branch_head_node_id: cursor.branch_head_node_id.clone(),
            workspace_node_id: cursor.workspace_node_id.clone(),
        },
    }
}

pub fn build_missing_run_control_audit_summary() -> RunControlAuditSummary {
    RunControlAuditSummary {
        action_evidence_summary: RunControlAuditActionSummary {
            status: "missing".to_string(),
            source_family: "run_control".to_string(),
            command_kind: None,
            boundary: None,
            result_kind: None,
            summary: "run-control audit summary unavailable".to_string(),
            target_summary: "target unavailable".to_string(),
            elapsed_ms: None,
            blocked: false,
            degraded: false,
            evidence_id: None,
            observed_at_ms: None,
            run_id: None,
            turn_id: None,
            checkpoint_turn_id: None,
            checkpoint_kind: None,
            recovery_mode: None,
            projected_command: None,
            degradation_reason: None,
            request_summary: None,
            start_reason: None,
        },
        current_context_projection: RunControlAuditCurrentContext {
            phase: "idle".to_string(),
            checkpoint_status: "missing".to_string(),
            active_run_id: None,
            checkpoint_kind: None,
            checkpoint_recovery_mode: None,
            submission_plan_command: None,
        },
    }
}

fn ensure_history_graph(session: &mut SessionState) -> bool {
    let mut changed = false;
    if session.history_cursor.session_id.is_empty() {
        session.history_cursor.session_id = session.conversation_id.clone();
        changed = true;
    }
    if session.history_branches.is_empty() {
        session.history_branches.push(HistoryBranch {
            branch_id: DEFAULT_HISTORY_BRANCH_ID.to_string(),
            session_id: session.conversation_id.clone(),
            base_node_id: None,
            head_node_id: None,
            forked_from_branch_id: None,
            forked_from_node_id: None,
            label: "main".to_string(),
            created_at_ms: session.updated_at_ms,
            updated_at_ms: session.updated_at_ms,
        });
        changed = true;
    }
    if session.history_nodes.is_empty() && !session.history.is_empty() {
        session.history_nodes.push(HistoryNode {
            node_id: legacy_history_root_node_id(session),
            session_id: session.conversation_id.clone(),
            parent_node_id: None,
            branch_id: DEFAULT_HISTORY_BRANCH_ID.to_string(),
            forked_from_node_id: None,
            kind: HistoryNodeKind::Checkpoint,
            run_id: None,
            workspace_ref: WorkspaceRef::default(),
            summary: DEFAULT_SESSION_SUMMARY.to_string(),
            title: DEFAULT_SESSION_TITLE.to_string(),
            history: Vec::new(),
            provider_native_transcript: Vec::new(),
            turn_trace_history: Vec::new(),
            turn_id: None,
            turn_trace_refs: Some(Vec::new()),
            long_term_memory_entries: Vec::new(),
            memory_write_evidence: Vec::new(),
            memory_write_hook_trace_records: Vec::new(),
            turn_count: 0,
            last_referenced_file: None,
            created_at_ms: session.updated_at_ms.saturating_sub(1),
            event_seq_range: None,
        });
        let user_indexes = session
            .history
            .iter()
            .enumerate()
            .filter_map(|(index, message)| (message.role == "user").then_some(index))
            .collect::<Vec<_>>();
        for (turn_index, _start) in user_indexes.iter().enumerate() {
            let end = user_indexes
                .get(turn_index + 1)
                .copied()
                .unwrap_or(session.history.len());
            let history = session.history[..end].to_vec();
            let mut materialized = SessionState {
                conversation_id: session.conversation_id.clone(),
                title: DEFAULT_SESSION_TITLE.to_string(),
                summary: DEFAULT_SESSION_SUMMARY.to_string(),
                history,
                provider_native_transcript: if turn_index + 1 == user_indexes.len() {
                    session.provider_native_transcript.clone()
                } else {
                    Vec::new()
                },
                turn_trace_history: session
                    .turn_trace_history
                    .iter()
                    .take((turn_index + 1).min(session.turn_trace_history.len()))
                    .cloned()
                    .collect(),
                trace_migration_state: session.trace_migration_state,
                turn_trace_refs: None,
                long_term_memory_entries: replay_long_term_memory(&session.history[..end]),
                memory_write_evidence: Vec::new(),
                memory_write_hook_trace_records: Vec::new(),
                history_state_evidence: Vec::new(),
                turn_count: 0,
                last_referenced_file: None,
                updated_at_ms: session.updated_at_ms,
                history_nodes: Vec::new(),
                history_branches: Vec::new(),
                history_cursor: HistoryCursor::default(),
                event_watermark: 0,
                last_commit_watermark: 0,
                workspace_id: session.workspace_id.clone(),
                title_override: None,
                archived: false,
            };
            refresh_session_metadata(&mut materialized, false);
            session.history_nodes.push(HistoryNode {
                node_id: legacy_history_node_id(session, turn_index + 1),
                session_id: session.conversation_id.clone(),
                parent_node_id: session
                    .history_nodes
                    .last()
                    .map(|node| node.node_id.clone()),
                branch_id: DEFAULT_HISTORY_BRANCH_ID.to_string(),
                forked_from_node_id: None,
                kind: HistoryNodeKind::TurnCommitted,
                run_id: materialized
                    .turn_trace_history
                    .last()
                    .map(|trace| trace.turn_id.clone()),
                workspace_ref: WorkspaceRef::default(),
                summary: materialized.summary.clone(),
                title: materialized.title.clone(),
                history: materialized.history.clone(),
                provider_native_transcript: materialized.provider_native_transcript.clone(),
                turn_trace_history: materialized.turn_trace_history.clone(),
                turn_id: materialized
                    .turn_trace_history
                    .last()
                    .map(|trace| trace.turn_id.clone()),
                turn_trace_refs: Some(
                    materialized
                        .turn_trace_history
                        .iter()
                        .map(|trace| TurnTraceRef {
                            turn_id: trace.turn_id.clone(),
                            updated_at_ms: trace.updated_at,
                            version: None,
                        })
                        .collect(),
                ),
                long_term_memory_entries: materialized.long_term_memory_entries.clone(),
                memory_write_evidence: materialized.memory_write_evidence.clone(),
                memory_write_hook_trace_records: materialized
                    .memory_write_hook_trace_records
                    .clone(),
                turn_count: materialized.turn_count,
                last_referenced_file: materialized.last_referenced_file.clone(),
                created_at_ms: session.updated_at_ms.saturating_add(turn_index as u64),
                event_seq_range: None,
            });
        }
        changed = true;
    }

    let root_node_id = legacy_history_root_node_id(session);
    let first_main_branch_turn_index = session
        .history_nodes
        .iter()
        .position(|node| node.branch_id == DEFAULT_HISTORY_BRANCH_ID && node.turn_count > 0);
    let has_root_node = session
        .history_nodes
        .iter()
        .any(|node| node.node_id == root_node_id);
    if !has_root_node {
        if let Some(first_index) = first_main_branch_turn_index {
            let first_node_id = session.history_nodes[first_index].node_id.clone();
            let main_branch_base_node_id = session
                .history_branches
                .iter()
                .find(|branch| branch.branch_id == DEFAULT_HISTORY_BRANCH_ID)
                .and_then(|branch| branch.base_node_id.clone());
            let parent_is_root_or_none = session.history_nodes[first_index]
                .parent_node_id
                .as_ref()
                .map(|p| p == &root_node_id)
                .unwrap_or(true);
            let should_insert_root = parent_is_root_or_none
                && main_branch_base_node_id
                    .as_deref()
                    .map(|base| base == first_node_id || base == root_node_id.as_str())
                    .unwrap_or(true);
            if should_insert_root {
                let created_at_ms = session.history_nodes[first_index]
                    .created_at_ms
                    .saturating_sub(1);
                session.history_nodes.insert(
                    first_index,
                    HistoryNode {
                        node_id: root_node_id.clone(),
                        session_id: session.conversation_id.clone(),
                        parent_node_id: None,
                        branch_id: DEFAULT_HISTORY_BRANCH_ID.to_string(),
                        forked_from_node_id: None,
                        kind: HistoryNodeKind::Checkpoint,
                        run_id: None,
                        workspace_ref: WorkspaceRef::default(),
                        summary: DEFAULT_SESSION_SUMMARY.to_string(),
                        title: DEFAULT_SESSION_TITLE.to_string(),
                        history: Vec::new(),
                        provider_native_transcript: Vec::new(),
                        turn_trace_history: Vec::new(),
                        turn_id: None,
                        turn_trace_refs: Some(Vec::new()),
                        long_term_memory_entries: Vec::new(),
                        memory_write_evidence: Vec::new(),
                        memory_write_hook_trace_records: Vec::new(),
                        turn_count: 0,
                        last_referenced_file: None,
                        created_at_ms,
                        event_seq_range: None,
                    },
                );
                if let Some(first_node) = session.history_nodes.get_mut(first_index + 1) {
                    first_node.parent_node_id = Some(root_node_id.clone());
                }
                changed = true;
            }
        }
    }

    let latest_node_id = session
        .history_nodes
        .last()
        .map(|node| node.node_id.clone());
    let main_branch_head_node_id = session
        .history_nodes
        .iter()
        .rev()
        .find(|node| node.branch_id == DEFAULT_HISTORY_BRANCH_ID)
        .map(|node| node.node_id.clone());
    let first_node_id = session
        .history_nodes
        .first()
        .map(|node| node.node_id.clone());
    let updated_at_ms = session.updated_at_ms;
    let has_legacy_root_node = session
        .history_nodes
        .iter()
        .any(|node| node.node_id == root_node_id);
    if let Some(main_branch) = history_branch_mut(session, DEFAULT_HISTORY_BRANCH_ID) {
        if main_branch.base_node_id.is_none() {
            main_branch.base_node_id = first_node_id;
            changed = true;
        }
        if main_branch.base_node_id.as_deref() != Some(root_node_id.as_str())
            && has_legacy_root_node
        {
            main_branch.base_node_id = Some(root_node_id.clone());
            changed = true;
        }
        if main_branch.head_node_id != main_branch_head_node_id {
            main_branch.head_node_id = main_branch_head_node_id.clone();
            main_branch.updated_at_ms = updated_at_ms;
            changed = true;
        }
    }

    if session.history_cursor.active_branch_id.is_none() {
        session.history_cursor.active_branch_id = Some(DEFAULT_HISTORY_BRANCH_ID.to_string());
        changed = true;
    }
    let active_branch_head_node_id =
        session
            .history_cursor
            .active_branch_id
            .as_deref()
            .and_then(|branch_id| {
                session
                    .history_branches
                    .iter()
                    .find(|branch| branch.branch_id == branch_id)
                    .and_then(|branch| branch.head_node_id.clone())
            });
    if session.history_cursor.branch_head_node_id != active_branch_head_node_id {
        session.history_cursor.branch_head_node_id = active_branch_head_node_id.clone();
        changed = true;
    }
    if session.history_cursor.visible_node_id.is_none() {
        session.history_cursor.visible_node_id = active_branch_head_node_id.clone();
        changed = true;
    }
    if session.history_cursor.workspace_node_id.is_none() {
        session.history_cursor.workspace_node_id = active_branch_head_node_id.or(latest_node_id);
        changed = true;
    }

    changed
}

fn prepare_session_for_new_turn(session: &mut SessionState) {
    let Some(visible_node_id) = session.history_cursor.visible_node_id.clone() else {
        return;
    };
    let Some(branch_head_node_id) = session.history_cursor.branch_head_node_id.clone() else {
        return;
    };
    if visible_node_id == branch_head_node_id {
        return;
    }

    let Some(visible_node) = history_node(session, &visible_node_id).cloned() else {
        return;
    };
    // PA-093：引用化节点（事件流重建视图）不在续写路径重复 hydrate——
    // checkout/restore/switch 已折叠填充；此处仅创建隐式 fork 分支。
    if visible_node.event_seq_range.is_none() {
        hydrate_session_from_node(session, &visible_node);
    }
    let previous_branch_id = session
        .history_cursor
        .active_branch_id
        .clone()
        .unwrap_or_else(|| DEFAULT_HISTORY_BRANCH_ID.to_string());
    let new_branch_id = new_history_branch_id(session, session.history_branches.len() + 1);
    let created_at_ms = now_timestamp_ms();
    session.history_branches.push(HistoryBranch {
        branch_id: new_branch_id.clone(),
        session_id: session.conversation_id.clone(),
        base_node_id: Some(visible_node_id.clone()),
        head_node_id: Some(visible_node_id.clone()),
        forked_from_branch_id: Some(previous_branch_id),
        forked_from_node_id: Some(visible_node_id.clone()),
        label: format!("fork-{}", session.history_branches.len() + 1),
        created_at_ms,
        updated_at_ms: created_at_ms,
    });
    session.history_cursor.visible_node_id = Some(visible_node_id.clone());
    session.history_cursor.active_branch_id = Some(new_branch_id.clone());
    session.history_cursor.branch_head_node_id = Some(visible_node_id.clone());
    session.history_cursor.workspace_node_id = Some(visible_node_id);
    session.history_cursor.mode = HistoryCursorMode::HistoricalDirty;
}

pub(super) fn commit_history_node_from_live_state(
    session: &mut SessionState,
    kind: HistoryNodeKind,
    run_id: Option<String>,
) {
    let created_at_ms = now_timestamp_ms();
    let parent_node_id = session.history_cursor.visible_node_id.clone();
    let branch_id = session
        .history_cursor
        .active_branch_id
        .clone()
        .unwrap_or_else(|| DEFAULT_HISTORY_BRANCH_ID.to_string());
    let forked_from_node_id = session
        .history_branches
        .iter()
        .find(|branch| branch.branch_id == branch_id)
        .and_then(|branch| {
            if branch.head_node_id == parent_node_id {
                None
            } else {
                branch.forked_from_node_id.clone()
            }
        });
    let node_id = new_history_node_id(session, session.history_nodes.len() + 1, created_at_ms);
    session.history_nodes.push(HistoryNode {
        node_id: node_id.clone(),
        session_id: session.conversation_id.clone(),
        parent_node_id,
        branch_id: branch_id.clone(),
        forked_from_node_id,
        kind,
        run_id,
        workspace_ref: WorkspaceRef::default(),
        summary: session.summary.clone(),
        title: session.title.clone(),
        history: session.history.clone(),
        provider_native_transcript: session.provider_native_transcript.clone(),
        turn_trace_history: session.turn_trace_history.clone(),
        turn_id: session
            .turn_trace_history
            .last()
            .map(|trace| trace.turn_id.clone()),
        turn_trace_refs: Some(
            session
                .turn_trace_history
                .iter()
                .map(|trace| TurnTraceRef {
                    turn_id: trace.turn_id.clone(),
                    updated_at_ms: trace.updated_at,
                    version: None,
                })
                .collect(),
        ),
        long_term_memory_entries: session.long_term_memory_entries.clone(),
        memory_write_evidence: session.memory_write_evidence.clone(),
        memory_write_hook_trace_records: session.memory_write_hook_trace_records.clone(),
        turn_count: session.turn_count,
        last_referenced_file: session.last_referenced_file.clone(),
        created_at_ms,
        // PA-093：引用化由 finalize_event_watermark 在事件 flush 后升级
        // （commit 先于 flush 的时序下，此处先以 legacy 快照提交）。
        event_seq_range: None,
    });
    // PA-093：记录本次提交时的事件水位（引用化区间起点）。
    session.last_commit_watermark = session.event_watermark;
    if let Some(branch) = history_branch_mut(session, &branch_id) {
        if branch.base_node_id.is_none() {
            branch.base_node_id = Some(node_id.clone());
        }
        branch.head_node_id = Some(node_id.clone());
        branch.updated_at_ms = created_at_ms;
    }
    session.history_cursor.visible_node_id = Some(node_id.clone());
    session.history_cursor.branch_head_node_id = Some(node_id.clone());
    session.history_cursor.workspace_node_id = Some(node_id.clone());
    session.history_cursor.mode = HistoryCursorMode::Live;
    session.history_cursor.checkout_mode = HistoryCheckoutMode::TranscriptOnly;
    session.history_cursor.checkout_status = HistoryCheckoutStatus::NotRequested;
    bind_unanchored_memory_write_evidence_to_history_node(session, &node_id);
}

fn sync_latest_history_node(session: &mut SessionState, run_id: Option<String>) {
    let Some(latest_node_id) = session.history_cursor.branch_head_node_id.clone() else {
        return;
    };
    let summary = session.summary.clone();
    // override 存在时 session.title 已冻结于改名时点（refresh 跳过赋值）；
    // 节点冻结标题必须表达"本次提交时刻的派生值"，故局部重算而非直取
    // session.title——否则改名后所有新节点都冻结同一陈旧标题。
    let title = if session.title_override.is_some() {
        build_title(&session.history)
    } else {
        session.title.clone()
    };
    let history = session.history.clone();
    let provider_native_transcript = session.provider_native_transcript.clone();
    let turn_trace_history = session.turn_trace_history.clone();
    let long_term_memory_entries = session.long_term_memory_entries.clone();
    let memory_write_evidence = session.memory_write_evidence.clone();
    let memory_write_hook_trace_records = session.memory_write_hook_trace_records.clone();
    let turn_count = session.turn_count;
    let last_referenced_file = session.last_referenced_file.clone();
    let Some(node) = history_node_mut(session, &latest_node_id) else {
        return;
    };
    node.summary = summary;
    node.title = title;
    // PA-093：引用化节点（event_seq_range 已定）不再内嵌快照——事件流是
    // 唯一真源，sync 只同步元数据（summary/title/run_id），避免快照复活。
    if node.event_seq_range.is_none() {
        node.history = history;
        node.provider_native_transcript = provider_native_transcript;
        node.turn_trace_history = turn_trace_history;
        // PA-088：同步更新 turn_id 与 refs，保持内存态与持久化重算一致
        // （前端 checkpoint 列表直接读 node.turnId）。
        node.turn_id = node
            .turn_trace_history
            .last()
            .map(|trace| trace.turn_id.clone());
        node.turn_trace_refs = Some(
            node.turn_trace_history
                .iter()
                .map(|trace| TurnTraceRef {
                    turn_id: trace.turn_id.clone(),
                    updated_at_ms: trace.updated_at,
                    version: None,
                })
                .collect(),
        );
        node.long_term_memory_entries = long_term_memory_entries;
        node.memory_write_evidence = memory_write_evidence;
        node.memory_write_hook_trace_records = memory_write_hook_trace_records;
        node.turn_count = turn_count;
        node.last_referenced_file = last_referenced_file;
    }
    if run_id.is_some() {
        node.run_id = run_id;
    }
}

fn bind_unanchored_memory_write_evidence_to_history_node(
    session: &mut SessionState,
    history_node_id: &str,
) {
    let mut changed = false;
    for evidence in &mut session.memory_write_evidence {
        if evidence.source_history_node_id.is_none() {
            evidence.source_history_node_id = Some(history_node_id.to_string());
            changed = true;
        }
    }
    if !changed {
        return;
    }

    let evidence = session.memory_write_evidence.clone();
    let Some(node) = history_node_mut(session, history_node_id) else {
        return;
    };
    node.memory_write_evidence = evidence;
}

pub(in crate::agent::session) fn hydrate_session_from_node(session: &mut SessionState, node: &HistoryNode) {
    session.title = node.title.clone();
    session.summary = node.summary.clone();
    session.history = node.history.clone();
    session.provider_native_transcript = node.provider_native_transcript.clone();
    session.turn_trace_history = node.turn_trace_history.clone();
    session.long_term_memory_entries = node.long_term_memory_entries.clone();
    session.memory_write_evidence = node.memory_write_evidence.clone();
    session.memory_write_hook_trace_records = node.memory_write_hook_trace_records.clone();
    session.turn_count = node.turn_count;
    session.last_referenced_file = node.last_referenced_file.clone();
}

/// PA-093：引用化节点 → 会话内存态（history/transcript/trace 由事件折叠提供，
/// memory/摘要等节点元数据仍从节点拷贝；transcript 事件流不覆盖 → 引用化后为空，
/// 前端 transcript 视图依赖 trace 数据，文档已声明降级）。
pub(in crate::agent::session) fn hydrate_session_from_projection(
    session: &mut SessionState,
    node: &HistoryNode,
    history: Vec<TurnHistoryMessage>,
    trace: Vec<TurnTraceRecord>,
) {
    session.title = node.title.clone();
    session.summary = node.summary.clone();
    session.history = history;
    session.provider_native_transcript = node.provider_native_transcript.clone();
    session.turn_trace_history = trace;
    session.long_term_memory_entries = node.long_term_memory_entries.clone();
    session.memory_write_evidence = node.memory_write_evidence.clone();
    session.memory_write_hook_trace_records = node.memory_write_hook_trace_records.clone();
    session.turn_count = node.turn_count;
    session.last_referenced_file = node.last_referenced_file.clone();
}

/// PA-096 Phase 2：事件流 → 会话视图（history/trace 双投影，按 Ancestor DAG 路径过滤）。
/// 基于目标节点沿 parent_node_id 递归回溯得到的祖先节点链路确定性重放，
/// 彻底消除同分支回滚或侧分支被撤回事件的复活。
pub(super) fn fold_session_views(
    events: &[(u64, String, crate::agent::turn_event::TurnEvent)],
    target_node_id: &str,
    nodes: &[HistoryNode],
    branches: &[HistoryBranch],
) -> (Vec<TurnHistoryMessage>, Vec<TurnTraceRecord>) {
    use crate::agent::projection::{
        fold_all_with_dag, HistoryProjectionState, MetricsProjectionState,
        TraceProjectionState,
    };
    let history_state = fold_all_with_dag::<_, HistoryProjectionState>(
        events,
        target_node_id,
        nodes,
        branches,
    );
    let trace_state = fold_all_with_dag::<_, TraceProjectionState>(
        events,
        target_node_id,
        nodes,
        branches,
    );
    // PA-094：ProviderCallCacheRecord 由 MetricsProjection 生成（design.md §5）——
    // trace 记录不再独立存储，重建时从 ProviderUsage 事件派生并挂载。
    let metrics_state = fold_all_with_dag::<_, MetricsProjectionState>(
        events,
        target_node_id,
        nodes,
        branches,
    );
    let mut traces = trace_state.traces();
    for trace in &mut traces {
        if let Some(records) = metrics_state.by_turn_records.get(&trace.turn_id) {
            trace.provider_call_records = records.clone();
        }
    }
    (history_state.messages(), traces)
}

/// PA-094（审核 P1）：全量折叠 trace（不带分支过滤）——trace 缓存重建专用。
/// 事件权威：缓存是投影缓存，`record_turn_trace` 写入所有 turn 的 trace，
/// 清表重建必须还原全集（按 active branch 过滤会丢失其他分支的 trace）。
fn fold_session_traces_all(
    events: &[(u64, String, crate::agent::turn_event::TurnEvent)],
) -> Vec<TurnTraceRecord> {
    use crate::agent::projection::{fold_all, MetricsProjectionState, TraceProjectionState};
    let events2: Vec<(u64, crate::agent::turn_event::TurnEvent)> = events
        .iter()
        .map(|(seq, _, event)| (*seq, event.clone()))
        .collect();
    let trace_state = fold_all::<_, TraceProjectionState>(&events2);
    let metrics_state = fold_all::<_, MetricsProjectionState>(&events2);
    let mut traces = trace_state.traces();
    for trace in &mut traces {
        if let Some(records) = metrics_state.by_turn_records.get(&trace.turn_id) {
            trace.provider_call_records = records.clone();
        }
    }
    traces
}

/// PA-088/PA-090：收集会话的 trace 全量 Union（顶层 ∪ 全部节点 trace，按 turn_id 去重取最新）。
/// 用于 Authoritative 会话写表——保证节点 refs 在重启 materialize 时可解析。
/// **稳定顺序（PA-090）**：顶层 trace 原位顺序优先 → 节点独有 trace 按节点顺序追加；
/// 同 turn_id 多版本取 updated_at 最新（同时间顶层优先）。禁止 HashMap 无序输出。
pub(super) fn collect_trace_union(session: &SessionState) -> Vec<TurnTraceRecord> {
    let mut by_id: HashMap<String, TurnTraceRecord> = HashMap::new();
    let mut ordered: Vec<TurnTraceRecord> = Vec::new();

    // 顶层原位优先
    for trace in &session.turn_trace_history {
        if let Some(existing) = by_id.get(&trace.turn_id) {
            if trace.updated_at > existing.updated_at {
                if let Some(slot) = ordered.iter_mut().find(|t| t.turn_id == trace.turn_id) {
                    *slot = trace.clone();
                }
                by_id.insert(trace.turn_id.clone(), trace.clone());
            }
        } else {
            by_id.insert(trace.turn_id.clone(), trace.clone());
            ordered.push(trace.clone());
        }
    }

    // 节点独有按节点顺序追加
    for node in &session.history_nodes {
        for trace in &node.turn_trace_history {
            if let Some(existing) = by_id.get(&trace.turn_id) {
                if trace.updated_at > existing.updated_at {
                    if let Some(slot) = ordered.iter_mut().find(|t| t.turn_id == trace.turn_id) {
                        *slot = trace.clone();
                    }
                    by_id.insert(trace.turn_id.clone(), trace.clone());
                }
            } else {
                by_id.insert(trace.turn_id.clone(), trace.clone());
                ordered.push(trace.clone());
            }
        }
    }

    ordered
}

/// PA-088：轻量节点投影——清空节点内嵌 trace（保留 turnId/refs/摘要/元数据）。
/// 前端 trace 面板读快照顶层 turn_trace_history；节点只用于 checkpoint/回滚映射。
/// 存量节点（legacy 反序列化）turn_id 可能为 None：先回填（trace 末条），
/// 否则前端 checkpoint 列表（依赖 node.turnId）会丢失全部存量节点。
fn project_lightweight_nodes(nodes: &[HistoryNode]) -> Vec<HistoryNode> {
    nodes
        .iter()
        .map(|node| {
            let mut projected = node.clone();
            if projected.turn_id.is_none() {
                projected.turn_id = projected
                    .turn_trace_history
                    .last()
                    .map(|trace| trace.turn_id.clone());
            }
            projected.turn_trace_history.clear();
            projected
        })
        .collect()
}

pub(super) fn session_state_for_backend(
    session: &SessionState,
    trace_mode: SeparateTraceTableMode,
) -> SessionState {
    let mut prepared = session.clone();
    match (trace_mode, prepared.trace_migration_state) {
        (SeparateTraceTableMode::Off, _) => {}
        (SeparateTraceTableMode::DualWrite, TraceMigrationState::LegacyBlob) => {
            prepared.trace_migration_state = TraceMigrationState::DualWrite;
        }
        (SeparateTraceTableMode::DualWrite, TraceMigrationState::DualWrite)
        | (SeparateTraceTableMode::DualWrite, TraceMigrationState::TraceTableAuthoritative) => {}
        (SeparateTraceTableMode::WriteSeparate, TraceMigrationState::TraceTableAuthoritative) => {
            prepared.trace_migration_state = TraceMigrationState::TraceTableAuthoritative;
        }
        // PA-088：WriteSeparate + DualWrite 保持双写（blob 完整 + 表），不晋升剥离。
        // 若在此晋升，内存态仍是 DualWrite（save_to_backend 按内存态分派跳过写表），
        // 持久化副本却被剥离 → 表里没有节点 trace → 重启 materialize 失败（数据丢失）。
        // 存量会话的迁移归 PA-090。
        (SeparateTraceTableMode::WriteSeparate, TraceMigrationState::DualWrite) => {}
        (SeparateTraceTableMode::WriteSeparate, TraceMigrationState::LegacyBlob) => {
            // Do not auto-promote a legacy-only session straight to authoritative.
        }
    }
    if matches!(trace_mode, SeparateTraceTableMode::WriteSeparate)
        && matches!(
            prepared.trace_migration_state,
            TraceMigrationState::TraceTableAuthoritative
        )
    {
        // PA-088：authoritative 会话持久化时剥离 trace，只保留轻量引用。
        // 内存中的 SessionState 不受影响（本函数只作用于持久化副本），
        // 运行中 checkout/fork 仍使用内存完整快照；重启后由 load_store 按 refs 物化。
        prepared.turn_trace_refs = Some(
            prepared
                .turn_trace_history
                .iter()
                .map(|trace| TurnTraceRef {
                    turn_id: trace.turn_id.clone(),
                    updated_at_ms: trace.updated_at,
                    version: None,
                })
                .collect(),
        );
        prepared.turn_trace_history.clear();
        for node in &mut prepared.history_nodes {
            node.turn_trace_refs = Some(
                node.turn_trace_history
                    .iter()
                    .map(|trace| TurnTraceRef {
                        turn_id: trace.turn_id.clone(),
                        updated_at_ms: trace.updated_at,
                        version: None,
                    })
                    .collect(),
            );
            node.turn_id = node
                .turn_trace_history
                .last()
                .map(|trace| trace.turn_id.clone());
            node.turn_trace_history.clear();
        }
    }
    prepared
}

enum TracePersistenceAction {
    ReplaceAll,
    UpsertOne {
        trace: TurnTraceRecord,
        trace_order: usize,
    },
    UpdateTerminalEvent {
        turn_id: String,
        event_id: Option<String>,
        event_type: Option<String>,
        event_version: Option<String>,
        sequence: Option<u64>,
        emitted_at_ms: Option<u64>,
        updated_at: u64,
    },
    AppendHookRecords {
        turn_id: String,
        hook_trace_records: Vec<HookTraceRecord>,
        updated_at: u64,
    },
}

fn trace_action_from_mutation(mutation: SessionTraceMutation) -> TracePersistenceAction {
    match mutation {
        SessionTraceMutation::ReplaceAll { .. } => TracePersistenceAction::ReplaceAll,
        SessionTraceMutation::UpsertOne { trace, trace_order } => {
            TracePersistenceAction::UpsertOne { trace, trace_order }
        }
        SessionTraceMutation::UpdateTerminalEvent {
            turn_id,
            event_id,
            event_type,
            event_version,
            sequence,
            emitted_at_ms,
            updated_at,
            event_watermark: _,
        } => TracePersistenceAction::UpdateTerminalEvent {
            turn_id,
            event_id,
            event_type,
            event_version,
            sequence,
            emitted_at_ms,
            updated_at,
        },
        SessionTraceMutation::AppendHookRecords {
            turn_id,
            hook_trace_records,
            updated_at,
        } => TracePersistenceAction::AppendHookRecords {
            turn_id,
            hook_trace_records,
            updated_at,
        },
    }
}

fn build_history_state_cursor_summary(cursor: &HistoryCursor) -> HistoryStateCursorSummary {
    HistoryStateCursorSummary {
        visible_node_id: cursor.visible_node_id.clone(),
        active_branch_id: cursor.active_branch_id.clone(),
        branch_head_node_id: cursor.branch_head_node_id.clone(),
        workspace_node_id: cursor.workspace_node_id.clone(),
        mode: history_cursor_mode_label(&cursor.mode).to_string(),
        checkout_mode: history_checkout_mode_label(&cursor.checkout_mode).to_string(),
        checkout_status: history_checkout_status_label(&cursor.checkout_status).to_string(),
    }
}

fn history_cursor_mode_label(mode: &HistoryCursorMode) -> &'static str {
    match mode {
        HistoryCursorMode::Live => "live",
        HistoryCursorMode::Historical => "historical",
        HistoryCursorMode::HistoricalDirty => "historical_dirty",
    }
}

fn history_checkout_mode_label(mode: &HistoryCheckoutMode) -> &'static str {
    match mode {
        HistoryCheckoutMode::TranscriptOnly => "transcript_only",
        HistoryCheckoutMode::TranscriptAndWorkspace => "transcript_and_workspace",
    }
}

fn history_checkout_status_label(status: &HistoryCheckoutStatus) -> &'static str {
    match status {
        HistoryCheckoutStatus::NotRequested => "not_requested",
        HistoryCheckoutStatus::Applied => "applied",
        HistoryCheckoutStatus::DegradedToTranscriptOnly => "degraded_to_transcript_only",
    }
}

fn history_state_boundary_label(hook_point: &HistoryStateHookPoint) -> Option<&'static str> {
    match hook_point {
        HistoryStateHookPoint::HistoryCheckoutStart => Some("history.checkout.start"),
        HistoryStateHookPoint::HistoryCheckoutResolved => Some("history.checkout.resolved"),
        HistoryStateHookPoint::BranchRestoreStart => Some("history.branch_restore.start"),
        HistoryStateHookPoint::BranchRestoreResolved => Some("history.branch_restore.resolved"),
        HistoryStateHookPoint::BranchForkStart => Some("history.branch_fork.start"),
        HistoryStateHookPoint::BranchForkResolved => Some("history.branch_fork.resolved"),
        HistoryStateHookPoint::BranchSwitchStart => Some("history.branch_switch.start"),
        HistoryStateHookPoint::BranchSwitchResolved => Some("history.branch_switch.resolved"),
    }
}

fn build_history_state_hook_envelope(
    session: &SessionState,
    hook_point: HistoryStateHookPoint,
    command_kind: HistoryStateCommandKind,
    requested_node_id: Option<&str>,
    requested_branch_id: Option<&str>,
    requested_mode: Option<&HistoryCheckoutMode>,
    resolved_node_id: Option<&str>,
    resolved_branch_id: Option<&str>,
    transcript_restore_applied: bool,
    workspace_rollback_capable: bool,
    workspace_rollback_applied: bool,
    degraded: bool,
    degradation_reason: Option<&str>,
) -> Option<HistoryStateHookEnvelope> {
    Some(HistoryStateHookEnvelope {
        session_id: session.conversation_id.clone(),
        hook_point: hook_point.clone(),
        command_kind,
        source_boundary: history_state_boundary_label(&hook_point)?.to_string(),
        requested_node_id: requested_node_id.map(|value| value.to_string()),
        requested_branch_id: requested_branch_id.map(|value| value.to_string()),
        requested_checkout_mode: requested_mode
            .map(|value| history_checkout_mode_label(value).to_string()),
        resolved_node_id: resolved_node_id.map(|value| value.to_string()),
        resolved_branch_id: resolved_branch_id.map(|value| value.to_string()),
        transcript_restore_applied,
        workspace_rollback_capable,
        workspace_rollback_applied,
        degraded,
        degradation_reason: degradation_reason.map(|value| value.to_string()),
        cursor_summary: build_history_state_cursor_summary(&session.history_cursor),
    })
}

fn history_state_hook_results_blocked(
    hook_results: &[crate::agent::hooks::HookExecutionResult],
) -> bool {
    hook_results.iter().any(|result| {
        result.blocked || matches!(result.structured_result, HookStructuredResult::Deny(_))
    })
}

fn history_state_command_kind_label(command_kind: &HistoryStateCommandKind) -> &'static str {
    match command_kind {
        HistoryStateCommandKind::CheckoutHistoryNode => "checkout_history_node",
        HistoryStateCommandKind::RestoreBranchHead => "restore_branch_head",
        HistoryStateCommandKind::ForkFromHistoryNode => "fork_from_history_node",
        HistoryStateCommandKind::SwitchHistoryBranch => "switch_history_branch",
    }
}

fn hook_result_kind_label(result_kind: &HookResultKind) -> &'static str {
    match result_kind {
        HookResultKind::Observe => "observe",
        HookResultKind::Allow => "allow",
        HookResultKind::Deny => "deny",
        HookResultKind::Patch => "patch",
        HookResultKind::SideEffectRequest => "side_effect_request",
    }
}

fn persist_history_state_hook_evidence(
    session: &mut SessionState,
    envelope: &HistoryStateHookEnvelope,
    hook_results: &[crate::agent::hooks::HookExecutionResult],
) {
    if hook_results.is_empty() {
        return;
    }

    let recorded_at_ms = now_timestamp_ms();
    for (index, result) in hook_results.iter().enumerate() {
        session
            .history_state_evidence
            .push(HistoryStateHookEvidence {
                evidence_id: format!(
                    "history-state:{}:{}:{}:{}",
                    session.conversation_id, envelope.source_boundary, recorded_at_ms, index
                ),
                session_id: session.conversation_id.clone(),
                boundary: envelope.source_boundary.clone(),
                command_kind: history_state_command_kind_label(&envelope.command_kind).to_string(),
                result_kind: hook_result_kind_label(&result.result_kind).to_string(),
                summary: result.trace_summary.clone(),
                elapsed_ms: result.elapsed_ms,
                blocked: result.blocked
                    || matches!(result.structured_result, HookStructuredResult::Deny(_)),
                degraded: envelope.degraded,
                requested_node_id: envelope.requested_node_id.clone(),
                requested_branch_id: envelope.requested_branch_id.clone(),
                resolved_node_id: envelope.resolved_node_id.clone(),
                resolved_branch_id: envelope.resolved_branch_id.clone(),
                recorded_at_ms,
            });
    }
}

fn replay_long_term_memory(history: &[TurnHistoryMessage]) -> Vec<LongTermMemoryRecord> {
    let mut entries = Vec::new();
    for message in history.iter().filter(|message| message.role == "user") {
        for entry in extract_long_term_memory_from_user_message(&message.content) {
            match entries
                .iter_mut()
                .find(|existing| memory_record_identity(existing) == memory_record_identity(&entry))
            {
                Some(existing) => *existing = entry,
                None => entries.push(entry),
            }
        }
    }
    entries
}

fn classify_turn_node_kind(assistant_message: &str) -> HistoryNodeKind {
    if assistant_message.trim() == "用户终止，发送消息可继续。" {
        HistoryNodeKind::TurnCancelled
    } else {
        HistoryNodeKind::TurnCommitted
    }
}

fn history_node<'a>(session: &'a SessionState, node_id: &str) -> Option<&'a HistoryNode> {
    session
        .history_nodes
        .iter()
        .find(|node| node.node_id == node_id)
}

fn history_node_mut<'a>(
    session: &'a mut SessionState,
    node_id: &str,
) -> Option<&'a mut HistoryNode> {
    session
        .history_nodes
        .iter_mut()
        .find(|node| node.node_id == node_id)
}

fn history_branch_mut<'a>(
    session: &'a mut SessionState,
    branch_id: &str,
) -> Option<&'a mut HistoryBranch> {
    session
        .history_branches
        .iter_mut()
        .find(|branch| branch.branch_id == branch_id)
}

fn legacy_history_node_id(session: &SessionState, turn_count: usize) -> String {
    format!("{}-legacy-node-{}", session.conversation_id, turn_count)
}

fn legacy_history_root_node_id(session: &SessionState) -> String {
    format!("{}-legacy-root", session.conversation_id)
}

fn new_history_node_id(session: &SessionState, ordinal: usize, created_at_ms: u64) -> String {
    format!(
        "{}-node-{}-{}",
        session.conversation_id, created_at_ms, ordinal
    )
}

fn new_history_branch_id(session: &SessionState, ordinal: usize) -> String {
    format!("{}-branch-{}", session.conversation_id, ordinal)
}

/// 会话投影元数据刷新（标题/摘要/计数等）。pub(in session) 仅为让兄弟测试模块
/// 直接驱动两个派生分支；生产调用方均在 store 内部。
pub(in crate::agent::session) fn refresh_session_metadata(session: &mut SessionState, touch_updated_at: bool) {
    normalize_trace_timeline_entries(&mut session.turn_trace_history);
    session.turn_count = session
        .history
        .iter()
        .filter(|message| message.role == "user")
        .count();
    session.last_referenced_file = session
        .history
        .iter()
        .rev()
        .find_map(|message| extract_explicit_file_name(&message.content));
    // 用户显式标题（override）存在时，两个派生分支都跳过 title 赋值：
    // session.title 字段就此冻结在改名时点的派生值上，投影一律走
    // effective_title()。派生链路的其他产物（summary 等）不受影响。
    let title_overridden = session.title_override.is_some();
    if session.history.is_empty() && !session.turn_trace_history.is_empty() {
        if let Some(trace) = session.turn_trace_history.last() {
            if !title_overridden {
                session.title = trace.title.clone();
            }
            session.summary = trace
                .session_summary
                .clone()
                .or(trace.error.clone())
                .or(trace.fallback_reason.clone())
                .unwrap_or_else(|| DEFAULT_SESSION_SUMMARY.to_string());
        }
    } else {
        if !title_overridden {
            session.title = build_title(&session.history);
        }
        session.summary =
            build_summary(session.turn_count, session.last_referenced_file.as_deref());
    }
    if touch_updated_at {
        session.updated_at_ms = now_timestamp_ms();
    }
}

fn canonical_trace_timeline_kind(kind: &str) -> &str {
    match kind {
        "context" => "build_context",
        "model" => "call_model",
        "tool" => "call_tool",
        "return" => "return_result",
        other => other,
    }
}

fn normalize_trace_timeline_entries(turn_trace_history: &mut [TurnTraceRecord]) {
    for trace in turn_trace_history {
        for entry in &mut trace.trace_timeline {
            entry.kind = canonical_trace_timeline_kind(&entry.kind).to_string();
        }
    }
}

fn update_long_term_memory_from_user_message(
    session: &mut SessionState,
    user_message: &str,
    hook_executor: &dyn MemoryWriteHookExecutor,
) -> bool {
    let extracted_entries = extract_long_term_memory_from_user_message(user_message);
    if extracted_entries.is_empty() {
        return false;
    }

    let existing_entries = session.long_term_memory_entries.clone();
    let mut planned_writes = plan_memory_write_intents(&existing_entries, &extracted_entries);
    if let Some(envelope) = build_memory_write_hook_envelope(
        &session.conversation_id,
        user_message,
        planned_writes.clone(),
    ) {
        let hook_results = hook_executor.execute(&envelope).unwrap_or_default();
        session.memory_write_hook_trace_records.extend(
            hook_results
                .iter()
                .map(crate::agent::hooks::HookExecutionResult::to_trace_record),
        );
        let hook_outcome =
            apply_memory_write_hook_results(&existing_entries, &mut planned_writes, &hook_results);
        if hook_outcome.blocked {
            return false;
        }
    }

    recalculate_planned_memory_write_operations(&existing_entries, &mut planned_writes);
    if let Some(envelope) = build_memory_write_hook_envelope(
        &session.conversation_id,
        user_message,
        planned_writes.clone(),
    ) {
        session.memory_write_evidence.extend(
            planned_writes
                .iter()
                .filter(|planned| planned.operation != MemoryWriteOperation::Noop)
                .map(|planned| build_persisted_memory_write_evidence(&envelope, planned)),
        );
    }

    let mut changed = false;
    for planned in planned_writes {
        let entry = planned.entry;
        match session
            .long_term_memory_entries
            .iter_mut()
            .find(|existing| memory_record_identity(existing) == memory_record_identity(&entry))
        {
            Some(existing)
                if existing.content == entry.content && existing.source == entry.source => {}
            Some(existing) => {
                *existing = entry;
                changed = true;
            }
            None => {
                session.long_term_memory_entries.push(entry);
                changed = true;
            }
        }
    }

    changed
}

#[derive(Default)]
struct MemoryWriteHookApplicationOutcome {
    blocked: bool,
}

#[derive(Clone)]
struct PlannedMemoryWrite {
    key: String,
    entry: LongTermMemoryRecord,
    operation: MemoryWriteOperation,
}

fn plan_memory_write_intents(
    existing_entries: &[LongTermMemoryRecord],
    extracted_entries: &[LongTermMemoryRecord],
) -> Vec<PlannedMemoryWrite> {
    extracted_entries
        .iter()
        .cloned()
        .map(|entry| {
            let operation = match existing_entries
                .iter()
                .find(|existing| memory_record_identity(existing) == memory_record_identity(&entry))
            {
                Some(existing)
                    if existing.content == entry.content && existing.source == entry.source =>
                {
                    MemoryWriteOperation::Noop
                }
                Some(_) => MemoryWriteOperation::Update,
                None => MemoryWriteOperation::Insert,
            };
            PlannedMemoryWrite {
                key: memory_record_identity(&entry),
                entry,
                operation,
            }
        })
        .collect()
}

fn recalculate_planned_memory_write_operations(
    existing_entries: &[LongTermMemoryRecord],
    planned_writes: &mut [PlannedMemoryWrite],
) {
    for planned in planned_writes {
        planned.operation = match existing_entries.iter().find(|existing| {
            memory_record_identity(existing) == memory_record_identity(&planned.entry)
        }) {
            Some(existing)
                if existing.content == planned.entry.content
                    && existing.source == planned.entry.source =>
            {
                MemoryWriteOperation::Noop
            }
            Some(_) => MemoryWriteOperation::Update,
            None => MemoryWriteOperation::Insert,
        };
        planned.key = memory_record_identity(&planned.entry);
    }
}

fn apply_memory_write_hook_results(
    existing_entries: &[LongTermMemoryRecord],
    planned_writes: &mut [PlannedMemoryWrite],
    hook_results: &[crate::agent::hooks::HookExecutionResult],
) -> MemoryWriteHookApplicationOutcome {
    for result in hook_results {
        if result.blocked {
            return MemoryWriteHookApplicationOutcome { blocked: true };
        }
        if matches!(result.structured_result, HookStructuredResult::Deny(_)) {
            return MemoryWriteHookApplicationOutcome { blocked: true };
        }
    }

    let transform_results = hook_results
        .iter()
        .filter(|result| result.result_kind == crate::agent::hooks::HookResultKind::Patch)
        .cloned()
        .collect::<Vec<_>>();
    if transform_results.is_empty() {
        return MemoryWriteHookApplicationOutcome { blocked: false };
    }

    let merged =
        match merge_patch_results(&transform_results, HookPatchConflictPolicy::LastWriteWins) {
            Ok(merged) => merged,
            Err(_) => return MemoryWriteHookApplicationOutcome { blocked: false },
        };

    for operation in merged.operations {
        let patch = operation.operation;
        if patch.target != HookPatchTarget::MemoryWriteIntent {
            continue;
        }
        if patch.operation != HookPatchOperationKind::Set {
            continue;
        }

        let Some((index, field)) = parse_memory_write_patch_path(&patch.path) else {
            continue;
        };
        let Some(planned) = planned_writes.get_mut(index) else {
            continue;
        };
        let Some(value_text) = patch
            .value_text
            .clone()
            .or_else(|| patch.value_summary.clone())
        else {
            continue;
        };

        match field {
            MemoryWritePatchField::Kind => planned.entry.kind = value_text,
            MemoryWritePatchField::Content => planned.entry.content = value_text,
            MemoryWritePatchField::Source => planned.entry.source = value_text,
            MemoryWritePatchField::Operation => {
                if let Some(operation) = parse_memory_write_operation(&value_text) {
                    planned.operation = operation;
                }
            }
        }
    }

    recalculate_planned_memory_write_operations(existing_entries, planned_writes);
    MemoryWriteHookApplicationOutcome { blocked: false }
}

#[derive(Clone, Copy)]
enum MemoryWritePatchField {
    Kind,
    Content,
    Source,
    Operation,
}

fn parse_memory_write_patch_path(path: &str) -> Option<(usize, MemoryWritePatchField)> {
    let suffix = path.strip_prefix("writes[")?;
    let (index_text, field_text) = suffix.split_once("].")?;
    let index = index_text.parse::<usize>().ok()?;
    let field = match field_text {
        "kind" => MemoryWritePatchField::Kind,
        "content" => MemoryWritePatchField::Content,
        "source" => MemoryWritePatchField::Source,
        "operation" => MemoryWritePatchField::Operation,
        _ => return None,
    };
    Some((index, field))
}

fn parse_memory_write_operation(value: &str) -> Option<MemoryWriteOperation> {
    match value.trim().to_ascii_lowercase().as_str() {
        "insert" => Some(MemoryWriteOperation::Insert),
        "update" => Some(MemoryWriteOperation::Update),
        "noop" => Some(MemoryWriteOperation::Noop),
        _ => None,
    }
}

fn build_memory_write_hook_envelope(
    session_id: &str,
    user_message: &str,
    planned_writes: Vec<PlannedMemoryWrite>,
) -> Option<MemoryWriteHookEnvelope> {
    if planned_writes.is_empty() {
        return None;
    }

    Some(MemoryWriteHookEnvelope {
        session_id: Some(session_id.to_string()),
        hook_point: MemoryWriteHookPoint::LongTermMemoryWrite,
        target: MemoryWriteTarget::LongTermMemory,
        source_boundary: "session.update_long_term_memory_from_user_message".to_string(),
        user_message_summary: summarize_memory_text(user_message),
        writes: planned_writes
            .into_iter()
            .map(|planned| MemoryWriteIntentRecord {
                key: planned.key,
                kind: planned.entry.kind,
                content_summary: summarize_memory_text(&planned.entry.content),
                content: planned.entry.content,
                source: planned.entry.source,
                operation: planned.operation,
            })
            .collect(),
    })
}

fn build_persisted_memory_write_evidence(
    envelope: &MemoryWriteHookEnvelope,
    planned: &PlannedMemoryWrite,
) -> PersistedEffectEvidence {
    PersistedEffectEvidence {
        evidence_id: format!(
            "memory-write:{}:{}",
            planned.key, planned.entry.updated_at_ms
        ),
        effect_kind: "memory_write.long_term_memory".to_string(),
        boundary: "session.update_long_term_memory_from_user_message".to_string(),
        target_session_id: envelope.session_id.clone(),
        source_history_node_id: None,
        target_summary: format!(
            "{}:{}",
            planned.entry.kind,
            summarize_memory_text(&planned.entry.content)
        ),
        persistence_ref: format!("long_term_memory_entries/{}", planned.key),
        replay_decision_basis:
            "persisted memory write evidence is required to avoid replaying side effects"
                .to_string(),
        persisted_at_ms: planned.entry.updated_at_ms,
        replay_required_if_missing: true,
    }
}

fn summarize_memory_text(text: &str) -> String {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let normalized = normalized.trim();
    if normalized.chars().count() <= 80 {
        return normalized.to_string();
    }
    let mut summary = String::new();
    for (index, ch) in normalized.chars().enumerate() {
        if index >= 77 {
            break;
        }
        summary.push(ch);
    }
    summary.push_str("...");
    summary
}

fn extract_long_term_memory_from_user_message(user_message: &str) -> Vec<LongTermMemoryRecord> {
    let lowered = user_message.to_lowercase();
    let updated_at_ms = now_timestamp_ms();
    let mut entries = Vec::new();

    if contains_any_phrase(
        &lowered,
        &[
            "全部使用中文",
            "请用中文回复",
            "请用中文回答",
            "用中文回复",
            "用中文回答",
            "中文回复",
            "中文回答",
            "reply in chinese",
            "answer in chinese",
        ],
    ) {
        entries.push(LongTermMemoryRecord {
            kind: "user_preference.response_language".to_string(),
            content: "Reply in Chinese.".to_string(),
            source: "explicit_user_message".to_string(),
            updated_at_ms,
        });
    }

    if contains_any_phrase(
        &lowered,
        &[
            "请简洁",
            "尽量简洁",
            "简洁一点",
            "简短一点",
            "回答简洁",
            "回复简洁",
            "keep it concise",
            "be concise",
            "brief answers",
        ],
    ) {
        entries.push(LongTermMemoryRecord {
            kind: "user_preference.response_style".to_string(),
            content: "Keep answers concise.".to_string(),
            source: "explicit_user_message".to_string(),
            updated_at_ms,
        });
    }

    if contains_any_phrase(
        &lowered,
        &[
            "请使用绝对路径",
            "请用绝对路径",
            "使用绝对路径",
            "用绝对路径",
            "请给我绝对路径",
            "引用文件时请使用绝对路径",
            "不要使用相对路径",
            "不要用相对路径",
            "use absolute paths",
            "use absolute file paths",
            "prefer absolute paths",
            "reference files with absolute paths",
        ],
    ) {
        entries.push(LongTermMemoryRecord {
            kind: "user_preference.file_reference_style".to_string(),
            content: "Use absolute paths when referencing workspace files.".to_string(),
            source: "explicit_user_message".to_string(),
            updated_at_ms,
        });
    }

    if contains_any_phrase(
        &lowered,
        &[
            "更新任务文档",
            "同步任务文档",
            "回写任务文档",
            "更新任务系统",
            "同步任务系统",
            "回写任务卡",
            "记得更新任务文档",
            "keep the task documents updated",
            "update the task documents",
            "sync the task system",
            "write back the task card",
        ],
    ) {
        entries.push(LongTermMemoryRecord {
            kind: "user_preference.task_system_sync".to_string(),
            content: "Keep task-system documents updated while progressing work.".to_string(),
            source: "explicit_user_message".to_string(),
            updated_at_ms,
        });
    }

    if contains_any_phrase(
        &lowered,
        &[
            "不要修改无关文件",
            "不要动无关文件",
            "不要碰无关文件",
            "不要回滚无关改动",
            "不要碰无关改动",
            "不要动无关改动",
            "不要改无关文件",
            "don't modify unrelated files",
            "do not modify unrelated files",
            "don't touch unrelated changes",
            "do not touch unrelated changes",
            "don't revert unrelated changes",
            "do not revert unrelated changes",
        ],
    ) {
        entries.push(LongTermMemoryRecord {
            kind: "user_preference.change_scope".to_string(),
            content: "Avoid modifying unrelated existing changes.".to_string(),
            source: "explicit_user_message".to_string(),
            updated_at_ms,
        });
    }

    if let Some(note) = extract_explicit_memory_note(user_message) {
        entries.push(LongTermMemoryRecord {
            kind: "user_memory.explicit_note".to_string(),
            content: note,
            source: "explicit_user_message".to_string(),
            updated_at_ms,
        });
    }

    if let Some(task_id) = extract_explicit_current_task_focus(user_message, &lowered) {
        entries.push(LongTermMemoryRecord {
            kind: "project_focus.active_task".to_string(),
            content: format!("Current active task is {task_id}."),
            source: "explicit_user_message".to_string(),
            updated_at_ms,
        });
    }

    if extract_explicit_acceptance_gate_requirement(&lowered) {
        entries.push(LongTermMemoryRecord {
            kind: "project_workflow.acceptance_gate".to_string(),
            content:
                "Establish acceptance criteria and run a closeout audit before claiming delivery."
                    .to_string(),
            source: "explicit_user_message".to_string(),
            updated_at_ms,
        });
    }

    if let Some(prerequisite) =
        extract_explicit_project_dependency_prerequisite(user_message, &lowered)
    {
        entries.push(LongTermMemoryRecord {
            kind: "project_dependency.prerequisite".to_string(),
            content: prerequisite,
            source: "explicit_user_message".to_string(),
            updated_at_ms,
        });
    }

    if extract_explicit_closeout_requirement(&lowered) {
        entries.push(LongTermMemoryRecord {
            kind: "project_workflow.closeout_requirement".to_string(),
            content:
                "Summarize changed files, verification performed, and unresolved risks at closeout."
                    .to_string(),
            source: "explicit_user_message".to_string(),
            updated_at_ms,
        });
    }

    if let Some(task_boundary) = extract_explicit_task_boundary(&lowered) {
        entries.push(LongTermMemoryRecord {
            kind: "project_scope.task_boundary".to_string(),
            content: task_boundary,
            source: "explicit_user_message".to_string(),
            updated_at_ms,
        });
    }

    entries
}

fn memory_record_identity(entry: &LongTermMemoryRecord) -> String {
    if entry.kind.starts_with("user_preference.")
        || entry.kind.starts_with("project_dependency.")
        || entry.kind == "project_focus.active_task"
        || entry.kind == "project_workflow.acceptance_gate"
        || entry.kind == "project_workflow.closeout_requirement"
        || entry.kind == "project_scope.task_boundary"
    {
        entry.kind.clone()
    } else {
        format!("{}::{}", entry.kind, entry.content)
    }
}

fn extract_explicit_memory_note(user_message: &str) -> Option<String> {
    let trimmed = user_message.trim();
    if trimmed.is_empty() {
        return None;
    }

    let chinese_markers = ["请记住", "帮我记住", "记住：", "记住:", "记住一下", "记住"];
    for marker in chinese_markers {
        if let Some(index) = trimmed.find(marker) {
            let note = trimmed[index + marker.len()..]
                .trim()
                .trim_start_matches(['，', ',', '：', ':', ' '])
                .trim();
            if !note.is_empty() {
                return Some(note.to_string());
            }
        }
    }

    let lowered = trimmed.to_lowercase();
    let english_markers = [
        "please remember that",
        "please remember",
        "remember that",
        "remember:",
        "remember this",
    ];
    for marker in english_markers {
        if let Some(index) = lowered.find(marker) {
            let note = trimmed[index + marker.len()..]
                .trim()
                .trim_start_matches([',', ':', ' '])
                .trim();
            if !note.is_empty() {
                return Some(note.to_string());
            }
        }
    }

    None
}

fn extract_explicit_current_task_focus(
    user_message: &str,
    lowered_user_message: &str,
) -> Option<String> {
    if !contains_any_phrase(
        lowered_user_message,
        &[
            "现在开始",
            "当前优先推进",
            "优先推进",
            "先做",
            "先推进",
            "本轮先做",
            "先处理",
            "当前主线",
            "focus on",
            "start with",
            "start on",
            "prioritize",
            "current priority",
            "work on",
        ],
    ) {
        return None;
    }

    first_task_like_token(user_message)
}

fn extract_explicit_acceptance_gate_requirement(lowered_user_message: &str) -> bool {
    if contains_any_phrase(
        lowered_user_message,
        &[
            "建立验收标准",
            "先建立验收标准",
            "补齐验收标准",
            "验收审计",
            "正式验收",
            "closeout audit",
            "acceptance criteria",
            "acceptance audit",
        ],
    ) {
        return true;
    }

    contains_any_phrase(lowered_user_message, &["验收标准"])
        && contains_any_phrase(
            lowered_user_message,
            &[
                "完成交付",
                "成功完成交付",
                "交付",
                "delivery",
                "claiming delivery",
                "mark the work complete",
            ],
        )
}

fn extract_explicit_project_dependency_prerequisite(
    user_message: &str,
    lowered_user_message: &str,
) -> Option<String> {
    let task_ids = collect_task_like_tokens(user_message);
    if task_ids.len() < 2 {
        return None;
    }

    if contains_any_phrase(
        lowered_user_message,
        &["依赖", "depends on", "blocked on", "prerequisite"],
    ) {
        return Some(format!("{} depends on {}.", task_ids[0], task_ids[1]));
    }

    if contains_any_phrase(
        lowered_user_message,
        &["先完成", "完成前", "before starting", "before working on"],
    ) && contains_any_phrase(
        lowered_user_message,
        &[
            "再做",
            "再推进",
            "再处理",
            "再开始",
            "then start",
            "then work on",
        ],
    ) {
        return Some(format!("{} depends on {}.", task_ids[1], task_ids[0]));
    }

    None
}

fn extract_explicit_closeout_requirement(lowered_user_message: &str) -> bool {
    contains_any_phrase(
        lowered_user_message,
        &[
            "改了哪些文件",
            "做了什么验证",
            "未解决风险",
            "changed files",
            "verification performed",
            "unresolved risks",
        ],
    ) && contains_any_phrase(
        lowered_user_message,
        &["验证", "verification", "风险", "risks"],
    )
}

fn extract_explicit_task_boundary(lowered_user_message: &str) -> Option<String> {
    let markers = [
        "不要越界到",
        "不能越界到",
        "不要扩到",
        "do not expand into",
        "don't expand into",
    ];

    for marker in markers {
        if let Some(index) = lowered_user_message.find(marker) {
            let boundary_text = &lowered_user_message[index + marker.len()..];
            let task_ids = collect_task_like_tokens(boundary_text);
            if !task_ids.is_empty() {
                return Some(format!("Do not expand scope into {}.", task_ids.join(", ")));
            }
        }
    }

    None
}

fn collect_task_like_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();

    for ch in text.chars().chain(std::iter::once(' ')) {
        if ch.is_ascii_alphanumeric() || ch == '-' {
            current.push(ch);
            continue;
        }

        let normalized = current.to_ascii_uppercase();
        if looks_like_task_id(&normalized) && !tokens.iter().any(|existing| existing == &normalized)
        {
            tokens.push(normalized);
        }
        current.clear();
    }

    tokens
}

fn first_task_like_token(text: &str) -> Option<String> {
    let mut current = String::new();
    for ch in text.chars().chain(std::iter::once(' ')) {
        if ch.is_ascii_alphanumeric() || ch == '-' {
            current.push(ch);
            continue;
        }

        let normalized = current.to_ascii_uppercase();
        if looks_like_task_id(&normalized) {
            return Some(normalized);
        }
        current.clear();
    }

    None
}

fn looks_like_task_id(token: &str) -> bool {
    let mut parts = token.split('-');
    let Some(prefix) = parts.next() else {
        return false;
    };
    let Some(number) = parts.next() else {
        return false;
    };
    if parts.next().is_some() {
        return false;
    }

    (2..=6).contains(&prefix.len())
        && prefix.chars().all(|ch| ch.is_ascii_uppercase())
        && (1..=6).contains(&number.len())
        && number.chars().all(|ch| ch.is_ascii_digit())
}

fn contains_any_phrase(text: &str, phrases: &[&str]) -> bool {
    phrases.iter().any(|phrase| text.contains(phrase))
}

fn sanitize_provider_native_transcript(session: &mut SessionState) -> bool {
    if !has_legacy_reasoning_gap(&session.provider_native_transcript)
        && !has_incomplete_tool_roundtrip(&session.provider_native_transcript)
    {
        return false;
    }

    session.provider_native_transcript.clear();
    true
}

fn has_legacy_reasoning_gap(transcript: &[Value]) -> bool {
    let mut awaiting_tool_turn_reasoning = false;

    for message in transcript {
        match message.get("role").and_then(Value::as_str) {
            Some("user") => {
                awaiting_tool_turn_reasoning = false;
            }
            Some("tool") => {
                awaiting_tool_turn_reasoning = true;
            }
            Some("assistant") => {
                let has_tool_calls = message
                    .get("tool_calls")
                    .and_then(Value::as_array)
                    .map(|calls| !calls.is_empty())
                    .unwrap_or(false);
                let missing_reasoning = message
                    .get("reasoning_content")
                    .map(reasoning_content_missing)
                    .unwrap_or(true);

                if (has_tool_calls || awaiting_tool_turn_reasoning) && missing_reasoning {
                    return true;
                }

                if has_tool_calls {
                    awaiting_tool_turn_reasoning = true;
                }
            }
            _ => {}
        }
    }

    false
}

fn reasoning_content_missing(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(text) => text.trim().is_empty(),
        Value::Array(items) => items.is_empty(),
        Value::Object(map) => map.is_empty(),
        _ => false,
    }
}

fn has_incomplete_tool_roundtrip(transcript: &[Value]) -> bool {
    let mut pending_tool_call_ids: Vec<String> = Vec::new();

    for message in transcript {
        match message.get("role").and_then(Value::as_str) {
            Some("assistant") => {
                let tool_call_ids = message
                    .get("tool_calls")
                    .and_then(Value::as_array)
                    .map(|calls| {
                        calls
                            .iter()
                            .filter_map(|call| call.get("id").and_then(Value::as_str))
                            .map(str::to_string)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();

                if !tool_call_ids.is_empty() {
                    pending_tool_call_ids = tool_call_ids;
                }
            }
            Some("tool") => {
                let Some(tool_call_id) = message.get("tool_call_id").and_then(Value::as_str) else {
                    return true;
                };

                if pending_tool_call_ids.is_empty() {
                    return true;
                }

                pending_tool_call_ids.retain(|id| id != tool_call_id);
            }
            Some("user") => {
                if !pending_tool_call_ids.is_empty() {
                    return true;
                }
            }
            _ => {}
        }
    }

    !pending_tool_call_ids.is_empty()
}

fn session_is_persistable(session: &SessionState) -> bool {
    !session.history.is_empty()
        || !session.turn_trace_history.is_empty()
        || !session.long_term_memory_entries.is_empty()
        || !session.memory_write_evidence.is_empty()
        || !session.memory_write_hook_trace_records.is_empty()
        || !session.history_state_evidence.is_empty()
        // A session that has been truncated to the initial (empty) state
        // still has a non-empty history graph (the legacy root node) and
        // must be persisted so the truncation survives a restart.
        || !session.history_nodes.is_empty()
}

fn build_title(history: &[TurnHistoryMessage]) -> String {
    history
        .iter()
        .find(|message| message.role == "user")
        .and_then(|message| normalize_title_candidate(&message.content))
        .unwrap_or_else(|| DEFAULT_SESSION_TITLE.to_string())
}

fn build_summary(_turn_count: usize, last_referenced_file: Option<&str>) -> String {
    match last_referenced_file {
        Some(path) => format!("{} / 当前关注 {}", DEFAULT_SESSION_SUMMARY, path),
        None => DEFAULT_SESSION_SUMMARY.to_string(),
    }
}

fn normalize_title_candidate(text: &str) -> Option<String> {
    let normalized = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))?;

    if normalized.is_empty() {
        return None;
    }

    let title = truncate_chars(&normalized, TITLE_MAX_CHARS);
    if title.is_empty() {
        None
    } else {
        Some(title)
    }
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let mut truncated = String::new();
    let mut count = 0;
    for ch in text.chars() {
        if count >= max_chars {
            truncated.push_str("...");
            return truncated;
        }
        truncated.push(ch);
        count += 1;
    }
    truncated
}

pub(super) fn default_session_title() -> String {
    DEFAULT_SESSION_TITLE.to_string()
}

fn extract_explicit_file_name(text: &str) -> Option<String> {
    let mut candidates = Vec::new();
    let mut current = String::new();

    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | '/' | '\\') {
            current.push(ch);
        } else if !current.is_empty() {
            candidates.push(std::mem::take(&mut current));
        }
    }

    if !current.is_empty() {
        candidates.push(current);
    }

    candidates
        .into_iter()
        .map(|segment| {
            segment
                .trim_matches(|ch: char| ch == '`' || ch == '.' || ch == '!')
                .to_string()
        })
        .find(|segment| {
            !segment.is_empty()
                && segment.contains('.')
                && !segment.starts_with("http://")
                && !segment.starts_with("https://")
                && segment
                    .rsplit('.')
                    .next()
                    .map(|ext| !ext.is_empty() && ext.chars().all(|ch| ch.is_ascii_alphanumeric()))
                    .unwrap_or(false)
        })
}

fn default_storage_path() -> PathBuf {
    #[cfg(test)]
    {
        // 测试必须隔离：绝不读写用户生产数据（%LOCALAPPDATA%/PonyAgent/）。
        unique_test_session_dir("pony-agent-storage").join("sessions.json")
    }

    #[cfg(not(test))]
    {
        dirs::data_local_dir()
            .or_else(dirs::home_dir)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."))
            .join("PonyAgent")
            .join("sessions.json")
    }
}

fn default_attachment_root() -> PathBuf {
    default_storage_path()
        .parent()
        .map(|parent| parent.join("attachments"))
        .unwrap_or_else(|| PathBuf::from("attachments"))
}

#[cfg(test)]
fn unique_test_session_dir(prefix: &str) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_nanos();
    std::env::temp_dir()
        .join(format!("{prefix}-{stamp}"))
        .join("sessions")
}

fn default_sessions() -> SessionMap {
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

pub(super) fn now_timestamp_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

pub(super) fn load_store_from_path(path: &Path) -> Option<PersistedStore> {
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

fn sanitize_attachment_references(session: &mut SessionState) -> bool {
    let mut changed = false;
    for message in &mut session.history {
        for attachment in &mut message.attachments {
            let normalized_relative_path = attachment.relative_path.replace('\\', "/");
            if normalized_relative_path != attachment.relative_path {
                attachment.relative_path = normalized_relative_path;
                changed = true;
            }
        }
    }
    changed
}

fn backfill_attachment_reference_assets(session: &mut SessionState) -> bool {
    let mut changed = false;
    for message in &mut session.history {
        for attachment in &mut message.attachments {
            if attachment.asset_id.trim().is_empty() {
                attachment.asset_id = attachment_asset_id(&attachment.relative_path);
                changed = true;
            }
        }
    }
    changed
}

fn rebuild_attachment_assets(
    sessions: &SessionMap,
    existing_assets: &AttachmentAssetMap,
    attachment_root: &Path,
) -> AttachmentAssetMap {
    let mut assets = scan_attachment_assets(attachment_root, existing_assets);
    merge_session_attachment_assets(sessions, &mut assets);
    assets
}

fn rebuild_attachment_assets_from_sessions(
    sessions: &SessionMap,
    existing_assets: &AttachmentAssetMap,
) -> AttachmentAssetMap {
    let mut assets = existing_assets.clone();
    merge_session_attachment_assets(sessions, &mut assets);
    assets
}

/// 判定消息附件是否应物化为 `AttachmentAsset`。
///
/// 图片资产的 `relative_path` 为 `<session_id>/att-...` 且 `asset_id` 已注册；doc/text
/// 引用（PA-078）仅携带 `relative_path`（`.tmp/imports/...`）、无 `asset_id`，不应进入
/// 附件目录——AC#4 收窄：只对图片保持 AttachmentAsset 生命周期合同。显式 `asset_id`
/// （引用已注册资产）始终保留。
fn is_asset_eligible_attachment(session_id: &str, attachment: &SessionAttachment) -> bool {
    if !attachment.asset_id.trim().is_empty() {
        return true;
    }
    let relative = attachment.relative_path.replace('\\', "/");
    relative.starts_with(&format!("{session_id}/"))
}

fn merge_session_attachment_assets(sessions: &SessionMap, assets: &mut AttachmentAssetMap) {
    for session in sessions.values() {
        for attachment in session
            .history
            .iter()
            .flat_map(|message| message.attachments.iter())
            .filter(|attachment| is_asset_eligible_attachment(&session.conversation_id, attachment))
        {
            let asset_id = if attachment.asset_id.trim().is_empty() {
                attachment_asset_id(&attachment.relative_path)
            } else {
                attachment.asset_id.clone()
            };
            let asset = assets
                .entry(asset_id.clone())
                .or_insert_with(|| AttachmentAsset {
                    id: asset_id.clone(),
                    session_id: session.conversation_id.clone(),
                    name: attachment.name.clone(),
                    mime_type: attachment.mime_type.clone(),
                    relative_path: attachment.relative_path.clone(),
                    size_bytes: attachment.size_bytes,
                    created_at_ms: attachment.created_at_ms,
                    status: AttachmentLifecycleStatus::Active,
                    reference_count: 0,
                    last_referenced_at_ms: None,
                    expires_at_ms: None,
                });
            asset.id = asset_id;
            asset.session_id = session.conversation_id.clone();
            asset.name = attachment.name.clone().or(asset.name.clone());
            if !attachment.mime_type.trim().is_empty() {
                asset.mime_type = attachment.mime_type.clone();
            }
            asset.relative_path = attachment.relative_path.replace('\\', "/");
            if attachment.size_bytes > 0 {
                asset.size_bytes = attachment.size_bytes;
            }
            if attachment.created_at_ms > 0 {
                asset.created_at_ms = attachment.created_at_ms;
            }
        }
    }
}

fn rebuild_session_attachment_index(sessions: &SessionMap) -> SessionAttachmentIndex {
    let mut index = SessionAttachmentIndex::new();
    for session in sessions.values() {
        let mut entry = Vec::new();
        for attachment in session
            .history
            .iter()
            .flat_map(|message| message.attachments.iter())
        {
            let asset_id = if attachment.asset_id.trim().is_empty() {
                attachment_asset_id(&attachment.relative_path)
            } else {
                attachment.asset_id.clone()
            };
            if !entry.iter().any(|existing| existing == &asset_id) {
                entry.push(asset_id);
            }
        }
        if !entry.is_empty() {
            index.insert(session.conversation_id.clone(), entry);
        }
    }
    index
}

fn attachment_assets_for_query(
    sessions: &SessionMap,
    attachment_assets: &AttachmentAssetMap,
    _session_attachment_index: &SessionAttachmentIndex,
    attachment_root: &Path,
    query: &AttachmentAssetQuery,
    now_ms: u64,
) -> Vec<AttachmentAsset> {
    let reference_stats = attachment_reference_stats(sessions);
    let requested_mime = query
        .mime_type
        .as_ref()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty());
    let requested_name = query
        .name_contains
        .as_ref()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty());
    let requested_statuses = query
        .statuses
        .iter()
        .cloned()
        .collect::<HashSet<AttachmentLifecycleStatus>>();

    let mut assets = attachment_assets
        .values()
        .filter(|asset| {
            query
                .session_id
                .as_ref()
                .map_or(true, |session_id| asset.session_id == *session_id)
        })
        .map(|asset| {
            decorate_attachment_asset(
                asset,
                attachment_root,
                reference_stats.get(&asset.id),
                now_ms,
            )
        })
        .filter(|asset| {
            requested_mime.as_ref().map_or(true, |mime| {
                asset.mime_type.to_ascii_lowercase().contains(mime)
            })
        })
        .filter(|asset| {
            requested_name.as_ref().map_or(true, |name| {
                asset
                    .name
                    .as_deref()
                    .map(|value| value.to_ascii_lowercase().contains(name))
                    .unwrap_or(false)
                    || asset.relative_path.to_ascii_lowercase().contains(name)
            })
        })
        .filter(|asset| {
            query
                .created_after_ms
                .map_or(true, |after_ms| asset.created_at_ms >= after_ms)
        })
        .filter(|asset| {
            query
                .created_before_ms
                .map_or(true, |before_ms| asset.created_at_ms <= before_ms)
        })
        .filter(|asset| requested_statuses.is_empty() || requested_statuses.contains(&asset.status))
        .collect::<Vec<_>>();
    assets.sort_by(|left, right| {
        right
            .created_at_ms
            .cmp(&left.created_at_ms)
            .then_with(|| left.id.cmp(&right.id))
    });
    assets
}

#[derive(Clone, Debug, Default)]
struct AttachmentReferenceStat {
    reference_count: usize,
    last_referenced_at_ms: Option<u64>,
}

fn attachment_reference_stats(sessions: &SessionMap) -> HashMap<String, AttachmentReferenceStat> {
    let mut stats = HashMap::<String, AttachmentReferenceStat>::new();
    for session in sessions.values() {
        for attachment in session
            .history
            .iter()
            .flat_map(|message| message.attachments.iter())
        {
            let asset_id = if attachment.asset_id.trim().is_empty() {
                attachment_asset_id(&attachment.relative_path)
            } else {
                attachment.asset_id.clone()
            };
            let entry = stats.entry(asset_id).or_default();
            entry.reference_count += 1;
            entry.last_referenced_at_ms = Some(
                entry
                    .last_referenced_at_ms
                    .unwrap_or(0)
                    .max(attachment.created_at_ms.max(session.updated_at_ms)),
            );
        }
    }
    stats
}

fn decorate_attachment_asset(
    asset: &AttachmentAsset,
    attachment_root: &Path,
    reference_stat: Option<&AttachmentReferenceStat>,
    now_ms: u64,
) -> AttachmentAsset {
    let mut asset = asset.clone();
    let reference_count = reference_stat.map(|stat| stat.reference_count).unwrap_or(0);
    let payload_exists = attachment_root.join(&asset.relative_path).is_file();
    let expires_at_ms = if reference_count == 0 && asset.created_at_ms > 0 {
        Some(
            asset
                .created_at_ms
                .saturating_add(DEFAULT_ATTACHMENT_RECLAIM_TTL_MS),
        )
    } else {
        None
    };

    asset.reference_count = reference_count;
    asset.last_referenced_at_ms = reference_stat.and_then(|stat| stat.last_referenced_at_ms);
    asset.expires_at_ms = expires_at_ms;
    asset.status = if reference_count > 0 {
        if payload_exists {
            AttachmentLifecycleStatus::Active
        } else {
            AttachmentLifecycleStatus::MissingPayload
        }
    } else if expires_at_ms.map_or(false, |deadline| now_ms >= deadline) {
        AttachmentLifecycleStatus::Expired
    } else {
        AttachmentLifecycleStatus::Reclaimable
    };
    asset
}

fn scan_attachment_assets(
    attachment_root: &Path,
    existing_assets: &AttachmentAssetMap,
) -> AttachmentAssetMap {
    let mut assets = AttachmentAssetMap::new();
    let Ok(session_dirs) = fs::read_dir(attachment_root) else {
        return assets;
    };

    for session_dir in session_dirs.flatten() {
        let session_path = session_dir.path();
        if !session_path.is_dir() {
            continue;
        }
        let Some(session_id) = session_path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        let Ok(entries) = fs::read_dir(&session_path) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            let relative_path = format!("{session_id}/{file_name}");
            let asset_id = attachment_asset_id(&relative_path);
            let mut asset = existing_assets.get(&asset_id).cloned().unwrap_or_else(|| {
                let metadata = fs::metadata(&path).ok();
                AttachmentAsset {
                    id: asset_id.clone(),
                    session_id: session_id.to_string(),
                    name: Some(infer_attachment_name(file_name)),
                    mime_type: infer_attachment_mime_type(&path),
                    relative_path: relative_path.clone(),
                    size_bytes: metadata.as_ref().map_or(0, |value| value.len()),
                    created_at_ms: metadata
                        .and_then(|value| value.modified().ok())
                        .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|value| value.as_millis() as u64)
                        .unwrap_or(0),
                    status: AttachmentLifecycleStatus::Reclaimable,
                    reference_count: 0,
                    last_referenced_at_ms: None,
                    expires_at_ms: None,
                }
            });
            asset.session_id = session_id.to_string();
            asset.relative_path = relative_path;
            assets.insert(asset_id, asset);
        }
    }

    assets
}

fn delete_session_attachment_dir(attachment_root: &Path, session_id: &str) {
    let _ = fs::remove_dir_all(attachment_root.join(session_id));
}

#[allow(dead_code)]
fn delete_attachment_file(attachment_root: &Path, attachment: &SessionAttachment) {
    let path = attachment_file_path(attachment_root, attachment);
    let _ = fs::remove_file(&path);
    if let Some(parent) = path.parent() {
        let _ = fs::remove_dir(parent);
    }
}

fn attachment_file_path(attachment_root: &Path, attachment: &SessionAttachment) -> PathBuf {
    attachment_root.join(&attachment.relative_path)
}

pub(super) fn attachment_asset_id(relative_path: &str) -> String {
    format!("asset:{}", relative_path.replace('\\', "/"))
}

fn infer_attachment_name(file_name: &str) -> String {
    file_name
        .strip_suffix(".dataurl")
        .unwrap_or(file_name)
        .to_string()
}

fn infer_attachment_mime_type(path: &Path) -> String {
    fs::read_to_string(path)
        .ok()
        .and_then(|content| {
            content
                .strip_prefix("data:")
                .and_then(|value| value.split(';').next())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "application/octet-stream".to_string())
}

fn load_attachment_image(
    attachment_root: &Path,
    attachment_assets: &AttachmentAssetMap,
    attachment: &SessionAttachment,
) -> Option<TurnInputImage> {
    let path = attachment_assets
        .get(&attachment.asset_id)
        .map(|asset| attachment_root.join(&asset.relative_path))
        .unwrap_or_else(|| attachment_file_path(attachment_root, attachment));
    let data_url = fs::read_to_string(path).ok()?;
    Some(TurnInputImage {
        data_url,
        mime_type: attachment.mime_type.clone(),
        name: attachment.name.clone(),
    })
}
