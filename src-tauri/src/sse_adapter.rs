use crate::agent::control_plane::{HostControlPlane, StartTurnStreamCommand};
use crate::agent::runtime::TurnStreamEvent;
use crate::agent::turn_flow::TurnEventSink;
use std::sync::{Arc, Mutex};

pub struct BufferingSseTurnEventSink {
    frames: Arc<Mutex<Vec<String>>>,
}

impl BufferingSseTurnEventSink {
    pub fn new() -> Self {
        Self {
            frames: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn frames(&self) -> Vec<String> {
        self.frames
            .lock()
            .map(|frames| frames.clone())
            .unwrap_or_default()
    }
}

impl TurnEventSink for BufferingSseTurnEventSink {
    fn emit(&self, name: &str, payload: TurnStreamEvent) {
        if let Ok(mut frames) = self.frames.lock() {
            frames.push(format_sse_event(name, &payload));
        }
    }
}

pub fn format_sse_event(name: &str, payload: &TurnStreamEvent) -> String {
    let data = serde_json::to_string(payload).unwrap_or_else(|_| "{}".to_string());
    let mut frame = String::new();
    frame.push_str("event: ");
    frame.push_str(name);
    frame.push('\n');
    frame.push_str("id: ");
    frame.push_str(&payload.turn_id);
    frame.push('\n');
    for line in data.lines() {
        frame.push_str("data: ");
        frame.push_str(line);
        frame.push('\n');
    }
    frame.push('\n');
    frame
}

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
            turn_id: "turn-123".to_string(),
            kind: "delta".to_string(),
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
            tool_activities: None,
            session_summary: None,
        };

        let frame = format_sse_event("turn:delta", &payload);

        assert!(frame.starts_with("event: turn:delta\nid: turn-123\n"));
        assert!(frame.contains("data: {\"turnId\":\"turn-123\""));
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
                    history: Vec::new(),
                    images: Vec::new(),
                },
            },
        );

        assert_eq!(frames.len(), 1);
        assert!(frames[0].contains("event: turn:failed"));
        assert!(frames[0].contains("\"error\":\"Message is empty.\""));
    }
}
