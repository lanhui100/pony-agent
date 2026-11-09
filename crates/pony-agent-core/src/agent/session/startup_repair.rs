//! 启动装载期的**有界修复**：空壳会话的派生字段保护 + trace 投影缓存重建预算。
//!
//! ## 背景（真实现场，2026-11-09）
//!
//! 真实库 `sessions.db` 1.2 GB / 143 会话 / `session-logs` 535 MB 时，`pony` 冷启动到首帧
//! 实测 **9.6 s**，日志里能看到三笔常数开销：
//!
//! ```text
//! session_fold_slow: elapsed=843ms budget=800ms folded=12 skipped=132
//! index_projection_filled: 143 / history_rebuilt_from_turn_events: 24
//! commit_boundary_committed: label=save_to_backend session_count=143 elapsed_ms=2330
//! ```
//!
//! 第三笔是**启动期整包回写**：`SessionStore::with_backend` 在装载后按「内存态与库不一致」
//! 判定要修，然后 `save_to_backend()` 把 143 个会话整体提交。它与自愈叠在一起就是
//! 「启动越用越慢」的机制。
//!
//! ## 两条不变量（改这里之前先读）
//!
//! 1. **空壳会话不得重算派生字段**。停写 blob 后索引里只剩 `__index_only` 轻量行：内容
//!    （history / trace）不在索引里，`refresh_session_metadata` 会据此把 `title` 打回默认
//!    标题、`turn_count` 打回 0。这些值随启动回写落库 ⇒ 一次「自愈被预算跳过」就能把
//!    用户的会话标题**永久降级**。因此只在确实有内容时重算。
//! 2. **修复必须有预算，且全进程共用一条 deadline**。启动是交互路径：宁可「本轮按索引
//!    视图，下轮再补」，也不能把首帧拖到秒级。预算由 `PONY_STARTUP_SELF_HEAL_BUDGET_MS`
//!    覆盖（`0` = 跳过全部启动自愈），默认 [`DEFAULT_STARTUP_SELF_HEAL_BUDGET_MS`]。
//!
//! 预算一次成型（进程级 `OnceLock`）而不是每个入口各算一次：`load_store`（日志 fold、索引
//! 投影、事件重建）与 `with_backend`（trace 投影缓存重建）在同一次启动里先后发生，各算一次
//! 就把「启动自愈上限」变成两倍、三倍。

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::backend::{PersistCommand, PersistCommandOutcome, SessionBackend};
use super::store::{fold_session_traces_all, refresh_session_metadata};
use super::types::SessionState;

/// 启动期自愈总预算（毫秒）。超预算 ⇒ 本轮跳过剩余自愈并落一条结构化日志。
pub(crate) const DEFAULT_STARTUP_SELF_HEAL_BUDGET_MS: u64 = 1200;

/// 当前**这一次装载**的启动预算截止时刻。
///
/// ADR 0107 修复（原 ADR 0106 缺陷）：旧实现用进程级 `OnceLock` 在**进程首次**调用时冻结
/// deadline，长驻进程里后续每次 `load_store()`（`/resume`、控制面重整、二次 `SessionStore::new()`）
/// 都会读到已过期的 deadline ⇒ 历史重建被**永久**跳过（实测表现：新一轮上下文为空）。
/// 现改为 `Mutex` 状态，由 [`reset_startup_budget`] 在**每次 `load_store` 开始时重起算**：
/// 同一次装载内的多个自愈入口仍共用一条 deadline（原意保留），换一次装载就是新预算。
static STARTUP_BUDGET_DEADLINE: Mutex<Option<Option<Instant>>> = Mutex::new(None);

/// 开始一次新的装载预算：下一次 [`startup_budget_deadline`] 以当前时刻重新起算。
///
/// 调用点：`SqliteSessionBackend::load_store` 的入口（每次装载一次）。
pub fn reset_startup_budget() {
    if let Ok(mut slot) = STARTUP_BUDGET_DEADLINE.lock() {
        *slot = None;
    }
}

/// 本次装载的绝对截止时刻（首次调用时按当前时刻冻结；`None` = 不设上限）。
///
/// `PONY_STARTUP_SELF_HEAL_BUDGET_MS` 非法值一律回退默认（不 panic、不静默变成 0）。
pub fn startup_budget_deadline() -> Option<Instant> {
    let mut slot = match STARTUP_BUDGET_DEADLINE.lock() {
        Ok(slot) => slot,
        Err(poisoned) => poisoned.into_inner(),
    };
    if let Some(deadline) = *slot {
        return deadline;
    }
    let budget_ms = std::env::var("PONY_STARTUP_SELF_HEAL_BUDGET_MS")
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_STARTUP_SELF_HEAL_BUDGET_MS);
    let deadline = Instant::now().checked_add(Duration::from_millis(budget_ms));
    *slot = Some(deadline);
    deadline
}

/// 预算是否已耗尽：`None`（不设上限）永不计时；`0ms` 预算在第一次调用即到期。
pub fn budget_exhausted(deadline: Option<Instant>) -> bool {
    matches!(deadline, Some(deadline) if Instant::now() >= deadline)
}

/// 该会话在**索引之外**是否还有内容（history 或 trace 至少一边非空）。
///
/// 两边都空 = 轻量索引行 / 本轮自愈被预算跳过的空壳：派生字段只能沿用索引里已有的值。
pub fn session_has_indexed_content(session: &SessionState) -> bool {
    !session.history.is_empty() || !session.turn_trace_history.is_empty()
}

