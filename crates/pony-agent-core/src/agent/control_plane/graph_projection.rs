// graph_projection: checkpoint / submission plan / control boundary / run control 投影。
// 由 control_plane/mod.rs 的 HostControlPlane 巨型 impl 拆分而来，行为与结构保持一致。
use super::*;

impl HostControlPlane {
    pub(crate) fn execution_checkpoint_from_graph_run_checkpoint(
        checkpoint: GraphRunCheckpoint,
    ) -> ExecutionCheckpoint {
        let phase = match checkpoint.phase {
            GraphRunPhase::Ready => "ready".to_string(),
            GraphRunPhase::Running => "running".to_string(),
            GraphRunPhase::WaitingUser => "waiting_user".to_string(),
            GraphRunPhase::Paused => "paused".to_string(),
            GraphRunPhase::Completed => "completed".to_string(),
            GraphRunPhase::Failed => "failed".to_string(),
            GraphRunPhase::Cancelled => "cancelled".to_string(),
        };
        let status = match checkpoint.phase {
            GraphRunPhase::Completed => "completed".to_string(),
            GraphRunPhase::Failed => "failed".to_string(),
            GraphRunPhase::Cancelled => "cancelled".to_string(),
            _ => "ready".to_string(),
        };
        let fallback_reason = checkpoint
            .stop_reason
            .as_ref()
            .map(|reason| match reason {
                GraphRunStopReason::UserStop => "graph_user_stop",
                GraphRunStopReason::Timeout => "graph_timeout",
                GraphRunStopReason::BudgetExhausted => "graph_budget_exhausted",
                GraphRunStopReason::ConsecutiveError => "graph_consecutive_error",
                GraphRunStopReason::RuntimeCancelled => "graph_runtime_cancelled",
                GraphRunStopReason::RuntimeFailed => "graph_runtime_failed",
            })
            .map(str::to_string);

        let mut checkpoint = ExecutionCheckpoint {
            contract_version: execution_checkpoint_contract_version().to_string(),
            turn_id: checkpoint
                .active_turn_id
                .clone()
                .or(checkpoint.last_completed_turn_id.clone())
                .unwrap_or_else(|| format!("graph-run:{}", checkpoint.run_id)),
            session_id: checkpoint.session_id,
            run_id: Some(checkpoint.run_id),
            checkpoint_kind: "recovery".to_string(),
            recovery_mode: if checkpoint.resumable {
                "persisted_effect".to_string()
            } else {
                "replay_required".to_string()
            },
            projected_runtime_phase: "ready".to_string(),
            submission_command: None,
            resumable: checkpoint.resumable,
            replayable: true,
            status,
            phase,
            provider_requested_name: checkpoint
                .last_handoff
                .as_ref()
                .map(|handoff| handoff.provider_name.clone()),
            provider_name: checkpoint
                .last_handoff
                .as_ref()
                .map(|handoff| handoff.provider_name.clone()),
            provider_protocol: None,
            provider_model: checkpoint
                .last_handoff
                .as_ref()
                .map(|handoff| handoff.provider_model.clone()),
            provider_source: Some("graph_checkpoint".to_string()),
            provider_mode: Some("recovery".to_string()),
            fallback_reason,
            completed_hops: checkpoint.steps.len(),
            max_hops: checkpoint.steps.len(),
            active_tool_name: None,
            trace_steps: Vec::new(),
            tool_activities: Vec::new(),
            persisted_effect_evidence: Vec::new(),
            error: None,
            started_at_ms: checkpoint.created_at_ms,
            updated_at_ms: checkpoint.updated_at_ms,
            stop_requested_at_ms: None,
        };
        refresh_execution_checkpoint_projection(&mut checkpoint);
        checkpoint
    }

