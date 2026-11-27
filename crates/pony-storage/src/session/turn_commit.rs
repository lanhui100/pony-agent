//! B-CTX-LOAD：`SessionStore` 的轮次原子提交、事件物化与分支头解析。
//!
//! 独立成文件的原因：`session/store.rs` 已触及 `scripts/file-size-baseline.json` 的
//! 行数上限。模块声明位于 `store.rs` 顶部 `#[path = "turn_commit.rs"] mod turn_commit;`。

use super::{
    Arc, DEFAULT_HISTORY_BRANCH_ID, DEFAULT_HISTORY_LIMIT, DEFAULT_SESSION_ID,
    HISTORY_EMERGENCY_LIMIT, HistoryCursorMode, HistoryMessageKind, HistoryNodeKind,
    PersistCommand, PersistCommandOutcome, SeparateTraceTableMode, SessionAttachment,
    SessionBackendMutationResult, SessionError, SessionMetaPatch, SessionSnapshot, SessionState,
    SessionStore, SessionTraceMutation, TraceMigrationState, TraceTerminalPatch,
    TurnHistoryMessage, TurnTraceRecord, Value, collect_trace_union,
    commit_history_node_from_live_state, ensure_history_graph, now_timestamp_ms,
    prepare_session_for_new_turn, refresh_session_metadata, report_index_deferred,
    session_is_persistable, session_state_for_backend, trace_action_from_mutation,
    update_long_term_memory_from_user_message,
};
use crate::session::{fold, history, host_resume};
use pony_protocol::turn_event::TurnEvent;

type MaterializedTurnMessages = (Option<String>, Option<(String, Option<String>)>);

/// 从事件流中按指定 `expected_turn_id`（精确隔离）或最近一个带归属的 turn 物化消息文本。
///
/// 返回 `Option<MaterializedTurnMessages>`：
/// - 当 `expected_turn_id = Some(tid)`（非空）时，**只物化 `event.turn_id() == Some(tid)` 的事件**；
///   若新事件中没有匹配 `tid` 的消息，返回 `None` 让调用方使用自身参数回退（绝不跨 turn 错拿）。
/// - 当 `expected_turn_id = None` 时，保留「取 `seq >= min_watermark` 最后一个带归属事件之 `turn_id`」口径。
pub(in crate::session) fn materialize_turn_messages(
    events: &[(u64, String, TurnEvent)],
    min_watermark: u64,
    expected_turn_id: Option<&str>,
) -> Option<MaterializedTurnMessages> {
    let fresh_events: Vec<&(u64, String, TurnEvent)> = events
        .iter()
        .filter(|(seq, _, _)| *seq >= min_watermark)
        .collect();
    if fresh_events.is_empty() {
        return None;
    }
    let target_turn_id = match expected_turn_id.filter(|tid| !tid.trim().is_empty()) {
        Some(tid) => tid.to_string(),
        None => fresh_events
            .iter()
            .rev()
            .find_map(|(_, _, event)| event.turn_id().map(str::to_string))?,
    };
    let mut matched = false;
    let mut user_text: Option<String> = None;
    let mut assistant: Option<(String, Option<String>)> = None;
    for (_, _, event) in fresh_events {
        if event.turn_id() != Some(target_turn_id.as_str()) {
            continue;
        }
        matched = true;
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
    if !matched {
        return None;
    }
    Some((user_text, assistant))
}

/// 兼容旧签名的包装：无 `expected_turn_id` 时取最近一个带归属事件的 turn。
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "legacy compat wrapper covered by session tests")
)]
pub(in crate::session) fn materialize_last_turn_messages(
    events: &[(u64, String, TurnEvent)],
    min_watermark: u64,
) -> Option<MaterializedTurnMessages> {
    materialize_turn_messages(events, min_watermark, None)
}

