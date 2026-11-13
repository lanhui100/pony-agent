//! Ctrl+G 目标详情覆盖层：`/goal` 的**只读**可视化（语义真源仍在上游 core）。
//!
//! 分层：本模块只做「`TuiState` → 行」的投影（[`render_lines`] 纯数据，可无 TTY 单测），
//! 渲染层 [`render`] 只负责折行/配色/覆盖层尺寸——口径与 `render_settings_panel`、
//! `render_pending_empty_state` 对齐（宽 72 + `draw_panel` + `theme::SURFACE` 底）。
//! 覆盖层**不产生任何状态写入**：暂停/恢复/编辑仍走 `/goal` 命令（单一路径，不另造旁路）。

use super::*;

/// 底部操作提示：面板是只读的，所有变更都回到 `/goal` 命令（逐字冻结，勿改字面量）。
pub const GOAL_PANEL_HINT: &str =
    "/goal pause · resume · edit <目标> · complete · blocked <理由> · clear";

/// `GoalPhase` → 中文短语（与 goal 横幅同一套译词，避免同一状态两处说法）。
fn phase_label(phase: &GoalPhase) -> &'static str {
    match phase {
        GoalPhase::Active => "进行中",
        GoalPhase::Completed => "已完成",
        GoalPhase::Blocked => "已阻塞",
        GoalPhase::Paused => "已暂停",
    }
}

/// 面板纯文本行（无 `Frame` 依赖）：唯一的「状态 → 行」投影，渲染与测试共用。
pub fn render_lines(state: &TuiState) -> Vec<String> {
    let Some(goal) = state.goal_state.as_ref() else {
        // 空态必须可引导：不允许整屏空白（与 `render_pending_empty_state` 同一纪律）。
        return vec![
            "当前会话没有目标".to_string(),
            String::new(),
            "用 /goal <目标描述> 创建".to_string(),
        ];
    };

    let mut lines: Vec<String> = Vec::new();
    // objective 可含换行：逐段落行；按宽度折行交给 `render`（纯数据不猜终端宽度）。
    let objective = goal.objective.trim();
    if objective.is_empty() {
        lines.push("（目标描述为空）".to_string());
    } else {
        lines.extend(objective.lines().map(|part| part.trim().to_string()));
    }
    lines.push(String::new());
    lines.push(format!("阶段 {}", phase_label(&goal.phase)));
    lines.push(match goal.max_rounds {
        Some(max) => format!("轮次 {} / {}", goal.round_count, max),
        None => format!("轮次 {}（无上限）", goal.round_count),
    });
    // PA-172 §8 B2：自动续轮已停时必须与轮数共存可见（`visible_line` 是唯一文案真源）。
    if let Some(reason) = state.goal_rounds_stopped.as_ref() {
        lines.push(format!("已停止：{}", reason.visible_line()));
    }
    lines.push(String::new());
    lines.push(GOAL_PANEL_HINT.to_string());
    lines
}

/// 行 → 样式：只分类**本模块自己生成**的行（文案即契约），不做模糊猜测。
fn row_style(text: &str) -> Style {
    if text == GOAL_PANEL_HINT {
        faint()
    } else if text.starts_with("已停止：") {
        Style::default().fg(theme::DANGER)
    } else if text.starts_with("阶段 ") || text.starts_with("轮次 ") {
        Style::default().fg(theme::INK_SOFT)
    } else if text.starts_with("用 /goal") || text.starts_with("（") {
        Style::default().fg(theme::MUTED)
    } else {
        Style::default().fg(theme::INK)
    }
}

