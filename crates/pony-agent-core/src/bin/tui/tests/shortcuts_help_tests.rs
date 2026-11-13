//! Ctrl+G 目标面板 / Ctrl+H 快捷键帮助：**跨模块不变量**测试。
//!
//! 分工边界（避免与实现者的内联单测重复）：
//! - `tui/shortcuts.rs` 内联单测已锁「判定函数只认精确 CONTROL+Char」与渲染文案；
//! - `tui/goal_panel.rs` 内联单测已锁「状态 → 行」的各字段投影；
//! - 本文件只锁**跨模块缝合处**：清单 token 与判定函数是否仍一一对应、
//!   新模式是否仍被判为模态（否则新面板不吞键 = Enter 漏进 Composer）、
//!   空态是否仍给出 `/goal` 入口、`/help` 文案是否仍指向快捷键面板。
//!   这些都可能在「只改一个模块」时被单方面打破，故放在独立缝隙测试里。

use super::*;
use crossterm::event::{KeyCode, KeyModifiers};

/// 清单扁平化后的 (键位 token, 说明) 序列——只读快照，不做断言。
fn flat_items() -> Vec<(&'static str, &'static str)> {
    shortcuts::SHORTCUT_SECTIONS
        .iter()
        .flat_map(|s| s.items.iter().map(|i| (i.keys, i.description)))
        .collect()
}

/// 清单里某个 token 的说明文本；token 必须存在，否则直接失败并打印全清单。
fn description_of(token: &str) -> &'static str {
    let items = flat_items();
    items
        .iter()
        .find(|(keys, _)| *keys == token)
        .map(|(_, description)| *description)
        .unwrap_or_else(|| panic!("快捷键清单缺少 {token} 条目：{items:?}"))
}

#[test]
fn new_overlays_are_modal_and_block_dispatch() {
    // 回归面：新覆盖层若忘记进 is_modal_mode，流式期合成 Enter 会漏进 Composer，
    // 表现为「面板开着却把字发出去」——这条断言是该错配的唯一看守。
    assert!(
        queue::is_modal_mode(&InteractionMode::ShortcutsHelp),
        "ShortcutsHelp 必须被判为模态（吞键）"
    );
    assert!(
        queue::is_modal_mode(&InteractionMode::GoalDetail),
        "GoalDetail 必须被判为模态（吞键）"
    );
    assert!(
        !queue::is_modal_mode(&InteractionMode::Normal),
        "Normal 不是模态（反例，防止 is_modal_mode 退化成恒真）"
    );

    // 派发闸门同一口径：覆盖层打开时不得开始新 turn。
    let mut state = TuiState::default();
    state.mode = InteractionMode::ShortcutsHelp;
    assert!(
        !queue::is_ui_idle_for_dispatch(&state),
        "ShortcutsHelp 下不得派发 turn"
    );
    state.mode = InteractionMode::GoalDetail;
    assert!(
        !queue::is_ui_idle_for_dispatch(&state),
        "GoalDetail 下不得派发 turn"
    );
    state.mode = InteractionMode::Normal;
    assert!(
        queue::is_ui_idle_for_dispatch(&state),
        "Normal 默认态应可派发（反例）"
    );
}

#[test]
fn shortcut_catalog_keeps_minimum_structure() {
    let sections = shortcuts::SHORTCUT_SECTIONS;
    assert!(
        sections.len() >= 4,
        "快捷键清单至少 4 组（全局/输入框编辑/流式与队列/面板与会话），实得 {}",
        sections.len()
    );
    let mut titles: Vec<&'static str> = Vec::new();
    for section in sections {
        assert!(
            !section.title.trim().is_empty(),
            "第 {} 组标题不得为空（空标题渲染出一条不属于任何分组的键位）",
            titles.len()
        );
        assert!(
            !titles.contains(&section.title),
            "组标题重复：{}（清单 {:?}）",
            section.title,
            titles
        );
        titles.push(section.title);
        assert!(
            !section.items.is_empty(),
            "组「{}」不得为空（空组渲染出一个没有条目的标题）",
            section.title
        );
        for item in section.items {
            assert!(
                !item.keys.trim().is_empty(),
                "组「{}」存在空键位条目",
                section.title
            );
            assert!(
                !item.description.trim().is_empty(),
                "键位「{}」缺少说明（清单是给用户看的，空说明等于占位）",
                item.keys
            );
        }
    }
    let items = flat_items();
    assert!(
        items.len() >= 20,
        "清单条目数明显缩水（现应 20+ 条，覆盖全部真实键位），实得 {}：{items:?}",
        items.len()
    );
}

