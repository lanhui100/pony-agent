//! 批2（task-7）验收测试：goal 轮次取消判定 / TerminalGuard RAII 幂等 / 输入泵错误分类。
//!
//! 沿用 `tests/tui_render_layout.rs` 已确立的 bin-as-module 模式（`#[path = "../src/bin/tui.rs"]`）：
//! `src/bin/tui.rs` 是 `[[bin]]` 目标，只能以模块方式引入**同一份真实源码**才能在编译期断言行为。
//! 本批三条被测面都是纯函数/RAII 结构，不需要 `TestBackend` 渲染。
//!
//! 用例与契约映射：
//! - N5 `should_abort_goal_rounds(cancelled, consecutive, max)`：取消 / 轮次上限的判定真值表；
//! - N6 `TerminalGuard`：drop 只恢复一次、`restore()` 幂等、restore 闭包 panic 时不得二次执行；
//! - N7 `classify_input_poll`：`Ok(_)` → Continue，`Err(io::Error)` → Stop（输入泵不得在错误上死循环）；
//! - N8 Esc 语义：**双层覆盖**。
//!   (a) `n8_cancellation_stops_the_goal_round_loop_at_the_next_round`：用 N5 纯函数做组合断言，
//!       模拟 goal 轮次循环，断言「按 Esc 后当轮即中止、未按 Esc 时恰好在上限轮中止」；
//!   (b) `n8b_cancel_token_drives_the_round_abort_decision`：exec-rust 已暴露可测入口
//!       `GoalRoundCancelToken`（`new/cancel/reset/is_cancelled`，`Arc<AtomicBool>` 共享态），
//!       故直接断言 Esc→`cancel()`→判定中止、新消息→`reset()`→恢复续轮，并额外锁住
//!       **克隆句柄共享取消位**（深拷贝会让跨线程取消永远失效，是该类缺陷的典型形态）。
//!
//! 签名对齐说明：N5/N6/N7 的调用形状已与 exec-rust 落地签名核对一致，未做任何断言语义调整。
//! 铁律：每条用例都有真实断言；无 `#[ignore]`/skip/「只运行不报错」；不修改任何实现代码。

#[path = "../src/bin/tui.rs"]
#[allow(dead_code, unused_imports, unused_variables, unused_mut)]
mod tui;

use std::io;
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

// ── N5：goal 轮次取消判定真值表 ─────────────────────────────────────────────────────

#[test]
fn n5_goal_round_abort_truth_table() {
    // 未取消且未达上限 ⇒ 继续（false）。
    assert!(
        !tui::should_abort_goal_rounds(false, 0, 10),
        "N5: 未取消、0 轮连续计数时不得中止"
    );
    assert!(
        !tui::should_abort_goal_rounds(false, 9, 10),
        "N5: 未取消、9/10 轮时不得中止"
    );

    // 达到上限 ⇒ 中止（true），且越界同样中止。
    assert!(
        tui::should_abort_goal_rounds(false, 10, 10),
        "N5: consecutive == max 必须中止（否则 goal 循环永不收敛）"
    );
    assert!(
        tui::should_abort_goal_rounds(false, 11, 10),
        "N5: consecutive > max 必须中止"
    );

    // 已取消 ⇒ 无论轮次计数如何都必须中止（Esc 必须立即生效）。
    assert!(
        tui::should_abort_goal_rounds(true, 0, 10),
        "N5: cancelled=true 时必须中止（即使还没跑任何一轮）"
    );
    assert!(
        tui::should_abort_goal_rounds(true, 9, 10),
        "N5: cancelled=true 时必须中止（未达上限也要停）"
    );
    assert!(
        tui::should_abort_goal_rounds(true, 10, 10),
        "N5: cancelled=true 且已达上限时必须中止"
    );
}

// ── N6：TerminalGuard RAII 幂等 ────────────────────────────────────────────────────

#[test]
fn n6a_guard_restores_exactly_once_on_drop() {
    let restore_calls = Arc::new(AtomicUsize::new(0));
    {
        let calls = Arc::clone(&restore_calls);
        let guard = tui::TerminalGuard::new(move || {
            calls.fetch_add(1, Ordering::SeqCst);
        });
        // 守卫活着时不执行恢复（终端仍在 TUI 模式）。
        assert_eq!(
            restore_calls.load(Ordering::SeqCst),
            0,
            "N6a: 守卫存活期间不得提前执行恢复"
        );
        drop(guard);
    }

    assert_eq!(
        restore_calls.load(Ordering::SeqCst),
        1,
        "N6a: drop 必须恰好执行一次恢复（一次进入 ⇔ 一次恢复）"
    );
}

