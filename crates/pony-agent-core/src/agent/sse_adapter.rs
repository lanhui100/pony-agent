//! SSE (Server-Sent Events) formatting for turn stream events.
//!
//! Produces standard SSE wire format frames:
//! ```text
//! event: <event-name>
//! id: <event-id-or-turn-id>
//! data: <json-payload-line-1>
//! data: <json-payload-line-2>
//!
//! ```
//!
//! Each frame is terminated by a blank line (`\n\n`) per the SSE spec.
//! The [`BufferingSseTurnEventSink`] buffers frames in memory for tests,
//! probes, and any consumer that needs to collect events before processing.

use crate::agent::control_plane::{HostControlPlane, StartTurnStreamCommand};
use crate::agent::runtime::TurnStreamEvent;
use crate::agent::turn_flow::TurnEventSink;
use std::sync::{Arc, Mutex};

/// A [`TurnEventSink`] that buffers SSE-formatted frames into a `Vec<String>`.
///
/// Primarily used in tests, probes, and CLI tools where events need to be
/// inspected after a turn completes rather than streamed in real time.
///
/// # Example
/// ```ignore
/// let sink = BufferingSseTurnEventSink::new();
/// control_plane.start_turn_stream(&sink, command);
/// for frame in sink.drain_frames() {
///     println!("{frame}");
/// }
/// ```
pub struct BufferingSseTurnEventSink {
    frames: Arc<Mutex<Vec<String>>>,
}

