//! TUI `/resume` 列表重排 + 会话归档的**独立验收测试**（test-expert 分级：**L2-T** 契约/组件级）。
//!
//! 契约真源：`.dev-team/contract-matrix-tui-resume-archive.md`（**frozen**，C1~C6 / T1~T8）。
//! 本文件由 Test Agent 编写并冻结：实现侧（Executor）只读，**不得就地改断言**；
//! 契约若需变更，必须由 Lead 原地改写契约矩阵并在文末流水登记后，由 Test Agent 重写用例。
//!
//! ## 为什么用 bin-as-module（而非读源码文本的黑盒断言）
//! TUI 的行格式/可见性/键位真值源是 `crates/pony-agent-core/src/bin/tui.rs`（`[[bin]]` 目标，
//! 不是 lib target）。本仓库既有测试（`tui_acceptance.rs` / `tui_render_layout.rs` /
//! `slash_commands_acceptance.rs` 等）已确立 `#[path = "../src/bin/tui.rs"] mod tui;` 模式：
//! 编译期直接引用**同一份真实源码**，断言对象是函数的真实返回值，不是字符串字面量。
//!
//! ## 零假测试纪律（本文件的自我约束）
//! - 每条用例 AAA 显式分区；断言对象是**返回值/屏幕像素**，不存在「跑通即绿」的空用例。
//! - 无 `#[ignore]`、无 `catch_unwind` 吞异常、无「实现未就绪就跳过」的分支。
//! - 真值表由**契约原文**独立推导（例如 C2 的 `is_main && !continuation_only && (show || !archived)`），
//!   不复用被测实现的内部判定，避免同义反复。
//! - 失败的合法形态：`todo!()` 空桩 panic（红相）或断言不成立——两者都必须让测试**非零退出**。
//!
//! ## 数据卫生
//! 本文件只调用**纯函数**与 `ui()` 渲染（TestBackend 内存后端）：
//! 不触碰用户数据目录、不读真实会话库、不启后台进程，用例之间无共享可变状态。
//!
//! ## 用例映射（T1~T8，逐条对应契约矩阵「断言映射」表）
//! | 断言 | 用例 |
//! |---|---|
//! | T1 | `t1_right_column_is_time_then_fixed_order_markers_without_turn_count` / `t1_markers_follow_active_and_selected_flags` / `t1_short_id_is_first_8_chars_of_last_dash_segment` / `t1_width_budget_invariant_across_widths` |
//! | T2 | `t2_cross_workspace_prefix_truth_table` / `t2_label_fallback_to_workspace_id` |
//! | T3 | `t3_visibility_truth_table_exhaustive` / `t3_subagent_and_teammate_never_visible` / `t3_filter_preserves_input_order` |
//! | T4 | `t4_panel_title_reports_visible_count` / `t4_empty_state_reports_current_and_total_counts` |
//! | T5 | `t5_switch_notice_truth_table` / `t5_switch_notice_label_fallback_to_new_id` |
//! | T6 | `t6_archive_plan_blocked_priority` / `t6_archive_plan_routing_matrix` |
//! | T7 | `t7_archive_shortcut_key_matrix` |
//! | T8 | `t8_available_commands_has_exactly_one_archive_entry` |

use pony_agent_core::tui;

use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::Terminal;

use tui::resume_list::{
    archive_plan, resume_row_parts, session_visible, workspace_switch_notice, ArchivePlan,
    ResumeRowInput,
};

// ── 夹具与断言脚手架 ────────────────────────────────────────────────────────────────

/// 行标题夹具：刻意不含 `·` / `轮` / 时间数字，保证「左列结构」断言不被正文干扰（C1 末条）。
const ROW_TITLE: &str = "重构会话列表";

/// 默认「同区、未归档、非当前、非选中」的行输入；各用例按需就地覆写字段。
fn row_input<'a>(session_id: &'a str, title: &'a str, updated_at: &'a str) -> ResumeRowInput<'a> {
    ResumeRowInput {
        session_id,
        title,
        workspace_id: None,
        workspace_label: None,
        updated_at,
        archived: false,
        is_active: false,
        is_selected: false,
        current_workspace_id: None,
    }
}

