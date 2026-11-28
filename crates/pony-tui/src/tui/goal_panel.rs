//! Ctrl+G 目标详情覆盖层：`/goal` 的**只读**可视化（语义真源仍在上游 core）。
//!
//! 分层：本模块只做「`TuiState` → 行」的投影（[`render_lines`] 纯数据，可无 TTY 单测），
//! 渲染层 [`render`] 只负责折行/配色/覆盖层尺寸——口径与 `render_settings_panel`、
//! `render_pending_empty_state` 对齐（宽 72 + `draw_panel` + `theme::SURFACE` 底）。
//! 覆盖层**不产生任何状态写入**：暂停/恢复/编辑仍走 `/goal` 命令（单一路径，不另造旁路）。

use super::*;

/// 底部操作提示：面板是只读的，所有变更都回到 `/goal` 命令（逐字冻结，勿改字面量）。
pub const GOAL_PANEL_HINT: &str = "/goal pause · resume · edit <目标> · complete · clear";

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
    // PA-172 §8 B2：自动续轮已停时必须与轮数共存可见。
    //
    // 措辞由 `visible_line()` 唯一提供（它已含 headline 的状态措辞），**不得**再套
    // 「已停止：」之类硬编码前缀：B193 的 `FirstRoundNotSucceeded` 语义是「从未启动」，
    // 旧前缀会把面板渲染成「已停止：…未启动」——一句自我否认，且前缀着色还会把
    // 「未启动」一并涂成 DANGER。前缀与配色一律改由枚举驱动。
    if let Some(reason) = state.goal_rounds_stopped.as_ref() {
        lines.push(reason.visible_line());
    }
    lines.push(String::new());
    lines.push(GOAL_PANEL_HINT.to_string());
    lines
}

/// 行 → 样式：只分类**本模块自己生成**的行（文案即契约），不做模糊猜测。
///
/// 停止行不再靠「已停止：」前缀识别（该前缀已删除），改为按 `state.goal_rounds_stopped`
/// 的枚举严重级着色——与消息区/状态栏共用 [`severity_for_goal_stop`]，三处配色同源。
fn row_style(text: &str, stopped: Option<SystemMessageSeverity>) -> Style {
    if text == GOAL_PANEL_HINT {
        faint()
    } else if let Some(severity) = stopped {
        severity_style(severity, muted())
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
    // 停止行是本模块唯一「带严重级」的行：先算出它，`row_style` 按同一严重级着色。
    let stopped = state
        .goal_rounds_stopped
        .as_ref()
        .map(pony_agent_core::agent::goal::GoalRoundStopReason::severity)
        .map(severity_for_goal_stop);
    let stop_text = state
        .goal_rounds_stopped
        .as_ref()
        .map(pony_agent_core::agent::goal::GoalRoundStopReason::visible_line);
    for text in render_lines(state) {
        if text.is_empty() {
            lines.push(blank());
        } else {
            let style = if stop_text.as_deref() == Some(text.as_str()) {
                row_style(&text, stopped)
            } else {
                row_style(&text, None)
            };
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
        // 措辞完全来自 core 的 `visible_line()`：面板**不得**再套「已停止：」之类前缀
        // —— B193 的「未启动」一旦被套上前缀就成了「已停止：…未启动」的自我否认。
        assert!(
            !joined.contains("已停止："),
            "面板不得硬编码「已停止：」前缀：{joined}"
        );
        assert!(
            joined.contains(&GoalRoundStopReason::Cancelled.visible_line()),
            "文案必须逐字来自 core 的 visible_line：{joined}"
        );
    }

    /// 面板行的配色必须与消息区同源（枚举严重级），不得靠前缀字面量识别停止行。
    #[test]
    fn stopped_row_style_follows_enum_severity() {
        let reason = GoalRoundStopReason::FirstRoundNotSucceeded;
        let severity = severity_for_goal_stop(reason.severity());
        assert_eq!(
            severity,
            SystemMessageSeverity::Error,
            "首轮失败在面板里也必须是 Error 级（旧实现套前缀涂 DANGER，把「未启动」也涂成危险色）"
        );
        let style = row_style(&reason.visible_line(), Some(severity));
        assert_eq!(
            style.fg,
            Some(theme::DANGER),
            "Error 级停止行必须用 DANGER 前景色"
        );
    }

    #[test]
    fn row_style_classifies_own_rows() {
        // 提示行用 faint、停止行按枚举严重级、元信息用 INK_SOFT、objective 用 INK。
        assert_eq!(row_style(GOAL_PANEL_HINT, None), faint());
        assert_eq!(
            row_style(
                &GoalRoundStopReason::Cancelled.visible_line(),
                Some(SystemMessageSeverity::Error)
            )
            .fg,
            Some(theme::DANGER)
        );
        assert_eq!(row_style("阶段 进行中", None).fg, Some(theme::INK_SOFT));
        assert_eq!(row_style("轮次 1 / 2", None).fg, Some(theme::INK_SOFT));
        assert_eq!(row_style("目标正文", None).fg, Some(theme::INK));
    }

    /// 阶段行与停止原因行不得互相打脸：阶段是「进行中」时，停止原因**不许**再说「已暂停」。
    ///
    /// 回归 `目标已暂停：续轮已停止` 撞 `阶段 进行中`（Ctrl+T Goal 页签同屏两个结论）。
    /// 阶段措辞的唯一家是 `phase_label`，续轮停止原因的家是 `headline()`。
    #[test]
    fn stopped_reason_never_contradicts_active_phase() {
        for phase in [GoalPhase::Active, GoalPhase::Paused, GoalPhase::Blocked] {
            let mut state = TuiState::default();
            state.goal_state = Some(TuiGoal::new(
                "goal-x".to_string(),
                "不撞词".to_string(),
                Some(3),
            ));
            if let Some(goal) = state.goal_state.as_mut() {
                goal.phase = phase;
            }
            state.goal_rounds_stopped = Some(GoalRoundStopReason::Cancelled);
            let joined = render_lines(&state).join("\n");
            if phase == GoalPhase::Active {
                assert!(joined.contains("阶段 进行中"), "阶段行必须仍在：{joined}");
                assert!(
                    !joined.contains("已暂停"),
                    "阶段「进行中」时停止原因不得自称「已暂停」：{joined}"
                );
            }
        }
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
