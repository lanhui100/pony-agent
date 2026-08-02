// history_commands: 历史 graph / 会话 runtime view / 分支变更 / inspect / delete 命令。
// 由 control_plane/mod.rs 的 HostControlPlane 巨型 impl 拆分而来，行为与结构保持一致。
use super::*;

impl HostControlPlane {
    pub fn load_graph_run_checkpoint(
        &self,
        query: GraphRunCheckpointQuery,
    ) -> Option<GraphRunCheckpoint> {
        let graph_runs = self.graph_runs.lock().unwrap_or_else(|e| {
            eprintln!("[pony-agent] graph run lock poisoned: {e}, recovering");
            e.into_inner()
        });
        let run_id = query.run_id?;
        let run = graph_runs.load_run(&run_id)?;
        Some(self.graph_runner.build_checkpoint(&run))
    }

    pub fn load_session_snapshot(&self, query: SessionSnapshotQuery) -> SessionSnapshot {
        let mut snapshot = self
            .sessions_rwlock
            .read()
            .unwrap_or_else(|e| {
                eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
                e.into_inner()
            })
            .snapshot_at_readonly(query.session_id.as_deref(), None, &[]);
        let checkpoint = self.load_execution_checkpoint(ExecutionCheckpointQuery {
            turn_id: None,
            session_id: Some(snapshot.conversation_id.clone()),
        });
        let run =
            self.resolve_graph_run_for_retrieval(None, Some(snapshot.conversation_id.as_str()));
        let submission_plan = Some(self.resolve_graph_run_submission_plan(
            GraphRunSubmissionPlanQuery {
                session_id: Some(snapshot.conversation_id.clone()),
                node_id: None,
                run_id: run.as_ref().map(|item| item.id.clone()),
            },
        ));
        snapshot.run_control_audit_summary = Self::project_run_control_audit_summary(
            run.as_ref(),
            checkpoint.as_ref(),
            submission_plan.as_ref(),
            None,
        );
        snapshot
    }

    pub fn load_history_graph(&self, query: HistoryGraphQuery) -> HistoryGraphView {
        let session_id = self.normalize_history_session_id(query.session_id);
        let (nodes, branches, cursor) = self
            .sessions_rwlock
            .write()
            .unwrap_or_else(|e| {
                eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
                e.into_inner()
            })
            .load_history_graph(Some(session_id.as_str()));
        HistoryGraphView {
            session_id,
            nodes: nodes.iter().map(Self::history_node_view).collect(),
            branches: branches.iter().map(Self::history_branch_view).collect(),
            cursor: Self::history_cursor_state(&cursor),
        }
    }

    pub fn load_history_cursor(&self, query: HistoryCursorQuery) -> HistoryCursorState {
        let session_id = self.normalize_history_session_id(query.session_id);
        let cursor = self
            .sessions_rwlock
            .write()
            .unwrap_or_else(|e| {
                eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
                e.into_inner()
            })
            .load_history_cursor(Some(session_id.as_str()));
        Self::history_cursor_state(&cursor)
    }

    pub fn checkout_history_node(
        &self,
        command: CheckoutHistoryNodeCommand,
    ) -> Result<HistoryCheckoutResponse, String> {
        let session_id = self.normalize_history_session_id(command.session_id);
        let requested_mode = command.mode;
        let mut sessions = self.sessions_rwlock.write().unwrap_or_else(|e| {
            eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
            e.into_inner()
        });
        let before_state = Self::project_message_state(&sessions.snapshot_at(
            Some(session_id.as_str()),
            None,
            &[],
        ));
        let snapshot = sessions.checkout_history_node(
            Some(session_id.as_str()),
            &command.node_id,
            Self::history_checkout_mode_to_session(requested_mode),
            command.expected_cursor_version,
        )?;
        drop(sessions);
        let message_state = Self::project_message_state(&snapshot);
        let message_delta = Self::diff_message_state(&before_state, &message_state);
        let cursor = Self::history_cursor_state(&snapshot.history_cursor);
        let applied_mode =
            Self::history_checkout_mode_from_session(snapshot.history_cursor.checkout_mode.clone());
        let degraded = matches!(
            snapshot.history_cursor.checkout_status,
            crate::agent::session::HistoryCheckoutStatus::DegradedToTranscriptOnly
        );
        let history_state_evidence = Self::project_history_state_evidence(&snapshot);
        Ok(HistoryCheckoutResponse {
            session_id,
            node_id: command.node_id,
            requested_mode,
            applied_mode,
            transcript_restore_applied: true,
            workspace_rollback_capable: snapshot
                .history_nodes
                .iter()
                .find(|node| Some(node.node_id.as_str()) == snapshot.resolved_node_id.as_deref())
                .map(|node| node.workspace_ref.rollback_capable)
                .unwrap_or(false),
            workspace_rollback_applied: requested_mode
                == HistoryCheckoutMode::TranscriptAndWorkspace
                && !degraded,
            degraded,
            degradation_reason: degraded.then(|| "workspace_rollback_unsupported".to_string()),
            history_state_evidence,
            history_state_audit_summary: Self::project_history_state_audit_summary(&snapshot),
            message_revision: message_state.revision,
            message_delta,
            cursor,
        })
    }

