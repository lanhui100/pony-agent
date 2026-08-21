// query_commands: 会话 / 模型监控 / graph run 的只读查询命令。
// 由 control_plane/mod.rs 的 HostControlPlane 巨型 impl 拆分而来，行为与结构保持一致。
use super::*;

impl HostControlPlane {
    pub fn load_execution_checkpoint(
        &self,
        query: ExecutionCheckpointQuery,
    ) -> Option<ExecutionCheckpoint> {
        let runtime_checkpoint = self
            .execution_control
            .load_checkpoint(query.turn_id.as_deref(), query.session_id.as_deref())
            .filter(|checkpoint| {
                query.turn_id.is_some()
                    || checkpoint.checkpoint_kind != "runtime_control"
                    || checkpoint.status == "running"
            });

        let mut checkpoint = runtime_checkpoint
            .or_else(|| {
                let run =
                    self.resolve_graph_run_for_retrieval(None, query.session_id.as_deref())?;
                let checkpoint = self.graph_runner.build_checkpoint(&run);
                Self::should_project_graph_checkpoint_as_recovery(&checkpoint)
                    .then(|| Self::execution_checkpoint_from_graph_run_checkpoint(checkpoint))
            })
            .or_else(|| self.load_trace_boundary_checkpoint(query.clone()))?;
        self.attach_session_persisted_effect_evidence(&mut checkpoint, query.session_id.as_deref());
        Some(checkpoint)
    }

    pub fn list_sessions(&self) -> Vec<SessionOverview> {
        self.sessions_rwlock
            .read()
            .unwrap_or_else(|e| {
                eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
                e.into_inner()
            })
            .list_sessions()
    }

    pub fn load_session_traces(&self, session_id: &str) -> Vec<TurnTraceRecord> {
        self.sessions_rwlock
            .read()
            .unwrap_or_else(|e| {
                eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
                e.into_inner()
            })
            .load_turn_traces(session_id)
    }

    /// PA-094：按引用加载 build_context_observation 全量 payload（大字段外置）。
    /// 引用格式 `bco:<turn_id>:<seq>`；未命中返回 None（legacy 内嵌数据走 trace 字段）。
    pub fn load_build_context_observation(
        &self,
        session_id: &str,
        observation_ref: &str,
    ) -> Option<crate::agent::provider::BuildContextObservation> {
        self.sessions_rwlock
            .read()
            .unwrap_or_else(|e| {
                eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
                e.into_inner()
            })
            .load_build_context_observation(session_id, observation_ref)
    }

