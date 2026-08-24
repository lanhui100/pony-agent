use crate::agent::capability_bridge::SkillDescriptor;
use crate::agent::hooks::HookTraceRecord;
use crate::agent::provider::{
    BuildContextObservation, ProviderClient, ProviderDecision, ProviderManager, ProviderRequest,
    ProviderStreamChunk, TokenUsage,
};
use crate::agent::session::TraceTimelineEntry;
use crate::agent::telemetry::{ProviderCallCacheRecord, TurnToolActivity, TurnTraceStep};
use crate::agent::tools::{ToolCall, ToolDefinition, ToolResult};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use super::context::RetrievedContextState;
use super::runtime::{TurnResult, TurnStreamEvent};

pub trait TurnEventSink {
    fn emit(&self, name: &str, payload: TurnStreamEvent);
    /// 事件溯源（PA-091）：将 turn 内过程事实入事件缓冲（默认空实现——
    /// 生产路径经全局注册表持久化；测试 sink 可覆写为收集）。
    /// 持久化失败 contained（只记日志不阻断流）；构造失败由调用方 fail loud。
    fn persist(&self, _event: crate::agent::turn_event::TurnEvent) {}
}

/// PA-095：同步入口（run_turn）的事件通道 sink——emit 空实现（同步 API 无流式
/// 推送），事件持久化经全局注册表进行（与 sink 无关），使同步入口产生与
/// streaming 入口等价的事件流。
pub struct NoopTurnEventSink;

impl TurnEventSink for NoopTurnEventSink {
    fn emit(&self, _name: &str, _payload: TurnStreamEvent) {}
}

/// PA-091：全局事件持久化注册表（OnceLock 模式，与 turn_event_sequence_registry 一致）。
/// 生产路径由 HostControlPlane 初始化时注册（持有 sessions_rwlock 的 Arc）；
/// 测试可注册 mock / 清空。签名：(session_id, turn_id, event, is_terminal)。
pub type EventPersistFn = dyn Fn(&str, &str, crate::agent::turn_event::TurnEvent, bool) + Send + Sync;

static EVENT_PERSIST_REGISTRY: OnceLock<Mutex<Option<Arc<EventPersistFn>>>> = OnceLock::new();

fn event_persist_registry() -> &'static Mutex<Option<Arc<EventPersistFn>>> {
    EVENT_PERSIST_REGISTRY.get_or_init(|| Mutex::new(None))
}

/// 注册生产事件持久化通道（幂等：重复注册覆盖）。
pub fn register_event_persist(f: Arc<EventPersistFn>) {
    let mut slot = event_persist_registry()
        .lock()
        .expect("event persist registry lock poisoned");
    // PA-095 #3（实施后审核 P2）：覆盖非同源通道时告警——多控制面并存时
    // 后建者的单槽注册会劫持先建者未绑定会话的事件流（结构性已知问题，
    // 登记于任务卡；测试侧应使用会话绑定路由）。
    if let Some(previous) = slot.as_ref() {
        if !Arc::ptr_eq(previous, &f) {
            eprintln!(
                "[pony-agent][runtime] event persist channel overwritten by a new control plane (multi-control-plane setups must route per session)"
            );
        }
    }
    *slot = Some(f);
}

/// 清空事件持久化通道（测试用）。
pub fn clear_event_persist() {
    let mut slot = event_persist_registry()
        .lock()
        .expect("event persist registry lock poisoned");
    *slot = None;
}

/// PA-093：非 turn 生命周期事实（checkpoint/checkout、fork/created）经全局
/// 注册表落盘。turn_id 用语义化字面量（事件无 turn 归属，列非空）。
/// 未注册通道（测试/老路径）时静默跳过——与 turn 事件路径的 contained 语义一致。
/// PA-095 #3：is_terminal=false——history 命令在持有 sessions 写锁时调用本函数，
/// 终态内联 flush 会同线程重入写锁死锁（fork 实测挂死）；改为仅入缓冲，由
/// history 命令包装释放写锁后的 flush_session_buffered_events 提交（或并入下一条
/// turn 终态批）。内存后端 flush 失败 contained（与既有语义一致）。
pub fn emit_global_event(
    session_id: &str,
    turn_id: &str,
    event: crate::agent::turn_event::TurnEvent,
) {
    if let Some(persist) = resolve_event_persist(session_id) {
        persist(session_id, turn_id, event, false);
    }
}

/// 当前注册的持久化通道（None = 未注册，事件不落盘）。
fn current_event_persist() -> Option<Arc<EventPersistFn>> {
    event_persist_registry()
        .lock()
        .ok()
        .and_then(|slot| slot.clone())
}

/// PA-095：分发事件到生产通道与测试多播 sink。生产单槽可被任意
/// HostControlPlane 构建覆盖（并行测试下互相抢占），测试多播 sink 只能被
/// 注册它的测试清除——依赖全局通道断言的测试必须走多播 sink。
fn dispatch_event_persist(
    session_id: &str,
    turn_id: &str,
    event: crate::agent::turn_event::TurnEvent,
    is_terminal: bool,
) {
    // PA-095 #3（实施后审核 P1）：匿名 turn（session_id=None）归一到默认会话——
    // 其 history/trace 本就落默认会话，事件缓冲键若保留 "" 将无人 flush（永久滞留）。
    let resolved_session_id = if session_id.is_empty() {
        crate::agent::session::DEFAULT_SESSION_ID
    } else {
        session_id
    };
    if let Some(persist) = resolve_event_persist(resolved_session_id) {
        persist(resolved_session_id, turn_id, event.clone(), is_terminal);
    }
    #[cfg(test)]
    for sink in event_persist_test_sinks()
        .lock()
        .expect("event persist test sinks lock poisoned")
        .iter()
    {
        sink(session_id, turn_id, event.clone(), is_terminal);
    }
}

/// PA-095 #3：事件通道解析——会话绑定优先，回退全局默认单槽。会话绑定是
/// "按 session 所有权路由的多通道模型"（任务卡登记的结构性改进）的落地：
/// 并行测试中各场景把自身控制面的通道绑到自己的 session，全局默认槽被其他
/// 测试构建覆盖不再影响本会话的事件落盘；生产路径无绑定时行为不变。
///
/// TODO(PA-095 #3 后续任务)：session 绑定路由目前仅测试构建生效（见下方
/// `#[cfg(test)]` 分支）；生产构建恒走全局默认单槽（last-build-wins）。
/// 生产侧按 session 接线尚未实施，勿因本签名看似接收 session_id 而误以为已接线。
///
/// （参数仅测试构建使用，故带下划线前缀以通过非测试编译的 unused 检查。）
fn resolve_event_persist(_session_id: &str) -> Option<Arc<EventPersistFn>> {
    #[cfg(test)]
    if let Some(bound) = session_event_persist_binding(_session_id) {
        return Some(bound);
    }
    current_event_persist()
}

/// PA-095：测试专用多播 sink 注册表（与生产单槽独立）。
#[cfg(test)]
static EVENT_PERSIST_TEST_SINKS: OnceLock<Mutex<Vec<Arc<EventPersistFn>>>> = OnceLock::new();

#[cfg(test)]
fn event_persist_test_sinks() -> &'static Mutex<Vec<Arc<EventPersistFn>>> {
    EVENT_PERSIST_TEST_SINKS.get_or_init(|| Mutex::new(Vec::new()))
}

/// PA-095：注册测试专用多播持久化 sink（不受生产单槽覆盖影响）。
/// 返回守卫——Drop 时仅移除本 sink（并行测试互不清除对方注册）。
#[cfg(test)]
pub fn register_event_persist_test_sink(
    f: Arc<EventPersistFn>,
) -> EventPersistTestSinkGuard {
    let mut sinks = event_persist_test_sinks()
        .lock()
        .expect("event persist test sinks lock poisoned");
    sinks.push(Arc::clone(&f));
    drop(sinks);
    EventPersistTestSinkGuard { sink: f }
}

/// PA-095：测试多播 sink 守卫——Drop 按指针相等移除自身。
#[cfg(test)]
pub struct EventPersistTestSinkGuard {
    sink: Arc<EventPersistFn>,
}

#[cfg(test)]
impl Drop for EventPersistTestSinkGuard {
    fn drop(&mut self) {
        let mut sinks = event_persist_test_sinks()
            .lock()
            .expect("event persist test sinks lock poisoned");
        sinks.retain(|sink| !Arc::ptr_eq(sink, &self.sink));
    }
}

/// PA-095 #3：会话绑定事件持久化通道（测试专用）——session 所有权路由，
/// 优先于全局默认单槽；守卫 Drop 时解除绑定。
#[cfg(test)]
static SESSION_EVENT_PERSIST_BINDINGS: OnceLock<Mutex<HashMap<String, Arc<EventPersistFn>>>> =
    OnceLock::new();

#[cfg(test)]
fn session_event_persist_bindings(
) -> &'static Mutex<HashMap<String, Arc<EventPersistFn>>> {
    SESSION_EVENT_PERSIST_BINDINGS.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(test)]
fn session_event_persist_binding(session_id: &str) -> Option<Arc<EventPersistFn>> {
    session_event_persist_bindings()
        .lock()
        .ok()?
        .get(session_id)
        .cloned()
}

/// PA-095 #3：绑定会话的事件持久化通道（测试专用）；返回守卫，Drop 解绑。
#[cfg(test)]
pub fn bind_event_persist_session(
    session_id: &str,
    f: Arc<EventPersistFn>,
) -> SessionEventPersistBindingGuard {
    session_event_persist_bindings()
        .lock()
        .expect("session event persist bindings lock poisoned")
        .insert(session_id.to_string(), Arc::clone(&f));
    SessionEventPersistBindingGuard {
        session_id: session_id.to_string(),
        bound: f,
    }
}

#[cfg(test)]
pub struct SessionEventPersistBindingGuard {
    session_id: String,
    bound: Arc<EventPersistFn>,
}

#[cfg(test)]
impl Drop for SessionEventPersistBindingGuard {
    fn drop(&mut self) {
        // PA-095 #3（实施后审核 P2）：仅当当前占用者仍是自己时才解绑——
        // 同名 session 的嵌套/并行绑定不得被先结束的守卫误摘。
        if let Ok(mut bindings) = session_event_persist_bindings().lock() {
            if bindings
                .get(&self.session_id)
                .is_some_and(|current| Arc::ptr_eq(current, &self.bound))
            {
                bindings.remove(&self.session_id);
            }
        }
    }
}

