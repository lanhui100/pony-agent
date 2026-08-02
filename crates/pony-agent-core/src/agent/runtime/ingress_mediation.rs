// ingress_mediation: capability/skill 源接入、hook 分发、capability mediation、inspection 与注册面。
// 由 runtime/mod.rs 的 AgentRuntime 巨型 impl 拆分而来，行为与结构保持一致。
use super::*;

impl AgentRuntime {
    pub fn apply_mcp_source_snapshot(&mut self, snapshot: McpSourceSnapshot) {
        let snapshot = enrich_mcp_source_snapshot(snapshot);
        self.sessions
            .write()
            .unwrap_or_else(|e| {
                eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
                e.into_inner()
            })
            .persist_mcp_source_snapshot(snapshot.clone());
        self.capability_registry
            .replace_mcp_source_snapshot(snapshot);
    }

    pub fn dispatch_mcp_source_ingress_hooks(
        &self,
        snapshot: &McpSourceSnapshot,
    ) -> Result<Vec<HookTraceRecord>, String> {
        let mut dispatch = self.dispatch_capability_mediation_hooks(
            CapabilityMediationHookPoint::McpSourceIngress,
            &self.build_mcp_source_ingress_envelope(snapshot),
        );
        if let Some(error) = dispatch.fail_turn_error.take() {
            return Err(error);
        }
        if let Some(error) = dispatch.blocked_error.take() {
            return Err(error);
        }
        Ok(dispatch.trace_records)
    }

    pub fn apply_skill_source_snapshot(
        &mut self,
        snapshot: SkillSourceSnapshot,
    ) -> Result<(), String> {
        let snapshot = enrich_skill_source_snapshot(snapshot);
        self.sessions
            .write()
            .unwrap_or_else(|e| {
                eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
                e.into_inner()
            })
            .persist_skill_source_snapshot(snapshot.clone());
        self.capability_registry
            .replace_skill_source_snapshot(snapshot)
    }

    pub fn dispatch_skill_source_ingress_hooks(
        &self,
        snapshot: &SkillSourceSnapshot,
    ) -> Result<Vec<HookTraceRecord>, String> {
        let mut dispatch = self.dispatch_capability_mediation_hooks(
            CapabilityMediationHookPoint::SkillSourceIngress,
            &self.build_skill_source_ingress_envelope(snapshot),
        );
        if let Some(error) = dispatch.fail_turn_error.take() {
            return Err(error);
        }
        if let Some(error) = dispatch.blocked_error.take() {
            return Err(error);
        }
        Ok(dispatch.trace_records)
    }

    pub fn capability_registry_snapshot(&self) -> CapabilityRegistry {
        self.capability_registry.clone()
    }

    pub fn register_hook_descriptor(
        &mut self,
        descriptor: AgentHookDescriptor,
    ) -> Result<(), String> {
        self.hook_registry.register(descriptor)
    }

    #[cfg(test)]
    pub fn set_hook_executor_for_test(&mut self, hook_executor: Box<dyn AgentHookExecutor>) {
        self.hook_executor = hook_executor;
    }

    #[cfg(test)]
    pub fn set_history_state_hook_executor_for_test(
        &mut self,
        hook_executor: Box<dyn crate::agent::hooks::HistoryStateHookExecutor>,
    ) {
        self.sessions
            .write()
            .unwrap_or_else(|e| {
                eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
                e.into_inner()
            })
            .set_history_state_hook_executor_for_test(hook_executor);
    }

    #[cfg(test)]
    pub fn record_turn_trace_for_test(&mut self, session_id: Option<&str>, trace: TurnTraceRecord) {
        self.sessions
            .write()
            .unwrap_or_else(|e| {
                eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
                e.into_inner()
            })
            .record_turn_trace(session_id, trace);
    }

