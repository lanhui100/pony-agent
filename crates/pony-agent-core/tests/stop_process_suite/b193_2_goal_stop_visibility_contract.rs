//! b193-2 契约：goal 续轮停止原因的**结构化拆分**（对齐 deepseek-harness `GoalBlockReason`）。
//!
//! 本文件只锁「一个事实一个家」的机器可判面，不重复 core 内单测（`agent::goal::b130_event_tests`）：
//!
//! 1. **日志与枚举同源**：`code()` 十值唯一、snake_case，且与历史上 `goal_rounds_stopped.reason`
//!    的取值逐字一致——`tui.rs` 里那份手写字符串表已删除，此处是防它复活的锚。
//! 2. **严重级由枚举给**：`FirstRoundNotSucceeded` 必须是 `Error`。旧实现用关键词表判级，
//!    该文案不含 `error`/`失败` 之外的 error 词 ⇒ 被降级成 Info 灰字，
//!    **为可见性新增的消息反而成了视觉权重最低的一条**（B193 目的被反向实现）。
//! 3. **文案不得回退语义区分**：`FirstRoundNotSucceeded` 必须说「未启动」且绝不说「已停止」。
//! 4. **历史准入**：`LoopAlreadyOwned` 不写消息区（事实是「另一个循环在跑，续轮并没有停」）。
//! 5. **禁用词**：用户可见文案不得外泄内部术语（`收口` / `落盘`）。
//!
//! 契约真源：`.dev-team/contract-matrix-pa171.md` → `Amendment: b193-2`。

use pony_agent_core::agent::goal::{GoalRoundStopReason, GoalStopSeverity};

/// 1. `code()` 是结构化日志 `reason` 字段的**唯一来源**。
///
/// 期望值与契约矩阵 `Amendment: b193-2` 逐字一致；若新增变体，本表必须同步
/// （漏登记 = 日志 / 配色 / 文案三处同时漏配）。
#[test]
fn goal_stop_codes_match_frozen_log_vocabulary() {
    let expected = [
        (GoalRoundStopReason::Cancelled, "cancelled"),
        (
            GoalRoundStopReason::ConsecutiveFailures { failures: 2 },
            "consecutive_failures",
        ),
        (
            GoalRoundStopReason::BudgetExhausted {
                rounds_started: 20,
                max_rounds: 20,
            },
            "budget_exhausted",
        ),
        (
            GoalRoundStopReason::AutoContinueBudget,
            "auto_continue_budget",
        ),
        (GoalRoundStopReason::TurnSuspended, "turn_suspended"),
        (GoalRoundStopReason::TurnUnclassified, "turn_unclassified"),
        (GoalRoundStopReason::LoopAlreadyOwned, "loop_already_owned"),
        (
            GoalRoundStopReason::BudgetPersistFailed,
            "budget_persist_failed",
        ),
        (GoalRoundStopReason::NoActiveGoal, "no_active_goal"),
        (
            GoalRoundStopReason::FirstRoundNotSucceeded,
            "first_round_not_succeeded",
        ),
    ];

    assert_eq!(
        expected.len(),
        GoalRoundStopReason::all_variants().len(),
        "b193-2：`all_variants()` 与本表的变体数必须一致（新增变体未登记会在此红）"
    );
    for (reason, code) in expected {
        assert_eq!(
            reason.code(),
            code,
            "b193-2：`code()` 必须逐字等于冻结的日志取值，实际 {reason:?} → {}",
            reason.code()
        );
        // lower_snake_case：只允许小写字母 / 数字 / 下划线，且首字符为小写字母。
        let mut chars = code.chars();
        assert!(
            chars.next().is_some_and(|c| c.is_ascii_lowercase()),
            "b193-2：`code()` 必须以小写字母开头：{code}"
        );
        assert!(
            code.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
            "b193-2：`code()` 必须是 lower_snake_case（仅 a-z0-9_）：{code}"
        );
    }
}

/// 1'. `code()` 唯一性：两个变体撞码会让日志侧无法区分停止原因。
#[test]
fn goal_stop_codes_are_unique() {
    let variants = GoalRoundStopReason::all_variants();
    let mut codes: Vec<&str> = variants.iter().map(|reason| reason.code()).collect();
    codes.sort_unstable();
    let before = codes.len();
    codes.dedup();
    assert_eq!(
        codes.len(),
        before,
        "b193-2：`code()` 必须唯一，实际重复：{codes:?}"
    );
}

