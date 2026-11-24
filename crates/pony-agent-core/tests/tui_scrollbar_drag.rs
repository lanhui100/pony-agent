//! TUI 右侧滚动条鼠标拖拽契约测试（B192）。
//!
//! 覆盖三层：
//! - 纯函数层：`scrollbar_offset_for_row` 的反解口径（首/末落点、越界饱和、单调性）；
//! - 命中层：`scrollbar_hit` 只认「本帧真画出来的列几何」，没轨道时恒 `None`；
//! - 状态层：`apply_scrollbar_row` 的写入语义（拖到底恢复贴底跟随、拖到中间解除跟随）。

use pony_agent_core::tui;

use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::Terminal;

/// 40 条长用户消息 ⇒ 必然溢出 24 行视口（不靠内容长度碰巧成立）。
fn overflow_state() -> tui::TuiState {
    let mut state = tui::TuiState::default();
    for i in 0..40 {
        state.add_user_message(&format!("第{i:02}问：这是一段足够长的用户消息，用来把消息区撑到溢出视口。"));
    }
    state
}

/// 单条短消息 ⇒ 内容远小于视口（不溢出，没有滚动条）。
fn non_overflow_state() -> tui::TuiState {
    let mut state = tui::TuiState::default();
    state.add_user_message("短消息");
    state
}

/// 渲染到自动贴底收敛（`auto_scroll` 是阻尼跟随，单帧最多推进 3 行）。
fn screen_until_settled(state: &mut tui::TuiState, width: u16, height: u16) -> Buffer {
    let mut terminal =
        Terminal::new(TestBackend::new(width, height)).expect("harness：TestBackend 必须可创建");
    let mut last = terminal.backend().buffer().clone();
    for _ in 0..400 {
        terminal
            .draw(|f| tui::ui(f, state))
            .expect("harness：ui() 渲染不得失败");
        last = terminal.backend().buffer().clone();
        if state.scroll_offset >= state.max_scroll_offset {
            break;
        }
    }
    state.effective_scroll_offset = state.scroll_offset;
    last
}

// ── A1：反解口径（纯函数表驱动）──────────────────────────────────────────────

#[test]
fn a1_offset_for_row_matches_thumb_geometry() {
    // 未溢出 ⇒ 没有轨道，反解必须拒绝（调用方不得改动偏移）。
    assert_eq!(
        tui::scrollbar_offset_for_row(10, 20, 5, 0),
        None,
        "A1：未溢出必须返回 None"
    );

    // total=200 / visible=20 ⇒ thumb_len=2、track=18、denom=180。
    assert_eq!(
        tui::scrollbar_offset_for_row(200, 20, 0, 0),
        Some(0),
        "A1：轨道首行 ⇒ offset 0"
    );
    assert_eq!(
        tui::scrollbar_offset_for_row(200, 20, 19, 0),
        Some(180),
        "A1：轨道末行 ⇒ offset 落在 denom（贴底，不出现「永远差一行」）"
    );
    assert_eq!(
        tui::scrollbar_offset_for_row(200, 20, 999, 0),
        Some(180),
        "A1：越界行号必须饱和到末行"
    );

    // 抓取偏移 1 行 ⇒ 提前 1 行到达顶部（saturating，不反向越界）。
    assert_eq!(
        tui::scrollbar_offset_for_row(200, 20, 0, 1),
        Some(0),
        "A1：抓取偏移大于行号时饱和到 0"
    );

    // 单调性：行号递增 ⇒ offset 非递减，且全程不越界。
    let mut prev = 0u32;
    for row in 0..20u16 {
        let offset = tui::scrollbar_offset_for_row(200, 20, row, 0).expect("A1：溢出必须有反解");
        assert!(
            offset >= prev,
            "A1：offset 必须单调不减，row={row} offset={offset} prev={prev}"
        );
        assert!(offset <= 180, "A1：offset 不得超过 denom，实际 {offset}");
        prev = offset;
    }
}