    pub(crate) fn dispatch_hook_trace_records(
        &self,
        hook_point: TurnHookPoint,
    ) -> HookDispatchOutcome {
        let descriptors = self.hook_registry.list_for_hook_point(&hook_point);
        let mut records = Vec::with_capacity(descriptors.len());
        let mut fail_turn_error = None;

        for (index, descriptor) in descriptors.into_iter().enumerate() {
            match self.hook_executor.execute(descriptor, hook_point.clone()) {
                Ok(mut result) => {
                    result.hook_order = (index + 1) as u32;
                    records.push(result.to_trace_record());
                }
                Err(error) => {
                    let (result_kind, structured_result) =
                        crate::agent::hooks::normalized_result_for_class(&descriptor.class);
                    records.push(HookTraceRecord {
                        hook_name: descriptor.name.clone(),
                        hook_class: descriptor.class.clone(),
                        hook_point: hook_point.clone(),
                        hook_order: (index + 1) as u32,
                        result_kind,
                        structured_result,
                        blocked: matches!(
                            descriptor.default_failure_policy,
                            HookFailurePolicy::FailTurn
                        ),
                        elapsed_ms: 0,
                        input_summary: Some(format!("hook executor failed: {error}")),
                        persistence_evidence_ref: None,
                        summary: format!(
                            "hook execution failed under {:?}: {error}",
                            descriptor.default_failure_policy
                        ),
                    });
                    if matches!(
                        descriptor.default_failure_policy,
                        HookFailurePolicy::FailTurn
                    ) {
                        fail_turn_error = Some(format!(
                            "hook `{}` forced turn failure at `{:?}`: {error}",
                            descriptor.name, hook_point
                        ));
                        break;
                    }
                }
            }
        }

        HookDispatchOutcome {
            trace_records: records,
            fail_turn_error,
        }
    }

    pub(crate) fn dispatch_capability_mediation_hooks(
        &self,
        hook_point: CapabilityMediationHookPoint,
        envelope: &CapabilityMediationEnvelope,
    ) -> CapabilityMediationDispatchOutcome {
        let turn_hook_point = turn_hook_point_for_capability_mediation_hook_point(&hook_point);
        let descriptors = self.hook_registry.list_for_hook_point(&turn_hook_point);
        let mut records = Vec::with_capacity(descriptors.len());
        let mut execution_results = Vec::new();
        let mut fail_turn_error = None;
        let mut blocked_error = None;

        for (index, descriptor) in descriptors.into_iter().enumerate() {
            match self.hook_executor.execute_capability_mediation(
                descriptor,
                hook_point.clone(),
                envelope,
            ) {
                Ok(mut result) => {
                    result.hook_order = (index + 1) as u32;
                    if let HookStructuredResult::Deny(deny) = &result.structured_result {
                        blocked_error = Some(format!(
                            "hook `{}` blocked capability mediation: {}",
                            descriptor.name, deny.message
                        ));
                    }
                    records.push(result.to_trace_record());
                    execution_results.push(result);
                    if blocked_error.is_some() {
                        break;
                    }
                }
                Err(error) => {
                    let (result_kind, structured_result) =
                        crate::agent::hooks::normalized_result_for_class(&descriptor.class);
                    records.push(HookTraceRecord {
                        hook_name: descriptor.name.clone(),
                        hook_class: descriptor.class.clone(),
                        hook_point: turn_hook_point.clone(),
                        hook_order: (index + 1) as u32,
                        result_kind,
                        structured_result,
                        blocked: matches!(
                            descriptor.default_failure_policy,
                            HookFailurePolicy::FailTurn
                        ),
                        elapsed_ms: 0,
                        input_summary: Some(format!("hook executor failed: {error}")),
                        persistence_evidence_ref: None,
                        summary: format!(
                            "hook execution failed under {:?}: {error}",
                            descriptor.default_failure_policy
                        ),
                    });
                    if matches!(
                        descriptor.default_failure_policy,
                        HookFailurePolicy::FailTurn
                    ) {
                        fail_turn_error = Some(format!(
                            "hook `{}` forced turn failure at `{:?}`: {error}",
                            descriptor.name, turn_hook_point
                        ));
                        break;
                    }
                }
            }
        }

        let arguments = if fail_turn_error.is_none() && blocked_error.is_none() {
            apply_capability_argument_patches(
                &hook_point,
                &envelope.argument_summary,
                &execution_results,
            )
            .unwrap_or_else(|error| {
                fail_turn_error = Some(error);
                normalized_arguments_from_summary(&envelope.argument_summary)
            })
        } else {
            normalized_arguments_from_summary(&envelope.argument_summary)
        };

        CapabilityMediationDispatchOutcome {
            arguments,
            trace_records: records,
            blocked_error,
            fail_turn_error,
        }
    }

