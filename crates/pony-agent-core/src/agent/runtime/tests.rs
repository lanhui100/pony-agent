use super::*;
use crate::agent::config::{
    ProviderModelCapabilities, ProviderSelectionResolver, ResolvedProviderSelection,
    ThinkingParamPattern,
};
use crate::agent::context::{DefaultTurnContextBuilder, TurnContextBuilder};
use crate::agent::control_plane::HostControlPlaneBuilder;
use crate::agent::dispatcher::ControlRequestConsumed;
use crate::agent::graph::GraphRunPhase;
use crate::agent::hooks::{
    hook_point_matches_canonical_boundary, AgentHookDescriptor, AgentHookExecutor,
    CapabilityMediationEnvelope, CapabilityMediationHookPoint, HookClass, HookFailurePolicy,
    HookPatchOperation, HookPatchOperationKind, HookPatchTarget, HookRecoveryMode,
    HookReplayRequirements, HookResultKind, HookSideEffectPersistenceRequirements,
    HookStructuredResult, HookTraceRequirements, PlannerFactsEnvelope, PlannerHookPoint,
    TurnHookPoint,
};
use crate::agent::planner::TurnPlanner;
use crate::agent::session::{
    FileSessionBackend, SessionSnapshot, SessionStore, TurnHistoryMessage,
};
use crate::agent::telemetry::DefaultTurnTelemetryBuilder;
use crate::agent::tool_runtime::PendingControlRequestState;
use crate::agent::tool_runtime::{InvocationOrigin, ToolDispatchRequest};
use serde_json::json;
use std::cell::RefCell;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn anthropic_tool_turn_transcript_uses_native_tool_blocks() {
    let tool_call = ToolCall {
        call_id: Some("toolu_list".to_string()),
        name: "List".to_string(),
        arguments: json!({ "path": ".", "description": "列出当前目录" }),
        plan: None,
    };
    let hop = ToolTurnHopRecord {
        assistant_message: Some(json!({
            "role": "assistant",
            "content": [{
                "type": "tool_use",
                "id": "toolu_list",
                "name": "List",
                "input": { "path": ".", "description": "列出当前目录" }
            }]
        })),
        assistant_output_text: String::new(),
        assistant_reasoning_content: None,
        assistant_reasoning_content_value: None,
        tool_call,
        tool_result: crate::agent::tools::ToolResult {
            tool_name: "List".to_string(),
            status: "ok".to_string(),
            output: "Cargo.toml\nsrc".to_string(),
            duration_ms: 1,
        },
    };
    let response = ProviderResponse {
        output_text: "Cargo.toml 和 src。".to_string(),
        tool_call: None,
        reasoning_content: None,
        reasoning_content_value: None,
        assistant_message: Some(json!({
            "role": "assistant",
            "content": [{ "type": "text", "text": "Cargo.toml 和 src。" }]
        })),
        provider_source: "test".to_string(),
        provider_mode: "live".to_string(),
        fallback_reason: None,
        token_usage: None,
    };

    let transcript = native_transcript_for_tool_turn(
        "anthropic",
        "当前文件夹下有哪些文件？",
        &[hop],
        &response,
    )
    .expect("transcript should be built");

    assert_eq!(
        transcript[1]["content"][0]["type"].as_str(),
        Some("tool_use")
    );
    assert_eq!(transcript[2]["role"].as_str(), Some("user"));
    assert_eq!(
        transcript[2]["content"][0]["type"].as_str(),
        Some("tool_result")
    );
    assert!(transcript
        .iter()
        .all(|message| { message.get("role").and_then(Value::as_str) != Some("tool") }));
}

struct ToolHopLimitOverrideGuard {
    previous: usize,
}

impl ToolHopLimitOverrideGuard {
    fn set(limit: usize) -> Self {
        let previous = tool_hop_limit_override_registry().swap(limit, AtomicOrdering::SeqCst);
        Self { previous }
    }
}

impl Drop for ToolHopLimitOverrideGuard {
    fn drop(&mut self) {
        tool_hop_limit_override_registry().store(self.previous, AtomicOrdering::SeqCst);
    }
}

struct RecordingTurnEventSink {
    events: RefCell<Vec<(String, TurnStreamEvent)>>,
}

impl RecordingTurnEventSink {
    fn new() -> Self {
        Self {
            events: RefCell::new(Vec::new()),
        }
    }
}

impl TurnEventSink for RecordingTurnEventSink {
    fn emit(&self, name: &str, payload: TurnStreamEvent) {
        self.events.borrow_mut().push((name.to_string(), payload));
    }
}

fn assert_hook_boundary_alignment(
    payload: &TurnStreamEvent,
    hook_point: TurnHookPoint,
    expected_event_type: &str,
    expected_phase: &str,
) {
    assert_eq!(payload.event_type.as_deref(), Some(expected_event_type));
    assert_eq!(payload.phase.as_deref(), Some(expected_phase));
    assert!(hook_point_matches_canonical_boundary(
        &hook_point,
        expected_event_type,
        expected_phase
    ));
}

#[derive(Clone)]
struct StaticResolver {
    selection: ResolvedProviderSelection,
}

impl ProviderSelectionResolver for StaticResolver {
    fn resolve_provider_selection(
        &self,
        _provider_id: Option<&str>,
        _model_id: Option<&str>,
    ) -> ResolvedProviderSelection {
        self.selection.clone()
    }
}

struct PassthroughPlanner;

impl TurnPlanner for PassthroughPlanner {
    fn preflight_decision(
        &self,
        _user_message: &str,
        _history: &[TurnHistoryMessage],
        _available_skills: &[crate::agent::capability_bridge::SkillDescriptor],
    ) -> Option<ProviderDecision> {
        None
    }

    fn select_tool_call(
        &self,
        _user_message: &str,
        _history: &[TurnHistoryMessage],
        _available_skills: &[crate::agent::capability_bridge::SkillDescriptor],
        provider_tool_call: Option<ToolCall>,
    ) -> Option<ToolCall> {
        provider_tool_call
    }
}

struct SlowPassthroughPlanner {
    delay_ms: u64,
}

struct PartialOutOfScopeToolExecutor;

impl ToolExecutor for PartialOutOfScopeToolExecutor {
    fn execute(&self, call: &ToolCall) -> crate::agent::tools::ToolResult {
        let output = match call.name.as_str() {
            "workspace_batch" | "workspace_gather_context" => json!({
                "status": "partial",
                "results": [
                    {
                        "index": 0,
                        "tool": "workspace_read_file",
                        "status": "ok",
                        "output": { "content": "ok" }
                    },
                    {
                        "index": 1,
                        "tool": "workspace_read_file",
                        "status": "error",
                        "error": {
                            "kind": "out_of_scope",
                            "message": "只允许访问当前工作区内的相对路径。"
                        }
                    }
                ],
                "summary": {
                    "text": "aggregate partial",
                    "firstError": {
                        "kind": "out_of_scope",
                        "message": "只允许访问当前工作区内的相对路径。"
                    }
                }
            })
            .to_string(),
            _ => json!({
                "error": {
                    "kind": "invocation_failed",
                    "message": format!("unexpected tool: {}", call.name)
                }
            })
            .to_string(),
        };

        crate::agent::tools::ToolResult {
            tool_name: call.name.clone(),
            status: "ok".to_string(),
            output,
            duration_ms: 1,
        }
    }
}

impl TurnPlanner for SlowPassthroughPlanner {
    fn preflight_decision(
        &self,
        _user_message: &str,
        _history: &[TurnHistoryMessage],
        _available_skills: &[crate::agent::capability_bridge::SkillDescriptor],
    ) -> Option<ProviderDecision> {
        std::thread::sleep(std::time::Duration::from_millis(self.delay_ms));
        None
    }

    fn select_tool_call(
        &self,
        _user_message: &str,
        _history: &[TurnHistoryMessage],
        _available_skills: &[crate::agent::capability_bridge::SkillDescriptor],
        provider_tool_call: Option<ToolCall>,
    ) -> Option<ToolCall> {
        provider_tool_call
    }
}

struct ForcedToolPlanner {
    tool_name: String,
    arguments: Value,
}

impl TurnPlanner for ForcedToolPlanner {
    fn preflight_decision(
        &self,
        _user_message: &str,
        _history: &[TurnHistoryMessage],
        _available_skills: &[crate::agent::capability_bridge::SkillDescriptor],
    ) -> Option<ProviderDecision> {
        Some(ProviderDecision {
            output_text: String::new(),
            tool_call: Some(ToolCall {
                call_id: Some("forced-tool-call".to_string()),
                name: self.tool_name.clone(),
                arguments: self.arguments.clone(),
                plan: Some(crate::agent::tools::ToolPlan {
                    kind: "forced".to_string(),
                    summary: format!("强制执行工具 `{}`。", self.tool_name),
                    parallel: false,
                    continue_on_error: false,
                    steps: Vec::new(),
                }),
            }),
            reasoning_content: None,
            reasoning_content_value: None,
            assistant_message: None,
            provider_source: "planner_preflight".to_string(),
            provider_mode: "preflight".to_string(),
            fallback_reason: None,
            token_usage: None,
        })
    }

    fn select_tool_call(
        &self,
        _user_message: &str,
        _history: &[TurnHistoryMessage],
        _available_skills: &[crate::agent::capability_bridge::SkillDescriptor],
        provider_tool_call: Option<ToolCall>,
    ) -> Option<ToolCall> {
        provider_tool_call
    }
}

/// Forced-Ask planner that defers to the provider on the second turn — used by the
/// phase-4 P0 end-to-end test so the resumed turn reaches the provider (and receives the
/// injected terminal result) instead of re-persisting a fresh Ask.
struct AskOnceThenDeferPlanner {
    tool_name: String,
    arguments: Value,
    ask_forced: std::sync::atomic::AtomicBool,
}

impl TurnPlanner for AskOnceThenDeferPlanner {
    fn preflight_decision(
        &self,
        _user_message: &str,
        _history: &[TurnHistoryMessage],
        _available_skills: &[crate::agent::capability_bridge::SkillDescriptor],
    ) -> Option<ProviderDecision> {
        if !self
            .ask_forced
            .swap(true, std::sync::atomic::Ordering::SeqCst)
        {
            return Some(ProviderDecision {
                output_text: String::new(),
                tool_call: Some(ToolCall {
                    call_id: Some("forced-ask-call".to_string()),
                    name: self.tool_name.clone(),
                    arguments: self.arguments.clone(),
                    plan: Some(crate::agent::tools::ToolPlan {
                        kind: "forced".to_string(),
                        summary: format!("强制执行工具 `{}`。", self.tool_name),
                        parallel: false,
                        continue_on_error: false,
                        steps: Vec::new(),
                    }),
                }),
                reasoning_content: None,
                reasoning_content_value: None,
                assistant_message: None,
                provider_source: "planner_preflight".to_string(),
                provider_mode: "preflight".to_string(),
                fallback_reason: None,
                token_usage: None,
            });
        }
        None
    }

    fn select_tool_call(
        &self,
        _user_message: &str,
        _history: &[TurnHistoryMessage],
        _available_skills: &[crate::agent::capability_bridge::SkillDescriptor],
        provider_tool_call: Option<ToolCall>,
    ) -> Option<ToolCall> {
        provider_tool_call
    }
}

struct StubToolExecutor;

impl crate::agent::tools::ToolExecutor for StubToolExecutor {
    fn execute(&self, call: &ToolCall) -> crate::agent::tools::ToolResult {
        let output = match call.name.as_str() {
            "workspace_list_files" => {
                "{\"entries\":[\"Cargo.toml\",\"tauri.conf.json\",\"src/\"]}".to_string()
            }
            "workspace_read_file" => {
                let path = call
                    .arguments
                    .get("path")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if path.contains("Windows/System32") || path.contains("..") {
                    "{\"ok\":false,\"tool\":\"workspace_read_file\",\"error\":{\"code\":\"out_of_scope\",\"message\":\"只允许访问当前工作区内的相对路径。\"}}".to_string()
                } else {
                    "{\n  \"productName\": \"Pony Agent\",\n  \"version\": \"0.1.0\"\n}".to_string()
                }
            }
            other => format!("unsupported tool in test: {}", other),
        };

        crate::agent::tools::ToolResult {
            tool_name: call.name.clone(),
            status: if output.contains("\"error\"") {
                "error".to_string()
            } else {
                "ok".to_string()
            },
            output,
            duration_ms: 1,
        }
    }
}

struct SlowToolExecutor {
    delay_ms: u64,
}

impl crate::agent::tools::ToolExecutor for SlowToolExecutor {
    fn execute(&self, call: &ToolCall) -> crate::agent::tools::ToolResult {
        std::thread::sleep(std::time::Duration::from_millis(self.delay_ms));
        StubToolExecutor.execute(call)
    }
}

struct RequestStopToolExecutor {
    control: Arc<ExecutionControlRegistry>,
    turn_id: String,
    delay_ms: u64,
}

impl crate::agent::tools::ToolExecutor for RequestStopToolExecutor {
    fn execute(&self, call: &ToolCall) -> crate::agent::tools::ToolResult {
        std::thread::sleep(std::time::Duration::from_millis(self.delay_ms));
        let _ = self.control.request_stop(&self.turn_id);
        StubToolExecutor.execute(call)
    }
}

struct ErrorToolExecutor;

struct CountingToolExecutor {
    calls: Arc<std::sync::atomic::AtomicUsize>,
}

impl crate::agent::tools::ToolExecutor for CountingToolExecutor {
    fn execute(&self, call: &ToolCall) -> crate::agent::tools::ToolResult {
        self.calls.fetch_add(1, AtomicOrdering::SeqCst);
        StubToolExecutor.execute(call)
    }
}

struct FailingHookExecutor;

struct RecordingToolExecutor {
    calls: Arc<Mutex<Vec<ToolCall>>>,
}

impl crate::agent::tools::ToolExecutor for RecordingToolExecutor {
    fn execute(&self, call: &ToolCall) -> crate::agent::tools::ToolResult {
        self.calls.lock().unwrap().push(call.clone());
        crate::agent::tools::ToolResult {
            tool_name: call.name.clone(),
            status: "ok".to_string(),
            output: call.arguments.to_string(),
            duration_ms: 1,
        }
    }
}

struct TransformingCapabilityHookExecutor;
struct TransformingPlannerHookExecutor;

impl AgentHookExecutor for FailingHookExecutor {
    fn execute(
        &self,
        descriptor: &AgentHookDescriptor,
        hook_point: TurnHookPoint,
    ) -> Result<crate::agent::hooks::HookExecutionResult, String> {
        Err(format!(
            "intentional hook failure for `{}` on `{:?}`",
            descriptor.name, hook_point
        ))
    }
}

impl AgentHookExecutor for TransformingCapabilityHookExecutor {
    fn execute(
        &self,
        descriptor: &AgentHookDescriptor,
        hook_point: TurnHookPoint,
    ) -> Result<crate::agent::hooks::HookExecutionResult, String> {
        NoopHookExecutor.execute(descriptor, hook_point)
    }

    fn execute_capability_mediation(
        &self,
        descriptor: &AgentHookDescriptor,
        hook_point: CapabilityMediationHookPoint,
        envelope: &CapabilityMediationEnvelope,
    ) -> Result<crate::agent::hooks::HookExecutionResult, String> {
        let operations = match hook_point {
            CapabilityMediationHookPoint::CapabilityResolve => vec![HookPatchOperation {
                target: HookPatchTarget::CapabilityMediation,
                path: "request.arguments".to_string(),
                operation: HookPatchOperationKind::Merge,
                value_summary: Some("rewrite capability arguments".to_string()),
                value_text: Some("{\"path\":\"src-tauri\"}".to_string()),
            }],
            CapabilityMediationHookPoint::SkillToolActionsResolve => vec![HookPatchOperation {
                target: HookPatchTarget::CapabilityMediation,
                path: "request.arguments".to_string(),
                operation: HookPatchOperationKind::Merge,
                value_summary: Some("rewrite skill arguments".to_string()),
                value_text: Some("{\"message\":\"patched by hook\"}".to_string()),
            }],
            CapabilityMediationHookPoint::McpSourceIngress
            | CapabilityMediationHookPoint::SkillSourceIngress => Vec::new(),
        };
        Ok(crate::agent::hooks::HookExecutionResult {
            hook_name: descriptor.name.clone(),
            hook_class: HookClass::Transform,
            hook_point: turn_hook_point_for_capability_mediation_hook_point(&hook_point),
            hook_order: 0,
            result_kind: HookResultKind::Patch,
            structured_result: HookStructuredResult::Patch { operations },
            blocked: false,
            elapsed_ms: 0,
            input_summary: Some(envelope.argument_summary.clone()),
            persistence_evidence_ref: None,
            trace_summary: format!("hook rewrote mediation arguments at {:?}", hook_point),
        })
    }
}

impl AgentHookExecutor for TransformingPlannerHookExecutor {
    fn execute(
        &self,
        descriptor: &AgentHookDescriptor,
        hook_point: TurnHookPoint,
    ) -> Result<crate::agent::hooks::HookExecutionResult, String> {
        NoopHookExecutor.execute(descriptor, hook_point)
    }

    fn execute_planner(
        &self,
        descriptor: &AgentHookDescriptor,
        hook_point: PlannerHookPoint,
        envelope: &PlannerFactsEnvelope,
    ) -> Result<crate::agent::hooks::HookExecutionResult, String> {
        let operations = match hook_point {
            PlannerHookPoint::TurnPreflight => vec![HookPatchOperation {
                target: HookPatchTarget::PlannerFacts,
                path: "provider_tool_call".to_string(),
                operation: HookPatchOperationKind::Set,
                value_summary: Some("rewrite provider tool call".to_string()),
                value_text: Some(
                    "{\"call_id\":null,\"name\":\"workspace_list_files\",\"arguments\":{\"path\":\"src-tauri\",\"limit\":5},\"plan\":{\"kind\":\"forced\",\"summary\":\"hook rewritten preflight tool\",\"parallel\":false,\"continue_on_error\":false,\"steps\":[]}}".to_string(),
                ),
            }],
            PlannerHookPoint::ToolSelection => vec![HookPatchOperation {
                target: HookPatchTarget::PlannerFacts,
                path: "selected_tool_call".to_string(),
                operation: HookPatchOperationKind::Set,
                value_summary: Some("rewrite selected tool call".to_string()),
                value_text: Some(
                    "{\"call_id\":null,\"name\":\"workspace_list_files\",\"arguments\":{\"path\":\"tests\",\"limit\":3},\"plan\":null}".to_string(),
                ),
            }],
            PlannerHookPoint::GraphDecision => vec![HookPatchOperation {
                target: HookPatchTarget::PlannerFacts,
                path: "decision_summary".to_string(),
                operation: HookPatchOperationKind::Set,
                value_summary: Some("rewrite graph decision summary".to_string()),
                value_text: Some("\"planner summary patched by hook\"".to_string()),
            }],
        };
        Ok(crate::agent::hooks::HookExecutionResult {
            hook_name: descriptor.name.clone(),
            hook_class: HookClass::Transform,
            hook_point: turn_hook_point_for_planner_hook_point(&hook_point),
            hook_order: 0,
            result_kind: HookResultKind::Patch,
            structured_result: HookStructuredResult::Patch { operations },
            blocked: false,
            elapsed_ms: 0,
            input_summary: envelope.user_message_summary.clone(),
            persistence_evidence_ref: None,
            trace_summary: format!("hook rewrote planner payload at {:?}", hook_point),
        })
    }
}

impl crate::agent::tools::ToolExecutor for ErrorToolExecutor {
    fn execute(&self, call: &ToolCall) -> crate::agent::tools::ToolResult {
        crate::agent::tools::ToolResult {
            tool_name: call.name.clone(),
            status: "error".to_string(),
            output: format!("tool {} failed in test", call.name),
            duration_ms: 1,
        }
    }
}

#[derive(Clone)]
struct MockHttpResponse {
    content_type: &'static str,
    body: String,
}

struct MockHttpServer {
    base_url: String,
    requests: Arc<Mutex<Vec<String>>>,
    handle: Option<thread::JoinHandle<()>>,
}

impl MockHttpServer {
    fn start(responses: Vec<MockHttpResponse>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
        let address = listener.local_addr().expect("mock server addr");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let requests_for_thread = Arc::clone(&requests);

        let handle = thread::spawn(move || {
            for response in responses {
                let (mut stream, _) = listener.accept().expect("accept mock request");
                let body = read_http_request_body(&mut stream);
                requests_for_thread.lock().unwrap().push(body);
                write_http_response(&mut stream, response);
            }
        });

        Self {
            base_url: format!("http://{}/v1", address),
            requests,
            handle: Some(handle),
        }
    }

    fn finish(mut self) -> Vec<String> {
        if let Some(handle) = self.handle.take() {
            handle.join().expect("join mock server");
        }
        self.requests.lock().unwrap().clone()
    }
}

fn read_http_request_body(stream: &mut TcpStream) -> String {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 4096];
    let mut header_end = None;
    let mut content_length = 0usize;

    loop {
        let read = stream.read(&mut chunk).expect("read mock request");
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);

        if header_end.is_none() {
            header_end = find_header_end(&buffer);
            if let Some(end) = header_end {
                let headers = String::from_utf8_lossy(&buffer[..end]).to_string();
                content_length = parse_content_length(&headers);
            }
        }

        if let Some(end) = header_end {
            if buffer.len() >= end + content_length {
                break;
            }
        }
    }

    let Some(end) = header_end else {
        return String::new();
    };
    String::from_utf8_lossy(&buffer[end..end + content_length]).to_string()
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|position| position + 4)
}

fn parse_content_length(headers: &str) -> usize {
    headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            if name.eq_ignore_ascii_case("Content-Length") {
                value.trim().parse::<usize>().ok()
            } else {
                None
            }
        })
        .unwrap_or(0)
}

fn write_http_response(stream: &mut TcpStream, response: MockHttpResponse) {
    let payload = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        response.content_type,
        response.body.len(),
        response.body
    );
    stream
        .write_all(payload.as_bytes())
        .expect("write mock response");
    stream.flush().expect("flush mock response");
}

fn json_response(body: serde_json::Value) -> MockHttpResponse {
    MockHttpResponse {
        content_type: "application/json",
        body: body.to_string(),
    }
}

fn json_completion(text: &str) -> MockHttpResponse {
    json_response(json!({
        "choices": [
            {
                "message": {
                    "role": "assistant",
                    "content": text
                }
            }
        ]
    }))
}

fn sse_response(chunks: &[serde_json::Value]) -> MockHttpResponse {
    let mut body = String::new();
    for chunk in chunks {
        body.push_str("data: ");
        body.push_str(&chunk.to_string());
        body.push_str("\n\n");
    }
    body.push_str("data: [DONE]\n\n");

    MockHttpResponse {
        content_type: "text/event-stream",
        body,
    }
}

fn test_provider_selection(base_url: String) -> ResolvedProviderSelection {
    ResolvedProviderSelection {
        requested_name: "test-openai".to_string(),
        provider_name: "test-openai".to_string(),
        protocol: crate::agent::provider::ProviderProtocol::OpenAi,
        base_url,
        auth_type: crate::agent::provider::ProviderAuthType::Auto,
        api_key_env_var: "TEST_API_KEY".to_string(),
        api_key: Some("test-key".to_string()),
        model: "gpt-5.4".to_string(),
        temperature: 0.2,
        max_output_tokens: 1024,
        reasoning_effort: None,
        reasoning_budget_tokens: None,
        capabilities: ProviderModelCapabilities {
            context_window_tokens: Some(128_000),
            supports_tools: true,
            supports_streaming: true,
            supports_image_input: false,
            supports_reasoning: true,
            ..Default::default()
        },
        thinking_param_pattern: ThinkingParamPattern::EffortStandard,
    }
}

/// 非 reasoning 模型配置：走 planner 预检决策 + tool-selection 修补流程
/// （与 reasoning 模型的 provider-native tool flow 不同）。
fn test_chat_provider_selection(base_url: String) -> ResolvedProviderSelection {
    ResolvedProviderSelection {
        requested_name: "test-openai-chat".to_string(),
        provider_name: "test-openai-chat".to_string(),
        protocol: crate::agent::provider::ProviderProtocol::OpenAi,
        base_url,
        auth_type: crate::agent::provider::ProviderAuthType::Auto,
        api_key_env_var: "TEST_API_KEY".to_string(),
        api_key: Some("test-key".to_string()),
        model: "gpt-4.1-mini".to_string(),
        temperature: 0.2,
        max_output_tokens: 1024,
        reasoning_effort: None,
        reasoning_budget_tokens: None,
        capabilities: ProviderModelCapabilities {
            context_window_tokens: Some(128_000),
            supports_tools: true,
            supports_streaming: true,
            supports_image_input: false,
            supports_reasoning: false,
            ..Default::default()
        },
        thinking_param_pattern: ThinkingParamPattern::None,
    }
}

fn deepseek_provider_selection(base_url: String) -> ResolvedProviderSelection {
    ResolvedProviderSelection {
        requested_name: "deepseek".to_string(),
        provider_name: "deepseek".to_string(),
        protocol: crate::agent::provider::ProviderProtocol::OpenAi,
        base_url,
        auth_type: crate::agent::provider::ProviderAuthType::Auto,
        api_key_env_var: "DEEPSEEK_API_KEY".to_string(),
        api_key: Some("test-key".to_string()),
        model: "deepseek-v4-flash".to_string(),
        temperature: 0.2,
        max_output_tokens: 1024,
        reasoning_effort: None,
        reasoning_budget_tokens: None,
        capabilities: ProviderModelCapabilities {
            context_window_tokens: Some(128_000),
            supports_tools: true,
            supports_streaming: true,
            supports_image_input: false,
            supports_reasoning: true,
            ..Default::default()
        },
        thinking_param_pattern: ThinkingParamPattern::EffortWithNone,
    }
}

fn build_runtime_for_test(selection: ResolvedProviderSelection) -> AgentRuntime {
    crate::agent::runtime_helper::TestRuntimeGuard::leak();
    AgentRuntime::with_dependencies(
        SessionStore::memory_only(),
        Box::new(StaticResolver { selection }),
        Box::new(StubToolExecutor),
        Box::new(PassthroughPlanner),
        Box::new(DefaultTurnContextBuilder),
        Box::new(DefaultTurnTelemetryBuilder),
    )
}

fn build_runtime_for_test_with_tool_executor(
    selection: ResolvedProviderSelection,
    tool_executor: Box<dyn ToolExecutor>,
) -> AgentRuntime {
    crate::agent::runtime_helper::TestRuntimeGuard::leak();
    AgentRuntime::with_dependencies(
        SessionStore::memory_only(),
        Box::new(StaticResolver { selection }),
        tool_executor,
        Box::new(PassthroughPlanner),
        Box::new(DefaultTurnContextBuilder),
        Box::new(DefaultTurnTelemetryBuilder),
    )
}

fn build_runtime_with_session_store(
    selection: ResolvedProviderSelection,
    sessions: SessionStore,
) -> AgentRuntime {
    AgentRuntime::with_dependencies(
        sessions,
        Box::new(StaticResolver { selection }),
        Box::new(StubToolExecutor),
        Box::new(PassthroughPlanner),
        Box::new(DefaultTurnContextBuilder),
        Box::new(DefaultTurnTelemetryBuilder),
    )
}

fn observe_hook_descriptor(
    name: &str,
    priority: i32,
    hook_point: TurnHookPoint,
) -> AgentHookDescriptor {
    AgentHookDescriptor {
        contract_version: "agent-hooks-v1".to_string(),
        name: name.to_string(),
        class: HookClass::Observe,
        priority,
        timeout_ms: 1_000,
        allowed_hook_points: vec![hook_point],
        allowed_result_kinds: vec![HookResultKind::Observe],
        can_block: false,
        default_failure_policy: HookFailurePolicy::Ignore,
        allowed_failure_policies: vec![HookFailurePolicy::Ignore],
        default_recovery_mode: HookRecoveryMode::ReplayRequired,
        trace_requirements: HookTraceRequirements {
            include_name: true,
            include_hook_point: true,
            include_elapsed_ms: true,
            include_result_summary: true,
        },
        replay_requirements: HookReplayRequirements {
            include_hook_order: true,
            include_input_summary: true,
        },
        side_effect_persistence_requirements: HookSideEffectPersistenceRequirements {
            require_persistence_evidence: false,
            require_effect_summary: false,
        },
    }
}

fn transform_hook_descriptor(
    name: &str,
    priority: i32,
    hook_point: TurnHookPoint,
) -> AgentHookDescriptor {
    AgentHookDescriptor {
        contract_version: "agent-hooks-v1".to_string(),
        name: name.to_string(),
        class: HookClass::Transform,
        priority,
        timeout_ms: 1_000,
        allowed_hook_points: vec![hook_point],
        allowed_result_kinds: vec![HookResultKind::Patch],
        can_block: false,
        default_failure_policy: HookFailurePolicy::Ignore,
        allowed_failure_policies: vec![HookFailurePolicy::Ignore, HookFailurePolicy::FailTurn],
        default_recovery_mode: HookRecoveryMode::ReplayRequired,
        trace_requirements: HookTraceRequirements {
            include_name: true,
            include_hook_point: true,
            include_elapsed_ms: true,
            include_result_summary: true,
        },
        replay_requirements: HookReplayRequirements {
            include_hook_order: true,
            include_input_summary: true,
        },
        side_effect_persistence_requirements: HookSideEffectPersistenceRequirements {
            require_persistence_evidence: false,
            require_effect_summary: false,
        },
    }
}

#[test]
fn capability_bridge_resolves_dotted_builtin_tool_calls_before_execution() {
    let runtime = build_runtime_for_test(test_provider_selection("http://localhost".to_string()));
    let execution = runtime.execute_capability_tool_call(&ToolCall {
        call_id: Some("call_time".to_string()),
        name: "time.now".to_string(),
        arguments: json!({}),
        plan: None,
    });

    assert_eq!(
        execution
            .capability
            .as_ref()
            .map(|capability| capability.capability_id.as_str()),
        Some("builtin:time_now")
    );
    assert_eq!(execution.tool_call.name, "time.now");
    assert_eq!(execution.tool_result.tool_name, "time.now");
    assert_eq!(execution.tool_result.status, "ok");
    assert_eq!(execution.failure_kind, None);
}

#[test]
fn runtime_hook_dispatch_returns_trace_records_in_priority_order() {
    let mut runtime =
        build_runtime_for_test(test_provider_selection("http://localhost".to_string()));
    runtime
        .register_hook_descriptor(observe_hook_descriptor(
            "observe.second",
            20,
            TurnHookPoint::ModelCallStart,
        ))
        .expect("register second hook");
    runtime
        .register_hook_descriptor(observe_hook_descriptor(
            "observe.first",
            10,
            TurnHookPoint::ModelCallStart,
        ))
        .expect("register first hook");

    let traces = runtime
        .dispatch_hook_trace_records(TurnHookPoint::ModelCallStart)
        .trace_records;

    assert_eq!(traces.len(), 2);
    assert_eq!(traces[0].hook_name, "observe.first");
    assert_eq!(traces[0].hook_order, 1);
    assert_eq!(traces[1].hook_name, "observe.second");
    assert_eq!(traces[1].hook_order, 2);
    assert!(traces
        .iter()
        .all(|trace| trace.hook_point == TurnHookPoint::ModelCallStart));
}

#[test]
fn runtime_hook_dispatch_returns_empty_for_unregistered_boundary() {
    let mut runtime =
        build_runtime_for_test(test_provider_selection("http://localhost".to_string()));
    runtime
        .register_hook_descriptor(observe_hook_descriptor(
            "observe.model",
            10,
            TurnHookPoint::ModelCallStart,
        ))
        .expect("register model hook");

    let traces = runtime
        .dispatch_hook_trace_records(TurnHookPoint::ToolCallEnd)
        .trace_records;

    assert!(traces.is_empty());
}

#[test]
fn runtime_hook_dispatch_records_executor_failure_without_stopping_turn_by_default() {
    let selection = test_provider_selection("http://localhost".to_string());
    let mut runtime = AgentRuntime::with_dependencies(
        SessionStore::memory_only(),
        Box::new(StaticResolver { selection }),
        Box::new(StubToolExecutor),
        Box::new(PassthroughPlanner),
        Box::new(DefaultTurnContextBuilder),
        Box::new(DefaultTurnTelemetryBuilder),
    );
    runtime.set_hook_executor_for_test(Box::new(FailingHookExecutor));
    runtime
        .register_hook_descriptor(observe_hook_descriptor(
            "observe.failure",
            10,
            TurnHookPoint::ModelCallStart,
        ))
        .expect("register failing hook");

    let outcome = runtime.dispatch_hook_trace_records(TurnHookPoint::ModelCallStart);
    let traces = outcome.trace_records;
    assert!(outcome.fail_turn_error.is_none());

    assert_eq!(traces.len(), 1);
    assert_eq!(traces[0].hook_name, "observe.failure");
    assert_eq!(traces[0].hook_order, 1);
    assert_eq!(traces[0].hook_point, TurnHookPoint::ModelCallStart);
    assert!(
        traces[0]
            .summary
            .contains("hook execution failed under ignore")
            || traces[0]
                .summary
                .contains("hook execution failed under Ignore")
    );
    assert!(traces[0]
        .input_summary
        .as_deref()
        .is_some_and(|summary| summary.contains("intentional hook failure")));
}

#[test]
fn runtime_hook_dispatch_records_degrade_failure_evidence_without_stopping_turn() {
    let selection = test_provider_selection("http://localhost".to_string());
    let mut runtime = AgentRuntime::with_dependencies(
        SessionStore::memory_only(),
        Box::new(StaticResolver { selection }),
        Box::new(StubToolExecutor),
        Box::new(PassthroughPlanner),
        Box::new(DefaultTurnContextBuilder),
        Box::new(DefaultTurnTelemetryBuilder),
    );
    runtime.set_hook_executor_for_test(Box::new(FailingHookExecutor));
    let mut descriptor =
        observe_hook_descriptor("observe.degrade-failure", 10, TurnHookPoint::ModelCallStart);
    descriptor.default_failure_policy = HookFailurePolicy::Degrade;
    descriptor.allowed_failure_policies =
        vec![HookFailurePolicy::Ignore, HookFailurePolicy::Degrade];
    runtime
        .register_hook_descriptor(descriptor)
        .expect("register degrade hook");

    let outcome = runtime.dispatch_hook_trace_records(TurnHookPoint::ModelCallStart);
    let traces = outcome.trace_records;
    assert!(outcome.fail_turn_error.is_none());

    assert_eq!(traces.len(), 1);
    assert_eq!(traces[0].hook_name, "observe.degrade-failure");
    assert!(traces[0]
        .summary
        .contains("hook execution failed under Degrade"));
}

#[test]
fn run_turn_records_planner_trace_records_in_terminal_trace() {
    let server = MockHttpServer::start(vec![json_completion("planner trace answer")]);
    let runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));

    let result = runtime.run_turn(TurnInput {
        message: "请总结当前状态".to_string(),
        display_message: None,
        provider_id: None,
        model_id: None,
        reasoning_effort: None,
        workspace_mode: None,
        session_id: Some("planner-trace-session".to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
    });

    let _ = server.finish();

    assert!(result
        .hook_trace_records
        .iter()
        .any(|record| record.hook_point == TurnHookPoint::PlannerTurnPreflight));
    assert!(result
        .hook_trace_records
        .iter()
        .any(|record| record.hook_point == TurnHookPoint::PlannerToolSelection));

    let snapshot = runtime.load_session_snapshot(Some("planner-trace-session"));
    let trace = snapshot
        .turn_trace_history
        .last()
        .expect("planner trace should persist");
    assert!(trace
        .hook_trace_records
        .iter()
        .any(|record| record.hook_name == "planner.preflight.observe"));
    assert!(trace
        .hook_trace_records
        .iter()
        .any(|record| record.hook_name == "planner.tool_selection.observe"));
}

