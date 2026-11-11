//! 双击 Esc「撤回」覆盖层（ADR 0113）。
//!
//! ## 交互（对齐 Claude Code 的心智模型）
//!
//! 非流式下**连按两次 Esc** 打开撤回菜单：
//! 1. **选轮次**（↑↓ 选择 · ↵ 下一步 · esc 取消）——列表是当前会话的用户轮次，新→旧；
//! 2. **选范围**（↑↓ 选择 · ↵ 确认 · esc 返回）——
//!    `恢复代码与会话`：撤销该轮及之后的工作区文件改动（写工具 pre-image 还原）；
//!    `仅恢复会话`：只截断会话，已落盘文件保持原样；
//! 3. 执行成功后：会话回滚到该轮**之前**，输入框回填该轮用户消息（可直接改完重发）。
//!
//! ## 分层
//!
//! - 纯判定（双击上膛 [`RewindEscArm`]、选择推进）与终端无关，可单测；
//! - 执行走控制面 `rewind_session`（会话截断复用既有 checkout 真值路径）；
//! - 渲染只用 `ui()` 的面板原语，不碰消息区布局。

use super::*;
use pony_agent_core::agent::control_plane::rewind_commands::{
    RewindMode, RewindPointView, RewindPointsQuery, RewindSessionCommand,
};

/// 双击 Esc 判定窗口（毫秒）。
///
/// 与流式「双击 Esc 停止」的 500ms 窗口相互独立：撤回只在**非流式**生效，
/// 两个判定不会在同一状态下同时命中。
pub const REWIND_ESC_WINDOW_MS: u64 = 700;

/// 双击 Esc 上膛状态（会话作用域 + 时间窗）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RewindEscArm {
    session_id: Option<String>,
    armed_at_ms: Option<u64>,
}

impl RewindEscArm {
    /// 记一次 Esc：同会话且窗口内的第二次 ⇒ 返回 `true`（开撤回面板），并清膛。
    pub fn press(&mut self, session_id: &str, now_ms: u64, window_ms: u64) -> bool {
        let hit = self
            .armed_at_ms
            .map(|armed| {
                self.session_id.as_deref() == Some(session_id)
                    && now_ms.saturating_sub(armed) <= window_ms
            })
            .unwrap_or(false);
        self.session_id = Some(session_id.to_string());
        self.armed_at_ms = if hit { None } else { Some(now_ms) };
        hit
    }

    /// 丢弃上膛（切会话 / 打开面板后调用）。
    pub fn reset(&mut self) {
        self.session_id = None;
        self.armed_at_ms = None;
    }
}

/// 撤回面板的两段式阶段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RewindStage {
    /// 第一段：选要撤掉的用户轮次。
    Turn,
    /// 第二段：选恢复范围。
    Scope,
}

/// 范围选项（顺序即面板行序，`selected_scope` 是行下标）。
/// 安全第一：首项为「仅恢复会话」，避免未注意范围项直接回车误回退工作区文件（ISSUE-20261111-005）。
pub const REWIND_SCOPE_ROWS: [(&str, RewindMode); 2] = [
    ("仅恢复会话", RewindMode::ConversationOnly),
    ("恢复代码与会话", RewindMode::ConversationAndFiles),
];

/// 撤回覆盖层状态。
#[derive(Debug, Clone)]
pub struct RewindState {
    /// 候选轮次（**新→旧**，与面板行序一致）。
    pub points: Vec<RewindPointView>,
    /// 轮次选中下标。
    pub selected: usize,
    pub stage: RewindStage,
    /// 范围选中下标（`REWIND_SCOPE_ROWS` 的下标）。
    pub selected_scope: usize,
}

impl RewindState {
    pub fn new(points: Vec<RewindPointView>) -> Self {
        Self {
            points,
            selected: 0,
            stage: RewindStage::Turn,
            selected_scope: 0,
        }
    }

    pub fn current_point(&self) -> Option<&RewindPointView> {
        self.points.get(self.selected)
    }

    pub fn scope(&self) -> RewindMode {
        REWIND_SCOPE_ROWS
            .get(self.selected_scope)
            .map(|(_, mode)| *mode)
            .unwrap_or(RewindMode::ConversationOnly)
    }

    /// ↑：新→旧列表里向上 = 更近的轮次（下标 -1）。
    pub fn select_prev(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn select_next(&mut self) {
        if self.selected + 1 < self.points.len() {
            self.selected += 1;
        }
    }

    pub fn select_scope_prev(&mut self) {
        self.selected_scope = self.selected_scope.saturating_sub(1);
    }

    pub fn select_scope_next(&mut self) {
        if self.selected_scope + 1 < REWIND_SCOPE_ROWS.len() {
            self.selected_scope += 1;
        }
    }
}

/// 按键裁决结果（事件循环据此决定是否执行撤回）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RewindAction {
    /// 面板内已消费该键。
    Consumed,
    /// 关闭面板（esc）。
    Cancel,
    /// 第二段确认：执行撤回。
    Confirm,
}

