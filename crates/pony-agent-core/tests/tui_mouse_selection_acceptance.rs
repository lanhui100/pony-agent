// TUI 鼠标文本选择与复制常规功能契约测试
// 严格遵循 AAA 原则与零假测试规范

#[path = "../src/bin/tui.rs"]
#[allow(dead_code, unused_imports, unused_variables, unused_mut)]
mod tui;

use tui::{
    encode_osc52_copy, extract_selected_text, is_coord_selected, MouseSelectionState, TextCoord,
    TuiState,
};

// ---------------------------------------------------------------------------
// 1. 坐标比较与规范化 normalized_range（正向拖选、反向拖选）
// ---------------------------------------------------------------------------

#[test]
fn test_mouse_selection_normalized_range_forward() {
    // Arrange: anchor 在前，head 在后（正向拖拽）
    let mut selection = MouseSelectionState::default();
    let anchor = TextCoord { line: 1, col: 5 };
    let head = TextCoord { line: 3, col: 10 };
    selection.anchor = Some(anchor);
    selection.head = Some(head);

    // Act
    let normalized = selection.normalized_range();

    // Assert
    assert_eq!(
        normalized,
        Some((anchor, head)),
        "正向拖拽规范化结果应保持 anchor 为起点，head 为终点"
    );
}

#[test]
fn test_mouse_selection_normalized_range_backward() {
    // Arrange: anchor 在后，head 在前（反向拖拽）
    let mut selection = MouseSelectionState::default();
    let anchor = TextCoord { line: 4, col: 2 };
    let head = TextCoord { line: 1, col: 8 };
    selection.anchor = Some(anchor);
    selection.head = Some(head);

    // Act
    let normalized = selection.normalized_range();

    // Assert
    assert_eq!(
        normalized,
        Some((head, anchor)),
        "反向拖拽规范化结果应将 head 调整为起点，anchor 调整为终点"
    );
}

#[test]
fn test_mouse_selection_normalized_range_same_line_backward() {
    // Arrange: 同一行反向拖拽
    let mut selection = MouseSelectionState::default();
    let anchor = TextCoord { line: 2, col: 20 };
    let head = TextCoord { line: 2, col: 5 };
    selection.anchor = Some(anchor);
    selection.head = Some(head);

    // Act
    let normalized = selection.normalized_range();

    // Assert
    assert_eq!(
        normalized,
        Some((head, anchor)),
        "同一行反向拖拽时列号小的应作为起点"
    );
}

#[test]
fn test_mouse_selection_normalized_range_empty_or_partial() {
    // Arrange
    let mut selection = MouseSelectionState::default();

    // Act & Assert 1: 两者皆为空
    assert_eq!(selection.normalized_range(), None);

    // Act & Assert 2: 仅 anchor 存在
    selection.anchor = Some(TextCoord { line: 0, col: 0 });
    assert_eq!(selection.normalized_range(), None);

    // Act & Assert 3: 仅 head 存在
    selection.anchor = None;
    selection.head = Some(TextCoord { line: 0, col: 0 });
    assert_eq!(selection.normalized_range(), None);
}

// ---------------------------------------------------------------------------
// 2. is_coord_selected 单行、跨行首尾、中间行的范围命中判定
// ---------------------------------------------------------------------------

#[test]
fn test_is_coord_selected_single_line() {
    // Arrange
    let anchor = TextCoord { line: 2, col: 5 };
    let head = TextCoord { line: 2, col: 15 };

    // Act & Assert: 正向与反向均应命中
    assert!(!is_coord_selected(TextCoord { line: 2, col: 4 }, anchor, head));
    assert!(is_coord_selected(TextCoord { line: 2, col: 5 }, anchor, head));
    assert!(is_coord_selected(TextCoord { line: 2, col: 10 }, anchor, head));
    assert!(is_coord_selected(TextCoord { line: 2, col: 15 }, anchor, head));
    assert!(!is_coord_selected(TextCoord { line: 2, col: 16 }, anchor, head));

    // 反向 anchor/head 判定结果一致
    assert!(is_coord_selected(TextCoord { line: 2, col: 10 }, head, anchor));
    assert!(!is_coord_selected(TextCoord { line: 2, col: 20 }, head, anchor));
}

#[test]
fn test_is_coord_selected_multi_line_boundary_and_intermediate() {
    // Arrange: 跨行选区从 (1, 10) 到 (3, 5)
    let anchor = TextCoord { line: 1, col: 10 };
    let head = TextCoord { line: 3, col: 5 };

    // Act & Assert:
    // 行 0: 彻底在选区外
    assert!(!is_coord_selected(TextCoord { line: 0, col: 15 }, anchor, head));

    // 行 1 (首行): col < 10 不命中，col >= 10 命中
    assert!(!is_coord_selected(TextCoord { line: 1, col: 9 }, anchor, head));
    assert!(is_coord_selected(TextCoord { line: 1, col: 10 }, anchor, head));
    assert!(is_coord_selected(TextCoord { line: 1, col: 999 }, anchor, head));

    // 行 2 (中间行): 任意列均命中
    assert!(is_coord_selected(TextCoord { line: 2, col: 0 }, anchor, head));
    assert!(is_coord_selected(TextCoord { line: 2, col: 50 }, anchor, head));

    // 行 3 (尾行): col <= 5 命中，col > 5 不命中
    assert!(is_coord_selected(TextCoord { line: 3, col: 0 }, anchor, head));
    assert!(is_coord_selected(TextCoord { line: 3, col: 5 }, anchor, head));
    assert!(!is_coord_selected(TextCoord { line: 3, col: 6 }, anchor, head));

    // 行 4: 彻底在选区外
    assert!(!is_coord_selected(TextCoord { line: 4, col: 0 }, anchor, head));
}

