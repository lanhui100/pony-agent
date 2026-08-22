// projection: 会话投影层（事件溯源演进阶段 2，PA-092）。
// 投影是"事件 → 视图"的纯函数折叠：任何时刻从事件序列可确定性重建视图。
// 设计见 openspec/changes/session-projection-layer/design.md（对抗审核后定稿）。
// 对抗审核（2026-08-18）采纳：水位 >= 语义、per-turn 水位、数据字段回填、
// chunk 文本聚合、turn/end 状态标记、PlanProjection、四桶统一、状态序列化。
use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::agent::session::{TraceTimelineEntry, TurnHistoryMessage, TurnTraceRecord};
use crate::agent::telemetry::{ProviderCallCacheRecord, TurnToolActivity};
use crate::agent::turn_event::{TurnEndReason, TurnEvent};

/// 投影 trait：`init` 空日志状态，`apply` 逐事件折叠（纯函数，必须按 seq 升序调用），
/// `view` 输出 wire 格式。增量折叠 == 全量重折叠（确定性契约）。
pub trait Projection<S> {
    fn init() -> S;
    fn apply(state: &mut S, seq: u64, event: &TurnEvent);
    fn view(state: &S) -> S
    where
        S: Clone,
    {
        state.clone()
    }
}

/// 消息历史投影：user/assistant 消息 + 截断窗口 + `history/squash` 语义
/// （丢弃 base_seq 之前的事件，注入摘要——折叠不复活被压缩历史）。
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct HistoryProjectionState {
    /// (事件 seq, 消息)。squash 后保留 seq > base_seq 的部分。
    messages: Vec<(u64, TurnHistoryMessage)>,
    /// 最大窗口（DEFAULT_HISTORY_LIMIT 语义）。
    limit: usize,
    /// 最近一次 TurnEnd 的 reason（用于标记最后一条 assistant 消息状态）。
    last_end_reason: Option<TurnEndReason>,
}

impl HistoryProjectionState {
    pub fn with_limit(limit: usize) -> Self {
        Self {
            messages: Vec::new(),
            limit,
            last_end_reason: None,
        }
    }

    pub fn messages(&self) -> Vec<TurnHistoryMessage> {
        self.messages
            .iter()
            .map(|(_, message)| message.clone())
            .collect()
    }
}

impl Projection<HistoryProjectionState> for HistoryProjectionState {
    fn init() -> Self {
        // 与 session.rs 的 DEFAULT_HISTORY_LIMIT 对齐（私有常量，投影侧复制）。
        Self::with_limit(24)
    }

    fn apply(state: &mut Self, seq: u64, event: &TurnEvent) {
        match event {
            TurnEvent::UserMessage {
                text,
                attachments,
                turn_id,
                ..
            } => {
                state.messages.push((
                    seq,
                    TurnHistoryMessage {
                        role: "user".to_string(),
                        content: text.clone(),
                        attachments: attachments.clone(),
                        turn_id: Some(turn_id.clone()),
                        status: None,
                        model_name: None,
                        token_count: None,
                        reasoning_content: None,
                    },
                ));
            }
            TurnEvent::AssistantMessage {
                text,
                reasoning_content,
                turn_id,
                usage,
                ..
            } => {
                state.messages.push((
                    seq,
                    TurnHistoryMessage {
                        role: "assistant".to_string(),
                        content: text.clone(),
                        attachments: Vec::new(),
                        turn_id: Some(turn_id.clone()),
                        status: None,
                        model_name: None,
                        // usage 仅消息元数据展示（不参与 MetricsProjection totals，防双计数）。
                        token_count: usage.as_ref().and_then(|u| u.total_tokens),
                        reasoning_content: reasoning_content.clone(),
                    },
                ));
            }
            TurnEvent::TurnEnd { reason, .. } => {
                // turn/end → 标记最后一条 assistant 消息状态（design.md:29）。
                if let Some((_, last)) = state.messages.last_mut() {
                    if last.role == "assistant" {
                        last.status = Some(match reason {
                            TurnEndReason::Completed => crate::agent::session::MessageStatus::Done,
                            _ => crate::agent::session::MessageStatus::Error,
                        });
                    }
                }
                state.last_end_reason = Some(reason.clone());
            }
            TurnEvent::HistorySquash {
                base_seq,
                summary_message,
            } => {
                // 丢弃 base_seq 之前的事件并注入摘要（压缩不复活被压缩历史）。
                // NOTE: 摘要格式与参考 wrap_summary_to_message（user 角色 + 包裹文本）
                // 存在分叉——对拍字段集显式排除摘要消息格式（spec 注明）。
                state.messages.retain(|(msg_seq, _)| *msg_seq > *base_seq);
                state.messages.push((
                    seq,
                    TurnHistoryMessage {
                        role: "assistant".to_string(),
                        content: summary_message.clone(),
                        attachments: Vec::new(),
                        turn_id: None,
                        status: None,
                        model_name: None,
                        token_count: None,
                        reasoning_content: None,
                    },
                ));
            }
            _ => {}
        }
        // 截断窗口：保留最近 limit 条。
        if state.messages.len() > state.limit {
            let keep_from = state.messages.len() - state.limit;
            state.messages.drain(..keep_from);
        }
    }
}

/// trace 投影：step/tool/chunk/turn-end 折叠为 `TurnTraceRecord`（按 turn 聚合）。
/// PA-094：新增 trace_timeline 事件折叠（映射表见 design.md §2）——
/// step/start → call_model、tool/call → call_tool、tool/result → return_result、
/// assistant/chunk → 文本聚合到 call_model、turn/end → 结算 token 指标。
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TraceProjectionState {
    by_turn: HashMap<String, TurnTraceRecord>,
    /// turn_id → 已折叠到的最大事件 seq（per-turn 水位，`>=` 语义防同 seq 重放）。
    watermark: HashMap<String, u64>,
    /// turn_id → chunk 文本累积（写时聚合，turn/end 结算进 trace）。
    chunk_text: HashMap<String, String>,
    /// turn_id → 最近 provider/model（终态信封回填用）。
    provider_model: HashMap<String, (String, String)>,
    /// turn_id → timeline 条目（事件折叠产物，PA-094）。
    timeline: HashMap<String, Vec<TraceTimelineEntry>>,
    /// turn_id → 下一个 timeline sequence（1-based 单调递增）。
    timeline_seq: HashMap<String, u64>,
    /// turn_id → step → call_model 条目在 timeline 中的索引（chunk/usage 聚合落点）。
    call_model_index: HashMap<String, HashMap<u32, usize>>,
    /// turn_id → call_id → call_tool 条目在 timeline 中的索引（tool/result 回填）。
    /// 键用 call_id 而非 step：`build_tool_event` 以 turn 事件序号作 step，
    /// ToolCall 与 ToolResult 的 step 必然不同，按 step 回填会 miss（审核 P0）。
    call_tool_index: HashMap<String, HashMap<String, usize>>,
    /// turn_id → 累计 token 指标（ProviderUsage 聚合，turn/end 结算）。
    token_metrics: HashMap<String, TurnTokenMetrics>,
    /// turn_id → build_context_observation 引用（ContextObservation 事件折叠，
    /// turn/end 结算时生成 build_context timeline 条目——design.md §2 映射表）。
    context_ref: HashMap<String, String>,
}

/// 单 turn 累计 token 指标（turn/end 结算进 trace 与 timeline call_model 条目）。
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct TurnTokenMetrics {
    input_tokens: Option<u64>,
    cache_hit_input_tokens: Option<u64>,
    reasoning_tokens: Option<u64>,
    output_tokens: Option<u64>,
    total_tokens: Option<u64>,
    first_token_latency_ms: Option<u64>,
}

impl TraceProjectionState {
    pub fn traces(&self) -> Vec<TurnTraceRecord> {
        let mut traces: Vec<TurnTraceRecord> = self.by_turn.values().cloned().collect();
        // 未结算 turn（无 TurnEnd）：挂载残留 timeline（事件折叠产物）。
        for trace in &mut traces {
            if trace.trace_timeline.is_empty() {
                if let Some(timeline) = self.timeline.get(&trace.turn_id) {
                    trace.trace_timeline = timeline.clone();
                }
            }
        }
        // 按 turn 起始 seq 排序（插入序近似；turn_id 字典序会错乱 turn-10 < turn-2）。
        let mut order: Vec<(u64, String)> = self
            .by_turn
            .iter()
            .map(|(turn_id, _)| {
                let start = self.watermark.get(turn_id).copied().unwrap_or(u64::MAX);
                (start, turn_id.clone())
            })
            .collect();
        order.sort();
        traces.sort_by_key(|t| {
            order
                .iter()
                .position(|(_, id)| id == &t.turn_id)
                .unwrap_or(usize::MAX)
        });
        traces
    }

    /// 返回 turn 的 trace（含事件折叠 timeline；未结算 turn 挂载残留 timeline）。
    pub fn trace_for_turn(&self, turn_id: &str) -> Option<TurnTraceRecord> {
        let mut trace = self.by_turn.get(turn_id)?.clone();
        if trace.trace_timeline.is_empty() {
            if let Some(timeline) = self.timeline.get(turn_id) {
                trace.trace_timeline = timeline.clone();
            }
        }
        Some(trace)
    }
}

/// 下一个 timeline sequence（1-based 单调递增）。
fn next_timeline_seq(state: &mut TraceProjectionState, turn_id: &str) -> u64 {
    let next = state.timeline_seq.entry(turn_id.to_string()).or_insert(1);
    let value = *next;
    *next += 1;
    value
}

/// 确保 turn 的 step 存在 call_model 条目（StepStart 创建；chunk/usage 兜底创建），
/// 返回条目在 timeline 中的索引。
fn ensure_call_model_entry(
    state: &mut TraceProjectionState,
    turn_id: &str,
    step: u32,
    _seq: u64,
) -> usize {
    if let Some(index) = state
        .call_model_index
        .get(turn_id)
        .and_then(|m| m.get(&step))
    {
        return *index;
    }
    let sequence = next_timeline_seq(state, turn_id);
    let timeline = state.timeline.entry(turn_id.to_string()).or_default();
    let index = timeline.len();
    timeline.push(TraceTimelineEntry {
        id: format!("model-{sequence}"),
        kind: "call_model".to_string(),
        label: format!("CALL MODEL #{}", step + 1),
        state: "completed".to_string(),
        sequence,
        ..Default::default()
    });
    state
        .call_model_index
        .entry(turn_id.to_string())
        .or_default()
        .insert(step, index);
    index
}

