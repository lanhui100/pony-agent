//! 快捷键清单**唯一真源** + Ctrl+H 帮助覆盖层（原 `tui.rs` 内联逻辑外移）。
//!
//! 设计分界：清单是**静态数据**（[`SHORTCUT_SECTIONS`]），按键判定是**纯函数**
//! （[`is_shortcuts_help_shortcut`] / [`is_goal_panel_shortcut`]）——两者都可在无 TTY
//! 环境单测；只有 [`render`] 触碰 `Frame`。新增/修改键位时只改本文件，别在 `tui.rs`
//! 里另写字面量表（避免「帮助面板与真实键位说谎」）。

use super::*;

/// 一条快捷键：左列键位（人读字面量）+ 右侧一句话说明。
pub struct ShortcutItem {
    pub keys: &'static str,
    pub description: &'static str,
}

/// 一组快捷键（标题 + 条目）；组间渲染时留一空行。
pub struct ShortcutSection {
    pub title: &'static str,
    pub items: &'static [ShortcutItem],
}

/// 键位列宽度（显示列）；超出则原样输出，由折行接管，不做截断。
const KEY_COL: usize = 17;

/// Ctrl+H 是否为「快捷键帮助」快捷键。
///
/// 只认「恰好 CONTROL + `Char('h')`」：`Ctrl+Shift+H` 会变成 `Char('H')`、`Alt+H`
/// 的 modifiers 不等，故都不会命中。raw mode 下 0x08（Ctrl+H）由 crossterm 解析为
/// `Char('h') + CONTROL`，Backspace 走 0x7F ⇒ 与退格**不冲突**（勿改回 `0x08` 判定）。
pub fn is_shortcuts_help_shortcut(code: KeyCode, modifiers: KeyModifiers) -> bool {
    modifiers == KeyModifiers::CONTROL && code == KeyCode::Char('h')
}

/// Ctrl+G 是否为「目标面板」快捷键（口径同 [`is_shortcuts_help_shortcut`]，只用 `==`）。
pub fn is_goal_panel_shortcut(code: KeyCode, modifiers: KeyModifiers) -> bool {
    modifiers == KeyModifiers::CONTROL && code == KeyCode::Char('g')
}

/// 本次按键对应的覆盖层目标：Ctrl+H → 快捷键帮助，Ctrl+G → 目标面板，其余 `None`。
///
/// 事件循环只消费返回值，不自己拼 `InteractionMode`（键位到目标模式的映射唯一真源在此）。
pub fn overlay_target(code: KeyCode, modifiers: KeyModifiers) -> Option<InteractionMode> {
    if is_shortcuts_help_shortcut(code, modifiers) {
        Some(InteractionMode::ShortcutsHelp)
    } else if is_goal_panel_shortcut(code, modifiers) {
        Some(InteractionMode::GoalDetail)
    } else {
        None
    }
}

/// 覆盖层开关的状态转换：`Normal ↔ 目标覆盖层` 是**唯一**合法转换。
///
/// 返回 `None` 表示「当前模态下不响应」（例如设置面板里按 Ctrl+H）——调用方据此吞键，
/// 既不会嵌套覆盖层，也不会把字符漏进 Composer。
pub fn toggled_mode(current: &InteractionMode, target: InteractionMode) -> Option<InteractionMode> {
    if *current == target {
        Some(InteractionMode::Normal)
    } else if *current == InteractionMode::Normal {
        Some(target)
    } else {
        None
    }
}

