//! B194（2026-11-15）：TUI 排队消息的**正文可见**与**立即发送**（对齐桌面端
//! `QueuedMessagesBubble.vue` + `promoteToSteer` + Alt+S）。
//!
//! 旧实现只有指标行的一句计数（`queue_indicator_line` → 「N 条排队 · 本轮结束后自动发送」），
//! 用户既看不到排队的是哪几条、也不能删/插队：排错一句话只能等本轮结束再收拾。
//!
//! 本模块只承载**纯函数**（行生成 / 决策）与一个渲染函数；派发仍走 `queue.rs` 的
//! 既有单飞闸门（C15/C21/C24/C26），不复制任何派发真值。

use super::*;

/// 内联排队面板最多展示的条目数（超出折叠成「还有 N 条」；计数仍在指标行）。
pub const QUEUE_PANEL_MAX_ROWS: usize = 3;

/// Alt+S「立即发送队首」的动作裁决（纯函数，唯一判定点）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueImmediateAction {
    /// 队列为空：只给一句提示，不动输入框。
    Empty,
    /// 有轮次在飞（流式 / goal 续轮）：先请求停止，收口后由既有闸门自动派发队首。
    StopThenDispatch,
    /// 界面空闲：立刻把队首写回输入框并自动提交。
    DispatchNow,
}

/// 裁决「立即发送」该做什么。
///
/// `turn_in_flight` 与会话在飞注册表同源（`TurnLoopInFlightGuard`）：goal 自动续轮期间
/// `is_streaming` 可能为假，但轮次线程仍活着 —— 此时**不得**直接派发（会与在飞轮
/// 交叠写同一会话历史），必须先停止。
pub fn queue_immediate_action(
    queued: usize,
    is_streaming: bool,
    turn_in_flight: bool,
) -> QueueImmediateAction {
    if queued == 0 {
        QueueImmediateAction::Empty
    } else if is_streaming || turn_in_flight {
        QueueImmediateAction::StopThenDispatch
    } else {
        QueueImmediateAction::DispatchNow
    }
}

/// 面板的一行（样式语义由渲染层解释，纯数据便于机械断言）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuePanelRow {
    pub text: String,
    /// 标题行（计数 + 操作提示）/ 折叠行（「还有 N 条」）走弱化样式，正文行走常规样式。
    pub faint: bool,
}

/// 单行化：排队正文是用户输入原样快照，可能含换行/制表；面板只占一行。
fn single_line(text: &str) -> String {
    text.split(['\n', '\r']).collect::<Vec<_>>().join(" ")
}

/// 生成内联排队面板的行（`width` 为可用列数，用于截断）。
///
/// 空队列 ⇒ 空 `Vec`（高度 0，不占布局）。
pub fn queue_panel_rows(state: &TuiState, width: usize) -> Vec<QueuePanelRow> {
    let queued = &state.queued_user_messages;
    if queued.is_empty() || width == 0 {
        return Vec::new();
    }
    let header = format!(
        "排队 {} 条 · Alt+S 立即发送队首 · Esc 暂停本轮",
        queued.len()
    );
    let mut rows = vec![QueuePanelRow {
        text: truncate_width(&header, width),
        faint: true,
    }];
    for (index, message) in queued.iter().take(QUEUE_PANEL_MAX_ROWS).enumerate() {
        let body = single_line(message);
        let body = body.trim();
        let text = format!("#{} {}", index + 1, body);
        rows.push(QueuePanelRow {
            text: truncate_width(&text, width),
            faint: false,
        });
    }
    if queued.len() > QUEUE_PANEL_MAX_ROWS {
        let rest = queued.len() - QUEUE_PANEL_MAX_ROWS;
        rows.push(QueuePanelRow {
            text: truncate_width(&format!("… 还有 {rest} 条"), width),
            faint: true,
        });
    }
    rows
}

/// 内联排队面板占用的行数（0 = 不渲染）。
pub fn inline_queue_height(state: &TuiState, width: usize) -> u16 {
    queue_panel_rows(state, width).len() as u16
}