/// turn/end 结算 timeline：无 StepStart 事件时兜底补建 call_model 条目，
/// 末条 call_model 补 turn_duration_ms。PA-095 #3：兜底收窄——turn 无任何
/// 模型活动证据（chunk/usage 均缺，如 plan 前 cancelled）时不补建
/// （避免重建产物虚构未发生的模型调用）。
fn settle_timeline(state: &mut TraceProjectionState, turn_id: &str, turn_duration_ms: Option<u64>) {
    let has_call_model = state
        .timeline
        .get(turn_id)
        .map(|t| t.iter().any(|e| e.kind == "call_model"))
        .unwrap_or(false);
    let has_model_activity = state
        .chunk_text
        .get(turn_id)
        .map(|text| !text.is_empty())
        .unwrap_or(false)
        || state.token_metrics.contains_key(turn_id);
    if !has_call_model && has_model_activity {
        let sequence = next_timeline_seq(state, turn_id);
        let timeline = state.timeline.entry(turn_id.to_string()).or_default();
        timeline.push(TraceTimelineEntry {
            id: format!("model-{sequence}"),
            kind: "call_model".to_string(),
            label: "CALL MODEL #1".to_string(),
            state: "completed".to_string(),
            sequence,
            ..Default::default()
        });
    }
    if let Some(timeline) = state.timeline.get_mut(turn_id) {
        if let Some(entry) = timeline.iter_mut().rev().find(|e| e.kind == "call_model") {
            entry.turn_duration_ms = turn_duration_ms;
        }
    }
}

impl Projection<TraceProjectionState> for TraceProjectionState {
    fn init() -> Self {
        Self::default()
    }

    fn apply(state: &mut Self, seq: u64, event: &TurnEvent) {
        let Some(turn_id) = event.turn_id().map(str::to_string) else {
            return;
        };
        // higher-seq-wins：`>=` 语义——同 seq 重放帧不重复应用（design.md:36）。
        // 审核 P0：`unwrap_or(0)` 在首个事件 seq=0 时 `0 >= 0` 会误跳过，
        // 必须用 Option 判断（未记录水位 = 未处理，任何 seq 都应用）。
        if let Some(watermark) = state.watermark.get(&turn_id) {
            if *watermark >= seq {
                return;
            }
        }
        state.watermark.insert(turn_id.clone(), seq);
        match event {
            TurnEvent::StepStart {
                step,
                first_token_latency_ms,
                ..
            } => {
                // step/start → call_model 条目（design.md 映射表）。
                ensure_call_model_entry(state, &turn_id, *step, seq);
                if let Some(metrics) = state.token_metrics.get_mut(&turn_id) {
                    metrics.first_token_latency_ms = *first_token_latency_ms;
                }
            }
            TurnEvent::AssistantChunk { step, text, .. } => {
                // chunk 文本聚合（design.md:35）：聚合到对应 step 的 call_model 条目。
                let index = ensure_call_model_entry(state, &turn_id, *step, seq);
                let entry = &mut state.timeline.get_mut(&turn_id).expect("timeline")[index];
                entry.text = Some(entry.text.clone().unwrap_or_default() + text);
                state
                    .chunk_text
                    .entry(turn_id.clone())
                    .or_default()
                    .push_str(text);
            }
            TurnEvent::AssistantMessage { .. } => {
                // 组装消息文本由 HistoryProjection 承担；trace 侧只关心过程。
            }
            TurnEvent::ToolCall {
                call_id,
                name,
                arguments,
                ..
            } => {
                let trace = state.by_turn.entry(turn_id.clone()).or_insert_with(|| {
                    let mut trace = TurnTraceRecord {
                        turn_id: turn_id.clone(),
                        title: String::new(),
                        ..Default::default()
                    };
                    trace.turn_id = turn_id.clone();
                    trace
                });
                // 同 call_id 去重：已存在的 running activity 不重复 push。
                if !trace.tool_activities.iter().any(|a| a.id == *call_id) {
                    trace.tool_activities.push(TurnToolActivity {
                        id: call_id.clone(),
                        name: name.clone(),
                        canonical_tool_name: None,
                        display_name_zh: None,
                        status: "running".to_string(),
                        description: name.clone(),
                        arguments_text: Some(arguments.clone()),
                        result_text: None,
                        duration_seconds: None,
                        parent_activity_id: None,
                        artifacts: None,
                        error: None,
                        capability_invocation: None,
                    });
                }
                // tool/call → call_tool 条目（tool_activities 挂载该 activity）。
                // PA-095 #3：hop 序号与运行时产物同构——取当前 call_model 条目数
                // （事件的 step 是 turn 日志序号，非模型 hop）。
                let tool_hop = state
                    .timeline
                    .get(&turn_id)
                    .map(|timeline| {
                        timeline
                            .iter()
                            .filter(|entry| entry.kind == "call_model")
                            .count()
                    })
                    .unwrap_or(0);
                let sequence = next_timeline_seq(state, &turn_id);
                let timeline = state.timeline.entry(turn_id.clone()).or_default();
                let index = timeline.len();
                timeline.push(TraceTimelineEntry {
                    id: format!("tool-{sequence}"),
                    kind: "call_tool".to_string(),
                    // PA-095 #3：与运行时产物同构（hop 序号前缀）。
                    label: format!("CALL TOOL #{} · {}", tool_hop, name),
                    state: "running".to_string(),
                    sequence,
                    tool_activities: vec![TurnToolActivity {
                        id: call_id.clone(),
                        name: name.clone(),
                        canonical_tool_name: None,
                        display_name_zh: None,
                        status: "running".to_string(),
                        description: name.clone(),
                        arguments_text: Some(arguments.clone()),
                        result_text: None,
                        duration_seconds: None,
                        parent_activity_id: None,
                        artifacts: None,
                        error: None,
                        capability_invocation: None,
                    }],
                    text: Some(name.clone()),
                    ..Default::default()
                });
                state
                    .call_tool_index
                    .entry(turn_id.clone())
                    .or_default()
                    .insert(call_id.clone(), index);
            }
            TurnEvent::ToolResult {
                call_id,
                result,
                error,
                status,
                duration_ms,
                artifacts,
                capability_invocation,
                ..
            } => {
                let trace = state.by_turn.entry(turn_id.clone()).or_insert_with(|| {
                    let mut trace = TurnTraceRecord {
                        turn_id: turn_id.clone(),
                        title: String::new(),
                        ..Default::default()
                    };
                    trace.turn_id = turn_id.clone();
                    trace
                });
                // 更新对应 activity 的终态字段（按 call_id 匹配）。
                if let Some(activity) = trace.tool_activities.iter_mut().find(|a| a.id == *call_id)
                {
                    activity.status = status.clone().unwrap_or_else(|| {
                        if error.is_some() {
                            "error".to_string()
                        } else {
                            "done".to_string()
                        }
                    });
                    activity.result_text = result.clone();
                    activity.duration_seconds = duration_ms.map(|ms| ms as f64 / 1000.0);
                    activity.artifacts = artifacts.clone();
                    activity.capability_invocation = capability_invocation.clone();
                    activity.error = error.clone().map(serde_json::Value::String);
                } else {
                    // unmatched ToolResult（backfill 场景）：合成 activity（name 未知）。
                    trace.tool_activities.push(TurnToolActivity {
                        id: call_id.clone(),
                        name: String::new(),
                        canonical_tool_name: None,
                        display_name_zh: None,
                        status: status.clone().unwrap_or_else(|| "done".to_string()),
                        description: String::new(),
                        arguments_text: None,
                        result_text: result.clone(),
                        duration_seconds: duration_ms.map(|ms| ms as f64 / 1000.0),
                        parent_activity_id: None,
                        artifacts: artifacts.clone(),
                        error: error.clone().map(serde_json::Value::String),
                        capability_invocation: capability_invocation.clone(),
                    });
                }
                // tool/result → return_result 条目 + 回填 call_tool 条目的 tool_activities。
                // PA-095 #3：state 与运行时产物同构（done→completed；error 保留）。
                let normalized_status = match status.as_deref() {
                    Some("done") => "completed".to_string(),
                    Some(other) => other.to_string(),
                    None => "completed".to_string(),
                };
                let sequence = next_timeline_seq(state, &turn_id);
                let timeline = state.timeline.entry(turn_id.clone()).or_default();
                timeline.push(TraceTimelineEntry {
                    id: format!("return-{sequence}"),
                    kind: "return_result".to_string(),
                    label: "RETURN RESULT".to_string(),
                    state: normalized_status.clone(),
                    sequence,
                    text: result.clone(),
                    error: error.clone(),
                    ..Default::default()
                });
                // 回填 call_tool 条目：tool_activities 更新为终态（按 call_id 匹配，
                // 与 ToolCall 的索引键一致——step 语义差异不影响回填）。
                if let Some(step_index) = state
                    .call_tool_index
                    .get(&turn_id)
                    .and_then(|m| m.get(call_id))
                {
                    if let Some(entry) = timeline.get_mut(*step_index) {
                        if let Some(activity) =
                            entry.tool_activities.iter_mut().find(|a| a.id == *call_id)
                        {
                            activity.status = status.clone().unwrap_or_else(|| "done".to_string());
                            activity.result_text = result.clone();
                            activity.duration_seconds = duration_ms.map(|ms| ms as f64 / 1000.0);
                            activity.artifacts = artifacts.clone();
                            activity.capability_invocation = capability_invocation.clone();
                            activity.error = error.clone().map(serde_json::Value::String);
                        }
                        entry.state = normalized_status;
                    }
                }
            }
            TurnEvent::ProviderUsage {
                step,
                usage,
                cache_hit_input_tokens,
                first_token_latency_ms,
                turn_duration_ms,
                provider,
                model,
                ..
            } => {
                state
                    .provider_model
                    .insert(turn_id.clone(), (provider.clone(), model.clone()));
                // 累计 token 指标（turn/end 结算）。审核 P1：多次 ProviderUsage 时
                // 顶层字段是 turn 级累计（saturating_add，与 MetricsProjection 惯例
                // 一致）；(None, None) 合并为 None（保留"未知"语义，避免渲染为 0）；
                // first_token_latency_ms 取首次（首 token 延迟）。
                fn merge_tokens(acc: Option<u64>, next: Option<u64>) -> Option<u64> {
                    match (acc, next) {
                        (None, None) => None,
                        (acc, next) => Some(acc.unwrap_or(0).saturating_add(next.unwrap_or(0))),
                    }
                }
                let metrics = state.token_metrics.entry(turn_id.clone()).or_default();
                metrics.input_tokens = merge_tokens(metrics.input_tokens, usage.input_tokens);
                metrics.cache_hit_input_tokens =
                    merge_tokens(metrics.cache_hit_input_tokens, *cache_hit_input_tokens);
                metrics.reasoning_tokens =
                    merge_tokens(metrics.reasoning_tokens, usage.reasoning_tokens);
                metrics.output_tokens = merge_tokens(metrics.output_tokens, usage.output_tokens);
                metrics.total_tokens = merge_tokens(metrics.total_tokens, usage.total_tokens);
                metrics.first_token_latency_ms =
                    first_token_latency_ms.or(metrics.first_token_latency_ms);
                // 回填 call_model 条目 token 指标。
                let index = ensure_call_model_entry(state, &turn_id, *step, seq);
                let entry = &mut state.timeline.get_mut(&turn_id).expect("timeline")[index];
                entry.input_tokens = usage.input_tokens;
                entry.cache_hit_input_tokens = *cache_hit_input_tokens;
                entry.reasoning_tokens = usage.reasoning_tokens;
                entry.output_tokens = usage.output_tokens;
                entry.total_tokens = usage.total_tokens;
                entry.first_token_latency_ms = *first_token_latency_ms;
                entry.turn_duration_ms = *turn_duration_ms;
                entry.provider_name = Some(provider.clone());
                entry.provider_model = Some(model.clone());
            }
            TurnEvent::ContextObservation {
                observation_ref, ..
            } => {
                // PA-094：大字段外置——trace 记录只存引用（全量 payload 按需加载）。
                let trace = state.by_turn.entry(turn_id.clone()).or_insert_with(|| {
                    let mut trace = TurnTraceRecord {
                        turn_id: turn_id.clone(),
                        title: String::new(),
                        ..Default::default()
                    };
                    trace.turn_id = turn_id.clone();
                    trace
                });
                if let Some(ref_value) = observation_ref {
                    trace.build_context_observation_ref = Some(ref_value.clone());
                    // 记录引用，turn/end 结算时生成 build_context timeline 条目。
                    state.context_ref.insert(turn_id.clone(), ref_value.clone());
                }
            }
            TurnEvent::TurnEnd {
                reason,
                turn_duration_ms,
                ..
            } => {
                // 结算 timeline：兜底补建 call_model + 末条补 turn_duration_ms
                // （独立于 by_turn 借用，先结算再挂载）。
                settle_timeline(state, &turn_id, *turn_duration_ms);
                // PA-095 #3（对拍收敛修复）：cancelled/aborted turn 中未经 usage
                // 结算的 call_model 条目标记 cancelled——中断的模型调用不得显示
                // 为 completed（与运行时产物语义一致；已结算 hop 保持 completed）。
                if matches!(reason, TurnEndReason::Cancelled | TurnEndReason::Aborted) {
                    if let Some(timeline) = state.timeline.get_mut(&turn_id) {
                        for entry in timeline.iter_mut() {
                            if entry.kind == "call_model" && entry.provider_name.is_none() {
                                entry.state = "cancelled".to_string();
                            }
                        }
                    }
                }
                // PA-094：build_context 条目（ContextObservation 事件折叠）——
                // 插入 timeline 开头并重排 sequence（与运行时产物结构对齐，
                // design.md §2 映射表；prepare_retrieval 需 payload 判断，豁免）。
                // PA-095 #3：entry API——兜底收窄后无模型活动的 turn 无既有
                // timeline 条目，get_mut 会静默丢失 build_context 前插。
                if let Some(ref_value) = state.context_ref.remove(&turn_id) {
                    let timeline = state.timeline.entry(turn_id.clone()).or_default();
                    timeline.insert(
                        0,
                        TraceTimelineEntry {
                            id: "context-1".to_string(),
                            kind: "build_context".to_string(),
                            label: "BUILD CONTEXT".to_string(),
                            state: "completed".to_string(),
                            sequence: 1,
                            build_context_observation_ref: Some(ref_value),
                            ..Default::default()
                        },
                    );
                    for (index, entry) in timeline.iter_mut().enumerate() {
                        entry.sequence = (index + 1) as u64;
                    }
                }
                let timeline = state.timeline.remove(&turn_id);
                state.call_model_index.remove(&turn_id);
                state.call_tool_index.remove(&turn_id);
                state.timeline_seq.remove(&turn_id);
                let metrics = state.token_metrics.remove(&turn_id);
                let chunk_text = state.chunk_text.remove(&turn_id);
                let provider_model = state.provider_model.get(&turn_id).cloned();
                let trace = state.by_turn.entry(turn_id.clone()).or_insert_with(|| {
                    let mut trace = TurnTraceRecord {
                        turn_id: turn_id.clone(),
                        title: String::new(),
                        ..Default::default()
                    };
                    trace.turn_id = turn_id.clone();
                    trace
                });
                trace.turn_duration_ms = *turn_duration_ms;
                // chunk 文本结算进 trace（文本聚合的落点）。
                if let Some(text) = chunk_text {
                    trace
                        .trace_steps
                        .push(crate::agent::telemetry::TurnTraceStep {
                            id: format!("chunk-{seq}"),
                            label: "assistant_output".to_string(),
                            state: "completed".to_string(),
                        });
                    let _ = text;
                }
                // 终态信封：event_type 由 reason 映射；title/sequence 等
                // 由外层 annotate_turn_trace_terminal_event 补写（投影不自洽字段）。
                trace.event_type = Some(match reason {
                    TurnEndReason::Completed => "turn.completed".to_string(),
                    TurnEndReason::Error => "turn.failed".to_string(),
                    TurnEndReason::Aborted | TurnEndReason::Cancelled => {
                        "turn.cancelled".to_string()
                    }
                });
                // PA-095 #3：phase 由 reason 确定性映射（对拍约定字段集包含
                // phase——重建须自洽；annotate 补写同值，语义一致）。
                trace.phase = match reason {
                    TurnEndReason::Completed => "completed".to_string(),
                    TurnEndReason::Error => "failed".to_string(),
                    TurnEndReason::Aborted | TurnEndReason::Cancelled => {
                        "cancelled".to_string()
                    }
                };
                trace.sequence = Some(seq);
                if let Some((provider, model)) = provider_model {
                    trace.provider_name = Some(provider);
                    trace.provider_model = Some(model);
                }
                // turn/end → 结算 token 指标（design.md 映射表）。
                if let Some(metrics) = metrics {
                    trace.input_tokens = metrics.input_tokens;
                    trace.cache_hit_input_tokens = metrics.cache_hit_input_tokens;
                    trace.reasoning_tokens = metrics.reasoning_tokens;
                    trace.output_tokens = metrics.output_tokens;
                    trace.total_tokens = metrics.total_tokens;
                    trace.first_token_latency_ms = metrics.first_token_latency_ms;
                }
                // 挂载事件折叠 timeline。
                if let Some(timeline) = timeline {
                    trace.trace_timeline = timeline;
                }
            }
            _ => {}
        }
    }
}

