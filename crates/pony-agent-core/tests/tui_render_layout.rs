// 渲染层回归测试：把 ui() 渲染进 TestBackend，对「布局不变量」做真实断言。
// 仅当 TUI_SNAPSHOT=1 时额外把每格字符/颜色导出到 /tmp/tui-snap 供人工视觉审查（不影响断言）。
#[path = "../src/bin/tui.rs"]
#[allow(dead_code, unused_imports, unused_variables, unused_mut)]
mod tui;

use ratatui::backend::TestBackend;
use ratatui::style::{Color, Modifier};
use ratatui::Terminal;
use std::fmt::Write as _;

fn screen(state: &mut tui::TuiState, width: u16, height: u16) -> ratatui::buffer::Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|f| tui::ui(f, state)).unwrap();
    terminal.backend().buffer().clone()
}

fn row_text(buf: &ratatui::buffer::Buffer, y: u16) -> String {
    let mut out = String::new();
    for x in 0..buf.area.width {
        out.push_str(buf[(x, y)].symbol());
    }
    out
}

fn all_text(buf: &ratatui::buffer::Buffer) -> String {
    (0..buf.area.height).map(|y| row_text(buf, y)).collect::<Vec<_>>().join("\n")
}

/// 宽字符右侧的占位 cell 是空格，CJK 断言前先去掉空白再比。
fn squeezed(buf: &ratatui::buffer::Buffer) -> String {
    all_text(buf).chars().filter(|c| !c.is_whitespace()).collect()
}

fn dump_if_requested(name: &str, buf: &ratatui::buffer::Buffer) {
    if std::env::var("TUI_SNAPSHOT").ok().as_deref() != Some("1") {
        return;
    }
    let color_hex = |c: Color| match c {
        Color::Rgb(r, g, b) => format!("#{:02x}{:02x}{:02x}", r, g, b),
        Color::Reset => "reset".to_string(),
        other => format!("{:?}", other),
    };
    let mut json = String::from("{\"cells\":[");
    let mut first = true;
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            let cell = &buf[(x, y)];
            if !first {
                json.push(',');
            }
            first = false;
            let mut bits = 0u32;
            if cell.modifier.contains(Modifier::BOLD) {
                bits |= 1;
            }
            if cell.modifier.contains(Modifier::ITALIC) {
                bits |= 4;
            }
            let _ = write!(
                json,
                "{{\"s\":{:?},\"f\":\"{}\",\"b\":\"{}\",\"m\":{}}}",
                cell.symbol(),
                color_hex(cell.fg),
                color_hex(cell.bg),
                bits
            );
        }
    }
    json.push_str(&format!("],\"w\":{},\"h\":{}}}", buf.area.width, buf.area.height));
    let _ = std::fs::create_dir_all("/tmp/tui-snap");
    let _ = std::fs::write(format!("/tmp/tui-snap/{}.json", name), json);
}

/// 该平台语义下的绝对路径判定（供 `starts_with_folded_or_absolute_path` 使用）。
///
/// Windows 上 `Path::is_absolute()` 只认「盘符 + 分隔符」或 UNC；这里显式实现，
/// 使判据在两种平台下语义一致（Windows 语义的镜像测试见
/// `tui_dialog_wave39.rs::a4_windows_path_shapes_are_testable_on_linux`）。
fn is_absolute_path_for_platform(token: &str) -> bool {
    if cfg!(windows) {
        let bytes = token.as_bytes();
        let drive_absolute = bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && (bytes[2] == b'\\' || bytes[2] == b'/');
        drive_absolute || (token.starts_with("\\\\") && token.len() > 2)
    } else {
        std::path::Path::new(token).is_absolute()
    }
}