#[test]
fn n6b_manual_restore_is_idempotent_and_drop_does_not_rerun() {
    let restore_calls = Arc::new(AtomicUsize::new(0));
    {
        let calls = Arc::clone(&restore_calls);
        let mut guard = tui::TerminalGuard::new(move || {
            calls.fetch_add(1, Ordering::SeqCst);
        });

        guard.restore();
        assert_eq!(
            restore_calls.load(Ordering::SeqCst),
            1,
            "N6b: 显式 restore() 必须执行一次恢复"
        );

        // 重复调用必须幂等（否则会重复 disable_raw_mode / LeaveAlternateScreen）。
        guard.restore();
        assert_eq!(
            restore_calls.load(Ordering::SeqCst),
            1,
            "N6b: 第二次 restore() 必须幂等（不得再次执行恢复闭包）"
        );
    }

    assert_eq!(
        restore_calls.load(Ordering::SeqCst),
        1,
        "N6b: 已显式 restore 后 drop 不得再次执行恢复（drop 与 restore 共用同一幂等位）"
    );
}

#[test]
fn n6c_panicking_restore_closure_runs_exactly_once() {
    let restore_calls = Arc::new(AtomicUsize::new(0));
    let calls = Arc::clone(&restore_calls);

    // 正常作用域结束时 drop 触发 restore，而 restore 闭包内 panic。
    // 用 catch_unwind 包住：既接受「实现内部 catch 后吞掉 panic」，也接受「panic 继续传播」，
    // 但两种情况都必须只执行**一次**恢复闭包（二次执行会让终端状态机进入未定义态）。
    let outcome = panic::catch_unwind(AssertUnwindSafe(move || {
        let guard = tui::TerminalGuard::new(move || {
            calls.fetch_add(1, Ordering::SeqCst);
            panic!("restore closure exploded");
        });
        drop(guard);
    }));

    assert_eq!(
        restore_calls.load(Ordering::SeqCst),
        1,
        "N6c: restore 闭包 panic 时也必须恰好执行一次（不得二次执行）"
    );

    match outcome {
        Ok(()) => {
            // 实现侧在守卫内部 catch 了 panic（不传播）——同样必须只执行一次（上面已断言）。
            assert_eq!(
                restore_calls.load(Ordering::SeqCst),
                1,
                "N6c: panic 被吞掉的分支下仍不得重复执行恢复"
            );
        }
        Err(payload) => {
            // panic 传播出来：必须是闭包原始的 panic，而不是守卫自身包装出来的二次 panic。
            let message = payload
                .downcast_ref::<&str>()
                .copied()
                .unwrap_or("<非 &str panic payload>");
            assert_eq!(
                message, "restore closure exploded",
                "N6c: 传播出的 panic 必须是 restore 闭包的原始 panic（不得被守卫二次 panic 覆盖）"
            );
        }
    }
}

// ── N7：输入泵 poll 错误分类 ────────────────────────────────────────────────────────

#[test]
fn n7_ok_polls_continue_the_pump() {
    assert!(
        matches!(
            tui::classify_input_poll(io::Result::Ok(true)),
            tui::InputPumpAction::Continue
        ),
        "N7: poll 读到事件（Ok(true)）必须继续泵"
    );
    assert!(
        matches!(
            tui::classify_input_poll(io::Result::Ok(false)),
            tui::InputPumpAction::Continue
        ),
        "N7: poll 空转（Ok(false)）同样必须继续泵（不得误判为停止）"
    );
}

#[test]
fn n7_io_error_stops_the_pump() {
    // 注意：不能用 `io::ErrorKind::Interrupted` 之外的语义假设——这里只断言「任何 io 错误都停止泵」，
    // 因为输入泵在错误上继续空转会 100% CPU 死循环（批2 的真实缺陷）。
    let error = io::Error::from_raw_os_error(5);
    assert!(
        matches!(
            tui::classify_input_poll(io::Result::Err(error)),
            tui::InputPumpAction::Stop
        ),
        "N7: io 错误（EIO=5）必须停止输入泵，不得继续空转"
    );
}

