//! B239：ACP 委派事件缓冲的并发去重状态（`(session,turn)` in-flight 幂等键）。
//!
//! 从 `mod.rs` 的 `build()` 闭包外移：`build` 有函数行数棘轮，且这里的
//! 「快照 → 提交 → 移除」协议本身就值得单独钉住。锁纪律：
//! 缓冲锁只在快照/收口两个短临界区持有，**绝不跨越 `sessions.write`**。

use super::BufferedTurnEvent;
use crate::agent::turn_event::TurnEvent;
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, MutexGuard};

pub(super) type BufferKey = (String, String);
/// 一次待提交快照：`(键, 事件批, 是否终态批)`。
pub(super) type Snapshot = (BufferKey, Vec<BufferedTurnEvent>, bool);

/// 入队事件的类别（用枚举替代 bool 形参：`fn_params_excessive_bools` 是 0 容忍）。
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum EventKind {
    Delta,
    Terminal,
}

/// 提交结果（成功可携带「是否终态批」；失败只释放幂等键）。
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum CommitOutcome {
    Committed { finalize: bool },
    Failed,
}

/// 进程级事件缓冲的键状态。
///
/// - `batches`：尚未提交的事件批次；
/// - `in_flight`：已快照、提交尚未完成的幂等键。并发的第二路 flush 见到即跳过，
///   保证同一批事件不会被提交两次；
/// - `terminal_pending`：该键已收到终态事件但 finalize 尚未执行。`finalize` 只在
///   终态提交成功后做一次（此时尚未 `append_turn`），step 边界增量 flush 不再
///   提前冻结上一轮的节点事件区间。
#[derive(Default)]
pub(super) struct EventBufferState {
    batches: HashMap<BufferKey, Vec<BufferedTurnEvent>>,
    in_flight: HashSet<BufferKey>,
    terminal_pending: HashSet<BufferKey>,
}

fn lock(buffer: &Mutex<EventBufferState>) -> MutexGuard<'_, EventBufferState> {
    buffer.lock().unwrap_or_else(|error| {
        eprintln!("[pony-agent][runtime] event buffer lock poisoned: {error}, recovering");
        error.into_inner()
    })
}

/// 入队一条事件；若它是终态且该键没有在途提交，返回需要立即提交的快照。
pub(super) fn enqueue(
    buffer: &Mutex<EventBufferState>,
    key: &BufferKey,
    event: TurnEvent,
    produced_at_ms: u64,
    kind: EventKind,
) -> Option<Vec<BufferedTurnEvent>> {
    let mut state = lock(buffer);
    let is_terminal = kind == EventKind::Terminal;
    if is_terminal {
        state.terminal_pending.insert(key.clone());
    }
    // 写时聚合：同一 turn 的连续 AssistantChunk 合并为一条。仅在键不在 in-flight
    // 时合并——快照里的末条 chunk 可能正在提交，再往它尾巴追加会把「已提交内容」
    // 改写成未提交版本。聚合段保留**首条** chunk 的产生时刻（时间下界）。
    let mut merged_chunk = false;
    if !state.in_flight.contains(key) {
        if let TurnEvent::AssistantChunk { text, .. } = &event {
            if let Some(last) = state.batches.get_mut(key).and_then(|batch| batch.last_mut()) {
                if let TurnEvent::AssistantChunk {
                    text: last_text, ..
                } = &mut last.event
                {
                    last_text.push_str(text);
                    merged_chunk = true;
                }
            }
        }
    }
    if !merged_chunk {
        state
            .batches
            .entry(key.clone())
            .or_default()
            .push(BufferedTurnEvent {
                event,
                produced_at_ms,
            });
    }
    if is_terminal && !state.in_flight.contains(key) {
        let events = state.batches.get(key).cloned().unwrap_or_default();
        if !events.is_empty() {
            state.in_flight.insert(key.clone());
            return Some(events);
        }
    }
    None
}

/// 快照某会话所有未在途的批次并标记 in-flight（缓冲锁内完成，提交在锁外）。
pub(super) fn drain(buffer: &Mutex<EventBufferState>, session_id: &str) -> Vec<Snapshot> {
    let mut state = lock(buffer);
    let mut keys: Vec<BufferKey> = state
        .batches
        .keys()
        .filter(|(sid, _)| sid == session_id)
        .cloned()
        .collect();
    // 稳定顺序：多 turn 提交顺序只由键排序决定，不随 HashMap 迭代顺序抖动。
    keys.sort();
    let mut snapshots = Vec::new();
    for key in keys {
        if state.in_flight.contains(&key) {
            continue;
        }
        let events = state.batches.get(&key).cloned().unwrap_or_default();
        if events.is_empty() {
            continue;
        }
        let finalize = state.terminal_pending.contains(&key);
        state.in_flight.insert(key.clone());
        snapshots.push((key, events, finalize));
    }
    snapshots
}

/// 提交结束后的收口：成功只摘掉快照覆盖的**前缀**（快照与提交之间新入队的事件
/// 必须留在尾部），失败只释放幂等键、保留缓冲可重试。
pub(super) fn finish_commit(
    buffer: &Mutex<EventBufferState>,
    key: &BufferKey,
    committed: usize,
    outcome: CommitOutcome,
) {
    let mut state = lock(buffer);
    let (success, finalize) = match outcome {
        CommitOutcome::Committed { finalize } => (true, finalize),
        CommitOutcome::Failed => (false, false),
    };
    if success {
        let mut empty = false;
        if let Some(batch) = state.batches.get_mut(key) {
            let prefix = committed.min(batch.len());
            batch.drain(..prefix);
            empty = batch.is_empty();
        }
        if empty {
            state.batches.remove(key);
        }
        if finalize {
            state.terminal_pending.remove(key);
        }
    }
    state.in_flight.remove(key);
}

/// 提交失败/拿不到写锁时释放尚未提交的快照标记（缓冲保留可重试）。
pub(super) fn release_in_flight<'a>(
    buffer: &Mutex<EventBufferState>,
    keys: impl Iterator<Item = &'a BufferKey>,
) {
    let mut state = lock(buffer);
    for key in keys {
        state.in_flight.remove(key);
    }
}