/// 指标行/最后一行是否以「折叠 cwd（`~`/`~\…`）」或「绝对路径」起排（平台无关）。
///
/// 原断言写作 `trim_start().starts_with('~') || trim_start().starts_with('/')`，
/// **只在 Unix 成立**：Windows 绝对路径形如 `C:\Users\...`（盘符 + 反斜杠），
/// 两个前缀都不命中 → 正向断言在 Windows 恒假红、反向断言在 Windows 恒真（空转）。
/// 平台无关判据：折叠形态以 `~` 起；其余按平台各自的绝对路径语义判定。
fn starts_with_folded_or_absolute_path(text: &str) -> bool {
    let token = text.trim_start().split_whitespace().next().unwrap_or("");
    token.starts_with('~') || is_absolute_path_for_platform(token)
}

#[test]
fn layout_invariants_hold_on_standard_terminal() {
    let mut state = tui::TuiState::default();
    state.set_input("hello");
    let buf = screen(&mut state, 110, 30);
    dump_if_requested("01-input", &buf);
    let text = all_text(&buf);

    // header / 发丝线 / 输入提示符 / 指标行
    // 契约出处：ADR 0038 §决策「分区只用 1px 发丝线 + 2 列沟槽，禁止整块边框」；
    // ADR 0042 §决策 1/3/4「composer 色带几何 + 移除右下角状态词 + 色带下方 1 行指标行」。
    // 断言漂移说明：本用例此前断言 `─`（旧实现字符）与 footer 状态词「就绪」——后者已被
    // ADR 0042 显式删除（`status_word()` 只服务正文流式活动行），故改为断言指标行。
    assert!(row_text(&buf, 0).contains("pony"), "header 必须包含 wordmark");

    // 发丝线：header 正下方一行由**同一字符**连续铺开、横贯内容列，且不得退化成方框边框。
    let rule_row = row_text(&buf, 1);
    let rule_glyphs: Vec<char> = rule_row.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(!rule_glyphs.is_empty(), "header 下方必须是发丝线，实际空行");
    let rule_glyph = rule_glyphs[0];
    assert!(
        rule_glyphs.iter().all(|g| *g == rule_glyph),
        "发丝线必须由单一字符连续铺开，实际 {rule_row:?}"
    );
    assert!(
        !['─', '│', '┌', '┐', '└', '┘', '┼'].contains(&rule_glyph),
        "发丝线不得使用方框/边框字符（ADR 0038：禁止整块边框），实际 {rule_glyph:?}"
    );
    assert!(
        rule_glyphs.len() >= 100,
        "发丝线必须横贯内容列（110 列宽 → 内容列 ≥100），实际 {} 列",
        rule_glyphs.len()
    );

    assert!(text.contains("›"), "输入区必须渲染提示符");

    // composer 几何与指标行（wave-39 v4 R13 + ADR 0042 R5/R7）：
    // 提示符在输入行；其下 1 行呼吸行（色带末行，必须全空）；再下一行是**最后一行**指标行，
    // 左侧为折叠成 `~` 的 cwd，右侧为 token/缓存/上下文簇（默认无用量时只有左侧）。
    let prompt_row = (0..buf.area.height)
        .find(|y| row_text(&buf, *y).contains('›'))
        .expect("未找到输入行");
    assert_eq!(
        prompt_row,
        buf.area.height - 3,
        "输入行必须紧贴底部区块（指标行是最后一行、中间 1 行呼吸），实际 y={prompt_row}"
    );
    assert!(
        row_text(&buf, prompt_row + 1).trim().is_empty(),
        "输入行与指标行之间必须是 1 行空白呼吸行，实际 {:?}",
        row_text(&buf, prompt_row + 1)
    );
    let metrics_row = row_text(&buf, buf.area.height - 1);
    // 平台无关判据替代 Unix-only 的 `starts_with('~') || starts_with('/')`（见 helper 注释）。
    assert!(
        starts_with_folded_or_absolute_path(&metrics_row),
        "指标行左侧必须是折叠后的 cwd 或绝对路径（ADR 0042 R7），实际 {metrics_row:?}"
    );
    assert!(
        !metrics_row.contains('›'),
        "指标行不得混入 composer 提示符，实际 {metrics_row:?}"
    );
    let canvas_bg = buf[(0, 2)].bg;
    assert_eq!(
        buf[(0, buf.area.height - 1)].bg,
        canvas_bg,
        "指标行必须落在画布底上（不在 composer 色带内，ADR 0042 R7），实际底色与内容区不一致"
    );

    // 左右沟槽：任何一行都不允许贴边渲染内容
    for y in 0..buf.area.height {
        for x in [0u16, 1, buf.area.width - 2, buf.area.width - 1] {
            let sym = buf[(x, y)].symbol();
            assert!(
                sym == " " || sym == "─" || sym == "▏",
                "第 {y} 行第 {x} 列贴边出现内容 {sym:?}，破坏沟槽留白"
            );
        }
    }

    // 控制字符绝不能进入渲染缓冲（防止 ANSI 污染/终端状态破坏）
    assert!(
        !text.chars().any(|c| c.is_control() && c != '\n'),
        "渲染缓冲中出现控制字符"
    );
}