/// 打开撤回面板：拉取当前会话可撤回轮次；没有轮次时只给提示、不开面板。
pub fn open(state: &mut TuiState, control_plane: &HostControlPlane) {
    let points = control_plane.list_rewind_points(RewindPointsQuery {
        session_id: Some(state.session_id.clone()),
    });
    if points.is_empty() {
        state.set_status_notice("没有可撤回的轮次（当前会话还没有已提交的用户消息）".to_string());
        return;
    }
    state.rewind_state = Some(RewindState::new(points));
    state.mode = InteractionMode::Rewind;
}

/// 面板内按键裁决（不改会话、不落盘；执行由 [`execute`] 负责）。
pub fn handle_key(state: &mut TuiState, key: &crossterm::event::KeyEvent) -> RewindAction {
    use crossterm::event::KeyCode;
    let Some(overlay) = state.rewind_state.as_mut() else {
        return RewindAction::Cancel;
    };
    match (overlay.stage, key.code) {
        // 轮次段 esc = 关闭；范围段 esc = 退回选轮次（与页脚「esc 返回」一致）。
        (RewindStage::Turn, KeyCode::Esc) => RewindAction::Cancel,
        (RewindStage::Scope, KeyCode::Esc) => {
            overlay.stage = RewindStage::Turn;
            RewindAction::Consumed
        }
        (RewindStage::Turn, KeyCode::Up) | (RewindStage::Turn, KeyCode::Char('k')) => {
            overlay.select_prev();
            RewindAction::Consumed
        }
        (RewindStage::Turn, KeyCode::Down) | (RewindStage::Turn, KeyCode::Char('j')) => {
            overlay.select_next();
            RewindAction::Consumed
        }
        (RewindStage::Turn, KeyCode::Enter) => {
            if overlay.current_point().is_some() {
                overlay.stage = RewindStage::Scope;
            }
            RewindAction::Consumed
        }
        (RewindStage::Scope, KeyCode::Up) | (RewindStage::Scope, KeyCode::Char('k')) => {
            overlay.select_scope_prev();
            RewindAction::Consumed
        }
        (RewindStage::Scope, KeyCode::Down) | (RewindStage::Scope, KeyCode::Char('j')) => {
            overlay.select_scope_next();
            RewindAction::Consumed
        }
        (RewindStage::Scope, KeyCode::Enter) => RewindAction::Confirm,
        (RewindStage::Scope, KeyCode::Char('1')) => {
            overlay.selected_scope = 0;
            RewindAction::Confirm
        }
        (RewindStage::Scope, KeyCode::Char('2')) => {
            overlay.selected_scope = 1;
            RewindAction::Confirm
        }
        _ => RewindAction::Consumed,
    }
}

/// 执行撤回：会话截断 + 可选文件还原，成功后回填该轮用户消息。
pub fn execute(state: &mut TuiState, control_plane: &HostControlPlane) {
    let Some(overlay) = state.rewind_state.as_ref() else {
        state.mode = InteractionMode::Normal;
        return;
    };
    let Some(point) = overlay.current_point().cloned() else {
        state.rewind_state = None;
        state.mode = InteractionMode::Normal;
        return;
    };
    let mode = overlay.scope();
    state.rewind_state = None;
    state.mode = InteractionMode::Normal;

    let command = RewindSessionCommand {
        session_id: Some(state.session_id.clone()),
        node_id: point.node_id.clone(),
        mode,
    };
    let response = match control_plane.rewind_session(command) {
        Ok(response) => response,
        Err(error) => {
            state.set_status_notice(format!("撤回失败：{error}"));
            return;
        }
    };

    // 与 `/resume` 同口径：复位会话作用域后按权威快照重建消息区。
    state.reset_session_scope();
    hydrate_state_from_session(state, control_plane, &state.session_id.clone());
    let _ = sync_pending_control_request(state, control_plane);

    let restored = response.files_restored.len();
    let deleted = response.files_deleted.len();
    let mut notice = format!(
        "已撤回到第 {} 轮之前 · {} · 输入框已回填该轮消息",
        response.turn_index,
        match mode {
            RewindMode::ConversationAndFiles =>
                format!("代码还原 {} 改 {} 删", restored, deleted),
            RewindMode::ConversationOnly => "仅会话（文件保持原样）".to_string(),
        }
    );
    if response.file_restore_degraded {
        notice.push_str(&format!(
            " · 部分文件未还原（失败 {} / 跳过 {}）",
            response.file_failures.len(),
            response.files_skipped.len()
        ));
    }
    state.set_status_notice(notice);
    if let Some(message) = response.user_message.clone() {
        if !message.trim().is_empty() {
            state.set_input(&message);
        }
    }
}