// ---------------------------------------------------------------------------
// 3. extract_selected_text 纯文本提取（单行部分、整行、跨行、越界安全）
// ---------------------------------------------------------------------------

#[test]
fn test_extract_selected_text_single_line_partial_and_full() {
    // Arrange
    let lines = vec![
        "Hello World".to_string(),
        "Rust TUI Mouse Selection".to_string(),
    ];

    // Act 1: 单行部分提取 "World" (col 6..=10)
    let text1 = extract_selected_text(
        &lines,
        TextCoord { line: 0, col: 6 },
        TextCoord { line: 0, col: 10 },
    );
    // Assert 1
    assert_eq!(text1, "World");

    // Act 2: 整行选取 (col 0..=10)
    let text2 = extract_selected_text(
        &lines,
        TextCoord { line: 0, col: 0 },
        TextCoord { line: 0, col: 10 },
    );
    // Assert 2
    assert_eq!(text2, "Hello World");
}

#[test]
fn test_extract_selected_text_multi_line() {
    // Arrange
    let lines = vec![
        "First line".to_string(),
        "Second line content".to_string(),
        "Third and final line".to_string(),
    ];

    // Act: 跨三行选区 (0, 6) 到 (2, 4)
    // line 0: "line"
    // line 1: "Second line content"
    // line 2: "Third"
    let text = extract_selected_text(
        &lines,
        TextCoord { line: 0, col: 6 },
        TextCoord { line: 2, col: 4 },
    );

    // Assert
    let expected = "line\nSecond line content\nThird";
    assert_eq!(text, expected);
}

#[test]
fn test_extract_selected_text_out_of_bounds_safety() {
    // Arrange
    let lines = vec!["Short".to_string()];

    // Act 1: col 远超该行字符长度 (自动饱和到行末字符)
    let text1 = extract_selected_text(
        &lines,
        TextCoord { line: 0, col: 1 },
        TextCoord { line: 0, col: 100 },
    );
    // Assert 1: "hort"
    assert_eq!(text1, "hort");

    // Act 2: line 远超 lines.len()
    let text2 = extract_selected_text(
        &lines,
        TextCoord { line: 10, col: 0 },
        TextCoord { line: 20, col: 5 },
    );
    // Assert 2: 优雅返回空串
    assert_eq!(text2, "");

    // Act 3: 空 lines
    let empty_lines: Vec<String> = vec![];
    let text3 = extract_selected_text(
        &empty_lines,
        TextCoord { line: 0, col: 0 },
        TextCoord { line: 0, col: 5 },
    );
    // Assert 3
    assert_eq!(text3, "");
}

// ---------------------------------------------------------------------------
// 4. OSC 52 编码输出格式断言（空串、普通英文字符、中文字符）
// ---------------------------------------------------------------------------

#[test]
fn test_encode_osc52_copy_format() {
    // Arrange & Act & Assert 1: 空串
    // "" base64 为 ""
    let empty_osc = encode_osc52_copy("");
    assert_eq!(empty_osc, "\x1b]52;c;\x07");

    // Arrange & Act & Assert 2: 普通英文
    // "hello" base64 为 "aGVsbG8="
    let ascii_osc = encode_osc52_copy("hello");
    assert_eq!(ascii_osc, "\x1b]52;c;aGVsbG8=\x07");

    // Arrange & Act & Assert 3: 中文字符
    // "复制测试" UTF-8 字节: [229, 164, 141, 229, 136, 182, 230, 181, 139, 232, 175, 149]
    // base64 为 "5aSN5Yi25rWL6K+V"
    let cjk_osc = encode_osc52_copy("复制测试");
    assert_eq!(cjk_osc, "\x1b]52;c;5aSN5Yi25rWL6K+V\x07");
    assert!(cjk_osc.starts_with("\x1b]52;c;"));
    assert!(cjk_osc.ends_with("\x07"));
}

// ---------------------------------------------------------------------------
// 5. TuiState 初始鼠标选择状态默认值与清理重置方法
// ---------------------------------------------------------------------------

#[test]
fn test_tuistate_initial_mouse_selection_defaults_and_clear() {
    // Arrange: 构造默认 TuiState
    let mut state = TuiState::default();

    // Assert 1: 默认开关开启，selection 处于未激活状态
    assert!(state.mouse_capture, "mouse_capture 默认应处于开启状态");
    assert!(!state.mouse_selection.active, "初始状态下选区不应处于激活状态");
    assert_eq!(state.mouse_selection.anchor, None);
    assert_eq!(state.mouse_selection.head, None);
    assert_eq!(state.mouse_selection.debounce_until, None);
    assert_eq!(state.mouse_selection.copied_range, None);

    // Act: 模拟产生选区与防抖状态
    state.mouse_selection.active = true;
    state.mouse_selection.anchor = Some(TextCoord { line: 1, col: 2 });
    state.mouse_selection.head = Some(TextCoord { line: 3, col: 4 });
    state.mouse_selection.debounce_until = Some(std::time::Instant::now());
    state.mouse_selection.copied_range = Some((
        TextCoord { line: 1, col: 2 },
        TextCoord { line: 3, col: 4 },
    ));

    // Act: 调用重置方法
    state.mouse_selection.clear();

    // Assert 2: 验证全部字段已被完全重置
    assert!(!state.mouse_selection.active);
    assert_eq!(state.mouse_selection.anchor, None);
    assert_eq!(state.mouse_selection.head, None);
    assert_eq!(state.mouse_selection.debounce_until, None);
    assert_eq!(state.mouse_selection.copied_range, None);
}
