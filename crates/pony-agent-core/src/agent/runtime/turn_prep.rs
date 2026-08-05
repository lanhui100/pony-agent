// turn_prep: 轮次准备（prepare_turn / plan_turn）与 planner / capability / skill 各类 envelope 与 trace 记录构建。
// 由 runtime/mod.rs 的 AgentRuntime 巨型 impl 拆分而来，行为与结构保持一致。
use super::*;

impl AgentRuntime {
    pub(crate) fn prepare_turn(
        &self,
        input: &TurnInput,
        reject_empty: bool,
        ask_injection: Option<&GraphAskResumeInjection>,
    ) -> Result<PreparedTurn, String> {
        let user_message = if reject_empty {
            let trimmed = input.message.trim();
            if trimmed.is_empty() {
                return Err("Message is empty.".to_string());
            }
            trimmed.to_string()
        } else {
            normalize_user_message(&input.message)
        };

        let provider = self.resolve_provider(input);
        let workspace_mode = input.workspace_mode.as_deref();

        // 上下文压缩检查：在构建上下文前，检查是否需要压缩历史
        let session = {
            let mut sessions = self.sessions.write().unwrap_or_else(|e| {
                eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
                e.into_inner()
            });

            // 先获取当前快照，检查是否需要压缩
            let current_snapshot = sessions.snapshot_at(
                input.session_id.as_deref(),
                input.node_id.as_deref(),
                &input.history,
            );

            let config = crate::agent::compression::CompressionConfig::default();
            if crate::agent::compression::should_compress(
                &current_snapshot.history,
                &provider,
                &config,
            ) {
                let (to_compress, _to_keep) =
                    crate::agent::compression::split_history(&current_snapshot.history, &config);

                if !to_compress.is_empty() {
                    let system_prompt =
                        crate::agent::compression::build_compression_system_prompt();
                    let mut compression_messages = vec![ProviderMessage::system(system_prompt)];
                    for msg in &to_compress {
                        compression_messages.push(ProviderMessage::user(&msg.content));
                    }

                    let compression_request = ProviderRequest {
                        model: provider.model().to_string(),
                        input: compression_messages,
                        images: Vec::new(),
                        native_messages: Vec::new(),
                        observation: ProviderRequestObservation::default(),
                        temperature: 0.3,
                        max_output_tokens: 2048,
                    };

                    match provider.decide_with_tools(&compression_request, &[]) {
                        Ok(decision) => {
                            let summary_msg = crate::agent::compression::wrap_summary_to_message(
                                &decision.output_text,
                            );
                            let result = crate::agent::compression::apply_compression(
                                current_snapshot.history.clone(),
                                summary_msg,
                                &config,
                            );
                            sessions.replace_session_history(
                                input.session_id.as_deref(),
                                result.history,
                            );
                        }
                        Err(e) => {
                            eprintln!(
                                "[pony-agent] session compression failed: {e}, \
                                 continuing without compression"
                            );
                        }
                    }
                }
            }

            // 获取最终（可能已压缩）的快照
            sessions.snapshot_at(
                input.session_id.as_deref(),
                input.node_id.as_deref(),
                &input.history,
            )
        };

        let preliminary_retrieved = self.context_builder.retrieve_context_state(
            &user_message,
            &[],
            workspace_mode,
            &session,
            None,
            None,
        );
        let effective_images =
            self.resolve_turn_images(input, &preliminary_retrieved, &provider)?;
        let tools = builtin_tools();
        let planner_skills = self.capability_registry.list_skills_for_planner();
        let retrieved = if effective_images.is_empty() {
            preliminary_retrieved
        } else {
            self.context_builder.retrieve_context_state(
                &user_message,
                &effective_images,
                workspace_mode,
                &session,
                None,
                None,
            )
        };
        let planning_request = self.context_builder.build_request(
            self.graph.name(),
            &provider,
            &retrieved,
            &planner_skills,
        );
        let build_context_observation = build_context_observation(&planning_request, &tools);
        let display_message = input
            .display_message
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| user_message.clone());