/// PA-095 #2：会话级事件缓冲 flush 请求——append_turn 物化前确保该会话的
/// 缓冲事件已提交事件表（commit 先于清空，由注册闭包保证）。生产由
/// HostControlPlaneBuilder 注册（与持久化闭包同源缓冲）；未注册（无控制面 /
/// 测试直连 store）返回 Ok(0)。
pub type EventFlushFn = dyn Fn(&str) -> Result<usize, String> + Send + Sync;

static EVENT_FLUSH_REGISTRY: OnceLock<Mutex<Option<Arc<EventFlushFn>>>> = OnceLock::new();

/// PA-095 #3：会话绑定 flush 通道（测试专用）——与持久化绑定同原理。
#[cfg(test)]
static SESSION_EVENT_FLUSH_BINDINGS: OnceLock<Mutex<HashMap<String, Arc<EventFlushFn>>>> =
    OnceLock::new();

#[cfg(test)]
fn session_event_flush_bindings() -> &'static Mutex<HashMap<String, Arc<EventFlushFn>>> {
    SESSION_EVENT_FLUSH_BINDINGS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// PA-095 #3：绑定会话的 flush 请求通道（测试专用）；返回守卫，Drop 解绑。
#[cfg(test)]
pub fn bind_event_flush_session(
    session_id: &str,
    f: Arc<EventFlushFn>,
) -> SessionEventFlushBindingGuard {
    session_event_flush_bindings()
        .lock()
        .expect("session event flush bindings lock poisoned")
        .insert(session_id.to_string(), Arc::clone(&f));
    SessionEventFlushBindingGuard {
        session_id: session_id.to_string(),
        bound: f,
    }
}

#[cfg(test)]
pub struct SessionEventFlushBindingGuard {
    session_id: String,
    bound: Arc<EventFlushFn>,
}

#[cfg(test)]
impl Drop for SessionEventFlushBindingGuard {
    fn drop(&mut self) {
        // 同 persist 守卫——仅解绑自己（Arc::ptr_eq 校验）。
        if let Ok(mut bindings) = session_event_flush_bindings().lock() {
            if bindings
                .get(&self.session_id)
                .is_some_and(|current| Arc::ptr_eq(current, &self.bound))
            {
                bindings.remove(&self.session_id);
            }
        }
    }
}

fn event_flush_registry() -> &'static Mutex<Option<Arc<EventFlushFn>>> {
    EVENT_FLUSH_REGISTRY.get_or_init(|| Mutex::new(None))
}

/// 注册会话级 flush 请求通道（幂等：重复注册覆盖）。
pub fn register_event_flush(f: Arc<EventFlushFn>) {
    let mut slot = event_flush_registry()
        .lock()
        .expect("event flush registry lock poisoned");
    *slot = Some(f);
}

/// 清空 flush 请求通道（测试用）。
pub fn clear_event_flush() {
    let mut slot = event_flush_registry()
        .lock()
        .expect("event flush registry lock poisoned");
    *slot = None;
}

/// 请求提交某会话的全部缓冲事件；返回提交的事件数（0 = 无缓冲）。
/// PA-095 #3：会话绑定 flush 通道优先（session 所有权路由），回退全局注册。
pub fn flush_session_buffered_events(session_id: &str) -> Result<usize, String> {
    #[cfg(test)]
    if let Some(bound) = session_event_flush_bindings()
        .lock()
        .ok()
        .and_then(|bindings| bindings.get(session_id).cloned())
    {
        return bound(session_id);
    }
    match event_flush_registry().lock().ok().and_then(|slot| {
        slot.as_ref()
            .map(|f| Arc::clone(f) as Arc<EventFlushFn>)
    }) {
        Some(flush) => flush(session_id),
        None => Ok(0),
    }
}

pub struct PreparedTurn {
    pub user_message: String,
    pub display_message: String,
    pub retrieved: RetrievedContextState,
    pub provider: ProviderManager,
    pub tools: Vec<ToolDefinition>,
    pub planner_skills: Vec<SkillDescriptor>,
    pub planning_request: ProviderRequest,
    pub build_context_observation: BuildContextObservation,
}

pub struct PlannedTurn {
    pub first_decision: ProviderDecision,
    pub resolved_tool_call: Option<ToolCall>,
    pub initial_decision_duration_ms: Option<u64>,
    pub planner_hook_trace_records: Vec<HookTraceRecord>,
}

pub struct PersistedTurnOutcome {
    pub session_summary: String,
    pub input_tokens: Option<u64>,
    pub cache_hit_input_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
}

#[derive(Clone)]
pub struct TurnEventEnvelope {
    pub event_id: String,
    pub event_type: String,
    pub event_version: String,
    pub sequence: u64,
    pub emitted_at_ms: u64,
}

pub struct ModelHopTraceContent {
    pub text: String,
    pub reasoning_content: Option<String>,
}

pub struct SyncToolTurnOutcome {
    pub assistant_message: String,
    pub assistant_reasoning_content: Option<String>,
    pub provider_native_transcript: Option<Vec<Value>>,
    pub provider_source: String,
    pub provider_mode: String,
    pub fallback_reason: Option<String>,
    pub token_usage: Option<TokenUsage>,
    pub trace_steps: Vec<TurnTraceStep>,
    pub tool_activities: Vec<TurnToolActivity>,
    pub model_hop_trace_contents: Vec<ModelHopTraceContent>,
    pub hook_trace_records: Vec<HookTraceRecord>,
    pub first_token_latency_ms: Option<u64>,
}

#[derive(Clone)]
pub struct ProviderEventMeta {
    pub requested_name: String,
    pub provider_name: String,
    pub protocol: String,
    pub model: String,
}

pub fn build_failed_turn_result(
    provider_meta: Option<&ProviderEventMeta>,
    user_message: String,
    assistant_message: String,
    trace_steps: Vec<TurnTraceStep>,
    tool_activities: Vec<TurnToolActivity>,
) -> TurnResult {
    build_failed_turn_result_with_hooks(
        provider_meta,
        user_message,
        assistant_message,
        trace_steps,
        tool_activities,
        Vec::new(),
    )
}

pub fn build_failed_turn_result_with_hooks(
    provider_meta: Option<&ProviderEventMeta>,
    user_message: String,
    assistant_message: String,
    trace_steps: Vec<TurnTraceStep>,
    tool_activities: Vec<TurnToolActivity>,
    hook_trace_records: Vec<HookTraceRecord>,
) -> TurnResult {
    TurnResult {
        event_id: None,
        event_type: None,
        event_version: None,
        sequence: None,
        emitted_at_ms: None,
        phase: "failed".to_string(),
        provider_requested_name: provider_meta
            .map(|meta| meta.requested_name.clone())
            .unwrap_or_default(),
        provider_name: provider_meta
            .map(|meta| meta.provider_name.clone())
            .unwrap_or_default(),
        provider_protocol: provider_meta
            .map(|meta| meta.protocol.clone())
            .unwrap_or_default(),
        provider_model: provider_meta
            .map(|meta| meta.model.clone())
            .unwrap_or_default(),
        provider_source: "failed".to_string(),
        provider_mode: "failed".to_string(),
        fallback_reason: None,
        build_context_observation: None,
        input_tokens: None,
        cache_hit_input_tokens: None,
        reasoning_tokens: None,
        output_tokens: None,
        total_tokens: None,
        first_token_latency_ms: None,
        turn_duration_ms: None,
        user_message,
        assistant_message: assistant_message.clone(),
        trace_steps,
        trace_timeline: Vec::new(),
        tool_activities,
        provider_call_records: Vec::new(),
        hook_trace_records,
        session_summary: assistant_message,
    }
}

pub fn provider_event_meta(provider: &ProviderManager) -> ProviderEventMeta {
    ProviderEventMeta {
        requested_name: provider.requested_name().to_string(),
        provider_name: provider.name().to_string(),
        protocol: provider.protocol_label().to_string(),
        model: provider.model().to_string(),
    }
}