/// Ctrl+G 覆盖层渲染：居中覆盖层，宽/高按终端尺寸饱和裁剪。
pub fn render(f: &mut Frame, area: Rect, state: &TuiState) {
    let pw = 72u16.min(area.width.saturating_sub(4)).max(20);
    let inner_w = (pw as usize).saturating_sub(2).max(8);
    let mut lines: Vec<Line<'static>> = Vec::new();
    for text in render_lines(state) {
        if text.is_empty() {
            lines.push(blank());
        } else {
            let style = row_style(&text);
            lines.extend(wrap_line(&Line::from(Span::styled(text, style)), inner_w));
        }
    }
    let ph = (lines.len() as u16 + 4)
        .min(area.height.saturating_sub(2))
        .max(6);
    let rect = Rect {
        x: area.x + (area.width.saturating_sub(pw)) / 2,
        y: area.y + (area.height.saturating_sub(ph)) / 2,
        width: pw,
        height: ph,
    };
    let body = draw_panel(f, rect, "目标", "Esc 关闭 · Ctrl+G 切换");
    let visible = body.height as usize;
    if lines.len() > visible && visible > 0 {
        lines.truncate(visible);
    }
    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme::SURFACE)),
        body,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use pony_agent_core::agent::goal::GoalRoundStopReason;

    #[test]
    fn empty_state_guides_user_to_goal_command() {
        let state = TuiState::default();
        let lines = render_lines(&state);
        assert!(lines.iter().any(|l| l.contains("当前会话没有目标")), "{lines:?}");
        assert!(
            lines.iter().any(|l| l.contains("/goal <目标描述> 创建")),
            "空态必须给出可执行入口：{lines:?}"
        );
    }

    #[test]
    fn goal_state_shows_objective_phase_rounds_and_hint() {
        let mut state = TuiState::default();
        state.goal_state = Some(TuiGoal::new(
            "goal-1".to_string(),
            "把 TUI 的目标面板补上".to_string(),
            Some(8),
        ));
        state.goal_state.as_mut().unwrap().round_count = 2;
        let lines = render_lines(&state);
        let joined = lines.join("\n");
        assert!(joined.contains("把 TUI 的目标面板补上"), "{joined}");
        assert!(joined.contains("阶段 进行中"), "{joined}");
        assert!(joined.contains("轮次 2 / 8"), "{joined}");
        assert!(joined.contains(GOAL_PANEL_HINT), "{joined}");
        assert!(!joined.contains("已停止"), "未停止时不得显示停止行：{joined}");
    }

    #[test]
    fn goal_state_without_max_rounds_says_unbounded() {
        let mut state = TuiState::default();
        state.goal_state = Some(TuiGoal::new("goal-2".to_string(), "无限轮次".to_string(), None));
        let lines = render_lines(&state);
        assert!(
            lines.iter().any(|l| l.contains("轮次 0（无上限）")),
            "{lines:?}"
        );
    }

    #[test]
    fn stopped_rounds_are_visible_with_core_wording() {
        let mut state = TuiState::default();
        state.goal_state = Some(TuiGoal::new("goal-3".to_string(), "被取消的目标".to_string(), Some(4)));
        state.goal_rounds_stopped = Some(GoalRoundStopReason::Cancelled);
        let joined = render_lines(&state).join("\n");
        assert!(joined.contains("已停止："), "{joined}");
        assert!(
            joined.contains("goal 续轮已停止（用户取消）"),
            "文案必须来自 core 的 visible_line：{joined}"
        );
    }

    #[test]
    fn row_style_classifies_own_rows() {
        // 提示行用 faint、停止行用 DANGER、元信息用 INK_SOFT、objective 用 INK。
        assert_eq!(row_style(GOAL_PANEL_HINT), faint());
        assert_eq!(row_style("已停止：x").fg, Some(theme::DANGER));
        assert_eq!(row_style("阶段 进行中").fg, Some(theme::INK_SOFT));
        assert_eq!(row_style("轮次 1 / 2").fg, Some(theme::INK_SOFT));
        assert_eq!(row_style("目标正文").fg, Some(theme::INK));
    }

    #[test]
    fn goal_phase_labels_cover_all_variants() {
        for (phase, expected) in [
            (GoalPhase::Active, "进行中"),
            (GoalPhase::Completed, "已完成"),
            (GoalPhase::Blocked, "已阻塞"),
            (GoalPhase::Paused, "已暂停"),
        ] {
            assert_eq!(phase_label(&phase), expected);
        }
    }
}