        // Phase-4 P0: seed a consumed Ask resume injection into the provider context as a
        // completed assistant tool-call + terminal tool-result pair (design.md Decision 5). The
        // pair is inserted immediately before the current user message so the model sees the
        // answer and continues instead of a fresh user turn.
        let mut planning_request = planning_request;
        if let Some(injection) = ask_injection {
            inject_ask_resume_into_planning_request(&provider, injection, &mut planning_request);
        }

        Ok(PreparedTurn {
            user_message,
            display_message,
            retrieved,
            provider,
            tools,
            planner_skills,
            planning_request,
            build_context_observation,
        })
    }

    pub(crate) fn plan_turn(&self, prepared: &PreparedTurn) -> Result<PlannedTurn, String> {
        let preflight_decision = self.planner.preflight_decision(
            &prepared.user_message,
            prepared.retrieved.planner_history(),
            &prepared.planner_skills,
        );
        let mut preflight_dispatch = self.dispatch_planner_hooks(
            PlannerHookPoint::TurnPreflight,
            &self.build_planner_preflight_envelope(
                &prepared.user_message,
                prepared.retrieved.planner_history(),
                &prepared.planner_skills,
                preflight_decision.as_ref(),
            ),
            preflight_decision,
            None,
        );
        if let Some(error) = preflight_dispatch.fail_turn_error.take() {
            return Err(error);
        }
        if let Some(error) = preflight_dispatch.blocked_error.take() {
            return Err(error);
        }
        let preflight_decision = preflight_dispatch.decision;
        let mut planner_hook_trace_records = preflight_dispatch.trace_records;
        planner_hook_trace_records.push(self.build_planner_preflight_trace_record(
            &prepared.user_message,
            &prepared.planner_skills,
            preflight_decision.as_ref(),
        ));

        let (mut first_decision, initial_decision_duration_ms) =
            if prepared.provider.requires_provider_native_tool_flow() {
                // native tool flow（reasoning 模型，如 DeepSeek thinking 模式）下必须由
                // provider 生成决策：工具执行后的 follow-up 请求需要把 assistant 消息的
                // reasoning_content 完整回传给 API，本地 planner 决策没有该字段，
                // 会被 DeepSeek 等 thinking 模式以 invalid_request_error 拒绝。
                let started_at = Instant::now();
                let decision = provider_decision(
                    &prepared.provider,
                    &prepared.planning_request,
                    &prepared.tools,
                )?;
                (decision, Some(started_at.elapsed().as_millis() as u64))
            } else {
                match preflight_decision {
                    Some(decision) => (decision, None),
                    None => {
                        let started_at = Instant::now();
                        let decision = provider_decision(
                            &prepared.provider,
                            &prepared.planning_request,
                            &prepared.tools,
                        )?;
                        (decision, Some(started_at.elapsed().as_millis() as u64))
                    }
                }
            };

        if let Some(tool_call) = first_decision.tool_call.take() {
            let normalized = normalize_tool_directive(
                tool_call,
                first_decision.assistant_message.take(),
                &first_decision.output_text,
                first_decision.reasoning_content.as_deref(),
                first_decision.reasoning_content_value.as_ref(),
            )?;
            first_decision.tool_call = Some(normalized.tool_call);
            first_decision.assistant_message = normalized.assistant_message;
        }

        if let Some(error) = provider_failure_message(
            &first_decision.provider_mode,
            first_decision.fallback_reason.as_deref(),
        ) {
            return Err(error);
        }

        let resolved_tool_call = self.resolve_tool_call(
            &prepared.user_message,
            prepared.retrieved.planner_history(),
            &prepared.planner_skills,
            first_decision.tool_call.clone(),
            !prepared.provider.requires_provider_native_tool_flow(),
        );
        let mut tool_selection_dispatch = self.dispatch_planner_hooks(
            PlannerHookPoint::ToolSelection,
            &self.build_planner_tool_selection_envelope(
                &prepared.user_message,
                prepared.retrieved.planner_history(),
                &prepared.planner_skills,
                first_decision.tool_call.as_ref(),
                resolved_tool_call.as_ref(),
            ),
            None,
            resolved_tool_call,
        );
        if let Some(error) = tool_selection_dispatch.fail_turn_error.take() {
            return Err(error);
        }
        if let Some(error) = tool_selection_dispatch.blocked_error.take() {
            return Err(error);
        }
        let resolved_tool_call = tool_selection_dispatch.selected_tool_call;
        planner_hook_trace_records.extend(tool_selection_dispatch.trace_records);
        planner_hook_trace_records.push(self.build_planner_tool_selection_trace_record(
            &prepared.user_message,
            first_decision.tool_call.as_ref(),
            resolved_tool_call.as_ref(),
        ));

        Ok(PlannedTurn {
            first_decision,
            resolved_tool_call,
            initial_decision_duration_ms,
            planner_hook_trace_records,
        })
    }

    pub(crate) fn build_planner_preflight_trace_record(
        &self,
        user_message: &str,
        planner_skills: &[crate::agent::capability_bridge::SkillDescriptor],
        decision: Option<&ProviderDecision>,
    ) -> HookTraceRecord {
        let summary = match decision.and_then(|decision| decision.tool_call.as_ref()) {
            Some(tool_call) => format!(
                "planner preflight produced normalized provider decision with tool `{}`",
                tool_call.name
            ),
            None if decision.is_some() => {
                "planner preflight produced normalized provider decision without tool call"
                    .to_string()
            }
            None => "planner preflight deferred first decision to provider resolution".to_string(),
        };
        build_observe_hook_trace_record(
            "planner.preflight.observe",
            TurnHookPoint::PlannerTurnPreflight,
            1,
            summary,
            Some(format!(
                "message={} skills={}",
                preview_text(user_message, 64),
                planner_skills.len()
            )),
        )
    }

    pub(crate) fn build_planner_tool_selection_trace_record(
        &self,
        user_message: &str,
        provider_tool_call: Option<&ToolCall>,
        resolved_tool_call: Option<&ToolCall>,
    ) -> HookTraceRecord {
        let provider_tool = provider_tool_call
            .map(|tool_call| tool_call.name.as_str())
            .unwrap_or("none");
        let resolved_tool = resolved_tool_call
            .map(|tool_call| tool_call.name.as_str())
            .unwrap_or("none");
        let summary = if provider_tool == resolved_tool {
            format!(
                "planner tool selection kept normalized tool path `{}`",
                resolved_tool
            )
        } else {
            format!(
                "planner tool selection rewrote normalized tool path from `{}` to `{}`",
                provider_tool, resolved_tool
            )
        };
        build_observe_hook_trace_record(
            "planner.tool_selection.observe",
            TurnHookPoint::PlannerToolSelection,
            1,
            summary,
            Some(format!("message={}", preview_text(user_message, 64))),
        )
    }

    pub(crate) fn build_planner_preflight_envelope(
        &self,
        user_message: &str,
        history: &[TurnHistoryMessage],
        available_skills: &[crate::agent::capability_bridge::SkillDescriptor],
        decision: Option<&ProviderDecision>,
    ) -> PlannerFactsEnvelope {
        PlannerFactsEnvelope {
            session_id: None,
            run_id: None,
            hook_point: PlannerHookPoint::TurnPreflight,
            source_boundary: "runtime.plan_turn.preflight".to_string(),
            planner_source: "turn_planner.preflight_decision".to_string(),
            user_message_summary: Some(preview_text(user_message, 64)),
            history_turn_count: history.len(),
            available_skill_ids: available_skills
                .iter()
                .map(|skill| skill.skill_id.clone())
                .collect(),
            provider_decision_summary: decision.map(summarize_provider_decision),
            provider_tool_call_name: decision
                .and_then(|decision| decision.tool_call.as_ref())
                .map(|tool_call| tool_call.name.clone()),
            graph_goal_summary: None,
            graph_step_count: None,
            current_decision_summary: decision.map(summarize_provider_decision),
        }
    }

    pub(crate) fn build_planner_tool_selection_envelope(
        &self,
        user_message: &str,
        history: &[TurnHistoryMessage],
        available_skills: &[crate::agent::capability_bridge::SkillDescriptor],
        provider_tool_call: Option<&ToolCall>,
        selected_tool_call: Option<&ToolCall>,
    ) -> PlannerFactsEnvelope {
        PlannerFactsEnvelope {
            session_id: None,
            run_id: None,
            hook_point: PlannerHookPoint::ToolSelection,
            source_boundary: "runtime.plan_turn.tool_selection".to_string(),
            planner_source: "turn_planner.select_tool_call".to_string(),
            user_message_summary: Some(preview_text(user_message, 64)),
            history_turn_count: history.len(),
            available_skill_ids: available_skills
                .iter()
                .map(|skill| skill.skill_id.clone())
                .collect(),
            provider_decision_summary: provider_tool_call
                .map(|tool_call| format!("provider suggested `{}`", tool_call.name)),
            provider_tool_call_name: provider_tool_call.map(|tool_call| tool_call.name.clone()),
            graph_goal_summary: None,
            graph_step_count: None,
            current_decision_summary: selected_tool_call
                .map(|tool_call| format!("selected `{}`", tool_call.name)),
        }
    }

    pub(crate) fn build_planner_graph_decision_trace_record(
        &self,
        run: &GraphRun,
        decision: &GraphDecision,
    ) -> HookTraceRecord {
        build_observe_hook_trace_record(
            "planner.graph_decision.observe",
            TurnHookPoint::PlannerGraphDecision,
            1,
            format!(
                "planner graph decision produced `{}` for run `{}`",
                graph_decision_kind_label(&decision.kind),
                run.id
            ),
            Some(format!(
                "run_phase={:?} target_phase={:?} reason={:?}",
                run.phase, decision.target_phase, decision.reason
            )),
        )
    }

    pub(crate) fn build_planner_graph_decision_envelope(
        &self,
        run: &GraphRun,
        decision: &GraphDecision,
    ) -> PlannerFactsEnvelope {
        PlannerFactsEnvelope {
            session_id: run.session_id.clone(),
            run_id: Some(run.id.clone()),
            hook_point: PlannerHookPoint::GraphDecision,
            source_boundary: "runtime.decide_graph_after_turn_with_planner".to_string(),
            planner_source: "graph_planner.decide_after_turn".to_string(),
            user_message_summary: None,
            history_turn_count: run.steps.len(),
            available_skill_ids: Vec::new(),
            provider_decision_summary: None,
            provider_tool_call_name: None,
            graph_goal_summary: Some(preview_text(&run.goal, 96)),
            graph_step_count: Some(run.steps.len()),
            current_decision_summary: Some(decision.summary.clone()),
        }
    }

    pub fn record_planner_graph_decision_trace(
        &self,
        session_id: Option<&str>,
        turn_id: &str,
        run: &GraphRun,
        decision: &GraphDecision,
    ) -> Option<HookTraceRecord> {
        let record = self.build_planner_graph_decision_trace_record(run, decision);
        if self.append_turn_trace_hook_records(session_id, turn_id, vec![record.clone()]) {
            Some(record)
        } else {
            None
        }
    }

    pub(crate) fn build_capability_resolution_trace_record(
        &self,
        tool_call: &ToolCall,
        execution: &CapabilityToolExecutionResult,
    ) -> HookTraceRecord {
        let summary = match (
            execution.capability.as_ref(),
            execution.failure_kind.as_ref(),
        ) {
            (Some(capability), None) => format!(
                "capability mediation resolved `{}` to `{}`",
                tool_call.name, capability.capability_id
            ),
            (Some(capability), Some(failure)) => format!(
                "capability mediation resolved `{}` to `{}` but execution failed with `{}`",
                tool_call.name,
                capability.capability_id,
                failure.as_str()
            ),
            (None, Some(failure)) => format!(
                "capability mediation failed to resolve `{}`: {}",
                tool_call.name,
                failure.as_str()
            ),
            (None, None) => {
                format!(
                    "capability mediation observed `{}` without resolution details",
                    tool_call.name
                )
            }
        };
        build_observe_hook_trace_record(
            "capability.resolve.observe",
            TurnHookPoint::CapabilityResolve,
            1,
            summary,
            Some(format!("tool={}", tool_call.name)),
        )
    }

    pub(crate) fn build_skill_resolution_trace_record(
        &self,
        request: &SkillInvocationRequest,
        execution: &SkillToolExecutionResult,
    ) -> HookTraceRecord {
        let summary = match (execution.skill.as_ref(), execution.failure_layer.as_ref()) {
            (Some(skill), None) => format!(
                "skill mediation resolved `{}` with {} composed capability actions",
                skill.skill_id,
                execution.capability_executions.len()
            ),
            (Some(skill), Some(layer)) => format!(
                "skill mediation resolved `{}` but failed at `{}`",
                skill.skill_id,
                layer.as_str()
            ),
            (None, Some(layer)) => format!(
                "skill mediation failed to resolve `{}` at `{}`",
                request.skill_id,
                layer.as_str()
            ),
            (None, None) => {
                format!(
                    "skill mediation observed `{}` without resolution details",
                    request.skill_id
                )
            }
        };
        build_observe_hook_trace_record(
            "skill.tool_actions.observe",
            TurnHookPoint::SkillToolActionsResolve,
            1,
            summary,
            Some(format!("skill_id={}", request.skill_id)),
        )
    }

    pub(crate) fn build_capability_mediation_envelope(
        &self,
        tool_call: &ToolCall,
    ) -> CapabilityMediationEnvelope {
        let candidate_ids =
            candidate_capability_ids_for_tool_name(&self.capability_registry, &tool_call.name);
        let resolved_capability = candidate_ids
            .first()
            .and_then(|capability_id| self.capability_registry.inspect_capability(capability_id));
        CapabilityMediationEnvelope {
            session_id: None,
            run_id: None,
            hook_point: CapabilityMediationHookPoint::CapabilityResolve,
            source_boundary: "runtime.execute_registered_tool_call".to_string(),
            mediation_source: "capability_registry.resolve_tool_call".to_string(),
            requested_capability_id: resolved_capability
                .as_ref()
                .map(|capability| capability.capability_id.clone()),
            requested_skill_id: None,
            capability_kind: resolved_capability
                .as_ref()
                .map(|capability| capability.kind.as_str().to_string()),
            candidate_ids,
            argument_summary: tool_call.arguments.to_string(),
            source_id: resolved_capability
                .as_ref()
                .map(|capability| capability.source_id.clone()),
            source_kind: resolved_capability
                .as_ref()
                .map(|capability| capability.source_kind.as_str().to_string()),
        }
    }

    pub(crate) fn build_skill_mediation_envelope(
        &self,
        tool_call: &ToolCall,
        skill: &crate::agent::capability_bridge::SkillDescriptor,
    ) -> CapabilityMediationEnvelope {
        let source = self
            .capability_registry
            .inspect_skill_source(&skill.source_id);
        CapabilityMediationEnvelope {
            session_id: None,
            run_id: None,
            hook_point: CapabilityMediationHookPoint::SkillToolActionsResolve,
            source_boundary: "runtime.execute_registered_tool_call".to_string(),
            mediation_source: "capability_registry.resolve_skill_tool_actions".to_string(),
            requested_capability_id: None,
            requested_skill_id: Some(skill.skill_id.clone()),
            capability_kind: None,
            candidate_ids: skill.composed_capability_refs.clone(),
            argument_summary: tool_call.arguments.to_string(),
            source_id: Some(skill.source_id.clone()),
            source_kind: source.map(|source| source.source_kind.as_str().to_string()),
        }
    }

    pub(crate) fn build_mcp_source_ingress_envelope(
        &self,
        snapshot: &McpSourceSnapshot,
    ) -> CapabilityMediationEnvelope {
        CapabilityMediationEnvelope {
            session_id: None,
            run_id: None,
            hook_point: CapabilityMediationHookPoint::McpSourceIngress,
            source_boundary: "control_plane.apply_mcp_source_snapshot".to_string(),
            mediation_source: "capability_registry.replace_mcp_source_snapshot".to_string(),
            requested_capability_id: None,
            requested_skill_id: None,
            capability_kind: None,
            candidate_ids: snapshot
                .capabilities
                .iter()
                .map(|capability| capability.capability_id.clone())
                .collect(),
            argument_summary: "{}".to_string(),
            source_id: Some(snapshot.source.source_id.clone()),
            source_kind: Some(snapshot.source.source_kind.as_str().to_string()),
        }
    }

    pub(crate) fn build_skill_source_ingress_envelope(
        &self,
        snapshot: &SkillSourceSnapshot,
    ) -> CapabilityMediationEnvelope {
        CapabilityMediationEnvelope {
            session_id: None,
            run_id: None,
            hook_point: CapabilityMediationHookPoint::SkillSourceIngress,
            source_boundary: "control_plane.apply_skill_source_snapshot".to_string(),
            mediation_source: "capability_registry.replace_skill_source_snapshot".to_string(),
            requested_capability_id: None,
            requested_skill_id: None,
            capability_kind: None,
            candidate_ids: snapshot
                .skills
                .iter()
                .map(|skill| skill.skill_id.clone())
                .collect(),
            argument_summary: "{}".to_string(),
            source_id: Some(snapshot.source.source_id.clone()),
            source_kind: Some(snapshot.source.source_kind.as_str().to_string()),
        }
    }

    pub(crate) fn resolve_turn_images(
        &self,
        input: &TurnInput,
        retrieved: &RetrievedContextState,
        provider: &ProviderManager,
    ) -> Result<Vec<TurnInputImage>, String> {
        let mut images = input.images.clone();

        if images.is_empty()
            && provider.supports_image_input()
            && should_recall_recent_images(retrieved)
        {
            let recall_limit = recalled_image_limit(&retrieved.turn_context.user_message);
            images = self
                .sessions
                .read()
                .unwrap_or_else(|e| {
                    eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
                    e.into_inner()
                })
                .load_recent_images(input.session_id.as_deref(), recall_limit);
        }

        validate_turn_images(&images)?;
        Ok(images)
    }
}