/// 渲染撤回面板（居中覆盖层，与其它选择器同一视觉语言）。
pub fn render(f: &mut Frame, area: Rect, state: &TuiState, max_rows: usize) {
    let Some(overlay) = state.rewind_state.as_ref() else {
        return;
    };
    let panel_rect = |w: u16, h: u16| -> Rect {
        let pw = w.min(area.width.saturating_sub(4)).max(20);
        let ph = h.min(area.height.saturating_sub(2)).max(6);
        Rect {
            x: area.x + (area.width.saturating_sub(pw)) / 2,
            y: area.y + (area.height.saturating_sub(ph)) / 2,
            width: pw,
            height: ph,
        }
    };
    match overlay.stage {
        RewindStage::Turn => {
            let total = overlay.points.len();
            let shown = total.min(10.min(max_rows)).max(1);
            let selected = overlay.selected.min(total.saturating_sub(1));
            let (start, end) = panel_window(total, selected, shown);
            let title = format!("撤回 · 选择轮次（第 {} 轮，共 {} 轮）", selected + 1, total);
            let rect = panel_rect(88, shown as u16 + 4);
            let body = draw_panel(f, rect, &title, "↑↓ 选择 · ↵ 下一步 · esc 取消");
            let inner_w = body.width as usize;
            let mut lines_out: Vec<Line<'static>> = Vec::new();
            for (idx, point) in overlay.points.iter().enumerate().take(end).skip(start) {
                let selected_row = idx == selected;
                let summary = point_message_summary(point);
                let right = if point.files_tracked > 0 {
                    format!("第 {} 轮 · {} 文件", point.turn_index, point.files_tracked)
                } else {
                    format!("第 {} 轮 · 仅会话", point.turn_index)
                };
                let right_w = str_width(&right) + 1;
                let left = truncate_width(&summary, inner_w.saturating_sub(right_w));
                lines_out.push(list_row(selected_row, &left, &right, inner_w));
            }
            f.render_widget(
                Paragraph::new(lines_out).style(Style::default().bg(theme::SURFACE)),
                body,
            );
        }
        RewindStage::Scope => {
            let point = overlay.current_point();
            let turn_index = point.map(|point| point.turn_index).unwrap_or(0);
            let title = format!("撤回 · 第 {} 轮之前", turn_index);
            let rect = panel_rect(88, REWIND_SCOPE_ROWS.len() as u16 + 5);
            let body = draw_panel(f, rect, &title, "↑↓ 选择 · ↵ 确认 · esc 返回");
            let inner_w = body.width as usize;
            let files = point.map(|point| point.files_tracked).unwrap_or(0);
            let mut lines_out: Vec<Line<'static>> = Vec::new();
            if let Some(message) = point.map(|point| point.user_message.clone()) {
                let preview = truncate_width(
                    &first_line(&message),
                    inner_w.saturating_sub(4).max(8),
                );
                lines_out.push(Line::from(Span::styled(
                    format!("  {preview}"),
                    faint(),
                )));
            }
            for (idx, (label, mode)) in REWIND_SCOPE_ROWS.iter().enumerate() {
                let right = match mode {
                    RewindMode::ConversationAndFiles => {
                        if files > 0 {
                            format!("{files} 文件")
                        } else {
                            "无文件改动".to_string()
                        }
                    }
                    RewindMode::ConversationOnly => "文件保持原样".to_string(),
                };
                let right_w = str_width(&right) + 1;
                let left = truncate_width(label, inner_w.saturating_sub(right_w));
                lines_out.push(list_row(idx == overlay.selected_scope, &left, &right, inner_w));
            }
            lines_out.push(blank());
            lines_out.push(Line::from(Span::styled(
                "  撤回后，该轮用户消息会回填到输入框，可直接修改后重发",
                faint(),
            )));
            f.render_widget(
                Paragraph::new(lines_out).style(Style::default().bg(theme::SURFACE)),
                body,
            );
        }
    }
}

/// 菜单行正文：优先用用户消息首行（比节点标题更接近用户记忆），缺失回退标题。
pub(super) fn point_message_summary(point: &RewindPointView) -> String {
    let message = first_line(&point.user_message);
    if message.trim().is_empty() {
        let title = point.title.trim();
        if title.is_empty() {
            format!("（第 {} 轮）", point.turn_index)
        } else {
            title.to_string()
        }
    } else {
        message
    }
}

/// 取首个非空行并裁剪首尾空白（多行消息在菜单里只占一行）。
fn first_line(text: &str) -> String {
    text.lines()
        .find(|line| !line.trim().is_empty())
        .map(|line| line.trim().to_string())
        .unwrap_or_default()
}
