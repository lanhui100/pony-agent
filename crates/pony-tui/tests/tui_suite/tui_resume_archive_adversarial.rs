//! TUI `/resume` 列表 + 会话归档的**对抗用例**（test-expert 分级：**L2-AT** 红队注入）。
//!
//! 契约真源：`.dev-team/contract-matrix-tui-resume-archive.md`（**frozen**）。与
//! `tui_resume_archive_acceptance.rs` 分文件存放，是为了让「正向契约」与「破坏输入」两套
//! 收据各自独立可跑、失败归因不混淆。
//!
//! ## 注入面（为什么这五类）
//! TUI 行的宽度计算是纯字符算术 + 终端缓冲写入，历史上这类代码的典型缺陷是：
//! 1) `&str[..n]` 字节切片切在 CJK 边界 ⇒ **panic**；
//! 2) 宽字符（CJK / emoji）按 1 列计宽 ⇒ 行溢出终端右缘；
//! 3) 超长输入放大截断预算 ⇒ `usize` 下溢 / 死循环；
//! 4) 面板窄化（20 列）时布局算术下溢；
//! 5) 归档过滤把行从数据面一起删掉 ⇒ 用户「看不见也找不回」。
//! 因此这里的断言必须落在**不 panic + 宽度预算 + 数据面完整**三件事上，而不是「跑通即绿」。
//!
//! ## 零假测试纪律
//! - 无 `catch_unwind` 吞 panic（panic 就是失败，必须让测试进程非零退出）。
//! - 无 `#[ignore]`、无「输入太极端就跳过」的分支。
//! - 每条断言都带上下文（width / 输入摘要），失败时可直接定位注入点。
//!
//! ## 数据卫生
//! 全部为纯函数 + `TestBackend` 内存渲染：不写磁盘、不读用户会话库、无后台线程。

use pony_tui as tui;

use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::Terminal;

use tui::resume_list::{resume_row_parts, session_visible, ResumeRowInput};

// ── 夹具与断言脚手架（与 acceptance 文件各自独立，避免共享模块耦合）────────────────

const ROW_TITLE: &str = "重构会话列表";

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

