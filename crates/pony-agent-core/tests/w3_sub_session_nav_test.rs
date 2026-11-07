// W3b：TUI 子智能体会话导航 + 身份徽标 —— 黑盒验收契约（冻结，Test Agent 所有）。
//
// 契约真值源：`docs/decisions/0083-tui-sub-session-navigation-and-badge.md` 的验收表 C1–C14；
// 实现真值源：`src/bin/tui.rs` 的 `sub_session_shortcut` / `sub_session_candidates` /
// `step_sub_session_target` / `sub_session_badge_text` / `is_agent_roster_command`
// 与 `TuiState::reset_session_scope`（`sub_session` / `return_session_id` 同属会话作用域）。
//
// 与 `src/bin/tui.rs` 内联单测（`mod w3b_sub_session_tests`）刻意不共享代码：
// 内联单测守实现细节，本文件守**对外契约**（文案逐字、渲染行位置与几何、作用域复位面、
// 命令分发顺序），二者互为镜像。用 ratatui TestBackend 做真实渲染，bin 的 main 不会被调用。

#[path = "../src/bin/tui.rs"]
#[allow(dead_code, unused_imports, unused_variables, unused_mut)]
mod tui;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::Color;
use ratatui::Terminal;

/// 主题字面值（`tui::theme` 为私有模块，无法从测试引用；按现实现值冻结）。
/// COMposer 色带 / 徽标行背景 = SURFACE，画布 = CANVAS。
const CANVAS: Color = Color::Rgb(18, 16, 14);
const SURFACE: Color = Color::Rgb(27, 25, 22);

/// 契约 C14/C15 的静态真值：命令分发顺序（`/tasks` 分支必须先于花名册分支）。
const TUI_SOURCE: &str = include_str!("../src/bin/tui.rs");

// ---------------------------------------------------------------------------
// 构造与取值工具
// ---------------------------------------------------------------------------

fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, modifiers)
}

fn roster_entry(target: &str, session_id: Option<&str>) -> tui::AgentRosterEntry {
    tui::AgentRosterEntry {
        target: target.to_string(),
        name: format!("{target}-name"),
        role: "executor".to_string(),
        status: "inactive".to_string(),
        pending_inbox: 0,
        driving: false,
        session_id: session_id.map(str::to_string),
    }
}

fn context(position: Option<(usize, usize)>) -> tui::SubSessionContext {
    tui::SubSessionContext {
        target: "worker-1".to_string(),
        name: "Ada".to_string(),
        role: "executor".to_string(),
        position,
    }
}

fn three_candidates() -> Vec<tui::SubSessionCandidate> {
    let entries = vec![
        roster_entry("worker-1", Some("sess-1")),
        roster_entry("worker-2", Some("sess-2")),
        roster_entry("worker-3", Some("sess-3")),
    ];
    tui::sub_session_candidates(&entries, |_| true)
}

fn targets(candidates: &[tui::SubSessionCandidate]) -> Vec<&str> {
    candidates
        .iter()
        .map(|candidate| candidate.target.as_str())
        .collect()
}

fn stepped(
    candidates: &[tui::SubSessionCandidate],
    current: Option<&str>,
    step: tui::SubSessionStep,
) -> Option<String> {
    tui::step_sub_session_target(candidates, current, step)
        .map(|candidate| candidate.target.clone())
}

fn render(state: &mut tui::TuiState, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("TestBackend 应可创建");
    terminal
        .draw(|frame| tui::ui(frame, state))
        .expect("ui() 渲染不得失败");
    terminal.backend().buffer().clone()
}

/// 与 tui.rs `char_width` 同构的东亚宽字符判定：宽字符右侧是占位空格，
/// 逐字符拼接会得到 `缓 存` 这类假失败，断言前必须按「视觉字符串」还原。
fn disp_width(ch: char) -> usize {
    let cp = ch as u32;
    let wide = matches!(cp,
        0x1100..=0x115F
            | 0x2E80..=0x303E
            | 0x3041..=0x33FF
            | 0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xA000..=0xA4CF
            | 0xA960..=0xA97F
            | 0xAC00..=0xD7A3
            | 0xF900..=0xFAFF
            | 0xFE10..=0xFE19
            | 0xFE30..=0xFE6F
            | 0xFF00..=0xFF60
            | 0xFFE0..=0xFFE6
            | 0x1F300..=0x1F64F
            | 0x1F900..=0x1F9FF
            | 0x20000..=0x3FFFD);
    if wide { 2 } else { 1 }
}

