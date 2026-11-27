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
//! - **上下文占用**是「下一跳预估」而非「上一次请求的采样」（B116，对齐 DSH
//!   `token-meter` 的 `projectedTokens`）：采样值 + 采样后新增请求表面的估算
//!   + compaction/history 缩减的有符号负增量，见 [`ContextSurfaceLedger`]。
//!
//! 消费方：TUI header（会话累计 + 上下文投影）与 `/resume` 恢复（历史回填）。

use super::types::{SessionSnapshot, TurnTraceRecord};
use pony_protocol::estimate::estimate_text_tokens;
use pony_protocol::metrics_snapshot::TurnMetricsSnapshot;
use pony_protocol::telemetry::ProviderCallCacheRecord;

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

// ---------------------------------------------------------------------------
// B116：上下文占用「下一跳预估」投影账本
//
// 口径真源：DSH `packages/llm/token-meter/src/usage-projection.ts`
//   projectedTokens = max(0, pressureTokens + surfaceTokens - sampledSurfaceTokens)
//
// 三个量都只在**一个**地方成型（本模块）：
// - `pressureTokens`：最近一次已结算调用的 prompt 侧压力（`context_input_tokens`
//   协议归一值，缺失回退裸 `input_tokens`）——端点采样，不是估算；
// - `surfaceTokens`：请求表面（下一跳会把整段历史发给 provider）的累计估算，
//   由调用方按「追加 / 替换」逐条记账；
// - `sampledSurfaceTokens`：采样发生那一刻的 `surfaceTokens`（采样锚点）。
//
// 为什么不能直接用采样值渲染：采样描述的是**上一次请求**，工具结果刚追加上、
// 下一跳尚未返回 usage 的窗口期内读数会冻结，用户直觉却是「现在还剩多少」。
// 投影把这段时间的 surface 变化叠加到采样值上，读数随工具结果前进、随
// compaction 后退（有符号增量，投影 clamp ≥0）。
//
// 持久化真值不受影响：本账本是**UI 侧瞬时投影**，不写回 session usage。
// ---------------------------------------------------------------------------

/// 请求表面上单条消息的封装开销（role 标记 / 分隔符 / 换行）。
///
/// 与压缩/预算侧同口径（`compression::estimate_native_message_tokens` 的 `+4`）；
/// 只在**这里**定义一次——投影估算不得另立第二个 magic 常量。
const SURFACE_MESSAGE_OVERHEAD_TOKENS: u64 = 4;

/// 文本 → token 估算（投影账本的唯一估算出处）。
///
/// 复用压缩/预算同一个估算器（每 4 **字符** ≈ 1 token，按字符而非字节：中文一字三字节，
/// 按字节会把中文会话放大三倍）。两个口径一旦分叉，「按 A 判压缩、按 B 估投影」会互相矛盾。
pub fn estimate_surface_text_tokens(text: &str) -> u64 {
    u64::try_from(estimate_text_tokens(text)).unwrap_or(u64::MAX)
}

/// 一条**追加到下一跳请求表面**的文本消息的 token 估算（文本 + 消息封装开销）。
pub fn estimate_surface_message_tokens(text: &str) -> u64 {
    SURFACE_MESSAGE_OVERHEAD_TOKENS.saturating_add(estimate_surface_text_tokens(text))
}

/// 一次工具往返对请求表面的追加估算。
///
/// 下一跳请求会同时携带 assistant 的 `tool_calls`（参数）与 tool 结果两条消息，故按两条
/// 消息各计价；字段缺失即该条不参与（不补 0 之外的伪内容）。
///
/// 已知下界：结果文本若已被显示层截断（`TurnToolActivity::result_text` 是展示投影），
/// 估算只会**低估**。方向正确性（工具结果追加后读数前进）不受影响，下一跳采样会重新锚定。
pub fn estimate_tool_roundtrip_surface_tokens(
    arguments_text: Option<&str>,
    result_text: Option<&str>,
) -> u64 {
    [arguments_text, result_text]
        .into_iter()
        .flatten()
        .filter(|text| !text.is_empty())
        .map(estimate_surface_message_tokens)
        .fold(0u64, u64::saturating_add)
}