    pub fn restore_branch_head(
        &self,
        command: RestoreBranchHeadCommand,
    ) -> Result<RestoreBranchHeadResponse, String> {
        let session_id = self.normalize_history_session_id(command.session_id);
        let mut sessions = self.sessions_rwlock.write().unwrap_or_else(|e| {
            eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
            e.into_inner()
        });
        let before_state = Self::project_message_state(&sessions.snapshot_at(
            Some(session_id.as_str()),
            None,
            &[],
        ));
        let snapshot = sessions.restore_branch_head(
            Some(session_id.as_str()),
            command.branch_id.as_deref(),
            command.expected_cursor_version,
        )?;
        drop(sessions);
        let message_state = Self::project_message_state(&snapshot);
        let message_delta = Self::diff_message_state(&before_state, &message_state);
        let workspace_rollback_capable = snapshot
            .history_nodes
            .iter()
            .find(|node| Some(node.node_id.as_str()) == snapshot.resolved_node_id.as_deref())
            .map(|node| node.workspace_ref.rollback_capable)
            .unwrap_or(false);
        Ok(RestoreBranchHeadResponse {
            session_id,
            branch_id: snapshot.history_cursor.active_branch_id.clone(),
            restored_node_id: snapshot.resolved_node_id.clone(),
            transcript_restore_applied: true,
            workspace_rollback_capable,
            workspace_rollback_applied: false,
            degraded: false,
            degradation_reason: None,
            history_state_evidence: Self::project_history_state_evidence(&snapshot),
            history_state_audit_summary: Self::project_history_state_audit_summary(&snapshot),
            message_revision: message_state.revision,
            message_delta,
            cursor: Self::history_cursor_state(&snapshot.history_cursor),
        })
    }

    pub fn fork_from_history_node(
        &self,
        command: ForkFromHistoryNodeCommand,
    ) -> Result<ForkFromHistoryNodeResponse, String> {
        let session_id = self.normalize_history_session_id(command.session_id);
        let mut sessions = self.sessions_rwlock.write().unwrap_or_else(|e| {
            eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
            e.into_inner()
        });
        let before_state = Self::project_message_state(&sessions.snapshot_at(
            Some(session_id.as_str()),
            None,
            &[],
        ));
        let before = sessions.load_history_cursor(Some(session_id.as_str()));
        let snapshot = sessions.fork_from_history_node(
            Some(session_id.as_str()),
            &command.node_id,
            command.expected_cursor_version,
        )?;
        drop(sessions);
        let message_state = Self::project_message_state(&snapshot);
        let message_delta = Self::diff_message_state(&before_state, &message_state);
        let created_branch_id = snapshot
            .history_cursor
            .active_branch_id
            .clone()
            .ok_or_else(|| "fork did not produce an active branch".to_string())?;
        let branch = snapshot
            .history_branches
            .iter()
            .find(|item| item.branch_id == created_branch_id)
            .cloned()
            .ok_or_else(|| "forked branch not found in snapshot".to_string())?;
        let mut branch_view = Self::history_branch_view(&branch);
        if branch_view.forked_from_branch_id.is_none() {
            branch_view.forked_from_branch_id = before.active_branch_id;
        }
        Ok(ForkFromHistoryNodeResponse {
            session_id,
            node_id: command.node_id,
            branch: branch_view,
            history_state_evidence: Self::project_history_state_evidence(&snapshot),
            history_state_audit_summary: Self::project_history_state_audit_summary(&snapshot),
            message_revision: message_state.revision,
            message_delta,
            cursor: Self::history_cursor_state(&snapshot.history_cursor),
        })
    }