fn row_text(buf: &Buffer, y: u16) -> String {
    let mut out = String::new();
    let mut skip_placeholder = false;
    for x in 0..buf.area.width {
        if skip_placeholder {
            skip_placeholder = false;
            continue;
        }
        let symbol = buf[(x, y)].symbol();
        out.push_str(symbol);
        let mut chars = symbol.chars();
        if let (Some(ch), None) = (chars.next(), chars.next()) {
            if disp_width(ch) == 2 {
                skip_placeholder = true;
            }
        }
    }
    out
}

/// 去空白匹配：宽字符占位格与截断空白不应影响「行内是否含某语义片段」。
fn find_row(buf: &Buffer, needle: &str) -> Option<u16> {
    (0..buf.area.height).find(|y| {
        let text: String = row_text(buf, *y)
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        text.contains(needle)
    })
}

// ---------------------------------------------------------------------------
// C1/C2：徽标文案真值（逐字断言）
// ---------------------------------------------------------------------------

#[test]
fn w3b_a1_badge_text_carries_identity_and_shortcut_hints() {
    let text = tui::sub_session_badge_text(&context(Some((2, 5))));
    assert_eq!(
        text, "子智能体 · Ada（executor） 2/5 · Ctrl+↑ 返回主会话 · Ctrl+←/→ 切换",
        "徽标文案是唯一真值（ADR 0083 §决策 7），必须逐字匹配"
    );
    // 语义四要素各自可见：身份、角色、1-based 序号、两条快捷键提示。
    for needle in ["子智能体", "Ada", "（executor）", "2/5", "Ctrl+↑ 返回主会话", "Ctrl+←/→ 切换"] {
        assert!(text.contains(needle), "徽标必须含 {needle:?}，实际 {text:?}");
    }
    // 序号必须带空格与 `·` 分隔（避免 `）2/5` 这种黏连）。
    assert!(
        text.contains("） 2/5 · "),
        "序号段必须以「空格 + 序号 + 空格 + ·」嵌入，实际 {text:?}"
    );
}

#[test]
fn w3b_a2_badge_text_omits_sequence_when_position_unknown() {
    let text = tui::sub_session_badge_text(&context(None));
    assert_eq!(
        text, "子智能体 · Ada（executor） · Ctrl+↑ 返回主会话 · Ctrl+←/→ 切换",
        "position=None 时必须省略整段序号（ADR 0083 §决策 7）"
    );
    // 不得输出半截 `n/`，也不得用占位符顶替（如 `?/?`、`0/0`、`—`）。
    // 注意：快捷键段本身含 `Ctrl+←/→` 的斜杠，故判据是「整段不得出现数字」
    // （序号是唯一数字来源，`0/0`、`?/?` 这类降级同样会被下面两项一并抓住）。
    assert!(
        !text.chars().any(|c| c.is_ascii_digit()),
        "position 未知时不得留下任何序号数字：{text:?}"
    );
    for placeholder in ['—', '?'] {
        assert!(
            !text.contains(placeholder),
            "position 未知时不得出现占位符 {placeholder:?}：{text:?}"
        );
    }
    let (_, tail) = text
        .split_once("（executor）")
        .expect("徽标必须含角色括号段");
    assert_eq!(
        tail, " · Ctrl+↑ 返回主会话 · Ctrl+←/→ 切换",
        "角色段与快捷键段必须直接相邻（中间只有单个分隔空格）：{text:?}"
    );
}

// ---------------------------------------------------------------------------
// C3/C4：徽标行渲染位置与极限窄终端
// ---------------------------------------------------------------------------