    pub fn load_model_monitor_summary(
        &self,
        query: ModelMonitorSummaryQuery,
    ) -> ModelMonitorSummaryView {
        let target_session_id = query
            .session_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let session_overviews = self
            .sessions_rwlock
            .read()
            .unwrap_or_else(|e| {
                eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
                e.into_inner()
            })
            .list_sessions();
        let selected_overviews = session_overviews
            .into_iter()
            .filter(|overview| {
                target_session_id
                    .as_deref()
                    .map(|session_id| overview.conversation_id == session_id)
                    .unwrap_or(true)
            })
            .collect::<Vec<_>>();

        let mut overview = ModelMonitorOverview {
            session_count: selected_overviews.len() as u64,
            ..ModelMonitorOverview::default()
        };
        let mut overview_first_token_latency = AverageAccumulator::default();
        let mut overview_turn_duration = AverageAccumulator::default();
        let mut overview_hook_duration = AverageAccumulator::default();
        let mut providers = std::collections::BTreeMap::<String, MonitorAggregate>::new();
        let mut models = std::collections::BTreeMap::<String, MonitorAggregate>::new();
        let mut tools = std::collections::BTreeMap::<String, ToolAggregate>::new();
        let mut hook_classes = std::collections::BTreeMap::<String, HookAggregate>::new();
        let mut hooks = std::collections::BTreeMap::<String, HookAggregate>::new();
        let mut capability_sources = std::collections::BTreeMap::<String, ActivityAggregate>::new();
        let mut capability_invocation_modes =
            std::collections::BTreeMap::<String, ActivityAggregate>::new();
        let mut capability_failure_classes =
            std::collections::BTreeMap::<String, ActivityAggregate>::new();
        let mut skill_selections = std::collections::BTreeMap::<String, ActivityAggregate>::new();
        let mut skill_sources = std::collections::BTreeMap::<String, ActivityAggregate>::new();
        let mut skill_failure_layers =
            std::collections::BTreeMap::<String, ActivityAggregate>::new();
        let mut sessions = Vec::with_capacity(selected_overviews.len());

        for session_overview in selected_overviews {
            let snapshot = self
                .sessions_rwlock
                .read()
                .unwrap_or_else(|e| {
                    eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
                    e.into_inner()
                })
                .snapshot_at_readonly(Some(session_overview.conversation_id.as_str()), None, &[]);
            let session_metrics = aggregate_session_metrics(&snapshot);
            merge_monitor_overview(
                &mut overview,
                &session_metrics,
                &mut overview_first_token_latency,
                &mut overview_turn_duration,
                &mut overview_hook_duration,
            );

            for trace in &snapshot.turn_trace_history {
                if !trace_has_canonical_terminal_envelope(trace) {
                    continue;
                }
                let (provider_key, provider_label) = provider_dimension_key_and_label(trace);
                providers
                    .entry(provider_key.clone())
                    .or_insert_with(|| {
                        MonitorAggregate::new(provider_key.clone(), provider_label.clone())
                    })
                    .add_trace(trace);

                let (model_key, model_label) = model_dimension_key_and_label(trace);
                models
                    .entry(model_key.clone())
                    .or_insert_with(|| MonitorAggregate::new(model_key.clone(), model_label))
                    .add_trace(trace);

                append_tool_aggregates(trace, &mut tools);
                append_hook_class_aggregates(trace, &mut hook_classes);
                append_hook_aggregates(trace, &mut hooks);
                append_capability_aggregates(
                    trace,
                    &mut capability_sources,
                    &mut capability_invocation_modes,
                    &mut capability_failure_classes,
                );
                append_skill_aggregates(
                    trace,
                    &mut skill_selections,
                    &mut skill_sources,
                    &mut skill_failure_layers,
                );
            }

            sessions.push(ModelMonitorSessionRow {
                session_id: snapshot.conversation_id.clone(),
                title: snapshot.title.clone(),
                summary: snapshot.summary.clone(),
                updated_at_ms: snapshot.updated_at_ms,
                request_count: session_metrics.request_count,
                model_call_count: session_metrics.model_call_count,
                tool_call_count: session_metrics.tool_call_count,
                hook_call_count: session_metrics.hook_call_count,
                blocked_hook_count: session_metrics.blocked_hook_count,
                failed_request_count: session_metrics.failed_request_count,
                retrieval_participation_count: session_metrics.retrieval_participation_count,
                input_tokens: session_metrics.input_tokens,
                cache_hit_input_tokens: session_metrics.cache_hit_input_tokens,
                output_tokens: session_metrics.output_tokens,
                total_tokens: session_metrics.total_tokens,
                avg_first_token_latency_ms: session_metrics.first_token_latency.average(),
                avg_turn_duration_ms: session_metrics.turn_duration.average(),
                avg_hook_duration_ms: session_metrics.hook_duration.average(),
                total_hook_duration_ms: session_metrics.total_hook_duration_ms,
            });
        }

        sessions.sort_by(|left, right| {
            right
                .updated_at_ms
                .cmp(&left.updated_at_ms)
                .then_with(|| left.session_id.cmp(&right.session_id))
        });

        overview.avg_first_token_latency_ms = overview_first_token_latency.average();
        overview.avg_turn_duration_ms = overview_turn_duration.average();
        overview.avg_hook_duration_ms = overview_hook_duration.average();

        let mut providers = providers
            .into_values()
            .map(|aggregate| aggregate.into_row())
            .collect::<Vec<_>>();
        providers.sort_by(|left, right| {
            right
                .request_count
                .cmp(&left.request_count)
                .then_with(|| left.label.cmp(&right.label))
        });

        let mut models = models
            .into_values()
            .map(|aggregate| aggregate.into_row())
            .collect::<Vec<_>>();
        models.sort_by(|left, right| {
            right
                .request_count
                .cmp(&left.request_count)
                .then_with(|| left.label.cmp(&right.label))
        });

        let mut tools = tools
            .into_values()
            .map(|aggregate| aggregate.into_row())
            .collect::<Vec<_>>();
        tools.sort_by(|left, right| {
            right
                .call_count
                .cmp(&left.call_count)
                .then_with(|| left.label.cmp(&right.label))
        });

        let mut hook_classes = hook_classes
            .into_values()
            .map(|aggregate| aggregate.into_row())
            .collect::<Vec<_>>();
        hook_classes.sort_by(hook_row_sort);

        let mut hooks = hooks
            .into_values()
            .map(|aggregate| aggregate.into_row())
            .collect::<Vec<_>>();
        hooks.sort_by(hook_row_sort);

        let mut capability_sources = capability_sources
            .into_values()
            .map(|aggregate| aggregate.into_row())
            .collect::<Vec<_>>();
        capability_sources.sort_by(activity_row_sort);

        let mut capability_invocation_modes = capability_invocation_modes
            .into_values()
            .map(|aggregate| aggregate.into_row())
            .collect::<Vec<_>>();
        capability_invocation_modes.sort_by(activity_row_sort);

        let mut capability_failure_classes = capability_failure_classes
            .into_values()
            .map(|aggregate| aggregate.into_row())
            .collect::<Vec<_>>();
        capability_failure_classes.sort_by(activity_row_sort);

        let mut skill_selections = skill_selections
            .into_values()
            .map(|aggregate| aggregate.into_row())
            .collect::<Vec<_>>();
        skill_selections.sort_by(activity_row_sort);

        let mut skill_sources = skill_sources
            .into_values()
            .map(|aggregate| aggregate.into_row())
            .collect::<Vec<_>>();
        skill_sources.sort_by(activity_row_sort);

        let mut skill_failure_layers = skill_failure_layers
            .into_values()
            .map(|aggregate| aggregate.into_row())
            .collect::<Vec<_>>();
        skill_failure_layers.sort_by(activity_row_sort);

        ModelMonitorSummaryView {
            overview,
            providers,
            models,
            tools,
            hook_classes,
            hooks,
            capability_sources,
            capability_invocation_modes,
            capability_failure_classes,
            skill_selections,
            skill_sources,
            skill_failure_layers,
            sessions,
            generated_at_ms: now_timestamp_ms(),
        }
    }