/// 渲染内联排队面板（紧贴 Composer 色带之上；不参与消息区几何）。
pub fn render_inline_queue(f: &mut Frame, area: Rect, state: &TuiState) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let rows = queue_panel_rows(state, area.width as usize);
    if rows.is_empty() {
        return;
    }
    let width = area.width as usize;
    let mut lines: Vec<Line<'static>> = Vec::new();
    for row in rows {
        let style = if row.faint {
            Style::default().fg(theme::MUTED)
        } else {
            Style::default().fg(theme::INK_SOFT)
        };
        // 截断已按 area 宽度做过；这里再包一层以吃掉终端宽度与字宽差异的边界。
        lines.extend(wrap_line(&Line::from(Span::styled(row.text, style)), width));
    }
    f.render_widget(Paragraph::new(lines), area);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_queue_has_no_rows_and_zero_height() {
        let mut state = TuiState::default();
        assert!(queue_panel_rows(&state, 80).is_empty());
        assert_eq!(inline_queue_height(&state, 80), 0);
        // 空队列时即便有 Draft 也不渲染面板。
        state.set_input("草稿");
        assert_eq!(inline_queue_height(&state, 80), 0);
    }

    #[test]
    fn header_and_bodies_are_visible_with_index() {
        let mut state = TuiState::default();
        let mut queue = Vec::new();
        assert!(push_queued_message(&mut queue, "先跑单测"));
        assert!(push_queued_message(&mut queue, "再补 ADR"));
        state.queued_user_messages = queue;

        let rows = queue_panel_rows(&state, 80);
        assert_eq!(rows.len(), 3, "标题 + 2 条正文：{rows:?}");
        assert!(rows[0].text.contains("排队 2 条"), "{:?}", rows[0]);
        assert!(rows[0].text.contains("Alt+S"), "操作提示必须可见：{:?}", rows[0]);
        assert_eq!(rows[1].text, "#1 先跑单测");
        assert_eq!(rows[2].text, "#2 再补 ADR");
        assert!(!rows[1].faint);
    }

    #[test]
    fn overflow_is_folded_with_remaining_count() {
        let mut state = TuiState::default();
        state.queued_user_messages = (1..=5).map(|i| format!("消息{i}")).collect();
        let rows = queue_panel_rows(&state, 80);
        assert_eq!(rows.len(), QUEUE_PANEL_MAX_ROWS + 2, "{rows:?}");
        let last = rows.last().unwrap();
        assert!(last.text.contains("还有 2 条"), "{last:?}");
        assert!(last.faint);
    }

    #[test]
    fn multiline_message_stays_on_one_row() {
        let mut state = TuiState::default();
        state.queued_user_messages = vec!["第一行\n第二行".to_string()];
        let rows = queue_panel_rows(&state, 80);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].text, "#1 第一行 第二行");
    }

    #[test]
    fn narrow_width_truncates_instead_of_wrapping() {
        let mut state = TuiState::default();
        state.queued_user_messages = vec!["这是一个很长的排队消息正文，需要被截断".to_string()];
        let rows = queue_panel_rows(&state, 12);
        assert_eq!(rows.len(), 2);
        assert!(str_width(&rows[1].text) <= 12, "{:?}", rows[1]);
    }

    #[test]
    fn immediate_action_truth_table() {
        assert_eq!(queue_immediate_action(0, false, false), QueueImmediateAction::Empty);
        assert_eq!(
            queue_immediate_action(1, true, false),
            QueueImmediateAction::StopThenDispatch
        );
        // goal 续轮期：is_streaming 可能为假，但在飞轮次线程必须拦住直接派发。
        assert_eq!(
            queue_immediate_action(1, false, true),
            QueueImmediateAction::StopThenDispatch
        );
        assert_eq!(
            queue_immediate_action(1, false, false),
            QueueImmediateAction::DispatchNow
        );
    }
}
