// session_ops: 会话 / 历史 / graph 状态查询与变更。
// 由 runtime/mod.rs 的 AgentRuntime 巨型 impl 拆分而来，行为与结构保持一致。
use super::*;

impl AgentRuntime {
    #[allow(dead_code)]
    pub fn start_graph_run(
        &self,
        run_id: impl Into<String>,
        goal: impl Into<String>,
        session_id: Option<&str>,
    ) -> GraphRun {
        self.graph.start_run(run_id, goal, session_id)
    }

    pub fn list_sessions(&self) -> Vec<SessionOverview> {
        self.sessions
            .read()
            .unwrap_or_else(|e| {
                eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
                e.into_inner()
            })
            .list_sessions()
    }

    pub fn load_turn_traces(&self, session_id: &str) -> Vec<TurnTraceRecord> {
        self.sessions
            .read()
            .unwrap_or_else(|e| {
                eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
                e.into_inner()
            })
            .load_turn_traces(session_id)
    }

    /// 当前会话已登记轮次数（ADR 0055：OTel `agent.turn.sequence` 的真值源）。
    ///
    /// 语义：返回**本轮开启前**已存在的轮次数，调用方可直接当作 0-based 序号。
    /// 此前两个 turn 入口（流式 `turn_stream` / 同步 `turn_sync`）均硬编码 `sequence = 0`，
    /// 导致所有 trace 同名 `pony-agent#turn-0@…`，按名检索与多轮聚合全部失真。
    pub fn session_turn_sequence(&self, session_id: Option<&str>) -> u64 {
        let session_key = session_id.unwrap_or(crate::agent::session::DEFAULT_SESSION_ID);
        let count = self
            .sessions
            .read()
            .unwrap_or_else(|e| {
                eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
                e.into_inner()
            })
            .turn_trace_count(session_key);
        u64::try_from(count).unwrap_or(u64::MAX)
    }

    pub fn load_session_snapshot(&self, session_id: Option<&str>) -> SessionSnapshot {
        self.load_session_snapshot_at(session_id, None)
    }

    pub fn load_session_snapshot_at(
        &self,
        session_id: Option<&str>,
        node_id: Option<&str>,
    ) -> SessionSnapshot {
        self.sessions
            .write()
            .unwrap_or_else(|e| {
                eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
                e.into_inner()
            })
            .snapshot_at(session_id, node_id, &[])
    }

    pub fn inspect_retrieved_context(
        &self,
        session_id: Option<&str>,
        run: Option<&GraphRun>,
        checkpoint: Option<&ExecutionCheckpoint>,
        workspace_mode: Option<&str>,
    ) -> RetrievedContextState {
        self.inspect_retrieved_context_at(session_id, None, run, checkpoint, workspace_mode)
    }

    pub fn inspect_retrieved_context_at(
        &self,
        session_id: Option<&str>,
        node_id: Option<&str>,
        run: Option<&GraphRun>,
        checkpoint: Option<&ExecutionCheckpoint>,
        workspace_mode: Option<&str>,
    ) -> RetrievedContextState {
        let snapshot = self.load_session_snapshot_at(session_id, node_id);
        let inspection_user_message = snapshot
            .history
            .iter()
            .rev()
            .find(|message| message.role == "user")
            .map(|message| message.content.as_str())
            .unwrap_or("");
        self.context_builder.retrieve_context_state(
            inspection_user_message,
            &[],
            workspace_mode,
            &snapshot,
            run,
            checkpoint,
        )
    }

    #[allow(dead_code)]
    pub fn build_graph_turn_handoff(
        &self,
        run: Option<&GraphRun>,
        turn_id: Option<&str>,
        session_id: Option<&str>,
        result: &TurnResult,
        checkpoint: Option<&ExecutionCheckpoint>,
        workspace_mode: Option<&str>,
    ) -> GraphTurnHandoff {
        let snapshot = self.load_session_snapshot(session_id);
        let retrieved = self.context_builder.retrieve_context_state(
            &result.user_message,
            &[],
            workspace_mode,
            &snapshot,
            run,
            checkpoint,
        );
        self.graph
            .build_turn_handoff(turn_id, session_id, result, &retrieved)
    }