    pub fn switch_history_branch(
        &self,
        command: SwitchHistoryBranchCommand,
    ) -> Result<SwitchHistoryBranchResponse, String> {
        let session_id = self.normalize_history_session_id(command.session_id);
        let mut sessions = self.sessions_rwlock.write().unwrap_or_else(|e| {
            eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
            e.into_inner()
        });
        let before_state = Self::project_message_state(&sessions.snapshot_at(
            Some(session_id.as_str()),
            None,
            &[],
        ));
        let snapshot = sessions.switch_history_branch(
            Some(session_id.as_str()),
            &command.branch_id,
            command.expected_cursor_version,
        )?;
        drop(sessions);
        let message_state = Self::project_message_state(&snapshot);
        let message_delta = Self::diff_message_state(&before_state, &message_state);
        Ok(SwitchHistoryBranchResponse {
            session_id,
            branch_id: command.branch_id,
            node_id: snapshot.resolved_node_id.clone(),
            history_state_evidence: Self::project_history_state_evidence(&snapshot),
            history_state_audit_summary: Self::project_history_state_audit_summary(&snapshot),
            message_revision: message_state.revision,
            message_delta,
            cursor: Self::history_cursor_state(&snapshot.history_cursor),
        })
    }

    pub fn load_session_runtime_view(&self, query: SessionRuntimeViewQuery) -> SessionRuntimeView {
        let checkpoint = self.load_execution_checkpoint(ExecutionCheckpointQuery {
            turn_id: query.turn_id,
            session_id: query.session_id.clone(),
        });
        let resolved_session_id =
            self.normalize_history_session_id(query.session_id.or_else(|| {
                checkpoint
                    .as_ref()
                    .and_then(|checkpoint| checkpoint.session_id.clone())
            }));

        // If no explicit node_id, fall back to the session cursor's visible
        // node so that checkout_history_node state is preserved across
        // client reconnects / session switches without every client
        // having to track visibleNodeId.
        let resolved_node_id = query.node_id.clone().or_else(|| {
            let cursor = self.load_history_cursor(HistoryCursorQuery {
                session_id: Some(resolved_session_id.clone()),
            });
            cursor.visible_node_id.filter(|visible| {
                cursor
                    .branch_head_node_id
                    .as_ref()
                    .map_or(true, |head| visible != head)
            })
        });

        let run = self.resolve_graph_run_for_retrieval(
            query.run_id.as_deref(),
            Some(resolved_session_id.as_str()),
        );
        let submission_plan = Some(self.resolve_graph_run_submission_plan(
            GraphRunSubmissionPlanQuery {
                session_id: Some(resolved_session_id.clone()),
                node_id: resolved_node_id.clone(),
                run_id: query.run_id.clone(),
            },
        ));
        let session = self
            .sessions_rwlock
            .read()
            .unwrap_or_else(|e| {
                eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
                e.into_inner()
            })
            .snapshot_at_readonly(
                Some(resolved_session_id.as_str()),
                resolved_node_id.as_deref(),
                &[],
            );
        let retrieved = self
            .runtime
            .read()
            .expect("runtime lock poisoned")
            .inspect_retrieved_context_at(
                Some(resolved_session_id.as_str()),
                resolved_node_id.as_deref(),
                run.as_ref(),
                checkpoint.as_ref(),
                None,
            );

        let resolved_visible_node_id = session.history_cursor.visible_node_id.clone();
        let active_branch_head_node_id = session.history_cursor.branch_head_node_id.clone();
        let is_at_branch_head = resolved_visible_node_id == active_branch_head_node_id;
        let message_state = Self::project_message_state(&session);
        SessionRuntimeView {
            message_state,
            history_state_evidence: Self::project_history_state_evidence(&session),
            history_state_audit_summary: Self::project_history_state_audit_summary(&session),
            run_control_audit_summary: Self::project_run_control_audit_summary(
                run.as_ref(),
                checkpoint.as_ref(),
                submission_plan.as_ref(),
                None,
            ),
            history_nodes: Some(
                session
                    .history_nodes
                    .iter()
                    .map(Self::history_node_view)
                    .collect(),
            ),
            history_branches: Some(
                session
                    .history_branches
                    .iter()
                    .map(Self::history_branch_view)
                    .collect(),
            ),
            history_cursor: Some(Self::history_cursor_state(&session.history_cursor)),
            control_boundary_evidence: Self::project_runtime_view_control_boundary_evidence(
                run.as_ref(),
            ),
            session,
            retrieved,
            checkpoint,
            submission_plan,
            authority_mode: "host_authoritative".to_string(),
            resolved_visible_node_id,
            active_branch_head_node_id,
            is_at_branch_head,
            cursor_version: Some(0),
        }
    }

