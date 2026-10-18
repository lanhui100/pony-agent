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

#[test]
fn layout_invariants_hold_on_standard_terminal() {
    let mut state = tui::TuiState::default();
    state.set_input("hello");
    let buf = screen(&mut state, 110, 30);
    dump_if_requested("01-input", &buf);
    let text = all_text(&buf);

    // header / 发丝线 / 输入提示符 / 状态词
    assert!(row_text(&buf, 0).contains("pony"), "header 必须包含 wordmark");
    assert!(row_text(&buf, 1).trim_start().starts_with('─'), "header 下方必须是发丝线");
    assert!(text.contains("›"), "输入区必须渲染提示符");
    assert!(squeezed(&buf).contains("就绪"), "footer 必须有状态词");

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
    // 中文正文必须真实上屏（宽字符右侧占位 cell 是空格，比较时先去掉空白）
    let squeeze = all_text(&buf).replace(' ', "");
    assert!(squeeze.contains("幂等键"), "中文正文未渲染");
}

#[test]
fn long_history_auto_scrolls_to_latest_message() {
    let mut state = tui::TuiState::default();
    for i in 0..12 {
        state.add_user_message(&format!("question number {i}"));
        state.append_assistant_chunk(&format!("answer number {i} with some detail"));

    }
    let buf = screen(&mut state, 80, 18);
    let text = all_text(&buf);
    assert!(text.contains("question number 11"), "自动贴底失败：最新消息不在视口");
    assert!(!text.contains("question number 0 "), "视口未滚动，仍停留在最早消息");
    assert!(state.auto_scroll, "默认必须保持自动贴底");
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
    let buf = screen(&mut state, 90, 24);
    dump_if_requested("03-panel", &buf);
    let text = all_text(&buf);
    assert!(squeezed(&buf).contains("会话"), "面板标题缺失");
    assert!(text.contains("First session") && text.contains("Second session"));
    // 面板不得吞掉底部输入/状态行
    assert!(squeezed(&buf).contains("就绪"), "面板遮挡了 footer 状态行");
    assert!(text.contains("›"), "面板遮挡了输入提示符");
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
    let buf = screen(&mut state, 60, 12);
    let row = (0..buf.area.height)
        .map(|y| row_text(&buf, y))
        .find(|r| r.contains('›'))
        .expect("未找到输入行");
    // 按显示列（字符）而非字节定位，避免多字节符号干扰
    let caret = row.chars().position(|c| c == '▏').expect("光标字符缺失");
    let prompt = row.chars().position(|c| c == '›').unwrap();
    // 光标应在 'ab' 之后（提示符 "› " 占 2 列）
    assert_eq!(caret, prompt + 2 + 2, "光标位置与输入光标不同步: {row:?}");
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
    assert!(squeezed(&buf).contains("你"), "用户轮标签缺失");

    // 流式活动行：只有一行、不重复状态词、并带耗时
    state.is_streaming = true;
    state.transition_runtime_mode(tui::RuntimeMode::Responding);
    state.turn_started_at = Some(std::time::Instant::now());
    state.spinner_tick = 3;
    let buf = screen(&mut state, 110, 26);
    dump_if_requested("06-streaming", &buf);
    let squeezed_text = squeezed(&buf);
    assert!(squeezed_text.contains("写入中"), "流式活动行缺失");
    assert!(squeezed_text.contains("秒"), "流式活动行缺少耗时");
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