impl BufferingSseTurnEventSink {
    /// Creates a new, empty sink.
    pub fn new() -> Self {
        Self {
            frames: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Returns a snapshot of all buffered frames (clones the buffer).
    ///
    /// If the lock is poisoned, returns an empty vec.
    pub fn frames(&self) -> Vec<String> {
        self.frames
            .lock()
            .map(|frames| frames.clone())
            .unwrap_or_default()
    }

    /// Drains all buffered frames, leaving the internal buffer empty.
    ///
    /// Useful for incremental consumption — unlike [`frames`](Self::frames),
    /// this does not clone the entire history on each call.
    pub fn drain_frames(&self) -> Vec<String> {
        self.frames
            .lock()
            .map(|mut frames| std::mem::take(&mut *frames))
            .unwrap_or_default()
    }
}

impl Default for BufferingSseTurnEventSink {
    fn default() -> Self {
        Self::new()
    }
}

impl TurnEventSink for BufferingSseTurnEventSink {
    fn emit(&self, name: &str, payload: TurnStreamEvent) {
        if let Ok(mut frames) = self.frames.lock() {
            if let Ok(frame) = format_sse_event(name, &payload) {
                frames.push(frame);
            }
            // Serialization failure: skip the frame silently.
            // The sink has no error channel; callers should monitor
            // the output for missing expected events.
        }
    }
}

/// Formats a single SSE frame from a turn stream event.
///
/// Returns the SSE text including the terminating blank line, or
/// a [`serde_json::Error`] if the payload cannot be serialized.
///
/// # Format
/// ```text
/// event: <name>
/// id: <event_id | turn_id>
/// data: <json>
///
/// ```
pub fn format_sse_event(name: &str, payload: &TurnStreamEvent) -> Result<String, serde_json::Error> {
    let data = serde_json::to_string(payload)?;
    let id = payload.event_id.as_deref().unwrap_or(&payload.turn_id);

    // Pre-allocate: 32 covers fixed overhead (event:\nid:\ndata:\n\n) plus event name and id.
    let mut frame = String::with_capacity(32 + name.len() + id.len() + data.len());

    frame.push_str("event: ");
    frame.push_str(name);
    frame.push('\n');

    frame.push_str("id: ");
    frame.push_str(id);
    frame.push('\n');

    for line in data.lines() {
        frame.push_str("data: ");
        frame.push_str(line);
        frame.push('\n');
    }
    frame.push('\n');

    Ok(frame)
}

/// Runs a turn through the control plane and collects all SSE frames.
///
/// Convenience wrapper for one-shot usage:
/// <!-- ignore for no_run -->
/// ```ignore
/// let frames = collect_turn_stream_frames(&control_plane, command);
/// ```
pub fn collect_turn_stream_frames(
    control_plane: &HostControlPlane,
    command: StartTurnStreamCommand,
) -> Vec<String> {
    let sink = BufferingSseTurnEventSink::new();
    control_plane.start_turn_stream(&sink, command);
    sink.frames()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::runtime::TurnInput;

    #[test]
    fn format_sse_event_uses_standard_event_id_and_data_lines() {
        let payload = TurnStreamEvent {
            event_id: Some("turn-123:1".to_string()),
            session_id: Some("sse-test".to_string()),
            turn_id: "turn-123".to_string(),
            kind: "delta".to_string(),
            event_type: Some("turn.delta".to_string()),
            event_version: Some("turn-event-v1".to_string()),
            sequence: Some(1),
            emitted_at_ms: Some(123),
            phase: Some("calling_model".to_string()),
            text: Some("hello".to_string()),
            reasoning_content: None,
            error: None,
            provider_requested_name: None,
            provider_name: None,
            provider_protocol: None,
            provider_model: None,
            provider_source: None,
            provider_mode: None,
            fallback_reason: None,
            build_context_observation: None,
            input_tokens: None,
            cache_hit_input_tokens: None,
            reasoning_tokens: None,
            output_tokens: None,
            total_tokens: None,
            first_token_latency_ms: Some(18),
            turn_duration_ms: None,
            trace_steps: None,
            trace_timeline: None,
            tool_activities: None,
            provider_call_records: None,
            hook_trace_records: None,
            session_summary: None,
        };

        let frame = format_sse_event("turn:delta", &payload)
            .expect("serialization should succeed");

        assert!(frame.starts_with("event: turn:delta\nid: turn-123:1\n"));
        assert!(frame.contains(
            "data: {\"eventId\":\"turn-123:1\",\"sessionId\":\"sse-test\",\"turnId\":\"turn-123\""
        ));
        assert!(frame.contains("\"eventType\":\"turn.delta\""));
        assert!(frame.contains("\"eventVersion\":\"turn-event-v1\""));
        assert!(frame.contains("\"sequence\":1"));
        assert!(frame.contains("\"emittedAtMs\":123"));
        assert!(frame.ends_with("\n\n"));
    }

    #[test]
    fn control_plane_can_stream_failed_turn_into_sse_sink() {
        let control_plane = HostControlPlane::new();

        let frames = collect_turn_stream_frames(
            &control_plane,
            StartTurnStreamCommand {
                turn_id: "turn-empty".to_string(),
                input: TurnInput {
                    message: "   ".to_string(),
                    display_message: None,
                    provider_id: None,
                    model_id: None,
                    reasoning_effort: None,
                    session_id: Some("sse-test".to_string()),
                    node_id: None,
                    history: Vec::new(),
                    images: Vec::new(),
                },
            },
        );

        assert_eq!(frames.len(), 1);
        assert!(frames[0].contains("event: turn:failed"));
        assert!(frames[0].contains("\"error\":\"Message is empty.\""));
    }

    #[test]
    fn default_sink_is_empty() {
        let sink = BufferingSseTurnEventSink::default();
        assert!(sink.frames().is_empty());
    }

    #[test]
    fn drain_frames_consumes_all() {
        let sink = BufferingSseTurnEventSink::new();
        let payload = TurnStreamEvent {
            event_id: None,
            session_id: None,
            turn_id: "t1".to_string(),
            kind: "delta".to_string(),
            event_type: None,
            event_version: None,
            sequence: None,
            emitted_at_ms: None,
            phase: None,
            text: Some("hello".to_string()),
            reasoning_content: None,
            error: None,
            provider_requested_name: None,
            provider_name: None,
            provider_protocol: None,
            provider_model: None,
            provider_source: None,
            provider_mode: None,
            fallback_reason: None,
            build_context_observation: None,
            input_tokens: None,
            cache_hit_input_tokens: None,
            reasoning_tokens: None,
            output_tokens: None,
            total_tokens: None,
            first_token_latency_ms: None,
            turn_duration_ms: None,
            trace_steps: None,
            trace_timeline: None,
            tool_activities: None,
            provider_call_records: None,
            hook_trace_records: None,
            session_summary: None,
        };

        // Emit twice
        sink.emit("turn:delta", payload);
        assert_eq!(sink.frames().len(), 1);
        assert_eq!(sink.frames().len(), 1); // still 1, not consumed

        let drained = sink.drain_frames();
        assert_eq!(drained.len(), 1);
        assert!(sink.frames().is_empty()); // drained
    }
}