#[test]
fn cjk_wrap_never_overflows_content_width() {
    let mut state = tui::TuiState::default();
    state.add_user_message("请分析这个仓库的重试逻辑，并给出一个最小改动的方案，同时说明幂等键的设计依据。");
    state.append_assistant_chunk("结论：当前的重试预算由 retry.rs 中的常量控制，指数退避上限为 2 秒，重试次数上限为 3 次。建议保持协议不变，只把提交边界后移到幂等键之后，这样可以在不改变对外契约的前提下消除重复投递，并且不引入新的并发窗口。");
    let buf = screen(&mut state, 72, 24);
    dump_if_requested("02-cjk", &buf);

    // 内容区宽度 = 72 - 2*2 = 68（左右沟槽各 2 列）。宽字符占 2 个 cell，右侧 cell 由
    // ratatui 填为空格，因此断言用「cell 边界」而非字符宽度：右沟槽必须全空，且正文不得越界。
    let gutter = 2usize;
    let content_w = buf.area.width as usize - gutter * 2;
    for y in 0..buf.area.height {
        for x in 0..buf.area.width as usize {
            let sym = buf[(x as u16, y)].symbol();
            let outside = x < gutter || x >= gutter + content_w;
            if outside {
                assert!(
                    sym == " " || sym == "─" || sym == "▏",
                    "第 {y} 行第 {x} 列越界渲染 {sym:?}（内容宽度 {content_w}）"
                );
            }
        }
    }

    // 折行真实发生：同一段中文必须分布在 >= 2 个屏幕行
    let cjk_rows = (0..buf.area.height)
        .filter(|y| row_text(&buf, *y).contains('请') || row_text(&buf, *y).contains('幂'))
        .count();
    assert!(cjk_rows >= 2, "长中文未折行，仍挤在单行（cjk_rows={cjk_rows}）");
    // 中文正文必须真实上屏：按**阅读序**比较——剔除空白与逐行重复的轨道字符 `▌`。
    // 原断言（`all_text(...).replace(' ',"")` 后 contains("幂等键")）之所以恒假：折行后
    // "幂" 落在上一行行尾、"等键" 折到下一行，拼接串里中间夹着换行与轨道字符 `▌`，
    // 短语永远不可能连续匹配（与正文是否渲染无关）。剔除轨道字符后仍能抓「丢字/重字/错序」。
    let reading_order: String = all_text(&buf)
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '▌')
        .collect();
    assert!(
        reading_order.contains("幂等键"),
        "中文正文未连续上屏（折行处字形丢失/错序），实际 {reading_order:?}"
    );
    assert!(
        reading_order.contains("消除重复投递"),
        "中文正文尾部缺失（未渲染或被提前截断），实际 {reading_order:?}"
    );
}