/// 快捷键清单：四组，条目全部对应 TUI **真实存在**的键位。
///
/// 键位列**不重复**（下一项防漂移测试会断言）：同一物理键在不同上下文语义不同时
/// （`Enter` 提交 vs 流式期排队、`Esc Esc` 流式停止 vs 非流式撤回），仍只保留一条，
/// 由说明把两种语义说全。
pub const SHORTCUT_SECTIONS: &[ShortcutSection] = &[
    ShortcutSection {
        title: "全局",
        items: &[
            ShortcutItem { keys: "F2", description: "切换鼠标捕获（关闭时可用终端原生拖选复制）" },
            ShortcutItem { keys: "F3", description: "轮转思考强度档位" },
            ShortcutItem { keys: "Ctrl+O", description: "显示 / 隐藏 Agent 过程（思考与工具调用）" },
            ShortcutItem { keys: "Ctrl+P", description: "轮转工具执行权限（只读 / 工作区 / 全开）" },
            ShortcutItem { keys: "Ctrl+C", description: "三态：流式中中断本轮 · 有草稿先清草稿 · 否则退出" },
            ShortcutItem { keys: "Ctrl+Z", description: "挂起到后台，fg 恢复（仅 Unix）" },
        ],
    },
    ShortcutSection {
        title: "输入框编辑",
        items: &[
            ShortcutItem { keys: "Enter", description: "提交输入（流式期自动改为排队，见「流式与队列」）" },
            ShortcutItem { keys: "Shift+Enter", description: "换行" },
            ShortcutItem { keys: "Alt+Enter", description: "换行" },
            ShortcutItem { keys: "Ctrl+J", description: "换行" },
            ShortcutItem { keys: "Ctrl+U", description: "清空输入框" },
            ShortcutItem { keys: "Ctrl+W", description: "删除光标前一个词" },
            ShortcutItem { keys: "Ctrl+A", description: "移到行首" },
            ShortcutItem { keys: "Ctrl+E", description: "移到行尾" },
        ],
    },
    ShortcutSection {
        title: "流式与队列",
        items: &[
            ShortcutItem { keys: "Enter（有正文）", description: "排队，本轮结束自动发送（不并发第二个 turn）" },
            ShortcutItem { keys: "Esc Esc（流式中）", description: "双击停止生成，终态由事件驱动" },
            ShortcutItem { keys: "Ctrl+C（流式中）", description: "中断本轮（不退出；有草稿时先清草稿）" },
        ],
    },
    ShortcutSection {
        title: "面板与会话",
        items: &[
            ShortcutItem { keys: "Ctrl+T", description: "任务看板（流式期同样可用）" },
            ShortcutItem { keys: "Ctrl+R", description: "模型面板，拉取并切换模型（流式期同样可用）" },
            ShortcutItem { keys: "Ctrl+M", description: "同 Ctrl+R（仅 kitty 协议终端可上报）" },
            ShortcutItem { keys: "Ctrl+↑", description: "子会话内返回主会话" },
            ShortcutItem { keys: "Ctrl+← / Ctrl+→", description: "切换子智能体会话" },
            ShortcutItem { keys: "Esc", description: "关闭当前面板 / 取消选择" },
            ShortcutItem { keys: "Esc Esc（非流式）", description: "双击打开撤回（选轮次 → 选范围）" },
            ShortcutItem { keys: "Ctrl+G", description: "目标面板（Esc 或再按 Ctrl+G 关闭）" },
            ShortcutItem { keys: "Ctrl+H", description: "本帮助面板（Esc 或再按 Ctrl+H 关闭）" },
        ],
    },
];

/// 按 inner 宽度生成可渲染行（节标题 + 键位/说明两列，说明折行保留缩进语义）。
fn section_lines(width: usize) -> Vec<Line<'static>> {
    let mut out: Vec<Line<'static>> = Vec::new();
    for (idx, section) in SHORTCUT_SECTIONS.iter().enumerate() {
        if idx > 0 {
            out.push(blank());
        }
        out.push(Line::from(Span::styled(
            format!("  {}", section.title),
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        )));
        for item in section.items {
            let keys = pad_display(item.keys, KEY_COL);
            let line = Line::from(vec![
                Span::styled(format!("  {keys} "), Style::default().fg(theme::INK)),
                Span::styled(item.description, faint()),
            ]);
            out.extend(wrap_line(&line, width));
        }
    }
    out
}

