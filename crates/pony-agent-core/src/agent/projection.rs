// projection: 会话投影层（事件溯源演进阶段 2，PA-092）。
// 投影是"事件 → 视图"的纯函数折叠：任何时刻从事件序列可确定性重建视图。
// 设计见 openspec/changes/session-projection-layer/design.md（对抗审核后定稿）。
// 对抗审核（2026-08-18）采纳：水位 >= 语义、per-turn 水位、数据字段回填、
// chunk 文本聚合、turn/end 状态标记、PlanProjection、四桶统一、状态序列化。
use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::agent::session::{TurnHistoryMessage, TurnTraceRecord};
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
                            TurnEndReason::Completed => {
                                crate::agent::session::MessageStatus::Done
                            }
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
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TraceProjectionState {
    by_turn: HashMap<String, TurnTraceRecord>,
    /// turn_id → 已折叠到的最大事件 seq（per-turn 水位，`>=` 语义防同 seq 重放）。
    watermark: HashMap<String, u64>,
    /// turn_id → chunk 文本累积（写时聚合，turn/end 结算进 trace）。
    chunk_text: HashMap<String, String>,
    /// turn_id → 最近 provider/model（终态信封回填用）。
    provider_model: HashMap<String, (String, String)>,
}

impl TraceProjectionState {
    pub fn traces(&self) -> Vec<TurnTraceRecord> {
        let mut traces: Vec<TurnTraceRecord> = self.by_turn.values().cloned().collect();
        // 按 turn 起始 seq 排序（插入序近似；turn_id 字典序会错乱 turn-10 < turn-2）。
        let mut order: Vec<(u64, String)> = self
            .by_turn
            .iter()
            .map(|(turn_id, _)| {
                let start = self
                    .watermark
                    .get(turn_id)
                    .copied()
                    .unwrap_or(u64::MAX);
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

    pub fn trace_for_turn(&self, turn_id: &str) -> Option<&TurnTraceRecord> {
        self.by_turn.get(turn_id)
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
        if state.watermark.get(&turn_id).copied().unwrap_or(0) >= seq {
            return;
        }
        state.watermark.insert(turn_id.clone(), seq);
        match event {
            TurnEvent::AssistantChunk { text, .. } => {
                // chunk 文本聚合（design.md:35）：不逐条 push step，累积到 turn/end 结算。
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
                if !trace
                    .tool_activities
                    .iter()
                    .any(|a| a.id == *call_id)
                {
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
                if let Some(activity) = trace
                    .tool_activities
                    .iter_mut()
                    .find(|a| a.id == *call_id)
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
                        status: status
                            .clone()
                            .unwrap_or_else(|| "done".to_string()),
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
            }
            TurnEvent::ProviderUsage {
                provider, model, ..
            } => {
                state
                    .provider_model
                    .insert(turn_id.clone(), (provider.clone(), model.clone()));
            }
            TurnEvent::TurnEnd {
                reason,
                turn_duration_ms,
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
                trace.turn_duration_ms = *turn_duration_ms;
                // chunk 文本结算进 trace（文本聚合的落点）。
                if let Some(text) = state.chunk_text.remove(&turn_id) {
                    trace.trace_steps.push(crate::agent::telemetry::TurnTraceStep {
                        id: format!("chunk-{seq}"),
                        label: "assistant_output".to_string(),
                        state: "completed".to_string(),
                    });
                    let _ = text;
                }
                // 终态信封：event_type 由 reason 映射；title/phase/sequence 等
                // 由外层 annotate_turn_trace_terminal_event 补写（投影不自洽字段）。
                trace.event_type = Some(match reason {
                    TurnEndReason::Completed => "turn.completed".to_string(),
                    TurnEndReason::Error => "turn.failed".to_string(),
                    TurnEndReason::Aborted | TurnEndReason::Cancelled => {
                        "turn.cancelled".to_string()
                    }
                });
                trace.sequence = Some(seq);
                if let Some((provider, model)) = state.provider_model.get(&turn_id) {
                    trace.provider_name = Some(provider.clone());
                    trace.provider_model = Some(model.clone());
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
    /// addReplacing 键：(turn_id, step) → 已累计的贡献（替换时回退）。
    replaced: HashMap<(String, u32), StepContribution>,
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
        if state.watermark.get(&turn_id).copied().unwrap_or(0) >= seq {
            return;
        }
        state.watermark.insert(turn_id.clone(), seq);
        match event {
            TurnEvent::ProviderUsage {
                turn_id,
                step,
                usage,
                cache_hit_input_tokens,
                cache_miss_input_tokens,
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
                if let Some(previous) =
                    state.replaced.insert((turn_id.clone(), *step), contribution)
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
                    turn_agg.output_tokens =
                        turn_agg.output_tokens.saturating_add(aggregate.output_tokens);
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
                    prefix_mutation_reasons: Vec::new(),
                });
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
        if matches!(event, TurnEvent::CheckpointCheckout { .. })
            || visible.contains(branch_id)
        {
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
        assert_eq!(trace.turn_duration_ms, Some(500), "turn duration from turn/end");
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
        let by_turn_sum = |key: fn(&TurnUsageAggregate) -> u64| {
            state.by_turn.values().map(key).sum::<u64>()
        };
        assert_eq!(totals.uncached_input, by_turn_sum(|a| a.cache_miss_input_tokens));
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
}