#[test]
fn long_history_auto_scrolls_to_latest_message() {
    let mut state = tui::TuiState::default();
    for i in 0..12 {
        state.add_user_message(&format!("question number {i}"));
        state.append_assistant_chunk(&format!("answer number {i} with some detail"));

    }
    // 原断言为什么错：贴底是**逐帧阻尼跟随**（wave-41 / `8c14eb0`：`ui()` 内每次 draw
    // 只把 offset 朝 `max_scroll_offset` 推进 3/2/1 行，用于消除画面撕裂），因此单帧渲染
    // 后当然还停在长历史上方。真实 TUI 每秒数十帧，几百毫秒内收敛。这里按帧推进到收敛。
    let mut buf = screen(&mut state, 80, 18);
    let first_frame_offset = state.effective_scroll_offset;
    let mut frames = 0usize;
    let converged = loop {
        if state.effective_scroll_offset >= state.max_scroll_offset {
            break true;
        }
        if frames >= 400 {
            break false;
        }
        let prev = state.effective_scroll_offset;
        buf = screen(&mut state, 80, 18);
        frames += 1;
        assert!(
            state.effective_scroll_offset >= prev,
            "自动贴底必须单调向底部推进：第 {frames} 帧倒退了（{prev} → {}）",
            state.effective_scroll_offset
        );
    };
    assert!(
        converged,
        "自动贴底未收敛：{frames} 帧后 offset={} / max_scroll_offset={}（阻尼步长卡死或不再推进）",
        state.effective_scroll_offset, state.max_scroll_offset
    );
    assert!(state.auto_scroll, "默认必须保持自动贴底");
    let text = all_text(&buf);
    assert!(text.contains("question number 11"), "自动贴底失败：最新消息不在视口");
    assert!(!text.contains("question number 0 "), "视口未滚动，仍停留在最早消息");
    // 夹具自检：确实存在可滚动空间，且首帧并没有直接贴底——这样「收敛」断言才有鉴别力。
    assert!(
        state.max_scroll_offset > 0,
        "长历史必须真的产生滚动空间（max_scroll_offset={}）",
        state.max_scroll_offset
    );
    assert!(
        first_frame_offset < state.max_scroll_offset,
        "首帧已经贴底（offset={first_frame_offset}），阻尼跟随未被覆盖，夹具失去鉴别力"
    );

    // 回归抓取：追加新消息后必须重新跟随到底（抓「auto_scroll 被永久关掉 / 贴底只在初始化时生效」）。
    state.add_user_message("question number 12");
    let offset_before_follow = state.effective_scroll_offset;
    for _ in 0..400 {
        buf = screen(&mut state, 80, 18);
        if state.effective_scroll_offset >= state.max_scroll_offset {
            break;
        }
    }
    assert!(
        state.effective_scroll_offset > offset_before_follow,
        "追加新消息后视口必须继续向底部跟随（before={offset_before_follow} / now={}）",
        state.effective_scroll_offset
    );
    assert!(
        all_text(&buf).contains("question number 12"),
        "追加新消息后必须自动贴底到最新一条"
    );
}

