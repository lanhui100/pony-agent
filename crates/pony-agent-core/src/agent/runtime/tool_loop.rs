// ---------------------------------------------------------------------------
// Shared tool-loop utility helpers
//
// These methods encapsulate patterns common to both the synchronous and
// streaming tool-turn paths so that handle_sync_tool_turn and
// handle_stream_tool_turn in mod.rs don't duplicate the same logic.
// ---------------------------------------------------------------------------

use crate::agent::provider::BuildContextObservation;
use crate::agent::runtime::{
    build_provider_call_cache_record, merge_fallback_reason, merge_token_usage,
    normalize_tool_directive, AgentRuntime,
};
use crate::agent::telemetry::{
    CapabilityInvocationRecord, ProviderLatencyKind, ProviderRequestKind, TurnToolActivity,
};
use crate::agent::tools::ToolCall;

// ---------------------------------------------------------------------------
// Deferred log buffer
//
// Collects log entries during a tool turn and flushes them in a batch,
// reducing interleaved I/O when multiple tools execute in rapid succession.
// ---------------------------------------------------------------------------

pub struct DeferredLog {
    entries: Vec<String>,
}

impl DeferredLog {
    pub fn new() -> Self {
        Self { entries: Vec::new() }
    }

    pub fn push(&mut self, message: String) {
        self.entries.push(message);
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Write all buffered entries via `runtime_log` and clear the buffer.
    pub fn flush(&mut self) {
        use crate::agent::turn_flow::runtime_log;
        for entry in self.entries.drain(..) {
            runtime_log(entry);
        }
    }
}

impl AgentRuntime {
    /// Build a tool-activity entry from a tool result and annotate it with
    /// capability invocation metadata, then push it into `tool_activities`.
    pub(super) fn push_tool_activity(
        &self,
        tool_activities: &mut Vec<TurnToolActivity>,
        current_tool_call: &ToolCall,
        tool_result: &crate::agent::tools::ToolResult,
        invocation_record: CapabilityInvocationRecord,
    ) {
        let activities = self
            .telemetry_builder
            .tool_activities_after_result(current_tool_call, tool_result);
        let activities = crate::agent::runtime::annotate_capability_tool_activities(
            activities,
            invocation_record,
        );
        tool_activities.extend(activities);
    }

    /// Normalize a follow-up provider response's tool directive, returning the
    /// updated response on success or an error message on failure.
    pub(super) fn normalize_followup_tool_directive(
        &self,
        response: &mut crate::agent::provider::ProviderResponse,
    ) -> Result<(), String> {
        if let Some(tool_call) = response.tool_call.take() {
            let normalized = normalize_tool_directive(
                tool_call,
                response.assistant_message.take(),
                &response.output_text,
                response.reasoning_content.as_deref(),
                response.reasoning_content_value.as_ref(),
            )?;
            response.tool_call = Some(normalized.tool_call);
            response.assistant_message = normalized.assistant_message;
        }
        Ok(())
    }

    /// Accumulate provider-call metadata and token usage after a follow-up call.
    pub(super) fn accumulate_followup_call_record(
        &self,
        provider_call_records: &mut Vec<crate::agent::telemetry::ProviderCallCacheRecord>,
        accumulated_token_usage: &mut Option<crate::agent::provider::TokenUsage>,
        accumulated_fallback_reason: &mut Option<String>,
        response: &crate::agent::provider::ProviderResponse,
        provider_call_duration_ms: u64,
        provider_call_first_token_latency: Option<u64>,
        context_observation: &BuildContextObservation,
    ) {
        let used_true_stream = response.provider_source == "provider_followup_stream";
        provider_call_records.push(build_provider_call_cache_record(
            ProviderRequestKind::ToolFollowup,
            Some(response.provider_source.as_str()),
            Some(response.provider_mode.as_str()),
            response.token_usage.as_ref(),
            provider_call_first_token_latency,
            Some(provider_call_duration_ms),
            if used_true_stream {
                ProviderLatencyKind::ProviderStream
            } else {
                ProviderLatencyKind::BufferedResponse
            },
            Some(context_observation),
        ));
        *accumulated_token_usage =
            merge_token_usage(accumulated_token_usage.take(), response.token_usage.as_ref());
        *accumulated_fallback_reason = merge_fallback_reason(
            accumulated_fallback_reason.take(),
            response.fallback_reason.clone(),
        );
    }
}