#[test]
fn shortcut_tokens_stay_in_sync_with_predicates() {
    // 不变量：清单里的「Ctrl+H / Ctrl+G」字面量必须真的能被对应判定函数命中。
    // 若有人改了判定（如改成 `contains` 或换键位）却忘了改清单，帮助面板就会说谎。
    let predicates: [(&str, fn(KeyCode, KeyModifiers) -> bool); 2] = [
        ("Ctrl+H", shortcuts::is_shortcuts_help_shortcut),
        ("Ctrl+G", shortcuts::is_goal_panel_shortcut),
    ];
    for (token, predicate) in predicates {
        let items = flat_items();
        assert!(
            items.iter().any(|(keys, _)| *keys == token),
            "清单缺少 {token}：{items:?}"
        );
        let letter = token
            .trim_start_matches("Ctrl+")
            .chars()
            .next()
            .expect("token 必为 Ctrl+<字母>")
            .to_ascii_lowercase();
        assert!(
            predicate(KeyCode::Char(letter), KeyModifiers::CONTROL),
            "{token} 在清单里存在，但判定函数不认 Ctrl+{letter}"
        );
        // 反向：两个快捷键不得互认（复制粘贴改键位时最常见的错）。
        assert!(
            !predicate(KeyCode::Char(letter), KeyModifiers::ALT),
            "{token} 判定不得退化为「裸字母 + 任意修饰键」"
        );
    }
    // 相互独立的交叉反例：H 只能是帮助，G 只能是目标面板。
    assert!(
        !shortcuts::is_shortcuts_help_shortcut(KeyCode::Char('g'), KeyModifiers::CONTROL),
        "Ctrl+G 被误判为快捷键帮助"
    );
    assert!(
        !shortcuts::is_goal_panel_shortcut(KeyCode::Char('h'), KeyModifiers::CONTROL),
        "Ctrl+H 被误判为目标面板"
    );
}

#[test]
fn overlay_entries_describe_their_panel_and_close_gesture() {
    // 可发现性契约：清单条目必须说明「开的是什么面板」且「怎么关」，
    // 否则用户按进去只能靠猜（Esc / 再按同键）。
    for (token, panel_word) in [("Ctrl+H", "帮助"), ("Ctrl+G", "目标")] {
        let description = description_of(token);
        assert!(
            description.contains(panel_word),
            "{token} 说明未点明面板语义（应含「{panel_word}」）：{description}"
        );
        assert!(
            description.contains("关闭"),
            "{token} 说明未写关闭手势：{description}"
        );
    }
}

#[test]
fn goal_panel_empty_state_points_at_slash_goal() {
    // 空态必须可执行：不许出现「整屏空白」或只有一句「没有目标」。
    let state = TuiState::default();
    assert!(
        state.goal_state.is_none(),
        "前置假设：默认 TuiState 没有目标"
    );
    let lines = goal_panel::render_lines(&state);
    assert!(!lines.is_empty(), "空态不得渲染零行");
    let joined = lines.join("\n");
    assert!(
        joined.contains("没有目标"),
        "空态应说明当前没有目标：{joined}"
    );
    assert!(
        joined.contains("/goal"),
        "空态必须给出 /goal 创建入口：{joined}"
    );
}

#[test]
fn goal_panel_with_goal_shows_objective_text() {
    let mut state = TuiState::default();
    let objective = "把 Ctrl+G 目标面板接到真实 goal 状态";
    state.goal_state = Some(TuiGoal::new(
        "goal-shortcuts-test".to_string(),
        objective.to_string(),
        Some(6),
    ));
    let lines = goal_panel::render_lines(&state);
    let joined = lines.join("\n");
    assert!(
        joined.contains(objective),
        "有目标时面板必须显示 objective 原文：{joined}"
    );
    // 顺序不变量：目标正文在最前，元信息（阶段/轮次）在其后——
    // 反过来的面板等于让用户先读元数据再读目标。
    let objective_idx = lines
        .iter()
        .position(|line| line.contains(objective))
        .expect("objective 行必须存在");
    let phase_idx = lines
        .iter()
        .position(|line| line.starts_with("阶段"))
        .expect("阶段行必须存在");
    assert!(
        objective_idx < phase_idx,
        "objective 行（#{objective_idx}）应在阶段行（#{phase_idx}）之前：{lines:?}"
    );
    // 面板是只读的：底部必须把变更路径指回 /goal 命令。
    assert!(
        lines.iter().any(|line| line.contains("/goal")),
        "面板底部必须给出 /goal 操作入口：{lines:?}"
    );
}

#[test]
fn help_command_description_mentions_shortcuts() {
    // 防文档撤回：/help 曾是「只看命令列表」的唯一入口，Ctrl+H 面板的可发现性
    // 依赖这条描述；有人把「快捷键」删掉，用户就再也找不到 Ctrl+H。
    let entry = AVAILABLE_COMMANDS
        .iter()
        .find(|(cmd, _)| *cmd == "/help")
        .unwrap_or_else(|| panic!("/help 必须仍在命令清单里：{AVAILABLE_COMMANDS:?}"));
    assert!(
        entry.1.contains("快捷键"),
        "/help 描述必须点明快捷键清单（当前：{}）",
        entry.1
    );
}
