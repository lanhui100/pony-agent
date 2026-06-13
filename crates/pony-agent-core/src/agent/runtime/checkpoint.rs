use crate::agent::execution_control::ExecutionControlRegistry;
use crate::agent::runtime::AgentRuntime;
use crate::agent::turn_flow::ProviderEventMeta;
use crate::agent::telemetry::{TurnToolActivity, TurnTraceStep};

impl AgentRuntime {
    pub(super) fn update_execution_checkpoint(
        &self,
        control: &ExecutionControlRegistry,
        turn_id: &str,
        phase: &str,
        provider_meta: Option<&ProviderEventMeta>,
        completed_hops: usize,
        active_tool_name: Option<&str>,
        trace_steps: &[TurnTraceStep],
        tool_activities: &[TurnToolActivity],
        provider_source: Option<&str>,
        provider_mode: Option<&str>,
        fallback_reason: Option<&str>,
        status: Option<&str>,
        error: Option<&str>,
    ) {
        control.update(turn_id, |checkpoint| {
            checkpoint.phase = phase.to_string();
            checkpoint.completed_hops = completed_hops;
            checkpoint.max_hops = super::max_tool_hops_per_turn();
            checkpoint.active_tool_name = active_tool_name.map(str::to_string);
            checkpoint.trace_steps = trace_steps.to_vec();
            checkpoint.tool_activities = tool_activities.to_vec();
            checkpoint.provider_requested_name =
                provider_meta.map(|meta| meta.requested_name.clone());
            checkpoint.provider_name = provider_meta.map(|meta| meta.provider_name.clone());
            checkpoint.provider_protocol = provider_meta.map(|meta| meta.protocol.clone());
            checkpoint.provider_model = provider_meta.map(|meta| meta.model.clone());
            checkpoint.provider_source = provider_source.map(str::to_string);
            checkpoint.provider_mode = provider_mode.map(str::to_string);
            checkpoint.fallback_reason = fallback_reason.map(str::to_string);
            checkpoint.error = error.map(str::to_string);
            if let Some(status) = status {
                checkpoint.status = status.to_string();
            }
        });
    }

    pub(super) fn should_cancel_turn(&self, control: &ExecutionControlRegistry, turn_id: &str) -> bool {
        control.is_stop_requested(turn_id)
    }
}