/// 上下文占用投影账本（下一跳预估的唯一真值；UI 侧瞬时态，不落库）。
///
/// 生命周期：
/// - `note_sample`：每次结算 usage 时调用（幂等：同一次结算重复投递只会重打锚点，不叠加）；
/// - `note_surface_append`：工具结果 / 新消息追加到请求表面时调用；
/// - `note_history_shrink` / `note_surface_delta`：compaction / 历史缩减的有符号负增量；
/// - `reset`：会话切换 / `/clear` 复位。
///
/// 不变式：`projected_tokens()` 为 `None` ⇒ 尚无采样，消费方必须回退现行口径
/// （不得用 0 冒充「空上下文」）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ContextSurfaceLedger {
    /// 最近一次已结算调用的 prompt 侧压力（`None` = 尚无采样）。
    pressure: Option<u64>,
    /// 采样锚点：采样发生时的表面累计。
    sampled_surface: u64,
    /// 运行中的请求表面累计（追加 +，替换/压缩 −，永不为负）。
    surface: u64,
}

impl ContextSurfaceLedger {
    /// 记录一次已结算调用的上下文采样，并把当前表面记为锚点。
    ///
    /// `pressure_tokens == 0` 视为**无采样**（与 [`last_context_input_tokens`] 的
    /// `.filter(> 0)` 同口径）：0 无法区分「空 prompt」与「usage 缺失」，不得把读数钉成 0%。
    pub fn note_sample(&mut self, pressure_tokens: u64) {
        if pressure_tokens == 0 {
            return;
        }
        self.pressure = Some(pressure_tokens);
        self.sampled_surface = self.surface;
    }

    /// 请求表面追加（工具结果、新消息等）：表面累计单调上升。
    pub fn note_surface_append(&mut self, tokens: u64) {
        self.surface = self.surface.saturating_add(tokens);
    }

    /// 请求表面的**有符号**增量（负值 = 替换/缩减释放了表面）。
    ///
    /// 表面累计本身 clamp ≥0（不存在负表面）；投影的 ≥0 clamp 在 [`Self::projected_tokens`]。
    pub fn note_surface_delta(&mut self, delta: i64) {
        if delta >= 0 {
            self.surface = self.surface.saturating_add(u64::try_from(delta).unwrap_or(u64::MAX));
        } else {
            self.surface = self.surface.saturating_sub(delta.unsigned_abs());
        }
    }

    /// compaction / 历史缩减：把 `after - before` 的负增量记入表面。
    ///
    /// 未缩减（`after >= before`）⇒ 不动：表面增长只允许经 [`Self::note_surface_append`]
    /// 显式记账，避免把「压缩后仍更长」的异常数据当成追加 double count。
    pub fn note_history_shrink(&mut self, before_tokens: u64, after_tokens: u64) {
        if after_tokens >= before_tokens {
            return;
        }
        let released = i64::try_from(before_tokens.saturating_sub(after_tokens)).unwrap_or(i64::MAX);
        self.note_surface_delta(-released);
    }

    /// 下一跳上下文占用预估：`max(0, pressure + surface − sampled_surface)`。
    ///
    /// `None` ⇒ 尚无采样（消费方回退现行口径）；`Some(0)` 是合法值（压缩后可降到 0）。
    pub fn projected_tokens(&self) -> Option<u64> {
        let pressure = self.pressure?;
        let projected = i128::from(pressure) + i128::from(self.surface)
            - i128::from(self.sampled_surface);
        Some(u64::try_from(projected.max(0)).unwrap_or(u64::MAX))
    }

    /// 会话作用域复位（`/clear` / 会话切换）。
    pub fn reset(&mut self) {
        *self = Self::default();
    }
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
    use pony_protocol::telemetry::{ProviderCallCacheRecord, ProviderRequestKind};