#[test]
fn a1_offset_for_row_round_trips_thumb_start() {
    // 反解与绘制同口径：把 offset 画成滑块起点，再用起点反解。
    // 轨道只有 `track` 行却要表达 `denom + 1` 个偏移，映射必然**多对一**：
    // 两端（0 / denom）必须精确落点，中间允许一行的颗粒度误差（`ceil(denom / track)`）。
    let (total, visible) = (200usize, 20usize);
    let (_, thumb_len) = tui::scrollbar_thumb(total, visible, 0).expect("A1：溢出必须绘制");
    let track = visible - thumb_len as usize;
    let denom = total - visible;
    let granularity = denom.div_ceil(track).max(1);

    for offset in [0usize, 9, 45, 90, 135, 180] {
        let (thumb_start, _) =
            tui::scrollbar_thumb(total, visible, offset).expect("A1：溢出必须绘制");
        let back = tui::scrollbar_offset_for_row(total, visible, thumb_start, 0)
            .expect("A1：溢出必须有反解") as usize;
        if offset == 0 || offset == denom {
            assert_eq!(
                back, offset,
                "A1：两端必须精确往返，offset={offset} thumb_start={thumb_start} back={back}"
            );
        } else {
            assert!(
                back.abs_diff(offset) <= granularity,
                "A1：中间往返误差不得超过一行颗粒度 {granularity}，offset={offset} back={back}"
            );
        }
    }
}

// ── A2：命中层（几何同源 + 不吞正文）─────────────────────────────────────────

#[test]
fn a2_hit_requires_drawn_scrollbar() {
    // 未溢出：整帧渲染后不得留下命中几何（正文列永不因为「最右列」被吞掉）。
    let mut state = non_overflow_state();
    let buf = screen_until_settled(&mut state, 80, 24);
    assert_eq!(
        state.last_scrollbar_rect, None,
        "A2：不溢出时不得回写滚动条几何"
    );
    assert_eq!(
        tui::scrollbar_hit(&state, buf.area.width - 1, 5),
        None,
        "A2：没有滚动条时最右列不得命中"
    );
}

#[test]
fn a2_hit_only_on_rightmost_column_of_chat_area() {
    let mut state = overflow_state();
    let buf = screen_until_settled(&mut state, 80, 24);
    let chat = state
        .last_chat_rect
        .expect("A2：ui() 必须回写 last_chat_rect");
    let rect = state
        .last_scrollbar_rect
        .expect("A2：溢出时 ui() 必须回写滚动条几何");

    assert_eq!(
        rect.x,
        buf.area.width - 1,
        "A2：滚动条固定在最右列"
    );
    assert_eq!(rect.width, 1, "A2：滚动条只占 1 列");
    assert_eq!(rect.height, chat.height, "A2：滚动条纵向与消息区等高");

    assert!(
        tui::scrollbar_hit(&state, buf.area.width - 1, chat.y).is_some(),
        "A2：消息区首行最右列必须命中"
    );
    assert_eq!(
        tui::scrollbar_hit(&state, buf.area.width - 2, chat.y),
        None,
        "A2：滚动条左侧一列不得命中（正文列不被侵占）"
    );
    assert_eq!(
        tui::scrollbar_hit(&state, buf.area.width - 1, chat.y.saturating_sub(1)),
        None,
        "A2：消息区之上（header/留白）不得命中"
    );
    assert_eq!(
        tui::scrollbar_hit(
            &state,
            buf.area.width - 1,
            chat.y.saturating_add(chat.height)
        ),
        None,
        "A2：消息区之下不得命中"
    );
    assert_eq!(
        tui::selection_point_at(&state, buf.area.width - 1, chat.y),
        None,
        "A2：滚动条列不得同时可起文本选区（两种手势互斥）"
    );
}

#[test]
fn a2_hit_grab_semantics() {
    let mut state = overflow_state();
    screen_until_settled(&mut state, 80, 24);
    let rect = state
        .last_scrollbar_rect
        .expect("A2：溢出时必须回写滚动条几何");
    let chat = state.last_chat_rect.expect("A2：必须回写 chat_rect");
    let visible = chat.height as usize;
    let total = state.max_scroll_offset as usize + visible;
    let (thumb_start, thumb_len) =
        tui::scrollbar_thumb(total, visible, state.effective_scroll_offset as usize)
            .expect("A2：溢出必须绘制滑块");

    // 指针在滑块内 ⇒ grab = 指针相对滑块顶部的行偏移（拖起来跟手，不瞬跳）。
    let inside_row = thumb_start + thumb_len / 2;
    let (_, grab) = tui::scrollbar_hit(&state, rect.x, rect.y + inside_row)
        .expect("A2：滑块内必须命中");
    assert_eq!(
        grab,
        thumb_len / 2,
        "A2：滑块内按下必须保留抓取偏移"
    );

    // 指针在轨道空白 ⇒ grab = thumb_len / 2（语义：让滑块中心吸附到指针）。
    let outside_row = if thumb_start > 0 { 0 } else { visible - 1 };
    let (_, grab) = tui::scrollbar_hit(&state, rect.x, rect.y + outside_row as u16)
        .expect("A2：轨道空白必须命中");
    assert_eq!(
        grab,
        thumb_len / 2,
        "A2：轨道空白按下必须取滑块中心作为抓取偏移"
    );
}