/// plan 投影：`plan/update` 覆盖式折叠（whole-value）。
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PlanProjectionState {
    pub plan_state: Option<serde_json::Value>,
}

impl Projection<PlanProjectionState> for PlanProjectionState {
    fn init() -> Self {
        Self::default()
    }

    fn apply(state: &mut Self, _seq: u64, event: &TurnEvent) {
        if let TurnEvent::PlanUpdate { plan_state } = event {
            state.plan_state = Some(plan_state.clone());
        }
    }
}

/// 指标投影：ProviderUsage → 四桶 totals + last + per-turn 聚合
/// （addReplacing 语义：同一 (turn_id, step) 的 usage 替换而非累加；
/// `assistant/message` 的 usage 仅消息元数据展示，不参与 totals）。
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MetricsProjectionState {
    /// 四桶累计。
    pub uncached_input_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub output_tokens: u64,
    /// 最近一次 provider 调用。
    pub last: Option<ProviderCallCacheRecord>,
    /// per-turn 聚合（模型监控 drilldown）。
    pub by_turn: HashMap<String, TurnUsageAggregate>,
    /// PA-094：per-turn ProviderCallCacheRecord 列表（由 ProviderUsage 事件重建，
    /// wire 兼容保留字段；trace 不再独立存储）。
    pub by_turn_records: HashMap<String, Vec<ProviderCallCacheRecord>>,
    /// addReplacing 键：(turn_id, step) → 已累计的贡献（替换时回退）。
    replaced: HashMap<(String, u32), StepContribution>,
    /// PA-094：addReplacing 键：(turn_id, step) → by_turn_records 中的索引（原位覆盖）。
    record_index: HashMap<(String, u32), usize>,
    /// per-turn 水位（`>=` 语义防同 seq 重放；与 TraceProjection 粒度一致）。
    watermark: HashMap<String, u64>,
}