#[test]
fn run_turn_records_capability_mediation_trace_for_forced_tool_planner() {
    let _rt_guard = crate::agent::runtime_helper::TestRuntimeGuard::new();
    let selection = test_chat_provider_selection("http://localhost".to_string());
    let runtime = AgentRuntime::with_dependencies(
        SessionStore::memory_only(),
        Box::new(StaticResolver { selection }),
        Box::new(StubToolExecutor),
        Box::new(ForcedToolPlanner {
            tool_name: "workspace_list_files".to_string(),
            arguments: json!({}),
        }),
        Box::new(DefaultTurnContextBuilder),
        Box::new(DefaultTurnTelemetryBuilder),
    );

    let result = runtime.run_turn(TurnInput {
        message: "列出当前目录".to_string(),
        display_message: None,
        provider_id: None,
        model_id: None,
        reasoning_effort: None,
        workspace_mode: None,
        session_id: Some("capability-trace-session".to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
    });

    assert!(result
        .hook_trace_records
        .iter()
        .any(|record| record.hook_point == TurnHookPoint::CapabilityResolve));

    let capability_trace = result
        .hook_trace_records
        .iter()
        .find(|record| record.hook_point == TurnHookPoint::CapabilityResolve)
        .expect("capability resolve trace should exist");
    assert!(capability_trace
        .summary
        .contains("capability mediation resolved"));

    let snapshot = runtime.load_session_snapshot(Some("capability-trace-session"));
    let trace = snapshot
        .turn_trace_history
        .last()
        .expect("capability trace should persist");
    assert!(trace
        .hook_trace_records
        .iter()
        .any(|record| record.hook_name == "capability.resolve.observe"));
}

#[test]
fn capability_mediation_hooks_can_rewrite_arguments_before_tool_execution() {
    let _rt_guard = crate::agent::runtime_helper::TestRuntimeGuard::new();
    let recorded_calls = Arc::new(Mutex::new(Vec::new()));
    let selection = test_chat_provider_selection("http://localhost".to_string());
    let mut runtime = AgentRuntime::with_dependencies(
        SessionStore::memory_only(),
        Box::new(StaticResolver { selection }),
        Box::new(RecordingToolExecutor {
            calls: Arc::clone(&recorded_calls),
        }),
        Box::new(ForcedToolPlanner {
            tool_name: "workspace_list_files".to_string(),
            arguments: json!({"path":"."}),
        }),
        Box::new(DefaultTurnContextBuilder),
        Box::new(DefaultTurnTelemetryBuilder),
    );
    runtime.set_hook_executor_for_test(Box::new(TransformingCapabilityHookExecutor));
    runtime
        .register_hook_descriptor(transform_hook_descriptor(
            "capability.rewrite",
            10,
            TurnHookPoint::CapabilityResolve,
        ))
        .expect("register capability transform hook");

    let result = runtime.run_turn(TurnInput {
        message: "列出当前目录".to_string(),
        display_message: None,
        provider_id: None,
        model_id: None,
        reasoning_effort: None,
        workspace_mode: None,
        session_id: Some("capability-hook-rewrite".to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
    });

    assert!(result
        .hook_trace_records
        .iter()
        .any(|record| record.hook_name == "capability.rewrite"));
    let calls = recorded_calls.lock().unwrap();
    let call = calls.last().expect("tool call should be recorded");
    assert_eq!(
        call.arguments.get("path").and_then(Value::as_str),
        Some("src-tauri")
    );

    let snapshot = runtime.load_session_snapshot(Some("capability-hook-rewrite"));
    let trace = snapshot
        .turn_trace_history
        .last()
        .expect("capability hook trace should persist");
    assert!(trace
        .hook_trace_records
        .iter()
        .any(|record| record.hook_name == "capability.rewrite"));
}

#[test]
fn planner_preflight_hooks_can_rewrite_tool_call_before_execution() {
    let _rt_guard = crate::agent::runtime_helper::TestRuntimeGuard::new();
    let recorded_calls = Arc::new(Mutex::new(Vec::new()));
    let selection = test_chat_provider_selection("http://localhost".to_string());
    let mut runtime = AgentRuntime::with_dependencies(
        SessionStore::memory_only(),
        Box::new(StaticResolver { selection }),
        Box::new(RecordingToolExecutor {
            calls: Arc::clone(&recorded_calls),
        }),
        Box::new(ForcedToolPlanner {
            tool_name: "workspace_list_files".to_string(),
            arguments: json!({"path":"."}),
        }),
        Box::new(DefaultTurnContextBuilder),
        Box::new(DefaultTurnTelemetryBuilder),
    );
    runtime.set_hook_executor_for_test(Box::new(TransformingPlannerHookExecutor));
    runtime
        .register_hook_descriptor(transform_hook_descriptor(
            "planner.preflight.rewrite",
            10,
            TurnHookPoint::PlannerTurnPreflight,
        ))
        .expect("register planner preflight transform hook");

    let result = runtime.run_turn(TurnInput {
        message: "列出当前目录".to_string(),
        display_message: None,
        provider_id: None,
        model_id: None,
        reasoning_effort: None,
        workspace_mode: None,
        session_id: Some("planner-preflight-rewrite".to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
    });

    assert!(result
        .hook_trace_records
        .iter()
        .any(|record| record.hook_name == "planner.preflight.rewrite"));
    let calls = recorded_calls.lock().unwrap();
    let call = calls
        .last()
        .expect("planner preflight tool call should execute");
    assert_eq!(
        call.arguments.get("path").and_then(Value::as_str),
        Some("src-tauri")
    );

    let snapshot = runtime.load_session_snapshot(Some("planner-preflight-rewrite"));
    let trace = snapshot
        .turn_trace_history
        .last()
        .expect("planner preflight hook trace should persist");
    assert!(trace
        .hook_trace_records
        .iter()
        .any(|record| record.hook_name == "planner.preflight.rewrite"));
}

#[test]
fn planner_tool_selection_hooks_can_rewrite_selected_tool_before_execution() {
    let _rt_guard = crate::agent::runtime_helper::TestRuntimeGuard::new();
    let recorded_calls = Arc::new(Mutex::new(Vec::new()));
    let selection = test_chat_provider_selection("http://localhost".to_string());
    let mut runtime = AgentRuntime::with_dependencies(
        SessionStore::memory_only(),
        Box::new(StaticResolver { selection }),
        Box::new(RecordingToolExecutor {
            calls: Arc::clone(&recorded_calls),
        }),
        Box::new(ForcedToolPlanner {
            tool_name: "workspace_list_files".to_string(),
            arguments: json!({"path":"."}),
        }),
        Box::new(DefaultTurnContextBuilder),
        Box::new(DefaultTurnTelemetryBuilder),
    );
    runtime.set_hook_executor_for_test(Box::new(TransformingPlannerHookExecutor));
    runtime
        .register_hook_descriptor(transform_hook_descriptor(
            "planner.tool_selection.rewrite",
            10,
            TurnHookPoint::PlannerToolSelection,
        ))
        .expect("register planner tool-selection transform hook");

    let result = runtime.run_turn(TurnInput {
        message: "列出当前目录".to_string(),
        display_message: None,
        provider_id: None,
        model_id: None,
        reasoning_effort: None,
        workspace_mode: None,
        session_id: Some("planner-tool-selection-rewrite".to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
    });

    assert!(result
        .hook_trace_records
        .iter()
        .any(|record| record.hook_name == "planner.tool_selection.rewrite"));
    let calls = recorded_calls.lock().unwrap();
    let call = calls
        .last()
        .expect("planner tool-selection call should execute");
    assert_eq!(
        call.arguments.get("path").and_then(Value::as_str),
        Some("tests")
    );

    let snapshot = runtime.load_session_snapshot(Some("planner-tool-selection-rewrite"));
    let trace = snapshot
        .turn_trace_history
        .last()
        .expect("planner tool-selection hook trace should persist");
    assert!(trace
        .hook_trace_records
        .iter()
        .any(|record| record.hook_name == "planner.tool_selection.rewrite"));
}

#[test]
fn skill_mediation_hooks_can_rewrite_arguments_before_skill_execution() {
    let _rt_guard = crate::agent::runtime_helper::TestRuntimeGuard::new();
    let recorded_calls = Arc::new(Mutex::new(Vec::new()));
    let selection = test_chat_provider_selection("http://localhost".to_string());
    let mut runtime = AgentRuntime::with_dependencies(
        SessionStore::memory_only(),
        Box::new(StaticResolver { selection }),
        Box::new(RecordingToolExecutor {
            calls: Arc::clone(&recorded_calls),
        }),
        Box::new(ForcedToolPlanner {
            tool_name: "echo_skill".to_string(),
            arguments: json!({"message":"original"}),
        }),
        Box::new(DefaultTurnContextBuilder),
        Box::new(DefaultTurnTelemetryBuilder),
    );
    runtime
        .apply_skill_source_snapshot(crate::agent::capability_bridge::SkillSourceSnapshot {
            source: crate::agent::capability_bridge::SkillSourceView {
                source_id: "builtin-skills".to_string(),
                source_kind: crate::agent::capability_bridge::SkillSourceKind::Host,
                display_name: "Builtin Skills".to_string(),
                availability: crate::agent::capability_bridge::CapabilityAvailability::Available,
                transport_kind: "host".to_string(),
                server_identity: "skills://builtin".to_string(),
                updated_at_ms: 1,
                last_ingress_observation: None,
            },
            skills: vec![crate::agent::capability_bridge::SkillDescriptor {
                skill_id: "skill:echo".to_string(),
                source_id: "builtin-skills".to_string(),
                source_kind: crate::agent::capability_bridge::SkillSourceKind::Host,
                label: "echo_skill".to_string(),
                description: "Echo message".to_string(),
                input_schema_summary: "{}".to_string(),
                safety_class: "".to_string(),
                visibility: "default".to_string(),
                observability_tags: vec![],
                requires_approval: false,
                host_mediated: false,
                permission_scope: "".to_string(),
                composed_capability_refs: vec!["builtin:echo_input".to_string()],
                composed_capability_kinds: vec![
                    crate::agent::capability_bridge::CapabilityKind::Tool,
                ],
                executable_in_v1: true,
            }],
        })
        .expect("skill snapshot should apply");
    runtime.set_hook_executor_for_test(Box::new(TransformingCapabilityHookExecutor));
    runtime
        .register_hook_descriptor(transform_hook_descriptor(
            "skill.rewrite",
            10,
            TurnHookPoint::SkillToolActionsResolve,
        ))
        .expect("register skill transform hook");

    let result = runtime.run_turn(TurnInput {
        message: "运行 skill".to_string(),
        display_message: None,
        provider_id: None,
        model_id: None,
        reasoning_effort: None,
        workspace_mode: None,
        session_id: Some("skill-hook-rewrite".to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
    });

    assert!(result
        .hook_trace_records
        .iter()
        .any(|record| record.hook_name == "skill.rewrite"));
    let calls = recorded_calls.lock().unwrap();
    let call = calls.last().expect("skill tool call should be recorded");
    assert_eq!(
        call.arguments.get("message").and_then(Value::as_str),
        Some("patched by hook")
    );

    let snapshot = runtime.load_session_snapshot(Some("skill-hook-rewrite"));
    let trace = snapshot
        .turn_trace_history
        .last()
        .expect("skill hook trace should persist");
    assert!(trace
        .hook_trace_records
        .iter()
        .any(|record| record.hook_name == "skill.rewrite"));
}

#[test]
fn start_turn_stream_does_not_dispatch_unstable_prepare_or_context_hooks() {
    let server = MockHttpServer::start(vec![sse_response(&[
        json!({
            "choices": [
                {
                    "delta": {
                        "content": "稳定边界答案。"
                    }
                }
            ]
        }),
        json!({
            "choices": [],
            "usage": {
                "prompt_tokens": 12,
                "completion_tokens": 4,
                "total_tokens": 16
            }
        }),
    ])]);
    let mut runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));
    runtime
        .register_hook_descriptor(observe_hook_descriptor(
            "observe.prepare-start",
            10,
            TurnHookPoint::TurnPrepareStart,
        ))
        .expect("register prepare start hook");
    runtime
        .register_hook_descriptor(observe_hook_descriptor(
            "observe.prepare-end",
            20,
            TurnHookPoint::TurnPrepareEnd,
        ))
        .expect("register prepare end hook");
    runtime
        .register_hook_descriptor(observe_hook_descriptor(
            "observe.context-start",
            30,
            TurnHookPoint::ContextBuildStart,
        ))
        .expect("register context start hook");
    runtime
        .register_hook_descriptor(observe_hook_descriptor(
            "observe.context-end",
            40,
            TurnHookPoint::ContextBuildEnd,
        ))
        .expect("register context end hook");
    runtime
        .register_hook_descriptor(observe_hook_descriptor(
            "observe.checkpoint-stable",
            50,
            TurnHookPoint::CheckpointPersistEnd,
        ))
        .expect("register checkpoint stable hook");
    let sink = RecordingTurnEventSink::new();

    runtime.start_turn_stream(
        &sink,
        "turn-unstable-boundary-not-dispatched".to_string(),
        TurnInput {
            message: "请给出稳定边界测试答案。".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            workspace_mode: None,
            session_id: Some("session-unstable-boundary-not-dispatched".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
        },
    );

    let _ = server.finish();

    let unstable_hook_names = [
        "observe.prepare-start",
        "observe.prepare-end",
        "observe.context-start",
        "observe.context-end",
    ];
    let event_hook_names = sink
        .events
        .borrow()
        .iter()
        .flat_map(|(_, payload)| payload.hook_trace_records.clone().unwrap_or_default())
        .map(|record| record.hook_name)
        .collect::<Vec<_>>();

    assert!(event_hook_names
        .iter()
        .any(|name| name == "observe.checkpoint-stable"));
    for unstable_hook_name in unstable_hook_names {
        assert!(
            !event_hook_names
                .iter()
                .any(|name| name == unstable_hook_name),
            "unstable hook `{unstable_hook_name}` should not be dispatched in streamed events"
        );
    }

    let snapshot = runtime.load_session_snapshot(Some("session-unstable-boundary-not-dispatched"));
    let trace = snapshot
        .turn_trace_history
        .last()
        .expect("persisted turn trace");
    assert!(trace
        .hook_trace_records
        .iter()
        .any(|record| record.hook_name == "observe.checkpoint-stable"));
    for unstable_hook_name in [
        "observe.prepare-start",
        "observe.prepare-end",
        "observe.context-start",
        "observe.context-end",
    ] {
        assert!(
            !trace
                .hook_trace_records
                .iter()
                .any(|record| record.hook_name == unstable_hook_name),
            "unstable hook `{unstable_hook_name}` should not leak into persisted traces"
        );
    }
}

#[test]
fn start_turn_stream_fail_turn_policy_emits_failed_terminal_with_hook_evidence() {
    let server = MockHttpServer::start(vec![]);
    let mut runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));
    runtime.set_hook_executor_for_test(Box::new(FailingHookExecutor));
    let mut descriptor =
        observe_hook_descriptor("observe.fail-turn", 10, TurnHookPoint::ModelCallStart);
    descriptor.default_failure_policy = HookFailurePolicy::FailTurn;
    descriptor.allowed_failure_policies =
        vec![HookFailurePolicy::Degrade, HookFailurePolicy::FailTurn];
    runtime
        .register_hook_descriptor(descriptor)
        .expect("register fail-turn hook");
    let sink = RecordingTurnEventSink::new();

    runtime.start_turn_stream(
        &sink,
        "turn-hook-failturn".to_string(),
        TurnInput {
            message: "请尝试开始一个会被 hook failturn 阻断的 turn".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            workspace_mode: None,
            session_id: Some("session-hook-failturn".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
        },
    );

    let request_bodies = server.finish();
    assert!(request_bodies.is_empty());

    let events = sink.events.borrow();
    let trace_event = events
        .iter()
        .find_map(|(name, payload)| (name == "turn:trace").then_some(payload.clone()))
        .expect("trace event");
    let failed_event = events
        .iter()
        .find_map(|(name, payload)| (name == "turn:failed").then_some(payload.clone()))
        .expect("failed event");

    assert_eq!(trace_event.phase.as_deref(), Some("calling_model"));
    assert_eq!(failed_event.phase.as_deref(), Some("failed"));
    assert!(failed_event
        .error
        .as_deref()
        .is_some_and(|error| error.contains("observe.fail-turn")));
    assert_eq!(
        trace_event
            .hook_trace_records
            .as_ref()
            .map(|records| records.len()),
        Some(1)
    );
    assert_eq!(
        failed_event
            .hook_trace_records
            .as_ref()
            .map(|records| records.len()),
        Some(1)
    );
    assert!(failed_event
        .hook_trace_records
        .as_ref()
        .and_then(|records| records.first())
        .is_some_and(|record| record.blocked));
    assert_hook_boundary_alignment(
        &failed_event,
        TurnHookPoint::TurnFinalizeEnd,
        "turn.failed",
        "failed",
    );

    let snapshot = runtime.load_session_snapshot(Some("session-hook-failturn"));
    let trace = snapshot
        .turn_trace_history
        .last()
        .expect("persisted failed trace");
    assert_eq!(trace.phase, "failed");
    assert!(trace
        .error
        .as_deref()
        .is_some_and(|error| error.contains("observe.fail-turn")));
    assert_eq!(trace.hook_trace_records.len(), 1);
    assert!(trace.hook_trace_records[0].blocked);
}

#[test]
fn start_turn_stream_fail_turn_policy_on_tool_call_start_stops_before_tool_execution() {
    let server = MockHttpServer::start(vec![
        json_response(decision_tool_call(
            "workspace_list_files",
            json!({"path": ".", "limit": 40}),
        )),
        json_response(decision_tool_call(
            "workspace_list_files",
            json!({"path": ".", "limit": 40}),
        )),
    ]);
    let tool_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut runtime = build_runtime_for_test_with_tool_executor(
        test_provider_selection(server.base_url.clone()),
        Box::new(CountingToolExecutor {
            calls: Arc::clone(&tool_calls),
        }),
    );
    runtime.set_hook_executor_for_test(Box::new(FailingHookExecutor));
    let mut descriptor = observe_hook_descriptor(
        "observe.tool-start-failturn",
        10,
        TurnHookPoint::ToolCallStart,
    );
    descriptor.default_failure_policy = HookFailurePolicy::FailTurn;
    descriptor.allowed_failure_policies =
        vec![HookFailurePolicy::Degrade, HookFailurePolicy::FailTurn];
    runtime
        .register_hook_descriptor(descriptor)
        .expect("register tool-start failturn hook");
    let sink = RecordingTurnEventSink::new();

    runtime.start_turn_stream(
        &sink,
        "turn-tool-start-failturn".to_string(),
        TurnInput {
            message: "请先列出文件。".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            workspace_mode: None,
            session_id: Some("session-tool-start-failturn".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
        },
    );

    let request_bodies = server.finish();
    assert!((1..=2).contains(&request_bodies.len()));
    assert_eq!(tool_calls.load(AtomicOrdering::SeqCst), 0);

    let events = sink.events.borrow();
    let tool_started = events
        .iter()
        .find_map(|(name, payload)| {
            (name == "turn:tool" && payload.event_type.as_deref() == Some("turn.tool_call_started"))
                .then_some(payload.clone())
        })
        .expect("tool started event");
    let failed = events
        .iter()
        .find_map(|(name, payload)| (name == "turn:failed").then_some(payload.clone()))
        .expect("failed event");

    assert_eq!(
        tool_started
            .hook_trace_records
            .as_ref()
            .map(|records| records.len()),
        Some(1)
    );
    assert_eq!(
        failed
            .hook_trace_records
            .as_ref()
            .map(|records| records.len()),
        Some(1)
    );
    assert!(failed
        .error
        .as_deref()
        .is_some_and(|error| error.contains("observe.tool-start-failturn")));

    let snapshot = runtime.load_session_snapshot(Some("session-tool-start-failturn"));
    let trace = snapshot
        .turn_trace_history
        .last()
        .expect("persisted failed trace");
    assert_eq!(trace.phase, "failed");
    assert_eq!(trace.hook_trace_records.len(), 1);
    assert!(trace.hook_trace_records[0].blocked);
}

#[test]
fn start_turn_stream_fail_turn_policy_on_tool_call_end_stops_before_followup_model_call() {
    let server = MockHttpServer::start(vec![
        json_response(decision_tool_call(
            "workspace_list_files",
            json!({"path": ".", "limit": 40}),
        )),
        json_response(decision_tool_call(
            "workspace_list_files",
            json!({"path": ".", "limit": 40}),
        )),
    ]);
    let tool_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut runtime = build_runtime_for_test_with_tool_executor(
        test_provider_selection(server.base_url.clone()),
        Box::new(CountingToolExecutor {
            calls: Arc::clone(&tool_calls),
        }),
    );
    runtime.set_hook_executor_for_test(Box::new(FailingHookExecutor));
    let mut descriptor =
        observe_hook_descriptor("observe.tool-end-failturn", 10, TurnHookPoint::ToolCallEnd);
    descriptor.default_failure_policy = HookFailurePolicy::FailTurn;
    descriptor.allowed_failure_policies =
        vec![HookFailurePolicy::Degrade, HookFailurePolicy::FailTurn];
    runtime
        .register_hook_descriptor(descriptor)
        .expect("register tool-end failturn hook");
    let sink = RecordingTurnEventSink::new();

    runtime.start_turn_stream(
        &sink,
        "turn-tool-end-failturn".to_string(),
        TurnInput {
            message: "请先列出文件。".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            workspace_mode: None,
            session_id: Some("session-tool-end-failturn".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
        },
    );

    let request_bodies = server.finish();
    assert!((1..=2).contains(&request_bodies.len()));
    assert_eq!(tool_calls.load(AtomicOrdering::SeqCst), 1);

    let events = sink.events.borrow();
    let tool_completed = events
        .iter()
        .find_map(|(name, payload)| {
            (name == "turn:tool"
                && payload.event_type.as_deref() == Some("turn.tool_call_completed"))
            .then_some(payload.clone())
        })
        .expect("tool completed event");
    let failed = events
        .iter()
        .find_map(|(name, payload)| (name == "turn:failed").then_some(payload.clone()))
        .expect("failed event");

    assert_eq!(
        tool_completed
            .hook_trace_records
            .as_ref()
            .map(|records| records.len()),
        Some(1)
    );
    assert_eq!(
        failed
            .hook_trace_records
            .as_ref()
            .map(|records| records.len()),
        Some(1)
    );
    assert!(failed
        .error
        .as_deref()
        .is_some_and(|error| error.contains("observe.tool-end-failturn")));

    let snapshot = runtime.load_session_snapshot(Some("session-tool-end-failturn"));
    let trace = snapshot
        .turn_trace_history
        .last()
        .expect("persisted failed trace");
    assert_eq!(trace.phase, "failed");
    assert_eq!(trace.hook_trace_records.len(), 1);
    assert!(trace.hook_trace_records[0].blocked);
}

#[test]
fn start_turn_stream_fail_turn_policy_on_checkpoint_boundary_emits_failed_instead_of_completed() {
    let server = MockHttpServer::start(vec![sse_response(&[
        json!({
            "choices": [
                {
                    "delta": {
                        "content": "checkpoint failturn answer"
                    }
                }
            ]
        }),
        json!({
            "choices": [],
            "usage": {
                "prompt_tokens": 12,
                "completion_tokens": 4,
                "total_tokens": 16
            }
        }),
    ])]);
    let mut runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));
    runtime.set_hook_executor_for_test(Box::new(FailingHookExecutor));
    let mut descriptor = observe_hook_descriptor(
        "observe.checkpoint-failturn",
        10,
        TurnHookPoint::CheckpointPersistEnd,
    );
    descriptor.default_failure_policy = HookFailurePolicy::FailTurn;
    descriptor.allowed_failure_policies =
        vec![HookFailurePolicy::Degrade, HookFailurePolicy::FailTurn];
    runtime
        .register_hook_descriptor(descriptor)
        .expect("register checkpoint failturn hook");
    let sink = RecordingTurnEventSink::new();

    runtime.start_turn_stream(
        &sink,
        "turn-checkpoint-failturn".to_string(),
        TurnInput {
            message: "请回答一个会在 checkpoint boundary failturn 的问题".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            workspace_mode: None,
            session_id: Some("session-checkpoint-failturn".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
        },
    );

    let _ = server.finish();
    let events = sink.events.borrow();
    assert!(!events.iter().any(|(name, _)| name == "turn:completed"));
    let checkpoint = events
        .iter()
        .find_map(|(name, payload)| {
            (name == "turn:checkpoint_persisted").then_some(payload.clone())
        })
        .expect("checkpoint persisted event");
    let failed = events
        .iter()
        .find_map(|(name, payload)| (name == "turn:failed").then_some(payload.clone()))
        .expect("failed event");

    assert!(checkpoint
        .hook_trace_records
        .as_ref()
        .is_some_and(|records| {
            records.iter().any(|record| {
                record.hook_name == "observe.checkpoint-failturn"
                    && record.hook_point == TurnHookPoint::CheckpointPersistEnd
                    && record.blocked
            })
        }));
    assert!(failed.hook_trace_records.as_ref().is_some_and(|records| {
        records.iter().any(|record| {
            record.hook_name == "observe.checkpoint-failturn"
                && record.hook_point == TurnHookPoint::CheckpointPersistEnd
                && record.blocked
        })
    }));
    assert!(failed
        .error
        .as_deref()
        .is_some_and(|error| error.contains("observe.checkpoint-failturn")));

    let snapshot = runtime.load_session_snapshot(Some("session-checkpoint-failturn"));
    let trace = snapshot
        .turn_trace_history
        .last()
        .expect("persisted failed trace");
    assert_eq!(trace.phase, "failed");
    assert!(trace.hook_trace_records.iter().any(|record| {
        record.hook_name == "observe.checkpoint-failturn"
            && record.hook_point == TurnHookPoint::CheckpointPersistEnd
            && record.blocked
    }));
}

#[test]
fn start_turn_stream_fail_turn_policy_on_finalize_boundary_emits_failed_with_terminal_hook_evidence(
) {
    let server = MockHttpServer::start(vec![sse_response(&[
        json!({
            "choices": [
                {
                    "delta": {
                        "content": "finalize failturn answer"
                    }
                }
            ]
        }),
        json!({
            "choices": [],
            "usage": {
                "prompt_tokens": 12,
                "completion_tokens": 4,
                "total_tokens": 16
            }
        }),
    ])]);
    let mut runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));
    runtime.set_hook_executor_for_test(Box::new(FailingHookExecutor));
    runtime
        .register_hook_descriptor(observe_hook_descriptor(
            "observe.checkpoint-ok",
            10,
            TurnHookPoint::CheckpointPersistEnd,
        ))
        .expect("register checkpoint observe hook");
    let mut descriptor = observe_hook_descriptor(
        "observe.finalize-failturn",
        20,
        TurnHookPoint::TurnFinalizeEnd,
    );
    descriptor.default_failure_policy = HookFailurePolicy::FailTurn;
    descriptor.allowed_failure_policies =
        vec![HookFailurePolicy::Degrade, HookFailurePolicy::FailTurn];
    runtime
        .register_hook_descriptor(descriptor)
        .expect("register finalize failturn hook");
    let sink = RecordingTurnEventSink::new();

    runtime.start_turn_stream(
        &sink,
        "turn-finalize-failturn".to_string(),
        TurnInput {
            message: "请回答一个会在 finalize boundary failturn 的问题".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            workspace_mode: None,
            session_id: Some("session-finalize-failturn".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
        },
    );

    let _ = server.finish();
    let events = sink.events.borrow();
    assert!(!events.iter().any(|(name, _)| name == "turn:completed"));
    let checkpoint = events
        .iter()
        .find_map(|(name, payload)| {
            (name == "turn:checkpoint_persisted").then_some(payload.clone())
        })
        .expect("checkpoint persisted event");
    let failed = events
        .iter()
        .find_map(|(name, payload)| (name == "turn:failed").then_some(payload.clone()))
        .expect("failed event");

    assert!(checkpoint
        .hook_trace_records
        .as_ref()
        .is_some_and(|records| {
            records.iter().any(|record| {
                record.hook_name == "observe.checkpoint-ok"
                    && record.hook_point == TurnHookPoint::CheckpointPersistEnd
            })
        }));
    assert!(failed
        .hook_trace_records
        .as_ref()
        .is_some_and(|records| records
            .iter()
            .any(|record| { record.hook_name == "observe.finalize-failturn" && record.blocked })));

    let snapshot = runtime.load_session_snapshot(Some("session-finalize-failturn"));
    let trace = snapshot
        .turn_trace_history
        .last()
        .expect("persisted failed trace");
    assert_eq!(trace.phase, "failed");
    assert!(trace
        .hook_trace_records
        .iter()
        .any(|record| record.hook_name == "observe.finalize-failturn" && record.blocked));
}

#[test]
fn run_turn_fail_turn_policy_on_model_call_start_returns_failed_result_with_hook_evidence() {
    let server = MockHttpServer::start(vec![]);
    let mut runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));
    runtime.set_hook_executor_for_test(Box::new(FailingHookExecutor));
    let mut descriptor = observe_hook_descriptor(
        "observe.sync-model-failturn",
        10,
        TurnHookPoint::ModelCallStart,
    );
    descriptor.default_failure_policy = HookFailurePolicy::FailTurn;
    descriptor.allowed_failure_policies =
        vec![HookFailurePolicy::Degrade, HookFailurePolicy::FailTurn];
    runtime
        .register_hook_descriptor(descriptor)
        .expect("register sync model failturn hook");

    let result = runtime.run_turn(TurnInput {
        message: "请尝试一个同步 failturn turn".to_string(),
        display_message: None,
        provider_id: None,
        model_id: None,
        reasoning_effort: None,
        workspace_mode: None,
        session_id: Some("sync-model-failturn".to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
    });

    let request_bodies = server.finish();
    assert!(request_bodies.is_empty());
    assert_eq!(result.phase, "failed");
    assert_eq!(result.hook_trace_records.len(), 1);
    assert!(result.hook_trace_records[0].blocked);
    assert!(result
        .assistant_message
        .contains("observe.sync-model-failturn"));
}

#[test]
fn run_turn_fail_turn_policy_on_tool_call_start_returns_failed_before_tool_execution() {
    let server = MockHttpServer::start(vec![json_response(decision_tool_call(
        "workspace_list_files",
        json!({"path": ".", "limit": 40}),
    ))]);
    let tool_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut runtime = build_runtime_for_test_with_tool_executor(
        test_provider_selection(server.base_url.clone()),
        Box::new(CountingToolExecutor {
            calls: Arc::clone(&tool_calls),
        }),
    );
    runtime.set_hook_executor_for_test(Box::new(FailingHookExecutor));
    let mut descriptor = observe_hook_descriptor(
        "observe.sync-tool-start-failturn",
        10,
        TurnHookPoint::ToolCallStart,
    );
    descriptor.default_failure_policy = HookFailurePolicy::FailTurn;
    descriptor.allowed_failure_policies =
        vec![HookFailurePolicy::Degrade, HookFailurePolicy::FailTurn];
    runtime
        .register_hook_descriptor(descriptor)
        .expect("register sync tool-start failturn hook");

    let result = runtime.run_turn(TurnInput {
        message: "请先列出文件。".to_string(),
        display_message: None,
        provider_id: None,
        model_id: None,
        reasoning_effort: None,
        workspace_mode: None,
        session_id: Some("sync-tool-start-failturn".to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
    });

    let request_bodies = server.finish();
    assert_eq!(request_bodies.len(), 1);
    assert_eq!(tool_calls.load(AtomicOrdering::SeqCst), 0);
    assert_eq!(result.phase, "failed");
    assert!(result
        .assistant_message
        .contains("observe.sync-tool-start-failturn"));
    assert!(result
        .hook_trace_records
        .iter()
        .any(|record| record.hook_name == "observe.sync-tool-start-failturn" && record.blocked));
}

#[test]
fn run_turn_fail_turn_policy_on_tool_call_end_returns_failed_before_followup_model_call() {
    let server = MockHttpServer::start(vec![json_response(decision_tool_call(
        "workspace_list_files",
        json!({"path": ".", "limit": 40}),
    ))]);
    let tool_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut runtime = build_runtime_for_test_with_tool_executor(
        test_provider_selection(server.base_url.clone()),
        Box::new(CountingToolExecutor {
            calls: Arc::clone(&tool_calls),
        }),
    );
    runtime.set_hook_executor_for_test(Box::new(FailingHookExecutor));
    let mut descriptor = observe_hook_descriptor(
        "observe.sync-tool-end-failturn",
        10,
        TurnHookPoint::ToolCallEnd,
    );
    descriptor.default_failure_policy = HookFailurePolicy::FailTurn;
    descriptor.allowed_failure_policies =
        vec![HookFailurePolicy::Degrade, HookFailurePolicy::FailTurn];
    runtime
        .register_hook_descriptor(descriptor)
        .expect("register sync tool-end failturn hook");

    let result = runtime.run_turn(TurnInput {
        message: "请先列出文件。".to_string(),
        display_message: None,
        provider_id: None,
        model_id: None,
        reasoning_effort: None,
        workspace_mode: None,
        session_id: Some("sync-tool-end-failturn".to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
    });

    let request_bodies = server.finish();
    assert_eq!(request_bodies.len(), 1);
    assert_eq!(tool_calls.load(AtomicOrdering::SeqCst), 1);
    assert_eq!(result.phase, "failed");
    assert!(result
        .assistant_message
        .contains("observe.sync-tool-end-failturn"));
    assert!(result
        .hook_trace_records
        .iter()
        .any(|record| record.hook_name == "observe.sync-tool-end-failturn" && record.blocked));
}

#[test]
fn run_turn_persists_terminal_hook_traces_on_completed_sync_turn() {
    let server = MockHttpServer::start(vec![json_response(json!({
        "choices": [
            {
                "message": {
                    "role": "assistant",
                    "content": "同步完成答案。"
                }
            }
        ],
        "usage": {
            "prompt_tokens": 16,
            "completion_tokens": 5,
            "total_tokens": 21
        }
    }))]);
    let mut runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));
    runtime
        .register_hook_descriptor(observe_hook_descriptor(
            "observe.sync-checkpoint",
            10,
            TurnHookPoint::CheckpointPersistEnd,
        ))
        .expect("register sync checkpoint hook");
    runtime
        .register_hook_descriptor(observe_hook_descriptor(
            "observe.sync-finalize",
            20,
            TurnHookPoint::TurnFinalizeEnd,
        ))
        .expect("register sync finalize hook");

    let result = runtime.run_turn(TurnInput {
        message: "请直接同步回答。".to_string(),
        display_message: None,
        provider_id: None,
        model_id: None,
        reasoning_effort: None,
        workspace_mode: None,
        session_id: Some("sync-hook-trace-terminal".to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
    });

    let request_bodies = server.finish();
    assert_eq!(request_bodies.len(), 1);
    assert_eq!(result.phase, "ready");
    assert!(result
        .event_id
        .as_deref()
        .is_some_and(|value| value.starts_with("sync:sync-hook-trace-terminal:")));
    assert_eq!(result.event_type.as_deref(), Some("turn.completed"));
    assert_eq!(result.event_version.as_deref(), Some("turn-event-v1"));
    // PA-095：同步入口事件化后终态信封取自真实发射（turn:started=seq1、
    // 终态=seq2），不再单独分配。
    assert_eq!(result.sequence, Some(2));
    assert!(result.emitted_at_ms.is_some());
    assert!(result
        .hook_trace_records
        .iter()
        .any(|record| record.hook_name == "observe.sync-checkpoint"
            && record.hook_point == TurnHookPoint::CheckpointPersistEnd));
    assert!(result
        .hook_trace_records
        .iter()
        .any(|record| record.hook_name == "observe.sync-finalize"
            && record.hook_point == TurnHookPoint::TurnFinalizeEnd));

    let snapshot = runtime.load_session_snapshot(Some("sync-hook-trace-terminal"));
    let trace = snapshot
        .turn_trace_history
        .last()
        .expect("persisted sync trace");
    assert_eq!(trace.phase, "completed");
    assert!(trace
        .event_id
        .as_deref()
        .is_some_and(|value| value.starts_with("sync:sync-hook-trace-terminal:")));
    assert_eq!(trace.event_type.as_deref(), Some("turn.completed"));
    assert_eq!(trace.event_version.as_deref(), Some("turn-event-v1"));
    // PA-095：trace 信封与事件流同源（终态=seq2）。
    assert_eq!(trace.sequence, Some(2));
    assert!(trace.emitted_at_ms.is_some());
    assert!(trace
        .hook_trace_records
        .iter()
        .any(|record| record.hook_name == "observe.sync-checkpoint"
            && record.hook_point == TurnHookPoint::CheckpointPersistEnd));
    assert!(trace
        .hook_trace_records
        .iter()
        .any(|record| record.hook_name == "observe.sync-finalize"
            && record.hook_point == TurnHookPoint::TurnFinalizeEnd));
}

#[test]
fn run_turn_fail_turn_policy_on_checkpoint_boundary_persists_failed_sync_trace() {
    let server = MockHttpServer::start(vec![json_response(json!({
        "choices": [
            {
                "message": {
                    "role": "assistant",
                    "content": "checkpoint failturn answer"
                }
            }
        ]
    }))]);
    let mut runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));
    runtime.set_hook_executor_for_test(Box::new(FailingHookExecutor));
    let mut descriptor = observe_hook_descriptor(
        "observe.sync-checkpoint-failturn",
        10,
        TurnHookPoint::CheckpointPersistEnd,
    );
    descriptor.default_failure_policy = HookFailurePolicy::FailTurn;
    descriptor.allowed_failure_policies =
        vec![HookFailurePolicy::Degrade, HookFailurePolicy::FailTurn];
    runtime
        .register_hook_descriptor(descriptor)
        .expect("register sync checkpoint failturn hook");

    let result = runtime.run_turn(TurnInput {
        message: "请回答一个会在 sync checkpoint failturn 的问题".to_string(),
        display_message: None,
        provider_id: None,
        model_id: None,
        reasoning_effort: None,
        workspace_mode: None,
        session_id: Some("sync-checkpoint-failturn".to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
    });

    let request_bodies = server.finish();
    assert_eq!(request_bodies.len(), 1);
    assert_eq!(result.phase, "failed");
    assert!(result
        .event_id
        .as_deref()
        .is_some_and(|value| value.starts_with("sync:sync-checkpoint-failturn:")));
    assert_eq!(result.event_type.as_deref(), Some("turn.failed"));
    assert_eq!(result.event_version.as_deref(), Some("turn-event-v1"));
    // PA-095：终态信封取自真实发射（started=seq1、failed=seq2）。
    assert_eq!(result.sequence, Some(2));
    assert!(result.emitted_at_ms.is_some());
    assert!(result.hook_trace_records.iter().any(|record| {
        record.hook_name == "observe.sync-checkpoint-failturn"
            && record.hook_point == TurnHookPoint::CheckpointPersistEnd
            && record.blocked
    }));
    assert!(result
        .assistant_message
        .contains("observe.sync-checkpoint-failturn"));

    let snapshot = runtime.load_session_snapshot(Some("sync-checkpoint-failturn"));
    let trace = snapshot
        .turn_trace_history
        .last()
        .expect("persisted failed sync trace");
    assert_eq!(trace.phase, "failed");
    assert!(trace
        .event_id
        .as_deref()
        .is_some_and(|value| value.starts_with("sync:sync-checkpoint-failturn:")));
    assert_eq!(trace.event_type.as_deref(), Some("turn.failed"));
    assert_eq!(trace.event_version.as_deref(), Some("turn-event-v1"));
    // PA-095：trace 信封与事件流同源（终态=seq2）。
    assert_eq!(trace.sequence, Some(2));
    assert!(trace.emitted_at_ms.is_some());
    assert!(trace.hook_trace_records.iter().any(|record| {
        record.hook_name == "observe.sync-checkpoint-failturn"
            && record.hook_point == TurnHookPoint::CheckpointPersistEnd
            && record.blocked
    }));
    assert!(trace
        .error
        .as_deref()
        .is_some_and(|error| error.contains("observe.sync-checkpoint-failturn")));
}

#[test]
fn run_turn_fail_turn_policy_on_finalize_boundary_persists_terminal_sync_hook_evidence() {
    let server = MockHttpServer::start(vec![json_response(json!({
        "choices": [
            {
                "message": {
                    "role": "assistant",
                    "content": "finalize failturn answer"
                }
            }
        ]
    }))]);
    let mut runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));
    runtime.set_hook_executor_for_test(Box::new(FailingHookExecutor));
    runtime
        .register_hook_descriptor(observe_hook_descriptor(
            "observe.sync-checkpoint-ok",
            10,
            TurnHookPoint::CheckpointPersistEnd,
        ))
        .expect("register sync checkpoint observe hook");
    let mut descriptor = observe_hook_descriptor(
        "observe.sync-finalize-failturn",
        20,
        TurnHookPoint::TurnFinalizeEnd,
    );
    descriptor.default_failure_policy = HookFailurePolicy::FailTurn;
    descriptor.allowed_failure_policies =
        vec![HookFailurePolicy::Degrade, HookFailurePolicy::FailTurn];
    runtime
        .register_hook_descriptor(descriptor)
        .expect("register sync finalize failturn hook");

    let result = runtime.run_turn(TurnInput {
        message: "请回答一个会在 sync finalize failturn 的问题".to_string(),
        display_message: None,
        provider_id: None,
        model_id: None,
        reasoning_effort: None,
        workspace_mode: None,
        session_id: Some("sync-finalize-failturn".to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
    });

    let request_bodies = server.finish();
    assert_eq!(request_bodies.len(), 1);
    assert_eq!(result.phase, "failed");
    assert!(result
        .event_id
        .as_deref()
        .is_some_and(|value| value.starts_with("sync:sync-finalize-failturn:")));
    assert_eq!(result.event_type.as_deref(), Some("turn.failed"));
    assert_eq!(result.event_version.as_deref(), Some("turn-event-v1"));
    // PA-095：终态信封取自真实发射（started=seq1、failed=seq2）。
    assert_eq!(result.sequence, Some(2));
    assert!(result.emitted_at_ms.is_some());
    assert!(result
        .hook_trace_records
        .iter()
        .any(|record| record.hook_name == "observe.sync-checkpoint-ok"
            && record.hook_point == TurnHookPoint::CheckpointPersistEnd));
    assert!(result
        .hook_trace_records
        .iter()
        .any(
            |record| record.hook_name == "observe.sync-finalize-failturn"
                && record.hook_point == TurnHookPoint::TurnFinalizeEnd
                && record.blocked
        ));

    let snapshot = runtime.load_session_snapshot(Some("sync-finalize-failturn"));
    let trace = snapshot
        .turn_trace_history
        .last()
        .expect("persisted failed sync trace");
    assert_eq!(trace.phase, "failed");
    assert!(trace
        .event_id
        .as_deref()
        .is_some_and(|value| value.starts_with("sync:sync-finalize-failturn:")));
    assert_eq!(trace.event_type.as_deref(), Some("turn.failed"));
    assert_eq!(trace.event_version.as_deref(), Some("turn-event-v1"));
    // PA-095：trace 信封与事件流同源（终态=seq2）。
    assert_eq!(trace.sequence, Some(2));
    assert!(trace.emitted_at_ms.is_some());
    assert!(trace
        .hook_trace_records
        .iter()
        .any(|record| record.hook_name == "observe.sync-finalize-failturn" && record.blocked));
}

#[test]
fn start_turn_stream_persists_terminal_hook_traces_on_stable_boundaries() {
    let server = MockHttpServer::start(vec![sse_response(&[
        json!({
            "choices": [
                {
                    "delta": {
                        "reasoning_content": "先想一下。"
                    }
                }
            ]
        }),
        json!({
            "choices": [
                {
                    "delta": {
                        "content": "最终答案。"
                    }
                }
            ]
        }),
        json!({
            "choices": [],
            "usage": {
                "prompt_tokens": 20,
                "completion_tokens": 6,
                "total_tokens": 26
            }
        }),
    ])]);
    let mut runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));
    runtime
        .register_hook_descriptor(observe_hook_descriptor(
            "observe.checkpoint",
            10,
            TurnHookPoint::CheckpointPersistEnd,
        ))
        .expect("register checkpoint hook");
    runtime
        .register_hook_descriptor(observe_hook_descriptor(
            "observe.finalize",
            20,
            TurnHookPoint::TurnFinalizeEnd,
        ))
        .expect("register finalize hook");
    let sink = RecordingTurnEventSink::new();

    runtime.start_turn_stream(
        &sink,
        "turn-hook-trace-terminal".to_string(),
        TurnInput {
            message: "请直接回答。".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            workspace_mode: None,
            session_id: Some("stream-hook-trace-terminal".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
        },
    );

    let _ = server.finish();
    {
        let events = sink.events.borrow();
        let checkpoint_persisted = events
            .iter()
            .find_map(|(name, payload)| {
                (name == "turn:checkpoint_persisted"
                    && payload.event_type.as_deref() == Some("turn.checkpoint_persisted"))
                .then_some(payload.clone())
            })
            .expect("checkpoint persisted event");
        let completed = events
            .iter()
            .find_map(|(name, payload)| {
                (name == "turn:completed"
                    && payload.event_type.as_deref() == Some("turn.completed"))
                .then_some(payload.clone())
            })
            .expect("completed event");

        let checkpoint_records = checkpoint_persisted
            .hook_trace_records
            .clone()
            .expect("checkpoint hook traces");
        assert_eq!(checkpoint_records.len(), 1);
        assert_eq!(checkpoint_records[0].hook_name, "observe.checkpoint");
        assert_eq!(
            checkpoint_records[0].hook_point,
            TurnHookPoint::CheckpointPersistEnd
        );

        let completed_records = completed
            .hook_trace_records
            .clone()
            .expect("completed hook traces");
        assert!(completed_records
            .iter()
            .any(|record| record.hook_name == "observe.checkpoint"
                && record.hook_point == TurnHookPoint::CheckpointPersistEnd));
        assert!(completed_records
            .iter()
            .any(|record| record.hook_name == "observe.finalize"
                && record.hook_point == TurnHookPoint::TurnFinalizeEnd));
    }

    let snapshot = runtime.load_session_snapshot(Some("stream-hook-trace-terminal"));
    let trace = snapshot
        .turn_trace_history
        .last()
        .expect("persisted turn trace");
    assert!(trace
        .hook_trace_records
        .iter()
        .any(|record| record.hook_name == "observe.checkpoint"
            && record.hook_point == TurnHookPoint::CheckpointPersistEnd));
    assert!(trace
        .hook_trace_records
        .iter()
        .any(|record| record.hook_name == "observe.finalize"
            && record.hook_point == TurnHookPoint::TurnFinalizeEnd));
}