#[test]
fn w3b_a3_sub_session_renders_badge_row_and_main_session_does_not() {
    // 主会话：不出现徽标（也不占行）。几何与 W3b 之前完全一致 ⇒ 输入行在 height-3。
    let mut main_state = tui::TuiState::default();
    let main = render(&mut main_state, 80, 24);
    assert!(
        find_row(&main, "子智能体").is_none(),
        "主会话不得渲染子会话徽标，实际命中行 {:?}",
        find_row(&main, "子智能体")
    );
    assert!(
        find_row(&main, "Ctrl+↑").is_none(),
        "主会话不得出现返回主会话提示"
    );
    let main_prompt_row = find_row(&main, "›").expect("主会话必须渲染输入提示符");

    // 子会话：徽标占 1 行，紧贴状态行（最后一行）上方 ⇒ 行号 height-2。
    let mut sub_state = tui::TuiState::default();
    sub_state.sub_session = Some(context(Some((1, 2))));
    let sub = render(&mut sub_state, 80, 24);
    let badge_row = find_row(&sub, "子智能体").expect("子会话必须渲染身份徽标");
    assert_eq!(
        badge_row,
        sub.area.height - 2,
        "徽标必须占「输入区正下方 1 行」（ADR 0083 §决策 7），实际 y={badge_row}"
    );
    let badge_text: String = row_text(&sub, badge_row)
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    assert!(
        badge_text.starts_with("◆"),
        "徽标行必须以 `◆` 标记起排：{badge_text:?}"
    );
    for needle in ["子智能体", "Ada", "executor", "1/2", "返回主会话"] {
        assert!(
            badge_text.contains(needle),
            "徽标行必须含 {needle:?}，实际 {badge_text:?}"
        );
    }

    // 恰好 +1 行：主会话输入行 height-3，子会话 height-4（徽标不多占、不少占）。
    let sub_prompt_row = find_row(&sub, "›").expect("子会话必须渲染输入提示符");
    assert_eq!(
        main_prompt_row,
        main.area.height - 3,
        "主会话输入行位置不得变化，实际 y={main_prompt_row}"
    );
    assert_eq!(
        sub_prompt_row,
        sub.area.height - 4,
        "徽标把输入区整体上移恰好 1 行，实际 y={sub_prompt_row}"
    );
    assert_eq!(
        sub_prompt_row + 3,
        badge_row + 1,
        "徽标行与输入区之间只允许 1 行呼吸行（输入行 → 呼吸行 → 徽标行 → 状态行）"
    );

    // 徽标行落在 composer 色带（SURFACE）上，左右外沟槽仍是画布（CANVAS）。
    let content_x = 3;
    assert_eq!(
        sub[(content_x, badge_row)].bg,
        SURFACE,
        "徽标行必须落在色带底（ADR 0083 §决策 7 与 ADR 0042 R7）"
    );
    assert_eq!(
        sub[(0, badge_row)].bg,
        CANVAS,
        "徽标不得写出内容列（左右沟槽保持画布底）"
    );
    // 最后一行（状态行）不得残留徽标身份：身份只在徽标行出现一次。
    let status_row = row_text(&sub, sub.area.height - 1);
    assert!(
        !status_row.contains("子智能体"),
        "状态行不得重复徽标身份，实际 {status_row:?}"
    );
}

