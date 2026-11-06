//! L2-AT 对抗用例 · TUI 流式排队派发：单飞 / 容量边界 / 拒绝不变量 / 回队幂等（A7 + A8 + A10 + A26）
//!
//! 真值源（只读，Lead 冻结 + 裁决修订）：`.dev-team/contract-matrix-tui-streaming-input.md`
//!   §1 C5/C6/C10/C15/C26 · §3 NFR `queue_drain_single_flight` · §4 A7/A8/A10/A26。
//!
//! 权威符号（裁决口径）：`MAX_QUEUED_MESSAGES`、`push_queued_message(&mut Vec<String>, &str)`、
//!   `drain_next_queued(&mut TuiState)`、`TuiState::{queued_user_messages, queued_count}`，
//!   以及 **C15 生产派发点** `take_next_queued_for_dispatch(&mut TuiState, is_streaming, ui_idle,
//!   turn_in_flight, dispatch_pending) -> Option<QueuedUserMessage>`。
//!
//! ⚠ A7 口径修订（2026-11-06，L3-B B2 判定假绿后重写）：
//!   旧版 A7 断言的是测试内自建 `SingleFlightDispatch` 闩（生产派发点零引用）⇒ 假绿。
//!   现 A7 **只断言生产符号 `take_next_queued_for_dispatch`**（四个闸门参数 + 草稿保护 + FIFO +
//!   回灌幂等），测试内自建闩已删除；`drain_next_queued` 的批量语义测试降级为「补充说明」，
//!   明确**不再作为单飞唯一证据**。
//!
//! 确定性纪律（防 Flaky）：
//! - 固定迭代次数，**无 `sleep`、无概率等待、无超时轮询**；
//! - 并发只经 `std::thread::scope` + `Mutex` 的真实竞争，不做时序假设；
//! - 断言全部落在不变量上（计数单调、长度守恒、每步恰好 −1、无重复派发）。

#[path = "../src/bin/tui.rs"]
#[allow(dead_code, unused_imports, unused_variables, unused_mut)]
mod tui;

/// 在 `&mut Vec<String>` 上执行唯一权威的 drain 原语（`drain_next_queued` 只接受 `&mut TuiState`）。
///
/// 用一次 swap 把真实队列挂到真实 `TuiState` 上：既坚持冻结签名，又让并发压力落在
/// **同一个**队列载荷上（不是复制品），保证断言验的是真实数据。
fn drain_via_state(queue: &mut Vec<String>) -> Option<String> {
    let mut state = tui::TuiState::default();
    std::mem::swap(&mut state.queued_user_messages, queue);
    let drained = tui::drain_next_queued(&mut state);
    std::mem::swap(&mut state.queued_user_messages, queue);
    drained
}

/// 造一个已排好队的状态（入队走真实原语，失败即 Arrange 失败）。
fn seeded_state(items: &[&str]) -> tui::TuiState {
    let mut state = tui::TuiState::default();
    for item in items {
        assert!(
            tui::push_queued_message(&mut state.queued_user_messages, item),
            "Arrange 失败：{item:?} 必须能入队"
        );
    }
    state
}

// ── A7 · L2-AT ─ C15/C5：生产派发点 `take_next_queued_for_dispatch` 单飞 ──────
//
// L3-B B2（2026-11-06）：旧版 A7 断言测试内自建闩 ⇒ 生产派发点零引用 ⇒ 假绿。
// 本版全部断言直接落在生产符号上；自建闩已删除。