    pub(crate) fn load_trace_boundary_checkpoint(
        &self,
        query: ExecutionCheckpointQuery,
    ) -> Option<ExecutionCheckpoint> {
        let session_id = query.session_id?;
        let snapshot = self
            .sessions_rwlock
            .read()
            .unwrap_or_else(|e| {
                eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
                e.into_inner()
            })
            .snapshot_at_readonly(Some(session_id.as_str()), None, &[]);
        let trace = if let Some(turn_id) = query.turn_id.as_deref() {
            snapshot.turn_trace_history.iter().find(|trace| {
                trace.turn_id == turn_id && Self::trace_has_checkpoint_boundary(trace)
            })
        } else {
            snapshot
                .turn_trace_history
                .iter()
                .rev()
                .find(|trace| Self::trace_has_checkpoint_boundary(trace))
        }?;
        Some(Self::execution_checkpoint_from_trace_boundary(trace))
    }

    pub(crate) fn trace_has_checkpoint_boundary(trace: &TurnTraceRecord) -> bool {
        trace
            .trace_timeline
            .iter()
            .any(|entry| entry.kind == "checkpoint_persist")
    }

    pub(crate) fn execution_checkpoint_from_trace_boundary(
        trace: &TurnTraceRecord,
    ) -> ExecutionCheckpoint {
        let phase = if Self::trace_has_checkpoint_boundary(trace) {
            "checkpointing".to_string()
        } else {
            trace.phase.clone()
        };
        let status = match trace.phase.as_str() {
            "failed" => "failed".to_string(),
            "cancelled" => "cancelled".to_string(),
            _ => "completed".to_string(),
        };
        let mut checkpoint = ExecutionCheckpoint {
            contract_version: execution_checkpoint_contract_version().to_string(),
            turn_id: trace.turn_id.clone(),
            session_id: trace.session_id.clone(),
            run_id: None,
            checkpoint_kind: "lifecycle_boundary".to_string(),
            recovery_mode: "replay_required".to_string(),
            projected_runtime_phase: String::new(),
            submission_command: None,
            resumable: false,
            replayable: false,
            status,
            phase,
            provider_requested_name: trace.provider_requested_name.clone(),
            provider_name: trace.provider_name.clone(),
            provider_protocol: trace.provider_protocol.clone(),
            provider_model: trace.provider_model.clone(),
            provider_source: trace.provider_source.clone(),
            provider_mode: trace.provider_mode.clone(),
            fallback_reason: trace.fallback_reason.clone(),
            completed_hops: trace
                .trace_timeline
                .iter()
                .filter(|entry| entry.kind == "call_tool")
                .count(),
            max_hops: trace
                .trace_timeline
                .iter()
                .filter(|entry| entry.kind == "call_tool")
                .count(),
            active_tool_name: None,
            trace_steps: trace.trace_steps.clone(),
            tool_activities: trace.tool_activities.clone(),
            persisted_effect_evidence: Vec::new(),
            error: trace.error.clone(),
            started_at_ms: trace.emitted_at_ms.unwrap_or(trace.updated_at),
            updated_at_ms: trace.updated_at,
            stop_requested_at_ms: None,
        };
        refresh_execution_checkpoint_projection(&mut checkpoint);
        checkpoint
    }

    pub(crate) fn should_project_graph_checkpoint_as_recovery(
        checkpoint: &GraphRunCheckpoint,
    ) -> bool {
        checkpoint.resumable
    }

    pub(crate) fn attach_session_persisted_effect_evidence(
        &self,
        checkpoint: &mut ExecutionCheckpoint,
        session_id_hint: Option<&str>,
    ) {
        let session_id = checkpoint
            .session_id
            .as_deref()
            .or(session_id_hint)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let Some(session_id) = session_id else {
            checkpoint.persisted_effect_evidence.clear();
            return;
        };

        let snapshot = self
            .sessions_rwlock
            .read()
            .unwrap_or_else(|e| {
                eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
                e.into_inner()
            })
            .snapshot_at_readonly(Some(session_id), None, &[]);
        let relevant_history_node_id =
            Self::resolve_checkpoint_history_node_id(&snapshot, checkpoint);
        checkpoint.persisted_effect_evidence = snapshot
            .memory_write_evidence
            .iter()
            .filter(|evidence| {
                relevant_history_node_id
                    .as_deref()
                    .is_some_and(|history_node_id| {
                        evidence.source_history_node_id.as_deref() == Some(history_node_id)
                    })
            })
            .cloned()
            .collect();

        if checkpoint.checkpoint_kind == "lifecycle_boundary" {
            if checkpoint.persisted_effect_evidence.is_empty() {
                checkpoint.recovery_mode = "replay_required".to_string();
                checkpoint.replayable = false;
            } else {
                checkpoint.recovery_mode = "persisted_effect".to_string();
                checkpoint.replayable = true;
            }
            refresh_execution_checkpoint_projection(checkpoint);
        }
    }

