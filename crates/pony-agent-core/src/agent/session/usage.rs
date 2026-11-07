//! 会话级用量累计（单一真值）。
//!
//! 口径契约：
//! - **只累加 per-call 记录**（`ProviderCallCacheRecord`），禁止用轮级/末次调用字段冒充会话累计；
//! - `input_tokens` 与 `cache_hit_input_tokens` 同源同量纲（`contextInput` 协议归一优先，
//!   缺失回退裸 `input`），缓存命中率 = Σ命中 / Σ输入，越界（> 1.0）⇒ `None`（fail-closed，
//!   禁止 clamp 成 100%）；
//! - 缺 `cache_hit_input_tokens` 的调用计入 `calls` 但不计入 `calls_with_cache`：
//!   「未知」不得被当成「0 命中」拉低会话命中率；
//! - 输出 token 按 per-call `output_tokens` 累加（会话累计，不随轮次清零）。
//!
//! 消费方：TUI header（会话累计）与 `/resume` 恢复（历史回填）。

use super::types::{SessionSnapshot, TurnTraceRecord};
use crate::agent::metrics_snapshot::TurnMetricsSnapshot;
use crate::agent::telemetry::ProviderCallCacheRecord;

/// 会话/轮次用量累计（单位：token）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SessionUsageTotals {
    /// Σ 调用上下文输入（与缓存命中同量纲）。
    pub input_tokens: u64,
    /// Σ 调用缓存命中输入。
    pub cache_hit_input_tokens: u64,
    /// Σ 调用输出。
    pub output_tokens: u64,
    /// 已结算调用数。
    pub calls: u64,
    /// 其中**上报了**缓存命中的调用数（命中率分母的可信度判据）。
    pub calls_with_cache: u64,
}

impl SessionUsageTotals {
    /// 单次调用累计（`cache_hit == None` ⇒ 只计调用数，不计命中）。
    pub fn from_call(input: Option<u64>, cache_hit: Option<u64>, output: Option<u64>) -> Self {
        let mut totals = Self {
            input_tokens: input.unwrap_or(0),
            output_tokens: output.unwrap_or(0),
            calls: u64::from(input.is_some() || cache_hit.is_some() || output.is_some()),
            ..Self::default()
        };
        if let Some(cache_hit) = cache_hit {
            totals.cache_hit_input_tokens = cache_hit;
            totals.calls_with_cache = 1;
        }
        totals
    }