    pub(crate) fn dispatch_planner_hooks(
        &self,
        hook_point: PlannerHookPoint,
        envelope: &PlannerFactsEnvelope,
        decision: Option<ProviderDecision>,
        selected_tool_call: Option<ToolCall>,
    ) -> PlannerDispatchOutcome {
        let turn_hook_point = turn_hook_point_for_planner_hook_point(&hook_point);
        let descriptors = self.hook_registry.list_for_hook_point(&turn_hook_point);
        let mut records = Vec::with_capacity(descriptors.len());
        let mut execution_results = Vec::new();
        let mut fail_turn_error = None;
        let mut blocked_error = None;

        for (index, descriptor) in descriptors.into_iter().enumerate() {
            match self
                .hook_executor
                .execute_planner(descriptor, hook_point.clone(), envelope)
            {
                Ok(mut result) => {
                    result.hook_order = (index + 1) as u32;
                    if let HookStructuredResult::Deny(deny) = &result.structured_result {
                        blocked_error = Some(format!(
                            "hook `{}` blocked planner mediation: {}",
                            descriptor.name, deny.message
                        ));
                    }
                    records.push(result.to_trace_record());
                    execution_results.push(result);
                    if blocked_error.is_some() {
                        break;
                    }
                }
                Err(error) => {
                    let (result_kind, structured_result) =
                        crate::agent::hooks::normalized_result_for_class(&descriptor.class);
                    records.push(HookTraceRecord {
                        hook_name: descriptor.name.clone(),
                        hook_class: descriptor.class.clone(),
                        hook_point: turn_hook_point.clone(),
                        hook_order: (index + 1) as u32,
                        result_kind,
                        structured_result,
                        blocked: matches!(
                            descriptor.default_failure_policy,
                            HookFailurePolicy::FailTurn
                        ),
                        elapsed_ms: 0,
                        input_summary: Some(format!("hook executor failed: {error}")),
                        persistence_evidence_ref: None,
                        summary: format!(
                            "hook execution failed under {:?}: {error}",
                            descriptor.default_failure_policy
                        ),
                    });
                    if matches!(
                        descriptor.default_failure_policy,
                        HookFailurePolicy::FailTurn
                    ) {
                        fail_turn_error = Some(format!(
                            "hook `{}` forced turn failure at `{:?}`: {error}",
                            descriptor.name, turn_hook_point
                        ));
                        break;
                    }
                }
            }
        }

        let (decision, selected_tool_call) = if fail_turn_error.is_none() && blocked_error.is_none()
        {
            apply_planner_patches(
                &hook_point,
                decision,
                selected_tool_call,
                &execution_results,
            )
            .unwrap_or_else(|error| {
                fail_turn_error = Some(error);
                (None, None)
            })
        } else {
            (decision, selected_tool_call)
        };

        PlannerDispatchOutcome {
            decision,
            selected_tool_call,
            trace_records: records,
            blocked_error,
            fail_turn_error,
        }
    }