    pub(crate) fn resolve_checkpoint_history_node_id(
        snapshot: &SessionSnapshot,
        checkpoint: &ExecutionCheckpoint,
    ) -> Option<String> {
        snapshot
            .history_nodes
            .iter()
            .rev()
            .find(|node| node.run_id.as_deref() == Some(checkpoint.turn_id.as_str()))
            .map(|node| node.node_id.clone())
            .or_else(|| snapshot.history_cursor.visible_node_id.clone())
    }

    pub(crate) fn build_graph_run_submission_plan(
        command: String,
        run_id: Option<String>,
        source: &str,
        checkpoint: Option<&ExecutionCheckpoint>,
    ) -> GraphRunSubmissionPlan {
        let checkpoint_context = checkpoint.map(|checkpoint| RunControlCheckpointContext {
            session_id: checkpoint.session_id.clone(),
            run_id: checkpoint.run_id.clone(),
            phase: checkpoint.phase.clone(),
            checkpoint_kind: checkpoint.checkpoint_kind.clone(),
            recovery_mode: checkpoint.recovery_mode.clone(),
            resumable: checkpoint.resumable,
            replayable: checkpoint.replayable,
        });
        let hook_envelope = build_submission_plan_run_control_hook_envelope(
            &command,
            run_id.as_deref(),
            source,
            checkpoint_context.as_ref(),
        );
        GraphRunSubmissionPlan {
            command,
            run_id,
            source: source.to_string(),
            hook_point: "submission_plan_resolved".to_string(),
            canonical_event_type: CanonicalGraphRunEventType::SubmissionPlanResolved
                .as_str()
                .to_string(),
            canonical_phase: "ready".to_string(),
            hook_envelope,
        }
    }

    pub(crate) fn resolve_submission_command_against_run(
        run: Option<&GraphRun>,
        requested_command: &str,
    ) -> (String, Option<String>, &'static str) {
        let Some(run) = run else {
            let command = if requested_command == "start_graph_run_stream" {
                "start_graph_run_stream"
            } else {
                "start_graph_run_stream"
            };
            return (command.to_string(), None, "default");
        };

        let run_id = Some(run.id.clone());
        let is_terminal = matches!(
            run.phase,
            GraphRunPhase::Completed | GraphRunPhase::Failed | GraphRunPhase::Cancelled
        );
        let can_resume = run.phase == GraphRunPhase::Paused && run.active_turn_id.is_none();
        let can_continue = !is_terminal && run.active_turn_id.is_none();

        let command = match requested_command {
            "resume_graph_run_stream" if can_resume => "resume_graph_run_stream",
            "resume_graph_run_stream" if can_continue => "continue_graph_run_stream",
            "continue_graph_run_stream" if can_continue => "continue_graph_run_stream",
            "start_graph_run_stream" => "start_graph_run_stream",
            _ if is_terminal => "start_graph_run_stream",
            _ if can_continue => "continue_graph_run_stream",
            _ => "start_graph_run_stream",
        };

        let run_id = if command == "start_graph_run_stream" {
            None
        } else {
            run_id
        };
        let source = if command == requested_command {
            "graph_run"
        } else {
            "graph_run_reconciled"
        };

        (command.to_string(), run_id, source)
    }

