//! 轮末状态噪声回归：resume 误报旧失败 + 停止原因重复占两行。
//!
//! 两条缺陷的实测症状（用户会话 `pa_01a1f82e`）：
//!
//! 1. resume 后信息流末尾冒出 `✕ 上一轮因错误终止，已保留已执行的步骤记录。`，
//!    但末轮其实是正常完成的。旧实现用 `rev().find_map(..)` 反向找「能匹配状态行的记录」，
//!    正常完成的末条没有信号 ⇒ 扫描越过它继续往前翻，把 6 轮之前的旧 provider 失败贴到末尾。
//! 2. 轮末连续两行：`本轮完成 · 用时 … · 工具 N 次` 紧跟
//!    `目标已暂停：续轮已停止｜…`。后者与前者重复，且把状态事实塞进对话流。
//!
//! 判定口径：1 锁 `resume_interrupted_status_line`（历史形状 → 是否补行）；
//! 2 锁 `apply_goal_rounds_stopped`（只写状态行与结构化原因，不写消息区）。

use super::*;
use pony_agent_core::agent::goal::GoalRoundStopReason;
use pony_agent_core::agent::session::{HistoryMessageKind, TurnHistoryMessage};

const CHECKPOINT: &str = "[中断轮次检查点]\n已执行的工具步骤：\n- Run 探测";

fn row(role: &str, content: &str, kind: Option<HistoryMessageKind>) -> TurnHistoryMessage {
    TurnHistoryMessage {
        role: role.to_string(),
        content: content.to_string(),
        kind,
        ..Default::default()
    }
}

/// 主因复现：历史上出现过失败轮，但**末轮正常完成** ⇒ 不得补任何中断状态行。
///
/// 这正是用户现场：8 条 `kind=error` 检查点之后又跑了 6 轮正常对话，resume 仍报错。
#[test]
fn resume_does_not_resurrect_stale_error_after_successful_turn() {
    let history = vec![
        row("user", "第一次提问", None),
        row("assistant", CHECKPOINT, Some(HistoryMessageKind::Error)),
        row("user", "继续", None),
        row("assistant", "**已确认：目录已不存在**", None),
        row("user", "本轮会话的 id 是什么？", None),
        row("assistant", "会话 ID：`pa_01a1f82e`", None),
    ];

    assert_eq!(
        errors::resume_interrupted_status_line(&history),
        None,
        "末轮正常完成时不得补中断状态行（旧实现会回溯到第 2 行那条旧失败）"
    );
}

/// 末轮真的失败 ⇒ 仍然补行，且文案必须是「因错误终止」而非「已中断」。
#[test]
fn resume_reports_error_only_when_last_assistant_row_is_error() {
    let history = vec![
        row("user", "提问", None),
        row("assistant", "上一轮正常回答", None),
        row("user", "再问", None),
        row("assistant", CHECKPOINT, Some(HistoryMessageKind::Error)),
    ];

    assert_eq!(
        errors::resume_interrupted_status_line(&history),
        Some(errors::INTERRUPTED_ERROR_LINE)
    );
}

/// 取消 / 中断两种终态各自的文案仍分流（B217 语义不因本次修复回退）。
#[test]
fn resume_keeps_cancelled_and_interrupted_distinct() {
    let cancelled = vec![row(
        "assistant",
        CHECKPOINT,
        Some(HistoryMessageKind::Cancelled),
    )];
    assert_eq!(
        errors::resume_interrupted_status_line(&cancelled),
        Some(errors::INTERRUPTED_CANCELLED_LINE)
    );

    let interrupted = vec![row(
        "assistant",
        CHECKPOINT,
        Some(HistoryMessageKind::Interrupted),
    )];
    assert_eq!(
        errors::resume_interrupted_status_line(&interrupted),
        Some(errors::INTERRUPTED_FAILED_LINE)
    );
}

/// 旧会话无 `kind`：末条正文带检查点标记时仍按标记回退（零迁移）。
#[test]
fn resume_legacy_marker_row_still_falls_back() {
    let history = vec![row("user", "提问", None), row("assistant", CHECKPOINT, None)];
    assert_eq!(
        errors::resume_interrupted_status_line(&history),
        Some(errors::INTERRUPTED_FAILED_LINE)
    );
}

