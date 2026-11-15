//! `/models` 面板的上下文窗口编辑器（面板内 `w`）。
//!
//! ## 为什么需要它
//!
//! 窗口是**逐模型声明**（`providers[].models[].capabilities.contextWindowTokens`），
//! 但桌面端之外曾没有任何入口：TUI 的 `/provider` 只有名称 / Base URL / API Key，
//! `/settings` 只有三项显示偏好，`/models` 只负责切换。于是「200K 模型被按 256K 算」
//! 这类问题只能靠手改 `providers.json` 才能修（ADR 2026-11-14-model-declared-window-tui-entry）。
//!
//! ## 纪律
//!
//! - **写入走 `agent::model_window`**：定点改一个键，未声明的模型保持未声明（不会被
//!   「保存一次就全部固化成 262144」）；本模块不碰 `save_view`。
//! - **留空 / `-` = 恢复跟随预设**（删键），不是写 0、也不是写预设默认值。
//! - **失败不静默**：解析错误 / 未命中模型 / 落盘失败一律留在面板里显示，不关面板、不谎报成功。
//! - 面板渲染只读 `state`（窗口徽标在列表构建时一次读盘），不在每帧访问文件系统。

use super::*;
use pony_agent_core::agent::config::DEFAULT_CONTEXT_WINDOW_TOKENS;
use pony_agent_core::agent::model_window::{
    find_context_window, format_context_window, list_context_windows, parse_context_window_input,
    set_context_window,
};

/// 正在编辑的窗口。`None`（`TuiState::model_window_edit`）= 编辑器未打开。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelWindowEditState {
    pub provider_id: String,
    pub provider_name: String,
    pub model_id: String,
    /// 线上模型名（确认「改的是哪个模型」用，和 id 可能不同）。
    pub model: String,
    pub model_name: String,
    /// 磁盘上的显式声明（`None` = 未声明）。
    pub declared: Option<u32>,
    /// 生效窗口（声明 > 预设补缺）。
    pub effective: Option<u32>,
    /// 输入缓冲（预填声明值；未声明时留空 = 保持跟随预设）。
    pub buffer: String,
    /// 上次提交的错误（解析失败 / 未命中 / 落盘失败），成功提交时清空。
    pub error: Option<String>,
}

impl ModelWindowEditState {
    /// 「当前生效窗口」的人读文案（含来源）：`200K（声明）` / `262.1K（预设）`。
    pub fn effective_label(&self) -> String {
        let value = self
            .effective
            .map(format_context_window)
            .unwrap_or_else(|| "—".to_string());
        if self.declared.is_some() {
            format!("{value}（声明）")
        } else {
            format!("{value}（预设）")
        }
    }

    /// 提交后要展示的新值提示（用于状态通知，不参与判定）。
    pub fn is_following_preset(&self) -> bool {
        self.declared.is_none()
    }
}

/// 某个 provider 的模型行：窗口声明与生效值随行携带，渲染期不读盘。
pub fn model_items_for_provider(
    store: &ProviderRegistryStore,
    provider_id: &str,
) -> Vec<ModelItem> {
    list_context_windows(store)
        .into_iter()
        .filter(|entry| entry.provider_id == provider_id)
        .map(|entry| ModelItem {
            id: entry.model_id,
            name: entry.model_name,
            declared_context_window: entry.declared,
            context_window_tokens: entry.effective,
        })
        .collect()
}

impl TuiState {
    /// 打开「上下文窗口」编辑器：目标 = 模型面板当前高亮项 + 当前活动 provider。
    ///
    /// 返回 `Err` 时面板保持原样（不进入编辑态），由调用方把原因写进状态提示。
    pub fn open_model_window_editor(&mut self) -> Result<(), String> {
        let Some(item) = model_panel_item_at(self, self.selected_model_index) else {
            return Err("模型面板为空，没有可编辑的模型".to_string());
        };
        let provider_id = resolve_active_provider_id(self)
            .unwrap_or_else(|| self.current_provider_id.clone());
        let store = self.registry_store();
        let entry = find_context_window(&store, &provider_id, &item.id)
            .ok_or_else(|| format!("注册表里没有模型 `{}`（{provider_id}）", item.name))?;

        self.model_window_edit = Some(ModelWindowEditState {
            provider_id: entry.provider_id,
            provider_name: entry.provider_name,
            model_id: entry.model_id,
            model: entry.model,
            model_name: entry.model_name,
            // 预填「已声明的值」：未声明时留空，回车即保持跟随预设，不会误写一个默认值。
            buffer: entry
                .declared
                .map(|tokens| tokens.to_string())
                .unwrap_or_default(),
            declared: entry.declared,
            effective: entry.effective,
            error: None,
        });
        // 覆盖层入栈：编辑器退出后回到模型面板，而不是裸 Normal（面板仍开着）。
        self.begin_overlay(InteractionMode::EditingModelWindow);
        Ok(())
    }