/// 只在有内容时重算 title / turn_count / summary；返回**是否有字段发生变化**。
///
/// 调用点语义与旧的 `refresh_session_metadata(session, false) + 比对` 等价，差别只有一条：
/// 空壳会话跳过重算（见模块文档不变量 1）。
pub fn refresh_metadata_if_content_present(session: &mut SessionState) -> bool {
    if !session_has_indexed_content(session) {
        return false;
    }
    let previous_title = session.title.clone();
    let previous_turn_count = session.turn_count;
    let previous_summary = session.summary.clone();
    refresh_session_metadata(session, false);
    session.title != previous_title
        || session.turn_count != previous_turn_count
        || session.summary != previous_summary
}

/// trace 投影缓存为空、但事件流存在的会话：从事件重建并**逐会话**回写 trace 表。
///
/// 返回 `true` 仅当「投影缓存回写失败」——此时需要一次整包提交来兜底修复；正常重建不回写
/// 整包（`AppendTrace` 已经把它按行落盘了），这是消灭启动期 2.3s 整包写的关键。
pub fn refold_missing_trace_cache(
    backend: &dyn SessionBackend,
    sessions: &mut HashMap<String, SessionState>,
    deadline: Option<Instant>,
) -> bool {
    let pending: Vec<String> = sessions
        .iter()
        .filter(|(_, session)| session.turn_trace_history.is_empty())
        .map(|(session_id, _)| session_id.clone())
        .collect();
    if pending.is_empty() {
        return false;
    }
    if budget_exhausted(deadline) {
        eprintln!(
            "[pony-agent][session] startup_self_heal_skipped: component=trace_cache_refold reason=budget_exhausted sessions={} — 本轮不重建 trace 投影缓存（下轮启动再补）",
            pending.len()
        );
        return false;
    }
    let mut needs_full_commit = false;
    for session_id in pending {
        if budget_exhausted(deadline) {
            eprintln!(
                "[pony-agent][session] startup_self_heal_skipped: component=trace_cache_refold reason=budget_exhausted remaining≈{session_id} — 剩余会话本轮跳过（下轮启动再补）"
            );
            break;
        }
        let events = backend.load_turn_events(&session_id, None);
        if events.is_empty() {
            continue;
        }
        let traces = fold_session_traces_all(&events);
        if traces.is_empty() {
            continue;
        }
        // 水位取终态 seq：下次启动据此判断缓存是否已覆盖事件流。
        let watermark = events.last().map(|(seq, _branch, _event)| *seq).unwrap_or(0);
        for (order, trace) in traces.iter().enumerate() {
            let mut cache_trace = trace.clone();
            cache_trace.sequence = Some(watermark);
            let outcome = backend.persist_command(PersistCommand::AppendTrace {
                epoch: 1,
                session_id: session_id.clone(),
                trace: cache_trace,
                trace_order: order,
            });
            if matches!(outcome, PersistCommandOutcome::Failed) {
                // 投影缓存回写失败不得静默（只影响缓存重建，但必须可排障）。
                eprintln!(
                    "[pony-agent][session] trace 投影缓存回写失败（session={session_id} order={order}）：事件流可重建，本次启动缓存不完整"
                );
                needs_full_commit = true;
            }
        }
        if let Some(session) = sessions.get_mut(session_id.as_str()) {
            session.turn_trace_history = traces;
        }
    }
    needs_full_commit
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 停写 blob 后的轻量索引行形态：内容为空，索引字段（title / turnCount）是真值。
    fn lightweight_index_row() -> SessionState {
        serde_json::from_str(
            r#"{
                "conversationId": "pa_index_only",
                "title": "真实标题（来自索引行）",
                "summary": "Pony Agent 本地开发会话",
                "history": [],
                "turnCount": 2,
                "lastReferencedFile": null,
                "updatedAtMs": 1794220832138,
                "__index_only": true
            }"#,
        )
        .expect("deserialize lightweight index row")
    }

    #[test]
    fn empty_index_row_keeps_derived_metadata() {
        let mut session = lightweight_index_row();
        assert!(
            !session_has_indexed_content(&session),
            "轻量索引行没有内容"
        );
        assert!(
            !refresh_metadata_if_content_present(&mut session),
            "空壳会话不得被判成「需要回写」"
        );
        assert_eq!(
            session.title, "真实标题（来自索引行）",
            "空壳会话的标题不得被打回默认值（否则随启动回写永久降级）"
        );
        assert_eq!(session.turn_count, 2, "空壳会话的轮次不得被打回 0");
    }

    #[test]
    fn content_session_still_recomputes_derived_metadata() {
        let mut session = lightweight_index_row();
        session.history = vec![super::super::types::TurnHistoryMessage {
            role: "user".to_string(),
            content: "把启动页品牌标缩小到 1/3".to_string(),
            attachments: Vec::new(),
            turn_id: Some("turn-1".to_string()),
            status: None,
            model_name: None,
            token_count: None,
            reasoning_content: None,
        }];
        assert!(session_has_indexed_content(&session));
        assert!(
            refresh_metadata_if_content_present(&mut session),
            "有内容时口径与旧实现一致：重算并报告差异"
        );
        assert_eq!(session.turn_count, 1, "轮次按真实用户消息重算");
    }

    #[test]
    fn exhausted_budget_is_detected_and_unlimited_is_not() {
        assert!(!budget_exhausted(None), "不设上限永不计时");
        let past = Instant::now()
            .checked_sub(Duration::from_millis(1))
            .expect("past instant");
        assert!(budget_exhausted(Some(past)), "已过去的截止时刻必须判定为耗尽");
    }
}