#[test]
fn a7_production_dispatch_gate_takes_exactly_one_and_respects_every_blocker() {
    // Arrange：3 条排队 + 空输入框。
    let mut state = seeded_state(&["q1", "q2", "q3"]);
    assert_eq!(state.queued_count(), 3, "Arrange：必须真的排了 3 条");
    assert!(
        state.input_buffer.trim().is_empty(),
        "Arrange：输入框必须为空（否则会被「草稿保护」挡住，测不到闸门本身）"
    );

    // Act 1：唯一放行组合 (is_streaming=false, ui_idle=true, turn_in_flight=false, dispatch_pending=false)
    //        ⇒ 只取队首 1 条，队列剩 2。
    let first = tui::take_next_queued_for_dispatch(&mut state, false, true, false, false);
    assert_eq!(
        first.as_deref(),
        Some("q1"),
        "C15/C5：放行组合必须取到队首（FIFO）"
    );
    assert_eq!(state.queued_count(), 2, "C15：一次派发只能 −1（不得批量连发）");
    state.clear_input(); // 隔离 C14 写回对后续断言的干扰（写回语义由 A13 专门断言）

    // Act 2：dispatch_pending=true（已有一条在派发中）⇒ None，且不消费队列（单飞）。
    let pending = tui::take_next_queued_for_dispatch(&mut state, false, true, false, true);
    assert_eq!(pending, None, "C15/C10：dispatch_pending 期间必须 None（单飞）");
    assert_eq!(state.queued_count(), 2, "C15：None 时不得消费队列");

    // Act 3：is_streaming=true ⇒ None。
    let streaming = tui::take_next_queued_for_dispatch(&mut state, true, true, false, false);
    assert_eq!(streaming, None, "C10/C15：流式中不得派发");
    assert_eq!(state.queued_count(), 2);

    // Act 4：ui_idle=false（面板/待作答占用）⇒ None。
    let not_idle = tui::take_next_queued_for_dispatch(&mut state, false, false, false, false);
    assert_eq!(
        not_idle, None,
        "C10/C15/C17：界面非空闲（面板/待作答挂起）不得派发"
    );
    assert_eq!(state.queued_count(), 2);

    // Act 5：turn_in_flight=true（轮次线程存活）⇒ None。
    let in_flight = tui::take_next_queued_for_dispatch(&mut state, false, true, true, false);
    assert_eq!(in_flight, None, "C10：轮次线程存活时不得派发（防并发 turn 撕裂历史）");
    assert_eq!(state.queued_count(), 2);

    // Act 6：输入框非空（用户草稿）⇒ None，且草稿不得被覆盖。
    state.set_input("正在写的草稿");
    let draft = tui::take_next_queued_for_dispatch(&mut state, false, true, false, false);
    assert_eq!(draft, None, "C15：输入框非空时不得派发（禁止覆盖用户草稿）");
    assert_eq!(
        state.input_buffer, "正在写的草稿",
        "C15：被拒绝的派发不得改动草稿"
    );
    assert_eq!(state.queued_count(), 2, "C15：被拒绝的派发不得消费队列");

    // Act 7：清空草稿后恢复派发，且仍保持 FIFO。
    state.clear_input();
    let second = tui::take_next_queued_for_dispatch(&mut state, false, true, false, false);
    assert_eq!(
        second.as_deref(),
        Some("q2"),
        "C5：恢复派发后必须继续 FIFO"
    );
    assert_eq!(state.queued_count(), 1);
}