    pub fn load_model_monitor_session_drilldown(
        &self,
        query: ModelMonitorSessionDrilldownQuery,
    ) -> ModelMonitorSessionDrilldownView {
        let session_id = self.normalize_history_session_id(Some(query.session_id));
        let runtime_view = self.load_session_runtime_view(SessionRuntimeViewQuery {
            session_id: Some(session_id.clone()),
            ..SessionRuntimeViewQuery::default()
        });
        let metrics = aggregate_session_metrics(&runtime_view.session);
        ModelMonitorSessionDrilldownView {
            session_id: session_id.clone(),
            metrics: ModelMonitorSessionRow {
                session_id,
                title: runtime_view.session.title.clone(),
                summary: runtime_view.session.summary.clone(),
                updated_at_ms: runtime_view.session.updated_at_ms,
                request_count: metrics.request_count,
                model_call_count: metrics.model_call_count,
                tool_call_count: metrics.tool_call_count,
                hook_call_count: metrics.hook_call_count,
                blocked_hook_count: metrics.blocked_hook_count,
                failed_request_count: metrics.failed_request_count,
                retrieval_participation_count: metrics.retrieval_participation_count,
                input_tokens: metrics.input_tokens,
                cache_hit_input_tokens: metrics.cache_hit_input_tokens,
                output_tokens: metrics.output_tokens,
                total_tokens: metrics.total_tokens,
                avg_first_token_latency_ms: metrics.first_token_latency.average(),
                avg_turn_duration_ms: metrics.turn_duration.average(),
                avg_hook_duration_ms: metrics.hook_duration.average(),
                total_hook_duration_ms: metrics.total_hook_duration_ms,
            },
            runtime_view,
        }
    }

    pub fn load_graph_run(&self, query: GraphRunQuery) -> Option<GraphRun> {
        let graph_runs = self.graph_runs.lock().unwrap_or_else(|e| {
            eprintln!("[pony-agent] graph run lock poisoned: {e}, recovering");
            e.into_inner()
        });
        let run_id = query.run_id?;
        graph_runs.load_run(&run_id)
    }

    pub fn list_graph_runs(&self) -> Vec<GraphRun> {
        let graph_runs = self.graph_runs.lock().unwrap_or_else(|e| {
            eprintln!("[pony-agent] graph run lock poisoned: {e}, recovering");
            e.into_inner()
        });
        graph_runs.list_runs()
    }
}