/// Phase-4 P0 (design.md Decision 5): seed a consumed Ask resume injection into the provider
/// request as a completed assistant tool-call + terminal tool-result pair, inserted immediately
/// before the current user message. The model sees the host's answer keyed to the original
/// tool-call id and continues, instead of a fresh user turn.
fn inject_ask_resume_into_planning_request(
    provider: &ProviderManager,
    injection: &GraphAskResumeInjection,
    request: &mut ProviderRequest,
) {
    let tool_call = ToolCall {
        call_id: Some(injection.call_id.clone()),
        name: injection.tool_name.clone(),
        arguments: injection
            .assistant_transcript
            .get("assistantMessage")
            .cloned()
            .unwrap_or_else(|| json!({})),
        plan: None,
    };
    let tool_result = ToolResult {
        tool_name: injection.tool_name.clone(),
        status: "ok".to_string(),
        output: injection.terminal_result.to_string(),
        duration_ms: 0,
    };
    let protocol_label = provider.protocol_label();
    let assistant_message =
        provider_native_assistant_tool_call_message_for_protocol(protocol_label, None, None, &tool_call);
    let tool_result_message =
        provider_native_tool_result_message_for_protocol(protocol_label, &tool_call, &tool_result);

    let insert_at = request
        .native_messages
        .iter()
        .rposition(is_native_user_message)
        .unwrap_or(request.native_messages.len());
    request
        .native_messages
        .insert(insert_at, assistant_message);
    request
        .native_messages
        .insert(insert_at + 1, tool_result_message);
}

fn is_native_user_message(message: &Value) -> bool {
    message.get("role").and_then(Value::as_str) == Some("user")
}