// ── A3：拖拽写入语义 ────────────────────────────────────────────────────────

#[test]
fn a3_drag_to_top_and_bottom() {
    let mut state = overflow_state();
    screen_until_settled(&mut state, 80, 24);
    let rect = state.last_scrollbar_rect.expect("A3：溢出必须回写几何");
    let max_offset = state.max_scroll_offset;
    assert!(max_offset > 0, "A3：夹具必须溢出");
    assert!(
        state.auto_scroll,
        "A3：贴底收敛后必须处于自动跟随态"
    );

    // 拖到顶行 ⇒ offset 0，且解除自动贴底（否则流式新行会立刻把视口拉回底部）。
    tui::apply_scrollbar_row(&mut state, 0, 0);
    assert_eq!(state.scroll_offset, 0, "A3：拖到顶行必须回到首行");
    assert!(!state.auto_scroll, "A3：离开底部必须解除自动贴底");

    // 拖到末行 ⇒ offset = max，并恢复自动贴底跟随。
    tui::apply_scrollbar_row(&mut state, rect.height.saturating_sub(1), 0);
    assert_eq!(
        state.scroll_offset, max_offset,
        "A3：拖到末行必须贴底（offset == max_scroll_offset）"
    );
    assert!(state.auto_scroll, "A3：拖到底部必须恢复自动贴底跟随");
}

#[test]
fn a3_drag_middle_is_stable_and_monotonic() {
    let mut state = overflow_state();
    screen_until_settled(&mut state, 80, 24);
    let rect = state.last_scrollbar_rect.expect("A3：溢出必须回写几何");

    // 拖到中间：非 0 非 max，且脱离跟随。
    let mid = rect.height / 2;
    tui::apply_scrollbar_row(&mut state, mid, 0);
    let mid_offset = state.scroll_offset;
    assert!(mid_offset > 0, "A3：中点 offset 必须 > 0，实际 {mid_offset}");
    assert!(
        mid_offset < state.max_scroll_offset,
        "A3：中点 offset 必须 < max，实际 {mid_offset}"
    );
    assert!(!state.auto_scroll, "A3：中点必须脱离自动贴底");

    // 同一点重复应用必须幂等（拖拽事件会连续到来，不得抖动）。
    tui::apply_scrollbar_row(&mut state, mid, 0);
    assert_eq!(
        state.scroll_offset, mid_offset,
        "A3：同一行重复应用必须幂等"
    );

    // 向下拖一行 ⇒ offset 非递减。
    tui::apply_scrollbar_row(&mut state, mid + 1, 0);
    assert!(
        state.scroll_offset >= mid_offset,
        "A3：向下拖必须非递减，前后 = {mid_offset} → {}",
        state.scroll_offset
    );
}

#[test]
fn a3_apply_without_geometry_is_noop() {
    // 没有滚动条（不溢出 / 面板态）时，任何行号写入都必须无副作用。
    let mut state = non_overflow_state();
    screen_until_settled(&mut state, 80, 24);
    let before = (state.scroll_offset, state.auto_scroll);
    tui::apply_scrollbar_row(&mut state, 5, 0);
    assert_eq!(
        (state.scroll_offset, state.auto_scroll),
        before,
        "A3：无轨道时写入必须是无副作用的空操作"
    );
}

#[test]
fn a3_grab_offset_reaches_ends_without_gap() {
    // 抓住滑块中部（grab > 0）拖到轨道最后一行，必须仍然贴底：
    // 反解不得把行号截断到末行，否则永远差 grab 个 offset 行。
    let mut state = overflow_state();
    screen_until_settled(&mut state, 80, 24);
    let rect = state.last_scrollbar_rect.expect("A3：溢出必须回写几何");
    let chat = state.last_chat_rect.expect("A3：必须回写 chat_rect");
    let visible = chat.height as usize;
    let total = state.max_scroll_offset as usize + visible;
    let (_, thumb_len) =
        tui::scrollbar_thumb(total, visible, state.effective_scroll_offset as usize)
            .expect("A3：溢出必须绘制滑块");
    let grab = thumb_len / 2;

    tui::apply_scrollbar_row(&mut state, rect.height.saturating_sub(1), grab);
    assert_eq!(
        state.scroll_offset, state.max_scroll_offset,
        "A3：抓住滑块中部拖到末行也必须贴底（grab={grab} thumb_len={thumb_len}）"
    );
    assert!(state.auto_scroll, "A3：贴底后必须恢复自动跟随");

    // 反向：拖到轨道首行必须回到 0。
    tui::apply_scrollbar_row(&mut state, 0, grab);
    assert_eq!(state.scroll_offset, 0, "A3：拖到首行必须回到 0");
}