#[test]
fn panel_renders_inside_screen_and_keeps_footer() {
    let mut state = tui::TuiState::default();
    state.open_session_selector(vec![
        tui::SessionItem {
            id: "session-aaaa".into(),
            title: "First session".into(),
            updated_at: "0".into(),
        },
        tui::SessionItem {
            id: "session-bbbb".into(),
            title: "Second session".into(),
            updated_at: "0".into(),
        },
    ]);
    let mut buf = screen(&mut state, 90, 24);
    dump_if_requested("03-panel", &buf);
    let text = all_text(&buf);
    assert!(squeezed(&buf).contains("会话"), "面板标题缺失");
    assert!(text.contains("First session") && text.contains("Second session"));
    // 原断言为什么错：它要求「面板不得遮挡 footer 状态词 + composer 提示符」，但
    // ADR 0042 §决策 3 已删除 composer 右下角状态词，且 `tui.rs:3593` 明确规定浮层态
    // 「composer 与指标行整体收起，键位提示由浮层自己承担」——该结论已被兄弟用例
    // `tests/tui_dialog_wave39.rs`（浮层态用例 3）冻结为绿。这里改为断言同一契约。
    assert!(
        !text.contains('›'),
        "浮层态必须收起 composer 提示符（tui.rs 浮层收起契约），实际屏幕: {text:?}"
    );
    assert!(
        squeezed(&buf).contains("选择") && squeezed(&buf).contains("确认"),
        "浮层必须自带键位提示（↑↓ 选择 · ↵ 确认 · esc 取消），实际: {text:?}"
    );
    let panel_top = (0..buf.area.height)
        .find(|y| row_text(&buf, *y).contains('┌'))
        .expect("浮层必须有上边框");
    let panel_bottom = (0..buf.area.height)
        .find(|y| row_text(&buf, *y).contains('└'))
        .expect("浮层必须有下边框");
    assert!(panel_bottom > panel_top, "浮层上下边框顺序异常");
    // 浮层必须完整落在内容列内（左右沟槽各 2 列）且不触碰屏幕上下边缘。
    let top_row = row_text(&buf, panel_top);
    let left = top_row.chars().position(|c| c == '┌').unwrap() as u16;
    let right = top_row.chars().position(|c| c == '┐').expect("浮层上边框必须闭合") as u16;
    let gutter = 2u16;
    assert!(
        left >= gutter && right <= buf.area.width - gutter - 1,
        "浮层越过内容列（left={left}, right={right}, width={}）",
        buf.area.width
    );
    assert!(panel_top > 0 && panel_bottom < buf.area.height - 1, "浮层不得贴屏幕上下边缘");
    // 浮层态指标行必须收起：最后一行不得出现折叠 cwd（ADR 0042 R7 的左侧簇）。
    // 原断言只查 `starts_with('~')`：cwd 不在 HOME 下时（Windows 上永远是 `C:\...`）
    // 该断言恒真 → 反向断言空转。改用平台无关判据后，绝对路径也会被抓到。
    let modal_last_row = row_text(&buf, buf.area.height - 1);
    assert!(
        !starts_with_folded_or_absolute_path(&modal_last_row),
        "浮层态必须收起指标行，实际最后一行 {modal_last_row:?}"
    );

    // 反向断言（防「面板卡死 / 永久吞掉输入区」）：关闭浮层后 composer 与指标行必须恢复。
    state.cancel_session_selector();
    buf = screen(&mut state, 90, 24);
    let restored = all_text(&buf);
    assert!(restored.contains('›'), "关闭浮层后 composer 提示符必须恢复，实际屏幕: {restored:?}");
    assert!(
        !restored.contains("First session"),
        "关闭浮层后面板必须消失，实际屏幕: {restored:?}"
    );
    let restored_last_row = row_text(&buf, buf.area.height - 1);
    assert!(
        starts_with_folded_or_absolute_path(&restored_last_row),
        "关闭浮层后指标行必须恢复（ADR 0042 R7），实际 {restored_last_row:?}"
    );
}

#[test]
fn tiny_terminal_degrades_without_panic() {
    let mut state = tui::TuiState::default();
    let buf = screen(&mut state, 20, 6);
    assert!(squeezed(&buf).contains("终端窗口过小"));
}