    pub fn resolve_graph_run_submission_plan(
        &self,
        query: GraphRunSubmissionPlanQuery,
    ) -> GraphRunSubmissionPlan {
        let run = query
            .run_id
            .as_deref()
            .and_then(|run_id| {
                self.graph_runs
                    .lock()
                    .unwrap_or_else(|e| {
                        eprintln!("[pony-agent] graph run lock poisoned, recovering: {e}");
                        e.into_inner()
                    })
                    .load_run(run_id)
            })
            .or_else(|| self.resolve_graph_run_for_retrieval(None, query.session_id.as_deref()));

        if let Some(checkpoint) = self.load_execution_checkpoint(ExecutionCheckpointQuery {
            turn_id: None,
            session_id: query.session_id.clone(),
        }) {
            if let Some(command) = checkpoint.submission_command.clone() {
                let checkpoint_run = run.as_ref().filter(|run| {
                    checkpoint.run_id.as_deref().or(query.run_id.as_deref())
                        == Some(run.id.as_str())
                });
                let (command, run_id, source) =
                    Self::resolve_submission_command_against_run(checkpoint_run, &command);
                return Self::build_graph_run_submission_plan(
                    command,
                    run_id,
                    if source == "default" {
                        "checkpoint"
                    } else {
                        "graph_run_reconciled"
                    },
                    Some(&checkpoint),
                );
            }
        }

        if let Some(run) = run.as_ref() {
            if run.phase == GraphRunPhase::Running {
                return Self::build_graph_run_submission_plan(
                    "continue_graph_run_stream".to_string(),
                    Some(run.id.clone()),
                    "graph_run",
                    None,
                );
            }
            let requested_command = match run.phase {
                GraphRunPhase::Completed | GraphRunPhase::Failed | GraphRunPhase::Cancelled => {
                    "start_graph_run_stream"
                }
                GraphRunPhase::Paused => "resume_graph_run_stream",
                _ => "continue_graph_run_stream",
            };
            let (command, run_id, source) =
                Self::resolve_submission_command_against_run(Some(run), requested_command);
            return Self::build_graph_run_submission_plan(command, run_id, source, None);
        }

        let _ = query.node_id;
        Self::build_graph_run_submission_plan(
            "start_graph_run_stream".to_string(),
            None,
            "default",
            None,
        )
    }

    pub fn load_retrieved_context(&self, query: RetrievedContextQuery) -> RetrievedContextState {
        let checkpoint = self.load_execution_checkpoint(ExecutionCheckpointQuery {
            turn_id: query.turn_id,
            session_id: query.session_id.clone(),
        });
        let resolved_session_id =
            self.normalize_history_session_id(query.session_id.or_else(|| {
                checkpoint
                    .as_ref()
                    .and_then(|checkpoint| checkpoint.session_id.clone())
            }));
        let run = self.resolve_graph_run_for_retrieval(
            query.run_id.as_deref(),
            Some(resolved_session_id.as_str()),
        );
        self.runtime
            .read()
            .expect("runtime lock poisoned")
            .inspect_retrieved_context_at(
                Some(resolved_session_id.as_str()),
                query.node_id.as_deref(),
                run.as_ref(),
                checkpoint.as_ref(),
                None,
            )
    }

    pub fn delete_session(&self, command: DeleteSessionCommand) -> Vec<SessionOverview> {
        self.sessions_rwlock
            .write()
            .unwrap_or_else(|e| {
                eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
                e.into_inner()
            })
            .remove_session(&command.session_id)
    }

    pub fn inspect(&self, query: HostInspectionQuery) -> HostInspectionSnapshot {
        let turn = self.load_execution_checkpoint(ExecutionCheckpointQuery {
            turn_id: query.turn_id.clone(),
            session_id: query.session_id.clone(),
        });
        let resolved_session_id = query.session_id.clone().or_else(|| {
            turn.as_ref()
                .and_then(|checkpoint| checkpoint.session_id.clone())
        });
        let run = (query.include_run || query.include_retrieved)
            .then(|| {
                self.resolve_graph_run_for_retrieval(
                    query.run_id.as_deref(),
                    resolved_session_id.as_deref(),
                )
            })
            .flatten();
        let retrieved = if query.include_retrieved {
            let runtime = self.runtime.read().expect("runtime lock poisoned");
            Some(runtime.inspect_retrieved_context(
                resolved_session_id.as_deref(),
                run.as_ref(),
                turn.as_ref(),
                None,
            ))
        } else {
            None
        };

        HostInspectionSnapshot {
            surface: "host-control-plane/v1".to_string(),
            turn,
            session: query.include_session.then(|| {
                self.load_session_snapshot(SessionSnapshotQuery {
                    session_id: resolved_session_id,
                })
            }),
            retrieved,
            sessions: query.include_sessions.then(|| self.list_sessions()),
            run: query.include_run.then_some(run).flatten(),
            runs: query.include_runs.then(|| self.list_graph_runs()),
        }
    }
}