    pub(crate) fn dispatch_graph_decision_hooks(
        &self,
        envelope: &PlannerFactsEnvelope,
        mut decision: GraphDecision,
    ) -> GraphDecisionDispatchOutcome {
        let hook_point = PlannerHookPoint::GraphDecision;
        let turn_hook_point = turn_hook_point_for_planner_hook_point(&hook_point);
        let descriptors = self.hook_registry.list_for_hook_point(&turn_hook_point);
        let mut records = Vec::with_capacity(descriptors.len());
        let mut execution_results = Vec::new();
        let mut fail_turn_error = None;
        let mut blocked_error = None;

        for (index, descriptor) in descriptors.into_iter().enumerate() {
            match self
                .hook_executor
                .execute_planner(descriptor, hook_point.clone(), envelope)
            {
                Ok(mut result) => {
                    result.hook_order = (index + 1) as u32;
                    if let HookStructuredResult::Deny(deny) = &result.structured_result {
                        blocked_error = Some(format!(
                            "hook `{}` blocked planner graph decision: {}",
                            descriptor.name, deny.message
                        ));
                    }
                    records.push(result.to_trace_record());
                    execution_results.push(result);
                    if blocked_error.is_some() {
                        break;
                    }
                }
                Err(error) => {
                    let (result_kind, structured_result) =
                        crate::agent::hooks::normalized_result_for_class(&descriptor.class);
                    records.push(HookTraceRecord {
                        hook_name: descriptor.name.clone(),
                        hook_class: descriptor.class.clone(),
                        hook_point: turn_hook_point.clone(),
                        hook_order: (index + 1) as u32,
                        result_kind,
                        structured_result,
                        blocked: matches!(
                            descriptor.default_failure_policy,
                            HookFailurePolicy::FailTurn
                        ),
                        elapsed_ms: 0,
                        input_summary: Some(format!("hook executor failed: {error}")),
                        persistence_evidence_ref: None,
                        summary: format!(
                            "hook execution failed under {:?}: {error}",
                            descriptor.default_failure_policy
                        ),
                    });
                    if matches!(
                        descriptor.default_failure_policy,
                        HookFailurePolicy::FailTurn
                    ) {
                        fail_turn_error = Some(format!(
                            "hook `{}` forced turn failure at `{:?}`: {error}",
                            descriptor.name, turn_hook_point
                        ));
                        break;
                    }
                }
            }
        }

        if fail_turn_error.is_none() && blocked_error.is_none() {
            if let Err(error) = apply_graph_decision_patches(&mut decision, &execution_results) {
                fail_turn_error = Some(error);
            }
        }

        GraphDecisionDispatchOutcome {
            decision,
            trace_records: records,
            blocked_error,
            fail_turn_error,
        }
    }

    #[cfg(test)]
    pub fn inspect_capability(
        &self,
        capability_id: &str,
    ) -> Option<crate::agent::capability_bridge::CapabilityView> {
        self.capability_registry.inspect_capability(capability_id)
    }

    #[cfg(test)]
    pub fn inspect_capability_source(
        &self,
        source_id: &str,
    ) -> Option<crate::agent::capability_bridge::CapabilitySourceView> {
        self.capability_registry.inspect_source(source_id)
    }

    #[cfg(test)]
    pub fn inspect_skill(
        &self,
        skill_id: &str,
    ) -> Option<crate::agent::capability_bridge::SkillDescriptor> {
        self.capability_registry.inspect_skill(skill_id)
    }

    #[cfg(test)]
    pub fn inspect_skill_source(
        &self,
        source_id: &str,
    ) -> Option<crate::agent::capability_bridge::SkillSourceView> {
        self.capability_registry.inspect_skill_source(source_id)
    }

    #[cfg(test)]
    pub fn register_mcp_capability_for_test(
        &mut self,
        capability: crate::agent::capability_bridge::CapabilityView,
    ) {
        self.capability_registry.register_mcp_capability(capability);
    }

    #[cfg(test)]
    pub fn remove_mcp_source_for_test(&mut self, source_id: &str) {
        self.capability_registry.remove_source_for_test(source_id);
    }
}