#[test]
fn input_caret_tracks_cursor_position() {
    let mut state = tui::TuiState::default();
    state.set_input("abcd");
    state.move_cursor_left();
    state.move_cursor_left();
    assert_eq!(state.input_cursor, 2, "夹具必须把光标停在 ab|cd");
    let buf = screen(&mut state, 60, 12);
    let (row_y, row) = (0..buf.area.height)
        .map(|y| (y, row_text(&buf, y)))
        .find(|(_, r)| r.contains('›'))
        .expect("未找到输入行");
    // 原断言为什么错：composer 的光标不是 `▏` 字形（`▏` 只用于 add-provider 对话框 / Ask 面板 /
    // 菜单选中竖条），而是把**光标下的字符**反白成块：`fg(CANVAS) + bg(ACCENT)`，行尾补空格
    // （见 `render_input`）。故改为「按 fg/bg 配对定位光标格」。
    let prompt_col = row.chars().position(|c| c == '›').unwrap() as u16;
    let accent = buf[(prompt_col, row_y)].fg;
    let canvas = buf[(0, 2)].bg;
    assert_ne!(
        accent, canvas,
        "提示符必须使用强调色前景（作为光标底色的同源色），实际 {accent:?}"
    );
    let caret_cells: Vec<u16> = (0..buf.area.width)
        .filter(|x| buf[(*x, row_y)].bg == accent)
        .collect();
    if caret_cells.len() != 1 {
        let dump: Vec<String> = (0..buf.area.width)
            .map(|x| {
                let cell = &buf[(x, row_y)];
                format!("({x},\"{}\",fg={:?},bg={:?})", cell.symbol(), cell.fg, cell.bg)
            })
            .collect();
        panic!(
            "输入行必须恰好有 1 个强调色底光标格，实际 {} 个 {:?}；整行格子: {}；行文本 {row:?}",
            caret_cells.len(),
            caret_cells,
            dump.join(" ")
        );
    }
    let caret = caret_cells[0];
    // 提示符 "› " 占 2 列，"ab" 再占 2 列 → 光标必须落在此列。
    assert_eq!(
        caret,
        prompt_col + 2 + 2,
        "光标位置与输入光标不同步: prompt_col={prompt_col}, caret={caret}, row={row:?}"
    );
    assert_eq!(
        buf[(caret, row_y)].symbol(),
        "c",
        "光标必须是块光标压在光标下字符 'c' 上，实际 {:?}",
        buf[(caret, row_y)].symbol()
    );
    assert_eq!(
        buf[(caret, row_y)].bg,
        accent,
        "光标格必须是强调色底（块光标语义）"
    );
    assert_eq!(
        buf[(caret, row_y)].fg,
        canvas,
        "光标格前景必须是画布色（反白），实际 {:?}",
        buf[(caret, row_y)].fg
    );
}