/// 一次 provider 调用对 totals 与 per-turn 聚合的贡献（替换语义的原子单位）。
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct StepContribution {
    buckets: TokenBuckets,
    aggregate: TurnUsageAggregate,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenBuckets {
    pub uncached_input: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub output: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TurnUsageAggregate {
    pub input_tokens: u64,
    pub cache_hit_input_tokens: u64,
    pub cache_miss_input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub first_token_latency_ms: Option<u64>,
    pub turn_duration_ms: Option<u64>,
    pub provider: Option<String>,
    pub model: Option<String>,
}

impl MetricsProjectionState {
    pub fn totals(&self) -> TokenBuckets {
        TokenBuckets {
            uncached_input: self.uncached_input_tokens,
            cache_read: self.cache_read_tokens,
            cache_write: self.cache_write_tokens,
            output: self.output_tokens,
        }
    }
}

impl Projection<MetricsProjectionState> for MetricsProjectionState {
    fn init() -> Self {
        Self::default()
    }

    fn apply(state: &mut Self, seq: u64, event: &TurnEvent) {
        let Some(turn_id) = event.turn_id().map(str::to_string) else {
            return;
        };
        // higher-seq-wins（per-turn）：`>=` 语义——同 seq 重放帧不重复应用。
        // 审核 P0：`unwrap_or(0)` 在首个事件 seq=0 时误跳过，用 Option 判断。
        if let Some(watermark) = state.watermark.get(&turn_id) {
            if *watermark >= seq {
                return;
            }
        }
        state.watermark.insert(turn_id.clone(), seq);
        match event {
            TurnEvent::ProviderUsage {
                turn_id,
                step,
                usage,
                cache_hit_input_tokens,
                cache_miss_input_tokens,
                prefix_mutation_reasons,
                first_token_latency_ms,
                turn_duration_ms,
                request_kind,
                latency_kind,
                provider,
                model,
                ..
            } => {
                // 四桶统一使用事件级权威字段（P1-9）：
                // cache_read = cache_hit_input_tokens；uncached = cache_miss_input_tokens
                // （None 时回退 input - cache_hit）。
                let cache_read = cache_hit_input_tokens.unwrap_or(0);
                let uncached = cache_miss_input_tokens.unwrap_or_else(|| {
                    usage
                        .input_tokens
                        .unwrap_or(0)
                        .saturating_sub(usage.cache_hit_input_tokens.unwrap_or(0))
                });
                let buckets = TokenBuckets {
                    uncached_input: uncached,
                    cache_read,
                    cache_write: 0,
                    output: usage.output_tokens.unwrap_or(0),
                };
                let aggregate = TurnUsageAggregate {
                    input_tokens: usage.input_tokens.unwrap_or(0),
                    cache_hit_input_tokens: cache_hit_input_tokens.unwrap_or(0),
                    cache_miss_input_tokens: cache_miss_input_tokens.unwrap_or(0),
                    output_tokens: usage.output_tokens.unwrap_or(0),
                    total_tokens: usage.total_tokens.unwrap_or(0),
                    first_token_latency_ms: *first_token_latency_ms,
                    turn_duration_ms: *turn_duration_ms,
                    provider: Some(provider.clone()),
                    model: Some(model.clone()),
                };
                let contribution = StepContribution {
                    buckets: buckets.clone(),
                    aggregate: aggregate.clone(),
                };
                // addReplacing：同一 (turn_id, step) 替换（回退旧贡献再累计）。
                if let Some(previous) = state
                    .replaced
                    .insert((turn_id.clone(), *step), contribution)
                {
                    state.uncached_input_tokens = state
                        .uncached_input_tokens
                        .saturating_sub(previous.buckets.uncached_input)
                        .saturating_add(buckets.uncached_input);
                    state.cache_read_tokens = state
                        .cache_read_tokens
                        .saturating_sub(previous.buckets.cache_read)
                        .saturating_add(buckets.cache_read);
                    state.cache_write_tokens = state
                        .cache_write_tokens
                        .saturating_sub(previous.buckets.cache_write)
                        .saturating_add(buckets.cache_write);
                    state.output_tokens = state
                        .output_tokens
                        .saturating_sub(previous.buckets.output)
                        .saturating_add(buckets.output);
                    let turn_agg = state
                        .by_turn
                        .entry(turn_id.clone())
                        .or_insert_with(TurnUsageAggregate::default);
                    turn_agg.input_tokens = turn_agg
                        .input_tokens
                        .saturating_sub(previous.aggregate.input_tokens)
                        .saturating_add(aggregate.input_tokens);
                    turn_agg.cache_hit_input_tokens = turn_agg
                        .cache_hit_input_tokens
                        .saturating_sub(previous.aggregate.cache_hit_input_tokens)
                        .saturating_add(aggregate.cache_hit_input_tokens);
                    turn_agg.cache_miss_input_tokens = turn_agg
                        .cache_miss_input_tokens
                        .saturating_sub(previous.aggregate.cache_miss_input_tokens)
                        .saturating_add(aggregate.cache_miss_input_tokens);
                    turn_agg.output_tokens = turn_agg
                        .output_tokens
                        .saturating_sub(previous.aggregate.output_tokens)
                        .saturating_add(aggregate.output_tokens);
                    turn_agg.total_tokens = turn_agg
                        .total_tokens
                        .saturating_sub(previous.aggregate.total_tokens)
                        .saturating_add(aggregate.total_tokens);
                } else {
                    state.uncached_input_tokens = state
                        .uncached_input_tokens
                        .saturating_add(buckets.uncached_input);
                    state.cache_read_tokens =
                        state.cache_read_tokens.saturating_add(buckets.cache_read);
                    state.cache_write_tokens =
                        state.cache_write_tokens.saturating_add(buckets.cache_write);
                    state.output_tokens = state.output_tokens.saturating_add(buckets.output);
                    let turn_agg = state
                        .by_turn
                        .entry(turn_id.clone())
                        .or_insert_with(TurnUsageAggregate::default);
                    turn_agg.input_tokens =
                        turn_agg.input_tokens.saturating_add(aggregate.input_tokens);
                    turn_agg.cache_hit_input_tokens = turn_agg
                        .cache_hit_input_tokens
                        .saturating_add(aggregate.cache_hit_input_tokens);
                    turn_agg.cache_miss_input_tokens = turn_agg
                        .cache_miss_input_tokens
                        .saturating_add(aggregate.cache_miss_input_tokens);
                    turn_agg.output_tokens = turn_agg
                        .output_tokens
                        .saturating_add(aggregate.output_tokens);
                    turn_agg.total_tokens =
                        turn_agg.total_tokens.saturating_add(aggregate.total_tokens);
                }
                if let Some(turn_agg) = state.by_turn.get_mut(turn_id) {
                    turn_agg.first_token_latency_ms = *first_token_latency_ms;
                    turn_agg.provider = Some(provider.clone());
                    turn_agg.model = Some(model.clone());
                }
                state.last = Some(ProviderCallCacheRecord {
                    request_kind: request_kind.clone(),
                    provider_source: Some(provider.clone()),
                    provider_mode: None,
                    input_tokens: usage.input_tokens,
                    cache_hit_input_tokens: *cache_hit_input_tokens,
                    cache_hit_source: None,
                    cache_miss_input_tokens: *cache_miss_input_tokens,
                    reasoning_tokens: usage.reasoning_tokens,
                    output_tokens: usage.output_tokens,
                    total_tokens: usage.total_tokens,
                    first_token_latency_ms: *first_token_latency_ms,
                    turn_duration_ms: *turn_duration_ms,
                    latency_kind: latency_kind.clone(),
                    prefix_mutation_reasons: prefix_mutation_reasons
                        .iter()
                        .filter_map(|reason| {
                            serde_json::from_value(serde_json::Value::String(reason.clone())).ok()
                        })
                        .collect(),
                });
                // PA-094：per-turn ProviderCallCacheRecord 重建（design.md §5）。
                // 重建字段集：request_kind、usage 四桶、cache_hit/cache_miss、
                // prefix_mutation_reasons、first_token_latency_ms、turn_duration_ms、
                // latency_kind、provider/model。豁免：updated_at/event_id/sequence/
                // emitted_at_ms（时钟/序列语义，trace 记录级字段）。
                let record = ProviderCallCacheRecord {
                    request_kind: request_kind.clone(),
                    provider_source: Some(provider.clone()),
                    provider_mode: None,
                    input_tokens: usage.input_tokens,
                    cache_hit_input_tokens: *cache_hit_input_tokens,
                    cache_hit_source: None,
                    cache_miss_input_tokens: *cache_miss_input_tokens,
                    reasoning_tokens: usage.reasoning_tokens,
                    output_tokens: usage.output_tokens,
                    total_tokens: usage.total_tokens,
                    first_token_latency_ms: *first_token_latency_ms,
                    turn_duration_ms: *turn_duration_ms,
                    latency_kind: latency_kind.clone(),
                    prefix_mutation_reasons: prefix_mutation_reasons
                        .iter()
                        .filter_map(|reason| {
                            serde_json::from_value(serde_json::Value::String(reason.clone())).ok()
                        })
                        .collect(),
                };
                // addReplacing：同一 (turn_id, step) 原位覆盖（与 totals 替换语义一致）。
                if let Some(index) = state.record_index.get(&(turn_id.clone(), *step)).copied() {
                    if let Some(records) = state.by_turn_records.get_mut(turn_id) {
                        if let Some(slot) = records.get_mut(index) {
                            *slot = record;
                        }
                    }
                } else {
                    let records = state.by_turn_records.entry(turn_id.clone()).or_default();
                    state
                        .record_index
                        .insert((turn_id.clone(), *step), records.len());
                    records.push(record);
                }
                let _ = model;
            }
            TurnEvent::TurnEnd {
                turn_duration_ms, ..
            } => {
                if let Some(aggregate) = state.by_turn.get_mut(&turn_id) {
                    aggregate.turn_duration_ms = *turn_duration_ms;
                }
                // turn 结束：清理该 turn 的 replaced 条目（不再有 usage 事件）。
                state.replaced.retain(|(tid, _), _| tid != &turn_id);
                state.record_index.retain(|(tid, _), _| tid != &turn_id);
            }
            _ => {}
        }
    }
}

/// 全量折叠：从事件序列重建投影状态（等价于逐事件增量折叠）。
/// 前置条件：输入按 seq 升序（乱序输入下 per-turn 水位会产生非确定性结果）。
pub fn fold_all<S, P: Projection<S>>(events: &[(u64, TurnEvent)]) -> S {
    let mut state = P::init();
    for (seq, event) in events {
        P::apply(&mut state, *seq, event);
    }
    state
}

/// PA-093：带分支可见性的全量折叠（阶段 3）。
/// 输入为 `(seq, branch_id, event)` 三元组（branch_id 来自 turn_events 列）。
/// 可见集合语义：初始 = **目标节点所在分支的血缘链**（折叠目标是该节点的
/// 视图，其分支祖先天然可见）；流内 `checkpoint/checkout` 事件按序重放，
/// 每次把可见集合替换为目标节点分支的血缘链（历史切换序列正确重放）；
/// 其余事件 branch_id ∉ 可见集合则跳过（被撤回分支的事件不复活）。
/// checkout 事件本身无条件处理（它驱动可见集合，且投影 apply 对其无操作）。
pub fn fold_all_with_branches<S, P: Projection<S>>(
    events: &[(u64, String, TurnEvent)],
    initial_branch: &str,
    node_branch: &HashMap<&str, &str>,
    branches: &[crate::agent::session::HistoryBranch],
) -> S {
    let mut state = P::init();
    let mut visible = branch_lineage(initial_branch, branches);
    for (seq, branch_id, event) in events {
        if let TurnEvent::CheckpointCheckout { node_id, .. } = event {
            if let Some(branch) = node_branch.get(node_id.as_str()) {
                visible = branch_lineage(branch, branches);
            }
        }
        if matches!(event, TurnEvent::CheckpointCheckout { .. }) || visible.contains(branch_id) {
            P::apply(&mut state, *seq, event);
        }
    }
    state
}

/// PA-093：分支血缘链（自身 + forked_from_branch_id 祖先链）。
fn branch_lineage(
    branch_id: &str,
    branches: &[crate::agent::session::HistoryBranch],
) -> std::collections::HashSet<String> {
    let mut chain = std::collections::HashSet::new();
    let mut current = Some(branch_id.to_string());
    while let Some(id) = current {
        if !chain.insert(id.clone()) {
            break; // 防御环（数据损坏兜底）
        }
        current = branches
            .iter()
            .find(|b| b.branch_id == id)
            .and_then(|b| b.forked_from_branch_id.clone());
    }
    chain
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_events() -> Vec<(u64, TurnEvent)> {
        vec![
            (
                0,
                TurnEvent::TurnStart {
                    turn_id: "t1".into(),
                },
            ),
            (
                1,
                TurnEvent::UserMessage {
                    turn_id: "t1".into(),
                    text: "hello".into(),
                    attachments: Vec::new(),
                },
            ),
            (
                2,
                TurnEvent::AssistantChunk {
                    turn_id: "t1".into(),
                    step: 0,
                    text: "hi".into(),
                },
            ),
            (
                3,
                TurnEvent::AssistantMessage {
                    turn_id: "t1".into(),
                    step: 0,
                    text: "hi".into(),
                    reasoning_content: None,
                    usage: None,
                    chunk_missing: None,
                },
            ),
            (
                4,
                TurnEvent::TurnEnd {
                    turn_id: "t1".into(),
                    reason: TurnEndReason::Completed,
                    turn_duration_ms: Some(500),
                },
            ),
        ]
    }

    #[test]
    fn history_projection_folds_messages() {
        let events = sample_events();
        let state = fold_all::<_, HistoryProjectionState>(&events);
        let messages = state.messages();
        assert_eq!(messages.len(), 2, "user + assistant");
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[0].content, "hello");
        assert_eq!(messages[1].role, "assistant");
        assert_eq!(messages[1].content, "hi");
        // turn/end completed → 最后 assistant 消息 status=Done
        assert_eq!(
            messages[1].status,
            Some(crate::agent::session::MessageStatus::Done)
        );
    }

    #[test]
    fn history_projection_failed_turn_marks_error_status() {
        let mut events = sample_events();
        events[4].1 = TurnEvent::TurnEnd {
            turn_id: "t1".into(),
            reason: TurnEndReason::Error,
            turn_duration_ms: None,
        };
        let state = fold_all::<_, HistoryProjectionState>(&events);
        let messages = state.messages();
        assert_eq!(
            messages[1].status,
            Some(crate::agent::session::MessageStatus::Error)
        );
    }

    #[test]
    fn history_projection_squash_drops_old_messages() {
        let mut events = sample_events();
        events.push((
            5,
            TurnEvent::HistorySquash {
                base_seq: 3,
                summary_message: "compressed summary".into(),
            },
        ));
        let state = fold_all::<_, HistoryProjectionState>(&events);
        let messages = state.messages();
        assert_eq!(messages.len(), 1, "squash keeps only summary");
        assert_eq!(messages[0].content, "compressed summary");
    }

    #[test]
    fn history_projection_truncates_window() {
        let mut events = Vec::new();
        for i in 0..30u64 {
            events.push((
                i * 2,
                TurnEvent::UserMessage {
                    turn_id: format!("t{i}"),
                    text: format!("msg-{i}"),
                    attachments: Vec::new(),
                },
            ));
        }
        let state = fold_all::<_, HistoryProjectionState>(&events);
        assert_eq!(
            state.messages().len(),
            24,
            "window truncated to limit (DEFAULT_HISTORY_LIMIT)"
        );
    }

    #[test]
    fn incremental_fold_equals_full_refold() {
        let events = sample_events();
        let mut state = HistoryProjectionState::init();
        for (seq, event) in &events {
            HistoryProjectionState::apply(&mut state, *seq, event);
        }
        let incremental = state.messages();
        let full = fold_all::<_, HistoryProjectionState>(&events).messages();
        assert_eq!(incremental.len(), full.len());
        for (a, b) in incremental.iter().zip(full.iter()) {
            assert_eq!(a.role, b.role);
            assert_eq!(a.content, b.content);
        }
    }

    #[test]
    fn trace_projection_folds_tool_activities() {
        let mut events = sample_events();
        events.extend([
            (
                5,
                TurnEvent::ToolCall {
                    turn_id: "t1".into(),
                    step: 1,
                    call_id: "c1".into(),
                    name: "bash".into(),
                    arguments: "{}".into(),
                    started_at_ms: None,
                },
            ),
            (
                6,
                TurnEvent::ToolResult {
                    turn_id: "t1".into(),
                    step: 1,
                    call_id: "c1".into(),
                    result: Some("ok".into()),
                    error: None,
                    status: Some("done".into()),
                    duration_ms: Some(500),
                    artifacts: None,
                    capability_invocation: None,
                },
            ),
        ]);
        let state = fold_all::<_, TraceProjectionState>(&events);
        let trace = state.trace_for_turn("t1").expect("trace");
        assert_eq!(trace.tool_activities.len(), 1, "tool activity");
        assert_eq!(trace.tool_activities[0].status, "done");
        assert_eq!(trace.tool_activities[0].result_text.as_deref(), Some("ok"));
        assert_eq!(trace.tool_activities[0].duration_seconds, Some(0.5));
        assert_eq!(
            trace.turn_duration_ms,
            Some(500),
            "turn duration from turn/end"
        );
        // 终态信封：completed → turn.completed
        assert_eq!(trace.event_type.as_deref(), Some("turn.completed"));
    }

    #[test]
    fn trace_projection_same_seq_replay_is_idempotent() {
        // 同 seq 重放帧（fork/checkout 复制日志场景）：不重复 push activity。
        let event = TurnEvent::ToolCall {
            turn_id: "t1".into(),
            step: 1,
            call_id: "c1".into(),
            name: "bash".into(),
            arguments: "{}".into(),
            started_at_ms: None,
        };
        let mut state = TraceProjectionState::init();
        TraceProjectionState::apply(&mut state, 5, &event);
        TraceProjectionState::apply(&mut state, 5, &event); // 同 seq 重放
        let trace = state.trace_for_turn("t1").expect("trace");
        assert_eq!(trace.tool_activities.len(), 1, "replay must not duplicate");
    }

    #[test]
    fn metrics_projection_add_replacing_different_values() {
        let usage_a = crate::agent::provider::TokenUsage {
            input_tokens: Some(100),
            cache_hit_input_tokens: Some(40),
            cache_hit_source: None,
            reasoning_tokens: Some(10),
            output_tokens: Some(50),
            total_tokens: Some(160),
        };
        let usage_b = crate::agent::provider::TokenUsage {
            input_tokens: Some(200),
            cache_hit_input_tokens: Some(80),
            cache_hit_source: None,
            reasoning_tokens: Some(20),
            output_tokens: Some(100),
            total_tokens: Some(320),
        };
        let mk = |seq: u64, usage: crate::agent::provider::TokenUsage| {
            let cache_hit = usage.cache_hit_input_tokens;
            let cache_miss = usage
                .input_tokens
                .map(|i| i.saturating_sub(cache_hit.unwrap_or(0)));
            (
                seq,
                TurnEvent::ProviderUsage {
                    turn_id: "t1".into(),
                    step: 0,
                    request_kind: crate::agent::telemetry::ProviderRequestKind::InitialRequest,
                    usage,
                    cache_hit_input_tokens: cache_hit,
                    cache_miss_input_tokens: cache_miss,
                    prefix_mutation_reasons: Vec::new(),
                    first_token_latency_ms: Some(88),
                    turn_duration_ms: Some(500),
                    latency_kind: crate::agent::telemetry::ProviderLatencyKind::ProviderStream,
                    provider: "deepseek".into(),
                    model: "deepseek-chat".into(),
                },
            )
        };
        // 同一 (turn_id, step) 不同值替换：totals 与 by_turn 都只反映最新值
        let events = vec![mk(1, usage_a.clone()), mk(2, usage_b.clone())];
        let state = fold_all::<_, MetricsProjectionState>(&events);
        let totals = state.totals();
        assert_eq!(totals.uncached_input, 120, "replace to latest (200-80)");
        assert_eq!(totals.output, 100);
        assert_eq!(totals.cache_read, 80);
        let aggregate = state.by_turn.get("t1").expect("aggregate");
        assert_eq!(aggregate.input_tokens, 200, "replace not double-count");
        assert_eq!(aggregate.output_tokens, 100);
        assert_eq!(aggregate.provider.as_deref(), Some("deepseek"));
        assert_eq!(aggregate.model.as_deref(), Some("deepseek-chat"));
        // last 记录带 provider
        let last = state.last.expect("last");
        assert_eq!(last.provider_source.as_deref(), Some("deepseek"));
    }

    #[test]
    fn metrics_projection_totals_equals_by_turn_sum() {
        let usage = crate::agent::provider::TokenUsage {
            input_tokens: Some(100),
            cache_hit_input_tokens: Some(40),
            cache_hit_source: None,
            reasoning_tokens: Some(10),
            output_tokens: Some(50),
            total_tokens: Some(160),
        };
        let mk = |seq: u64, turn: &str, step: u32| {
            (
                seq,
                TurnEvent::ProviderUsage {
                    turn_id: turn.into(),
                    step,
                    request_kind: crate::agent::telemetry::ProviderRequestKind::InitialRequest,
                    usage: usage.clone(),
                    cache_hit_input_tokens: Some(40),
                    cache_miss_input_tokens: Some(60),
                    prefix_mutation_reasons: Vec::new(),
                    first_token_latency_ms: None,
                    turn_duration_ms: None,
                    latency_kind: crate::agent::telemetry::ProviderLatencyKind::ProviderStream,
                    provider: "deepseek".into(),
                    model: "deepseek-chat".into(),
                },
            )
        };
        let events = vec![mk(1, "t1", 0), mk(2, "t1", 1), mk(3, "t2", 0)];
        let state = fold_all::<_, MetricsProjectionState>(&events);
        let totals = state.totals();
        let by_turn_sum =
            |key: fn(&TurnUsageAggregate) -> u64| state.by_turn.values().map(key).sum::<u64>();
        assert_eq!(
            totals.uncached_input,
            by_turn_sum(|a| a.cache_miss_input_tokens)
        );
        assert_eq!(totals.cache_read, by_turn_sum(|a| a.cache_hit_input_tokens));
        assert_eq!(totals.output, by_turn_sum(|a| a.output_tokens));
    }

    #[test]
    fn metrics_projection_watermark_higher_seq_wins() {
        let usage = crate::agent::provider::TokenUsage {
            input_tokens: Some(100),
            cache_hit_input_tokens: None,
            cache_hit_source: None,
            reasoning_tokens: None,
            output_tokens: Some(50),
            total_tokens: Some(150),
        };
        let mk = |seq: u64, turn: &str| {
            (
                seq,
                TurnEvent::ProviderUsage {
                    turn_id: turn.into(),
                    step: 0,
                    request_kind: crate::agent::telemetry::ProviderRequestKind::InitialRequest,
                    usage: usage.clone(),
                    cache_hit_input_tokens: None,
                    cache_miss_input_tokens: None,
                    prefix_mutation_reasons: Vec::new(),
                    first_token_latency_ms: None,
                    turn_duration_ms: None,
                    latency_kind: crate::agent::telemetry::ProviderLatencyKind::ProviderStream,
                    provider: "deepseek".into(),
                    model: "deepseek-chat".into(),
                },
            )
        };
        let mut state = MetricsProjectionState::init();
        // 同 turn：seq 2 先到达，seq 1 后到达（重放乱序）→ higher-seq-wins
        MetricsProjectionState::apply(&mut state, 2, &mk(2, "t1").1);
        MetricsProjectionState::apply(&mut state, 1, &mk(1, "t1").1);
        assert_eq!(state.by_turn.len(), 1, "lower-seq replay must not regress");
        // 不同 turn 的水位互不影响（per-turn 语义）
        MetricsProjectionState::apply(&mut state, 3, &mk(3, "t2").1);
        assert_eq!(state.by_turn.len(), 2, "per-turn watermark isolation");
    }

    #[test]
    fn plan_projection_whole_value() {
        let mut events = sample_events();
        events.push((
            5,
            TurnEvent::PlanUpdate {
                plan_state: serde_json::json!({"steps": ["a", "b"]}),
            },
        ));
        let state = fold_all::<_, PlanProjectionState>(&events);
        assert_eq!(
            state.plan_state,
            Some(serde_json::json!({"steps": ["a", "b"]}))
        );
    }

    #[test]
    fn projection_states_serialize_round_trip() {
        let events = sample_events();
        let history = fold_all::<_, HistoryProjectionState>(&events);
        let json = serde_json::to_string(&history).expect("serialize");
        let decoded: HistoryProjectionState = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded.messages().len(), history.messages().len());

        let metrics = fold_all::<_, MetricsProjectionState>(&events);
        let json = serde_json::to_string(&metrics).expect("serialize");
        let decoded: MetricsProjectionState = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded.totals(), metrics.totals());
    }

    /// PA-094：timeline 事件折叠映射表（design.md §2 表驱动）。
    /// step/start → call_model、tool/call → call_tool、tool/result → return_result、
    /// assistant/chunk → 文本聚合到 call_model、turn/end → 结算 token 指标。
    #[test]
    fn trace_projection_timeline_folding_table_driven() {
        let events: Vec<(u64, TurnEvent)> = vec![
            (
                0,
                TurnEvent::TurnStart {
                    turn_id: "t1".into(),
                },
            ),
            (
                1,
                TurnEvent::StepStart {
                    turn_id: "t1".into(),
                    step: 0,
                    first_token_latency_ms: Some(88),
                },
            ),
            (
                2,
                TurnEvent::AssistantChunk {
                    turn_id: "t1".into(),
                    step: 0,
                    text: "hel".into(),
                },
            ),
            (
                3,
                TurnEvent::AssistantChunk {
                    turn_id: "t1".into(),
                    step: 0,
                    text: "lo".into(),
                },
            ),
            (
                4,
                TurnEvent::ToolCall {
                    turn_id: "t1".into(),
                    step: 1,
                    call_id: "c1".into(),
                    name: "bash".into(),
                    arguments: "{}".into(),
                    started_at_ms: Some(1000),
                },
            ),
            (
                5,
                TurnEvent::ToolResult {
                    turn_id: "t1".into(),
                    step: 1,
                    call_id: "c1".into(),
                    result: Some("ok".into()),
                    error: None,
                    status: Some("done".into()),
                    duration_ms: Some(500),
                    artifacts: None,
                    capability_invocation: None,
                },
            ),
            (
                6,
                TurnEvent::ProviderUsage {
                    turn_id: "t1".into(),
                    step: 0,
                    request_kind: crate::agent::telemetry::ProviderRequestKind::InitialRequest,
                    usage: crate::agent::provider::TokenUsage {
                        input_tokens: Some(100),
                        cache_hit_input_tokens: Some(40),
                        cache_hit_source: None,
                        reasoning_tokens: Some(10),
                        output_tokens: Some(50),
                        total_tokens: Some(160),
                    },
                    cache_hit_input_tokens: Some(40),
                    cache_miss_input_tokens: Some(60),
                    prefix_mutation_reasons: Vec::new(),
                    first_token_latency_ms: Some(88),
                    turn_duration_ms: Some(500),
                    latency_kind: crate::agent::telemetry::ProviderLatencyKind::ProviderStream,
                    provider: "deepseek".into(),
                    model: "deepseek-chat".into(),
                },
            ),
            (
                7,
                TurnEvent::TurnEnd {
                    turn_id: "t1".into(),
                    reason: TurnEndReason::Completed,
                    turn_duration_ms: Some(1200),
                },
            ),
        ];
        let state = fold_all::<_, TraceProjectionState>(&events);
        let trace = state.trace_for_turn("t1").expect("trace");
        // 映射表断言：call_model（step/start）→ call_tool（tool/call）→
        // return_result（tool/result）→ call_model（ProviderUsage 回填 step 0）。
        let kinds: Vec<&str> = trace
            .trace_timeline
            .iter()
            .map(|e| e.kind.as_str())
            .collect();
        assert_eq!(
            kinds,
            vec!["call_model", "call_tool", "return_result"],
            "timeline mapping table"
        );
        // step/start → call_model：chunk 文本聚合（assistant/chunk → call_model.text）。
        let model_entry = &trace.trace_timeline[0];
        assert_eq!(model_entry.kind, "call_model");
        assert_eq!(
            model_entry.text.as_deref(),
            Some("hello"),
            "chunk aggregation"
        );
        // ProviderUsage 回填 call_model token 指标。
        assert_eq!(model_entry.input_tokens, Some(100));
        assert_eq!(model_entry.cache_hit_input_tokens, Some(40));
        assert_eq!(model_entry.output_tokens, Some(50));
        assert_eq!(model_entry.total_tokens, Some(160));
        assert_eq!(model_entry.first_token_latency_ms, Some(88));
        assert_eq!(model_entry.provider_name.as_deref(), Some("deepseek"));
        // tool/call → call_tool：tool_activities 挂载 + tool/result 回填终态。
        let tool_entry = &trace.trace_timeline[1];
        assert_eq!(tool_entry.kind, "call_tool");
        assert_eq!(tool_entry.tool_activities.len(), 1);
        assert_eq!(tool_entry.tool_activities[0].status, "done");
        assert_eq!(
            tool_entry.tool_activities[0].result_text.as_deref(),
            Some("ok")
        );
        assert_eq!(tool_entry.tool_activities[0].duration_seconds, Some(0.5));
        // tool/result → return_result。
        let return_entry = &trace.trace_timeline[2];
        assert_eq!(return_entry.kind, "return_result");
        assert_eq!(return_entry.text.as_deref(), Some("ok"));
        // turn/end → 结算 token 指标（trace 记录级）。
        assert_eq!(trace.turn_duration_ms, Some(1200));
        assert_eq!(trace.input_tokens, Some(100));
        assert_eq!(trace.output_tokens, Some(50));
        assert_eq!(trace.total_tokens, Some(160));
        assert_eq!(trace.first_token_latency_ms, Some(88));
        // 末条 call_model 补 turn_duration_ms（turn/end 结算）。
        assert_eq!(model_entry.turn_duration_ms, Some(1200));
    }

    /// PA-094：无 StepStart 事件时（当前运行时未发射），chunk/usage 兜底创建
    /// call_model 条目，turn/end 兜底补建。
    #[test]
    fn trace_projection_timeline_fallback_without_step_start() {
        let events: Vec<(u64, TurnEvent)> = vec![
            (
                0,
                TurnEvent::TurnStart {
                    turn_id: "t1".into(),
                },
            ),
            (
                1,
                TurnEvent::AssistantChunk {
                    turn_id: "t1".into(),
                    step: 0,
                    text: "hi".into(),
                },
            ),
            (
                2,
                TurnEvent::TurnEnd {
                    turn_id: "t1".into(),
                    reason: TurnEndReason::Completed,
                    turn_duration_ms: Some(300),
                },
            ),
        ];
        let state = fold_all::<_, TraceProjectionState>(&events);
        let trace = state.trace_for_turn("t1").expect("trace");
        assert_eq!(trace.trace_timeline.len(), 1, "fallback call_model entry");
        assert_eq!(trace.trace_timeline[0].kind, "call_model");
        assert_eq!(trace.trace_timeline[0].text.as_deref(), Some("hi"));
        assert_eq!(trace.trace_timeline[0].turn_duration_ms, Some(300));
    }

    /// PA-094：timeline 折叠水位幂等——同 seq 重放不重复 push timeline 条目。
    #[test]
    fn trace_projection_timeline_same_seq_replay_is_idempotent() {
        let event = TurnEvent::ToolCall {
            turn_id: "t1".into(),
            step: 1,
            call_id: "c1".into(),
            name: "bash".into(),
            arguments: "{}".into(),
            started_at_ms: None,
        };
        let mut state = TraceProjectionState::init();
        TraceProjectionState::apply(&mut state, 5, &event);
        TraceProjectionState::apply(&mut state, 5, &event); // 同 seq 重放
        let trace = state.trace_for_turn("t1").expect("trace");
        assert_eq!(
            trace.trace_timeline.len(),
            1,
            "timeline replay must not duplicate"
        );
        assert_eq!(trace.trace_timeline[0].kind, "call_tool");
    }

    /// PA-094（审核 P0）：首个事件 seq=0 不被 watermark 跳过（`unwrap_or(0) >= 0`
    /// 会误判已处理——修复后未记录水位时任何 seq 都应用）。
    #[test]
    fn trace_projection_first_event_seq_zero_is_applied() {
        // TurnStart 不创建 by_turn 条目，用 ToolCall 验证 seq=0 被应用。
        let event = TurnEvent::ToolCall {
            turn_id: "t1".into(),
            step: 0,
            call_id: "c1".into(),
            name: "bash".into(),
            arguments: "{}".into(),
            started_at_ms: None,
        };
        let mut state = TraceProjectionState::init();
        TraceProjectionState::apply(&mut state, 0, &event);
        let trace = state.trace_for_turn("t1").expect("trace");
        assert_eq!(trace.turn_id, "t1", "seq=0 first event must be applied");
        assert_eq!(trace.trace_timeline.len(), 1, "call_tool entry folded");
        assert_eq!(trace.trace_timeline[0].kind, "call_tool");
        // MetricsProjection 同样不跳过 seq=0。
        let usage_event = TurnEvent::ProviderUsage {
            turn_id: "t1".into(),
            step: 0,
            request_kind: crate::agent::telemetry::ProviderRequestKind::InitialRequest,
            usage: crate::agent::provider::TokenUsage {
                input_tokens: Some(100),
                cache_hit_input_tokens: None,
                cache_hit_source: None,
                reasoning_tokens: None,
                output_tokens: Some(50),
                total_tokens: Some(150),
            },
            cache_hit_input_tokens: None,
            cache_miss_input_tokens: None,
            prefix_mutation_reasons: Vec::new(),
            first_token_latency_ms: None,
            turn_duration_ms: None,
            latency_kind: crate::agent::telemetry::ProviderLatencyKind::ProviderStream,
            provider: "deepseek".into(),
            model: "deepseek-chat".into(),
        };
        let mut metrics = MetricsProjectionState::init();
        MetricsProjectionState::apply(&mut metrics, 0, &usage_event);
        assert_eq!(
            metrics.by_turn_records.get("t1").map(|r| r.len()),
            Some(1),
            "seq=0 usage event must be applied"
        );
    }

    /// PA-094（审核 P0）：真实 step 语义——`build_tool_event` 以 turn 事件序号作
    /// step（ToolCall 与 ToolResult 的 step 必然不同），call_tool 回填按 call_id
    /// 匹配，step 差异不影响终态回填。
    #[test]
    fn trace_projection_tool_backfill_with_real_step_semantics() {
        let events: Vec<(u64, TurnEvent)> = vec![
            (
                0,
                TurnEvent::TurnStart {
                    turn_id: "t1".into(),
                },
            ),
            (
                1,
                TurnEvent::StepStart {
                    turn_id: "t1".into(),
                    step: 0,
                    first_token_latency_ms: None,
                },
            ),
            (
                2,
                TurnEvent::AssistantChunk {
                    turn_id: "t1".into(),
                    step: 0,
                    text: "hi".into(),
                },
            ),
            // turn:tool(running) → seq 3（ToolCall step=3，turn 事件序号语义）
            (
                3,
                TurnEvent::ToolCall {
                    turn_id: "t1".into(),
                    step: 3,
                    call_id: "c1".into(),
                    name: "bash".into(),
                    arguments: "{}".into(),
                    started_at_ms: None,
                },
            ),
            // turn:tool(completed) → seq 5（ToolResult step=5，与 ToolCall 不同）
            (
                5,
                TurnEvent::ToolResult {
                    turn_id: "t1".into(),
                    step: 5,
                    call_id: "c1".into(),
                    result: Some("ok".into()),
                    error: None,
                    status: Some("done".into()),
                    duration_ms: Some(500),
                    artifacts: None,
                    capability_invocation: None,
                },
            ),
            (
                6,
                TurnEvent::TurnEnd {
                    turn_id: "t1".into(),
                    reason: TurnEndReason::Completed,
                    turn_duration_ms: Some(800),
                },
            ),
        ];
        let state = fold_all::<_, TraceProjectionState>(&events);
        let trace = state.trace_for_turn("t1").expect("trace");
        let kinds: Vec<&str> = trace
            .trace_timeline
            .iter()
            .map(|e| e.kind.as_str())
            .collect();
        assert_eq!(
            kinds,
            vec!["call_model", "call_tool", "return_result"],
            "timeline with real step semantics"
        );
        // call_tool 条目回填成功（step 差异不影响 call_id 匹配）。
        let tool_entry = &trace.trace_timeline[1];
        assert_eq!(tool_entry.kind, "call_tool");
        assert_eq!(tool_entry.tool_activities.len(), 1);
        assert_eq!(tool_entry.tool_activities[0].status, "done");
        assert_eq!(
            tool_entry.tool_activities[0].result_text.as_deref(),
            Some("ok")
        );
        assert_eq!(tool_entry.tool_activities[0].duration_seconds, Some(0.5));
        // return_result 条目非孤儿（text 携带结果）。
        let return_entry = &trace.trace_timeline[2];
        assert_eq!(return_entry.kind, "return_result");
        assert_eq!(return_entry.text.as_deref(), Some("ok"));
        // turn/end 结算：token 指标 + timeline 挂载。
        assert_eq!(trace.turn_duration_ms, Some(800));
        assert_eq!(trace.trace_timeline.len(), 3);
    }

    /// PA-094：ContextObservation 事件 → trace 记录只存引用（大字段外置）。
    #[test]
    fn trace_projection_context_observation_sets_ref() {
        let events: Vec<(u64, TurnEvent)> = vec![
            (
                0,
                TurnEvent::TurnStart {
                    turn_id: "t1".into(),
                },
            ),
            (
                1,
                TurnEvent::ContextObservation {
                    turn_id: "t1".into(),
                    step: 0,
                    observation: None,
                    observation_ref: Some("bco:t1:1".into()),
                },
            ),
            (
                2,
                TurnEvent::TurnEnd {
                    turn_id: "t1".into(),
                    reason: TurnEndReason::Completed,
                    turn_duration_ms: None,
                },
            ),
        ];
        let state = fold_all::<_, TraceProjectionState>(&events);
        let trace = state.trace_for_turn("t1").expect("trace");
        assert_eq!(
            trace.build_context_observation_ref.as_deref(),
            Some("bco:t1:1"),
            "trace 记录只存引用"
        );
        assert!(
            trace.build_context_observation.is_none(),
            "新数据不内嵌全量 payload"
        );
        // build_context timeline 条目（turn/end 结算生成，带引用，sequence 重排）。
        // PA-095 #3 兜底收窄：无模型活动证据（chunk/usage 均缺）不再补建
        // call_model——timeline 仅 build_context 条目。
        assert_eq!(
            trace.trace_timeline.len(),
            1,
            "build_context only (narrowed settle, no fabricated call_model)"
        );
        assert_eq!(trace.trace_timeline[0].kind, "build_context");
        assert_eq!(trace.trace_timeline[0].sequence, 1);
        assert_eq!(
            trace.trace_timeline[0]
                .build_context_observation_ref
                .as_deref(),
            Some("bco:t1:1"),
            "build_context entry carries ref"
        );
    }

    /// PA-094（审核 P0）：ContextObservation + 完整 turn → timeline 结构对齐运行时
    /// 产物（build_context 在开头，sequence 连续重排）。
    #[test]
    fn trace_projection_build_context_entry_prepended_and_sequence_renumbered() {
        let events: Vec<(u64, TurnEvent)> = vec![
            (
                0,
                TurnEvent::TurnStart {
                    turn_id: "t1".into(),
                },
            ),
            (
                1,
                TurnEvent::ContextObservation {
                    turn_id: "t1".into(),
                    step: 0,
                    observation: None,
                    observation_ref: Some("bco:t1:1".into()),
                },
            ),
            (
                2,
                TurnEvent::StepStart {
                    turn_id: "t1".into(),
                    step: 0,
                    first_token_latency_ms: None,
                },
            ),
            (
                3,
                TurnEvent::ToolCall {
                    turn_id: "t1".into(),
                    step: 3,
                    call_id: "c1".into(),
                    name: "bash".into(),
                    arguments: "{}".into(),
                    started_at_ms: None,
                },
            ),
            (
                4,
                TurnEvent::ToolResult {
                    turn_id: "t1".into(),
                    step: 5,
                    call_id: "c1".into(),
                    result: Some("ok".into()),
                    error: None,
                    status: Some("done".into()),
                    duration_ms: Some(500),
                    artifacts: None,
                    capability_invocation: None,
                },
            ),
            (
                5,
                TurnEvent::TurnEnd {
                    turn_id: "t1".into(),
                    reason: TurnEndReason::Completed,
                    turn_duration_ms: Some(800),
                },
            ),
        ];
        let state = fold_all::<_, TraceProjectionState>(&events);
        let trace = state.trace_for_turn("t1").expect("trace");
        let kinds: Vec<&str> = trace
            .trace_timeline
            .iter()
            .map(|e| e.kind.as_str())
            .collect();
        assert_eq!(
            kinds,
            vec!["build_context", "call_model", "call_tool", "return_result"],
            "build_context prepended, aligned with runtime timeline structure"
        );
        // sequence 连续重排（1-based）。
        let sequences: Vec<u64> = trace.trace_timeline.iter().map(|e| e.sequence).collect();
        assert_eq!(sequences, vec![1, 2, 3, 4], "sequence renumbered");
        // build_context 条目带引用。
        assert_eq!(
            trace.trace_timeline[0]
                .build_context_observation_ref
                .as_deref(),
            Some("bco:t1:1")
        );
    }

    /// PA-094（审核 P1）：多次 ProviderUsage 时顶层 token 指标为 turn 级累计
    /// （saturating_add），first_token_latency_ms 取首次；(None, None) 合并为 None。
    #[test]
    fn trace_projection_top_level_tokens_accumulate_across_usages() {
        let mk_usage =
            |input: u64, output: u64, total: u64, ttft: Option<u64>| TurnEvent::ProviderUsage {
                turn_id: "t1".into(),
                step: 0,
                request_kind: crate::agent::telemetry::ProviderRequestKind::InitialRequest,
                usage: crate::agent::provider::TokenUsage {
                    input_tokens: Some(input),
                    cache_hit_input_tokens: None,
                    cache_hit_source: None,
                    reasoning_tokens: None,
                    output_tokens: Some(output),
                    total_tokens: Some(total),
                },
                cache_hit_input_tokens: None,
                cache_miss_input_tokens: None,
                prefix_mutation_reasons: Vec::new(),
                first_token_latency_ms: ttft,
                turn_duration_ms: None,
                latency_kind: crate::agent::telemetry::ProviderLatencyKind::ProviderStream,
                provider: "deepseek".into(),
                model: "deepseek-chat".into(),
            };
        let events: Vec<(u64, TurnEvent)> = vec![
            (
                0,
                TurnEvent::TurnStart {
                    turn_id: "t1".into(),
                },
            ),
            (1, mk_usage(100, 50, 150, Some(88))),
            (2, mk_usage(80, 30, 110, None)),
            (
                3,
                TurnEvent::TurnEnd {
                    turn_id: "t1".into(),
                    reason: TurnEndReason::Completed,
                    turn_duration_ms: Some(1200),
                },
            ),
        ];
        let state = fold_all::<_, TraceProjectionState>(&events);
        let trace = state.trace_for_turn("t1").expect("trace");
        // turn/end 结算：顶层字段 = 多次 usage 累计。
        assert_eq!(trace.input_tokens, Some(180), "input accumulated");
        assert_eq!(trace.output_tokens, Some(80), "output accumulated");
        assert_eq!(trace.total_tokens, Some(260), "total accumulated");
        // first_token_latency_ms 取首次。
        assert_eq!(trace.first_token_latency_ms, Some(88), "first ttft wins");
    }

    /// PA-094：MetricsProjection 从 ProviderUsage 重建 ProviderCallCacheRecord
    /// （design.md §5 重建字段集；豁免 updated_at/event_id/sequence/emitted_at_ms）。
    #[test]
    fn metrics_projection_rebuilds_provider_call_records() {
        let usage = crate::agent::provider::TokenUsage {
            input_tokens: Some(100),
            cache_hit_input_tokens: Some(40),
            cache_hit_source: None,
            reasoning_tokens: Some(10),
            output_tokens: Some(50),
            total_tokens: Some(160),
        };
        let events: Vec<(u64, TurnEvent)> = vec![
            (
                1,
                TurnEvent::ProviderUsage {
                    turn_id: "t1".into(),
                    step: 0,
                    request_kind: crate::agent::telemetry::ProviderRequestKind::InitialRequest,
                    usage: usage.clone(),
                    cache_hit_input_tokens: Some(40),
                    cache_miss_input_tokens: Some(60),
                    prefix_mutation_reasons: vec!["session_summary_changed".into()],
                    first_token_latency_ms: Some(88),
                    turn_duration_ms: Some(500),
                    latency_kind: crate::agent::telemetry::ProviderLatencyKind::ProviderStream,
                    provider: "deepseek".into(),
                    model: "deepseek-chat".into(),
                },
            ),
            (
                2,
                TurnEvent::ProviderUsage {
                    turn_id: "t1".into(),
                    step: 1,
                    request_kind: crate::agent::telemetry::ProviderRequestKind::ToolFollowup,
                    usage: usage.clone(),
                    cache_hit_input_tokens: Some(40),
                    cache_miss_input_tokens: Some(60),
                    prefix_mutation_reasons: Vec::new(),
                    first_token_latency_ms: None,
                    turn_duration_ms: Some(300),
                    latency_kind: crate::agent::telemetry::ProviderLatencyKind::BufferedResponse,
                    provider: "deepseek".into(),
                    model: "deepseek-chat".into(),
                },
            ),
        ];
        let state = fold_all::<_, MetricsProjectionState>(&events);
        let records = state.by_turn_records.get("t1").expect("records");
        assert_eq!(records.len(), 2, "per-step records");
        // 重建字段集断言（design.md §5）。
        let first = &records[0];
        assert_eq!(
            first.request_kind,
            crate::agent::telemetry::ProviderRequestKind::InitialRequest
        );
        assert_eq!(first.provider_source.as_deref(), Some("deepseek"));
        assert_eq!(first.input_tokens, Some(100));
        assert_eq!(first.cache_hit_input_tokens, Some(40));
        assert_eq!(first.cache_miss_input_tokens, Some(60));
        assert_eq!(first.reasoning_tokens, Some(10));
        assert_eq!(first.output_tokens, Some(50));
        assert_eq!(first.total_tokens, Some(160));
        assert_eq!(first.first_token_latency_ms, Some(88));
        assert_eq!(first.turn_duration_ms, Some(500));
        assert_eq!(
            first.latency_kind,
            crate::agent::telemetry::ProviderLatencyKind::ProviderStream
        );
        assert_eq!(first.prefix_mutation_reasons.len(), 1);
        assert_eq!(
            first.prefix_mutation_reasons[0],
            crate::agent::provider::PrefixMutationReason::SessionSummaryChanged
        );
        // 豁免清单：updated_at/event_id/sequence/emitted_at_ms 不在记录上
        // （时钟/序列语义，trace 记录级字段）。
        let second = &records[1];
        assert_eq!(
            second.request_kind,
            crate::agent::telemetry::ProviderRequestKind::ToolFollowup
        );
        assert_eq!(
            second.latency_kind,
            crate::agent::telemetry::ProviderLatencyKind::BufferedResponse
        );
    }

    /// PA-094：同一 (turn_id, step) 的 ProviderUsage 替换 → 记录原位覆盖（不重复）。
    #[test]
    fn metrics_projection_provider_call_records_add_replacing() {
        let mk = |seq: u64, input: u64| {
            (
                seq,
                TurnEvent::ProviderUsage {
                    turn_id: "t1".into(),
                    step: 0,
                    request_kind: crate::agent::telemetry::ProviderRequestKind::InitialRequest,
                    usage: crate::agent::provider::TokenUsage {
                        input_tokens: Some(input),
                        cache_hit_input_tokens: None,
                        cache_hit_source: None,
                        reasoning_tokens: None,
                        output_tokens: Some(50),
                        total_tokens: Some(input + 50),
                    },
                    cache_hit_input_tokens: None,
                    cache_miss_input_tokens: None,
                    prefix_mutation_reasons: Vec::new(),
                    first_token_latency_ms: None,
                    turn_duration_ms: None,
                    latency_kind: crate::agent::telemetry::ProviderLatencyKind::ProviderStream,
                    provider: "deepseek".into(),
                    model: "deepseek-chat".into(),
                },
            )
        };
        let events = vec![mk(1, 100), mk(2, 200)];
        let state = fold_all::<_, MetricsProjectionState>(&events);
        let records = state.by_turn_records.get("t1").expect("records");
        assert_eq!(records.len(), 1, "same step replaces in place");
        assert_eq!(records[0].input_tokens, Some(200), "latest value wins");
    }
}