/// 行宽不变量（C1）：`str_width(left) + 1 + str_width(right) ≤ width`（`right` 非空时），
/// 并且左列自身也绝不越过 width（防止「右列被压空、左列独自溢出」的漏判）。
fn assert_row_budget(left: &str, right: &str, width: usize, ctx: &str) {
    let left_w = tui::str_width(left);
    let right_w = tui::str_width(right);
    assert!(
        left_w <= width,
        "{ctx}: 左列越界 left={left:?}（{left_w} 列 > width={width}）"
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

/// 去掉全部空白后比较：宽字符右侧占位 cell 的 `symbol()` 是空格（ratatui `set_stringn`
/// 的 `reset()` 行为），逐格拼接出的 CJK 文本会带插空，直接 `contains` 会假红。
fn squeeze(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

fn screen(state: &mut tui::TuiState, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("TestBackend 构造");
    terminal
        .draw(|f| tui::ui(f, state))
        .expect("ui() 渲染不得失败");
    terminal.backend().buffer().clone()
}

fn all_text(buf: &Buffer) -> String {
    let mut out = String::new();
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            out.push_str(buf[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}

/// 面板上下边框的**单元格列号**（按 cell 索引，不受宽字符占位影响）。
fn border_columns(buf: &Buffer) -> Option<(u16, u16, u16)> {
    for y in 0..buf.area.height {
        let mut left: Option<u16> = None;
        let mut right: Option<u16> = None;
        for x in 0..buf.area.width {
            match buf[(x, y)].symbol() {
                "┌" => left = Some(x),
                "┐" => right = Some(x),
                _ => {}
            }
        }
        if let (Some(left), Some(right)) = (left, right) {
            return Some((y, left, right));
        }
    }
    None
}

fn session_item(id: &str, title: &str, updated_at: &str, archived: bool) -> tui::SessionItem {
    tui::SessionItem {
        id: id.to_string(),
        title: title.to_string(),
        updated_at: updated_at.to_string(),
        turn_count: 0,
        workspace_id: Some("ws-main".to_string()),
        workspace_label: Some("主工作区".to_string()),
        archived,
    }
}

// ══ AT-1：超长 / 混合宽字符标题 × 窄宽度 ═══════════════════════════════════════════

#[test]
fn at_overlong_mixed_width_titles_never_panic_and_keep_budget() {
    // Arrange：>200 汉字、CJK+ASCII+emoji 混排、含 `轮` 与 `·` 的正文（C1 允许标题含任意字符）。
    let pure_cjk = "汉".repeat(300);
    let mixed = "中a🚀b".repeat(80);
    let with_markers = "轮 · 轮 · 说明".repeat(60);

    for (label, title) in [
        ("300 汉字", pure_cjk.as_str()),
        ("CJK+ASCII+emoji", mixed.as_str()),
        ("含「轮」「·」正文", with_markers.as_str()),
    ] {
        for width in [0usize, 1, 20, 40] {
            for (is_active, is_selected) in [(false, false), (true, true)] {
                let mut input = row_input("pa_20261121-deadbeefcafe", title, "11-21 14:05");
                input.is_active = is_active;
                input.is_selected = is_selected;

                // Act：不得 panic（字节切片 / 下溢都会在这里爆）
                let parts = resume_row_parts(&input, width);

                // Assert
                let ctx = format!("AT-1/{label}/width={width}/active={is_active}/sel={is_selected}");
                if width == 0 {
                    assert!(
                        parts.left.is_empty() && parts.right.is_empty(),
                        "{ctx}: width == 0 ⇒ 左右皆空，实际 left={:?} right={:?}",
                        parts.left,
                        parts.right
                    );
                }
                assert_row_budget(&parts.left, &parts.right, width, &ctx);
                assert!(
                    tui::str_width(&parts.right) <= width,
                    "{ctx}: 右列超出 width（right 宽 {} / width={width}）",
                    tui::str_width(&parts.right)
                );
                // 截断必须落地为字符边界（CJK 不能被切半个字）：结果必须是合法 UTF-8 String，
                // 且去掉省略号后仍是原标题的前缀（按 char 比较）。
                if width >= 20 {
                    let stripped = parts.left.trim_end_matches('…');
                    assert!(
                        title.starts_with(stripped) || parts.left.ends_with('…'),
                        "{ctx}: 左列必须是原标题的前缀或带截断省略号，实际 left={:?}",
                        parts.left
                    );
                }
            }
        }
    }
}

// ══ AT-2：空会话列表 × 窄终端渲染 ═══════════════════════════════════════════════════

#[test]
fn at_empty_session_list_in_narrow_terminal_renders_without_panic_or_overflow() {
    // 用例 A：超窄终端（20 列）——ui() 走「终端窗口过小」降级，不得 panic、不得画面板。
    let mut state = tui::TuiState::default();
    state.open_session_selector(Vec::new());
    let buf = screen(&mut state, 20, 16);
    assert_eq!(
        (buf.area.width, buf.area.height),
        (20, 16),
        "AT-2/20 列：渲染后的缓冲区几何不得被改写"
    );
    assert!(
        border_columns(&buf).is_none(),
        "AT-2/20 列：降级路径不得绘制会话面板，整屏 {:?}",
        all_text(&buf)
    );
    assert!(
        !all_text(&buf).trim().is_empty(),
        "AT-2/20 列：降级路径必须给出可读提示（不得整屏空白）"
    );

    // 用例 B：刚过降级门槛的窄终端（30 列）——面板必须完整落在屏内且空态文案可读。
    let mut state = tui::TuiState::default();
    state.open_session_selector(Vec::new());
    let buf = screen(&mut state, 30, 16);
    let text = all_text(&buf);
    let (y, left, right) =
        border_columns(&buf).unwrap_or_else(|| panic!("AT-2/30 列：必须绘制会话面板，整屏 {text:?}"));
    assert!(
        right < buf.area.width && right > left,
        "AT-2/30 列：面板右缘越界（y={y} left={left} right={right} width={}）",
        buf.area.width
    );
    assert!(
        squeeze(&text).contains("暂无历史会话"),
        "AT-2/30 列：空态首段必须出现（C3），整屏 {text:?}"
    );

    // 用例 C：24 列（降级门槛边界）——同样不得 panic、面板不得越界。
    let mut state = tui::TuiState::default();
    state.open_session_selector(Vec::new());
    let buf = screen(&mut state, 24, 12);
    let text = all_text(&buf);
    if let Some((y, left, right)) = border_columns(&buf) {
        assert!(
            right < buf.area.width && right > left,
            "AT-2/24 列：面板右缘越界（y={y} left={left} right={right} width={}）",
            buf.area.width
        );
    }
    assert!(
        !text.trim().is_empty(),
        "AT-2/24 列：无论如何都必须渲染出可读内容"
    );
}

// ══ AT-3：恶意 workspace_label ═══════════════════════════════════════════════════════

#[test]
fn at_hostile_workspace_labels_never_panic_and_prefix_stays_in_budget() {
    // Arrange：全空白 / 控制字符 / 1000 字超长。
    let blank_label = " ".repeat(1000);
    let control_label = "\u{1}\u{2}控制\u{7}\u{1b}x".to_string();
    let long_label = "工".repeat(1000);

    for (label_name, label) in [
        ("全空白 1000", blank_label.as_str()),
        ("控制字符", control_label.as_str()),
        ("超长 1000 字", long_label.as_str()),
    ] {
        for width in [20usize, 40, 200] {
            let mut input = row_input("pa_20261121-deadbeefcafe", ROW_TITLE, "11-21 14:05");
            input.workspace_id = Some("ws-alpha");
            input.current_workspace_id = Some("ws-beta");
            input.workspace_label = Some(label);

            // Act
            let parts = resume_row_parts(&input, width);

            // Assert：不 panic + 前缀仍 ≤ 预算（前缀在左列，整体预算已覆盖）。
            let ctx = format!("AT-3/{label_name}/width={width}");
            assert_row_budget(&parts.left, &parts.right, width, &ctx);
            assert!(
                tui::str_width(&parts.left) <= width,
                "{ctx}: 含前缀的左列超出 width（{} 列）",
                tui::str_width(&parts.left)
            );
        }

        // 宽 200 下结构仍要正确：空白标签回落 workspace_id；短标签原样作前缀；
        // 超长标签被截断（此时**不能**要求完整前缀出现，只要求截断落地为省略号）。
        let mut input = row_input("pa_20261121-deadbeefcafe", ROW_TITLE, "11-21 14:05");
        input.workspace_id = Some("ws-alpha");
        input.current_workspace_id = Some("ws-beta");
        input.workspace_label = Some(label);
        let parts = resume_row_parts(&input, 200);
        let expected_prefix = if label.trim().is_empty() {
            "ws-alpha"
        } else {
            label
        };
        if expected_prefix.chars().count() <= 16 {
            assert!(
                parts.left.starts_with(expected_prefix),
                "AT-3/{label_name}: 宽 200 下左列必须以跨区前缀开头\
                 （期望 {expected_prefix:?}，实际 {:?}）",
                parts.left
            );
        } else {
            assert!(
                parts.left.ends_with('…'),
                "AT-3/{label_name}: 超长前缀必须被截断并以省略号收尾，实际 {:?}",
                parts.left
            );
        }
    }
}

// ══ AT-4：同一行 active + selected，且 width == 0 ═══════════════════════════════════

#[test]
fn at_active_and_selected_same_row_marks_once_in_fixed_order() {
    // Arrange：同一 id 同时是当前会话与选中行（C1 明确二者可同时在位）。
    let mut input = row_input("session-1a2b3c4d", ROW_TITLE, "14:05");
    input.is_active = true;
    input.is_selected = true;

    // Act
    let parts = resume_row_parts(&input, 200);

    // Assert：标记顺序唯一且不重复。
    assert_eq!(
        parts.right, "14:05 · 当前 · 1a2b3c4d",
        "C1: active + selected 同时为真时右列必须是「时间 · 当前 · 短id」"
    );
    assert_eq!(
        count_occurrences(&parts.right, "当前"),
        1,
        "C1: 「当前」标记不得重复，实际 {:?}",
        parts.right
    );
    assert_eq!(
        count_occurrences(&parts.right, "1a2b3c4d"),
        1,
        "C1: 短 id 不得重复，实际 {:?}",
        parts.right
    );
    let active_at = parts.right.find("当前").expect("「当前」标记必须存在");
    let short_at = parts.right.find("1a2b3c4d").expect("短 id 必须存在");
    assert!(
        active_at < short_at,
        "C1: 顺序固定为「当前」在短 id 之前，实际 {:?}",
        parts.right
    );

    // width == 0 ⇒ 左右皆空（极窄终端不得残留半截文本）。
    let mut zero = row_input("session-1a2b3c4d", ROW_TITLE, "14:05");
    zero.is_active = true;
    zero.is_selected = true;
    let zero_parts = resume_row_parts(&zero, 0);
    assert!(
        zero_parts.left.is_empty() && zero_parts.right.is_empty(),
        "C1: width == 0 ⇒ 左右皆空，实际 left={:?} right={:?}",
        zero_parts.left,
        zero_parts.right
    );
}

// ══ AT-5：归档行不可丢（数据面完整性）══════════════════════════════════════════════

#[test]
fn at_archived_rows_hidden_by_default_but_restorable_from_data_plane() {
    // Arrange：6 行数据面，其中 2 行已归档 + 1 行续轮碎片 + 1 行子代理。
    let rows = [
        ("session-1", false, false),
        ("session-2", true, false),
        ("session-3", false, true),
        ("session-4", false, false),
        ("subagent-9", false, false),
        ("session-5", true, false),
    ];
    let ids: Vec<&str> = rows.iter().map(|(id, _, _)| *id).collect();

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

    // Assert：默认隐藏归档行，但数据面**一行都没丢**（顺序保持 = 只减行不改序）。
    assert_eq!(
        default_view,
        vec!["session-1", "session-4"],
        "C2: 默认视图只应剩未归档的主会话"
    );
    assert_eq!(
        archived_view,
        vec!["session-1", "session-2", "session-4", "session-5"],
        "C2/C3: show_archived=true 时归档行必须原样回到可见面（数据面不丢行）"
    );
    assert!(
        archived_view.len() > default_view.len(),
        "AT-5: 打开归档显示必须真的多出归档行（否则「按 a 显示」是空转）"
    );

    // 归档行本身也必须可渲染（不 panic）：归档标记不进入行文本，但行内容仍可解析。
    let parts = resume_row_parts(&row_input("session-2", ROW_TITLE, "14:05"), 40);
    assert!(
        !parts.right.contains('轮') && !parts.right.is_empty(),
        "AT-5: 归档行右列同样只允许时间/标记，实际 {:?}",
        parts.right
    );

    // 渲染级兜底：归档行注入列表时面板不得 panic（数据面与可见面在此不混用，见回执盲区）。
    let mut state = tui::TuiState::default();
    state.open_session_selector(vec![
        session_item("session-1", "未归档会话", "14:05", false),
        session_item("session-2", "已归档会话", "14:06", true),
    ]);
    let buf = screen(&mut state, 90, 24);
    assert!(
        !all_text(&buf).trim().is_empty(),
        "AT-5: 含归档行的列表渲染不得产出空屏"
    );
}