#[test]
fn capability_bridge_returns_normalized_not_found_failure_for_unknown_tools() {
    let runtime = build_runtime_for_test(test_provider_selection("http://localhost".to_string()));
    let execution = runtime.execute_capability_tool_call(&ToolCall {
        call_id: Some("call_unknown".to_string()),
        name: "unknown_tool".to_string(),
        arguments: json!({ "path": "src" }),
        plan: None,
    });

    assert!(execution.capability.is_none());
    assert_eq!(execution.tool_result.status, "error");
    assert_eq!(
        execution.failure_kind,
        Some(CapabilityFailureKind::CapabilityNotFound)
    );
    assert!(execution.tool_result.output.contains("capability registry"));
}

#[test]
fn capability_bridge_resolves_host_registered_mcp_tool_snapshot() {
    let mut runtime =
        build_runtime_for_test(test_provider_selection("http://localhost".to_string()));
    runtime.apply_mcp_source_snapshot(crate::agent::capability_bridge::McpSourceSnapshot {
        source: crate::agent::capability_bridge::CapabilitySourceView {
            source_id: "mcp-local".to_string(),
            source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
            display_name: "Local MCP".to_string(),
            transport_kind: "stdio".to_string(),
            server_identity: "mcp://local".to_string(),
            availability: crate::agent::capability_bridge::CapabilityAvailability::Available,
            declared_capabilities: vec![crate::agent::capability_bridge::CapabilityKind::Tool],
            permission_profile: "host-mediated".to_string(),
            updated_at_ms: 1,
            last_ingress_observation: None,
        },
        capabilities: vec![crate::agent::capability_bridge::CapabilityView {
            capability_id: "mcp:tool:workspace_search".to_string(),
            source_id: "mcp-local".to_string(),
            source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
            kind: crate::agent::capability_bridge::CapabilityKind::Tool,
            label: "workspace_search".to_string(),
            description: "List files through MCP".to_string(),
            invocation_mode:
                crate::agent::capability_bridge::CapabilityInvocationMode::DirectToolCall,
            input_schema_summary: "{\"path\":\"string\"}".to_string(),
            safety_class: "host_tool".to_string(),
            visibility: "default".to_string(),
            observability_tags: vec!["mcp".to_string(), "tool".to_string()],
            requires_approval: false,
            host_mediated: true,
            permission_scope: "workspace.read".to_string(),
        }],
    });

    let execution = runtime.execute_capability_tool_call(&ToolCall {
        call_id: Some("call_workspace_search".to_string()),
        name: "workspace_search".to_string(),
        arguments: json!({ "path": "." }),
        plan: None,
    });

    assert_eq!(
        execution
            .capability
            .as_ref()
            .map(|capability| capability.capability_id.as_str()),
        Some("mcp:tool:workspace_search")
    );
    assert_eq!(execution.tool_result.status, "ok");
    assert_eq!(execution.failure_kind, None);
}

#[test]
fn capability_bridge_keeps_mcp_as_runtime_ingress_not_planner_scheduler_state() {
    let mut runtime =
        build_runtime_for_test(test_provider_selection("http://localhost".to_string()));
    runtime.apply_mcp_source_snapshot(crate::agent::capability_bridge::McpSourceSnapshot {
        source: crate::agent::capability_bridge::CapabilitySourceView {
            source_id: "mcp-local".to_string(),
            source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
            display_name: "Local MCP".to_string(),
            transport_kind: "stdio".to_string(),
            server_identity: "mcp://local".to_string(),
            availability: crate::agent::capability_bridge::CapabilityAvailability::Available,
            declared_capabilities: vec![crate::agent::capability_bridge::CapabilityKind::Tool],
            permission_profile: "host-mediated".to_string(),
            updated_at_ms: 1,
            last_ingress_observation: None,
        },
        capabilities: vec![crate::agent::capability_bridge::CapabilityView {
            capability_id: "mcp:tool:workspace_search".to_string(),
            source_id: "mcp-local".to_string(),
            source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
            kind: crate::agent::capability_bridge::CapabilityKind::Tool,
            label: "workspace_search".to_string(),
            description: "List files through MCP".to_string(),
            invocation_mode:
                crate::agent::capability_bridge::CapabilityInvocationMode::DirectToolCall,
            input_schema_summary: "{\"query\":\"string\"}".to_string(),
            safety_class: "host_tool".to_string(),
            visibility: "default".to_string(),
            observability_tags: vec!["mcp".to_string(), "tool".to_string()],
            requires_approval: false,
            host_mediated: true,
            permission_scope: "workspace.read".to_string(),
        }],
    });

    let planner = LocalTurnPlanner;
    let provider_tool_call = ToolCall {
        call_id: Some("call_workspace_search".to_string()),
        name: "workspace_search".to_string(),
        arguments: json!({ "query": "Cargo.toml" }),
        plan: None,
    };
    let planned = planner
        .select_tool_call(
            "搜索 Cargo.toml",
            &Vec::<TurnHistoryMessage>::new(),
            &[],
            Some(provider_tool_call),
        )
        .expect("planner should preserve provider tool call");

    assert_eq!(planned.name, "workspace_search");
    assert_eq!(planned.arguments, json!({ "query": "Cargo.toml" }));
    assert!(planned.arguments.get("sourceId").is_none());
    assert!(planned.arguments.get("transport").is_none());
    assert!(planned.arguments.get("capabilityId").is_none());

    let execution = runtime.execute_capability_tool_call(&planned);

    assert_eq!(
        execution
            .capability
            .as_ref()
            .map(|capability| capability.capability_id.as_str()),
        Some("mcp:tool:workspace_search")
    );
    assert_eq!(execution.tool_call.name, "workspace_search");
    assert_eq!(
        execution.tool_call.arguments,
        json!({ "query": "Cargo.toml" })
    );
    assert_eq!(execution.tool_result.status, "ok");
}

#[test]
fn capability_bridge_propagates_source_unavailable_from_runtime_execution_path() {
    let mut runtime =
        build_runtime_for_test(test_provider_selection("http://localhost".to_string()));
    runtime.apply_mcp_source_snapshot(crate::agent::capability_bridge::McpSourceSnapshot {
        source: crate::agent::capability_bridge::CapabilitySourceView {
            source_id: "mcp-offline".to_string(),
            source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
            display_name: "Offline MCP".to_string(),
            transport_kind: "stdio".to_string(),
            server_identity: "mcp://offline".to_string(),
            availability: crate::agent::capability_bridge::CapabilityAvailability::Unreachable,
            declared_capabilities: vec![crate::agent::capability_bridge::CapabilityKind::Tool],
            permission_profile: "host-mediated".to_string(),
            updated_at_ms: 1,
            last_ingress_observation: None,
        },
        capabilities: vec![crate::agent::capability_bridge::CapabilityView {
            capability_id: "mcp:tool:offline_search".to_string(),
            source_id: "mcp-offline".to_string(),
            source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
            kind: crate::agent::capability_bridge::CapabilityKind::Tool,
            label: "offline_search".to_string(),
            description: "Offline search".to_string(),
            invocation_mode:
                crate::agent::capability_bridge::CapabilityInvocationMode::DirectToolCall,
            input_schema_summary: "{}".to_string(),
            safety_class: "host_tool".to_string(),
            visibility: "default".to_string(),
            observability_tags: vec!["mcp".to_string(), "tool".to_string()],
            requires_approval: false,
            host_mediated: true,
            permission_scope: "workspace.read".to_string(),
        }],
    });

    let execution = runtime.execute_capability_tool_call(&ToolCall {
        call_id: Some("call_offline_search".to_string()),
        name: "offline_search".to_string(),
        arguments: json!({}),
        plan: None,
    });

    assert_eq!(
        execution.failure_kind,
        Some(CapabilityFailureKind::SourceUnavailable)
    );
    assert!(execution.tool_result.output.contains("source"));
}

#[test]
fn capability_bridge_propagates_permission_denied_from_runtime_execution_path() {
    let mut runtime =
        build_runtime_for_test(test_provider_selection("http://localhost".to_string()));
    runtime.apply_mcp_source_snapshot(crate::agent::capability_bridge::McpSourceSnapshot {
        source: crate::agent::capability_bridge::CapabilitySourceView {
            source_id: "mcp-approval".to_string(),
            source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
            display_name: "Approval MCP".to_string(),
            transport_kind: "stdio".to_string(),
            server_identity: "mcp://approval".to_string(),
            availability: crate::agent::capability_bridge::CapabilityAvailability::Available,
            declared_capabilities: vec![crate::agent::capability_bridge::CapabilityKind::Tool],
            permission_profile: "requires-approval".to_string(),
            updated_at_ms: 1,
            last_ingress_observation: None,
        },
        capabilities: vec![crate::agent::capability_bridge::CapabilityView {
            capability_id: "mcp:tool:guarded_search".to_string(),
            source_id: "mcp-approval".to_string(),
            source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
            kind: crate::agent::capability_bridge::CapabilityKind::Tool,
            label: "guarded_search".to_string(),
            description: "Guarded search".to_string(),
            invocation_mode:
                crate::agent::capability_bridge::CapabilityInvocationMode::DirectToolCall,
            input_schema_summary: "{}".to_string(),
            safety_class: "host_tool".to_string(),
            visibility: "default".to_string(),
            observability_tags: vec!["mcp".to_string(), "tool".to_string()],
            requires_approval: true,
            host_mediated: false,
            permission_scope: "workspace.read".to_string(),
        }],
    });

    let execution = runtime.execute_capability_tool_call(&ToolCall {
        call_id: Some("call_guarded_search".to_string()),
        name: "guarded_search".to_string(),
        arguments: json!({}),
        plan: None,
    });

    assert_eq!(
        execution.failure_kind,
        Some(CapabilityFailureKind::PermissionDenied)
    );
    assert!(execution.tool_result.output.contains("审批"));
}

#[test]
fn capability_bridge_propagates_out_of_scope_from_runtime_execution_path() {
    let runtime = build_runtime_for_test(test_provider_selection("http://localhost".to_string()));

    let execution = runtime.execute_capability_tool_call(&ToolCall {
        call_id: Some("call_out_of_scope_read".to_string()),
        name: "workspace_read_file".to_string(),
        arguments: json!({
            "path": "C:/Windows/System32/drivers/etc/hosts"
        }),
        plan: None,
    });

    assert_eq!(
        execution.failure_kind,
        Some(CapabilityFailureKind::OutOfScope)
    );
    assert!(execution
        .tool_result
        .output
        .contains("当前工作区内的相对路径"));
}

#[test]
fn capability_bridge_propagates_malformed_response_from_runtime_execution_path() {
    let mut runtime =
        build_runtime_for_test(test_provider_selection("http://localhost".to_string()));
    runtime.register_mcp_capability_for_test(crate::agent::capability_bridge::CapabilityView {
        capability_id: "mcp:tool:orphaned".to_string(),
        source_id: "mcp-missing".to_string(),
        source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
        kind: crate::agent::capability_bridge::CapabilityKind::Tool,
        label: "orphaned_tool".to_string(),
        description: "Orphaned tool".to_string(),
        invocation_mode: crate::agent::capability_bridge::CapabilityInvocationMode::DirectToolCall,
        input_schema_summary: "{}".to_string(),
        safety_class: "host_tool".to_string(),
        visibility: "default".to_string(),
        observability_tags: vec!["mcp".to_string(), "tool".to_string()],
        requires_approval: false,
        host_mediated: true,
        permission_scope: "workspace.read".to_string(),
    });
    runtime.remove_mcp_source_for_test("mcp-missing");

    let execution = runtime.execute_capability_tool_call(&ToolCall {
        call_id: Some("call_orphaned_tool".to_string()),
        name: "orphaned_tool".to_string(),
        arguments: json!({}),
        plan: None,
    });

    assert_eq!(
        execution.failure_kind,
        Some(CapabilityFailureKind::MalformedResponse)
    );
    assert!(execution.tool_result.output.contains("registry"));
}

#[test]
fn skill_bridge_executes_tool_only_skill_without_second_scheduler() {
    let mut runtime =
        build_runtime_for_test(test_provider_selection("http://localhost".to_string()));
    runtime.apply_mcp_source_snapshot(crate::agent::capability_bridge::McpSourceSnapshot {
        source: crate::agent::capability_bridge::CapabilitySourceView {
            source_id: "mcp-skills".to_string(),
            source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
            display_name: "Skills MCP".to_string(),
            transport_kind: "stdio".to_string(),
            server_identity: "mcp://skills".to_string(),
            availability: crate::agent::capability_bridge::CapabilityAvailability::Available,
            declared_capabilities: vec![crate::agent::capability_bridge::CapabilityKind::Tool],
            permission_profile: "host-mediated".to_string(),
            updated_at_ms: 1,
            last_ingress_observation: None,
        },
        capabilities: vec![crate::agent::capability_bridge::CapabilityView {
            capability_id: "mcp:tool:workspace_search".to_string(),
            source_id: "mcp-skills".to_string(),
            source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
            kind: crate::agent::capability_bridge::CapabilityKind::Tool,
            label: "workspace_search".to_string(),
            description: "Search workspace".to_string(),
            invocation_mode:
                crate::agent::capability_bridge::CapabilityInvocationMode::DirectToolCall,
            input_schema_summary: "{}".to_string(),
            safety_class: "host_tool".to_string(),
            visibility: "default".to_string(),
            observability_tags: vec!["mcp".to_string(), "tool".to_string()],
            requires_approval: false,
            host_mediated: true,
            permission_scope: "workspace.read".to_string(),
        }],
    });
    runtime
        .apply_skill_source_snapshot(crate::agent::capability_bridge::SkillSourceSnapshot {
            source: crate::agent::capability_bridge::SkillSourceView {
                source_id: "host-skills".to_string(),
                source_kind: crate::agent::capability_bridge::SkillSourceKind::Host,
                display_name: "Host Skills".to_string(),
                availability: crate::agent::capability_bridge::CapabilityAvailability::Available,
                transport_kind: "host".to_string(),
                server_identity: "skills://host".to_string(),
                updated_at_ms: 2,
                last_ingress_observation: None,
            },
            skills: vec![crate::agent::capability_bridge::SkillDescriptor {
                skill_id: "skill:search".to_string(),
                source_id: "host-skills".to_string(),
                source_kind: crate::agent::capability_bridge::SkillSourceKind::Host,
                label: "search".to_string(),
                description: "Search workspace".to_string(),
                input_schema_summary: "{}".to_string(),
                safety_class: "".to_string(),
                visibility: "default".to_string(),
                observability_tags: vec!["host".to_string()],
                requires_approval: false,
                host_mediated: false,
                permission_scope: "".to_string(),
                composed_capability_refs: vec!["mcp:tool:workspace_search".to_string()],
                composed_capability_kinds: vec![],
                executable_in_v1: false,
            }],
        })
        .expect("skill snapshot should apply");

    let execution = runtime.execute_skill_tool_call(&SkillInvocationRequest {
        skill_id: "skill:search".to_string(),
        arguments: json!({ "query": "Cargo.toml" }),
    });

    assert_eq!(execution.failure_layer, None);
    assert_eq!(execution.capability_executions.len(), 1);
    assert_eq!(
        execution
            .skill
            .as_ref()
            .map(|skill| skill.composed_capability_refs.as_slice()),
        Some(["mcp:tool:workspace_search".to_string()].as_slice())
    );
    assert_eq!(
        execution.capability_executions[0].tool_call.arguments,
        json!({ "query": "Cargo.toml" })
    );
    assert!(execution.capability_executions[0]
        .invocation_record_with_skill_context(
            execution.skill.as_ref(),
            execution.failure_layer.as_ref()
        )
        .skill_id
        .is_some());
}

#[test]
fn runtime_executes_registered_skill_by_tool_name() {
    let mut runtime =
        build_runtime_for_test(test_provider_selection("http://localhost".to_string()));
    runtime
        .apply_skill_source_snapshot(crate::agent::capability_bridge::SkillSourceSnapshot {
            source: crate::agent::capability_bridge::SkillSourceView {
                source_id: "builtin-skills".to_string(),
                source_kind: crate::agent::capability_bridge::SkillSourceKind::Host,
                display_name: "Builtin Skills".to_string(),
                availability: crate::agent::capability_bridge::CapabilityAvailability::Available,
                transport_kind: "host".to_string(),
                server_identity: "skills://builtin".to_string(),
                updated_at_ms: 1,
                last_ingress_observation: None,
            },
            skills: vec![crate::agent::capability_bridge::SkillDescriptor {
                skill_id: "skill:clock".to_string(),
                source_id: "builtin-skills".to_string(),
                source_kind: crate::agent::capability_bridge::SkillSourceKind::Host,
                label: "clock".to_string(),
                description: "Get current time".to_string(),
                input_schema_summary: "{}".to_string(),
                safety_class: "".to_string(),
                visibility: "default".to_string(),
                observability_tags: vec![],
                requires_approval: false,
                host_mediated: false,
                permission_scope: "".to_string(),
                composed_capability_refs: vec!["builtin:time_now".to_string()],
                composed_capability_kinds: vec![],
                executable_in_v1: false,
            }],
        })
        .expect("skill snapshot should apply");

    let (tool_result, invocation_record, hook_trace_records) = runtime
        .execute_registered_tool_call(&ToolCall {
            call_id: None,
            name: "clock".to_string(),
            arguments: json!({}),
            plan: None,
        });

    assert_eq!(tool_result.status, "ok");
    assert_eq!(invocation_record.skill_id.as_deref(), Some("skill:clock"));
    assert_eq!(invocation_record.failure_layer.as_deref(), None);
    assert_eq!(hook_trace_records.len(), 1);
    assert_eq!(
        hook_trace_records[0].hook_point,
        TurnHookPoint::SkillToolActionsResolve
    );
}

#[test]
fn skill_bridge_rejects_non_tool_composed_skill_as_unsupported() {
    let mut runtime =
        build_runtime_for_test(test_provider_selection("http://localhost".to_string()));
    runtime.apply_mcp_source_snapshot(crate::agent::capability_bridge::McpSourceSnapshot {
        source: crate::agent::capability_bridge::CapabilitySourceView {
            source_id: "mcp-skills".to_string(),
            source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
            display_name: "Skills MCP".to_string(),
            transport_kind: "stdio".to_string(),
            server_identity: "mcp://skills".to_string(),
            availability: crate::agent::capability_bridge::CapabilityAvailability::Available,
            declared_capabilities: vec![crate::agent::capability_bridge::CapabilityKind::Resource],
            permission_profile: "host-mediated".to_string(),
            updated_at_ms: 1,
            last_ingress_observation: None,
        },
        capabilities: vec![crate::agent::capability_bridge::CapabilityView {
            capability_id: "mcp:resource:repo_index".to_string(),
            source_id: "mcp-skills".to_string(),
            source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
            kind: crate::agent::capability_bridge::CapabilityKind::Resource,
            label: "repo_index".to_string(),
            description: "Repository index".to_string(),
            invocation_mode:
                crate::agent::capability_bridge::CapabilityInvocationMode::ReadOnlyFetch,
            input_schema_summary: "{}".to_string(),
            safety_class: "read_only".to_string(),
            visibility: "default".to_string(),
            observability_tags: vec!["mcp".to_string(), "resource".to_string()],
            requires_approval: false,
            host_mediated: true,
            permission_scope: "workspace.read".to_string(),
        }],
    });
    runtime
        .apply_skill_source_snapshot(crate::agent::capability_bridge::SkillSourceSnapshot {
            source: crate::agent::capability_bridge::SkillSourceView {
                source_id: "host-skills".to_string(),
                source_kind: crate::agent::capability_bridge::SkillSourceKind::Host,
                display_name: "Host Skills".to_string(),
                availability: crate::agent::capability_bridge::CapabilityAvailability::Available,
                transport_kind: "host".to_string(),
                server_identity: "skills://host".to_string(),
                updated_at_ms: 2,
                last_ingress_observation: None,
            },
            skills: vec![crate::agent::capability_bridge::SkillDescriptor {
                skill_id: "skill:index".to_string(),
                source_id: "host-skills".to_string(),
                source_kind: crate::agent::capability_bridge::SkillSourceKind::Host,
                label: "index".to_string(),
                description: "Index repository".to_string(),
                input_schema_summary: "{}".to_string(),
                safety_class: "".to_string(),
                visibility: "default".to_string(),
                observability_tags: vec![],
                requires_approval: false,
                host_mediated: false,
                permission_scope: "".to_string(),
                composed_capability_refs: vec!["mcp:resource:repo_index".to_string()],
                composed_capability_kinds: vec![],
                executable_in_v1: false,
            }],
        })
        .expect("skill snapshot should apply");

    let execution = runtime.execute_skill_tool_call(&SkillInvocationRequest {
        skill_id: "skill:index".to_string(),
        arguments: json!({ "path": "." }),
    });

    assert_eq!(
        execution.failure_layer,
        Some(SkillFailureLayer::UnsupportedComposition)
    );
    assert_eq!(
        execution.capability_executions[0].tool_result.status,
        "error"
    );
}

#[test]
fn skill_bridge_propagates_underlying_capability_execution_failure() {
    let mut runtime = build_runtime_for_test_with_tool_executor(
        test_provider_selection("http://localhost".to_string()),
        Box::new(ErrorToolExecutor),
    );
    runtime
        .apply_skill_source_snapshot(crate::agent::capability_bridge::SkillSourceSnapshot {
            source: crate::agent::capability_bridge::SkillSourceView {
                source_id: "builtin-skills".to_string(),
                source_kind: crate::agent::capability_bridge::SkillSourceKind::Host,
                display_name: "Builtin Skills".to_string(),
                availability: crate::agent::capability_bridge::CapabilityAvailability::Available,
                transport_kind: "host".to_string(),
                server_identity: "skills://builtin".to_string(),
                updated_at_ms: 1,
                last_ingress_observation: None,
            },
            skills: vec![crate::agent::capability_bridge::SkillDescriptor {
                skill_id: "skill:read-file".to_string(),
                source_id: "builtin-skills".to_string(),
                source_kind: crate::agent::capability_bridge::SkillSourceKind::Host,
                label: "read-file".to_string(),
                description: "Read a file".to_string(),
                input_schema_summary: "{\"path\":\"string\"}".to_string(),
                safety_class: "".to_string(),
                visibility: "default".to_string(),
                observability_tags: vec![],
                requires_approval: false,
                host_mediated: false,
                permission_scope: "".to_string(),
                composed_capability_refs: vec!["builtin:workspace_read_file".to_string()],
                composed_capability_kinds: vec![],
                executable_in_v1: false,
            }],
        })
        .expect("skill snapshot should apply");

    let execution = runtime.execute_skill_tool_call(&SkillInvocationRequest {
        skill_id: "skill:read-file".to_string(),
        arguments: json!({ "path": "missing-file-for-skill-bridge-test.txt" }),
    });

    assert_eq!(
        execution.failure_layer,
        Some(SkillFailureLayer::UnderlyingCapabilityExecution)
    );
    assert_eq!(
        execution.capability_executions[0].failure_kind,
        Some(CapabilityFailureKind::InvocationFailed)
    );
}

#[test]
fn capability_bridge_propagates_partial_out_of_scope_from_runtime_execution_path() {
    let runtime = build_runtime_for_test_with_tool_executor(
        test_provider_selection("http://localhost".to_string()),
        Box::new(PartialOutOfScopeToolExecutor),
    );

    let execution = runtime.execute_capability_tool_call(&ToolCall {
        call_id: Some("call_partial_batch".to_string()),
        name: "workspace_batch".to_string(),
        arguments: json!({ "calls": [] }),
        plan: None,
    });

    assert_eq!(
        execution.failure_kind,
        Some(CapabilityFailureKind::OutOfScope)
    );
    assert_eq!(execution.tool_result.status, "ok");
    assert!(execution
        .tool_result
        .output
        .contains("\"status\":\"partial\""));
}

#[test]
fn skill_bridge_propagates_partial_out_of_scope_from_underlying_capability() {
    let mut runtime = build_runtime_for_test_with_tool_executor(
        test_provider_selection("http://localhost".to_string()),
        Box::new(PartialOutOfScopeToolExecutor),
    );
    runtime
        .apply_skill_source_snapshot(crate::agent::capability_bridge::SkillSourceSnapshot {
            source: crate::agent::capability_bridge::SkillSourceView {
                source_id: "builtin-skills".to_string(),
                source_kind: crate::agent::capability_bridge::SkillSourceKind::Host,
                display_name: "Builtin Skills".to_string(),
                availability: crate::agent::capability_bridge::CapabilityAvailability::Available,
                transport_kind: "host".to_string(),
                server_identity: "skills://builtin".to_string(),
                updated_at_ms: 1,
                last_ingress_observation: None,
            },
            skills: vec![crate::agent::capability_bridge::SkillDescriptor {
                skill_id: "skill:batch_execute".to_string(),
                source_id: "builtin-skills".to_string(),
                source_kind: crate::agent::capability_bridge::SkillSourceKind::Host,
                label: "batch_execute".to_string(),
                description: "批量执行多个工具子调用".to_string(),
                input_schema_summary: "{\"calls\":\"array\"}".to_string(),
                safety_class: "".to_string(),
                visibility: "default".to_string(),
                observability_tags: vec![],
                requires_approval: false,
                host_mediated: false,
                permission_scope: "".to_string(),
                composed_capability_refs: vec!["builtin:workspace_batch".to_string()],
                composed_capability_kinds: vec![],
                executable_in_v1: false,
            }],
        })
        .expect("skill snapshot should apply");

    let execution = runtime.execute_skill_tool_call(&SkillInvocationRequest {
        skill_id: "skill:batch_execute".to_string(),
        arguments: json!({ "calls": [] }),
    });

    assert_eq!(
        execution.failure_layer,
        Some(SkillFailureLayer::UnderlyingCapabilityExecution)
    );
    assert_eq!(
        execution.capability_executions[0].failure_kind,
        Some(CapabilityFailureKind::OutOfScope)
    );
    assert_eq!(execution.capability_executions[0].tool_result.status, "ok");
}

fn temp_marker_file_path(prefix: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("{prefix}-{stamp}.tmp"))
}

fn decision_tool_call(tool_name: &str, arguments: serde_json::Value) -> serde_json::Value {
    json!({
        "choices": [
            {
                "message": {
                    "role": "assistant",
                    "content": format!("先调用 {}。", tool_name),
                    "reasoning_content": format!("需要先执行 {}。", tool_name),
                    "tool_calls": [
                        {
                            "id": format!("call_{}", tool_name),
                            "type": "function",
                            "function": {
                                "name": tool_name,
                                "arguments": arguments.to_string()
                            }
                        }
                    ]
                }
            }
        ]
    })
}

fn sse_decision_tool_call(tool_name: &str, arguments: serde_json::Value) -> MockHttpResponse {
    sse_response(&[json!({
        "choices": [
            {
                "delta": {
                    "content": format!("先调用 {}。", tool_name),
                    "reasoning_content": format!("需要先执行 {}。", tool_name),
                    "tool_calls": [
                        {
                            "index": 0,
                            "id": format!("call_{}", tool_name),
                            "type": "function",
                            "function": {
                                "name": tool_name,
                                "arguments": arguments.to_string()
                            }
                        }
                    ]
                }
            }
        ]
    })])
}

fn decision_blank_tool_call(arguments: serde_json::Value) -> serde_json::Value {
    json!({
        "choices": [
            {
                "message": {
                    "role": "assistant",
                    "content": "继续读取目标文件。",
                    "reasoning_content": "参数已经足够，继续执行下一步读取。",
                    "tool_calls": [
                        {
                            "id": "call_blank_name",
                            "type": "function",
                            "function": {
                                "name": "",
                                "arguments": arguments.to_string()
                            }
                        }
                    ]
                }
            }
        ]
    })
}

#[test]
fn start_turn_stream_uses_sink_for_empty_input_failure() {
    let mut runtime = AgentRuntime::new();
    let sink = RecordingTurnEventSink::new();

    runtime.start_turn_stream(
        &sink,
        "turn-empty".to_string(),
        TurnInput {
            message: "   ".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            workspace_mode: None,
            session_id: Some("test-session".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
        },
    );

    let events = sink.events.borrow();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].0, "turn:failed");
    assert_eq!(events[0].1.turn_id, "turn-empty");
    assert_eq!(events[0].1.phase.as_deref(), Some("failed"));
    assert_eq!(events[0].1.error.as_deref(), Some("Message is empty."));
    assert_hook_boundary_alignment(
        &events[0].1,
        TurnHookPoint::TurnFinalizeEnd,
        "turn.failed",
        "failed",
    );
}

#[test]
fn runtime_default_tool_executor_routes_read_tools_through_governed_dispatcher() {
    // PA-076 runtime switch: the default engine is `build_governed_executor`, so a real
    // read tool call executed by the runtime must succeed through the governed dispatcher.
    let runtime = AgentRuntime::new();
    let (result, _record, _traces) = runtime.execute_registered_tool_call(&ToolCall {
        call_id: Some("call-gov-list".to_string()),
        name: "List".to_string(),
        arguments: json!({ "path": ".", "description": "test" }),
        plan: None,
    });
    assert_eq!(
        result.status, "ok",
        "default governed executor failed List: {}",
        result.output
    );
}

#[test]
fn runtime_default_tool_executor_fails_closed_for_run_without_sandbox() {
    // Execute scope + no `SandboxBackend` on the governed dispatcher => designed fail-closed
    // (design.md Decision 7 / task 5.5), surfaced through the runtime's default executor.
    let runtime = AgentRuntime::new();
    let (result, _record, _traces) = runtime.execute_registered_tool_call(&ToolCall {
        call_id: Some("call-gov-run".to_string()),
        name: "Run".to_string(),
        arguments: json!({ "command": "echo hi", "description": "test" }),
        plan: None,
    });
    assert_eq!(result.status, "error");
    assert!(
        result.output.contains("sandbox_unavailable"),
        "expected sandbox_unavailable, got: {}",
        result.output
    );
}