/// Ctrl+H 覆盖层：居中面板，风格与 `render_settings_panel` 同口径（宽 72 + `draw_panel`）。
pub fn render(f: &mut Frame, area: Rect) {
    let pw = 72u16.min(area.width.saturating_sub(4)).max(20);
    let inner_w = (pw as usize).saturating_sub(2).max(8);
    let mut rows = section_lines(inner_w);
    let ph = (rows.len() as u16 + 4)
        .min(area.height.saturating_sub(2))
        .max(6);
    let rect = Rect {
        x: area.x + (area.width.saturating_sub(pw)) / 2,
        y: area.y + (area.height.saturating_sub(ph)) / 2,
        width: pw,
        height: ph,
    };
    let body = draw_panel(f, rect, "快捷键", "Esc 关闭 · Ctrl+H 切换");
    // 高度不足时不留「被静默截断的假清单」：末行显式说明还有条目未显示。
    let visible = body.height as usize;
    if rows.len() > visible && visible > 0 {
        rows.truncate(visible.saturating_sub(1));
        rows.push(Line::from(Span::styled(
            "  … 面板高度不足，请放大终端（Esc 关闭）",
            faint(),
        )));
    }
    f.render_widget(
        Paragraph::new(rows).style(Style::default().bg(theme::SURFACE)),
        body,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_keys() -> Vec<&'static str> {
        SHORTCUT_SECTIONS
            .iter()
            .flat_map(|s| s.items.iter().map(|i| i.keys))
            .collect()
    }

    #[test]
    fn ctrl_h_only_matches_exact_control_h() {
        assert!(is_shortcuts_help_shortcut(
            KeyCode::Char('h'),
            KeyModifiers::CONTROL
        ));
        // 只认精确组合：Shift / Alt / 裸键 / Backspace 一律不命中。
        assert!(!is_shortcuts_help_shortcut(
            KeyCode::Char('H'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT
        ));
        assert!(!is_shortcuts_help_shortcut(
            KeyCode::Char('h'),
            KeyModifiers::ALT
        ));
        assert!(!is_shortcuts_help_shortcut(
            KeyCode::Char('h'),
            KeyModifiers::NONE
        ));
        assert!(!is_shortcuts_help_shortcut(
            KeyCode::Backspace,
            KeyModifiers::NONE
        ));
        assert!(!is_shortcuts_help_shortcut(
            KeyCode::Backspace,
            KeyModifiers::CONTROL
        ));
    }

    #[test]
    fn ctrl_g_only_matches_exact_control_g() {
        assert!(is_goal_panel_shortcut(
            KeyCode::Char('g'),
            KeyModifiers::CONTROL
        ));
        assert!(!is_goal_panel_shortcut(
            KeyCode::Char('G'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT
        ));
        assert!(!is_goal_panel_shortcut(
            KeyCode::Char('g'),
            KeyModifiers::ALT
        ));
        assert!(!is_goal_panel_shortcut(
            KeyCode::Char('g'),
            KeyModifiers::NONE
        ));
        // 不得串味：Ctrl+H 不是目标面板，Ctrl+G 不是帮助面板。
        assert!(!is_goal_panel_shortcut(
            KeyCode::Char('h'),
            KeyModifiers::CONTROL
        ));
        assert!(!is_shortcuts_help_shortcut(
            KeyCode::Char('g'),
            KeyModifiers::CONTROL
        ));
    }

    #[test]
    fn overlay_toggle_only_swaps_normal_and_own_overlay() {
        let help = InteractionMode::ShortcutsHelp;
        let goal = InteractionMode::GoalDetail;
        assert_eq!(overlay_target(KeyCode::Char('h'), KeyModifiers::CONTROL), Some(help.clone()));
        assert_eq!(overlay_target(KeyCode::Char('g'), KeyModifiers::CONTROL), Some(goal.clone()));
        assert_eq!(overlay_target(KeyCode::Char('h'), KeyModifiers::ALT), None);

        // Normal → 覆盖层；覆盖层 → Normal（再按同键关闭）。
        assert_eq!(toggled_mode(&InteractionMode::Normal, help.clone()), Some(help.clone()));
        assert_eq!(toggled_mode(&help, help.clone()), Some(InteractionMode::Normal));
        assert_eq!(toggled_mode(&goal, goal.clone()), Some(InteractionMode::Normal));
        // 其它模态：不响应（不嵌套两个覆盖层）。
        assert_eq!(toggled_mode(&help, goal.clone()), None);
        assert_eq!(toggled_mode(&InteractionMode::Settings, help), None);
        assert_eq!(toggled_mode(&InteractionMode::Rewind, goal), None);
    }

    #[test]
    fn sections_are_non_empty_and_cover_new_shortcuts() {
        assert_eq!(SHORTCUT_SECTIONS.len(), 4);
        assert!(SHORTCUT_SECTIONS
            .iter()
            .all(|s| !s.title.is_empty() && !s.items.is_empty()));
        assert!(SHORTCUT_SECTIONS
            .iter()
            .all(|s| s.items.iter().all(|i| !i.keys.is_empty() && !i.description.is_empty())));
        let keys = all_keys();
        for expected in ["Ctrl+G", "Ctrl+H"] {
            assert!(
                keys.contains(&expected),
                "快捷键清单缺少 {expected} 条目：{keys:?}"
            );
        }
    }

    #[test]
    fn shortcut_keys_are_unique_across_sections() {
        let keys = all_keys();
        let mut seen: Vec<&'static str> = Vec::new();
        for key in &keys {
            assert!(!seen.contains(key), "键位重复：{key}（清单：{keys:?}）");
            seen.push(key);
        }
    }

    #[test]
    fn rendered_sections_mention_keys_inline() {
        let text: String = section_lines(70)
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        for expected in ["Ctrl+H", "Ctrl+G", "Ctrl+O", "Ctrl+P", "Ctrl+Z", "Ctrl+U"] {
            assert!(text.contains(expected), "渲染结果缺少 {expected}:\n{text}");
        }
    }
}