#[test]
fn a7_consecutive_terminals_never_double_dispatch_via_production_gate() {
    // Arrange：3 条排队 + 连续三种终态（Completed → StopTerminal → Failed）。
    let terminals = ["StreamCompleted", "StopTerminal", "StreamFailed"];
    let mut state = seeded_state(&["q1", "q2", "q3"]);
    let mut dispatched: Vec<String> = Vec::new();
    let mut per_terminal: Vec<usize> = Vec::new();

    // Act：每次终态经生产闸门取件；同一终态内（dispatch_pending 已置位）再调必须 None。
    for terminal in terminals {
        let before = state.queued_count();
        state.clear_input();
        let taken = tui::take_next_queued_for_dispatch(&mut state, false, true, false, false);
        let dispatched_now = usize::from(taken.is_some());
        assert!(
            dispatched_now <= 1,
            "C5：单次终态 {terminal} 最多派发 1 条，实得 {dispatched_now}"
        );
        assert_eq!(
            before - state.queued_count(),
            dispatched_now,
            "C5：{terminal} 的队列减少量必须等于本次派发数"
        );
        if let Some(message) = taken {
            dispatched.push(message);
        }
        // 回灌/重入：同一终态内 dispatch_pending 已置位 ⇒ 生产闸门必须拒绝。
        let reentrant = tui::take_next_queued_for_dispatch(&mut state, false, true, false, true);
        assert_eq!(
            reentrant, None,
            "C5/C15：同一终态内 dispatch_pending=true 时重入必须 None（回灌幂等）"
        );
        per_terminal.push(dispatched_now);
    }

    // Assert
    assert_eq!(
        per_terminal,
        vec![1, 1, 1],
        "C5：3 条排队 + 3 次终态 ⇒ 每次终态派发 1 条（实测 {per_terminal:?}）"
    );
    let total: usize = per_terminal.iter().sum();
    assert!(
        total <= terminals.len(),
        "C5：派发总数 {total} 不得超过终态数 {}",
        terminals.len()
    );
    assert_eq!(
        dispatched,
        vec!["q1".to_string(), "q2".to_string(), "q3".to_string()],
        "C5：必须 FIFO 且无重复（双发会在这里显形）"
    );
    assert_eq!(state.queued_count(), 0, "C5：全部派发后队列必须为空");
}

#[test]
fn a7_production_gate_returns_none_on_empty_backlog() {
    // Arrange：队列仅 1 条，但连续 4 次终态（多于排队数）。
    let mut state = seeded_state(&["only-one"]);
    let mut per_terminal: Vec<usize> = Vec::new();
    let mut dispatched: Vec<String> = Vec::new();

    for _ in 0..4 {
        state.clear_input();
        let taken = tui::take_next_queued_for_dispatch(&mut state, false, true, false, false);
        per_terminal.push(usize::from(taken.is_some()));
        if let Some(message) = taken {
            dispatched.push(message);
        }
    }

    assert_eq!(
        per_terminal,
        vec![1, 0, 0, 0],
        "C5/C15：仅有 1 条可派发 ⇒ 后续终态必须 None（不得重复派发同一条、不得 panic）"
    );
    assert_eq!(dispatched, vec!["only-one".to_string()]);
    assert_eq!(
        state.queued_count(),
        0,
        "C5：派发干净后 count 必须为 0（usize 越界会先在下溢处 panic）"
    );
}

#[test]
fn a7_bulk_drain_loop_is_the_antipattern_production_gate_prevents() {
    // 补充说明（**不再作为单飞唯一证据**，单飞证据见上方三个生产闸门用例）：
    // 底层原语 `drain_next_queued` 每次只吐 1 条；主循环若绕过生产闸门写成 while-let 批量派发，
    // 一次终态就会掏空 3 条 ⇒ C15 要求「主循环只能经 take_next_queued_for_dispatch 派发」。
    let mut state = seeded_state(&["a", "b", "c"]);
    let mut bulk_dispatched = 0usize;
    while tui::drain_next_queued(&mut state).is_some() {
        bulk_dispatched += 1;
    }
    assert_eq!(
        bulk_dispatched, 3,
        "一次批量 drain 会掏空 3 条 ⇒ 生产闸门必须由主循环强制"
    );
    assert_eq!(state.queued_count(), 0);
}

// ── A8 · L2-AT ─ C6/C5：满队列高并发 push 不 panic / 不越界 / 不变负 ────────