/// C1 行宽不变量：`str_width(left) + 1 + str_width(right) ≤ width`（`right` 非空时）。
///
/// 宽度口径统一取 SUT 自身的 `tui::str_width`（C1 明确「宽字符按 2 列计」），
/// 因此这条断言对 CJK / emoji 同样成立，不会退化成字节数比较。
fn assert_row_budget(left: &str, right: &str, width: usize, ctx: &str) {
    let left_w = tui::str_width(left);
    let right_w = tui::str_width(right);
    assert!(
        left_w <= width,
        "{ctx}: 左列宽度越界 left={left:?}（{left_w} 列 > width={width}）"
    );
    if !right.is_empty() {
        assert!(
            left_w + 1 + right_w <= width,
            "{ctx}: 行宽预算越界 left={left:?}（{left_w}）+ 1 + right={right:?}（{right_w}）\
             = {} > width={width}",
            left_w + 1 + right_w
        );
    }
}

fn count_occurrences(haystack: &str, needle: &str) -> usize {
    assert!(!needle.is_empty(), "count_occurrences 不接受空针");
    haystack.matches(needle).count()
}

fn screen(state: &mut tui::TuiState, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("TestBackend 构造");
    terminal
        .draw(|f| tui::ui(f, state))
        .expect("ui() 渲染不得失败");
    terminal.backend().buffer().clone()
}

fn row_text(buf: &Buffer, y: u16) -> String {
    let mut out = String::new();
    for x in 0..buf.area.width {
        out.push_str(buf[(x, y)].symbol());
    }
    out
}