#[test]
fn w3b_a4_badge_survives_extremely_narrow_terminal() {
    // 退化宽高：既不得 panic，也不得把内容写到画布外（缓冲区尺寸不可变）。
    for (width, height) in [(1u16, 3u16), (2, 4), (3, 5), (8, 6), (12, 8), (40, 10), (80, 24)] {
        for position in [None, Some((1, 2))] {
            let mut state = tui::TuiState::default();
            state.sub_session = Some(context(position));
            let buf = render(&mut state, width, height);
            assert_eq!(
                (buf.area.width, buf.area.height),
                (width, height),
                "渲染不得改变画布尺寸（{width}x{height}）"
            );
        }
    }

    // 退化到「极小终端」阈值以内（<24 列或 <10 行）：只给一句可读提示，
    // 不得输出半截布局（徽标/输入区都不得出现），这是 ui() 的既有降级契约。
    for (width, height) in [(1u16, 3u16), (8, 9), (20, 10), (23, 24), (40, 9)] {
        let mut state = tui::TuiState::default();
        state.sub_session = Some(context(Some((1, 2))));
        let buf = render(&mut state, width, height);
        let text: String = (0..buf.area.height).map(|y| row_text(&buf, y)).collect();
        let squeezed: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        // 降级提示本身也受画布宽度限制：1 列连一个 2 列宽汉字都放不下，
        // 此时「画布全空但不 panic」才是可观测的真值；≥2 列必须出现提示首字。
        if width == 1 {
            assert!(
                squeezed.is_empty(),
                "1 列画布不得写出任何字形（宽字符放不下即整格丢弃），实际 {text:?}"
            );
        } else {
            assert!(
                squeezed.contains('终'),
                "{width}x{height} 必须走极小终端降级提示，实际 {text:?}"
            );
        }
        // 提示文案按画布列宽截断（6 个 CJK 字 = 12 列），故按列宽分级断言可读性。
        if width >= 8 {
            assert!(
                squeezed.contains("终端窗口"),
                "{width}x{height} 降级提示至少前 4 字必须可读，实际 {text:?}"
            );
        }
        if width >= 12 {
            assert!(
                squeezed.contains("终端窗口过小"),
                "{width}x{height} 降级提示必须完整可读，实际 {text:?}"
            );
        }
        assert!(
            !squeezed.contains("子智能体") && !squeezed.contains("›"),
            "{width}x{height} 降级态不得输出半截徽标/输入区，实际 {text:?}"
        );
    }

    // 刚过阈值（≥24 列、≥10 行）就必须渲染徽标，且截断保留头部（`◆` + 身份），
    // 不得把整块徽标丢弃，也不得写出画布。
    for width in [24u16, 28, 34, 40, 80] {
        let mut state = tui::TuiState::default();
        state.sub_session = Some(context(Some((3, 7))));
        let buf = render(&mut state, width, 12);
        let badge_row = find_row(&buf, "◆").expect("宽度 ≥24 时必须渲染徽标标记");
        let raw = row_text(&buf, badge_row);
        let trimmed = raw.trim_start();
        assert!(
            trimmed.starts_with('◆'),
            "宽度 {width} 时徽标必须仍以 `◆` 起排，实际 {raw:?}"
        );
        assert!(
            trimmed.contains('子'),
            "宽度 {width} 时徽标身份段不得整块丢弃，实际 {raw:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// C5/C6：快捷键裁决
// ---------------------------------------------------------------------------

#[test]
fn w3b_b1_ctrl_up_is_return_to_main_shortcut() {
    assert_eq!(
        tui::sub_session_shortcut(&key(KeyCode::Up, KeyModifiers::CONTROL)),
        Some(tui::SubSessionShortcut::ReturnToMain),
        "Ctrl+↑ 必须是「返回主会话」"
    );
    // 判据是 `modifiers.contains(CONTROL)`：与 SHIFT 组合不改变裁决。
    assert_eq!(
        tui::sub_session_shortcut(&key(
            KeyCode::Up,
            KeyModifiers::CONTROL | KeyModifiers::SHIFT
        )),
        Some(tui::SubSessionShortcut::ReturnToMain),
        "Ctrl+Shift+↑ 仍应返回主会话（contains 语义）"
    );
}

#[test]
fn w3b_b2_plain_and_alt_arrows_are_not_consumed() {
    // 裸方向键 / Alt+方向键 / Ctrl+Char / Ctrl+其它方向键一律 None（调用方不消费）。
    for code in [
        KeyCode::Up,
        KeyCode::Down,
        KeyCode::Left,
        KeyCode::Right,
        KeyCode::Home,
        KeyCode::End,
    ] {
        for modifiers in [KeyModifiers::NONE, KeyModifiers::ALT, KeyModifiers::SHIFT] {
            assert_eq!(
                tui::sub_session_shortcut(&key(code, modifiers)),
                None,
                "{code:?} + {modifiers:?} 不得被子会话导航消费（主会话语义必须保留）"
            );
        }
    }
    for code in [KeyCode::Char('a'), KeyCode::Char('↑'), KeyCode::Enter] {
        assert_eq!(
            tui::sub_session_shortcut(&key(code, KeyModifiers::CONTROL)),
            None,
            "Ctrl+{code:?} 不是子会话导航键"
        );
    }
    assert_eq!(
        tui::sub_session_shortcut(&key(KeyCode::Down, KeyModifiers::CONTROL)),
        None,
        "Ctrl+↓ 未定义 ⇒ 不得消费"
    );
}

// ---------------------------------------------------------------------------
// C7/C13：返回主会话 / 作用域复位
// ---------------------------------------------------------------------------

#[test]
fn w3b_b3_returning_to_main_resets_session_scope() {
    let mut state = tui::TuiState::default();
    let session_before = state.session_id.clone();
    // 模拟「已进入队友会话」：主会话返回目标 + 花名册面板 + 队友身份三者同时在场。
    state.return_session_id = Some("main-session".to_string());
    state.agent_roster_state = Some(tui::AgentRosterState::new(vec![
        roster_entry("worker-1", Some("sess-1")),
        roster_entry("worker-2", Some("sess-2")),
    ]));
    state.sub_session = Some(context(Some((2, 2))));

    // 返回主会话的收口（ADR 0083 §决策 5/8）= `reset_session_scope()` + 调用方写回主会话 id。
    state.reset_session_scope();
    assert!(
        state.sub_session.is_none(),
        "返回主会话后不得残留队友身份：{:?}",
        state.sub_session
    );
    assert!(
        state.return_session_id.is_none(),
        "返回主会话后不得残留返回目标：{:?}",
        state.return_session_id
    );
    assert!(
        state.agent_roster_state.is_none(),
        "返回主会话后花名册面板必须关闭"
    );
    // `reset_session_scope` 不改 `session_id`（切换会话由调用方负责）。
    assert_eq!(
        state.session_id, session_before,
        "reset_session_scope 不得自行改写 session_id"
    );
    // 复位后再渲染：徽标行彻底消失（不留空行）。
    let buf = render(&mut state, 80, 24);
    assert!(
        find_row(&buf, "子智能体").is_none(),
        "返回主会话后不得渲染徽标"
    );
    assert!(
        find_row(&buf, "›").is_some(),
        "返回主会话后输入区必须照常渲染"
    );
}

#[test]
fn w3b_e1_reset_session_scope_clears_identity_and_return_target() {
    // 与 b3 的分工：b3 守「返回主会话」的完整收口，本用例守复位面本身
    // 以及 ADR 0083 §决策 5 的写序契约（return_session_id 必须在 reset 之后写回）。
    let mut state = tui::TuiState::default();
    state.return_session_id = Some("main-session".to_string());
    state.sub_session = Some(context(None));
    state.reset_session_scope();
    assert!(state.sub_session.is_none(), "复位必须清空 sub_session");
    assert!(
        state.return_session_id.is_none(),
        "复位必须清空 return_session_id"
    );

    // 复位之后再写回返回目标 ⇒ 必须被保留（队友会话之间切换不得丢失最初主会话 id）。
    state.reset_session_scope();
    state.return_session_id = Some("main-session".to_string());
    state.sub_session = Some(context(Some((1, 3))));
    assert_eq!(
        state.return_session_id.as_deref(),
        Some("main-session"),
        "reset 之后写入的返回目标必须保留（写序：reset → 写回）"
    );
    assert!(
        state.sub_session.is_some(),
        "reset 之后写入的队友身份必须保留"
    );
}

// ---------------------------------------------------------------------------
// C8/C9/C10：Ctrl+←/→ 映射与循环步进
// ---------------------------------------------------------------------------

#[test]
fn w3b_c1_ctrl_arrows_map_to_next_and_prev() {
    assert_eq!(
        tui::sub_session_shortcut(&key(KeyCode::Right, KeyModifiers::CONTROL)),
        Some(tui::SubSessionShortcut::Switch(tui::SubSessionStep::Next)),
        "Ctrl+→ 必须是「下一个」"
    );
    assert_eq!(
        tui::sub_session_shortcut(&key(KeyCode::Left, KeyModifiers::CONTROL)),
        Some(tui::SubSessionShortcut::Switch(tui::SubSessionStep::Prev)),
        "Ctrl+← 必须是「上一个」"
    );
    // 方向不可互换（防左右写反）。
    assert_ne!(
        tui::sub_session_shortcut(&key(KeyCode::Left, KeyModifiers::CONTROL)),
        tui::sub_session_shortcut(&key(KeyCode::Right, KeyModifiers::CONTROL)),
        "Ctrl+← 与 Ctrl+→ 的裁决必须不同"
    );
}

#[test]
fn w3b_c2_switching_wraps_around_at_both_ends() {
    let candidates = three_candidates();
    assert_eq!(
        targets(&candidates),
        vec!["worker-1", "worker-2", "worker-3"],
        "候选顺序必须沿用花名册（不重排、不去重）"
    );
    let next = tui::SubSessionStep::Next;
    let prev = tui::SubSessionStep::Prev;

    // 两端回绕。
    assert_eq!(
        stepped(&candidates, Some("worker-3"), next).as_deref(),
        Some("worker-1"),
        "末位的下一个必须回绕到首位"
    );
    assert_eq!(
        stepped(&candidates, Some("worker-1"), prev).as_deref(),
        Some("worker-3"),
        "首位的上一个必须回绕到末位"
    );
    // 中间步进。
    assert_eq!(stepped(&candidates, Some("worker-1"), next).as_deref(), Some("worker-2"));
    assert_eq!(stepped(&candidates, Some("worker-3"), prev).as_deref(), Some("worker-2"));
    // 从「主会话」口径（current=None）进入：Next 取首、Prev 取末。
    assert_eq!(stepped(&candidates, None, next).as_deref(), Some("worker-1"));
    assert_eq!(stepped(&candidates, None, prev).as_deref(), Some("worker-3"));
    // 当前目标已不在候选内（会话被回收）：Next 取首、Prev 取末，不得 panic。
    assert_eq!(stepped(&candidates, Some("gone"), next).as_deref(), Some("worker-1"));
    assert_eq!(stepped(&candidates, Some("gone"), prev).as_deref(), Some("worker-3"));
}

#[test]
fn w3b_c3_single_candidate_steps_to_itself() {
    let candidates = tui::sub_session_candidates(
        &[roster_entry("worker-1", Some("sess-1"))],
        |_| true,
    );
    assert_eq!(candidates.len(), 1);
    assert_eq!(
        stepped(&candidates, Some("worker-1"), tui::SubSessionStep::Next).as_deref(),
        Some("worker-1"),
        "单候选的 Next 必须步进到自身（不得返回 None 触发误提示）"
    );
    assert_eq!(
        stepped(&candidates, Some("worker-1"), tui::SubSessionStep::Prev).as_deref(),
        Some("worker-1"),
        "单候选的 Prev 必须步进到自身"
    );
    // 单候选与「只有一个子会话」的 UI 提示口径一致：切换目标 == 当前目标。
    assert_eq!(
        stepped(&candidates, None, tui::SubSessionStep::Next).as_deref(),
        Some("worker-1"),
        "从主会话口径进入单候选必须命中它"
    );
}

// ---------------------------------------------------------------------------
// C11/C12：候选过滤
// ---------------------------------------------------------------------------

#[test]
fn w3b_d1_candidates_exclude_lead_sessionless_and_unloadable_entries() {
    let entries = vec![
        roster_entry("lead", Some("sess-lead")),
        roster_entry("worker-1", Some("sess-1")),
        roster_entry("worker-2", None),
        roster_entry("worker-3", Some("sess-3")),
        roster_entry("worker-4", Some("sess-4")),
    ];
    // `sess-3` 宿主加载不到（hydrate 只会得到空会话）；`sess-4` 可加载。
    let candidates = tui::sub_session_candidates(&entries, |session_id| {
        session_id != "sess-3"
    });
    assert_eq!(
        targets(&candidates),
        vec!["worker-1", "worker-4"],
        "必须剔除 lead / 无会话 / 不可加载三类条目，并保持花名册顺序"
    );
    // 字段面：候选必须携带面板展示与 hydrate 所需全部字段。
    assert_eq!(candidates[0].session_id, "sess-1");
    assert_eq!(candidates[0].name, "worker-1-name");
    assert_eq!(candidates[0].role, "executor");
    assert_eq!(candidates[1].session_id, "sess-4");
    // 全部不可加载 ⇒ 空候选（不得伪造空会话）。
    assert!(
        tui::sub_session_candidates(&entries, |_| false).is_empty(),
        "全部不可加载时必须返回空候选"
    );
    // 只有 lead ⇒ 空候选（主会话无需进入自己）。
    assert!(
        tui::sub_session_candidates(&[roster_entry("lead", Some("sess-lead"))], |_| true).is_empty(),
        "只有 lead 时不得产生候选"
    );
}

#[test]
fn w3b_d2_empty_candidates_make_stepping_a_safe_no_op() {
    let empty: Vec<tui::SubSessionCandidate> = Vec::new();
    for step in [tui::SubSessionStep::Next, tui::SubSessionStep::Prev] {
        assert!(
            tui::step_sub_session_target(&empty, None, step).is_none(),
            "空候选 + current=None 必须安全 no-op（{step:?}）"
        );
        assert!(
            tui::step_sub_session_target(&empty, Some("worker-1"), step).is_none(),
            "空候选 + current=Some 必须安全 no-op（{step:?}）"
        );
    }
}

// ---------------------------------------------------------------------------
// C14/C15：`/teams` 别名 与 `/tasks` → `/agents` 分发顺序契约
// ---------------------------------------------------------------------------

#[test]
fn w3b_f1_agents_and_teams_commands_are_equivalent() {
    for slug in ["/agents", "/teams"] {
        for raw in [slug.to_string(), format!("  {slug} "), slug.to_uppercase()] {
            assert!(
                tui::is_agent_roster_command(&raw),
                "{raw:?} 必须被判为花名册命令（trim + 大小写不敏感）"
            );
        }
    }
    // 两个斜杠命令必须走同一判定（ADR 0083 §决策 9：等价，不写两份字面量）。
    assert_eq!(
        tui::is_agent_roster_command("/agents"),
        tui::is_agent_roster_command("/teams"),
        "/agents 与 /teams 必须等价"
    );
    // 带参数 / 近似拼写 / 其它命令一律不得命中。
    for raw in [
        "/agents worker-1",
        "/teams worker-1",
        "/team",
        "/ags",
        "/agent",
        "",
        "agents",
        "/tasks",
    ] {
        assert!(
            !tui::is_agent_roster_command(raw),
            "{raw:?} 不得被判为花名册命令"
        );
    }
}

#[test]
fn w3b_f2_tasks_branch_precedes_agent_roster_branch() {
    // `/tasks` 打开 Team 共享看板，`/agents` 打开花名册——两者语义不同。
    // 分发点必须**先**匹配 `/tasks`，再走 `is_agent_roster_command`；
    // 同时判定函数不得把 `/tasks` 当作花名册别名（双重保险，任一失效都会被本用例抓住）。
    assert!(
        !tui::is_agent_roster_command("/tasks"),
        "`/tasks` 不得被花名册判定吞掉（否则看板命令会被改写成面板命令）"
    );

    let tasks_branch = TUI_SOURCE
        .find("if known_command == Some(\"/tasks\")")
        .expect("分发点必须存在 `/tasks` 分支");
    let roster_branch = TUI_SOURCE
        .find("if matches!(known_command, Some(cmd) if is_agent_roster_command(cmd))")
        .expect("分发点必须存在 `/agents`/`/teams` 花名册分支");
    assert!(
        tasks_branch < roster_branch,
        "`/tasks` 分支必须先于花名册分支（实际 {tasks_branch} vs {roster_branch}）"
    );

    // 花名册分支不得再硬编码 `/agents` 字面量（必须复用同一判定，避免两份真值漂移）。
    let roster_block = &TUI_SOURCE[roster_branch..(roster_branch + 400).min(TUI_SOURCE.len())];
    assert!(
        roster_block.contains("is_agent_roster_command(cmd)"),
        "花名册分支必须复用 `is_agent_roster_command`：{roster_block:?}"
    );
    assert!(
        !roster_block.contains("Some(\"/agents\")"),
        "花名册分支不得再写一份 `/agents` 字面量（ADR 0083 §决策 9）"
    );
}