#[test]
fn a8_capped_queue_survives_concurrent_push_and_drain() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    const PUSHERS: usize = 8;
    const ITERS: usize = 64; // 固定迭代次数：无 sleep、无概率等待
    const WORKERS: usize = 6;
    const OPS: usize = 50;

    // 场景一（Arrange）：队列预填到上限，随后多线程并发重复 push。
    let queue = Arc::new(Mutex::new(Vec::<String>::new()));
    {
        let mut guard = queue.lock().expect("队列锁不得中毒");
        let mut i = 0usize;
        while guard.len() < tui::MAX_QUEUED_MESSAGES {
            assert!(
                tui::push_queued_message(&mut guard, &format!("seed-{i}")),
                "预填充第 {i} 条必须成功"
            );
            i += 1;
        }
        assert_eq!(guard.len(), tui::MAX_QUEUED_MESSAGES);
    }

    let rejected = Arc::new(AtomicUsize::new(0));
    std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for t in 0..PUSHERS {
            let queue = Arc::clone(&queue);
            let rejected = Arc::clone(&rejected);
            handles.push(scope.spawn(move || {
                for i in 0..ITERS {
                    let mut guard = queue.lock().expect("队列锁不得中毒");
                    let ok = tui::push_queued_message(&mut guard, &format!("t{t}-{i}"));
                    assert!(
                        guard.len() <= tui::MAX_QUEUED_MESSAGES,
                        "C6：并发 push 后长度越界：len={} > MAX={}",
                        guard.len(),
                        tui::MAX_QUEUED_MESSAGES
                    );
                    if !ok {
                        rejected.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }));
        }
        for handle in handles {
            handle.join().expect("并发 push 线程不得 panic");
        }
    });

    // Act + Assert：满队列下所有并发 push 都必须被拒绝，既有条目一个不丢。
    {
        let guard = queue.lock().expect("队列锁不得中毒");
        assert_eq!(
            guard.len(),
            tui::MAX_QUEUED_MESSAGES,
            "C6：满队列被并发 push 后长度不得变化（不得扩容）"
        );
        assert_eq!(
            rejected.load(Ordering::Relaxed),
            PUSHERS * ITERS,
            "C6：满队列下每次 push 都必须返回 false（不得静默丢弃/覆盖）"
        );
        for (i, item) in guard.iter().enumerate() {
            assert_eq!(
                item,
                &format!("seed-{i}"),
                "C6：既有排队条目不得被并发 push 覆盖或丢弃（index {i}）"
            );
        }
    }

    // 场景二：drain 后计数严格单调递减，每步恰好 −1，最终归零不为负。
    {
        let mut guard = queue.lock().expect("队列锁不得中毒");
        let mut state = tui::TuiState::default();
        std::mem::swap(&mut state.queued_user_messages, &mut guard);
        let mut prev = state.queued_count();
        assert_eq!(prev, tui::MAX_QUEUED_MESSAGES);
        let mut drained = 0usize;
        while let Some(message) = tui::drain_next_queued(&mut state) {
            let now = state.queued_count();
            assert_eq!(
                now + 1,
                prev,
                "C5：每次 drain 必须恰好 −1（prev={prev}, now={now}, msg={message}）"
            );
            assert!(now < prev, "C5：计数必须严格单调递减");
            prev = now;
            drained += 1;
            assert!(
                drained <= tui::MAX_QUEUED_MESSAGES,
                "drain 次数不得超过容量（出现重复派发）"
            );
        }
        assert_eq!(prev, 0, "C5：drain 干净后计数必须为 0（不得为负/残留）");
        assert_eq!(drained, tui::MAX_QUEUED_MESSAGES);
        std::mem::swap(&mut state.queued_user_messages, &mut guard);
        assert_eq!(guard.len(), 0);
    }

    // 场景三：push 与 drain 并发交错（固定迭代）——长度守恒不变式：
    // final_len == 入队成功数 − 成功派发数，且始终 0 ≤ len ≤ MAX。
    let queue = Arc::new(Mutex::new(Vec::<String>::new()));
    let accepted = Arc::new(AtomicUsize::new(0));
    let popped = Arc::new(AtomicUsize::new(0));
    let empty_drains = Arc::new(AtomicUsize::new(0));
    std::thread::scope(|scope| {
        for t in 0..WORKERS {
            let queue = Arc::clone(&queue);
            let accepted = Arc::clone(&accepted);
            let popped = Arc::clone(&popped);
            let empty_drains = Arc::clone(&empty_drains);
            scope.spawn(move || {
                for i in 0..OPS {
                    let mut guard = queue.lock().expect("队列锁不得中毒");
                    if (t + i) % 2 == 0 {
                        if tui::push_queued_message(&mut guard, &format!("m{t}-{i}")) {
                            accepted.fetch_add(1, Ordering::Relaxed);
                        }
                    } else if drain_via_state(&mut guard).is_some() {
                        popped.fetch_add(1, Ordering::Relaxed);
                    } else {
                        // 空队列上的 drain 返回 None：归入第三类计数，预算守恒才成立。
                        empty_drains.fetch_add(1, Ordering::Relaxed);
                    }
                    assert!(
                        guard.len() <= tui::MAX_QUEUED_MESSAGES,
                        "C6：交错 push/drain 后长度越界：len={}",
                        guard.len()
                    );
                }
            });
        }
    });

    let guard = queue.lock().expect("队列锁不得中毒");
    let accepted = accepted.load(Ordering::Relaxed);
    let popped = popped.load(Ordering::Relaxed);
    let empty_drains = empty_drains.load(Ordering::Relaxed);
    assert_eq!(
        guard.len(),
        accepted - popped,
        "C6/C5：长度守恒被破坏（accepted={accepted}, popped={popped}, len={}）",
        guard.len()
    );
    assert_eq!(
        accepted + popped + empty_drains,
        WORKERS * OPS,
        "固定迭代预算必须用尽（accepted={accepted}, popped={popped}, empty={empty_drains}）"
    );
    assert!(
        guard.len() <= tui::MAX_QUEUED_MESSAGES,
        "C6：最终长度必须在 [0, MAX] 区间内"
    );
}