/// PA-095 #3：对拍约定字段集与豁免清单——唯一事实源（spec："agreed = 全字段集
/// − EXEMPT_FIELDS，清单为唯一事实源，禁止手抄字段列表"）。对拍测试经
/// `trace_field` / `timeline_entry_field` 取值器迭代消费常量；每项豁免须有
/// 反向探针（断言差异当前确实存在，防豁免腐化）。
pub mod parity {
    /// trace 记录级约定字段（对拍逐项断言；值经 `super::trace_field` 取）。
    /// spec 口径："phase, provider name/model, token fields, turn_duration_ms,
    /// event_type"——requested_name/source/mode 等六元数据不在约定集内。
    /// `turn_duration_ms` 由对拍 harness 的时钟漂移容差断言单独消费
    /// （EXEMPTIONS: turn_duration_ms_clock_drift）。
    pub const AGREED_TRACE_FIELDS: &[&str] = &[
        "phase",
        "provider_name",
        "provider_model",
        "input_tokens",
        "output_tokens",
        "total_tokens",
        "event_type",
    ];

    /// timeline 条目级约定字段（按 kind 配对后逐项断言；值经
    /// `super::timeline_entry_field` 取）。`sequence` 不在列——结构性豁免
    /// （prepare_retrieval/build_context 条目差异）导致绝对序号位置漂移，
    /// 改由"过滤后 kind 顺序一致"的保序断言承载。
    pub const AGREED_TIMELINE_ENTRY_FIELDS: &[&str] = &[
        "kind",
        "label",
        "state",
        "text",
        "tool_activities",
    ];