    /// 累加另一次结算（会话累计 = Σ per-call）。
    pub fn merge(&mut self, other: Self) {
        self.input_tokens = self.input_tokens.saturating_add(other.input_tokens);
        self.cache_hit_input_tokens = self
            .cache_hit_input_tokens
            .saturating_add(other.cache_hit_input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(other.output_tokens);
        self.calls = self.calls.saturating_add(other.calls);
        self.calls_with_cache = self.calls_with_cache.saturating_add(other.calls_with_cache);
    }

    /// 是否已有任何会话用量结算可展示。
    pub fn is_empty(&self) -> bool {
        self.calls == 0
    }

    /// 会话缓存命中率（0.0..=1.0）；无命中上报 / 无输入 / 越界 ⇒ `None`（fail-closed）。
    pub fn cache_hit_ratio(&self) -> Option<f64> {
        if self.calls_with_cache == 0 || self.input_tokens == 0 {
            return None;
        }
        let candidate = self.cache_hit_input_tokens as f64 / self.input_tokens as f64;
        if !candidate.is_finite() || candidate > 1.0 {
            return None;
        }
        Some(candidate.max(0.0))
    }
}

/// 单次调用的协议归一上下文输入：`context_input_tokens` 优先，缺失回退裸 `input_tokens`。
fn call_context_input(record: &ProviderCallCacheRecord) -> Option<u64> {
    record.context_input_tokens.or(record.input_tokens)
}

/// 单条轮次轨迹的 per-call 累计；无 per-call 记录时回退轨迹级字段（旧数据）。
pub fn turn_usage_totals(trace: &TurnTraceRecord) -> SessionUsageTotals {
    if trace.provider_call_records.is_empty() {
        return SessionUsageTotals::from_call(
            trace.context_input_tokens.or(trace.input_tokens),
            trace.cache_hit_input_tokens,
            trace.output_tokens,
        );
    }
    let mut totals = SessionUsageTotals::default();
    for record in &trace.provider_call_records {
        totals.merge(SessionUsageTotals::from_call(
            call_context_input(record),
            record.cache_hit_input_tokens,
            record.output_tokens,
        ));
    }
    totals
}

/// 会话累计用量：Σ 全部轮次的 per-call 结算（不含 UI 侧未结算的进行中调用）。
pub fn session_usage_totals(session: &SessionSnapshot) -> SessionUsageTotals {
    let mut totals = SessionUsageTotals::default();
    for trace in &session.turn_trace_history {
        totals.merge(turn_usage_totals(trace));
    }
    totals
}

/// 恢复会话时的「当前上下文占用」：末次已结算调用的上下文输入（ADR 0035 S1）。
pub fn last_context_input_tokens(session: &SessionSnapshot) -> Option<u64> {
    session
        .turn_trace_history
        .iter()
        .rev()
        .find_map(|trace| {
            trace
                .provider_call_records
                .last()
                .and_then(call_context_input)
                .or(trace.context_input_tokens)
                .or(trace.input_tokens)
        })
        .filter(|tokens| *tokens > 0)
}

/// 单条 per-call 快照的**轮级聚合**转为本轮累计（UI 会话台账的唯一输入）。
///
/// 取的是快照里与 per-call 组分离的 `turn*` 字段（Σ上下文输入 / Σ缓存命中 / Σ输出）——
/// 这些字段由生产侧逐条 records 相加得出，**不是**末次调用值，因此可以安全累加成会话累计。
pub fn turn_usage_totals_from_snapshot(snapshot: &TurnMetricsSnapshot) -> SessionUsageTotals {
    SessionUsageTotals {
        input_tokens: snapshot.turn_context_input_tokens,
        cache_hit_input_tokens: snapshot.turn_cache_hit_input_tokens,
        output_tokens: snapshot.turn_output_tokens,
        calls: u64::from(snapshot.turn_calls),
        calls_with_cache: u64::from(snapshot.turn_calls_with_cache),
    }
}

/// 会话用量台账：`已结算轮次的 Σ` + `本轮最新快照的轮级 Σ`。
///
/// 为什么需要它：per-call 快照只描述**末次调用**，直接渲染会把 `↓out`/缓存命中率退化成
/// 「本轮最后一次调用」；而 `aggregate_session_metrics` 只在落库快照上可用。台账在同一处
/// 承接两种来源（实时快照 / 历史恢复），保证 header 与 `/resume` 同口径。
///
/// 生命周期：
/// - `begin_turn`：轮次起点把本轮 Σ 结算进 `committed`（**不清零**——轮首仍要显示上一轮末值）；
/// - `set_turn`：同一轮的新快照**幂等覆盖**本轮 Σ（多跳不会重复累加）；
/// - `restore`：会话切换/恢复时用历史真值重置（`session_usage_totals`）；
/// - `reset`：会话作用域复位（`/clear` 新建会话）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SessionUsageLedger {
    committed: SessionUsageTotals,
    current_turn: SessionUsageTotals,
}

impl SessionUsageLedger {
    /// 会话累计 = 已结算轮次 + 本轮（会话累计的唯一读口）。
    pub fn totals(&self) -> SessionUsageTotals {
        let mut totals = self.committed;
        totals.merge(self.current_turn);
        totals
    }

    /// 轮次起点：把本轮 Σ 结算进 `committed`，本轮归零（不清 `committed`）。
    pub fn begin_turn(&mut self) {
        self.committed.merge(self.current_turn);
        self.current_turn = SessionUsageTotals::default();
    }

    /// 同一轮的新快照结果**覆盖**本轮 Σ（幂等；多跳重复上报不会重复累加）。
    pub fn set_turn(&mut self, turn: SessionUsageTotals) {
        self.current_turn = turn;
    }

    /// 历史恢复：`committed` 直接取会话真值，本轮清零。
    pub fn restore(&mut self, totals: SessionUsageTotals) {
        self.committed = totals;
        self.current_turn = SessionUsageTotals::default();
    }