    fn record(input: Option<u64>, cache_hit: Option<u64>, output: Option<u64>) -> ProviderCallCacheRecord {
        ProviderCallCacheRecord {
            upstream_status: None, // B139：非 2xx 才写值；此处无真实状态码 ⇒ 留空
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

    // ---- B116：上下文占用投影账本 ----

    #[test]
    fn context_projection_without_sample_falls_back_to_current_reading() {
        let ledger = ContextSurfaceLedger::default();
        assert_eq!(
            ledger.projected_tokens(),
            None,
            "无采样必须回退现行口径（None），不得用 0 冒充空上下文"
        );
        let mut zero_sample = ContextSurfaceLedger::default();
        zero_sample.note_sample(0);
        assert_eq!(
            zero_sample.projected_tokens(),
            None,
            "0 无法区分「空 prompt」与「usage 缺失」⇒ 视为无采样"
        );
    }

    #[test]
    fn context_projection_rises_after_tool_surface_append() {
        let mut ledger = ContextSurfaceLedger::default();
        ledger.note_sample(10_000);
        assert_eq!(ledger.projected_tokens(), Some(10_000), "采样瞬间投影 = 采样值");

        let appended = estimate_tool_roundtrip_surface_tokens(
            Some("{\"path\":\"src/main.rs\"}"),
            Some("fn main() {}"),
        );
        assert!(appended > 0, "工具往返估算必须为正：{appended}");
        ledger.note_surface_append(appended);
        assert_eq!(
            ledger.projected_tokens(),
            Some(10_000 + appended),
            "工具结果追加后投影必须上升（不得冻结）"
        );

        // 同一次结算重复投递：只重打锚点，不叠加（幂等）。
        ledger.note_sample(10_000);
        assert_eq!(
            ledger.projected_tokens(),
            Some(10_000),
            "重复采样必须幂等重锚，不得把已记入的表面重复计入"
        );
    }

    #[test]
    fn context_projection_drops_after_compaction_and_never_negative() {
        let mut ledger = ContextSurfaceLedger::default();
        ledger.note_sample(5_000);
        ledger.note_surface_append(3_000);
        assert_eq!(ledger.projected_tokens(), Some(8_000));

        // compaction：3_000 → 1_000，负增量 2_000。
        ledger.note_history_shrink(3_000, 1_000);
        assert_eq!(
            ledger.projected_tokens(),
            Some(6_000),
            "compaction 后投影必须下降（有符号负增量）"
        );

        // 极端缩减：不得把投影/表面压成负数。
        ledger.note_history_shrink(100_000, 0);
        assert_eq!(ledger.projected_tokens(), Some(5_000), "表面归零后投影 = 采样值");
        ledger.note_surface_delta(-1);
        assert!(ledger.projected_tokens().is_some(), "投影必须仍可计算（≥0 由 u64 类型保证）");

        // 未缩减（after >= before）不得被当成追加。
        let mut stable = ContextSurfaceLedger::default();
        stable.note_sample(900);
        stable.note_history_shrink(1_000, 1_200);
        assert_eq!(stable.projected_tokens(), Some(900));
    }

    #[test]
    fn context_projection_clamps_when_release_exceeds_pressure() {
        let mut ledger = ContextSurfaceLedger::default();
        ledger.note_sample(100);
        ledger.note_surface_append(50);
        // 表面被整体替换掉（比采样基线还小）⇒ pressure + surface - sampled 为负。
        ledger.note_surface_delta(-500);
        assert_eq!(
            ledger.projected_tokens(),
            Some(100),
            "表面归零时投影退化为采样值（不是负数）"
        );
        ledger.note_surface_delta(-500);
        assert_eq!(ledger.projected_tokens(), Some(100), "表面累计 clamp ≥0");
    }

    #[test]
    fn context_surface_estimator_is_shared_and_char_based() {
        // 4 字符 = 1 token；封装开销只在一处定义。
        assert_eq!(estimate_surface_text_tokens("abcd"), 1);
        assert_eq!(estimate_surface_text_tokens(""), 0);
        assert_eq!(estimate_surface_message_tokens("abcd"), 5);
        // 非 ASCII/中文按字符加权估算（约 1.5 token/字，非按字节）：4 个汉字 = 6 token + 4 开销。
        assert_eq!(estimate_surface_message_tokens("上下文占"), 10);
        // 缺字段不补内容。
        assert_eq!(estimate_tool_roundtrip_surface_tokens(None, None), 0);
        assert_eq!(
            estimate_tool_roundtrip_surface_tokens(Some("abcd"), None),
            5
        );
    }
}