    /// 对拍时从 stored timeline 中剔除的条目 kind（结构性豁免——重建产物
    /// 不可能携带的条目）。
    pub const EXEMPT_TIMELINE_KINDS: &[&str] = &["prepare_retrieval", "checkpoint_persist"];

    /// 豁免清单：(名称, 理由)。约定字段集之外的全部已知差异必须登记于此，
    /// 且每项有反向探针（断言差异当前确实存在，防豁免腐化）。
    pub const EXEMPTIONS: &[(&str, &str)] = &[
        (
            "clock_fields(updated_at/event_id/emitted_at_ms)",
            "时钟语义——重建产物不含发射时刻上下文",
        ),
        (
            "title/session_id",
            "运行时装饰，投影不自洽——由 annotate 补写",
        ),
        (
            "timeline.prepare_retrieval_entry",
            "需 payload 判断 build_context_uses_retrieval，事件只有引用",
        ),
        (
            "timeline.checkpoint_persist_entry",
            "运行时 checkpoint 装饰条目（completed turn 无条件追加）——无事件承载",
        ),
        (
            "timeline.build_context_provider_metadata",
            "事件无承载 provider 六元数据，后续可扩展",
        ),
        (
            "timeline.failed_last_hop_state",
            "#4 StepEnd 已落地但投影未消费 step/end 的 error 态——收敛后移除本项",
        ),
        (
            "timeline.sync_call_model_text",
            "同步入口无 chunk 流，hop 文本存于 AssistantMessage 事件而投影不折叠它——过程文本由流式入口承载（流式场景 text 在约定字段集内）",
        ),
        (
            "terminal_no_usage_turn_provider_metadata",
            "failed/cancelled turn 无 provider/usage 事件（无已完成模型调用可结算），provider 元数据不可从事件重建——后续可扩展 TurnStart 携带 provider 元数据（原 failed_turn_provider_metadata，PA-095 #3 cancelled 对拍扩展覆盖）",
        ),
        (
            "timeline.call_tool_text",
            "call_tool 条目 text 语义分叉：运行时填 activity 描述（执行状态文案），事件只有工具名",
        ),
        (
            "turn_duration_ms_clock_drift",
            "事件发射时刻与 trace 落库时刻的毫秒级时钟漂移（对拍允许 ≤250ms 容差）",
        ),
    ];
}