fn all_text(buf: &Buffer) -> String {
    (0..buf.area.height)
        .map(|y| row_text(buf, y))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 去掉全部空白后比较。
///
/// **为什么必须这样**：ratatui 的宽字符（CJK / emoji）占 2 列，右侧占位 cell 的
/// `symbol()` 是空格（`Buffer::set_stringn` 里 `reset()` 回落到默认 cell），所以逐格拼接
/// 出来的行文本是「暂 无 历 史 …」而不是「暂无历史…」。直接 `contains("暂无历史会话")`
/// 会**假红**。既有 `tui_render_layout.rs` 的 `squeezed()` 是同因同法。
fn squeeze(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

/// 取逐行 squeeze 后包含 `needle` 的**那一行**（面板文案断言只在该行内做，避免命中无关提示）。
fn line_containing(text: &str, needle: &str) -> Option<String> {
    text.lines().map(squeeze).find(|line| line.contains(needle))
}

/// 解析 `marker` 紧随其后的十进制数字（`marker` 必须存在且紧跟数字，否则硬失败）。
fn number_after(text: &str, marker: &str) -> u64 {
    let idx = text
        .find(marker)
        .unwrap_or_else(|| panic!("文案缺少 marker {marker:?}，实际整屏 {text:?}"));
    let rest = &text[idx + marker.len()..];
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    assert!(
        !digits.is_empty(),
        "marker {marker:?} 之后必须紧跟计数数字，实际尾部 {rest:?}"
    );
    digits.parse().expect("计数必须是合法十进制数字")
}

/// `/resume` 列表行的数据面夹具（字段口径 = 契约 C1/C2 的输入面）。
fn session_item(id: &str, title: &str, updated_at: &str, archived: bool) -> tui::SessionItem {
    tui::SessionItem {
        id: id.to_string(),
        title: title.to_string(),
        updated_at: updated_at.to_string(),
        turn_count: 7,
        workspace_id: Some("ws-main".to_string()),
        workspace_label: Some("主工作区".to_string()),
        archived,
    }
}

// ══ T1：行格式（契约 C1）══════════════════════════════════════════════════════════

#[test]
fn t1_right_column_is_time_then_fixed_order_markers_without_turn_count() {
    // Arrange：三种时间形态（同日 / 同年 / 缺失回退），同一夹具 7 轮（旧实现会把 7 轮塞进右列）。
    for (updated_at, session_id) in [
        ("14:05", "pa_20261121-deadbeefcafe"),
        ("11-21 14:05", "pa_20261121-deadbeefcafe"),
        ("—", "pa_20261121-deadbeefcafe"),
    ] {
        let mut input = row_input(session_id, ROW_TITLE, updated_at);
        input.is_active = true;
        input.is_selected = true;

        // Act
        let parts = resume_row_parts(&input, 200);

        // Assert：右列 = 时间 + 固定顺序标记，且**不得**出现轮次。
        assert_eq!(
            parts.right,
            format!("{updated_at} · 当前 · deadbeef"),
            "C1: 右列必须是「时间 · 当前 · 短id」（updated_at={updated_at:?}）"
        );
        assert_eq!(
            parts.right.split(" · ").next().expect("右列非空"),
            updated_at,
            "C1: 右列首 token 必须是 updated_at 原样（updated_at={updated_at:?}）"
        );
        assert!(
            !parts.right.contains('轮'),
            "C1: 右列不得出现轮次标记「轮」，实际右列 {:?}",
            parts.right
        );
        assert!(
            !parts.right.contains('7'),
            "C1: 右列不得残留 turn_count（夹具 turn_count=7 仅作为数据面字段），实际 {:?}",
            parts.right
        );
        assert!(
            !parts.left.contains(updated_at),
            "C1: 时间只能出现在右列，左列不得重复（updated_at={updated_at:?}，left={:?}）",
            parts.left
        );
        assert_row_budget(&parts.left, &parts.right, 200, "T1/宽 200");
    }
}

#[test]
fn t1_markers_follow_active_and_selected_flags() {
    // Arrange：四组条件 × 期望右列（顺序固定：时间 → 当前 → 短id）。
    let cases = [
        (false, false, "14:05"),
        (true, false, "14:05 · 当前"),
        (false, true, "14:05 · deadbeef"),
        (true, true, "14:05 · 当前 · deadbeef"),
    ];
    for (is_active, is_selected, expected) in cases {
        let mut input = row_input("pa_20261121-deadbeefcafe", ROW_TITLE, "14:05");
        input.is_active = is_active;
        input.is_selected = is_selected;

        // Act
        let parts = resume_row_parts(&input, 200);

        // Assert
        assert_eq!(
            parts.right, expected,
            "C1: is_active={is_active} / is_selected={is_selected} 的右列标记不正确"
        );
        assert_eq!(
            count_occurrences(&parts.right, "当前"),
            usize::from(is_active),
            "C1: 「当前」标记必须且只能由 is_active 决定，实际 {:?}",
            parts.right
        );
    }
}

#[test]
fn t1_short_id_is_first_8_chars_of_last_dash_segment() {
    // Arrange：`-` 末段 >8 / ==8 / <8、无 `-`、以及 CJK（按 char 计，不按字节）。
    for (session_id, short) in [
        ("pa_20261121-deadbeefcafe", "deadbeef"),
        ("session-0123456789", "01234567"),
        ("abcdefghijkl", "abcdefgh"),
        ("x-abc", "abc"),
        ("session-一二三四五六七八九十", "一二三四五六七八"),
    ] {
        let mut input = row_input(session_id, ROW_TITLE, "14:05");
        input.is_selected = true;

        // Act
        let parts = resume_row_parts(&input, 200);

        // Assert：短 id 是右列尾段，且按 char 截断（CJK 用例能区分「字节切片」实现）。
        assert_eq!(
            parts.right,
            format!("14:05 · {short}"),
            "C1: 短 id 必须是 `-` 末段前 8 char（session_id={session_id:?}）"
        );
    }
}

#[test]
fn t1_width_budget_invariant_across_widths() {
    // Arrange：短标题 + 超长 CJK + 混合宽字符（emoji 计 2 列）。
    let long_cjk = "汉".repeat(300);
    let mixed = "中a🚀".repeat(90);
    let titles: [&str; 3] = [ROW_TITLE, &long_cjk, &mixed];

    for width in [0usize, 1, 20, 40, 84, 200] {
        for title in titles {
            for (is_active, is_selected) in [(false, false), (true, true)] {
                let mut input = row_input("pa_20261121-deadbeefcafe", title, "11-21 14:05");
                input.is_active = is_active;
                input.is_selected = is_selected;

                // Act（不得 panic：空桩 todo!() 会在这里爆出，属合法红相）
                let parts = resume_row_parts(&input, width);

                // Assert
                let ctx = format!(
                    "T1/width={width}/title_len={}/active={is_active}/selected={is_selected}",
                    title.chars().count()
                );
                if width == 0 {
                    assert!(
                        parts.left.is_empty() && parts.right.is_empty(),
                        "{ctx}: C1 规定 width == 0 ⇒ 左右皆空，实际 left={:?} right={:?}",
                        parts.left,
                        parts.right
                    );
                }
                assert_row_budget(&parts.left, &parts.right, width, &ctx);
                assert!(
                    tui::str_width(&parts.right) <= width,
                    "{ctx}: 右列不得超出 width（right={:?} 宽 {}）",
                    parts.right,
                    tui::str_width(&parts.right)
                );
            }
        }
    }
}

// ══ T2：跨区前缀真值表（契约 C1 第四条）════════════════════════════════════════════

#[test]
fn t2_cross_workspace_prefix_truth_table() {
    // Arrange：(workspace_id, current_workspace_id, workspace_label, 期望前缀)
    // 前缀判定 = 两个 id 都非空 **且** 不相等；否则无前缀（左列 = 纯标题）。
    let cases: [(Option<&str>, Option<&str>, Option<&str>, Option<&str>); 8] = [
        // 跨区 + 有别名 ⇒ 用别名
        (Some("ws-alpha"), Some("ws-beta"), Some("阿尔法"), Some("阿尔法")),
        // 跨区 + 无别名 ⇒ 回落 workspace_id
        (Some("ws-alpha"), Some("ws-beta"), None, Some("ws-alpha")),
        // 跨区 + 别名全空白 ⇒ 回落 workspace_id
        (Some("ws-alpha"), Some("ws-beta"), Some("   "), Some("ws-alpha")),
        // 同区 ⇒ 无前缀
        (Some("ws-alpha"), Some("ws-alpha"), Some("阿尔法"), None),
        // 行侧 id 缺失 ⇒ 无前缀
        (None, Some("ws-beta"), Some("阿尔法"), None),
        // 当前侧 id 缺失 ⇒ 无前缀
        (Some("ws-alpha"), None, Some("阿尔法"), None),
        // 行侧 id 空白 ⇒ 视为非空 ⇒ 无前缀
        (Some("   "), Some("ws-beta"), Some("阿尔法"), None),
        // 当前侧 id 空白 ⇒ 无前缀
        (Some("ws-alpha"), Some("   "), Some("阿尔法"), None),
    ];

    for (workspace_id, current_workspace_id, label, expected_prefix) in cases {
        let mut input = row_input("pa_20261121-0123456789", ROW_TITLE, "14:05");
        input.workspace_id = workspace_id;
        input.workspace_label = label;
        input.current_workspace_id = current_workspace_id;

        // Act
        let parts = resume_row_parts(&input, 200);

        // Assert
        let ctx = format!("T2 ws={workspace_id:?} cur={current_workspace_id:?} label={label:?}");
        match expected_prefix {
            Some(prefix) => {
                let expected_left = format!("{prefix} · {ROW_TITLE}");
                assert_eq!(
                    parts.left, expected_left,
                    "{ctx}: C1 跨区前缀必须是「<label> · 」且归属左列"
                );
            }
            None => {
                assert_eq!(
                    parts.left, ROW_TITLE,
                    "{ctx}: C1 无跨区前缀时左列只能是标题本身"
                );
            }
        }
        assert_row_budget(&parts.left, &parts.right, 200, &ctx);
    }
}

#[test]
fn t2_label_fallback_to_workspace_id() {
    // Arrange：同一跨区对，仅 label 形态不同（缺失 / 空白 / 带首尾空白）。
    for (label, expected_label) in [
        (None, "ws-alpha"),
        (Some(""), "ws-alpha"),
        (Some("  \t  "), "ws-alpha"),
        (Some("  阿尔法  "), "阿尔法"),
    ] {
        let mut input = row_input("pa_20261121-0123456789", ROW_TITLE, "14:05");
        input.workspace_id = Some("ws-alpha");
        input.current_workspace_id = Some("ws-beta");
        input.workspace_label = label;

        // Act
        let parts = resume_row_parts(&input, 200);

        // Assert
        assert_eq!(
            parts.left,
            format!("{expected_label} · {ROW_TITLE}"),
            "C1: label={label:?} 时前缀必须是「{expected_label} · 」（去空白 / 回落 workspace_id）"
        );
    }
}

// ══ T3：归档可见性（契约 C2）════════════════════════════════════════════════════════

#[test]
fn t3_visibility_truth_table_exhaustive() {
    // 真值口径（契约 C2 原文）：is_main && !continuation_only && (show_archived || !archived)。
    // 这里把 2×2×2×2 = 16 组穷举跑完（契约要求「8 组」，穷举是严格更强的上界）。
    let main_ids = ["", "session-1a2b3c4d", "pa_20261121-deadbeef", "cli-session-abc"];

    for id in main_ids {
        for continuation_only in [false, true] {
            for archived in [false, true] {
                for show_archived in [false, true] {
                    let expected = !continuation_only && (show_archived || !archived);

                    // Act
                    let actual = session_visible(id, archived, continuation_only, show_archived);

                    // Assert
                    assert_eq!(
                        actual, expected,
                        "C2: id={id:?} archived={archived} continuation_only={continuation_only} \
                         show_archived={show_archived} 期望 {expected} 实际 {actual}"
                    );
                }
            }
        }
    }
}

#[test]
fn t3_subagent_and_teammate_never_visible() {
    // Arrange：队友/子代理会话在四种 show_archived × archived 组合下都必须不可见。
    for id in ["subagent-abc", "teammate-abc", "subagent-", "teammate-xyz-123"] {
        for archived in [false, true] {
            for show_archived in [false, true] {
                for continuation_only in [false, true] {
                    // Act
                    let visible = session_visible(id, archived, continuation_only, show_archived);

                    // Assert
                    assert!(
                        !visible,
                        "C2: {id:?} 属子代理/队友会话，任何组合下都必须不可见\
                         （archived={archived} continuation_only={continuation_only} \
                         show_archived={show_archived}）"
                    );
                }
            }
        }
    }
}

#[test]
fn t3_filter_preserves_input_order() {
    // Arrange：数据面乱序（非 updatedAt 排序刻意打乱），过滤只减行、不改序。
    let rows = [
        ("session-e", true, false),  // 归档 → 默认隐藏
        ("session-a", false, false), // 可见
        ("subagent-x", false, false),
        ("session-c", false, true), // 续轮碎片 → 隐藏
        ("session-b", false, false),
        ("session-d", true, false), // 归档 → 默认隐藏
    ];
    let ids: Vec<&str> = rows.iter().map(|(id, _, _)| *id).collect();
    let expected_default = vec!["session-a", "session-b"];
    let expected_show_archived = vec!["session-e", "session-a", "session-b", "session-d"];

    // Act
    let default_view: Vec<&str> = ids
        .iter()
        .zip(rows.iter())
        .filter(|(id, (_, archived, continuation))| {
            session_visible(id, *archived, *continuation, false)
        })
        .map(|(id, _)| *id)
        .collect();
    let archived_view: Vec<&str> = ids
        .iter()
        .zip(rows.iter())
        .filter(|(id, (_, archived, continuation))| {
            session_visible(id, *archived, *continuation, true)
        })
        .map(|(id, _)| *id)
        .collect();

    // Assert：顺序与输入完全一致（不排序 / 不去重 / 不分组）。
    assert_eq!(
        default_view, expected_default,
        "C2: 默认视图必须按输入序过滤（只减行不改序）"
    );
    assert_eq!(
        archived_view, expected_show_archived,
        "C2: show_archived=true 视图必须按输入序过滤（只减行不改序）"
    );
}

// ══ T4：空态与计数（契约 C3，渲染级）══════════════════════════════════════════════

#[test]
fn t4_panel_title_reports_visible_count() {
    // Arrange：3 条可见会话，全部归属当前工作区（不触发跨区前缀）。
    let mut state = tui::TuiState::default();
    state.active_workspace_id = Some("ws-main".to_string());
    state.open_session_selector(vec![
        session_item("session-1a2b3c4d", "会话甲", "14:05", false),
        session_item("session-2b3c4d5e", "会话乙", "14:06", false),
        session_item("session-3c4d5e6f", "会话丙", "14:07", false),
    ]);

    // Act
    let buf = screen(&mut state, 90, 24);
    let text = all_text(&buf);
    let squeezed = squeeze(&text);

    // Assert：标题含「共 {N} 条」，N = 当前可见行数（宽字符占位列的空格已在 squeeze 中消除）。
    assert!(
        squeezed.contains("共3条"),
        "C3: 面板标题必须含「共 3 条」，实际整屏 {text:?}"
    );
    assert_eq!(
        count_occurrences(&squeezed, "共3条"),
        1,
        "C3: 「共 3 条」只应出现在面板标题一次，实际 {text:?}"
    );
    // 三行都必须真的渲染出来（防止「计数对但列表空」的假绿）。
    for title in ["会话甲", "会话乙", "会话丙"] {
        assert!(
            squeezed.contains(title),
            "T4 夹具前提：可见行 {title:?} 必须渲染在面板内，实际 {text:?}"
        );
    }
}

#[test]
fn t4_empty_state_reports_current_and_total_counts() {
    // Arrange：空会话列表（数据面 0 行）。
    let mut state = tui::TuiState::default();
    state.active_workspace_id = Some("ws-main".to_string());
    state.open_session_selector(Vec::new());

    // Act
    let buf = screen(&mut state, 90, 24);
    let text = all_text(&buf);
    let empty_line = line_containing(&text, "暂无历史会话")
        .unwrap_or_else(|| panic!("C3: 空态首段必须以「暂无历史会话」开头，实际整屏 {text:?}"));

    // Assert：两个计数都在同一行内，且数字与数据面自洽（空列表 ⇒ 0 / 0）。
    let current = number_after(&empty_line, "当前工作区");
    let total = number_after(&empty_line, "全部");
    assert_eq!(
        (current, total),
        (0, 0),
        "C3: 空列表时「当前工作区 {current} 条 · 全部 {total} 条」必须自洽为 0/0，实际行 {empty_line:?}"
    );
}

// ══ T5：跨区提示（契约 C4）══════════════════════════════════════════════════════════

#[test]
fn t5_switch_notice_truth_table() {
    // Arrange：(old, new) 全组合；root_path 原样内嵌。
    let root = "/home/dm/projects/beta";
    let cases: [(Option<&str>, Option<&str>, bool); 7] = [
        (None, None, false),                  // 无新旧 ⇒ 无提示
        (None, Some(""), false),              // 新 id 空串 ⇒ 无效
        (None, Some("ws-beta"), true),        // 无历史工作区视为不相等
        (Some("ws-alpha"), None, false),      // 切到 None ⇒ 无提示
        (Some("ws-alpha"), Some(""), false),  // 新 id 空串 ⇒ 无提示
        (Some("ws-alpha"), Some("ws-alpha"), false), // 同工作区 ⇒ 无提示
        (Some("ws-alpha"), Some("ws-beta"), true),   // 真跨区 ⇒ 有提示
    ];

    for (old, new, has_notice) in cases {
        // Act
        let notice = workspace_switch_notice(old, new, Some("贝塔"), root);

        // Assert
        match (has_notice, new) {
            (true, Some(new_id)) => {
                let expected = format!("已切换到工作区 贝塔（{root}）");
                assert_eq!(
                    notice.as_deref(),
                    Some(expected.as_str()),
                    "C4: old={old:?} new={new_id:?} 必须给出精确提示文案"
                );
            }
            _ => assert!(
                notice.is_none(),
                "C4: old={old:?} new={new:?} 不得给出提示，实际 {notice:?}"
            ),
        }
    }
}

#[test]
fn t5_switch_notice_label_fallback_to_new_id() {
    // Arrange：label 缺失 / 全空白 ⇒ 回落 new_id；带首尾空白 ⇒ 去空白后内嵌。
    let root = "/repo";
    for (label, expected_label) in [
        (None, "ws-beta"),
        (Some(""), "ws-beta"),
        (Some("   "), "ws-beta"),
        (Some("  贝塔  "), "贝塔"),
    ] {
        // Act
        let notice = workspace_switch_notice(None, Some("ws-beta"), label, root);

        // Assert
        let expected = format!("已切换到工作区 {expected_label}（{root}）");
        assert_eq!(
            notice.as_deref(),
            Some(expected.as_str()),
            "C4: label={label:?} 时提示必须回落 new_id / 去空白，实际 {notice:?}"
        );
    }
}

// ══ T6：归档三态（契约 C5）══════════════════════════════════════════════════════════

#[test]
fn t6_archive_plan_blocked_priority() {
    // Arrange：阻塞优先于任何参数（参数不参与判定）。
    let cases: [(bool, bool, Option<&str>); 4] = [
        (true, false, None),
        (true, false, Some("abc")),
        (false, true, Some(" abc ")),
        (true, true, Some("   ")),
    ];

    for (is_streaming, turn_in_flight, arg) in cases {
        // Act
        let plan = archive_plan(is_streaming, turn_in_flight, arg);

        // Assert
        assert_eq!(
            plan,
            ArchivePlan::BlockedDuringStreaming,
            "C5: is_streaming={is_streaming} / turn_in_flight={turn_in_flight} 必须先判阻塞\
             （arg={arg:?}），实际 {plan:?}"
        );
    }
}

#[test]
fn t6_archive_plan_routing_matrix() {
    // Arrange：未阻塞 × arg ∈ {None, "", "  ", "abc", " abc ", " abc def "}
    let rotate_cases: [Option<&str>; 3] = [None, Some(""), Some("   ")];
    for arg in rotate_cases {
        // Act
        let plan = archive_plan(false, false, arg);

        // Assert
        assert_eq!(
            plan,
            ArchivePlan::ArchiveCurrentThenRotate,
            "C5: arg={arg:?} 全空白 ⇒ 归档当前会话并轮换 id，实际 {plan:?}"
        );
    }

    let other_cases: [&str; 3] = ["abc", " abc ", " abc def "];
    for arg in other_cases {
        // Act
        let plan = archive_plan(false, false, Some(arg));

        // Assert
        assert_eq!(
            plan,
            ArchivePlan::ArchiveOther(arg.trim().to_string()),
            "C5: arg={arg:?} 必须归档指定 id 且携带 trim 后的目标，实际 {plan:?}"
        );
    }
}

// ══ T7：键位矩阵（契约 C6）══════════════════════════════════════════════════════════

#[test]
fn t7_archive_shortcut_key_matrix() {
    use crossterm::event::{KeyCode, KeyModifiers};

    // Arrange：(code, modifiers, 期望命中)
    let cases = [
        (KeyCode::Char('x'), KeyModifiers::CONTROL, true),
        (KeyCode::Char('X'), KeyModifiers::CONTROL | KeyModifiers::SHIFT, false),
        (KeyCode::Char('x'), KeyModifiers::SHIFT, false),
        (KeyCode::Char('x'), KeyModifiers::ALT, false),
        (KeyCode::Char('X'), KeyModifiers::ALT, false),
        (KeyCode::Char('x'), KeyModifiers::CONTROL | KeyModifiers::ALT, false),
        (KeyCode::Char('x'), KeyModifiers::NONE, false),
        (KeyCode::Char('X'), KeyModifiers::CONTROL, false),
        (KeyCode::Esc, KeyModifiers::CONTROL, false),
    ];

    for (code, modifiers, expected) in cases {
        // Act
        let hit = tui::is_archive_shortcut(code, modifiers);

        // Assert
        assert_eq!(
            hit, expected,
            "C6: 只有「恰好 Ctrl+Char('x')」命中，code={code:?} modifiers={modifiers:?} 期望 {expected}"
        );
    }
}

// ══ T8：命令表（契约 C6）════════════════════════════════════════════════════════════

#[test]
fn t8_available_commands_has_exactly_one_archive_entry() {
    // Arrange：契约 C6 冻结的描述文案（逐字相等）。
    const EXPECTED_DESC: &str = "归档当前会话（/archive <id> 归档指定会话）";

    // Act
    let exact: Vec<(&str, &str)> = tui::AVAILABLE_COMMANDS
        .iter()
        .copied()
        .filter(|(cmd, _)| *cmd == "/archive")
        .collect();
    let prefixed: Vec<&str> = tui::AVAILABLE_COMMANDS
        .iter()
        .map(|(cmd, _)| *cmd)
        .filter(|cmd| cmd.starts_with("/archive"))
        .collect();

    // Assert：含且仅含一条。
    assert_eq!(
        exact.len(),
        1,
        "C6: AVAILABLE_COMMANDS 必须含且仅含一条 /archive，实际 {exact:?}"
    );
    assert_eq!(
        prefixed.len(),
        1,
        "C6: 不得存在 /archive* 的其它变体条目，实际 {prefixed:?}"
    );
    assert_eq!(
        exact[0].1, EXPECTED_DESC,
        "C6: /archive 描述文案必须与冻结契约逐字相等"
    );
    // 顺带守住「命令表不得被夹带别的 /archive 语义」：命令字面量本身也不得重复。
    let all_cmds: Vec<&str> = tui::AVAILABLE_COMMANDS.iter().map(|(cmd, _)| *cmd).collect();
    assert_eq!(
        count_occurrences(&format!("{all_cmds:?}"), "\"/archive\""),
        1,
        "C6: 命令字面量 /archive 只允许出现一次，实际 {all_cmds:?}"
    );
}