    /// 编辑器按键裁决。返回 `true` 表示界面需要重绘。
    ///
    /// 模态吸收：除本节列出的键外一律吞掉，绝不漏进 Composer（否则窗口数字会被当成
    /// 用户消息发出去）。
    pub fn handle_model_window_editor_key(&mut self, key: &crossterm::event::KeyEvent) -> bool {
        if self.model_window_edit.is_none() {
            return false;
        }
        match key.code {
            KeyCode::Esc => {
                self.cancel_model_window_editor();
                true
            }
            KeyCode::Enter => self.commit_model_window_editor(),
            KeyCode::Backspace => {
                if let Some(edit) = self.model_window_edit.as_mut() {
                    edit.buffer.pop();
                    edit.error = None;
                }
                true
            }
            KeyCode::Delete => {
                if let Some(edit) = self.model_window_edit.as_mut() {
                    edit.buffer.clear();
                    edit.error = None;
                }
                true
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(edit) = self.model_window_edit.as_mut() {
                    edit.buffer.clear();
                    edit.error = None;
                }
                true
            }
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                // 只收窗口表达式允许的字符：脏字符进 buffer 只会换来一条更难懂的解析错误。
                if accepts_window_char(ch) {
                    if let Some(edit) = self.model_window_edit.as_mut() {
                        edit.buffer.push(ch);
                        edit.error = None;
                    }
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    /// 提交编辑器输入。返回 `true` 表示界面需要重绘（成功或失败都重绘：失败要显示原因）。
    pub fn commit_model_window_editor(&mut self) -> bool {
        let Some(edit) = self.model_window_edit.as_ref() else {
            return false;
        };
        let provider_id = edit.provider_id.clone();
        let model_id = edit.model_id.clone();
        let model = edit.model.clone();
        let model_name = edit.model_name.clone();

        let parsed = match parse_context_window_input(&edit.buffer) {
            Ok(value) => value,
            Err(error) => {
                if let Some(edit) = self.model_window_edit.as_mut() {
                    edit.error = Some(error);
                }
                return true;
            }
        };

        // 没有任何变化就不写盘（避免「打开即保存」把文件 mtime 与内容无谓地刷一遍）。
        if parsed == edit.declared {
            let window = edit
                .effective
                .map(format_context_window)
                .unwrap_or_else(|| "—".to_string());
            self.model_window_edit = None;
            self.end_overlay();
            self.set_status_notice(format!("上下文窗口未变化 · {model_name} · {window}"));
            return true;
        }

        let store = self.registry_store();
        let entry = match set_context_window(&store, &provider_id, &model_id, parsed) {
            Ok(entry) => entry,
            Err(error) => {
                if let Some(edit) = self.model_window_edit.as_mut() {
                    edit.error = Some(error);
                }
                return true;
            }
        };

        // 面板列表整份重读（同一 provider 下其它条目也可能被外部改过），
        // 再按模型 id 重锚选中项，避免刷新后高亮漂移到第一行。
        self.model_list = model_items_for_provider(&store, &provider_id);
        self.selected_model_index = model_panel_index_of(self, &model_id).unwrap_or(0);

        // 改的就是当前在用的模型 ⇒ 立即更新右上角上下文分母（不必等下一轮重读注册表）。
        if self.model == model_id || self.model == model {
            self.context_window_tokens = entry
                .effective
                .or(entry.declared)
                .unwrap_or(DEFAULT_CONTEXT_WINDOW_TOKENS);
        }

        let notice = match entry.declared {
            Some(tokens) => format!(
                "上下文窗口已设为 {} · {model_name}",
                format_context_window(tokens)
            ),
            None => format!(
                "已恢复跟随预设 · {model_name} · 生效 {}",
                entry
                    .effective
                    .map(format_context_window)
                    .unwrap_or_else(|| "—".to_string())
            ),
        };
        self.model_window_edit = None;
        self.end_overlay();
        self.set_status_notice(notice);
        true
    }

    /// 取消编辑：丢弃缓冲，回到模型面板（不写盘）。
    pub fn cancel_model_window_editor(&mut self) {
        self.model_window_edit = None;
        if self.mode == InteractionMode::EditingModelWindow {
            self.end_overlay();
        }
    }
}

fn accepts_window_char(ch: char) -> bool {
    ch.is_ascii_digit() || matches!(ch, 'k' | 'K' | 'm' | 'M' | '_' | '.')
}

/// 居中面板矩形（`panel_rect` 是 `ui()` 里的局部闭包，模块内不可见；
/// 这里只做「限宽限高 + 居中」，与 `render_pending_empty_state` 同口径）。
fn editor_rect(area: Rect, want_w: u16, want_h: u16) -> Rect {
    let width = want_w.min(area.width.saturating_sub(4)).max(20);
    let height = want_h.min(area.height.saturating_sub(2)).max(6);
    Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    }
}

/// 编辑器面板渲染（独立于 `ui()`，只在 `EditingModelWindow` 时被调用）。
pub fn render(f: &mut Frame, state: &TuiState) {
    let Some(edit) = state.model_window_edit.as_ref() else {
        return;
    };
    let rect = editor_rect(f.area(), 72, 9);
    let body = draw_panel(
        f,
        rect,
        "上下文窗口",
        "↵ 保存 · esc 取消 · 留空 / - 恢复跟随预设",
    );
    let inner_w = body.width as usize;

    let mut rows: Vec<Line<'static>> = Vec::new();
    rows.push(Line::from(vec![
        Span::styled(format!("  {:<8}", "提供商"), faint()),
        Span::styled(edit.provider_name.clone(), muted()),
    ]));
    rows.push(Line::from(vec![
        Span::styled(format!("  {:<8}", "模型"), faint()),
        Span::styled(edit.model_name.clone(), muted()),
    ]));
    rows.push(Line::from(vec![
        Span::styled(format!("  {:<8}", "生效窗口"), faint()),
        Span::styled(edit.effective_label(), ink()),
    ]));
    rows.push(blank());
    let caret = if state.spinner_tick % 2 == 0 { CARET } else { "" };
    let value = if edit.buffer.is_empty() {
        // 空缓冲 = 恢复跟随预设：直接把这个选择的后果显示出来，用户不用猜。
        Span::styled(
            format!(
                "跟随预设（{}）",
                edit.effective
                    .map(format_context_window)
                    .unwrap_or_else(|| "—".to_string())
            ),
            faint(),
        )
    } else {
        Span::styled(edit.buffer.clone(), ink())
    };
    rows.push(Line::from(vec![
        Span::styled("  › ", Style::default().fg(theme::ACCENT)),
        value,
        Span::styled(caret, Style::default().fg(theme::ACCENT)),
    ]));
    rows.push(Line::from(Span::styled(
        "  支持 200000 / 200K / 1M；清空后按预设补缺",
        faint(),
    )));
    if let Some(error) = edit.error.as_deref() {
        rows.push(Line::from(Span::styled(
            truncate_width(&format!("  {error}"), inner_w),
            Style::default().fg(theme::DANGER),
        )));
    }

    f.render_widget(
        Paragraph::new(rows).style(Style::default().bg(theme::SURFACE)),
        body,
    );
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn accepts_only_window_expression_characters() {
        for ch in ['0', '9', 'k', 'K', 'm', 'M', '_', '.'] {
            assert!(accepts_window_char(ch), "{ch} 应被接受");
        }
        for ch in ['a', '中', '/', ' ', '-'] {
            assert!(!accepts_window_char(ch), "{ch} 不应进入窗口输入");
        }
    }

    #[test]
    fn effective_label_reports_source() {
        let declared = ModelWindowEditState {
            provider_id: "p".into(),
            provider_name: "P".into(),
            model_id: "m".into(),
            model: "m".into(),
            model_name: "M".into(),
            declared: Some(200_000),
            effective: Some(200_000),
            buffer: String::new(),
            error: None,
        };
        assert_eq!(declared.effective_label(), "200K（声明）");
        assert!(!declared.is_following_preset());

        let inherited = ModelWindowEditState {
            declared: None,
            effective: Some(DEFAULT_CONTEXT_WINDOW_TOKENS),
            ..declared
        };
        assert_eq!(inherited.effective_label(), "262.1K（预设）");
        assert!(inherited.is_following_preset());
    }
}