/// 对拍取值器：按字段名取 trace 记录字段值（`parity::AGREED_TRACE_FIELDS`
/// 的消费端；未知字段名 panic——字段集与取值器必须同步演进）。
pub fn trace_field(trace: &crate::agent::session::TurnTraceRecord, field: &str) -> Option<String> {
    match field {
        "phase" => Some(trace.phase.clone()),
        "provider_requested_name" => trace.provider_requested_name.clone(),
        "provider_name" => trace.provider_name.clone(),
        "provider_model" => trace.provider_model.clone(),
        "input_tokens" => trace.input_tokens.map(|v| v.to_string()),
        "output_tokens" => trace.output_tokens.map(|v| v.to_string()),
        "total_tokens" => trace.total_tokens.map(|v| v.to_string()),
        "turn_duration_ms" => trace.turn_duration_ms.map(|v| v.to_string()),
        "event_type" => trace.event_type.clone(),
        _ => panic!("unknown parity trace field: {field}"),
    }
}

/// 对拍取值器：按字段名取 timeline 条目字段值
/// （`parity::AGREED_TIMELINE_ENTRY_FIELDS` 的消费端）。
pub fn timeline_entry_field(
    entry: &crate::agent::session::TraceTimelineEntry,
    field: &str,
) -> Option<String> {
    match field {
        "kind" => Some(entry.kind.clone()),
        "label" => Some(entry.label.clone()),
        "state" => Some(entry.state.clone()),
        "sequence" => Some(entry.sequence.to_string()),
        "text" => entry.text.clone(),
        "tool_activities" => Some(format!("{}", entry.tool_activities.len())),
        _ => panic!("unknown parity timeline field: {field}"),
    }
}