/// 2. 严重级必须由枚举给：`FirstRoundNotSucceeded` 是 `Error`，不得回落到关键词表判成 Info。
#[test]
fn first_round_not_succeeded_is_error_severity() {
    assert_eq!(
        GoalRoundStopReason::FirstRoundNotSucceeded.severity(),
        GoalStopSeverity::Error,
        "b193-2：首轮失败必须是 Error 级（旧关键词表把它判成 Info 灰字，与 B193 目的相反）"
    );
    // 对照：同族「连续失败」也是 Error，两者视觉权重不得一个红一个灰。
    assert_eq!(
        GoalRoundStopReason::ConsecutiveFailures { failures: 2 }.severity(),
        GoalStopSeverity::Error,
        "b193-2：同族失败原因必须同级"
    );
    // 取消是用户主动动作，不应报警色。
    assert_eq!(
        GoalRoundStopReason::Cancelled.severity(),
        GoalStopSeverity::Info,
        "b193-2：用户取消不是错误，不得用 Error 级"
    );
}

/// 2'. 严重级不是从文案猜的：把它换成等价文案也应保持 Error（判据只看枚举）。
#[test]
fn severity_does_not_depend_on_wording() {
    let line = GoalRoundStopReason::FirstRoundNotSucceeded.visible_line();
    // 文案里**没有**「失败/错误/error」以外的错误词也无所谓——严重级根本不读文案。
    assert!(
        GoalRoundStopReason::FirstRoundNotSucceeded.severity() == GoalStopSeverity::Error,
        "b193-2：严重级必须只由枚举决定，与文案用词无关（实际文案：{line}）"
    );
}

/// 3. 语义区分不回退：首轮门「从未启动」，绝不能说「已停止」。
#[test]
fn first_round_not_succeeded_says_not_started_not_stopped() {
    let line = GoalRoundStopReason::FirstRoundNotSucceeded.visible_line();
    assert!(line.contains("未启动"), "b193-2：必须说「未启动」，实际：{line}");
    assert!(
        !line.contains("已停止"),
        "b193-2：首轮门从未进入续轮，不得说「已停止」，实际：{line}"
    );
    // 反例对照：真正跑起来后停下的原因必须说「已停止」，两者不能混为一谈。
    assert!(
        GoalRoundStopReason::Cancelled
            .visible_line()
            .contains("已停止"),
        "b193-2：真正停止的原因必须说「已停止」"
    );
}

/// 4. 历史准入：`LoopAlreadyOwned` 写进消息区就是一条假结论。
#[test]
fn loop_already_owned_does_not_enter_history() {
    assert!(
        !GoalRoundStopReason::LoopAlreadyOwned.enters_history(),
        "b193-2：另一个循环在跑 ≠ 续轮已停，不得写入会话可见历史"
    );
    for reason in GoalRoundStopReason::all_variants() {
        if reason == GoalRoundStopReason::LoopAlreadyOwned {
            continue;
        }
        assert!(
            reason.enters_history(),
            "b193-2：除 `LoopAlreadyOwned` 外都应留痕，实际被拒：{reason:?}"
        );
    }
}

/// 5. 用户可见文案不得外泄内部术语。
///
/// 唯一例外：`LoopAlreadyOwned` 的 detail 含「让位」——那是机制说明（本线程确实让位了），
/// 不是要求用户执行的内部动作，已在 core doc 注释里显式登记。
#[test]
fn user_facing_lines_avoid_internal_jargon() {
    let banned = ["收口", "落盘"];
    for reason in GoalRoundStopReason::all_variants() {
        let line = reason.visible_line();
        for term in banned {
            assert!(
                !line.contains(term),
                "b193-2：用户可见文案不得含内部术语「{term}」，实际 {reason:?} → {line}"
            );
        }
        // 状态行 / 横幅的既有译词是「进行中」（`GoalPhase::Active`），停止文案不得占用。
        assert!(
            !line.contains("进行中"),
            "b193-2：停止文案不得占用「进行中」（与阶段译词冲突），实际 {reason:?} → {line}"
        );
    }
}

/// 6. 三段式结构：`visible_line()` 必须以 `headline()` 开头，且给出可执行动作（除让位类）。
#[test]
fn visible_line_is_three_part_and_actionable() {
    for reason in GoalRoundStopReason::all_variants() {
        let line = reason.visible_line();
        assert!(
            line.starts_with(reason.headline()),
            "b193-2：`visible_line()` 必须以 `headline()` 开头，实际 {reason:?} → {line}"
        );
        if reason.enters_history() {
            assert!(
                reason.action_hint().is_some(),
                "b193-2：进历史的原因必须给下一步动作，实际 {reason:?} 无 action_hint"
            );
        }
    }
}

/// 7. `never_started()` 决定措辞：状态条据此在 `·停` / `·未续` 之间选，不得各自拼字面量。
#[test]
fn never_started_covers_the_three_non_stopped_reasons() {
    let mut never: Vec<&str> = GoalRoundStopReason::all_variants()
        .into_iter()
        .filter(|reason| reason.never_started())
        .map(|reason| reason.code())
        .collect();
    never.sort_unstable();
    assert_eq!(
        never,
        vec![
            "first_round_not_succeeded",
            "loop_already_owned",
            "no_active_goal"
        ],
        "b193-2：只有这三个原因「从未启动」，状态条不得把它们说成「停」"
    );
}