#[test]
fn start_turn_stream_can_emit_cancelled_when_stop_requested_before_plan() {
    let selection = test_provider_selection("http://127.0.0.1:1/v1".to_string());
    let runtime = build_runtime_for_test(selection);
    let sink = RecordingTurnEventSink::new();
    let control = ExecutionControlRegistry::new();

    control.register_turn("turn-cancelled", Some("stop-session"), None);
    let response = control.request_stop("turn-cancelled");
    assert!(response.accepted);

    runtime.start_turn_stream_with_control(
        &sink,
        &control,
        "turn-cancelled".to_string(),
        TurnInput {
            message: "继续读取 tauri.conf.json".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            workspace_mode: None,
            session_id: Some("stop-session".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
        },
    );

    let events = sink.events.borrow();
    assert!(events.iter().any(|(name, payload)| {
        name == "turn:cancelled"
            && payload.turn_id == "turn-cancelled"
            && payload.error.as_deref() == Some("stopped_by_user")
    }));
    let cancelled = events
        .iter()
        .find_map(|(name, payload)| (name == "turn:cancelled").then_some(payload.clone()))
        .expect("cancelled event");
    assert_hook_boundary_alignment(
        &cancelled,
        TurnHookPoint::TurnFinalizeEnd,
        "turn.cancelled",
        "cancelled",
    );

    let snapshot = runtime.load_session_snapshot(Some("stop-session"));
    assert_eq!(snapshot.history.len(), 2);
    assert_eq!(snapshot.history[0].role, "user");
    assert_eq!(snapshot.history[0].content, "继续读取 tauri.conf.json");
    assert_eq!(snapshot.history[1].role, "assistant");
    assert_eq!(snapshot.history[1].content, CANCELLED_TURN_MESSAGE);
}

/// PA-095：同步 run_turn 与 streaming 入口事件序列等价性——completed 主干 +
/// failed 分支逐事件类型对比。assistant/chunk 为 streaming 增量推送独有
/// （同步入口无 delta），剔除后骨架必须一致；悬挂 turn 不变式：凡发射
/// turn/start 的 turn 必有 turn/end 终态配对。事件经测试多播 sink 捕获
/// （生产单槽通道可被并行测试的 HostControlPlane 构建覆盖），按 session_id
/// 隔离其他测试的事件。
#[test]
fn run_turn_and_start_turn_stream_produce_equivalent_event_sequences() {
    let persisted: Arc<Mutex<Vec<(String, String, String)>>> = Arc::new(Mutex::new(Vec::new()));
    let persisted_for_channel = Arc::clone(&persisted);
    // 守卫式注册：Drop 仅移除本 sink（并行测试互不清除对方注册）。
    let _sink_guard = crate::agent::turn_flow::register_event_persist_test_sink(Arc::new(
        move |session_id, turn_id, event, _terminal| {
            persisted_for_channel.lock().expect("persisted lock").push((
                session_id.to_string(),
                turn_id.to_string(),
                event.type_name().to_string(),
            ));
        },
    ));

    // 骨架提取：本 session 的事件类型序列，剔除 streaming 独有的 assistant/chunk。
    fn skeleton(records: &[(String, String, String)], session: &str) -> Vec<String> {
        records
            .iter()
            .filter(|(sid, _, event_type)| sid == session && event_type != "assistant/chunk")
            .map(|(_, _, event_type)| event_type.clone())
            .collect()
    }

    // ---- completed 主干：同步入口（JSON completion 带 usage）----
    let sync_server = MockHttpServer::start(vec![json_response(json!({
        "choices": [{"message": {"role": "assistant", "content": "同步等价答案。"}}],
        "usage": {"prompt_tokens": 12, "completion_tokens": 4, "total_tokens": 16}
    }))]);
    let sync_runtime =
        build_runtime_for_test(test_provider_selection(sync_server.base_url.clone()));
    let sync_result = sync_runtime.run_turn(TurnInput {
        message: "同步等价问题".to_string(),
        display_message: None,
        provider_id: None,
        model_id: None,
        reasoning_effort: None,
        workspace_mode: None,
        session_id: Some("eq-sync-session".to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
    });
    // 同步入口成功终态的 TurnResult.phase 惯例为 "ready"（事件流终态仍为
    // turn/end completed——两者语义层不同）。
    assert_eq!(sync_result.phase, "ready", "sync turn must complete");
    let _ = sync_server.finish();

    // ---- completed 主干：streaming 入口（SSE 内容 + usage chunk）----
    let stream_server = MockHttpServer::start(vec![sse_response(&[
        json!({"choices": [{"delta": {"content": "流式等价答案。"}}]}),
        json!({
            "choices": [],
            "usage": {"prompt_tokens": 12, "completion_tokens": 4, "total_tokens": 16}
        }),
    ])]);
    let mut stream_runtime =
        build_runtime_for_test(test_provider_selection(stream_server.base_url.clone()));
    let sink = RecordingTurnEventSink::new();
    stream_runtime.start_turn_stream(
        &sink,
        "eq-stream-turn".to_string(),
        TurnInput {
            message: "流式等价问题".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            workspace_mode: None,
            session_id: Some("eq-stream-session".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
        },
    );
    let _ = stream_server.finish();

    // ---- failed 分支：两入口均打不可达端点 ----
    let failed_sync_runtime =
        build_runtime_for_test(test_provider_selection("http://127.0.0.1:1/v1".to_string()));
    let failed_sync_result = failed_sync_runtime.run_turn(TurnInput {
        message: "同步失败分支".to_string(),
        display_message: None,
        provider_id: None,
        model_id: None,
        reasoning_effort: None,
        workspace_mode: None,
        session_id: Some("eq-sync-failed-session".to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
    });
    assert_eq!(failed_sync_result.phase, "failed", "sync turn must fail");

    let mut failed_stream_runtime =
        build_runtime_for_test(test_provider_selection("http://127.0.0.1:1/v1".to_string()));
    let failed_sink = RecordingTurnEventSink::new();
    failed_stream_runtime.start_turn_stream(
        &failed_sink,
        "eq-stream-failed-turn".to_string(),
        TurnInput {
            message: "流式失败分支".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            workspace_mode: None,
            session_id: Some("eq-stream-failed-session".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
        },
    );

    let records = persisted.lock().expect("persisted lock").clone();
    drop(_sink_guard);

    // completed 主干骨架对比。
    let sync_skeleton = skeleton(&records, "eq-sync-session");
    let stream_skeleton = skeleton(&records, "eq-stream-session");
    assert_eq!(
        sync_skeleton, stream_skeleton,
        "completed skeleton must match (assistant/chunk excluded)"
    );
    assert_eq!(
        sync_skeleton,
        vec![
            "turn/start".to_string(),
            "context/observation".to_string(),
            "user/message".to_string(),
            // PA-095 #4：初始 call 的 StepStart（turn:started 承担 step 0）。
            "step/start".to_string(),
            "assistant/message".to_string(),
            "provider/usage".to_string(),
            // PA-095 #4：usage 结算同点的 StepEnd（单 call → 1 对）。
            "step/end".to_string(),
            "turn/end".to_string(),
        ],
        "completed skeleton shape"
    );

    // failed 分支骨架对比。
    let sync_failed_skeleton = skeleton(&records, "eq-sync-failed-session");
    let stream_failed_skeleton = skeleton(&records, "eq-stream-failed-session");
    assert_eq!(
        sync_failed_skeleton, stream_failed_skeleton,
        "failed skeleton must match"
    );
    assert_eq!(
        sync_failed_skeleton.last().map(String::as_str),
        Some("turn/end"),
        "failed branch terminates with turn/end"
    );

    // 悬挂 turn 不变式：凡有 turn/start 的 turn 必有 turn/end 终态。
    // 只检查本测试的 session（多播 sink 会收到其他并行测试的进行中 turn，
    // 其终态尚未发射，不构成悬挂）。
    let own_sessions: std::collections::HashSet<&str> = [
        "eq-sync-session",
        "eq-stream-session",
        "eq-sync-failed-session",
        "eq-stream-failed-session",
    ]
    .into_iter()
    .collect();
    let mut turns_with_start: std::collections::HashSet<(String, String)> =
        std::collections::HashSet::new();
    let mut turns_with_end: std::collections::HashSet<(String, String)> =
        std::collections::HashSet::new();
    for (session, turn, event_type) in &records {
        if !own_sessions.contains(session.as_str()) {
            continue;
        }
        match event_type.as_str() {
            "turn/start" => {
                turns_with_start.insert((session.clone(), turn.clone()));
            }
            "turn/end" => {
                turns_with_end.insert((session.clone(), turn.clone()));
            }
            _ => {}
        }
    }
    for turn in &turns_with_start {
        assert!(
            turns_with_end.contains(turn),
            "dangling turn without terminal: {turn:?}"
        );
    }
}

/// PA-095 #4：多 hop turn（streaming 3 次 provider call + 2 次工具）事件重建
/// timeline——call_model 条目数 = hop 数，chunk 文本归属正确 hop；
/// StepEnd 与 ProviderUsage 相邻成对。sync 变体（无 StepStart 的 step≥1）
/// 经 usage 兜底创建同样收敛到 hop 数。
#[test]
fn multi_hop_turn_rebuilds_timeline_with_per_hop_call_model_entries() {
    use crate::agent::projection::{Projection, TraceProjectionState};
    use crate::agent::turn_event::TurnEvent;

    // 事件收集：(session, turn, event)，发射序即日志序（合成 seq 用）。
    let collected: Arc<Mutex<Vec<(String, String, TurnEvent)>>> = Arc::new(Mutex::new(Vec::new()));
    let collected_for_sink = Arc::clone(&collected);
    // 守卫式注册：Drop 仅移除本 sink（并行测试互不清除对方注册）。
    let _sink_guard = crate::agent::turn_flow::register_event_persist_test_sink(Arc::new(
        move |session_id, turn_id, event, _terminal| {
            collected_for_sink.lock().expect("collected lock").push((
                session_id.to_string(),
                turn_id.to_string(),
                event,
            ));
        },
    ));

    let own = |records: &[(String, String, TurnEvent)], session: &str| -> Vec<TurnEvent> {
        records
            .iter()
            .filter(|(sid, _, _)| sid == session)
            .map(|(_, _, event)| event.clone())
            .collect()
    };

    // ---- streaming 变体：初始 call + 2 个 followup（各带工具）----
    let final_text = "第三行是 productName。";
    let server = MockHttpServer::start(vec![
        sse_decision_tool_call("workspace_list_files", json!({"path": "."})),
        sse_response(&[
            json!({"choices": [{"delta": {"content": "找到了！让我读取文件内容："}}]}),
            json!({"choices": [{"delta": {"tool_calls": [
                {"index": 0, "id": "call_read_file", "type": "function",
                 "function": {"name": "workspace_read_file", "arguments": "{\"path\":\"tauri.conf.json\"}"}}
            ]}}]}),
        ]),
        sse_response(&[
            json!({"choices": [{"delta": {"content": final_text}}]}),
            json!({"choices": [], "usage": {"prompt_tokens": 9, "completion_tokens": 3, "total_tokens": 12}}),
        ]),
    ]);
    let mut runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));
    let sink = RecordingTurnEventSink::new();
    runtime.start_turn_stream(
        &sink,
        "turn-step-hops".to_string(),
        TurnInput {
            message: "读取 tauri.conf.json 第三行".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            workspace_mode: None,
            session_id: Some("step-hop-stream".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
        },
    );
    server.finish();

    let records = collected.lock().expect("collected lock").clone();
    let stream_events = own(&records, "step-hop-stream");
    let stream_types: Vec<&str> = stream_events.iter().map(TurnEvent::type_name).collect();

    // StepStart 序列：step 0（started）+ step 1/2（followup calling_model trace）。
    let step_starts: Vec<u32> = stream_events
        .iter()
        .filter_map(|event| match event {
            TurnEvent::StepStart { step, .. } => Some(*step),
            _ => None,
        })
        .collect();
    assert_eq!(step_starts, vec![0, 1, 2], "step/start per provider call");

    // StepEnd 与 ProviderUsage 相邻成对（usage[i] 紧跟 step/end[i]）。
    for window in stream_types.windows(2) {
        if window[0] == "provider/usage" {
            assert_eq!(window[1], "step/end", "step/end adjacent to usage");
        }
    }
    assert_eq!(
        stream_types.iter().filter(|t| **t == "step/end").count(),
        3,
        "one step/end per settled provider call"
    );

    // chunk step 归属：hop1/hop2 的文本分别落各自事件。
    let chunk_steps: Vec<u32> = stream_events
        .iter()
        .filter_map(|event| match event {
            TurnEvent::AssistantChunk { step, text, .. } if !text.is_empty() => Some(*step),
            _ => None,
        })
        .collect();
    assert_eq!(
        chunk_steps,
        vec![0, 1, 2],
        "non-empty chunks carry their hop step"
    );

    // timeline 重建：call_model 条目数 = hop 数。
    let mut state = TraceProjectionState::init();
    for (index, event) in stream_events.iter().enumerate() {
        TraceProjectionState::apply(&mut state, index as u64 + 1, event);
    }
    let rebuilt = state
        .trace_for_turn("turn-step-hops")
        .expect("rebuilt trace");
    let call_models: Vec<&crate::agent::session::TraceTimelineEntry> = rebuilt
        .trace_timeline
        .iter()
        .filter(|entry| entry.kind == "call_model")
        .collect();
    assert_eq!(call_models.len(), 3, "call_model entries equal hop count");

    // chunk 文本归属正确 hop：hop2 条目含 followup 文本、hop3 含最终文本。
    let hop_texts: Vec<Option<&String>> = call_models
        .iter()
        .map(|entry| entry.text.as_ref())
        .collect();
    assert!(
        !hop_texts[0]
            .as_deref()
            .is_some_and(|text| text.contains("找到了")),
        "hop1 entry must not contain followup-1 text"
    );
    assert!(
        hop_texts[1]
            .as_deref()
            .is_some_and(|text| text.contains("找到了！让我读取文件内容：")),
        "followup-1 chunks aggregate into their own call_model entry"
    );
    assert!(
        hop_texts[2]
            .as_deref()
            .is_some_and(|text| text.contains(final_text)),
        "final chunks aggregate into their own call_model entry"
    );

    // ---- sync 变体：无 step≥1 的 StepStart，usage 兜底创建 call_model ----
    let sync_server = MockHttpServer::start(vec![
        json_response(json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "先调用工具。",
                    "reasoning_content": "需要先列出文件。",
                    "tool_calls": [{
                        "id": "call_sync_list",
                        "type": "function",
                        "function": {"name": "workspace_list_files", "arguments": "{\"path\":\".\"}"}
                    }]
                }
            }]
        })),
        json_completion("同步多跳完成。"),
    ]);
    let sync_runtime =
        build_runtime_for_test(test_provider_selection(sync_server.base_url.clone()));
    let sync_result = sync_runtime.run_turn(TurnInput {
        message: "同步多跳问题".to_string(),
        display_message: None,
        provider_id: None,
        model_id: None,
        reasoning_effort: None,
        workspace_mode: None,
        session_id: Some("step-hop-sync".to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
    });
    assert_eq!(sync_result.phase, "ready");
    sync_server.finish();

    let records = collected.lock().expect("collected lock").clone();
    drop(_sink_guard);

    let sync_events = own(&records, "step-hop-sync");
    let mut sync_state = TraceProjectionState::init();
    for (index, event) in sync_events.iter().enumerate() {
        TraceProjectionState::apply(&mut sync_state, index as u64 + 1, event);
    }
    let sync_rebuilt = sync_state
        .trace_for_turn("sync:step-hop-sync")
        .or_else(|| sync_state.trace_for_turn(&first_turn_id(&sync_events)))
        .expect("sync rebuilt trace");
    let sync_call_models = sync_rebuilt
        .trace_timeline
        .iter()
        .filter(|entry| entry.kind == "call_model")
        .count();
    assert_eq!(
        sync_call_models, 2,
        "sync multi-hop rebuilds via usage fallback (2 provider calls)"
    );
}

/// 事件列表中首个带 turn 归属的 turn_id（sync turn_id 运行时生成，测试不预知）。
fn first_turn_id(events: &[crate::agent::turn_event::TurnEvent]) -> String {
    events
        .iter()
        .find_map(|event| event.turn_id().map(str::to_string))
        .expect("at least one turn-scoped event")
}