    #[allow(dead_code)]
    pub fn decide_graph_after_turn(
        &mut self,
        turn_id: Option<&str>,
        session_id: Option<&str>,
        result: &TurnResult,
        checkpoint: Option<&ExecutionCheckpoint>,
        workspace_mode: Option<&str>,
    ) -> GraphDecision {
        let handoff = self.build_graph_turn_handoff(
            None,
            turn_id,
            session_id,
            result,
            checkpoint,
            workspace_mode,
        );
        self.graph.decide_after_turn(&handoff)
    }

    #[allow(dead_code)]
    pub fn decide_graph_after_turn_with_planner(
        &self,
        run: &GraphRun,
        turn_id: Option<&str>,
        session_id: Option<&str>,
        result: &TurnResult,
        checkpoint: Option<&ExecutionCheckpoint>,
        workspace_mode: Option<&str>,
        planner: &dyn GraphPlanner,
    ) -> Result<PlannerGraphDecisionDispatchOutcome, String> {
        let handoff = self.build_graph_turn_handoff(
            Some(run),
            turn_id,
            session_id,
            result,
            checkpoint,
            workspace_mode,
        );
        let decision = self
            .graph
            .decide_after_turn_with_planner(run, &handoff, planner);
        let mut dispatch = self.dispatch_graph_decision_hooks(
            &self.build_planner_graph_decision_envelope(run, &decision),
            decision,
        );
        if let Some(error) = dispatch.fail_turn_error.take() {
            return Err(error);
        }
        if let Some(error) = dispatch.blocked_error.take() {
            return Err(error);
        }
        Ok(PlannerGraphDecisionDispatchOutcome {
            decision: dispatch.decision,
            trace_records: dispatch.trace_records,
        })
    }

    pub fn remove_session(&mut self, session_id: &str) -> Vec<SessionOverview> {
        self.sessions
            .write()
            .unwrap_or_else(|e| {
                eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
                e.into_inner()
            })
            .remove_session(session_id)
    }

    pub fn load_history_graph(
        &mut self,
        session_id: Option<&str>,
    ) -> (Vec<HistoryNode>, Vec<HistoryBranch>, HistoryCursor) {
        self.sessions
            .write()
            .unwrap_or_else(|e| {
                eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
                e.into_inner()
            })
            .load_history_graph(session_id)
    }

    pub fn load_history_cursor(&mut self, session_id: Option<&str>) -> HistoryCursor {
        self.sessions
            .write()
            .unwrap_or_else(|e| {
                eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
                e.into_inner()
            })
            .load_history_cursor(session_id)
    }

    pub fn checkout_history_node(
        &mut self,
        session_id: Option<&str>,
        node_id: &str,
        mode: HistoryCheckoutMode,
        expected_cursor_version: Option<u64>,
    ) -> Result<SessionSnapshot, String> {
        self.sessions
            .write()
            .unwrap_or_else(|e| {
                eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
                e.into_inner()
            })
            .checkout_history_node(session_id, node_id, mode, expected_cursor_version)
    }

    pub fn restore_branch_head(
        &mut self,
        session_id: Option<&str>,
        branch_id: Option<&str>,
        expected_cursor_version: Option<u64>,
    ) -> Result<SessionSnapshot, String> {
        self.sessions
            .write()
            .unwrap_or_else(|e| {
                eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
                e.into_inner()
            })
            .restore_branch_head(session_id, branch_id, expected_cursor_version)
    }

    pub fn fork_from_history_node(
        &mut self,
        session_id: Option<&str>,
        node_id: &str,
        expected_cursor_version: Option<u64>,
    ) -> Result<SessionSnapshot, String> {
        self.sessions
            .write()
            .unwrap_or_else(|e| {
                eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
                e.into_inner()
            })
            .fork_from_history_node(session_id, node_id, expected_cursor_version)
    }

    pub fn switch_history_branch(
        &mut self,
        session_id: Option<&str>,
        branch_id: &str,
        expected_cursor_version: Option<u64>,
    ) -> Result<SessionSnapshot, String> {
        self.sessions
            .write()
            .unwrap_or_else(|e| {
                eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
                e.into_inner()
            })
            .switch_history_branch(session_id, branch_id, expected_cursor_version)
    }
}
