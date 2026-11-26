//! 权限审批的**底部内联**形态（与 Ask / 排队面板同一视觉语法）。
//!
//! 为什么不再居中弹窗：契约「需要人交互的东西都在底部」——Ask 与排队面板早已是
//! 「覆盖层贴 Composer 上缘、不参与消息区几何」的内联形态，权限审批是唯一的例外：
//! 居中弹窗会把刚写出的正文整块遮住，用户在「要不要放行这条命令」时看不到上下文
//! （ADR `2026-11-17-tui-task-visibility-bottom-drawer`）。
//!
//! 本模块只做几何与渲染；键位裁决仍在事件循环（Enter 确认 / Tab 切换 / Esc 拒绝）。

use super::*;

/// 「目标」行是否还要单独渲染：正文已经包含同一路径时不重复（说明优先，路径不写两遍）。
pub(crate) fn show_target_line(perm: &PermissionState) -> bool {
    let Some(target) = perm.request.command_or_path.as_deref() else {
        return false;
    };
    let target = target.trim();
    !target.is_empty() && !perm.request.description.contains(target)
}

/// 内联权限面板高度（渲染与 `ui()` 布局共用的**唯一**口径）。
///
/// 结构自上而下：发丝线 1 + 上呼吸 1 + 标题 1 + 信息行(≤3) + 呼吸 1 + 选项行 + 提示 1。
pub fn inline_permission_height(state: &TuiState, available_height: u16) -> u16 {
    if state.mode != InteractionMode::PermissionApproval {
        return 0;
    }
    let Some(perm) = state.permission_state.as_ref() else {
        return 0;
    };
    let info_rows = 2 + usize::from(show_target_line(perm));
    let choices = perm.choices.len().max(1);
    let needed = 1 + 1 + 1 + info_rows + 1 + choices + 1;
    // 上限 40% 屏：审批是「必须回答」，但不能把上下文全吃掉。
    let cap = ((available_height as usize * 2) / 5).max(8);
    (needed.min(cap) as u16).max(6)
}

/// 渲染内联权限面板：无边框、CANVAS 底色，下缘贴基座（由调用方给几何）。
pub fn render_permission_inline(f: &mut Frame, area: Rect, state: &TuiState) {
    let Some(perm) = state.permission_state.as_ref() else {
        return;
    };
    fill(f, area, theme::CANVAS);
    let width = area.width as usize;
    let budget = area.height as usize;
    let accent = Style::default()
        .fg(theme::TERRACOTTA_BRIGHT)
        .add_modifier(Modifier::BOLD);
    let label = Style::default().fg(theme::MUTED);
    let mut lines: Vec<Line<'static>> = Vec::new();

    if budget >= 2 {
        lines.push(hairline(width));
    }
    lines.push(blank());
    let tool_name = perm.request.tool_name.trim();
    let mut header = vec![
        Span::raw(" ".repeat(RAIL)),
        Span::styled("◆ 审批", accent),
    ];
    if !tool_name.is_empty() {
        header.push(Span::styled(
            format!("  {}", truncate_chars(tool_name, 40)),
            Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD),
        ));
    }
    lines.push(Line::from(header));
    lines.push(Line::from(vec![
        Span::raw(" ".repeat(RAIL)),
        Span::styled("  说明：", label),
        Span::styled(
            truncate_chars(&perm.request.description, width.saturating_sub(RAIL + 12)),
            Style::default().fg(theme::INK),
        ),
    ]));
    if show_target_line(perm) {
        let command = perm
            .request
            .command_or_path
            .as_deref()
            .unwrap_or_default()
            .trim();
        lines.push(Line::from(vec![
            Span::raw(" ".repeat(RAIL)),
            Span::styled("  目标：", label),
            Span::styled(
                truncate_chars(command, width.saturating_sub(RAIL + 12)),
                Style::default().fg(theme::INK_SOFT),
            ),
        ]));
    }
    lines.push(blank());

    let inner_w = width.saturating_sub(RAIL);
    for (index, choice) in perm.choices.iter().enumerate() {
        let selected = index == perm.selected_choice_index;
        let (text, desc) = match choice {
            PermissionChoice::AllowOnce => ("AllowOnce (允许一次)", "仅本次工具执行"),
            PermissionChoice::AllowSession => ("AllowSession (本会话允许)", "本会话内永久放行"),
            PermissionChoice::Deny => ("Deny (拒绝)", "拦截并取消本次调用"),
        };
        lines.push(list_row(selected, text, desc, inner_w));
    }

    lines.push(Line::from(vec![
        Span::raw(" ".repeat(RAIL)),
        Span::styled(
            "↑↓/Tab 选择 · Enter 确认 · Esc 拒绝",
            Style::default().fg(theme::GHOST),
        ),
    ]));

    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme::CANVAS)),
        area,
    );
}