    pub(crate) fn graph_run_phase_token(phase: &GraphRunPhase) -> &'static str {
        match phase {
            GraphRunPhase::Ready => "ready",
            GraphRunPhase::Running => "running",
            GraphRunPhase::WaitingUser => "waiting_user",
            GraphRunPhase::Paused => "paused",
            GraphRunPhase::Completed => "completed",
            GraphRunPhase::Failed => "failed",
            GraphRunPhase::Cancelled => "cancelled",
        }
    }

    pub(crate) fn build_graph_run_command_boundary_evidence(
        command: &str,
        hook_point: &str,
        canonical_event_type: &str,
        source: &str,
        run: &GraphRun,
        summary: &str,
    ) -> Option<GraphRunControlBoundaryEvidence> {
        let checkpoint_context = RunControlCheckpointContext {
            session_id: run.session_id.clone(),
            run_id: Some(run.id.clone()),
            phase: Self::graph_run_phase_token(&run.phase).to_string(),
            checkpoint_kind: "runtime_control".to_string(),
            recovery_mode: "replay_required".to_string(),
            resumable: matches!(
                run.phase,
                GraphRunPhase::Ready | GraphRunPhase::WaitingUser | GraphRunPhase::Paused
            ),
            replayable: false,
        };
        let hook_envelope = build_submission_plan_run_control_hook_envelope(
            command,
            Some(run.id.as_str()),
            source,
            Some(&checkpoint_context),
        )?;
        Some(GraphRunControlBoundaryEvidence {
            hook_point: hook_point.to_string(),
            canonical_event_type: canonical_event_type.to_string(),
            canonical_phase: hook_envelope.phase.clone(),
            summary: summary.to_string(),
            hook_envelope,
            created_at_ms: run.updated_at_ms,
        })
    }

    pub(crate) fn load_active_graph_run_for_session(
        &self,
        session_id: Option<&str>,
    ) -> Option<GraphRun> {
        let session_id = session_id
            .map(str::trim)
            .filter(|session_id| !session_id.is_empty())?;
        let graph_runs = self.graph_runs.lock().unwrap_or_else(|e| {
            eprintln!("[pony-agent] graph run lock poisoned: {e}, recovering");
            e.into_inner()
        });
        graph_runs.list_runs().into_iter().find(|run| {
            run.session_id.as_deref() == Some(session_id)
                && !matches!(
                    run.phase,
                    GraphRunPhase::Completed | GraphRunPhase::Failed | GraphRunPhase::Cancelled
                )
        })
    }

    pub(crate) fn resolve_graph_run_for_retrieval(
        &self,
        run_id: Option<&str>,
        session_id: Option<&str>,
    ) -> Option<GraphRun> {
        run_id
            .and_then(|run_id| {
                self.load_graph_run(GraphRunQuery {
                    run_id: Some(run_id.to_string()),
                })
            })
            .or_else(|| self.load_active_graph_run_for_session(session_id))
    }

    pub(crate) fn project_runtime_view_control_boundary_evidence(
        run: Option<&GraphRun>,
    ) -> Option<Vec<GraphRunControlBoundaryEvidence>> {
        let evidence = run
            .map(|run| run.control_boundary_evidence.clone())
            .unwrap_or_default();
        (!evidence.is_empty()).then_some(evidence)
    }

    pub(crate) fn project_history_state_evidence(
        snapshot: &SessionSnapshot,
    ) -> Option<Vec<HistoryStateHookEvidence>> {
        (!snapshot.history_state_evidence.is_empty())
            .then_some(snapshot.history_state_evidence.clone())
    }

    pub(crate) fn project_history_state_audit_summary(
        snapshot: &SessionSnapshot,
    ) -> HistoryStateAuditSummary {
        snapshot.history_state_audit_summary.clone()
    }

    pub(crate) fn project_run_control_start_reason(
        command: Option<&str>,
        checkpoint: Option<&ExecutionCheckpoint>,
        submission_plan: Option<&GraphRunSubmissionPlan>,
    ) -> Option<String> {
        let command = command
            .map(str::to_string)
            .or_else(|| submission_plan.map(|item| item.command.clone()))
            .or_else(|| checkpoint.and_then(|item| item.submission_command.clone()))?;
        if command != "start_graph_run_stream" {
            return None;
        }
        let checkpoint = checkpoint?;
        if checkpoint.checkpoint_kind == "lifecycle_boundary"
            || checkpoint.recovery_mode == "replay_required"
        {
            return Some("replay_from_checkpoint".to_string());
        }
        Some("restart_from_checkpoint".to_string())
    }

    pub(crate) fn project_run_control_audit_summary(
        run: Option<&GraphRun>,
        checkpoint: Option<&ExecutionCheckpoint>,
        submission_plan: Option<&GraphRunSubmissionPlan>,
        response_evidence: Option<&GraphRunControlBoundaryEvidence>,
    ) -> RunControlAuditSummary {
        let mut summary = build_missing_run_control_audit_summary();
        summary.current_context_projection = RunControlAuditCurrentContext {
            phase: run
                .map(|item| Self::graph_run_phase_token(&item.phase).to_string())
                .or_else(|| checkpoint.map(|item| item.projected_runtime_phase.clone()))
                .unwrap_or_else(|| "idle".to_string()),
            checkpoint_status: checkpoint
                .map(|item| item.status.clone())
                .unwrap_or_else(|| "missing".to_string()),
            active_run_id: run
                .map(|item| item.id.clone())
                .or_else(|| checkpoint.and_then(|item| item.run_id.clone())),
            checkpoint_kind: checkpoint.map(|item| item.checkpoint_kind.clone()),
            checkpoint_recovery_mode: checkpoint.map(|item| item.recovery_mode.clone()),
            submission_plan_command: submission_plan.map(|item| item.command.clone()),
        };

        let evidence = response_evidence
            .cloned()
            .or_else(|| run.and_then(|item| item.control_boundary_evidence.last().cloned()));
        let Some(evidence) = evidence else {
            return summary;
        };

        let command_kind = evidence
            .hook_envelope
            .command
            .as_submission_command()
            .to_string();
        let start_reason = Self::project_run_control_start_reason(
            Some(command_kind.as_str()),
            checkpoint,
            submission_plan,
        );
        if command_kind == "start_graph_run_stream" && start_reason.is_none() {
            return summary;
        }

        let degraded = checkpoint
            .map(|item| item.recovery_mode == "replay_required")
            .unwrap_or(false);
        let result_kind = match command_kind.as_str() {
            "stop_graph_run" => "accepted",
            "resume_graph_run_stream" => "resumed",
            "continue_graph_run_stream" => "continued",
            "start_graph_run_stream" if degraded => "replay_required",
            "start_graph_run_stream" => "started",
            _ => "available",
        };
        let run_phase = run
            .map(|item| Self::graph_run_phase_token(&item.phase).to_string())
            .unwrap_or_else(|| "unknown".to_string());
        summary.action_evidence_summary = RunControlAuditActionSummary {
            status: "available".to_string(),
            source_family: "run_control".to_string(),
            command_kind: Some(command_kind.clone()),
            boundary: Some(evidence.hook_point.clone()),
            result_kind: Some(result_kind.to_string()),
            summary: evidence.summary.clone(),
            target_summary: format!(
                "run {} · phase {}",
                run.map(|item| item.id.as_str())
                    .or(checkpoint.and_then(|item| item.run_id.as_deref()))
                    .unwrap_or("unknown"),
                run_phase
            ),
            elapsed_ms: Some(0),
            blocked: false,
            degraded,
            evidence_id: Some(format!(
                "{}:{}",
                evidence.created_at_ms, evidence.hook_point
            )),
            observed_at_ms: Some(evidence.created_at_ms),
            run_id: run
                .map(|item| item.id.clone())
                .or_else(|| checkpoint.and_then(|item| item.run_id.clone())),
            turn_id: run.and_then(|item| item.active_turn_id.clone()),
            checkpoint_turn_id: checkpoint.map(|item| item.turn_id.clone()),
            checkpoint_kind: checkpoint.map(|item| item.checkpoint_kind.clone()),
            recovery_mode: checkpoint.map(|item| item.recovery_mode.clone()),
            projected_command: submission_plan.map(|item| item.command.clone()),
            degradation_reason: degraded.then(|| "replay_required".to_string()),
            request_summary: None,
            start_reason,
        };
        summary
    }

    pub(crate) fn normalize_history_session_id(&self, session_id: Option<String>) -> String {
        session_id
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| {
                self.sessions_rwlock
                    .read()
                    .unwrap_or_else(|e| {
                        eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
                        e.into_inner()
                    })
                    .snapshot_at_readonly(None, None, &[])
                    .conversation_id
            })
    }

    pub(crate) fn history_cursor_mode_from_session(cursor: &HistoryCursor) -> HistoryCursorMode {
        match cursor.mode {
            crate::agent::session::HistoryCursorMode::Live => HistoryCursorMode::Live,
            crate::agent::session::HistoryCursorMode::Historical => HistoryCursorMode::Historical,
            crate::agent::session::HistoryCursorMode::HistoricalDirty => {
                HistoryCursorMode::HistoricalDirty
            }
        }
    }

    pub(crate) fn history_checkout_mode_to_session(
        mode: HistoryCheckoutMode,
    ) -> SessionHistoryCheckoutMode {
        match mode {
            HistoryCheckoutMode::TranscriptOnly => SessionHistoryCheckoutMode::TranscriptOnly,
            HistoryCheckoutMode::TranscriptAndWorkspace => {
                SessionHistoryCheckoutMode::TranscriptAndWorkspace
            }
        }
    }

    pub(crate) fn history_checkout_mode_from_session(
        mode: crate::agent::session::HistoryCheckoutMode,
    ) -> HistoryCheckoutMode {
        match mode {
            crate::agent::session::HistoryCheckoutMode::TranscriptOnly => {
                HistoryCheckoutMode::TranscriptOnly
            }
            crate::agent::session::HistoryCheckoutMode::TranscriptAndWorkspace => {
                HistoryCheckoutMode::TranscriptAndWorkspace
            }
        }
    }

    pub(crate) fn history_node_view(node: &HistoryNode) -> HistoryNodeView {
        HistoryNodeView {
            node_id: node.node_id.clone(),
            session_id: node.session_id.clone(),
            parent_node_id: node.parent_node_id.clone(),
            branch_id: node.branch_id.clone(),
            forked_from_node_id: node.forked_from_node_id.clone(),
            kind: serde_json::to_value(&node.kind)
                .ok()
                .and_then(|value| value.as_str().map(str::to_string))
                .unwrap_or_else(|| "turn_committed".to_string()),
            turn_id: node
                .turn_trace_history
                .last()
                .map(|trace| trace.turn_id.clone()),
            workspace_ref: node.workspace_ref.clone(),
            summary: Some(node.summary.clone()),
            title: Some(node.title.clone()),
            turn_count: Some(node.turn_count),
            created_at_ms: Some(node.created_at_ms),
        }
    }

    pub(crate) fn history_branch_view(branch: &HistoryBranch) -> HistoryBranchView {
        HistoryBranchView {
            branch_id: branch.branch_id.clone(),
            session_id: branch.session_id.clone(),
            base_node_id: branch.base_node_id.clone(),
            head_node_id: branch.head_node_id.clone(),
            forked_from_branch_id: branch.forked_from_branch_id.clone(),
            forked_from_node_id: branch.forked_from_node_id.clone(),
            label: branch.label.clone(),
            created_at_ms: Some(branch.created_at_ms),
            updated_at_ms: Some(branch.updated_at_ms),
        }
    }

    pub(crate) fn history_cursor_state(cursor: &HistoryCursor) -> HistoryCursorState {
        let is_at_branch_head = cursor.visible_node_id == cursor.branch_head_node_id;
        HistoryCursorState {
            session_id: cursor.session_id.clone(),
            visible_node_id: cursor.visible_node_id.clone(),
            active_branch_id: cursor.active_branch_id.clone(),
            branch_head_node_id: cursor.branch_head_node_id.clone(),
            workspace_node_id: cursor.workspace_node_id.clone(),
            mode: Self::history_cursor_mode_from_session(cursor),
            authority_mode: "host_authoritative".to_string(),
            cursor_version: Some(cursor.cursor_version),
            is_at_branch_head,
        }
    }
}