// ── A10 · L2-AT ─ C6 边界：满队列持续 push 恒 false、长度恒 20、坏载荷恒 false ─

#[test]
fn a10_queue_cap_boundary_never_grows_and_rejects_bad_payloads() {
    // Arrange：把队列推满，记录每条载荷以便事后校验原序不丢。
    let mut state = tui::TuiState::default();
    let mut accepted = 0usize;
    while state.queued_count() < tui::MAX_QUEUED_MESSAGES {
        let payload = format!("payload-{}", state.queued_count());
        assert!(
            tui::push_queued_message(&mut state.queued_user_messages, &payload),
            "未达上限时必须接受: {payload}"
        );
        accepted += 1;
    }
    assert_eq!(accepted, tui::MAX_QUEUED_MESSAGES, "上限前必须恰好接受 20 条");
    assert_eq!(state.queued_count(), tui::MAX_QUEUED_MESSAGES);

    // Act：满队列后固定 40 次 push（正常载荷 / 命令串 / 纯空白交替）。
    let trials: Vec<String> = (0..40)
        .map(|i| match i % 4 {
            0 => format!("overflow-{i}"),
            1 => "/clear".to_string(),
            2 => "/agents".to_string(),
            _ => "   \n\t ".to_string(),
        })
        .collect();

    // Assert：一律 false，且长度恒为上限（不越界、不丢条目）。
    for text in &trials {
        assert!(
            !tui::push_queued_message(&mut state.queued_user_messages, text),
            "C6：满队列必须拒绝 {text:?}"
        );
        assert_eq!(
            state.queued_count(),
            tui::MAX_QUEUED_MESSAGES,
            "C6：拒绝后长度必须恒为 {MAX}（实得 {actual}，载荷 {text:?}）",
            MAX = tui::MAX_QUEUED_MESSAGES,
            actual = state.queued_count()
        );
    }

    // 既有载荷必须原序无损。
    for i in 0..tui::MAX_QUEUED_MESSAGES {
        assert_eq!(
            state.queued_user_messages[i],
            format!("payload-{i}"),
            "C6：满队列拒绝期间既有条目不得被覆盖/丢失（index {i}）"
        );
    }

    // 容量恢复：drain 腾出一条后，正常载荷必须重新可入队。
    assert_eq!(
        tui::drain_next_queued(&mut state).as_deref(),
        Some("payload-0"),
        "C5：腾位必须先派发队首"
    );
    assert_eq!(state.queued_count(), tui::MAX_QUEUED_MESSAGES - 1);
    assert!(
        tui::push_queued_message(&mut state.queued_user_messages, "again"),
        "C6：腾出空位后必须重新接受正常载荷"
    );
    assert_eq!(state.queued_count(), tui::MAX_QUEUED_MESSAGES);

    // 未满时坏载荷同样恒 false（把「坏载荷」与「被满拒绝」两种原因区分开）。
    let mut empty: Vec<String> = Vec::new();
    for bad in [
        "", " ", "\n\t ", "/clear", "/compact", "/agents", "/help", "/resume",
    ] {
        assert!(
            !tui::push_queued_message(&mut empty, bad),
            "C6：{bad:?} 永远不得入队"
        );
    }
    assert!(empty.is_empty(), "C6：被拒绝载荷不得留下残渣: {empty:?}");
}