/// PA-095 #4：`TurnStreamEvent.step` wire 兼容——旧 payload（无 step 字段）
/// 反序列化得 None（缺省归 step 0）；新 payload 携带 step 序列化往返。
#[test]
fn turn_stream_event_step_serde_roundtrip() {
    let legacy_json = r#"{"turnId":"turn-legacy","kind":"delta"}"#;
    let legacy: TurnStreamEvent = serde_json::from_str(legacy_json).expect("legacy payload");
    assert_eq!(legacy.turn_id, "turn-legacy");
    assert_eq!(legacy.step, None, "missing step deserializes to None");

    let payload = TurnStreamEvent {
        event_id: None,
        session_id: None,
        turn_id: "turn-new".to_string(),
        kind: "delta".to_string(),
        event_type: None,
        event_version: None,
        sequence: None,
        emitted_at_ms: None,
        phase: None,
        text: Some("hi".to_string()),
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
        step: Some(2),
    };
    let json = serde_json::to_string(&payload).expect("serialize");
    assert!(json.contains(r#""step":2"#), "step serializes: {json}");
    let roundtrip: TurnStreamEvent = serde_json::from_str(&json).expect("roundtrip");
    assert_eq!(roundtrip.step, Some(2));
}

#[test]
fn runtime_can_build_graph_turn_handoff_from_stable_turn_artifacts() {
    let selection = test_provider_selection("http://127.0.0.1:1/v1".to_string());
    let mut runtime = build_runtime_for_test(selection);
    runtime.load_session_snapshot(Some("graph-session"));
    let result = TurnResult {
        event_id: None,
        event_type: None,
        event_version: None,
        sequence: None,
        emitted_at_ms: None,
        phase: "ready".to_string(),
        provider_requested_name: "OpenAI".to_string(),
        provider_name: "OpenAI".to_string(),
        provider_protocol: "openai".to_string(),
        provider_model: "gpt-5".to_string(),
        provider_source: "primary".to_string(),
        provider_mode: "standard".to_string(),
        fallback_reason: None,
        build_context_observation: None,
        input_tokens: None,
        cache_hit_input_tokens: None,
        reasoning_tokens: None,
        output_tokens: None,
        total_tokens: None,
        first_token_latency_ms: None,
        turn_duration_ms: None,
        user_message: "请继续处理".to_string(),
        assistant_message: "当前轮已收口。".to_string(),
        trace_steps: Vec::new(),
        trace_timeline: Vec::new(),
        tool_activities: Vec::new(),
        provider_call_records: Vec::new(),
        hook_trace_records: Vec::new(),
        session_summary: "summary".to_string(),
    };
    let checkpoint = ExecutionCheckpoint {
        contract_version: "execution-checkpoint-v1".to_string(),
        turn_id: "turn-graph".to_string(),
        session_id: Some("graph-session".to_string()),
        run_id: None,
        checkpoint_kind: "runtime_control".to_string(),
        recovery_mode: "replay_required".to_string(),
        projected_runtime_phase: "ready".to_string(),
        submission_command: None,
        resumable: false,
        replayable: false,
        status: "completed".to_string(),
        phase: "ready".to_string(),
        provider_requested_name: Some("OpenAI".to_string()),
        provider_name: Some("OpenAI".to_string()),
        provider_protocol: Some("openai".to_string()),
        provider_model: Some("gpt-5".to_string()),
        provider_source: Some("primary".to_string()),
        provider_mode: Some("standard".to_string()),
        fallback_reason: None,
        completed_hops: 0,
        max_hops: 16,
        active_tool_name: None,
        trace_steps: Vec::new(),
        tool_activities: Vec::new(),
        persisted_effect_evidence: Vec::new(),
        error: None,
        started_at_ms: 0,
        updated_at_ms: 0,
        stop_requested_at_ms: None,
    };

    let handoff = runtime.build_graph_turn_handoff(
        None,
        Some("turn-graph"),
        Some("graph-session"),
        &result,
        Some(&checkpoint),
        None,
    );
    let decision = runtime.decide_graph_after_turn(
        Some("turn-graph"),
        Some("graph-session"),
        &result,
        Some(&checkpoint),
        None,
    );

    assert_eq!(handoff.turn_id.as_deref(), Some("turn-graph"));
    assert_eq!(handoff.session_id.as_deref(), Some("graph-session"));
    assert_eq!(handoff.long_term_memory_status, "empty");
    assert_eq!(handoff.provider_name, "OpenAI");
    assert_eq!(
        decision.kind,
        crate::agent::graph::GraphDecisionKind::WaitUser
    );
}

#[test]
fn turn_input_workspace_id_stamps_first_turn_and_is_idempotent() {
    // PA-079 P1-1 闭环：全新会话首轮 turn 携带 workspace_id → 盖章 → snapshot/overview 携带；
    // 第二轮不同 id 不覆盖（幂等）。
    let _rt_guard = crate::agent::runtime_helper::TestRuntimeGuard::new();
    let server = MockHttpServer::start(vec![
        json_response(
            json!({ "choices": [ { "message": { "role": "assistant", "content": "ok" } } ] }),
        ),
        json_response(
            json!({ "choices": [ { "message": { "role": "assistant", "content": "ok2" } } ] }),
        ),
    ]);
    let sessions = SessionStore::memory_only();
    let selection = test_provider_selection(server.base_url.clone());
    let runtime = build_runtime_with_session_store(selection, sessions);

    let _ = runtime.run_turn(TurnInput {
        message: "hello".to_string(),
        display_message: None,
        provider_id: None,
        model_id: None,
        reasoning_effort: None,
        workspace_mode: None,
        session_id: Some("ws-session".to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: Some("ws-proj-1".to_string()),
    });

    let snapshot = runtime.load_session_snapshot(Some("ws-session"));
    assert_eq!(
        snapshot.workspace_id.as_deref(),
        Some("ws-proj-1"),
        "首轮 workspace_id 应盖章到会话"
    );
    let overviews = runtime.sessions_handle().read().unwrap().list_sessions();
    let overview = overviews
        .iter()
        .find(|session| session.conversation_id == "ws-session")
        .expect("session overview exists");
    assert_eq!(overview.workspace_id.as_deref(), Some("ws-proj-1"));

    // 第二轮不同 id → 幂等，不覆盖
    let _ = runtime.run_turn(TurnInput {
        message: "hello 2".to_string(),
        display_message: None,
        provider_id: None,
        model_id: None,
        reasoning_effort: None,
        workspace_mode: None,
        session_id: Some("ws-session".to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: Some("ws-proj-2".to_string()),
    });
    let snapshot2 = runtime.load_session_snapshot(Some("ws-session"));
    assert_eq!(
        snapshot2.workspace_id.as_deref(),
        Some("ws-proj-1"),
        "已盖章会话的后续轮不得覆盖"
    );

    let _ = server.finish();
}

#[test]
fn run_turn_fails_when_attachment_persistence_fails() {
    let _rt_guard = crate::agent::runtime_helper::TestRuntimeGuard::new();
    let server = MockHttpServer::start(vec![json_response(json!({
        "choices": [
            {
                "message": {
                    "role": "assistant",
                    "content": "我看到了图片。"
                }
            }
        ]
    }))]);
    let marker_path = temp_marker_file_path("pony-agent-attachment-failure");
    fs::write(&marker_path, "block attachment directory").expect("write marker file");
    let storage_path = marker_path.join("sessions.json");
    let sessions = SessionStore::with_backend(Box::new(FileSessionBackend::new(storage_path)));
    let mut selection = test_provider_selection(server.base_url.clone());
    selection.capabilities.supports_image_input = true;
    let runtime = build_runtime_with_session_store(selection, sessions);
    let result = runtime.run_turn(TurnInput {
        message: "请看这张图".to_string(),
        display_message: None,
        provider_id: None,
        model_id: None,
        reasoning_effort: None,
        workspace_mode: None,
        session_id: Some("attachment-failure".to_string()),
        node_id: None,
        history: Vec::new(),
        images: vec![TurnInputImage {
            data_url: "data:image/png;base64,AAAA".to_string(),
            mime_type: "image/png".to_string(),
            name: Some("diagram.png".to_string()),
        }],
        workspace_id: None,
    });

    assert_eq!(result.phase, "failed");
    assert!(result
        .assistant_message
        .contains("failed to create attachment directory"));
    let snapshot = runtime.load_session_snapshot(Some("attachment-failure"));
    // 失败轮次会以 [用户消息, 失败原因] 两条可见历史写入会话，
    // 与 session.rs 的 file_backend_roundtrip_persists_failed_turn_into_visible_history 保持一致。
    assert_eq!(snapshot.history.len(), 2);

    let _ = server.finish();
    let _ = fs::remove_file(&marker_path);
}

#[test]
fn recent_image_recall_requires_latest_user_turn_to_have_attachments() {
    let builder = DefaultTurnContextBuilder;
    let session = SessionSnapshot {
        conversation_id: "recall-session".to_string(),
        title: "新对话".to_string(),
        summary: "".to_string(),
        history: vec![
            TurnHistoryMessage {
                role: "user".to_string(),
                content: "[已附图片 1 张：old.png]".to_string(),
                attachments: vec![SessionAttachment {
                    id: "att-old".to_string(),
                    asset_id: "asset-recall-session-att-old".to_string(),
                    name: Some("old.png".to_string()),
                    mime_type: "image/png".to_string(),
                    relative_path: "recall-session/att-old.dataurl".to_string(),
                    size_bytes: 4,
                    created_at_ms: 1,
                }],
                ..Default::default()
            },
            TurnHistoryMessage {
                role: "assistant".to_string(),
                content: "我看到了旧图。".to_string(),
                attachments: Vec::new(),
                ..Default::default()
            },
            TurnHistoryMessage {
                role: "user".to_string(),
                content: "继续看 runtime.rs。".to_string(),
                attachments: Vec::new(),
                ..Default::default()
            },
            TurnHistoryMessage {
                role: "assistant".to_string(),
                content: "好的。".to_string(),
                attachments: Vec::new(),
                ..Default::default()
            },
        ],
        attachment_assets: Vec::new(),
        provider_native_transcript: Vec::new(),
        turn_trace_history: Vec::new(),
        long_term_memory_entries: Vec::new(),
        memory_write_evidence: Vec::new(),
        memory_write_hook_trace_records: Vec::new(),
        history_state_evidence: Vec::new(),
        history_state_audit_summary: crate::agent::session::HistoryStateAuditSummary::default(),
        run_control_audit_summary: crate::agent::session::build_missing_run_control_audit_summary(),
        turn_count: 2,
        last_referenced_file: None,
        updated_at_ms: 0,
        history_nodes: Vec::new(),
        history_branches: Vec::new(),
        history_cursor: Default::default(),
        resolved_node_id: None,
        latest_node_id: None,
        env_info: None,
        workspace_id: None,
    };

    let retrieved =
        builder.retrieve_context_state("那张图里有什么？", &[], None, &session, None, None);

    assert!(!should_recall_recent_images(&retrieved));
}

#[test]
fn recent_image_recall_uses_retrieved_context_when_latest_user_turn_has_attachments() {
    let builder = DefaultTurnContextBuilder;
    let session = SessionSnapshot {
        conversation_id: "recall-session".to_string(),
        title: "新对话".to_string(),
        summary: "".to_string(),
        history: vec![
            TurnHistoryMessage {
                role: "user".to_string(),
                content: "[已附图片 1 张：diagram.png]".to_string(),
                attachments: vec![SessionAttachment {
                    id: "att-latest".to_string(),
                    asset_id: "asset-recall-session-att-latest".to_string(),
                    name: Some("diagram.png".to_string()),
                    mime_type: "image/png".to_string(),
                    relative_path: "recall-session/att-latest.dataurl".to_string(),
                    size_bytes: 4,
                    created_at_ms: 1,
                }],
                ..Default::default()
            },
            TurnHistoryMessage {
                role: "assistant".to_string(),
                content: "我看到了图。".to_string(),
                attachments: Vec::new(),
                ..Default::default()
            },
        ],
        attachment_assets: Vec::new(),
        provider_native_transcript: Vec::new(),
        turn_trace_history: Vec::new(),
        long_term_memory_entries: Vec::new(),
        memory_write_evidence: Vec::new(),
        memory_write_hook_trace_records: Vec::new(),
        history_state_evidence: Vec::new(),
        history_state_audit_summary: crate::agent::session::HistoryStateAuditSummary::default(),
        run_control_audit_summary: crate::agent::session::build_missing_run_control_audit_summary(),
        turn_count: 1,
        last_referenced_file: None,
        updated_at_ms: 0,
        history_nodes: Vec::new(),
        history_branches: Vec::new(),
        history_cursor: Default::default(),
        resolved_node_id: None,
        latest_node_id: None,
        env_info: None,
        workspace_id: None,
    };

    let retrieved =
        builder.retrieve_context_state("继续看这张图里有什么？", &[], None, &session, None, None);

    assert!(should_recall_recent_images(&retrieved));
}

#[test]
fn run_turn_records_first_token_latency_for_reasoning_decision() {
    let server = MockHttpServer::start(vec![json_response(json!({
        "choices": [
            {
                "message": {
                    "role": "assistant",
                    "content": "最终答案。",
                    "reasoning_content": "先想一下。"
                }
            }
        ]
    }))]);
    let runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));

    let result = runtime.run_turn(TurnInput {
        message: "请直接回答。".to_string(),
        display_message: None,
        provider_id: None,
        model_id: None,
        reasoning_effort: None,
        workspace_mode: None,
        session_id: Some("sync-reasoning-latency".to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
    });

    assert_eq!(result.phase, "ready");
    assert_eq!(result.assistant_message, "最终答案。");
    assert_eq!(result.first_token_latency_ms, None);
    assert!(result
        .provider_call_records
        .iter()
        .any(|record| record.turn_duration_ms.is_some()));

    let _ = server.finish();
}

#[test]
fn run_turn_measures_first_token_latency_from_turn_start_across_tool_hops() {
    let final_text = "同步工具调用后返回最终答案。";
    let server = MockHttpServer::start(vec![
        json_response(decision_tool_call(
            "workspace_list_files",
            json!({"path": ".", "limit": 40}),
        )),
        json_response(json!({
            "choices": [
                {
                    "message": {
                        "role": "assistant",
                        "content": final_text
                    }
                }
            ],
            "usage": {
                "prompt_tokens": 40,
                "completion_tokens": 20,
                "total_tokens": 60
            }
        })),
    ]);
    let runtime = build_runtime_for_test_with_tool_executor(
        test_provider_selection(server.base_url.clone()),
        Box::new(SlowToolExecutor { delay_ms: 40 }),
    );

    let result = runtime.run_turn(TurnInput {
        message: "先调用工具再同步回答".to_string(),
        display_message: None,
        provider_id: None,
        model_id: None,
        reasoning_effort: None,
        workspace_mode: None,
        session_id: Some("sync-tool-hop-latency".to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
    });

    assert_eq!(result.assistant_message, final_text);
    assert_eq!(result.first_token_latency_ms, None);
    assert!(result
        .provider_call_records
        .iter()
        .all(|record| record.latency_kind == ProviderLatencyKind::BufferedResponse));

    let _ = server.finish();
}

#[test]
fn run_turn_completes_multi_hop_tool_followups_in_single_turn() {
    let final_text = "tauri.conf.json 的第 3 行是 `\"productName\": \"Pony Agent\",`。";
    let server = MockHttpServer::start(vec![
        json_response(decision_tool_call(
            "workspace_list_files",
            json!({"path": ".", "limit": 40}),
        )),
        json_response(decision_tool_call(
            "workspace_read_file",
            json!({"path": "tauri.conf.json"}),
        )),
        json_response(json!({
            "choices": [
                {
                    "message": {
                        "role": "assistant",
                        "content": final_text,
                        "reasoning_content": "已完成文件读取。"
                    }
                }
            ]
        })),
    ]);
    let runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));

    let result = runtime.run_turn(TurnInput {
        message: "tauri.conf.json 第三行是什么？".to_string(),
        display_message: None,
        provider_id: None,
        model_id: None,
        reasoning_effort: None,
        workspace_mode: None,
        session_id: Some("sync-multi-hop".to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
    });
    let request_bodies = server.finish();

    assert_eq!(result.phase, "ready");
    assert_eq!(result.assistant_message, final_text);
    assert_eq!(result.tool_activities.len(), 2);
    assert_eq!(request_bodies.len(), 3);
    assert!(request_bodies[1].contains("\"tool_choice\":\"auto\""));
    assert!(request_bodies[2].contains("\"tool_choice\":\"auto\""));

    let snapshot = runtime.load_session_snapshot(Some("sync-multi-hop"));
    assert_eq!(snapshot.provider_native_transcript.len(), 6);
    assert_eq!(
        snapshot.provider_native_transcript[1]
            .get("tool_calls")
            .and_then(serde_json::Value::as_array)
            .map(|calls| calls.len()),
        Some(1)
    );
    assert_eq!(
        snapshot.provider_native_transcript[3]
            .get("tool_calls")
            .and_then(serde_json::Value::as_array)
            .map(|calls| calls.len()),
        Some(1)
    );
}

#[test]
fn run_turn_accumulates_token_usage_across_tool_followups() {
    let final_text = "已累计整轮 token usage。";
    let server = MockHttpServer::start(vec![
        json_response(json!({
            "choices": [
                {
                    "message": {
                        "role": "assistant",
                        "content": "先调用 workspace_list_files。",
                        "reasoning_content": "需要先执行 workspace_list_files。",
                        "tool_calls": [
                            {
                                "id": "call_workspace_list_files",
                                "type": "function",
                                "function": {
                                    "name": "workspace_list_files",
                                    "arguments": json!({"path": ".", "limit": 40}).to_string()
                                }
                            }
                        ]
                    }
                }
            ],
            "usage": {
                "prompt_tokens": 100,
                "prompt_cache_hit_tokens": 30,
                "prompt_cache_miss_tokens": 70,
                "completion_tokens": 20,
                "total_tokens": 120,
                "completion_tokens_details": {
                    "reasoning_tokens": 7
                }
            }
        })),
        json_response(json!({
            "choices": [
                {
                    "message": {
                        "role": "assistant",
                        "content": "继续调用 workspace_read_file。",
                        "reasoning_content": "目录已找到，继续读取文件。",
                        "tool_calls": [
                            {
                                "id": "call_workspace_read_file",
                                "type": "function",
                                "function": {
                                    "name": "workspace_read_file",
                                    "arguments": json!({"path": "tauri.conf.json"}).to_string()
                                }
                            }
                        ]
                    }
                }
            ],
            "usage": {
                "prompt_tokens": 80,
                "prompt_cache_hit_tokens": 25,
                "prompt_cache_miss_tokens": 55,
                "completion_tokens": 10,
                "total_tokens": 90,
                "completion_tokens_details": {
                    "reasoning_tokens": 3
                }
            }
        })),
        json_response(json!({
            "choices": [
                {
                    "message": {
                        "role": "assistant",
                        "content": final_text,
                        "reasoning_content": "已经整理完最终结果。"
                    }
                }
            ],
            "usage": {
                "prompt_tokens": 60,
                "prompt_cache_hit_tokens": 15,
                "prompt_cache_miss_tokens": 45,
                "completion_tokens": 40,
                "total_tokens": 100,
                "completion_tokens_details": {
                    "reasoning_tokens": 2
                }
            }
        })),
    ]);
    let runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));

    let result = runtime.run_turn(TurnInput {
        message: "继续读取 tauri.conf.json 第三行".to_string(),
        display_message: None,
        provider_id: None,
        model_id: None,
        reasoning_effort: None,
        workspace_mode: None,
        session_id: Some("sync-usage-accumulated".to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
    });

    assert_eq!(result.phase, "ready");
    assert_eq!(result.assistant_message, final_text);
    assert_eq!(result.input_tokens, Some(240));
    assert_eq!(result.cache_hit_input_tokens, Some(70));
    assert_eq!(result.reasoning_tokens, Some(12));
    assert_eq!(result.output_tokens, Some(70));
    assert_eq!(result.total_tokens, Some(310));
    let snapshot = runtime.load_session_snapshot(Some("sync-usage-accumulated"));
    let trace = snapshot
        .turn_trace_history
        .last()
        .expect("sync accumulated trace");
    assert_eq!(trace.provider_call_records.len(), 3);
    assert_eq!(
        trace.provider_call_records[0].request_kind,
        ProviderRequestKind::InitialRequest
    );
    assert_eq!(
        trace.provider_call_records[1].request_kind,
        ProviderRequestKind::ToolFollowup
    );
    assert_eq!(
        trace.provider_call_records[2].cache_miss_input_tokens,
        Some(45)
    );

    let _ = server.finish();
}

#[test]
fn run_turn_repairs_blank_tool_name_before_execution() {
    let final_text = "tauri.conf.json 已成功读取。";
    let server = MockHttpServer::start(vec![
        json_response(decision_tool_call(
            "workspace_list_files",
            json!({"path": ".", "limit": 40}),
        )),
        json_response(decision_blank_tool_call(json!({
            "path": "tauri.conf.json"
        }))),
        json_response(json!({
            "choices": [
                {
                    "message": {
                        "role": "assistant",
                        "content": final_text,
                        "reasoning_content": "空工具名已修复并继续执行。"
                    }
                }
            ]
        })),
    ]);
    let runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));

    let result = runtime.run_turn(TurnInput {
        message: "继续查看 tauri.conf.json".to_string(),
        display_message: None,
        provider_id: None,
        model_id: None,
        reasoning_effort: None,
        workspace_mode: None,
        session_id: Some("repair-blank-tool-sync".to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
    });

    assert_eq!(result.phase, "ready");
    assert_eq!(result.assistant_message, final_text);
    assert_eq!(result.tool_activities.len(), 2);
    assert!(result
        .tool_activities
        .iter()
        .any(|activity| activity.name == "Read"));

    let snapshot = runtime.load_session_snapshot(Some("repair-blank-tool-sync"));
    assert_eq!(
        snapshot.provider_native_transcript[3]
            .get("tool_calls")
            .and_then(serde_json::Value::as_array)
            .and_then(|calls| calls.first())
            .and_then(|call| call.get("function"))
            .and_then(|function| function.get("name"))
            .and_then(serde_json::Value::as_str),
        Some("workspace_read_file")
    );

    let _ = server.finish();
}

#[test]
fn tool_hop_limit_uses_default_when_env_is_missing() {
    assert_eq!(
        parse_max_tool_hops_per_turn(None),
        DEFAULT_MAX_TOOL_HOPS_PER_TURN
    );
}

#[test]
fn tool_hop_limit_accepts_reasonable_env_override() {
    assert_eq!(parse_max_tool_hops_per_turn(Some("24")), 24);
    assert_eq!(parse_max_tool_hops_per_turn(Some("1000")), 1000);
}

#[test]
fn tool_hop_limit_rejects_invalid_env_values() {
    assert_eq!(
        parse_max_tool_hops_per_turn(Some("0")),
        DEFAULT_MAX_TOOL_HOPS_PER_TURN
    );
    assert_eq!(
        parse_max_tool_hops_per_turn(Some("5000")),
        DEFAULT_MAX_TOOL_HOPS_PER_TURN
    );
    assert_eq!(
        parse_max_tool_hops_per_turn(Some("not-a-number")),
        DEFAULT_MAX_TOOL_HOPS_PER_TURN
    );
}

#[test]
fn tool_followup_limit_uses_default_when_env_is_missing() {
    assert_eq!(
        parse_max_tool_followups_per_turn(None),
        DEFAULT_MAX_TOOL_FOLLOWUPS_PER_TURN
    );
}

#[test]
fn tool_followup_limit_accepts_reasonable_env_override() {
    assert_eq!(parse_max_tool_followups_per_turn(Some("3")), 3);
    assert_eq!(parse_max_tool_followups_per_turn(Some("12")), 12);
}

#[test]
fn tool_followup_limit_rejects_invalid_env_values() {
    assert_eq!(
        parse_max_tool_followups_per_turn(Some("0")),
        DEFAULT_MAX_TOOL_FOLLOWUPS_PER_TURN
    );
    assert_eq!(
        parse_max_tool_followups_per_turn(Some("64")),
        DEFAULT_MAX_TOOL_FOLLOWUPS_PER_TURN
    );
    assert_eq!(
        parse_max_tool_followups_per_turn(Some("not-a-number")),
        DEFAULT_MAX_TOOL_FOLLOWUPS_PER_TURN
    );
}

// ── consecutive tool failure stop-loss ───────────────────────────────────────────────────

#[test]
fn consecutive_failure_limit_uses_default_when_env_is_missing() {
    assert_eq!(
        parse_max_consecutive_tool_failures_per_turn(None),
        DEFAULT_MAX_CONSECUTIVE_TOOL_FAILURES_PER_TURN
    );
}

#[test]
fn consecutive_failure_limit_accepts_reasonable_env_override() {
    assert_eq!(parse_max_consecutive_tool_failures_per_turn(Some("3")), 3);
    assert_eq!(parse_max_consecutive_tool_failures_per_turn(Some("16")), 16);
}

#[test]
fn consecutive_failure_limit_rejects_invalid_env_values() {
    assert_eq!(
        parse_max_consecutive_tool_failures_per_turn(Some("0")),
        DEFAULT_MAX_CONSECUTIVE_TOOL_FAILURES_PER_TURN
    );
    assert_eq!(
        parse_max_consecutive_tool_failures_per_turn(Some("32")),
        DEFAULT_MAX_CONSECUTIVE_TOOL_FAILURES_PER_TURN
    );
    assert_eq!(
        parse_max_consecutive_tool_failures_per_turn(Some("not-a-number")),
        DEFAULT_MAX_CONSECUTIVE_TOOL_FAILURES_PER_TURN
    );
}

#[test]
fn consecutive_failure_tracker_counts_same_signal_and_resets_otherwise() {
    let mut tracker = ConsecutiveFailureTracker::new();
    let signal = Some((
        "workspace_read_file".to_string(),
        "invalid_path".to_string(),
    ));
    assert_eq!(tracker.record(signal.clone()), 1);
    assert_eq!(tracker.record(signal.clone()), 2);
    assert_eq!(tracker.record(signal.clone()), 3);
    // 同一工具、不同错误码：重新计数
    assert_eq!(
        tracker.record(Some((
            "workspace_read_file".to_string(),
            "not_found".to_string()
        ))),
        1
    );
    // 不同工具、同一错误码：重新计数
    assert_eq!(tracker.record(signal.clone()), 1);
    // 成功/无法分类的结果：清零
    assert_eq!(tracker.record(None), 0);
    assert_eq!(tracker.record(signal), 1);
}

#[test]
fn tool_failure_signal_skips_success_and_pending_control() {
    let call = ToolCall {
        call_id: None,
        name: "workspace_read_file".to_string(),
        arguments: json!({}),
        plan: None,
    };
    // ok → None
    assert_eq!(
        tool_failure_signal(
            &call,
            &ToolResult {
                tool_name: "workspace_read_file".to_string(),
                status: "ok".to_string(),
                output: "{}".to_string(),
                duration_ms: 0,
            }
        ),
        None
    );
    // error + code → Some((tool, code))
    let failing = ToolResult {
        tool_name: "workspace_read_file".to_string(),
        status: "error".to_string(),
        output: json!({
            "error": { "code": "invalid_path", "message": "路径不存在" }
        })
        .to_string(),
        duration_ms: 0,
    };
    assert_eq!(
        tool_failure_signal(&call, &failing),
        Some((
            "workspace_read_file".to_string(),
            "invalid_path".to_string()
        ))
    );
    // control_outcome_pending（Ask 挂起）不计失败
    let pending = ToolResult {
        tool_name: "Ask".to_string(),
        status: "error".to_string(),
        output: json!({
            "error": { "code": "control_outcome_pending", "message": "pending" }
        })
        .to_string(),
        duration_ms: 0,
    };
    assert_eq!(tool_failure_signal(&call, &pending), None);
    // 非结构化错误 output → None
    assert_eq!(
        tool_failure_signal(
            &call,
            &ToolResult {
                tool_name: "workspace_read_file".to_string(),
                status: "error".to_string(),
                output: "boom".to_string(),
                duration_ms: 0,
            }
        ),
        None
    );
}

#[test]
fn consecutive_failure_error_message_includes_tool_code_and_details() {
    let message = build_consecutive_tool_failure_error(
        3,
        "workspace_read_file",
        "invalid_path",
        "{\"error\":{\"code\":\"invalid_path\",\"message\":\"路径不存在\"}}",
    );
    assert!(message.contains("workspace_read_file"));
    assert!(message.contains("invalid_path"));
    assert!(message.contains("路径不存在"));
    assert!(message.contains("PONY_AGENT_MAX_CONSECUTIVE_TOOL_FAILURES_PER_TURN"));
}

#[test]
fn run_turn_stops_followup_after_consecutive_identical_tool_failures() {
    // 固定失败的 executor：同一工具永远返回同一错误码。
    struct AlwaysFailingExecutor;
    impl crate::agent::tools::ToolExecutor for AlwaysFailingExecutor {
        fn execute(&self, call: &ToolCall) -> crate::agent::tools::ToolResult {
            crate::agent::tools::ToolResult {
                tool_name: call.name.clone(),
                status: "error".to_string(),
                output: json!({
                    "ok": false,
                    "tool": call.name,
                    "error": {
                        "code": "invalid_path",
                        "message": "目标路径不存在，请检查后重试。"
                    }
                })
                .to_string(),
                duration_ms: 1,
            }
        }
    }

    let server = MockHttpServer::start(vec![
        json_response(decision_tool_call(
            "workspace_read_file",
            json!({ "path": "missing-a.txt", "description": "读取文件" }),
        )),
        json_response(decision_tool_call(
            "workspace_read_file",
            json!({ "path": "missing-b.txt", "description": "换个路径重试" }),
        )),
        json_response(decision_tool_call(
            "workspace_read_file",
            json!({ "path": "missing-c.txt", "description": "再试一次" }),
        )),
    ]);
    let runtime = build_runtime_for_test_with_tool_executor(
        test_provider_selection(server.base_url.clone()),
        Box::new(AlwaysFailingExecutor),
    );

    let result = runtime.run_turn(TurnInput {
        message: "读取 missing 文件".to_string(),
        display_message: None,
        provider_id: None,
        model_id: None,
        reasoning_effort: None,
        workspace_mode: None,
        session_id: Some("consecutive-failure-stop-loss".to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
    });

    assert_eq!(result.phase, "failed");
    assert!(
        result.assistant_message.contains("连续"),
        "应包含止损文案，实际：{}",
        result.assistant_message
    );
    assert!(result.assistant_message.contains("invalid_path"));
    assert!(result
        .assistant_message
        .contains("目标路径不存在，请检查后重试。"));
    // 第 3 次失败后止损：不再向 provider 发起第 4 次 follow-up。
    let requests = server.finish();
    assert_eq!(requests.len(), 3);
}

#[test]
fn run_turn_resets_consecutive_failure_counter_on_success() {
    let server = MockHttpServer::start(vec![
        json_response(decision_tool_call(
            "workspace_read_file",
            json!({ "path": "Windows/System32/a", "description": "越界路径" }),
        )),
        json_response(decision_tool_call(
            "workspace_read_file",
            json!({ "path": "tauri.conf.json", "description": "正常读取" }),
        )),
        json_response(decision_tool_call(
            "workspace_read_file",
            json!({ "path": "Windows/System32/b", "description": "越界重试一" }),
        )),
        json_response(decision_tool_call(
            "workspace_read_file",
            json!({ "path": "Windows/System32/c", "description": "越界重试二" }),
        )),
        json_response(json!({
            "choices": [
                {
                    "message": {
                        "role": "assistant",
                        "content": "已按当前可用信息完成处理。",
                        "reasoning_content": "两次失败不连续，未触发止损，正常收尾。"
                    }
                }
            ]
        })),
    ]);
    let runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));

    let result = runtime.run_turn(TurnInput {
        message: "读取文件并继续".to_string(),
        display_message: None,
        provider_id: None,
        model_id: None,
        reasoning_effort: None,
        workspace_mode: None,
        session_id: Some("consecutive-failure-reset".to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
    });

    // 失败-成功-失败-失败：没有连续 3 次同信号失败，turn 正常完成。
    assert_eq!(result.phase, "ready");
    assert_eq!(result.assistant_message, "已按当前可用信息完成处理。");
    assert_eq!(result.tool_activities.len(), 4);
    let requests = server.finish();
    assert_eq!(requests.len(), 5);
}

#[test]
fn tool_call_signature_normalizes_object_key_order() {
    let left = ToolCall {
        call_id: None,
        name: "workspace_list_files".to_string(),
        arguments: json!({"path":"src/agent","limit":20}),
        plan: None,
    };
    let right = ToolCall {
        call_id: None,
        name: "workspace_list_files".to_string(),
        arguments: json!({"limit":20,"path":"src/agent"}),
        plan: None,
    };

    assert_eq!(tool_call_signature(&left), tool_call_signature(&right));
}

#[test]
fn stream_reasoning_batcher_batches_until_threshold() {
    let mut batcher = StreamReasoningBatcher::default();
    assert!(batcher.push("abc".repeat(10)).is_none());
    let chunk = batcher
        .push("d".repeat(STREAM_REASONING_BATCH_CHARS))
        .unwrap();
    assert!(chunk.chars().count() >= STREAM_REASONING_BATCH_CHARS);
    assert!(batcher.flush().is_none());
}

#[test]
fn start_turn_stream_completes_after_multi_hop_followup_stream() {
    let _rt_guard = crate::agent::runtime_helper::TestRuntimeGuard::new();
    let final_text = "tauri.conf.json 的第 3 行是 `\"productName\": \"Pony Agent\",`。";
    let server = MockHttpServer::start(vec![
        sse_response(&[json!({
            "choices": [
                {
                    "delta": {
                        "reasoning_content": "需要先执行 workspace_list_files。",
                        "tool_calls": [
                            {
                                "index": 0,
                                "id": "call_workspace_list_files",
                                "type": "function",
                                "function": {
                                    "name": "workspace_list_files",
                                    "arguments": "{\"path\":\".\",\"limit\":40}"
                                }
                            }
                        ]
                    }
                }
            ]
        })]),
        sse_response(&[
            json!({
                "choices": [
                    {
                        "delta": {
                            "reasoning_content": "已经找到 tauri.conf.json。"
                        }
                    }
                ]
            }),
            json!({
                "choices": [
                    {
                        "delta": {
                            "content": "找到了！让我读取 tauri.conf.json 的内容："
                        }
                    }
                ]
            }),
            json!({
                "choices": [
                    {
                        "delta": {
                            "tool_calls": [
                                {
                                    "index": 0,
                                    "id": "call_read_file",
                                    "type": "function",
                                    "function": {
                                        "name": "workspace_read_file",
                                        "arguments": "{\"path\":\"tauri"
                                    }
                                }
                            ]
                        }
                    }
                ]
            }),
            json!({
                "choices": [
                    {
                        "delta": {
                            "tool_calls": [
                                {
                                    "index": 0,
                                    "function": {
                                        "arguments": ".conf.json\"}"
                                    }
                                }
                            ]
                        }
                    }
                ]
            }),
        ]),
        sse_response(&[
            json!({
                "choices": [
                    {
                        "delta": {
                            "reasoning_content": "已读取目标文件。"
                        }
                    }
                ]
            }),
            json!({
                "choices": [
                    {
                        "delta": {
                            "content": final_text
                        }
                    }
                ]
            }),
        ]),
    ]);
    let mut runtime = AgentRuntime::with_dependencies(
        SessionStore::memory_only(),
        Box::new(StaticResolver {
            selection: test_provider_selection(server.base_url.clone()),
        }),
        Box::new(StubToolExecutor),
        Box::new(SlowPassthroughPlanner { delay_ms: 40 }),
        Box::new(DefaultTurnContextBuilder),
        Box::new(DefaultTurnTelemetryBuilder),
    );
    runtime
        .register_hook_descriptor(observe_hook_descriptor(
            "observe.model",
            10,
            TurnHookPoint::ModelCallStart,
        ))
        .expect("register model hook");
    runtime
        .register_hook_descriptor(observe_hook_descriptor(
            "observe.tool-start",
            20,
            TurnHookPoint::ToolCallStart,
        ))
        .expect("register tool-start hook");
    runtime
        .register_hook_descriptor(observe_hook_descriptor(
            "observe.tool-end",
            30,
            TurnHookPoint::ToolCallEnd,
        ))
        .expect("register tool-end hook");
    let sink = RecordingTurnEventSink::new();

    runtime.start_turn_stream(
        &sink,
        "turn-multi-hop".to_string(),
        TurnInput {
            message: "继续读取 tauri.conf.json 第三行".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            workspace_mode: None,
            session_id: Some("stream-multi-hop".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
        },
    );
    let request_bodies = server.finish();
    let events = sink.events.borrow();
    let completed = events
        .iter()
        .find_map(|(name, payload)| {
            if name == "turn:completed" {
                Some(payload.clone())
            } else {
                None
            }
        })
        .expect("completed event");
    let tool_completed = events
        .iter()
        .find_map(|(name, payload)| {
            (name == "turn:tool"
                && payload.event_type.as_deref() == Some("turn.tool_call_completed"))
            .then_some(payload.clone())
        })
        .expect("tool completed event");
    let checkpoint_phase_changed = events
        .iter()
        .find_map(|(name, payload)| {
            (name == "turn:phase_changed"
                && payload.event_type.as_deref() == Some("turn.phase_changed")
                && payload.phase.as_deref() == Some("checkpointing"))
            .then_some(payload.clone())
        })
        .expect("checkpoint phase changed event");
    let checkpoint_persisted = events
        .iter()
        .find_map(|(name, payload)| {
            (name == "turn:checkpoint_persisted"
                && payload.event_type.as_deref() == Some("turn.checkpoint_persisted"))
            .then_some(payload.clone())
        })
        .expect("checkpoint persisted event");
    let tool_started = events
        .iter()
        .find_map(|(name, payload)| {
            (name == "turn:tool" && payload.event_type.as_deref() == Some("turn.tool_call_started"))
                .then_some(payload.clone())
        })
        .expect("tool started event");
    let model_started_events: Vec<TurnStreamEvent> = events
        .iter()
        .filter_map(|(name, payload)| {
            (name == "turn:trace"
                && payload.event_type.as_deref() == Some("turn.model_call_started"))
            .then_some(payload.clone())
        })
        .collect();

    assert_eq!(completed.text.as_deref(), Some(final_text));
    assert!(model_started_events.len() >= 2);
    assert!(model_started_events.iter().all(|payload| {
        payload
            .hook_trace_records
            .as_ref()
            .map(|records| {
                records.iter().any(|record| {
                    record.hook_name == "observe.model"
                        && record.hook_point == TurnHookPoint::ModelCallStart
                })
            })
            .unwrap_or(false)
    }));
    assert_hook_boundary_alignment(
        &tool_started,
        TurnHookPoint::ToolCallStart,
        "turn.tool_call_started",
        "executing_tool",
    );
    assert_eq!(
        tool_started
            .hook_trace_records
            .as_ref()
            .map(|records| records.len()),
        Some(1)
    );
    assert_eq!(
        tool_started
            .hook_trace_records
            .as_ref()
            .and_then(|records| records.first())
            .map(|record| record.hook_name.as_str()),
        Some("observe.tool-start")
    );
    assert_hook_boundary_alignment(
        &checkpoint_phase_changed,
        TurnHookPoint::CheckpointPersistStart,
        "turn.phase_changed",
        "checkpointing",
    );
    assert_hook_boundary_alignment(
        &checkpoint_persisted,
        TurnHookPoint::CheckpointPersistEnd,
        "turn.checkpoint_persisted",
        "checkpointing",
    );
    assert_hook_boundary_alignment(
        &completed,
        TurnHookPoint::TurnFinalizeEnd,
        "turn.completed",
        "completed",
    );
    assert_hook_boundary_alignment(
        &tool_completed,
        TurnHookPoint::ToolCallEnd,
        "turn.tool_call_completed",
        "tool_result_integrating",
    );
    assert_eq!(
        tool_completed
            .hook_trace_records
            .as_ref()
            .map(|records| records.len()),
        Some(1)
    );
    assert_eq!(
        tool_completed
            .hook_trace_records
            .as_ref()
            .and_then(|records| records.first())
            .map(|record| record.hook_name.as_str()),
        Some("observe.tool-end")
    );
    assert!(
        checkpoint_phase_changed.sequence.unwrap_or_default()
            < checkpoint_persisted.sequence.unwrap_or_default()
    );
    assert!(
        checkpoint_persisted.sequence.unwrap_or_default() < completed.sequence.unwrap_or_default()
    );
    assert_eq!(
        completed
            .trace_timeline
            .as_ref()
            .and_then(|timeline| timeline.last())
            .map(|entry| entry.kind.as_str()),
        Some("checkpoint_persist")
    );
    assert_eq!(
        completed
            .tool_activities
            .as_ref()
            .map(|activities| activities.len()),
        Some(2)
    );
    assert!(events.iter().any(|(name, payload)| {
        name == "turn:tool"
            && payload
                .tool_activities
                .as_ref()
                .map(|activities| activities.iter().any(|activity| activity.name == "Read"))
                .unwrap_or(false)
    }));
    assert_eq!(request_bodies.len(), 3);
    assert!(request_bodies[1].contains("\"tool_choice\":\"auto\""));
    assert!(request_bodies[2].contains("\"tool_choice\":\"auto\""));
}

#[test]
fn start_turn_stream_fails_with_canonical_finalize_boundary_when_tool_hop_limit_is_hit() {
    let _limit_guard = ToolHopLimitOverrideGuard::set(1);
    let server = MockHttpServer::start(vec![
        sse_decision_tool_call("workspace_list_files", json!({"path": ".", "limit": 40})),
        sse_response(&[
            json!({
                "choices": [
                    {
                        "delta": {
                            "reasoning_content": "已经找到目标文件。"
                        }
                    }
                ]
            }),
            json!({
                "choices": [
                    {
                        "delta": {
                            "tool_calls": [
                                {
                                    "index": 0,
                                    "id": "call_read_file",
                                    "type": "function",
                                    "function": {
                                        "name": "workspace_read_file",
                                        "arguments": "{\"path\":\"tauri.conf.json\"}"
                                    }
                                }
                            ]
                        }
                    }
                ]
            }),
        ]),
    ]);
    let mut runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));
    let sink = RecordingTurnEventSink::new();

    runtime.start_turn_stream(
        &sink,
        "turn-tool-hop-limit".to_string(),
        TurnInput {
            message: "请连续调用两个工具".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            workspace_mode: None,
            session_id: Some("stream-tool-hop-limit".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
        },
    );

    let request_bodies = server.finish();
    let events = sink.events.borrow();
    let failed = events
        .iter()
        .find_map(|(name, payload)| (name == "turn:failed").then_some(payload.clone()))
        .expect("failed event");
    let tool_completed = events
        .iter()
        .find_map(|(name, payload)| {
            (name == "turn:tool"
                && payload.event_type.as_deref() == Some("turn.tool_call_completed"))
            .then_some(payload.clone())
        })
        .expect("tool completed event before hop-limit failure");
    let expected_error = build_tool_hop_limit_error(1);

    assert_hook_boundary_alignment(
        &failed,
        TurnHookPoint::TurnFinalizeEnd,
        "turn.failed",
        "failed",
    );
    assert_hook_boundary_alignment(
        &tool_completed,
        TurnHookPoint::ToolCallEnd,
        "turn.tool_call_completed",
        "tool_result_integrating",
    );
    assert_eq!(failed.error.as_deref(), Some(expected_error.as_str()));
    assert_eq!(
        failed
            .tool_activities
            .as_ref()
            .map(|activities| activities.len()),
        Some(1)
    );
    assert_eq!(request_bodies.len(), 2);

    let snapshot = runtime.load_session_snapshot(Some("stream-tool-hop-limit"));
    let persisted_trace = snapshot
        .turn_trace_history
        .last()
        .expect("failed trace should be persisted");
    assert_eq!(persisted_trace.phase, "failed");
    assert_eq!(
        persisted_trace.error.as_deref(),
        Some(expected_error.as_str())
    );
    assert_eq!(persisted_trace.tool_activities.len(), 1);
}

#[test]
fn start_turn_stream_cancels_with_canonical_finalize_boundary_when_stop_is_requested_during_tool_execution(
) {
    let server = MockHttpServer::start(vec![sse_decision_tool_call(
        "workspace_list_files",
        json!({"path": ".", "limit": 40}),
    )]);
    let control = Arc::new(ExecutionControlRegistry::new());
    let turn_id = "turn-cancel-during-tool".to_string();
    control.register_turn(&turn_id, Some("stream-cancel-during-tool"), None);

    let runtime = build_runtime_for_test_with_tool_executor(
        test_provider_selection(server.base_url.clone()),
        Box::new(RequestStopToolExecutor {
            control: Arc::clone(&control),
            turn_id: turn_id.clone(),
            delay_ms: 30,
        }),
    );
    let sink = RecordingTurnEventSink::new();

    runtime.start_turn_stream_with_control(
        &sink,
        &control,
        turn_id.clone(),
        TurnInput {
            message: "先调用工具，然后我会中止".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            workspace_mode: None,
            session_id: Some("stream-cancel-during-tool".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
        },
    );

    let request_bodies = server.finish();
    let events = sink.events.borrow();
    let cancelled = events
        .iter()
        .find_map(|(name, payload)| (name == "turn:cancelled").then_some(payload.clone()))
        .expect("cancelled event");
    let tool_started = events
        .iter()
        .find_map(|(name, payload)| {
            (name == "turn:tool" && payload.event_type.as_deref() == Some("turn.tool_call_started"))
                .then_some(payload.clone())
        })
        .expect("tool started event");

    assert_hook_boundary_alignment(
        &tool_started,
        TurnHookPoint::ToolCallStart,
        "turn.tool_call_started",
        "executing_tool",
    );
    assert_hook_boundary_alignment(
        &cancelled,
        TurnHookPoint::TurnFinalizeEnd,
        "turn.cancelled",
        "cancelled",
    );
    assert_eq!(cancelled.error.as_deref(), Some("stopped_by_user"));
    assert_eq!(
        cancelled
            .tool_activities
            .as_ref()
            .map(|activities| activities.len()),
        Some(1)
    );
    assert!(!events.iter().any(|(name, payload)| {
        name == "turn:tool" && payload.event_type.as_deref() == Some("turn.tool_call_completed")
    }));
    assert_eq!(request_bodies.len(), 1);

    let snapshot = runtime.load_session_snapshot(Some("stream-cancel-during-tool"));
    assert_eq!(
        snapshot
            .history
            .last()
            .map(|message| message.content.as_str()),
        Some(CANCELLED_TURN_MESSAGE)
    );
    let persisted_trace = snapshot
        .turn_trace_history
        .last()
        .expect("cancelled trace should be persisted");
    assert_eq!(persisted_trace.phase, "cancelled");
    assert_eq!(persisted_trace.error.as_deref(), Some("stopped_by_user"));
    assert_eq!(persisted_trace.tool_activities.len(), 1);
}

#[test]
fn start_turn_stream_preserves_tool_error_activity_when_tool_execution_errors() {
    let server = MockHttpServer::start(vec![sse_decision_tool_call(
        "workspace_list_files",
        json!({"path": ".", "limit": 40}),
    )]);
    let mut runtime = build_runtime_for_test_with_tool_executor(
        test_provider_selection(server.base_url.clone()),
        Box::new(ErrorToolExecutor),
    );
    let sink = RecordingTurnEventSink::new();

    runtime.start_turn_stream(
        &sink,
        "turn-tool-error".to_string(),
        TurnInput {
            message: "调用工具但让它失败".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            workspace_mode: None,
            session_id: Some("stream-tool-error".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
        },
    );

    let request_bodies = server.finish();
    let events = sink.events.borrow();
    let tool_started = events
        .iter()
        .find_map(|(name, payload)| {
            (name == "turn:tool" && payload.event_type.as_deref() == Some("turn.tool_call_started"))
                .then_some(payload.clone())
        })
        .expect("tool started event");
    let tool_completed = events
        .iter()
        .find_map(|(name, payload)| {
            (name == "turn:tool"
                && payload.event_type.as_deref() == Some("turn.tool_call_completed"))
            .then_some(payload.clone())
        })
        .expect("tool completed event");
    let completed = events
        .iter()
        .find_map(|(name, payload)| (name == "turn:completed").then_some(payload.clone()))
        .expect("completed event");
    assert_hook_boundary_alignment(
        &tool_started,
        TurnHookPoint::ToolCallStart,
        "turn.tool_call_started",
        "executing_tool",
    );
    assert_hook_boundary_alignment(
        &tool_completed,
        TurnHookPoint::ToolCallEnd,
        "turn.tool_call_completed",
        "tool_result_integrating",
    );
    assert_eq!(
        completed
            .tool_activities
            .as_ref()
            .map(|activities| activities.len()),
        Some(1)
    );
    assert!(events.iter().any(|(name, _)| name == "turn:completed"));
    assert!(!events.iter().any(|(name, _)| name == "turn:failed"));
    assert_eq!(request_bodies.len(), 1);

    let snapshot = runtime.load_session_snapshot(Some("stream-tool-error"));
    let persisted_trace = snapshot
        .turn_trace_history
        .last()
        .expect("completed trace should be persisted");
    assert_eq!(persisted_trace.phase, "completed");
    assert_eq!(persisted_trace.error.as_deref(), None);
    assert_eq!(persisted_trace.tool_activities.len(), 1);
    let completed_text = completed.text.as_deref().unwrap_or_default();
    assert!(completed_text.contains("provider 在整合工具结果时失败"));
    assert!(completed_text.contains("tool workspace_list_files failed in test"));
    assert!(completed_text.contains("status=error"));
}

#[test]
fn start_turn_stream_emits_first_token_latency_on_reasoning_delta() {
    let server = MockHttpServer::start(vec![sse_response(&[
        json!({
            "choices": [
                {
                    "delta": {
                        "reasoning_content": "先想一下。"
                    }
                }
            ]
        }),
        json!({
            "choices": [
                {
                    "delta": {
                        "content": "最终答案。"
                    }
                }
            ]
        }),
        json!({
            "choices": [],
            "usage": {
                "prompt_tokens": 20,
                "completion_tokens": 6,
                "total_tokens": 26
            }
        }),
    ])]);
    let mut runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));
    let sink = RecordingTurnEventSink::new();

    runtime.start_turn_stream(
        &sink,
        "turn-reasoning-latency".to_string(),
        TurnInput {
            message: "请直接回答。".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            workspace_mode: None,
            session_id: Some("stream-reasoning-latency".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
        },
    );

    let requests = server.finish();
    let events = sink.events.borrow();
    let first_delta = events
        .iter()
        .find_map(|(name, payload)| {
            if name == "turn:delta" {
                Some(payload.clone())
            } else {
                None
            }
        })
        .expect("first delta event");
    let completed = events
        .iter()
        .find_map(|(name, payload)| {
            if name == "turn:completed" {
                Some(payload.clone())
            } else {
                None
            }
        })
        .expect("completed event");
    let checkpoint_phase_changed = events
        .iter()
        .find_map(|(name, payload)| {
            (name == "turn:phase_changed"
                && payload.event_type.as_deref() == Some("turn.phase_changed")
                && payload.phase.as_deref() == Some("checkpointing"))
            .then_some(payload.clone())
        })
        .expect("checkpoint phase changed event");
    let checkpoint_persisted = events
        .iter()
        .find_map(|(name, payload)| {
            (name == "turn:checkpoint_persisted"
                && payload.event_type.as_deref() == Some("turn.checkpoint_persisted"))
            .then_some(payload.clone())
        })
        .expect("checkpoint persisted event");

    assert_eq!(first_delta.reasoning_content.as_deref(), Some("先想一下。"));
    assert_eq!(first_delta.text, None);
    assert!(first_delta.first_token_latency_ms.is_some());
    assert_eq!(
        completed.first_token_latency_ms,
        first_delta.first_token_latency_ms
    );
    assert_hook_boundary_alignment(
        &checkpoint_phase_changed,
        TurnHookPoint::CheckpointPersistStart,
        "turn.phase_changed",
        "checkpointing",
    );
    assert_hook_boundary_alignment(
        &checkpoint_persisted,
        TurnHookPoint::CheckpointPersistEnd,
        "turn.checkpoint_persisted",
        "checkpointing",
    );
    assert!(
        checkpoint_persisted.sequence.unwrap_or_default() < completed.sequence.unwrap_or_default()
    );
    assert_eq!(
        completed
            .trace_timeline
            .as_ref()
            .and_then(|timeline| timeline.last())
            .map(|entry| entry.kind.as_str()),
        Some("checkpoint_persist")
    );
    let decision_request: Value =
        serde_json::from_str(&requests[0]).expect("decision request should be json");
    assert_eq!(
        decision_request.get("stream").and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        completed.provider_source.as_deref(),
        Some("provider_decision_stream")
    );
    let provider_call = completed
        .provider_call_records
        .as_ref()
        .and_then(|records| records.first())
        .expect("initial provider call record");
    assert_eq!(
        provider_call.latency_kind,
        ProviderLatencyKind::ProviderStream
    );
    assert!(provider_call.first_token_latency_ms.is_some());
    assert!(provider_call.turn_duration_ms.is_some());
    assert!(
        provider_call.first_token_latency_ms.unwrap() <= provider_call.turn_duration_ms.unwrap()
    );
    assert!(
        completed.first_token_latency_ms.unwrap() > provider_call.first_token_latency_ms.unwrap()
    );
}

#[test]
fn start_turn_stream_sync_fallback_for_initial_decision_does_not_emit_fake_ttft() {
    let server = MockHttpServer::start(vec![
        json_response(json!({
            "choices": [
                {
                    "message": {
                        "role": "assistant",
                        "content": "最终答案。",
                        "reasoning_content": "先想一下。"
                    }
                }
            ]
        })),
        json_response(json!({
            "choices": [
                {
                    "message": {
                        "role": "assistant",
                        "content": "最终答案。",
                        "reasoning_content": "先想一下。"
                    }
                }
            ]
        })),
    ]);
    let mut runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));
    let sink = RecordingTurnEventSink::new();

    runtime.start_turn_stream(
        &sink,
        "turn-sync-fallback-no-fake-ttft".to_string(),
        TurnInput {
            message: "请直接回答。".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            workspace_mode: None,
            session_id: Some("stream-sync-fallback-no-fake-ttft".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
        },
    );

    let requests = server.finish();
    let events = sink.events.borrow();
    let first_delta = events
        .iter()
        .find_map(|(name, payload)| (name == "turn:delta").then_some(payload.clone()))
        .expect("first delta event");
    let completed = events
        .iter()
        .find_map(|(name, payload)| (name == "turn:completed").then_some(payload.clone()))
        .expect("completed event");

    assert_eq!(first_delta.first_token_latency_ms, None);
    assert_eq!(completed.first_token_latency_ms, None);
    let streamed_decision_request: Value =
        serde_json::from_str(&requests[0]).expect("decision request should be json");
    let fallback_decision_request: Value =
        serde_json::from_str(&requests[1]).expect("fallback decision request should be json");
    assert_eq!(
        streamed_decision_request
            .get("stream")
            .and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        fallback_decision_request
            .get("stream")
            .and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(
        completed.provider_source.as_deref(),
        Some("provider_decision")
    );
}

#[test]
fn start_turn_stream_measures_first_token_latency_from_turn_start_across_tool_hops() {
    let final_text = "工具调用后返回最终答案。";
    let server = MockHttpServer::start(vec![
        sse_response(&[json!({
            "choices": [
                {
                    "delta": {
                        "tool_calls": [
                            {
                                "index": 0,
                                "id": "call_workspace_list_files",
                                "type": "function",
                                "function": {
                                    "name": "workspace_list_files",
                                    "arguments": "{\"path\":\".\",\"limit\":40}"
                                }
                            }
                        ]
                    }
                }
            ]
        })]),
        sse_response(&[
            json!({
                "choices": [
                    {
                        "delta": {
                            "content": final_text
                        }
                    }
                ]
            }),
            json!({
                "choices": [],
                "usage": {
                    "prompt_tokens": 40,
                    "completion_tokens": 20,
                    "total_tokens": 60
                }
            }),
        ]),
    ]);
    let mut runtime = build_runtime_for_test_with_tool_executor(
        test_provider_selection(server.base_url.clone()),
        Box::new(SlowToolExecutor { delay_ms: 40 }),
    );
    let sink = RecordingTurnEventSink::new();

    runtime.start_turn_stream(
        &sink,
        "turn-tool-hop-latency".to_string(),
        TurnInput {
            message: "先调用工具再回答".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            workspace_mode: None,
            session_id: Some("stream-tool-hop-latency".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
        },
    );

    let _ = server.finish();
    let events = sink.events.borrow();
    let first_delta = events
        .iter()
        .find_map(|(name, payload)| (name == "turn:delta").then_some(payload.clone()))
        .expect("first delta event");
    let tool_completed = events
        .iter()
        .find_map(|(name, payload)| {
            (name == "turn:tool"
                && payload.event_type.as_deref() == Some("turn.tool_call_completed"))
            .then_some(payload.clone())
        })
        .expect("tool completed event");
    let completed = events
        .iter()
        .find_map(|(name, payload)| (name == "turn:completed").then_some(payload.clone()))
        .expect("completed event");

    assert!(first_delta.first_token_latency_ms.unwrap_or_default() >= 40);
    assert_eq!(
        completed.first_token_latency_ms,
        first_delta.first_token_latency_ms
    );
    assert_hook_boundary_alignment(
        &tool_completed,
        TurnHookPoint::ToolCallEnd,
        "turn.tool_call_completed",
        "tool_result_integrating",
    );
    assert_hook_boundary_alignment(
        &completed,
        TurnHookPoint::TurnFinalizeEnd,
        "turn.completed",
        "completed",
    );
}

#[test]
fn start_turn_stream_uses_live_stream_for_deepseek_tool_followup() {
    let final_text = "deepseek follow-up completed";
    let server = MockHttpServer::start(vec![
        sse_response(&[
            json!({
                "choices": [
                    {
                        "delta": {
                            "reasoning_content": "need workspace listing before answering"
                        }
                    }
                ]
            }),
            json!({
                "choices": [
                    {
                        "delta": {
                            "content": "call a tool first",
                            "tool_calls": [
                                {
                                    "index": 0,
                                    "id": "call_workspace_list_files",
                                    "type": "function",
                                    "function": {
                                        "name": "workspace_list_files",
                                        "arguments": "{\"path\":\".\",\"limit\":40}"
                                    }
                                }
                            ]
                        }
                    }
                ]
            }),
        ]),
        sse_response(&[
            json!({
                "choices": [
                    {
                        "delta": {
                            "reasoning_content": "tool output is sufficient"
                        }
                    }
                ]
            }),
            json!({
                "choices": [
                    {
                        "delta": {
                            "content": final_text
                        }
                    }
                ]
            }),
            json!({
                "choices": [],
                "usage": {
                    "prompt_tokens": 60,
                    "completion_tokens": 24,
                    "total_tokens": 84
                }
            }),
        ]),
    ]);
    let mut runtime = build_runtime_for_test(deepseek_provider_selection(server.base_url.clone()));
    let sink = RecordingTurnEventSink::new();

    runtime.start_turn_stream(
        &sink,
        "turn-deepseek-followup-compat".to_string(),
        TurnInput {
            message: "read Cargo.toml then answer".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            workspace_mode: None,
            session_id: Some("deepseek-followup-compat".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
        },
    );

    let requests = server.finish();
    let events = sink.events.borrow();
    let text_delta = events
        .iter()
        .filter_map(|(name, payload)| (name == "turn:delta").then_some(payload.clone()))
        .find(|payload| payload.text.as_deref() == Some(final_text))
        .expect("text delta event");
    let completed = events
        .iter()
        .find_map(|(name, payload)| (name == "turn:completed").then_some(payload.clone()))
        .expect("completed event");

    assert_eq!(requests.len(), 2);
    let decision_request: Value =
        serde_json::from_str(&requests[0]).expect("decision request should be json");
    let followup_request: Value =
        serde_json::from_str(&requests[1]).expect("followup request should be json");
    assert_eq!(
        decision_request.get("stream").and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        followup_request.get("stream").and_then(Value::as_bool),
        Some(true)
    );
    assert!(followup_request.get("stream_options").is_some());
    let replayed_assistant_message = followup_request
        .get("messages")
        .and_then(Value::as_array)
        .and_then(|messages| {
            messages.iter().find(|message| {
                message.get("role").and_then(Value::as_str) == Some("assistant")
                    && message
                        .get("tool_calls")
                        .and_then(Value::as_array)
                        .map(|calls| !calls.is_empty())
                        .unwrap_or(false)
            })
        })
        .expect("follow-up request should replay assistant tool call message");
    assert_eq!(
        replayed_assistant_message
            .get("reasoning_content")
            .and_then(Value::as_str),
        Some("need workspace listing before answering")
    );
    assert_eq!(text_delta.text.as_deref(), Some(final_text));
    assert_eq!(completed.phase.as_deref(), Some("completed"));
    assert_eq!(
        completed.provider_source.as_deref(),
        Some("provider_followup_stream")
    );
    assert_eq!(completed.fallback_reason, None);
    let provider_calls = completed
        .provider_call_records
        .as_ref()
        .expect("provider call records");
    assert_eq!(provider_calls.len(), 2);
    assert!(provider_calls
        .iter()
        .all(|record| record.latency_kind == ProviderLatencyKind::ProviderStream));
    assert!(provider_calls
        .iter()
        .all(|record| record.first_token_latency_ms.is_some()));
}

#[test]
fn start_turn_stream_streams_duplicate_tool_recovery_answer() {
    let final_text = "根据已读取的内容，tauri.conf.json 中 productName 是 Pony Agent。";
    let duplicate_args = "{\"path\":\"tauri.conf.json\"}";
    let server = MockHttpServer::start(vec![
        sse_response(&[json!({
            "choices": [
                {
                    "delta": {
                        "reasoning_content": "先读取配置文件。",
                        "tool_calls": [
                            {
                                "index": 0,
                                "id": "call_workspace_read_file",
                                "type": "function",
                                "function": {
                                    "name": "workspace_read_file",
                                    "arguments": duplicate_args
                                }
                            }
                        ]
                    }
                }
            ]
        })]),
        sse_response(&[json!({
            "choices": [
                {
                    "delta": {
                        "reasoning_content": "还想重复读取同一个文件。",
                        "tool_calls": [
                            {
                                "index": 0,
                                "id": "call_workspace_read_file_again",
                                "type": "function",
                                "function": {
                                    "name": "workspace_read_file",
                                    "arguments": duplicate_args
                                }
                            }
                        ]
                    }
                }
            ]
        })]),
        sse_response(&[
            json!({
                "choices": [
                    {
                        "delta": {
                            "reasoning_content": "重复工具调用已停止，直接总结。"
                        }
                    }
                ]
            }),
            json!({
                "choices": [
                    {
                        "delta": {
                            "content": final_text
                        }
                    }
                ]
            }),
        ]),
    ]);
    let mut runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));
    let sink = RecordingTurnEventSink::new();

    runtime.start_turn_stream(
        &sink,
        "turn-duplicate-tool-recovery-stream".to_string(),
        TurnInput {
            message: "读取 tauri.conf.json 并回答 productName".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            workspace_mode: None,
            session_id: Some("stream-duplicate-tool-recovery".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
        },
    );

    let requests = server.finish();
    let events = sink.events.borrow();
    let text_delta = events
        .iter()
        .filter_map(|(name, payload)| (name == "turn:delta").then_some(payload.clone()))
        .find(|payload| payload.text.as_deref() == Some(final_text))
        .expect("recovery answer should be streamed as delta");
    let completed = events
        .iter()
        .find_map(|(name, payload)| (name == "turn:completed").then_some(payload.clone()))
        .expect("completed event");

    assert_eq!(requests.len(), 3);
    assert!(text_delta.trace_timeline.is_none());
    assert_eq!(completed.text.as_deref(), Some(final_text));
    assert_eq!(
        completed.provider_source.as_deref(),
        Some("provider_followup_stream")
    );
    let provider_calls = completed
        .provider_call_records
        .as_ref()
        .expect("provider call records");
    assert_eq!(
        provider_calls.last().map(|record| &record.latency_kind),
        Some(&ProviderLatencyKind::ProviderStream)
    );
    assert!(provider_calls
        .last()
        .and_then(|record| record.first_token_latency_ms)
        .is_some());
}

#[test]
fn start_turn_stream_accumulates_token_usage_across_tool_followups() {
    let final_text = "流式回合已累计整轮 token usage。";
    let server = MockHttpServer::start(vec![
        sse_response(&[
            json!({
                "choices": [
                    {
                        "delta": {
                            "reasoning_content": "需要先执行 workspace_list_files。",
                            "tool_calls": [
                                {
                                    "index": 0,
                                    "id": "call_workspace_list_files",
                                    "type": "function",
                                    "function": {
                                        "name": "workspace_list_files",
                                        "arguments": "{\"path\":\".\",\"limit\":40}"
                                    }
                                }
                            ]
                        }
                    }
                ]
            }),
            json!({
                "choices": [],
                "usage": {
                    "prompt_tokens": 100,
                    "prompt_cache_hit_tokens": 30,
                    "prompt_cache_miss_tokens": 70,
                    "completion_tokens": 20,
                    "total_tokens": 120,
                    "completion_tokens_details": {
                        "reasoning_tokens": 7
                    }
                }
            }),
        ]),
        sse_response(&[
            json!({
                "choices": [
                    {
                        "delta": {
                            "reasoning_content": "目录已找到。"
                        }
                    }
                ]
            }),
            json!({
                "choices": [
                    {
                        "delta": {
                            "content": "继续读取 tauri.conf.json。"
                        }
                    }
                ]
            }),
            json!({
                "choices": [
                    {
                        "delta": {
                            "tool_calls": [
                                {
                                    "index": 0,
                                    "id": "call_workspace_read_file",
                                    "type": "function",
                                    "function": {
                                        "name": "workspace_read_file",
                                        "arguments": "{\"path\":\"tauri.conf.json\"}"
                                    }
                                }
                            ]
                        }
                    }
                ]
            }),
            json!({
                "choices": [],
                "usage": {
                    "prompt_tokens": 80,
                    "prompt_cache_hit_tokens": 25,
                    "prompt_cache_miss_tokens": 55,
                    "completion_tokens": 10,
                    "total_tokens": 90,
                    "completion_tokens_details": {
                        "reasoning_tokens": 3
                    }
                }
            }),
        ]),
        sse_response(&[
            json!({
                "choices": [
                    {
                        "delta": {
                            "reasoning_content": "已经整理完最终结果。"
                        }
                    }
                ]
            }),
            json!({
                "choices": [
                    {
                        "delta": {
                            "content": final_text
                        }
                    }
                ]
            }),
            json!({
                "choices": [],
                "usage": {
                    "prompt_tokens": 60,
                    "prompt_cache_hit_tokens": 15,
                    "prompt_cache_miss_tokens": 45,
                    "completion_tokens": 40,
                    "total_tokens": 100,
                    "completion_tokens_details": {
                        "reasoning_tokens": 2
                    }
                }
            }),
        ]),
    ]);
    let mut runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));
    let sink = RecordingTurnEventSink::new();

    runtime.start_turn_stream(
        &sink,
        "turn-usage-accumulated".to_string(),
        TurnInput {
            message: "继续读取 tauri.conf.json 第三行".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            workspace_mode: None,
            session_id: Some("stream-usage-accumulated".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
        },
    );

    let events = sink.events.borrow();
    let completed = events
        .iter()
        .find_map(|(name, payload)| {
            if name == "turn:completed" {
                Some(payload.clone())
            } else {
                None
            }
        })
        .expect("stream completed event");

    assert_eq!(completed.text.as_deref(), Some(final_text));
    assert_eq!(completed.input_tokens, Some(240));
    assert_eq!(completed.cache_hit_input_tokens, Some(70));
    assert_eq!(completed.reasoning_tokens, Some(12));
    assert_eq!(completed.output_tokens, Some(70));
    assert_eq!(completed.total_tokens, Some(310));

    let snapshot = runtime.load_session_snapshot(Some("stream-usage-accumulated"));
    let trace = snapshot
        .turn_trace_history
        .last()
        .expect("stream accumulated trace");
    assert_eq!(trace.input_tokens, Some(240));
    assert_eq!(trace.cache_hit_input_tokens, Some(70));
    assert_eq!(trace.reasoning_tokens, Some(12));
    assert_eq!(trace.output_tokens, Some(70));
    assert_eq!(trace.total_tokens, Some(310));
    assert_eq!(trace.provider_call_records.len(), 3);
    assert_eq!(
        trace.provider_call_records[0].request_kind,
        ProviderRequestKind::InitialRequest
    );
    assert_eq!(
        trace.provider_call_records[1].request_kind,
        ProviderRequestKind::ToolFollowup
    );
    assert_eq!(
        trace.provider_call_records[2].cache_miss_input_tokens,
        Some(45)
    );

    let _ = server.finish();
}

#[test]
fn start_turn_stream_repairs_blank_tool_name_in_followup_stream() {
    let final_text = "tauri.conf.json 已在流式回合中成功读取。";
    let server = MockHttpServer::start(vec![
        sse_response(&[json!({
            "choices": [
                {
                    "delta": {
                        "reasoning_content": "需要先执行 workspace_list_files。",
                        "tool_calls": [
                            {
                                "index": 0,
                                "id": "call_workspace_list_files",
                                "type": "function",
                                "function": {
                                    "name": "workspace_list_files",
                                    "arguments": "{\"path\":\".\",\"limit\":40}"
                                }
                            }
                        ]
                    }
                }
            ]
        })]),
        sse_response(&[
            json!({
                "choices": [
                    {
                        "delta": {
                            "reasoning_content": "继续读取文件内容。"
                        }
                    }
                ]
            }),
            json!({
                "choices": [
                    {
                        "delta": {
                            "tool_calls": [
                                {
                                    "index": 0,
                                    "id": "call_blank_name_stream",
                                    "type": "function",
                                    "function": {
                                        "name": "",
                                        "arguments": "{\"path\":\"tauri.conf.json\"}"
                                    }
                                }
                            ]
                        }
                    }
                ]
            }),
        ]),
        sse_response(&[json!({
            "choices": [
                {
                    "delta": {
                        "content": final_text
                    }
                }
            ]
        })]),
    ]);
    let mut runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));
    let sink = RecordingTurnEventSink::new();

    runtime.start_turn_stream(
        &sink,
        "turn-repair-blank-stream".to_string(),
        TurnInput {
            message: "继续读取 tauri.conf.json".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            workspace_mode: None,
            session_id: Some("repair-blank-tool-stream".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
        },
    );

    let request_bodies = server.finish();
    let events = sink.events.borrow();
    let completed = events
        .iter()
        .find_map(|(name, payload)| {
            if name == "turn:completed" {
                Some(payload.clone())
            } else {
                None
            }
        })
        .expect("completed event");

    assert_eq!(completed.text.as_deref(), Some(final_text));
    assert!(events.iter().any(|(name, payload)| {
        name == "turn:tool"
            && payload
                .tool_activities
                .as_ref()
                .map(|activities| activities.iter().any(|activity| activity.name == "Read"))
                .unwrap_or(false)
    }));
    assert_eq!(request_bodies.len(), 3);
}

#[test]
fn runtime_can_rebuild_session_snapshot_and_retrieved_context_from_history_node() {
    let server = MockHttpServer::start(vec![json_completion("第一答"), json_completion("第二答")]);
    let mut runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));

    let first = runtime.run_turn(TurnInput {
        message: "第一问".to_string(),
        display_message: None,
        provider_id: None,
        model_id: None,
        reasoning_effort: None,
        workspace_mode: None,
        session_id: Some("runtime-history".to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
    });
    let second = runtime.run_turn(TurnInput {
        message: "第二问".to_string(),
        display_message: None,
        provider_id: None,
        model_id: None,
        reasoning_effort: None,
        workspace_mode: None,
        session_id: Some("runtime-history".to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
    });
    assert_eq!(first.assistant_message, "第一答");
    assert_eq!(second.assistant_message, "第二答");

    let (history_nodes, _, _) = runtime.load_history_graph(Some("runtime-history"));
    // 历史图首节点是 empty 的 root/checkpoint 节点（turn_count == 0），
    // 这里选取第一个真实轮次节点，以便重建出包含「第一问/第一答」的快照。
    let historical_node_id = history_nodes
        .iter()
        .find(|node| node.turn_count > 0)
        .map(|node| node.node_id.clone())
        .expect("historical turn node should exist");

    let snapshot = runtime
        .load_session_snapshot_at(Some("runtime-history"), Some(historical_node_id.as_str()));
    assert_eq!(
        snapshot.resolved_node_id.as_deref(),
        Some(historical_node_id.as_str())
    );
    assert_eq!(snapshot.history.len(), 2);
    assert_eq!(snapshot.history[0].content, "第一问");
    assert_eq!(
        snapshot.history_cursor.mode,
        crate::agent::session::HistoryCursorMode::Historical
    );

    let retrieved = runtime.inspect_retrieved_context_at(
        Some("runtime-history"),
        Some(historical_node_id.as_str()),
        None,
        None,
        None,
    );
    assert_eq!(retrieved.session_context.turn_count, 1);
    assert_eq!(retrieved.session_context.recent_history.len(), 2);

    let _ = server.finish();
}

#[test]
fn persisted_trace_timeline_uses_canonical_monitor_semantics() {
    let provider_meta = ProviderEventMeta {
        requested_name: "OpenAI".to_string(),
        provider_name: "OpenAI".to_string(),
        protocol: "openai".to_string(),
        model: "gpt-5".to_string(),
    };
    let build_context_observation = BuildContextObservation {
        request_format: "responses".to_string(),
        message_count: 4,
        image_count: 0,
        tool_count: 1,
        temperature: 0.0,
        max_output_tokens: 1024,
        stable_prefix_text: "system: stable".to_string(),
        semi_stable_context_text: "developer: retrieval summary".to_string(),
        volatile_input_text: "user: request".to_string(),
        prefix_mutation_reasons: vec![
            crate::agent::provider::PrefixMutationReason::HistoryBoundaryShifted,
        ],
        context_refresh_reason: None,
        instruction_scope_sources: Vec::new(),
        conversation_carry_mode: None,
        request_messages_text: "system: stable\nuser: request".to_string(),
        tool_definitions_text: "workspace.read_file(path)".to_string(),
    };
    let tool_activities = vec![crate::agent::telemetry::TurnToolActivity {
        id: "tool-read-file".to_string(),
        name: "workspace.read_file".to_string(),
        canonical_tool_name: Some("Read".to_string()),
        display_name_zh: Some("读取".to_string()),
        status: "done".to_string(),
        description: "read file done".to_string(),
        arguments_text: Some("{\"path\":\"src/main.ts\"}".to_string()),
        result_text: Some("{\"content\":\"ok\"}".to_string()),
        duration_seconds: Some(0.2),
        parent_activity_id: None,
        artifacts: None,
        error: None,
        capability_invocation: None,
    }];

    let timeline = build_persisted_trace_timeline(
        "读取文件",
        "completed",
        Some(&provider_meta),
        Some("primary"),
        Some("standard"),
        Some(&build_context_observation),
        &tool_activities,
        &[ModelHopTraceContent {
            text: "inspect before tool".to_string(),
            reasoning_content: Some("need file".to_string()),
        }],
        Some("final answer"),
        Some("summarize result"),
        None,
        None,
        Some(11),
        Some(3),
        Some(0),
        Some(7),
        Some(18),
        Some(99),
        Some(900),
    );

    let kinds = timeline
        .iter()
        .map(|entry| entry.kind.as_str())
        .collect::<Vec<_>>();
    // 持久化 trace timeline 自 aa2a0bf 起不再包含 "input" 首条目（trace 面板清理），
    // 用户输入条目由前端在展示层自行补全。这里断言规范化后的 monitor 语义。
    // PA-095 #3：tool/result → return_result 条目（与事件折叠重建同构）。
    assert_eq!(
        kinds,
        vec![
            "prepare_retrieval",
            "build_context",
            "call_model",
            "call_tool",
            "return_result",
            "call_model",
            "checkpoint_persist",
        ]
    );
    assert_eq!(timeline[0].label, "PREPARE RETRIEVAL");
    assert_eq!(timeline[3].label, "CALL TOOL #1 · workspace.read_file");
    // PA-095 #3：return_result 条目插入 tool 之后，后续条目索引 +1。
    assert_eq!(timeline[4].label, "RETURN RESULT");
    assert_eq!(timeline[5].label, "CALL MODEL #2");
    assert_eq!(timeline[2].text.as_deref(), Some("inspect before tool"));
    assert_eq!(timeline[2].reasoning_content.as_deref(), Some("need file"));
    assert_eq!(timeline[5].text.as_deref(), Some("final answer"));
    assert_eq!(
        timeline[5].reasoning_content.as_deref(),
        Some("summarize result")
    );
}

// ── PA-100：timeline 失败工具条目必须记录真实错误，而不是工具描述 ─────────────────────────

fn pa100_failed_activity(
    id: &str,
    name: &str,
    status: &str,
    error: Option<Value>,
) -> crate::agent::telemetry::TurnToolActivity {
    crate::agent::telemetry::TurnToolActivity {
        id: id.to_string(),
        name: name.to_string(),
        canonical_tool_name: Some("Read".to_string()),
        display_name_zh: Some("读取".to_string()),
        status: status.to_string(),
        description: format!("{name} 的中文描述文本"),
        arguments_text: Some("{\"path\":\"src/tauri_adapter.rs\",\"startLine\":80}".to_string()),
        result_text: Some(
            "{\"error\":{\"code\":\"invalid_arguments\",\"message\":\"unexpected argument \
             `startLine`\"},\"ok\":false}"
                .to_string(),
        ),
        duration_seconds: Some(0.0),
        parent_activity_id: None,
        artifacts: None,
        error,
        capability_invocation: None,
    }
}

fn pa100_build_timeline(
    activities: Vec<crate::agent::telemetry::TurnToolActivity>,
) -> Vec<crate::agent::session::TraceTimelineEntry> {
    let provider_meta = ProviderEventMeta {
        requested_name: "商汤Token Plan".to_string(),
        provider_name: "商汤Token Plan".to_string(),
        protocol: "openai-completions".to_string(),
        model: "deepseek-v4-flash".to_string(),
    };
    let observation = BuildContextObservation {
        request_format: "openai-completions".to_string(),
        message_count: 9,
        image_count: 0,
        tool_count: 20,
        temperature: 0.0,
        max_output_tokens: 65536,
        stable_prefix_text: String::new(),
        semi_stable_context_text: String::new(),
        volatile_input_text: "user: tauri_adapter.rs是什么？".to_string(),
        prefix_mutation_reasons: Vec::new(),
        context_refresh_reason: None,
        instruction_scope_sources: Vec::new(),
        conversation_carry_mode: None,
        request_messages_text: "user: tauri_adapter.rs是什么？".to_string(),
        tool_definitions_text: "Read(path)".to_string(),
    };
    build_persisted_trace_timeline(
        "tauri_adapter.rs是什么？",
        "completed",
        Some(&provider_meta),
        Some("primary"),
        Some("fallback"),
        Some(&observation),
        &activities,
        &[ModelHopTraceContent {
            text: "let me read".to_string(),
            reasoning_content: Some("read file".to_string()),
        }],
        Some("canned fallback"),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
}

#[test]
fn persisted_timeline_records_real_kind_shaped_error_instead_of_description() {
    // 真实主路径形状：telemetry 写入 ToolError 序列化 {kind, message}。
    let activity = pa100_failed_activity(
        "tool-read",
        "workspace_read_file_segment",
        "error",
        Some(json!({
            "kind": "invalid_arguments",
            "message": "unexpected argument `startLine` at `$`"
        })),
    );
    let timeline = pa100_build_timeline(vec![activity]);

    let call_tool = &timeline[3];
    assert_eq!(call_tool.kind, "call_tool");
    assert_eq!(call_tool.state, "error");
    // 描述文本仍留在 text 字段；error 字段必须是真实错误。
    assert_eq!(
        call_tool.text.as_deref(),
        Some("workspace_read_file_segment 的中文描述文本")
    );
    let error_text = call_tool.error.as_deref().expect("structured error");
    assert!(error_text.contains("invalid_arguments"), "{error_text}");
    assert!(error_text.contains("startLine"), "{error_text}");
    assert_ne!(error_text, call_tool.text.as_deref().unwrap_or_default());

    let return_result = &timeline[4];
    assert_eq!(return_result.kind, "return_result");
    assert_eq!(return_result.state, "error");
    assert_eq!(return_result.error.as_deref(), call_tool.error.as_deref());
}

#[test]
fn persisted_timeline_error_extracts_code_and_string_shapes() {
    // DispatchError into_outcome 形状：{code, message}。
    let code_shape = pa100_failed_activity(
        "tool-read-code",
        "workspace_gather_context",
        "error",
        Some(json!({
            "code": "invalid_arguments",
            "message": "schema forbids additional properties"
        })),
    );
    // projection 折叠形状：纯字符串。
    let string_shape = pa100_failed_activity(
        "tool-read-string",
        "workspace_search_text",
        "error",
        Some(Value::String("upstream provider exploded".to_string())),
    );

    let timeline = pa100_build_timeline(vec![code_shape, string_shape]);
    // 两个失败活动各占一组 model+tool+return 条目。
    assert_eq!(
        timeline[3].error.as_deref(),
        Some("invalid_arguments: schema forbids additional properties")
    );
    assert_eq!(
        timeline[6].error.as_deref(),
        Some("upstream provider exploded")
    );
}

#[test]
fn persisted_timeline_populates_error_for_aborted_without_flipping_state() {
    let activity = pa100_failed_activity(
        "tool-read-aborted",
        "workspace_read_file_segment",
        "aborted",
        Some(json!({
            "kind": "batch_aborted",
            "message": "sibling failure aborted this child"
        })),
    );
    let timeline = pa100_build_timeline(vec![activity]);

    let call_tool = &timeline[3];
    // state 判定维持原语义（仅 error 翻转），但 aborted 的错误信息不再丢失。
    assert_eq!(call_tool.state, "completed");
    assert_eq!(
        call_tool.error.as_deref(),
        Some("batch_aborted: sibling failure aborted this child")
    );
}

#[test]
fn persisted_timeline_leaves_error_none_when_no_structured_error_present() {
    // 无结构化错误时回退 None——刻意不回退 description，
    // 避免「错误」栏重复展示描述文本的假象（PA-100 审核裁决）。
    let activity = pa100_failed_activity("tool-read-bare", "workspace_read_file", "error", None);
    let timeline = pa100_build_timeline(vec![activity]);

    assert_eq!(timeline[3].state, "error");
    assert_eq!(timeline[3].error, None);
}

/// PA-100 审核 A 缺失测试②：错误文本 300 字符截断边界（含多字节字符按字符计）。
#[test]
fn activity_error_text_truncates_at_300_chars_by_char_count() {
    use crate::agent::runtime::trace_timeline::turn_tool_activity_error_text;

    // 仅 message、无 kind/code：返回文本即 message 本身，便于精确对齐边界。
    let exactly_300 = "错".repeat(300);
    let ok_activity = pa100_failed_activity(
        "t-ok",
        "workspace_read_file",
        "error",
        Some(json!({ "message": exactly_300 })),
    );
    let ok_text = turn_tool_activity_error_text(&ok_activity).expect("within limit");
    assert_eq!(ok_text.chars().count(), 300);
    assert!(!ok_text.ends_with('…'));

    let over_activity = pa100_failed_activity(
        "t-over",
        "workspace_read_file",
        "error",
        Some(json!({ "code": "invalid_arguments", "message": "y".repeat(400) })),
    );
    let over_text = turn_tool_activity_error_text(&over_activity).expect("over limit");
    // "{code}: {message}" 组合后截断到 300 字符 + 省略号。
    assert!(over_text.starts_with("invalid_arguments: "));
    assert_eq!(over_text.chars().count(), 301);
    assert!(over_text.ends_with('…'));
}

// ── PA-100：参数推断门——query+startLine 组合必须路由到 gather，而不是 segment ────────────

#[test]
fn tool_name_inference_routes_gather_signals_before_start_line() {
    use crate::agent::runtime::stream_support::infer_tool_name_from_arguments;

    // F1 之后 {path, query, startLine} 是合法的 gather 搜索+翻页组合；
    // 若先命中 startLine→segment（schema 无 query），会换门复现 invalid_arguments。
    assert_eq!(
        infer_tool_name_from_arguments(&json!({
            "path": "src/lib.rs",
            "query": "TokenManager",
            "startLine": 80
        }))
        .as_deref(),
        Some("workspace_gather_context")
    );

    // 纯分页仍归 segment。
    assert_eq!(
        infer_tool_name_from_arguments(&json!({ "path": "src/lib.rs", "startLine": 80 }))
            .as_deref(),
        Some("workspace_read_file_segment")
    );

    // 既有行为回归：lineCount 单独出现时仍指向 gather。
    assert_eq!(
        infer_tool_name_from_arguments(&json!({ "path": "src/lib.rs", "lineCount": 40 }))
            .as_deref(),
        Some("workspace_gather_context")
    );

    // PA-100 审核 A 缺失测试④：{startLine,lineCount} 组合在门序调整后路由 gather
    // （旧行为 segment）——钉住防止未来回调时无声翻转。
    assert_eq!(
        infer_tool_name_from_arguments(&json!({
            "path": "src/lib.rs",
            "startLine": 80,
            "lineCount": 40
        }))
        .as_deref(),
        Some("workspace_gather_context")
    );
}

/// PA-100 审核 A 缺失测试③：progress builder 与 persisted builder 的错误提取
/// 必须同构（turn_stream 生产路径仍在使用前者）。
#[test]
fn progress_timeline_records_real_tool_error_like_persisted_builder() {
    let provider_meta = ProviderEventMeta {
        requested_name: "商汤Token Plan".to_string(),
        provider_name: "商汤Token Plan".to_string(),
        protocol: "openai-completions".to_string(),
        model: "deepseek-v4-flash".to_string(),
    };
    let observation = BuildContextObservation {
        request_format: "openai-completions".to_string(),
        message_count: 9,
        image_count: 0,
        tool_count: 20,
        temperature: 0.0,
        max_output_tokens: 65536,
        stable_prefix_text: String::new(),
        semi_stable_context_text: String::new(),
        volatile_input_text: "user: q".to_string(),
        prefix_mutation_reasons: Vec::new(),
        context_refresh_reason: None,
        instruction_scope_sources: Vec::new(),
        conversation_carry_mode: None,
        request_messages_text: "user: q".to_string(),
        tool_definitions_text: "Read(path)".to_string(),
    };
    let activity = pa100_failed_activity(
        "tool-read-progress",
        "workspace_read_file_segment",
        "error",
        Some(json!({
            "kind": "invalid_arguments",
            "message": "unexpected argument `startLine`"
        })),
    );

    let timeline = build_stream_progress_trace_timeline(
        "tauri_adapter.rs是什么？",
        &provider_meta,
        None,
        None,
        &observation,
        std::slice::from_ref(&activity),
        &[ModelHopTraceContent {
            text: "hop".to_string(),
            reasoning_content: None,
        }],
        None,
        None,
        None,
        "calling_tool",
    );

    let tool_entry = timeline
        .iter()
        .find(|entry| entry.kind == "call_tool")
        .expect("progress call_tool entry");
    assert_eq!(tool_entry.state, "error");
    assert_eq!(
        tool_entry.error.as_deref(),
        Some("invalid_arguments: unexpected argument `startLine`")
    );
    let return_entry = timeline
        .iter()
        .find(|entry| entry.kind == "return_result")
        .expect("progress return_result entry");
    assert_eq!(return_entry.error.as_deref(), tool_entry.error.as_deref());
}

#[test]
fn stream_trace_keeps_each_model_hop_before_its_tool() {
    let provider_meta = ProviderEventMeta {
        requested_name: "OpenAI".to_string(),
        provider_name: "OpenAI".to_string(),
        protocol: "openai".to_string(),
        model: "gpt-5".to_string(),
    };
    let observation = BuildContextObservation {
        request_format: "responses".to_string(),
        message_count: 1,
        image_count: 0,
        tool_count: 1,
        temperature: 0.0,
        max_output_tokens: 1024,
        stable_prefix_text: String::new(),
        semi_stable_context_text: String::new(),
        volatile_input_text: "inspect".to_string(),
        prefix_mutation_reasons: Vec::new(),
        context_refresh_reason: None,
        instruction_scope_sources: Vec::new(),
        conversation_carry_mode: None,
        request_messages_text: "user: inspect".to_string(),
        tool_definitions_text: "Read(path)".to_string(),
    };
    let completed_model_hops = vec![
        ModelHopTraceContent {
            text: "first model output".to_string(),
            reasoning_content: Some("first reasoning".to_string()),
        },
        ModelHopTraceContent {
            text: "second model output".to_string(),
            reasoning_content: Some("second reasoning".to_string()),
        },
    ];

    let timeline = build_stream_progress_trace_timeline(
        "inspect",
        &provider_meta,
        None,
        None,
        &observation,
        &[],
        &completed_model_hops,
        Some("second model output"),
        Some("second reasoning"),
        None,
        "calling_tool",
    );
    let model_entries = timeline
        .iter()
        .filter(|entry| entry.kind == "call_model")
        .collect::<Vec<_>>();

    assert_eq!(model_entries.len(), 2);
    assert_eq!(model_entries[0].text.as_deref(), Some("first model output"));
    assert_eq!(
        model_entries[0].reasoning_content.as_deref(),
        Some("first reasoning")
    );
    assert_eq!(
        model_entries[1].text.as_deref(),
        Some("second model output")
    );
    assert_eq!(
        model_entries[1].reasoning_content.as_deref(),
        Some("second reasoning")
    );
}

#[test]
fn deepseek_tool_followup_uses_live_stream() {
    let final_text = "deepseek follow-up completed";
    let server = MockHttpServer::start(vec![
        sse_response(&[
            json!({
                "choices": [
                    {
                        "delta": {
                            "reasoning_content": "need workspace listing before answering"
                        }
                    }
                ]
            }),
            json!({
                "choices": [
                    {
                        "delta": {
                            "content": "call a tool first",
                            "tool_calls": [
                                {
                                    "index": 0,
                                    "id": "call_workspace_list_files",
                                    "type": "function",
                                    "function": {
                                        "name": "workspace_list_files",
                                        "arguments": "{\"path\":\".\",\"limit\":40}"
                                    }
                                }
                            ]
                        }
                    }
                ]
            }),
        ]),
        sse_response(&[
            json!({
                "choices": [
                    {
                        "delta": {
                            "reasoning_content": "tool output is sufficient"
                        }
                    }
                ]
            }),
            json!({
                "choices": [
                    {
                        "delta": {
                            "content": final_text
                        }
                    }
                ]
            }),
            json!({
                "choices": [],
                "usage": {
                    "prompt_tokens": 60,
                    "completion_tokens": 24,
                    "total_tokens": 84
                }
            }),
        ]),
    ]);
    let mut runtime = build_runtime_for_test(deepseek_provider_selection(server.base_url.clone()));
    let sink = RecordingTurnEventSink::new();

    runtime.start_turn_stream(
        &sink,
        "turn-deepseek-followup-compat".to_string(),
        TurnInput {
            message: "read Cargo.toml then answer".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            workspace_mode: None,
            session_id: Some("deepseek-followup-compat".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
        },
    );

    let requests = server.finish();
    let events = sink.events.borrow();
    let text_delta = events
        .iter()
        .filter_map(|(name, payload)| (name == "turn:delta").then_some(payload.clone()))
        .find(|payload| payload.text.as_deref() == Some(final_text))
        .expect("text delta event");
    let completed = events
        .iter()
        .find_map(|(name, payload)| (name == "turn:completed").then_some(payload.clone()))
        .expect("completed event");

    assert_eq!(requests.len(), 2);
    let decision_request: Value =
        serde_json::from_str(&requests[0]).expect("decision request should be json");
    let followup_request: Value =
        serde_json::from_str(&requests[1]).expect("followup request should be json");
    assert_eq!(
        decision_request.get("stream").and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        followup_request.get("stream").and_then(Value::as_bool),
        Some(true)
    );
    assert!(followup_request.get("stream_options").is_some());
    let replayed_assistant_message = followup_request
        .get("messages")
        .and_then(Value::as_array)
        .and_then(|messages| {
            messages.iter().find(|message| {
                message.get("role").and_then(Value::as_str) == Some("assistant")
                    && message
                        .get("tool_calls")
                        .and_then(Value::as_array)
                        .map(|calls| !calls.is_empty())
                        .unwrap_or(false)
            })
        })
        .expect("follow-up request should replay assistant tool call message");
    assert_eq!(
        replayed_assistant_message
            .get("reasoning_content")
            .and_then(Value::as_str),
        Some("need workspace listing before answering")
    );
    assert_eq!(text_delta.text.as_deref(), Some(final_text));
    assert_eq!(completed.phase.as_deref(), Some("completed"));
    assert_eq!(
        completed.provider_source.as_deref(),
        Some("provider_followup_stream")
    );
    assert_eq!(completed.fallback_reason, None);
    let provider_calls = completed
        .provider_call_records
        .as_ref()
        .expect("provider call records");
    assert_eq!(provider_calls.len(), 2);
    assert!(provider_calls
        .iter()
        .all(|record| record.first_token_latency_ms.is_some()));
}

#[test]
fn deepseek_followup_replays_full_reasoning_from_fragmented_sse() {
    // 真实 DeepSeek 以多个 SSE chunk 流式返回 reasoning_content，
    // follow-up 请求必须回传完整累计文本（DeepSeek thinking 模式校验）。
    let final_text = "deepseek fragmented follow-up completed";
    let full_reasoning = "第一步检查版本迁移，第二步执行搜索。";
    let server = MockHttpServer::start(vec![
        sse_response(&[
            json!({
                "choices": [{"delta": {"reasoning_content": "第一步检查"}}]
            }),
            json!({
                "choices": [{"delta": {"reasoning_content": "版本迁移，"}}]
            }),
            json!({
                "choices": [{"delta": {"reasoning_content": "第二步执行搜索。"}}]
            }),
            json!({
                "choices": [
                    {
                        "delta": {
                            "content": "call a tool first",
                            "tool_calls": [
                                {
                                    "index": 0,
                                    "id": "call_web_search",
                                    "type": "function",
                                    "function": {
                                        "name": "workspace_search_text",
                                        "arguments": "{\"query\":\"migration\",\"path\":\".\"}"
                                    }
                                }
                            ]
                        }
                    }
                ]
            }),
        ]),
        sse_response(&[
            json!({
                "choices": [{"delta": {"content": final_text}}]
            }),
            json!({
                "choices": [],
                "usage": {
                    "prompt_tokens": 60,
                    "completion_tokens": 24,
                    "total_tokens": 84
                }
            }),
        ]),
    ]);
    let mut runtime = build_runtime_for_test(deepseek_provider_selection(server.base_url.clone()));
    let sink = RecordingTurnEventSink::new();

    runtime.start_turn_stream(
        &sink,
        "turn-deepseek-fragmented".to_string(),
        TurnInput {
            message: "搜索迁移指南".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            workspace_mode: None,
            session_id: Some("deepseek-fragmented".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
        },
    );

    let requests = server.finish();
    assert_eq!(requests.len(), 2);
    let followup_request: Value =
        serde_json::from_str(&requests[1]).expect("followup request should be json");
    let messages = followup_request
        .get("messages")
        .and_then(Value::as_array)
        .expect("messages");
    let replayed = messages
        .iter()
        .find(|message| {
            message.get("role").and_then(Value::as_str) == Some("assistant")
                && message
                    .get("tool_calls")
                    .and_then(Value::as_array)
                    .map(|calls| !calls.is_empty())
                    .unwrap_or(false)
        })
        .expect("follow-up should replay assistant tool call message");
    assert_eq!(
        replayed.get("reasoning_content").and_then(Value::as_str),
        Some(full_reasoning),
        "follow-up 必须回传完整 reasoning_content，而不是首个分片"
    );
}

#[test]
fn deepseek_multi_hop_followup_preserves_structured_reasoning_content() {
    let final_text = "deepseek structured follow-up completed";
    let first_reasoning = json!([
        { "type": "reasoning", "text": "need workspace listing before answering" }
    ]);
    let second_reasoning = json!([
        { "type": "reasoning", "text": "need Cargo.toml content before answering" }
    ]);
    let final_reasoning = json!([
        { "type": "reasoning", "text": "tool output is sufficient" }
    ]);
    let server = MockHttpServer::start(vec![
        sse_response(&[
            json!({
                "choices": [
                    {
                        "delta": {
                            "reasoning_content": first_reasoning.clone()
                        }
                    }
                ]
            }),
            json!({
                "choices": [
                    {
                        "delta": {
                            "content": "call a tool first",
                            "tool_calls": [
                                {
                                    "index": 0,
                                    "id": "call_workspace_list_files",
                                    "type": "function",
                                    "function": {
                                        "name": "workspace_list_files",
                                        "arguments": "{\"path\":\".\",\"limit\":40}"
                                    }
                                }
                            ]
                        }
                    }
                ]
            }),
        ]),
        sse_response(&[
            json!({
                "choices": [
                    {
                        "delta": {
                            "reasoning_content": second_reasoning.clone()
                        }
                    }
                ]
            }),
            json!({
                "choices": [
                    {
                        "delta": {
                            "content": "read Cargo.toml next",
                            "tool_calls": [
                                {
                                    "index": 0,
                                    "id": "call_workspace_read_file",
                                    "type": "function",
                                    "function": {
                                        "name": "workspace_read_file",
                                        "arguments": "{\"path\":\"Cargo.toml\"}"
                                    }
                                }
                            ]
                        }
                    }
                ]
            }),
        ]),
        sse_response(&[
            json!({
                "choices": [
                    {
                        "delta": {
                            "reasoning_content": final_reasoning.clone()
                        }
                    }
                ]
            }),
            json!({
                "choices": [
                    {
                        "delta": {
                            "content": final_text
                        }
                    }
                ]
            }),
            json!({
                "choices": [],
                "usage": {
                    "prompt_tokens": 120,
                    "completion_tokens": 36,
                    "total_tokens": 156
                }
            }),
        ]),
    ]);
    let mut runtime = build_runtime_for_test(deepseek_provider_selection(server.base_url.clone()));
    let sink = RecordingTurnEventSink::new();

    runtime.start_turn_stream(
        &sink,
        "turn-deepseek-structured-followup".to_string(),
        TurnInput {
            message: "inspect workspace then read Cargo.toml".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            workspace_mode: None,
            session_id: Some("deepseek-structured-followup".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
        },
    );

    let requests = server.finish();
    assert_eq!(requests.len(), 3);
    let first_followup: Value =
        serde_json::from_str(&requests[1]).expect("first followup request should be json");
    let second_followup: Value =
        serde_json::from_str(&requests[2]).expect("second followup request should be json");

    let first_replayed = first_followup
        .get("messages")
        .and_then(Value::as_array)
        .and_then(|messages| {
            messages.iter().find(|message| {
                message.get("role").and_then(Value::as_str) == Some("assistant")
                    && message
                        .get("tool_calls")
                        .and_then(Value::as_array)
                        .map(|calls| !calls.is_empty())
                        .unwrap_or(false)
            })
        })
        .and_then(|message| message.get("reasoning_content"))
        .cloned()
        .expect("first followup should replay structured reasoning");
    let second_replayed = second_followup
        .get("messages")
        .and_then(Value::as_array)
        .and_then(|messages| {
            messages.iter().rev().find(|message| {
                message.get("role").and_then(Value::as_str) == Some("assistant")
                    && message
                        .get("tool_calls")
                        .and_then(Value::as_array)
                        .map(|calls| !calls.is_empty())
                        .unwrap_or(false)
            })
        })
        .and_then(|message| message.get("reasoning_content"))
        .cloned()
        .expect("second followup should replay structured reasoning");

    assert_eq!(
        first_replayed,
        json!([{ "type": "reasoning", "text": "need workspace listing before answering" }])
    );
    assert_eq!(
        second_replayed,
        json!([{ "type": "reasoning", "text": "need Cargo.toml content before answering" }])
    );
}

#[test]
fn registry_resource_tool_returns_structured_resource_result() {
    let mut runtime =
        build_runtime_for_test(test_provider_selection("http://localhost".to_string()));
    runtime.apply_mcp_source_snapshot(crate::agent::capability_bridge::McpSourceSnapshot {
        source: crate::agent::capability_bridge::CapabilitySourceView {
            source_id: "mcp-resource".to_string(),
            source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
            display_name: "Resource MCP".to_string(),
            availability: crate::agent::capability_bridge::CapabilityAvailability::Available,
            transport_kind: "stdio".to_string(),
            server_identity: "mcp://resource".to_string(),
            declared_capabilities: vec![crate::agent::capability_bridge::CapabilityKind::Resource],
            permission_profile: "host-mediated".to_string(),
            updated_at_ms: 1,
            last_ingress_observation: None,
        },
        capabilities: vec![crate::agent::capability_bridge::CapabilityView {
            capability_id: "mcp:resource:repo-index".to_string(),
            source_id: "mcp-resource".to_string(),
            source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
            kind: crate::agent::capability_bridge::CapabilityKind::Resource,
            label: "repo_index".to_string(),
            description: "Repository index".to_string(),
            invocation_mode:
                crate::agent::capability_bridge::CapabilityInvocationMode::ReadOnlyFetch,
            input_schema_summary: "{}".to_string(),
            safety_class: "read_only".to_string(),
            visibility: "default".to_string(),
            observability_tags: vec!["mcp".to_string(), "resource".to_string()],
            requires_approval: false,
            host_mediated: true,
            permission_scope: "workspace.read".to_string(),
        }],
    });

    let (tool_result, invocation_record, _) = runtime.execute_registered_tool_call(&ToolCall {
        call_id: None,
        name: "MCPResource".to_string(),
        arguments: json!({
            "capabilityId": "mcp:resource:repo-index",
            "arguments": { "path": "src" }
        }),
        plan: None,
    });

    // design Decision 11: the registry resource entrypoint fails closed with
    // `source_unavailable` until the real McpTransport-backed `McpResourceSurface` is wired;
    // the request arguments are never echoed back as resource content (PA-076 phase-7
    // review P1-1).
    assert_eq!(tool_result.status, "error");
    let payload =
        serde_json::from_str::<Value>(&tool_result.output).expect("resource tool output json");
    assert_eq!(
        payload.get("requestedCapabilityId").and_then(Value::as_str),
        Some("mcp:resource:repo-index")
    );
    assert_eq!(
        payload
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str),
        Some("source_unavailable")
    );
    assert_eq!(payload.get("content"), Some(&Value::Null));
    assert_eq!(
        invocation_record.capability_id.as_deref(),
        Some("mcp:resource:repo-index")
    );
    assert_eq!(
        invocation_record.capability_kind.as_deref(),
        Some("resource")
    );
    assert_eq!(
        invocation_record.invocation_mode.as_deref(),
        Some("read_only_fetch")
    );
    assert_eq!(
        invocation_record.failure_kind.as_deref(),
        Some("source_unavailable")
    );
}

#[test]
fn registry_resource_tool_accepts_canonical_and_dotted_aliases() {
    let mut runtime =
        build_runtime_for_test(test_provider_selection("http://localhost".to_string()));
    runtime.apply_mcp_source_snapshot(crate::agent::capability_bridge::McpSourceSnapshot {
        source: crate::agent::capability_bridge::CapabilitySourceView {
            source_id: "mcp-resource".to_string(),
            source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
            display_name: "Resource MCP".to_string(),
            availability: crate::agent::capability_bridge::CapabilityAvailability::Available,
            transport_kind: "stdio".to_string(),
            server_identity: "mcp://resource".to_string(),
            declared_capabilities: vec![crate::agent::capability_bridge::CapabilityKind::Resource],
            permission_profile: "host-mediated".to_string(),
            updated_at_ms: 1,
            last_ingress_observation: None,
        },
        capabilities: vec![crate::agent::capability_bridge::CapabilityView {
            capability_id: "mcp:resource:repo-index".to_string(),
            source_id: "mcp-resource".to_string(),
            source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
            kind: crate::agent::capability_bridge::CapabilityKind::Resource,
            label: "repo_index".to_string(),
            description: "Repository index".to_string(),
            invocation_mode:
                crate::agent::capability_bridge::CapabilityInvocationMode::ReadOnlyFetch,
            input_schema_summary: "{}".to_string(),
            safety_class: "read_only".to_string(),
            visibility: "default".to_string(),
            observability_tags: vec!["mcp".to_string(), "resource".to_string()],
            requires_approval: false,
            host_mediated: true,
            permission_scope: "workspace.read".to_string(),
        }],
    });

    for tool_name in ["mcp_resource_read", "mcp.resource_read"] {
        let (tool_result, invocation_record, _) = runtime.execute_registered_tool_call(&ToolCall {
            call_id: None,
            name: tool_name.to_string(),
            arguments: json!({
                "capabilityId": "mcp:resource:repo-index",
                "arguments": { "path": "src" }
            }),
            plan: None,
        });

        assert_eq!(tool_result.status, "error");
        assert_eq!(
            invocation_record.capability_id.as_deref(),
            Some("mcp:resource:repo-index")
        );
        assert_eq!(
            invocation_record.failure_kind.as_deref(),
            Some("source_unavailable")
        );
    }
}

#[test]
fn registry_resource_tool_requires_capability_id() {
    let runtime = build_runtime_for_test(test_provider_selection("http://localhost".to_string()));

    let (tool_result, invocation_record, _) = runtime.execute_registered_tool_call(&ToolCall {
        call_id: None,
        name: "MCPResource".to_string(),
        arguments: json!({}),
        plan: None,
    });

    assert_eq!(tool_result.status, "error");
    let payload =
        serde_json::from_str::<Value>(&tool_result.output).expect("resource error output json");
    assert_eq!(
        payload
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str),
        Some("missing_capability_id")
    );
    assert_eq!(
        invocation_record.failure_kind.as_deref(),
        Some("hook_blocked")
    );
}

#[test]
fn registry_resource_tool_returns_not_found_error_for_unknown_capability() {
    let runtime = build_runtime_for_test(test_provider_selection("http://localhost".to_string()));

    let (tool_result, invocation_record, _) = runtime.execute_registered_tool_call(&ToolCall {
        call_id: None,
        name: "MCPResource".to_string(),
        arguments: json!({
            "capabilityId": "mcp:resource:missing"
        }),
        plan: None,
    });

    assert_eq!(tool_result.status, "error");
    let payload =
        serde_json::from_str::<Value>(&tool_result.output).expect("resource missing output json");
    assert_eq!(
        payload.get("requestedCapabilityId").and_then(Value::as_str),
        Some("mcp:resource:missing")
    );
    assert_eq!(
        payload
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str),
        Some("capability_not_found")
    );
    assert_eq!(
        invocation_record.failure_kind.as_deref(),
        Some("capability_not_found")
    );
}

#[test]
fn tool_search_returns_registry_tool_candidates() {
    let mut runtime =
        build_runtime_for_test(test_provider_selection("http://localhost".to_string()));
    runtime.apply_mcp_source_snapshot(crate::agent::capability_bridge::McpSourceSnapshot {
        source: crate::agent::capability_bridge::CapabilitySourceView {
            source_id: "mcp-tools".to_string(),
            source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
            display_name: "Tools MCP".to_string(),
            availability: crate::agent::capability_bridge::CapabilityAvailability::Available,
            transport_kind: "stdio".to_string(),
            server_identity: "mcp://tools".to_string(),
            declared_capabilities: vec![crate::agent::capability_bridge::CapabilityKind::Tool],
            permission_profile: "host-mediated".to_string(),
            updated_at_ms: 1,
            last_ingress_observation: None,
        },
        capabilities: vec![crate::agent::capability_bridge::CapabilityView {
            capability_id: "mcp:tool:workspace-search".to_string(),
            source_id: "mcp-tools".to_string(),
            source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
            kind: crate::agent::capability_bridge::CapabilityKind::Tool,
            label: "workspace_search".to_string(),
            description: "Search workspace files".to_string(),
            invocation_mode:
                crate::agent::capability_bridge::CapabilityInvocationMode::DirectToolCall,
            input_schema_summary: "{}".to_string(),
            safety_class: "host_tool".to_string(),
            visibility: "default".to_string(),
            observability_tags: vec!["mcp".to_string(), "tool".to_string()],
            requires_approval: false,
            host_mediated: true,
            permission_scope: "workspace.read".to_string(),
        }],
    });

    let (tool_result, invocation_record, _) = runtime.execute_registered_tool_call(&ToolCall {
        call_id: None,
        name: "ToolSearch".to_string(),
        arguments: json!({
            "query": "workspace",
            "sourceId": "mcp-tools",
            "limit": 5
        }),
        plan: None,
    });

    assert_eq!(tool_result.status, "ok");
    let payload =
        serde_json::from_str::<Value>(&tool_result.output).expect("tool search output json");
    assert_eq!(
        payload.get("candidateCount").and_then(Value::as_u64),
        Some(1)
    );
    let candidates = payload
        .get("candidates")
        .and_then(Value::as_array)
        .expect("tool search candidates array");
    assert_eq!(candidates.len(), 1);
    assert_eq!(
        candidates[0].get("capabilityId").and_then(Value::as_str),
        Some("mcp:tool:workspace-search")
    );
    assert_eq!(
        candidates[0].get("tool_name").and_then(Value::as_str),
        Some("workspace_search")
    );
    assert_eq!(
        candidates[0]
            .get("source")
            .and_then(|source| source.get("sourceId"))
            .and_then(Value::as_str),
        Some("mcp-tools")
    );
    assert!(
        candidates[0]
            .get("confidence")
            .and_then(Value::as_f64)
            .unwrap_or(0.0)
            >= 0.7
    );
    assert_eq!(
        candidates[0].get("sourceId").and_then(Value::as_str),
        Some("mcp-tools")
    );
    assert_eq!(
        candidates[0].get("permissionScope").and_then(Value::as_str),
        Some("workspace.read")
    );
    assert_eq!(invocation_record.capability_kind.as_deref(), Some("tool"));
    assert_eq!(
        invocation_record.invocation_mode.as_deref(),
        Some("discovery")
    );
    assert_eq!(
        invocation_record.permission_scope.as_deref(),
        Some("capability.discovery")
    );
}

#[test]
fn tool_search_returns_empty_candidates_when_no_match_or_filtered_out() {
    let mut runtime =
        build_runtime_for_test(test_provider_selection("http://localhost".to_string()));
    runtime.apply_mcp_source_snapshot(crate::agent::capability_bridge::McpSourceSnapshot {
        source: crate::agent::capability_bridge::CapabilitySourceView {
            source_id: "mcp-tools".to_string(),
            source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
            display_name: "Tools MCP".to_string(),
            availability: crate::agent::capability_bridge::CapabilityAvailability::Available,
            transport_kind: "stdio".to_string(),
            server_identity: "mcp://tools".to_string(),
            declared_capabilities: vec![crate::agent::capability_bridge::CapabilityKind::Tool],
            permission_profile: "host-mediated".to_string(),
            updated_at_ms: 1,
            last_ingress_observation: None,
        },
        capabilities: vec![crate::agent::capability_bridge::CapabilityView {
            capability_id: "mcp:tool:workspace-search".to_string(),
            source_id: "mcp-tools".to_string(),
            source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
            kind: crate::agent::capability_bridge::CapabilityKind::Tool,
            label: "workspace_search".to_string(),
            description: "Search workspace files".to_string(),
            invocation_mode:
                crate::agent::capability_bridge::CapabilityInvocationMode::DirectToolCall,
            input_schema_summary: "{}".to_string(),
            safety_class: "host_tool".to_string(),
            visibility: "default".to_string(),
            observability_tags: vec!["mcp".to_string(), "tool".to_string()],
            requires_approval: false,
            host_mediated: true,
            permission_scope: "workspace.read".to_string(),
        }],
    });

    let (tool_result, _, _) = runtime.execute_registered_tool_call(&ToolCall {
        call_id: None,
        name: "ToolSearch".to_string(),
        arguments: json!({
            "query": "python",
            "sourceId": "other-tools",
            "limit": 5
        }),
        plan: None,
    });

    assert_eq!(tool_result.status, "ok");
    let payload =
        serde_json::from_str::<Value>(&tool_result.output).expect("tool search empty output json");
    assert_eq!(
        payload.get("candidateCount").and_then(Value::as_u64),
        Some(0)
    );
    assert_eq!(
        payload
            .get("candidates")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(0)
    );
}

#[test]
fn tool_search_clamps_limit_to_twenty() {
    let mut runtime =
        build_runtime_for_test(test_provider_selection("http://localhost".to_string()));
    let capabilities = (0..25)
        .map(|index| crate::agent::capability_bridge::CapabilityView {
            capability_id: format!("mcp:tool:item-{index}"),
            source_id: "mcp-tools".to_string(),
            source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
            kind: crate::agent::capability_bridge::CapabilityKind::Tool,
            label: format!("item_{index}"),
            description: format!("Item {index}"),
            invocation_mode:
                crate::agent::capability_bridge::CapabilityInvocationMode::DirectToolCall,
            input_schema_summary: "{}".to_string(),
            safety_class: "host_tool".to_string(),
            visibility: "default".to_string(),
            observability_tags: vec!["mcp".to_string(), "tool".to_string()],
            requires_approval: false,
            host_mediated: true,
            permission_scope: "workspace.read".to_string(),
        })
        .collect::<Vec<_>>();
    runtime.apply_mcp_source_snapshot(crate::agent::capability_bridge::McpSourceSnapshot {
        source: crate::agent::capability_bridge::CapabilitySourceView {
            source_id: "mcp-tools".to_string(),
            source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
            display_name: "Tools MCP".to_string(),
            availability: crate::agent::capability_bridge::CapabilityAvailability::Available,
            transport_kind: "stdio".to_string(),
            server_identity: "mcp://tools".to_string(),
            declared_capabilities: vec![crate::agent::capability_bridge::CapabilityKind::Tool],
            permission_profile: "host-mediated".to_string(),
            updated_at_ms: 1,
            last_ingress_observation: None,
        },
        capabilities,
    });

    let (tool_result, _, _) = runtime.execute_registered_tool_call(&ToolCall {
        call_id: None,
        name: "ToolSearch".to_string(),
        arguments: json!({
            "limit": 999
        }),
        plan: None,
    });

    assert_eq!(tool_result.status, "ok");
    let payload = serde_json::from_str::<Value>(&tool_result.output)
        .expect("tool search limited output json");
    assert_eq!(
        payload.get("candidateCount").and_then(Value::as_u64),
        Some(20)
    );
    assert_eq!(
        payload
            .get("candidates")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(20)
    );
}

#[test]
fn tool_search_accepts_canonical_and_dotted_aliases() {
    let mut runtime =
        build_runtime_for_test(test_provider_selection("http://localhost".to_string()));
    runtime.apply_mcp_source_snapshot(crate::agent::capability_bridge::McpSourceSnapshot {
        source: crate::agent::capability_bridge::CapabilitySourceView {
            source_id: "mcp-tools".to_string(),
            source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
            display_name: "Tools MCP".to_string(),
            availability: crate::agent::capability_bridge::CapabilityAvailability::Available,
            transport_kind: "stdio".to_string(),
            server_identity: "mcp://tools".to_string(),
            declared_capabilities: vec![crate::agent::capability_bridge::CapabilityKind::Tool],
            permission_profile: "host-mediated".to_string(),
            updated_at_ms: 1,
            last_ingress_observation: None,
        },
        capabilities: vec![crate::agent::capability_bridge::CapabilityView {
            capability_id: "mcp:tool:workspace-search".to_string(),
            source_id: "mcp-tools".to_string(),
            source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
            kind: crate::agent::capability_bridge::CapabilityKind::Tool,
            label: "workspace_search".to_string(),
            description: "Search workspace files".to_string(),
            invocation_mode:
                crate::agent::capability_bridge::CapabilityInvocationMode::DirectToolCall,
            input_schema_summary: "{}".to_string(),
            safety_class: "host_tool".to_string(),
            visibility: "default".to_string(),
            observability_tags: vec!["mcp".to_string(), "tool".to_string()],
            requires_approval: false,
            host_mediated: true,
            permission_scope: "workspace.read".to_string(),
        }],
    });

    for tool_name in ["tool_search", "tool.search"] {
        let (tool_result, _, _) = runtime.execute_registered_tool_call(&ToolCall {
            call_id: None,
            name: tool_name.to_string(),
            arguments: json!({
                "query": "workspace",
                "sourceId": "mcp-tools",
                "limit": 5
            }),
            plan: None,
        });

        assert_eq!(tool_result.status, "ok");
        let payload =
            serde_json::from_str::<Value>(&tool_result.output).expect("tool search output json");
        assert_eq!(
            payload.get("candidateCount").and_then(Value::as_u64),
            Some(1)
        );
    }
}

// ── Ask suspension in the governed sync turn loop (PA-076 P1-1) ─────────────────────

#[test]
fn governed_ask_suspends_turn_and_binds_waiting_user_without_provider_followup() {
    let _guard = crate::agent::runtime_helper::TestRuntimeGuard::leak();
    let workspace = temp_workspace_dir("ask-suspend-loop");
    let executor = build_governed_executor(Some(workspace.clone()), None);
    let mut store = GraphRunStore::new();
    GraphRunner::new().start_run(
        &mut store,
        GraphEngine::new("state-machine-v1").start_run(
            "run-ask-loop",
            "ask flow",
            Some("session-ask-loop"),
        ),
    );
    let store_arc = Arc::new(Mutex::new(store));
    // A fake provider that would panic if `provider_followup` is called after Ask — the
    // MockHttpServer with empty responses causes any follow-up connection to fail, which
    // would produce a failed (not suspended) TurnResult.
    let server = MockHttpServer::start(Vec::new());
    let mut runtime = AgentRuntime::with_dependencies(
        SessionStore::memory_only(),
        Box::new(StaticResolver {
            selection: test_chat_provider_selection(server.base_url.clone()),
        }),
        Box::new(executor),
        Box::new(ForcedToolPlanner {
            tool_name: "Ask".to_string(),
            // `text` becomes the prompt surfaced in the persisted request (P2-9).
            arguments: json!({ "text": "继续吗？", "description": "test" }),
        }),
        Box::new(DefaultTurnContextBuilder),
        Box::new(DefaultTurnTelemetryBuilder),
    );
    runtime.set_graph_run_store(Arc::clone(&store_arc));

    let result = runtime.run_turn_with_facts(
        TurnInput {
            message: "hi".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            workspace_mode: None,
            session_id: Some("session-ask-loop".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
        },
        RunTurnFacts {
            run_id: Some("run-ask-loop".to_string()),
            turn_id: Some("turn-ask-1".to_string()),
            workspace_root: Some(workspace.display().to_string()),
        },
    );

    // 1. The turn is suspended, not failed or completed.
    assert_eq!(result.phase, SUSPENDED_TURN_PHASE);
    assert!(result
        .fallback_reason
        .as_deref()
        .is_some_and(|reason| reason.contains("control_outcome_pending")));

    // 2. The pending request is persisted against the real session/run/turn.
    let dispatcher = runtime
        .governed_dispatcher()
        .expect("governed dispatcher must be present");
    let pending = dispatcher.pending_requests();
    assert_eq!(pending.len(), 1);
    assert_eq!(
        pending[0].request_kind,
        PendingControlRequestKind::Interaction
    );
    assert_eq!(pending[0].session_id.as_deref(), Some("session-ask-loop"));
    assert_eq!(pending[0].run_id.as_deref(), Some("run-ask-loop"));
    assert_eq!(pending[0].turn_id, "turn-ask-1");
    assert_eq!(pending[0].prompt.as_deref(), Some("继续吗？"));

    // 3. The graph run is WaitingUser with a bound ask wait.
    let store = store_arc.lock().unwrap();
    let run = store.load_run("run-ask-loop").expect("run must exist");
    assert_eq!(run.phase, GraphRunPhase::WaitingUser);
    let waits = GraphRunner::new().list_ask_waits(&store, "run-ask-loop");
    assert_eq!(waits.len(), 1);
    assert_eq!(waits[0].request_id, pending[0].request_id);
    assert_eq!(waits[0].expected_version, pending[0].version);
    drop(store);

    // 4. The provider was never contacted — no follow-up request after Ask suspension.
    let requests = server.finish();
    assert!(
        requests.is_empty(),
        "provider should not have been called: {requests:?}"
    );
}

#[test]
fn governed_ask_host_answer_resumes_injects_unique_terminal_result_with_original_call_id() {
    let _guard = crate::agent::runtime_helper::TestRuntimeGuard::leak();
    let workspace = temp_workspace_dir("ask-resume-loop");
    let executor = build_governed_executor(Some(workspace.clone()), None);
    let mut store = GraphRunStore::new();
    GraphRunner::new().start_run(
        &mut store,
        GraphEngine::new("state-machine-v1").start_run(
            "run-ask-resume",
            "ask resume flow",
            Some("session-ask-resume"),
        ),
    );
    let store_arc = Arc::new(Mutex::new(store));
    let server = MockHttpServer::start(Vec::new());
    let mut runtime = AgentRuntime::with_dependencies(
        SessionStore::memory_only(),
        Box::new(StaticResolver {
            selection: test_chat_provider_selection(server.base_url.clone()),
        }),
        Box::new(executor),
        Box::new(ForcedToolPlanner {
            tool_name: "Ask".to_string(),
            arguments: json!({ "text": "确认？", "description": "test" }),
        }),
        Box::new(DefaultTurnContextBuilder),
        Box::new(DefaultTurnTelemetryBuilder),
    );
    runtime.set_graph_run_store(Arc::clone(&store_arc));

    let result = runtime.run_turn_with_facts(
        TurnInput {
            message: "ask resume test".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            workspace_mode: None,
            session_id: Some("session-ask-resume".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
        },
        RunTurnFacts {
            run_id: Some("run-ask-resume".to_string()),
            turn_id: Some("turn-ask-resume-1".to_string()),
            workspace_root: Some(workspace.display().to_string()),
        },
    );
    assert_eq!(result.phase, SUSPENDED_TURN_PHASE);

    let dispatcher = runtime
        .governed_dispatcher()
        .expect("governed dispatcher must be present");
    let pending = dispatcher.pending_requests();
    assert_eq!(pending.len(), 1);
    let request_id = pending[0].request_id.clone();
    let expected_version = pending[0].version;

    // Host answers the Ask via the shared dispatcher.
    let authorization = crate::agent::dispatcher::ControlRequestAuthorization::for_request(
        &pending[0],
        Some(json!("确认继续")),
    );
    let consumed = dispatcher
        .answer_control_request(&request_id, &authorization)
        .expect("answer must succeed on shared dispatcher");
    assert_eq!(consumed.request.state, PendingControlRequestState::Consumed);
    assert_eq!(
        consumed.answer.as_ref().and_then(Value::as_str),
        Some("确认继续")
    );

    // Graph resume injects exactly one terminal result for the original call_id.
    let mut store = store_arc.lock().unwrap();
    let outcome = GraphRunner::new()
        .resume_ask_wait(
            &mut store,
            "run-ask-resume",
            &request_id,
            expected_version,
            json!("确认继续"),
        )
        .expect("graph resume must succeed");
    assert_eq!(outcome.call_id, pending[0].call_id);
    assert_eq!(
        outcome.terminal_result["output"]["answer"].as_str(),
        Some("确认继续")
    );
    assert_eq!(
        outcome.terminal_result["toolCallId"].as_str(),
        Some(pending[0].call_id.as_str())
    );
    assert!(GraphRunner::new()
        .list_ask_waits(&store, "run-ask-resume")
        .is_empty());
    drop(store);

    // The provider was never called (no follow-up after suspension).
    let requests = server.finish();
    assert!(
        requests.is_empty(),
        "provider should not have been called: {requests:?}"
    );
}

#[test]
fn governed_ask_resume_injects_terminal_result_into_next_turn_provider_context() {
    // PA-076 phase-4 P0: after the host answers a bound Ask and the graph resumes
    // (run -> Ready), the NEXT turn's provider request must carry the original assistant
    // tool-call + the terminal tool result (the answer) — injected as a completed tool
    // round, not a fresh user turn. The injection is one-shot.
    let _guard = crate::agent::runtime_helper::TestRuntimeGuard::leak();
    let workspace = temp_workspace_dir("ask-resume-inject-provider");
    let executor = build_governed_executor(Some(workspace.clone()), None);
    let mut store = GraphRunStore::new();
    GraphRunner::new().start_run(
        &mut store,
        GraphEngine::new("state-machine-v1").start_run(
            "run-ask-inject",
            "ask inject flow",
            Some("session-ask-inject"),
        ),
    );
    let store_arc = Arc::new(Mutex::new(store));
    // The resumed turn reaches the provider exactly once and receives a text completion.
    let server = MockHttpServer::start(vec![json_completion("好的，我已经看到你的回答，继续。")]);
    let mut runtime = AgentRuntime::with_dependencies(
        SessionStore::memory_only(),
        Box::new(StaticResolver {
            selection: test_chat_provider_selection(server.base_url.clone()),
        }),
        Box::new(executor),
        Box::new(AskOnceThenDeferPlanner {
            tool_name: "Ask".to_string(),
            arguments: json!({ "text": "继续吗？", "description": "test" }),
            ask_forced: std::sync::atomic::AtomicBool::new(false),
        }),
        Box::new(DefaultTurnContextBuilder),
        Box::new(DefaultTurnTelemetryBuilder),
    );
    runtime.set_graph_run_store(Arc::clone(&store_arc));

    // Turn 1: the planner forces the Ask tool -> dispatcher persists -> turn suspends and
    // the run is bound to WaitingUser. The provider is never called.
    let suspended = runtime.run_turn_with_facts(
        TurnInput {
            message: "start the ask flow".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            workspace_mode: None,
            session_id: Some("session-ask-inject".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
        },
        RunTurnFacts {
            run_id: Some("run-ask-inject".to_string()),
            turn_id: Some("turn-ask-inject-1".to_string()),
            workspace_root: Some(workspace.display().to_string()),
        },
    );
    assert_eq!(suspended.phase, SUSPENDED_TURN_PHASE);

    // Host answers through the shared dispatcher (CAS consumes the pending request).
    let dispatcher = runtime
        .governed_dispatcher()
        .expect("governed dispatcher must be present");
    let pending = dispatcher.pending_requests();
    assert_eq!(pending.len(), 1);
    let request_id = pending[0].request_id.clone();
    let expected_version = pending[0].version;
    let original_call_id = pending[0].call_id.clone();
    let authorization = crate::agent::dispatcher::ControlRequestAuthorization::for_request(
        &pending[0],
        Some(json!("继续执行")),
    );
    dispatcher
        .answer_control_request(&request_id, &authorization)
        .expect("answer must succeed on shared dispatcher");

    // Graph resume: run -> Ready and exactly one injection is persisted for the original
    // call id (the next turn consumes it).
    {
        let mut store = store_arc.lock().unwrap();
        let outcome = GraphRunner::new()
            .resume_ask_wait(
                &mut store,
                "run-ask-inject",
                &request_id,
                expected_version,
                json!("继续执行"),
            )
            .expect("graph resume must succeed");
        assert_eq!(outcome.call_id, original_call_id);
        assert_eq!(
            outcome.terminal_result["output"]["answer"].as_str(),
            Some("继续执行")
        );
        let run = store.load_run("run-ask-inject").expect("run present");
        assert_eq!(run.phase, GraphRunPhase::Ready);
        assert!(
            store.peek_ask_injection("run-ask-inject").is_some(),
            "the resume must persist the one-shot injection"
        );
    }

    // Turn 2: the resumed run consumes the one-shot injection. The provider request must
    // contain the original assistant tool-call + the terminal tool result (the answer).
    let resumed = runtime.run_turn_with_facts(
        TurnInput {
            message: "continue after the answer".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            workspace_mode: None,
            session_id: Some("session-ask-inject".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
        },
        RunTurnFacts {
            run_id: Some("run-ask-inject".to_string()),
            turn_id: Some("turn-ask-inject-2".to_string()),
            workspace_root: Some(workspace.display().to_string()),
        },
    );
    assert_ne!(
        resumed.phase, SUSPENDED_TURN_PHASE,
        "the resumed turn must not suspend again"
    );

    // The injection is one-shot: consumed (gone) after the resumed turn.
    assert!(
        store_arc
            .lock()
            .unwrap()
            .peek_ask_injection("run-ask-inject")
            .is_none(),
        "the injection must be consumed exactly once"
    );

    // The single provider request carries the injected pair.
    let requests = server.finish();
    assert_eq!(
        requests.len(),
        1,
        "provider should be called exactly once for the resumed turn: {requests:?}"
    );
    let body: Value = serde_json::from_str(&requests[0]).expect("request body json");
    let messages = body["messages"].as_array().expect("messages array");
    let assistant_index = messages
        .iter()
        .position(|message| {
            message.get("role").and_then(Value::as_str) == Some("assistant")
                && message
                    .get("tool_calls")
                    .and_then(Value::as_array)
                    .is_some_and(|calls| {
                        calls.iter().any(|call| {
                            call.get("id").and_then(Value::as_str)
                                == Some(original_call_id.as_str())
                        })
                    })
        })
        .expect("the original assistant tool-call must be injected into the request");
    let tool_result_index = messages
        .iter()
        .position(|message| {
            message.get("role").and_then(Value::as_str) == Some("tool")
                && message.get("tool_call_id").and_then(Value::as_str)
                    == Some(original_call_id.as_str())
                && message
                    .get("content")
                    .and_then(Value::as_str)
                    .is_some_and(|content| content.contains("继续执行"))
        })
        .expect("the terminal tool result with the answer must be injected into the request");
    assert!(
        tool_result_index > assistant_index,
        "the tool result must follow the assistant tool-call"
    );
}

#[test]
fn governed_ask_host_and_runtime_share_same_dispatcher() {
    // PA-076 P1-1: HostControlPlane built via `with_runtime` must share the runtime's
    // governed dispatcher so host `ask_answer` hits the pending request the runtime
    // persisted. This also exercises the `build()` auto-wiring: no explicit
    // `ask_dispatcher` is passed.
    let workspace = temp_workspace_dir("ask-shared-dispatch");
    let executor = build_governed_executor(Some(workspace.clone()), None);
    let runtime = AgentRuntime::with_dependencies(
        SessionStore::memory_only(),
        Box::new(ProviderRegistryStore::new()),
        Box::new(executor),
        Box::new(LocalTurnPlanner),
        Box::new(DefaultTurnContextBuilder),
        Box::new(DefaultTurnTelemetryBuilder),
    );

    let runtime_dispatcher = runtime
        .governed_dispatcher()
        .expect("runtime must have a governed dispatcher");

    // Control plane built with this runtime shares its dispatcher (no explicit
    // ask_dispatcher → auto-wired via build()).
    let control_plane = HostControlPlaneBuilder::new().runtime(runtime).build();

    // Same Arc pointer → host answer hits the runtime's store.
    assert!(Arc::ptr_eq(
        &control_plane.ask_dispatcher,
        &runtime_dispatcher
    ));

    // Dispatch an Ask through the shared dispatcher.
    let outcome = control_plane.ask_dispatcher.dispatch_governed(
        ToolDispatchRequest {
            origin: InvocationOrigin::Model,
            descriptor_id: "Ask".to_string(),
            call_id: "call-shared".to_string(),
            arguments: json!({ "text": "共享测试", "description": "test" }),
        },
        &DispatchContext {
            session_id: Some("session-shared".to_string()),
            run_id: Some("run-shared".to_string()),
            turn_id: Some("turn-shared".to_string()),
            ..Default::default()
        },
    );
    assert!(
        outcome.control_outcome.is_some(),
        "expected pending control outcome, got: {:?} / result={:?}",
        outcome.control_outcome,
        outcome.result
    );
    let request_id = outcome.control_outcome.unwrap().request_id;

    // Host answers through the control-plane surface — same dispatcher → CAS succeeds.
    let consumed = control_plane
        .answer_ask(&request_id, 1, json!("是的"))
        .expect("answer must hit the same pending request");
    let consumed_request = serde_json::from_value::<ControlRequestConsumed>(consumed)
        .expect("consumed request projection");
    assert_eq!(
        consumed_request.request.state,
        PendingControlRequestState::Consumed
    );
}

#[test]
fn governed_ask_stream_suspends_turn_and_binds_waiting_user_without_provider_followup() {
    // PA-076 P1-1: the stream path (`start_turn_stream_with_control_and_facts` →
    // `handle_stream_tool_turn`) must pause on a `control_outcome_pending` result exactly
    // like the sync path: bind the Ask wait to the graph run (→ `WaitingUser`), persist
    // the pending request against the real run/turn facts, emit the terminal
    // `turn:suspended` event, and never feed the marker back to the provider.
    let _guard = crate::agent::runtime_helper::TestRuntimeGuard::leak();
    let workspace = temp_workspace_dir("ask-stream-suspend-loop");
    let executor = build_governed_executor(Some(workspace.clone()), None);
    let mut store = GraphRunStore::new();
    GraphRunner::new().start_run(
        &mut store,
        GraphEngine::new("state-machine-v1").start_run(
            "run-ask-stream",
            "ask stream flow",
            Some("session-ask-stream"),
        ),
    );
    let store_arc = Arc::new(Mutex::new(store));
    // A fake provider that would panic if `provider_followup_stream` is called after Ask —
    // with zero queued responses the mock server's listener exits immediately, so any
    // follow-up connection is refused and the turn would fail instead of suspending.
    let server = MockHttpServer::start(Vec::new());
    let mut runtime = AgentRuntime::with_dependencies(
        SessionStore::memory_only(),
        Box::new(StaticResolver {
            selection: test_chat_provider_selection(server.base_url.clone()),
        }),
        Box::new(executor),
        Box::new(ForcedToolPlanner {
            tool_name: "Ask".to_string(),
            arguments: json!({ "text": "继续吗？", "description": "test" }),
        }),
        Box::new(DefaultTurnContextBuilder),
        Box::new(DefaultTurnTelemetryBuilder),
    );
    runtime.set_graph_run_store(Arc::clone(&store_arc));

    let sink = RecordingTurnEventSink::new();
    let control = ExecutionControlRegistry::new();
    control.register_turn(
        "turn-ask-stream-1",
        Some("session-ask-stream"),
        Some("run-ask-stream"),
    );
    runtime.start_turn_stream_with_control_and_facts(
        &sink,
        &control,
        "turn-ask-stream-1".to_string(),
        TurnInput {
            message: "hi".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            workspace_mode: None,
            session_id: Some("session-ask-stream".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
        },
        RunTurnFacts {
            run_id: Some("run-ask-stream".to_string()),
            turn_id: Some("turn-ask-stream-1".to_string()),
            workspace_root: Some(workspace.display().to_string()),
        },
    );

    // 1. The stream emitted a terminal `turn:suspended` event (never a completed/failed
    //    terminal) carrying the suspended phase.
    let events = sink.events.borrow();
    let event_names = || {
        events
            .iter()
            .map(|(name, _)| name.clone())
            .collect::<Vec<_>>()
    };
    assert!(
        events.iter().any(|(name, payload)| {
            name == "turn:suspended" && payload.phase.as_deref() == Some(SUSPENDED_TURN_PHASE)
        }),
        "stream should emit turn:suspended, got: {:?}",
        event_names()
    );
    assert!(
        !events.iter().any(|(name, _)| name == "turn:completed"),
        "stream must not emit turn:completed after Ask suspension: {:?}",
        event_names()
    );
    assert!(
        !events.iter().any(|(name, _)| name == "turn:failed"),
        "stream must not emit turn:failed after Ask suspension: {:?}",
        event_names()
    );
    drop(events);

    // 2. The pending request is persisted against the real session/run/turn.
    let dispatcher = runtime
        .governed_dispatcher()
        .expect("governed dispatcher must be present");
    let pending = dispatcher.pending_requests();
    assert_eq!(pending.len(), 1);
    assert_eq!(
        pending[0].request_kind,
        PendingControlRequestKind::Interaction
    );
    assert_eq!(pending[0].session_id.as_deref(), Some("session-ask-stream"));
    assert_eq!(pending[0].run_id.as_deref(), Some("run-ask-stream"));
    assert_eq!(pending[0].turn_id, "turn-ask-stream-1");
    assert_eq!(pending[0].prompt.as_deref(), Some("继续吗？"));

    // 3. The graph run is WaitingUser with a bound ask wait.
    let store = store_arc.lock().unwrap();
    let run = store.load_run("run-ask-stream").expect("run must exist");
    assert_eq!(run.phase, GraphRunPhase::WaitingUser);
    let waits = GraphRunner::new().list_ask_waits(&store, "run-ask-stream");
    assert_eq!(waits.len(), 1);
    assert_eq!(waits[0].request_id, pending[0].request_id);
    assert_eq!(waits[0].expected_version, pending[0].version);
    drop(store);

    // 4. The provider was never contacted — no follow-up request after Ask suspension.
    let requests = server.finish();
    assert!(
        requests.is_empty(),
        "provider should not have been called: {requests:?}"
    );
}

#[test]
fn governed_stream_normal_glob_tool_turn_completes() {
    // Regression: a normal (non-Ask) workspace tool
    // turn through the app's stream path must complete, not hang.
    let _guard = crate::agent::runtime_helper::TestRuntimeGuard::leak();
    let workspace = temp_workspace_dir("repro-glob-stream");
    std::fs::write(workspace.join("a.txt"), "hello").expect("write file");
    std::fs::write(workspace.join("b.md"), "world").expect("write file");
    let executor = build_governed_executor(Some(workspace.clone()), None);
    // One followup response (the assistant's final answer after the tool executes).
    let server = MockHttpServer::start(vec![json_completion("done listing files")]);
    let runtime = AgentRuntime::with_dependencies(
        SessionStore::memory_only(),
        Box::new(StaticResolver {
            selection: test_chat_provider_selection(server.base_url.clone()),
        }),
        Box::new(executor),
        Box::new(ForcedToolPlanner {
            tool_name: "workspace_glob_files".to_string(),
            arguments: json!({ "pattern": "**/*" }),
        }),
        Box::new(DefaultTurnContextBuilder),
        Box::new(DefaultTurnTelemetryBuilder),
    );
    let sink = RecordingTurnEventSink::new();
    let control = ExecutionControlRegistry::new();
    control.register_turn(
        "turn-repro-glob-1",
        Some("session-repro-glob"),
        Some("run-repro-glob"),
    );
    runtime.start_turn_stream_with_control_and_facts(
        &sink,
        &control,
        "turn-repro-glob-1".to_string(),
        TurnInput {
            message: "list files".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            workspace_mode: None,
            session_id: Some("session-repro-glob".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
        },
        RunTurnFacts {
            run_id: Some("run-repro-glob".to_string()),
            turn_id: Some("turn-repro-glob-1".to_string()),
            workspace_root: Some(workspace.display().to_string()),
        },
    );
    let events = sink.events.borrow();
    let names = || {
        events
            .iter()
            .map(|(name, _)| name.clone())
            .collect::<Vec<_>>()
    };
    assert!(
        events.iter().any(|(name, _)| name == "turn:completed"),
        "stream should complete a normal tool turn; got: {:?}",
        names()
    );
}

#[test]
fn governed_ask_stream_host_answer_resumes_injects_unique_terminal_result_with_original_call_id() {
    // PA-076 P1-1 end-to-end stream resume: the host answers the persisted Ask through the
    // shared dispatcher, then `resume_ask_wait` injects exactly one terminal result keyed
    // to the original assistant tool-call id.
    let _guard = crate::agent::runtime_helper::TestRuntimeGuard::leak();
    let workspace = temp_workspace_dir("ask-stream-resume-loop");
    let executor = build_governed_executor(Some(workspace.clone()), None);
    let mut store = GraphRunStore::new();
    GraphRunner::new().start_run(
        &mut store,
        GraphEngine::new("state-machine-v1").start_run(
            "run-ask-stream-resume",
            "ask stream resume flow",
            Some("session-ask-stream-resume"),
        ),
    );
    let store_arc = Arc::new(Mutex::new(store));
    let server = MockHttpServer::start(Vec::new());
    let mut runtime = AgentRuntime::with_dependencies(
        SessionStore::memory_only(),
        Box::new(StaticResolver {
            selection: test_chat_provider_selection(server.base_url.clone()),
        }),
        Box::new(executor),
        Box::new(ForcedToolPlanner {
            tool_name: "Ask".to_string(),
            arguments: json!({ "text": "确认？", "description": "test" }),
        }),
        Box::new(DefaultTurnContextBuilder),
        Box::new(DefaultTurnTelemetryBuilder),
    );
    runtime.set_graph_run_store(Arc::clone(&store_arc));

    let sink = RecordingTurnEventSink::new();
    let control = ExecutionControlRegistry::new();
    control.register_turn(
        "turn-ask-stream-resume-1",
        Some("session-ask-stream-resume"),
        Some("run-ask-stream-resume"),
    );
    runtime.start_turn_stream_with_control_and_facts(
        &sink,
        &control,
        "turn-ask-stream-resume-1".to_string(),
        TurnInput {
            message: "ask stream resume test".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            workspace_mode: None,
            session_id: Some("session-ask-stream-resume".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
        },
        RunTurnFacts {
            run_id: Some("run-ask-stream-resume".to_string()),
            turn_id: Some("turn-ask-stream-resume-1".to_string()),
            workspace_root: Some(workspace.display().to_string()),
        },
    );
    assert!(
        sink.events.borrow().iter().any(|(name, payload)| {
            name == "turn:suspended" && payload.phase.as_deref() == Some(SUSPENDED_TURN_PHASE)
        }),
        "stream should emit turn:suspended"
    );

    let dispatcher = runtime
        .governed_dispatcher()
        .expect("governed dispatcher must be present");
    let pending = dispatcher.pending_requests();
    assert_eq!(pending.len(), 1);
    let request_id = pending[0].request_id.clone();
    let expected_version = pending[0].version;
    let original_call_id = pending[0].call_id.clone();

    // Host answers the Ask via the shared dispatcher.
    let authorization = crate::agent::dispatcher::ControlRequestAuthorization::for_request(
        &pending[0],
        Some(json!("确认继续")),
    );
    let consumed = dispatcher
        .answer_control_request(&request_id, &authorization)
        .expect("answer must succeed on shared dispatcher");
    assert_eq!(consumed.request.state, PendingControlRequestState::Consumed);
    assert_eq!(
        consumed.answer.as_ref().and_then(Value::as_str),
        Some("确认继续")
    );

    // Graph resume injects exactly one terminal result for the original call_id.
    let mut store = store_arc.lock().unwrap();
    let outcome = GraphRunner::new()
        .resume_ask_wait(
            &mut store,
            "run-ask-stream-resume",
            &request_id,
            expected_version,
            json!("确认继续"),
        )
        .expect("graph resume must succeed");
    assert_eq!(outcome.call_id, original_call_id);
    assert_eq!(
        outcome.terminal_result["toolCallId"].as_str(),
        Some(original_call_id.as_str())
    );
    assert_eq!(
        outcome.terminal_result["output"]["answer"].as_str(),
        Some("确认继续")
    );
    assert!(GraphRunner::new()
        .list_ask_waits(&store, "run-ask-stream-resume")
        .is_empty());
    drop(store);

    // The provider was never called (no follow-up after suspension).
    let requests = server.finish();
    assert!(
        requests.is_empty(),
        "provider should not have been called: {requests:?}"
    );
}

fn temp_workspace_dir(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "pony-ask-runtime-test-{}-{}",
        name,
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("create temp workspace for ask test");
    root
}

/// PA-095 #3：对拍测试——事件折叠重建 vs 存储快照在约定字段集上逐项一致；
/// 豁免清单外零容忍；每项豁免有反向探针（断言差异当前确实存在，防豁免腐化）。
/// 场景统一走生产形态：control plane + SQLite store，事件从表读回
/// （observation ref / 终态注记均为落盘形态），重试吸收并行通道抢占噪声。
mod parity {
    use super::*;
    use crate::agent::control_plane::{
        ForkFromHistoryNodeCommand, HistoryGraphQuery, HostControlPlane, RunTurnCommand,
        StartTurnStreamCommand, SwitchHistoryBranchCommand,
    };
    use crate::agent::projection::parity::{
        AGREED_TIMELINE_ENTRY_FIELDS, AGREED_TRACE_FIELDS, EXEMPT_TIMELINE_KINDS,
    };
    use crate::agent::projection::{
        timeline_entry_field, trace_field, Projection, TraceProjectionState,
    };
    use crate::agent::session::TraceTimelineEntry;

    /// 生产形态场景脚手架：构建 control plane + SQLite store，执行 `run` 闭包，
    /// 读回表内事件与会话 trace。终态事件在表才算成功（重试吸收通道抢占）。
    fn scenario(
        name: &str,
        responses: Vec<MockHttpResponse>,
        expected_terminal: &str,
        customize: &dyn Fn(&mut AgentRuntime),
        run: &dyn Fn(&HostControlPlane),
    ) -> (
        Vec<crate::agent::turn_event::TurnEvent>,
        TurnTraceRecord,
        String,
    ) {
        let _rt_guard = crate::agent::runtime_helper::TestRuntimeGuard::new();
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("pony-parity-{name}-{stamp}"));
        fs::create_dir_all(&dir).expect("mkdir");

        let mut outcome: Option<(Vec<crate::agent::turn_event::TurnEvent>, TurnTraceRecord)> = None;
        for attempt in 0..8 {
            let sessions = SessionStore::with_backend(Box::new(
                crate::agent::sqlite_session::SqliteSessionBackend::new_with_trace_mode(
                    dir.join(format!("sessions-{attempt}.db")),
                    crate::agent::session::SeparateTraceTableMode::WriteSeparate,
                ),
            ));
            let server = MockHttpServer::start(responses.clone());
            let mut runtime = AgentRuntime::with_dependencies(
                sessions,
                Box::new(StaticResolver {
                    selection: test_provider_selection(server.base_url.clone()),
                }),
                Box::new(crate::agent::tools::ToolRouter::new()),
                Box::new(LocalTurnPlanner),
                Box::new(DefaultTurnContextBuilder),
                Box::new(DefaultTurnTelemetryBuilder),
            );
            customize(&mut runtime);
            let control_plane = HostControlPlane::with_runtime(runtime);
            // PA-095 #3：会话绑定路由——把本控制面的事件/flush 通道绑到本场景
            // session 上，全局默认单槽被并行测试构建覆盖不再影响事件落盘
            // （守卫在 attempt 结束时解绑）。
            let _persist_guard = crate::agent::turn_flow::bind_event_persist_session(
                name,
                control_plane.event_persist_channel(),
            );
            let _flush_guard = crate::agent::turn_flow::bind_event_flush_session(
                name,
                control_plane.event_flush_channel(),
            );
            run(&control_plane);
            drop(_persist_guard);
            drop(_flush_guard);
            server.finish();

            let store = control_plane
                .load_turn_events_checked(name, None)
                .expect("event table readable");
            let terminal_reached = store
                .last()
                .map(|(_, _, event)| event.type_name() == expected_terminal)
                .unwrap_or(false);
            if !terminal_reached {
                continue;
            }
            let turn_id = store
                .iter()
                .rev()
                .find_map(|(_, _, event)| event.turn_id().map(str::to_string))
                .expect("turn-scoped event");
            let trace = control_plane
                .load_session_traces(name)
                .into_iter()
                .find(|trace| trace.turn_id == turn_id)
                .unwrap_or_else(|| panic!("stored trace missing for {turn_id}"));
            let events = store.into_iter().map(|(_, _, event)| event).collect();
            outcome = Some((events, trace));
            break;
        }
        let _ = fs::remove_dir_all(&dir);
        let (events, trace) =
            outcome.unwrap_or_else(|| panic!("scenario {name} never reached terminal"));
        let turn_id = events
            .iter()
            .rev()
            .find_map(|event| event.turn_id().map(str::to_string))
            .expect("turn id");
        (events, trace, turn_id)
    }

    fn rebuilt_trace(
        events: &[crate::agent::turn_event::TurnEvent],
        turn_id: &str,
    ) -> TurnTraceRecord {
        let mut state = TraceProjectionState::init();
        for (index, event) in events.iter().enumerate() {
            TraceProjectionState::apply(&mut state, index as u64 + 1, event);
        }
        state
            .trace_for_turn(turn_id)
            .unwrap_or_else(|| panic!("rebuilt trace missing for turn {turn_id}"))
    }

    /// 对拍断言：约定字段集逐项一致 + 过滤豁免 kind 后保序。
    /// `expect_text=false` 消费 sync_call_model_text 豁免（同步入口无 chunk 流）。
    fn assert_parity(stored: &TurnTraceRecord, rebuilt: &TurnTraceRecord, expect_text: bool) {
        // 豁免消费：terminal_no_usage_turn_provider_metadata——failed/cancelled
        // turn 无 provider/usage 事件（无已完成模型调用），provider 元数据不可重建
        // （EXEMPTIONS 登记项；反向探针在 failed/cancelled 场景单独断言其存在）。
        let skip_provider = matches!(stored.phase.as_str(), "failed" | "cancelled")
            && rebuilt.provider_name.is_none();
        for field in AGREED_TRACE_FIELDS {
            if skip_provider && (*field == "provider_name" || *field == "provider_model") {
                continue;
            }
            assert_eq!(
                trace_field(stored, field),
                trace_field(rebuilt, field),
                "trace field `{field}` diverges (stored vs rebuilt)"
            );
        }
        let filter_exempt = |timeline: &[TraceTimelineEntry]| -> Vec<TraceTimelineEntry> {
            timeline
                .iter()
                .filter(|entry| !EXEMPT_TIMELINE_KINDS.contains(&entry.kind.as_str()))
                .cloned()
                .collect()
        };
        let stored_timeline = filter_exempt(&stored.trace_timeline);
        let rebuilt_timeline = filter_exempt(&rebuilt.trace_timeline);
        assert_eq!(
            stored_timeline
                .iter()
                .map(|entry| entry.kind.clone())
                .collect::<Vec<_>>(),
            rebuilt_timeline
                .iter()
                .map(|entry| entry.kind.clone())
                .collect::<Vec<_>>(),
            "timeline kind order diverges"
        );
        for (stored_entry, rebuilt_entry) in stored_timeline.iter().zip(rebuilt_timeline.iter()) {
            for field in AGREED_TIMELINE_ENTRY_FIELDS {
                if !expect_text && *field == "text" {
                    continue;
                }
                // 豁免消费：timeline.call_tool_text——text 语义分叉（EXEMPTIONS 登记项）。
                if *field == "text" && stored_entry.kind == "call_tool" {
                    continue;
                }
                // 豁免消费：timeline.failed_last_hop_state——failed turn 末 hop
                // state 差异（EXEMPTIONS 登记项；反向探针单独断言其存在）。
                if *field == "state"
                    && stored.phase == "failed"
                    && stored_entry.kind == "call_model"
                    && stored_entry.sequence
                        == stored_timeline.last().map(|e| e.sequence).unwrap_or(0)
                {
                    continue;
                }
                assert_eq!(
                    timeline_entry_field(stored_entry, field),
                    timeline_entry_field(rebuilt_entry, field),
                    "timeline field `{field}` diverges on kind {}",
                    stored_entry.kind
                );
            }
        }
        // 豁免消费：turn_duration_ms_clock_drift——毫秒级漂移容差（≤250ms）。
        match (stored.turn_duration_ms, rebuilt.turn_duration_ms) {
            (Some(stored_ms), Some(rebuilt_ms)) => {
                let drift = stored_ms.abs_diff(rebuilt_ms);
                assert!(
                    drift <= 250,
                    "turn_duration_ms drift {drift}ms exceeds clock tolerance"
                );
            }
            _ => {}
        }
    }

    /// 场景：无工具同步 turn——约定字段集对拍 + 时钟/title/checkpoint 装饰
    /// 反向探针。
    #[test]
    fn parity_no_tool_sync_turn() {
        let (events, stored, turn_id) = scenario(
            "parity-no-tool",
            vec![json_response(json!({
                "choices": [{"message": {"role": "assistant", "content": "无工具对拍答案。"}}],
                "usage": {"prompt_tokens": 7, "completion_tokens": 3, "total_tokens": 10}
            }))],
            "turn/end",
            &|_runtime| {},
            &|control_plane| {
                let _ = control_plane.run_turn(RunTurnCommand {
                    input: TurnInput {
                        message: "无工具对拍问题".to_string(),
                        display_message: None,
                        provider_id: None,
                        model_id: None,
                        reasoning_effort: None,
                        workspace_mode: None,
                        session_id: Some("parity-no-tool".to_string()),
                        node_id: None,
                        history: Vec::new(),
                        images: Vec::new(),
                        workspace_id: None,
                    },
                });
            },
        );
        let rebuilt = rebuilt_trace(&events, &turn_id);
        assert_parity(&stored, &rebuilt, false);

        // 反向探针：时钟字段豁免差异当前确实存在。
        assert_ne!(
            stored.updated_at, 0,
            "probe(clock): stored carries updated_at"
        );
        assert_eq!(
            rebuilt.updated_at, 0,
            "probe(clock): rebuilt lacks updated_at"
        );
        // 反向探针：checkpoint_persist 运行时装饰条目豁免。
        assert!(
            stored
                .trace_timeline
                .iter()
                .any(|entry| entry.kind == "checkpoint_persist"),
            "probe(checkpoint_persist): stored carries decoration entry"
        );
        assert!(
            !rebuilt
                .trace_timeline
                .iter()
                .any(|entry| entry.kind == "checkpoint_persist"),
            "probe(checkpoint_persist): rebuild lacks it (exemption live)"
        );
        // 反向探针：timeline.build_context_provider_metadata——差异当前确实存在
        //（存储侧 build_context 条目携带 provider 元数据，事件无承载——豁免防腐化）。
        let stored_context_entry = stored
            .trace_timeline
            .iter()
            .find(|entry| entry.kind == "build_context")
            .expect("probe(bctx-meta): stored build_context entry");
        let rebuilt_context_entry = rebuilt
            .trace_timeline
            .iter()
            .find(|entry| entry.kind == "build_context")
            .expect("probe(bctx-meta): rebuilt build_context entry");
        assert!(
            stored_context_entry.provider_name.is_some(),
            "probe(bctx-meta): stored build_context carries provider metadata"
        );
        assert!(
            rebuilt_context_entry.provider_name.is_none(),
            "probe(bctx-meta): rebuild cannot recover provider metadata (exemption live)"
        );
    }

    /// 场景：单工具流式 turn——chunk 文本归属与 tool 条目对拍。
    #[test]
    fn parity_single_tool_stream_turn() {
        let final_text = "单工具对拍最终答案。";
        let (events, stored, turn_id) = scenario(
            "parity-tool",
            vec![
                sse_decision_tool_call("workspace_list_files", json!({"path": "."})),
                sse_response(&[
                    json!({"choices": [{"delta": {"content": final_text}}]}),
                    json!({"choices": [], "usage": {"prompt_tokens": 5, "completion_tokens": 2, "total_tokens": 7}}),
                ]),
            ],
            "turn/end",
            &|_runtime| {},
            &|control_plane| {
                let sink = RecordingTurnEventSink::new();
                control_plane.start_turn_stream(
                    &sink,
                    StartTurnStreamCommand {
                        turn_id: "parity-tool-turn".to_string(),
                        input: TurnInput {
                            message: "单工具对拍问题".to_string(),
                            display_message: None,
                            provider_id: None,
                            model_id: None,
                            reasoning_effort: None,
                            workspace_mode: None,
                            session_id: Some("parity-tool".to_string()),
                            node_id: None,
                            history: Vec::new(),
                            images: Vec::new(),
                            workspace_id: None,
                        },
                    },
                );
            },
        );
        let rebuilt = rebuilt_trace(&events, &turn_id);
        {
            let mut state = TraceProjectionState::init();
            for (index, event) in events.iter().enumerate() {
                TraceProjectionState::apply(&mut state, index as u64 + 1, event);
                let kinds: Vec<String> = state
                    .trace_for_turn(turn_id.as_str())
                    .map(|trace| {
                        trace
                            .trace_timeline
                            .iter()
                            .map(|entry| entry.kind.clone())
                            .collect()
                    })
                    .unwrap_or_default();
                eprintln!("[probe] after {} -> {:?}", event.type_name(), kinds);
            }
        }
        eprintln!(
            "[probe] table events: {:?}",
            events
                .iter()
                .map(|event| event.type_name())
                .collect::<Vec<_>>()
        );
        assert_parity(&stored, &rebuilt, true);

        // 反向探针：流式场景 text 在约定字段集内且非空（chunk 折叠生效）。
        let rebuilt_model_text = rebuilt
            .trace_timeline
            .iter()
            .filter(|entry| entry.kind == "call_model")
            .filter_map(|entry| entry.text.clone())
            .any(|text| text.contains(final_text));
        assert!(
            rebuilt_model_text,
            "probe(text): streaming chunks aggregate into rebuilt call_model"
        );
    }

    /// 场景：failed 同步 turn（checkpoint hook fail-turn——该路径有存储 trace）；
    /// 反向探针断言 failed 末 hop state 豁免差异当前确实存在
    /// （stored=error vs rebuilt=completed）。
    #[test]
    fn parity_failed_sync_turn_and_last_hop_probe() {
        let (events, stored, turn_id) = scenario(
            "parity-failed",
            vec![json_completion("失败对拍答案")],
            "turn/end",
            &|runtime| {
                runtime.set_hook_executor_for_test(Box::new(FailingHookExecutor));
                let mut descriptor = observe_hook_descriptor(
                    "observe.parity-checkpoint-failturn",
                    10,
                    TurnHookPoint::CheckpointPersistEnd,
                );
                descriptor.default_failure_policy = HookFailurePolicy::FailTurn;
                descriptor.allowed_failure_policies =
                    vec![HookFailurePolicy::Degrade, HookFailurePolicy::FailTurn];
                runtime
                    .register_hook_descriptor(descriptor)
                    .expect("register parity checkpoint failturn hook");
            },
            &|control_plane| {
                let _ = control_plane.run_turn(RunTurnCommand {
                    input: TurnInput {
                        message: "失败对拍问题".to_string(),
                        display_message: None,
                        provider_id: None,
                        model_id: None,
                        reasoning_effort: None,
                        workspace_mode: None,
                        session_id: Some("parity-failed".to_string()),
                        node_id: None,
                        history: Vec::new(),
                        images: Vec::new(),
                        workspace_id: None,
                    },
                });
            },
        );
        let rebuilt = rebuilt_trace(&events, &turn_id);
        assert_parity(&stored, &rebuilt, false);

        // 反向探针：failed 末 hop state 豁免差异当前确实存在。
        let last_state = |trace: &TurnTraceRecord| {
            trace
                .trace_timeline
                .iter()
                .filter(|entry| entry.kind == "call_model")
                .last()
                .map(|entry| entry.state.clone())
        };
        assert_eq!(
            last_state(&stored).as_deref(),
            Some("error"),
            "probe(failed_last_hop): stored marks error"
        );
        assert_eq!(
            last_state(&rebuilt).as_deref(),
            Some("completed"),
            "probe(failed_last_hop): rebuild still yields completed (exemption live)"
        );
    }

    /// 场景：多 turn 同步会话——事件按 turn 分组折叠，逐 turn 独立对拍。
    #[test]
    fn parity_multi_turn_sync_session() {
        let _rt_guard = crate::agent::runtime_helper::TestRuntimeGuard::new();
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("pony-parity-multiturn-{stamp}"));
        fs::create_dir_all(&dir).expect("mkdir");

        let mut pairs: Option<Vec<(String, TurnTraceRecord, TurnTraceRecord)>> = None;
        for attempt in 0..8 {
            let sessions = SessionStore::with_backend(Box::new(
                crate::agent::sqlite_session::SqliteSessionBackend::new_with_trace_mode(
                    dir.join(format!("sessions-{attempt}.db")),
                    crate::agent::session::SeparateTraceTableMode::WriteSeparate,
                ),
            ));
            let server = MockHttpServer::start(vec![
                json_response(json!({
                    "choices": [{"message": {"role": "assistant", "content": "第一轮答案。"}}],
                    "usage": {"prompt_tokens": 4, "completion_tokens": 2, "total_tokens": 6}
                })),
                json_response(json!({
                    "choices": [{"message": {"role": "assistant", "content": "第二轮答案。"}}],
                    "usage": {"prompt_tokens": 6, "completion_tokens": 2, "total_tokens": 8}
                })),
            ]);
            let runtime = AgentRuntime::with_dependencies(
                sessions,
                Box::new(StaticResolver {
                    selection: test_provider_selection(server.base_url.clone()),
                }),
                Box::new(crate::agent::tools::ToolRouter::new()),
                Box::new(LocalTurnPlanner),
                Box::new(DefaultTurnContextBuilder),
                Box::new(DefaultTurnTelemetryBuilder),
            );
            let control_plane = HostControlPlane::with_runtime(runtime);
            for message in ["第一轮问题", "第二轮问题"] {
                let _ = control_plane.run_turn(RunTurnCommand {
                    input: TurnInput {
                        message: message.to_string(),
                        display_message: None,
                        provider_id: None,
                        model_id: None,
                        reasoning_effort: None,
                        workspace_mode: None,
                        session_id: Some("parity-multi-turn".to_string()),
                        node_id: None,
                        history: Vec::new(),
                        images: Vec::new(),
                        workspace_id: None,
                    },
                });
            }
            server.finish();

            let events = control_plane
                .load_turn_events_checked("parity-multi-turn", None)
                .expect("event table readable");
            let ends = events
                .iter()
                .filter(|(_, _, event)| event.type_name() == "turn/end")
                .count();
            if ends < 2 {
                continue;
            }
            // 全事件一次折叠（seq 升序），逐 turn 取重建产物。
            let mut state = TraceProjectionState::init();
            for (index, (_, _, event)) in events.iter().enumerate() {
                TraceProjectionState::apply(&mut state, index as u64 + 1, event);
            }
            let traces = control_plane.load_session_traces("parity-multi-turn");
            let mut collected: Vec<(String, TurnTraceRecord, TurnTraceRecord)> = Vec::new();
            let mut seen: Vec<String> = Vec::new();
            for (_, _, event) in &events {
                let Some(turn_id) = event.turn_id() else {
                    continue;
                };
                if seen.iter().any(|id| id == turn_id) {
                    continue;
                }
                seen.push(turn_id.to_string());
                let Some(stored) = traces.iter().find(|trace| trace.turn_id == turn_id) else {
                    continue;
                };
                let Some(rebuilt) = state.trace_for_turn(turn_id) else {
                    continue;
                };
                collected.push((turn_id.to_string(), stored.clone(), rebuilt));
            }
            if collected.len() == 2 {
                pairs = Some(collected);
                break;
            }
        }
        let _ = fs::remove_dir_all(&dir);
        let pairs = pairs.expect("multi-turn scenario never completed both turns");
        assert_eq!(pairs.len(), 2, "two turns paired");
        for (turn_id, stored, rebuilt) in &pairs {
            assert_parity(stored, rebuilt, false);
            assert_eq!(stored.phase, "completed", "turn {turn_id} phase");
        }
    }

    /// 场景：多 hop 流式 turn（3 次 provider call + 2 次工具）——多 hop
    /// timeline 对拍（call_model 条目数 = hop 数，含 return_result）。
    #[test]
    fn parity_multi_hop_stream_turn() {
        let (events, stored, turn_id) = scenario(
            "parity-multi-hop",
            vec![
                sse_decision_tool_call("workspace_list_files", json!({"path": "."})),
                sse_response(&[
                    json!({"choices": [{"delta": {"content": "找到了，继续读取。"}}]}),
                    json!({"choices": [{"delta": {"tool_calls": [
                        {"index": 0, "id": "call_read_file", "type": "function",
                         "function": {"name": "workspace_read_file", "arguments": "{\"path\":\"tauri.conf.json\"}"}}
                    ]}}]}),
                ]),
                sse_response(&[
                    json!({"choices": [{"delta": {"content": "多 hop 对拍最终答案。"}}]}),
                    json!({"choices": [], "usage": {"prompt_tokens": 9, "completion_tokens": 3, "total_tokens": 12}}),
                ]),
            ],
            "turn/end",
            &|_runtime| {},
            &|control_plane| {
                let sink = RecordingTurnEventSink::new();
                control_plane.start_turn_stream(
                    &sink,
                    StartTurnStreamCommand {
                        turn_id: "parity-multi-hop-turn".to_string(),
                        input: TurnInput {
                            message: "多 hop 对拍问题".to_string(),
                            display_message: None,
                            provider_id: None,
                            model_id: None,
                            reasoning_effort: None,
                            workspace_mode: None,
                            session_id: Some("parity-multi-hop".to_string()),
                            node_id: None,
                            history: Vec::new(),
                            images: Vec::new(),
                            workspace_id: None,
                        },
                    },
                );
            },
        );
        let rebuilt = rebuilt_trace(&events, &turn_id);
        assert_parity(&stored, &rebuilt, true);

        // 反向探针：多 hop 重建 call_model 条目数 = provider call 数。
        let rebuilt_hops = rebuilt
            .trace_timeline
            .iter()
            .filter(|entry| entry.kind == "call_model")
            .count();
        assert_eq!(
            rebuilt_hops, 3,
            "rebuilt call_model entries equal hop count"
        );
    }

    /// 冒烟回归探针：多 hop 工具流式 turn 落库后的会话历史——assistant 条目
    /// content 只能是最终模型文本；工具调用参数 JSON / 结果文本 / 描述不得进入
    /// 对话正文（否则前端恢复会话时会把工具内容当对话消息渲染）。
    #[test]
    fn history_after_multi_hop_tool_turn_keeps_tool_content_out_of_assistant_text() {
        let _rt_guard = crate::agent::runtime_helper::TestRuntimeGuard::new();
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("pony-history-tool-leak-{stamp}"));
        fs::create_dir_all(&dir).expect("mkdir");

        const SESSION: &str = "history-tool-leak";
        let mut outcome: Option<Vec<crate::agent::session::TurnHistoryMessage>> = None;
        for attempt in 0..8 {
            let sessions = SessionStore::with_backend(Box::new(
                crate::agent::sqlite_session::SqliteSessionBackend::new_with_trace_mode(
                    dir.join(format!("sessions-{attempt}.db")),
                    crate::agent::session::SeparateTraceTableMode::WriteSeparate,
                ),
            ));
            let server = MockHttpServer::start(vec![
                sse_decision_tool_call("workspace_list_files", json!({"path": "."})),
                sse_response(&[
                    json!({"choices": [{"delta": {"content": "找到了，继续读取。"}}]}),
                    json!({"choices": [{"delta": {"tool_calls": [
                        {"index": 0, "id": "call_read_file", "type": "function",
                         "function": {"name": "workspace_read_file", "arguments": "{\"path\":\"tauri.conf.json\"}"}}
                    ]}}]}),
                ]),
                sse_response(&[
                    json!({"choices": [{"delta": {"content": "历史泄漏探针最终答案。"}}]}),
                    json!({"choices": [], "usage": {"prompt_tokens": 9, "completion_tokens": 3, "total_tokens": 12}}),
                ]),
            ]);
            let runtime = AgentRuntime::with_dependencies(
                sessions,
                Box::new(StaticResolver {
                    selection: test_provider_selection(server.base_url.clone()),
                }),
                Box::new(crate::agent::tools::ToolRouter::new()),
                Box::new(LocalTurnPlanner),
                Box::new(DefaultTurnContextBuilder),
                Box::new(DefaultTurnTelemetryBuilder),
            );
            let control_plane = HostControlPlane::with_runtime(runtime);
            let _persist_guard = crate::agent::turn_flow::bind_event_persist_session(
                SESSION,
                control_plane.event_persist_channel(),
            );
            let _flush_guard = crate::agent::turn_flow::bind_event_flush_session(
                SESSION,
                control_plane.event_flush_channel(),
            );
            control_plane.start_turn_stream(
                &RecordingTurnEventSink::new(),
                StartTurnStreamCommand {
                    turn_id: "history-tool-leak-turn".to_string(),
                    input: TurnInput {
                        message: "历史泄漏探针问题".to_string(),
                        display_message: None,
                        provider_id: None,
                        model_id: None,
                        reasoning_effort: None,
                        workspace_mode: None,
                        session_id: Some(SESSION.to_string()),
                        node_id: None,
                        history: Vec::new(),
                        images: Vec::new(),
                        workspace_id: None,
                    },
                },
            );
            drop(_persist_guard);
            drop(_flush_guard);
            server.finish();

            let events = control_plane
                .load_turn_events_checked(SESSION, None)
                .expect("event table readable");
            let terminal_reached = events
                .last()
                .map(|(_, _, event)| event.type_name() == "turn/end")
                .unwrap_or(false);
            if !terminal_reached {
                continue;
            }
            let snapshot = control_plane.load_session_snapshot(
                crate::agent::control_plane::SessionSnapshotQuery {
                    session_id: Some(SESSION.to_string()),
                },
            );
            outcome = Some(snapshot.history);
            break;
        }
        let _ = fs::remove_dir_all(&dir);
        let history = outcome.expect("multi-hop tool turn scenario never reached terminal");

        let assistants: Vec<&crate::agent::session::TurnHistoryMessage> = history
            .iter()
            .filter(|message| message.role == "assistant")
            .collect();
        assert_eq!(assistants.len(), 1, "one assistant history entry");
        let assistant_content = assistants[0].content.as_str();
        assert_eq!(
            assistant_content, "历史泄漏探针最终答案。",
            "assistant history content must be the final model text only"
        );
        for leak_marker in [
            "workspace_list_files",
            "workspace_read_file",
            "\"path\"",
            "先调用",
            "找到了，继续读取",
            "tauri.conf.json",
        ] {
            assert!(
            !assistant_content.contains(leak_marker),
            "assistant history content must not contain tool-call info ({leak_marker}): {assistant_content:?}"
        );
        }
    }

    /// 场景：plan 前 cancelled 流式 turn——phase=cancelled 对拍；重建不虚构
    /// 未发生的模型调用（settle 兜底收窄）。
    #[test]
    fn parity_cancelled_stream_turn() {
        let _rt_guard = crate::agent::runtime_helper::TestRuntimeGuard::new();
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("pony-parity-cancel-{stamp}"));
        fs::create_dir_all(&dir).expect("mkdir");

        let mut outcome: Option<(Vec<crate::agent::turn_event::TurnEvent>, TurnTraceRecord)> = None;
        for attempt in 0..8 {
            let control = ExecutionControlRegistry::new();
            let sessions = SessionStore::with_backend(Box::new(
                crate::agent::sqlite_session::SqliteSessionBackend::new_with_trace_mode(
                    dir.join(format!("sessions-{attempt}.db")),
                    crate::agent::session::SeparateTraceTableMode::WriteSeparate,
                ),
            ));
            let server = MockHttpServer::start(vec![json_completion("不应被消费的响应")]);
            let runtime = AgentRuntime::with_dependencies(
                sessions,
                Box::new(StaticResolver {
                    selection: test_provider_selection(server.base_url.clone()),
                }),
                Box::new(crate::agent::tools::ToolRouter::new()),
                Box::new(LocalTurnPlanner),
                Box::new(DefaultTurnContextBuilder),
                Box::new(DefaultTurnTelemetryBuilder),
            );
            let control_plane = HostControlPlaneBuilder::new()
                .runtime(runtime)
                .execution_control(control.clone())
                .build();
            // PA-095 #3：会话绑定路由（同 scenario()——抗全局单槽抢占）。
            let _persist_guard = crate::agent::turn_flow::bind_event_persist_session(
                "parity-cancel",
                control_plane.event_persist_channel(),
            );
            let _flush_guard = crate::agent::turn_flow::bind_event_flush_session(
                "parity-cancel",
                control_plane.event_flush_channel(),
            );
            control.register_turn("parity-cancel-turn", Some("parity-cancel"), None);
            assert!(control.request_stop("parity-cancel-turn").accepted);
            control_plane.start_turn_stream(
                &crate::agent::turn_flow::NoopTurnEventSink,
                StartTurnStreamCommand {
                    turn_id: "parity-cancel-turn".to_string(),
                    input: TurnInput {
                        message: "取消对拍问题".to_string(),
                        display_message: None,
                        provider_id: None,
                        model_id: None,
                        reasoning_effort: None,
                        workspace_mode: None,
                        session_id: Some("parity-cancel".to_string()),
                        node_id: None,
                        history: Vec::new(),
                        images: Vec::new(),
                        workspace_id: None,
                    },
                },
            );
            // PA-095 #3：cancelled-before-plan 不发起 provider HTTP 请求，
            // MockHttpServer::finish 会 join 永远阻塞在 accept 的线程——挂死。
            // 直接 drop（JoinHandle 释放，阻塞线程随进程退出回收）。
            drop(server);

            let events = control_plane
                .load_turn_events_checked("parity-cancel", None)
                .expect("event table readable");
            let terminal_reached = events
                .last()
                .map(|(_, _, event)| event.type_name() == "turn/end")
                .unwrap_or(false);
            if !terminal_reached {
                continue;
            }
            let trace = control_plane
                .load_session_traces("parity-cancel")
                .into_iter()
                .find(|trace| trace.turn_id == "parity-cancel-turn")
                .expect("stored cancelled trace");
            outcome = Some((events.into_iter().map(|(_, _, e)| e).collect(), trace));
            break;
        }
        let _ = fs::remove_dir_all(&dir);
        let (events, stored) = outcome.expect("cancelled scenario never reached terminal");
        let rebuilt = rebuilt_trace(&events, "parity-cancel-turn");
        assert_parity(&stored, &rebuilt, false);

        // 反向探针：plan 前 cancelled——settle 兜底不虚构模型调用（timeline 的
        // call_model 条目来自 step/start 事件本身，非结算补建），且中断调用
        // 不得显示为 completed（PA-095 #3 收敛修复的防腐探针）。
        assert_eq!(stored.phase, "cancelled", "probe(cancelled): stored phase");
        for entry in rebuilt
            .trace_timeline
            .iter()
            .filter(|entry| entry.kind == "call_model")
        {
            assert_eq!(
                entry.state, "cancelled",
                "probe(cancelled): interrupted model call must not be marked completed"
            );
        }
        // 反向探针：terminal_no_usage_turn_provider_metadata——差异当前确实存在
        // （存储侧携带 provider 元数据，事件流无 usage 不可重建；豁免防腐化）。
        assert!(
            stored.provider_name.is_some(),
            "probe(provider-meta): stored cancelled trace carries provider metadata"
        );
        assert!(
            rebuilt.provider_name.is_none(),
            "probe(provider-meta): rebuild cannot recover provider metadata (exemption live)"
        );
    }

    /// 场景：fork-checkout 跨分支——分支命令发射 history-control 事件且日志水位
    /// 严格递增（#6 无 ABA 的端到端证明），分支后各分支上的 turn 事件折叠与
    /// 存储 trace 对拍一致（分支可见性不影响 per-turn 事实折叠）。
    #[test]
    fn parity_fork_checkout_branch_turns() {
        let _rt_guard = crate::agent::runtime_helper::TestRuntimeGuard::new();
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("pony-parity-branch-{stamp}"));
        fs::create_dir_all(&dir).expect("mkdir");

        struct BranchOutcome {
            events: Vec<(u64, String, crate::agent::turn_event::TurnEvent)>,
            traces: Vec<TurnTraceRecord>,
            fork_cursor_version: u64,
            switch_cursor_version: u64,
        }
        let mut outcome: Option<BranchOutcome> = None;
        for attempt in 0..8 {
            let sessions = SessionStore::with_backend(Box::new(
                crate::agent::sqlite_session::SqliteSessionBackend::new_with_trace_mode(
                    dir.join(format!("sessions-{attempt}.db")),
                    crate::agent::session::SeparateTraceTableMode::WriteSeparate,
                ),
            ));
            let server = MockHttpServer::start(vec![
                json_response(json!({
                    "choices": [{"message": {"role": "assistant", "content": "主干答案。"}}],
                    "usage": {"prompt_tokens": 4, "completion_tokens": 2, "total_tokens": 6}
                })),
                json_response(json!({
                    "choices": [{"message": {"role": "assistant", "content": "分支答案。"}}],
                    "usage": {"prompt_tokens": 6, "completion_tokens": 2, "total_tokens": 8}
                })),
            ]);
            let runtime = AgentRuntime::with_dependencies(
                sessions,
                Box::new(StaticResolver {
                    selection: test_provider_selection(server.base_url.clone()),
                }),
                Box::new(crate::agent::tools::ToolRouter::new()),
                Box::new(LocalTurnPlanner),
                Box::new(DefaultTurnContextBuilder),
                Box::new(DefaultTurnTelemetryBuilder),
            );
            let control_plane = HostControlPlane::with_runtime(runtime);
            let _persist_guard = crate::agent::turn_flow::bind_event_persist_session(
                "parity-branch",
                control_plane.event_persist_channel(),
            );
            let _flush_guard = crate::agent::turn_flow::bind_event_flush_session(
                "parity-branch",
                control_plane.event_flush_channel(),
            );
            let run = |control_plane: &HostControlPlane, message: &str| {
                control_plane
                    .run_turn(RunTurnCommand {
                        input: TurnInput {
                            message: message.to_string(),
                            display_message: None,
                            provider_id: None,
                            model_id: None,
                            reasoning_effort: None,
                            workspace_mode: None,
                            session_id: Some("parity-branch".to_string()),
                            node_id: None,
                            history: Vec::new(),
                            images: Vec::new(),
                            workspace_id: None,
                        },
                    })
                    .phase
            };
            if run(&control_plane, "第一问") != "ready" {
                continue;
            }

            // 从最近节点 fork 出新分支；立即用 fork 响应的版本回切主干（#6 乐观锁
            // round-trip：响应版本即下次通行版本，中间不得有 turn 使其过期）；
            // 再二次 fork 出工作分支跑分支 turn。
            let graph = control_plane.load_history_graph(HistoryGraphQuery {
                session_id: Some("parity-branch".to_string()),
            });
            let Some(head_node) = graph.nodes.last() else {
                continue;
            };
            let fork = control_plane
                .fork_from_history_node(ForkFromHistoryNodeCommand {
                    session_id: Some("parity-branch".to_string()),
                    node_id: head_node.node_id.clone(),
                    expected_cursor_version: None,
                })
                .expect("fork succeeds");
            let fork_expected_version = fork.cursor.cursor_version.unwrap_or(0);
            control_plane
                .switch_history_branch(SwitchHistoryBranchCommand {
                    session_id: Some("parity-branch".to_string()),
                    branch_id: "branch-main".to_string(),
                    expected_cursor_version: Some(fork_expected_version),
                })
                .expect("switch with fork-returned cursor version (round-trip)");
            let fork_work = control_plane
                .fork_from_history_node(ForkFromHistoryNodeCommand {
                    session_id: Some("parity-branch".to_string()),
                    node_id: head_node.node_id.clone(),
                    expected_cursor_version: None,
                })
                .expect("work branch fork succeeds");
            if run(&control_plane, "第二问（分支）") != "ready" {
                continue;
            }
            server.finish();

            let events = control_plane
                .load_turn_events_checked("parity-branch", None)
                .expect("event table readable");
            let ends = events
                .iter()
                .filter(|(_, _, event)| event.type_name() == "turn/end")
                .count();
            let has_fork_event = events
                .iter()
                .any(|(_, _, e)| e.type_name() == "fork/created");
            let has_checkout_event = events
                .iter()
                .any(|(_, _, e)| e.type_name() == "checkpoint/checkout");
            if ends < 2 || !has_fork_event || !has_checkout_event {
                continue;
            }
            // 水位单调性探针：history-control 事件 seq 落在其触发的 turn 事件之后
            // （日志只增不减，命令事实与 turn 事实同一条单调日志）。
            let first_end_seq = events
                .iter()
                .find(|(_, _, e)| e.type_name() == "turn/end")
                .map(|(seq, _, _)| *seq)
                .expect("first turn/end");
            let fork_seq = events
                .iter()
                .find(|(_, _, e)| e.type_name() == "fork/created")
                .map(|(seq, _, _)| *seq)
                .expect("fork/created seq");
            assert!(
                fork_seq > first_end_seq,
                "probe(monotonic): fork/created lands after prior turn facts"
            );
            let traces = control_plane.load_session_traces("parity-branch");
            outcome = Some(BranchOutcome {
                events,
                traces,
                fork_cursor_version: fork.cursor.cursor_version.unwrap_or(0),
                switch_cursor_version: fork_work.cursor.cursor_version.unwrap_or(0),
            });
            break;
        }
        drop(_rt_guard);
        let _ = fs::remove_dir_all(&dir);
        let outcome = outcome.expect("branch scenario never completed");

        // #6 无 ABA 端到端：后续命令版本严格大于先前命令版本（命令事件推进日志水位；
        // switch 的 round-trip 通过本身已验证"响应版本即下次通行版本"）。
        assert!(
            outcome.switch_cursor_version > outcome.fork_cursor_version,
            "probe(no-aba): cursor version strictly increases across history commands ({} -> {})",
            outcome.fork_cursor_version,
            outcome.switch_cursor_version
        );

        // 全事件一次折叠，逐 turn 对拍（跨分支 turn 各自独立成立）。
        let mut state = TraceProjectionState::init();
        for (index, (_, _, event)) in outcome.events.iter().enumerate() {
            TraceProjectionState::apply(&mut state, index as u64 + 1, event);
        }
        let mut seen: Vec<String> = Vec::new();
        for (_, _, event) in &outcome.events {
            let Some(turn_id) = event.turn_id() else {
                continue;
            };
            if seen.iter().any(|id| id == turn_id) {
                continue;
            }
            seen.push(turn_id.to_string());
            let Some(stored) = outcome.traces.iter().find(|trace| trace.turn_id == turn_id) else {
                continue;
            };
            let Some(rebuilt) = state.trace_for_turn(turn_id) else {
                continue;
            };
            assert_parity(stored, &rebuilt, false);
            assert_eq!(stored.phase, "completed", "turn {turn_id} phase");
        }
        assert_eq!(seen.len(), 2, "two turns across branches paired");
    }
}