/// 解析主分支（`main`）应当指向的 `head_node_id`。
///
/// 不变量：
/// 1. 若 `main` 分支当前 `head_node_id` 指向一个存在于 `main` 分支且**不是** `TurnRewound` 的节点，
///    保留它（尊重显式 `checkout_history_node` 截断，也兼容存量 `TurnCancelled` 撤回夹具）。
/// 2. 若 `head_node_id` 缺失、悬空或指向 `TurnRewound`，回扫 `main` 分支最后一个
///    非 `TurnRewound` 节点（包含活跃的 `TurnCancelled` 失败/取消轮次）。
pub(super) fn resolve_main_branch_head(session: &SessionState) -> Option<String> {
    if let Some(existing_head) = session
        .history_branches
        .iter()
        .find(|branch| branch.branch_id == DEFAULT_HISTORY_BRANCH_ID)
        .and_then(|branch| branch.head_node_id.as_deref())
    {
        if session.history_nodes.iter().any(|node| {
            node.node_id == existing_head
                && node.branch_id == DEFAULT_HISTORY_BRANCH_ID
                && !matches!(node.kind, HistoryNodeKind::TurnRewound)
        }) {
            return Some(existing_head.to_string());
        }
    }
    session
        .history_nodes
        .iter()
        .rev()
        .find(|node| {
            node.branch_id == DEFAULT_HISTORY_BRANCH_ID
                && !matches!(node.kind, HistoryNodeKind::TurnRewound)
        })
        .map(|node| node.node_id.clone())
}

impl SessionStore {
    /// 原子写入一轮历史（user/assistant + native transcript + 可选 `TurnTraceRecord` + `HistoryNode`）。
    ///
    /// - 写入前先 `ensure_session_hydrated` 再判 `history_contains_turn` 幂等，避免未水合时误放行或覆盖；
    /// - 事件源物化按 `turn_id` 精确隔离，防止滞后轮次偷走下一轮的 `UserMessage`；
    /// - 当提供 `trace` 时，在 `commit_history_node_from_live_state` 前写入 `turn_trace_history`
    ///   并把 `PersistCommand::AppendTrace` 放进同一 `persist_commands_batch` 事务，使节点
    ///   `turn_trace_refs` 与 trace 表严格同源同提交。
    fn resolve_turn_event_contents(
        &self,
        session_key: &str,
        turn_id: Option<&str>,
    ) -> Result<MaterializedTurnMessages, SessionError> {
        let current_watermark = self
            .sessions
            .get(session_key)
            .map_or(0, |s| s.event_watermark);
        if !self.backend.supports_turn_events() {
            return Ok((None, None));
        }
        match pony_protocol::turn_stream_types::flush_session_buffered_events(session_key) {
            Ok(_) => match self.load_turn_events_checked(session_key, None) {
                Ok(events) => {
                    if let Some(materialized) =
                        materialize_turn_messages(&events, current_watermark, turn_id)
                    {
                        Ok(materialized)
                    } else {
                        eprintln!(
                            "[pony-agent][session] append_turn: no turn-scoped events for session={session_key} (direct store usage?), falling back to caller text"
                        );
                        Ok((None, None))
                    }
                }
                Err(error) => Err(SessionError::EventStreamDegraded(format!(
                    "[pony-agent][session] append_turn materialization: event read failed: {error}"
                ))),
            },
            Err(error) => {
                eprintln!(
                    "[pony-agent][session] append_turn: buffered event flush failed ({error}); falling back to caller text"
                );
                Ok((None, None))
            }
        }
    }