pub fn emit_stream_failed(
    sink: &impl TurnEventSink,
    turn_id: String,
    provider_meta: Option<&ProviderEventMeta>,
    trace_steps: Vec<TurnTraceStep>,
    tool_activities: Option<Vec<TurnToolActivity>>,
    first_token_latency_ms: Option<u64>,
    turn_duration_ms: Option<u64>,
    build_context_observation: Option<BuildContextObservation>,
    trace_timeline: Option<Vec<TraceTimelineEntry>>,
    provider_call_records: Option<Vec<ProviderCallCacheRecord>>,
    hook_trace_records: Option<Vec<HookTraceRecord>>,
    error: String,
    session_id: Option<String>,
) -> TurnEventEnvelope {
    emit_event(
        sink,
        "turn:failed",
        TurnStreamEvent {
            event_id: None,
            session_id,
            turn_id,
            kind: "failed".to_string(),
            event_type: None,
            event_version: None,
            sequence: None,
            emitted_at_ms: None,
            phase: Some("failed".to_string()),
            text: Some("This turn failed.".to_string()),
            reasoning_content: None,
            error: Some(error),
            provider_requested_name: provider_meta.map(|meta| meta.requested_name.clone()),
            provider_name: provider_meta.map(|meta| meta.provider_name.clone()),
            provider_protocol: provider_meta.map(|meta| meta.protocol.clone()),
            provider_model: provider_meta.map(|meta| meta.model.clone()),
            provider_source: None,
            provider_mode: None,
            fallback_reason: None,
            build_context_observation,
            input_tokens: None,
            cache_hit_input_tokens: None,
            reasoning_tokens: None,
            output_tokens: None,
            total_tokens: None,
            first_token_latency_ms,
            turn_duration_ms,
            trace_steps: Some(trace_steps),
            trace_timeline,
            tool_activities,
            provider_call_records,
            hook_trace_records,
            session_summary: None,
            step: None,
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub fn emit_stream_cancelled(
    sink: &impl TurnEventSink,
    turn_id: String,
    provider_meta: Option<&ProviderEventMeta>,
    trace_steps: Vec<TurnTraceStep>,
    tool_activities: Option<Vec<TurnToolActivity>>,
    first_token_latency_ms: Option<u64>,
    turn_duration_ms: Option<u64>,
    build_context_observation: Option<BuildContextObservation>,
    trace_timeline: Option<Vec<TraceTimelineEntry>>,
    provider_call_records: Option<Vec<ProviderCallCacheRecord>>,
    error: String,
    session_id: Option<String>,
) {
    emit_event(
        sink,
        "turn:cancelled",
        TurnStreamEvent {
            event_id: None,
            session_id,
            turn_id,
            kind: "cancelled".to_string(),
            event_type: None,
            event_version: None,
            sequence: None,
            emitted_at_ms: None,
            phase: Some("cancelled".to_string()),
            text: Some("用户终止，发送消息可继续。".to_string()),
            reasoning_content: None,
            error: Some(error),
            provider_requested_name: provider_meta.map(|meta| meta.requested_name.clone()),
            provider_name: provider_meta.map(|meta| meta.provider_name.clone()),
            provider_protocol: provider_meta.map(|meta| meta.protocol.clone()),
            provider_model: provider_meta.map(|meta| meta.model.clone()),
            provider_source: None,
            provider_mode: None,
            fallback_reason: None,
            build_context_observation,
            input_tokens: None,
            cache_hit_input_tokens: None,
            reasoning_tokens: None,
            output_tokens: None,
            total_tokens: None,
            first_token_latency_ms,
            turn_duration_ms,
            trace_steps: Some(trace_steps),
            trace_timeline,
            tool_activities,
            provider_call_records,
            hook_trace_records: None,
            session_summary: None,
            step: None,
        },
    );
}

#[allow(clippy::too_many_arguments)]
pub fn emit_stream_event(
    sink: &impl TurnEventSink,
    name: &str,
    turn_id: String,
    kind: &str,
    phase: Option<&str>,
    text: Option<String>,
    reasoning_content: Option<String>,
    provider_meta: Option<&ProviderEventMeta>,
    provider_source: Option<String>,
    provider_mode: Option<String>,
    fallback_reason: Option<String>,
    build_context_observation: Option<BuildContextObservation>,
    input_tokens: Option<u64>,
    cache_hit_input_tokens: Option<u64>,
    reasoning_tokens: Option<u64>,
    output_tokens: Option<u64>,
    total_tokens: Option<u64>,
    first_token_latency_ms: Option<u64>,
    turn_duration_ms: Option<u64>,
    trace_steps: Option<Vec<TurnTraceStep>>,
    trace_timeline: Option<Vec<TraceTimelineEntry>>,
    tool_activities: Option<Vec<TurnToolActivity>>,
    provider_call_records: Option<Vec<ProviderCallCacheRecord>>,
    hook_trace_records: Option<Vec<HookTraceRecord>>,
    session_summary: Option<String>,
    session_id: Option<String>,
) -> TurnEventEnvelope {
    emit_stream_event_with_step(
        sink,
        name,
        turn_id,
        kind,
        phase,
        text,
        reasoning_content,
        provider_meta,
        provider_source,
        provider_mode,
        fallback_reason,
        build_context_observation,
        input_tokens,
        cache_hit_input_tokens,
        reasoning_tokens,
        output_tokens,
        total_tokens,
        first_token_latency_ms,
        turn_duration_ms,
        trace_steps,
        trace_timeline,
        tool_activities,
        provider_call_records,
        hook_trace_records,
        session_summary,
        session_id,
        None,
    )
}

/// PA-095 #4：携带逻辑 hop step 的事件发射（delta/chunk 归属 hop、
/// turn:trace(calling_model) 触发 StepStart）。
#[allow(clippy::too_many_arguments)]
pub fn emit_stream_event_with_step(
    sink: &impl TurnEventSink,
    name: &str,
    turn_id: String,
    kind: &str,
    phase: Option<&str>,
    text: Option<String>,
    reasoning_content: Option<String>,
    provider_meta: Option<&ProviderEventMeta>,
    provider_source: Option<String>,
    provider_mode: Option<String>,
    fallback_reason: Option<String>,
    build_context_observation: Option<BuildContextObservation>,
    input_tokens: Option<u64>,
    cache_hit_input_tokens: Option<u64>,
    reasoning_tokens: Option<u64>,
    output_tokens: Option<u64>,
    total_tokens: Option<u64>,
    first_token_latency_ms: Option<u64>,
    turn_duration_ms: Option<u64>,
    trace_steps: Option<Vec<TurnTraceStep>>,
    trace_timeline: Option<Vec<TraceTimelineEntry>>,
    tool_activities: Option<Vec<TurnToolActivity>>,
    provider_call_records: Option<Vec<ProviderCallCacheRecord>>,
    hook_trace_records: Option<Vec<HookTraceRecord>>,
    session_summary: Option<String>,
    session_id: Option<String>,
    step: Option<u32>,
) -> TurnEventEnvelope {
    let is_delta_event = name == "turn:delta";
    emit_event(
        sink,
        name,
        TurnStreamEvent {
            event_id: None,
            session_id,
            turn_id,
            kind: kind.to_string(),
            event_type: None,
            event_version: None,
            sequence: None,
            emitted_at_ms: None,
            phase: phase.map(|value| value.to_string()),
            text,
            reasoning_content,
            error: None,
            provider_requested_name: provider_meta.map(|meta| meta.requested_name.clone()),
            provider_name: provider_meta.map(|meta| meta.provider_name.clone()),
            provider_protocol: provider_meta.map(|meta| meta.protocol.clone()),
            provider_model: provider_meta.map(|meta| meta.model.clone()),
            provider_source,
            provider_mode,
            fallback_reason,
            build_context_observation: if is_delta_event {
                None
            } else {
                build_context_observation
            },
            input_tokens,
            cache_hit_input_tokens,
            reasoning_tokens,
            output_tokens,
            total_tokens,
            first_token_latency_ms,
            turn_duration_ms,
            trace_steps: if is_delta_event { None } else { trace_steps },
            trace_timeline: if is_delta_event { None } else { trace_timeline },
            tool_activities: if is_delta_event {
                None
            } else {
                tool_activities
            },
            provider_call_records: if is_delta_event {
                None
            } else {
                provider_call_records
            },
            hook_trace_records: if is_delta_event {
                None
            } else {
                hook_trace_records
            },
            session_summary,
            step,
        },
    )
}

fn resolve_canonical_event_type(
    name: &str,
    phase: Option<&str>,
    build_context_observation: Option<&BuildContextObservation>,
    tool_activities: Option<&[TurnToolActivity]>,
) -> String {
    match name {
        "turn:started" => "turn.created".to_string(),
        "turn:delta" => "turn.output_delta".to_string(),
        "turn:output_end" => "turn.output_end".to_string(),
        "turn:completed" => "turn.completed".to_string(),
        "turn:failed" => "turn.failed".to_string(),
        "turn:cancelled" => "turn.cancelled".to_string(),
        "turn:trace" => {
            if build_context_observation.is_some() && matches!(phase, Some("building_context")) {
                "turn.context_built".to_string()
            } else {
                match phase {
                    Some("calling_model") => "turn.model_call_started".to_string(),
                    _ => "turn.trace_updated".to_string(),
                }
            }
        }
        "turn:tool" => {
            if tool_activities
                .unwrap_or(&[])
                .iter()
                .any(|activity| activity.status == "running")
            {
                "turn.tool_call_started".to_string()
            } else {
                "turn.tool_call_completed".to_string()
            }
        }
        _ => name.replace(':', "."),
    }
}

fn turn_event_version() -> &'static str {
    "turn-event-v1"
}

fn now_timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn next_turn_event_sequence(turn_id: &str) -> u64 {
    let registry = turn_event_sequence_registry();
    let mut state = registry.lock().expect("turn event sequence lock poisoned");
    let next = state.get(turn_id).copied().unwrap_or(0).saturating_add(1);
    state.insert(turn_id.to_string(), next);
    next
}

fn clear_turn_event_sequence(turn_id: &str) {
    let registry = turn_event_sequence_registry();
    let mut state = registry.lock().expect("turn event sequence lock poisoned");
    state.remove(turn_id);
}

fn turn_event_sequence_registry() -> &'static Mutex<HashMap<String, u64>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, u64>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn build_terminal_turn_event_envelope(
    turn_id: &str,
    name: &str,
    phase: Option<&str>,
    build_context_observation: Option<&BuildContextObservation>,
    tool_activities: Option<&[TurnToolActivity]>,
) -> TurnEventEnvelope {
    let sequence = next_turn_event_sequence(turn_id);
    let envelope = TurnEventEnvelope {
        event_id: format!("{}:{}", turn_id, sequence),
        event_type: resolve_canonical_event_type(
            name,
            phase,
            build_context_observation,
            tool_activities,
        ),
        event_version: turn_event_version().to_string(),
        sequence,
        emitted_at_ms: now_timestamp_ms(),
    };

    if matches!(name, "turn:completed" | "turn:failed" | "turn:cancelled") {
        clear_turn_event_sequence(turn_id);
    }

    envelope
}

pub fn emit_turn_failed(
    sink: &impl TurnEventSink,
    turn_id: String,
    provider_requested_name: Option<String>,
    provider_name: Option<String>,
    provider_protocol: Option<String>,
    provider_model: Option<String>,
    trace_steps: Vec<TurnTraceStep>,
    error: String,
    session_id: Option<String>,
) {
    let provider_meta = match (
        provider_requested_name,
        provider_name,
        provider_protocol,
        provider_model,
    ) {
        (Some(requested_name), Some(provider_name), Some(protocol), Some(model)) => {
            Some(ProviderEventMeta {
                requested_name,
                provider_name,
                protocol,
                model,
            })
        }
        _ => None,
    };
    emit_stream_failed(
        sink,
        turn_id,
        provider_meta.as_ref(),
        trace_steps,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        error,
        session_id,
    );
}

pub fn emit_event(
    sink: &impl TurnEventSink,
    name: &str,
    payload: TurnStreamEvent,
) -> TurnEventEnvelope {
    let mut payload = payload;
    let terminal_turn_id = payload.turn_id.clone();
    let next_sequence = next_turn_event_sequence(&payload.turn_id);
    payload.event_id = Some(format!("{}:{}", payload.turn_id, next_sequence));
    payload.event_type = Some(resolve_canonical_event_type(
        name,
        payload.phase.as_deref(),
        payload.build_context_observation.as_ref(),
        payload.tool_activities.as_deref(),
    ));
    payload.event_version = Some(turn_event_version().to_string());
    payload.sequence = Some(next_sequence);
    payload.emitted_at_ms = Some(now_timestamp_ms());

    eprintln!(
        "[pony-agent][runtime] emit {} event_type={} turn={} seq={:?} phase={:?} text_len={} tools={}",
        name,
        payload.event_type.as_deref().unwrap_or("unknown"),
        payload.turn_id,
        payload.sequence,
        payload.phase,
        payload.text.as_ref().map(|text| text.len()).unwrap_or(0),
        payload
            .tool_activities
            .as_ref()
            .map(|tools| tools.len())
            .unwrap_or(0)
    );
    sink.emit(name, payload.clone());
    // PA-091：事件溯源——构造 TurnEvent 经全局注册表持久化（缓冲 + turn 终态 flush）。
    // 构造失败 fail loud（数据完整性错误）；持久化失败由注册通道 contained。
    // 审核 P0：turn:completed 的 assistant/message 不标记 terminal——由额外发射的
    // turn/end 统一触发 flush，避免拆批（AssistantMessage 先行落盘、TurnEnd 第二批，
    // 两事务间崩溃会留下半终态事件流）。failed/cancelled 无额外事件，保持 terminal。
    if let Some(event) = build_turn_event(name, &payload) {
        let session_id = payload.session_id.as_deref().unwrap_or("");
        let is_terminal = matches!(name, "turn:failed" | "turn:cancelled");
        dispatch_event_persist(session_id, &payload.turn_id, event, is_terminal);
    }
    // PA-094：大字段外置——turn:started 携带 build_context_observation 时，额外落一条
    // context/observation 事件（内存携带全量 payload，flush 时外置到独立表）。
    if let Some(observation_event) = build_context_observation_event(name, &payload) {
        let session_id = payload.session_id.as_deref().unwrap_or("");
        dispatch_event_persist(session_id, &payload.turn_id, observation_event, false);
    }
    // PA-094（审核 P0）：turn:started 额外发射 user/message——用户消息事件化，
    // HistoryProjection 从事件重建用户消息的前提（生产路径此前不发射该事件，
    // 事件重建的会话视图缺 user 消息）。payload.text 携带用户消息文本。
    if let Some(user_event) = build_user_message_event(name, &payload) {
        let session_id = payload.session_id.as_deref().unwrap_or("");
        dispatch_event_persist(session_id, &payload.turn_id, user_event, false);
    }
    // PA-095 #4：step/start 事件化（初始 call 由 turn:started 承担 step 0，
    // followup call 由 turn:trace(calling_model) 携带 hop 索引触发）。
    if let Some(step_start_event) = build_step_start_event(name, &payload) {
        let session_id = payload.session_id.as_deref().unwrap_or("");
        dispatch_event_persist(session_id, &payload.turn_id, step_start_event, false);
    }
    // PA-094：turn:completed 额外发射 provider/usage（usage 结算事件，MetricsProjection
    // 重建 ProviderCallCacheRecord 的事件源）与 turn/end（终态结算，TraceProjection
    // 的 token 指标/timeline 挂载依赖 turn/end 触发）。顺序：usage → end（end 为
    // terminal，触发 flush 时整批一起落盘——含 assistant/message，单事务）。
    // PA-095 #4：per-call 结算——usage 与 step/end 相邻成对发射
    // （usage[i] 紧跟 step_end[i]，投影的 call_model 条目结算落点一致）。
    let usage_events = build_provider_usage_event(name, &payload);
    let step_end_events = build_step_end_events(name, &payload, usage_events.len());
    for pair in usage_events.into_iter().zip(step_end_events) {
        let (usage_event, step_end_event) = pair;
        let session_id = payload.session_id.as_deref().unwrap_or("");
        dispatch_event_persist(session_id, &payload.turn_id, usage_event, false);
        dispatch_event_persist(session_id, &payload.turn_id, step_end_event, false);
    }
    if let Some(end_event) = build_turn_end_event(name, &payload) {
        let session_id = payload.session_id.as_deref().unwrap_or("");
        dispatch_event_persist(session_id, &payload.turn_id, end_event, true);
    }
    if matches!(name, "turn:completed" | "turn:failed" | "turn:cancelled") {
        clear_turn_event_sequence(&terminal_turn_id);
    }
    // PA-095：返回本次发射的信封（调用方可复用为 TurnResult 终态信封，
    // 避免二次分配序列号造成事件流与结果信封错位）。
    TurnEventEnvelope {
        event_id: payload.event_id.clone().unwrap_or_default(),
        event_type: payload.event_type.clone().unwrap_or_default(),
        event_version: payload.event_version.clone().unwrap_or_default(),
        sequence: payload.sequence.unwrap_or(0),
        emitted_at_ms: payload.emitted_at_ms.unwrap_or(0),
    }
}

/// PA-094：build_context_observation 事件化——turn:started 携带观察时构造
/// `ContextObservation` 事件（内存形态携带全量 payload；落盘形态只含引用）。
fn build_context_observation_event(
    name: &str,
    payload: &TurnStreamEvent,
) -> Option<crate::agent::turn_event::TurnEvent> {
    if name != "turn:started" {
        return None;
    }
    let observation = payload.build_context_observation.as_ref()?;
    Some(crate::agent::turn_event::TurnEvent::ContextObservation {
        turn_id: payload.turn_id.clone(),
        step: 0,
        observation: Some(observation.clone()),
        observation_ref: None,
    })
}

/// PA-094（审核 P0）：user/message 事件化——turn:started 携带用户消息文本时构造
/// `UserMessage` 事件（HistoryProjection 从事件重建用户消息的前提）。
/// 附件消息的用户消息重建不完整（TurnStreamEvent 无 attachments 字段，已知限制）。
fn build_user_message_event(
    name: &str,
    payload: &TurnStreamEvent,
) -> Option<crate::agent::turn_event::TurnEvent> {
    if name != "turn:started" {
        return None;
    }
    let text = payload.text.clone()?;
    if text.is_empty() {
        return None;
    }
    Some(crate::agent::turn_event::TurnEvent::UserMessage {
        turn_id: payload.turn_id.clone(),
        text,
        attachments: Vec::new(),
    })
}

/// PA-095 #4：step/start 事件化——每次 provider call 发射 StepStart（投影折叠为
/// 该 step 的 call_model 条目）。两个触发点：
/// - turn:started（phase=calling_model）→ 初始 call 的 StepStart{step:0}；
/// - turn:trace(calling_model) 且 payload.step ≥ 1 → followup call 的
///   StepStart{step:k}（运行时 followup 循环填充 hop 索引；直连路径的
///   calling_model trace 不带 step，不重复发射——初始 call 已由 started 覆盖）。
fn build_step_start_event(
    name: &str,
    payload: &TurnStreamEvent,
) -> Option<crate::agent::turn_event::TurnEvent> {
    match name {
        "turn:started" => Some(crate::agent::turn_event::TurnEvent::StepStart {
            turn_id: payload.turn_id.clone(),
            step: 0,
            first_token_latency_ms: None,
        }),
        "turn:trace"
            if payload.phase.as_deref() == Some("calling_model")
                && payload.step.is_some_and(|step| step >= 1) =>
        {
            Some(crate::agent::turn_event::TurnEvent::StepStart {
                turn_id: payload.turn_id.clone(),
                step: payload.step.unwrap_or(0),
                first_token_latency_ms: payload.first_token_latency_ms,
            })
        }
        _ => None,
    }
}

/// PA-095 #4：step/end 事件化——ProviderUsage 同点结算（每条 usage 记录配对
/// 一条 StepEnd{step}，相邻发射；spec："step/end at settlement"）。仅
/// turn:completed 结算（failed/cancelled 无 usage 数据可结算，TurnEnd(Error)
/// 已承担终态；投影 settle_timeline 对未闭合 step 容忍）。
fn build_step_end_events(
    name: &str,
    payload: &TurnStreamEvent,
    usage_count: usize,
) -> Vec<crate::agent::turn_event::TurnEvent> {
    if name != "turn:completed" || usage_count == 0 {
        return Vec::new();
    }
    (0..usage_count as u32)
        .map(|step| crate::agent::turn_event::TurnEvent::StepEnd {
            turn_id: payload.turn_id.clone(),
            step,
        })
        .collect()
}

/// PA-094：provider/usage 事件化——turn 终态（completed/failed/cancelled）携带
/// token 结算字段时构造 `ProviderUsage` 事件（MetricsProjection 重建
/// ProviderCallCacheRecord 的事件源；生产路径此前不发射该事件，重建记录恒为空
/// ——审核 P0 修复）。审核 P1：failed/cancelled 同样发射（多 hop turn 中途失败
/// 时已消耗的 token 不丢失）。
/// 优先逐条发射 `payload.provider_call_records`（step 递增，per-call 粒度，
/// request_kind/latency/prefix_mutation_reasons 保真）；无 records 时回退到
/// 累计 token 单条（step=0，turn 级结算）。
fn build_provider_usage_event(
    name: &str,
    payload: &TurnStreamEvent,
) -> Vec<crate::agent::turn_event::TurnEvent> {
    use crate::agent::turn_event::TurnEvent;
    if !matches!(name, "turn:completed" | "turn:failed" | "turn:cancelled") {
        return Vec::new();
    }
    // per-call 粒度：逐条发射 provider_call_records（step 递增）。
    if let Some(records) = payload.provider_call_records.as_deref() {
        if !records.is_empty() {
            return records
                .iter()
                .enumerate()
                .map(|(index, record)| {
                    let usage = crate::agent::provider::TokenUsage {
                        input_tokens: record.input_tokens,
                        cache_hit_input_tokens: record.cache_hit_input_tokens,
                        cache_hit_source: record.cache_hit_source.clone(),
                        reasoning_tokens: record.reasoning_tokens,
                        output_tokens: record.output_tokens,
                        total_tokens: record.total_tokens,
                    };
                    TurnEvent::ProviderUsage {
                        turn_id: payload.turn_id.clone(),
                        step: index as u32,
                        request_kind: record.request_kind.clone(),
                        usage,
                        cache_hit_input_tokens: record.cache_hit_input_tokens,
                        cache_miss_input_tokens: record.cache_miss_input_tokens,
                        prefix_mutation_reasons: record
                            .prefix_mutation_reasons
                            .iter()
                            .filter_map(|reason| {
                                // 审核 P1：用 serde 序列化（snake_case，如
                                // "session_summary_changed"）而非 Debug 格式
                                // （"SessionSummaryChanged"）——projection 按
                                // snake_case 反序列化，Debug 名称会全部丢失。
                                serde_json::to_value(reason)
                                    .ok()
                                    .and_then(|value| value.as_str().map(str::to_string))
                            })
                            .collect(),
                        first_token_latency_ms: record.first_token_latency_ms,
                        turn_duration_ms: record.turn_duration_ms,
                        latency_kind: record.latency_kind.clone(),
                        // PA-095 #3：provider 取名称（与存储 trace 的 provider 维度
                        // 同语义）；record.source 仅作无名称时的回退。
                        provider: payload
                            .provider_name
                            .clone()
                            .unwrap_or_else(|| {
                                record.provider_source.clone().unwrap_or_default()
                            }),
                        model: payload.provider_model.clone().unwrap_or_default(),
                    }
                })
                .collect();
        }
    }
    // 回退：累计 token 单条（turn 级结算）。
    let Some(usage) = payload_usage(payload) else {
        return Vec::new();
    };
    vec![TurnEvent::ProviderUsage {
        turn_id: payload.turn_id.clone(),
        step: 0,
        request_kind: crate::agent::telemetry::ProviderRequestKind::InitialRequest,
        usage,
        cache_hit_input_tokens: payload.cache_hit_input_tokens,
        cache_miss_input_tokens: payload.input_tokens.and_then(|input| {
            payload
                .cache_hit_input_tokens
                .map(|hit| input.saturating_sub(hit))
        }),
        prefix_mutation_reasons: Vec::new(),
        first_token_latency_ms: payload.first_token_latency_ms,
        turn_duration_ms: payload.turn_duration_ms,
        latency_kind: crate::agent::telemetry::ProviderLatencyKind::ProviderStream,
        provider: payload.provider_name.clone().unwrap_or_default(),
        model: payload.provider_model.clone().unwrap_or_default(),
    }]
}

/// PA-094：turn/end 事件化——turn:completed 额外构造 `TurnEnd`（reason=Completed）。
/// 此前 completed 只落 assistant/message，投影的 turn/end 结算（token 指标、
/// timeline 挂载、event_type 标记）永不触发——审核 P0 修复。failed/cancelled
/// 已由 build_turn_event 发射 TurnEnd，此处不重复。
fn build_turn_end_event(
    name: &str,
    payload: &TurnStreamEvent,
) -> Option<crate::agent::turn_event::TurnEvent> {
    use crate::agent::turn_event::{TurnEndReason, TurnEvent};
    if name != "turn:completed" {
        return None;
    }
    Some(TurnEvent::TurnEnd {
        turn_id: payload.turn_id.clone(),
        reason: TurnEndReason::Completed,
        turn_duration_ms: payload.turn_duration_ms,
    })
}

/// PA-091：TurnStreamEvent → TurnEvent 映射（design.md 映射表）。
/// `turn:output_end` / `turn:trace` / `turn.context_built` 显式不落盘（返回 None）。
fn build_turn_event(
    name: &str,
    payload: &TurnStreamEvent,
) -> Option<crate::agent::turn_event::TurnEvent> {
    use crate::agent::turn_event::{TurnEndReason, TurnEvent};
    match name {
        "turn:started" => Some(TurnEvent::TurnStart {
            turn_id: payload.turn_id.clone(),
        }),
        "turn:delta" => {
            // reasoning-only chunk（无 text）也落盘（text 为空串），保证
            // "every emitted event SHALL be persisted" 的语义完整。
            Some(TurnEvent::AssistantChunk {
                turn_id: payload.turn_id.clone(),
                // PA-095 #4：chunk 归属逻辑 hop（0-based；缺省归 step 0，
                // 与 legacy 无 step 的 wire payload 兼容）。
                step: payload.step.unwrap_or(0),
                text: payload.text.clone().unwrap_or_default(),
            })
        }
        "turn:completed" => Some(TurnEvent::AssistantMessage {
            turn_id: payload.turn_id.clone(),
            step: 0,
            text: payload.text.clone().unwrap_or_default(),
            reasoning_content: payload.reasoning_content.clone(),
            usage: payload_usage(payload),
            chunk_missing: None,
        }),
        "turn:failed" => Some(TurnEvent::TurnEnd {
            turn_id: payload.turn_id.clone(),
            reason: TurnEndReason::Error,
            turn_duration_ms: payload.turn_duration_ms,
        }),
        "turn:cancelled" => Some(TurnEvent::TurnEnd {
            turn_id: payload.turn_id.clone(),
            reason: TurnEndReason::Cancelled,
            turn_duration_ms: payload.turn_duration_ms,
        }),
        "turn:tool" => build_tool_event(payload),
        _ => None,
    }
}

/// 从终态 payload 的 token 字段构造 usage（全 None 时返回 None）。
fn payload_usage(payload: &TurnStreamEvent) -> Option<crate::agent::provider::TokenUsage> {
    if payload.input_tokens.is_none()
        && payload.cache_hit_input_tokens.is_none()
        && payload.reasoning_tokens.is_none()
        && payload.output_tokens.is_none()
        && payload.total_tokens.is_none()
    {
        return None;
    }
    Some(crate::agent::provider::TokenUsage {
        input_tokens: payload.input_tokens,
        cache_hit_input_tokens: payload.cache_hit_input_tokens,
        cache_hit_source: None,
        reasoning_tokens: payload.reasoning_tokens,
        output_tokens: payload.output_tokens,
        total_tokens: payload.total_tokens,
    })
}

/// turn:tool → ToolCall（running）/ ToolResult（completed），取最后一个 activity。
fn build_tool_event(payload: &TurnStreamEvent) -> Option<crate::agent::turn_event::TurnEvent> {
    use crate::agent::turn_event::TurnEvent;
    let activity = payload.tool_activities.as_deref()?.last()?;
    let step = payload.sequence.unwrap_or(0) as u32;
    if activity.status == "running" {
        Some(TurnEvent::ToolCall {
            turn_id: payload.turn_id.clone(),
            step,
            call_id: activity.id.clone(),
            name: activity.name.clone(),
            arguments: activity.arguments_text.clone().unwrap_or_default(),
            started_at_ms: None,
        })
    } else {
        Some(TurnEvent::ToolResult {
            turn_id: payload.turn_id.clone(),
            step,
            call_id: activity.id.clone(),
            result: activity.result_text.clone(),
            error: activity.error.as_ref().map(|e| e.to_string()),
            status: Some(activity.status.clone()),
            duration_ms: activity
                .duration_seconds
                .map(|seconds| (seconds * 1000.0) as u64),
            artifacts: activity.artifacts.clone(),
            capability_invocation: activity.capability_invocation.clone(),
        })
    }
}

pub fn normalize_user_message(message: &str) -> String {
    let normalized = message.trim();
    if normalized.is_empty() {
        "请先输入问题。".to_string()
    } else {
        normalized.to_string()
    }
}

pub fn token_usage_parts(
    token_usage: Option<&TokenUsage>,
) -> (
    Option<u64>,
    Option<u64>,
    Option<u64>,
    Option<u64>,
    Option<u64>,
) {
    match token_usage {
        Some(token_usage) => (
            token_usage.input_tokens,
            token_usage.cache_hit_input_tokens,
            token_usage.reasoning_tokens,
            token_usage.output_tokens,
            token_usage.total_tokens,
        ),
        None => (None, None, None, None, None),
    }
}

pub fn provider_failure_message(
    provider_mode: &str,
    fallback_reason: Option<&str>,
) -> Option<String> {
    if provider_mode == "mock" {
        return Some(
            fallback_reason
                .unwrap_or("模型调用失败，未返回真实结果。")
                .to_string(),
        );
    }

    None
}

fn stream_delta_chunks(
    sink: &impl TurnEventSink,
    turn_id: &str,
    phase: &str,
    text: Option<&str>,
    reasoning_content: Option<&str>,
    started_at: &std::time::Instant,
    initial_latency_ms: Option<u64>,
    measure_first_delta_latency: bool,
) -> Option<u64> {
    let source = text.or(reasoning_content).unwrap_or_default();
    let mut first_token_latency_ms = initial_latency_ms;
    let mut latency_emitted = false;
    let chunks = source
        .as_bytes()
        .chunks(48)
        .map(|chunk| String::from_utf8_lossy(chunk).to_string())
        .collect::<Vec<_>>();

    for (index, delta) in chunks.iter().enumerate() {
        let latency = if !latency_emitted && index == 0 {
            if let Some(value) = first_token_latency_ms {
                latency_emitted = true;
                Some(value)
            } else if measure_first_delta_latency {
                let value = started_at.elapsed().as_millis() as u64;
                first_token_latency_ms = Some(value);
                latency_emitted = true;
                Some(value)
            } else {
                None
            }
        } else {
            None
        };

        emit_stream_event(
            sink,
            "turn:delta",
            turn_id.to_string(),
            "delta",
            Some(phase),
            text.as_ref().map(|_| delta.clone()),
            reasoning_content.as_ref().map(|_| delta.clone()),
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
            latency,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        );

        if index + 1 < chunks.len() {
            std::thread::sleep(std::time::Duration::from_millis(14));
        }
    }

    first_token_latency_ms
}

pub fn stream_reasoning_chunks(
    sink: &impl TurnEventSink,
    turn_id: &str,
    phase: &str,
    reasoning_content: &str,
    started_at: &std::time::Instant,
    initial_latency_ms: Option<u64>,
    measure_first_delta_latency: bool,
) -> Option<u64> {
    stream_delta_chunks(
        sink,
        turn_id,
        phase,
        None,
        Some(reasoning_content),
        started_at,
        initial_latency_ms,
        measure_first_delta_latency,
    )
}

pub fn stream_text_chunks(
    sink: &impl TurnEventSink,
    turn_id: &str,
    phase: &str,
    text: &str,
    started_at: &std::time::Instant,
    initial_latency_ms: Option<u64>,
    measure_first_delta_latency: bool,
) -> Option<u64> {
    stream_delta_chunks(
        sink,
        turn_id,
        phase,
        Some(text),
        None,
        started_at,
        initial_latency_ms,
        measure_first_delta_latency,
    )
}

pub fn runtime_log(message: String) {
    eprintln!("[pony-runtime] {}", message);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct RecordingSink {
        events: RefCell<Vec<(String, TurnStreamEvent)>>,
    }

    impl RecordingSink {
        fn new() -> Self {
            Self {
                events: RefCell::new(Vec::new()),
            }
        }
    }

    impl TurnEventSink for RecordingSink {
        fn emit(&self, name: &str, payload: TurnStreamEvent) {
            self.events.borrow_mut().push((name.to_string(), payload));
        }
    }

    fn sample_payload(turn_id: &str, kind: &str, text: Option<&str>) -> TurnStreamEvent {
        TurnStreamEvent {
            event_id: None,
            session_id: Some("s1".to_string()),
            turn_id: turn_id.to_string(),
            kind: kind.to_string(),
            event_type: None,
            event_version: None,
            sequence: None,
            emitted_at_ms: None,
            phase: Some("calling_model".to_string()),
            text: text.map(str::to_string),
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
            step: None,
        }
    }

    #[test]
    fn event_name_mapping_table_driven() {
        use crate::agent::turn_event::TurnEvent;
        // 8 种现有发射名 → 映射断言（design.md 映射表）
        let cases: Vec<(&str, Option<&str>)> = vec![
            ("turn:started", Some("turn/start")),
            ("turn:delta", Some("assistant/chunk")),
            ("turn:output_end", None),
            ("turn:completed", Some("assistant/message")),
            ("turn:failed", Some("turn/end")),
            ("turn:cancelled", Some("turn/end")),
            ("turn:trace", None),
            ("turn:tool", None), // 无 tool_activities 时不落盘
        ];
        for (name, expected) in cases {
            let payload = sample_payload("turn-1", name, Some("hi"));
            let event = build_turn_event(name, &payload);
            match expected {
                Some(type_name) => {
                    let event = event.unwrap_or_else(|| panic!("{name} must map to {type_name}"));
                    assert_eq!(event.type_name(), type_name, "{name} mapping");
                }
                None => assert!(event.is_none(), "{name} must not persist"),
            }
        }
        // reason 分支断言：failed → Error，cancelled → Cancelled
        use crate::agent::turn_event::TurnEndReason;
        let failed = build_turn_event("turn:failed", &sample_payload("t1", "turn:failed", None))
            .expect("failed event");
        match failed {
            TurnEvent::TurnEnd { reason, .. } => {
                assert_eq!(reason, TurnEndReason::Error, "failed reason");
            }
            _ => panic!("expected TurnEnd"),
        }
        let cancelled = build_turn_event(
            "turn:cancelled",
            &sample_payload("t1", "turn:cancelled", None),
        )
        .expect("cancelled event");
        match cancelled {
            TurnEvent::TurnEnd { reason, .. } => {
                assert_eq!(reason, TurnEndReason::Cancelled, "cancelled reason");
            }
            _ => panic!("expected TurnEnd"),
        }
    }

    /// PA-094：turn:started 携带 build_context_observation → 额外构造
    /// context/observation 事件（内存形态携带全量 payload，落盘形态只含引用）。
    #[test]
    fn context_observation_event_built_from_started_payload() {
        use crate::agent::provider::BuildContextObservation;
        use crate::agent::turn_event::TurnEvent;
        let mut payload = sample_payload("turn-1", "turn:started", None);
        payload.build_context_observation = Some(BuildContextObservation {
            request_format: "chat".into(),
            message_count: 2,
            image_count: 0,
            tool_count: 1,
            temperature: 0.7,
            max_output_tokens: 4096,
            stable_prefix_text: String::new(),
            semi_stable_context_text: String::new(),
            volatile_input_text: String::new(),
            prefix_mutation_reasons: Vec::new(),
            context_refresh_reason: None,
            instruction_scope_sources: Vec::new(),
            conversation_carry_mode: None,
            request_messages_text: "messages".into(),
            tool_definitions_text: "tools".into(),
        });
        let event =
            build_context_observation_event("turn:started", &payload).expect("observation event");
        match event {
            TurnEvent::ContextObservation {
                turn_id,
                step,
                observation,
                observation_ref,
            } => {
                assert_eq!(turn_id, "turn-1");
                assert_eq!(step, 0);
                assert!(observation.is_some(), "payload carried in memory");
                assert!(observation_ref.is_none(), "ref assigned at flush");
            }
            _ => panic!("expected ContextObservation"),
        }
        // 非 started 事件不构造。
        let delta = sample_payload("turn-1", "turn:delta", Some("hi"));
        assert!(build_context_observation_event("turn:delta", &delta).is_none());
        // 无 observation 的 started 不构造。
        let bare = sample_payload("turn-1", "turn:started", None);
        assert!(build_context_observation_event("turn:started", &bare).is_none());
    }

    /// PA-094（审核 P0）：turn:started 携带用户消息文本时额外发射 user/message
    /// 事件（HistoryProjection 从事件重建用户消息的前提）。
    #[test]
    fn started_emits_user_message_event() {
        use crate::agent::turn_event::TurnEvent;
        let mut payload = sample_payload("turn-1", "turn:started", None);
        payload.text = Some("hello world".to_string());
        let event = build_user_message_event("turn:started", &payload).expect("user event");
        match event {
            TurnEvent::UserMessage {
                turn_id,
                text,
                attachments,
            } => {
                assert_eq!(turn_id, "turn-1");
                assert_eq!(text, "hello world");
                assert!(attachments.is_empty());
            }
            _ => panic!("expected UserMessage"),
        }
        // 非 started 不发射。
        let delta = sample_payload("turn-1", "turn:delta", Some("hi"));
        assert!(build_user_message_event("turn:delta", &delta).is_none());
        // 无文本的 started 不发射。
        let bare = sample_payload("turn-1", "turn:started", None);
        assert!(build_user_message_event("turn:started", &bare).is_none());
    }

    #[test]
    fn emit_event_persist_channel_buffers_aggregates_and_flushes() {
        use crate::agent::turn_event::TurnEvent;
        use std::sync::{Arc, Mutex};
        // 注册测试持久化通道：收集 (session_id, turn_id, events, is_terminal)
        let received: Arc<Mutex<Vec<(String, String, Vec<TurnEvent>, bool)>>> =
            Arc::new(Mutex::new(Vec::new()));
        let sink = RecordingSink::new();
        let received_clone = Arc::clone(&received);
        register_event_persist(Arc::new(move |session_id, turn_id, event, is_terminal| {
            received_clone.lock().expect("received lock").push((
                session_id.to_string(),
                turn_id.to_string(),
                vec![event],
                is_terminal,
            ));
        }));
        // emit 序列：started → delta ×2（同 turn 应合并为一条 chunk）→ completed
        emit_event(
            &sink,
            "turn:started",
            sample_payload("turn-1", "turn:started", None),
        );
        emit_event(
            &sink,
            "turn:delta",
            sample_payload("turn-1", "turn:delta", Some("hel")),
        );
        emit_event(
            &sink,
            "turn:delta",
            sample_payload("turn-1", "turn:delta", Some("lo")),
        );
        emit_event(
            &sink,
            "turn:completed",
            sample_payload("turn-1", "turn:completed", Some("hello")),
        );
        // 验证：4 次 emit → 5 条通道记录（聚合在持久化闭包内做，通道收到原事件；
        // 聚合正确性由 control_plane 闭包负责——此处验证 emit→persist 链路与终态标记。
        // PA-094：completed 额外发射 turn/end（终态结算事件），故 5 条；
        // provider/usage 仅在携带 token 结算字段时发射（sample 无 token，不发射）。
        // 审核 P0：completed 的 assistant/message 不标记 terminal（由 turn/end
        // 统一触发 flush，避免拆批），故 records[3] 为 false、records[4] 为 true。
        // 全局 EVENT_PERSIST_REGISTRY 可能被并行测试污染，只统计本 turn 的记录。
        let records = received.lock().expect("received lock");
        let own: Vec<_> = records
            .iter()
            .filter(|(_, turn_id, _, _)| turn_id == "turn-1")
            .cloned()
            .collect();
        // PA-095 #4：started 额外发射 step/start（初始 call 的 StepStart{0}），
        // 通道记录 5 → 6：[turn/start, step/start, chunk, chunk, assistant/message, turn/end]。
        assert_eq!(own.len(), 6, "every emitted event reaches persist");
        assert_eq!(own[0].3, false, "started not terminal");
        assert_eq!(
            own[4].3, false,
            "completed assistant/message not terminal (deferred)"
        );
        assert_eq!(own[5].3, true, "completed turn/end is terminal");
        assert_eq!(own[0].2[0].type_name(), "turn/start");
        assert_eq!(own[1].2[0].type_name(), "step/start");
        assert_eq!(own[2].2[0].type_name(), "assistant/chunk");
        assert_eq!(own[3].2[0].type_name(), "assistant/chunk");
        assert_eq!(own[4].2[0].type_name(), "assistant/message");
        assert_eq!(own[5].2[0].type_name(), "turn/end");
        drop(records);
        clear_event_persist();
    }

    #[test]
    fn event_mapping_tool_started_and_completed() {
        use crate::agent::telemetry::TurnToolActivity;
        use crate::agent::turn_event::TurnEvent;
        // running → ToolCall
        let mut payload = sample_payload("turn-1", "turn:tool", None);
        payload.tool_activities = Some(vec![TurnToolActivity {
            id: "act-1".to_string(),
            name: "bash".to_string(),
            canonical_tool_name: None,
            display_name_zh: None,
            status: "running".to_string(),
            description: "run".to_string(),
            arguments_text: Some("{}".to_string()),
            result_text: None,
            duration_seconds: None,
            parent_activity_id: None,
            artifacts: None,
            error: None,
            capability_invocation: None,
        }]);
        let event = build_turn_event("turn:tool", &payload).expect("tool call event");
        assert_eq!(event.type_name(), "tool/call");
        // completed → ToolResult
        payload.tool_activities = Some(vec![TurnToolActivity {
            id: "act-1".to_string(),
            name: "bash".to_string(),
            canonical_tool_name: None,
            display_name_zh: None,
            status: "done".to_string(),
            description: "run".to_string(),
            arguments_text: Some("{}".to_string()),
            result_text: Some("ok".to_string()),
            duration_seconds: Some(0.5),
            parent_activity_id: None,
            artifacts: None,
            error: None,
            capability_invocation: None,
        }]);
        let event = build_turn_event("turn:tool", &payload).expect("tool result event");
        assert_eq!(event.type_name(), "tool/result");
        match event {
            TurnEvent::ToolResult {
                result,
                duration_ms,
                ..
            } => {
                assert_eq!(result.as_deref(), Some("ok"));
                assert_eq!(duration_ms, Some(500));
            }
            _ => panic!("expected ToolResult"),
        }
    }

    #[test]
    fn event_mapping_completed_carries_usage() {
        use crate::agent::turn_event::TurnEvent;
        let mut payload = sample_payload("turn-1", "turn:completed", Some("done"));
        payload.input_tokens = Some(100);
        payload.output_tokens = Some(50);
        payload.total_tokens = Some(150);
        let event = build_turn_event("turn:completed", &payload).expect("completed event");
        match event {
            TurnEvent::AssistantMessage { usage, text, .. } => {
                assert_eq!(text, "done");
                let usage = usage.expect("usage present");
                assert_eq!(usage.input_tokens, Some(100));
                assert_eq!(usage.output_tokens, Some(50));
            }
            _ => panic!("expected AssistantMessage"),
        }
    }

    /// PA-094（审核 P0）：turn:completed 携带 token 结算字段时额外发射
    /// provider/usage（MetricsProjection 重建 ProviderCallCacheRecord 的事件源）
    /// 与 turn/end（终态结算，TraceProjection 的 token 指标/timeline 挂载依赖）。
    #[test]
    fn completed_emits_provider_usage_and_turn_end() {
        use crate::agent::turn_event::{TurnEndReason, TurnEvent};
        let mut payload = sample_payload("turn-1", "turn:completed", Some("done"));
        payload.input_tokens = Some(100);
        payload.cache_hit_input_tokens = Some(40);
        payload.reasoning_tokens = Some(10);
        payload.output_tokens = Some(50);
        payload.total_tokens = Some(160);
        payload.first_token_latency_ms = Some(88);
        payload.turn_duration_ms = Some(1200);
        payload.provider_name = Some("deepseek".to_string());
        payload.provider_model = Some("deepseek-chat".to_string());
        // provider/usage 事件：无 provider_call_records 时回退 turn 级结算
        // （step=0，InitialRequest）。
        let usage_events = build_provider_usage_event("turn:completed", &payload);
        assert_eq!(usage_events.len(), 1, "fallback single usage event");
        match &usage_events[0] {
            TurnEvent::ProviderUsage {
                turn_id,
                step,
                request_kind,
                usage,
                cache_hit_input_tokens,
                cache_miss_input_tokens,
                first_token_latency_ms,
                turn_duration_ms,
                latency_kind,
                provider,
                model,
                ..
            } => {
                assert_eq!(turn_id, "turn-1");
                assert_eq!(*step, 0);
                assert_eq!(
                    request_kind,
                    &crate::agent::telemetry::ProviderRequestKind::InitialRequest
                );
                assert_eq!(usage.input_tokens, Some(100));
                assert_eq!(usage.output_tokens, Some(50));
                assert_eq!(*cache_hit_input_tokens, Some(40));
                assert_eq!(*cache_miss_input_tokens, Some(60), "input - cache_hit");
                assert_eq!(*first_token_latency_ms, Some(88));
                assert_eq!(*turn_duration_ms, Some(1200));
                assert_eq!(
                    latency_kind,
                    &crate::agent::telemetry::ProviderLatencyKind::ProviderStream
                );
                assert_eq!(provider, "deepseek");
                assert_eq!(model, "deepseek-chat");
            }
            _ => panic!("expected ProviderUsage"),
        }
        // per-call 粒度：携带 provider_call_records 时逐条发射（step 递增）。
        payload.provider_call_records = Some(vec![
            crate::agent::telemetry::ProviderCallCacheRecord {
                request_kind: crate::agent::telemetry::ProviderRequestKind::InitialRequest,
                provider_source: Some("deepseek".to_string()),
                provider_mode: None,
                input_tokens: Some(100),
                cache_hit_input_tokens: Some(40),
                cache_hit_source: None,
                cache_miss_input_tokens: Some(60),
                reasoning_tokens: Some(10),
                output_tokens: Some(50),
                total_tokens: Some(160),
                first_token_latency_ms: Some(88),
                turn_duration_ms: Some(500),
                latency_kind: crate::agent::telemetry::ProviderLatencyKind::ProviderStream,
                prefix_mutation_reasons: Vec::new(),
            },
            crate::agent::telemetry::ProviderCallCacheRecord {
                request_kind: crate::agent::telemetry::ProviderRequestKind::ToolFollowup,
                provider_source: Some("deepseek".to_string()),
                provider_mode: None,
                input_tokens: Some(80),
                cache_hit_input_tokens: Some(30),
                cache_hit_source: None,
                cache_miss_input_tokens: Some(50),
                reasoning_tokens: Some(5),
                output_tokens: Some(30),
                total_tokens: Some(115),
                first_token_latency_ms: None,
                turn_duration_ms: Some(300),
                latency_kind: crate::agent::telemetry::ProviderLatencyKind::BufferedResponse,
                prefix_mutation_reasons: vec![
                    crate::agent::provider::PrefixMutationReason::SessionSummaryChanged,
                    crate::agent::provider::PrefixMutationReason::HistoryBoundaryShifted,
                ],
            },
        ]);
        let per_call_events = build_provider_usage_event("turn:completed", &payload);
        assert_eq!(per_call_events.len(), 2, "per-call usage events");
        match &per_call_events[0] {
            TurnEvent::ProviderUsage {
                step, request_kind, ..
            } => {
                assert_eq!(*step, 0);
                assert_eq!(
                    request_kind,
                    &crate::agent::telemetry::ProviderRequestKind::InitialRequest
                );
            }
            _ => panic!("expected ProviderUsage"),
        }
        match &per_call_events[1] {
            TurnEvent::ProviderUsage {
                step,
                request_kind,
                latency_kind,
                turn_duration_ms,
                prefix_mutation_reasons,
                ..
            } => {
                assert_eq!(*step, 1, "step increments per call");
                assert_eq!(
                    request_kind,
                    &crate::agent::telemetry::ProviderRequestKind::ToolFollowup
                );
                assert_eq!(
                    latency_kind,
                    &crate::agent::telemetry::ProviderLatencyKind::BufferedResponse
                );
                assert_eq!(*turn_duration_ms, Some(300));
                // 审核 P1：prefix_mutation_reasons 用 serde 序列化（snake_case），
                // 而非 Debug 格式——projection 按 snake_case 反序列化。
                assert_eq!(
                    prefix_mutation_reasons,
                    &vec![
                        "session_summary_changed".to_string(),
                        "history_boundary_shifted".to_string()
                    ],
                    "prefix reasons serialized as snake_case"
                );
            }
            _ => panic!("expected ProviderUsage"),
        }
        // turn/end 事件：reason=Completed。
        let end_event = build_turn_end_event("turn:completed", &payload).expect("end event");
        match end_event {
            TurnEvent::TurnEnd {
                turn_id,
                reason,
                turn_duration_ms,
            } => {
                assert_eq!(turn_id, "turn-1");
                assert_eq!(reason, TurnEndReason::Completed);
                assert_eq!(turn_duration_ms, Some(1200));
            }
            _ => panic!("expected TurnEnd"),
        }
        // 非 completed 不发射。
        let started = sample_payload("turn-1", "turn:started", None);
        assert!(build_provider_usage_event("turn:started", &started).is_empty());
        assert!(build_turn_end_event("turn:started", &started).is_none());
        // 无 token 的 completed 不发射 provider/usage（但 turn/end 仍发射）。
        let bare = sample_payload("turn-1", "turn:completed", Some("done"));
        assert!(build_provider_usage_event("turn:completed", &bare).is_empty());
        assert!(build_turn_end_event("turn:completed", &bare).is_some());
    }

    #[test]
    fn canonical_trace_event_aligns_with_context_build_end_hook_point() {
        let observation = BuildContextObservation {
            request_format: "chat".to_string(),
            message_count: 3,
            image_count: 0,
            tool_count: 0,
            temperature: 0.2,
            max_output_tokens: 512,
            stable_prefix_text: "prefix".to_string(),
            semi_stable_context_text: "ctx".to_string(),
            volatile_input_text: "input".to_string(),
            prefix_mutation_reasons: Vec::new(),
            context_refresh_reason: None,
            instruction_scope_sources: Vec::new(),
            conversation_carry_mode: None,
            request_messages_text: "messages".to_string(),
            tool_definitions_text: String::new(),
        };

        let event_type = resolve_canonical_event_type(
            "turn:trace",
            Some("building_context"),
            Some(&observation),
            None,
        );

        assert_eq!(event_type, "turn.context_built");
        assert!(crate::agent::hooks::hook_point_matches_canonical_boundary(
            &crate::agent::hooks::TurnHookPoint::ContextBuildEnd,
            &event_type,
            "building_context"
        ));
    }

    #[test]
    fn canonical_trace_event_aligns_with_model_call_start_hook_point() {
        let event_type =
            resolve_canonical_event_type("turn:trace", Some("calling_model"), None, None);

        assert_eq!(event_type, "turn.model_call_started");
        assert!(crate::agent::hooks::hook_point_matches_canonical_boundary(
            &crate::agent::hooks::TurnHookPoint::ModelCallStart,
            &event_type,
            "calling_model"
        ));
    }

    #[test]
    fn canonical_trace_event_prefers_model_call_started_over_context_built_outside_building_context(
    ) {
        let observation = BuildContextObservation {
            request_format: "responses".to_string(),
            message_count: 4,
            image_count: 0,
            tool_count: 1,
            temperature: 0.2,
            max_output_tokens: 512,
            stable_prefix_text: "prefix".to_string(),
            semi_stable_context_text: "ctx".to_string(),
            volatile_input_text: "input".to_string(),
            prefix_mutation_reasons: Vec::new(),
            context_refresh_reason: None,
            instruction_scope_sources: Vec::new(),
            conversation_carry_mode: None,
            request_messages_text: "messages".to_string(),
            tool_definitions_text: "tools".to_string(),
        };

        let event_type = resolve_canonical_event_type(
            "turn:trace",
            Some("calling_model"),
            Some(&observation),
            None,
        );

        assert_eq!(event_type, "turn.model_call_started");
        assert!(crate::agent::hooks::hook_point_matches_canonical_boundary(
            &crate::agent::hooks::TurnHookPoint::ModelCallStart,
            &event_type,
            "calling_model"
        ));
    }

    #[test]
    fn delta_event_aligns_with_model_response_end_hook_point() {
        let event_type =
            resolve_canonical_event_type("turn:delta", Some("streaming_response"), None, None);

        assert_eq!(event_type, "turn.output_delta");
        assert!(crate::agent::hooks::hook_point_matches_canonical_boundary(
            &crate::agent::hooks::TurnHookPoint::ModelResponseEnd,
            &event_type,
            "streaming_response"
        ));
    }

    #[test]
    fn delta_stream_event_strips_heavy_payload_fields() {
        let sink = RecordingSink::new();
        let observation = BuildContextObservation {
            request_format: "responses".to_string(),
            message_count: 4,
            image_count: 0,
            tool_count: 1,
            temperature: 0.2,
            max_output_tokens: 512,
            stable_prefix_text: "prefix".to_string(),
            semi_stable_context_text: "ctx".to_string(),
            volatile_input_text: "input".to_string(),
            prefix_mutation_reasons: Vec::new(),
            context_refresh_reason: None,
            instruction_scope_sources: Vec::new(),
            conversation_carry_mode: None,
            request_messages_text: "messages".to_string(),
            tool_definitions_text: "tools".to_string(),
        };

        emit_stream_event(
            &sink,
            "turn:delta",
            "turn-heavy-delta".to_string(),
            "delta",
            Some("calling_model"),
            Some("partial".to_string()),
            Some("thinking".to_string()),
            None,
            None,
            None,
            None,
            Some(observation),
            Some(11),
            Some(3),
            Some(5),
            Some(7),
            Some(18),
            Some(42),
            None,
            Some(vec![TurnTraceStep {
                id: "call-model".to_string(),
                label: "Call model".to_string(),
                state: "active".to_string(),
            }]),
            Some(vec![TraceTimelineEntry {
                id: "model-1".to_string(),
                kind: "call_model".to_string(),
                label: "CALL MODEL #1".to_string(),
                state: "active".to_string(),
                sequence: 1,
                text: Some("partial".to_string()),
                ..TraceTimelineEntry::default()
            }]),
            Some(vec![TurnToolActivity {
                id: "tool-1".to_string(),
                name: "workspace.read_file".to_string(),
                canonical_tool_name: Some("Read".to_string()),
                display_name_zh: Some("读取".to_string()),
                status: "done".to_string(),
                description: "read done".to_string(),
                arguments_text: None,
                result_text: None,
                duration_seconds: Some(0.1),
                parent_activity_id: None,
                artifacts: None,
                error: None,
                capability_invocation: None,
            }]),
            Some(vec![ProviderCallCacheRecord::default()]),
            Some(Vec::new()),
            Some("summary".to_string()),
            Some("session-1".to_string()),
        );

        let events = sink.events.borrow();
        let (_, payload) = events
            .first()
            .expect("delta event should have been emitted");

        assert_eq!(payload.text.as_deref(), Some("partial"));
        assert_eq!(payload.reasoning_content.as_deref(), Some("thinking"));
        assert_eq!(payload.first_token_latency_ms, Some(42));
        assert_eq!(payload.session_id.as_deref(), Some("session-1"));
        assert_eq!(payload.session_summary.as_deref(), Some("summary"));
        assert!(payload.build_context_observation.is_none());
        assert!(payload.trace_steps.is_none());
        assert!(payload.trace_timeline.is_none());
        assert!(payload.tool_activities.is_none());
        assert!(payload.provider_call_records.is_none());
        assert!(payload.hook_trace_records.is_none());
    }

    #[test]
    fn trace_event_with_observation_outside_building_context_falls_back_to_trace_updated() {
        let observation = BuildContextObservation {
            request_format: "responses".to_string(),
            message_count: 4,
            image_count: 0,
            tool_count: 1,
            temperature: 0.2,
            max_output_tokens: 512,
            stable_prefix_text: "prefix".to_string(),
            semi_stable_context_text: "ctx".to_string(),
            volatile_input_text: "input".to_string(),
            prefix_mutation_reasons: Vec::new(),
            context_refresh_reason: None,
            instruction_scope_sources: Vec::new(),
            conversation_carry_mode: None,
            request_messages_text: "messages".to_string(),
            tool_definitions_text: "tools".to_string(),
        };

        let event_type = resolve_canonical_event_type(
            "turn:trace",
            Some("tool_result_integrating"),
            Some(&observation),
            None,
        );

        assert_eq!(event_type, "turn.trace_updated");
    }

    #[test]
    fn tool_event_aligns_with_tool_call_start_hook_point() {
        let tool_activities = vec![TurnToolActivity {
            id: "tool-1".to_string(),
            name: "workspace.read_file".to_string(),
            canonical_tool_name: Some("Read".to_string()),
            display_name_zh: Some("读取".to_string()),
            status: "running".to_string(),
            description: "running".to_string(),
            arguments_text: None,
            result_text: None,
            duration_seconds: None,
            parent_activity_id: None,
            artifacts: None,
            error: None,
            capability_invocation: None,
        }];
        let event_type = resolve_canonical_event_type(
            "turn:tool",
            Some("executing_tool"),
            None,
            Some(&tool_activities),
        );

        assert_eq!(event_type, "turn.tool_call_started");
        assert!(crate::agent::hooks::hook_point_matches_canonical_boundary(
            &crate::agent::hooks::TurnHookPoint::ToolCallStart,
            &event_type,
            "executing_tool"
        ));
    }

    #[test]
    fn tool_event_aligns_with_tool_call_end_hook_point() {
        let tool_activities = vec![TurnToolActivity {
            id: "tool-1".to_string(),
            name: "workspace.read_file".to_string(),
            canonical_tool_name: Some("Read".to_string()),
            display_name_zh: Some("读取".to_string()),
            status: "done".to_string(),
            description: "ok".to_string(),
            arguments_text: None,
            result_text: None,
            duration_seconds: Some(0.1),
            parent_activity_id: None,
            artifacts: None,
            error: None,
            capability_invocation: None,
        }];
        let event_type = resolve_canonical_event_type(
            "turn:tool",
            Some("executing_tool"),
            None,
            Some(&tool_activities),
        );

        assert_eq!(event_type, "turn.tool_call_completed");
        assert!(crate::agent::hooks::hook_point_matches_canonical_boundary(
            &crate::agent::hooks::TurnHookPoint::ToolCallEnd,
            &event_type,
            "tool_result_integrating"
        ));
    }

    #[test]
    fn terminal_event_aligns_with_turn_finalize_end_hook_point() {
        let event_type =
            resolve_canonical_event_type("turn:completed", Some("completed"), None, None);

        assert_eq!(event_type, "turn.completed");
        assert!(crate::agent::hooks::hook_point_matches_canonical_boundary(
            &crate::agent::hooks::TurnHookPoint::TurnFinalizeEnd,
            &event_type,
            "completed"
        ));
    }
}

pub fn preview_text(text: &str, max_chars: usize) -> String {
    let normalized = text.replace('\n', "\\n");
    let count = normalized.chars().count();
    if count <= max_chars {
        normalized
    } else {
        let preview = normalized.chars().take(max_chars).collect::<String>();
        format!("{}...(+{} chars)", preview, count - max_chars)
    }
}

pub fn provider_decision<P: ProviderClient>(
    provider: &P,
    request: &ProviderRequest,
    tools: &[ToolDefinition],
) -> Result<ProviderDecision, String> {
    provider.decide_with_tools(request, tools)
}

pub fn provider_decision_stream<P, F>(
    provider: &P,
    request: &ProviderRequest,
    tools: &[ToolDefinition],
    on_delta: F,
) -> Result<ProviderDecision, String>
where
    P: ProviderClient,
    F: FnMut(ProviderStreamChunk),
{
    provider.decide_with_tools_stream(request, tools, on_delta)
}

pub fn provider_followup<P: ProviderClient>(
    provider: &P,
    request: &ProviderRequest,
    tools: &[ToolDefinition],
    accumulated_messages: &mut Vec<Value>,
    assistant_message: Option<&Value>,
    tool_call: &ToolCall,
    tool_result: &ToolResult,
) -> Result<crate::agent::provider::ProviderResponse, String> {
    provider.continue_with_tool_result(
        request,
        tools,
        accumulated_messages,
        assistant_message,
        tool_call,
        tool_result,
    )
}

pub fn provider_followup_stream<P, F>(
    provider: &P,
    request: &ProviderRequest,
    tools: &[ToolDefinition],
    accumulated_messages: &mut Vec<Value>,
    assistant_message: Option<&Value>,
    tool_call: &ToolCall,
    tool_result: &ToolResult,
    on_delta: F,
) -> Result<crate::agent::provider::ProviderResponse, String>
where
    P: ProviderClient,
    F: FnMut(ProviderStreamChunk),
{
    provider.continue_with_tool_result_stream(
        request,
        tools,
        accumulated_messages,
        assistant_message,
        tool_call,
        tool_result,
        on_delta,
    )
}