/// 末条不是 assistant（末轮尚未产出回复）⇒ 没有「上一轮」可断言，不补行。
#[test]
fn resume_silent_when_last_row_is_not_assistant() {
    let history = vec![
        row("assistant", CHECKPOINT, Some(HistoryMessageKind::Error)),
        row("user", "还在等回复", None),
    ];
    assert_eq!(errors::resume_interrupted_status_line(&history), None);
    assert_eq!(errors::resume_interrupted_status_line(&[]), None);
}

/// 停止原因不得写进消息区：轮末只留 B216 那一行终态摘要。
#[test]
fn goal_rounds_stopped_does_not_push_message_area_row() {
    let mut state = TuiState::default();
    state.turn_started_at = Some(std::time::Instant::now());
    state.push_settled_turn_summary(SettledTurnOutcome::Completed);
    let after_summary = state.messages.len();
    assert_eq!(after_summary, 1, "轮末摘要必须落一行");

    state.apply_goal_rounds_stopped(
        GoalRoundStopReason::TurnSuspended,
        GoalRoundStopReason::TurnSuspended.visible_line(),
    );

    assert_eq!(
        state.messages.len(),
        after_summary,
        "停止原因只进状态栏 / 面板 / 日志，不得再往消息区补第二行"
    );
    assert!(
        state.messages[0].content.starts_with("本轮完成"),
        "消息区唯一一行必须是终态摘要，实际 = {:?}",
        state.messages[0].content
    );
}

/// 停止原因不写消息区，但可见性三条通道一条都不能少。
#[test]
fn goal_rounds_stopped_keeps_visibility_channels() {
    let reason = GoalRoundStopReason::FirstRoundNotSucceeded;
    let line = reason.visible_line();
    let mut state = TuiState::default();
    state.apply_goal_rounds_stopped(reason.clone(), line.clone());

    assert_eq!(
        state.status_text, line,
        "状态行必须逐字等于 core 的 visible_line"
    );
    assert_eq!(
        state.goal_rounds_stopped,
        Some(reason),
        "结构化原因必须留痕（状态栏 `·未续` 与 /goal 面板据此取措辞）"
    );
}

/// B276：失败轮同样只留一行——「本轮失败 · 工具 N 次」之后不得再跟一条「目标已暂停」。
///
/// 现场症状：重试 5 次后失败，消息区连续两行
/// `✕ 本轮失败 · 工具 5 次` + `✕ 目标已暂停：本轮未成功｜首轮失败，自动续轮未启动｜发「继续」重试。`
/// 后者把「未成功」与「本轮失败」同义复述。首轮门（`FirstRoundNotSucceeded`）是最容易
/// 与摘要撞车的原因，故单独锁一条。
#[test]
fn failed_turn_keeps_single_row_even_for_first_round_gate() {
    let mut state = TuiState::default();
    state.turn_started_at = Some(std::time::Instant::now());
    state.push_settled_turn_summary(SettledTurnOutcome::Failed);
    assert_eq!(state.messages.len(), 1, "失败轮摘要必须落一行");
    assert!(
        state.messages[0].content.starts_with("本轮失败"),
        "摘要首段必须是终态，实际 = {:?}",
        state.messages[0].content
    );

    state.apply_goal_rounds_stopped(
        GoalRoundStopReason::FirstRoundNotSucceeded,
        GoalRoundStopReason::FirstRoundNotSucceeded.visible_line(),
    );

    assert_eq!(
        state.messages.len(),
        1,
        "首轮门停止原因不得在失败摘要之后再补一行（两者语义重复）"
    );
    // 可见性仍由状态栏承担：状态行就是 core 那句完整文案。
    assert_eq!(
        state.status_text,
        GoalRoundStopReason::FirstRoundNotSucceeded.visible_line()
    );
}

/// B276：取消轮同理——「本轮已停止」之后不得再跟一条停止原因。
#[test]
fn cancelled_turn_keeps_single_row() {
    let mut state = TuiState::default();
    state.turn_started_at = Some(std::time::Instant::now());
    state.push_settled_turn_summary(SettledTurnOutcome::Cancelled);

    state.apply_goal_rounds_stopped(
        GoalRoundStopReason::Cancelled,
        GoalRoundStopReason::Cancelled.visible_line(),
    );

    assert_eq!(
        state.messages.len(),
        1,
        "取消轮同样只留一行终态摘要"
    );
    assert!(state.messages[0].content.starts_with("本轮已停止"));
}