// ── A4：完整手势链（Down → Drag → Up，事件循环唯一入口）────────────────────

#[test]
fn a4_mouse_chain_scrolls_and_releases() {
    use crossterm::event::{MouseButton, MouseEventKind};

    let mut state = overflow_state();
    screen_until_settled(&mut state, 80, 24);
    let rect = state.last_scrollbar_rect.expect("A4：溢出必须回写几何");

    // ① 在滑块上按下：进入拖拽会话，并立刻脱离贴底（滑块不瞬跳，偏移基本不变）。
    assert!(
        state.auto_scroll,
        "A4：夹具必须先处于贴底跟随态（否则无法证明「按下即脱底」）"
    );
    assert!(
        tui::scrollbar_mouse_event(
            &mut state,
            MouseEventKind::Down(MouseButton::Left),
            rect.x,
            rect.y + rect.height - 1
        ),
        "A4：按下滚动条必须被滚动条消费（此前正是这里返回 false ⇒ 拖不动）"
    );
    assert!(
        state.scrollbar_drag.is_some(),
        "A4：按下后必须进入拖拽会话"
    );

    // ② 拖到轨道首行：offset 归零。
    assert!(
        tui::scrollbar_mouse_event(
            &mut state,
            MouseEventKind::Drag(MouseButton::Left),
            rect.x,
            rect.y
        ),
        "A4：拖拽必须被滚动条消费"
    );
    assert_eq!(state.scroll_offset, 0, "A4：拖到首行必须滚到顶");
    assert!(!state.auto_scroll, "A4：离开底部必须解除自动贴底");

    // ③ 拖到屏幕之外（行号越界）：不得 panic，且朝末端饱和。
    assert!(
        tui::scrollbar_mouse_event(
            &mut state,
            MouseEventKind::Drag(MouseButton::Left),
            rect.x,
            rect.y.saturating_add(u16::from(u8::MAX))
        ),
        "A4：越轨拖拽仍属于滚动条手势（否则半途断开会丢事件）"
    );
    assert_eq!(
        state.scroll_offset, state.max_scroll_offset,
        "A4：拖出轨道必须饱和到贴底"
    );

    // ④ 抬起：会话结束，后续拖拽交还正文选区（不得继续吞事件）。
    assert!(
        tui::scrollbar_mouse_event(
            &mut state,
            MouseEventKind::Up(MouseButton::Left),
            rect.x,
            rect.y
        ),
        "A4：抬起必须收口拖拽会话"
    );
    assert_eq!(state.scrollbar_drag, None, "A4：抬起后会话必须清空");
    assert!(
        !tui::scrollbar_mouse_event(
            &mut state,
            MouseEventKind::Drag(MouseButton::Left),
            rect.x,
            rect.y
        ),
        "A4：抬起后的 Drag 必须不再被滚动条消费"
    );
}

#[test]
fn a4_mouse_chain_ignores_body_and_missing_track() {
    use crossterm::event::{MouseButton, MouseEventKind};

    // 不溢出 ⇒ 任何位置的按下/拖动/抬起都不得被滚动条消费。
    let mut state = non_overflow_state();
    screen_until_settled(&mut state, 80, 24);
    let chat = state.last_chat_rect.expect("A4：必须回写 chat_rect");
    for kind in [
        MouseEventKind::Down(MouseButton::Left),
        MouseEventKind::Drag(MouseButton::Left),
        MouseEventKind::Up(MouseButton::Left),
    ] {
        assert!(
            !tui::scrollbar_mouse_event(&mut state, kind, 79, chat.y),
            "A4：没有轨道时 {kind:?} 不得被滚动条消费"
        );
    }

    // 溢出态：正文列（滚动条左侧）同样是正文的手势，不得被滚动条抢走。
    let mut state = overflow_state();
    screen_until_settled(&mut state, 80, 24);
    let rect = state.last_scrollbar_rect.expect("A4：溢出必须回写几何");
    assert!(
        !tui::scrollbar_mouse_event(
            &mut state,
            MouseEventKind::Down(MouseButton::Left),
            rect.x.saturating_sub(1),
            rect.y
        ),
        "A4：正文列的按下必须留给文本选区"
    );
    assert_eq!(state.scrollbar_drag, None, "A4：正文列按下不得起拖");
}