    fn prepare_trace_for_commit(
        &self,
        session_key: &str,
        trace: Option<TurnTraceRecord>,
    ) -> Option<TurnTraceRecord> {
        let mut trace_item = trace.map(|mut t| {
            t.updated_at = now_timestamp_ms();
            t.derive_usage_provenance_from_calls();
            t.build_context_observation = None;
            t
        })?;
        if trace_item.build_context_observation_ref.is_none() && self.backend.supports_turn_events()
        {
            let prefix = format!("bco:{}:", trace_item.turn_id);
            if let Some(reference) = self
                .backend
                .load_observation_refs(session_key)
                .into_iter()
                .find(|item| item.starts_with(&prefix))
            {
                trace_item.build_context_observation_ref = Some(reference);
            }
        }
        Some(trace_item)
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "batches all staged turn state fields into one PersistCommand transaction"
    )]
    fn persist_staged_turn_commands(
        &self,
        session_key: &str,
        before_state: &SessionState,
        staged_session: &SessionState,
        history_len_before: usize,
        transcript_delta: &[Value],
        appended_user: Option<TurnHistoryMessage>,
        appended_assistant: Option<TurnHistoryMessage>,
        appended_trace: Option<(TurnTraceRecord, usize)>,
        meta_patch: Option<SessionMetaPatch>,
    ) -> Result<(), SessionError> {
        let latest_node = staged_session.history_nodes.last().cloned();
        let latest_cursor = staged_session.history_cursor.clone();
        if !self.backend.supports_turn_events() {
            self.save_session_state_to_backend(session_key, staged_session);
            return Ok(());
        }
        let mut commands = Vec::new();
        fold::append_transcript_command(&mut commands, session_key, transcript_delta);
        fold::append_memory_command_if_changed(
            &mut commands,
            session_key,
            before_state,
            staged_session,
        );
        for (ordinal, message) in [
            (history_len_before, appended_user),
            (history_len_before + 1, appended_assistant),
        ] {
            let Some(message) = message else { continue };
            commands.push(PersistCommand::AppendMessage {
                epoch: 1,
                session_id: session_key.to_string(),
                message,
                ordinal,
            });
        }
        if let Some((trace_item, trace_order)) = appended_trace {
            commands.push(PersistCommand::AppendTrace {
                epoch: 1,
                session_id: session_key.to_string(),
                trace: trace_item,
                trace_order,
            });
        }
        if let Some(meta_patch) = meta_patch {
            commands.push(PersistCommand::UpdateSessionMeta {
                epoch: 1,
                session_id: session_key.to_string(),
                meta_patch,
            });
        }
        if let Some(node) = latest_node {
            commands.push(PersistCommand::UpdateHistoryNode {
                epoch: 1,
                session_id: session_key.to_string(),
                node,
            });
        }
        commands.push(PersistCommand::UpdateCursor {
            epoch: 1,
            session_id: session_key.to_string(),
            cursor: latest_cursor,
        });
        for branch in &staged_session.history_branches {
            commands.push(PersistCommand::UpdateBranch {
                epoch: 1,
                session_id: session_key.to_string(),
                branch: branch.clone(),
            });
        }
        let outcome = self.backend.persist_commands_batch(commands);
        if matches!(outcome, PersistCommandOutcome::WriterBusy) {
            return Err(SessionError::SessionWriterBusy {
                session_id: session_key.to_string(),
            });
        }
        if matches!(
            outcome,
            PersistCommandOutcome::Failed | PersistCommandOutcome::StaleEpoch
        ) {
            return Err(SessionError::BackendFailure(format!(
                "append_turn batch persist failed: {outcome:?}"
            )));
        }
        if !report_index_deferred(session_key, &outcome) {
            self.save_session_state_to_backend(session_key, staged_session);
        }
        Ok(())
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "fallible turn commit mirrors append_turn_with_kind_and_trace signature"
    )]
    pub fn append_turn_with_kind_and_trace_fallible(
        &mut self,
        session_id: Option<&str>,
        user_message: &str,
        assistant_message: &str,
        provider_native_transcript: Option<Vec<Value>>,
        attachments: Vec<SessionAttachment>,
        kind: Option<HistoryMessageKind>,
        turn_id: Option<&str>,
        trace: Option<TurnTraceRecord>,
    ) -> Result<SessionSnapshot, SessionError> {
        let session_key = session_id.unwrap_or(DEFAULT_SESSION_ID).to_string();
        self.ensure_session_hydrated(&session_key);
        let history = self.sessions.get(&session_key).map(|s| s.history.as_slice());
        if history::history_contains_turn(history, turn_id) {
            if let Some(prepared) = self.prepare_trace_for_commit(&session_key, trace) {
                return Ok(self.record_turn_trace(session_id, prepared));
            }
            return Ok(self.snapshot_for_session(&session_key));
        }

        let (event_user, event_assistant) =
            self.resolve_turn_event_contents(&session_key, turn_id)?;
        let user_content = event_user.unwrap_or_else(|| user_message.to_string());
        let (assistant_content, assistant_reasoning) =
            history::close_out_assistant_contents(kind, event_assistant, assistant_message);
        let assistant_content = pony_protocol::dsml::strip_dsml_protocol_text(&assistant_content);
        let assistant_reasoning = assistant_reasoning
            .as_deref()
            .map(pony_protocol::dsml::strip_dsml_protocol_text);
        let memory_write_hook_executor = Arc::clone(&self.memory_write_hook_executor);
        let prepared_trace = self.prepare_trace_for_commit(&session_key, trace);

        let before_state = self.ensure_session(&session_key).clone();
        let mut staged_session = before_state.clone();
        let transcript_delta: Vec<Value> = provider_native_transcript.clone().unwrap_or_default();
        let node_kind = match kind {
            Some(_) => HistoryNodeKind::TurnCancelled,
            None => HistoryNodeKind::TurnCommitted,
        };
        let mut appended_trace: Option<(TurnTraceRecord, usize)> = None;
        ensure_history_graph(&mut staged_session);
        if kind.is_some() {
            if let Some(branch_head) = staged_session.history_cursor.branch_head_node_id.clone() {
                staged_session.history_cursor.visible_node_id = Some(branch_head);
            }
            staged_session.history_cursor.mode = HistoryCursorMode::Live;
        } else {
            prepare_session_for_new_turn(&mut staged_session);
        }
        let history_len_before = staged_session.history.len();
        staged_session.history.push(TurnHistoryMessage {
            role: host_resume::user_history_role(&user_content).to_string(),
            content: user_content,
            attachments,
            turn_id: turn_id.map(str::to_string),
            ..Default::default()
        });
        staged_session.history.push(TurnHistoryMessage {
            role: "assistant".to_string(),
            content: assistant_content,
            attachments: Vec::new(),
            reasoning_content: assistant_reasoning,
            kind,
            turn_id: turn_id.map(str::to_string),
            ..Default::default()
        });
        if staged_session.history.len() > HISTORY_EMERGENCY_LIMIT {
            let keep_from = staged_session.history.len() - HISTORY_EMERGENCY_LIMIT;
            staged_session.history.drain(..keep_from);
        }
        if let Some(messages) = provider_native_transcript {
            staged_session.provider_native_transcript.extend(messages);
        }
        if let Some(trace_item) = prepared_trace {
            if let Some(existing) = staged_session
                .turn_trace_history
                .iter_mut()
                .find(|item| item.turn_id == trace_item.turn_id)
            {
                *existing = trace_item.clone();
            } else {
                staged_session.turn_trace_history.push(trace_item.clone());
            }
            if staged_session.turn_trace_history.len() > DEFAULT_HISTORY_LIMIT {
                let keep_from = staged_session.turn_trace_history.len() - DEFAULT_HISTORY_LIMIT;
                staged_session.turn_trace_history =
                    staged_session.turn_trace_history[keep_from..].to_vec();
            }
            let trace_order = staged_session
                .turn_trace_history
                .iter()
                .position(|item| item.turn_id == trace_item.turn_id)
                .unwrap_or(staged_session.turn_trace_history.len().saturating_sub(1));
            appended_trace = Some((trace_item, trace_order));
        }
        update_long_term_memory_from_user_message(
            &mut staged_session,
            user_message,
            memory_write_hook_executor.as_ref(),
        );
        refresh_session_metadata(&mut staged_session, true);
        commit_history_node_from_live_state(&mut staged_session, node_kind, None);
        let window_start = history_len_before.min(staged_session.history.len());
        let appended_user = staged_session.history.get(window_start).cloned();
        let appended_assistant = staged_session.history.get(window_start + 1).cloned();
        let meta_patch = Some(SessionMetaPatch {
            title: Some(staged_session.title.clone()),
            title_override: Some(staged_session.title_override.clone()),
            archived: Some(staged_session.archived),
            summary: Some(staged_session.summary.clone()),
            turn_count: Some(staged_session.turn_count),
            last_referenced_file: staged_session.last_referenced_file.clone(),
            updated_at_ms: Some(staged_session.updated_at_ms),
        });
        self.persist_staged_turn_commands(
            &session_key,
            &before_state,
            &staged_session,
            history_len_before,
            &transcript_delta,
            appended_user,
            appended_assistant,
            appended_trace,
            meta_patch,
        )?;
        self.sessions.insert(session_key.clone(), staged_session);
        self.refresh_attachment_catalog();
        Ok(self.snapshot_for_session(&session_key))
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "preserves public SessionStore::append_turn_with_kind_and_trace signature"
    )]
    pub fn append_turn_with_kind_and_trace(
        &mut self,
        session_id: Option<&str>,
        user_message: &str,
        assistant_message: &str,
        provider_native_transcript: Option<Vec<Value>>,
        attachments: Vec<SessionAttachment>,
        kind: Option<HistoryMessageKind>,
        turn_id: Option<&str>,
        trace: Option<TurnTraceRecord>,
    ) -> SessionSnapshot {
        match self.append_turn_with_kind_and_trace_fallible(
            session_id,
            user_message,
            assistant_message,
            provider_native_transcript,
            attachments,
            kind,
            turn_id,
            trace,
        ) {
            Ok(snapshot) => snapshot,
            Err(e) => {
                eprintln!("[pony-agent][session] append_turn_with_kind_and_trace error: {e}");
                self.snapshot_for_session(session_id.unwrap_or(DEFAULT_SESSION_ID))
            }
        }
    }

    pub(super) fn persist_session_and_trace_change_impl(
        &mut self,
        session_id: &str,
        mutation: SessionTraceMutation,
    ) {
        let Some(session) = self.sessions.get(session_id) else {
            return;
        };
        let prepared = session_state_for_backend(session, self.backend.trace_storage_mode());
        let is_authoritative = matches!(
            prepared.trace_migration_state,
            TraceMigrationState::TraceTableAuthoritative
        );
        let use_command_stream =
            self.backend.supports_turn_events() && session_is_persistable(session);
        let effective_mutation = if (is_authoritative || self.backend.supports_turn_events())
            && matches!(mutation, SessionTraceMutation::ReplaceAll { .. })
        {
            SessionTraceMutation::ReplaceAll {
                traces: collect_trace_union(session),
            }
        } else {
            mutation.clone()
        };

        if use_command_stream {
            match &effective_mutation {
                SessionTraceMutation::UpsertOne { trace, trace_order } => {
                    let append_cmd = PersistCommand::AppendTrace {
                        epoch: 1,
                        session_id: session_id.to_string(),
                        trace: trace.clone(),
                        trace_order: *trace_order,
                    };
                    let synced_node = session
                        .history_nodes
                        .last()
                        .filter(|node| node.turn_id.as_deref() == Some(trace.turn_id.as_str()))
                        .cloned();
                    let outcome = if let Some(node) = synced_node {
                        self.backend.persist_commands_batch(vec![
                            append_cmd,
                            PersistCommand::UpdateHistoryNode {
                                epoch: 1,
                                session_id: session_id.to_string(),
                                node,
                            },
                        ])
                    } else {
                        self.backend.persist_command(append_cmd)
                    };
                    if matches!(outcome, PersistCommandOutcome::Succeeded) {
                        if is_authoritative {
                            return;
                        }
                    } else {
                        report_index_deferred(session_id, &outcome);
                    }
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
                } => {
                    let outcome = self.backend.persist_command(PersistCommand::UpdateTraceTerminal {
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
                    });
                    if matches!(outcome, PersistCommandOutcome::Succeeded) {
                        if is_authoritative {
                            return;
                        }
                    } else {
                        report_index_deferred(session_id, &outcome);
                    }
                }
                SessionTraceMutation::AppendHookRecords {
                    turn_id,
                    hook_trace_records,
                    updated_at,
                } => {
                    let outcome = self.backend.persist_command(PersistCommand::AppendHookRecords {
                        epoch: 1,
                        session_id: session_id.to_string(),
                        turn_id: turn_id.clone(),
                        hook_trace_records: hook_trace_records.clone(),
                        updated_at: *updated_at,
                    });
                    if matches!(outcome, PersistCommandOutcome::Succeeded) {
                        if is_authoritative {
                            return;
                        }
                    } else {
                        report_index_deferred(session_id, &outcome);
                    }
                }
                SessionTraceMutation::ReplaceAll { .. } => {
                    let outcome = fold::persist_graph_projection_commands(
                        self.backend.as_ref(),
                        session_id,
                        session,
                        &effective_mutation,
                    );
                    if matches!(outcome, PersistCommandOutcome::Succeeded) {
                        if is_authoritative {
                            return;
                        }
                    } else {
                        report_index_deferred(session_id, &outcome);
                    }
                }
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
                    session.trace_migration_state = match session.trace_migration_state {
                        TraceMigrationState::LegacyBlob | TraceMigrationState::DualWrite => {
                            TraceMigrationState::DualWrite
                        }
                        TraceMigrationState::TraceTableAuthoritative => {
                            TraceMigrationState::TraceTableAuthoritative
                        }
                    };
                }
            }
            self.note_version_from_backend(session_id);
            return;
        }

        if matches!(result, SessionBackendMutationResult::NotFound)
            && matches!(
                prepared.trace_migration_state,
                TraceMigrationState::TraceTableAuthoritative
            )
        {
            let replace_all = SessionTraceMutation::ReplaceAll {
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
                self.note_version_from_backend(session_id);
                return;
            }
        }

        if matches!(result, SessionBackendMutationResult::Failed) {
            eprintln!(
                "[pony-agent][session] persist_session_and_trace_change backend write failed for session {session_id}; falling back to separate blob+table writes"
            );
        }

        self.persist_trace_change(session_id, trace_action_from_mutation(mutation));
        self.save_session_to_backend(session_id);
    }
}
