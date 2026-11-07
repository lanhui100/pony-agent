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
        tool_calls: Vec::new(),
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
        &crate::agent::provider::ProviderProtocol::AnthropicMessages,
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
        let forced_call = ToolCall {
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
        };
        Some(ProviderDecision {
            output_text: String::new(),
            // D1a：复数视图与单值字段同源派生（永不漂移）。
            tool_calls: crate::agent::provider::tool_calls_from_single(&Some(
                forced_call.clone(),
            )),
            tool_call: Some(forced_call),
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
            return Some({
                let forced_call = ToolCall {
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
                };
                ProviderDecision {
                    output_text: String::new(),
                    // D1a：复数视图与单值字段同源派生（永不漂移）。
                    tool_calls: crate::agent::provider::tool_calls_from_single(&Some(
                        forced_call.clone(),
                    )),
                    tool_call: Some(forced_call),
                    reasoning_content: None,
                    reasoning_content_value: None,
                    assistant_message: None,
                    provider_source: "planner_preflight".to_string(),
                    provider_mode: "preflight".to_string(),
                    fallback_reason: None,
                    token_usage: None,
                }
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
        let _ = self.control.request_stop(
            &self.turn_id,
            crate::agent::stop_escalation::StopReason::User,
        );
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
        crate::agent::hooks::NoopHookExecutor.execute(descriptor, hook_point)
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
        crate::agent::hooks::NoopHookExecutor.execute(descriptor, hook_point)
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
    /// 响应前的确定性延迟（毫秒）——用于锁定「墙钟预算」类时序行为；默认 0。
    delay_ms: u64,
}


/// wave-13：mock HTTP server 用例的**进程级串行闸门**（可重入：同一测试线程开多个 server 只持有一次）。
///
/// 背景（CI 复现）：`MockHttpServer` 的响应队列与预期请求严格 1:1；CI 负载下 provider 请求可能
/// 超时并触发**客户端重试**，重试连接在队列耗尽后会被拒绝 → 回合失败 → 事件流断言缺失
/// （`start_turn_stream_*` / `control_plane` 用例间歇性失败）。串行化 mock 用例消除负载竞争，
/// 从而消除「超时→重试→队列失配」。仅影响 `#[cfg(test)]` 构建。
static MOCK_SERVER_GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());

thread_local! {
    static MOCK_SERVER_GATE_DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static MOCK_SERVER_GATE_GUARD: std::cell::RefCell<Option<std::sync::MutexGuard<'static, ()>>> =
        const { std::cell::RefCell::new(None) };
}

pub(crate) struct MockServerGate;

impl MockServerGate {
    pub(crate) fn acquire() -> Self {
        MOCK_SERVER_GATE_DEPTH.with(|depth| {
            if depth.get() == 0 {
                let guard = MOCK_SERVER_GATE
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                MOCK_SERVER_GATE_GUARD.with(|slot| *slot.borrow_mut() = Some(guard));
            }
            depth.set(depth.get() + 1);
        });
        Self
    }
}

impl Drop for MockServerGate {
    fn drop(&mut self) {
        MOCK_SERVER_GATE_DEPTH.with(|depth| {
            depth.set(depth.get().saturating_sub(1));
            if depth.get() == 0 {
                MOCK_SERVER_GATE_GUARD.with(|slot| {
                    *slot.borrow_mut() = None;
                });
            }
        });
    }
}

/// wave-35/C2: mock server thread wait budget. A queue mismatch must fail the single test
/// (with expected/actual diagnostics) instead of hanging the process-wide gate forever.
/// 依据:对齐已冻结的 NFR 基准 `deadlock_watchdog_timeout_ms = 5000`(`.dev-team/nfr-baseline.json`);
/// 合法用例下末条响应被消费后 mock 线程立即退出,等待在毫秒级返回,缩短预算只压缩"队列失配"的
/// 暴露时间,不改变判定语义。
pub(crate) const MOCK_SERVER_FINISH_TIMEOUT: std::time::Duration =
    std::time::Duration::from_millis(5000);

/// wave-35/C2: bounded wait for a background thread to exit; returns `false` on timeout
/// (never silently swallows it -- caller panics with diagnostics so the gate is released).
pub(crate) fn wait_for_mock_thread(
    handle: &thread::JoinHandle<()>,
    timeout: std::time::Duration,
) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while !handle.is_finished() {
        if std::time::Instant::now() >= deadline {
            return false;
        }
        thread::sleep(std::time::Duration::from_millis(5));
    }
    true
}

/// wave-35/C2: bounded-wait primitive -- a stuck mock thread must be reported as
/// "not finished" within the budget instead of blocking the caller (and the process-wide gate).
#[test]
fn wait_for_mock_thread_reports_timeout_within_budget() {
    let stuck = thread::spawn(|| thread::sleep(std::time::Duration::from_secs(30)));
    let started = std::time::Instant::now();
    let finished = wait_for_mock_thread(&stuck, std::time::Duration::from_millis(50));
    assert!(!finished, "stuck thread must be reported as not finished");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "bounded wait must return without blocking: {:?}",
        started.elapsed()
    );
}

/// 归属令牌发号器：进程内单调递增，保证同一测试进程里任意两个 mock server 的令牌不同。
static MOCK_SERVER_OWNER_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// 生成一个新的 mock server 归属令牌（进程内唯一）。
fn next_mock_owner_token() -> String {
    let seq = MOCK_SERVER_OWNER_SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    format!("own{}-{seq}", std::process::id())
}

pub(crate) struct MockHttpServer {
    base_url: String,
    /// 本 server 的归属令牌（同时出现在 `base_url` 的路径段里）。
    owner_token: String,
    requests: Arc<Mutex<Vec<String>>>,
    handle: Option<thread::JoinHandle<()>>,
    /// wave-35/C2: mock queue length; finish() reports expected/actual on timeout.
    expected: usize,
    /// wave-13：持有串行闸门（drop 时释放），消除并行负载导致的「超时→重试→队列失配」。
    _gate: MockServerGate,
    /// PA-139：shutdown 标志——finish() 置位后 accept 轮询在 1ms 粒度内退出，
    /// mock 线程可立即 join（非阻塞 accept 轮询使标志位即终止信号，无需显式关监听）。
    shutdown: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl MockHttpServer {
    fn start(responses: Vec<MockHttpResponse>) -> Self {
        // wave-36 PA-139：默认谓词恒 true（不做**业务**层面的请求内容筛选）；
        // 但「归属」不再依赖谓词——见 start_with_predicate 里的 owner token 路径校验：
        // 非本用例的迟到/杂散连接一律 503 且不消费队列。
        Self::start_with_predicate(responses, Arc::new(|_: &str| true))
    }

    /// wave-36 PA-139 归属校验：predicate 收到完整请求体时返回该请求是否属于本用例；
    /// 不匹配的请求（如其它用例迟到重试/杂散探测命中本端口）返回确定性 503 且**不消费队列**。
    ///
    /// wave-47（PA-139 遗留 flake）：谓词只解决「内容可判定」的归属；默认谓词恒 true，
    /// 因此**结构层面**的归属必须另有依据，否则迟到连接会合法吃掉一个响应槽位。做法：
    /// 每个 server 生成进程内唯一的 owner token 并放进自己的 `base_url` 路径段
    /// （`http://host/v1/<token>`），provider 拼接出的请求行必然带它；任何不属于本用例的
    /// 连接（上一个用例 finish() 后仍在退避重试的请求、端口被内核复用后打进来的迟到请求）
    /// 带的都是**它自己的旧 token**，请求行匹配不上 → 确定性 503 且不消费队列。
    fn start_with_predicate(
        responses: Vec<MockHttpResponse>,
        predicate: Arc<dyn Fn(&str) -> bool + Send + Sync>,
    ) -> Self {
        let gate = MockServerGate::acquire();
        let expected = responses.len();
        let owner_token = next_mock_owner_token();
        let owner_token_for_thread = owner_token.clone();
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
        let address = listener.local_addr().expect("mock server addr");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let requests_for_thread = Arc::clone(&requests);
        // PA-139：`Arc<TcpListener>` + shutdown 标志——队列耗尽后 mock 线程**不再退出**，
        // listener 保持打开直到 `finish()` 置位 shutdown。旧实现「队列耗尽→线程退出→
        // listener 关闭」在 finish() 前留下一个 connect refused 窗口：并行负载下用例自身
        // 请求若晚于队列被异常消费到达，1-2ms 内即 `client error (Connect)` 假失败。
        // shutdown 前到达的多余连接返回确定性 503 且**不消费队列**（归属校验：非用例
        // 预期序列的连接无权取走响应）。
        let listener = std::sync::Arc::new(listener);
        let listener_for_thread = std::sync::Arc::clone(&listener);
        let shutdown = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let shutdown_for_thread = std::sync::Arc::clone(&shutdown);

        let handle = thread::spawn(move || {
            // PA-139：accept 循环持续到 shutdown；队列耗尽仅停止**发响应**，不关监听。
            let listener = listener_for_thread;
            loop {
                let stream = {
                    // 非阻塞轮询 accept：shutdown 置位后循环可在 1ms 粒度内退出
                    // （阻塞式 accept 无法被 shutdown 打断）。
                    listener.set_nonblocking(true).expect("set_nonblocking mock");
                    match listener.accept() {
                        Ok((stream, _)) => {
                            listener
                                .set_nonblocking(false)
                                .expect("unset_nonblocking mock");
                            // **Windows 会继承 non-blocking 标志**：accept() 返回的 socket
                            // 在 Windows 上沿用 listener 的 non-blocking 状态，而 Linux 上
                            // accept() 的 fd 恒为阻塞（不继承 O_NONBLOCK）。因此只把
                            // listener 设回阻塞**不够**，必须显式把 accepted stream 也设回
                            // 阻塞。否则 Windows 下 `read_http_request_body` 首次 read 立即
                            // WouldBlock → Err 分支 `return None` → 连接被无响应丢弃
                            // （Windows 发 RST，Linux 通常仅 FIN）→ 客户端侧
                            // `client error (SendRequest)` → 流式决策静默 fallback 到同步
                            // → 同步请求消费掉下一 hop 的 SSE 体 → `expected value at
                            // line 1 column 1`。这是 windows-latest CI 专属失败的根因。
                            stream
                                .set_nonblocking(false)
                                .expect("set_blocking accepted mock stream");
                            stream
                        }
                        Err(ref error)
                            if error.kind() == std::io::ErrorKind::WouldBlock =>
                        {
                            if shutdown_for_thread.load(std::sync::atomic::Ordering::SeqCst) {
                                return;
                            }
                            std::thread::sleep(std::time::Duration::from_millis(1));
                            continue;
                        }
                        Err(_) => {
                            if shutdown_for_thread.load(std::sync::atomic::Ordering::SeqCst) {
                                return;
                            }
                            std::thread::sleep(std::time::Duration::from_millis(1));
                            continue;
                        }
                    }
                };
                if shutdown_for_thread.load(std::sync::atomic::Ordering::SeqCst) {
                    continue;
                }
                let mut stream = stream;
                let Some(request) = read_http_request_body(&mut stream) else {
                    // 中途断开/半请求：忽略，不消费队列（wave-13 语义保留）。
                    continue;
                };
                // wave-47 归属校验（优先级 0）：请求行必须带本 server 的 owner token，
                // 否则视为「其它用例的迟到/杂散连接」——确定性 503 且**绝不消费队列**。
                // 这一步把 PA-139 依赖「谓词 + 端口不复用」的赌运气式归属，换成 URL 里
                // 可验证的显式归属：即便内核把本 server 的端口复用给了迟到重试，旧连接的
                // 请求行带的是旧 token，不会命中。
                if !request.request_line.contains(&owner_token_for_thread) {
                    eprintln!(
                        "[pony-agent][mock] 拒绝非本用例请求（归属令牌不匹配，不消费响应队列）: {}",
                        request.request_line
                    );
                    let _ = stream.write_all(
                        b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\
                          Connection: close\r\n\r\n",
                    );
                    continue;
                }
                let body = request.body;
                let consumed = requests_for_thread.lock().unwrap().len();
                // wave-36 PA-139：归属校验——先判断请求是否属于本用例，不属于则确定性
                // 503、不消费队列（与队列耗尽路径同款）。predicate 拿到完整请求体。
                if !predicate(&body) {
                    let _ = stream.write_all(
                        b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\
                          Connection: close\r\n\r\n",
                    );
                    continue;
                }
                if consumed >= responses.len() {
                    // PA-139 归属校验：队列已耗尽（或已被本用例外的连接消费）——
                    // 该连接不是用例预期的 provider 请求，确定性 503、不消费队列。
                    let _ = stream.write_all(
                        b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\
                          Connection: close\r\n\r\n",
                    );
                    continue;
                }
                let response = responses[consumed].clone();
                requests_for_thread.lock().unwrap().push(body);
                write_http_response(&mut stream, response);
            }
        });

        Self {
            // wave-47：owner token 作为路径段注入——provider 一律按
            // `format!("{}/chat/completions", base_url.trim_end_matches('/'))` 拼接，
            // 故此段必定出现在请求行里，供 mock 做归属校验。
            base_url: format!("http://{}/v1/{}", address, owner_token),
            owner_token,
            requests,
            handle: Some(handle),
            expected,
            _gate: gate,
            shutdown,
        }
    }

    // wave-35/C2；PA-139：finish 显式 shutdown——置位标志，accept 轮询在 1ms 粒度
    // 内退出；随后有界 join（预算不变 5000ms）。
    fn finish(mut self) -> Vec<String> {
        if let Some(handle) = self.handle.take() {
            self.shutdown
                .store(true, std::sync::atomic::Ordering::SeqCst);
            if !wait_for_mock_thread(&handle, MOCK_SERVER_FINISH_TIMEOUT) {
                let actual = self.requests.lock().map(|r| r.len()).unwrap_or_default();
                panic!(
                    "mock server thread still running after {MOCK_SERVER_FINISH_TIMEOUT:?}: \
                     expected={} requests, actual={} -- mock queue not drained \
                     (provider aborted early / retries exhausted the queue). \
                     Bounded join keeps this a single-test failure instead of hanging the suite.",
                    self.expected, actual
                );
            }
            handle.join().expect("join mock server");
        }
        self.requests.lock().unwrap().clone()
    }
}

/// 裸 HTTP 请求（不经 provider），供归属校验回归用例精确构造「本用例请求 / 外来迟到请求」。
fn send_raw_http_request(host: &str, port: u16, path: &str, body: &str) -> String {
    let mut stream = TcpStream::connect((host, port)).expect("connect mock server");
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(request.as_bytes())
        .expect("write raw mock request");
    stream.flush().expect("flush raw mock request");
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .expect("read raw mock response");
    response
}

/// wave-47 回归（PA-139 遗留 flake）：**非本用例的迟到/杂散请求绝不允许吃掉响应队列**。
///
/// 复刻真实失败形态：上一个用例 `finish()` 之后其 provider 仍在退避重试（0/500/1000/2000ms），
/// 端口被内核复用给下一个用例的 mock → 迟到请求命中新 server。修复前默认谓词恒 true，
/// 迟到请求会**合法消费一个响应槽位**，导致本用例自身请求落入「队列已耗尽」503 分支，
/// 整轮失败（本机实测 ~17% 整库 flake，两次红均为此形态：
/// `decision_stream:attempt=1..5/5 failed class=TransientRetryable … 503 Service Unavailable`）。
///
/// 本用例把该竞争**确定性**复现：先发一个「旧 token」的迟到请求，断言它被 503 且不消费队列；
/// 再发本用例自身请求，断言它仍拿到队列里的那条响应。旧实现下第一次断言即红。
#[test]
fn mock_server_rejects_foreign_request_without_consuming_queue() {
    let server = MockHttpServer::start(vec![json_completion("owned-response")]);
    let url = url::Url::parse(&server.base_url).expect("parse mock base_url");
    let host = url.host_str().expect("mock host").to_string();
    let port = url.port().expect("mock port");
    // 归属令牌必须真的落在 base_url 路径段里——这是 provider 请求行携带令牌的前提。
    assert!(
        server
            .base_url
            .ends_with(&format!("/{}", server.owner_token)),
        "owner token 必须注入 base_url 路径段，实际 base_url={}",
        server.base_url
    );
    let own_path = format!("/v1/{}/chat/completions", server.owner_token);

    // 1) 外来迟到请求：带的是**别的** server 的令牌（这里显式构造一个不同令牌）。
    let foreign = send_raw_http_request(
        &host,
        port,
        "/v1/own-other-test-999/chat/completions",
        "{\"marker\":\"foreign\"}",
    );
    assert!(
        foreign.starts_with("HTTP/1.1 503"),
        "外来请求必须被归属校验拒绝为 503，实际响应：{foreign}"
    );
    let consumed_by_foreign = server.requests.lock().unwrap().clone();
    assert!(
        consumed_by_foreign.is_empty(),
        "外来请求绝不允许消费响应队列，实际已消费：{consumed_by_foreign:?}"
    );

    // 2) 本用例自身请求：请求行带本 server 令牌 → 正常拿到队列里的响应。
    let owned = send_raw_http_request(&host, port, &own_path, "{\"marker\":\"owned\"}");
    assert!(
        owned.starts_with("HTTP/1.1 200"),
        "本用例请求必须正常服务，实际响应：{owned}"
    );
    assert!(
        owned.contains("owned-response"),
        "本用例请求必须拿到队列里的那条响应，实际响应：{owned}"
    );

    let expected = server.expected;
    let recorded = server.finish();
    assert_eq!(
        recorded.len(),
        1,
        "队列必须恰好被本用例自身请求消费一次，实际记录 {recorded:?}"
    );
    assert!(
        recorded[0].contains("owned") && !recorded[0].contains("foreign"),
        "记录的请求体必须是本用例自身请求（外来请求不得入队），实际记录 {recorded:?}"
    );
    assert_eq!(expected, 1);
}

/// wave-47：mock 收到的一次请求。
///
/// - `request_line`：请求行（`POST /v1/<owner-token>/chat/completions HTTP/1.1`），
///   用于**归属令牌**校验（PA-139 遗留 flake：迟到/杂散连接吃掉响应队列）。
/// - `body`：既有语义不变——只含 `Content-Length` 切片，谓词与用例断言完全不受影响。
struct MockHttpRequest {
    request_line: String,
    body: String,
}

/// wave-13：读请求改为**优雅失败**（`None`）。CI 负载下客户端可能提前断开/重复建连，
/// 旧实现 `read(...).expect(...)` 会 panic 打死 mock 线程 → 后续重试连接被拒 → 用例假失败。
fn read_http_request_body(stream: &mut TcpStream) -> Option<MockHttpRequest> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 4096];
    let mut header_end = None;
    let mut content_length = 0usize;

    loop {
        let read = match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => read,
            Err(_) => return None,
        };
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

    let end = header_end?;
    let head = String::from_utf8_lossy(&buffer[..end]);
    let request_line = head.lines().next().unwrap_or("").trim().to_string();
    Some(MockHttpRequest {
        request_line,
        body: String::from_utf8_lossy(&buffer[end..end + content_length]).to_string(),
    })
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
    if response.delay_ms > 0 {
        std::thread::sleep(std::time::Duration::from_millis(response.delay_ms));
    }
    let payload = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        response.content_type,
        response.body.len(),
        response.body
    );
    let _ = stream.write_all(payload.as_bytes());
    let _ = stream.flush();
}

fn json_response(body: serde_json::Value) -> MockHttpResponse {
    MockHttpResponse {
        content_type: "application/json",
        body: body.to_string(),
        delay_ms: 0,
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
        delay_ms: 0,
    }
}