// ── N8b：取消令牌直接断言（exec-rust 已暴露 `GoalRoundCancelToken`，升级为直连语义） ──
//
// 契约 F-G 的真实接线：Esc 分支调 `cancel_for(session)`、提交新消息调 `reset_for(session)`、
// 续轮循环每轮前用 `should_abort_goal_rounds(token.is_cancelled_for(session), consecutive, MAX)`。
// 这里直接锁住这三段接线，而不是只测纯函数（纯函数真值表由 N5 覆盖，二者互补）。
//
// 2026-11 修订：令牌由进程级 `Arc<AtomicBool>` 改为**按会话分区**——旧实现下 B 会话的新消息
// 会 `reset()` 掉 A 会话的取消位，A 的续轮因此被重新武装并跑进 B 的界面（跨会话串话）。

#[test]
fn n8b_cancel_token_drives_the_round_abort_decision() {
    const SESSION: &str = "session-n8b";
    let token = tui::GoalRoundCancelToken::new();
    assert!(
        !token.is_cancelled_for(SESSION),
        "N8b: 新令牌必须处于未取消态"
    );
    // 未取消 + 未达上限 ⇒ 继续续轮。
    assert!(
        !tui::should_abort_goal_rounds(token.is_cancelled_for(SESSION), 0, 8),
        "N8b: 未取消时不得中止续轮"
    );

    // Esc 置位：续轮判定必须立刻变为中止（即使一轮都没跑）。
    token.cancel_for(SESSION);
    assert!(
        token.is_cancelled_for(SESSION),
        "N8b: cancel_for() 后 is_cancelled_for() 必须为 true"
    );
    assert!(
        tui::should_abort_goal_rounds(token.is_cancelled_for(SESSION), 0, 8),
        "N8b: Esc 后 must-abort 判定必须成立（不得再续轮）"
    );

    // 克隆句柄必须共享同一取消位：Esc 在 UI 线程置位、续轮循环在 turn 线程读取，
    // 若克隆是深拷贝则跨线程取消永远失效（这是本类 bug 的典型形态）。
    let shared = token.clone();
    assert!(
        shared.is_cancelled_for(SESSION),
        "N8b: 克隆句柄必须共享取消位（Esc 线程置位要能被续轮线程看到）"
    );
    token.reset_for(SESSION);
    assert!(
        !shared.is_cancelled_for(SESSION),
        "N8b: reset_for() 必须对所有共享句柄同时生效（新一轮用户消息重置取消态）"
    );
    assert!(
        !tui::should_abort_goal_rounds(shared.is_cancelled_for(SESSION), 0, 8),
        "N8b: reset 后必须恢复续轮判定"
    );
}

/// 跨会话隔离（本次修复的核心回归）：取消位必须**按会话**分区。
///
/// - B 会话的 `cancel_for` 不得停掉 A 会话的续轮（各有各的 Esc）；
/// - B 会话的新消息 `reset_for(B)` 不得重新武装 A 会话（旧实现的串话根因）；
/// - 无会话事实 ⇒ fail-closed 视为已取消。
#[test]
fn n8b_cancel_token_is_partitioned_per_session() {
    const SESSION_A: &str = "session-a";
    const SESSION_B: &str = "session-b";
    let token = tui::GoalRoundCancelToken::new();

    // A 被取消后，B 的取消/重置都不得改变 A 的状态（反向亦然）。
    token.cancel_for(SESSION_A);
    assert!(token.is_cancelled_for(SESSION_A));
    assert!(
        !token.is_cancelled_for(SESSION_B),
        "N8b: A 会话的 Esc 不得停掉 B 会话的续轮"
    );

    token.reset_for(SESSION_B);
    assert!(
        token.is_cancelled_for(SESSION_A),
        "N8b: B 会话的新消息不得重新武装 A 会话（旧 Arc<AtomicBool> 的串话根因）"
    );

    // A 自己的新消息才重新武装 A。
    token.reset_for(SESSION_A);
    assert!(!token.is_cancelled_for(SESSION_A));

    // 空白/空会话 id ⇒ fail-closed：不知道该停谁，就按已取消处理。
    assert!(
        token.is_cancelled_for("  "),
        "N8b: 无会话事实必须 fail-closed（视为已取消）"
    );
}

// ── N8：Esc 后不再续轮（N5 组合断言，理由见文件头） ─────────────────────────────────