#[test]
fn conversation_hierarchy_and_streaming_render() {
    let mut state = tui::TuiState::default();
    state.add_user_message("Refactor the ingest path so retries are idempotent.");
    state.add_or_update_tool_event("Read  crates/ingest/src/pipeline.rs", false);
    state.add_or_update_tool_event("Read  crates/ingest/src/pipeline.rs  · 412 lines", true);
    state.append_assistant_chunk("Here is the plan:\n\n### 1. Split the commit boundary\n\n- Move `flush()` behind the idempotency key\n- Keep the retry budget unchanged\n\n```rust\nfn commit(batch: Batch) -> Result<()> { ... }\n```\n\nThe key invariant is **exactly-once** delivery per batch.");
    state.add_or_update_tool_event("Bash  cargo test -p ingest", true);
    state.add_user_message("Show me how the retry budget is computed.");
    state.append_assistant_chunk("It is `max_attempts = 3`, with exponential backoff capped at 2s.");
    let buf = screen(&mut state, 110, 30);
    dump_if_requested("02-conversation", &buf);
    let text = all_text(&buf);
    assert!(text.contains("exactly-once"), "正文粗体渲染丢失");
    assert!(text.contains("fn commit(batch: Batch)"), "代码块渲染丢失");
    // 原断言为什么错：`你` 角色文本标签已被删除（ADR 0043 §决策 6「角色改由『用户块材质 +
    // 轨道 + 留白』承担」，并点名 `你` 与 `pony` 字标同类；ADR 0042 亦确立「用材质而非文字
    // 标签表达角色」）。改为断言用户轮的**块材质**：行首轨道块 `▌` + 整块 SURFACE 底色 ≠ 画布。
    let canvas_bg = buf[(0, 2)].bg;
    // 用视口内可见的第二条用户轮做材质断言（首条已被自动贴底滚出视口）。
    let user_row = (0..buf.area.height)
        .find(|y| row_text(&buf, *y).contains("Show me how the retry budget"))
        .expect("用户轮正文未渲染");
    let user_row_text = row_text(&buf, user_row);
    let bar_col = user_row_text
        .chars()
        .position(|c| c == '▌')
        .expect("用户轮必须以轨道块 ▌ 起头（ADR 0043 角色材质）");
    let bar_bg = buf[(bar_col as u16, user_row)].bg;
    assert_ne!(
        bar_bg, canvas_bg,
        "用户轮必须有区别于画布的块材质（SURFACE 底），实际与画布同底: {user_row_text:?}"
    );
    assert_eq!(
        buf[(bar_col as u16 + 2, user_row)].bg,
        bar_bg,
        "用户轮正文必须与轨道同一块材质（整块底色连续），实际 {user_row_text:?}"
    );

    // 流式活动行：只有一行、不重复状态词、并带耗时
    // 注：活动行位于滚动内容**末尾**，而贴底是逐帧阻尼跟随（同 long_history 用例），
    // 故先推进到收敛再断言，否则活动行还在视口外。
    state.is_streaming = true;
    state.transition_runtime_mode(tui::RuntimeMode::Responding);
    state.turn_started_at = Some(std::time::Instant::now());
    state.spinner_tick = 3;
    let mut buf = screen(&mut state, 110, 26);
    for _ in 0..400 {
        if state.effective_scroll_offset >= state.max_scroll_offset {
            break;
        }
        buf = screen(&mut state, 110, 26);
    }
    dump_if_requested("06-streaming", &buf);
    assert_eq!(
        state.effective_scroll_offset, state.max_scroll_offset,
        "流式态必须贴底，否则活动行在视口外"
    );
    let squeezed_text = squeezed(&buf);
    assert!(squeezed_text.contains("生成中"), "流式活动行缺失");
    assert!(squeezed_text.contains("秒"), "流式活动行缺少耗时");
    // ADR 0043 §决策 3：三态词互斥——`生成中` 与 `思考中`/`执行` 不得同屏（活动行只有一行）。
    assert!(
        !squeezed_text.contains("思考中") && !squeezed_text.contains("执行·"),
        "三态状态词必须互斥，实际屏幕: {squeezed_text:?}"
    );
}

#[test]
fn command_menu_and_provider_panel_render() {
    let mut state = tui::TuiState::default();
    state.add_user_message("hello");
    state.append_assistant_chunk("Hi. What would you like to do?");
    state.set_input("/");
    let buf = screen(&mut state, 110, 26);
    dump_if_requested("04-command-menu", &buf);
    let menu_row = (0..buf.area.height)
        .map(|y| row_text(&buf, y))
        .find(|r| r.contains("/resume"))
        .expect("命令菜单未渲染 /resume");
    let menu_row_squeezed: String = menu_row.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(
        menu_row_squeezed.contains("选择并恢复历史会话"),
        "命令描述缺失: {menu_row:?}"
    );

    state.set_input("");
    state.open_provider_selector(vec![
        tui::ProviderItem {
            id: "ponyllm".into(),
            name: "ponyllm".into(),
            protocol: "openai".into(),
            base_url: "http://127.0.0.1:8080".into(),
        },
        tui::ProviderItem {
            id: "deepseek".into(),
            name: "DeepSeek".into(),
            protocol: "openai".into(),
            base_url: "https://api.deepseek.com".into(),
        },
    ]);
    let buf = screen(&mut state, 110, 30);
    dump_if_requested("05-provider-panel", &buf);
    assert!(squeezed(&buf).contains("提供商"), "提供商面板标题缺失");
    assert!(all_text(&buf).contains("DeepSeek"));
}