fn test_provider_selection(base_url: String) -> ResolvedProviderSelection {
    ResolvedProviderSelection {
        requested_name: "test-openai".to_string(),
        provider_name: "test-openai".to_string(),
        protocol: crate::agent::provider::ProviderProtocol::OpenAiCompletions,
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
        protocol: crate::agent::provider::ProviderProtocol::OpenAiCompletions,
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
        protocol: crate::agent::provider::ProviderProtocol::OpenAiCompletions,
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
        tool_authorization_mode: None,
        session_id: Some("planner-trace-session".to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
        plan_execution_approval: None,
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
        tool_authorization_mode: None,
        session_id: Some("capability-trace-session".to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
        plan_execution_approval: None,
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
        tool_authorization_mode: None,
        session_id: Some("capability-hook-rewrite".to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
        plan_execution_approval: None,
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
        tool_authorization_mode: None,
        session_id: Some("planner-preflight-rewrite".to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
        plan_execution_approval: None,
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
        tool_authorization_mode: None,
        session_id: Some("planner-tool-selection-rewrite".to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
        plan_execution_approval: None,
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
        tool_authorization_mode: None,
        session_id: Some("skill-hook-rewrite".to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
        plan_execution_approval: None,
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
            tool_authorization_mode: None,
            session_id: Some("session-unstable-boundary-not-dispatched".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
            plan_execution_approval: None,
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
            tool_authorization_mode: None,
            session_id: Some("session-hook-failturn".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
            plan_execution_approval: None,
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
            tool_authorization_mode: None,
            session_id: Some("session-tool-start-failturn".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
            plan_execution_approval: None,
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
            tool_authorization_mode: None,
            session_id: Some("session-tool-end-failturn".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
            plan_execution_approval: None,
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
            tool_authorization_mode: None,
            session_id: Some("session-checkpoint-failturn".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
            plan_execution_approval: None,
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
    // wave-36 PA-139：归属校验——本用例独有的用户消息（provider 请求 volatile 输入必含），
    // 杂散连接（其它用例迟到重试命中本端口）被 503 拒绝而不消费队列。
    let server = MockHttpServer::start_with_predicate(
        vec![sse_response(&[
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
        ])],
        std::sync::Arc::new(|body: &str| {
            body.contains("会在 finalize boundary failturn 的问题")
        }),
    );
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
            tool_authorization_mode: None,
            session_id: Some("session-finalize-failturn".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
            plan_execution_approval: None,
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
        tool_authorization_mode: None,
        session_id: Some("sync-model-failturn".to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
        plan_execution_approval: None,
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
        tool_authorization_mode: None,
        session_id: Some("sync-tool-start-failturn".to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
        plan_execution_approval: None,
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
        tool_authorization_mode: None,
        session_id: Some("sync-tool-end-failturn".to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
        plan_execution_approval: None,
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
        tool_authorization_mode: None,
        session_id: Some("sync-hook-trace-terminal".to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
        plan_execution_approval: None,
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
        tool_authorization_mode: None,
        session_id: Some("sync-checkpoint-failturn".to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
        plan_execution_approval: None,
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
        tool_authorization_mode: None,
        session_id: Some("sync-finalize-failturn".to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
        plan_execution_approval: None,
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
            tool_authorization_mode: None,
            session_id: Some("stream-hook-trace-terminal".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
            plan_execution_approval: None,
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


/// B017：失败轮保留已产出正文 —— `fail_stream_turn_with_hook_dispatch` 的 `partial_text` 透传。
///
/// 覆盖两面：
/// 1. `Some(正文)` ⇒ `turn:failed` 事件 text **含**该正文（trace `pa_01a1a134` turn-3 场景：
///    工具反复失败直到熔断，用户不该只看到 "This turn failed."）；
/// 2. `None` ⇒ 事件 text 逐字仍是 `This turn failed.`（历史冻结断言的兼容锁）。
#[test]
fn fail_stream_turn_with_hook_dispatch_forwards_partial_text() {
    let runtime = AgentRuntime::new();
    let control = crate::agent::execution_control::ExecutionControlRegistry::new();

    // ① Some(正文)
    control.register_turn("turn-b017-with-text", None, None);
    let sink = RecordingTurnEventSink::new();
    runtime.fail_stream_turn_with_hook_dispatch(
        &sink,
        &control,
        None,
        "turn-b017-with-text",
        "user message",
        None,
        None,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        None,
        None,
        1,
        "provider exploded".to_string(),
        Some("已经产出的一段正文".to_string()),
    );
    let events = sink.events.borrow();
    let failed = events
        .iter()
        .find(|(name, _)| name == "turn:failed")
        .unwrap_or_else(|| panic!("必须发出 turn:failed，实际事件={:?}", events.len()));
    let with_text = failed.1.text.clone().unwrap_or_default();
    drop(events);
    eprintln!("[b017] partial_text=Some ⇒ turn:failed text={with_text:?}");
    assert!(
        with_text.contains("已经产出的一段正文"),
        "B017：失败事件正文必须保留已产出内容，实际 text={with_text:?}"
    );
    assert_ne!(
        with_text, "This turn failed.",
        "B017：有正文时不得退化成占位文案"
    );

    // ② None（兼容锁）
    control.register_turn("turn-b017-no-text", None, None);
    let sink_without = RecordingTurnEventSink::new();
    runtime.fail_stream_turn_with_hook_dispatch(
        &sink_without,
        &control,
        None,
        "turn-b017-no-text",
        "user message",
        None,
        None,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        None,
        None,
        1,
        "provider exploded".to_string(),
        None,
    );
    let events = sink_without.events.borrow();
    let failed = events
        .iter()
        .find(|(name, _)| name == "turn:failed")
        .expect("必须发出 turn:failed");
    let without_text = failed.1.text.clone().unwrap_or_default();
    drop(events);
    eprintln!("[b017] partial_text=None ⇒ turn:failed text={without_text:?}");
    assert_eq!(
        without_text, "This turn failed.",
        "B017：无正文时必须逐字保持既有占位文案（兼容锁）"
    );
}

/// D1a：`tool_calls` 与 `tool_call` 在所有构造点**同源派生、永不漂移**。
///
/// 取一个真实的 src 内构造点（`ForcedToolPlanner::preflight_decision`，见上方）作为代表：
/// 它走 `tool_calls_from_single(&Some(call))` 派生路径，断言两字段严格一致。
#[test]
fn provider_decision_tool_calls_matches_single_tool_call() {
    let planner = ForcedToolPlanner {
        tool_name: "Read".to_string(),
        arguments: json!({ "path": "Cargo.toml" }),
    };
    let decision = planner
        .preflight_decision("read it", &[], &[])
        .expect("D1a：forced planner 必须给出决策");

    let single = decision
        .tool_call
        .as_ref()
        .expect("D1a：该构造点必须带 tool_call");
    // `ToolCall` 未实现 PartialEq ⇒ 逐字段比对（call_id / name / arguments / plan）。
    let from_plural = decision
        .tool_calls
        .first()
        .expect("D1a：有 tool_call 时 tool_calls 不得为空");
    eprintln!(
        "[D1a] tool_call={:?} tool_calls_len={} plural={:?}",
        single.name,
        decision.tool_calls.len(),
        from_plural.name
    );
    assert_eq!(
        decision.tool_calls.len(),
        1,
        "D1a：有 tool_call 的构造点，tool_calls 必须恰 1 个"
    );
    assert_eq!(from_plural.call_id, single.call_id, "D1a：call_id 不得漂移");
    assert_eq!(from_plural.name, single.name, "D1a：name 不得漂移");
    assert_eq!(
        from_plural.arguments, single.arguments,
        "D1a：arguments 不得漂移"
    );
    assert_eq!(
        from_plural.plan.as_ref().map(|plan| plan.kind.clone()),
        single.plan.as_ref().map(|plan| plan.kind.clone()),
        "D1a：plan.kind 不得漂移"
    );
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
            tool_authorization_mode: None,
            session_id: Some("test-session".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
            plan_execution_approval: None,
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
    // 注：`AgentRuntime::new()`（DesktopRuntimePreset）自 b8cba07 起总是装配生产沙箱后端
    // （Native 可用优先，否则 HostApprovedUnsandboxed），"无沙箱"状态只能经显式 override
    // 注入（与 governed_executor 的 `governed_executor_registers_production_sandbox_backend_for_run`
    // 同一验证入口），故此处用 builder + NoSandboxBackend override 构造 runtime 而非 new()。
    use crate::agent::governed_executor::build_governed_executor_with_sandbox_backend;
    use crate::agent::sandbox::NoSandboxBackend;
    use crate::agent::tool_runtime::SandboxBackend;
    let executor = build_governed_executor_with_sandbox_backend(
        None,
        None,
        Some(Arc::new(NoSandboxBackend) as Arc<dyn SandboxBackend>),
    );
    let runtime = AgentRuntimeBuilder::desktop()
        .tool_executor(Box::new(executor))
        .build();
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
    let response = control.request_stop(
        "turn-cancelled",
        crate::agent::stop_escalation::StopReason::User,
    );
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
            tool_authorization_mode: None,
            session_id: Some("stop-session".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
            plan_execution_approval: None,
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
        tool_authorization_mode: None,
        session_id: Some("eq-sync-session".to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
        plan_execution_approval: None,
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
            tool_authorization_mode: None,
            session_id: Some("eq-stream-session".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
            plan_execution_approval: None,
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
        tool_authorization_mode: None,
        session_id: Some("eq-sync-failed-session".to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
        plan_execution_approval: None,
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
            tool_authorization_mode: None,
            session_id: Some("eq-stream-failed-session".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
            plan_execution_approval: None,
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
            tool_authorization_mode: None,
            session_id: Some("step-hop-stream".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
            plan_execution_approval: None,
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
    // wave-36 PA-139：归属校验——`同步多跳问题` 是本用例独有的用户消息（首个 decision
    // 请求的 volatile 输入、followup 的 history 均携带；其它用例不会出现该文案），杂散
    // 连接（其它用例迟到重试命中本端口）会被 503 拒绝而不消费队列。
    let sync_server = MockHttpServer::start_with_predicate(
        vec![
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
    ],
        std::sync::Arc::new(|body: &str| body.contains("同步多跳问题")),
    );
    let sync_runtime =
        build_runtime_for_test(test_provider_selection(sync_server.base_url.clone()));
    let sync_result = sync_runtime.run_turn(TurnInput {
        message: "同步多跳问题".to_string(),
        display_message: None,
        provider_id: None,
        model_id: None,
        reasoning_effort: None,
        workspace_mode: None,
        tool_authorization_mode: None,
        session_id: Some("step-hop-sync".to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
        plan_execution_approval: None,
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
        trace_id: None,
        cost_usd: None,
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
        stop_reason: None,
        metrics: None,
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
        stop_reason: None,
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
    // 三级树 stamp 纵深防御：先注册工作区，携带注册 id 的首轮才按原样盖章。
    let ws_root = { let raw = std::env::temp_dir().join(format!("pa-rt-stamp-{}", std::process::id())); raw.canonicalize().unwrap_or(raw) };
    std::fs::create_dir_all(&ws_root).unwrap();
    let mut sessions = SessionStore::memory_only();
    let registered = sessions
        .create_workspace("Proj", &ws_root.display().to_string())
        .unwrap();
    let selection = test_provider_selection(server.base_url.clone());
    let runtime = build_runtime_with_session_store(selection, sessions);

    let _ = runtime.run_turn(TurnInput {
        message: "hello".to_string(),
        display_message: None,
        provider_id: None,
        model_id: None,
        reasoning_effort: None,
        workspace_mode: None,
        tool_authorization_mode: None,
        session_id: Some("ws-session".to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: Some(registered.id.clone()),
        plan_execution_approval: None,
    });

    let snapshot = runtime.load_session_snapshot(Some("ws-session"));
    assert_eq!(
        snapshot.workspace_id.as_deref(),
        Some(registered.id.as_str()),
        "首轮 workspace_id 应盖章到会话"
    );
    let overviews = runtime.sessions_handle().read().unwrap().list_sessions();
    let overview = overviews
        .iter()
        .find(|session| session.conversation_id == "ws-session")
        .expect("session overview exists");
    assert_eq!(overview.workspace_id.as_deref(), Some(registered.id.as_str()));

    // 第二轮不同 id → 幂等，不覆盖
    let _ = runtime.run_turn(TurnInput {
        message: "hello 2".to_string(),
        display_message: None,
        provider_id: None,
        model_id: None,
        reasoning_effort: None,
        workspace_mode: None,
        tool_authorization_mode: None,
        session_id: Some("ws-session".to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: Some("ws-proj-2".to_string()),
        plan_execution_approval: None,
    });
    let snapshot2 = runtime.load_session_snapshot(Some("ws-session"));
    assert_eq!(
        snapshot2.workspace_id.as_deref(),
        Some(registered.id.as_str()),
        "已盖章会话的后续轮不得覆盖"
    );

    std::fs::remove_dir_all(&ws_root).ok();
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
        tool_authorization_mode: None,
        session_id: Some("attachment-failure".to_string()),
        node_id: None,
        history: Vec::new(),
        images: vec![TurnInputImage {
            data_url: "data:image/png;base64,AAAA".to_string(),
            mime_type: "image/png".to_string(),
            name: Some("diagram.png".to_string()),
        }],
        workspace_id: None,
        plan_execution_approval: None,
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
        tool_authorization_mode: None,
        session_id: Some("sync-reasoning-latency".to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
        plan_execution_approval: None,
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
        tool_authorization_mode: None,
        session_id: Some("sync-tool-hop-latency".to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
        plan_execution_approval: None,
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
        tool_authorization_mode: None,
        session_id: Some("sync-multi-hop".to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
        plan_execution_approval: None,
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
        tool_authorization_mode: None,
        session_id: Some("sync-usage-accumulated".to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
        plan_execution_approval: None,
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
        tool_authorization_mode: None,
        session_id: Some("repair-blank-tool-sync".to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
        plan_execution_approval: None,
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
    // wave-13：上限已升至 MAX_ALLOWED_TOOL_FOLLOWUPS_PER_TURN=2048（ada9cd2），
    // "64" 现为合法覆盖值；仅超上限与非法输入回退默认值。
    assert_eq!(parse_max_tool_followups_per_turn(Some("64")), 64);
    assert_eq!(
        parse_max_tool_followups_per_turn(Some("5000")),
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
        "{}".to_string(),
        "invalid_path".to_string(),
    ));
    assert_eq!(tracker.record(signal.clone()), 1);
    assert_eq!(tracker.record(signal.clone()), 2);
    assert_eq!(tracker.record(signal.clone()), 3);
    // 同一工具、相同入参、不同错误码：重新计数
    assert_eq!(
        tracker.record(Some((
            "workspace_read_file".to_string(),
            "{}".to_string(),
            "not_found".to_string()
        ))),
        1
    );
    // 同一工具、不同入参、相同错误码：重新计数（防止误伤不同命令）
    assert_eq!(
        tracker.record(Some((
            "workspace_read_file".to_string(),
            "{\"path\":\"different\"}".to_string(),
            "invalid_path".to_string()
        ))),
        1
    );
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
    // error + code → Some((tool, arg_fingerprint, code))
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
            "{}".to_string(),
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

    // 测试 ConsecutiveFailureTracker 指纹判定
    let mut tracker = ConsecutiveFailureTracker::new();
    let different_arg_call = ToolCall {
        call_id: None,
        name: "workspace_read_file".to_string(),
        arguments: json!({"path": "other.txt"}),
        plan: None,
    };
    // 相同入参连续 2 次，达到计数 2
    assert_eq!(tracker.record(tool_failure_signal(&call, &failing)), 1);
    assert_eq!(tracker.record(tool_failure_signal(&call, &failing)), 2);
    // 换了不同入参的相同错误，指纹变化，重置并从 1 重新计数（防误伤不同命令）
    assert_eq!(tracker.record(tool_failure_signal(&different_arg_call, &failing)), 1);
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

    // wave-13：连续失败指纹 = (tool, 参数归一化, error_code)（43706da「指纹化」），
    // 三次调用必须**同参**才会被计为「连续相同失败」并触发止损；此前用不同路径导致
    // 指纹逐次变化、计数恒为 1，止损永不触发（phase 停留在 ready）。
    //
    // 计数闸契约（契约矩阵 C1/C2，2026-11-07 修订）：同签名重复闸已从 `BTreeSet` 判重升级为
    // `SameSignatureTracker` 计数判重，默认**允许 2 次执行**、第 N+1 次（= 第 3 次）才熔断。
    // 因此 3 个同参决策 = 前 2 次真实执行（工具永远失败）+ 第 3 次命中闸转「恢复式收尾」，
    // 故 mock 队列需要 4 个响应：3 个决策响应 + 1 个恢复式收尾拿到的最终作答。
    let server = MockHttpServer::start(vec![
        json_response(decision_tool_call(
            "workspace_read_file",
            json!({ "path": "missing-a.txt", "description": "读取文件" }),
        )),
        json_response(decision_tool_call(
            "workspace_read_file",
            json!({ "path": "missing-a.txt", "description": "读取文件" }),
        )),
        json_response(decision_tool_call(
            "workspace_read_file",
            json!({ "path": "missing-a.txt", "description": "读取文件" }),
        )),
        json_completion("两次读取都失败，停止重试并给出结论。"),
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
        tool_authorization_mode: None,
        session_id: Some("consecutive-failure-stop-loss".to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
        plan_execution_approval: None,
    });

    assert_eq!(result.phase, "ready");
    // 契约修订（2026-11-07，计数闸）：旧口径「重复工具调用恢复在第二次即拦、共 3 次请求」已失真。
    // 现行口径 = `SameSignatureTracker::is_exhausted`（count >= limit，默认 limit=2）：
    // 同参第 1、2 次真实执行（本用例工具永远失败，故执行后仍 `phase == ready`，
    // 未被「连续失败止损」命中——同指纹失败仅 2 次 < 阈值 3），第 3 次同参调用命中闸后走
    // 恢复式收尾；请求数 = 3 个决策响应 + 1 个恢复响应 = 4，且不得出现第 5 次。
    let requests = server.finish();
    assert_eq!(
        requests.len(),
        4,
        "计数闸口径：前 2 次同参执行 + 第 3 次命中闸的恢复式收尾 = 4 次请求（5 次即 follow-up 失控）"
    );
}

/// 契约矩阵 C5-8（item 3 端到端，AMEND-2-4 重锚）：写类成功执行 ⇒ 清空其它签名计数。
///
/// 整轮 `Read(A) → Read(A) → Edit(B, 成功) → Read(A)` → 最终作答（三次读同参；`description`
/// 不同也被签名归一化忽略）⇒ `Edit` 成功后第 3 次 `Read(A)` **不得**命中同签名重复闸。
///
/// 为什么必须换成这个序列（L3-B 变异实证）：旧序列 `Read(A) → Edit(B) → Read(A)` 在默认
/// `limit=2` 下，`Read(A)` 在被清空前只执行过 **1** 次；即便实现里**根本没有清空逻辑**，
/// 第 2 次 `Read` 也只把计数推到 2（未超上限、不熔断）⇒ 用例照样绿 = 零区分力 = 假绿。
/// 新序列先用两次同参 `Read` 把计数顶到上限（=2），再由 `Edit` 成功清空：一旦 `record_executed`
/// 的 `retain`（写类成功清空其它签名）被删/退化，第 3 次 `Read` 就是第 3 次记账 ⇒
/// `is_exhausted(_, 2)` 命中 ⇒ **少一次真实执行**（`executed_names` / `tool_activities.len()` 立刻变红）
/// 且请求体出现合成 tool result（`blocked_redundant_followup` 断言红）。
/// 注意：**请求总数不变**（命中闸时 = 3 次执行 + 1 次被拦决策 + 1 次恢复式收尾 = 5，与未命中时
/// 4 次决策 + 1 次最终作答相同），故 `requests.len()` 只锁上限，区分「是否命中闸」靠执行序断言。
#[test]
fn run_turn_write_success_clears_same_signature_counts_end_to_end() {
    // Arrange：会记录真实执行序的「永远成功」executor（反向照抄 AlwaysFailingExecutor 的写法，
    // 避免真实工具执行器受沙箱/读前写观察门约束而无法稳定让 Edit 成功）。
    struct RecordingSuccessExecutor {
        executed: Arc<Mutex<Vec<String>>>,
    }
    impl crate::agent::tools::ToolExecutor for RecordingSuccessExecutor {
        fn execute(&self, call: &ToolCall) -> crate::agent::tools::ToolResult {
            self.executed
                .lock()
                .expect("executed log poisoned")
                .push(call.name.clone());
            crate::agent::tools::ToolResult {
                tool_name: call.name.clone(),
                status: "ok".to_string(),
                output: json!({ "ok": true, "tool": call.name }).to_string(),
                duration_ms: 1,
            }
        }
    }

    let final_text = "写后回读内容一致，任务完成。";
    let read_a = |description: &str| {
        json_response(decision_tool_call(
            "workspace_read_file",
            json!({ "path": "notes/a.txt", "description": description }),
        ))
    };
    let server = MockHttpServer::start(vec![
        // 同参读两次（description 不同不会改变签名）⇒ 计数顶到 limit=2。
        read_a("先读取基线"),
        read_a("再确认一次基线"),
        json_response(decision_tool_call(
            "workspace_edit_file",
            json!({
                "path": "notes/b.txt",
                "oldString": "旧内容",
                "newString": "新内容",
                "description": "落地修改"
            }),
        )),
        // 写类成功 ⇒ 清空其它签名计数 ⇒ 这次同参回读必须放行。
        read_a("写后回读校验"),
        json_completion(final_text),
    ]);
    let executed: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let runtime = build_runtime_for_test_with_tool_executor(
        test_provider_selection(server.base_url.clone()),
        Box::new(RecordingSuccessExecutor {
            executed: Arc::clone(&executed),
        }),
    );

    // Act
    let result = runtime.run_turn(TurnInput {
        message: "先读两次基线，改完再回读确认".to_string(),
        display_message: None,
        provider_id: None,
        model_id: None,
        reasoning_effort: None,
        workspace_mode: None,
        tool_authorization_mode: None,
        session_id: Some("write-success-clears-duplicate-gate".to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
        plan_execution_approval: None,
    });

    // Assert
    let requests = server.finish();
    let executed_names = executed.lock().expect("executed log poisoned").clone();
    assert_eq!(result.phase, "ready");
    assert_eq!(
        result.assistant_message, final_text,
        "正常 follow-up 收尾（而非恢复式收尾）必须拿到最后一次作答"
    );
    assert_eq!(
        executed_names,
        vec![
            "workspace_read_file".to_string(),
            "workspace_read_file".to_string(),
            "workspace_edit_file".to_string(),
            "workspace_read_file".to_string(),
        ],
        "写类成功必须清空其它签名计数：写后回读（第 3 次同参 Read）必须真实执行，\
         不得被同签名重复闸拦下"
    );
    assert_eq!(
        result.tool_activities.len(),
        4,
        "四次工具调用必须全部执行（命中闸会少一次执行 ⇒ 3）"
    );
    assert_eq!(
        requests.len(),
        5,
        "4 次决策请求 + 1 次最终作答 = 5（注：命中闸同样是 5，故本断言只锁上限；区分是否命中靠上面两条执行序断言）"
    );
    assert!(
        requests
            .iter()
            .all(|body| !body.contains("blocked_redundant_followup")),
        "写后回读不得触发重复闸的恢复式收尾（任何请求体都不得含 blocked_redundant_followup）"
    );
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
        tool_authorization_mode: None,
        session_id: Some("consecutive-failure-reset".to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
        plan_execution_approval: None,
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

// ─────────────────────────────────────────────────────────────────────────────
// 同签名重复工具调用闸（`BTreeSet` 判重 → 计数判重）：契约矩阵 C1/C2/C4/C5 的单元锁定。
//
// 这些用例是**红相冻结**用例：src 侧此刻是空桩（`unimplemented!()`），必须真实失败；
// Executor 填实现后必须全部转绿，且**不得**修改本文件（测试分权底线）。
// ─────────────────────────────────────────────────────────────────────────────

/// C4 文案契约：事实陈述（「完全相同」+ 精确比较口径）、不再出现「近似」、
/// 含实际次数与上限文本、且指明是哪个工具被拦。
#[test]
fn duplicate_tool_call_error_states_exact_match_with_actual_counts() {
    // Arrange：一个同参被拦下的读类调用。occurrences=3（本次是第 3 次）、limit=2（允许 2 次）。
    let call = ToolCall {
        call_id: Some("call_duplicate".to_string()),
        name: "workspace_read_file".to_string(),
        arguments: json!({ "path": "tauri.conf.json", "description": "读取文件" }),
        plan: None,
    };

    // Act
    let message = build_duplicate_tool_call_error(&call, 3, 2);

    // Assert：四条硬项 + 工具名 + 两条**语义位置**断言。出路段落的措辞不锁死（由 Executor 润色），
    // 只锁契约矩阵 C4 的①②③④四条可判定事实。
    assert!(
        message.contains("完全相同"),
        "文案必须声明比较口径是「完全相同」，实际={message}"
    );
    assert!(
        !message.contains("近似"),
        "文案不得再出现「近似」：判定是归一化参数的精确相等，实际={message}"
    );
    assert!(
        message.contains("workspace_read_file"),
        "文案必须指明是哪个工具被拦，实际={message}"
    );
    assert!(
        message.contains('3'),
        "文案必须给出本次是第几次调用（occurrences=3），实际={message}"
    );
    assert!(
        message.contains('2'),
        "文案必须给出允许次数上限（limit=2），实际={message}"
    );
    // AMEND-2-4（L3-B 变异实证）：只锁 `contains('3')`/`contains('2')` 时，把
    // occurrences 与 limit 两个入参**互换**的变异仍能通过（两个数字都在文案里）。
    // 因此必须锁住数字的**语义位置**：次数落在「已第 N 次」、上限落在「最多允许 L 次」。
    assert!(
        message.contains("已第 3 次"),
        "本次是第几次必须落在「已第 N 次」的语义位置（occurrences=3），实际={message}"
    );
    assert!(
        message.contains("最多允许 2 次"),
        "允许上限必须落在「最多允许 L 次」的语义位置（limit=2），实际={message}"
    );
}

/// C2 tracker 语义：默认 limit=2 ⇒ 前 2 次 `is_exhausted == false`，两次记账后为 true；
/// 闸点只读（连调多次不改变计数）。
#[test]
fn same_signature_tracker_allows_limit_executions_then_exhausts() {
    let signature = "workspace_read_file:{\"path\":\"tauri.conf.json\"}";
    let mut tracker = SameSignatureTracker::new();

    assert_eq!(
        tracker.occurrence_count(signature),
        0,
        "新建 tracker 的同签名计数必须为 0"
    );
    assert!(
        !tracker.is_exhausted(signature, 2),
        "零次执行不得判为耗尽"
    );

    // 第 1 次执行：放行，计数 1。
    tracker.record_executed(signature.to_string(), false);
    assert_eq!(
        tracker.occurrence_count(signature),
        1,
        "第 1 次记账后计数必须为 1"
    );
    assert!(
        !tracker.is_exhausted(signature, 2),
        "第 1 次执行后仍必须放行（第 2 次不允许被拦）"
    );

    // 第 2 次执行：放行，计数 2；此时第 3 次必须被拦。
    tracker.record_executed(signature.to_string(), false);
    assert_eq!(
        tracker.occurrence_count(signature),
        2,
        "第 2 次记账后计数必须为 2"
    );
    assert!(
        tracker.is_exhausted(signature, 2),
        "第 2 次执行后，第 3 次同参调用必须熔断"
    );

    // 只读性：闸点连调 3 次不得记账（否则恢复式收尾后的重试会把自己推高计数）。
    for probe in 0..3 {
        assert!(
            tracker.is_exhausted(signature, 2),
            "第 {probe} 次重复探测必须保持耗尽态稳定"
        );
    }
    assert_eq!(
        tracker.occurrence_count(signature),
        2,
        "is_exhausted 必须只读，连续探测不得改变计数"
    );
}

/// C2 tracker 语义：不同签名互不影响（各自独立计数、各自独立熔断）。
///
/// AMEND-2-3：`SameSignatureTracker::clear()` 已按「生产零消费者」移除（宪法常载命约 3），
/// 故本用例不再断言 `clear()`，仅保留签名隔离不变量。
#[test]
fn same_signature_tracker_isolates_signatures() {
    let read_signature = "workspace_read_file:{\"path\":\"a.txt\"}";
    let search_signature = "workspace_search_text:{\"query\":\"needle\"}";
    let mut tracker = SameSignatureTracker::new();

    tracker.record_executed(read_signature.to_string(), false);
    tracker.record_executed(read_signature.to_string(), false);
    assert!(
        tracker.is_exhausted(read_signature, 2),
        "同一签名两次记账后必须耗尽"
    );
    assert_eq!(
        tracker.occurrence_count(search_signature),
        0,
        "其它签名不得被连带计数"
    );
    assert!(
        !tracker.is_exhausted(search_signature, 2),
        "其它签名不得被连带熔断"
    );

    // 另一签名各自独立累积：只增自己的计数，不得回头影响已耗尽的签名。
    tracker.record_executed(search_signature.to_string(), false);
    assert_eq!(
        tracker.occurrence_count(search_signature),
        1,
        "其它签名必须独立累计到 1（互不影响）"
    );
    assert_eq!(
        tracker.occurrence_count(read_signature),
        2,
        "另一签名记账不得改变既有签名的计数"
    );
}

/// AMEND-1 写类清空语义：写类**成功**执行后清空**其它**签名计数，但保留自身计数。
#[test]
fn same_signature_tracker_write_success_clears_other_signatures_only() {
    let read_signature = "workspace_read_file:{\"path\":\"notes.md\"}";
    let write_signature = "workspace_edit_file:{\"path\":\"notes.md\"}";
    let mut tracker = SameSignatureTracker::new();

    // Arrange：先积累一次读类计数（模拟 Read(A) 已执行过一次）。
    tracker.record_executed(read_signature.to_string(), false);
    assert_eq!(tracker.occurrence_count(read_signature), 1);

    // Act：写类成功执行一次（模拟 Edit(B, 成功)）。
    tracker.record_executed(write_signature.to_string(), true);

    // Assert：读类计数被清空（写后回读不是冗余），写类自身计数保留为 1。
    assert_eq!(
        tracker.occurrence_count(read_signature),
        0,
        "写类成功必须清空其它签名计数（否则写后回读会被误判为重复）"
    );
    assert_eq!(
        tracker.occurrence_count(write_signature),
        1,
        "写类成功必须把自身计入 1 次"
    );

    // 再次写类成功：自身计数递增（不得把自己清掉，否则「反复写同一处」无限放行）。
    tracker.record_executed(write_signature.to_string(), true);
    assert_eq!(
        tracker.occurrence_count(write_signature),
        2,
        "写类成功不得清空自身计数，必须递增"
    );
    assert_eq!(
        tracker.occurrence_count(read_signature),
        0,
        "写类成功后已清空的其它签名不得被凭空写回"
    );
}

fn tool_result_stub(tool_name: &str, status: &str) -> crate::agent::tools::ToolResult {
    crate::agent::tools::ToolResult {
        tool_name: tool_name.to_string(),
        status: status.to_string(),
        output: json!({ "ok": status == "ok" }).to_string(),
        duration_ms: 1,
    }
}

/// C2「改变世界」判定之一（AMEND-2-2：写类 + **负向状态列表** `status ∉ {"error","aborted","cancelled"}`）。
///
/// 对照组见 `tool_execution_changed_world_excludes_execute_kind`（AMEND-2-1：Execute 不计入）。
/// 负向列表的由来（L3-A）：原判据 `status != "error"` 会把 `aborted` **写类**误判为「已改变世界」
/// 并清空他人计数，等于把「被中止的写」当成任务推进。
#[test]
fn tool_execution_changed_world_is_write_only() {
    let call = |name: &str| ToolCall {
        call_id: Some(format!("call_{name}")),
        name: name.to_string(),
        arguments: json!({ "path": "notes.md" }),
        plan: None,
    };

    // ① 写类 + 成功 ⇒ 改变世界（唯一 true 分支）。
    assert!(
        tool_execution_changed_world(
            &call("workspace_edit_file"),
            &tool_result_stub("workspace_edit_file", "ok")
        ),
        "写类 + 成功必须判为改变世界"
    );
    // ② 写类 + 失败 ⇒ 不得清空（失败重试正是本闸要放行的场景）。
    assert!(
        !tool_execution_changed_world(
            &call("workspace_edit_file"),
            &tool_result_stub("workspace_edit_file", "error")
        ),
        "写类 + 失败不得判为改变世界（失败重试正是本闸要放行的场景）"
    );
    // ③ AMEND-2-2：负向成功列表 —— `aborted` 的写类不算改变世界（原 `!= "error"` 会误判为成功
    //    并清空他人计数，等于把「被中止的写」当成任务推进）。
    assert!(
        !tool_execution_changed_world(
            &call("workspace_edit_file"),
            &tool_result_stub("workspace_edit_file", "aborted")
        ),
        "写类 + aborted 不得判为改变世界（负向状态列表：error/aborted/cancelled 一律不算成功）"
    );
    assert!(
        !tool_execution_changed_world(
            &call("workspace_edit_file"),
            &tool_result_stub("workspace_edit_file", "cancelled")
        ),
        "写类 + cancelled 不得判为改变世界（负向状态列表）"
    );
    // ④ 只读 + 成功 ⇒ 不得清空（否则纯检索打转会被自己清空计数）。
    assert!(
        !tool_execution_changed_world(
            &call("workspace_read_file"),
            &tool_result_stub("workspace_read_file", "ok")
        ),
        "只读 + 成功不得判为改变世界（否则纯检索打转会被自己清空计数）"
    );
}

/// AMEND-2-1（Gate-1 BLOCKING）：`ToolKind::Execute` **不得**计入「改变世界」。
///
/// `Execute` 覆盖 `workspace_run_command` 与全部 `Job*`（`JobOutput`/`JobList`/`JobKill`，
/// `tools.rs:7980`），静态无法区分「写域 Run」「纯查询（ls）」「job 轮询」；若纳入清空，
/// 则 `JobOutput(A)` 与 `Run(X)` 成功**交替**会互相清空、两签名计数恒为 1、永不达上限 ⇒
/// 本方案反而新引入绕闸面（旧 `BTreeSet` 第 2 次即拦）。且 `retain` 恒保留自身，Execute 清空
/// 对「同参重试」零收益；「改完重跑同一条命令」的真实受益路径是中间的 `Edit`（Write）触发清空。
///
/// 本断言独立成用例（而非并入 `..._is_write_only`）：两条 AMEND-2 修复点各自给出独立的红相信号，
/// 避免「负向状态列表未改」的 panic 掩盖「Execute 未收窄」的缺陷。
#[test]
fn tool_execution_changed_world_excludes_execute_kind() {
    let call = |name: &str| ToolCall {
        call_id: Some(format!("call_{name}")),
        name: name.to_string(),
        arguments: json!({ "path": "notes.md" }),
        plan: None,
    };

    assert!(
        !tool_execution_changed_world(
            &call("workspace_run_command"),
            &tool_result_stub("workspace_run_command", "ok")
        ),
        "执行类 + 成功不得判为改变世界：Execute 静态无法区分写域 Run / 纯查询 / job 轮询，\
         纳入清空会与 Job* 形成互清绕闸面，故保守只认 Write"
    );
}

/// L2-AT 畸形/边界注入：同签名允许次数解析必须对非法取值**回退默认**，
/// 尤其**不得静默取 0**（0 ⇒ 每轮第一次调用后立刻熔断，等于把闸焊死）。
#[test]
fn same_signature_occurrence_limit_parse_falls_back_to_default_on_malformed_input() {
    // 默认值锁定：契约矩阵 C1 声明「允许执行 2 次、第 3 次拦」。
    assert_eq!(
        DEFAULT_MAX_SAME_SIGNATURE_OCCURRENCES_PER_TURN, 2,
        "默认允许执行次数必须是 2（前 2 次放行、第 3 次触发闸）"
    );

    // 畸形/边界注入：空、零、越界（>8）、非数字、纯空白、负数、带尾随空白。
    for raw in [
        None,
        Some(""),
        Some("0"),
        Some("9"),
        Some("abc"),
        Some("  "),
        Some("-1"),
        Some("2 "),
    ] {
        let parsed = parse_max_same_signature_occurrences_per_turn(raw);
        assert_eq!(
            parsed, DEFAULT_MAX_SAME_SIGNATURE_OCCURRENCES_PER_TURN,
            "非法/越界取值 {raw:?} 必须回退默认值"
        );
        assert_ne!(
            parsed, 0,
            "非法取值 {raw:?} 严禁静默取 0（0 = 每轮第一次调用后立即熔断）"
        );
    }

    // 合法取值必须逐字生效（下界与上界各取一个）。
    assert_eq!(
        parse_max_same_signature_occurrences_per_turn(Some("1")),
        1,
        "合法取值 1 必须生效"
    );
    assert_eq!(
        parse_max_same_signature_occurrences_per_turn(Some("8")),
        8,
        "合法取值 8（MAX_ALLOWED 上界）必须生效"
    );
    assert_eq!(
        parse_max_same_signature_occurrences_per_turn(Some(" 3 ")),
        3,
        "合法取值必须允许首尾空白"
    );
}

struct SameSignatureOccurrenceOverrideGuard {
    previous: usize,
}

impl SameSignatureOccurrenceOverrideGuard {
    fn set(limit: usize) -> Self {
        Self {
            previous: set_same_signature_occurrences_override(limit),
        }
    }
}

impl Drop for SameSignatureOccurrenceOverrideGuard {
    fn drop(&mut self) {
        set_same_signature_occurrences_override(self.previous);
    }
}

/// C1 getter + `#[cfg(test)]` 覆写槽：未覆写时回落默认，覆写生效、收尾后还原。
#[test]
fn same_signature_occurrence_limit_getter_honours_test_override() {
    assert_eq!(
        max_same_signature_occurrences_per_turn(),
        DEFAULT_MAX_SAME_SIGNATURE_OCCURRENCES_PER_TURN,
        "无覆写时必须回落默认值（env 未设置）"
    );

    {
        let _guard = SameSignatureOccurrenceOverrideGuard::set(5);
        assert_eq!(
            max_same_signature_occurrences_per_turn(),
            5,
            "用例覆写必须立刻生效"
        );
    }

    assert_eq!(
        max_same_signature_occurrences_per_turn(),
        DEFAULT_MAX_SAME_SIGNATURE_OCCURRENCES_PER_TURN,
        "覆写还原后必须重新回落默认值"
    );
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
            tool_authorization_mode: None,
            session_id: Some("stream-multi-hop".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
            plan_execution_approval: None,
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
    // wave-35/C1:`ToolHopLimitOverrideGuard` 写的是**进程级** registry(limits.rs),
    // 必须在持有 `MOCK_SERVER_GATE` 期间才生效:旧代码在 `MockHttpServer::start`(抢 gate)
    // **之前** `set(1)`,于是正在持 gate 跑 turn 的并发 gated 用例读到 hop=1 → turn 提前
    // 中止 → 其请求数 < mock 队列 → mock 线程永不等齐 → `finish()` 无限 join → gate 永不
    // 释放 → 整批 mock 用例 futex 等待(整库挂死)。
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
    // 此处已持有 gate(start 内部 acquire);override 只在此区间内可被观测。
    let _limit_guard = ToolHopLimitOverrideGuard::set(1);
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
            tool_authorization_mode: None,
            session_id: Some("stream-tool-hop-limit".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
            plan_execution_approval: None,
        },
    );

    // `finish()` 按值消费 server ⇒ 返回时 gate 已释放。必须先显式撤销进程级 override,
    // 否则「gate 已放出、override 仍在」的窗口里,下一个 gated 用例会读到 hop=1 并死锁。
    drop(_limit_guard);
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
            tool_authorization_mode: None,
            session_id: Some("stream-cancel-during-tool".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
            plan_execution_approval: None,
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
    // wave-tui-session-resilience C5：工具执行中被中断的轮次不再落占位文案
    // `CANCELLED_TURN_MESSAGE`，而是持久化「中断轮次检查点」投影
    // （纯函数 `session::interrupted_turn_checkpoint_text`，见 `session/store.rs:62-77`；
    // `bin/tui.rs` 依赖该 `[中断轮次检查点]` 标记做逐字回放）。因此这里断言投影结构：
    // 以标记开头 + 含已执行工具步骤段 + 含本用例唯一那条工具活动。
    let cancelled_history_text = snapshot
        .history
        .last()
        .map(|message| message.content.as_str())
        .expect("cancelled turn must persist a final history message");
    assert!(
        cancelled_history_text.starts_with("[中断轮次检查点]"),
        "中断轮次必须以检查点标记开头，实际：{cancelled_history_text:?}"
    );
    assert!(
        cancelled_history_text.contains("已执行的工具步骤："),
        "中断轮次检查点必须列出已执行工具步骤，实际：{cancelled_history_text:?}"
    );
    assert!(
        cancelled_history_text.contains("List Tool call finished with status: ok."),
        "检查点必须记录被中断前已完成的工具活动，实际：{cancelled_history_text:?}"
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
            tool_authorization_mode: None,
            session_id: Some("stream-tool-error".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
            plan_execution_approval: None,
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
            tool_authorization_mode: None,
            session_id: Some("stream-reasoning-latency".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
            plan_execution_approval: None,
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
            tool_authorization_mode: None,
            session_id: Some("stream-sync-fallback-no-fake-ttft".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
            plan_execution_approval: None,
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
            tool_authorization_mode: None,
            session_id: Some("stream-tool-hop-latency".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
            plan_execution_approval: None,
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
            tool_authorization_mode: None,
            session_id: Some("deepseek-followup-compat".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
            plan_execution_approval: None,
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
    // 计数闸契约（契约矩阵 C1/C5-5）：默认允许 2 次执行 ⇒ 第 3 个同参 tool_call 才触发闸，
    // 故队列 = 3 个同参决策响应 + 1 个恢复式收尾的总结响应 = 4 个响应、4 次请求。
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
        sse_response(&[json!({
            "choices": [
                {
                    "delta": {
                        "reasoning_content": "第三次同参读取同一文件。",
                        "tool_calls": [
                            {
                                "index": 0,
                                "id": "call_workspace_read_file_third",
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
            tool_authorization_mode: None,
            session_id: Some("stream-duplicate-tool-recovery".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
            plan_execution_approval: None,
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

    // 计数闸口径（契约矩阵 C5-5）：3 个同参决策响应全部被消费（前 2 次放行执行、
    // 第 3 次命中闸），第 4 次请求是恢复式收尾的总结请求。
    assert_eq!(
        requests.len(),
        4,
        "同签名计数闸：3 个同参 tool_call + 1 次恢复式收尾 = 4 次请求"
    );
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

/// 回归（trace `38590d52…` 实证）：重复工具调用的「冗余恢复」请求必须保留**完整**
/// transcript（system 前缀 + 历史对话 + 本轮 hops），不得塌缩成 `[当前 user] + hops`。
///
/// 旧实现把 `native_messages` 整体替换为 transcript 前缀，而 native 流程下 provider 直接
/// 把该字段当 messages ⇒ 末轮回答丢失 system prompt 与全部历史（实测 input 18896 → 2311、
/// cache_read=0）。本用例锁定「恢复请求前缀 == 正常 follow-up 前缀」这一不变量。
///
/// 计数闸适配（契约矩阵 C1/C2）：默认允许 2 次执行 ⇒ 需要**第 3 个**同参 tool_call 才命中闸，
/// 故队列为 3 个同参决策响应 + 1 个恢复作答；前缀比对基准随之为第 3 次请求（= 产出被拦调用的
/// 那次 follow-up 请求，其 messages 与本轮累计上下文逐字一致）。
#[test]
fn duplicate_tool_recovery_request_keeps_full_context() {
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
        sse_response(&[json!({
            "choices": [
                {
                    "delta": {
                        "reasoning_content": "第三次同参读取同一文件。",
                        "tool_calls": [
                            {
                                "index": 0,
                                "id": "call_workspace_read_file_third",
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
                        "content": "根据已读取的内容，tauri.conf.json 中 productName 是 Pony Agent。"
                    }
                }
            ]
        })]),
    ]);
    let mut runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));
    let sink = RecordingTurnEventSink::new();

    runtime.start_turn_stream(
        &sink,
        "turn-duplicate-recovery-context".to_string(),
        TurnInput {
            message: "读取 tauri.conf.json 并回答 productName".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            workspace_mode: None,
            tool_authorization_mode: None,
            session_id: Some("stream-duplicate-recovery-context".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
            plan_execution_approval: None,
        },
    );

    let requests = server.finish();
    assert_eq!(
        requests.len(),
        4,
        "计数闸口径：decision + 2 次放行执行的 follow-up + recovery 请求"
    );

    let messages_of = |raw: &str, label: &str| -> Vec<Value> {
        let body: Value = serde_json::from_str(raw)
            .unwrap_or_else(|e| panic!("{label} request should be json: {e}"));
        body.get("messages")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_else(|| panic!("{label} request should carry messages"))
    };
    let decision_messages = messages_of(&requests[0], "decision");
    let followup_messages = messages_of(&requests[2], "followup");
    let recovery_messages = messages_of(&requests[3], "recovery");

    assert_eq!(
        decision_messages
            .first()
            .and_then(|message| message.get("role"))
            .and_then(Value::as_str),
        Some("system"),
        "决策请求首条消息必须是 system 前缀（否则本用例失去判定意义）"
    );
    assert_eq!(
        recovery_messages.len(),
        followup_messages.len() + 2,
        "恢复请求 = 完整 transcript + 被阻断 assistant + 合成 tool result；followup={} recovery={}",
        followup_messages.len(),
        recovery_messages.len()
    );
    assert_eq!(
        &recovery_messages[..followup_messages.len()],
        &followup_messages[..],
        "恢复请求前缀必须与正常 follow-up 完全一致（system/历史/本轮用户输入不得丢失）"
    );

    let blocked = &recovery_messages[followup_messages.len()];
    assert_eq!(
        blocked.get("role").and_then(Value::as_str),
        Some("assistant"),
        "恢复请求倒数第二条必须是被阻断的 assistant 消息"
    );
    assert!(
        blocked
            .get("tool_calls")
            .and_then(Value::as_array)
            .map(|calls| !calls.is_empty())
            .unwrap_or(false),
        "被阻断的 assistant 消息必须保留 tool_calls（否则 tool result 无对应调用）"
    );
    let synthetic = &recovery_messages[followup_messages.len() + 1];
    assert_eq!(
        synthetic.get("role").and_then(Value::as_str),
        Some("tool")
    );
    assert!(
        synthetic
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .contains("blocked_redundant_followup"),
        "合成 tool result 必须携带 blocked_redundant_followup 语义"
    );
    assert!(
        recovery_messages.iter().any(|message| {
            message.get("role").and_then(Value::as_str) == Some("user")
                && message
                    .get("content")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .contains("读取 tauri.conf.json 并回答 productName")
        }),
        "恢复请求必须仍携带本轮用户输入"
    );
}

/// P1-1：`llm.chat` 请求形状三键（mode / messages / system_prefix）的纯函数出口。
///
/// 语义锁定：native transcript 非空时以它为准（native 流程下 provider 直接拿它当
/// messages），否则回退分层 `input`；`system_prefix` 必须是对**同一序列**首条 role 的真实判定。
#[test]
fn llm_request_shape_reports_native_transcript_and_system_prefix() {
    let build = |native_messages: Vec<Value>, input: Vec<ProviderMessage>| ProviderRequest {
        model: "gpt-5.4".to_string(),
        input,
        images: Vec::new(),
        native_messages,
        observation: Default::default(),
        temperature: 0.0,
        max_output_tokens: 1024,
    };

    let native_full = build(
        vec![
            json!({"role": "system", "content": "sys"}),
            json!({"role": "user", "content": "hi"}),
        ],
        vec![ProviderMessage::user("ignored")],
    );
    assert_eq!(
        crate::agent::turn_flow::llm_request_shape(&native_full),
        (2, true),
        "native transcript 非空时必须优先于分层 input"
    );

    // 恢复请求若塌陷成「只有 user」，该指纹必须是 (n, false) —— 这是 P0 缺陷的检索键。
    let collapsed = build(vec![json!({"role": "user", "content": "hi"})], Vec::new());
    assert_eq!(
        crate::agent::turn_flow::llm_request_shape(&collapsed),
        (1, false),
        "无 system 前缀必须如实报 false"
    );

    let layered = build(
        Vec::new(),
        vec![ProviderMessage::system("sys"), ProviderMessage::user("hi")],
    );
    assert_eq!(
        crate::agent::turn_flow::llm_request_shape(&layered),
        (2, true),
        "native transcript 为空时回退分层 input 并如实判定 system 前缀"
    );
}

/// P1-1：模式标签与属性映射是同一条出口（避免「枚举改了、属性没改」的静默漂移）。
#[test]
fn llm_request_mode_labels_and_attributes_stay_in_sync() {
    assert_eq!(
        crate::agent::turn_flow::LlmRequestMode::Initial.as_str(),
        "initial"
    );
    assert_eq!(
        crate::agent::turn_flow::LlmRequestMode::ToolFollowup.as_str(),
        "tool_followup"
    );
    assert_eq!(
        crate::agent::turn_flow::LlmRequestMode::ToolRecovery.as_str(),
        "tool_recovery"
    );

    let attributes = crate::agent::otel::llm_request_shape_attributes(
        crate::agent::turn_flow::LlmRequestMode::ToolRecovery.as_str(),
        4,
        false,
    );
    assert_eq!(attributes.len(), 3, "{attributes:?}");
    assert_eq!(attributes[0].0, "llm.request.mode");
    assert_eq!(
        attributes[0].1,
        crate::agent::otel::AttrValue::Str("tool_recovery".to_string())
    );
    assert_eq!(attributes[1].0, "llm.request.messages");
    assert_eq!(attributes[1].1, crate::agent::otel::AttrValue::I64(4));
    assert_eq!(attributes[2].0, "llm.request.system_prefix");
    assert_eq!(attributes[2].1, crate::agent::otel::AttrValue::Bool(false));
}

/// P1-2a：job 系列工具的 snake_case 别名在 schema 预检前被改写成 schema 口径。
///
/// 真实 trace 里 `job_output` 两种拼法各被拒一次（`unexpected argument job_id` /
/// `missing required argument jobId`），9 次调用全部失败。本用例锁定「预检前归一」。
#[test]
fn job_tool_argument_aliases_are_canonicalized_before_schema_check() {
    let mut call = ToolCall {
        call_id: None,
        name: "job_output".to_string(),
        arguments: json!({"job_id": "job-1", "timeout_ms": 5000, "wait": true}),
        plan: None,
    };
    canonicalize_tool_argument_aliases(&mut call);
    assert_eq!(
        call.arguments,
        json!({"jobId": "job-1", "timeoutMs": 5000, "wait": true}),
        "snake_case 别名必须被改写成 schema 口径"
    );

    // 幂等：已按 schema 写好的参数不得被二次改写。
    canonicalize_tool_argument_aliases(&mut call);
    assert_eq!(
        call.arguments,
        json!({"jobId": "job-1", "timeoutMs": 5000, "wait": true})
    );

    // 双拼法并存时保持原样（让 schema 的 additionalProperties 门给出确定失败，不做静默取舍）。
    let mut ambiguous = ToolCall {
        call_id: None,
        name: "job_kill".to_string(),
        arguments: json!({"jobId": "a", "job_id": "b"}),
        plan: None,
    };
    canonicalize_tool_argument_aliases(&mut ambiguous);
    assert_eq!(ambiguous.arguments, json!({"jobId": "a", "job_id": "b"}));

    // 非 job 工具零副作用。
    let mut run = ToolCall {
        call_id: None,
        name: "Run".to_string(),
        arguments: json!({"command": "ls", "timeout_ms": 1}),
        plan: None,
    };
    canonicalize_tool_argument_aliases(&mut run);
    assert_eq!(run.arguments, json!({"command": "ls", "timeout_ms": 1}));
}

/// P1-2a：serde 侧同样吃两种拼法（host / direct 调用不经过归一也要能解析）。
#[test]
fn job_args_accept_both_snake_and_camel_spellings() {
    let camel: crate::agent::jobs::JobOutputArgs =
        serde_json::from_value(json!({"jobId": "job-1", "timeoutMs": 10}))
            .expect("camelCase must parse");
    let snake: crate::agent::jobs::JobOutputArgs =
        serde_json::from_value(json!({"job_id": "job-1", "timeout_ms": 10}))
            .expect("snake_case must parse");
    assert_eq!(camel.job_id, "job-1");
    assert_eq!(snake.job_id, camel.job_id);
    assert_eq!(snake.timeout_ms, camel.timeout_ms);

    let kill: crate::agent::jobs::JobKillArgs = serde_json::from_value(json!({"jobId": "job-2"}))
        .expect("job_kill camelCase must parse");
    assert_eq!(kill.job_id, "job-2");

    let start: crate::agent::jobs::JobStartArgs =
        serde_json::from_value(json!({"command": "npm run dev", "timeoutMs": 1000}))
            .expect("job_start camelCase must parse");
    assert_eq!(start.timeout_ms, Some(1000));
}

/// P1-3：两个新闸门的环境变量解析（非法值一律回退默认，禁止静默取 0 造成「永不触发」）。
#[test]
fn turn_budget_and_same_tool_failure_limits_parse_env() {
    assert_eq!(parse_max_turn_wall_seconds(Some("600")), 600);
    assert_eq!(
        parse_max_turn_wall_seconds(Some("0")),
        DEFAULT_MAX_TURN_WALL_SECONDS
    );
    assert_eq!(
        parse_max_turn_wall_seconds(Some("999999")),
        DEFAULT_MAX_TURN_WALL_SECONDS
    );
    assert_eq!(
        parse_max_turn_wall_seconds(Some("abc")),
        DEFAULT_MAX_TURN_WALL_SECONDS
    );
    assert_eq!(parse_max_consecutive_same_tool_failures(Some("5")), 5);
    assert_eq!(
        parse_max_consecutive_same_tool_failures(Some("-1")),
        DEFAULT_MAX_CONSECUTIVE_SAME_TOOL_FAILURES
    );
    assert_eq!(
        parse_max_consecutive_same_tool_failures(None),
        DEFAULT_MAX_CONSECUTIVE_SAME_TOOL_FAILURES
    );
}

struct TurnWallOverrideGuard {
    previous: usize,
}

impl TurnWallOverrideGuard {
    fn set(seconds: usize) -> Self {
        Self {
            previous: set_turn_wall_override(seconds),
        }
    }
}

impl Drop for TurnWallOverrideGuard {
    fn drop(&mut self) {
        set_turn_wall_override(self.previous);
    }
}

struct SameToolFailureOverrideGuard {
    previous: usize,
}

impl SameToolFailureOverrideGuard {
    fn set(limit: usize) -> Self {
        Self {
            previous: set_same_tool_failure_override(limit),
        }
    }
}

impl Drop for SameToolFailureOverrideGuard {
    fn drop(&mut self) {
        set_same_tool_failure_override(self.previous);
    }
}

/// P1-3：同一工具连续失败（入参/错误码都不同）必须止损——既有指纹闸拦不住这种打转。
///
/// 真实 trace `dab6a42e…`：`JobOutput` 连续失败 5 次（参数各异）而回合继续跑，最终 942s / 71 次调用。
#[test]
fn same_tool_consecutive_failures_stop_turn_even_with_different_arguments() {
    let _limit = SameToolFailureOverrideGuard::set(4);
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

    // 5 个「同一工具、不同入参」的决策：三元组指纹每次都不同，只有工具级计数能拦住。
    let server = MockHttpServer::start(vec![
        json_response(decision_tool_call(
            "workspace_read_file",
            json!({ "path": "a.txt", "description": "尝试一" }),
        )),
        json_response(decision_tool_call(
            "workspace_read_file",
            json!({ "path": "b.txt", "description": "尝试二" }),
        )),
        json_response(decision_tool_call(
            "workspace_read_file",
            json!({ "path": "c.txt", "description": "尝试三" }),
        )),
        json_response(decision_tool_call(
            "workspace_read_file",
            json!({ "path": "d.txt", "description": "尝试四" }),
        )),
        json_response(decision_tool_call(
            "workspace_read_file",
            json!({ "path": "e.txt", "description": "尝试五" }),
        )),
    ]);
    let runtime = build_runtime_for_test_with_tool_executor(
        test_provider_selection(server.base_url.clone()),
        Box::new(AlwaysFailingExecutor),
    );

    let result = runtime.run_turn(TurnInput {
        message: "读取不存在的文件".to_string(),
        display_message: None,
        provider_id: None,
        model_id: None,
        reasoning_effort: None,
        workspace_mode: None,
        tool_authorization_mode: None,
        session_id: Some("same-tool-consecutive-failure".to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
        plan_execution_approval: None,
    });

    let requests = server.finish();
    assert_eq!(
        requests.len(),
        max_consecutive_same_tool_failures_per_turn(),
        "第 N 次同工具失败后必须立刻止损，不得继续 follow-up"
    );
    assert!(
        result.phase != "ready",
        "止损必须以失败相位收口，实际 phase={}",
        result.phase
    );
    assert!(
        result.assistant_message.contains("workspace_read_file")
            || result.assistant_message.contains("连续失败"),
        "错误文案必须指明是同一工具连续失败：{}",
        result.assistant_message
    );
}

/// P1-3：墙钟预算耗尽后不再 follow-up，而是走恢复式收尾（保留完整上下文）。
#[test]
fn turn_wall_budget_wraps_up_with_recovery_answer() {
    let _budget = TurnWallOverrideGuard::set(1);
    let final_text = "预算用尽，基于已获得的证据收尾。";
    let mut slow_first = sse_response(&[json!({
        "choices": [
            {
                "delta": {
                    "reasoning_content": "先读配置。",
                    "tool_calls": [
                        {
                            "index": 0,
                            "id": "call_slow",
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
    })]);
    slow_first.delay_ms = 1200;

    let server = MockHttpServer::start(vec![
        slow_first,
        sse_response(&[json!({
            "choices": [
                {
                    "delta": {
                        "tool_calls": [
                            {
                                "index": 0,
                                "id": "call_after_budget",
                                "type": "function",
                                "function": {
                                    "name": "workspace_read_file",
                                    "arguments": "{\"path\":\"package.json\"}"
                                }
                            }
                        ]
                    }
                }
            ]
        })]),
        sse_response(&[json!({
            "choices": [{ "delta": { "content": final_text } }]
        })]),
    ]);
    let mut runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));
    let sink = RecordingTurnEventSink::new();

    runtime.start_turn_stream(
        &sink,
        "turn-wall-budget".to_string(),
        TurnInput {
            message: "读取两个配置文件".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            workspace_mode: None,
            tool_authorization_mode: None,
            session_id: Some("stream-turn-wall-budget".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
            plan_execution_approval: None,
        },
    );

    let requests = server.finish();
    assert_eq!(
        requests.len(),
        3,
        "决策 + 一次 follow-up + 预算恢复请求（不得继续第二次 follow-up）"
    );
    let recovery: Value = serde_json::from_str(&requests[2]).expect("recovery request json");
    let recovery_messages = recovery
        .get("messages")
        .and_then(Value::as_array)
        .expect("recovery messages");
    assert!(
        recovery_messages.iter().any(|message| {
            message
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .contains("墙钟预算")
        }),
        "收尾请求必须携带墙钟预算原因（否则模型不知道为何停止）"
    );
    let events = sink.events.borrow();
    assert!(
        events.iter().any(|(name, payload)| name == "turn:delta"
            && payload.text.as_deref() == Some(final_text)),
        "预算收尾答案必须送达 UI"
    );
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
            tool_authorization_mode: None,
            session_id: Some("stream-usage-accumulated".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
            plan_execution_approval: None,
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
            tool_authorization_mode: None,
            session_id: Some("repair-blank-tool-stream".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
            plan_execution_approval: None,
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
        tool_authorization_mode: None,
        session_id: Some("runtime-history".to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
        plan_execution_approval: None,
    });
    let second = runtime.run_turn(TurnInput {
        message: "第二问".to_string(),
        display_message: None,
        provider_id: None,
        model_id: None,
        reasoning_effort: None,
        workspace_mode: None,
        tool_authorization_mode: None,
        session_id: Some("runtime-history".to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
        plan_execution_approval: None,
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
        context_window_tokens: None,
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
        context_window_tokens: None,
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

// ── 空洞 tool call 兜底：上游回 name="" arguments={} 时丢弃转纯文本，不整轮 failed ──

#[test]
fn empty_tool_call_with_empty_arguments_is_dropped_to_text() {
    use crate::agent::runtime::stream_support::normalize_tool_directive;
    use crate::agent::tools::ToolCall;

    let normalized = normalize_tool_directive(
        ToolCall {
            call_id: None,
            name: "".to_string(),
            arguments: json!({}),
            plan: None,
        },
        None,
        "",
        None,
        None,
    )
    .expect("空洞 tool call 应被丢弃而非报错");
    assert!(normalized.tool_call.is_none());
    assert!(!normalized.output_text.trim().is_empty());
}

#[test]
fn empty_tool_call_with_nonempty_arguments_still_repairable_or_error() {
    use crate::agent::runtime::stream_support::normalize_tool_directive;
    use crate::agent::tools::ToolCall;

    // 可修复：带 path 参数应修名为 workspace_read_file。
    let repaired = normalize_tool_directive(
        ToolCall {
            call_id: None,
            name: "".to_string(),
            arguments: json!({"path": "src/lib.rs"}),
            plan: None,
        },
        None,
        "",
        None,
        None,
    )
    .expect("带参数的空名调用应可修复");
    assert_eq!(
        repaired.tool_call.expect("应修复出工具名").name,
        "workspace_read_file"
    );

    // 不可修复的非空参数仍报错（保持原有严格行为）。
    assert!(normalize_tool_directive(
        ToolCall {
            call_id: None,
            name: "".to_string(),
            arguments: json!({"weird": 1}),
            plan: None,
        },
        None,
        "",
        None,
        None,
    )
    .is_err());
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
        context_window_tokens: None,
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
        context_window_tokens: None,
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
            tool_authorization_mode: None,
            session_id: Some("deepseek-followup-compat".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
            plan_execution_approval: None,
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
            tool_authorization_mode: None,
            session_id: Some("deepseek-fragmented".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
            plan_execution_approval: None,
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
            tool_authorization_mode: None,
            session_id: Some("deepseek-structured-followup".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
            plan_execution_approval: None,
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
            // `question` becomes the prompt surfaced in the persisted request (P2-9).
            arguments: json!({ "question": "继续吗？", "description": "test" }),
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
            tool_authorization_mode: None,
            session_id: Some("session-ask-loop".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
            plan_execution_approval: None,
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
            arguments: json!({ "question": "确认？", "description": "test" }),
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
            tool_authorization_mode: None,
            session_id: Some("session-ask-resume".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
            plan_execution_approval: None,
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
    // wave-36 PA-139：本端口在 turn 1（ask 挂起，不发 provider 请求）与 resume 之间存在
    // 时间窗，并行负载下其它用例的杂散连接可能提前消费队列里唯一的响应，导致 resume 的
    // 真实请求只能拿到 503×5。归属校验：实测线缆体只含 provider-native transcript 回放
    // （注入的 assistant tool-call + tool result 对，不含 system/session 元数据），故以
    // 本用例特有的注入 call id `forced-ask-call` 作为判据——杂散连接（如空白 {}、其它
    // 用例形态）不匹配即 503 且不消费队列。
    let server = MockHttpServer::start_with_predicate(
        vec![json_completion("好的，我已经看到你的回答，继续。")],
        std::sync::Arc::new(|body: &str| body.contains("forced-ask-call")),
    );
    let mut runtime = AgentRuntime::with_dependencies(
        SessionStore::memory_only(),
        Box::new(StaticResolver {
            selection: test_chat_provider_selection(server.base_url.clone()),
        }),
        Box::new(executor),
        Box::new(AskOnceThenDeferPlanner {
            tool_name: "Ask".to_string(),
            arguments: json!({ "question": "继续吗？", "description": "test" }),
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
            tool_authorization_mode: None,
            session_id: Some("session-ask-inject".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
            plan_execution_approval: None,
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
            tool_authorization_mode: None,
            session_id: Some("session-ask-inject".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
            plan_execution_approval: None,
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
    // ask_dispatcher → auto-wired via build()). The durable Ask store is injected explicitly:
    // this runtime's session store is memory-only, and the scoped CAS surface requires a
    // persisted record (ADR 0074 contract §4 — no memory-mirror fallback).
    let ask_store: Arc<dyn crate::agent::ask_persistence::AskPersistenceStore> = Arc::from(
        crate::agent::ask_persistence::open_in_memory_ask_store()
            .expect("in-memory ask store must open"),
    );
    let control_plane = HostControlPlaneBuilder::new()
        .runtime(runtime)
        .ask_store(Arc::clone(&ask_store))
        .build();

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
            arguments: json!({ "question": "共享测试", "description": "test" }),
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

    // Host answers through the scoped control-plane surface — same dispatcher → CAS succeeds.
    let consumed = control_plane
        .answer_ask_in_session("session-shared", &request_id, 1, json!("是的"))
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
            arguments: json!({ "question": "继续吗？", "description": "test" }),
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
            tool_authorization_mode: None,
            session_id: Some("session-ask-stream".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
            plan_execution_approval: None,
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
            tool_authorization_mode: None,
            session_id: Some("session-repro-glob".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
            plan_execution_approval: None,
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
            arguments: json!({ "question": "确认？", "description": "test" }),
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
            tool_authorization_mode: None,
            session_id: Some("session-ask-stream-resume".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
            plan_execution_approval: None,
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
    let root = { let raw = std::env::temp_dir().join(format!(
        "pony-ask-runtime-test-{}-{}",
        name,
        std::process::id()
    )); raw.canonicalize().unwrap_or(raw) };
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
        let dir = { let raw = std::env::temp_dir().join(format!("pony-parity-{name}-{stamp}")); raw.canonicalize().unwrap_or(raw) };
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
                        tool_authorization_mode: None,
                        session_id: Some("parity-no-tool".to_string()),
                        node_id: None,
                        history: Vec::new(),
                        images: Vec::new(),
                        workspace_id: None,
                        plan_execution_approval: None,
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
                            tool_authorization_mode: None,
                            session_id: Some("parity-tool".to_string()),
                            node_id: None,
                            history: Vec::new(),
                            images: Vec::new(),
                            workspace_id: None,
                            plan_execution_approval: None,
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
                        tool_authorization_mode: None,
                        session_id: Some("parity-failed".to_string()),
                        node_id: None,
                        history: Vec::new(),
                        images: Vec::new(),
                        workspace_id: None,
                        plan_execution_approval: None,
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
        let dir = { let raw = std::env::temp_dir().join(format!("pony-parity-multiturn-{stamp}")); raw.canonicalize().unwrap_or(raw) };
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
                        tool_authorization_mode: None,
                        session_id: Some("parity-multi-turn".to_string()),
                        node_id: None,
                        history: Vec::new(),
                        images: Vec::new(),
                        workspace_id: None,
                        plan_execution_approval: None,
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
                            tool_authorization_mode: None,
                            session_id: Some("parity-multi-hop".to_string()),
                            node_id: None,
                            history: Vec::new(),
                            images: Vec::new(),
                            workspace_id: None,
                            plan_execution_approval: None,
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
        let dir = { let raw = std::env::temp_dir().join(format!("pony-history-tool-leak-{stamp}")); raw.canonicalize().unwrap_or(raw) };
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
                        tool_authorization_mode: None,
                        session_id: Some(SESSION.to_string()),
                        node_id: None,
                        history: Vec::new(),
                        images: Vec::new(),
                        workspace_id: None,
                        plan_execution_approval: None,
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
        let dir = { let raw = std::env::temp_dir().join(format!("pony-parity-cancel-{stamp}")); raw.canonicalize().unwrap_or(raw) };
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
            assert!(control
                .request_stop(
                    "parity-cancel-turn",
                    crate::agent::stop_escalation::StopReason::User
                )
                .accepted);
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
                        tool_authorization_mode: None,
                        session_id: Some("parity-cancel".to_string()),
                        node_id: None,
                        history: Vec::new(),
                        images: Vec::new(),
                        workspace_id: None,
                        plan_execution_approval: None,
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
        let dir = { let raw = std::env::temp_dir().join(format!("pony-parity-branch-{stamp}")); raw.canonicalize().unwrap_or(raw) };
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
                            tool_authorization_mode: None,
                            session_id: Some("parity-branch".to_string()),
                            node_id: None,
                            history: Vec::new(),
                            images: Vec::new(),
                            workspace_id: None,
                            plan_execution_approval: None,
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

    // ── Stage1 红相验收：tool_result_failure_kind error:null 误分类（Bug B） ──
    // 桌面 Windows 会话中，Run 工具成功时 output 含 `"error": null`。旧实现
    // `parsed.get("error")` 对 `null` 返回 Some(Null)，被当成"有 error"→ 所有成功
    // Run 都被分类为 CapabilityFailureKind::InvocationFailed（UI 误标"调用失败"）。
    // 修复预期：error 为 JSON null 时按"无 error"处理。

    #[test]
    fn bug_b_ok_run_with_error_null_is_none_not_invocation_failed() {
        // B1：status="ok"、error:null、exitCode:0、ok:true 的成功 Run → None。
        // output 形状仿照 tools.rs run_command 成功路径（~L1942）逐字段复刻。
        let tool_result = ToolResult {
            tool_name: "workspace_run_command".to_string(),
            status: "ok".to_string(),
            output: json!({
                "ok": true,
                "cwd": ".",
                "absoluteCwd": "C:\\Users\\test\\work",
                "command": "powershell -NoProfile -Command \"Write-Output 'hello'\"",
                "delegateTool": "run_shell",
                "timeoutMs": 30000,
                "exitCode": 0,
                "stdout": "hello\r\n",
                "stderr": "",
                "stderrTruncated": false,
                "stdoutTruncated": false,
                "error": null,
                "summary": { "text": "命令在 . 执行完成，退出码为 0。" },
                "permission": {
                    "requiresApproval": false,
                    "permissionScope": "workspace.execute",
                    "hostMediated": false,
                    "permissionProfile": "builtin",
                    "approvalMode": "none",
                    "decisionSource": "runtime"
                }
            })
            .to_string(),
            duration_ms: 0,
        };
        assert_eq!(
            tool_result_failure_kind(&tool_result),
            None,
            "status=ok 且 error:null 的 Run 成功结果不应被分类为失败（Bug B 红相）"
        );
    }

    #[test]
    fn bug_b_error_status_or_error_object_is_invocation_failed() {
        // B2 回归：status="error"（无 error 字段）→ InvocationFailed。
        let status_error = ToolResult {
            tool_name: "workspace_run_command".to_string(),
            status: "error".to_string(),
            output: json!({ "ok": false }).to_string(),
            duration_ms: 0,
        };
        assert_eq!(
            tool_result_failure_kind(&status_error),
            Some(CapabilityFailureKind::InvocationFailed)
        );

        // B2 回归：error 为对象（含 kind/code/message）→ InvocationFailed。
        let object_error = ToolResult {
            tool_name: "workspace_run_command".to_string(),
            status: "error".to_string(),
            output: json!({
                "ok": false,
                "exitCode": 1,
                "error": {
                    "code": "non_zero_exit",
                    "kind": "non_zero_exit",
                    "message": "命令执行完成，但退出码为 1。"
                }
            })
            .to_string(),
            duration_ms: 0,
        };
        assert_eq!(
            tool_result_failure_kind(&object_error),
            Some(CapabilityFailureKind::InvocationFailed)
        );
    }

    #[test]
    fn bug_b_ok_without_error_field_is_none() {
        // B3 回归：status="ok" 且 output 无 error 字段 → None。
        let tool_result = ToolResult {
            tool_name: "workspace_read_file".to_string(),
            status: "ok".to_string(),
            output: json!({ "content": "hello" }).to_string(),
            duration_ms: 0,
        };
        assert_eq!(tool_result_failure_kind(&tool_result), None);
    }

    #[test]
    fn bug_b_ok_with_non_null_error_object_is_invocation_failed() {
        // B4 回归：status="ok" 但 error 为非 null 对象 → InvocationFailed。
        let tool_result = ToolResult {
            tool_name: "workspace_run_command".to_string(),
            status: "ok".to_string(),
            output: json!({
                "ok": false,
                "exitCode": 1,
                "error": {
                    "code": "non_zero_exit",
                    "kind": "non_zero_exit",
                    "message": "命令执行完成，但退出码为 1。"
                }
            })
            .to_string(),
            duration_ms: 0,
        };
        assert_eq!(
            tool_result_failure_kind(&tool_result),
            Some(CapabilityFailureKind::InvocationFailed)
        );
    }

    #[test]
    fn bug_b_ok_with_empty_string_error_is_none() {
        // B5（评审补测）：status="ok" 且 output 含 `"error": ""`（空串 error）→ None。
        // 与 tool_error_from_output 对 status="ok" 恒 None 的读侧语义一致；修复应把空串
        // error 视为无错误。当前实现下红或绿均记录实际（见 .dev-team/red-phase-b.md）。
        let tool_result = ToolResult {
            tool_name: "workspace_run_command".to_string(),
            status: "ok".to_string(),
            output: json!({
                "ok": true,
                "exitCode": 0,
                "error": ""
            })
            .to_string(),
            duration_ms: 0,
        };
        assert_eq!(
            tool_result_failure_kind(&tool_result),
            None,
            "status=ok 且 error 为空串的 Run 成功结果不应被分类为失败"
        );
    }
}

// ============================================================================
// Wave-15 契约验收（Test Agent 唯一编写方）
//
// 契约：.dev-team/contracts/wave-15-token-metrics.md
//   R1：`turn:hop_complete` 必须携带本次已完成调用的 `provider_call_records`（per-call）
//   R2：持久化 timeline 每条 `call_model` 条目携带**自身** input/output/TTFT/duration，
//       整轮聚合只落在 `checkpoint_persist`（及轮级记录）
//   T6：多跳轮持久化 timeline 的 per-call 断言
//
// 红相纪律：断言失败必须来自 SUT（`turn_stream.rs` hop 事件构造、`trace_timeline.rs`
// 持久化构建器）。本测试同时把捕获到的真实 `TurnStreamEvent` 序列 + 持久化 timeline
// 落盘 `.dev-team/evidence/wave15/wire-red.json`（先落盘后断言，红相亦留物理凭证）。
// ============================================================================

const WAVE15_HOP_INPUTS: [u64; 3] = [350_000, 360_000, 370_000];
const WAVE15_HOP_OUTPUTS: [u64; 3] = [50, 60, 100];
const WAVE15_HOP_TTFT_MS: [u64; 3] = [150, 180, 200];
const WAVE15_AGGREGATE_INPUT: u64 = 1_080_000;
const WAVE15_AGGREGATE_OUTPUT: u64 = 210;

fn wave15_evidence_dir() -> PathBuf {
    std::env::temp_dir().join("pony-wave15-evidence")
}

fn wave15_write_evidence(file_name: &str, document: &serde_json::Value) {
    let dir = wave15_evidence_dir();
    fs::create_dir_all(&dir).expect("create wave15 evidence dir");
    let path = dir.join(file_name);
    fs::write(
        &path,
        serde_json::to_string_pretty(document).expect("serialize wave15 wire document"),
    )
    .unwrap_or_else(|error| panic!("write {} failed: {error}", path.display()));
}

#[test]
fn wave15_multihop_stream_exposes_per_call_metrics_in_hop_events_and_persisted_timeline() {
    let final_text = "wave-15 三跳回合已返回最终答案。";
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
                    "prompt_tokens": 350000,
                    "completion_tokens": 50,
                    "total_tokens": 350050
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
                    "prompt_tokens": 360000,
                    "completion_tokens": 60,
                    "total_tokens": 360060
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
                    "prompt_tokens": 370000,
                    "completion_tokens": 100,
                    "total_tokens": 370100
                }
            }),
        ]),
    ]);
    let mut runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));
    let sink = RecordingTurnEventSink::new();

    runtime.start_turn_stream(
        &sink,
        "turn-wave15-wire".to_string(),
        TurnInput {
            message: "wave-15 三跳 token 口径验收".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            workspace_mode: None,
            tool_authorization_mode: None,
            session_id: Some("session-wave15-wire".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
            plan_execution_approval: None,
        },
    );

    let events = sink.events.borrow();
    let hop_events: Vec<serde_json::Value> = events
        .iter()
        .filter(|(name, _)| name == "turn:hop_complete")
        .map(|(_, payload)| serde_json::to_value(payload).expect("serialize hop event"))
        .collect();
    let completed = events
        .iter()
        .find_map(|(name, payload)| (name == "turn:completed").then_some(payload.clone()))
        .expect("turn:completed event");
    let output_end = events
        .iter()
        .find_map(|(name, payload)| (name == "turn:output_end").then_some(payload.clone()))
        .expect("turn:output_end event");

    let snapshot = runtime.load_session_snapshot(Some("session-wave15-wire"));
    let trace = snapshot
        .turn_trace_history
        .last()
        .expect("wave15 persisted trace")
        .clone();

    // ---- 物理凭证：真实 wire 事件序列 + 持久化 timeline（先落盘后断言） ----
    let wire_events: Vec<serde_json::Value> = events
        .iter()
        .map(|(name, payload)| {
            json!({
                "name": name,
                "payload": serde_json::to_value(payload).expect("serialize wave15 event"),
            })
        })
        .collect();
    let wire = json!({
        "wave": "wave-15",
        "phase": "red",
        "turnId": "turn-wave15-wire",
        "sessionId": "session-wave15-wire",
        "expectation": {
            "hopInputs": WAVE15_HOP_INPUTS,
            "hopOutputs": WAVE15_HOP_OUTPUTS,
            "hopFirstTokenLatencyMs": WAVE15_HOP_TTFT_MS,
            "aggregateInputTokens": WAVE15_AGGREGATE_INPUT,
            "aggregateOutputTokens": WAVE15_AGGREGATE_OUTPUT,
        },
        "turnAggregate": {
            "inputTokens": completed.input_tokens,
            "outputTokens": completed.output_tokens,
            "totalTokens": completed.total_tokens,
        },
        "events": wire_events,
        "persistedTimeline": serde_json::to_value(&trace.trace_timeline)
            .expect("serialize persisted timeline"),
        "persistedTurnTrace": serde_json::to_value(&trace).expect("serialize persisted trace"),
    });
    wave15_write_evidence("wire-red.json", &wire);
    if !wave15_evidence_dir().join("wire-red-frozen.json").exists() {
        wave15_write_evidence("wire-red-frozen.json", &wire);
    }

    // ---- R1（契约 §7.10，不变量 I1/I2/I3）：turn:hop_complete 载荷语义 ----
    // 协议事实：hop_complete 只在「该次模型调用后仍需执行工具」时发出（工具跳），
    // 末次模型调用直接走 turn:output_end / turn:completed；本回合 3 次调用 → 2 个 hop_complete。
    // 映射（§7.10）：hop_complete #k（0-based）的 records.last() = 第 k+1 次调用。
    assert_eq!(
        hop_events.len(),
        2,
        "三跳回合（其中 2 跳触发工具）必须产生 2 个 turn:hop_complete，实际 {}",
        hop_events.len()
    );
    let mut previous_record_len = 0_usize;
    for (index, payload) in hop_events.iter().enumerate() {
        let records = payload
            .get("providerCallRecords")
            .and_then(serde_json::Value::as_array)
            .unwrap_or_else(|| {
                panic!(
                    "hop {index} 的 turn:hop_complete 未携带 providerCallRecords（契约 R1/§7.10）；实际载荷: {payload}"
                )
            });

        // I1 集合一致：Σ records[i].input_tokens == payload.inputTokens（output 同理）。
        // 该断言必须先于 I2/I3，使滞后实现（records[..len-1]）的失败原因落在 I1。
        let payload_input = payload
            .get("inputTokens")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_else(|| panic!("hop {index} 的 turn:hop_complete 必须携带累加 inputTokens"));
        let payload_output = payload
            .get("outputTokens")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_else(|| panic!("hop {index} 的 turn:hop_complete 必须携带累加 outputTokens"));
        let sum_input: u64 = records
            .iter()
            .map(|record| {
                record
                    .get("inputTokens")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or_else(|| {
                        panic!("hop {index} 的 records[{record:?}] 缺少 inputTokens（§7.10 I1）")
                    })
            })
            .sum();
        let sum_output: u64 = records
            .iter()
            .map(|record| {
                record
                    .get("outputTokens")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or_else(|| {
                        panic!("hop {index} 的 records[{record:?}] 缺少 outputTokens（§7.10 I1）")
                    })
            })
            .sum();
        assert_eq!(
            sum_input, payload_input,
            "hop {index} 违反 §7.10 I1：Σ records[i].input_tokens ({sum_input}) != payload.inputTokens ({payload_input}) \
             —— records 必须是「截至该事件已完成的全部调用」，禁止排除末条（滞后一跳）"
        );
        assert_eq!(
            sum_output, payload_output,
            "hop {index} 违反 §7.10 I1：Σ records[i].output_tokens ({sum_output}) != payload.outputTokens ({payload_output})"
        );

        // I3 有序前缀：长度按 hop 单调 +1（首个 hop 至少含 2 次调用：call#0 + call#1）。
        let expected_record_len = if index == 0 { 2 } else { previous_record_len + 1 };
        assert_eq!(
            records.len(),
            expected_record_len,
            "hop {index} 违反 §7.10 I3：records 长度应为 {expected_record_len}（首 hop 起始为 2，之后每 hop +1），实际 {}（上一 hop {previous_record_len}）",
            records.len()
        );
        previous_record_len = records.len();

        // I2 末条即最新已完成调用：hop #k 的末条 = 第 k+1 次调用（hopInputs[k+1]）。
        let last = records
            .last()
            .unwrap_or_else(|| panic!("hop {index} 的 providerCallRecords 为空（契约 R1）"));
        assert_eq!(
            last.get("inputTokens").and_then(serde_json::Value::as_u64),
            Some(WAVE15_HOP_INPUTS[index + 1]),
            "hop {index} 违反 §7.10 I2：末条记录的 inputTokens 必须是第 {} 次调用（{}）",
            index + 1,
            WAVE15_HOP_INPUTS[index + 1]
        );
        assert_eq!(
            last.get("outputTokens").and_then(serde_json::Value::as_u64),
            Some(WAVE15_HOP_OUTPUTS[index + 1]),
            "hop {index} 违反 §7.10 I2：末条记录的 outputTokens 必须是第 {} 次调用（{}）",
            index + 1,
            WAVE15_HOP_OUTPUTS[index + 1]
        );
        assert_eq!(
            records[0].get("inputTokens").and_then(serde_json::Value::as_u64),
            Some(WAVE15_HOP_INPUTS[0]),
            "hop {index} 违反 §7.10 I3：records 必须从第 0 次调用起的有序前缀"
        );

        let record_duration = last
            .get("turnDurationMs")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_else(|| {
                panic!("hop {index} 的 per-call turnDurationMs（本次调用耗时）必须存在")
            });
        assert!(
            record_duration > 0,
            "hop {index} 的 per-call turnDurationMs 必须为有效正值"
        );
        let record_ttft = last
            .get("firstTokenLatencyMs")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_else(|| panic!("hop {index} 的 per-call firstTokenLatencyMs 必须存在"));
        assert!(
            record_ttft > 0,
            "hop {index} 的 per-call firstTokenLatencyMs 必须为有效正值"
        );
        // per-call TTFT 与「轮级 TTFT」的可比性边界（wave-15 偶发红的根因）：
        // 轮级 TTFT（`payload.firstTokenLatencyMs`）是**整轮单次采样**——「turn 起点 → 本回合
        // 第一个 delta」（`turn_stream.rs` 里 `first_token_latency` 跨 hop 只写一次）；而本
        // hop 的 `last()` 是**第 k+1 次调用**（§7.10 I2），它的 per-call TTFT 是「第 k+1 次
        // 调用起点 → 该次调用第一个 delta」。两者是**不同区间**，因此「轮级 ≥ per-call」
        // 不是不变量：并行负载下后者完全可能更慢（实测红：
        // `hop 0 的 per-call TTFT (80ms) 不得大于轮级 TTFT (71ms)`，复现率 1/15）。
        // 注意：即便是 records[0]（第 0 次调用）与轮级 TTFT 也只是**两次独立采样**
        // （起点不同、读取时刻相差若干指令），跨调度抢占后仍无逻辑保证 → 一律不做跨区间比较。
        // 改为断言两条同区间、逻辑恒真的不变量（鉴别力更强，且不依赖负载）：
        // (a) 轮级 TTFT 必须存在且为正（「TTFT 可信门控」真正要保护的东西）；
        // (b) 任一条 per-call TTFT ≤ 该条自身的 turnDurationMs（同一起点、后者到调用结束）；
        //     若有人把 per-call TTFT 改成从 turn 起点或整轮聚合值回填，这条必然红。
        // 另加 records[0] 的流式门控断言，避免「首条记录悄悄退化为 buffered（无 TTFT）」。
        let turn_level_ttft = payload
            .get("firstTokenLatencyMs")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_else(|| {
                panic!("hop {index} 的 turn:hop_complete 必须携带可用的轮级 firstTokenLatencyMs")
            });
        assert!(
            turn_level_ttft > 0,
            "hop {index} 的轮级 TTFT 必须为有效正值（TTFT 可信门控 §7.2）"
        );
        assert_eq!(
            records[0].get("latencyKind"),
            Some(&serde_json::to_value(ProviderLatencyKind::ProviderStream).unwrap()),
            "hop {index} 的 records[0]（第 0 次调用）latencyKind 必须为 provider_stream"
        );
        assert!(
            records[0]
                .get("firstTokenLatencyMs")
                .and_then(serde_json::Value::as_u64)
                .is_some_and(|value| value > 0),
            "hop {index} 的 records[0]（第 0 次调用）必须携带正值 per-call firstTokenLatencyMs"
        );
        for (record_index, record) in records.iter().enumerate() {
            let (Some(ttft), Some(duration)) = (
                record
                    .get("firstTokenLatencyMs")
                    .and_then(serde_json::Value::as_u64),
                record
                    .get("turnDurationMs")
                    .and_then(serde_json::Value::as_u64),
            ) else {
                continue;
            };
            assert!(
                ttft <= duration,
                "hop {index} 的 records[{record_index}] per-call TTFT ({ttft}ms) 不得大于该次调用\
                 总耗时 ({duration}ms)——同一起点，TTFT 必须落在调用区间内"
            );
        }
        assert_eq!(
            last.get("latencyKind"),
            Some(&serde_json::to_value(ProviderLatencyKind::ProviderStream).unwrap()),
            "hop {index} 的 per-call latencyKind 必须为 provider_stream（TTFT 可信门控 §7.2）"
        );
    }

    // ---- R1（§7.10）turn:output_end 载荷：I1 + I3（长度 == 调用总数） ----
    let output_end_records = output_end
        .provider_call_records
        .as_ref()
        .expect("turn:output_end 必须携带 provider_call_records（§7.10）");
    assert_eq!(
        output_end_records.len(),
        3,
        "§7.10 I3：turn:output_end 的 records 长度必须等于调用总数（3）"
    );
    let output_end_sum_input: u64 = output_end_records
        .iter()
        .map(|record| record.input_tokens.unwrap_or(0))
        .sum();
    let output_end_sum_output: u64 = output_end_records
        .iter()
        .map(|record| record.output_tokens.unwrap_or(0))
        .sum();
    assert_eq!(
        Some(output_end_sum_input),
        output_end.input_tokens,
        "§7.10 I1：turn:output_end 的 Σ records.input_tokens 必须等于其累加 inputTokens"
    );
    assert_eq!(
        Some(output_end_sum_output),
        output_end.output_tokens,
        "§7.10 I1：turn:output_end 的 Σ records.output_tokens 必须等于其累加 outputTokens"
    );
    assert_eq!(
        output_end_records.last().and_then(|record| record.input_tokens),
        Some(WAVE15_HOP_INPUTS[2]),
        "§7.10 I2：turn:output_end 的末条记录必须是末次调用（370000）"
    );

    // 轮级累加语义（S2）不得被改动；末次调用的 per-call 记录由 turn:completed 承载。
    assert_eq!(completed.input_tokens, Some(WAVE15_AGGREGATE_INPUT));
    assert_eq!(completed.output_tokens, Some(WAVE15_AGGREGATE_OUTPUT));
    assert_eq!(trace.input_tokens, Some(WAVE15_AGGREGATE_INPUT));
    let completed_records = completed
        .provider_call_records
        .as_ref()
        .expect("turn:completed 必须携带 provider_call_records（R5 既有语义）");
    assert_eq!(completed_records.len(), 3, "三跳轮应留下 3 条 per-call 记录");
    for (index, record) in completed_records.iter().enumerate() {
        assert_eq!(
            record.input_tokens,
            Some(WAVE15_HOP_INPUTS[index]),
            "第 {index} 条 per-call 记录的 input_tokens 必须是该次调用自身值"
        );
        assert_eq!(
            record.output_tokens,
            Some(WAVE15_HOP_OUTPUTS[index]),
            "第 {index} 条 per-call 记录的 output_tokens 必须是该次调用自身值"
        );
    }

    // ---- R2 / T6：持久化 timeline 每条 call_model 为自身值，聚合只落 checkpoint_persist ----
    let model_entries: Vec<_> = trace
        .trace_timeline
        .iter()
        .filter(|entry| entry.kind.as_str() == "call_model")
        .collect();
    assert_eq!(
        model_entries.len(),
        3,
        "三跳轮持久化 timeline 必须含 3 条 call_model 条目"
    );
    for (index, entry) in model_entries.iter().enumerate() {
        assert_eq!(
            entry.input_tokens,
            Some(WAVE15_HOP_INPUTS[index]),
            "第 {index} 条 call_model 的 input_tokens 必须是其自身调用值（R2 / RC2）"
        );
        assert_eq!(
            entry.output_tokens,
            Some(WAVE15_HOP_OUTPUTS[index]),
            "第 {index} 条 call_model 的 output_tokens 必须是其自身调用值（R2 / RC2）"
        );
        assert!(
            entry.turn_duration_ms.is_some_and(|value| value > 0),
            "第 {index} 条 call_model 必须携带自身 turn_duration_ms（R2）"
        );
    }
    assert_ne!(
        model_entries[2].input_tokens,
        Some(WAVE15_AGGREGATE_INPUT),
        "末条 call_model 严禁被整轮聚合覆盖（RC2）"
    );

    let checkpoint = trace
        .trace_timeline
        .iter()
        .find(|entry| entry.kind.as_str() == "checkpoint_persist")
        .expect("completed 轮必须存在 checkpoint_persist 条目承载整轮聚合");
    assert_eq!(checkpoint.input_tokens, Some(WAVE15_AGGREGATE_INPUT));
    assert_eq!(checkpoint.output_tokens, Some(WAVE15_AGGREGATE_OUTPUT));

    let _ = server.finish();
}

// ───────────────────── D2（P1-1 切片二）：批量工具结果 followup 入口 ─────────────────────
//
// 三个断言面（设计 `multi-tool-call-execution.md` §6 验收）：
// ① N=2 的 wire 级请求体（OpenAI `role=tool` 恰 2 条且 `tool_call_id` 保序；
//    Anthropic 恰 1 条 `role=user` 承载 2 个 `tool_result`）；
// ② N=1 时与既有 1:1 入口**逐字等价**（走的就是同一条代码路径）；
// ③ 长度不匹配 ⇒ `Err` 且 mock server **零请求**（含流式入口）。

fn d2_batch_provider_selection(base_url: String) -> ResolvedProviderSelection {
    test_provider_selection(base_url)
}

fn d2_anthropic_provider_selection(base_url: String) -> ResolvedProviderSelection {
    let mut selection = test_provider_selection(base_url);
    selection.requested_name = "test-anthropic".to_string();
    selection.provider_name = "test-anthropic".to_string();
    selection.protocol = crate::agent::provider::ProviderProtocol::AnthropicMessages;
    selection.auth_type = crate::agent::provider::ProviderAuthType::XApiKey;
    selection.api_key_env_var = "ANTHROPIC_API_KEY".to_string();
    selection.model = "claude-sonnet-5".to_string();
    selection.thinking_param_pattern = ThinkingParamPattern::None;
    selection
}

fn d2_batch_provider_request() -> ProviderRequest {
    ProviderRequest {
        model: "gpt-5.4".to_string(),
        input: vec![ProviderMessage::user("一次响应里调用了两个工具，请汇总结果")],
        images: Vec::new(),
        native_messages: Vec::new(),
        observation: ProviderRequestObservation::default(),
        temperature: 0.2,
        max_output_tokens: 1024,
    }
}

fn d2_batch_tools() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "read_file",
            description: "读取文件内容。",
            input_schema: json!({ "type": "object", "properties": {} }),
        },
        ToolDefinition {
            name: "list_dir",
            description: "列出目录内容。",
            input_schema: json!({ "type": "object", "properties": {} }),
        },
    ]
}

fn d2_tool_call(call_id: &str, name: &str) -> ToolCall {
    ToolCall {
        call_id: Some(call_id.to_string()),
        name: name.to_string(),
        arguments: json!({ "path": "." }),
        plan: None,
    }
}

fn d2_tool_result(name: &str, output: &str) -> ToolResult {
    ToolResult {
        tool_name: name.to_string(),
        status: "ok".to_string(),
        output: output.to_string(),
        duration_ms: 1,
    }
}

/// ① wire 级请求体断言（N=2）。
#[test]
fn d2_batch_tool_results_wire_shape_openai_and_anthropic() {
    // ---- OpenAI chat completions：1 assistant（全部 tool_calls）+ **N 条** role=tool ----
    let openai_server = MockHttpServer::start(vec![json_response(json!({
        "choices": [
            {
                "message": {
                    "role": "assistant",
                    "content": "两个工具结果已汇总。"
                }
            }
        ],
        "usage": { "prompt_tokens": 30, "completion_tokens": 8, "total_tokens": 38 }
    }))]);
    let openai_manager = ProviderManager::new(d2_batch_provider_selection(
        openai_server.base_url.clone(),
    ));
    let request = d2_batch_provider_request();
    let tools = d2_batch_tools();
    let calls = vec![
        d2_tool_call("call_read", "read_file"),
        d2_tool_call("call_list", "list_dir"),
    ];
    let results = vec![
        d2_tool_result("read_file", "read ok"),
        d2_tool_result("list_dir", "list ok"),
    ];
    let mut openai_accumulated = Vec::new();
    let openai_response = openai_manager
        .continue_with_tool_results(
            &request,
            &tools,
            &mut openai_accumulated,
            None,
            &calls,
            &results,
        )
        .expect("批量 followup（OpenAI）应成功");

    let openai_recorded = openai_server.finish();
    assert_eq!(
        openai_recorded.len(),
        1,
        "整批工具结果必须只发一次 followup 请求，实际 {} 次",
        openai_recorded.len()
    );
    let openai_body: Value =
        serde_json::from_str(&openai_recorded[0]).expect("openai body 应为 json");
    assert_eq!(
        openai_body.get("stream").and_then(Value::as_bool),
        Some(false),
        "N>1 走既有同步 post 通道"
    );
    let openai_messages = openai_body
        .get("messages")
        .and_then(Value::as_array)
        .expect("openai messages");
    let tool_messages: Vec<&Value> = openai_messages
        .iter()
        .filter(|message| message.get("role").and_then(Value::as_str) == Some("tool"))
        .collect();
    assert_eq!(
        tool_messages.len(),
        2,
        "N=2 时 role=tool 消息必须恰 2 条（每调用一条），实际 {openai_messages:?}"
    );
    let tool_message_ids: Vec<&str> = tool_messages
        .iter()
        .map(|message| {
            message
                .get("tool_call_id")
                .and_then(Value::as_str)
                .expect("role=tool 必须带 tool_call_id")
        })
        .collect();
    assert_eq!(
        tool_message_ids,
        vec!["call_read", "call_list"],
        "tool 消息的 tool_call_id 必须与执行顺序一致"
    );
    let assistant_tool_messages: Vec<&Value> = openai_messages
        .iter()
        .filter(|message| {
            message.get("role").and_then(Value::as_str) == Some("assistant")
                && message
                    .get("tool_calls")
                    .and_then(Value::as_array)
                    .map(|calls| !calls.is_empty())
                    .unwrap_or(false)
        })
        .collect();
    assert_eq!(
        assistant_tool_messages.len(),
        1,
        "回填的 assistant 必须是**一条**消息承载全部 tool_calls"
    );
    let assistant_tool_call_ids: Vec<&str> = assistant_tool_messages[0]
        .get("tool_calls")
        .and_then(Value::as_array)
        .expect("assistant tool_calls")
        .iter()
        .map(|call| call.get("id").and_then(Value::as_str).expect("tool_call id"))
        .collect();
    assert_eq!(
        assistant_tool_call_ids,
        vec!["call_read", "call_list"],
        "assistant.tool_calls 顺序必须与执行顺序一致"
    );
    assert_eq!(openai_response.output_text, "两个工具结果已汇总。");
    assert_eq!(openai_response.provider_source, "provider_followup_sync");
    // 累积转录被更新为「基础 + assistant + 2 tool」，供下一跳复用。
    assert_eq!(openai_accumulated.len(), openai_messages.len());

    // ---- Anthropic：1 assistant（全部 tool_use）+ **1 条** user（N 个 tool_result）----
    let anthropic_server = MockHttpServer::start(vec![json_response(json!({
        "content": [{ "type": "text", "text": "两个工具结果已汇总。" }],
        "usage": { "input_tokens": 20, "output_tokens": 9 }
    }))]);
    let anthropic_manager = ProviderManager::new(d2_anthropic_provider_selection(
        anthropic_server.base_url.clone(),
    ));
    let anthropic_calls = vec![
        d2_tool_call("toolu_read", "read_file"),
        d2_tool_call("toolu_list", "list_dir"),
    ];
    let anthropic_results = vec![
        d2_tool_result("read_file", "read ok"),
        d2_tool_result("list_dir", "list ok"),
    ];
    let mut anthropic_request = request.clone();
    anthropic_request.model = "claude-sonnet-5".to_string();
    let mut anthropic_accumulated = Vec::new();
    let anthropic_response = anthropic_manager
        .continue_with_tool_results(
            &anthropic_request,
            &tools,
            &mut anthropic_accumulated,
            None,
            &anthropic_calls,
            &anthropic_results,
        )
        .expect("批量 followup（Anthropic）应成功");

    let anthropic_recorded = anthropic_server.finish();
    assert_eq!(
        anthropic_recorded.len(),
        1,
        "整批工具结果必须只发一次 followup 请求，实际 {} 次",
        anthropic_recorded.len()
    );
    let anthropic_body: Value =
        serde_json::from_str(&anthropic_recorded[0]).expect("anthropic body 应为 json");
    let anthropic_messages = anthropic_body
        .get("messages")
        .and_then(Value::as_array)
        .expect("anthropic messages");
    assert_eq!(
        anthropic_messages.len(),
        3,
        "基础 user + 1 assistant + 1 user(tool_result)，实际 {anthropic_messages:?}"
    );
    let assistant_blocks = anthropic_messages[1]
        .get("content")
        .and_then(Value::as_array)
        .expect("assistant content blocks");
    let tool_use_ids: Vec<&str> = assistant_blocks
        .iter()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("tool_use"))
        .map(|block| block.get("id").and_then(Value::as_str).expect("tool_use id"))
        .collect();
    assert_eq!(
        tool_use_ids,
        vec!["toolu_read", "toolu_list"],
        "assistant 必须承载全部 tool_use 且保序"
    );
    let tool_result_messages: Vec<&Value> = anthropic_messages
        .iter()
        .filter(|message| {
            message
                .get("content")
                .and_then(Value::as_array)
                .map(|blocks| {
                    blocks
                        .iter()
                        .any(|block| block.get("type").and_then(Value::as_str) == Some("tool_result"))
                })
                .unwrap_or(false)
        })
        .collect();
    assert_eq!(
        tool_result_messages.len(),
        1,
        "N 个 tool_result 必须收在**一条** user 消息里（拆成多条会触发上游 400）"
    );
    assert_eq!(
        tool_result_messages[0].get("role").and_then(Value::as_str),
        Some("user")
    );
    let tool_result_blocks = tool_result_messages[0]
        .get("content")
        .and_then(Value::as_array)
        .expect("tool_result blocks");
    assert_eq!(tool_result_blocks.len(), 2, "user.content 必须含 2 个 tool_result");
    assert!(tool_result_blocks
        .iter()
        .all(|block| block.get("type").and_then(Value::as_str) == Some("tool_result")));
    let tool_result_ids: Vec<&str> = tool_result_blocks
        .iter()
        .map(|block| {
            block
                .get("tool_use_id")
                .and_then(Value::as_str)
                .expect("tool_result 必须带 tool_use_id")
        })
        .collect();
    assert_eq!(
        tool_result_ids,
        vec!["toolu_read", "toolu_list"],
        "tool_result 的 tool_use_id 必须与执行顺序一致"
    );
    assert_eq!(anthropic_response.output_text, "两个工具结果已汇总。");
    assert_eq!(anthropic_response.provider_source, "provider_followup_sync");
    assert_eq!(anthropic_accumulated.len(), anthropic_messages.len());
}

/// ② N=1：批量入口必须**逐字委派**既有 1:1 入口（同请求体 / 同响应 / 同累积转录）。
#[test]
fn d2_batch_tool_results_single_call_delegates_to_one_to_one_entry() {
    let server = MockHttpServer::start(vec![
        json_completion("单调用回填完成。"),
        json_completion("单调用回填完成。"),
    ]);
    let manager = ProviderManager::new(d2_batch_provider_selection(server.base_url.clone()));
    let request = d2_batch_provider_request();
    let tools = d2_batch_tools();
    let calls = vec![d2_tool_call("call_one", "read_file")];
    let results = vec![d2_tool_result("read_file", "read ok")];
    let assistant = json!({
        "role": "assistant",
        "content": "先读文件。",
        "tool_calls": [{
            "id": "call_one",
            "type": "function",
            "function": { "name": "read_file", "arguments": "{\"path\":\".\"}" }
        }]
    });

    let mut batch_accumulated = Vec::new();
    let batch_response = manager
        .continue_with_tool_results(
            &request,
            &tools,
            &mut batch_accumulated,
            Some(&assistant),
            &calls,
            &results,
        )
        .expect("N=1 批量入口应成功");
    let mut legacy_accumulated = Vec::new();
    let legacy_response = manager
        .continue_with_tool_result(
            &request,
            &tools,
            &mut legacy_accumulated,
            Some(&assistant),
            &calls[0],
            &results[0],
        )
        .expect("既有 1:1 入口应成功");

    let recorded = server.finish();
    assert_eq!(recorded.len(), 2, "两个入口各发一次请求");
    let batch_body: Value = serde_json::from_str(&recorded[0]).expect("batch body 应为 json");
    let legacy_body: Value = serde_json::from_str(&recorded[1]).expect("legacy body 应为 json");
    assert_eq!(
        batch_body, legacy_body,
        "N=1 时批量入口的 wire 请求体必须与既有 1:1 入口逐字相同（委派等价）"
    );
    assert_eq!(batch_response.output_text, legacy_response.output_text);
    assert_eq!(
        batch_response.provider_source, legacy_response.provider_source,
        "N=1 必须落在既有 provider_source（provider_followup_sync）"
    );
    assert_eq!(
        batch_accumulated, legacy_accumulated,
        "累积转录必须与 1:1 路径完全一致"
    );
}

/// ③ 长度不匹配：返回 `Err` 且 mock server **零请求**（同步 + 流式入口各验一次）。
#[test]
fn d2_batch_tool_results_length_mismatch_sends_zero_requests() {
    let sync_server = MockHttpServer::start(vec![json_completion("不该被调用")]);
    let stream_server = MockHttpServer::start(vec![json_completion("不该被调用")]);
    let sync_manager = ProviderManager::new(d2_batch_provider_selection(
        sync_server.base_url.clone(),
    ));
    let stream_manager = ProviderManager::new(d2_batch_provider_selection(
        stream_server.base_url.clone(),
    ));
    let request = d2_batch_provider_request();
    let tools = d2_batch_tools();
    let calls = vec![
        d2_tool_call("call_a", "read_file"),
        d2_tool_call("call_b", "list_dir"),
    ];
    let results = vec![d2_tool_result("read_file", "read ok")];
    let expected = "tool_calls/tool_results length mismatch: calls=2 results=1";

    let mut accumulated = Vec::new();
    let sync_error = sync_manager
        .continue_with_tool_results(&request, &tools, &mut accumulated, None, &calls, &results)
        .expect_err("长度不匹配必须返回 Err");
    assert_eq!(sync_error, expected);
    let stream_error = stream_manager
        .continue_with_tool_results_stream(
            &request,
            &tools,
            &mut accumulated,
            None,
            &calls,
            &results,
            |_chunk| {},
        )
        .expect_err("长度不匹配（流式入口）必须返回 Err");
    assert_eq!(stream_error, expected);
    assert!(
        accumulated.is_empty(),
        "长度不匹配不得写入累积转录，实际 {accumulated:?}"
    );

    let sync_recorded = sync_server.finish();
    let stream_recorded = stream_server.finish();
    assert!(
        sync_recorded.is_empty(),
        "长度不匹配时同步入口不得发任何请求，实际记录 {} 条：{sync_recorded:?}",
        sync_recorded.len()
    );
    assert!(
        stream_recorded.is_empty(),
        "长度不匹配时流式入口不得发任何请求，实际记录 {} 条：{stream_recorded:?}",
        stream_recorded.len()
    );
}

// ─────────── D3a（P1-1 / B018）：非流式路径的**批量**工具执行 ───────────

/// 一次上游响应携带**多个**调用（保序），供批执行用例使用。
fn decision_tool_calls_batch(calls: &[(&str, serde_json::Value)]) -> serde_json::Value {
    let tool_calls: Vec<serde_json::Value> = calls
        .iter()
        .map(|(name, arguments)| {
            json!({
                "id": format!("call_{}", name),
                "type": "function",
                "function": { "name": name, "arguments": arguments.to_string() }
            })
        })
        .collect();
    json!({
        "choices": [
            {
                "message": {
                    "role": "assistant",
                    "content": "一次执行这些工具。",
                    "reasoning_content": "需要一次执行多个工具调用。",
                    "tool_calls": tool_calls
                }
            }
        ]
    })
}

/// Ask 类工具执行器：`ask_user` 返回 `control_outcome_pending`（真实治理层的挂起标记形态），
/// 其余调用透传 Stub；每次调用都记录名字，用于断言「批到 Ask 即停」。
struct AskPendingToolExecutor {
    executed: Arc<Mutex<Vec<String>>>,
}

impl crate::agent::tools::ToolExecutor for AskPendingToolExecutor {
    fn execute(&self, call: &ToolCall) -> crate::agent::tools::ToolResult {
        self.executed.lock().unwrap().push(call.name.clone());
        let is_ask = crate::agent::tools::canonical_tool_name(&call.name)
            .map(|canonical| canonical == "ask_user")
            .unwrap_or(false);
        if !is_ask {
            return StubToolExecutor.execute(call);
        }
        crate::agent::tools::ToolResult {
            tool_name: call.name.clone(),
            status: "error".to_string(),
            output: json!({
                "ok": false,
                "error": {
                    "code": "control_outcome_pending",
                    "kind": "control_outcome_pending",
                    "message": "Ask 等待宿主应答。"
                }
            })
            .to_string(),
            duration_ms: 0,
        }
    }
}

/// D3a ①：OpenAI 一次返回 2 个 `tool_calls` ⇒ **两个工具都执行**（两份工具结果），
/// 且批后**一次** followup 的请求体是「1 条 assistant（含全部 tool_calls）+ 2 条 `role=tool`」。
#[test]
fn run_turn_executes_batch_of_openai_tool_calls_and_backfills_one_assistant_plus_n_tool_messages() {
    let final_text = "两个工具都已执行完成。";
    let server = MockHttpServer::start(vec![
        json_response(decision_tool_calls_batch(&[
            ("workspace_list_files", json!({"path": ".", "limit": 40})),
            ("workspace_read_file", json!({"path": "tauri.conf.json"})),
        ])),
        json_completion(final_text),
    ]);
    let executed: Arc<Mutex<Vec<ToolCall>>> = Arc::new(Mutex::new(Vec::new()));
    let runtime = build_runtime_for_test_with_tool_executor(
        test_provider_selection(server.base_url.clone()),
        Box::new(RecordingToolExecutor {
            calls: Arc::clone(&executed),
        }),
    );

    let result = runtime.run_turn(TurnInput {
        message: "先看目录再读配置。".to_string(),
        display_message: None,
        provider_id: None,
        model_id: None,
        reasoning_effort: None,
        workspace_mode: None,
        tool_authorization_mode: None,
        session_id: Some("sync-batch-tool-calls".to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
        plan_execution_approval: None,
    });
    let request_bodies = server.finish();

    assert_eq!(result.phase, "ready");
    assert_eq!(result.assistant_message, final_text);

    // ① 两个调用都执行，且顺序 = 上游顺序。
    let recorded = executed.lock().unwrap().clone();
    assert_eq!(
        recorded.len(),
        2,
        "批内两个调用都必须执行，实际 {recorded:?}"
    );
    assert_eq!(recorded[0].name, "workspace_list_files");
    assert_eq!(recorded[1].name, "workspace_read_file");

    // ② 两份工具结果（逐调用一条 hop 记录 ⇒ 两条顶层工具活动）。
    assert_eq!(result.tool_activities.len(), 2);
    // 工具活动用模型可见名（`List` / `Read`），顺序 = 批内执行顺序。
    assert_eq!(result.tool_activities[0].name, "List");
    assert_eq!(result.tool_activities[1].name, "Read");

    // ③ 批后只有**一次** followup 请求（初始决策 1 + 批后 1 = 2），且形状为 1 assistant + N tool。
    assert_eq!(
        request_bodies.len(),
        2,
        "批执行后应只发一次 followup，实际请求数 {}",
        request_bodies.len()
    );
    let followup: serde_json::Value =
        serde_json::from_str(&request_bodies[1]).expect("followup 请求体必须是 JSON");
    let messages = followup["messages"]
        .as_array()
        .expect("followup 请求体必须带 messages 数组");
    let assistant_with_tool_calls: Vec<&serde_json::Value> = messages
        .iter()
        .filter(|message| {
            message["role"] == "assistant" && message.get("tool_calls").is_some()
        })
        .collect();
    assert_eq!(
        assistant_with_tool_calls.len(),
        1,
        "回填必须恰为一条 assistant 消息（携带全部 tool_calls），实际 messages={messages:?}"
    );
    let assistant_tool_calls = assistant_with_tool_calls[0]["tool_calls"]
        .as_array()
        .expect("assistant 消息的 tool_calls 必须是数组");
    assert_eq!(
        assistant_tool_calls.len(),
        2,
        "assistant 消息必须携带批内全部 2 个 tool_calls，实际 {assistant_tool_calls:?}"
    );
    let tool_messages: Vec<&serde_json::Value> = messages
        .iter()
        .filter(|message| message["role"] == "tool")
        .collect();
    assert_eq!(
        tool_messages.len(),
        2,
        "N 个结果必须各占一条 role=tool 消息，实际 messages={messages:?}"
    );
    assert_eq!(tool_messages[0]["tool_call_id"], "call_workspace_list_files");
    assert_eq!(tool_messages[1]["tool_call_id"], "call_workspace_read_file");
}

/// D3a ③：批内出现 Ask（`ToolKind::Interactive`）⇒ 执行到 Ask 即悬挂，**剩余调用不执行**。
#[test]
fn run_turn_stops_batch_at_interactive_ask_and_leaves_remaining_calls_unexecuted() {
    let server = MockHttpServer::start(vec![json_response(decision_tool_calls_batch(&[
        ("workspace_list_files", json!({"path": ".", "limit": 40})),
        ("ask_user", json!({"question": "继续吗？"})),
        ("workspace_read_file", json!({"path": "tauri.conf.json"})),
    ]))]);
    let executed: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let runtime = build_runtime_for_test_with_tool_executor(
        test_provider_selection(server.base_url.clone()),
        Box::new(AskPendingToolExecutor {
            executed: Arc::clone(&executed),
        }),
    );

    let result = runtime.run_turn(TurnInput {
        message: "先看目录，再问我，最后读配置。".to_string(),
        display_message: None,
        provider_id: None,
        model_id: None,
        reasoning_effort: None,
        workspace_mode: None,
        tool_authorization_mode: None,
        session_id: Some("sync-batch-ask-terminator".to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
        plan_execution_approval: None,
    });
    let request_bodies = server.finish();

    // Ask 之后的第三个调用**不得**执行：执行次数恰为 2（List + ask_user）。
    let recorded = executed.lock().unwrap().clone();
    assert_eq!(
        recorded,
        vec!["workspace_list_files".to_string(), "ask_user".to_string()],
        "批必须在 Ask 处终止，剩余调用不得执行"
    );
    assert!(
        !recorded.iter().any(|name| name == "workspace_read_file"),
        "Ask 之后的调用绝不允许执行，实际 {recorded:?}"
    );

    // 挂起（既非完成也非失败），且没有发出 followup 请求。
    assert_eq!(result.phase, SUSPENDED_TURN_PHASE);
    assert!(result
        .fallback_reason
        .as_deref()
        .is_some_and(|reason| reason.contains("control_outcome_pending")));
    assert_eq!(
        request_bodies.len(),
        1,
        "Ask 挂起后不得再发 followup，实际请求数 {}",
        request_bodies.len()
    );
}

// ─────────── D3b（P1-1 / B018）：**流式**主路径的**批量**工具执行 ───────────

/// 流式决策一次携带**多个** `tool_calls`（同一 SSE delta 的数组、`index` 保序）。
///
/// 旧行为：SSE 累积器只保留 `index == 0` 的调用（其余静默丢弃）⇒ 本用例同时锁「解析层不再丢」。
fn sse_decision_tool_calls_batch(calls: &[(&str, serde_json::Value)]) -> MockHttpResponse {
    let tool_calls: Vec<serde_json::Value> = calls
        .iter()
        .enumerate()
        .map(|(index, (name, arguments))| {
            json!({
                "index": index,
                "id": format!("call_{}", name),
                "type": "function",
                "function": { "name": name, "arguments": arguments.to_string() }
            })
        })
        .collect();
    sse_response(&[json!({
        "choices": [
            {
                "delta": {
                    "content": "一次执行这些工具。",
                    "reasoning_content": "需要一次执行多个工具调用。",
                    "tool_calls": tool_calls
                }
            }
        ]
    })])
}

/// D3b ①：流式决策一次返回 2 个 `tool_calls` ⇒ **两个工具都执行**（两份工具结果、两条 hop），
/// 且批后**一次** followup 的请求体是「1 条 assistant（含全部 tool_calls）+ 2 条 `role=tool`」。
#[test]
fn start_turn_stream_executes_batch_of_tool_calls_and_backfills_one_assistant_plus_n_tool_messages() {
    let final_text = "流式批内两个工具都已执行完成。";
    let server = MockHttpServer::start(vec![
        sse_decision_tool_calls_batch(&[
            ("workspace_list_files", json!({"path": ".", "limit": 40})),
            ("workspace_read_file", json!({"path": "tauri.conf.json"})),
        ]),
        // N>1 的批后 followup 走 D2 批量入口（同步 post 后回放，非真增量）⇒ 该响应是**非流式** JSON。
        json_completion(final_text),
    ]);
    let executed: Arc<Mutex<Vec<ToolCall>>> = Arc::new(Mutex::new(Vec::new()));
    let mut runtime = build_runtime_for_test_with_tool_executor(
        test_provider_selection(server.base_url.clone()),
        Box::new(RecordingToolExecutor {
            calls: Arc::clone(&executed),
        }),
    );
    let sink = RecordingTurnEventSink::new();

    runtime.start_turn_stream(
        &sink,
        "turn-stream-batch".to_string(),
        TurnInput {
            message: "先看目录再读配置。".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            workspace_mode: None,
            tool_authorization_mode: None,
            session_id: Some("stream-batch-tool-calls".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
            plan_execution_approval: None,
        },
    );
    let request_bodies = server.finish();

    // ① 两个调用都执行，且顺序 = 上游顺序。
    let recorded = executed.lock().unwrap().clone();
    assert_eq!(
        recorded.len(),
        2,
        "批内两个调用都必须执行，实际 {recorded:?}"
    );
    assert_eq!(recorded[0].name, "workspace_list_files");
    assert_eq!(recorded[1].name, "workspace_read_file");

    // ② 两份工具结果 ⇒ 两条顶层工具活动；逐调用一条 hop/工具事件（per-call 语义不放宽）。
    {
        let events = sink.events.borrow();
        let completed = events
            .iter()
            .find_map(|(name, payload)| (name == "turn:completed").then_some(payload.clone()))
            .expect("流式批执行必须收口到 turn:completed");
        let activities = completed
            .tool_activities
            .as_ref()
            .expect("turn:completed 必须携带工具活动");
        assert_eq!(activities.len(), 2, "两份工具结果 ⇒ 两条工具活动");
        assert_eq!(activities[0].name, "List");
        assert_eq!(activities[1].name, "Read");
        let per_call_tool_ends = events
            .iter()
            .filter(|(name, payload)| {
                name == "turn:tool" && payload.phase.as_deref() == Some("tool_result_integrating")
            })
            .count();
        assert_eq!(
            per_call_tool_ends, 2,
            "逐调用一条 hop/工具事件（批不吞掉第二个调用的事件）"
        );
    }

    // ③ 批后只有**一次** followup（初始决策 1 + 批后 1 = 2），形状为 1 assistant + N tool。
    assert_eq!(
        request_bodies.len(),
        2,
        "批执行后应只发一次 followup，实际请求数 {}",
        request_bodies.len()
    );
    let followup: serde_json::Value =
        serde_json::from_str(&request_bodies[1]).expect("followup 请求体必须是 JSON");
    let messages = followup["messages"]
        .as_array()
        .expect("followup 请求体必须带 messages 数组");
    let assistant_with_tool_calls: Vec<&serde_json::Value> = messages
        .iter()
        .filter(|message| message["role"] == "assistant" && message.get("tool_calls").is_some())
        .collect();
    assert_eq!(
        assistant_with_tool_calls.len(),
        1,
        "回填必须恰为一条 assistant 消息（携带全部 tool_calls），实际 messages={messages:?}"
    );
    let assistant_tool_calls = assistant_with_tool_calls[0]["tool_calls"]
        .as_array()
        .expect("assistant 消息的 tool_calls 必须是数组");
    assert_eq!(
        assistant_tool_calls.len(),
        2,
        "assistant 消息必须携带批内全部 2 个 tool_calls，实际 {assistant_tool_calls:?}"
    );
    let tool_messages: Vec<&serde_json::Value> = messages
        .iter()
        .filter(|message| message["role"] == "tool")
        .collect();
    assert_eq!(
        tool_messages.len(),
        2,
        "N 个结果必须各占一条 role=tool 消息，实际 messages={messages:?}"
    );
    assert_eq!(tool_messages[0]["tool_call_id"], "call_workspace_list_files");
    assert_eq!(tool_messages[1]["tool_call_id"], "call_workspace_read_file");
}

/// D3b ③：流式批内出现 Ask（`ToolKind::Interactive`）⇒ 执行到 Ask 即悬挂，**剩余调用不执行**。
#[test]
fn start_turn_stream_stops_batch_at_interactive_ask_and_leaves_remaining_calls_unexecuted() {
    let server = MockHttpServer::start(vec![sse_decision_tool_calls_batch(&[
        ("workspace_list_files", json!({"path": ".", "limit": 40})),
        ("ask_user", json!({"question": "继续吗？"})),
        ("workspace_read_file", json!({"path": "tauri.conf.json"})),
    ])]);
    let executed: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let mut runtime = build_runtime_for_test_with_tool_executor(
        test_provider_selection(server.base_url.clone()),
        Box::new(AskPendingToolExecutor {
            executed: Arc::clone(&executed),
        }),
    );
    let sink = RecordingTurnEventSink::new();

    runtime.start_turn_stream(
        &sink,
        "turn-stream-batch-ask".to_string(),
        TurnInput {
            message: "先看目录，再问我，最后读配置。".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            workspace_mode: None,
            tool_authorization_mode: None,
            session_id: Some("stream-batch-ask-terminator".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
            plan_execution_approval: None,
        },
    );
    let request_bodies = server.finish();

    // Ask 之后的第三个调用**不得**执行：执行次数恰为 2（List + ask_user）。
    let recorded = executed.lock().unwrap().clone();
    assert_eq!(
        recorded,
        vec!["workspace_list_files".to_string(), "ask_user".to_string()],
        "批必须在 Ask 处终止，剩余调用不得执行"
    );
    assert!(
        !recorded.iter().any(|name| name == "workspace_read_file"),
        "Ask 之后的调用绝不允许执行，实际 {recorded:?}"
    );

    // 挂起（既非完成也非失败），且没有发出 followup 请求。
    {
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
            "流式批到 Ask 必须悬挂，实际事件 {:?}",
            event_names()
        );
        assert!(
            !events.iter().any(|(name, _)| name == "turn:completed"),
            "Ask 挂起后不得 emit turn:completed：{:?}",
            event_names()
        );
        assert!(
            !events.iter().any(|(name, _)| name == "turn:failed"),
            "Ask 挂起后不得 emit turn:failed：{:?}",
            event_names()
        );
    }
    assert_eq!(
        request_bodies.len(),
        1,
        "Ask 挂起后不得再发 followup，实际请求数 {}",
        request_bodies.len()
    );
}

// ─────────── D4（W28b）：批内执行段的**并发接线**（`PONY_AGENT_PARALLEL_TOOLS`，默认关） ───────────

const PARALLEL_PROBE_DELAY_MS: u64 = 60;

/// 用例级开关覆写守卫：panic 也能还原（并行跑的其他用例不受影响）。
struct ParallelToolsFlag(Option<bool>);

impl Drop for ParallelToolsFlag {
    fn drop(&mut self) {
        set_parallel_tools_override(self.0);
    }
}

fn with_parallel_tools(enabled: bool) -> ParallelToolsFlag {
    ParallelToolsFlag(set_parallel_tools_override(Some(enabled)))
}

/// 并发探针执行器：记录每次调用**进入时**的在途并发数，并累计实测峰值。
///
/// - `entries`：`(工具名, 进入时在途并发数)`，按真实进入顺序（并发下非确定，只作并发证据）；
/// - `peak`：实测峰值并发 —— `>= 2` 才证明并发**确实发生**（不是「两个都执行了」）；
/// - `delay`：制造重叠窗口；若执行仍是串行的，`peak` 恒为 1；
/// - `ask_user` 复用 D3a 的挂起标记形态（`control_outcome_pending`），其余调用返回 ok。
struct ConcurrencyProbeToolExecutor {
    entries: Arc<Mutex<Vec<(String, usize)>>>,
    active: Arc<std::sync::atomic::AtomicUsize>,
    peak: Arc<std::sync::atomic::AtomicUsize>,
    delay: std::time::Duration,
}

impl ConcurrencyProbeToolExecutor {
    fn new(delay_ms: u64) -> Self {
        Self {
            entries: Arc::new(Mutex::new(Vec::new())),
            active: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            peak: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            delay: std::time::Duration::from_millis(delay_ms),
        }
    }

    fn executed_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .entries
            .lock()
            .unwrap()
            .iter()
            .map(|(name, _)| name.clone())
            .collect();
        names.sort();
        names
    }

    /// 该工具每次调用「进入时」的在途并发数（1 = 独占执行）。
    fn entry_concurrency_of(&self, name: &str) -> Vec<usize> {
        self.entries
            .lock()
            .unwrap()
            .iter()
            .filter(|(executed, _)| executed == name)
            .map(|(_, active_at_entry)| *active_at_entry)
            .collect()
    }

    fn peak(&self) -> usize {
        self.peak.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl crate::agent::tools::ToolExecutor for ConcurrencyProbeToolExecutor {
    fn execute(&self, call: &ToolCall) -> crate::agent::tools::ToolResult {
        let active_at_entry =
            self.active.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        self.peak
            .fetch_max(active_at_entry, std::sync::atomic::Ordering::SeqCst);
        self.entries
            .lock()
            .unwrap()
            .push((call.name.clone(), active_at_entry));
        std::thread::sleep(self.delay);
        self.active.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);

        let is_ask = crate::agent::tools::canonical_tool_name(&call.name)
            .map(|canonical| canonical == "ask_user")
            .unwrap_or(false);
        if is_ask {
            return crate::agent::tools::ToolResult {
                tool_name: call.name.clone(),
                status: "error".to_string(),
                output: json!({
                    "ok": false,
                    "error": {
                        "code": "control_outcome_pending",
                        "kind": "control_outcome_pending",
                        "message": "Ask 等待宿主应答。"
                    }
                })
                .to_string(),
                duration_ms: 0,
            };
        }
        crate::agent::tools::ToolResult {
            tool_name: call.name.clone(),
            status: "ok".to_string(),
            // 结果自带「被执行的是哪个调用」的证据：并发下完成顺序不定，回填后仍须与调用一一对应。
            output: json!({ "executed": call.name, "arguments": call.arguments }).to_string(),
            duration_ms: 1,
        }
    }
}

/// 从 followup 请求体里取出 `role=tool` 消息的 `tool_call_id` 序列（= 批内回填顺序的唯一证据）。
fn followup_tool_call_ids(request_body: &str) -> Vec<String> {
    let followup: serde_json::Value =
        serde_json::from_str(request_body).expect("followup 请求体必须是 JSON");
    followup["messages"]
        .as_array()
        .expect("followup 请求体必须带 messages 数组")
        .iter()
        .filter(|message| message["role"] == "tool")
        .map(|message| {
            message["tool_call_id"]
                .as_str()
                .expect("tool 消息必须带 tool_call_id")
                .to_string()
        })
        .collect()
}

/// 从 followup 请求体里取出 `(tool_call_id, content)` 序列：既验顺序，也验「结果与调用一一对应」。
fn followup_tool_messages(request_body: &str) -> Vec<(String, String)> {
    let followup: serde_json::Value =
        serde_json::from_str(request_body).expect("followup 请求体必须是 JSON");
    followup["messages"]
        .as_array()
        .expect("followup 请求体必须带 messages 数组")
        .iter()
        .filter(|message| message["role"] == "tool")
        .map(|message| {
            (
                message["tool_call_id"]
                    .as_str()
                    .expect("tool 消息必须带 tool_call_id")
                    .to_string(),
                message["content"]
                    .as_str()
                    .expect("tool 消息必须带 content")
                    .to_string(),
            )
        })
        .collect()
}

fn parallel_batch_turn_input(message: &str, session_id: &str) -> TurnInput {
    TurnInput {
        message: message.to_string(),
        display_message: None,
        provider_id: None,
        model_id: None,
        reasoning_effort: None,
        workspace_mode: None,
        tool_authorization_mode: None,
        session_id: Some(session_id.to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
        plan_execution_approval: None,
    }
}

/// D4 ①（同步路径）：开关**开** + 2 个并发安全调用 ⇒ 都执行、回填**保序**，且并发确实发生。
#[test]
fn run_turn_parallel_tools_executes_concurrent_safe_segment_concurrently() {
    let _flag = with_parallel_tools(true);
    let final_text = "两个工具都已执行完成。";
    let server = MockHttpServer::start(vec![
        json_response(decision_tool_calls_batch(&[
            ("workspace_list_files", json!({"path": ".", "limit": 40})),
            ("workspace_read_file", json!({"path": "tauri.conf.json"})),
        ])),
        json_completion(final_text),
    ]);
    let probe = ConcurrencyProbeToolExecutor::new(PARALLEL_PROBE_DELAY_MS);
    let observed = ConcurrencyProbeToolExecutor {
        entries: Arc::clone(&probe.entries),
        active: Arc::clone(&probe.active),
        peak: Arc::clone(&probe.peak),
        delay: probe.delay,
    };
    let runtime = build_runtime_for_test_with_tool_executor(
        test_provider_selection(server.base_url.clone()),
        Box::new(observed),
    );

    let result = runtime.run_turn(parallel_batch_turn_input(
        "先看目录再读配置。",
        "sync-parallel-tools",
    ));
    let request_bodies = server.finish();

    assert_eq!(result.phase, "ready");
    assert_eq!(result.assistant_message, final_text);
    // 两个都执行。
    assert_eq!(
        probe.executed_names(),
        vec![
            "workspace_list_files".to_string(),
            "workspace_read_file".to_string()
        ],
        "批内两个安全调用都必须执行"
    );
    // 并发**确实发生**（不是只断「都执行了」）。
    assert!(
        probe.peak() >= 2,
        "并发段必须真的重叠执行，实测峰值并发 {}",
        probe.peak()
    );
    // 回填保序：tool 消息顺序 = 上游顺序（与完成先后无关）。
    assert_eq!(request_bodies.len(), 2, "批后只发一次 followup");
    assert_eq!(
        followup_tool_call_ids(&request_bodies[1]),
        vec![
            "call_workspace_list_files".to_string(),
            "call_workspace_read_file".to_string()
        ],
        "回填顺序必须与批内原序一致"
    );
    // 结果与调用**一一对应**（并发完成顺序不同，但 results[i] 必须属于 calls[i]）。
    let tool_messages = followup_tool_messages(&request_bodies[1]);
    assert!(
        tool_messages[0].1.contains("workspace_list_files"),
        "第 1 条结果必须来自第 1 个调用，实际 {}",
        tool_messages[0].1
    );
    assert!(
        tool_messages[1].1.contains("workspace_read_file"),
        "第 2 条结果必须来自第 2 个调用，实际 {}",
        tool_messages[1].1
    );
    // per-call 语义不丢：两条顶级工具活动。
    assert_eq!(result.tool_activities.len(), 2);
    assert_eq!(result.tool_activities[0].name, "List");
    assert_eq!(result.tool_activities[1].name, "Read");
}

/// D4 ②（同步路径）：开关**开** + 混入 `Run`（非并发安全）⇒ 并发段只含安全子集，
/// `Run` 独占执行（进入时在途并发恒为 1），回填仍保序。
#[test]
fn run_turn_parallel_tools_keeps_run_out_of_parallel_segment() {
    let _flag = with_parallel_tools(true);
    let server = MockHttpServer::start(vec![
        json_response(decision_tool_calls_batch(&[
            ("workspace_list_files", json!({"path": ".", "limit": 40})),
            ("workspace_read_file", json!({"path": "tauri.conf.json"})),
            ("workspace_run_command", json!({"command": "echo hi"})),
        ])),
        json_completion("三个工具都已执行完成。"),
    ]);
    let probe = ConcurrencyProbeToolExecutor::new(PARALLEL_PROBE_DELAY_MS);
    let observed = ConcurrencyProbeToolExecutor {
        entries: Arc::clone(&probe.entries),
        active: Arc::clone(&probe.active),
        peak: Arc::clone(&probe.peak),
        delay: probe.delay,
    };
    let runtime = build_runtime_for_test_with_tool_executor(
        test_provider_selection(server.base_url.clone()),
        Box::new(observed),
    );

    let result = runtime.run_turn(parallel_batch_turn_input(
        "读两个文件，再跑一条命令。",
        "sync-parallel-tools-mixed",
    ));
    let request_bodies = server.finish();

    assert_eq!(result.phase, "ready");
    assert_eq!(
        probe.executed_names(),
        vec![
            "workspace_list_files".to_string(),
            "workspace_read_file".to_string(),
            "workspace_run_command".to_string()
        ]
    );
    // 并发只发生在安全子集（List + Read）：峰值恰为 2。
    assert_eq!(
        probe.peak(),
        2,
        "并发段只应包含两个安全调用，实测峰值 {}",
        probe.peak()
    );
    // `Run` 绝不与任何调用重叠：进入时在途并发恒为 1。
    assert_eq!(
        probe.entry_concurrency_of("workspace_run_command"),
        vec![1],
        "Run 必须在并发段全部结束后独占执行"
    );
    assert_eq!(
        followup_tool_call_ids(&request_bodies[1]),
        vec![
            "call_workspace_list_files".to_string(),
            "call_workspace_read_file".to_string(),
            "call_workspace_run_command".to_string()
        ],
        "回填顺序必须与批内原序一致"
    );
    assert_eq!(result.tool_activities.len(), 3);
}

/// D4 ③（同步路径）：开关**关**（默认）⇒ 逐字现状：逐调用顺序执行，永不重叠。
#[test]
fn run_turn_parallel_tools_off_keeps_serial_execution() {
    let _flag = with_parallel_tools(false);
    let server = MockHttpServer::start(vec![
        json_response(decision_tool_calls_batch(&[
            ("workspace_list_files", json!({"path": ".", "limit": 40})),
            ("workspace_read_file", json!({"path": "tauri.conf.json"})),
        ])),
        json_completion("两个工具都已执行完成。"),
    ]);
    let probe = ConcurrencyProbeToolExecutor::new(PARALLEL_PROBE_DELAY_MS);
    let observed = ConcurrencyProbeToolExecutor {
        entries: Arc::clone(&probe.entries),
        active: Arc::clone(&probe.active),
        peak: Arc::clone(&probe.peak),
        delay: probe.delay,
    };
    let runtime = build_runtime_for_test_with_tool_executor(
        test_provider_selection(server.base_url.clone()),
        Box::new(observed),
    );

    let result = runtime.run_turn(parallel_batch_turn_input(
        "先看目录再读配置。",
        "sync-parallel-tools-off",
    ));
    let request_bodies = server.finish();

    assert_eq!(result.phase, "ready");
    assert_eq!(
        probe.executed_names(),
        vec![
            "workspace_list_files".to_string(),
            "workspace_read_file".to_string()
        ]
    );
    assert_eq!(
        probe.peak(),
        1,
        "开关关时必须逐调用顺序执行（实测峰值 {}）",
        probe.peak()
    );
    assert_eq!(
        followup_tool_call_ids(&request_bodies[1]),
        vec![
            "call_workspace_list_files".to_string(),
            "call_workspace_read_file".to_string()
        ]
    );
    assert_eq!(result.tool_activities.len(), 2);
}

/// D4 ①（流式主路径）：开关**开** + 2 个并发安全调用 ⇒ 都执行、并发确实发生、回填保序。
#[test]
fn start_turn_stream_parallel_tools_executes_concurrent_safe_segment_concurrently() {
    let _flag = with_parallel_tools(true);
    let final_text = "流式批内两个工具都已执行完成。";
    let server = MockHttpServer::start(vec![
        sse_decision_tool_calls_batch(&[
            ("workspace_list_files", json!({"path": ".", "limit": 40})),
            ("workspace_read_file", json!({"path": "tauri.conf.json"})),
        ]),
        json_completion(final_text),
    ]);
    let probe = ConcurrencyProbeToolExecutor::new(PARALLEL_PROBE_DELAY_MS);
    let observed = ConcurrencyProbeToolExecutor {
        entries: Arc::clone(&probe.entries),
        active: Arc::clone(&probe.active),
        peak: Arc::clone(&probe.peak),
        delay: probe.delay,
    };
    let mut runtime = build_runtime_for_test_with_tool_executor(
        test_provider_selection(server.base_url.clone()),
        Box::new(observed),
    );
    let sink = RecordingTurnEventSink::new();

    runtime.start_turn_stream(
        &sink,
        "turn-stream-parallel".to_string(),
        parallel_batch_turn_input("先看目录再读配置。", "stream-parallel-tools"),
    );
    let request_bodies = server.finish();

    assert_eq!(
        probe.executed_names(),
        vec![
            "workspace_list_files".to_string(),
            "workspace_read_file".to_string()
        ]
    );
    assert!(
        probe.peak() >= 2,
        "流式路径的并发段也必须真的重叠执行，实测峰值 {}",
        probe.peak()
    );
    assert_eq!(request_bodies.len(), 2, "批后只发一次 followup");
    assert_eq!(
        followup_tool_call_ids(&request_bodies[1]),
        vec![
            "call_workspace_list_files".to_string(),
            "call_workspace_read_file".to_string()
        ]
    );
    // 结果与调用一一对应（并发保序回填）。
    let tool_messages = followup_tool_messages(&request_bodies[1]);
    assert!(
        tool_messages[0].1.contains("workspace_list_files")
            && tool_messages[1].1.contains("workspace_read_file"),
        "并发段的结果必须按原序对回各自调用，实际 {tool_messages:?}"
    );
    // per-call 语义不丢：逐调用一条工具事件（批不吞掉第二个调用的事件）。
    let events = sink.events.borrow();
    let per_call_tool_ends = events
        .iter()
        .filter(|(name, payload)| {
            name == "turn:tool" && payload.phase.as_deref() == Some("tool_result_integrating")
        })
        .count();
    assert_eq!(per_call_tool_ends, 2);
    let completed = events
        .iter()
        .find_map(|(name, payload)| (name == "turn:completed").then_some(payload.clone()))
        .expect("流式并发批执行必须收口到 turn:completed");
    assert_eq!(
        completed
            .tool_activities
            .as_ref()
            .map(|activities| activities.len()),
        Some(2)
    );
}

/// D4 ④（流式主路径）：开关**开** + 批内含 Ask ⇒ Ask **不进并发段**（独占执行、在途并发恒 1），
/// 且仍是批终止符（其后调用不执行、本轮悬挂、不发 followup）。
#[test]
fn start_turn_stream_parallel_tools_keeps_ask_out_of_parallel_segment_and_still_terminates_batch() {
    let _flag = with_parallel_tools(true);
    let server = MockHttpServer::start(vec![sse_decision_tool_calls_batch(&[
        ("workspace_list_files", json!({"path": ".", "limit": 40})),
        ("workspace_read_file", json!({"path": "tauri.conf.json"})),
        ("ask_user", json!({"question": "继续吗？"})),
    ])]);
    let probe = ConcurrencyProbeToolExecutor::new(PARALLEL_PROBE_DELAY_MS);
    let observed = ConcurrencyProbeToolExecutor {
        entries: Arc::clone(&probe.entries),
        active: Arc::clone(&probe.active),
        peak: Arc::clone(&probe.peak),
        delay: probe.delay,
    };
    let mut runtime = build_runtime_for_test_with_tool_executor(
        test_provider_selection(server.base_url.clone()),
        Box::new(observed),
    );
    let sink = RecordingTurnEventSink::new();

    runtime.start_turn_stream(
        &sink,
        "turn-stream-parallel-ask".to_string(),
        parallel_batch_turn_input(
            "先看目录、再读配置，最后问我。",
            "stream-parallel-ask",
        ),
    );
    let request_bodies = server.finish();

    assert_eq!(
        probe.executed_names(),
        vec![
            "ask_user".to_string(),
            "workspace_list_files".to_string(),
            "workspace_read_file".to_string()
        ]
    );
    // 安全段确实并发（List + Read），Ask 独占。
    assert!(
        probe.peak() >= 2,
        "安全段必须并发执行，实测峰值 {}",
        probe.peak()
    );
    assert_eq!(
        probe.entry_concurrency_of("ask_user"),
        vec![1],
        "交互类工具（批终止符）绝不进并发段"
    );
    let events = sink.events.borrow();
    assert!(
        events.iter().any(|(name, payload)| {
            name == "turn:suspended" && payload.phase.as_deref() == Some(SUSPENDED_TURN_PHASE)
        }),
        "批到 Ask 必须悬挂"
    );
    assert!(!events.iter().any(|(name, _)| name == "turn:completed"));
    assert!(!events.iter().any(|(name, _)| name == "turn:failed"));
    drop(events);
    assert_eq!(
        request_bodies.len(),
        1,
        "Ask 挂起后不得再发 followup，实际请求数 {}",
        request_bodies.len()
    );
}

// ── W3b：共享 SessionStore（宿主 ↔ 队友引擎同一 Arc）──────────────────────────────

/// 契约 W3b：`AgentRuntimeBuilder::shared_session_store` 注入的 Arc 必须**原样**成为
/// runtime 的会话句柄（`Arc::ptr_eq` 同一实例，不是复制/重新包一层）。
///
/// 这是「队友会话能被宿主 `list_sessions()` 看到」的机制前提：宿主 control plane 通过
/// `runtime.sessions_handle()` 持有同一把锁；`HostTurnEngine` 建栈时复用同一 Arc，队友 turn
/// 的落库才写进宿主读取的那张表（此前的独立 store 永远不可加载，`/agents` Enter 只能提示）。
#[test]
fn builder_shared_session_store_is_the_runtime_sessions_handle() {
    crate::agent::runtime_helper::TestRuntimeGuard::leak();
    let shared = Arc::new(std::sync::RwLock::new(SessionStore::memory_only()));

    let runtime = AgentRuntimeBuilder::new()
        .shared_session_store(Arc::clone(&shared))
        .build();

    assert!(
        Arc::ptr_eq(&runtime.sessions_handle(), &shared),
        "W3b：builder 注入的共享 SessionStore 必须原样成为 runtime 会话句柄（ptr_eq 同一 Arc）"
    );
}

/// 契约 W3b 端到端：`HostTurnEngine` 复用宿主共享 `SessionStore` 后，队友 turn 落库的会话必须
/// 出现在**宿主 control plane** 的 `list_sessions()` 里 —— 即 `/agents` 的 Enter 从
/// “提示不可加载”变为真正可 `hydrate` 的机制证据。
///
/// 验证边界：provider 走本地 mock（不联网）；engine 直接实例化（不经全局槽位，避免与本进程
/// 其它用例对 `OnceLock` 首写者的顺序耦合）。
#[test]
fn w3b_shared_store_makes_teammate_session_visible_to_host_control_plane() {
    crate::agent::runtime_helper::TestRuntimeGuard::leak();
    let server = MockHttpServer::start(vec![json_completion("teammate ready")]);
    let shared = Arc::new(std::sync::RwLock::new(SessionStore::memory_only()));

    // 1) 宿主栈注入共享 store ⇒ control plane 与 runtime 共用同一 `Arc`（`sessions_handle`）。
    let host_runtime = AgentRuntimeBuilder::new()
        .shared_session_store(Arc::clone(&shared))
        .build();
    let control_plane = HostControlPlaneBuilder::new().runtime(host_runtime).build();

    // 2) 队友引擎复用同一 store 跑一次真实 turn（本地 mock provider）。
    let workspace = temp_workspace_dir("w3b-shared-session-visibility");
    let engine = crate::agent::subagent_runtime::HostTurnEngine::new()
        .with_workspace_root(Some(workspace.clone()))
        .with_shared_sessions(Some(Arc::clone(&shared)));
    let selection = test_chat_provider_selection(server.base_url.clone());
    let outcome = crate::agent::subagent_runtime::SubagentTurnEngine::run_turn_with_selection(
        &engine,
        "reply with ok",
        10_000,
        Some(workspace.as_path()),
        Some(&selection),
    )
    .expect("W3b：共享 store 下队友 turn 必须成功");
    assert!(
        !outcome.session_id.trim().is_empty(),
        "W3b：队友 turn 必须返回非空 session_id"
    );

    // 3) 宿主 control plane 能看到该队友会话（同一 Arc ⇒ 同一张会话表）。
    let session_ids: Vec<String> = control_plane
        .list_sessions()
        .into_iter()
        .map(|session| session.conversation_id)
        .collect();
    assert!(
        session_ids.iter().any(|id| id == &outcome.session_id),
        "W3b：宿主 control plane 必须能看到队友会话 {}，实际 = {session_ids:?}",
        outcome.session_id
    );

    server.finish();
}