    /// 会话作用域复位。
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// 是否已有任何会话用量（`false` ⇒ 消费方回退各自的历史口径）。
    pub fn is_empty(&self) -> bool {
        self.totals().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::telemetry::{ProviderCallCacheRecord, ProviderRequestKind};

    fn record(input: Option<u64>, cache_hit: Option<u64>, output: Option<u64>) -> ProviderCallCacheRecord {
        ProviderCallCacheRecord {
            request_kind: ProviderRequestKind::default(),
            input_tokens: input,
            cache_hit_input_tokens: cache_hit,
            output_tokens: output,
            ..Default::default()
        }
    }

    fn trace(records: Vec<ProviderCallCacheRecord>) -> TurnTraceRecord {
        TurnTraceRecord {
            provider_call_records: records,
            ..Default::default()
        }
    }

    #[test]
    fn per_call_sums_not_last_call() {
        let session = SessionSnapshot {
            conversation_id: "s".into(),
            title: String::new(),
            summary: String::new(),
            history: Vec::new(),
            attachment_assets: Vec::new(),
            provider_native_transcript: Vec::new(),
            turn_trace_history: vec![
                trace(vec![record(Some(100), Some(80), Some(10)), record(Some(200), Some(150), Some(20))]),
                trace(vec![record(Some(300), None, Some(30))]),
            ],
            long_term_memory_entries: Vec::new(),
            memory_write_evidence: Vec::new(),
            memory_write_hook_trace_records: Vec::new(),
            history_state_evidence: Vec::new(),
            history_state_audit_summary: Default::default(),
            run_control_audit_summary: Default::default(),
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
        let totals = session_usage_totals(&session);
        assert_eq!(totals.output_tokens, 60, "输出必须是 per-call Σ，不是末次调用 30");
        assert_eq!(totals.input_tokens, 600, "输入必须是 per-call Σ");
        assert_eq!(totals.cache_hit_input_tokens, 230);
        assert_eq!(totals.calls_with_cache, 2, "未上报缓存的调用不得计入命中分母");
        // 230 / 600 ≈ 0.383；若用末次调用（None/300）会被算成 0 或 None
        let ratio = totals.cache_hit_ratio().expect("会话命中率必须可算");
        assert!((ratio - 230.0 / 600.0).abs() < 1e-9, "会话命中率 = Σ命中/Σ输入");
    }

    #[test]
    fn cache_ratio_fails_closed_when_hit_exceeds_input() {
        let totals = SessionUsageTotals {
            input_tokens: 100,
            cache_hit_input_tokens: 140,
            output_tokens: 0,
            calls: 1,
            calls_with_cache: 1,
        };
        assert_eq!(totals.cache_hit_ratio(), None, "越界必须 fail-closed，不得 clamp 成 100%");
    }

    #[test]
    fn ledger_commits_turn_and_replaces_within_turn() {
        let mut ledger = SessionUsageLedger::default();
        ledger.set_turn(SessionUsageTotals { input_tokens: 100, cache_hit_input_tokens: 40, output_tokens: 10, calls: 1, calls_with_cache: 1 });
        ledger.set_turn(SessionUsageTotals { input_tokens: 250, cache_hit_input_tokens: 90, output_tokens: 25, calls: 2, calls_with_cache: 2 });
        assert_eq!(ledger.totals().output_tokens, 25, "同一轮多跳必须覆盖而非累加");
        ledger.begin_turn();
        assert_eq!(ledger.totals().output_tokens, 25, "轮首必须保留上一轮末值（不得清零）");
        ledger.set_turn(SessionUsageTotals { input_tokens: 300, cache_hit_input_tokens: 100, output_tokens: 5, calls: 1, calls_with_cache: 1 });
        assert_eq!(ledger.totals().output_tokens, 30, "跨轮必须是会话累计");
        assert_eq!(ledger.totals().input_tokens, 550);
    }

    #[test]
    fn restore_resets_to_history_truth() {
        let mut ledger = SessionUsageLedger::default();
        ledger.set_turn(SessionUsageTotals { input_tokens: 999, cache_hit_input_tokens: 0, output_tokens: 999, calls: 1, calls_with_cache: 0 });
        ledger.restore(session_usage_totals(&SessionSnapshot {
            conversation_id: "s".into(),
            title: String::new(),
            summary: String::new(),
            history: Vec::new(),
            attachment_assets: Vec::new(),
            provider_native_transcript: Vec::new(),
            turn_trace_history: vec![trace(vec![record(Some(10), Some(5), Some(2))])],
            long_term_memory_entries: Vec::new(),
            memory_write_evidence: Vec::new(),
            memory_write_hook_trace_records: Vec::new(),
            history_state_evidence: Vec::new(),
            history_state_audit_summary: Default::default(),
            run_control_audit_summary: Default::default(),
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
        }));
        assert_eq!(ledger.totals().output_tokens, 2, "恢复必须用历史真值覆盖，不得叠加恢复前的实时值");
    }

    #[test]
    fn last_context_input_prefers_last_call() {
        let session = SessionSnapshot {
            conversation_id: "s".into(),
            title: String::new(),
            summary: String::new(),
            history: Vec::new(),
            attachment_assets: Vec::new(),
            provider_native_transcript: Vec::new(),
            turn_trace_history: vec![
                trace(vec![record(Some(700), None, Some(1))]),
                trace(vec![record(Some(900), None, Some(1))]),
            ],
            long_term_memory_entries: Vec::new(),
            memory_write_evidence: Vec::new(),
            memory_write_hook_trace_records: Vec::new(),
            history_state_evidence: Vec::new(),
            history_state_audit_summary: Default::default(),
            run_control_audit_summary: Default::default(),
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
        assert_eq!(last_context_input_tokens(&session), Some(900), "上下文占用取末次已结算调用");
    }
}