#[test]
fn n8_cancellation_stops_the_goal_round_loop_at_the_next_round() {
    const MAX_ROUNDS: u32 = 10;
    const CANCEL_AT_ROUND: u32 = 4;

    // 场景 A：用户在第 4 轮按下 Esc ⇒ 循环必须在同一轮立刻收口（不得再续到第 5 轮）。
    let mut aborted_at_round: Option<u32> = None;
    for round in 0..20u32 {
        // 模拟：第 CANCEL_AT_ROUND 轮起取消令牌置位（Esc 生效）。
        let cancelled = round >= CANCEL_AT_ROUND;
        if tui::should_abort_goal_rounds(cancelled, round, MAX_ROUNDS) {
            aborted_at_round = Some(round);
            break;
        }
    }
    assert_eq!(
        aborted_at_round,
        Some(CANCEL_AT_ROUND),
        "N8: 按 Esc 后必须在当轮立即中止 goal 循环（不得再续一轮）"
    );

    // 场景 B：从未按 Esc ⇒ 恰好在上限轮收口（既不提前、也不无限续轮）。
    let mut aborted_at_round: Option<u32> = None;
    for round in 0..50u32 {
        if tui::should_abort_goal_rounds(false, round, MAX_ROUNDS) {
            aborted_at_round = Some(round);
            break;
        }
    }
    assert_eq!(
        aborted_at_round,
        Some(MAX_ROUNDS),
        "N8: 未取消时必须恰好在上限轮中止 goal 循环"
    );

    // 场景 C：取消与达上限同时发生 ⇒ 仍必须中止（两个条件任一成立即可，不得互相掩盖）。
    assert!(
        tui::should_abort_goal_rounds(true, MAX_ROUNDS, MAX_ROUNDS),
        "N8: 取消与上限同时成立时必须中止"
    );
}

// ── N9：输入泵失败事件面（契约 F-E） ────────────────────────────────────────────────
//
// 背景：TUI 启动时 stderr 被 dup2 到日志文件，全屏界面是用户**唯一**可见面；输入泵因
// poll/read 出错退出时若只写日志，用户会看到「键盘鼠标突然失灵且毫无提示」。故要求失败
// 原因经 `UiEvent::InputPumpFailed { error }` 送到事件循环。
//
// 断言范围说明（如实）：事件循环侧的展示文案（`state.status_text` / System 消息）当前**内联在
// 事件循环函数体**里，没有可独立调用的纯函数入口（无 `input_pump_failure_message(&str)` 之类），
// 因此本条只冻结**事件面**：变体存在、载荷为非空可读串且保留可辨识错误片段。若后续抽出文案
// 纯函数，可无损追加文案断言。

#[test]
fn n9_input_pump_failure_event_carries_readable_error() {
    let error_text = format!("poll failed: {}", io::Error::from_raw_os_error(5));
    let event = tui::UiEvent::InputPumpFailed {
        error: error_text.clone(),
    };

    match event {
        tui::UiEvent::InputPumpFailed { error } => {
            assert!(
                !error.is_empty(),
                "N9: InputPumpFailed.error 不得为空（否则用户只看到一条无诊断的告警）"
            );
            assert!(
                error.contains("poll failed"),
                "N9: 错误载荷必须保留可辨识片段（poll failed），实际 = {error:?}"
            );
            assert_eq!(
                error, error_text,
                "N9: 事件载荷必须原样携带调用方给出的错误文本（不得在构造处静默截断/改写）"
            );
        }
        // 注：`UiEvent` 未 derive Debug（实现侧未提供），故此处不做 Debug 格式化——
        // 该分支只是穷尽性兜底，真正的判别力来自上面三个载荷断言。
        _ => panic!("N9: 构造出的变体必须是 InputPumpFailed"),
    }
}

/// 契约 F-G3：会话里仍有待作答的 Ask / 审批请求时，goal 续轮必须让位给人。
///
/// 回归面：旧实现只看「取消 / 轮次上限」，于是模型 Ask 挂起后立刻开下一轮，新一轮入口把
/// 该提问判为过期（面板「该提问已失效」），模型只能重复提问 —— 实测连续 16 轮空转。
#[test]
fn n5b_goal_rounds_yield_to_pending_host_requests() {
    assert!(
        !tui::goal_rounds_must_yield_to_pending_requests(0, 0),
        "N5b: 没有待作答请求时续轮照常推进"
    );
    assert!(
        tui::goal_rounds_must_yield_to_pending_requests(1, 0),
        "N5b: 存在待作答 Ask 时必须停止续轮"
    );
    assert!(
        tui::goal_rounds_must_yield_to_pending_requests(0, 1),
        "N5b: 存在待决审批时必须停止续轮"
    );
    assert!(
        tui::goal_rounds_must_yield_to_pending_requests(3, 2),
        "N5b: 多请求同样必须停止续轮"
    );
}