// ── A26 · L2-AT ─ C26：trace-B 序列的回队幂等（不丢、不重复、不停摆）──────────

#[test]
fn a26_trace_b_sequence_restores_payload_without_loss_or_duplication() {
    // L3-A round4 的 trace-B：派发 m1 → 输入框被抹空 → 消费端把「空正文」当成用户草稿，
    // 于是 m1 **静默消失**（队列已 −1、正文也没了）。本用例用真实 TuiState 复刻该序列，
    // 断言恢复逻辑必须把载荷回队首；固定迭代、无定时器、无 sleep。
    let mut state = seeded_state(&["m1", "m2"]);

    for round in 0..2 {
        // 1) 经生产闸门取件，并按 C14 写回输入框。
        let taken = tui::take_next_queued_for_dispatch(&mut state, false, true, false, false)
            .unwrap_or_else(|| panic!("第 {round} 轮：放行组合必须能取到队首"));
        assert_eq!(taken, "m1", "第 {round} 轮：必须取到队首 m1（FIFO）");
        state.set_input(&tui::composer_buffer_for_dispatch(&taken));
        assert_eq!(state.queued_count(), 1, "第 {round} 轮：取件后队列应剩 1 条");

        // 2) 模拟 Ctrl+W 抹空输入框（trace-B 的触发条件）。
        state.clear_input();
        assert!(state.input_buffer.is_empty());

        // 3) 分歧必须被识别（否则 m1 会静默消失）。
        assert!(
            tui::queued_dispatch_diverged(Some(taken.as_str()), &state.input_buffer),
            "第 {round} 轮：C26 正文被抹空必须判分歧（trace-B 入口）"
        );

        // 4) 消费端恢复：回队首。
        tui::requeue_front(&mut state, taken);
        assert_eq!(
            state.queued_user_messages,
            vec!["m1".to_string(), "m2".to_string()],
            "第 {round} 轮：C26 恢复必须回到队首且顺序不乱（不得插到队尾）"
        );
        assert_eq!(state.queued_count(), 2, "第 {round} 轮：恢复后总条数必须仍为 2（无丢）");

        // 5) 不得停摆：闸门必须仍能取到 m1；随后回队复原（幂等）。
        let again = tui::take_next_queued_for_dispatch(&mut state, false, true, false, false);
        assert_eq!(
            again.as_deref(),
            Some("m1"),
            "第 {round} 轮：恢复后闸门必须重新能取到 m1（不得停摆）"
        );
        if let Some(message) = again {
            tui::requeue_front(&mut state, message);
        }
        assert_eq!(
            state.queued_user_messages,
            vec!["m1".to_string(), "m2".to_string()],
            "第 {round} 轮：往返后队列必须与初始完全一致（无丢、无重复、无乱序）"
        );
    }

    // 终态不变量：两轮往返后仍是 2 条且无重复。
    assert_eq!(state.queued_count(), 2, "C26：多轮往返后总条数恒为 2");
    let unique: std::collections::BTreeSet<&String> = state.queued_user_messages.iter().collect();
    assert_eq!(
        unique.len(),
        state.queued_count(),
        "C26：队列内不得出现重复载荷: {:?}",
        state.queued_user_messages
    );
}
